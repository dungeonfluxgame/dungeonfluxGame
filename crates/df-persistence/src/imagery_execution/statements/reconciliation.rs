//! Recovery-authorized LOCAL finalization of an exact already-paid candidate.
//! No demand, finance admission, supplier routing or audience grant is created here.
use super::common::{
    bump_job_revision, invalid, locked_job, map_db, move_pending_to_active_in, planned_pair,
    read_dispatch_view, require_read_committed, reserve_active_in,
};
use df_imagery::execution::{
    DispatchState, EnqueueCommand, EnqueueOutcome, ExecutionError, ExecutionIdentity,
    ReconciliationCommand, ReconciliationDispatch, RecoveryLease, StageBasis, StageClaim,
    StageKind, StageScope,
};
use tokio_postgres::Transaction;

/// Prepares or joins one canonical local child while preserving all paid-attempt identities.
pub(crate) async fn prepare_reconciliation_in(
    tx: &Transaction<'_>,
    parent: &RecoveryLease,
    command: &ReconciliationCommand,
) -> Result<ReconciliationDispatch, ExecutionError> {
    require_read_committed(tx).await?;
    if command.now_ms <= 0 || command.recipe_revision == 0 {
        return Err(ExecutionError::InvalidRecord);
    }
    super::recovery::validate_recovery_in(tx, parent, command.now_ms).await?;
    super::terminal::check_output_open_in(tx, &parent.stage.owner, &parent.stage.job).await?;
    let authority = tx
        .query_one(
            "SELECT r.lease_until_ms,d.claim_token FROM imagery.image_dispatch_recovery r \
                JOIN imagery.image_dispatch_attempt d USING(dispatch_id) WHERE r.dispatch_id=$1",
            &[&parent.stage.current.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?;
    let parent_lease_until: i64 = authority.get("lease_until_ms");
    if command.claim == parent.token
        || authority.get::<_, Vec<u8>>("claim_token") == command.claim.as_bytes()
    {
        return Err(ExecutionError::BasisConflict);
    }
    let owner = &parent.stage.owner;
    let job = &parent.stage.job;
    let candidate = read_reconciliation_candidate(tx, parent, command).await?;
    let (_, _, _, _, asset, attempt, job_bytes) = locked_job(tx, owner, job).await?;
    let (asset, attempt) = planned_pair(asset, attempt)?;
    let enqueue = EnqueueCommand::new(
        StageScope {
            owner: owner.clone(),
            job: *job,
            stage: command.child,
        },
        StageBasis {
            kind: StageKind::LocalTransform,
            input_revision: command.recipe_revision,
            input_digest: command.recipe_digest,
            config_revision: candidate.basis.config_revision,
            config_digest: candidate.basis.config_digest,
            parent_dispatch: Some(command.candidate_dispatch),
        },
        ExecutionIdentity { asset, attempt },
        command.capacity.clone(),
        true,
        command.now_ms,
    )
    .map_err(|_| ExecutionError::InvalidRecord)?;
    let stage = match super::enqueue::enqueue_reconciliation_stage_in(tx, &enqueue, parent).await? {
        EnqueueOutcome::Enqueued(stage) | EnqueueOutcome::Existing(stage) => stage,
    };
    let row = tx
        .query_one(
            "SELECT claim_token,claim_generation,lease_until_ms,capacity_reserved,stage_state, \
        reconciliation_dispatch,reconciliation_token,reconciliation_generation \
        FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2 FOR UPDATE",
            &[
                &job.as_bytes().as_slice(),
                &stage.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?;
    let old_claim = invalid(df_imagery::execution::ClaimToken::from_bytes(
        &row.get::<_, Vec<u8>>("claim_token"),
    ))?;
    let old_generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
    let lease_until: i64 = row.get("lease_until_ms");
    let same_authority = row
        .get::<_, Option<Vec<u8>>>("reconciliation_dispatch")
        .as_deref()
        == Some(parent.stage.current.dispatch.as_bytes().as_slice())
        && row
            .get::<_, Option<Vec<u8>>>("reconciliation_token")
            .as_deref()
            == Some(parent.token.as_bytes().as_slice())
        && row.get::<_, Option<i64>>("reconciliation_generation")
            == Some(invalid(i64::try_from(parent.generation))?);
    if lease_until > command.now_ms && old_claim != command.claim && !same_authority {
        return Err(ExecutionError::LostClaim);
    }
    let current=tx.query_opt("SELECT current_dispatch FROM imagery.image_stage_cursor WHERE job_id=$1 AND stage_id=$2 FOR UPDATE",
        &[&job.as_bytes().as_slice(),&stage.stage.as_bytes().as_slice()]).await.map_err(map_db)?;
    let current = if let Some(row) = current {
        let id = invalid(df_imagery::execution::DispatchId::from_bytes(
            &row.get::<_, Vec<u8>>("current_dispatch"),
        ))?;
        Some(read_dispatch_view(tx, owner, &id).await?)
    } else {
        None
    };
    if let Some(current) = current
        .as_ref()
        .filter(|current| current.state == DispatchState::Succeeded)
    {
        return Ok(ReconciliationDispatch {
            stage,
            claim: StageClaim {
                scope: StageScope {
                    owner: owner.clone(),
                    job: *job,
                    stage: current.stage,
                },
                token: old_claim,
                generation: old_generation,
            },
            current: current.clone(),
        });
    }
    if (old_claim == command.claim || same_authority)
        && lease_until > command.now_ms
        && current.is_some()
        && row.get::<_, bool>("capacity_reserved")
    {
        let current = current.ok_or(ExecutionError::InvalidRecord)?;
        return Ok(ReconciliationDispatch {
            stage,
            claim: StageClaim {
                scope: StageScope {
                    owner: owner.clone(),
                    job: *job,
                    stage: current.stage,
                },
                token: old_claim,
                generation: old_generation,
            },
            current,
        });
    }
    let generation = if old_claim == command.claim && lease_until > command.now_ms {
        old_generation
    } else {
        old_generation
            .checked_add(1)
            .ok_or(ExecutionError::InvalidRecord)?
    };
    if !row.get::<_, bool>("capacity_reserved") {
        if row.get::<_, String>("stage_state") == "planned" {
            move_pending_to_active_in(tx, command.capacity.as_str(), job_bytes, command.now_ms)
                .await?;
        } else {
            reserve_active_in(tx, command.capacity.as_str(), command.now_ms).await?;
        }
    }
    tx.execute("UPDATE imagery.image_execution_stage SET claim_token=$3,claim_generation=$4,lease_until_ms=$5, \
        claim_lease_ms=$6,stage_state='admitted',capacity_reserved=true,updated_at_ms=$7, \
        reconciliation_dispatch=$8,reconciliation_token=$9,reconciliation_generation=$10 \
        WHERE job_id=$1 AND stage_id=$2",
        &[&job.as_bytes().as_slice(),&stage.stage.as_bytes().as_slice(),&command.claim.as_bytes().as_slice(),
        &invalid(i64::try_from(generation))?,&parent_lease_until,&(parent_lease_until-command.now_ms),&command.now_ms,
        &parent.stage.current.dispatch.as_bytes().as_slice(),&parent.token.as_bytes().as_slice(),
        &invalid(i64::try_from(parent.generation))?]).await.map_err(map_db)?;
    let id = if let Some(current) = current.as_ref().filter(|current| {
        matches!(
            current.state,
            DispatchState::Prepared | DispatchState::Sent | DispatchState::Unknown
        )
    }) {
        current.dispatch
    } else {
        let sequence = if let Some(current) = &current {
            current
                .sequence
                .checked_add(1)
                .ok_or(ExecutionError::InvalidRecord)?
        } else {
            0
        };
        let parent = current
            .as_ref()
            .map(|current| current.dispatch.as_bytes().to_vec());
        let changed=tx.execute("INSERT INTO imagery.image_dispatch_attempt(dispatch_id,job_id,stage_id,dispatch_seq, \
            dispatch_state,settlement,usage_revision,book_call_id,lineage_parent,claim_token,claim_generation, \
            local_accounting,created_at_ms,updated_at_ms,claim_lease_until_ms) VALUES($1,$2,$3,$4,'prepared','held',0,$5,$6,$7,$8,true,$9,$9,$10) \
            ON CONFLICT DO NOTHING",&[&command.dispatch.as_bytes().as_slice(),&job.as_bytes().as_slice(),
            &stage.stage.as_bytes().as_slice(),&invalid(i32::try_from(sequence))?,
            &df_imagery::execution::book_call_id_for(&command.dispatch),&parent,&command.claim.as_bytes().as_slice(),
            &invalid(i64::try_from(generation))?,&command.now_ms,&parent_lease_until]).await.map_err(map_db)?;
        if changed != 1 {
            return Err(ExecutionError::IdentityConflict);
        }
        tx.execute("INSERT INTO imagery.image_stage_cursor(job_id,stage_id,current_dispatch,updated_at_ms) VALUES($1,$2,$3,$4) \
            ON CONFLICT(job_id,stage_id) DO UPDATE SET current_dispatch=EXCLUDED.current_dispatch,updated_at_ms=EXCLUDED.updated_at_ms",
            &[&job.as_bytes().as_slice(),&stage.stage.as_bytes().as_slice(),&command.dispatch.as_bytes().as_slice(),&command.now_ms])
            .await.map_err(map_db)?;
        command.dispatch
    };
    bump_job_revision(tx, job, command.now_ms).await?;
    let stage = super::reads::read_stage_in(tx, owner, job, &stage.stage).await?;
    Ok(ReconciliationDispatch {
        claim: StageClaim {
            scope: StageScope {
                owner: owner.clone(),
                job: *job,
                stage: stage.stage,
            },
            token: command.claim,
            generation,
        },
        stage,
        current: read_dispatch_view(tx, owner, &id).await?,
    })
}

async fn read_reconciliation_candidate(
    tx: &Transaction<'_>,
    authority: &RecoveryLease,
    command: &ReconciliationCommand,
) -> Result<df_imagery::execution::CandidateReceipt, ExecutionError> {
    let owner = &authority.stage.owner;
    let job = &authority.stage.job;
    let current = super::reads::read_dispatch_in(tx, owner, &command.candidate_dispatch).await?;
    if current.job != *job || current.state != DispatchState::Succeeded {
        return Err(ExecutionError::BasisConflict);
    }
    let source = super::reads::read_current_dispatch_in(tx, owner, job, &current.stage).await?;
    if source.as_ref().map(|dispatch| dispatch.dispatch) != Some(current.dispatch) {
        return Err(ExecutionError::Superseded);
    }
    let candidate = super::candidate::read_candidate_checkpoint_in(
        tx,
        owner,
        job,
        &current.stage,
        &current.dispatch,
    )
    .await?
    .ok_or(ExecutionError::ObservationMissing)?;
    if authority.stage.current.dispatch != current.dispatch {
        let actor = super::reads::read_current_dispatch_in(tx, owner, job, &authority.stage.stage)
            .await?
            .ok_or(ExecutionError::Superseded)?;
        let basis = super::decision::read_stage_basis_in(tx, owner, job, &actor.stage).await?;
        if actor.dispatch != authority.stage.current.dispatch
            || actor.state != DispatchState::Succeeded
            || basis.kind != StageKind::Inspection
            || basis.parent_dispatch != Some(current.dispatch)
        {
            return Err(ExecutionError::BasisConflict);
        }
        super::decision::read_decision_reference_in(tx, owner, job, &actor.stage, &actor.dispatch)
            .await?
            .ok_or(ExecutionError::ObservationMissing)?;
    }
    Ok(candidate)
}
