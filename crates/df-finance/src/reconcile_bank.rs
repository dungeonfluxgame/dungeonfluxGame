//! Read-only imports for bank statements and processor payout reports.

use std::collections::{BTreeMap, BTreeSet};

use tokio_postgres::Client;

use crate::service::FinanceError;

#[derive(Clone, Debug, Eq, PartialEq)]
/// One imported bank-statement row, represented in integer cents.
pub struct BankLine {
    pub date: String,
    pub description: String,
    pub reference: String,
    pub amount_cents: i64,
    pub currency: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One payout reported by the processor.
pub struct ProcessorPayout {
    pub payout_id: String,
    pub date: String,
    pub amount_cents: i64,
    pub currency: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One payout posting in the finance journal.
pub struct LedgerPayout {
    pub payout_id: String,
    pub amount_cents: i64,
    pub currency: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Matched and unmatched inputs for operator review (OPS-04).
pub struct BankReconciliation {
    pub mercury_format_verify_required: bool,
    pub matched_payout_ids: Vec<String>,
    pub unmatched_bank_lines: Vec<BankLine>,
    pub unmatched_processor_payouts: Vec<ProcessorPayout>,
    pub unmatched_ledger_payouts: Vec<LedgerPayout>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// CSV structure or value failed validation.
pub enum ImportError {
    MissingHeader(String),
    InvalidCsv,
    InvalidAmount,
    InvalidCurrency,
    InvalidDate,
}

/// Parses normalized bank columns. The Mercury header set is unverified; callers must display
/// `mercury_format_verify_required` until compared with an owner-provided sample.
pub fn import_bank_statement(csv: &str) -> Result<Vec<BankLine>, ImportError> {
    let records = parse_csv(csv)?;
    let Some((header, rows)) = records.split_first() else {
        return Err(ImportError::InvalidCsv);
    };
    let date = column(header, "date")?;
    let description = column(header, "description")?;
    let reference = column(header, "reference")?;
    let amount = column(header, "amount_cents")?;
    let currency = column(header, "currency")?;
    rows.iter()
        .map(|row| {
            Ok(BankLine {
                date: valid_date(field(row, date)?)?,
                description: field(row, description)?.to_owned(),
                reference: field(row, reference)?.to_owned(),
                amount_cents: parse_amount(field(row, amount)?)?,
                currency: valid_currency(field(row, currency)?)?,
            })
        })
        .collect()
}

/// Parses the invented Stripe payout fixture contract in integer cents.
pub fn import_processor_payouts(csv: &str) -> Result<Vec<ProcessorPayout>, ImportError> {
    let records = parse_csv(csv)?;
    let Some((header, rows)) = records.split_first() else {
        return Err(ImportError::InvalidCsv);
    };
    let payout_id = column(header, "payout_id")?;
    let date = column(header, "date")?;
    let amount = column(header, "amount_cents")?;
    let currency = column(header, "currency")?;
    rows.iter()
        .map(|row| {
            Ok(ProcessorPayout {
                payout_id: field(row, payout_id)?.to_owned(),
                date: valid_date(field(row, date)?)?,
                amount_cents: parse_amount(field(row, amount)?)?,
                currency: valid_currency(field(row, currency)?)?,
            })
        })
        .collect()
}

/// Reconciles imported bank and processor rows to immutable payout journal postings.
pub async fn reconcile_imports(
    client: &Client,
    bank_csv: &str,
    processor_csv: &str,
) -> Result<BankReconciliation, FinanceError> {
    let bank_lines = import_bank_statement(bank_csv).map_err(FinanceError::BankImport)?;
    let processor_payouts =
        import_processor_payouts(processor_csv).map_err(FinanceError::BankImport)?;
    let rows = client
        .query(
            "SELECT e.external_ref, k.currency, sum(l.amount_micro_dollars)::bigint \
               FROM finance.journal_entry e \
               JOIN finance.journal_line l ON l.entry_id = e.entry_id \
               JOIN finance.account_key k ON k.key_id = l.key_id \
               JOIN finance.ledger_account a ON a.code = k.code \
              WHERE e.external_ref_kind = 'processor_payout' AND a.code = 1000 \
              GROUP BY e.external_ref, k.currency ORDER BY e.external_ref, k.currency",
            &[],
        )
        .await?;
    let ledger = rows
        .into_iter()
        .map(|row| {
            let payout_id = row
                .get::<_, Option<String>>(0)
                .ok_or(FinanceError::BankPayoutReferenceMissing)?;
            Ok(LedgerPayout {
                payout_id,
                currency: row.get::<_, String>(1).trim().to_owned(),
                amount_cents: cents_from_micro(row.get(2)),
            })
        })
        .collect::<Result<Vec<_>, FinanceError>>()?;
    Ok(reconcile_rows(bank_lines, processor_payouts, ledger))
}

fn reconcile_rows(
    bank: Vec<BankLine>,
    processor: Vec<ProcessorPayout>,
    ledger: Vec<LedgerPayout>,
) -> BankReconciliation {
    let processor_by_id = processor
        .iter()
        .enumerate()
        .map(|(index, payout)| (payout.payout_id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let ledger_by_id = ledger
        .iter()
        .enumerate()
        .map(|(index, payout)| (payout.payout_id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut matched_processor = BTreeSet::new();
    let mut matched_ledger = BTreeSet::new();
    let mut matched_bank = BTreeSet::new();
    let mut matched_payout_ids = BTreeSet::new();
    for (bank_index, bank_line) in bank.iter().enumerate() {
        let Some(processor_index) = processor_by_id.get(bank_line.reference.as_str()) else {
            continue;
        };
        let Some(ledger_index) = ledger_by_id.get(bank_line.reference.as_str()) else {
            continue;
        };
        let processor_payout = &processor[*processor_index];
        let ledger_payout = &ledger[*ledger_index];
        if bank_line.amount_cents == processor_payout.amount_cents
            && bank_line.amount_cents == ledger_payout.amount_cents
            && bank_line.currency == processor_payout.currency
            && bank_line.currency == ledger_payout.currency
        {
            matched_bank.insert(bank_index);
            matched_processor.insert(*processor_index);
            matched_ledger.insert(*ledger_index);
            matched_payout_ids.insert(bank_line.reference.clone());
        }
    }
    BankReconciliation {
        mercury_format_verify_required: true,
        matched_payout_ids: matched_payout_ids.into_iter().collect(),
        unmatched_bank_lines: bank
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !matched_bank.contains(index))
            .map(|(_, line)| line)
            .collect(),
        unmatched_processor_payouts: processor
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !matched_processor.contains(index))
            .map(|(_, payout)| payout)
            .collect(),
        unmatched_ledger_payouts: ledger
            .into_iter()
            .enumerate()
            .filter(|(index, _)| !matched_ledger.contains(index))
            .map(|(_, payout)| payout)
            .collect(),
    }
}

fn cents_from_micro(micro_dollars: i64) -> i64 {
    let whole_cents = micro_dollars / 10_000;
    let remainder = micro_dollars % 10_000;
    if remainder.abs() < 5_000 {
        whole_cents
    } else if micro_dollars < 0 {
        whole_cents - 1
    } else {
        whole_cents + 1
    }
}

fn column(headers: &[String], name: &str) -> Result<usize, ImportError> {
    headers
        .iter()
        .position(|header| header.trim().eq_ignore_ascii_case(name))
        .ok_or_else(|| ImportError::MissingHeader(name.to_owned()))
}

fn field(row: &[String], index: usize) -> Result<&str, ImportError> {
    row.get(index)
        .map(String::as_str)
        .ok_or(ImportError::InvalidCsv)
}

fn parse_amount(value: &str) -> Result<i64, ImportError> {
    value.trim().parse().map_err(|_| ImportError::InvalidAmount)
}

fn valid_currency(value: &str) -> Result<String, ImportError> {
    let currency = value.trim().to_ascii_uppercase();
    if currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
        Ok(currency)
    } else {
        Err(ImportError::InvalidCurrency)
    }
}

fn valid_date(value: &str) -> Result<String, ImportError> {
    let date = value.trim();
    let bytes = date.as_bytes();
    if bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        Ok(date.to_owned())
    } else {
        Err(ImportError::InvalidDate)
    }
}

fn parse_csv(input: &str) -> Result<Vec<Vec<String>>, ImportError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = input.chars().peekable();
    let mut quoted = false;
    while let Some(character) = chars.next() {
        match character {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted => {}
            _ => field.push(character),
        }
    }
    if quoted {
        return Err(ImportError::InvalidCsv);
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    if !rows.is_empty() && rows.iter().any(|row| row.len() != rows[0].len()) {
        return Err(ImportError::InvalidCsv);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::{
        BankLine, LedgerPayout, ProcessorPayout, import_bank_statement, import_processor_payouts,
        reconcile_rows,
    };

    #[test]
    fn invented_payout_fixture_matches_ledger_and_bank_and_lists_unmatched_rows() {
        let bank = import_bank_statement(
            "date,description,reference,amount_cents,currency\n2026-10-25,Stripe payout,po_001,1912,USD\n2026-10-25,Other deposit,bank_002,500,USD\n",
        )
        .expect("invented bank fixture parses");
        let processor = import_processor_payouts(
            "payout_id,date,amount_cents,currency\npo_001,2026-10-25,1912,USD\npo_003,2026-10-25,200,USD\n",
        )
        .expect("invented processor fixture parses");
        let report = reconcile_rows(
            bank,
            processor,
            vec![LedgerPayout {
                payout_id: "po_001".to_owned(),
                amount_cents: 1912,
                currency: "USD".to_owned(),
            }],
        );
        assert_eq!(report.matched_payout_ids, ["po_001"]);
        assert_eq!(report.unmatched_bank_lines.len(), 1);
        assert_eq!(report.unmatched_processor_payouts.len(), 1);
        assert!(report.unmatched_ledger_payouts.is_empty());
        assert!(report.mercury_format_verify_required);
    }

    #[test]
    fn mismatched_payout_amount_is_not_marked_as_reconciled() {
        let report = reconcile_rows(
            vec![BankLine {
                date: "2026-10-25".to_owned(),
                description: "payout".to_owned(),
                reference: "po_002".to_owned(),
                amount_cents: 99,
                currency: "USD".to_owned(),
            }],
            vec![ProcessorPayout {
                payout_id: "po_002".to_owned(),
                date: "2026-10-25".to_owned(),
                amount_cents: 100,
                currency: "USD".to_owned(),
            }],
            vec![LedgerPayout {
                payout_id: "po_002".to_owned(),
                amount_cents: 100,
                currency: "USD".to_owned(),
            }],
        );
        assert!(report.matched_payout_ids.is_empty());
        assert_eq!(report.unmatched_bank_lines.len(), 1);
        assert_eq!(report.unmatched_processor_payouts.len(), 1);
        assert_eq!(report.unmatched_ledger_payouts.len(), 1);
    }
}
