use axum::http::StatusCode;
use df_content::persona::{Persona, PersonaEdit, PersonaField, PersonaId};
use serde_json::{Value, json};

use df_content::persona::backfill::{Gap, Latitude, NarrativeGuidance};

use super::cli::{GenerateTarget, PersonaCommand};
use super::options::{Asking, GuidanceArgs};
use super::parse as parse_persona;
use super::request::{
    AnswerError, Audience, CastOutcome, Edited, Generation, LineError, PersonaError, PersonaReply,
    PersonaStored, PersonaSummary,
};
use super::routes::path;
use super::storage::{
    backfills_file_name, list_personas, persona_file_name, read_backfills, read_persona,
    remove_persona, write_persona,
};
use crate::command::{Command, parse};
use crate::dev_client::{get_json, post_json};
use crate::dev_http::{BodyCause, DevReply, RouteError};

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn persona_json(id: &str) -> Value {
    json!({
        "schema": 1,
        "id": id,
        "display_name": "Marta Venn",
        "job": "Keeper of the inn",
        "personality": ["warm but shrewd"],
        "motivations": ["keep the hearth lit"],
        "goals": ["find a guide"],
        "needs": ["coin"],
        "fears": ["the tax collector"],
        "speech_style": "Dry and unhurried.",
        "backstory": "Took over the inn from her father.",
        "private_backstory": "Her brother vanished on the road.",
        "behaviors": [{"id": "guarded", "description": "Short answers.", "speech_style": ""}],
        "default_behavior": "guarded",
        "triggers": [{"id": "guild", "condition": "the speaker mentions the guild",
                      "behavior": "guarded"}]
    })
}

fn persona(id: &str) -> Persona {
    serde_json::from_value(persona_json(id)).unwrap()
}

fn command(text: &str) -> PersonaCommand {
    match parse(&words(text)).unwrap() {
        Command::Persona(command) => command,
        other => panic!("not a persona command: {other:?}"),
    }
}

#[test]
fn the_persona_grammar_reads_every_verb() {
    let id = |text: &str| PersonaId::new(text).unwrap();
    assert_eq!(command("persona"), PersonaCommand::List);
    assert_eq!(command("persona list"), PersonaCommand::List);
    assert_eq!(
        command("persona show elf"),
        PersonaCommand::Show {
            id: id("elf"),
            audience: Audience::Public
        }
    );
    assert_eq!(
        command("persona show elf owner"),
        PersonaCommand::Show {
            id: id("elf"),
            audience: Audience::Owner
        }
    );
    assert_eq!(
        command("persona write elf elf.json"),
        PersonaCommand::Write {
            id: id("elf"),
            path: "elf.json".into()
        }
    );
    assert_eq!(
        command("persona remove elf"),
        PersonaCommand::Remove { id: id("elf") }
    );
    assert_eq!(
        command("persona edit elf edits.json"),
        PersonaCommand::EditFile {
            id: id("elf"),
            path: "edits.json".into()
        }
    );
    assert_eq!(
        command("persona set elf job Scout of the pass"),
        PersonaCommand::Edit {
            id: id("elf"),
            edits: vec![PersonaEdit::SetText {
                field: PersonaField::Job,
                text: "Scout of the pass".to_owned()
            }]
        }
    );
    assert_eq!(
        command("persona add elf fears the dark"),
        PersonaCommand::Edit {
            id: id("elf"),
            edits: vec![PersonaEdit::AddEntry {
                field: PersonaField::Fears,
                text: "the dark".to_owned()
            }]
        }
    );
    assert_eq!(
        command("persona drop elf goals 2"),
        PersonaCommand::Edit {
            id: id("elf"),
            edits: vec![PersonaEdit::RemoveEntry {
                field: PersonaField::Goals,
                index: 2
            }]
        }
    );
    assert_eq!(
        command("persona generate all overwrite=true hint a tired crew"),
        PersonaCommand::Generate {
            target: GenerateTarget::All,
            overwrite: true,
            hint: Some("a tired crew".to_owned()),
            asking: Asking::default()
        }
    );
    assert_eq!(
        command("persona generate elf"),
        PersonaCommand::Generate {
            target: GenerateTarget::Character(id("elf")),
            overwrite: false,
            hint: None,
            asking: Asking::default()
        }
    );
    assert_eq!(
        command("persona say elf Someone asks the price"),
        PersonaCommand::Say {
            character: id("elf"),
            situation: "Someone asks the price".to_owned(),
            model: None,
            speak: false
        }
    );
    assert_eq!(
        command("persona say elf speak=true The price"),
        PersonaCommand::Say {
            character: id("elf"),
            situation: "The price".to_owned(),
            model: None,
            speak: true
        }
    );
}

#[test]
fn the_policy_verbs_build_the_edits_the_edit_route_applies() {
    let PersonaCommand::Edit { edits, .. } = command(
        "persona secret elf add tomas sensitivity=high reveal=proved_not_guild known_by=dwarf \
         He went north",
    ) else {
        panic!("not an edit");
    };
    let [PersonaEdit::PutSecret { secret }] = edits.as_slice() else {
        panic!("{edits:?}");
    };
    assert_eq!(secret.text, "He went north");
    assert_eq!(secret.reveal_conditions, ["proved_not_guild"]);
    assert_eq!(secret.known_by, ["dwarf"]);
    let PersonaCommand::Edit { edits, .. } =
        command("persona behavior elf add guarded default=true style=curt Short answers.")
    else {
        panic!("not an edit");
    };
    assert!(matches!(
        edits.as_slice(),
        [
            PersonaEdit::PutBehavior { .. },
            PersonaEdit::SetDefaultBehavior { id: Some(id) }
        ] if id == "guarded"
    ));
    let PersonaCommand::Edit { edits, .. } = command(
        "persona trigger elf add guild behavior=guarded event=guild_seen the guild comes up",
    ) else {
        panic!("not an edit");
    };
    let [PersonaEdit::PutTrigger { trigger }] = edits.as_slice() else {
        panic!("{edits:?}");
    };
    assert_eq!(trigger.condition, "the guild comes up");
    assert_eq!(trigger.event.as_deref(), Some("guild_seen"));
    assert!(matches!(
        command("persona trigger elf drop guild"),
        PersonaCommand::Edit { .. }
    ));
}

#[test]
fn a_mistaken_persona_command_is_refused_with_its_usage() {
    for bad in [
        "persona list extra",
        "persona show",
        "persona show not/an/id",
        "persona show elf public-ish",
        "persona write elf",
        "persona set elf job",
        "persona set elf id elfy",
        "persona set elf personality quiet",
        "persona add elf job x",
        "persona drop elf goals first",
        "persona secret elf add tomas",
        "persona secret elf add tomas sensitivity=huge text",
        "persona behavior elf add guarded default=maybe text",
        "persona trigger elf add guild the guild",
        "persona generate",
        "persona generate elf overwrite=sometimes",
        "persona generate elf hint",
        "persona say elf",
        "persona frobnicate",
    ] {
        assert!(parse(&words(bad)).is_err(), "{bad} should be refused");
    }
}

#[test]
fn every_persona_usage_is_part_of_the_group_usage() {
    use super::cli::*;
    for usage in [
        PERSONA_SHOW_USAGE,
        PERSONA_WRITE_USAGE,
        PERSONA_SET_USAGE,
        PERSONA_ADD_USAGE,
        PERSONA_DROP_USAGE,
        PERSONA_EDIT_USAGE,
        PERSONA_GENERATE_USAGE,
        PERSONA_SAY_USAGE,
        PERSONA_REMOVE_USAGE,
    ] {
        assert!(PERSONA_USAGE.contains(usage), "{usage}");
    }
    assert!(parse_persona(&words("list")).is_ok());
}

#[test]
fn a_stored_persona_round_trips_and_a_missing_one_is_none() {
    let folder = tempfile::tempdir().unwrap();
    let elf = persona("elf");
    assert_eq!(write_persona(folder.path(), &elf), Ok(false));
    assert_eq!(write_persona(folder.path(), &elf), Ok(true));
    let id = PersonaId::new("elf").unwrap();
    assert_eq!(read_persona(folder.path(), &id), Ok(Some(elf)));
    assert_eq!(
        read_persona(folder.path(), &PersonaId::new("dwarf").unwrap()),
        Ok(None)
    );
    assert_eq!(list_personas(&folder.path().join("none")), Ok(Vec::new()));
    assert_eq!(remove_persona(folder.path(), &id), Ok(true));
    assert_eq!(remove_persona(folder.path(), &id), Ok(false));
}

#[test]
fn an_unreadable_stored_persona_is_a_typed_error_naming_its_id() {
    let folder = tempfile::tempdir().unwrap();
    let id = PersonaId::new("elf").unwrap();
    std::fs::write(folder.path().join(persona_file_name(&id)), "{ not json").unwrap();
    let Err(PersonaError::StoredInvalid { id: named, detail }) = read_persona(folder.path(), &id)
    else {
        panic!("a damaged file is refused");
    };
    assert_eq!(named, "elf");
    assert!(!detail.contains("not json"), "{detail}");
    let other = serde_json::to_string(&persona("dwarf")).unwrap();
    std::fs::write(folder.path().join(persona_file_name(&id)), other).unwrap();
    assert_eq!(
        read_persona(folder.path(), &id),
        Err(PersonaError::StoredIdMismatch {
            id: "elf".to_owned(),
            found: "dwarf".to_owned()
        })
    );
}

/// One number for each variant of `PersonaError`, by an exhaustive match: a variant added without
/// a sample in [`every_persona_error`] fails the test that counts them.
fn variant_number(error: &PersonaError) -> usize {
    match error {
        PersonaError::InvalidBody(_) => 0,
        PersonaError::InvalidQuery { .. } => 1,
        PersonaError::InvalidPersona { .. } => 2,
        PersonaError::ReservedId { .. } => 3,
        PersonaError::InvalidEdit { .. } => 4,
        PersonaError::IdMismatch { .. } => 5,
        PersonaError::UnknownPersona { .. } => 6,
        PersonaError::FolderUnreadable { .. } => 7,
        PersonaError::Unreadable { .. } => 8,
        PersonaError::StoredInvalid { .. } => 9,
        PersonaError::StoredIdMismatch { .. } => 10,
        PersonaError::Unwritable { .. } => 11,
        PersonaError::Shown { .. } => 12,
        PersonaError::TextServiceMissing { .. } => 13,
        PersonaError::TextService { .. } => 14,
        PersonaError::AnswerRefused { .. } => 15,
        PersonaError::InvalidOperationKey => 16,
        PersonaError::InvalidModel { .. } => 17,
        PersonaError::NotFillable { .. } => 18,
        PersonaError::NoGaps => 19,
        PersonaError::PrivateDetailInPublicField => 20,
        PersonaError::BackfillsUnreadable { .. } => 21,
        PersonaError::BackfillsInvalid { .. } => 22,
        PersonaError::BackfillsUnwritable { .. } => 23,
        PersonaError::Stopped => 24,
    }
}

const PERSONA_ERROR_VARIANTS: usize = 25;

fn every_persona_error() -> Vec<PersonaError> {
    use crate::locate::IoCause;
    use crate::shown_scene::ShownSceneError;
    use crate::text::{ModelCause, ServiceReason, TextServiceCause};
    let id = || "innkeeper".to_owned();
    vec![
        PersonaError::InvalidBody(crate::dev_http::parse_body::<Value>(b"{").unwrap_err()),
        PersonaError::InvalidQuery {
            detail: String::new(),
        },
        PersonaError::InvalidPersona {
            cause: df_content::persona::PersonaError::InvalidId { id: id() },
        },
        PersonaError::ReservedId { id: id() },
        PersonaError::InvalidEdit {
            cause: df_content::persona::EditError::NotAList {
                field: PersonaField::Job,
            },
        },
        PersonaError::IdMismatch {
            expected: "a".to_owned(),
            found: "b".to_owned(),
        },
        PersonaError::UnknownPersona { id: id() },
        PersonaError::FolderUnreadable {
            cause: IoCause::Other,
        },
        PersonaError::Unreadable {
            id: id(),
            cause: IoCause::PermissionDenied,
        },
        PersonaError::StoredInvalid {
            id: id(),
            detail: String::new(),
        },
        PersonaError::StoredIdMismatch {
            id: id(),
            found: "b".to_owned(),
        },
        PersonaError::Unwritable {
            id: id(),
            cause: IoCause::StorageFull,
        },
        PersonaError::Shown {
            cause: ShownSceneError::NoBackdrop,
        },
        PersonaError::TextServiceMissing {
            missing: "set the text backend".to_owned(),
        },
        PersonaError::TextService {
            cause: TextServiceCause::NoAnswer,
            reason: Some(ServiceReason::Deadline),
        },
        PersonaError::AnswerRefused {
            replayed: true,
            cause: AnswerError::Line {
                error: LineError::Blank,
            },
        },
        PersonaError::InvalidOperationKey,
        PersonaError::InvalidModel {
            cause: ModelCause::TooManyModels,
        },
        PersonaError::NotFillable {
            field: PersonaField::Job,
        },
        PersonaError::NoGaps,
        PersonaError::PrivateDetailInPublicField,
        PersonaError::BackfillsUnreadable {
            id: id(),
            cause: IoCause::Other,
        },
        PersonaError::BackfillsInvalid {
            id: id(),
            detail: String::new(),
        },
        PersonaError::BackfillsUnwritable {
            id: id(),
            cause: IoCause::StorageFull,
            persona_restored: false,
        },
        PersonaError::Stopped,
    ]
}

#[test]
fn every_persona_error_key_is_its_serde_tag_or_its_causes_and_reads_back() {
    let errors = every_persona_error();
    let mut numbers: Vec<usize> = errors.iter().map(variant_number).collect();
    numbers.sort_unstable();
    assert_eq!(numbers, (0..PERSONA_ERROR_VARIANTS).collect::<Vec<_>>());
    for error in errors {
        let written = serde_json::to_value(&error).unwrap();
        match &error {
            PersonaError::Shown { .. } => {
                assert_eq!(written["cause"]["error"], error.key(), "{written}");
            }
            _ => assert_eq!(written["error"], error.key(), "{written}"),
        }
        assert_eq!(
            serde_json::from_value::<PersonaError>(written).unwrap(),
            error
        );
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn the_private_detail_refusal_says_the_check_is_not_a_guarantee() {
    let text = PersonaError::PrivateDetailInPublicField.to_string();
    assert!(
        text.contains("whole-word") && text.contains("not a guarantee"),
        "{text}"
    );
}

#[test]
fn a_refusal_that_is_the_callers_is_400_and_the_edit_refusal_is_422() {
    assert_eq!(
        PersonaError::ReservedId {
            id: "all".to_owned()
        }
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        PersonaError::InvalidEdit {
            cause: df_content::persona::EditError::NotAList {
                field: PersonaField::Job
            }
        }
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        PersonaError::BackfillsInvalid {
            id: "a".to_owned(),
            detail: String::new()
        }
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

async fn rejected<T: std::fmt::Debug>(reply: PersonaReply<T>) -> PersonaError {
    match reply {
        DevReply::Rejected { error, .. } => error,
        DevReply::Done { value } => panic!("expected a refusal, got {value:?}"),
    }
}

#[tokio::test]
async fn the_persona_routes_store_show_edit_and_remove() {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    let port = running.port;

    let stored: PersonaReply<PersonaStored> =
        post_json(port, path::PERSONA, &persona_json("innkeeper"))
            .await
            .unwrap();
    let DevReply::Done { value } = stored else {
        panic!("a valid persona is stored");
    };
    assert!(!value.replaced);
    let again: PersonaReply<PersonaStored> =
        post_json(port, path::PERSONA, &persona_json("innkeeper"))
            .await
            .unwrap();
    assert!(matches!(again, DevReply::Done { value } if value.replaced));

    let listed: PersonaReply<Vec<PersonaSummary>> = get_json(port, path::PERSONA).await.unwrap();
    let DevReply::Done { value } = listed else {
        panic!("listed");
    };
    assert_eq!(value.len(), 1);
    assert_eq!(value[0].job, "Keeper of the inn");

    let public: PersonaReply<Value> = get_json(port, "/dev/persona/innkeeper").await.unwrap();
    let DevReply::Done { value: public } = public else {
        panic!("public view");
    };
    let text = public.to_string();
    for private in [
        "keep the hearth lit",
        "find a guide",
        "coin",
        "tax collector",
        "brother vanished",
        "guarded",
        "guild",
    ] {
        assert!(!text.contains(private), "{private} leaked: {text}");
    }
    assert_eq!(public["display_name"], "Marta Venn");

    let owner: PersonaReply<Persona> = get_json(port, "/dev/persona/innkeeper?audience=owner")
        .await
        .unwrap();
    assert!(matches!(owner, DevReply::Done { value } if value.private_triggers().len() == 1));

    let edit = |edits: Value| json!({"id": "innkeeper", "edits": edits});
    let edited: PersonaReply<Edited> = post_json(
        port,
        path::PERSONA_EDIT,
        &edit(json!([{"op": "set_text", "field": "job", "text": "Landlady"}])),
    )
    .await
    .unwrap();
    let DevReply::Done { value } = edited else {
        panic!("edited");
    };
    assert!(value.changed);
    assert_eq!(value.persona.job(), "Landlady");
    let same: PersonaReply<Edited> = post_json(
        port,
        path::PERSONA_EDIT,
        &edit(json!([{"op": "set_text", "field": "job", "text": "Landlady"}])),
    )
    .await
    .unwrap();
    assert!(matches!(same, DevReply::Done { value } if !value.changed));

    let refused: PersonaReply<Edited> = post_json(
        port,
        path::PERSONA_EDIT,
        &edit(json!([{"op": "remove_behavior", "id": "guarded"}])),
    )
    .await
    .unwrap();
    let error = rejected(refused).await;
    assert!(
        matches!(error, PersonaError::InvalidEdit { .. }),
        "{error:?}"
    );
    let kept: PersonaReply<Persona> = get_json(port, "/dev/persona/innkeeper?audience=owner")
        .await
        .unwrap();
    assert!(matches!(kept, DevReply::Done { value } if value.private_behaviors().len() == 1));

    let unknown: PersonaReply<Value> = get_json(port, "/dev/persona/nobody").await.unwrap();
    let error = rejected(unknown).await;
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    let bad_query: PersonaReply<Value> = get_json(port, "/dev/persona/innkeeper?audience=all")
        .await
        .unwrap();
    assert!(matches!(
        rejected(bad_query).await,
        PersonaError::InvalidQuery { .. }
    ));
    let bad_id: PersonaReply<Value> = get_json(port, "/dev/persona/not.an.id").await.unwrap();
    assert!(matches!(
        rejected(bad_id).await,
        PersonaError::InvalidPersona { .. }
    ));
    let bad_body: PersonaReply<PersonaStored> =
        post_json(port, path::PERSONA, &json!({"id": "elf"}))
            .await
            .unwrap();
    let PersonaError::InvalidBody(body) = rejected(bad_body).await else {
        panic!("a body that is not a persona is refused");
    };
    assert_eq!(body.cause, BodyCause::Data);
    let missing_edit: PersonaReply<Edited> = post_json(
        port,
        path::PERSONA_EDIT,
        &json!({"id": "nobody", "edits": []}),
    )
    .await
    .unwrap();
    assert!(matches!(
        rejected(missing_edit).await,
        PersonaError::UnknownPersona { .. }
    ));

    let removed: PersonaReply<Value> =
        post_json(port, path::PERSONA_REMOVE, &json!({"id": "innkeeper"}))
            .await
            .unwrap();
    assert!(matches!(removed, DevReply::Done { .. }));
    let gone: PersonaReply<Value> =
        post_json(port, path::PERSONA_REMOVE, &json!({"id": "innkeeper"}))
            .await
            .unwrap();
    assert!(matches!(
        rejected(gone).await,
        PersonaError::UnknownPersona { .. }
    ));

    let lines: Vec<String> = running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("PERSONA"))
        .collect();
    for expected in [
        "PERSONA written id=innkeeper",
        "PERSONA edited id=innkeeper edits=1 changed=true",
        "PERSONA edited id=innkeeper edits=1 changed=false",
        "PERSONA refused verb=edit id=innkeeper error=invalid_edit",
        "PERSONA refused verb=show id=nobody error=unknown_persona",
        "PERSONA removed id=innkeeper",
        "PERSONA refused verb=remove id=innkeeper error=unknown_persona",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "{expected}: {lines:?}"
        );
    }
    assert!(
        lines.iter().all(|line| !line.contains("Marta")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn generating_without_a_shown_scene_is_a_typed_refusal_and_never_a_model_call() {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    let port = running.port;
    let reply: PersonaReply<Generation> =
        post_json(port, path::PERSONA_GENERATE, &json!({"character": "elf"}))
            .await
            .unwrap();
    assert!(matches!(
        rejected(reply).await,
        PersonaError::Shown {
            cause: crate::shown_scene::ShownSceneError::Scene { .. }
        }
    ));
    let reply: PersonaReply<Vec<super::request::CastRow>> =
        post_json(port, path::PERSONA_GENERATE_CAST, &json!({}))
            .await
            .unwrap();
    assert!(matches!(
        rejected(reply).await,
        PersonaError::Shown {
            cause: crate::shown_scene::ShownSceneError::Scene { .. }
        }
    ));
    let reply: PersonaReply<super::request::Spoken> = post_json(
        port,
        path::PERSONA_SAY,
        &json!({"character": "elf", "situation": "x"}),
    )
    .await
    .unwrap();
    assert!(matches!(
        rejected(reply).await,
        PersonaError::Shown {
            cause: crate::shown_scene::ShownSceneError::Scene { .. }
        }
    ));
    let outcome = CastOutcome::from(Err::<Generation, _>(PersonaError::Stopped));
    assert!(matches!(outcome, CastOutcome::Failed { .. }));
}

// TODO(DEV-12) owed: generate-cast on a scene with several characters, one of whose answers is
// malformed (the others still written; needs a layout with several regions).
// TODO(DEV-12) owed: say with a reply that names a behavior, trigger or secret the persona lacks
// (refused as answer_refused, nothing cued).
// TODO(DEV-12) owed: TextRuntime::ask on its own, one test per TextAskError variant (refused,
// no answer, rejected before, in flight, lost, cancelled, not kept).
// TODO(DEV-12) owed: a backfill under strict latitude refuses a new secret, behavior or trigger.

fn args(text: &str) -> Asking {
    match command(&format!("persona coverage elf {text} a question")) {
        PersonaCommand::Coverage { asking, .. } => asking,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_question_commands_read_the_model_and_the_guidance_keys_before_the_free_text() {
    let PersonaCommand::Ask {
        question,
        backfill,
        coverage,
        speak,
        situation,
        asking,
        ..
    } = command(
        "persona ask elf model=gpt-6.1-sol backfill=false situation=rain latitude=strict \
         arc=road hook=a hook=b tone=wry Where did you grow up?",
    )
    else {
        panic!("not an ask");
    };
    assert_eq!(question, "Where did you grow up?");
    assert!(!backfill);
    assert!(
        coverage && !speak,
        "coverage is on and speak is off unless said"
    );
    assert_eq!(situation.as_deref(), Some("rain"));
    assert_eq!(asking.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(asking.guidance.latitude, Some(Latitude::Strict));
    assert_eq!(asking.guidance.hooks, ["a", "b"]);
    assert_eq!(asking.guidance.arc.as_deref(), Some("road"));
    assert_eq!(args("").guidance, GuidanceArgs::default());
    let PersonaCommand::Backfill { gaps, .. } =
        command("persona backfill elf gap=backstory:childhood gap=fears:the:dark")
    else {
        panic!("not a backfill");
    };
    assert_eq!(gaps[1].topic, "the:dark");
    assert!(matches!(
        command("persona backfills elf"),
        PersonaCommand::Backfills { .. }
    ));
    for bad in [
        "persona ask elf",
        "persona ask elf model=a model=b why",
        "persona ask elf latitude=wild why",
        "persona ask elf backfill=maybe why",
        "persona coverage elf",
        "persona backfill elf",
        "persona backfill elf gap=job:x",
        "persona backfill elf gap=backstory",
        "persona backfill elf gap=backstory:x extra words",
        "persona backfills",
    ] {
        assert!(parse(&words(bad)).is_err(), "{bad} should be refused");
    }
}

#[test]
fn guidance_given_as_keys_replaces_the_files_value_and_a_file_alone_is_kept() {
    let file = NarrativeGuidance {
        arc: Some("file arc".to_owned()),
        hooks: vec!["file hook".to_owned()],
        must_avoid: vec!["file avoid".to_owned()],
        latitude: Latitude::Open,
        ..NarrativeGuidance::default()
    };
    assert_eq!(GuidanceArgs::default().merge(None), None);
    assert_eq!(
        GuidanceArgs::default().merge(Some(file.clone())),
        Some(file.clone())
    );
    let merged = args("arc=cli hook=one hook=two latitude=strict")
        .guidance
        .merge(Some(file.clone()))
        .unwrap();
    assert_eq!(merged.arc.as_deref(), Some("cli"));
    assert_eq!(merged.hooks, ["one", "two"]);
    assert_eq!(merged.latitude, Latitude::Strict);
    assert_eq!(merged.must_avoid, ["file avoid"]);
    let keys_only = args("tone=wry").guidance.merge(None).unwrap();
    assert_eq!(keys_only.tone.as_deref(), Some("wry"));
    assert_eq!(keys_only.latitude, Latitude::Guided);
}

fn backfill_record(edits: Vec<PersonaEdit>) -> super::request::BackfillRecord {
    super::request::BackfillRecord {
        sequence: 0,
        at_ms: 5,
        question: Some("where from?".to_owned()),
        gaps: vec![Gap {
            field: PersonaField::Backstory,
            topic: "childhood".to_owned(),
        }],
        edits,
        guidance: None,
        profile: "codex".to_owned(),
    }
}

#[test]
fn a_committed_backfill_only_adds_and_is_recorded_with_a_rising_sequence() {
    use super::storage::commit_backfill;
    let folder = tempfile::tempdir().unwrap();
    let before = persona("innkeeper");
    write_persona(folder.path(), &before).unwrap();
    let id = PersonaId::new("innkeeper").unwrap();
    let add = |text: &str| PersonaEdit::AppendText {
        field: PersonaField::Backstory,
        text: text.to_owned(),
    };
    let (first, one) = commit_backfill(
        folder.path(),
        &id,
        backfill_record(vec![add("Born in a cold spring.")]),
    )
    .unwrap();
    let (second, two) = commit_backfill(
        folder.path(),
        &id,
        backfill_record(vec![PersonaEdit::AddEntry {
            field: PersonaField::Fears,
            text: "fire".to_owned(),
        }]),
    )
    .unwrap();
    assert_eq!((one, two), (1, 2));
    assert!(first.backstory().starts_with(before.backstory()));
    assert!(first.backstory().ends_with("Born in a cold spring."));
    assert!(second.backstory().contains(before.backstory()));
    assert_eq!(second.private_backstory(), before.private_backstory());
    assert_eq!(second.private_secrets(), before.private_secrets());
    assert_eq!(
        second.private_fears().len(),
        before.private_fears().len() + 1
    );
    for kept in before.private_fears() {
        assert!(second.private_fears().contains(kept));
    }
    assert_eq!(read_persona(folder.path(), &id), Ok(Some(second)));
    let records = read_backfills(folder.path(), &id).unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(records[0].question.as_deref(), Some("where from?"));
    assert_eq!(list_personas(folder.path()).unwrap().len(), 1);
}

#[test]
fn a_backfill_record_that_does_not_read_stops_the_backfill_and_is_left_as_it_is() {
    use super::storage::commit_backfill;
    let folder = tempfile::tempdir().unwrap();
    write_persona(folder.path(), &persona("innkeeper")).unwrap();
    let id = PersonaId::new("innkeeper").unwrap();
    let sidecar = folder.path().join(backfills_file_name(&id));
    std::fs::write(&sidecar, "[ not json").unwrap();
    let edits = vec![PersonaEdit::AddEntry {
        field: PersonaField::Fears,
        text: "fire".to_owned(),
    }];
    let refused = commit_backfill(folder.path(), &id, backfill_record(edits)).unwrap_err();
    assert!(
        matches!(refused, PersonaError::BackfillsInvalid { .. }),
        "{refused:?}"
    );
    assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "[ not json");
    let stored = read_persona(folder.path(), &id).unwrap().unwrap();
    assert_eq!(stored, persona("innkeeper"));
}

#[tokio::test]
async fn a_backfill_naming_no_gap_or_a_field_it_cannot_fill_is_refused_before_anything_is_asked() {
    use crate::test_support::{STARTER_FILES, start};
    let running = start(&STARTER_FILES).await;
    let port = running.port;
    let none: PersonaReply<Value> = post_json(
        port,
        path::PERSONA_BACKFILL,
        &json!({"character": "elf", "gaps": []}),
    )
    .await
    .unwrap();
    assert_eq!(rejected(none).await, PersonaError::NoGaps);
    let job: PersonaReply<Value> = post_json(
        port,
        path::PERSONA_BACKFILL,
        &json!({"character": "elf", "gaps": [{"field": "job", "topic": "x"}]}),
    )
    .await
    .unwrap();
    assert_eq!(
        rejected(job).await,
        PersonaError::NotFillable {
            field: PersonaField::Job
        }
    );
    let empty_record: PersonaReply<Value> = get_json(port, "/dev/persona/backfills?id=elf")
        .await
        .unwrap();
    let DevReply::Done { value } = empty_record else {
        panic!("a persona with no backfills has an empty record");
    };
    assert_eq!(value, json!([]));
}

fn guarded_persona() -> Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": "innkeeper", "display_name": "Marta Venn", "job": "Keeper of the inn",
        "personality": ["warm but shrewd"],
        "fears": ["the guild collector"],
        "backstory": "Took over the Ember Roads inn from her father.",
        "private_backstory": "Her brother Tomas vanished going north.",
        "secrets": [{"id": "tomas", "text": "Tomas rode with a guild escort",
                     "sensitivity": "high"}]
    }))
    .unwrap()
}

fn append(text: &str) -> PersonaEdit {
    PersonaEdit::AppendText {
        field: PersonaField::Backstory,
        text: text.to_owned(),
    }
}

#[test]
fn a_backfill_that_copies_a_private_word_into_a_public_text_is_caught() {
    use super::guard::copies_private_detail_into_public_text as copies;
    let persona = guarded_persona();
    assert!(copies(
        &persona,
        &[append("She misses her brother TOMAS, who rode north.")]
    ));
    assert!(copies(
        &persona,
        &[PersonaEdit::AddEntry {
            field: PersonaField::Personality,
            text: "wary of the guild".to_owned()
        }]
    ));
    assert!(copies(&persona, &[append("Her fear is the collector.")]));
}

#[test]
fn a_backfill_with_new_public_detail_or_private_only_additions_passes_the_guard() {
    use super::guard::copies_private_detail_into_public_text as copies;
    let persona = guarded_persona();
    assert!(!copies(
        &persona,
        &[append("She learned to brew ale at nine.")]
    ));
    // A word the public side already uses is public, whatever the private side says.
    assert!(!copies(
        &persona,
        &[append("The Ember Roads were her first road.")]
    ));
    // Short words and stop words are not private detail; a private list is not a public field.
    assert!(!copies(
        &persona,
        &[append("She went with the cart, from the inn.")]
    ));
    assert!(!copies(
        &persona,
        &[PersonaEdit::AddEntry {
            field: PersonaField::Fears,
            text: "Tomas and the guild".to_owned()
        }]
    ));
    // Whole words only: a longer word that contains a private one does not match.
    assert!(!copies(
        &persona,
        &[append("Guildhall songs filled the evenings.")]
    ));
}

#[test]
fn the_seed_personas_and_their_backfill_records_are_valid() {
    let seeds =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/playground/personas");
    let mut personas = 0;
    for entry in std::fs::read_dir(&seeds).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.is_ascii(), "{name} has a character other than ASCII");
        if let Some(id) = name.strip_suffix(".persona.json") {
            let persona: Persona =
                serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(persona.id().as_str(), id, "{name}");
            for trigger in persona.private_triggers() {
                assert!(
                    persona
                        .private_behaviors()
                        .iter()
                        .any(|behavior| behavior.id == trigger.behavior),
                    "{name}: trigger {} names a behavior the persona lacks",
                    trigger.id
                );
            }
            personas += 1;
        } else if name.ends_with(".backfills.json") {
            let records: Vec<super::request::BackfillRecord> =
                serde_json::from_str(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
            let sequences: Vec<u64> = records.iter().map(|record| record.sequence).collect();
            assert_eq!(
                sequences,
                (1..=records.len() as u64).collect::<Vec<_>>(),
                "{name}"
            );
        } else {
            panic!("{name} is not a persona file");
        }
    }
    assert!(personas >= 5, "the seed personas are there");
}

#[tokio::test]
async fn a_damaged_persona_file_leaves_the_cast_listed_without_it_and_is_recorded() {
    use super::fixtures::scripted_host;
    use crate::cast::request::{CastList, CastReply};
    let host = scripted_host(vec![]).await;
    std::fs::create_dir_all(host.directory()).unwrap();
    std::fs::write(
        host.directory().join("innkeeper.persona.json"),
        "{ not json",
    )
    .unwrap();
    let reply: CastReply<CastList> = get_json(host.running.port, "/dev/scene/cast")
        .await
        .unwrap();
    let DevReply::Done { value } = reply else {
        panic!("a damaged persona does not stop the cast");
    };
    assert_eq!(value.members.len(), 1);
    assert!(value.personas.is_empty());
    assert!(
        host.persona_lines()
            .contains(&"PERSONA unreadable id=innkeeper error=stored_invalid".to_owned())
    );
}

#[test]
fn removing_a_persona_keeps_its_backfill_record_and_a_new_persona_continues_the_sequence() {
    use super::storage::commit_backfill_with;
    let folder = tempfile::tempdir().unwrap();
    let id = PersonaId::new("innkeeper").unwrap();
    let record = || super::request::BackfillRecord {
        sequence: 0,
        at_ms: 1,
        question: None,
        gaps: Vec::new(),
        edits: vec![PersonaEdit::AppendText {
            field: PersonaField::Backstory,
            text: "She kept a lantern.".to_owned(),
        }],
        guidance: None,
        profile: "codex".to_owned(),
    };
    let write = |_: &std::path::Path, _: &PersonaId, _: &[super::request::BackfillRecord]| Ok(());
    write_persona(folder.path(), &persona("innkeeper")).unwrap();
    // The writer is the real one for the first commit, so the record file is on disk.
    super::storage::commit_backfill_with(folder.path(), &id, record(), |directory, id, records| {
        let text = serde_json::to_string(records).unwrap();
        std::fs::write(directory.join(backfills_file_name(id)), text).unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(remove_persona(folder.path(), &id), Ok(true));
    assert_eq!(read_backfills(folder.path(), &id).unwrap().len(), 1);
    write_persona(folder.path(), &persona("innkeeper")).unwrap();
    let (_, sequence) = commit_backfill_with(folder.path(), &id, record(), write).unwrap();
    assert_eq!(sequence, 2);
}
