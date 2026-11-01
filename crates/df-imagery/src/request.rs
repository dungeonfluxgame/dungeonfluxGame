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
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
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

/// Private recipe text. Debug exposes its length, never appearance or scene facts.
#[derive(Clone, Eq, PartialEq)]
pub struct PrivateImageText(String);

impl PrivateImageText {
    pub fn new(text: &str) -> Self {
        Self(text.to_owned())
    }
    pub fn text(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PrivateImageText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateImageText")
            .field("len_bytes", &self.0.len())
            .finish()
    }
}

/// Frozen nonsecret analysis route; credentials are resolved by account at execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisRoute {
    pub account: crate::config::AccountHandle,
    pub model: crate::config::ModelName,
    pub endpoint: crate::config::ApiEndpoint,
    pub maximum_output_tokens: u64,
}

/// The creator's versioned paid classification uses public appearance as quoted data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatorClassification {
    pub route: AnalysisRoute,
    pub contract_revision: u32,
    pub public_appearance: PrivateImageText,
    pub general_prompt: Option<PrivateImageText>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectionKind {
    ScenePlate,
    CanonicalRoom,
    CanonicalCharacter,
}

/// Technical provider moderation, separate from the application's content rating.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderModeration {
    Auto,
    Low,
}

/// Legacy scene quality checks keep a useful image when the judge is unavailable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InspectionRequirement {
    #[default]
    BestEffort,
    Required,
}

/// Exact public identities whose spatial ordering the photograph must preserve.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedIdentity {
    pub locator_id: PrivateImageText,
    pub reference_id: PrivateImageText,
    pub public_name: PrivateImageText,
    pub hero: bool,
}

/// Frozen quality/custody policy; corrections are separate paid children, never hidden retries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageInspection {
    pub route: AnalysisRoute,
    pub requirement: InspectionRequirement,
    pub contract_revision: u32,
    pub kind: InspectionKind,
    pub expected_heroes: Option<u8>,
    pub party_reference: Option<u32>,
    pub hand_custody: PrivateImageText,
    pub staging: Vec<StagedIdentity>,
    pub maximum_corrections: u32,
    pub allow_visible_text: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageContentRating {
    Restrained,
    Mature,
}

/// Additional semantic stages frozen with the original caller intent before paid execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImagePipeline {
    pub revision: u32,
    pub moderation: Option<ProviderModeration>,
    pub rating: ImageContentRating,
    pub creator: Option<CreatorClassification>,
    pub inspection: Option<ImageInspection>,
}

/// Invalid stage metadata carries field names only, never private values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineError {
    ZeroRevision,
    OutputTokens,
    EmptyAppearance,
    EmptyGeneralPrompt,
    PartyMetadata,
    Corrections,
    Staging,
    UnsupportedContract,
    TextTooLong { field: &'static str },
}

/// Creator classifier payload schemas this build reads and writes. A request naming any other
/// revision is refused as invalid pipeline input; it is never guessed, coerced or remapped.
pub const CREATOR_CLASSIFICATION_CONTRACTS: &[u32] = &[3];

impl ImagePipeline {
    fn validate_structure(&self) -> Result<(), RequestError> {
        let invalid = |error| RequestError::InvalidPipeline(error);
        if self.revision == 0 {
            return Err(invalid(PipelineError::ZeroRevision));
        }
        if let Some(creator) = &self.creator {
            if creator.route.maximum_output_tokens == 0 {
                return Err(invalid(PipelineError::OutputTokens));
            }
            if creator.contract_revision == 0 {
                return Err(invalid(PipelineError::ZeroRevision));
            }
            if !CREATOR_CLASSIFICATION_CONTRACTS.contains(&creator.contract_revision) {
                return Err(invalid(PipelineError::UnsupportedContract));
            }
            if creator.public_appearance.text().trim().is_empty() {
                return Err(invalid(PipelineError::EmptyAppearance));
            }
            if creator
                .general_prompt
                .as_ref()
                .is_some_and(|text| text.text().trim().is_empty())
            {
                return Err(invalid(PipelineError::EmptyGeneralPrompt));
            }
        }
        if let Some(inspection) = &self.inspection {
            if inspection.route.maximum_output_tokens == 0 {
                return Err(invalid(PipelineError::OutputTokens));
            }
            if inspection.contract_revision == 0 {
                return Err(invalid(PipelineError::ZeroRevision));
            }
            match (inspection.expected_heroes, inspection.party_reference) {
                (None, None) => {}
                (Some(1..=6), Some(reference)) if reference > 0 => {}
                _ => return Err(invalid(PipelineError::PartyMetadata)),
            }
            // Compatibility v1 supports the existing one-correction quality policy.
            if inspection.maximum_corrections > 1 {
                return Err(invalid(PipelineError::Corrections));
            }
            if inspection.staging.len() > 24
                || inspection.staging.iter().any(|identity| {
                    identity.locator_id.text().is_empty()
                        || identity.reference_id.text().is_empty()
                        || identity.public_name.text().is_empty()
                })
            {
                return Err(invalid(PipelineError::Staging));
            }
        }
        Ok(())
    }

    pub(crate) fn validate(&self, limits: &RequestLimits) -> Result<(), RequestError> {
        self.validate_structure()?;
        let check = |text: &PrivateImageText, field| {
            if text.text().chars().count() > limits.max_prompt_chars {
                Err(RequestError::InvalidPipeline(PipelineError::TextTooLong {
                    field,
                }))
            } else {
                Ok(())
            }
        };
        if let Some(creator) = &self.creator {
            // The deployed creator-content-v3 contract bounds public appearance in bytes.
            if creator.public_appearance.text().len() > 8 * 1024 {
                return Err(RequestError::InvalidPipeline(PipelineError::TextTooLong {
                    field: "public_appearance",
                }));
            }
            check(&creator.public_appearance, "public_appearance")?;
            if let Some(prompt) = &creator.general_prompt {
                check(prompt, "general_prompt")?;
            }
        }
        if let Some(inspection) = &self.inspection {
            check(&inspection.hand_custody, "hand_custody")?;
            for identity in &inspection.staging {
                check(&identity.locator_id, "locator_id")?;
                check(&identity.reference_id, "reference_id")?;
                check(&identity.public_name, "public_name")?;
            }
        }
        Ok(())
    }
}

/// Bounded prompt text plus the template version that framed it. The text
/// field stays private and `Debug` reports only its length: prompts are
/// private descriptions and must never reach logs or diagnostics.
#[derive(Clone, Eq, PartialEq)]
pub struct TextPrompt {
    text: String,
    pub template_version: u32,
    pipeline: Option<ImagePipeline>,
    artwork: Option<crate::ArtworkBinding>,
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
            pipeline: None,
            artwork: None,
        })
    }

    /// Retains typed public identity and the actual run fence without altering supplier text.
    pub fn with_artwork(mut self, artwork: crate::ArtworkBinding) -> Self {
        self.artwork = Some(artwork);
        self
    }

    pub fn artwork(&self) -> Option<&crate::ArtworkBinding> {
        self.artwork.as_ref()
    }

    pub fn with_pipeline(mut self, pipeline: ImagePipeline) -> Result<Self, RequestError> {
        pipeline.validate_structure()?;
        self.pipeline = Some(pipeline);
        Ok(self)
    }

    pub fn pipeline(&self) -> Option<&ImagePipeline> {
        self.pipeline.as_ref()
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

impl ImageOperation {
    /// Complete ordered source claims, including transformation masks and coverage lineage.
    pub fn sources(&self) -> Vec<&SourceRef> {
        fn transform_source<'a>(source: &'a crate::TransformSource, pins: &mut Vec<&'a SourceRef>) {
            match source {
                crate::TransformSource::Image(source) => pins.push(source),
                crate::TransformSource::SegmentationMask { analyzed, .. } => pins.push(analyzed),
                crate::TransformSource::Canvas(_) => {}
                crate::TransformSource::Coverage(coverage) => {
                    pins.push(&coverage.coverage);
                    pins.push(&coverage.analyzed);
                }
            }
        }
        let mut pins = Vec::new();
        match self {
            Self::Generate(spec) => pins.extend(&spec.references),
            Self::Edit(spec) => {
                pins.push(&spec.base);
                pins.extend(&spec.references);
            }
            Self::Transform(spec) => {
                transform_source(&spec.source, &mut pins);
                for step in &spec.steps {
                    match step {
                        crate::ImageTransform::Mask(mask) => match mask.source() {
                            crate::MaskSource::Asset(source) => pins.push(source),
                            crate::MaskSource::Coverage(coverage) => {
                                pins.push(&coverage.coverage);
                                pins.push(&coverage.analyzed);
                            }
                        },
                        crate::ImageTransform::Composite(layers) => {
                            pins.extend(layers.iter().map(|layer| layer.source()));
                        }
                        _ => {}
                    }
                }
            }
        }
        pins
    }

    pub fn artwork(&self) -> Option<&crate::ArtworkBinding> {
        match self {
            Self::Generate(spec) => spec.prompt.artwork(),
            Self::Edit(spec) => spec.prompt.artwork(),
            Self::Transform(_) => None,
        }
    }

    pub fn pipeline(&self) -> Option<&ImagePipeline> {
        match self {
            Self::Generate(spec) => spec.prompt.pipeline(),
            Self::Edit(spec) => spec.prompt.pipeline(),
            Self::Transform(_) => None,
        }
    }
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
        if let Some(artwork) = self.operation.artwork() {
            artwork
                .validate_for(&self.identity.owner, self.purpose, &self.audience)
                .map_err(RequestError::InvalidArtwork)?;
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
    if let Some(pipeline) = prompt.pipeline() {
        pipeline.validate(limits)?;
    }
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
    InvalidPipeline(PipelineError),
    InvalidArtwork(crate::ArtworkError),
}
