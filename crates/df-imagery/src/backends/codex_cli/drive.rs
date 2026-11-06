//! Interactive Codex image turn driver: handshake, thread open, turn dispatch.
//!
//! The client sends `initialize`, validates the matching response, then sends
//! the `initialized` notification itself and proceeds to `thread/start`. The
//! server never sends `initialized`; waiting for one would stall every turn.
//! After `turn/start` is sent, any malformed, missing or conflicting turn
//! evidence stays `Unknown`: the request may already have started remote
//! work, so only a correlated refusal or terminal outcome proves failure.

use std::path::Path;
use std::time::Instant;

use serde_json::Value;
use tokio::sync::oneshot;

use super::message_bound;
use super::process::{ProcessError, ProcessSession, StreamLimits};
use super::protocol::{
    IMAGE_GENERATION_ITEM_TYPE, INITIALIZE_METHOD, INITIALIZED_METHOD, ITEM_COMPLETED_METHOD,
    ImageItem, Inbound, MAXIMUM_RESPONSE_LINES, ObservedUsage, ProtocolError, ResponseView, RpcId,
    THREAD_START_METHOD, TOKEN_USAGE_UPDATED_METHOD, TURN_COMPLETED_METHOD, TURN_INTERRUPT_METHOD,
    TURN_START_METHOD, TURN_STARTED_METHOD, TurnCompletedView, encode_notification, encode_request,
    extract_initialize_result, extract_thread_id, extract_token_usage_update,
    extract_turn_completed, extract_turn_id_from_start, extract_turn_started, initialize_params,
    observe_usage, parse_image_item, parse_line, thread_start_params, turn_interrupt_params,
    turn_start_params,
};
use super::settle::settle_turn;
use super::{
    CodexCliConfig, CodexConfigError, CodexError, CodexImageRequest, CodexOutcome, UnknownReason,
};

/// Request ids used on one connection: initialize, then thread open, then
/// turn start, then the best-effort interrupt. Values stay stable so recorded
/// fixtures keep their meaning.
const INITIALIZE_REQUEST_ID: u64 = 0;
const THREAD_REQUEST_ID: u64 = 1;
const TURN_REQUEST_ID: u64 = 2;
const INTERRUPT_REQUEST_ID: u64 = 3;

/// Bound for the tiny best-effort interrupt request.
const INTERRUPT_MESSAGE_BYTES: usize = 4096;
/// Cleanup grace for the best-effort interrupt write: cancellation must reach
/// the owned tree kill even when the helper stdin pipe is blocked, so the
/// interrupt send never waits past this grace.
const INTERRUPT_SEND_GRACE: std::time::Duration = std::time::Duration::from_millis(20);

enum WireEvent {
    Line(Vec<u8>),
    Eof,
    Cancelled,
    Expired,
    StreamFailed(ProcessError),
}

/// Frozen turn inputs and the shared absolute deadline for one owned session.
pub(super) struct TurnContext<'a> {
    pub config: &'a CodexCliConfig,
    pub request: &'a CodexImageRequest,
    pub scratch_text: &'a str,
    pub outputs_root: &'a Path,
    pub image_paths: &'a [String],
    pub instruction: &'a str,
    pub limits: &'a StreamLimits,
    pub deadline_at: Instant,
}

/// The wire state one connection carries across phases: the caller's
/// cancellation, the shared deadline and the flood count of server lines.
struct Wire {
    cancel: Option<oneshot::Receiver<()>>,
    deadline_at: Instant,
    max_line_bytes: usize,
    lines_seen: u64,
}

/// Runs one image turn without retry or fallback after an uncertain dispatch.
pub(super) async fn drive<S>(
    session: &mut S,
    context: TurnContext<'_>,
    cancel: Option<oneshot::Receiver<()>>,
) -> CodexOutcome
where
    S: ProcessSession,
{
    let mut observed = ObservedUsage::unknown();
    let mut wire = Wire {
        cancel,
        deadline_at: context.deadline_at,
        max_line_bytes: context.limits.max_line_bytes,
        lines_seen: 0,
    };
    let ending = run_turn(session, &context, &mut wire, &mut observed).await;
    // Every ending kills the owned tree. Before settlement this also stops
    // helper access, so the verified bytes cannot be swapped under the receipt.
    kill_session(session).await;
    let outcome = match ending {
        Ok(view) => {
            settle_turn(
                view,
                context.request,
                context.image_paths,
                context.outputs_root,
                context.config.codex_home(),
                context.config.max_artifact_bytes,
                context.deadline_at,
            )
            .await
        }
        Err(decided) => decided.into_outcome(),
    };
    with_usage(outcome, observed)
}

/// Kills the owned tree once the outcome is decided.
pub(super) async fn kill_session<S>(session: &mut S)
where
    S: ProcessSession,
{
    // A failed kill changes nothing: the outcome is already decided, and the
    // session's drop closes the same job or process group.
    let _ = session.kill().await;
}

/// Handshake, thread open and dispatch, then waits for the correlated
/// completed turn.
async fn run_turn<S>(
    session: &mut S,
    context: &TurnContext<'_>,
    wire: &mut Wire,
    observed: &mut ObservedUsage,
) -> Result<TurnCompletedView, Decided>
where
    S: ProcessSession,
{
    let bound = message_bound(context.config);
    let initialize = encode_request(
        INITIALIZE_REQUEST_ID,
        INITIALIZE_METHOD,
        initialize_params(),
        bound,
    );
    send_before_dispatch(session, initialize, wire.deadline_at).await?;
    let response = await_handshake_response(session, wire, INITIALIZE_REQUEST_ID).await?;
    response_error_or_initialized(response).map_err(Decided::Failed)?;
    // Client readiness is a notification the adapter sends, not one it waits
    // for. It travels under the same absolute deadline as every handshake
    // write; cancellation stays observed by the surrounding event reads.
    let initialized = encode_notification(INITIALIZED_METHOD, serde_json::json!({}), bound);
    send_before_dispatch(session, initialized, wire.deadline_at).await?;
    let thread_start = encode_request(
        THREAD_REQUEST_ID,
        THREAD_START_METHOD,
        thread_start_params(context.scratch_text, context.config.agent_model()),
        bound,
    );
    send_before_dispatch(session, thread_start, wire.deadline_at).await?;
    let response = await_handshake_response(session, wire, THREAD_REQUEST_ID).await?;
    let thread_id = response_error_or_thread(response).map_err(Decided::Failed)?;
    let turn_bytes = encode_request(
        TURN_REQUEST_ID,
        TURN_START_METHOD,
        turn_start_params(&thread_id, context.instruction, context.image_paths),
        bound,
    )
    .map_err(|error| Decided::Failed(CodexError::Protocol(error)))?;
    // A failed or partial turn write may already have reached the server.
    if send_bounded(session, &turn_bytes, wire.deadline_at)
        .await
        .is_err()
    {
        return Err(lost_output());
    }
    await_turn_completion(session, wire, &thread_id, observed).await
}

/// Sends one handshake message; nothing is dispatched yet, so every failure is
/// a typed refusal rather than uncertainty.
async fn send_before_dispatch<S>(
    session: &mut S,
    encoded: Result<Vec<u8>, ProtocolError>,
    deadline_at: Instant,
) -> Result<(), Decided>
where
    S: ProcessSession,
{
    let bytes = encoded.map_err(|error| Decided::Failed(CodexError::Protocol(error)))?;
    send_bounded(session, &bytes, deadline_at)
        .await
        .map_err(|error| Decided::Failed(map_handshake_stream_error(error)))
}

/// Reads until the response to `request_id` arrives. Before `turn/start`
/// nothing this adapter owns is in flight: the server never sends readiness,
/// and a started turn belongs to no dispatch of this adapter, so
/// notifications are ignored.
async fn await_handshake_response<S>(
    session: &mut S,
    wire: &mut Wire,
    request_id: u64,
) -> Result<ResponseView, Decided>
where
    S: ProcessSession,
{
    loop {
        let line = match read_event(session, wire).await {
            WireEvent::Line(line) => line,
            WireEvent::Eof => return Err(Decided::Failed(CodexError::LostOutput)),
            WireEvent::StreamFailed(error) => {
                return Err(Decided::Failed(map_handshake_stream_error(error)));
            }
            WireEvent::Cancelled => return Err(Decided::Unknown(UnknownReason::Cancelled)),
            WireEvent::Expired => return Err(Decided::Unknown(UnknownReason::Deadline)),
        };
        if !count_line(wire) {
            return Err(Decided::Failed(CodexError::Protocol(
                ProtocolError::TooManyMessages,
            )));
        }
        match parse_line(&line) {
            Ok(Inbound::Response(response)) if is_response_to(&response, request_id) => {
                return Ok(response);
            }
            Ok(Inbound::Response(response)) => {
                return Err(Decided::Failed(CodexError::Protocol(
                    ProtocolError::UnexpectedResponse {
                        id: response_id_text(&response),
                    },
                )));
            }
            Ok(Inbound::Notification { .. }) => {}
            Err(error) => return Err(Decided::Failed(CodexError::Protocol(error))),
        }
    }
}

/// Reads after dispatch until the correlated turn completes. The single
/// correlated turn identity for this dispatch is bound by the first evidence
/// and never moves: response and started evidence must agree with it, and a
/// completion for anything else cannot become Ready.
async fn await_turn_completion<S>(
    session: &mut S,
    wire: &mut Wire,
    thread_id: &str,
    observed: &mut ObservedUsage,
) -> Result<TurnCompletedView, Decided>
where
    S: ProcessSession,
{
    let mut correlated_turn: Option<String> = None;
    // Completed image items arrive as `item/completed` notifications for the bound turn; the
    // turn payload may not repeat them, so they are collected here and merged at completion.
    let mut completed_images: Vec<ImageItem> = Vec::new();
    loop {
        let line = match read_event(session, wire).await {
            WireEvent::Line(line) => line,
            WireEvent::Eof | WireEvent::StreamFailed(_) => return Err(lost_output()),
            WireEvent::Cancelled => {
                interrupt_best_effort(session, thread_id, correlated_turn.as_deref()).await;
                return Err(Decided::Unknown(UnknownReason::Cancelled));
            }
            WireEvent::Expired => return Err(Decided::Unknown(UnknownReason::Deadline)),
        };
        if !count_line(wire) {
            return Err(lost_output());
        }
        let Ok(inbound) = parse_line(&line) else {
            return Err(lost_output());
        };
        let (method, params) = match inbound {
            Inbound::Response(response) => {
                bind_turn_response(response, &mut correlated_turn)?;
                continue;
            }
            Inbound::Notification { method, params } => (method, params),
        };
        if method == TURN_STARTED_METHOD {
            bind_turn_started(&params, thread_id, &mut correlated_turn)?;
        } else if method == ITEM_COMPLETED_METHOD {
            collect_completed_image(
                &params,
                thread_id,
                correlated_turn.as_deref(),
                &mut completed_images,
            )?;
        } else if method == TURN_COMPLETED_METHOD {
            observed.merge(correlated_turn_usage(
                &params,
                thread_id,
                correlated_turn.as_deref(),
            ));
            let view = correlated_completion(
                &params,
                thread_id,
                correlated_turn.as_deref(),
                completed_images,
            )?;
            observed.merge(view.usage);
            return Ok(view);
        } else if is_usage_notification(&method, &params)
            && let Ok(update) = extract_token_usage_update(&params)
            && update.thread_id == thread_id
            && correlated_turn
                .as_deref()
                .is_some_and(|want| want == update.turn_id)
        {
            observed.merge(update.usage);
        }
    }
}

/// Applies the `turn/start` response: a correlated error is a genuine refusal
/// (nothing was dispatched); any other mismatch is uncertainty.
fn bind_turn_response(
    response: ResponseView,
    correlated_turn: &mut Option<String>,
) -> Result<(), Decided> {
    if !is_response_to(&response, TURN_REQUEST_ID) {
        return Err(lost_output());
    }
    if let Some(error) = response.error {
        return Err(Decided::Failed(CodexError::Refused { code: error.code }));
    }
    // turn/start was sent, so a missing or malformed ack may already have
    // started remote work: uncertainty, not failure.
    let Some(Ok(turn_id)) = response.result.as_ref().map(extract_turn_id_from_start) else {
        return Err(lost_output());
    };
    if !bind_turn_id(correlated_turn, turn_id) {
        return Err(lost_output());
    }
    Ok(())
}

fn bind_turn_started(
    params: &Value,
    thread_id: &str,
    correlated_turn: &mut Option<String>,
) -> Result<(), Decided> {
    let Ok((started_thread, turn_id)) = extract_turn_started(params) else {
        return Err(lost_output());
    };
    if started_thread != thread_id || !bind_turn_id(correlated_turn, turn_id) {
        return Err(lost_output());
    }
    Ok(())
}

/// Keeps a completed image item for the bound turn; items for anything else
/// are ignored, and a malformed image item for the bound turn is uncertainty.
fn collect_completed_image(
    params: &Value,
    thread_id: &str,
    bound_turn: Option<&str>,
    completed_images: &mut Vec<ImageItem>,
) -> Result<(), Decided> {
    let Some(bound_turn) = bound_turn else {
        return Ok(());
    };
    let text = |key: &str| params.get(key).and_then(Value::as_str);
    let item = params.get("item");
    let is_image = item
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str)
        == Some(IMAGE_GENERATION_ITEM_TYPE);
    if text("turnId") != Some(bound_turn) || text("threadId") != Some(thread_id) || !is_image {
        return Ok(());
    }
    match item.map(parse_image_item) {
        Some(Ok(image)) => {
            completed_images.push(image);
            Ok(())
        }
        _ => Err(lost_output()),
    }
}

/// Parses the terminal turn and merges collected image items into it. Ready
/// requires a supported start/response correlation: without a bound turn id,
/// or for a different one, the outcome stays unknown.
fn correlated_completion(
    params: &Value,
    thread_id: &str,
    bound_turn: Option<&str>,
    completed_images: Vec<ImageItem>,
) -> Result<TurnCompletedView, Decided> {
    let Ok(mut view) = extract_turn_completed(params) else {
        return Err(lost_output());
    };
    for image in completed_images {
        if !view.items.iter().any(|known| known.id == image.id) {
            view.items.push(image);
        }
    }
    if view.thread_id != thread_id || bound_turn != Some(view.turn_id.as_str()) {
        return Err(lost_output());
    }
    Ok(view)
}

fn lost_output() -> Decided {
    Decided::Unknown(UnknownReason::LostOutput)
}

/// An outcome decided before settlement. It is never `Ready`, so it stays
/// small on every early return; usage is merged once by `drive`.
enum Decided {
    Failed(CodexError),
    Unknown(UnknownReason),
}

impl Decided {
    fn into_outcome(self) -> CodexOutcome {
        match self {
            Self::Failed(error) => CodexOutcome::failed(error),
            Self::Unknown(reason) => CodexOutcome::unknown(reason),
        }
    }
}

/// Counts one server line against the flood bound; false once it is exceeded.
fn count_line(wire: &mut Wire) -> bool {
    wire.lines_seen += 1;
    wire.lines_seen <= MAXIMUM_RESPONSE_LINES
}

fn is_response_to(response: &ResponseView, request_id: u64) -> bool {
    matches!(response.id, RpcId::Number(id) if u64::try_from(id) == Ok(request_id))
}

fn with_usage(outcome: CodexOutcome, observed: ObservedUsage) -> CodexOutcome {
    let merged = |usage: ObservedUsage| {
        let mut merged = observed;
        merged.merge(usage);
        merged
    };
    match outcome {
        CodexOutcome::Ready { receipt, usage } => CodexOutcome::Ready {
            receipt,
            usage: merged(usage),
        },
        CodexOutcome::Failed { error, usage } => CodexOutcome::Failed {
            error,
            usage: merged(usage),
        },
        CodexOutcome::Unknown { reason, usage } => CodexOutcome::Unknown {
            reason,
            usage: merged(usage),
        },
    }
}

async fn send_bounded<S>(
    session: &mut S,
    bytes: &[u8],
    deadline_at: Instant,
) -> Result<(), ProcessError>
where
    S: ProcessSession,
{
    match tokio::time::timeout_at(deadline_at.into(), session.send(bytes)).await {
        Ok(result) => result,
        Err(_) => Err(ProcessError::StreamFailed {
            kind: std::io::ErrorKind::TimedOut,
        }),
    }
}

/// Binds one correlated turn identity for the dispatch: the first id wins and
/// never moves, so a conflicting later id reports disagreement instead of
/// silently rebinding the outcome.
fn bind_turn_id(bound: &mut Option<String>, candidate: String) -> bool {
    match bound {
        Some(existing) => *existing == candidate,
        None => {
            *bound = Some(candidate);
            true
        }
    }
}

/// Asks the server to stop the bound turn before the owned tree kill; without
/// a bound turn there is nothing to name, so nothing is sent.
pub(crate) async fn interrupt_best_effort<S>(
    session: &mut S,
    thread_id: &str,
    turn_id: Option<&str>,
) where
    S: ProcessSession,
{
    let Some(turn_id) = turn_id else {
        return;
    };
    let Ok(bytes) = encode_request(
        INTERRUPT_REQUEST_ID,
        TURN_INTERRUPT_METHOD,
        turn_interrupt_params(thread_id, turn_id),
        INTERRUPT_MESSAGE_BYTES,
    ) else {
        return;
    };
    // Best effort only: a blocked stdin pipe must not delay the owned tree
    // kill that follows every cancellation path.
    let _ = tokio::time::timeout(INTERRUPT_SEND_GRACE, session.send(&bytes)).await;
}

/// Maps a stream failure before `turn/start`: nothing was dispatched, so a
/// broken pipe is a transport failure rather than lost output.
fn map_handshake_stream_error(error: ProcessError) -> CodexError {
    match error {
        ProcessError::Configuration => CodexError::Configuration(CodexConfigError::InvalidField {
            field: "process",
            reason: "the child session was not drivable",
        }),
        ProcessError::SpawnFailed { kind } | ProcessError::StreamFailed { kind } => {
            CodexError::Transport { kind }
        }
        ProcessError::OutputTooLarge => CodexError::LostOutput,
    }
}

async fn read_event<S>(session: &mut S, wire: &mut Wire) -> WireEvent
where
    S: ProcessSession,
{
    let cancel = &mut wire.cancel;
    let cancel_fut = async {
        match cancel {
            Some(receiver) => {
                // A dropped sender withdraws interest exactly like a sent cancel.
                let _ = receiver.await;
            }
            None => std::future::pending().await,
        }
    };
    let deadline_tokio: tokio::time::Instant = wire.deadline_at.into();
    tokio::select! {
        biased;
        _ = cancel_fut => WireEvent::Cancelled,
        _ = tokio::time::sleep_until(deadline_tokio) => WireEvent::Expired,
        line = session.next_line(wire.max_line_bytes) => match line {
            Ok(Some(bytes)) => WireEvent::Line(bytes),
            Ok(None) => WireEvent::Eof,
            Err(error) => WireEvent::StreamFailed(error),
        },
    }
}

fn correlated_turn_usage(params: &Value, thread_id: &str, turn_id: Option<&str>) -> ObservedUsage {
    let Some(turn_id) = turn_id else {
        return ObservedUsage::unknown();
    };
    let Some(turn) = params.get("turn") else {
        return ObservedUsage::unknown();
    };
    if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
        || turn.get("id").and_then(Value::as_str) != Some(turn_id)
    {
        return ObservedUsage::unknown();
    }
    observe_usage(turn)
}

fn is_usage_notification(method: &str, params: &Value) -> bool {
    method == TOKEN_USAGE_UPDATED_METHOD || params.get("tokenUsage").is_some()
}

fn response_error_or_thread(response: ResponseView) -> Result<String, CodexError> {
    if let Some(error) = response.error {
        return Err(CodexError::Refused { code: error.code });
    }
    match response.result {
        Some(result) => extract_thread_id(&result).map_err(CodexError::Protocol),
        None => Err(CodexError::Protocol(ProtocolError::MissingThreadId)),
    }
}

fn response_error_or_initialized(response: ResponseView) -> Result<(), CodexError> {
    if let Some(error) = response.error {
        return Err(CodexError::Refused { code: error.code });
    }
    match response.result {
        Some(result) => extract_initialize_result(&result).map_err(CodexError::Protocol),
        None => Err(CodexError::Protocol(ProtocolError::MissingField {
            field: "result",
        })),
    }
}

fn response_id_text(response: &ResponseView) -> String {
    match &response.id {
        RpcId::Number(id) => id.to_string(),
        RpcId::Text(id) => id.clone(),
    }
}
