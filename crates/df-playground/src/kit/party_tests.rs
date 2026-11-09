//! The party strip's cards, layouts and health.

use crate::fixture::PartyMember;
use crate::kit::Fill;
use crate::kit::kit_tests::assert_css_words_are_serde_names;
use crate::kit::party::{
    COMPACT_CONDITIONS_AT_MOST, KitCard, PartyLayout, PartyStrip, TALL_CONDITIONS_AT_MOST,
};
use crate::overlay::rules::HealthTier;
use crate::overlay::{Glyph, PartyLook, SeatStrip};

fn member(name: &str, hit_points: u32, maximum: u32) -> PartyMember {
    PartyMember {
        name: name.to_owned(),
        hit_points,
        maximum_hit_points: maximum,
        conditions: Vec::new(),
        color: Some("#e5483f".to_owned()),
    }
}

fn strip(party: Vec<PartyMember>, open: u8) -> PartyStrip {
    PartyStrip {
        party,
        seats: Some(SeatStrip {
            looks: vec![PartyLook {
                name: "Kael".to_owned(),
                class: Some("Fighter".to_owned()),
                level: Some(3),
                portrait: Some("brakka.webp".to_owned()),
                glyph: Some(Glyph::Shield),
                accent: Some("#f6b14e".to_owned()),
            }],
            open,
        }),
        layout: None,
        show_hp_text: false,
        framed: None,
        downed: Vec::new(),
    }
}

fn heroes(count: usize) -> Vec<PartyMember> {
    (0..count)
        .map(|number| member(&format!("Hero {number}"), 1, 1))
        .collect()
}

#[test]
fn every_hero_is_drawn_with_their_look_then_the_open_seats() {
    let strip = strip(vec![member("Kael", 42, 42), member("Lyra", 9, 38)], 2);
    let cards = strip.cards();
    assert_eq!(cards.len(), 4);
    let KitCard::Hero(kael) = &cards[0] else {
        panic!("the first card is a hero");
    };
    assert_eq!(kael.class_name.as_deref(), Some("Fighter"));
    assert_eq!(kael.level.as_deref(), Some("Lv. 3"));
    assert_eq!(kael.accent, Some("#f6b14e"));
    assert_eq!(kael.hit_points_text, "HP 42/42");
    assert_eq!(kael.tier, HealthTier::High);
    let KitCard::Hero(lyra) = &cards[1] else {
        panic!("the second card is a hero");
    };
    assert_eq!(lyra.class_name, None);
    assert_eq!(lyra.level, None);
    assert_eq!(lyra.accent, Some("#e5483f"));
    assert_eq!(lyra.tier, HealthTier::Red);
    assert_eq!(cards[2], KitCard::Open);
}

#[test]
fn a_hero_is_downed_only_when_the_host_says_so() {
    let mut strip = strip(vec![member("Kael", 0, 42), member("Lyra", 0, 38)], 0);
    strip.downed = vec![1];
    let cards = strip.cards();
    let KitCard::Hero(kael) = &cards[0] else {
        panic!("a hero");
    };
    assert!(!kael.downed, "no hit points alone draws no downed look");
    assert_eq!(kael.bar_percent, 0.0);
    let KitCard::Hero(lyra) = &cards[1] else {
        panic!("a hero");
    };
    assert!(lyra.downed);
}

#[test]
fn past_six_cards_the_strip_is_dense() {
    assert!(!strip(heroes(4), 2).is_dense());
    assert!(strip(heroes(5), 2).is_dense());
    assert!(strip(heroes(8), 0).is_dense());
}

#[test]
fn the_strip_is_tall_up_to_six_cards_and_compact_past_them_unless_told() {
    assert_eq!(strip(heroes(4), 2).layout(), PartyLayout::Tall);
    let mut seven = strip(heroes(5), 2);
    assert_eq!(seven.layout(), PartyLayout::Compact);
    seven.layout = Some(PartyLayout::Tall);
    assert_eq!(seven.layout(), PartyLayout::Tall);
}

#[test]
fn a_tall_strip_is_one_blurred_bar_and_a_compact_one_loose_fine_cards_unless_told() {
    let mut tall = strip(heroes(4), 0);
    assert!(tall.framed());
    assert_eq!(tall.fill(), Fill::Blurred);
    tall.layout = Some(PartyLayout::Compact);
    assert!(!tall.framed());
    assert_eq!(tall.fill(), Fill::Fine);
    tall.framed = Some(true);
    assert_eq!(tall.fill(), Fill::Blurred);
}

#[test]
fn an_empty_strip_reads_and_has_no_cards() {
    let strip: PartyStrip = serde_json::from_str("{}").expect("an empty strip reads");
    assert!(strip.cards().is_empty());
    assert_css_words_are_serde_names(&[PartyLayout::Tall, PartyLayout::Compact], PartyLayout::key);
}

#[test]
fn a_card_shows_its_first_conditions_and_counts_the_rest() {
    let mut poisoned = member("Kael", 9, 9);
    poisoned.conditions = ["Poisoned", " ", "Frightened", "Restrained"]
        .map(str::to_owned)
        .to_vec();
    let mut tall = strip(vec![poisoned], 0);
    assert_eq!(tall.conditions_at_most(), TALL_CONDITIONS_AT_MOST);
    let cards = tall.cards();
    let KitCard::Hero(hero) = &cards[0] else {
        panic!("a hero");
    };
    assert_eq!(
        hero.conditions(),
        vec!["Poisoned", "Frightened", "Restrained"]
    );
    let chips = hero.condition_chips(TALL_CONDITIONS_AT_MOST);
    assert_eq!(chips.shown, vec!["Poisoned", "Frightened"]);
    assert_eq!(chips.more.as_deref(), Some("+1"));
    let all = hero.condition_chips(3);
    assert_eq!(all.shown.len(), 3);
    assert_eq!(all.more, None);
    tall.layout = Some(PartyLayout::Compact);
    assert_eq!(tall.conditions_at_most(), COMPACT_CONDITIONS_AT_MOST);
    let cards = tall.cards();
    let KitCard::Hero(hero) = &cards[0] else {
        panic!("a hero");
    };
    let chips = hero.condition_chips(COMPACT_CONDITIONS_AT_MOST);
    assert_eq!(chips.shown, vec!["Poisoned"]);
    assert_eq!(chips.more.as_deref(), Some("+2"));
}
