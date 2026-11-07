//! Bounded delivery of account-private draft images (IMG-DRAFT-DELIVERY-20261028-A1).
//!
//! Builds one buffered private candidate from the authoritative catalog and object store. The
//! caller supplies only the asset and a closed display/master selector; there is no object key,
//! URL, claimed audience, requested account or MIME override anywhere in this module. The
//! candidate carries the exact catalog basis the bytes were verified against; HTTP disclosure
//! stays in `df-portal`, and the final session-plus-catalog authorization snapshot stays in
//! `df-hosted`. This module never authorizes an account session.
//!
//! What it never does: streaming or ranges, MIME sniffing, public caching, or allocating from
//! an unbounded manifest. A `Range` request receives the full bounded 200 response upstream.

use crate::audience::Audience;
use crate::catalog_record::{AssetKind, Provenance};
use crate::catalog_store::{CatalogError, CatalogStore};
use crate::media_ids::{AssetId, AttemptId, WriteId};
use crate::object_store::{
    AssetVariant, Deadline, ObjectKey, ObjectStore, ObjectStoreError, ObjectVersion, StorageDomain,
    StoredObject,
};
use crate::{
    AssetManifest, AssetState, CatalogConfig, StorageOwner, VariantRecord, VariantState,
    record_owner,
};
use bytes::Bytes;
use df_types::AccountId;
use http_body::{Body as HttpBody, Frame, SizeHint};
use sha2::{Digest, Sha256};
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use std::time::Duration;

/// The closed image variant selector. Display is the portal default; master exists for exact
/// owner review. No other variant can be named.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DraftImageVariant {
    Display,
    Master,
}

impl DraftImageVariant {
    pub const ALL: [Self; 2] = [Self::Display, Self::Master];

    pub fn label(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Master => "master",
        }
    }

    /// The closed text form the portal query accepts; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|variant| variant.label() == text)
    }

    fn asset_variant(self) -> AssetVariant {
        match self {
            Self::Display => AssetVariant::Display,
            Self::Master => AssetVariant::Master,
        }
    }
}

/// Image media types the validated publication facts may name. Anything else is not disclosed.
pub const ALLOWED_IMAGE_MIMES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];

/// The configured byte bound for one delivered draft image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DraftDeliveryLimits {
    pub max_image_bytes: u64,
}

impl DraftDeliveryLimits {
    pub fn from_catalog(config: &CatalogConfig) -> Self {
        Self {
            max_image_bytes: config.maximum_bytes.image,
        }
    }
}

/// One delivery request: the authenticated owner, the asset, and the closed selector. The owner
/// always arrives from the account authority, never from the request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DraftDeliveryRequest<'a> {
    pub owner: &'a AccountId,
    pub asset: AssetId,
    pub variant: DraftImageVariant,
    pub limits: DraftDeliveryLimits,
    pub deadline: Deadline,
}

/// The concurrency bound for buffered draft delivery. One permit is acquired before the first
/// catalog read and held inside the candidate until the response is constructed, so the bound
/// covers the entire buffered handoff.
#[derive(Clone, Debug)]
pub struct DeliveryBudget {
    remaining: Arc<AtomicUsize>,
}

impl DeliveryBudget {
    pub fn new(max_concurrent_deliveries: usize) -> Self {
        Self {
            remaining: Arc::new(AtomicUsize::new(max_concurrent_deliveries)),
        }
    }

    pub(crate) fn acquire(&self) -> Option<DeliveryPermit> {
        let mut current = self.remaining.load(Ordering::Acquire);
        loop {
            if current == 0 {
                return None;
            }
            match self.remaining.compare_exchange_weak(
                current,
                current - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(DeliveryPermit {
                        remaining: Arc::clone(&self.remaining),
                    });
                }
                Err(actual) => current = actual,
            }
        }
    }
}

/// An owned concurrency slot. Returned to the budget when the candidate is consumed or dropped.
#[derive(Debug)]
pub struct DeliveryPermit {
    remaining: Arc<AtomicUsize>,
}

impl Drop for DeliveryPermit {
    fn drop(&mut self) {
        self.remaining.fetch_add(1, Ordering::Release);
    }
}

/// The exact catalog basis the candidate bytes were verified against. Constructed only here;
/// every disclosure check compares these values, never caller text. The selected variant
/// manifest (`digest`/`byte_len`/`version`) proves the bytes; the frozen aggregate
/// (`mime`/`published_byte_len`/`published_digest`) proves the publication facts did not move.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryBasis {
    asset: AssetId,
    owner: AccountId,
    attempt: AttemptId,
    write: WriteId,
    variant: AssetVariant,
    version: String,
    digest: [u8; 32],
    byte_len: u64,
    suppression_generation: u64,
    mime: String,
    published_byte_len: u64,
    published_digest: [u8; 32],
}

impl DeliveryBasis {
    pub fn asset(&self) -> AssetId {
        self.asset
    }

    pub fn owner(&self) -> AccountId {
        self.owner
    }

    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }

    pub fn write(&self) -> WriteId {
        self.write
    }

    pub fn variant(&self) -> AssetVariant {
        self.variant
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    pub fn suppression_generation(&self) -> u64 {
        self.suppression_generation
    }

    pub fn mime(&self) -> &str {
        &self.mime
    }

    pub fn published_byte_len(&self) -> u64 {
        self.published_byte_len
    }

    pub fn published_digest(&self) -> &[u8; 32] {
        &self.published_digest
    }

    #[cfg(any(test, feature = "test-fakes"))]
    pub fn for_tests(fields: DeliveryBasisFixture) -> Self {
        Self {
            asset: fields.asset,
            owner: fields.owner,
            attempt: fields.attempt,
            write: fields.write,
            variant: fields.variant,
            version: fields.version,
            digest: fields.digest,
            byte_len: fields.byte_len,
            suppression_generation: fields.suppression_generation,
            mime: fields.mime,
            published_byte_len: fields.published_byte_len,
            published_digest: fields.published_digest,
        }
    }
}

/// Explicit catalog and selected-variant facts for external delivery fixtures.
#[cfg(any(test, feature = "test-fakes"))]
pub struct DeliveryBasisFixture {
    pub asset: AssetId,
    pub owner: AccountId,
    pub attempt: AttemptId,
    pub write: WriteId,
    pub variant: AssetVariant,
    pub version: String,
    pub digest: [u8; 32],
    pub byte_len: u64,
    pub suppression_generation: u64,
    pub mime: String,
    pub published_byte_len: u64,
    pub published_digest: [u8; 32],
}

struct PermitBacking {
    bytes: Vec<u8>,
    _permit: DeliveryPermit,
}

impl AsRef<[u8]> for PermitBacking {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

/// One verified private draft image body. The bytes own their concurrency slot through
/// `Bytes::from_owner`: an unpolled body, a retained chunk, or a cancelled body all keep
/// the slot until the last byte alias is dropped.
#[derive(Debug)]
pub struct DraftImageBody {
    data: Option<Bytes>,
}

impl HttpBody for DraftImageBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut().data.take() {
            Some(chunk) => Poll::Ready(Some(Ok(Frame::data(chunk)))),
            None => Poll::Ready(None),
        }
    }

    fn is_end_stream(&self) -> bool {
        self.data.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        match &self.data {
            Some(chunk) => SizeHint::with_exact(u64::try_from(chunk.len()).unwrap_or(u64::MAX)),
            None => SizeHint::with_exact(0),
        }
    }
}

/// One verified private draft image, with the permit held until the body owns the bytes.
#[derive(Debug)]
pub struct DraftImageCandidate {
    basis: DeliveryBasis,
    mime: String,
    bytes: Vec<u8>,
    permit: DeliveryPermit,
}

impl DraftImageCandidate {
    pub fn basis(&self) -> &DeliveryBasis {
        &self.basis
    }

    pub fn mime(&self) -> &str {
        &self.mime
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn byte_len(&self) -> u64 {
        self.basis.byte_len
    }

    /// Moves the verified bytes into a body that retains the permit until the last
    /// byte alias is consumed or dropped, covering unpolled and cancelled responses.
    pub fn into_body(self) -> DraftImageBody {
        let backing = PermitBacking {
            bytes: self.bytes,
            _permit: self.permit,
        };
        DraftImageBody {
            data: Some(Bytes::from_owner(backing)),
        }
    }

    #[cfg(any(test, feature = "test-fakes"))]
    pub fn for_tests(basis: DeliveryBasis, mime: String, bytes: Vec<u8>) -> Self {
        Self {
            basis,
            mime,
            bytes,
            permit: DeliveryPermit {
                remaining: Arc::new(AtomicUsize::new(1)),
            },
        }
    }
}

/// Why no candidate is disclosed. `NotFound`, `TooLarge`, `UnsupportedMedia` and
/// `IntegrityMismatch` are concealed as 404 upstream; the rest are 503 authority failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DraftDeliveryError {
    /// Missing, forbidden, stale, mismatched or tombstoned: one answer for all.
    NotFound,
    /// The advertised or actual length exceeds the configured bound.
    TooLarge,
    /// The validated publication facts name an image type outside the allowed set.
    UnsupportedMedia,
    /// The stored bytes do not match their recorded manifest.
    IntegrityMismatch,
    /// The catalog or object store did not answer.
    Unavailable,
    /// The commit may or may not have taken effect; reread before any retry.
    Unknown,
    /// No concurrency slot was free before the deadline.
    Busy,
    /// The operation deadline passed before disclosure.
    DeadlineExceeded,
    /// The authority snapshot could not be read or decoded: an outage or schema fault, never a
    /// denial, so it is reported as 503 and not concealed as "no such draft".
    Snapshot(SnapshotCause),
}

/// Why the disclosure authority snapshot could not be read or decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotCause {
    /// The snapshot statement failed or the connection dropped.
    Query,
    /// A column was missing or had an unexpected type.
    Column,
    /// A stored integer was negative where an unsigned value is required.
    Range,
    /// The system clock was unreadable.
    Clock,
}

impl SnapshotCause {
    /// Every cause, in declaration order.
    pub const ALL: [Self; 4] = [Self::Query, Self::Column, Self::Range, Self::Clock];

    /// The stable lowercase name used in audit lines.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Column => "column",
            Self::Range => "range",
            Self::Clock => "clock",
        }
    }
}

impl DraftDeliveryError {
    /// Whether the refusal must conceal its reason (404) rather than report an outage (503).
    pub const fn concealed(self) -> bool {
        matches!(
            self,
            Self::NotFound | Self::TooLarge | Self::UnsupportedMedia | Self::IntegrityMismatch
        )
    }
}

fn catalog_failed(error: CatalogError) -> DraftDeliveryError {
    match error {
        CatalogError::Unavailable => DraftDeliveryError::Unavailable,
        CatalogError::Unknown => DraftDeliveryError::Unknown,
        CatalogError::InvalidRecord => DraftDeliveryError::NotFound,
        CatalogError::ScopeMoved => DraftDeliveryError::Unavailable,
    }
}

fn store_failed(error: ObjectStoreError) -> DraftDeliveryError {
    match error {
        ObjectStoreError::NotFound
        | ObjectStoreError::AlreadyExists
        | ObjectStoreError::PreconditionFailed => DraftDeliveryError::NotFound,
        ObjectStoreError::IntegrityMismatch => DraftDeliveryError::IntegrityMismatch,
        ObjectStoreError::TooLarge => DraftDeliveryError::TooLarge,
        ObjectStoreError::DeadlineExceeded => DraftDeliveryError::DeadlineExceeded,
        ObjectStoreError::Unknown => DraftDeliveryError::Unknown,
        ObjectStoreError::Unavailable
        | ObjectStoreError::Unsupported
        | ObjectStoreError::StorageFull
        | ObjectStoreError::Denied => DraftDeliveryError::Unavailable,
    }
}

/// One written variant row that exactly matches the requested selector and the record's current
/// attempt and winners. Missing, ambiguous or stale matches fail closed.
struct ChosenWrite {
    write: WriteId,
    key: ObjectKey,
    version: ObjectVersion,
    manifest: AssetManifest,
}

fn select_written(
    asset: AssetId,
    attempt: AttemptId,
    winners: &[WriteId],
    variants: &[VariantRecord],
    variant: AssetVariant,
) -> Option<ChosenWrite> {
    let mut chosen: Option<ChosenWrite> = None;
    for row in variants {
        if row.asset != asset
            || row.attempt != attempt
            || row.variant != variant
            || row.state != VariantState::Written
            || row.key.domain() != StorageDomain::PrivateMedia
            || !winners.contains(&row.write)
        {
            continue;
        }
        let (Some(version), Some(manifest)) = (row.version.clone(), row.manifest) else {
            continue;
        };
        if chosen.is_some() {
            return None;
        }
        chosen = Some(ChosenWrite {
            write: row.write,
            key: row.key.clone(),
            version,
            manifest,
        });
    }
    chosen
}

fn check_record<'a>(
    owner: &AccountId,
    record: &'a crate::AssetRecord,
) -> Option<(&'a crate::PublishedFacts, Vec<WriteId>, u64)> {
    if record.state != AssetState::Ready
        || record.kind != AssetKind::Image
        || record.provenance != Provenance::Generated
        || record.campaign.is_some()
        || record.audience != Audience::OwnerPrivate(*owner)
    {
        return None;
    }
    match record_owner(record) {
        Some(StorageOwner::AccountDraft(account)) if account == *owner => Some((
            record.published.as_ref()?,
            record.winning_writes.clone(),
            record.suppression_generation,
        )),
        _ => None,
    }
}

/// Buffers one verified private draft image for the authenticated owner. The permit covers the
/// whole call and stays inside the candidate; the catalog reread after the byte load discards
/// the buffer when the row moved, but that reread alone never authorizes an account session.
pub async fn deliver_draft_image<C: CatalogStore, O: ObjectStore>(
    catalog: &C,
    objects: &O,
    budget: &DeliveryBudget,
    request: &DraftDeliveryRequest<'_>,
) -> Result<DraftImageCandidate, DraftDeliveryError> {
    if request.deadline.has_passed() {
        return Err(DraftDeliveryError::DeadlineExceeded);
    }
    let permit = budget.acquire().ok_or(DraftDeliveryError::Busy)?;
    let record = catalog
        .asset(request.asset)
        .await
        .map_err(catalog_failed)?
        .ok_or(DraftDeliveryError::NotFound)?;
    let (published, winners, suppression) =
        check_record(request.owner, &record).ok_or(DraftDeliveryError::NotFound)?;
    if !ALLOWED_IMAGE_MIMES.contains(&published.mime.as_str()) {
        return Err(DraftDeliveryError::UnsupportedMedia);
    }
    let variants = catalog
        .variants(request.asset)
        .await
        .map_err(catalog_failed)?;
    let chosen = select_written(
        request.asset,
        record.attempt,
        &winners,
        &variants,
        request.variant.asset_variant(),
    )
    .ok_or(DraftDeliveryError::NotFound)?;
    if chosen.manifest.byte_len == 0 || chosen.manifest.byte_len > request.limits.max_image_bytes {
        return Err(DraftDeliveryError::TooLarge);
    }
    if request.deadline.has_passed() {
        return Err(DraftDeliveryError::DeadlineExceeded);
    }
    let object = StoredObject {
        key: chosen.key.clone(),
        version: chosen.version.clone(),
        manifest: chosen.manifest,
    };
    let bytes = objects
        .get_version(&object, request.deadline)
        .await
        .map_err(store_failed)?;
    let actual_len = u64::try_from(bytes.len()).map_err(|_| DraftDeliveryError::TooLarge)?;
    if actual_len != chosen.manifest.byte_len
        || actual_len > request.limits.max_image_bytes
        || Sha256::digest(&bytes).as_slice() != chosen.manifest.sha256
    {
        return Err(DraftDeliveryError::IntegrityMismatch);
    }
    let reread = catalog
        .asset(request.asset)
        .await
        .map_err(catalog_failed)?
        .ok_or(DraftDeliveryError::NotFound)?;
    if reread.state != AssetState::Ready
        || reread.kind != record.kind
        || reread.provenance != record.provenance
        || reread.campaign != record.campaign
        || reread.attempt != record.attempt
        || reread.audience != record.audience
        || reread.charge_scope != record.charge_scope
        || reread.origin_account != record.origin_account
        || reread.winning_writes != record.winning_writes
        || reread.published != record.published
        || reread.suppression_generation != suppression
    {
        return Err(DraftDeliveryError::NotFound);
    }
    let still = catalog
        .variants(request.asset)
        .await
        .map_err(catalog_failed)?;
    let confirmed = select_written(
        request.asset,
        reread.attempt,
        &reread.winning_writes,
        &still,
        request.variant.asset_variant(),
    )
    .ok_or(DraftDeliveryError::NotFound)?;
    if confirmed.write != chosen.write
        || confirmed.key != chosen.key
        || confirmed.version != chosen.version
        || confirmed.manifest != chosen.manifest
    {
        return Err(DraftDeliveryError::NotFound);
    }
    if request.deadline.has_passed() {
        return Err(DraftDeliveryError::DeadlineExceeded);
    }
    Ok(DraftImageCandidate {
        basis: DeliveryBasis {
            asset: request.asset,
            owner: *request.owner,
            attempt: record.attempt,
            write: chosen.write,
            variant: request.variant.asset_variant(),
            version: chosen.version.as_str().to_owned(),
            digest: chosen.manifest.sha256,
            byte_len: chosen.manifest.byte_len,
            suppression_generation: suppression,
            mime: published.mime.clone(),
            published_byte_len: published.byte_len,
            published_digest: published.sha256,
        },
        mime: published.mime.clone(),
        bytes,
        permit,
    })
}

/// How long one draft image operation may take end to end, set by composition.
pub const DRAFT_IMAGE_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
