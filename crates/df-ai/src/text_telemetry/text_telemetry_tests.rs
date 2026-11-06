use super::*;
use crate::text::{TextProfile, TextPurpose, TextRequestError, UnavailableReason};
use crate::text_attempts::AttemptStoreError;
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
