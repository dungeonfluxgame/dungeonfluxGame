//! Each part redraws when anything its view reads changes, and only then.

use crate::fixture::Fixture;
use crate::lobby::model::{LogoStyle, TopBar};
use crate::overlay::changes::{anchors_differ, panels_differ, pills_differ};

fn roleplay_scene() -> Fixture {
    Fixture::parse(include_str!(
        "../../../../assets/playground/fixtures/ui_roleplay_thought.json"
    ))
    .expect("the roleplay fixture reads")
}

fn home_scene() -> Fixture {
    Fixture::parse(include_str!(
        "../../../../assets/playground/fixtures/ui_home.json"
    ))
    .expect("the home fixture reads")
}

#[test]
fn turning_the_scene_logo_on_redraws_the_top_bar_that_drew_its_own() {
    let before = roleplay_scene();
    let mut after = before.clone();
    after.tv.brand = !before.tv.brand;
    assert!(before.tv.pills.is_some());
    assert!(pills_differ(&before.tv, &after.tv));
    // Without pills there is no bar to redraw.
    let (mut bare_before, mut bare_after) = (before.clone(), after);
    bare_before.tv.pills = None;
    bare_after.tv.pills = None;
    assert!(!pills_differ(&bare_before.tv, &bare_after.tv));
}

#[test]
fn a_pill_change_under_the_landing_page_redraws_its_panel() {
    let before = home_scene();
    let mut after = before.clone();
    after.tv.pills = Some(TopBar {
        logo: Some(LogoStyle::Inline),
        ..TopBar::default()
    });
    assert!(before.tv.home.is_some());
    assert!(panels_differ(&before.tv, &after.tv));
}

#[test]
fn a_new_layout_redraws_the_panels_and_the_character_items() {
    let before = roleplay_scene();
    let mut after = before.clone();
    if let Some(backdrop) = after.tv.backdrop.as_mut() {
        backdrop.layout = None;
    }
    let mut with_layout = before.clone();
    if let Some(backdrop) = with_layout.tv.backdrop.as_mut() {
        backdrop.layout = Some(df_scene_layout::SceneLayout {
            schema: df_scene_layout::SCHEMA_VERSION,
            picture: df_scene_layout::PictureBinding {
                name: backdrop.media.clone(),
                sha256: "ab".repeat(32),
                width_px: 1672,
                height_px: 941,
            },
            model: "sam-3.1".to_owned(),
            prompt: "person".to_owned(),
            regions: Vec::new(),
        });
    }
    assert!(panels_differ(&after.tv, &with_layout.tv));
    assert!(anchors_differ(&after.tv, &with_layout.tv));
    assert!(!anchors_differ(&before.tv, &before.tv));
}
