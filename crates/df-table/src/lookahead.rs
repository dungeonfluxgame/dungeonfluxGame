//! The look-ahead planner: which next scene to build, when to start, when to give up and what the
//! transition finds (SB-LA, idea 1 of "Ideas to come", O.16).
//!
//! The planner is the pure half of the look-ahead worker. The host's worker asks the broker for the
//! stages; this decides. It is fed three things, all with a `now_ms` the caller owns: the scenes the
//! story expects next (`set_expected`, from `lookahead_candidates`), a view of the current scene
//! (`plan`: when it became Ready, and whether its own plates or a live line need the lane) and the
//! results of builds it started (`build_finished`). It answers with `Action`s the host carries out:
//! start a build, cancel one, forget one that aged out. It never asks a provider for anything, never
//! reads a clock and holds no thread.
//!
//! Rules kept:
//! - *Certain first.* The next scene of an authored beat and the destination of a committed travel
//!   leg start at once. A scene that is only likely waits `lookahead_start_s` after the current
//!   scene is Ready, so it never competes with the current scene's own first plate. At most
//!   `lookahead_depth` wanted scenes are built or ready at a time (the default is the one certain
//!   scene); a built scene nobody wants stays in the book without using that budget, and a certain
//!   scene that finds the depth full of likely builds cancels the lowest-ranked one.
//! - *Never a gate.* It pauses starting anything while the current scene's plates or a live line
//!   need the lane, a build is cancelled at `lookahead_deadline_s`, and a build that is not done
//!   when the table enters the scene is dropped (`Take::Late`): the table builds the scene the
//!   normal way and plays on the establishing card meanwhile (K.2). It never touches the owner's
//!   queue: its only output is a list of actions for the lowest broker class.
//! - *Retarget.* When the story branches, `set_expected` cancels every running build whose scene is
//!   no longer expected or whose facts changed, and `enter` cancels every build but the one for the
//!   scene the table entered. A built scene nobody took stays in the book for reuse until
//!   `lookahead_book_age_s` ends, and a result for a build that was cancelled is refused by its
//!   token, so a late answer is never stored.
//! - *Stale at the transition.* `enter` compares the build's `WorldStamp` with the world now: a hit
//!   is handed over whole, a cast-only change keeps the picture and drops the description (the dress
//!   step runs again), and a change of place or part of the day discards the build.
//! - *Bounded.* The book and the list of expected scenes are held to large named maxima.

pub(crate) mod candidate;
pub mod settings;
pub mod stamp;

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::event::FailureCause;
use crate::identity::SceneId;
use crate::limits::MAXIMUM_BOOKED_SCENES;
use crate::listed::listed_enum;
use crate::lookahead::settings::LookaheadSettings;
use crate::lookahead::stamp::{StampChange, WorldStamp};
use crate::scene_plan::brief::SceneBrief;
use crate::scene_plan::{SceneStage, StageOutputs};

pub use crate::lookahead::candidate::{
    CERTAIN_PERMILLE, Candidate, Expectation, Expected, LookaheadError, lookahead_candidates,
};

/// Names one build the planner started, so a late result for a cancelled build is recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuildToken(u64);

impl BuildToken {
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for BuildToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

listed_enum! {
    /// Why the planner cancelled a build.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CancelReason {
        /// The story no longer expects the scene.
        Retargeted,
        /// The facts the build used changed while it ran.
        StampChanged,
        /// The build was still out at `lookahead_deadline_s`.
        Deadline,
        /// The table entered another scene.
        Branched,
        /// The table entered the scene before the build was done.
        Late,
        /// The table closed.
        Closed,
        /// The worker was turned off.
        Disabled,
    }
}

impl CancelReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::Retargeted => "retargeted",
            Self::StampChanged => "stamp_changed",
            Self::Deadline => "deadline",
            Self::Branched => "branched",
            Self::Late => "late",
            Self::Closed => "closed",
            Self::Disabled => "disabled",
        }
    }
}

/// What the host does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Build the scene through the broker on the look-ahead class.
    Start {
        token: BuildToken,
        brief: SceneBrief,
        expectation: Expectation,
        likelihood_permille: u16,
    },
    /// Cancel what the build has out and store nothing of it.
    Cancel {
        scene: SceneId,
        token: BuildToken,
        reason: CancelReason,
    },
    /// A built scene nobody took has aged out of the book.
    Expire { scene: SceneId },
}

/// What a build ended with, as the host reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildOutcome {
    /// The description and the background are stored.
    Ready(StageOutputs),
    Failed {
        stage: SceneStage,
        cause: FailureCause,
    },
}

/// A scene built ahead, taken out of the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookedScene {
    /// What the build stored: the description and the background.
    pub outputs: StageOutputs,
    pub ready_ms: u64,
    pub stamp: WorldStamp,
}

/// What the transition found (`lookahead.hit`, `lookahead.miss`, `lookahead.stale`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Take {
    /// The scene was Ready and its facts still hold.
    Hit(BookedScene),
    /// Only the cast changed: the background is kept and the description is for the dress step to
    /// write again.
    Redress(BookedScene),
    /// The place or the part of the day changed: the build was discarded.
    Stale { change: StampChange },
    /// The build was not done: it is cancelled, and the table builds the scene the normal way.
    Late,
    /// The build failed or timed out.
    Failed {
        stage: Option<SceneStage>,
        cause: FailureCause,
    },
    /// Nothing was built for the scene.
    Miss,
}

impl Take {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Hit(_) => "hit",
            Self::Redress(_) => "redress",
            Self::Stale { .. } => "stale",
            Self::Late => "late",
            Self::Failed { .. } => "failed",
            Self::Miss => "miss",
        }
    }
}

/// What entering a scene did to the look-ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entered {
    pub take: Take,
    /// The builds of other scenes that were cancelled (and the entered scene's, when it was late).
    pub actions: Vec<Action>,
}

/// What the current scene looks like to the planner at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlanView {
    /// When the current scene became Ready, if it has.
    pub current_ready_ms: Option<u64>,
    /// The current scene's plates or a live line need the lane: nothing new starts.
    pub foreground_busy: bool,
}

/// The counters behind the metrics of idea 1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookaheadCounts {
    pub started: u64,
    /// Builds that finished with a description and a background.
    pub built: u64,
    pub hit: u64,
    pub redressed: u64,
    pub stale: u64,
    pub late: u64,
    pub miss: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub expired: u64,
    /// Results refused because their build had been cancelled.
    pub refused_results: u64,
}

#[derive(Debug, Clone)]
enum Phase {
    Building {
        started_ms: u64,
    },
    Ready {
        ready_ms: u64,
        outputs: StageOutputs,
    },
    Failed {
        stage: Option<SceneStage>,
        cause: FailureCause,
    },
}

#[derive(Debug, Clone)]
struct Slot {
    token: BuildToken,
    stamp: WorldStamp,
    expectation: Expectation,
    phase: Phase,
    began_ms: u64,
}

impl Slot {
    fn is_built_or_building(&self) -> bool {
        matches!(self.phase, Phase::Building { .. } | Phase::Ready { .. })
    }
}

/// The look-ahead planner of one table.
#[derive(Debug, Clone)]
pub struct LookaheadPlanner {
    settings: LookaheadSettings,
    paused: bool,
    closed: bool,
    wanted: Vec<Candidate>,
    /// The place of each wanted scene in `wanted` (0 is the most wanted).
    rank: BTreeMap<SceneId, usize>,
    slots: BTreeMap<SceneId, Slot>,
    next_token: u64,
    counts: LookaheadCounts,
}

impl LookaheadPlanner {
    pub fn new(settings: LookaheadSettings) -> Self {
        Self {
            settings,
            paused: false,
            closed: false,
            wanted: Vec::new(),
            rank: BTreeMap::new(),
            slots: BTreeMap::new(),
            next_token: 1,
            counts: LookaheadCounts::default(),
        }
    }

    pub fn settings(&self) -> LookaheadSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: LookaheadSettings) {
        self.settings = settings;
    }

    /// Stops new builds from starting; builds already out run on at their low class.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn counts(&self) -> LookaheadCounts {
        self.counts
    }

    pub fn wanted(&self) -> &[Candidate] {
        &self.wanted
    }

    /// Whether the build `token` of `scene` is still the one the planner wants a result from.
    pub fn is_building(&self, scene: SceneId, token: BuildToken) -> bool {
        self.slots
            .get(&scene)
            .is_some_and(|slot| slot.token == token && matches!(slot.phase, Phase::Building { .. }))
    }

    /// How many scenes are built or building.
    pub fn booked(&self) -> usize {
        self.slots.len()
    }

    fn is_running(&self, scene: SceneId) -> bool {
        self.slots
            .get(&scene)
            .is_some_and(|slot| matches!(slot.phase, Phase::Building { .. }))
    }

    fn cancel(&mut self, scene: SceneId, reason: CancelReason) -> Option<Action> {
        let slot = self.slots.remove(&scene)?;
        if matches!(slot.phase, Phase::Building { .. }) {
            self.counts.cancelled += 1;
            Some(Action::Cancel {
                scene,
                token: slot.token,
                reason,
            })
        } else {
            None
        }
    }

    /// The story now expects `candidates` (ranked, from `lookahead_candidates`). Builds of scenes it
    /// no longer expects, or whose facts changed, are cancelled; a build that already finished for a
    /// scene no longer expected stays in the book for reuse. A closed planner refuses it.
    pub fn set_expected(
        &mut self,
        candidates: Vec<Candidate>,
    ) -> Result<Vec<Action>, LookaheadError> {
        if self.closed {
            return Err(LookaheadError::Closed);
        }
        let rank: BTreeMap<SceneId, usize> = candidates
            .iter()
            .enumerate()
            .map(|(place, candidate)| (candidate.scene(), place))
            .collect();
        let mut actions = Vec::new();
        let scenes: Vec<SceneId> = self.slots.keys().copied().collect();
        for scene in scenes {
            let now = rank.get(&scene).map(|place| &candidates[*place]);
            let Some(slot) = self.slots.get(&scene) else {
                continue;
            };
            let building = matches!(slot.phase, Phase::Building { .. });
            let failed = matches!(slot.phase, Phase::Failed { .. });
            let reason = match now {
                None if building => Some(CancelReason::Retargeted),
                Some(candidate) if (building || failed) && candidate.stamp() != slot.stamp => {
                    Some(CancelReason::StampChanged)
                }
                None if failed => Some(CancelReason::Retargeted),
                _ => None,
            };
            if let Some(reason) = reason {
                actions.extend(self.cancel(scene, reason));
            }
        }
        self.wanted = candidates;
        self.rank = rank;
        Ok(actions)
    }

    /// How many wanted scenes are built or building, and how many of those are certain. A built
    /// scene nobody wants stays in the book for reuse and is not counted: the book and the budget
    /// of `lookahead_depth` are separate.
    fn live(&self) -> (usize, usize) {
        let mut live = 0;
        let mut certain = 0;
        for (scene, slot) in &self.slots {
            if slot.is_built_or_building() && self.rank.contains_key(scene) {
                live += 1;
                if slot.expectation.is_certain() {
                    certain += 1;
                }
            }
        }
        (live, certain)
    }

    /// Cancels the lowest-ranked running build that is only likely, so a certain scene is not made
    /// to wait behind a guess. Whether it did.
    fn make_room(&mut self, actions: &mut Vec<Action>) -> bool {
        let victim = self
            .slots
            .iter()
            .filter(|(_, slot)| {
                matches!(slot.phase, Phase::Building { .. }) && !slot.expectation.is_certain()
            })
            .filter_map(|(scene, _)| self.rank.get(scene).map(|place| (*place, *scene)))
            .max();
        match victim {
            Some((_, scene)) => {
                actions.extend(self.cancel(scene, CancelReason::Retargeted));
                true
            }
            None => false,
        }
    }

    /// One look at the clock: ages out old builds, cancels builds past their deadline and starts
    /// what the rules allow. A certain scene that has no slot because likely ones fill the depth
    /// cancels the lowest-ranked running likely build and starts at once. A closed planner starts
    /// nothing. It allocates only when it starts a build.
    pub fn plan(&mut self, now_ms: u64, view: PlanView) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.closed {
            return actions;
        }
        if !self.settings.enabled() {
            let scenes: Vec<SceneId> = self.slots.keys().copied().collect();
            for scene in scenes {
                actions.extend(self.cancel(scene, CancelReason::Disabled));
            }
            return actions;
        }
        self.age_out(now_ms, &mut actions);
        if self.paused || view.foreground_busy {
            return actions;
        }
        let depth = self.settings.depth();
        let (mut live, mut live_certain) = self.live();
        let start_gate = view
            .current_ready_ms
            .map(|ready| ready.saturating_add(self.settings.start_ms()));
        for index in 0..self.wanted.len() {
            if self.slots.len() >= MAXIMUM_BOOKED_SCENES {
                break;
            }
            let candidate = &self.wanted[index];
            let scene = candidate.scene();
            let certain = candidate.is_certain();
            if live >= depth && (!certain || live_certain >= depth) {
                break;
            }
            if self.slots.contains_key(&scene) {
                continue;
            }
            if !certain && !start_gate.is_some_and(|gate| now_ms >= gate) {
                continue;
            }
            if live >= depth && self.make_room(&mut actions) {
                live -= 1;
            }
            let candidate = &self.wanted[index];
            let token = BuildToken(self.next_token);
            self.next_token += 1;
            self.counts.started += 1;
            live += 1;
            if certain {
                live_certain += 1;
            }
            self.slots.insert(
                scene,
                Slot {
                    token,
                    stamp: candidate.stamp(),
                    expectation: candidate.expectation(),
                    phase: Phase::Building { started_ms: now_ms },
                    began_ms: now_ms,
                },
            );
            actions.push(Action::Start {
                token,
                brief: candidate.brief().clone(),
                expectation: candidate.expectation(),
                likelihood_permille: candidate.likelihood_permille(),
            });
        }
        actions
    }

    fn age_out(&mut self, now_ms: u64, actions: &mut Vec<Action>) {
        let deadline = self.settings.deadline_ms();
        let book_age = self.settings.book_age_ms();
        let mut late = Vec::new();
        let mut aged = Vec::new();
        for (scene, slot) in &self.slots {
            match &slot.phase {
                Phase::Building { started_ms }
                    if now_ms.saturating_sub(*started_ms) >= deadline =>
                {
                    late.push(*scene);
                }
                Phase::Ready { ready_ms, .. } if now_ms.saturating_sub(*ready_ms) >= book_age => {
                    aged.push(*scene);
                }
                _ => {}
            }
        }
        for scene in late {
            if let Some(slot) = self.slots.get_mut(&scene) {
                let token = slot.token;
                slot.phase = Phase::Failed {
                    stage: None,
                    cause: FailureCause::TimedOut,
                };
                self.counts.cancelled += 1;
                self.counts.failed += 1;
                actions.push(Action::Cancel {
                    scene,
                    token,
                    reason: CancelReason::Deadline,
                });
            }
        }
        for scene in aged {
            self.slots.remove(&scene);
            self.counts.expired += 1;
            actions.push(Action::Expire { scene });
        }
    }

    /// The host reports how the build `token` of `scene` ended. A result for a build that is not the
    /// running one (cancelled, retargeted or replaced) is refused and stores nothing.
    pub fn build_finished(
        &mut self,
        now_ms: u64,
        scene: SceneId,
        token: BuildToken,
        outcome: BuildOutcome,
    ) -> Result<(), LookaheadError> {
        let slot = self
            .slots
            .get_mut(&scene)
            .ok_or(LookaheadError::NoBuildOf { scene })?;
        if slot.token != token || !matches!(slot.phase, Phase::Building { .. }) {
            self.counts.refused_results += 1;
            return Err(LookaheadError::StaleBuild { scene });
        }
        match outcome {
            BuildOutcome::Ready(outputs) => {
                self.counts.built += 1;
                slot.phase = Phase::Ready {
                    ready_ms: now_ms,
                    outputs,
                };
            }
            BuildOutcome::Failed { stage, cause } => {
                self.counts.failed += 1;
                slot.phase = Phase::Failed {
                    stage: Some(stage),
                    cause,
                };
            }
        }
        Ok(())
    }

    /// The table entered `scene` with the world as `now` says. What the build of it found is the
    /// answer; every other running build is cancelled (the story went another way), and a build of
    /// `scene` that was not done is cancelled as late. A built scene of another place stays in the
    /// book. The expected list is cleared: the machine states it again for the new scene.
    pub fn enter(&mut self, scene: SceneId, now: &WorldStamp) -> Entered {
        let mut actions = Vec::new();
        let others: Vec<SceneId> = self
            .slots
            .keys()
            .copied()
            .filter(|other| *other != scene)
            .collect();
        for other in others {
            if self.is_running(other) {
                actions.extend(self.cancel(other, CancelReason::Branched));
            }
        }
        self.wanted.clear();
        self.rank.clear();
        let take = match self.slots.remove(&scene) {
            None => {
                self.counts.miss += 1;
                Take::Miss
            }
            Some(slot) => match slot.phase {
                Phase::Building { .. } => {
                    self.counts.late += 1;
                    self.counts.cancelled += 1;
                    actions.push(Action::Cancel {
                        scene,
                        token: slot.token,
                        reason: CancelReason::Late,
                    });
                    Take::Late
                }
                Phase::Failed { stage, cause } => {
                    self.counts.miss += 1;
                    Take::Failed { stage, cause }
                }
                Phase::Ready { ready_ms, outputs } => {
                    let booked = BookedScene {
                        outputs,
                        ready_ms,
                        stamp: slot.stamp,
                    };
                    match slot.stamp.change_to(now) {
                        StampChange::Unchanged => {
                            self.counts.hit += 1;
                            Take::Hit(booked)
                        }
                        StampChange::CastOnly => {
                            self.counts.redressed += 1;
                            self.counts.stale += 1;
                            let mut kept = booked;
                            kept.outputs.description = None;
                            Take::Redress(kept)
                        }
                        change @ StampChange::PlaceOrTime => {
                            self.counts.stale += 1;
                            Take::Stale { change }
                        }
                    }
                }
            },
        };
        Entered { take, actions }
    }

    /// The table closed: every running build is cancelled, the book is emptied and the planner
    /// takes no more work (`set_expected` is refused, `plan` starts nothing).
    pub fn close(&mut self) -> Vec<Action> {
        self.closed = true;
        let scenes: Vec<SceneId> = self.slots.keys().copied().collect();
        let mut actions = Vec::new();
        for scene in scenes {
            actions.extend(self.cancel(scene, CancelReason::Closed));
        }
        self.slots.clear();
        self.wanted.clear();
        self.rank.clear();
        actions
    }
}

/// Read-only views of the planner, for `scene lookahead show`.
mod view {
    use std::fmt;

    use serde::{Deserialize, Serialize};

    use super::{Candidate, Expectation, LookaheadCounts, LookaheadPlanner, Phase, Slot};
    use crate::event::FailureCause;
    use crate::listed::listed_enum;
    use crate::scene_plan::SceneStage;

    /// A scene the story expects.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct WantedReport {
        pub scene: u64,
        pub place: String,
        pub expectation: Expectation,
        pub certain: bool,
        pub likelihood_permille: u16,
    }

    /// One scene in the book.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct BookReport {
        pub scene: u64,
        pub token: u64,
        pub expectation: Expectation,
        pub state: BookState,
        pub age_ms: u64,
        pub failure: Option<FailedBuild>,
    }

    listed_enum! { keyed
        /// Where a scene in the book stands.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum BookState {
            Building,
            Ready,
            Failed,
        }
    }

    impl BookState {
        pub fn key(self) -> &'static str {
            match self {
                Self::Building => "building",
                Self::Ready => "ready",
                Self::Failed => "failed",
            }
        }
    }

    impl fmt::Display for BookState {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.key())
        }
    }

    /// Why a build in the book failed: the stage that failed, or none when its deadline ran out.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct FailedBuild {
        pub stage: Option<SceneStage>,
        pub cause: FailureCause,
    }

    impl fmt::Display for FailedBuild {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            let stage = self.stage.map_or("deadline", SceneStage::key);
            write!(formatter, "{stage} {}", self.cause.key())
        }
    }

    /// The planner's settings, its expected scenes, its book and its counters.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct LookaheadReport {
        pub enabled: bool,
        pub paused: bool,
        pub depth: u32,
        pub start_s: u32,
        pub deadline_s: u32,
        pub book_age_s: u32,
        pub wanted: Vec<WantedReport>,
        pub book: Vec<BookReport>,
        pub counts: LookaheadCounts,
    }

    fn wanted_of(candidate: &Candidate) -> WantedReport {
        WantedReport {
            scene: candidate.scene().get(),
            place: candidate.brief().place().as_str().to_owned(),
            expectation: candidate.expectation(),
            certain: candidate.is_certain(),
            likelihood_permille: candidate.likelihood_permille(),
        }
    }

    fn book_of(scene: u64, slot: &Slot, now_ms: u64) -> BookReport {
        let (state, failure) = match &slot.phase {
            Phase::Building { .. } => (BookState::Building, None),
            Phase::Ready { .. } => (BookState::Ready, None),
            Phase::Failed { stage, cause } => (
                BookState::Failed,
                Some(FailedBuild {
                    stage: *stage,
                    cause: *cause,
                }),
            ),
        };
        BookReport {
            scene,
            token: slot.token.get(),
            expectation: slot.expectation,
            state,
            age_ms: now_ms.saturating_sub(slot.began_ms),
            failure,
        }
    }

    impl LookaheadPlanner {
        pub fn report(&self, now_ms: u64) -> LookaheadReport {
            let settings = self.settings();
            LookaheadReport {
                enabled: settings.enabled(),
                paused: self.is_paused(),
                depth: settings.value_of(crate::lookahead::settings::LookaheadSetting::Depth),
                start_s: settings.value_of(crate::lookahead::settings::LookaheadSetting::StartS),
                deadline_s: settings
                    .value_of(crate::lookahead::settings::LookaheadSetting::DeadlineS),
                book_age_s: settings
                    .value_of(crate::lookahead::settings::LookaheadSetting::BookAgeS),
                wanted: self.wanted.iter().map(wanted_of).collect(),
                book: self
                    .slots
                    .iter()
                    .map(|(scene, slot)| book_of(scene.get(), slot, now_ms))
                    .collect(),
                counts: self.counts,
            }
        }
    }
}

pub use view::{BookReport, BookState, FailedBuild, LookaheadReport, WantedReport};
