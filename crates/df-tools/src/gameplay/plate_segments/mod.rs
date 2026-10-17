//! Who is where in a scene plate, from hosted SAM 3.1 segmentation (PLAY-V09).
//!
//! The display plate is the photographic scene still on the TV. It is segmented once with the
//! noun phrases `person` and `head` (`df_providers::NativeFalSegmenter`, endpoint
//! `fal-ai/sam-3-1/image-rle`), and the instances become the facts the rest of the program stages
//! from:
//!
//! 1. [`geometry`]: each SAM person gets a tight body box, the tight box of its own head, and the
//!    loader anchor (the point just above the crown), all as fractions of the plate. Heads are
//!    paired to people by mask overlap, never by position words.
//! 2. [`identity`]: each person is given to a cast member or a hero. The existing head locator's
//!    boxes are matched to SAM heads by intersection over union; where the locator has nothing, a
//!    vision model sees the pinned reference portraits and numbered candidate crops and answers
//!    with a closed choice (a candidate number or `0` for none). Never by name or position.
//! 3. [`facing`]: one batched vision call per plate labels each assigned person `left`, `right`,
//!    `camera`, `away` or `unclear`, as the picture shows them.
//! 4. [`placement`]: the loader's side (open space, the way the person faces, never over another
//!    head) and the permille head, body and anchor boxes of the wire layout.
//! 5. [`staging`]: neutral facts for the storyteller and the NPC replies (screen region, depth
//!    order, facing, who they face, who is nearest), so narration agrees with the picture.
//!
//! Everything here is pure except [`service`], the hub adapter that makes the `PlateSegments` and
//! `PlateStaging` resources (the provider calls, the notes, the spend and the `PLATE-STAGING`
//! audit line). The plate director, the wire layout and the dev CLI sit above it.
//!
//! The closed-label prompts ([`JudgeRequest`]) are built here and sent through
//! `df_providers::NativePlateJudge`, whose strict JSON schema is the same enum the parser checks.

pub(in crate::gameplay) mod crops;
pub(in crate::gameplay) mod facing;
pub(in crate::gameplay) mod geometry;
pub(in crate::gameplay) mod identity;
pub(in crate::gameplay) mod placement;
pub(in crate::gameplay) mod service;
pub(in crate::gameplay) mod staging;

use serde_json::Value;

/// A closed-label question for the vision judge: the fixed instruction, and the strict JSON
/// schema whose enums are the only answers it may give.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gameplay) struct JudgeRequest {
    pub(in crate::gameplay) instruction: String,
    pub(in crate::gameplay) schema_name: &'static str,
    pub(in crate::gameplay) schema: Value,
}

/// Why a judge's answer was not accepted. The caller keeps what it had.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum ReadError {
    /// Not the object the schema describes.
    Shape,
    /// A label or number outside what was offered.
    OutsideChoices,
    /// Someone offered was left out or answered twice.
    Incomplete,
}

/// The label of the `n`th reference portrait (counted from 1), as the pictures are named.
pub(in crate::gameplay) fn reference_label(number: usize) -> String {
    format!("ref-{number}")
}

/// The label of candidate `number` (counted from 1), as the pictures are named.
pub(in crate::gameplay) fn candidate_label(number: usize) -> String {
    format!("cand-{number}")
}
