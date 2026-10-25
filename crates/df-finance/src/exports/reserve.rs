//! Operator liability and reserve snapshot, in integer micro-dollars.

use tokio_postgres::Client;

use crate::service::FinanceError;

#[derive(Clone, Debug, Eq, PartialEq)]
/// Liability totals as of an explicitly supplied UTC millisecond instant.
pub struct ReserveReport {
    pub as_of_ms: i64,
    pub purchased_unspent_micro_dollars: i64,
    pub refund_payable_micro_dollars: i64,
    pub promotional_credit_micro_dollars: i64,
    pub unsettled_purchased_holds_micro_dollars: i64,
    pub unsettled_promotional_holds_micro_dollars: i64,
    pub unknown_supplier_exposure_micro_dollars: i64,
    pub customer_credit_refund_reserve_micro_dollars: i64,
}

/// Computes balances from journal lines posted through `as_of_ms`.
/// Holds are included in purchased/promotional totals and shown separately, never added twice.
pub async fn report_as_of(client: &Client, as_of_ms: i64) -> Result<ReserveReport, FinanceError> {
    let row = client
        .query_one(
            "SELECT \
                coalesce(-sum(l.amount_micro_dollars) FILTER (WHERE a.code BETWEEN 2110 AND 2140), 0)::bigint, \
                coalesce(-sum(l.amount_micro_dollars) FILTER (WHERE a.code = 2500), 0)::bigint, \
                coalesce(-sum(l.amount_micro_dollars) FILTER (WHERE a.code BETWEEN 2210 AND 2230), 0)::bigint, \
                coalesce(-sum(l.amount_micro_dollars) FILTER (WHERE a.code = 2130), 0)::bigint, \
                coalesce(-sum(l.amount_micro_dollars) FILTER (WHERE a.code = 2230), 0)::bigint, \
                coalesce(sum(l.amount_micro_dollars) FILTER (WHERE a.code = 9100 AND EXISTS ( \
                    SELECT 1 FROM finance.provider_attempt p \
                     WHERE p.attempt_id = e.attempt_id AND p.supplier_outcome = 'unknown' \
                )), 0)::bigint \
             FROM finance.journal_entry e \
             JOIN finance.journal_line l ON l.entry_id = e.entry_id \
             JOIN finance.account_key k ON k.key_id = l.key_id \
             JOIN finance.ledger_account a ON a.code = k.code \
             WHERE e.recorded_at_ms <= $1",
            &[&as_of_ms],
        )
        .await?;
    report_from_balances(
        as_of_ms,
        row.get(0),
        row.get(1),
        row.get(2),
        row.get(3),
        row.get(4),
        row.get(5),
    )
}

fn report_from_balances(
    as_of_ms: i64,
    purchased: i64,
    refund: i64,
    promotional: i64,
    purchased_holds: i64,
    promotional_holds: i64,
    unknown_supplier_exposure: i64,
) -> Result<ReserveReport, FinanceError> {
    let reserve = purchased
        .checked_add(refund)
        .ok_or(FinanceError::ReconciliationOverflow)?;
    Ok(ReserveReport {
        as_of_ms,
        purchased_unspent_micro_dollars: purchased,
        refund_payable_micro_dollars: refund,
        promotional_credit_micro_dollars: promotional,
        unsettled_purchased_holds_micro_dollars: purchased_holds,
        unsettled_promotional_holds_micro_dollars: promotional_holds,
        unknown_supplier_exposure_micro_dollars: unknown_supplier_exposure,
        customer_credit_refund_reserve_micro_dollars: reserve,
    })
}

#[cfg(test)]
mod tests {
    use super::report_from_balances;

    #[test]
    fn purchased_credit_and_refund_payable_define_reserve_without_double_counting_holds() {
        let report =
            report_from_balances(1, 900, 100, 300, 200, 50, 400).expect("small reserve totals fit");
        assert_eq!(
            report.customer_credit_refund_reserve_micro_dollars,
            report.purchased_unspent_micro_dollars + report.refund_payable_micro_dollars
        );
        assert_ne!(report.customer_credit_refund_reserve_micro_dollars, 1_650);
    }

    #[test]
    fn reserve_overflow_is_an_error() {
        assert!(report_from_balances(1, i64::MAX, 1, 0, 0, 0, 0).is_err());
    }
}
