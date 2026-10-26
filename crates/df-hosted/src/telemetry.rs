//! Hosted logs, request traces and typed metric measurements use the shared SDK/SQLite spool.
//! Raw paths and audit payloads never cross the privacy boundary. Admission is fail-fast;
//! only an ingestion receipt establishes durability, and rejected/lost receipts are visible.
use axum::{
    extract::{MatchedPath, Request, State},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};
use df_observe::{
    AnyValue, CapturedBatch, LogInput, NativeProducer, PendingReceipt, ProducerId, RecordId,
    Severity, SourceSequence, SpanInput, TelemetryError, TelemetryIngress, TelemetryLimits,
};
use df_telemetry::Store;
use opentelemetry::{
    Context, KeyValue,
    trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    io::Write,
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

const MAXIMUM_PENDING_RECEIPTS: usize = 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static EMERGENCY_OUTPUT_FAILURES: AtomicU64 = AtomicU64::new(0);
static ACTIVE: Mutex<Weak<HostedTelemetry>> = Mutex::new(Weak::new());
thread_local! { static CURRENT: RefCell<Option<RequestContext>> = const { RefCell::new(None) }; }
#[derive(Clone, Debug)]
pub(crate) struct CorrelationId(pub(crate) String);

/// Capture this owned context before dispatch, then scope the dispatched future with it.
#[derive(Clone)]
pub(crate) struct RequestContext {
    telemetry: Arc<HostedTelemetry>,
    span: SpanContext,
}
impl RequestContext {
    pub(crate) async fn scope<F: Future>(&self, future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        std::future::poll_fn(|task| {
            let previous = CURRENT.replace(Some(self.clone()));
            struct Reset(Option<RequestContext>);
            impl Drop for Reset {
                fn drop(&mut self) {
                    CURRENT.set(self.0.take());
                }
            }
            // Re-enter on each poll, never hold thread-local context across suspension.
            let _reset = Reset(previous);
            let context = Context::new().with_remote_span_context(self.span.clone());
            let _otel = context.attach();
            future.as_mut().poll(task)
        })
        .await
    }
}
struct Capture {
    producer: NativeProducer,
    pending: VecDeque<PendingExport>,
}
struct PendingExport {
    receipt: PendingReceipt,
    trace: Option<SpanContext>,
}
struct RequestMeasurement<'a> {
    route: &'a str,
    method: &'static str,
    status: u16,
    start: SystemTime,
    elapsed: Duration,
}
pub(crate) struct HostedTelemetry {
    capture: Mutex<Option<Capture>>,
    store: Mutex<Option<Store>>,
    failures: AtomicU64,
    fallback_events: AtomicU64,
    started: Instant,
    traces: Mutex<Option<df_observe::NativeTraceBridge>>,
}
impl HostedTelemetry {
    pub(crate) fn open(dir: Option<&Path>) -> Arc<Self> {
        df_observe::install_audit_sink(forward_audit);
        let telemetry = match dir {
            Some(dir) => match Self::open_at(dir) {
                Ok(telemetry) => Arc::new(telemetry),
                Err(error) => {
                    emergency(&error);
                    let telemetry = Self::disabled();
                    telemetry.failures.store(1, Ordering::Relaxed);
                    Arc::new(telemetry)
                }
            },
            None => Arc::new(Self::disabled()),
        };
        if let Ok(mut active) = ACTIVE.lock() {
            *active = Arc::downgrade(&telemetry);
        } else {
            telemetry.record_failure(TelemetryError::Internal);
        }
        if let Ok(mut traces) = telemetry.traces.lock() {
            *traces = Some(df_observe::NativeTraceBridge::install(forward_span));
        } else {
            telemetry.record_failure(TelemetryError::Internal);
        }
        let event = if dir.is_none() {
            "hosted.telemetry_disabled"
        } else if telemetry.failures.load(Ordering::Relaxed) != 0 {
            "hosted.telemetry_unavailable"
        } else {
            "hosted.telemetry_ready"
        };
        df_observe::record_audit_line(event, format!("HOSTED event={event}"));
        telemetry
    }
    fn open_at(dir: &Path) -> Result<Self, TelemetryError> {
        let store = Store::open_configured(dir, TelemetryLimits::default())?;
        let mut telemetry = Self::from_ingress(store.ingress())?;
        telemetry.store = Mutex::new(Some(store));
        Ok(telemetry)
    }
    fn from_ingress(ingress: Arc<dyn TelemetryIngress>) -> Result<Self, TelemetryError> {
        let version = concat!("df-hosted-", env!("CARGO_PKG_VERSION"));
        let build = df_types::BuildIdentity::new(
            Some(version),
            Some(version),
            Some("not-applicable"),
            Some("hosted-telemetry-v1"),
            Some("not-applicable"),
        )
        .map_err(|_| TelemetryError::InvalidIdentity)?;
        let producer = NativeProducer::for_service(
            ProducerId::new(identity_bytes())?,
            SourceSequence::new(1)?,
            &build,
            ingress,
            TelemetryLimits::default(),
            "df-portal-hosted",
        )?;
        Ok(Self {
            capture: Mutex::new(Some(Capture {
                producer,
                pending: VecDeque::new(),
            })),
            ..Self::disabled()
        })
    }
    pub(crate) fn disabled() -> Self {
        Self {
            capture: Mutex::new(None),
            store: Mutex::new(None),
            failures: AtomicU64::new(0),
            fallback_events: AtomicU64::new(0),
            started: Instant::now(),
            traces: Mutex::new(None),
        }
    }
    fn record_failure(&self, error: TelemetryError) {
        self.record_failure_for(error, None);
    }
    fn record_failure_for(&self, error: TelemetryError, trace: Option<&SpanContext>) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        write_emergency(format_args!(
            "HOSTED telemetry_export_failed error={error:?} {}\n",
            correlation_fields(trace)
        ));
    }
    fn poll_receipts(&self, capture: &mut Capture) {
        for _ in 0..capture.pending.len() {
            if let Some(pending) = capture.pending.pop_front() {
                match pending.receipt.try_receive() {
                    Ok(None) => capture.pending.push_back(pending),
                    Ok(Some(_)) => {}
                    Err(error) => self.record_failure_for(error, pending.trace.as_ref()),
                }
            }
        }
    }
    fn capture(
        &self,
        trace: Option<SpanContext>,
        emit: impl FnOnce(
            &mut NativeProducer,
            df_observe::CaptureKey,
        ) -> Result<CapturedBatch, TelemetryError>,
    ) -> bool {
        // NativeProducer is sequential. This short lock covers bounded encoding/admission only;
        // the shared ingress worker owns every disk wait. Contention must not sample logs.
        let Ok(mut owner) = self.capture.lock() else {
            self.record_failure_for(TelemetryError::Internal, trace.as_ref());
            return false;
        };
        let Some(capture) = owner.as_mut() else {
            return false;
        };
        self.poll_receipts(capture);
        if capture.pending.len() >= MAXIMUM_PENDING_RECEIPTS {
            self.record_failure_for(TelemetryError::Capacity, trace.as_ref());
            return false;
        }
        let result = RecordId::new(identity_bytes())
            .and_then(|record| capture.producer.reserve(record))
            .and_then(|key| emit(&mut capture.producer, key));
        match result {
            Ok(batch) => {
                capture.pending.push_back(PendingExport {
                    receipt: batch.pending,
                    trace,
                });
                self.poll_receipts(capture);
                true
            }
            Err(error) => {
                self.record_failure_for(error, trace.as_ref());
                false
            }
        }
    }
    fn emit_log(
        &self,
        event: &'static str,
        severity: Severity,
        mut attributes: Vec<(String, AnyValue)>,
        trace: Option<SpanContext>,
    ) {
        attributes.push((
            "telemetry.emergency_output_failures".to_owned(),
            AnyValue::Int(
                i64::try_from(EMERGENCY_OUTPUT_FAILURES.load(Ordering::Relaxed))
                    .unwrap_or(i64::MAX),
            ),
        ));
        if let Ok(store) = self.store.try_lock()
            && let Some(store) = store.as_ref()
        {
            if let Err(error) = store.set_time(self.started.elapsed()) {
                self.record_failure(error);
            }
            let health = store.health();
            for (name, value) in [
                (
                    "telemetry.backlog_files",
                    u64::try_from(health.backlog_files).unwrap_or(u64::MAX),
                ),
                (
                    "telemetry.pending_items",
                    u64::try_from(health.pending_items).unwrap_or(u64::MAX),
                ),
                ("telemetry.lag_ms", health.lag_millis),
                ("telemetry.rejected", health.rejected),
                ("telemetry.emergency_records", health.emergency_records),
            ] {
                attributes.push((
                    name.to_owned(),
                    AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
                ));
            }
        }
        let now = SystemTime::now();
        if !self.capture(trace.clone(), |producer, key| {
            producer.emit_log(
                key,
                LogInput {
                    timestamp: now,
                    observed_timestamp: now,
                    severity,
                    body: AnyValue::String(event.into()),
                    attributes: std::iter::once((
                        "df.operation".to_owned(),
                        AnyValue::String(event.into()),
                    ))
                    .chain(attributes)
                    .collect(),
                    trace: trace.clone(),
                },
            )
        }) {
            self.fallback_events.fetch_add(1, Ordering::Relaxed);
            write_emergency(format_args!("{}", fallback_line(event, trace.as_ref())));
        }
    }
    fn record_audit(&self, event: &'static str, line: &str, trace: Option<SpanContext>) {
        // Legacy audit text may contain any credential. Preserve event identity and numeric
        // measurements; fail closed on all text payloads, not just recognizable secret formats.
        let mut attributes = vec![("audit.payload_redacted".to_owned(), AnyValue::Boolean(true))];
        for field in line.split_ascii_whitespace().take(64) {
            if let Some((key, value)) = field.split_once('=')
                && matches!(
                    key,
                    "ms" | "elapsed_ms" | "duration_ms" | "count" | "pending" | "capacity"
                )
                && let Ok(number) = value.parse::<i64>()
            {
                attributes.push((format!("audit.{key}"), AnyValue::Int(number)));
            }
        }
        let severity = if event.ends_with("_failed")
            || event.ends_with("_unavailable")
            || event.ends_with("_ended")
        {
            Severity::Error
        } else if event.ends_with("_refused") {
            Severity::Warn
        } else {
            Severity::Info
        };
        self.emit_log(event, severity, attributes, trace.clone());
        let outcome = match event {
            "hosted.purchase_posted" => Some((
                "payment.purchase",
                match line.split_ascii_whitespace().nth(1) {
                    Some("purchase") => "posted",
                    Some("purchase_replayed") => "replayed",
                    _ => "unclassified",
                },
            )),
            "hosted.stripe_failed" | "hosted.paypal_failed" => Some(("payment.provider", "failed")),
            "hosted.paypal_cancelled" => Some(("payment.cancel", "ok")),
            "account.pool_checkout_ok" => Some(("database.pool.checkout", "ok")),
            "account.pool_checkout_failed" => Some(("database.pool.checkout", "failed")),
            _ => None,
        };
        if let Some((operation, outcome)) = outcome {
            self.emit_log(
                "hosted.outcome",
                severity,
                vec![
                    (
                        "metric.operation".to_owned(),
                        AnyValue::String(operation.into()),
                    ),
                    (
                        "metric.outcome".to_owned(),
                        AnyValue::String(outcome.into()),
                    ),
                    ("metric.count".to_owned(), AnyValue::Int(1)),
                ],
                trace,
            );
        }
    }
    pub(crate) fn record_db_outcome(&self, success: bool) {
        self.emit_log(
            "hosted.database_open",
            if success {
                Severity::Info
            } else {
                Severity::Error
            },
            vec![
                ("metric.count".to_owned(), AnyValue::Int(1)),
                ("metric.ok".to_owned(), AnyValue::Boolean(success)),
            ],
            None,
        );
    }
    fn finish_request(
        &self,
        context: &RequestContext,
        parent: Context,
        measurement: RequestMeasurement<'_>,
    ) {
        let RequestMeasurement {
            route,
            method,
            status,
            start,
            elapsed,
        } = measurement;
        let duration_ms = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
        self.emit_log(
            "hosted.request",
            Severity::Info,
            vec![
                (
                    "http.route".to_owned(),
                    AnyValue::String(route.to_owned().into()),
                ),
                (
                    "http.method".to_owned(),
                    AnyValue::String(method.to_owned().into()),
                ),
                ("http.status".to_owned(), AnyValue::Int(i64::from(status))),
                ("metric.request_count".to_owned(), AnyValue::Int(1)),
                ("metric.duration_ms".to_owned(), AnyValue::Int(duration_ms)),
                (
                    "correlation.id".to_owned(),
                    AnyValue::String(context.span.trace_id().to_string().into()),
                ),
                (
                    "telemetry.export_failures".to_owned(),
                    AnyValue::Int(
                        i64::try_from(self.failures.load(Ordering::Relaxed)).unwrap_or(i64::MAX),
                    ),
                ),
            ],
            Some(context.span.clone()),
        );
        self.capture(Some(context.span.clone()), |producer, key| {
            producer.emit_correlated_span(
                key,
                SpanInput {
                    name: "hosted.request".to_owned(),
                    start,
                    end: SystemTime::now(),
                    parent,
                    links: Vec::new(),
                    attributes: vec![
                        KeyValue::new("http.route", route.to_owned()),
                        KeyValue::new("http.method", method.to_owned()),
                        KeyValue::new("http.status", i64::from(status)),
                    ],
                },
                context.span.clone(),
            )
        });
        if method == "POST"
            && matches!(
                route,
                "/app/api/payments/stripe/webhook" | "/app/api/payments/paypal/webhook"
            )
        {
            self.record_webhook(route, status, context.span.clone());
        }
    }
    fn record_webhook(&self, route: &str, status: u16, trace: SpanContext) {
        let outcome = match status {
            200..=299 => "accepted",
            400..=499 => "rejected",
            _ => "failed",
        };
        self.emit_log(
            "hosted.webhook",
            Severity::Info,
            vec![
                (
                    "http.route".to_owned(),
                    AnyValue::String(route.to_owned().into()),
                ),
                (
                    "metric.outcome".to_owned(),
                    AnyValue::String(outcome.into()),
                ),
                ("metric.count".to_owned(), AnyValue::Int(1)),
            ],
            Some(trace),
        );
    }
    pub(crate) fn shutdown(&self) {
        if let Ok(mut traces) = self.traces.lock()
            && let Some(traces) = traces.take()
            && let Err(error) = traces.shutdown(Duration::from_secs(1))
        {
            self.record_failure(error);
        }
        let mut owner = match self.capture.lock() {
            Ok(owner) => owner,
            Err(_) => {
                self.record_failure(TelemetryError::Internal);
                return;
            }
        };
        if let Ok(mut store) = self.store.lock()
            && let Some(store) = store.as_mut()
        {
            if let Err(error) = store.set_time(self.started.elapsed()) {
                self.record_failure(error);
            }
            if let Err(error) = store.flush(Duration::from_secs(1)) {
                self.record_failure(error);
            }
            if let Err(error) = store.shutdown(Duration::from_secs(2)) {
                self.record_failure(error);
            }
        }
        if let Some(mut capture) = owner.take() {
            self.poll_receipts(&mut capture);
            if !capture.pending.is_empty() {
                self.record_failure(TelemetryError::Deadline);
            }
            if let Err(error) = capture.producer.shutdown(Duration::from_secs(1)) {
                self.record_failure(error);
            }
        }
    }
    #[cfg(test)]
    fn failures(&self) -> u64 {
        if let Ok(mut owner) = self.capture.lock()
            && let Some(capture) = owner.as_mut()
        {
            self.poll_receipts(capture);
        }
        self.failures.load(Ordering::Relaxed)
    }
}
fn forward_span(mut input: df_observe::NativeSpanData) {
    let telemetry = CURRENT
        .with_borrow(|context| {
            context
                .as_ref()
                .map(|context| Arc::clone(&context.telemetry))
        })
        .or_else(active_telemetry);
    if let Some(telemetry) = telemetry {
        input.name = "hosted.operation".into();
        input.attributes = vec![KeyValue::new("span.payload_redacted", true)];
        input.span_context = without_trace_state(&input.span_context);
        for link in &mut input.links.links {
            link.attributes.clear();
            link.span_context = without_trace_state(&link.span_context);
        }
        for event in &mut input.events.events {
            event.name = "hosted.event".into();
            event.attributes = vec![KeyValue::new("event.payload_redacted", true)];
        }
        if let opentelemetry::trace::Status::Error { description } = &mut input.status {
            *description = "[redacted]".into();
        }
        let name = match input.instrumentation_scope.name() {
            "df-observe.transport-fixture" => "df-observe.transport-fixture",
            _ => "redacted",
        };
        input.instrumentation_scope = opentelemetry::InstrumentationScope::builder(name)
            .with_attributes([KeyValue::new("scope.payload_redacted", true)])
            .build();
        telemetry.capture(Some(input.span_context.clone()), |producer, key| {
            producer.emit_finished_span(key, input)
        });
    }
}
fn without_trace_state(context: &SpanContext) -> SpanContext {
    SpanContext::new(
        context.trace_id(),
        context.span_id(),
        context.trace_flags(),
        context.is_remote(),
        TraceState::default(),
    )
}
fn forward_audit(event: &'static str, line: String) {
    if let Some(context) = CURRENT.with_borrow(Clone::clone) {
        context
            .telemetry
            .record_audit(event, &line, Some(context.span));
    } else {
        let active = active_telemetry();
        if let Some(telemetry) = active {
            telemetry.record_audit(event, &line, None);
        } else {
            // Independent emergency output contains no untrusted audit payload.
            write_emergency(format_args!(
                "HOSTED audit event={event} payload=redacted\n"
            ));
        }
    }
}
fn active_telemetry() -> Option<Arc<HostedTelemetry>> {
    match ACTIVE.lock() {
        Ok(active) => active.upgrade(),
        Err(_) => {
            emergency(&TelemetryError::Internal);
            None
        }
    }
}
fn emergency(error: &TelemetryError) {
    // The failed exporter cannot report itself. Only a closed error class leaves this path.
    write_emergency(format_args!(
        "HOSTED telemetry_export_failed error={error:?}\n"
    ));
}
fn correlation_fields(trace: Option<&SpanContext>) -> String {
    match trace.filter(|trace| trace.is_valid()) {
        Some(trace) => format!("trace={} span={}", trace.trace_id(), trace.span_id()),
        None => "trace=none span=none".to_owned(),
    }
}
fn fallback_line(event: &'static str, trace: Option<&SpanContext>) -> String {
    format!(
        "HOSTED audit event={event} {} payload=redacted\n",
        correlation_fields(trace)
    )
}
fn write_emergency(arguments: std::fmt::Arguments<'_>) {
    if std::io::stderr().lock().write_fmt(arguments).is_err() {
        // There is no recursive fallback; retain the failure for the recovered pipeline.
        EMERGENCY_OUTPUT_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
}
fn identity_bytes() -> [u8; 16] {
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let hash =
        df_observe::operation_hash(format!("{}:{nanos}:{sequence}", std::process::id()).as_bytes());
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&hash.to_be_bytes());
    bytes[8..].copy_from_slice(&sequence.to_be_bytes());
    bytes
}
fn parse_traceparent(value: &str) -> Option<SpanContext> {
    if value.len() != 55 {
        return None;
    }
    let mut fields = value.split('-');
    let (Some("00"), Some(trace), Some(span), Some(flags), None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return None;
    };
    if trace.len() != 32
        || span.len() != 16
        || flags.len() != 2
        || ![trace, span, flags].iter().all(|part| {
            part.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return None;
    }
    let context = SpanContext::new(
        TraceId::from_hex(trace).ok()?,
        SpanId::from_hex(span).ok()?,
        TraceFlags::new(u8::from_str_radix(flags, 16).ok()?),
        true,
        TraceState::default(),
    );
    context.is_valid().then_some(context)
}
pub(crate) async fn observe(
    State(telemetry): State<Arc<HostedTelemetry>>,
    mut request: Request,
    next: Next,
) -> Response {
    let remote = request
        .headers()
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_traceparent);
    let trace = remote.as_ref().map_or_else(
        || TraceId::from_bytes(identity_bytes()),
        |parent| parent.trace_id(),
    );
    let span = SpanContext::new(
        trace,
        SpanId::from_bytes(NEXT_ID.fetch_add(1, Ordering::Relaxed).to_be_bytes()),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );
    let parent = remote.map_or_else(Context::new, |remote| {
        Context::new().with_remote_span_context(remote)
    });
    let context = RequestContext {
        telemetry: Arc::clone(&telemetry),
        span,
    };
    let method = match request.method().as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    };
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let correlation = CorrelationId(trace.to_string());
    request.extensions_mut().insert(correlation.clone());
    request.extensions_mut().insert(context.clone());
    let start = SystemTime::now();
    let started = Instant::now();
    context
        .scope(async {
            let mut response = next.run(request).await;
            telemetry.finish_request(
                &context,
                parent,
                RequestMeasurement {
                    route: &route,
                    method,
                    status: response.status().as_u16(),
                    start,
                    elapsed: started.elapsed(),
                },
            );
            if let Ok(value) =
                HeaderValue::from_str(&format!("00-{}-{}-01", trace, context.span.span_id()))
            {
                response.headers_mut().insert("traceparent", value);
            }
            if let Ok(value) = HeaderValue::from_str(&correlation.0) {
                response.headers_mut().insert("x-correlation-id", value);
            }
            response
        })
        .await
}
#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod telemetry_tests;
