//! The place label's words and place.

use crate::image_layout::CharacterMarker;
use crate::kit::Fill;
use crate::kit::kit_tests::assert_css_words_are_serde_names;
use crate::kit::place_label::{PlaceLabel, PlaceLook};

#[test]
fn a_label_reads_as_parchment_and_holds_its_place_on_the_screen() {
    let label: PlaceLabel =
        serde_json::from_str(r#"{"text":" Greyhaven ","x_permille":1400,"y_permille":620}"#)
            .expect("a label reads");
    assert_eq!(label.look, PlaceLook::Parchment);
    assert_eq!(label.text(), Some("Greyhaven"));
    assert_eq!(label.held_permille(), (1000, 620));
    assert_eq!(label.fill(), Fill::Solid);
}

#[test]
fn a_marker_without_a_layout_is_not_placed() {
    let marker: CharacterMarker =
        serde_json::from_str(r#"{"region":"ruins","text":"Ruins"}"#).expect("a marker reads");
    assert_eq!(PlaceLabel::from_marker(&marker, None), None);
    assert_css_words_are_serde_names(
        &[PlaceLook::Parchment, PlaceLook::Chip, PlaceLook::Current],
        PlaceLook::key,
    );
}
