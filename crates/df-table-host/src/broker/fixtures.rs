//! Shared fixtures for the broker's tests: a recording audit, small purpose tables and request
//! builders. Time is always an explicit number of milliseconds, never a sleep.

use std::sync::{Arc, Mutex, PoisonError};

use df_table::ports::PurposeName;
use df_table::testkit::correlation;

use super::events::{BrokerAudit, BrokerEvent};
use super::purpose::{
    Fallback, Price, PurposeFacts, PurposeSpec, PurposeTable, QueueClass, ResourceKind,
    RetryPolicy, SizeRule,
};
use super::request::{Epoch, NewRequest, RequestKey};
use super::scheduler::{BrokerCore, BrokerLimits};

/// Keeps every event, in order.
#[derive(Default)]
pub struct RecordingAudit {
    events: Mutex<Vec<BrokerEvent>>,
}

impl RecordingAudit {
    pub fn events(&self) -> Vec<BrokerEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn lines(&self) -> Vec<String> {
        self.events()
            .iter()
            .map(BrokerEvent::render_audit_line)
            .collect()
    }

    pub fn count(&self, matches: impl Fn(&BrokerEvent) -> bool) -> usize {
        self.events().iter().filter(|event| matches(event)).count()
    }
}

impl BrokerAudit for RecordingAudit {
    fn record(&self, event: &BrokerEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
    }
}

pub const TEST_RETRY: RetryPolicy = RetryPolicy {
    maximum_retries: 2,
    initial_backoff_ms: 100,
    maximum_backoff_ms: 400,
};

pub fn facts(kind: ResourceKind, class: QueueClass, deadline_ms: u64) -> PurposeFacts {
    PurposeFacts {
        kind,
        deadline_ms,
        size: match kind {
            ResourceKind::Text => SizeRule::TextCharacters { maximum: 100 },
            ResourceKind::Image => SizeRule::ImagePixels {
                width: 64,
                height: 64,
            },
        },
        retry: TEST_RETRY,
        class,
        price: Price::Unpriced,
        fallback: Fallback::Nothing,
    }
}

/// Purposes named for what they are in the tests: a text purpose and an image purpose in each
/// class, all with a deadline of 10 seconds.
pub fn test_table() -> PurposeTable {
    let mut specs = Vec::new();
    for (kind, label) in [(ResourceKind::Text, "text"), (ResourceKind::Image, "image")] {
        for class in QueueClass::ALL {
            let name = format!("{label}_{}", class.key());
            specs.push(PurposeSpec::new(&name, facts(kind, *class, 10_000)).expect("a valid spec"));
        }
    }
    PurposeTable::new(specs).expect("unique names")
}

pub fn core_with(limits: BrokerLimits) -> (BrokerCore, Arc<RecordingAudit>) {
    core_over(test_table(), limits)
}

pub fn core_over(table: PurposeTable, limits: BrokerLimits) -> (BrokerCore, Arc<RecordingAudit>) {
    let audit = Arc::new(RecordingAudit::default());
    let core = BrokerCore::new(table, limits, audit.clone());
    (core, audit)
}

pub fn name(text: &str) -> PurposeName {
    PurposeName::new(text).expect("a valid purpose name")
}

pub fn request_for(purpose: &str, key: u64, epoch: u64) -> NewRequest {
    NewRequest {
        purpose: name(purpose),
        key: RequestKey::new(key),
        epoch: Epoch::new(epoch),
        correlation: correlation(key),
    }
}
