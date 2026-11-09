mod shipped;

use crate::fixture::{
    ElementShape, ElementSource, Fixture, FixtureTitle, LiveMarker, PlateKind, StageElement,
    StagePivot, Status,
};
use crate::fixtures::assert_keys_are_serde_names;

const MINIMAL: &str = r#"{
    "name": "Minimal",
    "tv": {
        "place": "Seed Market",
        "status": {"kind": "ok"},
        "elements": "live"
    },
    "phone": {"hero": "Pip", "status": {"kind": "ok"}}
}"#;

#[test]
fn a_minimal_fixture_reads_and_fills_the_optional_parts_with_nothing() {
    let fixture = Fixture::parse(MINIMAL).expect("parses");
    assert_eq!(fixture.name, "Minimal");
    assert!(fixture.tv.party.is_empty());
    assert!(fixture.tv.caption.is_none());
    assert!(fixture.phone.suggested.is_empty());
    assert!(matches!(fixture.tv.elements, ElementSource::Live(_)));
}

#[test]
fn elements_may_be_listed_inline_and_extra_keys_on_an_element_are_ignored() {
    let text = MINIMAL.replace(
        r#""elements": "live""#,
        r##""elements": [
            {"id": "pip", "kind": "character", "label": "Pip", "x": 20, "y": 30,
             "color": "#4f8cff", "shape": "circle", "width_units": 6, "height_units": 6,
             "fields": {"name": "Pip"}, "problem": null}
        ]"##,
    );
    let fixture = Fixture::parse(&text).expect("parses");
    let ElementSource::Inline(elements) = fixture.tv.elements else {
        panic!("expected inline elements");
    };
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0].label, "Pip");
}

#[test]
fn a_word_other_than_live_where_live_is_expected_is_refused() {
    let text = MINIMAL.replace(r#""elements": "live""#, r#""elements": "later""#);
    assert!(Fixture::parse(&text).is_err());
}

#[test]
fn a_misspelt_key_is_refused_with_the_line_and_column_of_the_mistake() {
    let text = MINIMAL.replace(r#""place": "Seed Market""#, r#""plaace": "Seed Market""#);
    let error = Fixture::parse(&text).expect_err("refused");
    assert_eq!(error.line, 4);
    assert!(error.message.contains("plaace"), "got: {}", error.message);
}

#[test]
fn a_status_is_a_tagged_object_and_can_carry_who_is_waited_for() {
    let text = MINIMAL.replace(
        r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}"#,
        r#""phone": {"hero": "Pip", "status": {"kind": "waiting_for", "who": "Pip", "what": "to answer a reaction"}}"#,
    );
    let fixture = Fixture::parse(&text).expect("parses");
    assert_eq!(
        fixture.phone.status,
        Status::WaitingFor {
            who: "Pip".into(),
            what: "to answer a reaction".into()
        }
    );
}

#[test]
fn an_unknown_status_is_refused() {
    let text = MINIMAL.replace(
        r#""status": {"kind": "ok"}"#,
        r#""status": {"kind": "sleepy"}"#,
    );
    assert!(Fixture::parse(&text).is_err());
}

#[test]
fn a_missing_required_key_is_refused_by_name() {
    let text = MINIMAL.replace(r#""hero": "Pip", "#, "");
    let error = Fixture::parse(&text).expect_err("refused");
    assert!(error.message.contains("hero"), "got: {}", error.message);
}

fn inline_element(extra: &str) -> StageElement {
    let text = MINIMAL.replace(
        r#""elements": "live""#,
        &format!(
            r##""elements": [
                {{"id": "door", "kind": "set_piece", "label": "Door", "x": 20, "y": 30,
                  "color": null, "shape": "rect", "width_units": 4, "height_units": 8{extra}}}
            ]"##
        ),
    );
    let ElementSource::Inline(mut elements) = Fixture::parse(&text).expect("parses").tv.elements
    else {
        panic!("expected inline elements");
    };
    elements.remove(0)
}

#[test]
fn an_element_without_a_rotation_or_pivot_is_unturned_about_its_centre() {
    let element = inline_element("");
    assert_eq!(element.rotation_degrees, 0);
    assert_eq!(element.pivot, StagePivot::default());
    assert_eq!((element.pivot.x_percent, element.pivot.y_percent), (50, 50));
}

#[test]
fn an_element_with_a_rotation_and_a_pivot_keeps_both() {
    let element =
        inline_element(r#", "rotation_degrees": 270, "pivot": {"x_percent": 0, "y_percent": 50}"#);
    assert_eq!(element.rotation_degrees, 270);
    assert_eq!(
        element.pivot,
        StagePivot {
            x_percent: 0,
            y_percent: 50
        }
    );
}

#[test]
fn a_pivot_with_one_coordinate_takes_the_centre_for_the_other_and_a_misspelt_key_is_refused() {
    let element = inline_element(r#", "pivot": {"x_percent": 10}"#);
    assert_eq!(
        element.pivot,
        StagePivot {
            x_percent: 10,
            y_percent: 50
        }
    );
    let text = MINIMAL.replace(
        r#""elements": "live""#,
        r##""elements": [{"id": "d", "kind": "k", "label": "D", "x": 1, "y": 1, "color": null,
            "shape": "rect", "width_units": 1, "height_units": 1, "pivot": {"x": 10}}]"##,
    );
    assert!(Fixture::parse(&text).is_err());
}

#[test]
fn a_fixture_title_is_the_scenario_name_and_none_for_a_file_that_is_not_a_scenario() {
    let read = FixtureTitle::read("minimal.json".to_owned(), MINIMAL);
    assert_eq!(read.file, "minimal.json");
    assert_eq!(read.title.as_deref(), Some("Minimal"));
    let broken = FixtureTitle::read("broken.json".to_owned(), r#"{"name": "Half""#);
    assert_eq!(broken.file, "broken.json");
    assert_eq!(broken.title, None);
}

#[test]
fn the_keys_of_an_element_shape_and_the_live_marker_are_their_serde_names() {
    assert_keys_are_serde_names(
        &ElementShape::ALL,
        ElementShape::key,
        ElementShape::from_key,
    );
    assert_keys_are_serde_names(&LiveMarker::ALL, LiveMarker::key, LiveMarker::from_key);
}

#[test]
fn a_backdrop_layout_counts_only_on_a_room_plate_the_host_put_up_with_its_own_scan() {
    let mut scene = Fixture::parse(include_str!(
        "../../../assets/playground/fixtures/ui_roleplay_thought.json"
    ))
    .expect("the roleplay fixture reads");
    let backdrop = scene.tv.backdrop.as_mut().expect("a backdrop");
    let picture = df_scene_layout::PictureBinding {
        name: backdrop.media.clone(),
        sha256: "ab".repeat(32),
        width_px: 1672,
        height_px: 941,
    };
    backdrop.layout = Some(df_scene_layout::SceneLayout::new(
        picture,
        "sam-3.1",
        "person",
        Vec::new(),
    ));
    assert!(scene.tv.backdrop_layout().is_some(), "the room's own scan");
    // A close-up, or a room shown with the cast before it, has nobody where that layout's figures
    // stood; the host says which it is, and the picture's name is not read.
    for plate in [PlateKind::CloseUp, PlateKind::RoomKeptCast] {
        scene.tv.backdrop.as_mut().expect("a backdrop").plate = plate;
        assert!(scene.tv.backdrop_layout().is_none(), "{plate:?}");
        assert!(
            scene.tv.cast_layout().is_some(),
            "the cast keeps its names and faces"
        );
    }
}

#[test]
fn a_plate_kinds_key_is_its_serde_name_and_a_backdrop_is_a_room_plate_unless_told() {
    for plate in [PlateKind::Room, PlateKind::RoomKeptCast, PlateKind::CloseUp] {
        let serde_name = serde_json::to_string(&plate).expect("a plate kind writes");
        assert_eq!(serde_name, format!("\"{}\"", plate.key()));
    }
    let scene = Fixture::parse(include_str!(
        "../../../assets/playground/fixtures/ui_roleplay_thought.json"
    ))
    .expect("the roleplay fixture reads");
    assert_eq!(
        scene.tv.backdrop.expect("a backdrop").plate,
        PlateKind::Room
    );
}
