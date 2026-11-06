//! Meaningful identity and output groups shared by execution constructors.
use super::{CheckpointAlpha, ClaimToken, ExecutionStageId, ImageJobId};
use crate::ImageOwner;
use std::fmt;

/// One owned logical stage, before a particular dispatch is chosen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageScope {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
}
/// The exact lease identity presented by one normal stage worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageClaim {
    pub scope: StageScope,
    pub token: ClaimToken,
    pub generation: u64,
}
/// One immutable stored output identity; this remains metadata until catalog validation.
#[derive(Clone, Eq, PartialEq)]
pub struct OutputObject {
    pub storage_key: String,
    pub object_version: String,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}
impl fmt::Debug for OutputObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutputObject")
            .field("byte_len", &self.byte_len)
            .finish_non_exhaustive()
    }
}
/// Validated image metadata bound to a stored output by publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputImage {
    pub mime: String,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub alpha: CheckpointAlpha,
}
/// Private supplier resume data; origin, model and opaque bytes never appear in Debug.
#[derive(Clone, Eq, PartialEq)]
pub struct SupplierResume {
    pub checkpoint_version: u32,
    pub origin: String,
    pub model: String,
    pub checkpoint_bytes: Vec<u8>,
}
impl fmt::Debug for SupplierResume {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SupplierResume")
            .field("checkpoint_version", &self.checkpoint_version)
            .field("len_bytes", &self.checkpoint_bytes.len())
            .finish_non_exhaustive()
    }
}
