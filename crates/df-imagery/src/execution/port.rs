//! Neutral async port for the durable image execution journal.
//!
//! This trait owns the execution side of the journal: queue scanning,
//! enqueueing with durable reserve, the operator capacity and execution
//! gates, claim/prepare/send/supplier/complete/cancel/refetch/release, and
//! reads plus identity resolution. It performs no provider calls, pixel
//! work, storage, scheduling, authorization, finance writes or money
//! movement, and it never reads the clock, the environment or a credential.
//! Every scope, pin and fingerprint here is a caller claim; the service
//! authorizes the caller, while the store still enforces exact stored-owner
//! equality on every access.
//!
//! Replay rules live with each method: an exact replay of a claim or
//! dispatch returns the original row, a mismatched replay conflicts, and a
//! new send never supersedes a prepared, sent or unknown current attempt.
//! A failed commit is `Unknown`: the caller keeps its ids and rereads before
//! retrying, and never resends on that error.

use std::future::Future;

use super::ExecutionSetupError;
use super::{
    CancelCommand, ClaimCommand, CompleteCommand, DispatchId, DispatchView, EnqueueCommand,
    EnqueueOutcome, ExecutionError, ExecutionIdentity, ExecutionStageId, LocalAnalysisCheckpoint,
    LocalInspectionCheckpoint, MarkSentCommand, PrepareCommand, RefetchCommand, ReleaseCommand,
    ResolvedExecution, StageKind, StageState, StageView, SupplierCheckpoint, SupplierReport,
};
use crate::journal::ImageJobId;
use crate::request::ImageOwner;

/// One queued stage for the runtime's overall queue scan: planned or
/// admitted work in one capacity scope, oldest first. Claiming still goes
/// through `claim`, which rechecks the gate, lease and capacity under lock.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedStage {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub kind: StageKind,
    pub state: StageState,
    pub claim_generation: u64,
    pub lease_until_ms: i64,
    pub created_at_ms: i64,
}

/// Durable operator bounds for one capacity scope: active concurrency plus
/// the pending queue (job count and retained intent/config bytes) tracked
/// separately from it. Lowering a bound stops new work; it never invalidates
/// existing liability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityBounds {
    pub max_active: i64,
    pub max_pending_jobs: i64,
    pub max_pending_bytes: i64,
    pub max_decision_payload_bytes: i64,
    pub urgency_promotion_ms: i64,
}

/// Default fairness interval; every waiting stage reaches the visible FIFO bucket in ten seconds.
const DEFAULT_URGENCY_PROMOTION_MS: i64 = 5_000;

impl CapacityBounds {
    pub fn new(
        max_active: i64,
        max_pending_jobs: i64,
        max_pending_bytes: i64,
    ) -> Result<Self, ExecutionSetupError> {
        if max_active <= 0 || max_pending_jobs <= 0 || max_pending_bytes <= 0 {
            return Err(ExecutionSetupError::BadRevision);
        }
        Ok(Self {
            max_active,
            max_pending_jobs,
            max_pending_bytes,
            max_decision_payload_bytes: super::DEFAULT_DECISION_PAYLOAD_BYTES,
            urgency_promotion_ms: DEFAULT_URGENCY_PROMOTION_MS,
        })
    }
    /// Positive host fairness interval; two promotions suffice to join the visible FIFO bucket.
    pub fn with_urgency_promotion_ms(
        mut self,
        promotion_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        if promotion_ms <= 0 {
            return Err(ExecutionSetupError::BadTime);
        }
        self.urgency_promotion_ms = promotion_ms;
        Ok(self)
    }
    /// Host policy for newly retained decision payloads; exact historical replay is unaffected.
    pub fn with_decision_payload_bytes(
        mut self,
        maximum_bytes: i64,
    ) -> Result<Self, ExecutionSetupError> {
        if maximum_bytes <= 0 || maximum_bytes > super::MAXIMUM_SQL_DECISION_PAYLOAD_BYTES {
            return Err(ExecutionSetupError::ZeroBytes);
        }
        self.max_decision_payload_bytes = maximum_bytes;
        Ok(self)
    }
}

/// The actively applied execution gate for one scope: the revision every
/// dispatch owner has acknowledged, plus enablement. Claims name the
/// revision they hold; a committed disable refuses claims, prepares and
/// sends while already-sent work keeps its liability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionGate {
    pub scope: String,
    pub applied_revision: u64,
    pub enabled: bool,
}

/// Durable image execution journal. Every method is one transaction; see
/// each method for its replay and refusal contract. No method touches
/// finance rows: finance tuples stay opaque bytes admitted by their owner.
pub trait ImageExecutionJournal: Send + Sync {
    /// Plans one logical stage with its durable pending reserve, or replays
    /// the identical stage. A different immutable basis conflicts.
    fn enqueue(
        &self,
        command: &EnqueueCommand,
    ) -> impl Future<Output = Result<EnqueueOutcome, ExecutionError>> + Send;

    /// Claims a stage under the actively applied gate revision: an exact
    /// replay returns the current claim, an expired lease supersedes with a
    /// fresh token, and a live lease under another token is lost.
    fn claim(
        &self,
        command: &ClaimCommand,
    ) -> impl Future<Output = Result<StageView, ExecutionError>> + Send;

    /// Oldest planned or admitted stages in one scope for the runtime to
    /// claim. Reads only; claiming still rechecks everything under lock.
    fn scan_queue(
        &self,
        scope: &str,
        limit: u32,
        now_ms: i64,
    ) -> impl Future<Output = Result<Vec<QueuedStage>, ExecutionError>> + Send;

    /// Continues a bounded queue traversal under its frozen priority-ranking epoch.
    /// Eligibility uses actual now; wrapping None begins a new ranking epoch.
    fn scan_queue_after(
        &self,
        scope: &str,
        after: Option<super::QueueCursor>,
        limit: u32,
        now_ms: i64,
    ) -> impl Future<Output = Result<super::QueuePage, ExecutionError>> + Send;

    /// Discovers due retained dispatches after the cursor, excluding active owners and acknowledged
    /// work.
    fn scan_recovery(
        &self,
        scope: &str,
        after: Option<DispatchId>,
        limit: u32,
        now_ms: i64,
    ) -> impl Future<Output = Result<Vec<super::RecoveryStage>, ExecutionError>> + Send;

    /// Discovers finite actual terminal no-dispatch states; no row grants work or send authority.
    fn scan_unsent_terminal(
        &self,
        scope: &str,
        after: Option<super::TerminalCursor>,
        limit: u32,
        now_ms: i64,
    ) -> impl Future<Output = Result<Vec<super::UnsentTerminal>, ExecutionError>> + Send;

    /// Bounded historical Prepared discovery, plus exact sealed Released/Held tails for
    /// cancellation retry.
    /// Expiry alone never authorizes terminal cleanup or financial release.
    fn scan_unsent_prepared(
        &self,
        scope: &str,
        after: Option<DispatchId>,
        limit: u32,
        now_ms: i64,
    ) -> impl Future<Output = Result<Vec<super::UnsentPrepared>, ExecutionError>> + Send;

    /// Claims reconciliation ownership without changing the original stage claim or send authority.
    fn claim_recovery(
        &self,
        command: &super::ClaimRecoveryCommand,
    ) -> impl Future<Output = Result<super::RecoveryLease, ExecutionError>> + Send;

    /// Acknowledges durable terminal receipts or defers a bounded next poll, with exact lease
    /// replay.
    fn finish_recovery(
        &self,
        command: &super::FinishRecoveryCommand,
    ) -> impl Future<Output = Result<(), ExecutionError>> + Send;

    /// Prepares only local finalization under an exact active candidate recovery lease.
    fn prepare_reconciliation(
        &self,
        parent: &super::RecoveryLease,
        command: &super::ReconciliationCommand,
    ) -> impl Future<Output = Result<super::ReconciliationDispatch, ExecutionError>> + Send;

    /// Reads canonical immutable stage input/configuration and parent dependency.
    fn read_stage_basis(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> impl Future<Output = Result<super::StageBasis, ExecutionError>> + Send;

    /// Closes the exact sealed never-sent cost obligation only after the actual durable Book
    /// tombstone.
    fn acknowledge_unsent_projection(
        &self,
        command: &super::AcknowledgeUnsentProjection,
    ) -> impl Future<Output = Result<(), ExecutionError>> + Send;

    /// Persists protected technical submit rejection only after exact raw evidence is durable.
    fn record_decline_observation(
        &self,
        observation: &super::DeclineObservation,
    ) -> impl Future<Output = Result<super::DeclineReceipt, ExecutionError>> + Send;
    /// Exact selected historical metadata; cursor advancement does not erase old liability.
    fn read_decline_reference(
        &self,
        scope: &super::ExecutionScope,
    ) -> impl Future<Output = Result<Option<super::DeclineReceipt>, ExecutionError>> + Send;
    /// Requeues only a proved decline within retained retry policy; grants no send authority.
    fn requeue_declined(
        &self,
        command: &super::DeclineRetryCommand,
    ) -> impl Future<Output = Result<StageView, ExecutionError>> + Send;

    /// Retains observed decision bytes only after their exact raw usage revision is durable.
    fn record_decision_observation(
        &self,
        observation: &super::DecisionObservation,
    ) -> impl Future<Output = Result<super::DecisionObservation, ExecutionError>> + Send;

    /// Reads an exact observation version, or the latest retained raw observation with None.
    fn read_decision_observation(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
        version: Option<u32>,
    ) -> impl Future<Output = Result<Option<super::DecisionObservation>, ExecutionError>> + Send;

    /// Reads only the winning terminal decision selection, never an image or semantic approval.
    fn read_decision_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> impl Future<Output = Result<Option<super::DecisionObservation>, ExecutionError>> + Send;

    /// Retains inspected intermediate image metadata only after exact raw supplier bytes are
    /// durable.
    fn record_candidate_observation(
        &self,
        observation: &super::CandidateObservation,
    ) -> impl Future<Output = Result<super::CandidateReceipt, ExecutionError>> + Send;
    /// Reads exact or latest intermediate observations without claiming a final image is ready.
    fn read_candidate_observation(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
        version: Option<u32>,
    ) -> impl Future<Output = Result<Option<super::CandidateReceipt>, ExecutionError>> + Send;
    /// Reads only the immutable winning intermediate candidate selection.
    fn read_candidate_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> impl Future<Output = Result<Option<super::CandidateReceipt>, ExecutionError>> + Send;

    /// Reads the exact scoped current cursor; no planned-publication routing is inferred.
    fn read_current_dispatch(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> impl Future<Output = Result<Option<DispatchView>, ExecutionError>> + Send;
    /// Reads the canonical child identity; no candidate stage id is minted during recovery.
    fn read_child_stage(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        parent_dispatch: &DispatchId,
        kind: super::StageKind,
    ) -> impl Future<Output = Result<Option<StageView>, ExecutionError>> + Send;
    /// Defers only unsent work with no finance/current dispatch/live claim, returning held
    /// capacity.
    fn defer_queued(
        &self,
        command: &super::DeferQueuedCommand,
    ) -> impl Future<Output = Result<(), ExecutionError>> + Send;
    /// Records protected BestEffort pre-egress failure without inventing supplier usage or a
    /// dispatch.
    fn record_local_inspection(
        &self,
        command: &super::LocalInspectionCommand,
    ) -> impl Future<Output = Result<LocalInspectionCheckpoint, ExecutionError>> + Send;
    fn read_local_inspection(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> impl Future<Output = Result<Option<LocalInspectionCheckpoint>, ExecutionError>> + Send;
    /// Reads the one explicitly selected complete analytical result; intermediate decisions are not
    /// final.
    fn read_analysis_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
    ) -> impl Future<Output = Result<Option<super::AnalysisCheckpoint>, ExecutionError>> + Send;
    /// Selects the complete retained supplier/local chain using an outside-transaction private
    /// proof.
    fn select_analysis_checkpoint(
        &self,
        proof: &crate::runtime::QualifiedAnalysis,
        now_ms: i64,
    ) -> impl Future<Output = Result<super::AnalysisCheckpoint, ExecutionError>> + Send;
    /// Atomically records a real deterministic no-call result and its explicit final selection.
    fn record_local_analysis(
        &self,
        proof: &crate::runtime::QualifiedLocalAnalysis,
        claim: &super::StageClaim,
        now_ms: i64,
    ) -> impl Future<Output = Result<LocalAnalysisCheckpoint, ExecutionError>> + Send;
    fn read_local_analysis_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> impl Future<Output = Result<Option<LocalAnalysisCheckpoint>, ExecutionError>> + Send;
    /// Reads installed canonical host bounds; absence is an error, never fabricated defaults.
    fn read_capacity_bounds(
        &self,
        scope: &str,
    ) -> impl Future<Output = Result<CapacityBounds, ExecutionError>> + Send;

    /// Prepares one send with the next sequence and immutable retry lineage.
    /// An identical dispatch id replays; a new id while the cursor names a
    /// prepared, sent or unknown dispatch is refused, never superseded.
    fn prepare(
        &self,
        command: &PrepareCommand,
    ) -> impl Future<Output = Result<DispatchView, ExecutionError>> + Send;

    /// Commits the durable Sent marker for the cursor's current dispatch
    /// only, after the finance owner committed its own. Already-sent replays.
    fn mark_sent(
        &self,
        command: &MarkSentCommand,
    ) -> impl Future<Output = Result<DispatchView, ExecutionError>> + Send;

    /// Correlates a supplier receipt with its dispatch. The same id replays;
    /// a different id over a recorded one conflicts. Needs no live claim so
    /// late receipts still correlate.
    fn report_supplier(
        &self,
        command: &SupplierReport,
    ) -> impl Future<Output = Result<DispatchView, ExecutionError>> + Send;

    /// Retains one versioned supplier resume checkpoint for fal/BFL resume
    /// without resending. An identical version replays; a conflicting one is
    /// refused and the winning write stands.
    fn record_supplier_checkpoint(
        &self,
        checkpoint: &SupplierCheckpoint,
    ) -> impl Future<Output = Result<SupplierCheckpoint, ExecutionError>> + Send;

    /// The latest retained supplier checkpoint for one dispatch, if any.
    fn read_supplier_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> impl Future<Output = Result<Option<SupplierCheckpoint>, ExecutionError>> + Send;

    /// Reads scoped output metadata for local or hosted work. Catalog validation remains separate.
    fn read_output_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> impl Future<Output = Result<Option<super::OutputCheckpoint>, ExecutionError>> + Send;

    /// Completes the cursor's current dispatch. Terminal replay compares the
    /// full outcome and checkpoint: identical replays, conflicting refuses.
    /// Unknown accepts a late correlated receipt or recovered output without
    /// a new send and keeps its liability until finance mirrors settlement.
    fn complete(
        &self,
        command: &CompleteCommand,
    ) -> impl Future<Output = Result<DispatchView, ExecutionError>> + Send;

    /// Cancels one dispatch proven unsent. Sent, terminal and unknown work
    /// refuse; the finance owner releases its hold first.
    fn cancel(
        &self,
        command: &CancelCommand,
    ) -> impl Future<Output = Result<DispatchView, ExecutionError>> + Send;

    /// Persists which dispatch a refetch was requested for.
    fn request_refetch(
        &self,
        command: &RefetchCommand,
    ) -> impl Future<Output = Result<DispatchView, ExecutionError>> + Send;

    /// Releases a stage with no outstanding dispatch and retires its
    /// reserve. Sent work never loses its liability row here.
    fn release(
        &self,
        command: &ReleaseCommand,
    ) -> impl Future<Output = Result<StageView, ExecutionError>> + Send;

    /// Reads the stored stage with its current job revision.
    fn read_stage(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> impl Future<Output = Result<StageView, ExecutionError>> + Send;

    /// Reads one stored dispatch. A wrong owner learns nothing.
    fn read_dispatch(
        &self,
        owner: &ImageOwner,
        dispatch: &DispatchId,
    ) -> impl Future<Output = Result<DispatchView, ExecutionError>> + Send;

    /// Resolves planned or published identity to its job and stage. The
    /// published pair follows the retained allocation's stage id exactly;
    /// the planned pair prefers the output stage over classification.
    fn resolve(
        &self,
        owner: &ImageOwner,
        identity: &ExecutionIdentity,
    ) -> impl Future<Output = Result<ResolvedExecution, ExecutionError>> + Send;

    /// Sets the durable operator bounds for one capacity scope.
    fn set_capacity_bounds(
        &self,
        scope: &str,
        bounds: &CapacityBounds,
        now_ms: i64,
    ) -> impl Future<Output = Result<(), ExecutionError>> + Send;

    /// Sets the actively applied execution gate for one scope. A committed
    /// disable stops claims, prepares and sends; reconciliation continues.
    fn set_execution_gate(
        &self,
        scope: &str,
        applied_revision: u64,
        enabled: bool,
        now_ms: i64,
    ) -> impl Future<Output = Result<(), ExecutionError>> + Send;

    /// Reads the actively applied execution gate for one scope.
    fn read_execution_gate(
        &self,
        scope: &str,
    ) -> impl Future<Output = Result<ExecutionGate, ExecutionError>> + Send;
}
