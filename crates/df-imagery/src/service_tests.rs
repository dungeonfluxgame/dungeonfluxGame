//! Boundary tests for the consumer image service: exact replay, typed refusals,
//! stored authorization, idempotent withdrawal and safe diagnostics.
//!
//! The journal, clock, ids and authorizer below are explicit test fakes: they mirror
//! the accepted PostgreSQL journal's observable contract (atomic replay-or-create,
//! revision 1 on creation, last-withdrawal retirement) without claiming its
//! transactionality. PostgreSQL-backed verification belongs to the root harness.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use crate::intent_codec::encode_intent;
use crate::journal::{
    AcceptCommand, AcceptOutcome, AcceptTarget, FreezeCommand, FreezeOutcome, ImageInterest,
    ImageInterestId, ImageJob, ImageJobId, ImageJournal, InterestState, JobState, JoinRefusal,
    JournalError, OperationIdentity, OperationReceipt, OperationView, VersionedSnapshot,
    WithdrawOutcome, Withdrawal, is_expired, owner_scope_bytes,
};
use crate::output::{
    AlphaHandling, Dimensions, Encoding, LossyQuality, OutputProfile, RasterOutputSpec, Rgb,
};
use crate::policy::Refusal;
use crate::request::{
    AssetDigest, EditSpec, ExecutionMode, Freshness, GenerateSpec, ImageOperation, ImageOwner,
    ImageRequest, Purpose, PurposedUse, SourceRef, TextPrompt, Urgency, VariantRole,
};
use crate::service::ports::{
    AuthDenied, ImageAuthorizer, ServiceAction, ServiceIdError, ServiceIds,
    StoredImageAuthorization,
};
use crate::service::types::{
    CancelOutcome, ImageDemandView, ImageServiceError, OperationHandle, RequestCause,
    RequestOutcome, WatchObservation,
};
use crate::service::{ImageService, PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES};
use crate::transform::{FlipAxis, ImageTransform, TransformSource, TransformSpec};
use crate::{AssetId, Audience};
use df_types::{AccountId, CampaignId, OperationId, RunId};

use crate::config::{BackendId, GlobalMode, ImageConfig};

const START_MS: i64 = 1_700_000_000_000;
const RECORDED_DEADLINE_MS: i64 = START_MS + 240_000;

fn account_id(byte: u8) -> AccountId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    AccountId::from_bytes(&bytes).unwrap()
}

fn campaign_id(byte: u8) -> CampaignId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    CampaignId::from_bytes(&bytes).unwrap()
}

fn run_id(byte: u8) -> RunId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    RunId::from_bytes(&bytes).unwrap()
}

fn operation_id(byte: u8) -> OperationId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    OperationId::from_bytes(&bytes).unwrap()
}

fn draft_owner() -> ImageOwner {
    ImageOwner::Draft {
        account: account_id(0x11),
    }
}

fn other_draft_owner() -> ImageOwner {
    ImageOwner::Draft {
        account: account_id(0x99),
    }
}

fn run_owner() -> ImageOwner {
    ImageOwner::Run {
        campaign: campaign_id(0x33),
        run: run_id(0x44),
    }
}

fn source_ref() -> SourceRef {
    SourceRef {
        asset: AssetId::from_bytes(&[0x07; 16]).unwrap(),
        digest: AssetDigest::from_bytes([0x09; 32]),
        audience: Audience::PublicStatic,
    }
}

fn provider_output() -> OutputProfile {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(512, 512).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(85).unwrap(),
        },
    )
    .unwrap();
    OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), delivered)
}

fn local_output() -> OutputProfile {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(64, 64).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    OutputProfile::local(delivered)
}

fn base_request(owner: ImageOwner, operation: OperationId) -> ImageRequest {
    ImageRequest {
        identity: crate::request::OperationIdentity {
            owner,
            operation,
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a tavern at dusk", 3).unwrap(),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: 7,
        output: provider_output(),
    }
}

fn generate_request(owner: ImageOwner, operation: OperationId) -> ImageRequest {
    base_request(owner, operation)
}

fn edit_request(owner: ImageOwner, operation: OperationId) -> ImageRequest {
    let mut request = base_request(owner, operation);
    request.purpose = PurposedUse {
        purpose: Purpose::SceneEdit,
        role: VariantRole::Primary,
    };
    request.operation = ImageOperation::Edit(EditSpec {
        prompt: TextPrompt::new("add a lantern glow", 3).unwrap(),
        base: source_ref(),
        references: Vec::new(),
    });
    request
}

fn transform_request(owner: ImageOwner, operation: OperationId) -> ImageRequest {
    let mut request = base_request(owner, operation);
    request.purpose = PurposedUse {
        purpose: Purpose::ReferencePrep,
        role: VariantRole::Reference,
    };
    request.operation = ImageOperation::Transform(TransformSpec {
        source: TransformSource::Canvas(
            crate::transform::CanvasSpec::new(
                64,
                64,
                Rgb {
                    red: 1,
                    green: 2,
                    blue: 3,
                },
            )
            .unwrap(),
        ),
        steps: vec![ImageTransform::Flip(FlipAxis::Horizontal)],
    });
    request.output = local_output();
    request
}

#[derive(Clone, Copy)]
enum Kind {
    Generate,
    Edit,
    Transform,
}

fn request_for(kind: Kind, owner: ImageOwner, operation: OperationId) -> ImageRequest {
    match kind {
        Kind::Generate => generate_request(owner, operation),
        Kind::Edit => edit_request(owner, operation),
        Kind::Transform => transform_request(owner, operation),
    }
}

fn stamp(mut request: ImageRequest) -> ImageRequest {
    let encoded = encode_intent(&request, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    request.identity.fingerprint = *encoded.fingerprint();
    request
}

fn handle_for(owner: ImageOwner, operation: OperationId) -> OperationHandle {
    OperationHandle { owner, operation }
}

#[derive(Clone)]
struct TestPrincipal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoredPolicy {
    AllowAll,
    DenyAll,
    PublicOnly,
}

#[derive(Clone)]
struct FakeAuth {
    request_owner: ImageOwner,
    coarse_owner: Option<ImageOwner>,
    generate_denied: bool,
    stored_policy: StoredPolicy,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeAuth {
    fn permitting(owner: ImageOwner) -> Self {
        Self {
            request_owner: owner,
            coarse_owner: None,
            generate_denied: false,
            stored_policy: StoredPolicy::AllowAll,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn logged(&self, entry: &str) {
        self.calls.lock().unwrap().push(entry.to_owned());
    }

    fn auth_calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl ImageAuthorizer for FakeAuth {
    type Principal = TestPrincipal;

    fn authorize_request(
        &self,
        _principal: &Self::Principal,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        self.logged("request");
        if request.identity.owner == self.request_owner {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    fn authorize_owner(
        &self,
        _principal: &Self::Principal,
        owner: &ImageOwner,
        action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        self.logged(match action {
            ServiceAction::Read => "owner-read",
            ServiceAction::Cancel => "owner-cancel",
            ServiceAction::Generate => "owner-generate",
        });
        if action == ServiceAction::Generate && self.generate_denied {
            return Err(AuthDenied);
        }
        let allowed = self.coarse_owner.as_ref().unwrap_or(&self.request_owner);
        if owner == allowed {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    fn authorize_stored(
        &self,
        _principal: &Self::Principal,
        action: ServiceAction,
        _view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        self.logged(match action {
            ServiceAction::Read => "stored-read",
            ServiceAction::Cancel => "stored-cancel",
            ServiceAction::Generate => "stored-generate",
        });
        match self.stored_policy {
            StoredPolicy::AllowAll => Ok(()),
            StoredPolicy::DenyAll => Err(AuthDenied),
            StoredPolicy::PublicOnly => match stored.audience {
                Audience::PublicStatic => Ok(()),
                _ => Err(AuthDenied),
            },
        }
    }
}

#[derive(Clone)]
struct FakeClock {
    now: Arc<AtomicI64>,
}

impl FakeClock {
    fn new(start_ms: i64) -> Self {
        Self {
            now: Arc::new(AtomicI64::new(start_ms)),
        }
    }
}

impl crate::service::ports::ServiceClock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct FakeIds {
    jobs: Arc<AtomicU64>,
    interests: Arc<AtomicU64>,
}

impl FakeIds {
    fn new() -> Self {
        Self {
            jobs: Arc::new(AtomicU64::new(1)),
            interests: Arc::new(AtomicU64::new(1)),
        }
    }

    fn jobs_minted(&self) -> u64 {
        self.jobs.load(Ordering::SeqCst) - 1
    }

    fn interests_minted(&self) -> u64 {
        self.interests.load(Ordering::SeqCst) - 1
    }

    fn counter(counter: &AtomicU64) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[8..].copy_from_slice(&counter.fetch_add(1, Ordering::SeqCst).to_be_bytes());
        bytes
    }
}

impl crate::service::ports::ServiceIds for FakeIds {
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        ImageJobId::from_bytes(&Self::counter(&self.jobs))
            .map_err(|_| ServiceIdError::InvalidGeneratedId)
    }

    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        ImageInterestId::from_bytes(&Self::counter(&self.interests))
            .map_err(|_| ServiceIdError::InvalidGeneratedId)
    }
}

struct FakeInner {
    receipts: HashMap<(Vec<u8>, OperationId), OperationReceipt>,
    jobs: HashMap<ImageJobId, ImageJob>,
    interests: HashMap<ImageInterestId, ImageInterest>,
    calls: Vec<String>,
    fail_accept: Option<JournalError>,
    fail_lookup: Option<JournalError>,
    fail_withdraw: Option<JournalError>,
}

#[derive(Clone)]
struct FakeJournal {
    inner: Arc<Mutex<FakeInner>>,
}

impl FakeJournal {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeInner {
                receipts: HashMap::new(),
                jobs: HashMap::new(),
                interests: HashMap::new(),
                calls: Vec::new(),
                fail_accept: None,
                fail_lookup: None,
                fail_withdraw: None,
            })),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, FakeInner> {
        self.inner.lock().unwrap()
    }

    fn calls(&self) -> Vec<String> {
        self.locked().calls.clone()
    }

    fn fail_accept_once(&self, error: JournalError) {
        self.locked().fail_accept = Some(error);
    }

    fn fail_lookup_once(&self, error: JournalError) {
        self.locked().fail_lookup = Some(error);
    }

    fn receipts_len(&self) -> usize {
        self.locked().receipts.len()
    }

    fn attached_count(inner: &FakeInner, job: &ImageJobId) -> u64 {
        inner
            .interests
            .values()
            .filter(|interest| &interest.job == job && interest.state == InterestState::Attached)
            .count() as u64
    }

    async fn join_for_test(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
        fingerprint: [u8; 32],
        job: ImageJobId,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<OperationView, JournalError> {
        let (intent, config, sharing) = {
            let inner = self.locked();
            let stored = inner
                .jobs
                .get(&job)
                .cloned()
                .ok_or(JournalError::NotFound)?;
            (stored.intent.clone(), stored.config.clone(), stored.sharing)
        };
        self.accept(&AcceptCommand {
            identity: OperationIdentity {
                owner: owner.clone(),
                operation,
                fingerprint,
            },
            intent,
            config,
            sharing,
            interest,
            target: AcceptTarget::Join { job },
            now_ms,
        })
        .await
        .map(|outcome| outcome.view().clone())
    }

    fn corrupt_intent(&self, job: ImageJobId) {
        let mut inner = self.locked();
        if let Some(stored) = inner.jobs.get_mut(&job) {
            stored.intent = VersionedSnapshot::new(1, vec![0xFF; 16], 64).unwrap();
        }
    }
}

impl ImageJournal for FakeJournal {
    async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        let mut inner = self.locked();
        inner.calls.push("accept".to_owned());
        if let Some(error) = inner.fail_accept.take() {
            return Err(error);
        }
        if command.now_ms <= 0 {
            return Err(JournalError::InvalidRecord);
        }
        let key = (
            owner_scope_bytes(&command.identity.owner),
            command.identity.operation,
        );
        if let Some(receipt) = inner.receipts.get(&key) {
            let job = inner
                .jobs
                .get(&receipt.job)
                .cloned()
                .ok_or(JournalError::InvalidRecord)?;
            if receipt.fingerprint == command.identity.fingerprint && job.intent == command.intent {
                let interest = inner
                    .interests
                    .get(&receipt.interest)
                    .cloned()
                    .ok_or(JournalError::InvalidRecord)?;
                return Ok(AcceptOutcome::Replayed(OperationView {
                    receipt: receipt.clone(),
                    job,
                    interest,
                }));
            }
            return Err(JournalError::OperationConflict);
        }
        match &command.target {
            AcceptTarget::Create {
                job,
                absolute_deadline_ms,
            } => {
                if *absolute_deadline_ms <= 0 || *absolute_deadline_ms <= command.now_ms {
                    return Err(JournalError::InvalidRecord);
                }
                if inner.jobs.contains_key(job) || inner.interests.contains_key(&command.interest) {
                    return Err(JournalError::IdentityConflict);
                }
                let stored = ImageJob {
                    job: *job,
                    owner: command.identity.owner.clone(),
                    sharing: command.sharing,
                    intent: command.intent.clone(),
                    config: command.config.clone(),
                    absolute_deadline_ms: *absolute_deadline_ms,
                    state: JobState::Planning,
                    revision: 1,
                    resolved_key: None,
                    recipe: None,
                    created_at_ms: command.now_ms,
                    updated_at_ms: command.now_ms,
                };
                let interest = ImageInterest {
                    interest: command.interest,
                    owner: command.identity.owner.clone(),
                    job: *job,
                    operation: command.identity.operation,
                    state: InterestState::Attached,
                    created_at_ms: command.now_ms,
                    withdrawn_at_ms: None,
                };
                let receipt = OperationReceipt {
                    owner: command.identity.owner.clone(),
                    operation: command.identity.operation,
                    fingerprint: command.identity.fingerprint,
                    job: *job,
                    interest: command.interest,
                };
                inner.jobs.insert(*job, stored.clone());
                inner.interests.insert(command.interest, interest.clone());
                inner.receipts.insert(key, receipt.clone());
                Ok(AcceptOutcome::Created(OperationView {
                    receipt,
                    job: stored,
                    interest,
                }))
            }
            AcceptTarget::Join { job } | AcceptTarget::JoinReady { job } => {
                let mut stored = inner.jobs.get(job).cloned().ok_or(JournalError::NotFound)?;
                if stored.owner != command.identity.owner {
                    return Err(JournalError::NotFound);
                }
                if stored.state == JobState::Retired {
                    return Err(JournalError::JoinRefused(JoinRefusal::Retired));
                }
                if is_expired(stored.absolute_deadline_ms, command.now_ms) {
                    return Err(JournalError::JoinRefused(JoinRefusal::Expired));
                }
                if stored.sharing != command.sharing {
                    return Err(JournalError::JoinRefused(JoinRefusal::SharingMismatch));
                }
                if stored.intent != command.intent {
                    return Err(JournalError::JoinRefused(JoinRefusal::IntentMismatch));
                }
                if stored.config != command.config {
                    return Err(JournalError::JoinRefused(JoinRefusal::ConfigMismatch));
                }
                if inner.receipts.contains_key(&key)
                    || inner.interests.contains_key(&command.interest)
                {
                    return Err(JournalError::IdentityConflict);
                }
                stored.revision += 1;
                stored.updated_at_ms = command.now_ms;
                let interest = ImageInterest {
                    interest: command.interest,
                    owner: command.identity.owner.clone(),
                    job: *job,
                    operation: command.identity.operation,
                    state: InterestState::Attached,
                    created_at_ms: command.now_ms,
                    withdrawn_at_ms: None,
                };
                let receipt = OperationReceipt {
                    owner: command.identity.owner.clone(),
                    operation: command.identity.operation,
                    fingerprint: command.identity.fingerprint,
                    job: *job,
                    interest: command.interest,
                };
                inner.jobs.insert(*job, stored.clone());
                inner.interests.insert(command.interest, interest.clone());
                inner.receipts.insert(key, receipt.clone());
                Ok(AcceptOutcome::Joined(OperationView {
                    receipt,
                    job: stored,
                    interest,
                }))
            }
        }
    }

    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        let mut inner = self.locked();
        inner.calls.push("lookup".to_owned());
        if let Some(error) = inner.fail_lookup.take() {
            return Err(error);
        }
        let key = (owner_scope_bytes(owner), operation);
        let Some(receipt) = inner.receipts.get(&key).cloned() else {
            return Ok(None);
        };
        let job = inner
            .jobs
            .get(&receipt.job)
            .cloned()
            .ok_or(JournalError::InvalidRecord)?;
        if job.owner != *owner {
            return Err(JournalError::InvalidRecord);
        }
        let interest = inner
            .interests
            .get(&receipt.interest)
            .cloned()
            .ok_or(JournalError::InvalidRecord)?;
        Ok(Some(OperationView {
            receipt,
            job,
            interest,
        }))
    }

    async fn read_job(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<ImageJob>, JournalError> {
        let mut inner = self.locked();
        inner.calls.push("read_job".to_owned());
        let stored = inner.jobs.get(&job).cloned();
        Ok(stored.filter(|job| &job.owner == owner))
    }

    async fn freeze_recipe(&self, _command: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        self.locked().calls.push("freeze".to_owned());
        Err(JournalError::Unavailable)
    }

    async fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<WithdrawOutcome, JournalError> {
        let mut inner = self.locked();
        inner.calls.push("withdraw".to_owned());
        if let Some(error) = inner.fail_withdraw.take() {
            return Err(error);
        }
        if now_ms <= 0 {
            return Err(JournalError::InvalidRecord);
        }
        let scope = owner_scope_bytes(owner);
        let current = inner
            .interests
            .get(&interest)
            .cloned()
            .ok_or(JournalError::NotFound)?;
        if owner_scope_bytes(&current.owner) != scope {
            return Err(JournalError::NotFound);
        }
        let mut job = inner
            .jobs
            .get(&current.job)
            .cloned()
            .ok_or(JournalError::InvalidRecord)?;
        if job.owner != *owner {
            return Err(JournalError::InvalidRecord);
        }
        if current.state == InterestState::Withdrawn {
            return Ok(WithdrawOutcome::AlreadyWithdrawn(Withdrawal {
                attached_remaining: Self::attached_count(&inner, &job.job),
                job_revision: job.revision,
                interest: current,
            }));
        }
        let mut updated = current;
        updated.state = InterestState::Withdrawn;
        updated.withdrawn_at_ms = Some(now_ms);
        inner.interests.insert(interest, updated.clone());
        let attached_remaining = Self::attached_count(&inner, &job.job);
        job.state = if attached_remaining == 0 {
            JobState::Retired
        } else {
            job.state
        };
        job.revision += 1;
        job.updated_at_ms = now_ms;
        inner.jobs.insert(job.job, job.clone());
        Ok(WithdrawOutcome::Withdrawn(Withdrawal {
            attached_remaining,
            job_revision: job.revision,
            interest: updated,
        }))
    }
}

type Service = ImageService<FakeJournal, FakeClock, FakeIds, FakeAuth>;

struct Handles {
    journal: FakeJournal,
    clock: FakeClock,
    ids: FakeIds,
    auth: FakeAuth,
    principal: TestPrincipal,
}

fn setup(owner: ImageOwner, config: ImageConfig) -> (Service, Handles) {
    setup_with_policy(owner, config, StoredPolicy::AllowAll)
}

fn setup_with_policy(
    owner: ImageOwner,
    config: ImageConfig,
    stored_policy: StoredPolicy,
) -> (Service, Handles) {
    let journal = FakeJournal::new();
    let clock = FakeClock::new(START_MS);
    let ids = FakeIds::new();
    let auth = FakeAuth {
        request_owner: owner,
        coarse_owner: None,
        generate_denied: false,
        stored_policy,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let service = ImageService::new(
        journal.clone(),
        config,
        clock.clone(),
        ids.clone(),
        auth.clone(),
    )
    .unwrap();
    let handles = Handles {
        journal,
        clock,
        ids,
        auth,
        principal: TestPrincipal,
    };
    (service, handles)
}

fn live_config() -> ImageConfig {
    ImageConfig::defaults().unwrap()
}

fn config_with(mutator: impl FnOnce(&mut ImageConfig)) -> ImageConfig {
    let mut config = ImageConfig::defaults().unwrap();
    mutator(&mut config);
    config
}

fn recorded_view(outcome: RequestOutcome) -> ImageDemandView {
    match outcome {
        RequestOutcome::Recorded(view) => view,
        RequestOutcome::Replayed(_) => panic!("expected Recorded, got Replayed"),
        RequestOutcome::Reused(_) => panic!("expected Recorded, got Reused"),
        RequestOutcome::ReusedArtifact(_) => panic!("expected Recorded, got ReusedArtifact"),
        RequestOutcome::Refused(_) => panic!("expected Recorded, got Refused"),
    }
}

fn request_operation_source(request: &ImageRequest) -> TransformSource {
    match &request.operation {
        ImageOperation::Transform(spec) => spec.source.clone(),
        _ => panic!("expected a transform request"),
    }
}

#[tokio::test]
async fn round_trip_per_operation_kind() {
    for kind in [Kind::Generate, Kind::Edit, Kind::Transform] {
        let owner = draft_owner();
        let operation = operation_id(0x21);
        let (service, handles) = setup(owner.clone(), live_config());
        let request = stamp(request_for(kind, owner.clone(), operation));
        let handle = handle_for(owner.clone(), operation);

        let view = recorded_view(service.request(&handles.principal, &request).await.unwrap());
        assert_eq!(view.handle, handle);
        assert_eq!(view.job_state, JobState::Planning);
        assert_eq!(view.interest_state, InterestState::Attached);
        assert_eq!(view.job_revision, 1);
        assert_eq!(view.absolute_deadline_ms, RECORDED_DEADLINE_MS);

        let read = service.read(&handles.principal, &handle).await.unwrap();
        assert_eq!(read, view);

        match service.request(&handles.principal, &request).await.unwrap() {
            RequestOutcome::Replayed(replayed) => assert_eq!(replayed, view),
            other => panic!("expected Replayed, got {other:?}"),
        }

        match service.watch(&handles.principal, &handle, 0).await.unwrap() {
            WatchObservation::Changed(changed) => assert_eq!(*changed, view),
            WatchObservation::Unchanged { .. } => panic!("expected Changed for stale revision"),
        }
        match service.watch(&handles.principal, &handle, 1).await.unwrap() {
            WatchObservation::Unchanged { revision } => assert_eq!(revision, 1),
            WatchObservation::Changed(_) => panic!("expected Unchanged at current revision"),
        }

        match service.cancel(&handles.principal, &handle).await.unwrap() {
            CancelOutcome::Withdrawn(receipt) => {
                assert_eq!(receipt.attached_remaining, 0);
                assert_eq!(receipt.view.interest_state, InterestState::Withdrawn);
                assert_eq!(receipt.view.job_state, JobState::Retired);
                assert_eq!(receipt.view.job_revision, 2);
            }
            CancelOutcome::AlreadyWithdrawn(_) => panic!("expected first Withdrawn"),
        }

        let retired = service.read(&handles.principal, &handle).await.unwrap();
        assert_eq!(retired.job_state, JobState::Retired);
        match service.cancel(&handles.principal, &handle).await.unwrap() {
            CancelOutcome::AlreadyWithdrawn(receipt) => {
                assert_eq!(receipt.attached_remaining, 0);
                assert_eq!(receipt.view.job_revision, 2);
            }
            CancelOutcome::Withdrawn(_) => panic!("expected idempotent AlreadyWithdrawn"),
        }

        match service.watch(&handles.principal, &handle, 1).await.unwrap() {
            WatchObservation::Changed(changed) => assert_eq!(changed.job_revision, 2),
            WatchObservation::Unchanged { .. } => panic!("expected Changed after withdrawal"),
        }

        match service.request(&handles.principal, &request).await.unwrap() {
            RequestOutcome::Replayed(replayed) => {
                assert_eq!(replayed.job_state, JobState::Retired);
                assert_eq!(replayed.interest_state, InterestState::Withdrawn);
            }
            other => panic!("retired receipts stay readable, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn denial_before_mutation() {
    let owner = draft_owner();
    let (service, handles) = setup(owner.clone(), live_config());
    let mut withheld = stamp(generate_request(owner.clone(), operation_id(0x21)));
    withheld.audience = Audience::Withheld;
    assert_eq!(
        service
            .request(&handles.principal, &withheld)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRequest {
            cause: RequestCause::WithheldAudience
        }
    );

    let mut profile = stamp(generate_request(owner.clone(), operation_id(0x22)));
    profile.output = local_output();
    assert_eq!(
        service
            .request(&handles.principal, &profile)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRequest {
            cause: RequestCause::ProfileMismatch
        }
    );

    let mut tampered = generate_request(owner.clone(), operation_id(0x23));
    tampered.identity.fingerprint = [0xAA; 32];
    assert_eq!(
        service
            .request(&handles.principal, &tampered)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRequest {
            cause: RequestCause::FingerprintMismatch
        }
    );

    let forged = stamp(generate_request(other_draft_owner(), operation_id(0x24)));
    assert_eq!(
        service
            .request(&handles.principal, &forged)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert!(handles.journal.calls().is_empty());

    let tightened = config_with(|config| {
        config.request_limits.max_prompt_chars = 10;
        config.request_limits.max_references = 1;
    });
    let (tight, tight_handles) = setup(owner.clone(), tightened);
    let prompt = stamp(generate_request(owner.clone(), operation_id(0x25)));
    assert_eq!(
        tight
            .request(&tight_handles.principal, &prompt)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRequest {
            cause: RequestCause::PromptTooLong
        }
    );
    assert_eq!(tight_handles.journal.calls(), vec!["lookup".to_owned()]);

    let mut layered = transform_request(owner, operation_id(0x26));
    layered.operation = ImageOperation::Transform(TransformSpec {
        source: request_operation_source(&layered),
        steps: vec![ImageTransform::Composite(Vec::new())],
    });
    assert_eq!(
        tight
            .request(&tight_handles.principal, &layered)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRequest {
            cause: RequestCause::InvalidTransform
        }
    );
    assert!(
        tight_handles
            .journal
            .calls()
            .iter()
            .all(|call| call != "accept")
    );
}

#[tokio::test]
async fn forged_owner_and_wrong_owner_reveal_nothing() {
    let owner = draft_owner();
    let (service, handles) = setup(owner.clone(), live_config());
    let missing = handle_for(other_draft_owner(), operation_id(0x21));
    assert_eq!(
        service
            .read(&handles.principal, &missing)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert_eq!(
        service
            .cancel(&handles.principal, &missing)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert!(handles.journal.calls().is_empty());

    let seeder = FakeAuth::permitting(other_draft_owner());
    let seeder_service = ImageService::new(
        handles.journal.clone(),
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        seeder,
    )
    .unwrap();
    let seeded = stamp(generate_request(other_draft_owner(), operation_id(0x22)));
    assert!(matches!(
        seeder_service
            .request(&TestPrincipal, &seeded)
            .await
            .unwrap(),
        RequestOutcome::Recorded(_)
    ));
    let seeded_handle = handle_for(other_draft_owner(), operation_id(0x22));
    assert_eq!(
        service
            .read(&handles.principal, &seeded_handle)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    let auth_calls = handles.auth.auth_calls();
    assert!(auth_calls.contains(&"owner-read".to_owned()));
    assert!(!auth_calls.contains(&"stored-read".to_owned()));
}

#[tokio::test]
async fn stored_audience_denied_on_replay_and_read() {
    let owner = run_owner();
    let operation = operation_id(0x21);
    let (service, handles) =
        setup_with_policy(owner.clone(), live_config(), StoredPolicy::PublicOnly);
    let mut request = stamp(generate_request(owner.clone(), operation));
    request.audience = Audience::OwnerPrivate(account_id(0x77));
    let request = stamp(request);
    let seeder = ImageService::new(
        handles.journal.clone(),
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        FakeAuth::permitting(owner.clone()),
    )
    .unwrap();
    let view = recorded_view(seeder.request(&handles.principal, &request).await.unwrap());
    let handle = handle_for(owner, operation);
    assert_eq!(view.handle, handle);

    assert_eq!(
        service
            .request(&handles.principal, &request)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert_eq!(
        service.read(&handles.principal, &handle).await.unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
}

#[tokio::test]
async fn stored_denial_covers_replay_read_watch_and_cancel() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &request).await.unwrap());

    let denying = FakeAuth {
        request_owner: owner.clone(),
        coarse_owner: None,
        generate_denied: false,
        stored_policy: StoredPolicy::DenyAll,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let denied = ImageService::new(
        handles.journal.clone(),
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        denying,
    )
    .unwrap();
    let handle = handle_for(owner, operation);
    assert_eq!(
        denied
            .request(&handles.principal, &request)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert_eq!(
        denied.read(&handles.principal, &handle).await.unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert_eq!(
        denied
            .watch(&handles.principal, &handle, 0)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert_eq!(
        denied
            .cancel(&handles.principal, &handle)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
}

#[tokio::test]
async fn changed_intent_conflicts_and_stale_fingerprint_is_invalid() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &request).await.unwrap());

    let mut replay_mode = request.clone();
    replay_mode.execution = ExecutionMode::Replay;
    let replay_mode = stamp(replay_mode);
    assert_eq!(
        service
            .request(&handles.principal, &replay_mode)
            .await
            .unwrap_err(),
        ImageServiceError::Conflict
    );

    let mut fresh_nonce = request.clone();
    fresh_nonce.freshness = Freshness::Regenerate {
        nonce: operation_id(0x99),
    };
    let fresh_nonce = stamp(fresh_nonce);
    assert_eq!(
        service
            .request(&handles.principal, &fresh_nonce)
            .await
            .unwrap_err(),
        ImageServiceError::Conflict
    );

    let mut changed_prompt = request.clone();
    changed_prompt.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
        references: Vec::new(),
    });
    let changed_prompt = stamp(changed_prompt);
    assert_eq!(
        service
            .request(&handles.principal, &changed_prompt)
            .await
            .unwrap_err(),
        ImageServiceError::Conflict
    );

    let mut stale_print = changed_prompt;
    stale_print.identity.fingerprint = request.identity.fingerprint;
    assert_eq!(
        service
            .request(&handles.principal, &stale_print)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRequest {
            cause: RequestCause::FingerprintMismatch
        }
    );
}

#[tokio::test]
async fn replay_survives_config_change_deadline_and_retirement() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    let view = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    let handle = handle_for(owner.clone(), operation);

    let changed = config_with(|config| {
        config.mode = GlobalMode::Off;
        config.request_limits.max_prompt_chars = 5;
        config.request_limits.max_references = 1;
        config.scheduler.job_deadline_secs = 60;
        config.scheduler.stage_deadline_secs = 45;
    });
    let switched = ImageService::new(
        handles.journal.clone(),
        changed,
        handles.clock.clone(),
        handles.ids.clone(),
        handles.auth.clone(),
    )
    .unwrap();
    match switched
        .request(&handles.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Replayed(replayed) => {
            assert_eq!(replayed.absolute_deadline_ms, RECORDED_DEADLINE_MS);
            assert_eq!(replayed.job_revision, view.job_revision);
        }
        other => panic!("tightened config must not break old replay, got {other:?}"),
    }
    let read = switched.read(&handles.principal, &handle).await.unwrap();
    assert_eq!(read.absolute_deadline_ms, RECORDED_DEADLINE_MS);

    match service.cancel(&handles.principal, &handle).await.unwrap() {
        CancelOutcome::Withdrawn(_) => {}
        CancelOutcome::AlreadyWithdrawn(_) => panic!("expected first Withdrawn"),
    }
    match switched
        .request(&handles.principal, &request)
        .await
        .unwrap()
    {
        RequestOutcome::Replayed(replayed) => {
            assert_eq!(replayed.job_state, JobState::Retired);
            assert_eq!(replayed.absolute_deadline_ms, RECORDED_DEADLINE_MS);
        }
        other => panic!("retired receipts stay readable, got {other:?}"),
    }
}

#[tokio::test]
async fn current_limit_tightening_applies_to_new_work_only() {
    let owner = draft_owner();
    let (service, handles) = setup(owner.clone(), live_config());
    let mut recorded = generate_request(owner.clone(), operation_id(0x21));
    recorded.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("two references", 3).unwrap(),
        references: vec![source_ref(), source_ref()],
    });
    let recorded = stamp(recorded);
    recorded_view(
        service
            .request(&handles.principal, &recorded)
            .await
            .unwrap(),
    );

    let tightened = config_with(|config| {
        config.request_limits.max_references = 1;
    });
    let switched = ImageService::new(
        handles.journal.clone(),
        tightened,
        handles.clock.clone(),
        handles.ids.clone(),
        handles.auth.clone(),
    )
    .unwrap();
    match switched
        .request(&handles.principal, &recorded)
        .await
        .unwrap()
    {
        RequestOutcome::Replayed(_) => {}
        other => panic!("old replay must survive tightening, got {other:?}"),
    }
    let mut fresh = generate_request(owner, operation_id(0x22));
    fresh.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("two references", 3).unwrap(),
        references: vec![source_ref(), source_ref()],
    });
    let fresh = stamp(fresh);
    assert_eq!(
        switched
            .request(&handles.principal, &fresh)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRequest {
            cause: RequestCause::TooManyReferences
        }
    );
}

#[tokio::test]
async fn shared_job_withdraws_one_interest_without_retiring_others() {
    let owner = draft_owner();
    let first = operation_id(0x21);
    let second = operation_id(0x22);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), first));
    let recorded = recorded_view(service.request(&handles.principal, &request).await.unwrap());

    let interest = handles.ids.next_interest_id().unwrap();
    let joined = handles
        .journal
        .join_for_test(
            &owner,
            second,
            request.identity.fingerprint,
            recorded.job,
            interest,
            START_MS,
        )
        .await
        .unwrap();
    assert_eq!(joined.job.job, recorded.job);

    let first_handle = handle_for(owner.clone(), first);
    match service
        .cancel(&handles.principal, &first_handle)
        .await
        .unwrap()
    {
        CancelOutcome::Withdrawn(receipt) => {
            assert_eq!(receipt.attached_remaining, 1);
            assert_eq!(receipt.view.job_state, JobState::Planning);
        }
        CancelOutcome::AlreadyWithdrawn(_) => panic!("expected first Withdrawn"),
    }

    let second_handle = handle_for(owner, second);
    let remaining = service
        .read(&handles.principal, &second_handle)
        .await
        .unwrap();
    assert_eq!(remaining.interest_state, InterestState::Attached);
    assert_eq!(remaining.job_state, JobState::Planning);
    let withdrawn = service
        .read(&handles.principal, &first_handle)
        .await
        .unwrap();
    assert_eq!(withdrawn.interest_state, InterestState::Withdrawn);
}

#[tokio::test]
async fn cancel_uses_the_stored_interest_and_reports_stable_revisions() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    let recorded = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    let handle = handle_for(owner, operation);

    match service.cancel(&handles.principal, &handle).await.unwrap() {
        CancelOutcome::Withdrawn(receipt) => {
            assert_eq!(receipt.view.interest, recorded.interest);
            assert_eq!(receipt.view.job, recorded.job);
            assert_eq!(receipt.attached_remaining, 0);
            assert_eq!(receipt.view.job_revision, 2);
        }
        CancelOutcome::AlreadyWithdrawn(_) => panic!("expected first Withdrawn"),
    }
    match service.cancel(&handles.principal, &handle).await.unwrap() {
        CancelOutcome::AlreadyWithdrawn(receipt) => {
            assert_eq!(receipt.view.interest, recorded.interest);
            assert_eq!(receipt.attached_remaining, 0);
            assert_eq!(receipt.view.job_revision, 2);
        }
        CancelOutcome::Withdrawn(_) => panic!("expected idempotent AlreadyWithdrawn"),
    }
}

#[tokio::test]
async fn unknown_accept_resolves_through_lookup_without_duplicating() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    handles.journal.fail_accept_once(JournalError::Unknown);

    match service
        .request(&handles.principal, &request)
        .await
        .unwrap_err()
    {
        ImageServiceError::Unknown { handle } => {
            assert_eq!(handle, handle_for(owner.clone(), operation));
        }
        other => panic!("expected Unknown, got {other:?}"),
    }
    assert_eq!(handles.journal.receipts_len(), 0);

    let view = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    assert_eq!(view.job_revision, 1);
    assert_eq!(handles.journal.receipts_len(), 1);
}

#[tokio::test]
async fn lookup_unknown_surfaces_the_handle() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let handle = handle_for(owner, operation);
    handles.journal.fail_lookup_once(JournalError::Unknown);
    assert_eq!(
        service.read(&handles.principal, &handle).await.unwrap_err(),
        ImageServiceError::Unknown {
            handle: handle.clone()
        }
    );
    handles.journal.fail_lookup_once(JournalError::Unknown);
    assert_eq!(
        service
            .watch(&handles.principal, &handle, 0)
            .await
            .unwrap_err(),
        ImageServiceError::Unknown { handle }
    );
}

#[tokio::test]
async fn corrupt_snapshot_is_an_invalid_record() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    let view = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    handles.journal.corrupt_intent(view.job);

    let handle = handle_for(owner, operation);
    assert_eq!(
        service
            .request(&handles.principal, &request)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRecord
    );
    assert_eq!(
        service.read(&handles.principal, &handle).await.unwrap_err(),
        ImageServiceError::InvalidRecord
    );
}

#[tokio::test]
async fn diagnostics_never_carry_prompt_text() {
    const SENTINEL: &str = "SENTINEL_PROMPT_XYZ_123";
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let mut request = generate_request(owner.clone(), operation);
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new(SENTINEL, 3).unwrap(),
        references: Vec::new(),
    });
    let request = stamp(request);

    let mut rendered = Vec::new();
    let view = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    rendered.push(format!("{view:?}"));
    let handle = handle_for(owner, operation);
    rendered.push(format!(
        "{:?}",
        service.read(&handles.principal, &handle).await.unwrap()
    ));
    match service.request(&handles.principal, &request).await.unwrap() {
        RequestOutcome::Replayed(replayed) => rendered.push(format!("{replayed:?}")),
        other => panic!("expected Replayed, got {other:?}"),
    }
    rendered.push(format!(
        "{:?}",
        RequestOutcome::Refused(Refusal::ServiceOff)
    ));
    for error in [
        ImageServiceError::InvalidRequest {
            cause: RequestCause::FingerprintMismatch,
        },
        ImageServiceError::NotAuthorizedOrNotFound,
        ImageServiceError::Conflict,
        ImageServiceError::InvalidRecord,
        ImageServiceError::Unavailable,
        ImageServiceError::Unknown { handle },
    ] {
        rendered.push(format!("{error:?}"));
    }
    for text in rendered {
        assert!(
            !text.contains(SENTINEL),
            "diagnostic leaked prompt text: {text}"
        );
    }
}

#[tokio::test]
async fn lookup_only_modes_refuse_without_side_effects() {
    struct Case {
        kind: Kind,
        execution: ExecutionMode,
        configure: fn(&mut ImageConfig),
        refusal: Option<Refusal>,
    }
    let cases = [
        Case {
            kind: Kind::Generate,
            execution: ExecutionMode::PreparedOnly,
            configure: |_| {},
            refusal: Some(Refusal::PreparedMiss),
        },
        Case {
            kind: Kind::Generate,
            execution: ExecutionMode::Replay,
            configure: |_| {},
            refusal: Some(Refusal::ReplayMiss),
        },
        Case {
            kind: Kind::Edit,
            execution: ExecutionMode::Replay,
            configure: |_| {},
            refusal: Some(Refusal::ReplayMiss),
        },
        Case {
            kind: Kind::Transform,
            execution: ExecutionMode::PreparedOnly,
            configure: |_| {},
            refusal: Some(Refusal::PreparedMiss),
        },
        Case {
            kind: Kind::Generate,
            execution: ExecutionMode::Live,
            configure: |config| config.mode = GlobalMode::CacheOnly,
            refusal: Some(Refusal::CacheOnlyMiss),
        },
        Case {
            kind: Kind::Transform,
            execution: ExecutionMode::Live,
            configure: |config| config.mode = GlobalMode::CacheOnly,
            refusal: Some(Refusal::CacheOnlyMiss),
        },
        Case {
            kind: Kind::Generate,
            execution: ExecutionMode::Live,
            configure: |config| config.mode = GlobalMode::Off,
            refusal: Some(Refusal::ServiceOff),
        },
        Case {
            kind: Kind::Transform,
            execution: ExecutionMode::Live,
            configure: |config| config.mode = GlobalMode::Off,
            refusal: Some(Refusal::ServiceOff),
        },
        Case {
            kind: Kind::Generate,
            execution: ExecutionMode::Live,
            configure: |config| {
                config.purposes[Purpose::ScenePlate.index()].enabled = false;
            },
            refusal: Some(Refusal::PurposeDisabled),
        },
        Case {
            kind: Kind::Transform,
            execution: ExecutionMode::Live,
            configure: |config| {
                config.purposes[Purpose::ReferencePrep.index()].enabled = false;
            },
            refusal: Some(Refusal::PurposeDisabled),
        },
        Case {
            kind: Kind::Transform,
            execution: ExecutionMode::Live,
            configure: |config| config.transforms.enabled = false,
            refusal: Some(Refusal::TransformsDisabled),
        },
        Case {
            kind: Kind::Generate,
            execution: ExecutionMode::Live,
            configure: |config| {
                config.backends[BackendId::CodexCli.index()].enabled = false;
            },
            refusal: Some(Refusal::BackendDisabled),
        },
        Case {
            kind: Kind::Generate,
            execution: ExecutionMode::Live,
            configure: |config| config.mode = GlobalMode::LocalOnly,
            refusal: Some(Refusal::ProviderStageNotPermitted),
        },
        Case {
            kind: Kind::Transform,
            execution: ExecutionMode::Live,
            configure: |config| config.mode = GlobalMode::LocalOnly,
            refusal: None,
        },
    ];
    for (index, case) in cases.iter().enumerate() {
        let owner = draft_owner();
        let config = config_with(case.configure);
        let (service, handles) = setup(owner.clone(), config);
        let mut request = stamp(request_for(case.kind, owner, operation_id(0x21)));
        request.execution = case.execution;
        let request = stamp(request);
        match (
            service.request(&handles.principal, &request).await.unwrap(),
            case.refusal,
        ) {
            (RequestOutcome::Refused(refusal), Some(expected)) => {
                assert_eq!(refusal, expected, "case {index}");
            }
            (RequestOutcome::Recorded(_), None) => {}
            (outcome, expected) => {
                panic!("case {index}: got {outcome:?}, want refusal {expected:?}");
            }
        }
        if case.refusal.is_some() {
            assert_eq!(handles.ids.jobs_minted(), 0, "case {index}");
            assert_eq!(handles.ids.interests_minted(), 0, "case {index}");
            for call in handles.journal.calls() {
                assert!(
                    call != "accept" && call != "freeze" && call != "withdraw",
                    "case {index}: journal mutated through {call}"
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_exact_replay_records_once() {
    use std::collections::HashSet;

    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, _) = setup(owner.clone(), live_config());
    let service = Arc::new(service);
    let request = stamp(generate_request(owner, operation));

    let first = tokio::spawn({
        let service = service.clone();
        let request = request.clone();
        async move { service.request(&TestPrincipal, &request).await }
    });
    let second = tokio::spawn({
        let service = service.clone();
        let request = request.clone();
        async move { service.request(&TestPrincipal, &request).await }
    });
    let outcomes = [
        first.await.unwrap().unwrap(),
        second.await.unwrap().unwrap(),
    ];
    let mut kinds = HashSet::new();
    let mut jobs = HashSet::new();
    for outcome in outcomes {
        match outcome {
            RequestOutcome::Recorded(view) => {
                kinds.insert("recorded");
                jobs.insert(view.job);
            }
            RequestOutcome::Replayed(view) => {
                kinds.insert("replayed");
                jobs.insert(view.job);
            }
            RequestOutcome::Refused(_) => panic!("concurrent Live demand must not refuse"),
            RequestOutcome::Reused(_) | RequestOutcome::ReusedArtifact(_) => {
                panic!("no ready result exists in this fixture")
            }
        }
    }
    assert_eq!(kinds, HashSet::from(["recorded", "replayed"]));
    assert_eq!(jobs.len(), 1);
    let handle = handle_for(draft_owner(), operation);
    let reread = service.read(&TestPrincipal, &handle).await.unwrap();
    assert_eq!(reread.job_revision, 1);
}

#[tokio::test]
async fn stored_auth_precedes_conflict_on_lookup_replay() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let initial = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &initial).await.unwrap());

    let mut forked = generate_request(owner.clone(), operation);
    forked.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
        references: Vec::new(),
    });
    let forked = stamp(forked);

    let denying = FakeAuth {
        request_owner: owner.clone(),
        coarse_owner: None,
        generate_denied: false,
        stored_policy: StoredPolicy::DenyAll,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let denied = ImageService::new(
        handles.journal.clone(),
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        denying.clone(),
    )
    .unwrap();
    assert_eq!(
        denied
            .request(&handles.principal, &forked)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert!(denying.auth_calls().contains(&"stored-read".to_owned()));
    assert_eq!(
        service
            .request(&handles.principal, &forked)
            .await
            .unwrap_err(),
        ImageServiceError::Conflict
    );
}

#[tokio::test]
async fn stored_auth_precedes_comparison_on_concurrent_accept_replay() {
    #[derive(Clone)]
    struct MissOnceJournal {
        inner: FakeJournal,
        missed: Arc<AtomicBool>,
    }

    impl ImageJournal for MissOnceJournal {
        async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
            self.inner.accept(command).await
        }

        async fn lookup_operation(
            &self,
            owner: &ImageOwner,
            operation: OperationId,
        ) -> Result<Option<OperationView>, JournalError> {
            if !self.missed.swap(true, Ordering::SeqCst) {
                return Ok(None);
            }
            self.inner.lookup_operation(owner, operation).await
        }

        async fn read_job(
            &self,
            owner: &ImageOwner,
            job: ImageJobId,
        ) -> Result<Option<ImageJob>, JournalError> {
            self.inner.read_job(owner, job).await
        }

        async fn freeze_recipe(
            &self,
            command: &FreezeCommand,
        ) -> Result<FreezeOutcome, JournalError> {
            self.inner.freeze_recipe(command).await
        }

        async fn withdraw_interest(
            &self,
            owner: &ImageOwner,
            interest: ImageInterestId,
            now_ms: i64,
        ) -> Result<WithdrawOutcome, JournalError> {
            self.inner.withdraw_interest(owner, interest, now_ms).await
        }
    }

    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &request).await.unwrap());

    let denying = FakeAuth {
        request_owner: owner.clone(),
        coarse_owner: None,
        generate_denied: false,
        stored_policy: StoredPolicy::DenyAll,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let raced = ImageService::new(
        MissOnceJournal {
            inner: handles.journal.clone(),
            missed: Arc::new(AtomicBool::new(false)),
        },
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        denying.clone(),
    )
    .unwrap();
    assert_eq!(
        raced
            .request(&handles.principal, &request)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert!(denying.auth_calls().contains(&"stored-read".to_owned()));
}

#[tokio::test]
async fn concurrent_different_intent_denied_before_conflict_on_accept_race() {
    #[derive(Clone)]
    struct MissOnceJournal {
        inner: FakeJournal,
        missed: Arc<AtomicBool>,
    }

    impl ImageJournal for MissOnceJournal {
        async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
            self.inner.accept(command).await
        }

        async fn lookup_operation(
            &self,
            owner: &ImageOwner,
            operation: OperationId,
        ) -> Result<Option<OperationView>, JournalError> {
            if !self.missed.swap(true, Ordering::SeqCst) {
                return Ok(None);
            }
            self.inner.lookup_operation(owner, operation).await
        }

        async fn read_job(
            &self,
            owner: &ImageOwner,
            job: ImageJobId,
        ) -> Result<Option<ImageJob>, JournalError> {
            self.inner.read_job(owner, job).await
        }

        async fn freeze_recipe(
            &self,
            command: &FreezeCommand,
        ) -> Result<FreezeOutcome, JournalError> {
            self.inner.freeze_recipe(command).await
        }

        async fn withdraw_interest(
            &self,
            owner: &ImageOwner,
            interest: ImageInterestId,
            now_ms: i64,
        ) -> Result<WithdrawOutcome, JournalError> {
            self.inner.withdraw_interest(owner, interest, now_ms).await
        }
    }

    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let initial = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &initial).await.unwrap());

    let mut forked = generate_request(owner.clone(), operation);
    forked.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
        references: Vec::new(),
    });
    let forked = stamp(forked);

    let denying = FakeAuth {
        request_owner: owner.clone(),
        coarse_owner: None,
        generate_denied: false,
        stored_policy: StoredPolicy::DenyAll,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let raced = ImageService::new(
        MissOnceJournal {
            inner: handles.journal.clone(),
            missed: Arc::new(AtomicBool::new(false)),
        },
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        denying.clone(),
    )
    .unwrap();
    assert_eq!(
        raced
            .request(&handles.principal, &forked)
            .await
            .unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    assert!(denying.auth_calls().contains(&"stored-read".to_owned()));
    assert_eq!(handles.journal.receipts_len(), 1);
}

#[tokio::test]
async fn concurrent_different_intent_conflicts_for_allowed_reader_on_accept_race() {
    #[derive(Clone)]
    struct MissOnceJournal {
        inner: FakeJournal,
        missed: Arc<AtomicBool>,
    }

    impl ImageJournal for MissOnceJournal {
        async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
            self.inner.accept(command).await
        }

        async fn lookup_operation(
            &self,
            owner: &ImageOwner,
            operation: OperationId,
        ) -> Result<Option<OperationView>, JournalError> {
            if !self.missed.swap(true, Ordering::SeqCst) {
                return Ok(None);
            }
            self.inner.lookup_operation(owner, operation).await
        }

        async fn read_job(
            &self,
            owner: &ImageOwner,
            job: ImageJobId,
        ) -> Result<Option<ImageJob>, JournalError> {
            self.inner.read_job(owner, job).await
        }

        async fn freeze_recipe(
            &self,
            command: &FreezeCommand,
        ) -> Result<FreezeOutcome, JournalError> {
            self.inner.freeze_recipe(command).await
        }

        async fn withdraw_interest(
            &self,
            owner: &ImageOwner,
            interest: ImageInterestId,
            now_ms: i64,
        ) -> Result<WithdrawOutcome, JournalError> {
            self.inner.withdraw_interest(owner, interest, now_ms).await
        }
    }

    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let initial = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &initial).await.unwrap());

    let mut forked = generate_request(owner.clone(), operation);
    forked.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
        references: Vec::new(),
    });
    let forked = stamp(forked);

    let raced = ImageService::new(
        MissOnceJournal {
            inner: handles.journal.clone(),
            missed: Arc::new(AtomicBool::new(false)),
        },
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        handles.auth.clone(),
    )
    .unwrap();
    assert_eq!(
        raced
            .request(&handles.principal, &forked)
            .await
            .unwrap_err(),
        ImageServiceError::Conflict
    );
    assert!(
        handles
            .auth
            .auth_calls()
            .contains(&"stored-read".to_owned())
    );
    assert_eq!(handles.journal.receipts_len(), 1);
}

#[tokio::test]
async fn concurrent_conflict_reread_error_fails_closed_without_conflict() {
    #[derive(Clone)]
    struct FailRereadJournal {
        inner: FakeJournal,
        lookups: Arc<AtomicU64>,
    }

    impl ImageJournal for FailRereadJournal {
        async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
            self.inner.accept(command).await
        }

        async fn lookup_operation(
            &self,
            owner: &ImageOwner,
            operation: OperationId,
        ) -> Result<Option<OperationView>, JournalError> {
            let count = self.lookups.fetch_add(1, Ordering::SeqCst);
            if count == 0 {
                return Ok(None);
            }
            if count == 1 {
                return Err(JournalError::Unavailable);
            }
            self.inner.lookup_operation(owner, operation).await
        }

        async fn read_job(
            &self,
            owner: &ImageOwner,
            job: ImageJobId,
        ) -> Result<Option<ImageJob>, JournalError> {
            self.inner.read_job(owner, job).await
        }

        async fn freeze_recipe(
            &self,
            command: &FreezeCommand,
        ) -> Result<FreezeOutcome, JournalError> {
            self.inner.freeze_recipe(command).await
        }

        async fn withdraw_interest(
            &self,
            owner: &ImageOwner,
            interest: ImageInterestId,
            now_ms: i64,
        ) -> Result<WithdrawOutcome, JournalError> {
            self.inner.withdraw_interest(owner, interest, now_ms).await
        }
    }

    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let initial = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &initial).await.unwrap());

    let mut forked = generate_request(owner.clone(), operation);
    forked.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
        references: Vec::new(),
    });
    let forked = stamp(forked);

    let raced = ImageService::new(
        FailRereadJournal {
            inner: handles.journal.clone(),
            lookups: Arc::new(AtomicU64::new(0)),
        },
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        handles.auth.clone(),
    )
    .unwrap();
    assert_eq!(
        raced
            .request(&handles.principal, &forked)
            .await
            .unwrap_err(),
        ImageServiceError::Unavailable
    );
    assert_eq!(handles.journal.receipts_len(), 1);
}

#[tokio::test]
async fn canonical_fingerprint_stamps_record_and_replay_without_duplicate_encoding() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let mut request = generate_request(owner.clone(), operation);
    assert!(handles.journal.calls().is_empty());
    let fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert!(handles.journal.calls().is_empty());
    assert_eq!(handles.ids.jobs_minted(), 0);
    assert_eq!(handles.ids.interests_minted(), 0);
    request.identity.fingerprint = fingerprint;
    let recorded = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    match service.request(&handles.principal, &request).await.unwrap() {
        RequestOutcome::Replayed(replayed) => assert_eq!(replayed, recorded),
        other => panic!("expected Replayed, got {other:?}"),
    }

    let mut tampered = request.clone();
    tampered.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
        references: Vec::new(),
    });
    assert_eq!(
        service
            .request(&handles.principal, &tampered)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRequest {
            cause: RequestCause::FingerprintMismatch
        }
    );
    tampered.identity.fingerprint = service.canonical_fingerprint(&tampered).unwrap();
    assert_eq!(
        service
            .request(&handles.principal, &tampered)
            .await
            .unwrap_err(),
        ImageServiceError::Conflict
    );
}

#[tokio::test]
async fn zero_id_source_fails_without_writing() {
    #[derive(Clone)]
    struct ZeroIds {
        draws: Arc<AtomicU64>,
    }

    impl ServiceIds for ZeroIds {
        fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
            self.draws.fetch_add(1, Ordering::SeqCst);
            Err(ServiceIdError::InvalidGeneratedId)
        }

        fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
            self.draws.fetch_add(1, Ordering::SeqCst);
            Err(ServiceIdError::InvalidGeneratedId)
        }
    }

    let owner = draft_owner();
    let operation = operation_id(0x21);
    let journal = FakeJournal::new();
    let clock = FakeClock::new(START_MS);
    let zero = ZeroIds {
        draws: Arc::new(AtomicU64::new(0)),
    };
    let auth = FakeAuth::permitting(owner.clone());
    let service =
        ImageService::new(journal.clone(), live_config(), clock, zero.clone(), auth).unwrap();
    let request = stamp(generate_request(owner, operation));
    assert_eq!(
        service.request(&TestPrincipal, &request).await.unwrap_err(),
        ImageServiceError::Unavailable
    );
    assert_eq!(journal.calls(), vec!["lookup".to_owned()]);
    assert_eq!(zero.draws.load(Ordering::SeqCst), 1);

    let valid = FakeIds::new();
    let first_job = valid.next_job_id().unwrap();
    let second_job = valid.next_job_id().unwrap();
    assert_ne!(first_job, second_job);
    let first_interest = valid.next_interest_id().unwrap();
    let second_interest = valid.next_interest_id().unwrap();
    assert_ne!(first_interest, second_interest);
}

#[tokio::test]
async fn post_cancel_lookup_failure_returns_unknown_for_recovery() {
    #[derive(Clone)]
    struct FailRefreshJournal {
        inner: FakeJournal,
        lookups: Arc<AtomicU64>,
    }

    impl ImageJournal for FailRefreshJournal {
        async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
            self.inner.accept(command).await
        }

        async fn lookup_operation(
            &self,
            owner: &ImageOwner,
            operation: OperationId,
        ) -> Result<Option<OperationView>, JournalError> {
            let count = self.lookups.fetch_add(1, Ordering::SeqCst) + 1;
            if count == 2 {
                return Err(JournalError::Unavailable);
            }
            self.inner.lookup_operation(owner, operation).await
        }

        async fn read_job(
            &self,
            owner: &ImageOwner,
            job: ImageJobId,
        ) -> Result<Option<ImageJob>, JournalError> {
            self.inner.read_job(owner, job).await
        }

        async fn freeze_recipe(
            &self,
            command: &FreezeCommand,
        ) -> Result<FreezeOutcome, JournalError> {
            self.inner.freeze_recipe(command).await
        }

        async fn withdraw_interest(
            &self,
            owner: &ImageOwner,
            interest: ImageInterestId,
            now_ms: i64,
        ) -> Result<WithdrawOutcome, JournalError> {
            self.inner.withdraw_interest(owner, interest, now_ms).await
        }
    }

    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &request).await.unwrap());
    let handle = handle_for(owner.clone(), operation);

    let failing = ImageService::new(
        FailRefreshJournal {
            inner: handles.journal.clone(),
            lookups: Arc::new(AtomicU64::new(0)),
        },
        live_config(),
        handles.clock.clone(),
        handles.ids.clone(),
        handles.auth.clone(),
    )
    .unwrap();
    match failing
        .cancel(&handles.principal, &handle)
        .await
        .unwrap_err()
    {
        ImageServiceError::Unknown { handle: failed } => assert_eq!(failed, handle),
        other => panic!("expected Unknown after committed withdrawal, got {other:?}"),
    }
    let reread = service.read(&handles.principal, &handle).await.unwrap();
    assert_eq!(reread.interest_state, InterestState::Withdrawn);
    match service.cancel(&handles.principal, &handle).await.unwrap() {
        CancelOutcome::AlreadyWithdrawn(receipt) => {
            assert_eq!(receipt.view.interest_state, InterestState::Withdrawn);
        }
        CancelOutcome::Withdrawn(_) => panic!("expected idempotent AlreadyWithdrawn"),
    }
}

#[tokio::test]
async fn corrupt_linkage_is_invalid_record_without_mutation() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    let recorded = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    let handle = handle_for(owner.clone(), operation);

    {
        let mut inner = handles.journal.locked();
        let interest = inner.interests.get_mut(&recorded.interest).unwrap();
        interest.owner = other_draft_owner();
    }
    assert_eq!(
        service.read(&handles.principal, &handle).await.unwrap_err(),
        ImageServiceError::InvalidRecord
    );
    assert_eq!(
        service
            .cancel(&handles.principal, &handle)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRecord
    );
    let reread = service.read(&handles.principal, &handle).await.unwrap_err();
    assert_eq!(reread, ImageServiceError::InvalidRecord);
    assert!(!handles.journal.calls().contains(&"withdraw".to_owned()));
    assert_eq!(
        handles
            .journal
            .locked()
            .interests
            .get(&recorded.interest)
            .unwrap()
            .state,
        InterestState::Attached
    );

    let (fresh, fresh_handles) = setup(owner.clone(), live_config());
    let fresh_request = stamp(generate_request(owner.clone(), operation_id(0x22)));
    let fresh_recorded = recorded_view(
        fresh
            .request(&fresh_handles.principal, &fresh_request)
            .await
            .unwrap(),
    );
    {
        let other = stamp(generate_request(other_draft_owner(), operation_id(0x23)));
        let encoded = encode_intent(&other, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
        let mut inner = fresh_handles.journal.locked();
        let job = inner.jobs.get_mut(&fresh_recorded.job).unwrap();
        job.intent =
            VersionedSnapshot::new(1, encoded.bytes().to_vec(), PROTOCOL_MAX_SNAPSHOT_BYTES)
                .unwrap();
    }
    let fresh_handle = handle_for(owner, operation_id(0x22));
    assert_eq!(
        fresh
            .read(&fresh_handles.principal, &fresh_handle)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRecord
    );
}

#[tokio::test]
async fn cancellation_debug_omits_shared_interest_count() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    recorded_view(service.request(&handles.principal, &request).await.unwrap());
    let handle = handle_for(owner, operation);
    let outcome = service.cancel(&handles.principal, &handle).await.unwrap();
    let rendered = format!("{outcome:?}");
    assert!(!rendered.contains("attached_remaining"));
    match outcome {
        CancelOutcome::Withdrawn(receipt) | CancelOutcome::AlreadyWithdrawn(receipt) => {
            assert!(!format!("{receipt:?}").contains("attached_remaining"));
        }
    }
}

#[tokio::test]
async fn historical_config_remains_replayable_readable_watchable_and_cancellable() {
    let owner = draft_owner();
    let operation = operation_id(0x35);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    let recorded = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    let handle = handle_for(owner, operation);
    let historical = include_bytes!("../tests/fixtures/config-v1.bin");
    {
        let mut inner = handles.journal.locked();
        inner.jobs.get_mut(&recorded.job).unwrap().config =
            VersionedSnapshot::new(1, historical.to_vec(), PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    }
    assert!(matches!(
        service.request(&handles.principal, &request).await.unwrap(),
        RequestOutcome::Replayed(_)
    ));
    assert_eq!(
        service.read(&handles.principal, &handle).await.unwrap().job,
        recorded.job
    );
    assert!(matches!(
        service.watch(&handles.principal, &handle, 0).await.unwrap(),
        WatchObservation::Changed(_)
    ));
    assert!(matches!(
        service.cancel(&handles.principal, &handle).await.unwrap(),
        CancelOutcome::Withdrawn(_)
    ));
}

#[tokio::test]
async fn config_envelope_must_match_payload_even_when_both_versions_are_supported() {
    let owner = draft_owner();
    let operation = operation_id(0x36);
    let (service, handles) = setup(owner.clone(), live_config());
    let request = stamp(generate_request(owner.clone(), operation));
    let recorded = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    let handle = handle_for(owner, operation);
    {
        let mut inner = handles.journal.locked();
        let job = inner.jobs.get_mut(&recorded.job).unwrap();
        assert_eq!(
            job.config.bytes()[0],
            crate::intent_codec::CONFIG_CODEC_VERSION
        );
        job.config =
            VersionedSnapshot::new(1, job.config.bytes().to_vec(), PROTOCOL_MAX_SNAPSHOT_BYTES)
                .unwrap();
    }
    assert_eq!(
        service
            .request(&handles.principal, &request)
            .await
            .unwrap_err(),
        ImageServiceError::InvalidRecord
    );
    assert_eq!(
        service.read(&handles.principal, &handle).await.unwrap_err(),
        ImageServiceError::InvalidRecord
    );
}

#[derive(Clone)]
struct FakeResults {
    result: Arc<Mutex<Option<crate::publication::PublishedImage>>>,
    reads: Arc<AtomicU64>,
}
impl crate::publication::ImageResults for FakeResults {
    async fn read_result(
        &self,
        _owner: &ImageOwner,
        _job: &ImageJobId,
    ) -> Result<Option<crate::publication::PublishedImage>, crate::publication::PublicationError>
    {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.result.lock().unwrap().clone())
    }
}

#[tokio::test]
async fn published_results_are_read_after_stored_authorization_and_hidden_after_withdrawal() {
    let owner = draft_owner();
    let (service, handles) = setup(owner.clone(), live_config());
    let results = FakeResults {
        result: Arc::new(Mutex::new(None)),
        reads: Arc::new(AtomicU64::new(0)),
    };
    let service = service.with_results(results.clone());
    let request = stamp(generate_request(owner.clone(), operation_id(0x21)));
    let view = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    assert!(view.result.is_none());
    {
        let mut state = handles.journal.locked();
        state.jobs.get_mut(&view.job).unwrap().state = JobState::RecipeFrozen;
    }
    let intent =
        df_assets::ImageIntent::new("image/png", 1, 1, df_assets::ImageAlpha::Opaque).unwrap();
    *results.result.lock().unwrap() = Some(crate::publication::PublishedImage {
        job: view.job,
        stage: crate::execution::ExecutionStageId::from_bytes(&[1; 16]).unwrap(),
        dispatch: crate::execution::DispatchId::from_bytes(&[2; 16]).unwrap(),
        asset: AssetId::from_bytes(&[3; 16]).unwrap(),
        attempt: df_assets::AttemptId::from_bytes(&[4; 16]).unwrap(),
        recipe_digest: [5; 32],
        job_revision: 1,
        published_at_ms: START_MS,
        private_reconciliation: false,
        variants: vec![crate::publication::PublishedVariant {
            variant: df_assets::AssetVariant::Master,
            write: df_assets::WriteId::from_bytes(&[6; 16]).unwrap(),
            object_version: df_assets::ObjectVersion::new("immutable-v1".to_owned()).unwrap(),
            byte_len: 1,
            sha256: [7; 32],
            intent,
            alpha: df_assets::ImageAlpha::Opaque,
        }],
    });
    let read = service
        .read(&handles.principal, &view.handle)
        .await
        .unwrap();
    assert!(read.result.is_some());
    assert!(matches!(
        service
            .watch(&handles.principal, &view.handle, 0)
            .await
            .unwrap(),
        WatchObservation::Changed(view) if view.result.is_some()
    ));
    service
        .cancel(&handles.principal, &view.handle)
        .await
        .unwrap();
    let before = results.reads.load(Ordering::SeqCst);
    assert!(
        service
            .read(&handles.principal, &view.handle)
            .await
            .unwrap()
            .result
            .is_none()
    );
    assert_eq!(results.reads.load(Ordering::SeqCst), before);
    let denied = OperationHandle {
        owner: other_draft_owner(),
        operation: view.handle.operation,
    };
    assert_eq!(
        service.read(&handles.principal, &denied).await,
        Err(ImageServiceError::NotAuthorizedOrNotFound)
    );
    assert_eq!(results.reads.load(Ordering::SeqCst), before);
}

#[path = "service_tests/analysis.rs"]
mod analysis_boundary;

#[tokio::test]
async fn configured_prompt_above_former_protocol_count_fits_the_same_wire_budget() {
    let mut config = live_config();
    config.request_limits.max_prompt_chars = 300_000;
    let (service, handles) = setup(draft_owner(), config);
    let mut request = generate_request(draft_owner(), operation_id(0xda));
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new(&"x".repeat(262_145), 1).unwrap(),
        references: Vec::new(),
    });
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    let accepted = recorded_view(service.request(&handles.principal, &request).await.unwrap());
    assert_eq!(
        service
            .read(&handles.principal, &accepted.handle)
            .await
            .unwrap(),
        accepted
    );
    let encoded = encode_intent(&request, &PROTOCOL_LIMITS, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    assert!(encoded.bytes().len() < PROTOCOL_MAX_SNAPSHOT_BYTES);
    assert!(
        crate::intent_codec::decode_intent(
            &encoded.bytes()[..encoded.bytes().len() - 1],
            &PROTOCOL_LIMITS,
            PROTOCOL_MAX_SNAPSHOT_BYTES
        )
        .is_err()
    );
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new(&"x".repeat(PROTOCOL_MAX_SNAPSHOT_BYTES), 1).unwrap(),
        references: Vec::new(),
    });
    assert!(service.canonical_fingerprint(&request).is_err());
}

#[tokio::test]
async fn explicit_provider_moderation_requires_the_current_trusted_purpose_override() {
    for configured in [None, Some(crate::ProviderModeration::Low)] {
        let mut config = live_config();
        let mut request = generate_request(draft_owner(), operation_id(0xdb));
        config.purposes[request.purpose.purpose.index()].moderation_override = configured;
        request.operation = ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("fixture", 1)
                .unwrap()
                .with_pipeline(crate::ImagePipeline {
                    revision: 1,
                    rating: crate::ImageContentRating::Restrained,
                    moderation: Some(crate::ProviderModeration::Low),
                    creator: None,
                    inspection: None,
                })
                .unwrap(),
            references: Vec::new(),
        });
        let (service, handles) = setup(draft_owner(), config);
        request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
        let outcome = service.request(&handles.principal, &request).await;
        if configured.is_some() {
            assert!(matches!(outcome, Ok(RequestOutcome::Recorded(_))));
        } else {
            assert_eq!(
                outcome,
                Err(ImageServiceError::InvalidRequest {
                    cause: RequestCause::InvalidPipeline
                })
            );
        }
    }
}

#[path = "service_tests/visibility.rs"]
mod visibility;

#[tokio::test]
async fn a_principal_denied_generate_creates_no_paid_demand() {
    let mut auth = FakeAuth::permitting(draft_owner());
    auth.generate_denied = true;
    let journal = FakeJournal::new();
    let service = ImageService::new(
        journal.clone(),
        live_config(),
        FakeClock::new(START_MS),
        FakeIds::new(),
        auth,
    )
    .unwrap();
    let request = stamp(generate_request(draft_owner(), operation_id(0x31)));
    assert_eq!(
        service.request(&TestPrincipal, &request).await.unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound
    );
    let handle = handle_for(draft_owner(), operation_id(0x31));
    assert_eq!(
        service.read(&TestPrincipal, &handle).await.unwrap_err(),
        ImageServiceError::NotAuthorizedOrNotFound,
        "a denied request must leave no receipt behind"
    );
}

#[tokio::test]
async fn generate_permission_alone_records_new_demand() {
    let auth = FakeAuth::permitting(draft_owner());
    let service = ImageService::new(
        FakeJournal::new(),
        live_config(),
        FakeClock::new(START_MS),
        FakeIds::new(),
        auth,
    )
    .unwrap();
    let request = stamp(generate_request(draft_owner(), operation_id(0x32)));
    assert!(matches!(
        service.request(&TestPrincipal, &request).await.unwrap(),
        RequestOutcome::Recorded(_)
    ));
}

#[test]
fn invalid_request_display_is_a_lowercase_sentence_without_variant_names() {
    let error = ImageServiceError::InvalidRequest {
        cause: RequestCause::FingerprintMismatch,
    };
    assert_eq!(
        error.to_string(),
        "the request is invalid: the fingerprint is not the canonical intent hash"
    );
    for cause in [
        RequestCause::EmptyPrompt,
        RequestCause::PromptTooLong,
        RequestCause::TooManyReferences,
        RequestCause::WithheldAudience,
        RequestCause::InvalidLimit,
        RequestCause::ProfileMismatch,
        RequestCause::InvalidTransform,
        RequestCause::FingerprintMismatch,
        RequestCause::SnapshotTooLarge,
        RequestCause::Malformed,
        RequestCause::InvalidPipeline,
        RequestCause::InvalidArtwork,
    ] {
        let text = ImageServiceError::InvalidRequest { cause }.to_string();
        assert!(!text.chars().any(char::is_uppercase), "{text}");
    }
}
