//! Read-only accounting exports and operator liability reports.

use std::collections::BTreeMap;

use tokio_postgres::Client;

use crate::service::FinanceError;

pub mod reserve;

/// One journal side for deterministic cents-based exports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalSide {
    pub entry_id: String,
    pub line_no: i16,
    pub period_id: i32,
    pub date: String,
    pub description: String,
    pub account_code: i16,
    pub account_name: String,
    pub micro_dollars: i64,
}

/// Reads journal lines for one accounting period, ordered for byte-stable regeneration.
pub async fn journal_sides(
    client: &Client,
    period_id: i32,
) -> Result<Vec<JournalSide>, FinanceError> {
    if !(200001..=999912).contains(&period_id) {
        return Err(FinanceError::ReconciliationPeriod);
    }
    let rows = client
        .query(
            "SELECT encode(e.entry_id, 'hex'), l.line_no, e.period_id, \
                    to_char(to_timestamp(e.recorded_at_ms / 1000.0) AT TIME ZONE finance.time_zone_at(e.recorded_at_ms), 'YYYY-MM-DD'), \
                    e.kind, a.code, a.name, l.amount_micro_dollars \
               FROM finance.journal_entry e \
               JOIN finance.journal_line l ON l.entry_id = e.entry_id \
               JOIN finance.account_key k ON k.key_id = l.key_id \
               JOIN finance.ledger_account a ON a.code = k.code \
              WHERE e.period_id = $1 AND a.book = 'general' \
              ORDER BY e.entry_id, l.line_no",
            &[&period_id],
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| JournalSide {
            entry_id: row.get(0),
            line_no: row.get(1),
            period_id: row.get(2),
            date: row.get(3),
            description: row.get(4),
            account_code: row.get(5),
            account_name: row.get(6),
            micro_dollars: row.get(7),
        })
        .collect())
}

/// Returns the generic double-entry journal CSV with stable ids and integer cents.
pub fn generic_journal_csv(sides: &[JournalSide]) -> String {
    let mut csv = String::from(
        "line_id,period_id,date,entry_id,line_no,description,account_code,account,amount_cents\n",
    );
    for side in export_sides(sides) {
        let cents = rounded_cents(side.micro_dollars);
        let fields = [
            format!("{}-{}", side.entry_id, side.line_no),
            side.period_id.to_string(),
            csv_field(&side.date),
            side.entry_id.clone(),
            side.line_no.to_string(),
            csv_field(&side.description),
            side.account_code.to_string(),
            csv_field(&side.account_name),
            cents.to_string(),
        ];
        csv.push_str(&fields.join(","));
        csv.push('\n');
    }
    csv
}

/// Returns the Wave transaction import CSV: one account/amount row per entry side.
pub fn wave_transaction_csv(sides: &[JournalSide], account_map: &[(i16, String)]) -> String {
    let mut csv = String::from("date,description,account,amount,line_id\n");
    for side in export_sides(sides) {
        let account = account_map
            .iter()
            .find(|(code, _)| *code == side.account_code)
            .map(|(_, name)| name.as_str())
            .unwrap_or(side.account_name.as_str());
        csv.push_str(&format!(
            "{},{},{},{},{}-{}\n",
            csv_field(&side.date),
            csv_field(&side.description),
            csv_field(account),
            rounded_cents(side.micro_dollars),
            side.entry_id,
            side.line_no
        ));
    }
    csv
}

fn export_sides(sides: &[JournalSide]) -> Vec<JournalSide> {
    let mut export = sides.to_vec();
    let mut entry_totals = BTreeMap::<String, (i64, &JournalSide)>::new();
    for side in sides {
        let entry = entry_totals
            .entry(side.entry_id.clone())
            .or_insert((0, side));
        entry.0 += rounded_cents(side.micro_dollars);
    }
    for (entry_id, (cents, source)) in entry_totals {
        if cents != 0 {
            export.push(JournalSide {
                entry_id,
                line_no: 0,
                period_id: source.period_id,
                date: source.date.clone(),
                description: format!("{} cent rounding", source.description),
                account_code: 5700,
                account_name: "rounding".to_owned(),
                micro_dollars: -cents * 10_000,
            });
        }
    }
    export
}

fn rounded_cents(micro_dollars: i64) -> i64 {
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

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{JournalSide, generic_journal_csv, wave_transaction_csv};

    fn side(entry: &str, line: i16, amount: i64) -> JournalSide {
        JournalSide {
            entry_id: entry.to_owned(),
            line_no: line,
            period_id: 202610,
            date: "2026-10-25".to_owned(),
            description: "purchase, confirmed".to_owned(),
            account_code: 1100,
            account_name: "processor_clearing".to_owned(),
            micro_dollars: amount,
        }
    }

    #[test]
    fn exports_have_stable_line_ids_and_integer_cents() {
        let sides = [side("ab12", 1, 1_234_567), side("ab12", 2, -1_234_567)];
        let first = generic_journal_csv(&sides);
        assert_eq!(first, generic_journal_csv(&sides));
        assert!(first.contains(
            "ab12-1,202610,2026-10-25,ab12,1,\"purchase, confirmed\",1100,processor_clearing,123"
        ));
        assert!(first.contains(
            "ab12-2,202610,2026-10-25,ab12,2,\"purchase, confirmed\",1100,processor_clearing,-123"
        ));
        let wave = wave_transaction_csv(&sides, &[(1100, "Stripe clearing".to_owned())]);
        assert!(wave.contains("2026-10-25,\"purchase, confirmed\",Stripe clearing,123,ab12-1"));
    }

    #[test]
    fn cents_rounding_adds_a_stable_balancing_5700_side() {
        let sides = [
            side("ab34", 1, 6_000),
            side("ab34", 2, 6_000),
            side("ab34", 3, -12_000),
        ];
        let csv = generic_journal_csv(&sides);
        assert!(csv.contains(
            "ab34-0,202610,2026-10-25,ab34,0,\"purchase, confirmed cent rounding\",5700,rounding,-1"
        ));
        assert_eq!(csv, generic_journal_csv(&sides));
    }
}
