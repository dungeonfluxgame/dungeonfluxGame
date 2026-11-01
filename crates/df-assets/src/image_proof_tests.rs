use crate::*;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use sha2::{Digest, Sha256};
use std::io::Cursor;

fn fixture(alpha: u8) -> (VariantRecord, Vec<u8>, ImageIntent) {
    let mut buffer = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 3, Rgba([10, 20, 30, alpha])))
        .write_to(&mut buffer, ImageFormat::Png)
        .unwrap();
    let bytes = buffer.into_inner();
    let asset = AssetId::from_bytes(&[1; 16]).unwrap();
    let attempt = AttemptId::from_bytes(&[2; 16]).unwrap();
    let write = WriteId::from_bytes(&[3; 16]).unwrap();
    let record = VariantRecord {
        asset,
        attempt,
        write,
        variant: AssetVariant::Master,
        owner: ClaimToken::from_bytes(&[4; 16]).unwrap(),
        state: VariantState::Written,
        key: KeyLayout::new(ObjectEnvironment::new("proof").unwrap()).campaign_asset(
            &df_types::CampaignId::from_bytes(&[5; 16]).unwrap(),
            asset,
            attempt,
            write,
            AssetVariant::Master,
        ),
        version: ObjectVersion::new("opaque/etag-2".into()),
        manifest: Some(AssetManifest {
            byte_len: bytes.len().try_into().unwrap(),
            sha256: Sha256::digest(&bytes).into(),
        }),
        created_at_ms: 1,
    };
    let intent = ImageIntent::new(
        "image/png",
        2,
        3,
        if alpha < 255 {
            ImageAlpha::Transparent
        } else {
            ImageAlpha::Opaque
        },
    )
    .unwrap();
    (record, bytes, intent)
}

#[test]
fn actual_encoded_pixels_prove_alpha_dimensions_mime_and_digest() {
    for alpha in [0, 127, 255] {
        let (record, bytes, intent) = fixture(alpha);
        let proof = ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            intent,
            ImageProofLimits::default(),
        )
        .unwrap();
        assert_eq!(proof.record(), &record);
        assert_eq!(proof.intent(), intent);
        assert_eq!(proof.recipe_digest(), &[7; 32]);
    }
}

#[test]
fn caller_metadata_and_manifest_cannot_attest_invalid_bytes() {
    let (mut record, bytes, intent) = fixture(127);
    let opaque = ImageIntent::new("image/png", 2, 3, ImageAlpha::Opaque).unwrap();
    assert_eq!(
        ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            opaque,
            ImageProofLimits::default()
        ),
        Err(ImageProofError::IntentMismatch)
    );
    let wrong_size = ImageIntent::new("image/png", 3, 2, ImageAlpha::Transparent).unwrap();
    assert_eq!(
        ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            wrong_size,
            ImageProofLimits::default()
        ),
        Err(ImageProofError::IntentMismatch)
    );
    let jpeg = ImageIntent::new("image/jpeg", 2, 3, ImageAlpha::Transparent).unwrap();
    assert_eq!(
        ImageProof::decode(&record, &bytes, [7; 32], jpeg, ImageProofLimits::default()),
        Err(ImageProofError::IntentMismatch)
    );
    record.manifest.as_mut().unwrap().sha256 = [1; 32];
    assert_eq!(
        ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            intent,
            ImageProofLimits::default()
        ),
        Err(ImageProofError::IntegrityMismatch)
    );
    record.manifest = Some(AssetManifest {
        byte_len: 3,
        sha256: Sha256::digest(b"bad").into(),
    });
    assert_eq!(
        ImageProof::decode(
            &record,
            b"bad",
            [7; 32],
            intent,
            ImageProofLimits::default()
        ),
        Err(ImageProofError::UnsupportedFormat)
    );
}

#[test]
fn intended_and_unversioned_writes_cannot_be_proven() {
    let (mut record, bytes, intent) = fixture(255);
    record.state = VariantState::Intended;
    assert_eq!(
        ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            intent,
            ImageProofLimits::default()
        ),
        Err(ImageProofError::NotWritten)
    );
    record.state = VariantState::Written;
    record.version = None;
    assert_eq!(
        ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            intent,
            ImageProofLimits::default()
        ),
        Err(ImageProofError::NotWritten)
    );
}

#[test]
fn sixteen_bit_alpha_is_measured_without_quantization() {
    let (mut record, _, _) = fixture(255);
    let pixels = image::ImageBuffer::from_pixel(1, 1, image::Rgba([0_u16, 0, 0, 65534]));
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba16(pixels)
        .write_to(&mut output, ImageFormat::Png)
        .unwrap();
    let bytes = output.into_inner();
    record.manifest = Some(AssetManifest {
        byte_len: bytes.len().try_into().unwrap(),
        sha256: Sha256::digest(&bytes).into(),
    });
    let transparent = ImageIntent::new("image/png", 1, 1, ImageAlpha::Transparent).unwrap();
    assert!(
        ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            transparent,
            ImageProofLimits::default()
        )
        .is_ok()
    );
    let opaque = ImageIntent::new("image/png", 1, 1, ImageAlpha::Opaque).unwrap();
    assert_eq!(
        ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            opaque,
            ImageProofLimits::default()
        ),
        Err(ImageProofError::IntentMismatch)
    );
}

#[test]
fn injected_resource_bounds_allow_raised_configuration_without_large_allocations() {
    let (record, bytes, intent) = fixture(255);
    let limits = ImageProofLimits::new(1, 1, 1).unwrap();
    assert_eq!(
        ImageProof::decode(&record, &bytes, [7; 32], intent, limits),
        Err(ImageProofError::TooLarge)
    );
    let limits = ImageProofLimits::new(1024, 1, 1024).unwrap();
    assert_eq!(
        ImageProof::decode(&record, &bytes, [7; 32], intent, limits),
        Err(ImageProofError::TooManyPixels)
    );
    let large = ImageIntent::new("image/png", 6000, 6000, ImageAlphaRequirement::Preserve).unwrap();
    assert_eq!(
        ImageProofLimits::default().check_dimensions(large.width_px(), large.height_px()),
        Err(ImageProofError::TooManyPixels)
    );
    let raised = ImageProofLimits::new(256 * 1024 * 1024, 64_000_000, 512 * 1024 * 1024).unwrap();
    assert_eq!(
        raised.check_dimensions(large.width_px(), large.height_px()),
        Ok(())
    );
}

#[test]
fn preserved_alpha_accepts_both_opaque_and_transparent_pixels_and_records_actual_fact() {
    for alpha in [127, 255] {
        let (record, bytes, _) = fixture(alpha);
        let intent = ImageIntent::new("image/png", 2, 3, ImageAlphaRequirement::Preserve).unwrap();
        let proof = ImageProof::decode(
            &record,
            &bytes,
            [7; 32],
            intent,
            ImageProofLimits::default(),
        )
        .unwrap();
        assert_eq!(
            proof.alpha(),
            if alpha == 255 {
                ImageAlpha::Opaque
            } else {
                ImageAlpha::Transparent
            }
        );
    }
}
