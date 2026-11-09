//! The title card's parts, ornament and conversion from a close-up.

use crate::closeup::CloseUp;
use crate::kit::kit_tests::assert_css_words_are_serde_names;
use crate::kit::title_card::{CardAlign, CardScale, TitleCard};
use crate::kit::{Fill, Rarity};

#[test]
fn a_close_up_converts_into_a_centred_item_card_with_the_compass_rose() {
    let card = TitleCard::from(&CloseUp {
        title: Some("Sunfall Longsword".to_owned()),
        description: "A finely crafted longsword.".to_owned(),
    });
    assert!(card.is_drawn());
    assert_eq!(card.align, CardAlign::Center);
    assert_eq!(card.scale, CardScale::Item);
    assert_eq!(card.ornament, None);
    assert_eq!(card.fill(), Fill::Blurred);
    assert_eq!(card.title(), Some("Sunfall Longsword"));
}

#[test]
fn a_chapter_card_reads_its_kicker_and_a_card_without_words_is_not_drawn() {
    let chapter: TitleCard = serde_json::from_str(
        r#"{"kicker":"Entering","title":"The Ashen Citadel",
            "scale":"chapter","rarity":"very_rare"}"#,
    )
    .expect("a card reads");
    assert_eq!(chapter.kicker(), Some("Entering"));
    assert_eq!(chapter.rarity, Some(Rarity::VeryRare));
    assert_eq!(chapter.fill(), Fill::Solid);
    let empty: TitleCard =
        serde_json::from_str(r#"{"title":" ","text":""}"#).expect("a card reads");
    assert!(!empty.is_drawn());
}

#[test]
fn the_closed_keys_are_their_serde_names() {
    assert_css_words_are_serde_names(&[CardAlign::Center, CardAlign::Right], CardAlign::key);
    assert_css_words_are_serde_names(&[CardScale::Item, CardScale::Chapter], CardScale::key);
}
