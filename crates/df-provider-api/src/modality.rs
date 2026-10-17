//! Consumer-owned modality metadata around the common checked request boundary.
//!
//! Modality names do not define payload schemas, transport headers, provider routes,
//! billing meters, rights qualification, or permission to dispatch. Source and rights
//! revisions are equality fences supplied by the owning consumer; their presence does
//! not establish source rights, authorization, quote approval, or egress permission.

use std::fmt;

use df_model::checkpoint::AssetReference;
use df_model::checkpoint::ContentDigest;
use df_types::{LocaleTag, RevisionLabel};

use crate::{
    CheckedRequest, RequestBinding, RequestError, RequestLimits, RequestOwnerState, RequestUsage,
};

/// One of the six provider-work families currently named by the subsystem plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestModality {
    Text,
    Stt,
    Tts,
    Image,
    Video,
    Sound,
}

/// Explicit source, opaque rights/version marker, output format, and caller context.
///
/// Equality participates in the existing request's current-binding check. The opaque
/// markers are not themselves rights evidence, access credentials, or format schemas.
#[derive(Clone, Eq, PartialEq)]
pub struct RequestBasis<Semantic> {
    pub modality: RequestModality,
    pub source: ContentDigest,
    pub rights_revision: RevisionLabel,
    pub output_format_revision: RevisionLabel,
    pub semantic: Semantic,
}

impl<Semantic> fmt::Debug for RequestBasis<Semantic> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestBasis")
            .finish_non_exhaustive()
    }
}

/// A common checked request tagged with its consumer-selected modality basis.
///
/// Construction and current validation use the existing `CheckedRequest` policy for
/// job identity, execution mode, deadline, payload bytes, declared usage, and limits.
/// This type does not qualify modality payloads or initiate provider work.
pub struct CheckedModalityRequest<Semantic> {
    checked: CheckedRequest<RequestBasis<Semantic>>,
}

/// Typed rejection at the six-family boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModalityRequestError {
    WrongModality,
    Request(RequestError),
}

/// Text-generation context and the caller-owned output schema.
/// The context and schema stay typed and bounded by the owner; this API does not
/// validate prompt semantics or implement the output format.
#[derive(Clone, Eq, PartialEq)]
pub struct TextSemantics<Context, OutputSchema> {
    pub locale: LocaleTag,
    pub context: Context,
    pub output_schema: OutputSchema,
}

/// Speech-recognition input profile and partial/final transcript profile.
/// The owner selects a supported locale and audio/transcript schemas.
#[derive(Clone, Eq, PartialEq)]
pub struct SttSemantics<AudioFormat, TranscriptProfile> {
    pub locale: LocaleTag,
    pub audio_format: AudioFormat,
    pub transcript_profile: TranscriptProfile,
}

/// Speech synthesis text, voice, encoded output, PCM and timebase profiles.
/// The typed profile preserves ordering metadata without prescribing a codec.
#[derive(Clone, Eq, PartialEq)]
pub struct TtsSemantics<Text, Voice, OutputFormat, PcmFormat, Timebase> {
    pub locale: LocaleTag,
    pub text: Text,
    pub voice: Voice,
    pub output_format: OutputFormat,
    pub pcm_format: PcmFormat,
    pub timebase: Timebase,
}

/// Image prompt, caller-owned output profile, and optional canonical references.
/// Reference values retain the canonical asset identity; readiness stays native-owned.
#[derive(Clone, Eq, PartialEq)]
pub struct ImageSemantics<Prompt, OutputProfile> {
    pub locale: Option<LocaleTag>,
    pub prompt: Prompt,
    pub output_profile: OutputProfile,
    pub references: Vec<AssetReference>,
}

/// Video prompt, output profile, and typed caller-owned reference frames.
/// Frame schema remains owned by the supplying caller until it is frozen upstream.
#[derive(Clone, Eq, PartialEq)]
pub struct VideoSemantics<Prompt, OutputProfile, ReferenceFrame> {
    pub locale: Option<LocaleTag>,
    pub prompt: Prompt,
    pub output_profile: OutputProfile,
    pub reference_frames: Vec<ReferenceFrame>,
}

/// Sound intent, output profile and loop metadata.
/// Sound/music/ambience details and loop policy remain caller-owned typed values.
#[derive(Clone, Eq, PartialEq)]
pub struct SoundSemantics<Intent, OutputProfile, LoopMetadata> {
    pub locale: Option<LocaleTag>,
    pub intent: Intent,
    pub output_profile: OutputProfile,
    pub loop_metadata: LoopMetadata,
}

/// One family-specific request retaining the canonical checked envelope.
/// Construction validates the declared family before delegating bounds, identity,
/// mode, deadline and cancellation to the existing checked-request policy.
pub struct TextRequest<Context, OutputSchema> {
    checked: CheckedModalityRequest<TextSemantics<Context, OutputSchema>>,
}

pub struct SttRequest<AudioFormat, TranscriptProfile> {
    checked: CheckedModalityRequest<SttSemantics<AudioFormat, TranscriptProfile>>,
}

pub struct TtsRequest<Text, Voice, OutputFormat, PcmFormat, Timebase> {
    checked: CheckedModalityRequest<TtsSemantics<Text, Voice, OutputFormat, PcmFormat, Timebase>>,
}

pub struct ImageRequest<Prompt, OutputProfile> {
    checked: CheckedModalityRequest<ImageSemantics<Prompt, OutputProfile>>,
}

pub struct VideoRequest<Prompt, OutputProfile, ReferenceFrame> {
    checked: CheckedModalityRequest<VideoSemantics<Prompt, OutputProfile, ReferenceFrame>>,
}

pub struct SoundRequest<Intent, OutputProfile, LoopMetadata> {
    checked: CheckedModalityRequest<SoundSemantics<Intent, OutputProfile, LoopMetadata>>,
}

/// Canonical request binding for a text-to-speech request.
pub type TtsRequestBinding<Text, Voice, OutputFormat, PcmFormat, Timebase> =
    RequestBinding<RequestBasis<TtsSemantics<Text, Voice, OutputFormat, PcmFormat, Timebase>>>;

/// Current owner state for a text-to-speech request.
pub type TtsRequestOwner<'a, Text, Voice, OutputFormat, PcmFormat, Timebase> = RequestOwnerState<
    'a,
    RequestBasis<TtsSemantics<Text, Voice, OutputFormat, PcmFormat, Timebase>>,
>;

fn checked_for<Semantic: Eq>(
    expected: RequestModality,
    binding: RequestBinding<RequestBasis<Semantic>>,
    payload: &[u8],
    usage: RequestUsage,
    limits: RequestLimits,
    owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
) -> Result<CheckedModalityRequest<Semantic>, ModalityRequestError> {
    if binding.semantic_basis.modality != expected {
        return Err(ModalityRequestError::WrongModality);
    }
    CheckedModalityRequest::new(binding, payload, usage, limits, owner)
        .map_err(ModalityRequestError::Request)
}

impl<Context: Eq, OutputSchema: Eq> TextRequest<Context, OutputSchema> {
    pub fn new(
        binding: RequestBinding<RequestBasis<TextSemantics<Context, OutputSchema>>>,
        payload: &[u8],
        usage: RequestUsage,
        limits: RequestLimits,
        owner: RequestOwnerState<'_, RequestBasis<TextSemantics<Context, OutputSchema>>>,
    ) -> Result<Self, ModalityRequestError> {
        let checked = checked_for(
            RequestModality::Text,
            binding,
            payload,
            usage,
            limits,
            owner,
        )?;
        Ok(Self { checked })
    }

    pub fn checked(&self) -> &CheckedModalityRequest<TextSemantics<Context, OutputSchema>> {
        &self.checked
    }
}

impl<AudioFormat: Eq, TranscriptProfile: Eq> SttRequest<AudioFormat, TranscriptProfile> {
    pub fn new(
        binding: RequestBinding<RequestBasis<SttSemantics<AudioFormat, TranscriptProfile>>>,
        payload: &[u8],
        usage: RequestUsage,
        limits: RequestLimits,
        owner: RequestOwnerState<'_, RequestBasis<SttSemantics<AudioFormat, TranscriptProfile>>>,
    ) -> Result<Self, ModalityRequestError> {
        let checked = checked_for(RequestModality::Stt, binding, payload, usage, limits, owner)?;
        Ok(Self { checked })
    }

    pub fn checked(&self) -> &CheckedModalityRequest<SttSemantics<AudioFormat, TranscriptProfile>> {
        &self.checked
    }
}

impl<Text: Eq, Voice: Eq, OutputFormat: Eq, PcmFormat: Eq, Timebase: Eq>
    TtsRequest<Text, Voice, OutputFormat, PcmFormat, Timebase>
{
    pub fn new(
        binding: TtsRequestBinding<Text, Voice, OutputFormat, PcmFormat, Timebase>,
        payload: &[u8],
        usage: RequestUsage,
        limits: RequestLimits,
        owner: TtsRequestOwner<'_, Text, Voice, OutputFormat, PcmFormat, Timebase>,
    ) -> Result<Self, ModalityRequestError> {
        let checked = checked_for(RequestModality::Tts, binding, payload, usage, limits, owner)?;
        Ok(Self { checked })
    }

    pub fn checked(
        &self,
    ) -> &CheckedModalityRequest<TtsSemantics<Text, Voice, OutputFormat, PcmFormat, Timebase>> {
        &self.checked
    }
}

impl<Prompt: Eq, OutputProfile: Eq> ImageRequest<Prompt, OutputProfile> {
    pub fn new(
        binding: RequestBinding<RequestBasis<ImageSemantics<Prompt, OutputProfile>>>,
        payload: &[u8],
        usage: RequestUsage,
        limits: RequestLimits,
        owner: RequestOwnerState<'_, RequestBasis<ImageSemantics<Prompt, OutputProfile>>>,
    ) -> Result<Self, ModalityRequestError> {
        let checked = checked_for(
            RequestModality::Image,
            binding,
            payload,
            usage,
            limits,
            owner,
        )?;
        Ok(Self { checked })
    }

    pub fn checked(&self) -> &CheckedModalityRequest<ImageSemantics<Prompt, OutputProfile>> {
        &self.checked
    }
}

impl<Prompt: Eq, OutputProfile: Eq, ReferenceFrame: Eq>
    VideoRequest<Prompt, OutputProfile, ReferenceFrame>
{
    pub fn new(
        binding: RequestBinding<
            RequestBasis<VideoSemantics<Prompt, OutputProfile, ReferenceFrame>>,
        >,
        payload: &[u8],
        usage: RequestUsage,
        limits: RequestLimits,
        owner: RequestOwnerState<
            '_,
            RequestBasis<VideoSemantics<Prompt, OutputProfile, ReferenceFrame>>,
        >,
    ) -> Result<Self, ModalityRequestError> {
        let checked = checked_for(
            RequestModality::Video,
            binding,
            payload,
            usage,
            limits,
            owner,
        )?;
        Ok(Self { checked })
    }

    pub fn checked(
        &self,
    ) -> &CheckedModalityRequest<VideoSemantics<Prompt, OutputProfile, ReferenceFrame>> {
        &self.checked
    }
}

impl<Intent: Eq, OutputProfile: Eq, LoopMetadata: Eq>
    SoundRequest<Intent, OutputProfile, LoopMetadata>
{
    pub fn new(
        binding: RequestBinding<RequestBasis<SoundSemantics<Intent, OutputProfile, LoopMetadata>>>,
        payload: &[u8],
        usage: RequestUsage,
        limits: RequestLimits,
        owner: RequestOwnerState<
            '_,
            RequestBasis<SoundSemantics<Intent, OutputProfile, LoopMetadata>>,
        >,
    ) -> Result<Self, ModalityRequestError> {
        let checked = checked_for(
            RequestModality::Sound,
            binding,
            payload,
            usage,
            limits,
            owner,
        )?;
        Ok(Self { checked })
    }

    pub fn checked(
        &self,
    ) -> &CheckedModalityRequest<SoundSemantics<Intent, OutputProfile, LoopMetadata>> {
        &self.checked
    }
}

impl<Semantic> fmt::Debug for CheckedModalityRequest<Semantic> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CheckedModalityRequest")
            .finish_non_exhaustive()
    }
}

impl<Semantic: Eq> CheckedModalityRequest<Semantic> {
    /// Constructs the envelope through the canonical checked request boundary.
    pub fn new(
        binding: RequestBinding<RequestBasis<Semantic>>,
        payload: &[u8],
        usage: RequestUsage,
        limits: RequestLimits,
        owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
    ) -> Result<Self, RequestError> {
        let checked = CheckedRequest::new(binding, payload, usage, limits, owner)?;
        Ok(Self { checked })
    }

    /// Rechecks the unchanged canonical binding and the consumer's current basis.
    pub fn validate_current(
        &self,
        owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
    ) -> Result<(), RequestError> {
        self.checked.validate_current(owner)
    }
}

impl<Semantic> CheckedModalityRequest<Semantic> {
    pub fn binding(&self) -> &RequestBinding<RequestBasis<Semantic>> {
        self.checked.binding()
    }

    /// Returns the unchanged payload bytes for an authorized consumer.
    pub fn payload(&self) -> &[u8] {
        self.checked.payload()
    }

    pub fn usage(&self) -> RequestUsage {
        self.checked.usage()
    }

    pub fn limits(&self) -> RequestLimits {
        self.checked.limits()
    }

    pub fn into_parts(
        self,
    ) -> (
        RequestBinding<RequestBasis<Semantic>>,
        Vec<u8>,
        RequestUsage,
    ) {
        self.checked.into_parts()
    }
}
