//! First Sent commits finance and execution together; replay only verifies retained state.
use super::context::{self, Guard};
use super::records::{self, AdmittedFunding};
use super::{FundedImageAdmission, RuntimeImageAuthority, draft_error, finance_error};
use crate::imagery_draft_admission::CanonicalStageSend;
use df_finance::draft_image::{Error, StageSendPort, StoredStageQuote};
use df_finance::ports::{FencePort, MediaPort, MemberPort};
use df_finance::service::FinanceError;
use df_finance::{ProviderSendPort, ProviderStageBinding};
use df_imagery::execution::{DispatchMode, DispatchState, FinanceTuple, MarkSentCommand};
use df_imagery::runtime::{
    DispatchAdmission, ImageDispatchAdmission, RuntimeError, SendAuthorization,
};
use tokio_postgres::Transaction;

struct SendPort<'a, A> {
    canonical: CanonicalStageSend<Guard<'a, A>>,
    command: MarkSentCommand,
    finance: FinanceTuple,
}
fn execution_error(error: df_imagery::execution::ExecutionError) -> Error {
    match error {
        df_imagery::execution::ExecutionError::Unavailable => Error::PortUnavailable,
        df_imagery::execution::ExecutionError::Unknown => {
            Error::Finance(FinanceError::ImageStageUncertain {
                sqlstate: None,
                detail: "execution send transaction uncertain".to_owned(),
            })
        }
        _ => Error::LinkConflict,
    }
}
impl<A: RuntimeImageAuthority> SendPort<'_, A> {
    async fn mirror(&self, tx: &Transaction<'_>, new_send: bool) -> Result<(), Error> {
        let stored = if new_send {
            crate::imagery_execution::mark_hosted_sent_in(tx, &self.command, &self.finance)
                .await
                .map_err(execution_error)?
        } else {
            crate::imagery_execution::read_dispatch_in(
                tx,
                &self.command.owner,
                &self.command.dispatch,
            )
            .await
            .map_err(execution_error)?
        };
        if stored.job != self.command.job
            || stored.stage != self.command.stage
            || stored.finance != Some(self.finance)
            || stored.local_accounting
            || (new_send && stored.state != DispatchState::Sent)
            || (!new_send
                && !matches!(
                    stored.state,
                    DispatchState::Sent
                        | DispatchState::Succeeded
                        | DispatchState::Failed
                        | DispatchState::Unknown
                ))
        {
            return Err(Error::LinkConflict);
        }
        Ok(())
    }
}
impl<A: RuntimeImageAuthority> StageSendPort for SendPort<'_, A> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        quote: &StoredStageQuote,
        new_send: bool,
    ) -> Result<(), Error> {
        StageSendPort::verify(&self.canonical, tx, quote, new_send).await?;
        self.mirror(tx, new_send).await
    }
}
impl<A: RuntimeImageAuthority> ProviderSendPort for SendPort<'_, A> {
    async fn verify(
        &self,
        tx: &Transaction<'_>,
        binding: &ProviderStageBinding,
        new_send: bool,
    ) -> Result<(), FinanceError> {
        ProviderSendPort::verify(&self.canonical, tx, binding, new_send).await?;
        self.mirror(tx, new_send)
            .await
            .map_err(|error| match error {
                Error::Finance(error) => error,
                Error::PortUnavailable => FinanceError::PortUnavailable,
                _ => FinanceError::ImageStageConflict,
            })
    }
}

impl<F: FencePort, M: MediaPort, S: MemberPort, A: RuntimeImageAuthority, L: ImageDispatchAdmission>
    FundedImageAdmission<F, M, S, A, L>
{
    pub(super) async fn send_stage(
        &self,
        input: &DispatchAdmission<'_>,
        mode: DispatchMode,
    ) -> Result<SendAuthorization, RuntimeError> {
        if context::validate(input)? {
            if mode != DispatchMode::Local {
                return Err(RuntimeError::InvalidRecord);
            }
            return self.ports.local.mark_sent(input, mode).await;
        }
        let expected = mode.finance().ok_or(RuntimeError::InvalidRecord)?;
        let mut finance = self.ports.finance.lock().await;
        let stored = records::read_admitted_funding(&finance, input.job, input.dispatch).await?;
        let call = context::call(finance.client(), input).await?;
        if stored.call() != &call
            || stored.finance()? != expected
            || !context::route_matches(
                stored.route(),
                input.quote.ok_or(RuntimeError::InvalidRecord)?,
            )
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let now = finance.now_ms();
        let port = SendPort {
            canonical: CanonicalStageSend {
                authority: Guard {
                    authority: &self.ports.authority,
                    owner: &input.job.owner,
                    call: &call,
                    sources: input.sources,
                    sending: true,
                },
                claim: context::claim(input),
                now_ms: now,
            },
            command: MarkSentCommand::new(
                input.job.owner.clone(),
                input.job.job,
                input.stage.stage,
                input.dispatch,
                input.claim,
                input.stage.claim_generation,
                now,
            )
            .map_err(|_| RuntimeError::InvalidRecord)?,
            finance: expected,
        };
        let new_send = match stored {
            AdmittedFunding::Draft { account, quote } => finance
                .mark_draft_stage_sent(account, &quote, self.settings.dispatcher, &port)
                .await
                .map_err(draft_error)?,
            AdmittedFunding::Campaign(binding) => finance
                .mark_provider_stage_sent(&binding, self.settings.dispatcher, &port)
                .await
                .map_err(finance_error)?,
        };
        Ok(if new_send {
            SendAuthorization::New
        } else {
            SendAuthorization::AlreadySent
        })
    }
}

#[cfg(test)]
#[path = "sending_tests.rs"]
mod tests;
