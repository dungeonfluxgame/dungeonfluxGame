//! Durable text attempts on a real PostgreSQL database. Each test creates its own scratch
//! database on the server named by `DUNGEONFLUX_TEXT_PG_PORT` (user `DUNGEONFLUX_TEXT_PG_USER`,
//! default `postgres`) and drops it at the end. Without the port a test reports SKIPPED. The
//! loopback refusal and unreachable-server tests need no server.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use df_ai::{
    text::{
        TextOutcome, TextProfile, TextPurpose, TextReceipt, TextRequest, TextRequestError,
        TextService, TextUsage, UnavailableReason,
    },
    text_attempts::{
        AttemptMeta, AttemptOwner, AttemptPhase, AttemptScope, AttemptState, AttemptStoreError,
        Basis, DurableTextError, DurableTextService, OperationKey, OutcomeKind, Qualification,
        QualifyResult, StoredAttempt, Submission, TextAttemptStore, Verdict, fingerprint,
    },
    text_fixture::{FixedAuthority, ScriptedBackend},
};
use df_persistence::text_attempts::PostgresTextAttempts;
use serde_json::{Value, json};
use tokio_postgres::{Client, Config, NoTls};

type Service = DurableTextService<ScriptedBackend, PostgresTextAttempts, FixedAuthority>;

const BASIS: Basis = [7; 32];
const MOVED: Basis = [9; 32];

fn run<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn scratch_port() -> Option<u16> {
    let port = std::env::var("DUNGEONFLUX_TEXT_PG_PORT").ok()?;
    Some(
        port.parse()
            .expect("DUNGEONFLUX_TEXT_PG_PORT is a port number"),
    )
}

fn skipped(test: &str) {
    eprintln!("SKIPPED {test}: set DUNGEONFLUX_TEXT_PG_PORT to a scratch PostgreSQL server");
}

fn user() -> String {
    std::env::var("DUNGEONFLUX_TEXT_PG_USER").unwrap_or_else(|_| "postgres".to_owned())
}

fn config(port: u16, database: &str) -> Config {
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .user(user())
        .dbname(database);
    config
}

async fn connect(config: &Config) -> Client {
    let (client, connection) = config.connect(NoTls).await.expect("scratch connection");
    tokio::spawn(async move {
        let _ended = connection.await;
    });
    client
}

/// One scratch database for one test, dropped with its sessions at the end.
struct Scratch {
    port: u16,
    database: String,
    admin: Client,
}

impl Scratch {
    async fn create(port: u16, tag: &str) -> Self {
        let admin = connect(&config(port, "postgres")).await;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let database = format!("df_text_{tag}_{nanos}_{}", std::process::id());
        admin
            .batch_execute(&format!("CREATE DATABASE \"{database}\""))
            .await
            .expect("create scratch database");
        Self {
            port,
            database,
            admin,
        }
    }

    fn config(&self) -> Config {
        config(self.port, &self.database)
    }

    async fn store(&self) -> PostgresTextAttempts {
        PostgresTextAttempts::connect(&self.config())
            .await
            .expect("store connects and applies the schema")
    }

    /// A direct session on the scratch database, for what the store itself never does.
    async fn sql(&self) -> Client {
        connect(&self.config()).await
    }

    async fn finish(self) {
        self.admin
            .batch_execute(&format!(
                "DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)",
                self.database
            ))
            .await
            .expect("drop scratch database");
    }
}

fn owner(value: u8) -> AttemptOwner {
    AttemptOwner::new([value; 16]).expect("non-zero owner")
}

fn scope() -> AttemptScope {
    AttemptScope::new("local").expect("valid scope")
}

fn key(operation: &str) -> OperationKey {
    OperationKey::new(&scope(), operation).expect("valid key")
}

fn request() -> TextRequest {
    TextRequest {
        purpose: TextPurpose::Storyteller,
        profile: TextProfile::new("profile-a").expect("valid profile"),
        instructions: "Narrate one line.".to_owned(),
        facts: json!({"room": "harbor"}),
        output_name: "narration".to_owned(),
        output_schema: json!({"type": "object", "properties": {"line": {"type": "string"}}}),
        deadline: Duration::from_secs(30),
    }
}

fn answer() -> TextOutcome {
    TextOutcome::Answered(answer_value())
}

fn answer_value() -> Value {
    json!({"line": "The tide turns."})
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

async fn make_service(
    store: PostgresTextAttempts,
    receipts: Vec<TextReceipt>,
    owner_value: u8,
    authority: FixedAuthority,
) -> (Service, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let backend = ScriptedBackend::new(receipts, calls.clone());
    (
        DurableTextService::new(
            TextService::new(backend),
            store,
            authority,
            owner(owner_value),
        ),
        calls,
    )
}

fn holding() -> FixedAuthority {
    FixedAuthority::holding(BASIS)
}

fn answered() -> Vec<TextReceipt> {
    vec![answer().into()]
}

/// Admits `operation` for `request` directly, as an earlier process would have.
async fn admit(store: &PostgresTextAttempts, operation: &str, request: &TextRequest) {
    let fingerprint = fingerprint(request);
    let meta = AttemptMeta {
        purpose: request.purpose,
        profile: &request.profile,
        output_name: &request.output_name,
        fingerprint: &fingerprint,
    };
    store
        .admit(&key(operation), &meta)
        .await
        .expect("admission");
}

#[test]
fn a_remote_store_host_is_refused_before_any_connection() {
    run(async {
        let mut remote = Config::new();
        remote.host("example.com").port(5432).user("postgres");
        assert!(matches!(
            PostgresTextAttempts::connect(&remote).await,
            Err(AttemptStoreError::Invalid)
        ));
    });
}

#[test]
fn a_loopback_name_with_a_remote_hostaddr_or_a_resolved_name_is_refused() {
    run(async {
        let mut redirected = Config::new();
        redirected
            .host("127.0.0.1")
            .hostaddr("10.0.0.5".parse().unwrap())
            .port(5432)
            .user("postgres");
        assert!(matches!(
            PostgresTextAttempts::connect(&redirected).await,
            Err(AttemptStoreError::Invalid)
        ));
        let mut named = Config::new();
        named.host("localhost").port(5432).user("postgres");
        assert!(matches!(
            PostgresTextAttempts::connect(&named).await,
            Err(AttemptStoreError::Invalid)
        ));
    });
}

#[test]
fn a_loopback_server_that_is_not_there_is_unreachable_not_invalid() {
    run(async {
        let mut closed = Config::new();
        closed.host("127.0.0.1").port(9).user("postgres");
        assert!(matches!(
            PostgresTextAttempts::connect(&closed).await,
            Err(AttemptStoreError::Unreachable | AttemptStoreError::TimedOut)
        ));
    });
}

#[test]
fn a_qualified_answer_survives_a_new_connection_and_replays_without_a_send() {
    let Some(port) = scratch_port() else {
        return skipped("a_qualified_answer_survives_a_new_connection_and_replays_without_a_send");
    };
    run(async move {
        let scratch = Scratch::create(port, "replay").await;
        let (first, first_calls) =
            make_service(scratch.store().await, answered(), 1, holding()).await;
        assert_eq!(
            first.submit(&key("turn-1"), &request()).await,
            Ok(candidate(false))
        );
        assert_eq!(
            first
                .qualify(&key("turn-1"), &request(), Verdict::Accepted)
                .await,
            Ok(Qualification::Qualified)
        );
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        drop(first);

        let (second, second_calls) =
            make_service(scratch.store().await, vec![], 2, holding()).await;
        assert_eq!(
            second.submit(&key("turn-1"), &request()).await,
            Ok(settled(answer(), true))
        );
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);
        drop(second);
        scratch.finish().await;
    });
}

#[test]
fn an_unqualified_candidate_stays_a_candidate_across_a_restart() {
    let Some(port) = scratch_port() else {
        return skipped("an_unqualified_candidate_stays_a_candidate_across_a_restart");
    };
    run(async move {
        let scratch = Scratch::create(port, "candidate").await;
        let (first, _) = make_service(scratch.store().await, answered(), 1, holding()).await;
        assert_eq!(
            first.submit(&key("turn-1"), &request()).await,
            Ok(candidate(false))
        );
        drop(first);
        let (second, calls) = make_service(scratch.store().await, vec![], 2, holding()).await;
        assert_eq!(
            second.submit(&key("turn-1"), &request()).await,
            Ok(candidate(true))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        drop(second);
        scratch.finish().await;
    });
}

#[test]
fn a_rejected_candidate_stays_rejected_and_loses_its_answer_text() {
    let Some(port) = scratch_port() else {
        return skipped("a_rejected_candidate_stays_rejected_and_loses_its_answer_text");
    };
    run(async move {
        let scratch = Scratch::create(port, "reject").await;
        let (first, _) = make_service(scratch.store().await, answered(), 1, holding()).await;
        first.submit(&key("turn-1"), &request()).await.unwrap();
        assert_eq!(
            first
                .qualify(&key("turn-1"), &request(), Verdict::Rejected)
                .await,
            Ok(Qualification::Rejected)
        );
        drop(first);
        let (second, calls) = make_service(scratch.store().await, answered(), 2, holding()).await;
        assert_eq!(
            second.submit(&key("turn-1"), &request()).await,
            Ok(Submission::Rejected { usage: None })
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let stored: Option<String> = scratch
            .sql()
            .await
            .query_one("SELECT answer FROM df_text.text_attempt", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(stored, None);
        drop(second);
        scratch.finish().await;
    });
}

#[test]
fn a_qualified_answer_can_be_withdrawn_and_is_then_rejected() {
    let Some(port) = scratch_port() else {
        return skipped("a_qualified_answer_can_be_withdrawn_and_is_then_rejected");
    };
    run(async move {
        let scratch = Scratch::create(port, "invalidate").await;
        let (service, _) = make_service(scratch.store().await, answered(), 1, holding()).await;
        service.submit(&key("turn-1"), &request()).await.unwrap();
        service
            .qualify(&key("turn-1"), &request(), Verdict::Accepted)
            .await
            .unwrap();
        assert_eq!(
            service.invalidate(&key("turn-1"), &request()).await,
            Ok(true)
        );
        assert_eq!(
            service.invalidate(&key("turn-1"), &request()).await,
            Ok(false)
        );
        assert_eq!(
            service.submit(&key("turn-1"), &request()).await,
            Ok(Submission::Rejected { usage: None })
        );
        drop(service);
        scratch.finish().await;
    });
}

#[test]
fn a_verdict_after_the_basis_moved_discards_the_candidate_in_one_step() {
    let Some(port) = scratch_port() else {
        return skipped("a_verdict_after_the_basis_moved_discards_the_candidate_in_one_step");
    };
    run(async move {
        let scratch = Scratch::create(port, "moved").await;
        // Calls: submit start, before dispatch, after the backend, then the verdict.
        let authority =
            FixedAuthority::sequence([Some(BASIS), Some(BASIS), Some(BASIS), Some(MOVED)]);
        let usage = TextUsage {
            total_tokens: 30,
            input_tokens: Some(20),
            output_tokens: Some(10),
            cached_input_tokens: None,
            cache_write_tokens: None,
            reasoning_output_tokens: None,
        };
        let receipts = vec![TextReceipt {
            outcome: answer(),
            usage: Some(usage),
        }];
        let (service, _) = make_service(scratch.store().await, receipts, 1, authority).await;
        service.submit(&key("turn-1"), &request()).await.unwrap();
        assert_eq!(
            service
                .qualify(&key("turn-1"), &request(), Verdict::Accepted)
                .await,
            Ok(Qualification::Stale)
        );
        assert_eq!(
            service.submit(&key("turn-1"), &request()).await,
            Ok(Submission::Settled {
                outcome: TextOutcome::Stale,
                replayed: true,
                usage: Some(usage),
            })
        );
        drop(service);
        scratch.finish().await;
    });
}

#[test]
fn a_reused_key_with_a_different_request_or_profile_is_refused() {
    let Some(port) = scratch_port() else {
        return skipped("a_reused_key_with_a_different_request_or_profile_is_refused");
    };
    run(async move {
        let scratch = Scratch::create(port, "conflict").await;
        let (service, calls) = make_service(scratch.store().await, answered(), 1, holding()).await;
        service.submit(&key("turn-1"), &request()).await.unwrap();
        let mut other = request();
        other.instructions = "Narrate another line.".to_owned();
        assert_eq!(
            service.submit(&key("turn-1"), &other).await,
            Err(DurableTextError::Conflict)
        );
        let mut other = request();
        other.profile = TextProfile::new("profile-b").unwrap();
        assert_eq!(
            service.submit(&key("turn-1"), &other).await,
            Err(DurableTextError::Conflict)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(service);
        scratch.finish().await;
    });
}

#[test]
fn the_same_operation_in_another_scope_is_a_separate_attempt() {
    let Some(port) = scratch_port() else {
        return skipped("the_same_operation_in_another_scope_is_a_separate_attempt");
    };
    run(async move {
        let scratch = Scratch::create(port, "scopes").await;
        let receipts = vec![answer().into(), answer().into()];
        let (service, calls) = make_service(scratch.store().await, receipts, 1, holding()).await;
        let other = OperationKey::new(&AttemptScope::new("campaign-2").unwrap(), "turn-1").unwrap();
        assert_eq!(
            service.submit(&key("turn-1"), &request()).await,
            Ok(candidate(false))
        );
        assert_eq!(
            service.submit(&other, &request()).await,
            Ok(candidate(false))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        drop(service);
        scratch.finish().await;
    });
}

#[test]
fn a_refused_request_leaves_no_attempt_behind() {
    let Some(port) = scratch_port() else {
        return skipped("a_refused_request_leaves_no_attempt_behind");
    };
    run(async move {
        let scratch = Scratch::create(port, "refused").await;
        let (service, calls) = make_service(scratch.store().await, answered(), 1, holding()).await;
        let mut bad = request();
        bad.instructions = " ".to_owned();
        assert_eq!(
            service.submit(&key("turn-1"), &bad).await,
            Err(DurableTextError::Refused(
                TextRequestError::EmptyInstructions
            ))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(scratch.store().await.lookup(&key("turn-1")).await, Ok(None));
        drop(service);
        scratch.finish().await;
    });
}

#[test]
fn a_send_left_behind_by_a_crashed_process_is_unknown_and_never_resent() {
    let Some(port) = scratch_port() else {
        return skipped("a_send_left_behind_by_a_crashed_process_is_unknown_and_never_resent");
    };
    run(async move {
        let scratch = Scratch::create(port, "crash").await;
        // The crashed process admitted, wrote its dispatch marker and then died before a result.
        let crashed = scratch.store().await;
        admit(&crashed, "turn-1", &request()).await;
        assert_eq!(
            crashed
                .begin_dispatch(&key("turn-1"), owner(7), BASIS)
                .await,
            Ok(true)
        );
        drop(crashed);

        let (restarted, calls) =
            make_service(scratch.store().await, answered(), 2, holding()).await;
        assert_eq!(
            restarted.submit(&key("turn-1"), &request()).await,
            Ok(Submission::Unknown)
        );
        assert_eq!(
            restarted.submit(&key("turn-1"), &request()).await,
            Ok(Submission::Unknown)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        drop(restarted);
        scratch.finish().await;
    });
}

#[test]
fn a_backend_that_cannot_say_whether_it_sent_is_stored_as_unknown() {
    let Some(port) = scratch_port() else {
        return skipped("a_backend_that_cannot_say_whether_it_sent_is_stored_as_unknown");
    };
    run(async move {
        let scratch = Scratch::create(port, "lost").await;
        let (first, calls) = make_service(
            scratch.store().await,
            vec![TextOutcome::Unknown.into()],
            1,
            holding(),
        )
        .await;
        assert_eq!(
            first.submit(&key("turn-1"), &request()).await,
            Ok(Submission::Unknown)
        );
        drop(first);
        let (second, second_calls) =
            make_service(scratch.store().await, answered(), 2, holding()).await;
        assert_eq!(
            second.submit(&key("turn-1"), &request()).await,
            Ok(Submission::Unknown)
        );
        assert_eq!(
            calls.load(Ordering::SeqCst) + second_calls.load(Ordering::SeqCst),
            1
        );
        drop(second);
        scratch.finish().await;
    });
}

#[test]
fn a_cancel_before_dispatch_is_durable_and_prevents_the_send() {
    let Some(port) = scratch_port() else {
        return skipped("a_cancel_before_dispatch_is_durable_and_prevents_the_send");
    };
    run(async move {
        let scratch = Scratch::create(port, "cancel").await;
        let admitting = scratch.store().await;
        admit(&admitting, "turn-1", &request()).await;
        drop(admitting);
        let (service, calls) = make_service(scratch.store().await, answered(), 1, holding()).await;
        assert_eq!(service.cancel(&key("turn-1")).await, Ok(true));
        drop(service);
        let (later, later_calls) =
            make_service(scratch.store().await, answered(), 2, holding()).await;
        assert_eq!(
            later.submit(&key("turn-1"), &request()).await,
            Ok(Submission::Cancelled)
        );
        assert_eq!(
            calls.load(Ordering::SeqCst) + later_calls.load(Ordering::SeqCst),
            0
        );
        drop(later);
        scratch.finish().await;
    });
}

#[test]
fn terminal_phases_never_change_again() {
    let Some(port) = scratch_port() else {
        return skipped("terminal_phases_never_change_again");
    };
    run(async move {
        let scratch = Scratch::create(port, "terminal").await;
        let store = scratch.store().await;
        let turn = key("turn-1");
        admit(&store, "turn-1", &request()).await;
        assert_eq!(store.begin_dispatch(&turn, owner(1), BASIS).await, Ok(true));
        assert_eq!(
            store.settle(&turn, &TextOutcome::Deadline, None).await,
            Ok(true)
        );
        assert_eq!(store.mark_unknown(&turn).await, Ok(false));
        assert_eq!(store.cancel(&turn).await, Ok(false));
        assert_eq!(store.invalidate(&turn).await, Ok(false));
        assert_eq!(
            store.begin_dispatch(&turn, owner(2), BASIS).await,
            Ok(false)
        );
        assert_eq!(
            store.qualify(&turn, Verdict::Accepted, Some(BASIS)).await,
            Ok(QualifyResult::NotCandidate)
        );
        assert_eq!(
            store.lookup(&turn).await,
            Ok(Some(StoredAttempt {
                fingerprint: fingerprint(&request()),
                state: AttemptState::Settled {
                    outcome: TextOutcome::Deadline,
                    usage: None,
                },
            }))
        );
        drop(store);
        scratch.finish().await;
    });
}

#[test]
fn the_store_refuses_to_settle_an_answer_or_an_unknown_send_directly() {
    let Some(port) = scratch_port() else {
        return skipped("the_store_refuses_to_settle_an_answer_or_an_unknown_send_directly");
    };
    run(async move {
        let scratch = Scratch::create(port, "direct").await;
        let store = scratch.store().await;
        admit(&store, "turn-1", &request()).await;
        store
            .begin_dispatch(&key("turn-1"), owner(1), BASIS)
            .await
            .unwrap();
        assert_eq!(
            store.settle(&key("turn-1"), &answer(), None).await,
            Err(AttemptStoreError::Invalid)
        );
        assert_eq!(
            store
                .settle(&key("turn-1"), &TextOutcome::Unknown, None)
                .await,
            Err(AttemptStoreError::Invalid)
        );
        drop(store);
        scratch.finish().await;
    });
}

#[test]
fn a_backend_refusal_is_stored_and_replayed_as_the_same_refusal() {
    let Some(port) = scratch_port() else {
        return skipped("a_backend_refusal_is_stored_and_replayed_as_the_same_refusal");
    };
    run(async move {
        let scratch = Scratch::create(port, "refusal").await;
        let refusal = TextOutcome::Unavailable(UnavailableReason::Authentication);
        let (first, calls) = make_service(
            scratch.store().await,
            vec![refusal.clone().into()],
            1,
            holding(),
        )
        .await;
        assert_eq!(
            first.submit(&key("turn-1"), &request()).await,
            Ok(settled(refusal.clone(), false))
        );
        drop(first);
        let (second, second_calls) =
            make_service(scratch.store().await, vec![], 2, holding()).await;
        assert_eq!(
            second.submit(&key("turn-1"), &request()).await,
            Ok(settled(refusal, true))
        );
        assert_eq!(
            calls.load(Ordering::SeqCst) + second_calls.load(Ordering::SeqCst),
            1
        );
        drop(second);
        scratch.finish().await;
    });
}

#[test]
fn usage_is_stored_and_read_back_with_unreported_categories_unknown() {
    let Some(port) = scratch_port() else {
        return skipped("usage_is_stored_and_read_back_with_unreported_categories_unknown");
    };
    run(async move {
        let scratch = Scratch::create(port, "usage").await;
        let usage = TextUsage {
            total_tokens: 120,
            input_tokens: Some(80),
            output_tokens: Some(40),
            cached_input_tokens: None,
            cache_write_tokens: None,
            reasoning_output_tokens: Some(12),
        };
        let receipts = vec![TextReceipt {
            outcome: answer(),
            usage: Some(usage),
        }];
        let (first, _) = make_service(scratch.store().await, receipts, 1, holding()).await;
        first.submit(&key("turn-1"), &request()).await.unwrap();
        first
            .qualify(&key("turn-1"), &request(), Verdict::Accepted)
            .await
            .unwrap();
        drop(first);
        assert_eq!(
            scratch.store().await.lookup(&key("turn-1")).await,
            Ok(Some(StoredAttempt {
                fingerprint: fingerprint(&request()),
                state: AttemptState::Settled {
                    outcome: answer(),
                    usage: Some(usage),
                },
            }))
        );
        scratch.finish().await;
    });
}

#[test]
fn recent_lists_newest_first_within_the_limit_and_filters_by_scope_and_phase() {
    let Some(port) = scratch_port() else {
        return skipped(
            "recent_lists_newest_first_within_the_limit_and_filters_by_scope_and_phase",
        );
    };
    run(async move {
        let scratch = Scratch::create(port, "recent").await;
        let store = scratch.store().await;
        for operation in ["turn-1", "turn-2", "turn-3"] {
            admit(&store, operation, &request()).await;
        }
        store.cancel(&key("turn-2")).await.unwrap();
        let sql = scratch.sql().await;
        sql.batch_execute(
            "UPDATE df_text.text_attempt SET updated_at = now() - interval '2 hours' \
             WHERE operation_key = 'turn-1'; \
             UPDATE df_text.text_attempt SET updated_at = now() - interval '1 hour' \
             WHERE operation_key = 'turn-2'",
        )
        .await
        .unwrap();
        let all = store.recent(None, None, 10).await.unwrap();
        let keys: Vec<&str> = all.iter().map(|row| row.operation_key.as_str()).collect();
        assert_eq!(keys, vec!["turn-3", "turn-2", "turn-1"]);
        assert_eq!(store.recent(None, None, 2).await.unwrap().len(), 2);
        let cancelled = store
            .recent(None, Some(AttemptPhase::Cancelled), 10)
            .await
            .unwrap();
        assert_eq!(cancelled.len(), 1);
        assert_eq!(cancelled[0].operation_key, "turn-2");
        assert_eq!(cancelled[0].purpose, TextPurpose::Storyteller);
        assert_eq!(cancelled[0].profile, "profile-a");
        let elsewhere = AttemptScope::new("campaign-2").unwrap();
        assert!(
            store
                .recent(Some(&elsewhere), None, 10)
                .await
                .unwrap()
                .is_empty()
        );
        drop((store, sql));
        scratch.finish().await;
    });
}

#[test]
fn recent_refuses_a_bad_limit_before_any_query() {
    let Some(port) = scratch_port() else {
        return skipped("recent_refuses_a_bad_limit_before_any_query");
    };
    run(async move {
        let scratch = Scratch::create(port, "limit").await;
        let store = scratch.store().await;
        scratch.finish().await;
        // The database is gone, so only a refusal that never queried can still answer Invalid.
        assert_eq!(
            store.recent(None, None, 0).await,
            Err(AttemptStoreError::Invalid)
        );
        assert_eq!(
            store.recent(None, None, 101).await,
            Err(AttemptStoreError::Invalid)
        );
    });
}

#[test]
fn an_attempt_summary_carries_sizes_and_usage_but_never_the_answer() {
    let Some(port) = scratch_port() else {
        return skipped("an_attempt_summary_carries_sizes_and_usage_but_never_the_answer");
    };
    run(async move {
        let scratch = Scratch::create(port, "summary").await;
        let usage = TextUsage {
            total_tokens: 55,
            input_tokens: None,
            output_tokens: None,
            cached_input_tokens: None,
            cache_write_tokens: None,
            reasoning_output_tokens: None,
        };
        let receipts = vec![TextReceipt {
            outcome: answer(),
            usage: Some(usage),
        }];
        let (service, _) = make_service(scratch.store().await, receipts, 1, holding()).await;
        assert_eq!(
            scratch.store().await.attempt(&key("turn-9")).await,
            Ok(None)
        );
        service.submit(&key("turn-1"), &request()).await.unwrap();
        let summary = scratch
            .store()
            .await
            .attempt(&key("turn-1"))
            .await
            .unwrap()
            .expect("the attempt exists");
        assert_eq!(summary.phase, AttemptPhase::Candidate);
        assert_eq!(summary.outcome, None);
        assert_eq!(summary.answer_bytes, Some(answer_value().to_string().len()));
        assert_eq!(summary.usage_total, Some(55));
        assert!(!format!("{summary:?}").contains("tide"));
        service
            .qualify(&key("turn-1"), &request(), Verdict::Accepted)
            .await
            .unwrap();
        let qualified = scratch
            .store()
            .await
            .attempt(&key("turn-1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(qualified.phase, AttemptPhase::Settled);
        assert_eq!(qualified.outcome, Some(OutcomeKind::Answered));
        drop(service);
        scratch.finish().await;
    });
}

#[test]
fn prune_deletes_old_finished_attempts_and_keeps_everything_that_may_still_matter() {
    let Some(port) = scratch_port() else {
        return skipped(
            "prune_deletes_old_finished_attempts_and_keeps_everything_that_may_still_matter",
        );
    };
    run(async move {
        let scratch = Scratch::create(port, "prune").await;
        let store = scratch.store().await;
        for operation in ["cancelled", "unknown", "candidate", "fresh"] {
            admit(&store, operation, &request()).await;
        }
        store.cancel(&key("cancelled")).await.unwrap();
        store
            .begin_dispatch(&key("unknown"), owner(1), BASIS)
            .await
            .unwrap();
        store.mark_unknown(&key("unknown")).await.unwrap();
        store
            .begin_dispatch(&key("candidate"), owner(1), BASIS)
            .await
            .unwrap();
        store
            .record_candidate(&key("candidate"), &answer_value(), None)
            .await
            .unwrap();
        store.cancel(&key("fresh")).await.unwrap();
        scratch
            .sql()
            .await
            .batch_execute(
                "UPDATE df_text.text_attempt SET updated_at = now() - interval '40 days' \
                 WHERE operation_key <> 'fresh'",
            )
            .await
            .unwrap();
        assert_eq!(store.prune(30).await, Ok(1));
        assert_eq!(store.attempt(&key("cancelled")).await, Ok(None));
        for kept in ["unknown", "candidate", "fresh"] {
            assert!(
                store.attempt(&key(kept)).await.unwrap().is_some(),
                "{kept} was kept"
            );
        }
        drop(store);
        scratch.finish().await;
    });
}
