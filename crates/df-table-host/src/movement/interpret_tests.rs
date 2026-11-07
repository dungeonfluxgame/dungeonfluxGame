//! Tests of `interpret`.

use crate::movement::interpret::{
    InterpretError, LabelTable, LineReader, ScriptedReader, jev_facts, jev_schema, parse_reading,
    read_scripted, read_scripted_with,
};
use crate::movement::types::{Interpreted, MoveIntent, MoveKind, PartyMember};
use serde_json::{Value, json};

fn member(id: &str, display_name: &str) -> PartyMember {
    PartyMember {
        id: id.to_string(),
        display_name: display_name.to_string(),
    }
}

fn party() -> Vec<PartyMember> {
    vec![
        member("elf", "Lirael"),
        member("warrior", "Brann"),
        member("wizard", "Oren"),
    ]
}

fn moves(kind: MoveKind, who: &[&str], to: &str, line: &str) -> Interpreted {
    Interpreted::Move {
        intent: MoveIntent {
            kind,
            who: who.iter().map(|id| id.to_string()).collect(),
            to: to.to_string(),
            line: line.to_string(),
        },
    }
}

#[test]
fn walk_line_moves_the_whole_party_to_the_square() {
    let line = "we walk to the town square";
    assert_eq!(
        read_scripted(line, &party()),
        Ok(moves(
            MoveKind::Walk,
            &["elf", "warrior", "wizard"],
            "the town square",
            line
        ))
    );
}

#[test]
fn enter_line_keeps_the_article_of_the_destination() {
    let line = "we enter the hidden door";
    assert_eq!(
        read_scripted(line, &party()),
        Ok(moves(
            MoveKind::Enter,
            &["elf", "warrior", "wizard"],
            "the hidden door",
            line
        ))
    );
}

#[test]
fn run_line_drops_the_leading_down() {
    let line = "we run down the hallway";
    assert_eq!(
        read_scripted(line, &party()),
        Ok(moves(
            MoveKind::Run,
            &["elf", "warrior", "wizard"],
            "the hallway",
            line
        ))
    );
}

#[test]
fn go_down_is_a_descent() {
    let line = "we go down the stairs";
    assert_eq!(
        read_scripted(line, &party()),
        Ok(moves(
            MoveKind::Descend,
            &["elf", "warrior", "wizard"],
            "the stairs",
            line
        ))
    );
}

#[test]
fn two_named_characters_move_alone() {
    let line = "the elf and the warrior go out to the balcony";
    assert_eq!(
        read_scripted(line, &party()),
        Ok(moves(
            MoveKind::Walk,
            &["elf", "warrior"],
            "the balcony",
            line
        ))
    );
}

#[test]
fn display_name_names_a_character() {
    let line = "Brann will climb the ladder";
    assert_eq!(
        read_scripted(line, &party()),
        Ok(moves(MoveKind::Climb, &["warrior"], "the ladder", line))
    );
}

#[test]
fn a_name_must_match_whole_words() {
    let line = "the wizardry scholar and the warrior go to the hall";
    assert_eq!(
        read_scripted(line, &party()),
        Ok(moves(MoveKind::Walk, &["warrior"], "the hall", line))
    );
}

#[test]
fn a_line_without_a_movement_verb_is_not_a_move() {
    assert_eq!(
        read_scripted("what is the weather like in the square?", &party()),
        Ok(Interpreted::NotAMove)
    );
}

#[test]
fn a_verb_without_a_destination_needs_clarification() {
    assert_eq!(
        read_scripted("we walk to", &party()),
        Ok(Interpreted::NeedsClarification)
    );
    assert_eq!(
        read_scripted("we descend", &party()),
        Ok(Interpreted::NeedsClarification)
    );
}

#[test]
fn an_empty_line_is_an_error() {
    assert_eq!(
        read_scripted("   ", &party()),
        Err(InterpretError::EmptyLine)
    );
}

#[test]
fn an_empty_party_has_nobody_to_move() {
    assert_eq!(
        read_scripted("we walk to the town square", &[]),
        Ok(Interpreted::NeedsClarification)
    );
}

#[test]
fn a_valid_move_answer_is_accepted_with_its_destination_trimmed() {
    let line = "the elf runs to the hallway";
    let answer = json!({
        "reading": "move",
        "kind": "run",
        "who": ["elf"],
        "to": "  the hallway  ",
        "confidence": 0.9
    });
    assert_eq!(
        parse_reading(&answer, line, &party()),
        Ok(moves(MoveKind::Run, &["elf"], "the hallway", line))
    );
}

#[test]
fn an_empty_who_in_a_move_answer_moves_the_whole_party() {
    let line = "we walk to the square";
    let answer = json!({"reading": "move", "kind": "walk", "who": [], "to": "the square"});
    assert_eq!(
        parse_reading(&answer, line, &party()),
        Ok(moves(
            MoveKind::Walk,
            &["elf", "warrior", "wizard"],
            "the square",
            line
        ))
    );
}

#[test]
fn a_not_a_move_answer_is_accepted() {
    let answer = json!({"reading": "not_a_move", "kind": null, "who": [], "to": null});
    assert_eq!(
        parse_reading(&answer, "how are you?", &party()),
        Ok(Interpreted::NotAMove)
    );
}

#[test]
fn a_move_answer_with_an_empty_party_needs_clarification() {
    let answer = json!({"reading": "move", "kind": "walk", "who": [], "to": "the square"});
    assert_eq!(
        parse_reading(&answer, "we walk to the square", &[]),
        Ok(Interpreted::NeedsClarification)
    );
}

#[test]
fn refused_answers_name_their_error() {
    let refused: Vec<(Value, &str)> = vec![
        (json!("walk"), "malformed_answer"),
        (
            json!({"kind": null, "who": [], "to": null}),
            "malformed_answer",
        ),
        (
            json!({"reading": "dance", "kind": null, "who": [], "to": null}),
            "malformed_answer",
        ),
        (
            json!({"reading": "not_a_move", "kind": 3, "who": [], "to": null}),
            "malformed_answer",
        ),
        (
            json!({"reading": "move", "kind": "walk", "who": "elf", "to": "x"}),
            "malformed_answer",
        ),
        (
            json!({"reading": "move", "kind": "walk", "who": [3], "to": "x"}),
            "malformed_answer",
        ),
        (
            json!({"reading": "move", "kind": null, "who": [], "to": "x"}),
            "malformed_answer",
        ),
        (
            json!({"reading": "move", "kind": "walk", "who": []}),
            "malformed_answer",
        ),
        (
            json!({"reading": "move", "kind": "walk", "who": ["elf"], "to": 4}),
            "malformed_answer",
        ),
        (
            json!({"reading": "move", "kind": "fly", "who": [], "to": "x"}),
            "unknown_kind",
        ),
        (
            json!({"reading": "move", "kind": "walk", "who": ["dragon"], "to": "x"}),
            "unknown_character",
        ),
        (
            json!({"reading": "move", "kind": "walk", "who": [], "to": "   "}),
            "empty_destination",
        ),
    ];
    for (answer, key) in refused {
        let error = parse_reading(&answer, "we walk to the square", &party())
            .expect_err("a malformed answer must be refused");
        assert_eq!(error.key(), key, "answer {answer}");
    }
}

#[test]
fn an_answer_naming_a_character_outside_an_empty_party_is_refused() {
    let answer = json!({"reading": "move", "kind": "walk", "who": ["elf"], "to": "x"});
    assert_eq!(
        parse_reading(&answer, "we walk to x", &[]).map_err(|error| error.key()),
        Err("unknown_character")
    );
}

#[test]
fn an_empty_line_is_refused_before_the_answer_is_read() {
    let answer = json!({"reading": "not_a_move", "kind": null, "who": [], "to": null});
    assert_eq!(
        parse_reading(&answer, " ", &party()),
        Err(InterpretError::EmptyLine)
    );
}

#[test]
fn a_verb_from_a_json_table_is_read_without_a_code_change() {
    let table = LabelTable::from_json_str(
        r#"{"verbs":[{"phrase":"sprint","kind":"run"}],
            "prepositions":["to"],"whole_party":["we"]}"#,
    )
    .expect("the table is valid");
    let line = "we sprint to the gate";
    assert_eq!(
        ScriptedReader::new(table.clone()).read(line, &party()),
        Ok(moves(
            MoveKind::Run,
            &["elf", "warrior", "wizard"],
            "the gate",
            line
        ))
    );
    assert_eq!(
        read_scripted_with(&table, "we walk to the gate", &party()),
        Ok(Interpreted::NotAMove)
    );
}

#[test]
fn a_table_with_an_unknown_kind_is_refused() {
    let error = LabelTable::from_json_str(
        r#"{"verbs":[{"phrase":"fly","kind":"soar"}],"prepositions":[],"whole_party":[]}"#,
    )
    .expect_err("'soar' is not a movement kind");
    assert_eq!(error.key(), "invalid_label_table");
}

#[test]
fn every_error_has_a_stable_key() {
    let keys = [
        (InterpretError::EmptyLine, "empty_line"),
        (
            InterpretError::UnknownKind { key: "fly".into() },
            "unknown_kind",
        ),
        (
            InterpretError::UnknownCharacter { id: "x".into() },
            "unknown_character",
        ),
        (InterpretError::EmptyDestination, "empty_destination"),
        (
            InterpretError::MalformedAnswer { reason: "x".into() },
            "malformed_answer",
        ),
        (
            InterpretError::InvalidLabelTable { reason: "x".into() },
            "invalid_label_table",
        ),
    ];
    for (error, key) in keys {
        assert_eq!(error.key(), key);
    }
}

#[test]
fn schema_lists_every_movement_kind_and_the_closed_fields() {
    let schema = jev_schema();
    let required = schema["required"].as_array().map(Vec::len);
    assert_eq!(required, Some(4));
    let kinds = schema["properties"]["kind"]["enum"].as_array();
    assert!(
        kinds.is_some_and(|kinds| kinds.contains(&json!("sneak")) && kinds.contains(&Value::Null))
    );
}

#[test]
fn facts_carry_the_line_the_party_and_the_kinds() {
    let facts = jev_facts("we walk to the square", &party());
    assert_eq!(facts["line"], "we walk to the square");
    assert_eq!(facts["party"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        facts["kinds"].as_array().map(Vec::len),
        Some(MoveKind::ALL.len())
    );
}
