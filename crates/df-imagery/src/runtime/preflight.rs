//! Actual pre-egress request failures close their own claimed job without supplier identities.
use super::RuntimeError;
use crate::execution::*;
use crate::listed::listed_enum;
use crate::{ImageJob, ImageOwner};
use sha2::{Digest, Sha256};

listed_enum! {
    keyed
    /// The actual failure occurred before any supplier or finance dispatch existed.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum PreflightCause {
        MissingCredential,
        UnsupportedProfile,
    }
}
impl PreflightCause {
    pub fn key(self) -> &'static str {
        match self {
            Self::MissingCredential => "missing_credential",
            Self::UnsupportedProfile => "unsupported_profile",
        }
    }
}

/// Only the native worker constructs this proof after observing its actual closed preflight error.
#[derive(Clone)]
pub struct QualifiedPreflightRejected {
    owner: ImageOwner,
    job: crate::ImageJobId,
    revision: u64,
    intent_digest: [u8; 32],
    config_digest: [u8; 32],
    claim: StageClaim,
    basis: StageBasis,
    reason: PreflightCause,
    candidate: Option<CandidateReceipt>,
}
impl std::fmt::Debug for QualifiedPreflightRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QualifiedPreflightRejected")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}
impl QualifiedPreflightRejected {
    pub(super) fn observed(
        job: &ImageJob,
        claim: StageClaim,
        basis: StageBasis,
        reason: PreflightCause,
        candidate: Option<CandidateReceipt>,
    ) -> Result<Self, RuntimeError> {
        if claim.scope.owner != job.owner
            || claim.scope.job != job.job
            || claim.generation == 0
            || basis.config_digest != <[u8; 32]>::from(Sha256::digest(job.config.bytes()))
            || candidate.as_ref().is_some_and(|candidate| {
                candidate.scope.owner != job.owner || candidate.scope.job != job.job
            })
        {
            return Err(RuntimeError::InvalidRecord);
        }
        Ok(Self {
            owner: job.owner.clone(),
            job: job.job,
            revision: job.revision,
            intent_digest: Sha256::digest(job.intent.bytes()).into(),
            config_digest: basis.config_digest,
            claim,
            basis,
            reason,
            candidate,
        })
    }
    pub fn matches_metadata(
        &self,
        owner: &ImageOwner,
        job: &crate::ImageJobId,
        revision: u64,
        intent_digest: &[u8; 32],
        config_digest: &[u8; 32],
    ) -> bool {
        self.owner == *owner
            && self.job == *job
            && self.revision == revision
            && self.intent_digest == *intent_digest
            && self.config_digest == *config_digest
    }
    pub fn claim(&self) -> &StageClaim {
        &self.claim
    }
    pub fn basis(&self) -> &StageBasis {
        &self.basis
    }
    pub fn reason(&self) -> PreflightCause {
        self.reason
    }
    pub fn candidate(&self) -> Option<&CandidateReceipt> {
        self.candidate.as_ref()
    }
    pub fn expected_revision(&self) -> u64 {
        self.revision
    }
    pub fn intent_digest(&self) -> &[u8; 32] {
        &self.intent_digest
    }
    pub fn config_digest(&self) -> &[u8; 32] {
        &self.config_digest
    }
}
