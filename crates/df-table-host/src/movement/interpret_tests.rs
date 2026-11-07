//! Tests of `interpret`.

use crate::movement::interpret::{InterpretError, jev_facts, jev_schema, parse_reading};
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
fn a_first_person_answer_lists_the_speaker_as_a_mover() {
    let line = "I will walk to the balcony";
    let answer = json!({"reading": "move", "kind": "walk", "who": ["elf"], "to": "the balcony"});
    assert_eq!(
        parse_reading(&answer, line, &party()),
        Ok(moves(MoveKind::Walk, &["elf"], "the balcony", line))
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
fn a_duplicate_id_in_a_move_answer_is_listed_once() {
    let line = "the elf and the elf walk to the square";
    let answer = json!({
        "reading": "move",
        "kind": "walk",
        "who": ["elf", "elf"],
        "to": "the square"
    });
    assert_eq!(
        parse_reading(&answer, line, &party()),
        Ok(moves(MoveKind::Walk, &["elf"], "the square", line))
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
fn a_needs_clarification_answer_is_accepted() {
    let answer = json!({"reading": "needs_clarification", "kind": null, "who": [], "to": null});
    assert_eq!(
        parse_reading(&answer, "we walk to", &party()),
        Ok(Interpreted::NeedsClarification)
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
fn facts_carry_the_line_the_speaker_the_party_and_the_kinds() {
    let facts = jev_facts("I walk to the square", Some("elf"), None, &party());
    assert_eq!(facts["line"], "I walk to the square");
    assert_eq!(facts["speaker"]["id"], "elf");
    assert_eq!(facts["speaker"]["display_name"], "Lirael");
    assert_eq!(facts["party"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        facts["kinds"].as_array().map(Vec::len),
        Some(MoveKind::ALL.len())
    );
}

#[test]
fn facts_without_a_speaker_carry_none() {
    let facts = jev_facts("we walk to the square", None, None, &party());
    assert_eq!(facts["speaker"], Value::Null);
}
