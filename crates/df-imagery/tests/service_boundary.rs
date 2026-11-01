#![cfg(not(target_arch = "wasm32"))]

//! External-crate boundary test using only root public exports.
//! It implements the host authorizer without touching private codec modules
//! and stamps demand through the service-owned canonical fingerprint.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use df_imagery::{
    AcceptCommand, AcceptOutcome, AcceptTarget, AuthDenied, CancelOutcome, FreezeCommand,
    FreezeOutcome, ImageAuthorizer, ImageConfig, ImageDemandView, ImageInterest, ImageInterestId,
    ImageJob, ImageJobId, ImageJournal, ImageOwner, ImageRequest, ImageService, ImageServiceError,
    InterestState, JobState, JournalError, OperationHandle, OperationIdentity, OperationView,
    RequestOutcome, ServiceAction, ServiceClock, ServiceIdError, ServiceIds,
    StoredImageAuthorization,
};
use df_imagery::{
    AlphaHandling, Dimensions, Encoding, GenerateSpec, ImageOperation, LossyQuality, OutputProfile,
    Purpose, PurposedUse, RasterOutputSpec, TextPrompt, Urgency, VariantRole,
};
use df_imagery::{Audience, ExecutionMode, Freshness};
use df_types::{AccountId, OperationId};

fn account_id(byte: u8) -> AccountId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    AccountId::from_bytes(&bytes).unwrap()
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

fn base_request(owner: ImageOwner, operation: OperationId) -> ImageRequest {
    ImageRequest {
        identity: OperationIdentity {
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

#[derive(Clone)]
struct TestPrincipal;

#[derive(Clone)]
struct ExternalAuth {
    owner: ImageOwner,
    calls: Arc<Mutex<Vec<String>>>,
}

impl ImageAuthorizer for ExternalAuth {
    type Principal = TestPrincipal;

    fn authorize_request(
        &self,
        _principal: &Self::Principal,
        request: &ImageRequest,
    ) -> Result<(), AuthDenied> {
        self.calls.lock().unwrap().push("request".to_owned());
        if request.identity.owner == self.owner {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    fn authorize_owner(
        &self,
        _principal: &Self::Principal,
        owner: &ImageOwner,
        _action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        self.calls.lock().unwrap().push("owner".to_owned());
        if owner == &self.owner {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }

    fn authorize_stored(
        &self,
        _principal: &Self::Principal,
        _action: ServiceAction,
        _view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        self.calls.lock().unwrap().push("stored".to_owned());
        if stored.owner == &self.owner {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
}

#[derive(Clone)]
struct ExternalClock {
    now: Arc<AtomicI64>,
}

impl ServiceClock for ExternalClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct ExternalIds {
    jobs: Arc<AtomicU64>,
    interests: Arc<AtomicU64>,
}

impl ExternalIds {
    fn counter(counter: &AtomicU64) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[8..].copy_from_slice(&counter.fetch_add(1, Ordering::SeqCst).to_be_bytes());
        bytes
    }
}

impl ServiceIds for ExternalIds {
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        ImageJobId::from_bytes(&Self::counter(&self.jobs))
            .map_err(|_| ServiceIdError::InvalidGeneratedId)
    }

    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        ImageInterestId::from_bytes(&Self::counter(&self.interests))
            .map_err(|_| ServiceIdError::InvalidGeneratedId)
    }
}

#[derive(Clone)]
struct ZeroIds;

impl ServiceIds for ZeroIds {
    fn next_job_id(&self) -> Result<ImageJobId, ServiceIdError> {
        Err(ServiceIdError::InvalidGeneratedId)
    }

    fn next_interest_id(&self) -> Result<ImageInterestId, ServiceIdError> {
        Err(ServiceIdError::InvalidGeneratedId)
    }
}

struct Inner {
    receipts: HashMap<(Vec<u8>, OperationId), df_imagery::OperationReceipt>,
    jobs: HashMap<ImageJobId, ImageJob>,
    interests: HashMap<ImageInterestId, ImageInterest>,
    calls: Vec<String>,
    fail_accept: Option<JournalError>,
}

#[derive(Clone)]
struct ExternalJournal {
    inner: Arc<Mutex<Inner>>,
}

impl ExternalJournal {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                receipts: HashMap::new(),
                jobs: HashMap::new(),
                interests: HashMap::new(),
                calls: Vec::new(),
                fail_accept: None,
            })),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.inner.lock().unwrap().calls.clone()
    }

    fn fail_accept_once(&self, error: JournalError) {
        self.inner.lock().unwrap().fail_accept = Some(error);
    }
}

fn scope_bytes(owner: &ImageOwner) -> Vec<u8> {
    df_imagery::owner_scope_bytes(owner)
}

impl ImageJournal for ExternalJournal {
    async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push("accept".to_owned());
        if let Some(error) = inner.fail_accept.take() {
            return Err(error);
        }
        let key = (
            scope_bytes(&command.identity.owner),
            command.identity.operation,
        );
        if let Some(receipt) = inner.receipts.get(&key) {
            let job = inner
                .jobs
                .get(&receipt.job)
                .cloned()
                .ok_or(JournalError::InvalidRecord)?;
            if receipt.fingerprint == command.identity.fingerprint {
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
                let receipt = df_imagery::OperationReceipt {
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
            AcceptTarget::Join { .. } | AcceptTarget::JoinReady { .. } => {
                Err(JournalError::NotFound)
            }
        }
    }

    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push("lookup".to_owned());
        let key = (scope_bytes(owner), operation);
        let Some(receipt) = inner.receipts.get(&key).cloned() else {
            return Ok(None);
        };
        let job = inner
            .jobs
            .get(&receipt.job)
            .cloned()
            .ok_or(JournalError::InvalidRecord)?;
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
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .jobs
            .get(&job)
            .cloned()
            .filter(|stored| &stored.owner == owner))
    }

    async fn freeze_recipe(&self, _command: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        Err(JournalError::Unavailable)
    }

    async fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<df_imagery::WithdrawOutcome, JournalError> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push("withdraw".to_owned());
        let current = inner
            .interests
            .get(&interest)
            .cloned()
            .ok_or(JournalError::NotFound)?;
        if scope_bytes(&current.owner) != scope_bytes(owner) {
            return Err(JournalError::NotFound);
        }
        let mut job = inner
            .jobs
            .get(&current.job)
            .cloned()
            .ok_or(JournalError::InvalidRecord)?;
        if current.state == InterestState::Withdrawn {
            let attached = inner
                .interests
                .values()
                .filter(|candidate| {
                    candidate.job == job.job && candidate.state == InterestState::Attached
                })
                .count() as u64;
            return Ok(df_imagery::WithdrawOutcome::AlreadyWithdrawn(
                df_imagery::Withdrawal {
                    interest: current,
                    attached_remaining: attached,
                    job_revision: job.revision,
                },
            ));
        }
        let mut updated = current;
        updated.state = InterestState::Withdrawn;
        updated.withdrawn_at_ms = Some(now_ms);
        inner.interests.insert(interest, updated.clone());
        let attached = inner
            .interests
            .values()
            .filter(|candidate| {
                candidate.job == job.job && candidate.state == InterestState::Attached
            })
            .count() as u64;
        if attached == 0 {
            job.state = df_imagery::JobState::Retired;
        }
        job.revision += 1;
        job.updated_at_ms = now_ms;
        inner.jobs.insert(job.job, job.clone());
        Ok(df_imagery::WithdrawOutcome::Withdrawn(
            df_imagery::Withdrawal {
                interest: updated,
                attached_remaining: attached,
                job_revision: job.revision,
            },
        ))
    }
}

fn handles(
    owner: ImageOwner,
) -> (
    ExternalJournal,
    ExternalClock,
    ExternalIds,
    ExternalAuth,
    TestPrincipal,
) {
    let journal = ExternalJournal::new();
    let clock = ExternalClock {
        now: Arc::new(AtomicI64::new(1_700_000_000_000)),
    };
    let ids = ExternalIds {
        jobs: Arc::new(AtomicU64::new(1)),
        interests: Arc::new(AtomicU64::new(1)),
    };
    let auth = ExternalAuth {
        owner,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    (journal, clock, ids, auth, TestPrincipal)
}

#[tokio::test]
async fn external_record_and_replay_through_public_fingerprint() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (journal, clock, ids, auth, principal) = handles(owner.clone());
    let service = ImageService::new(
        journal.clone(),
        ImageConfig::defaults().unwrap(),
        clock,
        ids,
        auth,
    )
    .unwrap();

    let mut request = base_request(owner.clone(), operation);
    let fingerprint = service.canonical_fingerprint(&request).unwrap();
    request.identity.fingerprint = fingerprint;

    let recorded: ImageDemandView = match service.request(&principal, &request).await.unwrap() {
        RequestOutcome::Recorded(view) => view,
        other => panic!("expected Recorded, got {other:?}"),
    };
    let handle = OperationHandle { owner, operation };
    assert_eq!(recorded.handle, handle);

    match service.request(&principal, &request).await.unwrap() {
        RequestOutcome::Replayed(replayed) => assert_eq!(replayed, recorded),
        other => panic!("expected Replayed, got {other:?}"),
    }

    let read = service.read(&principal, &recorded.handle).await.unwrap();
    assert_eq!(read, recorded);
    match service.cancel(&principal, &recorded.handle).await.unwrap() {
        CancelOutcome::Withdrawn(_) => {}
        CancelOutcome::AlreadyWithdrawn(_) => panic!("expected first Withdrawn"),
    }
}

#[tokio::test]
async fn external_no_write_id_failure_conflict_and_unknown() {
    let owner = draft_owner();
    let operation = operation_id(0x21);
    let (journal, clock, ids, auth, principal) = handles(owner.clone());
    let service = ImageService::new(
        journal.clone(),
        ImageConfig::defaults().unwrap(),
        clock.clone(),
        ids,
        auth.clone(),
    )
    .unwrap();

    let mut request = base_request(owner.clone(), operation);
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    match service.request(&principal, &request).await.unwrap() {
        RequestOutcome::Recorded(_) => {}
        other => panic!("expected Recorded, got {other:?}"),
    }

    let zero = ImageService::new(
        journal.clone(),
        ImageConfig::defaults().unwrap(),
        clock,
        ZeroIds,
        auth,
    )
    .unwrap();
    let mut fresh = base_request(owner.clone(), operation_id(0x22));
    fresh.identity.fingerprint = zero.canonical_fingerprint(&fresh).unwrap();
    let accepts_before = journal
        .calls()
        .iter()
        .filter(|call| *call == "accept")
        .count();
    assert_eq!(
        zero.request(&principal, &fresh).await.unwrap_err(),
        ImageServiceError::Unavailable
    );
    let calls = journal.calls();
    assert!(calls.contains(&"lookup".to_owned()));
    assert_eq!(
        calls.iter().filter(|call| *call == "accept").count(),
        accepts_before
    );

    let mut forked = request.clone();
    forked.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("a different tavern entirely", 3).unwrap(),
        references: Vec::new(),
    });
    forked.identity.fingerprint = service.canonical_fingerprint(&forked).unwrap();
    assert_eq!(
        service.request(&principal, &forked).await.unwrap_err(),
        ImageServiceError::Conflict
    );

    let mut unknown_request = base_request(owner.clone(), operation_id(0x23));
    unknown_request.identity.fingerprint = service.canonical_fingerprint(&unknown_request).unwrap();
    journal.fail_accept_once(JournalError::Unknown);
    let handle = OperationHandle {
        owner: owner.clone(),
        operation: operation_id(0x23),
    };
    match service
        .request(&principal, &unknown_request)
        .await
        .unwrap_err()
    {
        ImageServiceError::Unknown { handle: failed } => assert_eq!(failed, handle),
        other => panic!("expected Unknown, got {other:?}"),
    }
    match service.request(&principal, &unknown_request).await.unwrap() {
        RequestOutcome::Recorded(view) => assert_eq!(view.job_revision, 1),
        other => panic!("expected Recorded after Unknown recovery, got {other:?}"),
    }
}
