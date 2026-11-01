//! Read-only analysis dependencies use current host grants and explicitly selected durable results.
use super::{
    image_host::NativeIds,
    image_sources::{AnalysisRead, AnalysisReadFuture, CatalogImageSources},
    imagery_identity::{ExactAssetAllowlist, ImageryAuthorizer},
};
use df_assets::NativeFileStore;
use df_imagery::runtime::{ImageSources, RuntimeError};
use df_imagery::{
    ImageAnalysisObservation, ImageAnalysisResult, ImageJournal, ImageObservations, ImageOwner,
    OperationHandle, OperationView, ServiceAction, SourceRef,
};
use df_persistence::imagery_runtime::{PostgresImageObservations, PostgresRuntimeJournal};
use std::sync::Weak;

pub(super) struct HostAnalysisReader {
    journal: PostgresRuntimeJournal<NativeIds>,
    observations: PostgresImageObservations,
    sources: Weak<CatalogImageSources<NativeFileStore>>,
}
impl HostAnalysisReader {
    pub(super) fn new(
        journal: PostgresRuntimeJournal<NativeIds>,
        observations: PostgresImageObservations,
        sources: Weak<CatalogImageSources<NativeFileStore>>,
    ) -> Self {
        Self {
            journal,
            observations,
            sources,
        }
    }
    async fn read_inner(
        &self,
        owner: &ImageOwner,
        handle: &OperationHandle,
    ) -> Result<ImageAnalysisObservation, RuntimeError> {
        if owner != &handle.owner {
            return Err(RuntimeError::SourceDenied);
        }
        let sources = self.sources.upgrade().ok_or(RuntimeError::Cancelled)?;
        self.authorize_owner(&sources, owner).await?;
        let view = self
            .journal
            .lookup_operation(owner, handle.operation)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::SourceDenied)?;
        validate_linkage(owner, handle, &view)?;
        let job = view.job.clone();
        let (pins, audience) = tokio::task::spawn_blocking(move || {
            Ok::<_, RuntimeError>((
                df_imagery::runtime::retained_sources(&job)?,
                df_imagery::runtime::retained_audience(&job)?,
            ))
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)??;
        for pin in &pins {
            let source = sources.read_source(owner, pin).await?;
            if source.receipt.pin != *pin {
                return Err(RuntimeError::InvalidRecord);
            }
        }
        let observed = self
            .observations
            .read_analysis(owner, view.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::SourceDenied)?;
        if observed.job_revision != view.job.revision {
            return Err(RuntimeError::SourceDenied);
        }
        if pins.first() != Some(result_source(&observed.result)) {
            return Err(RuntimeError::InvalidRecord);
        }
        // The final current-grant and catalog locks linearize authorization after all object reads.
        // No source reader is re-entered while holding this connection mutex.
        let mut connection = sources.authority.lock().await;
        let transaction = connection
            .transaction()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let principal = sources.grants.resolve(&transaction, owner).await?;
        ImageryAuthorizer::new(ExactAssetAllowlist::default())
            .authorize_audience(&principal, owner, &audience)
            .map_err(|_| RuntimeError::SourceDenied)?;
        df_persistence::imagery_publication::lock_image_sources_in(&transaction, &pins, &audience)
            .await
            .map_err(|error| match error {
                df_imagery::publication::PublicationError::Unavailable
                | df_imagery::publication::PublicationError::Unknown => {
                    RuntimeError::StorageUnavailable
                }
                _ => RuntimeError::SourceDenied,
            })?;
        sources
            .grants
            .authorize_catalog_sources(&transaction, owner, &pins)
            .await?;
        let current = self
            .journal
            .lookup_operation(owner, handle.operation)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::SourceDenied)?;
        validate_linkage(owner, handle, &current)?;
        if current.job.revision != observed.job_revision || current.receipt != view.receipt {
            return Err(RuntimeError::SourceDenied);
        }
        transaction
            .commit()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        Ok(observed)
    }
    async fn authorize_owner(
        &self,
        sources: &CatalogImageSources<NativeFileStore>,
        owner: &ImageOwner,
    ) -> Result<(), RuntimeError> {
        let mut connection = sources.authority.lock().await;
        let transaction = connection
            .transaction()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let principal = sources.grants.resolve(&transaction, owner).await?;
        df_imagery::ImageAuthorizer::authorize_owner(
            &ImageryAuthorizer::new(ExactAssetAllowlist::default()),
            &principal,
            owner,
            ServiceAction::Read,
        )
        .map_err(|_| RuntimeError::SourceDenied)?;
        transaction
            .commit()
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)
    }
}
impl AnalysisRead for HostAnalysisReader {
    fn read<'a>(
        &'a self,
        owner: &'a ImageOwner,
        handle: &'a OperationHandle,
    ) -> AnalysisReadFuture<'a> {
        Box::pin(self.read_inner(owner, handle))
    }
}
fn validate_linkage(
    owner: &ImageOwner,
    handle: &OperationHandle,
    view: &OperationView,
) -> Result<(), RuntimeError> {
    if view.receipt.owner != *owner
        || view.job.owner != *owner
        || view.interest.owner != *owner
        || view.receipt.operation != handle.operation
        || view.interest.operation != handle.operation
        || view.receipt.job != view.job.job
        || view.receipt.interest != view.interest.interest
        || view.interest.job != view.job.job
    {
        return Err(RuntimeError::InvalidRecord);
    }
    if view.interest.state != df_imagery::InterestState::Attached
        || view.job.state == df_imagery::JobState::Retired
    {
        return Err(RuntimeError::SourceDenied);
    }
    Ok(())
}
fn result_source(result: &ImageAnalysisResult) -> &SourceRef {
    match result {
        ImageAnalysisResult::PhotoInspection { source, .. }
        | ImageAnalysisResult::HeadLocations { source, .. }
        | ImageAnalysisResult::Segmentation { source, .. }
        | ImageAnalysisResult::StagingIdentity { source, .. }
        | ImageAnalysisResult::StagingFacing { source, .. } => source,
    }
}
