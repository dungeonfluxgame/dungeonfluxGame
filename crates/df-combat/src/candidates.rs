//! Bounded admission of canonical rules offers for later tactical ranking.
use df_model::checkpoint::{Basis, CheckpointPins};

/// Exact canonical session/run/revision and source/content/build pins.
/// Equality detects stale offers; possession of these values grants no authority.
#[derive(Clone, Copy, Debug)]
pub struct CandidateContext<'a> {
    pub basis: &'a Basis,
    pub pins: &'a CheckpointPins,
}

/// Rules and perception owners supply the canonical observation and offer types.
///
/// All methods must be pure, bounded and deterministic. Observation validation
/// must bind the actor, turn, causal input, logical time, combat policy and
/// perception scope to the current authority. Legal admission must recheck offer
/// identity, actor, source, expiry, current resources and timing using that same
/// authority; unsupported mechanics return an owner error, never a fallback.
/// `same_offer` compares canonical offer identities, not presentation payloads.
/// `offer_bytes` reports the exact canonical payload cost in bytes.
pub trait LegalOfferOwner {
    type Observation;
    type Offer;
    type Error;

    fn validate_observation(
        &self,
        observation: &Self::Observation,
        current: CandidateContext<'_>,
    ) -> Result<(), Self::Error>;

    fn is_current_legal(
        &self,
        observation: &Self::Observation,
        current: CandidateContext<'_>,
        offer: &Self::Offer,
    ) -> Result<bool, Self::Error>;

    fn same_offer(&self, left: &Self::Offer, right: &Self::Offer) -> Result<bool, Self::Error>;

    fn offer_bytes(&self, offer: &Self::Offer) -> Result<usize, Self::Error>;
}

/// Explicit finite input, work and admitted-output budgets. Zero is a valid cap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateLimits {
    pub max_offers: usize,
    pub max_identity_comparisons: usize,
    pub max_candidates: usize,
    pub max_candidate_bytes: usize,
}

/// Safe rejection classes and positions; no private offer payload is retained.
#[derive(Debug, Eq, PartialEq)]
pub enum CandidateError<E> {
    OfferCapacity { required: usize, limit: usize },
    ComparisonCapacity { required: usize, limit: usize },
    ComparisonCountOverflow,
    StaleBasis,
    StalePins,
    Duplicate { first: usize, duplicate: usize },
    CandidateCapacity { limit: usize },
    ByteCapacity { limit: usize },
    AllocationCapacity,
    Owner(E),
}

/// Exact borrowed legally admitted offers, in input order, with their current pins.
///
/// This receipt cannot be constructed by callers. Empty offers are a successful
/// no-action outcome. It is a staged result: rules/engine/session must revalidate
/// before selecting or committing a later proposal. It grants no disclosure rights.
pub struct AdmittedCandidates<'a, Offer> {
    context: CandidateContext<'a>,
    offers: Vec<&'a Offer>,
}

impl<'a, Offer> AdmittedCandidates<'a, Offer> {
    pub fn basis(&self) -> &'a Basis {
        self.context.basis
    }

    pub fn pins(&self) -> &'a CheckpointPins {
        self.context.pins
    }

    pub fn offers(&self) -> &[&'a Offer] {
        &self.offers
    }
}

pub type CandidateResult<'a, Owner> = Result<
    AdmittedCandidates<'a, <Owner as LegalOfferOwner>::Offer>,
    CandidateError<<Owner as LegalOfferOwner>::Error>,
>;

/// Admit a finite supplied legal-action inventory without inventing any offers.
///
/// Input/work capacity is checked before owner callbacks. Exact basis/pins must
/// match before observation, identity or legality checks. Duplicate identities
/// reject the whole batch, even if their payloads differ. Nonlegal offers are
/// omitted; owner failure or output capacity returns no partial receipt. At most
/// n*(n-1)/2 identity comparisons and n legality/byte checks occur. Offers are
/// borrowed, never cloned, ranked, resolved or mutated here.
pub fn enumerate_candidates<'a, Owner: LegalOfferOwner>(
    owner: &Owner,
    observation: &Owner::Observation,
    offered: CandidateContext<'_>,
    current: CandidateContext<'a>,
    offers: &'a [Owner::Offer],
    limits: CandidateLimits,
) -> CandidateResult<'a, Owner> {
    if offers.len() > limits.max_offers {
        return Err(CandidateError::OfferCapacity {
            required: offers.len(),
            limit: limits.max_offers,
        });
    }
    let comparisons =
        identity_comparisons(offers.len()).ok_or(CandidateError::ComparisonCountOverflow)?;
    if comparisons > limits.max_identity_comparisons {
        return Err(CandidateError::ComparisonCapacity {
            required: comparisons,
            limit: limits.max_identity_comparisons,
        });
    }
    if offered.basis != current.basis {
        return Err(CandidateError::StaleBasis);
    }
    if offered.pins != current.pins {
        return Err(CandidateError::StalePins);
    }
    owner
        .validate_observation(observation, current)
        .map_err(CandidateError::Owner)?;
    for (duplicate, offer) in offers.iter().enumerate() {
        for (first, earlier) in offers.iter().take(duplicate).enumerate() {
            if owner
                .same_offer(earlier, offer)
                .map_err(CandidateError::Owner)?
            {
                return Err(CandidateError::Duplicate { first, duplicate });
            }
        }
    }
    let mut admitted = Vec::new();
    admitted
        .try_reserve_exact(offers.len().min(limits.max_candidates))
        .map_err(|_| CandidateError::AllocationCapacity)?;
    let mut admitted_bytes = 0_usize;
    for offer in offers {
        if !owner
            .is_current_legal(observation, current, offer)
            .map_err(CandidateError::Owner)?
        {
            continue;
        }
        if admitted.len() == limits.max_candidates {
            return Err(CandidateError::CandidateCapacity {
                limit: limits.max_candidates,
            });
        }
        let cost = owner.offer_bytes(offer).map_err(CandidateError::Owner)?;
        admitted_bytes = admitted_bytes
            .checked_add(cost)
            .filter(|total| *total <= limits.max_candidate_bytes)
            .ok_or(CandidateError::ByteCapacity {
                limit: limits.max_candidate_bytes,
            })?;
        admitted.push(offer);
    }
    Ok(AdmittedCandidates {
        context: current,
        offers: admitted,
    })
}

fn identity_comparisons(count: usize) -> Option<usize> {
    if count < 2 {
        return Some(0);
    }
    if count.is_multiple_of(2) {
        (count / 2).checked_mul(count - 1)
    } else {
        count.checked_mul((count - 1) / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::identity_comparisons;

    #[test]
    fn work_estimate_handles_empty_exact_and_overflow_without_panicking() {
        assert_eq!(identity_comparisons(0), Some(0));
        assert_eq!(identity_comparisons(1), Some(0));
        assert_eq!(identity_comparisons(4), Some(6));
        assert_eq!(identity_comparisons(5), Some(10));
        assert_eq!(identity_comparisons(usize::MAX), None);
    }
}
