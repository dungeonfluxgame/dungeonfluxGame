//! Where credit goes when it comes back to a lot (planning/finance-ledger-research.md 14.4).
//!
//! One pure function answers for every release, surplus and credit, so no posting constructor
//! chooses a key itself and a Frozen or ChargedBack lot never receives spendable credit.

use crate::funding::{LotClass, LotState};

/// An allocation status (8.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationStatus {
    Active,
    PausedDeparted,
    ConsentIncompatible,
    Withdrawn,
    Closing,
    Closed,
}

impl AllocationStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::PausedDeparted => "paused_departed",
            Self::ConsentIncompatible => "consent_incompatible",
            Self::Withdrawn => "withdrawn",
            Self::Closing => "closing",
            Self::Closed => "closed",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "active" => Some(Self::Active),
            "paused_departed" => Some(Self::PausedDeparted),
            "consent_incompatible" => Some(Self::ConsentIncompatible),
            "withdrawn" => Some(Self::Withdrawn),
            "closing" => Some(Self::Closing),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }

    /// Credit may stay in this allocation's allocated key.
    pub fn keeps_allocated_credit(self) -> bool {
        matches!(self, Self::Active | Self::ConsentIncompatible)
    }
}

/// The key a returned amount lands in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReturnTarget {
    /// The allocation's allocated key.
    Allocated,
    /// The payer's available key.
    Available,
    /// The lot's frozen key (a dispute is open).
    Frozen,
    /// A chargeback recovery: Dr reserved / Cr chargeback loss; never spendable.
    ChargebackRecovery,
    /// No such return can occur (a closed or forfeited lot has no holds).
    Refuse,
}

/// The target of a hold or surplus released from a share of `class` on a lot in `lot_state`
/// whose allocation has `status`.
pub fn return_target(
    class: LotClass,
    lot_state: LotState,
    status: AllocationStatus,
) -> ReturnTarget {
    match (class, lot_state) {
        (_, LotState::Active) if status.keeps_allocated_credit() => ReturnTarget::Allocated,
        (_, LotState::Active) => ReturnTarget::Available,
        (LotClass::Purchased, LotState::Frozen) => ReturnTarget::Frozen,
        (LotClass::Purchased, LotState::ChargedBack) => ReturnTarget::ChargebackRecovery,
        _ => ReturnTarget::Refuse,
    }
}

/// The target of a customer credit (a reversal of a charge) to a lot: spendable only on an
/// `Active` lot, frozen on a `Frozen` one, refused otherwise (round 1; 14.4).
pub fn credit_target(class: LotClass, lot_state: LotState) -> ReturnTarget {
    match (class, lot_state) {
        (_, LotState::Active) => ReturnTarget::Available,
        (LotClass::Purchased, LotState::Frozen) => ReturnTarget::Frozen,
        _ => ReturnTarget::Refuse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [LotState; 5] = [
        LotState::Active,
        LotState::Frozen,
        LotState::ChargedBack,
        LotState::Closed,
        LotState::Forfeited,
    ];
    const STATUSES: [AllocationStatus; 6] = [
        AllocationStatus::Active,
        AllocationStatus::PausedDeparted,
        AllocationStatus::ConsentIncompatible,
        AllocationStatus::Withdrawn,
        AllocationStatus::Closing,
        AllocationStatus::Closed,
    ];

    #[test]
    fn return_target_answers_every_lot_state_and_allocation_status_pair() {
        for class in [LotClass::Purchased, LotClass::Promotional] {
            for state in STATES {
                for status in STATUSES {
                    let target = return_target(class, state, status);
                    match (class, state) {
                        (_, LotState::Active) => {
                            let expected = if status.keeps_allocated_credit() {
                                ReturnTarget::Allocated
                            } else {
                                ReturnTarget::Available
                            };
                            assert_eq!(target, expected, "{class:?} {status:?}");
                        }
                        (LotClass::Purchased, LotState::Frozen) => {
                            assert_eq!(target, ReturnTarget::Frozen);
                        }
                        (LotClass::Purchased, LotState::ChargedBack) => {
                            assert_eq!(target, ReturnTarget::ChargebackRecovery);
                        }
                        _ => assert_eq!(target, ReturnTarget::Refuse),
                    }
                }
            }
        }
    }

    #[test]
    fn a_frozen_or_charged_back_lot_never_gets_spendable_credit() {
        for status in STATUSES {
            for state in [LotState::Frozen, LotState::ChargedBack] {
                let target = return_target(LotClass::Purchased, state, status);
                assert!(!matches!(
                    target,
                    ReturnTarget::Allocated | ReturnTarget::Available
                ));
            }
        }
        assert_eq!(
            credit_target(LotClass::Purchased, LotState::ChargedBack),
            ReturnTarget::Refuse
        );
    }
}
