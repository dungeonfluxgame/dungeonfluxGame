//! PostgreSQL support inbox store. Access is restricted to the control-plane operator role.
mod queries;
mod rows;

use df_support::{
    contracts::{
        ActionProposalRecord, AuditEvent, MessagePage, MessagePageRequest, ProposalId,
        ProposalStatus, QueueContentGrant, QueuePage, QueuePageRequest, SupportTicket, TicketId,
        TriageCommand, TriageReceipt,
    },
    ports::{SupportStore as SupportStorePort, SupportStoreError},
};
use tokio::sync::Mutex;
use tokio_postgres::Client;

pub use rows::{ApprovalInsert, InboundMessage, IngestOutcome, RetentionPurge};

/// Borrowed adapter over an operator-authorized PostgreSQL connection.
pub struct SupportStore {
    // Transactions exclusively borrow this connection; cancellation queues rollback before reuse.
    client: Mutex<Client>,
}

impl SupportStore {
    pub fn new(client: Client) -> Self {
        Self {
            client: Mutex::new(client),
        }
    }

    /// Persist a delivery once by Gmail message id and its mailbox/thread identity.
    pub async fn ingest(
        &self,
        message: InboundMessage,
    ) -> Result<IngestOutcome, SupportStoreError> {
        queries::ingest(&mut *self.client.lock().await, message).await
    }

    /// Delete at most `limit` expired ticket trees. Erasure suppression IDs remain durable.
    pub async fn purge_expired(
        &self,
        now_unix_s: i64,
        limit: u16,
    ) -> Result<RetentionPurge, SupportStoreError> {
        queries::purge_expired(&*self.client.lock().await, now_unix_s, limit).await
    }

    /// Append a classification version scoped to a ticket revision.
    pub async fn save_classification(
        &self,
        classification_id: &str,
        ticket_id: &TicketId,
        source_revision: u64,
        data: &serde_json::Value,
        created_at_unix_s: i64,
    ) -> Result<(), SupportStoreError> {
        queries::save_classification(
            &*self.client.lock().await,
            classification_id,
            ticket_id,
            source_revision,
            data,
            created_at_unix_s,
        )
        .await
    }

    /// Persist a deadline while retaining the original receipt timestamp.
    pub async fn save_deadline(
        &self,
        ticket_id: &TicketId,
        kind: &str,
        due_at_unix_s: i64,
        based_on_received_at_unix_s: i64,
    ) -> Result<(), SupportStoreError> {
        queries::save_deadline(
            &*self.client.lock().await,
            ticket_id,
            kind,
            due_at_unix_s,
            based_on_received_at_unix_s,
        )
        .await
    }

    /// Persist approval evidence independently from proposal content.
    pub async fn save_approval(&self, approval: ApprovalInsert) -> Result<(), SupportStoreError> {
        queries::save_approval(&*self.client.lock().await, approval).await
    }

    /// Compare-and-set a proposal's state using the contract's closed transition graph.
    pub async fn transition_proposal(
        &self,
        proposal_id: &ProposalId,
        expected: ProposalStatus,
        next: ProposalStatus,
    ) -> Result<(), SupportStoreError> {
        queries::transition_proposal(&*self.client.lock().await, proposal_id, expected, next).await
    }
}

#[async_trait::async_trait]
impl SupportStorePort for SupportStore {
    async fn get_ticket(&self, ticket_id: &TicketId) -> Result<SupportTicket, SupportStoreError> {
        queries::get_ticket(&*self.client.lock().await, ticket_id).await
    }

    async fn list_messages(
        &self,
        ticket_id: &TicketId,
        request: MessagePageRequest,
    ) -> Result<MessagePage, SupportStoreError> {
        queries::list_messages(&*self.client.lock().await, ticket_id, request).await
    }

    async fn save_ticket(
        &self,
        ticket: SupportTicket,
        expected_revision: u64,
    ) -> Result<(), SupportStoreError> {
        queries::save_ticket(&*self.client.lock().await, ticket, expected_revision).await
    }

    async fn save_proposal(&self, proposal: ActionProposalRecord) -> Result<(), SupportStoreError> {
        queries::save_proposal(&*self.client.lock().await, proposal).await
    }

    async fn record_audit(&self, event: AuditEvent) -> Result<(), SupportStoreError> {
        queries::record_audit(&*self.client.lock().await, event).await
    }

    async fn list_queue(
        &self,
        request: QueuePageRequest,
        content: Option<QueueContentGrant>,
    ) -> Result<QueuePage, SupportStoreError> {
        let context = df_observe::current_audit_context();
        let started = std::time::Instant::now();
        let operation_hash = content
            .as_ref()
            .map(|grant| df_observe::operation_hash(grant.audit_id.as_str().as_bytes()));
        let result = queries::list_queue(&mut *self.client.lock().await, request, content).await;
        record_operation("queue_read", context, operation_hash, started, &result);
        result
    }

    async fn apply_triage(
        &self,
        command: TriageCommand,
    ) -> Result<TriageReceipt, SupportStoreError> {
        let context = df_observe::current_audit_context();
        let started = std::time::Instant::now();
        let operation_hash = df_observe::operation_hash(command.operation_id.as_str().as_bytes());
        let result = queries::apply_triage(&mut *self.client.lock().await, command).await;
        record_operation("triage", context, Some(operation_hash), started, &result);
        result
    }

    async fn read_messages_audited(
        &self,
        ticket_id: &TicketId,
        request: MessagePageRequest,
        content: QueueContentGrant,
    ) -> Result<MessagePage, SupportStoreError> {
        let context = df_observe::current_audit_context();
        let started = std::time::Instant::now();
        let operation_hash = df_observe::operation_hash(content.audit_id.as_str().as_bytes());
        let result = queries::read_messages_audited(
            &mut *self.client.lock().await,
            ticket_id,
            request,
            content,
        )
        .await;
        record_operation(
            "content_read",
            context,
            Some(operation_hash),
            started,
            &result,
        );
        result
    }
}

fn record_operation<T>(
    action: &'static str,
    context: df_observe::AuditContext,
    operation_hash: Option<u64>,
    started: std::time::Instant,
    result: &Result<T, SupportStoreError>,
) {
    let operation_hash = operation_hash.or(context.operation);
    let _scope = df_observe::audit_scope(context);
    let outcome = match result {
        Ok(_) => "succeeded",
        Err(SupportStoreError::NotFound) => "not_found",
        Err(SupportStoreError::Conflict) => "conflict",
        Err(SupportStoreError::AuditUnavailable) => "audit_unavailable",
        Err(SupportStoreError::Unavailable) => "unavailable",
        Err(SupportStoreError::InvalidTransition) => "invalid_transition",
        Err(SupportStoreError::InvalidRequest) => "invalid_request",
    };
    df_observe::record_audit_line(
        "support.store_operation",
        format!(
            "SUPPORT {action} status={outcome} op={} ms={}",
            operation_hash.map_or_else(|| "none".to_owned(), |value| value.to_string()),
            started.elapsed().as_millis(),
        ),
    );
}

#[cfg(test)]
mod tests;
