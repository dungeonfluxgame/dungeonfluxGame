//! The status banner restyled (component 22): a night-glass capsule at the top right, with the
//! ember rim while the status blocks input. The words and the blocking rule are `copy`'s.

use serde::{Deserialize, Serialize};

use crate::copy::{status_blocks_input, status_copy};
use crate::fixture::Status;

/// The banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusChip {
    pub status: Status,
}

impl StatusChip {
    /// The words, or `None` when all is well and nothing is drawn.
    pub fn text(&self) -> Option<String> {
        status_copy(&self.status)
    }

    pub fn blocks(&self) -> bool {
        status_blocks_input(&self.status)
    }
}
