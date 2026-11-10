//! The music table: the shipped file, every refusal of a table that does not hold together, the
//! track names, and the closed keys of the wire types.

use serde_json::{Value, json};

use super::cli::{TempoCommand, parse};
use super::request::{
    ConditionCause, DecisionAction, DecisionReason, EngineCause, ImpulseRequest, Momentum,
    PhaseKey, TableCause, TempoError,
};
use super::table::MusicTable;
use crate::command::{CommandError, split_words};
use crate::dev_http::{BodyCause, BodyError};

const SHIPPED: &str = include_str!("../../../../development/market-threads/free-roam.music.json");

fn shipped() -> Value {
    serde_json::from_str(SHIPPED).expect("the shipped table is JSON")
}

/// The shipped table edited by `edit`, read the way `tempo load` reads it.
fn read(edit: impl FnOnce(&mut Value)) -> Result<MusicTable, TempoError> {
    let mut value = shipped();
    edit(&mut value);
    MusicTable::from_json(value.to_string().as_bytes())
}

fn cause(edit: impl FnOnce(&mut Value)) -> TableCause {
    match read(edit) {
        Err(TempoError::InvalidTable { cause }) => cause,
        other => panic!("expected a refused table, got {other:?}"),
    }
}

fn body_cause(edit: impl FnOnce(&mut Value)) -> BodyCause {
    match read(edit) {
        Err(TempoError::InvalidBody(BodyError { cause, .. })) => cause,
        other => panic!("expected an unreadable table, got {other:?}"),
    }
}

#[test]
fn the_shipped_table_holds_together_and_is_one_score() {
    let table = MusicTable::from_json(SHIPPED.as_bytes()).expect("the shipped table is valid");
    let ids: Vec<&str> = table
        .profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect();
    assert_eq!(ids, ["show", "market", "study", "road", "town", "journey"]);
    assert_eq!(table.base, "journey");
    for profile in &table.profiles {
        assert!(
            (60..=90).contains(&profile.length_seconds),
            "{}",
            profile.id
        );
        assert!(profile.looping, "{}", profile.id);
        assert!(profile.prompt.contains("no vocals"), "{}", profile.id);
        assert!(profile.prompt.contains("G major"), "{}", profile.id);
        assert!(profile.prompt.contains("same key"), "{}", profile.id);
        assert!(profile.prompt.contains("no fade-out"), "{}", profile.id);
        // Music about a third of the voice, which plays at 80.
        assert!(
            (18..=28).contains(&profile.volume_percent),
            "{}",
            profile.id
        );
    }
}

#[test]
fn an_unknown_condition_or_phase_or_field_is_an_unreadable_table() {
    assert_eq!(
        body_cause(|table| table["profiles"][0]["when"]["weather"] = json!("rain")),
        BodyCause::Data
    );
    assert_eq!(
        body_cause(|table| table["profiles"][0]["when"]["phases"] = json!(["frantic"])),
        BodyCause::Data
    );
    assert_eq!(
        body_cause(|table| table["director"]["crossfade"] = json!(1)),
        BodyCause::Data
    );
    assert_eq!(
        body_cause(|table| {
            table.as_object_mut().expect("an object").remove("base");
        }),
        BodyCause::Data
    );
}

#[test]
fn a_condition_that_names_nothing_real_is_refused_with_its_profile() {
    let refused = |condition| TableCause::InvalidCondition {
        id: "show".to_owned(),
        condition,
    };
    assert_eq!(
        cause(|table| table["profiles"][0]["when"]["phases"] = json!([])),
        refused(ConditionCause::EmptyPhases)
    );
    assert_eq!(
        cause(|table| {
            table["profiles"][0]["when"]["intensity_min"] = json!(600);
            table["profiles"][0]["when"]["intensity_max"] = json!(300);
        }),
        refused(ConditionCause::InvalidIntensityRange)
    );
    assert_eq!(
        cause(|table| table["profiles"][0]["when"]["intensity_max"] = json!(1001)),
        refused(ConditionCause::InvalidIntensityRange)
    );
    assert_eq!(
        cause(|table| table["profiles"][0]["when"]["recent"]["kinds"] = json!([])),
        refused(ConditionCause::InvalidRecent)
    );
    assert_eq!(
        cause(|table| table["profiles"][0]["when"]["recent"]["within_ms"] = json!(0)),
        refused(ConditionCause::InvalidRecent)
    );
    assert_eq!(
        cause(|table| table["profiles"][0]["when"]["recent"]["kinds"] = json!(["jump"])),
        refused(ConditionCause::UnknownImpulseKind)
    );
}

#[test]
fn duplicate_ids_a_missing_base_and_bad_bounds_are_refused() {
    assert_eq!(
        cause(|table| table["profiles"][1]["id"] = json!("show")),
        TableCause::DuplicateProfile {
            id: "show".to_owned()
        }
    );
    assert_eq!(
        cause(|table| table["impulses"][1]["kind"] = json!("arrive_road")),
        TableCause::DuplicateImpulse {
            kind: "arrive_road".to_owned()
        }
    );
    assert_eq!(
        cause(|table| table["base"] = json!("nowhere")),
        TableCause::UnknownBase {
            id: "nowhere".to_owned()
        }
    );
    for (length_seconds, valid) in [(2, false), (3, true), (600, true), (601, false)] {
        let refused = read(|table| table["profiles"][0]["length_seconds"] = json!(length_seconds));
        assert_eq!(refused.is_ok(), valid, "{length_seconds}");
        if !valid {
            assert_eq!(
                refused.err(),
                Some(TempoError::InvalidTable {
                    cause: TableCause::InvalidLength {
                        id: "show".to_owned(),
                        length_seconds
                    }
                })
            );
        }
    }
    assert_eq!(
        cause(|table| table["profiles"][0]["volume_percent"] = json!(101)),
        TableCause::InvalidVolume {
            id: "show".to_owned(),
            volume_percent: 101
        }
    );
    assert_eq!(
        cause(|table| table["profiles"][0]["prompt"] = json!("   ")),
        TableCause::InvalidPrompt {
            id: "show".to_owned(),
            prompt_characters: 3
        }
    );
    assert_eq!(
        cause(|table| table["profiles"][0]["crossfade_ms"] = json!(60_001)),
        TableCause::InvalidCrossfade {
            id: "show".to_owned(),
            crossfade_ms: 60_001
        }
    );
    assert_eq!(
        cause(|table| table["profiles"][0]["id"] = json!("not a name")),
        TableCause::InvalidId {
            id: "not a name".to_owned()
        }
    );
    assert_eq!(
        cause(|table| table["impulses"][0]["impulse_permille"] = json!(1001)),
        TableCause::InvalidImpulse {
            kind: "arrive_road".to_owned()
        }
    );
    assert_eq!(
        cause(|table| table["profiles"] = json!([])),
        TableCause::NoProfiles
    );
}

#[test]
fn a_policy_the_engine_refuses_is_refused_by_the_table() {
    assert_eq!(
        cause(|table| table["policy"]["thresholds"] = json!([650, 380, 150])),
        TableCause::InvalidPolicy
    );
    assert_eq!(
        cause(|table| table["policy"]["decay_half_life_ms"] = json!(0)),
        TableCause::InvalidPolicy
    );
    assert_eq!(
        cause(|table| table["policy"]["rest_permille"] = json!(1001)),
        TableCause::InvalidPolicy
    );
    assert_eq!(
        cause(|table| table["director"]["track_prefix"] = json!("not a prefix")),
        TableCause::InvalidDirector
    );
}

#[test]
fn a_track_is_named_by_what_it_is_asked_for_and_found_again_by_that_name() {
    let table = MusicTable::from_json(SHIPPED.as_bytes()).expect("valid");
    let journey = table.profile("journey").expect("is there");
    let name = journey.track_name(&table.director);
    assert_eq!(name, journey.track_name(&table.director));
    assert!(name.starts_with("tempo-journey-"), "{name}");
    let changed = read(|table| {
        table["profiles"][5]["prompt"] = json!("another prompt entirely");
    })
    .expect("valid");
    let renamed = changed.profile("journey").expect("is there");
    assert_ne!(renamed.track_name(&changed.director), name);
    let longer = read(|table| table["profiles"][5]["length_seconds"] = json!(91)).expect("valid");
    assert_ne!(
        longer
            .profile("journey")
            .expect("is there")
            .track_name(&longer.director),
        name
    );
    // Volume and crossfade do not change what the composer makes.
    let louder = read(|table| table["profiles"][5]["volume_percent"] = json!(30)).expect("valid");
    assert_eq!(
        louder
            .profile("journey")
            .expect("is there")
            .track_name(&louder.director),
        name
    );
    let request = journey.music_request(&table.director);
    assert_eq!(
        (request.name.as_str(), request.length_seconds),
        (name.as_str(), 90)
    );
    assert_eq!(request.check(), Ok(()));
}

#[test]
fn an_impulse_with_a_strength_over_a_thousand_or_a_bad_note_is_refused() {
    let request = |intensity, note: Option<&str>| ImpulseRequest {
        kind: "look".to_owned(),
        intensity,
        note: note.map(str::to_owned),
    };
    assert_eq!(request(Some(1_000), Some("a fine note")).check(), Ok(()));
    assert_eq!(
        request(Some(1_001), None).check(),
        Err(TempoError::InvalidIntensity { intensity: 1_001 })
    );
    assert_eq!(
        request(None, Some("two\nlines")).check(),
        Err(TempoError::InvalidNote { note_characters: 9 })
    );
    let long = "x".repeat(201);
    assert_eq!(
        request(None, Some(&long)).check(),
        Err(TempoError::InvalidNote {
            note_characters: 201
        })
    );
}

fn words(line: &str) -> Vec<String> {
    split_words(line).expect("the line splits")
}

#[test]
fn the_tempo_verbs_parse_and_refuse_as_the_usage_says() {
    assert_eq!(parse(&words("")), Ok(TempoCommand::State));
    assert_eq!(parse(&words("state")), Ok(TempoCommand::State));
    assert_eq!(parse(&words("tick")), Ok(TempoCommand::Tick));
    assert_eq!(parse(&words("reset")), Ok(TempoCommand::Reset));
    assert_eq!(parse(&words("prepare")), Ok(TempoCommand::Prepare));
    assert_eq!(
        parse(&words(
            "impulse arrive_road 500 \"note=a road, then a town\""
        )),
        Ok(TempoCommand::Impulse(ImpulseRequest {
            kind: "arrive_road".to_owned(),
            intensity: Some(500),
            note: Some("a road, then a town".to_owned()),
        }))
    );
    assert_eq!(
        parse(&words("impulse look")),
        Ok(TempoCommand::Impulse(ImpulseRequest {
            kind: "look".to_owned(),
            intensity: None,
            note: None,
        }))
    );
    for line in [
        "impulse",
        "impulse look 5 6",
        "load",
        "load a b",
        "tick now",
        "dance",
    ] {
        assert!(
            matches!(parse(&words(line)), Err(CommandError::Usage(_))),
            "{line}"
        );
    }
    for line in [
        "impulse look loud",
        "impulse look 70000",
        "impulse bad.kind",
    ] {
        assert!(
            matches!(parse(&words(line)), Err(CommandError::BadChoice { .. })),
            "{line}"
        );
    }
}

#[test]
fn the_closed_keys_are_their_serde_names() {
    fn check<T: serde::Serialize + Copy>(all: &[T], key: impl Fn(T) -> &'static str) {
        for variant in all {
            let written = serde_json::to_value(variant).expect("writes");
            assert_eq!(written, Value::from(key(*variant)));
        }
    }
    check(&PhaseKey::ALL, PhaseKey::key);
    check(&Momentum::ALL, Momentum::key);
    check(&DecisionAction::ALL, DecisionAction::key);
    check(&DecisionReason::ALL, DecisionReason::key);
    check(&ConditionCause::ALL, ConditionCause::key);
    check(&EngineCause::ALL, EngineCause::key);
    for phase in PhaseKey::ALL {
        assert_eq!(PhaseKey::from_key(phase.key()), Some(phase));
    }
    let error = TempoError::UnknownImpulse {
        kind: "jump".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(&error).expect("writes")["error"],
        error.key()
    );
    let refused = TableCause::DuplicateProfile { id: "x".to_owned() };
    assert_eq!(
        serde_json::to_value(&refused).expect("writes")["cause"],
        refused.key()
    );
}

#[test]
fn a_director_that_never_decides_or_dwells_for_days_is_refused() {
    for edit in [
        |table: &mut Value| table["director"]["tick_ms"] = json!(0),
        |table: &mut Value| table["director"]["tick_ms"] = json!(3_600_001),
        |table: &mut Value| table["director"]["min_dwell_ms"] = json!(3_600_001),
        |table: &mut Value| table["director"]["hold_margin_permille"] = json!(1_001),
    ] {
        assert_eq!(cause(edit), TableCause::InvalidDirector);
    }
}
