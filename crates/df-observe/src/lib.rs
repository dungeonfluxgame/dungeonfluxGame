//! Shared OTEL conventions, experimental transport spans, and native bounded capture.
use opentelemetry::trace::{Span, SpanContext, SpanId, TraceFlags, TraceId, TraceState, Tracer};
use opentelemetry::{Context, KeyValue, global};

/// OTEL instrumentation scope of every operation span this crate starts.
const OPERATION_TRACER_SCOPE: &str = "df-observe.transport-fixture";
/// Length of a W3C `traceparent` version 00 value: `00-<32 hex>-<16 hex>-<2 hex>`.
const TRACE_PARENT_LENGTH: usize = 55;

/// Shared content-free operation identity, also used by browser PHONE diagnostics.
pub fn operation_hash(bytes: &[u8]) -> u64 {
    presentation::local_identity(&capture::lower_hex(bytes))
}

/// Seat identifiers use one diagnostic hash across native and browser records.
pub fn seat_hash(bytes: &[u8]) -> u64 {
    operation_hash(bytes)
}

/// Synchronous owner correlation. Async callers pass explicit fields at their boundary.
#[derive(Clone, Default)]
pub struct AuditContext {
    /// `operation_hash` of the operation being served.
    pub operation: Option<u64>,
    /// `seat_hash` of the acting seat.
    pub seat: Option<u64>,
    /// Diagnostic hash of the room.
    pub room: Option<u64>,
    /// Table revision label, such as `1:4`.
    pub revision: Option<String>,
}
thread_local! {
    static AUDIT_CONTEXT: std::cell::RefCell<AuditContext> =
        std::cell::RefCell::new(AuditContext::default());
    static AUDIT_FORWARDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static AUDIT_SINK: std::sync::OnceLock<fn(&'static str, String)> = std::sync::OnceLock::new();

/// Restore correlation even when an owner returns early or unwinds.
pub struct AuditScope(AuditContext, std::marker::PhantomData<std::rc::Rc<()>>);
impl Drop for AuditScope {
    fn drop(&mut self) {
        AUDIT_CONTEXT.with_borrow_mut(|context| *context = std::mem::take(&mut self.0));
    }
}
/// The correlation in force on this thread, owned, so asynchronous work can capture it at dispatch
/// and re-enter it with `audit_scope` when it later emits its audit lines.
pub fn current_audit_context() -> AuditContext {
    AUDIT_CONTEXT.with_borrow(Clone::clone)
}

/// Set correlation for synchronous work on the current thread only.
/// Never hold this guard across `.await`, including local browser futures: another task
/// may run on the same thread. Capture owned AuditContext for asynchronous dispatch instead.
pub fn audit_scope(context: AuditContext) -> AuditScope {
    AuditScope(
        AUDIT_CONTEXT.with_borrow_mut(|current| std::mem::replace(current, context)),
        std::marker::PhantomData,
    )
}

/// Install the application audit/timeline bridge once, before admission.
pub fn install_audit_sink(sink: fn(&'static str, String)) {
    // The sink is process-wide and set once; a later install keeps the first sink, which is
    // the documented contract, so the refusal carries nothing to handle.
    let _already_installed = AUDIT_SINK.set(sink);
}

/// Add explicit missing-context markers rather than silently attaching stale identities.
pub fn audit_line(mut line: String) -> String {
    AUDIT_CONTEXT.with_borrow(|context| {
        let numeric_alias = |key: &str| {
            line.split_ascii_whitespace()
                .find_map(|field| {
                    field
                        .strip_prefix(key)
                        .and_then(|value| value.parse::<u64>().ok())
                })
                .map(|value| value.to_string())
        };
        let operation = numeric_alias("operation_hash=")
            .or_else(|| context.operation.map(|value| value.to_string()));
        let seat =
            numeric_alias("seat_hash=").or_else(|| context.seat.map(|value| value.to_string()));
        let elapsed = numeric_alias("elapsed_ms=")
            .or_else(|| numeric_alias("duration_ms="))
            .or_else(|| numeric_alias("commit_ms="));
        for (key, value) in [
            ("op", operation),
            ("seat", seat),
            ("room", context.room.map(|value| value.to_string())),
            ("rev", context.revision.clone()),
            ("ms", elapsed.or_else(|| Some("unmeasured".to_owned()))),
        ] {
            if !line
                .split_ascii_whitespace()
                .any(|field| field.starts_with(&format!("{key}=")))
            {
                line.push_str(&format!(" {key}={}", value.as_deref().unwrap_or("none")));
            }
        }
    });
    line
}

/// Audit delivery is independent of whether OTEL has a provider or samples the span.
pub fn record_audit_line(event: &'static str, line: String) {
    let line = audit_line(line);
    if let Some(sink) = AUDIT_SINK.get() {
        AUDIT_FORWARDING.with(|forwarding| {
            if forwarding.replace(true) {
                return;
            }
            struct ForwardingReset<'a>(&'a std::cell::Cell<bool>);
            impl Drop for ForwardingReset<'_> {
                fn drop(&mut self) {
                    self.0.set(false);
                }
            }
            let _reset = ForwardingReset(forwarding);
            sink(event, line);
        });
    } else {
        println!("{line}");
    }
}

/// Correlation is diagnostic provenance, never authorization.
#[derive(Clone)]
pub struct OperationContext {
    /// W3C `traceparent` text; malformed or empty values start a new root.
    pub trace_parent: String,
    /// Build label admitted with the request.
    pub build: String,
}

fn parent(context: &OperationContext) -> Context {
    use opentelemetry::trace::TraceContextExt;
    // Version 00 has one fixed shape. Reject before traversing untrusted metadata.
    if context.trace_parent.len() != TRACE_PARENT_LENGTH {
        return Context::new();
    }
    let mut parts = context.trace_parent.split('-');
    if let (Some("00"), Some(trace), Some(span), Some(flags), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) && trace.len() == 32
        && span.len() == 16
        && flags.len() == 2
        && [trace, span, flags]
            .iter()
            .all(|part| capture::is_lower_hex(part))
        && let (Ok(trace), Ok(span), Ok(flags)) = (
            TraceId::from_hex(trace),
            SpanId::from_hex(span),
            u8::from_str_radix(flags, 16),
        )
        && trace != TraceId::INVALID
        && span != SpanId::INVALID
    {
        return Context::new().with_remote_span_context(SpanContext::new(
            trace,
            span,
            TraceFlags::new(flags),
            true,
            TraceState::default(),
        ));
    }
    Context::new()
}
/// Owned OTEL operation span. Drop records abandonment unless its owner finishes it.
pub struct OperationSpan {
    span: global::BoxedSpan,
    finished: bool,
    build: String,
    method: &'static str,
    audit: AuditContext,
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
}
/// Begin an operation using explicit correlation across asynchronous dispatch.
pub fn begin(context: &OperationContext, method: &'static str) -> OperationSpan {
    begin_with_tracer(&global::tracer(OPERATION_TRACER_SCOPE), context, method)
}
fn begin_with_tracer(
    tracer: &global::BoxedTracer,
    context: &OperationContext,
    method: &'static str,
) -> OperationSpan {
    OperationSpan::new(
        tracer.start_with_context(method, &parent(context)),
        context,
        method,
    )
}
impl OperationSpan {
    /// Own a started span with the admitted build and this thread's audit correlation.
    fn new(mut span: global::BoxedSpan, context: &OperationContext, method: &'static str) -> Self {
        span.set_attribute(KeyValue::new("fixture.build", context.build.clone()));
        Self {
            span,
            finished: false,
            build: context.build.clone(),
            method,
            audit: current_audit_context(),
            #[cfg(not(target_arch = "wasm32"))]
            started: std::time::Instant::now(),
        }
    }
    /// Attach a structured event with caller-selected safe provenance, never private payloads.
    pub fn event(&mut self, name: &'static str, attributes: &[(&'static str, String)]) {
        self.span.add_event(
            name,
            attributes
                .iter()
                .map(|(key, value)| KeyValue::new(*key, value.clone()))
                .collect(),
        );
    }
    /// Finish exactly once with a safe classification and actual observed byte count.
    pub fn finish(&mut self, status: &str, bytes: usize) {
        self.end(status, Some(bytes), true);
    }
    /// Finish without inventing a byte count when measurement is unavailable.
    pub fn finish_unmeasured(&mut self, status: &str) {
        self.end(status, None, true);
    }
    /// Finish a span only after its fact was already emitted through the shared audit sink.
    /// This explicit path avoids recursion without discarding ordinary `recorded` outcomes.
    pub fn finish_audit_record(&mut self) {
        self.end("recorded", None, false);
    }
    fn end(&mut self, status: &str, bytes: Option<usize>, emit_audit: bool) {
        if self.finished {
            return;
        }
        // OTEL integer attributes are signed. An unrepresentable observation is
        // unmeasured rather than a wrapped or fabricated byte count.
        let bytes = bytes.and_then(|bytes| i64::try_from(bytes).ok());
        self.span
            .set_attribute(KeyValue::new("rpc.status", status.to_owned()));
        self.span
            .set_attribute(KeyValue::new("rpc.bytes.measured", bytes.is_some()));
        if let Some(bytes) = bytes {
            self.span.set_attribute(KeyValue::new("rpc.bytes", bytes));
        }
        self.span.end();
        self.finished = true;
        // Audit-line spans explicitly opt out; ordinary classifications, including
        // "recorded", must remain visible even with no SDK or an unsampled trace.
        if emit_audit && !AUDIT_FORWARDING.with(std::cell::Cell::get) {
            #[cfg(not(target_arch = "wasm32"))]
            let elapsed = self.started.elapsed().as_millis().to_string();
            #[cfg(target_arch = "wasm32")]
            let elapsed = "unmeasured";
            let _scope = audit_scope(self.audit.clone());
            record_audit_line(
                self.method,
                format!(
                    "OUTCOME operation={} outcome={status} ms={} bytes={}",
                    self.method,
                    elapsed,
                    bytes.map_or_else(|| "unmeasured".to_owned(), |bytes| bytes.to_string()),
                ),
            );
        }
    }
}

impl Drop for OperationSpan {
    fn drop(&mut self) {
        self.finish_unmeasured("abandoned");
    }
}
/// Record a bounded lifecycle event without recording payload bytes.
/// Browser sends trace context in generated RPC metadata; this fixture installs
/// an exporting SDK on the native side only.
pub fn record(context: &OperationContext, method: &'static str, status: &str, bytes: usize) {
    begin(context, method).finish(status, bytes);
}
#[cfg(not(target_arch = "wasm32"))]
mod fixture_telemetry;
#[cfg(not(target_arch = "wasm32"))]
pub use fixture_telemetry::FixtureTelemetry;

#[cfg(not(target_arch = "wasm32"))]
mod ingress;
#[cfg(not(target_arch = "wasm32"))]
pub use ingress::*;
#[cfg(not(target_arch = "wasm32"))]
mod producer;
#[cfg(not(target_arch = "wasm32"))]
pub use producer::{
    AnyValue, CapturedBatch, LogInput, NativeProducer, NativeSpanData, NativeTraceBridge, Severity,
    SpanInput,
};

mod dispatch_context;
pub use dispatch_context::{begin_dispatched, begin_dispatched_with_audit};

mod capture;
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "capture, ingress and fixture bounds serve native-only modules"
    )
)]
mod limits;
pub use capture::{CaptureKey, ProducerId, RecordId, Signal, SourceSequence, TelemetryError};
mod browser;
pub mod presentation;
pub use browser::{
    BrowserBuffer, BrowserLog, BufferError, BufferSnapshot, UploadFailure, UploadLease,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod audit_tests {
    use super::*;
    static LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    fn sink(event: &'static str, line: String) {
        if event == "test.audit-terminal" {
            LINES.lock().unwrap().push(line);
        }
        let context = OperationContext {
            trace_parent: String::new(),
            build: "test".to_owned(),
        };
        begin(&context, "test.audit-record").finish_unmeasured("recorded");
    }
    #[test]
    fn terminal_audit_survives_no_sdk_and_keeps_dispatch_correlation_without_recursion() {
        install_audit_sink(sink);
        let context = OperationContext {
            trace_parent: String::new(),
            build: "test".to_owned(),
        };
        let owner = audit_scope(AuditContext {
            operation: Some(71),
            seat: Some(72),
            room: Some(73),
            revision: Some("1:4".to_owned()),
        });
        let mut operation = begin(&context, "test.audit-terminal");
        drop(owner);
        let _other = audit_scope(AuditContext {
            operation: Some(99),
            ..AuditContext::default()
        });
        operation.finish_unmeasured("retained");
        operation.finish_unmeasured("duplicate");
        let lines = LINES.lock().unwrap();
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(line.contains("outcome=retained"));
        assert!(line.contains("op=71 seat=72 room=73 rev=1:4"));
        assert!(!line.contains("op=99"));
        drop(lines);
        let parent_scope = audit_scope(AuditContext {
            operation: Some(81),
            seat: Some(82),
            room: Some(83),
            revision: Some("2:5".to_owned()),
        });
        let parent = begin(&context, "test.dispatch-parent");
        let dispatch = parent.capture_context();
        let audit = parent.capture_audit_context();
        drop(parent_scope);
        std::thread::spawn(move || {
            begin_dispatched_with_audit(&dispatch, &audit, "test.audit-terminal")
                .finish_unmeasured("worker-finished");
        })
        .join()
        .unwrap();
        let lines = LINES.lock().unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("outcome=worker-finished"));
        assert!(lines[1].contains("op=81 seat=82 room=83 rev=2:5"));
        drop(lines);
        begin(&context, "test.audit-terminal").finish_unmeasured("recorded");
        let lines = LINES.lock().unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("outcome=recorded"));
    }
    #[test]
    fn audit_scope_restores_context_and_explicit_fields_win() {
        let original = audit_line("TEST outcome=before".to_owned());
        {
            let _owner = audit_scope(AuditContext {
                operation: Some(7),
                ..AuditContext::default()
            });
            assert!(audit_line("TEST op=8 ms=4".to_owned()).contains("op=8 ms=4"));
            assert!(audit_line("TEST".to_owned()).contains("op=7"));
        }
        assert_eq!(audit_line("TEST outcome=before".to_owned()), original);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod diagnostics;
#[cfg(not(target_arch = "wasm32"))]
pub use diagnostics::{
    DiagnosticHealth, DiagnosticRecord, DiagnosticSink, DiagnosticTrace, NativeDiagnosticSink,
};
