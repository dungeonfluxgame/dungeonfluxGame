//! The plate scheduler: a clock that asks for a picture every `plate_interval_s` and never waits
//! for one (D16, RES5, RES9, D.1 to D.3, K.3, H.7).
//!
//! `plate_tick` is called by the owner when its wall clock says a tick is due. It snapshots the
//! table into a `PlateJob` that keeps its tick time, puts the job on a queue and returns at once;
//! the host's scene builder takes jobs from the queue in order and makes them through the broker.
//! A tick is skipped, and logged as skipped, when the table is not `Active` (D.3), when the scene
//! has no viewer (D.1), or when nothing a picture depends on has changed since the last job
//! (RES9, K.3: an overlay-level change such as the speaker or a hit point needs no new plate, and a
//! quiet table pays for none). A missed beat is not made up: after a stall one tick happens and the
//! next is due on the following beat, so a late clock never fires a burst.
//!
//! A job that failed or was dropped no longer counts as the last picture, so the next tick asks
//! again even though nothing changed. A scene change (a new generation) discards the queue, since
//! no plate of the old generation may reach the screen (D.2). The queue holds at most four jobs,
//! the plate bound of H.7: past it the oldest is dropped and counted (`plate.dropped_stale`).
//!
//! It never reads a clock, makes a request or knows how a plate is built.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::identity::SceneId;
use crate::listed::listed_enum;
use crate::plate_job::{JobCause, JobError, PlateJob, PlateJobId};
use crate::plate_timing::PlateTiming;
use crate::scene_plan_brief::SceneBrief;
use crate::table_state::{Presence, SceneGeneration, TableSnapshot, TableStatus};

/// Most plate jobs waiting to be built: the plate queue bound of J.4 and H.7.
pub const MAXIMUM_QUEUED_JOBS: usize = 4;

listed_enum! {
    /// Why a tick made no job.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum SkipReason {
        /// The table is idle or paused (D.3).
        TableNotActive,
        /// No seat is connected in the scene and the shared screen shows another (D.1).
        NoViewer,
        SceneClosed,
        /// Nothing a picture depends on changed since the last job (RES9, K.3).
        NothingChanged,
    }
}

impl SkipReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::TableNotActive => "table_not_active",
            Self::NoViewer => "no_viewer",
            Self::SceneClosed => "scene_closed",
            Self::NothingChanged => "nothing_changed",
        }
    }
}

/// What a tick did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// The next tick is not due yet.
    NotDue {
        due_in_ms: u64,
    },
    Skipped {
        reason: SkipReason,
    },
    /// A job was queued. `dropped` is the oldest queued job it pushed out, when the queue was full.
    Queued {
        job: PlateJobId,
        cause: JobCause,
        dropped: Option<PlateJobId>,
    },
}

/// A tick's outcome and the jobs a scene change threw away before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    pub outcome: TickOutcome,
    /// Queued jobs of an earlier generation, discarded because the scene moved on (D.2).
    pub discarded: Vec<PlateJobId>,
}

/// How a job ended, which decides whether its picture still counts as the last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobEnd {
    Ready,
    Failed,
    /// The broker's plate queue dropped it before it ran.
    Dropped,
    /// Its scene moved on.
    Stale,
}

/// Counts of everything the scheduler did, for `plates`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerStats {
    pub ticks: u64,
    pub queued: u64,
    pub skipped_table_not_active: u64,
    pub skipped_no_viewer: u64,
    pub skipped_scene_closed: u64,
    pub skipped_nothing_changed: u64,
    pub dropped_stale: u64,
    pub discarded_for_generation: u64,
    pub missed_beats: u64,
}

/// A queued job as `plates` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedJobReport {
    pub job: u64,
    pub tick_ms: u64,
    pub cause: String,
}

/// The scheduler's state for the dev view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerReport {
    pub scene: u64,
    pub generation: Option<u64>,
    pub interval_s: u32,
    pub plates_per_hour_at_most: u32,
    pub next_tick_ms: Option<u64>,
    pub queued: Vec<QueuedJobReport>,
    pub stats: SchedulerStats,
}

/// The clock of one scene's plates.
#[derive(Debug)]
pub struct PlateScheduler {
    scene: SceneId,
    timing: PlateTiming,
    next_job: u64,
    due_ms: Option<u64>,
    generation: Option<SceneGeneration>,
    queue: VecDeque<PlateJob>,
    last_picture: Option<(PlateJobId, u64)>,
    stats: SchedulerStats,
}

impl PlateScheduler {
    pub fn new(scene: SceneId, timing: PlateTiming) -> Self {
        Self {
            scene,
            timing,
            next_job: 0,
            due_ms: None,
            generation: None,
            queue: VecDeque::new(),
            last_picture: None,
            stats: SchedulerStats::default(),
        }
    }

    pub fn scene(&self) -> SceneId {
        self.scene
    }

    pub fn stats(&self) -> SchedulerStats {
        self.stats
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    pub fn next_tick_ms(&self) -> Option<u64> {
        self.due_ms
    }

    pub fn timing(&self) -> PlateTiming {
        self.timing
    }

    /// Changes the timings; the next tick keeps its time.
    pub fn set_timing(&mut self, timing: PlateTiming) {
        self.timing = timing;
    }

    fn next_job_id(&mut self) -> PlateJobId {
        self.next_job += 1;
        PlateJobId::new(self.next_job)
    }

    /// Forgets queued jobs of an earlier generation and the picture they would have followed.
    fn observe_generation(&mut self, generation: SceneGeneration) -> Vec<PlateJobId> {
        if self.generation == Some(generation) {
            return Vec::new();
        }
        self.generation = Some(generation);
        self.last_picture = None;
        let discarded: Vec<PlateJobId> = self.queue.drain(..).map(|job| job.id()).collect();
        self.stats.discarded_for_generation += u64::try_from(discarded.len()).unwrap_or(u64::MAX);
        discarded
    }

    fn push(&mut self, job: PlateJob) -> (PlateJobId, Option<PlateJobId>) {
        let id = job.id();
        self.last_picture = Some((id, job.staging_digest()));
        let dropped = if self.queue.len() >= MAXIMUM_QUEUED_JOBS {
            self.queue.pop_front().map(|oldest| oldest.id())
        } else {
            None
        };
        if dropped.is_some() {
            self.stats.dropped_stale += 1;
        }
        self.queue.push_back(job);
        self.stats.queued += 1;
        (id, dropped)
    }

    fn check_scene(&self, brief: &SceneBrief) -> Result<(), JobError> {
        if brief.scene() == self.scene {
            Ok(())
        } else {
            Err(JobError::BriefForOtherScene {
                asked: self.scene,
                brief: brief.scene(),
            })
        }
    }

    /// The scene was just entered: queues the first plate, which is built with the scene (K.2),
    /// and starts the clock one interval from now.
    pub fn enter_scene(
        &mut self,
        now_ms: u64,
        snapshot: &TableSnapshot,
        brief: &SceneBrief,
    ) -> Result<Tick, JobError> {
        self.check_scene(brief)?;
        let record = snapshot
            .scene(self.scene)
            .ok_or(JobError::SceneNotOpen { scene: self.scene })?;
        let discarded = self.observe_generation(record.generation());
        let id = self.next_job_id();
        let job = PlateJob::capture(id, JobCause::SceneEntered, now_ms, snapshot, brief)?;
        let (job, dropped) = self.push(job);
        self.due_ms = Some(now_ms.saturating_add(self.timing.plate_interval_ms()));
        Ok(Tick {
            outcome: TickOutcome::Queued {
                job,
                cause: JobCause::SceneEntered,
                dropped,
            },
            discarded,
        })
    }

    /// The owner's wall clock calls this. It makes one job when a tick is due and something
    /// visible changed, and returns at once either way; it never waits for a picture.
    pub fn plate_tick(
        &mut self,
        now_ms: u64,
        snapshot: &TableSnapshot,
        brief: &SceneBrief,
    ) -> Result<Tick, JobError> {
        self.check_scene(brief)?;
        let Some(record) = snapshot.scene(self.scene) else {
            let discarded: Vec<PlateJobId> = self.queue.drain(..).map(|job| job.id()).collect();
            return Ok(self.skipped(SkipReason::SceneClosed, discarded));
        };
        let discarded = self.observe_generation(record.generation());
        if let Some(due) = self.due_ms
            && now_ms < due
        {
            return Ok(Tick {
                outcome: TickOutcome::NotDue {
                    due_in_ms: due - now_ms,
                },
                discarded,
            });
        }
        self.advance_beat(now_ms);
        self.stats.ticks += 1;
        if snapshot.status() != TableStatus::Active {
            return Ok(self.skipped(SkipReason::TableNotActive, discarded));
        }
        if !has_viewer(snapshot, self.scene) {
            return Ok(self.skipped(SkipReason::NoViewer, discarded));
        }
        let id = PlateJobId::new(self.next_job + 1);
        let job = PlateJob::capture(id, JobCause::Tick, now_ms, snapshot, brief)?;
        if self
            .last_picture
            .is_some_and(|(_, digest)| digest == job.staging_digest())
        {
            return Ok(self.skipped(SkipReason::NothingChanged, discarded));
        }
        self.next_job += 1;
        let (job, dropped) = self.push(job);
        Ok(Tick {
            outcome: TickOutcome::Queued {
                job,
                cause: JobCause::Tick,
                dropped,
            },
            discarded,
        })
    }

    /// Moves the due time to the first beat after `now_ms`, counting the beats that passed unused.
    fn advance_beat(&mut self, now_ms: u64) {
        let interval = self.timing.plate_interval_ms();
        let next = match self.due_ms {
            Some(due) => {
                let behind = now_ms.saturating_sub(due);
                let missed = behind / interval;
                self.stats.missed_beats += missed;
                due.saturating_add(interval.saturating_mul(missed.saturating_add(1)))
            }
            None => now_ms.saturating_add(interval),
        };
        self.due_ms = Some(next);
    }

    fn skipped(&mut self, reason: SkipReason, discarded: Vec<PlateJobId>) -> Tick {
        match reason {
            SkipReason::TableNotActive => self.stats.skipped_table_not_active += 1,
            SkipReason::NoViewer => self.stats.skipped_no_viewer += 1,
            SkipReason::SceneClosed => self.stats.skipped_scene_closed += 1,
            SkipReason::NothingChanged => self.stats.skipped_nothing_changed += 1,
        }
        Tick {
            outcome: TickOutcome::Skipped { reason },
            discarded,
        }
    }

    /// The table resumed: one fresh plate is requested (D.3), so the next tick is due at once and
    /// the last picture no longer counts as current.
    pub fn resumed(&mut self) {
        self.last_picture = None;
        self.due_ms = None;
    }

    /// The next job to build, oldest first.
    pub fn take_next(&mut self) -> Option<PlateJob> {
        self.queue.pop_front()
    }

    /// A job ended. One that did not produce a picture no longer stands for the last one.
    pub fn job_ended(&mut self, job: PlateJobId, end: JobEnd) {
        if end == JobEnd::Ready {
            return;
        }
        if self.last_picture.is_some_and(|(last, _)| last == job) {
            self.last_picture = None;
        }
    }

    pub fn report(&self) -> SchedulerReport {
        SchedulerReport {
            scene: self.scene.get(),
            generation: self.generation.map(SceneGeneration::get),
            interval_s: self.timing.plate_interval_s(),
            plates_per_hour_at_most: self.timing.plates_per_hour_at_most(),
            next_tick_ms: self.due_ms,
            queued: self
                .queue
                .iter()
                .map(|job| QueuedJobReport {
                    job: job.id().get(),
                    tick_ms: job.tick_ms(),
                    cause: job.cause().key().to_owned(),
                })
                .collect(),
            stats: self.stats,
        }
    }
}

/// Whether anyone is looking at `scene` (D.1): the shared screen shows it, or a connected seat is
/// in it.
fn has_viewer(snapshot: &TableSnapshot, scene: SceneId) -> bool {
    snapshot.focus_scene() == Some(scene)
        || snapshot.seats().iter().any(|seat| {
            snapshot.seat(*seat).is_some_and(|state| {
                state.scene() == Some(scene) && state.presence() == Presence::Connected
            })
        })
}
