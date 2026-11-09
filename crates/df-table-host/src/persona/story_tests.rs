//! The storyteller's plot: an answer is kept only when every id, part and scene checks out.

use std::sync::Arc;

use serde_json::{Value, json};

use df_ai::text::TextOutcome;
use df_ai::text_fixture::ScriptedBackend;

use super::fixtures::{ScriptedHost, coverage_answer, scripted_host_cast};
use super::handles::Handles;
use super::request::{GivenGuidance, PersonaReply, PersonaStored};
use super::routes::path;
use super::storage::{guidance_file_name, persona_id, read_backfills, read_persona, write_persona};
use super::story::{
    Story, StoryError, StoryRoutes, apply_story, parse_moves, parse_story, read_names,
    stored_story, story_path, with_ids, with_scene_moves, write_story,
};
use crate::dev_client::{post_json, reply_value};
use crate::speech::fixtures::FakePort;
use crate::text::{BackendChoice, TextRuntime};

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

/// The scenes the test story is asked to play out over, in order.
fn scenes() -> Vec<String> {
    ["arrest", "march", "judgement"].map(str::to_owned).to_vec()
}

fn place(id: &str) -> Value {
    json!({"id": id, "place": "The market square.", "dressing": ["a counting-house sign"],
           "danger": "The watch is quick to the cudgel.",
           "music": "A slow lute over a market's murmur."})
}

fn answer() -> Value {
    json!({
        "title": "The Stolen Seal",
        "hook": "Theft of the town seal from the counting house.",
        "scenes": [place("march"), place("arrest"), place("judgement")],
        "truth": "The harbour merchant took the seal and hid it in the warrior's pack.",
        "culprit": "harbour_merchant",
        "motive": "The warrior saw him bribe the gate.",
        "characters": [
            {"id": "warrior", "name": "Kael", "new": false, "part": "accused",
             "role": "A travelling sword for hire.", "guidance": guidance("He is innocent."),
             "secrets": []},
            {"id": "harbour_merchant", "name": "Oswin Tarr", "new": true, "part": "framer",
             "role": "A merchant with a ledger of debts.",
             "guidance": guidance("He planted the seal."),
             "secrets": [{"id": "planted_seal", "text": "He hid the seal in the pack.",
                          "sensitivity": "critical",
                          "reveal_conditions": ["he is shown the gate's ledger"],
                          "known_by": ["harbour_merchant"]}]}
        ],
        "evidence": [
            {"id": "seal_in_pack", "what": "The seal found in Kael's pack.",
             "planted_by": "harbour_merchant",
             "flaw": "The pack was in the inn's stable all night.",
             "up_close": "The seal's wax is unbroken.", "needs": []}
        ],
        "beats": [
            {"id": "charge", "scene": "arrest", "happens": "The guards read the charge.",
             "turns_on": "Kael keeps his temper.", "exhibits": []},
            {"id": "street", "scene": "march", "happens": "The town watches.",
             "turns_on": "A witness looks away.", "exhibits": []},
            {"id": "trial", "scene": "judgement", "happens": "The mayor hears the case.",
             "turns_on": "The stable boy's word.",
             "exhibits": ["seal_in_pack"]}
        ],
        "ways": [
            {"from": "march", "to": "judgement", "stages": [
                {"id": "hall_steps", "sight": "Worn steps up to the hall.", "worth_a_look": null,
                 "touches": []},
                {"id": "hall_doors", "sight": "Iron-bound doors.",
                 "worth_a_look": "A notice nailed to the door.", "touches": []}
            ]},
            {"from": "arrest", "to": "march", "stages": [
                {"id": "stall_lane", "sight": "A lane between the stalls.",
                 "worth_a_look": null, "touches": []},
                {"id": "crowd_gathers", "sight": "A crowd gathers at the corner.",
                 "worth_a_look": null, "touches": []}
            ]}
        ]
    })
}

#[test]
fn a_plot_is_read_with_each_character_given_guided_guidance_under_the_title() {
    let story = parse_story(&answer(), (&["warrior".to_owned()], &[]), &scenes()).unwrap();
    assert_eq!(story.culprit, "harbour_merchant");
    let framer = &story.characters[1];
    assert!(framer.new);
    assert_eq!(framer.guidance.arc.as_deref(), Some("The Stolen Seal"));
    assert_eq!(framer.guidance.must_establish, ["He planted the seal."]);
    assert_eq!(story.beats.len(), 3);
    assert!(
        story.beats.iter().all(|beat| beat.moves.is_empty()),
        "a plot scripts no moves: play flows"
    );
    let ids: Vec<&str> = story.scenes.iter().map(|place| place.id.as_str()).collect();
    assert_eq!(
        ids,
        ["arrest", "march", "judgement"],
        "scenes in the order asked for"
    );
    let ways: Vec<(&str, &str)> = story
        .ways
        .iter()
        .map(|way| (way.from.as_str(), way.to.as_str()))
        .collect();
    assert_eq!(
        ways,
        [("arrest", "march"), ("march", "judgement")],
        "a way from each place to the next, in the scenes' order"
    );
    assert_eq!(
        framer.secrets[0].reveal_conditions,
        ["he is shown the gate's ledger"]
    );
}

#[test]
fn an_answer_that_leaves_out_the_cast_or_names_no_character_is_refused() {
    let refused = |answer: &Value, cast: &[&str]| {
        let cast: Vec<String> = cast.iter().map(|id| (*id).to_owned()).collect();
        matches!(
            parse_story(answer, (&cast, &["fruit_vendor".to_owned()]), &scenes()),
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
    // Someone who already lives in the world may be cast under their own id, never as new.
    let mut townsfolk = answer();
    let mut seller = townsfolk["characters"][1].clone();
    seller["id"] = json!("fruit_vendor");
    seller["new"] = json!(false);
    seller["secrets"] = json!([]);
    townsfolk["characters"].as_array_mut().unwrap().push(seller);
    let others = ["fruit_vendor".to_owned()];
    let cast_from_town = parse_story(&townsfolk, (&["warrior".to_owned()], &others), &scenes());
    assert!(!cast_from_town.unwrap().characters[2].new);
    let mut stranger_knows = answer();
    stranger_knows["characters"][1]["secrets"][0]["known_by"] = json!(["ghost"]);
    assert!(
        refused(&stranger_knows, &["warrior"]),
        "a secret is known only by characters"
    );
    let mut undescribed = answer();
    undescribed["scenes"] = json!([place("arrest"), place("march")]);
    assert!(
        refused(&undescribed, &["warrior"]),
        "every scene asked for is described"
    );
    let mut telling_id = answer();
    telling_id["characters"][1]["id"] = json!("harbour_framer");
    assert!(
        refused(&telling_id, &["warrior"]),
        "a new id names no part its character hides"
    );
    let mut bad_part = answer();
    bad_part["characters"][1]["part"] = json!("villain");
    assert!(refused(&bad_part, &["warrior"]));
}

/// The answer with a second piece of evidence, `ledger`, and the needs of both.
fn with_needs(seal_needs: Value, ledger_needs: Value) -> Value {
    let mut story = answer();
    story["evidence"][0]["needs"] = seal_needs;
    story["evidence"].as_array_mut().unwrap().push(json!({
        "id": "ledger", "what": "The stable's ledger.", "planted_by": "harbour_merchant",
        "flaw": "The pack is entered for the whole night.", "up_close": "A ruled page of names.",
        "needs": ledger_needs
    }));
    story
}

#[test]
fn clues_may_build_on_each_other_but_never_on_nothing_or_in_a_circle() {
    let read = |answer: &Value| parse_story(answer, (&["warrior".to_owned()], &[]), &scenes());
    let chain = read(&with_needs(json!(["ledger"]), json!([]))).unwrap();
    assert_eq!(
        chain.evidence[0].needs,
        ["ledger"],
        "the seal is read with the ledger"
    );
    for (seal, ledger, why) in [
        (
            json!(["ghost"]),
            json!([]),
            "a need names a piece of evidence",
        ),
        (
            json!(["seal_in_pack"]),
            json!([]),
            "a piece never needs itself",
        ),
        (
            json!(["ledger"]),
            json!(["seal_in_pack"]),
            "needs never go round",
        ),
    ] {
        assert!(
            matches!(
                read(&with_needs(seal, ledger)),
                Err(StoryError::MalformedAnswer { .. })
            ),
            "{why}"
        );
    }
}

#[test]
fn a_scenes_moves_fill_its_beats_and_name_only_characters() {
    let story = parse_story(&answer(), (&["warrior".to_owned()], &[]), &scenes()).unwrap();
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
    let story = parse_story(&answer(), (&["warrior".to_owned()], &[]), &scenes()).unwrap();
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

#[test]
fn a_scenes_moves_change_only_that_scene_in_the_stored_story() {
    let stored = parse_story(&answer(), (&["warrior".to_owned()], &[]), &scenes()).unwrap();
    // The view a moves question is asked with: names blank, every exhibit's look cleared.
    let mut asked = stored.clone();
    for character in &mut asked.characters {
        character.name.clear();
    }
    for piece in &mut asked.evidence {
        piece.up_close.clear();
    }
    let mut played = asked.clone();
    let trial = played
        .beats
        .iter_mut()
        .find(|beat| beat.id == "trial")
        .unwrap();
    trial.leads = "warrior".to_owned();
    trial.exhibits = vec!["seal_in_pack".to_owned()];
    played.evidence[0].up_close = "A fresh look at the seal.".to_owned();
    let kept = with_scene_moves(stored.clone(), &played, "judgement");
    assert_eq!(
        kept.characters[0].name, stored.characters[0].name,
        "names are kept"
    );
    let beat = kept.beats.iter().find(|beat| beat.id == "trial").unwrap();
    assert_eq!(beat.leads, "warrior");
    assert_eq!(kept.evidence[0].up_close, "A fresh look at the seal.");
    let other = with_scene_moves(stored.clone(), &played, "arrest");
    assert_eq!(
        other.evidence[0].up_close, stored.evidence[0].up_close,
        "a scene that does not show a piece leaves its look as it was"
    );
}

/// The plot's framer, `new` in the plot, with its guidance and one secret.
fn framer_plot() -> Story {
    serde_json::from_value(json!({
        "title": "The Vault", "hook": "h", "truth": "t", "culprit": "rival_scholar", "motive": "m",
        "scenes": [], "evidence": [], "beats": [],
        "characters": [{
            "id": "rival_scholar", "name": "Oren Vask", "new": true, "part": "framer",
            "role": "a rival scholar who plants the forged seal",
            "guidance": guidance("They planted the forged seal in the archive."),
            "secrets": [{"id": "forged_seal", "text": "They forged the seal.",
                         "reveal_conditions": ["shown the forger's mould"]}]
        }]
    }))
    .unwrap()
}

#[tokio::test]
async fn a_plain_persona_kept_for_a_plot_character_gets_its_part_once() {
    // Apply one: the coverage judge finds the part missing, the backfill adds it. Apply two: the
    // same guidance was given, so nothing is asked.
    let backfill = json!({"edits": [{
        "op": "append_text", "field": "private_backstory",
        "text": "He planted the forged seal in the archive.", "id": null, "sensitivity": null,
        "reveal_conditions": null, "known_by": null, "speech_style": null, "condition": null,
        "event": null, "behavior": null
    }]});
    let answers = vec![
        coverage_answer(&[("private_backstory", "the forged seal")]),
        backfill,
    ];
    let host = scripted_host_cast(
        &["rival_scholar"],
        answers,
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    host.running.write_asset("persona/story.json", "{}");
    write_story(&story_path(&host.running.state), &framer_plot()).unwrap();
    // The persona kept from before the plot gave it a part: a job and a look, nothing else.
    let plain = serde_json::from_value(json!({
        "schema": 1, "id": "rival_scholar", "display_name": "Oren Vask",
        "job": "Scholar: a lean grey-bearded man in an ink-stained robe"
    }))
    .unwrap();
    write_persona(&host.directory(), &plain).unwrap();
    let id = persona_id("rival_scholar").unwrap();

    let first = apply_story(host.port, None).await.unwrap();
    assert!(
        first.contains("a persona was already kept; 1 edit(s) from its guidance"),
        "{first}"
    );
    assert!(first.contains("1 secret(s) kept"), "{first}");
    let guided = read_persona(&host.directory(), &id).unwrap().unwrap();
    assert_eq!(guided.job(), plain.job(), "the plates know it by its job");
    assert_eq!(guided.display_name(), "Oren Vask");
    assert!(guided.private_backstory().contains("forged seal"));
    assert_eq!(guided.private_secrets()[0].id, "forged_seal");
    assert!(
        host.facts_of(0)
            .to_string()
            .contains("planted the forged seal"),
        "the coverage judge weighs the persona against its guidance"
    );
    let kept = host.file("rival_scholar.persona.json").unwrap();

    let second = apply_story(host.port, None).await.unwrap();
    assert!(
        second.contains("a persona was already kept; its guidance was given before"),
        "{second}"
    );
    assert_eq!(host.file("rival_scholar.persona.json").unwrap(), kept);
    assert_eq!(
        host.calls(),
        2,
        "neither the judge nor a backfill is asked again"
    );
}

/// The framer's persona as it was before the plot gave it a part: a job and a look, nothing else.
fn plain_framer() -> df_content::persona::Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": "rival_scholar", "display_name": "Oren Vask",
        "job": "Scholar: a lean grey-bearded man in an ink-stained robe"
    }))
    .unwrap()
}

/// A host whose scene shows the framer, with the plot stored and the plain framer kept, whose
/// text backend answers with `answers`.
async fn framer_host(answers: Vec<Value>) -> ScriptedHost {
    let host = scripted_host_cast(
        &["rival_scholar"],
        answers,
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    host.running.write_asset("persona/story.json", "{}");
    write_story(&story_path(&host.running.state), &framer_plot()).unwrap();
    write_persona(&host.directory(), &plain_framer()).unwrap();
    host
}

#[tokio::test]
async fn three_applies_add_edits_only_on_the_first_though_the_judge_finds_gaps_each_time() {
    // A judge that names gaps whenever it is asked (dungeon run 25 judged one unchanged persona
    // covered, then six gaps, then three): every named gap is filled on the first apply, and the
    // next two ask nothing, so the backstory stops growing.
    let backfill = json!({"edits": [{
        "op": "append_text", "field": "private_backstory",
        "text": "He planted the forged seal in the archive.", "id": null, "sensitivity": null,
        "reveal_conditions": null, "known_by": null, "speech_style": null, "condition": null,
        "event": null, "behavior": null
    }]});
    let short = coverage_answer(&[("private_backstory", "the seal"), ("fears", "being caught")]);
    let answers = vec![
        short.clone(),
        backfill.clone(),
        short.clone(),
        backfill.clone(),
        short,
        backfill,
    ];
    let host = framer_host(answers).await;

    let first = apply_story(host.port, None).await.unwrap();
    assert!(first.contains("1 edit(s) from its guidance"), "{first}");
    let gaps = host.facts_of(1).to_string();
    assert!(
        gaps.contains("being caught"),
        "every named gap is filled: {gaps}"
    );
    let kept = host.file("rival_scholar.persona.json").unwrap();
    for _ in 0..2 {
        let again = apply_story(host.port, None).await.unwrap();
        assert!(
            again.contains("its guidance was given before; nothing added"),
            "{again}"
        );
        assert_eq!(host.file("rival_scholar.persona.json").unwrap(), kept);
    }
    assert_eq!(
        host.calls(),
        2,
        "one judgement and one backfill, both on the first apply"
    );
    let id = persona_id("rival_scholar").unwrap();
    assert_eq!(read_backfills(&host.directory(), &id).unwrap().len(), 1);
}

#[tokio::test]
async fn given_guidance_is_judged_anew_once_the_persona_is_written_whole_or_the_plot_changes() {
    let host = framer_host(vec![
        coverage_answer(&[]),
        coverage_answer(&[]),
        coverage_answer(&[]),
    ])
    .await;
    let id = persona_id("rival_scholar").unwrap();

    let first = apply_story(host.port, None).await.unwrap();
    assert!(
        first.contains("already holds its guidance; nothing added"),
        "{first}"
    );
    let given: GivenGuidance =
        serde_json::from_str(&host.file(&guidance_file_name(&id)).unwrap()).unwrap();
    assert_eq!(
        given.guidance.must_establish,
        [
            "Their name is Oren Vask.",
            "They planted the forged seal in the archive."
        ]
    );
    apply_story(host.port, None).await.unwrap();
    assert_eq!(host.calls(), 1, "the same guidance is not judged again");

    // The clean persona put back whole (`persona write`, as a replay's restore does) holds none of
    // the guidance, so it is judged again.
    let written: PersonaReply<PersonaStored> = post_json(host.port, path::PERSONA, &plain_framer())
        .await
        .unwrap();
    reply_value(written).unwrap();
    assert!(host.file(&guidance_file_name(&id)).is_none());
    apply_story(host.port, None).await.unwrap();
    assert_eq!(host.calls(), 2, "a persona written whole is judged again");

    // A new plot gives the character other guidance.
    let mut replotted = framer_plot();
    replotted.characters[0].guidance.must_establish = vec!["They burned the ledger.".to_owned()];
    write_story(&story_path(&host.running.state), &replotted).unwrap();
    apply_story(host.port, None).await.unwrap();
    assert_eq!(host.calls(), 3, "changed guidance is judged anew");
}

#[tokio::test]
async fn reading_the_story_waits_for_a_held_story_lock_rather_than_failing() {
    let host = framer_host(Vec::new()).await;
    let text = TextRuntime::new(
        BackendChoice::Scripted(
            Arc::new(ScriptedBackend::new(
                Vec::<TextOutcome>::new(),
                Arc::default(),
            )),
            None,
        ),
        Arc::new(|_| {}),
    );
    let routes = Arc::new(StoryRoutes {
        app: Arc::clone(&host.running.state),
        text: Arc::new(text),
        story_write: tokio::sync::Mutex::new(()),
        stretch: std::sync::Mutex::default(),
    });
    let path = story_path(&routes.app);
    // A writer holds the lock with the story half written, as a clue or a sheet being kept does.
    let writing = routes.story_write.lock().await;
    std::fs::write(&path, r#"{"title": "The Va"#).unwrap();
    let reader = tokio::spawn({
        let routes = Arc::clone(&routes);
        async move { stored_story(&routes).await }
    });
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(!reader.is_finished(), "the reader waits for the writer");
    write_story(&path, &framer_plot()).unwrap();
    drop(writing);

    let read = reader.await.unwrap().unwrap();
    assert_eq!(read.title, "The Vault");
}
