//! The storyteller's plot: an answer is kept only when every id, part and scene checks out.

use serde_json::{Value, json};

use super::story::{StoryError, parse_story};

fn guidance(establish: &str) -> Value {
    json!({
        "phase": "the arrest", "beat": "the charge is read", "hooks": ["a debt unpaid"],
        "threads": ["who wanted Kael gone"], "must_establish": [establish],
        "must_avoid": ["the framer's name"], "tone": "tense", "note": "keep it period-true"
    })
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
             "flaw": "The pack was in the inn's stable all night."}
        ],
        "beats": [
            {"id": "charge", "scene": "arrest", "happens": "The guards read the charge.",
             "turns_on": "Kael keeps his temper."},
            {"id": "street", "scene": "march", "happens": "The town watches.",
             "turns_on": "A witness looks away."},
            {"id": "trial", "scene": "judgement", "happens": "The mayor hears the case.",
             "turns_on": "The stable boy's word."}
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
    let mut bad_part = answer();
    bad_part["characters"][1]["part"] = json!("villain");
    assert!(refused(&bad_part, &["warrior"]));
}
