//! Owned bounded mailbox sync, durable classification claims and manual fallback. No reply transport.
mod identity;
mod providers;
#[path = "../../df-persistence/src/support_store/ingestion.rs"]
mod state;

use std::{sync::Arc, time::Duration};

use df_support::{
    classification::parse_judgement,
    contracts::{
        CallId, JobId, MessageDirection, MessageId, MeterOutcome, MeteredCall, PartyId, TicketId,
    },
    ingest::{
        ClassificationJudge, InboundMail, MAXIMUM_BATCH, MailSource, MailSourceError, MailboxLease,
    },
};
use tokio::task::JoinHandle;

use identity::identifier;
pub(crate) use providers::from_env;

const MAXIMUM_FETCH_ATTEMPTS: usize = 3;
const CALL_DEADLINE: Duration = Duration::from_secs(15);
const SLICE_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IngestConfig {
    batch_limit: usize,
    max_fetch_attempts: usize,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            batch_limit: MAXIMUM_BATCH,
            max_fetch_attempts: MAXIMUM_FETCH_ATTEMPTS,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct IngestReport {
    fetched: usize,
    inserted: usize,
    duplicates: usize,
    retired: usize,
    classified: usize,
    unclassified: usize,
    fetch_attempts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum IngestError {
    SourceUnavailable,
    SourceRejected,
    StoreUnavailable,
    InvalidMail,
    DeadlineExceeded,
    Deferred,
}

/// A second connection isolates sync leases/audit from the existing support content adapter.
enum State {
    Ready(Arc<state::IngestState>),
    Connect(Box<crate::database::Connector>),
}

pub(crate) struct SupportIngest<S: MailSource, J: ClassificationJudge> {
    source: S,
    judge: J,
    store: Arc<df_persistence::support_store::SupportStore>,
    state: State,
    state_driver: Option<crate::database::Driver>,
    mailbox_id: String,
    mailbox_address: String,
    config: IngestConfig,
    retry_at_unix_s: i64,
    failures: usize,
}

impl<S: MailSource, J: ClassificationJudge> SupportIngest<S, J> {
    fn new(
        source: S,
        judge: J,
        store: Arc<df_persistence::support_store::SupportStore>,
        mailbox_id: String,
        mailbox_address: String,
        config: IngestConfig,
        state: State,
    ) -> Self {
        Self {
            source,
            judge,
            store,
            state,
            state_driver: None,
            mailbox_id,
            mailbox_address,
            config,
            retry_at_unix_s: 0,
            failures: 0,
        }
    }

    async fn state(&mut self) -> Result<Arc<state::IngestState>, IngestError> {
        if let State::Connect(connector) = &self.state {
            let connection =
                crate::database::Connection::open(&connector.config, &connector.transport)
                    .await
                    .map_err(|_| IngestError::StoreUnavailable)?;
            self.state = State::Ready(Arc::new(state::IngestState::new(connection.client)));
            self.state_driver = Some(connection.driver);
        }
        match &self.state {
            State::Ready(state) => Ok(Arc::clone(state)),
            State::Connect(_) => Err(IngestError::StoreUnavailable),
        }
    }

    async fn run_once(&mut self, now_unix_s: i64) -> Result<IngestReport, IngestError> {
        if now_unix_s < self.retry_at_unix_s {
            return Err(IngestError::Deferred);
        }
        let state = self.state().await?;
        let Some(lease) = state
            .claim_mailbox(&self.mailbox_id, &self.mailbox_address, now_unix_s)
            .await
            .map_err(|_| IngestError::StoreUnavailable)?
        else {
            return Err(IngestError::Deferred);
        };
        let context = df_observe::OperationContext {
            trace_parent: String::new(),
            build: concat!("df-hosted/", env!("CARGO_PKG_VERSION")).into(),
        };
        let mut span = df_observe::begin(&context, "hosted.support_ingest_slice");
        span.event("mailbox_claimed", &[("fence", lease.fence.to_string())]);
        let result = tokio::time::timeout(
            SLICE_DEADLINE,
            self.ingest_slice(&state, &lease, now_unix_s),
        )
        .await
        .unwrap_or(Err(IngestError::DeadlineExceeded));
        let (cursor, succeeded) = match &result {
            Ok((_, cursor)) => (cursor.as_deref(), true),
            Err(_) => (lease.cursor.as_deref(), false),
        };
        if !succeeded {
            self.failures = (self.failures + 1).min(MAXIMUM_FETCH_ATTEMPTS);
            let backoff_s = 30_i64 * (1_i64 << self.failures);
            self.retry_at_unix_s = self
                .retry_at_unix_s
                .max(now_unix_s.saturating_add(backoff_s));
        }
        state
            .finish_mailbox(
                &self.mailbox_id,
                &lease,
                cursor,
                succeeded,
                (!succeeded).then_some(self.retry_at_unix_s),
            )
            .await
            .map_err(|_| IngestError::StoreUnavailable)?;
        span.finish_unmeasured(if succeeded {
            "complete"
        } else {
            "retry_pending"
        });
        match result {
            Ok((report, _)) => {
                self.failures = 0;
                self.retry_at_unix_s = 0;
                audit(format!(
                    "HOSTED support_ingest status=complete fence={} fetched={} inserted={} duplicates={} classified={} unclassified={}",
                    lease.fence,
                    report.fetched,
                    report.inserted,
                    report.duplicates,
                    report.classified,
                    report.unclassified
                ));
                Ok(report)
            }
            Err(error) => {
                audit(format!(
                    "HOSTED support_ingest status=retry_pending fence={} reason={error:?} retry_at_unix_s={}",
                    lease.fence, self.retry_at_unix_s
                ));
                Err(error)
            }
        }
    }

    async fn ingest_slice(
        &mut self,
        state: &state::IngestState,
        lease: &MailboxLease,
        now_unix_s: i64,
    ) -> Result<(IngestReport, Option<String>), IngestError> {
        let (page, fetch_attempts) = self.fetch_page(lease, now_unix_s).await?;
        let mut report = IngestReport {
            fetched: page.messages.len(),
            fetch_attempts,
            ..IngestReport::default()
        };
        for mail in &page.messages {
            let admitted = self.admit_mail(state, lease, mail, now_unix_s).await?;
            report.inserted += admitted.inserted;
            report.duplicates += admitted.duplicates;
            report.retired += admitted.retired;
            report.classified += admitted.classified;
            report.unclassified += admitted.unclassified;
        }
        Ok((report, page.next_cursor))
    }

    async fn fetch_page(
        &mut self,
        lease: &MailboxLease,
        now_unix_s: i64,
    ) -> Result<(df_support::ingest::MailPage, usize), IngestError> {
        let limit = self.config.batch_limit.clamp(1, MAXIMUM_BATCH);
        let mut attempts = 0;
        let mut cursor = lease.cursor.clone();
        let page = loop {
            attempts += 1;
            let response = tokio::time::timeout(
                Duration::from_secs(55),
                self.source.fetch_page(cursor.as_deref(), limit),
            )
            .await;
            match response {
                Ok(Ok(page)) => break page,
                Ok(Err(MailSourceError::CursorExpired)) if cursor.is_some() => {
                    cursor = None;
                    audit(format!(
                        "HOSTED support_ingest status=cursor_recovery fence={}",
                        lease.fence
                    ));
                }
                Ok(Err(MailSourceError::RateLimited { retry_after_s })) => {
                    let delay = i64::try_from(retry_after_s).unwrap_or(i64::MAX);
                    self.retry_at_unix_s = now_unix_s.saturating_add(delay);
                    return Err(IngestError::SourceUnavailable);
                }
                Ok(Err(MailSourceError::Rejected)) => return Err(IngestError::SourceRejected),
                _ => return Err(IngestError::SourceUnavailable),
            }
            if attempts
                >= self
                    .config
                    .max_fetch_attempts
                    .clamp(1, MAXIMUM_FETCH_ATTEMPTS)
            {
                return Err(IngestError::SourceUnavailable);
            }
        };
        if page.messages.len() > limit {
            return Err(IngestError::InvalidMail);
        }
        Ok((page, attempts))
    }

    async fn admit_mail(
        &self,
        state: &state::IngestState,
        lease: &MailboxLease,
        mail: &InboundMail,
        now_unix_s: i64,
    ) -> Result<IngestReport, IngestError> {
        let mut report = IngestReport::default();
        let input = to_inbound(mail, &self.mailbox_id, &self.mailbox_address)?;
        let outcome = self
            .store
            .ingest(input)
            .await
            .map_err(|_| IngestError::StoreUnavailable)?;
        let ticket_id = match outcome {
            df_persistence::support_store::IngestOutcome::Inserted { ticket_id } => {
                report.inserted += 1;
                ticket_id
            }
            df_persistence::support_store::IngestOutcome::Duplicate { ticket_id } => {
                report.duplicates += 1;
                ticket_id
            }
            df_persistence::support_store::IngestOutcome::Retired => {
                report.retired += 1;
                return Ok(report);
            }
        };
        state
            .record_message_metadata(&self.mailbox_id, lease, mail)
            .await
            .map_err(|_| IngestError::StoreUnavailable)?;
        if mail.direction == MessageDirection::Outbound {
            return Ok(report);
        }
        let classification_id = identifier("cls", &self.mailbox_id, &mail.provider_message_id);
        let Some(revision) = state
            .claim_classification(
                &self.mailbox_id,
                lease,
                &ticket_id,
                &classification_id,
                now_unix_s,
                &mail.subject,
            )
            .await
            .map_err(|_| IngestError::StoreUnavailable)?
        else {
            return Ok(report);
        };
        let meter = ModelMeter {
            state,
            mailbox_id: &self.mailbox_id,
            lease,
            now_unix_s,
        };
        let basis = metering_basis(ticket_id, &classification_id)?;
        let operation_hash = df_observe::operation_hash(classification_id.as_bytes());
        audit(format!(
            "HOSTED support_classification status=started operation_hash={operation_hash} source_revision={revision}"
        ));
        let data = self.classify(mail, revision, &meter, basis).await;
        state
            .complete_classification(
                &self.mailbox_id,
                lease,
                &classification_id,
                &data,
                now_unix_s,
            )
            .await
            .map_err(|_| IngestError::StoreUnavailable)?;
        audit(format!(
            "HOSTED support_classification status=stored operation_hash={operation_hash} source_revision={revision}"
        ));
        if data["status"] == "classified" {
            report.classified += 1
        } else {
            report.unclassified += 1
        }
        Ok(report)
    }

    async fn classify(
        &self,
        mail: &InboundMail,
        revision: u64,
        meter: &dyn df_support::ports::Meter,
        basis: MeteredCall,
    ) -> serde_json::Value {
        let result = if mail.intake_incomplete {
            Err(df_support::ingest::JudgeError::Unusable)
        } else {
            tokio::time::timeout(
                CALL_DEADLINE,
                self.judge
                    .judge_metered(&mail.subject, &mail.body_text, meter, basis),
            )
            .await
            .unwrap_or(Err(df_support::ingest::JudgeError::DeadlineExceeded))
            .and_then(|raw| parse_judgement(&raw))
        };
        match result {
            Ok((draft, evidence)) => {
                serde_json::json!({"status":"classified","attempt_state":"complete",
                "category":category_name(draft.category),"triage_category":draft.category,
                "urgency":draft.urgency,"confidence":draft.confidence,"needs_review":draft.needs_review(),
                "suggested_action":draft.suggested_action,"evidence":evidence,"source_revision":revision,
                "schema":"dungeonflux.support-ingest-classification/1"})
            }
            Err(reason) => {
                audit(format!(
                    "HOSTED support_ingest status=unclassified reason={reason:?}"
                ));
                serde_json::json!({"status":"unclassified","attempt_state":"complete",
                    "reason":format!("{reason:?}"),"source_revision":revision,
                    "schema":"dungeonflux.support-ingest-classification/1"})
            }
        }
    }
}

fn metering_basis(
    ticket_id: TicketId,
    classification_id: &str,
) -> Result<MeteredCall, IngestError> {
    Ok(MeteredCall {
        call_id: CallId::new(classification_id.to_owned()).map_err(|_| IngestError::InvalidMail)?,
        job_id: JobId::new(classification_id.to_owned()).map_err(|_| IngestError::InvalidMail)?,
        ticket_id,
        attempt: 1,
        provider: String::new(),
        model: String::new(),
        prompt_revision: "support-ingest/1".into(),
        input_tokens: None,
        output_tokens: None,
        estimated_micro_dollars: None,
        actual_micro_dollars: None,
        outcome: MeterOutcome::Unknown,
        uncertainty: true,
    })
}

struct ModelMeter<'a> {
    state: &'a state::IngestState,
    mailbox_id: &'a str,
    lease: &'a MailboxLease,
    now_unix_s: i64,
}

#[async_trait::async_trait]
impl df_support::ports::Meter for ModelMeter<'_> {
    async fn record_call(&self, call: MeteredCall) -> Result<(), df_support::ports::MeterError> {
        self.state
            .record_model_call(self.mailbox_id, self.lease, call, self.now_unix_s)
            .await
            .map_err(|_| df_support::ports::MeterError::Unavailable)
    }
}

fn category_name(category: df_support::ingest::SupportCategory) -> &'static str {
    use df_support::ingest::SupportCategory;
    match category {
        SupportCategory::Billing => "billing_refund",
        SupportCategory::Bug => "bug_report",
        SupportCategory::Legal => "legal_notice_takedown_ip",
        SupportCategory::Account => "account_access",
        SupportCategory::Other => "undetermined",
    }
}

fn to_inbound(
    mail: &InboundMail,
    mailbox_id: &str,
    mailbox_address: &str,
) -> Result<df_persistence::support_store::InboundMessage, IngestError> {
    if mail.provider_message_id.is_empty()
        || mail.provider_message_id.len() > 512
        || mail.provider_thread_id.is_empty()
        || mail.provider_thread_id.len() > 512
        || mail.sender_address.len() > 2048
        || mail.recipient_addresses.len() > 100
    {
        return Err(IngestError::InvalidMail);
    }
    let party = |address: &str| {
        PartyId::new(identifier("party", mailbox_id, address)).map_err(|_| IngestError::InvalidMail)
    };
    Ok(df_persistence::support_store::InboundMessage {
        ticket_id: TicketId::new(identifier("ticket", mailbox_id, &mail.provider_thread_id))
            .map_err(|_| IngestError::InvalidMail)?,
        message_id: MessageId::new(identifier("msg", mailbox_id, &mail.provider_message_id))
            .map_err(|_| IngestError::InvalidMail)?,
        mailbox_id: mailbox_id.to_owned(),
        mailbox_address: mailbox_address.to_owned(),
        gmail_message_id: mail.provider_message_id.clone(),
        gmail_thread_id: mail.provider_thread_id.clone(),
        received_at_unix_s: mail.received_at_unix_s,
        sender: party(&mail.sender_address)?,
        sender_address_ref: mail.sender_address.clone(),
        sender_display_name: mail.sender_display_name.clone(),
        recipients: mail
            .recipient_addresses
            .iter()
            .map(|address| Ok((party(address)?, address.clone())))
            .collect::<Result<_, IngestError>>()?,
        body_text: df_support::ingest::bound_body(&mail.body_text),
    })
}

fn audit(line: String) {
    df_observe::record_audit_line("hosted.support_ingest", line);
}

pub(crate) struct WorkerHandle(Option<JoinHandle<()>>);

impl WorkerHandle {
    pub(crate) async fn shutdown(mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
            if let Err(error) = task.await {
                if !error.is_cancelled() {
                    audit("HOSTED support_ingest status=shutdown_failed".into());
                }
            }
        }
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

pub(crate) fn spawn<S, J>(mut worker: SupportIngest<S, J>, interval: Duration) -> WorkerHandle
where
    S: MailSource + 'static,
    J: ClassificationJudge + 'static,
{
    WorkerHandle(Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval.max(Duration::from_secs(30)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                Ok(duration) => match i64::try_from(duration.as_secs()) {
                    Ok(now) => now,
                    Err(_) => {
                        audit("HOSTED support_ingest status=clock_unavailable".into());
                        continue;
                    }
                },
                Err(_) => {
                    audit("HOSTED support_ingest status=clock_unavailable".into());
                    continue;
                }
            };
            if let Err(error) = worker.run_once(now).await {
                audit(format!(
                    "HOSTED support_ingest status=slice_failed reason={error:?}"
                ));
            }
        }
    })))
}

#[cfg(test)]
mod tests;
