//! A known unusable supplier result is replayed before it can release analytical liability.
use super::*;

pub(in crate::runtime) fn validate_standalone_failure(
    job: &ImageJob,
    dispatch: &DispatchView,
    basis: &StageBasis,
    usage: &RuntimeUsage,
) -> Result<(), RuntimeError> {
    let intent = analysis_codec::decode_analysis(job.intent.bytes())
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let config = super::retained_config(job)?;
    if intent.owner != job.owner
        || dispatch.owner != job.owner
        || dispatch.job != job.job
        || dispatch.stage != usage.stage
        || dispatch.dispatch != usage.dispatch
        || dispatch.state != DispatchState::Failed
        || u64::from(dispatch.usage_revision) != usage.revision.0
        || usage.owner != job.owner
        || usage.job != job.job
        || usage.evidence == UsageEvidence::Unknown
        || usage.usable
        || basis.kind != kind(&intent.operation)
        || basis.input_revision != job.intent.revision()
        || basis.input_digest != <[u8; 32]>::from(Sha256::digest(job.intent.bytes()))
        || basis.config_digest != <[u8; 32]>::from(Sha256::digest(job.config.bytes()))
        || usage.route.model != cost_basis(job, &config)?.model.as_str()
        || analysis_codec::sources(&intent.operation)
            .iter()
            .any(|source| intent.audience.meet(&source.audience) != intent.audience)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let (response, dependencies, previous, prompt_index) = decode_raw(
        usage
            .raw_output
            .as_deref()
            .ok_or(RuntimeError::InvalidRecord)?,
    )?;
    validate_dependency_shape(&intent.operation, &dependencies)?;
    validate_prompt_shape(
        &intent.operation,
        basis,
        &response,
        previous.as_ref(),
        prompt_index,
    )?;
    if backend::prepare_local_analysis(
        &intent.operation,
        dependencies.segmentation.as_ref(),
        dependencies.locations.as_ref(),
        dependencies.identity.as_ref(),
    )?
    .is_some()
    {
        return Err(RuntimeError::InvalidRecord);
    }
    if let Some((width, height)) = response.segmentation
        && (width == 0
            || height == 0
            || u64::from(width) * u64::from(height)
                > config.analysis.segmentation_preparation.max_pixels)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    if response.status().is_none() {
        return Err(RuntimeError::InvalidRecord);
    }
    super::codec::validate_raw_usage(&response, usage)?;
    interpret(
        &response,
        &intent,
        &dependencies,
        previous.as_ref(),
        prompt_index,
        response.body().len(),
    )
    .err()
    .ok_or(RuntimeError::InvalidRecord)?;
    let cause = super::codec::response_failure(&response);
    if dispatch.cause != Some(cause) {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(())
}

fn validate_dependency_shape(
    operation: &ImageAnalysisOperation,
    dependencies: &Dependencies,
) -> Result<(), RuntimeError> {
    let source = analysis_codec::source(operation);
    let segmentation = matches!(
        &dependencies.segmentation,
        Some(ImageAnalysisResult::Segmentation { source: pin, .. }) if pin == source
    );
    let locations = matches!(
        &dependencies.locations,
        Some(ImageAnalysisResult::HeadLocations { source: pin, .. }) if pin == source
    );
    let identity = matches!(
        &dependencies.identity,
        Some(ImageAnalysisResult::StagingIdentity { source: pin, .. }) if pin == source
    );
    let valid = match operation {
        ImageAnalysisOperation::StagingIdentity(spec) => {
            segmentation
                && if spec.locations.is_some() {
                    locations
                } else {
                    dependencies.locations.is_none()
                }
                && dependencies.identity.is_none()
        }
        ImageAnalysisOperation::StagingFacing(_) => {
            segmentation && identity && dependencies.locations.is_none()
        }
        _ => *dependencies == Dependencies::default(),
    };
    if !valid {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(())
}

fn validate_prompt_shape(
    operation: &ImageAnalysisOperation,
    basis: &StageBasis,
    response: &RawAnalysis,
    previous: Option<&ImageAnalysisResult>,
    prompt_index: usize,
) -> Result<(), RuntimeError> {
    let ImageAnalysisOperation::Segmentation(spec) = operation else {
        return if prompt_index == 0
            && previous.is_none()
            && basis.parent_dispatch.is_none()
            && response.segmentation.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeError::InvalidRecord)
        };
    };
    if prompt_index >= spec.prompts.len()
        || response.segmentation.is_none()
        || basis.parent_dispatch.is_some() != (prompt_index > 0)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    match previous {
        None if prompt_index == 0 => Ok(()),
        Some(ImageAnalysisResult::Segmentation {
            source,
            contract_revision,
            masks,
        }) if source == &spec.source
            && *contract_revision == spec.contract_revision
            && masks.len() == prompt_index
            && masks.iter().zip(&spec.prompts).all(|(mask, prompt)| {
                mask.prompt == *prompt
                    && Some((mask.width_px, mask.height_px)) == response.segmentation
                    && mask.instances.len() <= spec.maximum_masks as usize
            }) =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRecord),
    }
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;
