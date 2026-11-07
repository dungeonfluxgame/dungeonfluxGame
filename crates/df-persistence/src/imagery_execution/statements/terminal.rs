//! Job-locked output-only terminal tail validation; supplier and financial reconciliation remain open.
use super::common::{bytes16, check_owner, invalid, locked_job, map_db, require_read_committed};
use df_imagery::execution::*;
use df_imagery::{ImageJobId, ImageOwner, owner_scope_bytes};
use tokio_postgres::Transaction;

// Bounded metadata history; callers may rebase an older actor to the actual validated current tail.
const MAXIMUM_TERMINAL_ANCESTORS: i64 = 128;

pub(crate) async fn output_is_sealed_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<bool, ExecutionError> {
    let table: Option<String> = tx
        .query_one("SELECT to_regclass('imagery.image_no_delivery')::text", &[])
        .await
        .map_err(map_db)?
        .get(0);
    if table.is_none() {
        return Ok(false);
    }
    Ok(tx
        .query_opt(
            "SELECT 1 FROM imagery.image_no_delivery WHERE owner_scope=$1 AND job_id=$2",
            &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .is_some())
}
pub(crate) async fn check_output_open_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<(), ExecutionError> {
    if output_is_sealed_in(tx, owner, job).await? {
        return Err(ExecutionError::OutputSealed);
    }
    let ready_installed: bool = tx
        .query_one(
            "SELECT to_regclass('imagery.image_job_result') IS NOT NULL",
            &[],
        )
        .await
        .map_err(map_db)?
        .get(0);
    if ready_installed
        && tx
            .query_opt(
                "SELECT 1 FROM imagery.image_job_result WHERE owner_scope=$1 AND job_id=$2",
                &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()],
            )
            .await
            .map_err(map_db)?
            .is_some()
    {
        return Err(ExecutionError::OutputSealed);
    }
    // Successful standalone analysis closes future work without inventing a NoDelivery seal.
    let installed: bool = tx
        .query_one(
            "SELECT to_regclass('imagery.image_analysis_checkpoint') IS NOT NULL",
            &[],
        )
        .await
        .map_err(map_db)?
        .get(0);
    if installed && tx.query_opt(
        "SELECT 1 FROM imagery.image_analysis_checkpoint WHERE owner_scope=$1 AND job_id=$2",
        &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()]
    ).await.map_err(map_db)?.is_some() {
        return Err(ExecutionError::OutputSealed);
    }
    Ok(())
}
async fn no_unsent_history(tx: &Transaction<'_>, scope: &StageScope) -> Result<(), ExecutionError> {
    if tx
        .query_opt(
            "SELECT 1 FROM imagery.image_dispatch_attempt WHERE job_id=$1 AND stage_id=$2 LIMIT 1",
            &[
                &scope.job.as_bytes().as_slice(),
                &scope.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .is_some()
        || super::deferral::has_stage_finance(tx, &scope.owner, &scope.job, &scope.stage).await?
    {
        return Err(ExecutionError::Superseded);
    }
    Ok(())
}
async fn snapshot(
    tx: &Transaction<'_>,
    actor: &TerminalActor,
    current: bool,
) -> Result<TerminalStageSnapshot, ExecutionError> {
    let scope = actor.scope();
    let stage = super::reads::read_stage_in(tx, &scope.owner, &scope.job, &scope.stage).await?;
    let basis =
        super::decision::read_stage_basis_in(tx, &scope.owner, &scope.job, &scope.stage).await?;
    let dispatch = match actor {
        TerminalActor::Dispatch(expected) => {
            let actual =
                super::reads::read_dispatch_in(tx, &scope.owner, &expected.dispatch).await?;
            if actual.job != scope.job || actual.stage != scope.stage {
                return Err(ExecutionError::NotFound);
            }
            if current
                && super::reads::read_current_dispatch_in(
                    tx,
                    &scope.owner,
                    &scope.job,
                    &scope.stage,
                )
                .await?
                .map(|dispatch| dispatch.dispatch)
                    != Some(expected.dispatch)
            {
                return Err(ExecutionError::Superseded);
            }
            Some(actual)
        }
        TerminalActor::UnsentStage(_)
        | TerminalActor::UnclaimedStage(_)
        | TerminalActor::DeferredStage(_)
        | TerminalActor::ReleasedStage(_) => {
            no_unsent_history(tx, &scope).await?;
            None
        }
    };
    Ok(TerminalStageSnapshot {
        stage,
        basis,
        dispatch,
    })
}
async fn unsent_claim(
    tx: &Transaction<'_>,
    claim: &StageClaim,
    lapsed: bool,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let s = &claim.scope;
    let row=tx.query_one("SELECT claim_token,claim_generation,lease_until_ms,stage_state FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2",
        &[&s.job.as_bytes().as_slice(),&s.stage.as_bytes().as_slice()]).await.map_err(map_db)?;
    let matches = row.get::<_, Vec<u8>>("claim_token") == claim.token.as_bytes()
        && row.get::<_, i64>("claim_generation") == invalid(i64::try_from(claim.generation))?;
    let state: String = row.get("stage_state");
    if claim.generation > 0
        && matches
        && state == "admitted"
        && (row.get::<_, i64>("lease_until_ms") > now_ms || lapsed)
    {
        return Ok(());
    }
    if state == "released" {
        let recorded=tx.query_opt("SELECT 1 FROM imagery.image_local_inspection WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND claim_token=$4 AND claim_generation=$5",
            &[&owner_scope_bytes(&s.owner),&s.job.as_bytes().as_slice(),&s.stage.as_bytes().as_slice(),
            &claim.token.as_bytes().as_slice(),&invalid(i64::try_from(claim.generation))?]).await.map_err(map_db)?.is_some();
        if recorded {
            return Ok(());
        }
    }
    Err(ExecutionError::LostClaim)
}
async fn authority(
    tx: &Transaction<'_>,
    command: &TerminalCommand,
    tail: &TerminalStageSnapshot,
    lapsed: bool,
) -> Result<(), ExecutionError> {
    let scope = command.tail.scope();
    match &command.authority {
        TerminalAuthority::Unclaimed(proof) => {
            if command.actor != command.tail
                || !matches!(&command.tail, TerminalActor::UnclaimedStage(s) if *s==proof.scope)
                || proof.job_revision != command.expected_job_revision
            {
                return Err(ExecutionError::BasisConflict);
            }
            super::unclaimed::validate_unowned_in(
                tx,
                &UnsentTerminal::Unclaimed(proof.clone()),
                lapsed,
            )
            .await?;
        }
        TerminalAuthority::Deferred(proof) => {
            if command.actor != command.tail
                || !matches!(&command.tail, TerminalActor::DeferredStage(s) if *s==proof.scope)
                || proof.job_revision != command.expected_job_revision
            {
                return Err(ExecutionError::BasisConflict);
            }
            super::unclaimed::validate_unowned_in(
                tx,
                &UnsentTerminal::Deferred(proof.clone()),
                lapsed,
            )
            .await?;
        }
        TerminalAuthority::Released(proof) => {
            if command.actor != command.tail
                || !matches!(&command.tail, TerminalActor::ReleasedStage(s) if *s==proof.scope)
                || proof.job_revision != command.expected_job_revision
                || command.candidate_dispatch != Some(proof.candidate_dispatch)
            {
                return Err(ExecutionError::BasisConflict);
            }
            super::unclaimed::validate_unowned_in(
                tx,
                &UnsentTerminal::Released(proof.clone()),
                lapsed,
            )
            .await?;
        }
        TerminalAuthority::Claim(claim) => {
            if claim.scope != scope {
                return Err(ExecutionError::BasisConflict);
            }
            if let TerminalActor::UnsentStage(expected) = &command.tail {
                if claim != expected {
                    return Err(ExecutionError::BasisConflict);
                }
                unsent_claim(tx, claim, lapsed, command.now_ms).await?;
                if lapsed && tail.stage.state == StageState::Admitted {
                    super::unclaimed::validate_unowned_in(
                        tx,
                        &UnsentTerminal::Claimed(ClaimedTerminal {
                            claim: claim.clone(),
                            job_revision: command.expected_job_revision,
                        }),
                        true,
                    )
                    .await?;
                }
                return Ok(());
            }
            dispatch_claim(tx, claim, tail, lapsed, command.now_ms).await?;
        }
        TerminalAuthority::Recovery(lease) => {
            if lease.stage.owner != scope.owner
                || lease.stage.job != scope.job
                || lease.stage.stage != scope.stage
                || tail.dispatch.as_ref().map(|dispatch| dispatch.dispatch)
                    != Some(lease.stage.current.dispatch)
            {
                return Err(ExecutionError::BasisConflict);
            }
            super::recovery::validate_recovery_in(tx, lease, command.now_ms).await?;
        }
    }
    Ok(())
}
async fn descends_from(
    tx: &Transaction<'_>,
    job: &ImageJobId,
    start: DispatchId,
    target: DispatchId,
) -> Result<bool, ExecutionError> {
    if start == target {
        return Ok(true);
    }
    Ok(tx.query_opt("WITH RECURSIVE ancestry AS ( \
        SELECT d.dispatch_id,coalesce(d.lineage_parent,s.parent_dispatch) AS parent,0::bigint AS depth,ARRAY[d.dispatch_id] AS seen \
        FROM imagery.image_dispatch_attempt d JOIN imagery.image_execution_stage s USING(job_id,stage_id) WHERE d.job_id=$1 AND d.dispatch_id=$2 \
        UNION ALL SELECT p.dispatch_id,coalesce(p.lineage_parent,ps.parent_dispatch),a.depth+1,a.seen||p.dispatch_id \
        FROM ancestry a JOIN imagery.image_dispatch_attempt p ON p.dispatch_id=a.parent AND p.job_id=$1 \
        JOIN imagery.image_execution_stage ps ON ps.job_id=p.job_id AND ps.stage_id=p.stage_id \
        WHERE a.depth<$4 AND NOT p.dispatch_id=ANY(a.seen)) SELECT 1 FROM ancestry WHERE dispatch_id=$3 LIMIT 1",
        &[&job.as_bytes().as_slice(),&start.as_bytes().as_slice(),&target.as_bytes().as_slice(),&MAXIMUM_TERMINAL_ANCESTORS])
        .await.map_err(map_db)?.is_some())
}
async fn check_tail(
    tx: &Transaction<'_>,
    command: &TerminalCommand,
    actor: &TerminalStageSnapshot,
    tail: &TerminalStageSnapshot,
) -> Result<(), ExecutionError> {
    let scope = command.tail.scope();
    if command.actor != command.tail {
        let ancestor = actor
            .dispatch
            .as_ref()
            .map(|dispatch| dispatch.dispatch)
            .ok_or(ExecutionError::Superseded)?;
        let start = tail
            .dispatch
            .as_ref()
            .map(|dispatch| dispatch.dispatch)
            .or(tail.basis.parent_dispatch)
            .ok_or(ExecutionError::Superseded)?;
        if !descends_from(tx, &scope.job, start, ancestor).await? {
            return Err(ExecutionError::Superseded);
        }
    }
    if let Some(dispatch) = &tail.dispatch
        && tx.query_opt("SELECT 1 FROM imagery.image_execution_stage WHERE job_id=$1 AND parent_dispatch=$2 LIMIT 1",
            &[&scope.job.as_bytes().as_slice(),&dispatch.dispatch.as_bytes().as_slice()]).await.map_err(map_db)?.is_some() {
            return Err(ExecutionError::Superseded)
    }
    if let Some(candidate) = command.candidate_dispatch {
        let start = tail
            .dispatch
            .as_ref()
            .map(|dispatch| dispatch.dispatch)
            .or(tail.basis.parent_dispatch)
            .ok_or(ExecutionError::BasisConflict)?;
        if !descends_from(tx, &scope.job, start, candidate).await? {
            return Err(ExecutionError::BasisConflict);
        }
        if tail.basis.kind!=StageKind::LocalTransform && tx.query_opt("SELECT 1 FROM imagery.image_execution_stage WHERE job_id=$1 AND parent_dispatch=$2 AND stage_kind='local_transform' LIMIT 1",
            &[&scope.job.as_bytes().as_slice(),&candidate.as_bytes().as_slice()]).await.map_err(map_db)?.is_some() {return Err(ExecutionError::Superseded)}
    } else if matches!(
        tail.basis.kind,
        StageKind::Inspection | StageKind::Correction | StageKind::Extraction
    ) {
        return Err(ExecutionError::BasisConflict);
    }
    Ok(())
}
async fn allocation(
    tx: &Transaction<'_>,
    scope: &StageScope,
) -> Result<TerminalAllocation, ExecutionError> {
    let rows=tx.query("SELECT owner_scope,stage_id,asset_id,publication_attempt FROM imagery.image_publication_allocation WHERE job_id=$1 LIMIT 2",
        &[&scope.job.as_bytes().as_slice()]).await.map_err(map_db)?;
    match rows.as_slice() {
        [] => Ok(TerminalAllocation::NoAllocation),
        [row] => {
            check_owner(&row.get::<_, Vec<u8>>("owner_scope"), &scope.owner)?;
            Ok(TerminalAllocation::Existing {
                stage: invalid(ExecutionStageId::from_bytes(
                    &row.get::<_, Vec<u8>>("stage_id"),
                ))?,
                asset: bytes16(&row.get::<_, Vec<u8>>("asset_id"))?,
                attempt: bytes16(&row.get::<_, Vec<u8>>("publication_attempt"))?,
            })
        }
        _ => Err(ExecutionError::InvalidRecord),
    }
}
pub(crate) async fn validate_terminal_tail_in(
    tx: &Transaction<'_>,
    command: &TerminalCommand,
) -> Result<TerminalValidation, ExecutionError> {
    require_read_committed(tx).await?;
    if command.now_ms <= 0 || command.expected_job_revision == 0 {
        return Err(ExecutionError::InvalidRecord);
    }
    let scope = command.tail.scope();
    let original = command.actor.scope();
    if scope.owner != original.owner || scope.job != original.job {
        return Err(ExecutionError::BasisConflict);
    }
    let (_, revision, deadline, state, _, _, _) = locked_job(tx, &scope.owner, &scope.job).await?;
    if revision != command.expected_job_revision {
        return Err(ExecutionError::Stale {
            current_revision: revision,
        });
    }
    let lapsed = state == "retired"
        || deadline <= command.now_ms
        || super::common::attached_interest_count(tx, &scope.job).await? == 0;
    let actor = snapshot(tx, &command.actor, false).await?;
    let tail = snapshot(tx, &command.tail, true).await?;
    authority(tx, command, &tail, lapsed).await?;
    check_tail(tx, command, &actor, &tail).await?;
    let candidate = if let Some(dispatch) = command.candidate_dispatch {
        let view = super::reads::read_dispatch_in(tx, &scope.owner, &dispatch).await?;
        if view.job != scope.job {
            return Err(ExecutionError::NotFound);
        }
        Some(
            super::candidate::read_candidate_reference_in(
                tx,
                &scope.owner,
                &scope.job,
                &view.stage,
                &dispatch,
            )
            .await?
            .ok_or(ExecutionError::ObservationMissing)?,
        )
    } else {
        None
    };
    let allocation = allocation(tx, &scope).await?;
    Ok(TerminalValidation {
        actor,
        tail,
        candidate,
        allocation,
        job_revision: revision,
    })
}
fn identity(actor: &TerminalActor) -> (&'static str, Option<Vec<u8>>) {
    match actor {
        TerminalActor::Dispatch(scope) => ("dispatch", Some(scope.dispatch.as_bytes().to_vec())),
        TerminalActor::UnsentStage(_) => ("unsent_stage", None),
        TerminalActor::UnclaimedStage(_) => ("unclaimed_stage", None),
        TerminalActor::DeferredStage(_) => ("deferred_stage", None),
        TerminalActor::ReleasedStage(_) => ("released_stage", None),
    }
}
pub(crate) async fn close_terminal_in(
    tx: &Transaction<'_>,
    command: &TerminalCommand,
) -> Result<u64, ExecutionError> {
    require_read_committed(tx).await?;
    let s = command.tail.scope();
    let a = command.actor.scope();
    let (ak, ad) = identity(&command.actor);
    let (tk, td) = identity(&command.tail);
    let row=tx.query_one("SELECT verdict,job_revision FROM imagery.close_terminal_output($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        &[&owner_scope_bytes(&s.owner),&s.job.as_bytes().as_slice(),&ak,&a.stage.as_bytes().as_slice(),&ad,
        &tk,&s.stage.as_bytes().as_slice(),&td,&command.candidate_dispatch.map(|dispatch|dispatch.as_bytes().to_vec()),
        &invalid(i64::try_from(command.expected_job_revision))?,&command.now_ms]).await.map_err(map_db)?;
    let revision = invalid(u64::try_from(row.get::<_, i64>("job_revision")))?;
    match row.get::<_, String>("verdict").as_str() {
        "closed" | "existing" => Ok(revision),
        "not_found" => Err(ExecutionError::NotFound),
        "stale" => Err(ExecutionError::Stale {
            current_revision: revision,
        }),
        "paid" => Err(ExecutionError::Superseded),
        "invalid" => Err(ExecutionError::InvalidRecord),
        _ => Err(ExecutionError::BasisConflict),
    }
}

pub(super) async fn sealed_dispatch_in(
    tx: &Transaction<'_>,
    stage: &RecoveryStage,
) -> Result<bool, ExecutionError> {
    if !output_is_sealed_in(tx, &stage.owner, &stage.job).await? {
        return Ok(false);
    }
    Ok(tx.query_opt("SELECT 1 FROM imagery.image_no_delivery WHERE owner_scope=$1 AND job_id=$2 \
        AND ((actor_stage_id=$3 AND actor_dispatch_id=$4) OR (tail_stage_id=$3 AND tail_dispatch_id=$4) \
            OR (candidate_stage_id=$3 AND candidate_dispatch_id=$4))",
        &[&owner_scope_bytes(&stage.owner),&stage.job.as_bytes().as_slice(),&stage.stage.as_bytes().as_slice(),
            &stage.current.dispatch.as_bytes().as_slice()]).await.map_err(map_db)?.is_some())
}

async fn dispatch_claim(
    tx: &Transaction<'_>,
    claim: &StageClaim,
    tail: &TerminalStageSnapshot,
    lapsed: bool,
    now_ms: i64,
) -> Result<(), ExecutionError> {
    let s = &claim.scope;
    let d = tail
        .dispatch
        .as_ref()
        .ok_or(ExecutionError::BasisConflict)?;
    let row=tx.query_one("SELECT s.claim_token AS current_token,s.claim_generation AS current_generation,s.lease_until_ms,s.stage_state,s.retry_dispatch_id, \
        d.claim_token AS original_token,d.claim_generation AS original_generation,d.claim_lease_until_ms \
        FROM imagery.image_execution_stage s JOIN imagery.image_dispatch_attempt d USING(job_id,stage_id) \
        WHERE s.job_id=$1 AND s.stage_id=$2 AND d.dispatch_id=$3",
        &[&s.job.as_bytes().as_slice(),&s.stage.as_bytes().as_slice(),&d.dispatch.as_bytes().as_slice()]).await.map_err(map_db)?;
    let generation = invalid(i64::try_from(claim.generation))?;
    let current = row.get::<_, Vec<u8>>("current_token") == claim.token.as_bytes();
    let current_generation: i64 = row.get("current_generation");
    let original = row.get::<_, Vec<u8>>("original_token") == claim.token.as_bytes()
        && row.get::<_, i64>("original_generation") == generation
        && generation > 0;
    if d.state == DispatchState::Declined
        && current
        && current_generation == generation
        && generation > 0
        && row.get::<_, String>("stage_state") == "admitted"
        && row
            .get::<_, Option<Vec<u8>>>("retry_dispatch_id")
            .as_deref()
            == Some(d.dispatch.as_bytes())
        && (row.get::<_, i64>("lease_until_ms") > now_ms || lapsed)
    {
        return Ok(());
    }
    if current
        && original
        && current_generation == generation
        && row.get::<_, String>("stage_state") == "admitted"
        && (row.get::<_, i64>("lease_until_ms") > now_ms
            || lapsed && d.state == DispatchState::Prepared)
    {
        return Ok(());
    }
    if d.state == DispatchState::Declined
        && current
        && generation > 0
        && current_generation
            == generation
                .checked_add(1)
                .ok_or(ExecutionError::InvalidRecord)?
        && row.get::<_, String>("stage_state") == "released"
        && row.get::<_, Option<Vec<u8>>>("retry_dispatch_id").is_none()
    {
        let scope = ExecutionScope {
            owner: s.owner.clone(),
            job: s.job,
            stage: s.stage,
            dispatch: d.dispatch,
        };
        if super::decline::read_decline_reference_in(tx, &scope)
            .await?
            .is_some()
        {
            return Ok(());
        }
    }
    if matches!(d.state, DispatchState::Succeeded | DispatchState::Failed)
        && current
        && current_generation
            == generation
                .checked_add(1)
                .ok_or(ExecutionError::InvalidRecord)?
        && row.get::<_, String>("stage_state") == "released"
        && row.get::<_, Vec<u8>>("original_token") == claim.token.as_bytes()
        && row.get::<_, i64>("original_generation") == generation
        && row.get::<_, i64>("claim_lease_until_ms") > now_ms
    {
        return Ok(());
    }
    Err(ExecutionError::LostClaim)
}
