//! Read-only bounded discovery of exact current Prepared attempts and sealed unused-release retries.
use super::common::{invalid, map_db, parse_dispatch, parse_owner, require_read_committed};
use df_imagery::execution::{
    ClaimToken, DispatchId, ExecutionError, ExecutionScope, StageClaim, StageScope, UnsentPrepared,
};
use tokio_postgres::Transaction;
const MAXIMUM_SCAN_LIMIT: u32 = 128;
pub(crate) async fn scan_unsent_prepared_in(
    tx: &Transaction<'_>,
    scope: &str,
    after: Option<DispatchId>,
    limit: u32,
    now_ms: i64,
) -> Result<Vec<UnsentPrepared>, ExecutionError> {
    require_read_committed(tx).await?;
    if scope.is_empty()
        || scope.len() > 128
        || limit == 0
        || limit > MAXIMUM_SCAN_LIMIT
        || now_ms <= 0
    {
        return Err(ExecutionError::InvalidRecord);
    }
    let has_seals: bool = tx
        .query_one(
            "SELECT to_regclass('imagery.image_no_delivery') IS NOT NULL",
            &[],
        )
        .await
        .map_err(map_db)?
        .get(0);
    let (join, sealed) = if has_seals {
        (
            "LEFT JOIN imagery.image_no_delivery z ON z.owner_scope=s.owner_scope AND z.job_id=s.job_id AND z.tail_stage_id=s.stage_id AND z.tail_dispatch_id=d.dispatch_id",
            "z.job_id IS NOT NULL",
        )
    } else {
        ("", "false")
    };
    let terminal = "(j.state='retired' OR j.absolute_deadline_ms<=$4 OR NOT EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.job_id=j.job_id AND i.state='attached'))";
    let query = format!(
        "SELECT d.*,s.owner_scope,j.revision AS current_job_revision,{terminal} AS terminal_condition,{sealed} AS sealed \
        FROM imagery.image_execution_stage s JOIN imagery.image_stage_cursor c USING(job_id,stage_id) \
        JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=c.current_dispatch \
        JOIN imagery.image_job j ON j.job_id=s.job_id {join} \
        WHERE s.capacity_scope=$1 AND ($2::bytea IS NULL OR d.dispatch_id>$2) \
        AND ((d.dispatch_state='prepared' AND s.stage_state='admitted' AND s.claim_token=d.claim_token AND s.claim_generation=d.claim_generation \
            AND (s.lease_until_ms<=$4 OR {terminal})) \
            OR (d.dispatch_state='released' AND (d.settlement='held' OR (d.settlement='released' AND NOT d.cost_unsent_ack)) AND {sealed})) \
        ORDER BY d.dispatch_id LIMIT $3"
    );
    let after = after.map(|id| id.as_bytes().to_vec());
    let rows = tx
        .query(&query, &[&scope, &after, &i64::from(limit), &now_ms])
        .await
        .map_err(map_db)?;
    rows.iter()
        .map(|row| {
            let owner = parse_owner(&row.get::<_, Vec<u8>>("owner_scope"))?;
            let current = parse_dispatch(row, &owner)?;
            let scope = ExecutionScope {
                owner: owner.clone(),
                job: current.job,
                stage: current.stage,
                dispatch: current.dispatch,
            };
            Ok(UnsentPrepared {
                claim: StageClaim {
                    scope: StageScope {
                        owner,
                        job: current.job,
                        stage: current.stage,
                    },
                    token: invalid(ClaimToken::from_bytes(
                        &row.get::<_, Vec<u8>>("claim_token"),
                    ))?,
                    generation: current.claim_generation,
                },
                scope,
                current,
                job_revision: invalid(u64::try_from(row.get::<_, i64>("current_job_revision")))?,
                original_lease_until_ms: row.get("claim_lease_until_ms"),
                terminal_condition: row.get("terminal_condition"),
                sealed: row.get("sealed"),
            })
        })
        .collect()
}
