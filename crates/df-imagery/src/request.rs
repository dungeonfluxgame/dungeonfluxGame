//! Caller-claimed inputs for Generate, Edit and Transform image work.
//!
//! Every value here is a claim pending service authorization: holding a
//! request or a source pin never proves access to the scope, audience or
//! asset it names. The later service checks those before any provider or
//! pixel work. Nothing here touches a provider, the filesystem, or a
//! credential.

use std::fmt;

use df_assets::{AssetId, Audience};
use df_types::{AccountId, CampaignId, OperationId, RunId};

use crate::output::OutputProfile;
use crate::transform::TransformSpec;

/// Default prompt bound preserving the accepted provider/gameplay ceiling.
pub const DEFAULT_MAX_PROMPT_CHARS: usize = 32_000;
/// Default bound on ordered references per Generate or Edit.
pub const DEFAULT_MAX_REFERENCES: usize = 8;
/// Default bound on ordered steps per Transform recipe.
pub const DEFAULT_MAX_TRANSFORM_STEPS: usize = 32;
/// Default bound on layers per composite step.
pub const DEFAULT_MAX_COMPOSITE_LAYERS: usize = 8;

/// Operational request-shape bounds. Injected and validated: a value above a
/// default is valid when the injected configuration allows it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestLimits {
    pub max_prompt_chars: usize,
    pub max_references: usize,
    pub max_transform_steps: usize,
    pub max_composite_layers: usize,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            max_prompt_chars: DEFAULT_MAX_PROMPT_CHARS,
            max_references: DEFAULT_MAX_REFERENCES,
            max_transform_steps: DEFAULT_MAX_TRANSFORM_STEPS,
            max_composite_layers: DEFAULT_MAX_COMPOSITE_LAYERS,
        }
    }
}

impl RequestLimits {
    /// Every bound must admit at least one item; zero refuses all work.
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.max_prompt_chars == 0 {
            return Err(RequestError::InvalidLimit {
                field: "max_prompt_chars",
            });
        }
        if self.max_references == 0 {
            return Err(RequestError::InvalidLimit {
                field: "max_references",
            });
        }
        if self.max_transform_steps == 0 {
            return Err(RequestError::InvalidLimit {
                field: "max_transform_steps",
            });
        }
        if self.max_composite_layers == 0 {
            return Err(RequestError::InvalidLimit {
                field: "max_composite_layers",
            });
        }
        Ok(())
    }
}

/// Who the claimed work belongs to. Drafts need no already-created hero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageOwner {
    Run { campaign: CampaignId, run: RunId },
    Draft { account: AccountId },
}

/// Caller-claimed scope plus the stable operation identity of one intent.
/// The fingerprint binds canonical consumer intent before classification or
/// routing. The service validates it; changed intent is a conflict, never a retry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationIdentity {
    pub owner: ImageOwner,
    pub operation: OperationId,
    pub fingerprint: [u8; 32],
}

impl OperationIdentity {
    pub fn conflicts_with(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.operation == other.operation
            && self.fingerprint != other.fingerprint
    }
}

/// Closed set of image purposes. Variants inherit the originating domain
/// purpose plus a typed role rather than growing new purpose strings.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Purpose {
    CreatorPortrait,
    CanonicalPerson,
    CanonicalRoom,
    ScenePlate,
    SceneEdit,
    Expression,
    ItemCloseUp,
    PartyMural,
    CombatStill,
    AnimationFrame,
    ReferencePrep,
    ImportedDerivative,
}

impl Purpose {
    pub const COUNT: usize = 12;

    pub fn index(self) -> usize {
        match self {
            Self::CreatorPortrait => 0,
            Self::CanonicalPerson => 1,
            Self::CanonicalRoom => 2,
            Self::ScenePlate => 3,
            Self::SceneEdit => 4,
            Self::Expression => 5,
            Self::ItemCloseUp => 6,
            Self::PartyMural => 7,
            Self::CombatStill => 8,
            Self::AnimationFrame => 9,
            Self::ReferencePrep => 10,
            Self::ImportedDerivative => 11,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::CreatorPortrait => "creator_portrait",
            Self::CanonicalPerson => "canonical_person",
            Self::CanonicalRoom => "canonical_room",
            Self::ScenePlate => "scene_plate",
            Self::SceneEdit => "scene_edit",
            Self::Expression => "expression",
            Self::ItemCloseUp => "item_close_up",
            Self::PartyMural => "party_mural",
            Self::CombatStill => "combat_still",
            Self::AnimationFrame => "animation_frame",
            Self::ReferencePrep => "reference_prep",
            Self::ImportedDerivative => "imported_derivative",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::CreatorPortrait),
            1 => Some(Self::CanonicalPerson),
            2 => Some(Self::CanonicalRoom),
            3 => Some(Self::ScenePlate),
            4 => Some(Self::SceneEdit),
            5 => Some(Self::Expression),
            6 => Some(Self::ItemCloseUp),
            7 => Some(Self::PartyMural),
            8 => Some(Self::CombatStill),
            9 => Some(Self::AnimationFrame),
            10 => Some(Self::ReferencePrep),
            11 => Some(Self::ImportedDerivative),
            _ => None,
        }
    }
}

/// Role a variant plays under its originating purpose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VariantRole {
    Primary,
    Thumbnail,
    Mask,
    Reference,
}

/// A purpose plus the variant role of this request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PurposedUse {
    pub purpose: Purpose,
    pub role: VariantRole,
}

/// Expected SHA-256 of one complete immutable byte sequence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AssetDigest([u8; 32]);

impl AssetDigest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// One caller-claimed immutable source pin: catalog key, content digest and
/// the audience it was admitted under. No filesystem paths or remote URLs.
/// A claim pending service authorization; it never proves read access.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceRef {
    pub asset: AssetId,
    pub digest: AssetDigest,
    pub audience: Audience,
}

/// How the service may execute new work for this request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionMode {
    Live,
    PreparedOnly,
    Replay,
}

/// Reuse compatible output, or force a fresh generation under a new nonce.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Freshness {
    ReuseCompatible,
    Regenerate { nonce: OperationId },
}

/// Scheduling hint only; it never authorizes spend or overrides admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Urgency {
    Visible,
    Present,
    Speculative,
}

/// Bounded prompt text plus the template version that framed it. The text
/// field stays private and `Debug` reports only its length: prompts are
/// private descriptions and must never reach logs or diagnostics.
#[derive(Clone, Eq, PartialEq)]
pub struct TextPrompt {
    text: String,
    pub template_version: u32,
}

impl TextPrompt {
    /// Refuses empty text only. Length is checked at the request boundary
    /// against injected limits so configured ceilings apply.
    pub fn new(text: &str, template_version: u32) -> Result<Self, RequestError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(RequestError::EmptyPrompt);
        }
        Ok(Self {
            text: trimmed.to_owned(),
            template_version,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len_chars(&self) -> usize {
        self.text.chars().count()
    }
}

impl fmt::Debug for TextPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextPrompt")
            .field("len_chars", &self.len_chars())
            .field("template_version", &self.template_version)
            .finish()
    }
}

/// Generate inputs: explicit text and ordered claimed immutable references.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerateSpec {
    pub prompt: TextPrompt,
    pub references: Vec<SourceRef>,
}

/// Edit inputs: explicit text, one claimed base image, ordered references.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditSpec {
    pub prompt: TextPrompt,
    pub base: SourceRef,
    pub references: Vec<SourceRef>,
}

/// The operation a request asks the service to perform.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageOperation {
    Generate(GenerateSpec),
    Edit(EditSpec),
    Transform(TransformSpec),
}

/// The one caller-submitted request shape entering the later image service,
/// pending service authorization of scope, audience and source pins.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRequest {
    pub identity: OperationIdentity,
    pub purpose: PurposedUse,
    pub operation: ImageOperation,
    pub execution: ExecutionMode,
    pub freshness: Freshness,
    pub urgency: Urgency,
    pub audience: Audience,
    pub expected_source_revision: u64,
    pub output: OutputProfile,
}

impl ImageRequest {
    /// Boundary validation against injected limits. Counts and prompt length
    /// honor the injected configuration; mathematical and format invariants
    /// are enforced by the nested constructors and spec validation. The outer
    /// output is the single delivered authority: provider work needs a
    /// provider profile, local work needs a local profile.
    pub fn validate(&self, limits: &RequestLimits) -> Result<(), RequestError> {
        limits.validate()?;
        if matches!(self.audience, Audience::Withheld) {
            return Err(RequestError::WithheldAudience);
        }
        match &self.operation {
            ImageOperation::Generate(spec) => {
                check_prompt(&spec.prompt, limits)?;
                check_references(&spec.references, limits)?;
                if !matches!(self.output, OutputProfile::Provider { .. }) {
                    return Err(RequestError::ProfileMismatch);
                }
            }
            ImageOperation::Edit(spec) => {
                check_prompt(&spec.prompt, limits)?;
                check_references(&spec.references, limits)?;
                if !matches!(self.output, OutputProfile::Provider { .. }) {
                    return Err(RequestError::ProfileMismatch);
                }
            }
            ImageOperation::Transform(spec) => {
                spec.validate(limits)
                    .map_err(RequestError::InvalidTransform)?;
                if !matches!(self.output, OutputProfile::Local { .. }) {
                    return Err(RequestError::ProfileMismatch);
                }
            }
        }
        Ok(())
    }
}

fn check_prompt(prompt: &TextPrompt, limits: &RequestLimits) -> Result<(), RequestError> {
    let len_chars = prompt.len_chars();
    if len_chars > limits.max_prompt_chars {
        return Err(RequestError::PromptTooLong {
            len_chars,
            max_chars: limits.max_prompt_chars,
        });
    }
    Ok(())
}

fn check_references(references: &[SourceRef], limits: &RequestLimits) -> Result<(), RequestError> {
    if references.len() > limits.max_references {
        return Err(RequestError::TooManyReferences {
            count: references.len(),
        });
    }
    Ok(())
}

/// Why a request was refused before any provider or pixel work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    EmptyPrompt,
    PromptTooLong { len_chars: usize, max_chars: usize },
    TooManyReferences { count: usize },
    WithheldAudience,
    InvalidLimit { field: &'static str },
    ProfileMismatch,
    InvalidTransform(crate::transform::TransformError),
}
