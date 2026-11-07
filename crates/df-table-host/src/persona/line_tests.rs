//! The line pipeline over a scripted backend: one call per line that also reports the gaps and the
//! appraisal, the checks of its ids, the state the next line carries, fired triggers taking their
//! behavior, the one repeat retry, the backfill after the line, the quote held inside the stall,
//! the world as each speaker sees it and the compact facts.
//!
//! TODO(DEV-12): a concurrent ask of one persona while its backfill runs (the `Busy` outcome), and
//! the tone cued on the scene once the cue carries one.

use std::time::Duration;

use df_content::persona::Persona;
use serde_json::{Value, json};

use super::fixtures::{
    CHARACTER, ScriptedHost, append_backstory_answer, innkeeper, line_answer,
    line_answer_with_gaps, privacy_answer, scripted_host, scripted_host_cast, write_market,
};
use super::request::{
    Answered, BackfillOutcome, BackfillRecord, PersonaError, PersonaReply, Spoken,
};
use super::routes::path;
use super::storage::write_persona;
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;
use crate::speech::fixtures::FakePort;
use std::sync::Arc;

fn done<T: std::fmt::Debug>(reply: PersonaReply<T>) -> T {
    match reply {
        DevReply::Done { value } => value,
        DevReply::Rejected { error, message } => panic!("refused: {error:?} {message}"),
    }
}

/// A shopkeeper with two behaviors, a trigger of the old kind, and one emotional trigger of each
/// kind: by event, and by state.
fn vendor(id: &str) -> Persona {
    serde_json::from_value(json!({
        "schema": 1, "id": id, "display_name": "Vendor", "job": "Keeper of a stall",
        "personality": ["brisk"],
        "backstory": "Sells fruit.",
        "behaviors": [
            {"id": "warm", "description": "Chatty.", "speech_style": ""},
            {"id": "curt", "description": "Short answers.", "speech_style": ""}
        ],
        "default_behavior": "warm",
        "triggers": [{"id": "haggled", "condition": "the buyer haggles", "behavior": "curt"}],
        "emotional_triggers": [
            {"id": "fed-up", "when_event": "the buyer insults the keeper",
             "sets": [{"emotion": "irritation", "delta": 40}], "behavior": "curt"},
            {"id": "scared", "when_state": {"emotion": "fear", "at_least": 50},
             "behavior": "curt"}
        ]
    }))
    .unwrap()
}

async fn ask(host: &ScriptedHost, character: &str, extra: Value) -> PersonaReply<Answered> {
    let mut body = json!({"character": character, "question": "How much for the apples?"});
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    post_json(host.port, path::PERSONA_ASK, &body)
        .await
        .unwrap()
}

async fn say(host: &ScriptedHost, character: &str, extra: Value) -> PersonaReply<Spoken> {
    let mut body = json!({"character": character, "situation": "A buyer waits at the stall."});
    for (key, value) in extra.as_object().into_iter().flatten() {
        body[key] = value.clone();
    }
    post_json(host.port, path::PERSONA_SAY, &body)
        .await
        .unwrap()
}

/// The first audit line that starts with `start`, waiting for a task that runs after its request.
async fn wait_for_line(host: &ScriptedHost, start: &str) -> String {
    for _ in 0..100 {
        if let Some(line) = host.lines_starting_with(start).into_iter().next() {
            return line;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no {start} line: {:?}", host.persona_lines());
}

fn appraised(mut answer: Value, appraisal: Value) -> Value {
    answer["appraisal"] = appraisal;
    answer
}

fn no_attention() -> Value {
    json!({"focus": null, "topic": null, "noticed": []})
}

#[tokio::test]
async fn one_call_carries_the_gaps_and_an_appraisal_that_only_keeps_what_is_valid() {
    let answer = appraised(
        line_answer_with_gaps("Eight a basket, love.", &[("backstory", "childhood")]),
        json!({
            "emotions": [{"emotion": "irritation", "delta": 30}, {"emotion": "bogus", "delta": 10}],
            "toward": "pip", "rapport_delta": -5,
            "attention": {
                "focus": "pip", "topic": "the price",
                "noticed": [{"speaker": "pip", "gist": "asked"}, {"speaker": "ghost", "gist": "x"}]
            },
            "tone": "danger"
        }),
    );
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let answered = done(
        ask(
            &host,
            CHARACTER,
            json!({"asked_by": "pip", "backfill": false}),
        )
        .await,
    );
    assert_eq!(host.calls(), 1, "no separate coverage call");
    assert!(!answered.covered);
    assert_eq!(answered.gaps.len(), 1);
    assert_eq!(answered.backfill, BackfillOutcome::NotAsked);
    let said = answered.spoken.said;
    assert_eq!(said.dropped, ["emotion:bogus", "speaker:ghost"]);
    assert_eq!(said.appraisal.emotions.len(), 1);
    assert_eq!(said.appraisal.toward.as_deref(), Some("pip"));
    assert_eq!(said.appraisal.attention.noticed.len(), 1);
    assert_eq!(said.tone.as_deref(), Some("danger"));
    assert_eq!(said.state.level("irritation"), 30);
    assert_eq!(
        said.state
            .relationship("pip")
            .map(|standing| standing.rapport),
        Some(-5)
    );
    let schema_text = host.asked.lock().unwrap()[0].output_schema.to_string();
    assert!(
        schema_text.contains("irritation"),
        "the emotion ids are the schema's: {schema_text}"
    );
}

#[tokio::test]
async fn an_appraisal_naming_a_speaker_who_is_not_present_or_a_tone_not_offered_is_dropped() {
    let answer = appraised(
        line_answer("Aye."),
        json!({"emotions": [], "toward": "ghost", "rapport_delta": 20, "attention": no_attention(),
               "tone": "nonsense"}),
    );
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({"asked_by": "pip"})).await).said;
    assert_eq!(said.dropped, ["toward:ghost", "tone:nonsense"]);
    assert_eq!(said.appraisal.toward, None);
    assert_eq!(said.appraisal.rapport_delta, None);
    assert_eq!(said.tone, None);
}

#[tokio::test]
async fn an_emotional_trigger_the_persona_does_not_have_is_refused() {
    let mut answer = line_answer("Aye.");
    answer["fired_emotional_triggers"] = json!(["no-such-trigger"]);
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let reply = say(&host, CHARACTER, json!({})).await;
    assert!(
        matches!(
            reply,
            DevReply::Rejected {
                error: PersonaError::AnswerRefused { .. },
                ..
            }
        ),
        "{reply:?}"
    );
}

#[tokio::test]
async fn the_appraisal_changes_the_state_the_next_line_carries() {
    let first = appraised(
        line_answer("Eight, and not a copper less."),
        json!({"emotions": [{"emotion": "irritation", "delta": 60}], "toward": null,
               "rapport_delta": null, "attention": {"focus": "pip", "topic": "haggling",
               "noticed": []}, "tone": null}),
    );
    let host = scripted_host(vec![first, line_answer("Next.")]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({"asked_by": "pip"})).await).said;
    assert_eq!(said.state.level("irritation"), 60);
    assert_eq!(
        said.tone.as_deref(),
        Some("danger"),
        "the strongest emotion's tone"
    );
    done(say(&host, CHARACTER, json!({"asked_by": "pip"})).await);
    let state = &host.facts_of(1)["now"]["state"];
    assert!(
        state["emotions"]
            .as_array()
            .unwrap()
            .contains(&json!({"emotion": "irritation", "level": 60})),
        "{state}"
    );
    assert_eq!(state["attention"]["focus"], "pip");
    let instructions = host.instructions_of(1);
    assert!(instructions.contains("never the content"), "{instructions}");
    assert!(
        instructions.contains("state.attention.focus first"),
        "{instructions}"
    );
}

#[tokio::test]
async fn a_fired_trigger_makes_its_behavior_the_lines_even_when_the_model_named_another() {
    let mut old_kind = line_answer("Seven. Take it or leave it.");
    old_kind["fired_triggers"] = json!(["haggled"]);
    old_kind["behavior"] = json!("warm");
    let mut by_event = line_answer("Out.");
    by_event["fired_emotional_triggers"] = json!(["fed-up"]);
    let host = scripted_host(vec![old_kind, by_event]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(said.behavior.as_deref(), Some("curt"));
    let over = said.behavior_override.expect("the mismatch is recorded");
    assert_eq!(
        (
            over.model.as_deref(),
            over.trigger.as_str(),
            over.took.as_str()
        ),
        (Some("warm"), "haggled", "curt")
    );
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(said.behavior.as_deref(), Some("curt"));
    assert_eq!(said.fired_emotional_triggers, ["fed-up"]);
    assert_eq!(
        said.state.level("irritation"),
        40,
        "the trigger's change joined the appraisal"
    );
}

#[tokio::test]
async fn a_trigger_whose_behavior_the_model_already_took_is_no_override() {
    let mut answer = line_answer("Seven.");
    answer["fired_triggers"] = json!(["haggled"]);
    answer["behavior"] = json!("curt");
    let host = scripted_host(vec![answer]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(said.behavior.as_deref(), Some("curt"));
    assert_eq!(said.behavior_override, None);
}

#[tokio::test]
async fn a_trigger_of_the_state_is_checked_by_code_passed_in_the_facts_and_fires() {
    let frightened = appraised(
        line_answer("Please."),
        json!({"emotions": [{"emotion": "fear", "delta": 70}], "toward": null,
               "rapport_delta": null, "attention": no_attention(), "tone": null}),
    );
    let host = scripted_host(vec![frightened, line_answer("Go away.")]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let first = done(say(&host, CHARACTER, json!({})).await).said;
    assert!(first.state_triggered.is_empty());
    let second = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(
        host.facts_of(1)["now"]["state_triggered"],
        json!([{"id": "scared", "behavior": "curt"}])
    );
    assert_eq!(second.state_triggered, ["scared"]);
    assert_eq!(second.fired_emotional_triggers, ["scared"]);
    assert_eq!(second.behavior.as_deref(), Some("curt"));
    assert_eq!(
        second.behavior_override.map(|over| over.trigger),
        Some("scared".to_owned())
    );
}

#[tokio::test]
async fn a_line_the_model_says_repeats_recent_is_worded_once_more_with_the_reason() {
    let mut repeating = line_answer("Fresh today, love.");
    repeating["repeats_recent"] = json!(true);
    repeating["repeat_reason"] = json!("opens with the same words");
    let host = scripted_host(vec![repeating, line_answer("Ripe, and cheap.")]).await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({"recent": ["Fresh today, love."]})).await).said;
    assert_eq!(host.calls(), 2);
    assert_eq!(said.line, "Ripe, and cheap.");
    assert!(said.repeat_retried && !said.repeats_recent);
    assert_eq!(
        host.facts_of(1)["now"]["retry_reason"],
        "opens with the same words"
    );
    assert!(host.facts_of(0)["now"].get("retry_reason").is_none());
}

#[tokio::test]
async fn a_line_still_repeating_after_the_retry_is_kept_reported_and_never_asked_again()
 {
    let repeating = |line: &str| {
        let mut answer = line_answer(line);
        answer["repeats_recent"] = json!(true);
        answer["repeat_reason"] = json!("same shape");
        answer
    };
    let host = scripted_host(vec![
        repeating("One."),
        repeating("Two."),
        line_answer("Three."),
    ])
    .await;
    write_persona(&host.directory(), &vendor(CHARACTER)).unwrap();
    let said = done(say(&host, CHARACTER, json!({})).await).said;
    assert_eq!(host.calls(), 2, "one retry and no more");
    assert_eq!(said.line, "Two.");
    assert!(said.repeat_retried && said.repeats_recent);
}

#[tokio::test]
async fn the_backfill_runs_after_the_line_and_is_recorded_when_done() {
    let host = scripted_host(vec![
        line_answer_with_gaps("Above the inn, love.", &[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        privacy_answer(false),
    ])
    .await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let answered = done(ask(&host, CHARACTER, json!({})).await);
    assert_eq!(answered.backfill, BackfillOutcome::Running { record: 1 });
    assert_eq!(answered.spoken.said.line, "Above the inn, love.");
    let finished = wait_for_line(&host, "PERSONA backfill_done").await;
    assert!(
        finished.starts_with("PERSONA backfill_done id=innkeeper record=1 edits=1"),
        "{finished}"
    );
    let lines = host.persona_lines();
    let position = |start: &str| {
        lines
            .iter()
            .position(|line| line.starts_with(start))
            .unwrap()
    };
    assert!(
        position("PERSONA said") < position("PERSONA backfilled"),
        "{lines:?}"
    );
    let records: PersonaReply<Vec<BackfillRecord>> =
        get_json(host.port, "/dev/persona/backfills?id=innkeeper")
            .await
            .unwrap();
    assert_eq!(done(records).len(), 1);
    assert_eq!(
        host.calls(),
        3,
        "the line, the backfill and the privacy judge"
    );
}

#[tokio::test]
async fn backfill_sync_waits_for_the_backfill_before_answering() {
    let host = scripted_host(vec![
        line_answer_with_gaps("Above the inn, love.", &[("backstory", "childhood")]),
        append_backstory_answer("She learned to brew ale at nine."),
        privacy_answer(false),
    ])
    .await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    let answered = done(ask(&host, CHARACTER, json!({"backfill_sync": true})).await);
    assert_eq!(
        answered.backfill,
        BackfillOutcome::Backfilled {
            edits: 1,
            sequence: 1
        }
    );
    assert_eq!(host.calls(), 3);
}

#[tokio::test]
async fn a_quote_is_held_inside_the_stall_and_both_numbers_are_reported() {
    let quote = |total: u64| {
        let mut answer = line_answer("Here you are.");
        answer["quote"] = json!({"item": "apple", "qty": 5, "total_cp": total});
        answer
    };
    let host = scripted_host_cast(
        &["fruit_vendor"],
        vec![quote(30), quote(100), quote(44)],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    write_persona(&host.directory(), &vendor("fruit_vendor")).unwrap();
    write_market(&host);
    let quoted = |said: &super::request::SaidLine| {
        let quote = said.quote.clone().unwrap();
        (
            quote.proposed_total_cp,
            said.quoted_cp,
            quote.bounds.map(|bounds| bounds.floor_total_cp),
        )
    };
    let low = done(say(&host, "fruit_vendor", json!({})).await).said;
    assert_eq!(
        quoted(&low),
        (30, Some(43), Some(43)),
        "raised to the floor"
    );
    let high = done(say(&host, "fruit_vendor", json!({})).await).said;
    assert_eq!(
        quoted(&high),
        (100, Some(45), Some(43)),
        "lowered to the ask"
    );
    assert_eq!(high.quote.unwrap().bounds.unwrap().ask_total_cp, 45);
    let inside = done(say(&host, "fruit_vendor", json!({})).await).said;
    assert_eq!(quoted(&inside), (44, Some(44), Some(43)));
}

#[tokio::test]
async fn a_quote_by_someone_who_keeps_no_such_stall_is_reported_unchecked() {
    let mut answer = line_answer("Five for forty.");
    answer["quote"] = json!({"item": "apple", "qty": 5, "total_cp": 40});
    let host = scripted_host_cast(&["pip"], vec![answer], Arc::new(FakePort::unconfigured())).await;
    write_persona(&host.directory(), &vendor("pip")).unwrap();
    write_market(&host);
    let said = done(say(&host, "pip", json!({})).await).said;
    assert_eq!(said.quoted_cp, Some(40));
    assert_eq!(said.quote.unwrap().bounds, None);
}

#[tokio::test]
async fn each_speaker_is_given_the_world_as_they_see_it() {
    let host = scripted_host_cast(
        &["fruit_vendor", "pip"],
        vec![line_answer("Five apples, please."), line_answer("Eight.")],
        Arc::new(FakePort::unconfigured()),
    )
    .await;
    write_persona(&host.directory(), &vendor("fruit_vendor")).unwrap();
    write_persona(&host.directory(), &vendor("pip")).unwrap();
    write_market(&host);
    done(say(&host, "pip", json!({})).await);
    done(say(&host, "fruit_vendor", json!({})).await);
    let customer = host.facts_of(0)["context"]["world"].clone();
    let keeper = host.facts_of(1)["context"]["world"].clone();
    assert_eq!(customer["viewer_role"], "customer");
    assert_eq!(keeper["viewer_role"], "keeper");
    assert!(
        customer["stalls"][0].get("haggle_floor_permille").is_none(),
        "{customer}"
    );
    assert!(
        customer["stalls"][0]["items"][0]["quotes"][0]
            .get("floor_total_cp")
            .is_none()
    );
    assert!(
        keeper["stalls"][0].get("haggle_floor_permille").is_some(),
        "{keeper}"
    );
    assert!(
        keeper["stalls"][0]["items"][0]["quotes"][0]
            .get("floor_total_cp")
            .is_some()
    );
    let instructions = host.instructions_of(0);
    assert!(
        instructions.contains("only the keeper of a stall quotes one"),
        "{instructions}"
    );
}

fn has_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty() || items.iter().any(has_empty),
        Value::Object(fields) => fields.is_empty() || fields.values().any(has_empty),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

#[tokio::test]
async fn the_facts_are_compact_with_the_stable_part_first_and_the_changing_part_last() {
    let host = scripted_host(vec![line_answer("Aye.")]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    done(
        say(
            &host,
            CHARACTER,
            json!({"recent": ["Pip: Hello."], "asked_by": "pip"}),
        )
        .await,
    );
    let facts = host.facts_of(0);
    let keys: Vec<&String> = facts.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["context", "now"], "the stable part sorts first");
    assert!(
        !has_empty(&facts),
        "no null, empty text, list or object: {facts}"
    );
    let context = facts["context"].as_object().unwrap();
    for stable in ["character", "voice_layers", "public_note", "scene"] {
        assert!(context.contains_key(stable), "{stable}");
    }
    for changing in ["recent", "asked_by", "situation", "state"] {
        assert!(facts["now"].get(changing).is_some(), "{changing}");
        assert!(!context.contains_key(changing), "{changing}");
    }
    assert!(facts.to_string().starts_with("{\"context\""));
}

#[tokio::test]
async fn the_line_is_audited_with_its_timings_and_no_text() {
    let host = scripted_host(vec![line_answer("Aye, Marta Venn.")]).await;
    write_persona(&host.directory(), &innkeeper()).unwrap();
    done(say(&host, CHARACTER, json!({})).await);
    let line = host.lines_starting_with("PERSONA said").remove(0);
    for expected in [" model_ms=", " total_ms=", " retried=false", " gaps=0"] {
        assert!(line.contains(expected), "{expected}: {line}");
    }
    assert!(!line.contains("Marta"), "{line}");
}
