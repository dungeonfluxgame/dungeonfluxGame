//! The shape of one input as the owner receives it (C.1, RES21).
//!
//! It owns `InputEnvelope`: an operation id (the identity of the input, H.2), the `ViewStamp` it was
//! drawn from, who it came from (`InputOrigin`: a seat with a `PlayerInput`, or the engine with an
//! `EngineInput`) and the optional evidence reference of its text (RES36). `InputKind` is the closed
//! list of the nine kinds the plan names. A player input always names its seat and an engine input
//! never does, so a seat-less tap and a seated timer cannot be built. Typed text is a `PlayerText`,
//! which has no `Display` and shows only a length class in `Debug`, so an envelope is safe to print
//! in a failure message.
//!
//! It never validates meaning (the interpreter and the rules owner do), never checks that a seat
//! sits at the table (the owner queue does) and never assigns a `turn_seq` (the queue does, when it
//! serves the input).

use serde::{Deserialize, Serialize};

use crate::element::name_type;
use crate::identity::{EvidenceRef, OperationId, SeatId, SessionRevision};
use crate::listed::listed_enum;
use crate::ports::{ActionId, Readiness, ResourceHandle, RulingKey, RulingScope};
use crate::private_data::{LengthClass, Measured, PlayerText};
use crate::table_state::Presence;

name_type!(
    /// The registered name of a timer, such as `idle_nudge`; the owner of the timer decides what it
    /// means when it fires.
    TimerName
);
name_type!(
    /// Names the engine part that proposed something (RES21 `proposer_ref`): an NPC, a story beat,
    /// an encounter or a machine asking for a hand-off.
    ProposerRef
);

/// Identifies one line of free text of one seat, so a result read off the owner thread can be
/// matched to the line it answers (RES22 keys INTERPRET by seat, utterance and revision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtteranceId(u64);

impl UtteranceId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Names one open pending resolution that an answer is for (RES26). Step C4 (`pending`) owns the
/// pendings; the id is declared here until it exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PendingId(u64);

impl PendingId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The revision of the projection an input was drawn from (C.1, H.2). An input drawn from a view
/// older than the seat's last change of legal choices is re-checked, never trusted. Engine inputs
/// carry the revision they were produced against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewStamp {
    revision: SessionRevision,
}

impl ViewStamp {
    pub fn new(revision: SessionRevision) -> Self {
        Self { revision }
    }

    pub fn revision(self) -> SessionRevision {
        self.revision
    }
}

listed_enum! {
    /// The nine kinds of input (O.3 `envelope`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum InputKind {
        Tap,
        SubmitText,
        Say,
        PrivateMessage,
        SafetyCard,
        EngineProposal,
        Timer,
        ResourceResult,
        PendingAnswer,
    }
}

impl InputKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Tap => "tap",
            Self::SubmitText => "submit_text",
            Self::Say => "say",
            Self::PrivateMessage => "private_message",
            Self::SafetyCard => "safety_card",
            Self::EngineProposal => "engine_proposal",
            Self::Timer => "timer",
            Self::ResourceResult => "resource_result",
            Self::PendingAnswer => "pending_answer",
        }
    }
}

/// What a seat sent. A phone sends intents and never state (C.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerInput {
    /// A tap on an offered choice; it is already an intent and skips INTERPRET (C.1).
    Tap { action: ActionId },
    /// A line of free text typed into the phone's input box.
    SubmitText {
        utterance: UtteranceId,
        text: PlayerText,
    },
    /// A line the hero speaks aloud.
    Say {
        utterance: UtteranceId,
        text: PlayerText,
    },
    /// A message to one other seat, kept out of the TV and out of NPC context (F.7).
    PrivateMessage {
        utterance: UtteranceId,
        to: SeatId,
        text: PlayerText,
    },
    /// The pause control of the phone (F.7).
    SafetyCard,
    /// The answer to an open pending resolution (RES26).
    PendingAnswer {
        pending: PendingId,
        choice: ActionId,
    },
}

impl PlayerInput {
    fn kind(&self) -> InputKind {
        match self {
            Self::Tap { .. } => InputKind::Tap,
            Self::SubmitText { .. } => InputKind::SubmitText,
            Self::Say { .. } => InputKind::Say,
            Self::PrivateMessage { .. } => InputKind::PrivateMessage,
            Self::SafetyCard => InputKind::SafetyCard,
            Self::PendingAnswer { .. } => InputKind::PendingAnswer,
        }
    }

    fn text(&self) -> Option<&PlayerText> {
        match self {
            Self::SubmitText { text, .. }
            | Self::Say { text, .. }
            | Self::PrivateMessage { text, .. } => Some(text),
            Self::Tap { .. } | Self::SafetyCard | Self::PendingAnswer { .. } => None,
        }
    }
}

/// Where an engine proposal came from (RES21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProposalOrigin {
    Npc,
    World,
    Story,
    Encounter,
    Timer,
    /// A machine asking for a hand-off (RES21: machine-requested hand-offs enter as engine input).
    Machine,
}

listed_enum! {
    /// Why a pending resolution was cancelled by the engine rather than answered (C.4).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CancelCause {
        /// A seat the pending waits on left the table for good.
        SeatLeft,
        /// The scene the pending belongs to is not the table's scene any more.
        SceneChanged,
        /// The host chose to move on without the answer (the commands of `remote-play.md`).
        HostDecision,
    }
}

impl CancelCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::SeatLeft => "seat_left",
            Self::SceneChanged => "scene_changed",
            Self::HostDecision => "host_decision",
        }
    }
}

/// A change the engine asks for that is not a proposal of the story: every one is an input with an
/// operation id, so it is committed, recorded in a turn record and replayable (O.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineControl {
    /// A seat connected, fell asleep, dropped or stepped away: the turns and windows it holds are
    /// held or released and nothing is spent (C.4, D.4).
    SeatPresence { seat: SeatId, presence: Presence },
    /// The host takes a stored ruling back (RES48).
    RevokeRuling { key: RulingKey, scope: RulingScope },
    /// A pending resolution ends without an answer and without spending anything: the exit for a
    /// window whose seat left for good, or one the host moves past.
    CancelPending {
        pending: PendingId,
        cause: CancelCause,
    },
    /// A paused or idle table plays again.
    ResumeTable,
}

/// What the engine itself sent: it skips ADMIT's seat checks and INTERPRET (RES21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineInput {
    /// A table, seat, ruling or pending control (see `EngineControl`).
    Control(EngineControl),
    Proposal {
        origin: ProposalOrigin,
        proposer: ProposerRef,
    },
    Timer {
        timer: TimerName,
    },
    /// A resource request finished, or failed, off the owner (RES22).
    ResourceResult {
        handle: ResourceHandle,
        readiness: Readiness,
    },
}

impl EngineInput {
    fn kind(&self) -> InputKind {
        match self {
            Self::Proposal { .. } | Self::Control(_) => InputKind::EngineProposal,
            Self::Timer { .. } => InputKind::Timer,
            Self::ResourceResult { .. } => InputKind::ResourceResult,
        }
    }
}

/// Who an input came from. The seat of a player input comes from the authenticated session and
/// never from the message (C.0 ADMIT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputOrigin {
    Player { seat: SeatId, input: PlayerInput },
    Engine(EngineInput),
}

/// Who sent an input: a seat, or the engine. Operation ids are minted by their sender (a phone
/// client numbers its own, the engine numbers its own), so an id means something only together
/// with its sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sender {
    Engine,
    Seat(SeatId),
}

/// One input on its way to the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEnvelope {
    operation_id: OperationId,
    view: ViewStamp,
    origin: InputOrigin,
    evidence: Option<EvidenceRef>,
}

impl InputEnvelope {
    pub fn from_player(
        operation_id: OperationId,
        seat: SeatId,
        view: ViewStamp,
        input: PlayerInput,
    ) -> Self {
        Self::with_origin(operation_id, view, InputOrigin::Player { seat, input })
    }

    pub fn from_engine(operation_id: OperationId, view: ViewStamp, input: EngineInput) -> Self {
        Self::with_origin(operation_id, view, InputOrigin::Engine(input))
    }

    fn with_origin(operation_id: OperationId, view: ViewStamp, origin: InputOrigin) -> Self {
        Self {
            operation_id,
            view,
            origin,
            evidence: None,
        }
    }

    /// Names the specimen that holds this input's text in the diagnostic store (RES36).
    pub fn with_evidence(mut self, evidence: EvidenceRef) -> Self {
        self.evidence = Some(evidence);
        self
    }

    pub fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    pub fn view(&self) -> ViewStamp {
        self.view
    }

    pub fn origin(&self) -> &InputOrigin {
        &self.origin
    }

    pub fn evidence(&self) -> Option<EvidenceRef> {
        self.evidence
    }

    pub fn kind(&self) -> InputKind {
        match &self.origin {
            InputOrigin::Player { input, .. } => input.kind(),
            InputOrigin::Engine(input) => input.kind(),
        }
    }

    pub fn sender(&self) -> Sender {
        match &self.origin {
            InputOrigin::Player { seat, .. } => Sender::Seat(*seat),
            InputOrigin::Engine(_) => Sender::Engine,
        }
    }

    /// The seat of a player input; an engine input has none.
    pub fn seat(&self) -> Option<SeatId> {
        match &self.origin {
            InputOrigin::Player { seat, .. } => Some(*seat),
            InputOrigin::Engine(_) => None,
        }
    }

    /// How long the typed text is, which is all a record may say about it; `Empty` for an input
    /// that carries no text.
    pub fn length_class(&self) -> LengthClass {
        let text = match &self.origin {
            InputOrigin::Player { input, .. } => input.text(),
            InputOrigin::Engine(_) => None,
        };
        text.map_or(LengthClass::Empty, |text| {
            LengthClass::of_length(text.expose().length_in_chars())
        })
    }
}
