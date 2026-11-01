//! Bounded queue keyset pages preserve priority/age order without repeatedly scanning a denied prefix.
use super::common::{
    invalid, map_db, parse_owner, require_read_committed, stage_kind_of, stage_state_of,
};
use df_imagery::ImageJobId;
use df_imagery::execution::{
    ExecutionError, ExecutionStageId, QueueCursor, QueuePage, QueuedStage,
};
use tokio_postgres::Transaction;
const MAXIMUM_QUEUE_PAGE_SIZE: u32 = 128;
pub(crate) async fn scan_queue_after_in(
    tx: &Transaction<'_>,
    scope: &str,
    after: Option<QueueCursor>,
    limit: u32,
    now_ms: i64,
) -> Result<QueuePage, ExecutionError> {
    require_read_committed(tx).await?;
    if scope.is_empty()
        || scope.len() > 128
        || limit == 0
        || limit > MAXIMUM_QUEUE_PAGE_SIZE
        || now_ms <= 0
    {
        return Err(ExecutionError::InvalidRecord);
    }
    if after.is_some_and(|c| {
        c.ranking_at_ms <= 0
            || c.ranking_at_ms > now_ms
            || c.effective_urgency > 2
            || c.created_at_ms < 0
            || c.deadline_ms <= 0
    }) {
        return Err(ExecutionError::InvalidRecord);
    }
    let ranking_at_ms = after.map_or(now_ms, |c| c.ranking_at_ms);
    let rank = after.map(|c| i64::from(c.effective_urgency));
    let created = after.map(|c| c.created_at_ms);
    let deadline = after.map(|c| c.deadline_ms);
    let stage = after.map(|c| c.stage.as_bytes().to_vec());
    let job = after.map(|c| c.job.as_bytes().to_vec());
    let rows=tx.query("WITH eligible AS (SELECT s.job_id,s.stage_id,s.owner_scope,s.stage_kind,s.stage_state, \
        s.claim_generation,s.lease_until_ms,s.created_at_ms,j.absolute_deadline_ms, \
        GREATEST(0::bigint,schedule.urgency-LEAST(2::bigint,GREATEST(0::bigint,$4::bigint-s.created_at_ms)/policy.urgency_promotion_ms)) AS effective_urgency \
        FROM imagery.image_execution_stage s JOIN imagery.image_job j ON j.job_id=s.job_id \
        JOIN imagery.image_execution_capacity policy ON policy.capacity_scope=s.capacity_scope \
        JOIN LATERAL(SELECT MIN(COALESCE(h.urgency,1))::bigint AS urgency FROM imagery.image_interest i \
            LEFT JOIN imagery.image_interest_schedule h ON h.interest_id=i.interest_id WHERE i.job_id=s.job_id AND i.state='attached') schedule ON true \
        LEFT JOIN imagery.image_stage_cursor c ON c.job_id=s.job_id AND c.stage_id=s.stage_id \
        WHERE s.capacity_scope=$1 AND s.stage_state IN ('planned','admitted') AND j.state<>'retired' AND j.absolute_deadline_ms>$3 \
        AND s.lease_until_ms<=$3 AND s.queued_not_before_ms<=$3 AND (c.current_dispatch IS NULL OR (s.retry_dispatch_id=c.current_dispatch AND EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt retry JOIN imagery.image_decline_checkpoint proof ON proof.dispatch_id=retry.dispatch_id WHERE retry.dispatch_id=c.current_dispatch AND retry.dispatch_state='declined'))) \
        AND EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.job_id=s.job_id AND i.state='attached')) \
        SELECT * FROM eligible WHERE ($5::bigint IS NULL OR (effective_urgency,created_at_ms,absolute_deadline_ms,stage_id,job_id)> \
            ($5::bigint,$6::bigint,$7::bigint,$8::bytea,$9::bytea)) \
        ORDER BY effective_urgency,created_at_ms,absolute_deadline_ms,stage_id,job_id LIMIT $2",
        &[&scope,&i64::from(limit),&now_ms,&ranking_at_ms,&rank,&created,&deadline,&stage,&job]).await.map_err(map_db)?;
    let mut stages = Vec::with_capacity(rows.len());
    let mut last = None;
    for row in &rows {
        let job = invalid(ImageJobId::from_bytes(&row.get::<_, Vec<u8>>("job_id")))?;
        let stage = invalid(ExecutionStageId::from_bytes(
            &row.get::<_, Vec<u8>>("stage_id"),
        ))?;
        stages.push(QueuedStage {
            owner: parse_owner(&row.get::<_, Vec<u8>>("owner_scope"))?,
            job,
            stage,
            kind: stage_kind_of(&row.get::<_, String>("stage_kind"))?,
            state: stage_state_of(&row.get::<_, String>("stage_state"))?,
            claim_generation: invalid(u64::try_from(row.get::<_, i64>("claim_generation")))?,
            lease_until_ms: row.get("lease_until_ms"),
            created_at_ms: row.get("created_at_ms"),
        });
        last = Some(QueueCursor {
            ranking_at_ms,
            effective_urgency: invalid(u8::try_from(row.get::<_, i64>("effective_urgency")))?,
            created_at_ms: row.get("created_at_ms"),
            deadline_ms: row.get("absolute_deadline_ms"),
            stage,
            job,
        });
    }
    Ok(QueuePage {
        next: if rows.len() == usize::try_from(limit).map_err(|_| ExecutionError::InvalidRecord)? {
            last
        } else {
            None
        },
        stages,
    })
}
