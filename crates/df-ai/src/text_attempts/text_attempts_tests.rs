use super::*;
use crate::text::{TextEvent, TextLimits, TextReceipt, UnavailableReason};
use crate::text_fixture::{FixedAuthority, MemoryAttemptStore, ScriptedBackend};
use serde_json::json;
use std::{
    pin::pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Waker},
    time::Duration,
};

const BASIS: Basis = [7; 32];
const MOVED: Basis = [9; 32];

/// Every future in these tests completes on its first poll, so one poll suffices.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(value) => value,
        std::task::Poll::Pending => panic!("test future did not complete on its first poll"),
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<TextEvent>>);

impl TextObserver for Recorder {
    fn observe(&self, event: &TextEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

impl Recorder {
    fn kinds(&self) -> Vec<&'static str> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.kind.key())
            .collect()
    }
}

type Service =
    DurableTextService<ScriptedBackend, Arc<MemoryAttemptStore>, FixedAuthority, Arc<Recorder>>;

/// One service with its store, backend call count and event record, all readable by a test.
struct Harness {
    service: Service,
    store: Arc<MemoryAttemptStore>,
    calls: Arc<AtomicUsize>,
    events: Arc<Recorder>,
}

impl Harness {
    fn new(receipts: impl IntoIterator<Item = impl Into<TextReceipt>>) -> Self {
        Self::build(
            receipts,
            MemoryAttemptStore::default(),
            FixedAuthority::holding(BASIS),
            None,
        )
    }

    fn seeded(
        receipts: impl IntoIterator<Item = impl Into<TextReceipt>>,
        store: MemoryAttemptStore,
    ) -> Self {
        Self::build(receipts, store, FixedAuthority::holding(BASIS), None)
    }

    fn with_authority(
        receipts: impl IntoIterator<Item = impl Into<TextReceipt>>,
        authority: FixedAuthority,
    ) -> Self {
        Self::build(receipts, MemoryAttemptStore::default(), authority, None)
    }

    fn build(
        receipts: impl IntoIterator<Item = impl Into<TextReceipt>>,
        store: MemoryAttemptStore,
        authority: FixedAuthority,
        limits: Option<TextLimits>,
    ) -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let events = Arc::new(Recorder::default());
        let store = Arc::new(store);
        let mut text = TextService::new(ScriptedBackend::new(receipts, calls.clone()))
            .with_observer(events.clone());
        if let Some(limits) = limits {
            text = text.with_limits(limits);
        }
        Self {
            service: DurableTextService::new(text, store.clone(), authority, owner(1)),
            store,
            calls,
            events,
        }
    }

    fn submit(
        &self,
        operation: &str,
        request: &TextRequest,
    ) -> Result<Submission, DurableTextError> {
        block_on(self.service.submit(&key(operation), request))
    }

    fn qualify(
        &self,
        operation: &str,
        request: &TextRequest,
        verdict: Verdict,
    ) -> Result<Qualification, DurableTextError> {
        block_on(self.service.qualify(&key(operation), request, verdict))
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn kinds(&self) -> Vec<&'static str> {
        self.events.kinds()
    }
}

fn owner(value: u8) -> AttemptOwner {
    AttemptOwner::new([value; 16]).unwrap()
}

fn scope() -> AttemptScope {
    AttemptScope::new("local").unwrap()
}

fn key(operation: &str) -> OperationKey {
    OperationKey::new(&scope(), operation).unwrap()
}

fn request() -> TextRequest {
    TextRequest {
        purpose: TextPurpose::Storyteller,
        profile: TextProfile::new("profile-a").unwrap(),
        instructions: "Narrate one line.".to_owned(),
        facts: json!({"room": "harbor"}),
        output_name: "narration".to_owned(),
        output_schema: json!({"type": "object", "properties": {"line": {"type": "string"}}}),
        deadline: Duration::from_secs(30),
        images: Vec::new(),
    }
}

fn answer() -> TextOutcome {
    TextOutcome::Answered(json!({"line": "The tide turns."}))
}

fn answer_value() -> Value {
    json!({"line": "The tide turns."})
}

fn billed(total: u64) -> TextUsage {
    TextUsage {
        total_tokens: total,
        input_tokens: Some(total / 2),
        output_tokens: Some(total - total / 2),
        cached_input_tokens: None,
        cache_write_tokens: None,
        reasoning_output_tokens: None,
    }
}

fn candidate(replayed: bool) -> Submission {
    Submission::Candidate {
        answer: answer_value(),
        replayed,
        usage: None,
    }
}

fn settled(outcome: TextOutcome, replayed: bool) -> Submission {
    Submission::Settled {
        outcome,
        replayed,
        usage: None,
    }
}

#[test]
fn labels_refuse_empty_long_and_control_values() {
    let too_long = "k".repeat(MAXIMUM_OPERATION_KEY_BYTES + 1);
    assert_eq!(AttemptScope::new(""), Err(LabelError::Empty));
    assert_eq!(OperationKey::new(&scope(), ""), Err(LabelError::Empty));
    assert_eq!(
        OperationKey::new(&scope(), &too_long),
        Err(LabelError::TooLong)
    );
    assert_eq!(
        OperationKey::new(&scope(), "a\nb"),
        Err(LabelError::ControlCharacter)
    );
    assert_eq!(TextProfile::new(&too_long), Err(LabelError::TooLong));
    assert!(OperationKey::new(&scope(), &"k".repeat(MAXIMUM_OPERATION_KEY_BYTES)).is_ok());
}

#[test]
fn the_zero_owner_is_refused() {
    assert!(AttemptOwner::new([0; 16]).is_none());
}

#[test]
fn the_fingerprint_changes_with_every_field_that_shapes_the_answer() {
    let base = fingerprint(&request());
    assert_eq!(base, fingerprint(&request()));
    let mut other = request();
    other.instructions.push('!');
    assert_ne!(base, fingerprint(&other));
    let mut other = request();
    other.facts = json!({"room": "keep"});
    assert_ne!(base, fingerprint(&other));
    let mut other = request();
    other.deadline = Duration::from_secs(31);
    assert_ne!(base, fingerprint(&other));
    let mut other = request();
    other.profile = TextProfile::new("profile-b").unwrap();
    assert_ne!(base, fingerprint(&other));
}

#[test]
fn an_answer_is_a_candidate_and_a_repeat_is_not_a_hit_until_it_is_qualified() {
    let harness = Harness::new([answer()]);
    assert_eq!(harness.submit("turn-1", &request()), Ok(candidate(false)));
    assert_eq!(harness.submit("turn-1", &request()), Ok(candidate(true)));
    assert_eq!(
        harness.qualify("turn-1", &request(), Verdict::Accepted),
        Ok(Qualification::Qualified)
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(settled(answer(), true))
    );
    assert_eq!(harness.calls(), 1);
    assert_eq!(
        harness.kinds(),
        vec!["answered", "replayed", "qualified", "replayed"]
    );
}

#[test]
fn a_rejected_candidate_is_never_reused_or_regenerated() {
    let harness = Harness::new([answer()]);
    harness.submit("turn-1", &request()).unwrap();
    assert_eq!(
        harness.qualify("turn-1", &request(), Verdict::Rejected),
        Ok(Qualification::Rejected)
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Rejected { usage: None })
    );
    assert_eq!(harness.calls(), 1);
}

#[test]
fn a_qualified_answer_can_be_withdrawn_and_is_then_rejected() {
    let harness = Harness::new([answer()]);
    harness.submit("turn-1", &request()).unwrap();
    harness
        .qualify("turn-1", &request(), Verdict::Accepted)
        .unwrap();
    let withdrawn = block_on(harness.service.invalidate(&key("turn-1"), &request()));
    assert_eq!(withdrawn, Ok(true));
    assert_eq!(
        block_on(harness.service.invalidate(&key("turn-1"), &request())),
        Ok(false)
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Rejected { usage: None })
    );
    assert_eq!(harness.calls(), 1);
}

#[test]
fn a_verdict_only_lands_on_the_exact_request_and_an_existing_attempt() {
    let harness = Harness::new([answer()]);
    assert_eq!(
        harness.qualify("turn-1", &request(), Verdict::Accepted),
        Err(DurableTextError::UnknownOperation)
    );
    harness.submit("turn-1", &request()).unwrap();
    let mut other = request();
    other.instructions = "Narrate another line.".to_owned();
    assert_eq!(
        harness.qualify("turn-1", &other, Verdict::Accepted),
        Err(DurableTextError::Conflict)
    );
}

#[test]
fn a_verdict_without_a_candidate_changes_nothing() {
    let harness = Harness::new([answer()]);
    harness.submit("turn-1", &request()).unwrap();
    harness
        .qualify("turn-1", &request(), Verdict::Accepted)
        .unwrap();
    assert_eq!(
        harness.qualify("turn-1", &request(), Verdict::Rejected),
        Ok(Qualification::NotACandidate)
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(settled(answer(), true))
    );
}

#[test]
fn a_basis_that_moves_before_the_verdict_discards_the_candidate() {
    // Calls: submit start, before dispatch, after the backend, then the verdict.
    let authority = FixedAuthority::sequence([Some(BASIS), Some(BASIS), Some(BASIS), Some(MOVED)]);
    let harness = Harness::with_authority([answer()], authority);
    assert_eq!(harness.submit("turn-1", &request()), Ok(candidate(false)));
    assert_eq!(
        harness.qualify("turn-1", &request(), Verdict::Accepted),
        Ok(Qualification::Stale)
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(settled(TextOutcome::Stale, true))
    );
}

#[test]
fn a_basis_that_moves_during_dispatch_discards_the_answer_and_replays_stale() {
    let authority = FixedAuthority::sequence([Some(BASIS), Some(BASIS), Some(MOVED)]);
    let harness = Harness::with_authority([answer()], authority);
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(settled(TextOutcome::Stale, false))
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(settled(TextOutcome::Stale, true))
    );
    assert_eq!(harness.calls(), 1);
    assert_eq!(harness.kinds(), vec!["stale", "replayed"]);
}

#[test]
fn an_owner_that_holds_nothing_refuses_before_the_store_or_backend() {
    let harness = Harness::with_authority([answer()], FixedAuthority::sequence([None]));
    assert_eq!(
        harness.submit("turn-1", &request()),
        Err(DurableTextError::Unauthorized)
    );
    assert!(harness.store.is_empty());
    assert_eq!(harness.calls(), 0);
    assert_eq!(harness.kinds(), vec!["unauthorized"]);
}

#[test]
fn an_owner_that_loses_the_basis_before_dispatch_sends_nothing_and_leaves_the_attempt_accepted() {
    let authority = FixedAuthority::sequence([Some(BASIS), None]);
    let harness = Harness::with_authority([answer()], authority);
    assert_eq!(
        harness.submit("turn-1", &request()),
        Err(DurableTextError::Unauthorized)
    );
    assert_eq!(harness.calls(), 0);
    assert_eq!(
        harness.store.state(&key("turn-1")),
        Some(AttemptState::Accepted)
    );
}

#[test]
fn reusing_a_key_with_a_different_request_or_profile_is_a_conflict() {
    let harness = Harness::new([answer()]);
    harness.submit("turn-1", &request()).unwrap();
    let mut other = request();
    other.instructions = "Narrate another line.".to_owned();
    assert_eq!(
        harness.submit("turn-1", &other),
        Err(DurableTextError::Conflict)
    );
    let mut other = request();
    other.profile = TextProfile::new("profile-b").unwrap();
    assert_eq!(
        harness.submit("turn-1", &other),
        Err(DurableTextError::Conflict)
    );
    assert_eq!(harness.calls(), 1);
    assert_eq!(harness.kinds(), vec!["answered", "conflict", "conflict"]);
}

#[test]
fn the_same_operation_in_another_scope_is_a_separate_attempt() {
    let harness = Harness::new([answer(), answer()]);
    let other_scope = AttemptScope::new("campaign-2").unwrap();
    let first = block_on(harness.service.submit(&key("turn-1"), &request()));
    let second = block_on(harness.service.submit(
        &OperationKey::new(&other_scope, "turn-1").unwrap(),
        &request(),
    ));
    assert_eq!(first, Ok(candidate(false)));
    assert_eq!(second, Ok(candidate(false)));
    assert_eq!(harness.calls(), 2);
    assert_eq!(harness.store.len(), 2);
}

#[test]
fn a_refused_request_is_never_admitted() {
    let harness = Harness::new(Vec::<TextOutcome>::new());
    let mut bad = request();
    bad.instructions = "  ".to_owned();
    assert_eq!(
        harness.submit("turn-1", &bad),
        Err(DurableTextError::Refused(
            TextRequestError::EmptyInstructions
        ))
    );
    assert!(harness.store.is_empty());
    assert_eq!(harness.calls(), 0);
    assert_eq!(harness.kinds(), vec!["empty_instructions"]);
}

#[test]
fn a_send_recorded_by_a_previous_owner_is_unknown_and_never_resent() {
    let store = MemoryAttemptStore::seeded(
        &key("turn-1"),
        fingerprint(&request()),
        AttemptState::Dispatched { owner: owner(7) },
        Some(BASIS),
    );
    let harness = Harness::seeded([answer()], store);
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Unknown)
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Unknown)
    );
    assert_eq!(harness.calls(), 0);
    assert_eq!(
        harness.store.state(&key("turn-1")),
        Some(AttemptState::Unknown)
    );
    assert_eq!(harness.kinds(), vec!["unknown", "unknown"]);
}

#[test]
fn a_live_dispatch_by_this_owner_is_in_flight_not_unknown() {
    let store = MemoryAttemptStore::seeded(
        &key("turn-1"),
        fingerprint(&request()),
        AttemptState::Dispatched { owner: owner(1) },
        Some(BASIS),
    );
    let harness = Harness::seeded([answer()], store);
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::InFlight)
    );
    assert_eq!(harness.calls(), 0);
    assert_eq!(
        harness.store.state(&key("turn-1")),
        Some(AttemptState::Dispatched { owner: owner(1) })
    );
}

#[test]
fn an_accepted_but_never_dispatched_attempt_is_sent_once() {
    let store = MemoryAttemptStore::seeded(
        &key("turn-1"),
        fingerprint(&request()),
        AttemptState::Accepted,
        None,
    );
    let harness = Harness::seeded([answer()], store);
    assert_eq!(harness.submit("turn-1", &request()), Ok(candidate(false)));
    assert_eq!(harness.calls(), 1);
}

#[test]
fn cancel_before_dispatch_prevents_any_send() {
    let store = MemoryAttemptStore::seeded(
        &key("turn-1"),
        fingerprint(&request()),
        AttemptState::Accepted,
        None,
    );
    let harness = Harness::seeded([answer()], store);
    assert_eq!(block_on(harness.service.cancel(&key("turn-0"))), Ok(false));
    assert_eq!(block_on(harness.service.cancel(&key("turn-1"))), Ok(true));
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Cancelled)
    );
    assert_eq!(harness.calls(), 0);
    assert_eq!(harness.kinds(), vec!["cancelled"]);
}

#[test]
fn cancel_after_dispatch_changes_nothing() {
    let harness = Harness::new([answer()]);
    harness.submit("turn-1", &request()).unwrap();
    assert_eq!(block_on(harness.service.cancel(&key("turn-1"))), Ok(false));
    assert_eq!(harness.submit("turn-1", &request()), Ok(candidate(true)));
}

#[test]
fn a_failing_store_stops_before_any_send_and_reports_its_cause() {
    let harness = Harness::seeded(
        [answer()],
        MemoryAttemptStore::failing(AttemptStoreError::Unreachable),
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Err(DurableTextError::Store(AttemptStoreError::Unreachable))
    );
    assert_eq!(harness.calls(), 0);
    assert_eq!(harness.kinds(), vec!["store_failed"]);
    let events = harness.events.0.lock().unwrap();
    assert_eq!(events[0].kind.reason(), Some("unreachable"));
}

#[test]
fn a_backend_refusal_settles_and_replays_as_the_same_refusal() {
    let refusal = TextOutcome::Unavailable(UnavailableReason::Authentication);
    let harness = Harness::new([refusal.clone()]);
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(settled(refusal.clone(), false))
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(settled(refusal, true))
    );
    assert_eq!(harness.calls(), 1);
}

#[test]
fn a_backend_that_cannot_say_whether_it_sent_is_unknown_and_never_resent() {
    let harness = Harness::new([TextOutcome::Unknown]);
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Unknown)
    );
    assert_eq!(
        harness.store.state(&key("turn-1")),
        Some(AttemptState::Unknown)
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Unknown)
    );
    assert_eq!(harness.calls(), 1);
}

#[test]
fn usage_travels_with_the_candidate_the_replay_and_the_qualified_answer() {
    let receipt = TextReceipt {
        outcome: answer(),
        usage: Some(billed(42)),
    };
    let harness = Harness::new([receipt]);
    let with_usage = |replayed| Submission::Candidate {
        answer: answer_value(),
        replayed,
        usage: Some(billed(42)),
    };
    assert_eq!(harness.submit("turn-1", &request()), Ok(with_usage(false)));
    assert_eq!(harness.submit("turn-1", &request()), Ok(with_usage(true)));
    harness
        .qualify("turn-1", &request(), Verdict::Accepted)
        .unwrap();
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Settled {
            outcome: answer(),
            replayed: true,
            usage: Some(billed(42)),
        })
    );
}

#[test]
fn a_discarded_answer_keeps_the_usage_it_cost() {
    let receipt = TextReceipt {
        outcome: answer(),
        usage: Some(billed(17)),
    };
    let authority = FixedAuthority::sequence([Some(BASIS), Some(BASIS), Some(MOVED)]);
    let harness = Harness::with_authority([receipt], authority);
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Settled {
            outcome: TextOutcome::Stale,
            replayed: false,
            usage: Some(billed(17)),
        })
    );
}

#[test]
fn the_limits_still_apply_before_admission() {
    let limits = TextLimits::new(8, 1024, Duration::from_secs(60)).unwrap();
    let harness = Harness::build(
        Vec::<TextOutcome>::new(),
        MemoryAttemptStore::default(),
        FixedAuthority::holding(BASIS),
        Some(limits),
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Err(DurableTextError::Refused(
            TextRequestError::InstructionsTooLarge
        ))
    );
    assert!(harness.store.is_empty());
}

#[test]
fn debug_output_never_prints_the_answer() {
    let harness = Harness::new([answer()]);
    let submission = harness.submit("turn-1", &request()).unwrap();
    let state = harness.store.state(&key("turn-1")).unwrap();
    assert!(!format!("{submission:?}").contains("tide"));
    assert!(!format!("{state:?}").contains("tide"));
    assert!(!format!("{:?}", answer()).contains("tide"));
}

/// The phase and outcome keys are the PostgreSQL CHECK values; their spelling never drifts.
#[test]
fn stored_keys_keep_their_check_spelling_and_round_trip() {
    let phases: Vec<_> = AttemptPhase::ALL
        .into_iter()
        .map(AttemptPhase::key)
        .collect();
    assert_eq!(
        phases,
        [
            "accepted",
            "dispatched",
            "candidate",
            "settled",
            "rejected",
            "unknown",
            "cancelled",
        ]
    );
    for phase in AttemptPhase::ALL {
        assert_eq!(AttemptPhase::from_key(phase.key()), Some(phase));
    }
    let kinds: Vec<_> = OutcomeKind::ALL.into_iter().map(OutcomeKind::key).collect();
    assert_eq!(
        kinds,
        ["answered", "unavailable", "deadline", "invalid", "stale"]
    );
    for kind in OutcomeKind::ALL {
        assert_eq!(OutcomeKind::from_key(kind.key()), Some(kind));
    }
    assert_eq!(AttemptPhase::from_key("Accepted"), None);
}

#[test]
fn stored_keys_equal_their_serde_names() {
    for phase in AttemptPhase::ALL {
        assert_eq!(serde_json::to_value(phase).unwrap(), json!(phase.key()));
        assert_eq!(
            serde_json::from_value::<AttemptPhase>(json!(phase.key())).unwrap(),
            phase
        );
    }
    for kind in OutcomeKind::ALL {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.key()));
        assert_eq!(
            serde_json::from_value::<OutcomeKind>(json!(kind.key())).unwrap(),
            kind
        );
    }
}

#[test]
fn store_errors_have_snake_case_keys_and_durable_errors_keep_their_source() {
    let keys = [
        (AttemptStoreError::Unreachable, "unreachable"),
        (AttemptStoreError::TimedOut, "timed_out"),
        (AttemptStoreError::Rejected, "rejected"),
        (AttemptStoreError::Invalid, "invalid"),
    ];
    for (error, key) in keys {
        assert_eq!(error.key(), key);
        assert_eq!(TextEventKind::StoreFailed(error).reason(), Some(key));
    }
    let durable = DurableTextError::Store(AttemptStoreError::TimedOut);
    assert_eq!(
        durable.to_string(),
        "the text attempt store failed: the text attempt store did not answer in time"
    );
    let source = std::error::Error::source(&durable).map(ToString::to_string);
    assert_eq!(
        source.as_deref(),
        Some("the text attempt store did not answer in time")
    );
    assert!(std::error::Error::source(&DurableTextError::Conflict).is_none());
}

impl Harness {
    /// Each event's kind, usage and attempt, in order.
    fn facts(&self) -> Vec<(&'static str, Option<TextUsage>, u32)> {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|event| (event.kind.key(), event.usage, event.attempt))
            .collect()
    }
}

#[test]
fn an_answered_event_carries_the_usage_of_its_call_and_a_replay_carries_none() {
    let receipt = TextReceipt {
        outcome: answer(),
        usage: Some(billed(42)),
    };
    let harness = Harness::new([receipt]);
    harness.submit("turn-1", &request()).unwrap();
    harness.submit("turn-1", &request()).unwrap();
    harness
        .qualify("turn-1", &request(), Verdict::Accepted)
        .unwrap();
    assert_eq!(
        harness.facts(),
        vec![
            ("answered", Some(billed(42)), 1),
            ("replayed", None, 2),
            ("qualified", None, 2),
        ]
    );
}

#[test]
fn the_attempt_counts_each_submit_of_a_key_and_a_retry_that_sends_reports_its_usage() {
    // The first submit loses the basis before dispatch; the retry under the same key sends.
    let authority = FixedAuthority::sequence([Some(BASIS), None, Some(BASIS)]);
    let receipt = TextReceipt {
        outcome: TextOutcome::Invalid,
        usage: Some(billed(9)),
    };
    let harness = Harness::with_authority([receipt], authority);
    assert_eq!(
        harness.submit("turn-1", &request()),
        Err(DurableTextError::Unauthorized)
    );
    assert_eq!(
        harness.submit("turn-1", &request()),
        Ok(Submission::Settled {
            outcome: TextOutcome::Invalid,
            replayed: false,
            usage: Some(billed(9)),
        })
    );
    harness.submit("turn-2", &request()).unwrap();
    assert_eq!(
        harness.facts(),
        vec![
            ("unauthorized", None, 1),
            ("invalid", Some(billed(9)), 2),
            // Another key counts from 1; the exhausted script answers it unavailable.
            ("unavailable", None, 1),
        ]
    );
}

#[test]
fn a_new_key_past_the_count_bound_forgets_only_the_oldest_key() {
    let mut counts = SubmitCounts::default();
    assert_eq!(counts.count(&key("op-0")), 1);
    assert_eq!(counts.count(&key("op-0")), 2);
    for index in 1..MAXIMUM_COUNTED_OPERATIONS {
        counts.count(&key(&format!("op-{index}")));
    }
    assert_eq!(counts.count(&key("op-1")), 2);
    // The map is full: one new key forgets op-0 alone, and the recent keys keep their counts.
    assert_eq!(counts.count(&key("op-new")), 1);
    assert_eq!(counts.counts.len(), MAXIMUM_COUNTED_OPERATIONS);
    assert_eq!(counts.latest(&key("op-0")), 1);
    assert_eq!(counts.latest(&key("op-1")), 2);
    assert_eq!(counts.count(&key("op-0")), 1);
    assert_eq!(counts.counts.len(), MAXIMUM_COUNTED_OPERATIONS);
    assert_eq!(counts.order.len(), MAXIMUM_COUNTED_OPERATIONS);
}
