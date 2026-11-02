//! Text service events on the shared gameplay diagnostic path, shaped by the generation logging
//! contract (planning/generation-logging.md): schema `df.generation/1`, `generation.*` event names
//! and closed outcome, reason and send-state labels. Each text call emits one record through the
//! same sink as the image service. Prompts, answers, credentials and content hashes never appear.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use df_ai::text::{TextBackend, TextEvent, TextEventKind, TextObserver, TextService};
use df_observe::{AnyValue, DiagnosticRecord, DiagnosticSink, LogInput, Severity};

const SCHEMA: &str = "df.generation/1";
const EVENT_VERSION: i64 = 1;

/// Forwards each text event to the shared diagnostic sink. A failed emit never changes the
/// caller's outcome; failures are counted and visible through `failures`.
pub(super) struct TextTelemetry {
    sink: Arc<dyn DiagnosticSink>,
    failures: AtomicU64,
}

impl TextTelemetry {
    pub(super) fn new(sink: Arc<dyn DiagnosticSink>) -> Self {
        Self {
            sink,
            failures: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    pub(super) fn failures(&self) -> u64 {
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
            ("purpose".into(), text(event.purpose.label())),
            ("profile".into(), text(event.profile.as_str())),
            ("output".into(), text(&event.output_name)),
            ("outcome".into(), text(event.kind.label())),
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
                body: text(event.kind.label()),
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
pub(super) fn text_service<B: TextBackend>(
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
mod tests {
    use super::*;
    use df_ai::text::{TextProfile, TextPurpose, TextRequestError, UnavailableReason};
    use df_ai::text_attempts::AttemptStoreError;
    use df_observe::TelemetryError;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct Recording(Mutex<Vec<DiagnosticRecord>>, bool);

    impl DiagnosticSink for Recording {
        fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
            if self.1 {
                return Err(TelemetryError::Capacity);
            }
            self.0.lock().unwrap().push(record);
            Ok(())
        }
    }

    fn event(kind: TextEventKind) -> TextEvent {
        TextEvent {
            purpose: TextPurpose::Storyteller,
            profile: TextProfile::new("profile-a").unwrap(),
            scope: Some("campaign-1".to_owned()),
            operation: Some("turn-4".to_owned()),
            output_name: "narration".to_owned(),
            kind,
            elapsed: Duration::from_millis(12),
        }
    }

    fn attribute(record: &DiagnosticRecord, name: &str) -> Option<AnyValue> {
        record
            .input
            .attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }

    fn emit(kind: TextEventKind) -> DiagnosticRecord {
        let sink = Arc::new(Recording::default());
        TextTelemetry::new(sink.clone()).observe(&event(kind));
        let mut records = sink.0.lock().unwrap();
        assert_eq!(records.len(), 1);
        records.remove(0)
    }

    #[test]
    fn every_record_carries_the_generation_envelope_and_safe_references() {
        let record = emit(TextEventKind::Answered);
        assert_eq!(record.event_name, "generation.provider_result");
        assert_eq!(attribute(&record, "schema"), Some(text("df.generation/1")));
        assert_eq!(attribute(&record, "event_version"), Some(AnyValue::Int(1)));
        assert_eq!(attribute(&record, "operation_class"), Some(text("text")));
        assert_eq!(attribute(&record, "purpose"), Some(text("storyteller")));
        assert_eq!(attribute(&record, "profile"), Some(text("profile-a")));
        assert_eq!(attribute(&record, "scope_ref"), Some(text("campaign-1")));
        assert_eq!(attribute(&record, "operation_id"), Some(text("turn-4")));
        assert_eq!(attribute(&record, "send_state"), Some(text("known_sent")));
        assert_eq!(
            attribute(&record, "measurement_state"),
            Some(text("measured"))
        );
    }

    #[test]
    fn each_kind_lands_on_the_contract_event_with_its_severity() {
        let cases = [
            (
                TextEventKind::Answered,
                "generation.provider_result",
                Severity::Info,
            ),
            (TextEventKind::Replayed, "generation.cache", Severity::Info),
            (
                TextEventKind::InFlight,
                "generation.coalesced",
                Severity::Warn,
            ),
            (
                TextEventKind::Unknown,
                "generation.recovery",
                Severity::Warn,
            ),
            (TextEventKind::Stale, "generation.stale", Severity::Warn),
            (
                TextEventKind::Cancelled,
                "generation.cancel",
                Severity::Info,
            ),
            (
                TextEventKind::Qualified,
                "generation.text_qualification",
                Severity::Info,
            ),
            (
                TextEventKind::Rejected,
                "generation.text_qualification",
                Severity::Warn,
            ),
            (
                TextEventKind::Deadline,
                "generation.provider_result",
                Severity::Error,
            ),
            (
                TextEventKind::Conflict,
                "generation.request",
                Severity::Warn,
            ),
            (
                TextEventKind::Refused(TextRequestError::EmptyInstructions),
                "generation.request",
                Severity::Warn,
            ),
            (
                TextEventKind::StoreFailed(AttemptStoreError::TimedOut),
                "generation.request",
                Severity::Error,
            ),
        ];
        for (kind, name, level) in cases {
            let record = emit(kind);
            assert_eq!(record.event_name, name, "{kind:?}");
            assert_eq!(record.input.severity, level, "{kind:?}");
        }
    }

    #[test]
    fn a_closed_reason_travels_with_the_events_that_have_one() {
        let refused = emit(TextEventKind::Unavailable(
            UnavailableReason::Authentication,
        ));
        assert_eq!(attribute(&refused, "reason"), Some(text("authentication")));
        assert_eq!(attribute(&refused, "send_state"), Some(text("unsent")));
        let store = emit(TextEventKind::StoreFailed(AttemptStoreError::Unreachable));
        assert_eq!(attribute(&store, "reason"), Some(text("unreachable")));
        assert_eq!(attribute(&emit(TextEventKind::Answered), "reason"), None);
    }

    #[test]
    fn a_lost_send_is_possibly_sent_and_never_unsent() {
        assert_eq!(
            attribute(&emit(TextEventKind::Unknown), "send_state"),
            Some(text("possibly_sent"))
        );
        assert_eq!(
            attribute(&emit(TextEventKind::Deadline), "send_state"),
            Some(text("possibly_sent"))
        );
    }

    #[test]
    fn an_event_that_is_not_about_one_operation_says_so() {
        let sink = Arc::new(Recording::default());
        let mut orphan = event(TextEventKind::Answered);
        orphan.scope = None;
        orphan.operation = None;
        TextTelemetry::new(sink.clone()).observe(&orphan);
        let records = sink.0.lock().unwrap();
        assert_eq!(
            attribute(&records[0], "operation_id"),
            Some(text("not_applicable"))
        );
        assert_eq!(attribute(&records[0], "scope_ref"), None);
    }

    #[test]
    fn a_failed_emit_is_counted_and_never_changes_the_outcome() {
        let sink = Arc::new(Recording(Mutex::default(), true));
        let telemetry = TextTelemetry::new(sink);
        telemetry.observe(&event(TextEventKind::Deadline));
        assert_eq!(telemetry.failures(), 1);
    }
}
