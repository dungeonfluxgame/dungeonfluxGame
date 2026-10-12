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

/// How the owner keeps its session lease alive while nobody is acting.
///
/// The lease (and the access grants that ride on it) lasts a fixed time; an owner that renews
/// only when something is committed loses an idle table's game once the lease lapses. The owner
/// therefore renews on a timer, at a fifth of the lease, and again before serving any call that
/// arrives after a long pause (the timer is a deadline, so a pause makes it overdue).
pub(super) struct Heartbeat {
    period: Duration,
    next: Option<Instant>,
    failing: bool,
}

/// The wait before renewing again after a failed renewal.
const RENEWAL_RETRY: Duration = Duration::from_secs(5);

impl Heartbeat {
    /// A heartbeat for a lease of `lease_seconds`: every fifth of it, at least every second.
    pub(super) fn for_lease(lease_seconds: u64) -> Self {
        Self {
            period: Duration::from_secs((lease_seconds / 5).max(1)),
            next: None,
            failing: false,
        }
    }

    /// When the next renewal is due; before the first one, now.
    pub(super) fn next_due(&self, now: Instant) -> Instant {
        self.next.unwrap_or(now)
    }

    pub(super) fn due(&self, now: Instant) -> bool {
        self.next.is_none_or(|due| now >= due)
    }

    /// Records a renewal; returns whether renewals had been failing until now.
    pub(super) fn renewed(&mut self, now: Instant) -> bool {
        self.next = Some(now + self.period);
        std::mem::take(&mut self.failing)
    }

    /// Records a failed renewal; returns whether this is the first of a streak.
    pub(super) fn failed(&mut self, now: Instant) -> bool {
        self.next = Some(now + RENEWAL_RETRY);
        !std::mem::replace(&mut self.failing, true)
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

    /// A session whose ownership is already known to be gone.
    pub(super) fn lost(now: Instant) -> Self {
        Self {
            lost: true,
            ..Self::fenced(now, 0)
        }
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

    /// A model of the database's lease: it lapses `LEASE` after the last renewal, a call is
    /// refused ("Outcome unknown", a permission error) once it has lapsed, and only the owner
    /// holding the current fence may renew it.
    struct Lease {
        until: Instant,
        fence: u8,
    }
    const LEASE: Duration = Duration::from_secs(300);
    impl Lease {
        fn renew(&mut self, now: Instant, fence: u8) -> Result<(), RepositoryError> {
            if fence != self.fence {
                return Err(RepositoryError::RevisionConflict);
            }
            self.until = now + LEASE;
            Ok(())
        }
        fn serve(&self, now: Instant) -> Result<(), RepositoryError> {
            if now >= self.until {
                Err(RepositoryError::ExpiredOwner)
            } else {
                Ok(())
            }
        }
    }

    /// One owner over a controlled clock: it renews when its heartbeat is due, before any call
    /// and at each wake, exactly as the actor does.
    fn serve_with_heartbeat(
        lease: &mut Lease,
        heartbeat: &mut Heartbeat,
        now: Instant,
    ) -> Result<(), RepositoryError> {
        if heartbeat.due(now) {
            match lease.renew(now, 1) {
                Ok(()) => {
                    heartbeat.renewed(now);
                }
                Err(error) => {
                    heartbeat.failed(now);
                    return Err(error);
                }
            }
        }
        lease.serve(now)
    }

    #[test]
    fn an_idle_table_keeps_its_session_for_hours() {
        let start = Instant::now();
        let mut lease = Lease {
            until: start + LEASE,
            fence: 1,
        };
        let mut heartbeat = Heartbeat::for_lease(LEASE.as_secs());
        serve_with_heartbeat(&mut lease, &mut heartbeat, start).unwrap();
        // No player does anything for three hours; the owner only wakes at its own deadlines.
        let mut now = start;
        let end = start + Duration::from_secs(3 * 3600);
        while now < end {
            now = heartbeat.next_due(now);
            serve_with_heartbeat(&mut lease, &mut heartbeat, now).unwrap();
            assert!(lease.until > now, "the lease lapsed at {:?}", now - start);
        }
        // Then a choice, a typed line and a watch re-open all find a live session.
        for _ in 0..3 {
            serve_with_heartbeat(&mut lease, &mut heartbeat, end + Duration::from_secs(1)).unwrap();
        }
    }

    #[test]
    fn without_a_heartbeat_the_lease_lapses_and_every_call_is_refused() {
        let start = Instant::now();
        let lease = Lease {
            until: start + LEASE,
            fence: 1,
        };
        assert!(lease.serve(start + Duration::from_secs(299)).is_ok());
        assert_eq!(
            lease.serve(start + Duration::from_secs(301)),
            Err(RepositoryError::ExpiredOwner)
        );
    }

    #[test]
    fn a_lapsed_lease_is_renewed_before_the_first_call_after_a_long_pause() {
        let start = Instant::now();
        let mut lease = Lease {
            until: start + LEASE,
            fence: 1,
        };
        let mut heartbeat = Heartbeat::for_lease(LEASE.as_secs());
        serve_with_heartbeat(&mut lease, &mut heartbeat, start).unwrap();
        // The machine was suspended for ten minutes: the lease lapsed and the timer is overdue.
        let resumed = start + Duration::from_secs(600);
        assert!(lease.serve(resumed).is_err());
        assert!(heartbeat.due(resumed));
        assert_eq!(
            serve_with_heartbeat(&mut lease, &mut heartbeat, resumed),
            Ok(())
        );
    }

    #[test]
    fn a_renewal_that_fails_is_retried_soon_and_a_taken_over_session_is_lost() {
        let start = Instant::now();
        let mut heartbeat = Heartbeat::for_lease(300);
        assert!(
            heartbeat.failed(start),
            "the first failure of a streak is reported"
        );
        assert!(!heartbeat.failed(start), "later ones are not");
        assert_eq!(heartbeat.next_due(start), start + Duration::from_secs(5));
        assert!(
            heartbeat.renewed(start + Duration::from_secs(5)),
            "the streak ended"
        );
        assert_eq!(
            heartbeat.next_due(start),
            start + Duration::from_secs(5) + Duration::from_secs(60)
        );
        let mut lease = Lease {
            until: start + LEASE,
            fence: 2,
        };
        let error = lease.renew(start, 1).unwrap_err();
        assert_eq!(classify(error), Resolution::Retry(error));
        // Another owner holds the fence: the database says so as a revision conflict, which the
        // owner treats as lost ownership (see `Actor::heartbeat_if_due`).
        assert_eq!(error, RepositoryError::RevisionConflict);
        assert_eq!(Heartbeat::for_lease(0).period, Duration::from_secs(1));
    }
}
