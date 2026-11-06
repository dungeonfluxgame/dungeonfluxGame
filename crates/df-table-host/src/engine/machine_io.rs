//! `MachineIntent` in, `MachineReceipt` out: the input side of the wire contract over the owner,
//! and the dev forms of both sides (O.16 H3).
//!
//! `EngineHandle::machine_intent` is what a gRPC `SubmitMachineIntent` handler calls once the call
//! has identified its seat from its credentials (the request carries none): it admits the request
//! with `df_api::machine::admit_machine_intent`, turns the intent into the same input a typed
//! command makes (a tap on an offer is a tap, a line is a typed or spoken line the interpreter
//! reads, the safety card is the pause), submits it through the real owner queue under the
//! operation id the client minted (all 128 bits, so a retry is a duplicate and one effect), and
//! answers with the one receipt of the four outcomes (RES16), checked by `check_machine_response`
//! before it leaves. It never reads the words of a line: the interpreter does, behind the pipeline.
//!
//! What is not carried over yet, each answered typed and never silently: an answer to an open
//! pending (`PendingAnswer`) is declined `capability_unavailable` (no machine of the first path
//! opens a pending the phone must answer), and a decline's alternatives and missing piece are in
//! the seat's next view and not in the receipt, whose reason is the line the seat was shown. An
//! intent aimed at a machine other than the one running (a hand-off replaced it) is declined
//! `window_closed`, which is the wire's stale offer, by the owner, before the input is admitted.
//!
//! What is not a receipt is a typed `WireIntentError` and never a decline: a full lane (a stall,
//! RES22) and an input that waits in the queue with nothing run for it are UNAVAILABLE and are
//! retried under the same operation id; a seat the table does not know and a refusal this file has
//! no answer for are errors of their own, so none is told to a phone as "that cannot be done".
//!
//! The dev forms (`WireViewReport`, `WireIntentBody`, `WireReceiptReport`) are the JSON the dev
//! routes and the command line speak for the same messages: a summary a person reads, and for a
//! view the real protobuf bytes as base64, so a test can decode them and compare.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use df_api::machine::{
    AdmittedMachineIntent, AdmittedPayload, MachineIntentError, MachineReceiptError,
    admit_machine_intent, check_machine_response,
};
use df_protocol::common as wire;
use df_table::identity::{OperationId, RecoveryEpoch, SessionRevision};
use df_table::outcome::{Decline, DeclineCode, DeclineError, OutcomeKind};
use df_table::owner_queue::NotDeclinedCause;
use prost::Message;
use serde::{Deserialize, Serialize};

use crate::machine_wire::{WireDecline, wire_decline, wire_revision};

use super::handle::EngineHandle;
use super::owner::{STATUS_IN_PROGRESS, STATUS_QUEUED};
use super::report::{
    EngineError, RefusalCause, RefusalReport, SubmitReply, TurnReport, WireIntentCause,
};
use super::request::{InputBody, InputRequest, TraceOperation};

/// What a refused wire intent is, so every service answers the same way (RES40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    /// gRPC `INVALID_ARGUMENT`: the client sent something malformed.
    InvalidArgument,
    /// gRPC `UNAVAILABLE`: the owner could not take the call now.
    Unavailable,
    /// gRPC `RESOURCE_EXHAUSTED`: too much, too fast; no message.
    ResourceExhausted,
    /// gRPC `INTERNAL`: the server built an answer it may not send.
    Internal,
}

/// Why a wire intent got no receipt.
#[derive(Debug, Clone, PartialEq)]
pub enum WireIntentError {
    Refused(MachineIntentError),
    Engine(EngineError),
    /// The receipt the owner built broke a wire rule.
    Receipt(MachineReceiptError),
    /// The decline is of the class that is a call failure and not a message (`rate_limited` and
    /// `table_busy`, RES40); `retry_after_ms` is the pause the queue asked for, when it did.
    TooFast {
        retry_after_ms: Option<u64>,
    },
    /// A lane is full: a stall and not a decline (RES22). Retry the same operation id later.
    Stalled,
    /// The seat the call's credentials name is not at this table.
    UnknownSeat {
        seat: u64,
    },
    /// The input waits in the queue (the owner is holding, or it is queued) and nothing has run
    /// for it yet, so there is no receipt. Retry the same operation id.
    NotServedYet,
    /// The queue refused for a cause this file gives no answer for: a repeat, which is answered
    /// before `refused` is asked.
    UnknownRefusal {
        cause: RefusalCause,
    },
    /// A declined turn carries no decline code, so there is no code to tell the phone.
    MissingDeclineCode,
    /// The line a decline carries cannot make a decline.
    Decline(DeclineError),
}

impl WireIntentError {
    pub fn status(&self) -> CallStatus {
        match self {
            Self::Refused(_) => CallStatus::InvalidArgument,
            Self::TooFast { .. } => CallStatus::ResourceExhausted,
            Self::UnknownSeat { .. } => CallStatus::InvalidArgument,
            Self::Stalled
            | Self::NotServedYet
            | Self::Engine(EngineError::MailboxFull | EngineError::Stopped) => {
                CallStatus::Unavailable
            }
            Self::Engine(_)
            | Self::Receipt(_)
            | Self::UnknownRefusal { .. }
            | Self::MissingDeclineCode
            | Self::Decline(_) => CallStatus::Internal,
        }
    }
}

impl fmt::Display for WireIntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => {
                write!(formatter, "the intent was refused at the edge: {error:?}")
            }
            Self::Engine(error) => write!(formatter, "the owner could not take it: {error:?}"),
            Self::Receipt(error) => write!(formatter, "the receipt broke a wire rule: {error:?}"),
            Self::TooFast {
                retry_after_ms: Some(retry_after_ms),
            } => write!(
                formatter,
                "too many inputs too fast; retry in {retry_after_ms} ms"
            ),
            Self::TooFast {
                retry_after_ms: None,
            } => formatter.write_str("too many inputs too fast"),
            Self::Stalled => formatter.write_str("a lane is full; retry the same operation later"),
            Self::UnknownSeat { seat } => write!(formatter, "seat {seat} is not at this table"),
            Self::NotServedYet => {
                formatter.write_str("the input waits in the queue; retry the same operation id")
            }
            Self::UnknownRefusal { cause } => {
                write!(
                    formatter,
                    "the queue refused with the cause '{}'",
                    cause.key()
                )
            }
            Self::MissingDeclineCode => formatter.write_str("a declined turn has no decline code"),
            Self::Decline(error) => write!(formatter, "the decline cannot be built: {error}"),
        }
    }
}

impl std::error::Error for WireIntentError {}

impl From<WireIntentError> for EngineError {
    fn from(error: WireIntentError) -> Self {
        match error {
            WireIntentError::Engine(error) => error,
            WireIntentError::TooFast { retry_after_ms } => EngineError::TooFast { retry_after_ms },
            WireIntentError::Stalled => EngineError::Stalled,
            WireIntentError::NotServedYet => EngineError::NotServedYet,
            WireIntentError::UnknownSeat { seat } => EngineError::UnknownSeat { seat },
            WireIntentError::UnknownRefusal { cause } => EngineError::WireIntent {
                cause: WireIntentCause::UnknownRefusal,
                detail: Some(cause.key().to_owned()),
            },
            WireIntentError::MissingDeclineCode => EngineError::WireIntent {
                cause: WireIntentCause::MissingDeclineCode,
                detail: None,
            },
            WireIntentError::Decline(error) => EngineError::WireIntent {
                cause: WireIntentCause::DeclineInvalid,
                detail: Some(error.to_string()),
            },
            WireIntentError::Refused(error) => EngineError::WireIntent {
                cause: WireIntentCause::RefusedAtTheEdge,
                detail: Some(format!("{error:?}")),
            },
            WireIntentError::Receipt(error) => EngineError::WireIntent {
                cause: WireIntentCause::ReceiptInvalid,
                detail: Some(format!("{error:?}")),
            },
        }
    }
}

/// The input an admitted intent stands for, or `None` for a kind this table cannot take yet.
fn input_for(seat: u64, intent: &AdmittedMachineIntent) -> Option<InputBody> {
    match &intent.payload {
        AdmittedPayload::Tap { offer_id } => Some(InputBody::Tap {
            seat,
            action: offer_id.clone(),
        }),
        AdmittedPayload::SubmitText(line) => Some(InputBody::Text {
            seat,
            text: line.text.clone(),
        }),
        AdmittedPayload::Say(line) => Some(InputBody::Say {
            seat,
            text: line.text.clone(),
        }),
        AdmittedPayload::SafetyCard => Some(InputBody::Pause { seat }),
        AdmittedPayload::PendingAnswer { .. } => None,
    }
}

fn utterance_of(payload: &AdmittedPayload) -> Option<u64> {
    match payload {
        AdmittedPayload::SubmitText(line) | AdmittedPayload::Say(line) => Some(line.utterance_id),
        AdmittedPayload::Tap { .. }
        | AdmittedPayload::PendingAnswer { .. }
        | AdmittedPayload::SafetyCard => None,
    }
}

fn receipt_for(
    intent: &AdmittedMachineIntent,
    sequence: u64,
    replayed: bool,
    result: wire::machine_receipt::Result,
) -> wire::SubmitMachineIntentResponse {
    let revision = wire_revision(SessionRevision::new(RecoveryEpoch::FIRST, sequence));
    wire::SubmitMachineIntentResponse {
        outcome: Some(wire::submit_machine_intent_response::Outcome::Receipt(
            wire::MachineReceipt {
                operation_id: Some(wire::OperationId {
                    value: Some(intent.operation.as_bytes().to_vec()),
                }),
                revision: Some(revision),
                replayed,
                utterance_id: utterance_of(&intent.payload),
                result: Some(result),
            },
        )),
    }
}

/// The wire decline of `code` with the line the seat was shown as its reason. The two codes that
/// mean "too much, too fast" are not a receipt (RES40): they are `TooFast` with the pause the
/// queue asked for.
fn declined(
    code: DeclineCode,
    reason: &str,
    retry_after_ms: Option<u64>,
) -> Result<wire::machine_receipt::Result, WireIntentError> {
    let decline = Decline::new(code, reason).map_err(WireIntentError::Decline)?;
    match wire_decline(&decline) {
        WireDecline::Receipt(decline) => Ok(wire::machine_receipt::Result::Declined(decline)),
        WireDecline::ResourceExhausted => Err(WireIntentError::TooFast { retry_after_ms }),
    }
}

/// The answer to a refusal by the queue: a decline receipt for a decline code that is a message,
/// and a typed error for everything else. The match is exhaustive, so a cause the queue gains
/// without an answer here does not compile.
pub(super) fn refused(
    refusal: &RefusalReport,
    seat: u64,
) -> Result<wire::machine_receipt::Result, WireIntentError> {
    match refusal.cause {
        RefusalCause::Declined(code) => declined(code, &refusal.detail, refusal.retry_after_ms),
        RefusalCause::NotDeclined(NotDeclinedCause::LaneStalled) => Err(WireIntentError::Stalled),
        RefusalCause::NotDeclined(NotDeclinedCause::UnknownSeat) => {
            Err(WireIntentError::UnknownSeat { seat })
        }
        RefusalCause::NotDeclined(NotDeclinedCause::Duplicate) => {
            Err(WireIntentError::UnknownRefusal {
                cause: refusal.cause,
            })
        }
    }
}

/// The cause the queue gives a repeat of an operation it already admitted.
const DUPLICATE_CAUSE: RefusalCause = RefusalCause::NotDeclined(NotDeclinedCause::Duplicate);

/// What the owner answered, as the one result of the receipt, and the revision it was answered at.
/// An input that was admitted and has no turn yet (the owner is holding) is not an answer.
fn result_of(
    reply: &SubmitReply,
    observed: u64,
    seat: u64,
) -> Result<(wire::machine_receipt::Result, u64, bool), WireIntentError> {
    if let Some(refusal) = &reply.submission.refusal {
        return Ok((refused(refusal, seat)?, observed, false));
    }
    let Some(turn) = reply.turns.first() else {
        return Err(WireIntentError::NotServedYet);
    };
    result_of_turn(turn, observed).map(|(result, sequence)| (result, sequence, turn.replayed))
}

/// The one result of the receipt for a turn the owner ran, and the revision it ended at.
pub(super) fn result_of_turn(
    turn: &TurnReport,
    observed: u64,
) -> Result<(wire::machine_receipt::Result, u64), WireIntentError> {
    let sequence = turn.revision.unwrap_or(observed);
    let result = match turn.outcome {
        OutcomeKind::Applied => wire::machine_receipt::Result::Applied(wire::MachineApplied {}),
        OutcomeKind::NeedsClarification => {
            wire::machine_receipt::Result::NeedsClarification(wire::MachineClarification {
                question: turn.readout.clone(),
                options: Vec::new(),
            })
        }
        OutcomeKind::Pending => wire::machine_receipt::Result::Pending(wire::MachinePending {
            pending_id: turn.pending,
        }),
        OutcomeKind::Declined => declined(
            turn.decline.ok_or(WireIntentError::MissingDeclineCode)?,
            &turn.readout,
            None,
        )?,
    };
    Ok((result, sequence))
}

/// The answer to a repeat of an operation the owner no longer holds a turn for: the operation is
/// not remembered, which the wire says as an observation and not as a receipt.
pub(super) fn forgotten() -> wire::SubmitMachineIntentResponse {
    wire::SubmitMachineIntentResponse {
        outcome: Some(
            wire::submit_machine_intent_response::Outcome::OperationObservation(
                wire::OperationObservation {
                    code: wire::RejectionCode::OperationExpired as i32,
                },
            ),
        ),
    }
}

impl EngineHandle {
    /// Runs one wire intent for `seat` (which the call's credentials gave) and answers with its
    /// receipt, checked by the wire's guard.
    pub async fn machine_intent(
        &self,
        seat: u64,
        request: &wire::SubmitMachineIntentRequest,
    ) -> Result<wire::SubmitMachineIntentResponse, WireIntentError> {
        let intent = admit_machine_intent(request).map_err(WireIntentError::Refused)?;
        let observed = intent.observed_revision.sequence();
        let response = match input_for(seat, &intent) {
            None => {
                let result = declined(
                    DeclineCode::NotSupportedYet,
                    "That answer is not supported on this table yet.",
                    None,
                )?;
                receipt_for(&intent, observed, false, result)
            }
            Some(input) => {
                let reply = self
                    .submit(InputRequest {
                        input,
                        operation: None,
                        operation_id: Some(u128::from_be_bytes(*intent.operation.as_bytes())),
                        view: Some(observed),
                        machine: Some(intent.machine_id.clone()),
                    })
                    .await
                    .map_err(WireIntentError::Engine)?;
                let repeat = reply
                    .submission
                    .refusal
                    .as_ref()
                    .filter(|refusal| refusal.cause == DUPLICATE_CAUSE);
                if let Some(first) = repeat {
                    self.repeat_answer(seat, &intent, observed, first).await?
                } else {
                    let (result, sequence, replayed) = result_of(&reply, observed, seat)?;
                    receipt_for(&intent, sequence, replayed, result)
                }
            }
        };
        check_machine_response(&response, &intent).map_err(WireIntentError::Receipt)?;
        Ok(response)
    }
}

impl EngineHandle {
    /// The answer to a repeat of an operation the queue already admitted: the receipt of the turn
    /// the first delivery ran (from the trace the owner keeps), marked replayed, so a retry gets
    /// what the first send got and nothing runs twice (H.2). While the first copy still waits or
    /// runs there is nothing to replay yet, which is `NotServedYet` (retry the same id); only when
    /// the owner no longer holds the finished turn is the answer the observation that the
    /// operation is not remembered.
    async fn repeat_answer(
        &self,
        seat: u64,
        intent: &AdmittedMachineIntent,
        observed: u64,
        first: &RefusalReport,
    ) -> Result<wire::SubmitMachineIntentResponse, WireIntentError> {
        let unfinished = first.first_status.as_deref().is_some_and(|status| {
            status.starts_with(STATUS_QUEUED) || status.starts_with(STATUS_IN_PROGRESS)
        });
        if unfinished {
            return Err(WireIntentError::NotServedYet);
        }
        // An admitted operation id is never zero, so the conversion only guards the type change.
        let operation = OperationId::from_bytes(intent.operation.as_bytes())
            .map_err(|_| WireIntentError::Engine(EngineError::ZeroOperation))?;
        let trace = self
            .trace(TraceOperation::from(operation))
            .await
            .map_err(WireIntentError::Engine)?;
        let sender = format!("seat:{seat}");
        let Some(turn) = trace.turns.iter().find(|turn| turn.sender == sender) else {
            return Ok(forgotten());
        };
        let (result, sequence) = result_of_turn(turn, observed)?;
        Ok(receipt_for(intent, sequence, true, result))
    }
}

/// What a dev intent does, in the words of the command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WireIntentKind {
    Tap { offer: String },
    Text { text: String },
    Say { text: String },
    Pause,
}

/// The dev form of a `SubmitMachineIntentRequest`: the seat the call is for, what it does, and
/// optionally the operation id, the machine and the revision (the route fills what is left out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireIntentBody {
    pub seat: u64,
    pub intent: WireIntentKind,
    #[serde(default)]
    pub operation: Option<u64>,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub observed: Option<u64>,
}

/// The identity bytes of a dev session and run.
const DEV_SESSION_BYTE: u8 = 1;
const DEV_RUN_BYTE: u8 = 2;
/// The id of the line a dev text or say intent carries.
const DEV_UTTERANCE: u64 = 1;

impl WireIntentBody {
    /// The request this body stands for, once the route has filled the machine, the revision and
    /// the operation id.
    pub fn request(
        &self,
        machine: &str,
        observed: u64,
        operation: u128,
    ) -> wire::SubmitMachineIntentRequest {
        use wire::machine_intent::Payload;
        let line = |text: &str| wire::MachineText {
            utterance_id: Some(DEV_UTTERANCE),
            text: text.to_owned(),
        };
        let (offer_id, payload) = match &self.intent {
            WireIntentKind::Tap { offer } => {
                (Some(offer.clone()), Payload::Tap(wire::MachineTap {}))
            }
            WireIntentKind::Text { text } => (None, Payload::SubmitText(line(text))),
            WireIntentKind::Say { text } => (None, Payload::Say(line(text))),
            WireIntentKind::Pause => (None, Payload::SafetyCard(wire::MachineSafetyCard {})),
        };
        wire::SubmitMachineIntentRequest {
            session_id: Some(wire::SessionId {
                value: Some(vec![DEV_SESSION_BYTE; 16]),
            }),
            run_id: Some(wire::RunId {
                value: Some(vec![DEV_RUN_BYTE; 16]),
            }),
            observed_revision: Some(wire_revision(SessionRevision::new(
                RecoveryEpoch::FIRST,
                observed,
            ))),
            operation_id: Some(wire::OperationId {
                value: Some(operation.to_be_bytes().to_vec()),
            }),
            intent: Some(wire::MachineIntent {
                machine_id: machine.to_owned(),
                offer_id,
                payload: Some(payload),
            }),
        }
    }
}

/// One offer as the dev report shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireOfferReport {
    pub offer_id: String,
    pub label: String,
    pub unavailable_reason: Option<String>,
}

/// What the audience of a view is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "audience", rename_all = "snake_case")]
pub enum WireAudienceReport {
    SharedScreen {
        place: String,
        speaker: Option<String>,
        caption: String,
        party: Vec<String>,
    },
    Seat {
        hero: String,
        narration: Vec<String>,
        note: Option<String>,
        offers: Vec<WireOfferReport>,
    },
}

/// The dev form of a `MachineView`: what a person reads, and the protobuf bytes the wire carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireViewReport {
    pub revision: u64,
    pub machine: String,
    pub state: String,
    pub status: String,
    pub reason: Option<String>,
    pub readout: String,
    pub audience: WireAudienceReport,
    pub protobuf_bytes: usize,
    pub protobuf_base64: String,
}

fn status_key(status: Option<&wire::MachineStatus>) -> String {
    status
        .and_then(|status| wire::MachineStatusKind::try_from(status.kind).ok())
        .map_or_else(
            || "unknown".to_owned(),
            |kind| {
                kind.as_str_name()
                    .trim_start_matches("MACHINE_STATUS_KIND_")
                    .to_ascii_lowercase()
            },
        )
}

/// The dev report of `view`.
pub fn view_report(view: &wire::MachineView) -> WireViewReport {
    use wire::machine_view::Audience;
    let audience = match &view.audience {
        Some(Audience::SharedScreen(screen)) => WireAudienceReport::SharedScreen {
            place: screen.place.clone(),
            speaker: screen
                .caption
                .as_ref()
                .and_then(|caption| caption.speaker.clone()),
            caption: screen
                .caption
                .as_ref()
                .map(|caption| caption.text.clone())
                .unwrap_or_default(),
            party: screen
                .party
                .iter()
                .map(|member| {
                    format!(
                        "{} {}/{}",
                        member.name, member.hit_points, member.maximum_hit_points
                    )
                })
                .collect(),
        },
        Some(Audience::Seat(seat)) => WireAudienceReport::Seat {
            hero: seat.hero.clone(),
            narration: seat.narration.clone(),
            note: seat.note.as_ref().map(|note| note.text.clone()),
            offers: seat
                .offers
                .iter()
                .map(|offer| WireOfferReport {
                    offer_id: offer.offer_id.clone(),
                    label: offer.label.clone(),
                    unavailable_reason: offer.unavailable_reason.clone(),
                })
                .collect(),
        },
        None => WireAudienceReport::Seat {
            hero: String::new(),
            narration: Vec::new(),
            note: None,
            offers: Vec::new(),
        },
    };
    let bytes = view.encode_to_vec();
    WireViewReport {
        revision: view
            .revision
            .as_ref()
            .and_then(|revision| revision.sequence)
            .unwrap_or_default(),
        machine: view.machine_id.clone(),
        state: view.state_label.clone(),
        status: status_key(view.status.as_ref()),
        reason: view.reason.clone(),
        readout: view.readout.clone(),
        audience,
        protobuf_bytes: bytes.len(),
        protobuf_base64: STANDARD.encode(&bytes),
    }
}

/// The dev form of a `MachineReceipt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireReceiptReport {
    pub operation: String,
    pub revision: u64,
    pub replayed: bool,
    pub utterance: Option<u64>,
    /// `applied`, `needs_clarification`, `declined`, `pending` or `observation`.
    pub result: String,
    pub code: Option<String>,
    pub reason: Option<String>,
    pub pending: Option<u64>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The dev report of a response.
pub fn receipt_report(response: &wire::SubmitMachineIntentResponse) -> WireReceiptReport {
    use wire::machine_receipt::Result as Outcome;
    use wire::submit_machine_intent_response::Outcome as Response;
    let empty = WireReceiptReport {
        operation: String::new(),
        revision: 0,
        replayed: false,
        utterance: None,
        result: "observation".to_owned(),
        code: None,
        reason: None,
        pending: None,
    };
    let receipt = match &response.outcome {
        Some(Response::Receipt(receipt)) => receipt,
        Some(Response::OperationObservation(observation)) => {
            return WireReceiptReport {
                code: wire::RejectionCode::try_from(observation.code)
                    .ok()
                    .map(|code| code.as_str_name().to_ascii_lowercase()),
                ..empty
            };
        }
        None => return empty,
    };
    let (result, code, reason, pending) = match &receipt.result {
        Some(Outcome::Applied(_)) => ("applied", None, None, None),
        Some(Outcome::NeedsClarification(asked)) => (
            "needs_clarification",
            None,
            Some(asked.question.clone()),
            None,
        ),
        Some(Outcome::Declined(decline)) => (
            "declined",
            wire::RejectionCode::try_from(decline.code)
                .ok()
                .map(|code| code.as_str_name().to_ascii_lowercase()),
            Some(decline.reason.clone()),
            None,
        ),
        Some(Outcome::Pending(pending)) => ("pending", None, None, pending.pending_id),
        None => ("observation", None, None, None),
    };
    WireReceiptReport {
        operation: receipt
            .operation_id
            .as_ref()
            .and_then(|id| id.value.as_deref())
            .map(hex)
            .unwrap_or_default(),
        revision: receipt
            .revision
            .as_ref()
            .and_then(|revision| revision.sequence)
            .unwrap_or_default(),
        replayed: receipt.replayed,
        utterance: receipt.utterance_id,
        result: result.to_owned(),
        code,
        reason,
        pending,
    }
}
