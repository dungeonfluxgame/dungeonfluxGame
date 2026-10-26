//! The operator review facade (O2): a bounded pending list, safe metadata, an authorized
//! review-byte handle and a CAS decision. It reuses the catalog lifecycle; it never publishes.
//!
//! Frozen lifecycle meaning (O2, this module): a *pending manual screen* is a current attempt in
//! `Quarantined` or `Validated` with staged bytes recorded, no operator decision yet for that
//! attempt, an unresolved screen (`media.upload.screen` in pending/flagged/unjudged, or for
//! generated rows `judgement = 'unjudged'`), and an unexpired review deadline when one is set.
//! Approval requires `Validated`: decode/bounds validation has already passed under the step
//! lease. Approval only records the decision and marks the screen passed; the catalog owner's
//! `finalize` remains the sole path to `Ready`, so no code here writes `state = 'ready'`, moves
//! quota, or writes the settlement outbox on approval. Rejection is the catalog's terminal
//! rejection (`screen_rejected`): reservation released and exactly one outbox row, atomically
//! with the retained decision/audit row. A decided, rejected, expired, deleted or superseded
//! attempt can never be decided again.

use std::future::Future;

use df_types::{AccountId, OperationId};
use sha2::{Digest, Sha256};

use crate::asset_lifecycle::AssetState;
use crate::catalog_record::{AssetKind, Provenance};
use crate::media_ids::{AssetId, AttemptId};

/// Hard bound for one pending page; adapters clamp any larger request to this.
pub const MAX_PENDING_REVIEWS: usize = 100;
/// A decision reason is 1..=500 trimmed characters (ADMINFLOW design, duties table).
pub const MAX_REASON_CHARS: usize = 500;

/// The operator's verdict on one pending item.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReviewDecision {
    Approved,
    Rejected,
}

impl ReviewDecision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// Server-resolved operator context. The caller (AF19 composition) fills this from its
/// authenticated operator session and fresh-MFA check; this facade never trusts a raw header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperatorContext {
    pub operator: AccountId,
    /// The operator grant/session revision the composition resolved; carried into the audit row.
    pub grant_revision: i64,
}

/// One decision request. `expected_revision` is the `revision` of the pending item the operator
/// saw; `operation` makes a retry with the same payload return the first receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewRequest {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub expected_revision: i64,
    pub operation: OperationId,
    pub decision: ReviewDecision,
    pub reason: String,
}

impl ReviewRequest {
    /// The trimmed reason, or why it is unusable. Reasons are stored only in the audit row.
    pub fn validated_reason(&self) -> Result<&str, ReviewError> {
        let reason = self.reason.trim();
        if reason.is_empty() || reason.chars().count() > MAX_REASON_CHARS {
            return Err(ReviewError::InvalidReason);
        }
        Ok(reason)
    }

    /// Stable fingerprint of the retry payload (operator excluded: the operation id is already
    /// bound to one operator by the unique key's first writer, checked on replay).
    pub fn fingerprint(&self, operator: AccountId) -> [u8; 32] {
        let reason = self.reason.trim();
        let mut hash = Sha256::new();
        hash.update(b"df-operator-review-v1\0");
        hash.update(operator.as_bytes());
        hash.update(self.asset.as_bytes());
        hash.update(self.attempt.as_bytes());
        hash.update(self.expected_revision.to_be_bytes());
        hash.update([self.decision.label().len() as u8]);
        hash.update(self.decision.label().as_bytes());
        hash.update((reason.len() as u32).to_be_bytes());
        hash.update(reason.as_bytes());
        hash.finalize().into()
    }
}

/// Safe pending metadata: digests and sizes only. It deliberately has no object key, staged
/// key, version string or URL field, so it can cross to an operator client unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingReview {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub kind: AssetKind,
    pub provenance: Provenance,
    pub state: AssetState,
    /// Compare-and-set revision: the row's `updated_at_ms` when the item was read.
    pub revision: i64,
    pub byte_len: u64,
    pub sha256: [u8; 32],
    pub created_at_ms: i64,
    pub staged_at_ms: i64,
    /// `staged_at_ms + quarantine_max_age` for uploads; `None` when no deadline is recorded.
    pub review_deadline_ms: Option<i64>,
}

/// The retained decision. One row is both the receipt and the durable audit record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewReceipt {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub decision: ReviewDecision,
    pub operator: AccountId,
    pub operation: OperationId,
    pub decided_at_ms: i64,
    /// The asset row's revision after the decision committed.
    pub revision: i64,
}

/// An authorized handle to staged bytes. It names only ids and the expected digest; the byte
/// store resolves any private locator internally and never returns it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReviewByteHandle {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

/// Why a review operation refused or failed. Denied and absent bytes are indistinguishable
/// to callers outside the owning adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewError {
    InvalidReason,
    /// The item is not pending (wrong state, already decided, expired, or gone).
    NotPending {
        state: Option<AssetState>,
    },
    /// The attempt moved or the expected revision no longer matches.
    StaleRevision,
    /// A different payload already used this operation id.
    OperationConflict,
    /// Approval was attempted before validation passed.
    NotValidated,
    /// The review deadline passed; the sweeper owns expiry.
    Expired,
    /// Bytes are absent or the caller is not authorized to see them.
    BytesDenied,
    /// Stored bytes do not match the recorded digest/length.
    IntegrityMismatch,
    Unavailable,
    Unknown,
    InvalidRecord,
}

/// The pure approval gate shared by every adapter: only a validated current attempt may be
/// approved. Rejection is allowed from either pending state.
pub fn approval_permitted(state: AssetState) -> bool {
    state == AssetState::Validated
}

pub fn decision_permitted(state: AssetState) -> bool {
    matches!(state, AssetState::Quarantined | AssetState::Validated)
}

/// The durable operator review port. Every method is one transaction in the adapter.
pub trait OperatorReviewStore: Send + Sync {
    /// Pending items, oldest first, at most `limit` (clamped to `MAX_PENDING_REVIEWS`).
    fn pending_reviews(
        &self,
        now_ms: i64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<PendingReview>, ReviewError>> + Send;

    /// The byte handle for one pending item, only while it is still pending and current.
    fn review_byte_handle(
        &self,
        asset: AssetId,
        attempt: AttemptId,
    ) -> impl Future<Output = Result<ReviewByteHandle, ReviewError>> + Send;

    /// CAS decision: exactly one winner per attempt; a same-payload retry under the same
    /// operation id returns the first receipt, a different payload conflicts.
    fn decide(
        &self,
        context: OperatorContext,
        request: ReviewRequest,
        now_ms: i64,
    ) -> impl Future<Output = Result<ReviewReceipt, ReviewError>> + Send;

    /// The retained receipt for an operation, for retry-after-unknown recovery.
    fn receipt(
        &self,
        operation: OperationId,
    ) -> impl Future<Output = Result<Option<ReviewReceipt>, ReviewError>> + Send;
}

/// Reads staged bytes for a handle and verifies them against its digest before returning any.
/// Implementations resolve private locators internally; the locator never leaves the store.
pub trait ReviewByteStore: Send + Sync {
    fn read_exact(&self, handle: ReviewByteHandle) -> Result<Vec<u8>, ReviewError>;
}

/// A faithful local byte fixture: exact bytes in memory, hash-checked on every read. It stands
/// in for the private object store in tests; production adapters resolve the staged object
/// internally and apply the same `verify_review_bytes` check before disclosing bytes.
#[cfg(any(test, feature = "test-fakes"))]
pub struct LocalReviewBytes {
    objects: std::sync::Mutex<std::collections::HashMap<(AssetId, AttemptId), Vec<u8>>>,
}

#[cfg(any(test, feature = "test-fakes"))]
impl LocalReviewBytes {
    pub fn new() -> Self {
        Self {
            objects: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn insert(&self, asset: AssetId, attempt: AttemptId, bytes: Vec<u8>) {
        self.objects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((asset, attempt), bytes);
    }
}

#[cfg(any(test, feature = "test-fakes"))]
impl Default for LocalReviewBytes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-fakes"))]
impl ReviewByteStore for LocalReviewBytes {
    fn read_exact(&self, handle: ReviewByteHandle) -> Result<Vec<u8>, ReviewError> {
        let bytes = self
            .objects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(handle.asset, handle.attempt))
            .cloned()
            .ok_or(ReviewError::BytesDenied)?;
        verify_review_bytes(handle, &bytes)?;
        Ok(bytes)
    }
}

/// Verifies `bytes` are exactly the handle's staged bytes (length and SHA-256).
pub fn verify_review_bytes(handle: ReviewByteHandle, bytes: &[u8]) -> Result<(), ReviewError> {
    if bytes.len() as u64 != handle.byte_len {
        return Err(ReviewError::IntegrityMismatch);
    }
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    if digest != handle.sha256 {
        return Err(ReviewError::IntegrityMismatch);
    }
    Ok(())
}
