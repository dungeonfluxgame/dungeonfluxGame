//! Which overlay parts of the shared screen differ between two scenes, so the page redraws only
//! the slot that changed.
//!
//! The agents that add a component's data to `TvScene` add its comparison here: a function of the
//! two scenes, with a native test, and nothing else.

use crate::fixture::TvScene;

/// Whether the panels (lobby, roleplay) differ.
pub fn panels_differ(before: &TvScene, after: &TvScene) -> bool {
    lobby_screens_differ(before, after) || before.home != after.home
}

/// Whether the pills differ.
pub fn pills_differ(before: &TvScene, after: &TvScene) -> bool {
    before.pills != after.pills
}

/// Whether the lobby, the loading screen or the roleplay card differ (`lobby_model`); the panels
/// comparison above includes this.
pub fn lobby_screens_differ(before: &TvScene, after: &TvScene) -> bool {
    (&before.lobby, &before.loading, &before.roleplay)
        != (&after.lobby, &after.loading, &after.roleplay)
}
