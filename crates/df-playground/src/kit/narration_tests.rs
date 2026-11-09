//! The narration box's reading, its highlights, its hint and its conversion from the old box.

use crate::home::glyph::HomeIcon;
use crate::kit::PanelTone;
use crate::kit::kit_tests::assert_css_words_are_serde_names;
use crate::kit::narration::{
    ContinueStyle, LineSegment, Narration, NarrationHeader, NarrationPlace,
};
use crate::overlay::{Glyph, NarrationBox, NarrationPlacement};

fn minimal() -> Narration {
    serde_json::from_str(r#"{"text":"The road winds on."}"#).expect("a minimal box reads")
}

fn segment(text: &str, highlight: bool) -> LineSegment {
    LineSegment {
        text: text.to_owned(),
        highlight,
    }
}

#[test]
fn a_minimal_box_is_the_spoken_gold_centre_box_of_the_dungeon_master() {
    let narration = minimal();
    assert_eq!(narration.speaker(), "Dungeon Master");
    assert!(narration.voice);
    assert_eq!(narration.placement, NarrationPlace::Center);
    assert_eq!(narration.tone, PanelTone::Gold);
    assert_eq!(narration.continue_hint(), None);
    assert_eq!(narration.emphasis(), None);
    assert!(!narration.is_compact());
}

#[test]
fn segments_that_spell_the_line_are_drawn_with_their_highlights() {
    let mut narration = minimal();
    narration.text = "The port of Valemere stirs.".to_owned();
    narration.segments = vec![
        segment("The port of ", false),
        segment("Valemere", true),
        segment(" stirs.", false),
    ];
    let drawn = narration.segments();
    assert_eq!(drawn.len(), 3);
    assert!(drawn[1].highlight);
}

#[test]
fn segments_that_do_not_spell_the_line_give_way_to_the_plain_line() {
    let mut narration = minimal();
    narration.segments = vec![segment("Something else", true)];
    let drawn = narration.segments();
    assert_eq!(drawn.len(), 1);
    assert_eq!(drawn[0].text, narration.text);
    assert!(!drawn[0].highlight);
}

#[test]
fn a_worded_continue_label_draws_the_hint_in_its_style() {
    let mut narration = minimal();
    narration.continue_label = Some(" ".to_owned());
    assert_eq!(narration.continue_hint(), None);
    narration.continue_label = Some("Continue".to_owned());
    narration.continue_style = ContinueStyle::Ring;
    let hint = narration
        .continue_hint()
        .expect("a worded label draws a hint");
    assert_eq!(hint.label, "Continue");
    assert_eq!(hint.style, ContinueStyle::Ring);
}

#[test]
fn the_old_box_converts_with_its_badge_and_placement() {
    let old = NarrationBox {
        speaker: None,
        text: "Hello".to_owned(),
        continue_label: Some("Continue".to_owned()),
        badge: Some(Glyph::Dice),
        voice: false,
        progress: None,
        placement: NarrationPlacement::Right,
    };
    let narration = Narration::from(&old);
    assert_eq!(narration.badge, Some(HomeIcon::Overlay(Glyph::Dice)));
    assert_eq!(narration.placement, NarrationPlace::Right);
    assert_eq!(narration.continue_style, ContinueStyle::Ring);
    assert!(!narration.voice);
    assert_eq!(narration.text, "Hello");
}

#[test]
fn the_twist_reads_its_tone_and_emphasis_and_a_left_box_is_compact() {
    let twist: Narration = serde_json::from_str(
        r#"{"text":"Light erupts.","emphasis":"The real threat has awakened.",
            "tone":"danger","placement":"left"}"#,
    )
    .expect("a twist reads");
    assert_eq!(twist.tone, PanelTone::Danger);
    assert_eq!(twist.emphasis(), Some("The real threat has awakened."));
    assert!(twist.is_compact());
}

#[test]
fn the_closed_keys_are_their_serde_names() {
    assert_css_words_are_serde_names(
        &[
            NarrationPlace::Center,
            NarrationPlace::Right,
            NarrationPlace::Left,
        ],
        NarrationPlace::key,
    );
    assert_css_words_are_serde_names(
        &[ContinueStyle::Chevron, ContinueStyle::Ring],
        ContinueStyle::key,
    );
}

#[test]
fn the_centre_box_has_the_tab_header_and_the_others_the_plain_one() {
    let mut narration = minimal();
    assert_eq!(narration.header_style(), NarrationHeader::Tab);
    narration.placement = NarrationPlace::Right;
    assert_eq!(narration.header_style(), NarrationHeader::Plain);
    narration.placement = NarrationPlace::Center;
    narration.tone = PanelTone::Danger;
    assert_eq!(narration.header_style(), NarrationHeader::Plain);
    narration.header = Some(NarrationHeader::Tab);
    assert_eq!(narration.header_style(), NarrationHeader::Tab);
    assert_css_words_are_serde_names(
        &[NarrationHeader::Tab, NarrationHeader::Plain],
        NarrationHeader::key,
    );
}

#[test]
fn only_the_twist_sits_under_the_top_bar() {
    let mut narration = minimal();
    assert!(!narration.sits_on_top());
    narration.tone = PanelTone::Danger;
    assert!(narration.sits_on_top());
}
