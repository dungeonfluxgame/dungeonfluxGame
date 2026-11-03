//! The working context of one input while it passes through the stages.
//!
//! `Turn` carries what every stage needs and what the stages hand to each other: the correlation
//! every port call receives, the scope of the events, the clock reading the input started at, the
//! dice drawn so far, the effects committed, and the events that may only be recorded once the
//! host's commit has happened. Those are of two kinds: `on_commit` holds the events that say the
//! table changed (a pending opened, a ruling stored, the effects, `TurnApplied`) and is released
//! only when the turn committed, and `outcome_events` holds the lines that say what the input came
//! to and what it asked of the ports (a die drawn, a decline, a clarification, a routed line, a
//! plan that stopped part way), which are released with the turn whether or not it changed the
//! table. Nothing of either kind is recorded while the stages run, so a refused batch leaves no
//! line behind (O.16 S1). Only the timing line of a stage (`step.finished`) and a failure
//! (`BoundaryFailed`, `MachineFailed`, `EffectRefused`) are recorded at once: they happened
//! whatever the commit does.
//!
//! A stage that may fail after it staged something takes a `StageMark` first and rewinds to it, so
//! a step that is declined takes its staged lines and its ruling note with it. `Turn` never decides
//! what a stage does and holds nothing across inputs: a new `Turn` is built for every input, so the
//! pipeline keeps no state of its own.

use crate::effects::TableEffect;
use crate::envelope::{InputKind, PendingId, Sender};
use crate::event::{FailureCause, PipelineStep, TableEvent};
use crate::identity::{Correlation, SeatId, TableScope};
use crate::outcome::{Decline, OutcomeKind};
use crate::pending::StoredDraw;
use crate::ports::{ActionId, NonActionKind, PortName, ResultFact, RulingAnswer};

use super::events::PipelineEvent;
use super::table_access::PipelineTable;
use super::{Clarification, NarrationJob, Pipeline};

pub(super) struct Turn<'t, 'p> {
    pub(super) pipeline: &'t Pipeline<'p>,
    pub(super) correlation: Correlation,
    pub(super) kind: InputKind,
    pub(super) sender: Sender,
    pub(super) seat: Option<SeatId>,
    pub(super) scope: TableScope,
    pub(super) started_ms: u64,
    pub(super) hash_before: u64,
    pub(super) durable_before: u64,
    pub(super) draws: Vec<StoredDraw>,
    pub(super) effects: Vec<TableEffect>,
    /// Events about changes that exist only once the host's commit succeeded: the host releases
    /// them with the turn's result, and nothing records them before (O.16 S1). Only a turn that
    /// committed releases them.
    pub(super) on_commit: Vec<TableEvent>,
    /// The lines that say what the input came to and what it asked of the ports, released with the
    /// turn's result whether or not it changed the table, and never before.
    pub(super) outcome_events: Vec<TableEvent>,
    /// The table accepted this turn's commit.
    pub(super) committed: bool,
    /// The table was idle: the turn's first commit resumes it.
    pub(super) wake_idle: bool,
    /// The steps applied, in order, and the typed facts their results carried.
    pub(super) applied_steps: Vec<ActionId>,
    pub(super) result_facts: Vec<ResultFact>,
    /// A line that is not an action and was routed to its handler (C.8).
    pub(super) routed: Option<NonActionKind>,
    pub(super) pending: Option<PendingId>,
    pub(super) clarification: Option<Clarification>,
    pub(super) narration: Option<NarrationJob>,
    /// The seat and action the readout and the narration are about.
    pub(super) acted: Option<(SeatId, ActionId)>,
    /// A stored ruling that answered this input, shown as "Ruling (earlier)" (C.3).
    pub(super) ruling_note: Option<RulingAnswer>,
    /// The input cancelled a held action: the uncommitted part was dropped (RES26).
    pub(super) cancelled: bool,
    /// The decline of the step that stopped a plan after a valid prefix was applied (C.2).
    pub(super) unfinished: Option<Decline>,
    pub(super) hand_off_wanted: bool,
    pub(super) pause_requested: bool,
    pub(super) replayed: bool,
    /// The line the seat was shown the first time, when this is a duplicate delivery.
    pub(super) replayed_readout: Option<String>,
    /// Seats other than the sender whose own readout this turn changes: the seat whose pending
    /// it closed, so its phone does not keep saying it is waiting.
    pub(super) also_told: Vec<SeatId>,
}

/// How far the staged lines and the ruling note had got, to rewind to if a step is declined.
pub(super) struct StageMark {
    on_commit: usize,
    outcome_events: usize,
    ruling_note: Option<RulingAnswer>,
}

impl<'t, 'p> Turn<'t, 'p> {
    pub(super) fn begin(
        pipeline: &'t Pipeline<'p>,
        table: &PipelineTable,
        kind: InputKind,
        sender: Sender,
        seat: Option<SeatId>,
        correlation: Correlation,
    ) -> Self {
        Self {
            pipeline,
            correlation,
            kind,
            sender,
            seat,
            scope: table.scope(),
            started_ms: pipeline.ports.clock.now_ms(),
            hash_before: table.state_hash(),
            durable_before: table.durable_hash(),
            draws: Vec::new(),
            effects: Vec::new(),
            on_commit: Vec::new(),
            outcome_events: Vec::new(),
            committed: false,
            wake_idle: false,
            applied_steps: Vec::new(),
            result_facts: Vec::new(),
            routed: None,
            pending: None,
            clarification: None,
            narration: None,
            acted: None,
            ruling_note: None,
            cancelled: false,
            unfinished: None,
            hand_off_wanted: false,
            pause_requested: false,
            replayed: false,
            replayed_readout: None,
            also_told: Vec::new(),
        }
    }

    /// Where the staged lines stand now.
    pub(super) fn mark(&self) -> StageMark {
        StageMark {
            on_commit: self.on_commit.len(),
            outcome_events: self.outcome_events.len(),
            ruling_note: self.ruling_note.clone(),
        }
    }

    /// Drops what was staged since `mark`, and the ruling note a declined step set.
    pub(super) fn rewind(&mut self, mark: StageMark) {
        self.on_commit.truncate(mark.on_commit);
        self.outcome_events.truncate(mark.outcome_events);
        self.ruling_note = mark.ruling_note;
    }

    /// Stages a line about what the input came to, to be released with the turn.
    pub(super) fn stage_outcome(&mut self, event: TableEvent) {
        self.outcome_events.push(event);
    }

    pub(super) fn now(&self) -> u64 {
        self.pipeline.ports.clock.now_ms()
    }

    pub(super) fn elapsed_since(&self, started_ms: u64) -> u64 {
        self.now().saturating_sub(started_ms)
    }

    pub(super) fn emit(&self, event: &TableEvent) {
        self.pipeline.ports.sink.record(event);
    }

    pub(super) fn pipeline_event(
        &self,
        build: impl FnOnce(TableScope, Correlation) -> PipelineEvent,
    ) -> TableEvent {
        TableEvent::Pipeline(build(self.scope.clone(), self.correlation))
    }

    /// Records that a stage finished (`step.finished`): `outcome` is set when the stage ended the
    /// input and absent when it passed it on.
    pub(super) fn stage_done(
        &self,
        step: PipelineStep,
        started_ms: u64,
        outcome: Option<OutcomeKind>,
    ) {
        self.emit(&TableEvent::StepFinished {
            scope: self.scope.clone(),
            correlation: self.correlation,
            step,
            outcome,
            ms: self.elapsed_since(started_ms),
        });
    }

    /// Records a port failure, once, at the stage that handles it.
    pub(super) fn boundary_failed(&self, port: PortName, cause: FailureCause, started_ms: u64) {
        self.emit(&TableEvent::BoundaryFailed {
            scope: self.scope.clone(),
            correlation: self.correlation,
            port,
            cause,
            ms: self.elapsed_since(started_ms),
        });
    }
}
