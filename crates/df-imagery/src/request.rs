//! Caller-claimed inputs for Generate, Edit and Transform image work.
//!
//! Every value here is a claim pending service authorization: holding a
//! request or a source pin never proves access to the scope, audience or
//! asset it names. The later service checks those before any provider or
//! pixel work. Nothing here touches a provider, the filesystem, or a
//! credential.

use crate::listed::listed_enum;
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

/// Bound on staged identities in one inspection; inspection contract v1.
pub(crate) const MAXIMUM_STAGED_IDENTITIES: usize = 24;
/// The deployed creator-content-v3 contract bounds public appearance in UTF-8 bytes.
pub(crate) const MAXIMUM_PUBLIC_APPEARANCE_BYTES: usize = 8 * 1024;
/// Largest party an inspection may expect; matches the table's hero seats.
const MAXIMUM_EXPECTED_HEROES: u8 = 6;
/// Compatibility v1 supports the existing one-correction quality policy.
const MAXIMUM_INSPECTION_CORRECTIONS: u32 = 1;

/// Operational request-shape bounds. Injected and validated: a value above a
/// default is valid when the injected configuration allows it. Outside this
/// crate a value exists only through [`RequestLimits::new`], so it is valid;
/// the configuration overlay writes the fields and validates the whole config.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestLimits {
    pub(crate) max_prompt_chars: usize,
    pub(crate) max_references: usize,
    pub(crate) max_transform_steps: usize,
    pub(crate) max_composite_layers: usize,
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
    /// The bounds in order: prompt characters, references, transform steps and layers per
    /// composite. Every bound must admit at least one item.
    pub fn new(
        max_prompt_chars: usize,
        max_references: usize,
        max_transform_steps: usize,
        max_composite_layers: usize,
    ) -> Result<Self, RequestError> {
        let limits = Self {
            max_prompt_chars,
            max_references,
            max_transform_steps,
            max_composite_layers,
        };
        limits.validate()?;
        Ok(limits)
    }

    pub fn max_prompt_chars(&self) -> usize {
        self.max_prompt_chars
    }

    pub fn max_references(&self) -> usize {
        self.max_references
    }

    pub fn max_transform_steps(&self) -> usize {
        self.max_transform_steps
    }

    pub fn max_composite_layers(&self) -> usize {
        self.max_composite_layers
    }

    /// Every bound must admit at least one item; zero refuses all work.
    pub(crate) fn validate(&self) -> Result<(), RequestError> {
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
    /// True when the same scoped operation returns with a different intent fingerprint.
    pub fn conflicts_with(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.operation == other.operation
            && self.fingerprint != other.fingerprint
    }
}

listed_enum! {
    keyed
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
}

impl Purpose {
    /// Number of purposes; [`Self::index`] runs from zero to one below this.
    pub const COUNT: usize = 12;

    /// Stable dense index, used for per-purpose tables and canonical encodings.
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

    /// Stable snake_case name for diagnostics and configuration keys.
    pub fn key(self) -> &'static str {
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

    /// The purpose at a stable index, or `None` past [`Self::COUNT`].
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
    /// Wraps an expected digest; it is verified against bytes only when they are read.
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
    /// Keeps the text exactly; bounds are checked with the pipeline that carries it.
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

/// What an inspection judges, which selects its contract questions.
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

/// The application's content rating for generated pictures.
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
    TextTooLong { field: PipelineTextField },
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroRevision => formatter.write_str("a pipeline revision is zero"),
            Self::OutputTokens => formatter.write_str("the output token bound is invalid"),
            Self::EmptyAppearance => formatter.write_str("the public appearance is empty"),
            Self::EmptyGeneralPrompt => formatter.write_str("the general prompt is empty"),
            Self::PartyMetadata => formatter.write_str("the party metadata is invalid"),
            Self::Corrections => formatter.write_str("the correction metadata is invalid"),
            Self::Staging => formatter.write_str("the staging metadata is invalid"),
            Self::UnsupportedContract => {
                formatter.write_str("the pipeline contract is not supported")
            }
            Self::TextTooLong { field } => {
                write!(formatter, "the pipeline text {field:?} is too long")
            }
        }
    }
}

impl std::error::Error for PipelineError {}

/// Which pipeline text exceeded its bound; never the text itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineTextField {
    PublicAppearance,
    GeneralPrompt,
    HandCustody,
    LocatorId,
    ReferenceId,
    PublicName,
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
                (Some(heroes), Some(reference))
                    if (1..=MAXIMUM_EXPECTED_HEROES).contains(&heroes) && reference > 0 => {}
                _ => return Err(invalid(PipelineError::PartyMetadata)),
            }
            if inspection.maximum_corrections > MAXIMUM_INSPECTION_CORRECTIONS {
                return Err(invalid(PipelineError::Corrections));
            }
            if inspection.staging.len() > MAXIMUM_STAGED_IDENTITIES
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
        let too_long = |field| RequestError::InvalidPipeline(PipelineError::TextTooLong { field });
        let check = |text: &PrivateImageText, field| {
            if text.text().chars().count() > limits.max_prompt_chars {
                Err(too_long(field))
            } else {
                Ok(())
            }
        };
        if let Some(creator) = &self.creator {
            if creator.public_appearance.text().len() > MAXIMUM_PUBLIC_APPEARANCE_BYTES {
                return Err(too_long(PipelineTextField::PublicAppearance));
            }
            check(
                &creator.public_appearance,
                PipelineTextField::PublicAppearance,
            )?;
            if let Some(prompt) = &creator.general_prompt {
                check(prompt, PipelineTextField::GeneralPrompt)?;
            }
        }
        if let Some(inspection) = &self.inspection {
            check(&inspection.hand_custody, PipelineTextField::HandCustody)?;
            for identity in &inspection.staging {
                check(&identity.locator_id, PipelineTextField::LocatorId)?;
                check(&identity.reference_id, PipelineTextField::ReferenceId)?;
                check(&identity.public_name, PipelineTextField::PublicName)?;
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

    /// Attaches stage metadata after checking its structure; text bounds are checked at the
    /// request boundary against injected limits.
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextPrompt")
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

    /// The artwork binding a Generate or Edit prompt carries; transforms carry none.
    pub fn artwork(&self) -> Option<&crate::ArtworkBinding> {
        match self {
            Self::Generate(spec) => spec.prompt.artwork(),
            Self::Edit(spec) => spec.prompt.artwork(),
            Self::Transform(_) => None,
        }
    }

    /// The stage metadata a Generate or Edit prompt carries; transforms carry none.
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

impl std::fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPrompt => formatter.write_str("the prompt is empty"),
            Self::PromptTooLong {
                len_chars,
                max_chars,
            } => write!(
                formatter,
                "the prompt has {len_chars} characters, more than {max_chars}"
            ),
            Self::TooManyReferences { count } => {
                write!(formatter, "{count} references are too many")
            }
            Self::WithheldAudience => formatter.write_str("nobody may see this output"),
            Self::InvalidLimit { field } => {
                write!(formatter, "the request limit {field} admits nothing")
            }
            Self::ProfileMismatch => {
                formatter.write_str("the output profile does not suit the operation")
            }
            Self::InvalidTransform(error) => write!(formatter, "the transform is invalid: {error}"),
            Self::InvalidPipeline(error) => write!(formatter, "the pipeline is invalid: {error}"),
            Self::InvalidArtwork(error) => write!(formatter, "the artwork is invalid: {error}"),
        }
    }
}

impl std::error::Error for RequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTransform(error) => Some(error),
            Self::InvalidPipeline(error) => Some(error),
            Self::InvalidArtwork(error) => Some(error),
            _ => None,
        }
    }
}
