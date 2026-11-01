//! Actual PostgreSQL host, supplier fixture and SDK/Store observation acceptance.
use super::*;
use crate::gameplay::{
    image_analysis_runtime_tests::{DurableCosts, Supplier, analysis_request, complete, patch},
    image_test_support::ImageHostFixture,
    imagery::{ImageKey, Lookup, ReferenceImage, analysis},
};
use df_imagery::{
    AssetId, BackendId, GlobalMode, ImageAnalysisOperation, ImageDemandView, ImageJobId,
    PrivateImageText, RequestOutcome, RuntimePatch, SegmentationPreparation, SegmentationPrompt,
};

const WAIT: Duration = Duration::from_secs(5);
const PRIVATE: &str = "private-analysis-description-secret-sentinel";

struct Capture {
    root: PathBuf,
    store: df_telemetry::Store,
    native: Arc<NativeDiagnosticSink>,
    captured: Arc<CapturingSink>,
    observer: Arc<SharedImageObserver>,
}
fn capture() -> Capture {
    let root = root();
    let limits = TelemetryLimits::default();
    let store = df_telemetry::Store::open_configured(&root, limits).unwrap();
    let build = df_types::BuildIdentity::new(
        Some("actual-host-observation"),
        Some("native-fixture"),
        Some("not-applicable"),
        Some("actual-host-config"),
        Some("synthetic-pixels"),
    )
    .unwrap();
    let producer = NativeProducer::for_service(
        ProducerId::new([31; 16]).unwrap(),
        SourceSequence::new(1).unwrap(),
        &build,
        store.ingress(),
        limits,
        "actual-image-host-fixture",
    )
    .unwrap();
    let native = Arc::new(NativeDiagnosticSink::new(producer, 256, next_record).unwrap());
    let captured = Arc::new(CapturingSink {
        shared: native.clone(),
        records: Mutex::default(),
    });
    let observer = Arc::new(SharedImageObserver::new(captured.clone()));
    observer.set_level(ImageLogLevel::Debug);
    Capture {
        root,
        store,
        native,
        captured,
        observer,
    }
}
async fn located_head(host: &ImageHostFixture, reference: &ReferenceImage) -> ImageDemandView {
    let context = host.imagery().consumer_context().await.unwrap();
    let mut request = analysis_request(
        &context,
        reference.canonical_source().unwrap().clone(),
        0x79,
    );
    let ImageAnalysisOperation::HeadLocations(spec) = &mut request.operation else {
        unreachable!()
    };
    spec.targets[0].description = PrivateImageText::new(PRIVATE);
    request.identity.fingerprint = context
        .service
        .canonical_analysis_fingerprint(&request)
        .unwrap();
    let accepted = match context
        .service
        .request_analysis(&context.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Recorded(view) => view,
        other => panic!("new actual analysis demand expected: {other:?}"),
    };
    let head = complete(&context, &accepted.handle).await;
    assert!(head.analysis_result.is_some());
    head
}
async fn empty_staging(host: &mut ImageHostFixture, key: &ImageKey, costs: &DurableCosts) {
    let segmentation = analysis::segments(
        host.imagery(),
        key,
        SegmentationPreparation::ScenePlate,
        vec![SegmentationPrompt::Person, SegmentationPrompt::Head],
        8,
    )
    .await
    .unwrap();
    assert!(
        segmentation
            .value
            .iter()
            .all(|set| set.instances.is_empty())
    );
    let calls = costs.port.totals().calls;
    assert_eq!(calls, 3);
    {
        let mut control = host.control().lock().unwrap();
        let revision = control.read_effective().provenance.revision;
        let mut patch = RuntimePatch::empty();
        patch.mode = Some(GlobalMode::LocalOnly);
        patch.backend_enabled = [Some(false); BackendId::COUNT];
        control.apply(patch, revision).unwrap();
    }
    host.restart().await;
    let identities = analysis::identity(host.imagery(), &segmentation, None, &[])
        .await
        .unwrap();
    assert!(identities.value.is_empty());
    let facings = analysis::facing(host.imagery(), &segmentation, &identities)
        .await
        .unwrap();
    assert!(facings.value.is_empty());
    assert_eq!(costs.port.totals().calls, calls);
}
async fn exercise(
    host: &mut ImageHostFixture,
    costs: &DurableCosts,
    observer: &SharedImageObserver,
) -> (ImageKey, ImageJobId, AssetId) {
    let (key, reference) = host
        .import_bytes(df_providers::ImageFormat::Png, source_png())
        .await
        .unwrap();
    assert!(matches!(
        host.imagery().image(key.as_str()).await,
        Lookup::Image { .. }
    ));
    let head = located_head(host, &reference).await;
    empty_staging(host, &key, costs).await;
    let context = host.imagery().consumer_context().await.unwrap();
    assert_eq!(
        complete(&context, &head.handle).await.analysis_result,
        head.analysis_result
    );
    context
        .service
        .cancel(&context.principal, &head.handle)
        .await
        .unwrap();
    drop(context);
    assert!(matches!(
        host.imagery().image(key.as_str()).await,
        Lookup::Image { .. }
    ));
    let identity =
        df_observe::presentation::local_identity(&format!("/gameplay/imagery/{}", key.as_str()));
    let publication = host
        .imagery()
        .displayed_observation(identity)
        .await
        .unwrap()
        .unwrap();
    displayed(
        observer,
        &publication,
        &TimingEvent::lifecycle("plate-shown", "ready", identity, 17, 321),
        ProducerId::new([32; 16]).unwrap(),
    );
    (key, head.job, publication.asset)
}
fn has(record: &CapturedRecord, key: &str, value: &AnyValue) -> bool {
    record
        .attributes
        .iter()
        .any(|(name, actual)| name == key && actual == value)
}
fn assert_lifecycle(records: &[CapturedRecord], job: ImageJobId, asset: AssetId) {
    for name in [
        "image.requested",
        "image.queued",
        "image.claimed",
        "image.admitted",
        "image.sent",
        "image.usage_recorded",
        "image.decision_checkpointed",
        "image.stage_completed",
        "image.settlement_recorded",
        "image.usage_projected",
        "image.analysis_completed",
        "image.local_analysis_completed",
        "image.transform_completed",
        "image.output_stored",
        "image.published",
        "image.configuration_changed",
        "image.delivery_authorized",
        "image.displayed",
        "image.cancelled",
    ] {
        assert!(
            records.iter().any(|record| record.event_name == name),
            "missing {name}"
        );
    }
    let job = AnyValue::String(job.to_hex().into());
    let sent = records
        .iter()
        .find(|record| record.event_name == "image.sent" && has(record, "image.job_id", &job))
        .unwrap();
    let attempt = sent
        .attributes
        .iter()
        .find(|(key, _)| key == "image.attempt_id")
        .unwrap();
    let stage = sent
        .attributes
        .iter()
        .find(|(key, _)| key == "image.stage_id")
        .unwrap();
    let supplier = records
        .iter()
        .find(|record| {
            record.event_name == "image.supplier_completed" && has(record, "image.job_id", &job)
        })
        .unwrap();
    assert_duration(supplier);
    for name in [
        "image.sent",
        "image.usage_recorded",
        "image.decision_checkpointed",
        "image.stage_completed",
        "image.settlement_recorded",
        "image.usage_projected",
        "image.analysis_completed",
        "image.cancelled",
    ] {
        assert!(
            records.iter().any(|record| record.event_name == name
                && has(record, "image.job_id", &job)
                && (name == "image.cancelled"
                    || (record.attributes.contains(attempt) && record.attributes.contains(stage)))),
            "missing actual head correlation for {name}"
        );
    }
    assert!(records.iter().any(|record| {
        record.event_name == "image.delivery_authorized"
            && has(
                record,
                "image.asset_id",
                &AnyValue::String(asset.to_hex().into()),
            )
            && record
                .attributes
                .iter()
                .all(|(key, _)| key != "image.job_id")
    }));
    assert_local_analysis(records);
    assert!(!format!("{records:?}").contains(PRIVATE));
}
fn assert_local_analysis(records: &[CapturedRecord]) {
    let local = records
        .iter()
        .filter(|record| record.event_name == "image.local_analysis_completed")
        .collect::<Vec<_>>();
    assert_eq!(local.len(), 2);
    for record in local {
        assert_duration(record);
        assert!(has(
            record,
            "image.supplier_dispatched",
            &AnyValue::Boolean(false)
        ));
        assert!(has(
            record,
            "image.raw_checkpointed",
            &AnyValue::Boolean(false)
        ));
        assert!(record.attributes.iter().all(|(key, _)| !matches!(
            key.as_str(),
            "image.attempt_id"
                | "image.catalog_attempt_id"
                | "image.cost_call_id"
                | "image.backend"
        )));
    }
}

fn assert_duration(record: &CapturedRecord) {
    assert!(has(
        record,
        "image.clock_domain",
        &AnyValue::String("native_monotonic".into())
    ));
    assert!(
        record
            .attributes
            .iter()
            .any(|(key, value)| key == "image.duration_ns"
                && matches!(value, AnyValue::Int(duration) if *duration >= 0))
    );
}

#[tokio::test]
async fn actual_pg_host_lifecycle_and_no_call_analysis_reach_shared_store_without_private_content()
{
    let mut capture = capture();
    let supplier = Supplier::open().await;
    let costs = DurableCosts::open();
    let Some(mut host) = ImageHostFixture::open_with_observer(
        supplier.address(),
        patch(&supplier),
        costs.port.clone(),
        capture.observer.clone(),
    )
    .await
    else {
        // Required PG absence fails in the fixture when REQUIRE_SCRATCH_PG is set.
        supplier.close().await;
        capture.native.shutdown(WAIT).unwrap();
        capture.store.shutdown(WAIT).unwrap();
        return;
    };
    let (key, job, asset) = exercise(&mut host, &costs, capture.observer.as_ref()).await;
    assert_eq!(capture.observer.health().failures, 0);
    capture.native.shutdown(WAIT).unwrap();
    let admitted = capture.native.health().admitted;
    assert_eq!(capture.native.health().confirmed, admitted);
    assert_eq!(capture.native.health().refused, 0);
    assert_lifecycle(&capture.captured.records.lock().unwrap(), job, asset);
    // Shared diagnostics failure cannot turn currently authorized bytes into failure.
    assert!(matches!(
        host.imagery().image(key.as_str()).await,
        Lookup::Image { .. }
    ));
    assert!(capture.observer.health().failures > 0);
    host.close().await;
    supplier.close().await;
    capture.store.shutdown(WAIT).unwrap();
    drop(capture.store);
    let records = committed_records(&capture.root);
    assert_eq!(records.len() as u64, admitted);
    assert!(records.iter().all(|record| {
        !record
            .otlp
            .windows(PRIVATE.len())
            .any(|bytes| bytes == PRIVATE.as_bytes())
    }));
}
