use super::*;
use crate::gameplay::{
    adventure::dossier::{NpcRole, Patience},
    npc_dossier_tests::{adventure, member, view},
    pack::SceneKey,
    social::PatienceState,
};
use std::time::Duration;

#[test]
fn merchant_shop_role_vocatives_address_without_generated_names() {
    assert!(room::addresses(
        "Merchant, what do you sell?",
        "Marten Vell"
    ));
    assert!(room::addresses("Soldier: is the road safe?", "Rusk"));
    assert!(room::addresses(
        "Adventurer, have you seen the road?",
        "Mara"
    ));
    assert!(!room::addresses("I see the soldier by the door.", "Rusk"));
}

fn room() -> (
    super::super::adventure::GeneratedAdventure,
    Arc<SocialLedger>,
) {
    let mut adventure = adventure(0x97);
    for scene in &mut adventure.pack.scenes {
        scene.setting = Some(adventure.settings.scene(scene.key).clone());
    }
    let ledger = SocialLedger::open(adventure.dossiers.clone(), "room-test", None).unwrap();
    ledger
        .install_rooms(&adventure.settings, &adventure.pack.scenes)
        .unwrap();
    (adventure, ledger)
}

fn context(
    adventure: &super::super::adventure::GeneratedAdventure,
    ledger: &Arc<SocialLedger>,
    words: &str,
) -> NpcReplyContext {
    NpcReplyContext::for_cast(
        &view(adventure, SceneKey::Opening, "Brakka"),
        member(1),
        words,
        &StoryFacts::from_pack(&adventure.pack),
        ledger,
        "Opening",
    )
    .unwrap()
}

#[test]
fn the_reply_knows_the_current_visible_room_without_other_peoples_secrets() {
    let (adventure, ledger) = room();
    let context = context(&adventure, &ledger, "Who is here with you?");
    let room = &context.facts["now"]["room"];
    let model = adventure.settings.meeting.model.as_ref().unwrap();
    assert_eq!(
        room["people"].as_array().unwrap().len(),
        model.for_scene(SceneKey::Opening).people.len()
    );
    assert!(
        room["people"]
            .as_array()
            .unwrap()
            .iter()
            .all(|person| person.get("knowledge").is_none())
    );
    assert!(!room.to_string().contains("closer_look"));
    assert!(!room.to_string().contains("clue_fact_id"));
    assert!(room["things"].as_array().is_some());
    assert!(room["ways_out"].as_array().is_some());
    assert!(room["your_knowledge"].as_array().is_some());
}

#[tokio::test]
async fn a_question_about_a_background_person_gets_a_room_answer_from_the_host() {
    let (adventure, ledger) = room();
    let model = adventure.settings.meeting.model.as_ref().unwrap();
    let person = model
        .people
        .iter()
        .find(|person| person.is_background)
        .unwrap();
    let context = context(&adventure, &ledger, &format!("Who is {}?", person.name));
    assert_eq!(
        context.speaker,
        adventure
            .dossiers
            .iter()
            .find(|d| d.role == NpcRole::Patron)
            .unwrap()
            .name
    );
    let answer = NpcVoice::silent()
        .converse_in_character(context, &TableLog::new())
        .await
        .unwrap();
    assert!(answer.text.contains(&person.name), "{}", answer.text);
    assert!(answer.text.contains(&person.role), "{}", answer.text);
}

#[test]
fn the_listener_has_their_own_visible_identity_in_reply_facts() {
    let (adventure, ledger) = room();
    let mut projected = view(&adventure, SceneKey::Opening, "Brakka");
    let Some(rpc::view_message::Audience::Player(player)) = projected.audience.as_mut() else {
        unreachable!()
    };
    player
        .journey
        .as_mut()
        .unwrap()
        .own_character
        .as_mut()
        .unwrap()
        .facts
        .push(rpc::SheetFact {
            label: "Gender".to_owned(),
            value: "man".to_owned(),
            ..Default::default()
        });
    let context = NpcReplyContext::for_cast(
        &projected,
        member(1),
        "Excuse me, miss, what is your name?",
        &StoryFacts::from_pack(&adventure.pack),
        &ledger,
        "Opening",
    )
    .unwrap();
    assert!(context.facts["hero_visible"].to_string().contains("man"));
}

#[test]
fn an_honest_unknown_answer_survives_an_earlier_unrelated_admission() {
    let rules = npc_dossier::spoken_rules(false, &[], true);
    for rule in rules {
        assert!(
            rule.violation("I don't know who the lady with the tray is.")
                .is_none()
        );
    }
}

#[tokio::test]
async fn repeated_questions_exhaust_patience_once_then_get_no_answer() {
    let (mut adventure, _) = room();
    let dossier = adventure
        .dossiers
        .iter_mut()
        .find(|d| d.role == NpcRole::Patron)
        .unwrap();
    let mut profile = dossier.emotional_profile();
    profile.patience = Patience::Low;
    dossier.emotional_profile = Some(profile);
    let ledger = SocialLedger::open(adventure.dossiers.clone(), "room-patience", None).unwrap();
    ledger
        .install_rooms(&adventure.settings, &adventure.pack.scenes)
        .unwrap();
    let voice = NpcVoice::silent();
    let log = TableLog::new();
    let mut refusal = 0;
    for _ in 0..5 {
        if let Some(answer) = voice
            .converse_in_character(context(&adventure, &ledger, "Where are we?"), &log)
            .await
        {
            refusal += usize::from(answer.text.contains("enough"));
        }
    }
    assert_eq!(refusal, 1);
    let dossier = adventure
        .dossiers
        .iter()
        .find(|d| d.role == NpcRole::Patron)
        .unwrap();
    assert_eq!(
        ledger
            .emotional_state(member(1), dossier, "Opening")
            .patience,
        PatienceState::Done
    );
    assert!(
        voice
            .converse_in_character(context(&adventure, &ledger, "Where are we?"), &log)
            .await
            .is_none()
    );
    assert!(!ledger.done_for(member(2), dossier, "Opening"));
    assert!(!ledger.done_for(member(1), dossier, "DialogueSeal"));
}

#[test]
fn fresh_dossiers_store_their_unified_profile() {
    let adventure = adventure(17);
    assert!(
        adventure
            .dossiers
            .iter()
            .all(|dossier| dossier.emotional_profile.is_some())
    );
}

#[test]
fn compact_records_hydrate_runtime_scene_models_without_changing_document_pins() {
    let mut adventure = adventure(0x97);
    // This fixture exercises document hydration, not admission of the newly declared
    // initiative/dead-space policies into Standard mode. Production keeps its strict gate.
    adventure.rules_mode = crate::gameplay::adventure::RulesMode::CustomAllowed;
    let documents = adventure.documents(true).unwrap();
    let original = documents.clone();
    let raw = super::super::pack::load_from_bytes(&documents.pack, &documents.tuning).unwrap();
    let (loaded, record) = super::super::adventure::record::reload(&documents).unwrap();
    assert_eq!(loaded.pack_revision, raw.pack_revision);
    assert_eq!(loaded.tuning_revision, raw.tuning_revision);
    assert_eq!(documents.pack, original.pack);
    assert_eq!(documents.record, original.record);
    let settings = record.settings.unwrap();
    for scene in loaded
        .pack
        .scenes
        .iter()
        .chain(loaded.pack.beat_script.as_ref().unwrap().scenes.values())
    {
        let setting = scene.setting.as_ref().unwrap();
        assert_eq!(
            setting.model,
            settings.scene(scene.key).model,
            "{:?}",
            scene.key
        );
    }
    let mut legacy = original.clone();
    let mut value: Value = serde_json::from_slice(&legacy.record).unwrap();
    for place in ["meeting", "confrontation", "destination"] {
        let model = value["settings"][place]["model"]
            .as_object_mut()
            .expect("legacy setting model object");
        model.remove("inventory_version");
        for thing in model["things"].as_array_mut().expect("things array") {
            thing
                .as_object_mut()
                .expect("thing object")
                .remove("provenance");
        }
        for offer in model["offers"].as_array_mut().expect("offers array") {
            offer
                .as_object_mut()
                .expect("offer object")
                .remove("item_id");
        }
    }
    legacy.record = serde_json::to_vec(&value).unwrap();
    let (legacy_loaded, legacy_record) =
        super::super::adventure::record::reload(&legacy).expect("legacy record migrates");
    let migrated = legacy_record
        .settings
        .as_ref()
        .unwrap()
        .meeting
        .model
        .as_ref()
        .unwrap();
    assert_eq!(migrated.inventory_version, 1);
    assert_eq!(
        migrated.things[0].provenance.source_id,
        migrated.things[0].id
    );
    assert!(
        migrated
            .offers
            .iter()
            .all(|offer| { offer.item_id == format!("item-{}", offer.id) })
    );
    assert_eq!(
        legacy_loaded.pack.scenes[0]
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .inventory_version,
        1
    );

    let mut invalid = original;
    let mut value: Value = serde_json::from_slice(&invalid.record).unwrap();
    value["settings"]["meeting"]["model"]["weather"] = json!("");
    invalid.record = serde_json::to_vec(&value).unwrap();
    assert!(super::super::adventure::record::reload(&invalid).is_err());
}

#[tokio::test]
async fn a_named_hosts_followup_knows_her_coworker_instead_of_repeating_the_errand() {
    let (adventure, ledger) = room();
    let voice = NpcVoice::silent();
    let log = TableLog::new();
    voice
        .converse_in_character(
            context(&adventure, &ledger, "Ilsa, who is the woman with the tray?"),
            &log,
        )
        .await
        .unwrap();
    let answer = voice
        .converse_in_character(
            context(&adventure, &ledger, "Ilsa, does she work here with you?"),
            &log,
        )
        .await
        .unwrap();
    assert!(answer.text.contains("works alongside"), "{}", answer.text);
}

#[tokio::test]
async fn directly_addressing_a_bystander_uses_their_own_name_and_knowledge() {
    let (adventure, ledger) = room();
    let person = adventure
        .settings
        .meeting
        .model
        .as_ref()
        .unwrap()
        .people
        .iter()
        .find(|person| person.is_background)
        .unwrap();
    let addressed = context(
        &adventure,
        &ledger,
        &format!("{}, what is your name?", person.name),
    );
    assert_eq!(addressed.speaker, person.name);
    let answer = NpcVoice::silent()
        .converse_in_character(addressed, &TableLog::new())
        .await
        .unwrap();
    assert!(answer.text.contains(&person.name), "{}", answer.text);
    assert!(!answer.text.contains("contact"));
}

#[test]
fn canonical_scene_snapshots_replace_derived_state_and_accept_hero_nodes() {
    let (_, ledger) = room();
    let mut changed = ledger.scene_now("Opening").unwrap();
    // Session-owned heroes are not generated NPCs: no knowledge or aliases is invented.
    changed.model.people[0].id = "Brakka".to_owned();
    changed.model.people[0].name = "Brakka".to_owned();
    changed.model.people[0].knowledge.clear();
    changed.model.people[0].aliases.clear();
    changed.model.things[0].holder_id = Some("Brakka".to_owned());
    changed.changes.push("Brakka takes the tankard.".to_owned());
    ledger.sync_room("Opening", changed.clone()).unwrap();
    ledger.sync_room("Opening", changed.clone()).unwrap();
    assert_eq!(ledger.scene_now("Opening").unwrap(), changed);
    let accepted = changed.clone();
    changed.changes.push("x".repeat(181));
    assert!(ledger.sync_room("Opening", changed).is_err());
    assert_eq!(ledger.scene_now("Opening").unwrap(), accepted);
}

#[test]
fn perceived_patience_is_player_only() {
    use crate::gameplay::social::{Appraisal, EmotionLevel, EmotionReaction};
    let (adventure, ledger) = room();
    let dossier = &adventure.dossiers[0];
    ledger.apply_appraisal(
        member(1),
        dossier,
        "Opening",
        Appraisal {
            reaction: EmotionReaction::LosingPatience,
            strength: EmotionLevel::ALittle,
            willingness: None,
            effect: super::super::adventure::dossier::Effect::Steady,
        },
    );
    let mut own = view(&adventure, SceneKey::Opening, "Brakka");
    ledger.attach(&mut own, member(1));
    let Some(rpc::view_message::Audience::Player(player)) = own.audience else {
        unreachable!()
    };
    assert!(
        player.journey.unwrap().standing[0]
            .perceived_reaction
            .contains("losing patience")
    );
    let mut another = view(&adventure, SceneKey::Opening, "Sigrun");
    ledger.attach(&mut another, member(2));
    let Some(rpc::view_message::Audience::Player(player)) = another.audience else {
        unreachable!()
    };
    assert!(
        !player.journey.unwrap().standing[0]
            .perceived_reaction
            .contains("losing patience")
    );
    let mut display = rpc::ViewMessage {
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView::default(),
        )),
        ..Default::default()
    };
    let before = display.clone();
    ledger.attach(&mut display, member(1));
    assert_eq!(display, before);
}

#[tokio::test]
async fn a_fake_model_can_admit_ignorance_without_a_stock_line_replacing_it() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller};
    let (adventure, ledger) = room();
    let words = "Who is that masked pilgrim?";
    let endpoint = FakeEndpoint::start(vec![Reply::Output(
        json!({"reply": "I don't know the masked pilgrim.", "approach": "question"}),
    )]);
    let cache = cache_directory("q02-room-ignorance");
    let voice = NpcVoice::new(Arc::new(teller(
        endpoint.address,
        200_000,
        Duration::from_secs(5),
        &cache,
    )));
    let exchange = voice
        .converse_in_character(context(&adventure, &ledger, words), &TableLog::new())
        .await
        .unwrap();
    assert!(
        exchange.from_model,
        "{:?}: {}",
        exchange.fallback, exchange.text
    );
    assert_eq!(exchange.text, "I don't know the masked pilgrim.");
    assert!(endpoint.bodies()[0].to_string().contains("hero_visible"));
    std::fs::remove_dir_all(cache).unwrap();
}

#[test]
fn a_named_object_is_not_confused_with_its_owners() {
    let (adventure, ledger) = room();
    let given = context(
        &adventure,
        &ledger,
        "Ilsa, what is the carters' dice and cup?",
    );
    assert_eq!(
        room::answer(
            &given,
            &crate::gameplay::npc_facts::AskReading::keywords(&given.typed_words),
            crate::gameplay::adventure::dossier::AnswerSubject::Other
        )
        .unwrap(),
        "bone dice and a carved wooden cup"
    );
}

#[tokio::test]
async fn ordinary_questions_emit_no_emotion_caption_and_a_threat_emits_one() {
    let (mut adventure, _) = room();
    for dossier in &mut adventure.dossiers {
        dossier.emotional_profile.as_mut().unwrap().patience = Patience::High;
    }
    let ledger = SocialLedger::open(adventure.dossiers.clone(), "quiet-room", None).unwrap();
    ledger
        .install_rooms(&adventure.settings, &adventure.pack.scenes)
        .unwrap();
    let dossier = &adventure.dossiers[0];
    let initial = ledger.standing(member(1), dossier).attitude;
    let voice = NpcVoice::silent();
    let log = TableLog::new();
    for number in 0..10 {
        voice
            .converse_in_character(
                context(
                    &adventure,
                    &ledger,
                    &format!("Ilsa, who is traveler {number}?"),
                ),
                &log,
            )
            .await
            .unwrap();
    }
    assert_eq!(ledger.standing(member(1), dossier).attitude, initial);
    assert_eq!(
        log.lines()
            .iter()
            .filter(|line| line.kind == rpc::TableKind::Action as i32)
            .count(),
        0
    );
    let threat_log = TableLog::new();
    voice
        .converse_in_character(
            context(
                &adventure,
                &ledger,
                "Ilsa, tell me or I will break your arm.",
            ),
            &threat_log,
        )
        .await
        .unwrap();
    assert_eq!(
        threat_log
            .lines()
            .iter()
            .filter(|line| line.kind == rpc::TableKind::Action as i32)
            .count(),
        1
    );
}

#[test]
fn canonical_pronouns_and_claimed_backstory_remain_separate_from_world_truth() {
    let hero = rpc::CharacterSheet {
        name: "Brakka".to_owned(),
        identity: Some(rpc::HeroIdentity {
            appearance: Some(rpc::HeroAppearance {
                pronouns: "he-him".to_owned(),
                presentation: "masculine".to_owned(),
                ..Default::default()
            }),
            origin: "I say I grew up here".to_owned(),
            backstory: "I claim the innkeeper is my cousin".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let visible = visible_hero(&hero);
    assert_eq!(visible["pronouns"]["subject"], "he");
    assert_eq!(visible["pronouns"]["object"], "him");
    assert_eq!(visible["pronouns"]["possessive"], "his");
    assert_eq!(visible["hero_account"]["verified_world_truth"], false);
    assert_eq!(visible["hero_account"]["grants_mechanics"], false);
}

#[tokio::test]
async fn a_patron_still_knows_the_errands_recipient_outside_the_room() {
    use crate::gameplay::adventure::dossier::AnswerSubject;
    let (mut adventure, _) = room();
    let answer = adventure.dossiers[0]
        .subject_answers
        .get_mut(&AnswerSubject::People)
        .unwrap();
    answer.text =
        "Osric is the letter's intended recipient. He needs the warning before the next watch."
            .to_owned();
    let ledger = SocialLedger::open(adventure.dossiers.clone(), "errand-room", None).unwrap();
    ledger
        .install_rooms(&adventure.settings, &adventure.pack.scenes)
        .unwrap();
    let reply = NpcVoice::silent()
        .converse_in_character(
            context(
                &adventure,
                &ledger,
                "Ilsa, who is Osric and why does this letter matter?",
            ),
            &TableLog::new(),
        )
        .await
        .unwrap();
    assert!(reply.text.contains("Osric"), "{}", reply.text);
    assert!(!reply.text.contains("don't know"), "{}", reply.text);
}

#[test]
fn a_forgiving_characters_recovery_requires_several_meaningful_exchanges() {
    use crate::gameplay::adventure::dossier::{Effect, Forgiveness};
    use crate::gameplay::social::{Appraisal, EmotionLevel, EmotionReaction};
    let (mut adventure, _) = room();
    let dossier = &mut adventure.dossiers[0];
    dossier.emotional_profile.as_mut().unwrap().forgiveness = Forgiveness::Forgiving;
    let ledger = SocialLedger::open(adventure.dossiers.clone(), "recovery-room", None).unwrap();
    let dossier = &adventure.dossiers[0];
    ledger.apply_appraisal(
        member(1),
        dossier,
        "Opening",
        Appraisal {
            reaction: EmotionReaction::Offended,
            strength: EmotionLevel::Much,
            willingness: None,
            effect: Effect::Cool,
        },
    );
    for _ in 0..2 {
        ledger.apply_appraisal(
            member(1),
            dossier,
            "Opening",
            Appraisal {
                reaction: EmotionReaction::Warmed,
                strength: EmotionLevel::ALittle,
                willingness: None,
                effect: Effect::Warm,
            },
        );
        assert_eq!(
            ledger
                .emotional_state(member(1), dossier, "Opening")
                .resentful,
            EmotionLevel::Much
        );
    }
    ledger.apply_appraisal(
        member(1),
        dossier,
        "Opening",
        Appraisal {
            reaction: EmotionReaction::Warmed,
            strength: EmotionLevel::ALittle,
            willingness: None,
            effect: Effect::Warm,
        },
    );
    assert_eq!(
        ledger
            .emotional_state(member(1), dossier, "Opening")
            .resentful,
        EmotionLevel::ALittle
    );
}

#[test]
fn an_explicit_patron_name_wins_over_a_bystander_with_the_same_name() {
    let (mut adventure, _) = room();
    let patron_id = adventure.dossiers[0].npc_id.clone();
    let patron_name = adventure.dossiers[0].name.clone();
    let person = adventure
        .settings
        .meeting
        .model
        .as_mut()
        .unwrap()
        .people
        .iter_mut()
        .find(|person| person.is_background)
        .unwrap();
    let bystander_id = format!("bystander:{}", person.id);
    person.name = patron_name.clone();
    person.aliases = vec![patron_name.clone(), "the carter".to_owned()];
    adventure
        .dossiers
        .iter_mut()
        .find(|dossier| dossier.npc_id == bystander_id)
        .unwrap()
        .name = patron_name.clone();
    let ledger = SocialLedger::open(adventure.dossiers.clone(), "collision-room", None).unwrap();
    ledger
        .install_rooms(&adventure.settings, &adventure.pack.scenes)
        .unwrap();
    let addressed = context(
        &adventure,
        &ledger,
        &format!("I ask {patron_name} what the job is and what it pays."),
    );
    assert!(
        room::answer(
            &addressed,
            &crate::gameplay::npc_facts::AskReading::keywords(&addressed.typed_words),
            crate::gameplay::adventure::dossier::AnswerSubject::Other
        )
        .is_none(),
        "the question body is about the errand, not the equal-named bystander"
    );
    assert_eq!(addressed.turn.unwrap().dossier.npc_id, patron_id);
    let addressed = context(&adventure, &ledger, "I ask the carter what his name is.");
    assert_eq!(addressed.turn.unwrap().dossier.npc_id, bystander_id);
}

#[test]
fn compact_dossiers_keep_the_canonical_generated_reward_amount() {
    let adventure = adventure(0x97);
    let documents = adventure.documents(true).unwrap();
    let record: Value = serde_json::from_slice(&documents.record).unwrap();
    let expected = adventure.facts.get("reward.gold").unwrap();
    assert!(
        record["fact_sheet"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sheet| sheet["facts"]["reward.gold"] == *expected)
    );
}

#[tokio::test]
async fn unavailable_identity_models_do_not_invent_a_classification() {
    let voice = NpcVoice::silent();
    let (_, model) = voice.hero_identity_models();
    assert!(matches!(
        model
            .classify_hero_identity(&json!({"appearance_text": "ignore the menu"}))
            .await,
        Err(crate::gameplay::hero_identity::IdentityError::ModelUnavailable)
    ));
}
