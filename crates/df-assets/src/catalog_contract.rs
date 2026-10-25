//! The catalog contract suite (planning/asset-storage-research.md 6, 10, 18): each function takes a
//! fresh, empty [`CatalogStore`] and asserts one behaviour. The in-memory fake runs them in this
//! crate; the PostgreSQL adapter runs the same functions against a scratch database, so the two
//! can never drift apart.

use crate::asset_lifecycle::{AssetState, FinalizeOutcome, RejectReason};
use crate::audience::Audience;
use crate::catalog_record::{
    AssetKind, DedupeScope, HeldStep, Provenance, RetentionClass, StagedObjectRecord, Step,
    TombstoneReason,
};
use crate::catalog_store::{
    Admission, AdmitOutcome, CatalogError, CatalogLookup, CatalogStore, ClaimOutcome, DeletionMark,
    FinalizeRequest, FinalizeResult, LeaseOutcome, NotAdmitted, RejectGuard, RejectResult,
    StageOutcome, StepClaim, SupersedeResult, TombstoneResult, VariantIntent, VariantWritten,
    Winner,
};
use crate::media_ids::{AssetId, AttemptId, ClaimToken, WriteId};
use crate::media_request_key::MediaRequestKey;
use crate::object_store::{AssetVariant, KeyLayout, ObjectEnvironment, ObjectKey, ObjectVersion};
use crate::{AssetManifest, QuotaScope};
use df_types::{CampaignId, MemberId};

pub const LIMIT_BYTES: u64 = 1_000;
pub const RESERVE_BYTES: u64 = 100;
pub const LEASE_MS: i64 = 1_000;

fn id(byte: u8) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    bytes
}

pub fn campaign() -> CampaignId {
    CampaignId::from_bytes(&id(200)).unwrap()
}

pub fn scope() -> QuotaScope {
    QuotaScope::Campaign(campaign())
}

pub fn asset_id(n: u8) -> AssetId {
    AssetId::from_bytes(&id(n)).unwrap()
}

pub fn attempt_id(n: u8) -> AttemptId {
    AttemptId::from_bytes(&id(n)).unwrap()
}

pub fn claim_token(n: u8) -> ClaimToken {
    ClaimToken::from_bytes(&id(n)).unwrap()
}

pub fn write_id(n: u8) -> WriteId {
    WriteId::from_bytes(&id(n)).unwrap()
}

pub fn request_key(n: u8) -> MediaRequestKey {
    MediaRequestKey::from_bytes([n; 32])
}

fn layout() -> KeyLayout {
    KeyLayout::new(ObjectEnvironment::new("contract").unwrap())
}

/// A generation admission for asset `n` under request key `n`, attempt `n`.
pub fn admission(n: u8) -> Admission {
    Admission {
        asset: asset_id(n),
        attempt: attempt_id(n),
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope: scope(),
        origin_account: None,
        campaign: Some(campaign()),
        audience: Audience::Members(
            campaign(),
            crate::MemberSet::new(vec![MemberId::from_bytes(&id(7)).unwrap()]).unwrap(),
        ),
        dedupe_scope: DedupeScope::campaign(&campaign()),
        request_key: Some(request_key(n)),
        retention: RetentionClass::CampaignMedia,
        reserve_bytes: RESERVE_BYTES,
        byte_limit: LIMIT_BYTES,
        in_flight_upload_limit: None,
    }
}

pub fn manifest(byte_len: u64, tag: u8) -> AssetManifest {
    AssetManifest {
        byte_len,
        sha256: [tag; 32],
    }
}

pub fn staged_record(asset: u8, attempt: u8, staged_at_ms: i64) -> StagedObjectRecord {
    StagedObjectRecord {
        key: layout().staged(asset_id(asset), attempt_id(attempt), write_id(90)),
        version: ObjectVersion::new("v1".to_owned()).unwrap(),
        manifest: manifest(10, 9),
        staged_at_ms,
    }
}

pub fn variant_key(asset: u8, attempt: u8, write: u8, variant: AssetVariant) -> ObjectKey {
    layout().campaign_asset(
        &campaign(),
        asset_id(asset),
        attempt_id(attempt),
        write_id(write),
        variant,
    )
}

pub fn held(asset: u8, attempt: u8, owner: u8) -> HeldStep {
    HeldStep {
        asset: asset_id(asset),
        attempt: attempt_id(attempt),
        owner: claim_token(owner),
    }
}

pub fn claim(asset: u8, attempt: u8, owner: u8, state: AssetState, until_ms: i64) -> StepClaim {
    StepClaim {
        asset: asset_id(asset),
        attempt: attempt_id(attempt),
        step: if state == AssetState::Quarantined {
            Step::Validate
        } else {
            Step::Publish
        },
        expected_state: state,
        owner: claim_token(owner),
        lease_until_ms: until_ms,
    }
}

/// Admits asset `n` and records its staged object, leaving it `Quarantined`.
pub async fn admit_and_stage<C: CatalogStore>(catalog: &C, n: u8, now_ms: i64) {
    assert_eq!(
        catalog.admit(&admission(n), now_ms).await.unwrap(),
        AdmitOutcome::Admitted
    );
    assert_eq!(
        catalog
            .record_staged(
                asset_id(n),
                attempt_id(n),
                &staged_record(n, n, now_ms),
                now_ms
            )
            .await
            .unwrap(),
        StageOutcome::Staged
    );
}

/// Claims and validates asset `n` as `owner`, leaving it `Validated` with a live publish lease.
pub async fn validate_as<C: CatalogStore>(catalog: &C, n: u8, owner: u8, now_ms: i64) {
    let taken = claim(n, n, owner, AssetState::Quarantined, now_ms + LEASE_MS);
    assert_eq!(
        catalog.claim_step(taken, now_ms).await.unwrap(),
        ClaimOutcome::Claimed
    );
    assert_eq!(
        catalog
            .mark_validated(held(n, n, owner), now_ms)
            .await
            .unwrap(),
        LeaseOutcome::Done
    );
}

/// Writes one variant as `owner` under write id `write`: intent, then `written`.
pub async fn write_variant<C: CatalogStore>(
    catalog: &C,
    n: u8,
    owner: u8,
    write: u8,
    variant: AssetVariant,
    byte_len: u64,
    now_ms: i64,
) {
    let intent = VariantIntent {
        variant,
        write: write_id(write),
        key: variant_key(n, n, write, variant),
    };
    assert_eq!(
        catalog
            .intend_variant(held(n, n, owner), &intent, now_ms)
            .await
            .unwrap(),
        LeaseOutcome::Done
    );
    let written = VariantWritten {
        variant,
        write: write_id(write),
        version: ObjectVersion::new(format!("w{write}")).unwrap(),
        manifest: manifest(byte_len, write),
    };
    assert!(
        catalog
            .mark_variant_written(asset_id(n), attempt_id(n), &written)
            .await
            .unwrap()
    );
}

pub fn finalize_request(masters: &[(AssetVariant, u8)]) -> FinalizeRequest {
    FinalizeRequest {
        winners: masters
            .iter()
            .map(|(variant, write)| Winner {
                variant: *variant,
                write: write_id(*write),
            })
            .collect(),
        mime: "image/jpeg".to_owned(),
        width_px: Some(64),
        height_px: Some(48),
        byte_limit: LIMIT_BYTES,
    }
}

/// Drives asset `n` from nothing to `Ready` as `owner` (master 40 bytes, display 20 bytes).
pub async fn make_ready<C: CatalogStore>(catalog: &C, n: u8, owner: u8, now_ms: i64) {
    admit_and_stage(catalog, n, now_ms).await;
    validate_as(catalog, n, owner, now_ms).await;
    write_variant(
        catalog,
        n,
        owner,
        owner * 2,
        AssetVariant::Master,
        40,
        now_ms,
    )
    .await;
    write_variant(
        catalog,
        n,
        owner,
        owner * 2 + 1,
        AssetVariant::Display,
        20,
        now_ms,
    )
    .await;
    let request = finalize_request(&[
        (AssetVariant::Master, owner * 2),
        (AssetVariant::Display, owner * 2 + 1),
    ]);
    assert_eq!(
        catalog
            .finalize(held(n, n, owner), &request, now_ms)
            .await
            .unwrap(),
        FinalizeResult::Ready {
            committed_bytes: 60
        }
    );
}

pub async fn admission_reserves_once_and_refuses_a_second_live_row_for_the_key<C: CatalogStore>(
    catalog: &C,
) {
    assert_eq!(
        catalog.admit(&admission(1), 10).await.unwrap(),
        AdmitOutcome::Admitted
    );
    let usage = catalog.quota_usage(scope()).await.unwrap();
    assert_eq!(usage.reserved_bytes, RESERVE_BYTES);
    let mut second = admission(2);
    second.request_key = Some(request_key(1));
    assert_eq!(
        catalog.admit(&second, 11).await.unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::InFlight {
            attempt: attempt_id(1)
        })
    );
    assert_eq!(catalog.quota_usage(scope()).await.unwrap(), usage);
    assert!(catalog.asset(asset_id(2)).await.unwrap().is_none());
    assert_eq!(
        catalog
            .lookup(DedupeScope::campaign(&campaign()), &request_key(1))
            .await
            .unwrap(),
        CatalogLookup::InFlight {
            attempt: attempt_id(1)
        }
    );
}

pub async fn admission_beyond_the_limit_is_refused_with_the_room_left_and_clears_when_space_frees<
    C: CatalogStore,
>(
    catalog: &C,
) {
    let mut first = admission(1);
    first.byte_limit = 150;
    let mut second = admission(2);
    second.byte_limit = 150;
    assert_eq!(
        catalog.admit(&first, 10).await.unwrap(),
        AdmitOutcome::Admitted
    );
    let AdmitOutcome::NotAdmitted(NotAdmitted::QuotaFull(refusal)) =
        catalog.admit(&second, 11).await.unwrap()
    else {
        panic!("a second reservation beyond the limit must be refused");
    };
    assert_eq!(
        (refusal.needed_bytes, refusal.available_bytes),
        (RESERVE_BYTES, 50)
    );
    assert!(catalog.asset(asset_id(2)).await.unwrap().is_none());
    catalog
        .reject(
            asset_id(1),
            attempt_id(1),
            RejectReason::NotDispatched,
            RejectGuard::Current,
            12,
        )
        .await
        .unwrap();
    assert_eq!(
        catalog.admit(&second, 13).await.unwrap(),
        AdmitOutcome::Admitted
    );
}

pub async fn uploads_are_limited_by_their_in_flight_count<C: CatalogStore>(catalog: &C) {
    let mut first = admission(1);
    first.in_flight_upload_limit = Some(1);
    let mut second = admission(2);
    second.in_flight_upload_limit = Some(1);
    assert_eq!(
        catalog.admit(&first, 10).await.unwrap(),
        AdmitOutcome::Admitted
    );
    assert_eq!(
        catalog
            .quota_usage(scope())
            .await
            .unwrap()
            .in_flight_uploads,
        1
    );
    assert_eq!(
        catalog.admit(&second, 11).await.unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::UploadLimit)
    );
    catalog
        .reject(
            asset_id(1),
            attempt_id(1),
            RejectReason::Abandoned,
            RejectGuard::Current,
            12,
        )
        .await
        .unwrap();
    assert_eq!(
        catalog
            .quota_usage(scope())
            .await
            .unwrap()
            .in_flight_uploads,
        0
    );
}

pub async fn a_step_claim_is_exclusive_until_its_lease_expires_and_each_claim_has_a_new_owner<
    C: CatalogStore,
>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    let first = claim(1, 1, 1, AssetState::Quarantined, 1_000);
    assert_eq!(
        catalog.claim_step(first, 0).await.unwrap(),
        ClaimOutcome::Claimed
    );
    let second = claim(1, 1, 2, AssetState::Quarantined, 2_500);
    assert_eq!(
        catalog.claim_step(second, 500).await.unwrap(),
        ClaimOutcome::Held { until_ms: 1_000 }
    );
    assert_eq!(
        catalog.claim_step(second, 1_001).await.unwrap(),
        ClaimOutcome::Claimed
    );
    let wrong_state = claim(1, 1, 3, AssetState::Validated, 9_000);
    assert_eq!(
        catalog.claim_step(wrong_state, 3_000).await.unwrap(),
        ClaimOutcome::Stale
    );
    let wrong_attempt = claim(1, 9, 3, AssetState::Quarantined, 9_000);
    assert_eq!(
        catalog.claim_step(wrong_attempt, 3_000).await.unwrap(),
        ClaimOutcome::Stale
    );
    assert_eq!(
        catalog
            .renew_step(held(1, 1, 1), 5_000, 1_002)
            .await
            .unwrap(),
        LeaseOutcome::LeaseLost
    );
    assert_eq!(
        catalog.mark_validated(held(1, 1, 1), 1_002).await.unwrap(),
        LeaseOutcome::LeaseLost
    );
    assert_eq!(
        catalog
            .renew_step(held(1, 1, 2), 5_000, 1_002)
            .await
            .unwrap(),
        LeaseOutcome::Done
    );
    let record = catalog.asset(asset_id(1)).await.unwrap().unwrap();
    assert_eq!(record.lease.unwrap().owner, claim_token(2));
    assert_eq!(record.lease.unwrap().until_ms, 5_000);
}

pub async fn staging_is_recorded_once_and_only_for_the_current_attempt<C: CatalogStore>(
    catalog: &C,
) {
    assert_eq!(
        catalog.admit(&admission(1), 0).await.unwrap(),
        AdmitOutcome::Admitted
    );
    let staged = staged_record(1, 1, 5);
    assert_eq!(
        catalog
            .record_staged(asset_id(1), attempt_id(9), &staged, 5)
            .await
            .unwrap(),
        StageOutcome::NotCurrent
    );
    assert_eq!(
        catalog
            .record_staged(asset_id(77), attempt_id(1), &staged, 5)
            .await
            .unwrap(),
        StageOutcome::NotCurrent
    );
    assert_eq!(
        catalog
            .record_staged(asset_id(1), attempt_id(1), &staged, 5)
            .await
            .unwrap(),
        StageOutcome::Staged
    );
    assert_eq!(
        catalog
            .record_staged(asset_id(1), attempt_id(1), &staged_record(1, 1, 6), 6)
            .await
            .unwrap(),
        StageOutcome::AlreadyStaged
    );
    let record = catalog.asset(asset_id(1)).await.unwrap().unwrap();
    assert_eq!(record.state, AssetState::Quarantined);
    assert_eq!(record.staged, Some(staged));
}

pub async fn finalize_commits_the_quota_and_reports_ready_exactly_once<C: CatalogStore>(
    catalog: &C,
) {
    make_ready(catalog, 1, 1, 100).await;
    let usage = catalog.quota_usage(scope()).await.unwrap();
    assert_eq!((usage.reserved_bytes, usage.committed_bytes), (0, 60));
    let record = catalog.asset(asset_id(1)).await.unwrap().unwrap();
    assert_eq!(record.state, AssetState::Ready);
    assert_eq!(record.winning_writes, vec![write_id(2), write_id(3)]);
    assert_eq!(record.ready_at_ms, Some(100));
    assert_eq!(
        record.published.as_ref().map(|facts| facts.byte_len),
        Some(60)
    );
    assert_eq!(
        record.published.as_ref().map(|facts| facts.sha256),
        Some([2; 32])
    );
    let outbox = catalog.outbox_entry(attempt_id(1)).await.unwrap().unwrap();
    assert_eq!(outbox.outcome, FinalizeOutcome::Ready);
    let again = finalize_request(&[(AssetVariant::Master, 2), (AssetVariant::Display, 3)]);
    assert_eq!(
        catalog.finalize(held(1, 1, 1), &again, 101).await.unwrap(),
        FinalizeResult::Ready {
            committed_bytes: 60
        }
    );
    assert_eq!(catalog.pending_outbox(10).await.unwrap().len(), 1);
    assert_eq!(catalog.quota_usage(scope()).await.unwrap(), usage);
    assert_eq!(
        catalog
            .lookup(DedupeScope::campaign(&campaign()), &request_key(1))
            .await
            .unwrap(),
        CatalogLookup::Ready { asset: asset_id(1) }
    );
}

pub async fn finalize_names_only_written_winners_and_a_rewrite_under_a_new_write_succeeds<
    C: CatalogStore,
>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    validate_as(catalog, 1, 1, 0).await;
    let intent = VariantIntent {
        variant: AssetVariant::Master,
        write: write_id(2),
        key: variant_key(1, 1, 2, AssetVariant::Master),
    };
    catalog
        .intend_variant(held(1, 1, 1), &intent, 0)
        .await
        .unwrap();
    let request = finalize_request(&[(AssetVariant::Master, 2)]);
    assert_eq!(
        catalog.finalize(held(1, 1, 1), &request, 1).await.unwrap(),
        FinalizeResult::WinnerMissing {
            variant: AssetVariant::Master
        }
    );
    assert_eq!(
        catalog.asset(asset_id(1)).await.unwrap().unwrap().state,
        AssetState::Validated
    );
    write_variant(catalog, 1, 1, 4, AssetVariant::Master, 40, 2).await;
    let rewritten = finalize_request(&[(AssetVariant::Master, 4)]);
    assert_eq!(
        catalog
            .finalize(held(1, 1, 1), &rewritten, 3)
            .await
            .unwrap(),
        FinalizeResult::Ready {
            committed_bytes: 40
        }
    );
}

pub async fn a_worker_whose_lease_was_replaced_can_neither_write_nor_finalize<C: CatalogStore>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    validate_as(catalog, 1, 1, 0).await;
    let takeover = claim(1, 1, 2, AssetState::Validated, 5_000);
    assert_eq!(
        catalog.claim_step(takeover, LEASE_MS + 1).await.unwrap(),
        ClaimOutcome::Claimed
    );
    let intent = VariantIntent {
        variant: AssetVariant::Master,
        write: write_id(2),
        key: variant_key(1, 1, 2, AssetVariant::Master),
    };
    assert_eq!(
        catalog
            .intend_variant(held(1, 1, 1), &intent, LEASE_MS + 2)
            .await
            .unwrap(),
        LeaseOutcome::LeaseLost
    );
    assert!(catalog.variants(asset_id(1)).await.unwrap().is_empty());
    let request = finalize_request(&[(AssetVariant::Master, 2)]);
    assert_eq!(
        catalog
            .finalize(held(1, 1, 1), &request, LEASE_MS + 2)
            .await
            .unwrap(),
        FinalizeResult::LeaseLost
    );
    assert_eq!(catalog.pending_outbox(10).await.unwrap().len(), 0);
}

pub async fn an_expired_lease_cannot_start_a_write_even_before_anyone_replaces_it<
    C: CatalogStore,
>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    validate_as(catalog, 1, 1, 0).await;
    let intent = VariantIntent {
        variant: AssetVariant::Master,
        write: write_id(2),
        key: variant_key(1, 1, 2, AssetVariant::Master),
    };
    assert_eq!(
        catalog
            .intend_variant(held(1, 1, 1), &intent, LEASE_MS + 1)
            .await
            .unwrap(),
        LeaseOutcome::LeaseLost
    );
}

pub async fn rejection_releases_the_reservation_and_reports_once_and_frees_the_key<
    C: CatalogStore,
>(
    catalog: &C,
) {
    assert_eq!(
        catalog.admit(&admission(1), 0).await.unwrap(),
        AdmitOutcome::Admitted
    );
    assert_eq!(
        catalog
            .reject(
                asset_id(1),
                attempt_id(1),
                RejectReason::OverMaximumBytes,
                RejectGuard::Current,
                5
            )
            .await
            .unwrap(),
        RejectResult::Rejected(FinalizeOutcome::Invalid(RejectReason::OverMaximumBytes))
    );
    assert_eq!(
        catalog.quota_usage(scope()).await.unwrap().reserved_bytes,
        0
    );
    let record = catalog.asset(asset_id(1)).await.unwrap().unwrap();
    assert_eq!(record.state, AssetState::Rejected);
    assert_eq!(record.reject_reason, Some(RejectReason::OverMaximumBytes));
    assert_eq!(
        catalog
            .reject(
                asset_id(1),
                attempt_id(1),
                RejectReason::StagedLost,
                RejectGuard::Current,
                6
            )
            .await
            .unwrap(),
        RejectResult::AlreadyEnded {
            state: AssetState::Rejected
        }
    );
    assert_eq!(
        catalog
            .reject(
                asset_id(1),
                attempt_id(9),
                RejectReason::StagedLost,
                RejectGuard::Current,
                6
            )
            .await
            .unwrap(),
        RejectResult::NotCurrent
    );
    let entries = catalog.pending_outbox(10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].outcome,
        FinalizeOutcome::Invalid(RejectReason::OverMaximumBytes)
    );
    assert_eq!(
        catalog
            .lookup(DedupeScope::campaign(&campaign()), &request_key(1))
            .await
            .unwrap(),
        CatalogLookup::Absent
    );
    let mut again = admission(2);
    again.request_key = Some(request_key(1));
    assert_eq!(
        catalog.admit(&again, 7).await.unwrap(),
        AdmitOutcome::Admitted
    );
}

pub async fn a_replaced_attempt_refuses_the_old_attempts_late_work_and_reports_superseded<
    C: CatalogStore,
>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    assert_eq!(
        catalog
            .supersede(asset_id(1), attempt_id(1), attempt_id(2), 5)
            .await
            .unwrap(),
        SupersedeResult::Superseded
    );
    let record = catalog.asset(asset_id(1)).await.unwrap().unwrap();
    assert_eq!(
        (record.attempt, record.state),
        (attempt_id(2), AssetState::Uploading)
    );
    assert!(record.staged.is_none());
    assert_eq!(
        catalog.quota_usage(scope()).await.unwrap().reserved_bytes,
        RESERVE_BYTES
    );
    assert_eq!(
        catalog
            .outbox_entry(attempt_id(1))
            .await
            .unwrap()
            .unwrap()
            .outcome,
        FinalizeOutcome::Superseded
    );
    assert_eq!(
        catalog
            .record_staged(asset_id(1), attempt_id(1), &staged_record(1, 1, 6), 6)
            .await
            .unwrap(),
        StageOutcome::NotCurrent
    );
    let late = claim(1, 1, 3, AssetState::Quarantined, 9_000);
    assert_eq!(
        catalog.claim_step(late, 6).await.unwrap(),
        ClaimOutcome::Stale
    );
    assert_eq!(
        catalog
            .supersede(asset_id(1), attempt_id(1), attempt_id(3), 7)
            .await
            .unwrap(),
        SupersedeResult::NotCurrent
    );
    let orphans = catalog.orphan_staged(10).await.unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].attempt, attempt_id(1));
    catalog.forget_staged(&orphans[0], 8).await.unwrap();
    assert!(catalog.orphan_staged(10).await.unwrap().is_empty());
}

pub async fn the_orphan_sweep_removes_an_older_claims_writes_and_never_a_winner<C: CatalogStore>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    validate_as(catalog, 1, 1, 0).await;
    write_variant(catalog, 1, 1, 2, AssetVariant::Master, 40, 1).await;
    let takeover = claim(1, 1, 2, AssetState::Validated, 5_000);
    assert_eq!(
        catalog.claim_step(takeover, LEASE_MS + 1).await.unwrap(),
        ClaimOutcome::Claimed
    );
    write_variant(catalog, 1, 2, 4, AssetVariant::Master, 40, LEASE_MS + 2).await;
    let orphans = catalog.orphan_variants(i64::MAX, 10).await.unwrap();
    assert_eq!(
        orphans.len(),
        1,
        "only the older claim's write is an orphan"
    );
    assert_eq!(orphans[0].write, write_id(2));
    let request = finalize_request(&[(AssetVariant::Master, 4)]);
    assert_eq!(
        catalog
            .finalize(held(1, 1, 2), &request, LEASE_MS + 3)
            .await
            .unwrap(),
        FinalizeResult::Ready {
            committed_bytes: 40
        }
    );
    let orphans = catalog.orphan_variants(i64::MAX, 10).await.unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].write, write_id(2), "the winner is never listed");
    assert_eq!(
        catalog.begin_variant_deletion(&orphans[0]).await.unwrap(),
        DeletionMark::Marked
    );
    assert_eq!(
        catalog.begin_variant_deletion(&orphans[0]).await.unwrap(),
        DeletionMark::Marked,
        "marking again after a crash is allowed"
    );
    catalog.forget_variant(&orphans[0]).await.unwrap();
    assert!(
        catalog
            .orphan_variants(i64::MAX, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(catalog.variants(asset_id(1)).await.unwrap().len(), 1);
}

pub async fn a_write_a_sweep_marked_deleting_makes_finalize_answer_winner_missing<
    C: CatalogStore,
>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    validate_as(catalog, 1, 1, 0).await;
    write_variant(catalog, 1, 1, 2, AssetVariant::Master, 40, 1).await;
    let takeover = claim(1, 1, 2, AssetState::Validated, 5_000);
    catalog.claim_step(takeover, LEASE_MS + 1).await.unwrap();
    let orphans = catalog.orphan_variants(i64::MAX, 10).await.unwrap();
    assert_eq!(
        catalog.begin_variant_deletion(&orphans[0]).await.unwrap(),
        DeletionMark::Marked
    );
    let request = finalize_request(&[(AssetVariant::Master, 2)]);
    assert_eq!(
        catalog
            .finalize(held(1, 1, 2), &request, LEASE_MS + 2)
            .await
            .unwrap(),
        FinalizeResult::WinnerMissing {
            variant: AssetVariant::Master
        }
    );
}

pub async fn a_write_finalize_named_as_a_winner_is_no_longer_deletable<C: CatalogStore>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    validate_as(catalog, 1, 1, 0).await;
    write_variant(catalog, 1, 1, 2, AssetVariant::Master, 40, 1).await;
    let takeover = claim(1, 1, 2, AssetState::Validated, 5_000);
    catalog.claim_step(takeover, LEASE_MS + 1).await.unwrap();
    let orphans = catalog.orphan_variants(i64::MAX, 10).await.unwrap();
    let request = finalize_request(&[(AssetVariant::Master, 2)]);
    assert_eq!(
        catalog
            .finalize(held(1, 1, 2), &request, LEASE_MS + 2)
            .await
            .unwrap(),
        FinalizeResult::Ready {
            committed_bytes: 40
        }
    );
    assert_eq!(
        catalog.begin_variant_deletion(&orphans[0]).await.unwrap(),
        DeletionMark::NoLongerDeletable
    );
}

pub async fn a_tombstone_frees_the_bytes_and_a_purge_claim_leaves_the_winners_to_the_purge<
    C: CatalogStore,
>(
    catalog: &C,
) {
    make_ready(catalog, 1, 1, 10).await;
    assert_eq!(
        catalog
            .tombstone(asset_id(1), TombstoneReason::UserDelete, 20)
            .await
            .unwrap(),
        TombstoneResult::Deleted { released_bytes: 60 }
    );
    assert_eq!(
        catalog.quota_usage(scope()).await.unwrap().committed_bytes,
        0
    );
    assert_eq!(
        catalog
            .tombstone(asset_id(1), TombstoneReason::UserDelete, 21)
            .await
            .unwrap(),
        TombstoneResult::NotDeletable {
            state: AssetState::Deleted
        }
    );
    assert_eq!(
        catalog
            .lookup(DedupeScope::campaign(&campaign()), &request_key(1))
            .await
            .unwrap(),
        CatalogLookup::Suppressed
    );
    let purge = StepClaim {
        asset: asset_id(1),
        attempt: attempt_id(1),
        step: Step::Purge,
        expected_state: AssetState::Deleted,
        owner: claim_token(40),
        lease_until_ms: 9_000,
    };
    assert_eq!(
        catalog.claim_step(purge, 30).await.unwrap(),
        ClaimOutcome::Claimed
    );
    assert!(
        catalog
            .orphan_variants(i64::MAX, 10)
            .await
            .unwrap()
            .is_empty(),
        "a purge claim must not make the winners sweepable"
    );
    let staged = catalog.orphan_staged(10).await.unwrap();
    assert_eq!(
        staged.len(),
        1,
        "the staged object of a ready row is no longer needed"
    );
}

pub async fn due_rows_are_those_without_a_live_lease_oldest_first<C: CatalogStore>(catalog: &C) {
    admit_and_stage(catalog, 1, 10).await;
    admit_and_stage(catalog, 2, 20).await;
    let first = claim(1, 1, 1, AssetState::Quarantined, 1_000);
    catalog.claim_step(first, 30).await.unwrap();
    let due = catalog.due_rows(500, 10).await.unwrap();
    assert_eq!(
        due.iter().map(|row| row.asset).collect::<Vec<_>>(),
        vec![asset_id(2)]
    );
    let due = catalog.due_rows(1_001, 10).await.unwrap();
    assert_eq!(due.len(), 2);
    assert_eq!(due[0].asset, asset_id(2), "the older update comes first");
    assert_eq!(catalog.due_rows(1_001, 1).await.unwrap().len(), 1);
}

pub async fn a_quota_full_finalize_rejects_an_upload_instead_of_overcommitting<C: CatalogStore>(
    catalog: &C,
) {
    let mut small = admission(1);
    small.provenance = Provenance::Uploaded;
    small.reserve_bytes = 10;
    small.byte_limit = 50;
    assert_eq!(
        catalog.admit(&small, 0).await.unwrap(),
        AdmitOutcome::Admitted
    );
    catalog
        .record_staged(asset_id(1), attempt_id(1), &staged_record(1, 1, 0), 0)
        .await
        .unwrap();
    validate_as(catalog, 1, 1, 0).await;
    write_variant(catalog, 1, 1, 2, AssetVariant::Master, 40, 1).await;
    write_variant(catalog, 1, 1, 3, AssetVariant::Display, 20, 1).await;
    let mut request = finalize_request(&[(AssetVariant::Master, 2), (AssetVariant::Display, 3)]);
    request.byte_limit = 50;
    assert_eq!(
        catalog.finalize(held(1, 1, 1), &request, 2).await.unwrap(),
        FinalizeResult::Rejected(FinalizeOutcome::Rejected(RejectReason::QuotaExceeded))
    );
    let usage = catalog.quota_usage(scope()).await.unwrap();
    assert_eq!((usage.reserved_bytes, usage.committed_bytes), (0, 0));
}

pub async fn generated_output_commits_even_beyond_its_reservation_and_the_scope_limit<
    C: CatalogStore,
>(
    catalog: &C,
) {
    let mut small = admission(1);
    small.reserve_bytes = 10;
    small.byte_limit = 50;
    assert_eq!(
        catalog.admit(&small, 0).await.unwrap(),
        AdmitOutcome::Admitted
    );
    catalog
        .record_staged(asset_id(1), attempt_id(1), &staged_record(1, 1, 0), 0)
        .await
        .unwrap();
    validate_as(catalog, 1, 1, 0).await;
    write_variant(catalog, 1, 1, 2, AssetVariant::Master, 40, 1).await;
    write_variant(catalog, 1, 1, 3, AssetVariant::Display, 20, 1).await;
    let mut request = finalize_request(&[(AssetVariant::Master, 2), (AssetVariant::Display, 3)]);
    request.byte_limit = 50;
    assert_eq!(
        catalog.finalize(held(1, 1, 1), &request, 2).await.unwrap(),
        FinalizeResult::Ready {
            committed_bytes: 60
        },
        "paid output within the kind maximum is never refused for room (D21)"
    );
    let usage = catalog.quota_usage(scope()).await.unwrap();
    assert_eq!((usage.reserved_bytes, usage.committed_bytes), (0, 60));
}

pub async fn a_late_finalize_after_a_takeover_finalized_reports_lease_lost_with_one_outbox_row<
    C: CatalogStore,
>(
    catalog: &C,
) {
    admit_and_stage(catalog, 1, 0).await;
    validate_as(catalog, 1, 1, 0).await;
    write_variant(catalog, 1, 1, 2, AssetVariant::Master, 40, 1).await;
    let takeover = claim(1, 1, 2, AssetState::Validated, 5_000);
    assert_eq!(
        catalog.claim_step(takeover, LEASE_MS + 1).await.unwrap(),
        ClaimOutcome::Claimed
    );
    write_variant(catalog, 1, 2, 4, AssetVariant::Master, 40, LEASE_MS + 2).await;
    let winning = finalize_request(&[(AssetVariant::Master, 4)]);
    assert_eq!(
        catalog
            .finalize(held(1, 1, 2), &winning, LEASE_MS + 3)
            .await
            .unwrap(),
        FinalizeResult::Ready {
            committed_bytes: 40
        }
    );
    let late = finalize_request(&[(AssetVariant::Master, 2)]);
    assert_eq!(
        catalog
            .finalize(held(1, 1, 1), &late, LEASE_MS + 4)
            .await
            .unwrap(),
        FinalizeResult::LeaseLost
    );
    let intent = VariantIntent {
        variant: AssetVariant::Display,
        write: write_id(6),
        key: variant_key(1, 1, 6, AssetVariant::Display),
    };
    assert_eq!(
        catalog
            .intend_variant(held(1, 1, 1), &intent, LEASE_MS + 4)
            .await
            .unwrap(),
        LeaseOutcome::LeaseLost
    );
    assert_eq!(
        catalog
            .mark_validated(held(1, 1, 1), LEASE_MS + 4)
            .await
            .unwrap(),
        LeaseOutcome::LeaseLost
    );
    assert_eq!(catalog.pending_outbox(10).await.unwrap().len(), 1);
    assert_eq!(
        catalog.quota_usage(scope()).await.unwrap().committed_bytes,
        40
    );
}

pub async fn a_rejection_guard_never_ends_newer_work<C: CatalogStore>(catalog: &C) {
    admit_and_stage(catalog, 1, 0).await;
    assert_eq!(
        catalog
            .reject(
                asset_id(1),
                attempt_id(1),
                RejectReason::OverMaximumBytes,
                RejectGuard::WhileUploading,
                1
            )
            .await
            .unwrap(),
        RejectResult::GuardFailed {
            state: AssetState::Quarantined
        },
        "a staged row is not an oversize writer's to reject"
    );
    let taken = claim(1, 1, 1, AssetState::Quarantined, LEASE_MS);
    catalog.claim_step(taken, 0).await.unwrap();
    assert_eq!(
        catalog
            .reject(
                asset_id(1),
                attempt_id(1),
                RejectReason::StagedExpired,
                RejectGuard::WhileUnleased,
                LEASE_MS - 1
            )
            .await
            .unwrap(),
        RejectResult::GuardFailed {
            state: AssetState::Quarantined
        },
        "a live lease protects its holder's work from a sweeper decision"
    );
    assert_eq!(
        catalog.asset(asset_id(1)).await.unwrap().unwrap().state,
        AssetState::Quarantined
    );
    assert!(matches!(
        catalog
            .reject(
                asset_id(1),
                attempt_id(1),
                RejectReason::StagedExpired,
                RejectGuard::WhileUnleased,
                LEASE_MS + 1
            )
            .await
            .unwrap(),
        RejectResult::Rejected(_)
    ));
    assert_eq!(
        catalog.admit(&admission(2), 5).await.unwrap(),
        AdmitOutcome::Admitted
    );
    assert!(matches!(
        catalog
            .reject(
                asset_id(2),
                attempt_id(2),
                RejectReason::OverMaximumBytes,
                RejectGuard::WhileUploading,
                6
            )
            .await
            .unwrap(),
        RejectResult::Rejected(_)
    ));
}

pub async fn a_staged_object_outside_quarantine_is_refused<C: CatalogStore>(catalog: &C) {
    assert_eq!(
        catalog.admit(&admission(1), 0).await.unwrap(),
        AdmitOutcome::Admitted
    );
    let mut foreign = staged_record(1, 1, 0);
    foreign.key = variant_key(1, 1, 2, AssetVariant::Master);
    assert_eq!(
        catalog
            .record_staged(asset_id(1), attempt_id(1), &foreign, 0)
            .await,
        Err(CatalogError::InvalidRecord)
    );
    assert_eq!(
        catalog.asset(asset_id(1)).await.unwrap().unwrap().state,
        AssetState::Uploading
    );
}

pub async fn a_reservation_too_large_to_add_is_refused_and_never_wraps<C: CatalogStore>(
    catalog: &C,
) {
    assert_eq!(
        catalog.admit(&admission(1), 0).await.unwrap(),
        AdmitOutcome::Admitted
    );
    let mut huge = admission(2);
    huge.reserve_bytes = u64::MAX;
    huge.byte_limit = u64::MAX;
    assert!(matches!(
        catalog.admit(&huge, 1).await.unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::QuotaFull(_))
    ));
    assert!(catalog.asset(asset_id(2)).await.unwrap().is_none());
    assert_eq!(
        catalog.quota_usage(scope()).await.unwrap().reserved_bytes,
        RESERVE_BYTES
    );
}
