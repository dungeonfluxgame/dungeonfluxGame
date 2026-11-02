//! The lanes of the owner queue, the causes of a refusal that is not a decline, and the bounds.
//!
//! It owns the closed list of lanes in the order they are served, the closed list of refusal causes
//! an event names, and `QueueLimits` with the named large constants behind its defaults. It never
//! queues anything: `owner_queue` does.

use serde::{Deserialize, Serialize};

use crate::listed::listed_enum;

/// The most seats one queue tracks. Deliberately huge so it never limits play; scope it down later
/// (owner, 2026-11-02).
pub const MAXIMUM_SEATS: usize = 1_000_000;

/// The most inputs the engine lane holds. The plan gives no number: J.4 leaves the lane caps to the
/// first run over real adapters. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_ENGINE_LANE_LENGTH: usize = 1_000_000;

/// The most inputs the timers and results lane holds. Same status as the engine lane cap.
pub const MAXIMUM_TIMERS_AND_RESULTS_LANE_LENGTH: usize = 1_000_000;

/// The most seat controls (pause cards and pending answers) one seat may have waiting. The plan
/// gives no number. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_SEAT_CONTROLS_PENDING: usize = 1_000_000;

/// The most seat controls waiting at the table in all, across every seat. Without it a seat could
/// queue a million per seat and keep the engine and player lanes waiting behind them. The plan gives
/// no number. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_CONTROLS_LANE_LENGTH: usize = 1_000_000;

/// The most finished operations whose result a duplicate can still find (O.7); the oldest finished
/// one is forgotten first. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_REMEMBERED_OPERATIONS: usize = 1_000_000;

/// The longest an input may stay in progress before `reap_expired` may abandon it, in milliseconds
/// (one hour). J.4's 60 s job lease and 20 s stall watchdog are the broker's; this is the owner's
/// backstop for a lost `finish`. Deliberately huge; scope it down later (owner, 2026-11-02).
pub const MAXIMUM_IN_PROGRESS_MS: u64 = 3_600_000;

listed_enum! {
    /// The lanes of the owner queue, declared in the order they are served. H.2 names three; the
    /// seat controls lane is the fourth, served first.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Lane {
        /// The pause card and the answer to a pending, round-robin by seat, never rate limited.
        SeatControls,
        /// Engine proposals and machine hand-offs (RES21).
        Engine,
        /// Timers, and results of resource requests that ran off the owner (RES22).
        TimersAndResults,
        /// Seat input that costs a model call or a turn, served round-robin by seat.
        Player,
    }
}

impl Lane {
    pub fn key(self) -> &'static str {
        match self {
            Self::SeatControls => "seat_controls",
            Self::Engine => "engine",
            Self::TimersAndResults => "timers_and_results",
            Self::Player => "player",
        }
    }
}

listed_enum! {
    /// Why an input was refused without a decline code: nothing a player is told, so the event
    /// names the cause instead.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum NotDeclinedCause {
        /// The seat is not at this table.
        UnknownSeat,
        /// The sender sent this operation id before; the first one's status is the answer.
        Duplicate,
        /// A lane or a seat's controls are full: a stall, not a decline (RES22).
        LaneStalled,
    }
}

impl NotDeclinedCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::UnknownSeat => "unknown_seat",
            Self::Duplicate => "duplicate",
            Self::LaneStalled => "lane_stalled",
        }
    }
}

/// The bounds of one queue other than the per-seat settings. Each is a limit on a collection that
/// grows with play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueLimits {
    pub seats: usize,
    pub engine_lane_length: usize,
    pub timers_and_results_lane_length: usize,
    /// Seat controls one seat may have waiting.
    pub seat_controls_pending: usize,
    /// Seat controls waiting at the table in all.
    pub controls_lane_length: usize,
    /// Finished operations remembered for duplicate detection; zero remembers none.
    pub remembered_operations: usize,
    /// How long an input may stay in progress before it can be reaped, in milliseconds.
    pub in_progress_lease_ms: u64,
}

impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            seats: MAXIMUM_SEATS,
            engine_lane_length: MAXIMUM_ENGINE_LANE_LENGTH,
            timers_and_results_lane_length: MAXIMUM_TIMERS_AND_RESULTS_LANE_LENGTH,
            seat_controls_pending: MAXIMUM_SEAT_CONTROLS_PENDING,
            controls_lane_length: MAXIMUM_CONTROLS_LANE_LENGTH,
            remembered_operations: MAXIMUM_REMEMBERED_OPERATIONS,
            in_progress_lease_ms: MAXIMUM_IN_PROGRESS_MS,
        }
    }
}
