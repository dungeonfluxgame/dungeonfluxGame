//! The questions the table state answers without changing anything: may the party split, may a
//! play input run, has the table been quiet long enough to idle, and is this result still current.
//!
//! It owns the answers and their typed shapes: `split_party_decline` and `judge_split` (RES25,
//! K.10), `PlayInput` and `judge_play_input` (D.3, RES29), `IdleClock`, `InFlight`, `IdleVerdict`
//! and `judge_idle` (D.3), and `ResultStamp`, `StaleCause`, `StaleResultError` and `judge_result`
//! (D.2, RES42). Each is a pure function of a snapshot and the values handed in, so a test or the
//! CLI asks the same question the pipeline does. The time and the number of pending resolutions and
//! held turns arrive as arguments: the table reads no clock, and pending resolutions belong to
//! the pending handler.
//!
//! A verdict is advice for the caller to commit as an effect (`TableIdled`, `TableResumed`); it
//! never changes the table, and a declined answer is a typed `Decline` the player reads (K.5).

use serde::{Deserialize, Serialize};

use crate::event::TableEvent;
use crate::identity::{Correlation, SceneId, SeatId, SessionRevision, TableScope};
use crate::listed::listed_enum;
use crate::outcome::{Decline, DeclineCode};
use crate::table_state::events::TableStateEvent;
use crate::table_state::scene::SceneGeneration;
use crate::table_state::snapshot::TableSnapshot;
use crate::table_state::status::{SplitPolicy, TableStatus};

const SPLIT_REASON: &str = "The party stays together for now. To be on your own, move to \
    another area of this place, or ask the host to pause.";
const SPLIT_ALTERNATIVES: [&str; 2] = [
    "Move to another area of this place",
    "Ask the host to pause",
];
const ALONE_REASON: &str = "There is no one else at the table to split from.";
const PAUSED_REASON: &str = "The game is paused. The host can resume it.";

/// A decline whose authored text is used, or the code's own default reason if that text were ever
/// refused: a named, deterministic fallback and never a panic.
fn authored(code: DeclineCode, reason: &str, alternatives: &[&str]) -> Decline {
    let Ok(mut decline) = Decline::new(code, reason) else {
        return Decline::with_default_reason(code);
    };
    for alternative in alternatives {
        match decline.with_alternative(alternative) {
            Ok(with_alternative) => decline = with_alternative,
            Err(_) => return Decline::with_default_reason(code),
        }
    }
    decline
}

/// The decline of a `Split` while the table keeps the party together (RES25): `not_supported_yet`,
/// with the two things the player can do instead.
pub fn split_party_decline() -> Decline {
    authored(
        DeclineCode::NotSupportedYet,
        SPLIT_REASON,
        &SPLIT_ALTERNATIVES,
    )
}

/// What the table does with a play input given its status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayInput {
    Proceed,
    /// The table was idle: commit `TableResumed` first, then run the input.
    WakeIdleTable,
}

/// The clock a table reads to decide it has been quiet: all of it handed in (H.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleClock {
    pub last_input_ms: u64,
    pub now_ms: u64,
    /// `idle_minutes` of J.4 in milliseconds.
    pub idle_after_ms: u64,
}

/// What keeps a table from idling (D.3): a table cannot go idle while any pending resolution or
/// held turn exists, and a held turn counts as activity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InFlight {
    pub open_pendings: usize,
    pub held_turns: usize,
}

impl InFlight {
    pub fn is_clear(self) -> bool {
        self.open_pendings == 0 && self.held_turns == 0
    }
}

/// The answer to "should this table go idle now".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleVerdict {
    /// Commit `TableIdled`.
    GoIdle,
    /// Not quiet long enough: this many milliseconds remain.
    StayActive { idle_in_ms: u64 },
    /// Something is waiting on a person, so the table cannot idle.
    Blocked(InFlight),
    /// The table is idle or paused already.
    NotActive(TableStatus),
}

/// What a job or result was started under, which `judge_result` checks (D.2, RES42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultStamp {
    pub scene: SceneId,
    pub generation: SceneGeneration,
    /// The revision of the snapshot the job was built from.
    pub revision: SessionRevision,
}

listed_enum! {
    /// Why a result is no longer current.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum StaleCause {
        /// It was started before the last recovery.
        OlderRecoveryEpoch,
        /// The table is paused: results in flight are held back until it resumes.
        TablePaused,
        /// It was started before the last pause, which flushed the queue (D.3).
        BeforePause,
        /// The scene it was for is not open any more.
        SceneClosed,
        /// The scene was left and entered again, or never had this generation.
        GenerationMismatch,
    }
}

impl StaleCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::OlderRecoveryEpoch => "older_recovery_epoch",
            Self::TablePaused => "table_paused",
            Self::BeforePause => "before_pause",
            Self::SceneClosed => "scene_closed",
            Self::GenerationMismatch => "generation_mismatch",
        }
    }
}

/// A result that is no longer current: stored as late and logged, never shown (D.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleResultError {
    pub cause: StaleCause,
    pub stamp: ResultStamp,
    /// The generation the scene has now, when it is open.
    pub current_generation: Option<SceneGeneration>,
}

impl std::fmt::Display for StaleResultError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "a result of generation {} is no longer current ({})",
            self.stamp.generation.get(),
            self.cause.key()
        )
    }
}

impl std::error::Error for StaleResultError {}

impl StaleResultError {
    /// The class A event that records the rejection, once, where it is handled.
    pub fn event(&self, scope: &TableScope, correlation: Correlation) -> TableEvent {
        TableEvent::TableState(TableStateEvent::ResultRejected {
            scope: scope.clone(),
            correlation,
            cause: self.cause,
            generation: self.stamp.generation.get(),
            current: self.current_generation.map(SceneGeneration::get),
        })
    }
}

impl TableSnapshot {
    /// Whether `seat` may split off from the party. A seat that is not at the table, or is the only
    /// one there, has nothing to split from (`not_viable`). Otherwise, while the table keeps the
    /// party together every split is declined (RES25, question 60). The applier refuses the same
    /// move for the same reason, as `split_party_declined`.
    pub fn judge_split(&self, seat: SeatId) -> Result<(), Decline> {
        if self.seat(seat).is_none() {
            return Err(Decline::with_default_reason(DeclineCode::NotViable));
        }
        if !self.facts.has_company(seat) {
            return Err(authored(DeclineCode::NotViable, ALONE_REASON, &[]));
        }
        match self.split_policy() {
            SplitPolicy::Declined => Err(split_party_decline()),
            SplitPolicy::Allowed => Ok(()),
        }
    }

    /// Whether a play input may run now. A paused table declines it as `time_locked` (the public
    /// class `paused`); an idle table runs it after waking.
    pub fn judge_play_input(&self) -> Result<PlayInput, Decline> {
        match self.status() {
            TableStatus::Active => Ok(PlayInput::Proceed),
            TableStatus::Idle => Ok(PlayInput::WakeIdleTable),
            TableStatus::Paused => Err(authored(DeclineCode::TimeLocked, PAUSED_REASON, &[])),
        }
    }

    /// Whether the table should go idle (D.3). A clock that ran backwards counts as no time passed.
    pub fn judge_idle(&self, clock: IdleClock, in_flight: InFlight) -> IdleVerdict {
        if self.status() != TableStatus::Active {
            return IdleVerdict::NotActive(self.status());
        }
        if !in_flight.is_clear() {
            return IdleVerdict::Blocked(in_flight);
        }
        let quiet_ms = clock.now_ms.saturating_sub(clock.last_input_ms);
        match clock.idle_after_ms.checked_sub(quiet_ms) {
            Some(remaining) if remaining > 0 => IdleVerdict::StayActive {
                idle_in_ms: remaining,
            },
            _ => IdleVerdict::GoIdle,
        }
    }

    /// Whether a result started under `stamp` may still be used (D.2, D.3, RES42).
    pub fn judge_result(&self, stamp: &ResultStamp) -> Result<(), StaleResultError> {
        let current_generation = self.scene(stamp.scene).map(|record| record.generation());
        let stale = |cause| StaleResultError {
            cause,
            stamp: *stamp,
            current_generation,
        };
        if stamp.revision.epoch() < self.revision().epoch() {
            return Err(stale(StaleCause::OlderRecoveryEpoch));
        }
        if self.status() == TableStatus::Paused {
            return Err(stale(StaleCause::TablePaused));
        }
        if self
            .last_paused_at()
            .is_some_and(|paused| stamp.revision <= paused)
        {
            return Err(stale(StaleCause::BeforePause));
        }
        match current_generation {
            None => Err(stale(StaleCause::SceneClosed)),
            Some(generation) if generation != stamp.generation => {
                Err(stale(StaleCause::GenerationMismatch))
            }
            Some(_) => Ok(()),
        }
    }
}
