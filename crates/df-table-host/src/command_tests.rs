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
