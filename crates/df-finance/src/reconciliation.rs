//! Read-only comparisons between journal references and source records from processor, supplier,
//! and dispatch systems. This module never repairs or posts a balancing entry.

use std::collections::BTreeMap;

use df_commerce::ledger::AccountCode;
use serde::Serialize;
use tokio_postgres::Client;

use crate::service::FinanceError;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ReferenceKind {
    ProcessorCapture,
    ProcessorRefund,
    ProcessorDispute,
    SupplierInvoice,
    Dispatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLine {
    pub kind: ReferenceKind,
    pub reference: String,
    pub period_id: i32,
    pub account: AccountCode,
    pub amount_micro_dollars: i64,
}

pub trait ReconciliationSources: Send + Sync {
    fn records(
        &self,
        period_id: i32,
    ) -> impl Future<Output = Result<Vec<SourceLine>, FinanceError>> + Send;
}

use std::future::Future;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AccountDifference {
    pub period_id: i32,
    pub account_code: u16,
    pub expected_micro_dollars: i64,
    pub journal_micro_dollars: i64,
    pub unexplained_micro_dollars: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReconciliationReport {
    pub period_id: i32,
    pub differences: Vec<AccountDifference>,
}

impl ReconciliationReport {
    pub fn is_clean(&self) -> bool {
        self.differences.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JournalVerification {
    pub valid: bool,
    pub broken_entries: Vec<String>,
    pub broken_stream_heads: Vec<String>,
}

impl JournalVerification {
    pub fn machine_readable(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
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
            "SELECT s.stream_kind || '/' || encode(s.stream_id, 'hex') \
             FROM finance.ledger_stream s \
             LEFT JOIN (SELECT stream_kind, stream_id, max(stream_seq) AS seq \
                          FROM finance.journal_entry GROUP BY 1, 2) e \
               ON e.stream_kind = s.stream_kind AND e.stream_id = s.stream_id \
             WHERE s.last_seq <> coalesce(e.seq, 0) ORDER BY 1",
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
    let transaction = client.transaction().await?;
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1::integer)", &[&period_id])
        .await?;
    let source_lines = sources.records(period_id).await?;
    let mut balances = BTreeMap::<(i32, AccountCode), (i64, i64)>::new();
    for line in source_lines {
        if line.period_id != period_id {
            return Err(FinanceError::ReconciliationSourcePeriod);
        }
        let totals = balances.entry((period_id, line.account)).or_default();
        totals.0 = totals
            .0
            .checked_add(line.amount_micro_dollars)
            .ok_or(FinanceError::ReconciliationOverflow)?;
    }
    let journal = transaction
        .query(
            "SELECT e.period_id, a.code, sum(l.amount_micro_dollars)::bigint \
             FROM finance.journal_entry e \
             JOIN finance.journal_line l ON l.entry_id = e.entry_id \
             JOIN finance.account_key k ON k.key_id = l.key_id \
             JOIN finance.ledger_account a ON a.code = k.code \
             WHERE e.period_id = $1 AND e.external_ref_kind IN \
                ('processor_payment', 'processor_refund', 'processor_dispute', 'supplier_invoice', 'attempt') \
             GROUP BY e.period_id, a.code ORDER BY a.code",
            &[&period_id],
        )
        .await?;
    for row in journal {
        let code = u16::try_from(row.get::<_, i16>(1))
            .map_err(|_| FinanceError::ReconciliationOverflow)?;
        let Some(account) = AccountCode::from_number(code) else {
            return Err(FinanceError::ReconciliationUnknownAccount(code));
        };
        let totals = balances.entry((row.get(0), account)).or_default();
        totals.1 = row.get(2);
    }
    let differences = balances
        .into_iter()
        .filter_map(|((period_id, account), (expected, journal))| {
            let unexplained = expected.checked_sub(journal)?;
            (unexplained != 0).then_some(AccountDifference {
                period_id,
                account_code: account.number(),
                expected_micro_dollars: expected,
                journal_micro_dollars: journal,
                unexplained_micro_dollars: unexplained,
            })
        })
        .collect();
    transaction.commit().await?;
    Ok(ReconciliationReport {
        period_id,
        differences,
    })
}

/// Collects each fake or production source independently; errors prevent a partial report.
pub struct ReconciliationPorts<P, I, D> {
    pub processor: P,
    pub invoices: I,
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
pub struct FakeReconciliationSource {
    records: std::sync::Mutex<Vec<SourceLine>>,
}

impl FakeReconciliationSource {
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
