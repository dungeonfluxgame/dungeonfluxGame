//! Shared infrastructure ownership forwards to the same live adapter and authority.
use super::ports::*;
use crate::execution::{DispatchId, DispatchMode, OutputCheckpoint};
use crate::publication::{FinalOutputSelection, PublishedImage};
use crate::{AccountHandle, BackendId, ImageJob, ImageOwner, SourceRef};
use std::sync::Arc;

impl<P: ImagePublisher> ImagePublisher for Arc<P> {
    async fn no_delivery_if_sources_denied(
        &self,
        terminal: &crate::execution::TerminalCommand,
    ) -> Result<crate::publication::SourceClosure, RuntimeError> {
        self.as_ref().no_delivery_if_sources_denied(terminal).await
    }
    async fn read_no_delivery(
        &self,
        owner: &ImageOwner,
        job: &crate::ImageJobId,
    ) -> Result<Option<crate::publication::NoDeliveryReceipt>, RuntimeError> {
        self.as_ref().read_no_delivery(owner, job).await
    }
    async fn no_delivery(
        &self,
        request: &crate::publication::NoDeliveryRequest<'_>,
    ) -> Result<crate::publication::NoDeliveryReceipt, RuntimeError> {
        self.as_ref().no_delivery(request).await
    }

    async fn image_economic_use(
        &self,
        scope: &crate::execution::ExecutionScope,
    ) -> Result<Option<ImageEconomicDisposition>, RuntimeError> {
        self.as_ref().image_economic_use(scope).await
    }

    async fn select_output(
        &self,
        context: &PublicationContext,
        selection: &FinalOutputSelection,
    ) -> Result<(), RuntimeError> {
        self.as_ref().select_output(context, selection).await
    }
    async fn store_output(
        &self,
        context: &PublicationContext,
        output: &PreparedImageOutput,
    ) -> Result<OutputCheckpoint, RuntimeError> {
        self.as_ref().store_output(context, output).await
    }
    async fn publish(&self, context: &PublicationContext) -> Result<PublishedImage, RuntimeError> {
        self.as_ref().publish(context).await
    }
}
impl<F: ImageDispatchAdmission> ImageDispatchAdmission for Arc<F> {
    async fn admit_with_artifact(
        &self,
        input: &DispatchAdmission<'_>,
    ) -> Result<ArtifactAdmission, RuntimeError> {
        self.as_ref().admit_with_artifact(input).await
    }

    async fn outcome_no_output(
        &self,
        input: &DispatchNoOutputSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        self.as_ref().outcome_no_output(input).await
    }
    async fn release_unused(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        receipt: &crate::publication::NoDeliveryReceipt,
    ) -> Result<(), RuntimeError> {
        self.as_ref().release_unused(job, dispatch, receipt).await
    }
    async fn outcome_decision(
        &self,
        input: &DispatchDecisionSettlement<'_>,
    ) -> Result<(), RuntimeError> {
        self.as_ref().outcome_decision(input).await
    }
    async fn admit(&self, input: &DispatchAdmission<'_>) -> Result<DispatchMode, RuntimeError> {
        self.as_ref().admit(input).await
    }
    async fn mark_sent(
        &self,
        input: &DispatchAdmission<'_>,
        mode: DispatchMode,
    ) -> Result<SendAuthorization, RuntimeError> {
        self.as_ref().mark_sent(input, mode).await
    }
    async fn outcome(
        &self,
        job: &ImageJob,
        dispatch: DispatchId,
        mode: DispatchMode,
        usable: Option<bool>,
    ) -> Result<(), RuntimeError> {
        self.as_ref().outcome(job, dispatch, mode, usable).await
    }
}
impl<S: ImageSources> ImageSources for Arc<S> {
    async fn read_analysis(
        &self,
        owner: &ImageOwner,
        handle: &crate::OperationHandle,
    ) -> Result<crate::ImageAnalysisObservation, RuntimeError> {
        self.as_ref().read_analysis(owner, handle).await
    }
    async fn read_source(
        &self,
        owner: &ImageOwner,
        pin: &SourceRef,
    ) -> Result<AuthorizedImageSource, RuntimeError> {
        self.as_ref().read_source(owner, pin).await
    }
}
impl<K: ImageCredentials> ImageCredentials for Arc<K> {
    fn quota_registry(&self) -> Option<Arc<super::QuotaRegistry>> {
        self.as_ref().quota_registry()
    }
    fn quota_identity(&self, backend: BackendId, account: &AccountHandle) -> Option<[u8; 32]> {
        self.as_ref().quota_identity(backend, account)
    }
    fn analysis_credential(&self, account: &AccountHandle) -> Result<String, RuntimeError> {
        self.as_ref().analysis_credential(account)
    }
    fn credential(
        &self,
        backend: BackendId,
        account: &AccountHandle,
    ) -> Result<String, RuntimeError> {
        self.as_ref().credential(backend, account)
    }
}
