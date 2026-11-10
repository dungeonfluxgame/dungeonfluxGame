//! The banner's placement, numbering and working state.

use crate::home::glyph::HomeIcon;
use crate::kit::PanelTone;
use crate::kit::banner::{Banner, BannerPlace, TURN_MS, turn_delay_ms};
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

#[test]
fn rings_drawn_at_different_times_stay_at_the_angle_the_page_clock_gives() {
    let angle_at = |now: u64, drawn_at: u64| (now - drawn_at + drawn_at % TURN_MS) % TURN_MS;
    assert_eq!(turn_delay_ms(1_700), "-500ms");
    assert_eq!(angle_at(9_999, 1_700), angle_at(9_999, 6_700));
    assert_eq!(angle_at(9_999, 1_700), 9_999 % TURN_MS);
}

#[test]
fn the_turn_time_is_the_one_in_the_style_sheet() {
    assert_eq!(TURN_MS, 1_200);
    let css = include_str!("../../../../assets/playground/kit.css");
    assert!(css.contains("kit-turn 1.2s linear var(--kit-turn-delay, 0ms) infinite"));
}

/// The classes in a selector: what its specificity counts, as no banner selector has an id.
fn class_count(selector: &str) -> usize {
    selector.matches('.').count()
}

#[test]
fn no_rule_for_the_banner_icon_can_drop_the_working_rings_turn() {
    let css = include_str!("../../../../assets/playground/kit.css");
    let mut turning = None;
    let mut others = Vec::new();
    for block in css.split('}') {
        let Some((selector, body)) = block.split_once('{') else {
            continue;
        };
        let selector = selector.rsplit("*/").next().unwrap_or(selector).trim();
        if !selector.contains("kit-banner-icon") || !body.contains("animation") {
            continue;
        }
        if selector.contains("kit-turning") {
            turning = Some((selector.to_owned(), body.to_owned()));
        } else {
            others.push(selector.to_owned());
        }
    }
    let (selector, body) = turning.expect("a rule turns the working ring over the banner icon");
    assert!(body.contains("kit-turn 1.2s linear var(--kit-turn-delay, 0ms) infinite"));
    assert!(!others.is_empty());
    for other in others {
        assert!(
            class_count(&selector) > class_count(&other),
            "{other} ties or beats the working ring's rule {selector}"
        );
    }
}
