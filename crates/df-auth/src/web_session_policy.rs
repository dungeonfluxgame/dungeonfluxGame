//! Web session validity for the customer portal (coordinator decision of 2026-10-24: opaque
//! cookie token, idle and absolute expiry, revocation, credential-generation fencing per G04-D01).
//!
//! Pure: the caller supplies the stored facts and the time; nothing here reads a cookie, a clock
//! or a database.

use std::time::Duration;

/// How long a session may live. Owner-tunable; the defaults are the portal's customer values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionLifetimes {
    /// A session unused for this long expires.
    pub idle: Duration,
    /// A session expires this long after it was created, however often it is used.
    pub absolute: Duration,
    /// The stored last-use time is refreshed at most this often, to bound writes.
    pub touch_interval: Duration,
}

/// The stored facts of one session, as read inside the caller's lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredSession {
    pub created_at_ms: u64,
    pub last_seen_at_ms: u64,
    pub is_revoked: bool,
    /// The account's credential generation when the session was issued.
    pub credential_generation: u64,
}

/// Whether a presented session may act.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionCheck {
    Active {
        needs_touch: bool,
    },
    Revoked,
    /// The password changed or was reset after the session was issued.
    StaleGeneration,
    IdleExpired,
    AbsoluteExpired,
}

impl SessionCheck {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Active { .. })
    }
}

/// Decides one session's validity at `now_ms`. Checks run in order of what the person can act on:
/// an explicit revocation or credential change wins over plain expiry.
pub fn check_session(
    stored: StoredSession,
    current_credential_generation: u64,
    lifetimes: SessionLifetimes,
    now_ms: u64,
) -> SessionCheck {
    if stored.is_revoked {
        return SessionCheck::Revoked;
    }
    if stored.credential_generation != current_credential_generation {
        return SessionCheck::StaleGeneration;
    }
    let since_created = now_ms.saturating_sub(stored.created_at_ms);
    if u128::from(since_created) >= lifetimes.absolute.as_millis() {
        return SessionCheck::AbsoluteExpired;
    }
    let since_seen = now_ms.saturating_sub(stored.last_seen_at_ms);
    if u128::from(since_seen) >= lifetimes.idle.as_millis() {
        return SessionCheck::IdleExpired;
    }
    SessionCheck::Active {
        needs_touch: u128::from(since_seen) >= lifetimes.touch_interval.as_millis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE_MS: u64 = 60_000;

    fn lifetimes() -> SessionLifetimes {
        SessionLifetimes {
            idle: Duration::from_secs(60 * 60),
            absolute: Duration::from_secs(4 * 60 * 60),
            touch_interval: Duration::from_secs(5 * 60),
        }
    }

    fn stored(created_at_ms: u64, last_seen_at_ms: u64) -> StoredSession {
        StoredSession {
            created_at_ms,
            last_seen_at_ms,
            is_revoked: false,
            credential_generation: 3,
        }
    }

    #[test]
    fn a_fresh_session_is_active_without_a_touch() {
        assert_eq!(
            check_session(stored(0, 0), 3, lifetimes(), MINUTE_MS),
            SessionCheck::Active { needs_touch: false }
        );
    }

    #[test]
    fn a_session_used_five_minutes_ago_needs_a_touch() {
        assert_eq!(
            check_session(stored(0, 0), 3, lifetimes(), 5 * MINUTE_MS),
            SessionCheck::Active { needs_touch: true }
        );
    }

    #[test]
    fn an_idle_hour_expires_the_session_but_one_minute_less_does_not() {
        assert!(check_session(stored(0, 0), 3, lifetimes(), 59 * MINUTE_MS).is_active());
        assert_eq!(
            check_session(stored(0, 0), 3, lifetimes(), 60 * MINUTE_MS),
            SessionCheck::IdleExpired
        );
    }

    #[test]
    fn the_absolute_lifetime_expires_a_session_in_constant_use() {
        let now = 4 * 60 * MINUTE_MS;
        assert_eq!(
            check_session(stored(0, now - MINUTE_MS), 3, lifetimes(), now),
            SessionCheck::AbsoluteExpired
        );
    }

    #[test]
    fn a_changed_password_makes_an_older_session_stale() {
        assert_eq!(
            check_session(stored(0, 0), 4, lifetimes(), MINUTE_MS),
            SessionCheck::StaleGeneration
        );
    }

    #[test]
    fn revocation_wins_over_every_other_outcome() {
        let revoked = StoredSession {
            is_revoked: true,
            ..stored(0, 0)
        };
        assert_eq!(
            check_session(revoked, 9, lifetimes(), u64::MAX),
            SessionCheck::Revoked
        );
    }
}
