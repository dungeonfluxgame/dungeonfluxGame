//! Account-owned portrait funding, explicitly approved 2026-10-28.
//! Every durable mutation uses the existing finance journal in migration 0043.
//! Admission requires a catalog/journal port in the same transaction; it cannot send.

use std::future::Future;
use std::time::Duration;

use df_commerce::digest::business_key;
use df_commerce::draft_image::{Binding, Consent, ConsentId, Maximums, Refusal};
use df_commerce::finance_ids::{AttemptId, ReservationId, hex};
use df_types::AccountId;
use serde_json::{Value, json};
use tokio_postgres::Transaction;

use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::{FinanceError, FinanceService};

mod commands;
mod rows;
mod stages;
pub use commands::{AdmissionCommand, QuoteCommand, Settlement, SettlementCommand, Usability};
pub use stages::{
    StageAdmissionPort, StageCancellationPort, StageQuoteCommand, StageReceipt, StageSendPort,
    StageSettlementCommand, StageSettlementPort, StageUsageEvidence, StoredStageQuote,
};

#[derive(Debug)]
pub enum Error {
    Policy(Refusal),
    Refused(AdmissionRefusal),
    Finance(FinanceError),
    InvalidRecord,
    PortUnavailable,
    LinkConflict,
    CatalogRefused,
    EconomicProofPending,
    /// Replay the identical operation before taking any further action.
    CommitUnknown {
        sqlstate: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionRefusal {
    InvalidTransaction,
    AccountUnavailable,
    Conflict,
    MissingOrForeign,
    Revoked,
    ConsentExpired,
    BindingChanged,
    QuoteExpired,
    ConsentCapExceeded,
    InvalidUnits,
    Unpriced,
    TooManyFundingLots,
    InsufficientFunds,
    InvalidDispatch,
    PotentiallySent,
    RestrictedLot,
    SupplierBlocked,
    CurrencyMismatch,
}

/// Stored finance evidence; never a provider dispatch permit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub reservation: ReservationId,
    pub attempt: AttemptId,
    pub maximums: Maximums,
    pub price_version: [u8; 16],
    pub fingerprint: [u8; 32],
    pub admitted_at_ms: i64,
    pub replay: bool,
}

/// The composition owner must admit or verify the exact private account media
/// and frozen imagery mapping. A replay is read-only and cannot repair a link.
pub trait AdmissionPort: Send + Sync {
    fn bind(
        &self,
        transaction: &Transaction<'_>,
        binding: &Binding,
        receipt: &Receipt,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

/// Includes quote and consent identity; switching quotes is changed payload.
pub fn admission_fingerprint(command: &AdmissionCommand) -> [u8; 32] {
    business_key(
        "df-finance/draft-image-admission-v1",
        &[
            &command.binding.digest(),
            command.quote.as_bytes(),
            &command.consent.0,
            &command.consent_revision.to_be_bytes(),
        ],
    )
}

impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    /// The host supplies source-authenticated account identity, separately from terms.
    pub async fn record_draft_image_consent(
        &mut self,
        account: AccountId,
        consent: &Consent,
    ) -> Result<u64, Error> {
        consent
            .validate(account, self.clock.now_ms())
            .map_err(Error::Policy)?;
        if consent.revoked || consent.revision != 1 {
            return Err(Error::Policy(Refusal::InvalidConsent));
        }
        let document = json!({"action":"consent", "consent":hex(&consent.id.0),
            "scope":consent.scope, "scope_digest":hex(&consent.scope.digest()),
            "cap_micro_dollars":consent.cap_micro_dollars,
            "expires_at_ms":consent.expires_at_ms});
        let result = self.draft_command(account, document).await?;
        rows::uint(&result, "revision")
    }

    pub async fn revoke_draft_image_consent(
        &mut self,
        account: AccountId,
        consent: ConsentId,
    ) -> Result<(), Error> {
        self.draft_command(
            account,
            json!({"action":"revoke", "consent":hex(&consent.0)}),
        )
        .await?;
        Ok(())
    }

    pub async fn quote_draft_image(
        &mut self,
        account: AccountId,
        command: &QuoteCommand,
    ) -> Result<Maximums, Error> {
        command
            .admission
            .binding
            .validate(account)
            .map_err(Error::Policy)?;
        let mut document = rows::admission_document(&command.admission);
        document["action"] = json!("quote");
        document["expires_at_ms"] = json!(command.expires_at_ms);
        let result = self.draft_command(account, document).await?;
        Ok(Maximums {
            retail_micro_dollars: rows::uint(&result, "retail")?,
            supplier_micro_dollars: rows::uint(&result, "supplier")?,
        })
    }

    pub async fn admit_draft_image<P: AdmissionPort>(
        &mut self,
        account: AccountId,
        command: &AdmissionCommand,
        port: &P,
    ) -> Result<Receipt, Error> {
        command.binding.validate(account).map_err(Error::Policy)?;
        let mut document = rows::admission_document(command);
        document["action"] = json!("admit");
        self.stamp_draft_document(&mut document);
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let transaction = Self::start(&mut self.client)
            .await
            .map_err(Error::Finance)?;
        let result = rows::execute(&transaction, account, &document).await?;
        let receipt = rows::receipt(&result, admission_fingerprint(command))?;
        match tokio::time::timeout(
            deadline,
            port.bind(&transaction, &command.binding, &receipt),
        )
        .await
        {
            Ok(outcome) => outcome?,
            Err(_) => return Err(Error::PortUnavailable),
        }
        transaction.commit().await.map_err(rows::commit_unknown)?;
        Ok(receipt)
    }

    /// Commit dispatch state before bytes leave. Replay never grants another send.
    pub async fn prepare_draft_image(
        &mut self,
        account: AccountId,
        attempt: AttemptId,
    ) -> Result<bool, Error> {
        let result = self
            .draft_command(
                account,
                json!({"action":"prepare", "attempt":attempt.hex()}),
            )
            .await?;
        rows::is_new(&result)
    }

    pub async fn mark_draft_image_sent(
        &mut self,
        account: AccountId,
        attempt: AttemptId,
        dispatcher: [u8; 16],
    ) -> Result<bool, Error> {
        if dispatcher == [0; 16] {
            return Err(Error::Refused(AdmissionRefusal::InvalidDispatch));
        }
        let result = self
            .draft_command(
                account,
                json!({"action":"sent", "attempt":attempt.hex(),
            "dispatcher":hex(&dispatcher)}),
            )
            .await?;
        rows::is_new(&result)
    }

    pub async fn mark_draft_image_unknown(
        &mut self,
        account: AccountId,
        attempt: AttemptId,
    ) -> Result<(), Error> {
        self.draft_command(
            account,
            json!({"action":"unknown", "attempt":attempt.hex()}),
        )
        .await?;
        Ok(())
    }

    pub async fn settle_draft_image(
        &mut self,
        account: AccountId,
        command: &SettlementCommand,
    ) -> Result<commands::Settlement, Error> {
        let result = self
            .draft_command(
                account,
                json!({"action":"settle",
            "attempt":command.attempt.hex(), "actual_units":command.actual_units,
            "usability":command.usability.label()}),
            )
            .await?;
        rows::settlement(&result)
    }

    pub async fn release_unsent_draft_image(
        &mut self,
        account: AccountId,
        attempt: AttemptId,
    ) -> Result<commands::Settlement, Error> {
        let result = self
            .draft_command(
                account,
                json!({"action":"release", "attempt":attempt.hex()}),
            )
            .await?;
        rows::settlement(&result)
    }

    fn stamp_draft_document(&self, document: &mut Value) {
        document["now_ms"] = json!(self.clock.now_ms());
        document["event"] = json!(hex(&self.id()));
        document["entry"] = json!(hex(&self.id()));
        document["reservation"] = json!(hex(&self.id()));
        document["usage"] = json!(hex(&self.id()));
    }

    async fn draft_command(
        &mut self,
        account: AccountId,
        mut document: Value,
    ) -> Result<Value, Error> {
        self.stamp_draft_document(&mut document);
        let transaction = Self::start(&mut self.client)
            .await
            .map_err(Error::Finance)?;
        let result = rows::execute(&transaction, account, &document).await?;
        transaction.commit().await.map_err(rows::commit_unknown)?;
        Ok(result)
    }
}
