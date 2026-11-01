use super::*;
use crate::config::{CliProfileRevision, ImageConfig, ModelName};
use crate::intent_codec::{CONFIG_CODEC_VERSION, encode_intent};
use crate::journal::{
    AcceptCommand, AcceptOutcome, FreezeCommand, FreezeOutcome, ImageInterest, ImageInterestId,
    ImageJob, InterestState, JobState, OperationReceipt, SharingIdentity, WithdrawOutcome,
};
use crate::output::{AlphaHandling, Dimensions, Encoding, OutputProfile, RasterOutputSpec};
use crate::publication::PublishedVariant;
use crate::request::{
    GenerateSpec, ImageOperation, ImageOwner, OperationIdentity, Purpose, PurposedUse, TextPrompt,
    Urgency, VariantRole,
};
use crate::service::ports::{AuthDenied, ServiceIdError, StoredImageAuthorization};
use df_assets::{
    AssetId, AttemptId, Audience, ImageAlpha, ImageIntent, MediaRequestKey, ObjectVersion, WriteId,
};
use df_types::{AccountId, OperationId};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

const NOW_MS: i64 = 1_700_000_000_000;

fn owner() -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_bytes(&[1; 16]).unwrap(),
    }
}

fn request() -> ImageRequest {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(1, 1).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    ImageRequest {
        identity: OperationIdentity {
            owner: owner(),
            operation: OperationId::from_bytes(&[2; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a quiet inn", 1).unwrap(),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        audience: Audience::PublicStatic,
        expected_source_revision: 7,
        output: OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), delivered),
        urgency: Urgency::Present,
    }
}

fn snapshots(
    request: &ImageRequest,
    config: &ImageConfig,
) -> (VersionedSnapshot, VersionedSnapshot) {
    let intent = encode_intent(request, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    let config = encode_config(config, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    (
        VersionedSnapshot::new(
            u32::from(intent.bytes()[0]),
            intent.bytes().to_vec(),
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .unwrap(),
        VersionedSnapshot::new(
            u32::from(CONFIG_CODEC_VERSION),
            config.bytes().to_vec(),
            PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .unwrap(),
    )
}

fn stored_view(request: &ImageRequest, config: &ImageConfig, number: u64) -> OperationView {
    let mut bytes = [0; 16];
    bytes[8..].copy_from_slice(&number.to_be_bytes());
    let job = ImageJobId::from_bytes(&bytes).unwrap();
    let interest = ImageInterestId::from_bytes(&bytes).unwrap();
    let operation = OperationId::from_bytes(&bytes).unwrap();
    let (intent, config) = snapshots(request, config);
    let fingerprint = *encode_intent(request, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES)
        .unwrap()
        .fingerprint();
    let image_intent = ImageIntent::new("image/png", 1, 1, ImageAlpha::Opaque).unwrap();
    let recipe = FrozenOutput::new(image_intent, None, b"retained-execution-recipe".to_vec())
        .unwrap()
        .snapshot()
        .unwrap();
    OperationView {
        receipt: OperationReceipt {
            owner: request.identity.owner.clone(),
            operation,
            fingerprint,
            job,
            interest,
        },
        job: ImageJob {
            job,
            owner: request.identity.owner.clone(),
            sharing: SharingIdentity::from_bytes(&[3; 32]).unwrap(),
            intent,
            config,
            absolute_deadline_ms: NOW_MS + 240_000,
            state: JobState::RecipeFrozen,
            revision: 3,
            resolved_key: Some(MediaRequestKey::from_bytes([4; 32])),
            recipe: Some(recipe),
            created_at_ms: NOW_MS - i64::try_from(number).unwrap(),
            updated_at_ms: NOW_MS,
        },
        interest: ImageInterest {
            interest,
            owner: request.identity.owner.clone(),
            job,
            operation,
            state: InterestState::Attached,
            created_at_ms: NOW_MS,
            withdrawn_at_ms: None,
        },
    }
}

fn published(view: &OperationView) -> PublishedImage {
    let intent = ImageIntent::new("image/png", 1, 1, ImageAlpha::Opaque).unwrap();
    PublishedImage {
        job: view.job.job,
        stage: crate::execution::ExecutionStageId::from_bytes(&[5; 16]).unwrap(),
        dispatch: crate::execution::DispatchId::from_bytes(&[6; 16]).unwrap(),
        asset: AssetId::from_bytes(&[7; 16]).unwrap(),
        attempt: AttemptId::from_bytes(&[8; 16]).unwrap(),
        recipe_digest: crate::publication::recipe_digest(view.job.recipe.as_ref().unwrap()),
        job_revision: view.job.revision,
        published_at_ms: NOW_MS - 1,
        private_reconciliation: false,
        variants: vec![PublishedVariant {
            variant: AssetVariant::Master,
            write: WriteId::from_bytes(&[9; 16]).unwrap(),
            object_version: ObjectVersion::new("original-version".to_owned()).unwrap(),
            byte_len: 23,
            sha256: [10; 32],
            intent,
            alpha: ImageAlpha::Opaque,
        }],
    }
}

#[derive(Clone)]
struct Journal {
    views: Vec<OperationView>,
    pages: Arc<AtomicUsize>,
    fail: bool,
    exhaust_clock: Option<(Arc<AtomicI64>, i64)>,
}

impl ImageJournal for Journal {
    async fn accept(&self, _: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        panic!("compatible lookup must not mutate demand");
    }

    async fn compatible_candidates(
        &self,
        expected_owner: &ImageOwner,
        _: &VersionedSnapshot,
        _: &VersionedSnapshot,
        after: Option<CompatibleCursor>,
    ) -> Result<Vec<OperationView>, JournalError> {
        self.pages.fetch_add(1, Ordering::SeqCst);
        if let Some((clock, deadline_ms)) = &self.exhaust_clock {
            clock.store(*deadline_ms, Ordering::SeqCst);
        }
        if self.fail {
            return Err(JournalError::Unavailable);
        }
        Ok(self
            .views
            .iter()
            .filter(|view| {
                &view.job.owner == expected_owner
                    && after.is_none_or(|previous| CompatibleCursor::of(view).precedes(previous))
            })
            .take(MAXIMUM_COMPATIBLE_CANDIDATES)
            .cloned()
            .collect())
    }

    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        Ok(self
            .views
            .iter()
            .find(|view| view.receipt.owner == *owner && view.receipt.operation == operation)
            .cloned())
    }

    async fn read_job(
        &self,
        _: &ImageOwner,
        _: ImageJobId,
    ) -> Result<Option<ImageJob>, JournalError> {
        panic!("lookup-only cache must use coherent receipt pages");
    }

    async fn freeze_recipe(&self, _: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        panic!("compatible lookup must not freeze a recipe");
    }

    async fn withdraw_interest(
        &self,
        _: &ImageOwner,
        _: ImageInterestId,
        _: i64,
    ) -> Result<WithdrawOutcome, JournalError> {
        panic!("compatible lookup must not withdraw an interest");
    }
}

#[derive(Clone)]
struct Clock(Arc<AtomicI64>);
impl ServiceClock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Default)]
struct Ids(Arc<AtomicUsize>);
impl ServiceIds for Ids {
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ServiceIdError::InvalidGeneratedId)
    }

    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(ServiceIdError::InvalidGeneratedId)
    }
}

#[derive(Clone)]
struct Authorizer {
    allowed: Arc<AtomicBool>,
}
impl ImageAuthorizer for Authorizer {
    type Principal = ();

    fn authorize_request(&self, _: &(), _: &ImageRequest) -> Result<(), AuthDenied> {
        Ok(())
    }

    fn authorize_owner(&self, _: &(), _: &ImageOwner, _: ServiceAction) -> Result<(), AuthDenied> {
        Ok(())
    }

    fn authorize_stored(
        &self,
        _: &(),
        _: ServiceAction,
        _: &OperationView,
        _: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        if self.allowed.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
}

#[derive(Clone)]
struct Results {
    artifact: Option<crate::publication::ReadyArtifact>,
    result: Option<PublishedImage>,
    current_grant: Arc<AtomicBool>,
    reads: Arc<AtomicUsize>,
}
impl ImageResults for Results {
    async fn find_ready_artifact(
        &self,
        _: &ImageRequest,
        _: &VersionedSnapshot,
        _: i64,
    ) -> Result<Option<crate::publication::ReadyArtifact>, PublicationError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .artifact
            .clone()
            .filter(|_| self.current_grant.load(Ordering::SeqCst)))
    }
    async fn read_result(
        &self,
        _: &ImageOwner,
        _: &ImageJobId,
    ) -> Result<Option<PublishedImage>, PublicationError> {
        panic!("ordinary result metadata is not compatible reuse authority");
    }

    async fn read_artifact(
        &self,
        _: &ImageOwner,
        _: &ImageJobId,
        _: i64,
    ) -> Result<Option<crate::publication::ReadyArtifact>, PublicationError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .artifact
            .clone()
            .filter(|_| self.current_grant.load(Ordering::SeqCst)))
    }
    async fn read_reusable_result(
        &self,
        _: &ImageOwner,
        job: &ImageJobId,
        _: i64,
    ) -> Result<Option<PublishedImage>, PublicationError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .result
            .clone()
            .filter(|result| result.job == *job && self.current_grant.load(Ordering::SeqCst)))
    }
}

struct Observations(
    Option<ImageExecutionState>,
    Option<crate::ImageInspectionObservation>,
);
impl ImageObservations for Observations {
    async fn read_inspection(
        &self,
        _: &ImageOwner,
        _: ImageJobId,
    ) -> Result<Option<crate::ImageInspectionObservation>, JournalError> {
        Ok(self.1.clone())
    }

    async fn read_progress(
        &self,
        _: &ImageOwner,
        _: ImageJobId,
        _: i64,
    ) -> Result<Option<super::super::types::ImageProgress>, JournalError> {
        Ok(self.0.map(|state| super::super::types::ImageProgress {
            revision: 3,
            state,
            assessment: None,
            failure: None,
            operational_notice: None,
        }))
    }
}

type Service = ImageService<Journal, Clock, Ids, Authorizer, Results, Observations>;

fn service(
    config: ImageConfig,
    views: Vec<OperationView>,
    result: Option<PublishedImage>,
) -> Service {
    ImageService::new(
        Journal {
            views,
            pages: Arc::new(AtomicUsize::new(0)),
            fail: false,
            exhaust_clock: None,
        },
        config,
        Clock(Arc::new(AtomicI64::new(NOW_MS))),
        Ids::default(),
        Authorizer {
            allowed: Arc::new(AtomicBool::new(true)),
        },
    )
    .unwrap()
    .with_results(Results {
        artifact: None,
        result,
        current_grant: Arc::new(AtomicBool::new(true)),
        reads: Arc::new(AtomicUsize::new(0)),
    })
    .with_observations(Observations(None, None))
}

#[test]
fn execution_and_global_admission_modes_share_content_without_changing_receipt_fingerprints() {
    let original = request();
    let config = ImageConfig::defaults().unwrap();
    let (original_intent, original_config) = snapshots(&original, &config);
    let original_fingerprint =
        encode_intent(&original, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    for execution in [ExecutionMode::PreparedOnly, ExecutionMode::Replay] {
        let mut lookup = original.clone();
        lookup.execution = execution;
        let mut cache_config = config.clone();
        cache_config.mode = GlobalMode::CacheOnly;
        let (intent, config) = snapshots(&lookup, &cache_config);
        assert!(
            compatible_snapshots(&original_intent, &original_config, &intent, &config).unwrap()
        );
        assert_eq!(
            compatible_key(&original_intent, &original_config).unwrap(),
            compatible_key(&intent, &config).unwrap()
        );
        assert_ne!(
            original_fingerprint.fingerprint(),
            encode_intent(&lookup, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES)
                .unwrap()
                .fingerprint()
        );
    }
}

#[test]
fn provider_model_profile_bounds_and_source_revision_remain_content_identity() {
    let original = request();
    let config = ImageConfig::defaults().unwrap();
    let (intent, frozen_config) = snapshots(&original, &config);
    let mut variants = Vec::new();
    let mut changed = config.clone();
    changed.backends[0].generation_model = Some(ModelName::new("different-model").unwrap());
    variants.push(changed);
    let mut changed = config.clone();
    changed.codex.cli_profile = CliProfileRevision::new("different-profile").unwrap();
    variants.push(changed);
    let mut changed = config.clone();
    changed.transforms.max_decoded_bytes += 1;
    variants.push(changed);
    let mut changed = config.clone();
    changed.default_backend = crate::config::BackendId::OpenAi;
    variants.push(changed);
    for changed in variants {
        let (_, other_config) = snapshots(&original, &changed);
        assert!(!compatible_snapshots(&intent, &frozen_config, &intent, &other_config).unwrap());
        assert_ne!(
            compatible_key(&intent, &frozen_config).unwrap(),
            compatible_key(&intent, &other_config).unwrap()
        );
    }
    let mut changed = original;
    changed.expected_source_revision += 1;
    let (other_intent, _) = snapshots(&changed, &config);
    assert!(!compatible_snapshots(&intent, &frozen_config, &other_intent, &frozen_config).unwrap());
}

#[tokio::test]
async fn historical_ready_reuse_preserves_retired_withdrawn_and_original_handle_without_ids() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let mut view = stored_view(&original, &config, 1);
    view.job.state = JobState::Retired;
    view.job.absolute_deadline_ms = NOW_MS - 1;
    view.interest.state = InterestState::Withdrawn;
    view.interest.withdrawn_at_ms = Some(NOW_MS);
    let actual = published(&view);
    let expected_operation = view.receipt.operation;
    let mut cache_config = config;
    cache_config.mode = GlobalMode::CacheOnly;
    let service = service(cache_config.clone(), vec![view], Some(actual.clone()));
    let mut lookup = original;
    lookup.execution = ExecutionMode::PreparedOnly;
    let (intent, config) = snapshots(&lookup, &cache_config);
    let Some(CompatibleImage::Ready { demand, .. }) = service
        .find_compatible(&(), &lookup, &intent, &config)
        .await
        .unwrap()
    else {
        panic!("retained actual Ready output must be reusable");
    };
    assert_eq!(demand.handle.operation, expected_operation);
    assert_eq!(demand.job_state, JobState::Retired);
    assert_eq!(demand.interest_state, InterestState::Withdrawn);
    assert_eq!(demand.result, Some(actual));
    assert_eq!(demand.execution, ImageExecutionState::Ready);
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stored_denial_precedes_canonical_comparison_and_result_authority() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let view = stored_view(&original, &config, 1);
    let result = published(&view);
    let service = service(config.clone(), vec![view], Some(result));
    service.authorizer.allowed.store(false, Ordering::SeqCst);
    let (intent, config) = snapshots(&original, &config);
    assert!(
        service
            .find_compatible(&(), &original, &intent, &config)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(service.results.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn revoked_current_result_grant_is_a_miss_even_when_stored_snapshots_match() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let view = stored_view(&original, &config, 1);
    let result = published(&view);
    let service = service(config.clone(), vec![view], Some(result));
    service.results.current_grant.store(false, Ordering::SeqCst);
    let (intent, config) = snapshots(&original, &config);
    assert!(
        service
            .find_compatible(&(), &original, &intent, &config)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(service.results.reads.load(Ordering::SeqCst), 1);
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn matching_legacy_ready_after_the_first_full_page_is_found() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let mut unrelated = original.clone();
    unrelated.expected_source_revision += 1;
    let mut views: Vec<_> = (1..=64)
        .map(|number| stored_view(&unrelated, &config, number))
        .collect();
    let winner = stored_view(&original, &config, 65);
    let result = published(&winner);
    views.push(winner);
    let service = service(config.clone(), views, Some(result.clone()));
    let (intent, config) = snapshots(&original, &config);
    let Some(CompatibleImage::Ready { demand, .. }) = service
        .find_compatible(&(), &original, &intent, &config)
        .await
        .unwrap()
    else {
        panic!("the first page is not a total history limit");
    };
    assert_eq!(demand.result, Some(result));
    assert_eq!(service.journal.pages.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn explicit_regeneration_never_searches_for_compatible_output() {
    let config = ImageConfig::defaults().unwrap();
    let mut original = request();
    original.freshness = Freshness::Regenerate {
        nonce: OperationId::from_bytes(&[90; 16]).unwrap(),
    };
    let service = service(config.clone(), Vec::new(), None);
    let (intent, config) = snapshots(&original, &config);
    assert!(
        service
            .find_compatible(&(), &original, &intent, &config)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(service.journal.pages.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn exact_live_unknown_work_coalesces_and_lookup_only_demand_never_joins_it() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let view = stored_view(&original, &config, 1);
    let mut service = service(config.clone(), vec![view], None);
    service.observations = Observations(Some(ImageExecutionState::Unknown), None);
    let (intent, frozen_config) = snapshots(&original, &config);
    assert!(matches!(
        service
            .find_compatible(&(), &original, &intent, &frozen_config)
            .await
            .unwrap(),
        Some(CompatibleImage::Pending(_))
    ));
    let mut lookup = original;
    lookup.execution = ExecutionMode::PreparedOnly;
    let (intent, config) = snapshots(&lookup, &config);
    assert!(
        service
            .find_compatible(&(), &lookup, &intent, &config)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn publication_fingerprint_and_private_reconciliation_never_fabricate_ready() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let view = stored_view(&original, &config, 1);
    let mut changed_digest = published(&view);
    changed_digest.recipe_digest = [99; 32];
    let mut private = published(&view);
    private.private_reconciliation = true;
    for invalid in [changed_digest, private] {
        let service = service(config.clone(), vec![view.clone()], Some(invalid));
        let (intent, config) = snapshots(&original, &config);
        assert!(matches!(
            service
                .find_compatible(&(), &original, &intent, &config)
                .await,
            Err(ImageServiceError::InvalidRecord)
        ));
    }
}

#[tokio::test]
async fn lookup_failure_never_becomes_a_cache_miss_that_could_start_paid_work() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let mut service = service(config.clone(), Vec::new(), None);
    service.journal.fail = true;
    let (intent, config) = snapshots(&original, &config);
    assert!(matches!(
        service
            .find_compatible(&(), &original, &intent, &config)
            .await,
        Err(ImageServiceError::Unavailable)
    ));
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn public_lookup_modes_reuse_actual_ready_without_minting_or_mutating_demand() {
    let original = request();
    let production_config = ImageConfig::defaults().unwrap();
    let view = stored_view(&original, &production_config, 1);
    let result = published(&view);
    for (execution, mode) in [
        (ExecutionMode::PreparedOnly, GlobalMode::Live),
        (ExecutionMode::Replay, GlobalMode::Live),
        (ExecutionMode::Live, GlobalMode::CacheOnly),
    ] {
        let mut config = production_config.clone();
        config.mode = mode;
        let service = service(config, vec![view.clone()], Some(result.clone()));
        let mut lookup = original.clone();
        lookup.execution = execution;
        lookup.identity.fingerprint = service.canonical_fingerprint(&lookup).unwrap();
        let outcome = service.request(&(), &lookup).await.unwrap();
        let crate::service::RequestOutcome::Reused(demand) = outcome else {
            panic!("lookup-only hit must be a read-only result");
        };
        assert_eq!(demand.handle.operation, view.receipt.operation);
        assert_eq!(demand.result, Some(result.clone()));
        assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn reusing_an_existing_operation_with_changed_execution_still_conflicts() {
    let original = request();
    let config = ImageConfig::defaults().unwrap();
    let view = stored_view(&original, &config, 1);
    let result = published(&view);
    let operation = view.receipt.operation;
    let service = service(config, vec![view], Some(result));
    let mut changed = original;
    changed.identity.operation = operation;
    changed.execution = ExecutionMode::PreparedOnly;
    changed.identity.fingerprint = service.canonical_fingerprint(&changed).unwrap();
    assert_eq!(
        service.request(&(), &changed).await.unwrap_err(),
        ImageServiceError::Conflict
    );
    assert_eq!(service.journal.pages.load(Ordering::SeqCst), 0);
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[test]
fn source_digest_audience_output_pipeline_rating_and_regeneration_nonce_do_not_share_content() {
    let config = ImageConfig::defaults().unwrap();
    let mut original = request();
    let ImageOperation::Generate(spec) = &mut original.operation else {
        unreachable!();
    };
    spec.references.push(crate::SourceRef {
        asset: AssetId::from_bytes(&[70; 16]).unwrap(),
        digest: crate::AssetDigest::from_bytes([71; 32]),
        audience: Audience::PublicStatic,
    });
    let (intent, frozen_config) = snapshots(&original, &config);
    let mut variants = Vec::new();
    let mut changed = original.clone();
    let ImageOperation::Generate(spec) = &mut changed.operation else {
        unreachable!();
    };
    spec.references[0].digest = crate::AssetDigest::from_bytes([72; 32]);
    variants.push(changed);
    let mut changed = original.clone();
    changed.audience = Audience::OwnerPrivate(AccountId::from_bytes(&[1; 16]).unwrap());
    variants.push(changed);
    let mut changed = original.clone();
    changed.output = OutputProfile::provider(
        Dimensions::new(1024, 1024).unwrap(),
        RasterOutputSpec::new(
            Dimensions::new(2, 1).unwrap(),
            AlphaHandling::Opaque,
            Encoding::Png,
        )
        .unwrap(),
    );
    variants.push(changed);
    let mut changed = original.clone();
    changed.freshness = Freshness::Regenerate {
        nonce: OperationId::from_bytes(&[73; 16]).unwrap(),
    };
    variants.push(changed);
    for changed in variants {
        let (other, _) = snapshots(&changed, &config);
        assert!(!compatible_snapshots(&intent, &frozen_config, &other, &frozen_config).unwrap());
    }

    let with_pipeline = |rating| {
        let mut request = original.clone();
        let ImageOperation::Generate(spec) = &mut request.operation else {
            unreachable!();
        };
        spec.prompt = spec
            .prompt
            .clone()
            .with_pipeline(crate::ImagePipeline {
                revision: 1,
                moderation: Some(crate::ProviderModeration::Auto),
                rating,
                creator: None,
                inspection: None,
            })
            .unwrap();
        request
    };
    let (restrained, _) = snapshots(
        &with_pipeline(crate::ImageContentRating::Restrained),
        &config,
    );
    let (mature, _) = snapshots(&with_pipeline(crate::ImageContentRating::Mature), &config);
    assert!(!compatible_snapshots(&restrained, &frozen_config, &mature, &frozen_config).unwrap());
    assert!(!compatible_snapshots(&intent, &frozen_config, &restrained, &frozen_config).unwrap());
}

#[test]
fn malformed_snapshot_envelopes_never_create_an_indexed_compatible_identity() {
    let config = ImageConfig::defaults().unwrap();
    let (intent, config) = snapshots(&request(), &config);
    let wrong_envelope = VersionedSnapshot::new(
        intent.revision() + 1,
        intent.bytes().to_vec(),
        PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .unwrap();
    assert_eq!(
        compatible_key(&wrong_envelope, &config),
        Err(JournalError::InvalidRecord)
    );
    let mut trailing = intent.bytes().to_vec();
    trailing.push(0);
    let trailing =
        VersionedSnapshot::new(intent.revision(), trailing, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    assert_eq!(
        compatible_key(&trailing, &config),
        Err(JournalError::InvalidRecord)
    );
}

#[tokio::test]
async fn exhausting_the_configured_lookup_budget_is_unavailable_and_never_a_false_miss() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let mut service = service(config.clone(), Vec::new(), None);
    let deadline_ms = NOW_MS + i64::try_from(config.scheduler.job_deadline_secs).unwrap() * 1_000;
    service.journal.exhaust_clock = Some((service.clock.0.clone(), deadline_ms));
    let (intent, config) = snapshots(&original, &config);
    assert!(matches!(
        service
            .find_compatible(&(), &original, &intent, &config)
            .await,
        Err(ImageServiceError::Unavailable)
    ));
    assert_eq!(service.journal.pages.load(Ordering::SeqCst), 1);
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[test]
fn operational_disable_queue_deadline_concurrency_and_retry_tuning_preserve_content_identity() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let (intent, frozen_config) = snapshots(&original, &config);
    let mutations: &[fn(&mut ImageConfig)] = &[
        |config| config.mode = GlobalMode::CacheOnly,
        |config| config.transforms.enabled = false,
        |config| config.transforms.max_concurrency += 1,
        |config| config.transforms.queue_capacity += 1,
        |config| config.transforms.deadline_secs += 1,
        |config| config.scheduler.foreground_limit += 1,
        |config| config.scheduler.presence_limit += 1,
        |config| config.scheduler.background_limit += 1,
        |config| config.scheduler.max_pending_jobs += 1,
        |config| config.scheduler.max_pending_bytes += 1,
        |config| config.scheduler.queue_age_secs += 1,
        |config| config.scheduler.job_deadline_secs += 1,
        |config| config.scheduler.stage_deadline_secs += 1,
        |config| config.retry.retry_deadline_secs += 1,
        |config| config.retry.backoff_secs += 1,
        |config| config.quota.enabled = false,
        |config| config.quota.cooldown_ms += 1,
        |config| config.quota.maximum_accounts += 1,
    ];
    for mutate in mutations {
        let mut changed = config.clone();
        mutate(&mut changed);
        let (_, changed_config) = snapshots(&original, &changed);
        assert!(compatible_snapshots(&intent, &frozen_config, &intent, &changed_config).unwrap());
        assert_eq!(
            compatible_key(&intent, &frozen_config).unwrap(),
            compatible_key(&intent, &changed_config).unwrap()
        );
    }
    for index in 0..crate::config::BackendId::COUNT {
        for enabled in [false, true] {
            let mut changed = config.clone();
            changed.backends[index].enabled = enabled;
            changed.backends[index].max_concurrent_jobs += 1;
            let (_, changed_config) = snapshots(&original, &changed);
            assert!(
                compatible_snapshots(&intent, &frozen_config, &intent, &changed_config).unwrap()
            );
        }
    }
    for index in 0..Purpose::COUNT {
        let mut changed = config.clone();
        changed.purposes[index].enabled = false;
        let (_, changed_config) = snapshots(&original, &changed);
        assert!(compatible_snapshots(&intent, &frozen_config, &intent, &changed_config).unwrap());
    }
}

#[test]
fn acceptance_retry_attempts_preparation_and_cli_bounds_remain_content_identity() {
    let config = ImageConfig::defaults().unwrap();
    let original = request();
    let (intent, frozen_config) = snapshots(&original, &config);
    let mutations: &[fn(&mut ImageConfig)] = &[
        |config| config.transforms.max_decoded_bytes += 1,
        |config| config.transforms.max_output_bytes += 1,
        |config| config.request_limits.max_prompt_chars += 1,
        |config| config.request_limits.max_references += 1,
        |config| config.retry.max_attempts += 1,
        |config| config.analysis.preparation.max_pixels += 1,
        |config| config.analysis.preparation.max_picture_bytes += 1,
        |config| config.analysis.preparation.max_request_bytes += 1,
        |config| config.analysis.segmentation_preparation.max_pixels += 1,
        |config| config.analysis.segmentation_preparation.max_picture_bytes += 1,
        |config| config.analysis.segmentation_preparation.max_request_bytes += 1,
        |config| config.codex.max_stdout_bytes += 1,
        |config| {
            config.creator_bfl_account =
                Some(crate::config::AccountHandle::new("other-creator").unwrap());
        },
        |config| config.codex.max_stderr_bytes += 1,
        |config| config.codex.max_message_bytes += 1,
        |config| config.codex.workdir = crate::config::JobWorkDir::new("another-workdir").unwrap(),
        |config| {
            config.backends[0].account = crate::config::AccountHandle::new("other-account").unwrap()
        },
        |config| {
            config.backends[0].endpoint =
                Some(crate::config::ApiEndpoint::new("http://127.0.0.1:55471").unwrap());
        },
        |config| config.quality.provider_quality = Some(crate::config::GenerationQuality::High),
    ];
    for mutate in mutations {
        let mut changed = config.clone();
        mutate(&mut changed);
        let (_, changed_config) = snapshots(&original, &changed);
        assert!(!compatible_snapshots(&intent, &frozen_config, &intent, &changed_config).unwrap());
        assert_ne!(
            compatible_key(&intent, &frozen_config).unwrap(),
            compatible_key(&intent, &changed_config).unwrap()
        );
    }
}

#[tokio::test]
async fn disabled_backend_and_local_workers_can_reuse_ready_but_service_and_purpose_off_refuse() {
    let original = request();
    let config = ImageConfig::defaults().unwrap();
    let view = stored_view(&original, &config, 1);
    let result = published(&view);
    let mut changed = config;
    changed.backends[0].enabled = false;
    changed.transforms.enabled = false;
    changed.mode = GlobalMode::CacheOnly;
    let cache_service = service(changed.clone(), vec![view], Some(result.clone()));
    let mut lookup = original.clone();
    lookup.identity.fingerprint = cache_service.canonical_fingerprint(&lookup).unwrap();
    assert!(matches!(
        cache_service.request(&(), &lookup).await.unwrap(),
        crate::service::RequestOutcome::Reused(demand) if demand.result == Some(result)
    ));
    for (mode, purpose_enabled, refusal) in [
        (GlobalMode::Off, true, crate::policy::Refusal::ServiceOff),
        (
            GlobalMode::Live,
            false,
            crate::policy::Refusal::PurposeDisabled,
        ),
    ] {
        let mut config = changed.clone();
        config.mode = mode;
        config.purposes[original.purpose.purpose.index()].enabled = purpose_enabled;
        let view = stored_view(&original, &ImageConfig::defaults().unwrap(), 1);
        let result = published(&view);
        let service = service(config, vec![view], Some(result));
        assert_eq!(
            service.request(&(), &lookup).await.unwrap(),
            crate::service::RequestOutcome::Refused(refusal)
        );
        assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
    }
}

fn staged_request(config: &mut ImageConfig) -> ImageRequest {
    let route = crate::AnalysisRoute {
        account: crate::config::AccountHandle::new("inspection").unwrap(),
        model: ModelName::new("inspection-model").unwrap(),
        endpoint: crate::config::ApiEndpoint::new("http://127.0.0.1:55471").unwrap(),
        maximum_output_tokens: 1000,
    };
    config.analysis.inspection = Some(route.clone());
    let mut request = request();
    let ImageOperation::Generate(spec) = &mut request.operation else {
        unreachable!();
    };
    spec.prompt = spec
        .prompt
        .clone()
        .with_pipeline(crate::ImagePipeline {
            revision: 1,
            moderation: None,
            rating: crate::ImageContentRating::Restrained,
            creator: None,
            inspection: Some(crate::ImageInspection {
                route,
                requirement: crate::InspectionRequirement::Required,
                contract_revision: 7,
                kind: crate::InspectionKind::ScenePlate,
                expected_heroes: None,
                party_reference: None,
                hand_custody: crate::PrivateImageText::new("the lantern stays in the left hand"),
                staging: vec![crate::StagedIdentity {
                    locator_id: crate::PrivateImageText::new("hero-locator"),
                    reference_id: crate::PrivateImageText::new("hero-reference"),
                    public_name: crate::PrivateImageText::new("Mira"),
                    hero: true,
                }],
                maximum_corrections: 1,
                allow_visible_text: false,
            }),
        })
        .unwrap();
    request
}

fn winning_inspection() -> crate::PhotoInspectionAssessment {
    crate::PhotoInspectionAssessment {
        contract_revision: 7,
        verdict: crate::PhotoInspectionVerdict::Pass,
        visible_features: vec![crate::PhotoVisibleFeature {
            name: crate::PrivateImageText::new("lantern"),
            look: crate::PrivateImageText::new("warm brass"),
            region: [100, 200, 300, 400],
            recorded_id: Some(crate::PrivateImageText::new("visible-lantern")),
        }],
        source_binding: Some(crate::PhotoFeatureBinding {
            place: crate::PrivateImageText::new("inn"),
            source_hash: crate::PrivateImageText::new("winning-candidate-hash"),
        }),
    }
}

#[tokio::test]
async fn historical_ready_reuse_retains_winning_inspection_and_frozen_staging() {
    let mut config = ImageConfig::defaults().unwrap();
    let original = staged_request(&mut config);
    let expected_staging = original
        .operation
        .pipeline()
        .unwrap()
        .inspection
        .as_ref()
        .unwrap()
        .staging
        .clone();
    let mut view = stored_view(&original, &config, 1);
    view.job.state = JobState::Retired;
    view.job.absolute_deadline_ms = NOW_MS - 1;
    view.interest.state = InterestState::Withdrawn;
    view.interest.withdrawn_at_ms = Some(NOW_MS);
    let result = published(&view);
    let mut cache_config = config;
    cache_config.mode = GlobalMode::CacheOnly;
    let mut service = service(cache_config, vec![view], Some(result));
    let inspection = winning_inspection();
    service.observations.1 = Some(crate::ImageInspectionObservation {
        job_revision: 3,
        assessment: inspection.clone(),
    });
    let mut lookup = original;
    lookup.execution = ExecutionMode::PreparedOnly;
    lookup.identity.fingerprint = service.canonical_fingerprint(&lookup).unwrap();
    let crate::RequestOutcome::Reused(demand) = service.request(&(), &lookup).await.unwrap() else {
        panic!("the qualified historical image must be reused");
    };
    assert_eq!(demand.inspection, Some(inspection));
    assert_eq!(demand.staging, expected_staging);
    assert_eq!(demand.job_state, JobState::Retired);
    assert_eq!(demand.interest_state, InterestState::Withdrawn);
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_winning_inspection_read_at_a_different_job_revision_never_appears_as_reused_ready() {
    let mut config = ImageConfig::defaults().unwrap();
    let original = staged_request(&mut config);
    let view = stored_view(&original, &config, 1);
    let result = published(&view);
    let mut cache_config = config;
    cache_config.mode = GlobalMode::CacheOnly;
    let mut service = service(cache_config, vec![view], Some(result));
    service.observations.1 = Some(crate::ImageInspectionObservation {
        job_revision: 4,
        assessment: winning_inspection(),
    });
    let mut lookup = original;
    lookup.identity.fingerprint = service.canonical_fingerprint(&lookup).unwrap();
    assert!(matches!(
        service.request(&(), &lookup).await,
        Err(ImageServiceError::Unknown { .. })
    ));
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

fn historical_config(config: &ImageConfig, revision: u8) -> VersionedSnapshot {
    assert_eq!(
        CONFIG_CODEC_VERSION, 12,
        "review the historical suffix fixture on codec changes"
    );
    assert!(matches!(revision, 10 | 11));
    assert!(config.creator_bfl_account.is_none());
    let encoded = encode_config(config, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    let mut bytes = encoded.bytes().to_vec();
    assert_eq!(
        bytes.last(),
        Some(&0),
        "v12 appends the absent alternate creator account"
    );
    let suffix_bytes = if revision == 10 { 14 } else { 1 };
    bytes.truncate(bytes.len() - suffix_bytes);
    bytes[0] = revision;
    decode_config(&bytes, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    VersionedSnapshot::new(u32::from(revision), bytes, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap()
}

#[test]
fn historical_config_reencodes_to_current_identity_and_retains_creator_account_semantics() {
    let original = request();
    let produced = ImageConfig::defaults().unwrap();
    let (intent, _) = snapshots(&original, &produced);
    let mut current = produced.clone();
    current.mode = GlobalMode::CacheOnly;
    current.quota.enabled = false;
    current.quota.cooldown_ms += 1;
    current.quota.maximum_accounts += 1;
    let (_, current_config) = snapshots(&original, &current);
    for revision in [10, 11] {
        let historical = historical_config(&produced, revision);
        assert!(compatible_snapshots(&intent, &historical, &intent, &current_config).unwrap());
        let old_key = compatible_lookup_key(&intent, &historical).unwrap();
        let current_key = compatible_lookup_key(&intent, &current_config).unwrap();
        assert_eq!(old_key, current_key);
        assert_eq!(old_key.config_revision, u32::from(CONFIG_CODEC_VERSION));
        assert_eq!(old_key.schema_version, COMPATIBLE_KEY_VERSION);
    }
    current.creator_bfl_account =
        Some(crate::config::AccountHandle::new("different-creator").unwrap());
    let (_, different_account) = snapshots(&original, &current);
    assert!(
        !compatible_snapshots(
            &intent,
            &historical_config(&produced, 10),
            &intent,
            &different_account,
        )
        .unwrap()
    );
}

#[tokio::test]
async fn stored_v10_and_v11_ready_reuse_current_config_with_no_new_ids_or_execution() {
    let produced = ImageConfig::defaults().unwrap();
    let original = request();
    for revision in [10, 11] {
        let mut view = stored_view(&original, &produced, 1);
        view.job.config = historical_config(&produced, revision);
        let expected = published(&view);
        let mut current = produced.clone();
        current.mode = GlobalMode::CacheOnly;
        current.quota.enabled = false;
        current.quota.cooldown_ms += 1;
        current.quota.maximum_accounts += 1;
        let service = service(current, vec![view], Some(expected.clone()));
        let mut lookup = original.clone();
        lookup.execution = ExecutionMode::PreparedOnly;
        lookup.identity.fingerprint = service.canonical_fingerprint(&lookup).unwrap();
        let crate::RequestOutcome::Reused(demand) = service.request(&(), &lookup).await.unwrap()
        else {
            panic!(
                "the historical actual Ready result must survive operational-only codec changes"
            );
        };
        assert_eq!(demand.result, Some(expected));
        assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
        assert_eq!(service.results.reads.load(Ordering::SeqCst), 1);
    }
}

#[path = "artifact_cache_tests.rs"]
mod artifact_tests;
