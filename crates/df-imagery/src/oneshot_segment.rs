//! One segmentation of one picture through the fal SAM backend (SAM 3.1 by default), with no
//! journal, no database and no durable job.
//!
//! The durable service is the way the game asks for analyses it will keep; this is the direct door
//! for a developer tool that wants the layout of a single picture now, beside `oneshot` for
//! generation. It builds the same supplier request the service's segmentation stage builds (the
//! picture prepared as a scene plate, one noun prompt, fal's queue submit, GET-only polling to
//! completion) and reads the answer with the same parser, so its masks are the service's masks:
//! zero-based runs over the prepared picture's size, which is the source size up to a long side
//! of 2560 pixels.
//!
//! It never prices anything, never retries and never falls back to another provider: a refusal,
//! a timeout or a malformed answer comes back as a typed `SegmentError` and the caller decides.
//! The credential is passed in by the caller and never logged; nothing here reads the environment.
//!
//! `segment` is `Send`, so a host awaits it on its own task; hashing, decoding and preparing the
//! picture run on a blocking thread.

use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::backends::analysis::standalone::{
    PreparedSegmentation, SegmentationInput, parse_segmentation,
};
use crate::config::{AnalysisPreparationLimits, SegmentationProvider, SegmentationRoute};
use crate::listed::listed_enum;
use crate::request::{AssetDigest, SourceRef};
use crate::runtime::RuntimeError;
use crate::service::analysis_types::SegmentationPreparation;
pub use crate::service::analysis_types::{
    AnalysisMask, AnalysisMaskRun, AnalysisMaskSet, AnalysisUnit, SegmentationPrompt,
};
use crate::{AccountHandle, ApiEndpoint, ModelName};

/// fal's queue origin.
pub const DEFAULT_ENDPOINT: &str = "https://queue.fal.run";
/// The owner's segmentation model.
pub const DEFAULT_MODEL: &str = "fal-ai/sam-3-1/image-rle";
/// The most instances one scan returns (the supplier contract's bound).
pub const MAXIMUM_MASKS: u32 = 32;
/// The largest answer read, in bytes: the native HTTP client's own ceiling.
pub const MAXIMUM_RESPONSE_BYTES: usize = df_providers::MAXIMUM_BODY_BYTES;
/// The longest a scan may take: the native HTTP client's own ceiling.
pub const MAXIMUM_TIMEOUT: Duration = df_providers::MAXIMUM_EXCHANGE_TIME;
/// The most decoded picture memory the preparation may use, in bytes.
pub const MAXIMUM_DECODED_BYTES: u64 = 512 * 1024 * 1024;
/// The largest prepared picture, in bytes.
pub const MAXIMUM_PICTURE_BYTES: u64 = 64 * 1024 * 1024;
/// The largest supplier request, in bytes: the native HTTP client's own ceiling (a lossless
/// widening; `usize` is at most 64 bits on every supported target).
pub const MAXIMUM_REQUEST_BYTES: u64 = df_providers::MAXIMUM_BODY_BYTES as u64;

/// Where to send the scan and with what key.
#[derive(Clone)]
pub struct SegmentConfig {
    /// The supplier origin, such as `https://queue.fal.run`.
    pub endpoint: String,
    /// The model path, such as `fal-ai/sam-3-1/image-rle`.
    pub model: String,
    /// The supplier key. Never logged.
    pub credential: String,
    /// The absolute limit for submit and polling together.
    pub timeout: Duration,
}

impl std::fmt::Debug for SegmentConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SegmentConfig")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// One picture to scan.
#[derive(Debug, Clone)]
pub struct SegmentRequest {
    /// The picture's bytes (PNG, JPEG or WebP).
    pub source: Vec<u8>,
    /// The one noun the supplier looks for.
    pub prompt: SegmentationPrompt,
    /// 1 to `MAXIMUM_MASKS`.
    pub maximum_masks: u32,
}

listed_enum! {
    keyed
    /// Why a scan did not give masks.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SegmentError {
        /// The endpoint, model, key or limits are not usable.
        Configuration,
        /// The picture could not be read or prepared within the limits.
        Picture,
        /// The supplier refused the request.
        Refused,
        /// The supplier failed, ran out of quota or rate-limited the key.
        Supplier,
        /// The time limit passed before the answer.
        Timeout,
        /// The answer did not have the contract's shape.
        Malformed,
    }
}

impl SegmentError {
    /// A short stable name for a log line.
    pub fn key(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Picture => "picture",
            Self::Refused => "refused",
            Self::Supplier => "supplier",
            Self::Timeout => "timeout",
            Self::Malformed => "malformed",
        }
    }

    fn from_runtime(error: &RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidConfiguration | RuntimeError::CredentialUnavailable => {
                Self::Configuration
            }
            RuntimeError::SourceDenied | RuntimeError::UnsupportedCapability => Self::Picture,
            RuntimeError::ProviderRefused => Self::Refused,
            RuntimeError::Expired | RuntimeError::Cancelled => Self::Timeout,
            RuntimeError::SupplierFailure(_)
            | RuntimeError::SupplierQuotaExhausted
            | RuntimeError::SupplierRateLimited
            | RuntimeError::QuotaBlocked { .. }
            | RuntimeError::ProviderUnknown => Self::Supplier,
            _ => Self::Malformed,
        }
    }
}

impl std::fmt::Display for SegmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.key())
    }
}

impl std::error::Error for SegmentError {}

/// Scans one picture and returns its masks at the prepared picture's size.
pub async fn segment(
    config: &SegmentConfig,
    request: SegmentRequest,
) -> Result<AnalysisMaskSet, SegmentError> {
    if !(1..=MAXIMUM_MASKS).contains(&request.maximum_masks)
        || config.timeout.is_zero()
        || config.timeout > MAXIMUM_TIMEOUT
    {
        return Err(SegmentError::Configuration);
    }
    let route = SegmentationRoute {
        provider: SegmentationProvider::Fal,
        account: AccountHandle::new("oneshot-segment").map_err(|_| SegmentError::Configuration)?,
        model: ModelName::new(&config.model).map_err(|_| SegmentError::Configuration)?,
        endpoint: ApiEndpoint::new(&config.endpoint).map_err(|_| SegmentError::Configuration)?,
    };
    let (prompt, maximum_masks) = (request.prompt, request.maximum_masks);
    let (timeout, credential) = (config.timeout, config.credential.clone());
    let mut prepared =
        tokio::task::spawn_blocking(move || prepare(&request, &route, timeout, &credential))
            .await
            // A preparation that panicked never produced a picture to send.
            .map_err(|_| SegmentError::Picture)??;
    let (width_px, height_px) = (prepared.width_px, prepared.height_px);
    let raw = prepared
        .submit()
        .await
        .map_err(|error| SegmentError::from_runtime(&error))?;
    let checkpoint = prepared
        .checkpoint(&raw)
        .map_err(|error| SegmentError::from_runtime(&error))?;
    let answer = prepared
        .resume(&checkpoint)
        .await
        .map_err(|error| SegmentError::from_runtime(&error))?;
    let value: serde_json::Value =
        serde_json::from_slice(&answer.body).map_err(|_| SegmentError::Malformed)?;
    parse_segmentation(&value, prompt, width_px, height_px, maximum_masks)
        .map_err(|_| SegmentError::Malformed)
}

/// Hashes, decodes and prepares the picture as a scene plate, on a blocking thread.
fn prepare(
    request: &SegmentRequest,
    route: &SegmentationRoute,
    timeout: Duration,
    credential: &str,
) -> Result<PreparedSegmentation, SegmentError> {
    let source_digest: [u8; 32] = Sha256::digest(&request.source).into();
    let pin = SourceRef {
        asset: df_assets::AssetId::from_bytes(&source_digest[..16])
            .map_err(|_| SegmentError::Picture)?,
        digest: AssetDigest::from_bytes(source_digest),
        audience: df_assets::Audience::PublicStatic,
    };
    let preparation = AnalysisPreparationLimits {
        max_picture_bytes: MAXIMUM_PICTURE_BYTES,
        max_request_bytes: MAXIMUM_REQUEST_BYTES,
        ..Default::default()
    };
    PreparedSegmentation::prepare(
        SegmentationInput {
            source: &request.source,
            source_pin: &pin,
            prompt: request.prompt,
            preparation_recipe: SegmentationPreparation::ScenePlate,
            maximum_masks: request.maximum_masks,
            route,
            preparation: &preparation,
            max_decoded_bytes: MAXIMUM_DECODED_BYTES,
            timeout,
            maximum_response_bytes: MAXIMUM_RESPONSE_BYTES,
        },
        credential,
    )
    .map_err(|error| match error {
        RuntimeError::InvalidRecord | RuntimeError::SourceDenied => SegmentError::Picture,
        // Preparation refuses limits the HTTP client cannot take as an unsupported capability.
        RuntimeError::UnsupportedCapability => SegmentError::Configuration,
        other => SegmentError::from_runtime(&other),
    })
}

/// The SHA-256 of a picture's bytes as lowercase hex: the identity a layout is bound to.
pub fn picture_sha256(bytes: &[u8]) -> String {
    crate::lower_hex(&Sha256::digest(bytes))
}

/// The box and the head point of one instance, in pixels of its mask set's picture: `left` and
/// `top` inclusive, `right` and `bottom` exclusive; the head point is the mean column of the
/// mask's topmost rows, on its top row (just above the crown for a person).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstanceGeometry {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub head_x: f32,
    pub head_y: u32,
}

/// The geometry of `mask` in `set`, or `None` for an empty or unreadable mask. `head_rows` is how
/// many of the topmost rows are averaged for the head point.
pub fn instance_geometry(
    set: &AnalysisMaskSet,
    mask: &AnalysisMask,
    head_rows: u32,
) -> Option<InstanceGeometry> {
    let rows = df_providers::MaskRows::from_runs(
        set.width_px,
        set.height_px,
        mask.runs
            .iter()
            .map(|run| df_providers::MaskRun {
                start: run.start,
                length: run.length,
            })
            .collect(),
    )
    .ok()?;
    let bounds = rows.bounds()?;
    let (head_x, head_y) = rows.top_centre(head_rows)?;
    Some(InstanceGeometry {
        left: bounds.left,
        top: bounds.top,
        right: bounds.right,
        bottom: bounds.bottom,
        head_x,
        head_y,
    })
}

/// The mask of one instance as a PNG data URL the size of its mask set's picture: white where the
/// instance is, fully transparent elsewhere, so a page can paint it with CSS `mask-image`. `None`
/// for a mask whose runs do not fit the picture.
pub fn mask_data_url(set: &AnalysisMaskSet, mask: &AnalysisMask) -> Option<String> {
    use base64::Engine as _;
    let total = u64::from(set.width_px) * u64::from(set.height_px);
    let mut pixels = vec![0u8; usize::try_from(total).ok()?.checked_mul(2)?];
    for run in &mask.runs {
        let start = u64::from(run.start);
        let end = start.checked_add(u64::from(run.length))?;
        if end > total {
            return None;
        }
        for index in start..end {
            let at = usize::try_from(index).ok()? * 2;
            pixels[at] = 255;
            pixels[at + 1] = 255;
        }
    }
    let image = image::GrayAlphaImage::from_raw(set.width_px, set.height_px, pixels)?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLumaA8(image)
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(out.into_inner())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(model: &str, credential: &str) -> SegmentConfig {
        SegmentConfig {
            endpoint: "http://127.0.0.1:9".to_owned(),
            model: model.to_owned(),
            credential: credential.to_owned(),
            timeout: Duration::from_secs(5),
        }
    }

    fn request(source: Vec<u8>, maximum_masks: u32) -> SegmentRequest {
        SegmentRequest {
            source,
            prompt: SegmentationPrompt::Person,
            maximum_masks,
        }
    }

    fn png() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([20, 40, 60, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[tokio::test]
    async fn a_mask_count_outside_the_contract_is_refused_before_any_call() {
        for count in [0, MAXIMUM_MASKS + 1] {
            let result = segment(&config(DEFAULT_MODEL, "key"), request(png(), count)).await;
            assert_eq!(result, Err(SegmentError::Configuration));
        }
    }

    #[tokio::test]
    async fn an_empty_key_is_refused_before_any_call() {
        let result = segment(&config(DEFAULT_MODEL, ""), request(png(), 8)).await;
        assert_eq!(result, Err(SegmentError::Configuration));
    }

    #[tokio::test]
    async fn bytes_that_are_not_a_picture_are_refused_before_any_call() {
        let result = segment(
            &config(DEFAULT_MODEL, "key"),
            request(b"not a picture".to_vec(), 8),
        )
        .await;
        assert_eq!(result, Err(SegmentError::Picture));
    }

    #[test]
    fn the_scan_future_can_move_between_worker_threads() {
        fn assert_send<T: Send>(_: &T) {}
        let config = config(DEFAULT_MODEL, "key");
        assert_send(&segment(&config, request(png(), 8)));
    }

    #[test]
    fn an_instance_has_its_box_and_the_middle_of_its_top_rows() {
        // A 10 by 6 picture; the mask covers columns 3..7 of row 1 and columns 2..8 of row 2.
        let set = AnalysisMaskSet {
            prompt: SegmentationPrompt::Person,
            width_px: 10,
            height_px: 6,
            instances: Vec::new(),
        };
        let mask = AnalysisMask {
            index: 0,
            score: AnalysisUnit::new(0.9).unwrap(),
            runs: vec![
                AnalysisMaskRun {
                    start: 13,
                    length: 4,
                },
                AnalysisMaskRun {
                    start: 22,
                    length: 6,
                },
            ],
        };
        let geometry = instance_geometry(&set, &mask, 1).unwrap();
        assert_eq!(
            (geometry.left, geometry.top, geometry.right, geometry.bottom),
            (2, 1, 8, 3)
        );
        assert_eq!(geometry.head_y, 1);
        assert!((geometry.head_x - 5.0).abs() < 0.01);
    }

    #[test]
    fn a_mask_becomes_a_transparent_png_of_the_picture_size() {
        use base64::Engine as _;
        let set = AnalysisMaskSet {
            prompt: SegmentationPrompt::Person,
            width_px: 10,
            height_px: 6,
            instances: Vec::new(),
        };
        let mask = AnalysisMask {
            index: 0,
            score: AnalysisUnit::new(0.9).unwrap(),
            runs: vec![AnalysisMaskRun {
                start: 13,
                length: 4,
            }],
        };
        let url = mask_data_url(&set, &mask).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(url.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        let picture = image::load_from_memory(&bytes).unwrap().to_luma_alpha8();
        assert_eq!(picture.dimensions(), (10, 6));
        assert_eq!(picture.get_pixel(3, 1).0, [255, 255]);
        assert_eq!(picture.get_pixel(7, 1).0[1], 0);
        assert_eq!(picture.get_pixel(0, 0).0[1], 0);
        let outside = AnalysisMask {
            runs: vec![AnalysisMaskRun {
                start: 58,
                length: 4,
            }],
            ..mask
        };
        assert_eq!(mask_data_url(&set, &outside), None);
    }

    #[test]
    fn the_config_never_prints_its_key() {
        let text = format!("{:?}", config(DEFAULT_MODEL, "very-secret-key"));
        assert!(!text.contains("very-secret-key"));
    }
}
