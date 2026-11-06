//! Text service events on the shared diagnostic path, shaped by the generation logging
//! contract (planning/generation-logging.md): schema `df.generation/1`, `generation.*` event names
//! and closed outcome, reason and send-state labels. Each text call emits one record through the
//! process's shared sink. Prompts, answers, credentials and content hashes never appear.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use crate::text::{TextBackend, TextEvent, TextEventKind, TextObserver, TextService};
use df_observe::{AnyValue, DiagnosticRecord, DiagnosticSink, LogInput, Severity};

const SCHEMA: &str = "df.generation/1";
const EVENT_VERSION: i64 = 1;

/// Forwards each text event to the shared diagnostic sink. A failed emit never changes the
/// caller's outcome; failures are counted and visible through `failures`.
pub struct TextTelemetry {
    sink: Arc<dyn DiagnosticSink>,
    failures: AtomicU64,
}

impl TextTelemetry {
    /// An observer that emits through `sink` with no failures counted yet.
    pub fn new(sink: Arc<dyn DiagnosticSink>) -> Self {
        Self {
            sink,
            failures: AtomicU64::new(0),
        }
    }

    /// How many records the sink refused. A refusal never changes the caller's outcome.
    pub fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }
}

impl TextObserver for TextTelemetry {
    fn observe(&self, event: &TextEvent) {
        let now = SystemTime::now();
        let elapsed_ns = i64::try_from(event.elapsed.as_nanos()).unwrap_or(i64::MAX);
        let mut attributes = vec![
            ("schema".into(), text(SCHEMA)),
            ("event_version".into(), AnyValue::Int(EVENT_VERSION)),
            ("operation_class".into(), text("text")),
            ("purpose".into(), text(event.purpose.key())),
            ("profile".into(), text(event.profile.as_str())),
            ("output".into(), text(&event.output_name)),
            ("outcome".into(), text(event.kind.key())),
            ("duration_ns".into(), AnyValue::Int(elapsed_ns)),
            ("clock_domain".into(), text("process_monotonic")),
            ("measurement_state".into(), text("measured")),
        ];
        match (&event.scope, &event.operation) {
            (Some(scope), Some(operation)) => {
                attributes.push(("scope_ref".into(), text(scope)));
                attributes.push(("operation_id".into(), text(operation)));
            }
            _ => attributes.push(("operation_id".into(), text("not_applicable"))),
        }
        if let Some(reason) = event.kind.reason() {
            attributes.push(("reason".into(), text(reason)));
        }
        if let Some(state) = send_state(event.kind) {
            attributes.push(("send_state".into(), text(state)));
        }
        let record = DiagnosticRecord {
            event_name: event_name(event.kind),
            input: LogInput {
                timestamp: now,
                observed_timestamp: now,
                severity: severity(event.kind),
                body: text(event.kind.key()),
                attributes,
                trace: None,
            },
        };
        if self.sink.emit(record).is_err() {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The text service wired to the shared diagnostic path.
pub fn logged_service<B: TextBackend>(
    backend: B,
    sink: Arc<dyn DiagnosticSink>,
) -> TextService<B, Arc<TextTelemetry>> {
    TextService::new(backend).with_observer(Arc::new(TextTelemetry::new(sink)))
}

/// The contract's event that owns each kind of fact: admission, reuse, attempt results, recovery,
/// qualification and cancellation or staleness.
fn event_name(kind: TextEventKind) -> &'static str {
    match kind {
        TextEventKind::Refused(_)
        | TextEventKind::Conflict
        | TextEventKind::Unauthorized
        | TextEventKind::StoreFailed(_) => "generation.request",
        TextEventKind::Replayed => "generation.cache",
        TextEventKind::InFlight => "generation.coalesced",
        TextEventKind::Answered
        | TextEventKind::Invalid
        | TextEventKind::Unavailable(_)
        | TextEventKind::Deadline => "generation.provider_result",
        TextEventKind::Unknown => "generation.recovery",
        TextEventKind::Qualified
        | TextEventKind::Rejected
        | TextEventKind::Invalidated
        | TextEventKind::NotCandidate => "generation.text_qualification",
        TextEventKind::Cancelled => "generation.cancel",
        TextEventKind::Stale => "generation.stale",
    }
}

/// INFO is the default; WARN is a handled degradation or uncertainty; ERROR is failed owned work.
/// A refusal or a cancellation is not an error by itself.
fn severity(kind: TextEventKind) -> Severity {
    match kind {
        TextEventKind::Answered
        | TextEventKind::Replayed
        | TextEventKind::Cancelled
        | TextEventKind::Qualified
        | TextEventKind::Invalidated => Severity::Info,
        TextEventKind::Refused(_)
        | TextEventKind::Invalid
        | TextEventKind::Conflict
        | TextEventKind::InFlight
        | TextEventKind::Unauthorized
        | TextEventKind::Stale
        | TextEventKind::Unknown
        | TextEventKind::Rejected
        | TextEventKind::NotCandidate => Severity::Warn,
        TextEventKind::Unavailable(_) | TextEventKind::Deadline | TextEventKind::StoreFailed(_) => {
            Severity::Error
        }
    }
}

/// Whether a request left the process, for the events where that is a fact of the call.
fn send_state(kind: TextEventKind) -> Option<&'static str> {
    match kind {
        TextEventKind::Answered | TextEventKind::Invalid | TextEventKind::Stale => {
            Some("known_sent")
        }
        TextEventKind::Deadline | TextEventKind::Unknown => Some("possibly_sent"),
        TextEventKind::Unavailable(_)
        | TextEventKind::Refused(_)
        | TextEventKind::Conflict
        | TextEventKind::Unauthorized
        | TextEventKind::StoreFailed(_) => Some("unsent"),
        TextEventKind::Replayed
        | TextEventKind::InFlight
        | TextEventKind::Cancelled
        | TextEventKind::Qualified
        | TextEventKind::Rejected
        | TextEventKind::Invalidated
        | TextEventKind::NotCandidate => None,
    }
}

fn text(value: &str) -> AnyValue {
    AnyValue::String(value.to_owned().into())
}

#[cfg(test)]
mod text_telemetry_tests;
