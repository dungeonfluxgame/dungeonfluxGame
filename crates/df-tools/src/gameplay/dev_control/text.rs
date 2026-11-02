//! `dev text`: the durable text service of the run, as an operator sees it. `list`, `attempt` and
//! `config` read. `cancel` cancels an attempt that has not dispatched and `prune` deletes old
//! finished attempts. `probe`, `qualify` and `invalidate` drive one scripted fixture request
//! through the real durable service, so the attempt row, the candidate handshake and the log
//! records can be checked end to end without any provider. Probes use their own purpose and
//! profile, and every one is logged to the shared telemetry sink; a run without that sink refuses
//! to probe. A store that is off says so instead of failing.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use df_ai::{
    text::{
        TextLimits, TextOutcome, TextProfile, TextPurpose, TextRequest, TextUsage,
        UnavailableReason,
    },
    text_attempts::{
        AttemptOwner, AttemptPhase, AttemptScope, AttemptStoreError, DurableTextError,
        DurableTextService, OperationKey, Qualification, Submission, TextAttemptStore, Verdict,
    },
    text_fixture::{FixedAuthority, ScriptedBackend},
};
use df_observe::DiagnosticSink;
use df_persistence::text_attempts::{AttemptSummary, MAXIMUM_RECENT, PostgresTextAttempts};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError, atomic::AtomicUsize},
};

use super::{Outcome, Shared, authorized, bad_request};
use crate::gameplay::{
    text_runtime::process_owner,
    text_telemetry::{TextTelemetry, text_service},
};

const DEFAULT_LIMIT: u32 = 20;
/// The scope operator commands act in unless `scope` says otherwise.
const DEFAULT_SCOPE: &str = "dev";
/// The profile every probe runs under, so probe rows are never mistaken for a game profile.
const PROBE_PROFILE: &str = "dev-probe";
/// The basis the probe's fixture owner holds. The probe never reads a campaign authority.
const PROBE_BASIS: [u8; 32] = [0x50; 32];
/// The answer a probe's scripted backend returns; tests assert it never reaches a log record.
const PROBE_ANSWER: &str = "probe-answer";
const MAXIMUM_PRUNE_DAYS: u32 = 3650;
const OFF_REASON: &str = "text attempts are off for this run: see the TEXT start-up line";
const NO_LOGGING_REASON: &str =
    "text probes need the shared telemetry sink, which this run does not have";

/// What a run keeps for the text service: its durable store and the shared sink its events use.
#[derive(Clone)]
pub(in crate::gameplay) struct Runtime {
    store: Arc<PostgresTextAttempts>,
    sink: Option<Arc<dyn DiagnosticSink>>,
}

static CURRENT: Mutex<Option<Runtime>> = Mutex::new(None);

fn locked() -> MutexGuard<'static, Option<Runtime>> {
    CURRENT.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(in crate::gameplay) fn install(
    store: Arc<PostgresTextAttempts>,
    sink: Option<Arc<dyn DiagnosticSink>>,
) {
    *locked() = Some(Runtime { store, sink });
}

pub(in crate::gameplay) fn current() -> Option<Runtime> {
    locked().clone()
}

#[cfg(test)]
pub(in crate::gameplay) fn uninstall() {
    *locked() = None;
}

/// A `--limit` of attempts: 1 to 100, 20 when absent.
fn parse_limit(text: Option<&String>) -> Result<u32, String> {
    match text {
        None => Ok(DEFAULT_LIMIT),
        Some(text) => match text.trim().parse::<u32>() {
            Ok(limit) if (1..=MAXIMUM_RECENT).contains(&limit) => Ok(limit),
            _ => Err(format!("limit {text:?} must be 1 to {MAXIMUM_RECENT}")),
        },
    }
}

fn parse_phase(text: &str) -> Result<AttemptPhase, String> {
    AttemptPhase::parse(text.trim()).ok_or_else(|| {
        let known: Vec<&str> = AttemptPhase::ALL
            .iter()
            .map(|phase| phase.label())
            .collect();
        format!("phase {text:?} must be one of {}", known.join(", "))
    })
}

fn parse_scope(text: Option<&str>) -> Result<AttemptScope, String> {
    AttemptScope::new(text.unwrap_or(DEFAULT_SCOPE))
        .map_err(|_| "text scope is not a valid scope label".to_owned())
}

fn parse_key(scope: Option<&str>, operation: &str) -> Result<OperationKey, String> {
    OperationKey::new(&parse_scope(scope)?, operation)
        .map_err(|_| "text operation key is not a valid label".to_owned())
}

fn json_ok(value: Value) -> Outcome {
    Ok((StatusCode::OK, Json(value)))
}

/// The answer when the store is not available. Nothing is changed.
fn unavailable(reason: &str) -> Outcome {
    json_ok(json!({"status": "unavailable", "surface": "text", "reason": reason}))
}

fn store_failure(error: AttemptStoreError) -> Outcome {
    match error {
        AttemptStoreError::Invalid => {
            Err(bad_request("the text attempt store refused this request"))
        }
        cause => json_ok(json!({
            "status": "unavailable",
            "surface": "text",
            "reason": "the text attempt store did not complete the request",
            "cause": cause.label(),
        })),
    }
}

fn summary_json(summary: &AttemptSummary) -> Value {
    json!({
        "scope": summary.scope,
        "operation_key": summary.operation_key,
        "purpose": summary.purpose.label(),
        "profile": summary.profile,
        "output_name": summary.output_name,
        "phase": summary.phase.label(),
        "outcome": summary.outcome.map(|outcome| outcome.label()),
        "reason": summary.reason.map(|reason| reason.label()),
        "answer_bytes": summary.answer_bytes,
        "usage_total_tokens": summary.usage_total,
        "updated_at": summary.updated_at,
    })
}

fn body_text<'a>(body: &'a Value, name: &str) -> Option<&'a str> {
    body.get(name).and_then(Value::as_str)
}

/// `GET /dev/text[?scope=<s>][&phase=<p>][&limit=<n>]`: the newest attempts.
pub(super) async fn list(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let limit = parse_limit(params.get("limit")).map_err(bad_request)?;
    let phase = params
        .get("phase")
        .map(|phase| parse_phase(phase))
        .transpose()
        .map_err(bad_request)?;
    let scope = params
        .get("scope")
        .map(|scope| parse_scope(Some(scope)))
        .transpose()
        .map_err(bad_request)?;
    let Some(runtime) = current() else {
        return unavailable(OFF_REASON);
    };
    match runtime.store.recent(scope.as_ref(), phase, limit).await {
        Ok(attempts) => json_ok(json!({
            "status": "ok",
            "attempts": attempts.iter().map(summary_json).collect::<Vec<_>>(),
        })),
        Err(error) => store_failure(error),
    }
}

/// `GET /dev/text/attempt?key=<operation>[&scope=<s>]`: one attempt, or `attempt: null`.
pub(super) async fn one(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let operation = params
        .get("key")
        .ok_or_else(|| bad_request("text attempt needs ?key=<operation key>"))?;
    let key = parse_key(params.get("scope").map(String::as_str), operation).map_err(bad_request)?;
    let Some(runtime) = current() else {
        return unavailable(OFF_REASON);
    };
    match runtime.store.attempt(&key).await {
        Ok(attempt) => json_ok(json!({
            "status": "ok",
            "attempt": attempt.as_ref().map(summary_json),
        })),
        Err(error) => store_failure(error),
    }
}

/// `GET /dev/text/config`: what the text service runs with, as the operator can see it.
pub(super) async fn config(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let runtime = current();
    let limits = TextLimits::default();
    json_ok(json!({
        "status": "ok",
        "store": runtime.is_some(),
        "logging": runtime.as_ref().is_some_and(|runtime| runtime.sink.is_some()),
        "limits": {
            "instruction_bytes": limits.maximum_instruction_bytes(),
            "facts_bytes": limits.maximum_facts_bytes(),
            "deadline_seconds": limits.maximum_deadline().as_secs(),
        },
        "purposes": TextPurpose::ALL.iter().map(|purpose| purpose.label()).collect::<Vec<_>>(),
        "phases": AttemptPhase::ALL.iter().map(|phase| phase.label()).collect::<Vec<_>>(),
        "probe": {"scope": DEFAULT_SCOPE, "profile": PROBE_PROFILE},
        "retention": "manual: dev text prune --days <n> deletes finished attempts only",
    }))
}

/// `POST /dev/text/cancel` with `{"key", "scope"?}`: cancels an attempt that has not dispatched.
/// `cancelled: false` means it dispatched, settled, or never existed; nothing changed.
pub(super) async fn cancel(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let operation =
        body_text(&body, "key").ok_or_else(|| bad_request("text cancel needs a key"))?;
    let key = parse_key(body_text(&body, "scope"), operation).map_err(bad_request)?;
    let Some(runtime) = current() else {
        return unavailable(OFF_REASON);
    };
    match runtime.store.cancel(&key).await {
        Ok(cancelled) => json_ok(json!({"status": "ok", "cancelled": cancelled})),
        Err(error) => store_failure(error),
    }
}

/// `POST /dev/text/prune` with `{"days": 1..3650}`: deletes settled, rejected and cancelled
/// attempts untouched for that long. Unknown, accepted, dispatched and candidate attempts stay.
pub(super) async fn prune(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let days = body
        .get("days")
        .and_then(Value::as_u64)
        .and_then(|days| u32::try_from(days).ok())
        .filter(|days| (1..=MAXIMUM_PRUNE_DAYS).contains(days))
        .ok_or_else(|| {
            bad_request(format!(
                "text prune needs days from 1 to {MAXIMUM_PRUNE_DAYS}"
            ))
        })?;
    let Some(runtime) = current() else {
        return unavailable(OFF_REASON);
    };
    match runtime.store.prune(days).await {
        Ok(deleted) => json_ok(json!({"status": "ok", "deleted": deleted})),
        Err(error) => store_failure(error),
    }
}

/// The probe's scripted backend answer, by name. Anything else is refused.
fn scripted_outcome(name: &str) -> Option<TextOutcome> {
    match name {
        "answer" => Some(TextOutcome::Answered(json!({"line": PROBE_ANSWER}))),
        "refuse" => Some(TextOutcome::Unavailable(UnavailableReason::Authentication)),
        "deadline" => Some(TextOutcome::Deadline),
        "lost" => Some(TextOutcome::Unknown),
        _ => None,
    }
}

/// The probe's request. It is fixed, so a probe repeated under one key replays its first result
/// and `qualify` and `invalidate` can only ever match probe attempts.
fn probe_request() -> TextRequest {
    TextRequest {
        purpose: TextPurpose::DevProbe,
        profile: TextProfile::new(PROBE_PROFILE).expect("the probe profile is a valid label"),
        instructions: "Development probe: answer with one line.".to_owned(),
        facts: json!({"probe": true}),
        output_name: "probe".to_owned(),
        output_schema: json!({"type": "object", "properties": {"line": {"type": "string"}}}),
        deadline: std::time::Duration::from_secs(30),
    }
}

type ProbeService<S> = DurableTextService<ScriptedBackend, S, FixedAuthority, Arc<TextTelemetry>>;

/// The durable service a probe runs through: the real service, a scripted backend and a fixture
/// owner that always holds its basis.
fn probe_service<S: TextAttemptStore>(
    store: S,
    sink: Arc<dyn DiagnosticSink>,
    scripted: Option<TextOutcome>,
    owner: AttemptOwner,
) -> ProbeService<S> {
    let backend = ScriptedBackend::new(
        scripted.into_iter().collect::<Vec<_>>(),
        Arc::new(AtomicUsize::new(0)),
    );
    DurableTextService::new(
        text_service(backend, sink),
        store,
        FixedAuthority::holding(PROBE_BASIS),
        owner,
    )
}

fn outcome_label(outcome: &TextOutcome) -> &'static str {
    match outcome {
        TextOutcome::Answered(_) => "answered",
        TextOutcome::Unavailable(_) => "unavailable",
        TextOutcome::Deadline => "deadline",
        TextOutcome::Invalid => "invalid",
        TextOutcome::Stale => "stale",
        TextOutcome::Unknown => "unknown",
    }
}

fn usage_json(usage: Option<TextUsage>) -> Value {
    usage.map_or(
        Value::Null,
        |usage| json!({"total_tokens": usage.total_tokens}),
    )
}

/// A submission as the operator sees it. An answer is never printed, only that one exists.
fn submission_json(result: &Result<Submission, DurableTextError>) -> Value {
    match result {
        Ok(Submission::Settled {
            outcome,
            replayed,
            usage,
        }) => json!({
            "status": "ok",
            "submission": "settled",
            "outcome": outcome_label(outcome),
            "replayed": replayed,
            "usage": usage_json(*usage),
        }),
        Ok(Submission::Candidate {
            replayed, usage, ..
        }) => json!({
            "status": "ok",
            "submission": "candidate",
            "replayed": replayed,
            "usage": usage_json(*usage),
        }),
        Ok(Submission::Rejected { usage }) => {
            json!({"status": "ok", "submission": "rejected", "usage": usage_json(*usage)})
        }
        Ok(Submission::InFlight) => json!({"status": "ok", "submission": "in_flight"}),
        Ok(Submission::Unknown) => json!({"status": "ok", "submission": "unknown"}),
        Ok(Submission::Cancelled) => json!({"status": "ok", "submission": "cancelled"}),
        Err(error) => error_json(*error),
    }
}

fn error_json(error: DurableTextError) -> Value {
    match error {
        DurableTextError::Refused(error) => {
            json!({"status": "refused", "submission": "refused", "reason": error.label()})
        }
        DurableTextError::Conflict => json!({"status": "refused", "submission": "conflict"}),
        DurableTextError::Unauthorized => {
            json!({"status": "refused", "submission": "unauthorized"})
        }
        DurableTextError::UnknownOperation => {
            json!({"status": "refused", "submission": "unknown_operation"})
        }
        DurableTextError::Store(cause) => json!({
            "status": "unavailable",
            "submission": "store_failed",
            "cause": cause.label(),
        }),
    }
}

/// The runtime a probe needs: the store, a sink to log to, and a process owner.
fn probe_runtime() -> Result<(Runtime, Arc<dyn DiagnosticSink>, AttemptOwner), Outcome> {
    let runtime = current().ok_or_else(|| unavailable(OFF_REASON))?;
    let sink = runtime
        .sink
        .clone()
        .ok_or_else(|| unavailable(NO_LOGGING_REASON))?;
    let owner = process_owner().ok_or_else(|| unavailable(OFF_REASON))?;
    Ok((runtime, sink, owner))
}

/// `POST /dev/text/probe` with `{"key", "scope"?, "outcome"?: "answer"|"refuse"|"deadline"|"lost"}`.
/// A probe writes one attempt row and logs its call. It never calls a provider: the backend is the
/// scripted fixture. An answer comes back as a candidate; `qualify` decides whether it is reusable.
pub(super) async fn probe(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let operation = body_text(&body, "key").ok_or_else(|| bad_request("text probe needs a key"))?;
    let key = parse_key(body_text(&body, "scope"), operation).map_err(bad_request)?;
    let name = body_text(&body, "outcome").unwrap_or("answer");
    let scripted = scripted_outcome(name).ok_or_else(|| {
        bad_request("text probe outcome must be answer, refuse, deadline or lost")
    })?;
    let (runtime, sink, owner) = match probe_runtime() {
        Ok(parts) => parts,
        Err(outcome) => return outcome,
    };
    let service = probe_service(runtime.store, sink, Some(scripted), owner);
    json_ok(submission_json(
        &service.submit(&key, &probe_request()).await,
    ))
}

/// `POST /dev/text/qualify` with `{"key", "scope"?, "verdict": "accept"|"reject"}`: the feature's
/// verdict on a probe candidate. It can only match a probe attempt, because the probe request is
/// fixed and the stored fingerprint must match it.
pub(super) async fn qualify(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let operation =
        body_text(&body, "key").ok_or_else(|| bad_request("text qualify needs a key"))?;
    let key = parse_key(body_text(&body, "scope"), operation).map_err(bad_request)?;
    let verdict = match body_text(&body, "verdict") {
        Some("accept") => Verdict::Accepted,
        Some("reject") => Verdict::Rejected,
        _ => return Err(bad_request("text qualify verdict must be accept or reject")),
    };
    let (runtime, sink, owner) = match probe_runtime() {
        Ok(parts) => parts,
        Err(outcome) => return outcome,
    };
    let service = probe_service(runtime.store, sink, None, owner);
    let result = service.qualify(&key, &probe_request(), verdict).await;
    json_ok(match result {
        Ok(qualification) => json!({
            "status": "ok",
            "qualification": match qualification {
                Qualification::Qualified => "qualified",
                Qualification::Rejected => "rejected",
                Qualification::NotACandidate => "not_a_candidate",
                Qualification::Stale => "stale",
            },
        }),
        Err(error) => error_json(error),
    })
}

/// `POST /dev/text/invalidate` with `{"key", "scope"?}`: withdraws a qualified probe answer so it
/// is never reused. `invalidated: false` means there was no reusable answer.
pub(super) async fn invalidate(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let operation =
        body_text(&body, "key").ok_or_else(|| bad_request("text invalidate needs a key"))?;
    let key = parse_key(body_text(&body, "scope"), operation).map_err(bad_request)?;
    let (runtime, sink, owner) = match probe_runtime() {
        Ok(parts) => parts,
        Err(outcome) => return outcome,
    };
    let service = probe_service(runtime.store, sink, None, owner);
    json_ok(match service.invalidate(&key, &probe_request()).await {
        Ok(invalidated) => json!({"status": "ok", "invalidated": invalidated}),
        Err(error) => error_json(error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_ai::text_fixture::MemoryAttemptStore;
    use df_observe::{DiagnosticRecord, TelemetryError};

    /// Keeps every record whole so a test can search its names and values.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<DiagnosticRecord>>);

    impl DiagnosticSink for Recorder {
        fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
            self.0.lock().unwrap().push(record);
            Ok(())
        }
    }

    fn key(operation: &str) -> OperationKey {
        parse_key(None, operation).unwrap()
    }

    fn owner() -> AttemptOwner {
        AttemptOwner::new([3; 16]).unwrap()
    }

    /// The probe runs on a current-thread runtime, as the server's handlers do.
    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn service(
        store: &Arc<MemoryAttemptStore>,
        sink: &Arc<Recorder>,
        scripted: Option<TextOutcome>,
    ) -> ProbeService<Arc<MemoryAttemptStore>> {
        probe_service(store.clone(), sink.clone(), scripted, owner())
    }

    #[test]
    fn limits_are_one_to_one_hundred_with_twenty_by_default() {
        assert_eq!(parse_limit(None), Ok(20));
        assert_eq!(parse_limit(Some(&"1".to_owned())), Ok(1));
        assert_eq!(parse_limit(Some(&" 100 ".to_owned())), Ok(100));
        assert!(parse_limit(Some(&"101".to_owned())).is_err());
        assert!(parse_limit(Some(&"0".to_owned())).is_err());
        assert!(parse_limit(Some(&"many".to_owned())).is_err());
    }

    #[test]
    fn phases_and_scopes_are_checked_against_their_closed_sets() {
        assert_eq!(parse_phase("candidate"), Ok(AttemptPhase::Candidate));
        assert!(parse_phase("pending").is_err());
        assert!(parse_scope(Some("")).is_err());
        assert_eq!(parse_scope(None).unwrap().as_str(), "dev");
    }

    #[test]
    fn scripted_outcomes_are_only_the_four_named_ones() {
        for name in ["answer", "refuse", "deadline", "lost"] {
            assert!(scripted_outcome(name).is_some(), "{name}");
        }
        assert!(scripted_outcome("provider").is_none());
    }

    #[test]
    fn a_probe_answer_is_a_candidate_until_it_is_qualified_and_every_call_is_logged() {
        let sink = Arc::new(Recorder::default());
        let store = Arc::new(MemoryAttemptStore::default());
        let scripted = scripted_outcome("answer");
        let first = run(service(&store, &sink, scripted).submit(&key("p-1"), &probe_request()));
        assert!(matches!(
            first,
            Ok(Submission::Candidate {
                replayed: false,
                ..
            })
        ));
        let repeat = run(service(&store, &sink, None).submit(&key("p-1"), &probe_request()));
        assert!(matches!(
            repeat,
            Ok(Submission::Candidate { replayed: true, .. })
        ));
        let qualified = run(service(&store, &sink, None).qualify(
            &key("p-1"),
            &probe_request(),
            Verdict::Accepted,
        ));
        assert_eq!(qualified, Ok(Qualification::Qualified));
        let reused = run(service(&store, &sink, None).submit(&key("p-1"), &probe_request()));
        assert!(matches!(
            reused,
            Ok(Submission::Settled { replayed: true, .. })
        ));
        let records = sink.0.lock().unwrap();
        let names: Vec<&str> = records.iter().map(|record| record.event_name).collect();
        assert_eq!(
            names,
            vec![
                "generation.provider_result",
                "generation.cache",
                "generation.text_qualification",
                "generation.cache",
            ]
        );
    }

    #[test]
    fn no_log_record_carries_the_probe_answer_or_instructions() {
        let sink = Arc::new(Recorder::default());
        let store = Arc::new(MemoryAttemptStore::default());
        let scripted = scripted_outcome("answer");
        run(service(&store, &sink, scripted).submit(&key("p-2"), &probe_request())).unwrap();
        let records = sink.0.lock().unwrap();
        let rendered = format!("{records:?}");
        assert!(!rendered.contains(PROBE_ANSWER));
        assert!(!rendered.contains("Development probe: answer"));
    }

    #[test]
    fn a_lost_probe_is_unknown_and_a_rejected_one_is_never_regenerated() {
        let sink = Arc::new(Recorder::default());
        let store = Arc::new(MemoryAttemptStore::default());
        let lost =
            run(service(&store, &sink, scripted_outcome("lost"))
                .submit(&key("p-3"), &probe_request()));
        assert!(matches!(lost, Ok(Submission::Unknown)));
        let answered = run(service(&store, &sink, scripted_outcome("answer"))
            .submit(&key("p-4"), &probe_request()));
        assert!(matches!(answered, Ok(Submission::Candidate { .. })));
        run(service(&store, &sink, None).qualify(&key("p-4"), &probe_request(), Verdict::Rejected))
            .unwrap();
        let again = run(service(&store, &sink, scripted_outcome("answer"))
            .submit(&key("p-4"), &probe_request()));
        assert!(matches!(again, Ok(Submission::Rejected { .. })));
    }

    #[test]
    fn a_verdict_only_matches_a_probe_attempt() {
        let sink = Arc::new(Recorder::default());
        let store = Arc::new(MemoryAttemptStore::default());
        run(service(&store, &sink, scripted_outcome("answer"))
            .submit(&key("p-5"), &probe_request()))
        .unwrap();
        let mut other = probe_request();
        other.instructions = "A different request.".to_owned();
        let result =
            run(service(&store, &sink, None).qualify(&key("p-5"), &other, Verdict::Accepted));
        assert_eq!(result, Err(DurableTextError::Conflict));
    }

    #[test]
    fn the_operator_view_of_a_submission_never_prints_the_answer() {
        let sink = Arc::new(Recorder::default());
        let store = Arc::new(MemoryAttemptStore::default());
        let result = run(service(&store, &sink, scripted_outcome("answer"))
            .submit(&key("p-6"), &probe_request()));
        let shown = submission_json(&result).to_string();
        assert!(shown.contains("candidate"));
        assert!(!shown.contains(PROBE_ANSWER));
    }
}
