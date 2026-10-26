//! Deterministic local adapters for support contract tests; no provider or network access.
use crate::contracts::{
    ActionProposalBatch, ActionProposalRecord, ApprovedAction, AuditEvent, ExecutionOutcome,
    JevClassification, MessagePage, MessagePageRequest, MeteredCall, ModelActionProposal,
    SupportTicket, TicketId,
};
use crate::ports::*;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct FakeGmail;

#[async_trait::async_trait]
impl MailTransport for FakeGmail {
    async fn send_approved(
        &self,
        _action: &ApprovedAction,
        request: MailSendRequest,
    ) -> Result<MailSendReceipt, MailTransportError> {
        Ok(MailSendReceipt {
            message_id: request.operation_id.as_str().to_owned(),
            outcome: ExecutionOutcome::Succeeded,
        })
    }
}

pub struct FakeJev {
    result: Mutex<Option<JevClassification>>,
}

impl FakeJev {
    pub fn new(result: JevClassification) -> Self {
        Self {
            result: Mutex::new(Some(result)),
        }
    }
}

#[async_trait::async_trait]
impl Classifier for FakeJev {
    async fn classify(
        &self,
        _ticket: &SupportTicket,
    ) -> Result<JevClassification, ClassifierError> {
        self.result
            .lock()
            .map_err(|_| ClassifierError::Unavailable)?
            .take()
            .ok_or(ClassifierError::Unusable)
    }
}

pub struct FakeLlm {
    source_revision: u64,
    proposals: Mutex<Vec<ModelActionProposal>>,
}

impl FakeLlm {
    pub fn new(source_revision: u64, proposals: Vec<ModelActionProposal>) -> Self {
        Self {
            source_revision,
            proposals: Mutex::new(proposals),
        }
    }
}

#[async_trait::async_trait]
impl ActionPlanner for FakeLlm {
    async fn plan(&self, _ticket: &SupportTicket) -> Result<ActionProposalBatch, PlannerError> {
        let proposals = self
            .proposals
            .lock()
            .map_err(|_| PlannerError::Unavailable)
            .map(|mut proposals| std::mem::take(&mut *proposals))?;
        ActionProposalBatch::new(self.source_revision, proposals)
            .map_err(|_| PlannerError::TooManyProposals)
    }
}

#[derive(Default)]
pub struct FakeGitHub;

#[async_trait::async_trait]
impl GitHubActionAdapter for FakeGitHub {
    async fn create_bug_issue(
        &self,
        action: &ApprovedAction,
        request: BugIssueRequest,
    ) -> Result<BugIssueReceipt, ActionAdapterError> {
        if action.proposal().operation_id != request.operation_id {
            return Err(ActionAdapterError::Conflict);
        }
        Ok(BugIssueReceipt {
            issue_ref: Some(format!("fake:{}", request.operation_id.as_str())),
            outcome: ExecutionOutcome::Succeeded,
        })
    }
}

#[derive(Default)]
pub struct FakeStore {
    tickets: Mutex<HashMap<TicketId, SupportTicket>>,
    proposals: Mutex<HashMap<crate::contracts::ProposalId, ActionProposalRecord>>,
    audit: Mutex<Vec<AuditEvent>>,
}

impl FakeStore {
    pub fn insert_ticket(&self, ticket: SupportTicket) -> Result<(), SupportStoreError> {
        let mut tickets = self
            .tickets
            .lock()
            .map_err(|_| SupportStoreError::Unavailable)?;
        tickets.insert(ticket.id.clone(), ticket);
        Ok(())
    }
}

#[async_trait::async_trait]
impl SupportStore for FakeStore {
    async fn get_ticket(&self, ticket_id: &TicketId) -> Result<SupportTicket, SupportStoreError> {
        self.tickets
            .lock()
            .map_err(|_| SupportStoreError::Unavailable)?
            .get(ticket_id)
            .cloned()
            .ok_or(SupportStoreError::NotFound)
    }

    async fn list_messages(
        &self,
        ticket_id: &TicketId,
        _request: MessagePageRequest,
    ) -> Result<MessagePage, SupportStoreError> {
        self.get_ticket(ticket_id).await?;
        Ok(MessagePage {
            messages: Vec::new(),
            next_cursor: None,
        })
    }

    async fn save_ticket(
        &self,
        ticket: SupportTicket,
        expected_revision: u64,
    ) -> Result<(), SupportStoreError> {
        let mut tickets = self
            .tickets
            .lock()
            .map_err(|_| SupportStoreError::Unavailable)?;
        let current = tickets.get(&ticket.id).ok_or(SupportStoreError::NotFound)?;
        if current.revision != expected_revision {
            return Err(SupportStoreError::Conflict);
        }
        tickets.insert(ticket.id.clone(), ticket);
        Ok(())
    }

    async fn save_proposal(&self, proposal: ActionProposalRecord) -> Result<(), SupportStoreError> {
        self.proposals
            .lock()
            .map_err(|_| SupportStoreError::Unavailable)?
            .insert(proposal.id.clone(), proposal);
        Ok(())
    }

    async fn record_audit(&self, event: AuditEvent) -> Result<(), SupportStoreError> {
        self.audit
            .lock()
            .map_err(|_| SupportStoreError::AuditUnavailable)?
            .push(event);
        Ok(())
    }
}

/// Scripted mail source for ingest tests: queued pages/errors are returned in order;
/// once the script is exhausted it returns an empty final page. `calls` counts fetches.
pub struct FakeMailSource {
    script: Mutex<
        std::collections::VecDeque<Result<crate::ingest::MailPage, crate::ingest::MailSourceError>>,
    >,
    pub calls: std::sync::atomic::AtomicUsize,
    pub cursors: Mutex<Vec<Option<String>>>,
}

impl FakeMailSource {
    pub fn new(
        script: Vec<Result<crate::ingest::MailPage, crate::ingest::MailSourceError>>,
    ) -> Self {
        Self {
            script: Mutex::new(script.into()),
            calls: std::sync::atomic::AtomicUsize::new(0),
            cursors: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl crate::ingest::MailSource for FakeMailSource {
    async fn fetch_page(
        &self,
        cursor: Option<&str>,
        _limit: usize,
    ) -> Result<crate::ingest::MailPage, crate::ingest::MailSourceError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.cursors
            .lock()
            .map_err(|_| crate::ingest::MailSourceError::Unavailable)?
            .push(cursor.map(str::to_owned));
        self.script
            .lock()
            .map_err(|_| crate::ingest::MailSourceError::Unavailable)?
            .pop_front()
            .unwrap_or(Ok(crate::ingest::MailPage {
                messages: Vec::new(),
                next_cursor: None,
            }))
    }
}

/// Scripted Jev judge for ingest tests: returns canned raw model output (or an error)
/// in order and counts calls, so malformed-output and retry paths are observable.
pub struct FakeJevJudge {
    script: Mutex<std::collections::VecDeque<Result<String, crate::ingest::JudgeError>>>,
    pub calls: std::sync::atomic::AtomicUsize,
}

impl FakeJevJudge {
    pub fn new(script: Vec<Result<String, crate::ingest::JudgeError>>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl crate::ingest::ClassificationJudge for FakeJevJudge {
    async fn judge(
        &self,
        _subject: &str,
        _body_text: &str,
    ) -> Result<String, crate::ingest::JudgeError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.script
            .lock()
            .map_err(|_| crate::ingest::JudgeError::Unavailable)?
            .pop_front()
            .unwrap_or(Err(crate::ingest::JudgeError::Unusable))
    }
}

#[derive(Default)]
pub struct FakeMeter;

#[async_trait::async_trait]
impl Meter for FakeMeter {
    async fn record_call(&self, _call: MeteredCall) -> Result<(), MeterError> {
        Ok(())
    }
}
