//! The playout buffer: when a finished plate reaches the screen (D17, K.3, D.2, RES10).
//!
//! The clock requests plates and the buffer decides when to show them. It holds finished plates
//! and shows them in order at the configured rhythm: one swap at least `plate_interval_s` after the
//! last, so a pipeline that catches up never produces a burst. When the next plate is not ready at
//! its slot the buffer keeps the current one and reports how late things are, in two numbers that
//! are never merged: the slot's lateness, and the age of the oldest job still outstanding (the
//! tick time every job keeps). A quiet table has no outstanding job and so no lateness (RES9).
//!
//! A swap happens only between lines (K.3): the stage conductor tells the buffer what moment the
//! screen is at, and only a line boundary lets a plate in, never mid-line (REF-068), during a dice
//! or result card, or during a reveal beat. A plate also never replaces one shown for less than the
//! minimum on-screen time the caller gives (the reading time of the lines over it, RES41), and it
//! dissolves in over at least the dissolve minimum. A ready plate waiting longer than
//! `plate_swap_wait_s` for a boundary is reported overdue once, and is still never forced in.
//!
//! It accepts a plate only when its scene and generation are the buffer's (D.2): a plate of another
//! scene or an earlier generation is refused and counted as late, an older plate than the one
//! shown is refused, and a scene change empties the buffer, so a picture of an old scene is never
//! shown under a new one and the generation never goes backwards. It holds only plates that
//! finished, so a slot is never filled with a half-built picture.
//!
//! It reads no clock: the caller hands `now_ms` in.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::identity::SceneId;
use crate::plate_job::{PlateJob, PlateJobId};
use crate::plate_timing::{PLATE_DISSOLVE_MINIMUM_MS, PlateTiming};
use crate::scene_plan::{ReadyPlate, StoredPicture};
use crate::scene_plan_locate::LocateOutcome;
use crate::table_state::SceneGeneration;

/// What the stage conductor says the screen is doing, which decides whether a plate may swap
/// (K.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageMoment {
    /// Between two lines: the only moment a plate may dissolve in.
    LineBoundary,
    /// A line is on screen or being voiced.
    MidLine,
    DiceOrResultCard,
    RevealBeat,
}

impl StageMoment {
    pub fn key(self) -> &'static str {
        match self {
            Self::LineBoundary => "line_boundary",
            Self::MidLine => "mid_line",
            Self::DiceOrResultCard => "dice_or_result_card",
            Self::RevealBeat => "reveal_beat",
        }
    }

    pub fn allows_swap(self) -> bool {
        self == Self::LineBoundary
    }
}

/// Why a plate or a job was refused by the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    WrongScene,
    /// Its generation is not the scene's now: it is stored as late and never shown (D.2).
    StaleGeneration,
    /// A newer plate than this one is already on screen.
    OlderThanShown,
}

impl RejectReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::WrongScene => "wrong_scene",
            Self::StaleGeneration => "stale_generation",
            Self::OlderThanShown => "older_than_shown",
        }
    }
}

/// Why the buffer could not move to a scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationRegression {
    pub current: SceneGeneration,
    pub requested: SceneGeneration,
}

/// Whether a delivered plate was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Accepted,
    Rejected(RejectReason),
}

/// Why a ready plate has not swapped in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    /// The rhythm: the next slot is not due.
    SlotNotDue { due_in_ms: u64 },
    /// The plate on screen has not been up as long as its lines need to be read.
    MinimumOnScreen { remaining_ms: u64 },
    /// The screen is not between lines.
    Moment(StageMoment),
}

impl WaitReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::SlotNotDue { .. } => "slot_not_due",
            Self::MinimumOnScreen { .. } => "minimum_on_screen",
            Self::Moment(moment) => moment.key(),
        }
    }
}

/// A plate that went on screen: the numbers `plate.swapped` records (RES41).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swap {
    pub to: u64,
    pub from: Option<u64>,
    pub dissolve_ms: u64,
    /// How far past its slot the swap happened.
    pub slot_late_ms: u64,
    /// How old the plate's picture was, from its tick: how far the screen was behind the table.
    pub content_age_ms: u64,
    pub prior_on_screen_ms: Option<u64>,
}

/// What a step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepAction {
    /// No plate on screen and none expected: the establishing card keeps the screen.
    Quiet,
    /// The current plate stays: nothing is ready, and nothing is late or the slot is not due.
    Holding,
    /// A plate is ready and not in yet.
    Waiting {
        reason: WaitReason,
        waited_ms: u64,
    },
    Swapped(Swap),
}

/// How late things are. The two numbers answer different questions and are never added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lateness {
    /// How far past its slot the next plate is, while a job is outstanding; zero otherwise.
    pub slot_late_ms: u64,
    /// The age of the oldest job still outstanding, from its tick.
    pub oldest_outstanding_age_ms: Option<u64>,
}

/// The result of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub action: StepAction,
    pub lateness: Lateness,
    /// The "picture catching up" mark (RES10, K.3): the plate on screen is behind the table by
    /// more than `image_stale_limit_s` and disagrees with it. A quiet stale plate shows nothing.
    pub catching_up: bool,
    /// True once per waiting plate, when it passes `plate_swap_wait_s` without a boundary.
    pub swap_overdue: bool,
}

/// Counts for the dev view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferStats {
    pub accepted: u64,
    pub rejected_wrong_scene: u64,
    pub rejected_stale_generation: u64,
    pub rejected_older_than_shown: u64,
    pub swaps: u64,
    pub swaps_overdue: u64,
    pub forgotten_jobs: u64,
}

struct Pending {
    tick_ms: u64,
    digest: u64,
}

/// The plate on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownPlate {
    job: PlateJobId,
    tick_ms: u64,
    digest: u64,
    shown_ms: u64,
    plate: StoredPicture,
    locate: LocateOutcome,
}

impl ShownPlate {
    pub fn job(&self) -> PlateJobId {
        self.job
    }

    pub fn tick_ms(&self) -> u64 {
        self.tick_ms
    }

    pub fn shown_ms(&self) -> u64 {
        self.shown_ms
    }

    pub fn plate(&self) -> StoredPicture {
        self.plate
    }

    /// Where everything is in the picture on screen.
    pub fn locate(&self) -> &LocateOutcome {
        &self.locate
    }
}

/// The buffer of the scene the shared screen shows.
pub struct PlayoutBuffer {
    scene: SceneId,
    generation: SceneGeneration,
    timing: PlateTiming,
    pending: BTreeMap<PlateJobId, Pending>,
    ready: BTreeMap<PlateJobId, ReadyPlate>,
    current: Option<ShownPlate>,
    next_slot_ms: Option<u64>,
    waiting_since_ms: Option<u64>,
    overdue_reported: bool,
    stats: BufferStats,
}

impl PlayoutBuffer {
    pub fn new(scene: SceneId, generation: SceneGeneration, timing: PlateTiming) -> Self {
        Self {
            scene,
            generation,
            timing,
            pending: BTreeMap::new(),
            ready: BTreeMap::new(),
            current: None,
            next_slot_ms: None,
            waiting_since_ms: None,
            overdue_reported: false,
            stats: BufferStats::default(),
        }
    }

    pub fn scene(&self) -> SceneId {
        self.scene
    }

    pub fn generation(&self) -> SceneGeneration {
        self.generation
    }

    pub fn current(&self) -> Option<&ShownPlate> {
        self.current.as_ref()
    }

    pub fn stats(&self) -> BufferStats {
        self.stats
    }

    pub fn next_slot_ms(&self) -> Option<u64> {
        self.next_slot_ms
    }

    /// The oldest finished plate that has not been shown: the one the next swap would show.
    pub fn next_ready_job(&self) -> Option<PlateJobId> {
        self.ready.keys().next().copied()
    }

    /// Jobs asked for and not yet shown or given up.
    pub fn outstanding(&self) -> usize {
        self.pending.len() + self.ready.len()
    }

    /// The table moved the screen to another scene, or entered it again: everything held is for
    /// the old one, so it is thrown away and the screen shows no plate until one of the new scene
    /// is ready (K.2). The generation only goes up (D.2); a request to go back is refused.
    pub fn focus_changed(
        &mut self,
        scene: SceneId,
        generation: SceneGeneration,
    ) -> Result<(), GenerationRegression> {
        if generation <= self.generation {
            return Err(GenerationRegression {
                current: self.generation,
                requested: generation,
            });
        }
        self.scene = scene;
        self.generation = generation;
        self.pending.clear();
        self.ready.clear();
        self.current = None;
        self.next_slot_ms = None;
        self.waiting_since_ms = None;
        self.overdue_reported = false;
        Ok(())
    }

    /// Notes that a job was asked for, with the tick time that lateness is measured from. A job of
    /// another scene or generation is refused.
    pub fn expect(&mut self, job: &PlateJob) -> Result<(), RejectReason> {
        self.check(job.stamp().scene, job.stamp().generation)?;
        self.pending.insert(
            job.id(),
            Pending {
                tick_ms: job.tick_ms(),
                digest: job.staging_digest(),
            },
        );
        Ok(())
    }

    fn check(&self, scene: SceneId, generation: SceneGeneration) -> Result<(), RejectReason> {
        if scene != self.scene {
            return Err(RejectReason::WrongScene);
        }
        if generation != self.generation {
            return Err(RejectReason::StaleGeneration);
        }
        Ok(())
    }

    /// Takes a finished plate, or refuses it (D.2). A refused plate is never shown.
    pub fn deliver(&mut self, plate: ReadyPlate) -> Delivery {
        let stamp = plate.job.stamp();
        if let Err(reason) = self.check(stamp.scene, stamp.generation) {
            match reason {
                RejectReason::WrongScene => self.stats.rejected_wrong_scene += 1,
                _ => self.stats.rejected_stale_generation += 1,
            }
            return Delivery::Rejected(reason);
        }
        let id = plate.job.id();
        if self.current.as_ref().is_some_and(|shown| id <= shown.job) {
            self.pending.remove(&id);
            self.stats.rejected_older_than_shown += 1;
            return Delivery::Rejected(RejectReason::OlderThanShown);
        }
        self.pending.remove(&id);
        self.ready.insert(id, plate);
        self.stats.accepted += 1;
        Delivery::Accepted
    }

    /// A job gave up (failed, dropped or went stale): it is no longer expected, so it no longer
    /// makes the next slot late.
    pub fn forget(&mut self, job: PlateJobId) {
        if self.pending.remove(&job).is_some() {
            self.stats.forgotten_jobs += 1;
        }
    }

    /// The oldest tick time among jobs not yet shown.
    fn oldest_outstanding_tick_ms(&self) -> Option<u64> {
        let pending = self.pending.values().map(|pending| pending.tick_ms);
        let ready = self.ready.values().map(|plate| plate.job.tick_ms());
        pending.chain(ready).min()
    }

    /// The digest of the newest job not yet shown.
    fn newest_outstanding_digest(&self) -> Option<u64> {
        let pending = self
            .pending
            .iter()
            .next_back()
            .map(|(id, pending)| (*id, pending.digest));
        let ready = self
            .ready
            .iter()
            .next_back()
            .map(|(id, plate)| (*id, plate.job.staging_digest()));
        pending
            .into_iter()
            .chain(ready)
            .max_by_key(|(id, _)| *id)
            .map(|(_, digest)| digest)
    }

    fn lateness(&self, now_ms: u64) -> Lateness {
        let slot_late_ms = match (self.current.as_ref(), self.next_slot_ms) {
            (Some(_), Some(slot)) if self.outstanding() > 0 => now_ms.saturating_sub(slot),
            _ => 0,
        };
        Lateness {
            slot_late_ms,
            oldest_outstanding_age_ms: self
                .oldest_outstanding_tick_ms()
                .map(|tick| now_ms.saturating_sub(tick)),
        }
    }

    fn is_catching_up(&self, lateness: Lateness) -> bool {
        let (Some(current), Some(newest), Some(age)) = (
            self.current.as_ref(),
            self.newest_outstanding_digest(),
            lateness.oldest_outstanding_age_ms,
        ) else {
            return false;
        };
        age >= self.timing.image_stale_limit_ms() && newest != current.digest
    }

    /// What blocks the oldest ready plate from swapping in now, if anything.
    fn blocker(
        &self,
        now_ms: u64,
        moment: StageMoment,
        minimum_on_screen_ms: u64,
    ) -> Option<WaitReason> {
        if let Some(current) = self.current.as_ref() {
            let slot = self.next_slot_ms.unwrap_or(now_ms);
            if let Some(due_in_ms) = slot.checked_sub(now_ms).filter(|remaining| *remaining > 0) {
                return Some(WaitReason::SlotNotDue { due_in_ms });
            }
            let on_screen = now_ms.saturating_sub(current.shown_ms);
            if let Some(remaining_ms) = minimum_on_screen_ms
                .checked_sub(on_screen)
                .filter(|remaining| *remaining > 0)
            {
                return Some(WaitReason::MinimumOnScreen { remaining_ms });
            }
        }
        (!moment.allows_swap()).then_some(WaitReason::Moment(moment))
    }

    /// One look at the buffer at `now_ms`: swap the oldest ready plate in if the rhythm, the
    /// reading time and the moment all allow it, else say why not and how late things are.
    /// `minimum_on_screen_ms` is the reading time of the lines over the plate on screen.
    pub fn step(&mut self, now_ms: u64, moment: StageMoment, minimum_on_screen_ms: u64) -> Step {
        let lateness = self.lateness(now_ms);
        let catching_up = self.is_catching_up(lateness);
        let Some(id) = self.ready.keys().next().copied() else {
            self.waiting_since_ms = None;
            let action = if self.current.is_none() && self.pending.is_empty() {
                StepAction::Quiet
            } else {
                StepAction::Holding
            };
            return Step {
                action,
                lateness,
                catching_up,
                swap_overdue: false,
            };
        };
        if let Some(reason) = self.blocker(now_ms, moment, minimum_on_screen_ms) {
            return self.wait(now_ms, reason, lateness, catching_up);
        }
        let Some(plate) = self.ready.remove(&id) else {
            return Step {
                action: StepAction::Holding,
                lateness,
                catching_up,
                swap_overdue: false,
            };
        };
        let swap = self.put_on_screen(now_ms, plate);
        let lateness = self.lateness(now_ms);
        Step {
            action: StepAction::Swapped(swap),
            lateness,
            catching_up: self.is_catching_up(lateness),
            swap_overdue: false,
        }
    }

    fn wait(
        &mut self,
        now_ms: u64,
        reason: WaitReason,
        lateness: Lateness,
        catching_up: bool,
    ) -> Step {
        let wants_in = !matches!(
            reason,
            WaitReason::SlotNotDue { .. } | WaitReason::MinimumOnScreen { .. }
        );
        let waited_ms = if wants_in {
            let since = *self.waiting_since_ms.get_or_insert(now_ms);
            now_ms.saturating_sub(since)
        } else {
            self.waiting_since_ms = None;
            0
        };
        let overdue = wants_in && waited_ms >= self.timing.swap_wait_ms() && !self.overdue_reported;
        if overdue {
            self.overdue_reported = true;
            self.stats.swaps_overdue += 1;
        }
        Step {
            action: StepAction::Waiting { reason, waited_ms },
            lateness,
            catching_up,
            swap_overdue: overdue,
        }
    }

    fn put_on_screen(&mut self, now_ms: u64, plate: ReadyPlate) -> Swap {
        let slot_late_ms = match (self.current.as_ref(), self.next_slot_ms) {
            (Some(_), Some(slot)) => now_ms.saturating_sub(slot),
            _ => 0,
        };
        let swap = Swap {
            to: plate.job.id().get(),
            from: self.current.as_ref().map(|shown| shown.job.get()),
            dissolve_ms: PLATE_DISSOLVE_MINIMUM_MS,
            slot_late_ms,
            content_age_ms: now_ms.saturating_sub(plate.job.tick_ms()),
            prior_on_screen_ms: self
                .current
                .as_ref()
                .map(|shown| now_ms.saturating_sub(shown.shown_ms)),
        };
        self.current = Some(ShownPlate {
            job: plate.job.id(),
            tick_ms: plate.job.tick_ms(),
            digest: plate.job.staging_digest(),
            shown_ms: now_ms,
            plate: plate.plate,
            locate: plate.locate,
        });
        self.next_slot_ms = Some(now_ms.saturating_add(self.timing.plate_interval_ms()));
        self.waiting_since_ms = None;
        self.overdue_reported = false;
        self.stats.swaps += 1;
        swap
    }

    pub fn report(&self, now_ms: u64) -> BufferReport {
        let lateness = self.lateness(now_ms);
        BufferReport {
            scene: self.scene.get(),
            generation: self.generation.get(),
            current: self.current.as_ref().map(|shown| ShownReport {
                job: shown.job.get(),
                tick_ms: shown.tick_ms,
                shown_ms: shown.shown_ms,
                on_screen_ms: now_ms.saturating_sub(shown.shown_ms),
                located_figures: shown.locate.located_figures(),
                unlocated_figures: shown.locate.unlocated_figures(),
                located_items: shown.locate.located_items(),
                own_picture_items: shown.locate.own_picture_items(),
                unlocated_items: shown.locate.unlocated_items(),
            }),
            ready: self.ready.keys().map(|job| job.get()).collect(),
            pending: self
                .pending
                .iter()
                .map(|(job, pending)| PendingReport {
                    job: job.get(),
                    tick_ms: pending.tick_ms,
                    age_ms: now_ms.saturating_sub(pending.tick_ms),
                })
                .collect(),
            next_slot_ms: self.next_slot_ms,
            lateness,
            catching_up: self.is_catching_up(lateness),
            stats: self.stats,
        }
    }
}

/// The plate on screen, for the dev view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShownReport {
    pub job: u64,
    pub tick_ms: u64,
    pub shown_ms: u64,
    pub on_screen_ms: u64,
    pub located_figures: usize,
    pub unlocated_figures: usize,
    pub located_items: usize,
    pub own_picture_items: usize,
    pub unlocated_items: usize,
}

/// A job the buffer is still waiting for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingReport {
    pub job: u64,
    pub tick_ms: u64,
    pub age_ms: u64,
}

/// The buffer's state for the dev view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferReport {
    pub scene: u64,
    pub generation: u64,
    pub current: Option<ShownReport>,
    pub ready: Vec<u64>,
    pub pending: Vec<PendingReport>,
    pub next_slot_ms: Option<u64>,
    pub lateness: Lateness,
    pub catching_up: bool,
    pub stats: BufferStats,
}
