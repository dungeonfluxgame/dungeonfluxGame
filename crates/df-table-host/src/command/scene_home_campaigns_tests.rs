//! The `scene home campaigns` verbs: what each reads into, what it refuses, and that a run of them
//! builds the strip the tour needs.

use df_playground::home_campaigns::{CampaignsData, CampaignsEdit};
use df_playground::home_change::HomeEdit;
use df_playground::overlay::Edit;
use df_playground::scene::SceneChange;

use super::CAMPAIGNS_USAGE;
use crate::command::{Command, CommandError, split_words};

use super::parse;

fn edit(line: &str) -> Result<CampaignsEdit, CommandError> {
    let words = split_words(line).expect("the line splits");
    // The words after `scene home campaigns`; the dispatch of `scene home` itself is the page's own.
    match parse(&words[3..])? {
        Command::Scene(SceneChange::Home {
            edit: HomeEdit::Campaigns { edit },
        }) => Ok(edit),
        other => panic!("expected a campaigns change, got {other:?}"),
    }
}

#[test]
fn set_reads_the_header_and_leaves_out_what_it_does_not_name() {
    assert_eq!(
        edit(r#"scene home campaigns set title="Pick up again" view-all="See all""#)
            .expect("reads"),
        CampaignsEdit::Set {
            title: Some("Pick up again".to_owned()),
            view_all: Some("See all".to_owned()),
        }
    );
    assert_eq!(
        edit("scene home campaigns set view-all=").expect("reads"),
        CampaignsEdit::Set {
            title: None,
            view_all: Some(String::new()),
        }
    );
    assert_eq!(
        edit("scene home campaigns set"),
        Err(CommandError::Usage(CAMPAIGNS_USAGE))
    );
    assert!(matches!(
        edit("scene home campaigns set colour=red"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn a_card_is_added_with_its_title_first() {
    assert_eq!(
        edit(
            r#"scene home campaigns card-add "Neon Hollow" art=campaign-neon-alley.webp last="Last played: 3 days ago" selected=true"#
        )
        .expect("reads"),
        CampaignsEdit::CardAdd {
            art: Some("campaign-neon-alley.webp".to_owned()),
            title: "Neon Hollow".to_owned(),
            last_played: Some("Last played: 3 days ago".to_owned()),
            selected: Some(true),
        }
    );
    assert!(matches!(
        edit("scene home campaigns card-add Plain").expect("reads"),
        CampaignsEdit::CardAdd {
            art: None,
            last_played: None,
            selected: None,
            ..
        }
    ));
    assert!(matches!(
        edit("scene home campaigns card-add X selected=perhaps"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn a_card_is_changed_by_number_and_none_takes_its_art_away() {
    assert_eq!(
        edit("scene home campaigns card 2 art=a.webp title=Crown last=Today selected=false")
            .expect("reads"),
        CampaignsEdit::CardSet {
            number: 2,
            art: Edit::Set("a.webp".to_owned()),
            title: Some("Crown".to_owned()),
            last_played: Some("Today".to_owned()),
            selected: Some(false),
        }
    );
    let CampaignsEdit::CardSet { art, title, .. } =
        edit("scene home campaigns card 1 art=none").expect("reads")
    else {
        panic!("a card change");
    };
    assert_eq!((art, title), (Edit::Clear, None));
    assert_eq!(
        edit("scene home campaigns card 1"),
        Err(CommandError::Usage(CAMPAIGNS_USAGE))
    );
    assert!(matches!(
        edit("scene home campaigns card two title=x"),
        Err(CommandError::BadCount { .. })
    ));
}

#[test]
fn cards_are_removed_cleared_selected_and_the_strip_taken_away() {
    assert_eq!(
        edit("scene home campaigns card-remove 3").expect("reads"),
        CampaignsEdit::CardRemove { number: 3 }
    );
    assert_eq!(
        edit("scene home campaigns cards-clear").expect("reads"),
        CampaignsEdit::CardsClear {}
    );
    assert_eq!(
        edit("scene home campaigns select 2").expect("reads"),
        CampaignsEdit::Select { number: Some(2) }
    );
    assert_eq!(
        edit("scene home campaigns select none").expect("reads"),
        CampaignsEdit::Select { number: None }
    );
    assert!(matches!(
        edit("scene home campaigns select first"),
        Err(CommandError::BadChoice { .. })
    ));
    assert_eq!(
        edit("scene home campaigns clear").expect("reads"),
        CampaignsEdit::Clear {}
    );
    for line in [
        "scene home campaigns",
        "scene home campaigns nonsense",
        "scene home campaigns card-remove",
        "scene home campaigns clear now",
    ] {
        assert_eq!(
            edit(line),
            Err(CommandError::Usage(CAMPAIGNS_USAGE)),
            "{line}"
        );
    }
}

#[test]
fn a_run_of_verbs_builds_the_strip_of_the_render() {
    let lines = [
        "scene home campaigns card-add \"The Ashen Citadel\" art=campaign-ashen-citadel.webp last=\"Last played: Yesterday\" selected=true",
        "scene home campaigns card-add \"Neon Hollow\" art=campaign-neon-alley.webp last=\"Last played: 3 days ago\"",
        "scene home campaigns card-add \"Crown of the Last Machine\" art=campaign-ringed-station.webp last=\"Last played: 1 week ago\"",
        "scene home campaigns card-add \"Broken Star Frontier\" art=campaign-starfield-frontier.webp last=\"Last played: 2 weeks ago\"",
        "scene home campaigns select 3",
    ];
    let mut slot: Option<CampaignsData> = None;
    for line in lines {
        edit(line)
            .expect("reads")
            .apply(&mut slot)
            .unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    let strip = slot.expect("a strip");
    assert_eq!(strip.title, "Recent Campaigns");
    assert_eq!(strip.cards.len(), 4);
    assert_eq!(
        strip
            .cards
            .iter()
            .map(|card| card.selected)
            .collect::<Vec<_>>(),
        vec![false, false, true, false]
    );
}
