//! The broker's events through the host's event sink: the same change as a line and as a record.
//!
//! `BrokerEvent` writes its `RESOURCE` or `SPEND` line in `broker/events.rs`; this file writes the
//! same event's attributes from the same value with an exhaustive `match`, so an event added to
//! the broker without its attributes does not compile and the line and the record cannot
//! disagree. `SinkBrokerAudit` is the broker's audit port wired to the sink, which replaces
//! `LogBrokerAudit` where the host wants the broker in the structured stream too: a request
//! carries the operation it was made for (`df.operation`), so `trace` shows the broker's side of
//! one input, and every charge is a record the telemetry path keeps. A request has an operation
//! only when it is made, so the later events of one request join it by `handle`.

use std::sync::Arc;

use crate::broker::purpose::Price;
use crate::broker::{BrokerAudit, BrokerEvent};
use crate::observe::{AttributeValue, HostEventSink, RecordClass, RecordDraft, RecordSeverity};

use super::audit_fields::{text, whole};

/// What one broker event contributes to its record, beyond its line.
struct Parts {
    name: &'static str,
    kind: &'static str,
    severity: RecordSeverity,
    operation: Option<String>,
    fields: Vec<(&'static str, AttributeValue)>,
}

impl Parts {
    fn new(name: &'static str, kind: &'static str) -> Self {
        Self {
            name,
            kind,
            severity: RecordSeverity::Info,
            operation: None,
            fields: Vec::new(),
        }
    }

    fn field(mut self, name: &'static str, value: AttributeValue) -> Self {
        self.fields.push((name, value));
        self
    }

    fn warn(mut self) -> Self {
        self.severity = RecordSeverity::Warn;
        self
    }
}

fn parts_of(event: &BrokerEvent) -> Parts {
    match event {
        BrokerEvent::Requested {
            hub: _,
            handle,
            purpose,
            class,
            epoch,
            key,
            operation,
            depth,
        } => {
            let mut parts = Parts::new("broker.requested", "request")
                .field("purpose", text(&purpose.to_string()))
                .field("class", text(class.key()))
                .field("handle", whole(handle.get()))
                .field("epoch", whole(epoch.get()))
                .field("key", text(&format!("{:016x}", key.get())))
                .field("depth", whole(*depth));
            parts.operation = Some(operation.to_string());
            parts
        }
        BrokerEvent::Joined {
            hub: _,
            handle,
            purpose,
            holders,
        } => Parts::new("broker.joined", "joined")
            .field("purpose", text(&purpose.to_string()))
            .field("handle", whole(handle.get()))
            .field("holders", whole(*holders)),
        BrokerEvent::Refused {
            hub: _,
            purpose,
            kind,
        } => Parts::new("broker.refused", "refused")
            .warn()
            .field("purpose", text(&purpose.to_string()))
            .field("cause", text(kind.key())),
        BrokerEvent::Claimed {
            hub: _,
            handle,
            purpose,
            attempt,
            worker,
            number,
            waited_ms,
        } => Parts::new("broker.claimed", "claimed")
            .field("purpose", text(&purpose.to_string()))
            .field("handle", whole(handle.get()))
            .field("attempt", whole(attempt.get()))
            .field("worker", whole(worker.get()))
            .field("number", whole(*number))
            .field("waited_ms", whole(*waited_ms)),
        BrokerEvent::Spent {
            hub: _,
            handle,
            purpose,
            attempt,
            price,
        } => {
            let parts = Parts::new("broker.spent", "commit")
                .field("purpose", text(&purpose.to_string()))
                .field("handle", whole(handle.get()))
                .field("attempt", whole(attempt.get()));
            match price {
                Price::Priced { micro_dollars } => {
                    parts.field("micro_dollars", whole(*micro_dollars))
                }
                Price::Unpriced => parts.field("price", text("unpriced")),
            }
        }
        BrokerEvent::Retrying {
            hub: _,
            handle,
            purpose,
            retry,
            backoff_ms,
            cause,
        } => Parts::new("broker.retrying", "retry")
            .warn()
            .field("purpose", text(&purpose.to_string()))
            .field("handle", whole(handle.get()))
            .field("retry", whole(*retry))
            .field("backoff_ms", whole(*backoff_ms))
            .field("cause", text(cause.key())),
        BrokerEvent::Ready {
            hub: _,
            handle,
            purpose,
            ms,
        } => Parts::new("broker.ready", "ready")
            .field("purpose", text(&purpose.to_string()))
            .field("handle", whole(handle.get()))
            .field("ms", whole(*ms)),
        BrokerEvent::Failed {
            hub: _,
            handle,
            purpose,
            cause,
            ms,
            unknown_dispatch,
        } => {
            let mut parts = Parts::new("broker.failed", "failed")
                .field("purpose", text(&purpose.to_string()))
                .field("handle", whole(handle.get()))
                .field("cause", text(cause.key()))
                .field("ms", whole(*ms))
                .field("unknown_dispatch", AttributeValue::Flag(*unknown_dispatch));
            parts.severity = RecordSeverity::Error;
            parts
        }
        BrokerEvent::Cancelled {
            hub: _,
            handle,
            purpose,
        } => Parts::new("broker.cancelled", "cancelled")
            .field("purpose", text(&purpose.to_string()))
            .field("handle", whole(handle.get())),
        BrokerEvent::Preempted {
            hub: _,
            handle,
            purpose,
            by,
        } => Parts::new("broker.preempted", "preempted")
            .warn()
            .field("purpose", text(&purpose.to_string()))
            .field("handle", whole(handle.get()))
            .field("by", whole(by.get())),
        BrokerEvent::Dropped {
            hub: _,
            handle,
            purpose,
            class,
        } => Parts::new("broker.dropped", "dropped")
            .warn()
            .field("purpose", text(&purpose.to_string()))
            .field("handle", whole(handle.get()))
            .field("class", text(class.key())),
        BrokerEvent::Stale {
            hub: _,
            handle,
            purpose,
            epoch,
            current,
            reason,
        } => {
            let parts = Parts::new("broker.stale", "stale")
                .warn()
                .field("purpose", text(&purpose.to_string()))
                .field("epoch", whole(epoch.get()))
                .field("current", whole(current.get()))
                .field("reason", text(reason.key()));
            match handle {
                Some(handle) => parts.field("handle", whole(handle.get())),
                None => parts,
            }
        }
        BrokerEvent::Abandoned {
            hub: _,
            handle,
            attempt,
            reason,
        } => Parts::new("broker.abandoned", "abandoned")
            .warn()
            .field("handle", whole(handle.get()))
            .field("attempt", whole(attempt.get()))
            .field("reason", text(reason.key())),
        BrokerEvent::LeaseExpired {
            hub: _,
            handle,
            attempt,
            sent,
            requeued,
        } => Parts::new("broker.lease_expired", "lease_expired")
            .warn()
            .field("handle", whole(handle.get()))
            .field("attempt", whole(attempt.get()))
            .field("sent", AttributeValue::Flag(*sent))
            .field("requeued", AttributeValue::Flag(*requeued)),
        BrokerEvent::EpochAdvanced {
            hub: _,
            from,
            to,
            superseded,
        } => Parts::new("broker.epoch_advanced", "epoch_advanced")
            .field("from", whole(from.get()))
            .field("to", whole(to.get()))
            .field("superseded", whole(*superseded)),
        BrokerEvent::Closed { hub: _, cancelled } => {
            Parts::new("broker.closed", "closed").field("cancelled", whole(*cancelled))
        }
    }
}

/// The record of one broker event: always class A (a request, a charge or a failure is never
/// sampled), with the line the broker rendered.
pub fn broker_draft(event: &BrokerEvent) -> RecordDraft {
    let parts = parts_of(event);
    let mut draft = RecordDraft::new(
        RecordClass::Always,
        parts.severity,
        event.area(),
        parts.name,
        parts.kind,
        event.render_audit_line(),
    );
    if let Some(operation) = parts.operation {
        draft = draft.about_operation(operation);
    }
    draft = draft.with_field("hub", text(event.hub().key()));
    for (name, value) in parts.fields {
        draft = draft.with_field(name, value);
    }
    draft
}

/// The broker's audit port, wired to the host's event sink.
pub struct SinkBrokerAudit {
    sink: Arc<HostEventSink>,
}

impl SinkBrokerAudit {
    pub fn new(sink: Arc<HostEventSink>) -> Self {
        Self { sink }
    }
}

impl BrokerAudit for SinkBrokerAudit {
    fn record(&self, event: &BrokerEvent) {
        self.sink.record_draft(broker_draft(event));
    }
}
