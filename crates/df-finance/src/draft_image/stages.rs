//! Stage grants and durable quote identity on the existing account money workflow.

use super::commands::Settlement;
use super::{Error, Receipt, Usability, rows};
use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::FinanceService;
use df_commerce::digest::business_key;
use df_commerce::draft_image::{
    ConsentId, JobConsent, JobScope, Maximums, Refusal, StageBinding, StageCall, StageRoute,
};
use df_commerce::finance_ids::{AttemptId, EffectId, OperationKey, QuoteId, hex};
use df_types::AccountId;
use serde_json::{Value, json};
use std::future::Future;
use std::time::Duration;
use tokio_postgres::Transaction;

/// Pins the actual frozen cost route under an explicitly consented route envelope.
#[derive(Clone, Debug)]
pub struct StageQuoteCommand {
    pub consent: ConsentId,
    pub consent_revision: u64,
    pub scope: JobScope,
    pub call: StageCall,
    pub route: StageRoute,
    pub quote_lifetime_ms: u64,
}
impl StageQuoteCommand {
    fn fingerprint(&self) -> [u8; 32] {
        business_key(
            "df-finance/draft-stage-quote-v1",
            &[
                &self.consent.0,
                &self.consent_revision.to_be_bytes(),
                &self.scope.digest(),
                &self.call.digest(),
                &self.scope.selected_scope(&self.route).digest(),
                &self.route.key,
                self.route.cause.as_bytes(),
                self.route.detail.as_bytes(),
                &self.quote_lifetime_ms.to_be_bytes(),
            ],
        )
    }
}

/// Returned ids and expiry are immutable; an exact quote retry recovers this same tuple.
#[derive(Clone, Debug)]
pub struct StoredStageQuote {
    binding: StageBinding,
    call: StageCall,
    route: StageRoute,
    consent: ConsentId,
    consent_revision: u64,
    quote: QuoteId,
    fingerprint: [u8; 32],
    maximums: Maximums,
    expires_at_ms: i64,
}
impl StoredStageQuote {
    pub fn binding(&self) -> &StageBinding {
        &self.binding
    }
    pub fn call(&self) -> &StageCall {
        &self.call
    }
    pub fn route(&self) -> &StageRoute {
        &self.route
    }
    pub fn quote(&self) -> QuoteId {
        self.quote
    }
    pub fn consent(&self) -> ConsentId {
        self.consent
    }
    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
    pub fn maximums(&self) -> Maximums {
        self.maximums
    }
    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }
    pub fn operation(&self) -> OperationKey {
        OperationKey::from_array(self.binding.operation)
    }
    pub fn attempt(&self) -> AttemptId {
        AttemptId::from_array(self.binding.attempt)
    }
    pub fn effect(&self) -> EffectId {
        EffectId::from_array(self.binding.effect)
    }
    fn document(&self, action: &str) -> Value {
        let mut binding = json!(self.binding);
        binding["call"] = json!(self.call);
        binding["stage_route"] = json!(self.route);
        json!({"action":action,"consent":hex(&self.consent.0),"consent_revision":self.consent_revision,
            "quote":self.quote.hex(),"binding":binding,"fingerprint":hex(&self.fingerprint),
            "price_version":hex(&self.route.price_version),"operation":hex(&self.binding.operation),
            "attempt":hex(&self.binding.attempt),"effect":hex(&self.binding.effect),
            "asset":if self.call.kind==df_commerce::draft_image::PaidStageKind::Classification {
                Value::Null } else {self.binding.planned_publication.map_or(Value::Null,|p|json!(hex(&p.asset.0)))}})
    }
}

/// The money receipt and stored execution/cost identity share a single transaction.
#[derive(Clone, Debug)]
pub struct StageReceipt {
    pub quote: StoredStageQuote,
    pub money: Receipt,
}

/// NEW must prepare the exact hosted dispatch; replay must verify without repair or allocation.
pub trait StageAdmissionPort: Send + Sync {
    fn bind(
        &self,
        transaction: &Transaction<'_>,
        receipt: &StageReceipt,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

/// Evidence strength is part of settlement identity; unknown never releases a hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageUsageEvidence {
    Reported,
    Estimated,
    Unknown,
}
impl StageUsageEvidence {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Estimated => "estimated",
            Self::Unknown => "unknown",
        }
    }
}

/// The host supplies the exact durable usage revision, never an unaudited usage guess.
#[derive(Clone, Debug)]
pub struct StageSettlementCommand {
    pub quote: StoredStageQuote,
    pub usage_revision: u32,
    pub evidence: StageUsageEvidence,
    pub actual_units: Vec<(String, u64)>,
    pub usability: Usability,
}
/// Verification precedes money posting; the execution mirror follows it in the same transaction.
pub trait StageSettlementPort: Send + Sync {
    fn verify(
        &self,
        transaction: &Transaction<'_>,
        command: &StageSettlementCommand,
    ) -> impl Future<Output = Result<(), Error>> + Send;
    fn mirror(
        &self,
        transaction: &Transaction<'_>,
        command: &StageSettlementCommand,
        settlement: &Settlement,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    pub async fn record_draft_job_consent(
        &mut self,
        account: AccountId,
        consent: &JobConsent,
    ) -> Result<u64, Error> {
        consent
            .validate(account, self.clock.now_ms())
            .map_err(Error::Policy)?;
        let mut scope = json!(consent.scope);
        scope["scope_kind"] = json!("job");
        let result = self
            .draft_command(
                account,
                json!({"action":"consent","consent":hex(&consent.id.0),
            "scope":scope,"scope_digest":hex(&consent.scope.digest()),
            "cap_micro_dollars":consent.cap_micro_dollars,"expires_at_ms":consent.expires_at_ms}),
            )
            .await?;
        rows::uint(&result, "revision")
    }

    pub async fn quote_draft_stage(
        &mut self,
        account: AccountId,
        command: &StageQuoteCommand,
    ) -> Result<StoredStageQuote, Error> {
        command.scope.validate(account).map_err(Error::Policy)?;
        command
            .call
            .validate(&command.scope, &command.route)
            .map_err(Error::Policy)?;
        if command.consent.0 == [0; 16]
            || command.consent_revision == 0
            || command.quote_lifetime_ms == 0
            || command.quote_lifetime_ms > i64::MAX as u64
        {
            return Err(Error::Policy(Refusal::InvalidConsent));
        }
        let binding = StageBinding {
            scope: command.scope.selected_scope(&command.route),
            stage: command.call.stage,
            planned_publication: command.call.planned_publication,
            operation: self.id(),
            attempt: self.id(),
            effect: self.id(),
            payload_digest: command.call.payload_digest,
        };
        let mut bound = json!(binding);
        bound["call"] = json!(command.call);
        bound["stage_route"] = json!(command.route);
        let mut document = json!({"action":"stage_quote","consent":hex(&command.consent.0),
            "consent_revision":command.consent_revision,"quote":hex(&self.id()),"binding":bound,
            "stage_fingerprint":hex(&command.fingerprint()),"quote_lifetime_ms":command.quote_lifetime_ms});
        self.stamp_draft_document(&mut document);
        let transaction = Self::start(&mut self.client)
            .await
            .map_err(Error::Finance)?;
        let result = rows::execute_stage(&transaction, account, &document).await?;
        let stored = parse_quote(&result)?;
        transaction.commit().await.map_err(rows::commit_unknown)?;
        Ok(stored)
    }

    pub async fn admit_draft_stage<P: StageAdmissionPort>(
        &mut self,
        account: AccountId,
        quote: &StoredStageQuote,
        port: &P,
    ) -> Result<StageReceipt, Error> {
        quote.binding.validate(account).map_err(Error::Policy)?;
        let mut document = quote.document("admit");
        self.stamp_draft_document(&mut document);
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let transaction = Self::start(&mut self.client)
            .await
            .map_err(Error::Finance)?;
        let result = rows::execute(&transaction, account, &document).await?;
        let receipt = StageReceipt {
            quote: quote.clone(),
            money: rows::receipt(&result, quote.fingerprint)?,
        };
        tokio::time::timeout(deadline, port.bind(&transaction, &receipt))
            .await
            .map_err(|_| Error::PortUnavailable)??;
        transaction.commit().await.map_err(rows::commit_unknown)?;
        Ok(receipt)
    }

    pub async fn settle_draft_stage<P: StageSettlementPort>(
        &mut self,
        account: AccountId,
        command: &StageSettlementCommand,
        port: &P,
    ) -> Result<Settlement, Error> {
        command
            .quote
            .binding
            .validate(account)
            .map_err(Error::Policy)?;
        if command.usage_revision == 0 {
            return Err(Error::InvalidRecord);
        }
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let mut document =
            command
                .quote
                .document(if command.evidence == StageUsageEvidence::Unknown {
                    "unknown"
                } else {
                    "settle"
                });
        document["actual_units"] = json!(command.actual_units);
        document["usability"] = json!(command.usability.label());
        document["usage_revision"] = json!(command.usage_revision);
        document["evidence"] = json!(command.evidence.label());
        self.stamp_draft_document(&mut document);
        let transaction = Self::start(&mut self.client)
            .await
            .map_err(Error::Finance)?;
        // Finance precedes raw usage and the terminal imagery proof in the global lock order.
        let mut locked = document.clone();
        locked["action"] = json!("lock");
        rows::execute_stage(&transaction, account, &locked).await?;
        // This is a durable read, not a provider call, and keeps the reservation untouched on refusal.
        tokio::time::timeout(deadline, port.verify(&transaction, command))
            .await
            .map_err(|_| Error::PortUnavailable)??;
        let result = rows::execute_stage(&transaction, account, &document).await?;
        let settlement = if command.evidence == StageUsageEvidence::Unknown {
            Settlement {
                charged_micro_dollars: 0,
                released_micro_dollars: 0,
                replay: !rows::is_new(&result)?,
            }
        } else {
            rows::settlement(&result)?
        };
        tokio::time::timeout(deadline, port.mirror(&transaction, command, &settlement))
            .await
            .map_err(|_| Error::PortUnavailable)??;
        transaction.commit().await.map_err(rows::commit_unknown)?;
        Ok(settlement)
    }
}

fn parse_quote(result: &Value) -> Result<StoredStageQuote, Error> {
    let mut binding = result.get("binding").cloned().ok_or(Error::InvalidRecord)?;
    let call = serde_json::from_value(binding.get("call").cloned().ok_or(Error::InvalidRecord)?)
        .map_err(|_| Error::InvalidRecord)?;
    let route = serde_json::from_value(
        binding
            .get("stage_route")
            .cloned()
            .ok_or(Error::InvalidRecord)?,
    )
    .map_err(|_| Error::InvalidRecord)?;
    let object = binding.as_object_mut().ok_or(Error::InvalidRecord)?;
    object.remove("call");
    object.remove("stage_route");
    object.remove("stage_fingerprint");
    let text = |key| {
        result
            .get(key)
            .and_then(Value::as_str)
            .ok_or(Error::InvalidRecord)
    };
    let fingerprint = parse_bytes::<32>(text("fingerprint")?).ok_or(Error::InvalidRecord)?;
    Ok(StoredStageQuote {
        binding: serde_json::from_value(binding).map_err(|_| Error::InvalidRecord)?,
        call,
        route,
        consent: ConsentId(parse_bytes::<16>(text("consent")?).ok_or(Error::InvalidRecord)?),
        consent_revision: rows::uint(result, "consent_revision")?,
        quote: QuoteId::from_hex(text("quote")?).ok_or(Error::InvalidRecord)?,
        fingerprint,
        maximums: Maximums {
            retail_micro_dollars: rows::uint(result, "retail")?,
            supplier_micro_dollars: rows::uint(result, "supplier")?,
        },
        expires_at_ms: result
            .get("expires_at_ms")
            .and_then(Value::as_i64)
            .ok_or(Error::InvalidRecord)?,
    })
}

fn parse_bytes<const N: usize>(text: &str) -> Option<[u8; N]> {
    df_commerce::finance_ids::unhex(text)?.try_into().ok()
}

/// Source winners and canonical dispatch authority are locked before committing first Sent.
pub trait StageSendPort: Send + Sync {
    fn verify(
        &self,
        tx: &Transaction<'_>,
        quote: &StoredStageQuote,
        new_send: bool,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}
impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    pub async fn mark_draft_stage_sent<P: StageSendPort>(
        &mut self,
        account: AccountId,
        quote: &StoredStageQuote,
        dispatcher: [u8; 16],
        port: &P,
    ) -> Result<bool, Error> {
        quote.binding.validate(account).map_err(Error::Policy)?;
        if dispatcher == [0; 16] {
            return Err(Error::InvalidRecord);
        }
        let mut document = quote.document("sent");
        document["dispatcher"] = json!(hex(&dispatcher));
        self.stamp_draft_document(&mut document);
        let prepared_event = hex(&self.id());
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let tx = Self::start(&mut self.client)
            .await
            .map_err(Error::Finance)?;
        // Preparation and Sent are both registered lifecycle events in this same transaction.
        let mut prepared = document.clone();
        prepared["action"] = json!("prepare");
        prepared["event"] = json!(prepared_event);
        let state=tx.query_one("SELECT dispatch_state FROM finance.provider_attempt WHERE attempt_id=$1 AND account_id=$2",
            &[&quote.binding.attempt.as_slice(),&account.as_bytes().as_slice()]).await.map_err(|e|Error::Finance(e.into()))?;
        if state.get::<_, String>(0) == "reserved" {
            rows::execute_stage(&tx, account, &prepared).await?;
        }
        let result = rows::execute_stage(&tx, account, &document).await?;
        let new_send = rows::is_new(&result)?;
        tokio::time::timeout(deadline, port.verify(&tx, quote, new_send))
            .await
            .map_err(|_| Error::PortUnavailable)??;
        tx.commit().await.map_err(rows::commit_unknown)?;
        Ok(new_send)
    }
}

impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    /// Recovers an already stored quote without minting ids or evaluating a new grant.
    pub async fn read_draft_stage_quote(
        &self,
        account: AccountId,
        call: &StageCall,
    ) -> Result<Option<StoredStageQuote>, Error> {
        let row=self.client.query_opt("SELECT jsonb_build_object('quote',encode(q.quote_id,'hex'), \
            'consent',encode(q.consent_id,'hex'),'consent_revision',q.consent_revision,'binding',q.draft_binding, \
            'fingerprint',encode(q.request_digest,'hex'),'retail',q.retail_maximum_micro_dollars, \
            'supplier',q.supplier_maximum_micro_dollars,'expires_at_ms',q.expires_at_ms)::text \
            FROM finance.quote q WHERE q.account_id=$1 AND q.draft_binding->'call'->'job'=$2::text::jsonb \
            AND q.draft_binding->'call'->'stage'=$3::text::jsonb AND q.draft_binding->'call'->'dispatch'=$4::text::jsonb",
            &[&account.as_bytes().as_slice(),&json!(call.job).to_string(),&json!(call.stage).to_string(),&json!(call.dispatch).to_string()])
            .await.map_err(|error|Error::Finance(error.into()))?;
        let Some(row) = row else { return Ok(None) };
        let value: Value = serde_json::from_str(
            &row.try_get::<_, String>(0)
                .map_err(|_| Error::InvalidRecord)?,
        )
        .map_err(|_| Error::InvalidRecord)?;
        let quote = parse_quote(&value)?;
        if quote.call() != call {
            return Err(Error::LinkConflict);
        }
        Ok(Some(quote))
    }
}

#[path = "stages/cancellation.rs"]
mod cancellation;
pub use cancellation::StageCancellationPort;
