use serde_json::{Value, json};

use super::fixtures::{
    CHARACTER, ScriptedHost, append_backstory_answer, coverage_answer, innkeeper, line_answer,
    privacy_answer, scripted_host,
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
    let mut body =
        json!({"character": CHARACTER, "question": "Where did you grow up?", "coverage": true});
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
        privacy_answer(false),
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
    assert_eq!(
        host.calls(),
        4,
        "coverage, backfill, privacy judge and line"
    );
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
        json!({"edits": "still not a list"}),
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
async fn a_backfill_the_model_judges_to_leak_private_detail_writes_neither_file() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("Her brother Tomas rode north when she was nine."),
        privacy_answer(true),
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
        privacy_answer(false),
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
    assert_eq!(host.calls(), 3, "the refusal made no call");
}

#[tokio::test]
async fn no_persona_audit_line_holds_persona_question_line_or_edit_text() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        privacy_answer(false),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&host);
    let _ = ask(&host, json!({"situation": "a rainy night"})).await;
    let lines = host.persona_lines();
    for expected in [
        "PERSONA covered id=innkeeper enough=false gaps=1",
        "PERSONA backfilled id=innkeeper gaps=1 edits=1 spoken=0 duration_ms=",
        "PERSONA said id=innkeeper model_ms=",
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

// Cancel-safety of `ask`: every model-spending persona route runs its work through `run_owned`,
// whose guarantee (the work finishes when the request is dropped) is tested in `blocking_tests`.

#[tokio::test]
async fn guidance_reaches_the_coverage_question_the_backfill_and_the_spoken_line() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        privacy_answer(false),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&host);
    let guidance = json!({
        "arc": "the guild road",
        "must_establish": ["she has walked the north road"],
        "must_avoid": ["the missing brother"],
        "tone": "dry",
        "latitude": "strict"
    });
    done(ask(&host, json!({"guidance": guidance})).await);
    assert_eq!(host.calls(), 4);
    for (call, step) in [(0, "coverage"), (1, "backfill"), (3, "line")] {
        let facts = host.facts_of(call);
        let facts = if step == "line" {
            facts["now"].clone()
        } else {
            facts
        };
        assert_eq!(
            facts["guidance"]["must_establish"],
            json!(["she has walked the north road"]),
            "the {step} question carries the guidance: {facts}"
        );
        assert_eq!(
            facts["guidance"]["must_avoid"],
            json!(["the missing brother"]),
            "{step}"
        );
        assert_eq!(facts["guidance"]["latitude"], "strict", "{step}");
        assert_eq!(facts["guidance"]["tone"], "dry", "{step}");
    }
    let instructions = host.instructions_of(3);
    assert!(instructions.contains("must_establish"), "{instructions}");
    assert!(instructions.contains("must_avoid"), "{instructions}");
    assert!(instructions.contains("latitude"), "{instructions}");
}

#[tokio::test]
async fn without_guidance_the_line_is_asked_with_none() {
    let host = scripted_host(vec![
        coverage_answer(&[]),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&host);
    done(ask(&host, json!({})).await);
    assert!(
        host.facts_of(1)["guidance"].is_null(),
        "{}",
        host.facts_of(1)
    );
}

#[tokio::test]
async fn a_backfill_with_non_ascii_text_leaves_ascii_only_files_that_read_back() {
    let curly = "She learned to brew ale at nine, in her father\u{2019}s cellar. Pony: Br\u{e1}n.";
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer(curly),
        privacy_answer(false),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&host);
    done(ask(&host, json!({})).await);
    let persona_file = host.file(PERSONA_FILE).expect("the persona file");
    let record_file = host.file(RECORD_FILE).expect("the record file");
    assert!(persona_file.is_ascii(), "{persona_file}");
    assert!(record_file.is_ascii(), "{record_file}");
    assert!(persona_file.contains(r"\u2019"), "{persona_file}");
    // Read back through the route, the stored persona has the characters as written.
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
    assert!(
        stored.persona.backstory().ends_with(curly),
        "{}",
        stored.persona.backstory()
    );
    let records: Vec<BackfillRecord> = serde_json::from_str(&record_file).unwrap();
    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn a_strict_latitude_refuses_a_new_secret_and_the_line_is_still_said() {
    let new_secret = json!({"edits": [{
        "op": "put_secret", "field": null, "text": "She hides a second ledger.", "id": "ledger",
        "sensitivity": "high", "reveal_conditions": [], "known_by": [],
        "speech_style": null, "condition": null, "event": null, "behavior": null
    }]});
    let host = scripted_host(vec![
        coverage_answer(&[("secrets", "what she hides")]),
        new_secret.clone(),
        new_secret,
        line_answer("Nothing, love."),
    ])
    .await;
    store_innkeeper(&host);
    let before = host.file(PERSONA_FILE);
    let answered = done(ask(&host, json!({"guidance": {"latitude": "strict"}})).await);
    assert!(
        matches!(answered.backfill, BackfillOutcome::BackfillFailed { .. }),
        "{:?}",
        answered.backfill
    );
    assert!(answered.spoken.revision > 0, "the line is still cued");
    assert_eq!(host.file(PERSONA_FILE), before, "no secret was written");
    assert_eq!(host.file(RECORD_FILE), None, "and no record");
}

#[tokio::test]
async fn what_the_backfill_did_not_fill_is_told_to_the_line_as_not_established() {
    // A failed backfill: the line is asked with the topic the persona still lacks.
    let failed = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood"), ("backstory", "first horse")]),
        json!({"edits": "not a list"}),
        json!({"edits": "still not a list"}),
        line_answer("I do not recall."),
    ])
    .await;
    store_innkeeper(&failed);
    done(ask(&failed, json!({})).await);
    assert_eq!(
        failed.facts_of(3)["now"]["not_established"],
        json!(["childhood", "first horse"])
    );
    assert!(failed.instructions_of(3).contains("not_established"));
    assert!(failed.instructions_of(3).contains("invent no"));
    // A backfill that was not asked for leaves the same topics not established.
    let not_asked = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        line_answer("I do not recall."),
    ])
    .await;
    store_innkeeper(&not_asked);
    done(ask(&not_asked, json!({"backfill": false})).await);
    assert_eq!(
        not_asked.facts_of(1)["now"]["not_established"],
        json!(["childhood"])
    );
    // A backfill that filled the gap, and a persona that held enough, leave nothing.
    let filled = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        privacy_answer(false),
        line_answer("Above the inn, love."),
    ])
    .await;
    store_innkeeper(&filled);
    done(ask(&filled, json!({})).await);
    assert!(filled.facts_of(3)["now"].get("not_established").is_none());
    let covered = scripted_host(vec![coverage_answer(&[]), line_answer("Above the inn.")]).await;
    store_innkeeper(&covered);
    done(ask(&covered, json!({})).await);
    assert!(covered.facts_of(1)["now"].get("not_established").is_none());
}

#[tokio::test]
async fn a_public_price_in_the_world_facts_reaches_the_judge_and_the_line_and_is_not_a_leak() {
    let host = scripted_host(vec![
        coverage_answer(&[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        privacy_answer(false),
        line_answer("Three copper an apple, love."),
    ])
    .await;
    store_innkeeper(&host);
    host.running.write_asset(
        "world/bible.json",
        include_str!("../../../../assets/playground/world/bible.json"),
    );
    host.running
        .write_asset("world/state.json", r#"{"place": "market"}"#);
    let answered = done(ask(&host, json!({})).await);
    assert!(
        matches!(answered.backfill, BackfillOutcome::Backfilled { .. }),
        "the judge's no-leak verdict lets the backfill commit"
    );
    // The judge is told the price is public, and is told the price.
    let judge = host.facts_of(2);
    assert_eq!(judge["known_public_world_facts"]["audience"], "public");
    let apple = &judge["known_public_world_facts"]["stalls"][0]["items"][0];
    assert_eq!(
        (
            apple["id"].clone(),
            apple["quotes"][0]["ask_total_cp"].clone()
        ),
        (json!("apple"), json!(3))
    );
    assert!(host.instructions_of(2).contains("never a leak"));
    // The line is told the same facts, and is told to state a price when asked.
    let line = host.facts_of(3);
    assert_eq!(
        line["context"]["world"]["stalls"][0]["items"][0]["quotes"][0]["ask_total_cp"],
        3
    );
    assert_eq!(line["context"]["world"]["audience"], "public");
    let instructions = host.instructions_of(3);
    assert!(instructions.contains("always states them when asked"));
    assert!(instructions.contains("is never a leak"));
}
