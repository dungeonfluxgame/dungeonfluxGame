//! The specs of the video clip kind of the generation hub: the home loop and the scene loop.
//!
//! A spec names everything that decides what the clip looks like: the still it is made from, the
//! motion prompt, the model, the duration, the frame size and the recipe version. The hub key is
//! the hash of those with the kind and the contract version, and the clip's file in the store is
//! named by the whole digest of the same text, so a changed prompt, model, duration, size or
//! recipe is a different clip and an old loop is never served for it.
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::gameplay::{
    resource_adapters::{hex, slug, spec_digest},
    resources::{Chrome, ResourceKind, ResourceSpec},
};
use df_render::art_contract::Neutral;

/// What a clip is made from, or why it cannot be made.
///
/// A request that cannot be formed (the painting is unreadable, the scene description is empty) is
/// still keyed, so the hub remembers its failure like any other and a changed input is a new key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct ClipInput {
    still_digest: Option<String>,
    prompt: Option<String>,
    unavailable: Option<&'static str>,
}

impl ClipInput {
    /// A still and the motion prompt it is given with.
    pub(in crate::gameplay) fn ready(still: &[u8], prompt: &str) -> Self {
        Self {
            still_digest: Some(hex(&Sha256::digest(still))),
            prompt: Some(prompt.to_owned()),
            unavailable: None,
        }
    }

    /// A request that cannot be formed. `still` is the plate when there is one.
    pub(in crate::gameplay) fn refused(still: Option<&[u8]>, reason: &'static str) -> Self {
        Self {
            still_digest: still.map(|still| hex(&Sha256::digest(still))),
            prompt: None,
            unavailable: Some(reason),
        }
    }

    pub(in crate::gameplay) fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }
}

/// How a clip is requested and cached. Changing any field is a different clip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::gameplay) struct ClipRecipe {
    pub(in crate::gameplay) model: String,
    pub(in crate::gameplay) duration_seconds: u32,
    pub(in crate::gameplay) width: u32,
    pub(in crate::gameplay) height: u32,
    /// The version of the way the clip is requested and cached (`df-home-video/2`).
    pub(in crate::gameplay) recipe: String,
    /// What is done to the provider's clip before it is stored (`as_delivered`, later a tail
    /// crossfade or a ping-pong). Part of the key: a finished clip is a different file.
    pub(in crate::gameplay) finish: &'static str,
}

/// The kind of motion a clip's prompt asks for. The wording lives in one place per variant (the
/// engine's prompt functions); the spec carries the variant so a clip made for one kind of motion
/// is never served for another, even if two wordings happened to be equal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum PromptVariant {
    /// A gentle ambience loop of a still: locked-off camera, only small ambient motion.
    Ambient,
    /// The ambient loop of a scene plate, staged from the plate's own description.
    SceneAmbient,
}

impl PromptVariant {
    pub(in crate::gameplay) const fn label(self) -> &'static str {
        match self {
            Self::Ambient => "ambient",
            Self::SceneAmbient => "scene_ambient",
        }
    }
}

/// How the still is given to the model. Both loops give the same still as the first and the last
/// frame, so the clip ends where it began and loops without a seam.
const FRAMES: &str = "first_and_last_frame_same_still";

fn canonical(
    variant: &str,
    prompt_variant: PromptVariant,
    input: &ClipInput,
    recipe: &ClipRecipe,
) -> String {
    json!({
        "variant": variant,
        "prompt_variant": prompt_variant.label(),
        "frames": FRAMES,
        "still": input.still_digest,
        "prompt": input.prompt,
        "unavailable": input.unavailable,
        "model": recipe.model,
        "duration_seconds": recipe.duration_seconds,
        "width": recipe.width,
        "height": recipe.height,
        "recipe": recipe.recipe,
        "finish": recipe.finish,
    })
    .to_string()
}

/// The home screen's looping clip of one painting.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct HomeLoopSpec {
    painting: String,
    input: ClipInput,
    recipe: ClipRecipe,
}

impl HomeLoopSpec {
    pub(in crate::gameplay) fn new(painting: &str, input: ClipInput, recipe: ClipRecipe) -> Self {
        Self {
            painting: painting.to_owned(),
            input,
            recipe,
        }
    }
}

impl ResourceSpec for HomeLoopSpec {
    const KIND: ResourceKind = ResourceKind::VideoClip;
    const CONTRACT_VERSION: u32 = 1;

    fn canonical(&self) -> String {
        canonical(
            "home_loop",
            PromptVariant::Ambient,
            &self.input,
            &self.recipe,
        )
    }

    fn subject(&self) -> String {
        let stem = self.painting.rsplit('/').next().unwrap_or_default();
        format!("video:home:{}", slug(stem, 48))
    }

    fn chrome(&self) -> Chrome {
        Neutral::Nothing
    }
}

/// The ambient loop of one scene plate.
#[derive(Clone, Debug)]
pub(in crate::gameplay) struct SceneLoopSpec {
    plate_key: String,
    input: ClipInput,
    recipe: ClipRecipe,
}

impl SceneLoopSpec {
    pub(in crate::gameplay) fn new(plate_key: &str, input: ClipInput, recipe: ClipRecipe) -> Self {
        Self {
            plate_key: plate_key.to_owned(),
            input,
            recipe,
        }
    }
}

impl ResourceSpec for SceneLoopSpec {
    const KIND: ResourceKind = ResourceKind::VideoClip;
    const CONTRACT_VERSION: u32 = 1;

    fn canonical(&self) -> String {
        canonical(
            "scene_loop",
            PromptVariant::SceneAmbient,
            &self.input,
            &self.recipe,
        )
    }

    fn subject(&self) -> String {
        let short: String = self.plate_key.chars().take(12).collect();
        format!("video:scene:{}", slug(&short, 12))
    }

    fn chrome(&self) -> Chrome {
        Neutral::Nothing
    }
}

/// The clip's file name and the key it is served by: the whole digest of the spec, whose first 16
/// bytes are the hub key.
pub(in crate::gameplay) fn file_key<S: ResourceSpec>(spec: &S) -> String {
    hex(&spec_digest(spec))
}
