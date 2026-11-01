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

use tokio::sync::oneshot;

use super::message_bound;
use super::process::{ProcessError, ProcessSession, StreamLimits};
use super::protocol::{
    INITIALIZE_METHOD, INITIALIZED_METHOD, Inbound, MAXIMUM_RESPONSE_LINES, ObservedUsage,
    ProtocolError, ResponseView, RpcId, TOKEN_USAGE_UPDATED_METHOD, TURN_COMPLETED_METHOD,
    TURN_STARTED_METHOD, encode_notification, encode_request, extract_initialize_result,
    extract_thread_id, extract_token_usage_update, extract_turn_completed,
    extract_turn_id_from_start, extract_turn_started, initialize_params, observe_usage, parse_line,
    thread_start_params, turn_interrupt_params, turn_start_params,
};
use super::protocol::{
    ITEM_COMPLETED_METHOD, ImageItem, THREAD_START_METHOD, TURN_INTERRUPT_METHOD,
    TURN_START_METHOD, parse_image_item,
};
use super::settle::settle_turn;
use super::{
    CodexCliConfig, CodexError, CodexImageRequest, CodexOutcome, ConfigError, UnknownReason,
};

/// Request ids used on one connection: initialize, then thread open, then
/// turn start. Values stay stable so recorded fixtures keep their meaning.
const INITIALIZE_REQUEST_ID: u64 = 0;
const THREAD_REQUEST_ID: u64 = 1;
const TURN_REQUEST_ID: u64 = 2;

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

/// Runs one image turn without retry or fallback after an uncertain dispatch.
pub(super) async fn drive<S>(
    session: &mut S,
    context: TurnContext<'_>,
    mut cancel: Option<oneshot::Receiver<()>>,
) -> CodexOutcome
where
    S: ProcessSession,
{
    let TurnContext {
        config,
        request,
        scratch_text,
        outputs_root,
        image_paths,
        instruction,
        limits,
        deadline_at,
    } = context;
    let mut observed = ObservedUsage::unknown();
    let mut lines_seen: u64 = 0;
    let initialize_bytes = match encode_request(
        INITIALIZE_REQUEST_ID,
        INITIALIZE_METHOD,
        initialize_params(),
        message_bound(config),
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = session.kill().await;
            return with_usage(CodexOutcome::failed(CodexError::Protocol(error)), observed);
        }
    };
    if let Err(error) = send_bounded(session, &initialize_bytes, deadline_at).await {
        let _ = session.kill().await;
        return with_usage(
            CodexOutcome::failed(map_stream_error(error, false)),
            observed,
        );
    }
    loop {
        match read_event(session, &mut cancel, deadline_at, limits.max_line_bytes).await {
            WireEvent::Line(line) => {
                lines_seen += 1;
                if lines_seen > MAXIMUM_RESPONSE_LINES {
                    let _ = session.kill().await;
                    return with_usage(
                        CodexOutcome::failed(CodexError::Protocol(ProtocolError::TooManyMessages)),
                        observed,
                    );
                }
                match parse_line(&line) {
                    Ok(Inbound::Response(response)) => {
                        if response.id == RpcId::Number(INITIALIZE_REQUEST_ID as i64) {
                            match response_error_or_initialized(response) {
                                Ok(()) => break,
                                Err(error) => {
                                    let _ = session.kill().await;
                                    return with_usage(CodexOutcome::failed(error), observed);
                                }
                            }
                        } else {
                            let _ = session.kill().await;
                            return with_usage(
                                CodexOutcome::failed(CodexError::Protocol(
                                    ProtocolError::UnexpectedResponse {
                                        id: response_id_text(&response),
                                    },
                                )),
                                observed,
                            );
                        }
                    }
                    // The server never sends readiness: any notification here
                    // is ignored while the client waits for its own
                    // initialize response.
                    Ok(Inbound::Notification { .. }) => {}
                    Err(error) => {
                        let _ = session.kill().await;
                        return with_usage(
                            CodexOutcome::failed(CodexError::Protocol(error)),
                            observed,
                        );
                    }
                }
            }
            WireEvent::Eof => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::failed(CodexError::LostOutput), observed);
            }
            WireEvent::StreamFailed(error) => {
                let _ = session.kill().await;
                return with_usage(
                    CodexOutcome::failed(map_stream_error(error, false)),
                    observed,
                );
            }
            WireEvent::Cancelled => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::unknown(UnknownReason::Cancelled), observed);
            }
            WireEvent::Expired => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::unknown(UnknownReason::Deadline), observed);
            }
        }
    }
    // Client readiness is a notification the adapter sends, not one it waits
    // for. It travels under the same absolute deadline as every handshake
    // write; cancellation stays observed by the surrounding event loop.
    let initialized_bytes = match encode_notification(
        INITIALIZED_METHOD,
        serde_json::json!({}),
        message_bound(config),
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = session.kill().await;
            return with_usage(CodexOutcome::failed(CodexError::Protocol(error)), observed);
        }
    };
    if let Err(error) = send_bounded(session, &initialized_bytes, deadline_at).await {
        let _ = session.kill().await;
        return with_usage(
            CodexOutcome::failed(map_stream_error(error, false)),
            observed,
        );
    }
    let thread_bytes = match encode_request(
        THREAD_REQUEST_ID,
        THREAD_START_METHOD,
        thread_start_params(scratch_text, config.agent_model()),
        message_bound(config),
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = session.kill().await;
            return with_usage(CodexOutcome::failed(CodexError::Protocol(error)), observed);
        }
    };
    if let Err(error) = send_bounded(session, &thread_bytes, deadline_at).await {
        let _ = session.kill().await;
        return with_usage(
            CodexOutcome::failed(map_stream_error(error, false)),
            observed,
        );
    }
    let thread_id = loop {
        match read_event(session, &mut cancel, deadline_at, limits.max_line_bytes).await {
            WireEvent::Line(line) => {
                lines_seen += 1;
                if lines_seen > MAXIMUM_RESPONSE_LINES {
                    let _ = session.kill().await;
                    return with_usage(
                        CodexOutcome::failed(CodexError::Protocol(ProtocolError::TooManyMessages)),
                        observed,
                    );
                }
                match parse_line(&line) {
                    Ok(Inbound::Response(response)) => {
                        if response.id == RpcId::Number(THREAD_REQUEST_ID as i64) {
                            match response_error_or_thread(response) {
                                Ok(id) => break id,
                                Err(error) => {
                                    let _ = session.kill().await;
                                    return with_usage(CodexOutcome::failed(error), observed);
                                }
                            }
                        } else {
                            let _ = session.kill().await;
                            return with_usage(
                                CodexOutcome::failed(CodexError::Protocol(
                                    ProtocolError::UnexpectedResponse {
                                        id: response_id_text(&response),
                                    },
                                )),
                                observed,
                            );
                        }
                    }
                    // A started turn before this adapter sent turn/start
                    // belongs to no dispatch it owns.
                    Ok(Inbound::Notification { .. }) => {}
                    Err(error) => {
                        let _ = session.kill().await;
                        return with_usage(
                            CodexOutcome::failed(CodexError::Protocol(error)),
                            observed,
                        );
                    }
                }
            }
            WireEvent::Eof => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::failed(CodexError::LostOutput), observed);
            }
            WireEvent::StreamFailed(error) => {
                let _ = session.kill().await;
                return with_usage(
                    CodexOutcome::failed(map_stream_error(error, false)),
                    observed,
                );
            }
            WireEvent::Cancelled => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::unknown(UnknownReason::Cancelled), observed);
            }
            WireEvent::Expired => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::unknown(UnknownReason::Deadline), observed);
            }
        }
    };
    let turn_bytes = match encode_request(
        TURN_REQUEST_ID,
        TURN_START_METHOD,
        turn_start_params(&thread_id, instruction, image_paths),
        message_bound(config),
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = session.kill().await;
            return with_usage(CodexOutcome::failed(CodexError::Protocol(error)), observed);
        }
    };
    if let Err(_error) = send_bounded(session, &turn_bytes, deadline_at).await {
        let _ = session.kill().await;
        return with_usage(CodexOutcome::unknown(UnknownReason::LostOutput), observed);
    }
    // The single correlated turn identity for this dispatch. The first
    // binding wins and never moves: response and started evidence must agree
    // with it, and a completion for anything else cannot become Ready.
    let mut correlated_turn: Option<String> = None;
    // Completed image items arrive as `item/completed` notifications for the bound turn; the
    // turn payload may not repeat them, so they are collected here and merged at completion.
    let mut completed_images: Vec<ImageItem> = Vec::new();
    loop {
        match read_event(session, &mut cancel, deadline_at, limits.max_line_bytes).await {
            WireEvent::Line(line) => {
                lines_seen += 1;
                if lines_seen > MAXIMUM_RESPONSE_LINES {
                    let _ = session.kill().await;
                    return with_usage(CodexOutcome::unknown(UnknownReason::LostOutput), observed);
                }
                match parse_line(&line) {
                    Ok(Inbound::Response(response)) => {
                        if response.id == RpcId::Number(TURN_REQUEST_ID as i64) {
                            if let Some(error) = response.error {
                                // A correlated error answer to turn/start is
                                // a genuine refusal: nothing was dispatched.
                                let _ = session.kill().await;
                                return with_usage(
                                    CodexOutcome::failed(CodexError::Refused { code: error.code }),
                                    observed,
                                );
                            }
                            match response.result {
                                Some(result) => match extract_turn_id_from_start(&result) {
                                    Ok(turn_id) => {
                                        if !bind_turn_id(&mut correlated_turn, turn_id) {
                                            let _ = session.kill().await;
                                            return with_usage(
                                                CodexOutcome::unknown(UnknownReason::LostOutput),
                                                observed,
                                            );
                                        }
                                    }
                                    // turn/start was sent, so a missing or
                                    // malformed ack may already have started
                                    // remote work: uncertainty, not failure.
                                    Err(_) => {
                                        let _ = session.kill().await;
                                        return with_usage(
                                            CodexOutcome::unknown(UnknownReason::LostOutput),
                                            observed,
                                        );
                                    }
                                },
                                None => {
                                    let _ = session.kill().await;
                                    return with_usage(
                                        CodexOutcome::unknown(UnknownReason::LostOutput),
                                        observed,
                                    );
                                }
                            }
                        } else {
                            let _ = session.kill().await;
                            return with_usage(
                                CodexOutcome::unknown(UnknownReason::LostOutput),
                                observed,
                            );
                        }
                    }
                    Ok(Inbound::Notification { method, params }) => {
                        if method == TURN_STARTED_METHOD {
                            match extract_turn_started(&params) {
                                Ok((started_thread, turn_id)) => {
                                    if started_thread != thread_id
                                        || !bind_turn_id(&mut correlated_turn, turn_id)
                                    {
                                        let _ = session.kill().await;
                                        return with_usage(
                                            CodexOutcome::unknown(UnknownReason::LostOutput),
                                            observed,
                                        );
                                    }
                                }
                                Err(_) => {
                                    let _ = session.kill().await;
                                    return with_usage(
                                        CodexOutcome::unknown(UnknownReason::LostOutput),
                                        observed,
                                    );
                                }
                            }
                        } else if method == ITEM_COMPLETED_METHOD {
                            let bound = correlated_turn.as_deref();
                            let for_turn = params.get("turnId").and_then(|value| value.as_str())
                                == bound
                                && params.get("threadId").and_then(|value| value.as_str())
                                    == Some(thread_id.as_str());
                            let is_image = params
                                .get("item")
                                .and_then(|item| item.get("type"))
                                .and_then(|value| value.as_str())
                                == Some("imageGeneration");
                            if bound.is_some() && for_turn && is_image {
                                match params.get("item").map(parse_image_item) {
                                    Some(Ok(item)) => completed_images.push(item),
                                    _ => {
                                        let _ = session.kill().await;
                                        return with_usage(
                                            CodexOutcome::unknown(UnknownReason::LostOutput),
                                            observed,
                                        );
                                    }
                                }
                            }
                        } else if method == TURN_COMPLETED_METHOD {
                            observed.merge(correlated_turn_usage(
                                &params,
                                &thread_id,
                                correlated_turn.as_deref(),
                            ));
                            match extract_turn_completed(&params) {
                                Ok(mut view) => {
                                    for image in completed_images.drain(..) {
                                        if !view.items.iter().any(|known| known.id == image.id) {
                                            view.items.push(image);
                                        }
                                    }
                                    if view.thread_id != thread_id {
                                        let _ = session.kill().await;
                                        return with_usage(
                                            CodexOutcome::unknown(UnknownReason::LostOutput),
                                            observed,
                                        );
                                    }
                                    // Ready requires a supported
                                    // start/response correlation: without a
                                    // bound turn id, or for a different one,
                                    // the outcome stays unknown.
                                    match correlated_turn.as_deref() {
                                        Some(bound) if view.turn_id == bound => {}
                                        _ => {
                                            let _ = session.kill().await;
                                            return with_usage(
                                                CodexOutcome::unknown(UnknownReason::LostOutput),
                                                observed,
                                            );
                                        }
                                    }
                                    observed.merge(view.usage);
                                    // Stop helper access before the final
                                    // import so the verified bytes cannot be
                                    // swapped under the receipt.
                                    let _ = session.kill().await;
                                    let settled = settle_turn(
                                        view,
                                        request,
                                        image_paths,
                                        outputs_root,
                                        config.max_artifact_bytes,
                                        deadline_at,
                                    )
                                    .await;
                                    return with_usage(settled, observed);
                                }
                                Err(_) => {
                                    let _ = session.kill().await;
                                    return with_usage(
                                        CodexOutcome::unknown(UnknownReason::LostOutput),
                                        observed,
                                    );
                                }
                            }
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
                    Err(_) => {
                        let _ = session.kill().await;
                        return with_usage(
                            CodexOutcome::unknown(UnknownReason::LostOutput),
                            observed,
                        );
                    }
                }
            }
            WireEvent::Eof => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::unknown(UnknownReason::LostOutput), observed);
            }
            WireEvent::StreamFailed(_) => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::unknown(UnknownReason::LostOutput), observed);
            }
            WireEvent::Cancelled => {
                let turn_ref = correlated_turn.as_deref();
                interrupt_best_effort(session, &thread_id, &turn_ref).await;
                let _ = session.kill().await;
                return with_usage(CodexOutcome::unknown(UnknownReason::Cancelled), observed);
            }
            WireEvent::Expired => {
                let _ = session.kill().await;
                return with_usage(CodexOutcome::unknown(UnknownReason::Deadline), observed);
            }
        }
    }
}

fn with_usage(outcome: CodexOutcome, observed: ObservedUsage) -> CodexOutcome {
    match outcome {
        CodexOutcome::Ready { receipt, usage } => {
            let mut merged = observed;
            merged.merge(usage);
            CodexOutcome::Ready {
                receipt,
                usage: merged,
            }
        }
        CodexOutcome::Failed { error, usage } => {
            let mut merged = observed;
            merged.merge(usage);
            CodexOutcome::Failed {
                error,
                usage: merged,
            }
        }
        CodexOutcome::Unknown { reason, usage } => {
            let mut merged = observed;
            merged.merge(usage);
            CodexOutcome::Unknown {
                reason,
                usage: merged,
            }
        }
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

pub(crate) async fn interrupt_best_effort<S>(
    session: &mut S,
    thread_id: &str,
    turn_id: &Option<&str>,
) where
    S: ProcessSession,
{
    let Some(turn_id) = turn_id else {
        return;
    };
    let Ok(bytes) = encode_request(
        3,
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

fn map_stream_error(error: ProcessError, dispatched: bool) -> CodexError {
    match error {
        ProcessError::Configuration => CodexError::Configuration(ConfigError::InvalidField {
            field: "process",
            reason: "the child session was not drivable",
        }),
        ProcessError::SpawnFailed { kind } => CodexError::Transport { kind },
        ProcessError::StreamFailed { kind } => {
            if dispatched {
                CodexError::LostOutput
            } else {
                CodexError::Transport { kind }
            }
        }
        ProcessError::OutputTooLarge => CodexError::LostOutput,
    }
}

async fn read_event<S>(
    session: &mut S,
    cancel: &mut Option<oneshot::Receiver<()>>,
    deadline_at: Instant,
    max_line_bytes: usize,
) -> WireEvent
where
    S: ProcessSession,
{
    let cancel_fut = async {
        match cancel {
            Some(receiver) => {
                let _ = receiver.await;
            }
            None => std::future::pending().await,
        }
    };
    let deadline_tokio: tokio::time::Instant = deadline_at.into();
    tokio::select! {
        biased;
        _ = cancel_fut => WireEvent::Cancelled,
        _ = tokio::time::sleep_until(deadline_tokio) => WireEvent::Expired,
        line = session.next_line(max_line_bytes) => match line {
            Ok(Some(bytes)) => WireEvent::Line(bytes),
            Ok(None) => WireEvent::Eof,
            Err(error) => WireEvent::StreamFailed(error),
        },
    }
}

fn correlated_turn_usage(
    params: &serde_json::Value,
    thread_id: &str,
    turn_id: Option<&str>,
) -> ObservedUsage {
    let Some(turn_id) = turn_id else {
        return ObservedUsage::unknown();
    };
    let Some(turn) = params.get("turn") else {
        return ObservedUsage::unknown();
    };
    if params.get("threadId").and_then(serde_json::Value::as_str) != Some(thread_id)
        || turn.get("id").and_then(serde_json::Value::as_str) != Some(turn_id)
    {
        return ObservedUsage::unknown();
    }
    observe_usage(turn)
}

fn is_usage_notification(method: &str, params: &serde_json::Value) -> bool {
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
