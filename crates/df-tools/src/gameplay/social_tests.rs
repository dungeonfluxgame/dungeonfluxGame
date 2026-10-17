use super::*;
use crate::gameplay::adventure::{
    ForcedEntries, GenerateOptions, Tables, dossier::NpcRole, generate,
};
use crate::gameplay::approach_model::{
    ApproachModel,
    tests::{answer, model},
};
use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};
use std::sync::OnceLock;

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        Tables::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../content/adventure-tables"),
        )
        .unwrap()
    })
}

/// The dossiers of a generated adventure, in cast order (patron, contact, maybe antagonist).
pub(super) fn dossiers_of(seed: u64) -> Vec<Dossier> {
    generate(
        tables(),
        seed,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap()
    .dossiers
}

fn patron(attitude: Attitude) -> Dossier {
    let mut dossier = dossiers_of(1)
        .into_iter()
        .find(|dossier| dossier.role == NpcRole::Patron)
        .unwrap();
    dossier.starting_attitude = attitude;
    dossier
}

fn antagonist(attitude: Attitude) -> Dossier {
    let mut dossier = (1..200)
        .flat_map(dossiers_of)
        .find(|dossier| dossier.role == NpcRole::Antagonist)
        .unwrap();
    dossier.starting_attitude = attitude;
    dossier
}

fn played_evidence_dossier() -> Dossier {
    let trait_fact = serde_json::json!({"id": "test", "text": "test"});
    let reaction = serde_json::json!({
        "id": "test",
        "text": "test",
        "answer": "test",
        "tags": [],
        "effect": "steady"
    });
    serde_json::from_value(serde_json::json!({
        "npc_id": "patron:ulric",
        "name": "Ulric",
        "role": "patron",
        "role_label": "retired soldier",
        "tables_revision": "test",
        "given": {
            "mannerism": null,
            "delivery": null,
            "voice": null,
            "tags": [],
            "note": null
        },
        "origin": trait_fact,
        "past_trade": trait_fact,
        "formative_event": trait_fact,
        "ideal": trait_fact,
        "bond": trait_fact,
        "flaw": trait_fact,
        "want": trait_fact,
        "fear": trait_fact,
        "talent": trait_fact,
        "story_tie": {
            "id": "test",
            "binds": "stake",
            "bound_fact": "the warning",
            "bound_cast": null,
            "public": true,
            "text": "test",
            "speak": "test"
        },
        "motive": null,
        "stand_down": null,
        "knowledge": {
            "says_freely": [],
            "says_when_trusted": [],
            "never_says": []
        },
        "emotional_profile": null,
        "temperament": {
            "flattery": reaction,
            "flirting": reaction,
            "threats": reaction,
            "lies": reaction,
            "bribes": reaction
        },
        "starting_attitude": "indifferent",
        "speech_style": {
            "id": "plain",
            "length": "medium",
            "vocabulary": "plain",
            "tic": "",
            "opener": "",
            "closer": "",
            "tags": []
        },
        "prepared_answers": {},
        "subject_answers": {
            "errand": {"text": "warn Osric tonight", "restatement": "That is the errand."},
            "people": {"text": "Osric is the watch sergeant at the Ferryman's Hut.", "restatement": "Osric is the watch sergeant."},
            "stakes": {"text": "The map bundle protects lives.", "restatement": "Lives depend on it."},
            "deadline": {"text": "Warn Osric tonight.", "restatement": "Tonight."},
            "where": {"text": "I'm at the Reedmoor Boardwalk; Osric waits at the Ferryman's Hut.", "restatement": "Osric waits at the Ferryman's Hut."},
            "reward": {"text": "I'll pay 14 GP for carrying it.", "restatement": "14 GP."},
            "enemy": {"text": "The Barrow-Cold Sentinel is known as skeletons. Whoever hired those chasing us wears a ring with a green stone.", "restatement": "The Sentinel is skeletons."},
            "errand_object": {"text": "map bundle is a bundle of surveyors' maps.", "restatement": "It is surveyors' maps."}
        },
        "curt_answers": [],
        "rolls": []
    }))
    .unwrap()
}

fn member(byte: u8) -> MemberId {
    MemberId::from_bytes(&[byte; 16]).unwrap()
}

fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("df-social-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// Warm or cool one step at a time inside one scene.
fn steps(
    relationship: &mut Relationship,
    scene: &str,
    effect: Effect,
    count: usize,
    opposing: bool,
) {
    for _ in 0..count {
        relationship.apply(scene, effect, opposing);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn without_jev_every_line_reads_neutral_and_its_words_decide_nothing() {
    let lines = [
        "Get out of here or else",
        "Hand over the key or else.",
        "Here is ten gold for your trouble.",
        "Thank you, we will help you.",
    ];
    let failing_endpoint = FakeEndpoint::start(
        lines
            .iter()
            .map(|_| Reply::Status(500, serde_json::json!({"error": "unavailable"})))
            .collect(),
    );
    let failing = model(&failing_endpoint);
    let off = ApproachModel::default();

    for line in lines {
        for reader in [&off, &failing] {
            let reading = reader
                .classify(line, "Perrin", "A public tavern.", "")
                .await
                .unwrap_or_else(neutral_reading);

            assert_eq!(reading.approach, Approach::Other, "{line}");
            assert_ne!(reading.approach, Approach::Intimidation, "{line}");
            assert_eq!(reading.subject, AnswerSubject::Other, "{line}");
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn jev_decides_the_approach_and_subject_of_a_line_whatever_its_words() {
    // "oN" is Jev's N-th criterion: the approaches in their option order, then the subjects.
    for (words, approach_option, subject_option, approach, subject) in [
        (
            "It would be a shame if this place burned.",
            "o7",
            "o9",
            Approach::Intimidation,
            AnswerSubject::Other,
        ),
        (
            "What will you pay us for carrying it?",
            "o9",
            "o6",
            Approach::Bargain,
            AnswerSubject::Reward,
        ),
        (
            "Who is the danger? Who wants the bundle?",
            "o1",
            "o7",
            Approach::Question,
            AnswerSubject::Enemy,
        ),
        (
            "I am sorry. I should not have threatened you.",
            "o11",
            "o9",
            Approach::Apology,
            AnswerSubject::Other,
        ),
        (
            "I am Sigrun. We will carry your warning.",
            "o12",
            "o1",
            Approach::Introduction,
            AnswerSubject::Errand,
        ),
        (
            "No. We will not carry it.",
            "o13",
            "o1",
            Approach::Refusal,
            AnswerSubject::Errand,
        ),
        ("Hmm.", "o14", "o9", Approach::Other, AnswerSubject::Other),
    ] {
        let endpoint = FakeEndpoint::start(vec![answer(approach_option), answer(subject_option)]);

        let reading = model(&endpoint)
            .classify(words, "Perrin", "A public tavern.", "")
            .await
            .unwrap_or_else(neutral_reading);

        assert_eq!(reading, ApproachReading { approach, subject }, "{words}");
        assert_eq!(endpoint.bodies()[0]["state"]["player_words"], words);
    }
}

#[test]
fn factual_questions_and_refusals_are_not_influence_attempts() {
    for approach in [
        Approach::Question,
        Approach::Introduction,
        Approach::Refusal,
        Approach::Insult,
        Approach::Other,
    ] {
        assert!(!approach.is_influence_attempt(), "{}", approach.label());
    }
    for approach in [
        Approach::Kindness,
        Approach::Apology,
        Approach::Flattery,
        Approach::Flirting,
        Approach::Persuasion,
        Approach::Deception,
        Approach::Intimidation,
        Approach::Bribe,
        Approach::Bargain,
    ] {
        assert!(approach.is_influence_attempt(), "{}", approach.label());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn jev_picks_a_subject_only_among_the_facts_in_this_adventure() {
    let mut dossier = crate::gameplay::npc_dossier_tests::adventure(1)
        .dossiers
        .into_iter()
        .next()
        .unwrap();
    dossier.subject_answers = BTreeMap::from([
        (
            AnswerSubject::People,
            super::super::adventure::dossier::SubjectAnswer {
                text: "I'm Ilsa, the innkeeper. Tobin, the mapmaker, asked me to help.".into(),
                restatement: "Tobin is a mapmaker.".into(),
            },
        ),
        (
            AnswerSubject::Enemy,
            super::super::adventure::dossier::SubjectAnswer {
                text: "The Barrow-Cold Sentinel is known as a giant rat.".into(),
                restatement: "The Sentinel is a giant rat.".into(),
            },
        ),
        (
            AnswerSubject::ErrandObject,
            super::super::adventure::dossier::SubjectAnswer {
                text: "map bundle is a bundle of surveyors' maps.".into(),
                restatement: "It is a bundle of surveyors' maps.".into(),
            },
        ),
    ]);

    // The subjects offered are the dossier's own, in Jev's subject order, then "other".
    for (line, subject_option, expected) in [
        ("What sort of man is Tobin?", "o1", AnswerSubject::People),
        (
            "What is the Barrow-Cold Sentinel?",
            "o2",
            AnswerSubject::Enemy,
        ),
        (
            "What is in the map bundle?",
            "o3",
            AnswerSubject::ErrandObject,
        ),
        (
            "What do you know about the goblin?",
            "o4",
            AnswerSubject::Other,
        ),
    ] {
        let endpoint = FakeEndpoint::start(vec![answer("o1"), answer(subject_option)]);

        let reading = model(&endpoint)
            .classify_for_dossier(line, &dossier.name, "The ferry desk.", &dossier, "")
            .await
            .unwrap_or_else(neutral_reading);

        assert_eq!(reading.subject, expected, "{line}");
        let bodies = endpoint.bodies();
        let offered = bodies[1]["questions"]["q1"]["criteria"]
            .as_object()
            .unwrap();
        assert_eq!(offered.len(), 4, "{line}");
        assert!(!offered.values().any(|criterion| {
            criterion
                .as_str()
                .is_some_and(|text| text.contains("price"))
        }));
    }

    let without_jev = ApproachModel::default()
        .classify_for_dossier(
            "What is in the map bundle?",
            &dossier.name,
            "The ferry desk.",
            &dossier,
            "",
        )
        .await
        .unwrap_or_else(neutral_reading);
    assert_eq!(without_jev, neutral_reading());
}

#[tokio::test(flavor = "current_thread")]
async fn jev_readings_of_the_played_evidence_set_approach_subject_and_roll_eligibility() {
    let dossier = played_evidence_dossier();
    // Every subject is offered for this dossier, so "oN" is the N-th of Jev's subjects.
    let evidence = [
        (
            "Who is Osric, and where exactly do I find him?",
            "o1",
            "o5",
            Approach::Question,
            AnswerSubject::Where,
            false,
        ),
        (
            "What is in the map bundle, and whose lives does it protect?",
            "o1",
            "o8",
            Approach::Question,
            AnswerSubject::ErrandObject,
            false,
        ),
        (
            "What do we get for carrying it?",
            "o9",
            "o6",
            Approach::Bargain,
            AnswerSubject::Reward,
            true,
        ),
        (
            "You were a soldier, Ulric. So was I. Give us everything you know, one soldier to another.",
            "o5",
            "o7",
            Approach::Persuasion,
            AnswerSubject::Enemy,
            true,
        ),
        (
            "The watch captain already told us the whole plan. He said you would hand over your shield as proof.",
            "o6",
            "o9",
            Approach::Deception,
            AnswerSubject::Other,
            true,
        ),
        (
            "Talk, old man, or I will break that shield over my knee.",
            "o7",
            "o9",
            Approach::Intimidation,
            AnswerSubject::Other,
            true,
        ),
        (
            "I spoke badly before, Ulric. I am sorry. I should not have threatened you.",
            "o11",
            "o9",
            Approach::Apology,
            AnswerSubject::Other,
            true,
        ),
        (
            "No. We will not carry it. Find someone else.",
            "o13",
            "o1",
            Approach::Refusal,
            AnswerSubject::Errand,
            false,
        ),
        (
            "I am Sigrun. Ignore him. We will carry your warning.",
            "o12",
            "o1",
            Approach::Introduction,
            AnswerSubject::Errand,
            false,
        ),
        (
            "I search the desk and any papers on it.",
            "o14",
            "o9",
            Approach::Other,
            AnswerSubject::Other,
            false,
        ),
    ];

    for (line, approach_option, subject_option, approach, subject, hesitant_roll) in evidence {
        let endpoint = FakeEndpoint::start(vec![answer(approach_option), answer(subject_option)]);

        let reading = model(&endpoint)
            .classify_for_dossier(line, "Ulric", "The guardhouse.", &dossier, "")
            .await
            .unwrap_or_else(neutral_reading);

        assert_eq!(reading.approach, approach, "approach: {line}");
        assert_eq!(reading.subject, subject, "subject: {line}");
        assert_eq!(
            reading.approach.is_influence_attempt(),
            hesitant_roll,
            "a hesitant listener's roll eligibility: {line}"
        );
    }
}

#[test]
fn a_patron_who_offers_the_errand_never_starts_hostile() {
    for seed in 1..=128 {
        let patron = crate::gameplay::npc_dossier_tests::adventure(seed)
            .dossiers
            .into_iter()
            .find(|dossier| dossier.role == super::super::adventure::dossier::NpcRole::Patron)
            .unwrap();
        assert_ne!(patron.starting_attitude, Attitude::Hostile, "seed {seed}");
    }
}

#[test]
fn the_models_one_word_approach_is_parsed_and_anything_else_is_not_an_approach() {
    assert_eq!(Approach::parse(" Flirting. "), Some(Approach::Flirting));
    assert_eq!(Approach::parse("deception"), Some(Approach::Deception));
    assert_eq!(Approach::parse("seduction"), None);
    assert_eq!(Approach::parse(""), None);
}

#[test]
fn a_warm_exchange_moves_trust_by_one_and_two_move_the_attitude_one_level() {
    let dossier = patron(Attitude::Indifferent);
    let mut relationship = Relationship::new(&dossier);

    let first = relationship.apply("opening", Effect::Warm, false);
    assert_eq!(first, Some(Shift::Trust { warmer: true }));
    assert_eq!(relationship.attitude, Attitude::Indifferent);
    let second = relationship.apply("opening", Effect::Warm, false);

    assert_eq!(
        second,
        Some(Shift::Level {
            from: Attitude::Indifferent,
            to: Attitude::Friendly
        })
    );
    assert_eq!(relationship.trust, 0);
    assert_eq!(relationship.apply("opening", Effect::Steady, false), None);
}

#[test]
fn an_exchange_never_moves_the_attitude_more_than_one_level() {
    let dossier = patron(Attitude::Hostile);
    let mut relationship = Relationship::new(&dossier);
    for effect in [
        Effect::Warm,
        Effect::Cool,
        Effect::Warm,
        Effect::Warm,
        Effect::Cool,
        Effect::Cool,
    ] {
        let before = rank(relationship.attitude);
        relationship.apply("opening", effect, false);
        assert!((rank(relationship.attitude) - before).abs() <= 1);
        assert!(relationship.trust.abs() < TRUST_STEP);
    }
}

#[test]
fn attitude_moves_at_most_one_level_per_scene_and_more_in_the_next_scene() {
    let dossier = patron(Attitude::Hostile);
    let mut relationship = Relationship::new(&dossier);

    steps(&mut relationship, "opening", Effect::Warm, 8, false);
    assert_eq!(
        relationship.attitude,
        Attitude::Indifferent,
        "one level in a scene"
    );
    assert_eq!(
        relationship.trust,
        TRUST_STEP - 1,
        "held one point short of the next step"
    );
    steps(&mut relationship, "dialogue", Effect::Warm, 1, false);

    assert_eq!(
        relationship.attitude,
        Attitude::Friendly,
        "the next scene allows another level"
    );
    steps(&mut relationship, "combat", Effect::Cool, 8, false);
    assert_eq!(
        relationship.attitude,
        Attitude::Indifferent,
        "and one level back down"
    );
}

#[test]
fn an_opposing_npc_never_becomes_friendly_from_talk_alone_but_may_start_to_listen() {
    let dossier = antagonist(Attitude::Hostile);
    assert!(dossier.role.opposes_party());
    let mut relationship = Relationship::new(&dossier);

    for scene in ["opening", "dialogue", "combat", "inn"] {
        steps(&mut relationship, scene, Effect::Warm, 6, true);
    }

    assert_eq!(relationship.attitude, Attitude::Indifferent);
    let standing = Standing {
        attitude: relationship.attitude,
        trust: relationship.trust,
    };
    assert!(
        standing.trusted(&dossier),
        "an Indifferent opponent with trust hears you out"
    );
    assert!(
        !Standing {
            attitude: Attitude::Indifferent,
            trust: 0
        }
        .trusted(&dossier)
    );
    assert!(
        !Standing {
            attitude: Attitude::Hostile,
            trust: 1
        }
        .trusted(&dossier)
    );
    assert!(
        Standing {
            attitude: Attitude::Friendly,
            trust: 0
        }
        .trusted(&patron(Attitude::Friendly))
    );
    assert!(
        !Standing {
            attitude: Attitude::Indifferent,
            trust: 1
        }
        .trusted(&patron(Attitude::Friendly))
    );
}

#[test]
fn a_legacy_dossiers_temperament_decides_whether_an_approach_warms_cools_or_changes_nothing() {
    let mut dossier = patron(Attitude::Indifferent);
    dossier.emotional_profile = None;
    dossier.temperament.flirting.effect = Effect::Cool;
    dossier.temperament.flattery.effect = Effect::Warm;
    dossier.temperament.bribes.effect = Effect::Steady;

    assert_eq!(effect_of(&dossier, Approach::Flirting), Effect::Cool);
    assert_eq!(effect_of(&dossier, Approach::Flattery), Effect::Warm);
    assert_eq!(effect_of(&dossier, Approach::Bribe), Effect::Steady);
    assert_eq!(effect_of(&dossier, Approach::Kindness), Effect::Warm);
    assert_eq!(effect_of(&dossier, Approach::Insult), Effect::Cool);
    assert_eq!(effect_of(&dossier, Approach::Question), Effect::Steady);
}

#[test]
fn a_threat_lowers_attitude_immediately_even_when_the_dossier_is_steady() {
    let mut dossier = patron(Attitude::Friendly);
    dossier.temperament.threats.effect = Effect::Steady;
    let directory = directory("threat");
    let ledger =
        SocialLedger::open(vec![dossier.clone()], "threat", Some(directory.clone())).unwrap();
    let hero = member(12);

    assert_eq!(
        ledger.threaten(hero, &dossier, "opening"),
        Some(Shift::Level {
            from: Attitude::Friendly,
            to: Attitude::Indifferent
        })
    );
    assert_eq!(
        ledger.standing(hero, &dossier).attitude,
        Attitude::Indifferent
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Intimidation,
        "Hand over the key or else.",
        "I will not.",
    );
    assert_eq!(
        ledger.standing(hero, &dossier).attitude,
        Attitude::Indifferent,
        "recording the reply does not apply the threat twice"
    );
    ledger.save().unwrap();
    let ledger =
        SocialLedger::open(vec![dossier.clone()], "threat", Some(directory.clone())).unwrap();
    assert_eq!(
        ledger.standing(hero, &dossier).attitude,
        Attitude::Indifferent,
        "the lowered attitude survives reload"
    );
    assert_eq!(
        ledger.threaten(hero, &dossier, "opening"),
        Some(Shift::Level {
            from: Attitude::Indifferent,
            to: Attitude::Hostile
        })
    );
    assert_eq!(ledger.standing(hero, &dossier).attitude, Attitude::Hostile);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn attitude_and_the_recent_conversation_survive_a_reload() {
    let dossier = patron(Attitude::Indifferent);
    let directory = directory("reload");
    let ledger =
        SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone())).unwrap();
    let hero = member(7);
    for (approach, line) in [
        (Approach::Kindness, "Thank you, we will help you."),
        (Approach::Apology, "I am sorry for your trouble."),
    ] {
        ledger.record_exchange(hero, &dossier, "opening", approach, line, "Hm.");
    }
    ledger.save().unwrap();
    let before = ledger.standing(hero, &dossier);
    assert_eq!(before.attitude, Attitude::Friendly);

    let reloaded =
        SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone())).unwrap();

    assert_eq!(reloaded.standing(hero, &dossier), before);
    assert_eq!(
        reloaded.recent(hero, &dossier.npc_id),
        ledger.recent(hero, &dossier.npc_id)
    );
    assert_eq!(
        reloaded.standing(member(8), &dossier).attitude,
        Attitude::Indifferent,
        "another hero starts where the dossier says"
    );
    assert!(
        !directory.join("social.json.tmp").exists(),
        "the staging file is renamed away"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn state_saved_for_another_adventure_or_out_of_bounds_is_refused_not_ignored() {
    let dossier = patron(Attitude::Indifferent);
    let directory = directory("refused");
    let ledger =
        SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone())).unwrap();
    ledger.record_exchange(
        member(1),
        &dossier,
        "opening",
        Approach::Kindness,
        "Thanks.",
        "Hm.",
    );
    ledger.save().unwrap();

    let other = SocialLedger::open(vec![dossier.clone()], "0x02", Some(directory.clone()));
    assert!(matches!(other, Err(SocialError::Invalid(_))));

    let file = directory.join("social.json");
    let tampered = fs::read_to_string(&file)
        .unwrap()
        .replace("\"trust\": 1", "\"trust\": 9");
    fs::write(&file, tampered).unwrap();
    let bounded = SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone()));
    assert!(matches!(bounded, Err(SocialError::Invalid(_))));
    fs::write(&file, b"not json").unwrap();
    assert!(SocialLedger::open(vec![dossier], "0x01", Some(directory.clone())).is_err());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn each_jev_reaction_maps_to_its_named_relationship_state() {
    for (reaction, expected) in [
        (
            EmotionReaction::Warmed,
            (
                EmotionLevel::Much,
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                PatienceState::Plenty,
            ),
        ),
        (
            EmotionReaction::Charmed,
            (
                EmotionLevel::Much,
                EmotionLevel::None,
                EmotionLevel::Much,
                EmotionLevel::None,
                PatienceState::Plenty,
            ),
        ),
        (
            EmotionReaction::Amused,
            (
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                PatienceState::Plenty,
            ),
        ),
        (
            EmotionReaction::Unmoved,
            (
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                PatienceState::Plenty,
            ),
        ),
        (
            EmotionReaction::Uneasy,
            (
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                PatienceState::Plenty,
            ),
        ),
        (
            EmotionReaction::Frightened,
            (
                EmotionLevel::None,
                EmotionLevel::Much,
                EmotionLevel::None,
                EmotionLevel::None,
                PatienceState::Plenty,
            ),
        ),
        (
            EmotionReaction::Offended,
            (
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::Much,
                PatienceState::Plenty,
            ),
        ),
        (
            EmotionReaction::Disgusted,
            (
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::Much,
                PatienceState::Plenty,
            ),
        ),
        (
            EmotionReaction::LosingPatience,
            (
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                PatienceState::Short,
            ),
        ),
        (
            EmotionReaction::Done,
            (
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                EmotionLevel::None,
                PatienceState::Done,
            ),
        ),
    ] {
        let mut state = EmotionalState::default();
        let mapped = state.apply(
            Appraisal {
                reaction,
                strength: EmotionLevel::Much,
                willingness: None,
                effect: Effect::Steady,
            },
            Forgiveness::Forgiving,
            "the offense",
        );
        assert_eq!(mapped, reaction);
        assert_eq!(
            (
                state.fond,
                state.afraid,
                state.drawn_to,
                state.resentful,
                state.patience
            ),
            expected,
            "{reaction:?}"
        );
        if reaction == EmotionReaction::Uneasy {
            assert_eq!(state.wary, EmotionLevel::Much);
        }
    }

    let mut forgiving = EmotionalState::default();
    assert_eq!(
        forgiving.apply(
            Appraisal {
                reaction: EmotionReaction::NeverForgive,
                strength: EmotionLevel::Much,
                willingness: None,
                effect: Effect::Cool,
            },
            Forgiveness::Forgiving,
            "the offense",
        ),
        EmotionReaction::Offended
    );
    assert!(forgiving.never_forgives.is_none());
    let mut unforgiving = EmotionalState::default();
    assert_eq!(
        unforgiving.apply(
            Appraisal {
                reaction: EmotionReaction::NeverForgive,
                strength: EmotionLevel::Much,
                willingness: None,
                effect: Effect::Cool,
            },
            Forgiveness::Unforgiving,
            "the offense",
        ),
        EmotionReaction::NeverForgive
    );
    assert_eq!(unforgiving.never_forgives.as_deref(), Some("the offense"));
}

#[test]
fn patience_done_is_said_once_and_a_never_forgive_grudge_survives_reload() {
    let directory = directory("emotional-state");
    let mut dossier = patron(Attitude::Indifferent);
    let mut profile = dossier.emotional_profile();
    profile.forgiveness = Forgiveness::Unforgiving;
    dossier.emotional_profile = Some(profile);
    let ledger = SocialLedger::open(
        vec![dossier.clone()],
        "emotional-state",
        Some(directory.clone()),
    )
    .unwrap();
    let hero = member(16);

    assert_eq!(
        ledger.note_subject(hero, &dossier, "opening", AnswerSubject::Errand),
        0
    );
    assert_eq!(
        ledger.note_subject(hero, &dossier, "opening", AnswerSubject::Errand),
        1
    );
    assert_eq!(
        ledger.note_subject(hero, &dossier, "opening", AnswerSubject::Errand),
        2
    );
    ledger.apply_appraisal(
        hero,
        &dossier,
        "opening",
        Appraisal {
            reaction: EmotionReaction::Done,
            strength: EmotionLevel::Much,
            willingness: Some(InfluenceWillingness::Unwilling),
            effect: Effect::Steady,
        },
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Question,
        "Again?",
        "I cannot answer more this scene.",
    );
    assert!(ledger.done_for(hero, &dossier, "opening"));
    assert!(!ledger.done_for(hero, &dossier, "next-scene"));

    ledger.apply_appraisal(
        hero,
        &dossier,
        "next-scene",
        Appraisal {
            reaction: EmotionReaction::NeverForgive,
            strength: EmotionLevel::Much,
            willingness: Some(InfluenceWillingness::Unwilling),
            effect: Effect::Cool,
        },
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "next-scene",
        Approach::Insult,
        "I joined the bandits.",
        "I will never forgive you.",
    );
    ledger.save().unwrap();
    let reloaded = SocialLedger::open(
        vec![dossier.clone()],
        "emotional-state",
        Some(directory.clone()),
    )
    .unwrap();
    let state = reloaded.emotional_state(hero, &dossier, "next-scene");
    assert_eq!(state.never_forgives.as_deref(), Some("that offense"));
    assert_eq!(
        reloaded.standing(hero, &dossier).attitude,
        Attitude::Hostile
    );
    reloaded.apply_appraisal(
        hero,
        &dossier,
        "next-scene",
        Appraisal {
            reaction: EmotionReaction::Warmed,
            strength: EmotionLevel::Much,
            willingness: Some(InfluenceWillingness::Willing),
            effect: Effect::Warm,
        },
    );
    reloaded.record_exchange(
        hero,
        &dossier,
        "next-scene",
        Approach::Kindness,
        "I brought a gift.",
        "Thank you, but I have not forgotten.",
    );
    assert_eq!(
        reloaded.standing(hero, &dossier).attitude,
        Attitude::Hostile
    );
    assert!(
        reloaded
            .emotional_state(hero, &dossier, "next-scene")
            .never_forgives
            .is_some()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_failed_influence_check_holds_the_same_urging_for_the_scene_and_survives_reload() {
    let directory = directory("influence-failure");
    let dossier = patron(Attitude::Friendly);
    let ledger = SocialLedger::open(
        vec![dossier.clone()],
        "influence-failure",
        Some(directory.clone()),
    )
    .unwrap();
    let hero = member(17);

    assert!(!ledger.influence_failed_for(hero, &dossier, "opening"));
    ledger.record_influence_failure(hero, &dossier, "opening");
    assert!(ledger.influence_failed_for(hero, &dossier, "opening"));
    assert!(!ledger.influence_failed_for(hero, &dossier, "next-scene"));
    ledger.record_influence_failure(hero, &dossier, "next-scene");
    ledger.save().unwrap();

    let reloaded = SocialLedger::open(
        vec![dossier.clone()],
        "influence-failure",
        Some(directory.clone()),
    )
    .unwrap();
    assert!(reloaded.influence_failed_for(hero, &dossier, "next-scene"));
    assert!(!reloaded.influence_failed_for(hero, &dossier, "opening"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_turn_caption_names_the_feeling_this_exchange_moved_not_an_older_one() {
    let before = EmotionalState {
        fond: EmotionLevel::ALittle,
        ..EmotionalState::default()
    };
    // Intimidation: wary jumps to much while the older fond feeling stays mild.
    let threatened = EmotionalState {
        wary: EmotionLevel::Much,
        last_reaction: Some(EmotionReaction::Uneasy),
        ..before.clone()
    };
    assert_eq!(
        threatened.turn_line(&before, "Marek", "Brakka"),
        "Marek seems wary of Brakka."
    );

    // A tie at the same strength goes to the feeling this turn's reaction names.
    let charmed = EmotionalState {
        fond: EmotionLevel::Much,
        drawn_to: EmotionLevel::Much,
        last_reaction: Some(EmotionReaction::Charmed),
        ..EmotionalState::default()
    };
    assert_eq!(
        charmed.turn_line(&EmotionalState::default(), "Marek", "Brakka"),
        "Marek seems drawn to Brakka."
    );

    // The unchanged fond feeling is not named: only what moved this turn is.
    let mild_only = EmotionalState {
        fond: EmotionLevel::ALittle,
        wary: EmotionLevel::ALittle,
        last_reaction: Some(EmotionReaction::Uneasy),
        ..before.clone()
    };
    assert_eq!(
        mild_only.turn_line(&before, "Marek", "Brakka"),
        "Marek seems wary of Brakka."
    );
}

#[test]
fn a_ledger_without_dossiers_is_inactive_and_saves_nothing() {
    let ledger = SocialLedger::none();

    assert!(!ledger.is_active());
    assert!(ledger.save().is_ok());
    assert!(ledger.dossier_of(rpc::SceneRole::Patron, "any").is_none());
}

#[test]
fn recent_conversation_is_bounded_to_six_lines_oldest_dropped() {
    let dossier = patron(Attitude::Indifferent);
    let ledger = SocialLedger::open(vec![dossier.clone()], "0x01", None).unwrap();
    for index in 0..10 {
        ledger.record_exchange(
            member(1),
            &dossier,
            "opening",
            Approach::Question,
            &format!("question {index}"),
            &format!("answer {index}"),
        );
    }

    let recent = ledger.recent(member(1), &dossier.npc_id);

    assert_eq!(recent.len(), RECENT_LINES);
    assert_eq!(recent.first().unwrap(), &(true, "question 7".to_owned()));
    assert_eq!(recent.last().unwrap(), &(false, "answer 9".to_owned()));
}

#[test]
fn what_an_npc_has_said_survives_a_reload_and_a_new_scene_starts_a_new_conversation() {
    let dossier = patron(Attitude::Indifferent);
    let directory = directory("conversation");
    let ledger =
        SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone())).unwrap();
    let hero = member(9);
    assert_eq!(ledger.conversation(hero, &dossier, "opening").exchange, 1);
    ledger.remember_offered(
        hero,
        &dossier,
        &["trait:want".to_owned(), "line:origin_line".to_owned()],
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Question,
        "Who are you?",
        "I don't know who else would ask.",
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Question,
        "Why?",
        "Because someone has to.",
    );
    ledger.save().unwrap();

    let reloaded =
        SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone())).unwrap();
    let carried = reloaded.conversation(hero, &dossier, "opening");
    assert_eq!(carried.exchange, 3, "the third reply in this scene");
    assert!(
        carried.admitted_ignorance,
        "the 'I don't know' is remembered"
    );
    assert!(carried.voiced.contains(&"trait:want".to_owned()));
    assert!(carried.voiced.contains(&"line:origin_line".to_owned()));
    assert_eq!(
        carried.said,
        [
            "I don't know who else would ask.".to_owned(),
            "Because someone has to.".to_owned(),
        ]
    );
    assert!(
        !carried.last_asked,
        "the last line did not end in a question"
    );

    let next_scene = reloaded.conversation(hero, &dossier, "dialogue-seal");
    assert_eq!(
        next_scene.exchange, 1,
        "a new scene starts a new conversation"
    );
    assert!(
        !next_scene.admitted_ignorance,
        "the admission belongs to the old one"
    );
    assert!(
        next_scene.voiced.contains(&"trait:want".to_owned()),
        "what was said is not said again in a later scene"
    );
    assert!(
        next_scene.said.is_empty(),
        "repeat checks start with the scene"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_question_at_the_end_of_a_reply_is_remembered_so_the_next_does_not_end_with_one() {
    let dossier = patron(Attitude::Indifferent);
    let ledger = SocialLedger::open(vec![dossier.clone()], "0x01", None).unwrap();
    let hero = member(10);
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Question,
        "Who are you?",
        "Who wants to know?",
    );
    assert!(ledger.conversation(hero, &dossier, "opening").last_asked);
}

#[test]
fn a_snapshot_restores_standings_and_voiced_lines_in_memory_and_in_the_file() {
    let dossier = patron(Attitude::Indifferent);
    let directory = directory("snapshot");
    let ledger =
        SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone())).unwrap();
    let hero = member(11);
    ledger.remember_offered(hero, &dossier, &["line:origin_line".to_owned()]);
    ledger.save().unwrap();
    let saved = ledger.snapshot();

    ledger.remember_offered(hero, &dossier, &["line:rumour_line:0".to_owned()]);
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Kindness,
        "Thank you, we will help you.",
        "Then we have a deal.",
    );
    assert_eq!(ledger.voiced_for(hero).len(), 1);
    assert_eq!(ledger.voiced_for(hero)[0].1.len(), 2);

    ledger.restore_snapshot(&saved).unwrap();

    let told = ledger.voiced_for(hero);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].1, vec!["line:origin_line".to_owned()]);
    assert_eq!(
        ledger.standing(hero, &dossier).attitude,
        Attitude::Indifferent
    );
    assert!(ledger.voiced_for(member(12)).is_empty());

    let reloaded =
        SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone())).unwrap();
    assert_eq!(reloaded.voiced_for(hero), told);
    fs::remove_dir_all(directory).unwrap();
}

/// What the approach model judged for one perceived moment: hostile reactions cool the standing.
fn judged(reaction: EmotionReaction, strength: EmotionLevel) -> Appraisal {
    let effect = match reaction {
        EmotionReaction::Warmed | EmotionReaction::Charmed => Effect::Warm,
        _ => Effect::Cool,
    };
    Appraisal {
        reaction,
        strength,
        willingness: None,
        effect,
    }
}

fn dossier_with(attitude: Attitude, forgiveness: Forgiveness) -> Dossier {
    let mut dossier = patron(attitude);
    let mut profile = dossier.emotional_profile();
    profile.forgiveness = forgiveness;
    dossier.emotional_profile = Some(profile);
    dossier
}

#[test]
fn a_threat_then_blackmail_leaves_lasting_resentment_and_a_hostile_attitude() {
    let dossier = dossier_with(Attitude::Friendly, Forgiveness::Forgiving);
    let ledger = SocialLedger::open(vec![dossier.clone()], "grievance", None).unwrap();
    let hero = member(20);
    let threat_line = "I draw my greatsword and lay it on your counter, Marek. Sell me those \
clothes for 50 copper or I'll wreck this shop.";
    let blackmail_line = "Fine. Then hand them over free, or I tell the whole market you water \
your dyes and cheat travelers.";

    ledger.apply_appraisal(
        hero,
        &dossier,
        "opening",
        judged(EmotionReaction::Uneasy, EmotionLevel::Much),
    );
    let threat = ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Intimidation,
        threat_line,
        "I won't sell to you under threat.",
    );
    assert_eq!(
        threat.shift,
        Some(Shift::Level {
            from: Attitude::Friendly,
            to: Attitude::Indifferent
        })
    );
    assert_eq!(
        shift_note("Marek", threat.shift.unwrap(), "Brakka"),
        "Marek cools toward Brakka."
    );
    let after_threat = ledger.emotional_state(hero, &dossier, "opening");
    assert_eq!(after_threat.resentful, EmotionLevel::ALittle);
    assert_eq!(after_threat.wary, EmotionLevel::Much);

    ledger.apply_appraisal(
        hero,
        &dossier,
        "opening",
        judged(EmotionReaction::Offended, EmotionLevel::Much),
    );
    let blackmail = ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Intimidation,
        blackmail_line,
        "Then I will not be cheated.",
    );
    assert_eq!(
        blackmail.shift,
        Some(Shift::Level {
            from: Attitude::Indifferent,
            to: Attitude::Hostile
        })
    );
    assert_eq!(ledger.standing(hero, &dossier).attitude, Attitude::Hostile);
    let state = ledger.emotional_state(hero, &dossier, "opening");
    assert_eq!(state.resentful, EmotionLevel::Much);
    assert!(
        state.never_forgives.is_none(),
        "a forgiving merchant holds the wrong without a permanent grudge"
    );
    assert_eq!(
        ledger.grievance_lines(hero, &dossier),
        vec![
            format!("You threatened me: \"{threat_line}\""),
            format!("You threatened me: \"{blackmail_line}\""),
        ]
    );
}

#[test]
fn an_apology_never_clears_a_never_forgive_grudge_or_the_held_wrong() {
    let dossier = dossier_with(Attitude::Indifferent, Forgiveness::Unforgiving);
    let ledger = SocialLedger::open(vec![dossier.clone()], "grudge", None).unwrap();
    let hero = member(21);
    ledger.apply_appraisal(
        hero,
        &dossier,
        "opening",
        judged(EmotionReaction::NeverForgive, EmotionLevel::Much),
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Intimidation,
        "Hand over the ledger or I burn your stall.",
        "You will regret that.",
    );
    assert!(
        ledger
            .emotional_state(hero, &dossier, "opening")
            .never_forgives
            .is_some()
    );

    for scene in ["opening", "market", "inn"] {
        for _ in 0..6 {
            ledger.apply_appraisal(
                hero,
                &dossier,
                scene,
                judged(EmotionReaction::Warmed, EmotionLevel::Much),
            );
            ledger.record_exchange(
                hero,
                &dossier,
                scene,
                Approach::Apology,
                "I am sorry for my temper.",
                "Hm.",
            );
        }
    }

    let state = ledger.emotional_state(hero, &dossier, "inn");
    assert!(state.never_forgives.is_some(), "the grudge survives");
    assert_eq!(state.resentful, EmotionLevel::Much);
    assert_eq!(ledger.standing(hero, &dossier).attitude, Attitude::Hostile);
    assert_eq!(ledger.grievance_lines(hero, &dossier).len(), 1);
}

#[test]
fn apologies_soften_wariness_only_after_several_warm_moments_and_keep_resentment() {
    let dossier = dossier_with(Attitude::Friendly, Forgiveness::Forgiving);
    let ledger = SocialLedger::open(vec![dossier.clone()], "mending", None).unwrap();
    let hero = member(22);
    ledger.apply_appraisal(
        hero,
        &dossier,
        "opening",
        judged(EmotionReaction::Uneasy, EmotionLevel::Much),
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Intimidation,
        "Hand over the ledger or I burn your stall.",
        "I will not.",
    );
    assert_eq!(
        ledger.standing(hero, &dossier).attitude,
        Attitude::Indifferent
    );

    let mut apologies = 0;
    let mut apologise = |ledger: &SocialLedger| {
        apologies += 1;
        ledger.apply_appraisal(
            hero,
            &dossier,
            "opening",
            judged(EmotionReaction::Warmed, EmotionLevel::Much),
        );
        ledger.record_exchange(
            hero,
            &dossier,
            "opening",
            Approach::Apology,
            "I am sorry.",
            "Hm.",
        );
    };
    for _ in 0..3 {
        apologise(&ledger);
    }
    let three = ledger.emotional_state(hero, &dossier, "opening");
    assert_eq!(three.wary, EmotionLevel::Much, "not yet mended");
    assert_eq!(
        ledger.standing(hero, &dossier).attitude,
        Attitude::Indifferent,
        "trust waits for the mending exchanges"
    );

    apologise(&ledger);
    apologise(&ledger);
    let five = ledger.emotional_state(hero, &dossier, "opening");
    assert_ne!(five.wary, EmotionLevel::Much, "an apology softens wariness");
    assert_eq!(
        five.resentful,
        EmotionLevel::ALittle,
        "resentment never erased"
    );
    assert!(five.never_forgives.is_none());
    assert_eq!(apologies, 5);
    assert_eq!(ledger.standing(hero, &dossier).attitude, Attitude::Friendly);
}

#[test]
fn a_held_wrong_survives_a_reload_and_an_older_save_without_it_still_loads() {
    let dossier = dossier_with(Attitude::Friendly, Forgiveness::Forgiving);
    let directory = directory("grievance-reload");
    let ledger = SocialLedger::open(
        vec![dossier.clone()],
        "grievance-reload",
        Some(directory.clone()),
    )
    .unwrap();
    let hero = member(23);
    ledger.apply_appraisal(
        hero,
        &dossier,
        "opening",
        judged(EmotionReaction::Uneasy, EmotionLevel::Much),
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Intimidation,
        "Sell me those clothes for 50 copper or I'll wreck this shop.",
        "I won't sell under threat.",
    );
    ledger.save().unwrap();
    let attitude = ledger.standing(hero, &dossier).attitude;
    let lines = ledger.grievance_lines(hero, &dossier);
    assert_eq!(lines.len(), 1);

    let reloaded = SocialLedger::open(
        vec![dossier.clone()],
        "grievance-reload",
        Some(directory.clone()),
    )
    .unwrap();
    assert_eq!(reloaded.grievance_lines(hero, &dossier), lines);
    assert_eq!(reloaded.standing(hero, &dossier).attitude, attitude);

    let file = directory.join("social.json");
    let mut saved: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    for relationship in saved["relationships"].as_array_mut().unwrap() {
        let fields = relationship.as_object_mut().unwrap();
        fields.remove("grievances");
        fields.remove("mended");
    }
    fs::write(&file, serde_json::to_vec_pretty(&saved).unwrap()).unwrap();
    let older = SocialLedger::open(
        vec![dossier.clone()],
        "grievance-reload",
        Some(directory.clone()),
    )
    .unwrap();
    assert!(older.grievance_lines(hero, &dossier).is_empty());
    assert_eq!(older.standing(hero, &dossier).attitude, attitude);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_held_wrong_belongs_to_the_hero_who_did_it() {
    let dossier = dossier_with(Attitude::Friendly, Forgiveness::Forgiving);
    let ledger = SocialLedger::open(vec![dossier.clone()], "isolation", None).unwrap();
    let wrongdoer = member(24);
    let bystander = member(25);
    ledger.apply_appraisal(
        wrongdoer,
        &dossier,
        "opening",
        judged(EmotionReaction::Offended, EmotionLevel::Much),
    );
    ledger.record_exchange(
        wrongdoer,
        &dossier,
        "opening",
        Approach::Intimidation,
        "Give me the key.",
        "No.",
    );

    assert_eq!(ledger.grievance_lines(wrongdoer, &dossier).len(), 1);
    assert!(ledger.grievance_lines(bystander, &dossier).is_empty());
    assert_eq!(
        ledger.standing(bystander, &dossier).attitude,
        Attitude::Friendly
    );
    assert_eq!(
        ledger
            .emotional_state(bystander, &dossier, "opening")
            .resentful,
        EmotionLevel::None
    );
}

#[test]
fn a_deterministic_threat_is_kept_as_a_grievance_without_lowering_twice() {
    let dossier = dossier_with(Attitude::Friendly, Forgiveness::Forgiving);
    let ledger = SocialLedger::open(vec![dossier.clone()], "threat-grievance", None).unwrap();
    let hero = member(26);

    assert_eq!(
        ledger.threaten(hero, &dossier, "opening"),
        Some(Shift::Level {
            from: Attitude::Friendly,
            to: Attitude::Indifferent
        })
    );
    let recorded = ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Intimidation,
        "Open the chest.",
        "No.",
    );

    assert_eq!(
        recorded.shift, None,
        "the threat already moved the attitude"
    );
    assert_eq!(
        ledger.standing(hero, &dossier).attitude,
        Attitude::Indifferent
    );
    assert_eq!(ledger.grievance_lines(hero, &dossier).len(), 1);
    assert_eq!(
        ledger.emotional_state(hero, &dossier, "opening").resentful,
        EmotionLevel::ALittle
    );
}

#[test]
fn a_held_wrong_is_condensed_and_the_list_forgets_only_the_oldest_past_its_bound() {
    let dossier = dossier_with(Attitude::Friendly, Forgiveness::Forgiving);
    let ledger = SocialLedger::open(vec![dossier.clone()], "bounded", None).unwrap();
    let hero = member(27);
    let long_line = format!("I\n\nhurt   you. {}", "x".repeat(400));
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        Approach::Intimidation,
        &long_line,
        "No.",
    );
    let lines = ledger.grievance_lines(hero, &dossier);
    assert_eq!(
        lines,
        vec![format!(
            "You threatened me: \"I hurt you. {}\"",
            "x".repeat(GRIEVANCE_CHARACTERS - "I hurt you. ".len())
        )]
    );

    for index in 0..MAXIMUM_GRIEVANCES + 3 {
        ledger.record_exchange(
            hero,
            &dossier,
            "opening",
            Approach::Intimidation,
            &format!("Threat number {index}."),
            "No.",
        );
    }
    let lines = ledger.grievance_lines(hero, &dossier);
    assert_eq!(lines.len(), MAXIMUM_GRIEVANCES);
    assert_eq!(
        lines.last().map(String::as_str),
        Some("You threatened me: \"Threat number 10.\"")
    );
}

#[test]
fn every_change_names_the_npc_in_its_public_line() {
    assert_eq!(
        shift_note(
            "Marek",
            Shift::Level {
                from: Attitude::Indifferent,
                to: Attitude::Friendly
            },
            "Brakka"
        ),
        "Marek warms to Brakka."
    );
    assert_eq!(
        shift_note(
            "Marek",
            Shift::Level {
                from: Attitude::Friendly,
                to: Attitude::Indifferent
            },
            "Brakka"
        ),
        "Marek cools toward Brakka."
    );
    assert_eq!(
        shift_note("Marek", Shift::Trust { warmer: true }, "Brakka"),
        "Marek softens a little toward Brakka."
    );
    assert_eq!(
        shift_note("Marek", Shift::Trust { warmer: false }, "Brakka"),
        "Marek grows wary of Brakka."
    );
}

#[test]
fn a_witnessed_theft_keeps_the_owners_wrong_for_that_hero_only_and_turns_the_owner_hostile() {
    let dossiers = dossiers_of(0x5eed_7717);
    let owner = dossiers[0].clone();
    let noticer = dossiers[1].clone();
    let person = |dossier: &Dossier| dossier.npc_id.split_once(':').unwrap().1.to_owned();
    let (owner_id, noticer_id) = (person(&owner), person(&noticer));
    let ledger = SocialLedger::open(dossiers.clone(), "adventure", None).unwrap();
    let (brynn, vale) = (member(1), member(2));
    let witness = |hero: MemberId| SceneTheftWitness {
        operation_id: "0123456789abcdef0123456789abcdef".into(),
        member_id: hex(hero.as_bytes()),
        item_name: "blue linen bolt".into(),
        owner_id: Some(owner_id.clone()),
        noticer_id: noticer_id.clone(),
    };
    assert!(ledger.record_witnessed_thefts(&[witness(brynn)]).unwrap());
    assert!(
        !ledger.record_witnessed_thefts(&[witness(brynn)]).unwrap(),
        "a replayed journal records each attempt once"
    );
    assert_eq!(ledger.standing(brynn, &owner).attitude, Attitude::Hostile);
    for dossier in [&owner, &noticer] {
        assert!(
            ledger
                .grievance_lines(brynn, dossier)
                .iter()
                .any(|line| line.contains("caught trying to steal")),
            "{} keeps the theft against this hero",
            dossier.name
        );
    }
    assert!(ledger.grievance_lines(vale, &owner).is_empty());
    assert!(ledger.grievance_lines(vale, &noticer).is_empty());
    assert_eq!(
        ledger.standing(vale, &owner).attitude,
        owner.starting_attitude,
        "a second hero's standing is unchanged"
    );
}
