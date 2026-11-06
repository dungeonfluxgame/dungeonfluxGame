//! Caller-driven FAL media queue and complete encoded candidate acquisition.
//! No paid retry, asset publication, codec decoding, rights or billing authority.

use std::{fmt, future::Future, net::SocketAddr, time::Duration};

use df_model::checkpoint::{AssetKind, AssetReference, ExecutionMode};
use df_provider_api::{
    CheckedModalityRequest, RequestBasis, RequestError, RequestOwnerState, SoundRequest,
    SoundSemantics, VideoRequest, VideoSemantics,
};
use df_types::{Usage, UsageUnit};
use serde_json::{Value, json};

use crate::media_container::{
    EncodedMediaFacts, MediaContainerCause, MediaTimebase, inspect_mp4, inspect_wave,
};
use crate::native_http::{
    MAXIMUM_BODY_BYTES, NativeHttp, NativeHttpResponse, is_loopback_socket, is_valid_credential,
};
use crate::native_media_fetch::{FalCdnPin, MAXIMUM_MEDIA_HEADER_BYTES, NativeMediaFetch};
use crate::output_fetch::{MAXIMUM_LOCATOR_BYTES, is_bounded_image_size, strict_fal_locator};
use crate::text_schema::{TextSchemaLimits, parse_unique_json};
use crate::{
    FalQueueObservation, HttpMethod, HttpRequest, NativeHttpCause, NativeHttpLimits,
    ProviderFailureClass, ProviderRequestId, classify_fal_status_response,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FalMediaFamily {
    Video,
    Sound,
}

impl FalMediaFamily {
    fn model(self) -> &'static str {
        match self {
            Self::Video => "fal-ai/wan-25-preview/image-to-video",
            Self::Sound => "fal-ai/stable-audio-25/text-to-audio",
        }
    }
    fn unit(self) -> UsageUnit {
        match self {
            Self::Video => UsageUnit::VideoMillisecond,
            Self::Sound => UsageUnit::AudioMillisecond,
        }
    }
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

pub type FalVideoSemantics = VideoSemantics<String, FalVideoProfile, FalReferenceFrame>;
pub type FalSoundSemantics = SoundSemantics<String, FalSoundProfile, Option<FalLoopFrames>>;
pub type FalVideoBasis = RequestBasis<FalVideoSemantics>;
pub type FalSoundBasis = RequestBasis<FalSoundSemantics>;
pub type FalVideoJob<'a> = FalMediaJob<'a, FalVideoSemantics>;
pub type FalSoundJob<'a> = FalMediaJob<'a, FalSoundSemantics>;
pub type FalVideoCompletionEvent =
    df_provider_api::VideoEvent<ProviderRequestId, (), FalQueueObservation, Vec<EncodedFalMedia>>;

pub type FalVideoRequest = VideoRequest<String, FalVideoProfile, FalReferenceFrame>;
pub type FalSoundRequest = SoundRequest<String, FalSoundProfile, Option<FalLoopFrames>>;

/// Native factory configuration. Fixture sockets are literal loopback and explicit.
pub struct FalMediaConfig {
    family: FalMediaFamily,
    fixture_api: Option<SocketAddr>,
    fixture_media: Option<SocketAddr>,
    pins: Vec<FalCdnPin>,
    /// A video endpoint other than the family's default, chosen by a validated setting.
    video_model: Option<String>,
}

/// The video endpoints the adapter knows how to request and check.
pub const KLING_O1_IMAGE_TO_VIDEO: &str = "fal-ai/kling-video/o1/image-to-video";

// Request bounds from the queue endpoints' input contracts: supplier limits, not tuning knobs.
/// The largest reference frame the video endpoints take, in bytes.
const MAXIMUM_REFERENCE_FRAME_BYTES: u64 = 10 * 1024 * 1024;
/// The largest inline (base64 data URI) reference frame, in bytes of base64 text.
const MAXIMUM_INLINE_REFERENCE_BYTES: usize = 14 * 1024 * 1024;
/// The longest video prompt, in characters.
const MAXIMUM_VIDEO_PROMPT_CHARACTERS: usize = 1500;
/// The longest sound prompt, in characters.
const MAXIMUM_SOUND_PROMPT_CHARACTERS: usize = 10_000;
/// The most diffusion steps a sound request may ask for.
const MAXIMUM_SOUND_STEPS: u32 = 250;
/// The longest clip either family may be asked for or accept.
const MAXIMUM_CLIP_DURATION: Duration = Duration::from_secs(180);

impl FalMediaConfig {
    pub fn live(family: FalMediaFamily, pins: Vec<FalCdnPin>) -> Self {
        Self {
            family,
            fixture_api: None,
            fixture_media: None,
            pins,
            video_model: None,
        }
    }

    /// Selects the video endpoint. Only the endpoints this adapter has a body and checks for are
    /// accepted; anything else refuses before a request is built.
    pub fn with_video_model(mut self, model: &str) -> Result<Self, FalMediaError> {
        if self.family != FalMediaFamily::Video
            || (model != FalMediaFamily::Video.model() && model != KLING_O1_IMAGE_TO_VIDEO)
        {
            return Err(error(FalMediaCause::Profile, None));
        }
        self.video_model = Some(model.to_owned());
        Ok(self)
    }

    /// The endpoint id requests for this configuration are sent to.
    pub fn model(&self) -> &str {
        self.video_model
            .as_deref()
            .unwrap_or_else(|| self.family.model())
    }
    pub fn loopback_fixture(
        family: FalMediaFamily,
        api: SocketAddr,
        media: SocketAddr,
    ) -> Result<Self, FalMediaError> {
        if !is_loopback_socket(api) || !is_loopback_socket(media) {
            return Err(error(FalMediaCause::Profile, None));
        }
        Ok(Self {
            family,
            fixture_api: Some(api),
            fixture_media: Some(media),
            pins: Vec::new(),
            video_model: None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FalMediaCause {
    Profile,
    Payload,
    Reference,
    QueueIdentity,
    QueueUrls,
    Json,
    Status,
    NotCompleted,
    Owner(RequestError),
    Transport(NativeHttpCause),
    Container(MediaContainerCause),
    Metadata,
    UsageOverrun,
    /// The live price lookup failed or answered something unusable; nothing was submitted.
    Pricing,
}

impl fmt::Display for FalMediaCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile => formatter.write_str("the media profile is invalid"),
            Self::Payload => formatter.write_str("the media request payload is invalid"),
            Self::Reference => formatter.write_str("a reference frame is invalid"),
            Self::QueueIdentity => formatter.write_str("the queue answer named no request id"),
            Self::QueueUrls => formatter.write_str("the queue answer named no usable URLs"),
            Self::Json => formatter.write_str("the answer is not valid JSON"),
            Self::Status => formatter.write_str("the supplier answered a failure status"),
            Self::NotCompleted => formatter.write_str("the media request has not completed"),
            Self::Owner(cause) => write!(formatter, "the request owner refused it ({cause:?})"),
            Self::Transport(cause) => write!(formatter, "the media call failed: {cause}"),
            Self::Container(cause) => write!(formatter, "the media output is unusable: {cause}"),
            Self::Metadata => formatter.write_str("the media output metadata is unusable"),
            Self::UsageOverrun => formatter.write_str("the media usage exceeded its bound"),
            Self::Pricing => formatter.write_str("the media price lookup failed"),
        }
    }
}

impl std::error::Error for FalMediaCause {}

/// Safe retained facts on rejection. Supplier identity and observed usage survive failure.
#[derive(Debug)]
pub struct FalMediaError {
    pub cause: FalMediaCause,
    pub request_id: Option<ProviderRequestId>,
    pub observed_usage: Option<Usage>,
    /// What the supplier answered, when the failure came with an HTTP response.
    pub http: Option<FalHttpFacts>,
}

impl fmt::Display for FalMediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.cause.fmt(formatter)
    }
}

/// The message is the cause's own, so the cause is not repeated as a source.
impl std::error::Error for FalMediaError {}

/// The supplier's HTTP answer to a failed call: the status and a bounded excerpt of the body with
/// the credential, long opaque tokens (an echoed image) and control characters removed. It is safe
/// to log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FalHttpFacts {
    pub status: u16,
    pub body: String,
}

/// The longest body excerpt kept, in characters.
const MAXIMUM_FAILURE_BODY_CHARACTERS: usize = 400;
/// A run of this many token characters is opaque data (a key, a data URI) and is not kept.
const OPAQUE_RUN_CHARACTERS: usize = 32;

fn http_facts(response: &NativeHttpResponse, credential: &str) -> FalHttpFacts {
    let mut text = String::from_utf8_lossy(&response.body).into_owned();
    if !credential.is_empty() {
        text = text.replace(credential, "[credential]");
    }
    let mut kept = String::new();
    let mut run = String::new();
    fn flush(run: &mut String, kept: &mut String) {
        if run.chars().count() >= OPAQUE_RUN_CHARACTERS {
            kept.push_str("[opaque]");
        } else {
            kept.push_str(run);
        }
        run.clear();
    }
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=' | '_' | '-') {
            run.push(character);
        } else {
            flush(&mut run, &mut kept);
            kept.push(if character.is_control() {
                ' '
            } else {
                character
            });
        }
    }
    flush(&mut run, &mut kept);
    FalHttpFacts {
        status: response.status,
        body: kept.chars().take(MAXIMUM_FAILURE_BODY_CHARACTERS).collect(),
    }
}

impl FalMediaError {
    fn with_http(mut self, facts: &FalHttpFacts) -> Self {
        self.http = Some(facts.clone());
        self
    }

    pub fn failure_class(&self) -> ProviderFailureClass {
        match self.cause {
            FalMediaCause::Owner(RequestError::Cancelled) => ProviderFailureClass::Cancelled,
            FalMediaCause::Owner(RequestError::DeadlineExceeded) => ProviderFailureClass::Deadline,
            FalMediaCause::Transport(cause) => cause.failure_class(),
            FalMediaCause::Profile
            | FalMediaCause::Payload
            | FalMediaCause::Reference
            | FalMediaCause::Owner(_) => ProviderFailureClass::InvalidRequest,
            FalMediaCause::Status | FalMediaCause::QueueIdentity | FalMediaCause::QueueUrls => {
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
    /// Inspect a retained video with the same bounded container checks as acquired outputs.
    /// This does not decode frames, contact the supplier or mint an asset reference.
    pub fn inspect_video(bytes: &[u8]) -> Result<EncodedMediaFacts, FalMediaError> {
        inspect_mp4(bytes).map_err(|cause| error(FalMediaCause::Container(cause), None))
    }

    pub(crate) fn new(
        config: FalMediaConfig,
        credential: &str,
        limits: NativeHttpLimits,
    ) -> Result<Self, FalMediaError> {
        if !is_valid_credential(credential)
            || limits.maximum_header_bytes > MAXIMUM_MEDIA_HEADER_BYTES
        {
            return Err(error(FalMediaCause::Profile, None));
        }
        let http = NativeHttp::new(limits, config.fixture_api.is_some())
            .map_err(|failure| error(FalMediaCause::Transport(failure.cause), None))?;
        let fetch = NativeMediaFetch::new(limits, config.pins.clone(), config.fixture_media)
            .map_err(|cause| error(FalMediaCause::Transport(cause), None))?;
        Ok(Self {
            config,
            credential: credential.to_owned(),
            http,
            fetch,
        })
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
            || p.model != self.config.model()
            || !matches!(p.duration_seconds, 5 | 10)
            || resolution_side == 0
            || p.width.min(p.height) != resolution_side
            || !is_bounded_image_size(p.width, p.height)
            || !valid_interval(p.accepted_duration_min, p.accepted_duration_max)
        {
            return Err(error(FalMediaCause::Profile, None));
        }
        let frames = semantic.reference_frames.as_slice();
        let Some(frame) = frames.first() else {
            return Err(error(FalMediaCause::Reference, None));
        };
        if frames.len() > 2
            || (frames.len() == 2 && p.model != KLING_O1_IMAGE_TO_VIDEO)
            || frames.iter().any(|frame| {
                frame.asset.kind != AssetKind::Image
                    || frame.asset.byte_length == 0
                    || frame.asset.byte_length > MAXIMUM_REFERENCE_FRAME_BYTES
                    || !reference_locator_ok(&frame.locator)
            })
        {
            return Err(error(FalMediaCause::Reference, None));
        }
        validate_prompt(&semantic.prompt, MAXIMUM_VIDEO_PROMPT_CHARACTERS)?;
        validate_declared(
            request.checked(),
            Duration::from_secs(u64::from(p.duration_seconds)),
            UsageUnit::VideoMillisecond,
        )?;
        // One reference loops back to itself; two references supply distinct bookends.
        // Kling's body has no resolution or seed fields.
        let body = if p.model == KLING_O1_IMAGE_TO_VIDEO {
            let end = frames
                .last()
                .ok_or_else(|| error(FalMediaCause::Reference, None))?;
            json!({
                "prompt": semantic.prompt,
                "start_image_url": frame.locator,
                "end_image_url": end.locator,
                "duration": p.duration_seconds.to_string(),
            })
        } else {
            json!({
                "prompt": semantic.prompt,
                "image_url": frame.locator,
                "duration": p.duration_seconds.to_string(),
                "resolution": p.resolution,
                "seed": p.seed,
                "enable_safety_checker": p.safety,
                "enable_prompt_expansion": p.prompt_expansion,
            })
        };
        self.submit(
            request.checked(),
            owner,
            body,
            ExpectedMedia::Video(p.clone()),
            cancellation,
        )
        .await
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
            || u64::from(p.duration_seconds) > MAXIMUM_CLIP_DURATION.as_secs()
            || p.steps == 0
            || p.steps > MAXIMUM_SOUND_STEPS
            || !(8000..=192000).contains(&p.sample_rate)
            || !matches!(p.channels, 1 | 2)
            || !matches!(p.bits_per_sample, 8 | 16 | 24 | 32)
            || !valid_interval(p.accepted_duration_min, p.accepted_duration_max)
            || semantic
                .loop_metadata
                .is_some_and(|v| v.first >= v.end_exclusive)
        {
            return Err(error(FalMediaCause::Profile, None));
        }
        validate_prompt(&semantic.intent, MAXIMUM_SOUND_PROMPT_CHARACTERS)?;
        validate_declared(
            request.checked(),
            Duration::from_secs(u64::from(p.duration_seconds)),
            UsageUnit::AudioMillisecond,
        )?;
        let body = json!({
            "prompt": semantic.intent,
            "seconds_total": p.duration_seconds,
            "num_inference_steps": p.steps,
            "seed": p.seed,
            "enable_safety_checker": p.safety,
        });
        let expected = ExpectedMedia::Sound(p.clone(), semantic.loop_metadata);
        self.submit(request.checked(), owner, body, expected, cancellation)
            .await
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
            .map_err(|r| error(FalMediaCause::Owner(r), None))?;
        if request.binding().mode != ExecutionMode::Live {
            return Err(error(FalMediaCause::Profile, None));
        }
        let serialized =
            serde_json::to_vec(&body).map_err(|_| error(FalMediaCause::Payload, None))?;
        // The checked payload must be exactly the admitted typed request serialization.
        if !request.payload().is_empty() && request.payload() != serialized {
            return Err(error(FalMediaCause::Payload, None));
        }
        let path = format!("/{}", self.config.model());
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
        let http = http_facts(&response, &self.credential);
        // A refused submit (a bad key, a rejected field, a server fault) is a status failure
        // whatever its body, so it is reported as one with the supplier's own words, not as a
        // queue reply that lost its identity.
        if !matches!(response.status, 200 | 202) {
            let named = parse_json(&response, None)
                .ok()
                .and_then(|value| request_id_of(&value));
            return Err(error(
                FalMediaCause::Status,
                named.as_ref().or(response.transport_request_id.as_ref()),
            )
            .with_http(&http));
        }
        let value = parse_json(&response, None).map_err(|failure| failure.with_http(&http))?;
        let id = request_id_of(&value).ok_or_else(|| {
            error(
                FalMediaCause::QueueIdentity,
                response.transport_request_id.as_ref(),
            )
            .with_http(&http)
        })?;
        // fal names a request under its endpoint's path, or under the owner and app that hold it
        // (`fal-ai/kling-video/requests/{id}` for `fal-ai/kling-video/o1/image-to-video`). Either
        // is this request on the queue host, so either is accepted; anything else is refused.
        let status = confined_url(&value, "status_url", Some("status"), &id)?;
        let result = confined_url(&value, "response_url", None, &id)?;
        let cancel = confined_url(&value, "cancel_url", Some("cancel"), &id)?;
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

    /// The live unit price of a video endpoint in USD per second, from fal's pricing API. Read
    /// only: it sends no queue request and spends nothing. Anything but a USD per-second price
    /// for this endpoint is a `Pricing` refusal.
    pub async fn video_price_per_second(
        &self,
        deadline: Duration,
        cancellation: impl Future<Output = ()>,
    ) -> Result<f64, FalMediaError> {
        let model = self.config.model();
        let origin = self.config.fixture_api.map_or_else(
            || "https://api.fal.ai".to_owned(),
            |a| format!("http://{a}"),
        );
        let request = HttpRequest::fal_native(
            HttpMethod::Get,
            &format!("{origin}/v1/models/pricing?endpoint_id={model}"),
            &self.credential,
            Vec::new(),
        );
        let response = self
            .http
            .execute(request, deadline, cancellation)
            .await
            .map_err(|failure| error(FalMediaCause::Transport(failure.cause), None))?;
        let value = parse_json(&response, None)?;
        let price = value
            .get("prices")
            .and_then(Value::as_array)
            .and_then(|prices| {
                prices.iter().find(|entry| {
                    entry.get("endpoint_id").and_then(Value::as_str) == Some(model)
                        && entry.get("unit").and_then(Value::as_str) == Some("seconds")
                        && entry.get("currency").and_then(Value::as_str) == Some("USD")
                })
            })
            .and_then(|entry| entry.get("unit_price"))
            .and_then(Value::as_f64)
            .filter(|price| price.is_finite() && *price > 0.0);
        price.ok_or_else(|| error(FalMediaCause::Pricing, None))
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
                let mut value = error(FalMediaCause::Transport(failure.cause), id);
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
            .map_err(|r| error(FalMediaCause::Owner(r), Some(&self.id)))?;
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
        let http = http_facts(&response, &self.provider.credential);
        let value =
            parse_json(&response, Some(&self.id)).map_err(|failure| failure.with_http(&http))?;
        if value
            .get("response_url")
            .is_some_and(|v| v.as_str() != Some(self.result.as_str()))
        {
            return Err(error(FalMediaCause::QueueUrls, Some(&self.id)));
        }
        let metric = value.get("metrics").and_then(|v| v.get("inference_time"));
        if metric.is_some_and(|v| !v.is_number()) {
            return Err(error(FalMediaCause::Json, Some(&self.id)));
        }
        for field in ["error", "error_type"] {
            if value
                .get(field)
                .is_some_and(|v| !v.is_null() && !v.is_string())
            {
                return Err(error(FalMediaCause::Json, Some(&self.id)));
            }
        }
        // A status body may omit the request id; the status URL is already confined to this
        // request, so an absent id is this request. A different id is still refused.
        let observed_id = value
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_else(|| self.id.as_str());
        let observation = classify_fal_status_response(
            response.status,
            &self.id,
            Some(observed_id),
            value.get("status").and_then(Value::as_str),
            value.get("error_type").and_then(Value::as_str),
            value.get("error").and_then(Value::as_str),
            metric.and_then(Value::as_f64),
        )
        .map_err(|_| error(FalMediaCause::Status, Some(&self.id)).with_http(&http))?;
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
            return Err(error(FalMediaCause::QueueIdentity, Some(&self.id)));
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
            return Err(error(FalMediaCause::NotCompleted, Some(&self.id)));
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
        let http = http_facts(&response, &self.provider.credential);
        if response.status != 200 {
            return Err(error(FalMediaCause::Status, Some(&self.id)).with_http(&http));
        }
        let value =
            parse_json(&response, Some(&self.id)).map_err(|failure| failure.with_http(&http))?;
        if value
            .get("request_id")
            .is_some_and(|v| v.as_str() != Some(self.id.as_str()))
            || value.get("error").is_some()
            || value.get("error_type").is_some()
        {
            return Err(error(FalMediaCause::QueueIdentity, Some(&self.id)).with_http(&http));
        }
        let files = output_files(&value, &self.expected, &self.id)?;
        let maximum = self.provider.http.limits().maximum_response_bytes;
        let mut retained = response.body.len();
        let mut outputs = Vec::with_capacity(files.len());
        let mut quantity = 0u128;
        for file in files {
            let elapsed = owner.elapsed.checked_add(start.elapsed()).ok_or_else(|| {
                error(
                    FalMediaCause::Owner(RequestError::DeadlineExceeded),
                    Some(&self.id),
                )
            })?;
            self.request
                .validate_current(RequestOwnerState {
                    current: owner.current,
                    elapsed,
                    cancelled: owner.cancelled,
                })
                .map_err(|r| error(FalMediaCause::Owner(r), Some(&self.id)))?;
            let remaining = maximum.checked_sub(retained).ok_or_else(|| {
                error(
                    FalMediaCause::Transport(NativeHttpCause::ResponseTooLarge),
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
                .map_err(|cause| error(FalMediaCause::Transport(cause), Some(&self.id)))?;
            if file
                .metadata
                .get("file_size")
                .is_some_and(|v| v.as_u64() != Some(body.len() as u64))
            {
                return Err(error(FalMediaCause::Metadata, Some(&self.id)));
            }
            retained = retained.checked_add(body.len()).ok_or_else(|| {
                error(
                    FalMediaCause::Transport(NativeHttpCause::ResponseTooLarge),
                    Some(&self.id),
                )
            })?;
            let (body, facts) = match self.expected {
                ExpectedMedia::Video(_) => inspect_mp4(&body).map(|facts| (body, facts)),
                ExpectedMedia::Sound(_, _) => inspect_wave(&body).map(|facts| (body, facts)),
            }
            .map_err(|cause| error(FalMediaCause::Container(cause), Some(&self.id)))?;
            let units = validate_output(&self.expected, &file.metadata, facts, &self.id)?;
            quantity = quantity
                .checked_add(units)
                .ok_or_else(|| error(FalMediaCause::UsageOverrun, Some(&self.id)))?;
            outputs.push(EncodedFalMedia { bytes: body, facts });
        }
        let usage = Usage::new(quantity, self.provider.config.family.unit());
        if quantity > self.request.limits().max_usage().quantity() {
            return Err(FalMediaError {
                cause: FalMediaCause::UsageOverrun,
                request_id: Some(self.id),
                observed_usage: Some(usage),
                http: None,
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
    /// Fresh current-owner validation is required after all network work and before output
    /// consumption.
    pub fn complete(
        self,
        owner: RequestOwnerState<'_, RequestBasis<Semantic>>,
    ) -> Result<FalMediaCompletion, FalMediaError> {
        self.request
            .validate_current(owner)
            .map_err(|cause| FalMediaError {
                cause: FalMediaCause::Owner(cause),
                request_id: Some(self.id.clone()),
                observed_usage: Some(self.usage),
                http: None,
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
    Video(FalVideoProfile),
    Sound(FalSoundProfile, Option<FalLoopFrames>),
}
impl ExpectedMedia {
    fn mime(&self) -> &'static str {
        match self {
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
        ExpectedMedia::Video(_) => vec![
            value
                .get("video")
                .ok_or_else(|| error(FalMediaCause::Metadata, Some(id)))?,
        ],
        ExpectedMedia::Sound(_, _) => vec![
            value
                .get("audio")
                .ok_or_else(|| error(FalMediaCause::Metadata, Some(id)))?,
        ],
    };
    let mut files = Vec::with_capacity(items.len());
    for item in items {
        let locator = item
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| error(FalMediaCause::Metadata, Some(id)))?;
        if !item.is_object()
            || locator.len() > MAXIMUM_LOCATOR_BYTES
            || !strict_fal_locator(locator)
            || item.get("content_type").and_then(Value::as_str) != Some(expected.mime())
            || files
                .iter()
                .any(|file: &OutputFile| file.locator == locator)
        {
            return Err(error(FalMediaCause::Metadata, Some(id)));
        }
        files.push(OutputFile {
            locator: locator.to_owned(),
            metadata: item.clone(),
        });
    }
    Ok(files)
}

/// Whether two frame sizes have the same aspect ratio, within two percent, and the first is
/// at least 320 pixels on its short side.
fn same_frame_shape(actual: (u32, u32), requested: (u32, u32)) -> bool {
    let (width, height) = (u64::from(actual.0), u64::from(actual.1));
    let (requested_width, requested_height) = (u64::from(requested.0), u64::from(requested.1));
    if width.min(height) < 320 || requested_width == 0 || requested_height == 0 {
        return false;
    }
    // width / height against requested_width / requested_height, cross-multiplied.
    let left = width * requested_height;
    let right = height * requested_width;
    left.abs_diff(right) * 50 <= right
}

fn validate_output(
    expected: &ExpectedMedia,
    metadata: &Value,
    facts: EncodedMediaFacts,
    id: &ProviderRequestId,
) -> Result<u128, FalMediaError> {
    let mismatch = || error(FalMediaCause::Metadata, Some(id));
    match (expected, facts) {
        (
            ExpectedMedia::Video(p),
            EncodedMediaFacts::Mp4 {
                width,
                height,
                frames,
                timebase,
            },
        ) => {
            // The supplier may return a smaller or larger frame than the profile asks for (720p
            // for a 1080p request): the shape must match, not the exact size.
            if !same_frame_shape((width, height), (p.width, p.height))
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
                .map_err(|failure| error(FalMediaCause::Container(failure.cause), Some(id)))
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
                .map_err(|failure| error(FalMediaCause::Container(failure.cause), Some(id)))
        }
        _ => Err(mismatch()),
    }
}

/// A reference frame is a fal.media locator (at most 8192 bytes), or an inline JPEG or PNG the
/// queue accepts as a data URI. The inline form is bounded by the frame's declared byte length,
/// and it is base64 only, so nothing else can ride in the field.
fn reference_locator_ok(locator: &str) -> bool {
    let inline = locator
        .strip_prefix("data:image/jpeg;base64,")
        .or_else(|| locator.strip_prefix("data:image/png;base64,"));
    match inline {
        Some(data) => {
            !data.is_empty()
                && data.len() <= MAXIMUM_INLINE_REFERENCE_BYTES
                && data
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        }
        None => locator.len() <= MAXIMUM_LOCATOR_BYTES && strict_fal_locator(locator),
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
fn valid_interval(minimum: Duration, maximum: Duration) -> bool {
    !minimum.is_zero() && minimum <= maximum && maximum <= MAXIMUM_CLIP_DURATION
}
fn duration_in(time: MediaTimebase, minimum: Duration, maximum: Duration) -> bool {
    let actual = u128::from(time.ticks) * 1_000_000_000;
    let scale = u128::from(time.ticks_per_second);
    scale > 0 && actual >= minimum.as_nanos() * scale && actual <= maximum.as_nanos() * scale
}
fn validate_prompt(prompt: &str, maximum: usize) -> Result<(), FalMediaError> {
    if prompt.trim().is_empty() || prompt.len() > maximum * 4 || prompt.chars().count() > maximum {
        return Err(error(FalMediaCause::Payload, None));
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
        return Err(error(FalMediaCause::Profile, None));
    }
    Ok(())
}
fn error(cause: FalMediaCause, id: Option<&ProviderRequestId>) -> FalMediaError {
    FalMediaError {
        cause,
        request_id: id.cloned(),
        observed_usage: None,
        http: None,
    }
}
/// The queue request id a reply names, when it is a valid one.
fn request_id_of(value: &Value) -> Option<ProviderRequestId> {
    value
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| valid_id(id))
        .and_then(|id| ProviderRequestId::new(id).ok())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
/// Whether `url` is this request on fal's queue host:
/// `https://queue.fal.run/<owner>/<app>[/...]/requests/<id>`, then nothing or the expected
/// `suffix`. Any query, fragment or traversal, any other host, and any other request id is
/// refused. `suffix` of `None` is the result, which fal names as the request itself or as
/// `.../response`.
fn queue_url_confined(url: &str, id: &ProviderRequestId, suffix: Option<&str>) -> bool {
    let Some(tail) = url.strip_prefix("https://queue.fal.run/") else {
        return false;
    };
    if tail.contains(['?', '#']) || tail.contains("..") {
        return false;
    }
    let needle = format!("/requests/{}", id.as_str());
    let Some(position) = tail.rfind(&needle) else {
        return false;
    };
    let (app, rest) = (&tail[..position], &tail[position + needle.len()..]);
    let is_app = app.split('/').count() >= 2 && app.split('/').all(|part| !part.is_empty());
    let rest_ok = match suffix {
        Some(expected) => rest == format!("/{expected}"),
        None => rest.is_empty() || rest == "/response",
    };
    is_app && rest_ok
}

/// The queue URL a reply names in `field`, when it is confined to this request (see
/// [`queue_url_confined`]; a `suffix` of `None` is the result URL).
fn confined_url(
    value: &Value,
    field: &str,
    suffix: Option<&str>,
    id: &ProviderRequestId,
) -> Result<String, FalMediaError> {
    let url = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| error(FalMediaCause::QueueUrls, Some(id)))?;
    if !queue_url_confined(url, id, suffix) {
        return Err(error(FalMediaCause::QueueUrls, Some(id)));
    }
    Ok(url.to_owned())
}
fn queue_path(url: &str) -> Result<&str, FalMediaError> {
    url.strip_prefix("https://queue.fal.run")
        .ok_or_else(|| error(FalMediaCause::QueueUrls, None))
}
fn parse_json(
    response: &NativeHttpResponse,
    id: Option<&ProviderRequestId>,
) -> Result<Value, FalMediaError> {
    parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: MAXIMUM_BODY_BYTES,
            maximum_depth: 32,
            maximum_nodes: 65_536,
        },
    )
    .map_err(|_| {
        error(
            FalMediaCause::Json,
            id.or(response.transport_request_id.as_ref()),
        )
    })
}
impl FalMediaCompletion {
    pub fn into_video_event(self) -> Result<FalVideoCompletionEvent, FalMediaError> {
        if self.usage.unit() != UsageUnit::VideoMillisecond {
            return Err(error(FalMediaCause::Profile, Some(&self.request_id)));
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
            return Err(error(FalMediaCause::Profile, Some(&self.request_id)));
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

#[cfg(test)]
mod frame_shape_tests {
    use super::same_frame_shape;

    #[test]
    fn a_smaller_frame_of_the_requested_shape_is_accepted_and_another_shape_is_not() {
        assert!(same_frame_shape((1920, 1080), (1920, 1080)));
        assert!(
            same_frame_shape((1280, 720), (1920, 1080)),
            "720p for a 1080p request"
        );
        assert!(
            same_frame_shape((1920, 1088), (1920, 1080)),
            "a codec's padded height"
        );
        assert!(
            !same_frame_shape((640, 480), (1920, 1080)),
            "4:3 is another shape"
        );
        assert!(
            !same_frame_shape((320, 180), (1920, 1080)),
            "too small to show"
        );
        assert!(
            !same_frame_shape((1080, 1920), (1920, 1080)),
            "portrait is another shape"
        );
        assert!(!same_frame_shape((1280, 720), (0, 0)));
    }
}
