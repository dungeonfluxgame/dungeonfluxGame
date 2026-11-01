//! Consumer checks complement the native current-grant/catalog proof without adopting old demand.
use df_assets::{AssetVariant, ImageAlphaRequirement, ImageIntent};

use super::{
    ImageAuthorizer, ImageJournal, ImageObservations, ImageService, ImageServiceError,
    ServiceClock, ServiceIds,
};
use crate::publication::{ImageResults, PublicationError, ReadyArtifact};
use crate::{AlphaHandling, ImageOperation, ImageRequest, OutputProfile, VersionedSnapshot};
use df_assets::Audience;

impl<J, C, I, A, R, O> ImageService<J, C, I, A, R, O>
where
    J: ImageJournal,
    C: ServiceClock,
    I: ServiceIds,
    A: ImageAuthorizer,
    R: ImageResults,
    O: ImageObservations,
{
    pub(super) async fn find_current_artifact(
        &self,
        request: &ImageRequest,
        config: &VersionedSnapshot,
    ) -> Result<Option<ReadyArtifact>, ImageServiceError> {
        let Some(recipe) = super::artifact_recipe::campaign_artifact_recipe(request, config)
            .map_err(|_| ImageServiceError::InvalidRecord)?
        else {
            return Ok(None);
        };
        let Some(artifact) = self
            .results
            .find_ready_artifact(request, config, self.clock.now_ms())
            .await
            .map_err(map_artifact_error)?
        else {
            return Ok(None);
        };
        verify_current_artifact(request, &recipe, &artifact)?;
        Ok(Some(artifact))
    }

    pub(super) async fn project_owned_artifact(
        &self,
        handle: &super::OperationHandle,
        view: &crate::OperationView,
        demand: &mut super::ImageDemandView,
    ) -> Result<(), ImageServiceError> {
        if !super::final_result_visible(view)
            || demand.result.is_some()
            || demand.analysis_result.is_some()
            || demand.failure.is_some()
            || demand
                .usage
                .as_ref()
                .is_some_and(|usage| !usage.stages.is_empty())
            || matches!(
                demand.execution,
                crate::ImageExecutionState::Unknown
                    | crate::ImageExecutionState::Uncertain
                    | crate::ImageExecutionState::Failed { .. }
            )
        {
            return Ok(());
        }
        let Some(recipe) = super::artifact_recipe::retained_campaign_artifact_recipe(
            &view.job.intent,
            &view.job.config,
        )
        .map_err(|_| ImageServiceError::InvalidRecord)?
        else {
            return Ok(());
        };
        let Some(artifact) = self
            .results
            .read_artifact(&handle.owner, &view.job.job, self.clock.now_ms())
            .await
            .map_err(|error| match error {
                PublicationError::Unknown => ImageServiceError::Unknown {
                    handle: handle.clone(),
                },
                error => map_artifact_error(error),
            })?
        else {
            return Ok(());
        };
        let current = self
            .journal
            .lookup_operation(&handle.owner, handle.operation)
            .await
            .map_err(|error| super::map_lookup_error(error, handle))?
            .ok_or(ImageServiceError::InvalidRecord)?;
        super::validate_linkage(handle, &current, &handle.owner)?;
        if current.job.job != view.job.job
            || current.receipt.interest != view.receipt.interest
            || current.job.revision != view.job.revision
            || !super::final_result_visible(&current)
        {
            return Err(ImageServiceError::Unknown {
                handle: handle.clone(),
            });
        }
        let stored = crate::intent_codec::decode_intent(
            view.job.intent.bytes(),
            &super::PROTOCOL_LIMITS,
            super::PROTOCOL_MAX_SNAPSHOT_BYTES,
        )
        .map_err(|_| ImageServiceError::InvalidRecord)?;
        verify_artifact_parts(
            &stored.operation,
            &stored.audience,
            stored.output,
            &recipe,
            &artifact,
        )?;
        demand.inspection = artifact.inspection.clone();
        demand.staging = artifact.staging.clone();
        demand.artifact = Some(artifact);
        demand.execution = crate::ImageExecutionState::Ready;
        demand.operational_notice = None;
        Ok(())
    }
}

fn map_artifact_error(error: PublicationError) -> ImageServiceError {
    match error {
        PublicationError::Unavailable | PublicationError::Unknown => ImageServiceError::Unavailable,
        _ => ImageServiceError::InvalidRecord,
    }
}

pub(super) fn verify_current_artifact(
    request: &ImageRequest,
    recipe: &super::artifact_recipe::CampaignArtifactRecipe,
    artifact: &ReadyArtifact,
) -> Result<(), ImageServiceError> {
    verify_artifact_parts(
        &request.operation,
        &request.audience,
        request.output,
        recipe,
        artifact,
    )
}

pub(super) fn verify_artifact_parts(
    operation: &ImageOperation,
    audience: &Audience,
    output: OutputProfile,
    recipe: &super::artifact_recipe::CampaignArtifactRecipe,
    artifact: &ReadyArtifact,
) -> Result<(), ImageServiceError> {
    if &artifact.audience != audience
        || artifact.semantic_key != recipe.key
        || artifact.published_at_ms <= 0
        || artifact.variants.len() != 1
    {
        return Err(ImageServiceError::InvalidRecord);
    }
    let delivered = output.delivered();
    let dimensions = delivered.dimensions();
    let expected = ImageIntent::new(
        delivered.encoding().format().mime(),
        dimensions.width_px(),
        dimensions.height_px(),
        if matches!(delivered.alpha(), AlphaHandling::PreserveAlpha) {
            ImageAlphaRequirement::Preserve
        } else {
            ImageAlphaRequirement::Opaque
        },
    )
    .map_err(|_| ImageServiceError::InvalidRecord)?;
    let actual = &artifact.variants[0];
    if actual.variant != AssetVariant::Master
        || actual.byte_len == 0
        || !expected.accepts(actual.intent, actual.alpha)
    {
        return Err(ImageServiceError::InvalidRecord);
    }
    let inspection = operation
        .pipeline()
        .and_then(|pipeline| pipeline.inspection.as_ref());
    let expected_staging = inspection.map_or(&[][..], |inspection| inspection.staging.as_slice());
    if artifact.staging != expected_staging {
        return Err(ImageServiceError::InvalidRecord);
    }
    if let Some(inspection) = inspection {
        if artifact
            .inspection
            .as_ref()
            .is_some_and(|actual| actual.contract_revision != inspection.contract_revision)
            || (inspection.requirement == crate::InspectionRequirement::Required
                && !artifact
                    .inspection
                    .as_ref()
                    .is_some_and(|actual| actual.verdict == crate::PhotoInspectionVerdict::Pass))
        {
            return Err(ImageServiceError::InvalidRecord);
        }
    } else if artifact.inspection.is_some() {
        return Err(ImageServiceError::InvalidRecord);
    }
    Ok(())
}
