//! The catalog contract suite run against the in-memory fake.

use crate::InMemoryCatalog;
use crate::catalog_contract as contract;
use crate::catalog_store::{AdmitOutcome, CatalogStore, NotAdmitted};
use crate::object_store_tests::block_on;
use crate::{AssetKind, Audience, DedupeScope, Provenance, QuotaScope, RetentionClass};

fn draft_account(byte: u8) -> df_types::AccountId {
    df_types::AccountId::from_bytes(&[byte; 16]).unwrap()
}

fn draft_admission(asset: u8, attempt: u8, key: u8) -> crate::Admission {
    draft_admission_as(asset, attempt, key, 7)
}

fn draft_admission_as(asset: u8, attempt: u8, key: u8, account: u8) -> crate::Admission {
    let owner = draft_account(account);
    crate::Admission {
        asset: contract::asset_id(asset),
        attempt: contract::attempt_id(attempt),
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope: QuotaScope::Account(owner),
        origin_account: Some(owner),
        campaign: None,
        audience: Audience::OwnerPrivate(owner),
        dedupe_scope: DedupeScope::account(&owner),
        request_key: Some(contract::request_key(key)),
        retention: RetentionClass::Personal,
        reserve_bytes: contract::RESERVE_BYTES,
        byte_limit: contract::LIMIT_BYTES,
        in_flight_upload_limit: None,
    }
}

fn account_usage(catalog: &InMemoryCatalog, account: u8) -> crate::QuotaUsage {
    block_on(catalog.quota_usage(QuotaScope::Account(draft_account(account)))).unwrap()
}

fn draft_make_ready(catalog: &InMemoryCatalog, n: u8, now_ms: i64) {
    use crate::object_store::AssetVariant;
    assert_eq!(
        block_on(catalog.admit(&draft_admission(n, n, n), now_ms)).unwrap(),
        AdmitOutcome::Admitted
    );
    assert_eq!(
        block_on(catalog.record_staged(
            contract::asset_id(n),
            contract::attempt_id(n),
            &contract::staged_record(n, n, now_ms),
            now_ms
        ))
        .unwrap(),
        crate::catalog_store::StageOutcome::Staged
    );
    block_on(contract::validate_as(catalog, n, 1, now_ms));
    block_on(contract::write_variant(
        catalog,
        n,
        1,
        20,
        AssetVariant::Master,
        40,
        now_ms,
    ));
    block_on(contract::write_variant(
        catalog,
        n,
        1,
        21,
        AssetVariant::Display,
        20,
        now_ms,
    ));
    let request =
        contract::finalize_request(&[(AssetVariant::Master, 20), (AssetVariant::Display, 21)]);
    assert_eq!(
        block_on(catalog.finalize(contract::held(n, n, 1), &request, now_ms)).unwrap(),
        crate::catalog_store::FinalizeResult::Ready {
            committed_bytes: 60
        }
    );
}

macro_rules! fake_contract {
    ($($scenario:ident),+ $(,)?) => {
        $(
            #[test]
            fn $scenario() {
                block_on(contract::$scenario(&InMemoryCatalog::new()));
            }
        )+
    };
}

fake_contract!(
    admission_reserves_once_and_refuses_a_second_live_row_for_the_key,
    admission_beyond_the_limit_is_refused_with_the_room_left_and_clears_when_space_frees,
    uploads_are_limited_by_their_in_flight_count,
    a_step_claim_is_exclusive_until_its_lease_expires_and_each_claim_has_a_new_owner,
    staging_is_recorded_once_and_only_for_the_current_attempt,
    finalize_commits_the_quota_and_reports_ready_exactly_once,
    finalize_names_only_written_winners_and_a_rewrite_under_a_new_write_succeeds,
    a_worker_whose_lease_was_replaced_can_neither_write_nor_finalize,
    an_expired_lease_cannot_start_a_write_even_before_anyone_replaces_it,
    rejection_releases_the_reservation_and_reports_once_and_frees_the_key,
    a_replaced_attempt_refuses_the_old_attempts_late_work_and_reports_superseded,
    the_orphan_sweep_removes_an_older_claims_writes_and_never_a_winner,
    a_write_a_sweep_marked_deleting_makes_finalize_answer_winner_missing,
    a_write_finalize_named_as_a_winner_is_no_longer_deletable,
    a_tombstone_frees_the_bytes_and_a_purge_claim_leaves_the_winners_to_the_purge,
    due_rows_are_those_without_a_live_lease_oldest_first,
    a_quota_full_finalize_rejects_an_upload_instead_of_overcommitting,
    generated_output_commits_even_beyond_its_reservation_and_the_scope_limit,
    a_late_finalize_after_a_takeover_finalized_reports_lease_lost_with_one_outbox_row,
    a_rejection_guard_never_ends_newer_work,
    a_staged_object_outside_quarantine_is_refused,
    a_reservation_too_large_to_add_is_refused_and_never_wraps,
);

#[test]
fn a_valid_account_draft_admits_and_reserves_account_quota() {
    let catalog = InMemoryCatalog::new();
    assert_eq!(
        block_on(catalog.admit(&draft_admission(1, 1, 1), 10)).unwrap(),
        AdmitOutcome::Admitted
    );
    let row = block_on(catalog.asset(contract::asset_id(1)))
        .unwrap()
        .unwrap();
    assert_eq!(row.campaign, None);
    assert_eq!(row.origin_account, Some(draft_account(7)));
    assert_eq!(
        account_usage(&catalog, 7).reserved_bytes,
        contract::RESERVE_BYTES
    );
    assert_eq!(
        block_on(catalog.quota_usage(contract::scope()))
            .unwrap()
            .reserved_bytes,
        0
    );
}

#[test]
fn each_mismatched_draft_field_is_refused_before_any_side_effect() {
    type DraftMutation = fn(&mut crate::Admission);
    let cases: Vec<(&str, DraftMutation)> = vec![
        ("charge", |admission| {
            admission.charge_scope = contract::scope()
        }),
        ("origin", |admission| {
            admission.origin_account = Some(draft_account(8))
        }),
        ("origin_missing", |admission| {
            admission.origin_account = None
        }),
        ("audience", |admission| {
            admission.audience = Audience::OwnerPrivate(draft_account(8))
        }),
        ("dedupe", |admission| {
            admission.dedupe_scope = DedupeScope::account(&draft_account(8))
        }),
        ("retention", |admission| {
            admission.retention = RetentionClass::CampaignMedia
        }),
        ("kind", |admission| admission.kind = AssetKind::Video),
        ("key_missing", |admission| admission.request_key = None),
        ("provenance", |admission| {
            admission.provenance = Provenance::Uploaded
        }),
    ];
    for (name, mutate) in cases {
        let catalog = InMemoryCatalog::new();
        let mut admission = draft_admission(1, 1, 1);
        mutate(&mut admission);
        if name == "provenance" {
            assert_eq!(
                block_on(catalog.admit(&admission, 10)).unwrap(),
                AdmitOutcome::Admitted,
                "{name} keeps the legacy admittable path"
            );
            continue;
        }
        assert_eq!(
            block_on(catalog.admit(&admission, 10)).unwrap(),
            AdmitOutcome::NotAdmitted(NotAdmitted::OwnerUnsupported),
            "{name}"
        );
        assert_eq!(catalog.row_count(), 0, "{name} inserts nothing");
        assert_eq!(
            account_usage(&catalog, 7).reserved_bytes,
            0,
            "{name} reserves nothing"
        );
        assert_eq!(
            account_usage(&catalog, 8).reserved_bytes,
            0,
            "{name} reserves nothing"
        );
    }
}

#[test]
fn a_no_campaign_non_image_generation_stays_refused() {
    let catalog = InMemoryCatalog::new();
    let mut admission = draft_admission(1, 1, 1);
    admission.kind = crate::AssetKind::Audio;
    assert_eq!(
        block_on(catalog.admit(&admission, 10)).unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::OwnerUnsupported)
    );
    assert_eq!(catalog.row_count(), 0);
}

#[test]
fn same_account_same_key_dedupes_with_one_reservation() {
    let catalog = InMemoryCatalog::new();
    draft_make_ready(&catalog, 1, 10);
    let mut second = draft_admission_as(2, 2, 1, 7);
    second.asset = contract::asset_id(2);
    assert_eq!(
        block_on(catalog.admit(&second, 11)).unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::Ready {
            asset: contract::asset_id(1)
        })
    );
    assert_eq!(account_usage(&catalog, 7).reserved_bytes, 0);
    assert_eq!(account_usage(&catalog, 7).committed_bytes, 60);
    assert_eq!(catalog.row_count(), 1);
    let live = InMemoryCatalog::new();
    assert_eq!(
        block_on(live.admit(&draft_admission(1, 1, 1), 10)).unwrap(),
        AdmitOutcome::Admitted
    );
    assert_eq!(
        block_on(live.admit(&draft_admission_as(2, 2, 1, 7), 11)).unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::InFlight {
            attempt: contract::attempt_id(1)
        })
    );
    assert_eq!(
        account_usage(&live, 7).reserved_bytes,
        contract::RESERVE_BYTES
    );
}

#[test]
fn a_forced_identical_account_campaign_id_never_leaks_the_other_owners_row() {
    let catalog = InMemoryCatalog::new();
    assert_eq!(
        block_on(catalog.admit(&contract::admission(1), 10)).unwrap(),
        AdmitOutcome::Admitted
    );
    // An actual uniqueness collision: the same 16 raw bytes as the campaign id, so the
    // (dedupe_scope, request_key) pair collides with the live campaign row while the
    // owner kind differs (account draft vs campaign). The previous fixture used an
    // all-200 account whose scope never matched the campaign scope, so nothing collided.
    let raw = *contract::campaign().as_bytes();
    let owner = df_types::AccountId::from_bytes(&raw).unwrap();
    let mut draft = draft_admission_as(2, 2, 1, 7);
    draft.charge_scope = QuotaScope::Account(owner);
    draft.origin_account = Some(owner);
    draft.audience = Audience::OwnerPrivate(owner);
    draft.dedupe_scope = DedupeScope::account(&owner);
    draft.request_key = Some(contract::request_key(1));
    assert_eq!(
        block_on(catalog.admit(&draft, 11)).unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::OwnerUnsupported)
    );
    assert_eq!(
        catalog.row_count(),
        1,
        "the losing admission inserts nothing"
    );
    assert_eq!(account_usage(&catalog, 7).reserved_bytes, 0);
}

#[test]
fn a_draft_rewrite_after_a_sweep_marked_deleting_stays_under_the_draft_owner() {
    use crate::catalog_store::{DeletionMark, FinalizeResult};
    use crate::object_store::AssetVariant;
    let catalog = InMemoryCatalog::new();
    assert_eq!(
        block_on(catalog.admit(&draft_admission(1, 1, 1), 0)).unwrap(),
        AdmitOutcome::Admitted
    );
    assert_eq!(
        block_on(catalog.record_staged(
            contract::asset_id(1),
            contract::attempt_id(1),
            &contract::staged_record(1, 1, 0),
            0
        ))
        .unwrap(),
        crate::catalog_store::StageOutcome::Staged
    );
    block_on(contract::validate_as(&catalog, 1, 1, 0));
    block_on(contract::write_variant(
        &catalog,
        1,
        1,
        20,
        AssetVariant::Master,
        40,
        0,
    ));
    assert_eq!(
        block_on(catalog.claim_step(
            contract::claim(1, 1, 2, crate::AssetState::Validated, 5_000),
            contract::LEASE_MS + 1
        ))
        .unwrap(),
        crate::catalog_store::ClaimOutcome::Claimed
    );
    let orphans = block_on(catalog.orphan_variants(i64::MAX, 10)).unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(
        block_on(catalog.begin_variant_deletion(&orphans[0])).unwrap(),
        DeletionMark::Marked
    );
    let stale = contract::finalize_request(&[(AssetVariant::Master, 20)]);
    assert_eq!(
        block_on(catalog.finalize(contract::held(1, 1, 2), &stale, contract::LEASE_MS + 2))
            .unwrap(),
        FinalizeResult::WinnerMissing {
            variant: AssetVariant::Master
        }
    );
    block_on(contract::write_variant(
        &catalog,
        1,
        2,
        22,
        AssetVariant::Master,
        40,
        12,
    ));
    block_on(contract::write_variant(
        &catalog,
        1,
        2,
        23,
        AssetVariant::Display,
        20,
        12,
    ));
    let fresh =
        contract::finalize_request(&[(AssetVariant::Master, 22), (AssetVariant::Display, 23)]);
    assert_eq!(
        block_on(catalog.finalize(contract::held(1, 1, 2), &fresh, 12)).unwrap(),
        FinalizeResult::Ready {
            committed_bytes: 60
        }
    );
    let row = block_on(catalog.asset(contract::asset_id(1)))
        .unwrap()
        .unwrap();
    assert_eq!(row.campaign, None);
    assert_eq!(row.origin_account, Some(draft_account(7)));
    assert_eq!(account_usage(&catalog, 7).committed_bytes, 60);
}

#[test]
fn a_draft_tombstone_frees_account_bytes_once_and_respects_live_references() {
    use crate::TombstoneReason;
    use crate::catalog_store::TombstoneResult;
    let catalog = InMemoryCatalog::new();
    draft_make_ready(&catalog, 1, 10);
    catalog.hold_live_reference(contract::asset_id(1));
    assert_eq!(
        block_on(catalog.tombstone(contract::asset_id(1), TombstoneReason::UserDelete, 20))
            .unwrap(),
        TombstoneResult::LiveReferences
    );
    assert_eq!(account_usage(&catalog, 7).committed_bytes, 60);
    let plain = InMemoryCatalog::new();
    draft_make_ready(&plain, 1, 10);
    assert_eq!(
        block_on(plain.tombstone(contract::asset_id(1), TombstoneReason::AccountDeletion, 20))
            .unwrap(),
        TombstoneResult::Deleted { released_bytes: 60 }
    );
    assert_eq!(account_usage(&plain, 7).committed_bytes, 0);
    assert_eq!(
        block_on(plain.tombstone(contract::asset_id(1), TombstoneReason::AccountDeletion, 21))
            .unwrap(),
        TombstoneResult::NotDeletable {
            state: crate::AssetState::Deleted
        }
    );
    assert_eq!(account_usage(&plain, 7).committed_bytes, 0);
}

#[test]
fn a_live_reference_refuses_the_fakes_tombstone_until_it_is_gone() {
    use crate::catalog_store::{CatalogStore, TombstoneResult};
    use crate::{TombstoneReason, catalog_contract::asset_id};
    let catalog = InMemoryCatalog::new();
    block_on(contract::make_ready(&catalog, 1, 1, 10));
    catalog.hold_live_reference(asset_id(1));
    assert_eq!(
        block_on(catalog.tombstone(asset_id(1), TombstoneReason::UserDelete, 20)).unwrap(),
        TombstoneResult::LiveReferences
    );
    assert_eq!(
        block_on(catalog.quota_usage(contract::scope()))
            .unwrap()
            .committed_bytes,
        60
    );
}

#[test]
fn a_claimed_library_draft_uses_only_its_existing_private_account_scope() {
    let catalog = InMemoryCatalog::new();
    let mut admission = draft_admission(1, 1, 1);
    admission.provenance = Provenance::ClaimedLibrary;
    assert_eq!(
        crate::admission_owner(&admission),
        Ok(Some(crate::StorageOwner::AccountDraft(draft_account(7))))
    );
    assert_eq!(
        block_on(catalog.admit(&admission, 10)).unwrap(),
        AdmitOutcome::Admitted
    );
    let record = block_on(catalog.asset(admission.asset)).unwrap().unwrap();
    assert_eq!(record.provenance, Provenance::ClaimedLibrary);
    assert_eq!(
        crate::record_owner(&record),
        Some(crate::StorageOwner::AccountDraft(draft_account(7)))
    );
    assert_eq!(
        account_usage(&catalog, 7).reserved_bytes,
        contract::RESERVE_BYTES
    );
    assert_eq!(account_usage(&catalog, 7).in_flight_uploads, 0);
    assert_eq!(
        block_on(catalog.quota_usage(contract::scope())).unwrap(),
        crate::QuotaUsage::default()
    );
    assert_eq!(
        block_on(catalog.admit(&admission, 11)).unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::InFlight {
            attempt: admission.attempt
        })
    );
    assert_eq!(catalog.row_count(), 1);
    assert_eq!(
        account_usage(&catalog, 7).reserved_bytes,
        contract::RESERVE_BYTES
    );
}

#[test]
fn claimed_library_scope_mismatches_reserve_nothing_and_cannot_recover_an_owner() {
    type Mutation = fn(&mut crate::Admission);
    let cases: &[(&str, Mutation)] = &[
        ("campaign_quota", |a| a.charge_scope = contract::scope()),
        ("other_account_quota", |a| {
            a.charge_scope = QuotaScope::Account(draft_account(8))
        }),
        ("origin_missing", |a| a.origin_account = None),
        ("other_origin", |a| {
            a.origin_account = Some(draft_account(8))
        }),
        ("public_audience", |a| a.audience = Audience::PublicStatic),
        ("campaign_audience", |a| {
            a.audience = Audience::CampaignShared(contract::campaign())
        }),
        ("host_audience", |a| {
            a.audience = Audience::Host(contract::campaign())
        }),
        ("other_private_audience", |a| {
            a.audience = Audience::OwnerPrivate(draft_account(8))
        }),
        ("other_dedupe", |a| {
            a.dedupe_scope = DedupeScope::account(&draft_account(8))
        }),
        ("no_request_key", |a| a.request_key = None),
        ("quarantine_retention", |a| {
            a.retention = RetentionClass::Quarantine
        }),
        ("campaign_retention", |a| {
            a.retention = RetentionClass::CampaignMedia
        }),
        ("audio", |a| a.kind = AssetKind::Audio),
        ("character_json", |a| a.kind = AssetKind::CharacterJson),
    ];
    let valid_catalog = InMemoryCatalog::new();
    let mut valid = draft_admission(1, 1, 1);
    valid.provenance = Provenance::ClaimedLibrary;
    assert_eq!(
        block_on(valid_catalog.admit(&valid, 10)).unwrap(),
        AdmitOutcome::Admitted
    );
    let record = block_on(valid_catalog.asset(valid.asset)).unwrap().unwrap();
    for (name, mutate) in cases {
        let catalog = InMemoryCatalog::new();
        let mut invalid = valid.clone();
        mutate(&mut invalid);
        assert_eq!(
            crate::admission_owner(&invalid),
            Err(NotAdmitted::OwnerUnsupported),
            "{name}"
        );
        assert_eq!(
            block_on(catalog.admit(&invalid, 10)).unwrap(),
            AdmitOutcome::NotAdmitted(NotAdmitted::OwnerUnsupported),
            "{name}"
        );
        assert_eq!(catalog.row_count(), 0, "{name}");
        assert_eq!(
            account_usage(&catalog, 7),
            crate::QuotaUsage::default(),
            "{name}"
        );
        assert_eq!(
            account_usage(&catalog, 8),
            crate::QuotaUsage::default(),
            "{name}"
        );
        assert_eq!(
            block_on(catalog.quota_usage(contract::scope())).unwrap(),
            crate::QuotaUsage::default(),
            "{name}"
        );
        let mut legacy = record.clone();
        legacy.kind = invalid.kind;
        legacy.charge_scope = invalid.charge_scope;
        legacy.origin_account = invalid.origin_account;
        legacy.audience = invalid.audience;
        legacy.dedupe_scope = invalid.dedupe_scope;
        legacy.request_key = invalid.request_key;
        legacy.retention = invalid.retention;
        assert_eq!(crate::record_owner(&legacy), None, "{name}");
    }
}

#[test]
fn claimed_library_withheld_audience_has_no_admission_or_recovered_owner() {
    let catalog = InMemoryCatalog::new();
    let mut admission = draft_admission(1, 1, 1);
    admission.provenance = Provenance::ClaimedLibrary;
    assert_eq!(
        block_on(catalog.admit(&admission, 10)).unwrap(),
        AdmitOutcome::Admitted
    );
    let mut record = block_on(catalog.asset(admission.asset)).unwrap().unwrap();
    admission.audience = Audience::Withheld;
    record.audience = Audience::Withheld;
    assert_eq!(
        crate::admission_owner(&admission),
        Err(NotAdmitted::AudienceEmpty)
    );
    assert_eq!(crate::record_owner(&record), None);
    let refused = InMemoryCatalog::new();
    assert_eq!(
        block_on(refused.admit(&admission, 10)).unwrap(),
        AdmitOutcome::NotAdmitted(NotAdmitted::AudienceEmpty)
    );
    assert_eq!(refused.row_count(), 0);
    assert_eq!(account_usage(&refused, 7), crate::QuotaUsage::default());
}

#[test]
fn uploaded_and_reviewed_private_rows_keep_their_legacy_unpublishable_owner() {
    for provenance in [Provenance::Uploaded, Provenance::Reviewed] {
        let catalog = InMemoryCatalog::new();
        let mut admission = draft_admission(1, 1, 1);
        admission.provenance = provenance;
        assert_eq!(crate::admission_owner(&admission), Ok(None));
        assert_eq!(
            block_on(catalog.admit(&admission, 10)).unwrap(),
            AdmitOutcome::Admitted
        );
        let record = block_on(catalog.asset(admission.asset)).unwrap().unwrap();
        assert_eq!(crate::record_owner(&record), None);
    }
}
