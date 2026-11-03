//! The DM pipeline: the seven stages over the ports, run in memory and released after the host's
//! commit (C, RES15, RES16, C.7).
//!
//! An input served by the owner queue passes ADMIT (who sent it, may it run), INTERPRET (free text
//! to a registered plan, never a guess), CHECK (the rules owner: legal, needs a yes, needs a
//! ruling), DECIDE (the one draw point and the rules owner's resolution), APPLY (one commit of the
//! machine's step, the effects of the rules owner and the machine, the pending resolutions and the
//! rulings), NARRATE (the storyteller's flavour, staged as a job) and PROJECT (the structured
//! readout and one projection per audience). Every input ends in one of four outcomes: applied,
//! needs clarification, declined with a reason, or pending a choice, a roll, a reaction or a
//! ruling (RES16). Engine inputs (timers, resource results, proposals and controls) skip ADMIT's
//! seat checks and INTERPRET (RES21).
//!
//! How it runs (the group commit of O.16 S1): `Pipeline::run` changes the `PipelineTable` in
//! memory, so the next input of a batch sees it, and does nothing visible outside it before the
//! host's commit. What would be visible is staged in the result: the events of the changes
//! (`TurnResult::committed_events`, only for a turn that committed) and the lines that say what
//! the input came to and what it asked of the ports (`outcome_events`: a die drawn, a decline, a
//! clarification, a plan that stopped part way), both released by `release_events`; the resource
//! requests and the reservations a cancelled pending gave up (recorded as intents in the table and
//! told to the broker and the rules owner by `issue_requests`); and the narration job (run by the
//! host with the narrator and given back to `attach_narration`). Only the timing line of a stage
//! and a failure are recorded as the stages run. The dice a refused batch drew are given back
//! (`TurnResult::return_draws`), and a refused batch is undone by restoring the checkpoint the
//! host took before it (`PipelineTable::checkpoint`). A `TurnRecord` carries no commit identity:
//! the host stamps the batch commit. The pipeline holds no state of its own, and a duplicate
//! operation id is found in the result book (committed) or in the session's memory of how
//! uncommitted inputs ended, before any port is asked, so a replay draws no die, calls nothing,
//! and gives the seat the answer it got the first time without writing to the views again.
//!
//! Every port call carries the `Correlation` of the input, each stage records
//! `step.finished{step, machine, ms}` through the `Clock` port, and a failed port is recorded once,
//! at the stage that handles it, with a closed cause (O.6). A failed or slow model call never
//! guesses and never freezes the table: a failed interpreter becomes "I could not read that. Pick
//! one:" with the legal choices as buttons and the draft kept, a failed or late narrator leaves the
//! structured readout alone, and a failed rules owner or dice port declines the input with nothing
//! committed (C.7). It never reads a clock or draws a die except through its ports, never touches
//! the table except through `PipelineTable`, never searches the words of a line, and writes no
//! rules text: a decline's reason comes from the rules owner or from the code's default.
//!
//! Where it differs from the plan, so nobody is surprised: a plan that opens a pending ends at it
//! (C.2). Every pending, a confirmation and a ruling included, sets the machine to
//! `Waiting{pending_id}` (C.3, RES26, K.5), so a second pending for the same machine cannot stack,
//! and each ends by an answer, by the seat's own cancel or by `cancel_pending` (a seat that left
//! for good, a scene that changed, the host's decision).

use std::sync::Arc;

use crate::config::Settings;
use crate::element::NameError;
use crate::envelope::{InputEnvelope, InputOrigin, PendingId};
use crate::event::{PipelineStep, TableEvent};
use crate::identity::{Correlation, SeatId};
use crate::machine::Machine;
use crate::outcome::{Decline, DeclineCode, Outcome, OutcomeKind};
use crate::pending::StoredDraw;
use crate::ports::{
    ActionId, Broker, Clock, Dice, EventSink, Interpreter, KnowledgeView, Narrator, NonActionKind,
    RulesOwner,
};
use crate::projection::AudienceProjection;
use crate::registry::Registry;
use crate::table_state::TableSnapshot;

mod action;
mod admit;
mod answer;
mod apply;
mod books;
mod check;
mod decide;
mod engine;
pub mod events;
mod hold;
mod interpret;
mod narrate;
mod offers;
mod project;
mod record;
mod stored;
mod table_access;
mod turn;

pub use action::SPLIT_ACTION;
pub use books::{
    BookLoadError, ReleaseBook, ReleaseEntry, ReleaseRecord, RequestBook, RequestEntry,
    RequestRecord, ResultBook, ResultKey, ResultRecord, StoredClarification, StoredEnding,
    StoredOutcome, StoredResult, StoredSender, TakenRequest,
};
pub use events::{ClarifyCause, ViewPart};
pub use narrate::NarrationJob;
pub use record::{Readout, TurnRecord};
pub use table_access::{PipelineLoadError, PipelineRecord, PipelineTable};

use admit::{Admission, Reading};
use stored::replayed_outcome;
use table_access::Running;
use turn::Turn;

pub use crate::limits::{
    MAXIMUM_DRAW_ROUNDS, MAXIMUM_NARRATION_CHARS, MAXIMUM_OUTSTANDING_REQUESTS,
    MAXIMUM_PENDING_NESTING,
};

/// The most steps an action plan has (C.0). A rule of the plan, not an invented cap.
pub const MAXIMUM_PLAN_STEPS: usize = 6;

/// The name of the timer that sweeps the open pendings for the nudge line (J.4).
pub const PENDING_NUDGE_TIMER: &str = "pending_nudge";

/// The bounds of the pipeline and of the books it keeps in the table. Every default is a very
/// large named constant of `limits` (owner, 2026-11-02); a test or a measurement lowers one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineLimits {
    pub maximum_pending_nesting: u32,
    pub maximum_draw_rounds: u32,
    pub maximum_outstanding_requests: usize,
    pub maximum_narration_chars: usize,
    pub maximum_open_pendings: usize,
    pub maximum_stored_rulings: usize,
    pub maximum_remembered_results: usize,
}

impl Default for PipelineLimits {
    fn default() -> Self {
        Self {
            maximum_pending_nesting: crate::limits::MAXIMUM_PENDING_NESTING,
            maximum_draw_rounds: crate::limits::MAXIMUM_DRAW_ROUNDS,
            maximum_outstanding_requests: crate::limits::MAXIMUM_OUTSTANDING_REQUESTS,
            maximum_narration_chars: crate::limits::MAXIMUM_NARRATION_CHARS,
            maximum_open_pendings: crate::limits::MAXIMUM_OPEN_PENDINGS,
            maximum_stored_rulings: crate::limits::MAXIMUM_STORED_RULINGS,
            maximum_remembered_results: crate::limits::MAXIMUM_REMEMBERED_RESULTS,
        }
    }
}

/// The eight ports the pipeline works over (O.4). The narrator is not among the ones `run` calls:
/// the host runs the narration job with it and gives the answer to `attach_narration`.
#[derive(Clone, Copy)]
pub struct Ports<'a> {
    pub rules: &'a dyn RulesOwner,
    pub interpreter: &'a dyn Interpreter,
    pub narrator: &'a dyn Narrator,
    pub broker: &'a dyn Broker,
    pub clock: &'a dyn Clock,
    pub dice: &'a dyn Dice,
    pub knowledge: &'a dyn KnowledgeView,
    pub sink: &'a dyn EventSink,
}

/// Why a pipeline could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    /// One of the fixed answer names is not a valid name, which no build of this crate does.
    InvalidChoiceName(NameError),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidChoiceName(error) => {
                write!(formatter, "a fixed answer name is invalid: {error}")
            }
        }
    }
}

impl std::error::Error for PipelineError {}

/// The fixed names of the answers the pipeline itself offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    pub confirm: ActionId,
    pub cancel: ActionId,
    pub roll: ActionId,
    pub accept_suggestion: ActionId,
    pub decline_ruling: ActionId,
    pub cancel_ruling: ActionId,
}

impl Choices {
    fn new() -> Result<Self, NameError> {
        Ok(Self {
            confirm: ActionId::new("confirm")?,
            cancel: ActionId::new("cancel")?,
            roll: ActionId::new("roll")?,
            accept_suggestion: ActionId::new("accept_suggestion")?,
            decline_ruling: ActionId::new("decline_ruling")?,
            cancel_ruling: ActionId::new("cancel_ruling")?,
        })
    }
}

/// What the seat is asked to pick from instead of acting (C.5, C.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clarification {
    pub options: Vec<ActionId>,
    pub cause: ClarifyCause,
    /// The player's typed line stays in the phone's box (C.7).
    pub keeps_draft: bool,
}

/// What one input came to, for the host to commit, release and project.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    pub record: TurnRecord,
    pub outcome: Outcome,
    /// The structured line shown at once; never blank.
    pub readout: Readout,
    /// The narration to run after the commit: the host asks the narrator for it and gives the
    /// answer to `Pipeline::attach_narration`.
    pub narration_job: Option<NarrationJob>,
    pub clarification: Option<Clarification>,
    /// The pending the input opened or kept open.
    pub pending: Option<PendingId>,
    /// A plan stopped at a step after a valid prefix was applied: that step's decline is the
    /// question about it (RES26 `PartiallyCompleted`).
    pub unfinished: Option<Decline>,
    /// The line was not an action and was routed to the handler of its kind (C.8).
    pub routed: Option<NonActionKind>,
    /// The machine is finished and asks the DM for a hand-off (RES21); the host enqueues it as an
    /// engine proposal.
    pub hand_off_wanted: bool,
    /// The seat used the safety card: the table is paused (F.7) and the host flushes its queue.
    pub pause_requested: bool,
    /// One projection per audience, built from the table as this turn left it: the shared screen
    /// and every seat at the table, never empty (C5).
    pub projections: Vec<AudienceProjection>,
    /// The lines that say what the input came to and what it asked of the ports (a die drawn, a
    /// decline, a clarification, a routed line, a plan that stopped part way), released after the
    /// host's commit and not before, whether or not the turn changed the table.
    pub outcome_events: Vec<TableEvent>,
    /// The events of the changes this turn made, released after the host's commit and not before.
    /// Empty unless the turn committed.
    pub committed_events: Vec<TableEvent>,
}

impl TurnResult {
    /// Records the lines of this turn: what it came to, then what it changed. The host calls it
    /// after its commit succeeded, once per turn, and never for a refused batch.
    pub fn release_events(&self, sink: &dyn EventSink) {
        for event in self.outcome_events.iter().chain(&self.committed_events) {
            sink.record(event);
        }
    }

    /// The dice this turn drew, in order.
    pub fn draws(&self) -> Vec<crate::ports::Draw> {
        self.record
            .draws
            .iter()
            .filter_map(|stored| StoredDraw::restore(*stored).ok())
            .collect()
    }

    /// Gives this turn's dice back to the source: the host calls it for every turn of a batch it
    /// refused, newest turn first, so the retried inputs meet the same values in the same order.
    pub fn return_draws(&self, dice: &dyn Dice) {
        let draws = self.draws();
        if !draws.is_empty() {
            dice.give_back(self.record.correlation, &draws);
        }
    }
}

/// The DM pipeline over a set of ports, a registry and the settings. It holds no table state.
pub struct Pipeline<'a> {
    ports: Ports<'a>,
    registry: &'a Registry,
    settings: &'a Settings,
    limits: PipelineLimits,
    choices: Choices,
}

impl<'a> Pipeline<'a> {
    pub fn new(
        ports: Ports<'a>,
        registry: &'a Registry,
        settings: &'a Settings,
    ) -> Result<Self, PipelineError> {
        Ok(Self {
            ports,
            registry,
            settings,
            limits: PipelineLimits::default(),
            choices: Choices::new().map_err(PipelineError::InvalidChoiceName)?,
        })
    }

    pub fn with_limits(mut self, limits: PipelineLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The names of the answers the pipeline offers (confirm, cancel, roll, and the ruling taps).
    pub fn choices(&self) -> &Choices {
        &self.choices
    }

    /// The machine that runs and the state it saved, read from `snapshot`.
    fn active_machine<'s>(
        &self,
        table: &PipelineTable,
        snapshot: &'s TableSnapshot,
    ) -> Option<(&'a Arc<dyn Machine>, Running<'s>)> {
        let running = table.running(snapshot)?;
        let machine = self.registry.machine(running.id)?;
        Some((machine, running))
    }

    /// Runs one served input against the table in memory and says how it ended. Nothing it did is
    /// visible outside the table until the host commits and releases the result.
    pub fn run(
        &self,
        table: &mut PipelineTable,
        envelope: &InputEnvelope,
        correlation: Correlation,
    ) -> TurnResult {
        let mut turn = Turn::begin(
            self,
            table,
            envelope.kind(),
            envelope.sender(),
            envelope.seat(),
            correlation,
        );
        let operation = envelope.operation_id();
        // An operation id means something only together with its sender and that sender's
        // membership: another seat's id, or the id of a seat's earlier occupant, is a new input.
        if turn.is_unidentified() && !self.may_join_unseated(&turn, table, envelope) {
            // A seat that never sat has no membership to keep a result under; only the action a
            // machine declares for an unseated seat (the lobby's join) may run for it.
            let outcome = Outcome::Declined(Decline::with_default_reason(DeclineCode::NotViable));
            return self.finish(turn, table, outcome);
        }
        let key = turn.result_key();
        let remembered = table.results().find(key).or_else(|| table.ends().find(key));
        if let Some(stored) = remembered {
            let outcome = replayed_outcome(&mut turn, table, stored);
            return self.finish(turn, table, outcome);
        }
        if table.facts().has_applied(turn.sender, &operation) {
            // The table remembers an operation whose result was forgotten: it was applied, so it
            // is not run again and no die is drawn for it.
            turn.replayed = true;
            let outcome =
                Outcome::Declined(Decline::with_default_reason(DeclineCode::IntentInvalid));
            return self.finish(turn, table, outcome);
        }
        let outcome = match envelope.origin() {
            InputOrigin::Player { seat, input } => {
                self.run_player(&mut turn, table, envelope, *seat, input)
            }
            InputOrigin::Engine(input) => engine::run_engine(&mut turn, table, input),
        };
        self.finish(turn, table, outcome)
    }

    /// Whether the input is the action the active machine declares for a seat that is not at the
    /// table, the one way a seat that never sat can become seated.
    fn may_join_unseated(
        &self,
        turn: &Turn<'_, '_>,
        table: &PipelineTable,
        envelope: &InputEnvelope,
    ) -> bool {
        match envelope.origin() {
            InputOrigin::Player {
                input: crate::envelope::PlayerInput::Tap { action },
                ..
            } => admit::declared_for_unseated(turn, table, action),
            InputOrigin::Player { .. } | InputOrigin::Engine(_) => false,
        }
    }

    fn run_player(
        &self,
        turn: &mut Turn<'_, '_>,
        table: &mut PipelineTable,
        envelope: &InputEnvelope,
        seat: SeatId,
        input: &crate::envelope::PlayerInput,
    ) -> Outcome {
        let started = turn.now();
        let admission = admit::admit(turn, table, envelope, seat, input);
        let ended = match &admission {
            Admission::Declined(_) => Some(OutcomeKind::Declined),
            Admission::Pause => Some(OutcomeKind::Applied),
            Admission::Answer { .. } | Admission::Tap { .. } | Admission::Text(_) => None,
        };
        turn.stage_done(PipelineStep::Admit, started, ended);
        match admission {
            Admission::Declined(decline) => Outcome::Declined(decline),
            Admission::Pause => pause(turn, table),
            Admission::Answer { pending, choice } => {
                answer::answer(turn, table, seat, pending, choice)
            }
            Admission::Tap { action, reading } => action::run_plan(
                turn,
                table,
                seat,
                std::slice::from_ref(action),
                reading,
                action::StepMode::default(),
                None,
            ),
            Admission::Text(text) => match interpret::interpret(turn, table, seat, text) {
                interpret::Interpreted::Plan(plan) => action::run_plan(
                    turn,
                    table,
                    seat,
                    &plan,
                    Reading::Interpreted,
                    action::StepMode::default(),
                    None,
                ),
                interpret::Interpreted::Clarify(clarification) => {
                    turn.clarification = Some(clarification);
                    Outcome::NeedsClarification
                }
                interpret::Interpreted::Routed(kind) => {
                    turn.routed = Some(kind);
                    Outcome::Applied
                }
                interpret::Interpreted::Declined(decline) => Outcome::Declined(decline),
            },
        }
    }

    /// Tells the outside what the commit made due: issues to the broker the resource requests that
    /// were committed and not issued yet (each under the correlation of the input that asked, so
    /// the broker's work joins back to it) and binds each broker handle to its request, and tells
    /// the rules owner which reservations a cancelled pending gave up. The host calls it after its
    /// commit, never before: a refused batch leaves no request at the broker and no reservation
    /// released. Issuing is idempotent per request key (a request that already has a handle is not
    /// asked for again), so a restart that kept the intents issues what was missing. A request the
    /// broker refuses, or a release the rules owner refuses, is recorded once and stays due for
    /// the next call. Returns how many requests were issued.
    pub fn issue_requests(&self, table: &mut PipelineTable) -> usize {
        let mut issued = 0;
        for entry in table.requests().unissued() {
            let started = self.ports.clock.now_ms();
            let asked_by = entry.asked_by();
            match self.ports.broker.request(asked_by, entry.purpose()) {
                Ok(handle) => {
                    table
                        .requests_mut()
                        .bind_handle(entry.machine(), entry.key(), handle.get());
                    issued += 1;
                }
                Err(error) => self.ports.sink.record(&TableEvent::BoundaryFailed {
                    scope: table.scope(),
                    correlation: asked_by,
                    port: error.port,
                    cause: error.cause,
                    ms: self.ports.clock.now_ms().saturating_sub(started),
                }),
            }
        }
        for entry in table.releases().due() {
            let started = self.ports.clock.now_ms();
            let asked_by = entry.asked_by();
            match self.ports.rules.abandon(
                asked_by,
                entry.seat(),
                entry.action(),
                PendingId::new(entry.pending()),
            ) {
                Ok(()) => table.releases_mut().done(entry.pending()),
                Err(error) => self.ports.sink.record(&TableEvent::BoundaryFailed {
                    scope: table.scope(),
                    correlation: asked_by,
                    port: error.port,
                    cause: error.cause,
                    ms: self.ports.clock.now_ms().saturating_sub(started),
                }),
            }
        }
        issued
    }
}

/// The safety card pauses the table (F.7): a committed `TablePaused`, no model call, no check.
fn pause(turn: &mut Turn<'_, '_>, table: &mut PipelineTable) -> Outcome {
    turn.pause_requested = true;
    let mut staged = apply::Staged::new();
    staged
        .effects
        .push(crate::effects::TableEffect::TablePaused);
    match apply::commit(turn, table, staged) {
        Ok(()) => Outcome::Applied,
        Err(decline) => Outcome::Declined(decline),
    }
}
