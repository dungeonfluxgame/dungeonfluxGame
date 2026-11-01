//! Bounded watch waits. A caller may wait for a revision change up to a fixed cap; each poll
//! reauthorizes through the normal read path. Waiters are capped per owner and in total, derived
//! from scheduler capacity, so one owner or one burst of players cannot exhaust the database.
//! A caller over a cap gets a retryable `Unavailable` before any read.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::SchedulerBounds;
use crate::request::ImageOwner;

use super::types::ImageServiceError;

/// The longest any single watch call may hold a connection open.
pub const WATCH_MAX_WAIT: Duration = Duration::from_secs(30);

/// How often a waiting call rereads its demand while it waits.
pub const WATCH_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Waiter slots per unit of scheduler capacity, before the floors below apply.
const WAITERS_PER_CAPACITY_SLOT: usize = 8;
const MINIMUM_TOTAL_WAITERS: usize = 16;
const MINIMUM_OWNER_WAITERS: usize = 2;

#[derive(Default)]
struct WaiterState {
    total: usize,
    by_owner: HashMap<ImageOwner, usize>,
}

/// Counts active waiters against the caps; a permit releases its slot when dropped.
pub(crate) struct WatchWaiters {
    total_cap: usize,
    owner_cap: usize,
    state: Mutex<WaiterState>,
}

impl WatchWaiters {
    pub(crate) fn new(scheduler: &SchedulerBounds) -> Self {
        let slots = (scheduler.foreground_limit as usize)
            .saturating_add(scheduler.presence_limit as usize)
            .saturating_add(scheduler.background_limit as usize);
        Self {
            total_cap: slots
                .saturating_mul(WAITERS_PER_CAPACITY_SLOT)
                .max(MINIMUM_TOTAL_WAITERS),
            owner_cap: slots.max(MINIMUM_OWNER_WAITERS),
            state: Mutex::new(WaiterState::default()),
        }
    }

    /// Takes a slot for `owner`, or refuses without waiting when either cap is full.
    pub(crate) fn enter(
        self: &Arc<Self>,
        owner: &ImageOwner,
    ) -> Result<WaitPermit, ImageServiceError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ImageServiceError::Unavailable)?;
        let owned = state.by_owner.get(owner).copied().unwrap_or(0);
        if state.total >= self.total_cap || owned >= self.owner_cap {
            return Err(ImageServiceError::Unavailable);
        }
        state.total += 1;
        *state.by_owner.entry(owner.clone()).or_insert(0) += 1;
        Ok(WaitPermit {
            waiters: Arc::clone(self),
            owner: owner.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn active(&self) -> usize {
        self.state.lock().map(|state| state.total).unwrap_or(0)
    }
}

/// One active waiter. Dropping it frees the slot, including on cancellation or panic.
pub(crate) struct WaitPermit {
    waiters: Arc<WatchWaiters>,
    owner: ImageOwner,
}

impl Drop for WaitPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.waiters.state.lock() {
            state.total = state.total.saturating_sub(1);
            if let Some(owned) = state.by_owner.get_mut(&self.owner) {
                *owned = owned.saturating_sub(1);
                if *owned == 0 {
                    state.by_owner.remove(&self.owner);
                }
            }
        }
    }
}

/// The wait a caller may actually get: never more than the cap.
pub(crate) fn bounded_wait(requested: Duration) -> Duration {
    requested.min(WATCH_MAX_WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ImageConfig;
    use df_types::{AccountId, CampaignId, RunId};

    fn draft(byte: u8) -> ImageOwner {
        let mut bytes = [0_u8; 16];
        bytes[15] = byte;
        ImageOwner::Draft {
            account: AccountId::from_bytes(&bytes).unwrap(),
        }
    }

    fn run() -> ImageOwner {
        ImageOwner::Run {
            campaign: CampaignId::from_bytes(&[1; 16]).unwrap(),
            run: RunId::from_bytes(&[2; 16]).unwrap(),
        }
    }

    #[test]
    fn waits_are_capped_at_thirty_seconds() {
        assert_eq!(bounded_wait(Duration::ZERO), Duration::ZERO);
        assert_eq!(bounded_wait(Duration::from_secs(5)), Duration::from_secs(5));
        assert_eq!(bounded_wait(Duration::from_secs(3_600)), WATCH_MAX_WAIT);
    }

    #[test]
    fn owner_cap_refuses_one_owner_without_starving_another() {
        let scheduler = ImageConfig::defaults().unwrap().scheduler;
        let waiters = Arc::new(WatchWaiters::new(&scheduler));
        let mut held = Vec::new();
        for _ in 0..waiters.owner_cap {
            held.push(waiters.enter(&draft(1)).unwrap());
        }
        assert!(matches!(
            waiters.enter(&draft(1)),
            Err(ImageServiceError::Unavailable)
        ));
        held.push(waiters.enter(&run()).unwrap());
        assert_eq!(waiters.active(), held.len());
    }

    #[test]
    fn total_cap_refuses_new_owners_and_dropping_a_permit_frees_its_slot() {
        let scheduler = ImageConfig::defaults().unwrap().scheduler;
        let waiters = Arc::new(WatchWaiters::new(&scheduler));
        let mut held = Vec::new();
        let mut index = 0_u8;
        while held.len() < waiters.total_cap {
            index = index.wrapping_add(1);
            // One waiter per owner keeps the owner cap out of the way.
            held.push(waiters.enter(&draft(index)).unwrap());
        }
        assert!(matches!(
            waiters.enter(&draft(200)),
            Err(ImageServiceError::Unavailable)
        ));
        held.pop();
        assert!(waiters.enter(&draft(200)).is_ok());
    }
}
