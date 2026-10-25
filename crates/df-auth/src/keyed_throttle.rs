//! A bounded map of fixed-window throttles that **fails closed** (`planning/campaigns-and-table-
//! research.md` 4.3). One instance serves one layer (lookup per IP, lookup per account, display
//! challenge per IP, ...), so flooding one layer's keys cannot lock out another.
//!
//! At capacity an insert first sweeps windows that have already elapsed (they carry no
//! information). If the map is still full the new key is refused (`Saturated`); a live window is
//! never evicted, so a flood of fresh keys cannot reset a real key's counter. Pure: the time is
//! supplied, nothing is read from a clock.

use std::collections::HashMap;
use std::hash::Hash;

use crate::throttle::{ThrottleDecision, ThrottlePolicy, Window, counted, decide};

/// The outcome of counting one attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Throttle {
    Allowed,
    Throttled {
        retry_after_ms: u64,
    },
    /// The map is full of live windows and this key is new: refused, nothing evicted.
    Saturated,
}

pub struct KeyedWindows<K> {
    policy: ThrottlePolicy,
    maximum_keys: usize,
    windows: HashMap<K, Window>,
    saturated_refusals: u64,
}

impl<K: Eq + Hash> KeyedWindows<K> {
    pub fn new(policy: ThrottlePolicy, maximum_keys: usize) -> Self {
        Self {
            policy,
            maximum_keys,
            windows: HashMap::new(),
            saturated_refusals: 0,
        }
    }

    /// Counts one attempt for `key` at `now_ms` and says whether it may proceed. A throttled
    /// attempt is not counted again.
    pub fn attempt(&mut self, key: K, now_ms: u64) -> Throttle {
        if let Some(window) = self.windows.get(&key).copied() {
            return match decide(self.policy, Some(window), now_ms) {
                ThrottleDecision::Allowed => {
                    self.windows
                        .insert(key, counted(self.policy, Some(window), now_ms));
                    Throttle::Allowed
                }
                ThrottleDecision::Throttled { retry_after_ms } => {
                    Throttle::Throttled { retry_after_ms }
                }
            };
        }
        if self.windows.len() >= self.maximum_keys {
            self.sweep_elapsed(now_ms);
            if self.windows.len() >= self.maximum_keys {
                self.saturated_refusals = self.saturated_refusals.saturating_add(1);
                return Throttle::Saturated;
            }
        }
        self.windows.insert(key, counted(self.policy, None, now_ms));
        Throttle::Allowed
    }

    /// Whether `key` would be throttled now, without counting an attempt.
    pub fn is_throttled(&self, key: &K, now_ms: u64) -> bool {
        matches!(
            decide(self.policy, self.windows.get(key).copied(), now_ms),
            ThrottleDecision::Throttled { .. }
        )
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// How many attempts were refused because the map was full (for `dev campaigns`).
    pub fn saturated_refusals(&self) -> u64 {
        self.saturated_refusals
    }

    fn sweep_elapsed(&mut self, now_ms: u64) {
        let window_ms = u64::try_from(self.policy.window.as_millis()).unwrap_or(u64::MAX);
        self.windows
            .retain(|_, window| now_ms.saturating_sub(window.started_at_ms) < window_ms);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const POLICY: ThrottlePolicy = ThrottlePolicy {
        limit: 2,
        window: Duration::from_secs(60),
    };

    #[test]
    fn a_key_is_allowed_up_to_its_limit_then_throttled_until_the_window_elapses() {
        let mut windows = KeyedWindows::new(POLICY, 4);
        assert_eq!(windows.attempt("a", 0), Throttle::Allowed);
        assert_eq!(windows.attempt("a", 10), Throttle::Allowed);
        assert_eq!(
            windows.attempt("a", 20),
            Throttle::Throttled {
                retry_after_ms: 59_980
            }
        );
        assert_eq!(windows.attempt("a", 60_000), Throttle::Allowed);
    }

    #[test]
    fn a_full_map_refuses_new_keys_and_keeps_live_windows() {
        let mut windows = KeyedWindows::new(POLICY, 2);
        assert_eq!(windows.attempt("a", 0), Throttle::Allowed);
        assert_eq!(windows.attempt("a", 1), Throttle::Allowed);
        assert_eq!(windows.attempt("b", 2), Throttle::Allowed);
        assert_eq!(windows.attempt("c", 3), Throttle::Saturated);
        assert_eq!(windows.attempt("d", 4), Throttle::Saturated);
        assert_eq!(windows.saturated_refusals(), 2);
        assert_eq!(windows.len(), 2);
        // The live window of "a" was not evicted, so its counter did not reset.
        assert_eq!(
            windows.attempt("a", 5),
            Throttle::Throttled {
                retry_after_ms: 59_995
            }
        );
    }

    #[test]
    fn a_full_map_sweeps_elapsed_windows_before_refusing() {
        let mut windows = KeyedWindows::new(POLICY, 2);
        assert_eq!(windows.attempt("a", 0), Throttle::Allowed);
        assert_eq!(windows.attempt("b", 0), Throttle::Allowed);
        assert_eq!(windows.attempt("c", 61_000), Throttle::Allowed);
        assert_eq!(windows.len(), 1);
    }

    #[test]
    fn flooding_one_layer_does_not_lock_out_another() {
        let mut per_ip = KeyedWindows::new(POLICY, 2);
        let mut per_account = KeyedWindows::new(POLICY, 2);
        assert_eq!(per_ip.attempt("x", 0), Throttle::Allowed);
        assert_eq!(per_ip.attempt("y", 0), Throttle::Allowed);
        assert_eq!(per_ip.attempt("z", 0), Throttle::Saturated);
        assert_eq!(per_account.attempt("alice", 0), Throttle::Allowed);
    }

    #[test]
    fn peeking_does_not_count_an_attempt() {
        let mut windows = KeyedWindows::new(POLICY, 2);
        assert_eq!(windows.attempt("a", 0), Throttle::Allowed);
        assert!(!windows.is_throttled(&"a", 1));
        assert_eq!(windows.attempt("a", 2), Throttle::Allowed);
        assert!(windows.is_throttled(&"a", 3));
    }
}
