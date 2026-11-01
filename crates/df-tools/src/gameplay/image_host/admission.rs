//! The mounted accounting mode delegates to exactly one configured admission owner.

use std::sync::Arc;

use df_imagery::{
    AccountingMode, ImageJob,
    execution::{DispatchId, DispatchMode},
    publication::NoDeliveryReceipt,
    runtime::{
        DispatchAdmission, DispatchDecisionSettlement, DispatchNoOutputSettlement,
        ImageDispatchAdmission, RuntimeError, SendAuthorization,
    },
};
use df_persistence::imagery_publication::NativeImagePublisher;

use super::{NativeClock, NativeFileStore, NativeIds};
use crate::gameplay::image_hosted_authority::HostedAdmission;

pub(super) type HostPublisher = NativeImagePublisher<NativeFileStore, NativeIds, NativeClock>;

pub(super) enum MountedAdmission {
    Local(Arc<HostPublisher>),
    Hosted(Arc<HostedAdmission>),
}

impl MountedAdmission {
    pub(super) fn accounting(&self) -> AccountingMode {
        match self {
            Self::Local(_) => AccountingMode::RecordOnly,
            Self::Hosted(_) => AccountingMode::Hosted,
        }
    }
}

impl ImageDispatchAdmission for MountedAdmission {
    async fn admit(&self, input: &DispatchAdmission<'_>) -> Result<DispatchMode, RuntimeError> {
        match self {
            Self::Local(admission) => admission.admit(input).await,
            Self::Hosted(admission) => admission.admit(input).await,
        }
    }

    async fn mark_sent(
        &self,
        input: &DispatchAdmission<'_>,
        mode: DispatchMode,
    ) -> Result<SendAuthorization, RuntimeError> {
        match self {
            Self::Local(admission) => admission.mark_sent(input, mode).await,
            Self::Hosted(admission) => admission.mark_sent(input, mode).await,
        }
    }

    async fn outcome(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        mode: DispatchMode,
        usable: Option<bool>,
    ) -> Result<(), RuntimeError> {
        match self {
            Self::Local(admission) => admission.outcome(job, dispatch, mode, usable).await,
            Self::Hosted(admission) => admission.outcome(job, dispatch, mode, usable).await,
        }
    }

    async fn outcome_decision(
        &self,
        input: &DispatchDecisionSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        match self {
            Self::Local(admission) => admission.outcome_decision(input).await,
            Self::Hosted(admission) => admission.outcome_decision(input).await,
        }
    }

    async fn outcome_no_output(
        &self,
        input: &DispatchNoOutputSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        match self {
            Self::Local(admission) => admission.outcome_no_output(input).await,
            Self::Hosted(admission) => admission.outcome_no_output(input).await,
        }
    }

    async fn release_unused(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        receipt: &NoDeliveryReceipt,
    ) -> Result<(), RuntimeError> {
        match self {
            Self::Local(admission) => admission.release_unused(job, dispatch, receipt).await,
            Self::Hosted(admission) => admission.release_unused(job, dispatch, receipt).await,
        }
    }
}
