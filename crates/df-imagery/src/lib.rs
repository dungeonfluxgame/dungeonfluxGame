//! Validated image request, configuration, output and transform contracts.
//!
//! This crate freezes the vocabulary of image work: what a Generate, Edit or
//! Transform asks for, how it is configured, and what delivery it expects. It
//! performs no provider calls, pixel work, storage, scheduling or money
//! movement; a later service owner executes these contracts and maps the
//! caller-claimed source pins onto `df-assets` at the publication boundary.

mod config;
mod journal;
mod output;
mod policy;
mod request;
mod transform;

pub use config::{
    AccountHandle, BackendGate, BackendId, ConfigError, GlobalMode, ImageConfig, PurposeGate,
    SchedulerBounds, TransformLimits,
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
    AssetDigest, DEFAULT_MAX_COMPOSITE_LAYERS, DEFAULT_MAX_PROMPT_CHARS, DEFAULT_MAX_REFERENCES,
    DEFAULT_MAX_TRANSFORM_STEPS, EditSpec, ExecutionMode, Freshness, GenerateSpec, ImageOperation,
    ImageOwner, ImageRequest, OperationIdentity, Purpose, PurposedUse, RequestError, RequestLimits,
    SourceRef, TextPrompt, Urgency, VariantRole,
};
pub use transform::{
    AlignAnchor, Anchor1D, BlendMode, BlurAlgorithm, BlurSpec, BrightnessOffset, CanvasSpec,
    ColorEffect, CompositeLayer, CoverageRef, ExposureStops, FlipAxis, ImageTransform,
    Interpolation, MaskChannel, MaskMode, MaskSource, MaskSpec, PadSpec, PixelRect, ResizeMode,
    ResizeSpec, RotateSpec, SharpenSpec, TransformError, TransformSource, TransformSpec,
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
