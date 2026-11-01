//! Durable pre-egress Unjudged inspection evidence, with retained BestEffort policy proof.
use super::{DispatchId, ExecutionScope, StageBasis, StageClaim, StageScope};
use crate::{ImageJob, InspectionRequirement};
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalInspectionReason {
    Preparation,
    Disabled,
}
impl LocalInspectionReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparation => "preparation",
            Self::Disabled => "disabled",
        }
    }
    pub fn parse_label(text: &str) -> Option<Self> {
        match text {
            "preparation" => Some(Self::Preparation),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalInspectionError {
    InvalidRecord,
    NotBestEffort,
}

/// Only bounded canonical intent/config decoding can construct the recording policy proof.
#[derive(Clone, Eq, PartialEq)]
pub struct LocalInspectionCommand {
    claim: StageClaim,
    candidate_dispatch: DispatchId,
    source_digest: [u8; 32],
    reason: LocalInspectionReason,
    observed_at_ms: i64,
    intent_snapshot_revision: u32,
    intent_digest: [u8; 32],
    config_snapshot_revision: u32,
    config_digest: [u8; 32],
}
impl LocalInspectionCommand {
    /// Run before opening a journal transaction; retained configured limits govern original intent.
    pub fn new(
        job: &ImageJob,
        claim: StageClaim,
        candidate_dispatch: DispatchId,
        source_digest: [u8; 32],
        reason: LocalInspectionReason,
        observed_at_ms: i64,
    ) -> Result<Self, LocalInspectionError> {
        if claim.scope.owner != job.owner
            || claim.scope.job != job.job
            || claim.generation == 0
            || observed_at_ms <= 0
            || job.intent.bytes().first().copied().map(u32::from) != Some(job.intent.revision())
            || job.config.bytes().first().copied().map(u32::from) != Some(job.config.revision())
        {
            return Err(LocalInspectionError::InvalidRecord);
        }
        let maximum = crate::service::PROTOCOL_MAX_SNAPSHOT_BYTES;
        let config = crate::intent_codec::decode_config(job.config.bytes(), maximum)
            .map_err(|_| LocalInspectionError::InvalidRecord)?;
        let intent =
            crate::intent_codec::decode_intent(job.intent.bytes(), &config.request_limits, maximum)
                .map_err(|_| LocalInspectionError::InvalidRecord)?;
        if intent.owner != job.owner {
            return Err(LocalInspectionError::InvalidRecord);
        }
        let inspection = intent
            .operation
            .pipeline()
            .and_then(|pipeline| pipeline.inspection.as_ref())
            .ok_or(LocalInspectionError::NotBestEffort)?;
        if inspection.requirement != InspectionRequirement::BestEffort {
            return Err(LocalInspectionError::NotBestEffort);
        }
        Ok(Self {
            claim,
            candidate_dispatch,
            source_digest,
            reason,
            observed_at_ms,
            intent_snapshot_revision: job.intent.revision(),
            intent_digest: Sha256::digest(job.intent.bytes()).into(),
            config_snapshot_revision: job.config.revision(),
            config_digest: Sha256::digest(job.config.bytes()).into(),
        })
    }
    pub fn claim(&self) -> &StageClaim {
        &self.claim
    }
    pub fn candidate_dispatch(&self) -> DispatchId {
        self.candidate_dispatch
    }
    pub fn source_digest(&self) -> &[u8; 32] {
        &self.source_digest
    }
    pub fn reason(&self) -> LocalInspectionReason {
        self.reason
    }
    pub fn observed_at_ms(&self) -> i64 {
        self.observed_at_ms
    }
    pub fn intent_snapshot_revision(&self) -> u32 {
        self.intent_snapshot_revision
    }
    pub fn intent_digest(&self) -> &[u8; 32] {
        &self.intent_digest
    }
    pub fn config_snapshot_revision(&self) -> u32 {
        self.config_snapshot_revision
    }
    pub fn config_digest(&self) -> &[u8; 32] {
        &self.config_digest
    }
}
impl fmt::Debug for LocalInspectionCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalInspectionCommand")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}
/// A closed pre-egress Unjudged outcome; it names no supplier dispatch, image object or money.
#[derive(Clone, Eq, PartialEq)]
pub struct LocalInspectionCheckpoint {
    pub scope: StageScope,
    pub basis: StageBasis,
    pub candidate_scope: ExecutionScope,
    pub candidate_observation_version: u32,
    pub candidate_usage_revision: u32,
    pub source_digest: [u8; 32],
    pub reason: LocalInspectionReason,
    pub observed_at_ms: i64,
    pub schema_revision: u32,
    pub validator_revision: u32,
}
impl fmt::Debug for LocalInspectionCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalInspectionCheckpoint")
            .field("reason", &self.reason)
            .field("schema_revision", &self.schema_revision)
            .field("validator_revision", &self.validator_revision)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "local_inspection_tests.rs"]
mod tests;
