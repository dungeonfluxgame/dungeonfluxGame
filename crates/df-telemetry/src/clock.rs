//! Cloneable monotonic clock injection without owning the Store or its disk worker.

use crate::Counters;
use df_observe::TelemetryError;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

/// Advances the existing ingestion clock; this handle has no admission or I/O authority.
#[derive(Clone)]
pub struct StoreClock {
    counters: Arc<Counters>,
}

impl StoreClock {
    pub(super) fn new(counters: Arc<Counters>) -> Self {
        Self { counters }
    }

    /// Time never moves backwards, including updates from concurrent capture producers.
    pub fn set_time(&self, elapsed: Duration) -> Result<(), TelemetryError> {
        let millis =
            u64::try_from(elapsed.as_millis()).map_err(|_| TelemetryError::InvalidLimits)?;
        self.counters.clock.fetch_max(millis, Ordering::AcqRel);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloned_clock_retains_the_largest_parallel_update_without_a_store_lock() {
        let counters = Arc::new(Counters::default());
        let clock = StoreClock::new(counters.clone());
        std::thread::scope(|scope| {
            for millis in 1..=64 {
                let clock = clock.clone();
                scope.spawn(move || clock.set_time(Duration::from_millis(millis)).unwrap());
            }
        });
        clock.set_time(Duration::from_millis(1)).unwrap();
        assert_eq!(counters.clock.load(Ordering::Acquire), 64);
    }

    #[test]
    fn unrepresentable_time_keeps_the_existing_clock() {
        let counters = Arc::new(Counters::default());
        let clock = StoreClock::new(counters.clone());
        clock.set_time(Duration::from_millis(12)).unwrap();
        assert!(matches!(
            clock.set_time(Duration::MAX),
            Err(TelemetryError::InvalidLimits)
        ));
        assert_eq!(counters.clock.load(Ordering::Acquire), 12);
    }
}
