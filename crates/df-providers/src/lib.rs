//! Dated supplier route evidence for native provider adapters.
//!
//! Published rates and model pages are research evidence. They do not establish
//! account access, current capacity, rights, or permission to dispatch provider work.

/// OpenAI Images Edit `prompt` maximum (`maxLength: 32000`), documented at:
/// <https://developers.openai.com/api/reference/resources/images/methods/edit>.
pub const IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS: usize = 32_000;

mod http_contract;
mod output_fetch;
mod reconciliation;
mod registry;

#[cfg(not(target_arch = "wasm32"))]
mod native_http;
#[cfg(not(target_arch = "wasm32"))]
mod speech_adapter;
#[cfg(not(target_arch = "wasm32"))]
mod systemone;
#[cfg(not(target_arch = "wasm32"))]
mod text_response;
#[cfg(not(target_arch = "wasm32"))]
mod text_schema;
#[cfg(not(target_arch = "wasm32"))]
pub use systemone::{SystemOneChoiceProvider, SystemOneError};

#[cfg(not(target_arch = "wasm32"))]
pub use reconciliation::{reconcile_speech_attempt, reconcile_stt_attempt};
#[cfg(not(target_arch = "wasm32"))]
pub use registry::registered_speech_provider;
#[cfg(not(target_arch = "wasm32"))]
pub use speech_adapter::{
    ELEVEN_STT_LANGUAGE, ELEVEN_STT_MODEL, ELEVEN_TTS_LANGUAGE, ELEVEN_TTS_MODEL,
    ElevenSpeechBasis, ElevenSpeechRequest, ElevenSttBasis, ElevenSttRequest, EncodedSpeechAudio,
    NativeSpeechObservation, NativeSpeechProvider, PreparedSpeechResponse, PreparedSttResponse,
    SpeechAdapterError, SpeechAdapterReason, SpeechInputFormat, SpeechOutputFormat,
    SpeechResponseMetadata, SpeechTranscriptProfile, TranscriptWord, TranscriptWordKind,
    ValidatedTranscript,
};

#[cfg(not(target_arch = "wasm32"))]
pub use native_http::{NativeHttpLimits, NativeHttpRefusal, ProviderLimits};
#[cfg(not(target_arch = "wasm32"))]
pub use registry::registered_text_provider;
#[cfg(not(target_arch = "wasm32"))]
pub use text_response::{
    NativeTextProvider, PreparedTextResponse, ReasoningEffort, TextProviderConfig,
    TextResponseContract, TextResponseError, TextResponseObservation, TextTokenUsage,
    ValidatedTextOutput,
};
#[cfg(not(target_arch = "wasm32"))]
pub use text_schema::{TextOutputSchema, TextSchemaError, TextSchemaLimits};

pub use df_provider_api::ProviderFailureClass;
pub use http_contract::{
    ElevenLabsAudioObservation, FalQueueObservation, FalSubmitObservation, HttpMethod, HttpRequest,
    HttpRequestError, ProviderImageMetadata, ProviderRequestId, classify_fal_status,
    classify_fal_status_response, classify_fal_submit_response, elevenlabs_stream_request,
    fal_result_request, fal_status_request, fal_submit_request, validate_elevenlabs_audio,
    validate_fal_images,
};
pub use reconciliation::{
    ProviderAttemptIdentity, ReconciliationCapability, ReconciliationInput, ReconciliationLookup,
    reconcile_provider_attempt, reconciliation_capability,
};
pub use registry::{
    BillingUnit, CandidateRoute, CandidateRouteId, Capability, DispatchBlock, EvidenceReference,
    Gate, GateStatus, LicenseEvidence, PublishedRate, RoundingRule, UnsupportedCapability,
    assess_dispatch_gates, candidate_for, candidate_routes,
};

pub use output_fetch::{
    DecodedImageFacts, DecoderPolicy, DecoderRequirements, DnsInspection, FetchResponseFacts,
    OutputFetchPlan, OutputFetchRefusal, decode_output, fetch_output, inspect_decoded_image,
    inspect_dns_answers, inspect_fetch_response, prepare_output_fetch_plan,
};

#[cfg(not(target_arch = "wasm32"))]
mod fal_media;
#[cfg(not(target_arch = "wasm32"))]
mod media_container;
#[cfg(not(target_arch = "wasm32"))]
mod native_media_fetch;
#[cfg(not(target_arch = "wasm32"))]
pub use fal_media::{
    EncodedFalMedia, FalCancellationObservation, FalHttpFacts, FalImageBasis, FalImageJob,
    FalImageProfile, FalImageRequest, FalImageSemantics, FalLoopFrames, FalMediaCompletion,
    FalMediaConfig, FalMediaError, FalMediaFamily, FalMediaJob, FalMediaReason, FalReferenceFrame,
    FalSoundBasis, FalSoundJob, FalSoundProfile, FalSoundRequest, FalSoundSemantics, FalVideoBasis,
    FalVideoCompletionEvent, FalVideoJob, FalVideoProfile, FalVideoRequest, FalVideoSemantics,
    KLING_O1_IMAGE_TO_VIDEO, NativeFalMediaProvider, PreparedFalMediaResponse,
};
#[cfg(not(target_arch = "wasm32"))]
pub use media_container::{EncodedMediaFacts, MediaContainerRefusal, MediaTimebase};
#[cfg(not(target_arch = "wasm32"))]
pub use native_media_fetch::FalCdnPin;
#[cfg(not(target_arch = "wasm32"))]
pub use registry::registered_fal_media_provider;
#[cfg(not(target_arch = "wasm32"))]
mod head_locate;
#[cfg(not(target_arch = "wasm32"))]
pub use head_locate::{
    HeadBox, HeadLocateConfig, HeadLocateError, LocateTarget, LocatedHead, NativeHeadLocator,
};
#[cfg(not(target_arch = "wasm32"))]
mod embeddings;
#[cfg(not(target_arch = "wasm32"))]
pub use embeddings::{
    EMBEDDING_DIMENSIONS, EMBEDDING_MAXIMUM_INPUT_CHARACTERS, EMBEDDING_MAXIMUM_INPUTS,
    EMBEDDING_MODEL_SMALL, EmbeddingBatch, EmbeddingConfig, EmbeddingError,
    NativeEmbeddingProvider,
};
#[cfg(not(target_arch = "wasm32"))]
mod image_edit;
#[cfg(not(target_arch = "wasm32"))]
pub use image_edit::{
    GeneratedImage, ImageBackground, ImageEdit, ImageEditConfig, ImageEditError, ImageEditModel,
    ImageFormat, ImageModeration, ImageQuality, ImageUsage, NativeImageEditProvider,
    ReferenceImage,
};
#[cfg(not(target_arch = "wasm32"))]
mod fal_image;
#[cfg(not(target_arch = "wasm32"))]
pub use fal_image::{
    BACKGROUND_REMOVAL, FLUX_KONTEXT_MULTI, FLUX_ULTRA, FalImageConfig, NativeFalImageProvider,
};
#[cfg(not(target_arch = "wasm32"))]
mod eleven_music;
#[cfg(not(target_arch = "wasm32"))]
pub use eleven_music::{
    AMBIENCE_MAXIMUM_SECONDS, ElevenMusicConfig, ElevenMusicError, GeneratedAudio,
    MUSIC_MAXIMUM_LENGTH_MILLIS, MUSIC_MAXIMUM_PROMPT_CHARACTERS, MUSIC_MINIMUM_LENGTH_MILLIS,
    NativeElevenMusicProvider,
};
#[cfg(not(target_arch = "wasm32"))]
mod fal_segment;
#[cfg(not(target_arch = "wasm32"))]
pub use fal_segment::{
    FalSegmentConfig, FalSegmentError, MAXIMUM_MASKS, MaskError, MaskRows, MaskRun,
    NativeFalSegmenter, PixelBox, SegmentAnswer, SegmentInstance, SegmentModel, SegmentRequest,
};
#[cfg(not(target_arch = "wasm32"))]
mod plate_judge;
#[cfg(not(target_arch = "wasm32"))]
pub use plate_judge::{
    JudgeAnswer, JudgePicture, NativePlateJudge, PhotoDress, PhotoFrame, PhotoJudgeKind,
    PhotoMedium, PhotoPeopleCount, PhotoQualityAnswer, PhotoText, PlateJudgeConfig,
    PlateJudgeError,
};
