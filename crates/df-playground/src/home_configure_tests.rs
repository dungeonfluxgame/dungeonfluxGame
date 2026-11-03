//! The settings panel's data and live changes: the bounds, what each card draws, and that every
//! change is all or nothing.

use crate::home_configure::{
    Accent, CardKind, ConfigureData, ConfigureEdit, ConfigureError, ConfigureIcon, MAXIMUM_CARDS,
    MAXIMUM_MINOR_UNITS, PanelLayout, Row, RowEdit, RowKind, SliderUnit, StatValue, money_text,
};
use crate::overlay::Edit;

/// The panel as render 19 draws it, with the extra rows the session-settings renders list.
const PANEL: &str = r#"{
  "title": "Configure Session",
  "currency": "$",
  "presets": {"label": "Presets:", "value": "Balanced Adventure", "options": ["Balanced Adventure", "Story First"]},
  "cards": [
    {"key": "world", "kind": "select", "label": "World / Setting", "icon": "globe", "banner": "world.webp",
     "rows": [{"row": "select", "key": "setting", "label": "World / Setting", "value": "Arcane Sci-Fi", "options": ["Medieval Fantasy", "Arcane Sci-Fi"]}]},
    {"key": "party", "kind": "select", "label": "Party", "icon": "party", "accent": "violet",
     "rows": [{"row": "select", "key": "players", "label": "Party", "value": "4 Players (Local)", "options": ["4 Players (Local)", "6 Players (Local)"]},
              {"row": "select", "key": "length", "label": "Session Length", "value": "~3-4 Hours", "options": ["~3-4 Hours"]}]},
    {"key": "rules", "kind": "sliders", "label": "Rules & Gameplay", "icon": "swords",
     "rows": [{"row": "slider", "key": "rules_fidelity", "label": "Rules Fidelity", "value": 80},
              {"row": "slider", "key": "combat_intensity", "label": "Combat Intensity", "value": 60},
              {"row": "slider", "key": "narrative_freedom", "label": "Narrative Freedom", "value": 70},
              {"row": "slider", "key": "difficulty", "label": "Difficulty", "value": 65, "unit": "steps", "steps": ["Relaxed", "Moderate", "Challenging", "Punishing"]}]},
    {"key": "ai", "kind": "mixed", "label": "AI & Narration", "icon": "core",
     "rows": [{"row": "select", "key": "voice", "label": "Narrator Voice", "value": "Aria (Epic)", "options": ["Aria (Epic)"]},
              {"row": "slider", "key": "imagination", "label": "Imagination Level", "value": 80},
              {"row": "select", "key": "speed", "label": "Response Speed", "value": "Balanced", "options": ["Fast", "Balanced"]}]},
    {"key": "budget", "kind": "budget", "label": "Session Budget", "icon": "coins", "accent": "green",
     "rows": [{"row": "stat", "key": "balance", "label": "Balance", "value": {"kind": "money", "cents": 2840}},
              {"row": "stat", "key": "rate", "label": "Spend Rate", "info": true, "value": {"kind": "per_hour", "cents": 390}},
              {"row": "stat", "key": "unknown", "label": "Unknown"},
              {"row": "stat", "key": "estimate", "label": "Estimated (2-3 hrs)", "highlight": true, "value": {"kind": "range", "low_cents": 780, "high_cents": 1170}}]}
  ]
}"#;

fn panel() -> ConfigureData {
    serde_json::from_str(PANEL).expect("the panel reads")
}

fn changed(edit: ConfigureEdit, data: &ConfigureData) -> Result<ConfigureData, ConfigureError> {
    let mut slot = Some(data.clone());
    edit.apply(&mut slot)?;
    Ok(slot.expect("the panel stays"))
}

fn refused(edit: ConfigureEdit) -> ConfigureError {
    let before = panel();
    let mut slot = Some(before.clone());
    let error = edit.apply(&mut slot).expect_err("refused");
    assert_eq!(
        slot,
        Some(before),
        "a refused change leaves the panel as it was"
    );
    error
}

fn slider_row(card: &str, row: &str, edit: RowEdit) -> ConfigureEdit {
    ConfigureEdit::Row {
        card: card.to_owned(),
        row: row.to_owned(),
        edit,
    }
}

#[test]
fn the_panel_reads_checks_and_round_trips() {
    let data = panel();
    data.check().expect("in range");
    assert_eq!(data.cards.len(), 5);
    let again: ConfigureData =
        serde_json::from_str(&serde_json::to_string(&data).expect("writes")).expect("reads");
    assert_eq!(again, data);
    // A key the panel does not know is a mistake in the file.
    assert!(serde_json::from_str::<ConfigureData>(r#"{"colour": "red"}"#).is_err());
    // An empty file is a panel with the default title and no cards.
    let empty: ConfigureData = serde_json::from_str("{}").expect("reads");
    empty.check().expect("in range");
    assert!(empty.cards.is_empty() && empty.presets.is_none());
}

#[test]
fn sliders_read_out_a_percent_or_the_step_they_fall_on() {
    let data = panel();
    let rules = data.card("rules").expect("a card");
    let reads: Vec<String> = rules
        .rows
        .iter()
        .map(|row| match row {
            Row::Slider(slider) => slider.read_out(),
            other => panic!("a slider, got {other:?}"),
        })
        .collect();
    assert_eq!(reads, ["80%", "60%", "70%", "Challenging"]);
    // Four steps split the track in thirds: 0 is the first, 100 the last.
    let step = |value: u8| {
        let mut data = data.clone();
        let Some(Row::Slider(slider)) = data.cards[2].rows.last_mut() else {
            panic!("a slider");
        };
        slider.value = value;
        slider.read_out()
    };
    assert_eq!(
        [step(0), step(16), step(17), step(50), step(84), step(100)],
        [
            "Relaxed",
            "Relaxed",
            "Moderate",
            "Challenging",
            "Punishing",
            "Punishing"
        ]
    );
}

#[test]
fn money_is_set_out_with_the_sign_and_hundredths_and_only_when_present() {
    assert_eq!(money_text(2840, "$"), "$28.40");
    assert_eq!(money_text(5, "$"), "$0.05");
    assert_eq!(money_text(123_456_789, "$"), "$1,234,567.89");
    assert_eq!(StatValue::PerHour { cents: 390 }.text("$"), "$3.90 / hr");
    assert_eq!(
        StatValue::Range {
            low_cents: 780,
            high_cents: 1170
        }
        .text("$"),
        "$7.80 \u{2013} $11.70"
    );
    assert_eq!(
        StatValue::Text {
            text: "Free".into()
        }
        .text("$"),
        "Free"
    );
    // The budget card draws three of its four rows: the one with no value is left out.
    let data = panel();
    let budget = data.card("budget").expect("a card");
    let drawn: Vec<&str> = budget.drawn_rows().iter().map(|row| row.key()).collect();
    assert_eq!(drawn, ["balance", "rate", "estimate"]);
}

#[test]
fn a_card_draws_the_first_rows_and_keeps_the_rest_in_the_data() {
    let data = panel();
    let keys = |card: &str| -> Vec<String> {
        data.card(card)
            .expect("a card")
            .drawn_rows()
            .iter()
            .map(|row| row.key().to_owned())
            .collect()
    };
    // A select card draws its first dropdown only.
    assert_eq!(keys("party"), ["players"]);
    // A sliders card draws `shown_rows` (3) of its four.
    assert_eq!(
        keys("rules"),
        ["rules_fidelity", "combat_intensity", "narrative_freedom"]
    );
    assert_eq!(data.card("rules").expect("a card").rows.len(), 4);
    assert_eq!(keys("ai"), ["voice", "imagination", "speed"]);
    let more = changed(
        ConfigureEdit::Card {
            card: "rules".to_owned(),
            label: None,
            icon: Edit::Keep,
            banner: Edit::Keep,
            accent: None,
            shown_rows: Some(4),
        },
        &data,
    )
    .expect("applies");
    assert_eq!(more.card("rules").expect("a card").drawn_rows().len(), 4);
}

#[test]
fn every_bound_of_the_data_is_checked() {
    let with = |change: &dyn Fn(&mut ConfigureData)| {
        let mut data = panel();
        change(&mut data);
        data.check().expect_err("out of range")
    };
    assert_eq!(
        with(&|d| d.title = " ".into()),
        ConfigureError::Empty("the panel title")
    );
    assert_eq!(
        with(&|d| d.currency.clear()),
        ConfigureError::Empty("the currency sign")
    );
    assert_eq!(
        with(&|d| d.cards[0].key = "has space".into()),
        ConfigureError::BadKey("has space".into())
    );
    assert_eq!(
        with(&|d| d.cards[1].key = "world".into()),
        ConfigureError::DuplicateKey {
            what: "the card",
            key: "world".into()
        }
    );
    assert_eq!(
        with(&|d| d.cards[0].shown_rows = 0),
        ConfigureError::BadShownRows
    );
    assert_eq!(
        with(&|d| d.cards[0].banner = Some(" ".into())),
        ConfigureError::Empty("a banner picture name")
    );
    assert_eq!(
        with(&|d| {
            if let Row::Slider(slider) = &mut d.cards[2].rows[0] {
                slider.value = 101;
            }
        }),
        ConfigureError::BadPercent {
            row: "rules_fidelity".into(),
            value: 101
        }
    );
    assert_eq!(
        with(&|d| {
            if let Row::Slider(slider) = &mut d.cards[2].rows[3] {
                slider.steps = vec!["One".into()];
            }
        }),
        ConfigureError::TooFewSteps("difficulty".into())
    );
    assert_eq!(
        with(&|d| {
            if let Row::Select(select) = &mut d.cards[0].rows[0] {
                select.value = "Dream".into();
            }
        }),
        ConfigureError::NotAnOption {
            row: "setting".into(),
            value: "Dream".into()
        }
    );
    assert_eq!(
        with(&|d| d.presets.as_mut().expect("presets").value = "Nothing".into()),
        ConfigureError::NotAnOption {
            row: "the presets".into(),
            value: "Nothing".into()
        }
    );
    assert_eq!(
        with(&|d| {
            let row = d.cards[0].rows[0].clone();
            d.cards[2].rows.push(row);
        }),
        ConfigureError::RowNotAllowed {
            card: "rules".into(),
            kind: CardKind::Sliders,
            row: "setting".into()
        }
    );
    assert_eq!(
        with(&|d| {
            if let Row::Stat(stat) = &mut d.cards[4].rows[3] {
                stat.value = Some(StatValue::Range {
                    low_cents: 2000,
                    high_cents: 100,
                });
            }
        }),
        ConfigureError::BadRange {
            low: 2000,
            high: 100
        }
    );
    assert_eq!(
        with(&|d| {
            if let Row::Stat(stat) = &mut d.cards[4].rows[0] {
                stat.value = Some(StatValue::Money {
                    cents: MAXIMUM_MINOR_UNITS + 1,
                });
            }
        }),
        ConfigureError::BadAmount(MAXIMUM_MINOR_UNITS + 1)
    );
    assert!(matches!(
        with(&|d| d.cards = vec![d.cards[0].clone(); MAXIMUM_CARDS + 1]),
        ConfigureError::TooMany { .. }
    ));
}

#[test]
fn the_panel_and_preset_changes_apply_and_a_panel_is_made_when_there_is_none() {
    let data = panel();
    let after = changed(
        ConfigureEdit::Panel {
            title: Some("Tonight".into()),
            currency: Some("\u{a3}".into()),
            layout: Some(PanelLayout::Tiles),
        },
        &data,
    )
    .expect("applies");
    assert_eq!(
        (after.title.as_str(), after.currency.as_str()),
        ("Tonight", "\u{a3}")
    );
    assert_eq!(after.layout, PanelLayout::Tiles);
    // A tile is one focus target: its rows are left to the data and the CLI.
    assert_eq!(
        after.focus_ids(),
        [
            "configure:preset",
            "configure:1",
            "configure:2",
            "configure:3",
            "configure:4",
            "configure:5"
        ]
    );
    let after = changed(
        ConfigureEdit::Preset {
            label: None,
            value: Some("Story First".into()),
            options: None,
        },
        &data,
    )
    .expect("applies");
    assert_eq!(after.presets.expect("presets").value, "Story First");
    let mut slot = None;
    ConfigureEdit::Preset {
        label: None,
        value: Some("Quick".into()),
        options: Some(vec!["Quick".into()]),
    }
    .apply(&mut slot)
    .expect("makes the panel");
    let made = slot.expect("a panel");
    assert_eq!(made.title, "Configure Session");
    assert_eq!(made.presets.expect("presets").label, "Presets:");
    assert!(
        changed(ConfigureEdit::PresetRemove {}, &data)
            .expect("applies")
            .presets
            .is_none()
    );
    assert_eq!(
        refused(ConfigureEdit::Panel {
            title: None,
            currency: None,
            layout: None
        }),
        ConfigureError::NoChange
    );
    assert_eq!(
        refused(ConfigureEdit::Preset {
            label: None,
            value: Some("Nothing".into()),
            options: None
        }),
        ConfigureError::NotAnOption {
            row: "the presets".into(),
            value: "Nothing".into()
        }
    );
}

#[test]
fn a_change_with_no_panel_is_refused_unless_it_makes_one() {
    for edit in [
        ConfigureEdit::PresetRemove {},
        ConfigureEdit::CardRemove {
            card: "world".into(),
        },
        ConfigureEdit::Row {
            card: "a".into(),
            row: "b".into(),
            edit: RowEdit {
                level: Some(1),
                ..RowEdit::default()
            },
        },
    ] {
        let mut slot = None;
        assert_eq!(edit.apply(&mut slot), Err(ConfigureError::NoPanel));
        assert_eq!(slot, None);
    }
    let mut slot = None;
    ConfigureEdit::CardAdd {
        card: "first".into(),
        kind: CardKind::Sliders,
        label: "First".into(),
        icon: None,
        banner: None,
        accent: None,
        shown_rows: None,
        at: None,
    }
    .apply(&mut slot)
    .expect("makes the panel");
    assert_eq!(slot.expect("a panel").cards.len(), 1);
    let mut slot = Some(panel());
    ConfigureEdit::Clear {}.apply(&mut slot).expect("clears");
    assert_eq!(slot, None);
}

#[test]
fn a_card_is_changed_added_moved_and_removed_by_key() {
    let data = panel();
    let after = changed(
        ConfigureEdit::Card {
            card: "world".into(),
            label: Some("Setting".into()),
            icon: Edit::Set(ConfigureIcon::Dice),
            banner: Edit::Clear,
            accent: Some(Accent::Green),
            shown_rows: Some(2),
        },
        &data,
    )
    .expect("applies");
    let card = after.card("world").expect("a card");
    assert_eq!(
        (
            card.label.as_str(),
            card.icon,
            card.banner.as_deref(),
            card.accent,
            card.shown_rows
        ),
        ("Setting", Some(ConfigureIcon::Dice), None, Accent::Green, 2)
    );
    let after = changed(
        ConfigureEdit::CardAdd {
            card: "extras".into(),
            kind: CardKind::Mixed,
            label: "Extras".into(),
            icon: Some(ConfigureIcon::Book),
            banner: None,
            accent: None,
            shown_rows: None,
            at: Some(1),
        },
        &data,
    )
    .expect("applies");
    let order = |data: &ConfigureData| -> Vec<String> {
        data.cards.iter().map(|card| card.key.clone()).collect()
    };
    assert_eq!(
        order(&after),
        ["extras", "world", "party", "rules", "ai", "budget"]
    );
    let after = changed(
        ConfigureEdit::CardMove {
            card: "extras".into(),
            position: 6,
        },
        &after,
    )
    .expect("applies");
    assert_eq!(
        order(&after),
        ["world", "party", "rules", "ai", "budget", "extras"]
    );
    let after = changed(
        ConfigureEdit::CardRemove {
            card: "extras".into(),
        },
        &after,
    )
    .expect("applies");
    assert_eq!(order(&after), ["world", "party", "rules", "ai", "budget"]);
    assert_eq!(
        refused(ConfigureEdit::CardRemove {
            card: "nope".into()
        }),
        ConfigureError::UnknownCard("nope".into())
    );
    assert_eq!(
        refused(ConfigureEdit::CardMove {
            card: "world".into(),
            position: 0
        }),
        ConfigureError::BadPosition {
            position: 0,
            count: 5
        }
    );
    assert_eq!(
        refused(ConfigureEdit::CardMove {
            card: "world".into(),
            position: 6
        }),
        ConfigureError::BadPosition {
            position: 6,
            count: 5
        }
    );
    assert_eq!(
        refused(ConfigureEdit::CardAdd {
            card: "world".into(),
            kind: CardKind::Select,
            label: "Again".into(),
            icon: None,
            banner: None,
            accent: None,
            shown_rows: None,
            at: None
        }),
        ConfigureError::DuplicateKey {
            what: "the card",
            key: "world".into()
        }
    );
    assert_eq!(
        refused(ConfigureEdit::CardAdd {
            card: "late".into(),
            kind: CardKind::Select,
            label: "Late".into(),
            icon: None,
            banner: None,
            accent: None,
            shown_rows: None,
            at: Some(7)
        }),
        ConfigureError::BadPosition {
            position: 7,
            count: 6
        }
    );
}

#[test]
fn rows_are_changed_added_moved_and_removed_and_a_field_must_fit_the_row() {
    let data = panel();
    let after = changed(
        slider_row(
            "rules",
            "difficulty",
            RowEdit {
                level: Some(100),
                steps: Some(vec!["Easy".into(), "Hard".into()]),
                ..RowEdit::default()
            },
        ),
        &data,
    )
    .expect("applies");
    let Some(Row::Slider(slider)) = after.card("rules").and_then(|card| card.rows.last()) else {
        panic!("a slider");
    };
    assert_eq!(slider.read_out(), "Hard");
    let after = changed(
        slider_row(
            "budget",
            "rate",
            RowEdit {
                stat: Edit::Set(StatValue::PerHour { cents: 450 }),
                info: Some(false),
                ..RowEdit::default()
            },
        ),
        &data,
    )
    .expect("applies");
    let Some(Row::Stat(stat)) = after.card("budget").map(|card| &card.rows[1]) else {
        panic!("a stat");
    };
    assert_eq!(
        (stat.value.clone(), stat.info),
        (Some(StatValue::PerHour { cents: 450 }), false)
    );
    let after = changed(
        slider_row(
            "budget",
            "balance",
            RowEdit {
                stat: Edit::Clear,
                ..RowEdit::default()
            },
        ),
        &after,
    )
    .expect("applies");
    let drawn = after.card("budget").expect("a card").drawn_rows().len();
    assert_eq!(drawn, 2, "a stat row with no value is not drawn");

    let added = changed(
        ConfigureEdit::RowAdd {
            card: "rules".into(),
            row: "luck".into(),
            kind: RowKind::Slider,
            edit: RowEdit {
                label: Some("Luck".into()),
                level: Some(40),
                ..RowEdit::default()
            },
            at: Some(1),
        },
        &data,
    )
    .expect("applies");
    let keys = |data: &ConfigureData| -> Vec<String> {
        data.card("rules")
            .expect("a card")
            .rows
            .iter()
            .map(|row| row.key().to_owned())
            .collect()
    };
    assert_eq!(keys(&added)[0], "luck");
    let moved = changed(
        ConfigureEdit::RowMove {
            card: "rules".into(),
            row: "luck".into(),
            position: 5,
        },
        &added,
    )
    .expect("applies");
    assert_eq!(keys(&moved).last().map(String::as_str), Some("luck"));
    let removed = changed(
        ConfigureEdit::RowRemove {
            card: "rules".into(),
            row: "luck".into(),
        },
        &moved,
    )
    .expect("applies");
    assert_eq!(keys(&removed), keys(&data));

    assert_eq!(
        refused(slider_row(
            "rules",
            "rules_fidelity",
            RowEdit {
                value: Some("x".into()),
                ..RowEdit::default()
            }
        )),
        ConfigureError::FieldNotForRow {
            field: "value",
            row: "rules_fidelity".into(),
            expected: "slider"
        }
    );
    assert_eq!(
        refused(slider_row(
            "world",
            "setting",
            RowEdit {
                level: Some(5),
                ..RowEdit::default()
            }
        )),
        ConfigureError::FieldNotForRow {
            field: "level",
            row: "setting".into(),
            expected: "select"
        }
    );
    assert_eq!(
        refused(slider_row("rules", "rules_fidelity", RowEdit::default())),
        ConfigureError::NoChange
    );
    assert_eq!(
        refused(slider_row(
            "rules",
            "nope",
            RowEdit {
                level: Some(1),
                ..RowEdit::default()
            }
        )),
        ConfigureError::UnknownRow {
            card: "rules".into(),
            row: "nope".into()
        }
    );
    assert_eq!(
        refused(slider_row(
            "rules",
            "rules_fidelity",
            RowEdit {
                level: Some(150),
                ..RowEdit::default()
            }
        )),
        ConfigureError::BadPercent {
            row: "rules_fidelity".into(),
            value: 150
        }
    );
    assert_eq!(
        refused(slider_row(
            "rules",
            "rules_fidelity",
            RowEdit {
                unit: Some(SliderUnit::Steps),
                ..RowEdit::default()
            }
        )),
        ConfigureError::TooFewSteps("rules_fidelity".into())
    );
    // A stat row cannot join a sliders card.
    assert_eq!(
        refused(ConfigureEdit::RowAdd {
            card: "rules".into(),
            row: "cost".into(),
            kind: RowKind::Stat,
            edit: RowEdit {
                label: Some("Cost".into()),
                ..RowEdit::default()
            },
            at: None
        }),
        ConfigureError::RowNotAllowed {
            card: "rules".into(),
            kind: CardKind::Sliders,
            row: "cost".into()
        }
    );
}

#[test]
fn the_audit_line_names_the_field_and_the_keys_but_never_free_text() {
    let (field, pairs) = ConfigureEdit::Row {
        card: "rules".into(),
        row: "combat_intensity".into(),
        edit: RowEdit {
            label: Some("Secret words".into()),
            level: Some(60),
            ..RowEdit::default()
        },
    }
    .audit_fields();
    assert_eq!(field, "row");
    assert!(pairs.contains(&("card", "rules".to_owned())));
    assert!(pairs.contains(&("row", "combat_intensity".to_owned())));
    assert!(pairs.contains(&("fields", "label,level".to_owned())));
    assert!(pairs.contains(&("level", "60".to_owned())));
    assert!(pairs.iter().all(|(_, value)| !value.contains("Secret")));
    let (field, pairs) = ConfigureEdit::CardMove {
        card: "ai".into(),
        position: 2,
    }
    .audit_fields();
    assert_eq!(field, "card_move");
    assert_eq!(
        pairs,
        [("card", "ai".to_owned()), ("position", "2".to_owned())]
    );
    assert_eq!(
        ConfigureEdit::Clear {}.audit_fields(),
        ("clear", Vec::new())
    );
}

#[test]
fn the_banners_of_the_data_and_of_a_change_are_named_for_the_host_to_check() {
    assert_eq!(panel().media_names(), ["world.webp"]);
    let set = ConfigureEdit::Card {
        card: "world".into(),
        label: None,
        icon: Edit::Keep,
        banner: Edit::Set("b.webp".into()),
        accent: None,
        shown_rows: None,
    };
    assert_eq!(set.media_names(), ["b.webp"]);
    let clear = ConfigureEdit::Card {
        card: "world".into(),
        label: None,
        icon: Edit::Keep,
        banner: Edit::Clear,
        accent: None,
        shown_rows: None,
    };
    assert!(clear.media_names().is_empty());
}

#[test]
fn every_icon_has_a_name_that_reads_back_and_a_drawing() {
    for icon in ConfigureIcon::ALL {
        assert_eq!(ConfigureIcon::from_key(icon.key()), Some(icon));
        assert!(icon.markup().starts_with('<'), "{}", icon.key());
    }
    assert_eq!(ConfigureIcon::from_key("banana"), None);
    assert!(ConfigureIcon::keys_text().contains("coins"));
}

#[test]
fn a_change_reads_from_and_writes_to_json_the_way_the_host_sends_it() {
    let edit = ConfigureEdit::Row {
        card: "rules".into(),
        row: "luck".into(),
        edit: RowEdit {
            level: Some(10),
            stat: Edit::Set(StatValue::Money { cents: 5 }),
            ..RowEdit::default()
        },
    };
    let text = serde_json::to_string(&edit).expect("writes");
    assert!(text.contains(r#""action":"row""#), "{text}");
    assert_eq!(
        serde_json::from_str::<ConfigureEdit>(&text).expect("reads"),
        edit
    );
    assert!(
        serde_json::from_str::<ConfigureEdit>(
            r#"{"action":"row","card":"a","row":"b","colour":1}"#
        )
        .is_err()
    );
    assert!(serde_json::from_str::<ConfigureEdit>(r#"{"action":"wobble"}"#).is_err());
}

#[test]
fn every_card_row_and_the_preset_selector_has_a_focus_id_the_page_can_look_up() {
    let ids = panel().focus_ids();
    assert_eq!(
        ids,
        [
            "configure:preset",
            "configure:1",
            "configure:2",
            "configure:3",
            "configure:3:1",
            "configure:3:2",
            "configure:3:3",
            "configure:4",
            "configure:4:1",
            "configure:4:2",
            "configure:4:3",
            "configure:5",
            "configure:5:1",
            "configure:5:2",
            "configure:5:3",
        ]
    );
    assert!(ids.iter().all(|id| crate::home_model::is_focus_id(id)));
    // No preset selector, no preset id.
    let mut data = panel();
    data.presets = None;
    assert_eq!(
        data.focus_ids().first().map(String::as_str),
        Some("configure:1")
    );
}

#[test]
fn a_slider_lights_the_segments_of_its_value_to_the_nearest_tenth() {
    let lit = |value: u8| {
        let mut data = panel();
        let Row::Slider(slider) = &mut data.cards[2].rows[0] else {
            panic!("a slider");
        };
        slider.value = value;
        slider.lit_segments()
    };
    assert_eq!(
        [lit(0), lit(4), lit(5), lit(80), lit(94), lit(95), lit(100)],
        [0, 0, 1, 8, 9, 10, 10]
    );
}
