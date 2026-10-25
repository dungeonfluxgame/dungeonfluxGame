use std::sync::atomic::{AtomicU64, Ordering};

use df_commerce::{CreditState, SpendConsent, SpendCounter, SpendSnapshot};
use df_types::{Currency, Money, OperationId};

const MICROS_PER_DOLLAR: u128 = 1_000_000;
const UNCONSTRAINED_LIMIT_DOLLARS: u128 = 1_000_000;
const OPERATION_NAMESPACE_BYTE: u8 = 0x0D;
const JOB_COUNTER_INDEX: usize = 5;

/// A distinct operation identity per `sequence`: a fixed nonzero namespace, then the sequence as
/// eight big-endian bytes, so no two sequences collide and no sequence yields the invalid zero id.
pub fn operation(sequence: u64) -> OperationId {
    let mut bytes = [OPERATION_NAMESPACE_BYTE; 16];
    bytes[8..].copy_from_slice(&sequence.to_be_bytes());
    OperationId::from_bytes(&bytes).expect("the namespace bytes make every identity nonzero")
}

/// An exact USD amount in micro-dollars.
pub fn usd_micros(micros: u128) -> Money {
    let usd = Currency::parse("USD").expect("USD is a valid currency tag");
    Money::new(usd, micros)
}

/// An exact whole-dollar USD amount.
pub fn dollars(whole: u128) -> Money {
    usd_micros(whole * MICROS_PER_DOLLAR)
}

/// A payer with no credit at revision 0.
pub fn empty_credit_state() -> CreditState {
    CreditState {
        currency: dollars(0).currency(),
        revision: 0,
        topup_principal: dollars(0),
        deferred_credit: dollars(0),
        earned_credit: dollars(0),
        refunded_principal: dollars(0),
        refund_hold: dollars(0),
    }
}

/// A supplier-exposure snapshot at revision 0 where only the job counter is bounded.
///
/// The company's supplier caps are configuration that defaults to unlimited (LAUNCH hard rules,
/// finance design 3.2), and only the job counter is bounded in hosted play, by the quote's
/// supplier maximum. Every other scope therefore has a limit no scenario reaches. This is the one
/// place a `SpendCounter` is built: the day `limit` becomes `ExposureLimit`, only this changes.
pub fn spend_snapshot_limited_by_job(job_limit: Money) -> SpendSnapshot {
    let unconstrained = SpendCounter {
        used: dollars(0),
        limit: dollars(UNCONSTRAINED_LIMIT_DOLLARS),
    };
    let mut counters = [unconstrained; 6];
    counters[JOB_COUNTER_INDEX].limit = job_limit;
    SpendSnapshot {
        currency: job_limit.currency(),
        revision: 0,
        counters,
    }
}

/// A payer's consent at revision 1 for at most `maximum` per operation.
pub fn spend_consent(maximum: Money) -> SpendConsent {
    SpendConsent {
        revision: 1,
        maximum,
    }
}

/// A clock that moves only when a test advances it, in whole seconds.
#[derive(Debug, Default)]
pub struct ControlledClock {
    seconds: AtomicU64,
}

impl ControlledClock {
    pub fn starting_at(seconds: u64) -> Self {
        Self {
            seconds: AtomicU64::new(seconds),
        }
    }

    pub fn now_seconds(&self) -> u64 {
        self.seconds.load(Ordering::SeqCst)
    }

    pub fn advance(&self, seconds: u64) {
        self.seconds.fetch_add(seconds, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids_are_distinct_across_the_whole_sequence_range() {
        let ids = [0, 1, 255, 256, 65_536, u64::MAX].map(operation);

        for (index, id) in ids.iter().enumerate() {
            assert!(ids[index + 1..].iter().all(|other| other != id));
        }
    }

    #[test]
    fn the_controlled_clock_moves_only_when_advanced() {
        let clock = ControlledClock::starting_at(10);

        assert_eq!(clock.now_seconds(), 10);
        clock.advance(5);
        assert_eq!(clock.now_seconds(), 15);
    }
}
