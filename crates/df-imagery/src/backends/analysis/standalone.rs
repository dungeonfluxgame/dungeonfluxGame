//! Private standalone image analysis adapters. Durable authority and accounting belong to the runtime.
mod chat;
mod geometry;
mod heads;
mod photo;
mod pictures;
mod segmentation;
mod staging;

pub(crate) use chat::{
    PreparedStandaloneChat, StandaloneChatInput, interpret_chat, prepare_local_analysis,
};
pub(crate) use segmentation::{
    PreparedSegmentation, SegmentationCheckpoint, SegmentationInput, parse_segmentation,
};

pub(crate) use photo::{
    PhotoInspectionInput, PreparedPhotoInspection, interpret_photo, project_photo_assessment,
    restore_photo_assessment, unjudged_photo,
};

#[cfg(test)]
use chat::prepare_local_staging;

use crate::runtime::RuntimeError;
use serde_json::Value;

struct Question {
    instruction: String,
    schema_name: &'static str,
    schema: Value,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadError {
    Shape,
    OutsideChoices,
    Incomplete,
}
impl From<ReadError> for RuntimeError {
    fn from(_: ReadError) -> Self {
        Self::InvalidRecord
    }
}
fn reference_label(number: usize) -> String {
    format!("ref-{number}")
}
fn candidate_label(number: usize) -> String {
    format!("cand-{number}")
}

#[cfg(test)]
#[path = "standalone/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "standalone/legacy_photo_tests.rs"]
mod legacy_photo_tests;

#[cfg(test)]
#[path = "standalone/legacy_crop_tests.rs"]
mod legacy_crop_tests;

#[cfg(test)]
#[path = "standalone/legacy_head_tests.rs"]
mod legacy_head_tests;

#[cfg(test)]
#[path = "standalone/legacy_segment_tests.rs"]
mod legacy_segment_tests;
