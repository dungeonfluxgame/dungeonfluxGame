use std::mem::size_of;
use std::time::Duration;

use df_types::{RunId, SessionId};

const MAXIMUM_REFUSALS: usize = 4096;
const MAXIMUM_RECORD_BYTES: usize = 1024 * 1024;

/// The authoritative caller's fixed session/run scope. IDs do not confer access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefusalScope {
    pub session: SessionId,
    pub run: RunId,
}

/// Refusal memory admits explicit declines only; absence of input is not a decline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalError {
    InvalidCapacity,
    InvalidCooldown,
    RecordBudget,
    WrongSession,
    WrongRun,
    TimeRegression,
    TimeOverflow,
    Capacity,
}

/// Only voluntary opportunity eligibility changes; no mechanical penalty is emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalEligibility {
    Eligible,
    CoolingDown { remaining: Duration },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclineOutcome {
    Recorded { ready_at: Duration },
    AlreadyRecorded { ready_at: Duration },
}

#[derive(Clone, Copy)]
struct Refusal<ParticipantId, OpportunityId> {
    participant: ParticipantId,
    opportunity: OpportunityId,
    ready_at: Duration,
}

/// Bounded, pure refusal state owned by the authoritative caller's candidate.
///
/// Participant and opportunity IDs must be trusted, inline caller-owned IDs, not
/// private content. The caller validates consent, opportunity provenance and
/// accepted-input identity before calling `record_decline`. Cloning this value
/// stages a candidate; this crate does not commit gameplay or publish a cue.
///
/// Cooldown uses explicit elapsed logical time from a fixed run origin. An exact
/// retained participant/opportunity retry never extends its deadline. Expired
/// records are evicted only when admitting a new decline at capacity; the caller
/// owns durable input deduplication beyond this bounded retention. Active records
/// are never evicted to make optional opportunities eligible.
#[derive(Clone)]
pub struct RefusalMemory<ParticipantId, OpportunityId> {
    scope: RefusalScope,
    max_refusals: usize,
    cooldown: Duration,
    last_decline_time: Duration,
    refusals: Vec<Refusal<ParticipantId, OpportunityId>>,
}

impl<ParticipantId: Copy + Eq, OpportunityId: Copy + Eq>
    RefusalMemory<ParticipantId, OpportunityId>
{
    /// Allocates at most 4096 records and one MiB of inline refusal storage.
    pub fn new(
        scope: RefusalScope,
        max_refusals: usize,
        cooldown: Duration,
    ) -> Result<Self, RefusalError> {
        if max_refusals == 0 || max_refusals > MAXIMUM_REFUSALS {
            return Err(RefusalError::InvalidCapacity);
        }
        if cooldown.is_zero() {
            return Err(RefusalError::InvalidCooldown);
        }
        let record_bytes = size_of::<Refusal<ParticipantId, OpportunityId>>()
            .checked_mul(max_refusals)
            .ok_or(RefusalError::RecordBudget)?;
        if record_bytes > MAXIMUM_RECORD_BYTES {
            return Err(RefusalError::RecordBudget);
        }
        let mut refusals = Vec::new();
        refusals
            .try_reserve_exact(max_refusals)
            .map_err(|_| RefusalError::Capacity)?;
        Ok(Self {
            scope,
            max_refusals,
            cooldown,
            last_decline_time: Duration::ZERO,
            refusals,
        })
    }

    /// Records one explicitly declined opportunity. Failure changes no state.
    /// A distinct explicit decline may extend this participant's cooldown.
    pub fn record_decline(
        &mut self,
        scope: RefusalScope,
        participant: ParticipantId,
        opportunity: OpportunityId,
        now: Duration,
    ) -> Result<DeclineOutcome, RefusalError> {
        self.validate(scope, now)?;
        if let Some(refusal) = self.refusals.iter().find(|refusal| {
            refusal.participant == participant && refusal.opportunity == opportunity
        }) {
            let ready_at = refusal.ready_at;
            self.last_decline_time = now;
            return Ok(DeclineOutcome::AlreadyRecorded { ready_at });
        }
        let ready_at = now
            .checked_add(self.cooldown)
            .ok_or(RefusalError::TimeOverflow)?;
        let expired_slot = if self.refusals.len() == self.max_refusals {
            Some(
                self.refusals
                    .iter()
                    .position(|refusal| refusal.ready_at <= now)
                    .ok_or(RefusalError::Capacity)?,
            )
        } else {
            None
        };
        // Clone may retain only the used slots. Restore the validated exact
        // reservation before mutation so push cannot double beyond the byte cap.
        self.refusals
            .try_reserve_exact(self.max_refusals - self.refusals.len())
            .map_err(|_| RefusalError::Capacity)?;
        if let Some(index) = expired_slot {
            self.refusals.remove(index);
        }
        self.refusals.push(Refusal {
            participant,
            opportunity,
            ready_at,
        });
        self.last_decline_time = now;
        Ok(DeclineOutcome::Recorded { ready_at })
    }

    /// Observes eligibility without recording refusal or inferring motivation.
    /// Eligibility at the exact deadline is restored without any further input.
    pub fn eligibility(
        &self,
        scope: RefusalScope,
        participant: ParticipantId,
        now: Duration,
    ) -> Result<RefusalEligibility, RefusalError> {
        self.validate(scope, now)?;
        match self
            .refusals
            .iter()
            .filter(|refusal| refusal.participant == participant)
            .map(|refusal| refusal.ready_at)
            .max()
            .filter(|ready_at| *ready_at > now)
        {
            Some(ready_at) => Ok(RefusalEligibility::CoolingDown {
                remaining: ready_at.saturating_sub(now),
            }),
            None => Ok(RefusalEligibility::Eligible),
        }
    }

    pub fn len(&self) -> usize {
        self.refusals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.refusals.is_empty()
    }

    fn validate(&self, scope: RefusalScope, now: Duration) -> Result<(), RefusalError> {
        if scope.session != self.scope.session {
            return Err(RefusalError::WrongSession);
        }
        if scope.run != self.scope.run {
            return Err(RefusalError::WrongRun);
        }
        if now < self.last_decline_time {
            return Err(RefusalError::TimeRegression);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type LargeMemory = RefusalMemory<[u8; 16385], u16>;

    fn scope() -> RefusalScope {
        RefusalScope {
            session: SessionId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
        }
    }

    fn fill_within_record_budget(memory: &mut LargeMemory, first_opportunity: u16) {
        for opportunity in first_opportunity..63 {
            memory
                .record_decline(scope(), [1; 16385], opportunity, Duration::ZERO)
                .unwrap();
            let allocated_record_bytes =
                memory.refusals.capacity() * size_of::<Refusal<[u8; 16385], u16>>();
            assert!(allocated_record_bytes <= MAXIMUM_RECORD_BYTES);
            assert_eq!(memory.refusals.len(), usize::from(opportunity) + 1);
        }
    }

    #[test]
    fn filling_an_empty_clone_preserves_the_validated_allocation_bound() {
        let accepted = LargeMemory::new(scope(), 63, Duration::from_secs(1)).unwrap();
        let mut candidate = accepted.clone();
        fill_within_record_budget(&mut candidate, 0);
        assert!(accepted.is_empty());
        assert_eq!(candidate.len(), 63);
    }

    #[test]
    fn filling_a_partial_clone_preserves_the_validated_allocation_bound() {
        let mut accepted = LargeMemory::new(scope(), 63, Duration::from_secs(1)).unwrap();
        accepted
            .record_decline(scope(), [1; 16385], 0, Duration::ZERO)
            .unwrap();
        let mut candidate = accepted.clone();
        fill_within_record_budget(&mut candidate, 1);
        assert_eq!(accepted.len(), 1);
        assert_eq!(candidate.len(), 63);
        assert_eq!(
            candidate.record_decline(scope(), [2; 16385], 64, Duration::ZERO),
            Err(RefusalError::Capacity)
        );
    }
}
