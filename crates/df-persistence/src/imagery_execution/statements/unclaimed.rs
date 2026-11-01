//! Bounded actual terminal no-dispatch discovery; every write rechecks the finite stored state.
use super::common::{invalid, map_db, parse_owner, require_read_committed};
use df_imagery::execution::{
    ClaimToken, ClaimedTerminal, DeferredTerminal, DispatchId, ExecutionError, ExecutionStageId,
    ReleasedTerminal, StageClaim, StageScope, TerminalCursor, UnclaimedTerminal, UnsentTerminal,
};
use df_imagery::{ImageJobId, owner_scope_bytes};
use tokio_postgres::{Row, Transaction};
const MAXIMUM_SCAN_LIMIT: u32 = 128;
fn parse(row: &Row) -> Result<UnsentTerminal, ExecutionError> {
    let scope = StageScope {
        owner: parse_owner(&row.get::<_, Vec<u8>>("owner_scope"))?,
        job: invalid(ImageJobId::from_bytes(&row.get::<_, Vec<u8>>("job_id")))?,
        stage: invalid(ExecutionStageId::from_bytes(
            &row.get::<_, Vec<u8>>("stage_id"),
        ))?,
    };
    let job_revision = invalid(u64::try_from(row.get::<_, i64>("current_job_revision")))?;
    let generation = invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?;
    match row.get::<_, String>("stage_state").as_str() {
        "planned" if generation == 0 => Ok(UnsentTerminal::Unclaimed(UnclaimedTerminal {
            scope,
            job_revision,
        })),
        "planned" => Ok(UnsentTerminal::Deferred(DeferredTerminal {
            scope,
            job_revision,
            claim_generation: generation,
            not_before_ms: row.get("queued_not_before_ms"),
        })),
        "admitted" => Ok(UnsentTerminal::Claimed(ClaimedTerminal {
            claim: StageClaim {
                scope,
                token: invalid(ClaimToken::from_bytes(
                    &row.get::<_, Vec<u8>>("claim_token"),
                ))?,
                generation,
            },
            job_revision,
        })),
        "released" => Ok(UnsentTerminal::Released(ReleasedTerminal {
            scope,
            job_revision,
            claim_generation: generation,
            candidate_dispatch: invalid(DispatchId::from_bytes(
                &row.get::<_, Vec<u8>>("parent_dispatch"),
            ))?,
        })),
        _ => Err(ExecutionError::InvalidRecord),
    }
}
async fn no_final_guard(tx: &Transaction<'_>) -> Result<String, ExecutionError> {
    let tables=tx.query_one("SELECT to_regclass('imagery.image_job_result') IS NOT NULL AS ready,to_regclass('imagery.image_analysis_checkpoint') IS NOT NULL AS analysis",&[]).await.map_err(map_db)?;
    let mut guard = String::new();
    if tables.get::<_, bool>("ready") {
        guard.push_str(" AND NOT EXISTS(SELECT 1 FROM imagery.image_job_result f WHERE f.job_id=s.job_id AND f.owner_scope=s.owner_scope)");
    }
    if tables.get::<_, bool>("analysis") {
        guard.push_str(" AND NOT EXISTS(SELECT 1 FROM imagery.image_analysis_checkpoint f WHERE f.job_id=s.job_id AND f.owner_scope=s.owner_scope)");
    }
    Ok(guard)
}
const RELEASED_PARENT: &str = "EXISTS(SELECT 1 FROM imagery.image_publication_allocation a JOIN media.asset m ON m.asset_id=a.asset_id WHERE a.job_id=s.job_id AND a.owner_scope=s.owner_scope AND m.state IN ('uploading','quarantined','validated')) AND EXISTS(SELECT 1 FROM imagery.image_candidate_checkpoint p JOIN imagery.image_candidate_observation o USING(dispatch_id,observation_version) JOIN imagery.image_stage_cursor pc ON pc.current_dispatch=o.dispatch_id AND pc.job_id=o.job_id AND pc.stage_id=o.stage_id JOIN imagery.image_dispatch_attempt pd ON pd.dispatch_id=o.dispatch_id WHERE o.job_id=s.job_id AND o.owner_scope=s.owner_scope AND o.dispatch_id=s.parent_dispatch AND pd.dispatch_state='succeeded' AND p.usage_revision=o.usage_revision AND pd.usage_revision=o.usage_revision)";
fn finite_states() -> String {
    format!(
        "((s.stage_state='planned' AND NOT s.capacity_reserved AND s.lease_until_ms=0 AND s.claim_token=s.stage_id AND s.claim_config_revision=0 AND s.claim_lease_ms=0 AND (s.claim_generation=0 OR (s.claim_generation>0 AND s.queued_not_before_ms>0))) OR (s.stage_state='admitted' AND s.capacity_reserved AND s.claim_generation>0) OR (s.stage_state='released' AND NOT s.capacity_reserved AND s.lease_until_ms=0 AND {RELEASED_PARENT}))"
    )
}
async fn absence_guards(tx: &Transaction<'_>) -> Result<String, ExecutionError> {
    let usage: bool = tx
        .query_one(
            "SELECT to_regclass('imagery.image_dispatch_usage') IS NOT NULL",
            &[],
        )
        .await
        .map_err(map_db)?
        .get(0);
    let mut guard =
        String::from(" AND imagery.stage_has_finance(s.owner_scope,s.job_id,s.stage_id) IS FALSE");
    if usage {
        guard.push_str(" AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_usage u WHERE u.job_id=s.job_id AND u.stage_id=s.stage_id)");
    }
    Ok(guard)
}
pub(crate) async fn scan_unsent_terminal_in(
    tx: &Transaction<'_>,
    scope: &str,
    after: Option<TerminalCursor>,
    limit: u32,
    now_ms: i64,
) -> Result<Vec<UnsentTerminal>, ExecutionError> {
    require_read_committed(tx).await?;
    if scope.is_empty()
        || scope.len() > 128
        || limit == 0
        || limit > MAXIMUM_SCAN_LIMIT
        || now_ms <= 0
    {
        return Err(ExecutionError::InvalidRecord);
    }
    let guards = absence_guards(tx).await?;
    let final_guard = no_final_guard(tx).await?;
    let states = finite_states();
    let query = format!(
        "SELECT s.*,j.revision AS current_job_revision FROM imagery.image_execution_stage s JOIN imagery.image_job j ON j.job_id=s.job_id LEFT JOIN imagery.image_stage_cursor c ON c.job_id=s.job_id AND c.stage_id=s.stage_id WHERE s.capacity_scope=$1 AND ($2::bytea IS NULL OR (s.stage_id,s.job_id)>($2::bytea,$3::bytea)) AND (j.state='retired' OR j.absolute_deadline_ms<=$5 OR NOT EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.job_id=s.job_id AND i.state='attached')) AND {states} AND c.current_dispatch IS NULL AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt d WHERE d.job_id=s.job_id AND d.stage_id=s.stage_id) {guards} {final_guard} ORDER BY s.stage_id,s.job_id LIMIT $4"
    );
    let stage = after.map(|c| c.stage.as_bytes().to_vec());
    let job = after.map(|c| c.job.as_bytes().to_vec());
    let rows = tx
        .query(&query, &[&scope, &stage, &job, &i64::from(limit), &now_ms])
        .await
        .map_err(map_db)?;
    rows.iter().map(parse).collect()
}
/// The caller holds the owned job lock and already compared the exact job revision.
pub(super) async fn validate_unowned_in(
    tx: &Transaction<'_>,
    expected: &UnsentTerminal,
    lapsed: bool,
) -> Result<(), ExecutionError> {
    if !lapsed {
        return Err(ExecutionError::OperationConflict);
    }
    let scope = expected.scope();
    let guards = absence_guards(tx).await?;
    let final_guard = no_final_guard(tx).await?;
    let states = finite_states();
    let query = format!(
        "SELECT s.*,j.revision AS current_job_revision FROM imagery.image_execution_stage s JOIN imagery.image_job j ON j.job_id=s.job_id LEFT JOIN imagery.image_stage_cursor c ON c.job_id=s.job_id AND c.stage_id=s.stage_id WHERE s.owner_scope=$1 AND s.job_id=$2 AND s.stage_id=$3 AND {states} AND c.current_dispatch IS NULL AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt d WHERE d.job_id=s.job_id AND d.stage_id=s.stage_id) {guards} {final_guard} FOR UPDATE OF s"
    );
    let row = tx
        .query_opt(
            &query,
            &[
                &owner_scope_bytes(&scope.owner),
                &scope.job.as_bytes().as_slice(),
                &scope.stage.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(map_db)?
        .ok_or(ExecutionError::Superseded)?;
    let actual = parse(&row)?;
    if &actual != expected {
        return Err(ExecutionError::BasisConflict);
    }
    Ok(())
}
