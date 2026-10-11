//! Caller-driven FAL media queue and complete encoded candidate acquisition.
//! No paid retry, asset publication, codec decoding, rights or billing authority.

use std::{fmt, future::Future, net::SocketAddr, time::Duration};

use df_model::checkpoint::{AssetKind, AssetReference, ExecutionMode};
use df_provider_api::{
    CheckedModalityRequest, ImageRequest, ImageSemantics, RequestBasis, RequestError,
    RequestOwnerState, SoundRequest, SoundSemantics, VideoRequest, VideoSemantics,
};
use df_types::{Usage, UsageUnit};
use serde_json::{Value, json};

use crate::media_container::{
    EncodedMediaFacts, MediaContainerRefusal, MediaTimebase, inspect_mp4, inspect_png, inspect_wave,
};
use crate::native_http::{NativeHttp, NativeHttpResponse};
use crate::native_media_fetch::{FalCdnPin, NativeMediaFetch};
use crate::output_fetch::strict_fal_locator;
use crate::text_schema::{TextSchemaLimits, parse_unique_json};
use crate::{
    FalQueueObservation, HttpMethod, HttpRequest, NativeHttpLimits, NativeHttpRefusal,
    ProviderFailureClass, ProviderRequestId, classify_fal_status_response,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FalMediaFamily {
    Image,
    Video,
    Sound,
}

impl FalMediaFamily {
    fn model(self) -> &'static str {
        match self {
            Self::Image => "fal-ai/flux/schnell",
            Self::Video => "fal-ai/wan-25-preview/image-to-video",
            Self::Sound => "fal-ai/stable-audio-25/text-to-audio",
        }
    }
    fn unit(self) -> UsageUnit {
        match self {
            Self::Image => UsageUnit::Image,
            Self::Video => UsageUnit::VideoMillisecond,
            Self::Sound => UsageUnit::AudioMillisecond,
        }
    }
}

/// Explicit request settings; unsupported choices refuse before submission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FalImageProfile {
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub images: u32,
    pub steps: u32,
    pub seed: u32,
    pub safety: bool,
    pub format: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FalVideoProfile {
    pub model: String,
    pub duration_seconds: u32,
    pub resolution: String,
    pub width: u32,
    pub height: u32,
    pub accepted_duration_min: Duration,
    pub accepted_duration_max: Duration,
    pub seed: u32,
    pub safety: bool,
    pub prompt_expansion: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FalSoundProfile {
    pub model: String,
    pub duration_seconds: u32,
    pub steps: u32,
    pub seed: u32,
    pub safety: bool,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub accepted_duration_min: Duration,
    pub accepted_duration_max: Duration,
}

#[derive(Clone, Eq, PartialEq)]
pub struct FalReferenceFrame {
    pub asset: AssetReference,
    pub locator: String,
}
impl fmt::Debug for FalReferenceFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FalReferenceFrame")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FalLoopFrames {
    pub first: u64,
    pub end_exclusive: u64,
}

pub type FalImageSemantics = ImageSemantics<String, FalImageProfile>;
pub type FalVideoSemantics = VideoSemantics<String, FalVideoProfile, FalReferenceFrame>;
pub type FalSoundSemantics = SoundSemantics<String, FalSoundProfile, Option<FalLoopFrames>>;
pub type FalImageBasis = RequestBasis<FalImageSemantics>;
pub type FalVideoBasis = RequestBasis<FalVideoSemantics>;
pub type FalSoundBasis = RequestBasis<FalSoundSemantics>;
pub type FalImageJob<'a> = FalMediaJob<'a, FalImageSemantics>;
pub type FalVideoJob<'a> = FalMediaJob<'a, FalVideoSemantics>;
pub type FalSoundJob<'a> = FalMediaJob<'a, FalSoundSemantics>;
pub type FalVideoCompletionEvent =
    df_provider_api::VideoEvent<ProviderRequestId, (), FalQueueObservation, Vec<EncodedFalMedia>>;

pub type FalImageRequest = ImageRequest<String, FalImageProfile>;
pub type FalVideoRequest = VideoRequest<String, FalVideoProfile, FalReferenceFrame>;
pub type FalSoundRequest = SoundRequest<String, FalSoundProfile, Option<FalLoopFrames>>;

/// Native factory configuration. Fixture sockets are literal loopback and explicit.
pub struct FalMediaConfig {
    family: FalMediaFamily,
    fixture_api: Option<SocketAddr>,
    fixture_media: Option<SocketAddr>,
    pins: Vec<FalCdnPin>,
}

impl FalMediaConfig {
    pub fn live(family: FalMediaFamily, pins: Vec<FalCdnPin>) -> Self {
        Self {
            family,
            fixture_api: None,
            fixture_media: None,
            pins,
        }
    }
    pub fn loopback_fixture(
        family: FalMediaFamily,
        api: SocketAddr,
        media: SocketAddr,
    ) -> Result<Self, FalMediaReason> {
        if !api.ip().is_loopback()
            || api.port() == 0
            || !media.ip().is_loopback()
            || media.port() == 0
        {
            return Err(FalMediaReason::Profile);
        }
        Ok(Self {
            family,
            fixture_api: Some(api),
            fixture_media: Some(media),
            pins: Vec::new(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FalMediaReason {
    Profile,
    Payload,
    Reference,
    QueueIdentity,
    QueueUrls,
    Json,
    Status,
    NotCompleted,
    Owner(RequestError),
    Transport(NativeHttpRefusal),
    Container(MediaContainerRefusal),
    Metadata,
    UsageOverrun,
}

/// Safe retained facts on rejection. Supplier identity and observed usage survive failure.
#[derive(Debug)]
pub struct FalMediaError {
    pub reason: FalMediaReason,
    pub request_id: Option<ProviderRequestId>,
    pub observed_usage: Option<Usage>,
}

impl FalMediaError {
    pub fn failure_class(&self) -> ProviderFailureClass {
        match self.reason {
            FalMediaReason::Owner(RequestError::Cancelled) => ProviderFailureClass::Cancelled,
            FalMediaReason::Owner(RequestError::DeadlineExceeded) => ProviderFailureClass::Deadline,
            FalMediaReason::Transport(reason) => reason.failure_class(),
            FalMediaReason::Profile
            | FalMediaReason::Payload
            | FalMediaReason::Reference
            | FalMediaReason::Owner(_) => ProviderFailureClass::InvalidRequest,
            FalMediaReason::Status | FalMediaReason::QueueIdentity | FalMediaReason::QueueUrls => {
                ProviderFailureClass::Unknown
            }
            _ => ProviderFailureClass::Contract,
        }
    }
}

pub struct NativeFalMediaProvider {
    config: FalMediaConfig,
    credential: String,
    http: NativeHttp,
    fetch: NativeMediaFetch,
}
impl fmt::Debug for NativeFalMediaProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeFalMediaProvider")
            .finish_non_exhaustive()
    }
}

impl NativeFalMediaProvider {
    pub(crate) fn new(
        config: FalMediaConfig,
        credential: &str,
        limits: NativeHttpLimits,
    ) -> Result<Self, FalMediaError> {
        if credential.is_empty()
            || credential.len() > 4096
            || !credential.bytes().all(|b| (0x21..=0x7e).contains(&b))
            || limits.maximum_header_bytes > 16_384
        {
            return Err(error(FalMediaReason::Profile, None));
        }
        let http = NativeHttp::new(limits, config.fixture_api.is_some())
            .map_err(|r| error(FalMediaReason::Transport(r), None))?;
        let fetch = NativeMediaFetch::new(limits, config.pins.clone(), config.fixture_media)
            .map_err(|r| error(FalMediaReason::Transport(r), None))?;
        Ok(Self {
            config,
            credential: credential.to_owned(),
            http,
            fetch,
        })
    }

    pub async fn submit_image<'a>(
        &'a self,
        request: &'a FalImageRequest,
        owner: RequestOwnerState<'_, FalImageBasis>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<FalImageJob<'a>, FalMediaError> {
        let semantic = &request.checked().binding().semantic_basis.semantic;
        let profile = &semantic.output_profile;
        if self.config.family != FalMediaFamily::Image
            || profile.model != FalMediaFamily::Image.model()
            || !bounded_dimensions(profile.width, profile.height)
            || profile.images == 0
            || profile.images > 4
            || profile.steps == 0
            || profile.steps > 12
            || profile.format != "png"
        {
            return Err(error(FalMediaReason::Profile, None));
        }
        if !semantic.references.is_empty() {
            return Err(error(FalMediaReason::Reference, None));
        }
        validate_prompt(&semantic.prompt, 10_000)?;
        let declared = request.checked().usage();
        if declared.usage().unit() != UsageUnit::Image
            || declared.usage().quantity() != u128::from(profile.images)
            || !declared.duration().is_zero()
            || declared.tokens() != 0
        {
            return Err(error(FalMediaReason::Profile, None));
        }
        self.submit(request.checked(), owner, json!({
            "prompt":semantic.prompt,"image_size":{"width":profile.width,"height":profile.height},
            "num_images":profile.images,"output_format":profile.format,"num_inference_steps":profile.steps,
            "seed":profile.seed,"enable_safety_checker":profile.safety
        }), ExpectedMedia::Image(profile.clone()), cancellation).await
    }

    pub async fn submit_video<'a>(
        &'a self,
        request: &'a FalVideoRequest,
        owner: RequestOwnerState<'_, FalVideoBasis>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<FalVideoJob<'a>, FalMediaError> {
        let semantic = &request.checked().binding().semantic_basis.semantic;
        let p = &semantic.output_profile;
        let resolution_side = match p.resolution.as_str() {
            "480p" => 480,
            "720p" => 720,
            "1080p" => 1080,
            _ => 0,
        };
        if self.config.family != FalMediaFamily::Video
            || p.model != FalMediaFamily::Video.model()
            || !matches!(p.duration_seconds, 5 | 10)
            || resolution_side == 0
            || p.width.min(p.height) != resolution_side
            || !bounded_dimensions(p.width, p.height)
            || !valid_interval(p.accepted_duration_min, p.accepted_duration_max)
        {
            return Err(error(FalMediaReason::Profile, None));
        }
        let [frame] = semantic.reference_frames.as_slice() else {
            return Err(error(FalMediaReason::Reference, None));
        };
        if frame.asset.kind != AssetKind::Image
            || frame.asset.byte_length == 0
            || frame.asset.byte_length > 10 * 1024 * 1024
            || !strict_fal_locator(&frame.locator)
            || frame.locator.len() > 8192
        {
            return Err(error(FalMediaReason::Reference, None));
        }
        validate_prompt(&semantic.prompt, 1500)?;
        validate_declared(
            request.checked(),
            Duration::from_secs(u64::from(p.duration_seconds)),
            UsageUnit::VideoMillisecond,
        )?;
        self.submit(request.checked(), owner, json!({
            "prompt":semantic.prompt,"image_url":frame.locator,"duration":p.duration_seconds.to_string(),
            "resolution":p.resolution,"seed":p.seed,"enable_safety_checker":p.safety,
            "enable_prompt_expansion":p.prompt_expansion
        }), ExpectedMedia::Video(p.clone()), cancellation).await
    }

    pub async fn submit_sound<'a>(
        &'a self,
        request: &'a FalSoundRequest,
        owner: RequestOwnerState<'_, FalSoundBasis>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<FalSoundJob<'a>, FalMediaError> {
        let semantic = &request.checked().binding().semantic_basis.semantic;
        let p = &semantic.output_profile;
        if self.config.family != FalMediaFamily::Sound
            || p.model != FalMediaFamily::Sound.model()
            || p.duration_seconds == 0
            || p.duration_seconds > 180
            || p.steps == 0
            || p.steps > 250
            || !(8000..=192000).contains(&p.sample_rate)
            || !matches!(p.channels, 1 | 2)
            || !matches!(p.bits_per_sample, 8 | 16 | 24 | 32)
            || !valid_interval(p.accepted_duration_min, p.accepted_duration_max)
            || semantic
                .loop_metadata
                .is_some_and(|v| v.first >= v.end_exclusive)
        {
            return Err(error(FalMediaReason::Profile, None));
        }
        validate_prompt(&semantic.intent, 10_000)?;
        validate_declared(
            request.checked(),
            Duration::from_secs(u64::from(p.duration_seconds)),
            UsageUnit::AudioMillisecond,
        )?;
        self.submit(request.checked(), owner, json!({
            "prompt":semantic.intent,"seconds_total":p.duration_seconds,"num_inference_steps":p.steps,
            "seed":p.seed,"enable_safety_checker":p.safety
        }), ExpectedMedia::Sound(p.clone(), semantic.loop_metadata), cancellation).await
    }

    async fn submit<'a, Semantic: Eq>(
        &'a self,
        request: &'a CheckedModalityRequest<Semantic>,
        owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
        body: Value,
        expected: ExpectedMedia,
        cancellation: impl Future<Output = ()>,
    ) -> Result<FalMediaJob<'a, Semantic>, FalMediaError> {
        request
            .validate_current(RequestOwnerState {
                current: owner.current,
                elapsed: owner.elapsed,
                cancelled: owner.cancelled,
            })
            .map_err(|r| error(FalMediaReason::Owner(r), None))?;
        if request.binding().mode != ExecutionMode::Live {
            return Err(error(FalMediaReason::Profile, None));
        }
        let serialized =
            serde_json::to_vec(&body).map_err(|_| error(FalMediaReason::Payload, None))?;
        // The checked payload must be exactly the admitted typed request serialization.
        if !request.payload().is_empty() && request.payload() != serialized {
            return Err(error(FalMediaReason::Payload, None));
        }
        let path = format!("/{}", self.config.family.model());
        let response = self
            .execute(
                HttpMethod::Post,
                &path,
                serialized,
                request.binding().deadline.saturating_sub(owner.elapsed),
                cancellation,
                None,
            )
            .await?;
        let value = parse_json(&response, None)?;
        let id = value
            .get("request_id")
            .and_then(Value::as_str)
            .filter(|id| valid_id(id))
            .and_then(|id| ProviderRequestId::new(id).ok());
        let id = id.ok_or_else(|| {
            error(
                FalMediaReason::QueueIdentity,
                response.transport_request_id.as_ref(),
            )
        })?;
        if !matches!(response.status, 200 | 202) {
            return Err(error(FalMediaReason::Status, Some(&id)));
        }
        let prefix = format!(
            "https://queue.fal.run/{}/requests/{}",
            self.config.family.model(),
            id.as_str()
        );
        let status = confined_url(&value, "status_url", &prefix, "status", &id)?;
        let result = confined_url(&value, "response_url", &prefix, "response", &id)?;
        let cancel = confined_url(&value, "cancel_url", &prefix, "cancel", &id)?;
        Ok(FalMediaJob {
            provider: self,
            request,
            id,
            status,
            result,
            cancel,
            expected,
            completed: false,
        })
    }

    async fn execute(
        &self,
        method: HttpMethod,
        path: &str,
        body: Vec<u8>,
        deadline: Duration,
        cancellation: impl Future<Output = ()>,
        id: Option<&ProviderRequestId>,
    ) -> Result<NativeHttpResponse, FalMediaError> {
        let origin = self.config.fixture_api.map_or_else(
            || "https://queue.fal.run".to_owned(),
            |a| format!("http://{a}"),
        );
        let request =
            HttpRequest::fal_native(method, &format!("{origin}{path}"), &self.credential, body);
        self.http
            .execute(request, deadline, cancellation)
            .await
            .map_err(|failure| {
                let mut value = error(FalMediaReason::Transport(failure.reason), id);
                if value.request_id.is_none() {
                    value.request_id = failure.transport_request_id;
                }
                value
            })
    }
}

/// One captured remote attempt. Polling/cancellation never submits another request.
pub struct FalMediaJob<'a, Semantic> {
    provider: &'a NativeFalMediaProvider,
    request: &'a CheckedModalityRequest<Semantic>,
    id: ProviderRequestId,
    status: String,
    result: String,
    cancel: String,
    expected: ExpectedMedia,
    completed: bool,
}
impl<Semantic> fmt::Debug for FalMediaJob<'_, Semantic> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FalMediaJob")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FalCancellationObservation {
    Requested,
    AlreadyCompleted,
    NotFound,
    Unknown,
}

impl<Semantic: Eq> FalMediaJob<'_, Semantic> {
    pub fn request_id(&self) -> &ProviderRequestId {
        &self.id
    }

    fn validate(
        &self,
        owner: &RequestOwnerState<'_, RequestBasis<Semantic>>,
    ) -> Result<Duration, FalMediaError> {
        self.request
            .validate_current(RequestOwnerState {
                current: owner.current,
                elapsed: owner.elapsed,
                cancelled: owner.cancelled,
            })
            .map_err(|r| error(FalMediaReason::Owner(r), Some(&self.id)))?;
        Ok(self
            .request
            .binding()
            .deadline
            .saturating_sub(owner.elapsed))
    }

    pub async fn status(
        &mut self,
        owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<FalQueueObservation, FalMediaError> {
        self.completed = false;
        let deadline = self.validate(&owner)?;
        let response = self
            .provider
            .execute(
                HttpMethod::Get,
                queue_path(&self.status)?,
                Vec::new(),
                deadline,
                cancellation,
                Some(&self.id),
            )
            .await?;
        let value = parse_json(&response, Some(&self.id))?;
        if value
            .get("response_url")
            .is_some_and(|v| v.as_str() != Some(self.result.as_str()))
        {
            return Err(error(FalMediaReason::QueueUrls, Some(&self.id)));
        }
        let metric = value.get("metrics").and_then(|v| v.get("inference_time"));
        if metric.is_some_and(|v| !v.is_number()) {
            return Err(error(FalMediaReason::Json, Some(&self.id)));
        }
        for field in ["error", "error_type"] {
            if value
                .get(field)
                .is_some_and(|v| !v.is_null() && !v.is_string())
            {
                return Err(error(FalMediaReason::Json, Some(&self.id)));
            }
        }
        let observation = classify_fal_status_response(
            response.status,
            &self.id,
            value.get("request_id").and_then(Value::as_str),
            value.get("status").and_then(Value::as_str),
            value.get("error_type").and_then(Value::as_str),
            value.get("error").and_then(Value::as_str),
            metric.and_then(Value::as_f64),
        )
        .map_err(|_| error(FalMediaReason::Status, Some(&self.id)))?;
        self.completed = matches!(observation, FalQueueObservation::Completed { .. });
        Ok(observation)
    }

    pub async fn cancel(
        &mut self,
        owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<FalCancellationObservation, FalMediaError> {
        let deadline = self.validate(&owner)?;
        let response = self
            .provider
            .execute(
                HttpMethod::Put,
                queue_path(&self.cancel)?,
                Vec::new(),
                deadline,
                cancellation,
                Some(&self.id),
            )
            .await?;
        let value = parse_json(&response, Some(&self.id))?;
        if value
            .get("request_id")
            .is_some_and(|v| v.as_str() != Some(self.id.as_str()))
        {
            return Err(error(FalMediaReason::QueueIdentity, Some(&self.id)));
        }
        let observation = match (response.status, value.get("status").and_then(Value::as_str)) {
            (202, Some("CANCELLATION_REQUESTED")) => FalCancellationObservation::Requested,
            (400, Some("ALREADY_COMPLETED")) => FalCancellationObservation::AlreadyCompleted,
            (404, Some("NOT_FOUND")) => FalCancellationObservation::NotFound,
            _ => FalCancellationObservation::Unknown,
        };
        self.completed = false;
        Ok(observation)
    }
}

impl<'a, Semantic: Eq> FalMediaJob<'a, Semantic> {
    /// Consumes this local result-fetch capability once. A failure retains remote identity.
    pub async fn result(
        self,
        owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
        cancellation: impl Future<Output = ()>,
    ) -> Result<PreparedFalMediaResponse<'a, Semantic>, FalMediaError> {
        let deadline = self.validate(&owner)?;
        if !self.completed {
            return Err(error(FalMediaReason::NotCompleted, Some(&self.id)));
        }
        let start = tokio::time::Instant::now();
        tokio::pin!(cancellation);
        let response = self
            .provider
            .execute(
                HttpMethod::Get,
                queue_path(&self.result)?,
                Vec::new(),
                deadline,
                cancellation.as_mut(),
                Some(&self.id),
            )
            .await?;
        if response.status != 200 {
            return Err(error(FalMediaReason::Status, Some(&self.id)));
        }
        let value = parse_json(&response, Some(&self.id))?;
        if value
            .get("request_id")
            .is_some_and(|v| v.as_str() != Some(self.id.as_str()))
            || value.get("error").is_some()
            || value.get("error_type").is_some()
        {
            return Err(error(FalMediaReason::QueueIdentity, Some(&self.id)));
        }
        let files = output_files(&value, &self.expected, &self.id)?;
        let maximum = self.provider.http.limits().maximum_response_bytes;
        let mut retained = response.body.len();
        let mut outputs = Vec::with_capacity(files.len());
        let mut quantity = 0u128;
        for file in files {
            let elapsed = owner.elapsed.checked_add(start.elapsed()).ok_or_else(|| {
                error(
                    FalMediaReason::Owner(RequestError::DeadlineExceeded),
                    Some(&self.id),
                )
            })?;
            self.request
                .validate_current(RequestOwnerState {
                    current: owner.current,
                    elapsed,
                    cancelled: owner.cancelled,
                })
                .map_err(|r| error(FalMediaReason::Owner(r), Some(&self.id)))?;
            let remaining = maximum.checked_sub(retained).ok_or_else(|| {
                error(
                    FalMediaReason::Transport(NativeHttpRefusal::ResponseTooLarge),
                    Some(&self.id),
                )
            })?;
            let body = self
                .provider
                .fetch
                .fetch(
                    &file.locator,
                    self.expected.mime(),
                    remaining,
                    self.request.binding().deadline.saturating_sub(elapsed),
                    cancellation.as_mut(),
                )
                .await
                .map_err(|r| error(FalMediaReason::Transport(r), Some(&self.id)))?;
            if file
                .metadata
                .get("file_size")
                .is_some_and(|v| v.as_u64() != Some(body.len() as u64))
            {
                return Err(error(FalMediaReason::Metadata, Some(&self.id)));
            }
            retained = retained.checked_add(body.len()).ok_or_else(|| {
                error(
                    FalMediaReason::Transport(NativeHttpRefusal::ResponseTooLarge),
                    Some(&self.id),
                )
            })?;
            let facts = match self.expected {
                ExpectedMedia::Image(_) => inspect_png(&body),
                ExpectedMedia::Video(_) => inspect_mp4(&body),
                ExpectedMedia::Sound(_, _) => inspect_wave(&body),
            }
            .map_err(|r| error(FalMediaReason::Container(r), Some(&self.id)))?;
            let units = validate_output(&self.expected, &file.metadata, facts, &self.id)?;
            quantity = quantity
                .checked_add(units)
                .ok_or_else(|| error(FalMediaReason::UsageOverrun, Some(&self.id)))?;
            outputs.push(EncodedFalMedia { bytes: body, facts });
        }
        let usage = Usage::new(quantity, self.provider.config.family.unit());
        if quantity > self.request.limits().max_usage().quantity() {
            return Err(FalMediaError {
                reason: FalMediaReason::UsageOverrun,
                request_id: Some(self.id),
                observed_usage: Some(usage),
            });
        }
        Ok(PreparedFalMediaResponse {
            request: self.request,
            id: self.id,
            usage,
            outputs,
        })
    }
}

/// Complete encoded bytes and structural facts. No durable AssetReference is minted.
pub struct EncodedFalMedia {
    bytes: Vec<u8>,
    facts: EncodedMediaFacts,
}
impl EncodedFalMedia {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn facts(&self) -> EncodedMediaFacts {
        self.facts
    }
}
impl fmt::Debug for EncodedFalMedia {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedFalMedia")
            .field("byte_length", &self.bytes.len())
            .field("facts", &self.facts)
            .finish()
    }
}

pub struct PreparedFalMediaResponse<'a, Semantic> {
    request: &'a CheckedModalityRequest<Semantic>,
    id: ProviderRequestId,
    usage: Usage,
    outputs: Vec<EncodedFalMedia>,
}
impl<Semantic> fmt::Debug for PreparedFalMediaResponse<'_, Semantic> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedFalMediaResponse")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct FalMediaCompletion {
    pub identity: df_provider_api::RequestIdentity,
    pub request_id: ProviderRequestId,
    pub usage: Usage,
    pub outputs: Vec<EncodedFalMedia>,
}
impl<Semantic: Eq> PreparedFalMediaResponse<'_, Semantic> {
    /// Fresh current-owner validation is required after all network work and before output consumption.
    pub fn complete(
        self,
        owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
    ) -> Result<FalMediaCompletion, FalMediaError> {
        self.request
            .validate_current(owner)
            .map_err(|reason| FalMediaError {
                reason: FalMediaReason::Owner(reason),
                request_id: Some(self.id.clone()),
                observed_usage: Some(self.usage),
            })?;
        Ok(FalMediaCompletion {
            identity: self.request.binding().identity,
            request_id: self.id,
            usage: self.usage,
            outputs: self.outputs,
        })
    }
}

enum ExpectedMedia {
    Image(FalImageProfile),
    Video(FalVideoProfile),
    Sound(FalSoundProfile, Option<FalLoopFrames>),
}
impl ExpectedMedia {
    fn mime(&self) -> &'static str {
        match self {
            Self::Image(_) => "image/png",
            Self::Video(_) => "video/mp4",
            Self::Sound(_, _) => "audio/wav",
        }
    }
}

struct OutputFile {
    locator: String,
    metadata: Value,
}

fn output_files(
    value: &Value,
    expected: &ExpectedMedia,
    id: &ProviderRequestId,
) -> Result<Vec<OutputFile>, FalMediaError> {
    let items: Vec<&Value> = match expected {
        ExpectedMedia::Image(profile) => {
            let values = value
                .get("images")
                .and_then(Value::as_array)
                .ok_or_else(|| error(FalMediaReason::Metadata, Some(id)))?;
            if values.len() != profile.images as usize {
                return Err(error(FalMediaReason::Metadata, Some(id)));
            }
            values.iter().collect()
        }
        ExpectedMedia::Video(_) => vec![
            value
                .get("video")
                .ok_or_else(|| error(FalMediaReason::Metadata, Some(id)))?,
        ],
        ExpectedMedia::Sound(_, _) => vec![
            value
                .get("audio")
                .ok_or_else(|| error(FalMediaReason::Metadata, Some(id)))?,
        ],
    };
    let mut files = Vec::with_capacity(items.len());
    for item in items {
        let locator = item
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| error(FalMediaReason::Metadata, Some(id)))?;
        if !item.is_object()
            || locator.len() > 8192
            || !strict_fal_locator(locator)
            || item.get("content_type").and_then(Value::as_str) != Some(expected.mime())
            || files
                .iter()
                .any(|file: &OutputFile| file.locator == locator)
        {
            return Err(error(FalMediaReason::Metadata, Some(id)));
        }
        files.push(OutputFile {
            locator: locator.to_owned(),
            metadata: item.clone(),
        });
    }
    Ok(files)
}

fn validate_output(
    expected: &ExpectedMedia,
    metadata: &Value,
    facts: EncodedMediaFacts,
    id: &ProviderRequestId,
) -> Result<u128, FalMediaError> {
    let mismatch = || error(FalMediaReason::Metadata, Some(id));
    match (expected, facts) {
        (ExpectedMedia::Image(p), EncodedMediaFacts::Png { width, height }) => {
            if (width, height) != (p.width, p.height)
                || metadata.get("width").and_then(Value::as_u64) != Some(u64::from(width))
                || metadata.get("height").and_then(Value::as_u64) != Some(u64::from(height))
            {
                return Err(mismatch());
            }
            Ok(1)
        }
        (
            ExpectedMedia::Video(p),
            EncodedMediaFacts::Mp4 {
                width,
                height,
                frames,
                timebase,
            },
        ) => {
            if (width, height) != (p.width, p.height)
                || !duration_in(timebase, p.accepted_duration_min, p.accepted_duration_max)
                || optional_u64_mismatch(metadata, "width", u64::from(width))
                || optional_u64_mismatch(metadata, "height", u64::from(height))
                || optional_u64_mismatch(metadata, "num_frames", frames)
                || optional_number_mismatch(
                    metadata,
                    "duration",
                    timebase.ticks as f64 / f64::from(timebase.ticks_per_second),
                )
                || optional_number_mismatch(
                    metadata,
                    "fps",
                    frames as f64 * f64::from(timebase.ticks_per_second) / timebase.ticks as f64,
                )
            {
                return Err(mismatch());
            }
            timebase
                .milliseconds_ceil()
                .map_err(|r| error(FalMediaReason::Container(r), Some(id)))
        }
        (
            ExpectedMedia::Sound(p, loops),
            EncodedMediaFacts::PcmWave {
                sample_rate,
                channels,
                bits_per_sample,
                frames,
                timebase,
            },
        ) => {
            if (sample_rate, channels, bits_per_sample)
                != (p.sample_rate, p.channels, p.bits_per_sample)
                || !duration_in(timebase, p.accepted_duration_min, p.accepted_duration_max)
                || loops.is_some_and(|v| v.first >= v.end_exclusive || v.end_exclusive > frames)
                || optional_number_mismatch(
                    metadata,
                    "duration",
                    timebase.ticks as f64 / f64::from(timebase.ticks_per_second),
                )
            {
                return Err(mismatch());
            }
            timebase
                .milliseconds_ceil()
                .map_err(|r| error(FalMediaReason::Container(r), Some(id)))
        }
        _ => Err(mismatch()),
    }
}

fn optional_u64_mismatch(value: &Value, field: &str, expected: u64) -> bool {
    value
        .get(field)
        .is_some_and(|v| v.as_u64() != Some(expected))
}
fn optional_number_mismatch(value: &Value, field: &str, expected: f64) -> bool {
    value.get(field).is_some_and(|v| {
        v.as_f64()
            .is_none_or(|observed| !observed.is_finite() || observed != expected)
    })
}
fn bounded_dimensions(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= 8192
        && height <= 8192
        && u64::from(width) * u64::from(height) <= 16_777_216
}
fn valid_interval(minimum: Duration, maximum: Duration) -> bool {
    !minimum.is_zero() && minimum <= maximum && maximum <= Duration::from_secs(180)
}
fn duration_in(time: MediaTimebase, minimum: Duration, maximum: Duration) -> bool {
    let actual = u128::from(time.ticks) * 1_000_000_000;
    let scale = u128::from(time.ticks_per_second);
    scale > 0 && actual >= minimum.as_nanos() * scale && actual <= maximum.as_nanos() * scale
}
fn validate_prompt(prompt: &str, maximum: usize) -> Result<(), FalMediaError> {
    if prompt.trim().is_empty() || prompt.len() > maximum * 4 || prompt.chars().count() > maximum {
        return Err(error(FalMediaReason::Payload, None));
    }
    Ok(())
}
fn validate_declared<Semantic>(
    request: &CheckedModalityRequest<Semantic>,
    duration: Duration,
    unit: UsageUnit,
) -> Result<(), FalMediaError> {
    let usage = request.usage();
    if usage.tokens() != 0
        || usage.duration() != duration
        || usage.usage().unit() != unit
        || usage.usage().quantity() != duration.as_millis()
        || request.limits().max_usage().unit() != unit
    {
        return Err(error(FalMediaReason::Profile, None));
    }
    Ok(())
}
fn error(reason: FalMediaReason, id: Option<&ProviderRequestId>) -> FalMediaError {
    FalMediaError {
        reason,
        request_id: id.cloned(),
        observed_usage: None,
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn confined_url(
    value: &Value,
    field: &str,
    prefix: &str,
    suffix: &str,
    id: &ProviderRequestId,
) -> Result<String, FalMediaError> {
    let url = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| error(FalMediaReason::QueueUrls, Some(id)))?;
    if url != format!("{prefix}/{suffix}") {
        return Err(error(FalMediaReason::QueueUrls, Some(id)));
    }
    Ok(url.to_owned())
}
fn queue_path(url: &str) -> Result<&str, FalMediaError> {
    url.strip_prefix("https://queue.fal.run")
        .ok_or_else(|| error(FalMediaReason::QueueUrls, None))
}
fn parse_json(
    response: &NativeHttpResponse,
    id: Option<&ProviderRequestId>,
) -> Result<Value, FalMediaError> {
    parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: 16_777_216,
            maximum_depth: 32,
            maximum_nodes: 65_536,
        },
    )
    .map_err(|_| {
        error(
            FalMediaReason::Json,
            id.or(response.transport_request_id.as_ref()),
        )
    })
}
impl FalMediaCompletion {
    pub fn into_image_event(
        self,
    ) -> Result<df_provider_api::ImageEvent<ProviderRequestId, Vec<EncodedFalMedia>>, FalMediaError>
    {
        if self.usage.unit() != UsageUnit::Image {
            return Err(error(FalMediaReason::Profile, Some(&self.request_id)));
        }
        Ok(df_provider_api::ProviderEvent {
            provider_request_id: df_provider_api::ProviderRequestId(self.request_id),
            usage: self.usage,
            kind: df_provider_api::ImageEventKind::Completed {
                identity: self.identity,
                output: self.outputs,
            },
        })
    }
    pub fn into_video_event(self) -> Result<FalVideoCompletionEvent, FalMediaError> {
        if self.usage.unit() != UsageUnit::VideoMillisecond {
            return Err(error(FalMediaReason::Profile, Some(&self.request_id)));
        }
        Ok(df_provider_api::ProviderEvent {
            provider_request_id: df_provider_api::ProviderRequestId(self.request_id),
            usage: self.usage,
            kind: df_provider_api::VideoEventKind::Completed {
                identity: self.identity,
                output: self.outputs,
            },
        })
    }
    pub fn into_sound_event(
        self,
    ) -> Result<df_provider_api::SoundEvent<ProviderRequestId, Vec<EncodedFalMedia>>, FalMediaError>
    {
        if self.usage.unit() != UsageUnit::AudioMillisecond {
            return Err(error(FalMediaReason::Profile, Some(&self.request_id)));
        }
        Ok(df_provider_api::ProviderEvent {
            provider_request_id: df_provider_api::ProviderRequestId(self.request_id),
            usage: self.usage,
            kind: df_provider_api::SoundEventKind::Completed {
                identity: self.identity,
                output: self.outputs,
            },
        })
    }
}
