//! The outlet onto df-observe's shared diagnostic path: unsampled native logs through the one
//! producer to the durable ingress and the telemetry store.
//!
//! It adapts one `ObservedRecord` to df-observe's `DiagnosticRecord` (the audit line as the body,
//! the attributes as typed values, the event name, severity and time) and hands it to a
//! `NativeDiagnosticSink` that the composition owns: the producer identity, the ingress and the
//! store belong to whoever starts the process, never to this adapter. Admission is a bounded,
//! nonblocking encode on the owner's thread; durability is the ingress's, and a record counts as
//! `confirmed` only when its receipt arrives. A refusal (a full queue, a closed ingress, a record
//! the path will not take) is counted by class and leaves a source gap the store itself reports.
//!
//! Ingestion lag is the age of the oldest record still waiting for its receipt. It keeps no
//! trace: spans (H.10) are not carried by this adapter, only logs.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use df_observe::{
    AnyValue, DiagnosticRecord, DiagnosticSink, LogInput, NativeDiagnosticSink, Severity,
    TelemetryError,
};

use super::outlet::{Losses, OutletRefusal, OutletStatus, RecordOutlet};
use super::record::{AttributeValue, ObservedRecord, RecordSeverity};

/// The shared diagnostic path as an outlet.
pub struct SharedPathOutlet {
    sink: Arc<NativeDiagnosticSink>,
    /// The observed time of each record still waiting for its receipt, oldest first.
    waiting: Mutex<VecDeque<u64>>,
    losses: Losses,
}

impl SharedPathOutlet {
    pub fn new(sink: Arc<NativeDiagnosticSink>) -> Self {
        Self {
            sink,
            waiting: Mutex::new(VecDeque::new()),
            losses: Losses::default(),
        }
    }

    fn locked(&self) -> MutexGuard<'_, VecDeque<u64>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps only the records the path still reports as waiting: it settles them in order, so the
    /// newest `pending` are the ones whose receipt has not come, and a receipt that failed is no
    /// longer waiting (the path counts it as a receipt failure).
    fn forget_settled(&self, waiting: &mut VecDeque<u64>) {
        let pending = usize::try_from(self.sink.health().pending).unwrap_or(usize::MAX);
        while waiting.len() > pending {
            waiting.pop_front();
        }
    }
}

fn severity(severity: RecordSeverity) -> Severity {
    match severity {
        RecordSeverity::Debug => Severity::Debug,
        RecordSeverity::Info => Severity::Info,
        RecordSeverity::Warn => Severity::Warn,
        RecordSeverity::Error => Severity::Error,
    }
}

fn value(value: &AttributeValue) -> AnyValue {
    match value {
        AttributeValue::Text(text) => AnyValue::String(text.clone().into()),
        AttributeValue::Whole(whole) => AnyValue::Int(*whole),
        AttributeValue::Flag(flag) => AnyValue::Boolean(*flag),
    }
}

/// One record as df-observe takes it: its audit line is the body, so the two renderings carry the
/// same text, and its time is the observed time in both of the record's clocks.
pub fn diagnostic_record(record: &ObservedRecord) -> DiagnosticRecord {
    let at = UNIX_EPOCH + Duration::from_millis(record.observed_ms);
    DiagnosticRecord {
        event_name: record.event_name,
        input: LogInput {
            timestamp: at,
            observed_timestamp: at,
            severity: severity(record.severity),
            body: AnyValue::String(record.audit_line.clone().into()),
            attributes: record
                .attributes
                .iter()
                .map(|(name, attribute)| (name.clone(), value(attribute)))
                .collect(),
            trace: None,
        },
    }
}

/// How a refusal of the shared path reads to the sink. Exhaustive, so a new refusal of the path
/// does not compile until it is placed.
fn refusal_of(error: &TelemetryError) -> OutletRefusal {
    match error {
        TelemetryError::Capacity | TelemetryError::SequenceExhausted => OutletRefusal::Full,
        TelemetryError::Oversized
        | TelemetryError::Malformed
        | TelemetryError::InvalidIdentity
        | TelemetryError::InvalidLimits => OutletRefusal::Rejected,
        TelemetryError::Closed
        | TelemetryError::Deadline
        | TelemetryError::Conflict
        | TelemetryError::Corrupt
        | TelemetryError::ForeignRoot
        | TelemetryError::Ownership
        | TelemetryError::Io
        | TelemetryError::SinkUnavailable
        | TelemetryError::Internal => OutletRefusal::Closed,
    }
}

impl RecordOutlet for SharedPathOutlet {
    fn name(&self) -> &'static str {
        "shared_path"
    }

    fn accept(&self, record: &ObservedRecord) -> Result<(), OutletRefusal> {
        let mut waiting = self.locked();
        match self.sink.emit(diagnostic_record(record)) {
            Ok(()) => {
                waiting.push_back(record.observed_ms);
                self.forget_settled(&mut waiting);
                Ok(())
            }
            Err(error) => {
                self.losses.record(record.class);
                Err(refusal_of(&error))
            }
        }
    }

    fn status(&self, now_ms: u64) -> OutletStatus {
        self.sink.poll_receipts();
        let mut waiting = self.locked();
        self.forget_settled(&mut waiting);
        let health = self.sink.health();
        OutletStatus {
            name: self.name().to_owned(),
            accepted: health.admitted,
            confirmed: health.confirmed,
            pending: health.pending,
            lag_ms: waiting
                .front()
                .map_or(0, |observed| now_ms.saturating_sub(*observed)),
            lost_always: self.losses.always(),
            lost_verbose: self.losses.verbose(),
            evicted: 0,
            unconfirmed: health.receipt_failures,
        }
    }
}
