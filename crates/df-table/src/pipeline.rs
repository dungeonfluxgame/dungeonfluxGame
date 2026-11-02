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
//! (`TurnResult::committed_events`, released by `release_events`), the resource requests (recorded
//! as intents in the table and issued by `issue_requests`) and the narration job (run by the host
//! with the narrator and given back to `attach_narration`). A refused batch is undone by restoring
//! the checkpoint the host took before it (`PipelineTable::checkpoint`). A `TurnRecord` carries no
//! commit identity: the host stamps the batch commit. The pipeline holds no state of its own, and
//! a duplicate operation id that was committed is found in the result book before any port is
//! asked, so a replay draws no die and calls nothing.
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
//! (C.2); a confirmation and a ruling hold the action in the pending and do not set the machine to
//! `Waiting`, because a refused ruling would leave a machine waiting, while a window, a choice and
//! a roll do, and they can end by `cancel_pending`.

use std::sync::Arc;

use crate::config::Settings;
use crate::element::NameError;
use crate::envelope::{InputEnvelope, InputOrigin, PendingId};
use crate::event::{PipelineStep, TableEvent};
use crate::identity::{Correlation, SeatId, SessionRevision};
use crate::machine::Machine;
use crate::outcome::{Decline, DeclineCode, Outcome, OutcomeKind, PendingKind};
use crate::ports::{
    ActionId, Audience, Broker, Clock, Dice, EventSink, Interpreter, KnowledgeView, Narrator,
    NonActionKind, RulesOwner,
};
use crate::projection::{
    AttachmentError, Attachments, AudienceProjection, NonEmptyText, ProjectionStatus, StoryLine,
    project,
};
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
mod interpret;
mod narrate;
mod record;
mod table_access;
mod turn;

pub use action::SPLIT_ACTION;
pub use books::{
    BookLoadError, RequestBook, RequestEntry, RequestRecord, ResultBook, ResultRecord,
    StoredOutcome, TakenRequest,
};
pub use events::ClarifyCause;
pub use narrate::NarrationJob;
pub use record::{Readout, TurnRecord};
pub use table_access::{PipelineLoadError, PipelineRecord, PipelineTable};

use admit::{Admission, Reading};
use events::PipelineEvent;
use record::{applied_line, routed_line};
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
    /// The events of the changes this turn made, released after the host's commit and not before.
    pub committed_events: Vec<TableEvent>,
}

impl TurnResult {
    /// Records the events of this turn's changes. The host calls it after its commit succeeded.
    pub fn release_events(&self, sink: &dyn EventSink) {
        for event in &self.committed_events {
            sink.record(event);
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
        if let Some(stored) = table.results().find(envelope.operation_id()) {
            let outcome = replayed_outcome(&mut turn, table, stored);
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

    /// PROJECT: the structured readout, the projections, the events that explain the end, and the
    /// record.
    fn finish(
        &self,
        mut turn: Turn<'_, '_>,
        table: &mut PipelineTable,
        outcome: Outcome,
    ) -> TurnResult {
        let started = turn.now();
        self.record_end(&mut turn, table, &outcome);
        if turn.committed {
            let stored = stored_outcome(&turn, &outcome);
            table
                .results_mut()
                .record(turn.correlation.operation_id, stored);
        }
        let readout = Readout::new(self.readout_text(&turn, table, &outcome));
        let projections = project_audiences(&turn, table, &outcome, &readout);
        turn.stage_done(PipelineStep::Project, started, Some(outcome.kind()));
        let declined = match &outcome {
            Outcome::Declined(decline) => Some(decline.code()),
            Outcome::Applied | Outcome::NeedsClarification | Outcome::Pending(_) => None,
        };
        let record = TurnRecord {
            correlation: turn.correlation,
            sender: turn.sender,
            kind: turn.kind,
            outcome: outcome.kind(),
            declined,
            pending: turn.pending,
            draws: turn.draws,
            effects: turn.effects,
            state_hash_before: turn.hash_before,
            state_hash_after: table.state_hash(),
            replayed: turn.replayed,
        };
        TurnResult {
            record,
            outcome,
            readout,
            narration_job: turn.narration,
            clarification: turn.clarification,
            pending: turn.pending,
            unfinished: turn.unfinished,
            routed: turn.routed,
            hand_off_wanted: turn.hand_off_wanted,
            pause_requested: turn.pause_requested,
            projections,
            committed_events: turn.on_commit,
        }
    }

    /// Records how the input ended, once: a decline, a clarification, a routed line and the dice
    /// drawn. These say what was asked of the ports and what the seat was told; they are not
    /// changes of the table, so they are recorded now.
    fn record_end(&self, turn: &mut Turn<'_, '_>, table: &mut PipelineTable, outcome: &Outcome) {
        if !turn.draws.is_empty() {
            turn.emit(
                &turn.pipeline_event(|scope, correlation| PipelineEvent::DiceDrawn {
                    scope,
                    correlation,
                    count: turn.draws.len(),
                }),
            );
        }
        if let Some(kind) = turn.routed {
            turn.emit(
                &turn.pipeline_event(|scope, correlation| PipelineEvent::InputRouted {
                    scope,
                    correlation,
                    kind,
                }),
            );
        }
        match outcome {
            Outcome::Declined(decline) => turn.emit(&TableEvent::InputDeclined {
                scope: turn.scope.clone(),
                correlation: turn.correlation,
                code: decline.code(),
            }),
            Outcome::NeedsClarification => {
                if let Some(clarification) = &turn.clarification {
                    let (options, cause) = (clarification.options.len(), clarification.cause);
                    turn.emit(&turn.pipeline_event(|scope, correlation| {
                        PipelineEvent::InputClarified {
                            scope,
                            correlation,
                            options,
                            cause,
                        }
                    }));
                    if let Some(seat) = turn.seat {
                        table.set_clarification(seat, clarification.options.clone());
                    }
                }
            }
            Outcome::Applied | Outcome::Pending(_) => {}
        }
    }

    fn readout_text(
        &self,
        turn: &Turn<'_, '_>,
        table: &PipelineTable,
        outcome: &Outcome,
    ) -> String {
        match outcome {
            Outcome::Declined(decline) => declined_line(decline),
            Outcome::NeedsClarification => match turn.clarification.as_ref().map(|c| c.cause) {
                Some(ClarifyCause::Ambiguous) => "Which did you mean? Pick one:".to_owned(),
                Some(ClarifyCause::Unclear | ClarifyCause::Unreadable) | None => {
                    "I could not read that. Pick one:".to_owned()
                }
            },
            Outcome::Pending(kind) => pending_line(table, turn.pending, *kind),
            Outcome::Applied => applied_text(turn),
        }
    }

    /// Issues to the broker the resource requests that were committed and not issued yet, and
    /// binds each broker handle to its request. The host calls it after its commit, never before:
    /// a refused batch leaves no request at the broker. Issuing is idempotent per request key (a
    /// request that already has a handle is not asked for again), so a restart that kept the
    /// intents issues what was missing. A request the broker refuses is recorded once and stays
    /// unissued for the next call. Returns how many were issued.
    pub fn issue_requests(&self, table: &mut PipelineTable, correlation: Correlation) -> usize {
        let mut issued = 0;
        for entry in table.requests().unissued() {
            let started = self.ports.clock.now_ms();
            match self.ports.broker.request(correlation, entry.purpose()) {
                Ok(handle) => {
                    table
                        .requests_mut()
                        .bind_handle(entry.machine(), entry.key(), handle.get());
                    issued += 1;
                }
                Err(error) => self.ports.sink.record(&TableEvent::BoundaryFailed {
                    scope: table.scope(),
                    correlation,
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

/// The outcome of an operation that was committed before: found in the result book, so no port
/// is asked and no die is drawn.
fn replayed_outcome(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    stored: StoredOutcome,
) -> Outcome {
    turn.replayed = true;
    match stored {
        StoredOutcome::Applied => Outcome::Applied,
        StoredOutcome::Declined { code } => Outcome::Declined(Decline::with_default_reason(code)),
        StoredOutcome::Pending { pending, kind } => {
            let id = PendingId::new(pending);
            if table.pending().get(id).is_some() {
                turn.pending = Some(id);
                Outcome::Pending(kind)
            } else {
                // The pending resolved since: telling the seat it is still waiting would be a
                // pending nobody can answer.
                Outcome::Declined(Decline::with_default_reason(DeclineCode::WindowClosed))
            }
        }
    }
}

fn stored_outcome(turn: &Turn<'_, '_>, outcome: &Outcome) -> StoredOutcome {
    match (outcome, turn.pending) {
        (Outcome::Pending(kind), Some(id)) => StoredOutcome::Pending {
            pending: id.get(),
            kind: *kind,
        },
        (Outcome::Declined(decline), _) => StoredOutcome::Declined {
            code: decline.code(),
        },
        (Outcome::Applied | Outcome::NeedsClarification | Outcome::Pending(_), _) => {
            StoredOutcome::Applied
        }
    }
}

/// The line for a decline: the rules owner's reason, then what would be possible (C.6).
fn declined_line(decline: &Decline) -> String {
    let mut line = decline.reason().to_owned();
    let alternatives = decline.alternatives();
    let Some((last, earlier)) = alternatives.split_last() else {
        return line;
    };
    line.push_str(" You could ");
    match earlier.split_last() {
        None => line.push_str(last),
        Some((only, [])) => line.push_str(&format!("{only} or {last}")),
        Some(_) => {
            for alternative in earlier {
                line.push_str(alternative);
                line.push_str(", ");
            }
            line.push_str(&format!("or {last}"));
        }
    }
    line.push('.');
    line
}

fn applied_text(turn: &Turn<'_, '_>) -> String {
    if let Some(kind) = turn.routed {
        return routed_line(kind).to_owned();
    }
    let mut line = String::new();
    if let Some(answer) = &turn.ruling_note {
        line.push_str(&format!(
            "Ruling (earlier): {}. ",
            answer.as_str().replace(['_', '-'], " ")
        ));
    }
    if turn.cancelled {
        line.push_str("Cancelled.");
    } else if let Some((seat, action)) = &turn.acted {
        line.push_str(&applied_line(*seat, action, &turn.result_facts));
    } else {
        line.push_str("Done.");
    }
    if let Some(decline) = &turn.unfinished {
        line.push(' ');
        line.push_str(decline.reason());
    }
    line
}

fn pending_line(table: &PipelineTable, pending: Option<PendingId>, kind: PendingKind) -> String {
    let waiting_on = pending
        .and_then(|id| table.pending().get(id))
        .and_then(|held| held.window().awaiting().next());
    match (kind, waiting_on) {
        (PendingKind::Ruling, _) => "Needs a ruling.".to_owned(),
        (PendingKind::Choice, Some(seat)) => format!("Waiting for seat {seat} to choose."),
        (PendingKind::Roll, Some(seat)) => format!("Waiting for seat {seat} to roll."),
        (PendingKind::Reaction, Some(seat)) => {
            format!("Waiting for seat {seat} to answer a reaction.")
        }
        (_, None) => "Waiting for an answer.".to_owned(),
    }
}

/// The activity every view shows while the table waits on a person (K.5), or `Ok` again.
fn activity_of(
    table: &PipelineTable,
    outcome: &Outcome,
    pending: Option<PendingId>,
) -> ProjectionStatus {
    let Outcome::Pending(kind) = outcome else {
        return ProjectionStatus::Ok;
    };
    let waiting_on = pending
        .and_then(|id| table.pending().get(id))
        .and_then(|held| held.window().awaiting().next());
    let (who, what) = match (kind, waiting_on) {
        (PendingKind::Ruling, _) => ("the host".to_owned(), "to rule"),
        (PendingKind::Choice, Some(seat)) => (format!("seat {seat}"), "to choose"),
        (PendingKind::Roll, Some(seat)) => (format!("seat {seat}"), "to roll"),
        (PendingKind::Reaction, Some(seat)) => (format!("seat {seat}"), "to answer a reaction"),
        (_, None) => ("the table".to_owned(), "to answer"),
    };
    ProjectionStatus::waiting_for(&who, what).unwrap_or(ProjectionStatus::Degraded)
}

/// Publishes what this turn said to the attachments the projections read, then projects the
/// shared screen and every seat at the table from the table as the turn left it. An attachment
/// that is refused (a text over the wire frame, a bundle at its bound) leaves the views at the
/// overview with a `degraded` status, which says the table could not show more, and the turn is
/// not failed by it.
fn project_audiences(
    turn: &Turn<'_, '_>,
    table: &mut PipelineTable,
    outcome: &Outcome,
    readout: &Readout,
) -> Vec<AudienceProjection> {
    let text = NonEmptyText::or_fallback(readout.as_str());
    let activity = activity_of(table, outcome, turn.pending);
    let revision = table.facts().snapshot().revision();
    let attachments = table.attachments_mut();
    if publish(attachments, turn, outcome, text, activity, revision).is_err() {
        // The status is a closed value; if even it is refused the views keep the last one.
        let _ = attachments.set_activity(ProjectionStatus::Degraded);
    }
    let snapshot = table.facts().snapshot();
    let attachments = table.attachments();
    let mut projections = Vec::new();
    projections.extend(project(&snapshot, attachments, Audience::Public).ok());
    for seat in snapshot.seats().iter() {
        projections.extend(project(&snapshot, attachments, Audience::Seat(*seat)).ok());
    }
    projections
}

fn publish(
    attachments: &mut Attachments,
    turn: &Turn<'_, '_>,
    outcome: &Outcome,
    text: NonEmptyText,
    activity: ProjectionStatus,
    revision: SessionRevision,
) -> Result<(), AttachmentError> {
    attachments.set_activity(activity)?;
    match outcome {
        Outcome::Applied => {
            attachments.set_shared_readout(text.clone())?;
            if let Some(seat) = turn.seat {
                attachments.set_seat_readout(seat, text.clone())?;
            }
            attachments.push_public(StoryLine::new(text))?;
        }
        Outcome::Declined(decline) => {
            if let Some(seat) = turn.seat {
                attachments.set_notice(seat, decline.clone(), revision)?;
                attachments.set_seat_readout(seat, text)?;
            }
        }
        Outcome::NeedsClarification | Outcome::Pending(_) => {
            if let Some(seat) = turn.seat {
                attachments.set_seat_readout(seat, text)?;
            }
        }
    }
    Ok(())
}
