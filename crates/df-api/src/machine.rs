//! The wire surface of a table machine: what a server may send and what it may accept.
//!
//! It owns three guards over the generated messages of `df-protocol` (plan O.16 W0, RES33,
//! RES40): `check_machine_view`, which the server runs on a view before it leaves and a client may
//! run on one it received, `check_machine_receipt`, which the server runs on its answer to an
//! intent it admitted, and `admit_machine_intent`, which turns a request into typed values or one
//! named refusal. Each refusal says which RPC status class it is (`rpc_status`), so a service
//! does not invent its own mapping. It also holds a hand-built
//! sample of each message (`sample`), the stand-in for a real projection until the seam below
//! is filled.
//!
//! The guards enforce what a protobuf type cannot say: a view is never empty (a plain `readout`,
//! a known status, non-blank text in every slot that is present), a status other than ok has a
//! reason, a waiting status names who and what, a message is no larger than the bridge can carry,
//! an offer lives only in a seat's view, and an intent carries exactly the fields of its kind. They never authenticate a seat, check that an offer is still legal, or look an operation
//! up: the session owner does all three after admission.
//!
//! The seam to `df-table` (O.8). The wire types here do not depend on `df-table`, and neither does
//! this crate: `df-table-host::machine_wire::machine_view` is the one function that knows both
//! (it copies an `AudienceProjection` into a `MachineView` and ends with `check_machine_view`, so
//! a projection that broke the never-empty rule is a typed error and not a blank screen). A
//! decline reaches the wire through `DeclineCode::public_class` mapped to `RejectionCode`, with a
//! class of `InternalReasonKey` as `REJECTION_CODE_UNSPECIFIED` and its reason text, and
//! `ResourceExhausted` as a `RESOURCE_EXHAUSTED` call failure and not a message. The reverse
//! direction needs no seam: `admit_machine_intent` yields plain values the owner turns into an
//! `InputEnvelope`.

use std::collections::BTreeSet;
use std::fmt;

use prost::Message;

use df_protocol::common as wire;
use df_protocol::common::machine_intent::Payload;
use df_protocol::common::machine_view::Audience;
use df_types::{OperationId, RunId, SessionId, SessionRevision};

use crate::request::{self, RequestError};

/// The largest encoded view, receipt or request this module lets through, in bytes. It sits below
/// the 65,536 byte message limit of the RPC bridge (`RPC_MESSAGE_BYTES` in `df-rpc-bridge`, which
/// this crate cannot name; a test in that crate pins the order), so a message that passes here
/// always fits the call that carries it, and one that does not is a typed refusal and not an
/// opaque transport error in the middle of a stream.
pub const MAXIMUM_MESSAGE_BYTES: usize = 60 * 1024;

/// The longest typed or spoken line an intent may carry, in bytes. The request around it also
/// holds identities and framing, so it is checked against `MAXIMUM_MESSAGE_BYTES` too.
pub const MAXIMUM_INTENT_TEXT_BYTES: usize = 56 * 1024;

/// Names a slot of a view without holding what was in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewSlot {
    Revision,
    MachineId,
    StateLabel,
    Status,
    WaitingWho,
    WaitingWhat,
    Reason,
    Readout,
    Audience,
    OfferId,
    OfferLabel,
    OfferExplain,
    UnavailableReason,
    OfferGroup,
    Place,
    CaptionSpeaker,
    CaptionText,
    MediaName,
    MediaCaption,
    PartyName,
    Condition,
    Hero,
    Goal,
    NarrationLine,
    NoteText,
}

/// Why a view may not be sent or shown. No variant holds the text that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineViewError {
    /// A required slot is absent.
    Missing(ViewSlot),
    /// A slot that is present holds blank text.
    Blank(ViewSlot),
    /// A slot is set that this status or audience does not allow.
    Unexpected(ViewSlot),
    /// The status number is not one this build knows (a newer server).
    UnknownStatus(i32),
    /// Two offers carry the same id, so a tap could not name one of them.
    DuplicateOffer,
    Revision(RequestError),
    /// The encoded view is longer than `MAXIMUM_MESSAGE_BYTES`.
    TooLarge {
        bytes: usize,
    },
}

fn blank(text: &str) -> bool {
    text.trim().is_empty()
}

fn required(text: &str, slot: ViewSlot) -> Result<(), MachineViewError> {
    if blank(text) {
        Err(MachineViewError::Blank(slot))
    } else {
        Ok(())
    }
}

fn present(text: Option<&str>, slot: ViewSlot) -> Result<(), MachineViewError> {
    match text {
        Some(text) => required(text, slot),
        None => Ok(()),
    }
}

fn check_image(image: Option<&wire::MachineImage>) -> Result<(), MachineViewError> {
    let Some(image) = image else {
        return Ok(());
    };
    required(&image.media, ViewSlot::MediaName)?;
    present(image.caption.as_deref(), ViewSlot::MediaCaption)
}

fn check_offer(offer: &wire::MachineOffer) -> Result<(), MachineViewError> {
    required(&offer.offer_id, ViewSlot::OfferId)?;
    required(&offer.label, ViewSlot::OfferLabel)?;
    present(offer.explain.as_deref(), ViewSlot::OfferExplain)?;
    present(
        offer.unavailable_reason.as_deref(),
        ViewSlot::UnavailableReason,
    )?;
    present(offer.group.as_deref(), ViewSlot::OfferGroup)
}

/// Every offer is readable and no two share an id.
fn check_offers(
    offers: &[wire::MachineOffer],
    duplicate: MachineViewError,
) -> Result<(), MachineViewError> {
    let mut offer_ids = BTreeSet::new();
    for offer in offers {
        check_offer(offer)?;
        if !offer_ids.insert(offer.offer_id.as_str()) {
            return Err(duplicate);
        }
    }
    Ok(())
}

fn check_status(
    status: Option<&wire::MachineStatus>,
    reason: Option<&str>,
) -> Result<(), MachineViewError> {
    let status = status.ok_or(MachineViewError::Missing(ViewSlot::Status))?;
    let kind = wire::MachineStatusKind::try_from(status.kind)
        .map_err(|_| MachineViewError::UnknownStatus(status.kind))?;
    let waiting = kind == wire::MachineStatusKind::WaitingFor;
    match kind {
        wire::MachineStatusKind::Unspecified => {
            return Err(MachineViewError::Missing(ViewSlot::Status));
        }
        wire::MachineStatusKind::Ok => {
            if reason.is_some() {
                return Err(MachineViewError::Unexpected(ViewSlot::Reason));
            }
        }
        _ => match reason {
            None => return Err(MachineViewError::Missing(ViewSlot::Reason)),
            Some(text) => required(text, ViewSlot::Reason)?,
        },
    }
    match (waiting, &status.waiting_who, &status.waiting_what) {
        (true, Some(who), Some(what)) => {
            required(who, ViewSlot::WaitingWho)?;
            required(what, ViewSlot::WaitingWhat)
        }
        (true, None, _) => Err(MachineViewError::Missing(ViewSlot::WaitingWho)),
        (true, _, None) => Err(MachineViewError::Missing(ViewSlot::WaitingWhat)),
        (false, Some(_), _) => Err(MachineViewError::Unexpected(ViewSlot::WaitingWho)),
        (false, _, Some(_)) => Err(MachineViewError::Unexpected(ViewSlot::WaitingWhat)),
        (false, None, None) => Ok(()),
    }
}

fn check_shared_screen(screen: &wire::MachineSharedScreen) -> Result<(), MachineViewError> {
    required(&screen.place, ViewSlot::Place)?;
    if let Some(caption) = &screen.caption {
        present(caption.speaker.as_deref(), ViewSlot::CaptionSpeaker)?;
        required(&caption.text, ViewSlot::CaptionText)?;
    }
    check_image(screen.backdrop.as_ref())?;
    check_image(screen.card.as_ref())?;
    for member in &screen.party {
        required(&member.name, ViewSlot::PartyName)?;
        for condition in &member.conditions {
            required(condition, ViewSlot::Condition)?;
        }
    }
    Ok(())
}

fn check_seat(seat: &wire::MachineSeatView) -> Result<(), MachineViewError> {
    required(&seat.hero, ViewSlot::Hero)?;
    present(seat.goal.as_deref(), ViewSlot::Goal)?;
    for line in &seat.narration {
        required(line, ViewSlot::NarrationLine)?;
    }
    if let Some(note) = &seat.note {
        required(&note.text, ViewSlot::NoteText)?;
    }
    check_image(seat.image.as_ref())?;
    check_offers(&seat.offers, MachineViewError::DuplicateOffer)
}

/// Checks a view against the rules a protobuf type cannot state. `Ok` means every text slot that
/// is present reads as text, the readout is not blank, the status is known and explained, and the
/// view names its audience.
pub fn check_machine_view(view: &wire::MachineView) -> Result<(), MachineViewError> {
    let bytes = view.encoded_len();
    if bytes > MAXIMUM_MESSAGE_BYTES {
        return Err(MachineViewError::TooLarge { bytes });
    }
    request::session_revision(view.revision.as_ref()).map_err(MachineViewError::Revision)?;
    required(&view.machine_id, ViewSlot::MachineId)?;
    required(&view.state_label, ViewSlot::StateLabel)?;
    required(&view.readout, ViewSlot::Readout)?;
    check_status(view.status.as_ref(), view.reason.as_deref())?;
    match &view.audience {
        Some(Audience::SharedScreen(screen)) => check_shared_screen(screen),
        Some(Audience::Seat(seat)) => check_seat(seat),
        None => Err(MachineViewError::Missing(ViewSlot::Audience)),
    }
}

/// The kind of RPC failure a refusal is, so every service answers the same way. A refusal of what
/// a client sent is the client's fault; a refusal of what the server was about to send is the
/// server's, and the client is told only that (RPC-level failures keep their own status, never a
/// domain code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcStatusClass {
    /// gRPC `INVALID_ARGUMENT`.
    InvalidArgument,
    /// gRPC `INTERNAL`.
    Internal,
}

impl MachineViewError {
    /// A view the server built and cannot send is the server's fault.
    pub fn rpc_status(&self) -> RpcStatusClass {
        RpcStatusClass::Internal
    }
}

/// Names a slot of a receipt without holding what was in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptSlot {
    OperationId,
    Outcome,
    Result,
    PendingId,
    Reason,
    Missing,
    Question,
    ObservationCode,
}

/// Why an answer to an intent may not be sent. No variant holds the text that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineReceiptError {
    Missing(ReceiptSlot),
    Blank(ReceiptSlot),
    /// The receipt names another operation than the one the intent carried.
    OperationMismatch,
    /// The receipt's utterance id is not the intent's: a typed or spoken line is named by its id
    /// and any other kind of intent has none.
    UtteranceMismatch,
    /// A decline code this build does not know (a newer server).
    UnknownCode(i32),
    /// A code that is not a machine decline: only the six classes of RES40 and "no public class"
    /// (unspecified, with its reason text) are.
    NotAMachineDecline(wire::RejectionCode),
    /// An offer inside the answer (an alternative or a clarification option) failed its check.
    Offer(MachineViewError),
    /// Two alternatives or two options share an offer id.
    DuplicateOffer,
    /// The operation id or the revision is absent or malformed.
    Request(RequestError),
    /// The encoded answer is longer than `MAXIMUM_MESSAGE_BYTES`.
    TooLarge {
        bytes: usize,
    },
}

impl MachineReceiptError {
    /// An answer the server built and cannot send is the server's fault.
    pub fn rpc_status(&self) -> RpcStatusClass {
        RpcStatusClass::Internal
    }
}

fn check_decline_code(code: i32) -> Result<(), MachineReceiptError> {
    let known =
        wire::RejectionCode::try_from(code).map_err(|_| MachineReceiptError::UnknownCode(code))?;
    match known {
        wire::RejectionCode::Unspecified
        | wire::RejectionCode::WrongTurn
        | wire::RejectionCode::StaleOffer
        | wire::RejectionCode::InvalidSelection
        | wire::RejectionCode::ResourceMissing
        | wire::RejectionCode::Paused
        | wire::RejectionCode::CapabilityUnavailable => Ok(()),
        wire::RejectionCode::RecoveryRequired
        | wire::RejectionCode::RevisionConflict
        | wire::RejectionCode::OperationConflict
        | wire::RejectionCode::OperationExpired => {
            Err(MachineReceiptError::NotAMachineDecline(known))
        }
    }
}

fn check_offer_list(offers: &[wire::MachineOffer]) -> Result<(), MachineReceiptError> {
    check_offers(offers, MachineViewError::DuplicateOffer).map_err(|error| match error {
        MachineViewError::DuplicateOffer => MachineReceiptError::DuplicateOffer,
        other => MachineReceiptError::Offer(other),
    })
}

fn receipt_text(value: &str, slot: ReceiptSlot) -> Result<(), MachineReceiptError> {
    if blank(value) {
        Err(MachineReceiptError::Blank(slot))
    } else {
        Ok(())
    }
}

fn check_result(result: &wire::machine_receipt::Result) -> Result<(), MachineReceiptError> {
    use wire::machine_receipt::Result as Outcome;
    match result {
        Outcome::Applied(_) => Ok(()),
        Outcome::Pending(pending) => pending
            .pending_id
            .map(|_| ())
            .ok_or(MachineReceiptError::Missing(ReceiptSlot::PendingId)),
        Outcome::NeedsClarification(clarification) => {
            receipt_text(&clarification.question, ReceiptSlot::Question)?;
            check_offer_list(&clarification.options)
        }
        Outcome::Declined(decline) => {
            check_decline_code(decline.code)?;
            receipt_text(&decline.reason, ReceiptSlot::Reason)?;
            if let Some(missing) = &decline.missing {
                receipt_text(missing, ReceiptSlot::Missing)?;
            }
            check_offer_list(&decline.alternatives)
        }
    }
}

/// Checks the answer to an intent the server admitted: it names that intent's operation and, for
/// a typed or spoken line, its utterance id; it has one of the four outcomes, each well formed; a
/// decline carries a machine decline class and a reason; and it fits the bridge.
pub fn check_machine_receipt(
    receipt: &wire::MachineReceipt,
    intent: &AdmittedMachineIntent,
) -> Result<(), MachineReceiptError> {
    let bytes = receipt.encoded_len();
    if bytes > MAXIMUM_MESSAGE_BYTES {
        return Err(MachineReceiptError::TooLarge { bytes });
    }
    let operation = match request::operation_id(receipt.operation_id.as_ref()) {
        Ok(operation) => operation,
        Err(RequestError::Missing(_)) => {
            return Err(MachineReceiptError::Missing(ReceiptSlot::OperationId));
        }
        Err(other) => return Err(MachineReceiptError::Request(other)),
    };
    if operation != intent.operation {
        return Err(MachineReceiptError::OperationMismatch);
    }
    request::session_revision(receipt.revision.as_ref()).map_err(MachineReceiptError::Request)?;
    let expected_utterance = match &intent.payload {
        AdmittedPayload::SubmitText(line) | AdmittedPayload::Say(line) => Some(line.utterance_id),
        AdmittedPayload::Tap { .. }
        | AdmittedPayload::PendingAnswer { .. }
        | AdmittedPayload::SafetyCard => None,
    };
    if receipt.utterance_id != expected_utterance {
        return Err(MachineReceiptError::UtteranceMismatch);
    }
    let result = receipt
        .result
        .as_ref()
        .ok_or(MachineReceiptError::Missing(ReceiptSlot::Result))?;
    check_result(result)
}

/// Checks a whole response: a receipt as `check_machine_receipt` does, or an observation that the
/// operation did not commit, which must carry a known code (never "unspecified").
pub fn check_machine_response(
    response: &wire::SubmitMachineIntentResponse,
    intent: &AdmittedMachineIntent,
) -> Result<(), MachineReceiptError> {
    use wire::submit_machine_intent_response::Outcome;
    match response
        .outcome
        .as_ref()
        .ok_or(MachineReceiptError::Missing(ReceiptSlot::Outcome))?
    {
        Outcome::Receipt(receipt) => check_machine_receipt(receipt, intent),
        Outcome::OperationObservation(observation) => {
            match wire::RejectionCode::try_from(observation.code) {
                Ok(wire::RejectionCode::Unspecified) => {
                    Err(MachineReceiptError::Missing(ReceiptSlot::ObservationCode))
                }
                Ok(_) => Ok(()),
                Err(_) => Err(MachineReceiptError::UnknownCode(observation.code)),
            }
        }
    }
}

/// Names a slot of an intent without holding what was in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentSlot {
    Intent,
    MachineId,
    OfferId,
    Payload,
    UtteranceId,
    PendingId,
    Text,
}

/// Why an intent was refused at the edge. Not a durable decision and not a receipt: the owner has
/// not seen it. No variant holds a typed line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineIntentError {
    /// The session, run, operation or observed revision is absent or malformed.
    Request(RequestError),
    Missing(IntentSlot),
    Blank(IntentSlot),
    /// A field is set that this kind of intent does not carry.
    Unexpected(IntentSlot),
    /// The line is longer than `MAXIMUM_INTENT_TEXT_BYTES`.
    TextTooLong {
        bytes: usize,
    },
    /// The whole request is longer than `MAXIMUM_MESSAGE_BYTES`.
    RequestTooLarge {
        bytes: usize,
    },
}

impl MachineIntentError {
    /// Whatever is wrong with a request, the client sent it.
    pub fn rpc_status(&self) -> RpcStatusClass {
        RpcStatusClass::InvalidArgument
    }
}

/// A typed or spoken line. `Debug` shows its length and never its words.
#[derive(Clone, PartialEq, Eq)]
pub struct IntentText {
    pub utterance_id: u64,
    pub text: String,
}

impl fmt::Debug for IntentText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntentText")
            .field("utterance_id", &self.utterance_id)
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

/// What the intent asks, by kind. A private message is not a kind yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmittedPayload {
    Tap {
        offer_id: String,
    },
    SubmitText(IntentText),
    Say(IntentText),
    PendingAnswer {
        pending_id: u64,
        offer_id: String,
    },
    /// The seat's pause control. It names no offer.
    SafetyCard,
}

/// A request that passed the edge: typed identities and one well-formed payload. It is a claim
/// the client made. The seat is not here because it comes from the call's credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedMachineIntent {
    pub session: SessionId,
    pub run: RunId,
    pub observed_revision: SessionRevision,
    pub operation: OperationId,
    pub machine_id: String,
    pub payload: AdmittedPayload,
}

fn offer_for(intent: &wire::MachineIntent) -> Result<String, MachineIntentError> {
    match intent.offer_id.as_deref() {
        None => Err(MachineIntentError::Missing(IntentSlot::OfferId)),
        Some(id) if blank(id) => Err(MachineIntentError::Blank(IntentSlot::OfferId)),
        Some(id) => Ok(id.to_owned()),
    }
}

fn no_offer(intent: &wire::MachineIntent) -> Result<(), MachineIntentError> {
    if intent.offer_id.is_some() {
        Err(MachineIntentError::Unexpected(IntentSlot::OfferId))
    } else {
        Ok(())
    }
}

fn line(text: &wire::MachineText) -> Result<IntentText, MachineIntentError> {
    let utterance_id = text
        .utterance_id
        .ok_or(MachineIntentError::Missing(IntentSlot::UtteranceId))?;
    if blank(&text.text) {
        return Err(MachineIntentError::Blank(IntentSlot::Text));
    }
    if text.text.len() > MAXIMUM_INTENT_TEXT_BYTES {
        return Err(MachineIntentError::TextTooLong {
            bytes: text.text.len(),
        });
    }
    Ok(IntentText {
        utterance_id,
        text: text.text.clone(),
    })
}

/// Turns a request into typed values, or names the first thing wrong with it.
pub fn admit_machine_intent(
    request: &wire::SubmitMachineIntentRequest,
) -> Result<AdmittedMachineIntent, MachineIntentError> {
    let bytes = request.encoded_len();
    if bytes > MAXIMUM_MESSAGE_BYTES {
        return Err(MachineIntentError::RequestTooLarge { bytes });
    }
    let session =
        request::session_id(request.session_id.as_ref()).map_err(MachineIntentError::Request)?;
    let run = request::run_id(request.run_id.as_ref()).map_err(MachineIntentError::Request)?;
    let observed_revision = request::session_revision(request.observed_revision.as_ref())
        .map_err(MachineIntentError::Request)?;
    let operation = request::operation_id(request.operation_id.as_ref())
        .map_err(MachineIntentError::Request)?;
    let intent = request
        .intent
        .as_ref()
        .ok_or(MachineIntentError::Missing(IntentSlot::Intent))?;
    if blank(&intent.machine_id) {
        return Err(MachineIntentError::Blank(IntentSlot::MachineId));
    }
    let payload = match intent
        .payload
        .as_ref()
        .ok_or(MachineIntentError::Missing(IntentSlot::Payload))?
    {
        Payload::Tap(_) => AdmittedPayload::Tap {
            offer_id: offer_for(intent)?,
        },
        Payload::SubmitText(text) => {
            no_offer(intent)?;
            AdmittedPayload::SubmitText(line(text)?)
        }
        Payload::Say(text) => {
            no_offer(intent)?;
            AdmittedPayload::Say(line(text)?)
        }
        Payload::SafetyCard(_) => {
            no_offer(intent)?;
            AdmittedPayload::SafetyCard
        }
        Payload::PendingAnswer(answer) => AdmittedPayload::PendingAnswer {
            pending_id: answer
                .pending_id
                .ok_or(MachineIntentError::Missing(IntentSlot::PendingId))?,
            offer_id: offer_for(intent)?,
        },
    };
    Ok(AdmittedMachineIntent {
        session,
        run,
        observed_revision,
        operation,
        machine_id: intent.machine_id.clone(),
        payload,
    })
}

/// Hand-built messages that stand in for a real projection and a real client until the seam in
/// the module docs is filled. The bridge round trip, the browser page and the tests share them,
/// so a field that changes in the schema changes here once.
pub mod sample {
    use df_protocol::common as wire;
    use df_protocol::common::machine_intent::Payload;
    use df_protocol::common::machine_view::Audience;

    /// The identity bytes every sample uses: 16 bytes with one nonzero byte per role.
    pub const SESSION_BYTES: [u8; 16] = [1; 16];
    pub const RUN_BYTES: [u8; 16] = [2; 16];
    pub const OPERATION_BYTES: [u8; 16] = [3; 16];
    /// The epoch and sequence of the sample revision.
    pub const EPOCH: u64 = 1;
    pub const SEQUENCE: u64 = 42;
    /// The machine, offer and line the sample intents name.
    pub const MACHINE_ID: &str = "explore";
    pub const OFFER_ID: &str = "look-around";
    pub const UTTERANCE_ID: u64 = 7;
    pub const SAY_TEXT: &str = "Is anyone there?";
    /// The line a tap on the sample offer adds to the seat's narration.
    pub const TAP_NARRATION: &str = "You take a slow look around the wharf.";

    pub fn revision() -> wire::SessionRevision {
        wire::SessionRevision {
            epoch: Some(wire::RecoveryEpoch { value: Some(EPOCH) }),
            sequence: Some(SEQUENCE),
        }
    }

    fn view(audience: Audience) -> wire::MachineView {
        wire::MachineView {
            revision: Some(revision()),
            machine_id: MACHINE_ID.to_owned(),
            state_label: "Exploring the harbour".to_owned(),
            status: Some(wire::MachineStatus {
                kind: wire::MachineStatusKind::Ok as i32,
                waiting_who: None,
                waiting_what: None,
            }),
            reason: None,
            readout: "Kira is on the Lantern Wharf. The harbour master is waiting.".to_owned(),
            audience: Some(audience),
        }
    }

    /// The shared screen's view: a place, a caption, the party and no choices.
    pub fn shared_screen_view() -> wire::MachineView {
        view(Audience::SharedScreen(wire::MachineSharedScreen {
            place: "Lantern Wharf".to_owned(),
            caption: Some(wire::MachineCaption {
                speaker: Some("Harbour master".to_owned()),
                text: "You are late, and the tide is not.".to_owned(),
            }),
            backdrop: Some(wire::MachineImage {
                media: "wharf.webp".to_owned(),
                caption: None,
            }),
            card: None,
            party: vec![wire::MachinePartyMember {
                name: "Kira".to_owned(),
                hit_points: 11,
                maximum_hit_points: 12,
                conditions: vec!["Prone".to_owned()],
            }],
        }))
    }

    /// One seat's view: narration, a private note, one open offer and one that is not available.
    pub fn seat_view() -> wire::MachineView {
        view(Audience::Seat(wire::MachineSeatView {
            hero: "Kira".to_owned(),
            goal: Some("Find the missing courier".to_owned()),
            narration: vec![
                "The wharf smells of tar and wet rope.".to_owned(),
                "The harbour master taps a ledger and does not look up.".to_owned(),
            ],
            note: Some(wire::MachineNote {
                text: "The ledger has a page torn out.".to_owned(),
            }),
            image: None,
            offers: vec![
                wire::MachineOffer {
                    offer_id: OFFER_ID.to_owned(),
                    label: "Look around".to_owned(),
                    explain: Some("Take a slow look at the wharf.".to_owned()),
                    unavailable_reason: None,
                    group: None,
                },
                wire::MachineOffer {
                    offer_id: "open-the-ledger".to_owned(),
                    label: "Open the ledger".to_owned(),
                    explain: None,
                    unavailable_reason: Some("The harbour master is holding it.".to_owned()),
                    group: Some("Investigate".to_owned()),
                },
            ],
        }))
    }

    fn request(intent: wire::MachineIntent) -> wire::SubmitMachineIntentRequest {
        wire::SubmitMachineIntentRequest {
            session_id: Some(wire::SessionId {
                value: Some(SESSION_BYTES.to_vec()),
            }),
            run_id: Some(wire::RunId {
                value: Some(RUN_BYTES.to_vec()),
            }),
            observed_revision: Some(revision()),
            operation_id: Some(wire::OperationId {
                value: Some(OPERATION_BYTES.to_vec()),
            }),
            intent: Some(intent),
        }
    }

    /// A tap on the open offer of `seat_view`.
    pub fn tap_request() -> wire::SubmitMachineIntentRequest {
        request(wire::MachineIntent {
            machine_id: MACHINE_ID.to_owned(),
            offer_id: Some(OFFER_ID.to_owned()),
            payload: Some(Payload::Tap(wire::MachineTap {})),
        })
    }

    /// A spoken line.
    pub fn say_request() -> wire::SubmitMachineIntentRequest {
        request(wire::MachineIntent {
            machine_id: MACHINE_ID.to_owned(),
            offer_id: None,
            payload: Some(Payload::Say(wire::MachineText {
                utterance_id: Some(UTTERANCE_ID),
                text: SAY_TEXT.to_owned(),
            })),
        })
    }

    /// The seat's pause control.
    pub fn safety_card_request() -> wire::SubmitMachineIntentRequest {
        request(wire::MachineIntent {
            machine_id: MACHINE_ID.to_owned(),
            offer_id: None,
            payload: Some(Payload::SafetyCard(wire::MachineSafetyCard {})),
        })
    }

    /// The server's answer to a tap: applied, at the revision after the sample view's.
    pub fn applied_response() -> wire::SubmitMachineIntentResponse {
        wire::SubmitMachineIntentResponse {
            outcome: Some(wire::submit_machine_intent_response::Outcome::Receipt(
                wire::MachineReceipt {
                    operation_id: Some(wire::OperationId {
                        value: Some(OPERATION_BYTES.to_vec()),
                    }),
                    revision: Some(wire::SessionRevision {
                        epoch: Some(wire::RecoveryEpoch { value: Some(EPOCH) }),
                        sequence: Some(SEQUENCE + 1),
                    }),
                    replayed: false,
                    utterance_id: None,
                    result: Some(wire::machine_receipt::Result::Applied(
                        wire::MachineApplied {},
                    )),
                },
            )),
        }
    }
}
