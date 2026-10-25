//! The image quota breaker. It lives in the engine, not the provider crate, because its state is
//! per engine (one per provider route) and its audit lines need the imagery context.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use super::ImageFailure;

/// Where the breaker stands. `Probing` means one call holds the probe; no other call reaches the
/// provider until that call settles it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum QuotaState {
    Closed,
    /// Calls fail fast until `until`; from then on one call may probe.
    Open {
        until: Instant,
    },
    Probing,
}

/// The decision a call takes once, before its retry loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    /// Closed: the call runs without a probe.
    Closed,
    /// The cooldown has passed: the call may claim the probe once it holds its permit.
    MayProbe,
    /// Open within its cooldown, or another call holds the probe: fail fast.
    Refused,
}

/// What a provider result says about the account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum QuotaAnswer {
    /// The provider accepted the work, so the account has quota.
    Succeeded,
    /// A quota answer: the account has no quota left.
    Exhausted,
    /// Anything else (rate limit, transport, output). It says nothing about the account.
    Inconclusive,
}

/// A state change the caller audits. A closed breaker that opens reports `Opened`; an open or probing
/// breaker that closes reports `Recovered`. Re-failing an open breaker reports nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Transition {
    Opened,
    Recovered,
}

/// The outcome of claiming the probe, which happens only once a call holds its permit.
pub(super) enum Claim<'breaker> {
    Claimed(ProbeClaim<'breaker>),
    /// The breaker closed while the call waited for its permit: no probe is needed.
    NotNeeded,
    /// Another call holds the probe, or the cooldown has not passed: fail fast.
    Busy,
}

/// The single probe. Settling it with an answer records that answer. Dropping it unanswered
/// (cancellation, or a failure before the provider answered) re-arms the breaker so the next call
/// may probe.
pub(super) struct ProbeClaim<'breaker> {
    breaker: &'breaker QuotaBreaker,
    settled: bool,
}

impl ProbeClaim<'_> {
    pub(super) fn settle(mut self, answer: QuotaAnswer, now: Instant) -> Option<Transition> {
        self.settled = true;
        self.breaker.apply(answer, now, true)
    }
}

impl Drop for ProbeClaim<'_> {
    fn drop(&mut self) {
        if !self.settled {
            self.breaker
                .apply(QuotaAnswer::Inconclusive, Instant::now(), true);
        }
    }
}

pub(super) struct QuotaBreaker {
    cooldown: Duration,
    state: Mutex<QuotaState>,
}

impl QuotaBreaker {
    pub(super) fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            state: Mutex::new(QuotaState::Closed),
        }
    }

    /// The decision for a call, taken once before it waits for a permit. Changes nothing.
    pub(super) fn admit(&self, now: Instant) -> Admission {
        match *super::locked(&self.state) {
            QuotaState::Closed => Admission::Closed,
            QuotaState::Open { until } if now < until => Admission::Refused,
            QuotaState::Open { .. } => Admission::MayProbe,
            QuotaState::Probing => Admission::Refused,
        }
    }

    /// Revalidates admission after the call holds its permit.
    pub(super) fn claim_admission(&self, admission: Admission, now: Instant) -> Claim<'_> {
        let mut state = super::locked(&self.state);
        match *state {
            QuotaState::Closed if admission == Admission::Closed => Claim::NotNeeded,
            QuotaState::Closed => Claim::Busy,
            QuotaState::Open { until } if admission == Admission::MayProbe && now >= until => {
                *state = QuotaState::Probing;
                Claim::Claimed(ProbeClaim {
                    breaker: self,
                    settled: false,
                })
            }
            QuotaState::Open { .. } | QuotaState::Probing => Claim::Busy,
        }
    }

    /// Records an answer from a call that does not hold the probe. Only success and quota answers
    /// count; an inconclusive answer never moves a call that is not the probe.
    pub(super) fn observe(&self, answer: QuotaAnswer, now: Instant) -> Option<Transition> {
        self.apply(answer, now, false)
    }

    fn apply(&self, answer: QuotaAnswer, now: Instant, from_probe: bool) -> Option<Transition> {
        let mut state = super::locked(&self.state);
        match answer {
            QuotaAnswer::Succeeded => match *state {
                QuotaState::Closed => None,
                QuotaState::Open { .. } | QuotaState::Probing => {
                    *state = QuotaState::Closed;
                    Some(Transition::Recovered)
                }
            },
            QuotaAnswer::Exhausted => {
                let opening = *state == QuotaState::Closed;
                *state = QuotaState::Open {
                    until: now + self.cooldown,
                };
                opening.then_some(Transition::Opened)
            }
            QuotaAnswer::Inconclusive => {
                if from_probe && *state == QuotaState::Probing {
                    *state = QuotaState::Open {
                        until: now + self.cooldown,
                    };
                }
                None
            }
        }
    }

    #[cfg(test)]
    pub(super) fn state(&self) -> QuotaState {
        *super::locked(&self.state)
    }

    /// Ends the cooldown now, so the next call may probe.
    #[cfg(test)]
    pub(super) fn expire_for_test(&self) {
        let mut state = super::locked(&self.state);
        if let QuotaState::Open { .. } = *state {
            *state = QuotaState::Open {
                until: Instant::now(),
            };
        }
    }
}

/// What a provider result says about the account. Only success and a quota answer do.
pub(super) fn quota_answer_of<T>(result: &Result<T, ImageFailure>) -> QuotaAnswer {
    match result {
        Ok(_) => QuotaAnswer::Succeeded,
        Err(ImageFailure::QuotaExhausted) => QuotaAnswer::Exhausted,
        Err(_) => QuotaAnswer::Inconclusive,
    }
}
