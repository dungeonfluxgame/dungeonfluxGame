//! Bounded unsampled native logs over the shared producer and durable ingress.
//! Admission is unconfirmed until a receipt arrives; refusals and tail uncertainty stay visible.

use crate::{LogInput, NativeProducer, PendingReceipt, RecordId, TelemetryError};
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use std::{
    collections::VecDeque,
    sync::{
        Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Correlation contains IDs and flags only; vendor trace-state text never enters diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticTrace {
    trace_id: TraceId,
    span_id: SpanId,
    flags: TraceFlags,
}

impl DiagnosticTrace {
    /// Rebuild a local span context carrying only the IDs and flags.
    pub fn span_context(self) -> SpanContext {
        SpanContext::new(
            self.trace_id,
            self.span_id,
            self.flags,
            false,
            TraceState::default(),
        )
    }
}

impl From<&SpanContext> for DiagnosticTrace {
    fn from(context: &SpanContext) -> Self {
        Self {
            trace_id: context.trace_id(),
            span_id: context.span_id(),
            flags: context.trace_flags(),
        }
    }
}

impl From<SpanContext> for DiagnosticTrace {
    fn from(context: SpanContext) -> Self {
        Self::from(&context)
    }
}

/// Content-safe structured event supplied by its subsystem's typed privacy adapter.
#[derive(Debug)]
pub struct DiagnosticRecord {
    pub event_name: &'static str,
    pub input: LogInput,
}

/// Native composition owns the sink; implementations perform no blocking disk work here.
pub trait DiagnosticSink: Send + Sync {
    fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError>;
}

/// Receipt watermarks are observations, never a claim that intervening sequences are complete.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticHealth {
    pub attempted: u64,
    pub admitted: u64,
    pub confirmed: u64,
    pub refused: u64,
    pub receipt_failures: u64,
    pub pending: u64,
    pub last_reserved: u64,
    pub last_confirmed: u64,
}

struct PendingLog {
    sequence: u64,
    receipt: PendingReceipt,
}

struct Capture {
    producer: NativeProducer,
    pending: VecDeque<PendingLog>,
}

/// One native producer with bounded receipt tracking; the host owns its ingress/store lifetime.
pub struct NativeDiagnosticSink {
    capture: Mutex<Option<Capture>>,
    maximum_pending: usize,
    next_record: fn() -> [u8; 16],
    attempted: AtomicU64,
    admitted: AtomicU64,
    confirmed: AtomicU64,
    refused: AtomicU64,
    receipt_failures: AtomicU64,
    pending: AtomicU64,
    last_reserved: AtomicU64,
    last_confirmed: AtomicU64,
}

impl NativeDiagnosticSink {
    /// `next_record` supplies fresh opaque entropy, independent of private input or log content.
    pub fn new(
        producer: NativeProducer,
        maximum_pending: usize,
        next_record: fn() -> [u8; 16],
    ) -> Result<Self, TelemetryError> {
        if maximum_pending == 0 {
            return Err(TelemetryError::InvalidLimits);
        }
        Ok(Self {
            capture: Mutex::new(Some(Capture {
                producer,
                pending: VecDeque::new(),
            })),
            maximum_pending,
            next_record,
            attempted: AtomicU64::new(0),
            admitted: AtomicU64::new(0),
            confirmed: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            receipt_failures: AtomicU64::new(0),
            pending: AtomicU64::new(0),
            last_reserved: AtomicU64::new(0),
            last_confirmed: AtomicU64::new(0),
        })
    }

    /// Counters at this moment; each is read independently, without a snapshot lock.
    pub fn health(&self) -> DiagnosticHealth {
        DiagnosticHealth {
            attempted: self.attempted.load(Ordering::Relaxed),
            admitted: self.admitted.load(Ordering::Relaxed),
            confirmed: self.confirmed.load(Ordering::Relaxed),
            refused: self.refused.load(Ordering::Relaxed),
            receipt_failures: self.receipt_failures.load(Ordering::Relaxed),
            pending: self.pending.load(Ordering::Relaxed),
            last_reserved: self.last_reserved.load(Ordering::Relaxed),
            last_confirmed: self.last_confirmed.load(Ordering::Relaxed),
        }
    }

    /// Harvest matched receipts without making generation wait for ingestion.
    pub fn poll_receipts(&self) {
        if let Ok(mut capture) = self.capture.try_lock()
            && let Some(capture) = capture.as_mut()
        {
            self.receive(capture);
        }
    }

    fn receive(&self, capture: &mut Capture) {
        for _ in 0..capture.pending.len() {
            if let Some(pending) = capture.pending.pop_front() {
                match pending.receipt.try_receive() {
                    Ok(None) => capture.pending.push_back(pending),
                    Ok(Some(_)) => self.confirm(pending.sequence),
                    Err(_) => {
                        self.receipt_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        self.pending.store(
            u64::try_from(capture.pending.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    fn confirm(&self, sequence: u64) {
        self.confirmed.fetch_add(1, Ordering::Relaxed);
        self.last_confirmed.fetch_max(sequence, Ordering::Relaxed);
    }

    fn capture(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
        // Only bounded SDK encoding and nonblocking ingress run here. Ordinary producer
        // contention must not discard enabled logs while queue capacity remains available.
        let mut owner = self.capture.lock().map_err(|_| TelemetryError::Internal)?;
        let capture = owner.as_mut().ok_or(TelemetryError::Closed)?;
        self.receive(capture);
        let key = capture
            .producer
            .reserve(RecordId::new((self.next_record)())?)?;
        let sequence =
            u64::try_from(key.sequence.get()).map_err(|_| TelemetryError::InvalidIdentity)?;
        self.last_reserved.store(sequence, Ordering::Relaxed);
        // Reserve before refusal so a bounded queue outage leaves a visible source gap.
        if capture.pending.len() >= self.maximum_pending {
            return Err(TelemetryError::Capacity);
        }
        let batch = capture
            .producer
            .emit_named_log(key, record.event_name, record.input)?;
        capture.pending.push_back(PendingLog {
            sequence,
            receipt: batch.pending,
        });
        self.admitted.fetch_add(1, Ordering::Relaxed);
        self.receive(capture);
        Ok(())
    }

    /// Host shutdown only: drain already admitted receipts within one absolute deadline.
    pub fn shutdown(&self, timeout: Duration) -> Result<(), TelemetryError> {
        let started = Instant::now();
        let mut waited = false;
        let mut capture = loop {
            match self.capture.try_lock() {
                Ok(mut owner) => {
                    if waited && started.elapsed() >= timeout {
                        return Err(TelemetryError::Deadline);
                    }
                    break owner.take().ok_or(TelemetryError::Closed)?;
                }
                Err(TryLockError::Poisoned(_)) => return Err(TelemetryError::Internal),
                Err(TryLockError::WouldBlock) => {
                    waited = true;
                    let remaining = timeout.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        return Err(TelemetryError::Deadline);
                    }
                    std::thread::sleep(remaining.min(Duration::from_millis(1)));
                }
            }
        };
        let mut failure = None;
        while let Some(pending) = capture.pending.pop_front() {
            match pending
                .receipt
                .wait(timeout.saturating_sub(started.elapsed()))
            {
                Ok(_) => self.confirm(pending.sequence),
                Err(error) => {
                    self.receipt_failures.fetch_add(1, Ordering::Relaxed);
                    failure.get_or_insert(error);
                }
            }
        }
        self.pending.store(0, Ordering::Relaxed);
        if let Err(error) = capture
            .producer
            .shutdown(timeout.saturating_sub(started.elapsed()))
        {
            failure.get_or_insert(error);
        }
        failure.map_or(Ok(()), Err)
    }
}

impl DiagnosticSink for NativeDiagnosticSink {
    fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
        self.attempted.fetch_add(1, Ordering::Relaxed);
        let result = self.capture(record);
        if result.is_err() {
            self.refused.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
}
