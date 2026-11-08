//! The storyteller's plot: an answer is kept only when every id, part and scene checks out.

use serde_json::{Value, json};

use super::handles::Handles;
use super::story::{StoryError, parse_moves, parse_story, read_names, with_ids};

fn guidance(establish: &str) -> Value {
    json!({
        "phase": "the arrest", "beat": "the charge is read", "hooks": ["a debt unpaid"],
        "threads": ["who wanted Kael gone"], "must_establish": [establish],
        "must_avoid": ["the framer's name"], "tone": "tense", "note": "keep it period-true"
    })
}

/// `count` moves by `speaker`, each a question put to `to`.
fn defence(speaker: &str, to: &str, count: usize) -> Value {
    let one = json!({"speaker": speaker, "to": to,
                     "goal": "Ask the merchant, as a question to him, where he was."});
    Value::Array(vec![one; count])
}

fn answer() -> Value {
    json!({
        "title": "The Stolen Seal",
        "accusation": "Theft of the town seal from the counting house.",
        "truth": "The harbour merchant took the seal and hid it in the warrior's pack.",
        "culprit": "harbour_merchant",
        "motive": "The warrior saw him bribe the gate.",
        "characters": [
            {"id": "warrior", "name": "Kael", "new": false, "part": "accused",
             "role": "A travelling sword for hire.", "guidance": guidance("He is innocent.")},
            {"id": "harbour_merchant", "name": "Oswin Tarr", "new": true, "part": "framer",
             "role": "A merchant with a ledger of debts.",
             "guidance": guidance("He planted the seal.")}
        ],
        "evidence": [
            {"id": "seal_in_pack", "what": "The seal found in Kael's pack.",
             "planted_by": "harbour_merchant",
             "flaw": "The pack was in the inn's stable all night.",
             "up_close": "The seal's wax is unbroken."}
        ],
        "beats": [
            {"id": "charge", "scene": "arrest", "happens": "The guards read the charge.",
             "turns_on": "Kael keeps his temper.", "leads": "warrior",
             "moves": [{"speaker": "harbour_merchant", "to": "warrior",
                        "goal": "Accuse the sword for hire, as a question to him."}]},
            {"id": "street", "scene": "march", "happens": "The town watches.",
             "turns_on": "A witness looks away.", "leads": "warrior",
             "moves": [{"speaker": "warrior", "to": "harbour_merchant",
                        "goal": "Ask the merchant, as a question to him, why."}]},
            {"id": "trial", "scene": "judgement", "happens": "The mayor hears the case.",
             "turns_on": "The stable boy's word.", "leads": "warrior",
             "moves": defence("warrior", "harbour_merchant", 3)}
        ]
    })
}

#[test]
fn a_plot_is_read_with_each_character_given_guided_guidance_under_the_title() {
    let story = parse_story(&answer(), &["warrior".to_owned()], &[]).unwrap();
    assert_eq!(story.culprit, "harbour_merchant");
    let framer = &story.characters[1];
    assert!(framer.new);
    assert_eq!(framer.guidance.arc.as_deref(), Some("The Stolen Seal"));
    assert_eq!(framer.guidance.must_establish, ["He planted the seal."]);
    assert_eq!(story.beats.len(), 3);
    assert_eq!(story.beats[0].moves[0].speaker, "harbour_merchant");
    let mut unheard = answer();
    unheard["beats"][2]["moves"] = defence("warrior", "harbour_merchant", 1);
    assert!(
        matches!(
            parse_story(&unheard, &["warrior".to_owned()], &[]),
            Err(StoryError::MalformedAnswer { .. })
        ),
        "a plot whose judgement leaves the accused unheard"
    );
}

#[test]
fn an_answer_that_leaves_out_the_cast_or_names_no_character_is_refused() {
    let refused = |answer: &Value, cast: &[&str]| {
        let cast: Vec<String> = cast.iter().map(|id| (*id).to_owned()).collect();
        matches!(
            parse_story(answer, &cast, &["fruit_vendor".to_owned()]),
            Err(StoryError::MalformedAnswer { .. })
        )
    };
    assert!(
        refused(&answer(), &["warrior", "elf"]),
        "the cast's elf takes no part"
    );
    let mut wrong_culprit = answer();
    wrong_culprit["culprit"] = json!("ghost");
    assert!(refused(&wrong_culprit, &["warrior"]));
    let mut cast_marked_new = answer();
    cast_marked_new["characters"][0]["new"] = json!(true);
    assert!(refused(&cast_marked_new, &["warrior"]));
    let mut named_id = answer();
    named_id["characters"][1]["id"] = json!("Oswin Tarr");
    assert!(refused(&named_id, &["warrior"]), "an id is a plain id");
    let mut no_judgement = answer();
    no_judgement["beats"][2]["scene"] = json!("march");
    assert!(
        refused(&no_judgement, &["warrior"]),
        "every scene has a beat"
    );
    let mut reused = answer();
    reused["characters"][1]["id"] = json!("fruit_vendor");
    assert!(
        refused(&reused, &["warrior"]),
        "a new character takes no stored persona's id"
    );
    let mut stranger_move = answer();
    stranger_move["beats"][0]["moves"][0]["speaker"] = json!("ghost");
    assert!(
        refused(&stranger_move, &["warrior"]),
        "a move is made by a character"
    );
    let mut bad_part = answer();
    bad_part["characters"][1]["part"] = json!("villain");
    assert!(refused(&bad_part, &["warrior"]));
}

#[test]
fn a_scenes_moves_fill_its_beats_and_name_only_characters() {
    let story = parse_story(&answer(), &["warrior".to_owned()], &[]).unwrap();
    // Three moves by `speaker` put to `to`, and `to`'s reply when it is someone.
    let moves = |speaker: &str, to: &str, id: &str| {
        let mut list = defence(speaker, to, 3).as_array().unwrap().clone();
        if to != "room" {
            list.push(json!({"speaker": to, "to": speaker, "goal": "Answer the charge."}));
        }
        let beat = json!({"id": id, "leads": "warrior", "moves": list, "exhibits": []});
        json!({ "beats": [beat] })
    };
    let both = ["warrior".to_owned(), "harbour_merchant".to_owned()];
    let refused = |answer: &Value| {
        matches!(
            parse_moves(answer, &story, ("judgement", &both)),
            Err(StoryError::MalformedAnswer { .. })
        )
    };
    let played = parse_moves(
        &moves("warrior", "harbour_merchant", "trial"),
        &story,
        ("judgement", &both),
    )
    .unwrap();
    let trial = played.beats.iter().find(|beat| beat.id == "trial").unwrap();
    assert_eq!(trial.moves[0].speaker, "warrior");
    assert_eq!(trial.moves[0].to, "harbour_merchant");
    assert!(refused(&moves("ghost", "room", "trial")));
    assert!(
        refused(&moves("warrior", "ghost", "trial")),
        "a move is put to a character or the room"
    );
    assert!(
        refused(&moves("warrior", "warrior", "trial")),
        "nobody puts a move to themself"
    );
    assert!(
        refused(&moves("warrior", "room", "charge")),
        "a beat of another scene"
    );
    let unheard = json!({"beats": [{"id": "trial", "leads": "warrior",
                                    "moves": defence("warrior", "room", 2),
                                    "exhibits": []}]});
    assert!(
        refused(&unheard),
        "the accused speaks for himself at least three times"
    );
    let accused_silent = json!({"beats": [{"id": "trial", "leads": "warrior",
                                           "moves": defence("harbour_merchant", "room", 3),
                                           "exhibits": []}]});
    assert!(refused(&accused_silent));
    assert!(
        matches!(
            parse_moves(
                &moves("warrior", "harbour_merchant", "trial"),
                &story,
                ("judgement", &["warrior".to_owned()]),
            ),
            Err(StoryError::MalformedAnswer { .. })
        ),
        "nobody is put a move who is not there"
    );
    assert!(
        refused(&moves("warrior", "room", "trial")),
        "everyone there makes a move of their own"
    );
}

#[test]
fn the_storyteller_sees_handles_for_ids_and_its_moves_come_back_under_the_ids() {
    let story = parse_story(&answer(), &["warrior".to_owned()], &[]).unwrap();
    let handles = Handles::author(
        story
            .characters
            .iter()
            .map(|character| character.id.as_str()),
    );
    let asked = with_ids(story.clone(), |id| handles.shown(id));
    assert_eq!(
        asked.culprit, "person-2",
        "no hidden part in the ids it is asked with"
    );
    assert_eq!(asked.beats[2].moves[0].to, "person-2");
    // The accused's three moves, and the merchant's reply.
    let mut moves = defence("person-1", "person-2", 3)
        .as_array()
        .unwrap()
        .clone();
    moves.push(defence("person-2", "person-1", 1)[0].clone());
    let exhibits = json!([{"id": "seal_in_pack", "up_close": "The seal's wax is fresh."}]);
    let answered = json!({"beats": [{"id": "trial", "leads": "person-1", "moves": moves,
                                     "exhibits": exhibits}]});
    let here = ["person-1".to_owned(), "person-2".to_owned()];
    let played = parse_moves(&answered, &asked, ("judgement", &here)).unwrap();
    let back = with_ids(played, |called| {
        handles.id_of(called).unwrap_or_else(|| called.to_owned())
    });
    assert_eq!(back.culprit, "harbour_merchant");
    assert_eq!(back.evidence[0].up_close, "The seal's wax is fresh.");
    let trial = back.beats.iter().find(|beat| beat.id == "trial").unwrap();
    assert_eq!(trial.exhibits, ["seal_in_pack".to_owned()]);
    assert_eq!(trial.moves[0].speaker, "warrior");
    assert_eq!(trial.moves[0].to, "harbour_merchant");
    assert_eq!(
        back.beats[0], story.beats[0],
        "other scenes come back as they were"
    );
}

#[test]
fn the_names_judge_answer_lists_texts_by_index_and_refuses_one_that_is_no_text() {
    let answer = json!({"naming": [{"index": 1, "name": "Haldor"}]});
    assert_eq!(
        read_names(&answer, 2).unwrap(),
        vec![(1, "Haldor".to_owned())]
    );
    assert!(read_names(&json!({"naming": []}), 2).unwrap().is_empty());
    assert!(matches!(
        read_names(&json!({"naming": [{"index": 2, "name": "Haldor"}]}), 2),
        Err(StoryError::MalformedAnswer { .. })
    ));
}
