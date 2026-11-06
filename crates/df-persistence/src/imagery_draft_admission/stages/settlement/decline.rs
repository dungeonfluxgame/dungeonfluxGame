//! Only the protected selected submit-decline receipt can authorize historical Declined holds.
use super::*;
use df_imagery::execution::ExecutionScope;

impl CanonicalStageSettlement {
    pub(super) async fn verify_declined(
        &self,
        tx: &Transaction<'_>,
        usage: &UsageMatch<'_>,
        row: &tokio_postgres::Row,
    ) -> Result<(), Error> {
        if !matches!(usage.proof, EconomicEvidence::Image)
            || usage.evidence != StageUsageEvidence::Unknown
        {
            return Err(Error::LinkConflict);
        }
        let call = usage.call;
        let scope = ExecutionScope {
            owner: self.owner.clone(),
            job: ImageJobId::from_bytes(&call.job.0).map_err(|_| Error::InvalidRecord)?,
            stage: ExecutionStageId::from_bytes(&call.stage.0).map_err(|_| Error::InvalidRecord)?,
            dispatch: DispatchId::from_bytes(&call.dispatch).map_err(|_| Error::InvalidRecord)?,
        };
        let basis = crate::imagery_execution::read_stage_basis_in(
            tx,
            &scope.owner,
            &scope.job,
            &scope.stage,
        )
        .await
        .map_err(execution_error)?;
        let receipt = crate::imagery_execution::read_decline_reference_in(tx, &scope)
            .await
            .map_err(execution_error)?
            .ok_or(Error::LinkConflict)?;
        let raw: Option<Vec<u8>> = row.get("raw_output_sha256");
        let bytes: Option<i64> = row.get("raw_bytes");
        if receipt.scope != scope
            || receipt.basis != basis
            || receipt.finance != Some(usage.finance)
            || receipt.receipt.usage_revision != usage.revision
            || receipt.status != 429
            || receipt.raw_bytes == 0
            || raw.as_deref() != Some(receipt.raw_sha256.as_slice())
            || bytes.and_then(|value| u64::try_from(value).ok()) != Some(receipt.raw_bytes)
            || receipt.route.provider != usage.route.provider
            || receipt.route.model != usage.route.model
            || receipt.route.cause != usage.route.cause
            || receipt.route.detail != usage.route.detail
            || basis.kind.key() != call.kind.label()
            || basis.input_revision != call.input_revision
            || basis.input_digest != call.input_digest
            || basis.config_revision != call.config_revision
            || basis.config_digest != call.config_digest
            || basis.parent_dispatch.map(|id| *id.as_bytes()) != call.parent_dispatch
        {
            return Err(Error::LinkConflict);
        }
        let sources=tx.query_opt("SELECT 1 FROM imagery.image_dispatch_sources WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 AND dispatch_id=$4",
            &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&scope.dispatch.as_bytes().as_slice()]).await.map_err(database_error)?;
        if sources.is_none() {
            return Err(Error::LinkConflict);
        }
        Ok(())
    }
}
