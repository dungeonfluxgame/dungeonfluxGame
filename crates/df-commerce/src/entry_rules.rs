//! The entry-kind allowlist (planning/finance-ledger-research.md 17.6): for each kind of entry,
//! which accounts may take a debit and which a credit. The SQL seed of
//! `finance.entry_kind_rule` is compared with this table by a test, so the pure postings and the
//! database boundary cannot drift apart.

use crate::ledger::{AccountCode, EntryKind, EntryRefusal, JournalEntry, StreamKind};

use AccountCode::{
    AbsorbedSupplierCost as Absorbed, BankOperating as Bank, ChargebackLoss,
    CostOfServices as Cost, CostOfServicesPromotional as CostPromo,
    DisputeFundsWithheld as Withheld, OwnerEquity as Equity, ProcessingFees as Fees,
    ProcessorClearing as Clearing, PromotionalAllocated as PAllocated,
    PromotionalAvailable as PAvailable, PromotionalCreditExpense as PExpense,
    PromotionalOffset as POffset, PromotionalReserved as PReserved, PromotionalRevenue as PRevenue,
    PurchasedAllocated as Allocated, PurchasedAvailable as Available, PurchasedFrozen as Frozen,
    PurchasedReserved as Reserved, RefundPayable, RefundsOfConsumedService as Refunds,
    RevenueAiUsage as RevenueAi, RevenueTableTime as RevenueTime, Rounding as Round,
    SupplierAccrued as Accrued, SupplierExposureOffset as ExpoOffset,
    SupplierExposureOpen as ExpoOpen, SupplierPayable as Payable, SupplierPrepaid as Prepaid,
    TaxPayable as Tax,
};

/// One kind's row of the allowlist.
#[derive(Clone, Copy, Debug)]
pub struct KindRule {
    pub kind: EntryKind,
    /// The stream the kind posts to; `None` means the stream of the entry it reverses.
    pub stream: Option<StreamKind>,
    pub debits: &'static [AccountCode],
    pub credits: &'static [AccountCode],
    pub reversible: bool,
}

const fn rule(
    kind: EntryKind,
    stream: Option<StreamKind>,
    debits: &'static [AccountCode],
    credits: &'static [AccountCode],
    reversible: bool,
) -> KindRule {
    KindRule {
        kind,
        stream,
        debits,
        credits,
        reversible,
    }
}

const MANUAL: &[AccountCode] = &[
    Payable,
    Accrued,
    Tax,
    Equity,
    RevenueTime,
    RevenueAi,
    Refunds,
    Cost,
    CostPromo,
    Absorbed,
    Fees,
    ChargebackLoss,
    Round,
];

/// The allowlist of 17.6, in the order of the SQL seed.
pub static RULES: [KindRule; 33] = [
    rule(
        EntryKind::TrialGrant,
        Some(StreamKind::Payer),
        &[POffset],
        &[PAvailable],
        false,
    ),
    rule(
        EntryKind::GoodwillGrant,
        Some(StreamKind::Payer),
        &[POffset],
        &[PAvailable],
        false,
    ),
    rule(
        EntryKind::Purchase,
        Some(StreamKind::Payer),
        &[Clearing],
        &[Available, Tax],
        false,
    ),
    rule(
        EntryKind::ProcessingFee,
        Some(StreamKind::Platform),
        &[Fees],
        &[Clearing],
        true,
    ),
    rule(
        EntryKind::Payout,
        Some(StreamKind::Platform),
        &[Bank],
        &[Clearing],
        true,
    ),
    rule(
        EntryKind::Allocation,
        Some(StreamKind::Payer),
        &[Available, PAvailable],
        &[Allocated, PAllocated],
        false,
    ),
    rule(
        EntryKind::AllocationRelease,
        Some(StreamKind::Payer),
        &[Allocated, PAllocated],
        &[Available, PAvailable],
        false,
    ),
    rule(
        EntryKind::Reservation,
        Some(StreamKind::Session),
        &[Allocated, PAllocated, ExpoOpen],
        &[Reserved, PReserved, ExpoOffset],
        false,
    ),
    rule(
        EntryKind::MeterHold,
        Some(StreamKind::Session),
        &[Allocated, PAllocated],
        &[Reserved, PReserved],
        false,
    ),
    rule(
        EntryKind::Release,
        Some(StreamKind::Session),
        &[Reserved, PReserved, ExpoOffset],
        &[
            Allocated,
            Available,
            Frozen,
            ChargebackLoss,
            PAllocated,
            PAvailable,
            ExpoOpen,
        ],
        false,
    ),
    rule(
        EntryKind::Settlement,
        Some(StreamKind::Session),
        &[Reserved, PReserved, PExpense, Cost, CostPromo, ExpoOffset],
        &[
            RevenueAi,
            RevenueTime,
            Tax,
            Allocated,
            Available,
            Frozen,
            ChargebackLoss,
            PAllocated,
            PAvailable,
            POffset,
            PRevenue,
            Payable,
            Prepaid,
            ExpoOpen,
        ],
        false,
    ),
    rule(
        EntryKind::MeterCharge,
        Some(StreamKind::Session),
        &[Reserved, PReserved, PExpense, Cost, CostPromo],
        &[
            RevenueAi,
            RevenueTime,
            Tax,
            Allocated,
            Available,
            Frozen,
            ChargebackLoss,
            PAllocated,
            PAvailable,
            POffset,
            PRevenue,
            Payable,
            Prepaid,
        ],
        false,
    ),
    rule(
        EntryKind::CompanyLoss,
        Some(StreamKind::Session),
        &[Reserved, PReserved, Absorbed, ExpoOffset],
        &[
            Allocated,
            Available,
            Frozen,
            ChargebackLoss,
            PAllocated,
            PAvailable,
            Payable,
            Prepaid,
            ExpoOpen,
        ],
        false,
    ),
    rule(
        EntryKind::CustomerCredit,
        Some(StreamKind::Session),
        &[RevenueAi, RevenueTime, Absorbed, POffset, PRevenue],
        &[Available, Frozen, Cost, CostPromo, PAvailable, PExpense],
        false,
    ),
    rule(
        EntryKind::RefundRequested,
        Some(StreamKind::Payer),
        &[Available],
        &[RefundPayable],
        false,
    ),
    rule(
        EntryKind::RefundPaid,
        Some(StreamKind::Payer),
        &[RefundPayable, Tax],
        &[Clearing],
        false,
    ),
    rule(
        EntryKind::ManualRefundPayout,
        Some(StreamKind::Payer),
        &[RefundPayable, Tax],
        &[Bank],
        false,
    ),
    rule(
        EntryKind::RefundReturnedToCredit,
        Some(StreamKind::Payer),
        &[RefundPayable],
        &[Available, Frozen],
        false,
    ),
    rule(
        EntryKind::ExternalRefund,
        Some(StreamKind::Payer),
        &[Available, Allocated, Reserved, Frozen, Refunds],
        &[Clearing],
        false,
    ),
    rule(
        EntryKind::DisputeFreeze,
        Some(StreamKind::Payer),
        &[Available, Allocated],
        &[Frozen],
        false,
    ),
    rule(
        EntryKind::DisputeWithheld,
        Some(StreamKind::Platform),
        &[Withheld, Fees],
        &[Clearing],
        false,
    ),
    rule(
        EntryKind::DisputeWon,
        Some(StreamKind::Payer),
        &[Clearing, Frozen],
        &[Withheld, Available, Fees],
        false,
    ),
    rule(
        EntryKind::DisputeLost,
        Some(StreamKind::Payer),
        &[Frozen, RefundPayable, ChargebackLoss],
        &[Withheld, Available],
        false,
    ),
    rule(
        EntryKind::ChargebackRecovery,
        Some(StreamKind::Session),
        &[Reserved],
        &[ChargebackLoss],
        false,
    ),
    rule(
        EntryKind::PromotionalExpiry,
        Some(StreamKind::Payer),
        &[PAvailable],
        &[POffset],
        false,
    ),
    rule(
        EntryKind::PromotionalForfeit,
        Some(StreamKind::Payer),
        &[PAvailable],
        &[POffset],
        false,
    ),
    rule(
        EntryKind::Rounding,
        Some(StreamKind::Payer),
        &[Available],
        &[Round],
        true,
    ),
    rule(
        EntryKind::SupplierExposureWriteOff,
        Some(StreamKind::Platform),
        &[Absorbed, ExpoOffset],
        &[Accrued, ExpoOpen],
        true,
    ),
    rule(
        EntryKind::SupplierInvoiceMatch,
        Some(StreamKind::Platform),
        &[Accrued, Payable, Round, ExpoOffset],
        &[Absorbed, Round, ExpoOpen],
        true,
    ),
    rule(
        EntryKind::SupplierPayment,
        Some(StreamKind::Platform),
        &[Payable],
        &[Bank],
        true,
    ),
    rule(
        EntryKind::SupplierPrepayment,
        Some(StreamKind::Platform),
        &[Prepaid],
        &[Bank],
        true,
    ),
    rule(
        EntryKind::ManualAdjustment,
        Some(StreamKind::Platform),
        MANUAL,
        MANUAL,
        true,
    ),
    rule(EntryKind::Reversal, None, &[], &[], false),
];

/// The rule of `kind`.
pub fn rule_of(kind: EntryKind) -> &'static KindRule {
    RULES
        .iter()
        .find(|row| row.kind == kind)
        .unwrap_or(&RULES[RULES.len() - 1])
}

impl KindRule {
    pub fn allows(&self, code: AccountCode, debit: bool) -> bool {
        if self.kind == EntryKind::Reversal {
            return true;
        }
        if debit {
            self.debits.contains(&code)
        } else {
            self.credits.contains(&code)
        }
    }
}

/// Every line must be one the kind's allowlist names.
pub fn check_allowlist(entry: &JournalEntry) -> Result<(), EntryRefusal> {
    let rule = rule_of(entry.kind);
    for line in &entry.lines {
        let debit = line.amount_micro_dollars > 0;
        if !rule.allows(line.key.code, debit) {
            return Err(EntryRefusal::NotAllowed {
                code: line.key.code,
                debit,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_exactly_one_rule() {
        for row in &RULES {
            assert_eq!(
                RULES.iter().filter(|other| other.kind == row.kind).count(),
                1
            );
        }
    }

    #[test]
    fn a_manual_adjustment_cannot_touch_a_customer_credit_account() {
        let rule = rule_of(EntryKind::ManualAdjustment);
        for code in [
            Available,
            Allocated,
            Reserved,
            Frozen,
            RefundPayable,
            PAvailable,
            PAllocated,
            PReserved,
            Clearing,
            Bank,
            ExpoOpen,
            ExpoOffset,
            PRevenue,
            PExpense,
        ] {
            assert!(
                !rule.allows(code, true) && !rule.allows(code, false),
                "{code:?}"
            );
        }
    }

    #[test]
    fn no_kind_lets_a_promotional_account_meet_an_exposure_account() {
        // A line pair Dr promotional_offset / Cr supplier_exposure_open balances in no book.
        for row in &RULES {
            let promo = row
                .debits
                .iter()
                .chain(row.credits)
                .any(|code| code.book() == crate::ledger::Book::Promo);
            let exposure = row
                .debits
                .iter()
                .chain(row.credits)
                .any(|code| code.book() == crate::ledger::Book::Exposure);
            if promo && exposure {
                // Only the reservation, release, settlement and company-loss kinds touch both, and each book balances alone.
                assert!(
                    matches!(
                        row.kind,
                        EntryKind::Reservation
                            | EntryKind::Release
                            | EntryKind::Settlement
                            | EntryKind::CompanyLoss
                    ),
                    "{:?}",
                    row.kind
                );
            }
        }
    }
}
