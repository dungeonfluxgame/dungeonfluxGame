//! Decoder half of the intent codec: bounded reconstruction of one consumer
//! intent and one config snapshot through the validated constructors, so a
//! restart replays checked work. Every vector count proves its minimum wire
//! width against the remaining input before anything is reserved.

use df_assets::{AssetId, Audience, MemberSet};
use df_types::{AccountId, CampaignId, MemberId, OperationId, RunId};

use crate::config::{
    Accounting, AccountingMode, AnalysisPreparationLimits, AnalysisSettings, BackendGate,
    BackendId, CliProfileRevision, CodexExecutable, CodexSettings, GenerationQuality, GlobalMode,
    ImageConfig, JobWorkDir, NamedOutputProfile, OutputProfileName, PriceFileRef, PurposeGate,
    QualityDefaults, QuotaSettings, RetryPolicy, SchedulerBounds, SegmentationProvider,
    SegmentationRoute, TransformLimits,
};
use crate::output::{
    AlphaHandling, Dimensions, Encoding, LossyQuality, OutputProfile, RasterOutputSpec, Rgb,
};
use crate::request::{
    AssetDigest, EditSpec, ExecutionMode, Freshness, GenerateSpec, ImageOperation, ImageOwner,
    Purpose, PurposedUse, RequestError, RequestLimits, SourceRef, TextPrompt, VariantRole,
};
use crate::transform::{
    AlignAnchor, Anchor1D, BlendMode, BlurAlgorithm, BlurSpec, BrightnessOffset, CanvasSpec,
    ColorEffect, CompositeLayer, CoverageRef, EncodedAlphaBlend, EncodedFilter, ExposureStops,
    FlipAxis, ImageTransform, Interpolation, MaskChannel, MaskMode, MaskSource, MaskSpec, PadSpec,
    PixelRect, ResizeMode, ResizeSpec, RotateSpec, SharpenSpec, TransformError, TransformField,
    TransformSource, TransformSpec,
};

use super::primitives::{
    ENCODED_STEP_REVISION, MAXIMUM_CLI_PROFILE_BYTES, MAXIMUM_CODEX_PATH_BYTES,
    MAXIMUM_PRICE_FILE_BYTES, MAXIMUM_PROFILE_NAME_BYTES, MIN_LAYER_BYTES, MIN_MEMBER_BYTES,
    MIN_PROFILE_BYTES, MIN_SOURCE_REF_BYTES, MIN_STEP_BYTES, Reader, decode_account_handle,
    decode_api_endpoint, decode_model_name, decode_moderation, decode_route, prompt_byte_bound,
};
use super::{CONFIG_CODEC_VERSION, CodecError, DecodedIntent};

/// Frozen analysis preparation bounds of snapshots written before version 9.
const LEGACY_ANALYSIS_PREPARATION: AnalysisPreparationLimits = AnalysisPreparationLimits {
    max_pixels: 20_000_000,
    max_picture_bytes: 1_048_576,
    max_request_bytes: 16_777_216,
};
/// Frozen segmentation preparation bounds of snapshots written before version 10.
const LEGACY_SEGMENTATION_PREPARATION: AnalysisPreparationLimits = AnalysisPreparationLimits {
    max_pixels: 20_000_000,
    max_picture_bytes: 3_145_728,
    max_request_bytes: 6_291_456,
};
/// Frozen quota policy before quota settings joined the snapshot in version 11.
const LEGACY_QUOTA: QuotaSettings = QuotaSettings {
    enabled: true,
    cooldown_ms: 900_000,
    maximum_accounts: 1_024,
};
/// Frozen Codex receipt bounds before they joined the snapshot in versions 4 and 5.
const LEGACY_CODEX_STDOUT_BYTES: u64 = 4 * 1024 * 1024;
const LEGACY_CODEX_STDERR_BYTES: u64 = 256 * 1024;
const LEGACY_CODEX_MESSAGE_BYTES: u64 = 1024 * 1024;

/// Proves `count` items of at least `min_width_bytes` each fit in
/// `remaining_bytes` before the caller reserves. The multiplication is checked
/// and the comparison is against unread input, so the reservation that follows
/// is bounded by bytes already in hand.
fn proven_count(
    count: usize,
    min_width_bytes: usize,
    remaining_bytes: usize,
) -> Result<usize, CodecError> {
    let need = count
        .checked_mul(min_width_bytes)
        .ok_or(CodecError::TooLong)?;
    if need > remaining_bytes {
        return Err(CodecError::TooLong);
    }
    Ok(count)
}

/// Rebuilds the consumer intent from canonical bytes for restart
/// reconstruction. Rejects a version mismatch, malformed or trailing bytes
/// and any value the validated constructors refuse under `limits`.
pub(crate) fn decode_intent(
    bytes: &[u8],
    limits: &RequestLimits,
    max_bytes: usize,
) -> Result<DecodedIntent, CodecError> {
    limits.validate().map_err(CodecError::Request)?;
    if bytes.len() > max_bytes {
        return Err(CodecError::TooLong);
    }
    let mut input = Reader::new(bytes);
    let version = super::intent_version(bytes)?;
    input.byte()?;
    let purpose = Purpose::from_index(usize::from(input.byte()?)).ok_or(CodecError::Malformed)?;
    let role = decode_role(input.byte()?)?;
    let operation_kind = input.byte()?;
    let owner = decode_owner(&mut input)?;
    let mut operation = decode_operation(&mut input, operation_kind, limits)?;
    let execution = decode_execution(input.byte()?)?;
    let freshness = decode_freshness(&mut input)?;
    let audience = decode_audience(&mut input)?;
    if matches!(audience, Audience::Withheld) {
        return Err(CodecError::Request(RequestError::WithheldAudience));
    }
    let expected_source_revision = input.u64_be()?;
    let output = decode_output(&mut input)?;
    match (&operation, &output) {
        (ImageOperation::Transform(_), OutputProfile::Local { .. })
        | (ImageOperation::Generate(_), OutputProfile::Provider { .. })
        | (ImageOperation::Edit(_), OutputProfile::Provider { .. }) => {}
        _ => return Err(CodecError::Request(RequestError::ProfileMismatch)),
    }
    let pipeline = if version == super::ARTWORK_INTENT_CODEC_VERSION {
        if input.boolean()? {
            Some(super::pipeline::decode(
                &mut input,
                limits,
                super::PIPELINE_INTENT_CODEC_VERSION,
            )?)
        } else {
            None
        }
    } else if version >= super::LEGACY_PIPELINE_INTENT_CODEC_VERSION {
        Some(super::pipeline::decode(&mut input, limits, version)?)
    } else {
        None
    };
    if let Some(pipeline) = pipeline {
        operation = match operation {
            ImageOperation::Generate(mut spec) => {
                spec.prompt = spec
                    .prompt
                    .with_pipeline(pipeline)
                    .map_err(CodecError::Request)?;
                ImageOperation::Generate(spec)
            }
            ImageOperation::Edit(mut spec) => {
                spec.prompt = spec
                    .prompt
                    .with_pipeline(pipeline)
                    .map_err(CodecError::Request)?;
                ImageOperation::Edit(spec)
            }
            ImageOperation::Transform(_) => return Err(CodecError::Malformed),
        };
    }
    if version == super::ARTWORK_INTENT_CODEC_VERSION {
        let artwork = super::artwork::decode(&mut input)?;
        artwork
            .validate_for(&owner, PurposedUse { purpose, role }, &audience)
            .map_err(RequestError::InvalidArtwork)
            .map_err(CodecError::Request)?;
        operation = match operation {
            ImageOperation::Generate(mut spec) => {
                spec.prompt = spec.prompt.with_artwork(artwork);
                ImageOperation::Generate(spec)
            }
            ImageOperation::Edit(mut spec) => {
                spec.prompt = spec.prompt.with_artwork(artwork);
                ImageOperation::Edit(spec)
            }
            ImageOperation::Transform(_) => return Err(CodecError::Malformed),
        };
    }
    input.finish()?;
    Ok(DecodedIntent {
        owner,
        purpose: PurposedUse { purpose, role },
        operation,
        execution,
        freshness,
        audience,
        expected_source_revision,
        output,
    })
}

pub(crate) fn decode_role(tag: u8) -> Result<VariantRole, CodecError> {
    match tag {
        0 => Ok(VariantRole::Primary),
        1 => Ok(VariantRole::Thumbnail),
        2 => Ok(VariantRole::Mask),
        3 => Ok(VariantRole::Reference),
        _ => Err(CodecError::Malformed),
    }
}

pub(crate) fn decode_owner(input: &mut Reader<'_>) -> Result<ImageOwner, CodecError> {
    match input.byte()? {
        0 => {
            let campaign =
                CampaignId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            let run = RunId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            Ok(ImageOwner::Run { campaign, run })
        }
        1 => {
            let account =
                AccountId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            Ok(ImageOwner::Draft { account })
        }
        _ => Err(CodecError::Malformed),
    }
}

pub(crate) fn decode_audience(input: &mut Reader<'_>) -> Result<Audience, CodecError> {
    match input.byte()? {
        1 => Ok(Audience::PublicStatic),
        2 => {
            let campaign =
                CampaignId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            Ok(Audience::CampaignShared(campaign))
        }
        3 => {
            let campaign =
                CampaignId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            let members = decode_members(input)?;
            Ok(Audience::Members(campaign, members))
        }
        4 => {
            let campaign =
                CampaignId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            Ok(Audience::Host(campaign))
        }
        5 => {
            let account =
                AccountId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            Ok(Audience::OwnerPrivate(account))
        }
        _ => Err(CodecError::Malformed),
    }
}

fn decode_members(input: &mut Reader<'_>) -> Result<MemberSet, CodecError> {
    let count = usize::try_from(input.u32_be()?).map_err(|_| CodecError::TooLong)?;
    let count = proven_count(count, MIN_MEMBER_BYTES, input.remaining())?;
    let mut members = Vec::with_capacity(count);
    for _ in 0..count {
        let member = MemberId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
        members.push(member);
    }
    MemberSet::new(members).ok_or(CodecError::Malformed)
}

pub(crate) fn decode_source_ref(input: &mut Reader<'_>) -> Result<SourceRef, CodecError> {
    let asset = AssetId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
    let digest = AssetDigest::from_bytes(
        input
            .take(32)?
            .try_into()
            .map_err(|_| CodecError::Truncated)?,
    );
    let audience = decode_audience(input)?;
    Ok(SourceRef {
        asset,
        digest,
        audience,
    })
}

pub(crate) fn decode_execution(tag: u8) -> Result<ExecutionMode, CodecError> {
    match tag {
        0 => Ok(ExecutionMode::Live),
        1 => Ok(ExecutionMode::PreparedOnly),
        2 => Ok(ExecutionMode::Replay),
        _ => Err(CodecError::Malformed),
    }
}

pub(crate) fn decode_freshness(input: &mut Reader<'_>) -> Result<Freshness, CodecError> {
    match input.byte()? {
        0 => Ok(Freshness::ReuseCompatible),
        1 => {
            let nonce =
                OperationId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            Ok(Freshness::Regenerate { nonce })
        }
        _ => Err(CodecError::Malformed),
    }
}

fn decode_prompt(input: &mut Reader<'_>, limits: &RequestLimits) -> Result<TextPrompt, CodecError> {
    let template_version = input.u32_be()?;
    let text = input.text(prompt_byte_bound(limits))?;
    let len_chars = text.chars().count();
    if len_chars > limits.max_prompt_chars {
        return Err(CodecError::Request(RequestError::PromptTooLong {
            len_chars,
            max_chars: limits.max_prompt_chars,
        }));
    }
    TextPrompt::new(text, template_version).map_err(CodecError::Request)
}

fn decode_references(
    input: &mut Reader<'_>,
    limits: &RequestLimits,
) -> Result<Vec<SourceRef>, CodecError> {
    let count = usize::try_from(input.u32_be()?).map_err(|_| CodecError::TooLong)?;
    if count > limits.max_references {
        return Err(CodecError::Request(RequestError::TooManyReferences {
            count,
        }));
    }
    let count = proven_count(count, MIN_SOURCE_REF_BYTES, input.remaining())?;
    let mut references = Vec::with_capacity(count);
    for _ in 0..count {
        references.push(decode_source_ref(input)?);
    }
    Ok(references)
}

fn decode_operation(
    input: &mut Reader<'_>,
    kind: u8,
    limits: &RequestLimits,
) -> Result<ImageOperation, CodecError> {
    match kind {
        0 => {
            let prompt = decode_prompt(input, limits)?;
            let references = decode_references(input, limits)?;
            Ok(ImageOperation::Generate(GenerateSpec {
                prompt,
                references,
            }))
        }
        1 => {
            let prompt = decode_prompt(input, limits)?;
            let base = decode_source_ref(input)?;
            let references = decode_references(input, limits)?;
            Ok(ImageOperation::Edit(EditSpec {
                prompt,
                base,
                references,
            }))
        }
        2 => {
            let spec = decode_transform(input, limits)?;
            Ok(ImageOperation::Transform(spec))
        }
        _ => Err(CodecError::Malformed),
    }
}

fn decode_output(input: &mut Reader<'_>) -> Result<OutputProfile, CodecError> {
    match input.byte()? {
        0 => {
            let generated =
                Dimensions::new(input.u32_be()?, input.u32_be()?).map_err(CodecError::Output)?;
            let delivered = decode_raster(input)?;
            Ok(OutputProfile::provider(generated, delivered))
        }
        1 => {
            let delivered = decode_raster(input)?;
            Ok(OutputProfile::local(delivered))
        }
        _ => Err(CodecError::Malformed),
    }
}

fn decode_raster(input: &mut Reader<'_>) -> Result<RasterOutputSpec, CodecError> {
    let dimensions =
        Dimensions::new(input.u32_be()?, input.u32_be()?).map_err(CodecError::Output)?;
    let alpha = match input.byte()? {
        0 => AlphaHandling::Opaque,
        1 => AlphaHandling::PreserveAlpha,
        2 => AlphaHandling::Matte(decode_rgb(input)?),
        _ => return Err(CodecError::Malformed),
    };
    let encoding = match input.byte()? {
        0 => Encoding::Jpeg {
            quality: LossyQuality::new(input.byte()?).map_err(CodecError::Output)?,
        },
        1 => Encoding::WebP {
            quality: match input.byte()? {
                0 => None,
                1 => Some(LossyQuality::new(input.byte()?).map_err(CodecError::Output)?),
                _ => return Err(CodecError::Malformed),
            },
        },
        2 => Encoding::Png,
        _ => return Err(CodecError::Malformed),
    };
    RasterOutputSpec::new(dimensions, alpha, encoding).map_err(CodecError::Output)
}

fn decode_rgb(input: &mut Reader<'_>) -> Result<Rgb, CodecError> {
    let bytes = input.take(3)?;
    Ok(Rgb {
        red: bytes[0],
        green: bytes[1],
        blue: bytes[2],
    })
}

fn decode_transform(
    input: &mut Reader<'_>,
    limits: &RequestLimits,
) -> Result<TransformSpec, CodecError> {
    let source = decode_transform_source(input)?;
    let count = usize::try_from(input.u32_be()?).map_err(|_| CodecError::TooLong)?;
    if count > limits.max_transform_steps {
        return Err(CodecError::Request(RequestError::InvalidTransform(
            TransformError::TooManySteps,
        )));
    }
    let count = proven_count(count, MIN_STEP_BYTES, input.remaining())?;
    let mut steps = Vec::with_capacity(count);
    for _ in 0..count {
        steps.push(decode_step(input, limits)?);
    }
    let spec = TransformSpec { source, steps };
    spec.validate(limits)
        .map_err(RequestError::InvalidTransform)
        .map_err(CodecError::Request)?;
    Ok(spec)
}

fn decode_transform_source(input: &mut Reader<'_>) -> Result<TransformSource, CodecError> {
    match input.byte()? {
        0 => Ok(TransformSource::Image(decode_source_ref(input)?)),
        1 => {
            let canvas = CanvasSpec::new(input.u32_be()?, input.u32_be()?, decode_rgb(input)?)
                .map_err(transform_error)?;
            Ok(TransformSource::Canvas(canvas))
        }
        2 => Ok(TransformSource::Coverage(decode_coverage(input)?)),
        3 => {
            let analyzed = decode_source_ref(input)?;
            let owner = decode_owner(input)?;
            let operation = df_types::OperationId::from_bytes(input.take(16)?)
                .map_err(|_| CodecError::Malformed)?;
            let prompt = match input.byte()? {
                0 => crate::SegmentationPrompt::Person,
                1 => crate::SegmentationPrompt::Head,
                _ => return Err(CodecError::Malformed),
            };
            Ok(TransformSource::SegmentationMask {
                analyzed,
                segmentation: crate::OperationHandle { owner, operation },
                prompt,
                mask_index: input.u32_be()?,
                contract_revision: input.u32_be()?,
            })
        }
        _ => Err(CodecError::Malformed),
    }
}

fn decode_coverage(input: &mut Reader<'_>) -> Result<CoverageRef, CodecError> {
    Ok(CoverageRef {
        coverage: decode_source_ref(input)?,
        analyzed: decode_source_ref(input)?,
        analysis_revision: input.u32_be()?,
    })
}

fn transform_error(error: TransformError) -> CodecError {
    CodecError::Request(RequestError::InvalidTransform(error))
}

fn decode_step(
    input: &mut Reader<'_>,
    limits: &RequestLimits,
) -> Result<ImageTransform, CodecError> {
    match input.byte()? {
        0 => Ok(ImageTransform::Resize(
            ResizeSpec::new(
                input.u32_be()?,
                input.u32_be()?,
                decode_resize_mode(input.byte()?)?,
                decode_anchor(input)?,
                decode_interpolation(input.byte()?)?,
                input.boolean()?,
                match input.byte()? {
                    0 => None,
                    1 => Some(decode_rgb(input)?),
                    _ => return Err(CodecError::Malformed),
                },
            )
            .map_err(transform_error)?,
        )),
        1 => Ok(ImageTransform::Crop(
            PixelRect::new(
                input.u32_be()?,
                input.u32_be()?,
                input.u32_be()?,
                input.u32_be()?,
            )
            .map_err(transform_error)?,
        )),
        2 => Ok(ImageTransform::Pad(
            PadSpec::new(
                input.u32_be()?,
                input.u32_be()?,
                decode_anchor(input)?,
                decode_rgb(input)?,
            )
            .map_err(transform_error)?,
        )),
        3 => Ok(ImageTransform::Rotate(decode_rotate(input)?)),
        4 => Ok(ImageTransform::Flip(match input.byte()? {
            0 => FlipAxis::Horizontal,
            1 => FlipAxis::Vertical,
            _ => return Err(CodecError::Malformed),
        })),
        5 => Ok(ImageTransform::Brighten(
            BrightnessOffset::new(input.float(TransformField::Brightness.key())?)
                .map_err(transform_error)?,
        )),
        6 => Ok(ImageTransform::Expose(
            ExposureStops::new(input.float(TransformField::Exposure.key())?)
                .map_err(transform_error)?,
        )),
        7 => Ok(ImageTransform::Color(decode_color(input)?)),
        8 => Ok(ImageTransform::Blur(
            BlurSpec::new(
                input.float(TransformField::Blur.key())?,
                match input.byte()? {
                    0 => BlurAlgorithm::Box,
                    1 => BlurAlgorithm::Gaussian,
                    _ => return Err(CodecError::Malformed),
                },
            )
            .map_err(transform_error)?,
        )),
        9 => Ok(ImageTransform::Sharpen(
            SharpenSpec::new(
                input.float(TransformField::Sharpen.key())?,
                input.float(TransformField::Sharpen.key())?,
                input.byte()?,
            )
            .map_err(transform_error)?,
        )),
        10 => Ok(ImageTransform::Mask(decode_mask(input)?)),
        11 => {
            let count = usize::try_from(input.u32_be()?).map_err(|_| CodecError::TooLong)?;
            if count > limits.max_composite_layers {
                return Err(CodecError::Request(RequestError::InvalidTransform(
                    TransformError::TooManyLayers,
                )));
            }
            if count == 0 {
                return Err(CodecError::Request(RequestError::InvalidTransform(
                    TransformError::EmptyLayers,
                )));
            }
            let count = proven_count(count, MIN_LAYER_BYTES, input.remaining())?;
            let mut layers = Vec::with_capacity(count);
            for _ in 0..count {
                layers.push(decode_layer(input)?);
            }
            Ok(ImageTransform::Composite(layers))
        }
        12 => {
            decode_encoded_revision(input)?;
            let dimensions =
                Dimensions::new(input.u32_be()?, input.u32_be()?).map_err(CodecError::Output)?;
            let filter = match input.byte()? {
                0 => EncodedFilter::Triangle,
                1 => EncodedFilter::Lanczos3,
                _ => return Err(CodecError::Malformed),
            };
            Ok(ImageTransform::EncodedResize { dimensions, filter })
        }
        13 => {
            decode_encoded_revision(input)?;
            Ok(ImageTransform::EncodedThumbnail(
                Dimensions::new(input.u32_be()?, input.u32_be()?).map_err(CodecError::Output)?,
            ))
        }
        14 => {
            decode_encoded_revision(input)?;
            let fill = decode_rgb(input)?;
            let blend = match input.byte()? {
                0 => EncodedAlphaBlend::IntegerFloor,
                1 => EncodedAlphaBlend::ImageSourceOver,
                _ => return Err(CodecError::Malformed),
            };
            Ok(ImageTransform::EncodedMatte { fill, blend })
        }
        15 => {
            decode_encoded_revision(input)?;
            Ok(ImageTransform::DiscardAlpha)
        }
        _ => Err(CodecError::Malformed),
    }
}

/// Compatibility algorithms are immutable independently of the outer intent schema.
fn decode_encoded_revision(input: &mut Reader<'_>) -> Result<(), CodecError> {
    if input.u32_be()? != ENCODED_STEP_REVISION {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

fn decode_resize_mode(tag: u8) -> Result<ResizeMode, CodecError> {
    match tag {
        0 => Ok(ResizeMode::Contain),
        1 => Ok(ResizeMode::Cover),
        2 => Ok(ResizeMode::Stretch),
        3 => Ok(ResizeMode::Pad),
        _ => Err(CodecError::Malformed),
    }
}

fn decode_interpolation(tag: u8) -> Result<Interpolation, CodecError> {
    match tag {
        0 => Ok(Interpolation::Nearest),
        1 => Ok(Interpolation::Bilinear),
        2 => Ok(Interpolation::Bicubic),
        3 => Ok(Interpolation::Lanczos3),
        _ => Err(CodecError::Malformed),
    }
}

fn decode_anchor(input: &mut Reader<'_>) -> Result<AlignAnchor, CodecError> {
    let axis = |tag: u8| match tag {
        0 => Ok(Anchor1D::Start),
        1 => Ok(Anchor1D::Center),
        2 => Ok(Anchor1D::End),
        _ => Err(CodecError::Malformed),
    };
    Ok(AlignAnchor {
        x: axis(input.byte()?)?,
        y: axis(input.byte()?)?,
    })
}

fn decode_rotate(input: &mut Reader<'_>) -> Result<RotateSpec, CodecError> {
    match input.byte()? {
        0 => {
            let turns = input.byte()?;
            let spec = RotateSpec::QuarterTurns(turns);
            spec.validate().map_err(transform_error)?;
            Ok(spec)
        }
        1 => {
            let degrees = input.float(TransformField::Rotation.key())?;
            let expand_canvas = input.boolean()?;
            let fill = decode_rgb(input)?;
            let interpolation = decode_interpolation(input.byte()?)?;
            let spec = RotateSpec::Arbitrary {
                degrees,
                expand_canvas,
                fill,
                interpolation,
            };
            spec.validate().map_err(transform_error)?;
            Ok(spec)
        }
        _ => Err(CodecError::Malformed),
    }
}

fn decode_color(input: &mut Reader<'_>) -> Result<ColorEffect, CodecError> {
    let effect = match input.byte()? {
        0 => ColorEffect::contrast(
            input.float(TransformField::Contrast.key())?,
            input.float(TransformField::Contrast.key())?,
        )
        .map_err(transform_error)?,
        1 => ColorEffect::saturation(input.float(TransformField::Saturation.key())?)
            .map_err(transform_error)?,
        2 => ColorEffect::hue_shift(input.float(TransformField::Hue.key())?)
            .map_err(transform_error)?,
        3 => ColorEffect::Grayscale,
        _ => return Err(CodecError::Malformed),
    };
    effect.validate().map_err(transform_error)?;
    Ok(effect)
}

fn decode_mask(input: &mut Reader<'_>) -> Result<MaskSpec, CodecError> {
    let source = match input.byte()? {
        0 => MaskSource::Asset(decode_source_ref(input)?),
        1 => MaskSource::Coverage(decode_coverage(input)?),
        _ => return Err(CodecError::Malformed),
    };
    let dimensions =
        Dimensions::new(input.u32_be()?, input.u32_be()?).map_err(CodecError::Output)?;
    let align = decode_anchor(input)?;
    let channel = match input.byte()? {
        0 => MaskChannel::Alpha,
        1 => MaskChannel::Luminance,
        _ => return Err(CodecError::Malformed),
    };
    let invert = input.boolean()?;
    let feather_radius_px = input.float(TransformField::Mask.key())?;
    let mode = match input.byte()? {
        0 => MaskMode::Multiply,
        1 => MaskMode::Replace,
        _ => return Err(CodecError::Malformed),
    };
    MaskSpec::new(
        source,
        dimensions,
        align,
        channel,
        invert,
        feather_radius_px,
        mode,
    )
    .map_err(transform_error)
}

fn decode_layer(input: &mut Reader<'_>) -> Result<CompositeLayer, CodecError> {
    let source = decode_source_ref(input)?;
    let layer = CompositeLayer::new(
        source,
        input.i32_be()?,
        input.i32_be()?,
        input.u16_be()?,
        match input.byte()? {
            0 => BlendMode::SourceOver,
            _ => return Err(CodecError::Malformed),
        },
    )
    .map_err(transform_error)?;
    Ok(layer)
}

/// Rebuilds the full config snapshot from canonical bytes. Version 1
/// snapshots decode with new fields taking built-in defaults, preserving
/// the old encoding semantics for stored snapshots. Rejects any other
/// version, malformed or trailing bytes and any bound the validated
/// constructors refuse. Fields are read in wire order; each later version
/// only appends.
pub(crate) fn decode_config(bytes: &[u8], max_bytes: usize) -> Result<ImageConfig, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::TooLong);
    }
    let mut input = Reader::new(bytes);
    let version = input.byte()?;
    if !(1..=CONFIG_CODEC_VERSION).contains(&version) {
        return Err(CodecError::InvalidVersion { actual: version });
    }
    let mode = decode_mode(input.byte()?)?;
    let default_backend = decode_backend(input.byte()?)?;
    let mut purposes = decode_purpose_gates(&mut input, version)?;
    let backends = decode_backend_gates(&mut input, version)?;
    let transforms = decode_transform_limits(&mut input)?;
    let scheduler = decode_scheduler(&mut input)?;
    let request_limits = decode_request_limits(&mut input)?;
    let mut profile = decode_profile_sections(&mut input, version)?;
    decode_codex_receipt_bounds(&mut input, version, &mut profile.codex)?;
    let analysis = decode_analysis_and_purpose_overrides(&mut input, version, &mut purposes)?;
    let quota = if version >= 11 {
        decode_quota(&mut input)?
    } else {
        LEGACY_QUOTA
    };
    let creator_bfl_account = if version >= 12 {
        decode_optional(&mut input, decode_account_handle)?
    } else {
        None
    };
    input.finish()?;
    let config = ImageConfig {
        mode,
        default_backend,
        purposes: purposes.try_into().map_err(|_| CodecError::Malformed)?,
        backends: backends.try_into().map_err(|_| CodecError::Malformed)?,
        transforms,
        scheduler,
        request_limits,
        codex: profile.codex,
        default_output_profile: profile.default_output_profile,
        output_profiles: profile.output_profiles,
        retry: profile.retry,
        accounting: profile.accounting,
        quality: profile.quality,
        analysis,
        quota,
        creator_bfl_account,
    };
    config.validate().map_err(CodecError::Config)?;
    Ok(config)
}

fn decode_mode(tag: u8) -> Result<GlobalMode, CodecError> {
    match tag {
        0 => Ok(GlobalMode::Live),
        1 => Ok(GlobalMode::LocalOnly),
        2 => Ok(GlobalMode::CacheOnly),
        3 => Ok(GlobalMode::Off),
        _ => Err(CodecError::Malformed),
    }
}

fn decode_purpose_gates(
    input: &mut Reader<'_>,
    version: u8,
) -> Result<Vec<PurposeGate>, CodecError> {
    let mut purposes = Vec::with_capacity(Purpose::COUNT);
    for _ in 0..Purpose::COUNT {
        purposes.push(decode_purpose_gate(input, version)?);
    }
    Ok(purposes)
}

fn decode_backend_gates(
    input: &mut Reader<'_>,
    version: u8,
) -> Result<Vec<BackendGate>, CodecError> {
    let mut backends = Vec::with_capacity(BackendId::COUNT);
    for _ in 0..BackendId::COUNT {
        backends.push(decode_backend_gate(input, version)?);
    }
    Ok(backends)
}

/// The config sections version 2 added after the request limits.
struct ProfileSections {
    codex: CodexSettings,
    default_output_profile: OutputProfileName,
    output_profiles: Vec<NamedOutputProfile>,
    retry: RetryPolicy,
    accounting: Accounting,
    quality: QualityDefaults,
}

/// Version 1 never wrote these sections, so they take the built-in defaults.
fn decode_profile_sections(
    input: &mut Reader<'_>,
    version: u8,
) -> Result<ProfileSections, CodecError> {
    if version == 1 {
        let defaults = ImageConfig::defaults().map_err(CodecError::Config)?;
        return Ok(ProfileSections {
            codex: defaults.codex,
            default_output_profile: defaults.default_output_profile,
            output_profiles: defaults.output_profiles,
            retry: defaults.retry,
            accounting: defaults.accounting,
            quality: defaults.quality,
        });
    }
    Ok(ProfileSections {
        codex: decode_codex(input)?,
        default_output_profile: decode_output_profile_name(input)?,
        output_profiles: decode_output_profiles(input)?,
        retry: decode_retry(input)?,
        accounting: decode_accounting(input)?,
        quality: decode_quality(input, version)?,
    })
}

/// Version 4 appended the stdout and stderr receipt bounds, version 5 the message bound.
fn decode_codex_receipt_bounds(
    input: &mut Reader<'_>,
    version: u8,
    codex: &mut CodexSettings,
) -> Result<(), CodecError> {
    if version >= 4 {
        codex.max_stdout_bytes = input.u64_be()?;
        codex.max_stderr_bytes = input.u64_be()?;
    }
    if version >= 5 {
        codex.max_message_bytes = input.u64_be()?;
    }
    Ok(())
}

/// Versions 6 to 10 interleave analysis settings with per-purpose overrides,
/// so both are read here in wire order.
fn decode_analysis_and_purpose_overrides(
    input: &mut Reader<'_>,
    version: u8,
    purposes: &mut [PurposeGate],
) -> Result<AnalysisSettings, CodecError> {
    let mut analysis = legacy_analysis_settings();
    if version >= 6 {
        analysis.classification = decode_optional(input, decode_route)?;
        analysis.inspection = decode_optional(input, decode_route)?;
    }
    if version >= 7 {
        decode_quality_and_model_overrides(input, purposes)?;
    }
    if version >= 8 {
        decode_moderation_overrides(input, purposes)?;
    }
    if version >= 9 {
        analysis.preparation = decode_preparation_limits(input)?;
    }
    if version >= 10 {
        decode_staging_and_segmentation(input, &mut analysis)?;
        decode_account_overrides(input, purposes)?;
    }
    Ok(analysis)
}

fn decode_quota(input: &mut Reader<'_>) -> Result<QuotaSettings, CodecError> {
    Ok(QuotaSettings {
        enabled: input.boolean()?,
        cooldown_ms: input.u64_be()?,
        maximum_accounts: input.u32_be()?,
    })
}

fn decode_transform_limits(input: &mut Reader<'_>) -> Result<TransformLimits, CodecError> {
    Ok(TransformLimits {
        enabled: input.boolean()?,
        max_concurrency: input.u32_be()?,
        max_decoded_bytes: input.u64_be()?,
        max_output_bytes: input.u64_be()?,
        queue_capacity: input.u32_be()?,
        deadline_secs: input.u64_be()?,
    })
}

fn decode_scheduler(input: &mut Reader<'_>) -> Result<SchedulerBounds, CodecError> {
    Ok(SchedulerBounds {
        foreground_limit: input.u32_be()?,
        presence_limit: input.u32_be()?,
        background_limit: input.u32_be()?,
        max_pending_jobs: input.u32_be()?,
        max_pending_bytes: input.u64_be()?,
        queue_age_secs: input.u64_be()?,
        job_deadline_secs: input.u64_be()?,
        stage_deadline_secs: input.u64_be()?,
    })
}

fn decode_request_limits(input: &mut Reader<'_>) -> Result<RequestLimits, CodecError> {
    Ok(RequestLimits {
        max_prompt_chars: usize_decode(input.u64_be()?)?,
        max_references: usize_decode(input.u64_be()?)?,
        max_transform_steps: usize_decode(input.u64_be()?)?,
        max_composite_layers: usize_decode(input.u64_be()?)?,
    })
}

/// Analysis settings of a snapshot before any analysis field was encoded.
fn legacy_analysis_settings() -> AnalysisSettings {
    AnalysisSettings {
        head_locations: None,
        staging_identity: None,
        staging_facing: None,
        segmentation: None,
        segmentation_preparation: LEGACY_SEGMENTATION_PREPARATION,
        classification: None,
        inspection: None,
        preparation: LEGACY_ANALYSIS_PREPARATION,
    }
}

fn decode_preparation_limits(
    input: &mut Reader<'_>,
) -> Result<AnalysisPreparationLimits, CodecError> {
    Ok(AnalysisPreparationLimits {
        max_pixels: input.u64_be()?,
        max_picture_bytes: input.u64_be()?,
        max_request_bytes: input.u64_be()?,
    })
}

fn decode_quality_and_model_overrides(
    input: &mut Reader<'_>,
    purposes: &mut [PurposeGate],
) -> Result<(), CodecError> {
    for purpose in purposes {
        purpose.quality_override = decode_optional(input, decode_named_generation_quality)?;
        purpose.model_override = decode_optional(input, decode_model_name)?;
    }
    Ok(())
}

fn decode_moderation_overrides(
    input: &mut Reader<'_>,
    purposes: &mut [PurposeGate],
) -> Result<(), CodecError> {
    for purpose in purposes {
        purpose.moderation_override = decode_optional(input, decode_moderation)?;
    }
    Ok(())
}

fn decode_account_overrides(
    input: &mut Reader<'_>,
    purposes: &mut [PurposeGate],
) -> Result<(), CodecError> {
    for purpose in purposes {
        purpose.account_override = decode_optional(input, decode_account_handle)?;
    }
    Ok(())
}

/// Version 10 routes: head locations, staging identity and facing, then segmentation with
/// its preparation bounds.
fn decode_staging_and_segmentation(
    input: &mut Reader<'_>,
    analysis: &mut AnalysisSettings,
) -> Result<(), CodecError> {
    analysis.head_locations = decode_optional(input, decode_route)?;
    analysis.staging_identity = decode_optional(input, decode_route)?;
    analysis.staging_facing = decode_optional(input, decode_route)?;
    analysis.segmentation = decode_optional(input, decode_segmentation_route)?;
    analysis.segmentation_preparation = decode_preparation_limits(input)?;
    Ok(())
}

fn decode_segmentation_route(input: &mut Reader<'_>) -> Result<SegmentationRoute, CodecError> {
    let provider = match input.byte()? {
        0 => SegmentationProvider::Fal,
        _ => return Err(CodecError::Malformed),
    };
    Ok(SegmentationRoute {
        provider,
        account: decode_account_handle(input)?,
        model: decode_model_name(input)?,
        endpoint: decode_api_endpoint(input)?,
    })
}

fn decode_backend(tag: u8) -> Result<BackendId, CodecError> {
    match tag {
        0 => Ok(BackendId::OpenAi),
        1 => Ok(BackendId::Fal),
        2 => Ok(BackendId::Bfl),
        3 => Ok(BackendId::CodexCli),
        _ => Err(CodecError::Malformed),
    }
}

fn decode_purpose_gate(input: &mut Reader<'_>, version: u8) -> Result<PurposeGate, CodecError> {
    let enabled = input.boolean()?;
    let backend_override = match input.byte()? {
        0 => None,
        1 => Some(decode_backend(input.byte()?)?),
        _ => return Err(CodecError::Malformed),
    };
    let output_profile = if version == 1 {
        None
    } else {
        decode_optional(input, decode_output_profile_name)?
    };
    Ok(PurposeGate {
        account_override: None,
        enabled,
        backend_override,
        output_profile,
        quality_override: None,
        model_override: None,
        moderation_override: None,
    })
}

fn decode_backend_gate(input: &mut Reader<'_>, version: u8) -> Result<BackendGate, CodecError> {
    let enabled = input.boolean()?;
    let account = decode_account_handle(input)?;
    let max_concurrent_jobs = input.u32_be()?;
    let (model, endpoint) = if version == 1 {
        (None, None)
    } else {
        (
            decode_optional(input, decode_model_name)?,
            decode_optional(input, decode_api_endpoint)?,
        )
    };
    let (generation_model, edit_model, alpha_model) = if version >= 3 {
        (
            decode_optional(input, decode_model_name)?,
            decode_optional(input, decode_model_name)?,
            decode_optional(input, decode_model_name)?,
        )
    } else {
        (None, None, None)
    };
    Ok(BackendGate {
        enabled,
        account,
        max_concurrent_jobs,
        generation_model,
        edit_model,
        alpha_model,
        model,
        endpoint,
    })
}

fn decode_optional<T>(
    input: &mut Reader<'_>,
    decode: impl FnOnce(&mut Reader<'_>) -> Result<T, CodecError>,
) -> Result<Option<T>, CodecError> {
    match input.byte()? {
        0 => Ok(None),
        1 => decode(input).map(Some),
        _ => Err(CodecError::Malformed),
    }
}

fn decode_output_profile_name(input: &mut Reader<'_>) -> Result<OutputProfileName, CodecError> {
    OutputProfileName::new(input.text(MAXIMUM_PROFILE_NAME_BYTES)?).map_err(CodecError::Config)
}

fn decode_codex(input: &mut Reader<'_>) -> Result<CodexSettings, CodecError> {
    let executable =
        CodexExecutable::new(input.text(MAXIMUM_CODEX_PATH_BYTES)?).map_err(CodecError::Config)?;
    let workdir =
        JobWorkDir::new(input.text(MAXIMUM_CODEX_PATH_BYTES)?).map_err(CodecError::Config)?;
    let cli_profile = CliProfileRevision::new(input.text(MAXIMUM_CLI_PROFILE_BYTES)?)
        .map_err(CodecError::Config)?;
    let agent_model = decode_optional(input, decode_model_name)?;
    Ok(CodexSettings {
        executable,
        workdir,
        cli_profile,
        agent_model,
        max_stdout_bytes: LEGACY_CODEX_STDOUT_BYTES,
        max_stderr_bytes: LEGACY_CODEX_STDERR_BYTES,
        max_message_bytes: LEGACY_CODEX_MESSAGE_BYTES,
    })
}

fn decode_output_profiles(input: &mut Reader<'_>) -> Result<Vec<NamedOutputProfile>, CodecError> {
    let count = usize::try_from(input.u32_be()?).map_err(|_| CodecError::TooLong)?;
    let count = proven_count(count, MIN_PROFILE_BYTES, input.remaining())?;
    let mut profiles = Vec::with_capacity(count);
    for _ in 0..count {
        let name = decode_output_profile_name(input)?;
        let profile = decode_output(input)?;
        profiles.push(NamedOutputProfile::new(name, profile));
    }
    // Canonical order is by name (see the encoder): foreign or historical
    // bytes decode into the same normalized table.
    profiles.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
    Ok(profiles)
}

fn decode_retry(input: &mut Reader<'_>) -> Result<RetryPolicy, CodecError> {
    Ok(RetryPolicy {
        max_attempts: input.u32_be()?,
        retry_deadline_secs: input.u64_be()?,
        backoff_secs: input.u64_be()?,
    })
}

fn decode_accounting(input: &mut Reader<'_>) -> Result<Accounting, CodecError> {
    let mode = match input.byte()? {
        0 => AccountingMode::RecordOnly,
        1 => AccountingMode::Hosted,
        _ => return Err(CodecError::Malformed),
    };
    let price_file = decode_optional(input, |input| {
        PriceFileRef::new(input.text(MAXIMUM_PRICE_FILE_BYTES)?).map_err(CodecError::Config)
    })?;
    Ok(Accounting { mode, price_file })
}

fn decode_named_generation_quality(
    input: &mut Reader<'_>,
) -> Result<GenerationQuality, CodecError> {
    match input.byte()? {
        0 => Ok(GenerationQuality::Low),
        1 => Ok(GenerationQuality::Medium),
        2 => Ok(GenerationQuality::High),
        3 => Ok(GenerationQuality::Auto),
        4 => Ok(GenerationQuality::LegacyNumeric(
            LossyQuality::new(input.byte()?).map_err(CodecError::Output)?,
        )),
        _ => Err(CodecError::Malformed),
    }
}

fn decode_quality(input: &mut Reader<'_>, version: u8) -> Result<QualityDefaults, CodecError> {
    let jpeg_quality = LossyQuality::new(input.byte()?).map_err(CodecError::Output)?;
    let provider_quality = match input.byte()? {
        0 => None,
        1 if version == 2 => Some(GenerationQuality::LegacyNumeric(
            LossyQuality::new(input.byte()?).map_err(CodecError::Output)?,
        )),
        1 => Some(decode_named_generation_quality(input)?),
        _ => return Err(CodecError::Malformed),
    };
    Ok(QualityDefaults {
        jpeg_quality,
        provider_quality,
    })
}

fn usize_decode(value: u64) -> Result<usize, CodecError> {
    usize::try_from(value).map_err(|_| CodecError::TooLong)
}
