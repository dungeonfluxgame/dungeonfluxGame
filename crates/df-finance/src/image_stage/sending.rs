//! Campaign first-send authorization using existing lifecycle plans and a mandatory source port.
use super::ProviderStageBinding;
use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::{FinanceError, FinanceService};
use df_commerce::finance_ids::EventId;
use df_commerce::lifecycle::{Deadlines, plan_mark_sent, plan_prepare_dispatch};
use df_commerce::snapshot::DispatchState;
use serde_json::json;
use std::future::Future;
use std::time::Duration;
use tokio_postgres::Transaction;

/// Same-tx source and current-dispatch verification; replay never permits another send.
pub trait ProviderSendPort: Send + Sync {
    fn verify(
        &self,
        tx: &Transaction<'_>,
        binding: &ProviderStageBinding,
        new_send: bool,
    ) -> impl Future<Output = Result<(), FinanceError>> + Send;
}
impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    pub async fn mark_provider_stage_sent<P: ProviderSendPort>(
        &mut self,
        binding: &ProviderStageBinding,
        dispatcher: [u8; 16],
        port: &P,
    ) -> Result<bool, FinanceError> {
        if dispatcher == [0; 16] {
            return Err(FinanceError::ImageStageConflict);
        }
        let prepare_event = EventId::from_array(self.id());
        let sent_event = EventId::from_array(self.id());
        let now_ms = self.clock.now_ms();
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let tx = Self::start(&mut self.client).await?;
        let args = json!({"attempt":binding.attempt.hex()});
        let mut document = Self::lock(&tx, "attempt", args.clone()).await?;
        let parse = |document: &serde_json::Value| {
            crate::wire::parse_attempt_row(document.get("attempt").ok_or(FinanceError::Stale)?)
                .map_err(FinanceError::Wire)
        };
        let mut attempt = parse(&document)?;
        if attempt.dispatch == DispatchState::Reserved {
            let plan = plan_prepare_dispatch(prepare_event, &attempt, Deadlines::default(), now_ms)
                .map_err(FinanceError::Lifecycle)?;
            Self::commit_plan(&tx, &plan).await?;
            document = Self::lock(&tx, "attempt", args).await?;
            attempt = parse(&document)?;
        }
        let new_send = match attempt.dispatch {
            DispatchState::Sent => {
                let row=tx.query_one("SELECT dispatcher_instance_id FROM finance.provider_attempt WHERE attempt_id=$1",
                    &[&binding.attempt.as_bytes().as_slice()]).await?;
                let stored: Option<Vec<u8>> = row.get(0);
                if stored.as_deref() != Some(dispatcher.as_slice()) {
                    return Err(FinanceError::ImageStageConflict);
                }
                false
            }
            DispatchState::DispatchPrepared => {
                let plan = plan_mark_sent(
                    sent_event,
                    &attempt,
                    dispatcher,
                    Deadlines::default(),
                    now_ms,
                )
                .map_err(FinanceError::Lifecycle)?;
                Self::commit_plan(&tx, &plan).await?;
                true
            }
            _ => return Err(FinanceError::Stale),
        };
        tokio::time::timeout(deadline, port.verify(&tx, binding, new_send))
            .await
            .map_err(|_| FinanceError::PortUnavailable)??;
        tx.commit().await.map_err(super::commit_unknown)?;
        Ok(new_send)
    }
}
