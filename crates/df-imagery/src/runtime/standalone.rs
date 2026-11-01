//! Owned standalone analysis execution. Decisions retain sources and raw usage without publishing media.
use super::{
    DecisionValidity, PublicationDecisionEvidence, analysis, inspection_policy, ports::*,
    retained_config, worker::NativeImageRuntime,
};
use crate::backends::analysis::standalone as backend;
use crate::cost::{ImageCostProjection, UsageEvidence};
use crate::execution::*;
use crate::service::analysis_codec::{self, DecodedAnalysis};
use crate::service::analysis_types::*;
use crate::{ImageConfig, ImageJob, ImageJournal, ServiceClock};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub(crate) fn kind(operation: &ImageAnalysisOperation) -> StageKind {
    match operation {
        ImageAnalysisOperation::PhotoInspection(_) => StageKind::PhotoInspection,
        ImageAnalysisOperation::HeadLocations(_) => StageKind::HeadLocations,
        ImageAnalysisOperation::Segmentation(_) => StageKind::Segmentation,
        ImageAnalysisOperation::StagingIdentity(_) => StageKind::StagingIdentity,
        ImageAnalysisOperation::StagingFacing(_) => StageKind::StagingFacing,
    }
}
fn decision_kind(operation: &ImageAnalysisOperation) -> DecisionKind {
    match operation {
        ImageAnalysisOperation::PhotoInspection(_) => DecisionKind::PhotoInspection,
        ImageAnalysisOperation::HeadLocations(_) => DecisionKind::HeadLocations,
        ImageAnalysisOperation::Segmentation(_) => DecisionKind::Segmentation,
        ImageAnalysisOperation::StagingIdentity(_) => DecisionKind::StagingIdentity,
        ImageAnalysisOperation::StagingFacing(_) => DecisionKind::StagingFacing,
    }
}
pub(crate) fn cost_basis(
    job: &ImageJob,
    config: &ImageConfig,
) -> Result<RuntimeCostBasis, RuntimeError> {
    let intent = analysis_codec::decode_analysis(job.intent.bytes())
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let (model, route) = match &intent.operation {
        ImageAnalysisOperation::Segmentation(_) => (
            config
                .analysis
                .segmentation
                .as_ref()
                .ok_or(RuntimeError::InvalidConfiguration)?
                .model
                .clone(),
            None,
        ),
        _ => {
            let route = chat_route(config, &intent.operation)?;
            (route.model.clone(), Some(route.clone()))
        }
    };
    Ok(RuntimeCostBasis {
        purpose: intent.purpose,
        kind: kind(&intent.operation),
        backend: matches!(&intent.operation, ImageAnalysisOperation::Segmentation(_))
            .then_some(crate::BackendId::Fal),
        model,
        template_version: analysis_codec::revision(&intent.operation),
        prompt_bytes: 0,
        reference_count: u64::try_from(analysis_codec::sources(&intent.operation).len())
            .map_err(|_| RuntimeError::InvalidRecord)?,
        generated: None,
        generation_quality: None,
        maximum_output_tokens: route.as_ref().map(|route| route.maximum_output_tokens),
        analysis: route,
    })
}
pub(crate) fn reservation(
    accept: &crate::AcceptCommand,
    capacity: CapacityScope,
    config_revision: u64,
    local_accounting: bool,
    ids: &impl RuntimeIds,
) -> Result<EnqueueCommand, crate::JournalError> {
    let intent = analysis_codec::decode_analysis(accept.intent.bytes())
        .map_err(|_| crate::JournalError::InvalidRecord)?;
    let job = match accept.target {
        crate::AcceptTarget::Create { job, .. } | crate::AcceptTarget::Join { job } => job,
        crate::AcceptTarget::JoinReady { .. } => return Err(crate::JournalError::InvalidRecord),
    };
    EnqueueCommand::new(
        StageScope {
            owner: accept.identity.owner.clone(),
            job,
            stage: ExecutionStageId::from_bytes(&ids.next_bytes())
                .map_err(|_| crate::JournalError::InvalidRecord)?,
        },
        StageBasis {
            kind: kind(&intent.operation),
            input_revision: accept.intent.revision(),
            input_digest: Sha256::digest(accept.intent.bytes()).into(),
            config_revision,
            config_digest: Sha256::digest(accept.config.bytes()).into(),
            parent_dispatch: None,
        },
        None,
        capacity,
        local_accounting,
        accept.now_ms,
    )
    .map_err(|_| crate::JournalError::InvalidRecord)
}
fn chat_route<'a>(
    config: &'a ImageConfig,
    operation: &ImageAnalysisOperation,
) -> Result<&'a crate::AnalysisRoute, RuntimeError> {
    match operation {
        ImageAnalysisOperation::PhotoInspection(spec)
            if config.analysis.inspection.as_ref() == Some(&spec.inspection.route) =>
        {
            config.analysis.inspection.as_ref()
        }
        ImageAnalysisOperation::HeadLocations(_) => config.analysis.head_locations.as_ref(),
        ImageAnalysisOperation::StagingIdentity(_) => config.analysis.staging_identity.as_ref(),
        ImageAnalysisOperation::StagingFacing(_) => config.analysis.staging_facing.as_ref(),
        ImageAnalysisOperation::Segmentation(_) | ImageAnalysisOperation::PhotoInspection(_) => {
            None
        }
    }
    .ok_or(RuntimeError::InvalidConfiguration)
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Dependencies {
    segmentation: Option<ImageAnalysisResult>,
    locations: Option<ImageAnalysisResult>,
    identity: Option<ImageAnalysisResult>,
}
struct Inputs {
    source: Vec<u8>,
    face: Option<Vec<u8>>,
    references: Vec<(crate::SourceRef, Vec<u8>)>,
    receipts: Vec<SourceReceipt>,
    dependencies: Dependencies,
    dependency_refs: Vec<AnalysisDependencyRef>,
}
struct ResolvedDependency {
    result: ImageAnalysisResult,
    reference: AnalysisDependencyRef,
}
enum Prepared {
    Photo(backend::PreparedPhotoInspection),
    Chat(backend::PreparedStandaloneChat),
    Segmentation(backend::PreparedSegmentation),
    Local(ImageAnalysisResult),
}
impl Prepared {
    fn request_facts(&self) -> Result<(u64, u64), RuntimeError> {
        match self {
            Self::Photo(photo) => Ok((photo.text_bytes, photo.picture_count)),
            Self::Chat(chat) => Ok((chat.text_bytes, chat.picture_count)),
            Self::Segmentation(segmentation) => {
                Ok((segmentation.text_bytes, segmentation.picture_count))
            }
            Self::Local(_) => Err(RuntimeError::InvalidRecord),
        }
    }
}
mod codec;
mod completion;
mod failure;
pub(in crate::runtime) use failure::validate_standalone_failure;
mod inputs;
mod qualification;
mod recovery;
use qualification::verdict;
pub use qualification::{QualifiedAnalysis, QualifiedLocalAnalysis, analysis_result};
#[cfg(feature = "test-support")]
mod test_capture;
mod worker;
pub(super) use codec::validate_standalone_evidence;
use codec::{decode_payload, decode_raw, encode_payload, encode_raw, interpret};
#[cfg(feature = "test-support")]
pub use test_capture::{StandaloneUsageCapture, capture_standalone_usage};

fn scope(context: &PublicationContext) -> ExecutionScope {
    ExecutionScope {
        owner: context.job.owner.clone(),
        job: context.job.job,
        stage: context.stage.stage,
        dispatch: context.dispatch,
    }
}
#[derive(Clone, Copy)]
enum AnalysisTransportFailure {
    Transport,
    Deadline,
}
enum AnalysisResponse {
    Http { status: u16, body: Vec<u8> },
    Transport(AnalysisTransportFailure),
}
struct RawAnalysis {
    response: AnalysisResponse,
    segmentation: Option<(u32, u32)>,
}
impl RawAnalysis {
    fn body(&self) -> &[u8] {
        match &self.response {
            AnalysisResponse::Http { body, .. } => body,
            AnalysisResponse::Transport(_) => &[],
        }
    }
    fn status(&self) -> Option<u16> {
        match self.response {
            AnalysisResponse::Http { status, .. } => Some(status),
            AnalysisResponse::Transport(_) => None,
        }
    }
}

struct AnalysisInterpretation<'a> {
    intent: &'a DecodedAnalysis,
    dependencies: &'a Dependencies,
    previous: Option<ImageAnalysisResult>,
    prompt_index: usize,
    maximum: usize,
    supplier_elapsed: Option<Duration>,
}
struct AnalysisFinish<'a> {
    intent: &'a DecodedAnalysis,
    result: &'a ImageAnalysisResult,
    dependencies: &'a Dependencies,
    maximum: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PreparationMode {
    Fresh,
    Resume,
}
struct AnalysisPreparation {
    prompt_index: usize,
    timeout: Duration,
    maximum: usize,
    mode: PreparationMode,
}

fn telemetry_kind(operation: &ImageAnalysisOperation) -> crate::telemetry::AnalysisKind {
    use crate::telemetry::AnalysisKind;
    match operation {
        ImageAnalysisOperation::PhotoInspection(_) => AnalysisKind::PhotoInspection,
        ImageAnalysisOperation::HeadLocations(_) => AnalysisKind::HeadLocations,
        ImageAnalysisOperation::Segmentation(_) => AnalysisKind::Segmentation,
        ImageAnalysisOperation::StagingIdentity(_) => AnalysisKind::StagingIdentity,
        ImageAnalysisOperation::StagingFacing(_) => AnalysisKind::StagingFacing,
    }
}
