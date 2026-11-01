//! Campaign stage settlement on the existing held snapshot and exact planners.
use super::{ImageStagePhase, ProviderStageBinding};
use crate::draft_image::{StageUsageEvidence, Usability};
use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::{Committed, FinanceError, FinanceService};
use df_commerce::finance_ids::{EntryId, EventId, UsageEventId};
use df_commerce::settlement::{
    SettleRequest, UnusableOutcome, UnusableRequest, UsageSource, plan_settlement,
    plan_unusable_settlement,
};
use df_commerce::snapshot::DispatchState;
use serde_json::json;
use std::future::Future;
use std::time::Duration;
use tokio_postgres::Transaction;

/// A selected durable raw usage revision for one admitted campaign call.
#[derive(Clone, Debug)]
pub struct ProviderStageSettlement {
    pub binding: ProviderStageBinding,
    pub usage_revision: u32,
    pub evidence: StageUsageEvidence,
    pub actual_units: Vec<(String, u64)>,
    pub usability: Usability,
}
/// A released customer hold with open supplier liability is a typed stale refusal, not success.
#[derive(Clone, Debug)]
pub enum ProviderSettlementOutcome {
    Applied(Committed),
    Replay,
    UnknownOutstanding,
}
/// The canonical port verifies selected raw usage before posting and mirrors after posting.
pub trait ProviderSettlementPort: Send + Sync {
    fn verify(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
    ) -> impl Future<Output = Result<(), FinanceError>> + Send;
    fn mirror(
        &self,
        tx: &Transaction<'_>,
        command: &ProviderStageSettlement,
        phase: ImageStagePhase,
    ) -> impl Future<Output = Result<(), FinanceError>> + Send;
}
impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    pub async fn settle_provider_stage<P: ProviderSettlementPort>(
        &mut self,
        command: &ProviderStageSettlement,
        port: &P,
    ) -> Result<ProviderSettlementOutcome, FinanceError> {
        if command.usage_revision == 0 {
            return Err(FinanceError::ImageStageConflict);
        }
        let event_id = EventId::from_array(self.id());
        let entry_id = EntryId::from_array(self.id());
        let usage_event = UsageEventId::from_array(self.id());
        let now_ms = self.clock.now_ms();
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let tx = Self::start(&mut self.client).await?;
        let snapshot = Self::lock(
            &tx,
            "settlement",
            json!({"attempt":command.binding.attempt.hex()}),
        )
        .await?;
        tokio::time::timeout(deadline, port.verify(&tx, command))
            .await
            .map_err(|_| FinanceError::PortUnavailable)??;
        let held = crate::wire::parse_held_snapshot(&snapshot)?;
        if command.evidence == StageUsageEvidence::Unknown {
            let row=tx.query_one("SELECT dispatch_state,supplier_state FROM finance.provider_attempt WHERE attempt_id=$1",
                &[&command.binding.attempt.as_bytes().as_slice()]).await?;
            let state: String = row.get(0);
            let supplier: String = row.get(1);
            if !matches!(
                state.as_str(),
                "sent" | "outcome_unknown" | "failed_unknown"
            ) || supplier != "exposure_open"
            {
                return Err(FinanceError::Stale);
            }
            tokio::time::timeout(deadline, port.mirror(&tx, command, ImageStagePhase::New))
                .await
                .map_err(|_| FinanceError::PortUnavailable)??;
            tx.commit().await.map_err(super::commit_unknown)?;
            return Ok(ProviderSettlementOutcome::UnknownOutstanding);
        }
        let source = match command.evidence {
            StageUsageEvidence::Reported => UsageSource::Reported,
            StageUsageEvidence::Estimated => UsageSource::Estimated,
            StageUsageEvidence::Unknown => return Err(FinanceError::Stale),
        };
        let Some(held) = held else {
            let row=tx.query_one("SELECT supplier_state,usability FROM finance.provider_attempt WHERE attempt_id=$1",
                &[&command.binding.attempt.as_bytes().as_slice()]).await?;
            let supplier: String = row.get(0);
            let usability: Option<String> = row.get(1);
            let expected = if command.usability == Usability::Usable {
                "usable"
            } else {
                "no_usable_output"
            };
            if supplier != "cost_recorded" || usability.as_deref() != Some(expected) {
                return Err(FinanceError::Stale);
            }
            // The port must verify the existing mirror on replay; it never repairs missing linkage.
            tokio::time::timeout(deadline, port.mirror(&tx, command, ImageStagePhase::Replay))
                .await
                .map_err(|_| FinanceError::PortUnavailable)??;
            tx.rollback().await?;
            return Ok(ProviderSettlementOutcome::Replay);
        };
        if held.attempt.attempt != command.binding.attempt
            || !matches!(
                held.attempt.dispatch,
                DispatchState::Sent | DispatchState::OutcomeUnknown
            )
        {
            return Err(FinanceError::Stale);
        }
        let plan = if command.usability == Usability::Usable {
            plan_settlement(
                &held,
                &SettleRequest {
                    event_id,
                    entry_id,
                    usage_event,
                    actual_units: command.actual_units.clone(),
                    usage_source: source,
                    now_ms,
                },
            )
        } else {
            plan_unusable_settlement(
                &held,
                &UnusableRequest {
                    event_id,
                    entry_id,
                    actual_units: command.actual_units.clone(),
                    usage_source: source,
                    outcome: UnusableOutcome::FailedNoOutput,
                    now_ms,
                },
            )
        }
        .map_err(FinanceError::Settle)?;
        let committed = Self::commit_plan(&tx, &plan).await?;
        tokio::time::timeout(deadline, port.mirror(&tx, command, ImageStagePhase::New))
            .await
            .map_err(|_| FinanceError::PortUnavailable)??;
        tx.commit().await.map_err(super::commit_unknown)?;
        Ok(ProviderSettlementOutcome::Applied(committed))
    }
}
