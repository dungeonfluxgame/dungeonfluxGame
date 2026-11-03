//! The machine contract: what a machine is given, what it returns and how it fails (RES27, O.3).
//!
//! A machine is a pure, synchronous module that owns a closed enum of states and nothing else
//! (principle 1, D24). It reads no clock, draws no dice and calls no port: the time, a read view
//! of the table, the rules owner's decision and the dice all arrive as arguments, and everything it
//! wants done leaves in the `StepResult` as table effects and resource requests (principle 2).
//! The pipeline (C3) owns the calls: it asks `accepts`, decides with the rules owner, calls `step`,
//! commits, and polls `wants_hand_off`.
//!
//! The trait follows RES27. `enter(table, entry) -> state`, `accepts(table, input, state) -> bool`
//! (the seat is the one inside `MachineInput::Intent`, so a seat and its input cannot disagree),
//! `step(state, input) -> StepResult` and `exit(table, state) -> HandOff`. The table is passed
//! read-only to every call that needs a shared fact, so a machine keeps only its own facts in its
//! state and never a copy of the seats or the capacity that can go stale. State crosses the trait
//! as a `MachineSlot`, a versioned saved record, because the registry holds machines of different
//! state types; a machine decodes it into its closed enum at the start of a call and encodes the
//! new state at the end, and a step that changes nothing returns no state at all. `step` returns
//! `Waiting{pending_id}` as a `StepOutcome` value, not as an `Outcome` (O.7).
//!
//! Leaving is level-triggered: `wants_hand_off(state)` is derived from the state, so a hand-off the
//! engine refuses (`pending_open`, a full stack) leaves the machine asking, and a restart asks
//! again. RES21 says a machine may ask for a hand-off and the DM applies it; the pipeline polls
//! this after a step and enqueues the ask as an engine proposal, and `exit` gives the target and
//! the typed carry list. A hand-off names a machine, releases a seat lane, or returns to the
//! frame below (`HandOffTarget`); the registry checks the value `exit` returned (`check_hand_off`).
//!
//! A resource request carries a key the machine chose and stores in its state; the broker handle
//! never reaches a machine, and a result is delivered only when final (`ResourceOutcome` has no
//! pending value), keyed by the same key. `project` is a stub for C5, and `inspect` is the report
//! that `inspect::MachinesOf` serves, with the saved payload for the operator only (C7);
//! `migrate` and `validate` are the H.12 hooks, and `resume` for the hand-off stack waits for C6.
//!
//! It never validates a hand-off against the stack or the pending resolutions (C6, C4), never
//! applies an effect (`effects` does) and never records an event by itself. Each event has a
//! builder next to the code that knows the fact, for the pipeline (C3) to call: `step_events`
//! (transitioned, declined), `entered_event`, `exited_event`, `MachineError::event` (failed),
//! `TableState::apply` and `EffectError::event` (effect applied, refused) and
//! `LoadError::refusal_event` and `RecordRefused::event` (load refused). Until C3 exists the
//! conformance driver is their only caller.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::effects::{TableEffect, TableState};
use crate::element::NameError;
use crate::envelope::{PendingId, ProposalOrigin, ProposerRef, TimerName};
use crate::event::{FailureCause, TableEvent};
use crate::identity::{Correlation, SceneId, SeatId, TableScope};
use crate::listed::listed_enum;
use crate::machine_slot::{LoadError, LoadFault, MachineSlot, SlotError};
use crate::outcome::{Decline, DeclineError, Outcome};
use crate::ports::{ActionId, Audience, Draw, PurposeName, Resolution};

pub use crate::identity::MachineId;
pub use crate::machine_spec::{
    InputClaim, MachineSpec, Scope, SpecError, StateName, SuspendPolicy,
};

/// The most items a hand-off carries. Seats are bounded by `MAXIMUM_SEATS`, so this never limits
/// play; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_CARRY_ITEMS: usize = 1_000_000;

/// One thing a machine hands to the next: the typed carry list of a hand-off (RES27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarryItem {
    Seat(SeatId),
    Scene(SceneId),
}

/// Why a hand-off could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandOffError {
    TooManyCarryItems,
}

impl fmt::Display for HandOffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyCarryItems => {
                write!(
                    formatter,
                    "a hand-off carries at most {MAXIMUM_CARRY_ITEMS} items"
                )
            }
        }
    }
}

impl std::error::Error for HandOffError {}

listed_enum! {
    /// The three ways a machine can end, without their payload.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum HandOffKind {
        Machine,
        ReleaseLane,
        ReturnToFrameBelow,
    }
}

impl HandOffKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Machine => "machine",
            Self::ReleaseLane => "release_lane",
            Self::ReturnToFrameBelow => "return_to_frame_below",
        }
    }
}

/// Where the table goes when a machine ends. A seat lane has no machine to go to, so it releases
/// the lane; a group machine pushed onto the stack may return to the frame below it (C6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandOffTarget {
    Machine(MachineId),
    ReleaseLane,
    ReturnToFrameBelow,
}

impl HandOffTarget {
    pub fn kind(&self) -> HandOffKind {
        match self {
            Self::Machine(_) => HandOffKind::Machine,
            Self::ReleaseLane => HandOffKind::ReleaseLane,
            Self::ReturnToFrameBelow => HandOffKind::ReturnToFrameBelow,
        }
    }
}

/// Where the table goes when a machine exits, and what goes with it (RES27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandOff {
    target: HandOffTarget,
    carry: Vec<CarryItem>,
}

impl HandOff {
    /// A hand-off to another machine.
    pub fn new(target: MachineId, carry: Vec<CarryItem>) -> Result<Self, HandOffError> {
        Self::to(HandOffTarget::Machine(target), carry)
    }

    pub fn to(target: HandOffTarget, carry: Vec<CarryItem>) -> Result<Self, HandOffError> {
        if carry.len() > MAXIMUM_CARRY_ITEMS {
            return Err(HandOffError::TooManyCarryItems);
        }
        Ok(Self { target, carry })
    }

    pub fn target(&self) -> &HandOffTarget {
        &self.target
    }

    /// The machine the hand-off names, if it names one.
    pub fn machine_target(&self) -> Option<&MachineId> {
        match &self.target {
            HandOffTarget::Machine(machine) => Some(machine),
            HandOffTarget::ReleaseLane | HandOffTarget::ReturnToFrameBelow => None,
        }
    }

    pub fn carry(&self) -> &[CarryItem] {
        &self.carry
    }
}

/// What a machine is entered with: the carry list of the hand-off that brought the table to it,
/// or nothing when the table starts there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    carry: Vec<CarryItem>,
}

impl Entry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn carrying(carry: Vec<CarryItem>) -> Self {
        Self { carry }
    }

    pub fn from_hand_off(hand_off: &HandOff) -> Self {
        Self::carrying(hand_off.carry().to_vec())
    }

    pub fn carry(&self) -> &[CarryItem] {
        &self.carry
    }

    /// The first scene carried, if any.
    pub fn scene(&self) -> Option<SceneId> {
        self.carry.iter().find_map(|item| match item {
            CarryItem::Scene(scene) => Some(*scene),
            CarryItem::Seat(_) => None,
        })
    }

    /// The seats carried, in order.
    pub fn seats(&self) -> impl Iterator<Item = SeatId> + '_ {
        self.carry.iter().filter_map(|item| match item {
            CarryItem::Seat(seat) => Some(*seat),
            CarryItem::Scene(_) => None,
        })
    }
}

/// Why `enter` refused to start a machine: the entry and the table disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryRefusal {
    MissingScene,
    SceneNotActive { scene: SceneId },
    NoSeats,
    SeatNotAtTable { seat: SeatId },
}

impl fmt::Display for EntryRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingScene => formatter.write_str("the entry carries no scene"),
            Self::SceneNotActive { scene } => {
                write!(formatter, "scene {scene} is not the table's active scene")
            }
            Self::NoSeats => formatter.write_str("the entry carries no seat"),
            Self::SeatNotAtTable { seat } => write!(formatter, "seat {seat} is not at the table"),
        }
    }
}

impl std::error::Error for EntryRefusal {}

/// Why a projection could not be built: it would have been empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadoutError {
    Blank,
}

impl fmt::Display for ReadoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a readout cannot be blank")
    }
}

impl std::error::Error for ReadoutError {}

listed_enum! {
    /// The closed cause of a failed machine call, as the `MachineFailed` event records it. A
    /// record that could not be read keeps the fault that `load.refused` would name.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum MachineFailureCause {
        SlotWrongMachine,
        SlotUnsupportedVersion,
        SlotMalformed,
        EntryRefused,
        /// A state or its name could not be written.
        StateInvalid,
        HandOffRefused,
        /// A projection would have been empty.
        ProjectionBlank,
        /// A decline was built without a usable reason.
        DeclineInvalid,
    }
}

impl MachineFailureCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::SlotWrongMachine => "slot_wrong_machine",
            Self::SlotUnsupportedVersion => "slot_unsupported_version",
            Self::SlotMalformed => "slot_malformed",
            Self::EntryRefused => "entry_refused",
            Self::StateInvalid => "state_invalid",
            Self::HandOffRefused => "hand_off_refused",
            Self::ProjectionBlank => "projection_blank",
            Self::DeclineInvalid => "decline_invalid",
        }
    }
}

impl From<LoadFault> for MachineFailureCause {
    fn from(fault: LoadFault) -> Self {
        match fault {
            LoadFault::WrongMachine => Self::SlotWrongMachine,
            LoadFault::UnsupportedVersion => Self::SlotUnsupportedVersion,
            LoadFault::Malformed => Self::SlotMalformed,
        }
    }
}

/// Why a machine call failed. A failure is a typed error the pipeline records once; it is never a
/// panic and never a fabricated success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineError {
    Load(LoadError),
    Entry(EntryRefusal),
    Slot(SlotError),
    HandOff(HandOffError),
    Projection(ReadoutError),
    Decline(DeclineError),
}

impl MachineError {
    pub fn cause(&self) -> MachineFailureCause {
        match self {
            Self::Load(error) => error.fault().into(),
            Self::Entry(_) => MachineFailureCause::EntryRefused,
            Self::Slot(_) => MachineFailureCause::StateInvalid,
            Self::HandOff(_) => MachineFailureCause::HandOffRefused,
            Self::Projection(_) => MachineFailureCause::ProjectionBlank,
            Self::Decline(_) => MachineFailureCause::DeclineInvalid,
        }
    }

    /// The class A event that records the failure.
    pub fn event(&self, scope: &TableScope, correlation: Correlation) -> TableEvent {
        TableEvent::MachineFailed {
            scope: scope.clone(),
            correlation,
            cause: self.cause(),
        }
    }
}

impl fmt::Display for MachineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => error.fmt(formatter),
            Self::Entry(error) => error.fmt(formatter),
            Self::Slot(error) => error.fmt(formatter),
            Self::HandOff(error) => error.fmt(formatter),
            Self::Projection(error) => error.fmt(formatter),
            Self::Decline(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for MachineError {}

macro_rules! machine_error_from {
    ($($source:ty => $variant:ident),+ $(,)?) => {
        $(
            impl From<$source> for MachineError {
                fn from(source: $source) -> Self {
                    Self::$variant(source)
                }
            }
        )+
    };
}

machine_error_from!(
    LoadError => Load,
    EntryRefusal => Entry,
    SlotError => Slot,
    HandOffError => HandOff,
    ReadoutError => Projection,
    DeclineError => Decline,
);

impl From<NameError> for MachineError {
    fn from(error: NameError) -> Self {
        Self::Slot(SlotError::InvalidStateName(error))
    }
}

/// A key a machine chose for one of its resource requests, and stores in its state to recognise
/// the result by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestKey(u64);

impl RequestKey {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// How a resource request ended. Only a final answer is delivered to a machine: a request still
/// pending is not an input, so there is no value for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceOutcome {
    Ready,
    Failed(FailureCause),
}

/// What reaches a machine. Typed text never does: a free line was interpreted into an action
/// before the pipeline got here, and a tap already is one (C.1). The operation id is in the
/// correlation and the view revision was checked at ADMIT, so neither is a machine's concern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineInput {
    /// A seat's intent: a tap or an interpreted line. A target joins it at C3.
    Intent {
        seat: SeatId,
        action: ActionId,
    },
    /// The dice of an open roll, drawn at the single draw point (RES26, RES27 `Dice`).
    Dice {
        pending: PendingId,
        draws: Vec<Draw>,
    },
    /// A resource request this machine made has finished, keyed by the key it chose (RES22).
    ResourceResult {
        key: RequestKey,
        outcome: ResourceOutcome,
    },
    Timer {
        timer: TimerName,
    },
    Proposal {
        origin: ProposalOrigin,
        proposer: ProposerRef,
    },
    /// The pending this machine reported `Waiting` on was cancelled without an answer (a seat left
    /// for good, the host moved on, C.4): the machine drops what it held for it and nothing is
    /// spent. A machine that is not waiting on that pending declines it.
    PendingCancelled {
        pending: PendingId,
    },
}

/// What DECIDE settled before `step` is called (RES27: "the DECIDE outcome and its draws").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The rules owner resolved the action.
    Resolved(Resolution),
    /// A player must roll: the pending handler opened this pending (RES26).
    RollPending(PendingId),
    /// A window or a roller's choice is open: the pending handler opened this pending and the
    /// machine reports `Waiting` on it until the answers come back as a `Dice` input (C.4).
    Pending(PendingId),
}

/// Everything `step` may read besides the saved state. Time, the table and the decision are here
/// so the machine never asks for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepInput<'a> {
    pub correlation: Correlation,
    pub now_ms: u64,
    pub table: &'a TableState,
    pub input: MachineInput,
    pub decision: Option<Decision>,
}

/// A request for a resource. The broker answers later with an input keyed by `key`; the machine
/// never waits and never sees the broker's handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRequest {
    pub key: RequestKey,
    pub purpose: PurposeName,
}

/// How a step ended: one of the four outcomes, or the machine's report that a pending is open.
/// The pipeline records the input's outcome as `pending{...}` for `Waiting` (O.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Settled(Outcome),
    Waiting(PendingId),
}

/// What one step produced. A declined or clarified step changes nothing: no state, no effects,
/// no requests. A waiting step may change only the state, to record the pending it waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    /// The new state, or `None` when the state did not change.
    pub state: Option<MachineSlot>,
    pub table_effects: Vec<TableEffect>,
    pub resource_requests: Vec<ResourceRequest>,
    pub outcome: StepOutcome,
}

impl StepResult {
    /// An input that changed nothing and was refused.
    pub fn declined(decline: Decline) -> Self {
        Self {
            state: None,
            table_effects: Vec::new(),
            resource_requests: Vec::new(),
            outcome: StepOutcome::Settled(Outcome::Declined(decline)),
        }
    }

    /// An input that was applied, moving to `state` and proposing `table_effects`.
    pub fn applied(state: MachineSlot, table_effects: Vec<TableEffect>) -> Self {
        Self {
            state: Some(state),
            table_effects,
            resource_requests: Vec::new(),
            outcome: StepOutcome::Settled(Outcome::Applied),
        }
    }

    /// An input that was applied without changing the machine's own state.
    pub fn applied_in_place(table_effects: Vec<TableEffect>) -> Self {
        Self {
            state: None,
            table_effects,
            resource_requests: Vec::new(),
            outcome: StepOutcome::Settled(Outcome::Applied),
        }
    }

    /// The state after the step: the new one, or the one the step started from.
    pub fn state_after<'a>(&'a self, before: &'a MachineSlot) -> &'a MachineSlot {
        self.state.as_ref().unwrap_or(before)
    }
}

/// What a projection says about a machine for one audience. It cannot be blank (H.15): the one
/// way to build it refuses empty text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineReadout(String);

impl MachineReadout {
    pub fn new(text: &str) -> Result<Self, ReadoutError> {
        if text.trim().is_empty() {
            return Err(ReadoutError::Blank);
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A machine's report on itself: which machine, in which state, under which schema version. The
/// audience-aware report that serves it is `inspect::MachinesOf`, which adds the saved payload for
/// the operator only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectReport {
    pub machine: MachineId,
    pub state: StateName,
    pub schema_version: u32,
}

/// A machine. See the module documentation for what each method owes and what it never does.
pub trait Machine: Send + Sync {
    fn spec(&self) -> &MachineSpec;

    /// Starts the machine for this entry, or refuses an entry the table disagrees with. It reads
    /// the table and never changes it.
    fn enter(&self, table: &TableState, entry: &Entry) -> Result<MachineSlot, MachineError>;

    /// Whether this input is addressed to the machine in this state. An input that is not
    /// accepted is declined by `step` and leaves the state as it was. A slot that cannot be read
    /// accepts nothing.
    fn accepts(&self, table: &TableState, input: &MachineInput, state: &MachineSlot) -> bool;

    fn step(&self, state: &MachineSlot, input: &StepInput<'_>) -> Result<StepResult, MachineError>;

    /// Whether the machine is finished and asks the DM for a hand-off (RES21). It is derived from
    /// the state, so it is still true after a hand-off the engine refused, and after a restart.
    fn wants_hand_off(&self, state: &MachineSlot) -> Result<bool, MachineError>;

    /// Where the table goes from here and what goes with it, for the table as it is now.
    fn exit(&self, table: &TableState, state: &MachineSlot) -> Result<HandOff, MachineError>;

    /// The closed name of the state the slot holds, read from its tag without decoding the rest;
    /// every name is one the spec lists.
    fn state_name(&self, state: &MachineSlot) -> Result<StateName, MachineError>;

    /// Decodes the whole state, proving every field of the record readable.
    fn validate(&self, state: &MachineSlot) -> Result<(), MachineError>;

    /// What the machine shows `audience`, never empty (stub until C5).
    fn project(
        &self,
        table: &TableState,
        state: &MachineSlot,
        audience: Audience,
    ) -> Result<MachineReadout, MachineError>;

    /// The actions `seat`, which is at the table, may take now: the buttons its phone holds while
    /// nothing is pending (H3). A list the machine states itself, never a guess from the words of
    /// a line; the pipeline publishes it only when asked to (`Pipeline::with_machine_surface`),
    /// and a tap on one still goes through `accepts` and the rules like any other. The default is
    /// none, so a machine with no standing choices needs no code.
    fn standing_offers(
        &self,
        _table: &TableState,
        _state: &MachineSlot,
        _seat: SeatId,
    ) -> Vec<ActionId> {
        Vec::new()
    }

    /// The machine's report on itself: the conformance suite requires it and
    /// `inspect::MachinesOf` serves it.
    fn inspect(&self, state: &MachineSlot) -> Result<InspectReport, MachineError>;

    /// Moves a record one schema version forward. A machine that changes its state overrides
    /// this; the default refuses, so an unmigratable record is never loaded wrongly (H.12).
    fn migrate(&self, from: &MachineSlot) -> Result<MachineSlot, LoadError> {
        Err(LoadError::UnsupportedVersion {
            found: from.schema_version(),
            expected: self.spec().schema_version(),
        })
    }

    /// Reads a saved record: migrates it step by step to the current schema version, then checks
    /// that its whole state can be read. An unmigratable or unreadable record is refused (H.12),
    /// with the typed reason `validate` gave.
    fn load(&self, saved: &MachineSlot) -> Result<MachineSlot, LoadError> {
        let spec = self.spec();
        if saved.machine() != spec.id() {
            return Err(LoadError::WrongMachine {
                expected: spec.id().clone(),
                found: saved.machine().clone(),
            });
        }
        let expected = spec.schema_version();
        let mut slot = saved.clone();
        while slot.schema_version() != expected {
            let found = slot.schema_version();
            let refused = LoadError::UnsupportedVersion { found, expected };
            if found == 0 || found > expected {
                return Err(refused);
            }
            let next = self.migrate(&slot)?;
            if next.machine() != spec.id() || found.checked_add(1) != Some(next.schema_version()) {
                return Err(refused);
            }
            slot = next;
        }
        match self.validate(&slot) {
            Ok(()) => Ok(slot),
            Err(MachineError::Load(error)) => Err(error),
            Err(_) => Err(LoadError::Malformed {
                version: expected,
                class: crate::machine_slot::ErrorClass::Data,
            }),
        }
    }
}

/// The events one step stands for: a transition when the state name changed, and the decline
/// when the input was declined. Effects and failures have their own events (`TableState::apply`,
/// `MachineError::event`).
pub fn step_events(
    scope: &TableScope,
    correlation: Correlation,
    before: &StateName,
    after: &StateName,
    outcome: &StepOutcome,
) -> Vec<TableEvent> {
    let mut events = Vec::new();
    if before != after {
        events.push(TableEvent::MachineTransitioned {
            scope: scope.clone(),
            correlation,
            from: before.clone(),
            to: after.clone(),
        });
    }
    if let StepOutcome::Settled(Outcome::Declined(decline)) = outcome {
        events.push(TableEvent::InputDeclined {
            scope: scope.clone(),
            correlation,
            code: decline.code(),
        });
    }
    events
}

/// The event for a machine starting in `state`.
pub fn entered_event(scope: &TableScope, correlation: Correlation, state: StateName) -> TableEvent {
    TableEvent::MachineEntered {
        scope: scope.clone(),
        correlation,
        state,
    }
}

/// The event for a machine ending with this hand-off.
pub fn exited_event(
    scope: &TableScope,
    correlation: Correlation,
    hand_off: &HandOff,
) -> TableEvent {
    TableEvent::MachineExited {
        scope: scope.clone(),
        correlation,
        kind: hand_off.target().kind(),
        target: hand_off.machine_target().cloned(),
        carried: hand_off.carry().len(),
    }
}
