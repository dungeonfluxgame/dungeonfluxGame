//! Tests of the emotional layer's host side: the emotion list as data, the state store, the state
//! and temperament routes and the new `persona` verbs. The text service is a scripted fake.

use df_content::persona::{
    Appraisal, Attention, EmotionDelta, EmotionalTrigger, Noticed, PersonaEdit, PersonaId,
    PersonaState, StateCondition, TemperamentField,
};
use serde_json::{Value, json};

use super::cli::PersonaCommand;
use super::emotion_routes::path;
use super::emotions::{EmotionError, emotions_path, read_emotions};
use super::fixtures::{CHARACTER, ScriptedHost, scripted_host};
use super::parse as parse_persona;
use super::request::{AnswerError, Edited, PersonaError, PersonaReply};
use super::state::{apply_appraisal, load_state, read_state, state_directory, write_state};
use super::state_view::{SetEmotionRequest, StateTarget, StateView, TemperamentRequest};
use super::storage::write_persona;
use crate::command::CommandError;
use crate::dev_client::{get_json, post_json};
use crate::dev_http::{BodyCause, DevReply};

type EmotionReply<T> = DevReply<T, EmotionError>;

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn persona() -> df_content::persona::Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": CHARACTER, "display_name": "Marta Venn", "job": "Keeper of the inn",
        "behaviors": [{"id": "curt", "description": "Short answers.", "speech_style": ""}],
        "temperament": {
            "baseline": [{"emotion": "trust", "level": 30}],
            "reactivity": 50, "attention_capacity": 2
        },
        "emotional_triggers": [
            {"id": "scared", "when_state": {"emotion": "fear", "at_least": 80}, "behavior": "curt"},
            {"id": "insulted", "when_event": "the speaker insults the keeper"}
        ]
    }))
    .unwrap()
}

async fn host_with_persona(answers: Vec<Value>) -> ScriptedHost {
    let host = scripted_host(answers).await;
    write_persona(&host.directory(), &persona()).unwrap();
    host
}

fn appraisal(deltas: &[(&str, i8)]) -> Appraisal {
    Appraisal {
        emotions: deltas
            .iter()
            .map(|(emotion, delta)| EmotionDelta {
                emotion: (*emotion).to_owned(),
                delta: *delta,
            })
            .collect(),
        ..Appraisal::default()
    }
}

fn done<T: std::fmt::Debug>(reply: EmotionReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

fn refused<T: std::fmt::Debug>(reply: EmotionReply<T>) -> EmotionError {
    match reply {
        DevReply::Done { value } => panic!("done: {value:?}"),
        DevReply::Rejected { error, .. } => error,
    }
}

fn lines_with(host: &ScriptedHost, start: &str) -> Vec<String> {
    host.lines_starting_with(start)
}

#[test]
fn the_emotion_file_is_data_a_missing_one_is_the_built_in_list_and_a_bad_one_is_typed() {
    let directory = tempfile::TempDir::new().unwrap();
    let file = directory.path().join("emotions.json");
    assert_eq!(
        read_emotions(&file),
        Ok(df_content::persona::EmotionConfig::default())
    );

    std::fs::write(&file, "{ not json").unwrap();
    let error = read_emotions(&file).unwrap_err();
    assert_eq!(error.key(), "emotions_invalid");
    assert!(matches!(
        error,
        EmotionError::EmotionsInvalid { cause } if cause.cause == BodyCause::Syntax
    ));

    std::fs::write(&file, r#"{"emotions": []}"#).unwrap();
    assert_eq!(read_emotions(&file).unwrap_err().key(), "emotions_rejected");
    std::fs::write(&file, r#"{"decay_percent": 5, "surprise": 1}"#).unwrap();
    assert_eq!(read_emotions(&file).unwrap_err().key(), "emotions_invalid");

    std::fs::write(
        &file,
        r#"{"emotions": [{"id": "awe", "label": "Awe", "valence": 40, "arousal": 60,
            "tone": "mystic"}], "decay_percent": 35, "rapport": {"friendly_at_least": 50}}"#,
    )
    .unwrap();
    let config = read_emotions(&file).unwrap();
    assert_eq!(config.ids(), ["awe"]);
    assert_eq!(config.decay_percent, 35);
    assert_eq!(config.rapport.friendly_at_least, 50);
    assert_eq!(config.rapport.hostile_at_most, -30);
}

#[test]
fn the_shipped_emotion_file_is_the_built_in_list() {
    let shipped: df_content::persona::EmotionConfig = serde_json::from_str(include_str!(
        "../../../../assets/playground/persona/emotions.json"
    ))
    .unwrap();
    assert_eq!(shipped.check(), Ok(()));
    assert_eq!(shipped, df_content::persona::EmotionConfig::default());
}

#[test]
fn a_state_file_round_trips_and_a_wrong_or_broken_one_is_typed() {
    let directory = tempfile::TempDir::new().unwrap();
    assert_eq!(read_state(directory.path(), "innkeeper"), Ok(None));
    let state = PersonaState {
        id: "innkeeper".to_owned(),
        turn: 3,
        attention: Attention {
            focus: Some("warrior".to_owned()),
            topic: None,
            noticed: vec![Noticed {
                speaker: "warrior".to_owned(),
                gist: "asked the price".to_owned(),
            }],
        },
        ..PersonaState::default()
    };
    write_state(directory.path(), &state).unwrap();
    assert_eq!(
        read_state(directory.path(), "innkeeper"),
        Ok(Some(state.clone()))
    );

    let other = PersonaState {
        id: "wizard".to_owned(),
        ..state
    };
    write_state(directory.path(), &other).unwrap();
    std::fs::rename(
        directory.path().join("wizard.json"),
        directory.path().join("innkeeper.json"),
    )
    .unwrap();
    assert_eq!(
        read_state(directory.path(), "innkeeper").unwrap_err().key(),
        "state_id_mismatch"
    );
    std::fs::write(directory.path().join("innkeeper.json"), "{ nope").unwrap();
    assert_eq!(
        read_state(directory.path(), "innkeeper").unwrap_err().key(),
        "state_invalid"
    );
}

#[tokio::test]
async fn an_appraisal_is_kept_per_persona_read_again_and_follows_the_file_without_a_restart() {
    let host = host_with_persona(Vec::new()).await;
    let app = &host.running.state;
    let own = persona();

    let fresh = load_state(app, CHARACTER);
    assert_eq!(
        (fresh.level("trust"), fresh.level("joy"), fresh.turn),
        (30, 0, 0)
    );
    assert!(!state_directory(app).join("innkeeper.json").exists());

    let after = apply_appraisal(app, &own, &appraisal(&[("joy", 40), ("rage", 50)]));
    assert_eq!((after.level("joy"), after.turn), (40, 1));
    assert_eq!(load_state(app, CHARACTER), after);
    assert!(state_directory(app).join("innkeeper.json").exists());
    let applied = lines_with(&host, "PERSONA state_applied");
    assert_eq!(applied.len(), 1);
    assert!(applied[0].contains("dropped_emotions=1"), "{}", applied[0]);

    let faded = apply_appraisal(app, &own, &Appraisal::default());
    assert_eq!(
        faded.level("joy"),
        32,
        "20% of the gap, from the built-in list"
    );

    host.running
        .write_asset(
            "persona/emotions.json",
            concat!(
                r#"{"decay_percent": 100, "emotions": ["#,
                r#"{"id": "joy", "label": "Joy", "valence": 80, "arousal": 60, "tone": "ally", "#,
                r#""decay_percent": 100}, "#,
                r#"{"id": "trust", "label": "Trust", "valence": 60, "arousal": 30, "#,
                r#""tone": "ally"}]}"#
            ),
        );
    let gone = apply_appraisal(app, &own, &Appraisal::default());
    assert_eq!(gone.level("joy"), 0, "the file was read again");
    assert_eq!(gone.turn, 3);

    let attended = apply_appraisal(
        app,
        &own,
        &Appraisal {
            attention: Attention {
                focus: None,
                topic: None,
                noticed: (0..3)
                    .map(|n| Noticed {
                        speaker: "warrior".to_owned(),
                        gist: format!("line {n}"),
                    })
                    .collect(),
            },
            ..Appraisal::default()
        },
    );
    assert_eq!(
        attended.attention.noticed.len(),
        2,
        "the temperament's capacity"
    );
}

#[tokio::test]
async fn an_emotion_file_that_does_not_read_never_blocks_a_line_and_is_recorded() {
    let host = host_with_persona(Vec::new()).await;
    host.running
        .write_asset("persona/emotions.json", "{ broken");
    assert!(emotions_path(&host.running.state).exists());
    let state = apply_appraisal(&host.running.state, &persona(), &appraisal(&[("joy", 10)]));
    assert_eq!(state.level("joy"), 10, "the built-in list stood in");
    let unavailable = lines_with(&host, "PERSONA state_unavailable");
    assert!(
        unavailable
            .iter()
            .any(|line| line == "PERSONA state_unavailable id=innkeeper error=emotions_invalid"),
        "{unavailable:?}"
    );
}

#[tokio::test]
async fn the_state_routes_show_set_and_reset_the_live_state() {
    let host = host_with_persona(Vec::new()).await;
    let port = host.port;
    let shown: EmotionReply<StateView> = get_json(port, &format!("{}?id=innkeeper", path::STATE))
        .await
        .unwrap();
    let view = done(shown);
    assert_eq!(view.state.level("trust"), 30);
    assert!(view.holding.is_empty());

    let set: EmotionReply<StateView> = post_json(
        port,
        path::STATE_SET,
        &SetEmotionRequest {
            id: "innkeeper".to_owned(),
            emotion: "fear".to_owned(),
            level: 90,
        },
    )
    .await
    .unwrap();
    let view = done(set);
    assert_eq!(view.state.level("fear"), 90);
    assert_eq!(
        view.holding,
        ["scared"],
        "only the numeric trigger, never the event one"
    );
    assert!(view.state.mood.valence < 0);

    let unknown: EmotionReply<StateView> = post_json(
        port,
        path::STATE_SET,
        &SetEmotionRequest {
            id: "innkeeper".to_owned(),
            emotion: "rage".to_owned(),
            level: 5,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        refused(unknown),
        EmotionError::UnknownEmotion {
            emotion: "rage".to_owned()
        }
    );
    let missing: EmotionReply<StateView> = post_json(
        port,
        path::STATE_SET,
        &SetEmotionRequest {
            id: "wizard".to_owned(),
            emotion: "fear".to_owned(),
            level: 5,
        },
    )
    .await
    .unwrap();
    assert_eq!(refused(missing).key(), "unknown_persona");

    let reset: EmotionReply<StateView> = post_json(
        port,
        path::STATE_RESET,
        &StateTarget {
            id: "innkeeper".to_owned(),
        },
    )
    .await
    .unwrap();
    let view = done(reset);
    assert_eq!(
        (view.state.level("fear"), view.state.level("trust")),
        (0, 30)
    );
    assert!(
        !state_directory(&host.running.state)
            .join("innkeeper.json")
            .exists()
    );
    assert!(
        lines_with(
            &host,
            "PERSONA refused verb=state-set id=wizard error=unknown_persona"
        )
        .len()
            == 1
    );
}

#[tokio::test]
async fn an_edit_naming_an_emotion_off_the_list_is_refused_and_one_on_it_lands() {
    let host = host_with_persona(Vec::new()).await;
    let edit = |emotion: &str| {
        json!({"id": "innkeeper", "edits": [
            {"op": "set_baseline", "emotion": emotion, "level": 55}
        ]})
    };
    let refused_edit: PersonaReply<Edited> =
        post_json(host.port, super::routes::path::PERSONA_EDIT, &edit("rage"))
            .await
            .unwrap();
    assert!(matches!(
        refused_edit,
        DevReply::Rejected {
            error: PersonaError::InvalidEdit { .. },
            ..
        }
    ));
    let landed: PersonaReply<Edited> =
        post_json(host.port, super::routes::path::PERSONA_EDIT, &edit("joy"))
            .await
            .unwrap();
    let DevReply::Done { value } = landed else {
        panic!("refused")
    };
    assert_eq!(value.persona.temperament().rest_of("joy"), 55);
    assert_eq!(value.persona.temperament().rest_of("trust"), 30);
}

fn temperament_answer(emotion: &str) -> Value {
    json!({
        "temperament": {
            "baseline": [{"emotion": emotion, "level": 40}],
            "reactivity": 70, "patience": 25, "warmth": 60, "pride": 65,
            "decay_percent": 30, "attention_capacity": 3, "distractibility": 20
        },
        "emotional_triggers": [
            {"id": "fed-up", "when_state": {"emotion": "irritation", "at_least": 70},
             "when_event": null, "sets": [{"emotion": "trust", "delta": -10}],
             "behavior": "curt"},
            {"id": "flattered", "when_state": null,
             "when_event": "a customer praises the keeper's work",
             "sets": [{"emotion": "joy", "delta": 15}], "behavior": null}
        ]
    })
}

#[tokio::test]
async fn the_temperament_question_writes_only_the_temperament_and_checks_every_id() {
    let host = host_with_persona(vec![
        temperament_answer("pride"),
        temperament_answer("rage"),
    ])
    .await;
    let request = TemperamentRequest {
        id: "innkeeper".to_owned(),
        model: None,
    };
    let written: EmotionReply<Edited> = post_json(host.port, path::TEMPERAMENT, &request)
        .await
        .unwrap();
    let edited = done(written).persona;
    assert_eq!(edited.temperament().reactivity, Some(70));
    assert_eq!(edited.emotional_triggers().len(), 2);
    assert_eq!(edited.display_name(), "Marta Venn");
    let facts = host.facts_of(0);
    assert_eq!(facts["emotions"][0]["id"], "joy");
    assert_eq!(facts["persona"]["private"]["behaviors"][0]["id"], "curt");
    assert!(
        lines_with(&host, "PERSONA temperament_generated id=innkeeper").len() == 1,
        "{:?}",
        host.persona_lines()
    );

    let stray: EmotionReply<Edited> = post_json(host.port, path::TEMPERAMENT, &request)
        .await
        .unwrap();
    assert!(matches!(
        refused(stray),
        EmotionError::Persona { cause } if matches!(
            *cause,
            PersonaError::AnswerRefused { cause: AnswerError::Generation { .. }, .. }
        )
    ));
}

fn id() -> PersonaId {
    PersonaId::new("innkeeper").unwrap()
}

#[test]
fn the_new_verbs_build_typed_edits_and_commands() {
    let one = |text: &str| parse_persona(&words(text)).unwrap();
    assert_eq!(
        one("temperament innkeeper set reactivity 70"),
        PersonaCommand::Edit {
            id: id(),
            edits: vec![PersonaEdit::SetTemperament {
                field: TemperamentField::Reactivity,
                value: 70
            }]
        }
    );
    assert_eq!(
        one("temperament innkeeper baseline joy 40"),
        PersonaCommand::Edit {
            id: id(),
            edits: vec![PersonaEdit::SetBaseline {
                emotion: "joy".to_owned(),
                level: 40
            }]
        }
    );
    assert_eq!(
        one("temperament innkeeper generate model=gpt-x"),
        PersonaCommand::TemperamentGenerate {
            id: id(),
            model: Some("gpt-x".to_owned())
        }
    );
    assert_eq!(
        one(concat!(
            "emotion innkeeper trigger add fed-up state=irritation:70 set=trust:-10 set=joy:5 ",
            "behavior=curt event=the speaker keeps haggling"
        )),
        PersonaCommand::Edit {
            id: id(),
            edits: vec![PersonaEdit::PutEmotionalTrigger {
                trigger: EmotionalTrigger {
                    id: "fed-up".to_owned(),
                    when_state: Some(StateCondition {
                        emotion: "irritation".to_owned(),
                        at_least: 70
                    }),
                    when_event: Some("the speaker keeps haggling".to_owned()),
                    sets: vec![
                        EmotionDelta {
                            emotion: "trust".to_owned(),
                            delta: -10
                        },
                        EmotionDelta {
                            emotion: "joy".to_owned(),
                            delta: 5
                        }
                    ],
                    behavior: Some("curt".to_owned()),
                }
            }]
        }
    );
    assert!(matches!(
        one("emotion innkeeper trigger add praised a customer praises the work"),
        PersonaCommand::Edit { edits, .. } if matches!(
            edits.as_slice(),
            [PersonaEdit::PutEmotionalTrigger { trigger }]
                if trigger.when_event.as_deref() == Some("a customer praises the work")
                    && trigger.when_state.is_none()
        )
    ));
    assert_eq!(
        one("emotion innkeeper trigger drop fed-up"),
        PersonaCommand::Edit {
            id: id(),
            edits: vec![PersonaEdit::RemoveEmotionalTrigger {
                id: "fed-up".to_owned()
            }]
        }
    );
    assert_eq!(one("state innkeeper"), PersonaCommand::State { id: id() });
    assert_eq!(
        one("state innkeeper reset"),
        PersonaCommand::StateReset { id: id() }
    );
    assert_eq!(
        one("state innkeeper set fear 80"),
        PersonaCommand::StateSet {
            id: id(),
            emotion: "fear".to_owned(),
            level: 80
        }
    );
}

#[test]
fn the_new_verbs_refuse_a_wrong_field_a_level_out_of_range_and_a_trigger_that_never_fires() {
    for text in [
        "temperament innkeeper set mood 70",
        "temperament innkeeper baseline joy 101",
        "emotion innkeeper trigger add fed-up state=irritation:101 event=x",
        "emotion innkeeper trigger add fed-up set=joy:101 event=x",
        "state innkeeper set fear 101",
    ] {
        assert!(
            matches!(
                parse_persona(&words(text)),
                Err(CommandError::BadChoice { .. })
            ),
            "{text}"
        );
    }
    for text in [
        "emotion innkeeper trigger add fed-up behavior=curt",
        "temperament innkeeper",
        "state innkeeper reset now",
    ] {
        assert!(
            matches!(parse_persona(&words(text)), Err(CommandError::Usage(_))),
            "{text}"
        );
    }
}
