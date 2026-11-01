//! PostgreSQL adapter for the durable image execution journal (migration
//! 0040). PostgreSQL is the authority for whether a stage, dispatch, cursor,
//! checkpoint or capacity row exists. Every trait method is one transaction;
//! queries are parameterized; a failure before commit is `Unavailable` (or the
//! typed refusal the row state demands) and a failed commit is `Unknown`. The
//! statements live in `imagery_execution/statements`, taking the caller's
//! transaction, so later finance and catalog composition can call the
//! functions below in its own transaction.
//!
//! What it owns: stage claims, dispatch send/outcome rows, cursors,
//! checkpoints, durable capacity, and the accept-plus-enqueue helper. What it
//! never does: no finance writes, fence, quota or catalog writes; no provider, network
//! or clock reads; no 0039 admission binding insert (resolution reads the
//! retained allocation only); no change to existing 0039 first-output
//! admission behavior; no fabricated pre-classification asset rows. The hosted Sent
//! definer reads the exact canonical finance receipt before its execution marker.

use std::sync::Arc;

pub use df_imagery::execution::LocalSentOutcome;

use df_imagery::execution::{
    CancelCommand, CapacityBounds, ClaimCommand, CompleteCommand, DispatchId, DispatchSettlement,
    DispatchView, EnqueueCommand, EnqueueOutcome, ExecutionError, ExecutionGate, ExecutionIdentity,
    ExecutionStageId, ImageExecutionJournal, MarkSentCommand, PrepareCommand, QueuedStage,
    RefetchCommand, ReleaseCommand, ResolvedExecution, StageView, SupplierCheckpoint,
    SupplierReport,
};
use df_imagery::{AcceptCommand, AcceptOutcome, ImageJobId, ImageOwner, JournalError};
use tokio_postgres::{Config, Transaction};

mod analysis;
mod statements;
pub use analysis::{
    read_analysis_checkpoint_in, read_local_analysis_checkpoint_in, record_local_analysis_in,
    select_analysis_checkpoint_in,
};
/// Supplemental standalone analysis schema installed by the composition root after existing imagery migrations.
pub const IMAGERY_ANALYSIS_MIGRATION_VERSION: i32 = 46;
pub const IMAGERY_ANALYSIS_MIGRATION_SQL: &str = analysis::MIGRATION_SQL;

use crate::imagery_sql as journal_sql;
use crate::media_pool::Pool;
use crate::postgres_tls::PostgresTransport;

const MIGRATIONS: [(i32, &str); 1] =
    [(40, include_str!("../migrations/0040_imagery_execution.sql"))];

/// Migration 0040, applied by the composition root after 0038 (the journal)
/// is migrated. Exposed here so root needs no shared migration-list edit to
/// install it. Requires the 0038 `image_job` table for the planned-identity
/// columns and the 0039 `image_publication_allocation` table for published
/// resolution; this journal never writes that allocation.
pub const IMAGERY_EXECUTION_MIGRATION_VERSION: i32 = 40;

/// The exact SQL of migration 0040 for the composition installer.
pub const IMAGERY_EXECUTION_MIGRATION_SQL: &str =
    include_str!("../migrations/0040_imagery_execution.sql");

/// Acknowledges the actual same-call durable cost seal after exact output-only cleanup.
pub async fn acknowledge_unsent_projection_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::AcknowledgeUnsentProjection,
) -> Result<(), ExecutionError> {
    statements::acknowledge_unsent_projection_in(tx, command).await
}

/// Supplemental submit-rejection schema; installed by composition before mounting workers.
pub const IMAGERY_SUBMIT_DECLINE_MIGRATION_VERSION: i32 = 55;
pub const IMAGERY_SUBMIT_DECLINE_MIGRATION_SQL: &str =
    include_str!("../migrations/0055_imagery_submit_decline.sql");
/// Retain actual protected submit evidence before selecting a semantic decline.
pub async fn record_decline_observation_in(
    tx: &Transaction<'_>,
    observation: &df_imagery::execution::DeclineObservation,
) -> Result<df_imagery::execution::DeclineReceipt, ExecutionError> {
    statements::record_decline_observation_in(tx, observation).await
}
/// Read exact selected historical proof without acquiring execution locks after finance locks.
pub async fn read_decline_reference_in(
    tx: &Transaction<'_>,
    scope: &df_imagery::execution::ExecutionScope,
) -> Result<Option<df_imagery::execution::DeclineReceipt>, ExecutionError> {
    statements::read_decline_reference_in(tx, scope).await
}
/// Reserve a queued retry only after technical decline and retained bounds are verified.
pub async fn requeue_declined_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::DeclineRetryCommand,
) -> Result<StageView, ExecutionError> {
    statements::requeue_declined_in(tx, command).await
}

/// Opening the execution journal failed; nothing was served.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageExecutionOpenError {
    Transport(crate::postgres_tls::TransportRefusal),
    Unreachable,
    MigrationFailed,
    MissingSchema,
    InvalidConnections,
}

fn map_journal(error: JournalError) -> ExecutionError {
    match error {
        JournalError::Unavailable => ExecutionError::Unavailable,
        JournalError::Unknown => ExecutionError::Unknown,
        JournalError::InvalidRecord => ExecutionError::InvalidRecord,
        JournalError::OperationConflict => ExecutionError::OperationConflict,
        JournalError::IdentityConflict => ExecutionError::IdentityConflict,
        JournalError::JoinRefused(_) => ExecutionError::BasisConflict,
        JournalError::NotFound => ExecutionError::NotFound,
        JournalError::RecipeConflict => ExecutionError::BasisConflict,
        JournalError::Stale { current_revision } => ExecutionError::Stale { current_revision },
        JournalError::Retired => ExecutionError::Retired,
        JournalError::Expired => ExecutionError::Expired,
    }
}

/// Atomically accepts demand and reserves its durable execution slot in the
/// caller's transaction: the journal receipt plus interest, then the stage
/// row with its capacity reserve. Either conflict rolls the whole transaction
/// back together, so demand never exists without its reserve or the reverse.
/// The accepted view's job must equal the enqueue's job; anything else is an
/// identity conflict and the caller rolls back.
pub async fn accept_with_execution_in(
    tx: &Transaction<'_>,
    accept: &AcceptCommand,
    enqueue: &EnqueueCommand,
) -> Result<(AcceptOutcome, EnqueueOutcome), ExecutionError> {
    if matches!(accept.target, df_imagery::AcceptTarget::JoinReady { .. }) {
        return Err(ExecutionError::BasisConflict);
    }
    let outcome = journal_sql::accept_in(tx, accept)
        .await
        .map_err(map_journal)?;
    if outcome.view().job.job != enqueue.job {
        return Err(ExecutionError::IdentityConflict);
    }
    let reserved = statements::enqueue_stage_in(tx, enqueue).await?;
    Ok((outcome, reserved))
}

/// Plans one logical stage with its durable capacity reserve in the caller's
/// transaction. See `statements::enqueue_stage_in` for replay semantics.
pub async fn enqueue_stage_in(
    tx: &Transaction<'_>,
    command: &EnqueueCommand,
) -> Result<EnqueueOutcome, ExecutionError> {
    statements::enqueue_stage_in(tx, command).await
}

/// Claims a stage for one owner in the caller's transaction.
pub async fn claim_stage_in(
    tx: &Transaction<'_>,
    command: &ClaimCommand,
) -> Result<StageView, ExecutionError> {
    statements::claim_stage_in(tx, command).await
}

/// Prepares one send in the caller's transaction.
pub async fn prepare_dispatch_in(
    tx: &Transaction<'_>,
    command: &PrepareCommand,
) -> Result<DispatchView, ExecutionError> {
    statements::prepare_dispatch_in(tx, command).await
}

/// Commits the durable Sent marker in the caller's transaction.
pub async fn mark_sent_in(
    tx: &Transaction<'_>,
    command: &MarkSentCommand,
) -> Result<DispatchView, ExecutionError> {
    statements::mark_sent_in(tx, command).await
}

/// Correlates a supplier receipt in the caller's transaction.
pub async fn record_supplier_in(
    tx: &Transaction<'_>,
    command: &SupplierReport,
) -> Result<DispatchView, ExecutionError> {
    statements::record_supplier_in(tx, command).await
}

/// Completes one dispatch in the caller's transaction.
pub async fn complete_dispatch_in(
    tx: &Transaction<'_>,
    command: &CompleteCommand,
) -> Result<DispatchView, ExecutionError> {
    statements::complete_dispatch_in(tx, command).await
}

/// Mirrors a proven finance settlement in the caller's transaction.
pub async fn mirror_settlement_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    settlement: DispatchSettlement,
    now_ms: i64,
) -> Result<DispatchView, ExecutionError> {
    statements::mirror_settlement_in(tx, owner, job, stage, dispatch, settlement, now_ms).await
}

/// Cancels one prepared dispatch in the caller's transaction.
pub async fn cancel_dispatch_in(
    tx: &Transaction<'_>,
    command: &CancelCommand,
) -> Result<DispatchView, ExecutionError> {
    statements::cancel_dispatch_in(tx, command).await
}

/// Persists a refetch selection in the caller's transaction.
pub async fn request_refetch_in(
    tx: &Transaction<'_>,
    command: &RefetchCommand,
) -> Result<DispatchView, ExecutionError> {
    statements::request_refetch_in(tx, command).await
}

/// Releases a stage and retires its reserve in the caller's transaction.
pub async fn release_stage_in(
    tx: &Transaction<'_>,
    command: &ReleaseCommand,
) -> Result<StageView, ExecutionError> {
    statements::release_stage_in(tx, command).await
}

/// Resolves planned or published identity in the caller's transaction.
pub async fn resolve_execution_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    identity: &ExecutionIdentity,
) -> Result<ResolvedExecution, ExecutionError> {
    statements::resolve_execution_in(tx, owner, identity).await
}

/// Reads the stored stage in the caller's transaction.
pub async fn read_stage_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<StageView, ExecutionError> {
    statements::read_stage_in(tx, owner, job, stage).await
}

/// Reads one stored dispatch in the caller's transaction.
pub async fn read_dispatch_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    dispatch: &DispatchId,
) -> Result<DispatchView, ExecutionError> {
    statements::read_dispatch_in(tx, owner, dispatch).await
}

/// Sets the durable operator bounds for one capacity scope in the caller's
/// transaction, preserving the current counters. Lowering a bound below
/// current usage stops new work without invalidating existing liability.
pub async fn set_capacity_bounds_in(
    tx: &Transaction<'_>,
    scope: &str,
    bounds: &CapacityBounds,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    statements::set_capacity_bounds_in(tx, scope, bounds, now_ms).await
}

/// Sets the actively applied execution gate for one scope in the caller's
/// transaction. A committed disable stops claims, prepares and sends while
/// already-sent work keeps its liability.
pub async fn set_execution_gate_in(
    tx: &Transaction<'_>,
    scope: &str,
    applied_revision: u64,
    enabled: bool,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    statements::set_execution_gate_in(tx, scope, applied_revision, enabled, now_ms).await
}

/// Reads the actively applied execution gate for one scope in the caller's
/// transaction.
pub async fn read_execution_gate_in(
    tx: &Transaction<'_>,
    scope: &str,
) -> Result<ExecutionGate, ExecutionError> {
    statements::read_execution_gate_in(tx, scope).await
}

/// Scans planned or admitted stages in one scope in the caller's
/// transaction, oldest first. Reads only; claiming rechecks under lock.
pub async fn scan_queue_in(
    tx: &Transaction<'_>,
    scope: &str,
    limit: u32,
    now_ms: i64,
) -> Result<Vec<QueuedStage>, ExecutionError> {
    statements::scan_queue_in(tx, scope, limit, now_ms).await
}

/// Retains one versioned supplier resume checkpoint in the caller's
/// transaction. An identical version replays; a conflicting one is refused.
pub async fn record_supplier_checkpoint_in(
    tx: &Transaction<'_>,
    checkpoint: &SupplierCheckpoint,
) -> Result<SupplierCheckpoint, ExecutionError> {
    statements::record_supplier_checkpoint_in(tx, checkpoint).await
}

/// Reads the latest retained supplier checkpoint for one dispatch in the
/// caller's transaction, if any.
pub async fn read_supplier_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<SupplierCheckpoint>, ExecutionError> {
    statements::read_supplier_checkpoint_in(tx, owner, job, stage, dispatch).await
}

/// Reads retained output metadata for local or hosted work under its owned job.
pub async fn read_output_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<df_imagery::execution::OutputCheckpoint>, ExecutionError> {
    statements::read_output_checkpoint_in(tx, owner, job, stage, dispatch).await
}

/// Reads a bounded queue page without resetting a denied prefix or its ranking epoch.
pub async fn scan_queue_after_in(
    tx: &Transaction<'_>,
    scope: &str,
    after: Option<df_imagery::execution::QueueCursor>,
    limit: u32,
    now_ms: i64,
) -> Result<df_imagery::execution::QueuePage, ExecutionError> {
    statements::scan_queue_after_in(tx, scope, after, limit, now_ms).await
}

/// Discovers due retained dispatches with stable dispatch-id pagination.
pub async fn scan_recovery_in(
    tx: &Transaction<'_>,
    scope: &str,
    after: Option<DispatchId>,
    limit: u32,
    now_ms: i64,
) -> Result<Vec<df_imagery::execution::RecoveryStage>, ExecutionError> {
    statements::scan_recovery_in(tx, scope, after, limit, now_ms).await
}

/// Discovers actual terminal no-dispatch snapshots, revalidated under the writer's owned job lock.
pub async fn scan_unsent_terminal_in(
    tx: &Transaction<'_>,
    scope: &str,
    after: Option<df_imagery::execution::TerminalCursor>,
    limit: u32,
    now_ms: i64,
) -> Result<Vec<df_imagery::execution::UnsentTerminal>, ExecutionError> {
    statements::scan_unsent_terminal_in(tx, scope, after, limit, now_ms).await
}

/// Discovers retained unsent metadata without authorizing send, seal or financial cancellation.
pub async fn scan_unsent_prepared_in(
    tx: &Transaction<'_>,
    scope: &str,
    after: Option<DispatchId>,
    limit: u32,
    now_ms: i64,
) -> Result<Vec<df_imagery::execution::UnsentPrepared>, ExecutionError> {
    statements::scan_unsent_prepared_in(tx, scope, after, limit, now_ms).await
}

/// Claims exclusive reconciliation ownership without granting any fresh-send authority.
pub async fn claim_recovery_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::ClaimRecoveryCommand,
) -> Result<df_imagery::execution::RecoveryLease, ExecutionError> {
    statements::claim_recovery_in(tx, command).await
}

/// Records verified terminal receipts or the next due poll for one exact recovery lease.
pub async fn finish_recovery_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::FinishRecoveryCommand,
) -> Result<(), ExecutionError> {
    statements::finish_recovery_in(tx, command).await
}

/// Checks an exclusive recovery lease inside the publication owner's existing transaction.
pub async fn validate_recovery_in(
    tx: &Transaction<'_>,
    lease: &df_imagery::execution::RecoveryLease,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    statements::validate_recovery_in(tx, lease, now_ms).await
}

/// Reads canonical stage input/configuration and immutable dependency for finance validation.
pub async fn read_stage_basis_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<df_imagery::execution::StageBasis, ExecutionError> {
    statements::read_stage_basis_in(tx, owner, job, stage).await
}

/// Retains private decision evidence only after the corresponding raw usage observation is durable.
pub async fn record_decision_observation_in(
    tx: &Transaction<'_>,
    observation: &df_imagery::execution::DecisionObservation,
) -> Result<df_imagery::execution::DecisionObservation, ExecutionError> {
    statements::record_decision_observation_in(tx, observation).await
}

/// Reads exact or latest scoped source evidence; this read makes no semantic approval claim.
pub async fn read_decision_observation_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    version: Option<u32>,
) -> Result<Option<df_imagery::execution::DecisionObservation>, ExecutionError> {
    statements::read_decision_observation_in(tx, owner, job, stage, dispatch, version).await
}

/// Reads only the selected terminal decision payload for one exact owned dispatch.
pub async fn read_decision_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<df_imagery::execution::DecisionObservation>, ExecutionError> {
    statements::read_decision_checkpoint_in(tx, owner, job, stage, dispatch).await
}

/// Retains a protected real-image candidate observation after exact raw usage bytes exist.
pub async fn record_candidate_observation_in(
    tx: &Transaction<'_>,
    observation: &df_imagery::execution::CandidateObservation,
) -> Result<df_imagery::execution::CandidateReceipt, ExecutionError> {
    statements::record_candidate_observation_in(tx, observation).await
}
/// Reads exact or latest measured candidate metadata within the caller's transaction.
pub async fn read_candidate_observation_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
    version: Option<u32>,
) -> Result<Option<df_imagery::execution::CandidateReceipt>, ExecutionError> {
    statements::read_candidate_observation_in(tx, owner, job, stage, dispatch, version).await
}
/// Reads the winning intermediate receipt while retaining the exact owned job lock.
pub async fn read_candidate_checkpoint_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<df_imagery::execution::CandidateReceipt>, ExecutionError> {
    statements::read_candidate_checkpoint_in(tx, owner, job, stage, dispatch).await
}

/// Prepares only local finalization under an active exact parent candidate recovery lease.
pub async fn prepare_reconciliation_in(
    tx: &Transaction<'_>,
    parent: &df_imagery::execution::RecoveryLease,
    command: &df_imagery::execution::ReconciliationCommand,
) -> Result<df_imagery::execution::ReconciliationDispatch, ExecutionError> {
    statements::prepare_reconciliation_in(tx, parent, command).await
}

/// Reads installed canonical host capacity/payload/fairness policy without inventing defaults.
pub async fn read_capacity_bounds_in(
    tx: &Transaction<'_>,
    scope: &str,
) -> Result<CapacityBounds, ExecutionError> {
    statements::read_capacity_bounds_in(tx, scope).await
}
/// Reads the exact scoped current cursor, independent of any planned/publication allocation route.
pub async fn read_current_dispatch_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<Option<DispatchView>, ExecutionError> {
    statements::read_current_dispatch_in(tx, owner, job, stage).await
}
/// Persists the first scheduling hint for the actual accepted interest in the caller's transaction.
pub async fn record_urgency_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    interest: &df_imagery::ImageInterestId,
    urgency: df_imagery::Urgency,
    now_ms: i64,
) -> Result<df_imagery::Urgency, ExecutionError> {
    statements::record_urgency_in(tx, owner, job, interest, urgency, now_ms).await
}

/// Commits a LOCAL-only marker inside the caller's current source/grant-locked transaction.
pub async fn mark_local_sent_in(
    tx: &Transaction<'_>,
    command: &MarkSentCommand,
) -> Result<df_imagery::execution::LocalSentOutcome, ExecutionError> {
    statements::mark_local_sent_in(tx, command).await
}

/// Commits a caller-owned execution transaction. A failed commit is
/// `Unknown`: the writes may or may not have taken effect, so the caller
/// keeps the involved ids and rereads before retrying, and never resends on
/// this error. Covers only the store commit, not provider dispatch.
pub async fn commit_execution(transaction: Transaction<'_>) -> Result<(), ExecutionError> {
    transaction.commit().await.map_err(|error| {
        statements::record_execution_failure(&error);
        ExecutionError::Unknown
    })
}

/// The PostgreSQL image execution journal. Clones share one connection pool.
#[derive(Clone)]
pub struct PostgresImageExecution {
    pool: Arc<Pool>,
    source_authority: Option<Arc<dyn crate::imagery_publication::PublicationSourceAuthority>>,
}

impl PostgresImageExecution {
    /// Reuses the host's current source grant policy for deterministic analytical completion.
    pub fn with_source_authority(
        mut self,
        authority: Arc<dyn crate::imagery_publication::PublicationSourceAuthority>,
    ) -> Self {
        self.source_authority = Some(authority);
        self
    }
    /// Opens a migrated execution schema without DDL; the root installer owns all migrations.
    pub async fn open_existing(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
    ) -> Result<Self, ImageExecutionOpenError> {
        crate::postgres_tls::check_transport(&config, &transport)
            .map_err(ImageExecutionOpenError::Transport)?;
        if connections == 0 {
            return Err(ImageExecutionOpenError::InvalidConnections);
        }
        let pool = Pool::new(config, transport, connections);
        {
            let mut slot = pool
                .checkout()
                .await
                .map_err(|_| ImageExecutionOpenError::Unreachable)?;
            let client = slot
                .client
                .as_mut()
                .ok_or(ImageExecutionOpenError::Unreachable)?;
            let ready:bool=client.query_one("SELECT to_regclass('imagery.image_job') IS NOT NULL \
                AND to_regclass('imagery.image_dispatch_recovery') IS NOT NULL \
                AND to_regclass('imagery.image_decline_observation') IS NOT NULL AND to_regclass('imagery.image_decline_checkpoint') IS NOT NULL \
                AND to_regclass('imagery.image_local_analysis') IS NOT NULL \
                AND to_regclass('imagery.image_analysis_checkpoint') IS NOT NULL \
                AND to_regclass('imagery.image_decision_observation') IS NOT NULL \
                AND to_regclass('imagery.image_candidate_checkpoint') IS NOT NULL \
                AND to_regclass('imagery.image_interest_schedule') IS NOT NULL \
                AND to_regclass('imagery.image_dispatch_usage') IS NOT NULL \
                AND to_regprocedure('imagery.mark_local_dispatch_sent(bytea,bytea,bytea,bytea,bytea,bigint,bigint)') IS NOT NULL \
                AND to_regprocedure('imagery.mark_hosted_dispatch_sent(bytea,bytea,bytea,bytea,bytea,bigint,bigint,bytea,bytea,bytea)') IS NOT NULL",&[])
                .await.map_err(|_|ImageExecutionOpenError::MissingSchema)?.get(0);
            if !ready {
                return Err(ImageExecutionOpenError::MissingSchema);
            }
            client
                .query(
                    "SELECT intent_digest,config_digest FROM imagery.image_job LIMIT 0",
                    &[],
                )
                .await
                .map_err(|_| ImageExecutionOpenError::MissingSchema)?;
            client.query("SELECT schema_revision,validator_revision FROM imagery.image_local_inspection LIMIT 0",&[])
                .await.map_err(|_|ImageExecutionOpenError::MissingSchema)?;
            client.query("SELECT decision_payload_bytes_bound,urgency_promotion_ms FROM imagery.image_execution_capacity LIMIT 0",&[])
                .await.map_err(|_|ImageExecutionOpenError::MissingSchema)?;
            client.query("SELECT o.payload_sha256,d.claim_lease_until_ms,d.cost_unsent_ack,s.reconciliation_dispatch,s.reconciliation_token,s.reconciliation_generation,s.queued_not_before_ms,s.queued_notice,s.retry_dispatch_id \
                FROM imagery.image_decision_observation o CROSS JOIN imagery.image_dispatch_attempt d CROSS JOIN imagery.image_execution_stage s LIMIT 0",&[])
                .await.map_err(|_|ImageExecutionOpenError::MissingSchema)?;
        }
        Ok(Self {
            pool: Arc::new(pool),
            source_authority: None,
        })
    }

    pub async fn open(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
    ) -> Result<Self, ImageExecutionOpenError> {
        crate::postgres_tls::check_transport(&config, &transport)
            .map_err(ImageExecutionOpenError::Transport)?;
        let pool = Pool::new(config, transport, connections);
        {
            let mut slot = pool
                .checkout()
                .await
                .map_err(|_| ImageExecutionOpenError::Unreachable)?;
            let client = slot
                .client
                .as_mut()
                .ok_or(ImageExecutionOpenError::Unreachable)?;
            migrate(client)
                .await
                .map_err(|_| ImageExecutionOpenError::MigrationFailed)?;
        }
        Ok(Self {
            pool: Arc::new(pool),
            source_authority: None,
        })
    }
}

/// Runs `$body` (an expression using `.await` and `?`) in one READ COMMITTED
/// transaction from the pool; an early return drops the transaction, which
/// rolls it back. A failed commit is `Unknown`: the writes may or may not
/// have taken effect, so the caller keeps its ids and rereads before
/// retrying, and never resends on this error.
macro_rules! in_execution_transaction {
    ($self:expr, $tx:ident => $body:expr) => {{
        let mut slot = $self
            .pool
            .checkout()
            .await
            .map_err(|_| ExecutionError::Unavailable)?;
        let client = slot.client.as_mut().ok_or(ExecutionError::Unavailable)?;
        let $tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(|_| ExecutionError::Unavailable)?;
        let value = $body;
        commit_execution($tx).await?;
        Ok(value)
    }};
}

impl ImageExecutionJournal for PostgresImageExecution {
    async fn read_analysis_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
    ) -> Result<Option<df_imagery::execution::AnalysisCheckpoint>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_analysis_checkpoint_in(&tx,owner,job).await?)
    }
    async fn select_analysis_checkpoint(
        &self,
        proof: &df_imagery::runtime::QualifiedAnalysis,
        now_ms: i64,
    ) -> Result<df_imagery::execution::AnalysisCheckpoint, ExecutionError> {
        analysis::select(self, proof, now_ms).await
    }
    async fn record_local_analysis(
        &self,
        proof: &df_imagery::runtime::QualifiedLocalAnalysis,
        claim: &df_imagery::execution::StageClaim,
        now_ms: i64,
    ) -> Result<df_imagery::execution::LocalAnalysisCheckpoint, ExecutionError> {
        analysis::record_local(self, proof, claim, now_ms).await
    }
    async fn read_local_analysis_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<Option<df_imagery::execution::LocalAnalysisCheckpoint>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_local_analysis_checkpoint_in(&tx,owner,job,stage).await?)
    }
    async fn enqueue(&self, command: &EnqueueCommand) -> Result<EnqueueOutcome, ExecutionError> {
        in_execution_transaction!(self, tx => enqueue_stage_in(&tx, command).await?)
    }

    async fn claim(&self, command: &ClaimCommand) -> Result<StageView, ExecutionError> {
        in_execution_transaction!(self, tx => claim_stage_in(&tx, command).await?)
    }

    async fn scan_queue(
        &self,
        scope: &str,
        limit: u32,
        now_ms: i64,
    ) -> Result<Vec<QueuedStage>, ExecutionError> {
        in_execution_transaction!(self, tx => scan_queue_in(&tx, scope, limit, now_ms).await?)
    }

    async fn scan_recovery(
        &self,
        scope: &str,
        after: Option<DispatchId>,
        limit: u32,
        now_ms: i64,
    ) -> Result<Vec<df_imagery::execution::RecoveryStage>, ExecutionError> {
        in_execution_transaction!(self, tx => scan_recovery_in(&tx,scope,after,limit,now_ms).await?)
    }

    async fn scan_queue_after(
        &self,
        scope: &str,
        after: Option<df_imagery::execution::QueueCursor>,
        limit: u32,
        now_ms: i64,
    ) -> Result<df_imagery::execution::QueuePage, ExecutionError> {
        in_execution_transaction!(self, tx => scan_queue_after_in(&tx,scope,after,limit,now_ms).await?)
    }

    async fn scan_unsent_terminal(
        &self,
        scope: &str,
        after: Option<df_imagery::execution::TerminalCursor>,
        limit: u32,
        now_ms: i64,
    ) -> Result<Vec<df_imagery::execution::UnsentTerminal>, ExecutionError> {
        in_execution_transaction!(self,tx=>scan_unsent_terminal_in(&tx,scope,after,limit,now_ms).await?)
    }

    async fn scan_unsent_prepared(
        &self,
        scope: &str,
        after: Option<DispatchId>,
        limit: u32,
        now_ms: i64,
    ) -> Result<Vec<df_imagery::execution::UnsentPrepared>, ExecutionError> {
        in_execution_transaction!(self, tx => scan_unsent_prepared_in(&tx,scope,after,limit,now_ms).await?)
    }

    async fn claim_recovery(
        &self,
        command: &df_imagery::execution::ClaimRecoveryCommand,
    ) -> Result<df_imagery::execution::RecoveryLease, ExecutionError> {
        in_execution_transaction!(self, tx => claim_recovery_in(&tx,command).await?)
    }

    async fn finish_recovery(
        &self,
        command: &df_imagery::execution::FinishRecoveryCommand,
    ) -> Result<(), ExecutionError> {
        in_execution_transaction!(self, tx => finish_recovery_in(&tx,command).await?)
    }

    async fn read_stage_basis(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<df_imagery::execution::StageBasis, ExecutionError> {
        in_execution_transaction!(self,tx=>read_stage_basis_in(&tx,owner,job,stage).await?)
    }
    async fn acknowledge_unsent_projection(
        &self,
        command: &df_imagery::execution::AcknowledgeUnsentProjection,
    ) -> Result<(), ExecutionError> {
        in_execution_transaction!(self,tx=>acknowledge_unsent_projection_in(&tx,command).await?)
    }
    async fn record_decline_observation(
        &self,
        observation: &df_imagery::execution::DeclineObservation,
    ) -> Result<df_imagery::execution::DeclineReceipt, ExecutionError> {
        in_execution_transaction!(self,tx=>record_decline_observation_in(&tx,observation).await?)
    }
    async fn read_decline_reference(
        &self,
        scope: &df_imagery::execution::ExecutionScope,
    ) -> Result<Option<df_imagery::execution::DeclineReceipt>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_decline_reference_in(&tx,scope).await?)
    }
    async fn requeue_declined(
        &self,
        command: &df_imagery::execution::DeclineRetryCommand,
    ) -> Result<StageView, ExecutionError> {
        in_execution_transaction!(self,tx=>requeue_declined_in(&tx,command).await?)
    }

    async fn record_decision_observation(
        &self,
        observation: &df_imagery::execution::DecisionObservation,
    ) -> Result<df_imagery::execution::DecisionObservation, ExecutionError> {
        in_execution_transaction!(self,tx=>record_decision_observation_in(&tx,observation).await?)
    }
    async fn read_decision_observation(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
        version: Option<u32>,
    ) -> Result<Option<df_imagery::execution::DecisionObservation>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_decision_observation_in(&tx,owner,job,stage,dispatch,version).await?)
    }
    async fn read_decision_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> Result<Option<df_imagery::execution::DecisionObservation>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_decision_checkpoint_in(&tx,owner,job,stage,dispatch).await?)
    }

    async fn record_candidate_observation(
        &self,
        observation: &df_imagery::execution::CandidateObservation,
    ) -> Result<df_imagery::execution::CandidateReceipt, ExecutionError> {
        in_execution_transaction!(self,tx=>record_candidate_observation_in(&tx,observation).await?)
    }
    async fn read_candidate_observation(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
        version: Option<u32>,
    ) -> Result<Option<df_imagery::execution::CandidateReceipt>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_candidate_observation_in(&tx,owner,job,stage,dispatch,version).await?)
    }
    async fn read_candidate_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> Result<Option<df_imagery::execution::CandidateReceipt>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_candidate_checkpoint_in(&tx,owner,job,stage,dispatch).await?)
    }

    async fn prepare_reconciliation(
        &self,
        parent: &df_imagery::execution::RecoveryLease,
        command: &df_imagery::execution::ReconciliationCommand,
    ) -> Result<df_imagery::execution::ReconciliationDispatch, ExecutionError> {
        in_execution_transaction!(self,tx=>prepare_reconciliation_in(&tx,parent,command).await?)
    }

    async fn read_current_dispatch(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<Option<DispatchView>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_current_dispatch_in(&tx,owner,job,stage).await?)
    }
    async fn read_child_stage(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        parent_dispatch: &DispatchId,
        kind: df_imagery::execution::StageKind,
    ) -> Result<Option<StageView>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_child_stage_in(&tx,owner,job,parent_dispatch,kind).await?)
    }
    async fn defer_queued(
        &self,
        command: &df_imagery::execution::DeferQueuedCommand,
    ) -> Result<(), ExecutionError> {
        in_execution_transaction!(self,tx=>defer_queued_in(&tx,command).await?)
    }
    async fn record_local_inspection(
        &self,
        command: &df_imagery::execution::LocalInspectionCommand,
    ) -> Result<df_imagery::execution::LocalInspectionCheckpoint, ExecutionError> {
        in_execution_transaction!(self,tx=>record_local_inspection_in(&tx,command).await?)
    }
    async fn read_local_inspection(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<Option<df_imagery::execution::LocalInspectionCheckpoint>, ExecutionError> {
        in_execution_transaction!(self,tx=>read_local_inspection_in(&tx,owner,job,stage).await?)
    }
    async fn read_capacity_bounds(&self, scope: &str) -> Result<CapacityBounds, ExecutionError> {
        in_execution_transaction!(self,tx=>read_capacity_bounds_in(&tx,scope).await?)
    }

    async fn prepare(&self, command: &PrepareCommand) -> Result<DispatchView, ExecutionError> {
        in_execution_transaction!(self, tx => prepare_dispatch_in(&tx, command).await?)
    }

    async fn mark_sent(&self, command: &MarkSentCommand) -> Result<DispatchView, ExecutionError> {
        in_execution_transaction!(self, tx => mark_sent_in(&tx, command).await?)
    }

    async fn report_supplier(
        &self,
        command: &SupplierReport,
    ) -> Result<DispatchView, ExecutionError> {
        in_execution_transaction!(self, tx => record_supplier_in(&tx, command).await?)
    }

    async fn record_supplier_checkpoint(
        &self,
        checkpoint: &SupplierCheckpoint,
    ) -> Result<SupplierCheckpoint, ExecutionError> {
        in_execution_transaction!(self, tx => record_supplier_checkpoint_in(&tx, checkpoint).await?)
    }

    async fn read_supplier_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> Result<Option<SupplierCheckpoint>, ExecutionError> {
        in_execution_transaction!(self, tx => {
            read_supplier_checkpoint_in(&tx, owner, job, stage, dispatch).await?
        })
    }

    async fn read_output_checkpoint(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
        dispatch: &DispatchId,
    ) -> Result<Option<df_imagery::execution::OutputCheckpoint>, ExecutionError> {
        in_execution_transaction!(self, tx => {
            read_output_checkpoint_in(&tx, owner, job, stage, dispatch).await?
        })
    }

    async fn complete(&self, command: &CompleteCommand) -> Result<DispatchView, ExecutionError> {
        in_execution_transaction!(self, tx => complete_dispatch_in(&tx, command).await?)
    }

    async fn cancel(&self, command: &CancelCommand) -> Result<DispatchView, ExecutionError> {
        in_execution_transaction!(self, tx => cancel_dispatch_in(&tx, command).await?)
    }

    async fn request_refetch(
        &self,
        command: &RefetchCommand,
    ) -> Result<DispatchView, ExecutionError> {
        in_execution_transaction!(self, tx => request_refetch_in(&tx, command).await?)
    }

    async fn release(&self, command: &ReleaseCommand) -> Result<StageView, ExecutionError> {
        in_execution_transaction!(self, tx => release_stage_in(&tx, command).await?)
    }

    async fn read_stage(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        stage: &ExecutionStageId,
    ) -> Result<StageView, ExecutionError> {
        in_execution_transaction!(self, tx => read_stage_in(&tx, owner, job, stage).await?)
    }

    async fn read_dispatch(
        &self,
        owner: &ImageOwner,
        dispatch: &DispatchId,
    ) -> Result<DispatchView, ExecutionError> {
        in_execution_transaction!(self, tx => read_dispatch_in(&tx, owner, dispatch).await?)
    }

    async fn resolve(
        &self,
        owner: &ImageOwner,
        identity: &ExecutionIdentity,
    ) -> Result<ResolvedExecution, ExecutionError> {
        in_execution_transaction!(self, tx => resolve_execution_in(&tx, owner, identity).await?)
    }

    async fn set_capacity_bounds(
        &self,
        scope: &str,
        bounds: &CapacityBounds,
        now_ms: i64,
    ) -> Result<(), ExecutionError> {
        in_execution_transaction!(self, tx => set_capacity_bounds_in(&tx, scope, bounds, now_ms).await?)
    }

    async fn set_execution_gate(
        &self,
        scope: &str,
        applied_revision: u64,
        enabled: bool,
        now_ms: i64,
    ) -> Result<(), ExecutionError> {
        in_execution_transaction!(self, tx => {
            set_execution_gate_in(&tx, scope, applied_revision, enabled, now_ms).await?
        })
    }

    async fn read_execution_gate(&self, scope: &str) -> Result<ExecutionGate, ExecutionError> {
        in_execution_transaction!(self, tx => read_execution_gate_in(&tx, scope).await?)
    }
}

/// Applies the execution migration once, serialised by an advisory lock. Uses
/// the same `imagery.schema_migration` version table as the journal so root's
/// composition and this opener never apply it twice. Requires the 0038
/// `image_job` table for the planned-identity columns; without it the batch
/// fails and open reports `MigrationFailed`.
async fn migrate(client: &mut tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    let transaction = client.transaction().await?;
    transaction
        .batch_execute(
            "SELECT pg_advisory_xact_lock(hashtext('df_imagery_execution')); \
             CREATE SCHEMA IF NOT EXISTS imagery; \
             CREATE TABLE IF NOT EXISTS imagery.schema_migration ( \
                 version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())",
        )
        .await?;
    for (version, script) in MIGRATIONS {
        let applied = transaction
            .query_opt(
                "SELECT 1 FROM imagery.schema_migration WHERE version = $1",
                &[&version],
            )
            .await?
            .is_some();
        if !applied {
            transaction.batch_execute(script).await?;
            transaction
                .execute(
                    "INSERT INTO imagery.schema_migration (version) VALUES ($1)",
                    &[&version],
                )
                .await?;
        }
    }
    transaction.commit().await
}

/// Reads immutable selected decision metadata without loading or hashing private payload bytes.
pub async fn read_decision_reference_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<df_imagery::execution::DecisionReference>, ExecutionError> {
    statements::read_decision_reference_in(tx, owner, job, stage, dispatch).await
}

/// Mirrors Hosted Sent only after the exact canonical finance attempt was sent in this transaction.
/// Source/grant verification and its locks remain mandatory in the caller's finance send port.
pub async fn mark_hosted_sent_in(
    tx: &Transaction<'_>,
    command: &MarkSentCommand,
    finance: &df_imagery::execution::FinanceTuple,
) -> Result<DispatchView, ExecutionError> {
    statements::mark_hosted_sent_in(tx, command, finance).await
}

/// Reads the unique immutable child of an exact owned same-job parent without mutating demand.
pub async fn read_child_stage_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    parent_dispatch: &DispatchId,
    kind: df_imagery::execution::StageKind,
) -> Result<Option<StageView>, ExecutionError> {
    statements::read_child_stage_in(tx, owner, job, parent_dispatch, kind).await
}

/// Defers unsent source-denied work without retaining an active slot or creating new demand.
pub async fn defer_queued_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::DeferQueuedCommand,
) -> Result<(), ExecutionError> {
    statements::defer_queued_in(tx, command).await
}

/// Caller holds the owned job lock; reads exact current candidate metadata without execution locks.
pub async fn read_candidate_reference_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<df_imagery::execution::CandidateReceipt>, ExecutionError> {
    statements::read_candidate_reference_in(tx, owner, job, stage, dispatch).await
}
/// Caller holds the owned job lock; reads exact current decision metadata without execution locks.
pub async fn read_decision_reference_unlocked_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
    dispatch: &DispatchId,
) -> Result<Option<df_imagery::execution::DecisionReference>, ExecutionError> {
    statements::read_decision_reference_unlocked_in(tx, owner, job, stage, dispatch).await
}

/// Records a protected BestEffort-only local pre-egress failure with no supplier dispatch or usage.
pub async fn record_local_inspection_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::LocalInspectionCommand,
) -> Result<df_imagery::execution::LocalInspectionCheckpoint, ExecutionError> {
    statements::record_local_inspection_in(tx, command).await
}
pub async fn read_local_inspection_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    stage: &ExecutionStageId,
) -> Result<Option<df_imagery::execution::LocalInspectionCheckpoint>, ExecutionError> {
    statements::read_local_inspection_in(tx, owner, job, stage).await
}

/// Atomically adopts a unique strictly newer startup revision; stale hosts cannot lower the gate.
pub async fn advance_execution_gate_in(
    tx: &Transaction<'_>,
    scope: &str,
    expected_revision: u64,
    enabled: bool,
    now_ms: i64,
) -> Result<ExecutionGate, ExecutionError> {
    statements::advance_execution_gate_in(tx, scope, expected_revision, enabled, now_ms).await
}
impl PostgresImageExecution {
    /// The operator mount adopts this acknowledged epoch before starting owned work.
    pub async fn advance_execution_gate(
        &self,
        scope: &str,
        expected_revision: u64,
        enabled: bool,
        now_ms: i64,
    ) -> Result<ExecutionGate, ExecutionError> {
        in_execution_transaction!(self,tx=>advance_execution_gate_in(&tx,scope,expected_revision,enabled,now_ms).await?)
    }
}

/// First-write terminal CAS only; exact immutable seal replay is recognized by publication first.
pub async fn validate_terminal_tail_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::TerminalCommand,
) -> Result<df_imagery::execution::TerminalValidation, ExecutionError> {
    statements::validate_terminal_tail_in(tx, command).await
}
/// Consumes the exact inserted seal, closes unsent capacity and retires the job once atomically.
pub async fn close_terminal_in(
    tx: &Transaction<'_>,
    command: &df_imagery::execution::TerminalCommand,
) -> Result<u64, ExecutionError> {
    statements::close_terminal_in(tx, command).await
}
/// Caller owns the job lock; canonical NoDelivery fences output only, never existing reconciliation.
pub async fn check_output_open_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<(), ExecutionError> {
    statements::check_output_open_in(tx, owner, job).await
}
