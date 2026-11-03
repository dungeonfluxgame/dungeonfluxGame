//! Native image contracts, durable demand, configuration and private execution backends.
//! The consumer service and application migration are being integrated.

#![cfg(not(target_arch = "wasm32"))]

mod artwork;
pub use artwork::{
    ArtworkBinding, ArtworkError, ArtworkSubject, ArtworkSubjectKey, CAMPAIGN_ARTWORK_SPEC_VERSION,
    CampaignArtworkSpec, MAXIMUM_ARTWORK_SUBJECT_BYTES, RunSelectionFence,
};

mod config;
mod journal;
mod output;
mod policy;
mod request;
mod transform;

pub use config::{
    AccountHandle, Accounting, AccountingMode, AnalysisPreparationLimits, AnalysisSettings,
    ApiEndpoint, BackendGate, BackendId, CliProfileRevision, CodexExecutable, CodexSettings,
    ConfigError, GenerationQuality, GlobalMode, ImageConfig, JobWorkDir, ModelName,
    NamedOutputProfile, OutputProfileName, PriceFileRef, PurposeGate, QualityDefaults,
    QuotaSettings, RetryPolicy, SchedulerBounds, SegmentationProvider, SegmentationRoute,
    TransformLimits,
};
pub use df_assets::{AssetId, Audience, MemberSet};
pub use journal::{
    AcceptCommand, AcceptOutcome, AcceptTarget, FreezeCommand, FreezeOutcome, ImageInterest,
    ImageInterestId, ImageJob, ImageJobId, ImageJournal, InterestState, JobState, JoinRefusal,
    JournalError, JournalIdError, OperationReceipt, OperationView, SharingIdentity, SnapshotError,
    VersionedSnapshot, WithdrawOutcome, Withdrawal, is_expired, owner_of_bytes, owner_scope_bytes,
};
pub use output::{
    AlphaHandling, DEFAULT_JPEG_QUALITY_0_100, Dimensions, Encoding, LossyQuality,
    MAXIMUM_OUTPUT_PIXELS, OutputError, OutputFormat, OutputProfile, RasterOutputSpec, Rgb,
};
pub use policy::{PolicyDecision, PolicyInput, Refusal, decide};
pub use request::{
    AnalysisRoute, AssetDigest, CreatorClassification, DEFAULT_MAX_COMPOSITE_LAYERS,
    DEFAULT_MAX_PROMPT_CHARS, DEFAULT_MAX_REFERENCES, DEFAULT_MAX_TRANSFORM_STEPS, EditSpec,
    ExecutionMode, Freshness, GenerateSpec, ImageContentRating, ImageInspection, ImageOperation,
    ImageOwner, ImagePipeline, ImageRequest, InspectionKind, InspectionRequirement,
    OperationIdentity, PipelineError, PrivateImageText, ProviderModeration, Purpose, PurposedUse,
    RequestError, RequestLimits, SourceRef, StagedIdentity, TextPrompt, Urgency, VariantRole,
};
pub use transform::{
    AlignAnchor, Anchor1D, BlendMode, BlurAlgorithm, BlurSpec, BrightnessOffset, CanvasSpec,
    ColorEffect, CompositeLayer, CoverageRef, EncodedAlphaBlend, EncodedFilter, ExposureStops,
    FlipAxis, ImageTransform, Interpolation, MaskChannel, MaskMode, MaskSource, MaskSpec, PadSpec,
    PixelRect, ResizeMode, ResizeSpec, RotateSpec, SharpenSpec, TransformError, TransformSource,
    TransformSpec,
};

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod output_tests;
#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod request_tests;
#[cfg(test)]
mod transform_tests;

mod config_control;
pub use config_control::{
    AppliedLocal, ConfigApplyError, ConfigControl, PersistedExport, PreparedExport,
};
#[cfg(test)]
mod config_control_tests;

mod config_overlay;
pub use config_overlay::{
    AnalysisRouteFields, AnalysisRoutePatch, BodyField, FieldProvenance, OverlayError,
    OverlaySource, ProfileEdit, Provenance, ResolvedConfig, RuntimePatch, SegmentationRouteFields,
    SegmentationRoutePatch, apply_runtime, parse_runtime_value,
};
#[cfg(test)]
mod config_overlay_tests;

mod intent_codec;
#[cfg(test)]
mod intent_codec_tests;

#[cfg(not(target_arch = "wasm32"))]
mod backends;

#[cfg(not(target_arch = "wasm32"))]
mod raster;
pub use raster::decode::{SourceHeader, SourceHeaderError, source_header};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod raster_tests;

mod service;
pub use service::artifact_recipe::{
    CampaignArtifactRecipe, campaign_artifact_recipe, project_retained_campaign_artifact,
    retained_campaign_artifact_recipe,
};
pub use service::cache::{
    COMPATIBLE_KEY_VERSION, CompatibleCursor, CompatibleKey, MAXIMUM_COMPATIBLE_CANDIDATES,
    compatible_key, compatible_lookup_key, compatible_snapshots,
};
pub use service::{
    AnalysisFacing, AnalysisFacingDirection, AnalysisIdentityMatch, AnalysisLocatedHead,
    AnalysisMask, AnalysisMaskRun, AnalysisMaskSet, AnalysisReference, AnalysisTarget,
    AnalysisUnit, AnalysisValueError, AuthDenied, CancelOutcome, CancelReceipt, CreatorAssessment,
    CreatorCategory, CreatorRoute, HeadLocationsSpec, ImageAnalysisObservation,
    ImageAnalysisOperation, ImageAnalysisRequest, ImageAnalysisResult, ImageAuthorizer,
    ImageDemandView, ImageExecutionState, ImageInspectionObservation, ImageObservations,
    ImageProgress, ImageService, ImageServiceApi, ImageServiceError, ImageServiceFuture,
    ImageUsageObservation, NoImageObservations, NormalizedImageBox, OperationHandle,
    PhotoFeatureBinding, PhotoInspectionAssessment, PhotoInspectionFailure, PhotoInspectionSpec,
    PhotoInspectionUnjudgedReason, PhotoInspectionVerdict, PhotoVisibleFeature, RequestOutcome,
    RequestProblem, SegmentationPreparation, SegmentationPrompt, SegmentationSpec, ServiceAction,
    ServiceClock, ServiceIdError, ServiceIds, StagingFacingSpec, StagingIdentitySpec,
    StoredAnalysisAuthorization, StoredImageAuthorization, WatchObservation,
};
#[cfg(test)]
mod service_tests;

/// Projection into the host-owned cost book.
pub mod cost;

/// Durable stage and dispatch infrastructure contracts.
pub mod execution;

pub mod oneshot;

pub mod publication;

pub mod runtime;

/// Structured imagery lifecycle diagnostics through shared instrumentation.
pub mod telemetry;

/// Local-only infrastructure fixtures for consumer regression tests.
#[cfg(feature = "test-support")]
pub mod test_support;
