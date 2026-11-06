use crate::command::{Command, CommandError, parse, parse_script_line, split_words, usage_text};
use crate::wire::{EffectRequest, ValueForm};

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("the line splits")
}

fn effects(line: &str) -> Vec<EffectRequest> {
    let Command::Effects(requests) = parse(&words(line)).expect("parses") else {
        panic!("expected effects");
    };
    requests
}

#[test]
fn a_quoted_value_with_spaces_stays_one_word() {
    assert_eq!(
        words(r#"spawn character pip name="Pip the Brave" x=20"#),
        vec!["spawn", "character", "pip", "name=Pip the Brave", "x=20"]
    );
}

#[test]
fn an_unclosed_quote_is_refused() {
    assert_eq!(
        split_words(r#"spawn character pip name="Pip"#),
        Err(CommandError::UnclosedQuote)
    );
}

#[test]
fn spawn_puts_position_in_the_request_and_everything_else_in_fields_as_text() {
    let requests = effects("spawn character pip x=20 y=30 name=Pip hit_points=7");
    let EffectRequest::Spawn {
        id,
        kind,
        x,
        y,
        fields,
        values,
    } = &requests[0]
    else {
        panic!("expected a spawn");
    };
    assert_eq!((id.as_str(), kind.as_str()), ("pip", "character"));
    assert_eq!((*x, *y), (Some(20.0), Some(30.0)));
    assert_eq!(fields.get("hit_points"), Some(&serde_json::json!("7")));
    assert_eq!(*values, ValueForm::Text);
}

#[test]
fn spawn_without_a_kind_and_an_id_shows_its_usage() {
    assert!(matches!(
        parse(&words("spawn character")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn a_position_that_is_not_a_finite_number_is_refused_by_name() {
    for bad in ["left", "nan", "NaN", "inf", "-inf", "1e999"] {
        assert_eq!(
            parse(&words(&format!("spawn character pip x={bad}"))),
            Err(CommandError::BadNumber {
                what: "x",
                text: bad.into()
            }),
            "{bad}"
        );
    }
    assert!(matches!(
        parse(&words("move pip nan 5")),
        Err(CommandError::BadNumber { .. })
    ));
}

#[test]
fn a_field_without_an_equals_sign_is_refused() {
    assert_eq!(
        parse(&words("spawn character pip name")),
        Err(CommandError::BadPair("name".into()))
    );
}

#[test]
fn a_key_given_twice_is_refused_instead_of_keeping_the_last() {
    assert_eq!(
        parse(&words("spawn character pip name=a name=b")),
        Err(CommandError::DuplicateKey("name".into()))
    );
    assert_eq!(
        parse(&words("spawn character pip x=1 x=2")),
        Err(CommandError::DuplicateKey("x".into()))
    );
}

#[test]
fn set_sends_one_request_per_pair() {
    let requests = effects("set pip hit_points=3 name=Pippin");
    assert_eq!(requests.len(), 2);
    assert!(matches!(&requests[0], EffectRequest::Set { field, .. } if field == "hit_points"));
    assert!(matches!(&requests[1], EffectRequest::Set { value, .. } if value == "Pippin"));
    assert!(matches!(
        parse(&words("set pip")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn move_needs_an_id_and_two_numbers() {
    assert_eq!(
        effects("move pip 10 20"),
        vec![EffectRequest::Move {
            id: "pip".into(),
            x: 10.0,
            y: 20.0
        }]
    );
    assert!(matches!(
        parse(&words("move pip 10")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn despawn_takes_several_ids_and_clear_takes_none() {
    assert_eq!(effects("despawn a b c").len(), 3);
    assert_eq!(effects("clear"), vec![EffectRequest::Clear {}]);
    assert!(matches!(
        parse(&words("despawn")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn events_may_name_the_last_line_already_seen_or_ask_for_the_newest() {
    assert_eq!(parse(&words("events")), Ok(Command::Events { after: None }));
    assert_eq!(
        parse(&words("events 12")),
        Ok(Command::Events { after: Some(12) })
    );
    assert!(matches!(
        parse(&words("events soon")),
        Err(CommandError::BadNumber { .. })
    ));
    assert!(matches!(
        parse(&words("events 1 2")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn the_plain_commands_parse() {
    assert_eq!(parse(&words("canvas")), Ok(Command::Canvas));
    assert_eq!(parse(&words("kinds")), Ok(Command::Kinds));
    assert_eq!(parse(&words("reload")), Ok(Command::Reload));
    assert_eq!(parse(&words("help")), Ok(Command::Help));
    assert!(matches!(
        parse(&words("script a.txt")),
        Ok(Command::Script(_))
    ));
    assert!(matches!(
        parse(&words("script")),
        Err(CommandError::Usage(_))
    ));
}

#[test]
fn an_unknown_word_and_an_empty_line_are_refused() {
    assert_eq!(
        parse(&words("dance")),
        Err(CommandError::Unknown("dance".into()))
    );
    assert_eq!(parse(&[]), Err(CommandError::Empty));
}

#[test]
fn a_script_line_may_not_run_another_script_or_ask_for_help() {
    assert_eq!(
        parse_script_line(&words("script other.txt")),
        Err(CommandError::NotAllowedInScript("script"))
    );
    assert_eq!(
        parse_script_line(&words("help")),
        Err(CommandError::NotAllowedInScript("help"))
    );
    assert!(parse_script_line(&words("kinds")).is_ok());
}

#[test]
fn the_help_text_names_every_command_and_the_rules_of_a_multi_effect_command() {
    let help = usage_text();
    for name in [
        "spawn", "set", "move", "despawn", "clear", "canvas", "kinds", "events", "reload",
        "script", "help",
    ] {
        assert!(help.contains(name), "{name}");
    }
    assert!(help.contains("applied in order and every failure is reported"));
    assert!(help.contains("a script stops at its first failing line"));
}

fn playground_changes(line: &str) -> Vec<df_playground::controls::ControlsChange> {
    let Command::PlaygroundSet(changes) = parse(&words(line)).expect("parses") else {
        panic!("expected playground changes");
    };
    changes
}

#[test]
fn playground_alone_shows_and_set_reads_each_key_by_its_choices() {
    use df_playground::controls::{ControlsChange, Device};
    use df_playground::copy::ReadingPace;
    assert_eq!(parse(&words("playground")), Ok(Command::Playground));
    assert_eq!(
        playground_changes(
            "playground set fixture=a.json device=tv pace=slow status=rolling unavailable=true grid=off"
        ),
        vec![
            ControlsChange::Fixture("a.json".into()),
            ControlsChange::Device(Device::Tv),
            ControlsChange::Pace(ReadingPace::Slow),
            ControlsChange::Status("rolling".into()),
            ControlsChange::ShowUnavailable(true),
            ControlsChange::ShowGrid(false),
        ]
    );
}

#[test]
fn a_playground_value_that_is_not_a_choice_names_the_key_and_the_choices() {
    for (line, key) in [
        ("playground set device=watch", "device"),
        ("playground set pace=sleepy", "pace"),
        ("playground set grid=maybe", "grid"),
        ("playground set unavailable=2", "unavailable"),
    ] {
        let error = parse(&words(line)).expect_err("refused");
        assert!(
            matches!(&error, CommandError::BadChoice { key: found, .. } if found == key),
            "{line}"
        );
    }
    let unknown = parse(&words("playground set volume=3")).expect_err("refused");
    assert!(unknown.to_string().contains("fixture=<name>"));
}

#[test]
fn playground_set_needs_pairs_and_refuses_a_key_given_twice() {
    assert!(matches!(
        parse(&words("playground set")),
        Err(CommandError::Usage(_))
    ));
    assert!(matches!(
        parse(&words("playground show")),
        Err(CommandError::Usage(_))
    ));
    assert_eq!(
        parse(&words("playground set grid=on grid=off")),
        Err(CommandError::DuplicateKey("grid".into()))
    );
}

#[test]
fn fixture_commands_take_a_name_and_for_write_a_file() {
    assert_eq!(parse(&words("fixtures")), Ok(Command::Fixtures));
    assert_eq!(
        parse(&words("fixture show tavern")),
        Ok(Command::FixtureShow("tavern".into()))
    );
    assert_eq!(
        parse(&words("fixture remove tavern")),
        Ok(Command::FixtureRemove("tavern".into()))
    );
    assert_eq!(
        parse(&words("fixture write tavern scene.json")),
        Ok(Command::FixtureWrite {
            name: "tavern".into(),
            path: "scene.json".into()
        })
    );
    for line in [
        "fixture",
        "fixture show",
        "fixture write tavern",
        "fixture burn tavern",
        "fixtures all",
    ] {
        assert!(
            matches!(parse(&words(line)), Err(CommandError::Usage(_))),
            "{line}"
        );
    }
}

#[test]
fn the_help_text_lists_the_playground_and_fixture_commands() {
    let help = usage_text();
    for text in [
        "playground",
        "fixtures",
        "fixture show",
        "device=both|tv|phone",
    ] {
        assert!(help.contains(text), "{text}");
    }
}

#[test]
fn repl_clients_and_fixtures_with_stray_words_answer_with_their_usage_not_unknown_command() {
    for (line, usage) in [
        ("repl now", "repl"),
        ("repl --port 7878", "repl"),
        ("clients all", "clients"),
        ("fixtures x", "fixtures"),
    ] {
        assert_eq!(
            parse(&words(line)),
            Err(CommandError::Usage(usage)),
            "{line}"
        );
    }
    assert_eq!(parse(&words("repl")), Ok(Command::Repl));
    assert_eq!(parse(&words("clients")), Ok(Command::Clients));
    assert_eq!(parse(&words("fixtures")), Ok(Command::Fixtures));
}

#[test]
fn the_help_text_and_the_errors_are_built_from_the_limit_constants() {
    use df_playground::limits::{
        MAXIMUM_CAMERA_MS, MAXIMUM_TRANSITION_MS, MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT,
    };
    let help = usage_text();
    assert!(
        help.contains(&format!("ms=0-{MAXIMUM_TRANSITION_MS}")),
        "{help}"
    );
    assert!(
        help.contains(&format!(
            "{MINIMUM_ZOOM_PERCENT} to {MAXIMUM_ZOOM_PERCENT} percent, over is 0 to {MAXIMUM_CAMERA_MS} ms"
        )),
        "{help}"
    );
    let Err(CommandError::BadChoice { allowed, .. }) = parse(&words(&format!(
        "wait {}",
        u64::from(u32::MAX) * 100_000_000
    ))) else {
        panic!("refused");
    };
    assert!(
        allowed.contains(&crate::limits::MAXIMUM_WAIT_MS.to_string()),
        "{allowed}"
    );
    let Err(CommandError::BadChoice { allowed, .. }) =
        parse(&words("playground cycle a b times=0"))
    else {
        panic!("refused");
    };
    assert!(
        allowed.contains(&crate::limits::MAXIMUM_CYCLES.to_string()),
        "{allowed}"
    );
}

/// Every top-level verb table `parse` dispatches on, with its name for the failure message.
fn verb_tables() -> [(&'static str, &'static [&'static str]); 4] {
    [
        ("command::VERBS", &crate::command::VERBS),
        ("engine::VERBS", &crate::engine::VERBS),
        (
            "scene_builder::cli::VERBS",
            &crate::scene_builder::cli::VERBS,
        ),
        ("director::cli::VERBS", &crate::director::cli::VERBS),
    ]
}

#[test]
fn no_top_level_verb_is_claimed_by_two_verb_tables() {
    let mut owners = std::collections::BTreeMap::new();
    for (table, verbs) in verb_tables() {
        for verb in verbs {
            if let Some(earlier) = owners.insert(*verb, table) {
                panic!("'{verb}' is in both {earlier} and {table}");
            }
        }
    }
    for verb in crate::command::VERBS {
        assert_ne!(
            parse(&words(verb)),
            Err(CommandError::Unknown(verb.to_owned())),
            "{verb} is in command::VERBS but parse does not match it"
        );
    }
}

#[test]
fn the_help_text_starts_an_entry_with_every_top_level_verb() {
    let help = usage_text();
    for (table, verbs) in verb_tables() {
        for verb in verbs {
            let entry_start = format!("\n  {verb}");
            assert!(
                help.contains(&entry_start),
                "{verb} of {table} has no help entry"
            );
        }
    }
}

/// Every sub-verb `scene` accepts.
const SCENE_SUB_VERBS: [&str; 29] = [
    "caption",
    "caption-clear",
    "party",
    "choices",
    "image",
    "image-clear",
    "slide",
    "camera",
    "camera-reset",
    "camera-on",
    "cast",
    "cue",
    "music",
    "music-stop",
    "sound",
    "reset",
    "narration",
    "seat",
    "seats",
    "logo",
    "lobby",
    "loading",
    "roleplay",
    "pill",
    "home",
    "start",
    "build",
    "show",
    "lookahead",
];

#[test]
fn the_scene_usage_names_every_sub_verb_the_scene_parser_accepts() {
    let Err(CommandError::Usage(scene_usage)) = parse(&words("scene")) else {
        panic!("scene alone answers with its usage");
    };
    let named: Vec<&str> = scene_usage
        .trim_start_matches("scene ")
        .split(" | ")
        .filter_map(|entry| entry.split_whitespace().next())
        .collect();
    for verb in SCENE_SUB_VERBS {
        assert!(
            named.contains(&verb),
            "{verb} is missing from: {scene_usage}"
        );
        assert_ne!(
            parse(&words(&format!("scene {verb}"))),
            Err(CommandError::Usage(scene_usage)),
            "scene {verb} is not accepted by the scene parser"
        );
    }
    assert_eq!(named.len(), SCENE_SUB_VERBS.len(), "{named:?}");
}
