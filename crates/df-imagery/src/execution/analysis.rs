//! Explicit standalone analytical results and immutable provenance, independent of media
//! publication.
use super::{DecisionKind, DecisionReference, StageBasis, StageScope};
use crate::{OperationHandle, VersionedSnapshot};
use std::fmt;

/// One immutable result selected for an owned analytical job; selection requires runtime
/// qualification.
#[derive(Clone, Eq, PartialEq)]
pub struct AnalysisCheckpoint {
    pub scope: StageScope,
    pub kind: DecisionKind,
    pub checkpoint_version: u32,
    pub schema_revision: u32,
    pub validator_revision: u32,
    pub result: VersionedSnapshot,
    pub result_sha256: [u8; 32],
    pub intent_digest: [u8; 32],
    pub config_digest: [u8; 32],
    pub source_digest: [u8; 32],
    pub stages: Vec<AnalysisStageReference>,
    pub dependencies: Vec<AnalysisDependencyRef>,
}
impl fmt::Debug for AnalysisCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalysisCheckpoint")
            .field("kind", &self.kind)
            .field("checkpoint_version", &self.checkpoint_version)
            .field("schema_revision", &self.schema_revision)
            .field("validator_revision", &self.validator_revision)
            .field("stage_count", &self.stages.len())
            .field("dependency_count", &self.dependencies.len())
            .finish_non_exhaustive()
    }
}

/// Exact reference to an already selected immutable final result, never a latest-stage query.
#[derive(Clone, Eq, PartialEq)]
pub struct AnalysisCheckpointReference {
    pub scope: StageScope,
    pub kind: DecisionKind,
    pub checkpoint_version: u32,
    pub schema_revision: u32,
    pub validator_revision: u32,
    pub result_sha256: [u8; 32],
    pub intent_digest: [u8; 32],
    pub config_digest: [u8; 32],
    pub source_digest: [u8; 32],
}
impl AnalysisCheckpointReference {
    pub fn from_checkpoint(checkpoint: &AnalysisCheckpoint) -> Self {
        Self {
            scope: checkpoint.scope.clone(),
            kind: checkpoint.kind,
            checkpoint_version: checkpoint.checkpoint_version,
            schema_revision: checkpoint.schema_revision,
            validator_revision: checkpoint.validator_revision,
            result_sha256: checkpoint.result_sha256,
            intent_digest: checkpoint.intent_digest,
            config_digest: checkpoint.config_digest,
            source_digest: checkpoint.source_digest,
        }
    }
}
impl fmt::Debug for AnalysisCheckpointReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalysisCheckpointReference")
            .field("kind", &self.kind)
            .field("checkpoint_version", &self.checkpoint_version)
            .finish_non_exhaustive()
    }
}

/// The exact scoped operation and its explicitly selected analytical result consumed downstream.
#[derive(Clone, Eq, PartialEq)]
pub struct AnalysisDependencyRef {
    pub handle: OperationHandle,
    pub checkpoint: AnalysisCheckpointReference,
}
impl fmt::Debug for AnalysisDependencyRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalysisDependencyRef")
            .field("checkpoint", &self.checkpoint)
            .finish_non_exhaustive()
    }
}

/// One actually consumed paid decision or proven deterministic result in canonical execution order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalysisStageReference {
    Supplier(DecisionReference),
    Local(LocalAnalysisReference),
}

/// A no-model result constructed only through the runtime's actual deterministic validator.
/// It names no supplier dispatch, usage receipt, publication allocation or money entry.
#[derive(Clone, Eq, PartialEq)]
pub struct LocalAnalysisCheckpoint {
    pub scope: StageScope,
    pub kind: DecisionKind,
    pub checkpoint_version: u32,
    pub schema_revision: u32,
    pub validator_revision: u32,
    pub basis: StageBasis,
    pub result: VersionedSnapshot,
    pub result_sha256: [u8; 32],
    pub intent_digest: [u8; 32],
    pub config_digest: [u8; 32],
    pub source_digest: [u8; 32],
    pub dependencies: Vec<AnalysisDependencyRef>,
}
impl fmt::Debug for LocalAnalysisCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalAnalysisCheckpoint")
            .field("kind", &self.kind)
            .field("checkpoint_version", &self.checkpoint_version)
            .field("schema_revision", &self.schema_revision)
            .field("validator_revision", &self.validator_revision)
            .field("dependency_count", &self.dependencies.len())
            .finish_non_exhaustive()
    }
}

/// Metadata identifying one immutable no-dispatch result in a full analytical execution chain.
#[derive(Clone, Eq, PartialEq)]
pub struct LocalAnalysisReference {
    pub scope: StageScope,
    pub kind: DecisionKind,
    pub checkpoint_version: u32,
    pub schema_revision: u32,
    pub validator_revision: u32,
    pub basis: StageBasis,
    pub result_sha256: [u8; 32],
    pub intent_digest: [u8; 32],
    pub config_digest: [u8; 32],
    pub source_digest: [u8; 32],
}
impl LocalAnalysisReference {
    pub fn from_checkpoint(checkpoint: &LocalAnalysisCheckpoint) -> Self {
        Self {
            scope: checkpoint.scope.clone(),
            kind: checkpoint.kind,
            checkpoint_version: checkpoint.checkpoint_version,
            schema_revision: checkpoint.schema_revision,
            validator_revision: checkpoint.validator_revision,
            basis: checkpoint.basis.clone(),
            result_sha256: checkpoint.result_sha256,
            intent_digest: checkpoint.intent_digest,
            config_digest: checkpoint.config_digest,
            source_digest: checkpoint.source_digest,
        }
    }
}
impl fmt::Debug for LocalAnalysisReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalAnalysisReference")
            .field("kind", &self.kind)
            .field("checkpoint_version", &self.checkpoint_version)
            .finish_non_exhaustive()
    }
}
