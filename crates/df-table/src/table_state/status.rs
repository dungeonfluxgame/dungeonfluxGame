//! The table's status and its rule about splitting the party.
//!
//! It owns `TableStatus` (`active`, `idle` or `paused`, D.3) and `SplitPolicy` (whether seats may
//! be in different scenes at once, RES25, K.10). It never changes either: only an effect does, in
//! `apply`, and the rules that read them live in `judge`.

use serde::{Deserialize, Serialize};

use crate::listed::listed_enum;

listed_enum! {
    /// Whether play is going on (D.3). The plate scheduler stops when the table is not `Active`,
    /// and the world clock is frozen while it is `Paused`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TableStatus {
        Active,
        /// No input for `idle_minutes`; the next play input wakes the table.
        Idle,
        /// The host paused the game; play input is declined until the host resumes.
        Paused,
    }
}

impl TableStatus {
    pub fn key(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Paused => "paused",
        }
    }
}

listed_enum! {
    /// Whether the party may be split across scenes. The first release declines a split (RES25,
    /// question 60) while the scenes list keeps the door open.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum SplitPolicy {
        Declined,
        Allowed,
    }
}

impl SplitPolicy {
    pub fn key(self) -> &'static str {
        match self {
            Self::Declined => "declined",
            Self::Allowed => "allowed",
        }
    }
}
