//! Demand journal composition: accepted demand and durable queue reservation commit together.

use crate::media_pool::Pool;
use crate::postgres_tls::PostgresTransport;
use df_imagery::execution::{CapacityScope, ExecutionError};
use df_imagery::runtime::RuntimeIds;
use df_imagery::{
    AcceptCommand, AcceptOutcome, FreezeCommand, FreezeOutcome, ImageInterestId, ImageJob,
    ImageJobId, ImageJournal, ImageOwner, JournalError, OperationView, WithdrawOutcome,
};
use df_types::OperationId;
use std::sync::Arc;
use tokio_postgres::Config;

/// Runtime journal opened against an already migrated image/catalog database.
#[derive(Clone)]
pub struct PostgresRuntimeJournal<I> {
    pool: Arc<Pool>,
    ids: Arc<I>,
    scope: CapacityScope,
    active_config_revision: u64,
    local_accounting: bool,
}

impl<I: RuntimeIds> PostgresRuntimeJournal<I> {
    /// Captures a newly acknowledged gate revision while retaining the same bounded pool.
    pub fn with_config_revision(
        &self,
        revision: u64,
        local_accounting: bool,
    ) -> Result<Self, JournalError> {
        if revision == 0 {
            return Err(JournalError::InvalidRecord);
        }
        Ok(Self {
            pool: self.pool.clone(),
            ids: self.ids.clone(),
            scope: self.scope.clone(),
            active_config_revision: revision,
            local_accounting,
        })
    }

    pub fn open_existing(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
        ids: Arc<I>,
        scope: CapacityScope,
        active_config_revision: u64,
        local_accounting: bool,
    ) -> Result<Self, JournalError> {
        crate::postgres_tls::check_transport(&config, &transport)
            .map_err(|_| JournalError::Unavailable)?;
        if active_config_revision == 0 || connections == 0 {
            return Err(JournalError::InvalidRecord);
        }
        Ok(Self {
            pool: Arc::new(Pool::new(config, transport, connections)),
            ids,
            scope,
            active_config_revision,
            local_accounting,
        })
    }
}

fn execution_error(error: ExecutionError) -> JournalError {
    match error {
        ExecutionError::Unknown => JournalError::Unknown,
        ExecutionError::Unavailable
        | ExecutionError::CapacityFull
        | ExecutionError::Disabled
        | ExecutionError::ConfigFenced { .. } => JournalError::Unavailable,
        ExecutionError::OperationConflict => JournalError::OperationConflict,
        ExecutionError::IdentityConflict => JournalError::IdentityConflict,
        _ => JournalError::InvalidRecord,
    }
}

macro_rules! transaction {
    ($self:expr, $tx:ident => $body:expr) => {{
        let mut slot = $self
            .pool
            .checkout()
            .await
            .map_err(|_| JournalError::Unavailable)?;
        let client = slot.client.as_mut().ok_or(JournalError::Unavailable)?;
        let $tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(|_| JournalError::Unavailable)?;
        let value = $body;
        $tx.commit().await.map_err(|error| {
            // A failed commit may or may not have taken effect; the cause must not be silent.
            df_observe::record_audit_line(
                "IMAGERY",
                format!(
                    "IMAGERY journal_commit_failed sqlstate={} constraint={}",
                    error.code().map_or("none", |code| code.code()),
                    error
                        .as_db_error()
                        .and_then(|db| db.constraint())
                        .unwrap_or("none")
                ),
            );
            JournalError::Unknown
        })?;
        Ok(value)
    }};
}

impl<I: RuntimeIds> ImageJournal for PostgresRuntimeJournal<I> {
    async fn compatible_candidates(
        &self,
        owner: &ImageOwner,
        intent: &df_imagery::VersionedSnapshot,
        config: &df_imagery::VersionedSnapshot,
        after: Option<df_imagery::CompatibleCursor>,
    ) -> Result<Vec<OperationView>, JournalError> {
        transaction!(self,tx=>crate::imagery_sql::compatible_candidates_in(&tx,owner,intent,config,after).await?)
    }
    async fn accept(&self, command: &AcceptCommand) -> Result<AcceptOutcome, JournalError> {
        self.accept_scheduled(command, df_imagery::Urgency::Present)
            .await
    }
    async fn accept_scheduled(
        &self,
        command: &AcceptCommand,
        urgency: df_imagery::Urgency,
    ) -> Result<AcceptOutcome, JournalError> {
        transaction!(self, tx => {
            if let Some(view) = crate::imagery_store::lookup_in(&tx, &command.identity.owner,
                command.identity.operation).await? {
                // Exact replay remains the journal's responsibility, without minting reservation ids.
                if view.receipt.fingerprint != command.identity.fingerprint || view.job.intent != command.intent {
                    return Err(JournalError::OperationConflict);
                }
                crate::imagery_execution::record_urgency_in(&tx,&command.identity.owner,&view.job.job,
                    &view.interest.interest,urgency,command.now_ms).await.map_err(execution_error)?;
                AcceptOutcome::Replayed(view)
            } else {
                if matches!(command.target,df_imagery::AcceptTarget::Join{..}|df_imagery::AcceptTarget::JoinReady{..}) {
                    let outcome=crate::imagery_store::accept_in(&tx,command).await?;
                    if matches!(command.target,df_imagery::AcceptTarget::Join{..}) {
                        let job=outcome.view().job.job;
                        let active=tx.query_one("SELECT NOT EXISTS(SELECT 1 FROM imagery.image_no_delivery n WHERE n.job_id=$1) AND NOT EXISTS(SELECT 1 FROM imagery.image_job_result r WHERE r.job_id=$1) AND (EXISTS(SELECT 1 FROM imagery.image_execution_stage s WHERE s.job_id=$1 AND s.stage_state IN ('planned','admitted')) OR EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt d JOIN imagery.image_stage_cursor c ON c.current_dispatch=d.dispatch_id WHERE d.job_id=$1 AND (d.dispatch_state IN ('sent','unknown') OR d.settlement IN ('held','unknown_outstanding'))))",
                            &[&job.as_bytes().as_slice()]).await.map_err(|_|JournalError::Unavailable)?.get::<_,bool>(0);
                        if !active {return Err(JournalError::JoinRefused(df_imagery::JoinRefusal::IntentMismatch));}
                        crate::imagery_execution::record_urgency_in(&tx,&command.identity.owner,&job,&outcome.view().interest.interest,urgency,command.now_ms).await.map_err(execution_error)?;
                    }
                    outcome
                } else {
                let enqueue = df_imagery::runtime::reservation_for(command, self.scope.clone(),
                    self.active_config_revision, self.local_accounting, self.ids.as_ref())?;
                let (outcome, _) = crate::imagery_execution::accept_with_execution_in(&tx, command, &enqueue).await
                    .map_err(execution_error)?;
                let current = crate::imagery_store::lookup_in(&tx, &command.identity.owner,
                    command.identity.operation).await?.ok_or(JournalError::InvalidRecord)?;
                crate::imagery_execution::record_urgency_in(&tx,&command.identity.owner,&current.job.job,
                    &current.interest.interest,urgency,command.now_ms).await.map_err(execution_error)?;
                match outcome {
                    AcceptOutcome::Created(_) => AcceptOutcome::Created(current),
                    AcceptOutcome::Joined(_) => AcceptOutcome::Joined(current),
                    AcceptOutcome::Replayed(_) => AcceptOutcome::Replayed(current),
                }
                }
            }
        })
    }
    async fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> Result<Option<OperationView>, JournalError> {
        transaction!(self, tx => crate::imagery_store::lookup_in(&tx, owner, operation).await?)
    }
    async fn read_job(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> Result<Option<ImageJob>, JournalError> {
        transaction!(self, tx => crate::imagery_store::read_in(&tx, owner, job).await?)
    }
    async fn freeze_recipe(&self, command: &FreezeCommand) -> Result<FreezeOutcome, JournalError> {
        transaction!(self, tx => crate::imagery_store::freeze_in(&tx, command).await?)
    }
    async fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> Result<WithdrawOutcome, JournalError> {
        transaction!(self, tx => crate::imagery_store::withdraw_in(&tx, owner, interest, now_ms).await?)
    }
    async fn withdraw_owner_interests(
        &self,
        owner: &ImageOwner,
        now_ms: i64,
    ) -> Result<Vec<WithdrawOutcome>, JournalError> {
        transaction!(self, tx => crate::imagery_sql::withdraw_owner_in(&tx, owner, now_ms).await?)
    }
}

mod usage;
pub use usage::{
    IMAGERY_USAGE_MIGRATION_SQL, IMAGERY_USAGE_MIGRATION_VERSION, PostgresImageUsage,
    read_usage_in, usage_definitive_failure_in, usage_output_matches_in, usage_recorded_in,
    usage_unknown_in,
};

mod observations;
pub use observations::PostgresImageObservations;

mod sources;
pub use sources::{read_sources_in, record_sources_in, sources_digest_in};

pub mod usage_projection;
pub use usage_projection::{UsageProjectionRows, read_usage_in as read_usage_projection_in};
