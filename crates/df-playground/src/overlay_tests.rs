//! The narration box and the seat strip: how they are read from a scene file, what they default
//! to, what a scene file may not say, and the drawings of the glyphs.

use crate::fixture::Fixture;
use crate::overlay::{Glyph, NarrationBox, NarrationPlacement, SeatStrip};

/// A fixture with the given `tv` extras spliced in after the place.
fn scene(extras: &str) -> String {
    format!(
        r#"{{"name":"S","tv":{{"place":"P","status":{{"kind":"ok"}},{extras}"elements":[]}},"phone":{{"hero":"H","status":{{"kind":"ok"}}}}}}"#
    )
}

#[test]
fn a_scene_that_names_no_overlay_is_read_as_before() {
    let fixture = Fixture::parse(&scene("")).expect("reads");
    assert_eq!(fixture.tv.narration, None);
    assert_eq!(fixture.tv.seats, None);
}

#[test]
fn a_narration_box_needs_only_its_text_and_the_rest_defaults() {
    let fixture = Fixture::parse(&scene(r#""narration":{"text":"Rain."},"#)).expect("reads");
    let narration = fixture.tv.narration.expect("a box");
    assert_eq!(
        narration,
        NarrationBox {
            speaker: None,
            text: "Rain.".to_owned(),
            continue_label: None,
            badge: None,
            voice: true,
            progress: None,
            placement: NarrationPlacement::Center,
        }
    );
}

#[test]
fn a_seat_strip_defaults_to_no_looks_and_no_open_seats() {
    let fixture = Fixture::parse(&scene(r#""seats":{},"#)).expect("reads");
    assert_eq!(fixture.tv.seats, Some(SeatStrip::default()));
}

#[test]
fn a_seat_strip_reads_looks_and_open_seats() {
    let fixture = Fixture::parse(&scene(
        r##""seats":{"looks":[{"name":"Kael","class":"Fighter","level":3,"glyph":"shield","accent":"#e0675a"}],"open":2},"##,
    ))
    .expect("reads");
    let seats = fixture.tv.seats.expect("seats");
    assert_eq!(seats.open, 2);
    assert_eq!(seats.looks.len(), 1);
    assert_eq!(seats.looks[0].glyph, Some(Glyph::Shield));
    assert_eq!(seats.looks[0].level, Some(3));
}

#[test]
fn a_misspelt_key_or_a_glyph_that_is_not_drawn_is_refused_with_its_position() {
    for extras in [
        r#""narration":{"text":"x","speeker":"y"},"#,
        r#""narration":{"text":"x","badge":"trophy"},"#,
        r#""seats":{"open":300},"#,
        r#""seats":{"looks":[{"nam":"Kael"}]},"#,
        r#""narration":{},"#,
    ] {
        let error = Fixture::parse(&scene(extras)).expect_err(extras);
        assert!(error.line >= 1 && error.column >= 1, "{extras}: {error:?}");
    }
}

#[test]
fn a_scene_with_both_a_caption_and_a_narration_box_is_refused_where_the_box_is() {
    let error = Fixture::parse(&scene(
        r#""caption":{"text":"a"},"narration":{"text":"b"},"#,
    ))
    .expect_err("both");
    assert!(error.message.contains("narration"), "{}", error.message);
    assert!(error.message.contains("not both"), "{}", error.message);
}

#[test]
fn a_scene_number_outside_its_total_is_refused() {
    for progress in [r#"{"index":0,"total":5}"#, r#"{"index":6,"total":5}"#] {
        let error = Fixture::parse(&scene(&format!(
            r#""narration":{{"text":"b","progress":{progress}}},"#
        )))
        .expect_err(progress);
        assert!(
            error.message.contains("does not exist"),
            "{}",
            error.message
        );
        assert!(error.message.contains("progress"), "{}", error.message);
    }
    Fixture::parse(&scene(
        r#""narration":{"text":"b","progress":{"index":5,"total":5,"title":"End"}},"#,
    ))
    .expect("the last scene exists");
}

#[test]
fn blank_text_in_the_overlay_is_refused() {
    for extras in [
        r#""narration":{"text":"  "},"#,
        r#""narration":{"text":"x","speaker":""},"#,
        r#""seats":{"looks":[{"name":" "}]},"#,
    ] {
        assert!(Fixture::parse(&scene(extras)).is_err(), "{extras}");
    }
}

#[test]
fn every_glyph_has_a_drawing_made_of_shapes_and_no_script() {
    let all = [
        Glyph::Sword,
        Glyph::Shield,
        Glyph::Bow,
        Glyph::Sun,
        Glyph::Book,
        Glyph::Scales,
        Glyph::Mug,
        Glyph::Mask,
        Glyph::Hammer,
        Glyph::Plus,
        Glyph::Voice,
        Glyph::Dice,
        Glyph::Crown,
        Glyph::Star,
        Glyph::Check,
        Glyph::Lock,
        Glyph::Pencil,
    ];
    for glyph in all {
        let markup = glyph.markup();
        assert!(markup.starts_with('<'), "{glyph:?}");
        assert!(
            !markup.contains("script") && !markup.contains(" on"),
            "{glyph:?}"
        );
        assert_eq!(
            markup.matches('<').count(),
            markup.matches("/>").count(),
            "{glyph:?}"
        );
    }
}

#[test]
fn a_glyph_is_named_the_same_in_a_scene_file_and_on_the_command_line() {
    for glyph in Glyph::ALL {
        let written = serde_json::to_string(&glyph).expect("writes");
        assert_eq!(written, format!("\"{}\"", glyph.key()));
        assert_eq!(Glyph::from_key(glyph.key()), Some(glyph));
        assert!(!glyph.markup().is_empty());
    }
    assert_eq!(Glyph::from_key("trophy"), None);
    assert!(Glyph::keys_text().starts_with("sword, shield"));
}

#[test]
fn a_placement_is_named_the_same_in_a_scene_file_and_on_the_command_line() {
    for placement in [NarrationPlacement::Center, NarrationPlacement::Right] {
        let written = serde_json::to_string(&placement).expect("writes");
        assert_eq!(written, format!("\"{}\"", placement.key()));
        assert_eq!(
            NarrationPlacement::from_key(placement.key()),
            Some(placement)
        );
    }
    assert_eq!(NarrationPlacement::from_key("left"), None);
}
