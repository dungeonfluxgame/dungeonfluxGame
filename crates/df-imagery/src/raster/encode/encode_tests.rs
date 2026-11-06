use super::*;
use crate::config::TransformLimits;
use crate::output::{Dimensions, LossyQuality, Rgb};

#[test]
fn lossy_admission_retains_source_rgb_and_matte_until_native_return() {
    let width_px = 7;
    let height_px = 5;
    let pixels = u64::from(width_px) * u64::from(height_px);
    for (alpha, retained_bytes_per_pixel) in [
        (AlphaHandling::PreserveAlpha, 4),
        (AlphaHandling::Opaque, 7),
        (
            AlphaHandling::Matte(Rgb {
                red: 0,
                green: 0,
                blue: 0,
            }),
            11,
        ),
    ] {
        let source_alpha = if alpha == AlphaHandling::Opaque {
            255
        } else {
            128
        };
        let image = RgbaImage::from_pixel(
            width_px,
            height_px,
            image::Rgba([200, 30, 30, source_alpha]),
        );
        let output = RasterOutputSpec::new(
            Dimensions::new(width_px, height_px).unwrap(),
            alpha,
            Encoding::WebP {
                quality: Some(LossyQuality::new(100).unwrap()),
            },
        )
        .unwrap();
        let required_bytes = super::super::webp_lossy::estimate_native_peak_bytes(
            width_px,
            height_px,
            alpha == AlphaHandling::PreserveAlpha,
            1_024,
        ) + pixels * retained_bytes_per_pixel;
        let mut limits = TransformLimits {
            enabled: true,
            max_concurrency: 1,
            max_decoded_bytes: required_bytes - 1,
            max_output_bytes: 1_024,
            queue_capacity: 1,
            deadline_secs: 30,
        };
        let check = || Ok(());
        let refused = encode_output(&image, &output, &LiveBudget::new(&limits).unwrap(), &check);
        assert!(matches!(refused,
            Err(RasterError::LiveBudgetExceeded { needed_bytes })
                if needed_bytes == required_bytes
        ));
        limits.max_decoded_bytes = required_bytes;
        let encoded =
            encode_output(&image, &output, &LiveBudget::new(&limits).unwrap(), &check).unwrap();
        let decoded = image::load_from_memory(&encoded.bytes).unwrap();
        assert_eq!(decoded.width(), width_px);
        assert_eq!(decoded.height(), height_px);
    }
}
