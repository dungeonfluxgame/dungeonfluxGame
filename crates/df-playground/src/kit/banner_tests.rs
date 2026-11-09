//! The banner's placement, numbering and working state.

use crate::home::glyph::HomeIcon;
use crate::kit::PanelTone;
use crate::kit::banner::{Banner, BannerPlace};
use crate::kit::kit_tests::assert_css_words_are_serde_names;
use crate::lobby::model::SceneTitle;
use crate::overlay::Glyph;

#[test]
fn a_gold_banner_is_a_header_and_a_danger_one_sits_at_the_top_centre() {
    let header: Banner =
        serde_json::from_str(r#"{"title":"Negotiate","number":3,"icon":"scales"}"#)
            .expect("a banner reads");
    assert_eq!(header.placement(), BannerPlace::Header);
    assert_eq!(header.number_text().as_deref(), Some("3."));
    let combat: Banner =
        serde_json::from_str(r#"{"title":"Combat Encounter","tone":"danger","icon":"swords"}"#)
            .expect("a banner reads");
    assert_eq!(combat.tone, PanelTone::Danger);
    assert_eq!(combat.placement(), BannerPlace::TopCenter);
    assert_eq!(combat.number_text(), None);
}

#[test]
fn a_scene_title_converts_and_the_spinner_marks_work_under_way() {
    let banner = Banner::from(&SceneTitle {
        glyph: Glyph::Spinner,
        text: "Drawing the scene".to_owned(),
    });
    assert!(banner.is_working());
    assert_eq!(banner.icon, Some(HomeIcon::Overlay(Glyph::Spinner)));
    assert_eq!(banner.title(), Some("Drawing the scene"));
}

#[test]
fn a_blank_title_is_not_drawn_and_the_keys_are_serde_names() {
    let blank: Banner = serde_json::from_str(r#"{"title":" "}"#).expect("a banner reads");
    assert_eq!(blank.title(), None);
    assert_css_words_are_serde_names(
        &[BannerPlace::Header, BannerPlace::TopCenter],
        BannerPlace::key,
    );
}
