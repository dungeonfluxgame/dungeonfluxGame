//! A fenced session heals itself.
//!
//! When a commit's outcome is unknown (it ran past its bound, or its acknowledgement was lost)
//! the owner fences the session: no view is served until the durable state has been read again.
//! Waiting for a player to retry would leave every screen frozen, so the owner retries the exact
//! retained operation itself, with a backoff, for as long as the process lives. A retry that
//! finds the commit recovers the session; one that finds the ownership itself gone marks the
//! session lost, which every screen is told plainly instead of hanging.

use std::time::{Duration, Instant};

use df_model::checkpoint::GameInput;
use df_observe::OperationContext;
use df_session::inbox::AdmissionSequence;
use df_session::inbox::Reducer;
use df_session::submission::{
    DurableOwner, OwnedInput, PublicationOwner, RepositoryError, SessionEngine, SessionRepository,
    SubmissionOutcome,
};

/// The first wait before a recovery attempt, doubling to [`CEILING`].
const FIRST_WAIT: Duration = Duration::from_millis(250);
const CEILING: Duration = Duration::from_secs(5);

/// The wait before recovery attempt number `attempt` (0 for the first).
pub(super) fn delay(attempt: u32) -> Duration {
    FIRST_WAIT
        .checked_mul(1_u32.checked_shl(attempt.min(8)).unwrap_or(u32::MAX))
        .map_or(CEILING, |wait| wait.min(CEILING))
}

/// How one recovery attempt ended.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Resolution {
    /// The durable state was read again and the session may serve views.
    Recovered,
    /// Not yet: the database was unreachable or the commit still unresolved. Try again.
    Retry(RepositoryError),
    /// The ownership of the saved state is gone for good; only a restart helps.
    Lost(RepositoryError),
}

/// Sorts a failure into "try again" and "the saved state is lost".
pub(super) fn classify(error: RepositoryError) -> Resolution {
    match error {
        RepositoryError::StaleFence
        | RepositoryError::ExpiredOwner
        | RepositoryError::StaleEpoch
        | RepositoryError::RetiredNamespace
        | RepositoryError::SequenceExhausted => Resolution::Lost(error),
        other => Resolution::Retry(other),
    }
}

/// Retries the retained operation (`scope` and `input`) against `owner`: an uncertain commit is
/// resolved by looking its receipt up again, any other fence by a validated reload.
pub(super) fn recover<R, E, P>(
    owner: &mut DurableOwner<R, E, P>,
    scope: R::Scope,
    input: &GameInput,
    context: &OperationContext,
) -> Resolution
where
    R: SessionRepository,
    E: SessionEngine<R::Scope>,
    P: PublicationOwner<R::Scope>,
{
    if !owner.has_uncertain_operation() {
        return match owner.reload_current(&scope, context) {
            Ok(()) if owner.is_current() => Resolution::Recovered,
            Ok(()) => Resolution::Retry(RepositoryError::UnresolvedCommit),
            Err(error) => classify(error),
        };
    }
    match owner.matches_uncertain_retry(&scope, input) {
        Ok(true) => {}
        Ok(false) => return Resolution::Retry(RepositoryError::UnresolvedCommit),
        Err(error) => return classify(error),
    }
    let (item, receipt) = OwnedInput::new(
        OperationContext {
            trace_parent: context.trace_parent.clone(),
            build: context.build.clone(),
        },
        scope,
        input.clone(),
    );
    owner.reduce(AdmissionSequence(0), item);
    match receipt.try_recv() {
        Ok(SubmissionOutcome::Confirmed(_)) if owner.is_current() => Resolution::Recovered,
        Ok(SubmissionOutcome::Refused(error)) => classify(error),
        Ok(_) | Err(_) => Resolution::Retry(RepositoryError::UnresolvedCommit),
    }
}

/// What the owner remembers about the session while it is fenced.
pub(super) struct Health {
    since: Instant,
    commit_millis: u64,
    attempts: u32,
    next: Instant,
    lost: bool,
}

impl Health {
    pub(super) fn fenced(now: Instant, commit_millis: u64) -> Self {
        Self {
            since: now,
            commit_millis,
            attempts: 0,
            next: now,
            lost: false,
        }
    }

    /// The `SESSION` line printed when the session becomes fenced.
    pub(super) fn fenced_line(&self) -> String {
        format!(
            "SESSION fenced (commit outcome unknown after {} ms)",
            self.commit_millis
        )
    }

    pub(super) fn is_lost(&self) -> bool {
        self.lost
    }

    /// When the next attempt is due; `None` once the session is lost (nothing more to try).
    pub(super) fn next_attempt(&self) -> Option<Instant> {
        (!self.lost).then_some(self.next)
    }

    pub(super) fn attempts(&self) -> u32 {
        self.attempts
    }

    pub(super) fn down_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.since)
    }

    /// Records the end of an attempt: schedules the next one, or marks the session lost.
    pub(super) fn attempted(&mut self, now: Instant, resolution: &Resolution) {
        self.attempts = self.attempts.saturating_add(1);
        match resolution {
            Resolution::Lost(_) => self.lost = true,
            Resolution::Retry(_) | Resolution::Recovered => {
                self.next = now + delay(self.attempts);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_grows_to_a_ceiling_and_never_gives_up() {
        let waits: Vec<u64> = (0..8).map(|n| delay(n).as_millis() as u64).collect();
        assert_eq!(waits[..5], [250, 500, 1_000, 2_000, 4_000]);
        assert!(waits[5..].iter().all(|wait| *wait == 5_000), "{waits:?}");
        assert_eq!(delay(u32::MAX), CEILING);
    }

    #[test]
    fn only_lost_ownership_is_final() {
        for error in [
            RepositoryError::Unavailable,
            RepositoryError::UnresolvedCommit,
            RepositoryError::RevisionConflict,
            RepositoryError::Capacity,
            RepositoryError::InputBinding,
        ] {
            assert_eq!(classify(error), Resolution::Retry(error), "{error:?}");
        }
        for error in [
            RepositoryError::StaleFence,
            RepositoryError::ExpiredOwner,
            RepositoryError::StaleEpoch,
            RepositoryError::RetiredNamespace,
        ] {
            assert_eq!(classify(error), Resolution::Lost(error), "{error:?}");
        }
    }

    #[test]
    fn health_schedules_each_attempt_and_stops_when_lost() {
        let start = Instant::now();
        let mut health = Health::fenced(start, 2_100);
        assert_eq!(
            health.fenced_line(),
            "SESSION fenced (commit outcome unknown after 2100 ms)"
        );
        assert_eq!(health.next_attempt(), Some(start));
        health.attempted(start, &Resolution::Retry(RepositoryError::Unavailable));
        assert_eq!(health.next_attempt(), Some(start + delay(1)));
        assert_eq!(health.attempts(), 1);
        health.attempted(start, &Resolution::Lost(RepositoryError::StaleFence));
        assert!(health.is_lost());
        assert_eq!(health.next_attempt(), None);
        assert_eq!(
            health.down_for(start + Duration::from_secs(3)),
            Duration::from_secs(3)
        );
    }
}
