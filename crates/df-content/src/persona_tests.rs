use crate::persona::*;

fn draft() -> PersonaDraft {
    PersonaDraft {
        id: "innkeeper".to_owned(),
        display_name: "Marta Venn".to_owned(),
        job: "Keeper of the Ember Roads inn".to_owned(),
        personality: vec![
            "warm but shrewd".to_owned(),
            "gossips for a price".to_owned(),
        ],
        motivations: vec!["keep the inn out of debt".to_owned()],
        goals: vec!["find a guide for her missing brother".to_owned()],
        needs: vec!["coin before winter".to_owned()],
        fears: vec!["the tax collector".to_owned()],
        speech_style: "Dry and unhurried; calls strangers 'love'.".to_owned(),
        backstory: "Took over the inn from her father twelve years ago.".to_owned(),
        private_backstory: "Her brother vanished on the Ember Roads and she blames the guild."
            .to_owned(),
        ..PersonaDraft::default()
    }
}

#[test]
fn a_valid_draft_makes_a_persona_that_reads_back_what_it_was_given() {
    let persona = Persona::new(draft()).expect("valid");
    assert_eq!(persona.id().as_str(), "innkeeper");
    assert_eq!(persona.display_name(), "Marta Venn");
    assert_eq!(persona.job(), "Keeper of the Ember Roads inn");
    assert_eq!(persona.personality().len(), 2);
    assert_eq!(persona.private_fears(), ["the tax collector"]);
}

#[test]
fn the_public_view_leaves_out_the_drives_and_the_private_backstory() {
    let persona = Persona::new(draft()).expect("valid");
    let public = serde_json::to_value(persona.public_view()).expect("writes");
    let text = public.to_string();
    for private in [
        "private_backstory",
        "motivations",
        "goals",
        "needs",
        "fears",
        "her brother vanished",
        "tax collector",
    ] {
        assert!(!text.contains(private), "{private} leaked: {text}");
    }
    assert_eq!(public["job"], "Keeper of the Ember Roads inn");
    assert_eq!(public["backstory"], persona.backstory());
}

#[test]
fn a_persona_writes_and_reads_as_json_and_the_file_says_its_schema() {
    let persona = Persona::new(draft()).expect("valid");
    let text = serde_json::to_string(&persona).expect("writes");
    assert!(text.contains("\"schema\":1"), "{text}");
    let back: Persona = serde_json::from_str(&text).expect("reads");
    assert_eq!(back, persona);
}

#[test]
fn optional_parts_may_be_left_out_of_a_file() {
    let text = r#"{"schema":1,"id":"warrior","display_name":"Brann","job":"Sellsword"}"#;
    let persona: Persona = serde_json::from_str(text).expect("reads");
    assert!(persona.personality().is_empty());
    assert_eq!(persona.backstory(), "");
}

#[test]
fn a_file_in_another_schema_or_with_an_unknown_key_is_refused() {
    let other = r#"{"schema":2,"id":"a","display_name":"A","job":"B"}"#;
    let error = serde_json::from_str::<Persona>(other).expect_err("refused");
    assert!(error.to_string().contains("schema 2"), "{error}");
    let unknown = r#"{"schema":1,"id":"a","display_name":"A","job":"B","mood":"grim"}"#;
    assert!(serde_json::from_str::<Persona>(unknown).is_err());
}

#[test]
fn an_id_is_a_layout_style_name() {
    for good in ["innkeeper", "braided_woman", "a-1", "A"] {
        assert!(PersonaId::new(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "has space",
        "dot.dot",
        "slash/slash",
        "na\u{ef}ve",
        "../up",
    ] {
        assert!(
            matches!(PersonaId::new(bad), Err(PersonaError::InvalidId { .. })),
            "{bad}"
        );
    }
    assert!(PersonaId::new(&"a".repeat(MAXIMUM_ID_CHARS)).is_ok());
    assert!(PersonaId::new(&"a".repeat(MAXIMUM_ID_CHARS + 1)).is_err());
}

#[test]
fn a_blank_name_job_or_entry_is_refused_and_names_the_part() {
    let mut blank_job = draft();
    blank_job.job = "   ".to_owned();
    assert_eq!(
        Persona::new(blank_job),
        Err(PersonaError::Blank {
            field: PersonaField::Job
        })
    );
    let mut blank_trait = draft();
    blank_trait.personality.push(String::new());
    assert_eq!(
        Persona::new(blank_trait),
        Err(PersonaError::BlankEntry {
            field: PersonaField::Personality,
            index: 2
        })
    );
}

#[test]
fn text_and_lists_are_bounded_with_generous_named_limits() {
    let mut long = draft();
    long.backstory = "x".repeat(MAXIMUM_BACKSTORY_CHARS);
    assert!(Persona::new(long.clone()).is_ok());
    long.backstory.push('x');
    assert!(matches!(
        Persona::new(long),
        Err(PersonaError::TooLong {
            field: PersonaField::Backstory,
            ..
        })
    ));
    let mut many = draft();
    many.fears = vec!["x".to_owned(); MAXIMUM_LIST_ENTRIES + 1];
    assert!(matches!(
        Persona::new(many),
        Err(PersonaError::TooMany {
            field: PersonaField::Fears,
            ..
        })
    ));
}

#[test]
fn a_control_character_other_than_a_newline_or_tab_is_refused() {
    let mut multiline = draft();
    multiline.backstory = "First line\n\tSecond line".to_owned();
    assert!(Persona::new(multiline).is_ok());
    let mut bell = draft();
    bell.speech_style = "rings a bell\u{7}".to_owned();
    assert_eq!(
        Persona::new(bell),
        Err(PersonaError::ControlCharacter {
            field: PersonaField::SpeechStyle
        })
    );
}

#[test]
fn every_field_key_is_its_serde_name_and_reads_back() {
    for field in PersonaField::ALL {
        let wire = serde_json::to_value(field).expect("writes");
        assert_eq!(wire, serde_json::Value::from(field.key()));
        assert_eq!(PersonaField::from_key(field.key()), Some(field));
    }
    assert_eq!(PersonaField::from_key("mood"), None);
}

#[test]
fn a_listener_brief_is_the_public_persona_and_an_own_brief_adds_the_private_side() {
    let persona = Persona::new(draft()).expect("valid");
    let listener = serde_json::to_value(persona.brief(BriefScope::Listener)).expect("writes");
    assert!(listener.get("private").is_none(), "{listener}");
    assert_eq!(listener["job"], "Keeper of the Ember Roads inn");
    assert!(!listener.to_string().contains("tax collector"));
    let own = serde_json::to_value(persona.brief(BriefScope::Own)).expect("writes");
    assert_eq!(own["job"], listener["job"]);
    assert_eq!(own["private"]["fears"][0], "the tax collector");
    assert!(
        own["private"]["backstory"]
            .as_str()
            .is_some_and(|text| text.contains("brother vanished"))
    );
}

#[test]
fn the_id_grammar_and_bound_are_the_scene_layout_labels_so_a_persona_links_by_equal_text() {
    assert_eq!(MAXIMUM_ID_CHARS, df_scene_layout::MAXIMUM_NAME_CHARS);
    let long = "a".repeat(MAXIMUM_ID_CHARS + 1);
    for sample in [
        "innkeeper",
        "braided_woman",
        "a-1",
        "A",
        "",
        "has space",
        "dot.dot",
        "slash/slash",
        "na\u{ef}ve",
        "../up",
        "tab\there",
        "new\nline",
        long.as_str(),
    ] {
        assert_eq!(
            PersonaId::new(sample).is_ok(),
            df_scene_layout::is_character_name(sample),
            "{sample:?}"
        );
    }
}

#[test]
fn the_seed_persona_file_reads() {
    let text = include_str!("../../../assets/playground/personas/innkeeper.persona.json");
    let persona: Persona = serde_json::from_str(text).expect("the seed innkeeper reads");
    assert_eq!(persona.id().as_str(), "innkeeper");
}

#[test]
fn a_name_job_or_trait_cannot_hold_a_line_break_but_a_backstory_can() {
    let with = |change: fn(&mut PersonaDraft)| {
        let mut changed = draft();
        change(&mut changed);
        changed
    };
    for (field, changed) in [
        (
            PersonaField::DisplayName,
            with(|draft| draft.display_name = "A\nB".to_owned()),
        ),
        (
            PersonaField::Job,
            with(|draft| draft.job = "A\tB".to_owned()),
        ),
        (
            PersonaField::SpeechStyle,
            with(|draft| draft.speech_style = "a\nb".to_owned()),
        ),
        (
            PersonaField::Personality,
            with(|draft| draft.personality = vec!["a\nb".to_owned()]),
        ),
    ] {
        assert_eq!(
            Persona::new(changed),
            Err(PersonaError::ControlCharacter { field }),
            "{field:?}"
        );
    }
    let multiline = with(|draft| draft.backstory = "one\ntwo".to_owned());
    assert!(Persona::new(multiline).is_ok());
}

#[test]
fn debug_output_names_the_public_identity_and_never_a_secret() {
    let mut with_private = draft();
    with_private.private_backstory = "Her brother vanished.".to_owned();
    with_private.fears = vec!["the tax collector".to_owned()];
    let persona = Persona::new(with_private).expect("valid");
    let shown = format!("{persona:?} {:?}", persona.brief(BriefScope::Own));
    assert!(shown.contains("Marta Venn"), "{shown}");
    for private in ["brother vanished", "tax collector"] {
        assert!(!shown.contains(private), "{private} leaked: {shown}");
    }
}
