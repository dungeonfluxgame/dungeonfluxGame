//! Fake-backed delivery checks: owner success, exact variant selection, concealed denials,
//! mid-load tombstones, and deadline/concurrency bounds. Real PostgreSQL coverage lives in
//! `df-hosted`.

use crate::TombstoneReason;
use crate::audience::Audience;
use crate::catalog_delivery::{
    DeliveryBudget, DraftDeliveryError, DraftDeliveryLimits, DraftDeliveryRequest,
    DraftImageCandidate, DraftImageVariant, deliver_draft_image,
};
use crate::catalog_record::{
    AssetKind, DedupeScope, Provenance, RetentionClass, StagedObjectRecord, Step,
};
use crate::catalog_store::{
    Admission, AdmitOutcome, CatalogStore, ClaimOutcome, LeaseOutcome, StageOutcome, StepClaim,
};
use crate::media_ids::{AssetId, AttemptId, ClaimToken, WriteId};
use crate::object_store::{
    AssetVariant, Deadline, KeyLayout, ListPage, ObjectEnvironment, ObjectKey, ObjectPrefix,
    ObjectStore, ObjectStoreError, ObjectVersion, StorageDomain, StoredObject, VersionListing,
    VersioningStatus,
};
use crate::object_store_tests::block_on;
use crate::{
    AssetManifest, AssetState, ConditionalWrites, FakeConfig, FakeObjectStore, FinalizeRequest,
    FinalizeResult, HeldStep, InMemoryCatalog, QuotaScope, SupersedeResult, VariantIntent,
    VariantWritten, Winner,
};
use df_types::AccountId;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const NOW_MS: i64 = 1_000_000;
const MAX_BYTES: u64 = 1024 * 1024;

struct DraftFixture {
    catalog: InMemoryCatalog,
    store: FakeObjectStore,
    account: AccountId,
    asset: AssetId,
    attempt: AttemptId,
    master: Vec<u8>,
    display: Vec<u8>,
    master_write: WriteId,
    display_write: WriteId,
    master_key: ObjectKey,
    display_key: ObjectKey,
}

fn ids(tag: u8) -> (AccountId, AssetId, AttemptId, ClaimToken, WriteId, WriteId) {
    let account = AccountId::from_bytes(&[tag; 16]).unwrap();
    let asset = AssetId::from_bytes(&[tag + 10; 16]).unwrap();
    let attempt = AttemptId::from_bytes(&[tag + 20; 16]).unwrap();
    let claim = ClaimToken::from_bytes(&[tag + 30; 16]).unwrap();
    let master = WriteId::from_bytes(&[tag + 40; 16]).unwrap();
    let display = WriteId::from_bytes(&[tag + 41; 16]).unwrap();
    (account, asset, attempt, claim, master, display)
}

fn manifest_of(bytes: &[u8]) -> AssetManifest {
    AssetManifest {
        byte_len: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    }
}

fn layout() -> KeyLayout {
    KeyLayout::new(ObjectEnvironment::new("test").unwrap())
}

/// Publishes the written fixture through the catalog's actual finalization boundary.
fn ready_draft(tag: u8, mime: &str, extra_display: Option<(WriteId, Vec<u8>)>) -> DraftFixture {
    let (draft, held, winners) = written_draft(tag, extra_display);
    assert!(matches!(
        block_on(draft.catalog.finalize(
            held,
            &FinalizeRequest {
                winners,
                mime: mime.to_owned(),
                width_px: Some(2),
                height_px: Some(2),
                byte_limit: u64::MAX,
            },
            NOW_MS
        ))
        .unwrap(),
        FinalizeResult::Ready { .. }
    ));
    draft
}

/// Written variants remain unpublished so replacement can exercise a live attempt.
fn written_draft(
    tag: u8,
    extra_display: Option<(WriteId, Vec<u8>)>,
) -> (DraftFixture, HeldStep, Vec<Winner>) {
    let (account, asset, attempt, claim, master_write, display_write) = ids(tag);
    let catalog = InMemoryCatalog::new();
    let store = FakeObjectStore::new(FakeConfig::default());
    let admission = Admission {
        asset,
        attempt,
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope: QuotaScope::Account(account),
        origin_account: Some(account),
        campaign: None,
        audience: Audience::OwnerPrivate(account),
        dedupe_scope: DedupeScope::account(&account),
        request_key: Some(crate::MediaRequestKey::from_bytes([tag; 32])),
        retention: RetentionClass::Personal,
        reserve_bytes: 10_000,
        byte_limit: u64::MAX,
        in_flight_upload_limit: None,
    };
    assert_eq!(
        block_on(catalog.admit(&admission, NOW_MS)).unwrap(),
        AdmitOutcome::Admitted
    );
    let staged = StagedObjectRecord {
        key: layout().staged(
            asset,
            attempt,
            WriteId::from_bytes(&[tag + 50; 16]).unwrap(),
        ),
        version: ObjectVersion::new("s1".to_owned()).unwrap(),
        manifest: manifest_of(&[9; 4]),
        staged_at_ms: NOW_MS,
    };
    assert_eq!(
        block_on(catalog.record_staged(asset, attempt, &staged, NOW_MS)).unwrap(),
        StageOutcome::Staged
    );
    let held = HeldStep {
        asset,
        attempt,
        owner: claim,
    };
    assert_eq!(
        block_on(catalog.claim_step(
            StepClaim {
                asset,
                attempt,
                step: Step::Validate,
                expected_state: AssetState::Quarantined,
                owner: claim,
                lease_until_ms: NOW_MS + 60_000,
            },
            NOW_MS
        ))
        .unwrap(),
        ClaimOutcome::Claimed
    );
    assert_eq!(
        block_on(catalog.mark_validated(held, NOW_MS)).unwrap(),
        LeaseOutcome::Done
    );
    let master = vec![7; 512];
    let display = vec![8; 256];
    let master_key =
        layout().account_draft_asset(&account, asset, attempt, master_write, AssetVariant::Master);
    write_variant(
        &catalog,
        &store,
        held,
        AssetVariant::Master,
        &master,
        &master_key,
        master_write,
    );
    let display_key = layout().account_draft_asset(
        &account,
        asset,
        attempt,
        display_write,
        AssetVariant::Display,
    );
    write_variant(
        &catalog,
        &store,
        held,
        AssetVariant::Display,
        &display,
        &display_key,
        display_write,
    );
    let mut winners = vec![
        Winner {
            variant: AssetVariant::Master,
            write: master_write,
        },
        Winner {
            variant: AssetVariant::Display,
            write: display_write,
        },
    ];
    if let Some((extra_write, extra_bytes)) = extra_display {
        let extra_key = layout().account_draft_asset(
            &account,
            asset,
            attempt,
            extra_write,
            AssetVariant::Display,
        );
        write_variant(
            &catalog,
            &store,
            held,
            AssetVariant::Display,
            &extra_bytes,
            &extra_key,
            extra_write,
        );
        winners.push(Winner {
            variant: AssetVariant::Display,
            write: extra_write,
        });
    }
    (
        DraftFixture {
            catalog,
            store,
            account,
            asset,
            attempt,
            master,
            display,
            master_write,
            display_write,
            master_key,
            display_key,
        },
        held,
        winners,
    )
}

fn write_variant(
    catalog: &InMemoryCatalog,
    store: &FakeObjectStore,
    held: HeldStep,
    variant: AssetVariant,
    bytes: &[u8],
    key: &ObjectKey,
    write: WriteId,
) {
    assert_eq!(
        block_on(catalog.intend_variant(
            held,
            &VariantIntent {
                variant,
                write,
                key: key.clone(),
            },
            NOW_MS
        ))
        .unwrap(),
        LeaseOutcome::Done
    );
    let stored = block_on(store.put_new(
        key,
        bytes,
        manifest_of(bytes),
        Deadline::after(Duration::from_secs(5)),
    ))
    .unwrap();
    assert!(
        block_on(catalog.mark_variant_written(
            held.asset,
            held.attempt,
            &VariantWritten {
                variant,
                write,
                version: stored.version,
                manifest: manifest_of(bytes),
            },
        ))
        .unwrap()
    );
}

fn ask<'a>(
    owner: &'a AccountId,
    asset: AssetId,
    variant: DraftImageVariant,
    max_bytes: u64,
) -> (DeliveryBudget, DraftDeliveryRequest<'a>) {
    (
        DeliveryBudget::new(4),
        DraftDeliveryRequest {
            owner,
            asset,
            variant,
            limits: DraftDeliveryLimits {
                max_image_bytes: max_bytes,
            },
            deadline: Deadline::after(Duration::from_secs(30)),
        },
    )
}

fn deliver(
    draft: &DraftFixture,
    owner: &AccountId,
    variant: DraftImageVariant,
) -> Result<DraftImageCandidate, DraftDeliveryError> {
    let (budget, request) = ask(owner, draft.asset, variant, MAX_BYTES);
    block_on(deliver_draft_image(
        &draft.catalog,
        &draft.store,
        &budget,
        &request,
    ))
}

#[test]
fn owner_display_bytes_are_exact_and_carry_the_basis() {
    let draft = ready_draft(1, "image/png", None);
    let candidate = deliver(&draft, &draft.account, DraftImageVariant::Display).unwrap();
    assert_eq!(candidate.mime(), "image/png");
    assert_eq!(candidate.bytes(), draft.display.as_slice());
    assert_eq!(candidate.byte_len(), draft.display.len() as u64);
    let basis = candidate.basis();
    assert_eq!(basis.asset(), draft.asset);
    assert_eq!(basis.owner(), draft.account);
    assert_eq!(basis.attempt(), draft.attempt);
    assert_eq!(basis.write(), draft.display_write);
    assert_eq!(basis.variant(), AssetVariant::Display);
    assert_eq!(basis.digest(), &manifest_of(&draft.display).sha256);
    assert_eq!(basis.byte_len(), draft.display.len() as u64);
    assert_eq!(basis.suppression_generation(), 0);
    let rows = block_on(draft.catalog.variants(draft.asset)).unwrap();
    let row = rows
        .iter()
        .find(|row| row.write == draft.display_write)
        .unwrap();
    assert_eq!(basis.version(), row.version.as_ref().unwrap().as_str());
}

#[test]
fn master_selection_is_exact() {
    let draft = ready_draft(2, "image/png", None);
    let candidate = deliver(&draft, &draft.account, DraftImageVariant::Master).unwrap();
    assert_eq!(candidate.bytes(), draft.master.as_slice());
    assert_ne!(candidate.bytes(), draft.display.as_slice());
    assert_eq!(candidate.basis().write(), draft.master_write);
}

#[test]
fn foreign_owner_is_denied() {
    let draft = ready_draft(3, "image/png", None);
    let foreign = AccountId::from_bytes(&[99; 16]).unwrap();
    assert_eq!(
        deliver(&draft, &foreign, DraftImageVariant::Display).unwrap_err(),
        DraftDeliveryError::NotFound
    );
}

#[test]
fn ambiguous_display_winners_are_denied() {
    let extra_write = WriteId::from_bytes(&[71; 16]).unwrap();
    let draft = ready_draft(4, "image/png", Some((extra_write, vec![6; 128])));
    assert_eq!(
        deliver(&draft, &draft.account, DraftImageVariant::Display).unwrap_err(),
        DraftDeliveryError::NotFound
    );
    deliver(&draft, &draft.account, DraftImageVariant::Master).unwrap();
}

#[test]
fn display_without_a_winner_is_denied() {
    let (account, asset, attempt, claim, master_write, _) = ids(5);
    let catalog = InMemoryCatalog::new();
    let store = FakeObjectStore::new(FakeConfig::default());
    let admission = Admission {
        asset,
        attempt,
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope: QuotaScope::Account(account),
        origin_account: Some(account),
        campaign: None,
        audience: Audience::OwnerPrivate(account),
        dedupe_scope: DedupeScope::account(&account),
        request_key: Some(crate::MediaRequestKey::from_bytes([5; 32])),
        retention: RetentionClass::Personal,
        reserve_bytes: 10_000,
        byte_limit: u64::MAX,
        in_flight_upload_limit: None,
    };
    assert_eq!(
        block_on(catalog.admit(&admission, NOW_MS)).unwrap(),
        AdmitOutcome::Admitted
    );
    let staged = StagedObjectRecord {
        key: layout().staged(asset, attempt, WriteId::from_bytes(&[55; 16]).unwrap()),
        version: ObjectVersion::new("s1".to_owned()).unwrap(),
        manifest: manifest_of(&[9; 4]),
        staged_at_ms: NOW_MS,
    };
    assert_eq!(
        block_on(catalog.record_staged(asset, attempt, &staged, NOW_MS)).unwrap(),
        StageOutcome::Staged
    );
    let held = HeldStep {
        asset,
        attempt,
        owner: claim,
    };
    assert_eq!(
        block_on(catalog.claim_step(
            StepClaim {
                asset,
                attempt,
                step: Step::Validate,
                expected_state: AssetState::Quarantined,
                owner: claim,
                lease_until_ms: NOW_MS + 60_000,
            },
            NOW_MS
        ))
        .unwrap(),
        ClaimOutcome::Claimed
    );
    assert_eq!(
        block_on(catalog.mark_validated(held, NOW_MS)).unwrap(),
        LeaseOutcome::Done
    );
    let bytes = vec![7; 512];
    let key =
        layout().account_draft_asset(&account, asset, attempt, master_write, AssetVariant::Master);
    write_variant(
        &catalog,
        &store,
        held,
        AssetVariant::Master,
        &bytes,
        &key,
        master_write,
    );
    assert!(matches!(
        block_on(catalog.finalize(
            held,
            &FinalizeRequest {
                winners: vec![Winner {
                    variant: AssetVariant::Master,
                    write: master_write,
                }],
                mime: "image/png".to_owned(),
                width_px: Some(2),
                height_px: Some(2),
                byte_limit: u64::MAX,
            },
            NOW_MS
        ))
        .unwrap(),
        FinalizeResult::Ready { .. }
    ));
    let draft = DraftFixture {
        catalog,
        store,
        account,
        asset,
        attempt,
        master: bytes,
        display: Vec::new(),
        master_write,
        display_write: WriteId::from_bytes(&[56; 16]).unwrap(),
        master_key: key,
        display_key: layout().account_draft_asset(
            &account,
            asset,
            attempt,
            WriteId::from_bytes(&[56; 16]).unwrap(),
            AssetVariant::Display,
        ),
    };
    assert_eq!(
        deliver(&draft, &draft.account, DraftImageVariant::Display).unwrap_err(),
        DraftDeliveryError::NotFound
    );
}

/// A store that counts version reads and optionally tombstones the asset mid-load.
struct MidLoadStore<'a> {
    inner: &'a FakeObjectStore,
    catalog: &'a InMemoryCatalog,
    asset: AssetId,
    tombstone_on_read: bool,
    reads: AtomicUsize,
}

impl ObjectStore for MidLoadStore<'_> {
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
        self.inner.put_new(key, body, expected, deadline).await
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
        self.reads.fetch_add(1, Ordering::SeqCst);
        let bytes = self.inner.get_version(object, deadline).await?;
        if self.tombstone_on_read {
            block_on(
                self.catalog
                    .tombstone(self.asset, TombstoneReason::UserDelete, NOW_MS + 1),
            )
            .unwrap();
        }
        Ok(bytes)
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

#[test]
fn oversize_advertised_length_never_reads_the_store() {
    let (account, asset, attempt, claim, master_write, display_write) = ids(6);
    let catalog = InMemoryCatalog::new();
    let store = FakeObjectStore::new(FakeConfig::default());
    let admission = Admission {
        asset,
        attempt,
        kind: AssetKind::Image,
        provenance: Provenance::Generated,
        charge_scope: QuotaScope::Account(account),
        origin_account: Some(account),
        campaign: None,
        audience: Audience::OwnerPrivate(account),
        dedupe_scope: DedupeScope::account(&account),
        request_key: Some(crate::MediaRequestKey::from_bytes([6; 32])),
        retention: RetentionClass::Personal,
        reserve_bytes: 10_000,
        byte_limit: u64::MAX,
        in_flight_upload_limit: None,
    };
    assert_eq!(
        block_on(catalog.admit(&admission, NOW_MS)).unwrap(),
        AdmitOutcome::Admitted
    );
    let staged_write = WriteId::from_bytes(&[66; 16]).unwrap();
    let staged = StagedObjectRecord {
        key: layout().staged(asset, attempt, staged_write),
        version: ObjectVersion::new("s1".to_owned()).unwrap(),
        manifest: manifest_of(&[9; 4]),
        staged_at_ms: NOW_MS,
    };
    assert_eq!(
        block_on(catalog.record_staged(asset, attempt, &staged, NOW_MS)).unwrap(),
        StageOutcome::Staged
    );
    let held = HeldStep {
        asset,
        attempt,
        owner: claim,
    };
    assert_eq!(
        block_on(catalog.claim_step(
            StepClaim {
                asset,
                attempt,
                step: Step::Validate,
                expected_state: AssetState::Quarantined,
                owner: claim,
                lease_until_ms: NOW_MS + 60_000,
            },
            NOW_MS
        ))
        .unwrap(),
        ClaimOutcome::Claimed
    );
    assert_eq!(
        block_on(catalog.mark_validated(held, NOW_MS)).unwrap(),
        LeaseOutcome::Done
    );
    let bytes = vec![7; 64];
    let master_key =
        layout().account_draft_asset(&account, asset, attempt, master_write, AssetVariant::Master);
    let display_key = layout().account_draft_asset(
        &account,
        asset,
        attempt,
        display_write,
        AssetVariant::Display,
    );
    write_variant(
        &catalog,
        &store,
        held,
        AssetVariant::Master,
        &bytes,
        &master_key,
        master_write,
    );
    assert_eq!(
        block_on(catalog.intend_variant(
            held,
            &VariantIntent {
                variant: AssetVariant::Display,
                write: display_write,
                key: display_key.clone(),
            },
            NOW_MS
        ))
        .unwrap(),
        LeaseOutcome::Done
    );
    let stored = block_on(store.put_new(
        &display_key,
        &bytes,
        manifest_of(&bytes),
        Deadline::after(Duration::from_secs(5)),
    ))
    .unwrap();
    let lying = AssetManifest {
        byte_len: MAX_BYTES + 1,
        sha256: manifest_of(&bytes).sha256,
    };
    assert!(
        block_on(catalog.mark_variant_written(
            asset,
            attempt,
            &VariantWritten {
                variant: AssetVariant::Display,
                write: display_write,
                version: stored.version,
                manifest: lying,
            },
        ))
        .unwrap()
    );
    assert!(matches!(
        block_on(catalog.finalize(
            held,
            &FinalizeRequest {
                winners: vec![
                    Winner {
                        variant: AssetVariant::Master,
                        write: master_write,
                    },
                    Winner {
                        variant: AssetVariant::Display,
                        write: display_write,
                    },
                ],
                mime: "image/png".to_owned(),
                width_px: Some(2),
                height_px: Some(2),
                byte_limit: u64::MAX,
            },
            NOW_MS
        ))
        .unwrap(),
        FinalizeResult::Ready { .. }
    ));
    let watching = MidLoadStore {
        inner: &store,
        catalog: &catalog,
        asset,
        tombstone_on_read: false,
        reads: AtomicUsize::new(0),
    };
    let (budget, request) = ask(&account, asset, DraftImageVariant::Display, MAX_BYTES);
    assert_eq!(
        block_on(deliver_draft_image(&catalog, &watching, &budget, &request)).unwrap_err(),
        DraftDeliveryError::TooLarge
    );
    assert_eq!(watching.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn corrupted_and_removed_bytes_are_denied() {
    let draft = ready_draft(7, "image/png", None);
    draft.store.corrupt_behind(&draft.display_key);
    assert_eq!(
        deliver(&draft, &draft.account, DraftImageVariant::Display).unwrap_err(),
        DraftDeliveryError::IntegrityMismatch
    );
    draft.store.remove_behind(&draft.master_key);
    assert_eq!(
        deliver(&draft, &draft.account, DraftImageVariant::Master).unwrap_err(),
        DraftDeliveryError::NotFound
    );
}

#[test]
fn tombstone_during_load_discards_the_buffer() {
    let draft = ready_draft(8, "image/png", None);
    let watching = MidLoadStore {
        inner: &draft.store,
        catalog: &draft.catalog,
        asset: draft.asset,
        tombstone_on_read: true,
        reads: AtomicUsize::new(0),
    };
    let (budget, request) = ask(
        &draft.account,
        draft.asset,
        DraftImageVariant::Display,
        MAX_BYTES,
    );
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &watching,
            &budget,
            &request
        ))
        .unwrap_err(),
        DraftDeliveryError::NotFound
    );
    assert_eq!(watching.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        block_on(draft.catalog.asset(draft.asset))
            .unwrap()
            .unwrap()
            .state,
        AssetState::Deleted
    );
}

#[test]
fn superseded_attempt_rows_are_not_disclosed() {
    let (draft, held, winners) = written_draft(9, None);
    assert_eq!(
        block_on(draft.catalog.asset(draft.asset))
            .unwrap()
            .unwrap()
            .state,
        AssetState::Validated
    );
    let next = AttemptId::from_bytes(&[91; 16]).unwrap();
    assert_eq!(
        block_on(
            draft
                .catalog
                .supersede(draft.asset, draft.attempt, next, NOW_MS)
        )
        .unwrap(),
        SupersedeResult::Superseded
    );
    let current = block_on(draft.catalog.asset(draft.asset)).unwrap().unwrap();
    assert_eq!(current.attempt, next);
    assert_eq!(current.state, AssetState::Uploading);
    let old_variants = block_on(draft.catalog.variants(draft.asset)).unwrap();
    assert_eq!(old_variants.len(), 2);
    assert!(old_variants.iter().all(|variant| {
        variant.attempt == draft.attempt && variant.state == crate::VariantState::Written
    }));
    assert_eq!(
        block_on(draft.catalog.finalize(
            held,
            &FinalizeRequest {
                winners,
                mime: "image/png".to_owned(),
                width_px: Some(2),
                height_px: Some(2),
                byte_limit: u64::MAX,
            },
            NOW_MS
        ))
        .unwrap(),
        FinalizeResult::Superseded
    );
    let watching = MidLoadStore {
        inner: &draft.store,
        catalog: &draft.catalog,
        asset: draft.asset,
        tombstone_on_read: false,
        reads: AtomicUsize::new(0),
    };
    let (budget, request) = ask(
        &draft.account,
        draft.asset,
        DraftImageVariant::Display,
        MAX_BYTES,
    );
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &watching,
            &budget,
            &request
        ))
        .unwrap_err(),
        DraftDeliveryError::NotFound
    );
    assert_eq!(watching.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn unsupported_mime_is_denied() {
    let draft = ready_draft(10, "image/gif", None);
    let watching = MidLoadStore {
        inner: &draft.store,
        catalog: &draft.catalog,
        asset: draft.asset,
        tombstone_on_read: false,
        reads: AtomicUsize::new(0),
    };
    let (budget, request) = ask(
        &draft.account,
        draft.asset,
        DraftImageVariant::Display,
        MAX_BYTES,
    );
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &watching,
            &budget,
            &request
        ))
        .unwrap_err(),
        DraftDeliveryError::UnsupportedMedia
    );
    let foreign = AccountId::from_bytes(&[99; 16]).unwrap();
    let (budget, request) = ask(&foreign, draft.asset, DraftImageVariant::Display, MAX_BYTES);
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &watching,
            &budget,
            &request
        ))
        .unwrap_err(),
        DraftDeliveryError::NotFound
    );
    assert_eq!(watching.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn deadline_and_concurrency_bound_the_work() {
    let draft = ready_draft(11, "image/png", None);
    let watching = MidLoadStore {
        inner: &draft.store,
        catalog: &draft.catalog,
        asset: draft.asset,
        tombstone_on_read: false,
        reads: AtomicUsize::new(0),
    };
    let budget = DeliveryBudget::new(4);
    let past = DraftDeliveryRequest {
        owner: &draft.account,
        asset: draft.asset,
        variant: DraftImageVariant::Display,
        limits: DraftDeliveryLimits {
            max_image_bytes: MAX_BYTES,
        },
        deadline: Deadline::after(Duration::from_millis(1)),
    };
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &watching,
            &budget,
            &past
        ))
        .unwrap_err(),
        DraftDeliveryError::DeadlineExceeded
    );
    assert_eq!(watching.reads.load(Ordering::SeqCst), 0);
    let exhausted = DeliveryBudget::new(0);
    let (_, request) = ask(
        &draft.account,
        draft.asset,
        DraftImageVariant::Display,
        MAX_BYTES,
    );
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &watching,
            &exhausted,
            &request
        ))
        .unwrap_err(),
        DraftDeliveryError::Busy
    );
    assert_eq!(watching.reads.load(Ordering::SeqCst), 0);
    let held_budget = DeliveryBudget::new(1);
    let _permit = held_budget.acquire();
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &watching,
            &held_budget,
            &request
        ))
        .unwrap_err(),
        DraftDeliveryError::Busy
    );
    assert_eq!(watching.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn variant_selector_parses_only_its_closed_forms() {
    assert_eq!(
        DraftImageVariant::parse("display"),
        Some(DraftImageVariant::Display)
    );
    assert_eq!(
        DraftImageVariant::parse("master"),
        Some(DraftImageVariant::Master)
    );
    assert_eq!(DraftImageVariant::parse("original"), None);
    assert_eq!(DraftImageVariant::parse("DISPLAY"), None);
    assert_eq!(DraftImageVariant::parse(""), None);
}

#[test]
fn selected_basis_is_distinct_from_the_published_aggregate() {
    let draft = ready_draft(12, "image/png", None);
    assert_ne!(draft.master, draft.display);
    let display = deliver(&draft, &draft.account, DraftImageVariant::Display).unwrap();
    let master = deliver(&draft, &draft.account, DraftImageVariant::Master).unwrap();
    assert_eq!(display.bytes(), draft.display.as_slice());
    assert_eq!(master.bytes(), draft.master.as_slice());
    let record = block_on(draft.catalog.asset(draft.asset)).unwrap().unwrap();
    let published = record.published.as_ref().unwrap();
    assert_eq!(
        published.byte_len,
        draft.master.len() as u64 + draft.display.len() as u64
    );
    for candidate in [&display, &master] {
        let basis = candidate.basis();
        assert_eq!(basis.mime(), "image/png");
        assert_eq!(basis.published_byte_len(), published.byte_len);
        assert_eq!(basis.published_digest(), &published.sha256);
        assert!(basis.byte_len() < basis.published_byte_len());
    }
    assert_eq!(display.basis().byte_len(), draft.display.len() as u64);
    assert_eq!(
        display.basis().digest(),
        &manifest_of(&draft.display).sha256
    );
    assert_eq!(master.basis().byte_len(), draft.master.len() as u64);
    assert_eq!(master.basis().digest(), &manifest_of(&draft.master).sha256);
}

#[test]
fn unpolled_body_holds_its_permit_until_drop() {
    let draft = ready_draft(13, "image/png", None);
    let budget = DeliveryBudget::new(1);
    let (_, probe) = ask(
        &draft.account,
        draft.asset,
        DraftImageVariant::Display,
        MAX_BYTES,
    );
    let candidate = block_on(deliver_draft_image(
        &draft.catalog,
        &draft.store,
        &budget,
        &probe,
    ))
    .unwrap();
    let body = candidate.into_body();
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &draft.store,
            &budget,
            &probe
        ))
        .unwrap_err(),
        DraftDeliveryError::Busy
    );
    drop(body);
    let candidate = block_on(deliver_draft_image(
        &draft.catalog,
        &draft.store,
        &budget,
        &probe,
    ))
    .unwrap();
    assert_eq!(candidate.bytes(), draft.display.as_slice());
}

#[test]
fn retained_chunk_holds_its_permit_until_drop() {
    use http_body::Body as _;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
    let draft = ready_draft(14, "image/png", None);
    let budget = DeliveryBudget::new(1);
    let (_, probe) = ask(
        &draft.account,
        draft.asset,
        DraftImageVariant::Display,
        MAX_BYTES,
    );
    let candidate = block_on(deliver_draft_image(
        &draft.catalog,
        &draft.store,
        &budget,
        &probe,
    ))
    .unwrap();
    let mut body = candidate.into_body();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut pinned = pin!(&mut body);
    let chunk = match pinned.as_mut().poll_frame(&mut context) {
        Poll::Ready(Some(Ok(frame))) => frame.into_data().unwrap(),
        _ => panic!("one bounded chunk"),
    };
    assert_eq!(chunk.as_ref(), draft.display.as_slice());
    drop(body);
    assert_eq!(
        block_on(deliver_draft_image(
            &draft.catalog,
            &draft.store,
            &budget,
            &probe
        ))
        .unwrap_err(),
        DraftDeliveryError::Busy
    );
    drop(chunk);
    let candidate = block_on(deliver_draft_image(
        &draft.catalog,
        &draft.store,
        &budget,
        &probe,
    ))
    .unwrap();
    assert_eq!(candidate.bytes(), draft.display.as_slice());
}
