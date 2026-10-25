//! The ports the catalog service needs from other owners (planning/asset-storage-research.md 6.2),
//! each with a faithful fake in `catalog_ports_fake`. Nothing here reaches a provider: F decides
//! money, O1's dispatcher fetches provider output, and the validator is this crate's own decode
//! and re-encode step.

use crate::asset_lifecycle::RejectReason;
use crate::catalog_record::AssetKind;
use crate::media_ids::{AssetId, AttemptId};
use crate::object_store::Deadline;
use std::future::Future;

/// Wall-clock milliseconds since the Unix epoch, read at the edge and injected everywhere else.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// Where F's `ProviderAttempt` stands, as far as the catalog needs to know (plan 6.2, 6.3).
/// TODO(O2-01): align with F's `ProviderAttempt` states in `planning/finance-ledger-research.md`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderAttemptState {
    /// Admitted and reserved; the dispatcher has not claimed it.
    Reserved,
    DispatchPrepared,
    Sent,
    OutcomeUnknown,
    /// The customer hold was released: a money event, not a supplier outcome.
    CustomerReleased,
    /// The supplier answered; a failed result has no output to wait for.
    ResultKnown {
        succeeded: bool,
        at_ms: i64,
    },
    /// F's terminal failure after its reconciliation limit (D25); F credits the customer.
    FailedAfterReconciliation,
    /// `cancel_unclaimed_effect` succeeded: nothing was sent.
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelOutcome {
    Cancelled,
    /// The dispatcher claimed the effect first; it may already be sent.
    AlreadyClaimed,
}

/// F could not answer; the caller changes nothing and retries at its next pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinanceUnavailable;

/// What the sweeper reads from and asks of F.
/// TODO(O2-02): the real cancel runs inside the same PostgreSQL transaction as the catalog rejection
/// (F rows first, D25); this port is the contract until F's transaction-taking API lands.
pub trait FinanceAttempts: Send + Sync {
    fn attempt_state(
        &self,
        attempt: AttemptId,
    ) -> impl Future<Output = Result<ProviderAttemptState, FinanceUnavailable>> + Send;

    /// Conditional on the effect being unclaimed; exactly one of cancel and claim wins.
    fn cancel_unclaimed_effect(
        &self,
        attempt: AttemptId,
    ) -> impl Future<Output = Result<CancelOutcome, FinanceUnavailable>> + Send;
}

/// The dispatcher's answer to a re-fetch request (D18, D23).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefetchAnswer {
    /// The dispatcher will stage the output through the staging endpoint.
    Requested,
    /// No spool and an expired fetch handle: the output is gone for good.
    OutputGone,
    /// A timeout or any unknown answer: the row stays in flight.
    Unavailable,
}

/// The egress dispatcher, from the catalog's side.
/// TODO(O2-03): the dispatcher's staging call, durable output hold and idempotent re-fetch (O1).
pub trait Dispatcher: Send + Sync {
    /// Idempotent per attempt; never calls the provider again.
    fn request_refetch(
        &self,
        asset: AssetId,
        attempt: AttemptId,
    ) -> impl Future<Output = RefetchAnswer> + Send;
}

/// The variants a validated upload or output becomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedMedia {
    pub master: Vec<u8>,
    pub display: Option<Vec<u8>>,
    pub mime: String,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
}

/// Decode, bounds checks and re-encode of staged bytes (13.4).
/// TODO(O2-04): the real implementation is `df-assets::validation` (FILE-01) in the upload round.
pub trait MediaValidator: Send + Sync {
    fn validate(
        &self,
        kind: AssetKind,
        staged: &[u8],
    ) -> impl Future<Output = Result<ValidatedMedia, RejectReason>> + Send;
}

/// Why the provider output stream gave no next chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamError {
    /// The stream broke before its end.
    Failed,
    /// The chunk did not arrive by its deadline.
    DeadlineExceeded,
}

/// The provider output stream the staging endpoint reads, chunk by chunk.
pub trait StagingStream: Send {
    /// The next chunk, or `None` at the end of the stream. The implementation gives up with
    /// `DeadlineExceeded` when `deadline` passes; the caller also bounds the whole transfer.
    fn next_chunk(
        &mut self,
        deadline: Deadline,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, StreamError>> + Send;
}
