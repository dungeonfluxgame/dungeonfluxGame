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
