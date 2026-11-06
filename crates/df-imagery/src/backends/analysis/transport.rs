//! One prepared photographic inspection exchange, with no retry or settlement owner.

use super::photo;
use crate::backends::api::{check_credential, check_endpoint_url};
use crate::config::AnalysisPreparationLimits;
use crate::request::{ImageInspection, SourceRef};
use crate::runtime::RuntimeError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use df_providers::{HttpRequest, NativeHttp, NativeHttpCause, NativeHttpResponse, ProviderLimits};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Write},
    time::{Duration, Instant},
};

const MAXIMUM_INSTRUCTION_CHARACTERS: usize = 6000;
const MAXIMUM_VISION_WIDTH: u32 = 1024;

/// References retain canonical one-based order; the runtime authorizes every pin.
pub(crate) struct InspectionInput<'a> {
    pub(crate) spec: &'a ImageInspection,
    pub(crate) candidate: &'a [u8],
    pub(crate) references: &'a [(SourceRef, Vec<u8>)],
    pub(crate) timeout: Duration,
    pub(crate) maximum_response_bytes: usize,
    pub(crate) preparation: &'a AnalysisPreparationLimits,
    pub(crate) max_decoded_bytes: u64,
}

/// Exact bounded body and cost facts prepared before admission; execute sends once.
pub(crate) struct PreparedInspection {
    http: NativeHttp,
    request: HttpRequest,
    expires: Instant,
    pub(crate) body_bytes: u64,
    pub(crate) text_bytes: u64,
    pub(crate) picture_count: u64,
}

impl std::fmt::Debug for PreparedInspection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedInspection")
            .field("body_bytes", &self.body_bytes)
            .field("text_bytes", &self.text_bytes)
            .field("picture_count", &self.picture_count)
            .finish_non_exhaustive()
    }
}

impl PreparedInspection {
    /// CPU preparation must run under the caller's owned local-work permit.
    /// The caller verifies the route against trusted configuration before resolving its key.
    pub(crate) fn prepare(
        input: InspectionInput<'_>,
        credential: &str,
    ) -> Result<Self, RuntimeError> {
        let expires = Instant::now()
            .checked_add(input.timeout)
            .ok_or(RuntimeError::InvalidConfiguration)?;
        if input.timeout.is_zero() {
            return Err(RuntimeError::Expired);
        }
        check_credential(credential).map_err(|_| RuntimeError::InvalidConfiguration)?;
        let allow_http = check_endpoint_url(input.spec.route.endpoint.as_str())
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
        if input.spec.route.maximum_output_tokens == 0
            || input.maximum_response_bytes == 0
            || input.spec.contract_revision == 0
            || input.max_decoded_bytes == 0
            || input.preparation.max_pixels == 0
            || input.preparation.max_picture_bytes == 0
            || input.preparation.max_request_bytes == 0
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        let contract = photo::contract(input.spec).map_err(|_| RuntimeError::InvalidRecord)?;
        if contract.instruction.trim().is_empty()
            || contract.instruction.chars().count() > MAXIMUM_INSTRUCTION_CHARACTERS
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let text_bytes = contract
            .instruction
            .len()
            .checked_add(contract.schema.to_string().len())
            .and_then(|count| u64::try_from(count).ok())
            .ok_or(RuntimeError::InvalidRecord)?;
        let retained_bytes = input.references.iter().try_fold(
            u64::try_from(input.candidate.len()).map_err(|_| RuntimeError::InvalidRecord)?,
            |total, (_, bytes)| {
                total
                    .checked_add(
                        u64::try_from(bytes.len()).map_err(|_| RuntimeError::InvalidRecord)?,
                    )
                    .filter(|count| *count <= input.max_decoded_bytes)
                    .ok_or(RuntimeError::InvalidRecord)
            },
        )?;
        let candidate = prepare_picture(
            input.candidate,
            input.preparation,
            input.max_decoded_bytes,
            retained_bytes,
        )?;
        let reference = if contract.needs_reference {
            let index = input
                .spec
                .party_reference
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| usize::try_from(index).ok())
                .ok_or(RuntimeError::InvalidRecord)?;
            let (pin, bytes) = input
                .references
                .get(index)
                .ok_or(RuntimeError::InvalidRecord)?;
            let digest: [u8; 32] = Sha256::digest(bytes).into();
            if &digest != pin.digest.as_bytes() {
                return Err(RuntimeError::SourceDenied);
            }
            let retained = retained_bytes
                .checked_add(candidate.capacity() as u64)
                .ok_or(RuntimeError::InvalidRecord)?;
            Some(prepare_picture(
                bytes,
                input.preparation,
                input.max_decoded_bytes,
                retained,
            )?)
        } else {
            None
        };
        let mut content = vec![json!({
            "type":"text",
            "text":contract.instruction
        })];
        let mut pictures = vec![("candidate-1", candidate)];
        if let Some(reference) = reference {
            pictures.push(("reference-sheet", reference));
        }
        let picture_count =
            u64::try_from(pictures.len()).map_err(|_| RuntimeError::InvalidRecord)?;
        let encoded_bound = pictures.iter().try_fold(0u64, |total, (_, picture)| {
            (picture.len() as u64)
                .checked_add(2)
                .and_then(|size| size.checked_div(3))
                .and_then(|size| size.checked_mul(4))
                .and_then(|size| size.checked_add(23))
                .and_then(|size| total.checked_add(size))
                .ok_or(RuntimeError::InvalidRecord)
        })?;
        if encoded_bound > input.preparation.max_request_bytes {
            return Err(RuntimeError::InvalidRecord);
        }
        for (label, picture) in pictures {
            content.push(json!({
                "type":"text",
                "text":format!("Picture {label}:")
            }));
            content.push(json!({
                "type":"image_url",
                "image_url":{
                    "url":format!("data:image/jpeg;base64,{}",
                    STANDARD.encode(picture)),
                    "detail":contract.image_detail
                }
            }));
        }
        let body = serde_json::to_vec(&json!({
            "model":input.spec.route.model.as_str(),
            "max_completion_tokens":input.spec.route.maximum_output_tokens,
            "messages":[
                {
                    "role":"user",
                    "content":content
                }
            ],
            "response_format":{
                "type":"json_schema",
                "json_schema":{
                    "name":contract.schema_name,
                    "strict":true,
                    "schema":contract.schema
                }
            },

        }))
        .map_err(|_| RuntimeError::InvalidRecord)?;
        if body.len() as u64 > input.preparation.max_request_bytes {
            return Err(RuntimeError::InvalidRecord);
        }
        let body_bytes = u64::try_from(body.len()).map_err(|_| RuntimeError::InvalidRecord)?;
        let http = NativeHttp::new(
            ProviderLimits::new(
                input.timeout,
                usize::try_from(input.preparation.max_request_bytes).unwrap_or(usize::MAX),
                input.maximum_response_bytes,
            )
            .build(),
            allow_http,
        )
        .map_err(|_| RuntimeError::InvalidConfiguration)?;
        let request = HttpRequest::bearer_native(
            input.spec.route.endpoint.as_str(),
            credential,
            "application/json".to_owned(),
            body,
        );
        Ok(Self {
            http,
            request,
            expires,
            body_bytes,
            text_bytes,
            picture_count,
        })
    }

    /// Returns every HTTP status and its raw bounded body before any quality interpretation.
    pub(crate) async fn execute(self) -> Result<NativeHttpResponse, RuntimeError> {
        let remaining = self.expires.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(RuntimeError::Expired);
        }
        self.http
            .execute(self.request, remaining, std::future::pending())
            .await
            .map_err(|failure| match failure.cause {
                NativeHttpCause::Deadline => RuntimeError::Expired,
                NativeHttpCause::Cancelled => RuntimeError::Cancelled,
                NativeHttpCause::ProviderDisabled => RuntimeError::Disabled,
                _ => RuntimeError::ProviderUnknown,
            })
    }
}

struct PictureBuffer {
    bytes: Vec<u8>,
    maximum: usize,
}
impl Write for PictureBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let needed = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|needed| *needed <= self.maximum)
            .ok_or_else(|| std::io::Error::other("bounded inspection picture"))?;
        if needed > self.bytes.capacity() {
            let target = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(needed)
                .min(self.maximum);
            self.bytes
                .try_reserve_exact(target - self.bytes.len())
                .map_err(|_| std::io::Error::other("inspection picture allocation"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Header-only admission covers simultaneously retained pixel buffers. Decoder limits also
/// receive the remaining allocation allowance; their codec-specific scratch bound is best effort.
fn picture_budget(
    width: u32,
    height: u32,
    maximum_pixels: u64,
    memory: u64,
    retained: u64,
) -> Result<(u32, u32, u64), RuntimeError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .filter(|pixels| *pixels > 0 && *pixels <= maximum_pixels)
        .ok_or(RuntimeError::InvalidRecord)?;
    let width_px = width.min(MAXIMUM_VISION_WIDTH);
    let height_px = u32::try_from(u64::from(height) * u64::from(width_px) / u64::from(width))
        .map_err(|_| RuntimeError::InvalidRecord)?
        .max(1);
    // Decode may retain 16-bit RGBA plus its 8-bit conversion (12 bytes/pixel).
    let decode = pixels.checked_mul(12).ok_or(RuntimeError::InvalidRecord)?;
    // Triangle resize retains RGB source, an f32 intermediate, and RGB destination.
    let resize = if (width_px, height_px) == (width, height) {
        pixels.checked_mul(7)
    } else {
        pixels
            .checked_mul(3)
            .and_then(|n| n.checked_add(u64::from(width_px) * u64::from(height) * 12))
            .and_then(|n| n.checked_add(u64::from(width_px) * u64::from(height_px) * 3))
    }
    .ok_or(RuntimeError::InvalidRecord)?;
    if retained
        .checked_add(decode.max(resize))
        .is_none_or(|needed| needed > memory)
    {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok((width_px, height_px, pixels))
}

fn prepare_picture(
    bytes: &[u8],
    preparation: &AnalysisPreparationLimits,
    memory: u64,
    retained: u64,
) -> Result<Vec<u8>, RuntimeError> {
    if bytes.is_empty() || retained > memory {
        return Err(RuntimeError::InvalidRecord);
    }
    let format = match image::guess_format(bytes).map_err(|_| RuntimeError::InvalidRecord)? {
        format
        @ (image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP) => format,
        _ => return Err(RuntimeError::UnsupportedCapability),
    };
    let (width, height) = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let (width_px, height_px, pixels) =
        picture_budget(width, height, preparation.max_pixels, memory, retained)?;
    let mut decoder_limits = image::Limits::no_limits();
    decoder_limits.max_image_width = Some(width);
    decoder_limits.max_image_height = Some(height);
    decoder_limits.max_alloc = Some(memory - retained - pixels * 4);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(decoder_limits);
    let rgba = reader
        .decode()
        .map_err(|_| RuntimeError::InvalidRecord)?
        .into_rgba8();
    let mut rgb = image::RgbImage::new(width, height);
    for (source, target) in rgba.pixels().zip(rgb.pixels_mut()) {
        let alpha = u16::from(source[3]);
        for (channel, target) in target.0.iter_mut().enumerate() {
            let mixed = (u16::from(source[channel]) * alpha + 128 * (255 - alpha)) / 255;
            *target = u8::try_from(mixed).map_err(|_| RuntimeError::InvalidRecord)?;
        }
    }
    drop(rgba);
    let rgb = if width_px == width && height_px == height {
        rgb
    } else {
        let resized = image::imageops::resize(
            &rgb,
            width_px,
            height_px,
            image::imageops::FilterType::Triangle,
        );
        drop(rgb);
        resized
    };
    let available = memory
        .checked_sub(retained)
        .and_then(|n| n.checked_sub(rgb.len() as u64))
        .ok_or(RuntimeError::InvalidRecord)?;
    // Growth can temporarily retain old and new allocations; reserve half the available bytes.
    let maximum =
        usize::try_from(preparation.max_picture_bytes.min(available / 2)).unwrap_or(usize::MAX);
    let mut encoded = PictureBuffer {
        bytes: Vec::new(),
        maximum,
    };
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 80)
        .encode_image(&image::DynamicImage::ImageRgb8(rgb))
        .map_err(|_| RuntimeError::InvalidRecord)?;
    Ok(encoded.bytes)
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
