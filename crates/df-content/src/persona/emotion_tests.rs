use serde_json::json;

use crate::persona::emotion::DEFAULT_EMOTIONS;
use crate::persona::emotion_generation::{
    TEMPERAMENT_INSTRUCTION, read_temperament, temperament_facts, temperament_schema,
};
use crate::persona::emotion_logic::MAXIMUM_RELATIONSHIPS;
use crate::persona::generation::{GenerationError, output_schema};
use crate::persona::*;

fn level(emotion: &str, level: u8) -> EmotionLevel {
    EmotionLevel {
        emotion: emotion.to_owned(),
        level,
    }
}

fn delta(emotion: &str, delta: i8) -> EmotionDelta {
    EmotionDelta {
        emotion: emotion.to_owned(),
        delta,
    }
}

fn appraise(emotions: Vec<EmotionDelta>) -> Appraisal {
    Appraisal {
        emotions,
        ..Appraisal::default()
    }
}

fn calm() -> Temperament {
    Temperament::default()
}

fn at_rest() -> PersonaState {
    fresh_state("innkeeper", &EmotionConfig::default(), &calm())
}

fn persona_with(temperament: Temperament, triggers: Vec<EmotionalTrigger>) -> Persona {
    Persona::new(PersonaDraft {
        id: "innkeeper".to_owned(),
        display_name: "Marta Venn".to_owned(),
        job: "Keeper of the inn".to_owned(),
        behaviors: vec![PersonaBehavior {
            id: "curt".to_owned(),
            description: "Short answers.".to_owned(),
            speech_style: String::new(),
        }],
        temperament,
        emotional_triggers: triggers,
        ..PersonaDraft::default()
    })
    .expect("valid")
}

fn state_trigger(id: &str, emotion: &str, at_least: u8) -> EmotionalTrigger {
    EmotionalTrigger {
        id: id.to_owned(),
        when_state: Some(StateCondition {
            emotion: emotion.to_owned(),
            at_least,
        }),
        when_event: None,
        sets: vec![delta("irritation", 5)],
        behavior: Some("curt".to_owned()),
    }
}

#[test]
fn the_built_in_list_is_the_default_emotions_and_is_usable() {
    let config = EmotionConfig::default();
    assert_eq!(config.ids(), DEFAULT_EMOTIONS);
    assert_eq!(config.check(), Ok(()));
}

#[test]
fn a_partial_emotion_file_keeps_the_built_in_rest_and_a_stray_key_is_refused() {
    let config: EmotionConfig = serde_json::from_str(r#"{"decay_percent": 10}"#).expect("reads");
    assert_eq!(config.decay_percent, 10);
    assert_eq!(config.ids(), DEFAULT_EMOTIONS);
    assert!(serde_json::from_str::<EmotionConfig>(r#"{"moods": []}"#).is_err());
}

#[test]
fn an_emotion_list_with_a_flaw_is_refused_with_its_key() {
    let mut config = EmotionConfig::default();
    config.emotions.clear();
    assert_eq!(config.check(), Err(EmotionConfigError::NoEmotions));

    let mut config = EmotionConfig::default();
    config.emotions[0].id = "bad id".to_owned();
    assert_eq!(config.check().unwrap_err().key(), "invalid_id");

    let mut config = EmotionConfig::default();
    config.emotions[1].id = "joy".to_owned();
    assert_eq!(config.check().unwrap_err().key(), "duplicate_id");

    let mut config = EmotionConfig::default();
    config.emotions[0].arousal = 101;
    assert_eq!(
        config.check(),
        Err(EmotionConfigError::OutOfRange {
            setting: EmotionSetting::Arousal
        })
    );

    let config = EmotionConfig {
        attention_capacity: 6,
        ..EmotionConfig::default()
    };
    assert_eq!(config.check().unwrap_err().key(), "out_of_range");

    let mut config = EmotionConfig::default();
    config.rapport.hostile_at_most = 40;
    assert_eq!(config.check(), Err(EmotionConfigError::ThresholdsCross));

    let mut config = EmotionConfig::default();
    config.emotions[0].tone = " ".to_owned();
    assert_eq!(config.check().unwrap_err().key(), "invalid_text");
    for setting in EmotionSetting::ALL {
        assert_eq!(EmotionSetting::from_key(setting.key()), Some(setting));
        assert_eq!(
            serde_json::to_value(setting).expect("writes"),
            json!(setting.key())
        );
    }
}

#[test]
fn a_fresh_state_rests_at_the_baseline_and_its_mood_follows() {
    let temperament = Temperament {
        baseline: vec![level("trust", 60), level("joy", 30)],
        ..calm()
    };
    let state = fresh_state("innkeeper", &EmotionConfig::default(), &temperament);
    assert_eq!(state.level("trust"), 60);
    assert_eq!(state.level("joy"), 30);
    assert_eq!(state.level("fear"), 0);
    assert_eq!(state.emotions.len(), DEFAULT_EMOTIONS.len());
    assert_eq!(state.turn, 0);
    assert!(state.mood.valence > 0);
    assert_eq!(at_rest().mood, Mood::default());
}

#[test]
fn a_change_lands_scaled_by_the_reactivity_and_clamped() {
    let config = EmotionConfig::default();
    let start = at_rest();
    let normal = apply_appraisal(&config, &calm(), &start, &appraise(vec![delta("joy", 40)])).0;
    assert_eq!(normal.level("joy"), 40);
    let keen = Temperament {
        reactivity: Some(100),
        ..calm()
    };
    let doubled = apply_appraisal(&config, &keen, &start, &appraise(vec![delta("joy", 40)])).0;
    assert_eq!(doubled.level("joy"), 80);
    let numb = Temperament {
        reactivity: Some(25),
        ..calm()
    };
    let halved = apply_appraisal(&config, &numb, &start, &appraise(vec![delta("joy", 40)])).0;
    assert_eq!(halved.level("joy"), 20);
    let flat = Temperament {
        reactivity: Some(0),
        ..calm()
    };
    let unmoved = apply_appraisal(&config, &flat, &start, &appraise(vec![delta("joy", 40)])).0;
    assert_eq!(unmoved.level("joy"), 0);

    let over = apply_appraisal(&config, &keen, &normal, &appraise(vec![delta("joy", 100)])).0;
    assert_eq!(over.level("joy"), 100);
    let under = apply_appraisal(&config, &calm(), &start, &appraise(vec![delta("joy", -50)])).0;
    assert_eq!(under.level("joy"), 0);
}

#[test]
fn grief_lingers_while_surprise_passes_and_a_volatile_character_cools_faster() {
    let config = EmotionConfig::default();
    let at = |decay: u8| {
        let temperament = Temperament {
            decay_percent: Some(decay),
            ..calm()
        };
        let mut state = fresh_state("innkeeper", &config, &temperament);
        state.emotions.iter_mut().for_each(|seen| {
            seen.level = match seen.emotion.as_str() {
                "sadness" | "surprise" => 40,
                _ => 0,
            }
        });
        apply_appraisal(&config, &temperament, &state, &Appraisal::default()).0
    };
    let ordinary = at(20);
    assert_eq!(ordinary.level("sadness"), 38, "40 - ceil(40 * 5%)");
    assert_eq!(ordinary.level("surprise"), 16, "40 - ceil(40 * 60%)");
    let volatile = at(40);
    assert_eq!(volatile.level("sadness"), 36, "twice the ordinary rate: 40 - ceil(40 * 10%)");
}

#[test]
fn an_emotion_still_being_provoked_does_not_fade_that_turn() {
    let config = EmotionConfig::default();
    let temperament = Temperament {
        decay_percent: Some(20),
        ..calm()
    };
    let mut state = fresh_state("innkeeper", &config, &temperament);
    state.emotions.iter_mut().for_each(|seen| {
        seen.level = match seen.emotion.as_str() {
            "irritation" | "joy" => 50,
            _ => 0,
        }
    });
    let provoked = appraise(vec![delta("irritation", 10)]);
    let after = apply_appraisal(&config, &temperament, &state, &provoked).0;
    assert_eq!(after.level("irritation"), 60, "50 + 10, with no fade while provoked");
    assert_eq!(after.level("joy"), 40, "an emotion not pushed still fades");
}

#[test]
fn feelings_fade_toward_the_baseline_a_share_each_turn_and_always_arrive() {
    let config = EmotionConfig::default();
    let temperament = Temperament {
        baseline: vec![level("trust", 50)],
        decay_percent: Some(20),
        ..calm()
    };
    let mut state = fresh_state("innkeeper", &config, &temperament);
    state.emotions.iter_mut().for_each(|seen| {
        seen.level = match seen.emotion.as_str() {
            "joy" => 80,
            "trust" => 10,
            _ => 0,
        }
    });
    let nothing = Appraisal::default();
    let after = apply_appraisal(&config, &temperament, &state, &nothing).0;
    assert_eq!(after.level("joy"), 64);
    assert_eq!(after.level("trust"), 14, "10 + ceil(40 * 10%): trust fades at its own rate");
    assert_eq!(after.turn, 1);
    let mut settling = after;
    for _ in 0..60 {
        settling = apply_appraisal(&config, &temperament, &settling, &nothing).0;
    }
    assert_eq!(settling.level("joy"), 0);
    assert_eq!(settling.level("trust"), 50);

    let still = Temperament {
        decay_percent: Some(0),
        ..calm()
    };
    let held = apply_appraisal(&config, &still, &state, &nothing).0;
    assert_eq!(held.level("joy"), 80);
}

#[test]
fn an_emotion_off_the_list_is_dropped_and_counted_and_the_rest_applies() {
    let config = EmotionConfig::default();
    let (state, dropped) = apply_appraisal(
        &config,
        &calm(),
        &at_rest(),
        &appraise(vec![delta("rage", 50), delta("fear", 20), delta("Joy", 10)]),
    );
    assert_eq!(dropped.emotions, 2);
    assert_eq!(state.level("fear"), 20);
    assert!(!dropped.is_none());
    assert!(!state.emotions.iter().any(|seen| seen.emotion == "rage"));
}

#[test]
fn a_rise_lands_whole_up_to_half_way_and_with_less_force_above_it() {
    let config = EmotionConfig::default();
    let push = Appraisal {
        emotions: vec![EmotionDelta {
            emotion: "joy".to_owned(),
            delta: 20,
        }],
        ..Appraisal::default()
    };
    let mut low = at_rest();
    assert!(set_emotion(&config, &calm(), &mut low, "joy", 30));
    let (low, _) = apply_appraisal(&config, &calm(), &low, &push);
    assert_eq!(low.level("joy"), 50, "30 + 20, whole");
    let mut high = at_rest();
    assert!(set_emotion(&config, &calm(), &mut high, "joy", 80));
    let (high, _) = apply_appraisal(&config, &calm(), &high, &push);
    assert_eq!(high.level("joy"), 88, "80 + 20 * 20 / 50: the room left above it");
    let fall = Appraisal {
        emotions: vec![EmotionDelta {
            emotion: "joy".to_owned(),
            delta: -20,
        }],
        ..Appraisal::default()
    };
    let (fallen, _) = apply_appraisal(&config, &calm(), &high, &fall);
    assert_eq!(fallen.level("joy"), 68, "a fall lands whole");
}

#[test]
fn a_standing_set_by_hand_takes_its_attitude_and_never_points_at_the_character_itself() {
    let config = EmotionConfig::default();
    let mut state = at_rest();
    assert!(set_rapport(&config, &mut state, "warrior", 45));
    let standing = state.relationship("warrior").expect("standing");
    assert_eq!(
        (standing.rapport, standing.attitude),
        (45, Attitude::Friendly)
    );
    assert!(set_rapport(&config, &mut state, "warrior", -100));
    assert_eq!(
        state.relationship("warrior").map(|seen| seen.attitude),
        Some(Attitude::Hostile)
    );
    assert!(!set_rapport(&config, &mut state, "innkeeper", 10));
    assert!(!set_rapport(&config, &mut state, "Not A Name", 10));
    assert_eq!(state.relationships.len(), 1);
}

#[test]
fn rapport_moves_the_attitude_across_the_thresholds_and_is_clamped() {
    let config = EmotionConfig::default();
    assert_eq!(config.attitude_for(30), Attitude::Friendly);
    assert_eq!(config.attitude_for(29), Attitude::Indifferent);
    assert_eq!(config.attitude_for(-29), Attitude::Indifferent);
    assert_eq!(config.attitude_for(-30), Attitude::Hostile);

    let toward = |name: &str, change: i8| Appraisal {
        toward: Some(name.to_owned()),
        rapport_delta: Some(change),
        ..Appraisal::default()
    };
    let (liked, _) = apply_appraisal(&config, &calm(), &at_rest(), &toward("warrior", 35));
    let standing = liked.relationship("warrior").expect("standing");
    assert_eq!(
        (standing.rapport, standing.attitude),
        (35, Attitude::Friendly)
    );
    let (cooled, _) = apply_appraisal(&config, &calm(), &liked, &toward("warrior", -10));
    assert_eq!(
        cooled.relationship("warrior").map(|seen| seen.attitude),
        Some(Attitude::Indifferent)
    );
    let (soured, _) = apply_appraisal(&config, &calm(), &cooled, &toward("warrior", -60));
    assert_eq!(
        soured
            .relationship("warrior")
            .map(|seen| (seen.rapport, seen.attitude)),
        Some((-35, Attitude::Hostile))
    );
    let (floor, _) = apply_appraisal(&config, &calm(), &soured, &toward("warrior", -128));
    assert_eq!(
        floor.relationship("warrior").map(|seen| seen.rapport),
        Some(-100)
    );
    let (ceiling, _) = apply_appraisal(&config, &calm(), &floor, &toward("warrior", 127));
    let (ceiling, _) = apply_appraisal(&config, &calm(), &ceiling, &toward("warrior", 127));
    assert_eq!(
        ceiling.relationship("warrior").map(|seen| seen.rapport),
        Some(100)
    );

    let mut strict = config.clone();
    strict.rapport = RapportThresholds {
        friendly_at_least: 60,
        hostile_at_most: -10,
    };
    let (guarded, _) = apply_appraisal(&strict, &calm(), &at_rest(), &toward("warrior", 35));
    assert_eq!(
        guarded.relationship("warrior").map(|seen| seen.attitude),
        Some(Attitude::Indifferent)
    );
}

#[test]
fn a_standing_toward_itself_or_a_non_label_is_dropped_and_the_list_is_bounded() {
    let config = EmotionConfig::default();
    let toward = |name: &str| Appraisal {
        toward: Some(name.to_owned()),
        rapport_delta: Some(10),
        ..Appraisal::default()
    };
    let (state, dropped) = apply_appraisal(&config, &calm(), &at_rest(), &toward("innkeeper"));
    assert_eq!(dropped.relationships, 1);
    assert!(state.relationships.is_empty());
    let (state, dropped) = apply_appraisal(&config, &calm(), &state, &toward("two words"));
    assert_eq!((dropped.relationships, state.relationships.len()), (1, 0));
    let mut crowded = at_rest();
    for index in 0..MAXIMUM_RELATIONSHIPS {
        crowded.relationships.push(Relationship {
            toward: format!("guest{index}"),
            attitude: Attitude::Indifferent,
            rapport: 0,
        });
    }
    let (crowded, dropped) = apply_appraisal(&config, &calm(), &crowded, &toward("stranger"));
    assert_eq!(dropped.relationships, 1);
    assert_eq!(crowded.relationships.len(), MAXIMUM_RELATIONSHIPS);
}

fn noticed(speaker: &str, gist: &str) -> Noticed {
    Noticed {
        speaker: speaker.to_owned(),
        gist: gist.to_owned(),
    }
}

#[test]
fn attention_keeps_the_newest_lines_up_to_the_capacity_and_holds_the_focus() {
    let config = EmotionConfig::default();
    let temperament = Temperament {
        attention_capacity: Some(2),
        ..calm()
    };
    let said = |lines: Vec<Noticed>, focus: Option<&str>| Appraisal {
        attention: Attention {
            focus: focus.map(str::to_owned),
            topic: None,
            noticed: lines,
        },
        ..Appraisal::default()
    };
    let (first, _) = apply_appraisal(
        &config,
        &temperament,
        &at_rest(),
        &said(
            vec![
                noticed("warrior", "asked the price"),
                noticed("wizard", "laughed"),
            ],
            Some("warrior"),
        ),
    );
    assert_eq!(first.attention.noticed.len(), 2);
    let (second, _) = apply_appraisal(
        &config,
        &temperament,
        &first,
        &said(vec![noticed("warrior", "haggled")], None),
    );
    let gists: Vec<&str> = second
        .attention
        .noticed
        .iter()
        .map(|line| line.gist.as_str())
        .collect();
    assert_eq!(gists, ["laughed", "haggled"], "the oldest is dropped");
    assert_eq!(second.attention.focus.as_deref(), Some("warrior"));

    let (default_capacity, _) = apply_appraisal(
        &config,
        &calm(),
        &at_rest(),
        &said(
            (0..5)
                .map(|n| noticed("warrior", &format!("line {n}")))
                .collect(),
            None,
        ),
    );
    assert_eq!(default_capacity.attention.noticed.len(), 3);
    assert_eq!(default_capacity.attention.noticed[0].gist, "line 2");

    let (blank, dropped) = apply_appraisal(
        &config,
        &calm(),
        &at_rest(),
        &said(
            vec![noticed("warrior", "  "), noticed("", "said")],
            Some("   "),
        ),
    );
    assert_eq!(dropped.noticed, 2);
    assert!(blank.attention.noticed.is_empty());
    assert_eq!(blank.attention.focus, None);
}

#[test]
fn the_mood_is_the_weighted_valence_and_arousal_of_the_levels() {
    let config = EmotionConfig::default();
    let mood = |pairs: &[(&str, u8)]| {
        derive_mood(
            &config,
            &pairs
                .iter()
                .map(|(name, at)| level(name, *at))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(mood(&[]), Mood::default());
    assert_eq!(
        mood(&[("joy", 100)]),
        Mood {
            valence: 80,
            arousal: 60
        }
    );
    assert_eq!(
        mood(&[("joy", 10)]),
        Mood {
            valence: 8,
            arousal: 6
        }
    );
    let mixed = mood(&[("joy", 50), ("irritation", 50)]);
    assert_eq!(mixed.valence, 10);
    assert_eq!(mixed.arousal, 65);
    assert!(mood(&[("fear", 90)]).valence < 0);
    assert_eq!(mood(&[("unknown", 90)]), Mood::default());
}

#[test]
fn a_stored_state_survives_an_added_or_dropped_emotion() {
    let mut config = EmotionConfig::default();
    config.emotions.retain(|emotion| emotion.id != "fear");
    config.emotions.push(EmotionDefinition {
        id: "awe".to_owned(),
        label: "Awe".to_owned(),
        valence: 40,
        arousal: 60,
        tone: "mystic".to_owned(),
        decay_percent: None,
    });
    let mut state = at_rest();
    state.emotions[3].level = 77;
    let temperament = Temperament {
        baseline: vec![level("awe", 15)],
        ..calm()
    };
    let (next, _) = apply_appraisal(&config, &temperament, &state, &Appraisal::default());
    assert_eq!(next.level("fear"), 0);
    assert!(!next.emotions.iter().any(|seen| seen.emotion == "fear"));
    assert_eq!(next.level("awe"), 15);
    assert_eq!(next.emotions.len(), config.emotions.len());
}

#[test]
fn an_override_sets_a_level_and_the_mood_follows_and_an_unknown_emotion_is_refused() {
    let config = EmotionConfig::default();
    let mut state = at_rest();
    assert!(set_emotion(&config, &calm(), &mut state, "fear", 90));
    assert_eq!(state.level("fear"), 90);
    assert!(state.mood.valence < 0);
    assert!(set_emotion(&config, &calm(), &mut state, "fear", 250));
    assert_eq!(state.level("fear"), 100);
    let before = state.clone();
    assert!(!set_emotion(&config, &calm(), &mut state, "rage", 5));
    assert_eq!(state, before);
}

#[test]
fn state_triggers_are_those_whose_numeric_condition_holds_and_never_event_only_ones() {
    let event_only = EmotionalTrigger {
        id: "insult".to_owned(),
        when_state: None,
        when_event: Some("the speaker insults the keeper".to_owned()),
        sets: Vec::new(),
        behavior: None,
    };
    let persona = persona_with(
        calm(),
        vec![
            state_trigger("fed-up", "irritation", 60),
            state_trigger("merry", "joy", 50),
            event_only,
        ],
    );
    let mut state = at_rest();
    assert!(state_triggers(&persona, &state).is_empty());
    state.emotions.iter_mut().for_each(|seen| {
        if seen.emotion == "irritation" {
            seen.level = 60;
        }
    });
    let held: Vec<&str> = state_triggers(&persona, &state)
        .into_iter()
        .map(|trigger| trigger.id.as_str())
        .collect();
    assert_eq!(held, ["fed-up"]);
}

#[test]
fn a_persona_with_a_temperament_round_trips_and_an_old_file_reads_without_one() {
    let temperament = Temperament {
        baseline: vec![level("trust", 40)],
        reactivity: Some(70),
        attention_capacity: Some(4),
        ..calm()
    };
    let persona = persona_with(
        temperament.clone(),
        vec![state_trigger("fed-up", "irritation", 60)],
    );
    let text = serde_json::to_string(&persona).expect("writes");
    let back: Persona = serde_json::from_str(&text).expect("reads");
    assert_eq!(back, persona);
    assert_eq!(back.temperament(), &temperament);
    assert_eq!(back.emotional_triggers().len(), 1);

    let old = r#"{"schema":1,"id":"warrior","display_name":"Brann","job":"Sellsword"}"#;
    let old: Persona = serde_json::from_str(old).expect("reads");
    assert!(old.temperament().is_blank());
    assert!(old.emotional_triggers().is_empty());
    let written = serde_json::to_string(&old).expect("writes");
    assert!(
        !written.contains("temperament") && !written.contains("emotional"),
        "{written}"
    );
}

#[test]
fn the_temperament_is_the_characters_own_and_never_in_the_public_view() {
    let persona = persona_with(
        Temperament {
            reactivity: Some(70),
            ..calm()
        },
        vec![state_trigger("fed-up", "irritation", 60)],
    );
    let public = serde_json::to_string(&persona.public_view()).expect("writes");
    let listener = serde_json::to_string(&persona.brief(BriefScope::Listener)).expect("writes");
    for text in [public, listener] {
        assert!(
            !text.contains("reactivity") && !text.contains("fed-up"),
            "{text}"
        );
    }
    let own = serde_json::to_value(persona.brief(BriefScope::Own)).expect("writes");
    assert_eq!(own["private"]["temperament"]["reactivity"], 70);
    assert_eq!(own["private"]["emotional_triggers"][0]["id"], "fed-up");
}

#[test]
fn a_temperament_or_trigger_out_of_range_or_naming_nothing_real_is_refused() {
    let try_with = |temperament: Temperament, triggers: Vec<EmotionalTrigger>| {
        Persona::new(PersonaDraft {
            id: "innkeeper".to_owned(),
            display_name: "Marta".to_owned(),
            job: "Keeper".to_owned(),
            behaviors: vec![PersonaBehavior {
                id: "curt".to_owned(),
                description: "Short.".to_owned(),
                speech_style: String::new(),
            }],
            temperament,
            emotional_triggers: triggers,
            ..PersonaDraft::default()
        })
    };
    let capacity = |value| Temperament {
        attention_capacity: Some(value),
        ..calm()
    };
    assert!(try_with(capacity(1), Vec::new()).is_ok());
    assert!(try_with(capacity(5), Vec::new()).is_ok());
    for value in [0, 6] {
        assert!(matches!(
            try_with(capacity(value), Vec::new()),
            Err(PersonaError::OutOfRange {
                field: PersonaField::Temperament,
                ..
            })
        ));
    }
    assert!(matches!(
        try_with(
            Temperament {
                warmth: Some(101),
                ..calm()
            },
            Vec::new()
        ),
        Err(PersonaError::OutOfRange { .. })
    ));
    assert!(matches!(
        try_with(
            Temperament {
                baseline: vec![level("joy", 10), level("joy", 20)],
                ..calm()
            },
            Vec::new()
        ),
        Err(PersonaError::DuplicateItemId { .. })
    ));
    assert!(matches!(
        try_with(
            Temperament {
                baseline: vec![level("bad id", 10)],
                ..calm()
            },
            Vec::new()
        ),
        Err(PersonaError::InvalidItemId { .. })
    ));
    let mut stray = state_trigger("fed-up", "irritation", 60);
    stray.behavior = Some("missing".to_owned());
    assert!(matches!(
        try_with(calm(), vec![stray]),
        Err(PersonaError::UnknownBehavior { .. })
    ));
    assert!(matches!(
        try_with(
            calm(),
            vec![state_trigger("a", "joy", 10), state_trigger("a", "joy", 20)]
        ),
        Err(PersonaError::DuplicateItemId { .. })
    ));
    assert!(matches!(
        try_with(calm(), vec![state_trigger("a", "joy", 101)]),
        Err(PersonaError::OutOfRange { .. })
    ));
    let mut wild = state_trigger("a", "joy", 10);
    wild.sets = vec![delta("joy", -128)];
    assert!(matches!(
        try_with(calm(), vec![wild]),
        Err(PersonaError::OutOfRange { .. })
    ));
    let mut multiline = state_trigger("a", "joy", 10);
    multiline.when_event = Some("one\ntwo".to_owned());
    assert!(matches!(
        try_with(calm(), vec![multiline]),
        Err(PersonaError::ControlCharacter { .. })
    ));
}

#[test]
fn typed_edits_change_only_the_temperament_and_its_triggers() {
    let persona = persona_with(calm(), Vec::new());
    let edited = persona
        .edit(&[
            PersonaEdit::SetTemperament {
                field: TemperamentField::Patience,
                value: 20,
            },
            PersonaEdit::SetBaseline {
                emotion: "trust".to_owned(),
                level: 45,
            },
            PersonaEdit::SetBaseline {
                emotion: "joy".to_owned(),
                level: 10,
            },
            PersonaEdit::SetBaseline {
                emotion: "joy".to_owned(),
                level: 0,
            },
            PersonaEdit::PutEmotionalTrigger {
                trigger: state_trigger("fed-up", "irritation", 60),
            },
            PersonaEdit::PutEmotionalTrigger {
                trigger: state_trigger("fed-up", "irritation", 70),
            },
        ])
        .expect("valid");
    assert_eq!(edited.temperament().patience, Some(20));
    assert_eq!(edited.temperament().baseline, vec![level("trust", 45)]);
    let held = &edited.emotional_triggers()[0];
    assert_eq!(held.when_state.as_ref().map(|c| c.at_least), Some(70));
    assert_eq!(edited.emotional_triggers().len(), 1);
    assert_eq!(edited.display_name(), persona.display_name());

    let removed = edited
        .edit(&[PersonaEdit::RemoveEmotionalTrigger {
            id: "fed-up".to_owned(),
        }])
        .expect("valid");
    assert!(removed.emotional_triggers().is_empty());
    assert!(matches!(
        removed.edit(&[PersonaEdit::RemoveEmotionalTrigger {
            id: "fed-up".to_owned()
        }]),
        Err(EditError::NoSuchItem {
            field: PersonaField::EmotionalTriggers,
            ..
        })
    ));
    assert!(matches!(
        persona.edit(&[PersonaEdit::SetTemperament {
            field: TemperamentField::AttentionCapacity,
            value: 9
        }]),
        Err(EditError::Persona {
            cause: PersonaError::OutOfRange { .. }
        })
    ));
    for field in TemperamentField::ALL {
        assert_eq!(TemperamentField::from_key(field.key()), Some(field));
        assert_eq!(
            serde_json::to_value(field).expect("writes"),
            json!(field.key())
        );
    }
}

#[test]
fn emotion_ids_are_checked_against_the_list_as_data() {
    let persona = persona_with(
        Temperament {
            baseline: vec![level("awe", 20)],
            ..calm()
        },
        Vec::new(),
    );
    assert_eq!(
        check_emotion_ids(&persona, &EmotionConfig::default()),
        Err(EditError::UnknownEmotion {
            emotion: "awe".to_owned()
        })
    );
    let mut campaign = EmotionConfig::default();
    campaign.emotions.push(EmotionDefinition {
        id: "awe".to_owned(),
        label: "Awe".to_owned(),
        valence: 40,
        arousal: 60,
        tone: "mystic".to_owned(),
        decay_percent: None,
    });
    assert_eq!(check_emotion_ids(&persona, &campaign), Ok(()));

    let in_trigger = persona_with(calm(), vec![state_trigger("a", "boredom", 10)]);
    assert!(check_emotion_ids(&in_trigger, &EmotionConfig::default()).is_err());
    let mut set_unknown = state_trigger("a", "joy", 10);
    set_unknown.sets = vec![delta("boredom", 10)];
    assert!(
        check_emotion_ids(
            &persona_with(calm(), vec![set_unknown]),
            &EmotionConfig::default()
        )
        .is_err()
    );
}

fn temperament_answer() -> serde_json::Value {
    json!({
        "temperament": {
            "baseline": [{"emotion": "trust", "level": 35}],
            "reactivity": 65, "patience": 30, "warmth": 55, "pride": 70,
            "decay_percent": 25, "attention_capacity": 3, "distractibility": 40
        },
        "emotional_triggers": [
            {"id": "fed-up", "when_state": {"emotion": "irritation", "at_least": 60},
             "when_event": null, "sets": [{"emotion": "trust", "delta": -10}], "behavior": "curt"},
            {"id": "flattered", "when_state": null,
             "when_event": "a customer praises the keeper's work",
             "sets": [{"emotion": "joy", "delta": 15}],
             "behavior": null}
        ]
    })
}

#[test]
fn a_temperament_answer_becomes_one_edit_and_every_id_is_checked() {
    let persona = persona_with(calm(), Vec::new());
    let config = EmotionConfig::default();
    let edit = read_temperament(&temperament_answer(), &persona, &config).expect("reads");
    let edited = persona.edit(&[edit]).expect("valid");
    assert_eq!(edited.temperament().reactivity, Some(65));
    assert_eq!(edited.emotional_triggers().len(), 2);

    let mut unknown_emotion = temperament_answer();
    unknown_emotion["temperament"]["baseline"][0]["emotion"] = json!("rage");
    assert_eq!(
        read_temperament(&unknown_emotion, &persona, &config),
        Err(GenerationError::UnknownEmotion {
            id: "rage".to_owned()
        })
    );
    let mut unknown_behavior = temperament_answer();
    unknown_behavior["emotional_triggers"][0]["behavior"] = json!("furious");
    assert!(matches!(
        read_temperament(&unknown_behavior, &persona, &config),
        Err(GenerationError::Persona {
            cause: PersonaError::UnknownBehavior { .. }
        })
    ));
    let mut stray = temperament_answer();
    stray["mood"] = json!("sour");
    assert!(matches!(
        read_temperament(&stray, &persona, &config),
        Err(GenerationError::Malformed { .. })
    ));
    let mut too_high = temperament_answer();
    too_high["temperament"]["attention_capacity"] = json!(9);
    assert!(matches!(
        read_temperament(&too_high, &persona, &config),
        Err(GenerationError::Persona { .. })
    ));
}

#[test]
fn the_temperament_question_offers_the_list_and_the_behaviors_and_a_strict_schema() {
    let persona = persona_with(calm(), Vec::new());
    let facts = temperament_facts(&persona, &EmotionConfig::default());
    assert_eq!(facts["emotions"][0]["id"], "joy");
    assert_eq!(facts["persona"]["private"]["behaviors"][0]["id"], "curt");
    let schema = temperament_schema(&EmotionConfig::default());
    assert_eq!(schema["additionalProperties"], false);
    let baseline = &schema["properties"]["temperament"]["properties"]["baseline"];
    assert_eq!(baseline["items"]["properties"]["emotion"]["enum"][0], "joy");
    assert_eq!(
        schema["required"],
        json!(["temperament", "emotional_triggers"])
    );
    assert!(TEMPERAMENT_INSTRUCTION.contains("never a list of keywords"));
    let persona_schema = output_schema();
    assert!(persona_schema["properties"]["temperament"].is_object());
    assert!(
        persona_schema["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("emotional_triggers")))
    );
}
