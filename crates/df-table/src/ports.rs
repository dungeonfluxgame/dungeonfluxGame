//! The seams between this crate and everything outside it (O.4).
//!
//! It owns the eight ports (`RulesOwner`, `Interpreter`, `Narrator`, `Broker`, `Clock`, `Dice`,
//! `KnowledgeView`, `EventSink`) and the small values they exchange. A port answers its question
//! and never waits: no method is async and none blocks, so a slow owner is a `Broker` request that
//! reports its readiness later, not a call that holds the owner thread. Every port except `Clock`
//! and `EventSink` takes the `Correlation` of the input it serves, so its work joins back to that
//! input. Each port is implemented twice, by an adapter outside the crate and by a fake in
//! `testkit`, and the shared ones are `Send + Sync`.
//!
//! A port never panics and never fabricates an answer. When it cannot answer it returns a
//! `PortError` naming which port failed and a closed `FailureCause`; the pipeline records that once,
//! at the boundary that handles it. `EventSink` is the exception: a sink that cannot record drops
//! the event and counts the loss itself. `Clock` and `EventSink` take no `Correlation`: time
//! belongs to no input, and an event carries its own correlation.
//!
//! The values here are the smallest that make each question askable. They grow additively as the
//! machines and the pipeline need more (a spatial query, rest and time costs, a speech envelope).

use std::fmt;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::element::name_type;
use crate::event::{FailureCause, TableEvent};
use crate::identity::{Correlation, FactId, SeatId};
use crate::listed::listed_enum;
use crate::outcome::{Decline, OutcomeKind};
use crate::private_data::{Narration, PlayerText, Secret};

name_type!(
    /// The registry name of an action, which is also the key of its rules entry (O.5). It is a
    /// name, not an identity: the action registry owns the set.
    ActionId
);
name_type!(
    /// The purpose of a resource request, such as `plate`; the broker holds its deadline and class.
    PurposeName
);

listed_enum! {
    /// The ports that can fail. `Clock` and `EventSink` cannot, so they are not listed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PortName {
        RulesOwner,
        Interpreter,
        Narrator,
        Broker,
        Dice,
        KnowledgeView,
    }
}

impl PortName {
    pub fn key(self) -> &'static str {
        match self {
            Self::RulesOwner => "rules_owner",
            Self::Interpreter => "interpreter",
            Self::Narrator => "narrator",
            Self::Broker => "broker",
            Self::Dice => "dice",
            Self::KnowledgeView => "knowledge_view",
        }
    }
}

/// A port could not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortError {
    pub port: PortName,
    pub cause: FailureCause,
}

impl PortError {
    pub fn new(port: PortName, cause: FailureCause) -> Self {
        Self { port, cause }
    }
}

impl fmt::Display for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the {} port failed: {}",
            self.port.key(),
            self.cause.key()
        )
    }
}

impl std::error::Error for PortError {}

/// Who a question about knowledge is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Audience {
    /// Everyone at the table.
    Public,
    /// One seat: its public facts and its own.
    Seat(SeatId),
}

/// One die to roll: the number of sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawRequest {
    pub sides: NonZeroU16,
}

/// One die that was rolled. The value is always between 1 and the number of sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    sides: NonZeroU16,
    value: u16,
}

impl Draw {
    /// `None` when the value is not on a die of that many sides.
    pub fn new(sides: NonZeroU16, value: u16) -> Option<Self> {
        (1..=sides.get())
            .contains(&value)
            .then_some(Self { sides, value })
    }

    pub fn sides(self) -> NonZeroU16 {
        self.sides
    }

    pub fn value(self) -> u16 {
        self.value
    }
}

/// What the rules need before an action can be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preparation {
    /// The dice to roll, in order; empty when the action needs none.
    Ready { draws: Vec<DrawRequest> },
    /// The rules owner is the only party that knows what is missing and what would be legal, so
    /// the whole decline travels (RES16, O.7).
    Declined(Decline),
}

/// What the rules decided about a prepared action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Applied,
    Declined(Decline),
}

/// What a line of free text means, as a closed choice (C.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interpretation {
    /// One of the legal choices it was shown.
    Action(ActionId),
    NeedsClarification,
    /// Not usable as input; the pipeline declines `intent_invalid`.
    Rejected,
}

/// What the storyteller is asked to put into words: an outcome that is already committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedOutcome {
    pub seat: SeatId,
    pub action: ActionId,
    pub outcome: OutcomeKind,
}

/// A resource request the broker accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceHandle(u64);

impl ResourceHandle {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// How far a resource request has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Pending,
    Ready,
    Failed(FailureCause),
}

/// The rules owner: what is legal, and what a legal action does, with explicit draws.
pub trait RulesOwner: Send + Sync {
    /// The actions the rules allow this seat now.
    fn legal_choices(
        &self,
        correlation: Correlation,
        seat: SeatId,
    ) -> Result<Vec<ActionId>, PortError>;

    /// Checks an action and names the dice it needs, or declines it.
    fn prepare(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
    ) -> Result<Preparation, PortError>;

    /// Resolves a prepared action with the dice that were drawn for it.
    fn resolve(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        draws: &[Draw],
    ) -> Result<Resolution, PortError>;
}

/// The interpreter: free text to a closed choice, never to a guess.
pub trait Interpreter: Send + Sync {
    fn interpret(
        &self,
        correlation: Correlation,
        seat: SeatId,
        text: &PlayerText,
        legal_choices: &[ActionId],
    ) -> Result<Interpretation, PortError>;
}

/// The narrator: a committed outcome to words.
pub trait Narrator: Send + Sync {
    fn narrate(
        &self,
        correlation: Correlation,
        committed: &CommittedOutcome,
    ) -> Result<Narration, PortError>;
}

/// The broker: asks for a resource and reports its readiness, and never waits for it.
pub trait Broker: Send + Sync {
    fn request(
        &self,
        correlation: Correlation,
        purpose: &PurposeName,
    ) -> Result<ResourceHandle, PortError>;

    fn readiness(
        &self,
        correlation: Correlation,
        handle: ResourceHandle,
    ) -> Result<Readiness, PortError>;
}

/// The clock: monotonic time, in milliseconds. It takes no correlation because time belongs to
/// no input.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// The dice: the one draw point. Recovery never draws again (coding-style section 8, pattern 5).
pub trait Dice: Send + Sync {
    fn draw(&self, correlation: Correlation, request: DrawRequest) -> Result<Draw, PortError>;
}

/// The knowledge view: what an audience may know. `None` means the audience may not know it, which
/// is different from a failure.
pub trait KnowledgeView: Send + Sync {
    fn fact_for(
        &self,
        correlation: Correlation,
        audience: Audience,
        fact: &FactId,
    ) -> Result<Option<Secret<String>>, PortError>;
}

/// Receives every typed event the table produces.
///
/// An implementation must not block and must not fail the caller: a sink that cannot record
/// drops the event and counts the loss itself. An event carries its own correlation, so this
/// port takes none.
pub trait EventSink: Send + Sync {
    fn record(&self, event: &TableEvent);
}
