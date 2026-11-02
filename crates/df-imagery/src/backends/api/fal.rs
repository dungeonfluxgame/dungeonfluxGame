//! Private fal backend: FLUX generation plus paid alpha extraction.
//!
//! Relocated without behavior change: text-only requests use FLUX 1.1 Pro
//! Ultra in raw mode, reference edits use multi-reference FLUX Kontext Max,
//! and transparent cut-outs pass through BiRefNet. Every call asks for
//! `sync_mode`, so images arrive as data URIs with no media-host fetch.
//!
//! Generation and alpha removal are explicit separately metered stages. An
//! opaque edit performs one paid stage; a transparent edit performs two, and
//! the outcome carries both receipts so no second paid call is hidden.

#[cfg(test)]
use std::net::SocketAddr;
use std::{collections::HashSet, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

#[cfg(test)]
use super::codec::{jpeg_size, webp_facts};
use super::{
    ApiError, ApiExchange, ApiFailure, DeclineContract, ImageBackground, ImageEdit, ImageFormat,
    ImageModeration, OperationalBounds, check_credential, check_credentialed_origin,
    check_endpoint_url, check_model_slug,
};
#[cfg(test)]
use super::{GeneratedImage, ImageUsage};
#[cfg(test)]
use df_providers::{EncodedMediaFacts, inspect_png_off_worker};
use df_providers::{
    HttpMethod, HttpRequest, NativeHttp, NativeHttpLimits, NativeHttpRefusal, TextSchemaLimits,
    parse_unique_json,
};

pub(crate) const QUEUE_ORIGIN: &str = "https://queue.fal.run";
const GENERATION_CHECKPOINT_KIND: &str = "fal-generation-v2";
const ALPHA_CHECKPOINT_KIND: &str = "fal-alpha-v2";
/// Maximum journal envelope, including kind and field separators.
const MAXIMUM_CHECKPOINT_BYTES: usize = 24_576;
/// Maximum origin or origin-relative polling locator retained in a checkpoint.
const MAXIMUM_LOCATOR_BYTES: usize = 8192;
pub(crate) const FLUX_ULTRA: &str = "fal-ai/flux-pro/v1.1-ultra";
pub(crate) const FLUX_KONTEXT_MULTI: &str = "fal-ai/flux-pro/kontext/max/multi";
pub(crate) const BACKGROUND_REMOVAL: &str = "fal-ai/birefnet/v2";
const ASPECT_RATIOS: [(&str, f64); 9] = [
    ("21:9", 21.0 / 9.0),
    ("16:9", 16.0 / 9.0),
    ("3:2", 3.0 / 2.0),
    ("4:3", 4.0 / 3.0),
    ("1:1", 1.0),
    ("3:4", 3.0 / 4.0),
    ("2:3", 2.0 / 3.0),
    ("9:16", 9.0 / 16.0),
    ("9:21", 9.0 / 21.0),
];

/// Explicit origin and model selection. Origins are the credential destination
/// (`live`), a loopback fixture socket, or a validated operator base URL.
/// Model slugs are distinct configurable strings for schema-compatible
/// models; defaults preserve current behavior. URLs never appear in
/// diagnostics.
pub(crate) struct FalConfig {
    origin: String,
    allow_http: bool,
    generation_model: String,
    edit_model: String,
    alpha_model: String,
}

impl FalConfig {
    pub(crate) fn live() -> Self {
        Self {
            origin: QUEUE_ORIGIN.to_owned(),
            allow_http: false,
            generation_model: FLUX_ULTRA.to_owned(),
            edit_model: FLUX_KONTEXT_MULTI.to_owned(),
            alpha_model: BACKGROUND_REMOVAL.to_owned(),
        }
    }

    #[cfg(test)]
    pub(crate) fn loopback_fixture(address: SocketAddr) -> Result<Self, ApiError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(ApiError::InvalidRequest);
        }
        Ok(Self {
            origin: format!("http://{address}"),
            allow_http: true,
            generation_model: FLUX_ULTRA.to_owned(),
            edit_model: FLUX_KONTEXT_MULTI.to_owned(),
            alpha_model: BACKGROUND_REMOVAL.to_owned(),
        })
    }

    pub(crate) fn operator(origin_url: &str) -> Result<Self, ApiError> {
        let allow_http = check_credentialed_origin(origin_url, "queue.fal.run")?;
        Ok(Self {
            origin: origin_url.trim_end_matches('/').to_owned(),
            allow_http,
            generation_model: FLUX_ULTRA.to_owned(),
            edit_model: FLUX_KONTEXT_MULTI.to_owned(),
            alpha_model: BACKGROUND_REMOVAL.to_owned(),
        })
    }

    pub(crate) fn with_models(
        mut self,
        generation: &str,
        edit: &str,
        alpha: &str,
    ) -> Result<Self, ApiError> {
        check_model_slug(generation)?;
        check_model_slug(edit)?;
        check_model_slug(alpha)?;
        self.generation_model = generation.to_owned();
        self.edit_model = edit.to_owned();
        self.alpha_model = alpha.to_owned();
        Ok(self)
    }
}

impl std::fmt::Debug for FalConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FalConfig")
            .field("generation_model", &self.generation_model)
            .field("edit_model", &self.edit_model)
            .field("alpha_model", &self.alpha_model)
            .finish_non_exhaustive()
    }
}

/// Retained facts for one metered fal stage: endpoint slug, supplier request
/// id when known, and the HTTP status of the terminal exchange.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FalStageReceipt {
    pub(crate) endpoint: String,
    pub(crate) request_id: Option<String>,
    pub(crate) http_status: Option<u16>,
}

/// Receipt for the generation stage alone; bytes are actual decoded output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FalGenerationReceipt {
    pub(crate) stage: FalStageReceipt,
}

/// Receipt for the paid alpha-extraction stage alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FalAlphaReceipt {
    pub(crate) stage: FalStageReceipt,
}

/// Complete transparent-capable outcome with both metering identities.
/// `alpha` is `Some` exactly when the request asked for transparency.
#[cfg(test)]
pub(crate) struct FalEditOutcome {
    pub(crate) image: GeneratedImage,
    pub(crate) generation: FalGenerationReceipt,
    pub(crate) alpha: Option<FalAlphaReceipt>,
}

/// Durable submit checkpoint for one fal generation stage. The runner persists
/// this validated receipt before polling. `origin` binds the configured
/// credential destination, `endpoint` binds the selected model route, and
/// `request_id` is the bounded opaque supplier correlation; paths are
/// origin-relative locators revalidated on resume. Locators never appear in
/// diagnostics.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct FalGenerationCheckpoint {
    pub(crate) origin: String,
    pub(crate) endpoint: String,
    pub(crate) request_id: String,
    pub(crate) status_path: String,
    pub(crate) result_path: String,
}

/// Durable submit checkpoint for the paid alpha-extraction stage. Separate
/// from generation so the second paid call is never hidden inside it. The
/// closed kind plus the bound origin and alpha route keep a generation
/// receipt from resuming as alpha and vice versa.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct FalAlphaCheckpoint {
    pub(crate) origin: String,
    pub(crate) endpoint: String,
    pub(crate) request_id: String,
    pub(crate) status_path: String,
    pub(crate) result_path: String,
}

impl std::fmt::Debug for FalGenerationCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FalGenerationCheckpoint")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for FalAlphaCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FalAlphaCheckpoint")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// Versioned text encoding for the stage journal. Fields are length-checked
/// before clone and allocation; any mismatch is `InvalidRequest` and
/// performs no egress. The closed kind (`fal-generation-v2`,
/// `fal-alpha-v2`) keeps stages distinct so a generation receipt never
/// decodes as alpha.
impl FalGenerationCheckpoint {
    pub(crate) fn encode(&self) -> String {
        format!(
            "fal-generation-v2|{}|{}|{}|{}|{}",
            self.origin, self.endpoint, self.request_id, self.status_path, self.result_path
        )
    }

    pub(crate) fn decode(encoded: &str) -> Result<Self, ApiError> {
        if encoded.is_empty() || encoded.len() > MAXIMUM_CHECKPOINT_BYTES {
            return Err(ApiError::InvalidRequest);
        }
        let mut parts = encoded.split('|');
        match (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) {
            (
                Some("fal-generation-v2"),
                Some(origin),
                Some(endpoint),
                Some(request_id),
                Some(status_path),
                Some(result_path),
                None,
            ) => {
                check_checkpoint_fields(
                    GENERATION_CHECKPOINT_KIND,
                    origin,
                    endpoint,
                    request_id,
                    status_path,
                    result_path,
                )?;
                Ok(Self {
                    origin: origin.to_owned(),
                    endpoint: endpoint.to_owned(),
                    request_id: request_id.to_owned(),
                    status_path: status_path.to_owned(),
                    result_path: result_path.to_owned(),
                })
            }
            _ => Err(ApiError::InvalidRequest),
        }
    }
}

impl FalAlphaCheckpoint {
    pub(crate) fn encode(&self) -> String {
        format!(
            "fal-alpha-v2|{}|{}|{}|{}|{}",
            self.origin, self.endpoint, self.request_id, self.status_path, self.result_path
        )
    }

    pub(crate) fn decode(encoded: &str) -> Result<Self, ApiError> {
        if encoded.is_empty() || encoded.len() > MAXIMUM_CHECKPOINT_BYTES {
            return Err(ApiError::InvalidRequest);
        }
        let mut parts = encoded.split('|');
        match (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) {
            (
                Some("fal-alpha-v2"),
                Some(origin),
                Some(endpoint),
                Some(request_id),
                Some(status_path),
                Some(result_path),
                None,
            ) => {
                check_checkpoint_fields(
                    ALPHA_CHECKPOINT_KIND,
                    origin,
                    endpoint,
                    request_id,
                    status_path,
                    result_path,
                )?;
                Ok(Self {
                    origin: origin.to_owned(),
                    endpoint: endpoint.to_owned(),
                    request_id: request_id.to_owned(),
                    status_path: status_path.to_owned(),
                    result_path: result_path.to_owned(),
                })
            }
            _ => Err(ApiError::InvalidRequest),
        }
    }
}

/// Crate-private executor; owned by the later service worker.
pub(crate) struct FalExecutor {
    origin: String,
    credential: String,
    http: NativeHttp,
    bounds: OperationalBounds,
    generation_model: String,
    edit_model: String,
    alpha_model: String,
}

impl std::fmt::Debug for FalExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FalExecutor")
            .finish_non_exhaustive()
    }
}

impl FalExecutor {
    pub(crate) fn new(
        config: FalConfig,
        credential: &str,
        bounds: OperationalBounds,
        total_timeout: Duration,
    ) -> Result<Self, ApiError> {
        check_credential(credential)?;
        bounds.validate()?;
        let total_timeout = total_timeout.min(super::HTTP_TIMEOUT_CEILING);
        let limits = NativeHttpLimits {
            connect_timeout: Duration::from_secs(5).min(total_timeout),
            read_timeout: total_timeout,
            total_timeout,
            maximum_request_bytes: bounds.fal_max_request_bytes,
            maximum_response_bytes: bounds.fal_max_response_bytes,
            maximum_header_bytes: 16 * 1024,
            maximum_chunks: 65_536,
        };
        let http =
            NativeHttp::new(limits, config.allow_http).map_err(|_| ApiError::InvalidRequest)?;
        Ok(Self {
            origin: config.origin,
            credential: credential.to_owned(),
            http,
            bounds,
            generation_model: config.generation_model,
            edit_model: config.edit_model,
            alpha_model: config.alpha_model,
        })
    }

    #[cfg(test)]
    pub(crate) async fn submit_generation(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<FalGenerationCheckpoint, ApiError> {
        self.submit_generation_observed(edit, deadline)
            .await
            .map_err(|failure| failure.error)
    }
    #[cfg(test)]
    pub(crate) async fn resume_generation(
        &self,
        checkpoint: &FalGenerationCheckpoint,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalGenerationReceipt), ApiError> {
        self.resume_generation_observed(checkpoint, deadline)
            .await
            .map_err(|failure| failure.error)
    }
    #[cfg(test)]
    pub(crate) async fn submit_alpha_removal(
        &self,
        generated: &[u8],
        format: ImageFormat,
        deadline: Duration,
    ) -> Result<FalAlphaCheckpoint, ApiError> {
        self.submit_alpha_removal_observed(generated, format, deadline)
            .await
            .map_err(|failure| failure.error)
    }
    #[cfg(test)]
    pub(crate) async fn resume_alpha_removal(
        &self,
        checkpoint: &FalAlphaCheckpoint,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalAlphaReceipt), ApiError> {
        self.resume_alpha_removal_observed(checkpoint, deadline)
            .await
            .map_err(|failure| failure.error)
    }
    /// Paid submit for the generation stage: validates, POSTs once and
    /// returns the durable checkpoint. A transport failure or malformed
    /// receipt after the POST is unknown to the service, never proven
    /// unsent: the runner persists nothing and must not auto-resubmit.
    pub(crate) async fn submit_generation_observed(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<FalGenerationCheckpoint, ApiFailure> {
        validate_generation(edit).map_err(ApiFailure::before_transport)?;
        let opaque = if edit.output == ImageFormat::Jpeg {
            ImageFormat::Jpeg
        } else {
            ImageFormat::Png
        };
        let (slug, body) = if edit.references.is_empty() {
            (
                self.generation_model.as_str(),
                generation_body(edit, opaque),
            )
        } else {
            (self.edit_model.as_str(), kontext_body(edit, opaque))
        };
        let queue = self
            .submit_stage(GENERATION_CHECKPOINT_KIND, slug, &body, deadline)
            .await?;
        Ok(FalGenerationCheckpoint {
            origin: self.origin.clone(),
            endpoint: slug.to_owned(),
            request_id: queue.request_id,
            status_path: queue.status_path,
            result_path: queue.result_path,
        })
    }

    /// Resume a persisted generation checkpoint with a fresh executor. Never
    /// POSTs: polls status and reads the result only. The checkpoint origin,
    /// model route and paths are validated before any egress; egress always
    /// targets this executor's origin, never a host from retained data.
    pub(crate) async fn resume_generation_observed(
        &self,
        checkpoint: &FalGenerationCheckpoint,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalGenerationReceipt), ApiFailure> {
        self.check_generation_checkpoint(checkpoint)
            .map_err(|error| ApiFailure::unknown(error, ApiExchange::Poll))?;
        let started = std::time::Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        self.poll_result(
            checkpoint.request_id.clone(),
            checkpoint.endpoint.clone(),
            checkpoint.status_path.clone(),
            checkpoint.result_path.clone(),
            remaining(),
        )
        .await
        .map(|(bytes, format, stage)| (bytes, format, FalGenerationReceipt { stage }))
    }

    /// Paid submit for alpha extraction: validates, POSTs once and returns
    /// the durable checkpoint. Unknown-send semantics match generation.
    pub(crate) async fn submit_alpha_removal_observed(
        &self,
        generated: &[u8],
        format: ImageFormat,
        deadline: Duration,
    ) -> Result<FalAlphaCheckpoint, ApiFailure> {
        if generated.is_empty() || generated.len() > self.bounds.fal_max_image_bytes {
            return Err(ApiFailure::before_transport(ApiError::InvalidRequest));
        }
        let body = json!({
            "image_url": data_uri(format, generated),
            "model": "General Use (Heavy)",
            "operating_resolution": "1024x1024",
            "output_format": "png",
            "sync_mode": true,
        });
        let queue = self
            .submit_stage(
                ALPHA_CHECKPOINT_KIND,
                self.alpha_model.as_str(),
                &body,
                deadline,
            )
            .await?;
        Ok(FalAlphaCheckpoint {
            origin: self.origin.clone(),
            endpoint: self.alpha_model.clone(),
            request_id: queue.request_id,
            status_path: queue.status_path,
            result_path: queue.result_path,
        })
    }

    /// Resume a persisted alpha checkpoint. Never POSTs; validates origin,
    /// route and paths before any egress.
    pub(crate) async fn resume_alpha_removal_observed(
        &self,
        checkpoint: &FalAlphaCheckpoint,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalAlphaReceipt), ApiFailure> {
        self.check_alpha_checkpoint(checkpoint)
            .map_err(|error| ApiFailure::unknown(error, ApiExchange::Poll))?;
        let started = std::time::Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        self.poll_result(
            checkpoint.request_id.clone(),
            checkpoint.endpoint.clone(),
            checkpoint.status_path.clone(),
            checkpoint.result_path.clone(),
            remaining(),
        )
        .await
        .map(|(bytes, format, stage)| (bytes, format, FalAlphaReceipt { stage }))
    }

    /// Generation stage alone: text or multi-reference FLUX dispatch with
    /// queue submit, status polling and in-band result read.
    #[cfg(test)]
    pub(crate) async fn run_generation(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalGenerationReceipt), ApiError> {
        let started = std::time::Instant::now();
        let checkpoint = self
            .submit_generation(edit, deadline.saturating_sub(started.elapsed()))
            .await?;
        self.resume_generation(&checkpoint, deadline.saturating_sub(started.elapsed()))
            .await
    }

    /// Paid alpha-extraction stage alone: one BiRefNet dispatch over the
    /// generation bytes supplied by the caller.
    #[cfg(test)]
    pub(crate) async fn run_alpha_removal(
        &self,
        generated: &[u8],
        format: ImageFormat,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalAlphaReceipt), ApiError> {
        let started = std::time::Instant::now();
        let checkpoint = self
            .submit_alpha_removal(
                generated,
                format,
                deadline.saturating_sub(started.elapsed()),
            )
            .await?;
        self.resume_alpha_removal(&checkpoint, deadline.saturating_sub(started.elapsed()))
            .await
    }

    /// Full edit preserving legacy behavior: generation, then alpha removal
    /// only for transparent requests, with both receipts retained.
    #[cfg(test)]
    pub(crate) async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<FalEditOutcome, ApiError> {
        let started = std::time::Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        let (bytes, format, generation) = self.run_generation(edit, remaining()).await?;
        let (bytes, format, alpha) = if edit.background == ImageBackground::Transparent {
            let (bytes, format, receipt) =
                self.run_alpha_removal(&bytes, format, remaining()).await?;
            (bytes, format, Some(receipt))
        } else {
            (bytes, format, None)
        };
        let (bytes, (width, height, has_alpha_channel)) = facts(format, bytes).await?;
        if edit.background == ImageBackground::Transparent && !has_alpha_channel {
            return Err(ApiError::Output);
        }
        Ok(FalEditOutcome {
            image: GeneratedImage {
                bytes,
                format,
                width,
                height,
                has_alpha_channel,
                usage: ImageUsage::default(),
            },
            generation,
            alpha,
        })
    }

    async fn submit_stage(
        &self,
        kind: &str,
        endpoint: &str,
        body: &Value,
        deadline: Duration,
    ) -> Result<QueueReply, ApiFailure> {
        check_model_slug(endpoint).map_err(ApiFailure::before_transport)?;
        let started = std::time::Instant::now();
        let submitted = self
            .exchange(
                HttpMethod::Post,
                &format!("/{endpoint}"),
                body.to_string().into_bytes(),
                deadline.saturating_sub(started.elapsed()),
                ApiExchange::Submit,
            )
            .await?;
        queue_reply(
            kind,
            &self.origin,
            endpoint,
            &submitted,
            self.bounds.fal_max_response_bytes,
        )
        .map_err(|error| {
            ApiFailure::response(
                error,
                ApiExchange::Submit,
                DeclineContract::FalQueue429V1,
                submitted,
            )
        })
    }
    async fn poll_result(
        &self,
        request_id: String,
        endpoint: String,
        status_path: String,
        result_path: String,
        deadline: Duration,
    ) -> Result<(Vec<u8>, ImageFormat, FalStageReceipt), ApiFailure> {
        let started = std::time::Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        loop {
            let response = self
                .exchange(
                    HttpMethod::Get,
                    &status_path,
                    Vec::new(),
                    remaining(),
                    ApiExchange::Poll,
                )
                .await?;
            let state = json_body(&response, self.bounds.fal_max_response_bytes)
                .and_then(|value| status_of(&value))
                .map_err(|error| {
                    ApiFailure::response(
                        error,
                        ApiExchange::Poll,
                        DeclineContract::FalQueue429V1,
                        response,
                    )
                })?;
            match state {
                Status::Completed => break,
                Status::Waiting => {
                    let pause = self.bounds.poll_interval.min(remaining());
                    if pause.is_zero() {
                        return Err(ApiFailure::accepted(
                            ApiError::Transport(NativeHttpRefusal::Deadline),
                            ApiExchange::Poll,
                        ));
                    }
                    tokio::time::sleep(pause).await;
                }
            }
        }
        let result = self
            .exchange(
                HttpMethod::Get,
                &result_path,
                Vec::new(),
                remaining(),
                ApiExchange::Result,
            )
            .await?;
        let receipt = FalStageReceipt {
            endpoint,
            request_id: Some(request_id),
            http_status: Some(result.status),
        };
        let decoded = json_body(&result, self.bounds.fal_max_response_bytes)
            .and_then(|value| first_image(&value, self.bounds.fal_max_image_bytes))
            .map_err(|error| {
                ApiFailure::response(
                    error,
                    ApiExchange::Result,
                    DeclineContract::FalQueue429V1,
                    result,
                )
            })?;
        Ok((decoded.0, decoded.1, receipt))
    }

    fn check_generation_checkpoint(
        &self,
        checkpoint: &FalGenerationCheckpoint,
    ) -> Result<(), ApiError> {
        if checkpoint.origin != self.origin {
            return Err(ApiError::InvalidRequest);
        }
        if checkpoint.endpoint != self.generation_model && checkpoint.endpoint != self.edit_model {
            return Err(ApiError::InvalidRequest);
        }
        check_checkpoint_fields(
            GENERATION_CHECKPOINT_KIND,
            &checkpoint.origin,
            &checkpoint.endpoint,
            &checkpoint.request_id,
            &checkpoint.status_path,
            &checkpoint.result_path,
        )?;
        Ok(())
    }

    fn check_alpha_checkpoint(&self, checkpoint: &FalAlphaCheckpoint) -> Result<(), ApiError> {
        if checkpoint.origin != self.origin {
            return Err(ApiError::InvalidRequest);
        }
        if checkpoint.endpoint != self.alpha_model {
            return Err(ApiError::InvalidRequest);
        }
        check_checkpoint_fields(
            ALPHA_CHECKPOINT_KIND,
            &checkpoint.origin,
            &checkpoint.endpoint,
            &checkpoint.request_id,
            &checkpoint.status_path,
            &checkpoint.result_path,
        )?;
        Ok(())
    }

    async fn exchange(
        &self,
        method: HttpMethod,
        path: &str,
        body: Vec<u8>,
        remaining: Duration,
        exchange: ApiExchange,
    ) -> Result<df_providers::NativeHttpResponse, ApiFailure> {
        let request = HttpRequest::fal_native(
            method,
            &format!("{}{path}", self.origin),
            &self.credential,
            body,
        );
        let response = self
            .http
            .execute(request, remaining, std::future::pending())
            .await
            .map_err(|failure| ApiFailure::transport(failure.reason, exchange))?;
        let error = match response.status {
            200 | 202 => None,
            401 | 403 => Some(ApiError::Unauthorized),
            422 if is_content_refusal(&response.body) => Some(ApiError::Refused),
            429 => Some(ApiError::RateLimited),
            status => Some(ApiError::Provider(status)),
        };
        match error {
            Some(error) => Err(ApiFailure::response(
                error,
                exchange,
                DeclineContract::FalQueue429V1,
                response,
            )),
            None => Ok(response),
        }
    }
}

fn validate_generation(edit: &ImageEdit<'_>) -> Result<(), ApiError> {
    let pixels = u64::from(edit.width) * u64::from(edit.height);
    let sides_ok = edit.width.is_multiple_of(16)
        && edit.height.is_multiple_of(16)
        && edit.width.max(edit.height) <= 3840
        && edit.width.max(edit.height) <= edit.width.min(edit.height) * 3;
    let references_ok = edit.references.len() <= 4
        && edit.references.iter().all(|reference| {
            !reference.bytes.is_empty() && reference.bytes.len() <= 4 * 1024 * 1024
        });
    let output_ok = matches!(
        (edit.background, edit.output),
        (
            ImageBackground::Transparent,
            ImageFormat::Png | ImageFormat::Webp
        ) | (
            ImageBackground::Opaque,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Webp
        )
    );
    if edit.prompt.trim().is_empty()
        || edit.prompt.chars().count() > super::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS
        || !references_ok
        || !sides_ok
        || !(655_360..=4_194_304).contains(&pixels)
        || !output_ok
    {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

fn aspect_ratio(width: u32, height: u32) -> &'static str {
    let wanted = f64::from(width) / f64::from(height.max(1));
    ASPECT_RATIOS
        .iter()
        .min_by(|left, right| (left.1 - wanted).abs().total_cmp(&(right.1 - wanted).abs()))
        .map_or("16:9", |(name, _)| name)
}

fn safety_tolerance(moderation: ImageModeration) -> &'static str {
    match moderation {
        ImageModeration::Low => "5",
        ImageModeration::Auto => "2",
    }
}

fn wire_format(format: ImageFormat) -> &'static str {
    if format == ImageFormat::Jpeg {
        "jpeg"
    } else {
        "png"
    }
}

fn generation_body(edit: &ImageEdit<'_>, format: ImageFormat) -> Value {
    json!({
        "prompt": edit.prompt,
        "raw": true,
        "aspect_ratio": aspect_ratio(edit.width, edit.height),
        "output_format": wire_format(format),
        "safety_tolerance": safety_tolerance(edit.moderation),
        "num_images": 1,
        "sync_mode": true,
    })
}

fn kontext_body(edit: &ImageEdit<'_>, format: ImageFormat) -> Value {
    let mut seen = HashSet::new();
    let images: Vec<String> = edit
        .references
        .iter()
        .map(|reference| data_uri(reference.format, reference.bytes))
        .filter(|image| seen.insert(image.clone()))
        .collect();
    json!({
        "prompt": edit.prompt,
        "image_urls": images,
        "aspect_ratio": aspect_ratio(edit.width, edit.height),
        "output_format": wire_format(format),
        "safety_tolerance": safety_tolerance(edit.moderation),
        "guidance_scale": 3.5,
        "num_images": 1,
        "sync_mode": true,
    })
}

fn data_uri(format: ImageFormat, bytes: &[u8]) -> String {
    format!("data:{};base64,{}", format.mime(), STANDARD.encode(bytes))
}

fn json_body(
    response: &df_providers::NativeHttpResponse,
    max_bytes: usize,
) -> Result<Value, ApiError> {
    if response.body.len() > max_bytes {
        return Err(ApiError::Output);
    }
    let value = parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: max_bytes,
            maximum_depth: 16,
            maximum_nodes: 65_536,
        },
    )
    .map_err(|_| ApiError::Output)?;
    if !value.is_object() {
        return Err(ApiError::Output);
    }
    Ok(value)
}

fn is_content_refusal(body: &[u8]) -> bool {
    [&b"content_policy"[..], &b"nsfw"[..], &b"safety"[..]]
        .iter()
        .any(|needle| body.windows(needle.len()).any(|window| window == *needle))
}

struct QueueReply {
    status_path: String,
    result_path: String,
    request_id: String,
}

fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn check_checkpoint_fields(
    kind: &str,
    origin: &str,
    endpoint: &str,
    request_id: &str,
    status_path: &str,
    result_path: &str,
) -> Result<(), ApiError> {
    if origin.len() > MAXIMUM_LOCATOR_BYTES
        || status_path.len() > MAXIMUM_LOCATOR_BYTES
        || result_path.len() > MAXIMUM_LOCATOR_BYTES
    {
        return Err(ApiError::InvalidRequest);
    }
    check_endpoint_url(origin)?;
    check_model_slug(endpoint)?;
    if !valid_request_id(request_id)
        || endpoint
            .split('/')
            .any(|segment| segment.is_empty() || segment == ".")
    {
        return Err(ApiError::InvalidRequest);
    }
    let encoded_bytes = kind.len()
        + 5
        + origin.len()
        + endpoint.len()
        + request_id.len()
        + status_path.len()
        + result_path.len();
    if encoded_bytes > MAXIMUM_CHECKPOINT_BYTES {
        return Err(ApiError::InvalidRequest);
    }
    let status_route = check_confined_shape(status_path, request_id, Some("status"))?;
    let result_route = check_confined_shape(result_path, request_id, None)?;
    if status_route != result_route || !queue_route_allowed(endpoint, status_route) {
        return Err(ApiError::InvalidRequest);
    }
    Ok(())
}

fn queue_route_allowed(endpoint: &str, route: &str) -> bool {
    if route == endpoint {
        return true;
    }
    // fal queues variants under either the full endpoint or its owner/application base.
    let mut segments = endpoint.split('/');
    let Some(owner) = segments.next() else {
        return false;
    };
    let Some(application) = segments.next() else {
        return false;
    };
    route.len() == owner.len() + 1 + application.len()
        && route
            .strip_prefix(owner)
            .and_then(|tail| tail.strip_prefix('/'))
            == Some(application)
}

fn confined_path<'a>(origin: &str, url: &'a str) -> Option<&'a str> {
    url.strip_prefix(origin)
        .or_else(|| url.strip_prefix(QUEUE_ORIGIN))
}

fn check_confined_shape<'a>(
    path: &'a str,
    id: &str,
    suffix: Option<&str>,
) -> Result<&'a str, ApiError> {
    if path.is_empty()
        || path.len() > MAXIMUM_LOCATOR_BYTES
        || path.contains("..")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
    {
        return Err(ApiError::InvalidRequest);
    }
    let tail = path.strip_prefix('/').ok_or(ApiError::InvalidRequest)?;
    let (app, rest) = tail
        .rsplit_once("/requests/")
        .ok_or(ApiError::InvalidRequest)?;
    if app.split('/').count() < 2
        || app
            .split('/')
            .any(|segment| segment.is_empty() || segment == ".")
    {
        return Err(ApiError::InvalidRequest);
    }
    let request_id = match suffix {
        Some("status") => rest.strip_suffix("/status"),
        None => Some(rest.strip_suffix("/response").unwrap_or(rest)),
        Some(_) => None,
    };
    if request_id != Some(id) {
        return Err(ApiError::InvalidRequest);
    }
    Ok(app)
}

fn queue_reply(
    kind: &str,
    origin: &str,
    endpoint: &str,
    response: &df_providers::NativeHttpResponse,
    max_bytes: usize,
) -> Result<QueueReply, ApiError> {
    let value = json_body(response, max_bytes)?;
    let id = value
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or(ApiError::Output)?;
    let path = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .and_then(|url| confined_path(origin, url))
            .ok_or(ApiError::Output)
    };
    let status_path = path("status_url")?;
    let result_path = path("response_url")?;
    check_checkpoint_fields(kind, origin, endpoint, id, status_path, result_path)
        .map_err(|_| ApiError::Output)?;
    Ok(QueueReply {
        status_path: status_path.to_owned(),
        result_path: result_path.to_owned(),
        request_id: id.to_owned(),
    })
}

enum Status {
    Waiting,
    Completed,
}

fn status_of(value: &Value) -> Result<Status, ApiError> {
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(ApiError::Provider(500));
    }
    match value.get("status").and_then(Value::as_str) {
        Some("IN_QUEUE" | "IN_PROGRESS") => Ok(Status::Waiting),
        Some("COMPLETED") => Ok(Status::Completed),
        Some(_) => Err(ApiError::Provider(500)),
        None => Err(ApiError::Output),
    }
}

fn first_image(value: &Value, max_image_bytes: usize) -> Result<(Vec<u8>, ImageFormat), ApiError> {
    if value
        .get("has_nsfw_concepts")
        .and_then(Value::as_array)
        .is_some_and(|flags| flags.iter().any(|flag| flag.as_bool() == Some(true)))
    {
        return Err(ApiError::Refused);
    }
    let url = value
        .get("images")
        .and_then(Value::as_array)
        .and_then(|images| images.first())
        .or_else(|| value.get("image"))
        .and_then(|image| image.get("url"))
        .and_then(Value::as_str)
        .ok_or(ApiError::Refused)?;
    let (header, payload) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .ok_or(ApiError::Output)?;
    let format = match header {
        "image/jpeg" => ImageFormat::Jpeg,
        "image/png" => ImageFormat::Png,
        "image/webp" => ImageFormat::Webp,
        _ => return Err(ApiError::Output),
    };
    let bytes = STANDARD.decode(payload).map_err(|_| ApiError::Output)?;
    if bytes.is_empty() || bytes.len() > max_image_bytes {
        return Err(ApiError::Output);
    }
    Ok((bytes, format))
}

#[cfg(test)]
async fn facts(
    format: ImageFormat,
    bytes: Vec<u8>,
) -> Result<(Vec<u8>, (u32, u32, bool)), ApiError> {
    match format {
        ImageFormat::Png => {
            let (bytes, EncodedMediaFacts::Png { width, height }) = inspect_png_off_worker(bytes)
                .await
                .map_err(|_| ApiError::Output)?
            else {
                return Err(ApiError::Output);
            };
            let color_type = bytes.get(25).copied().ok_or(ApiError::Output)?;
            Ok((bytes, (width, height, matches!(color_type, 4 | 6))))
        }
        ImageFormat::Jpeg => {
            let (width, height) = jpeg_size(&bytes).ok_or(ApiError::Output)?;
            Ok((bytes, (width, height, false)))
        }
        ImageFormat::Webp => {
            let facts = webp_facts(&bytes).ok_or(ApiError::Output)?;
            Ok((bytes, facts))
        }
    }
}
