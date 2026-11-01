//! Proven unused-stage release on the existing account release command and journal.
use super::{StoredStageQuote, rows};
use crate::ImageStagePhase;
use crate::draft_image::{Error, Settlement};
use crate::ports::{FencePort, MediaPort, MemberPort};
use crate::service::FinanceService;
use df_types::AccountId;
use serde_json::json;
use std::{future::Future, time::Duration};
use tokio_postgres::Transaction;

/// Canonical terminal authority is verified before releasing, then mirrored in the same transaction.
pub trait StageCancellationPort: Send + Sync {
    fn verify(
        &self,
        tx: &Transaction<'_>,
        quote: &StoredStageQuote,
        phase: ImageStagePhase,
    ) -> impl Future<Output = Result<(), Error>> + Send;
    fn mirror(
        &self,
        tx: &Transaction<'_>,
        quote: &StoredStageQuote,
        settlement: &Settlement,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}
impl<F: FencePort, M: MediaPort, S: MemberPort> FinanceService<F, M, S> {
    pub async fn release_unsent_draft_stage<P: StageCancellationPort>(
        &mut self,
        account: AccountId,
        quote: &StoredStageQuote,
        port: &P,
    ) -> Result<Settlement, Error> {
        quote.binding().validate(account).map_err(Error::Policy)?;
        let mut document = quote.document("release");
        self.stamp_draft_document(&mut document);
        let deadline = Duration::from_millis(self.config.port_deadline_ms);
        let tx = Self::start(&mut self.client)
            .await
            .map_err(Error::Finance)?;
        let mut locked = document.clone();
        locked["action"] = json!("lock");
        rows::execute_stage(&tx, account, &locked).await?;
        let row = tx.query_one("SELECT a.dispatch_state,a.customer_state,a.supplier_state,r.state FROM finance.provider_attempt a JOIN finance.reservation r USING(reservation_id) WHERE a.attempt_id=$1 AND a.account_id=$2",
            &[&quote.attempt().as_bytes().as_slice(), &account.as_bytes().as_slice()])
            .await.map_err(|error| Error::Finance(error.into()))?;
        let dispatch: String = row.get(0);
        let customer: String = row.get(1);
        let supplier: String = row.get(2);
        let reservation: String = row.get(3);
        let phase = if reservation == "held"
            && matches!(dispatch.as_str(), "reserved" | "dispatch_prepared")
            && customer == "held"
            && supplier == "exposure_open"
        {
            ImageStagePhase::New
        } else if reservation == "released"
            && dispatch == "cancelled_unsent"
            && customer == "released"
            && supplier == "exposure_released"
        {
            ImageStagePhase::Replay
        } else {
            return Err(Error::LinkConflict);
        };
        tokio::time::timeout(deadline, port.verify(&tx, quote, phase))
            .await
            .map_err(|_| Error::PortUnavailable)??;
        if phase == ImageStagePhase::Replay {
            // Existing release returns its exact amount; neither command nor mirror may repair rows.
            let result = rows::execute_stage(&tx, account, &document).await?;
            let settlement = rows::settlement(&result)?;
            if !settlement.replay {
                return Err(Error::LinkConflict);
            }
            tokio::time::timeout(deadline, port.mirror(&tx, quote, &settlement))
                .await
                .map_err(|_| Error::PortUnavailable)??;
            tx.rollback()
                .await
                .map_err(|error| Error::Finance(error.into()))?;
            return Ok(settlement);
        }
        let result = rows::execute_stage(&tx, account, &document).await?;
        let settlement = rows::settlement(&result)?;
        if settlement.replay {
            return Err(Error::LinkConflict);
        }
        tokio::time::timeout(deadline, port.mirror(&tx, quote, &settlement))
            .await
            .map_err(|_| Error::PortUnavailable)??;
        tx.commit().await.map_err(rows::commit_unknown)?;
        Ok(settlement)
    }
}
