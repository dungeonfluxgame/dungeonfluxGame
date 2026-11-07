use serde_json::{Value, json};

use super::fixtures::{
    CHARACTER, ScriptedHost, append_backstory_answer, coverage_answer, innkeeper, line_answer,
    scripted_host,
};
use super::request::{
    Answered, BackfillOutcome, BackfillRecord, Backfilled, Covered, PersonaError, PersonaReply,
};
use super::routes::path;
use super::storage::write_persona;
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;

fn store_innkeeper(host: &ScriptedHost) {
    write_persona(&host.directory(), &innkeeper()).unwrap();
}

async fn ask(host: &ScriptedHost, extra: Value) -> PersonaReply<Answered> {
    let mut body = json!({"character": CHARACTER, "question": "Where did you grow up?"});
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    post_json(host.port, path::PERSONA_ASK, &body)
        .await
        .unwrap()
}

fn done<T: std::fmt::Debug>(reply: PersonaReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

const PERSONA_FILE: &str = "innkeeper.persona.json";
const RECORD_FILE: &str = "innkeeper.backfills.json";

#[tokio::test]
async fn a_covered_question_is_answered_without_a_backfill_call_and_writes_no_file() {
    let host = scripted_host(vec![
        coverage_answer(&[]),
        line_answer("Ale, bread and a bed."),
    ])
    .await;
    store_innkeeper(&host);
    let before = host.file(PERSONA_FILE);
    let answered = done(ask(&host, json!({})).await);
    assert!(answered.covered);
    assert_eq!(answered.backfill, BackfillOutcome::NotNeeded);
    assert_eq!(answered.spoken.said.line, "Ale, bread and a bed.");
    assert!(
        answered.spoken.revision > 0,
        "the line was cued on the scene"
    );
    assert_eq!(answered.profile, "scripted");
    assert_eq!(
        host.calls(),
        2,
        "the coverage and the line, and no backfill"
    );
    assert_eq!(host.file(PERSONA_FILE), before);
    assert_eq!(host.file(RECORD_FILE), None);
}

#[tokio::test]
async fn gaps_are_backfilled_with_an_addition_that_is_recorded_and_the_line_is_cued() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&host);
    let before = innkeeper();
    let answered = done(ask(&host, json!({"guidance": {"arc": "the road"}})).await);
    assert!(!answered.covered);
    assert_eq!(
        answered.backfill,
        BackfillOutcome::Backfilled {
            edits: 1,
            sequence: 1
        }
    );
    assert!(answered.spoken.revision > 0);
    assert_eq!(host.calls(), 3);
    let stored: super::request::Edited = {
        let reply: PersonaReply<super::request::Edited> = post_json(
            host.port,
            path::PERSONA_EDIT,
            &json!({"id": CHARACTER, "edits": []}),
        )
        .await
        .unwrap();
        done(reply)
    };
    assert!(stored.persona.backstory().starts_with(before.backstory()));
    assert!(
        stored
            .persona
            .backstory()
            .ends_with("She learned to brew ale at nine.")
    );
    assert_eq!(
        stored.persona.private_backstory(),
        before.private_backstory()
    );
    assert_eq!(stored.persona.private_fears(), before.private_fears());
    let records: Vec<BackfillRecord> =
        serde_json::from_str(&host.file(RECORD_FILE).expect("a record file")).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].sequence, 1);
    assert_eq!(
        records[0].question.as_deref(),
        Some("Where did you grow up?")
    );
    assert_eq!(
        records[0].guidance.as_ref().and_then(|g| g.arc.as_deref()),
        Some("the road")
    );
    assert_eq!(records[0].profile, "scripted");
    let listed: PersonaReply<Vec<BackfillRecord>> =
        get_json(host.port, "/dev/persona/backfills?id=innkeeper")
            .await
            .unwrap();
    assert_eq!(done(listed), records);
}

#[tokio::test]
async fn a_backfill_answer_that_is_malformed_is_reported_and_the_line_is_still_said() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        json!({"edits": "not a list"}),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&host);
    let before = host.file(PERSONA_FILE);
    let answered = done(ask(&host, json!({})).await);
    let BackfillOutcome::BackfillFailed { error } = &answered.backfill else {
        panic!("{:?}", answered.backfill);
    };
    assert!(
        matches!(
            error,
            PersonaError::AnswerRefused {
                replayed: false,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(answered.spoken.said.line, "Above the inn, love.");
    assert_eq!(host.file(PERSONA_FILE), before);
    assert_eq!(host.file(RECORD_FILE), None);
    assert!(
        host.persona_lines()
            .iter()
            .any(|line| line == "PERSONA backfill_failed id=innkeeper error=answer_refused")
    );
}

#[tokio::test]
async fn a_backfill_that_copies_a_private_word_into_the_backstory_writes_neither_file() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("Her brother Tomas rode north when she was nine."),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&host);
    let before = host.file(PERSONA_FILE);
    let answered = done(ask(&host, json!({})).await);
    assert_eq!(
        answered.backfill,
        BackfillOutcome::BackfillFailed {
            error: PersonaError::PrivateDetailInPublicField
        }
    );
    assert_eq!(host.file(PERSONA_FILE), before);
    assert_eq!(host.file(RECORD_FILE), None);
    assert!(
        host.persona_lines()
            .iter()
            .any(|line| line == "PERSONA backfill_failed id=innkeeper error=private_detail")
    );
}

#[tokio::test]
async fn a_requests_model_reaches_the_text_started_line_and_the_reply() {
    let host = scripted_host(vec![coverage_answer(&[]), line_answer("Aye.")]).await;
    store_innkeeper(&host);
    let answered = done(ask(&host, json!({"model": "gpt-6.1-sol"})).await);
    assert_eq!(answered.profile, "scripted:gpt-6.1-sol");
    let started = host.lines_starting_with("TEXT started");
    assert_eq!(started.len(), 2, "{started:?}");
    assert!(
        started
            .iter()
            .all(|line| line.contains("profile=scripted:gpt-6.1-sol")),
        "{started:?}"
    );
}

#[tokio::test]
async fn a_model_name_that_is_not_usable_is_refused_before_any_question() {
    let host = scripted_host(vec![]).await;
    store_innkeeper(&host);
    for model in ["", "-flag", "a b", "gpt\n6", &"m".repeat(101)] {
        let reply = ask(&host, json!({"model": model})).await;
        let DevReply::Rejected { error, .. } = reply else {
            panic!("{model:?} is refused");
        };
        assert!(
            matches!(error, PersonaError::InvalidModel { .. }),
            "{error:?}"
        );
    }
    assert_eq!(host.calls(), 0);
}

#[tokio::test]
async fn coverage_and_backfill_have_routes_of_their_own_and_refuse_what_they_cannot_fill() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
    ])
    .await;
    store_innkeeper(&host);
    let covered: PersonaReply<Covered> = post_json(
        host.port,
        path::PERSONA_COVERAGE,
        &json!({"character": CHARACTER, "question": "Where from?"}),
    )
    .await
    .unwrap();
    let covered = done(covered);
    assert!(!covered.enough);
    assert_eq!(covered.profile, "scripted");
    let filled: PersonaReply<Backfilled> = post_json(
        host.port,
        path::PERSONA_BACKFILL,
        &json!({"character": CHARACTER, "gaps": [{"field": "backstory", "topic": "childhood"}]}),
    )
    .await
    .unwrap();
    assert_eq!(done(filled).sequence, 1);
    let job: PersonaReply<Backfilled> = post_json(
        host.port,
        path::PERSONA_BACKFILL,
        &json!({"character": CHARACTER, "gaps": [{"field": "job", "topic": "x"}]}),
    )
    .await
    .unwrap();
    assert!(matches!(
        job,
        DevReply::Rejected {
            error: PersonaError::NotFillable { .. },
            ..
        }
    ));
    assert_eq!(host.calls(), 2, "the refusal made no call");
}

#[tokio::test]
async fn no_persona_audit_line_holds_persona_question_line_or_edit_text() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&host);
    let _ = ask(&host, json!({"situation": "a rainy night"})).await;
    let lines = host.persona_lines();
    for expected in [
        "PERSONA covered id=innkeeper enough=false gaps=1",
        "PERSONA backfilled id=innkeeper gaps=1 edits=1 duration_ms=",
        "PERSONA said id=innkeeper duration_ms=",
    ] {
        assert!(
            lines.iter().any(|line| line.starts_with(expected)),
            "{expected}: {lines:?}"
        );
    }
    for forbidden in [
        "Where did you grow up",
        "brew ale",
        "Above the inn",
        "rainy night",
        "Marta",
        "Tomas",
    ] {
        assert!(
            lines.iter().all(|line| !line.contains(forbidden)),
            "{forbidden}: {lines:?}"
        );
    }
}

// TODO(DEV-12): cancel-safety of `ask`: a client that drops the request between the coverage and
// the backfill still gets its backfill committed (needs a scripted backend that waits).
