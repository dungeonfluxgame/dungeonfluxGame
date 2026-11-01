//! Immutable scheduling hints belong to accepted interests, never content or operation fingerprints.
use super::common::{locked_job, map_db, require_read_committed};
use df_imagery::execution::ExecutionError;
use df_imagery::{ImageInterestId, ImageJobId, ImageOwner, Urgency, owner_scope_bytes};
use tokio_postgres::Transaction;

/// First hint wins for each accepted interest; visible coalesced demand can raise effective urgency.
pub(crate) async fn record_urgency_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
    interest: &ImageInterestId,
    urgency: Urgency,
    now_ms: i64,
) -> Result<Urgency, ExecutionError> {
    require_read_committed(tx).await?;
    if now_ms <= 0 {
        return Err(ExecutionError::InvalidRecord);
    }
    locked_job(tx, owner, job).await?;
    let exists=tx.query_opt("SELECT 1 FROM imagery.image_interest WHERE interest_id=$1 AND job_id=$2 AND owner_scope=$3",
        &[&interest.as_bytes().as_slice(),&job.as_bytes().as_slice(),&owner_scope_bytes(owner)])
        .await.map_err(map_db)?.is_some();
    if !exists {
        return Err(ExecutionError::NotFound);
    }
    let value: i16 = match urgency {
        Urgency::Visible => 0,
        Urgency::Present => 1,
        Urgency::Speculative => 2,
    };
    tx.execute("INSERT INTO imagery.image_interest_schedule(interest_id,urgency,accepted_at_ms) VALUES($1,$2,$3) \
        ON CONFLICT(interest_id) DO NOTHING",&[&interest.as_bytes().as_slice(),&value,&now_ms]).await.map_err(map_db)?;
    let stored: i16 = tx
        .query_one(
            "SELECT urgency FROM imagery.image_interest_schedule WHERE interest_id=$1",
            &[&interest.as_bytes().as_slice()],
        )
        .await
        .map_err(map_db)?
        .get(0);
    match stored {
        0 => Ok(Urgency::Visible),
        1 => Ok(Urgency::Present),
        2 => Ok(Urgency::Speculative),
        _ => Err(ExecutionError::InvalidRecord),
    }
}
