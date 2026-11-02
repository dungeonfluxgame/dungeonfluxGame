//! Support text tests: fake backends through the real durable service, and the backends' pure
//! mapping functions. No network, no PostgreSQL.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicUsize, atomic::Ordering},
};

use df_ai::text::NoTextObserver;
use df_ai::{
    text::{TextEvent, TextEventKind},
    text_attempts::AttemptState,
    text_fixture::ScriptedBackend,
};
use df_support::classification::parse_judgement;

use super::*;

const MAILBOX: &str = "mailbox-support";

#[derive(Default)]
struct Recorder(Mutex<Vec<TextEvent>>);

impl TextObserver for Recorder {
    fn observe(&self, event: &TextEvent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
    }
}

impl Recorder {
    fn kinds(&self, purpose: TextPurpose) -> Vec<TextEventKind> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|event| event.purpose == purpose)
            .map(|event| event.kind)
            .collect()
    }
}

fn judge_answer(category: &str) -> Value {
    json!({
        "category": {"choice": category, "confidence": 0.95},
        "urgency": {"choice": "high", "confidence": 0.9},
        "privacy": {"choice": "no", "confidence": 0.9},
        "legal": {"choice": "uncertain", "confidence": 0.8},
        "safety": {"choice": "no", "confidence": 0.9},
    })
}

fn draft_record(category: &str) -> Value {
    json!({"category": category, "urgency": "high", "confidence": 0.95,
        "suggested_action": {"kind": "refund_review", "summary": "Review the duplicate charge"}})
}

fn answered(value: Value) -> TextReceipt {
    TextOutcome::Answered(value).into()
}

fn usage(input: u64, output: u64) -> TextUsage {
    TextUsage {
        total_tokens: input + output,
        input_tokens: Some(input),
        output_tokens: Some(output),
        cached_input_tokens: None,
        cache_write_tokens: None,
        reasoning_output_tokens: None,
    }
}

/// Records what `JevJudge` asks of the transport so tests start from the real wire.
#[derive(Clone, Default)]
struct Spy(Arc<Mutex<Vec<Value>>>);

#[async_trait::async_trait]
impl ModelTransport for Spy {
    fn model_name(&self, _: bool) -> &str {
        "spy"
    }

    async fn judge(&self, request: Value) -> Result<Value, JudgeError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(judge_value(&judge_answer("billing"), None))
    }

    async fn draft(&self, request: Value) -> Result<Value, JudgeError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Err(JudgeError::Unavailable)
    }
}

/// The judge wire and the draft wire `JevJudge` builds for one email.
async fn wires() -> (Value, Value) {
    let spy = Spy::default();
    JevJudge::new(spy.clone())
        .judge("Refund", "I was charged twice")
        .await
        .unwrap();
    let seen = spy.0.lock().unwrap_or_else(PoisonError::into_inner);
    (seen[0].clone(), seen[1].clone())
}

struct Rig<S> {
    judge: SupportJudge<ScriptedBackend, ScriptedBackend, S, Arc<Recorder>>,
    events: Arc<Recorder>,
    judge_calls: Arc<AtomicUsize>,
    draft_calls: Arc<AtomicUsize>,
}

impl<S: TextAttemptStore> Rig<S> {
    fn transport(
        &self,
    ) -> &SupportTextTransport<ScriptedBackend, ScriptedBackend, S, Arc<Recorder>> {
        &self.judge.transport
    }

    fn calls(&self) -> (usize, usize) {
        (
            self.judge_calls.load(Ordering::SeqCst),
            self.draft_calls.load(Ordering::SeqCst),
        )
    }
}

fn rig_with<S: TextAttemptStore>(
    store: Arc<S>,
    judge: Vec<TextReceipt>,
    draft: Vec<TextReceipt>,
    configured: bool,
) -> Rig<S> {
    let events = Arc::new(Recorder::default());
    let (judge_calls, draft_calls) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let model = |name: &str| configured.then(|| name.to_owned());
    let transport = SupportTextTransport::new(Parts {
        mailbox: MAILBOX.to_owned(),
        judge: ScriptedBackend::new(judge, judge_calls.clone()),
        draft: ScriptedBackend::new(draft, draft_calls.clone()),
        store,
        observer: events.clone(),
        judge_model: model("jev-test"),
        draft_model: model("draft-test"),
    })
    .unwrap();
    Rig {
        judge: SupportJudge::new(transport),
        events,
        judge_calls,
        draft_calls,
    }
}

fn rig(
    store: Arc<MemoryAttemptStore>,
    judge: Vec<TextReceipt>,
    draft: Vec<TextReceipt>,
) -> Rig<MemoryAttemptStore> {
    rig_with(store, judge, draft, true)
}

fn key(operation: &str) -> OperationKey {
    OperationKey::new(
        &AttemptScope::new("support:mailbox-support").unwrap(),
        operation,
    )
    .unwrap()
}

#[tokio::test]
async fn a_classification_makes_one_judge_call_then_one_draft_call_each_logged_once() {
    let store = Arc::new(MemoryAttemptStore::default());
    let rig = rig(
        store,
        vec![answered(judge_answer("billing"))],
        vec![answered(draft_record("billing"))],
    );
    let raw = rig
        .judge
        .judge("Refund", "I was charged twice")
        .await
        .unwrap();
    let (draft, evidence) = parse_judgement(&raw).unwrap();
    assert!(draft.suggested_action.is_some());
    assert_eq!(evidence["draft_unavailable"], json!(false));
    assert_eq!(rig.calls(), (1, 1));
    let sent_and_accepted = vec![TextEventKind::Answered, TextEventKind::Qualified];
    assert_eq!(
        rig.events.kinds(TextPurpose::SupportJudgment),
        sent_and_accepted
    );
    assert_eq!(
        rig.events.kinds(TextPurpose::SupportDraft),
        sent_and_accepted
    );
}

#[tokio::test]
async fn the_same_composition_works_over_the_hosted_store_type() {
    let store = Arc::new(SupportAttemptStore::Memory {
        store: MemoryAttemptStore::default(),
        reason: "test",
    });
    let rig = rig_with(
        store,
        vec![answered(judge_answer("billing"))],
        vec![answered(draft_record("billing"))],
        true,
    );
    assert!(rig.judge.judge("Refund", "charged twice").await.is_ok());
    assert_eq!(rig.calls(), (1, 1));
}

#[tokio::test]
async fn a_restart_replays_the_stored_judge_result_without_asking_again() {
    let (judge_wire, _) = wires().await;
    let store = Arc::new(MemoryAttemptStore::default());
    let first = rig(
        store.clone(),
        vec![TextReceipt {
            outcome: TextOutcome::Answered(judge_answer("billing")),
            usage: Some(usage(100, 7)),
        }],
        vec![],
    );
    let fresh = first
        .transport()
        .for_job("cls-abc")
        .judge(judge_wire.clone())
        .await
        .unwrap();
    assert_eq!(fresh["usage"]["input_tokens"], json!(100));
    drop(first);

    let restarted = rig(store.clone(), vec![], vec![]);
    let replayed = restarted
        .transport()
        .for_job("cls-abc")
        .judge(judge_wire)
        .await
        .unwrap();
    assert_eq!(replayed, fresh);
    assert_eq!(restarted.calls(), (0, 0));
    assert!(matches!(
        store.state(&key("jev-cls-abc")),
        Some(AttemptState::Settled { .. })
    ));
}

#[tokio::test]
async fn an_unknown_judge_send_is_not_retried_and_surfaces_as_the_existing_error() {
    let (judge_wire, _) = wires().await;
    let store = Arc::new(MemoryAttemptStore::default());
    let rig = rig(
        store.clone(),
        vec![
            TextOutcome::Unknown.into(),
            answered(judge_answer("billing")),
        ],
        vec![],
    );
    let transport = rig.transport().for_job("cls-abc");
    assert_eq!(
        transport.judge(judge_wire.clone()).await,
        Err(JudgeError::Unavailable)
    );
    assert_eq!(
        transport.judge(judge_wire).await,
        Err(JudgeError::Unavailable)
    );
    assert_eq!(rig.calls(), (1, 0));
    assert_eq!(
        store.state(&key("jev-cls-abc")),
        Some(AttemptState::Unknown)
    );
}

#[tokio::test]
async fn a_label_outside_the_option_set_is_rejected_at_qualify_and_is_unusable() {
    let store = Arc::new(MemoryAttemptStore::default());
    let rig = rig(
        store.clone(),
        vec![
            answered(judge_answer("refund")),
            answered(judge_answer("billing")),
        ],
        vec![],
    );
    assert_eq!(
        rig.judge.judge("Refund", "charged twice").await,
        Err(JudgeError::Unusable)
    );
    assert_eq!(
        rig.events.kinds(TextPurpose::SupportJudgment),
        vec![TextEventKind::Answered, TextEventKind::Rejected]
    );
    // The rejected answer is never regenerated under its key.
    assert_eq!(
        rig.judge.judge("Refund", "charged twice").await,
        Err(JudgeError::Unusable)
    );
    assert_eq!(rig.calls(), (1, 0));
}

#[tokio::test]
async fn a_confidence_outside_zero_through_one_is_rejected() {
    let mut answer = judge_answer("billing");
    answer["category"]["confidence"] = json!(1.5);
    let rig = rig(Arc::default(), vec![answered(answer)], vec![]);
    assert_eq!(
        rig.judge.judge("Refund", "charged twice").await,
        Err(JudgeError::Unusable)
    );
}

#[tokio::test]
async fn a_draft_that_changes_jevs_decision_is_rejected_and_unusable() {
    let (_, draft_wire) = wires().await;
    let store = Arc::new(MemoryAttemptStore::default());
    let rig = rig(store.clone(), vec![], vec![answered(draft_record("bug"))]);
    let transport = rig.transport().for_job("cls-abc");
    assert_eq!(transport.draft(draft_wire).await, Err(JudgeError::Unusable));
    assert!(matches!(
        store.state(&key("draft-cls-abc")),
        Some(AttemptState::Rejected { .. })
    ));
}

#[tokio::test]
async fn a_draft_failure_leaves_jevs_decision_in_place() {
    let rig = rig(
        Arc::default(),
        vec![answered(judge_answer("billing"))],
        vec![TextOutcome::Unknown.into()],
    );
    let raw = rig.judge.judge("Refund", "charged twice").await.unwrap();
    let (draft, evidence) = parse_judgement(&raw).unwrap();
    assert!(draft.suggested_action.is_none());
    assert_eq!(evidence["draft_unavailable"], json!(true));
}

#[tokio::test]
async fn settled_non_answers_map_to_the_existing_errors() {
    let (judge_wire, _) = wires().await;
    let cases = [
        (TextOutcome::Deadline, JudgeError::DeadlineExceeded),
        (TextOutcome::Invalid, JudgeError::Unusable),
        (
            TextOutcome::Unavailable(UnavailableReason::Authentication),
            JudgeError::Unavailable,
        ),
    ];
    for (index, (outcome, expected)) in cases.into_iter().enumerate() {
        let rig = rig(Arc::default(), vec![outcome.into()], vec![]);
        let transport = rig.transport().for_job(&format!("cls-{index}"));
        assert_eq!(transport.judge(judge_wire.clone()).await, Err(expected));
    }
}

#[tokio::test]
async fn releasing_the_run_stops_new_calls_before_anything_is_stored() {
    let store = Arc::new(MemoryAttemptStore::default());
    let rig = rig(
        store.clone(),
        vec![answered(judge_answer("billing"))],
        vec![],
    );
    rig.transport().composition.authority.release();
    assert_eq!(
        rig.judge.judge("Refund", "charged twice").await,
        Err(JudgeError::Unavailable)
    );
    assert_eq!(rig.calls(), (0, 0));
    assert!(store.is_empty());
}

#[tokio::test]
async fn an_unconfigured_model_is_unavailable_without_touching_the_store() {
    let store = Arc::new(MemoryAttemptStore::default());
    let rig = rig_with(store.clone(), vec![], vec![], false);
    assert_eq!(rig.transport().model_name(false), "unconfigured");
    assert_eq!(
        rig.judge.judge("Refund", "charged twice").await,
        Err(JudgeError::Unavailable)
    );
    assert!(store.is_empty());
    assert_eq!(rig.calls(), (0, 0));
}

#[tokio::test]
async fn operations_are_scoped_to_the_mailbox_and_named_after_the_job() {
    let (judge_wire, draft_wire) = wires().await;
    let store = Arc::new(MemoryAttemptStore::default());
    let rig = rig(
        store.clone(),
        vec![answered(judge_answer("billing"))],
        vec![answered(draft_record("billing"))],
    );
    let transport = rig.transport().for_job("cls-abc");
    transport.judge(judge_wire).await.unwrap();
    transport.draft(draft_wire).await.unwrap();
    assert_eq!(store.len(), 2);
    assert!(store.state(&key("jev-cls-abc")).is_some());
    assert!(store.state(&key("draft-cls-abc")).is_some());
}

#[tokio::test]
async fn the_judge_wire_becomes_the_same_batched_systemone_body_as_before() {
    let (wire, _) = wires().await;
    let request = judge_request(&wire).unwrap();
    assert_eq!(request.purpose, TextPurpose::SupportJudgment);
    assert_eq!(request.profile.as_str(), "support-jev");
    let body = JudgeWire::decode(&request).unwrap().body("jev-test");
    let mut old = wire;
    old["model"] = json!("jev-test");
    assert_eq!(body, old);
}

#[tokio::test]
async fn the_draft_wire_becomes_the_same_responses_body_as_before() {
    let (_, wire) = wires().await;
    let request = draft_request(&wire).unwrap();
    assert_eq!(request.purpose, TextPurpose::SupportDraft);
    assert_eq!(request.profile.as_str(), "support-draft");
    let old = json!({"model":"draft-test","store":false,
        "instructions":wire["instructions"],
        "input":json!({"state":wire["state"],"classification":wire["classification"],
            "concerns":wire["concerns"]}).to_string(),
        "max_output_tokens":4096,"text":{"format":{"type":"json_schema",
            "name":"support_action_draft","strict":true,"schema":wire["schema"]}}});
    assert_eq!(draft_body(&request, "draft-test"), old);
}

#[tokio::test]
async fn the_judge_schema_has_one_required_listed_choice_per_question() {
    let (wire, _) = wires().await;
    let request = judge_request(&wire).unwrap();
    let ids: Vec<&str> = question_ids(&request);
    assert_eq!(ids.len(), 5);
    for id in ["category", "urgency", "privacy", "legal", "safety"] {
        assert!(ids.contains(&id));
    }
    assert_eq!(
        request.output_schema["required"].as_array().map(Vec::len),
        Some(5)
    );
    assert_eq!(
        request.output_schema["properties"]["privacy"]["properties"]["choice"]["enum"],
        json!(["no", "uncertain", "yes"])
    );
}

fn good_judge_body() -> Body {
    let answers: Map<String, Value> = judge_value(&judge_answer("billing"), None)["answers"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    Body::Complete(
        json!({"answers":answers,"usage":{"input_tokens":100,"output_tokens":4}})
            .to_string()
            .into_bytes(),
    )
}

fn good_draft_body(text: &str) -> Body {
    Body::Complete(
        json!({"status":"completed","output":[{"content":[{"type":"output_text","text":text}]}],
            "usage":{"input_tokens":200,"output_tokens":30,"total_tokens":230,
                "input_tokens_details":{"cached_tokens":50},
                "output_tokens_details":{"reasoning_tokens":5}}})
        .to_string()
        .into_bytes(),
    )
}

fn outcome(receipt: TextReceipt) -> TextOutcome {
    receipt.outcome
}

#[tokio::test]
async fn the_judge_backend_maps_every_http_result_with_no_network() {
    let (wire, _) = wires().await;
    let request = judge_request(&wire).unwrap();
    let complete = || Body::Complete(b"{}".to_vec());
    let cases = [
        (
            401,
            complete(),
            TextOutcome::Unavailable(UnavailableReason::Authentication),
        ),
        (
            403,
            complete(),
            TextOutcome::Unavailable(UnavailableReason::Authentication),
        ),
        (
            302,
            complete(),
            TextOutcome::Unavailable(UnavailableReason::Configuration),
        ),
        (400, complete(), TextOutcome::Invalid),
        (429, Body::TooLarge, TextOutcome::Invalid),
        (500, complete(), TextOutcome::Invalid),
        (502, Body::Broken, TextOutcome::Unknown),
        (200, Body::Broken, TextOutcome::Unknown),
        (200, Body::TooLarge, TextOutcome::Invalid),
        (
            200,
            Body::Complete(b"not json".to_vec()),
            TextOutcome::Invalid,
        ),
        (200, complete(), TextOutcome::Invalid),
    ];
    for (status, body, expected) in cases {
        assert_eq!(
            outcome(judge_receipt(&request, status, &body)),
            expected,
            "status {status} body {body:?}"
        );
    }
    let receipt = judge_receipt(&request, 200, &good_judge_body());
    assert_eq!(receipt.usage, Some(usage(100, 4)));
    assert_eq!(
        receipt.outcome,
        TextOutcome::Answered(judge_answer("billing"))
    );
}

#[tokio::test]
async fn the_judge_backend_refuses_answers_for_other_questions_or_with_extra_fields() {
    let (wire, _) = wires().await;
    let request = judge_request(&wire).unwrap();
    let mut answers = judge_value(&judge_answer("billing"), None)["answers"].clone();
    answers["extra"] = answers["category"].clone();
    let body = Body::Complete(json!({"answers":answers}).to_string().into_bytes());
    assert_eq!(
        outcome(judge_receipt(&request, 200, &body)),
        TextOutcome::Invalid
    );
    let mut answers = judge_value(&judge_answer("billing"), None)["answers"].clone();
    answers["category"]["note"] = json!("x");
    let body = Body::Complete(json!({"answers":answers}).to_string().into_bytes());
    assert_eq!(
        outcome(judge_receipt(&request, 200, &body)),
        TextOutcome::Invalid
    );
}

#[test]
fn the_draft_backend_maps_every_http_result_with_no_network() {
    let record = draft_record("billing");
    let text = record.to_string();
    let complete = || Body::Complete(b"{}".to_vec());
    let cases = [
        (
            401,
            complete(),
            TextOutcome::Unavailable(UnavailableReason::Authentication),
        ),
        (
            403,
            complete(),
            TextOutcome::Unavailable(UnavailableReason::Authentication),
        ),
        (
            307,
            complete(),
            TextOutcome::Unavailable(UnavailableReason::Configuration),
        ),
        (422, complete(), TextOutcome::Invalid),
        (503, Body::Broken, TextOutcome::Unknown),
        (200, Body::Broken, TextOutcome::Unknown),
        (200, Body::TooLarge, TextOutcome::Invalid),
        (200, complete(), TextOutcome::Invalid),
        (200, good_draft_body("[1]"), TextOutcome::Invalid),
        (200, good_draft_body("not json"), TextOutcome::Invalid),
        (200, good_draft_body(&text), TextOutcome::Answered(record)),
    ];
    for (status, body, expected) in cases {
        assert_eq!(
            outcome(draft_receipt(status, &body)),
            expected,
            "status {status} body {body:?}"
        );
    }
}

#[test]
fn a_draft_reply_that_is_not_one_completed_output_text_is_invalid() {
    let incomplete = json!({"status":"incomplete","output":[]})
        .to_string()
        .into_bytes();
    let two = json!({"status":"completed","output":[{"content":[
        {"type":"output_text","text":"{}"},{"type":"output_text","text":"{}"}]}]})
    .to_string()
    .into_bytes();
    for body in [incomplete, two] {
        assert_eq!(
            outcome(draft_receipt(200, &Body::Complete(body))),
            TextOutcome::Invalid
        );
    }
}

#[test]
fn draft_usage_keeps_every_reported_category_and_leaves_the_rest_unknown() {
    let receipt = draft_receipt(200, &good_draft_body("{}"));
    assert_eq!(
        receipt.usage,
        Some(TextUsage {
            total_tokens: 230,
            input_tokens: Some(200),
            output_tokens: Some(30),
            cached_input_tokens: Some(50),
            cache_write_tokens: None,
            reasoning_output_tokens: Some(5),
        })
    );
}

#[test]
fn usage_is_never_zero_clamped_or_invented() {
    assert_eq!(text_usage(None, None, None, None, None), None);
    assert_eq!(
        text_usage(None, Some(9), None, None, None)
            .map(|usage| (usage.total_tokens, usage.output_tokens)),
        Some((9, None))
    );
    assert_eq!(text_usage(None, Some(u64::MAX), Some(1), None, None), None);
    assert_eq!(
        text_usage(Some(5), None, None, None, None).map(|usage| usage.input_tokens),
        Some(None)
    );
    let huge = Body::Complete(
        br#"{"answers":{},"usage":{"input_tokens":18446744073709551616,"output_tokens":2}}"#
            .to_vec(),
    );
    let (wire, _) = futures_free_wires();
    let request = judge_request(&wire).unwrap();
    // 2^64 does not fit a u64: that category is unknown and the total is the reported rest.
    let receipt = judge_receipt(&request, 200, &huge);
    assert_eq!(
        receipt
            .usage
            .map(|usage| (usage.input_tokens, usage.output_tokens)),
        Some((None, Some(2)))
    );
}

/// The judge wire without an async runtime, for pure tests.
fn futures_free_wires() -> (Value, Value) {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map(|runtime| runtime.block_on(wires()))
        .unwrap()
}

#[test]
fn send_failures_map_by_whether_the_request_could_have_left() {
    use SendFailure::*;
    assert_eq!(
        failure_outcome(Builder),
        TextOutcome::Unavailable(UnavailableReason::Configuration)
    );
    assert_eq!(
        failure_outcome(Connect),
        TextOutcome::Unavailable(UnavailableReason::Interrupted)
    );
    assert_eq!(failure_outcome(ConnectTimeout), TextOutcome::Deadline);
    assert_eq!(failure_outcome(Other), TextOutcome::Unknown);
}

fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

fn route_with(base: &str, key: &str, model: &str) -> Option<ModelRoute> {
    route(
        &environment(&[("P_API_KEY", key), ("P_BASE_URL", base), ("P_MODEL", model)]),
        "P",
    )
}

#[test]
fn a_plain_https_route_is_accepted_and_trimmed() {
    let route = route_with("https://models.example.test/api/", "k", "m").unwrap();
    assert_eq!(route.endpoint, "https://models.example.test/api");
    assert_eq!(route.model, "m");
}

#[test]
fn a_route_with_an_unsafe_base_or_empty_value_is_refused() {
    for base in [
        "http://models.example.test",
        "https://user@models.example.test",
        "https://user:pass@models.example.test",
        "https://models.example.test?x=1",
        "https://models.example.test#frag",
        "not a url",
        "file:///etc/passwd",
    ] {
        assert!(route_with(base, "k", "m").is_none(), "{base}");
    }
    assert!(route_with("https://models.example.test", "", "m").is_none());
    assert!(route_with("https://models.example.test", "k", "").is_none());
    assert!(route(&environment(&[("P_API_KEY", "k")]), "P").is_none());
}

#[test]
fn the_attempt_store_is_the_database_only_over_a_safe_transport() {
    let mut remote = tokio_postgres::Config::new();
    remote.host("db.example.test").port(5432).user("support");
    let mut local = tokio_postgres::Config::new();
    local.host("127.0.0.1").port(5432).user("support");
    let missing = PostgresTransport::VerifiedTls {
        ca_file: std::env::temp_dir().join("df-support-text-no-such-ca.pem"),
    };
    let plaintext = PostgresTransport::PlaintextLoopbackOnly;

    let store = SupportAttemptStore::choose(&remote, &plaintext);
    assert_eq!(
        (store.label(), store.fallback_reason()),
        ("memory", Some("plaintext_to_remote_host"))
    );
    let store = SupportAttemptStore::choose(&remote, &missing);
    assert_eq!(
        (store.label(), store.fallback_reason()),
        ("memory", Some("ca_unreadable"))
    );
    let store = SupportAttemptStore::choose(&local, &plaintext);
    assert_eq!((store.label(), store.fallback_reason()), ("postgres", None));
}

#[test]
fn a_mailbox_label_that_cannot_name_a_scope_is_refused() {
    let result =
        SupportTextTransport::<ScriptedBackend, ScriptedBackend, MemoryAttemptStore, _>::new(
            Parts {
                mailbox: "m".repeat(200),
                judge: ScriptedBackend::new(Vec::<TextReceipt>::new(), Arc::default()),
                draft: ScriptedBackend::new(Vec::<TextReceipt>::new(), Arc::default()),
                store: Arc::default(),
                observer: Arc::new(NoTextObserver),
                judge_model: None,
                draft_model: None,
            },
        );
    assert!(matches!(
        result.err(),
        Some(SetupError::Mailbox(LabelError::TooLong))
    ));
}
