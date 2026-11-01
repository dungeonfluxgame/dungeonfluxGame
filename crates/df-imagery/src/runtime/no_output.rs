//! A definitive no-output proof retains the exact failed supplier and raw usage revision.
use super::{RuntimeError, RuntimeUsage};
use crate::cost::UsageEvidence;
use crate::execution::{DispatchState, DispatchView, ExecutionScope, StageBasis, StageKind};
use crate::{ImageJob, ImageOwner};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct QualifiedNoOutput {
    owner: ImageOwner,
    job: crate::ImageJobId,
    expected_revision: u64,
    intent_digest: [u8; 32],
    config_digest: [u8; 32],
    scope: ExecutionScope,
    basis: StageBasis,
    usage_revision: u32,
    raw_sha256: Option<[u8; 32]>,
}
impl std::fmt::Debug for QualifiedNoOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QualifiedNoOutput").finish_non_exhaustive()
    }
}
impl QualifiedNoOutput {
    pub fn intent_digest(&self) -> &[u8; 32] {
        &self.intent_digest
    }
    pub fn config_digest(&self) -> &[u8; 32] {
        &self.config_digest
    }
    pub fn matches_financial_metadata(
        &self,
        owner: &ImageOwner,
        job: &crate::ImageJobId,
        intent_digest: &[u8; 32],
        config_digest: &[u8; 32],
    ) -> bool {
        self.owner == *owner
            && self.job == *job
            && self.intent_digest == *intent_digest
            && self.config_digest == *config_digest
    }
    pub fn scope(&self) -> &ExecutionScope {
        &self.scope
    }
    pub fn expected_revision(&self) -> u64 {
        self.expected_revision
    }
    pub fn usage_revision(&self) -> u32 {
        self.usage_revision
    }
    pub fn raw_sha256(&self) -> Option<&[u8; 32]> {
        self.raw_sha256.as_ref()
    }
    pub fn basis(&self) -> &StageBasis {
        &self.basis
    }
    pub fn matches(
        &self,
        job: &ImageJob,
        dispatch: &DispatchView,
        basis: &StageBasis,
        raw_sha256: Option<&[u8; 32]>,
    ) -> bool {
        self.owner == job.owner
            && self.job == job.job
            && self.expected_revision == job.revision
            && self.intent_digest == <[u8; 32]>::from(Sha256::digest(job.intent.bytes()))
            && self.config_digest == <[u8; 32]>::from(Sha256::digest(job.config.bytes()))
            && self.scope.owner == job.owner
            && self.scope.job == dispatch.job
            && self.scope.stage == dispatch.stage
            && self.scope.dispatch == dispatch.dispatch
            && dispatch.state == DispatchState::Failed
            && self.usage_revision == dispatch.usage_revision
            && self.basis == *basis
            && self.raw_sha256.as_ref() == raw_sha256
    }
}
pub fn qualify_no_output(
    job: &ImageJob,
    dispatch: &DispatchView,
    basis: &StageBasis,
    usage: &RuntimeUsage,
) -> Result<QualifiedNoOutput, RuntimeError> {
    if basis.kind.is_standalone() {
        super::standalone::validate_standalone_failure(job, dispatch, basis, usage)?;
    }
    let revision = u32::try_from(usage.revision.0).map_err(|_| RuntimeError::InvalidRecord)?;
    if dispatch.job != job.job
        || dispatch.stage != usage.stage
        || dispatch.dispatch != usage.dispatch
        || usage.owner != job.owner
        || usage.job != job.job
        || revision == 0
        || revision != dispatch.usage_revision
        || dispatch.state != DispatchState::Failed
        || usage.evidence == UsageEvidence::Unknown
        || usage.usable
        || basis.kind == StageKind::LocalTransform
        || basis.config_digest != <[u8; 32]>::from(Sha256::digest(job.config.bytes()))
    {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(QualifiedNoOutput {
        owner: job.owner.clone(),
        job: job.job,
        expected_revision: job.revision,
        intent_digest: Sha256::digest(job.intent.bytes()).into(),
        config_digest: basis.config_digest,
        scope: ExecutionScope {
            owner: job.owner.clone(),
            job: job.job,
            stage: dispatch.stage,
            dispatch: dispatch.dispatch,
        },
        basis: basis.clone(),
        usage_revision: revision,
        raw_sha256: usage
            .raw_output
            .as_deref()
            .map(|bytes| Sha256::digest(bytes).into()),
    })
}
