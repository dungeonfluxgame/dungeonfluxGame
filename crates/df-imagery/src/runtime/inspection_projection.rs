//! Audience-safe projections of the actual immutable winning inspection receipt.
use super::{RuntimeError, analysis, inspection_policy};
use crate::execution::{
    DecisionKind, DecisionObservation, LocalInspectionCheckpoint, LocalInspectionReason,
};
use crate::{ImageJob, PhotoInspectionAssessment};
use sha2::{Digest, Sha256};

pub fn project_inspection_decision(
    job: &ImageJob,
    decision: &DecisionObservation,
) -> Result<PhotoInspectionAssessment, RuntimeError> {
    if decision.scope.owner != job.owner
        || decision.scope.job != job.job
        || decision.kind != DecisionKind::Inspection
        || decision.basis.stage.config_digest
            != <[u8; 32]>::from(Sha256::digest(job.config.bytes()))
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let spec = inspection_spec(job)?;
    let value = analysis::parse_response(decision.payload(), decision.payload().len())?;
    let assessment = inspection_policy::InspectionAssessment::from_value(&value, &spec)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    Ok(crate::backends::analysis::standalone::project_photo_assessment(&assessment))
}
pub fn project_local_inspection(
    job: &ImageJob,
    local: &LocalInspectionCheckpoint,
) -> Result<PhotoInspectionAssessment, RuntimeError> {
    let spec = inspection_spec(job)?;
    if local.scope.owner != job.owner
        || local.scope.job != job.job
        || spec.requirement != crate::InspectionRequirement::BestEffort
        || local.basis.config_digest != <[u8; 32]>::from(Sha256::digest(job.config.bytes()))
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let reason = match local.reason {
        LocalInspectionReason::Preparation => inspection_policy::UnjudgedReason::Preparation,
        LocalInspectionReason::Disabled => inspection_policy::UnjudgedReason::Disabled,
    };
    let assessment = inspection_policy::InspectionAssessment::unjudged(&spec, reason);
    Ok(crate::backends::analysis::standalone::project_photo_assessment(&assessment))
}
fn inspection_spec(job: &ImageJob) -> Result<crate::ImageInspection, RuntimeError> {
    let config = super::retained_config(job)?;
    let intent = crate::intent_codec::decode_intent(
        job.intent.bytes(),
        &config.request_limits,
        crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| RuntimeError::InvalidRecord)?;
    let spec = intent
        .operation
        .pipeline()
        .and_then(|pipeline| pipeline.inspection.as_ref())
        .cloned()
        .ok_or(RuntimeError::InvalidRecord)?;
    if config.analysis.inspection.as_ref() != Some(&spec.route) {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(spec)
}
