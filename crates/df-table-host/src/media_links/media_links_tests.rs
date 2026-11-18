//! Which names of an answer a phone is given links to: only what the host put in picture fields.

use serde_json::json;

use super::named_media;

/// The names `value`'s picture fields give, in name order (an object's fields have no order).
fn named(value: &serde_json::Value) -> Vec<String> {
    let mut found = Vec::new();
    named_media(value, &mut found);
    found.sort();
    found
}

#[test]
fn a_note_that_spells_another_heros_file_name_gets_no_link() {
    let pages = json!({
        "kept": [
            {"id": "n1", "kind": "written", "seq": 1, "title": "Note", "text": "ref-6.png"},
            {"id": "n2", "kind": "snapped", "seq": 2, "title": "Snap", "text": "",
             "picture": "plate-quay.webp"}
        ],
        "people": [{"id": "dain", "name": "head-6.png", "portrait": "dain-rusk.webp"}],
        "places": [{"id": "quay", "name": "Quay", "plate": "quay-in-the-rain.webp"}],
        "insert": {"picture": "close-up.png", "caption": "ready-6.png"}
    });
    assert_eq!(
        named(&pages),
        [
            "close-up.png",
            "dain-rusk.webp",
            "plate-quay.webp",
            "quay-in-the-rain.webp"
        ],
        "only picture fields are read: a note, a name or a caption is never signed"
    );
}

#[test]
fn a_pack_item_and_a_frames_image_and_sounds_are_read_through_their_fields() {
    let answer = json!({
        "pack": {"headshot": {"status": "ready", "picture": "head-7.png"},
                 "sheet": {"status": "pending"}},
        "scene": {
            "phone_image": {"set": {"media": "letter.png", "caption": "a.png"}},
            "music": {"set": {"media": "rain.ogg", "target": "both"}},
            "cues": [{"id": 1, "media": "door.wav", "text": "bell.wav"}]
        }
    });
    assert_eq!(
        named(&answer),
        ["door.wav", "head-7.png", "letter.png", "rain.ogg"]
    );
}

#[test]
fn only_a_phones_own_answers_are_signed_and_never_a_fixture() {
    for own in [
        "/play",
        "/play/seat/view",
        "/play/seat/pages",
        "/play/look",
        "/playground/frame",
    ] {
        assert!(super::signs_answers_of(own), "{own}");
    }
    for other in [
        "/playground/fixtures/roam.json",
        "/playground/fixtures.json",
        "/playground/theme.css",
        "/player",
    ] {
        assert!(!super::signs_answers_of(other), "{other}");
    }
}
