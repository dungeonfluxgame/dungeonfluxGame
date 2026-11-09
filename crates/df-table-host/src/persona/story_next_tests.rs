//! The game master's next move: kept only when everyone named is there; a served place is left
//! for the next of the story's places.

use serde_json::{Value, json};

use super::story::StoryError;
use super::story_next::{
    every_player_said, every_player_spoke, fewest_spoken, next_place, parse_next, read_knowledge,
    read_served, turn_speakers,
};

fn here() -> Vec<String> {
    ["person-1", "person-2", "person-3"]
        .map(str::to_owned)
        .to_vec()
}

fn places() -> Vec<String> {
    ["market", "inn", "graveyard"].map(str::to_owned).to_vec()
}

fn answer(speaker: &str, to: &str) -> Value {
    json!({"speaker": speaker, "to": to, "goal": "Ask the stranger where the bell was heard.",
           "end": false})
}

fn refused(answer: &Value) -> bool {
    matches!(
        parse_next(answer, (&here(), &here()), ("inn", &places()), false),
        Err(StoryError::MalformedAnswer { .. })
    )
}

#[test]
fn a_move_by_someone_there_to_someone_else_there_is_read() {
    let next = parse_next(
        &answer("person-1", "person-2"),
        (&here(), &here()),
        ("inn", &places()),
        false,
    )
    .unwrap();
    assert_eq!(next.speaker, "person-1");
    assert_eq!(next.move_to, None);
    assert!(!next.served);
}

#[test]
fn a_move_by_or_to_someone_not_there_is_refused() {
    assert!(refused(&answer("person-9", "person-2")));
    assert!(refused(&answer("person-1", "person-9")));
    assert!(
        refused(&answer("person-1", "person-1")),
        "nobody puts a move to themself"
    );
}

#[test]
fn when_a_player_must_move_nobody_else_may() {
    let players = ["person-1".to_owned()];
    let townsperson = answer("person-2", "person-1");
    assert!(matches!(
        parse_next(&townsperson, (&players, &here()), ("inn", &places()), false),
        Err(StoryError::MalformedAnswer { .. })
    ));
    let player = answer("person-1", "person-2");
    assert!(parse_next(&player, (&players, &here()), ("inn", &places()), false).is_ok());
}

/// A knowledge judge's verdict: which refusals hold, and its detail.
fn verdict((beyond, to_self, repeats, sets_off): (bool, bool, bool, bool), detail: Value) -> Value {
    json!({"beyond": beyond, "to_self": to_self, "repeats": repeats, "sets_off": sets_off,
           "detail": detail})
}

#[test]
fn a_goal_beyond_its_speakers_knowledge_comes_with_what_it_holds() {
    let fine = verdict((false, false, false, false), Value::Null);
    assert_eq!(read_knowledge(&fine, false).unwrap(), None);
    let beyond = verdict((true, false, false, false), json!("the graveyard lights"));
    assert_eq!(
        read_knowledge(&beyond, false).unwrap().as_deref(),
        Some("the goal holds what its speaker could not know yet: the graveyard lights")
    );
    let to_self = verdict((false, true, false, false), json!("the fruit seller"));
    assert_eq!(
        read_knowledge(&to_self, false).unwrap().as_deref(),
        Some("the goal sends its speaker to themself: the fruit seller")
    );
    let blank = verdict((true, false, false, false), Value::Null);
    assert!(read_knowledge(&blank, false).is_err());
    assert!(read_knowledge(&json!({"beyond": false, "detail": null}), false).is_err());
    let stays = verdict((false, false, false, false), Value::Null);
    assert_eq!(read_knowledge(&stays, false).unwrap(), None);
    assert!(
        read_knowledge(&stays, true).unwrap().is_some(),
        "a served place is left"
    );
    let leaves = verdict((false, false, false, true), Value::Null);
    assert_eq!(read_knowledge(&leaves, true).unwrap(), None);
    assert!(
        read_knowledge(&leaves, false).unwrap().is_some(),
        "a place is not left before it is served"
    );
    let repeats = verdict(
        (false, false, true, false),
        json!("the ruts were already followed"),
    );
    assert_eq!(
        read_knowledge(&repeats, false).unwrap().as_deref(),
        Some("the goal repeats what was already asked or done: the ruts were already followed")
    );
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

#[test]
fn the_pace_judge_answers_true_or_false() {
    assert!(read_served(&json!({"why": "the clue is out", "served": true})).unwrap());
    assert!(!read_served(&json!({"why": "nothing yet", "served": false})).unwrap());
    assert!(read_served(&json!({"why": "unsure"})).is_err());
}

#[test]
fn a_served_place_is_left_or_the_story_ends() {
    let here = (&here()[..], &here()[..]);
    let move_ = answer("person-1", "room");
    let staying = parse_next(&move_, here, ("inn", &places()), false).unwrap();
    assert_eq!((staying.move_to, staying.end), (None, false));
    let leaving = parse_next(&move_, here, ("inn", &places()), true).unwrap();
    assert_eq!(leaving.move_to.as_deref(), Some("graveyard"));
    assert!(!leaving.end);
    let ending = parse_next(&move_, here, ("graveyard", &places()), true).unwrap();
    assert_eq!((ending.move_to, ending.end), (None, true));
}

#[test]
fn the_next_place_follows_the_story_order() {
    assert_eq!(next_place("market", &places()).as_deref(), Some("inn"));
    assert_eq!(next_place("graveyard", &places()), None);
    assert_eq!(next_place("castle", &places()), None);
}

#[test]
fn a_place_waits_for_every_player_to_speak_there() {
    let is_player = |id: &str| id != "person-3";
    let everyone = self::here();
    let here = ["person-3: Welcome.", "person-1: A bed?"].map(str::to_owned);
    assert!(!every_player_spoke(
        &here,
        (&everyone, &everyone),
        &is_player
    ));
    let last = ["person-2".to_owned()];
    assert!(
        every_player_spoke(&here, (&everyone, &last), &is_player),
        "the one player yet to speak has this move"
    );
    let none: [String; 0] = [];
    assert!(!every_player_spoke(&none, (&everyone, &last), &is_player));
    let all = ["person-1: A bed?", "person-2: And work?"].map(str::to_owned);
    assert!(every_player_spoke(&all, (&everyone, &everyone), &is_player));
}

#[test]
fn a_place_is_left_once_every_player_has_had_their_turns() {
    let is_player = |id: &str| id != "person-3";
    let here = [
        "person-1: One.",
        "person-2: Two.",
        "person-1: Three.",
        "person-2: Four.",
    ]
    .map(str::to_owned);
    assert!(every_player_said(&here, &self::here(), &is_player, 2));
    assert!(!every_player_said(&here, &self::here(), &is_player, 3));
}
