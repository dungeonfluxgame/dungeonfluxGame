//! The one privileged local Sent gate; source/grant locks belong to its caller's transaction.
use super::common::{invalid, map_db, require_read_committed};
use df_imagery::execution::{ExecutionError, LocalSentOutcome, MarkSentCommand};
use tokio_postgres::Transaction;

pub(crate) async fn mark_local_sent_in(
    tx: &Transaction<'_>,
    command: &MarkSentCommand,
) -> Result<LocalSentOutcome, ExecutionError> {
    require_read_committed(tx).await?;
    let row=tx.query_one("SELECT verdict,current_revision FROM imagery.mark_local_dispatch_sent($1,$2,$3,$4,$5,$6,$7)",
        &[&df_imagery::owner_scope_bytes(&command.owner),&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice(),
        &command.dispatch.as_bytes().as_slice(),&command.claim.as_bytes().as_slice(),
        &invalid(i64::try_from(command.claim_generation))?,&command.now_ms]).await.map_err(map_db)?;
    parse_verdict(&row)
}
fn parse_verdict(row: &tokio_postgres::Row) -> Result<LocalSentOutcome, ExecutionError> {
    match row.get::<_, String>("verdict").as_str() {
        "new" => Ok(LocalSentOutcome::New),
        "already_sent" => Ok(LocalSentOutcome::AlreadySent),
        "not_found" => Err(ExecutionError::NotFound),
        "invalid" => Err(ExecutionError::InvalidRecord),
        "basis" => Err(ExecutionError::BasisConflict),
        "sealed" => Err(ExecutionError::OutputSealed),
        "superseded" => Err(ExecutionError::Superseded),
        "lost_claim" => Err(ExecutionError::LostClaim),
        "retired" => Err(ExecutionError::Retired),
        "expired" => Err(ExecutionError::Expired),
        "disabled" => Err(ExecutionError::Disabled),
        "demand_lapsed" => Err(ExecutionError::DemandLapsed),
        "fenced" => Err(ExecutionError::ConfigFenced {
            current_revision: invalid(u64::try_from(row.get::<_, i64>("current_revision")))?,
        }),
        _ => Err(ExecutionError::InvalidRecord),
    }
}

pub(crate) async fn mark_hosted_sent_in(
    tx: &Transaction<'_>,
    command: &MarkSentCommand,
    finance: &df_imagery::execution::FinanceTuple,
) -> Result<df_imagery::execution::DispatchView, ExecutionError> {
    require_read_committed(tx).await?;
    let row=tx.query_one("SELECT verdict,current_revision FROM imagery.mark_hosted_dispatch_sent($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        &[&df_imagery::owner_scope_bytes(&command.owner),&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice(),
        &command.dispatch.as_bytes().as_slice(),&command.claim.as_bytes().as_slice(),&invalid(i64::try_from(command.claim_generation))?,
        &command.now_ms,&finance.attempt.as_slice(),&finance.operation.as_slice(),&finance.effect.as_slice()]).await.map_err(map_db)?;
    parse_verdict(&row)?;
    super::common::read_dispatch_view(tx, &command.owner, &command.dispatch).await
}
