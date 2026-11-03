//! The `scene home adventure` verbs: what each reads into, what it refuses, and that a run of them
//! builds the card the tour needs.

use df_playground::home_adventure::{AdventureData, AdventureEdit};
use df_playground::home_change::HomeEdit;
use df_playground::home_glyph::{HomeGlyph, HomeIcon};
use df_playground::overlay::{Edit, Glyph};
use df_playground::scene::SceneChange;

use super::ADVENTURE_USAGE;
use crate::command::{Command, CommandError, split_words};

use super::parse;

fn edit(line: &str) -> Result<AdventureEdit, CommandError> {
    let words = split_words(line).expect("the line splits");
    // The words after `scene home adventure`; the dispatch of `scene home` itself is the page's own.
    match parse(&words[3..])? {
        Command::Scene(SceneChange::Home {
            edit: HomeEdit::Adventure { edit },
        }) => Ok(edit),
        other => panic!("expected an adventure change, got {other:?}"),
    }
}

#[test]
fn set_reads_every_field_of_the_card() {
    let edit = edit(
        r#"scene home adventure set heading="Tonight's Adventure" picture=a.webp title="The Ashen Citadel" blurb="Twin moons." launch="Launch Session" enabled=false"#,
    )
    .expect("reads");
    assert_eq!(
        edit,
        AdventureEdit::Set {
            heading: Some("Tonight's Adventure".to_owned()),
            picture: Edit::Set("a.webp".to_owned()),
            title: Some("The Ashen Citadel".to_owned()),
            blurb: Some("Twin moons.".to_owned()),
            launch_label: Some("Launch Session".to_owned()),
            enabled: Some(false),
        }
    );
}

#[test]
fn a_key_left_out_stays_and_an_empty_or_none_picture_takes_the_picture_away() {
    let edit = edit("scene home adventure set title=Ember").expect("reads");
    assert_eq!(
        edit,
        AdventureEdit::Set {
            heading: None,
            picture: Edit::Keep,
            title: Some("Ember".to_owned()),
            blurb: None,
            launch_label: None,
            enabled: None,
        }
    );
    for line in [
        "scene home adventure set picture=none",
        "scene home adventure set picture=",
    ] {
        let AdventureEdit::Set { picture, .. } = self::edit(line).expect("reads") else {
            panic!("a set");
        };
        assert_eq!(picture, Edit::Clear);
    }
    let AdventureEdit::Set { blurb, .. } =
        self::edit("scene home adventure set blurb=").expect("reads")
    else {
        panic!("a set");
    };
    assert_eq!(blurb, Some(String::new()));
}

#[test]
fn set_needs_a_known_key_a_boolean_and_no_key_twice() {
    assert_eq!(
        edit("scene home adventure set"),
        Err(CommandError::Usage(ADVENTURE_USAGE))
    );
    assert!(matches!(
        edit("scene home adventure set colour=red"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home adventure set enabled=maybe"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home adventure set title=a title=b"),
        Err(CommandError::DuplicateKey(_))
    ));
}

#[test]
fn a_fact_row_is_changed_by_number_with_any_icon_of_either_set() {
    assert_eq!(
        edit("scene home adventure fact 3 icon=skull label=\"Threat Level\" value=High")
            .expect("reads"),
        AdventureEdit::FactSet {
            number: 3,
            icon: Some(HomeIcon::Page(HomeGlyph::Skull)),
            label: Some("Threat Level".to_owned()),
            value: Some("High".to_owned()),
        }
    );
    assert_eq!(
        edit("scene home adventure fact 1 icon=crown").expect("reads"),
        AdventureEdit::FactSet {
            number: 1,
            icon: Some(HomeIcon::Overlay(Glyph::Crown)),
            label: None,
            value: None,
        }
    );
}

#[test]
fn a_fact_row_refuses_an_icon_that_does_not_exist_and_says_which_do() {
    let Err(CommandError::BadChoice { key, text, allowed }) =
        edit("scene home adventure fact 1 icon=banana")
    else {
        panic!("expected a refusal");
    };
    assert_eq!((key.as_str(), text.as_str()), ("icon", "banana"));
    assert!(allowed.contains("skull") && allowed.contains("crown"));
    assert!(matches!(
        edit("scene home adventure fact x label=a"),
        Err(CommandError::BadCount { .. })
    ));
    assert_eq!(
        edit("scene home adventure fact 1"),
        Err(CommandError::Usage(ADVENTURE_USAGE))
    );
}

#[test]
fn rows_are_added_removed_and_cleared_and_the_card_taken_away() {
    assert_eq!(
        edit("scene home adventure fact-add Region \"Veylmar Expanse\" icon=mountain")
            .expect("reads"),
        AdventureEdit::FactAdd {
            icon: Some(HomeIcon::Page(HomeGlyph::Mountain)),
            label: "Region".to_owned(),
            value: "Veylmar Expanse".to_owned(),
        }
    );
    assert!(matches!(
        edit("scene home adventure fact-add Region X").expect("reads"),
        AdventureEdit::FactAdd { icon: None, .. }
    ));
    assert_eq!(
        edit("scene home adventure fact-remove 2").expect("reads"),
        AdventureEdit::FactRemove { number: 2 }
    );
    assert_eq!(
        edit("scene home adventure facts-clear").expect("reads"),
        AdventureEdit::FactsClear {}
    );
    assert_eq!(
        edit("scene home adventure clear").expect("reads"),
        AdventureEdit::Clear {}
    );
    for line in [
        "scene home adventure",
        "scene home adventure nonsense",
        "scene home adventure fact-add OnlyLabel",
        "scene home adventure fact-remove",
        "scene home adventure clear now",
    ] {
        assert_eq!(
            edit(line),
            Err(CommandError::Usage(ADVENTURE_USAGE)),
            "{line}"
        );
    }
}

#[test]
fn a_run_of_verbs_builds_the_card_of_the_render() {
    let lines = [
        "scene home adventure set title=\"The Ashen Citadel\" picture=adventure-citadel-waterfall.webp blurb=\"The Iron Abbey stirs beneath twin moons.\"",
        "scene home adventure fact-add Region \"Veylmar Expanse\" icon=mountain",
        "scene home adventure fact-add Biome \"Ruined Highlands\" icon=trees",
        "scene home adventure fact-add \"Threat Level\" High icon=skull",
        "scene home adventure fact-add \"Est. Duration\" \"2 - 3 hours\" icon=hourglass",
        "scene home adventure fact 3 value=Extreme",
        "scene home adventure set enabled=false",
    ];
    let mut slot: Option<AdventureData> = None;
    for line in lines {
        edit(line)
            .expect("reads")
            .apply(&mut slot)
            .unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    let card = slot.expect("a card");
    assert_eq!(card.title, "The Ashen Citadel");
    assert_eq!(card.facts.len(), 4);
    assert_eq!(card.facts[2].value, "Extreme");
    assert!(!card.enabled);
    assert_eq!(card.heading, "Tonight's Adventure");
}
