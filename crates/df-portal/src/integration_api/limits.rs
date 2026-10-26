//! Bounded token and account windows. In-flight permits live through the response projection.
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
const MAXIMUM_WINDOWS: usize = 4096;

#[derive(Clone, Copy)]
pub struct RatePolicy {
    token_per_minute: u32,
    account_per_minute: u32,
    concurrency: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidRatePolicy;
impl RatePolicy {
    pub fn new(
        token_per_minute: u32,
        account_per_minute: u32,
        concurrency: usize,
    ) -> Result<Self, InvalidRatePolicy> {
        if token_per_minute == 0
            || account_per_minute < token_per_minute
            || account_per_minute > 60_000
            || !(1..=100).contains(&concurrency)
        {
            return Err(InvalidRatePolicy);
        }
        Ok(Self {
            token_per_minute,
            account_per_minute,
            concurrency,
        })
    }
}
impl Default for RatePolicy {
    fn default() -> Self {
        Self {
            token_per_minute: 120,
            account_per_minute: 600,
            concurrency: 10,
        }
    }
}
struct TokenWindow {
    window: (u64, u32),
    semaphore: Arc<Semaphore>,
}
pub(super) struct Limits {
    policy: RatePolicy,
    global: (u64, u32),
    accounts: BTreeMap<[u8; 16], (u64, u32)>,
    tokens: BTreeMap<[u8; 16], TokenWindow>,
}
impl Default for Limits {
    fn default() -> Self {
        Self::new(RatePolicy::default())
    }
}
fn count(window: &mut (u64, u32), now: u64, maximum: u32) -> bool {
    if now.saturating_sub(window.0) >= 60_000 {
        *window = (now, 0);
    }
    if window.1 >= maximum {
        return false;
    }
    window.1 += 1;
    true
}
impl Limits {
    pub(super) fn new(policy: RatePolicy) -> Self {
        Self {
            policy,
            global: (0, 0),
            accounts: BTreeMap::new(),
            tokens: BTreeMap::new(),
        }
    }
    pub(super) fn global(&mut self, now: u64) -> bool {
        count(&mut self.global, now, 1200)
    }
    pub(super) fn token(
        &mut self,
        token: [u8; 16],
        account: [u8; 16],
        now: u64,
    ) -> Option<OwnedSemaphorePermit> {
        self.accounts
            .retain(|_, w| now.saturating_sub(w.0) < 60_000);
        self.tokens.retain(|_, w| {
            now.saturating_sub(w.window.0) < 60_000 || Arc::strong_count(&w.semaphore) > 1
        });
        if (self.accounts.len() >= MAXIMUM_WINDOWS && !self.accounts.contains_key(&account))
            || (self.tokens.len() >= MAXIMUM_WINDOWS && !self.tokens.contains_key(&token))
        {
            return None;
        }
        let bucket = self.tokens.entry(token).or_insert_with(|| TokenWindow {
            window: (now, 0),
            semaphore: Arc::new(Semaphore::new(self.policy.concurrency)),
        });
        let permit = Arc::clone(&bucket.semaphore).try_acquire_owned().ok()?;
        if !count(&mut bucket.window, now, self.policy.token_per_minute)
            || !count(
                self.accounts.entry(account).or_insert((now, 0)),
                now,
                self.policy.account_per_minute,
            )
        {
            return None;
        }
        Some(permit)
    }
}
