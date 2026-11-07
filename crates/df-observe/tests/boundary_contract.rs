//! Private whole-Original contract ledger. These candidates grant no application authority.
use df_observe::{
    CaptureKey, OperationContext, ProducerId, RecordId, SourceSequence, TelemetryError,
};
use opentelemetry_proto::tonic::{
    collector::metrics::v1::{
        ExportMetricsPartialSuccess, ExportMetricsServiceRequest, ExportMetricsServiceResponse,
    },
    common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value::Value},
    metrics::v1::{
        AggregationTemporality, Exemplar, ExponentialHistogram, ExponentialHistogramDataPoint,
        Gauge, Histogram, HistogramDataPoint, Metric, MetricsData, NumberDataPoint,
        ResourceMetrics, ScopeMetrics, Sum, Summary, SummaryDataPoint, exemplar,
        exponential_histogram_data_point, metric, number_data_point, summary_data_point,
    },
    resource::v1::Resource,
};
use prost::Message;

// One fixed ledger for this aggregate; no public schema, runtime registry, or new graph owner.
// Columns: Original role, canonical shape, owner, exact consumer hook, limit/authority.
const CONTRACT: &[(&str, &str, &str, &str, &str)] = &[
    (
        "context",
        "df_observe::OperationContext",
        "dispatch caller",
        "df-rpc-bridge/src/browser.rs driver; df-tools fixtures; df-assets operations",
        "trace_parent/build are correlation only; df-auth authorizes separately",
    ),
    (
        "resource/scope/attributes",
        "OTLP Resource/InstrumentationScope/KeyValue/AnyValue",
        "NativeProducer or BrowserBuffer",
        "OTLP requests -> df-telemetry wire -> RecordView.otlp",
        "supplied BuildIdentity labels; no tested-build or rights proof",
    ),
    (
        "producer/record/sequence",
        "ProducerId/RecordId/SourceSequence/CaptureKey",
        "capture caller",
        "NativeProducer::reserve; Store dedupe; QueryPage source_watermarks",
        "nonzero distinct identities; positive checked i64 sequence; retry original bytes",
    ),
    (
        "log/span/link",
        "OTLP LogRecord/Span/Span.Link",
        "NativeProducer",
        "emit_log/emit_span -> LocalIngress -> Store -> DiagnosticReader",
        "typed values, source/observed time and unsampled correlation; no privacy sanitizer",
    ),
    (
        "metric",
        "OTLP MetricsData/ResourceMetrics/ScopeMetrics/Metric data variants",
        "opentelemetry-proto0.31.0 canonical library",
        "private fixture prost roundtrip only",
        "application producer, labels, exporter and collector absent; Signal is Logs/Spans",
    ),
    (
        "bounded TelemetryBatch",
        "CapturedBatch/TelemetryLimits/OTLP request; Store private Batch",
        "producer transient bytes then Store queue/writer",
        "TelemetryIngress::submit",
        "8192B record;65536B/32 batch;1024 items/1MiB queue; no new public batch wrapper",
    ),
    (
        "stage/watermark/gaps",
        "PendingReceipt/IngestReceipt; QueryPage/SourceWatermark",
        "Store writer; DiagnosticReader read-only snapshot",
        "wait/try_receive; reader.query",
        "pending is unconfirmed; Spooled/Committed explicit; gap/retention visible",
    ),
    (
        "emergency",
        "classified native emergency bytes; QueryPage emergency linkage",
        "Store emergency writer",
        "df-telemetry emergency tests -> DiagnosticReader",
        "independent of OTEL;32 records/8192B then stderr; no shared record DTO",
    ),
    (
        "stream/cleanup",
        "OperationSpan; NativeProducer; Store; DiagnosticReader",
        "explicit span/exporter/writer/reader owner",
        "finish/drop; shutdown finite joins; query",
        "request cancellation cannot undo accepted writer work; pure result has no SDK",
    ),
    (
        "browser ownership",
        "BrowserBuffer/UploadLease/BufferSnapshot/BrowserLog",
        "one page/binding generation",
        "retain_log/upload_records/acknowledge_durable local seam",
        "100000 records/64MiB;32/64KiB prefix;60s; owner/generation/attempt/prefix fences",
    ),
    (
        "normal/failure/rights/privacy/recovery",
        "existing caller policy and typed telemetry outcomes",
        "service/auth/rights caller; native storage owner",
        "commercial-validation.md: Offer, activation and launch evidence; Rights-complete launch gate; Reproducible economics and cash; Joint offer-cost qualification; D01-D04",
        "caller owns consent/audience/rights and guest recovery; no trace authentication, clearance, private content or market claim",
    ),
];

#[test]
fn whole_original_ledger_freezes_current_owners_and_explicit_candidate_limits() {
    // Compile against the current context and distinct capture identities, not copied wrappers.
    let context = OperationContext {
        trace_parent: String::new(),
        build: "fixture".into(),
    };
    assert!(context.trace_parent.is_empty());
    let key = CaptureKey {
        producer: ProducerId::new([1; 16]).unwrap(),
        record: RecordId::new([2; 16]).unwrap(),
        sequence: SourceSequence::new(1).unwrap(),
    };
    assert_eq!(
        ProducerId::from_hex(&key.producer.hex()).unwrap(),
        key.producer
    );
    assert_eq!(RecordId::from_hex(&key.record.hex()).unwrap(), key.record);
    assert_eq!(SourceSequence::new(0), Err(TelemetryError::InvalidIdentity));
    assert_eq!(
        ProducerId::new([0; 16]),
        Err(TelemetryError::InvalidIdentity)
    );
    assert_eq!(
        RecordId::from_hex(&"A".repeat(32)),
        Err(TelemetryError::InvalidIdentity)
    );
    for (index, row) in CONTRACT.iter().enumerate() {
        assert!(!CONTRACT[..index].iter().any(|other| other.0 == row.0));
        assert!(!row.1.is_empty() && !row.2.is_empty() && !row.3.is_empty() && !row.4.is_empty());
        println!(
            "contract role={} shape={} owner={} hook={} limit={}",
            row.0, row.1, row.2, row.3, row.4
        );
    }
}

#[test]
fn canonical_otlp_metric_candidates_preserve_typed_values_causality_and_aggregation() {
    let exemplar = Exemplar {
        time_unix_nano: 19,
        trace_id: vec![1; 16],
        span_id: vec![2; 8],
        value: Some(exemplar::Value::AsInt(i64::MIN)),
        ..Default::default()
    };
    let number = NumberDataPoint {
        start_time_unix_nano: 10,
        time_unix_nano: 20,
        exemplars: vec![exemplar.clone()],
        value: Some(number_data_point::Value::AsInt(i64::MAX)),
        ..Default::default()
    };
    let variants = vec![
        metric::Data::Gauge(Gauge {
            data_points: vec![number.clone()],
        }),
        metric::Data::Sum(Sum {
            data_points: vec![number],
            aggregation_temporality: AggregationTemporality::Delta as i32,
            is_monotonic: true,
        }),
        metric::Data::Histogram(Histogram {
            data_points: vec![HistogramDataPoint {
                start_time_unix_nano: 10,
                time_unix_nano: 20,
                count: 3,
                sum: Some(6.0),
                min: Some(1.0),
                max: Some(3.0),
                bucket_counts: vec![1, 2],
                explicit_bounds: vec![1.5],
                exemplars: vec![exemplar],
                ..Default::default()
            }],
            aggregation_temporality: AggregationTemporality::Cumulative as i32,
        }),
        metric::Data::ExponentialHistogram(ExponentialHistogram {
            data_points: vec![ExponentialHistogramDataPoint {
                start_time_unix_nano: 10,
                time_unix_nano: 20,
                count: 3,
                sum: Some(0.0),
                scale: -1,
                zero_count: 1,
                positive: Some(exponential_histogram_data_point::Buckets {
                    offset: -1,
                    bucket_counts: vec![1],
                }),
                negative: Some(exponential_histogram_data_point::Buckets {
                    offset: 1,
                    bucket_counts: vec![1],
                }),
                min: Some(-1.0),
                max: Some(1.0),
                zero_threshold: 0.5,
                ..Default::default()
            }],
            aggregation_temporality: AggregationTemporality::Delta as i32,
        }),
        metric::Data::Summary(Summary {
            data_points: vec![SummaryDataPoint {
                start_time_unix_nano: 10,
                time_unix_nano: 20,
                count: 3,
                sum: 6.0,
                quantile_values: vec![summary_data_point::ValueAtQuantile {
                    quantile: 0.5,
                    value: 2.0,
                }],
                ..Default::default()
            }],
        }),
    ];
    let resource_metrics = ResourceMetrics {
        resource: Some(Resource {
            attributes: vec![KeyValue {
                key: "service.name".into(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue("fixture-only".into())),
                }),
            }],
            ..Default::default()
        }),
        scope_metrics: vec![ScopeMetrics {
            scope: Some(InstrumentationScope {
                name: "fixture-only".into(),
                version: "0".into(),
                ..Default::default()
            }),
            metrics: variants
                .into_iter()
                .map(|data| Metric {
                    // Application metric names, units, labels and catalog remain deliberately unselected.
                    data: Some(data),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let request = ExportMetricsServiceRequest {
        resource_metrics: vec![resource_metrics],
    };
    let bytes = request.encode_to_vec();
    assert_eq!(
        ExportMetricsServiceRequest::decode(bytes.as_slice()).unwrap(),
        request
    );
    let data = MetricsData {
        resource_metrics: request.resource_metrics.clone(),
    };
    assert_eq!(
        MetricsData::decode(data.encode_to_vec().as_slice()).unwrap(),
        data
    );
    assert_eq!(
        request.resource_metrics[0].scope_metrics[0].metrics.len(),
        5
    );
    assert!(
        request.resource_metrics[0].scope_metrics[0]
            .metrics
            .iter()
            .all(|m| m.name.is_empty() && m.metadata.is_empty())
    );
}

#[test]
fn canonical_metric_response_keeps_partial_rejection_and_warning_explicit() {
    for partial_success in [
        None,
        Some(ExportMetricsPartialSuccess {
            rejected_data_points: 2,
            error_message: "fixture classified refusal".into(),
        }),
        Some(ExportMetricsPartialSuccess {
            rejected_data_points: 0,
            error_message: "fixture warning".into(),
        }),
    ] {
        let response = ExportMetricsServiceResponse { partial_success };
        assert_eq!(
            ExportMetricsServiceResponse::decode(response.encode_to_vec().as_slice()).unwrap(),
            response
        );
    }
    // This is a candidate wire shape, never a running collector or native ingestion receipt.
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_batch_queue_storage_limits_and_unconfirmed_receipts_remain_finite() {
    use df_observe::{Durability, IngestReceipt, PendingReceipt, TelemetryLimits};
    let limits = TelemetryLimits::default();
    assert_eq!(
        (
            limits.record_bytes,
            limits.batch_bytes,
            limits.batch_records
        ),
        (8192, 65536, 32)
    );
    assert_eq!((limits.queue_items, limits.queue_bytes), (1024, 1048576));
    assert_eq!(
        (limits.spool_bytes, limits.sqlite_bytes),
        (33554432, 67108864)
    );
    for invalid in [
        TelemetryLimits {
            record_bytes: 8193,
            ..limits
        },
        TelemetryLimits {
            batch_bytes: 65537,
            ..limits
        },
        TelemetryLimits {
            batch_records: 33,
            ..limits
        },
        TelemetryLimits {
            queue_items: 1025,
            ..limits
        },
        TelemetryLimits {
            queue_bytes: 1048577,
            ..limits
        },
        TelemetryLimits {
            spool_bytes: 33554433,
            ..limits
        },
        TelemetryLimits {
            sqlite_bytes: 262143,
            ..limits
        },
    ] {
        assert!(matches!(
            invalid.validate(),
            Err(TelemetryError::InvalidLimits)
        ));
    }
    let (sender, pending) = PendingReceipt::channel();
    assert_eq!(pending.try_receive().unwrap(), None);
    let spooled = IngestReceipt {
        durability: Durability::Spooled,
        spool_position: 1,
        committed_watermark: 0,
        records: 1,
    };
    sender.send(Ok(spooled)).unwrap();
    assert_eq!(pending.try_receive().unwrap(), Some(spooled));
    drop(sender);
    assert_eq!(pending.try_receive(), Err(TelemetryError::Closed));
}
