//! The adventure card's data and its live edits: defaults, bounds, all-or-nothing edits, numbering
//! and what the audit line carries.

use super::adventure::*;
use crate::home::glyph::{HomeGlyph, HomeIcon};
use crate::limits::{MAXIMUM_FACT_ROWS, MAXIMUM_NAME_CHARACTERS};
use crate::overlay::{Edit, Glyph};

fn row(label: &str, value: &str) -> FactRow {
    FactRow {
        icon: HomeIcon::Page(HomeGlyph::Mountain),
        label: label.to_owned(),
        value: value.to_owned(),
    }
}

fn card() -> AdventureData {
    AdventureData {
        title: "The Ashen Citadel".to_owned(),
        picture: Some("adventure-citadel-waterfall.webp".to_owned()),
        facts: vec![
            row("Region", "Veylmar Expanse"),
            row("Biome", "Ruined Highlands"),
        ],
        ..AdventureData::default()
    }
}

fn set_title(text: &str) -> AdventureEdit {
    AdventureEdit::Set {
        heading: None,
        picture: Edit::Keep,
        title: Some(text.to_owned()),
        blurb: None,
        launch_label: None,
        enabled: None,
    }
}

#[test]
fn an_empty_card_reads_with_the_default_heading_and_an_enabled_launch_button() {
    let data: AdventureData = serde_json::from_str("{}").expect("an empty card reads");
    assert_eq!(data.heading, DEFAULT_HEADING);
    assert_eq!(data.launch_label, DEFAULT_LAUNCH_LABEL);
    assert!(data.enabled);
    assert!(data.facts.is_empty());
    assert_eq!(data.check(), Ok(()));
}

#[test]
fn a_card_survives_a_round_trip_and_refuses_a_field_it_does_not_have() {
    let data = card();
    let text = serde_json::to_string(&data).expect("writes");
    assert_eq!(
        serde_json::from_str::<AdventureData>(&text).expect("reads"),
        data
    );
    assert!(serde_json::from_str::<AdventureData>(r#"{"titel":"x"}"#).is_err());
    assert!(serde_json::from_str::<FactRow>(r#"{"label":"a","value":"b","size":1}"#).is_err());
}

#[test]
fn the_card_draws_the_first_three_fact_rows_and_keeps_the_rest_in_the_data() {
    let mut data = card();
    assert_eq!(data.shown_facts().len(), 2);
    data.facts = vec![row("a", "b"); 5];
    assert_eq!(data.shown_facts().len(), FACTS_SHOWN);
    assert_eq!(data.facts.len(), 5);
    assert_eq!(LAUNCH_FOCUS_ID, "adventure:launch");
}

#[test]
fn a_fact_row_without_an_icon_takes_the_default_and_a_name_that_is_no_icon_is_refused() {
    let row: FactRow = serde_json::from_str(r#"{"label":"Region","value":"X"}"#).expect("reads");
    assert_eq!(row.icon, DEFAULT_FACT_ICON);
    let overlay: FactRow =
        serde_json::from_str(r#"{"icon":"sword","label":"a","value":"b"}"#).expect("reads");
    assert_eq!(overlay.icon, HomeIcon::Overlay(Glyph::Sword));
    assert!(serde_json::from_str::<FactRow>(r#"{"icon":"nope","label":"a","value":"b"}"#).is_err());
}

#[test]
fn the_bounds_are_checked_at_the_edge() {
    let mut data = card();
    data.title = "x".repeat(MAXIMUM_NAME_CHARACTERS + 1);
    assert_eq!(
        data.check(),
        Err(AdventureError::TooLong {
            what: "the title",
            limit: MAXIMUM_NAME_CHARACTERS
        })
    );
    let mut data = card();
    data.picture = Some("../secret.webp".to_owned());
    assert_eq!(
        data.check(),
        Err(AdventureError::InvalidPicture("../secret.webp".to_owned()))
    );
    data.picture = Some("sound.wav".to_owned());
    assert!(matches!(
        data.check(),
        Err(AdventureError::InvalidPicture(_))
    ));
    let mut data = card();
    data.facts = vec![row("a", "b"); MAXIMUM_FACT_ROWS + 1];
    assert_eq!(data.check(), Err(AdventureError::TooManyFacts));
    data.facts.pop();
    assert_eq!(data.check(), Ok(()));
}

#[test]
fn a_set_makes_the_card_when_there_is_none_and_changes_only_what_it_names() {
    let mut slot = None;
    set_title("Ember Roads")
        .apply(&mut slot)
        .expect("makes the card");
    let made = slot.clone().expect("a card");
    assert_eq!(made.title, "Ember Roads");
    assert_eq!(made.heading, DEFAULT_HEADING);
    AdventureEdit::Set {
        heading: Some("Next up".to_owned()),
        picture: Edit::Set("adventure-citadel-waterfall.webp".to_owned()),
        title: None,
        blurb: Some("A blurb.".to_owned()),
        launch_label: Some("Start".to_owned()),
        enabled: Some(false),
    }
    .apply(&mut slot)
    .expect("changes");
    let changed = slot.clone().expect("a card");
    assert_eq!(changed.title, "Ember Roads");
    assert_eq!(changed.heading, "Next up");
    assert_eq!(
        changed.picture.as_deref(),
        Some("adventure-citadel-waterfall.webp")
    );
    assert_eq!(changed.blurb, "A blurb.");
    assert_eq!(changed.launch_label, "Start");
    assert!(!changed.enabled);
    AdventureEdit::Set {
        heading: None,
        picture: Edit::Clear,
        title: None,
        blurb: None,
        launch_label: None,
        enabled: None,
    }
    .apply(&mut slot)
    .expect("takes the picture away");
    assert_eq!(slot.expect("a card").picture, None);
}

#[test]
fn a_set_that_names_nothing_is_refused() {
    let mut slot = Some(card());
    let edit = AdventureEdit::Set {
        heading: None,
        picture: Edit::Keep,
        title: None,
        blurb: None,
        launch_label: None,
        enabled: None,
    };
    assert_eq!(edit.apply(&mut slot), Err(AdventureError::NothingToChange));
    assert_eq!(slot, Some(card()));
}

#[test]
fn a_refused_edit_leaves_the_card_exactly_as_it_was() {
    let mut slot = Some(card());
    let too_long = set_title(&"x".repeat(MAXIMUM_NAME_CHARACTERS + 1));
    assert!(too_long.apply(&mut slot).is_err());
    let bad_picture = AdventureEdit::Set {
        heading: Some("Changed".to_owned()),
        picture: Edit::Set("a b.webp".to_owned()),
        title: None,
        blurb: None,
        launch_label: None,
        enabled: None,
    };
    assert!(bad_picture.apply(&mut slot).is_err());
    assert_eq!(slot, Some(card()));
}

#[test]
fn fact_rows_are_numbered_from_one_and_every_part_can_change() {
    let mut slot = Some(card());
    AdventureEdit::FactSet {
        number: 2,
        icon: Some(HomeIcon::Overlay(Glyph::Crown)),
        label: Some("Threat Level".to_owned()),
        value: Some("High".to_owned()),
    }
    .apply(&mut slot)
    .expect("changes row 2");
    let facts = slot.clone().expect("a card").facts;
    assert_eq!(facts[1].label, "Threat Level");
    assert_eq!(facts[1].value, "High");
    assert_eq!(facts[1].icon, HomeIcon::Overlay(Glyph::Crown));
    assert_eq!(facts[0].label, "Region");
    for number in [0, 3] {
        let edit = AdventureEdit::FactSet {
            number,
            icon: None,
            label: Some("x".to_owned()),
            value: None,
        };
        assert_eq!(
            edit.apply(&mut slot),
            Err(AdventureError::InvalidFactNumber { number, count: 2 })
        );
    }
    let nothing = AdventureEdit::FactSet {
        number: 1,
        icon: None,
        label: None,
        value: None,
    };
    assert_eq!(
        nothing.apply(&mut slot),
        Err(AdventureError::NothingToChange)
    );
}

#[test]
fn rows_are_added_at_the_end_removed_by_number_and_cleared() {
    let mut slot = Some(card());
    AdventureEdit::FactAdd {
        icon: None,
        label: "Est. Duration".to_owned(),
        value: "2 - 3 hours".to_owned(),
    }
    .apply(&mut slot)
    .expect("adds");
    let facts = slot.clone().expect("a card").facts;
    assert_eq!(facts.len(), 3);
    assert_eq!(facts[2].icon, DEFAULT_FACT_ICON);
    AdventureEdit::FactRemove { number: 1 }
        .apply(&mut slot)
        .expect("removes row 1");
    assert_eq!(slot.clone().expect("a card").facts[0].label, "Biome");
    assert_eq!(
        AdventureEdit::FactRemove { number: 9 }.apply(&mut slot),
        Err(AdventureError::InvalidFactNumber {
            number: 9,
            count: 2
        })
    );
    AdventureEdit::FactsClear {}
        .apply(&mut slot)
        .expect("clears");
    assert!(slot.clone().expect("a card").facts.is_empty());
}

#[test]
fn an_empty_card_has_no_fact_to_change_and_clear_takes_the_card_away() {
    let mut slot = None;
    assert_eq!(
        AdventureEdit::FactRemove { number: 1 }.apply(&mut slot),
        Err(AdventureError::InvalidFactNumber {
            number: 1,
            count: 0
        })
    );
    assert_eq!(slot, None);
    let mut slot = Some(card());
    AdventureEdit::Clear {}.apply(&mut slot).expect("clears");
    assert_eq!(slot, None);
}

#[test]
fn adding_past_the_limit_is_refused_and_changes_nothing() {
    let mut data = card();
    data.facts = vec![row("a", "b"); MAXIMUM_FACT_ROWS];
    let mut slot = Some(data.clone());
    let add = AdventureEdit::FactAdd {
        icon: None,
        label: "a".to_owned(),
        value: "b".to_owned(),
    };
    assert_eq!(add.apply(&mut slot), Err(AdventureError::TooManyFacts));
    assert_eq!(slot, Some(data));
}

#[test]
fn the_audit_line_carries_the_field_numbers_and_flags_but_never_the_text() {
    let (field, pairs) = AdventureEdit::Set {
        heading: Some("Secret heading".to_owned()),
        picture: Edit::Set("adventure-citadel-waterfall.webp".to_owned()),
        title: Some("Secret title".to_owned()),
        blurb: None,
        launch_label: None,
        enabled: Some(true),
    }
    .audit_fields();
    assert_eq!(field, "set");
    assert!(pairs.contains(&("heading_chars", "14".to_owned())));
    assert!(pairs.contains(&("picture", "set".to_owned())));
    assert!(pairs.contains(&("enabled", "true".to_owned())));
    for (_, value) in &pairs {
        assert!(!value.contains("Secret"));
    }
    let (field, pairs) = AdventureEdit::FactSet {
        number: 3,
        icon: Some(HomeIcon::Page(HomeGlyph::Skull)),
        label: Some("Secret".to_owned()),
        value: None,
    }
    .audit_fields();
    assert_eq!(field, "fact");
    assert!(pairs.contains(&("fact", "3".to_owned())));
    assert!(pairs.contains(&("icon", "skull".to_owned())));
    assert!(pairs.contains(&("label_chars", "6".to_owned())));
}

#[test]
fn an_edit_names_the_pictures_it_shows_and_only_those() {
    let shows = AdventureEdit::Set {
        heading: None,
        picture: Edit::Set("a.webp".to_owned()),
        title: None,
        blurb: None,
        launch_label: None,
        enabled: None,
    };
    assert_eq!(shows.media_names(), vec!["a.webp"]);
    assert!(set_title("x").media_names().is_empty());
    assert!(AdventureEdit::FactsClear {}.media_names().is_empty());
    assert_eq!(
        card().media_names(),
        vec!["adventure-citadel-waterfall.webp"]
    );
}

#[test]
fn an_edit_reads_from_json_with_the_action_tag_and_refuses_an_unknown_field() {
    let edit: AdventureEdit = serde_json::from_str(
        r#"{"action":"fact_add","label":"Region","value":"X","icon":"mountain"}"#,
    )
    .expect("reads");
    assert!(matches!(edit, AdventureEdit::FactAdd { .. }));
    let set: AdventureEdit =
        serde_json::from_str(r#"{"action":"set","picture":{"set":"a.webp"}}"#).expect("reads");
    assert_eq!(set.media_names(), vec!["a.webp"]);
    assert!(serde_json::from_str::<AdventureEdit>(r#"{"action":"set","color":"red"}"#).is_err());
    assert!(serde_json::from_str::<AdventureEdit>(r#"{"action":"nope"}"#).is_err());
}

#[test]
fn every_error_says_what_to_do() {
    for error in [
        AdventureError::TooManyFacts,
        AdventureError::NothingToChange,
        AdventureError::InvalidFactNumber {
            number: 0,
            count: 2,
        },
        AdventureError::InvalidPicture("x".to_owned()),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn a_card_out_of_range_is_refused_where_it_is_read() {
    let read = serde_json::from_str::<AdventureData>(r#"{"picture": "bell.wav"}"#);
    let error = read.expect_err("a sound is not a picture");
    assert!(error.to_string().contains("bell.wav"), "{error}");
    let defaults = serde_json::from_str::<AdventureData>("{}").expect("every field has a default");
    assert_eq!(defaults, AdventureData::default());
    assert!(serde_json::from_str::<AdventureData>(r#"{"rool": 1}"#).is_err());
}
