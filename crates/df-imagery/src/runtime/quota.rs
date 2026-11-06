//! Shared supplier-account quota admission (image-generation architecture, scheduling).
//!
//! The host retains one registry across workers and configuration rebuilds. Only newly
//! observed supplier outcomes settle a permit: cache hits and checkpoint recovery do not.
//! Open/probing accounts are never evicted. Closed unused entries leave in access order.

use crate::ServiceClock;
use crate::config::QuotaSettings;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

/// Host-owned shared quota state. Contains no credentials or persisted account identifiers.
pub struct QuotaRegistry {
    clock: Arc<dyn ServiceClock>,
    entries: Mutex<VecDeque<Arc<Entry>>>,
}

/// Private fingerprint; deliberately lacks Debug/Display and is never persisted or observed.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct QuotaAccount([u8; 32]);
impl QuotaAccount {
    pub(crate) fn from_credential(
        endpoint: &str,
        secret: &str,
        trusted_identity: Option<&[u8; 32]>,
    ) -> Result<Self, QuotaError> {
        crate::backends::api::check_credential(secret).map_err(|_| QuotaError::InvalidIdentity)?;
        crate::backends::api::check_endpoint_url(endpoint)
            .map_err(|_| QuotaError::InvalidIdentity)?;
        let endpoint = url::Url::parse(endpoint).map_err(|_| QuotaError::InvalidIdentity)?;
        let origin = endpoint.origin().ascii_serialization();
        let mut hash = Sha256::new();
        hash.update(b"df-imagery-quota-account-v1\0");
        hash.update(origin.as_bytes());
        hash.update([0]);
        if let Some(identity) = trusted_identity {
            // Only trusted custody supplies this identity, never a recipe or request field.
            hash.update(b"custody\0");
            hash.update(identity);
        } else {
            hash.update(b"credential\0");
            hash.update(secret.as_bytes());
        }
        Ok(Self(hash.finalize().into()))
    }
}

/// Closed quota admission and diagnostic failures; no private account facts are carried.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaError {
    Cooldown { retry_at_ms: i64 },
    Probing,
    AdmissionChanged,
    RegistryFull,
    InvalidIdentity,
    InvalidPolicy,
    InvalidTime,
    StateUnavailable,
}

impl std::fmt::Display for QuotaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cooldown { retry_at_ms } => write!(
                formatter,
                "the supplier quota cools down until {retry_at_ms} ms"
            ),
            Self::Probing => formatter.write_str("the supplier quota is being probed"),
            Self::AdmissionChanged => formatter.write_str("the quota admission changed"),
            Self::RegistryFull => formatter.write_str("the quota registry is full"),
            Self::InvalidIdentity => formatter.write_str("the quota identity is invalid"),
            Self::InvalidPolicy => formatter.write_str("the quota policy is invalid"),
            Self::InvalidTime => formatter.write_str("the quota time is invalid"),
            Self::StateUnavailable => formatter.write_str("the quota state is unavailable"),
        }
    }
}

impl std::error::Error for QuotaError {}

/// Bounded account-state counters for existing developer diagnostics; never an admission decision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QuotaSummary {
    pub closed_accounts: u32,
    pub open_accounts: u32,
    pub probing_accounts: u32,
}

/// Only actual new supplier responses yield these answers; transport/output failures are
/// inconclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuotaAnswer {
    Succeeded,
    Exhausted,
    Inconclusive,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuotaTransition {
    Opened,
    Recovered,
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Condition {
    Closed,
    Open { until_ms: i64 },
    Probing { generation: u64 },
}
struct State {
    condition: Condition,
    probe_generation: u64,
}
struct Entry {
    account: QuotaAccount,
    state: Mutex<State>,
}
#[derive(Clone, Copy)]
enum ObservedAdmission {
    Closed,
    MayProbe,
}

/// Holds the observed admission until actual capacity is owned and admission is revalidated.
pub(crate) struct QuotaAdmission {
    entry: Option<Arc<Entry>>,
    observed: ObservedAdmission,
    observed_at_ms: i64,
    settings: QuotaSettings,
    clock: Arc<dyn ServiceClock>,
}

/// Retained across safe rate-limit retries. Drop rearms an unanswered probe for a full cooldown.
pub(crate) struct QuotaPermit {
    entry: Option<Arc<Entry>>,
    probe_generation: Option<u64>,
    claimed_at_ms: i64,
    settings: QuotaSettings,
    clock: Arc<dyn ServiceClock>,
    settled: bool,
}

impl QuotaRegistry {
    /// An empty registry reading time from `clock`; the host keeps one across workers.
    pub fn new(clock: Arc<dyn ServiceClock>) -> Self {
        Self {
            clock,
            entries: Mutex::new(VecDeque::new()),
        }
    }

    /// Reads aggregate state without revealing account identities or changing cooldown/probe state.
    pub fn summary(&self) -> Result<QuotaSummary, QuotaError> {
        let entries = locked(&self.entries)?;
        let mut summary = QuotaSummary::default();
        for entry in entries.iter() {
            let count = match locked(&entry.state)?.condition {
                Condition::Closed => &mut summary.closed_accounts,
                Condition::Open { .. } => &mut summary.open_accounts,
                Condition::Probing { .. } => &mut summary.probing_accounts,
            };
            *count = count.checked_add(1).ok_or(QuotaError::StateUnavailable)?;
        }
        Ok(summary)
    }

    pub(crate) fn admit(
        &self,
        account: &QuotaAccount,
        settings: QuotaSettings,
    ) -> Result<QuotaAdmission, QuotaError> {
        if settings.cooldown_ms == 0 || settings.maximum_accounts == 0 {
            return Err(QuotaError::InvalidPolicy);
        }
        if !settings.enabled {
            return Ok(QuotaAdmission {
                entry: None,
                observed: ObservedAdmission::Closed,
                observed_at_ms: 0,
                settings,
                clock: self.clock.clone(),
            });
        }
        let now = checked_now(self.clock.as_ref())?;
        let mut entries = locked(&self.entries)?;
        let entry = if let Some(index) = entries.iter().position(|entry| entry.account == *account)
        {
            let entry = entries.remove(index).ok_or(QuotaError::StateUnavailable)?;
            entries.push_back(entry.clone());
            entry
        } else {
            let maximum = usize::try_from(settings.maximum_accounts)
                .map_err(|_| QuotaError::InvalidPolicy)?;
            while entries.len() >= maximum {
                let mut unused = None;
                for (index, entry) in entries.iter().enumerate() {
                    if Arc::strong_count(entry) == 1
                        && locked(&entry.state)?.condition == Condition::Closed
                    {
                        unused = Some(index);
                        break;
                    }
                }
                let index = unused.ok_or(QuotaError::RegistryFull)?;
                entries.remove(index).ok_or(QuotaError::StateUnavailable)?;
            }
            let entry = Arc::new(Entry {
                account: *account,
                state: Mutex::new(State {
                    condition: Condition::Closed,
                    probe_generation: 0,
                }),
            });
            entries.push_back(entry.clone());
            entry
        };
        let observed = match locked(&entry.state)?.condition {
            Condition::Closed => ObservedAdmission::Closed,
            Condition::Open { until_ms } if now >= until_ms => ObservedAdmission::MayProbe,
            Condition::Open { until_ms } => {
                return Err(QuotaError::Cooldown {
                    retry_at_ms: until_ms,
                });
            }
            Condition::Probing { .. } => return Err(QuotaError::Probing),
        };
        Ok(QuotaAdmission {
            entry: Some(entry),
            observed,
            observed_at_ms: now,
            settings,
            clock: self.clock.clone(),
        })
    }
}

impl QuotaAdmission {
    pub(crate) fn claim(self) -> Result<QuotaPermit, QuotaError> {
        let mut probe_generation = None;
        let now = if self.entry.is_some() {
            checked_now(self.clock.as_ref())?.max(self.observed_at_ms)
        } else {
            0
        };
        if let Some(entry) = &self.entry {
            let mut state = locked(&entry.state)?;
            match (state.condition, self.observed) {
                (Condition::Closed, ObservedAdmission::Closed) => {}
                (Condition::Closed, ObservedAdmission::MayProbe) => {
                    return Err(QuotaError::AdmissionChanged);
                }
                (Condition::Open { until_ms }, ObservedAdmission::MayProbe) if now >= until_ms => {
                    let generation = state
                        .probe_generation
                        .checked_add(1)
                        .ok_or(QuotaError::StateUnavailable)?;
                    state.probe_generation = generation;
                    state.condition = Condition::Probing { generation };
                    probe_generation = Some(generation);
                }
                (Condition::Open { until_ms }, _) => {
                    return Err(QuotaError::Cooldown {
                        retry_at_ms: until_ms,
                    });
                }
                (Condition::Probing { .. }, _) => return Err(QuotaError::Probing),
            }
        }
        Ok(QuotaPermit {
            entry: self.entry,
            probe_generation,
            claimed_at_ms: now,
            settings: self.settings,
            clock: self.clock,
            settled: false,
        })
    }
}

impl QuotaPermit {
    pub(crate) fn finish(
        mut self,
        answer: QuotaAnswer,
    ) -> Result<Option<QuotaTransition>, QuotaError> {
        let transition = if let Some(entry) = &self.entry {
            let now = checked_now(self.clock.as_ref())?.max(self.claimed_at_ms);
            let mut state = locked(&entry.state)?;
            match answer {
                QuotaAnswer::Succeeded => {
                    let was_open = state.condition != Condition::Closed;
                    state.condition = Condition::Closed;
                    was_open.then_some(QuotaTransition::Recovered)
                }
                QuotaAnswer::Exhausted => {
                    let was_closed = state.condition == Condition::Closed;
                    state.condition = Condition::Open {
                        until_ms: cooldown_end(now, self.settings.cooldown_ms),
                    };
                    was_closed.then_some(QuotaTransition::Opened)
                }
                QuotaAnswer::Inconclusive => {
                    rearm_probe(
                        &mut state,
                        self.probe_generation,
                        now,
                        self.settings.cooldown_ms,
                    );
                    None
                }
            }
        } else {
            None
        };
        self.settled = true;
        Ok(transition)
    }
}
impl Drop for QuotaPermit {
    fn drop(&mut self) {
        if !self.settled
            && let Some(entry) = &self.entry
            && let Some(generation) = self.probe_generation
        {
            // Poisoned state already makes all future admission fail closed; Drop cannot report
            // errors.
            if let Ok(mut state) = entry.state.lock() {
                rearm_probe(
                    &mut state,
                    Some(generation),
                    self.clock.now_ms().max(self.claimed_at_ms),
                    self.settings.cooldown_ms,
                );
            }
        }
    }
}
fn rearm_probe(state: &mut State, generation: Option<u64>, now: i64, cooldown_ms: u64) {
    if let Some(generation) = generation
        && state.condition == (Condition::Probing { generation })
    {
        state.condition = Condition::Open {
            until_ms: cooldown_end(now, cooldown_ms),
        };
    }
}
fn cooldown_end(now: i64, cooldown_ms: u64) -> i64 {
    i64::try_from(i128::from(now) + i128::from(cooldown_ms)).unwrap_or(i64::MAX)
}
fn checked_now(clock: &dyn ServiceClock) -> Result<i64, QuotaError> {
    let now = clock.now_ms();
    if now > 0 {
        Ok(now)
    } else {
        Err(QuotaError::InvalidTime)
    }
}
fn locked<T>(value: &Mutex<T>) -> Result<MutexGuard<'_, T>, QuotaError> {
    value.lock().map_err(|_| QuotaError::StateUnavailable)
}

#[cfg(test)]
mod tests;
