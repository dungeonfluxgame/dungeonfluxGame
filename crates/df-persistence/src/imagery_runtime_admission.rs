//! Funded runtime stages reuse the existing finance admission, send and settlement ports.
//! Funding descriptors come from host-owned stored consent; this adapter never creates consent.

use crate::imagery_draft_admission::StagePayer;
use crate::imagery_runtime::PostgresImageUsage;
use df_assets::CatalogConfig;
use df_commerce::draft_image::{ConsentId, JobScope, StageCall, StageRoute};
use df_finance::ProviderStageAdmission;
use df_finance::ports::{FencePort, MediaPort, MemberPort};
use df_finance::service::FinanceService;
use df_imagery::ImageOwner;
use df_imagery::runtime::{DispatchAdmission, ImageDispatchAdmission, RuntimeError, SourceReceipt};
use df_types::AccountId;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_postgres::Transaction;

mod cancellation;
mod context;
mod records;
mod sending;
mod settlement;
pub use records::{AdmittedFunding, read_admitted_funding};

/// Reconstructs the canonical stage call from the retained job and actual runtime admission.
pub async fn runtime_stage_call(
    client: &tokio_postgres::Client,
    input: &DispatchAdmission<'_>,
) -> Result<StageCall, RuntimeError> {
    context::call(client, input).await
}

/// Actual stored creator job consent or campaign admission; no descriptor is an egress permit.
#[derive(Clone, Debug)]
pub enum RuntimeFunding {
    Draft(Box<DraftFunding>),
    Campaign(Box<ProviderStageAdmission>),
}

/// The host's persisted creator job grant and selected approved route.
#[derive(Clone, Debug)]
pub struct DraftFunding {
    pub account: AccountId,
    pub consent: ConsentId,
    pub consent_revision: u64,
    pub scope: JobScope,
    pub route: StageRoute,
}

/// Reads host-owned funding and locks current account/session/fence/member/source authority.
pub trait RuntimeImageAuthority: Send + Sync {
    fn funding(
        &self,
        input: &DispatchAdmission<'_>,
    ) -> impl Future<Output = Result<RuntimeFunding, RuntimeError>> + Send;
    fn verify(
        &self,
        tx: &Transaction<'_>,
        payer: StagePayer<'_>,
        owner: &ImageOwner,
        call: &StageCall,
        sources: &[SourceReceipt],
        replay: bool,
    ) -> impl Future<Output = Result<(), df_finance::draft_image::Error>> + Send;
}

/// Production composition supplies the existing finance owner and configured native local edge.
pub struct RuntimeAdmissionPorts<F, M, S, A, L> {
    pub finance: Arc<Mutex<FinanceService<F, M, S>>>,
    pub authority: A,
    pub local: Arc<L>,
    pub usage: PostgresImageUsage,
}

/// Dispatcher identity and quote duration belong to composition, not image request data.
pub struct RuntimeAdmissionSettings {
    pub dispatcher: [u8; 16],
    pub quote_lifetime_ms: u64,
    pub catalog: CatalogConfig,
}

/// One funded admission edge; the supplied local publisher remains its separate CPU-only edge.
pub struct FundedImageAdmission<F, M, S, A, L> {
    ports: RuntimeAdmissionPorts<F, M, S, A, L>,
    settings: RuntimeAdmissionSettings,
}

impl<F: FencePort, M: MediaPort, S: MemberPort, A: RuntimeImageAuthority, L: ImageDispatchAdmission>
    FundedImageAdmission<F, M, S, A, L>
{
    pub fn new(
        settings: RuntimeAdmissionSettings,
        ports: RuntimeAdmissionPorts<F, M, S, A, L>,
    ) -> Result<Self, RuntimeError> {
        if settings.dispatcher == [0; 16]
            || settings.quote_lifetime_ms == 0
            || settings.quote_lifetime_ms > i64::MAX as u64
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        Ok(Self { ports, settings })
    }
}

fn finance_error(error: df_finance::service::FinanceError) -> RuntimeError {
    use df_finance::service::FinanceError;
    #[cfg(test)]
    {
        let kind = match &error {
            FinanceError::ImageStageConflict => Some("stage_conflict".to_owned()),
            FinanceError::ImageStageMissingLink => Some("stage_missing_link".to_owned()),
            FinanceError::ImageStageBinding(reason) => Some(format!("stage_binding_{reason:?}")),
            FinanceError::Wire(_) => Some("wire".to_owned()),
            _ => None,
        };
        if let Some(kind) = kind {
            df_observe::record_audit_line(
                "IMAGERY",
                format!("IMAGERY test_finance_refusal kind={kind}"),
            );
        }
    }
    match error {
        FinanceError::ImageStageCommitUnknown { .. } | FinanceError::ImageStageUncertain { .. } => {
            RuntimeError::ProviderUnknown
        }
        FinanceError::ImageStageEconomicProofPending => RuntimeError::CostProjectionPending,
        FinanceError::Database(_)
        | FinanceError::Boundary { .. }
        | FinanceError::PortUnavailable
        | FinanceError::ImageStageUnavailable { .. } => RuntimeError::StorageUnavailable,
        FinanceError::ImageStageConflict
        | FinanceError::ImageStageMissingLink
        | FinanceError::ImageStageBinding(_)
        | FinanceError::Wire(_) => RuntimeError::InvalidRecord,
        _ => RuntimeError::SourceDenied,
    }
}
fn draft_error(error: df_finance::draft_image::Error) -> RuntimeError {
    use df_finance::draft_image::Error;
    match error {
        Error::Finance(error) => finance_error(error),
        Error::CommitUnknown { .. } => RuntimeError::ProviderUnknown,
        Error::EconomicProofPending => RuntimeError::CostProjectionPending,
        Error::PortUnavailable => RuntimeError::StorageUnavailable,
        Error::InvalidRecord | Error::LinkConflict => RuntimeError::InvalidRecord,
        Error::Policy(_) | Error::Refused(_) | Error::CatalogRefused => RuntimeError::SourceDenied,
    }
}
fn stage_error(error: RuntimeError) -> df_finance::draft_image::Error {
    use df_finance::draft_image::Error;
    match error {
        RuntimeError::StorageUnavailable => Error::PortUnavailable,
        RuntimeError::ProviderUnknown => {
            Error::Finance(df_finance::service::FinanceError::ImageStageUncertain {
                sqlstate: None,
                detail: "runtime authority result uncertain".to_owned(),
            })
        }
        _ => Error::LinkConflict,
    }
}

impl<F: FencePort, M: MediaPort, S: MemberPort, A: RuntimeImageAuthority, L: ImageDispatchAdmission>
    ImageDispatchAdmission for FundedImageAdmission<F, M, S, A, L>
{
    async fn admit(
        &self,
        input: &DispatchAdmission<'_>,
    ) -> Result<df_imagery::execution::DispatchMode, RuntimeError> {
        self.admit_stage(input).await
    }
    async fn mark_sent(
        &self,
        input: &DispatchAdmission<'_>,
        mode: df_imagery::execution::DispatchMode,
    ) -> Result<df_imagery::runtime::SendAuthorization, RuntimeError> {
        self.send_stage(input, mode).await
    }
    async fn outcome_no_output(
        &self,
        input: &df_imagery::runtime::DispatchNoOutputSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        self.settle_no_output(input).await
    }
    async fn release_unused(
        &self,
        job: &df_imagery::ImageJob,
        dispatch: df_imagery::execution::DispatchId,
        receipt: &df_imagery::publication::NoDeliveryReceipt,
    ) -> Result<(), RuntimeError> {
        self.release_unused_stage(job, dispatch, receipt).await
    }
    async fn outcome_decision(
        &self,
        input: &df_imagery::runtime::DispatchDecisionSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        self.settle_decision(input).await
    }
    async fn outcome(
        &self,
        job: &df_imagery::ImageJob,
        dispatch: df_imagery::execution::DispatchId,
        mode: df_imagery::execution::DispatchMode,
        usable: Option<bool>,
    ) -> Result<(), RuntimeError> {
        self.settle_stage(
            job,
            dispatch,
            mode,
            settlement::SettlementProof::Image(usable),
        )
        .await
    }
}

#[cfg(test)]
mod tests;
