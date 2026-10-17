use super::*;

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

#[test]
fn played_investigation_and_item_lines_keep_their_target_and_book_skill() {
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
    for (words, target_id, skill) in cases {
        let target = local_target(words, &room());
        assert_eq!(target.id, target_id, "{words}");
        assert_eq!(local_skill(words), skill, "{words}");
        if skill != "speech" {
            let reading = local_reading(words, &room())
                .unwrap_or_else(|| panic!("unrouted attempt: {words}"));
            assert!(reading.intent.is_attempt(), "{words}");
            assert_eq!(reading.skill, Some(skill), "{words}");
            let observation = Observation {
                target,
                skill: skill.into(),
                succeeded: false,
                repeated: false,
                finding: None,
                private_finding: false,
            };
            let answer = observation.fallback();
            assert!(!answer.is_empty(), "{words}");
            assert!(!answer.contains("nothing unusual"), "{words}: {answer}");
        }
    }
}

#[test]
fn conversational_looking_forward_and_listen_address_do_not_become_checks() {
    assert!(local_reading("I look forward to it.", &room()).is_none());
    assert!(local_reading("Listen, old man.", &room()).is_none());
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
    assert_eq!(observation.fallback(), "There is no notice on the door.");
    observation.repeated = true;
    assert!(observation.fallback().contains("already"));
    assert!(observation.fallback().contains("notice"));
    assert!(observation.facts()["finding"].is_null());
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
async fn target_choice_timeout_preserves_the_named_target() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};
    let endpoint = FakeEndpoint::start(vec![Reply::DelayedStatus(
        Duration::from_millis(900),
        200,
        json!({"answers": {}}),
    )]);
    let provider = SystemOneChoiceProvider::new(
        "jev-fixture-sentinel".into(),
        &format!("http://{}", endpoint.address),
    )
    .unwrap();
    let selected = resolve_target(
        Some(&provider),
        "I search Brakka's pack while he isn't looking.",
        &room(),
    )
    .await;
    assert_eq!(selected.name, "Brakka's pack");
}

#[tokio::test(flavor = "current_thread")]
async fn storyteller_receives_only_target_facts_and_no_failed_finding() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller};
    let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
        "observation": "You see the hearth, but cannot make out anything further in its ashes."
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
        "You see the hearth, but cannot make out anything further in its ashes."
    );
    assert!(
        !serde_json::to_string(&endpoint.bodies())
            .unwrap()
            .contains("PRIVATE-FINDING-MUST-NOT-ENTER-PROMPT")
    );
    assert_eq!(observation.facts()["finding"], Value::Null);
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
        format!("You can't tell what {name} is thinking.")
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
        let (mut current, member) = opening();
        for (index, words) in [
            "I look around.",
            "I look around again, more carefully this time.",
            "I look at the moon.",
            "I read the notice pinned to the door.",
            "I listen at the back door for footsteps.",
            "I study her face.",
        ]
        .into_iter()
        .enumerate()
        {
            let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
            let catalog = targets(&scene, &party_targets(current.state()).unwrap());
            let target = local_target(words, &catalog);
            let skill = local_skill(words);
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
            assert!(
                result.finding.is_none(),
                "{words} must not reveal a clue on another target"
            );
            if index == 1 {
                assert!(result.repeated);
            }
        }
    });
}

#[test]
fn unseen_search_of_companion_pack_uses_companions_passive_perception() {
    let (current, member) = opening();
    let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
    let catalog = targets(&scene, &party_targets(current.state()).unwrap());
    let target = local_target("I search Vale's pack while he isn't looking.", &catalog);
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
    let scene = pack::scene(pack::SceneKey::Opening);
    let target = local_target("I examine the traveller's café token.", &[]);
    let check = rpc::AbilityCheckResult {
        source_revision: format!("srd521-p6:opening:investigation:target-{}", target.id),
        ..Default::default()
    };
    assert_eq!(
        check_target(&scene, &check).unwrap().name,
        "the traveller's café token"
    );
    let long = local_target(&format!("I examine the {}.", "é".repeat(100)), &[]);
    let check = rpc::AbilityCheckResult {
        source_revision: format!("srd521-p6:opening:investigation:target-{}", long.id),
        ..Default::default()
    };
    assert!(check_target(&scene, &check).is_some());
    assert!(crate::gameplay::model::label(&long.id).is_ok());
}

#[test]
fn an_absent_notice_is_not_resolved_to_the_door_it_would_be_pinned_to() {
    let mut options = room();
    options.retain(|entry| entry.id != "notice");
    let target = local_target("I read the notice pinned to the door.", &options);
    assert!(!target.present);
    assert_eq!(target.name, "the notice");
    assert!(target.description.contains("no notice"));
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
    let options = targets(&scene, &[]);
    assert_eq!(local_target("I look around the room", &options).id, "room");
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
    let options = targets(&scene, &[]);
    let target = local_target(
        "I inspect the courier presses the packet against his.",
        &options,
    );
    assert!(!target.present);
    assert!(target.id.starts_with("absent-"));
}

#[test]
fn searching_an_absent_satchel_does_not_search_its_owners_face() {
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
    let target = local_target("I search her satchel", &options);
    assert!(!target.present);
    assert!(!target.person);
    assert_eq!(target.name, "her satchel");
}
#[test]
fn looking_behind_the_counter_is_an_investigation_in_the_keyless_route() {
    let reading = local_reading("I look behind the counter", &[]).unwrap();
    assert!(reading.intent.is_attempt());
    assert_eq!(reading.skill, Some("perception"));
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
    assert_eq!(
        local_target("I look behind the counter.", &options).id,
        expected
    );
    assert_eq!(local_target("I examine the mug.", &options).id, mug_id);
}

#[test]
fn jev_skill_choice_wins_over_listen_and_sleight_of_hand_keywords() {
    // Arrange: the keyword reader would pick perception, then sleight-of-hand, for these words.
    // Act and assert: Jev's chosen skill is the reading, so the keywords do not override it.
    assert_eq!(
        skill(
            "I listen at the door for the rustle of coins.",
            Some("insight")
        ),
        "insight"
    );
    assert_eq!(
        skill("I palm the coin while I listen.", Some("perception")),
        "perception"
    );
}

#[test]
fn keyword_skill_applies_only_when_jev_gave_no_skill() {
    // Jev unavailable: the keyword reader's skill is today's result for these words.
    assert_eq!(skill("I listen at the door.", None), "perception");
    assert_eq!(skill("I palm the coin.", None), "sleight-of-hand");
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
    )
    .unwrap();
    // "tankards" is a scene alias, so only Jev's absent answer keeps it out of the scene.
    let selected = resolve_target(Some(&provider), "I examine the tankards.", &options).await;
    assert!(!selected.present);
    assert!(selected.id.starts_with("absent-"));
}

#[tokio::test(flavor = "current_thread")]
async fn without_jev_the_keyword_reader_resolves_the_named_target() {
    let options = room();
    let selected = resolve_target(None, "I examine the tankards.", &options).await;
    assert_eq!(selected, local_target("I examine the tankards.", &options));
    assert_eq!(selected.id, "tankards");
}
