//! The view decisions of the narration box and the party strip.

use crate::fixture::PartyMember;
use crate::overlay::{NarrationBox, PartyLook, Progress, SeatStrip};
use crate::overlay_rules::{
    DEFAULT_NARRATOR_LABEL, HealthTier, StripCard, bar_percent, class_line, health_tier,
    narrator_label, progress_text, strip_cards,
};

fn member(name: &str, hit_points: u32, maximum: u32) -> PartyMember {
    PartyMember {
        name: name.to_owned(),
        hit_points,
        maximum_hit_points: maximum,
        conditions: Vec::new(),
        color: None,
    }
}

fn look(name: &str) -> PartyLook {
    PartyLook {
        name: name.to_owned(),
        class: Some("Fighter".to_owned()),
        level: Some(3),
        portrait: None,
        glyph: None,
        accent: None,
    }
}

fn narration(speaker: Option<&str>) -> NarrationBox {
    NarrationBox {
        speaker: speaker.map(str::to_owned),
        text: "x".to_owned(),
        continue_label: None,
        badge: None,
        voice: true,
        progress: None,
        placement: Default::default(),
    }
}

#[test]
fn the_narrator_is_the_dungeon_master_unless_the_box_names_someone() {
    assert_eq!(narrator_label(&narration(None)), DEFAULT_NARRATOR_LABEL);
    assert_eq!(
        narrator_label(&narration(Some("  "))),
        DEFAULT_NARRATOR_LABEL
    );
    assert_eq!(narrator_label(&narration(Some("Garrick"))), "Garrick");
}

#[test]
fn the_scene_progress_reads_as_a_sentence_with_or_without_a_name() {
    let named = Progress {
        index: 2,
        total: 5,
        title: Some("Introduction".to_owned()),
    };
    assert_eq!(progress_text(&named), "Scene 2 of 5: Introduction");
    let bare = Progress {
        title: Some(" ".to_owned()),
        ..named
    };
    assert_eq!(progress_text(&bare), "Scene 2 of 5");
}

#[test]
fn every_party_member_is_a_card_in_order_and_open_seats_come_after() {
    let party = [member("Kael", 5, 10), member("Lyra", 7, 7)];
    let seats = SeatStrip {
        looks: vec![look("Lyra")],
        open: 2,
    };
    let cards = strip_cards(&party, Some(&seats));
    assert_eq!(cards.len(), 4);
    assert!(matches!(cards[0], StripCard::Hero(m, None) if m.name == "Kael"));
    assert!(
        matches!(cards[1], StripCard::Hero(m, Some(l)) if m.name == "Lyra" && l.name == "Lyra")
    );
    assert_eq!(cards[2], StripCard::Open);
    assert_eq!(cards[3], StripCard::Open);
}

#[test]
fn a_look_for_someone_who_is_not_in_the_party_draws_no_card() {
    let party = [member("Kael", 5, 10)];
    let seats = SeatStrip {
        looks: vec![look("Ghost")],
        open: 0,
    };
    assert_eq!(strip_cards(&party, Some(&seats)).len(), 1);
}

#[test]
fn with_no_party_and_no_open_seats_there_is_nothing_to_draw() {
    assert!(strip_cards(&[], None).is_empty());
    assert!(strip_cards(&[], Some(&SeatStrip::default())).is_empty());
}

#[test]
fn a_party_without_seat_looks_still_shows_every_hero() {
    let party = [
        member("Kael", 5, 10),
        member("Lyra", 7, 7),
        member("Pip", 1, 9),
    ];
    let cards = strip_cards(&party, None);
    assert_eq!(cards.len(), 3);
    assert!(
        cards
            .iter()
            .all(|card| matches!(card, StripCard::Hero(_, None)))
    );
}

#[test]
fn the_class_line_joins_class_and_level_and_leaves_out_what_is_missing() {
    assert_eq!(
        class_line(Some("Fighter"), Some(3)).as_deref(),
        Some("Fighter Lv. 3")
    );
    assert_eq!(class_line(Some(" Rogue "), None).as_deref(), Some("Rogue"));
    assert_eq!(class_line(None, Some(5)).as_deref(), Some("Lv. 5"));
    assert_eq!(class_line(Some(" "), None), None);
    assert_eq!(class_line(None, None), None);
}

#[test]
fn a_health_bar_is_red_below_a_quarter_amber_below_a_half_and_high_otherwise() {
    assert_eq!(health_tier(10, 10), HealthTier::High);
    assert_eq!(health_tier(5, 10), HealthTier::High);
    assert_eq!(health_tier(4, 10), HealthTier::Amber);
    assert_eq!(health_tier(3, 10), HealthTier::Amber);
    assert_eq!(health_tier(2, 10), HealthTier::Red);
    assert_eq!(health_tier(0, 10), HealthTier::Red);
    assert_eq!(
        health_tier(5, 0),
        HealthTier::Red,
        "no maximum is an empty bar"
    );
    assert_eq!(HealthTier::Amber.key(), "amber");
}

#[test]
fn a_bar_fills_to_its_share_and_never_past_full() {
    assert!((bar_percent(22, 28) - 78.5).abs() < 0.001);
    assert!((bar_percent(40, 28) - 100.0).abs() < 0.001);
    assert!(bar_percent(3, 0).abs() < 0.001);
}
