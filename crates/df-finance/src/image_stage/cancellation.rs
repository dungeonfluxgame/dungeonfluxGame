//! Proven unused-stage release reuses the campaign NotDispatched release planner.
use super::{ImageStagePhase, ProviderStageBinding};
use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::{Committed, FinanceError, FinanceService};
use df_commerce::finance_ids::{EntryId, EventId};
use df_commerce::settlement::{ReleaseReason, ReleaseRequest, plan_release};
use serde_json::json;
use std::{future::Future, time::Duration};
use tokio_postgres::Transaction;

#[derive(Clone, Debug)]
pub enum ProviderCancellationOutcome {
    Applied(Committed),
    Replay,
}
/// The actual terminal receipt and never-sent execution identity share the money transaction.
pub trait ProviderStageCancellationPort: Send + Sync {
    fn verify(
        &self,
        tx: &Transaction<'_>,
        binding: &ProviderStageBinding,
        phase: ImageStagePhase,
    ) -> impl Future<Output = Result<(), FinanceError>> + Send;
    fn mirror(
        &self,
        tx: &Transaction<'_>,
        binding: &ProviderStageBinding,
        phase: ImageStagePhase,
    ) -> impl Future<Output = Result<(), FinanceError>> + Send;
}
impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    pub async fn release_unsent_provider_stage<P: ProviderStageCancellationPort>(
        &mut self,
        binding: &ProviderStageBinding,
        port: &P,
    ) -> Result<ProviderCancellationOutcome, FinanceError> {
        let request = ReleaseRequest {
            event_id: EventId::from_array(self.id()),
            entry_id: EntryId::from_array(self.id()),
            reason: ReleaseReason::NotDispatched,
            now_ms: self.clock.now_ms(),
        };
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let tx = Self::start(&mut self.client).await?;
        let snapshot = Self::lock(&tx, "release", json!({"attempt":binding.attempt.hex()})).await?;
        let held = crate::wire::parse_held_snapshot(&snapshot)?;
        let row = tx.query_opt("SELECT a.dispatch_state,a.customer_state,a.supplier_state,r.state FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id) WHERE a.attempt_id=$1 AND a.campaign_id=$2 AND a.tenant_id=$3 AND r.operation_id=$4 AND r.session_id=$5 AND a.effect_id=$6",
            &[&binding.attempt.as_bytes().as_slice(), &binding.campaign.as_bytes().as_slice(),
            &binding.tenant.as_bytes().as_slice(), &binding.operation.as_bytes().as_slice(),
            &binding.session.as_bytes().as_slice(), &binding.effect.as_bytes().as_slice()]).await?.ok_or(FinanceError::ImageStageConflict)?;
        let dispatch: String = row.get(0);
        let customer: String = row.get(1);
        let supplier: String = row.get(2);
        let reservation: String = row.get(3);
        let phase = if held.is_some()
            && reservation == "held"
            && matches!(dispatch.as_str(), "reserved" | "dispatch_prepared")
            && customer == "held"
            && supplier == "exposure_open"
        {
            ImageStagePhase::New
        } else if held.is_none()
            && reservation == "released"
            && dispatch == "cancelled_unsent"
            && customer == "released"
            && supplier == "exposure_released"
        {
            ImageStagePhase::Replay
        } else {
            return Err(FinanceError::ImageStageConflict);
        };
        tokio::time::timeout(deadline, port.verify(&tx, binding, phase))
            .await
            .map_err(|_| FinanceError::PortUnavailable)??;
        if phase == ImageStagePhase::Replay {
            tokio::time::timeout(deadline, port.mirror(&tx, binding, phase))
                .await
                .map_err(|_| FinanceError::PortUnavailable)??;
            tx.rollback().await?;
            return Ok(ProviderCancellationOutcome::Replay);
        }
        let held = held.ok_or(FinanceError::Stale)?;
        if held.attempt.attempt != binding.attempt {
            return Err(FinanceError::ImageStageConflict);
        }
        let plan = plan_release(&held, &request).map_err(FinanceError::Settle)?;
        let committed = Self::commit_plan(&tx, &plan).await?;
        tokio::time::timeout(deadline, port.mirror(&tx, binding, phase))
            .await
            .map_err(|_| FinanceError::PortUnavailable)??;
        tx.commit().await.map_err(super::commit_unknown)?;
        Ok(ProviderCancellationOutcome::Applied(committed))
    }
}
