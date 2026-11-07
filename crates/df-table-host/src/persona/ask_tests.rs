//! `TextRoute::ask` on its own, over a scripted backend: what each way of not getting a usable
//! answer comes back as. The service states behind in-flight, cancelled and not-kept answers are
//! tested where they are made, in `df-ai`'s `text_attempts` tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use df_ai::text::{TextOutcome, UnavailableReason};
use df_ai::text_fixture::ScriptedBackend;
use serde_json::{Value, json};

use super::generate::{operation_key, persona_route, text_request};
use crate::text::{
    BackendChoice, ServiceReason, TextAskError, TextRoute, TextRuntime, TextServiceCause,
};

fn runtime(outcomes: Vec<TextOutcome>) -> (TextRuntime, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = ScriptedBackend::new(outcomes, Arc::clone(&calls));
    let runtime = TextRuntime::new(
        BackendChoice::Scripted(Arc::new(backend), None),
        Arc::new(|_| {}),
    );
    (runtime, calls)
}

fn request(route: &TextRoute, facts: Value) -> df_ai::text::TextRequest {
    text_request(
        route.profile(),
        "Answer.",
        facts,
        "answer",
        json!({"type": "object"}),
    )
}

fn accepting(answer: &Value) -> Result<Value, String> {
    Ok(answer.clone())
}

fn refusing(_: &Value) -> Result<Value, String> {
    Err("not what was asked".to_owned())
}

#[tokio::test]
async fn an_answer_the_reader_refuses_is_refused_and_is_never_asked_again() {
    let (text, calls) = runtime(vec![TextOutcome::Answered(json!({"a": 1}))]);
    let route = persona_route(&text, None).unwrap();
    let key = operation_key("say").unwrap();
    let asked = request(&route, json!({"q": 1}));
    let refused = route.ask(&key, &asked, refusing).await.unwrap_err();
    assert_eq!(
        refused,
        TextAskError::AnswerRefused("not what was asked".to_owned())
    );
    assert_eq!(
        refused.service_cause(),
        (
            TextServiceCause::AnswerRefused,
            Some("not what was asked".to_owned())
        )
    );
    // The same question again does not reach the model: its answer was rejected.
    let again = route.ask(&key, &asked, accepting).await.unwrap_err();
    assert_eq!(
        again,
        TextAskError::Service {
            cause: TextServiceCause::RejectedBefore,
            reason: None
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_kept_answer_replays_without_a_call_and_a_reader_that_refuses_it_is_told_so() {
    let (text, calls) = runtime(vec![TextOutcome::Answered(json!({"a": 1}))]);
    let route = persona_route(&text, None).unwrap();
    let key = operation_key("say").unwrap();
    let asked = request(&route, json!({"q": 1}));
    let first = route.ask(&key, &asked, accepting).await.unwrap();
    assert!(!first.replayed);
    let replay = route.ask(&key, &asked, accepting).await.unwrap();
    assert!(replay.replayed, "the kept answer is replayed");
    let refused = route.ask(&key, &asked, refusing).await.unwrap_err();
    assert_eq!(
        refused,
        TextAskError::ReplayRefused("not what was asked".to_owned())
    );
    assert_eq!(refused.service_cause().0, TextServiceCause::ReplayRefused);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "one call served all three");
}

#[tokio::test]
async fn a_question_the_service_settles_without_an_answer_says_why() {
    for (outcome, reason) in [
        (TextOutcome::Invalid, ServiceReason::Invalid),
        (TextOutcome::Deadline, ServiceReason::Deadline),
        (
            TextOutcome::Unavailable(UnavailableReason::Configuration),
            ServiceReason::Configuration,
        ),
        (
            TextOutcome::Unavailable(UnavailableReason::Authentication),
            ServiceReason::Authentication,
        ),
    ] {
        let (text, _) = runtime(vec![outcome.clone()]);
        let route = persona_route(&text, None).unwrap();
        let key = operation_key("say").unwrap();
        let error = route
            .ask(&key, &request(&route, json!({"q": 1})), accepting)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            TextAskError::Service {
                cause: TextServiceCause::NoAnswer,
                reason: Some(reason)
            },
            "{outcome:?}"
        );
    }
}

#[tokio::test]
async fn a_question_whose_result_was_lost_is_never_sent_again() {
    let (text, calls) = runtime(vec![TextOutcome::Unknown, TextOutcome::Answered(json!({}))]);
    let route = persona_route(&text, None).unwrap();
    let key = operation_key("say").unwrap();
    let asked = request(&route, json!({"q": 1}));
    let first = route.ask(&key, &asked, accepting).await.unwrap_err();
    let again = route.ask(&key, &asked, accepting).await.unwrap_err();
    for error in [&first, &again] {
        assert!(
            matches!(
                error,
                TextAskError::Service {
                    cause: TextServiceCause::Lost,
                    ..
                }
            ),
            "{error:?}"
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "never resent");
}

#[tokio::test]
async fn the_same_key_with_a_different_question_is_refused_before_the_model() {
    let (text, calls) = runtime(vec![TextOutcome::Answered(json!({"a": 1}))]);
    let route = persona_route(&text, None).unwrap();
    let key = operation_key("say").unwrap();
    route
        .ask(&key, &request(&route, json!({"q": 1})), accepting)
        .await
        .unwrap();
    let error = route
        .ask(&key, &request(&route, json!({"q": 2})), accepting)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TextAskError::Service {
            cause: TextServiceCause::Refused,
            reason: Some(ServiceReason::Conflict)
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
