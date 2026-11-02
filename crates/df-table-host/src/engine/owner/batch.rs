//! Serving the queue in batches with group commit (S1 decision), over the real pipeline.
//!
//! A batch serves inputs in the queue's total order until the queue is empty, the policy's time
//! slice or count is reached, and runs each through `Pipeline::run` against a checkpoint of the
//! committed table. The pipeline stages everything a turn changes in that copy and holds the
//! events and views in the turn's result. Then one checkpoint is written (`save`). Only after the
//! write succeeds do the copy become the committed table, the staged events reach the log, each
//! input finish in the queue, each narration get asked for, each committed resource request go to
//! the broker, and each report carry the commit's id and the sender's view. A write that fails or
//! misses its deadline fails the whole batch: the copy, the staged events and the views are
//! dropped, every input ends `declined` with code `table_busy` and its operation id is released so
//! the same id may be sent again (H.4), and a `COMMIT failed` line names the cause. A batch whose
//! table is unchanged writes nothing.

use df_table::envelope::{EngineInput, InputOrigin, PlayerInput, ProposalOrigin};
use df_table::event::{FailureCause, TableEvent};
use df_table::identity::{Correlation, TableScope};
use df_table::outcome::{DeclineCode, OutcomeKind};
use df_table::owner_queue::{ServeError, ServedInput};
use df_table::pipeline::{PipelineTable, TurnResult};
use df_table::private_data::LengthClass;

use crate::engine::audit::EngineAudit;
use crate::engine::labels::sender_label;
use crate::engine::limits::MAXIMUM_RECENT_TURNS;
use crate::engine::projection_adapter::sender_view;
use crate::engine::report::{CommitDisposition, EngineError, ServedReply, TurnReport};
use crate::engine::request::QueueControl;
use crate::engine::turn::{TurnContext, machine_state, run_turn};

use super::Owner;

/// What a turn that could not be saved reads as to the seat (H.4).
const NOT_SAVED_READOUT: &str = "Could not save that. Try again.";

struct PendingTurn {
    served: ServedInput,
    result: TurnResult,
    machine_before: (String, String),
    machine_after: (String, String),
    /// What the sending seat is shown after this input, held until the batch is saved.
    view: Option<serde_json::Value>,
}

/// Why a batch stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BatchEnd {
    /// Nothing was waiting that could be served.
    Empty,
    Committed,
    /// The commit failed and the owner is backing off.
    Failed,
    /// The owner is backing off from an earlier failure and started no batch.
    BackingOff,
}

impl Owner {
    /// Serves batches until nothing is waiting or a commit fails.
    pub(super) fn drain(&mut self) -> Vec<TurnReport> {
        let mut all = Vec::new();
        loop {
            let (turns, end) = self.run_batch(usize::MAX);
            all.extend(turns);
            if end != BatchEnd::Committed {
                break;
            }
        }
        all
    }

    pub(super) fn control_queue(
        &mut self,
        control: QueueControl,
    ) -> Result<ServedReply, EngineError> {
        let (verb, turns) = match control {
            QueueControl::Hold => {
                self.held = true;
                ("hold", Vec::new())
            }
            QueueControl::Release => {
                self.held = false;
                ("release", self.drain())
            }
            QueueControl::Step { count } => {
                if count == 0 {
                    return Err(EngineError::BadRequest {
                        reason: "a step serves at least one input".to_owned(),
                    });
                }
                ("step", self.run_batch(count).0)
            }
        };
        self.audit(EngineAudit::Queue {
            verb,
            served: turns.len(),
        });
        Ok(ServedReply { turns })
    }

    /// Serves one batch of at most `limit` inputs and commits it.
    pub(super) fn run_batch(&mut self, limit: usize) -> (Vec<TurnReport>, BatchEnd) {
        let started_ms = self.now_ms();
        if self.backoff_remaining_ms(started_ms).is_some() {
            return (Vec::new(), BatchEnd::BackingOff);
        }
        let limit = limit.min(self.config.policy.maximum_inputs());
        let mut working = self.committed.checkpoint();
        let mut pending: Vec<PendingTurn> = Vec::new();
        let mut hand_off_asked = false;
        while pending.len() < limit {
            if let Some(slice_ms) = self.config.policy.time_slice_ms()
                && !pending.is_empty()
                && self.now_ms().saturating_sub(started_ms) >= slice_ms
            {
                break;
            }
            let served = match self.queue.serve_next(self.now_ms()) {
                Ok(Some(served)) => served,
                Ok(None) => break,
                Err(ServeError::TurnSequenceExhausted) => {
                    self.audit(EngineAudit::Stalled {
                        cause: "turn_sequence_exhausted",
                    });
                    break;
                }
            };
            let ran = {
                let ctx = self.turn_context();
                let before = machine_state(&ctx, &working);
                run_turn(&ctx, &mut working, &served).map(|result| {
                    let after = machine_state(&ctx, &working);
                    (result, before, after)
                })
            };
            let Ok((result, machine_before, machine_after)) = ran else {
                self.stop_unbuildable(&served);
                break;
            };
            if is_hand_off(&served) && result.record.outcome == OutcomeKind::Applied {
                hand_off_asked = false;
            }
            if result.hand_off_wanted
                && !hand_off_asked
                && let Some(machine) = working.running_machine()
            {
                hand_off_asked = self.queue_hand_off(&machine);
            }
            let view = served
                .envelope
                .seat()
                .and_then(|seat| sender_view(&result.projections, seat))
                .and_then(|view| self.view_or_event(view));
            pending.push(PendingTurn {
                served,
                result,
                machine_before,
                machine_after,
                view,
            });
        }
        if pending.is_empty() {
            return (Vec::new(), BatchEnd::Empty);
        }
        self.finish_batch(working, pending)
    }

    /// A view that could not be written is recorded once, as the input's own failure, and the
    /// report goes without it.
    fn view_or_event(
        &self,
        view: Result<serde_json::Value, crate::engine::projection_adapter::ViewError>,
    ) -> Option<serde_json::Value> {
        match view {
            Ok(view) => Some(view),
            Err(_) => {
                self.audit(EngineAudit::Stalled {
                    cause: "view_unwritable",
                });
                None
            }
        }
    }

    /// The pipeline could not be built, which `Owner::new` proves it can: the input is released
    /// so its id may be sent again, and the owner says it stopped serving.
    fn stop_unbuildable(&mut self, served: &ServedInput) {
        self.audit(EngineAudit::Stalled {
            cause: "pipeline_unbuildable",
        });
        let now_ms = self.now_ms();
        // The input was served a moment ago and nothing else runs, so releasing it cannot fail.
        let _ = self.queue.release(
            served.envelope.sender(),
            served.envelope.operation_id(),
            FailureCause::Unavailable,
            now_ms,
        );
    }

    fn finish_batch(
        &mut self,
        working: PipelineTable,
        pending: Vec<PendingTurn>,
    ) -> (Vec<TurnReport>, BatchEnd) {
        let inputs = pending.len();
        if working.state_hash() == self.committed.state_hash() {
            self.committed = working;
            self.audit(EngineAudit::NothingToCommit { inputs });
            let reports = self.release(pending, &CommitDisposition::NothingToCommit);
            return (reports, BatchEnd::Committed);
        }
        let staged: usize = pending
            .iter()
            .map(|turn| turn.result.committed_events.len())
            .sum();
        let last_turn = pending
            .last()
            .map_or(self.queue.last_turn_seq(), |turn| turn.served.turn_seq);
        match self.commit_world(working, inputs, staged, last_turn) {
            Ok(saved) => {
                let disposition = CommitDisposition::Committed {
                    id: saved.commit.get(),
                };
                let reports = self.release(pending, &disposition);
                self.issue_committed_requests(saved.correlation);
                (reports, BatchEnd::Committed)
            }
            Err(error) => (
                self.fail_batch(pending, error.key().to_owned()),
                BatchEnd::Failed,
            ),
        }
    }

    /// Sends to the broker the resource requests the commit recorded. A refused batch leaves no
    /// request at the broker, which is why this waits for the commit.
    fn issue_committed_requests(&mut self, correlation: Correlation) {
        let ctx = TurnContext {
            registry: &self.registry,
            catalog: &self.catalog,
            settings: &self.config.settings,
            limits: &self.config.limits,
            adapters: &self.adapters,
            sink: &*self.sink,
        };
        let committed = &mut self.committed;
        // A pipeline that cannot be built was reported when the batch ran.
        let _ = ctx.with_pipeline(|pipeline| pipeline.issue_requests(committed, correlation));
    }

    /// Ends every input of a batch that was saved: its staged events reach the log, the queue
    /// frees its seat, a narration is asked for, a result of the broker that no machine takes is
    /// dropped, and the report carries the commit and the view.
    fn release(
        &mut self,
        pending: Vec<PendingTurn>,
        disposition: &CommitDisposition,
    ) -> Vec<TurnReport> {
        let mut reports = Vec::with_capacity(pending.len());
        for turn in pending {
            turn.result.release_events(&*self.sink);
            self.end_in_queue(&turn.served, turn.result.record.outcome);
            self.take_unconsumed_result(&turn.served);
            let narration = self.narrate(&turn);
            let mut report = turn_report(&turn, disposition, self.revision(), narration);
            report.view = turn.view;
            reports.push(report);
        }
        self.remember_turns(&reports);
        reports
    }

    /// A resource result no machine consumes yet is taken out of the broker, so it is not kept for
    /// nobody (the broker's own line says how the request ended).
    fn take_unconsumed_result(&self, served: &ServedInput) {
        if let InputOrigin::Engine(EngineInput::ResourceResult { handle, .. }) =
            served.envelope.origin()
        {
            // A request that failed has no result to take.
            let _ = self.adapters.broker.hub.take_result(*handle);
        }
    }

    /// NARRATE: after the commit, for a turn that asked for it. The narrator answers and the
    /// pipeline qualifies the answer against the clock; it never changes game state.
    fn narrate(&mut self, turn: &PendingTurn) -> Option<LengthClass> {
        let job = turn.result.narration_job.as_ref()?;
        let shown_at_ms = self.now_ms();
        let answer = self
            .adapters
            .narrator
            .narrate(job.correlation, &job.committed);
        let ctx = TurnContext {
            registry: &self.registry,
            catalog: &self.catalog,
            settings: &self.config.settings,
            limits: &self.config.limits,
            adapters: &self.adapters,
            sink: &*self.sink,
        };
        let committed = &mut self.committed;
        let attached = ctx
            .with_pipeline(|pipeline| {
                pipeline.attach_narration(committed, job, shown_at_ms, answer)
            })
            .ok()
            .flatten();
        attached.map(|narration| LengthClass::of_length(narration.expose().chars().count()))
    }

    /// Fails a whole batch whose commit did not happen (`commit_world` has counted it, audited it
    /// and started the backoff): its staged events, views and table are dropped, each input ends
    /// refused `table_busy`, and its operation id is released, so the same id sent again is a new
    /// operation (H.4).
    fn fail_batch(&mut self, pending: Vec<PendingTurn>, cause: String) -> Vec<TurnReport> {
        let disposition = CommitDisposition::Failed { cause };
        let now_ms = self.now_ms();
        let mut reports = Vec::with_capacity(pending.len());
        for turn in pending {
            let (sender, operation) = (
                turn.served.envelope.sender(),
                turn.served.envelope.operation_id(),
            );
            // The owner served this input a moment ago and nothing else runs in between, so the
            // queue still knows it and releasing it cannot be refused.
            let _ = self
                .queue
                .release(sender, operation, FailureCause::Unavailable, now_ms);
            self.sink.record(&TableEvent::InputDeclined {
                scope: TableScope::of_table(self.config.table),
                correlation: turn.served.correlation(),
                code: DeclineCode::TableBusy,
            });
            let mut report = turn_report(&turn, &disposition, self.revision(), None);
            report.outcome = OutcomeKind::Declined;
            report.decline = Some(DeclineCode::TableBusy);
            report.readout = NOT_SAVED_READOUT.to_owned();
            report.machine_after = report.machine_before.clone();
            report.state_after = report.state_before.clone();
            report.effects = 0;
            report.pending = None;
            reports.push(report);
        }
        self.remember_turns(&reports);
        reports
    }

    fn end_in_queue(&mut self, served: &ServedInput, outcome: OutcomeKind) {
        let sender = served.envelope.sender();
        // The owner served this input a moment ago and nothing else runs in between, so the queue
        // still knows it and ending it cannot be refused.
        let _ = self
            .queue
            .finish(sender, served.envelope.operation_id(), outcome);
    }

    /// Keeps the report for `queue show`, without its view: the list is bounded by count, not by
    /// size, and a view is only what the answer to the sender carries.
    fn remember_turns(&mut self, reports: &[TurnReport]) {
        self.recent_turns
            .extend(reports.iter().map(|report| TurnReport {
                view: None,
                ..report.clone()
            }));
        while self.recent_turns.len() > MAXIMUM_RECENT_TURNS {
            self.recent_turns.pop_front();
        }
    }
}

/// Whether the input is the engine proposal a machine's hand-off is made by.
fn is_hand_off(served: &ServedInput) -> bool {
    matches!(
        served.envelope.origin(),
        InputOrigin::Engine(EngineInput::Proposal {
            origin: ProposalOrigin::Machine,
            ..
        })
    )
}

/// The action a seat's input named: the one it tapped, or the one a line was read as when the
/// pipeline asked for its narration.
fn action_of(served: &ServedInput, result: &TurnResult) -> Option<String> {
    match served.envelope.origin() {
        InputOrigin::Player {
            input: PlayerInput::Tap { action },
            ..
        } => Some(action.as_str().to_owned()),
        _ => result
            .narration_job
            .as_ref()
            .map(|job| job.committed.action.as_str().to_owned()),
    }
}

fn turn_report(
    turn: &PendingTurn,
    disposition: &CommitDisposition,
    revision: u64,
    narration: Option<LengthClass>,
) -> TurnReport {
    let (served, result) = (&turn.served, &turn.result);
    let (machine_before, state_before) = turn.machine_before.clone();
    let (machine_after, state_after) = turn.machine_after.clone();
    TurnReport {
        turn_seq: served.turn_seq.get(),
        operation: served.envelope.operation_id().to_string(),
        sender: sender_label(served.envelope.sender()),
        kind: served.envelope.kind().key().to_owned(),
        lane: served.lane,
        waited_ms: served.waited_ms,
        outcome: result.record.outcome,
        decline: result.record.declined,
        readout: result.readout.as_str().to_owned(),
        action: action_of(served, result),
        replayed: result.record.replayed,
        pending: result.pending.map(|pending| pending.get()),
        machine_before,
        state_before,
        machine_after,
        state_after,
        effects: result.record.effects.len(),
        commit: disposition.clone(),
        revision: Some(revision),
        narration,
        view: None,
    }
}
