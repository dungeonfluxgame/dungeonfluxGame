use super::*;
use df_imagery::telemetry::{ImageLogLevel, SharedImageObserver};
use df_observe::{
    AnyValue, DiagnosticRecord, DiagnosticSink, NativeDiagnosticSink, NativeProducer,
    SourceSequence, TelemetryError, TelemetryLimits,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

#[path = "image_observability_host_tests.rs"]
mod host_tests;

struct CapturingSink {
    shared: Arc<NativeDiagnosticSink>,
    records: Mutex<Vec<CapturedRecord>>,
}

#[derive(Debug)]
struct CapturedRecord {
    event_name: &'static str,
    attributes: Vec<(String, AnyValue)>,
}

impl DiagnosticSink for CapturingSink {
    fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
        let captured = CapturedRecord {
            event_name: record.event_name,
            attributes: record.input.attributes.clone(),
        };
        self.shared.emit(record)?;
        self.records.lock().unwrap().push(captured);
        Ok(())
    }
}

fn next_record() -> [u8; 16] {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    u128::from(NEXT.fetch_add(1, Ordering::Relaxed)).to_be_bytes()
}

fn root() -> PathBuf {
    let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("artifacts/tmp/imagery-finish-20261029/logging/consumer-observability-fixtures");
    std::fs::create_dir_all(&parent).unwrap();
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    parent.join(format!("consumer-{}-{nonce}", std::process::id()))
}

fn source_png() -> bytes::Bytes {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[70, 80, 90, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes.into()
}

fn committed_records(root: &Path) -> Vec<df_telemetry::RecordView> {
    let mut reader = df_telemetry::DiagnosticReader::open(root).unwrap();
    assert!(reader.verify_write_denied().unwrap());
    let mut records = Vec::new();
    let mut cursor = 0;
    loop {
        let page = reader
            .query(&df_telemetry::QueryFilter::default(), cursor, 100)
            .unwrap();
        assert!(
            page.source_watermarks
                .iter()
                .all(|source| source.missing_through_highest == 0)
        );
        if page.records.is_empty() {
            return records;
        }
        assert!(page.next_cursor > cursor);
        cursor = page.next_cursor;
        records.extend(page.records);
    }
}

#[tokio::test]
async fn actual_central_delivery_and_browser_report_reach_shared_store_without_private_content() {
    const WAIT: Duration = Duration::from_secs(5);
    const PRIVATE: &str = "private-image-subject-sentinel";
    let root = root();
    let limits = TelemetryLimits::default();
    let mut store = df_telemetry::Store::open_configured(&root, limits).unwrap();
    let build = df_types::BuildIdentity::new(
        Some("image-consumer-observation"),
        Some("native-fixture"),
        Some("not-applicable"),
        Some("local-only-fixture"),
        Some("synthetic-pixels"),
    )
    .unwrap();
    let producer = NativeProducer::for_service(
        ProducerId::new([1; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build,
        store.ingress(),
        limits,
        "image-consumer-fixture",
    )
    .unwrap();
    let native = Arc::new(NativeDiagnosticSink::new(producer, 128, next_record).unwrap());
    let captured = Arc::new(CapturingSink {
        shared: native.clone(),
        records: Mutex::default(),
    });
    let observer = Arc::new(SharedImageObserver::new(captured.clone()));
    observer.set_level(ImageLogLevel::Debug);
    configuration_changed(observer.as_ref(), 1);
    let fixture =
        super::super::imagery::test_support::TransformFixture::with_observer(observer.clone())
            .await;
    let source = fixture
        .reference(df_providers::ImageFormat::Png, source_png())
        .await;
    let key = fixture
        .imagery()
        .register_reviewed(PRIVATE, source.clone())
        .await
        .unwrap();
    let identity =
        df_observe::presentation::local_identity(&format!("/gameplay/imagery/{}", key.as_str()));
    let publication = fixture
        .imagery()
        .displayed_observation(identity)
        .await
        .unwrap()
        .unwrap();
    let report = TimingEvent::lifecycle("plate-shown", "ready", identity, 17, 321);
    let wire = df_observe::presentation::encode(&[report]).unwrap();
    let reports = df_observe::presentation::decode(&wire).unwrap();
    displayed(
        observer.as_ref(),
        &publication,
        &reports[0],
        ProducerId::new([2; 16]).unwrap(),
    );
    assert_eq!(publication.config_revision, None);
    assert_eq!(observer.health().failures, 0);
    native.shutdown(WAIT).unwrap();
    let admitted = native.health().admitted;
    assert_eq!(native.health().confirmed, admitted);
    assert_eq!(native.health().pending, 0);
    assert_eq!(native.health().refused, 0);
    {
        let records = captured.records.lock().unwrap();
        for name in [
            "image.configuration_changed",
            "image.requested",
            "image.queued",
            "image.transform_started",
            "image.transform_completed",
            "image.output_stored",
            "image.stage_completed",
            "image.published",
            "image.completed",
            "image.delivery_authorized",
            "image.displayed",
        ] {
            assert!(
                records.iter().any(|record| record.event_name == name),
                "missing {name}"
            );
        }
        let terminal = &records
            .iter()
            .find(|record| record.event_name == "image.stage_completed")
            .unwrap()
            .attributes;
        assert!(
            terminal
                .iter()
                .any(|(name, value)| name == "image.stage_outcome"
                    && value == &AnyValue::String("output_stored".into()))
        );
        assert!(
            terminal
                .iter()
                .any(|(name, value)| name == "image.stage_kind"
                    && value == &AnyValue::String("local_transform".into()))
        );
        assert!(
            records
                .iter()
                .filter(|record| record.event_name == "image.transform_completed")
                .any(|record| record
                    .attributes
                    .iter()
                    .any(|(name, value)| name == "image.duration_ns"
                        && matches!(value, AnyValue::Int(duration) if *duration >= 0)))
        );
        let job = terminal
            .iter()
            .find(|(name, _)| name == "image.job_id")
            .unwrap();
        for name in [
            "image.queued",
            "image.published",
            "image.delivery_authorized",
            "image.displayed",
        ] {
            assert!(
                records
                    .iter()
                    .filter(|record| record.event_name == name)
                    .any(|record| record.attributes.contains(job)),
                "uncorrelated {name}"
            );
        }
        for record in records.iter().filter(|record| {
            record.attributes.iter().any(|(name, value)| {
                name == "image.stage_kind" && value == &AnyValue::String("local_transform".into())
            })
        }) {
            assert!(
                record
                    .attributes
                    .iter()
                    .all(|(name, _)| name != "image.cost_call_id" && name != "image.backend")
            );
        }
        let display = &records
            .iter()
            .find(|record| record.event_name == "image.displayed")
            .unwrap()
            .attributes;
        assert!(
            display
                .iter()
                .any(|(name, value)| name == "image.browser_at_ms" && value == &AnyValue::Int(321))
        );
        assert!(display.iter().all(|(name, _)| name != "image.delivery_id"));
        assert!(
            display
                .iter()
                .all(|(name, _)| name != "image.config_revision")
        );
        let delivery = &records
            .iter()
            .find(|record| record.event_name == "image.delivery_authorized")
            .unwrap()
            .attributes;
        assert!(delivery.iter().any(|(name, _)| name == "image.delivery_id"));
        assert!(!format!("{records:?}").contains(PRIVATE));
    }
    // Logging after closure must not turn a valid replayed image into an unavailable result.
    assert_eq!(
        fixture
            .imagery()
            .register_reviewed(PRIVATE, source)
            .await
            .unwrap(),
        key
    );
    assert!(observer.health().failures > 0);
    fixture.close().await;
    store.shutdown(WAIT).unwrap();
    drop(store);
    let records = committed_records(&root);
    assert_eq!(records.len() as u64, admitted);
    for name in [
        "image.configuration_changed",
        "image.delivery_authorized",
        "image.displayed",
    ] {
        assert!(records.iter().any(|record| {
            record
                .otlp
                .windows(name.len())
                .any(|bytes| bytes == name.as_bytes())
        }));
    }
    assert!(records.iter().all(|record| {
        !record
            .otlp
            .windows(PRIVATE.len())
            .any(|bytes| bytes == PRIVATE.as_bytes())
    }));
}

#[test]
fn loaded_decoded_actor_or_requested_events_are_not_image_display_acknowledgments() {
    for kind in [
        "plate-requested",
        "plate-loaded",
        "plate-decoded",
        "figure-shown",
        "visual-source",
    ] {
        assert!(
            displayed_asset_identity(&TimingEvent::lifecycle(kind, "ready", 1, 2, 3)).is_none()
        );
    }
    assert_eq!(
        displayed_asset_identity(&TimingEvent::lifecycle("plate-shown", "ready", 1, 2, 3)),
        Some(1)
    );
}

#[test]
fn delivery_without_entropy_preserves_actual_asset_without_inventing_job_or_display() {
    #[derive(Default)]
    struct Collected(Mutex<Vec<DiagnosticRecord>>);
    impl DiagnosticSink for Collected {
        fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
            self.0.lock().unwrap().push(record);
            Ok(())
        }
    }
    let sink = Arc::new(Collected::default());
    let observer = SharedImageObserver::new(sink.clone());
    let asset = df_imagery::AssetId::from_bytes(&[21; 16]).unwrap();
    delivery_authorized(
        &observer,
        df_imagery::telemetry::EventContext {
            asset: Some(asset),
            ..Default::default()
        },
        None,
    );
    let records = sink.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event_name, "image.delivery_authorized");
    let attributes = &records[0].input.attributes;
    assert!(
        attributes
            .iter()
            .any(|(name, value)| name == "image.asset_id"
                && value == &AnyValue::String(asset.to_hex().into()))
    );
    assert!(
        attributes
            .iter()
            .any(|(name, value)| name == "image.delivery_identified"
                && value == &AnyValue::Boolean(false))
    );
    assert!(attributes.iter().all(|(name, _)| !matches!(
        name.as_str(),
        "image.job_id" | "image.delivery_id" | "image.config_revision" | "df.operation"
    )));
    assert_eq!(observer.health().failures, 0);
}

#[test]
fn no_call_analysis_and_supplier_settlement_keep_distinct_correlated_records() {
    use df_imagery::{
        execution::{DispatchId, DispatchSettlement, ExecutionStageId, StageKind},
        telemetry::{
            AnalysisKind, AnalysisVerdict, EventContext, EventKind, ImageEvent, StageOutcome,
        },
    };
    const WAIT: Duration = Duration::from_secs(5);
    let root = root();
    let limits = TelemetryLimits::default();
    let mut store = df_telemetry::Store::open_configured(&root, limits).unwrap();
    let build = df_types::BuildIdentity::new(
        Some("analysis-contract-fixture"),
        Some("native-fixture"),
        Some("not-applicable"),
        Some("fixture-config"),
        Some("synthetic-content"),
    )
    .unwrap();
    let producer = NativeProducer::for_service(
        ProducerId::new([11; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build,
        store.ingress(),
        limits,
        "analysis-contract-fixture",
    )
    .unwrap();
    let native = Arc::new(NativeDiagnosticSink::new(producer, 16, next_record).unwrap());
    let captured = Arc::new(CapturingSink {
        shared: native.clone(),
        records: Mutex::default(),
    });
    let observer = SharedImageObserver::new(captured.clone());
    let parent = DispatchId::from_bytes(&[12; 16]).unwrap();
    let local = EventContext {
        operation: Some(df_types::OperationId::from_bytes(&[13; 16]).unwrap()),
        job: Some(df_imagery::ImageJobId::from_bytes(&[14; 16]).unwrap()),
        stage: Some(ExecutionStageId::from_bytes(&[15; 16]).unwrap()),
        parent_attempt: Some(parent),
        kind: Some(StageKind::StagingIdentity),
        analysis_kind: Some(AnalysisKind::StagingIdentity),
        config_revision: Some(3),
        gate_revision: Some(5),
        ..EventContext::default()
    };
    let mut no_call = ImageEvent::new(EventKind::LocalAnalysisCompleted, local.clone());
    no_call.measurements.supplier_dispatched = Some(false);
    no_call.measurements.raw_checkpointed = Some(false);
    no_call.measurements.analysis_verdict = Some(AnalysisVerdict::Empty);
    observer.observe(no_call);
    let supplier = EventContext {
        attempt: Some(DispatchId::from_bytes(&[16; 16]).unwrap()),
        kind: Some(StageKind::PhotoInspection),
        analysis_kind: Some(AnalysisKind::PhotoInspection),
        ..local
    };
    let mut analyzed = ImageEvent::new(EventKind::AnalysisCompleted, supplier.clone());
    analyzed.measurements.supplier_dispatched = Some(true);
    analyzed.measurements.raw_checkpointed = Some(true);
    analyzed.measurements.analysis_verdict = Some(AnalysisVerdict::Unjudged);
    observer.observe(analyzed);
    let mut completed = ImageEvent::new(EventKind::StageCompleted, supplier.clone());
    completed.measurements.stage_outcome = Some(StageOutcome::Unknown);
    observer.observe(completed);
    let mut settlement = ImageEvent::new(EventKind::SettlementRecorded, supplier);
    settlement.measurements.settlement = Some(DispatchSettlement::UnknownOutstanding);
    settlement.measurements.usage_evidence = Some(df_imagery::cost::UsageEvidence::Unknown);
    settlement.measurements.usage_revision = Some(df_imagery::cost::UsageRevision(1));
    observer.observe(settlement);
    native.shutdown(WAIT).unwrap();
    assert_eq!(native.health().confirmed, 4);
    let records = captured.records.lock().unwrap();
    let local = &records
        .iter()
        .find(|record| record.event_name == "image.local_analysis_completed")
        .unwrap()
        .attributes;
    assert!(local.iter().all(|(name, _)| !matches!(
        name.as_str(),
        "image.attempt_id" | "image.cost_call_id" | "image.backend"
    )));
    assert!(
        local
            .iter()
            .any(|(name, value)| name == "image.parent_attempt_id"
                && value == &AnyValue::String(parent.to_hex().into()))
    );
    assert!(
        local
            .iter()
            .any(|(name, value)| name == "image.supplier_dispatched"
                && value == &AnyValue::Boolean(false))
    );
    let supplier = &records
        .iter()
        .find(|record| record.event_name == "image.analysis_completed")
        .unwrap()
        .attributes;
    assert!(
        supplier
            .iter()
            .any(|(name, value)| name == "image.raw_checkpointed"
                && value == &AnyValue::Boolean(true))
    );
    assert!(
        supplier
            .iter()
            .any(|(name, value)| name == "image.analysis_verdict"
                && value == &AnyValue::String("unjudged".into()))
    );
    assert!(
        supplier
            .iter()
            .any(|(name, value)| name == "image.config_revision" && value == &AnyValue::Int(3))
    );
    assert!(
        supplier
            .iter()
            .any(|(name, value)| name == "image.gate_revision" && value == &AnyValue::Int(5))
    );
    assert!(
        records
            .iter()
            .find(|record| record.event_name == "image.stage_completed")
            .unwrap()
            .attributes
            .iter()
            .any(|(name, value)| name == "image.stage_outcome"
                && value == &AnyValue::String("unknown".into()))
    );
    let cost = supplier
        .iter()
        .find(|(name, _)| name == "image.cost_call_id")
        .unwrap();
    let settlement = &records
        .iter()
        .find(|record| record.event_name == "image.settlement_recorded")
        .unwrap()
        .attributes;
    assert!(settlement.contains(cost));
    assert!(
        settlement
            .iter()
            .any(|(name, value)| name == "image.settlement"
                && value == &AnyValue::String("unknown_outstanding".into()))
    );
    drop(records);
    store.shutdown(WAIT).unwrap();
    drop(store);
    let mut reader = df_telemetry::DiagnosticReader::open(&root).unwrap();
    let page = reader
        .query(&df_telemetry::QueryFilter::default(), 0, 16)
        .unwrap();
    assert_eq!(page.records.len(), 4);
    assert_eq!(page.source_watermarks[0].missing_through_highest, 0);
}
