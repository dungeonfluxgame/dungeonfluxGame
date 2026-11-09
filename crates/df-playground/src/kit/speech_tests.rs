//! The speech bubble's words, the thinking cue, its place and its conversion from the old bubble.

use crate::home::glyph::HomeIcon;
use crate::kit::kit_tests::assert_css_words_are_serde_names;
use crate::kit::speech::{BubblePlace, NameStyle, SpeechBubble, TailLean, TailSide};
use crate::lobby::model::{Bubble, BubbleTone};
use crate::overlay::Glyph;

fn bubble(json: &str) -> SpeechBubble {
    serde_json::from_str(json).expect("a bubble reads")
}

#[test]
fn a_bubble_with_words_shows_them_and_its_emphasis() {
    let said = bubble(r#"{"speaker":" Kael ","line":"Too dear.","emphasis":"Far too dear."}"#);
    assert!(!said.is_thinking());
    assert_eq!(said.speaker(), "Kael");
    assert_eq!(said.line(), Some("Too dear."));
    assert_eq!(said.emphasis(), Some("Far too dear."));
    assert_eq!(said.name_style, NameStyle::Head);
    assert_eq!(said.tail, TailSide::Below);
    assert_eq!(said.tone, BubbleTone::Ally);
}

#[test]
fn no_words_or_the_thought_tone_is_the_thinking_cue_and_shows_no_words() {
    assert!(bubble(r#"{"speaker":"Kael"}"#).is_thinking());
    let thought = bubble(r#"{"speaker":"Kael","line":"secret","emphasis":"x","tone":"thought"}"#);
    assert!(thought.is_thinking());
    assert_eq!(thought.line(), None);
    assert_eq!(thought.emphasis(), None);
}

#[test]
fn a_place_is_drawn_where_it_was_given_with_its_tail_off_the_corners() {
    let mut free = bubble(r#"{"speaker":"Kael","line":"Hi"}"#);
    free.place = Some(BubblePlace {
        left_percent: 70,
        tail_y_percent: 12,
        width_percent: 2,
        tail_percent: 100,
    });
    let drawn = free.drawn_place().expect("a place");
    assert_eq!(drawn.width_percent, 8);
    assert_eq!(drawn.left_percent, 70, "the bubble is not moved");
    assert_eq!(drawn.tail_y_percent, 12, "the bubble is not moved");
    assert_eq!(drawn.tail_percent, 90);
    assert!(!free.tail_leans_left());
}

#[test]
fn a_tail_in_the_left_half_leans_left() {
    let mut free = bubble(r#"{"speaker":"Kael","line":"Hi"}"#);
    assert!(free.tail_leans_left());
    free.place = Some(BubblePlace {
        left_percent: 10,
        tail_y_percent: 40,
        width_percent: 20,
        tail_percent: 30,
    });
    assert!(free.tail_leans_left());
}

#[test]
fn the_old_bubble_converts_with_its_tail_side_and_place() {
    let old = Bubble {
        speaker: "Lyra".to_owned(),
        line: "Wait.".to_owned(),
        emphasis: None,
        glyph: Some(Glyph::Bow),
        portrait: None,
        tone: BubbleTone::Hero,
        left_percent: 20,
        top_percent: 40,
        width_percent: 20,
        tail_x_percent: 25,
        tail_y_percent: 30,
        anchor: None,
    };
    let converted = SpeechBubble::from(&old);
    assert_eq!(converted.tail, TailSide::Above);
    assert_eq!(converted.glyph, Some(HomeIcon::Overlay(Glyph::Bow)));
    let place = converted.place.expect("the old numbers place it");
    assert_eq!(place.tail_percent, 25);
    assert_eq!(place.left_percent, 20);
    assert_eq!(place.tail_y_percent, 30);
}

#[test]
fn the_closed_keys_are_their_serde_names() {
    assert_css_words_are_serde_names(&[NameStyle::Head, NameStyle::Tab], NameStyle::key);
    assert_css_words_are_serde_names(
        &[
            TailSide::Below,
            TailSide::Above,
            TailSide::Left,
            TailSide::Right,
        ],
        TailSide::key,
    );
}

#[test]
fn the_tail_is_held_where_its_place_put_it() {
    let place = BubblePlace {
        left_percent: 70,
        tail_y_percent: 50,
        width_percent: 25,
        tail_percent: 40,
    };
    assert!((place.tail_offset_percent() - 10.0).abs() < f32::EPSILON);
}

#[test]
fn a_given_lean_wins_over_the_side_of_the_place() {
    let mut said = bubble(r#"{"speaker":"Kael","line":"Hm."}"#);
    assert!(said.tail_leans_left());
    said.lean = Some(TailLean::Right);
    assert!(!said.tail_leans_left());
}
