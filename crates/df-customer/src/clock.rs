//! The service's only source of "now". Tests use the fake clock; nothing else reads wall time.

use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync + 'static {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        // A clock before 1970 reads as the epoch; a clock past u64 milliseconds cannot occur.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            })
    }
}

#[cfg(any(test, feature = "test-fakes"))]
pub use fake::FakeClock;

#[cfg(any(test, feature = "test-fakes"))]
mod fake {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    /// A clock tests move by hand. Clones share one time.
    #[derive(Clone)]
    pub struct FakeClock(Arc<AtomicU64>);

    impl FakeClock {
        pub fn at(now_ms: u64) -> Self {
            Self(Arc::new(AtomicU64::new(now_ms)))
        }

        pub fn advance_ms(&self, by_ms: u64) {
            self.0.fetch_add(by_ms, Ordering::SeqCst);
        }
    }

    impl super::Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }
}
