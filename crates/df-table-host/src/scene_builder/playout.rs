//! The playout buffer with its audit: the host's side of when a plate reaches the screen (K.3).
//!
//! The pure `PlayoutBuffer` decides and returns typed answers; this wrapper records each of them
//! as one `PLATE` event, once: a delivery or a refusal (`plate.rejected_epoch`), a swap with the
//! numbers of RES41 (`plate.swapped{from, to, prior_on_screen_ms}`), a ready plate waiting for a
//! boundary, the wait passing `plate_swap_wait_s`, the next slot going late (once for each slot)
//! and the "picture catching up" mark starting and ending (RES10). Polling the buffer every few
//! milliseconds therefore never repeats a line. It makes no decision of its own and reads no clock.

use std::sync::Arc;

use df_table::identity::SceneId;
use df_table::plate::buffer::{
    Delivery, GenerationRegressionError, PlayoutBuffer, StageMoment, Step, StepAction,
};
use df_table::plate::job::{PlateJob, PlateJobId};
use df_table::plate::timing::PlateTiming;
use df_table::scene_plan::ReadyPlate;
use df_table::table_state::SceneGeneration;

use super::events::{SceneBuilderAudit, SceneBuilderEvent};

/// The playout buffer of the scene the shared screen shows, and what it has already said.
pub struct Playout {
    buffer: PlayoutBuffer,
    timing: PlateTiming,
    audit: Arc<dyn SceneBuilderAudit>,
    late_reported_for_slot: Option<u64>,
    catching_up_reported: bool,
    waiting_reported: Option<(PlateJobId, &'static str)>,
}

impl Playout {
    pub fn new(
        scene: SceneId,
        generation: SceneGeneration,
        timing: PlateTiming,
        audit: Arc<dyn SceneBuilderAudit>,
    ) -> Self {
        Self {
            buffer: PlayoutBuffer::new(scene, generation, timing),
            timing,
            audit,
            late_reported_for_slot: None,
            catching_up_reported: false,
            waiting_reported: None,
        }
    }

    pub fn buffer(&self) -> &PlayoutBuffer {
        &self.buffer
    }

    /// The timings the buffer was made with, which it swaps by for as long as it lives:
    /// `df-table`'s buffer takes timings only when made.
    pub fn timing(&self) -> PlateTiming {
        self.timing
    }

    fn scene(&self) -> SceneId {
        self.buffer.scene()
    }

    /// Moves the screen to a scene: everything held was for the old one and is thrown away.
    pub fn focus_changed(
        &mut self,
        scene: SceneId,
        generation: SceneGeneration,
    ) -> Result<(), GenerationRegressionError> {
        self.buffer.focus_changed(scene, generation)?;
        self.late_reported_for_slot = None;
        self.catching_up_reported = false;
        self.waiting_reported = None;
        Ok(())
    }

    /// Notes that a job was asked for. A job of another scene or generation is refused and
    /// recorded.
    pub fn expect(&mut self, job: &PlateJob) {
        if let Err(reason) = self.buffer.expect(job) {
            self.audit.record(&SceneBuilderEvent::Rejected {
                scene: job.scene(),
                generation: job.stamp().generation,
                job: job.id(),
                reason,
            });
        }
    }

    /// Takes a finished plate, or refuses it, and records which.
    pub fn deliver(&mut self, plate: ReadyPlate) -> Delivery {
        let scene = plate.job.scene();
        let generation = plate.job.stamp().generation;
        let job = plate.job.id();
        let delivery = self.buffer.deliver(plate);
        self.audit.record(&match delivery {
            Delivery::Accepted => SceneBuilderEvent::Delivered {
                scene,
                generation,
                job,
            },
            Delivery::Rejected(reason) => SceneBuilderEvent::Rejected {
                scene,
                generation,
                job,
                reason,
            },
        });
        delivery
    }

    pub fn forget(&mut self, job: PlateJobId) {
        self.buffer.forget(job);
    }

    /// One look at the buffer: swaps a plate in when the rhythm, the reading time and the moment
    /// allow, and records what changed since the last look.
    pub fn step(&mut self, now_ms: u64, moment: StageMoment, minimum_on_screen_ms: u64) -> Step {
        let waiting_job = self.buffer.next_ready_job();
        let step = self.buffer.step(now_ms, moment, minimum_on_screen_ms);
        let scene = self.scene();
        match step.action {
            StepAction::Swapped(swap) => {
                self.audit
                    .record(&SceneBuilderEvent::Swapped { scene, swap });
                self.late_reported_for_slot = None;
                self.waiting_reported = None;
            }
            StepAction::Waiting { reason, waited_ms } => {
                if let Some(job) = waiting_job
                    && self.waiting_reported != Some((job, reason.key()))
                {
                    self.waiting_reported = Some((job, reason.key()));
                    self.audit.record(&SceneBuilderEvent::Waiting {
                        scene,
                        job: Some(job.get()),
                        reason,
                        waited_ms,
                    });
                }
            }
            StepAction::Quiet | StepAction::Holding => {}
        }
        if step.swap_overdue
            && let StepAction::Waiting { waited_ms, .. } = step.action
        {
            self.audit
                .record(&SceneBuilderEvent::SwapOverdue { scene, waited_ms });
        }
        self.report_lateness(&step, scene);
        step
    }

    fn report_lateness(&mut self, step: &Step, scene: SceneId) {
        let slot = self.buffer.next_slot_ms();
        if step.lateness.slot_late_ms > 0
            && self.late_reported_for_slot != slot
            && let Some(age) = step.lateness.oldest_outstanding_age_ms
        {
            self.late_reported_for_slot = slot;
            self.audit.record(&SceneBuilderEvent::Late {
                scene,
                slot_late_ms: step.lateness.slot_late_ms,
                oldest_outstanding_age_ms: age,
            });
        }
        if step.catching_up != self.catching_up_reported {
            self.catching_up_reported = step.catching_up;
            self.audit.record(&SceneBuilderEvent::CatchingUp {
                scene,
                on: step.catching_up,
                age_ms: step.lateness.oldest_outstanding_age_ms.unwrap_or(0),
            });
        }
    }
}
