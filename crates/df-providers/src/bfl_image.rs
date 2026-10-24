//! Direct FLUX 3 Image. One submit, read-only polling, credential-free pinned delivery.
//! Schema: https://docs.bfl.ai/api-reference/utility/generate-an-image-with-flux-3
//! Observed 2026-10-23. Hosted moderation stays at its default; no refusal fallback.
use crate::media_container::{EncodedMediaFacts, inspect_png};
use crate::native_http::{NativeHttp, NativeHttpResponse};
use crate::native_media_fetch::NativeMediaFetch;
use crate::text_schema::{TextSchemaLimits, parse_unique_json};
use crate::{
    GeneratedImage, HttpMethod, HttpRequest, ImageBackground, ImageEdit, ImageEditError,
    ImageFormat, ImageUsage, NativeHttpRefusal, ProviderLimits,
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

pub const BFL_FLUX3_IMAGE: &str = "flux-3-image";
const MAXIMUM_BYTES: usize = 6 * 1024 * 1024;
const INTERVAL: Duration = Duration::from_millis(500);
pub struct BflImageConfig {
    fixture: Option<SocketAddr>,
}
impl BflImageConfig {
    pub fn live() -> Self {
        Self { fixture: None }
    }
    pub fn loopback_fixture(address: SocketAddr) -> Result<Self, ImageEditError> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(ImageEditError::InvalidRequest);
        }
        Ok(Self {
            fixture: Some(address),
        })
    }
}
/// Closed boundary diagnostics; never includes response bodies, URLs or credentials.
#[derive(Debug)]
pub struct BflImageFailure {
    pub stage: &'static str,
    pub http_status: Option<u16>,
    pub delivery_host: Option<String>,
    pub admission_reason: Option<&'static str>,
    pub error: ImageEditError,
}
pub struct NativeBflImageProvider {
    origin: String,
    credential: String,
    http: NativeHttp,
    fixture: Option<SocketAddr>,
}
impl std::fmt::Debug for NativeBflImageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeBflImageProvider")
            .finish_non_exhaustive()
    }
}
impl NativeBflImageProvider {
    pub fn new(
        config: BflImageConfig,
        credential: &str,
        total_timeout: Duration,
    ) -> Result<Self, ImageEditError> {
        if credential.is_empty()
            || credential.len() > 4096
            || !credential.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err(ImageEditError::InvalidRequest);
        }
        let http = NativeHttp::new(
            ProviderLimits::new(total_timeout, 64 * 1024, 64 * 1024).build(),
            config.fixture.is_some(),
        )
        .map_err(ImageEditError::Transport)?;
        Ok(Self {
            origin: config.fixture.map_or_else(
                || "https://api.bfl.ai".into(),
                |address| format!("http://{address}"),
            ),
            credential: credential.into(),
            http,
            fixture: config.fixture,
        })
    }
    /// Only the opaque creator portrait profile is admitted. No reference upload or search.
    pub async fn edit(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<(GeneratedImage, Option<String>), ImageEditError> {
        self.edit_diagnosed(edit, deadline)
            .await
            .map_err(|failure| failure.error)
    }
    pub async fn edit_diagnosed(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
    ) -> Result<(GeneratedImage, Option<String>), BflImageFailure> {
        let mut stage = "validation";
        let mut http_status = None;
        let mut delivery_host = None;
        let mut admission_reason = None;
        self.edit_inner(
            edit,
            deadline,
            &mut stage,
            &mut http_status,
            &mut delivery_host,
            &mut admission_reason,
        )
        .await
        .map_err(|error| BflImageFailure {
            stage,
            http_status,
            delivery_host,
            admission_reason,
            error,
        })
    }
    async fn edit_inner(
        &self,
        edit: &ImageEdit<'_>,
        deadline: Duration,
        stage: &mut &'static str,
        http_status: &mut Option<u16>,
        delivery_host: &mut Option<String>,
        admission_reason: &mut Option<&'static str>,
    ) -> Result<(GeneratedImage, Option<String>), ImageEditError> {
        edit.validate()?;
        if !edit.references.is_empty()
            || edit.background != ImageBackground::Opaque
            || edit.width != 1024
            || edit.height != 1536
        {
            return Err(ImageEditError::InvalidRequest);
        }
        let started = Instant::now();
        let remaining = || deadline.saturating_sub(started.elapsed());
        let body = generation_body(edit.prompt);
        *stage = "submit";
        let submit = self
            .exchange(
                HttpMethod::Post,
                &format!("{}/v1/flux-3-image", self.origin),
                body.to_string().into_bytes(),
                remaining(),
            )
            .await?;
        *stage = "submit_response";
        let submitted = json_body(&submit)?;
        let credits = submitted
            .get("cost")
            .and_then(Value::as_number)
            .map(ToString::to_string)
            .filter(|value| value.len() <= 32 && !value.starts_with('-'));
        let poll = submitted
            .get("polling_url")
            .and_then(Value::as_str)
            .ok_or(ImageEditError::Output)?;
        if !poll_allowed(poll, self.fixture) {
            return Err(ImageEditError::Output);
        }
        loop {
            *stage = "poll";
            let response = self
                .exchange(HttpMethod::Get, poll, vec![], remaining())
                .await?;
            *stage = "poll_response";
            let value = json_body(&response)?;
            match status(&value)? {
                Status::Pending => {
                    let pause = INTERVAL.min(remaining());
                    if pause.is_zero() {
                        return Err(ImageEditError::Transport(NativeHttpRefusal::Deadline));
                    }
                    tokio::time::sleep(pause).await;
                }
                Status::Ready => {
                    let locator = value
                        .get("result")
                        .and_then(|result| result.get("sample"))
                        .and_then(Value::as_str)
                        .ok_or(ImageEditError::Output)?;
                    *delivery_host = reqwest::Url::parse(locator).ok().and_then(|url| {
                        url.host_str()
                            .filter(|host| host.ends_with(".bfl.ai") && host.len() <= 253)
                            .map(str::to_owned)
                    });
                    *stage = "delivery_admission";
                    let fetch = NativeMediaFetch::bfl(
                        ProviderLimits::new(
                            remaining().min(Duration::from_secs(30)),
                            1,
                            MAXIMUM_BYTES,
                        )
                        .build(),
                        locator,
                        self.fixture,
                    )
                    .await
                    .map_err(|(error, reason)| {
                        *admission_reason = Some(reason);
                        ImageEditError::Transport(error)
                    })?;
                    *stage = "download";
                    let (bytes, mime) = fetch
                        .fetch_bfl_image(locator, MAXIMUM_BYTES, remaining())
                        .await
                        .map_err(|error| {
                            *http_status = fetch.observed_status();
                            ImageEditError::Transport(error)
                        })?;
                    *stage = "image_validation";
                    let (width, height, format, has_alpha_channel) = if mime == "image/png" {
                        let EncodedMediaFacts::Png { width, height } =
                            inspect_png(&bytes).map_err(|_| ImageEditError::Output)?
                        else {
                            return Err(ImageEditError::Output);
                        };
                        (
                            width,
                            height,
                            ImageFormat::Png,
                            matches!(bytes.get(25), Some(4 | 6)),
                        )
                    } else {
                        let (width, height) =
                            crate::image_edit::jpeg_size(&bytes).ok_or(ImageEditError::Output)?;
                        (width, height, ImageFormat::Jpeg, false)
                    };
                    if width > 4096
                        || height > 4096
                        || u64::from(width) * u64::from(height) > 4_194_304
                        || width * 3 != height * 2
                    {
                        return Err(ImageEditError::Output);
                    }
                    return Ok((
                        GeneratedImage {
                            bytes,
                            format,
                            width,
                            height,
                            has_alpha_channel,
                            usage: ImageUsage::default(),
                        },
                        credits,
                    ));
                }
            }
        }
    }
    async fn exchange(
        &self,
        method: HttpMethod,
        url: &str,
        body: Vec<u8>,
        remaining: Duration,
    ) -> Result<NativeHttpResponse, ImageEditError> {
        let response = self
            .http
            .execute(
                HttpRequest::bfl_native(method, url, &self.credential, body),
                remaining,
                std::future::pending(),
            )
            .await
            .map_err(|failure| ImageEditError::Transport(failure.reason))?;
        if matches!(response.status, 401 | 403) {
            return Err(ImageEditError::Unauthorized);
        }
        // Moderated tasks may carry a 503. Inspect that terminal state before HTTP handling.
        if matches!(response.status, 200 | 503)
            && let Ok(value) = json_body(&response)
            && matches!(
                value.get("status").and_then(Value::as_str),
                Some("Request Moderated" | "Content Moderated")
            )
        {
            return Err(ImageEditError::Refused);
        }
        if response.status != 200 {
            return Err(ImageEditError::Provider(response.status));
        }
        Ok(response)
    }
}
fn generation_body(prompt: &str) -> Value {
    json!({"prompt":prompt,"aspect_ratio":"2:3","resolution":"1.5k","grounding":false,"safety_tolerance":2,"version":"latest"})
}
fn poll_allowed(locator: &str, fixture: Option<SocketAddr>) -> bool {
    let Ok(url) = reqwest::Url::parse(locator) else {
        return false;
    };
    if url.username() != ""
        || url.password().is_some()
        || url.fragment().is_some()
        || locator.contains(['\\', '\0'])
        || locator.len() > 8192
    {
        return false;
    }
    if let Some(address) = fixture {
        return url.scheme() == "http"
            && url.host_str() == Some(address.ip().to_string().as_str())
            && url.port() == Some(address.port());
    }
    url.scheme() == "https"
        && url.port().is_none()
        && url.path() == "/v1/get_result"
        && url.host_str().is_some_and(|host| {
            host == "api.bfl.ai" || (host.starts_with("api.") && host.ends_with(".bfl.ai"))
        })
}
fn json_body(response: &NativeHttpResponse) -> Result<Value, ImageEditError> {
    parse_unique_json(
        &response.body,
        TextSchemaLimits {
            maximum_schema_bytes: 64 * 1024,
            maximum_depth: 16,
            maximum_nodes: 2048,
        },
    )
    .map_err(|_| ImageEditError::Output)
}
#[derive(Debug, PartialEq)]
enum Status {
    Pending,
    Ready,
}
fn status(value: &Value) -> Result<Status, ImageEditError> {
    match value.get("status").and_then(Value::as_str) {
        Some("Pending" | "Reasoning" | "Generating") => Ok(Status::Pending),
        Some("Ready") => Ok(Status::Ready),
        Some("Request Moderated" | "Content Moderated") => Err(ImageEditError::Refused),
        Some("Error" | "Task not found") => Err(ImageEditError::Output),
        _ => Err(ImageEditError::Output),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn edit() -> ImageEdit<'static> {
        ImageEdit {
            model: crate::ImageEditModel::Gpt25Flare,
            prompt: "fictional adult adventurer",
            references: &[],
            width: 1024,
            height: 1536,
            quality: crate::ImageQuality::Medium,
            background: ImageBackground::Opaque,
            output: ImageFormat::Jpeg,
            moderation: crate::ImageModeration::Auto,
        }
    }
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(split) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..split]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= split + 4 + length {
                    return String::from_utf8(bytes).unwrap();
                }
            }
        }
    }
    fn reply(stream: &mut std::net::TcpStream, status: u16, mime: &str, bytes: &[u8]) {
        write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();
        stream.write_all(bytes).unwrap();
    }
    #[tokio::test]
    async fn moderated_job_sends_once_and_never_downloads_or_resubmits() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.starts_with("POST /v1/flux-3-image "));
            assert!(request.to_ascii_lowercase().contains("x-key: fixture-key"));
            reply(&mut stream,200,"application/json",json!({"id":"job-1","polling_url":format!("http://{address}/v1/get_result?id=job-1")}).to_string().as_bytes());
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.starts_with("GET /v1/get_result?id=job-1 "));
            reply(
                &mut stream,
                503,
                "application/json",
                br#"{"status":"Content Moderated"}"#,
            );
        });
        let provider = NativeBflImageProvider::new(
            BflImageConfig::loopback_fixture(address).unwrap(),
            "fixture-key",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            provider.edit(&edit(), Duration::from_secs(5)).await,
            Err(ImageEditError::Refused)
        );
        server.join().unwrap();
    }
    #[tokio::test]
    async fn ready_job_download_is_credential_free_and_preserves_real_png_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let payload: Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(payload["grounding"], false);
            assert_eq!(payload["safety_tolerance"], 2);
            reply(&mut stream,200,"application/json",json!({"id":"job-1","polling_url":format!("http://{address}/v1/get_result?id=job-1"),"cost":7}).to_string().as_bytes());
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.to_ascii_lowercase().contains("x-key: fixture-key"));
            reply(&mut stream,200,"application/json",br#"{"status":"Ready","result":{"sample":"https://delivery.us1.bfl.ai/image.png?signature=synthetic"}}"#);
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.starts_with("GET /image.png "));
            assert!(!request.contains("fixture-key"));
            assert!(!request.to_ascii_lowercase().contains("x-key:"));
            assert!(!request.to_ascii_lowercase().contains("authorization:"));
            let png=STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAQAAAAGACAIAAADNoCvpAAABNUlEQVR42u3BMQEAAADCoPVPbQ0PoAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAC+DIG8AAGyVa+PAAAAAElFTkSuQmCC").unwrap();
            reply(&mut stream, 200, "image/png", &png);
        });
        let provider = NativeBflImageProvider::new(
            BflImageConfig::loopback_fixture(address).unwrap(),
            "fixture-key",
            Duration::from_secs(5),
        )
        .unwrap();
        let (image, credits) = provider
            .edit(&edit(), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(credits.as_deref(), Some("7"));
        assert_eq!(
            (image.width, image.height, image.format),
            (256, 384, ImageFormat::Png)
        );
        server.join().unwrap();
    }
    #[tokio::test]
    async fn diagnostics_preserve_submit_poll_and_download_boundaries_without_resubmission() {
        for (expected, response_status) in [("submit", 402), ("poll", 429), ("download", 403)] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                assert!(read_request(&mut stream).starts_with("POST "));
                if expected == "submit" {
                    reply(&mut stream, response_status, "application/json", b"{}");
                    return;
                }
                reply(
                    &mut stream,
                    200,
                    "application/json",
                    json!({"polling_url":format!("http://{address}/v1/get_result?id=fixture")})
                        .to_string()
                        .as_bytes(),
                );
                let (mut stream, _) = listener.accept().unwrap();
                assert!(read_request(&mut stream).starts_with("GET /v1/get_result"));
                if expected == "poll" {
                    reply(&mut stream, response_status, "application/json", b"{}");
                    return;
                }
                reply(&mut stream,200,"application/json",br#"{"status":"Ready","result":{"sample":"https://delivery.us1.bfl.ai/image.png"}}"#);
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_request(&mut stream);
                assert!(!request.contains("fixture-key"));
                reply(&mut stream, response_status, "image/png", b"denied");
            });
            let provider = NativeBflImageProvider::new(
                BflImageConfig::loopback_fixture(address).unwrap(),
                "fixture-key",
                Duration::from_secs(5),
            )
            .unwrap();
            let failure = provider
                .edit_diagnosed(&edit(), Duration::from_secs(5))
                .await
                .unwrap_err();
            assert_eq!(failure.stage, expected);
            if expected == "download" {
                assert_eq!(failure.http_status, Some(403));
            }
            if expected == "download" {
                assert_eq!(
                    failure.error,
                    ImageEditError::Transport(NativeHttpRefusal::Unknown)
                );
            } else {
                assert_eq!(failure.error, ImageEditError::Provider(response_status));
            }
            server.join().unwrap();
        }
    }
    #[test]
    fn request_never_disables_filters_or_searches_for_subjects() {
        let body = generation_body("public");
        assert_eq!(body["safety_tolerance"], 2);
        assert_eq!(body["grounding"], false);
        assert!(body.get("images").is_none());
        assert!(body.get("enable_safety_checker").is_none());
    }
    #[test]
    fn moderation_is_terminal_and_untrusted_poll_hosts_never_receive_credentials() {
        for state in ["Request Moderated", "Content Moderated"] {
            assert_eq!(
                status(&json!({"status":state})),
                Err(ImageEditError::Refused)
            );
        }
        for url in [
            "http://api.bfl.ai/v1/get_result?id=x",
            "https://evil.test/v1/get_result?id=x",
            "https://api.bfl.ai@evil.test/v1/get_result",
            "https://api.bfl.ai:444/v1/get_result",
            "https://delivery.us.bfl.ai/v1/get_result",
        ] {
            assert!(!poll_allowed(url, None));
        }
        assert!(poll_allowed(
            "https://api.us1.bfl.ai/v1/get_result?id=x",
            None
        ));
    }
}
