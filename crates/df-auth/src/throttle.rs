//! Fixed-window attempt throttling for anonymous and account flows (login, signup, reset), per
//! the coordinator decision of 2026-10-24 and the `service-operations.md` rate ceilings.
//!
//! Pure: one key's window and the supplied time in, a decision out. The bounded map of keys lives
//! with the native caller.

use std::time::Duration;

/// At most `limit` counted attempts per `window`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThrottlePolicy {
    pub limit: u32,
    pub window: Duration,
}

/// One key's current window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Window {
    pub started_at_ms: u64,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThrottleDecision {
    Allowed,
    Throttled { retry_after_ms: u64 },
}

fn window_ms(policy: ThrottlePolicy) -> u64 {
    u64::try_from(policy.window.as_millis()).unwrap_or(u64::MAX)
}

fn current(policy: ThrottlePolicy, window: Option<Window>, now_ms: u64) -> Option<Window> {
    window.filter(|window| now_ms.saturating_sub(window.started_at_ms) < window_ms(policy))
}

/// Whether another attempt may proceed now, without counting it.
pub fn decide(policy: ThrottlePolicy, window: Option<Window>, now_ms: u64) -> ThrottleDecision {
    match current(policy, window, now_ms) {
        Some(window) if window.count >= policy.limit => ThrottleDecision::Throttled {
            retry_after_ms: window
                .started_at_ms
                .saturating_add(window_ms(policy))
                .saturating_sub(now_ms),
        },
        _ => ThrottleDecision::Allowed,
    }
}

/// The window after counting one attempt at `now_ms`; an elapsed window starts over.
pub fn counted(policy: ThrottlePolicy, window: Option<Window>, now_ms: u64) -> Window {
    match current(policy, window, now_ms) {
        Some(window) => Window {
            count: window.count.saturating_add(1),
            ..window
        },
        None => Window {
            started_at_ms: now_ms,
            count: 1,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: ThrottlePolicy = ThrottlePolicy {
        limit: 3,
        window: Duration::from_secs(60),
    };

    #[test]
    fn the_attempt_after_the_limit_is_throttled_until_the_window_ends() {
        let mut window = None;
        for now in [0, 10, 20] {
            assert_eq!(decide(POLICY, window, now), ThrottleDecision::Allowed);
            window = Some(counted(POLICY, window, now));
        }
        assert_eq!(
            decide(POLICY, window, 30),
            ThrottleDecision::Throttled {
                retry_after_ms: 59_970
            }
        );
    }

    #[test]
    fn an_elapsed_window_allows_attempts_again_and_restarts_the_count() {
        let full = Some(Window {
            started_at_ms: 0,
            count: 3,
        });
        assert_eq!(decide(POLICY, full, 60_000), ThrottleDecision::Allowed);
        assert_eq!(
            counted(POLICY, full, 60_000),
            Window {
                started_at_ms: 60_000,
                count: 1
            }
        );
    }
}
