use df_content::persona::{Persona, PersonaEdit, PersonaField, PersonaId};
use serde_json::{Value, json};

use std::sync::Arc;

use super::fixtures::{
    CHARACTER, ScriptedHost, innkeeper, leaking_line_answer, line_answer, scripted_host,
    scripted_host_cast,
};
use super::generate::operation_key;
use super::request::{
    BackfillRecord, CastOutcome, Generation, PersonaError, PersonaReply, PersonaStored, Spoken,
};
use super::routes::path;
use super::storage::{
    RESERVED_IDS, commit_backfill_with, persona_id, read_backfills, read_persona, store_generated,
    write_persona,
};
use crate::dev_client::post_json;
use crate::dev_http::DevReply;
use crate::io_cause::IoCause;
use crate::speech::fixtures::FakePort;

fn done<T: std::fmt::Debug>(reply: PersonaReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

/// What a model writes for a persona; `backstory` and `private_backstory` are the parts a test
/// varies.
fn persona_answer(backstory: &str, private_backstory: &str) -> Value {
    json!({
        "display_name": "Marta Venn", "job": "Keeper of the inn",
        "personality": ["warm"], "motivations": [], "goals": [], "needs": [],
        "fears": ["debt"], "speech_style": "Dry.", "backstory": backstory,
        "private_backstory": private_backstory, "boundaries": [], "secrets": [],
        "behaviors": [], "default_behavior": null, "triggers": [],
        "leaks_private_side": false, "leaked_excerpt": null
    })
}

async fn generate(host: &ScriptedHost, extra: Value) -> PersonaReply<Generation> {
    let mut body = json!({"character": CHARACTER});
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    post_json(host.port, path::PERSONA_GENERATE, &body)
        .await
        .unwrap()
}

#[tokio::test]
async fn generating_without_overwrite_keeps_a_stored_persona_and_makes_no_model_call() {
    let host = scripted_host(vec![]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let kept = done(generate(&host, json!({})).await);
    assert!(matches!(kept, Generation::KeptExisting { .. }), "{kept:?}");
    assert_eq!(host.calls(), 0);
    assert!(
        host.persona_lines()
            .contains(&"PERSONA kept id=innkeeper".to_owned())
    );
}

#[tokio::test]
async fn a_generated_persona_is_stored_and_flags_a_leak_the_model_reported() {
    let mut answer = persona_answer(
        "She keeps a ledger of every debt.",
        "The ledger hides a forged page.",
    );
    answer["leaks_private_side"] = json!(true);
    answer["leaked_excerpt"] = json!("a ledger of every debt");
    let host = scripted_host(vec![answer]).await;
    let written = done(generate(&host, json!({"hint": "wary"})).await);
    let Generation::Generated {
        persona,
        leak_suspect,
    } = written
    else {
        panic!("{written:?}");
    };
    assert_eq!(persona.id, CHARACTER);
    assert!(leak_suspect, "the model reported a leak");
    assert_eq!(host.calls(), 1);
    assert!(host.file("innkeeper.persona.json").is_some());
    let lines = host.persona_lines();
    assert!(
        lines
            .iter()
            .any(|line| line
                .starts_with("PERSONA generate_started id=innkeeper operation=generate:")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(
            |line| line.starts_with("PERSONA generated id=innkeeper duration_ms=")
                && line.ends_with("leak_suspect=true")
        ),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("ledger") && !line.contains("wary"))
    );
    // The stored persona is kept the next time, with no further call.
    let again = done(generate(&host, json!({})).await);
    assert!(matches!(again, Generation::KeptExisting { .. }));
    assert_eq!(host.calls(), 1);
}

#[tokio::test]
async fn a_persona_that_appeared_while_the_model_was_asked_is_not_overwritten() {
    let folder = tempfile::tempdir().unwrap();
    let generated = innkeeper();
    let mut rival = innkeeper();
    rival = rival
        .edit(&[PersonaEdit::SetText {
            field: PersonaField::Job,
            text: "Landlady".to_owned(),
        }])
        .unwrap();
    // Two sequential writes stand in for the race: the rival's lands first.
    assert_eq!(store_generated(folder.path(), &rival, false), Ok(None));
    let kept = store_generated(folder.path(), &generated, false).unwrap();
    assert_eq!(kept.as_ref().map(Persona::job), Some("Landlady"));
    let id = PersonaId::new(CHARACTER).unwrap();
    assert_eq!(read_persona(folder.path(), &id), Ok(Some(rival)));
    // With overwrite the new persona replaces it.
    assert_eq!(store_generated(folder.path(), &generated, true), Ok(None));
    assert_eq!(read_persona(folder.path(), &id), Ok(Some(generated)));
}

#[tokio::test]
async fn a_line_the_model_reports_as_leaking_is_a_suspect_and_still_cued() {
    let host = scripted_host(vec![
        leaking_line_answer("Tomas went north, love."),
        line_answer("Welcome, love."),
    ])
    .await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let say = |situation: &'static str| {
        let body = json!({"character": CHARACTER, "situation": situation});
        async move {
            let reply: PersonaReply<Spoken> = post_json(host.port, path::PERSONA_SAY, &body)
                .await
                .unwrap();
            done(reply)
        }
    };
    let leaking = say("a guest asks about her family").await;
    assert!(leaking.said.leak_suspect);
    assert!(leaking.revision > 0);
    let clean = say("a guest arrives").await;
    assert!(!clean.said.leak_suspect);
    let said: Vec<String> = host
        .persona_lines()
        .into_iter()
        .filter(|line| line.starts_with("PERSONA said"))
        .collect();
    assert!(said[0].contains(" leak_suspect=true "), "{said:?}");
    assert!(said[1].contains(" leak_suspect=false "), "{said:?}");
}

#[test]
fn an_operation_key_holds_the_kind_and_a_counter_and_never_the_character() {
    for kind in ["generate", "say", "coverage", "backfill"] {
        let first = operation_key(kind).unwrap();
        let second = operation_key(kind).unwrap();
        assert_ne!(first.operation(), second.operation(), "each call is new");
        assert!(first.operation().starts_with(kind));
        assert!(first.operation().len() <= 128, "{}", first.operation());
    }
    // A persona id of the longest length is still a valid id, and no key is made from it.
    let longest = "a".repeat(df_content::persona::MAXIMUM_ID_CHARS);
    assert!(persona_id(&longest).is_ok());
}

#[test]
fn every_reserved_word_is_refused_as_a_persona_id_and_covers_the_routes_last_segments() {
    for word in RESERVED_IDS {
        assert_eq!(
            persona_id(word),
            Err(PersonaError::ReservedId {
                id: word.to_owned()
            }),
            "{word}"
        );
    }
    assert!(persona_id("innkeeper").is_ok());
    for route in [
        path::PERSONA_REMOVE,
        path::PERSONA_EDIT,
        path::PERSONA_GENERATE,
        path::PERSONA_GENERATE_CAST,
        path::PERSONA_SAY,
        path::PERSONA_ASK,
        path::PERSONA_COVERAGE,
        path::PERSONA_BACKFILL,
        path::PERSONA_BACKFILLS,
    ] {
        let last = route.rsplit('/').next().unwrap();
        assert!(RESERVED_IDS.contains(&last), "{last} is a route segment");
    }
}

#[tokio::test]
async fn every_route_that_creates_or_names_a_persona_refuses_a_reserved_id() {
    let host = scripted_host(vec![]).await;
    let reserved = |error: &PersonaError| matches!(error, PersonaError::ReservedId { .. });
    let mut persona = serde_json::to_value(innkeeper()).unwrap();
    persona["id"] = json!("remove");
    let written: PersonaReply<PersonaStored> =
        post_json(host.port, path::PERSONA, &persona).await.unwrap();
    assert!(matches!(written, DevReply::Rejected { error, .. } if reserved(&error)));
    let edited: PersonaReply<Value> = post_json(
        host.port,
        path::PERSONA_EDIT,
        &json!({"id": "all", "edits": []}),
    )
    .await
    .unwrap();
    assert!(matches!(edited, DevReply::Rejected { error, .. } if reserved(&error)));
    let removed: PersonaReply<Value> =
        post_json(host.port, path::PERSONA_REMOVE, &json!({"id": "list"}))
            .await
            .unwrap();
    assert!(matches!(removed, DevReply::Rejected { error, .. } if reserved(&error)));
    let listed: PersonaReply<Value> =
        crate::dev_client::get_json(host.port, "/dev/persona/backfills?id=say")
            .await
            .unwrap();
    assert!(matches!(listed, DevReply::Rejected { error, .. } if reserved(&error)));
    assert!(host.file("remove.persona.json").is_none());
}

fn record(edits: Vec<PersonaEdit>) -> BackfillRecord {
    BackfillRecord {
        sequence: 0,
        at_ms: 1,
        question: None,
        gaps: Vec::new(),
        edits,
        guidance: None,
        profile: "scripted".to_owned(),
    }
}

fn append(text: &str) -> PersonaEdit {
    PersonaEdit::AppendText {
        field: PersonaField::Backstory,
        text: text.to_owned(),
    }
}

#[test]
fn a_backfill_whose_record_cannot_be_written_puts_the_persona_file_back() {
    let folder = tempfile::tempdir().unwrap();
    let id = PersonaId::new(CHARACTER).unwrap();
    write_persona(folder.path(), &innkeeper()).unwrap();
    let failed = commit_backfill_with(
        folder.path(),
        &id,
        record(vec![append("She learned to brew ale at nine.")]),
        |_, _, _| Err(IoCause::StorageFull),
    );
    assert_eq!(
        failed,
        Err(PersonaError::BackfillsUnwritable {
            id: CHARACTER.to_owned(),
            cause: IoCause::StorageFull,
            persona_restored: true,
        })
    );
    assert_eq!(read_persona(folder.path(), &id), Ok(Some(innkeeper())));
    assert_eq!(read_backfills(folder.path(), &id), Ok(Vec::new()));
}

#[tokio::test]
async fn a_line_naming_a_behavior_trigger_or_secret_the_persona_lacks_is_refused() {
    let answers = ["behavior", "fired_triggers", "revealed_secrets"]
        .into_iter()
        .map(|field| {
            let mut answer = line_answer("Welcome, love.");
            answer[field] = if field == "behavior" {
                json!("furious")
            } else {
                json!(["not-there"])
            };
            answer
        })
        .collect();
    let host = scripted_host(answers).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    for _ in 0..3 {
        let body = json!({"character": CHARACTER, "situation": "a guest arrives"});
        let reply: PersonaReply<Spoken> = post_json(host.port, path::PERSONA_SAY, &body)
            .await
            .unwrap();
        let DevReply::Rejected { error, .. } = reply else {
            panic!("a line naming something the persona lacks is refused");
        };
        assert!(
            matches!(error, PersonaError::AnswerRefused { .. }),
            "{error:?}"
        );
    }
    let said = host
        .persona_lines()
        .into_iter()
        .filter(|line| line.starts_with("PERSONA said"))
        .count();
    assert_eq!(said, 0, "no refused line is reported as said");
}

#[tokio::test]
async fn generate_cast_writes_the_characters_whose_answers_read_and_reports_the_one_that_does_not()
{
    let host = scripted_host_cast(
        &["innkeeper", "cook", "elf"],
        vec![
            persona_answer("Keeps the inn.", "Owes the guild."),
            json!({"job": "a malformed answer"}),
            persona_answer("Maps the roads.", "Left home in shame."),
        ],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    let reply: PersonaReply<Vec<super::request::CastRow>> =
        post_json(host.port, path::PERSONA_GENERATE_CAST, &json!({}))
            .await
            .unwrap();
    let rows = done(reply);
    assert_eq!(rows.len(), 3, "one row for each named character");
    let generated = rows
        .iter()
        .filter(|row| matches!(row.outcome, CastOutcome::Generated { .. }))
        .count();
    let failed: Vec<&super::request::CastRow> = rows
        .iter()
        .filter(|row| matches!(row.outcome, CastOutcome::Failed { .. }))
        .collect();
    assert_eq!((generated, failed.len()), (2, 1), "{rows:?}");
    assert!(
        matches!(
            failed[0].outcome,
            CastOutcome::Failed {
                error: PersonaError::AnswerRefused { .. }
            }
        ),
        "{:?}",
        failed[0].outcome
    );
    // The two that read are stored; the one that did not has no file.
    let stored = |name: &str| host.file(&format!("{name}.persona.json")).is_some();
    let files = ["innkeeper", "cook", "elf"]
        .iter()
        .filter(|n| stored(n))
        .count();
    assert_eq!(files, 2);
    assert!(
        !stored(&failed[0].character),
        "no file for {}",
        failed[0].character
    );
}
