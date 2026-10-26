use super::*;
use axum::{
    Extension, Router,
    http::{Request, StatusCode},
    middleware::from_fn_with_state,
    routing::{any, get, post},
};
use df_observe::{
    Durability, IngestReceipt, PendingReceipt, Signal, TelemetryError, TelemetryIngress,
};
use http_body_util::BodyExt;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
const TRACE: &str = "11111111111111111111111111111111";
const TRACEPARENT: &str = "00-11111111111111111111111111111111-2222222222222222-01";

#[derive(Debug, Default)]
struct Collected {
    bytes: Mutex<Vec<Vec<u8>>>,
}
impl TelemetryIngress for Collected {
    fn submit(&self, _signal: Signal, bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        self.bytes.lock().unwrap().push(bytes.to_vec());
        let (sender, receipt) = PendingReceipt::channel();
        sender
            .send(Ok(IngestReceipt {
                durability: Durability::Committed,
                spool_position: 1,
                committed_watermark: 1,
                records: 1,
            }))
            .unwrap();
        Ok(receipt)
    }
}
#[derive(Debug)]
struct Failing;
impl TelemetryIngress for Failing {
    fn submit(&self, _signal: Signal, _bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        Err(TelemetryError::Capacity)
    }
}
#[derive(Debug, Default)]
struct Delayed {
    replies: Mutex<Vec<std::sync::mpsc::Sender<Result<IngestReceipt, TelemetryError>>>>,
}
impl TelemetryIngress for Delayed {
    fn submit(&self, _signal: Signal, _bytes: &[u8]) -> Result<PendingReceipt, TelemetryError> {
        let (sender, receipt) = PendingReceipt::channel();
        self.replies.lock().unwrap().push(sender);
        Ok(receipt)
    }
}
fn pipeline(ingress: Arc<dyn TelemetryIngress>) -> Arc<HostedTelemetry> {
    df_observe::install_audit_sink(forward_audit);
    Arc::new(HostedTelemetry::from_ingress(ingress).unwrap())
}
async fn echo(Extension(id): Extension<CorrelationId>) -> String {
    id.0
}
fn app(telemetry: Arc<HostedTelemetry>) -> Router {
    Router::new()
        .route("/app/items/{id}", get(echo))
        .route(
            "/app/api/payments/stripe/webhook",
            post(|| async { StatusCode::OK }),
        )
        .route("/method", any(|| async { StatusCode::NO_CONTENT }))
        .layer(from_fn_with_state(telemetry, observe))
}
fn contains(bytes: &[u8], text: &str) -> bool {
    bytes
        .windows(text.len())
        .any(|window| window == text.as_bytes())
}
async fn body(response: Response) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}
fn request(uri: &str) -> Request<axum::body::Body> {
    Request::get(uri).body(axum::body::Body::empty()).unwrap()
}
struct Root(std::path::PathBuf);
impl Root {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("artifacts/tmp/agents/OBSEXPORT/data");
        std::fs::create_dir_all(&base).unwrap();
        Self(base.join(format!(
            "fixture-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        if self.0.exists() {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

#[tokio::test]
async fn correlation_propagates_request_response_and_malformed_input_is_never_echoed() {
    let telemetry = pipeline(Arc::new(Collected::default()));
    let router = app(telemetry);
    let response = router
        .clone()
        .oneshot(
            Request::get("/app/items/abc")
                .header("traceparent", TRACEPARENT)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["x-correlation-id"], TRACE);
    assert!(
        response.headers()["traceparent"]
            .to_str()
            .unwrap()
            .starts_with(&format!("00-{TRACE}-"))
    );
    assert_eq!(body(response).await, TRACE);
    for malformed in [
        "secret@example.com",
        "00-00000000000000000000000000000000-2222222222222222-01",
        "00-11111111111111111111111111111111-0000000000000000-01",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::get("/app/items/abc")
                    .header("traceparent", malformed)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let generated = response.headers()["x-correlation-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(generated.len(), 32);
        assert_ne!(generated, malformed);
        assert_eq!(body(response).await, generated);
    }
}

#[tokio::test]
async fn metrics_export_only_route_templates_fixed_unmatched_and_bounded_methods() {
    let ingress = Arc::new(Collected::default());
    let telemetry = pipeline(ingress.clone());
    let router = app(telemetry);
    for path in [
        "/app/items/shorttoken?email=person@example.com",
        "/secret",
        "/person%40example.com",
        "/abcdefghijklmnopqrstuvwxyz",
        "/x?token=abc",
    ] {
        router.clone().oneshot(request(path)).await.unwrap();
    }
    router
        .oneshot(
            Request::builder()
                .method("SECRET_METHOD")
                .uri("/method")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = ingress.bytes.lock().unwrap();
    assert!(bytes.iter().any(|bytes| contains(bytes, "/app/items/{id}")));
    assert!(bytes.iter().any(|bytes| contains(bytes, "unmatched")));
    assert!(bytes.iter().any(|bytes| contains(bytes, "OTHER")));
    assert!(
        bytes
            .iter()
            .any(|bytes| contains(bytes, "metric.request_count")
                && contains(bytes, "metric.duration_ms"))
    );
    for secret in [
        "shorttoken",
        "person@example.com",
        "%40",
        "abcdefghijklmnopqrstuvwxyz",
        "SECRET_METHOD",
        "token=abc",
    ] {
        assert!(
            !bytes.iter().any(|bytes| contains(bytes, secret)),
            "leaked {secret}"
        );
    }
}

#[tokio::test]
async fn audit_redaction_fails_closed_for_emails_tokens_queries_and_unknown_text_fields() {
    let ingress = Arc::new(Collected::default());
    let telemetry = pipeline(ingress.clone());
    telemetry.record_audit("test.audit", "AUTH email=person@example.com authorization=Bearer abc token=short url=/reset/abc?token=secret arbitrary=opaque_credential elapsed_ms=17", None);
    let bytes = ingress.bytes.lock().unwrap();
    assert_eq!(bytes.len(), 1);
    assert!(contains(&bytes[0], "test.audit"));
    assert!(contains(&bytes[0], "audit.elapsed_ms"));
    assert!(contains(&bytes[0], "audit.payload_redacted"));
    for secret in [
        "person@example.com",
        "Bearer",
        "Bearer abc",
        "short",
        "/reset",
        "secret",
        "opaque_credential",
    ] {
        assert!(!contains(&bytes[0], secret), "leaked {secret}");
    }
}

#[tokio::test]
async fn admission_failure_does_not_block_requests_and_emits_independent_diagnostic() {
    let telemetry = pipeline(Arc::new(Failing));
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        app(telemetry.clone()).oneshot(request("/app/items/x")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(telemetry.failures() >= 2);
}

#[tokio::test]
async fn post_admission_receipt_failure_is_visible_without_changing_http_outcome() {
    let ingress = Arc::new(Delayed::default());
    let telemetry = pipeline(ingress.clone());
    let response = app(telemetry.clone())
        .oneshot(request("/app/items/x"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(telemetry.failures(), 0);
    for reply in ingress.replies.lock().unwrap().drain(..) {
        reply.send(Err(TelemetryError::Io)).unwrap();
    }
    assert_eq!(telemetry.failures(), 2);
}

#[tokio::test]
async fn pending_receipts_are_bounded_and_full_export_never_stalls_a_request() {
    let ingress = Arc::new(Delayed::default());
    let telemetry = pipeline(ingress);
    for _ in 0..MAXIMUM_PENDING_RECEIPTS {
        telemetry.emit_log("test.pending", Severity::Info, Vec::new(), None);
    }
    let response = app(telemetry.clone())
        .oneshot(request("/app/items/x"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(telemetry.failures() >= 2);
    assert_eq!(
        telemetry
            .capture
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .pending
            .len(),
        MAXIMUM_PENDING_RECEIPTS
    );
}

#[tokio::test]
async fn payment_and_webhook_measurements_are_exported_from_authoritative_events() {
    let ingress = Arc::new(Collected::default());
    let telemetry = pipeline(ingress.clone());
    telemetry.record_audit(
        "hosted.purchase_posted",
        "SPEND purchase account=secret",
        None,
    );
    telemetry.record_audit(
        "hosted.purchase_posted",
        "SPEND purchase_replayed account=secret",
        None,
    );
    telemetry.record_audit(
        "hosted.stripe_failed",
        "SPEND stripe_failed error=secret",
        None,
    );
    telemetry.record_audit(
        "account.pool_checkout_failed",
        "ACCOUNT elapsed_ms=12",
        None,
    );
    app(telemetry)
        .oneshot(
            Request::post("/app/api/payments/stripe/webhook")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = ingress.bytes.lock().unwrap();
    for expected in [
        "payment.purchase",
        "posted",
        "replayed",
        "payment.provider",
        "database.pool.checkout",
        "hosted.webhook",
        "accepted",
        "metric.count",
    ] {
        assert!(
            bytes.iter().any(|bytes| contains(bytes, expected)),
            "missing {expected}"
        );
    }
    assert!(!bytes.iter().any(|bytes| contains(bytes, "secret")));
}

async fn async_audit(
    Extension(id): Extension<CorrelationId>,
    Extension(context): Extension<RequestContext>,
) -> String {
    df_observe::record_audit_line("test.before", "AUTH email=person@example.com".to_owned());
    tokio::task::yield_now().await;
    let dispatched = tokio::spawn(async move {
        context
            .scope(async {
                tokio::task::yield_now().await;
                df_observe::record_audit_line("test.dispatched", "AUTH token=secret".to_owned());
            })
            .await
    });
    dispatched.await.unwrap();
    df_observe::record_audit_line("test.after", "AUTH query=?secret".to_owned());
    id.0
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_correlation_survives_interleaved_awaits_and_explicit_owned_dispatch() {
    let root = Root::new();
    let telemetry = Arc::new(HostedTelemetry::open_at(&root.0).unwrap());
    df_observe::install_audit_sink(forward_audit);
    let router = Router::new()
        .route("/audit", get(async_audit))
        .layer(from_fn_with_state(telemetry.clone(), observe));
    let first = Request::get("/audit")
        .header("traceparent", TRACEPARENT)
        .body(axum::body::Body::empty())
        .unwrap();
    let second = Request::get("/audit")
        .header(
            "traceparent",
            "00-33333333333333333333333333333333-4444444444444444-01",
        )
        .body(axum::body::Body::empty())
        .unwrap();
    let (first, second) = tokio::join!(router.clone().oneshot(first), router.oneshot(second));
    assert_eq!(body(first.unwrap()).await, TRACE);
    assert_eq!(
        body(second.unwrap()).await,
        "33333333333333333333333333333333"
    );
    telemetry.shutdown();
    let mut reader = df_telemetry::DiagnosticReader::open(&root.0).unwrap();
    assert!(reader.verify_write_denied().unwrap());
    for trace in [[0x11; 16], [0x33; 16]] {
        let page = reader
            .query(
                &df_telemetry::QueryFilter {
                    trace: Some(trace),
                    ..Default::default()
                },
                0,
                100,
            )
            .unwrap();
        assert_eq!(page.records.len(), 5);
        assert_eq!(
            page.records
                .iter()
                .filter(|record| record.signal == Signal::Spans)
                .count(),
            1
        );
        for event in [
            "test.before",
            "test.after",
            "test.dispatched",
            "hosted.request",
            "df-portal-hosted",
        ] {
            assert!(
                page.records
                    .iter()
                    .any(|record| contains(&record.otlp, event)),
                "missing {event}"
            );
        }
        for secret in [
            "person@example.com",
            "token=secret",
            "?secret",
            "synthetic-native",
        ] {
            assert!(
                !page
                    .records
                    .iter()
                    .any(|record| contains(&record.otlp, secret)),
                "leaked {secret}"
            );
        }
    }
    drop(reader);
    drop(telemetry);
}

#[test]
fn global_shared_operation_spans_reach_sqlite_without_private_payloads() {
    use opentelemetry::trace::{Span, Tracer};
    let root = Root::new();
    let telemetry = HostedTelemetry::open(Some(&root.0));
    let mut operation = df_observe::begin(
        &df_observe::OperationContext {
            trace_parent: TRACEPARENT.to_owned(),
            build: "private-build-token".to_owned(),
        },
        "test.operation",
    );
    operation.event(
        "test.private",
        &[("email", "person@example.com".to_owned())],
    );
    operation.finish_unmeasured("complete");
    drop(operation);
    let context = SpanContext::new(
        TraceId::from_bytes([0x11; 16]),
        SpanId::from_bytes([0x22; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::from_key_value([("private", "credential-sentinel-private")]).unwrap(),
    );
    let mut direct = opentelemetry::global::tracer("private-scope").start_with_context(
        "private-name",
        &Context::new().with_remote_span_context(context.clone()),
    );
    direct.add_link(context, vec![KeyValue::new("email", "person@example.com")]);
    direct.end();
    telemetry.shutdown();
    let mut reader = df_telemetry::DiagnosticReader::open(&root.0).unwrap();
    let page = reader
        .query(
            &df_telemetry::QueryFilter {
                trace: Some([0x11; 16]),
                ..Default::default()
            },
            0,
            100,
        )
        .unwrap();
    assert_eq!(page.records.len(), 2);
    assert_eq!(page.records[0].signal, Signal::Spans);
    assert!(contains(&page.records[0].otlp, "hosted.operation"));
    assert!(contains(&page.records[0].otlp, "hosted.event"));
    assert!(contains(&page.records[0].otlp, "event.payload_redacted"));
    assert!(!contains(&page.records[0].otlp, "person@example.com"));
    assert!(!contains(&page.records[0].otlp, "private-build-token"));
    for secret in [
        "credential-sentinel-private",
        "private-scope",
        "private-name",
        "person@example.com",
    ] {
        assert!(
            !page
                .records
                .iter()
                .any(|record| contains(&record.otlp, secret)),
            "leaked {secret}"
        );
    }
    drop(reader);
    drop(telemetry);
}

#[test]
fn disabled_audit_pipeline_emits_redacted_independent_fallback_instead_of_losing_logs() {
    let telemetry = HostedTelemetry::disabled();
    telemetry.record_audit(
        "test.disabled",
        "AUTH email=person@example.com token=secret",
        None,
    );
    assert_eq!(telemetry.fallback_events.load(Ordering::Relaxed), 1);
    assert_eq!(telemetry.failures(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthy_parallel_requests_export_every_log_and_span_without_contention_loss() {
    let ingress = Arc::new(Collected::default());
    let telemetry = pipeline(ingress.clone());
    let router = app(telemetry.clone());
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let router = router.clone();
        requests.spawn(async move { router.oneshot(request("/app/items/x")).await.unwrap() });
    }
    while let Some(response) = requests.join_next().await {
        assert_eq!(response.unwrap().status(), StatusCode::OK);
    }
    assert_eq!(ingress.bytes.lock().unwrap().len(), 64);
    assert_eq!(telemetry.failures(), 0);
}

#[test]
fn emergency_fallback_keeps_only_validated_correlation_identifiers() {
    let context = parse_traceparent(TRACEPARENT).unwrap();
    let line = fallback_line("test.failure", Some(&context));
    assert!(line.contains(&format!("trace={TRACE}")));
    assert!(line.contains("span=2222222222222222"));
    assert!(line.contains("payload=redacted"));
    let invalid = SpanContext::empty_context();
    assert!(fallback_line("test.failure", Some(&invalid)).contains("trace=none span=none"));
}
