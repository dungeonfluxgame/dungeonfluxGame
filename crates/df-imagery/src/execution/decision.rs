//! Private observed decision payloads and immutable stage dependency basis.
//! Bytes and source digests never appear in Debug; schema/validator pins are lineage, not approval.

use super::{DispatchId, ExecutionSetupError, ExecutionStageId, ImageJobId, StageKind};
use crate::ImageOwner;
use sha2::{Digest, Sha256};
use std::fmt;

/// Default host policy; operators may raise or lower it independently of the SQL storage ceiling.
pub const DEFAULT_DECISION_PAYLOAD_BYTES: i64 = 262_144;
/// PostgreSQL varlena payload ceiling, excluding its four-byte header.
pub const MAXIMUM_SQL_DECISION_PAYLOAD_BYTES: i64 = 1_073_741_820;

/// A stage dependency, distinct from a dispatch's same-stage retry lineage.
#[derive(Clone, Eq, PartialEq)]
pub struct StageBasis {
    pub kind: StageKind,
    pub input_revision: u32,
    pub input_digest: [u8; 32],
    pub config_revision: u64,
    pub config_digest: [u8; 32],
    pub parent_dispatch: Option<DispatchId>,
}
impl fmt::Debug for StageBasis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StageBasis")
            .field("kind", &self.kind)
            .field("input_revision", &self.input_revision)
            .field("config_revision", &self.config_revision)
            .field("has_parent", &self.parent_dispatch.is_some())
            .finish_non_exhaustive()
    }
}

/// The two actual JSON-producing stages; final images retain OutputCheckpoint instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionKind {
    Classification,
    Inspection,
    PhotoInspection,
    HeadLocations,
    Segmentation,
    StagingIdentity,
    StagingFacing,
}
impl DecisionKind {
    pub fn stage_kind(self) -> StageKind {
        match self {
            Self::Classification => StageKind::Classification,
            Self::Inspection => StageKind::Inspection,
            Self::StagingFacing => StageKind::StagingFacing,
            Self::StagingIdentity => StageKind::StagingIdentity,
            Self::Segmentation => StageKind::Segmentation,
            Self::HeadLocations => StageKind::HeadLocations,
            Self::PhotoInspection => StageKind::PhotoInspection,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Classification => "classification",
            Self::Inspection => "inspection",
            Self::StagingFacing => "staging_facing",
            Self::StagingIdentity => "staging_identity",
            Self::Segmentation => "segmentation",
            Self::HeadLocations => "head_locations",
            Self::PhotoInspection => "photo_inspection",
        }
    }
    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "classification" => Some(Self::Classification),
            "inspection" => Some(Self::Inspection),
            "staging_facing" => Some(Self::StagingFacing),
            "staging_identity" => Some(Self::StagingIdentity),
            "segmentation" => Some(Self::Segmentation),
            "head_locations" => Some(Self::HeadLocations),
            "photo_inspection" => Some(Self::PhotoInspection),
            _ => None,
        }
    }
}

/// Exact private owner/job/stage/dispatch scope for observed decision bytes.
#[derive(Clone, Eq, PartialEq)]
pub struct ExecutionScope {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
}
impl fmt::Debug for ExecutionScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExecutionScope { .. }")
    }
}

/// Compatibility name for the same exact scoped observation identity.
pub type DecisionScope = ExecutionScope;

/// Pins the interpretation contract and exact frozen source/configuration dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionBasis {
    pub schema_revision: u32,
    pub validator_revision: u32,
    pub stage: StageBasis,
}

/// The raw usage revision must already be durable; missing cost evidence never means zero cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationReceipt {
    pub usage_revision: u32,
    pub observed_at_ms: i64,
}

/// Decision observations use the shared immutable observation receipt.
pub type DecisionReceipt = ObservationReceipt;

/// Bounded observed bytes with explicit lineage; only runtime qualification approves their meaning.
#[derive(Clone, Eq, PartialEq)]
pub struct DecisionObservation {
    pub scope: DecisionScope,
    pub kind: DecisionKind,
    pub observation_version: u32,
    pub basis: DecisionBasis,
    pub receipt: DecisionReceipt,
    payload: Box<[u8]>,
    payload_sha256: [u8; 32],
}
impl DecisionObservation {
    /// Hash bounded source bytes on the owning worker before opening a journal transaction.
    pub fn new(
        scope: DecisionScope,
        kind: DecisionKind,
        observation_version: u32,
        basis: DecisionBasis,
        payload: Vec<u8>,
        receipt: DecisionReceipt,
        maximum_payload_bytes: usize,
    ) -> Result<Self, ExecutionSetupError> {
        if observation_version == 0
            || basis.schema_revision == 0
            || basis.validator_revision == 0
            || basis.stage.input_revision == 0
            || basis.stage.config_revision == 0
            || receipt.usage_revision == 0
            || basis.stage.kind != kind.stage_kind()
        {
            return Err(ExecutionSetupError::BadRevision);
        }
        if maximum_payload_bytes == 0 {
            return Err(ExecutionSetupError::ZeroBytes);
        }
        let technical = usize::try_from(MAXIMUM_SQL_DECISION_PAYLOAD_BYTES).map_err(|_| {
            ExecutionSetupError::TooLong {
                max_bytes: usize::MAX,
            }
        })?;
        if maximum_payload_bytes > technical {
            return Err(ExecutionSetupError::TooLong {
                max_bytes: technical,
            });
        }
        if payload.is_empty() {
            return Err(ExecutionSetupError::Empty);
        }
        if payload.len() > maximum_payload_bytes {
            return Err(ExecutionSetupError::TooLong {
                max_bytes: maximum_payload_bytes,
            });
        }
        if receipt.observed_at_ms <= 0 {
            return Err(ExecutionSetupError::BadTime);
        }
        Ok(Self {
            scope,
            kind,
            observation_version,
            basis,
            receipt,
            payload_sha256: Sha256::digest(&payload).into(),
            payload: payload.into_boxed_slice(),
        })
    }
    pub fn payload_sha256(&self) -> &[u8; 32] {
        &self.payload_sha256
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}
impl fmt::Debug for DecisionObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecisionObservation")
            .field("kind", &self.kind)
            .field("observation_version", &self.observation_version)
            .field("schema_revision", &self.basis.schema_revision)
            .field("validator_revision", &self.basis.validator_revision)
            .field("receipt", &self.receipt)
            .field("payload_len_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}

/// Cheap immutable terminal decision identity; source bytes are qualified outside publication
/// locks.
#[derive(Clone, Eq, PartialEq)]
pub struct DecisionReference {
    pub scope: ExecutionScope,
    pub kind: DecisionKind,
    pub observation_version: u32,
    pub basis: DecisionBasis,
    pub receipt: ObservationReceipt,
    pub payload_sha256: [u8; 32],
}
impl fmt::Debug for DecisionReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecisionReference")
            .field("kind", &self.kind)
            .field("observation_version", &self.observation_version)
            .field("receipt", &self.receipt)
            .finish_non_exhaustive()
    }
}
