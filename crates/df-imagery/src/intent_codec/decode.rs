//! Decoder half of the intent codec: bounded reconstruction of one consumer
//! intent and one config snapshot through the validated constructors, so a
//! restart replays checked work. Every vector count proves its minimum wire
//! width against the remaining input before anything is reserved.

use df_assets::{AssetId, Audience, MemberSet};
use df_types::{AccountId, CampaignId, MemberId, OperationId, RunId};

use crate::config::{
    AccountHandle, BackendGate, BackendId, GlobalMode, ImageConfig, PurposeGate, SchedulerBounds,
    TransformLimits,
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
    ColorEffect, CompositeLayer, CoverageRef, ExposureStops, FlipAxis, ImageTransform,
    Interpolation, MaskChannel, MaskMode, MaskSource, MaskSpec, PadSpec, PixelRect, ResizeMode,
    ResizeSpec, RotateSpec, SharpenSpec, TransformError, TransformSource, TransformSpec,
};

use super::primitives::{
    MIN_LAYER_BYTES, MIN_MEMBER_BYTES, MIN_SOURCE_REF_BYTES, MIN_STEP_BYTES, Reader,
    prompt_byte_bound,
};
use super::{CONFIG_CODEC_VERSION, CodecError, DecodedIntent, INTENT_CODEC_VERSION};

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
    if input.byte()? != INTENT_CODEC_VERSION {
        return Err(CodecError::BadVersion {
            actual: bytes[0],
        });
    }
    let purpose = Purpose::from_index(usize::from(input.byte()?)).ok_or(CodecError::Malformed)?;
    let role = decode_role(input.byte()?)?;
    let operation_kind = input.byte()?;
    let owner = decode_owner(&mut input)?;
    let operation = decode_operation(&mut input, operation_kind, limits)?;
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

fn decode_role(tag: u8) -> Result<VariantRole, CodecError> {
    match tag {
        0 => Ok(VariantRole::Primary),
        1 => Ok(VariantRole::Thumbnail),
        2 => Ok(VariantRole::Mask),
        3 => Ok(VariantRole::Reference),
        _ => Err(CodecError::Malformed),
    }
}

fn decode_owner(input: &mut Reader<'_>) -> Result<ImageOwner, CodecError> {
    match input.byte()? {
        0 => {
            let campaign =
                CampaignId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
            let run =
                RunId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
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

fn decode_audience(input: &mut Reader<'_>) -> Result<Audience, CodecError> {
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
        let member =
            MemberId::from_bytes(input.take(16)?).map_err(|_| CodecError::InvalidId)?;
        members.push(member);
    }
    MemberSet::new(members).ok_or(CodecError::Malformed)
}

fn decode_source_ref(input: &mut Reader<'_>) -> Result<SourceRef, CodecError> {
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

fn decode_execution(tag: u8) -> Result<ExecutionMode, CodecError> {
    match tag {
        0 => Ok(ExecutionMode::Live),
        1 => Ok(ExecutionMode::PreparedOnly),
        2 => Ok(ExecutionMode::Replay),
        _ => Err(CodecError::Malformed),
    }
}

fn decode_freshness(input: &mut Reader<'_>) -> Result<Freshness, CodecError> {
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

fn decode_prompt(
    input: &mut Reader<'_>,
    limits: &RequestLimits,
) -> Result<TextPrompt, CodecError> {
    let template_version = input.u32_be()?;
    let text = input.counted(prompt_byte_bound(limits))?;
    let text = core::str::from_utf8(text).map_err(|_| CodecError::InvalidText)?;
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
            let generated = Dimensions::new(input.u32_be()?, input.u32_be()?)
                .map_err(CodecError::Output)?;
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
        0 => Ok(ImageTransform::Resize(ResizeSpec::new(
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
        .map_err(transform_error)?)),
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
            BrightnessOffset::new(input.float("brightness")?).map_err(transform_error)?,
        )),
        6 => Ok(ImageTransform::Expose(
            ExposureStops::new(input.float("exposure")?).map_err(transform_error)?,
        )),
        7 => Ok(ImageTransform::Color(decode_color(input)?)),
        8 => Ok(ImageTransform::Blur(
            BlurSpec::new(input.float("blur")?, match input.byte()? {
                0 => BlurAlgorithm::Box,
                1 => BlurAlgorithm::Gaussian,
                _ => return Err(CodecError::Malformed),
            })
            .map_err(transform_error)?,
        )),
        9 => Ok(ImageTransform::Sharpen(
            SharpenSpec::new(
                input.float("sharpen")?,
                input.float("sharpen")?,
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
        _ => Err(CodecError::Malformed),
    }
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
            let degrees = input.float("rotation")?;
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
        0 => ColorEffect::contrast(input.float("contrast")?, input.float("contrast")?)
            .map_err(transform_error)?,
        1 => ColorEffect::saturation(input.float("saturation")?).map_err(transform_error)?,
        2 => ColorEffect::hue_shift(input.float("hue")?).map_err(transform_error)?,
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
    let feather_radius_px = input.float("mask")?;
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

/// Rebuilds the full config snapshot from canonical bytes. Rejects a version
/// mismatch, malformed or trailing bytes and any bound the validated
/// constructors refuse.
pub(crate) fn decode_config(
    bytes: &[u8],
    max_bytes: usize,
) -> Result<ImageConfig, CodecError> {
    if bytes.len() > max_bytes {
        return Err(CodecError::TooLong);
    }
    let mut input = Reader::new(bytes);
    if input.byte()? != CONFIG_CODEC_VERSION {
        return Err(CodecError::BadVersion {
            actual: bytes[0],
        });
    }
    let mode = match input.byte()? {
        0 => GlobalMode::Live,
        1 => GlobalMode::LocalOnly,
        2 => GlobalMode::CacheOnly,
        3 => GlobalMode::Off,
        _ => return Err(CodecError::Malformed),
    };
    let default_backend = decode_backend(input.byte()?)?;
    let mut purposes = Vec::with_capacity(Purpose::COUNT);
    for _ in 0..Purpose::COUNT {
        purposes.push(decode_purpose_gate(&mut input)?);
    }
    let mut backends = Vec::with_capacity(BackendId::COUNT);
    for _ in 0..BackendId::COUNT {
        backends.push(decode_backend_gate(&mut input)?);
    }
    let transforms = TransformLimits {
        enabled: input.boolean()?,
        max_concurrency: input.u32_be()?,
        max_decoded_bytes: input.u64_be()?,
        max_output_bytes: input.u64_be()?,
        queue_capacity: input.u32_be()?,
        deadline_secs: input.u64_be()?,
    };
    let scheduler = SchedulerBounds {
        foreground_limit: input.u32_be()?,
        presence_limit: input.u32_be()?,
        background_limit: input.u32_be()?,
        max_pending_jobs: input.u32_be()?,
        max_pending_bytes: input.u64_be()?,
        queue_age_secs: input.u64_be()?,
        job_deadline_secs: input.u64_be()?,
        stage_deadline_secs: input.u64_be()?,
    };
    let request_limits = RequestLimits {
        max_prompt_chars: usize_decode(input.u64_be()?)?,
        max_references: usize_decode(input.u64_be()?)?,
        max_transform_steps: usize_decode(input.u64_be()?)?,
        max_composite_layers: usize_decode(input.u64_be()?)?,
    };
    input.finish()?;
    let config = ImageConfig {
        mode,
        default_backend,
        purposes: purposes
            .try_into()
            .map_err(|_| CodecError::Malformed)?,
        backends: backends
            .try_into()
            .map_err(|_| CodecError::Malformed)?,
        transforms,
        scheduler,
        request_limits,
    };
    config.validate().map_err(CodecError::Config)?;
    Ok(config)
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

fn decode_purpose_gate(input: &mut Reader<'_>) -> Result<PurposeGate, CodecError> {
    Ok(PurposeGate {
        enabled: input.boolean()?,
        backend_override: match input.byte()? {
            0 => None,
            1 => Some(decode_backend(input.byte()?)?),
            _ => return Err(CodecError::Malformed),
        },
    })
}

fn decode_backend_gate(input: &mut Reader<'_>) -> Result<BackendGate, CodecError> {
    let enabled = input.boolean()?;
    let account_bytes = input.counted(64)?;
    let account_text =
        core::str::from_utf8(account_bytes).map_err(|_| CodecError::InvalidText)?;
    let account = AccountHandle::new(account_text).map_err(CodecError::Config)?;
    Ok(BackendGate {
        enabled,
        account,
        max_concurrent_jobs: input.u32_be()?,
    })
}

fn usize_decode(value: u64) -> Result<usize, CodecError> {
    usize::try_from(value).map_err(|_| CodecError::TooLong)
}
