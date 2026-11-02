//! The working context of one input while it passes through the stages.
//!
//! `Turn` carries what every stage needs and what the stages hand to each other: the correlation
//! every port call receives, the scope of the events, the clock reading the input started at, the
//! dice drawn so far, the effects committed, and the events that may only be recorded once the
//! commit has happened (a pending opened, a ruling stored). It owns the one way a stage records
//! its end (`step.finished`) and the one way a port failure is recorded (`BoundaryFailed`, once,
//! by the stage that handles it). It never decides what a stage does and holds nothing across
//! inputs: a new `Turn` is built for every input, so the pipeline keeps no state of its own.

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
    pub(super) draws: Vec<StoredDraw>,
    pub(super) effects: Vec<TableEffect>,
    /// Events about changes that exist only once the host's commit succeeded: the host releases
    /// them with the turn's result, and nothing records them before (O.16 S1).
    pub(super) on_commit: Vec<TableEvent>,
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
            draws: Vec::new(),
            effects: Vec::new(),
            on_commit: Vec::new(),
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
        }
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
