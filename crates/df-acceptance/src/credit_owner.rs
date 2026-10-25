use std::collections::BTreeMap;

use df_commerce::{
    CreditAdjustment, CreditEvent, CreditObservation, CreditReceipt, CreditRefusal, CreditState,
    ObservationAuthority, OriginalCharge, RefundObligation, propose_credit_adjustment,
};
use df_types::{Money, OperationId};

/// Result of applying a credit command through the owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditApplied {
    Committed,
    Replayed,
}

/// A fake of the native credit owner that `df-commerce`'s `credit` module docs describe.
///
/// It supplies what the pure policy delegates to its owner: the prior receipt of the exact
/// operation, the outstanding refund and the remaining reversible charge, and it commits the
/// transition and receipt together. It adds no rule of the policy, but the receipt bookkeeping is
/// the fake's own, so tests that only exercise that bookkeeping are self-tests below, not
/// acceptance evidence. `credit.rs` retires with decision 30; the tests that use this owner are
/// retargeted to `df-finance` when it lands.
#[derive(Debug)]
pub struct CreditOwner {
    state: CreditState,
    receipts: BTreeMap<OperationId, CreditReceipt>,
    refunds: BTreeMap<OperationId, RefundObligation>,
    reversible: BTreeMap<OperationId, Money>,
}

impl CreditOwner {
    pub fn new(state: CreditState) -> Self {
        Self {
            state,
            receipts: BTreeMap::new(),
            refunds: BTreeMap::new(),
            reversible: BTreeMap::new(),
        }
    }

    pub fn state(&self) -> CreditState {
        self.state
    }

    /// Principal a payer can still spend: deferred credit not held for a refund.
    pub fn available(&self) -> Money {
        self.state
            .deferred_credit
            .checked_sub(self.state.refund_hold)
            .expect("the policy keeps refund holds within deferred credit")
    }

    pub fn apply(
        &mut self,
        operation: OperationId,
        event: CreditEvent,
        authority: ObservationAuthority,
    ) -> Result<CreditApplied, CreditRefusal> {
        self.apply_expecting(self.state.revision, operation, event, authority)
    }

    /// Applies a command computed against `expected_revision`, which may be stale.
    pub fn apply_expecting(
        &mut self,
        expected_revision: u64,
        operation: OperationId,
        event: CreditEvent,
        authority: ObservationAuthority,
    ) -> Result<CreditApplied, CreditRefusal> {
        let receipt = CreditReceipt { operation, event };
        let observation = CreditObservation {
            authority,
            expected_revision,
            prior_receipt: self.receipts.get(&operation).copied(),
            refund: self.outstanding_refund(event),
            reversible_charge: self.reversible_charge(event),
        };
        match propose_credit_adjustment(self.state, receipt, observation) {
            Ok(transition) => {
                self.state = transition.next;
                self.receipts.insert(operation, receipt);
                self.record(operation, transition.adjustment);
                Ok(CreditApplied::Committed)
            }
            Err(CreditRefusal::AlreadyRecorded) => Ok(CreditApplied::Replayed),
            Err(refusal) => Err(refusal),
        }
    }

    fn outstanding_refund(&self, event: CreditEvent) -> Option<RefundObligation> {
        match event {
            CreditEvent::ObserveRefund {
                refund_operation, ..
            } => self.refunds.get(&refund_operation).copied(),
            _ => None,
        }
    }

    fn reversible_charge(&self, event: CreditEvent) -> Option<OriginalCharge> {
        match event {
            CreditEvent::ReverseConsumption {
                original_operation, ..
            } => self
                .reversible
                .get(&original_operation)
                .map(|remaining| OriginalCharge {
                    operation: original_operation,
                    remaining_reversible: *remaining,
                }),
            _ => None,
        }
    }

    fn record(&mut self, operation: OperationId, adjustment: CreditAdjustment) {
        match adjustment {
            CreditAdjustment::ConsumptionEarned { amount } => {
                self.reversible.insert(operation, amount);
            }
            CreditAdjustment::ConsumptionReversed {
                original_operation,
                amount,
            } => {
                if let Some(remaining) = self.reversible.get_mut(&original_operation) {
                    *remaining = remaining
                        .checked_sub(amount)
                        .expect("the policy bounds a reversal by the remaining charge");
                }
            }
            CreditAdjustment::RefundHeld(obligation)
            | CreditAdjustment::RefundUnknown(obligation) => {
                self.refunds.insert(obligation.operation, obligation);
            }
            CreditAdjustment::RefundPaid { operation, .. }
            | CreditAdjustment::RefundFailed { operation, .. } => {
                self.refunds.remove(&operation);
            }
            CreditAdjustment::TopupDeferred { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dollars, empty_credit_state, operation};

    fn topup() -> CreditEvent {
        CreditEvent::SettledTopup {
            amount: dollars(20),
        }
    }

    #[test]
    fn a_command_replayed_against_the_same_owner_is_a_replay_and_changes_nothing() {
        let mut owner = CreditOwner::new(empty_credit_state());
        owner
            .apply(operation(1), topup(), ObservationAuthority::VerifiedLatest)
            .expect("the first application commits");
        let after_first = owner.state();

        let replay = owner.apply(operation(1), topup(), ObservationAuthority::VerifiedLatest);

        assert_eq!(replay, Ok(CreditApplied::Replayed));
        assert_eq!(owner.state(), after_first);
    }

    #[test]
    fn an_owner_rebuilt_from_old_state_without_its_receipts_posts_the_same_command_again() {
        // Why receipts must live in the same durable store as the balances they protect.
        let mut original = CreditOwner::new(empty_credit_state());
        original
            .apply(operation(1), topup(), ObservationAuthority::VerifiedLatest)
            .expect("the first application commits");
        let mut rebuilt = CreditOwner::new(empty_credit_state());

        let again = rebuilt.apply(operation(1), topup(), ObservationAuthority::VerifiedLatest);

        assert_eq!(again, Ok(CreditApplied::Committed));
    }

    #[test]
    fn an_expected_revision_older_than_the_state_reaches_the_policy_as_stale() {
        let mut owner = CreditOwner::new(empty_credit_state());
        owner
            .apply(operation(1), topup(), ObservationAuthority::VerifiedLatest)
            .expect("the first application commits");

        let stale = owner.apply_expecting(
            0,
            operation(2),
            topup(),
            ObservationAuthority::VerifiedLatest,
        );

        assert_eq!(stale, Err(CreditRefusal::StaleRevision));
    }
}
