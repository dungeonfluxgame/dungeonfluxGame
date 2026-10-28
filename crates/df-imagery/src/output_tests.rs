use super::*;

fn opaque_jpeg(width_px: u32, height_px: u32) -> RasterOutputSpec {
    RasterOutputSpec::new(
        Dimensions::new(width_px, height_px).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(DEFAULT_JPEG_QUALITY_0_100).unwrap(),
        },
    )
    .unwrap()
}

#[test]
fn jpeg_with_preserved_alpha_is_refused_without_matte() {
    assert_eq!(
        RasterOutputSpec::new(
            Dimensions::new(64, 64).unwrap(),
            AlphaHandling::PreserveAlpha,
            Encoding::Jpeg {
                quality: LossyQuality::new(85).unwrap(),
            },
        )
        .unwrap_err(),
        OutputError::AlphaWithoutMatte
    );
}

#[test]
fn matte_on_alpha_capable_formats_flattens_onto_an_explicit_color() {
    for encoding in [
        Encoding::Png,
        Encoding::WebP {
            quality: Some(LossyQuality::new(90).unwrap()),
        },
    ] {
        RasterOutputSpec::new(
            Dimensions::new(64, 64).unwrap(),
            AlphaHandling::Matte(Rgb {
                red: 128,
                green: 128,
                blue: 128,
            }),
            encoding,
        )
        .unwrap();
    }
}

#[test]
fn jpeg_from_opaque_pixels_needs_no_matte() {
    let spec = opaque_jpeg(64, 64);
    spec.validate().unwrap();
    spec.validate_for_source_alpha(false).unwrap();
}

#[test]
fn dropping_alpha_from_alpha_bearing_input_requires_an_explicit_matte() {
    let opaque_jpeg = opaque_jpeg(64, 64);
    assert_eq!(
        opaque_jpeg.validate_for_source_alpha(true).unwrap_err(),
        OutputError::AlphaWithoutMatte
    );
    let opaque_png = RasterOutputSpec::new(
        Dimensions::new(64, 64).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    assert_eq!(
        opaque_png.validate_for_source_alpha(true).unwrap_err(),
        OutputError::AlphaWithoutMatte
    );
    let matte_png = RasterOutputSpec::new(
        Dimensions::new(64, 64).unwrap(),
        AlphaHandling::Matte(Rgb {
            red: 18,
            green: 18,
            blue: 18,
        }),
        Encoding::Png,
    )
    .unwrap();
    matte_png.validate_for_source_alpha(true).unwrap();
}

#[test]
fn zero_dimensions_cannot_be_constructed() {
    assert_eq!(
        Dimensions::new(0, 64).unwrap_err(),
        OutputError::ZeroDimension
    );
    assert_eq!(
        Dimensions::new(64, 0).unwrap_err(),
        OutputError::ZeroDimension
    );
}

#[test]
fn oversized_dimensions_are_refused() {
    let pixels = 100_000_u64 * 100_000_u64;
    assert_eq!(
        Dimensions::new(100_000, 100_000).unwrap_err(),
        OutputError::TooManyPixels { pixels }
    );
}

#[test]
fn zero_quality_is_refused() {
    assert_eq!(
        LossyQuality::new(0).unwrap_err(),
        OutputError::QualityOutOfRange { value: 0 }
    );
}

#[test]
fn quality_above_one_hundred_is_refused() {
    assert_eq!(
        LossyQuality::new(101).unwrap_err(),
        OutputError::QualityOutOfRange { value: 101 }
    );
    assert_eq!(
        LossyQuality::new(255).unwrap_err(),
        OutputError::QualityOutOfRange { value: 255 }
    );
}

#[test]
fn quality_at_one_hundred_is_accepted() {
    assert_eq!(LossyQuality::new(100).unwrap().value(), 100);
    assert_eq!(LossyQuality::new(1).unwrap().value(), 1);
}

#[test]
fn generated_and_delivered_sizes_stay_independent() {
    let generated = Dimensions::new(1024, 1536).unwrap();
    let profile = OutputProfile::provider(generated, opaque_jpeg(96, 144));
    assert_eq!(profile.generated(), Some(generated));
    assert!(
        profile.delivered().dimensions().pixel_count() < profile.generated().unwrap().pixel_count()
    );
}

#[test]
fn local_profile_carries_no_generated_size() {
    let profile = OutputProfile::local(opaque_jpeg(64, 64));
    assert_eq!(profile.generated(), None);
    assert_eq!(
        profile.delivered().dimensions(),
        Dimensions::new(64, 64).unwrap()
    );
}

#[test]
fn shared_pixel_ceiling_matches_asset_admission() {
    assert_eq!(MAXIMUM_OUTPUT_PIXELS, df_assets::MAX_IMAGE_PIXELS);
}
