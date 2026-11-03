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

use crate::effects::TableEffect;
use crate::element::{NameError, name_type};
use crate::envelope::PendingId;
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

name_type!(
    /// The name of one typed fact of a resolved action, such as `slashing_damage` (C.7, RES18).
    ResultLabel
);

/// One typed fact of a resolved action: a closed label and a whole number, from which the
/// structured readout is written ("Pip hits Dain Rusk for 7 slashing"). Never model text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultFact {
    pub label: ResultLabel,
    pub amount: u64,
}

/// What a resolved action did: the table effects the rules owner produced (RES18: mechanical
/// effects come only from the rules owner) and the typed facts of the result. APPLY commits the
/// effects with the machine's own, and the readout names the facts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub effects: Vec<TableEffect>,
    pub facts: Vec<ResultFact>,
}

impl Resolved {
    pub fn with_effects(effects: Vec<TableEffect>) -> Self {
        Self {
            effects,
            facts: Vec::new(),
        }
    }

    /// Adds a typed fact, refusing a label that is not a valid name.
    pub fn with_fact(mut self, label: &str, amount: u64) -> Result<Self, NameError> {
        self.facts.push(ResultFact {
            label: ResultLabel::new(label)?,
            amount,
        });
        Ok(self)
    }
}

/// What the rules decided about a prepared action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The action was resolved: what it changed and its typed facts.
    Applied(Resolved),
    Declined(Decline),
    /// The rules need more dice before they can decide, asked for one stage at a time (the
    /// roller's later stages, a damage roll only a hit reaches). The pipeline draws them and asks
    /// again with every die drawn so far.
    NeedsDraws(Vec<DrawRequest>),
    /// Someone must answer before the rules can go on: a window, a choice or a roll (C.4).
    Await(Awaiting),
}

impl Resolution {
    /// A resolved action that changed nothing the rules owner reports.
    pub fn applied() -> Self {
        Self::Applied(Resolved::default())
    }
}

/// What an `Awaiting` resolution waits for, which is the kind of pending it opens (RES26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AwaitingKind {
    /// The roller's own choice after a roll stage (M2-D4), or the order of simultaneous effects.
    Choice,
    /// A roll a player must make: the draws are made when the player answers `roll` (RES26).
    Roll,
    /// A window in which other seats may react (C.4).
    Reaction,
}

/// What one seat may answer, in the order the phone shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub seat: SeatId,
    pub choices: Vec<ActionId>,
}

/// A resolution that cannot go on until its offers are answered. Every seat with an offer answers
/// once; the rules owner is then asked to resume with all the answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaiting {
    pub kind: AwaitingKind,
    pub offers: Vec<Offer>,
    /// The dice a `Roll` makes when it is answered; empty for the other kinds.
    pub draws: Vec<DrawRequest>,
}

/// One seat's answer to its offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowAnswer {
    pub seat: SeatId,
    pub choice: ActionId,
}

/// Everything the rules owner is given to resume or finish a resolution: every die drawn so far in
/// order (recovery never draws again), the answers in the order of the offers, and a ruling that
/// applies.
#[derive(Debug, Clone, Copy)]
pub struct Resumption<'a> {
    pub draws: &'a [Draw],
    pub answers: &'a [WindowAnswer],
    pub ruling: Option<&'a RulingAnswer>,
}

name_type!(
    /// The normalised source-question key of a ruling, with the affordance id inside it (RES48).
    RulingKey
);
name_type!(
    /// One answer a ruling may have, a name the rules owner defines and validates (RES48).
    RulingAnswer
);

listed_enum! {
    /// Why an action needs a one-tap confirmation from its own seat (C.5).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ConfirmationReason {
        HarmsAnAlly,
        HarmsANeutral,
        Irreversible,
    }
}

impl ConfirmationReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::HarmsAnAlly => "harms_an_ally",
            Self::HarmsANeutral => "harms_a_neutral",
            Self::Irreversible => "irreversible",
        }
    }
}

listed_enum! {
    /// How much a ruling gives, as the source's tiers name it (RES48).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RulingTier {
        Ordinary,
        Advantage,
        AutomaticSuccess,
    }
}

impl RulingTier {
    pub fn key(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::Advantage => "advantage",
            Self::AutomaticSuccess => "automatic_success",
        }
    }

    /// A reuse that grants advantage or automatic success is confirmed once a scene (RES48).
    pub fn needs_reconfirmation(self) -> bool {
        match self {
            Self::Ordinary => false,
            Self::Advantage | Self::AutomaticSuccess => true,
        }
    }
}

listed_enum! {
    /// How far a ruling binds later play.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RulingScope {
        Table,
        /// Only the scene it was made in.
        Scene,
    }
}

/// An answer a ruling may take and what it gives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RulingOption {
    pub answer: RulingAnswer,
    pub tier: RulingTier,
}

/// Where the source leaves a real choice open (RES15): the question, the engine's suggestion and
/// the supported alternatives. A model never sets a die, a DC or an automatic success.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RulingRequest {
    pub key: RulingKey,
    pub scope: RulingScope,
    pub suggested: RulingOption,
    pub alternatives: Vec<RulingOption>,
    /// A reviewed, source-grounded automatic adjudication settles it at once (RES15).
    pub reviewed_automatic: bool,
}

listed_enum! {
    /// The dispositions that are not an action (C.8, K.1).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum NonActionKind {
        Question,
        Social,
        PlanOnly,
        Meta,
        Joke,
    }
}

impl NonActionKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Social => "social",
            Self::PlanOnly => "plan_only",
            Self::Meta => "meta",
            Self::Joke => "joke",
        }
    }
}

/// What a line of free text means, as a closed choice (C.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interpretation {
    /// One of the legal choices it was shown.
    Action(ActionId),
    /// An `ActionPlan`: the steps in order, at most six (C.0).
    Plan(Vec<ActionId>),
    NeedsClarification,
    /// Ambiguous between these choices: the phone shows exactly them (C.5, G.4).
    Ambiguous {
        candidates: Vec<ActionId>,
    },
    /// A question, a social line, a plan note, a rules question or a joke (C.8).
    NonAction(NonActionKind),
    /// Not usable as input; the pipeline declines `intent_invalid`.
    Rejected,
}

/// What the storyteller is asked to put into words: an outcome that is already committed. For a
/// plan, `steps` lists every applied step in order and `action` is the last of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedOutcome {
    pub seat: SeatId,
    pub action: ActionId,
    pub outcome: OutcomeKind,
    pub steps: Vec<ActionId>,
    pub facts: Vec<ResultFact>,
}

/// What the earlier steps of a plan have already done when a later step is checked (C.2): the
/// steps applied so far and the effects they staged, so the rules owner sees the economy they
/// spent and can decline the step that no longer fits.
#[derive(Debug, Clone, Copy)]
pub struct PlanSoFar<'a> {
    pub steps: &'a [ActionId],
    pub effects: &'a [TableEffect],
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

    /// Checks one step of a plan against what its earlier steps already did (C.2). An owner that
    /// tracks no economy says so by checking the step alone (`self.prepare(..)`): there is no
    /// default, so an adapter that forgot the economy does not compile.
    fn prepare_in_plan(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        so_far: &PlanSoFar<'_>,
    ) -> Result<Preparation, PortError>;

    /// Tells the owner that a resolution which awaited answers will not be resumed, so it may
    /// release what it reserved (its pending was cancelled, C.4). The pipeline calls it after the
    /// commit that cancelled the pending is durable, never while the stages run, so a refused
    /// batch has released nothing; `pending` names which reservation it is, so a pending that
    /// came back from a checkpoint is matched. An owner that reserved nothing returns `Ok(())`.
    fn abandon(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        pending: PendingId,
    ) -> Result<(), PortError>;

    /// Whether the action needs the seat's one-tap confirmation first (C.5): it harms an ally or a
    /// neutral, or cannot be undone. An owner that knows of none returns `Ok(None)`.
    fn confirmation_needed(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
    ) -> Result<Option<ConfirmationReason>, PortError>;

    /// The ruling the source leaves open for this action, if any (RES15). An owner with none
    /// returns `Ok(None)`.
    fn ruling_needed(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
    ) -> Result<Option<RulingRequest>, PortError>;

    /// Continues a resolution that was waiting for answers or a ruling, re-validating it against
    /// the state now (C.4). An owner with no windows and no rulings resolves with the dice drawn
    /// so far (`self.resolve(.., resumption.draws)`).
    fn resume(
        &self,
        correlation: Correlation,
        seat: SeatId,
        action: &ActionId,
        resumption: &Resumption<'_>,
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

    /// The one retry after a failure (C.7): a shorter prompt that carries only the legal choices.
    /// The default asks the same question again.
    fn retry_with_choices_only(
        &self,
        correlation: Correlation,
        seat: SeatId,
        text: &PlayerText,
        legal_choices: &[ActionId],
    ) -> Result<Interpretation, PortError> {
        self.interpret(correlation, seat, text, legal_choices)
    }
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
///
/// A draw is provisional until the host's commit: the pipeline draws while it runs an input in
/// memory, and a batch the host refuses must not have used those dice up, or the retried inputs
/// would draw other values. The host therefore gives back the draws of every refused turn,
/// newest turn first (`TurnResult::return_draws`), and the source serves them again, in the order
/// they were drawn, before it draws anything new. A replay stays deterministic because a retried
/// input meets the same values in the same order.
pub trait Dice: Send + Sync {
    fn draw(&self, correlation: Correlation, request: DrawRequest) -> Result<Draw, PortError>;

    /// Takes back the draws of one refused turn, in the order they were drawn: the next draws
    /// are these again, in this order. The host calls it for the newest refused turn first, so
    /// the oldest turn's draws end up first.
    fn give_back(&self, correlation: Correlation, draws: &[Draw]);
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
