//! The machine contract: what a machine is given, what it returns and how it fails (RES27, O.3).
//!
//! A machine is a pure, synchronous module that owns a closed enum of states and nothing else
//! (principle 1, D24). It reads no clock, draws no dice and calls no port: the time, the rules
//! owner's decision and the dice all arrive in the `StepInput`, and everything it wants done
//! leaves in the `StepResult` as table effects and resource requests (principle 2). The pipeline
//! (C3) owns the calls: it asks `accepts`, decides with the rules owner, then calls `step` and
//! commits.
//!
//! The trait follows RES27. `enter(table, entry) -> state`, `accepts(input, state) -> bool` (the
//! seat is the one inside `MachineInput::Intent`, so a seat and its input cannot disagree),
//! `step(state, input) -> StepResult`, and `exit(state) -> HandOff`. State crosses the trait as a
//! `MachineSlot`, a versioned saved record, because the registry holds machines of different
//! state types; a machine decodes it into its closed enum at the start of `step` and encodes the
//! new state at the end. `step` returns `Waiting{pending_id}` as a `StepOutcome` value, not as an
//! `Outcome` (O.7). Three small stubs stand in for later steps: `project` for C5, `inspect` for C7
//! and `migrate` for H.12.
//!
//! RES27 has no field that lets a machine ask to leave, though RES21 says a machine may ask for a
//! hand-off and the DM applies it, so `StepResult::asks_for_hand_off` carries the ask; the target
//! and the typed carry list come from `exit`.
//!
//! It never validates a hand-off against the stack or the pending resolutions (C6, C4), never
//! applies an effect (`effects` does) and never records an event by itself: `step_events` turns a
//! step into events for whoever drives it.

use std::fmt;

use crate::effects::{TableEffect, TableState};
use crate::element::NameError;
use crate::envelope::{PendingId, ProposalOrigin, ProposerRef, TimerName};
use crate::event::TableEvent;
use crate::identity::{Correlation, SceneId, SeatId, TableScope};
use crate::listed::listed_enum;
use crate::machine_slot::{LoadError, MachineSlot, SlotError};
use crate::outcome::{Decline, DeclineError, Outcome};
use crate::ports::{ActionId, Audience, Draw, PurposeName, Readiness, Resolution, ResourceHandle};

pub use crate::identity::MachineId;
pub use crate::machine_spec::{MachineSpec, Scope, SpecError, StateName, SuspendPolicy};

use serde::{Deserialize, Serialize};

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

/// Where the table goes when a machine exits, and what goes with it (RES27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandOff {
    target: MachineId,
    carry: Vec<CarryItem>,
}

impl HandOff {
    pub fn new(target: MachineId, carry: Vec<CarryItem>) -> Result<Self, HandOffError> {
        if carry.len() > MAXIMUM_CARRY_ITEMS {
            return Err(HandOffError::TooManyCarryItems);
        }
        Ok(Self { target, carry })
    }

    pub fn target(&self) -> &MachineId {
        &self.target
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
    /// The closed cause of a failed machine call, as the `MachineFailed` event records it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum MachineFailureCause {
        /// The saved state could not be read for this machine.
        SlotRefused,
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
            Self::SlotRefused => "slot_refused",
            Self::EntryRefused => "entry_refused",
            Self::StateInvalid => "state_invalid",
            Self::HandOffRefused => "hand_off_refused",
            Self::ProjectionBlank => "projection_blank",
            Self::DeclineInvalid => "decline_invalid",
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
            Self::Load(_) => MachineFailureCause::SlotRefused,
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

/// What reaches a machine. Typed text never does: a free line was interpreted into an action
/// before the pipeline got here, and a tap already is one (C.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineInput {
    /// A seat's intent: a tap or an interpreted line.
    Intent {
        seat: SeatId,
        action: ActionId,
    },
    /// The dice of an open roll, drawn at the single draw point (RES26, RES27 `Dice`).
    Dice {
        pending: PendingId,
        draws: Vec<Draw>,
    },
    /// A resource request this machine made has finished or failed (RES22).
    ResourceResult {
        handle: ResourceHandle,
        readiness: Readiness,
    },
    Timer {
        timer: TimerName,
    },
    Proposal {
        origin: ProposalOrigin,
        proposer: ProposerRef,
    },
}

/// What DECIDE settled before `step` is called (RES27: "the DECIDE outcome and its draws").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The rules owner resolved the action.
    Resolved(Resolution),
    /// A player must roll: the pending handler opened this pending (RES26).
    RollPending(PendingId),
}

/// Everything `step` may read besides the saved state. Time and the decision are here so the
/// machine never asks for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepInput {
    pub correlation: Correlation,
    pub now_ms: u64,
    pub input: MachineInput,
    pub decision: Option<Decision>,
}

/// A request for a resource. The broker answers later with an input; the machine never waits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRequest {
    pub purpose: PurposeName,
}

/// How a step ended: one of the four outcomes, or the machine's report that a pending is open.
/// The pipeline records the input's outcome as `pending{...}` for `Waiting` (O.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Settled(Outcome),
    Waiting(PendingId),
}

/// What one step produced: the new state, what the table should change, what is wanted from the
/// broker, how it ended and whether the machine asks to hand off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    pub state: MachineSlot,
    pub table_effects: Vec<TableEffect>,
    pub resource_requests: Vec<ResourceRequest>,
    pub outcome: StepOutcome,
    pub asks_for_hand_off: bool,
}

impl StepResult {
    /// An input that changed nothing and was refused.
    pub fn declined(state: &MachineSlot, decline: Decline) -> Self {
        Self {
            state: state.clone(),
            table_effects: Vec::new(),
            resource_requests: Vec::new(),
            outcome: StepOutcome::Settled(Outcome::Declined(decline)),
            asks_for_hand_off: false,
        }
    }

    /// An input that was applied, moving to `state` and proposing `table_effects`.
    pub fn applied(state: MachineSlot, table_effects: Vec<TableEffect>) -> Self {
        Self {
            state,
            table_effects,
            resource_requests: Vec::new(),
            outcome: StepOutcome::Settled(Outcome::Applied),
            asks_for_hand_off: false,
        }
    }

    /// The same result, with the machine asking the DM for a hand-off (RES21).
    pub fn asking_for_hand_off(mut self) -> Self {
        self.asks_for_hand_off = true;
        self
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
/// full audience-aware report arrives with `inspect` (C7).
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
    fn accepts(&self, input: &MachineInput, state: &MachineSlot) -> bool;

    fn step(&self, state: &MachineSlot, input: &StepInput) -> Result<StepResult, MachineError>;

    /// Where the table goes from here and what goes with it.
    fn exit(&self, state: &MachineSlot) -> Result<HandOff, MachineError>;

    /// The closed name of the state the slot holds; every name is one the spec lists.
    fn state_name(&self, state: &MachineSlot) -> Result<StateName, MachineError>;

    /// What the machine shows `audience`, never empty (stub until C5).
    fn project(
        &self,
        state: &MachineSlot,
        audience: Audience,
    ) -> Result<MachineReadout, MachineError>;

    /// The machine's report on itself (stub until C7).
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
    /// that its state can be read. An unmigratable or unreadable record is refused (H.12).
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
        self.state_name(&slot)
            .map_err(|_| LoadError::Malformed { version: expected })?;
        Ok(slot)
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
