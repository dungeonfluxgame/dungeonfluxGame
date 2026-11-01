//! First Sent requires current canonical source and execution authority under the finance locks.
use super::{StageAuthority, StagePayer, database_error};
use df_commerce::draft_image::{StageCall, StageRoute};
use df_finance::draft_image::{Error, StageSendPort, StoredStageQuote};
use df_finance::service::FinanceError;
use df_finance::{ProviderSendPort, ProviderStageBinding};
use df_imagery::execution::{FinanceTuple, StageClaim};
use df_imagery::{ImageOwner, owner_scope_bytes};
use serde_json::json;
use tokio_postgres::Transaction;

/// The same source authority used at admission; no provider I/O is allowed in this port.
pub struct CanonicalStageSend<A> {
    pub authority: A,
    pub claim: StageClaim,
    pub now_ms: i64,
}
struct SendCheck<'a> {
    owner: &'a ImageOwner,
    payer: StagePayer<'a>,
    call: &'a StageCall,
    route: &'a StageRoute,
    finance: FinanceTuple,
    fingerprint: Option<[u8; 32]>,
}
impl<A: StageAuthority> CanonicalStageSend<A> {
    async fn verify_send(
        &self,
        tx: &Transaction<'_>,
        check: SendCheck<'_>,
        new_send: bool,
    ) -> Result<(), Error> {
        let call = check.call;
        let scope = &self.claim.scope;
        let owner = check.owner;
        if &scope.owner != owner
            || scope.job.as_bytes() != &call.job.0
            || scope.stage.as_bytes() != &call.stage.0
            || self.now_ms <= 0
        {
            return Err(Error::LinkConflict);
        }
        self.authority
            .verify(tx, check.payer, owner, call, !new_send)
            .await?;
        let row=tx.query_opt("SELECT 1 FROM imagery.image_finance_admission_binding b \
            JOIN imagery.image_dispatch_attempt d ON d.dispatch_id=b.dispatch_id \
            WHERE b.finance_attempt=$1 AND b.owner_scope=$2 AND b.job_id=$3 AND b.execution_stage_id=$4 \
            AND b.dispatch_id=$5 AND b.operation_id=$6 AND b.effect_id=$7 AND ($8::bytea IS NULL OR b.receipt_fingerprint=$8) AND b.stage_call=$9::text::jsonb AND b.cost_route=$10::text::jsonb \
            AND d.finance_attempt=b.finance_attempt AND d.finance_operation=b.operation_id AND d.finance_effect=b.effect_id",
            &[&check.finance.attempt.as_slice(),&owner_scope_bytes(owner),&call.job.0.as_slice(),&call.stage.0.as_slice(),
              &call.dispatch.as_slice(),&check.finance.operation.as_slice(),&check.finance.effect.as_slice(),&check.fingerprint.map(|digest|digest.to_vec()),
              &json!(call).to_string(),&json!(check.route).to_string()])
            .await.map_err(database_error)?;
        if row.is_none() {
            return Err(Error::LinkConflict);
        }
        if !new_send {
            return Ok(());
        }
        // Serialize the shared job before the guarded Sent helper takes its exclusive gate.
        if tx
            .query_opt(
                "SELECT 1 FROM imagery.image_job WHERE job_id=$1 AND owner_scope=$2 FOR UPDATE",
                &[&call.job.0.as_slice(), &owner_scope_bytes(owner)],
            )
            .await
            .map_err(database_error)?
            .is_none()
        {
            return Err(Error::LinkConflict);
        }
        let row=tx.query_opt("SELECT 1 FROM imagery.image_job j JOIN imagery.image_execution_stage s USING(job_id) \
            JOIN imagery.image_stage_cursor c USING(job_id,stage_id) JOIN imagery.image_dispatch_attempt d \
            ON d.dispatch_id=c.current_dispatch AND d.job_id=s.job_id AND d.stage_id=s.stage_id \
            JOIN imagery.image_execution_gate g ON g.gate_scope=s.capacity_scope \
            WHERE j.owner_scope=$1 AND j.job_id=$2 AND s.stage_id=$3 AND c.current_dispatch=$4 \
            AND j.state<>'retired' AND j.absolute_deadline_ms>$5 AND s.stage_state='admitted' \
            AND s.claim_token=$6 AND s.claim_generation=$7 AND s.lease_until_ms>$5 \
            AND d.claim_token=$6 AND d.claim_generation=$7 AND d.dispatch_state='prepared' \
            AND g.enabled AND g.applied_revision=s.claim_config_revision \
            AND EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.job_id=j.job_id AND i.state='attached') FOR SHARE OF j,s,d,g",
            &[&owner_scope_bytes(owner),&call.job.0.as_slice(),&call.stage.0.as_slice(),&call.dispatch.as_slice(),
              &self.now_ms,&self.claim.token.as_bytes().as_slice(),
              &i64::try_from(self.claim.generation).map_err(|_|Error::InvalidRecord)?]).await.map_err(database_error)?;
        if row.is_none() {
            return Err(Error::LinkConflict);
        }
        Ok(())
    }
}

impl<A: StageAuthority> StageSendPort for CanonicalStageSend<A> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        quote: &StoredStageQuote,
        new_send: bool,
    ) -> Result<(), Error> {
        let binding = quote.binding();
        let owner = ImageOwner::Draft {
            account: binding.scope.account,
        };
        self.verify_send(
            tx,
            SendCheck {
                owner: &owner,
                payer: StagePayer::Account(binding.scope.account),
                call: quote.call(),
                route: quote.route(),
                finance: FinanceTuple::new(binding.attempt, binding.operation, binding.effect)
                    .map_err(|_| Error::InvalidRecord)?,
                fingerprint: Some(quote.fingerprint()),
            },
            new_send,
        )
        .await
    }
}
impl<A: StageAuthority> ProviderSendPort for CanonicalStageSend<A> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        binding: &ProviderStageBinding,
        new_send: bool,
    ) -> Result<(), FinanceError> {
        self.verify_send(
            tx,
            SendCheck {
                owner: &self.claim.scope.owner,
                payer: StagePayer::Campaign(binding),
                call: &binding.call,
                route: &binding.route,
                finance: FinanceTuple::new(
                    *binding.attempt.as_bytes(),
                    *binding.operation.as_bytes(),
                    *binding.effect.as_bytes(),
                )
                .map_err(|_| FinanceError::ImageStageConflict)?,
                fingerprint: None,
            },
            new_send,
        )
        .await
        .map_err(|error| match error {
            Error::Finance(error) => error,
            Error::PortUnavailable => FinanceError::PortUnavailable,
            _ => FinanceError::ImageStageConflict,
        })
    }
}
