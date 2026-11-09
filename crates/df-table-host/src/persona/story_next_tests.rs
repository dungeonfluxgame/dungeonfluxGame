//! The game master's next move: kept only when everyone named is there; a served place is left
//! for the next of the story's places.

use df_content::persona::PublicPersona;
use serde_json::{Value, json};

use super::handles::Handles;
use super::reveal::verdicts_schema;
use super::story::Story;
use super::story::StoryError;
use super::story::{AuthorView, HeldSecret, Revealable, with_ids};
use super::story_next::{
    Departure, Pace, every_player_said, every_player_spoke, fewest_spoken, next_place, own_part,
    parse_next, party_found, read_knowledge, read_revealable, read_served, revealable_fact,
    still_judged, strikes, tell_only_who_is_here, turn_speakers,
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

const STAYING: Departure<'static> = Departure::Staying;
const LEAVING: Departure<'static> = Departure::Leaving { to: "the inn" };

#[test]
fn a_goal_beyond_its_speakers_knowledge_comes_with_what_it_holds() {
    let fine = verdict((false, false, false, false), Value::Null);
    assert_eq!(read_knowledge(&fine, STAYING).unwrap(), None);
    let beyond = verdict((true, false, false, false), json!("the graveyard lights"));
    assert_eq!(
        read_knowledge(&beyond, STAYING).unwrap().as_deref(),
        Some("the goal holds what its speaker could not know yet: the graveyard lights")
    );
    let to_self = verdict((false, true, false, false), json!("the fruit seller"));
    assert_eq!(
        read_knowledge(&to_self, STAYING).unwrap().as_deref(),
        Some("the goal sends its speaker to themself: the fruit seller")
    );
    let blank = verdict((true, false, false, false), Value::Null);
    assert!(read_knowledge(&blank, STAYING).is_err());
    assert!(read_knowledge(&json!({"beyond": false, "detail": null}), STAYING).is_err());
    let stays = verdict((false, false, false, false), Value::Null);
    assert_eq!(read_knowledge(&stays, STAYING).unwrap(), None);
    assert!(
        read_knowledge(&stays, LEAVING).unwrap().is_some(),
        "a served place is left"
    );
    let leaves = verdict((false, false, false, true), Value::Null);
    assert_eq!(read_knowledge(&leaves, LEAVING).unwrap(), None);
    assert!(
        read_knowledge(&leaves, STAYING).unwrap().is_some(),
        "a place is not left before it is served"
    );
    assert_eq!(
        read_knowledge(&leaves, Departure::Travelling).unwrap(),
        None,
        "on the way going on is neither asked nor refused"
    );
    assert_eq!(read_knowledge(&stays, Departure::Travelling).unwrap(), None);
    let repeats = verdict(
        (false, false, true, false),
        json!("the ruts were already followed"),
    );
    assert_eq!(
        read_knowledge(&repeats, STAYING).unwrap().as_deref(),
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
    let pace = |served, struck| Pace { served, struck };
    assert_eq!(
        read_served(&json!({"why": "the clue is out", "served": true, "struck": false})).unwrap(),
        pace(true, false)
    );
    assert_eq!(
        read_served(&json!({"why": "the rival stepped out", "served": false, "struck": true}))
            .unwrap(),
        pace(false, true)
    );
    assert!(read_served(&json!({"why": "unsure", "served": true})).is_err());
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

#[test]
fn a_person_is_said_by_the_look_their_job_gives() {
    use super::story_next::the_look;
    assert_eq!(
        the_look("Adventurer: a dark-haired woman in a green hood"),
        "the dark-haired woman in a green hood"
    );
    assert_eq!(the_look("an old scholar"), "the old scholar");
    assert_eq!(the_look("Torchbearer"), "Torchbearer");
}

/// A story whose seal was seen and worked out at the scriptorium and whose charter was only seen,
/// at the vault.
fn found_story() -> Story {
    let piece = |id: &str| {
        json!({"id": id, "what": format!("the {id}"), "planted_by": "rival",
               "flaw": format!("what the {id} gives away"),
               "up_close": format!("how the {id} looks")})
    };
    let finding = |piece: &str, place: &str| json!({"piece": piece, "place": place, "line": "x"});
    serde_json::from_value(json!({
        "title": "t", "hook": "h", "truth": "the truth", "culprit": "rival", "motive": "m",
        "scenes": [], "characters": [], "beats": [],
        "evidence": [piece("seal"), piece("charter"), piece("ledger")],
        "findings": {
            "seen": [finding("seal", "scriptorium"), finding("charter", "vault")],
            "worked_out": [finding("seal", "scriptorium")]
        }
    }))
    .unwrap()
}

#[test]
fn a_player_knows_what_the_party_found_anywhere_and_anyone_else_is_told_none() {
    let story = found_story();
    let player = party_found(&story, true);
    assert_eq!(
        player,
        json!({"seen": ["how the seal looks", "how the charter looks"],
               "worked_out": ["what the seal gives away"]}),
        "a goal citing a piece seen at an earlier place is in what its player could know"
    );
    let other = party_found(&story, false);
    assert_eq!(other, json!({"seen": [], "worked_out": []}));
    assert!(
        !player.to_string().contains("ledger"),
        "a piece nobody found is not known"
    );
}

#[test]
fn the_game_master_is_told_the_part_but_never_the_handle_of_someone_not_here() {
    let character = |id: &str, part: &str| {
        json!({"id": id, "name": format!("Name of {id}"), "new": false, "part": part,
               "role": "r", "guidance": {},
               "secrets": [{"id": "s", "text": "a secret", "known_by": ["rival"]}]})
    };
    let stored: Story = serde_json::from_value(json!({
        "title": "t", "hook": "h", "truth": "t", "culprit": "rival", "motive": "m",
        "scenes": [], "beats": [],
        "characters": [character("warrior", "player"), character("rival", "framer"),
                       character("witness", "witness")],
        "evidence": [{"id": "seal", "what": "the seal", "planted_by": "rival", "flaw": "f",
                      "up_close": "u"}]
    }))
    .unwrap();
    let handles = Handles::author(["warrior", "rival", "witness"]);
    let public = |id: &str| -> PublicPersona {
        serde_json::from_value(json!({
            "id": handles.shown(id), "display_name": "", "job": format!("Job: a look of {id}"),
            "personality": [], "speech_style": "", "backstory": ""
        }))
        .unwrap()
    };
    let mut view = AuthorView {
        asked: with_ids(stored.clone(), |id| handles.shown(id)),
        cast: vec![public("warrior"), public("rival"), public("witness")],
        present: vec![handles.shown("warrior"), handles.shown("witness")],
        names: Vec::new(),
        handles: handles.clone(),
        stored,
    };
    let recent = [
        "rival: You will find nothing here.".to_owned(),
        "warrior: Who are you?".to_owned(),
    ];
    let told = tell_only_who_is_here(&mut view, &recent);
    let facts = json!({"story": view.asked, "cast": view.cast, "recent": told});
    let (absent, warrior) = (handles.shown("rival"), handles.shown("warrior"));
    assert!(
        !facts.to_string().contains(&absent),
        "no handle of someone not here is offered: {facts}"
    );
    assert_eq!(
        told,
        [
            "not here: You will find nothing here.".to_owned(),
            format!("{warrior}: Who are you?")
        ]
    );
    let rival = &view.asked.characters[1];
    assert_eq!(
        (rival.id.as_str(), rival.part.as_str()),
        ("not here", "framer")
    );
    assert_eq!(view.asked.culprit, "not here");
    assert_eq!(
        view.asked.characters[0].id, warrior,
        "who is there keeps their handle"
    );
    assert_eq!(view.cast.len(), 2, "the cast holds only who is there");
}

#[test]
fn a_move_is_the_strike_only_when_the_danger_strikes_now_and_its_speaker_holds_it() {
    let strikers = ["person-2".to_owned()];
    assert!(strikes(true, &strikers, "person-2"));
    assert!(
        !strikes(true, &strikers, "person-1"),
        "a player's move is never the strike"
    );
    assert!(
        !strikes(false, &strikers, "person-2"),
        "the one the danger holds speaks before it strikes"
    );
    assert!(!strikes(true, &[], "person-2"));
}

#[test]
fn the_knowledge_judge_is_told_the_speakers_own_part_and_the_danger_of_the_culprit() {
    let character = |id: &str, part: &str, role: &str| {
        json!({"id": id, "name": format!("Name of {id}"), "new": false, "part": part,
               "role": role, "guidance": {}, "secrets": []})
    };
    let story: Story = serde_json::from_value(json!({
        "title": "t", "hook": "h", "truth": "t", "culprit": "person-2", "motive": "m",
        "scenes": [{"id": "vault", "place": "the vault", "dressing": [],
                    "danger": "The rival waits in the dark."}],
        "beats": [], "evidence": [],
        "characters": [character("person-1", "player", "Adventurer"),
                       character("person-2", "framer", "A rival sent to seize the charter")]
    }))
    .unwrap();
    assert_eq!(
        own_part(&story, "person-2", "vault"),
        json!({"part": "framer", "role": "A rival sent to seize the charter",
               "danger": "The rival waits in the dark."}),
        "the rival is told a goal naming the rival is his own"
    );
    assert_eq!(
        own_part(&story, "person-1", "vault"),
        json!({"part": "player", "role": "Adventurer", "danger": null})
    );
    assert_eq!(own_part(&story, "person-9", "vault"), Value::Null);
}

#[test]
fn the_revealable_judge_reads_one_verdict_per_secret_id_and_keeps_those_that_hold() {
    let ids = ["forged_charter", "old_debt"];
    assert_eq!(
        verdicts_schema(&ids)["properties"]["verdicts"]["items"]["properties"]["id"]["enum"],
        json!(ids),
        "the judge may name only the secrets asked about"
    );
    let answer = json!({"verdicts": [
        {"id": "forged_charter", "holds": true, "reason": "The forgery was worked out."},
        {"id": "old_debt", "holds": false, "reason": "Nothing of the debt was found."}
    ]});
    assert_eq!(
        read_revealable(&answer, &ids).unwrap(),
        vec!["forged_charter".to_owned()]
    );
    let stranger = json!({"verdicts": [
        {"id": "forged_charter", "holds": true, "reason": "Found."},
        {"id": "old_debt", "holds": true, "reason": "Found."},
        {"id": "ghost", "holds": true, "reason": "?"}
    ]});
    assert!(read_revealable(&stranger, &ids).is_err());
    let missing = json!({"verdicts": [
        {"id": "forged_charter", "holds": true, "reason": "Found."}
    ]});
    assert!(
        read_revealable(&missing, &ids).is_err(),
        "every secret gets a verdict"
    );
}

fn rivals_secret() -> Vec<HeldSecret> {
    vec![HeldSecret {
        holder: "rival".to_owned(),
        secret: "forged_charter".to_owned(),
    }]
}

#[test]
fn the_revealable_fact_appears_only_once_the_danger_has_struck() {
    let handles = Handles::author(["pip", "rival"]);
    let held = rivals_secret();
    assert_eq!(revealable_fact(false, &held, &handles), Value::Null);
    assert_eq!(
        revealable_fact(true, &held, &handles),
        json!([{"holder": "person-2", "secret": "forged_charter"}]),
        "the holder is told by handle"
    );
}

#[test]
fn a_kept_revealable_judgement_stands_until_the_findings_change() {
    let kept = Revealable {
        place: "vault".to_owned(),
        findings: 4,
        secrets: rivals_secret(),
    };
    assert_eq!(
        still_judged(Some(&kept), "vault", 4),
        Some(rivals_secret().as_slice())
    );
    assert!(still_judged(Some(&kept), "vault", 5).is_none());
    assert!(still_judged(Some(&kept), "inn", 4).is_none());
    assert!(still_judged(None, "vault", 4).is_none());
}
