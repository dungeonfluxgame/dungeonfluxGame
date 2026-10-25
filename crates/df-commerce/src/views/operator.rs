//! Operator finance views (18.9): the side of the money no customer sees. Served only to an
//! operator holding `FinanceInspect`; this module shares no type with `views::customer`.

use serde::Serialize;

/// An amount in micro-dollars; signed because a margin can be negative.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct SignedMoney {
    pub micro_dollars: i64,
}

/// The three states of an attempt, as stored.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AttemptStatesView {
    pub dispatch: String,
    pub customer: String,
    pub supplier: String,
}

/// One payer's share of a charge.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PayerShareView {
    pub payer: String,
    pub allocation: String,
    pub lot: String,
    pub class: String,
    pub held: SignedMoney,
    pub charged: SignedMoney,
    pub released: SignedMoney,
}

/// A company loss on an attempt and its class.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompanyLossView {
    pub loss_class: String,
    pub amount: SignedMoney,
}

/// Where one attempt's money went: retail, supplier, margin, shares and states.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OperatorBreakdown {
    pub attempt: String,
    pub session: String,
    pub provider: String,
    pub model: String,
    pub price_version_label: String,
    pub markup_numerator: u64,
    pub markup_denominator: u64,
    pub retail_maximum: SignedMoney,
    pub retail_charge: SignedMoney,
    pub supplier_maximum: SignedMoney,
    pub supplier_actual: Option<SignedMoney>,
    pub margin: Option<SignedMoney>,
    pub company_loss: Option<CompanyLossView>,
    pub shares: Vec<PayerShareView>,
    pub states: AttemptStatesView,
    pub provider_request_ref: Option<String>,
}

impl OperatorBreakdown {
    /// The margin of a settled attempt: what the table paid less what the supplier charged.
    pub fn margin_of(retail_charge: u64, supplier_actual: u64) -> SignedMoney {
        SignedMoney {
            micro_dollars: i64::try_from(retail_charge)
                .unwrap_or(i64::MAX)
                .saturating_sub(i64::try_from(supplier_actual).unwrap_or(i64::MAX)),
        }
    }
}

/// The five balances of plan 6, in the general book and the promotional memo book.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FiveBalancesView {
    pub currency: String,
    pub purchased_available: SignedMoney,
    pub purchased_allocated: SignedMoney,
    pub purchased_reserved: SignedMoney,
    pub purchased_frozen: SignedMoney,
    pub promotional_outstanding: SignedMoney,
}

/// One difference a reconciliation run found.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReconciliationDifferenceView {
    pub run: String,
    pub kind: String,
    pub reference: String,
    pub expected: Option<SignedMoney>,
    pub observed: Option<SignedMoney>,
    pub state: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_margin_is_the_charge_less_the_supplier_actual_and_can_be_negative() {
        assert_eq!(OperatorBreakdown::margin_of(525, 350).micro_dollars, 175);
        assert_eq!(OperatorBreakdown::margin_of(0, 40).micro_dollars, -40);
    }
}
