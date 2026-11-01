//! An exact terminal seal and independent never-sent money state authorize unused release.
use super::{FundedImageAdmission, RuntimeImageAuthority, draft_error, finance_error, records};
use df_commerce::draft_image::{StageCall, StageRoute};
use df_finance::draft_image::{Error, Settlement, StageCancellationPort, StoredStageQuote};
use df_finance::ports::{FencePort, MediaPort, MemberPort};
use df_finance::service::FinanceError;
use df_finance::{ImageStagePhase, ProviderStageBinding, ProviderStageCancellationPort};
use df_imagery::execution::{DispatchId, DispatchSettlement, ExecutionScope, FinanceTuple};
use df_imagery::publication::{NoDeliveryReceipt, NoDeliveryStage};
use df_imagery::runtime::{ImageDispatchAdmission, RuntimeError};
use df_imagery::{ImageJob, owner_scope_bytes};
use serde_json::json;
use tokio_postgres::Transaction;

struct CanonicalCancellation<'a> {
    job: &'a ImageJob,
    receipt: &'a NoDeliveryReceipt,
    now_ms: i64,
}
struct Link<'a> {
    call: &'a StageCall,
    route: &'a StageRoute,
    finance: FinanceTuple,
    account: Option<Vec<u8>>,
    campaign: Option<Vec<u8>>,
    fingerprint: Option<[u8; 32]>,
}
fn database(error: tokio_postgres::Error) -> Error {
    Error::Finance(error.into())
}
fn campaign_error(error: Error) -> FinanceError {
    match error {
        Error::Finance(error) => error,
        Error::PortUnavailable => FinanceError::PortUnavailable,
        _ => FinanceError::ImageStageConflict,
    }
}
impl CanonicalCancellation<'_> {
    fn scope(&self, call: &StageCall) -> Result<&ExecutionScope, Error> {
        let NoDeliveryStage::Dispatch {
            scope,
            usage_revision: 0,
        } = &self.receipt.tail
        else {
            return Err(Error::LinkConflict);
        };
        if self.now_ms <= 0
            || self.receipt.owner != self.job.owner
            || self.receipt.job != self.job.job
            || self.receipt.expected_job_revision == 0
            || self.receipt.expected_job_revision.checked_add(1) != Some(self.receipt.job_revision)
            || scope.owner != self.job.owner
            || scope.job != self.job.job
            || scope.job.as_bytes() != &call.job.0
            || scope.stage.as_bytes() != &call.stage.0
            || scope.dispatch.as_bytes() != &call.dispatch
        {
            return Err(Error::LinkConflict);
        }
        Ok(scope)
    }
    async fn verify_link(
        &self,
        tx: &Transaction<'_>,
        link: Link<'_>,
        phase: ImageStagePhase,
    ) -> Result<(), Error> {
        let scope = self.scope(link.call)?;
        // Same global finance -> raw -> job order as settlement and the actual usage writer.
        tx.query_one(
            "SELECT pg_advisory_xact_lock(hashtextextended(encode($1::bytea,'hex'),44))",
            &[&scope.dispatch.as_bytes().as_slice()],
        )
        .await
        .map_err(database)?;
        let job = tx.query_opt("SELECT state,revision,intent_bytes,config_bytes FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
            &[&owner_scope_bytes(&self.job.owner), &self.job.job.as_bytes().as_slice()]).await.map_err(database)?.ok_or(Error::LinkConflict)?;
        if job.get::<_, String>("state") != "retired"
            || u64::try_from(job.get::<_, i64>("revision")).map_err(|_| Error::InvalidRecord)?
                < self.receipt.job_revision
            || job.get::<_, Vec<u8>>("intent_bytes") != self.job.intent.bytes()
            || job.get::<_, Vec<u8>>("config_bytes") != self.job.config.bytes()
        {
            return Err(Error::LinkConflict);
        }
        let stored =
            crate::imagery_publication::read_no_delivery_in(tx, &self.job.owner, &self.job.job)
                .await
                .map_err(|_| Error::PortUnavailable)?
                .ok_or(Error::LinkConflict)?;
        if &stored != self.receipt {
            return Err(Error::LinkConflict);
        }
        let settlement = if phase == ImageStagePhase::New {
            "held"
        } else {
            "released"
        };
        let finance_state = if phase == ImageStagePhase::New {
            "exposure_open"
        } else {
            "exposure_released"
        };
        let row = tx.query_opt("SELECT 1 FROM imagery.image_finance_admission_binding b \
            JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=b.dispatch_id \
            JOIN imagery.image_execution_stage s ON s.job_id=d.job_id AND s.stage_id=d.stage_id \
            JOIN imagery.image_stage_cursor c ON c.job_id=d.job_id AND c.stage_id=d.stage_id \
            JOIN finance.provider_attempt a ON a.attempt_id=b.finance_attempt \
            JOIN finance.reservation r ON r.reservation_id=a.reservation_id \
            WHERE b.owner_scope=$1 AND b.job_id=$2 AND b.execution_stage_id=$3 AND b.dispatch_id=$4 \
            AND b.finance_attempt=$5 AND b.operation_id=$6 AND b.effect_id=$7 \
            AND b.stage_call=$8::text::jsonb AND b.cost_route=$9::text::jsonb AND b.route_key=$10 \
            AND b.account_id IS NOT DISTINCT FROM $11 AND b.campaign_id IS NOT DISTINCT FROM $12 \
            AND ($13::bytea IS NULL OR b.receipt_fingerprint=$13) \
            AND d.job_id=b.job_id AND d.stage_id=b.execution_stage_id AND d.finance_attempt=b.finance_attempt \
            AND d.finance_operation=b.operation_id AND d.finance_effect=b.effect_id AND NOT d.local_accounting \
            AND d.dispatch_state='released' AND d.usage_revision=0 AND d.settlement=$14 \
            AND s.owner_scope=b.owner_scope AND s.stage_state='released' AND NOT s.capacity_reserved \
            AND c.current_dispatch=d.dispatch_id AND a.effect_id=b.effect_id AND r.operation_id=b.operation_id \
            AND a.account_id IS NOT DISTINCT FROM b.account_id AND a.campaign_id IS NOT DISTINCT FROM b.campaign_id \
            AND a.tenant_id IS NOT DISTINCT FROM b.tenant_id AND r.session_id IS NOT DISTINCT FROM b.session_id \
            AND a.provider=$17 AND a.model=$18 AND r.price_version_id=$19 \
            AND a.supplier_state=$15 AND ((r.state='held' AND a.customer_state='held' AND a.dispatch_state IN ('reserved','dispatch_prepared') AND $16) \
                OR (r.state='released' AND a.dispatch_state='cancelled_unsent' AND a.customer_state='released' AND NOT $16)) \
            AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_usage u WHERE u.dispatch_id=d.dispatch_id) FOR UPDATE OF d",
            &[&owner_scope_bytes(&self.job.owner), &scope.job.as_bytes().as_slice(), &scope.stage.as_bytes().as_slice(),
              &scope.dispatch.as_bytes().as_slice(), &link.finance.attempt.as_slice(), &link.finance.operation.as_slice(), &link.finance.effect.as_slice(),
              &json!(link.call).to_string(), &json!(link.route).to_string(), &link.route.key.as_slice(), &link.account, &link.campaign,
              &link.fingerprint.map(|value|value.to_vec()), &settlement, &finance_state, &(phase==ImageStagePhase::New), &link.route.provider, &link.route.model, &link.route.price_version.as_slice()])
            .await.map_err(database)?;
        row.map(|_| ()).ok_or(Error::LinkConflict)
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        call: &StageCall,
        finance: FinanceTuple,
        replay: bool,
    ) -> Result<(), Error> {
        let scope = self.scope(call)?;
        if tx.query_opt("SELECT 1 FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id) WHERE a.attempt_id=$1 AND a.effect_id=$2 AND r.operation_id=$3 AND a.dispatch_state='cancelled_unsent' AND a.customer_state='released' AND a.supplier_state='exposure_released' AND r.state='released'",
            &[&finance.attempt.as_slice(), &finance.effect.as_slice(), &finance.operation.as_slice()]).await.map_err(database)?.is_none() { return Err(Error::LinkConflict) }
        if replay {
            let current =
                crate::imagery_execution::read_dispatch_in(tx, &self.job.owner, &scope.dispatch)
                    .await
                    .map_err(|_| Error::LinkConflict)?;
            if current.settlement != DispatchSettlement::Released {
                return Err(Error::LinkConflict);
            }
        } else {
            crate::imagery_execution::mirror_settlement_in(
                tx,
                &self.job.owner,
                &scope.job,
                &scope.stage,
                &scope.dispatch,
                DispatchSettlement::Released,
                self.now_ms,
            )
            .await
            .map_err(|_| Error::LinkConflict)?;
        }
        Ok(())
    }
}
fn draft_link(quote: &StoredStageQuote) -> Result<Link<'_>, Error> {
    let binding = quote.binding();
    Ok(Link {
        call: quote.call(),
        route: quote.route(),
        finance: FinanceTuple::new(binding.attempt, binding.operation, binding.effect)
            .map_err(|_| Error::InvalidRecord)?,
        account: Some(binding.scope.account.as_bytes().to_vec()),
        campaign: None,
        fingerprint: Some(quote.fingerprint()),
    })
}
fn campaign_link(binding: &ProviderStageBinding) -> Result<Link<'_>, Error> {
    Ok(Link {
        call: &binding.call,
        route: &binding.route,
        finance: FinanceTuple::new(
            *binding.attempt.as_bytes(),
            *binding.operation.as_bytes(),
            *binding.effect.as_bytes(),
        )
        .map_err(|_| Error::InvalidRecord)?,
        account: None,
        campaign: Some(binding.campaign.as_bytes().to_vec()),
        fingerprint: None,
    })
}
impl StageCancellationPort for CanonicalCancellation<'_> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        quote: &StoredStageQuote,
        phase: ImageStagePhase,
    ) -> Result<(), Error> {
        self.verify_link(tx, draft_link(quote)?, phase).await
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        quote: &StoredStageQuote,
        settlement: &Settlement,
    ) -> Result<(), Error> {
        if settlement.charged_micro_dollars != 0
            || settlement.released_micro_dollars != quote.maximums().retail_micro_dollars
        {
            return Err(Error::LinkConflict);
        }
        self.mirror(
            tx,
            quote.call(),
            draft_link(quote)?.finance,
            settlement.replay,
        )
        .await
    }
}
impl ProviderStageCancellationPort for CanonicalCancellation<'_> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        binding: &ProviderStageBinding,
        phase: ImageStagePhase,
    ) -> Result<(), FinanceError> {
        self.verify_link(tx, campaign_link(binding).map_err(campaign_error)?, phase)
            .await
            .map_err(campaign_error)
    }
    async fn mirror(
        &self,
        tx: &Transaction<'_>,
        binding: &ProviderStageBinding,
        phase: ImageStagePhase,
    ) -> Result<(), FinanceError> {
        self.mirror(
            tx,
            &binding.call,
            campaign_link(binding).map_err(campaign_error)?.finance,
            phase == ImageStagePhase::Replay,
        )
        .await
        .map_err(campaign_error)
    }
}
impl<F: FencePort, M: MediaPort, S: MemberPort, A: RuntimeImageAuthority, L: ImageDispatchAdmission>
    FundedImageAdmission<F, M, S, A, L>
{
    pub(super) async fn release_unused_stage(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        receipt: &NoDeliveryReceipt,
    ) -> Result<(), RuntimeError> {
        if receipt.owner != job.owner || receipt.job != job.job {
            return Err(RuntimeError::InvalidRecord);
        }
        let mut finance = self.ports.finance.lock().await;
        let stored = records::read_admitted_funding(&finance, job, dispatch).await?;
        let port = CanonicalCancellation {
            job,
            receipt,
            now_ms: finance.now_ms(),
        };
        match stored {
            records::AdmittedFunding::Draft { account, quote } => {
                finance
                    .release_unsent_draft_stage(account, &quote, &port)
                    .await
                    .map_err(draft_error)?;
            }
            records::AdmittedFunding::Campaign(binding) => {
                finance
                    .release_unsent_provider_stage(&binding, &port)
                    .await
                    .map_err(finance_error)?;
            }
        }
        Ok(())
    }
}
