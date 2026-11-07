use crate::persona::*;

fn behavior(id: &str) -> PersonaBehavior {
    PersonaBehavior {
        id: id.to_owned(),
        description: "Short answers, watches hands.".to_owned(),
        speech_style: String::new(),
    }
}

fn secret(id: &str) -> PersonaSecret {
    PersonaSecret {
        id: id.to_owned(),
        text: "Her brother was last seen going north with a guild escort.".to_owned(),
        sensitivity: Sensitivity::High,
        reveal_conditions: vec!["the speaker has proved they are not guild".to_owned()],
        known_by: vec!["wizard".to_owned()],
    }
}

fn trigger(id: &str, behavior: &str) -> PersonaTrigger {
    PersonaTrigger {
        id: id.to_owned(),
        condition: "the speaker mentions the guild".to_owned(),
        event: Some("insulted".to_owned()),
        behavior: behavior.to_owned(),
    }
}

fn draft() -> PersonaDraft {
    PersonaDraft {
        id: "innkeeper".to_owned(),
        display_name: "Marta Venn".to_owned(),
        job: "Keeper of the inn".to_owned(),
        boundaries: vec!["never sells a guest's name".to_owned()],
        secrets: vec![secret("tomas")],
        behaviors: vec![behavior("guarded"), behavior("haggling")],
        default_behavior: Some("guarded".to_owned()),
        triggers: vec![trigger("guild-mention", "guarded")],
        ..PersonaDraft::default()
    }
}

#[test]
fn a_persona_with_secrets_behaviors_and_triggers_round_trips_as_json() {
    let persona = Persona::new(draft()).expect("valid");
    let text = serde_json::to_string(&persona).expect("writes");
    let back: Persona = serde_json::from_str(&text).expect("reads");
    assert_eq!(back, persona);
    assert_eq!(back.private_secrets()[0].sensitivity, Sensitivity::High);
    assert_eq!(back.private_default_behavior(), Some("guarded"));
}

#[test]
fn a_file_without_the_new_parts_still_reads_and_writes_without_them() {
    let old = r#"{"schema":1,"id":"warrior","display_name":"Brann","job":"Sellsword"}"#;
    let persona: Persona = serde_json::from_str(old).expect("reads");
    assert!(persona.private_secrets().is_empty());
    let written = serde_json::to_string(&persona).expect("writes");
    assert!(!written.contains("secrets"), "{written}");
    assert!(!written.contains("triggers"), "{written}");
}

#[test]
fn a_listener_brief_never_carries_secrets_behaviors_or_triggers() {
    let persona = Persona::new(draft()).expect("valid");
    let listener = serde_json::to_string(&persona.brief(BriefScope::Listener)).expect("writes");
    for private in [
        "guild escort",
        "guarded",
        "guild-mention",
        "never sells",
        "tomas",
    ] {
        assert!(!listener.contains(private), "{private} leaked: {listener}");
    }
    let own = serde_json::to_value(persona.brief(BriefScope::Own)).expect("writes");
    assert_eq!(own["private"]["secrets"][0]["id"], "tomas");
    assert_eq!(own["private"]["default_behavior"], "guarded");
    assert_eq!(own["private"]["triggers"][0]["behavior"], "guarded");
}

#[test]
fn a_trigger_or_default_naming_a_missing_behavior_is_refused() {
    let mut bad_trigger = draft();
    bad_trigger.triggers = vec![trigger("t", "frightened")];
    assert_eq!(
        Persona::new(bad_trigger),
        Err(PersonaError::UnknownBehavior {
            behavior: "frightened".to_owned(),
            used_by: BehaviorUse::Trigger { id: "t".to_owned() }
        })
    );
    let mut bad_default = draft();
    bad_default.default_behavior = Some("frightened".to_owned());
    assert!(matches!(
        Persona::new(bad_default),
        Err(PersonaError::UnknownBehavior {
            used_by: BehaviorUse::Default,
            ..
        })
    ));
}

#[test]
fn the_refusal_says_what_names_the_missing_behavior_and_what_to_do() {
    let mut bad_default = draft();
    bad_default.default_behavior = Some("frightened".to_owned());
    let default_error = Persona::new(bad_default).unwrap_err();
    let sentence = default_error.to_string();
    assert!(sentence.contains("default behavior"), "{sentence}");
    assert!(sentence.contains("set another default first"), "{sentence}");
    let mut bad_trigger = draft();
    bad_trigger.triggers = vec![trigger("t", "frightened")];
    let sentence = Persona::new(bad_trigger).unwrap_err().to_string();
    assert!(sentence.contains("trigger \"t\""), "{sentence}");
    assert!(
        sentence.contains("change or remove the trigger"),
        "{sentence}"
    );
    let wire = serde_json::to_value(&default_error).expect("writes");
    assert_eq!(wire["error"], "unknown_behavior");
    assert_eq!(wire["used_by"]["by"], "default");
}

#[test]
fn item_ids_must_be_labels_and_unique() {
    let mut spaced = draft();
    spaced.secrets = vec![secret("has space")];
    assert!(matches!(
        Persona::new(spaced),
        Err(PersonaError::InvalidItemId {
            field: PersonaField::Secrets,
            ..
        })
    ));
    let mut twice = draft();
    twice.behaviors = vec![behavior("guarded"), behavior("guarded")];
    assert!(matches!(
        Persona::new(twice),
        Err(PersonaError::DuplicateItemId {
            field: PersonaField::Behaviors,
            ..
        })
    ));
}

#[test]
fn blank_secret_text_conditions_and_known_by_ids_are_refused() {
    let mut blank = draft();
    blank.secrets[0].text = " ".to_owned();
    assert!(matches!(
        Persona::new(blank),
        Err(PersonaError::Blank {
            field: PersonaField::Secrets
        })
    ));
    let mut blank_condition = draft();
    blank_condition.triggers[0].condition = String::new();
    assert!(matches!(
        Persona::new(blank_condition),
        Err(PersonaError::Blank {
            field: PersonaField::Triggers
        })
    ));
    let mut bad_known_by = draft();
    bad_known_by.secrets[0].known_by = vec!["not a name".to_owned()];
    assert!(matches!(
        Persona::new(bad_known_by),
        Err(PersonaError::InvalidItemId {
            field: PersonaField::Secrets,
            ..
        })
    ));
    // TODO(DEV-12): cover the bounds of each list (1,000 entries) and a control character in a
    // secret's text.
}

#[test]
fn sensitivity_keys_are_the_serde_names() {
    for level in Sensitivity::ALL {
        let wire = serde_json::to_value(level).expect("writes");
        assert_eq!(wire, serde_json::Value::from(level.key()));
        assert_eq!(Sensitivity::from_key(level.key()), Some(level));
    }
}

#[test]
fn known_by_is_bounded() {
    let mut many = draft();
    many.secrets[0].known_by = vec!["wizard".to_owned(); MAXIMUM_LIST_ENTRIES + 1];
    assert!(matches!(
        Persona::new(many),
        Err(PersonaError::TooMany {
            field: PersonaField::Secrets,
            ..
        })
    ));
}

#[test]
fn a_secret_text_or_condition_cannot_hold_a_line_break() {
    let mut text = draft();
    text.secrets[0].text = "one\ntwo".to_owned();
    assert!(matches!(
        Persona::new(text),
        Err(PersonaError::ControlCharacter {
            field: PersonaField::Secrets
        })
    ));
    let mut condition = draft();
    condition.triggers[0].condition = "a\nb".to_owned();
    assert!(matches!(
        Persona::new(condition),
        Err(PersonaError::ControlCharacter {
            field: PersonaField::Triggers
        })
    ));
}
