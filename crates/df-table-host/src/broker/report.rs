//! The read-only views of the broker: queue depths, what is running and what has been spent.
//!
//! These are the data behind the dev verbs `broker queues` and `broker ledger` (H.7: every queue
//! reports depth, oldest age and drops). A report is a copy: building it never changes the broker
//! and holds no lock past the call that made it.

use df_table::ports::PurposeName;
use serde::{Deserialize, Serialize};

use super::ledger::{LedgerEntry, PurposeTotals};
use super::purpose::QueueClass;
use super::scheduler::BrokerCore;

/// One class queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassReport {
    pub class: QueueClass,
    pub depth: usize,
    /// How long the oldest waiting request has waited.
    pub oldest_age_ms: Option<u64>,
    pub dropped: u64,
}

/// One provider call that holds a lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningReport {
    pub attempt: u64,
    pub handle: u64,
    pub purpose: PurposeName,
    pub class: QueueClass,
    pub sent: bool,
    pub lease_expires_ms: u64,
}

/// The queues and the running calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuesReport {
    pub epoch: u64,
    pub closed: bool,
    pub text_slots: usize,
    pub image_slots: usize,
    pub classes: Vec<ClassReport>,
    pub running: Vec<RunningReport>,
    pub dropped_completions: u64,
}

/// What each purpose has cost, and the newest ledger entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerReport {
    pub totals: Vec<PurposeTotalsReport>,
    pub entries: Vec<LedgerEntry>,
    pub forgotten_entries: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurposeTotalsReport {
    pub purpose: PurposeName,
    #[serde(flatten)]
    pub totals: PurposeTotals,
}

impl BrokerCore {
    pub fn queues_report(&self, now_ms: u64) -> QueuesReport {
        let classes = QueueClass::ALL
            .iter()
            .map(|class| {
                let oldest = self
                    .waiting_in(*class)
                    .next()
                    .and_then(|handle| self.requested_ms(handle));
                ClassReport {
                    class: *class,
                    depth: self.waiting_in(*class).count(),
                    oldest_age_ms: oldest.map(|requested| now_ms.saturating_sub(requested)),
                    dropped: self.dropped_in(*class),
                }
            })
            .collect();
        let running = self
            .running()
            .filter_map(|running| {
                Some(RunningReport {
                    attempt: running.attempt.get(),
                    handle: running.handle.get(),
                    purpose: self.purpose_of(running.handle)?.clone(),
                    class: running.class,
                    sent: running.sent,
                    lease_expires_ms: running.lease_expires_ms,
                })
            })
            .collect();
        QueuesReport {
            epoch: self.epoch().get(),
            closed: self.is_closed(),
            text_slots: self.limits().text_slots,
            image_slots: self.limits().image_slots,
            classes,
            running,
            dropped_completions: self.dropped_completions(),
        }
    }

    /// The totals of every purpose and up to `newest` ledger entries, newest last.
    pub fn ledger_report(&self, newest: usize) -> LedgerReport {
        let skipped = self.ledger().entries().count().saturating_sub(newest);
        LedgerReport {
            totals: self
                .ledger()
                .totals()
                .map(|(purpose, totals)| PurposeTotalsReport {
                    purpose: purpose.clone(),
                    totals: *totals,
                })
                .collect(),
            entries: self.ledger().entries().skip(skipped).cloned().collect(),
            forgotten_entries: self.ledger().forgotten_entries(),
        }
    }
}
