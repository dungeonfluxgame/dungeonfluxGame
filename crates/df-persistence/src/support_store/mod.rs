//! PostgreSQL support inbox store. Access is restricted to the control-plane operator role.
mod queries;
mod rows;

use df_support::{
    contracts::{
        ActionProposalRecord, AuditEvent, MessagePage, MessagePageRequest, ProposalId,
        ProposalStatus, SupportTicket, TicketId,
    },
    ports::{SupportStore as SupportStorePort, SupportStoreError},
};
use tokio::sync::Mutex;
use tokio_postgres::Client;

pub use rows::{ApprovalInsert, InboundMessage, IngestOutcome, RetentionPurge};

/// Borrowed adapter over an operator-authorized PostgreSQL connection.
pub struct SupportStore {
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
}

#[cfg(test)]
mod tests;
