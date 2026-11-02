//! A fixture file passes the ranges the overlay and the command line enforce, and a break is
//! refused with the line of the value.

use crate::fixture::{Fixture, FixtureError};
use crate::limits::{
    MAXIMUM_CAMERA_MS, MAXIMUM_NAME_CHARACTERS, MAXIMUM_SLIDE_MS, MAXIMUM_ZOOM_PERCENT,
};

/// A fixture with `tv` extras (between the status and the elements), `phone` extras, a `music`
/// entry and the `elements` list, each on lines of their own.
fn text(tv_extra: &str, phone_extra: &str, music: &str, elements: &str) -> String {
    format!(
        "{{\n  \"name\": \"F\",\n  \"tv\": {{\n    \"place\": \"P\",\n    \"status\": {{\"kind\": \"ok\"}},\n    {tv_extra}\n    \"elements\": {elements}\n  }},\n  \"phone\": {{\"hero\": \"H\", \"status\": {{\"kind\": \"ok\"}}{phone_extra}}},\n  {music}\n  \"description\": \"\"\n}}"
    )
}

fn line_of(text: &str, needle: &str) -> usize {
    text.lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
        .unwrap_or_else(|| panic!("no line with {needle} in {text}"))
}

fn refused(text: &str) -> FixtureError {
    Fixture::parse(text).expect_err("refused")
}

fn card(rotation: u32, slide: &str) -> String {
    text(
        &format!(
            "\"card\": {{\"media\": \"a.png\", \"caption\": null, \"rotation_degrees\": {rotation}, \"slide\": {slide}}},"
        ),
        "",
        "",
        "[]",
    )
}

#[test]
fn a_card_turn_of_400_is_refused_with_the_range_and_the_line() {
    let source = card(400, "null");
    let error = refused(&source);
    assert_eq!(error.line, line_of(&source, "rotation_degrees"));
    assert!(error.column > 1);
    assert_eq!(
        error.message,
        "tv.card.rotation_degrees: a rotation is from 0 to 359 degrees, not 400"
    );
}

#[test]
fn the_edges_of_every_range_are_accepted() {
    let source = text(
        "\"card\": {\"media\": \"a.png\", \"caption\": null, \"rotation_degrees\": 359, \"slide\": {\"from_percent\": 100, \"to_percent\": 0, \"duration_ms\": 1}},\n    \"camera\": {\"target\": \"a\", \"zoom_percent\": 100, \"duration_ms\": 0},",
        "",
        "\"music\": {\"media\": \"m.ogg\", \"target\": \"both\", \"volume\": 100, \"looping\": true},",
        "[{\"id\": \"a\", \"kind\": \"k\", \"label\": \"A\", \"x\": 1, \"y\": 1, \"color\": null, \"shape\": \"circle\", \"width_units\": 1, \"height_units\": 1, \"rotation_degrees\": 0, \"pivot\": {\"x_percent\": 100, \"y_percent\": 0}}]",
    );
    Fixture::parse(&source).expect("reads");
    let widest = text(
        &format!(
            "\"card\": {{\"media\": \"a.png\", \"caption\": null, \"slide\": {{\"from_percent\": 0, \"to_percent\": 100, \"duration_ms\": {MAXIMUM_SLIDE_MS}}}}},\n    \"camera\": {{\"target\": null, \"zoom_percent\": {MAXIMUM_ZOOM_PERCENT}, \"duration_ms\": {MAXIMUM_CAMERA_MS}}},"
        ),
        "",
        "",
        "[]",
    );
    Fixture::parse(&widest).expect("reads");
}

#[test]
fn a_slide_off_the_screen_or_of_no_length_is_refused() {
    for (slide, needle) in [
        (
            "{\"from_percent\": 200, \"to_percent\": 255, \"duration_ms\": 0}",
            "from_percent",
        ),
        (
            "{\"from_percent\": 10, \"to_percent\": 101, \"duration_ms\": 10}",
            "to_percent",
        ),
        (
            "{\"from_percent\": 10, \"to_percent\": 90, \"duration_ms\": 0}",
            "duration_ms",
        ),
    ] {
        let source = card(0, slide);
        let error = refused(&source);
        assert!(
            error.message.starts_with("tv.card.slide: "),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("0 to 100 percent"),
            "{}",
            error.message
        );
        assert_eq!(error.line, line_of(&source, needle));
    }
    let too_long = card(
        0,
        &format!(
            "{{\"from_percent\": 10, \"to_percent\": 90, \"duration_ms\": {}}}",
            MAXIMUM_SLIDE_MS + 1
        ),
    );
    assert!(
        refused(&too_long)
            .message
            .contains(&MAXIMUM_SLIDE_MS.to_string())
    );
}

#[test]
fn only_the_card_may_slide_in_a_fixture_as_in_the_overlay() {
    let source = text(
        "\"backdrop\": {\"media\": \"a.png\", \"caption\": null, \"slide\": {\"from_percent\": 1, \"to_percent\": 2, \"duration_ms\": 5}},",
        "",
        "",
        "[]",
    );
    let error = refused(&source);
    assert_eq!(
        error.message,
        "tv.backdrop.slide: only the card on the shared screen can slide"
    );
    let phone = text(
        "",
        ", \"image\": {\"media\": \"a.png\", \"caption\": null, \"slide\": {\"from_percent\": 1, \"to_percent\": 2, \"duration_ms\": 5}}",
        "",
        "[]",
    );
    assert!(refused(&phone).message.starts_with("phone.image.slide: "));
}

#[test]
fn a_camera_with_a_zoom_a_time_or_a_target_out_of_range_is_refused() {
    let camera = |target: &str, zoom: u32, ms: u64, elements: &str| {
        text(
            &format!(
                "\"camera\": {{\"target\": {target}, \"zoom_percent\": {zoom}, \"duration_ms\": {ms}}},"
            ),
            "",
            "",
            elements,
        )
    };
    let finn = "[{\"id\": \"finn\", \"kind\": \"k\", \"label\": \"F\", \"x\": 1, \"y\": 1, \"color\": null, \"shape\": \"circle\", \"width_units\": 1, \"height_units\": 1}]";
    let zoom = camera("\"finn\"", 65_535, 4_000_000_000, finn);
    let error = refused(&zoom);
    assert!(
        error.message.starts_with("tv.camera.zoom_percent: "),
        "{}",
        error.message
    );
    assert!(error.message.contains("65535"));
    assert_eq!(error.line, line_of(&zoom, "zoom_percent"));
    let time = camera("\"finn\"", 200, 4_000_000_000, finn);
    assert!(
        refused(&time)
            .message
            .starts_with("tv.camera.duration_ms: ")
    );
    let low = camera("\"finn\"", 99, 0, finn);
    assert!(refused(&low).message.contains("99"));
    let long = camera(
        &format!("\"{}\"", "x".repeat(MAXIMUM_NAME_CHARACTERS + 1)),
        200,
        0,
        finn,
    );
    assert!(refused(&long).message.starts_with("tv.camera.target: "));
    let blank = camera("\" \"", 200, 0, finn);
    assert!(refused(&blank).message.contains("cannot be empty"));
}

#[test]
fn a_camera_that_names_no_element_of_the_fixtures_own_stage_is_refused_but_a_live_stage_is_not_judged()
 {
    let missing = text(
        "\"camera\": {\"target\": \"ghost\", \"zoom_percent\": 200, \"duration_ms\": 0},",
        "",
        "",
        "[]",
    );
    assert_eq!(
        refused(&missing).message,
        "tv.camera.target: the camera looks at 'ghost', which is not on the stage"
    );
    let live = text(
        "\"camera\": {\"target\": \"ghost\", \"zoom_percent\": 200, \"duration_ms\": 0},",
        "",
        "",
        "\"live\"",
    );
    Fixture::parse(&live).expect("the canvas decides at run time");
}

#[test]
fn music_louder_than_a_hundred_percent_is_refused() {
    let source = text(
        "",
        "",
        "\"music\": {\"media\": \"m.ogg\", \"target\": \"both\", \"volume\": 255, \"looping\": false},",
        "[]",
    );
    let error = refused(&source);
    assert_eq!(error.line, line_of(&source, "volume"));
    assert_eq!(
        error.message,
        "music.volume: a volume is from 0 to 100 percent, not 255"
    );
}

#[test]
fn a_party_member_needs_a_maximum_of_at_least_one() {
    let source = text(
        "\"party\": [\n      {\"name\": \"A\", \"hit_points\": 1, \"maximum_hit_points\": 0, \"color\": null}\n    ],",
        "",
        "",
        "[]",
    );
    let error = refused(&source);
    assert_eq!(error.line, line_of(&source, "maximum_hit_points"));
    assert!(
        error
            .message
            .starts_with("tv.party[0].maximum_hit_points: ")
    );
}

#[test]
fn a_figure_with_a_turn_or_a_pivot_out_of_range_is_refused_at_its_line() {
    let element = |extra: &str| {
        text(
            "",
            "",
            "",
            &format!(
                "[\n    {{\"id\": \"a\", \"kind\": \"k\", \"label\": \"A\", \"x\": 1, \"y\": 1, \"color\": null, \"shape\": \"circle\", \"width_units\": 1, \"height_units\": 1}},\n    {{\"id\": \"d\", \"kind\": \"door\", \"label\": \"D\", \"x\": 1, \"y\": 1, \"color\": null, \"shape\": \"rect\", \"width_units\": 1, \"height_units\": 1,\n      {extra}}}\n  ]"
            ),
        )
    };
    let source =
        element("\"rotation_degrees\": 400, \"pivot\": {\"x_percent\": 255, \"y_percent\": 0}");
    let error = refused(&source);
    assert_eq!(error.line, line_of(&source, "rotation_degrees"));
    assert!(
        error
            .message
            .starts_with("tv.elements[1].rotation_degrees: ")
    );
    let pivot = element("\"pivot\": {\"x_percent\": 255, \"y_percent\": 0}");
    assert!(
        refused(&pivot)
            .message
            .starts_with("tv.elements[1].pivot.x_percent: ")
    );
}

#[test]
fn the_mistake_nearest_the_top_is_the_one_reported() {
    let source = text(
        "\"card\": {\"media\": \"a.png\", \"caption\": null, \"rotation_degrees\": 400},",
        "",
        "\"music\": {\"media\": \"m.ogg\", \"target\": \"both\", \"volume\": 255, \"looping\": false},",
        "[]",
    );
    assert!(
        refused(&source)
            .message
            .starts_with("tv.card.rotation_degrees")
    );
}

#[test]
fn braces_quotes_and_escapes_in_earlier_text_do_not_throw_the_line_off() {
    let source = text(
        "\"caption\": {\"speaker\": \"a \\\"}\\\" [\", \"text\": \"{ \\\\ ]\"},\n    \"card\": {\"media\": \"a.png\", \"caption\": \"}{\", \"rotation_degrees\": 361},",
        "",
        "",
        "[]",
    );
    let error = refused(&source);
    assert_eq!(error.line, line_of(&source, "361"));
    assert_eq!(
        error.column,
        source
            .lines()
            .nth(error.line - 1)
            .unwrap()
            .find("361")
            .unwrap()
            + 1
    );
}

#[test]
fn a_fixture_that_cannot_be_read_at_all_still_reports_where_serde_stopped() {
    let error = refused("{\n  \"name\": 5\n}");
    assert_eq!(error.line, 2);
}

#[test]
fn the_numbers_only_the_host_sets_are_refused_in_a_file() {
    let live_camera = |extra: &str| {
        text(
            &format!(
                "\"camera\": {{\"target\": \"finn\", \"zoom_percent\": 200, \"duration_ms\": 1500{extra}}},"
            ),
            "",
            "",
            "\"live\"",
        )
    };
    for field in ["serial", "began_ms", "elapsed_ms"] {
        let source = live_camera(&format!(", \"{field}\": 3"));
        let error = refused(&source);
        assert_eq!(error.line, line_of(&source, field), "{field}");
        assert_eq!(
            error.message,
            format!(
                "tv.camera.{field}: '{field}' is set by the host; leave it out of a scene file"
            )
        );
    }
    Fixture::parse(&live_camera(", \"serial\": 0")).expect("zero is the same as left out");
    let slide = card(
        0,
        "{\"from_percent\": 1, \"to_percent\": 2, \"duration_ms\": 5, \"elapsed_ms\": 9999}",
    );
    assert!(
        refused(&slide)
            .message
            .starts_with("tv.card.slide.elapsed_ms: ")
    );
}

#[test]
fn a_caption_a_name_or_a_label_a_file_could_not_set_by_the_command_line_is_refused() {
    let long = "x".repeat(crate::limits::MAXIMUM_CAPTION_CHARACTERS + 1);
    let caption = text(
        &format!("\"caption\": {{\"speaker\": null, \"text\": \"{long}\"}},"),
        "",
        "",
        "[]",
    );
    let error = refused(&caption);
    assert!(
        error.message.starts_with("tv.caption.text: "),
        "{}",
        error.message
    );
    assert_eq!(error.line, line_of(&caption, "caption"));
    let blank = text(
        "\"caption\": {\"speaker\": \" \", \"text\": \"hi\"},",
        "",
        "",
        "[]",
    );
    assert!(refused(&blank).message.starts_with("tv.caption.speaker: "));
    let nameless = text(
        "\"party\": [{\"name\": \"\", \"hit_points\": 1, \"maximum_hit_points\": 1, \"color\": null}],",
        "",
        "",
        "[]",
    );
    assert!(refused(&nameless).message.starts_with("tv.party[0].name: "));
    let condition = text(
        "\"party\": [{\"name\": \"A\", \"hit_points\": 1, \"maximum_hit_points\": 1, \"conditions\": [\"ok\", \"\"], \"color\": null}],",
        "",
        "",
        "[]",
    );
    assert!(
        refused(&condition)
            .message
            .starts_with("tv.party[0].conditions[1]: ")
    );
    let label = text(
        "",
        ", \"suggested\": [{\"label\": \"\", \"explain\": null}]",
        "",
        "[]",
    );
    assert!(
        refused(&label)
            .message
            .starts_with("phone.suggested[0].label: ")
    );
    let group = text(
        "",
        ", \"groups\": [{\"name\": \" \", \"choices\": []}]",
        "",
        "[]",
    );
    assert!(
        refused(&group)
            .message
            .starts_with("phone.groups[0].name: ")
    );
    let reason = text(
        "",
        ", \"unavailable\": [{\"label\": \"Door\", \"reason\": \"\"}]",
        "",
        "[]",
    );
    assert!(
        refused(&reason)
            .message
            .starts_with("phone.unavailable[0].reason: ")
    );
    let media = text(
        "\"card\": {\"media\": \" \", \"caption\": null},",
        "",
        "",
        "[]",
    );
    assert!(refused(&media).message.starts_with("tv.card.media: "));
}

#[test]
fn a_list_past_the_count_the_overlay_allows_is_refused_and_one_at_it_is_not() {
    let choices = |count: usize| {
        (0..count)
            .map(|index| format!("{{\"label\": \"c{index}\", \"explain\": null}}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let at = text(
        "",
        &format!(
            ", \"suggested\": [{}]",
            choices(crate::limits::MAXIMUM_CHOICES_PER_LIST)
        ),
        "",
        "[]",
    );
    Fixture::parse(&at).expect("at the limit");
    let over = text(
        "",
        &format!(
            ", \"suggested\": [{}]",
            choices(crate::limits::MAXIMUM_CHOICES_PER_LIST + 1)
        ),
        "",
        "[]",
    );
    assert_eq!(
        refused(&over).message,
        format!(
            "phone.suggested: a list holds at most {} choices",
            crate::limits::MAXIMUM_CHOICES_PER_LIST
        )
    );
    let member = |index: usize| {
        format!(
            "{{\"name\": \"m{index}\", \"hit_points\": 1, \"maximum_hit_points\": 1, \"color\": null}}"
        )
    };
    let party = (0..=crate::limits::MAXIMUM_PARTY_MEMBERS)
        .map(member)
        .collect::<Vec<_>>()
        .join(",");
    let crowded = text(&format!("\"party\": [{party}],"), "", "", "[]");
    assert!(refused(&crowded).message.starts_with("tv.party: "));
}

#[test]
fn a_column_counts_characters_for_both_kinds_of_mistake() {
    // Two multi-byte characters before the value: serde counts bytes, the check counts characters.
    let range = text(
        "\"caption\": {\"speaker\": \"\u{e9}\u{e9}\", \"text\": \"hi\"}, \"card\": {\"media\": \"a.png\", \"caption\": null, \"rotation_degrees\": 400},",
        "",
        "",
        "[]",
    );
    let error = refused(&range);
    let line = range.lines().nth(error.line - 1).expect("a line");
    let characters_before = line[..line.find("400").expect("the value")].chars().count();
    assert_eq!(error.column, characters_before + 1);
    let broken = "{\"name\": \"\u{e9}\u{e9}\u{e9}\", \"tv\": 5}";
    let error = refused(broken);
    let characters_before = broken[..broken.find('5').expect("the value")]
        .chars()
        .count();
    assert_eq!(error.line, 1);
    assert!(
        error.column >= characters_before && error.column <= characters_before + 2,
        "{} against {characters_before}",
        error.column
    );
    assert!(!error.message.contains(" at line "), "{}", error.message);
}
