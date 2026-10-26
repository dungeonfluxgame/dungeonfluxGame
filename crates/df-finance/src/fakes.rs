//! Faithful fakes of the ports, for tests and local use. TODO(FIN-08): each is replaced by the
//! owner's adapter; these only answer what was configured and record what was asked.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use df_commerce::digest::sha256;
use df_commerce::finance_ids::{CampaignKey, PayerId};
use tokio_postgres::Transaction;

use crate::ports::{
    Clock, FencePort, FenceRequest, FenceVerdict, IdSource, MediaPort, MediaRequest, MediaVerdict,
    MemberPort, PortUnavailable, RefundCall, RefundProcessor,
};

/// A clock the test moves by hand.
pub struct FakeClock(AtomicI64);

impl FakeClock {
    pub fn new(now_ms: i64) -> Self {
        Self(AtomicI64::new(now_ms))
    }

    pub fn advance(&self, milliseconds: i64) {
        self.0.fetch_add(milliseconds, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A shared fake clock stays movable by the test after the service owns a handle to it.
impl Clock for std::sync::Arc<FakeClock> {
    fn now_ms(&self) -> i64 {
        self.as_ref().now_ms()
    }
}

/// The wall clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            })
    }
}

/// Ids derived from a seed and a counter: unique, reproducible, not secret.
pub struct SeededIds {
    seed: u64,
    counter: AtomicU64,
}

impl SeededIds {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            counter: AtomicU64::new(0),
        }
    }
}

impl IdSource for SeededIds {
    fn next_id(&self) -> [u8; 16] {
        let count = self.counter.fetch_add(1, Ordering::SeqCst);
        let mut input = self.seed.to_be_bytes().to_vec();
        input.extend_from_slice(&count.to_be_bytes());
        let digest = sha256(&input);
        let mut id = [0_u8; 16];
        id.copy_from_slice(&digest[..16]);
        id
    }
}

/// A fence that answers a configured verdict and records how often it was asked.
pub struct FakeFence {
    verdict: Mutex<FenceVerdict>,
    asked: AtomicU64,
}

impl FakeFence {
    pub fn new(verdict: FenceVerdict) -> Self {
        Self {
            verdict: Mutex::new(verdict),
            asked: AtomicU64::new(0),
        }
    }

    pub fn set(&self, verdict: FenceVerdict) {
        if let Ok(mut slot) = self.verdict.lock() {
            *slot = verdict;
        }
    }

    pub fn times_asked(&self) -> u64 {
        self.asked.load(Ordering::SeqCst)
    }
}

impl FencePort for FakeFence {
    async fn fence_current(
        &self,
        _transaction: &Transaction<'_>,
        _request: &FenceRequest,
    ) -> Result<FenceVerdict, PortUnavailable> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.verdict
            .lock()
            .map(|verdict| *verdict)
            .map_err(|_| PortUnavailable)
    }
}

/// A media catalog that answers a configured verdict.
pub struct FakeMedia {
    verdict: Mutex<MediaVerdict>,
    asked: Mutex<Vec<MediaRequest>>,
}

impl FakeMedia {
    pub fn new(verdict: MediaVerdict) -> Self {
        Self {
            verdict: Mutex::new(verdict),
            asked: Mutex::new(Vec::new()),
        }
    }

    pub fn asked(&self) -> Vec<MediaRequest> {
        self.asked
            .lock()
            .map(|asked| asked.clone())
            .unwrap_or_default()
    }
}

impl MediaPort for FakeMedia {
    async fn admit_generation(
        &self,
        _transaction: &Transaction<'_>,
        request: &MediaRequest,
    ) -> Result<MediaVerdict, PortUnavailable> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(*request);
        }
        self.verdict
            .lock()
            .map(|verdict| *verdict)
            .map_err(|_| PortUnavailable)
    }
}

/// How the fake processor answers refund calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeRefundMode {
    Succeed,
    /// The first call for each key times out; later calls with the same key succeed.
    TimeoutOnce,
    KnownFailure,
}

/// A fake processor refund endpoint: paid once per idempotency key, however often retried.
pub struct FakeRefundProcessor {
    mode: FakeRefundMode,
    attempts: AtomicU64,
    timed_out_keys: Mutex<std::collections::BTreeSet<String>>,
    paid_keys: Mutex<std::collections::BTreeSet<String>>,
}

impl FakeRefundProcessor {
    pub fn new(mode: FakeRefundMode) -> Self {
        Self {
            mode,
            attempts: AtomicU64::new(0),
            timed_out_keys: Mutex::new(std::collections::BTreeSet::new()),
            paid_keys: Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    pub fn attempts(&self) -> u64 {
        self.attempts.load(Ordering::SeqCst)
    }

    /// How many distinct keys were actually paid: a double refund would make this exceed 1.
    pub fn paid_count(&self) -> usize {
        self.paid_keys.lock().map(|keys| keys.len()).unwrap_or(0)
    }
}

impl RefundProcessor for FakeRefundProcessor {
    async fn refund(&self, idempotency_key: &str, _amount_micro_dollars: u64) -> RefundCall {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let first_for_key = self
            .timed_out_keys
            .lock()
            .map(|mut keys| keys.insert(idempotency_key.to_owned()))
            .unwrap_or(false);
        match self.mode {
            FakeRefundMode::KnownFailure => RefundCall::KnownFailure,
            FakeRefundMode::TimeoutOnce if first_for_key => RefundCall::Unknown,
            _ => {
                if let Ok(mut keys) = self.paid_keys.lock() {
                    keys.insert(idempotency_key.to_owned());
                }
                RefundCall::Paid {
                    processor_refund_ref: format!("fake-refund-{idempotency_key}"),
                }
            }
        }
    }
}

/// A membership directory: the payers it lists are authorized.
pub struct FakeMembers {
    authorized: Mutex<BTreeSet<PayerId>>,
}

impl FakeMembers {
    pub fn new(authorized: impl IntoIterator<Item = PayerId>) -> Self {
        Self {
            authorized: Mutex::new(authorized.into_iter().collect()),
        }
    }

    pub fn revoke(&self, payer: PayerId) {
        if let Ok(mut set) = self.authorized.lock() {
            set.remove(&payer);
        }
    }
}

impl MemberPort for FakeMembers {
    async fn authorized_payers(
        &self,
        _campaign: CampaignKey,
        payers: &[PayerId],
    ) -> Result<BTreeSet<PayerId>, PortUnavailable> {
        let set = self.authorized.lock().map_err(|_| PortUnavailable)?;
        Ok(payers
            .iter()
            .copied()
            .filter(|payer| set.contains(payer))
            .collect())
    }
}
