use std::sync::atomic::{AtomicU64, Ordering};

use df_support::{
    contracts::{
        AuditEvent, AuditId, ClassificationCategory, MessageId, MessagePageRequest, OperationId,
        OperatorId, PartyId, ProposalId, ProposalStatus, QueueContentGrant, QueueFilter,
        QueuePageRequest, TicketId, TicketStatus, TriageCommand,
    },
    ports::{SupportStore as _, SupportStoreError},
};
use tokio_postgres::{Config, NoTls};

use super::{InboundMessage, IngestOutcome, SupportStore};

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);
const MIGRATION: &str = include_str!("../../migrations/0027_support_inbox.sql");
const TRIAGE_MIGRATION: &str = include_str!("../../migrations/0034_support_triage.sql");

struct Database {
    name: String,
    runtime_role: String,
    admin: Option<tokio_postgres::Client>,
    client: Option<tokio_postgres::Client>,
    concurrent_client: Option<tokio_postgres::Client>,
    drivers: Vec<tokio::task::JoinHandle<()>>,
}

impl Database {
    async fn new() -> Option<Self> {
        let port: u16 = std::env::var("DUNGEONFLUX_SUPPORT_PG_PORT")
            .ok()?
            .parse()
            .unwrap();
        let mut admin_config = Config::new();
        admin_config
            .host("127.0.0.1")
            .port(port)
            .user("postgres")
            .dbname("postgres");
        let (admin, admin_connection) = admin_config.connect(NoTls).await.unwrap();
        let mut drivers = vec![tokio::spawn(async move {
            let _ = admin_connection.await;
        })];
        let id = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
        let name = format!("af16_{}_{}_{}", std::process::id(), now_micros(), id);
        let runtime_role = format!("{name}_runtime");
        admin
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .unwrap();
        admin
            .batch_execute(&format!("CREATE ROLE {runtime_role} NOLOGIN"))
            .await
            .unwrap();
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user("postgres")
            .dbname(&name);
        let (client, connection) = config.connect(NoTls).await.unwrap();
        drivers.push(tokio::spawn(async move {
            let _ = connection.await;
        }));
        client.batch_execute(MIGRATION).await.unwrap();
        client.batch_execute(TRIAGE_MIGRATION).await.unwrap();
        let (concurrent_client, concurrent_connection) = config.connect(NoTls).await.unwrap();
        drivers.push(tokio::spawn(async move {
            let _ = concurrent_connection.await;
        }));
        Some(Self {
            name,
            runtime_role,
            admin: Some(admin),
            client: Some(client),
            concurrent_client: Some(concurrent_client),
            drivers,
        })
    }

    async fn cleanup(&mut self) {
        if let Some(client) = self.admin.take() {
            let _ = client
                .batch_execute(&format!(
                    "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                    self.name
                ))
                .await;
            let _ = client
                .batch_execute(&format!("DROP ROLE IF EXISTS {}", self.runtime_role))
                .await;
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
        .unwrap()
        .as_micros()
}

fn inbound(message: &str, ticket: &str, thread: &str, received: i64) -> InboundMessage {
    InboundMessage {
        ticket_id: TicketId::new(ticket).unwrap(),
        message_id: MessageId::new(format!("stored-{message}")).unwrap(),
        mailbox_id: "mailbox-support".into(),
        mailbox_address: "support@example.invalid".into(),
        gmail_message_id: message.into(),
        gmail_thread_id: thread.into(),
        received_at_unix_s: received,
        sender: PartyId::new("party-customer").unwrap(),
        sender_address_ref: "enc:v1:opaque-address-reference".into(),
        sender_display_name: Some("Customer".into()),
        recipients: vec![(
            PartyId::new("party-support").unwrap(),
            "enc:v1:support-address".into(),
        )],
        body_text: "Private message body".into(),
    }
}

#[tokio::test]
async fn duplicate_delivery_cas_retention_and_privileges_hold() {
    let Some(mut database) = Database::new().await else {
        eprintln!("SKIPPED: DUNGEONFLUX_SUPPORT_PG_PORT must name the owned scratch PostgreSQL");
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    let concurrent_store = SupportStore::new(database.concurrent_client.take().unwrap());
    assert_eq!(
        store
            .ingest(inbound("gmail-1", "ticket-1", "thread-1", 100))
            .await
            .unwrap(),
        IngestOutcome::Inserted {
            ticket_id: TicketId::new("ticket-1").unwrap(),
        }
    );
    assert_eq!(
        store
            .ingest(inbound("gmail-1", "ticket-1", "thread-1", 100))
            .await
            .unwrap(),
        IngestOutcome::Duplicate {
            ticket_id: TicketId::new("ticket-1").unwrap(),
        }
    );
    assert_eq!(
        store
            .get_ticket(&TicketId::new("ticket-1").unwrap())
            .await
            .unwrap()
            .revision,
        1
    );
    let page = store
        .list_messages(
            &TicketId::new("ticket-1").unwrap(),
            MessagePageRequest::new(None, 10).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(
        page.messages[0].content.delimit_for_prompt().to_string(),
        "<untrusted_email_data>\nPrivate message body\n</untrusted_email_data>"
    );

    store
        .client
        .lock()
        .await
        .execute(
            "INSERT INTO support.proposal (proposal_id,ticket_id,operation_id,source_revision,status,data) VALUES ('proposal-1','ticket-1','op-1',1,'draft','{\"status\":\"draft\"}'::jsonb)",
            &[],
        )
        .await
        .unwrap();
    let proposal_id = ProposalId::new("proposal-1").unwrap();
    store
        .transition_proposal(
            &proposal_id,
            ProposalStatus::Draft,
            ProposalStatus::Approved,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .transition_proposal(
                &proposal_id,
                ProposalStatus::Draft,
                ProposalStatus::Rejected
            )
            .await,
        Err(SupportStoreError::Conflict)
    );
    store
        .record_audit(AuditEvent::ContentRead {
            ticket_id: TicketId::new("ticket-1").unwrap(),
            operator_id: OperatorId::new("operator-1").unwrap(),
            audit_id: AuditId::new("read-1").unwrap(),
        })
        .await
        .unwrap();
    let audit_timestamp: i64 = store
        .client
        .lock()
        .await
        .query_one(
            "SELECT created_at_unix_s FROM support.audit WHERE ticket_id='ticket-1' AND audit_id='read-1'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(audit_timestamp > 0);

    let mut ticket = store
        .get_ticket(&TicketId::new("ticket-1").unwrap())
        .await
        .unwrap();
    ticket.status = TicketStatus::Open;
    let first = store.save_ticket(ticket.clone(), 1);
    let second = concurrent_store.save_ticket(ticket, 1);
    let (left, right) = tokio::join!(first, second);
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert!(matches!(left, Ok(()) | Err(SupportStoreError::Conflict)));
    assert!(matches!(right, Ok(()) | Err(SupportStoreError::Conflict)));

    store
        .ingest(inbound("gmail-2", "ticket-2", "thread-2", 200))
        .await
        .unwrap();
    store
        .ingest(inbound("gmail-3", "ticket-3", "thread-3", 300))
        .await
        .unwrap();
    store
        .client
        .lock()
        .await
        .execute(
            "UPDATE support.ticket SET retained_until=to_timestamp(0)",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(store.purge_expired(10, 1).await.unwrap().deleted_tickets, 1);
    assert_eq!(store.purge_expired(10, 1).await.unwrap().deleted_tickets, 1);
    assert_eq!(store.purge_expired(10, 1).await.unwrap().deleted_tickets, 1);
    assert_eq!(store.purge_expired(10, 1).await.unwrap().deleted_tickets, 0);
    assert_eq!(
        store
            .ingest(inbound("gmail-1", "ticket-1", "thread-1", 100))
            .await
            .unwrap(),
        IngestOutcome::Retired
    );

    let privilege = store.client.lock().await.query_one(
        &format!("SELECT has_table_privilege('df_control','support.ticket','SELECT'), has_schema_privilege('{}','support','USAGE'), has_table_privilege('{}','support.ticket','SELECT')", database.runtime_role, database.runtime_role),
        &[],
    ).await.unwrap();
    assert!(privilege.get::<_, bool>(0));
    assert!(!privilege.get::<_, bool>(1));
    assert!(!privilege.get::<_, bool>(2));
    assert_eq!(
        TicketStatus::New.transition(TicketStatus::Resolved),
        Err(df_support::contracts::ContractError::InvalidTransition)
    );
    database.cleanup().await;
}

fn triage_command(
    operation: &str,
    ticket: &str,
    expected_revision: u64,
    status: Option<TicketStatus>,
    category: Option<ClassificationCategory>,
) -> TriageCommand {
    TriageCommand::new(
        OperationId::new(operation).unwrap(),
        TicketId::new(ticket).unwrap(),
        OperatorId::new("operator-1").unwrap(),
        expected_revision,
        expected_revision,
        status,
        category,
        "Manual triage correction".into(),
    )
    .unwrap()
}

#[tokio::test]
async fn support_queue_pages_and_filters_real_pg() {
    let Some(mut database) = Database::new().await else {
        eprintln!("SKIPPED: DUNGEONFLUX_SUPPORT_PG_PORT must name the owned scratch PostgreSQL");
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    store
        .ingest(inbound("q-gmail-1", "q-ticket-1", "q-thread-1", 100))
        .await
        .unwrap();
    store
        .ingest(inbound("q-gmail-2", "q-ticket-2", "q-thread-2", 300))
        .await
        .unwrap();
    store
        .ingest(inbound("q-gmail-3", "q-ticket-3", "q-thread-3", 200))
        .await
        .unwrap();
    store
        .save_classification(
            "q-class-1",
            &TicketId::new("q-ticket-2").unwrap(),
            1,
            &serde_json::json!({"category": "bug_report"}),
            300,
        )
        .await
        .unwrap();

    let first = store
        .list_queue(
            QueuePageRequest::new(QueueFilter::default(), None, 2).unwrap(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(first.entries.len(), 2);
    assert!(first.entries.iter().all(|entry| entry.snippet.is_none()));
    assert!(first.next_cursor.is_some());
    assert_eq!(first.entries[0].ticket_id.as_str(), "q-ticket-2");
    let second = store
        .list_queue(
            QueuePageRequest::new(QueueFilter::default(), first.next_cursor, 2).unwrap(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(second.entries.len(), 1);
    assert!(second.next_cursor.is_none());
    let seen: Vec<&str> = first
        .entries
        .iter()
        .chain(second.entries.iter())
        .map(|entry| entry.ticket_id.as_str())
        .collect();
    assert_eq!(seen, vec!["q-ticket-2", "q-ticket-3", "q-ticket-1"]);

    let by_status = store
        .list_queue(
            QueuePageRequest::new(
                QueueFilter {
                    status: Some(TicketStatus::New),
                    category: None,
                },
                None,
                10,
            )
            .unwrap(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(by_status.entries.len(), 3);
    let by_category = store
        .list_queue(
            QueuePageRequest::new(
                QueueFilter {
                    status: None,
                    category: Some(ClassificationCategory::BugReport),
                },
                None,
                10,
            )
            .unwrap(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(by_category.entries.len(), 1);
    assert_eq!(by_category.entries[0].ticket_id.as_str(), "q-ticket-2");
    assert_eq!(
        by_category.entries[0].category,
        Some(ClassificationCategory::BugReport)
    );
    database.cleanup().await;
}

#[tokio::test]
async fn support_triage_cas_and_audit_commit_together() {
    let Some(mut database) = Database::new().await else {
        eprintln!("SKIPPED: DUNGEONFLUX_SUPPORT_PG_PORT must name the owned scratch PostgreSQL");
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    store
        .ingest(inbound("t-gmail-1", "t-ticket-1", "t-thread-1", 100))
        .await
        .unwrap();
    let receipt = store
        .apply_triage(triage_command(
            "op-status-1",
            "t-ticket-1",
            1,
            Some(TicketStatus::Open),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(receipt.previous_revision, 1);
    assert_eq!(receipt.new_revision, 2);
    assert!(!receipt.replayed);
    let ticket = store
        .get_ticket(&TicketId::new("t-ticket-1").unwrap())
        .await
        .unwrap();
    assert_eq!(ticket.status, TicketStatus::Open);
    assert_eq!(ticket.revision, 2);
    let audits: i64 = store
        .client
        .lock()
        .await
        .query_one(
            "SELECT count(*) FROM support.audit WHERE ticket_id='t-ticket-1'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(audits, 1);

    // A stale CAS revision refuses and writes no further audit row.
    assert_eq!(
        store
            .apply_triage(triage_command(
                "op-status-stale",
                "t-ticket-1",
                1,
                Some(TicketStatus::Resolved),
                None,
            ))
            .await,
        Err(SupportStoreError::Conflict)
    );
    // An invalid transition refuses without a revision change.
    assert_eq!(
        store
            .apply_triage(triage_command(
                "op-status-invalid",
                "t-ticket-1",
                2,
                Some(TicketStatus::New),
                None,
            ))
            .await,
        Err(SupportStoreError::InvalidTransition)
    );
    // A forced audit failure rolls the ticket change back with it.
    store
        .client
        .lock()
        .await
        .execute(
            "INSERT INTO support.audit (audit_id,operation_id,ticket_id,data,created_at_unix_s) VALUES ('triage:op-audit-fail','seed','t-ticket-1','{}'::jsonb,1)",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .apply_triage(triage_command(
                "op-audit-fail",
                "t-ticket-1",
                2,
                Some(TicketStatus::Resolved),
                None,
            ))
            .await,
        Err(SupportStoreError::AuditUnavailable)
    );
    let ticket = store
        .get_ticket(&TicketId::new("t-ticket-1").unwrap())
        .await
        .unwrap();
    assert_eq!(ticket.status, TicketStatus::Open);
    assert_eq!(ticket.revision, 2);
    store
        .client
        .lock()
        .await
        .batch_execute(
            "ALTER TABLE support.triage_operation ADD CONSTRAINT fail_receipt \
         CHECK (operation_id <> 'op-receipt-fail')",
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .apply_triage(triage_command(
                "op-receipt-fail",
                "t-ticket-1",
                2,
                Some(TicketStatus::Resolved),
                Some(ClassificationCategory::Feedback)
            ))
            .await,
        Err(SupportStoreError::Unavailable)
    );
    let row = store.client.lock().await.query_one(
        "SELECT revision, status, classification_head, \
         (SELECT count(*) FROM support.classification), (SELECT count(*) FROM support.audit), \
         (SELECT count(*) FROM support.triage_operation) FROM support.ticket WHERE ticket_id='t-ticket-1'", &[]
    ).await.unwrap();
    assert_eq!(row.get::<_, i64>(0), 2);
    assert_eq!(row.get::<_, String>(1), "open");
    assert_eq!(row.get::<_, Option<String>>(2), None);
    assert_eq!(row.get::<_, i64>(3), 0);
    assert_eq!(row.get::<_, i64>(4), 2);
    assert_eq!(row.get::<_, i64>(5), 1);
    database.cleanup().await;
}

#[tokio::test]
async fn support_category_correction_keeps_prior_classification() {
    let Some(mut database) = Database::new().await else {
        eprintln!("SKIPPED: DUNGEONFLUX_SUPPORT_PG_PORT must name the owned scratch PostgreSQL");
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    store
        .ingest(inbound("c-gmail-1", "c-ticket-1", "c-thread-1", 100))
        .await
        .unwrap();
    store
        .save_classification(
            "model-class-1",
            &TicketId::new("c-ticket-1").unwrap(),
            1,
            &serde_json::json!({"category": "bug_report", "confidence": 0.9}),
            100,
        )
        .await
        .unwrap();
    let receipt = store
        .apply_triage(triage_command(
            "op-category-1",
            "c-ticket-1",
            1,
            None,
            Some(ClassificationCategory::BillingRefund),
        ))
        .await
        .unwrap();
    assert_eq!(
        receipt.category,
        Some(ClassificationCategory::BillingRefund)
    );
    let status_receipt = store
        .apply_triage(triage_command(
            "status-after-category",
            "c-ticket-1",
            2,
            Some(TicketStatus::Open),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        status_receipt.category,
        Some(ClassificationCategory::BillingRefund)
    );
    let evidence = store
        .client
        .lock()
        .await
        .query_one(
            "SELECT source_revision, data->>'revision', data->>'reason', data->>'operator_id' \
         FROM support.classification WHERE classification_id='manual:op-category-1'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(evidence.get::<_, i64>(0), 1);
    assert_eq!(evidence.get::<_, String>(1), "2");
    assert_eq!(evidence.get::<_, String>(2), "Manual triage correction");
    assert_eq!(evidence.get::<_, String>(3), "operator-1");
    let ticket = store
        .get_ticket(&TicketId::new("c-ticket-1").unwrap())
        .await
        .unwrap();
    assert_eq!(ticket.first_received_at_unix_s, 100);
    let rows = store
        .client
        .lock()
        .await
        .query(
            "SELECT classification_id, data->>'category' FROM support.classification WHERE ticket_id='c-ticket-1' ORDER BY classification_id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let prior: String = rows
        .iter()
        .find(|row| row.get::<_, String>(0) == "model-class-1")
        .unwrap()
        .get(1);
    assert_eq!(prior, "bug_report");
    let queue = store
        .list_queue(
            QueuePageRequest::new(QueueFilter::default(), None, 10).unwrap(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        queue.entries[0].category,
        Some(ClassificationCategory::BillingRefund)
    );
    database.cleanup().await;
}

#[tokio::test]
async fn support_triage_conflicting_retry_is_rejected() {
    let Some(mut database) = Database::new().await else {
        eprintln!("SKIPPED: DUNGEONFLUX_SUPPORT_PG_PORT must name the owned scratch PostgreSQL");
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    store
        .ingest(inbound("r-gmail-1", "r-ticket-1", "r-thread-1", 100))
        .await
        .unwrap();
    let command = triage_command(
        "op-retry-1",
        "r-ticket-1",
        1,
        Some(TicketStatus::Open),
        None,
    );
    let first = store.apply_triage(command.clone()).await.unwrap();
    let replay = store.apply_triage(command).await.unwrap();
    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(replay.new_revision, first.new_revision);
    assert_eq!(replay.created_at_unix_s, first.created_at_unix_s);
    assert_eq!(
        store
            .get_ticket(&TicketId::new("r-ticket-1").unwrap())
            .await
            .unwrap()
            .revision,
        2
    );
    let conflicting = TriageCommand::new(
        OperationId::new("op-retry-1").unwrap(),
        TicketId::new("r-ticket-1").unwrap(),
        OperatorId::new("operator-1").unwrap(),
        1,
        1,
        None,
        Some(ClassificationCategory::Spam),
        "Different correction".into(),
    )
    .unwrap();
    assert_eq!(
        store.apply_triage(conflicting).await,
        Err(SupportStoreError::Conflict)
    );
    database.cleanup().await;
}

#[tokio::test]
async fn support_restricted_queue_does_not_leak_content() {
    let Some(mut database) = Database::new().await else {
        eprintln!("SKIPPED: DUNGEONFLUX_SUPPORT_PG_PORT must name the owned scratch PostgreSQL");
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    store
        .ingest(inbound("s-gmail-1", "s-ticket-1", "s-thread-1", 100))
        .await
        .unwrap();
    store
        .ingest(inbound("s-gmail-2", "s-ticket-2", "s-thread-2", 99))
        .await
        .unwrap();
    store
        .client
        .lock()
        .await
        .execute(
            "UPDATE support.ticket SET sensitivity='restricted' WHERE ticket_id='s-ticket-1'",
            &[],
        )
        .await
        .unwrap();

    let redacted = store
        .list_queue(
            QueuePageRequest::new(QueueFilter::default(), None, 10).unwrap(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(redacted.entries.len(), 2);
    let entry = &redacted.entries[0];
    assert!(entry.restricted);
    assert_eq!(entry.subject, None);
    assert_eq!(entry.snippet, None);
    assert_eq!(entry.attachment_count, None);
    assert!(!format!("{entry:?}").contains("Private message body"));

    assert!(matches!(
        store
            .read_messages_audited(
                &TicketId::new("s-ticket-1").unwrap(),
                MessagePageRequest::new(None, 10).unwrap(),
                QueueContentGrant {
                    operator_id: OperatorId::new("operator-1").unwrap(),
                    audit_id: AuditId::new("wrong-scope").unwrap(),
                    restricted_ticket_ids: vec![TicketId::new("s-ticket-2").unwrap()]
                }
            )
            .await,
        Err(SupportStoreError::NotFound)
    ));
    let ordinary_grant = QueueContentGrant {
        operator_id: OperatorId::new("operator-1").unwrap(),
        audit_id: AuditId::new("ordinary-read").unwrap(),
        restricted_ticket_ids: Vec::new(),
    };
    let ordinary = store
        .list_queue(
            QueuePageRequest::new(QueueFilter::default(), None, 10).unwrap(),
            Some(ordinary_grant),
        )
        .await
        .unwrap();
    assert!(ordinary.entries[0].snippet.is_none());
    assert!(ordinary.entries[1].snippet.is_some());
    let grant = QueueContentGrant {
        operator_id: OperatorId::new("operator-1").unwrap(),
        audit_id: AuditId::new("queue-read-1").unwrap(),
        restricted_ticket_ids: vec![TicketId::new("s-ticket-1").unwrap()],
    };
    let revealed = store
        .list_queue(
            QueuePageRequest::new(QueueFilter::default(), None, 10).unwrap(),
            Some(grant),
        )
        .await
        .unwrap();
    assert!(
        revealed.entries[0]
            .snippet
            .as_deref()
            .unwrap()
            .contains("Private message body")
    );
    assert_eq!(revealed.entries[0].attachment_count, None);

    let page = store
        .read_messages_audited(
            &TicketId::new("s-ticket-1").unwrap(),
            MessagePageRequest::new(None, 10).unwrap(),
            QueueContentGrant {
                operator_id: OperatorId::new("operator-1").unwrap(),
                audit_id: AuditId::new("content-read-1").unwrap(),
                restricted_ticket_ids: vec![TicketId::new("s-ticket-1").unwrap()],
            },
        )
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    // A failed read audit returns no content.
    store
        .client
        .lock()
        .await
        .execute(
            "INSERT INTO support.audit (audit_id,operation_id,ticket_id,data,created_at_unix_s) VALUES ('content-read-fail','seed','s-ticket-1','{}'::jsonb,1)",
            &[],
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .read_messages_audited(
                &TicketId::new("s-ticket-1").unwrap(),
                MessagePageRequest::new(None, 10).unwrap(),
                QueueContentGrant {
                    operator_id: OperatorId::new("operator-1").unwrap(),
                    audit_id: AuditId::new("content-read-fail").unwrap(),
                    restricted_ticket_ids: vec![TicketId::new("s-ticket-1").unwrap()]
                },
            )
            .await,
        Err(SupportStoreError::AuditUnavailable)
    ));
    database.cleanup().await;
}

#[tokio::test]
async fn support_concurrent_retries_replay_and_survive_adapter_restart() {
    let Some(mut database) = Database::new().await else {
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    let other = SupportStore::new(database.concurrent_client.take().unwrap());
    store
        .ingest(inbound("concurrent", "ticket", "thread", 10))
        .await
        .unwrap();
    let command = triage_command("concurrent-op", "ticket", 1, Some(TicketStatus::Open), None);
    let (first, second) = tokio::join!(
        store.apply_triage(command.clone()),
        other.apply_triage(command.clone())
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_ne!(first.replayed, second.replayed);
    assert_eq!(first.created_at_unix_s, second.created_at_unix_s);
    assert_eq!(first.audit_id, second.audit_id);
    drop(store);
    drop(other);
    let (client, connection) = Config::new()
        .host("127.0.0.1")
        .port(
            std::env::var("DUNGEONFLUX_SUPPORT_PG_PORT")
                .unwrap()
                .parse()
                .unwrap(),
        )
        .user("postgres")
        .dbname(&database.name)
        .connect(NoTls)
        .await
        .unwrap();
    database.drivers.push(tokio::spawn(async move {
        // Scratch-database cleanup intentionally closes this owned connection.
        let _ = connection.await;
    }));
    let restarted = SupportStore::new(client);
    let replay = restarted.apply_triage(command).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.created_at_unix_s, first.created_at_unix_s);
    let row = restarted.client.lock().await.query_one(
        "SELECT (SELECT count(*) FROM support.audit), (SELECT count(*) FROM support.triage_operation)", &[]
    ).await.unwrap();
    assert_eq!(row.get::<_, i64>(0), 1);
    assert_eq!(row.get::<_, i64>(1), 1);
    database.cleanup().await;
}

#[tokio::test]
async fn support_store_rejects_invalid_requests_without_writes() {
    let Some(mut database) = Database::new().await else {
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    store
        .ingest(inbound("invalid", "ticket", "thread", 10))
        .await
        .unwrap();
    let mut command = triage_command("invalid-op", "ticket", 1, Some(TicketStatus::Open), None);
    for reason in [" ".to_owned(), "x".repeat(501)] {
        command.reason = reason;
        assert_eq!(
            store.apply_triage(command.clone()).await,
            Err(SupportStoreError::InvalidRequest)
        );
    }
    command.reason = "Valid reason".into();
    command.source_revision = 2;
    assert_eq!(
        store.apply_triage(command.clone()).await,
        Err(SupportStoreError::InvalidRequest)
    );
    command.source_revision = 1;
    command.status = None;
    assert_eq!(
        store.apply_triage(command).await,
        Err(SupportStoreError::InvalidRequest)
    );
    for page_size in [0, 101] {
        assert_eq!(
            store
                .list_queue(
                    QueuePageRequest {
                        filter: QueueFilter::default(),
                        cursor: None,
                        page_size
                    },
                    None
                )
                .await,
            Err(SupportStoreError::InvalidRequest)
        );
        assert!(matches!(
            store
                .read_messages_audited(
                    &TicketId::new("ticket").unwrap(),
                    MessagePageRequest {
                        cursor: None,
                        page_size
                    },
                    QueueContentGrant {
                        operator_id: OperatorId::new("operator").unwrap(),
                        audit_id: AuditId::new("invalid-read").unwrap(),
                        restricted_ticket_ids: Vec::new()
                    },
                )
                .await,
            Err(SupportStoreError::InvalidRequest)
        ));
    }
    let row = store.client.lock().await.query_one(
        "SELECT revision, (SELECT count(*) FROM support.audit) FROM support.ticket WHERE ticket_id='ticket'", &[]
    ).await.unwrap();
    assert_eq!(row.get::<_, i64>(0), 1);
    assert_eq!(row.get::<_, i64>(1), 0);
    database.cleanup().await;
}

#[tokio::test]
async fn support_queue_audit_failure_withholds_entire_page_and_rolls_back() {
    let Some(mut database) = Database::new().await else {
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    for (message, ticket) in [("one", "one"), ("two", "two")] {
        store
            .ingest(inbound(message, ticket, ticket, 10))
            .await
            .unwrap();
    }
    store
        .client
        .lock()
        .await
        .execute(
            "INSERT INTO support.audit (audit_id,operation_id,ticket_id,data,created_at_unix_s) \
         VALUES ('queue:read-fail:1','seed','two','{}',1)",
            &[],
        )
        .await
        .unwrap();
    let grant = QueueContentGrant {
        operator_id: OperatorId::new("operator").unwrap(),
        audit_id: AuditId::new("read-fail").unwrap(),
        restricted_ticket_ids: Vec::new(),
    };
    assert_eq!(
        store
            .list_queue(
                QueuePageRequest::new(QueueFilter::default(), None, 10).unwrap(),
                Some(grant)
            )
            .await,
        Err(SupportStoreError::AuditUnavailable)
    );
    let row = store
        .client
        .lock()
        .await
        .query_one("SELECT count(*) FROM support.audit", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 1);
    database.cleanup().await;
}

#[tokio::test]
async fn support_queue_ties_use_ticket_id_and_do_not_return_an_empty_next_page() {
    let Some(mut database) = Database::new().await else {
        return;
    };
    let store = SupportStore::new(database.client.take().unwrap());
    for (index, name) in ["c", "a", "b", &"az".repeat(64)].into_iter().enumerate() {
        store
            .ingest(inbound(&format!("message-{index}"), name, name, 10))
            .await
            .unwrap();
    }
    let mut cursor = None;
    let mut seen = Vec::new();
    for _ in 0..4 {
        let page = store
            .list_queue(
                QueuePageRequest::new(QueueFilter::default(), cursor, 1).unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        seen.push(page.entries[0].ticket_id.as_str().to_owned());
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(cursor.is_none());
    assert_eq!(
        seen,
        vec![
            "a".to_owned(),
            "az".repeat(64),
            "b".to_owned(),
            "c".to_owned()
        ]
    );
    database.cleanup().await;
}
