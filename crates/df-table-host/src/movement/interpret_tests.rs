//! Tests of `interpret`.

use crate::movement::interpret::{
    InterpretError, jev_facts, jev_schema, parse_reading, retry_after,
};
use crate::movement::types::{
    Clarification, Declared, Interpreted, MoveIntent, MoveKind, PartyMember, Unclear,
};
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

fn clarifies(unclear: Unclear, reason: Option<&str>) -> Interpreted {
    Interpreted::NeedsClarification(Clarification {
        unclear,
        reason: reason.map(str::to_owned),
    })
}

#[test]
fn a_needs_clarification_answer_is_accepted_with_what_is_unclear_and_why() {
    let answer = json!({
        "reading": "needs_clarification", "kind": null, "who": [], "to": null,
        "unclear": "destination", "reason": "  The line stops before it names a place.  "
    });
    assert_eq!(
        parse_reading(&answer, "we walk to", &party()),
        Ok(clarifies(
            Unclear::Destination,
            Some("The line stops before it names a place.")
        ))
    );
}

#[test]
fn a_clarification_without_a_named_cause_is_other_and_a_blank_reason_is_dropped() {
    let answer = json!({
        "reading": "needs_clarification", "kind": null, "who": [], "to": null,
        "unclear": null, "reason": "   "
    });
    assert_eq!(
        parse_reading(&answer, "we walk to", &party()),
        Ok(clarifies(Unclear::Other, None))
    );
}

#[test]
fn a_clarification_with_an_unknown_cause_or_without_the_fields_is_refused() {
    for answer in [
        json!({"reading": "needs_clarification", "kind": null, "who": [], "to": null,
               "unclear": "mood", "reason": null}),
        json!({"reading": "needs_clarification", "kind": null, "who": [], "to": null}),
        json!({"reading": "needs_clarification", "kind": null, "who": [], "to": null,
               "unclear": 3, "reason": null}),
    ] {
        let error = parse_reading(&answer, "we walk to", &party())
            .expect_err("a malformed clarification must be refused");
        assert_eq!(error.key(), "malformed_answer", "answer {answer}");
    }
}

#[test]
fn a_move_answer_with_an_empty_party_needs_clarification() {
    let answer = json!({"reading": "move", "kind": "walk", "who": [], "to": "the square"});
    assert_eq!(
        parse_reading(&answer, "we walk to the square", &[]),
        Ok(clarifies(Unclear::Other, None))
    );
}

#[test]
fn the_two_lines_a_live_roam_refused_read_as_moves_when_the_answer_names_the_place() {
    let cases = [
        (
            "The next town along the road. I want to see what the late light does to its walls.",
            "the next town along the road",
        ),
        (
            concat!(
                "I'll go see that little window above the open shutter. ",
                "The late light might catch its edge."
            ),
            "the little window above the open shutter",
        ),
    ];
    let party = vec![member("danny", "Danny")];
    for (line, to) in cases {
        let refusal = json!({
            "reading": "needs_clarification", "kind": null, "who": [], "to": null,
            "unclear": "destination", "reason": "The movement names no destination."
        });
        let fixed = json!({
            "reading": "move", "kind": "walk", "who": ["danny"], "to": to,
            "unclear": null, "reason": null
        });
        assert_eq!(
            parse_reading(&refusal, line, &party),
            Ok(clarifies(
                Unclear::Destination,
                Some("The movement names no destination.")
            ))
        );
        assert_eq!(
            parse_reading(&fixed, line, &party),
            Ok(moves(MoveKind::Walk, &["danny"], to, line))
        );
    }
}

#[test]
fn only_a_missing_destination_in_a_declared_move_is_read_again() {
    let no_place = clarifies(Unclear::Destination, Some("no place"));
    let other = clarifies(Unclear::Other, None);
    let walks = moves(MoveKind::Walk, &["elf"], "the square", "x");
    assert_eq!(
        retry_after(&no_place, Declared::Move),
        Some(&Clarification {
            unclear: Unclear::Destination,
            reason: Some("no place".to_owned())
        })
    );
    assert_eq!(retry_after(&no_place, Declared::Unknown), None);
    assert_eq!(retry_after(&other, Declared::Move), None);
    assert_eq!(retry_after(&Interpreted::NotAMove, Declared::Move), None);
    assert_eq!(retry_after(&walks, Declared::Move), None);
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
    assert_eq!(required, Some(6));
    let kinds = schema["properties"]["kind"]["enum"].as_array();
    assert!(
        kinds.is_some_and(|kinds| kinds.contains(&json!("sneak")) && kinds.contains(&Value::Null))
    );
    assert_eq!(
        schema["properties"]["unclear"]["enum"],
        json!(["destination", "other", null])
    );
}

#[test]
fn facts_carry_the_line_the_speaker_the_party_and_the_kinds() {
    let facts = jev_facts(
        "I walk to the square",
        Some("elf"),
        None,
        Declared::Unknown,
        None,
        &party(),
    );
    assert_eq!(facts["line"], "I walk to the square");
    assert_eq!(facts["declared"], "unknown");
    assert_eq!(facts["refused_before"], Value::Null);
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
    let facts = jev_facts(
        "we walk to the square",
        None,
        None,
        Declared::Unknown,
        None,
        &party(),
    );
    assert_eq!(facts["speaker"], Value::Null);
}

#[test]
fn facts_carry_a_declared_move_and_the_earlier_refusal() {
    let refused = Clarification {
        unclear: Unclear::Destination,
        reason: Some("no place".to_owned()),
    };
    let facts = jev_facts(
        "the next town",
        Some("elf"),
        Some("a goal of going somewhere"),
        Declared::Move,
        Some(&refused),
        &party(),
    );
    assert_eq!(facts["declared"], "move");
    assert_eq!(facts["context"], "a goal of going somewhere");
    assert_eq!(
        facts["refused_before"],
        json!({"unclear": "destination", "reason": "no place"})
    );
}
