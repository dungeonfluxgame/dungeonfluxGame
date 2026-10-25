use super::*;

#[tokio::test]
async fn hs71_item_connection_is_jev_decided_and_authored_clues_need_no_model() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::FakeEndpoint,
    };
    let (current, _) = opening();
    let mut scene = merchant_shop_room();
    let mut thing = scene
        .setting
        .as_ref()
        .unwrap()
        .model
        .as_ref()
        .unwrap()
        .things[0]
        .clone();
    thing.finding_id = None;
    thing.clue_fact_id = None;
    let judge = FakeEndpoint::start(vec![answer("o1")]);
    let ordinary = decide_item_story(
        &Storyteller::disabled(),
        &model(&judge),
        current.state(),
        &scene,
        &thing,
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(ordinary.connection, ItemConnection::Ordinary);
    assert!(ordinary.hidden.is_none() && ordinary.surface.is_none());
    thing.clue_fact_id = Some("authored-secret.id".into());
    let authored = decide_item_story(
        &Storyteller::disabled(),
        &model(&judge),
        current.state(),
        &scene,
        &thing,
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(authored.connection, ItemConnection::Clue);
    assert_eq!(
        authored.authored_finding.as_deref(),
        Some("authored-secret.id")
    );
    assert_eq!(
        authored.authored_source.as_ref().unwrap().sentence,
        thing.closer_look
    );
    let private = pack::Finding {
        id: "member-private-mark".into(),
        sentence: "A protected personal inscription.".into(),
        skill: pack::FindingSkill::Investigation,
        dc: 13,
        private: true,
        consequence: Some("retained-authored-source".into()),
    };
    scene.findings.push(private.clone());
    thing.finding_id = Some(private.id.clone());
    let private_decision = decide_item_story(
        &Storyteller::disabled(),
        &model(&judge),
        current.state(),
        &scene,
        &thing,
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(
        private_decision.authored_source,
        Some(private),
        "private audience, DC and authored consequence survive exactly"
    );
    assert_eq!(
        judge.connections(),
        1,
        "authored identity never gets reclassified"
    );
}

#[tokio::test]
async fn hs71_generated_clue_has_a_public_surface_and_separate_srd_hidden_check() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    let (current, _) = opening();
    assert!(!current.state().narrative.open_threads.is_empty());
    let scene = merchant_shop_room();
    let mut thing = scene
        .setting
        .as_ref()
        .unwrap()
        .model
        .as_ref()
        .unwrap()
        .things[0]
        .clone();
    thing.finding_id = None;
    thing.clue_fact_id = None;
    let surface = "A patch of old blue wax clings to the rim.";
    let hidden = "A maker's gull mark is scratched beneath the base.";
    let story = FakeEndpoint::start(vec![Reply::Output(json!({"detail":json!({
        "surface":surface,"hidden":{"detail":hidden,"skill":"investigation","dc":15}
    }).to_string()}))]);
    let judge = FakeEndpoint::start(vec![answer("o3"), answer("o1")]);
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS71/clue-decision")
        .join(std::process::id().to_string());
    let decision = decide_item_story(
        &teller(story.address, 0, Duration::from_secs(5), &cache),
        &model(&judge),
        current.state(),
        &scene,
        &thing,
        json!({}),
    )
    .await
    .unwrap();
    assert!(decision.valid());
    assert_eq!(decision.connection, ItemConnection::Clue);
    assert_eq!(decision.surface.as_deref(), Some(surface));
    assert_eq!(decision.hidden.as_ref().unwrap().detail, hidden);
    assert_eq!(decision.hidden.as_ref().unwrap().dc, 15);
    assert!(judge.bodies()[0].to_string().contains("open_threads"));
    assert!(judge.bodies()[1].to_string().contains(hidden));
}

#[tokio::test]
async fn hs71_failed_second_look_has_no_hidden_source_and_success_reveals_exactly_the_bound_fact() {
    use crate::gameplay::{approach_model::tests::model, storyteller::tests::FakeEndpoint};
    let mut scene = merchant_shop_room();
    let thing = &mut scene
        .setting
        .as_mut()
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .things[0];
    thing.finding_id = Some("hs71-secret-mark".into());
    thing.clue_fact_id = None;
    let item = thing.id.clone();
    let hidden = "A maker's gull mark is scratched beneath the base.";
    scene.findings.push(pack::Finding {
        id: "hs71-secret-mark".into(),
        sentence: hidden.into(),
        skill: pack::FindingSkill::Investigation,
        dc: 15,
        private: false,
        consequence: None,
    });
    let judge = FakeEndpoint::start(vec![]);
    let mut check = rpc::AbilityCheckResult {
        succeeded: false,
        total: 8,
        source_revision: format!("srd521-p6:room:investigation:target-thing-{item}"),
        ..Default::default()
    };
    assert!(observation_sources(&scene, &check).is_empty());
    let failed = complete_finding_for_question(
        &Storyteller::disabled(),
        &model(&judge),
        &scene,
        &check,
        "Brynn",
        "I search beneath it.",
        None,
    )
    .await;
    assert!(failed.selected_source_ids.is_empty());
    assert!(!failed.text.contains(hidden));
    check.succeeded = true;
    check.total = 19;
    let success = complete_finding_for_question(
        &Storyteller::disabled(),
        &model(&judge),
        &scene,
        &check,
        "Brynn",
        "I search beneath it.",
        None,
    )
    .await;
    assert_eq!(success.selected_source_ids, vec!["hs71-secret-mark"]);
    assert!(success.text.contains(hidden));
    scene.findings.last_mut().unwrap().private = true;
    assert!(
        checked_item_finding(&scene, &check).is_none(),
        "member-private facts never reach shared narration"
    );
    assert_eq!(
        judge.connections(),
        0,
        "rules success/failure cannot be overridden by expression choice"
    );
}

#[tokio::test]
async fn perception_title_names_the_attempt_in_the_actual_open_street() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    let mut scene = merchant_shop_room();
    let setting = scene.setting.as_mut().unwrap();
    setting.name = "Gullwatch Market Street".into();
    setting.short = "a rain-slick open market street".into();
    setting.exterior = true;
    let expression = FakeEndpoint::start(vec![Reply::Output(json!({"intention":
        json!({"action":"watches the stallholders' hands"}).to_string()}))]);
    let jev = FakeEndpoint::start(vec![answer("o1")]);
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS40/intention")
        .join(std::process::id().to_string());
    let storyteller = teller(expression.address, 0, Duration::from_secs(5), &cache);
    let intention = check_intention(
        &storyteller,
        &model(&jev),
        &scene,
        "Pip",
        "I watch the stallholders' hands for anything out of place.",
    )
    .await
    .unwrap();
    let check = rpc::AbilityCheckResult {
        succeeded: true,
        die: 16,
        total: 18,
        source_revision: "srd521-p6:market:perception:target-room".into(),
        ..Default::default()
    };
    let observed = observation(&scene, &check).unwrap();
    let line = observed.roll_line_with_intention("Pip", &check, Some(&intention));
    assert!(line.starts_with("Pip watches the stallholders' hands:"));
    assert!(!line.contains("searches the room"));
    assert!(
        observed
            .roll_line("Pip", &check)
            .contains("Gullwatch Market Street")
    );
    assert!(
        expression.bodies()[0]["input"]
            .as_str()
            .unwrap()
            .contains("Gullwatch Market Street")
    );
}

#[tokio::test]
async fn question_finding_answers_the_question_and_reuses_an_approved_physical_detail() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    let scene = merchant_shop_room();
    let check = rpc::AbilityCheckResult {
        succeeded: true,
        total: 18,
        source_revision: "srd521-p6:room:investigation:target-thing-shop-counter".into(),
        ..Default::default()
    };
    let detail = "The counter's legs sit flat and steady on the boards.";
    let text = "Pip finds the counter steady; none of its legs rocks on the boards.";
    let jev = FakeEndpoint::start(vec![answer("o1"), answer("o1"), answer("o1"), answer("o1")]);
    let expression = FakeEndpoint::start(vec![
        Reply::Output(
            json!({"answer":json!({"answer":text, "established_detail":detail}).to_string()}),
        ),
        Reply::Output(
            json!({"answer":json!({"answer":text, "established_detail":null}).to_string()}),
        ),
    ]);
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS40/question-finding")
        .join(std::process::id().to_string());
    let storyteller = teller(expression.address, 0, Duration::from_secs(5), &cache);
    let first = complete_finding_for_question(
        &storyteller,
        &model(&jev),
        &scene,
        &check,
        "Pip",
        "Does the counter wobble?",
        None,
    )
    .await;
    assert!(!first.unavailable);
    assert_eq!(first.text, text);
    assert_eq!(first.established_detail.as_ref().unwrap().detail, detail);
    assert!(
        first.selected_source_ids.is_empty(),
        "new detail is not an authored source id"
    );
    let again = complete_finding_for_question(
        &storyteller,
        &model(&jev),
        &scene,
        &check,
        "Pip",
        "Is it still steady?",
        first.established_detail.as_ref(),
    )
    .await;
    assert_eq!(again.text, text);
    assert!(!again.unavailable);
    assert!(again.established_detail.is_none());
    let bodies = expression.bodies();
    let first_prompt = bodies[0]["input"].as_str().unwrap();
    assert!(first_prompt.contains("Does the counter wobble?"));
    let repeat_prompt = bodies[1]["input"].as_str().unwrap();
    assert!(repeat_prompt.contains(detail));
    assert!(repeat_prompt.contains("Is it still steady?"));
}

#[tokio::test]
async fn question_finding_failed_roll_and_extra_schema_fields_cannot_establish_facts() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    let scene = merchant_shop_room();
    for (case, (succeeded, candidate)) in [
        (
            false,
            json!({"answer":"Pip sees a steady counter.", "established_detail":"The legs are steady."}).to_string(),
        ),
        (
            true,
            json!({"answer":"Pip sees the marks.", "established_detail":null, "secret":"A hidden vault."}).to_string(),
        ),
        (true, r#"{"answer":"Pip sees the marks.","answer":"Pip sees new marks.","established_detail":null}"#.into()),
    ].into_iter().enumerate() {
        let check = rpc::AbilityCheckResult {
            succeeded,
            total: 18,
            source_revision: "srd521-p6:room:investigation:target-thing-shop-counter".into(),
            ..Default::default()
        };
        let jev = FakeEndpoint::start(vec![answer("o1")]);
        let expression =
            FakeEndpoint::start(vec![Reply::Output(json!({"answer":candidate}))]);
        let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS40/question-rejection")
            .join(format!("{}-{case}", std::process::id()));
        let storyteller = teller(expression.address, 0, Duration::from_secs(5), &cache);
        let result = complete_finding_for_question(
            &storyteller,
            &model(&jev),
            &scene,
            &check,
            "Pip",
            "Does the counter wobble?",
            None,
        )
        .await;
        assert!(result.unavailable);
        assert!(result.established_detail.is_none());
        assert!(result.selected_source_ids.is_empty());
        assert_eq!(jev.connections(), 1, "a malformed candidate must not reach consistency approval");
    }
}

fn room() -> Vec<Target> {
    [
        (
            "room",
            "the room",
            "The taproom is lit by a hearth.",
            &["room", "place", "around", "in here"][..],
        ),
        (
            "tankards",
            "the tankards",
            "Pewter tankards stand on the shelf behind the counter.",
            &["tankard", "tankards"][..],
        ),
        (
            "hearth",
            "the hearth",
            "Ashes lie in the hearth.",
            &["hearth", "ashes"][..],
        ),
        (
            "window",
            "the window",
            "The window overlooks the street.",
            &["window", "street"][..],
        ),
        (
            "table",
            "the table",
            "A candle lights the table.",
            &["under the table", "table"][..],
        ),
        ("rug", "the rug", "The rug lies on the floor.", &["rug"][..]),
        (
            "notice",
            "the notice",
            "There is no notice on the door.",
            &["notice"][..],
        ),
        (
            "ilsa",
            "Ilsa",
            "Ilsa stands straight beside the hearth.",
            &["ilsa", "her", "she", "innkeeper", "current speaker"][..],
        ),
        (
            "back-door",
            "the back door",
            "The back door leads outside.",
            &["back door"][..],
        ),
        (
            "lock",
            "the lock",
            "A lock fastens the cellar door.",
            &["lock"][..],
        ),
        (
            "floor",
            "the floor",
            "The floor is worn by passing boots.",
            &["floor", "tracks"][..],
        ),
        (
            "herbs",
            "the dried herbs",
            "Dried herbs hang from the beams.",
            &["herbs", "herb"][..],
        ),
        (
            "shrine",
            "the shrine",
            "There is no shrine here.",
            &["shrine", "holy sign"][..],
        ),
        (
            "men",
            "the men at the back table",
            "Two men sit at the back table.",
            &["people at the back table", "men at the back", "back table"][..],
        ),
        (
            "door",
            "the door",
            "The door opens into the street.",
            &["on the door", "door"][..],
        ),
        (
            "letter",
            "the letter",
            "There is no letter here.",
            &["letter"][..],
        ),
        (
            "moon",
            "the moon",
            "The moon is not visible from here.",
            &["moon"][..],
        ),
        (
            "brakka-pack",
            "Brakka's pack",
            "Brakka carries his pack.",
            &["brakka's pack"][..],
        ),
        ("wall", "the wall", "The wall is plastered.", &["wall"][..]),
        (
            "counter",
            "behind the counter",
            "Shelves stand behind the counter.",
            &["behind the counter"][..],
        ),
        (
            "candle",
            "the candle",
            "A candle stands on the table.",
            &["candle"][..],
        ),
        (
            "bread",
            "the bread",
            "Ilsa is cutting bread.",
            &["bread"][..],
        ),
        (
            "jug",
            "the jug",
            "A jug of ale stands on the bar.",
            &["jug", "ale"][..],
        ),
    ]
    .into_iter()
    .map(|(id, name, description, aliases)| Target {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
        present: !matches!(id, "notice" | "shrine" | "letter" | "moon"),
        person: matches!(id, "ilsa" | "men"),
        finding_ids: Vec::new(),
    })
    .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn played_investigation_and_item_lines_resolve_to_the_target_jev_chooses() {
    use crate::gameplay::approach_model::tests::{answer, model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let cases = [
        ("I look around.", "room", "perception"),
        ("What do I see in here?", "room", "perception"),
        ("I scan the room for anything odd.", "room", "perception"),
        ("Anything out of place here?", "room", "perception"),
        ("I give the place a good once-over.", "room", "perception"),
        (
            "I look closely at the tankards on the shelf behind the counter.",
            "tankards",
            "perception",
        ),
        (
            "I examine the hearth and the ashes in it.",
            "hearth",
            "investigation",
        ),
        (
            "I check the window. What's outside?",
            "window",
            "perception",
        ),
        (
            "I pick up the candle and look under the table.",
            "table",
            "perception",
        ),
        ("I look under the rug.", "rug", "perception"),
        (
            "I read the notice pinned to the door.",
            "notice",
            "investigation",
        ),
        (
            "Is she telling us the whole truth? I study her face.",
            "ilsa",
            "insight",
        ),
        (
            "I listen at the back door for footsteps.",
            "back-door",
            "perception",
        ),
        (
            "I examine the lock on the cellar door for scratches.",
            "lock",
            "investigation",
        ),
        (
            "I check the floor for muddy tracks. Who came in tonight?",
            "floor",
            "survival",
        ),
        (
            "Does she look ill herself? I check her colour and her breathing.",
            "ilsa",
            "medicine",
        ),
        (
            "What do I know about this place's history?",
            "room",
            "history",
        ),
        (
            "Is there anything magical about this room?",
            "room",
            "arcana",
        ),
        (
            "What are the dried herbs hanging from the beams?",
            "herbs",
            "nature",
        ),
        (
            "Is there a shrine or holy sign anywhere here?",
            "shrine",
            "religion",
        ),
        (
            "I watch the people at the back table. What are they up to?",
            "men",
            "insight",
        ),
        (
            "I keep an eye on the door while we talk.",
            "door",
            "perception",
        ),
        ("I look around.", "room", "perception"),
        (
            "I look around again, more carefully this time.",
            "room",
            "perception",
        ),
        (
            "May I see the letter? I look it over closely for any mark.",
            "letter",
            "investigation",
        ),
        (
            "I lean on the bar and ask the innkeeper who else came asking tonight.",
            "ilsa",
            "speech",
        ),
        ("I look at the moon.", "moon", "perception"),
        (
            "I search Brakka's pack while he isn't looking.",
            "brakka-pack",
            "sleight-of-hand",
        ),
        (
            "I lick the wall to see what it tastes like.",
            "wall",
            "perception",
        ),
        ("I cast detect magic.", "room", "arcana"),
        ("I search behind the counter.", "counter", "perception"),
        ("What is behind the counter?", "counter", "perception"),
        (
            "I pick up the pewter tankard on the bar and look at it.",
            "tankards",
            "perception",
        ),
        (
            "I take a candle from the table and hold it up to look at the shelves.",
            "candle",
            "perception",
        ),
        (
            "I look closely at the bread Ilsa is cutting.",
            "bread",
            "perception",
        ),
        (
            "I examine the dried herbs hanging from the beams. Are any of them the white fever-herb?",
            "herbs",
            "nature",
        ),
        (
            "I look out of the window at the street.",
            "window",
            "perception",
        ),
        ("I pour myself an ale from the jug.", "jug", "perception"),
    ];
    let options = room();
    let replies = cases
        .iter()
        .map(|(_, target_id, _)| {
            let position = options
                .iter()
                .position(|entry| entry.id == *target_id)
                .unwrap();
            answer(&format!("o{}", position + 1))
        })
        .collect();
    let endpoint = FakeEndpoint::start(replies);
    let model = model(&endpoint);
    for (words, target_id, skill) in cases {
        let target = model.resolve_investigation_target(words, &options).await;
        assert_eq!(target.id, target_id, "{words}");
        let observation = Observation {
            target,
            skill: skill.into(),
            succeeded: false,
            repeated: false,
            finding: None,
            private_finding: false,
        };
        let text = observation.fallback();
        assert!(!text.is_empty(), "{words}");
        assert!(!text.contains("nothing unusual"), "{words}: {text}");
    }
}

#[test]
fn moon_and_general_look_do_not_reveal_findings_on_another_target() {
    let scene = super::pack::scene(super::pack::SceneKey::Opening);
    let mut target = room()
        .into_iter()
        .find(|target| target.id == "moon")
        .unwrap();
    assert!(finding_on(&scene, &target, "perception").is_none());
    target.id = "room".into();
    assert!(finding_on(&scene, &target, "perception").is_none());
}

#[test]
fn repeats_and_absence_are_answered_without_a_refusal_or_secret() {
    let target = room()
        .into_iter()
        .find(|target| target.id == "notice")
        .unwrap();
    let mut observation = Observation {
        target,
        skill: "investigation".into(),
        succeeded: true,
        repeated: false,
        finding: None,
        private_finding: false,
    };
    assert_eq!(
        observation.fallback(),
        "The hero observes: There is no notice on the door."
    );
    observation.repeated = true;
    assert_eq!(
        observation.fallback(),
        "The hero has already examined the notice; nothing new turns up."
    );
    assert!(observation.facts()["finding"].is_null());
}

#[test]
fn player_fallback_names_the_hero_and_never_uses_second_person() {
    let target = room()
        .into_iter()
        .find(|target| target.id == "hearth")
        .unwrap();
    let observation = Observation {
        target,
        skill: "investigation".into(),
        succeeded: true,
        repeated: false,
        finding: None,
        private_finding: false,
    };

    let text = observation.fallback_for("Sigrun", None);
    assert!(text.contains("Sigrun"));
    assert!(!text.to_lowercase().contains("you"), "{text}");
}

#[test]
fn roll_text_preserves_apostrophes_and_shows_outcome_without_difficulty_class() {
    let target = room()
        .into_iter()
        .find(|target| target.id == "brakka-pack")
        .unwrap();
    let observation = Observation {
        target,
        skill: "sleight-of-hand".into(),
        succeeded: true,
        repeated: false,
        finding: None,
        private_finding: false,
    };
    let check = df_protocol::common::AbilityCheckResult {
        die: 16,
        ability_modifier: 2,
        proficiency_bonus: 2,
        total: 20,
        difficulty_class: 13,
        succeeded: true,
        ..Default::default()
    };
    let line = observation.roll_line("Sigrun", &check);
    assert!(line.contains("Brakka's pack"));
    assert!(line.contains("Dexterity (Sleight of Hand) 16 + 4 = 20"));
    assert!(line.contains("success"));
    assert!(!line.contains("13"));
}

#[tokio::test(flavor = "current_thread")]
async fn jev_target_choice_uses_scene_names_and_resolves_a_description() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};
    let endpoint = FakeEndpoint::start(vec![Reply::Status(
        200,
        json!({"answers": {"q1": {"type": "choice", "choice": "o14", "confidence": 0.99}},
            "model": "jev-1.13.0", "usage": {"input_tokens": 10, "output_tokens": 1}}),
    )]);
    let provider = SystemOneChoiceProvider::new(
        "jev-fixture-sentinel".into(),
        &format!("http://{}", endpoint.address),
        crate::gameplay::cost::test_meter(),
    )
    .unwrap();
    let selected =
        resolve_target(Some(&provider), "I watch the people at the back.", &room()).await;
    assert_eq!(selected.id, "men");
    let bodies = endpoint.bodies();
    let body = &bodies[0];
    assert_eq!(body["state"]["current_speaker"], "Ilsa");
    assert_eq!(
        body["state"]["targets"][13]["name"],
        "the men at the back table"
    );
    assert!(!body.to_string().contains("enemy-position"));
}

#[tokio::test(flavor = "current_thread")]
async fn target_choice_timeout_is_absent_and_named_by_the_players_words() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};
    let endpoint = FakeEndpoint::start(vec![Reply::DelayedStatus(
        Duration::from_millis(900),
        200,
        json!({"answers": {}}),
    )]);
    let selected = crate::gameplay::approach_model::tests::model(&endpoint)
        .resolve_investigation_target("I search Brakka's pack while he isn't looking.", &room())
        .await;
    assert!(!selected.present);
    assert_eq!(
        selected.name,
        "I search Brakka's pack while he isn't looking"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn storyteller_receives_only_target_facts_and_no_failed_finding() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller};
    let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
        "observation": "Brakka notices the hearth, but cannot make out anything further in its ashes."
    }))]);
    let cache = cache_directory("V08-target-expression");
    let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
    let observation = Observation {
        target: room()
            .into_iter()
            .find(|entry| entry.id == "hearth")
            .unwrap(),
        skill: "investigation".into(),
        succeeded: false,
        repeated: false,
        finding: Some("PRIVATE-FINDING-MUST-NOT-ENTER-PROMPT".into()),
        private_finding: true,
    };
    let answer = describe(&storyteller, &observation).await;
    assert_eq!(
        answer,
        "Brakka notices the hearth, but cannot make out anything further in its ashes."
    );
    let request = serde_json::to_string(&endpoint.bodies()).unwrap();
    assert!(request.contains("Narrate in third person"));
    assert!(request.contains("never address the player as you"));
    assert!(!request.contains("PRIVATE-FINDING-MUST-NOT-ENTER-PROMPT"));
    assert_eq!(observation.facts()["finding"], Value::Null);
}

#[test]
fn storyteller_facts_include_the_acting_hero_and_available_pronouns() {
    let observation = Observation {
        target: room()
            .into_iter()
            .find(|entry| entry.id == "hearth")
            .unwrap(),
        skill: "investigation".into(),
        succeeded: true,
        repeated: false,
        finding: None,
        private_finding: false,
    };
    let pronouns = Pronouns {
        subject: "she",
        object: "her",
        possessive: "her",
        reflexive: "herself",
    };

    let facts = observation.facts_for("Sigrun", Some(pronouns));
    assert_eq!(facts["hero"]["name"], "Sigrun");
    assert_eq!(facts["hero"]["pronouns"]["subject"], "she");
    assert!(!facts.to_string().to_lowercase().contains("you"));
}

#[test]
fn private_findings_do_not_change_public_expression_facts() {
    let target = room().into_iter().find(|entry| entry.id == "ilsa").unwrap();
    let mut observation = Observation {
        target,
        skill: "insight".into(),
        succeeded: true,
        repeated: false,
        finding: Some("Private detail A".into()),
        private_finding: true,
    };
    let first = observation.public_view().facts();
    observation.finding = Some("Private detail B".into());
    assert_eq!(first, observation.public_view().facts());
    assert_eq!(first["finding"], Value::Null);
    assert_eq!(observation.facts()["finding"], "Private detail B");
}

#[test]
fn a_failed_insight_read_says_the_hero_cannot_tell_instead_of_restating_the_appearance() {
    let target = room().into_iter().find(|entry| entry.id == "ilsa").unwrap();
    let name = target.name.clone();
    let observation = Observation {
        target,
        skill: "insight".into(),
        succeeded: false,
        repeated: false,
        finding: None,
        private_finding: false,
    };
    assert_eq!(
        observation.fallback(),
        format!("The hero can't tell what {name} is thinking.")
    );
}

#[test]
fn an_absent_target_fallback_uses_third_person_without_changing_its_identity() {
    let target = Target {
        id: "absent-racks".into(),
        name: "the dark corner behind the racks".into(),
        description: "Nothing of that is visible here.".into(),
        aliases: Vec::new(),
        present: false,
        person: false,
        finding_ids: Vec::new(),
    };
    let observation = Observation {
        target,
        skill: "perception".into(),
        succeeded: true,
        repeated: false,
        finding: None,
        private_finding: false,
    };

    assert_eq!(
        observation.fallback(),
        "The hero observes: Nothing of that is visible here."
    );
    assert_eq!(
        observation.facts()["target"]["name"],
        "the dark corner behind the racks"
    );
}

#[test]
fn exact_target_binding_is_required_even_when_two_targets_use_the_same_skill() {
    let mut scene = pack::scene(pack::SceneKey::Opening);
    scene.findings = vec![pack::Finding {
        id: "counter-clue".into(),
        sentence: "You notice a mark on the counter.".into(),
        skill: pack::FindingSkill::Perception,
        dc: 10,
        private: false,
        consequence: None,
    }];
    let mut counter = room()
        .into_iter()
        .find(|entry| entry.id == "counter")
        .unwrap();
    counter.finding_ids.push("counter-clue".into());
    assert_eq!(
        finding_on(&scene, &counter, "perception").unwrap().id,
        "counter-clue"
    );
    let moon = room().into_iter().find(|entry| entry.id == "moon").unwrap();
    assert!(finding_on(&scene, &moon, "perception").is_none());
    assert!(finding_on(&scene, &counter, "investigation").is_none());
}

fn opening() -> (df_model::checkpoint::Checkpoint, MemberId) {
    let current = crate::gameplay::beat_script_tests::prepared_story();
    let member = MemberId::from_bytes(&[0x61; 16]).unwrap();
    let input =
        crate::gameplay::beat_script_tests::input(&current, member, 5, "begin-story", vec![]);
    let opened = crate::gameplay::journey::stage_with_supplier(&current, &input, &mut |_| {
        panic!("opening must not roll")
    })
    .unwrap();
    (opened, member)
}

#[test]
fn registered_search_accepts_repeated_and_different_targets_without_unrelated_findings() {
    let mut content: pack::PackFile = serde_json::from_slice(include_bytes!(
        "../../../../content/harbor-journey/pack.json"
    ))
    .unwrap();
    content.creation.pick_lists.skill = vec!["perception".into(), "survival".into()];
    content
        .scenes
        .iter_mut()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .findings = vec![
        pack::Finding {
            id: "outside-rats".into(),
            sentence: "Rats wait at the forecourt.".into(),
            skill: pack::FindingSkill::Perception,
            dc: 10,
            private: false,
            consequence: None,
        },
        pack::Finding {
            id: "outside-document".into(),
            sentence: "A document waits at the workshop.".into(),
            skill: pack::FindingSkill::Investigation,
            dc: 10,
            private: false,
            consequence: None,
        },
    ];
    let loaded = pack::load_from_bytes(
        &serde_json::to_vec(&content).unwrap(),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded, || {
        use crate::gameplay::storyteller::tests::FakeEndpoint;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut current, member) = opening();
        // Jev answers the room, the courier, and the absent option for the moon, notice and back
        // door, in the order of the script below.
        let initial = targets(
            &crate::gameplay::beat_script_runtime::scene(current.state()).unwrap(),
            &party_targets(current.state()).unwrap(),
        );
        let absent = format!("o{}", initial.len() + 1);
        let courier = initial
            .iter()
            .position(|entry| entry.id == "person-courier")
            .unwrap();
        let endpoint = FakeEndpoint::start(vec![
            crate::gameplay::approach_model::tests::answer("o1"),
            crate::gameplay::approach_model::tests::answer("o1"),
            crate::gameplay::approach_model::tests::answer(&absent),
            crate::gameplay::approach_model::tests::answer(&absent),
            crate::gameplay::approach_model::tests::answer(&absent),
            crate::gameplay::approach_model::tests::answer(&format!("o{}", courier + 1)),
        ]);
        let model = crate::gameplay::approach_model::tests::model(&endpoint);
        for (index, (words, skill)) in [
            ("I look around.", "perception"),
            (
                "I look around again, more carefully this time.",
                "perception",
            ),
            ("I look at the moon.", "perception"),
            ("I read the notice pinned to the door.", "investigation"),
            ("I listen at the back door for footsteps.", "perception"),
            ("I study her face.", "insight"),
        ]
        .into_iter()
        .enumerate()
        {
            let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
            let catalog = targets(&scene, &party_targets(current.state()).unwrap());
            let target = runtime.block_on(model.resolve_investigation_target(words, &catalog));
            let choices = vec![
                (
                    crate::gameplay::model::label("attempt-skill").unwrap(),
                    crate::gameplay::model::label(skill).unwrap(),
                ),
                (
                    crate::gameplay::model::label("attempt-thing").unwrap(),
                    crate::gameplay::model::label(&target.id).unwrap(),
                ),
            ];
            let input = crate::gameplay::beat_script_tests::input(
                &current,
                member,
                u8::try_from(10 + index).unwrap(),
                "script-choice",
                choices,
            );
            current =
                crate::gameplay::journey::stage_with_supplier(&current, &input, &mut |_| Ok(20))
                    .unwrap_or_else(|error| {
                        panic!(
                            "{words}: {error:?}; step={:?}",
                            crate::gameplay::refusal_detail::take()
                        )
                    });
            let accepted =
                crate::gameplay::journey::accepted(current.state().decisions.last().unwrap())
                    .unwrap();
            let check = accepted.check.unwrap();
            assert!(
                crate::gameplay::beat_script_runtime::observation_finding(&scene, &check).is_none(),
                "{words} revealed a finding on another target"
            );
            let result = observation(&scene, &check).unwrap();
            assert_eq!(result.target.id, target.id, "{words}");
            assert!(!result.fallback().is_empty());
            // Salt Quay now records the courier's own purpose (RP-04). That exact
            // inspection fact is allowed; the unrelated rat/document clues still are not.
            assert_eq!(
                result.finding,
                recorded_finding(&scene, &target, skill).map(|finding| finding.sentence),
                "{words} may reveal only its recorded target's inspection fact"
            );
            if index == 1 {
                assert!(result.repeated);
            }
        }
    });
}

#[test]
fn unseen_search_of_companion_pack_uses_companions_passive_perception() {
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let (current, member) = opening();
    let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
    let catalog = targets(&scene, &party_targets(current.state()).unwrap());
    let vale = catalog
        .iter()
        .position(|entry| entry.name == "Vale's pack")
        .unwrap();
    let endpoint = FakeEndpoint::start(vec![crate::gameplay::approach_model::tests::answer(
        &format!("o{}", vale + 1),
    )]);
    let target = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(
            crate::gameplay::approach_model::tests::model(&endpoint).resolve_investigation_target(
                "I search Vale's pack while he isn't looking.",
                &catalog,
            ),
        );
    assert_eq!(target.name, "Vale's pack");
    let choices = vec![
        (
            crate::gameplay::model::label("attempt-skill").unwrap(),
            crate::gameplay::model::label("sleight-of-hand").unwrap(),
        ),
        (
            crate::gameplay::model::label("attempt-thing").unwrap(),
            crate::gameplay::model::label(&target.id).unwrap(),
        ),
    ];
    let plan = plan(
        &current,
        member,
        &choices,
        OperationId::from_bytes(&[15; 16]).unwrap(),
    )
    .unwrap_or_else(|error| {
        panic!(
            "pack plan: {error:?}; step={:?}",
            crate::gameplay::refusal_detail::take()
        )
    });
    let companion = crate::gameplay::journey::player_entity(
        MemberId::from_bytes(&[0x62; 16]).unwrap(),
        &current,
    )
    .unwrap();
    let passive =
        crate::gameplay::beat_script_runtime::passive_perception(current.state(), companion)
            .unwrap();
    assert_eq!(i16::from(plan.difficulty_class), passive);
    assert_eq!(plan.skill, "sleight-of-hand");
}

#[test]
fn absent_target_key_preserves_apostrophes_and_unicode_without_an_input_panic() {
    use crate::gameplay::approach_model::tests::{answer, model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let scene = pack::scene(pack::SceneKey::Opening);
    // With no scene options, Jev's only option is the absent one.
    let endpoint = FakeEndpoint::start(vec![answer("o1"), answer("o1")]);
    let jev = model(&endpoint);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let target = runtime
        .block_on(jev.resolve_investigation_target("I examine the traveller's café token.", &[]));
    let check = rpc::AbilityCheckResult {
        source_revision: format!("srd521-p6:opening:investigation:target-{}", target.id),
        ..Default::default()
    };
    assert_eq!(
        check_target(&scene, &check).unwrap().name,
        "I examine the traveller's café token"
    );
    let long = runtime.block_on(
        jev.resolve_investigation_target(&format!("I examine the {}.", "é".repeat(100)), &[]),
    );
    let check = rpc::AbilityCheckResult {
        source_revision: format!("srd521-p6:opening:investigation:target-{}", long.id),
        ..Default::default()
    };
    assert!(check_target(&scene, &check).is_some());
    assert!(crate::gameplay::model::label(&long.id).is_ok());
}

#[test]
fn an_absent_notice_is_not_resolved_to_the_door_it_would_be_pinned_to() {
    use crate::gameplay::approach_model::tests::{answer, model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let mut options = room();
    options.retain(|entry| entry.id != "notice");
    let endpoint = FakeEndpoint::start(vec![answer(&format!("o{}", options.len() + 1))]);
    let target = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(
            model(&endpoint)
                .resolve_investigation_target("I read the notice pinned to the door.", &options),
        );
    assert!(!target.present);
    assert!(target.id.starts_with("absent-"));
    assert_eq!(target.name, "I read the notice pinned to the door");
}

#[test]
fn looking_around_the_taproom_does_not_select_the_lodging_offer_named_room() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    let mut scene = generated
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .clone();
    scene.setting = Some(generated.settings.meeting);
    use crate::gameplay::approach_model::tests::{answer, model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let options = targets(&scene, &[]);
    // The room is always the first option listed, so Jev's first answer is the room.
    let endpoint = FakeEndpoint::start(vec![answer("o1")]);
    let target = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(
            model(&endpoint).resolve_investigation_target("I look around the room", &options),
        );
    assert_eq!(target.id, "room");
}

#[test]
fn narration_sentence_is_not_promoted_to_an_investigation_target() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    let mut scene = generated
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .clone();
    scene.narration = "The courier presses the packet against his chest.".into();
    scene.setting = Some(generated.settings.meeting);
    use crate::gameplay::approach_model::tests::{answer, model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let options = targets(&scene, &[]);
    let endpoint = FakeEndpoint::start(vec![answer(&format!("o{}", options.len() + 1))]);
    let target = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(model(&endpoint).resolve_investigation_target(
            "I inspect the courier presses the packet against his.",
            &options,
        ));
    assert!(!target.present);
    assert!(target.id.starts_with("absent-"));
}

#[test]
fn searching_an_absent_satchel_does_not_search_its_owners_face() {
    use crate::gameplay::approach_model::tests::{answer, model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let mut options = room();
    options.push(Target {
        id: "person-innkeeper".into(),
        name: "Ilsa".into(),
        description: "Ilsa watches the room.".into(),
        aliases: vec!["Ilsa".into(), "her".into()],
        person: true,
        present: true,
        finding_ids: Vec::new(),
    });
    // The innkeeper is the last option, so Jev's absent answer follows her entry.
    let endpoint = FakeEndpoint::start(vec![answer(&format!("o{}", options.len() + 1))]);
    let target = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(model(&endpoint).resolve_investigation_target("I search her satchel", &options));
    assert!(!target.present);
    assert!(!target.person);
    assert_eq!(target.name, "I search her satchel");
}
#[test]
fn the_phrase_gives_common_nouns_their_article_and_leaves_names_and_articles_alone() {
    assert_eq!(the_phrase("folded paper label"), "the folded paper label");
    assert_eq!(the_phrase("street door"), "the street door");
    assert_eq!(
        the_phrase("the market street outside"),
        "the market street outside"
    );
    assert_eq!(the_phrase("a wooden counter"), "a wooden counter");
    assert_eq!(the_phrase("Corin's pack"), "Corin's pack");
    assert_eq!(sentence_the("folded paper label"), "The folded paper label");
    assert_eq!(sentence_the("Corin's pack"), "Corin's pack");
}

#[test]
fn authored_fragments_become_capitalised_sentences_with_a_full_stop() {
    assert_eq!(
        room_sentence("a small cloth shop with a wooden counter"),
        "You are in a small cloth shop with a wooden counter."
    );
    assert_eq!(room_sentence("A quiet taproom."), "A quiet taproom.");
    assert_eq!(
        narration_sentence("a practical set of traveler's clothes"),
        "A practical set of traveler's clothes."
    );
}

#[test]
fn every_looked_at_target_reads_as_a_grammatical_sentence() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    let mut scene = generated
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .clone();
    scene.setting = Some(generated.settings.meeting);
    let options = targets(&scene, &[]);
    assert!(options.iter().any(|target| target.id == "room"));
    for option in &options {
        let first = option.description.chars().next();
        assert!(
            first.is_some_and(char::is_uppercase),
            "description must open with a capital: {}",
            option.description
        );
        assert!(
            option.description.ends_with(['.', '!', '?']),
            "description must end a sentence: {}",
            option.description
        );
    }
    let way = options
        .iter()
        .find(|target| target.id.starts_with("way-"))
        .expect("the scene has a way out");
    assert!(
        way.description.contains(" leads to "),
        "{}",
        way.description
    );
}

#[test]
fn scene_names_accept_ordinary_synonyms_without_matching_nearby_objects() {
    use crate::gameplay::approach_model::tests::answer;
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let mut scene = pack::scene(pack::SceneKey::Opening);
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    scene.setting = Some(generated.settings.meeting);
    let model = scene.setting.as_mut().unwrap().model.as_mut().unwrap();
    // The bar is named by object and person positions, rather than a separate prop.
    let expected = "surface-bar";
    let mug = model
        .things
        .iter_mut()
        .find(|thing| thing.name.contains("tankard"))
        .unwrap();
    mug.aliases = vec!["tankard".into()];
    let mug_id = format!("thing-{}", mug.id);
    let options = targets(&scene, &[]);
    let bar_at = options
        .iter()
        .position(|entry| entry.id == expected)
        .unwrap();
    let mug_at = options.iter().position(|entry| entry.id == mug_id).unwrap();
    let endpoint = FakeEndpoint::start(vec![
        answer(&format!("o{}", bar_at + 1)),
        answer(&format!("o{}", mug_at + 1)),
    ]);
    let jev = crate::gameplay::approach_model::tests::model(&endpoint);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(
        runtime
            .block_on(jev.resolve_investigation_target("I look behind the counter.", &options))
            .id,
        expected
    );
    assert_eq!(
        runtime
            .block_on(jev.resolve_investigation_target("I examine the mug.", &options))
            .id,
        mug_id
    );
}

#[tokio::test(flavor = "current_thread")]
async fn jev_absent_answer_is_not_replaced_by_an_alias_match() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};
    let options = room();
    // The absent option follows every scene name, so its choice label is one past the last name.
    let absent_choice = format!("o{}", options.len() + 1);
    let endpoint = FakeEndpoint::start(vec![Reply::Status(
        200,
        json!({"answers": {"q1": {"type": "choice", "choice": absent_choice, "confidence": 0.9}},
            "model": "jev-1.13.0", "usage": {"input_tokens": 10, "output_tokens": 1}}),
    )]);
    let provider = SystemOneChoiceProvider::new(
        "jev-fixture-sentinel".into(),
        &format!("http://{}", endpoint.address),
        crate::gameplay::cost::test_meter(),
    )
    .unwrap();
    // "tankards" is a scene alias, so only Jev's absent answer keeps it out of the scene.
    let selected = resolve_target(Some(&provider), "I examine the tankards.", &options).await;
    assert!(!selected.present);
    assert!(selected.id.starts_with("absent-"));
}

#[tokio::test(flavor = "current_thread")]
async fn jev_absent_answer_gives_the_absent_target_named_by_the_words() {
    use crate::gameplay::approach_model::tests::{answer, model};
    use crate::gameplay::storyteller::tests::FakeEndpoint;
    let options = room();
    let words = "I examine the tankards.";
    let endpoint = FakeEndpoint::start(vec![answer(&format!("o{}", options.len() + 1))]);
    let selected = model(&endpoint)
        .resolve_investigation_target(words, &options)
        .await;
    assert_eq!(selected, absent_target(words));
}

fn merchant_shop_room() -> pack::Scene {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/merchant-shop/pack.json"),
        include_bytes!("../../../../content/merchant-shop/tuning.json"),
    )
    .expect("the merchant shop pack loads");
    let model = serde_json::from_slice(include_bytes!(
        "../../../../content/merchant-shop/scene-model.json"
    ))
    .expect("the merchant shop scene model");
    let mut scene = loaded
        .pack
        .scenes
        .into_iter()
        .find(|scene| scene.key == pack::SceneKey::Room)
        .expect("the shop has a room scene");
    scene
        .setting
        .as_mut()
        .expect("the room has a setting")
        .model = Some(model);
    scene
}

#[test]
fn an_insight_read_names_the_acting_hero_instead_of_the_hero() {
    let scene = merchant_shop_room();
    let catalog = targets(&scene, &[]);
    let marek = catalog
        .iter()
        .find(|target| target.name == "Marek")
        .expect("Marek is a catalog target");
    let read = state_of_mind_for(&scene, marek, None, "Brakka")
        .expect("Marek's state of mind is modelled");
    assert!(read.starts_with("Marek seems friendly"), "{read}");
    assert!(read.contains("toward Brakka"), "{read}");
    assert!(!read.contains("the hero"), "{read}");
}

#[test]
fn a_present_person_is_choosable_by_role_as_well_as_by_name() {
    let scene = merchant_shop_room();
    let catalog = targets(&scene, &[]);
    let marek = catalog
        .iter()
        .find(|target| target.name == "Marek")
        .expect("Marek is a catalog target");
    assert!(marek.person);
    assert!(
        marek.description.contains("cloth merchant"),
        "{}",
        marek.description
    );
    assert!(
        marek.aliases.iter().any(|alias| alias == "cloth merchant"),
        "{:?}",
        marek.aliases
    );
}

#[tokio::test(flavor = "current_thread")]
async fn without_jev_the_target_is_absent_even_when_the_words_name_a_present_thing() {
    use crate::gameplay::approach_model::ApproachModel;
    // No key means no provider, so nothing is requested and no scene thing is named on a guess.
    let options = room();
    let selected = ApproachModel::configured(None)
        .resolve_investigation_target("I examine the tankards.", &options)
        .await;
    assert!(!selected.present);
    assert!(selected.id.starts_with("absent-"));
    assert_eq!(selected.name, "I examine the tankards");
}

#[test]
fn plain_prices_and_lists_read_as_the_table_says_them() {
    assert_eq!(price_text(10), "1 sp");
    assert_eq!(price_text(400), "4 gp");
    assert_eq!(price_text(7), "7 cp");
    let items = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
    assert_eq!(listed(&items[..1]), "a");
    assert_eq!(listed(&items[..2]), "a and b");
    assert_eq!(listed(&items), "a, b and c");
    assert_eq!(
        place_sentence("The Market Yard", "a cobbled yard where coaches wait"),
        "The Market Yard is a cobbled yard where coaches wait."
    );
}

#[test]
fn a_plain_look_names_the_hero_and_the_place_and_never_says_you() {
    let look = PlainLook {
        hero: "Brakka".to_owned(),
        place: "The Open Market".to_owned(),
        subject: None,
        facts: vec!["Tavin, the fruit seller, is at his stall.".to_owned()],
        atmosphere: "You step into the market.".to_owned(),
    };
    let line = look.fallback();
    assert_eq!(
        line,
        "Brakka looks around The Open Market. Tavin, the fruit seller, is at his stall."
    );
    assert!(!line.contains("You") && !line.contains("the room"));
}

#[test]
fn successful_person_read_and_item_inspection_have_recorded_findings() {
    let scene = merchant_shop_room();
    let check_for = |skill: &str, target: &str| rpc::AbilityCheckResult {
        succeeded: true,
        source_revision: format!("srd521-p6:room:{skill}:target-{target}"),
        ..Default::default()
    };
    let person = observation(&scene, &check_for("insight", "person-merchant")).unwrap();
    let finding = person
        .finding
        .as_ref()
        .expect("a successful read shows the recorded want");
    assert!(
        finding.contains("Sell suitable clothing at a fair price"),
        "{finding}"
    );
    assert!(!finding.contains("is here"));
    assert!(person.fallback_for("Pip", None).contains(finding));
    assert!(person.public_view().fallback().contains(finding));
    let item = observation(&scene, &check_for("investigation", "thing-shop-counter")).unwrap();
    assert!(
        item.finding
            .unwrap()
            .contains("years of repeated measurements")
    );
}

#[test]
fn failed_item_inspection_cannot_reveal_the_recorded_closer_look() {
    let mut check = rpc::AbilityCheckResult {
        source_revision: "srd521-p6:room:investigation:target-thing-shop-counter".into(),
        ..Default::default()
    };
    let scene = merchant_shop_room();
    let failed = observation(&scene, &check).unwrap();
    assert!(failed.finding.is_none());
    let line = failed.fallback_for("Pip", None);
    assert!(line.contains("can't make out any further detail"), "{line}");
    assert!(
        failed
            .fallback()
            .contains("can't make out any further detail")
    );
    assert!(
        !failed
            .facts()
            .to_string()
            .contains("years of repeated measurements")
    );
    check.succeeded = true;
    check.source_revision = "srd521-p6:room:investigation:repeat-target-thing-shop-counter".into();
    let repeated = observation(&scene, &check).unwrap();
    assert!(repeated.finding.is_none());
    assert!(repeated.fallback().contains("already examined"));
}

#[test]
fn item_inspection_has_the_source_price_and_recorded_condition() {
    let scene = merchant_shop_room();
    let options = offered_targets(&scene);
    let item = options
        .iter()
        .find(|target| target.id == "offer-warm-cloak-wool")
        .unwrap();
    let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
    let offer = model
        .offers
        .iter()
        .find(|offer| offer.id == "warm-cloak-wool")
        .unwrap();
    assert!(item.description.contains("3 gp"), "{}", item.description);
    assert!(
        item.description
            .contains(offer.description.trim_end_matches('.'))
    );
    let check = rpc::AbilityCheckResult {
        succeeded: true,
        source_revision: "srd521-p6:room:investigation:target-offer-warm-cloak-wool".into(),
        ..Default::default()
    };
    let inspected = observation(&scene, &check).unwrap();
    let line = inspected.fallback_for("Pip", None);
    assert!(line.contains("Pip inspects"));
    assert!(line.contains("3 gp"));
    assert!(line.contains(offer.description.trim_end_matches('.')));
    let failed = observation(
        &scene,
        &rpc::AbilityCheckResult {
            succeeded: false,
            ..check
        },
    )
    .unwrap();
    let failure_line = failed.fallback_for("Pip", None);
    assert!(failure_line.contains("can't make out"));
    assert!(failure_line.contains("3 gp"));
    assert!(failure_line.contains(offer.description.trim_end_matches('.')));
    assert!(failed.public_view().fallback().contains("3 gp"));
}

#[test]
fn look_around_offers_people_goods_and_props_beyond_the_old_eight_option_limit() {
    let mut scene = merchant_shop_room();
    let options = offered_targets(&scene);
    assert!(options.len() > 8);
    assert!(options.iter().any(|target| target.id == "person-merchant"));
    assert!(
        options
            .iter()
            .any(|target| target.id == "thing-shop-counter")
    );
    assert!(
        options
            .iter()
            .any(|target| target.id == "offer-clothes-fine")
    );
    let model = scene.setting.as_mut().unwrap().model.as_mut().unwrap();
    let hidden = model
        .things
        .iter_mut()
        .find(|thing| thing.id == "folded-label")
        .unwrap();
    hidden.publicly_visible = false;
    assert!(
        !offered_targets(&scene)
            .iter()
            .any(|target| target.id == "thing-folded-label")
    );
}

#[test]
fn legacy_watch_finding_requires_its_target_binding_and_insight_skill() {
    let mut scene = merchant_shop_room();
    scene.speaker = "Marek".into();
    scene.cast[0].id = "merchant".into();
    scene.cast[0].name = "Marek".into();
    scene.findings.push(pack::Finding {
        id: "errand-truth".into(),
        sentence: "Marek wants the frayed cloth sold before winter.".into(),
        skill: pack::FindingSkill::Insight,
        dc: 10,
        private: false,
        consequence: None,
    });
    let mut check = rpc::AbilityCheckResult {
        succeeded: true,
        source_revision: "srd521-p6:room:insight:watch-merchant".into(),
        ..Default::default()
    };
    let found = crate::gameplay::beat_script_runtime::observation_finding(&scene, &check).unwrap();
    assert_eq!(found.id, "errand-truth");
    check.succeeded = false;
    assert!(crate::gameplay::beat_script_runtime::observation_finding(&scene, &check).is_none());
}

#[tokio::test]
async fn observation_completion_jev_selects_real_source_once_and_expression_sees_only_it() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    let scene = merchant_shop_room();
    let check = rpc::AbilityCheckResult {
        succeeded: true,
        total: 18,
        source_revision: "srd521-p6:room:investigation:target-thing-shop-counter".into(),
        ..Default::default()
    };
    let sources = observation_sources(&scene, &check);
    assert!(
        sources
            .iter()
            .any(|source| source.text.contains("repeated measurements"))
    );
    let jev = FakeEndpoint::start(vec![answer("o2")]);
    let text = "Pip notices the marks left by years of repeated measurements.";
    let expression = FakeEndpoint::start(vec![Reply::Output(json!({"finding": text}))]);
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/HS23/test-expression-success-{}",
        std::process::id()
    ));
    let storyteller = teller(expression.address, 20_000, Duration::from_secs(5), &cache);
    let (ids, line, unavailable) =
        complete_finding(&storyteller, &model(&jev), &scene, &check, "Pip").await;
    assert_eq!(ids, vec![sources[0].id.clone()]);
    assert_eq!(line, text);
    assert!(!unavailable);
    assert_eq!(jev.connections(), 1);
    assert_eq!(expression.connections(), 1);
    assert!(!expression.bodies()[0].to_string().contains("knowledge"));
}

#[tokio::test]
async fn observation_completion_failure_and_provider_failure_never_disclose_success() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    let mut scene = merchant_shop_room();
    scene.findings.push(pack::Finding {
        id: "private-proof".into(),
        sentence: "PRIVATE-CANARY-DESTINATION".into(),
        skill: pack::FindingSkill::Investigation,
        dc: 5,
        private: true,
        consequence: None,
    });
    let check = rpc::AbilityCheckResult {
        succeeded: false,
        total: 4,
        source_revision: "srd521-p6:room:investigation:target-thing-shop-counter".into(),
        ..Default::default()
    };
    let jev = FakeEndpoint::start(vec![answer("o1")]);
    let expression = FakeEndpoint::start(vec![Reply::Output(
        json!({"finding": "Pip cannot make out any further detail."}),
    )]);
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/HS23/test-expression-failure-{}",
        std::process::id()
    ));
    let storyteller = teller(expression.address, 20_000, Duration::from_secs(5), &cache);
    let (ids, line, unavailable) =
        complete_finding(&storyteller, &model(&jev), &scene, &check, "Pip").await;
    assert!(ids.is_empty());
    assert_eq!(line, "Pip cannot make out any further detail.");
    assert!(!unavailable);
    for body in jev.bodies().into_iter().chain(expression.bodies()) {
        assert!(!body.to_string().contains("PRIVATE-CANARY"));
    }
    let succeeded = rpc::AbilityCheckResult {
        succeeded: true,
        total: 20,
        ..check
    };
    let (ids, line, unavailable) = complete_finding(
        &Storyteller::disabled(),
        &Default::default(),
        &scene,
        &succeeded,
        "Pip",
    )
    .await;
    assert!(ids.is_empty());
    assert!(unavailable);
    assert!(line.contains("can't make it out"));
    assert!(!line.contains("repeated measurements"));
    let jev = FakeEndpoint::start(vec![answer("o2")]);
    let (ids, line, unavailable) = complete_finding(
        &Storyteller::disabled(),
        &model(&jev),
        &scene,
        &succeeded,
        "Pip",
    )
    .await;
    assert!(ids.is_empty());
    assert!(unavailable);
    assert!(line.contains("can't make it out"));
    assert_eq!(jev.connections(), 1);
}

#[test]
fn observation_completion_private_facts_and_harder_dc_are_not_eligible_sources() {
    let mut scene = merchant_shop_room();
    let check = rpc::AbilityCheckResult {
        succeeded: true,
        total: 10,
        source_revision: "srd521-p6:room:investigation:target-thing-shop-counter".into(),
        ..Default::default()
    };
    let model = scene.setting.as_mut().unwrap().model.as_mut().unwrap();
    model.people[0].knowledge[0].statement = "PRIVATE-ALPHA".into();
    let first = serde_json::to_value(observation_sources(&scene, &check)).unwrap();
    scene
        .setting
        .as_mut()
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .people[0]
        .knowledge[0]
        .statement = "PRIVATE-BETA".into();
    assert_eq!(
        first,
        serde_json::to_value(observation_sources(&scene, &check)).unwrap()
    );
    let counter = scene
        .setting
        .as_mut()
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .things
        .iter_mut()
        .find(|thing| thing.id == "shop-counter")
        .unwrap();
    counter.dc = 25;
    counter.closer_look = "HARDER-CANARY".into();
    assert!(
        !serde_json::to_string(&observation_sources(&scene, &check))
            .unwrap()
            .contains("HARDER-CANARY")
    );
    counter_private_sources(&mut scene, &check);
}

fn counter_private_sources(scene: &mut pack::Scene, check: &rpc::AbilityCheckResult) {
    let counter = scene
        .setting
        .as_mut()
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .things
        .iter_mut()
        .find(|thing| thing.id == "shop-counter")
        .unwrap();
    counter.publicly_visible = false;
    counter.picture_description = "PRIVATE-OBJECT-CANARY".into();
    assert!(
        !serde_json::to_string(&observation_sources(scene, check))
            .unwrap()
            .contains("PRIVATE-OBJECT-CANARY")
    );
}

#[test]
fn observation_journal_duplicate_stale_and_phone_tv_notebook_use_exact_stored_text() {
    use crate::gameplay::{beat_script_runtime, journey, model, notebook, wire};
    use prost::Message;
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded, || {
        let (current, member) = opening();
        let choices = [
            ("attempt-skill", "insight"),
            ("attempt-thing", "person-courier"),
        ]
        .into_iter()
        .map(|(group, option)| (model::label(group).unwrap(), model::label(option).unwrap()))
        .collect();
        let input = crate::gameplay::beat_script_tests::input(
            &current,
            member,
            77,
            "script-choice",
            choices,
        );
        let checked = journey::stage_with_supplier(&current, &input, &mut |_| Ok(18)).unwrap();
        let check = journey::accepted(checked.state().decisions.last().unwrap())
            .unwrap()
            .check
            .unwrap();
        let scene = beat_script_runtime::scene(checked.state()).unwrap();
        let operation = checked.state().decisions.last().unwrap().operation;
        let who = journey::player_entity(member, &checked).unwrap();
        let sources = observation_sources(&scene, &check);
        assert!(
            sources
                .iter()
                .any(|source| source.text != "Courier is here.")
        );
        let record = journey::living_scene::ObservationFinding {
            id: format!("observation-{}", journey::hex(operation.as_bytes())),
            operation: journey::hex(operation.as_bytes()),
            actor: journey::hex(who.as_bytes()),
            hero: journey::name(checked.state(), who).unwrap(),
            audience: "shared".into(),
            check: journey::hex(&check.encode_to_vec()),
            target: "person-courier".into(),
            source_revision: check.source_revision.clone(),
            source_epoch: checked.basis().revision.epoch().get(),
            source_sequence: checked.basis().revision.sequence(),
            source_hash: journey::living_scene::observation_source_hash(&scene).unwrap(),
            scene: scene.key,
            selected_source_ids: vec![sources[0].id.clone()],
            text: format!("Brynn notices: {}", sources[0].text),
            unavailable: false,
        };
        let completed =
            journey::living_scene::commit_observation(&checked, checked.basis(), record.clone())
                .unwrap();
        let duplicate =
            journey::living_scene::commit_observation(&completed, checked.basis(), record.clone())
                .unwrap();
        assert_eq!(duplicate, completed);
        assert_eq!(
            journey::living_scene::observation_for(completed.state(), operation).unwrap(),
            Some(record.clone())
        );
        let mut stale = checked.basis();
        stale.revision = stale.revision.next_sequence().unwrap();
        assert!(
            journey::living_scene::commit_observation(&checked, stale, record.clone()).is_err()
        );
        let mut wrong_source = record.clone();
        wrong_source.source_hash = "changed".into();
        assert!(
            journey::living_scene::commit_observation(&checked, checked.basis(), wrong_source)
                .is_err()
        );
        let mut tampered = completed.state().clone();
        let decision = tampered.decisions.last_mut().unwrap();
        let text = decision
            .semantic_output
            .as_deref()
            .unwrap()
            .strip_prefix("living-scene-2:")
            .unwrap();
        let mut encoded: serde_json::Value = serde_json::from_str(text).unwrap();
        encoded["observation"]["source_sequence"] = json!(record.source_sequence + 1);
        decision.semantic_output = Some(format!("living-scene-2:{}", encoded));
        assert!(journey::living_scene::observation_journal(&tampered).is_err());
        let projected = beat_script_runtime::scene(completed.state()).unwrap();
        assert_eq!(
            beat_script_runtime::observation_finding(&projected, &check)
                .unwrap()
                .sentence,
            record.text
        );
        let phone = wire::journey_view(
            &completed,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            member,
        )
        .unwrap();
        let Some(rpc::view_message::Audience::Player(phone)) = phone.audience else {
            panic!("phone audience");
        };
        assert_eq!(phone.narration, record.text);
        assert_eq!(phone.check, Some(check.clone()));
        let tv = wire::journey_view(
            &completed,
            df_persistence::local_demo_scope::LocalDemoRole::Display,
            member,
        )
        .unwrap();
        let Some(rpc::view_message::Audience::Display(tv)) = tv.audience else {
            panic!("tv audience");
        };
        assert_eq!(tv.check, Some(check));
        let ledger = notebook::NotebookLedger::none();
        let other_member = MemberId::from_bytes(&[0x62; 16]).unwrap();
        ledger
            .add_clue(
                member,
                &record.hero,
                &record.id,
                &record.text,
                "The Broken Seal",
            )
            .unwrap();
        assert!(ledger.visible_to(other_member).is_empty());
        let table = crate::gameplay::table_log::TableLog::new();
        let mut published = Default::default();
        for _ in 0..2 {
            crate::gameplay::actor::materialize_observation(
                &table,
                &ledger,
                &mut published,
                member,
                &record,
            )
            .unwrap();
        }
        assert_eq!(ledger.visible_to(member)[0].text, record.text);
        assert_eq!(ledger.visible_to(member).len(), 1);
        assert_eq!(ledger.visible_to(other_member)[0].text, record.text);
        assert_eq!(table.lines().last().unwrap().text, record.text);
        assert_eq!(table.lines().len(), 1);
    });
}

#[test]
fn salt_quay_opening_catalog_has_recorded_visible_objects_and_safe_inspection_sources() {
    let content: pack::PackFile = serde_json::from_slice(include_bytes!(
        "../../../../content/harbor-journey/pack.json"
    ))
    .unwrap();
    let scene = content
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap();
    let model = scene.setting.as_ref().unwrap().model.as_ref().unwrap();
    model.validate_structure().unwrap();
    let offers = offered_targets(scene);
    for id in [
        "sealed-packet",
        "broken-seal",
        "salted-fish-crates",
        "moored-boat",
        "rope-coils",
        "lantern-post",
    ] {
        assert!(
            offers
                .iter()
                .any(|target| target.id == format!("thing-{id}")),
            "missing {id}"
        );
    }
    let check = rpc::AbilityCheckResult {
        succeeded: true,
        total: 13,
        source_revision: "srd521-p6:opening:insight:target-person-courier".into(),
        ..Default::default()
    };
    let sources = serde_json::to_string(&observation_sources(scene, &check)).unwrap();
    assert!(sources.contains("delivering the packet"));
    assert!(!sources.contains("Vell"));
    assert!(!sources.contains("Harbor Inn"));
}

#[tokio::test]
async fn observation_completion_hidden_target_names_do_not_change_jev_or_expression_prompts() {
    use crate::gameplay::{
        approach_model::tests::{answer, model},
        storyteller::tests::{FakeEndpoint, Reply, teller},
    };
    let mut scene = merchant_shop_room();
    let check = rpc::AbilityCheckResult {
        succeeded: true,
        total: 20,
        source_revision: "srd521-p6:room:investigation:target-thing-shop-counter".into(),
        ..Default::default()
    };
    let jev = FakeEndpoint::start(vec![answer("o1"), answer("o1")]);
    let line = "Pip cannot make out anything certain about the scene.";
    let expression = FakeEndpoint::start(vec![Reply::Output(json!({"finding": line}))]);
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/HS23/hidden-target-{}",
        std::process::id()
    ));
    let storyteller = teller(expression.address, 20_000, Duration::from_secs(5), &cache);
    let mut outcomes = Vec::new();
    for hidden in ["PRIVATE-NAME-ALPHA", "PRIVATE-NAME-BETA"] {
        let thing = scene
            .setting
            .as_mut()
            .unwrap()
            .model
            .as_mut()
            .unwrap()
            .things
            .iter_mut()
            .find(|thing| thing.id == "shop-counter")
            .unwrap();
        thing.publicly_visible = false;
        thing.name = hidden.into();
        thing.picture_description = format!("PRIVATE-PICTURE-{hidden}");
        outcomes.push(complete_finding(&storyteller, &model(&jev), &scene, &check, "Pip").await);
    }
    assert_eq!(outcomes[0], outcomes[1]);
    assert_eq!(outcomes[0].1, line);
    for body in jev.bodies().into_iter().chain(expression.bodies()) {
        assert!(!body.to_string().contains("PRIVATE-NAME"));
        assert!(!body.to_string().contains("PRIVATE-PICTURE"));
    }
}

#[test]
fn private_observation_grants_ignore_shared_projection_rows_with_identical_check_bytes() {
    use prost::Message;
    let mut scene = merchant_shop_room();
    let person = scene
        .setting
        .as_mut()
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .people
        .iter_mut()
        .find(|person| person.id == "merchant")
        .unwrap();
    person.knowledge[0].fact_id = "private-inspection".into();
    let check = rpc::AbilityCheckResult {
        succeeded: true,
        total: 18,
        source_revision: "srd521-p6:room:insight:target-person-merchant".into(),
        ..Default::default()
    };
    scene.findings.push(pack::Finding {
        id: "private-inspection".into(),
        sentence: "MEMBER-PRIVATE-FACT".into(),
        skill: pack::FindingSkill::Insight,
        dc: 10,
        private: true,
        consequence: None,
    });
    scene.findings.push(pack::Finding {
        id: "observation-prior-hero".into(),
        sentence: "First hero's public line.".into(),
        skill: pack::FindingSkill::Perception,
        dc: 0,
        private: false,
        consequence: Some(format!(
            "observation-check:{}",
            crate::gameplay::journey::hex(&check.encode_to_vec())
        )),
    });
    for _hero in ["first hero", "second hero"] {
        assert_eq!(
            private_observation_finding(&scene, &check)
                .unwrap()
                .sentence,
            "MEMBER-PRIVATE-FACT"
        );
    }
    assert!(
        !serde_json::to_string(&observation_sources(&scene, &check))
            .unwrap()
            .contains("MEMBER-PRIVATE-FACT")
    );
}

#[test]
fn hs56_investigation_keeps_public_surfaces_and_ways_before_stacks_and_omits_private_custody() {
    let mut scene = merchant_shop_room();
    let model = scene.setting.as_mut().unwrap().model.as_mut().unwrap();
    let mut template = model.things[0].clone();
    template.portable = true;
    template.finding_id = None;
    template.clue_fact_id = None;
    for index in 0..80 {
        let mut thing = template.clone();
        thing.id = format!("equipment-private-stack-{index}");
        thing.name = format!("private stack {index}");
        thing.publicly_visible = false;
        thing.holder_id = Some("Other Hero".into());
        model.things.push(thing);
    }
    for index in 0..80 {
        let mut thing = template.clone();
        thing.id = format!("public-stack-{index}");
        thing.name = format!("public stack {index}");
        thing.publicly_visible = true;
        model.things.push(thing);
    }
    let way = model.ways_out[0].id.clone();
    let targets = targets(&scene, &[]);
    assert_eq!(targets.len(), 64);
    assert!(
        targets
            .iter()
            .all(|target| !target.id.contains("equipment-private-stack-"))
    );
    assert!(
        targets
            .iter()
            .any(|target| target.id == format!("way-{way}"))
    );
    assert!(
        targets.iter().any(|target| target.id == "surface-counter"
            || target.name.to_lowercase().contains("counter"))
    );
}
