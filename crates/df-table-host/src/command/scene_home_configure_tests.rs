//! The `scene home configure` verbs: what each group reads into, what it refuses, and that a run of
//! them builds the panel the tour needs.

use df_playground::home_change::HomeEdit;
use df_playground::home_configure::{
    Accent, CardKind, ConfigureEdit, ConfigureIcon, PanelLayout, RowEdit, RowKind, SliderUnit,
    StatValue,
};
use df_playground::overlay::Edit;
use df_playground::scene::SceneChange;

use super::CONFIGURE_USAGE;
use crate::command::{Command, CommandError, split_words};

use super::parse;

fn edit(line: &str) -> Result<ConfigureEdit, CommandError> {
    let words = split_words(line).expect("the line splits");
    // The words after `scene home configure`; the dispatch of `scene home` itself is the page's own.
    match parse(&words[3..])? {
        Command::Scene(SceneChange::Home {
            edit: HomeEdit::Configure { edit },
        }) => Ok(edit),
        other => panic!("expected a configure change, got {other:?}"),
    }
}

#[test]
fn the_panel_and_the_preset_selector_read_their_fields() {
    assert_eq!(
        edit(r#"scene home configure panel title="Tonight's Table" currency=GBP layout=tiles"#)
            .expect("reads"),
        ConfigureEdit::Panel {
            title: Some("Tonight's Table".to_owned()),
            currency: Some("GBP".to_owned()),
            layout: Some(PanelLayout::Tiles),
        }
    );
    assert!(matches!(
        edit("scene home configure panel layout=wide"),
        Err(CommandError::BadChoice { .. })
    ));
    assert_eq!(
        edit(r#"scene home configure preset value="Story First" options="Balanced Adventure,Story First""#)
            .expect("reads"),
        ConfigureEdit::Preset {
            label: None,
            value: Some("Story First".to_owned()),
            options: Some(vec!["Balanced Adventure".to_owned(), "Story First".to_owned()]),
        }
    );
    assert_eq!(
        edit("scene home configure preset-remove").expect("reads"),
        ConfigureEdit::PresetRemove {}
    );
    for line in ["scene home configure panel", "scene home configure preset"] {
        assert_eq!(edit(line), Err(CommandError::Usage(CONFIGURE_USAGE)));
    }
}

#[test]
fn a_card_changes_its_own_fields_and_none_takes_the_icon_and_banner_away() {
    assert_eq!(
        edit(
            "scene home configure card world label=Setting icon=globe banner=a.webp accent=violet shown=2"
        )
        .expect("reads"),
        ConfigureEdit::Card {
            card: "world".to_owned(),
            label: Some("Setting".to_owned()),
            icon: Edit::Set(ConfigureIcon::Globe),
            banner: Edit::Set("a.webp".to_owned()),
            accent: Some(Accent::Violet),
            shown_rows: Some(2),
        }
    );
    let ConfigureEdit::Card { icon, banner, .. } =
        edit("scene home configure card world icon=none banner=").expect("reads")
    else {
        panic!("a card");
    };
    assert_eq!((icon, banner), (Edit::Clear, Edit::Clear));
    let Err(CommandError::BadChoice { key, allowed, .. }) =
        edit("scene home configure card world icon=banana")
    else {
        panic!("expected a refusal");
    };
    assert_eq!(key, "icon");
    assert!(allowed.contains("globe") && allowed.contains("coins"));
    assert!(matches!(
        edit("scene home configure card world accent=pink"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home configure card world shown=300"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn cards_are_added_removed_and_moved_by_key_and_position() {
    assert_eq!(
        edit("scene home configure card-add extras kind=mixed label=Extras icon=dice at=2")
            .expect("reads"),
        ConfigureEdit::CardAdd {
            card: "extras".to_owned(),
            kind: CardKind::Mixed,
            label: "Extras".to_owned(),
            icon: Some(ConfigureIcon::Dice),
            banner: None,
            accent: None,
            shown_rows: None,
            at: Some(2),
        }
    );
    assert_eq!(
        edit("scene home configure card-remove extras").expect("reads"),
        ConfigureEdit::CardRemove {
            card: "extras".to_owned()
        }
    );
    assert_eq!(
        edit("scene home configure card-move extras 1").expect("reads"),
        ConfigureEdit::CardMove {
            card: "extras".to_owned(),
            position: 1
        }
    );
    // A new card needs its kind and its label.
    assert_eq!(
        edit("scene home configure card-add extras label=Extras"),
        Err(CommandError::Usage(CONFIGURE_USAGE))
    );
    assert!(matches!(
        edit("scene home configure card-add extras kind=huge label=A"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home configure card-move extras x"),
        Err(CommandError::BadCount { .. })
    ));
}

#[test]
fn a_slider_row_reads_its_level_unit_and_steps() {
    assert_eq!(
        edit("scene home configure row rules combat_intensity level=60 unit=steps steps=Low,Medium,High label=Combat")
            .expect("reads"),
        ConfigureEdit::Row {
            card: "rules".to_owned(),
            row: "combat_intensity".to_owned(),
            edit: RowEdit {
                label: Some("Combat".to_owned()),
                level: Some(60),
                unit: Some(SliderUnit::Steps),
                steps: Some(vec!["Low".to_owned(), "Medium".to_owned(), "High".to_owned()]),
                ..RowEdit::default()
            },
        }
    );
    assert!(matches!(
        edit("scene home configure row rules a level=300"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home configure row rules a unit=ratio"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn a_select_row_reads_its_value_and_options() {
    assert_eq!(
        edit(r#"scene home configure row ai narrator_voice value="Aria (Epic)" options="Aria (Epic),Vaelin (Epic)""#)
            .expect("reads"),
        ConfigureEdit::Row {
            card: "ai".to_owned(),
            row: "narrator_voice".to_owned(),
            edit: RowEdit {
                value: Some("Aria (Epic)".to_owned()),
                options: Some(vec!["Aria (Epic)".to_owned(), "Vaelin (Epic)".to_owned()]),
                ..RowEdit::default()
            },
        }
    );
}

#[test]
fn a_stat_row_reads_an_amount_in_hundredths_or_with_a_decimal_point() {
    let stat = |line: &str| match edit(line).expect("reads") {
        ConfigureEdit::Row { edit, .. } => edit,
        other => panic!("expected a row change, got {other:?}"),
    };
    assert_eq!(
        stat("scene home configure row budget balance money=28.40").stat,
        Edit::Set(StatValue::Money { cents: 2840 })
    );
    assert_eq!(
        stat("scene home configure row budget balance money=2840").stat,
        Edit::Set(StatValue::Money { cents: 2840 })
    );
    assert_eq!(
        stat("scene home configure row budget balance money=5.5").stat,
        Edit::Set(StatValue::Money { cents: 550 })
    );
    assert_eq!(
        stat("scene home configure row budget spend per-hour=3.90 info=true highlight=false"),
        RowEdit {
            stat: Edit::Set(StatValue::PerHour { cents: 390 }),
            info: Some(true),
            highlight: Some(false),
            ..RowEdit::default()
        }
    );
    assert_eq!(
        stat("scene home configure row budget estimate range=7.80-11.70").stat,
        Edit::Set(StatValue::Range {
            low_cents: 780,
            high_cents: 1170
        })
    );
    assert_eq!(
        stat("scene home configure row budget estimate text=Free").stat,
        Edit::Set(StatValue::Text {
            text: "Free".to_owned()
        })
    );
    assert_eq!(
        stat("scene home configure row budget estimate no-value=true").stat,
        Edit::Clear
    );
}

#[test]
fn a_stat_row_refuses_a_bad_amount_and_two_values_at_once() {
    for line in [
        "scene home configure row budget a money=abc",
        "scene home configure row budget a money=1.234",
        "scene home configure row budget a money=1.",
        "scene home configure row budget a range=7.80",
    ] {
        assert!(
            matches!(edit(line), Err(CommandError::BadChoice { .. })),
            "{line}"
        );
    }
    assert_eq!(
        edit("scene home configure row budget a money=1 text=x"),
        Err(CommandError::Usage(CONFIGURE_USAGE))
    );
    assert_eq!(
        edit("scene home configure row budget a"),
        Err(CommandError::Usage(CONFIGURE_USAGE))
    );
    assert!(matches!(
        edit("scene home configure row budget a colour=red"),
        Err(CommandError::BadChoice { .. })
    ));
    assert!(matches!(
        edit("scene home configure row budget a info=maybe"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn rows_are_added_removed_and_moved_by_key_and_position() {
    assert_eq!(
        edit("scene home configure row-add rules luck kind=slider label=Luck level=40 at=1")
            .expect("reads"),
        ConfigureEdit::RowAdd {
            card: "rules".to_owned(),
            row: "luck".to_owned(),
            kind: RowKind::Slider,
            edit: RowEdit {
                label: Some("Luck".to_owned()),
                level: Some(40),
                ..RowEdit::default()
            },
            at: Some(1),
        }
    );
    assert_eq!(
        edit("scene home configure row-remove rules luck").expect("reads"),
        ConfigureEdit::RowRemove {
            card: "rules".to_owned(),
            row: "luck".to_owned()
        }
    );
    assert_eq!(
        edit("scene home configure row-move rules luck 3").expect("reads"),
        ConfigureEdit::RowMove {
            card: "rules".to_owned(),
            row: "luck".to_owned(),
            position: 3
        }
    );
    assert_eq!(
        edit("scene home configure row-add rules luck label=Luck"),
        Err(CommandError::Usage(CONFIGURE_USAGE))
    );
    assert!(matches!(
        edit("scene home configure row-add rules luck kind=gauge"),
        Err(CommandError::BadChoice { .. })
    ));
}

#[test]
fn clear_takes_the_whole_panel_away_and_other_words_say_how_to_use_it() {
    assert_eq!(
        edit("scene home configure clear").expect("reads"),
        ConfigureEdit::Clear {}
    );
    for line in [
        "scene home configure",
        "scene home configure clear now",
        "scene home configure wobble",
    ] {
        let words = split_words(line).expect("the line splits");
        assert_eq!(
            parse(&words[3..]),
            Err(CommandError::Usage(CONFIGURE_USAGE)),
            "{line}"
        );
    }
}
