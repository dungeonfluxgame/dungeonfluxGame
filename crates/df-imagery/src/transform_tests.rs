use super::*;
use crate::output::{Dimensions, Rgb};
use crate::request::{
    AssetDigest, DEFAULT_MAX_COMPOSITE_LAYERS, DEFAULT_MAX_TRANSFORM_STEPS, RequestLimits,
    SourceRef,
};
use df_assets::{AssetId, Audience};

fn source_with(byte: u8) -> SourceRef {
    SourceRef {
        asset: AssetId::from_bytes(&[byte; 16]).unwrap(),
        digest: AssetDigest::from_bytes([0x09; 32]),
        audience: Audience::PublicStatic,
    }
}

fn source() -> SourceRef {
    source_with(0x07)
}

fn limits() -> RequestLimits {
    RequestLimits::default()
}

fn canvas_recipe(first: ImageTransform, second: ImageTransform) -> TransformSpec {
    TransformSpec {
        source: TransformSource::Canvas(
            CanvasSpec::new(
                256,
                256,
                Rgb {
                    red: 32,
                    green: 32,
                    blue: 32,
                },
            )
            .unwrap(),
        ),
        steps: vec![first, second],
    }
}

fn resize(width_px: u32, height_px: u32) -> ImageTransform {
    ImageTransform::Resize(
        ResizeSpec::new(
            width_px,
            height_px,
            ResizeMode::Contain,
            AlignAnchor::center(),
            Interpolation::Lanczos3,
            false,
            None,
        )
        .unwrap(),
    )
}

fn gray() -> Rgb {
    Rgb {
        red: 18,
        green: 18,
        blue: 18,
    }
}

#[test]
fn an_empty_recipe_is_a_valid_encode_only_transform() {
    let spec = TransformSpec {
        source: TransformSource::Image(source()),
        steps: Vec::new(),
    };
    spec.validate(&limits()).unwrap();
}

#[test]
fn crop_then_resize_differs_from_resize_then_crop() {
    let crop = ImageTransform::Crop(PixelRect::new(0, 0, 128, 128).unwrap());
    let first = canvas_recipe(crop.clone(), resize(64, 64));
    let second = canvas_recipe(resize(64, 64), crop);
    first.validate(&limits()).unwrap();
    second.validate(&limits()).unwrap();
    assert_ne!(first.steps, second.steps);
}

#[test]
fn neutral_brightness_and_exposure_are_zero() {
    assert!(BrightnessOffset::new(0.0).unwrap().is_neutral());
    assert!(ExposureStops::new(0.0).unwrap().is_neutral());
    assert_eq!(BrightnessOffset::new(0.0).unwrap().value(), 0.0);
}

#[test]
fn brightness_and_exposure_accept_any_finite_value() {
    BrightnessOffset::new(2.0).unwrap();
    BrightnessOffset::new(-3.5).unwrap();
    ExposureStops::new(20.0).unwrap();
    ExposureStops::new(-20.0).unwrap();
}

#[test]
fn non_finite_brightness_is_refused() {
    assert_eq!(
        BrightnessOffset::new(f32::NAN).unwrap_err(),
        TransformError::NonFinite {
            field: "brightness"
        }
    );
    assert_eq!(
        BrightnessOffset::new(f32::INFINITY).unwrap_err(),
        TransformError::NonFinite {
            field: "brightness"
        }
    );
}

#[test]
fn non_finite_exposure_is_refused() {
    assert_eq!(
        ExposureStops::new(f32::NAN).unwrap_err(),
        TransformError::NonFinite { field: "exposure" }
    );
}

#[test]
fn blur_rejects_non_finite_radius_but_accepts_large_finite_radii() {
    assert_eq!(
        BlurSpec::new(f32::NAN, BlurAlgorithm::Gaussian).unwrap_err(),
        TransformError::NonFinite { field: "blur" }
    );
    assert_eq!(
        BlurSpec::new(0.0, BlurAlgorithm::Box).unwrap_err(),
        TransformError::OutOfRange { field: "blur" }
    );
    BlurSpec::new(500.0, BlurAlgorithm::Gaussian).unwrap();
}

#[test]
fn sharpen_rejects_non_finite_and_negative_amount() {
    assert_eq!(
        SharpenSpec::new(f32::NAN, 1.0, 0).unwrap_err(),
        TransformError::NonFinite { field: "sharpen" }
    );
    assert_eq!(
        SharpenSpec::new(1.0, -1.0, 0).unwrap_err(),
        TransformError::OutOfRange { field: "sharpen" }
    );
    SharpenSpec::new(200.0, 5.0, 255).unwrap();
}

#[test]
fn contrast_rejects_non_finite_and_non_positive_gain() {
    assert_eq!(
        ColorEffect::contrast(f32::NAN, 0.5).unwrap_err(),
        TransformError::NonFinite { field: "contrast" }
    );
    assert_eq!(
        ColorEffect::contrast(1.0, f32::NAN).unwrap_err(),
        TransformError::NonFinite { field: "contrast" }
    );
    assert_eq!(
        ColorEffect::contrast(0.0, 0.5).unwrap_err(),
        TransformError::OutOfRange { field: "contrast" }
    );
    assert_eq!(
        ColorEffect::contrast(1.0, 1.5).unwrap_err(),
        TransformError::OutOfRange { field: "contrast" }
    );
    ColorEffect::contrast(100.0, 1.0).unwrap();
}

#[test]
fn saturation_rejects_non_finite_and_negative_factor() {
    assert_eq!(
        ColorEffect::saturation(f32::NAN).unwrap_err(),
        TransformError::NonFinite {
            field: "saturation",
        }
    );
    assert_eq!(
        ColorEffect::saturation(-0.5).unwrap_err(),
        TransformError::OutOfRange {
            field: "saturation",
        }
    );
    ColorEffect::saturation(8.0).unwrap();
}

#[test]
fn hue_shift_rejects_non_finite_degrees_but_accepts_any_finite_turn() {
    assert_eq!(
        ColorEffect::hue_shift(f32::NAN).unwrap_err(),
        TransformError::NonFinite { field: "hue" }
    );
    ColorEffect::hue_shift(720.0).unwrap();
}

#[test]
fn literally_constructed_non_finite_color_is_refused_at_the_boundary() {
    for effect in [
        ColorEffect::Saturation { factor: f32::NAN },
        ColorEffect::Contrast {
            gain: f32::NAN,
            pivot: 0.5,
        },
        ColorEffect::Contrast {
            gain: 1.0,
            pivot: f32::INFINITY,
        },
        ColorEffect::HueShift { degrees: f32::NAN },
    ] {
        let spec = TransformSpec {
            source: TransformSource::Image(source()),
            steps: vec![ImageTransform::Color(effect)],
        };
        assert!(spec.validate(&limits()).is_err(), "{effect:?}");
    }
}

#[test]
fn literally_constructed_non_finite_rotation_is_refused_at_the_boundary() {
    let spec = TransformSpec {
        source: TransformSource::Image(source()),
        steps: vec![ImageTransform::Rotate(RotateSpec::Arbitrary {
            degrees: f32::NAN,
            expand_canvas: true,
            fill: gray(),
            interpolation: Interpolation::Bicubic,
        })],
    };
    assert_eq!(
        spec.validate(&limits()).unwrap_err(),
        TransformError::NonFinite { field: "rotation" }
    );
}

#[test]
fn arbitrary_rotation_accepts_full_turns_with_named_interpolation() {
    let spec = TransformSpec {
        source: TransformSource::Image(source()),
        steps: vec![ImageTransform::Rotate(RotateSpec::Arbitrary {
            degrees: 720.0,
            expand_canvas: false,
            fill: gray(),
            interpolation: Interpolation::Bilinear,
        })],
    };
    spec.validate(&limits()).unwrap();
    match &spec.steps[0] {
        ImageTransform::Rotate(RotateSpec::Arbitrary { interpolation, .. }) => {
            assert_eq!(*interpolation, Interpolation::Bilinear);
        }
        _ => unreachable!(),
    }
}

#[test]
fn quarter_turns_above_three_are_refused() {
    let spec = TransformSpec {
        source: TransformSource::Image(source()),
        steps: vec![ImageTransform::Rotate(RotateSpec::QuarterTurns(4))],
    };
    assert_eq!(
        spec.validate(&limits()).unwrap_err(),
        TransformError::OutOfRange { field: "rotation" }
    );
}

#[test]
fn mask_rejects_non_finite_and_negative_feather() {
    let mask = |feather: f32| {
        MaskSpec::new(
            MaskSource::Asset(source()),
            Dimensions::new(64, 64).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Alpha,
            false,
            feather,
            MaskMode::Multiply,
        )
    };
    assert_eq!(
        mask(f32::NAN).unwrap_err(),
        TransformError::NonFinite { field: "mask" }
    );
    assert_eq!(
        mask(-1.0).unwrap_err(),
        TransformError::OutOfRange { field: "mask" }
    );
    mask(400.0).unwrap();
}

#[test]
fn mask_carries_source_alignment_sizing_and_channel_policy() {
    let spec = MaskSpec::new(
        MaskSource::Asset(source()),
        Dimensions::new(64, 32).unwrap(),
        AlignAnchor::center(),
        MaskChannel::Luminance,
        true,
        2.0,
        MaskMode::Replace,
    )
    .unwrap();
    assert!(matches!(spec.source(), MaskSource::Asset(_)));
    assert_eq!((spec.width_px(), spec.height_px()), (64, 32));
    assert_eq!(spec.channel(), MaskChannel::Luminance);
    assert!(spec.invert());
    assert_eq!(spec.mode(), MaskMode::Replace);
    let recipe = TransformSpec {
        source: TransformSource::Image(source()),
        steps: vec![ImageTransform::Mask(spec)],
    };
    recipe.validate(&limits()).unwrap();
}

#[test]
fn mask_accepts_coverage_bound_to_its_analyzed_source() {
    let coverage = CoverageRef {
        coverage: source_with(0x0a),
        analyzed: source_with(0x0b),
        analysis_revision: 3,
    };
    assert_ne!(coverage.coverage.asset, coverage.analyzed.asset);
    let spec = MaskSpec::new(
        MaskSource::Coverage(coverage),
        Dimensions::new(64, 64).unwrap(),
        AlignAnchor::center(),
        MaskChannel::Alpha,
        false,
        0.0,
        MaskMode::Multiply,
    )
    .unwrap();
    let recipe = TransformSpec {
        source: TransformSource::Coverage(CoverageRef {
            coverage: source_with(0x0a),
            analyzed: source_with(0x0b),
            analysis_revision: 3,
        }),
        steps: vec![ImageTransform::Mask(spec)],
    };
    recipe.validate(&limits()).unwrap();
}

#[test]
fn zero_canvas_is_refused() {
    assert_eq!(
        CanvasSpec::new(0, 64, gray()).unwrap_err(),
        TransformError::ZeroSize
    );
}

#[test]
fn zero_resize_is_refused() {
    assert!(
        ResizeSpec::new(
            0,
            64,
            ResizeMode::Contain,
            AlignAnchor::center(),
            Interpolation::Nearest,
            false,
            None,
        )
        .is_err()
    );
}

#[test]
fn oversize_resize_targets_are_refused() {
    let pixels = 100_000_u64 * 100_000_u64;
    assert_eq!(
        ResizeSpec::new(
            100_000,
            100_000,
            ResizeMode::Contain,
            AlignAnchor::center(),
            Interpolation::Nearest,
            false,
            None,
        )
        .unwrap_err(),
        TransformError::TooManyPixels { pixels }
    );
}

#[test]
fn resize_pad_requires_an_explicit_fill_and_only_pad_takes_one() {
    assert_eq!(
        ResizeSpec::new(
            64,
            64,
            ResizeMode::Pad,
            AlignAnchor::center(),
            Interpolation::Bilinear,
            false,
            None,
        )
        .unwrap_err(),
        TransformError::MissingPadFill
    );
    assert_eq!(
        ResizeSpec::new(
            64,
            64,
            ResizeMode::Contain,
            AlignAnchor::center(),
            Interpolation::Bilinear,
            false,
            Some(gray()),
        )
        .unwrap_err(),
        TransformError::UnexpectedPadFill
    );
    ResizeSpec::new(
        64,
        64,
        ResizeMode::Pad,
        AlignAnchor::center(),
        Interpolation::Bilinear,
        false,
        Some(gray()),
    )
    .unwrap();
}

#[test]
fn composite_layer_count_honors_injected_limits() {
    let layers: Vec<CompositeLayer> = (0..=DEFAULT_MAX_COMPOSITE_LAYERS)
        .map(|_| CompositeLayer::new(source(), 0, 0, 1000, BlendMode::SourceOver).unwrap())
        .collect();
    let spec = TransformSpec {
        source: TransformSource::Image(source()),
        steps: vec![ImageTransform::Composite(layers)],
    };
    assert_eq!(
        spec.validate(&limits()).unwrap_err(),
        TransformError::TooManyLayers
    );
    let raised = RequestLimits {
        max_composite_layers: DEFAULT_MAX_COMPOSITE_LAYERS + 1,
        ..limits()
    };
    spec.validate(&raised).unwrap();
}

#[test]
fn composite_opacity_above_permille_is_refused() {
    assert_eq!(
        CompositeLayer::new(source(), 0, 0, 1001, BlendMode::SourceOver).unwrap_err(),
        TransformError::OutOfRange { field: "opacity" }
    );
    CompositeLayer::new(source(), -4, 7, 1000, BlendMode::SourceOver).unwrap();
}

#[test]
fn empty_composite_layers_are_refused() {
    let spec = TransformSpec {
        source: TransformSource::Image(source()),
        steps: vec![ImageTransform::Composite(Vec::new())],
    };
    assert_eq!(
        spec.validate(&limits()).unwrap_err(),
        TransformError::EmptyLayers
    );
}

#[test]
fn step_count_honors_injected_limits() {
    let steps = vec![ImageTransform::Flip(FlipAxis::Vertical); DEFAULT_MAX_TRANSFORM_STEPS + 1];
    let spec = TransformSpec {
        source: TransformSource::Image(source()),
        steps,
    };
    assert_eq!(
        spec.validate(&limits()).unwrap_err(),
        TransformError::TooManySteps
    );
    let raised = RequestLimits {
        max_transform_steps: DEFAULT_MAX_TRANSFORM_STEPS + 1,
        ..limits()
    };
    spec.validate(&raised).unwrap();
}
