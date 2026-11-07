use serde_json::{Value, json};

use super::backfill::{COVERAGE_INSTRUCTION, coverage_facts};
use super::generation::{GenerationError, GenerationRequest, SceneContext, output_schema};
use super::generation::{facts as generation_facts, read_answer};
use super::voice::{VoiceField, read_voice, voice_facts, voice_schema};
use super::{
    EditError, Persona, PersonaDraft, PersonaEdit, PersonaError, PersonaField, PersonaId, Voice,
};

fn stallkeeper() -> Persona {
    Persona::new(PersonaDraft {
        id: "mara".to_owned(),
        display_name: "Mara Vell".to_owned(),
        job: "Fruit seller".to_owned(),
        private_backstory: "She hides her debts.".to_owned(),
        ..PersonaDraft::default()
    })
    .expect("valid")
}

fn packs() -> Vec<Value> {
    vec![json!({"id": "market-folk", "description": "Stall talk."})]
}

fn voice_answer(packs: Value) -> Value {
    json!({
        "voice": {
            "rhythm": "short, brisk",
            "diction": {"favored": ["ripe"], "never": ["cheap"]},
            "tics": ["Now then"],
            "address": "calls strangers 'friend'",
            "idioms": ["sweet as a peach"],
            "deflection": "talks about the weather",
            "samples": ["Ripe today, friend."]
        },
        "expertise": ["Knows which fruit keeps best on a long road."],
        "voice_packs": packs
    })
}

#[test]
fn a_persona_file_without_a_voice_still_loads_and_writes_none_back() {
    let old = json!({
        "schema": 1, "id": "mara", "display_name": "Mara Vell", "job": "Fruit seller"
    });
    let persona: Persona = serde_json::from_value(old).expect("old files load");
    assert!(persona.voice().is_blank());
    assert!(persona.voice_packs().is_empty());
    assert!(persona.expertise().is_empty());
    let written = serde_json::to_value(&persona).expect("writes");
    for key in ["voice", "voice_packs", "expertise"] {
        assert!(written.get(key).is_none(), "{key}: {written}");
    }
}

#[test]
fn a_voice_survives_a_round_trip_and_the_public_view_carries_voice_and_expertise() {
    let edit = read_voice(&voice_answer(json!(["market-folk"])), &stallkeeper(), &packs())
        .expect("reads");
    let persona = stallkeeper().edit(&[edit]).expect("applies");
    let text = serde_json::to_string(&persona).expect("writes");
    let back: Persona = serde_json::from_str(&text).expect("reads");
    assert_eq!(back, persona);
    assert_eq!(back.voice_packs(), ["market-folk"]);
    let public = serde_json::to_value(persona.public_view()).expect("writes");
    assert_eq!(public["voice"]["rhythm"], "short, brisk");
    assert_eq!(public["voice"]["diction"]["never"][0], "cheap");
    assert_eq!(
        public["expertise"][0],
        "Knows which fruit keeps best on a long road."
    );
    assert!(!public.to_string().contains("debts"));
}

#[test]
fn the_typed_voice_edits_change_one_part_and_refuse_the_wrong_kind_of_part() {
    let set = |field, text: &str| PersonaEdit::SetVoiceText {
        field,
        text: text.to_owned(),
    };
    let persona = stallkeeper()
        .edit(&[
            set(VoiceField::Rhythm, " clipped "),
            PersonaEdit::AddVoiceEntry {
                field: VoiceField::Samples,
                text: "Fresh this morning.".to_owned(),
            },
            PersonaEdit::AddVoiceEntry {
                field: VoiceField::Never,
                text: "cheap".to_owned(),
            },
            PersonaEdit::AddVoicePack {
                pack: "market-folk".to_owned(),
            },
            PersonaEdit::AddEntry {
                field: PersonaField::Expertise,
                text: "Judges ripeness by smell.".to_owned(),
            },
        ])
        .expect("applies");
    assert_eq!(persona.voice().rhythm, "clipped");
    assert_eq!(persona.voice().samples, ["Fresh this morning."]);
    assert_eq!(persona.voice().diction.never, ["cheap"]);
    assert_eq!(persona.voice_packs(), ["market-folk"]);
    assert_eq!(persona.expertise(), ["Judges ripeness by smell."]);

    let dropped = persona
        .edit(&[
            PersonaEdit::RemoveVoiceEntry {
                field: VoiceField::Samples,
                index: 0,
            },
            PersonaEdit::RemoveVoicePack {
                pack: "market-folk".to_owned(),
            },
            PersonaEdit::RemoveEntry {
                field: PersonaField::Expertise,
                index: 0,
            },
        ])
        .expect("applies");
    assert!(dropped.voice().samples.is_empty());
    assert!(dropped.voice_packs().is_empty());
    assert!(dropped.expertise().is_empty());

    assert_eq!(
        persona.edit(&[set(VoiceField::Tics, "x")]),
        Err(EditError::NotAVoiceText {
            field: VoiceField::Tics
        })
    );
    assert_eq!(
        persona.edit(&[PersonaEdit::AddVoiceEntry {
            field: VoiceField::Rhythm,
            text: "x".to_owned()
        }]),
        Err(EditError::NotAVoiceList {
            field: VoiceField::Rhythm
        })
    );
    assert_eq!(
        persona.edit(&[PersonaEdit::RemoveVoiceEntry {
            field: VoiceField::Idioms,
            index: 0
        }]),
        Err(EditError::NoSuchVoiceEntry {
            field: VoiceField::Idioms,
            index: 0
        })
    );
    assert_eq!(
        persona.edit(&[PersonaEdit::AddVoicePack {
            pack: "market-folk".to_owned()
        }]),
        Err(EditError::Persona {
            cause: PersonaError::DuplicateItemId {
                field: PersonaField::VoicePacks,
                id: "market-folk".to_owned()
            }
        })
    );
    assert!(matches!(
        persona.edit(&[PersonaEdit::AddVoicePack {
            pack: "not an id".to_owned()
        }]),
        Err(EditError::Persona {
            cause: PersonaError::InvalidItemId { .. }
        })
    ));
}

#[test]
fn a_voice_answer_replaces_only_the_voice_parts_and_names_only_packs_on_offer() {
    let before = stallkeeper();
    let edit = read_voice(&voice_answer(json!(["market-folk"])), &before, &packs()).expect("reads");
    let after = before.edit(&[edit]).expect("applies");
    assert_eq!(after.job(), before.job());
    assert_eq!(after.private_backstory(), before.private_backstory());
    assert_eq!(after.voice().samples, ["Ripe today, friend."]);

    assert_eq!(
        read_voice(&voice_answer(json!(["harbor-guild"])), &before, &packs()),
        Err(GenerationError::UnknownVoicePack {
            id: "harbor-guild".to_owned()
        })
    );
    assert_eq!(
        read_voice(&voice_answer(json!(["market-folk"])), &before, &[]),
        Err(GenerationError::UnknownVoicePack {
            id: "market-folk".to_owned()
        })
    );
    let mut no_samples = voice_answer(json!([]));
    no_samples["voice"]["samples"] = json!([" "]);
    assert!(matches!(
        read_voice(&no_samples, &before, &packs()),
        Err(GenerationError::Persona { .. })
    ));
}

#[test]
fn the_voice_question_shows_a_listener_the_persona_and_the_packs_and_is_strict() {
    let facts = voice_facts(&stallkeeper(), &packs());
    assert_eq!(facts["available_packs"][0]["id"], "market-folk");
    assert!(!facts.to_string().contains("debts"), "{facts}");
    let schema = voice_schema();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["required"],
        json!(["voice", "expertise", "voice_packs"])
    );
}

fn generation_request(available_packs: Vec<Value>) -> GenerationRequest {
    GenerationRequest {
        character: PersonaId::new("mara").unwrap(),
        hint: None,
        scene: SceneContext {
            title: "Market".to_owned(),
            description: "Stalls.".to_owned(),
        },
        cast: Vec::new(),
        guidance: None,
        available_packs,
    }
}

#[test]
fn a_generated_persona_carries_voice_expertise_and_packs_checked_against_the_list() {
    let mut answer = json!({
        "display_name": "Mara Vell", "job": "Fruit seller", "personality": [],
        "motivations": [], "goals": [], "needs": [], "fears": [],
        "speech_style": "Brisk.", "backstory": "", "private_backstory": ""
    });
    let voice = voice_answer(json!(["market-folk"]));
    for key in ["voice", "expertise", "voice_packs"] {
        answer[key] = voice[key].clone();
    }
    let request = generation_request(packs());
    let persona = read_answer(&answer, &request).expect("reads");
    assert_eq!(persona.voice().tics, ["Now then"]);
    assert_eq!(persona.voice_packs(), ["market-folk"]);
    assert_eq!(generation_facts(&request)["available_packs"][0]["id"], "market-folk");
    assert_eq!(
        read_answer(&answer, &generation_request(Vec::new())),
        Err(GenerationError::UnknownVoicePack {
            id: "market-folk".to_owned()
        })
    );
    let schema = output_schema();
    for key in ["voice", "expertise", "voice_packs"] {
        assert!(schema["properties"].get(key).is_some(), "{key}");
        assert!(schema["required"].as_array().unwrap().contains(&json!(key)));
    }
}

#[test]
fn coverage_counts_expertise_and_world_facts_as_held_knowledge() {
    let persona = stallkeeper()
        .edit(&[PersonaEdit::AddEntry {
            field: PersonaField::Expertise,
            text: "Knows which fruit keeps best.".to_owned(),
        }])
        .expect("applies");
    let facts = coverage_facts(&persona, "What keeps best?", None);
    assert_eq!(
        facts["persona"]["expertise"][0],
        "Knows which fruit keeps best."
    );
    assert!(COVERAGE_INSTRUCTION.contains("expertise"));
    assert!(COVERAGE_INSTRUCTION.contains("world"));
}

#[test]
fn a_blank_voice_is_blank_and_one_with_a_part_is_not() {
    assert!(Voice::default().is_blank());
    let voice = Voice {
        address: "friend".to_owned(),
        ..Voice::default()
    };
    assert!(!voice.is_blank());
}
