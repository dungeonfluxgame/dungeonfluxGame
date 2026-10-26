//! The ports the money core needs from other owners (planning/finance-ledger-research.md 11.2,
//! 18), each with a faithful fake in `fakes`. Consumer-owned: the owners implement them when
//! their rounds land.
//!
//! * `FencePort` is O1's `df_runtime.fence_current` called inside the admission transaction.
//! * `MediaPort` is O2's `media_catalog::admit_generation` called inside the same transaction.
//! * `MemberPort` is S1's lock-free `member_snapshot` read, per payer, at admission.
//!
//! TODO(FIN-08): replace the fakes with the owners' adapters.

use std::collections::BTreeSet;
use std::future::Future;

use df_commerce::finance_ids::{AttemptId, CampaignKey, PayerId, TenantKey};
use tokio_postgres::Transaction;

/// Time and identity sources, injected so tests are deterministic.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// Sixteen-byte ids; uniqueness is the only contract.
pub trait IdSource: Send + Sync {
    fn next_id(&self) -> [u8; 16];
}

/// O1's three-part owner fence (6.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FenceRequest {
    pub tenant: TenantKey,
    pub campaign: CampaignKey,
    pub recovery_epoch: i64,
    pub claim_seq: i64,
    pub placement_id: [u8; 16],
}

/// What the fence said.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FenceVerdict {
    Current,
    /// The owner changed.
    Stale,
    /// Money reconstruction is missing or failed for the campaign.
    PaidWorkFrozen,
    /// The service is halting (`DF002`).
    ServiceHalting,
}

/// The port failed to answer; the admission is retried, never half done.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortUnavailable;

pub trait FencePort: Send + Sync {
    fn fence_current(
        &self,
        transaction: &Transaction<'_>,
        request: &FenceRequest,
    ) -> impl Future<Output = Result<FenceVerdict, PortUnavailable>> + Send;
}

/// O2's media admission inputs; the catalog decides whether the work may start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaRequest {
    pub attempt: AttemptId,
    pub asset: [u8; 16],
}

/// What O2's `admit_generation` said; a refusal rolls the whole admission back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaVerdict {
    Admitted,
    InFlight,
    Ready,
    AudienceEmpty,
    QuotaFull,
}

pub trait MediaPort: Send + Sync {
    fn admit_generation(
        &self,
        transaction: &Transaction<'_>,
        request: &MediaRequest,
    ) -> impl Future<Output = Result<MediaVerdict, PortUnavailable>> + Send;
}

/// What one processor refund call concluded (14.1). A timeout is `Unknown`, never a
/// failure: the refund lot keeps its hold until a conclusive answer with the same key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefundCall {
    Paid { processor_refund_ref: String },
    KnownFailure,
    Unknown,
}

/// The payment processor's refund call, keyed by the refund lot id (14.1): one
/// processor refund per original charge, however often the call is retried.
pub trait RefundProcessor: Send + Sync {
    fn refund(
        &self,
        idempotency_key: &str,
        amount_micro_dollars: u64,
    ) -> impl Future<Output = RefundCall> + Send;
}

pub trait MemberPort: Send + Sync {
    /// The payers among `payers` whose fresh `member_snapshot` for `campaign` is authorized.
    fn authorized_payers(
        &self,
        campaign: CampaignKey,
        payers: &[PayerId],
    ) -> impl Future<Output = Result<BTreeSet<PayerId>, PortUnavailable>> + Send;
}
