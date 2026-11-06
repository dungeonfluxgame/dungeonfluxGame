//! Protected, bounded inspection of intermediate supplier images, without a catalog asset.
//! Persisted projections expose metadata only; only inspect() constructs recordable observations.
use super::{CheckpointAlpha, ExecutionScope, ObservationReceipt, StageBasis, StageKind};
use df_assets::{ImageAlpha, ImageIntent, ImageProof, ImageProofError, ImageProofLimits};
use sha2::{Digest, Sha256};
use std::fmt;

/// Revision of the bounded decoder/actual-alpha validator used for candidate observations.
pub const CANDIDATE_VALIDATOR_REVISION: u32 = 1;

/// Actual validated source properties; hashes, dimensions and MIME do not appear in Debug.
#[derive(Clone, Eq, PartialEq)]
pub struct CandidateDescriptor {
    pub mime: String,
    pub width_px: u32,
    pub height_px: u32,
    pub alpha: CheckpointAlpha,
    pub byte_len: u64,
    pub sha256: [u8; 32],
    pub validator_revision: u32,
}
impl fmt::Debug for CandidateDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateDescriptor")
            .field("byte_len", &self.byte_len)
            .field("validator_revision", &self.validator_revision)
            .finish_non_exhaustive()
    }
}

/// Immutable metadata read from the journal, not authority to record uninspected bytes.
#[derive(Clone, Eq, PartialEq)]
pub struct CandidateReceipt {
    pub scope: ExecutionScope,
    pub basis: StageBasis,
    pub observation_version: u32,
    pub receipt: ObservationReceipt,
    pub descriptor: CandidateDescriptor,
}
impl fmt::Debug for CandidateReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateReceipt")
            .field("kind", &self.basis.kind)
            .field("observation_version", &self.observation_version)
            .field("receipt", &self.receipt)
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

/// Rejections contain only typed causes and never encoded bytes or source content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateError {
    InvalidRecord,
    WrongStage,
    Validation(ImageProofError),
}

impl std::fmt::Display for CandidateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRecord => formatter.write_str("the candidate record is invalid"),
            Self::WrongStage => formatter.write_str("the candidate belongs to another stage"),
            Self::Validation(error) => write!(
                formatter,
                "the candidate image failed validation: {error:?}"
            ),
        }
    }
}

impl std::error::Error for CandidateError {}

/// Recordable evidence can only be constructed through real bounded image inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateObservation {
    inner: CandidateReceipt,
}
impl CandidateObservation {
    /// Run on the owning blocking worker: decoding and actual-alpha inspection are bounded CPU
    /// work.
    pub fn inspect(
        scope: ExecutionScope,
        basis: StageBasis,
        observation_version: u32,
        receipt: ObservationReceipt,
        bytes: &[u8],
        bounds: ImageProofLimits,
        intent: ImageIntent,
    ) -> Result<Self, CandidateError> {
        if !matches!(
            basis.kind,
            StageKind::Generation | StageKind::Edit | StageKind::Correction | StageKind::Extraction
        ) {
            return Err(CandidateError::WrongStage);
        }
        if observation_version == 0
            || receipt.usage_revision == 0
            || receipt.observed_at_ms <= 0
            || basis.input_revision == 0
            || basis.config_revision == 0
        {
            return Err(CandidateError::InvalidRecord);
        }
        let alpha = ImageProof::inspect_encoded(bytes, intent, bounds)
            .map_err(CandidateError::Validation)?;
        let alpha = match alpha {
            ImageAlpha::Opaque => CheckpointAlpha::Opaque,
            ImageAlpha::Transparent => CheckpointAlpha::Transparent,
        };
        Ok(Self {
            inner: CandidateReceipt {
                scope,
                basis,
                observation_version,
                receipt,
                descriptor: CandidateDescriptor {
                    mime: intent.mime().to_owned(),
                    width_px: intent.width_px(),
                    height_px: intent.height_px(),
                    alpha,
                    byte_len: u64::try_from(bytes.len())
                        .map_err(|_| CandidateError::InvalidRecord)?,
                    sha256: Sha256::digest(bytes).into(),
                    validator_revision: CANDIDATE_VALIDATOR_REVISION,
                },
            },
        })
    }
    pub fn as_receipt(&self) -> &CandidateReceipt {
        &self.inner
    }
}
