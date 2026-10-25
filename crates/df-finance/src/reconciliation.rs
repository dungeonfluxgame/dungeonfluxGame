//! Read-only comparisons between journal references and source records from processor, supplier,
//! and dispatch systems. This module never repairs or posts a balancing entry.

use std::{collections::BTreeMap, future::Future};

use df_commerce::ledger::AccountCode;
use serde::Serialize;
use tokio_postgres::Client;

use crate::service::FinanceError;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
/// The source system and reference class reconciled against a journal entry.
pub enum ReferenceKind {
    ProcessorCapture,
    ProcessorRefund,
    ProcessorDispute,
    SupplierInvoice,
    Dispatch,
}

impl ReferenceKind {
    fn label(self) -> &'static str {
        match self {
            Self::ProcessorCapture => "processor_payment",
            Self::ProcessorRefund => "processor_refund",
            Self::ProcessorDispute => "processor_dispute",
            Self::SupplierInvoice => "supplier_invoice",
            Self::Dispatch => "attempt",
        }
    }

    fn parse(label: &str) -> Option<Self> {
        match label {
            "processor_payment" => Some(Self::ProcessorCapture),
            "processor_refund" => Some(Self::ProcessorRefund),
            "processor_dispute" => Some(Self::ProcessorDispute),
            "supplier_invoice" => Some(Self::SupplierInvoice),
            "attempt" => Some(Self::Dispatch),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One signed account delta from a processor, invoice, or dispatch source.
pub struct SourceLine {
    pub kind: ReferenceKind,
    pub reference: String,
    pub period_id: i32,
    pub account: AccountCode,
    pub amount_micro_dollars: i64,
}

pub trait ReconciliationSources: Send + Sync {
    /// Returns the authoritative source records for the requested posting period.
    fn records(
        &self,
        period_id: i32,
    ) -> impl Future<Output = Result<Vec<SourceLine>, FinanceError>> + Send;
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// An account and period whose source amount differs from its journal amount.
pub struct AccountDifference {
    pub period_id: i32,
    pub account_code: u16,
    pub reference_kind: &'static str,
    pub reference: String,
    pub expected_micro_dollars: i64,
    pub journal_micro_dollars: i64,
    pub unexplained_micro_dollars: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// All source-to-journal differences found in one accounting period.
pub struct ReconciliationReport {
    pub period_id: i32,
    pub differences: Vec<AccountDifference>,
}

impl ReconciliationReport {
    /// Whether every reconciled reference and account has zero unexplained difference.
    pub fn is_clean(&self) -> bool {
        self.differences.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
/// Read-only result of checking journal entries and their stream heads.
pub struct JournalVerification {
    pub valid: bool,
    pub broken_entries: Vec<String>,
    pub broken_stream_heads: Vec<String>,
}

impl JournalVerification {
    /// Serializes a restore-drill-friendly result with a `valid` boolean.
    pub fn machine_readable(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&serde_json::json!({
            "valid": self.valid,
            "broken_entries": self.broken_entries,
            "broken_stream_heads": self.broken_stream_heads,
        }))
    }
}

/// Verifies every stored journal entry and stream head without changing finance state.
pub async fn verify_journal_hash_chain(
    client: &Client,
) -> Result<JournalVerification, FinanceError> {
    let broken_entries = client
        .query(
            "SELECT encode(e.entry_id, 'hex') || ':' || e.stream_seq::text \
             FROM (SELECT entry_id, stream_seq, prev_hash, entry_hash, entry_hash_version, \
                          lag(entry_hash) OVER w AS before_hash, lag(stream_seq) OVER w AS before_seq \
                     FROM finance.journal_entry \
                    WINDOW w AS (PARTITION BY stream_kind, stream_id ORDER BY stream_seq)) e \
             WHERE e.stream_seq <> coalesce(e.before_seq, 0) + 1 \
                OR e.prev_hash <> coalesce(e.before_hash, decode(repeat('00', 32), 'hex')) \
                OR e.entry_hash <> finance.entry_hash( \
                    e.entry_hash_version, e.prev_hash, finance.entry_json(e.entry_id)) \
             ORDER BY e.stream_seq, e.entry_id",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect::<Vec<String>>();
    let broken_stream_heads = client
        .query(
            "SELECT coalesce(s.stream_kind, e.stream_kind) || '/' || \
                    encode(coalesce(s.stream_id, e.stream_id), 'hex') \
             FROM finance.ledger_stream s \
             FULL OUTER JOIN (SELECT stream_kind, stream_id, max(stream_seq) AS seq \
                               FROM finance.journal_entry GROUP BY 1, 2) e \
               ON e.stream_kind = s.stream_kind AND e.stream_id = s.stream_id \
             WHERE s.stream_kind IS NULL OR s.last_seq <> coalesce(e.seq, 0) ORDER BY 1",
            &[],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect::<Vec<String>>();
    Ok(JournalVerification {
        valid: broken_entries.is_empty() && broken_stream_heads.is_empty(),
        broken_entries,
        broken_stream_heads,
    })
}

/// Compares source totals with externally referenced journal lines for one accounting period.
/// A per-period advisory lock gives concurrent runs one stable serialization point.
pub async fn reconcile_period<S: ReconciliationSources>(
    client: &mut Client,
    period_id: i32,
    sources: &S,
) -> Result<ReconciliationReport, FinanceError> {
    if !(200001..=999912).contains(&period_id) {
        return Err(FinanceError::ReconciliationPeriod);
    }
    let source_lines = sources.records(period_id).await?;
    let transaction = client.transaction().await?;
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1::integer)", &[&period_id])
        .await?;
    let mut balances = BTreeMap::<(i32, AccountCode, ReferenceKind, String), (i64, i64)>::new();
    for line in source_lines {
        if line.period_id != period_id {
            return Err(FinanceError::ReconciliationSourcePeriod);
        }
        let totals = balances
            .entry((period_id, line.account, line.kind, line.reference))
            .or_default();
        totals.0 = totals
            .0
            .checked_add(line.amount_micro_dollars)
            .ok_or(FinanceError::ReconciliationOverflow)?;
    }
    let journal = transaction
        .query(
            "SELECT e.period_id, e.external_ref_kind, coalesce(e.external_ref, ''), a.code, \
                    sum(l.amount_micro_dollars)::bigint \
             FROM finance.journal_entry e \
             JOIN finance.journal_line l ON l.entry_id = e.entry_id \
             JOIN finance.account_key k ON k.key_id = l.key_id \
             JOIN finance.ledger_account a ON a.code = k.code \
             WHERE e.period_id = $1 AND e.external_ref_kind IN \
                ('processor_payment', 'processor_refund', 'processor_dispute', 'supplier_invoice', 'attempt') \
             GROUP BY e.period_id, e.external_ref_kind, e.external_ref, a.code \
             ORDER BY e.external_ref_kind, e.external_ref, a.code",
            &[&period_id],
        )
        .await?;
    for row in journal {
        let reference_kind_label: &str = row.get(1);
        let reference_kind = ReferenceKind::parse(reference_kind_label).ok_or_else(|| {
            FinanceError::ReconciliationUnknownReferenceKind(reference_kind_label.to_owned())
        })?;
        let reference: String = row.get(2);
        let code = u16::try_from(row.get::<_, i16>(3))
            .map_err(|_| FinanceError::ReconciliationOverflow)?;
        let Some(account) = AccountCode::from_number(code) else {
            return Err(FinanceError::ReconciliationUnknownAccount(code));
        };
        let totals = balances
            .entry((row.get(0), account, reference_kind, reference))
            .or_default();
        totals.1 = row.get(4);
    }
    let mut differences = Vec::new();
    for ((period_id, account, reference_kind, reference), (expected, journal)) in balances {
        let unexplained = expected
            .checked_sub(journal)
            .ok_or(FinanceError::ReconciliationOverflow)?;
        if unexplained != 0 {
            differences.push(AccountDifference {
                period_id,
                account_code: account.number(),
                reference_kind: reference_kind.label(),
                reference,
                expected_micro_dollars: expected,
                journal_micro_dollars: journal,
                unexplained_micro_dollars: unexplained,
            });
        }
    }
    transaction.commit().await?;
    Ok(ReconciliationReport {
        period_id,
        differences,
    })
}

/// Collects each fake or production source independently; errors prevent a partial report.
/// Processor records include captures, refunds, and disputes; other ports return invoices and
/// dispatches.
pub struct ReconciliationPorts<P, I, D> {
    /// Processor captures, refunds, and disputes.
    pub processor: P,
    /// Supplier invoice details.
    pub invoices: I,
    /// Supplier dispatch or attempt details.
    pub dispatches: D,
}

impl<P: ReconciliationSources, I: ReconciliationSources, D: ReconciliationSources>
    ReconciliationSources for ReconciliationPorts<P, I, D>
{
    async fn records(&self, period_id: i32) -> Result<Vec<SourceLine>, FinanceError> {
        let (processor, invoices, dispatches) = tokio::try_join!(
            self.processor.records(period_id),
            self.invoices.records(period_id),
            self.dispatches.records(period_id),
        )?;
        Ok(processor
            .into_iter()
            .chain(invoices)
            .chain(dispatches)
            .collect())
    }
}

#[derive(Default)]
/// Configurable source used by tests and local reconciliation drills.
pub struct FakeReconciliationSource {
    records: std::sync::Mutex<Vec<SourceLine>>,
}

impl FakeReconciliationSource {
    /// Creates a fake containing the given signed source lines.
    pub fn new(records: Vec<SourceLine>) -> Self {
        Self {
            records: std::sync::Mutex::new(records),
        }
    }
}

impl ReconciliationSources for FakeReconciliationSource {
    async fn records(&self, period_id: i32) -> Result<Vec<SourceLine>, FinanceError> {
        self.records
            .lock()
            .map(|records| {
                records
                    .iter()
                    .filter(|line| line.period_id == period_id)
                    .cloned()
                    .collect()
            })
            .map_err(|_| FinanceError::ReconciliationPort)
    }
}
