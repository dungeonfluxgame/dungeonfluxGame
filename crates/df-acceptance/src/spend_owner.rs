use std::collections::BTreeMap;

use df_commerce::{
    SpendConsent, SpendOperationObservation, SpendOperationStatus, SpendProposal, SpendRefusal,
    SpendRequest, SpendReservation, SpendReservationStatus, SpendSettlementObservation,
    SpendSettlementProposal, SpendSettlementRefusal, SpendSnapshot, propose_spend,
    propose_spend_settlement,
};
use df_types::{Money, OperationId};

/// A fake of the native supplier-exposure owner that `df-commerce`'s `spend` module docs describe:
/// one snapshot with a revision, a durable status per operation and one reservation per admitted
/// operation.
///
/// It adds two things the policy delegates to its owner and that are the fake's own: the
/// compare-and-set of a candidate against the snapshot revision, and the operation and
/// reservation bookkeeping. Tests that only exercise those are self-tests below, not acceptance
/// evidence. It adds no rule of the policy.
#[derive(Debug)]
pub struct SpendOwner {
    snapshot: SpendSnapshot,
    consent: SpendConsent,
    operations: BTreeMap<OperationId, SpendOperationStatus>,
    reservations: BTreeMap<OperationId, SpendReservation>,
}

impl SpendOwner {
    pub fn new(snapshot: SpendSnapshot, consent: SpendConsent) -> Self {
        Self {
            snapshot,
            consent,
            operations: BTreeMap::new(),
            reservations: BTreeMap::new(),
        }
    }

    pub fn snapshot(&self) -> SpendSnapshot {
        self.snapshot
    }

    pub fn consent(&self) -> SpendConsent {
        self.consent
    }

    pub fn set_consent(&mut self, consent: SpendConsent) {
        self.consent = consent;
    }

    pub fn reservation(&self, operation: OperationId) -> Option<SpendReservation> {
        self.reservations.get(&operation).copied()
    }

    /// Builds a candidate from the current snapshot without committing it.
    pub fn propose(
        &self,
        operation: OperationId,
        maximum_supplier_liability: Money,
    ) -> Result<SpendProposal, SpendRefusal> {
        let status = self
            .operations
            .get(&operation)
            .copied()
            .unwrap_or(SpendOperationStatus::Unseen);
        propose_spend(
            &self.snapshot,
            self.consent,
            SpendRequest {
                operation,
                expected_revision: self.snapshot.revision,
                expected_consent_revision: self.consent.revision,
                maximum_supplier_liability,
            },
            SpendOperationObservation { operation, status },
        )
    }

    /// Commits a candidate only if the snapshot is still at the revision it was built from.
    pub fn commit(&mut self, proposal: SpendProposal) -> Result<(), SpendRefusal> {
        if proposal.before_revision != self.snapshot.revision {
            return Err(SpendRefusal::StaleRevision);
        }
        self.snapshot = proposal.next;
        self.operations
            .insert(proposal.operation, SpendOperationStatus::Recorded);
        self.reservations.insert(
            proposal.operation,
            SpendReservation {
                operation: proposal.operation,
                maximum_supplier_liability: proposal.maximum_supplier_liability,
                status: SpendReservationStatus::Dispatching,
            },
        );
        Ok(())
    }

    /// Proposes and commits in one step, as an owner holding the counter locks would.
    pub fn admit(
        &mut self,
        operation: OperationId,
        maximum_supplier_liability: Money,
    ) -> Result<SpendProposal, SpendRefusal> {
        let proposal = self.propose(operation, maximum_supplier_liability)?;
        self.commit(proposal)?;
        Ok(proposal)
    }

    /// Records that the supplier may have accepted the request but the outcome is not known.
    pub fn mark_outcome_unknown(&mut self, operation: OperationId) {
        self.operations
            .insert(operation, SpendOperationStatus::Unknown);
        self.set_status(operation, SpendReservationStatus::UnknownLiability);
    }

    /// Releases the customer-facing hold. The supplier liability stays open as unknown until it is
    /// reconciled (plan 6.3, finance design 12.3), so the reservation becomes `UnknownLiability`,
    /// never `Released` (which means "never dispatched").
    pub fn release_customer_hold(&mut self, operation: OperationId) {
        self.operations
            .insert(operation, SpendOperationStatus::Unknown);
        self.set_status(operation, SpendReservationStatus::UnknownLiability);
    }

    pub fn settle(
        &mut self,
        observation: SpendSettlementObservation,
    ) -> Result<SpendSettlementProposal, SpendSettlementRefusal> {
        let reservation = self
            .reservations
            .get(&observation.operation)
            .copied()
            .ok_or(SpendSettlementRefusal::WrongOperation)?;
        let proposal = propose_spend_settlement(&self.snapshot, reservation, observation)?;
        self.snapshot = proposal.next;
        self.set_status(observation.operation, SpendReservationStatus::SettledKnown);
        Ok(proposal)
    }

    fn set_status(&mut self, operation: OperationId, status: SpendReservationStatus) {
        if let Some(reservation) = self.reservations.get_mut(&operation) {
            reservation.status = status;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dollars, operation, spend_consent, spend_snapshot_limited_by_job};

    fn owner() -> SpendOwner {
        SpendOwner::new(
            spend_snapshot_limited_by_job(dollars(10)),
            spend_consent(dollars(1)),
        )
    }

    #[test]
    fn candidates_built_from_one_snapshot_commit_exactly_once() {
        let mut owner = owner();
        let candidates: Vec<_> = (1..=6)
            .map(|sequence| owner.propose(operation(sequence), dollars(1)))
            .collect::<Result<_, _>>()
            .expect("every candidate fits when proposed alone");

        let commits: Vec<_> = candidates
            .into_iter()
            .map(|candidate| owner.commit(candidate))
            .collect();

        assert_eq!(commits.iter().filter(|c| c.is_ok()).count(), 1);
        assert!(
            commits[1..]
                .iter()
                .all(|c| *c == Err(SpendRefusal::StaleRevision))
        );
    }

    #[test]
    fn releasing_the_customer_hold_keeps_the_supplier_liability_open() {
        let mut owner = owner();
        owner.admit(operation(1), dollars(1)).expect("fits");

        owner.release_customer_hold(operation(1));

        assert_eq!(
            owner.reservation(operation(1)).map(|r| r.status),
            Some(SpendReservationStatus::UnknownLiability)
        );
    }
}
