use df_table::canvas::Canvas;
use df_table::kind::Shape;
use df_table::testkit::{CHARACTER_KIND, MARKER_KIND, RecordingSink, registry_from};
use serde_json::json;

use crate::event_log::LoggedLine;
use crate::render::{MAXIMUM_LABEL_CHARACTERS, escape, page, shorten_label};
use crate::view::{CanvasView, ElementView};
use crate::wire::EffectRequest;

fn spawn(canvas: &mut Canvas, request: serde_json::Value) {
    let request: EffectRequest = serde_json::from_value(request).expect("parses");
    let effect = request.into_effect(canvas).expect("converts");
    canvas
        .apply(effect, &RecordingSink::default())
        .expect("applies");
}

fn character_canvas() -> Canvas {
    let mut canvas = Canvas::new(registry_from(&[CHARACTER_KIND, MARKER_KIND]));
    spawn(
        &mut canvas,
        json!({"op": "spawn", "id": "pip", "kind": "character", "x": 20, "y": 20, "fields": {"name": "Pip"}}),
    );
    canvas
}

fn line(sequence: u64, text: &str) -> LoggedLine {
    LoggedLine {
        sequence,
        text: text.to_owned(),
    }
}

#[test]
fn escape_covers_every_character_that_can_break_out_of_text_or_an_attribute() {
    assert_eq!(
        escape(r#"<a href="x">'&'</a>"#),
        "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
    );
    assert_eq!(escape("plain"), "plain");
}

#[test]
fn a_label_at_the_limit_is_kept_and_a_longer_one_is_cut_to_the_limit_with_an_ellipsis() {
    let exact = "a".repeat(MAXIMUM_LABEL_CHARACTERS);
    assert_eq!(shorten_label(&exact), exact);
    let longer = "a".repeat(MAXIMUM_LABEL_CHARACTERS + 1);
    let cut = shorten_label(&longer);
    assert_eq!(cut.chars().count(), MAXIMUM_LABEL_CHARACTERS);
    assert!(cut.ends_with('\u{2026}'));
}

#[test]
fn shortening_counts_characters_not_bytes() {
    let wide = "\u{e9}".repeat(MAXIMUM_LABEL_CHARACTERS + 5);
    assert_eq!(
        shorten_label(&wide).chars().count(),
        MAXIMUM_LABEL_CHARACTERS
    );
}

#[test]
fn the_page_refreshes_with_a_meta_tag_and_contains_no_script() {
    let html = page(&CanvasView::from_canvas(&character_canvas()), &[]);
    assert!(html.contains(r#"<meta http-equiv="refresh" content="1">"#));
    assert!(!html.to_ascii_lowercase().contains("<script"));
}

#[test]
fn an_element_is_drawn_with_its_kind_shape_color_and_label() {
    let html = page(&CanvasView::from_canvas(&character_canvas()), &[]);
    assert!(html.contains("<circle"));
    assert!(html.contains("#4f8cff"));
    assert!(html.contains(">Pip</text>"));
    assert!(html.contains("<title>pip (character)</title>"));
}

#[test]
fn a_rectangular_kind_is_drawn_as_a_rect_centred_on_the_element() {
    let mut view = CanvasView::from_canvas(&character_canvas());
    view.elements[0].shape = Shape::Rect;
    view.elements[0].width_units = 10.0;
    view.elements[0].height_units = 4.0;
    let html = page(&view, &[]);
    assert!(html.contains(r#"<rect x="15.00" y="18.00" width="10.00" height="4.00""#));
}

#[test]
fn an_element_whose_kind_was_removed_is_drawn_as_a_flagged_square_labelled_by_its_id() {
    let mut canvas = character_canvas();
    canvas.replace_registry(registry_from(&[MARKER_KIND]), &RecordingSink::default());
    let html = page(&CanvasView::from_canvas(&canvas), &[]);
    assert!(html.contains("<rect"));
    assert!(html.contains("stroke-dasharray"));
    assert!(html.contains(">pip</text>"));
    assert!(html.contains("class=\"problem\""));
}

#[test]
fn a_kind_with_an_empty_label_falls_back_to_the_id() {
    let mut canvas = Canvas::new(registry_from(&[MARKER_KIND]));
    spawn(
        &mut canvas,
        json!({"op": "spawn", "id": "m1", "kind": "marker"}),
    );
    let html = page(&CanvasView::from_canvas(&canvas), &[]);
    assert!(html.contains(">m1</text>"));
}

fn hostile_element() -> ElementView {
    ElementView {
        id: "<b>id".into(),
        kind: "k\"ind".into(),
        x: 10.0,
        y: 10.0,
        label: "<img src=x onerror=1>".into(),
        color: Some("\"><script>".into()),
        shape: Shape::Circle,
        width_units: 4.0,
        height_units: 4.0,
        fields: serde_json::Map::from_iter([("note".to_owned(), json!("<i>"))]),
        problem: Some("<problem>".into()),
    }
}

#[test]
fn every_piece_of_text_from_the_canvas_and_the_log_is_escaped() {
    let view = CanvasView {
        revision: 1,
        elements: vec![hostile_element()],
        kinds: Vec::new(),
    };
    let html = page(&view, &[line(1, "CANVAS <script>alert(1)</script>")]);
    assert!(!html.contains("<img"));
    assert!(!html.contains("<script"));
    assert!(!html.contains("<b>id"));
    assert!(!html.contains("<problem>"));
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
}

#[test]
fn the_log_is_drawn_newest_first_from_lines_given_oldest_first() {
    let view = CanvasView::from_canvas(&character_canvas());
    let html = page(&view, &[line(1, "older-entry"), line(2, "newer-entry")]);
    let newer = html.find("newer-entry").expect("shows the newer line");
    let older = html.find("older-entry").expect("shows the older line");
    assert!(newer < older);
}

#[test]
fn an_empty_canvas_and_log_tell_the_reader_what_to_do_or_that_nothing_happened() {
    let canvas = Canvas::new(registry_from(&[]));
    let html = page(&CanvasView::from_canvas(&canvas), &[]);
    assert!(html.contains("Nothing here yet"));
    assert!(html.contains("No kinds are loaded"));
    assert!(html.contains("Nothing has happened yet."));
}

#[test]
fn the_header_states_the_revision_and_the_counts() {
    let html = page(&CanvasView::from_canvas(&character_canvas()), &[]);
    assert!(html.contains("revision 1 with 1 elements and 2 kinds"));
}

#[test]
fn a_label_near_the_bottom_edge_stays_on_the_canvas() {
    let mut view = CanvasView::from_canvas(&character_canvas());
    view.elements[0].y = 56.0;
    let html = page(&view, &[]);
    assert!(html.contains(r#"y="55.75""#));
}
