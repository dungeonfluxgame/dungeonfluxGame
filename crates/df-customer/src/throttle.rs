//! The bounded per-key attempt counters behind `df_auth::throttle`.
//!
//! Keys are keyed hashes (HMAC-SHA256 under a random per-process key) of a normalized email
//! address, an account id or a client IP, so an unknown address throttles exactly like a known one
//! and no address or IP is held in memory in the clear.

use std::{collections::HashMap, net::IpAddr, sync::Mutex};

use df_auth::throttle::{ThrottleDecision, ThrottlePolicy, Window, counted, decide};
use df_types::AccountId;

use crate::tokens::{RandomUnavailable, keyed_digest, random_bytes};

/// Keys tracked at once; at the bound, expired windows go first, then the oldest.
pub const MAXIMUM_TRACKED_KEYS: usize = 65_536;

/// What a counter limits. Each has its own policy in the tunables.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ThrottleScope {
    LoginFailuresPerEmail,
    LoginPerIp,
    SignupPerIp,
    SignupPerEmail,
    ResetPerEmail,
    ResetPerIp,
    VerificationResendPerAccount,
    TokenPagePerIp,
    PasswordChangePerAccount,
    EmailChangePerAccount,
    ProfilePerAccount,
    DataPerAccount,
}

impl ThrottleScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LoginFailuresPerEmail => "login_failures_per_email",
            Self::LoginPerIp => "login_per_ip",
            Self::SignupPerIp => "signup_per_ip",
            Self::SignupPerEmail => "signup_per_email",
            Self::ResetPerEmail => "reset_per_email",
            Self::ResetPerIp => "reset_per_ip",
            Self::VerificationResendPerAccount => "verification_resend_per_account",
            Self::TokenPagePerIp => "token_page_per_ip",
            Self::PasswordChangePerAccount => "password_change_per_account",
            Self::EmailChangePerAccount => "email_change_per_account",
            Self::ProfilePerAccount => "profile_per_account",
            Self::DataPerAccount => "data_per_account",
        }
    }
}

/// The value a counter is keyed on.
#[derive(Clone, Copy, Debug)]
pub enum Subject<'a> {
    Email(&'a str),
    Ip(Option<IpAddr>),
    Account(AccountId),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Key {
    scope: ThrottleScope,
    digest: [u8; 16],
}

pub struct Throttles {
    key: [u8; 32],
    windows: Mutex<HashMap<Key, Window>>,
    in_flight: Mutex<HashMap<[u8; 16], u32>>,
}

/// One of at most `MAXIMUM_IN_FLIGHT_PER_EMAIL` sign-ins running for an address at once. A
/// failed sign-in is only counted when it ends, so without this a burst of parallel guesses would
/// all run before the first failure was counted.
pub struct InFlight<'a> {
    throttles: &'a Throttles,
    digest: [u8; 16],
}

/// Sign-in checks that may run at once for one address.
pub const MAXIMUM_IN_FLIGHT_PER_EMAIL: u32 = 2;

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut in_flight = self.throttles.in_flight();
        if let Some(count) = in_flight.get_mut(&self.digest) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                in_flight.remove(&self.digest);
            }
        }
    }
}

impl Throttles {
    pub fn new() -> Result<Self, RandomUnavailable> {
        Ok(Self {
            key: random_bytes::<32>()?,
            windows: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
        })
    }

    fn in_flight(&self) -> std::sync::MutexGuard<'_, HashMap<[u8; 16], u32>> {
        self.in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Takes one in-flight slot for `email`, or `None` when the address already has the maximum
    /// running. The map holds only entries that are running now.
    pub fn enter_sign_in(&self, email: &str) -> Option<InFlight<'_>> {
        let key = self.key(ThrottleScope::LoginFailuresPerEmail, Subject::Email(email));
        let mut in_flight = self.in_flight();
        let count = in_flight.entry(key.digest).or_insert(0);
        if *count >= MAXIMUM_IN_FLIGHT_PER_EMAIL {
            return None;
        }
        *count += 1;
        Some(InFlight {
            throttles: self,
            digest: key.digest,
        })
    }

    fn key(&self, scope: ThrottleScope, subject: Subject<'_>) -> Key {
        let material = match subject {
            Subject::Email(email) => [b"email:".as_slice(), email.as_bytes()].concat(),
            // A request without a peer address shares one counter: the deployment must pass
            // connection info for per-IP limits to be per IP.
            Subject::Ip(None) => b"ip:none".to_vec(),
            Subject::Ip(Some(ip)) => format!("ip:{ip}").into_bytes(),
            Subject::Account(account) => [b"account:".as_slice(), account.as_bytes()].concat(),
        };
        let digest = keyed_digest(&self.key, &material);
        let mut short = [0_u8; 16];
        short.copy_from_slice(&digest[..16]);
        Key {
            scope,
            digest: short,
        }
    }

    fn windows(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Window>> {
        // A poisoned map still holds valid windows; counting continues.
        self.windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether an attempt may proceed now, without counting it.
    pub fn check(
        &self,
        scope: ThrottleScope,
        subject: Subject<'_>,
        policy: ThrottlePolicy,
        now_ms: u64,
    ) -> ThrottleDecision {
        let key = self.key(scope, subject);
        decide(policy, self.windows().get(&key).copied(), now_ms)
    }

    /// Counts one attempt.
    pub fn count(
        &self,
        scope: ThrottleScope,
        subject: Subject<'_>,
        policy: ThrottlePolicy,
        now_ms: u64,
    ) {
        let key = self.key(scope, subject);
        let mut windows = self.windows();
        let next = counted(policy, windows.get(&key).copied(), now_ms);
        if !windows.contains_key(&key) && windows.len() >= MAXIMUM_TRACKED_KEYS {
            evict(&mut windows, policy, now_ms);
        }
        windows.insert(key, next);
    }

    /// Checks and, when allowed, counts in one step (for limits on every attempt).
    pub fn admit(
        &self,
        scope: ThrottleScope,
        subject: Subject<'_>,
        policy: ThrottlePolicy,
        now_ms: u64,
    ) -> ThrottleDecision {
        let decision = self.check(scope, subject, policy, now_ms);
        if decision == ThrottleDecision::Allowed {
            self.count(scope, subject, policy, now_ms);
        }
        decision
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.windows().len()
    }
}

fn evict(windows: &mut HashMap<Key, Window>, policy: ThrottlePolicy, now_ms: u64) {
    let window_ms = u64::try_from(policy.window.as_millis()).unwrap_or(u64::MAX);
    windows.retain(|_, window| now_ms.saturating_sub(window.started_at_ms) < window_ms);
    if windows.len() >= MAXIMUM_TRACKED_KEYS
        && let Some(oldest) = windows
            .iter()
            .min_by_key(|(_, window)| window.started_at_ms)
            .map(|(key, _)| *key)
    {
        windows.remove(&oldest);
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
    fn an_unknown_address_is_throttled_exactly_like_a_known_one() {
        let throttles = Throttles::new().unwrap();
        for email in ["known@example.test", "nobody@example.test"] {
            for _ in 0..2 {
                throttles.count(
                    ThrottleScope::ResetPerEmail,
                    Subject::Email(email),
                    POLICY,
                    0,
                );
            }
            assert!(matches!(
                throttles.check(
                    ThrottleScope::ResetPerEmail,
                    Subject::Email(email),
                    POLICY,
                    1
                ),
                ThrottleDecision::Throttled { .. }
            ));
        }
    }

    #[test]
    fn scopes_and_subjects_count_separately() {
        let throttles = Throttles::new().unwrap();
        let email = Subject::Email("robin@example.test");
        for _ in 0..2 {
            throttles.count(ThrottleScope::LoginFailuresPerEmail, email, POLICY, 0);
        }
        assert_eq!(
            throttles.check(ThrottleScope::ResetPerEmail, email, POLICY, 0),
            ThrottleDecision::Allowed
        );
        assert_eq!(
            throttles.check(
                ThrottleScope::LoginFailuresPerEmail,
                Subject::Email("marian@example.test"),
                POLICY,
                0
            ),
            ThrottleDecision::Allowed
        );
    }

    #[test]
    fn an_address_has_a_bounded_number_of_sign_ins_in_flight_and_frees_them_on_drop() {
        let throttles = Throttles::new().unwrap();
        let first = throttles.enter_sign_in("robin@example.test").unwrap();
        let second = throttles.enter_sign_in("robin@example.test").unwrap();
        assert!(throttles.enter_sign_in("robin@example.test").is_none());
        assert!(throttles.enter_sign_in("marian@example.test").is_some());
        drop(first);
        let third = throttles.enter_sign_in("robin@example.test");
        assert!(third.is_some());
        drop((second, third));
        assert!(throttles.in_flight().is_empty());
    }

    #[test]
    fn the_map_never_grows_past_its_bound() {
        let throttles = Throttles::new().unwrap();
        for index in 0..(MAXIMUM_TRACKED_KEYS + 10) {
            let ip = IpAddr::from([10, (index >> 16) as u8, (index >> 8) as u8, index as u8]);
            throttles.count(ThrottleScope::LoginPerIp, Subject::Ip(Some(ip)), POLICY, 0);
        }
        assert_eq!(throttles.tracked(), MAXIMUM_TRACKED_KEYS);
    }
}
