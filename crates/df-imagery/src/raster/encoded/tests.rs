use super::*;
use std::cell::Cell;

fn budget(max_decoded_bytes: u64) -> LiveBudget {
    LiveBudget::new(&crate::TransformLimits {
        enabled: true,
        max_concurrency: 1,
        max_decoded_bytes,
        max_output_bytes: 1_000_000,
        queue_capacity: 1,
        deadline_secs: 30,
    })
    .unwrap()
}

#[test]
fn resize_counts_source_float_intermediate_output_and_vector_growth_before_sampling() {
    let source = RgbaImage::from_pixel(17, 23, Rgba([30, 60, 90, 128]));
    let dimensions = Dimensions::new(9, 11).unwrap();
    let required_bytes = 17 * 23 * 4 + 9 * 11 * 4 + 17 * 11 * 16 + (17 + 23) * 3 * 4;
    let calls = Cell::new(0);
    let check = || {
        calls.set(calls.get() + 1);
        Ok(())
    };
    let refused = resize(
        source.clone(),
        dimensions,
        EncodedFilter::Triangle,
        &budget(required_bytes - 1),
        &check,
    );
    assert_eq!(
        refused,
        Err(RasterError::LiveBudgetExceeded {
            needed_bytes: required_bytes
        })
    );
    assert_eq!(calls.get(), 0);
    assert!(
        resize(
            source,
            dimensions,
            EncodedFilter::Triangle,
            &budget(required_bytes),
            &check
        )
        .is_ok()
    );
}

#[test]
fn library_sampling_returns_cancelled_after_the_owned_synchronous_call() {
    for uses_thumbnail in [false, true] {
        let source = RgbaImage::from_pixel(17, 23, Rgba([30, 60, 90, 128]));
        let dimensions = Dimensions::new(9, 11).unwrap();
        let calls = Cell::new(0);
        let check = || {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err(RasterError::Cancelled)
            } else {
                Ok(())
            }
        };
        let result = if uses_thumbnail {
            thumbnail(source, dimensions, &budget(1_000_000), &check)
        } else {
            resize(
                source,
                dimensions,
                EncodedFilter::Lanczos3,
                &budget(1_000_000),
                &check,
            )
        };
        assert_eq!(result, Err(RasterError::Cancelled));
        assert_eq!(calls.get(), 2);
    }
}

#[test]
fn byte_loops_poll_cancellation_between_rows() {
    for discards_alpha in [false, true] {
        let source = RgbaImage::from_pixel(8, 64, Rgba([30, 60, 90, 128]));
        let calls = Cell::new(0);
        let check = || {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                Err(RasterError::Cancelled)
            } else {
                Ok(())
            }
        };
        let result = if discards_alpha {
            discard_alpha(source, &check)
        } else {
            matte(
                source,
                Rgb {
                    red: 128,
                    green: 128,
                    blue: 128,
                },
                EncodedAlphaBlend::IntegerFloor,
                &check,
            )
        };
        assert_eq!(result, Err(RasterError::Cancelled));
        assert_eq!(calls.get(), 3);
    }
}

#[test]
fn discard_alpha_keeps_hidden_rgb_and_floor_matte_matches_fixed_channel_values() {
    let hidden = RgbaImage::from_pixel(1, 1, Rgba([200, 30, 60, 0]));
    assert_eq!(
        discard_alpha(hidden, &|| Ok(())).unwrap().get_pixel(0, 0).0,
        [200, 30, 60, 255]
    );
    let translucent = RgbaImage::from_pixel(1, 1, Rgba([200, 30, 60, 128]));
    let flat = matte(
        translucent,
        Rgb {
            red: 128,
            green: 128,
            blue: 128,
        },
        EncodedAlphaBlend::IntegerFloor,
        &|| Ok(()),
    )
    .unwrap();
    assert_eq!(flat.get_pixel(0, 0).0, [164, 78, 93, 255]);
}
