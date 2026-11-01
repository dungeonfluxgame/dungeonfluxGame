use super::*;
use df_observe::TelemetryError;
use std::sync::Mutex;

#[derive(Default)]
struct Collected(Mutex<Vec<DiagnosticRecord>>);

impl DiagnosticSink for Collected {
    fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
        self.0.lock().unwrap().push(record);
        Ok(())
    }
}

struct Failing;

impl DiagnosticSink for Failing {
    fn emit(&self, _record: DiagnosticRecord) -> Result<(), TelemetryError> {
        Err(TelemetryError::Capacity)
    }
}

fn context() -> EventContext {
    EventContext {
        operation: Some(OperationId::from_bytes(&[1; 16]).unwrap()),
        request: Some(ImageInterestId::from_bytes(&[2; 16]).unwrap()),
        job: Some(ImageJobId::from_bytes(&[3; 16]).unwrap()),
        stage: Some(ExecutionStageId::from_bytes(&[4; 16]).unwrap()),
        attempt: Some(DispatchId::from_bytes(&[5; 16]).unwrap()),
        run: Some(RunId::from_bytes(&[6; 16]).unwrap()),
        config_revision: Some(7),
        backend: Some(BackendId::CodexCli),
        kind: Some(StageKind::Generation),
        purpose: Some(Purpose::CreatorPortrait),
        ..EventContext::default()
    }
}

fn attribute<'a>(record: &'a DiagnosticRecord, key: &str) -> Option<&'a AnyValue> {
    record
        .input
        .attributes
        .iter()
        .find_map(|(name, value)| (name == key).then_some(value))
}

#[test]
fn enabled_events_without_sdk_preserve_owned_context_and_monotonic_duration() {
    let collected = Arc::new(Collected::default());
    let observer = Arc::new(SharedImageObserver::new(collected.clone()));
    observer.set_level(ImageLogLevel::Debug);
    let mut original = context();
    let event =
        ImageEvent::new(EventKind::Sent, original.clone()).elapsed(Duration::from_nanos(123_456));
    original.operation = Some(OperationId::from_bytes(&[9; 16]).unwrap());
    assert_ne!(original.operation, event.context.operation);
    let child = observer.clone();
    std::thread::spawn(move || ImageObserver::observe(child.as_ref(), event))
        .join()
        .unwrap();
    let records = collected.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.event_name, "image.sent");
    assert_eq!(
        attribute(record, "df.operation"),
        Some(&text(&"01".repeat(16)))
    );
    assert_eq!(
        attribute(record, "image.attempt_id"),
        Some(&text(&"05".repeat(16)))
    );
    assert_eq!(
        attribute(record, "image.cost_call_id"),
        Some(&text(&format!("img-{}", "05".repeat(16))))
    );
    assert_eq!(
        attribute(record, "image.duration_ns"),
        Some(&AnyValue::Int(123_456))
    );
    assert_eq!(
        attribute(record, "image.clock_domain"),
        Some(&text("native_monotonic"))
    );
    assert!(record.input.trace.is_none());
    assert_eq!(observer.health().admitted, 1);
}

#[test]
fn logging_selection_is_readable_and_refusal_never_returns_a_generation_error() {
    let observer = SharedImageObserver::new(Arc::new(Failing));
    ImageObserver::observe(&observer, ImageEvent::new(EventKind::Sent, context()));
    assert_eq!(observer.health().failures, 1);
    observer.set_level(ImageLogLevel::Off);
    ImageObserver::observe(&observer, ImageEvent::new(EventKind::Failed, context()));
    assert_eq!(
        observer.health(),
        ObserverHealth {
            level: ImageLogLevel::Off,
            filtered: 1,
            admitted: 0,
            failures: 1,
        }
    );
    observer.set_level(ImageLogLevel::Warn);
    ImageObserver::observe(&observer, ImageEvent::new(EventKind::Sent, context()));
    ImageObserver::observe(&observer, ImageEvent::new(EventKind::Refused, context()));
    assert_eq!(observer.health().filtered, 2);
    assert_eq!(observer.health().failures, 2);
}

#[test]
fn local_transform_has_lineage_but_no_supplier_cost_call() {
    let mut context = context();
    context.kind = Some(StageKind::LocalTransform);
    context.backend = None;
    let mut event = ImageEvent::new(EventKind::TransformCompleted, context);
    event.measurements.transform_steps = Some(3);
    event.measurements.algorithm_revision = Some(2);
    let record = event.into_record();
    assert_eq!(attribute(&record, "image.processing"), Some(&text("local")));
    assert_eq!(
        attribute(&record, "image.transform_id"),
        Some(&text(&"04".repeat(16)))
    );
    assert_eq!(
        attribute(&record, "image.transform_steps"),
        Some(&AnyValue::Int(3))
    );
    assert!(attribute(&record, "image.cost_call_id").is_none());
}

#[test]
fn stage_projection_cannot_forward_snapshot_account_or_supplier_payloads() {
    use crate::execution::StageState;
    use crate::{JobState, SharingIdentity, VersionedSnapshot};
    use df_types::AccountId;

    let private = b"private-prompt reference-bytes player-words token=secret stderr-prose";
    let job = ImageJob {
        job: ImageJobId::from_bytes(&[1; 16]).unwrap(),
        owner: ImageOwner::Draft {
            account: AccountId::from_bytes(&[2; 16]).unwrap(),
        },
        sharing: SharingIdentity::from_bytes(&[3; 32]).unwrap(),
        intent: VersionedSnapshot::new(1, private.to_vec(), 1024).unwrap(),
        config: VersionedSnapshot::new(1, private.to_vec(), 1024).unwrap(),
        absolute_deadline_ms: 100,
        state: JobState::Planning,
        revision: 1,
        resolved_key: None,
        recipe: Some(VersionedSnapshot::new(1, private.to_vec(), 1024).unwrap()),
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    let stage = StageView {
        owner: job.owner.clone(),
        job: job.job,
        stage: ExecutionStageId::from_bytes(&[4; 16]).unwrap(),
        kind: StageKind::Generation,
        state: StageState::Planned,
        claim_generation: 1,
        lease_until_ms: 100,
        local_accounting: true,
        planned_publication: Some(
            crate::execution::PublicationIdentity::new([5; 16], [6; 16]).unwrap(),
        ),
        job_revision: 1,
    };
    let event = ImageEvent::new(
        EventKind::Failed,
        EventContext::for_stage(&job, &stage, None),
    )
    .failed(FailureKind::ProviderUnknown);
    let debug = format!("{event:?}");
    let record = event.into_record();
    let encoded = format!("{record:?}");
    for sentinel in [
        "private-prompt",
        "reference-bytes",
        "player-words",
        "secret",
        "stderr-prose",
    ] {
        assert!(!debug.contains(sentinel));
        assert!(!encoded.contains(sentinel));
    }
    assert!(attribute(&record, "image.account").is_none());
    assert_eq!(
        attribute(&record, "image.catalog_attempt_id"),
        Some(&text(&"06".repeat(16)))
    );
    assert_eq!(
        attribute(&record, "image.failure"),
        Some(&text("provider_unknown"))
    );
    let analysis = StageView {
        kind: StageKind::PhotoInspection,
        planned_publication: None,
        ..stage
    };
    let context = EventContext::for_stage(&job, &analysis, None);
    assert!(context.catalog_attempt.is_none());
    assert!(context.asset.is_none());
    assert!(context.attempt.is_none());
}

#[test]
fn unsigned_measurements_preserve_exact_overflow_and_usage_evidence() {
    let mut event = ImageEvent::new(EventKind::UsageRecorded, context());
    event.measurements.output_bytes = Some(u64::MAX);
    event.measurements.usage_revision = Some(crate::cost::UsageRevision(8));
    event.measurements.usage_evidence = Some(crate::cost::UsageEvidence::Unknown);
    let record = event.into_record();
    assert_eq!(
        attribute(&record, "image.output_bytes"),
        Some(&text(&u64::MAX.to_string()))
    );
    assert_eq!(
        attribute(&record, "image.usage_revision"),
        Some(&AnyValue::Int(8))
    );
    assert_eq!(
        attribute(&record, "image.usage_evidence"),
        Some(&text("unknown"))
    );
}

#[test]
fn supplier_failure_fields_preserve_classification_without_urls_or_prose() {
    let mut event =
        ImageEvent::new(EventKind::Failed, context()).failed(FailureKind::ProviderUnknown);
    event.measurements.provider_step = Some(ProviderStep::Download);
    event.measurements.provider_status = Some(503);
    event.measurements.delivery_identified = Some(true);
    event.measurements.admission_refused = Some(false);
    let record = event.into_record();
    assert_eq!(
        attribute(&record, "image.provider_step"),
        Some(&text("download"))
    );
    assert_eq!(
        attribute(&record, "image.provider_status"),
        Some(&AnyValue::Int(503))
    );
    assert_eq!(
        attribute(&record, "image.delivery_identified"),
        Some(&AnyValue::Boolean(true))
    );
    assert_eq!(
        attribute(&record, "image.admission_refused"),
        Some(&AnyValue::Boolean(false))
    );
}

#[test]
fn preflight_and_quota_reasons_remain_closed_without_inventing_supplier_identity() {
    use crate::execution::QuotaDeferralReason;
    use crate::runtime::PreflightRejection;
    for (reason, label) in [
        (PreflightRejection::MissingCredential, "missing_credential"),
        (
            PreflightRejection::UnsupportedProfile,
            "unsupported_profile",
        ),
    ] {
        let mut event = ImageEvent::new(EventKind::Refused, EventContext::default());
        event.measurements.preflight_rejection = Some(reason);
        event.measurements.supplier_dispatched = Some(false);
        let record = event.into_record();
        assert_eq!(
            attribute(&record, "image.preflight_reason"),
            Some(&text(label))
        );
        assert!(attribute(&record, "image.attempt_id").is_none());
        assert!(attribute(&record, "image.cost_call_id").is_none());
    }
    for (reason, label) in [
        (QuotaDeferralReason::Cooldown, "cooldown"),
        (QuotaDeferralReason::Probing, "probing"),
    ] {
        let mut event = ImageEvent::new(EventKind::SourceDeferred, EventContext::default());
        event.measurements.quota_deferral = Some(reason);
        let record = event.into_record();
        assert_eq!(
            attribute(&record, "image.quota_deferral"),
            Some(&text(label))
        );
        assert!(attribute(&record, "image.attempt_id").is_none());
    }
}
