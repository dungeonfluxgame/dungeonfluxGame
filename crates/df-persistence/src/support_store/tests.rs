use std::sync::atomic::{AtomicU64, Ordering};

use df_support::{
    contracts::{
        AuditEvent, AuditId, MessageId, MessagePageRequest, OperatorId, PartyId, ProposalId,
        ProposalStatus, TicketId, TicketStatus,
    },
    ports::{SupportStore as _, SupportStoreError},
};
use tokio_postgres::{Config, NoTls};

use super::{InboundMessage, IngestOutcome, SupportStore};

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);
const MIGRATION: &str = include_str!("../../migrations/0027_support_inbox.sql");

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
            .ok()?;
        let mut admin_config = Config::new();
        admin_config
            .host("127.0.0.1")
            .port(port)
            .user("postgres")
            .dbname("postgres");
        let (admin, admin_connection) = admin_config.connect(NoTls).await.ok()?;
        let mut drivers = vec![tokio::spawn(async move {
            let _ = admin_connection.await;
        })];
        let id = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
        let name = format!("df_supportb_{}_{}_{}", std::process::id(), now_micros(), id);
        let runtime_role = format!("{name}_runtime");
        admin
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .ok()?;
        admin
            .batch_execute(&format!("CREATE ROLE {runtime_role} NOLOGIN"))
            .await
            .ok()?;
        let mut config = Config::new();
        config
            .host("127.0.0.1")
            .port(port)
            .user("postgres")
            .dbname(&name);
        let (client, connection) = config.connect(NoTls).await.ok()?;
        drivers.push(tokio::spawn(async move {
            let _ = connection.await;
        }));
        client.batch_execute(MIGRATION).await.ok()?;
        let (concurrent_client, concurrent_connection) = config.connect(NoTls).await.ok()?;
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
