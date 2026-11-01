//! One admitted SAM submit and a separately resumable GET-only queue exchange.
use super::{chat::verify, pictures};
use crate::backends::api::{check_credential, check_endpoint_url};
use crate::{
    config::{AnalysisPreparationLimits, SegmentationProvider, SegmentationRoute},
    request::SourceRef,
    runtime::RuntimeError,
    service::analysis_types::*,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use df_providers::{
    HttpMethod, HttpRequest, NativeHttp, NativeHttpRefusal, NativeHttpResponse, ProviderLimits,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
const MAXIMUM_CHECKPOINT_BYTES: usize = 24_576;
const MAXIMUM_LOCATOR_BYTES: usize = 8192;
const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Authorized source and one noun prompt; each input represents a separately admitted call.
pub(crate) struct SegmentationInput<'a> {
    pub(crate) source: &'a [u8],
    pub(crate) source_pin: &'a SourceRef,
    pub(crate) prompt: SegmentationPrompt,
    pub(crate) preparation_recipe: SegmentationPreparation,
    pub(crate) maximum_masks: u32,
    pub(crate) route: &'a SegmentationRoute,
    pub(crate) preparation: &'a AnalysisPreparationLimits,
    pub(crate) max_decoded_bytes: u64,
    pub(crate) timeout: Duration,
    pub(crate) maximum_response_bytes: usize,
}
/// Validated supplier correlation bound to source, recipe and configured credential destination.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct SegmentationCheckpoint {
    base: String,
    model: String,
    request_id: String,
    status_url: String,
    result_url: String,
    prompt: SegmentationPrompt,
    preparation_recipe: SegmentationPreparation,
    width_px: u32,
    height_px: u32,
    maximum_masks: u32,
    digest: [u8; 32],
}
impl std::fmt::Debug for SegmentationCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentationCheckpoint")
            .field("prompt", &self.prompt)
            .field("width_px", &self.width_px)
            .field("height_px", &self.height_px)
            .finish_non_exhaustive()
    }
}
impl SegmentationCheckpoint {
    pub(crate) fn encode(&self) -> String {
        json!({
            "kind": "fal-sam-rle-v1",
            "preparation": match self.preparation_recipe {
                SegmentationPreparation::FigureMask => "figure_mask",
                SegmentationPreparation::ScenePlate => "scene_plate",
            },
            "base": self.base,
            "model": self.model,
            "request_id": self.request_id,
            "status_url": self.status_url,
            "result_url": self.result_url,
            "prompt": self.prompt.label(),
            "width_px": self.width_px,
            "height_px": self.height_px,
            "maximum_masks": self.maximum_masks,
            "digest": self.digest,
        })
        .to_string()
    }
    pub(crate) fn decode(encoded: &str) -> Result<Self, RuntimeError> {
        let value = parse(encoded.as_bytes(), MAXIMUM_CHECKPOINT_BYTES)?;
        let fields = value
            .as_object()
            .filter(|map| map.len() == 12)
            .ok_or(RuntimeError::InvalidRecord)?;
        if fields.get("kind").and_then(Value::as_str) != Some("fal-sam-rle-v1") {
            return Err(RuntimeError::InvalidRecord);
        }
        let string = |field| {
            fields
                .get(field)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(RuntimeError::InvalidRecord)
        };
        let number = |field| {
            fields
                .get(field)
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or(RuntimeError::InvalidRecord)
        };
        let prompt = match fields.get("prompt").and_then(Value::as_str) {
            Some("person") => SegmentationPrompt::Person,
            Some("head") => SegmentationPrompt::Head,
            _ => return Err(RuntimeError::InvalidRecord),
        };
        let preparation_recipe = match fields.get("preparation").and_then(Value::as_str) {
            Some("figure_mask") => SegmentationPreparation::FigureMask,
            Some("scene_plate") => SegmentationPreparation::ScenePlate,
            _ => return Err(RuntimeError::InvalidRecord),
        };
        let bytes = fields
            .get("digest")
            .and_then(Value::as_array)
            .filter(|bytes| bytes.len() == 32)
            .ok_or(RuntimeError::InvalidRecord)?;
        let mut digest = [0; 32];
        for (slot, byte) in digest.iter_mut().zip(bytes) {
            *slot = byte
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or(RuntimeError::InvalidRecord)?;
        }
        let checkpoint = Self {
            base: string("base")?,
            model: string("model")?,
            request_id: string("request_id")?,
            status_url: string("status_url")?,
            result_url: string("result_url")?,
            prompt,
            preparation_recipe,
            width_px: number("width_px")?,
            height_px: number("height_px")?,
            maximum_masks: number("maximum_masks")?,
            digest,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }
    fn validate(&self) -> Result<(), RuntimeError> {
        if self.request_id.is_empty()
            || self.request_id.len() > 256
            || !self
                .request_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            || self.width_px == 0
            || self.height_px == 0
            || !(1..=32).contains(&self.maximum_masks)
        {
            return Err(RuntimeError::InvalidRecord);
        }
        check_endpoint_url(&self.base).map_err(|_| RuntimeError::InvalidConfiguration)?;
        check_model_path(&self.model)?;
        let status = confined(
            &self.base,
            &self.model,
            &self.status_url,
            &self.request_id,
            true,
        )?;
        let result = confined(
            &self.base,
            &self.model,
            &self.result_url,
            &self.request_id,
            false,
        )?;
        if status != result {
            return Err(RuntimeError::InvalidRecord);
        }
        Ok(())
    }
}
/// Prepared SAM body with a consumable submit and a separate GET-only resume path.
pub(crate) struct PreparedSegmentation {
    http: NativeHttp,
    submit_request: Option<HttpRequest>,
    credential: String,
    expires: Instant,
    base: String,
    model: String,
    prompt: SegmentationPrompt,
    preparation_recipe: SegmentationPreparation,
    digest: [u8; 32],
    pub(crate) width_px: u32,
    pub(crate) height_px: u32,
    maximum_masks: u32,
    maximum_response_bytes: usize,
    pub(crate) text_bytes: u64,
    pub(crate) body_bytes: u64,
    pub(crate) picture_count: u64,
}
impl std::fmt::Debug for PreparedSegmentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedSegmentation")
            .field("prompt", &self.prompt)
            .field("width_px", &self.width_px)
            .field("height_px", &self.height_px)
            .field("body_bytes", &self.body_bytes)
            .finish_non_exhaustive()
    }
}
impl PreparedSegmentation {
    pub(crate) fn prepare(
        input: SegmentationInput<'_>,
        credential: &str,
    ) -> Result<Self, RuntimeError> {
        let expires = Instant::now()
            .checked_add(input.timeout)
            .ok_or(RuntimeError::InvalidConfiguration)?;
        if input.timeout.is_zero() {
            return Err(RuntimeError::Expired);
        }
        check_credential(credential).map_err(|_| RuntimeError::InvalidConfiguration)?;
        let allow_http = check_endpoint_url(input.route.endpoint.as_str())
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
        check_model_path(input.route.model.as_str())?;
        if input.route.provider != SegmentationProvider::Fal
            || input.maximum_response_bytes == 0
            || input.preparation.max_pixels == 0
            || input.preparation.max_picture_bytes == 0
            || input.preparation.max_request_bytes == 0
            || input.max_decoded_bytes == 0
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        if !(1..=32).contains(&input.maximum_masks) {
            return Err(RuntimeError::InvalidRecord);
        }
        verify(input.source_pin, input.source)?;
        let (jpeg, width_px, height_px) = match input.preparation_recipe {
            SegmentationPreparation::ScenePlate => pictures::plate(
                input.source,
                input.preparation,
                input.max_decoded_bytes,
                input.source.len() as u64,
            )?,
            SegmentationPreparation::FigureMask => pictures::figure(
                input.source,
                input.preparation,
                input.max_decoded_bytes,
                input.source.len() as u64,
            )?,
        };
        // Result masks use the shared, bounded geometry representation. Refuse an
        // unrepresentable effective recipe size before admission or supplier dispatch.
        df_providers::MaskRows::from_runs(width_px, height_px, Vec::new())
            .map_err(|_| RuntimeError::UnsupportedCapability)?;
        let encoded = jpeg
            .len()
            .div_ceil(3)
            .checked_mul(4)
            .ok_or(RuntimeError::InvalidRecord)?;
        if encoded as u64 > input.preparation.max_request_bytes {
            return Err(RuntimeError::InvalidRecord);
        }
        let body = serde_json::to_vec(&json!({
            "image_url": format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg)),
            "prompt": input.prompt.label(),
            "include_scores": true,
            "include_boxes": true,
            "return_multiple_masks": true,
            "max_masks": input.maximum_masks,
        }))
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let body_bytes = body.len() as u64;
        if body_bytes > input.preparation.max_request_bytes {
            return Err(RuntimeError::InvalidRecord);
        }
        let text_bytes = body
            .len()
            .checked_sub(encoded)
            .ok_or(RuntimeError::InvalidRecord)? as u64;
        let base = input
            .route
            .endpoint
            .as_str()
            .trim_end_matches('/')
            .to_owned();
        let model = input.route.model.as_str().to_owned();
        let http = NativeHttp::new(
            ProviderLimits::new(
                input.timeout,
                usize::try_from(input.preparation.max_request_bytes)
                    .map_err(|_| RuntimeError::UnsupportedCapability)?,
                input.maximum_response_bytes,
            )
            .build(),
            allow_http,
        )
        .map_err(|reason| {
            if reason == NativeHttpRefusal::InvalidLimits {
                RuntimeError::UnsupportedCapability
            } else {
                RuntimeError::InvalidConfiguration
            }
        })?;
        let submit_request = Some(HttpRequest::fal_native(
            HttpMethod::Post,
            &format!("{base}/{model}"),
            credential,
            body,
        ));
        Ok(Self {
            http,
            submit_request,
            credential: credential.to_owned(),
            expires,
            base,
            model,
            prompt: input.prompt,
            preparation_recipe: input.preparation_recipe,
            digest: *input.source_pin.digest.as_bytes(),
            width_px,
            height_px,
            maximum_masks: input.maximum_masks,
            maximum_response_bytes: input.maximum_response_bytes,
            text_bytes,
            body_bytes,
            picture_count: 1,
        })
    }
    fn remaining(&self) -> Result<Duration, RuntimeError> {
        let remaining = self.expires.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(RuntimeError::Expired)
        } else {
            Ok(remaining)
        }
    }
    pub(crate) async fn submit(&mut self) -> Result<NativeHttpResponse, RuntimeError> {
        let remaining = self.remaining()?;
        let request = self
            .submit_request
            .take()
            .ok_or(RuntimeError::InvalidRecord)?;
        self.http
            .execute(request, remaining, std::future::pending())
            .await
            .map_err(|failure| transport(failure.reason))
    }
    pub(crate) fn checkpoint(
        &self,
        response: &NativeHttpResponse,
    ) -> Result<SegmentationCheckpoint, RuntimeError> {
        self.checkpoint_parts(response.status, &response.body)
    }
    /// Revalidates actual retained submit status and bytes without inventing HTTP metadata.
    pub(crate) fn checkpoint_parts(
        &self,
        status: u16,
        body: &[u8],
    ) -> Result<SegmentationCheckpoint, RuntimeError> {
        if !matches!(status, 200 | 202) {
            return Err(RuntimeError::ProviderUnknown);
        }
        let value = parse(body, self.maximum_response_bytes)?;
        let field = |field| {
            value
                .get(field)
                .and_then(Value::as_str)
                .filter(|text| text.len() <= MAXIMUM_LOCATOR_BYTES)
                .map(str::to_owned)
                .ok_or(RuntimeError::InvalidRecord)
        };
        let checkpoint = SegmentationCheckpoint {
            base: self.base.clone(),
            model: self.model.clone(),
            request_id: field("request_id")?,
            status_url: field("status_url")?,
            result_url: field("response_url")?,
            prompt: self.prompt,
            preparation_recipe: self.preparation_recipe,
            width_px: self.width_px,
            height_px: self.height_px,
            maximum_masks: self.maximum_masks,
            digest: self.digest,
        };
        self.bind(&checkpoint)?;
        Ok(checkpoint)
    }
    fn bind(&self, checkpoint: &SegmentationCheckpoint) -> Result<(), RuntimeError> {
        checkpoint.validate()?;
        if checkpoint.base != self.base
            || checkpoint.model != self.model
            || checkpoint.prompt != self.prompt
            || checkpoint.preparation_recipe != self.preparation_recipe
            || checkpoint.width_px != self.width_px
            || checkpoint.height_px != self.height_px
            || checkpoint.maximum_masks != self.maximum_masks
            || checkpoint.digest != self.digest
        {
            return Err(RuntimeError::InvalidRecord);
        }
        Ok(())
    }
    pub(crate) async fn resume(
        &self,
        checkpoint: &SegmentationCheckpoint,
    ) -> Result<NativeHttpResponse, RuntimeError> {
        self.bind(checkpoint)?;
        loop {
            let status = self
                .http
                .execute(
                    HttpRequest::fal_native(
                        HttpMethod::Get,
                        &checkpoint.status_url,
                        &self.credential,
                        Vec::new(),
                    ),
                    self.remaining()?,
                    std::future::pending(),
                )
                .await
                .map_err(|failure| transport(failure.reason))?;
            if !matches!(status.status, 200 | 202) {
                return Ok(status);
            }
            let Ok(value) = parse(&status.body, self.maximum_response_bytes) else {
                return Ok(status);
            };
            if value.get("error").is_some_and(|error| !error.is_null()) {
                return Ok(status);
            }
            match value.get("status").and_then(Value::as_str) {
                Some("COMPLETED") => {
                    return self
                        .http
                        .execute(
                            HttpRequest::fal_native(
                                HttpMethod::Get,
                                &checkpoint.result_url,
                                &self.credential,
                                Vec::new(),
                            ),
                            self.remaining()?,
                            std::future::pending(),
                        )
                        .await
                        .map_err(|failure| transport(failure.reason));
                }
                Some("IN_QUEUE" | "IN_PROGRESS") => {
                    tokio::time::sleep(POLL_INTERVAL.min(self.remaining()?)).await
                }
                _ => return Ok(status),
            }
        }
    }
}
fn transport(reason: NativeHttpRefusal) -> RuntimeError {
    match reason {
        NativeHttpRefusal::Deadline => RuntimeError::Expired,
        NativeHttpRefusal::Cancelled => RuntimeError::Cancelled,
        NativeHttpRefusal::ProviderDisabled => RuntimeError::Disabled,
        _ => RuntimeError::ProviderUnknown,
    }
}
fn parse(bytes: &[u8], maximum: usize) -> Result<Value, RuntimeError> {
    df_providers::parse_unique_json(
        bytes,
        df_providers::TextSchemaLimits {
            maximum_schema_bytes: maximum,
            maximum_depth: 16,
            maximum_nodes: 262_144,
        },
    )
    .map_err(|_| RuntimeError::InvalidRecord)
}
fn check_model_path(model: &str) -> Result<(), RuntimeError> {
    let typed =
        crate::config::ModelName::new(model).map_err(|_| RuntimeError::InvalidConfiguration)?;
    // Decode revalidates the same typed grammar before relative-path confinement.
    if typed.as_str() != model
        || model
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(RuntimeError::InvalidConfiguration);
    }
    Ok(())
}
fn confined(
    base: &str,
    model: &str,
    locator: &str,
    id: &str,
    status: bool,
) -> Result<String, RuntimeError> {
    if locator.len() > MAXIMUM_LOCATOR_BYTES || locator.contains(['#', '?', '\\']) {
        return Err(RuntimeError::InvalidRecord);
    }
    let base = url::Url::parse(base).map_err(|_| RuntimeError::InvalidRecord)?;
    let target = url::Url::parse(locator).map_err(|_| RuntimeError::InvalidRecord)?;
    if target.scheme() != base.scheme()
        || target.host() != base.host()
        || target.port_or_known_default() != base.port_or_known_default()
        || !target.username().is_empty()
        || target.password().is_some()
    {
        return Err(RuntimeError::InvalidRecord);
    }
    // Inspect the original path before URL normalization can erase traversal segments.
    let raw = locator
        .split_once("://")
        .and_then(|(_, tail)| tail.find('/').map(|index| &tail[index..]))
        .ok_or(RuntimeError::InvalidRecord)?;
    let prefix = format!("{}/", base.path().trim_end_matches('/'));
    let tail = raw
        .strip_prefix(&prefix)
        .ok_or(RuntimeError::InvalidRecord)?;
    let (application, rest) = tail
        .rsplit_once("/requests/")
        .ok_or(RuntimeError::InvalidRecord)?;
    if application
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == ".." || part.contains('%'))
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let base_model = model.split('/').take(2).collect::<Vec<_>>().join("/");
    if application != model && application != base_model {
        return Err(RuntimeError::InvalidRecord);
    }
    let actual = if status {
        rest.strip_suffix("/status")
    } else {
        Some(rest.strip_suffix("/response").unwrap_or(rest))
    };
    if actual != Some(id) {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(application.to_owned())
}
/// Converts legacy one-based RLE to validated neutral zero-based runs at submitted dimensions.
pub(crate) fn parse_segmentation(
    value: &Value,
    prompt: SegmentationPrompt,
    width_px: u32,
    height_px: u32,
    maximum_masks: u32,
) -> Result<AnalysisMaskSet, RuntimeError> {
    if width_px == 0 || height_px == 0 || !(1..=32).contains(&maximum_masks) {
        return Err(RuntimeError::InvalidRecord);
    }
    let masks = value
        .get("rle")
        .and_then(Value::as_array)
        .ok_or(RuntimeError::InvalidRecord)?;
    let scores = value
        .get("scores")
        .and_then(Value::as_array)
        .ok_or(RuntimeError::InvalidRecord)?;
    if masks.len() > maximum_masks as usize || scores.len() != masks.len() {
        return Err(RuntimeError::InvalidRecord);
    }
    let metadata = value.get("metadata").and_then(Value::as_array);
    if metadata.is_some_and(|rows| rows.len() != masks.len()) {
        return Err(RuntimeError::InvalidRecord);
    }
    let mut instances = Vec::with_capacity(masks.len());
    for (position, (mask, score)) in masks.iter().zip(scores).enumerate() {
        let score = score
            .as_f64()
            .filter(|n| (0.0..=1.0).contains(n))
            .ok_or(RuntimeError::InvalidRecord)? as f32;
        let rows = df_providers::MaskRows::parse(
            mask.as_str().ok_or(RuntimeError::InvalidRecord)?,
            width_px,
            height_px,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        if rows.is_empty() {
            continue;
        }
        let index = match metadata
            .and_then(|rows| rows.get(position))
            .and_then(|row| row.get("index"))
            .and_then(Value::as_u64)
        {
            Some(index) => u32::try_from(index).map_err(|_| RuntimeError::InvalidRecord)?,
            None => u32::try_from(position).map_err(|_| RuntimeError::InvalidRecord)?,
        };
        instances.push(AnalysisMask {
            index,
            score: AnalysisUnit::new(score).map_err(|_| RuntimeError::InvalidRecord)?,
            runs: rows
                .runs()
                .iter()
                .map(|run| AnalysisMaskRun {
                    start: run.start,
                    length: run.length,
                })
                .collect(),
        });
    }
    let result = AnalysisMaskSet {
        prompt,
        width_px,
        height_px,
        instances,
    };
    result.validate().map_err(|_| RuntimeError::InvalidRecord)?;
    Ok(result)
}
