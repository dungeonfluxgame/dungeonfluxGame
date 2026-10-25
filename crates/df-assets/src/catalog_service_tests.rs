//! The staging, finalize and sweep paths of the catalog service over the fakes (F, O1's dispatcher,
//! the validator, the object store and the in-memory catalog), on a manual clock.

use crate::asset_lifecycle::{AssetState, FinalizeOutcome, RejectReason};
use crate::audience::{Audience, MemberSet};
use crate::catalog_contract::{self as contract, scope};
use crate::catalog_record::{AssetKind, DedupeScope, RetentionClass, Step};
use crate::catalog_store::{CatalogStore, ClaimOutcome, NotAdmitted, StepClaim};
use crate::object_store::{
    Deadline, KeyLayout, ListPage, ObjectEnvironment, ObjectKey, ObjectPrefix, ObjectStore,
    ObjectStoreError, ObjectVersion, StorageDomain, StoredObject, VersionListing, VersioningStatus,
};
use crate::object_store_tests::block_on;
use crate::{
    Advance, AssetId, AssetManifest, CatalogConfig, CatalogPorts, CatalogService, ChunkedBytes,
    Clock, ConditionalWrites, FakeConfig, FakeDispatcher, FakeFinance, FakeObjectStore,
    FakeValidator, Fault, GenerationAdmission, GenerationRequest, InMemoryCatalog, ManualClock,
    ProviderAttemptState, RefetchAnswer, SequentialIds, StagingOutcome, StagingRefusal,
};
use df_types::MemberId;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;

type AfterPut = Arc<dyn Fn(&ObjectKey) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Delegates to a fake store and runs a hook after each successful `put_new`, so a test can make
/// something else happen between two of a worker's steps (controlled interleaving, no sleeps).
struct HookedStore {
    inner: Arc<FakeObjectStore>,
    after_put: Mutex<Option<AfterPut>>,
}

impl HookedStore {
    fn hook(&self, hook: AfterPut) {
        *self.after_put.lock().unwrap() = Some(hook);
    }
}

impl ObjectStore for HookedStore {
    fn conditional_writes(&self) -> ConditionalWrites {
        self.inner.conditional_writes()
    }

    async fn put_new(
        &self,
        key: &ObjectKey,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        let stored = self.inner.put_new(key, body, expected, deadline).await?;
        let hook = self.after_put.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(key).await;
        }
        Ok(stored)
    }

    async fn put_if_match(
        &self,
        key: &ObjectKey,
        current: &ObjectVersion,
        body: &[u8],
        expected: AssetManifest,
        deadline: Deadline,
    ) -> Result<StoredObject, ObjectStoreError> {
        self.inner
            .put_if_match(key, current, body, expected, deadline)
            .await
    }

    async fn head(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> Result<Option<StoredObject>, ObjectStoreError> {
        self.inner.head(key, deadline).await
    }

    async fn get_version(
        &self,
        object: &StoredObject,
        deadline: Deadline,
    ) -> Result<Vec<u8>, ObjectStoreError> {
        self.inner.get_version(object, deadline).await
    }

    async fn list_versions(
        &self,
        key: &ObjectKey,
        deadline: Deadline,
    ) -> Result<Vec<ObjectVersion>, VersionListing> {
        self.inner.list_versions(key, deadline).await
    }

    async fn delete_version(
        &self,
        key: &ObjectKey,
        version: &ObjectVersion,
        deadline: Deadline,
    ) -> Result<(), ObjectStoreError> {
        self.inner.delete_version(key, version, deadline).await
    }

    async fn list_page(
        &self,
        prefix: &ObjectPrefix,
        after: Option<&str>,
        limit: usize,
        deadline: Deadline,
    ) -> Result<ListPage, ObjectStoreError> {
        self.inner.list_page(prefix, after, limit, deadline).await
    }

    async fn versioning_status(
        &self,
        domain: StorageDomain,
        deadline: Deadline,
    ) -> Result<VersioningStatus, ObjectStoreError> {
        self.inner.versioning_status(domain, deadline).await
    }
}

type Service =
    CatalogService<InMemoryCatalog, HookedStore, FakeFinance, FakeDispatcher, FakeValidator>;

struct World {
    catalog: Arc<InMemoryCatalog>,
    store: Arc<FakeObjectStore>,
    hooked: Arc<HookedStore>,
    finance: Arc<FakeFinance>,
    dispatcher: Arc<FakeDispatcher>,
    validator: Arc<FakeValidator>,
    clock: Arc<ManualClock>,
    service: Service,
    config: CatalogConfig,
}

fn world_with(config: CatalogConfig, validator: FakeValidator) -> World {
    let catalog = Arc::new(InMemoryCatalog::new());
    let store = Arc::new(FakeObjectStore::new(FakeConfig::default()));
    let hooked = Arc::new(HookedStore {
        inner: Arc::clone(&store),
        after_put: Mutex::new(None),
    });
    let finance = Arc::new(FakeFinance::default());
    let dispatcher = Arc::new(FakeDispatcher::answering(RefetchAnswer::Unavailable));
    let validator = Arc::new(validator);
    let clock = Arc::new(ManualClock::new(1_000_000));
    let service = CatalogService::new(
        CatalogPorts {
            catalog: Arc::clone(&catalog),
            objects: Arc::clone(&hooked),
            finance: Arc::clone(&finance),
            dispatcher: Arc::clone(&dispatcher),
            validator: Arc::clone(&validator),
            clock: clock.clone(),
            ids: Arc::new(SequentialIds::new()),
        },
        KeyLayout::new(ObjectEnvironment::new("test").unwrap()),
        config,
    );
    World {
        catalog,
        store,
        hooked,
        finance,
        dispatcher,
        validator,
        clock,
        service,
        config,
    }
}

fn world() -> World {
    world_with(CatalogConfig::default(), FakeValidator::accepting())
}

fn png_fixture() -> Vec<u8> {
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    encoded.into_inner()
}

fn admit_uploaded_portrait(world: &World, attempt_number: u8) -> AssetId {
    let campaign = contract::campaign();
    let account = df_types::AccountId::from_bytes(&[8; 16]).unwrap();
    let attempt = contract::attempt_id(attempt_number);
    let asset = AssetId::from_bytes(&[attempt_number + 40; 16]).unwrap();
    let admission = crate::Admission {
        asset,
        attempt,
        kind: AssetKind::Image,
        provenance: crate::Provenance::Uploaded,
        charge_scope: scope(),
        origin_account: Some(account),
        campaign: Some(campaign),
        audience: Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&[7; 16]).unwrap()]).unwrap(),
        ),
        dedupe_scope: DedupeScope::campaign(&campaign),
        request_key: None,
        retention: RetentionClass::CampaignMedia,
        reserve_bytes: world.config.maximum_bytes.image,
        byte_limit: world.config.quota.byte_limit(scope()),
        in_flight_upload_limit: Some(1),
    };
    assert_eq!(
        block_on(world.catalog.admit(&admission, world.clock.now_ms())).unwrap(),
        crate::AdmitOutcome::Admitted
    );
    asset
}

fn upload_ticket(
    world: &World,
    asset: AssetId,
    attempt_number: u8,
    bytes: &[u8],
) -> (
    crate::UploadTicket,
    df_types::AccountId,
    df_types::CampaignId,
) {
    let attempt = contract::attempt_id(attempt_number);
    let campaign = contract::campaign();
    let account = df_types::AccountId::from_bytes(&[8; 16]).unwrap();
    let key = world.service.layout.staged(
        asset,
        attempt,
        crate::WriteId::from_bytes(&[attempt_number; 16]).unwrap(),
    );
    let now_ms = world.clock.now_ms();
    (
        crate::UploadTicket {
            upload: crate::UploadId::from_bytes(&[attempt_number + 20; 16]).unwrap(),
            asset,
            attempt,
            account,
            campaign,
            purpose: crate::UploadPurpose::Portrait,
            object_key: key,
            declared_bytes: bytes.len() as u64,
            expected_sha256: Sha256::digest(bytes).into(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + 60_000,
        },
        account,
        campaign,
    )
}

fn request(attempt: u8) -> GenerationRequest {
    let campaign = contract::campaign();
    GenerationRequest {
        attempt: contract::attempt_id(attempt),
        kind: AssetKind::Image,
        charge_scope: scope(),
        origin_account: None,
        campaign: Some(campaign),
        audience: Audience::Members(
            campaign,
            MemberSet::new(vec![MemberId::from_bytes(&[7; 16]).unwrap()]).unwrap(),
        ),
        dedupe_scope: DedupeScope::campaign(&campaign),
        request_key: contract::request_key(attempt),
        retention: RetentionClass::CampaignMedia,
    }
}

impl World {
    fn admit(&self, attempt: u8) -> AssetId {
        match block_on(self.service.admit_generation(&request(attempt))).unwrap() {
            GenerationAdmission::Admitted { asset } => asset,
            other => panic!("expected admission, got {other:?}"),
        }
    }

    fn stage(&self, asset: AssetId, attempt: u8, bytes: usize) -> StagingOutcome {
        let mut stream = ChunkedBytes::new(vec![7; bytes], 300);
        block_on(
            self.service
                .stage_output(asset, contract::attempt_id(attempt), &mut stream),
        )
        .unwrap()
    }

    fn admit_and_stage(&self, attempt: u8) -> AssetId {
        let asset = self.admit(attempt);
        assert!(matches!(
            self.stage(asset, attempt, 1_000),
            StagingOutcome::Staged { bytes: 1_000 }
        ));
        asset
    }

    fn advance(&self, asset: AssetId) -> Advance {
        block_on(self.service.advance(asset)).unwrap()
    }

    fn row(&self, asset: AssetId) -> crate::AssetRecord {
        block_on(self.catalog.asset(asset)).unwrap().unwrap()
    }

    fn outbox(&self, attempt: u8) -> Option<FinalizeOutcome> {
        block_on(self.catalog.outbox_entry(contract::attempt_id(attempt)))
            .unwrap()
            .map(|entry| entry.outcome)
    }

    fn sweep(&self) -> crate::SweepReport {
        block_on(self.service.sweep_once()).unwrap()
    }

    fn keys(&self, domain: StorageDomain) -> usize {
        self.store.key_count(domain)
    }
}

#[test]
fn an_authenticated_portrait_is_claimed_validated_and_promoted_once() {
    let world = world();
    let asset = admit_uploaded_portrait(&world, 1);
    let bytes = png_fixture();
    let (ticket, account, campaign) = upload_ticket(&world, asset, 1, &bytes);
    assert!(matches!(
        world.stage(asset, 1, 8),
        StagingOutcome::Refused(StagingRefusal::NotAccepting {
            state: AssetState::Uploading
        })
    ));
    let mut stream = ChunkedBytes::new(bytes, 128);
    assert!(matches!(
        block_on(world.service.stage_player_portrait(
            &ticket,
            account,
            campaign,
            "image/png",
            "png",
            &mut stream,
        ))
        .unwrap(),
        StagingOutcome::Promoted { .. }
    ));
    assert_eq!(world.row(asset).state, AssetState::Ready);

    let mut replay = ChunkedBytes::new(Vec::new(), 1);
    assert!(matches!(
        block_on(world.service.stage_player_portrait(
            &ticket,
            account,
            campaign,
            "image/png",
            "png",
            &mut replay,
        ))
        .unwrap(),
        StagingOutcome::Refused(StagingRefusal::UploadTicket(_))
    ));
}

#[test]
fn a_polyglot_portrait_is_quarantined_and_never_promoted() {
    let world = world();
    let asset = admit_uploaded_portrait(&world, 2);
    let mut bytes = png_fixture();
    bytes.extend_from_slice(b"<script>payload</script>");
    let (ticket, account, campaign) = upload_ticket(&world, asset, 2, &bytes);
    let mut stream = ChunkedBytes::new(bytes, 128);
    assert!(matches!(
        block_on(world.service.stage_player_portrait(
            &ticket,
            account,
            campaign,
            "image/png",
            "png",
            &mut stream,
        ))
        .unwrap(),
        StagingOutcome::Refused(StagingRefusal::ImageValidation(
            crate::ImageRejection::InvalidHeader
        ))
    ));
    assert_eq!(world.row(asset).state, AssetState::Rejected);
    assert_eq!(world.keys(StorageDomain::Quarantine), 1);
}

#[test]
fn a_wrong_type_portrait_is_quarantined_and_never_promoted() {
    let world = world();
    let asset = admit_uploaded_portrait(&world, 3);
    let bytes = png_fixture();
    let (ticket, account, campaign) = upload_ticket(&world, asset, 3, &bytes);
    let mut stream = ChunkedBytes::new(bytes, 128);
    assert!(matches!(
        block_on(world.service.stage_player_portrait(
            &ticket,
            account,
            campaign,
            "image/jpeg",
            "jpg",
            &mut stream,
        ))
        .unwrap(),
        StagingOutcome::Refused(StagingRefusal::ImageValidation(
            crate::ImageRejection::MimeMismatch
        ))
    ));
    assert_eq!(world.row(asset).state, AssetState::Rejected);
    assert_eq!(world.keys(StorageDomain::Quarantine), 1);
}

#[test]
fn an_oversize_portrait_is_refused_without_promotion() {
    let world = world();
    let asset = admit_uploaded_portrait(&world, 4);
    let bytes = vec![0; crate::MAX_UPLOAD_BYTES + 1];
    let (ticket, account, campaign) = upload_ticket(&world, asset, 4, &bytes);
    let mut stream = ChunkedBytes::new(bytes, 32 * 1024);
    assert_eq!(
        block_on(world.service.stage_player_portrait(
            &ticket,
            account,
            campaign,
            "image/png",
            "png",
            &mut stream,
        ))
        .unwrap(),
        StagingOutcome::TooLarge
    );
    assert_eq!(world.row(asset).state, AssetState::Rejected);
    assert_eq!(world.keys(StorageDomain::PrivateMedia), 0);
}

#[test]
fn a_withheld_audience_is_refused_before_the_catalog_is_touched() {
    let world = world();
    let mut withheld = request(1);
    withheld.audience = Audience::Withheld;
    assert_eq!(
        block_on(world.service.admit_generation(&withheld)).unwrap(),
        GenerationAdmission::NotAdmitted(NotAdmitted::AudienceEmpty)
    );
    assert_eq!(world.catalog.row_count(), 0);
}

#[test]
fn a_generated_picture_flows_from_admission_to_ready_with_one_settlement_report() {
    let world = world();
    let asset = world.admit(1);
    let usage = block_on(world.catalog.quota_usage(scope())).unwrap();
    assert_eq!(usage.reserved_bytes, world.config.maximum_bytes.image);
    assert!(matches!(
        world.stage(asset, 1, 1_000),
        StagingOutcome::Staged { bytes: 1_000 }
    ));
    assert_eq!(world.advance(asset), Advance::Ready);
    let row = world.row(asset);
    assert_eq!(row.state, AssetState::Ready);
    assert_eq!(row.winning_writes.len(), 2);
    assert_eq!(world.keys(StorageDomain::PrivateMedia), 2);
    assert_eq!(world.keys(StorageDomain::Quarantine), 1);
    assert_eq!(world.outbox(1), Some(FinalizeOutcome::Ready));
    let usage = block_on(world.catalog.quota_usage(scope())).unwrap();
    assert_eq!(usage.reserved_bytes, 0);
    assert_eq!(usage.committed_bytes, row.published.unwrap().byte_len);
    assert_eq!(
        world.advance(asset),
        Advance::NotWaiting {
            state: AssetState::Ready
        }
    );
}

#[test]
fn a_second_admission_of_one_request_key_is_not_admitted_and_costs_no_dispatch() {
    let world = world();
    let asset = world.admit(1);
    let again = block_on(world.service.admit_generation(&request(1))).unwrap();
    assert!(matches!(
        again,
        GenerationAdmission::NotAdmitted(NotAdmitted::InFlight { .. })
    ));
    assert_eq!(world.catalog.row_count(), 1);
    assert_eq!(world.row(asset).state, AssetState::Uploading);
}

#[test]
fn an_admission_beyond_the_campaign_quota_is_deferred_not_a_failure() {
    let mut config = CatalogConfig::default();
    config.quota.campaign_bytes = config.maximum_bytes.image + 1;
    let world = world_with(config, FakeValidator::accepting());
    world.admit(1);
    let second = block_on(world.service.admit_generation(&request(2))).unwrap();
    assert!(matches!(
        second,
        GenerationAdmission::NotAdmitted(NotAdmitted::QuotaFull(_))
    ));
    assert!(world.dispatcher.requests().is_empty());
    assert!(world.finance.cancel_calls().is_empty());
}

#[test]
fn a_staging_stream_past_the_maximum_stops_stores_nothing_and_ends_the_attempt_invalid() {
    let mut config = CatalogConfig::default();
    config.maximum_bytes.image = 100;
    let world = world_with(config, FakeValidator::accepting());
    let asset = world.admit(1);
    let mut stream = ChunkedBytes::new(vec![1; 500], 60);
    let outcome = block_on(
        world
            .service
            .stage_output(asset, contract::attempt_id(1), &mut stream),
    )
    .unwrap();
    assert_eq!(outcome, StagingOutcome::TooLarge);
    assert_eq!(
        stream.position(),
        120,
        "reading stopped at the first chunk past the limit"
    );
    assert_eq!(world.keys(StorageDomain::Quarantine), 0);
    assert_eq!(
        world.outbox(1),
        Some(FinalizeOutcome::Invalid(RejectReason::OverMaximumBytes))
    );
    assert_eq!(
        block_on(world.catalog.quota_usage(scope()))
            .unwrap()
            .reserved_bytes,
        0
    );
}

#[test]
fn a_staging_write_for_another_attempt_or_asset_or_a_second_write_is_refused() {
    let world = world();
    let asset = world.admit(1);
    assert_eq!(
        world.stage(asset, 9, 10),
        StagingOutcome::Refused(StagingRefusal::NotCurrent)
    );
    assert_eq!(
        world.stage(contract::asset_id(99), 1, 10),
        StagingOutcome::Refused(StagingRefusal::UnknownAsset)
    );
    assert!(matches!(
        world.stage(asset, 1, 10),
        StagingOutcome::Staged { .. }
    ));
    assert_eq!(
        world.stage(asset, 1, 20),
        StagingOutcome::Refused(StagingRefusal::AlreadyStaged)
    );
    assert_eq!(world.keys(StorageDomain::Quarantine), 1);
}

#[test]
fn a_broken_provider_stream_records_nothing_and_leaves_the_row_open_for_the_next_send() {
    let world = world();
    let asset = world.admit(1);
    let mut stream = ChunkedBytes::new(vec![1; 900], 300).failing_after(1);
    let outcome = block_on(
        world
            .service
            .stage_output(asset, contract::attempt_id(1), &mut stream),
    )
    .unwrap();
    assert_eq!(outcome, StagingOutcome::StreamFailed);
    assert_eq!(world.row(asset).state, AssetState::Uploading);
    assert!(matches!(
        world.stage(asset, 1, 900),
        StagingOutcome::Staged { .. }
    ));
}

#[test]
fn a_staged_object_whose_row_was_lost_to_a_supersede_is_deleted_by_the_losing_writer() {
    let world = world();
    let asset = world.admit(1);
    let catalog = Arc::clone(&world.catalog);
    world.hooked.hook(Arc::new(move |_| {
        let catalog = Arc::clone(&catalog);
        Box::pin(async move {
            catalog
                .supersede(asset, contract::attempt_id(1), contract::attempt_id(2), 0)
                .await
                .unwrap();
        })
    }));
    assert_eq!(
        world.stage(asset, 1, 100),
        StagingOutcome::Refused(StagingRefusal::NotCurrent)
    );
    assert_eq!(world.keys(StorageDomain::Quarantine), 0);
}

#[test]
fn a_lost_acknowledgement_on_the_staging_write_is_confirmed_by_head_and_not_repeated() {
    let world = world();
    let asset = world.admit(1);
    world.store.push_fault(Fault::LoseAcknowledgement);
    assert!(matches!(
        world.stage(asset, 1, 100),
        StagingOutcome::Staged { .. }
    ));
    assert_eq!(world.keys(StorageDomain::Quarantine), 1);
    assert_eq!(world.advance(asset), Advance::Ready);
}

#[test]
fn a_staged_object_altered_behind_the_catalog_is_staged_lost_once_with_no_retry() {
    let world = world();
    let asset = world.admit_and_stage(1);
    world
        .store
        .corrupt_behind(&world.row(asset).staged.unwrap().key);
    assert_eq!(
        world.advance(asset),
        Advance::Ended(FinalizeOutcome::Rejected(RejectReason::StagedLost))
    );
    assert_eq!(
        world.advance(asset),
        Advance::NotWaiting {
            state: AssetState::Rejected
        }
    );
    assert_eq!(world.keys(StorageDomain::PrivateMedia), 0);
    assert_eq!(world.validator.calls(), 0);
    assert_eq!(world.sweep().rejected, 0);
}

#[test]
fn a_staged_object_removed_behind_the_catalog_is_staged_lost() {
    let world = world();
    let asset = world.admit_and_stage(1);
    world
        .store
        .remove_behind(&world.row(asset).staged.unwrap().key);
    assert_eq!(
        world.advance(asset),
        Advance::Ended(FinalizeOutcome::Rejected(RejectReason::StagedLost))
    );
}

#[test]
fn unusable_output_is_reported_invalid_and_writes_nothing_to_private_media() {
    let world = world_with(
        CatalogConfig::default(),
        FakeValidator::rejecting(RejectReason::ValidationFailed),
    );
    let asset = world.admit_and_stage(1);
    assert_eq!(
        world.advance(asset),
        Advance::Ended(FinalizeOutcome::Invalid(RejectReason::ValidationFailed))
    );
    assert_eq!(world.keys(StorageDomain::PrivateMedia), 0);
}

#[test]
fn a_result_staged_twenty_one_hours_after_admission_validates_and_the_age_counts_from_staging() {
    let world = world();
    let asset = world.admit(1);
    let admitted_ms = world.clock.now_ms();
    world.clock.advance(21 * HOUR_MS);
    assert!(matches!(
        world.stage(asset, 1, 100),
        StagingOutcome::Staged { .. }
    ));
    let staged_at = world.row(asset).staged.unwrap().staged_at_ms;
    assert_eq!(staged_at, admitted_ms + 21 * HOUR_MS);
    assert_eq!(world.advance(asset), Advance::Ready);
}

#[test]
fn a_row_still_quarantined_at_the_maximum_age_ends_staged_expired_with_a_report() {
    let world = world();
    let asset = world.admit_and_stage(1);
    world.clock.advance(world.config.quarantine_max_age_ms - 1);
    let early = world.sweep();
    assert_eq!(
        early.advanced, 1,
        "inside the window the sweeper resumes the row"
    );
    assert_eq!(world.row(asset).state, AssetState::Ready);
    let second = world.admit_and_stage(2);
    world.clock.advance(world.config.quarantine_max_age_ms);
    let late = world.sweep();
    assert_eq!(late.rejected, 1);
    assert_eq!(
        world.outbox(2),
        Some(FinalizeOutcome::Rejected(RejectReason::StagedExpired))
    );
    assert_eq!(world.row(second).state, AssetState::Rejected);
}

#[test]
fn a_worker_that_loses_its_lease_mid_publish_stops_and_the_sweep_removes_only_its_write() {
    let world = world();
    let asset = world.admit_and_stage(1);
    let fired = Arc::new(AtomicBool::new(false));
    let (catalog, clock, lease_ms) = (
        Arc::clone(&world.catalog),
        Arc::clone(&world.clock),
        world.config.step_lease_ms,
    );
    world.hooked.hook(Arc::new(move |key: &ObjectKey| {
        let is_first_variant =
            key.domain() == StorageDomain::PrivateMedia && !fired.swap(true, Ordering::SeqCst);
        let (catalog, clock) = (Arc::clone(&catalog), Arc::clone(&clock));
        Box::pin(async move {
            if !is_first_variant {
                return;
            }
            clock.advance(lease_ms + 1);
            let takeover = StepClaim {
                asset,
                attempt: contract::attempt_id(1),
                step: Step::Publish,
                expected_state: AssetState::Validated,
                owner: contract::claim_token(99),
                lease_until_ms: clock.now_ms() + lease_ms,
            };
            let now_ms = clock.now_ms();
            assert_eq!(
                catalog.claim_step(takeover, now_ms).await.unwrap(),
                ClaimOutcome::Claimed
            );
        })
    }));
    assert_eq!(world.advance(asset), Advance::LeaseLost);
    assert_eq!(world.keys(StorageDomain::PrivateMedia), 1);
    assert_eq!(world.outbox(1), None);
    world.clock.advance(world.config.step_lease_ms + 1);
    let resumed = world.sweep();
    assert_eq!((resumed.advanced, resumed.variants_deleted), (1, 1));
    assert_eq!(world.keys(StorageDomain::PrivateMedia), 2);
    let pending = block_on(world.catalog.pending_outbox(10)).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].outcome, FinalizeOutcome::Ready);
    assert_eq!(world.row(asset).winning_writes.len(), 2);
}

#[test]
fn an_unclaimed_reservation_is_cancelled_through_f_and_rejected_not_dispatched() {
    let world = world();
    let asset = world.admit(1);
    let report = world.sweep();
    assert_eq!(report.rejected, 1);
    assert_eq!(world.finance.cancel_calls(), vec![contract::attempt_id(1)]);
    assert_eq!(world.row(asset).state, AssetState::Rejected);
    assert_eq!(
        world.outbox(1),
        Some(FinalizeOutcome::Rejected(RejectReason::NotDispatched))
    );
}

#[test]
fn when_the_dispatcher_claims_first_the_row_stays_live_and_the_output_finalizes() {
    let world = world();
    let asset = world.admit(1);
    world.finance.set_dispatcher_claims_first(true);
    let report = world.sweep();
    assert_eq!((report.rejected, report.kept_in_flight), (0, 1));
    assert_eq!(world.row(asset).state, AssetState::Uploading);
    assert!(matches!(
        world.stage(asset, 1, 100),
        StagingOutcome::Staged { .. }
    ));
    assert_eq!(world.advance(asset), Advance::Ready);
}

#[test]
fn a_possibly_sent_attempt_stays_in_flight_for_hours_and_its_late_result_finalizes() {
    let world = world();
    let asset = world.admit(1);
    for state in [
        ProviderAttemptState::DispatchPrepared,
        ProviderAttemptState::Sent,
        ProviderAttemptState::OutcomeUnknown,
        ProviderAttemptState::CustomerReleased,
    ] {
        world.finance.set(contract::attempt_id(1), state);
        let report = world.sweep();
        assert_eq!(
            (report.rejected, report.kept_in_flight),
            (0, 1),
            "{state:?}"
        );
        world.clock.advance(HOUR_MS / 2);
    }
    assert!(world.finance.cancel_calls().is_empty());
    let key = contract::request_key(1);
    assert!(matches!(
        block_on(
            world
                .service
                .by_request_key(DedupeScope::campaign(&contract::campaign()), &key)
        ),
        Ok(crate::CatalogLookup::InFlight { .. })
    ));
    world.clock.advance(HOUR_MS);
    assert!(matches!(
        world.stage(asset, 1, 100),
        StagingOutcome::Staged { .. }
    ));
    assert_eq!(world.advance(asset), Advance::Ready);
}

#[test]
fn a_failed_supplier_result_ends_the_attempt_and_f_terminal_failure_refuses_a_later_result() {
    let world = world();
    let failed = world.admit(1);
    world.finance.set(
        contract::attempt_id(1),
        ProviderAttemptState::ResultKnown {
            succeeded: false,
            at_ms: 0,
        },
    );
    let abandoned = world.admit(2);
    world.finance.set(
        contract::attempt_id(2),
        ProviderAttemptState::FailedAfterReconciliation,
    );
    assert_eq!(world.sweep().rejected, 2);
    assert_eq!(
        world.outbox(1),
        Some(FinalizeOutcome::Rejected(RejectReason::SupplierFailed))
    );
    assert_eq!(
        world.outbox(2),
        Some(FinalizeOutcome::Rejected(
            RejectReason::UnknownOutcomeFailed
        ))
    );
    assert_eq!(world.row(failed).state, AssetState::Rejected);
    assert_eq!(
        world.stage(abandoned, 2, 100),
        StagingOutcome::Refused(StagingRefusal::NotAccepting {
            state: AssetState::Rejected
        })
    );
}

#[test]
fn a_missing_output_is_refetched_with_back_off_and_only_output_gone_frees_the_key() {
    let world = world();
    let asset = world.admit(1);
    world.finance.set(
        contract::attempt_id(1),
        ProviderAttemptState::ResultKnown {
            succeeded: true,
            at_ms: world.clock.now_ms(),
        },
    );
    assert_eq!(
        world.sweep().kept_in_flight,
        1,
        "inside the staging deadline"
    );
    world.clock.advance(world.config.staging_deadline_ms);
    assert_eq!(world.sweep().refetch_requested, 1);
    assert_eq!(
        world.sweep().kept_in_flight,
        1,
        "the back-off holds the next request"
    );
    assert_eq!(world.dispatcher.requests().len(), 1);
    world.clock.advance(world.config.refetch_backoff_ms);
    assert_eq!(world.sweep().refetch_requested, 1);
    assert_eq!(world.dispatcher.requests().len(), 2);
    assert_eq!(world.row(asset).state, AssetState::Uploading);
    world.clock.advance(world.config.refetch_backoff_cap_ms);
    world.dispatcher.set_answer(RefetchAnswer::OutputGone);
    assert_eq!(world.sweep().rejected, 1);
    assert_eq!(
        world.outbox(1),
        Some(FinalizeOutcome::Rejected(RejectReason::OutputLost))
    );
}

#[test]
fn an_unreachable_finance_service_changes_nothing() {
    let world = world();
    let asset = world.admit(1);
    world.finance.set_unavailable(true);
    let report = world.sweep();
    assert_eq!((report.rejected, report.deferred), (0, 1));
    assert_eq!(world.row(asset).state, AssetState::Uploading);
}

#[test]
fn a_superseded_attempts_staged_object_is_removed_by_the_next_sweep() {
    let world = world();
    let asset = world.admit_and_stage(1);
    assert_eq!(world.keys(StorageDomain::Quarantine), 1);
    block_on(world.catalog.supersede(
        asset,
        contract::attempt_id(1),
        contract::attempt_id(2),
        world.clock.now_ms(),
    ))
    .unwrap();
    assert_eq!(world.sweep().staged_deleted, 1);
    assert_eq!(world.keys(StorageDomain::Quarantine), 0);
    assert_eq!(world.outbox(1), Some(FinalizeOutcome::Superseded));
}

#[test]
fn a_generated_admission_without_a_campaign_is_refused_with_a_typed_outcome() {
    let world = world();
    let mut personal = request(1);
    personal.campaign = None;
    personal.charge_scope =
        crate::QuotaScope::Account(df_types::AccountId::from_bytes(&[3; 16]).unwrap());
    assert_eq!(
        block_on(world.service.admit_generation(&personal)).unwrap(),
        GenerationAdmission::NotAdmitted(NotAdmitted::OwnerUnsupported)
    );
    assert_eq!(world.catalog.row_count(), 0);
}

#[test]
fn one_row_that_cannot_be_worked_is_deferred_and_the_pass_carries_on() {
    use crate::catalog_store::Admission;
    let world = world();
    let mut stuck: Admission = contract::admission(50);
    stuck.campaign = None;
    block_on(world.catalog.admit(&stuck, world.clock.now_ms())).unwrap();
    block_on(world.catalog.record_staged(
        stuck.asset,
        stuck.attempt,
        &contract::staged_record(50, 50, world.clock.now_ms()),
        world.clock.now_ms(),
    ))
    .unwrap();
    let healthy = world.admit_and_stage(1);
    let report = world.sweep();
    assert_eq!(report.examined, 2);
    assert_eq!(report.advanced, 1, "the healthy row still finishes");
    assert_eq!(report.deferred, 1, "the stuck row is counted, not fatal");
    assert_eq!(world.row(healthy).state, AssetState::Ready);
    assert_eq!(
        world.row(stuck.asset).state,
        AssetState::Quarantined,
        "nothing was claimed or written for the stuck row"
    );
    assert!(world.row(stuck.asset).lease.is_none());
}

/// Before its first chunk, which is over the limit, completes a whole valid write of the same
/// attempt: the interleaving of one complete writer and one oversize writer.
struct RacingStream<'a> {
    service: &'a Service,
    asset: AssetId,
    attempt: crate::AttemptId,
    has_raced: bool,
}

impl crate::StagingStream for RacingStream<'_> {
    async fn next_chunk(
        &mut self,
        _deadline: Deadline,
    ) -> Result<Option<Vec<u8>>, crate::StreamError> {
        if !self.has_raced {
            self.has_raced = true;
            let mut complete = ChunkedBytes::new(vec![7; 50], 50);
            let outcome = self
                .service
                .stage_output(self.asset, self.attempt, &mut complete)
                .await
                .unwrap();
            assert!(matches!(outcome, StagingOutcome::Staged { bytes: 50 }));
        }
        Ok(Some(vec![1; 500]))
    }
}

#[test]
fn an_oversize_writer_cannot_reject_a_row_a_complete_writer_just_staged() {
    let mut config = CatalogConfig::default();
    config.maximum_bytes.image = 100;
    let world = world_with(config, FakeValidator::accepting());
    let asset = world.admit(1);
    let mut oversize = RacingStream {
        service: &world.service,
        asset,
        attempt: contract::attempt_id(1),
        has_raced: false,
    };
    let outcome = block_on(world.service.stage_output(
        asset,
        contract::attempt_id(1),
        &mut oversize,
    ))
    .unwrap();
    assert_eq!(
        outcome,
        StagingOutcome::Refused(StagingRefusal::AlreadyStaged)
    );
    let row = world.row(asset);
    assert_eq!(row.state, AssetState::Quarantined);
    assert_eq!(row.staged.unwrap().manifest.byte_len, 50);
    assert_eq!(
        world.outbox(1),
        None,
        "no settlement report for a row still alive"
    );
    assert_eq!(world.advance(asset), Advance::Ready);
}

#[test]
fn a_zero_refetch_back_off_in_the_configuration_falls_back_and_still_holds_the_next_request() {
    let config = CatalogConfig {
        refetch_backoff_ms: 0,
        ..CatalogConfig::default()
    };
    let world = world_with(config, FakeValidator::accepting());
    world.admit(1);
    world.finance.set(
        contract::attempt_id(1),
        ProviderAttemptState::ResultKnown {
            succeeded: true,
            at_ms: world.clock.now_ms(),
        },
    );
    world.clock.advance(world.config.staging_deadline_ms);
    assert_eq!(world.sweep().refetch_requested, 1);
    assert_eq!(
        world.sweep().refetch_requested,
        0,
        "the back-off is not zero"
    );
    assert_eq!(world.dispatcher.requests().len(), 1);
}

#[test]
fn a_chunk_that_misses_its_deadline_times_the_transfer_out_and_records_nothing() {
    let world = world();
    let asset = world.admit(1);
    let mut stalled = ChunkedBytes::new(vec![1; 900], 300).stalling_after(1);
    let outcome = block_on(world.service.stage_output(
        asset,
        contract::attempt_id(1),
        &mut stalled,
    ))
    .unwrap();
    assert_eq!(outcome, StagingOutcome::TimedOut);
    assert_eq!(world.row(asset).state, AssetState::Uploading);
    assert_eq!(world.keys(StorageDomain::Quarantine), 0);
}

#[test]
fn a_slow_trickle_is_cut_off_by_the_total_deadline() {
    let config = CatalogConfig {
        staging_total_timeout: std::time::Duration::from_millis(20),
        ..CatalogConfig::default()
    };
    let world = world_with(config, FakeValidator::accepting());
    let asset = world.admit(1);
    let mut trickle =
        ChunkedBytes::new(vec![1; 100], 10).taking_per_chunk(std::time::Duration::from_millis(15));
    let outcome = block_on(world.service.stage_output(
        asset,
        contract::attempt_id(1),
        &mut trickle,
    ))
    .unwrap();
    assert_eq!(outcome, StagingOutcome::TimedOut);
    assert!(trickle.position() < 100, "the stream was abandoned early");
    assert_eq!(world.keys(StorageDomain::Quarantine), 0);
}

/// Pins the window until F's cancel runs inside the rejection transaction (TODO(O2-02)): F
/// cancelled, the catalog never heard, and the next pass repairs it through the cancelled state.
#[test]
fn a_crash_between_f_cancelling_and_the_catalog_rejecting_is_repaired_by_the_next_pass() {
    let world = world();
    let asset = world.admit(1);
    let cancelled = block_on(crate::FinanceAttempts::cancel_unclaimed_effect(
        &*world.finance,
        contract::attempt_id(1),
    ))
    .unwrap();
    assert_eq!(cancelled, crate::CancelOutcome::Cancelled);
    assert_eq!(world.row(asset).state, AssetState::Uploading);
    let first = world.sweep();
    assert_eq!(first.rejected, 1);
    assert_eq!(
        world.outbox(1),
        Some(FinalizeOutcome::Rejected(RejectReason::NotDispatched))
    );
    assert_eq!(
        world.finance.cancel_calls().len(),
        1,
        "F is not asked to cancel twice"
    );
    assert_eq!(world.sweep().rejected, 0);
    assert_eq!(
        block_on(world.catalog.pending_outbox(10)).unwrap().len(),
        1,
        "exactly one report"
    );
}
