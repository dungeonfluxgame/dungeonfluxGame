//! Ingest tests over scratch PostgreSQL (`supportingest_` databases, dropped after).
//! Fakes only: FakeMailSource + FakeJevJudge; no live provider calls.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use df_support::{
    contracts::{MessagePageRequest, TicketId},
    fakes::{FakeJevJudge, FakeMailSource},
    ingest::{InboundMail, MailPage, MailSourceError},
    ports::SupportStore as _,
};
use tokio_postgres::{Config, NoTls};

use super::{IngestConfig, IngestError, SupportIngest};

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);
const MIGRATION: &str = include_str!("../../../df-persistence/migrations/0027_support_inbox.sql");
const GOOD_JUDGEMENT: &str = r#"{"category":"billing","urgency":"high","confidence":0.95,"suggested_action":{"kind":"refund_review","summary":"Review the duplicate charge refund"}}"#;

struct Database {
    name: String,
    admin: Option<tokio_postgres::Client>,
    inspect: Option<tokio_postgres::Client>,
    store: Option<Arc<df_persistence::support_store::SupportStore>>,
    state: Arc<super::state::IngestState>,
    drivers: Vec<tokio::task::JoinHandle<()>>,
}

impl Database {
    async fn new() -> Self {
        let port: u16 = std::env::var("DUNGEONFLUX_SUPPORT_PG_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(55911);
        let mut admin_config = Config::new();
        admin_config
            .host("127.0.0.1")
            .port(port)
            .user("postgres")
            .dbname("postgres");
        let (admin, admin_connection) = admin_config
            .connect(NoTls)
            .await
            .expect("scratch PostgreSQL 55911 must be reachable");
        let mut drivers = vec![tokio::spawn(async move {
            let _ = admin_connection.await;
        })];
        let id = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "supportingest_{}_{}_{}",
            std::process::id(),
            now_micros(),
            id
        );
        admin
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .expect("create scratch database");
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user("postgres")
            .dbname(&name);
        let (client, connection) = config.connect(NoTls).await.expect("scratch connection");
        drivers.push(tokio::spawn(async move {
            let _ = connection.await;
        }));
        client
            .batch_execute(MIGRATION)
            .await
            .expect("support migration");
        let (inspect, inspect_connection) =
            config.connect(NoTls).await.expect("scratch connection");
        drivers.push(tokio::spawn(async move {
            let _ = inspect_connection.await;
        }));
        let (state_client, state_connection) =
            config.connect(NoTls).await.expect("scratch connection");
        drivers.push(tokio::spawn(async move {
            let _ = state_connection.await;
        }));
        Self {
            state: Arc::new(super::state::IngestState::new(state_client)),
            name,
            admin: Some(admin),
            inspect: Some(inspect),
            store: Some(Arc::new(df_persistence::support_store::SupportStore::new(
                client,
            ))),
            drivers,
        }
    }

    async fn classification_status(&self, ticket: &str) -> Option<String> {
        let admin = self.inspect.as_ref()?;
        let row = admin
            .query_opt(
                "SELECT data->>'status' FROM support.classification WHERE ticket_id=$1",
                &[&ticket],
            )
            .await
            .ok()??;
        row.get(0)
    }

    async fn cleanup(&mut self) {
        self.store.take();
        if let Some(admin) = self.admin.take() {
            admin
                .batch_execute(&format!(
                    "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                    self.name
                ))
                .await
                .expect("drop owned scratch database");
        }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        for driver in &self.drivers {
            driver.abort();
        }
    }
}

fn now_micros() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0)
}

fn mail(id: &str, thread: &str) -> InboundMail {
    InboundMail {
        provider_message_id: id.into(),
        provider_thread_id: thread.into(),
        received_at_unix_s: 1_700_000_000,
        sender_address: "ada@example.invalid".into(),
        sender_display_name: Some("Ada".into()),
        recipient_addresses: vec!["support@dungeonfluxdnd.com".into()],
        subject: "Charged twice".into(),
        body_text: "I was charged twice for my subscription.".into(),
        intake_incomplete: false,
        direction: df_support::contracts::MessageDirection::Inbound,
        labels: vec!["INBOX".into()],
    }
}

fn page(mail: InboundMail) -> MailPage {
    MailPage {
        messages: vec![mail],
        next_cursor: None,
    }
}

fn worker(
    source: FakeMailSource,
    judge: FakeJevJudge,
    store: Arc<df_persistence::support_store::SupportStore>,
    state: Arc<super::state::IngestState>,
) -> SupportIngest<FakeMailSource, FakeJevJudge> {
    SupportIngest::new(
        source,
        judge,
        store,
        "mailbox-support".into(),
        "support@dungeonfluxdnd.com".into(),
        IngestConfig::default(),
        super::State::Ready(state),
    )
}

#[tokio::test]
async fn new_message_is_queued_and_classified_once() {
    let mut db = Database::new().await;
    let store = db.store.clone().expect("store present");
    let judge = FakeJevJudge::new(vec![Ok(GOOD_JUDGEMENT.into())]);
    let mut ingest = worker(
        FakeMailSource::new(vec![Ok(page(mail("gmail-1", "thr-1")))]),
        judge,
        store.clone(),
        db.state.clone(),
    );
    let report = ingest
        .run_once(1_700_000_100)
        .await
        .expect("ingest succeeds");
    assert_eq!(report.inserted, 1);
    assert_eq!(report.classified, 1);
    assert_eq!(report.unclassified, 0);
    assert_eq!(ingest.judge.calls(), 1);
    let ticket = store
        .get_ticket(
            &TicketId::new(super::identifier("ticket", "mailbox-support", "thr-1"))
                .expect("ticket id"),
        )
        .await
        .expect("ticket queued");
    assert_eq!(ticket.status, df_support::contracts::TicketStatus::New);
    let messages = store
        .list_messages(
            &ticket.id,
            MessagePageRequest::new(None, 10).expect("page request"),
        )
        .await
        .expect("messages listed");
    assert_eq!(messages.messages.len(), 1);
    assert_eq!(
        db.classification_status(&super::identifier("ticket", "mailbox-support", "thr-1"))
            .await
            .as_deref(),
        Some("classified")
    );
    db.cleanup().await;
}

#[tokio::test]
async fn replay_does_not_duplicate_or_reclassify() {
    let mut db = Database::new().await;
    let store = db.store.clone().expect("store present");
    let judge = FakeJevJudge::new(vec![Ok(GOOD_JUDGEMENT.into())]);
    let mut ingest = worker(
        FakeMailSource::new(vec![
            Ok(page(mail("gmail-1", "thr-1"))),
            Ok(page(mail("gmail-1", "thr-1"))),
        ]),
        judge,
        store.clone(),
        db.state.clone(),
    );
    let first = ingest.run_once(1_700_000_100).await.expect("first ingest");
    assert_eq!(first.inserted, 1);
    let second = ingest.run_once(1_700_000_200).await.expect("replay ingest");
    assert_eq!(second.inserted, 0);
    assert_eq!(second.duplicates, 1);
    assert_eq!(second.classified, 0);
    assert_eq!(ingest.judge.calls(), 1);
    let ticket_id =
        TicketId::new(super::identifier("ticket", "mailbox-support", "thr-1")).expect("ticket id");
    let messages = store
        .list_messages(
            &ticket_id,
            MessagePageRequest::new(None, 10).expect("page request"),
        )
        .await
        .expect("messages listed");
    assert_eq!(messages.messages.len(), 1);
    assert_eq!(
        db.classification_status(&super::identifier("ticket", "mailbox-support", "thr-1"))
            .await
            .as_deref(),
        Some("classified")
    );
    db.cleanup().await;
}

#[tokio::test]
async fn provider_error_is_retried_and_not_lost() {
    let mut db = Database::new().await;
    let store = db.store.clone().expect("store present");
    let mut exhausted = worker(
        FakeMailSource::new(vec![
            Err(MailSourceError::Unavailable),
            Err(MailSourceError::Unavailable),
            Err(MailSourceError::Unavailable),
            Ok(page(mail("gmail-9", "thr-9"))),
        ]),
        FakeJevJudge::new(vec![Ok(GOOD_JUDGEMENT.into())]),
        store.clone(),
        db.state.clone(),
    );
    // Bounded attempts are exhausted: the slice fails without losing the page.
    assert_eq!(
        exhausted.run_once(1_700_000_100).await,
        Err(IngestError::SourceUnavailable)
    );
    assert_eq!(
        exhausted.run_once(1_700_000_101).await,
        Err(IngestError::Deferred)
    );
    assert_eq!(
        exhausted.run_once(1_700_000_200).await,
        Err(IngestError::SourceUnavailable)
    );
    assert_eq!(
        exhausted.run_once(1_700_000_400).await,
        Err(IngestError::SourceUnavailable)
    );
    // Later scheduled slices retry the same durable cursor after bounded backoff.
    let report = exhausted
        .run_once(1_700_001_000)
        .await
        .expect("retry succeeds");
    assert_eq!(report.inserted, 1);
    assert_eq!(report.classified, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn malformed_model_output_queues_as_unclassified() {
    let mut db = Database::new().await;
    let store = db.store.clone().expect("store present");
    let mut ingest = worker(
        FakeMailSource::new(vec![Ok(page(mail("gmail-2", "thr-2")))]),
        FakeJevJudge::new(vec![Ok("this is not classification json".into())]),
        store.clone(),
        db.state.clone(),
    );
    let report = ingest
        .run_once(1_700_000_100)
        .await
        .expect("ingest succeeds");
    assert_eq!(report.inserted, 1);
    assert_eq!(report.classified, 0);
    assert_eq!(report.unclassified, 1);
    let ticket_id =
        TicketId::new(super::identifier("ticket", "mailbox-support", "thr-2")).expect("ticket id");
    store
        .get_ticket(&ticket_id)
        .await
        .expect("ticket stays queued");
    assert_eq!(
        db.classification_status(&super::identifier("ticket", "mailbox-support", "thr-2"))
            .await
            .as_deref(),
        Some("unclassified")
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_restart_resumes_the_durable_cursor_without_reclassifying_mail() {
    let mut db = Database::new().await;
    let store = db.store.clone().unwrap();
    let mut first = worker(
        FakeMailSource::new(vec![Ok(MailPage {
            messages: vec![mail("1", "thread")],
            next_cursor: Some("continuation".into()),
        })]),
        FakeJevJudge::new(vec![Ok(GOOD_JUDGEMENT.into())]),
        store.clone(),
        db.state.clone(),
    );
    first.run_once(1000).await.unwrap();
    drop(first);
    let mut restarted = worker(
        FakeMailSource::new(vec![Ok(page(mail("1", "thread")))]),
        FakeJevJudge::new(Vec::new()),
        store,
        db.state.clone(),
    );
    assert_eq!(restarted.run_once(1100).await.unwrap().duplicates, 1);
    assert_eq!(
        restarted.source.cursors.lock().unwrap().as_slice(),
        [Some("continuation".into())]
    );
    assert_eq!(restarted.judge.calls(), 0);
    db.cleanup().await;
}

#[tokio::test]
async fn distinct_sender_addresses_do_not_collapse_into_one_party() {
    let mut db = Database::new().await;
    let mut first = mail("1", "thread");
    first.sender_address = "ada+tag@example.invalid".into();
    let mut second = mail("2", "thread");
    second.sender_address = "ada-tag@example.invalid".into();
    let mut ingest = worker(
        FakeMailSource::new(vec![Ok(MailPage {
            messages: vec![first, second],
            next_cursor: None,
        })]),
        FakeJevJudge::new(vec![Ok(GOOD_JUDGEMENT.into()); 2]),
        db.store.clone().unwrap(),
        db.state.clone(),
    );
    assert_eq!(ingest.run_once(1000).await.unwrap().classified, 2);
    let count: i64 = db
        .inspect
        .as_ref()
        .unwrap()
        .query_one(
            "SELECT count(DISTINCT sender_party_id) FROM support.message",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 2);
    let revisions: Vec<i64> = db
        .inspect
        .as_ref()
        .unwrap()
        .query(
            "SELECT source_revision FROM support.classification ORDER BY source_revision",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(revisions, vec![1, 2]);
    db.cleanup().await;
}

#[tokio::test]
async fn an_expired_mailbox_owner_cannot_advance_its_successors_cursor() {
    let mut db = Database::new().await;
    let first = db
        .state
        .claim_mailbox("mailbox-support", "support@dungeonfluxdnd.com", 1000)
        .await
        .unwrap()
        .unwrap();
    assert!(
        db.state
            .claim_mailbox("mailbox-support", "support@dungeonfluxdnd.com", 1001)
            .await
            .unwrap()
            .is_none()
    );
    let second = db
        .state
        .claim_mailbox("mailbox-support", "support@dungeonfluxdnd.com", 1091)
        .await
        .unwrap()
        .unwrap();
    assert!(second.fence > first.fence);
    assert_eq!(
        db.state
            .finish_mailbox("mailbox-support", &first, Some("stale"), true, None)
            .await,
        Err(df_support::ports::SupportStoreError::Conflict)
    );
    db.state
        .finish_mailbox("mailbox-support", &second, Some("current"), true, None)
        .await
        .unwrap();
    let third = db
        .state
        .claim_mailbox("mailbox-support", "support@dungeonfluxdnd.com", 1200)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(third.cursor.as_deref(), Some("current"));
    db.cleanup().await;
}

#[tokio::test]
async fn classification_storage_failure_keeps_manual_work_without_another_model_call() {
    let mut db = Database::new().await;
    db.inspect.as_ref().unwrap().batch_execute(
        "ALTER TABLE support.classification ADD CONSTRAINT reject_result CHECK(data->>'status'<>'classified') NOT VALID"
    ).await.unwrap();
    let mut ingest = worker(
        FakeMailSource::new(vec![Ok(page(mail("1", "thread"))); 2]),
        FakeJevJudge::new(vec![Ok(GOOD_JUDGEMENT.into())]),
        db.store.clone().unwrap(),
        db.state.clone(),
    );
    assert_eq!(
        ingest.run_once(1000).await,
        Err(IngestError::StoreUnavailable)
    );
    db.inspect
        .as_ref()
        .unwrap()
        .batch_execute("ALTER TABLE support.classification DROP CONSTRAINT reject_result")
        .await
        .unwrap();
    let replay = ingest.run_once(1200).await.unwrap();
    assert_eq!(replay.duplicates, 1);
    assert_eq!(ingest.judge.calls(), 1);
    assert_eq!(
        db.classification_status(&super::identifier("ticket", "mailbox-support", "thread"))
            .await
            .as_deref(),
        Some("unclassified")
    );
    db.cleanup().await;
}

#[tokio::test]
async fn the_same_provider_identifiers_in_different_mailboxes_remain_distinct() {
    let mut db = Database::new().await;
    let mut first = worker(
        FakeMailSource::new(vec![Ok(page(mail("1", "thread")))]),
        FakeJevJudge::new(vec![Ok(GOOD_JUDGEMENT.into())]),
        db.store.clone().unwrap(),
        db.state.clone(),
    );
    first.run_once(1000).await.unwrap();
    let mut second = worker(
        FakeMailSource::new(vec![Ok(page(mail("1", "thread")))]),
        FakeJevJudge::new(vec![Ok(GOOD_JUDGEMENT.into())]),
        db.store.clone().unwrap(),
        db.state.clone(),
    );
    second.mailbox_id = "another-mailbox".into();
    assert_eq!(second.run_once(1100).await.unwrap().inserted, 1);
    let count: i64 = db
        .inspect
        .as_ref()
        .unwrap()
        .query_one("SELECT count(*) FROM support.ticket", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn an_interrupted_classification_claim_stays_manual_on_replay() {
    let mut db = Database::new().await;
    let incoming = mail("1", "thread");
    let input =
        super::to_inbound(&incoming, "mailbox-support", "support@dungeonfluxdnd.com").unwrap();
    let ticket_id = input.ticket_id.clone();
    db.store.as_ref().unwrap().ingest(input).await.unwrap();
    let lease = db
        .state
        .claim_mailbox("mailbox-support", "support@dungeonfluxdnd.com", 1000)
        .await
        .unwrap()
        .unwrap();
    db.state
        .claim_classification(
            "mailbox-support",
            &lease,
            &ticket_id,
            &super::identifier("cls", "mailbox-support", "1"),
            1000,
            "Subject",
        )
        .await
        .unwrap();
    db.state
        .finish_mailbox("mailbox-support", &lease, None, false, Some(1060))
        .await
        .unwrap();
    let mut restarted = worker(
        FakeMailSource::new(vec![Ok(page(incoming))]),
        FakeJevJudge::new(Vec::new()),
        db.store.clone().unwrap(),
        db.state.clone(),
    );
    assert_eq!(restarted.run_once(1001).await, Err(IngestError::Deferred));
    assert_eq!(restarted.source.calls(), 0);
    assert_eq!(restarted.run_once(1100).await.unwrap().duplicates, 1);
    assert_eq!(restarted.judge.calls(), 0);
    assert_eq!(
        db.classification_status(ticket_id.as_str())
            .await
            .as_deref(),
        Some("unclassified")
    );
    db.cleanup().await;
}

#[tokio::test]
async fn an_oversized_source_page_is_rejected_before_any_mail_is_admitted() {
    let mut db = Database::new().await;
    let mut ingest = worker(
        FakeMailSource::new(vec![Ok(MailPage {
            messages: vec![mail("1", "thread"); 101],
            next_cursor: Some("never-commit".into()),
        })]),
        FakeJevJudge::new(Vec::new()),
        db.store.clone().unwrap(),
        db.state.clone(),
    );
    assert_eq!(ingest.run_once(1000).await, Err(IngestError::InvalidMail));
    let count: i64 = db
        .inspect
        .as_ref()
        .unwrap()
        .query_one("SELECT count(*) FROM support.message", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
    assert_eq!(ingest.judge.calls(), 0);
    db.cleanup().await;
}

struct Models;

#[async_trait::async_trait]
impl df_support::classification::ModelTransport for Models {
    fn model_name(&self, _: bool) -> &str {
        "synthetic-model"
    }
    async fn judge(
        &self,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, df_support::ingest::JudgeError> {
        let answers: serde_json::Map<String, serde_json::Value> = [
            ("category", "billing"),
            ("urgency", "high"),
            ("privacy", "no"),
            ("legal", "uncertain"),
            ("safety", "no"),
        ]
        .into_iter()
        .map(|(key, choice)| {
            (
                key.into(),
                serde_json::json!({"type":"choice","choice":choice,"confidence":0.95}),
            )
        })
        .collect();
        Ok(serde_json::json!({"answers":answers,"usage":{"input_tokens":100,"output_tokens":0}}))
    }
    async fn draft(
        &self,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, df_support::ingest::JudgeError> {
        Ok(
            serde_json::json!({"record":serde_json::from_str::<serde_json::Value>(GOOD_JUDGEMENT).unwrap(),
            "usage":{"input_tokens":200,"output_tokens":30}}),
        )
    }
}

#[tokio::test]
async fn model_pending_usage_and_settlement_are_durable_without_private_text_in_audit() {
    let mut db = Database::new().await;
    let mut ingest = SupportIngest::new(
        FakeMailSource::new(vec![Ok(page(mail("1", "thread")))]),
        df_support::classification::JevJudge::new(Models),
        db.store.clone().unwrap(),
        "mailbox-support".into(),
        "support@dungeonfluxdnd.com".into(),
        IngestConfig::default(),
        super::State::Ready(db.state.clone()),
    );
    assert_eq!(ingest.run_once(1000).await.unwrap().classified, 1);
    let rows = db
        .inspect
        .as_ref()
        .unwrap()
        .query(
            "SELECT data::text FROM support.audit WHERE data ? 'call_id' ORDER BY audit_id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    let records: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| serde_json::from_str(&row.get::<_, String>(0)).unwrap())
        .collect();
    assert_eq!(
        records
            .iter()
            .filter(|value| value["outcome"] == "unknown")
            .count(),
        2
    );
    assert_eq!(
        records
            .iter()
            .filter(|value| value["outcome"] == "completed")
            .count(),
        2
    );
    assert!(records.iter().any(|value| value["output_tokens"] == 30));
    for record in records {
        let text = record.to_string();
        assert!(!text.contains("ada@example.invalid"));
        assert!(!text.contains("charged twice"));
        assert!(record["actual_micro_dollars"].is_null());
    }
    db.cleanup().await;
}
