//! Frozen output requirements and owner-scoped publication reads; no SQL or transport.

use crate::execution::{DispatchId, ExecutionStageId};
use crate::{ImageJobId, ImageOwner, VersionedSnapshot};
use df_assets::{
    AssetId, AssetVariant, AttemptId, Audience, ImageAlpha, ImageAlphaRequirement, ImageIntent,
    ImageProofError, MediaRequestKey, ObjectVersion, WriteId,
};
use sha2::{Digest, Sha256};
use std::future::Future;

const MAGIC: &[u8; 8] = b"DFIMGP01";
/// The only frozen-output envelope revision this build writes and reads.
const FROZEN_OUTPUT_REVISION: u32 = 1;
/// One encoded intent: mime tag, width, height and alpha tag.
const INTENT_ENCODED_BYTES: usize = 1 + 4 + 4 + 1;
/// Display-presence flag plus the execution recipe length prefix.
const ENVELOPE_FIELD_BYTES: usize = 1 + 4;
/// Revision-one mime tags by index; `ImageIntent` admits exactly these types.
const MIME_TAGS: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];
/// Maximum complete frozen recipe envelope, in bytes; matches the durable snapshot allocation.
pub const MAXIMUM_RECIPE_BYTES: usize = 1_048_576;

/// Claimed final execution selection, verified against the durable allocation and candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct FinalOutputSelection {
    pub allocation_stage: ExecutionStageId,
    pub candidate: Option<crate::execution::CandidateReceipt>,
    pub resolved_recipe: VersionedSnapshot,
    pub decisions: Vec<FinalDecisionRef>,
    pub local_inspections: Vec<crate::execution::LocalInspectionCheckpoint>,
}

/// Protected semantic or demand-ended evidence; caller booleans cannot seal a job.
#[derive(Clone, Copy)]
pub enum NoDeliveryCause<'a> {
    Semantic(&'a crate::runtime::QualifiedNoDelivery),
    DemandEnded(&'a crate::runtime::QualifiedDemandEnded),
    RequiredPreflight(&'a crate::runtime::QualifiedRequiredPreflight),
    NoOutput(&'a crate::runtime::QualifiedNoOutput),
    PreflightRejected(&'a crate::runtime::QualifiedPreflightRejected),
    SubmitDeclined(&'a crate::execution::DeclineReceipt),
}

/// Exact execution tail authority and protected cause for atomic output cleanup.
pub struct NoDeliveryRequest<'a> {
    pub terminal: &'a crate::execution::TerminalCommand,
    pub cause: NoDeliveryCause<'a>,
}

/// Durable terminal metadata names no fabricated dispatch or usage for unsent stages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoDeliveryStage {
    Dispatch {
        scope: crate::execution::ExecutionScope,
        usage_revision: u32,
    },
    UnsentStage(crate::execution::StageScope),
    UnclaimedStage(crate::execution::StageScope),
    DeferredStage(crate::execution::StageScope),
    ReleasedStage(crate::execution::StageScope),
}

/// Whether the sealed job had already allocated a catalog asset and attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoDeliveryAllocation {
    NoAllocation,
    Existing {
        stage: ExecutionStageId,
        asset: AssetId,
        attempt: AttemptId,
    },
}

/// Immutable no-delivery acknowledgement; it neither settles nor releases supplier liability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoDeliveryReceipt {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub actor: NoDeliveryStage,
    pub tail: NoDeliveryStage,
    pub candidate: Option<crate::execution::ExecutionScope>,
    pub allocation: NoDeliveryAllocation,
    pub reason: crate::runtime::NoDeliveryReason,
    pub expected_job_revision: u64,
    pub job_revision: u64,
    pub sealed_at_ms: i64,
}

/// Actual current source refusal; infrastructure and structural binding errors remain separate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceDenial {
    GrantRefused,
    Missing,
    Tombstoned,
    NotReady,
    WinnerChanged,
    AudienceChanged,
    RetentionChanged,
}

/// A native recheck closes only an actual current source refusal and revives no provider work.
#[derive(Clone, Debug, Eq, PartialEq)]
// Public cross-crate enum built by df-persistence; boxing a variant would break that API.
#[allow(clippy::large_enum_variant, reason = "df-persistence builds it")]
pub enum SourceClosure {
    Closed(NoDeliveryReceipt),
    PublicationAllowed,
}

/// Exact observed decision lineage; checkpoint existence alone never implies semantic approval.
#[derive(Clone, Eq, PartialEq)]
pub struct FinalDecisionRef {
    pub scope: crate::execution::ExecutionScope,
    pub kind: crate::execution::DecisionKind,
    pub observation_version: u32,
    pub usage_revision: u32,
    pub basis: crate::execution::DecisionBasis,
    pub payload_sha256: [u8; 32],
}

impl std::fmt::Debug for FinalDecisionRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FinalDecisionRef")
            .field("kind", &self.kind)
            .field("observation_version", &self.observation_version)
            .field("usage_revision", &self.usage_revision)
            .finish_non_exhaustive()
    }
}

impl FinalDecisionRef {
    /// The lineage reference of one observed decision, including its payload digest.
    pub fn from_observation(observation: &crate::execution::DecisionObservation) -> Self {
        Self {
            scope: observation.scope.clone(),
            kind: observation.kind,
            observation_version: observation.observation_version,
            usage_revision: observation.receipt.usage_revision,
            basis: observation.basis.clone(),
            payload_sha256: *observation.payload_sha256(),
        }
    }
}

/// Canonical output requirements plus the complete opaque canonical execution plan.
#[derive(Clone, Eq, PartialEq)]
pub struct FrozenOutput {
    master: ImageIntent,
    display: Option<ImageIntent>,
    execution_recipe: Vec<u8>,
}

impl std::fmt::Debug for FrozenOutput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrozenOutput")
            .field("master", &self.master)
            .field("display", &self.display)
            .field("execution_recipe_bytes", &self.execution_recipe.len())
            .finish()
    }
}

impl FrozenOutput {
    /// Refuses an empty plan and any envelope over [`MAXIMUM_RECIPE_BYTES`].
    pub fn new(
        master: ImageIntent,
        display: Option<ImageIntent>,
        execution_recipe: Vec<u8>,
    ) -> Result<Self, PublicationError> {
        let envelope_bytes = MAGIC.len()
            + INTENT_ENCODED_BYTES
            + ENVELOPE_FIELD_BYTES
            + if display.is_some() {
                INTENT_ENCODED_BYTES
            } else {
                0
            };
        let encoded_bytes = execution_recipe
            .len()
            .checked_add(envelope_bytes)
            .ok_or(PublicationError::InvalidRecipe)?;
        if execution_recipe.is_empty() || encoded_bytes > MAXIMUM_RECIPE_BYTES {
            return Err(PublicationError::InvalidRecipe);
        }
        Ok(Self {
            master,
            display,
            execution_recipe,
        })
    }

    pub fn intent(&self, variant: AssetVariant) -> Option<ImageIntent> {
        match variant {
            AssetVariant::Master => Some(self.master),
            AssetVariant::Display => self.display,
        }
    }

    pub fn execution_recipe(&self) -> &[u8] {
        &self.execution_recipe
    }

    /// Revision one encodes both expected variants and the full execution plan without ambiguity.
    pub fn snapshot(&self) -> Result<VersionedSnapshot, PublicationError> {
        let mut bytes = MAGIC.to_vec();
        encode_intent(&mut bytes, self.master)?;
        bytes.push(u8::from(self.display.is_some()));
        if let Some(display) = self.display {
            encode_intent(&mut bytes, display)?;
        }
        let length = u32::try_from(self.execution_recipe.len())
            .map_err(|_| PublicationError::InvalidRecipe)?;
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(&self.execution_recipe);
        VersionedSnapshot::new(FROZEN_OUTPUT_REVISION, bytes, MAXIMUM_RECIPE_BYTES)
            .map_err(|_| PublicationError::InvalidRecipe)
    }

    /// Unknown versions, trailing bytes and oversized plans are refused, never inferred.
    pub fn decode(snapshot: &VersionedSnapshot) -> Result<Self, PublicationError> {
        if snapshot.revision() != FROZEN_OUTPUT_REVISION
            || snapshot.bytes().len() > MAXIMUM_RECIPE_BYTES
        {
            return Err(PublicationError::InvalidRecipe);
        }
        let mut bytes = snapshot.bytes();
        if take(&mut bytes, MAGIC.len())? != MAGIC {
            return Err(PublicationError::InvalidRecipe);
        }
        let master = decode_intent(&mut bytes)?;
        let display = match take(&mut bytes, 1)? {
            [0] => None,
            [1] => Some(decode_intent(&mut bytes)?),
            _ => return Err(PublicationError::InvalidRecipe),
        };
        let length =
            usize::try_from(read_u32(&mut bytes)?).map_err(|_| PublicationError::InvalidRecipe)?;
        let execution_recipe = take(&mut bytes, length)?.to_vec();
        if !bytes.is_empty() {
            return Err(PublicationError::InvalidRecipe);
        }
        Self::new(master, display, execution_recipe)
    }
}

/// An unknown mime type is refused rather than written under another type's tag.
fn encode_intent(bytes: &mut Vec<u8>, intent: ImageIntent) -> Result<(), PublicationError> {
    let mime_tag = MIME_TAGS
        .iter()
        .position(|mime| *mime == intent.mime())
        .and_then(|index| u8::try_from(index).ok())
        .ok_or(PublicationError::InvalidRecipe)?;
    bytes.push(mime_tag);
    bytes.extend_from_slice(&intent.width_px().to_be_bytes());
    bytes.extend_from_slice(&intent.height_px().to_be_bytes());
    bytes.push(match intent.alpha_requirement() {
        ImageAlphaRequirement::Opaque => 0,
        ImageAlphaRequirement::Transparent => 1,
        ImageAlphaRequirement::Preserve => 2,
    });
    Ok(())
}

fn take<'a>(bytes: &mut &'a [u8], length: usize) -> Result<&'a [u8], PublicationError> {
    let (taken, rest) = bytes
        .split_at_checked(length)
        .ok_or(PublicationError::InvalidRecipe)?;
    *bytes = rest;
    Ok(taken)
}

fn read_u32(bytes: &mut &[u8]) -> Result<u32, PublicationError> {
    let value = take(bytes, 4)?
        .try_into()
        .map_err(|_| PublicationError::InvalidRecipe)?;
    Ok(u32::from_be_bytes(value))
}

fn decode_intent(bytes: &mut &[u8]) -> Result<ImageIntent, PublicationError> {
    let [mime_tag] = take(bytes, 1)? else {
        return Err(PublicationError::InvalidRecipe);
    };
    let mime = MIME_TAGS
        .get(usize::from(*mime_tag))
        .ok_or(PublicationError::InvalidRecipe)?;
    let width = read_u32(bytes)?;
    let height = read_u32(bytes)?;
    let alpha = match take(bytes, 1)? {
        [0] => ImageAlphaRequirement::Opaque,
        [1] => ImageAlphaRequirement::Transparent,
        [2] => ImageAlphaRequirement::Preserve,
        _ => return Err(PublicationError::InvalidRecipe),
    };
    ImageIntent::new(mime, width, height, alpha).map_err(PublicationError::Proof)
}

/// Digest includes the envelope revision as well as every frozen byte.
pub fn recipe_digest(snapshot: &VersionedSnapshot) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"df-imagery/frozen-output-v1");
    digest.update(snapshot.revision().to_be_bytes());
    digest.update(snapshot.bytes());
    digest.finalize().into()
}

/// Byte properties of one winning, version-pinned catalog representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedVariant {
    pub variant: AssetVariant,
    pub write: WriteId,
    pub object_version: ObjectVersion,
    pub byte_len: u64,
    pub sha256: [u8; 32],
    pub intent: ImageIntent,
    pub alpha: ImageAlpha,
}

/// A durable result is read only within its owner scope and while its catalog asset is Ready.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedImage {
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub recipe_digest: [u8; 32],
    pub job_revision: u64,
    pub published_at_ms: i64,
    pub private_reconciliation: bool,
    pub variants: Vec<PublishedVariant>,
}

/// A shared actual artifact conveys no foreign execution handle, interest or fabricated usage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadyArtifact {
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub audience: Audience,
    pub semantic_key: MediaRequestKey,
    pub published_at_ms: i64,
    pub variants: Vec<PublishedVariant>,
    pub inspection: Option<crate::PhotoInspectionAssessment>,
    pub staging: Vec<crate::StagedIdentity>,
}

/// Publication refusals do not authorize new generation; Unknown requires rereading the same job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicationError {
    Unavailable,
    Unknown,
    NotFound,
    InvalidRecord,
    InvalidRecipe,
    BindingConflict,
    SourceDenied(SourceDenial),
    StaleClaim,
    CheckpointMismatch,
    Proof(ImageProofError),
    Catalog(df_assets::CatalogError),
    Execution(crate::execution::ExecutionError),
}

impl std::fmt::Display for PublicationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("the publication store is unavailable"),
            Self::Unknown => formatter.write_str("the publication outcome is unknown"),
            Self::NotFound => formatter.write_str("no such publication"),
            Self::InvalidRecord => formatter.write_str("a publication record is invalid"),
            Self::InvalidRecipe => formatter.write_str("the publication recipe is invalid"),
            Self::BindingConflict => formatter.write_str("the publication binding conflicts"),
            Self::SourceDenied(denial) => write!(formatter, "the source was denied: {denial:?}"),
            Self::StaleClaim => formatter.write_str("the publication claim is stale"),
            Self::CheckpointMismatch => formatter.write_str("the output checkpoint does not match"),
            Self::Proof(error) => write!(formatter, "the image proof failed: {error:?}"),
            Self::Catalog(error) => write!(formatter, "the catalog refused the asset: {error:?}"),
            Self::Execution(error) => write!(formatter, "the execution store failed: {error}"),
        }
    }
}

impl std::error::Error for PublicationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Execution(error) => Some(error),
            _ => None,
        }
    }
}

/// The caller authorizes this owner before reading; implementations cannot widen the scope.
pub trait ImageResults: Send + Sync {
    /// Resolves a typed shared artifact read target using the current caller's authority.
    fn read_ready_artifact(
        &self,
        _current_owner: &ImageOwner,
        _asset: &AssetId,
        _now_ms: i64,
    ) -> impl Future<Output = Result<Option<ReadyArtifact>, PublicationError>> + Send {
        async { Ok(None) }
    }

    /// A current Ready artifact compatible with the request under its frozen config, if any.
    fn find_ready_artifact(
        &self,
        _request: &crate::ImageRequest,
        _config: &VersionedSnapshot,
        _now_ms: i64,
    ) -> impl Future<Output = Result<Option<ReadyArtifact>, PublicationError>> + Send {
        async { Ok(None) }
    }

    /// Resolves only the requesting job's durable artifact dependency under current authority.
    fn read_artifact(
        &self,
        _owner: &ImageOwner,
        _job: &ImageJobId,
        _now_ms: i64,
    ) -> impl Future<Output = Result<Option<ReadyArtifact>, PublicationError>> + Send {
        async { Ok(None) }
    }

    /// Historical Ready reuse requires current catalog/source/grant checks and revives no demand.
    fn read_reusable_result(
        &self,
        _owner: &ImageOwner,
        _job: &ImageJobId,
        _now_ms: i64,
    ) -> impl Future<Output = Result<Option<PublishedImage>, PublicationError>> + Send {
        async { Ok(None) }
    }

    /// The job's published result within its owner scope, or `None`.
    fn read_result(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
    ) -> impl Future<Output = Result<Option<PublishedImage>, PublicationError>> + Send;
}

#[cfg(test)]
mod publication_tests;
