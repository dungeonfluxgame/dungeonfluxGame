use crate::persona::*;

fn persona() -> Persona {
    Persona::new(PersonaDraft {
        id: "innkeeper".to_owned(),
        display_name: "Marta Venn".to_owned(),
        job: "Keeper of the inn".to_owned(),
        personality: vec!["warm".to_owned(), "shrewd".to_owned()],
        behaviors: vec![PersonaBehavior {
            id: "guarded".to_owned(),
            description: "Short answers.".to_owned(),
            speech_style: String::new(),
        }],
        triggers: vec![PersonaTrigger {
            id: "guild".to_owned(),
            condition: "the speaker mentions the guild".to_owned(),
            event: None,
            behavior: "guarded".to_owned(),
        }],
        ..PersonaDraft::default()
    })
    .expect("valid")
}

fn secret(id: &str) -> PersonaSecret {
    PersonaSecret {
        id: id.to_owned(),
        text: "A secret.".to_owned(),
        sensitivity: Sensitivity::Low,
        reveal_conditions: Vec::new(),
        known_by: Vec::new(),
    }
}

#[test]
fn edits_are_applied_in_order_and_give_a_new_valid_persona() {
    let edited = persona()
        .edit(&[
            PersonaEdit::SetText {
                field: PersonaField::Job,
                text: "Keeper of the inn and stables".to_owned(),
            },
            PersonaEdit::AddEntry {
                field: PersonaField::Fears,
                text: "winter".to_owned(),
            },
            PersonaEdit::SetEntry {
                field: PersonaField::Personality,
                index: 0,
                text: "warm but wary".to_owned(),
            },
            PersonaEdit::RemoveEntry {
                field: PersonaField::Personality,
                index: 1,
            },
            PersonaEdit::PutSecret {
                secret: secret("tomas"),
            },
            PersonaEdit::SetDefaultBehavior {
                id: Some("guarded".to_owned()),
            },
        ])
        .expect("applies");
    assert_eq!(edited.job(), "Keeper of the inn and stables");
    assert_eq!(edited.private_fears(), ["winter"]);
    assert_eq!(edited.personality(), ["warm but wary"]);
    assert_eq!(edited.private_secrets().len(), 1);
    assert_eq!(edited.private_default_behavior(), Some("guarded"));
}

#[test]
fn a_put_replaces_the_item_with_the_same_id() {
    let once = persona()
        .edit(&[PersonaEdit::PutSecret {
            secret: secret("tomas"),
        }])
        .expect("applies");
    let mut changed = secret("tomas");
    changed.text = "A different secret.".to_owned();
    let twice = once
        .edit(&[PersonaEdit::PutSecret { secret: changed }])
        .expect("applies");
    assert_eq!(twice.private_secrets().len(), 1);
    assert_eq!(twice.private_secrets()[0].text, "A different secret.");
}

#[test]
fn a_refused_edit_changes_nothing_and_the_error_says_why() {
    let before = persona();
    // Removing the behavior a trigger names leaves an invalid persona.
    let error = before
        .edit(&[PersonaEdit::RemoveBehavior {
            id: "guarded".to_owned(),
        }])
        .expect_err("refused");
    assert_eq!(
        error,
        EditError::Persona {
            cause: PersonaError::UnknownBehavior {
                behavior: "guarded".to_owned(),
                used_by: BehaviorUse::Trigger {
                    id: "guild".to_owned()
                }
            }
        }
    );
    // Removing the trigger first, in the same list, is fine.
    let both = before
        .edit(&[
            PersonaEdit::RemoveTrigger {
                id: "guild".to_owned(),
            },
            PersonaEdit::RemoveBehavior {
                id: "guarded".to_owned(),
            },
        ])
        .expect("applies");
    assert!(both.private_behaviors().is_empty());
    assert_eq!(before.private_behaviors().len(), 1);
}

#[test]
fn an_edit_of_the_wrong_kind_of_field_or_a_missing_item_is_refused() {
    let persona = persona();
    assert_eq!(
        persona.edit(&[PersonaEdit::SetText {
            field: PersonaField::Id,
            text: "someone-else".to_owned()
        }]),
        Err(EditError::NotAText {
            field: PersonaField::Id
        })
    );
    assert_eq!(
        persona.edit(&[PersonaEdit::AddEntry {
            field: PersonaField::Job,
            text: "x".to_owned()
        }]),
        Err(EditError::NotAList {
            field: PersonaField::Job
        })
    );
    assert_eq!(
        persona.edit(&[PersonaEdit::RemoveEntry {
            field: PersonaField::Personality,
            index: 9
        }]),
        Err(EditError::NoSuchEntry {
            field: PersonaField::Personality,
            index: 9
        })
    );
    assert_eq!(
        persona.edit(&[PersonaEdit::RemoveSecret {
            id: "nope".to_owned()
        }]),
        Err(EditError::NoSuchItem {
            field: PersonaField::Secrets,
            id: "nope".to_owned()
        })
    );
}

#[test]
fn edits_read_from_json_and_refuse_an_unknown_op() {
    let text = r#"{"op":"put_secret","secret":{"id":"tomas","text":"x","sensitivity":"high"}}"#;
    let edit: PersonaEdit = serde_json::from_str(text).expect("reads");
    assert!(matches!(edit, PersonaEdit::PutSecret { .. }));
    let set: PersonaEdit =
        serde_json::from_str(r#"{"op":"set_text","field":"job","text":"Cook"}"#).expect("reads");
    assert_eq!(persona().edit(&[set]).expect("applies").job(), "Cook");
    assert!(serde_json::from_str::<PersonaEdit>(r#"{"op":"burn_it_down"}"#).is_err());
    assert!(
        serde_json::from_str::<PersonaEdit>(r#"{"op":"remove_secret","id":"a","extra":1}"#)
            .is_err()
    );
}

#[test]
fn to_draft_and_new_give_back_the_same_persona() {
    let persona = persona();
    assert_eq!(Persona::new(persona.to_draft()).expect("valid"), persona);
}

#[test]
fn appended_and_added_text_is_stored_without_the_blanks_round_it() {
    let edited = persona()
        .edit(&[
            PersonaEdit::AppendText {
                field: PersonaField::Backstory,
                text: " Grew up above the taproom. ".to_owned(),
            },
            PersonaEdit::AppendText {
                field: PersonaField::Backstory,
                text: "
Left at sixteen."
                    .to_owned(),
            },
            PersonaEdit::AddEntry {
                field: PersonaField::Fears,
                text: "  winter ".to_owned(),
            },
        ])
        .expect("applies");
    assert_eq!(
        edited.backstory(),
        "Grew up above the taproom.
Left at sixteen."
    );
    assert_eq!(edited.private_fears(), ["winter"]);
}
