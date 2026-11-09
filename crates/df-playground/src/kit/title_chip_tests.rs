//! The title chip's parts, its track and the top bar's pills as chips.

use crate::home::glyph::{HomeGlyph, HomeIcon};
use crate::kit::Step;
use crate::kit::title_chip::{TitleChip, top_bar_chips};
use crate::lobby::model::{PartyCount, TopBar};

#[test]
fn a_chip_keeps_its_worded_parts_and_drops_blank_ones() {
    let chip: TitleChip = serde_json::from_str(
        r#"{"icon":"book","title":"The Ember Roads","parts":["Scene 3 of 10"," "],"subtitle":""}"#,
    )
    .expect("a chip reads");
    assert_eq!(chip.title(), Some("The Ember Roads"));
    assert_eq!(chip.parts(), vec!["Scene 3 of 10"]);
    assert_eq!(chip.subtitle(), None);
    assert!(chip.track().is_empty());
}

#[test]
fn a_track_draws_one_dot_per_scene() {
    let chip: TitleChip =
        serde_json::from_str(r#"{"title":"Road","track":{"index":2,"total":3,"title":null}}"#)
            .expect("a chip reads");
    assert_eq!(chip.track(), vec![Step::Past, Step::Current, Step::Future]);
}

#[test]
fn a_blank_title_is_not_drawn() {
    let chip: TitleChip = serde_json::from_str(r#"{"title":"  "}"#).expect("a chip reads");
    assert_eq!(chip.title(), None);
}

#[test]
fn the_top_bar_pills_become_chips_in_order_with_their_words() {
    let bar = TopBar {
        session: Some("The Ember Roads".to_owned()),
        party: Some(PartyCount {
            joined: 4,
            capacity: 6,
        }),
        balance: Some(" ".to_owned()),
        rate: Some("$3.90/hr".to_owned()),
        ..TopBar::default()
    };
    let chips = top_bar_chips(&bar);
    let titles: Vec<&str> = chips.iter().filter_map(TitleChip::title).collect();
    assert_eq!(
        titles,
        vec!["Session: The Ember Roads", "Party: 4 / 6", "$3.90/hr"]
    );
    assert_eq!(chips[1].icon, Some(HomeIcon::Page(HomeGlyph::Players)));
    assert!(top_bar_chips(&TopBar::default()).is_empty());
}
