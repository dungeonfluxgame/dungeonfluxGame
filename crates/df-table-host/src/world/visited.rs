//! The places designed in play that the party has been to, kept with the world state so the next
//! design is told what is already used: each place's name and the one feature that set it apart
//! (the design's `feature` id), oldest first. A design is refused when its feature repeats one kept
//! here (`movement::design::check_feature`), so two roads do not both end at the same bare road.
//! The list ends when a place of the bible is made the active one.

use serde::{Deserialize, Serialize};

use super::store::{read_visited, world_directory};
use crate::state::AppState;

/// A place the party has been to: its name and the feature id that set it apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visited {
    pub name: String,
    pub feature: String,
}

/// The places the party has been to, oldest first: none when there are none, or the world state
/// cannot be read (one `WORLD visited_unavailable error=<key>` line).
pub fn visited_of(app: &AppState) -> Vec<Visited> {
    match read_visited(&world_directory(app)) {
        Ok(visited) => visited,
        Err(error) => {
            app.record_line(format!("WORLD visited_unavailable error={}", error.key()));
            Vec::new()
        }
    }
}
