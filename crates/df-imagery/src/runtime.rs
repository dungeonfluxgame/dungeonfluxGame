//! Owned native execution behind ImageService. Claims and sends are durably fenced.

mod analysis;
mod artifact_basis;
mod auxiliary;
mod auxiliary_recovery;
mod classification;
mod decline_closure;
mod diagnostics;
mod dispatch;
pub(crate) mod inspection_policy;
mod inspection_projection;
mod lease;
mod no_output;
mod pipeline;
pub mod ports;
mod preflight;
mod preflight_rejection;
mod qualification;
mod quota;
pub(crate) mod quota_gate;
mod reconciliation;
mod recovery;
mod retry;
mod shared;
mod source_closure;
pub(crate) mod standalone;
mod terminal;
mod terminal_cleanup;
pub mod usage_projection;
mod usage_retention;
mod worker;

pub(crate) use artifact_basis::artifact_executor_basis;
pub use inspection_projection::{project_inspection_decision, project_local_inspection};
pub use no_output::{QualifiedNoOutput, qualify_no_output};
pub use ports::*;
pub use preflight::{PreflightCause, QualifiedPreflightRejected};
pub use qualification::{
    DecisionValidity, NoDeliveryReason, PublicationDecisionEvidence, PublicationEvidence,
    QualifiedDecision, QualifiedDemandEnded, QualifiedNoDelivery, QualifiedPipeline,
    QualifiedRequiredPreflight, qualify_demand_ended, qualify_economic_decision,
    qualify_no_delivery, qualify_publication, qualify_required_preflight,
};
pub use quota::{QuotaError, QuotaRegistry, QuotaSummary};
pub use standalone::{QualifiedAnalysis, QualifiedLocalAnalysis, analysis_result};
#[cfg(feature = "test-support")]
pub use standalone::{StandaloneUsageCapture, capture_standalone_usage};
pub use usage_projection::{
    ImageStageUsage, ImageUsageAmount, ImageUsageSummary, StageUsageObservation,
    UsageProjectionLimits,
};
pub use worker::{NativeImageRuntime, RuntimeHandle, RuntimeSettings};

use crate::execution::{
    CapacityScope, EnqueueCommand, ExecutionIdentity, ExecutionStageId, StageBasis, StageKind,
    StageScope,
};
use crate::intent_codec::DecodedIntent;
use crate::{AcceptCommand, AcceptTarget, ImageOperation};
use sha2::{Digest, Sha256};

/// Builds the execution reserve in the same transaction as accepted demand.
/// All ids are fresh host entropy; no account or finance identity is synthesized.
pub fn reservation_for(
    accept: &AcceptCommand,
    scope: CapacityScope,
    active_config_revision: u64,
    local_accounting: bool,
    ids: &impl RuntimeIds,
) -> Result<EnqueueCommand, crate::JournalError> {
    if crate::service::analysis_codec::is_analysis_intent(accept.intent.bytes()) {
        return standalone::reservation(
            accept,
            scope,
            active_config_revision,
            local_accounting,
            ids,
        );
    }
    let job = match accept.target {
        AcceptTarget::Create { job, .. } | AcceptTarget::Join { job } => job,
        AcceptTarget::JoinReady { .. } => return Err(crate::JournalError::InvalidRecord),
    };
    let intent = decode_protocol_intent(accept.intent.bytes())
        .map_err(|_| crate::JournalError::InvalidRecord)?;
    let config = decode_protocol_config(accept.config.bytes())
        .map_err(|_| crate::JournalError::InvalidRecord)?;
    if (config.accounting.mode == crate::AccountingMode::RecordOnly) != local_accounting {
        return Err(crate::JournalError::InvalidRecord);
    }
    let kind = if intent
        .operation
        .pipeline()
        .is_some_and(|pipeline| pipeline.creator.is_some())
    {
        StageKind::Classification
    } else {
        match intent.operation {
            ImageOperation::Generate(_) => StageKind::Generation,
            ImageOperation::Edit(_) => StageKind::Edit,
            ImageOperation::Transform(_) => StageKind::LocalTransform,
        }
    };
    EnqueueCommand::new(
        StageScope {
            owner: accept.identity.owner.clone(),
            job,
            stage: ExecutionStageId::from_bytes(&ids.next_bytes())
                .map_err(|_| crate::JournalError::InvalidRecord)?,
        },
        StageBasis {
            kind,
            input_revision: accept.intent.revision(),
            input_digest: Sha256::digest(accept.intent.bytes()).into(),
            config_revision: active_config_revision,
            config_digest: Sha256::digest(accept.config.bytes()).into(),
            parent_dispatch: None,
        },
        ExecutionIdentity::new(ids.next_bytes(), ids.next_bytes())
            .map_err(|_| crate::JournalError::InvalidRecord)?,
        scope,
        local_accounting || kind == StageKind::LocalTransform,
        accept.now_ms,
    )
    .map_err(|_| crate::JournalError::InvalidRecord)
}

/// Retained audience decoded under the fixed protocol bound for native catalog composition.
pub fn retained_audience(job: &crate::ImageJob) -> Result<crate::Audience, RuntimeError> {
    if crate::service::analysis_codec::is_analysis_intent(job.intent.bytes()) {
        return crate::service::analysis_codec::decode_analysis(job.intent.bytes())
            .map(|intent| intent.audience)
            .map_err(|_| RuntimeError::InvalidRecord);
    }
    retained_intent(job).map(|intent| intent.audience)
}

/// Retained validated configuration for native output and publication resource bounds.
pub fn retained_config(job: &crate::ImageJob) -> Result<crate::ImageConfig, RuntimeError> {
    decode_protocol_config(job.config.bytes())
}

/// The job's retained image intent, decoded under the fixed protocol bounds so a later
/// tightening of runtime limits never makes an accepted job unreadable.
pub(crate) fn retained_intent(job: &crate::ImageJob) -> Result<DecodedIntent, RuntimeError> {
    decode_protocol_intent(job.intent.bytes())
}

fn decode_protocol_intent(bytes: &[u8]) -> Result<DecodedIntent, RuntimeError> {
    crate::intent_codec::decode_intent(
        bytes,
        &crate::service::PROTOCOL_LIMITS,
        crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .map_err(|_| RuntimeError::InvalidRecord)
}

fn decode_protocol_config(bytes: &[u8]) -> Result<crate::ImageConfig, RuntimeError> {
    crate::intent_codec::decode_config(bytes, crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES)
        .map_err(|_| RuntimeError::InvalidRecord)
}

/// Decodes only the source facts needed for real cost maxima; prompts never leave this crate.
pub fn retained_cost_basis(
    job: &crate::ImageJob,
    config: &crate::ImageConfig,
    kind: StageKind,
) -> Result<RuntimeCostBasis, RuntimeError> {
    if crate::service::analysis_codec::is_analysis_intent(job.intent.bytes()) {
        return standalone::cost_basis(job, config);
    }
    let intent = retained_intent(job)?;
    let (prompt_bytes, reference_count) = match &intent.operation {
        ImageOperation::Generate(spec) => (spec.prompt.text().len(), spec.references.len()),
        ImageOperation::Edit(spec) => (spec.prompt.text().len(), spec.references.len() + 1),
        ImageOperation::Transform(_) => (0, 0),
    };
    let analysis = intent.operation.pipeline().and_then(|pipeline| match kind {
        StageKind::Classification => pipeline
            .creator
            .as_ref()
            .map(|creator| creator.route.clone()),
        StageKind::Inspection => pipeline
            .inspection
            .as_ref()
            .map(|inspection| inspection.route.clone()),
        _ => None,
    });
    let model = analysis
        .as_ref()
        .map(|route| route.model.clone())
        .map_or_else(
            || {
                dispatch::model(config, &intent).and_then(|model| {
                    crate::ModelName::new(&model).map_err(|_| RuntimeError::InvalidConfiguration)
                })
            },
            Ok,
        )?;
    let template_version = match &intent.operation {
        ImageOperation::Generate(spec) => spec.prompt.template_version,
        ImageOperation::Edit(spec) => spec.prompt.template_version,
        ImageOperation::Transform(_) => 0,
    };
    Ok(RuntimeCostBasis {
        purpose: intent.purpose,
        kind,
        backend: if analysis.is_some() {
            None
        } else {
            Some(dispatch::backend(config, &intent))
        },
        model,
        template_version,
        prompt_bytes: u64::try_from(prompt_bytes).map_err(|_| RuntimeError::InvalidRecord)?,
        reference_count: u64::try_from(reference_count).map_err(|_| RuntimeError::InvalidRecord)?,
        generated: intent.output.generated(),
        generation_quality: if kind == StageKind::LocalTransform {
            None
        } else {
            config
                .purpose(intent.purpose.purpose)
                .quality_override
                .or(config.quality.provider_quality)
        },
        maximum_output_tokens: analysis.as_ref().map(|route| route.maximum_output_tokens),
        analysis,
    })
}

/// Exact retained source pins, including edit bases, coverage evidence, masks and layers.
pub fn retained_sources(job: &crate::ImageJob) -> Result<Vec<crate::SourceRef>, RuntimeError> {
    if crate::service::analysis_codec::is_analysis_intent(job.intent.bytes()) {
        return crate::service::analysis_codec::decode_analysis(job.intent.bytes())
            .map(|intent| intent.operation.sources().into_iter().cloned().collect())
            .map_err(|_| RuntimeError::InvalidRecord);
    }
    let intent = retained_intent(job)?;
    Ok(dispatch::references(&intent.operation)
        .into_iter()
        .cloned()
        .collect())
}

/// Pipeline decisions and separately metered fal extraction cannot use legacy direct publication.
pub fn requires_final_selection(job: &crate::ImageJob) -> Result<bool, RuntimeError> {
    let intent = retained_intent(job)?;
    let config = retained_config(job)?;
    Ok(intent.operation.pipeline().is_some()
        || (dispatch::backend(&config, &intent) == crate::BackendId::Fal
            && matches!(
                intent.output.delivered().alpha(),
                crate::AlphaHandling::PreserveAlpha
            )))
}

/// Canonical full consumed-source fingerprint, including current policy class and exact versions.
pub fn source_receipts_digest(sources: &[SourceReceipt]) -> Result<[u8; 32], RuntimeError> {
    let mut digest = Sha256::new();
    digest.update(b"df-imagery/consumed-sources-v1");
    digest.update(
        u64::try_from(sources.len())
            .map_err(|_| RuntimeError::InvalidRecord)?
            .to_be_bytes(),
    );
    for source in sources {
        digest.update(source.pin.asset.as_bytes());
        digest.update(source.pin.digest.as_bytes());
        let mut audience = Vec::new();
        source.pin.audience.encode_into(&mut audience);
        for bytes in [
            audience.as_slice(),
            source.version.as_str().as_bytes(),
            source.retention.label().as_bytes(),
        ] {
            digest.update(
                u64::try_from(bytes.len())
                    .map_err(|_| RuntimeError::InvalidRecord)?
                    .to_be_bytes(),
            );
            digest.update(bytes);
        }
        digest.update(source.attempt.as_bytes());
        digest.update(source.write.as_bytes());
        digest.update(source.suppression_generation.to_be_bytes());
    }
    Ok(digest.finalize().into())
}

/// Image stages require the actual planned media pair; analytical stages carry none.
pub fn planned_publication(
    stage: &crate::execution::StageView,
) -> Result<crate::execution::PublicationIdentity, RuntimeError> {
    stage.planned_publication.ok_or(RuntimeError::InvalidRecord)
}

/// Exact canonical analytical handles consumed by a raster derivative or standalone operation.
pub fn retained_analysis_dependencies(
    job: &crate::ImageJob,
) -> Result<Vec<(crate::OperationHandle, crate::SourceRef)>, RuntimeError> {
    if crate::service::analysis_codec::is_analysis_intent(job.intent.bytes()) {
        let intent = crate::service::analysis_codec::decode_analysis(job.intent.bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        let source = intent
            .operation
            .sources()
            .first()
            .cloned()
            .cloned()
            .ok_or(RuntimeError::InvalidRecord)?;
        return Ok(intent
            .operation
            .dependencies()
            .into_iter()
            .map(|handle| (handle.clone(), source.clone()))
            .collect());
    }
    let intent = retained_intent(job)?;
    if let crate::ImageOperation::Transform(crate::TransformSpec {
        source:
            crate::TransformSource::SegmentationMask {
                segmentation,
                analyzed,
                ..
            },
        ..
    }) = intent.operation
    {
        Ok(vec![(segmentation, analyzed)])
    } else {
        Ok(Vec::new())
    }
}
