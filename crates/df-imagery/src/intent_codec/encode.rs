//! Encoder half of the intent codec: canonical bytes plus SHA-256 for one
//! consumer intent and for one config snapshot. Excluded fields stay out here:
//! the operation id, caller fingerprint and urgency never reach these bytes.

use df_assets::{Audience, MemberSet};
use sha2::{Digest, Sha256};

use crate::config::{
    BackendGate, BackendId, GlobalMode, ImageConfig, PurposeGate, SchedulerBounds, TransformLimits,
};
use crate::output::{AlphaHandling, Encoding, OutputProfile, RasterOutputSpec, Rgb};
use crate::request::{
    ExecutionMode, Freshness, ImageOperation, ImageOwner, ImageRequest, RequestError,
    RequestLimits, SourceRef, TextPrompt, VariantRole,
};
use crate::transform::{
    AlignAnchor, Anchor1D, BlendMode, BlurAlgorithm, ColorEffect, CompositeLayer, CoverageRef,
    FlipAxis, ImageTransform, Interpolation, MaskChannel, MaskMode, MaskSource, MaskSpec,
    ResizeMode, RotateSpec, TransformError, TransformSource, TransformSpec,
};

use super::primitives::{MIN_MEMBER_BYTES, Writer, prompt_byte_bound};
use super::{CONFIG_CODEC_VERSION, CodecError, EncodedConfig, EncodedIntent, INTENT_CODEC_VERSION};

/// Encodes the consumer intent of `request` under `limits` without exceeding
/// `max_bytes`. The operation id, caller fingerprint and urgency are excluded
/// on purpose: equal intent encodes equally whatever call carried it.
pub(crate) fn encode_intent(
    request: &ImageRequest,
    limits: &RequestLimits,
    max_bytes: usize,
) -> Result<EncodedIntent, CodecError> {
    request.validate(limits).map_err(CodecError::Request)?;
    let mut out = Writer::new(max_bytes);
    out.push(INTENT_CODEC_VERSION)?;
    out.push(u8::try_from(request.purpose.purpose.index()).map_err(|_| CodecError::TooLong)?)?;
    out.push(role_tag(request.purpose.role))?;
    out.push(operation_tag(&request.operation))?;
    encode_owner(&mut out, &request.identity.owner)?;
    encode_operation(&mut out, &request.operation, limits)?;
    encode_execution(&mut out, request.execution)?;
    encode_freshness(&mut out, &request.freshness)?;
    encode_audience(&mut out, &request.audience)?;
    out.u64_be(request.expected_source_revision)?;
    encode_output(&mut out, &request.output)?;
    let bytes = out.finish();
    let fingerprint = Sha256::digest(&bytes).into();
    Ok(EncodedIntent {
        version: INTENT_CODEC_VERSION,
        bytes,
        fingerprint,
    })
}

fn role_tag(role: VariantRole) -> u8 {
    match role {
        VariantRole::Primary => 0,
        VariantRole::Thumbnail => 1,
        VariantRole::Mask => 2,
        VariantRole::Reference => 3,
    }
}

fn operation_tag(operation: &ImageOperation) -> u8 {
    match operation {
        ImageOperation::Generate(_) => 0,
        ImageOperation::Edit(_) => 1,
        ImageOperation::Transform(_) => 2,
    }
}

fn encode_owner(out: &mut Writer, owner: &ImageOwner) -> Result<(), CodecError> {
    match owner {
        ImageOwner::Run { campaign, run } => {
            out.push(0)?;
            out.extend(campaign.as_bytes())?;
            out.extend(run.as_bytes())?;
        }
        ImageOwner::Draft { account } => {
            out.push(1)?;
            out.extend(account.as_bytes())?;
        }
    }
    Ok(())
}

fn encode_audience(out: &mut Writer, audience: &Audience) -> Result<(), CodecError> {
    match audience {
        Audience::PublicStatic => out.push(1),
        Audience::CampaignShared(campaign) => {
            out.push(2)?;
            out.extend(campaign.as_bytes())
        }
        Audience::Members(campaign, members) => {
            out.push(3)?;
            out.extend(campaign.as_bytes())?;
            encode_members(out, members)
        }
        Audience::Host(campaign) => {
            out.push(4)?;
            out.extend(campaign.as_bytes())
        }
        Audience::OwnerPrivate(account) => {
            out.push(5)?;
            out.extend(account.as_bytes())
        }
        Audience::Withheld => Err(CodecError::Request(RequestError::WithheldAudience)),
    }
}

fn encode_members(out: &mut Writer, members: &MemberSet) -> Result<(), CodecError> {
    // MemberSet::new sorts and dedups at construction, so members() already
    // iterates the canonical order: no clone or sort here.
    let count = u32::try_from(members.members().len()).map_err(|_| CodecError::TooLong)?;
    let need = usize::try_from(count)
        .map_err(|_| CodecError::TooLong)?
        .checked_mul(MIN_MEMBER_BYTES)
        .and_then(|width| width.checked_add(4))
        .ok_or(CodecError::TooLong)?;
    if need > out.remaining() {
        return Err(CodecError::TooLong);
    }
    out.u32_be(count)?;
    for member in members.members() {
        out.extend(member.as_bytes())?;
    }
    Ok(())
}

fn encode_source_ref(out: &mut Writer, source: &SourceRef) -> Result<(), CodecError> {
    out.extend(source.asset.as_bytes())?;
    out.extend(source.digest.as_bytes())?;
    encode_audience(out, &source.audience)
}

fn encode_execution(out: &mut Writer, execution: ExecutionMode) -> Result<(), CodecError> {
    let tag = match execution {
        ExecutionMode::Live => 0,
        ExecutionMode::PreparedOnly => 1,
        ExecutionMode::Replay => 2,
    };
    out.push(tag)
}

fn encode_freshness(out: &mut Writer, freshness: &Freshness) -> Result<(), CodecError> {
    match freshness {
        Freshness::ReuseCompatible => out.push(0),
        Freshness::Regenerate { nonce } => {
            out.push(1)?;
            out.extend(nonce.as_bytes())
        }
    }
}

fn encode_prompt(
    out: &mut Writer,
    prompt: &TextPrompt,
    limits: &RequestLimits,
) -> Result<(), CodecError> {
    let len_chars = prompt.len_chars();
    if len_chars > limits.max_prompt_chars {
        return Err(CodecError::Request(RequestError::PromptTooLong {
            len_chars,
            max_chars: limits.max_prompt_chars,
        }));
    }
    let text = prompt.text().as_bytes();
    if text.len() > prompt_byte_bound(limits) {
        return Err(CodecError::TooLong);
    }
    // Template version first: decode_prompt reads it before the counted text.
    out.u32_be(prompt.template_version)?;
    out.counted(text)
}

fn encode_references(
    out: &mut Writer,
    references: &[SourceRef],
    limits: &RequestLimits,
) -> Result<(), CodecError> {
    if references.len() > limits.max_references {
        return Err(CodecError::Request(RequestError::TooManyReferences {
            count: references.len(),
        }));
    }
    let count = u32::try_from(references.len()).map_err(|_| CodecError::TooLong)?;
    out.u32_be(count)?;
    for reference in references {
        encode_source_ref(out, reference)?;
    }
    Ok(())
}

fn encode_operation(
    out: &mut Writer,
    operation: &ImageOperation,
    limits: &RequestLimits,
) -> Result<(), CodecError> {
    match operation {
        ImageOperation::Generate(spec) => {
            encode_prompt(out, &spec.prompt, limits)?;
            encode_references(out, &spec.references, limits)?;
        }
        ImageOperation::Edit(spec) => {
            encode_prompt(out, &spec.prompt, limits)?;
            encode_source_ref(out, &spec.base)?;
            encode_references(out, &spec.references, limits)?;
        }
        ImageOperation::Transform(spec) => {
            encode_transform(out, spec, limits)?;
        }
    }
    Ok(())
}

fn encode_output(out: &mut Writer, output: &OutputProfile) -> Result<(), CodecError> {
    match output {
        OutputProfile::Provider { generated, delivered } => {
            out.push(0)?;
            out.u32_be(generated.width_px())?;
            out.u32_be(generated.height_px())?;
            encode_raster(out, *delivered)?;
        }
        OutputProfile::Local { delivered } => {
            out.push(1)?;
            encode_raster(out, *delivered)?;
        }
    }
    Ok(())
}

fn encode_raster(out: &mut Writer, raster: RasterOutputSpec) -> Result<(), CodecError> {
    let dimensions = raster.dimensions();
    out.u32_be(dimensions.width_px())?;
    out.u32_be(dimensions.height_px())?;
    match raster.alpha() {
        AlphaHandling::Opaque => out.push(0)?,
        AlphaHandling::PreserveAlpha => out.push(1)?,
        AlphaHandling::Matte(rgb) => {
            out.push(2)?;
            encode_rgb(out, rgb)?;
        }
    }
    match raster.encoding() {
        Encoding::Jpeg { quality } => {
            out.push(0)?;
            out.push(quality.value())?;
        }
        Encoding::WebP { quality } => {
            out.push(1)?;
            match quality {
                Some(quality) => {
                    out.push(1)?;
                    out.push(quality.value())?;
                }
                None => out.push(0)?,
            }
        }
        Encoding::Png => out.push(2)?,
    }
    Ok(())
}

fn encode_rgb(out: &mut Writer, rgb: Rgb) -> Result<(), CodecError> {
    out.push(rgb.red)?;
    out.push(rgb.green)?;
    out.push(rgb.blue)
}

fn encode_transform(
    out: &mut Writer,
    spec: &TransformSpec,
    limits: &RequestLimits,
) -> Result<(), CodecError> {
    if spec.steps.len() > limits.max_transform_steps {
        return Err(CodecError::Request(RequestError::InvalidTransform(
            TransformError::TooManySteps,
        )));
    }
    encode_transform_source(out, &spec.source)?;
    let count = u32::try_from(spec.steps.len()).map_err(|_| CodecError::TooLong)?;
    out.u32_be(count)?;
    for step in &spec.steps {
        encode_step(out, step, limits)?;
    }
    Ok(())
}

fn encode_transform_source(out: &mut Writer, source: &TransformSource) -> Result<(), CodecError> {
    match source {
        TransformSource::Image(reference) => {
            out.push(0)?;
            encode_source_ref(out, reference)?;
        }
        TransformSource::Canvas(canvas) => {
            out.push(1)?;
            out.u32_be(canvas.width_px())?;
            out.u32_be(canvas.height_px())?;
            encode_rgb(out, canvas.fill())?;
        }
        TransformSource::Coverage(coverage) => {
            out.push(2)?;
            encode_coverage(out, coverage)?;
        }
    }
    Ok(())
}

fn encode_coverage(out: &mut Writer, coverage: &CoverageRef) -> Result<(), CodecError> {
    encode_source_ref(out, &coverage.coverage)?;
    encode_source_ref(out, &coverage.analyzed)?;
    out.u32_be(coverage.analysis_revision)
}

fn encode_step(
    out: &mut Writer,
    step: &ImageTransform,
    limits: &RequestLimits,
) -> Result<(), CodecError> {
    match step {
        ImageTransform::Resize(spec) => {
            out.push(0)?;
            out.u32_be(spec.width_px())?;
            out.u32_be(spec.height_px())?;
            out.push(match spec.mode() {
                ResizeMode::Contain => 0,
                ResizeMode::Cover => 1,
                ResizeMode::Stretch => 2,
                ResizeMode::Pad => 3,
            })?;
            encode_anchor(out, spec.anchor())?;
            out.push(match spec.interpolation() {
                Interpolation::Nearest => 0,
                Interpolation::Bilinear => 1,
                Interpolation::Bicubic => 2,
                Interpolation::Lanczos3 => 3,
            })?;
            out.boolean(spec.allow_upscale())?;
            match spec.pad_fill() {
                Some(fill) => {
                    out.push(1)?;
                    encode_rgb(out, fill)?;
                }
                None => out.push(0)?,
            }
        }
        ImageTransform::Crop(rect) => {
            out.push(1)?;
            out.u32_be(rect.x_px())?;
            out.u32_be(rect.y_px())?;
            out.u32_be(rect.width_px())?;
            out.u32_be(rect.height_px())?;
        }
        ImageTransform::Pad(spec) => {
            out.push(2)?;
            out.u32_be(spec.width_px())?;
            out.u32_be(spec.height_px())?;
            encode_anchor(out, spec.anchor())?;
            encode_rgb(out, spec.fill())?;
        }
        ImageTransform::Rotate(spec) => {
            out.push(3)?;
            match spec {
                RotateSpec::QuarterTurns(turns) => {
                    out.push(0)?;
                    out.push(*turns)?;
                }
                RotateSpec::Arbitrary {
                    degrees,
                    expand_canvas,
                    fill,
                    interpolation,
                } => {
                    out.push(1)?;
                    out.float(*degrees, "rotation")?;
                    out.boolean(*expand_canvas)?;
                    encode_rgb(out, *fill)?;
                    out.push(match interpolation {
                        Interpolation::Nearest => 0,
                        Interpolation::Bilinear => 1,
                        Interpolation::Bicubic => 2,
                        Interpolation::Lanczos3 => 3,
                    })?;
                }
            }
        }
        ImageTransform::Flip(axis) => {
            out.push(4)?;
            out.push(match axis {
                FlipAxis::Horizontal => 0,
                FlipAxis::Vertical => 1,
            })?;
        }
        ImageTransform::Brighten(offset) => {
            out.push(5)?;
            out.float(offset.value(), "brightness")?;
        }
        ImageTransform::Expose(stops) => {
            out.push(6)?;
            out.float(stops.value(), "exposure")?;
        }
        ImageTransform::Color(effect) => {
            out.push(7)?;
            match effect {
                ColorEffect::Contrast { gain, pivot } => {
                    out.push(0)?;
                    out.float(*gain, "contrast")?;
                    out.float(*pivot, "contrast")?;
                }
                ColorEffect::Saturation { factor } => {
                    out.push(1)?;
                    out.float(*factor, "saturation")?;
                }
                ColorEffect::HueShift { degrees } => {
                    out.push(2)?;
                    out.float(*degrees, "hue")?;
                }
                ColorEffect::Grayscale => out.push(3)?,
            }
        }
        ImageTransform::Blur(spec) => {
            out.push(8)?;
            out.float(spec.radius_px(), "blur")?;
            out.push(match spec.algorithm() {
                BlurAlgorithm::Box => 0,
                BlurAlgorithm::Gaussian => 1,
            })?;
        }
        ImageTransform::Sharpen(spec) => {
            out.push(9)?;
            out.float(spec.radius_px(), "sharpen")?;
            out.float(spec.amount_factor(), "sharpen")?;
            out.push(spec.threshold_0_255())?;
        }
        ImageTransform::Mask(spec) => {
            out.push(10)?;
            encode_mask(out, spec)?;
        }
        ImageTransform::Composite(layers) => {
            out.push(11)?;
            if layers.len() > limits.max_composite_layers {
                return Err(CodecError::Request(RequestError::InvalidTransform(
                    TransformError::TooManyLayers,
                )));
            }
            let count = u32::try_from(layers.len()).map_err(|_| CodecError::TooLong)?;
            out.u32_be(count)?;
            for layer in layers {
                encode_layer(out, layer)?;
            }
        }
    }
    Ok(())
}

fn encode_anchor(out: &mut Writer, anchor: AlignAnchor) -> Result<(), CodecError> {
    out.push(match anchor.x {
        Anchor1D::Start => 0,
        Anchor1D::Center => 1,
        Anchor1D::End => 2,
    })?;
    out.push(match anchor.y {
        Anchor1D::Start => 0,
        Anchor1D::Center => 1,
        Anchor1D::End => 2,
    })
}

fn encode_mask(out: &mut Writer, spec: &MaskSpec) -> Result<(), CodecError> {
    match spec.source() {
        MaskSource::Asset(reference) => {
            out.push(0)?;
            encode_source_ref(out, reference)?;
        }
        MaskSource::Coverage(coverage) => {
            out.push(1)?;
            encode_coverage(out, coverage)?;
        }
    }
    out.u32_be(spec.width_px())?;
    out.u32_be(spec.height_px())?;
    encode_anchor(out, spec.align())?;
    out.push(match spec.channel() {
        MaskChannel::Alpha => 0,
        MaskChannel::Luminance => 1,
    })?;
    out.boolean(spec.invert())?;
    out.float(spec.feather_radius_px(), "mask")?;
    out.push(match spec.mode() {
        MaskMode::Multiply => 0,
        MaskMode::Replace => 1,
    })
}

fn encode_layer(out: &mut Writer, layer: &CompositeLayer) -> Result<(), CodecError> {
    encode_source_ref(out, layer.source())?;
    out.i32_be(layer.x_px())?;
    out.i32_be(layer.y_px())?;
    out.u16_be(layer.opacity_permille())?;
    out.push(match layer.blend() {
        BlendMode::SourceOver => 0,
    })
}

/// Encodes the full config snapshot without exceeding `max_bytes`. Every
/// field is encoded: mode, default backend, all purpose and backend gates,
/// transform and scheduler bounds and request limits.
pub(crate) fn encode_config(
    config: &ImageConfig,
    max_bytes: usize,
) -> Result<EncodedConfig, CodecError> {
    config.validate().map_err(CodecError::Config)?;
    let mut out = Writer::new(max_bytes);
    out.push(CONFIG_CODEC_VERSION)?;
    out.push(match config.mode {
        GlobalMode::Live => 0,
        GlobalMode::LocalOnly => 1,
        GlobalMode::CacheOnly => 2,
        GlobalMode::Off => 3,
    })?;
    out.push(backend_tag(config.default_backend))?;
    for gate in &config.purposes {
        encode_purpose_gate(&mut out, gate)?;
    }
    for gate in &config.backends {
        encode_backend_gate(&mut out, gate)?;
    }
    encode_transform_limits(&mut out, &config.transforms)?;
    encode_scheduler(&mut out, &config.scheduler)?;
    out.u64_be(u64::try_from(config.request_limits.max_prompt_chars).map_err(
        |_| CodecError::TooLong,
    )?)?;
    out.u64_be(
        u64::try_from(config.request_limits.max_references).map_err(|_| CodecError::TooLong)?,
    )?;
    out.u64_be(
        u64::try_from(config.request_limits.max_transform_steps)
            .map_err(|_| CodecError::TooLong)?,
    )?;
    out.u64_be(
        u64::try_from(config.request_limits.max_composite_layers)
            .map_err(|_| CodecError::TooLong)?,
    )?;
    let bytes = out.finish();
    let fingerprint = Sha256::digest(&bytes).into();
    Ok(EncodedConfig {
        version: CONFIG_CODEC_VERSION,
        bytes,
        fingerprint,
    })
}

fn backend_tag(backend: BackendId) -> u8 {
    match backend {
        BackendId::OpenAi => 0,
        BackendId::Fal => 1,
        BackendId::Bfl => 2,
        BackendId::CodexCli => 3,
    }
}

fn encode_purpose_gate(out: &mut Writer, gate: &PurposeGate) -> Result<(), CodecError> {
    out.boolean(gate.enabled)?;
    match gate.backend_override {
        Some(backend) => {
            out.push(1)?;
            out.push(backend_tag(backend))?;
        }
        None => out.push(0)?,
    }
    Ok(())
}

fn encode_backend_gate(out: &mut Writer, gate: &BackendGate) -> Result<(), CodecError> {
    out.boolean(gate.enabled)?;
    out.counted(gate.account.as_str().as_bytes())?;
    out.u32_be(gate.max_concurrent_jobs)
}

fn encode_transform_limits(
    out: &mut Writer,
    limits: &TransformLimits,
) -> Result<(), CodecError> {
    out.boolean(limits.enabled)?;
    out.u32_be(limits.max_concurrency)?;
    out.u64_be(limits.max_decoded_bytes)?;
    out.u64_be(limits.max_output_bytes)?;
    out.u32_be(limits.queue_capacity)?;
    out.u64_be(limits.deadline_secs)
}

fn encode_scheduler(out: &mut Writer, scheduler: &SchedulerBounds) -> Result<(), CodecError> {
    out.u32_be(scheduler.foreground_limit)?;
    out.u32_be(scheduler.presence_limit)?;
    out.u32_be(scheduler.background_limit)?;
    out.u32_be(scheduler.max_pending_jobs)?;
    out.u64_be(scheduler.max_pending_bytes)?;
    out.u64_be(scheduler.queue_age_secs)?;
    out.u64_be(scheduler.job_deadline_secs)?;
    out.u64_be(scheduler.stage_deadline_secs)
}
