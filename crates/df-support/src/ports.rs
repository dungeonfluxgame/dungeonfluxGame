//! Consumer-owned ports for support persistence, classification, mail and approved effects.
use crate::contracts::{
    ActionProposalBatch, ActionProposalRecord, ApprovedAction, AuditEvent, ExecutionOutcome,
    JevClassification, MessagePage, MessagePageRequest, MeteredCall, OperationId, SupportTicket,
    TicketId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SupportStoreError {
    #[error("record was not found")]
    NotFound,
    #[error("revision conflict")]
    Conflict,
    #[error("audit is unavailable")]
    AuditUnavailable,
    #[error("storage is unavailable")]
    Unavailable,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MailTransportError {
    #[error("mailbox is unavailable")]
    Unavailable,
    #[error("message was rejected")]
    Rejected,
    #[error("send outcome is unknown")]
    Unknown,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ClassifierError {
    #[error("classification was unusable")]
    Unusable,
    #[error("classification timed out")]
    DeadlineExceeded,
    #[error("classification provider is unavailable")]
    Unavailable,
    #[error("meter recording failed")]
    MeterUnavailable,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PlannerError {
    #[error("proposal was unusable")]
    Unusable,
    #[error("planning timed out")]
    DeadlineExceeded,
    #[error("planner is unavailable")]
    Unavailable,
    #[error("meter recording failed")]
    MeterUnavailable,
    #[error("planner returned more than five proposals")]
    TooManyProposals,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ActionAdapterError {
    #[error("precondition changed")]
    Conflict,
    #[error("operation is unknown")]
    Unknown,
    #[error("capability or identity is missing")]
    Unauthorized,
    #[error("identity proof is required")]
    NeedsIdentity,
    #[error("capability is required")]
    NeedsCapability,
    #[error("fresh MFA proof is required")]
    NeedsMfa,
    #[error("operation expired")]
    OperationExpired,
    #[error("owner service is unavailable")]
    Unavailable,
    #[error("operation was rejected")]
    Rejected,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MeterError {
    #[error("meter is unavailable")]
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MailSendRequest {
    pub ticket_id: TicketId,
    pub operation_id: OperationId,
    pub thread_id: String,
    pub recipient_handles: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub approved_payload_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MailSendReceipt {
    pub message_id: String,
    pub outcome: ExecutionOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefundRequest {
    pub account_ref: String,
    pub lot_ref: String,
    pub amount_micro_dollars: u64,
    pub preview_revision: u64,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalReceipt {
    pub external_ref: Option<String>,
    pub outcome: ExecutionOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DisputeEvidenceRequest {
    pub dispute_ref: String,
    pub evidence_refs: Vec<String>,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequest {
    pub account_ref: String,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RightsActionRequest {
    pub subject_ref: String,
    pub right: String,
    pub scope_refs: Vec<String>,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestrictedEvidenceRequest {
    pub ticket_ref: String,
    pub evidence_refs: Vec<String>,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BugIssueRequest {
    pub repository_key: String,
    pub title: String,
    pub redacted_body: String,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BugIssueReceipt {
    pub issue_ref: Option<String>,
    pub outcome: ExecutionOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MailboxEvidenceRequest {
    pub ticket_id: TicketId,
    pub message_id: crate::contracts::MessageId,
    pub operation_id: OperationId,
}

#[async_trait::async_trait]
pub trait SupportStore: Send + Sync {
    async fn get_ticket(&self, ticket_id: &TicketId) -> Result<SupportTicket, SupportStoreError>;
    async fn list_messages(
        &self,
        ticket_id: &TicketId,
        request: MessagePageRequest,
    ) -> Result<MessagePage, SupportStoreError>;
    async fn save_ticket(
        &self,
        ticket: SupportTicket,
        expected_revision: u64,
    ) -> Result<(), SupportStoreError>;
    async fn save_proposal(&self, proposal: ActionProposalRecord) -> Result<(), SupportStoreError>;
    async fn record_audit(&self, event: AuditEvent) -> Result<(), SupportStoreError>;
}

#[async_trait::async_trait]
pub trait MailTransport: Send + Sync {
    async fn send_approved(
        &self,
        action: &ApprovedAction,
        request: MailSendRequest,
    ) -> Result<MailSendReceipt, MailTransportError>;
}

#[async_trait::async_trait]
pub trait Classifier: Send + Sync {
    async fn classify(&self, ticket: &SupportTicket) -> Result<JevClassification, ClassifierError>;
}

#[async_trait::async_trait]
pub trait ActionPlanner: Send + Sync {
    async fn plan(&self, ticket: &SupportTicket) -> Result<ActionProposalBatch, PlannerError>;
}

#[async_trait::async_trait]
pub trait FActionAdapter: Send + Sync {
    async fn refund(
        &self,
        action: &ApprovedAction,
        request: RefundRequest,
    ) -> Result<ExternalReceipt, ActionAdapterError>;
    async fn prepare_dispute_evidence(
        &self,
        action: &ApprovedAction,
        request: DisputeEvidenceRequest,
    ) -> Result<ExternalReceipt, ActionAdapterError>;
}

#[async_trait::async_trait]
pub trait BActionAdapter: Send + Sync {
    async fn request_recovery(
        &self,
        action: &ApprovedAction,
        request: RecoveryRequest,
    ) -> Result<ExternalReceipt, ActionAdapterError>;
    async fn open_rights_case(
        &self,
        action: &ApprovedAction,
        request: RightsActionRequest,
    ) -> Result<ExternalReceipt, ActionAdapterError>;
}

#[async_trait::async_trait]
pub trait O2ActionAdapter: Send + Sync {
    async fn restrict_evidence(
        &self,
        action: &ApprovedAction,
        request: RestrictedEvidenceRequest,
    ) -> Result<ExternalReceipt, ActionAdapterError>;
    async fn record_mailbox_evidence(
        &self,
        action: &ApprovedAction,
        request: MailboxEvidenceRequest,
    ) -> Result<ExternalReceipt, ActionAdapterError>;
}

#[async_trait::async_trait]
pub trait GitHubActionAdapter: Send + Sync {
    async fn create_bug_issue(
        &self,
        action: &ApprovedAction,
        request: BugIssueRequest,
    ) -> Result<BugIssueReceipt, ActionAdapterError>;
}

#[async_trait::async_trait]
pub trait Meter: Send + Sync {
    async fn record_call(&self, call: MeteredCall) -> Result<(), MeterError>;
}
