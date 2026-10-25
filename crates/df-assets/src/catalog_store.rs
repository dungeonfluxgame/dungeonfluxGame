//! The consumer-owned catalog port (planning/asset-storage-research.md 6, 10): every operation is
//! one atomic catalog transaction that locks the quota row before the asset row (D25) and reports a
//! typed outcome. PostgreSQL is the authority (`df-persistence::media_catalog`); the in-memory fake
//! exists only in tests. No operation calls an object store or any other owner.

use crate::asset_lifecycle::{AssetState, FinalizeOutcome, RejectReason};
use crate::audience::Audience;
use crate::catalog_record::{
    AssetKind, AssetRecord, DedupeScope, HeldStep, OutboxEntry, Provenance, QuotaUsage,
    RetentionClass, StagedObjectRecord, Step, TombstoneReason, VariantRecord,
};
use crate::media_ids::{AssetId, AttemptId, ClaimToken, WriteId};
use crate::media_request_key::MediaRequestKey;
use crate::object_store::{AssetVariant, ObjectKey, ObjectVersion};
use crate::upload_ticket::{UploadTicket, UploadTicketClaim};
use crate::{AssetManifest, QuotaRefusal, QuotaScope};
use df_types::{AccountId, CampaignId};
use std::future::Future;

/// A catalog operation did not complete; callers branch on the variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogError {
    /// Nothing was changed: the database was unreachable or the transaction failed before commit.
    Unavailable,
    /// The commit may or may not have taken effect; re-read before any retry.
    Unknown,
    /// A stored row did not parse as a catalog value.
    InvalidRecord,
    /// The asset moved to another quota scope while the transaction waited; retry the call.
    ScopeMoved,
}

/// What a generation or upload asks the catalog to admit (6.2). The audience is already the meet of
/// the requested audience and every reference input's; `Withheld` is refused before this call.
#[derive(Clone, Debug)]
pub struct Admission {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub kind: AssetKind,
    pub provenance: Provenance,
    pub charge_scope: QuotaScope,
    pub origin_account: Option<AccountId>,
    pub campaign: Option<CampaignId>,
    pub audience: Audience,
    pub dedupe_scope: DedupeScope,
    pub request_key: Option<MediaRequestKey>,
    pub retention: RetentionClass,
    /// The worst case held against the scope until the attempt ends (10).
    pub reserve_bytes: u64,
    /// The scope's byte limit from configuration.
    pub byte_limit: u64,
    /// An upload also counts against the account's in-flight upload limit.
    pub in_flight_upload_limit: Option<u32>,
}

/// Why an admission added nothing. No reservation is held for any of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotAdmitted {
    /// The audience meet is empty: nothing nobody may see is bought.
    AudienceEmpty,
    /// A live row already holds the request key and has not finished.
    InFlight { attempt: AttemptId },
    /// A live row already holds the request key and is deliverable.
    Ready { asset: AssetId },
    /// The scope lacks room for the reservation (a storage limit, not a spend cap, D10).
    QuotaFull(QuotaRefusal),
    /// The account is at its in-flight upload limit.
    UploadLimit,
    /// Generated output is stored under campaign keys only until the upload round adds
    /// character-owned keys (TODO(O2-05)); nothing is reserved or bought for it.
    OwnerUnsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmitOutcome {
    Admitted,
    NotAdmitted(NotAdmitted),
}

/// What the hub learns for a request key (6.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogLookup {
    Ready {
        asset: AssetId,
    },
    InFlight {
        attempt: AttemptId,
    },
    /// The newest archived row: neutral chrome, restore offered, never re-bought.
    Archived {
        asset: AssetId,
    },
    /// Tombstoned until an explicit regenerate.
    Suppressed,
    Absent,
}

/// An attempt to take a step lease (6.1): compare-and-set on the row's current state, attempt and
/// an absent or expired lease.
#[derive(Clone, Copy, Debug)]
pub struct StepClaim {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub step: Step,
    pub expected_state: AssetState,
    /// Minted fresh for this claim by the claimant.
    pub owner: ClaimToken,
    pub lease_until_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimOutcome {
    Claimed,
    /// Another claim is still within its lease.
    Held {
        until_ms: i64,
    },
    /// The row is in another state or attempt, or does not exist.
    Stale,
}

/// The result of recording a staged object (6.2 output path).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageOutcome {
    Staged,
    /// A staged object is already recorded for this attempt; validated bytes are never replaced.
    AlreadyStaged,
    /// The attempt is not the asset's current attempt, or the asset does not exist.
    NotCurrent,
    /// The row is no longer accepting output (rejected, ready, ...).
    NotAccepting {
        state: AssetState,
    },
}

/// The result of an operation that must still hold its lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseOutcome {
    Done,
    /// A newer claim replaced this one, or it expired and was replaced; stop, delete nothing.
    LeaseLost,
    /// The attempt was superseded or the row ended; stop, delete nothing.
    Superseded,
}

/// The intent to write one variant object, recorded before the object exists (6.2).
#[derive(Clone, Debug)]
pub struct VariantIntent {
    pub variant: AssetVariant,
    pub write: WriteId,
    pub key: ObjectKey,
}

/// What a completed variant write stored.
#[derive(Clone, Debug)]
pub struct VariantWritten {
    pub variant: AssetVariant,
    pub write: WriteId,
    pub version: ObjectVersion,
    pub manifest: AssetManifest,
}

/// One winning write named by `Ready`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Winner {
    pub variant: AssetVariant,
    pub write: WriteId,
}

#[derive(Clone, Debug)]
pub struct FinalizeRequest {
    pub winners: Vec<Winner>,
    pub mime: String,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    /// The charge scope's byte limit, rechecked when the actual size exceeds the reservation.
    pub byte_limit: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalizeResult {
    /// The row is `Ready` and the quota moved from reserved to committed.
    Ready {
        committed_bytes: u64,
    },
    /// A named winner is not `written` (never recorded, or already being deleted): rewrite it
    /// under a new write id.
    WinnerMissing {
        variant: AssetVariant,
    },
    LeaseLost,
    Superseded,
    /// The actual size did not fit the scope: the row went terminal.
    Rejected(FinalizeOutcome),
}

/// What a rejection requires of the row at the moment it commits, checked under the row lock in
/// the same transaction, so a decision made from an older read never ends newer work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectGuard {
    /// The current attempt, whatever the state: a lease holder's own verdict (validation failed).
    Current,
    /// Only while no staging write has been recorded (a stream that ran past the maximum).
    WhileUploading,
    /// Only while no live lease holds the row (every sweeper decision).
    WhileUnleased,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectResult {
    /// The terminal transition committed; the outbox row was written.
    Rejected(FinalizeOutcome),
    /// The guard failed: the row moved on or a live lease holds it; nothing changed.
    GuardFailed { state: AssetState },
    /// The row had already ended; nothing changed.
    AlreadyEnded { state: AssetState },
    /// The row now belongs to another attempt (or does not exist); nothing changed.
    NotCurrent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupersedeResult {
    /// The row moved to the new attempt and the old attempt's `Superseded` report was written.
    Superseded,
    /// The old attempt is no longer current, or the row is past `Validated`.
    NotCurrent,
}

/// A row whose work is not owned by a live lease (6.2 sweeper input).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DueRow {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub state: AssetState,
    pub provenance: Provenance,
    pub staged: Option<StagedObjectRecord>,
    pub created_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TombstoneResult {
    /// The row is `Deleted`, access is denied from this commit and the committed bytes are freed.
    Deleted {
        released_bytes: u64,
    },
    /// A `live` reference still holds the asset; delete is refused (11).
    LiveReferences,
    /// Only `Ready` and `Archived` rows can be tombstoned.
    NotDeletable {
        state: AssetState,
    },
    NotFound,
}

/// A variant write that the orphan rule may allow to be deleted (6.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanVariant {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub write: WriteId,
    pub variant: AssetVariant,
    pub key: ObjectKey,
    pub version: Option<ObjectVersion>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeletionMark {
    /// The variant is now `deleting`: delete its object, then forget the row.
    Marked,
    /// A finalize won the race, or the row changed: leave it alone.
    NoLongerDeletable,
    Gone,
}

/// A staged object nothing needs any more.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanStaged {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub key: ObjectKey,
    pub version: ObjectVersion,
}

/// The media catalog. Time arrives as `now_ms` from the caller's injected clock.
pub trait CatalogStore: Send + Sync {
    /// Commits a one-use upload claim before its request body is accepted.
    fn claim_upload_ticket(
        &self,
        ticket: &UploadTicket,
        account: AccountId,
        campaign: CampaignId,
        purpose: crate::upload_ticket::UploadPurpose,
        now_ms: i64,
    ) -> impl Future<Output = Result<UploadTicketClaim, CatalogError>> + Send;

    /// Reserves quota and inserts the row, or says why not. Locks the quota row first.
    fn admit(
        &self,
        admission: &Admission,
        now_ms: i64,
    ) -> impl Future<Output = Result<AdmitOutcome, CatalogError>> + Send;

    fn lookup(
        &self,
        scope: DedupeScope,
        key: &MediaRequestKey,
    ) -> impl Future<Output = Result<CatalogLookup, CatalogError>> + Send;

    fn asset(
        &self,
        asset: AssetId,
    ) -> impl Future<Output = Result<Option<AssetRecord>, CatalogError>> + Send;

    fn variants(
        &self,
        asset: AssetId,
    ) -> impl Future<Output = Result<Vec<VariantRecord>, CatalogError>> + Send;

    fn quota_usage(
        &self,
        scope: QuotaScope,
    ) -> impl Future<Output = Result<QuotaUsage, CatalogError>> + Send;

    /// Compare-and-set on `(asset, attempt, state, lease absent or expired)`; the owner is a token
    /// minted fresh for this claim.
    fn claim_step(
        &self,
        claim: StepClaim,
        now_ms: i64,
    ) -> impl Future<Output = Result<ClaimOutcome, CatalogError>> + Send;

    /// Extends a held lease; `LeaseLost` when a newer claim replaced it.
    fn renew_step(
        &self,
        held: HeldStep,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> impl Future<Output = Result<LeaseOutcome, CatalogError>> + Send;

    /// Records the first complete staging write and moves `Uploading` to `Quarantined`. A staged
    /// object outside the `Quarantine` domain is a caller defect: `InvalidRecord`.
    fn record_staged(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        staged: &StagedObjectRecord,
        now_ms: i64,
    ) -> impl Future<Output = Result<StageOutcome, CatalogError>> + Send;

    /// `Quarantined` to `Validated` for the holder of the lease; the lease moves to `publish`.
    fn mark_validated(
        &self,
        held: HeldStep,
        now_ms: i64,
    ) -> impl Future<Output = Result<LeaseOutcome, CatalogError>> + Send;

    /// Records the intent to write a variant, only while the lease is live: `LeaseLost` or
    /// `Superseded` means the worker must not write.
    fn intend_variant(
        &self,
        held: HeldStep,
        intent: &VariantIntent,
        now_ms: i64,
    ) -> impl Future<Output = Result<LeaseOutcome, CatalogError>> + Send;

    /// Marks an intended variant `written`. `false` when the row is no longer `intended` (the
    /// sweep marked it `deleting`).
    fn mark_variant_written(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        written: &VariantWritten,
    ) -> impl Future<Output = Result<bool, CatalogError>> + Send;

    /// `Validated` to `Ready` in one transaction: quota row, then asset row; every winner must be
    /// `written`; compare-and-set on `(asset, attempt, state, owner)`.
    fn finalize(
        &self,
        held: HeldStep,
        request: &FinalizeRequest,
        now_ms: i64,
    ) -> impl Future<Output = Result<FinalizeResult, CatalogError>> + Send;

    /// The terminal rejection of the current attempt if `guard` holds: releases the reservation and
    /// writes the settlement outbox row. Idempotent: a second call changes nothing.
    fn reject(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        reason: RejectReason,
        guard: RejectGuard,
        now_ms: i64,
    ) -> impl Future<Output = Result<RejectResult, CatalogError>> + Send;

    /// F01 replacement: moves a live row from one attempt to another; the old attempt's late work
    /// fails its conditional writes. The quota row is locked first and the reservation transfers
    /// to the new attempt unchanged (net zero), the one deviation from design section 10.
    fn supersede(
        &self,
        asset: AssetId,
        from: AttemptId,
        to: AttemptId,
        now_ms: i64,
    ) -> impl Future<Output = Result<SupersedeResult, CatalogError>> + Send;

    /// `Ready` or `Archived` to `Deleted`: quota row first, committed bytes released, the
    /// suppression generation advanced. Refused while a `live` reference holds the asset.
    fn tombstone(
        &self,
        asset: AssetId,
        reason: TombstoneReason,
        now_ms: i64,
    ) -> impl Future<Output = Result<TombstoneResult, CatalogError>> + Send;

    /// Live rows (`Uploading`, `Quarantined`, `Validated`) with no live lease, oldest first.
    fn due_rows(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<DueRow>, CatalogError>> + Send;

    /// Variant writes the orphan rule may delete, created before `created_before_ms`.
    fn orphan_variants(
        &self,
        created_before_ms: i64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<OrphanVariant>, CatalogError>> + Send;

    /// Under the asset row's share lock, re-evaluates the orphan rule and marks the variant
    /// `deleting`.
    fn begin_variant_deletion(
        &self,
        orphan: &OrphanVariant,
    ) -> impl Future<Output = Result<DeletionMark, CatalogError>> + Send;

    /// Removes the row of a variant whose object is gone.
    fn forget_variant(
        &self,
        orphan: &OrphanVariant,
    ) -> impl Future<Output = Result<(), CatalogError>> + Send;

    fn orphan_staged(
        &self,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<OrphanStaged>, CatalogError>> + Send;

    fn forget_staged(
        &self,
        orphan: &OrphanStaged,
        now_ms: i64,
    ) -> impl Future<Output = Result<(), CatalogError>> + Send;

    /// Settlement reports F has not consumed, oldest first.
    fn pending_outbox(
        &self,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<OutboxEntry>, CatalogError>> + Send;

    fn outbox_entry(
        &self,
        attempt: AttemptId,
    ) -> impl Future<Output = Result<Option<OutboxEntry>, CatalogError>> + Send;
}
