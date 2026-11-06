use super::*;
use crate::{AnalysisMaskRun, AnalysisUnit, ImageConfig, SegmentationPrompt};

fn set() -> AnalysisMaskSet {
    AnalysisMaskSet {
        prompt: SegmentationPrompt::Person,
        width_px: 40,
        height_px: 20,
        instances: vec![AnalysisMask {
            index: 10,
            score: AnalysisUnit::new(0.95).unwrap(),
            runs: (5..15)
                .map(|row| AnalysisMaskRun {
                    start: row * 40 + 10,
                    length: 20,
                })
                .collect(),
        }],
    }
}

#[test]
fn projected_focus_alpha_keeps_the_old_aspect_crown_and_feather_pixels() {
    let set = set();
    let limits = ImageConfig::defaults().unwrap().transforms;
    let budget = LiveBudget::new(&limits).unwrap();
    let image = render(&set, &set.instances[0], 2400, 1200, &budget, &|| Ok(())).unwrap();
    assert_eq!(image.dimensions(), (768, 384));
    assert_eq!(image.get_pixel(384, 192).0, [255, 255, 255, 255]);
    assert_eq!(image.get_pixel(0, 0)[3], 0);
    assert!(image.get_pixel(192, 192)[3] > 0 && image.get_pixel(192, 192)[3] < 255);
    assert_eq!(image.get_pixel(180, 192)[3], 0);
    assert_eq!(image.get_pixel(204, 192)[3], 255);
    let portrait = render(&set, &set.instances[0], 400, 800, &budget, &|| Ok(())).unwrap();
    assert_eq!(portrait.dimensions(), (384, 768));
}

#[test]
fn source_derived_workspace_threshold_refuses_before_any_pixel_allocation() {
    let set = set();
    let needed = workspace_bytes(768, 384);
    let mut limits = ImageConfig::defaults().unwrap().transforms;
    limits.max_decoded_bytes = needed - 1;
    let budget = LiveBudget::new(&limits).unwrap();
    let called = std::cell::Cell::new(false);
    assert!(
        matches!(render(&set, &set.instances[0], 2400, 1200, &budget, &|| {
        called.set(true); Ok(())
    }), Err(RasterError::LiveBudgetExceeded { needed_bytes }) if needed_bytes == needed)
    );
    assert!(!called.get());
    limits.max_decoded_bytes = needed;
    assert!(
        render(
            &set,
            &set.instances[0],
            2400,
            1200,
            &LiveBudget::new(&limits).unwrap(),
            &|| Ok(())
        )
        .is_ok()
    );
}

#[test]
fn invalid_overlapping_runs_and_cancelled_projection_fail_explicitly() {
    let mut set = set();
    set.instances[0].runs.insert(
        1,
        AnalysisMaskRun {
            start: 5 * 40 + 10,
            length: 1,
        },
    );
    assert_eq!(
        validate_runs(&set, &set.instances[0]),
        Err(RasterError::ResolveFailed)
    );
    let set = self::set();
    let limits = ImageConfig::defaults().unwrap().transforms;
    let count = std::cell::Cell::new(0);
    let result = render(
        &set,
        &set.instances[0],
        2400,
        1200,
        &LiveBudget::new(&limits).unwrap(),
        &|| {
            count.set(count.get() + 1);
            if count.get() >= 3 {
                Err(RasterError::Cancelled)
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(RasterError::Cancelled)));
}

fn entry_fixture(
    width: u32,
    height: u32,
) -> (
    crate::TransformSpec,
    crate::RasterOutputSpec,
    ImageAnalysisResult,
    Vec<u8>,
) {
    use sha2::{Digest, Sha256};
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        width,
        height,
        image::Rgb([23, 45, 67]),
    ))
    .write_to(&mut bytes, image::ImageFormat::Png)
    .unwrap();
    let bytes = bytes.into_inner();
    let pin = crate::SourceRef {
        asset: df_assets::AssetId::from_bytes(&[1; 16]).unwrap(),
        digest: crate::AssetDigest::from_bytes(Sha256::digest(&bytes).into()),
        audience: df_assets::Audience::PublicStatic,
    };
    let source = TransformSource::SegmentationMask {
        analyzed: pin.clone(),
        segmentation: crate::OperationHandle {
            owner: crate::ImageOwner::Draft {
                account: df_types::AccountId::from_bytes(&[2; 16]).unwrap(),
            },
            operation: df_types::OperationId::from_bytes(&[3; 16]).unwrap(),
        },
        prompt: SegmentationPrompt::Person,
        mask_index: 10,
        contract_revision: 1,
    };
    let scale = 768.0 / f64::from(width.max(height).max(768));
    let dimensions = crate::Dimensions::new(
        (f64::from(width) * scale).round().max(1.0) as u32,
        (f64::from(height) * scale).round().max(1.0) as u32,
    )
    .unwrap();
    (
        crate::TransformSpec {
            source,
            steps: Vec::new(),
        },
        crate::RasterOutputSpec::new(
            dimensions,
            crate::AlphaHandling::PreserveAlpha,
            crate::Encoding::Png,
        )
        .unwrap(),
        ImageAnalysisResult::Segmentation {
            source: pin,
            contract_revision: 1,
            masks: vec![set()],
        },
        bytes,
    )
}

// Frozen raster_focus_mask algorithm from the former gameplay consumer. It consumes
// MaskRows.row_segments independently of the new kernel's run projection loop.
fn legacy_png(source_width: u32, source_height: u32, mask: &df_providers::MaskRows) -> Vec<u8> {
    let longest = source_width.max(source_height);
    let scale = 768.0 / f64::from(longest.max(768));
    let width = (f64::from(source_width) * scale).round().max(1.0) as u32;
    let height = (f64::from(source_height) * scale).round().max(1.0) as u32;
    let mut alpha = image::GrayImage::new(width, height);
    let project = |pixel: u32, output: u32, input: u32| {
        ((u64::from(pixel) * u64::from(output) + u64::from(input / 2)) / u64::from(input)) as u32
    };
    for (row, from, to) in mask.row_segments() {
        for y in project(row, height, mask.height())..project(row + 1, height, mask.height()) {
            for x in project(from, width, mask.width())..project(to, width, mask.width()) {
                alpha.put_pixel(x, y, image::Luma([255]));
            }
        }
    }
    let alpha = image::imageops::blur(&alpha, (height as f32 * 0.005).max(0.5));
    let rgba = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([255, 255, 255, alpha.get_pixel(x, y).0[0]])
    });
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    png.into_inner()
}

#[test]
fn complete_private_entry_matches_legacy_png_bytes_for_small_landscape_portrait_and_thin_sources() {
    let limits = ImageConfig::defaults().unwrap().transforms;
    let selected = set();
    let mask = df_providers::MaskRows::from_runs(
        40,
        20,
        selected.instances[0]
            .runs
            .iter()
            .map(|run| df_providers::MaskRun {
                start: run.start,
                length: run.length,
            })
            .collect(),
    )
    .unwrap();
    for (width, height) in [(40, 20), (2400, 1200), (400, 800), (1, 1200)] {
        let (spec, output, analysis, bytes) = entry_fixture(width, height);
        let actual = crate::raster::execute_segmentation_mask(
            &spec,
            &output,
            &crate::RequestLimits::default(),
            &limits,
            &analysis,
            &|_| Ok(bytes.clone()),
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(
            actual.bytes,
            legacy_png(width, height, &mask),
            "{width}x{height}"
        );
    }
}

#[test]
fn private_entry_charges_retained_analysis_capacity_at_the_workspace_boundary() {
    let (spec, output, mut analysis, bytes) = entry_fixture(40, 20);
    let ImageAnalysisResult::Segmentation { masks, .. } = &mut analysis else {
        unreachable!()
    };
    // Spare capacity is resident too, even though the renderer reads only ten runs.
    masks[0].instances[0].runs.reserve_exact(1000);
    let retained = retained_bytes(&analysis).unwrap();
    let workspace = workspace_bytes(40, 20);
    assert!(workspace > bytes.len() as u64);
    let mut limits = ImageConfig::defaults().unwrap().transforms;
    limits.max_decoded_bytes = retained + workspace - 1;
    let error = crate::raster::execute_segmentation_mask(
        &spec,
        &output,
        &crate::RequestLimits::default(),
        &limits,
        &analysis,
        &|_| Ok(bytes.clone()),
        &|| Ok(()),
    )
    .unwrap_err();
    assert_eq!(
        error,
        RasterError::LiveBudgetExceeded {
            needed_bytes: workspace
        }
    );
    limits.max_decoded_bytes += 1;
    assert!(
        crate::raster::execute_segmentation_mask(
            &spec,
            &output,
            &crate::RequestLimits::default(),
            &limits,
            &analysis,
            &|_| Ok(bytes.clone()),
            &|| Ok(()),
        )
        .is_ok()
    );
}

#[test]
fn private_entry_rejects_wrong_source_revision_duplicate_sets_missing_index_and_wrong_digest() {
    let (spec, output, analysis, bytes) = entry_fixture(40, 20);
    let limits = ImageConfig::defaults().unwrap().transforms;
    let run = |value: &ImageAnalysisResult, content: &[u8]| {
        crate::raster::execute_segmentation_mask(
            &spec,
            &output,
            &crate::RequestLimits::default(),
            &limits,
            value,
            &|_| Ok(content.to_vec()),
            &|| Ok(()),
        )
    };
    let mut wrong = analysis.clone();
    if let ImageAnalysisResult::Segmentation { source, .. } = &mut wrong {
        source.asset = df_assets::AssetId::from_bytes(&[4; 16]).unwrap();
    }
    assert_eq!(run(&wrong, &bytes).unwrap_err(), RasterError::ResolveFailed);
    let mut wrong = analysis.clone();
    if let ImageAnalysisResult::Segmentation {
        contract_revision, ..
    } = &mut wrong
    {
        *contract_revision = 2;
    }
    assert_eq!(run(&wrong, &bytes).unwrap_err(), RasterError::ResolveFailed);
    let mut wrong = analysis.clone();
    if let ImageAnalysisResult::Segmentation { masks, .. } = &mut wrong {
        masks.push(masks[0].clone());
    }
    assert_eq!(run(&wrong, &bytes).unwrap_err(), RasterError::ResolveFailed);
    let mut wrong = analysis.clone();
    if let ImageAnalysisResult::Segmentation { masks, .. } = &mut wrong {
        masks[0].instances[0].index = 11;
    }
    assert_eq!(run(&wrong, &bytes).unwrap_err(), RasterError::ResolveFailed);
    assert!(matches!(
        run(&analysis, b"wrong digest"),
        Err(RasterError::DigestMismatch)
    ));
}
