//! Which overlay parts of the shared screen differ between two scenes, so the page redraws only
//! the slot that changed.
//!
//! Each part has one `*_inputs` function naming every field its view reads; the part differs when
//! its inputs differ. A view that starts reading another field adds it to its inputs here, in the
//! same change, or the slot is not redrawn when that field changes.

use crate::fixture::TvScene;
use crate::home::model::HomeScene;
use crate::image_layout::ImageLayout;
use crate::lobby::model::{LoadingScreen, LobbyScreen, RoleplayScene, SceneTitle, TopBar};

/// What the panels read: the lobby, the loading screen, the roleplay card (whose anchored bubbles
/// it leaves to the world layer, decided by the backdrop's layout) and the landing page, which
/// draws the pills in its own top bar.
type PanelInputs<'a> = (
    &'a Option<LobbyScreen>,
    &'a Option<LoadingScreen>,
    &'a Option<RoleplayScene>,
    &'a Option<HomeScene>,
    Option<&'a Option<TopBar>>,
    Option<&'a ImageLayout>,
    Option<&'a ImageLayout>,
);

fn panel_inputs(scene: &TvScene) -> PanelInputs<'_> {
    (
        &scene.lobby,
        &scene.loading,
        &scene.roleplay,
        &scene.home,
        scene.home.as_ref().map(|_| &scene.pills),
        scene.backdrop_layout(),
        scene.cast_layout(),
    )
}

/// What the top bar reads: its pills and, only when there are pills, whether the landing page's
/// top bar takes their place and whether the scene's own logo is drawn (then the bar leaves its
/// logo out).
fn pill_inputs(scene: &TvScene) -> Option<(&TopBar, bool, bool)> {
    let pills = scene.pills.as_ref()?;
    let home_has_top = scene.home.as_ref().is_some_and(|home| home.top.is_some());
    Some((pills, home_has_top, scene.brand))
}

/// What the world layer's character items read: the roleplay card and the backdrop's layout.
fn anchor_inputs(scene: &TvScene) -> (Option<&RoleplayScene>, Option<&ImageLayout>) {
    (scene.roleplay.as_ref(), scene.backdrop_layout())
}

/// What the place column reads: the place's name, whether the logo is drawn and the roleplay
/// card's title banner.
fn place_inputs(scene: &TvScene) -> (&str, bool, Option<&SceneTitle>) {
    (
        &scene.place,
        scene.brand,
        scene.roleplay.as_ref().and_then(|card| card.title.as_ref()),
    )
}

/// Whether the place column differs.
pub fn place_differ(before: &TvScene, after: &TvScene) -> bool {
    place_inputs(before) != place_inputs(after)
}

/// Whether the panels (lobby, loading, roleplay, landing page) differ.
pub fn panels_differ(before: &TvScene, after: &TvScene) -> bool {
    panel_inputs(before) != panel_inputs(after)
}

/// Whether the top bar differs.
pub fn pills_differ(before: &TvScene, after: &TvScene) -> bool {
    pill_inputs(before) != pill_inputs(after)
}

/// Whether the items tied to characters (anchored bubbles, markers) differ.
pub fn anchors_differ(before: &TvScene, after: &TvScene) -> bool {
    anchor_inputs(before) != anchor_inputs(after)
}

/// Whether the story's mark differs; the chip and the line read nothing but it.
pub fn story_differ(before: &TvScene, after: &TvScene) -> bool {
    before.story != after.story
}

/// Whether the inventory panel differs; the panel reads nothing but itself.
pub fn inventory_differ(before: &TvScene, after: &TvScene) -> bool {
    before.inventory != after.inventory
}

/// Whether the clue board differs; the board reads nothing but itself.
pub fn clues_differ(before: &TvScene, after: &TvScene) -> bool {
    before.clues != after.clues
}
