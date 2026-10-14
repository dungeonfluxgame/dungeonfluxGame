use super::*;
use crate::gameplay::adventure::{
    ForcedEntries, GenerateOptions, Tables, dossier::NpcRole, generate,
};
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

#[test]
fn the_keyword_reader_names_the_social_approach_of_a_line() {
    for (line, expected) in [
        ("Who are you, really?", Approach::Question),
        ("You are the wisest person I have met.", Approach::Flattery),
        (
            "You are beautiful, has anyone told you?",
            Approach::Flirting,
        ),
        ("Hand over the key or else.", Approach::Intimidation),
        ("Here is ten gold for your trouble.", Approach::Bribe),
        ("Thank you, we will help you.", Approach::Kindness),
        ("You pathetic coward.", Approach::Insult),
        (
            "The king sent me, I am the royal envoy.",
            Approach::Deception,
        ),
        ("Please, trust me on this.", Approach::Persuasion),
        ("Hmm.", Approach::Other),
    ] {
        assert_eq!(classify_keywords(line), expected, "{line}");
    }
}

#[test]
fn the_keyword_reader_covers_ten_lines_for_each_approach_srd_5_2_1_page_184() {
    // SRD 5.2.1, p. 184: “Describe or roleplay how you're communicating with the monster.”
    // These examples classify table talk only; they do not make or resolve an Influence check.
    let examples: [(Approach, [&str; 10]); 11] = [
        (
            Approach::Question,
            [
                "Who is waiting at the ferry?",
                "What happened to the parcel?",
                "Why did you come here?",
                "Where should we go next?",
                "When does the boat leave?",
                "How do you know the mapmaker?",
                "Could you explain the warning?",
                "Tell me about this office.",
                "Would you name the person involved?",
                "May I ask what you need?",
            ],
        ),
        (
            Approach::Kindness,
            [
                "Thank you for trusting us.",
                "I am sorry this frightened you.",
                "We will help you carry it.",
                "Don't worry, you're safe with us.",
                "Are you alright after that journey?",
                "I understand this must be hard.",
                "Here to help, if you'll let us.",
                "Thanks for telling us the truth.",
                "Must be difficult to wait alone.",
                "I will help you reach your friend.",
            ],
        ),
        (
            Approach::Flattery,
            [
                "You are wise to be cautious.",
                "That was a brilliant plan.",
                "Your work is impressive.",
                "You are wonderfully clever.",
                "That is an amazing map.",
                "You have a famous reputation.",
                "You are a talented guide.",
                "Your judgment is magnificent.",
                "That was a great choice.",
                "You are the bravest clerk I know.",
            ],
        ),
        (
            Approach::Flirting,
            [
                "You look beautiful tonight.",
                "That smile is charming.",
                "Would you have dinner with me?",
                "Your eyes are lovely.",
                "I would gladly kiss you.",
                "You are gorgeous in that coat.",
                "May I call you sweetheart?",
                "I fancy you, if I'm honest.",
                "You have a handsome face.",
                "Care to share a dance, darling?",
            ],
        ),
        (
            Approach::Persuasion,
            [
                "Trust me, we can do this.",
                "I promise we will return it.",
                "Believe me, this is the safest path.",
                "Surely you can see the reason.",
                "Please let us explain our plan.",
                "The map will help us find him.",
                "We can reach the inn before night.",
                "Our plan gives everyone a chance.",
                "This is the best way to protect them.",
                "You should let us carry the message.",
            ],
        ),
        (
            Approach::Deception,
            [
                "I am the harbourmaster's deputy.",
                "I'm the captain's new courier.",
                "The harbourmaster sent us to inspect it.",
                "I work for the city watch.",
                "We came from the royal office.",
                "The captain told us you agreed already.",
                "I am a sworn mapmaker from the guild.",
                "The guard ordered us to take charge.",
                "I'm carrying an official order from the duke.",
                "The magistrate says you must tell us everything.",
            ],
        ),
        (
            Approach::Intimidation,
            [
                "Give it to me or else.",
                "Hand over the key or I will break your arm.",
                "I will hurt you if you refuse.",
                "Tell me or I will end you.",
                "You will regret making me wait.",
                "Talk or I draw my sword.",
                "Last warning before I burn this place down.",
                "I will make you bleed for that lie.",
                "Tell me now or I leave and Tomas can fend for himself.",
                "Give up the letter, or I'll cut your throat.",
            ],
        ),
        (
            Approach::Bribe,
            [
                "Here are ten gold pieces for your trouble.",
                "I will pay you to keep quiet.",
                "Take this silver if you help us.",
                "I offer a purse of coins for the key.",
                "Let me give you gold for the map.",
                "A few coins will make this worth your while.",
                "I'll pay you well to carry the note.",
                "Accept this payment and open the door.",
                "This gold is yours if you guide us.",
                "I can reward you with coin for your silence.",
            ],
        ),
        (
            Approach::Bargain,
            [
                "What will you pay us for carrying it?",
                "What do we get if we help you?",
                "How much is the reward?",
                "Name your price for the errand.",
                "Our price is ten gold.",
                "Will we be compensated for the danger?",
                "What do you offer in return?",
                "What's in it for us if we agree?",
                "Is there a payment for the journey?",
                "What reward can we expect?",
            ],
        ),
        (
            Approach::Insult,
            [
                "You are an idiot.",
                "That was foolish of you.",
                "You coward, answer me.",
                "That is a stupid idea.",
                "You are worthless at this job.",
                "What a pathetic excuse.",
                "You are useless to us.",
                "Don't be such a liar.",
                "You look disgusting in that coat.",
                "Only a fool would lose the letter.",
            ],
        ),
        (
            Approach::Other,
            [
                "Hmm.",
                "I tap the table twice.",
                "The rain is getting heavier.",
                "Let me think for a moment.",
                "I set my pack down here.",
                "That is a curious sound.",
                "We should keep moving soon.",
                "I look toward the doorway.",
                "This room smells of smoke.",
                "I nod once and wait.",
            ],
        ),
    ];
    for (expected, lines) in examples {
        for line in lines {
            assert_eq!(classify_keywords(line), expected, "{line}");
        }
    }
}

#[test]
fn the_keyword_reader_recognizes_plain_threats_without_mistaking_unrelated_death_words() {
    for line in [
        "Hand over the coin you owe us or I will break your arm.",
        "I said hand it over, old man, or you die here.",
        "Kill you if you touch that again.",
        "I will cut you if you refuse.",
        "I will gut you if you lie.",
        "I will break your arm.",
        "I will hurt you.",
        "I will burn it down if you make me.",
        "I will draw my sword.",
        "Put that blade to your throat.",
        "Last warning: give it to me.",
        "Or else, you will regret this.",
        "Hand it over now.",
    ] {
        assert_eq!(classify_keywords(line), Approach::Intimidation, "{line}");
    }
    for line in ["I WOULD KILL FOR A HOT MEAL.", "She nearly died laughing."] {
        assert_ne!(classify_keywords(line), Approach::Intimidation, "{line}");
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
fn the_dossiers_temperament_decides_whether_an_approach_warms_cools_or_changes_nothing() {
    let mut dossier = patron(Attitude::Indifferent);
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
fn every_change_has_a_short_public_line() {
    assert_eq!(
        shift_line(
            Shift::Level {
                from: Attitude::Indifferent,
                to: Attitude::Friendly
            },
            "Brakka"
        ),
        "warms to Brakka"
    );
    assert_eq!(
        shift_line(
            Shift::Level {
                from: Attitude::Friendly,
                to: Attitude::Indifferent
            },
            "Brakka"
        ),
        "cools toward Brakka"
    );
    assert_eq!(
        shift_line(Shift::Trust { warmer: false }, "Brakka"),
        "grows wary of Brakka"
    );
}

#[test]
fn attitude_and_the_recent_conversation_survive_a_reload() {
    let dossier = patron(Attitude::Indifferent);
    let directory = directory("reload");
    let ledger =
        SocialLedger::open(vec![dossier.clone()], "0x01", Some(directory.clone())).unwrap();
    let hero = member(7);
    for line in [
        "Thank you, we will help you.",
        "I am sorry for your trouble.",
    ] {
        ledger.record_exchange(
            hero,
            &dossier,
            "opening",
            classify_keywords(line),
            line,
            "Hm.",
        );
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
        classify_keywords("Who are you?"),
        "Who are you?",
        "I don't know who else would ask.",
    );
    ledger.record_exchange(
        hero,
        &dossier,
        "opening",
        classify_keywords("Why?"),
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
        classify_keywords("Who are you?"),
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
        classify_keywords("Thank you, we will help you."),
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
