//! The campaign strip's data and its live edits: defaults, bounds, all-or-nothing edits, numbering,
//! the one selected card and what the audit line carries.

use super::campaigns::*;
use crate::limits::{MAXIMUM_CAMPAIGN_CARDS, MAXIMUM_NAME_CHARACTERS};
use crate::overlay::Edit;

fn card(title: &str, selected: bool) -> CampaignCard {
    CampaignCard {
        art: Some("campaign-ashen-citadel.webp".to_owned()),
        title: title.to_owned(),
        last_played: "Last played: Yesterday".to_owned(),
        selected,
    }
}

fn strip() -> CampaignsData {
    CampaignsData {
        cards: vec![card("The Ashen Citadel", true), card("Neon Hollow", false)],
        ..CampaignsData::default()
    }
}

fn add(title: &str, selected: Option<bool>) -> CampaignsEdit {
    CampaignsEdit::CardAdd {
        art: None,
        title: title.to_owned(),
        last_played: None,
        selected,
    }
}

#[test]
fn an_empty_strip_reads_with_the_default_title_and_button() {
    let data: CampaignsData = serde_json::from_str("{}").expect("an empty strip reads");
    assert_eq!(data.title, DEFAULT_STRIP_TITLE);
    assert_eq!(data.view_all, DEFAULT_VIEW_ALL);
    assert!(data.cards.is_empty());
    assert_eq!(data.check(), Ok(()));
}

#[test]
fn a_strip_survives_a_round_trip_and_refuses_a_field_it_does_not_have() {
    let data = strip();
    let text = serde_json::to_string(&data).expect("writes");
    assert_eq!(
        serde_json::from_str::<CampaignsData>(&text).expect("reads"),
        data
    );
    assert!(serde_json::from_str::<CampaignsData>(r#"{"cards":[{"titel":"x"}]}"#).is_err());
    let lone: CampaignCard = serde_json::from_str(r#"{"title":"Only"}"#).expect("reads");
    assert_eq!(lone.art, None);
    assert!(!lone.selected);
}

#[test]
fn the_bounds_are_checked_at_the_edge() {
    let mut data = strip();
    data.cards[0].title = "x".repeat(MAXIMUM_NAME_CHARACTERS + 1);
    assert!(matches!(data.check(), Err(CampaignsError::TooLong { .. })));
    let mut data = strip();
    data.cards[1].art = Some("notes.txt".to_owned());
    assert_eq!(
        data.check(),
        Err(CampaignsError::InvalidArt("notes.txt".to_owned()))
    );
    let mut data = strip();
    data.cards = vec![card("a", false); MAXIMUM_CAMPAIGN_CARDS + 1];
    assert_eq!(data.check(), Err(CampaignsError::TooManyCards));
    data.cards.pop();
    assert_eq!(data.check(), Ok(()));
}

#[test]
fn two_selected_cards_in_a_file_are_refused() {
    let mut data = strip();
    data.cards[1].selected = true;
    assert_eq!(data.check(), Err(CampaignsError::ManySelected { count: 2 }));
}

#[test]
fn the_strip_draws_the_first_three_cards_and_keeps_the_rest_in_the_data() {
    assert!(CampaignsData::default().shown_cards().is_empty());
    assert_eq!(strip().shown_cards().len(), 2);
    let mut data = strip();
    data.cards = vec![card("a", false); 7];
    assert_eq!(data.shown_cards().len(), CARDS_SHOWN);
    assert_eq!(data.cards.len(), 7);
}

#[test]
fn focus_ids_name_the_card_by_its_number_and_the_see_all_button() {
    assert_eq!(card_focus_id(2), "campaigns:2");
}

#[test]
fn a_set_makes_the_strip_and_changes_only_what_it_names() {
    let mut slot = None;
    CampaignsEdit::Set {
        title: Some("Pick up again".to_owned()),
        view_all: None,
    }
    .apply(&mut slot)
    .expect("makes the strip");
    let made = slot.clone().expect("a strip");
    assert_eq!(made.title, "Pick up again");
    assert_eq!(made.view_all, DEFAULT_VIEW_ALL);
    CampaignsEdit::Set {
        title: None,
        view_all: Some(String::new()),
    }
    .apply(&mut slot)
    .expect("takes the button away");
    assert_eq!(slot.clone().expect("a strip").view_all, "");
    let nothing = CampaignsEdit::Set {
        title: None,
        view_all: None,
    };
    assert_eq!(
        nothing.apply(&mut slot),
        Err(CampaignsError::NothingToChange)
    );
}

#[test]
fn adding_a_selected_card_moves_the_selection_to_it() {
    let mut slot = Some(strip());
    add("Crown", Some(true)).apply(&mut slot).expect("adds");
    let cards = slot.clone().expect("a strip").cards;
    assert_eq!(cards.len(), 3);
    assert_eq!(
        cards.iter().map(|card| card.selected).collect::<Vec<_>>(),
        vec![false, false, true]
    );
    add("Frontier", None).apply(&mut slot).expect("adds");
    assert_eq!(slot.expect("a strip").selected_count(), 1);
}

#[test]
fn a_card_is_numbered_from_one_and_every_part_can_change() {
    let mut slot = Some(strip());
    CampaignsEdit::CardSet {
        number: 2,
        art: Edit::Clear,
        title: Some("Neon Hollow II".to_owned()),
        last_played: Some("Last played: Today".to_owned()),
        selected: Some(true),
    }
    .apply(&mut slot)
    .expect("changes card 2");
    let cards = slot.clone().expect("a strip").cards;
    assert_eq!(cards[1].title, "Neon Hollow II");
    assert_eq!(cards[1].art, None);
    assert_eq!(cards[1].last_played, "Last played: Today");
    assert!(cards[1].selected && !cards[0].selected);
    for number in [0, 3] {
        let edit = CampaignsEdit::CardSet {
            number,
            art: Edit::Keep,
            title: Some("x".to_owned()),
            last_played: None,
            selected: None,
        };
        assert_eq!(
            edit.apply(&mut slot),
            Err(CampaignsError::InvalidCardNumber { number, count: 2 })
        );
    }
    let nothing = CampaignsEdit::CardSet {
        number: 1,
        art: Edit::Keep,
        title: None,
        last_played: None,
        selected: None,
    };
    assert_eq!(
        nothing.apply(&mut slot),
        Err(CampaignsError::NothingToChange)
    );
}

#[test]
fn deselecting_the_only_selected_card_leaves_none_selected() {
    let mut slot = Some(strip());
    CampaignsEdit::CardSet {
        number: 1,
        art: Edit::Keep,
        title: None,
        last_played: None,
        selected: Some(false),
    }
    .apply(&mut slot)
    .expect("deselects");
    assert_eq!(slot.expect("a strip").selected_count(), 0);
}

#[test]
fn select_moves_the_pick_and_without_a_number_clears_it() {
    let mut slot = Some(strip());
    CampaignsEdit::Select { number: Some(2) }
        .apply(&mut slot)
        .expect("selects");
    let cards = slot.clone().expect("a strip").cards;
    assert!(!cards[0].selected && cards[1].selected);
    CampaignsEdit::Select { number: None }
        .apply(&mut slot)
        .expect("clears");
    assert_eq!(slot.clone().expect("a strip").selected_count(), 0);
    assert_eq!(
        CampaignsEdit::Select { number: Some(3) }.apply(&mut slot),
        Err(CampaignsError::InvalidCardNumber {
            number: 3,
            count: 2
        })
    );
}

#[test]
fn cards_are_removed_by_number_cleared_and_the_strip_taken_away() {
    let mut slot = Some(strip());
    CampaignsEdit::CardRemove { number: 1 }
        .apply(&mut slot)
        .expect("removes");
    assert_eq!(slot.clone().expect("a strip").cards[0].title, "Neon Hollow");
    assert_eq!(
        CampaignsEdit::CardRemove { number: 2 }.apply(&mut slot),
        Err(CampaignsError::InvalidCardNumber {
            number: 2,
            count: 1
        })
    );
    CampaignsEdit::CardsClear {}
        .apply(&mut slot)
        .expect("clears");
    assert!(slot.clone().expect("a strip").cards.is_empty());
    CampaignsEdit::Clear {}
        .apply(&mut slot)
        .expect("takes away");
    assert_eq!(slot, None);
}

#[test]
fn a_refused_edit_leaves_the_strip_exactly_as_it_was() {
    let mut slot = Some(strip());
    let bad = CampaignsEdit::CardAdd {
        art: Some("a b.webp".to_owned()),
        title: "x".to_owned(),
        last_played: None,
        selected: Some(true),
    };
    assert!(bad.apply(&mut slot).is_err());
    assert_eq!(slot, Some(strip()));
    let mut full = strip();
    full.cards = vec![card("a", false); MAXIMUM_CAMPAIGN_CARDS];
    let mut slot = Some(full.clone());
    assert_eq!(
        add("one more", None).apply(&mut slot),
        Err(CampaignsError::TooManyCards)
    );
    assert_eq!(slot, Some(full));
}

#[test]
fn the_audit_line_carries_the_field_numbers_and_flags_but_never_the_text() {
    let (field, pairs) = CampaignsEdit::CardSet {
        number: 2,
        art: Edit::Set("campaign-neon-alley.webp".to_owned()),
        title: Some("Secret title".to_owned()),
        last_played: Some("Secret day".to_owned()),
        selected: Some(true),
    }
    .audit_fields();
    assert_eq!(field, "card");
    assert!(pairs.contains(&("card", "2".to_owned())));
    assert!(pairs.contains(&("art", "set".to_owned())));
    assert!(pairs.contains(&("title_chars", "12".to_owned())));
    assert!(pairs.contains(&("selected", "true".to_owned())));
    for (_, value) in &pairs {
        assert!(!value.contains("Secret"));
    }
    let (field, pairs) = CampaignsEdit::Select { number: None }.audit_fields();
    assert_eq!(
        (field, pairs),
        ("select", vec![("card", "none".to_owned())])
    );
}

#[test]
fn an_edit_names_the_pictures_it_shows_and_only_those() {
    let shows = CampaignsEdit::CardAdd {
        art: Some("a.webp".to_owned()),
        title: "x".to_owned(),
        last_played: None,
        selected: None,
    };
    assert_eq!(shows.media_names(), vec!["a.webp"]);
    let sets = CampaignsEdit::CardSet {
        number: 1,
        art: Edit::Set("b.webp".to_owned()),
        title: None,
        last_played: None,
        selected: None,
    };
    assert_eq!(sets.media_names(), vec!["b.webp"]);
    assert!(add("x", None).media_names().is_empty());
    assert!(CampaignsEdit::CardsClear {}.media_names().is_empty());
}

#[test]
fn an_edit_reads_from_json_with_the_action_tag_and_refuses_an_unknown_field() {
    let edit: CampaignsEdit =
        serde_json::from_str(r#"{"action":"card_add","title":"Neon Hollow"}"#).expect("reads");
    assert!(matches!(edit, CampaignsEdit::CardAdd { .. }));
    let select: CampaignsEdit = serde_json::from_str(r#"{"action":"select"}"#).expect("reads");
    assert_eq!(select, CampaignsEdit::Select { number: None });
    assert!(serde_json::from_str::<CampaignsEdit>(r#"{"action":"select","card":1}"#).is_err());
}

#[test]
fn every_error_says_what_to_do() {
    for error in [
        CampaignsError::TooManyCards,
        CampaignsError::NothingToChange,
        CampaignsError::InvalidCardNumber {
            number: 0,
            count: 2,
        },
        CampaignsError::ManySelected { count: 2 },
        CampaignsError::InvalidArt("x".to_owned()),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn a_strip_out_of_range_is_refused_where_it_is_read() {
    let title = "x".repeat(MAXIMUM_NAME_CHARACTERS + 1);
    let read = serde_json::from_str::<CampaignsData>(&format!(r#"{{"title": "{title}"}}"#));
    assert!(read.is_err());
    let defaults = serde_json::from_str::<CampaignsData>("{}").expect("every field has a default");
    assert_eq!(defaults, CampaignsData::default());
    assert!(serde_json::from_str::<CampaignsData>(r#"{"rool": 1}"#).is_err());
}
