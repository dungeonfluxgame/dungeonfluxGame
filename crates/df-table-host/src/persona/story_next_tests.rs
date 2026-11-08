//! The game master's next move: kept only when everyone named is there and a place set off for
//! is another of the story's.

use serde_json::{Value, json};

use super::story::StoryError;
use super::story_next::{fewest_spoken, parse_next, read_knowledge, turn_speakers};

fn here() -> Vec<String> {
    ["person-1", "person-2", "person-3"]
        .map(str::to_owned)
        .to_vec()
}

fn places() -> Vec<String> {
    ["market", "inn", "graveyard"].map(str::to_owned).to_vec()
}

fn answer(speaker: &str, to: &str, move_to: Value) -> Value {
    json!({"speaker": speaker, "to": to, "goal": "Ask the stranger where the bell was heard.",
           "move_to": move_to, "end": false})
}

fn refused(answer: &Value) -> bool {
    matches!(
        parse_next(answer, (&here(), &here()), ("inn", &places())),
        Err(StoryError::MalformedAnswer { .. })
    )
}

#[test]
fn a_move_by_someone_there_to_someone_else_there_is_read() {
    let next = parse_next(
        &answer("person-1", "person-2", Value::Null),
        (&here(), &here()),
        ("inn", &places()),
    )
    .unwrap();
    assert_eq!(next.speaker, "person-1");
    assert_eq!(next.move_to, None);
    let off = parse_next(
        &answer("person-1", "room", json!("graveyard")),
        (&here(), &here()),
        ("inn", &places()),
    )
    .unwrap();
    assert_eq!(off.move_to.as_deref(), Some("graveyard"));
}

#[test]
fn a_move_by_or_to_someone_not_there_or_to_no_new_place_is_refused() {
    assert!(refused(&answer("person-9", "person-2", Value::Null)));
    assert!(refused(&answer("person-1", "person-9", Value::Null)));
    assert!(
        refused(&answer("person-1", "person-1", Value::Null)),
        "nobody puts a move to themself"
    );
    assert!(
        refused(&answer("person-1", "room", json!("inn"))),
        "setting off goes somewhere else"
    );
    assert!(refused(&answer("person-1", "room", json!("castle"))));
}

#[test]
fn when_a_player_must_move_nobody_else_may() {
    let players = ["person-1".to_owned()];
    let townsperson = answer("person-2", "person-1", Value::Null);
    assert!(matches!(
        parse_next(&townsperson, (&players, &here()), ("inn", &places())),
        Err(StoryError::MalformedAnswer { .. })
    ));
    let player = answer("person-1", "person-2", Value::Null);
    assert!(parse_next(&player, (&players, &here()), ("inn", &places())).is_ok());
}

#[test]
fn a_goal_beyond_its_speakers_knowledge_comes_with_what_it_holds() {
    assert_eq!(
        read_knowledge(&json!({"beyond": false, "detail": null})).unwrap(),
        None
    );
    assert_eq!(
        read_knowledge(&json!({"beyond": true, "detail": "the graveyard lights"})).unwrap(),
        Some("the graveyard lights".to_owned())
    );
    assert!(read_knowledge(&json!({"beyond": true, "detail": null})).is_err());
}

#[test]
fn the_players_who_have_spoken_least_have_the_next_turn() {
    let recent = [
        "person-1: Who saw the lights?",
        "person-3: Only a rumour, sir.",
        "person-2: And the watch?",
        "person-1: When was it?",
    ]
    .map(str::to_owned);
    let speakers = here();
    let is_player = |id: &str| id != "person-3";
    assert_eq!(
        fewest_spoken(&recent, &speakers, &is_player),
        ["person-2".to_owned()],
        "the townsperson is no player, and person-1 spoke twice"
    );
    assert_eq!(
        fewest_spoken(&[], &speakers, &is_player),
        ["person-1".to_owned(), "person-2".to_owned()]
    );
}

/// Two players (`person-1`, `person-2`) and a townsperson (`person-3`).
fn turn(recent: &[&str], opening: bool, last: Option<&str>) -> Vec<String> {
    let recent: Vec<String> = recent.iter().map(|line| (*line).to_owned()).collect();
    turn_speakers(&here(), &recent, (opening, last), &|id: &str| {
        id != "person-3"
    })
}

#[test]
fn a_place_opens_with_whoever_is_there_and_then_the_players_take_turns() {
    assert_eq!(
        turn(&[], true, None),
        ["person-3"],
        "the townsperson sets the scene"
    );
    let opened = ["person-3: Mind the lanes after dark."];
    assert_eq!(
        turn(&opened, false, Some("person-3")),
        ["person-1", "person-2"],
        "after a townsperson, a player"
    );
    let one_spoke = ["person-3: Mind the lanes.", "person-1: Why?"];
    assert_eq!(
        turn(&one_spoke, false, Some("person-1")),
        ["person-2"],
        "the player behind has the turn, never the townsperson"
    );
    let level = ["person-1: Why?", "person-2: And when?"];
    assert_eq!(
        turn(&level, false, Some("person-2")),
        ["person-1", "person-3"],
        "with every player level, the last speaker waits and anyone else may move"
    );
}
