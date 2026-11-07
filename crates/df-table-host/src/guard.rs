//! Who may talk to the developer API.
//!
//! The API is for the person at this machine, so every request must name a loopback host, which
//! stops a web page that has been pointed at 127.0.0.1 by DNS from reading the canvas. A request
//! that changes something must also be JSON and must not come from another site, which stops a
//! web page from posting a form to it. The one route that takes a recording
//! (`speech::routes::path::SPEECH_TRANSCRIBE`) requires `audio/L16` instead of JSON: that is not a
//! content type a cross-site form can send without a preflight, which the origin checks refuse.
//! There is no password: a request without a browser can
//! name any host it likes, so on a shared machine any local process can drive the canvas.
//!
//! It owns these rules as plain functions over headers, and never touches the canvas.

use axum::http::{HeaderMap, Method, StatusCode, header};

use crate::speech::routes::path::SPEECH_TRANSCRIBE;

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardError {
    ForeignHost,
    ForeignOrigin,
    CrossSiteRequest,
    NotJson,
    /// A recording route was not sent `audio/L16`.
    NotAudio,
}

/// What a route that changes something takes as its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyType {
    Json,
    /// `audio/L16`, with any parameters (the recording's `rate=16000`).
    Audio,
}

impl BodyType {
    /// The body type of the route at `path`: JSON, but for the recording route.
    fn of(path: &str) -> Self {
        if path == SPEECH_TRANSCRIBE {
            Self::Audio
        } else {
            Self::Json
        }
    }

    fn media_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Audio => "audio/l16",
        }
    }

    fn refusal(self) -> GuardError {
        match self {
            Self::Json => GuardError::NotJson,
            Self::Audio => GuardError::NotAudio,
        }
    }
}

impl GuardError {
    pub const ALL: [Self; 5] = [
        Self::ForeignHost,
        Self::ForeignOrigin,
        Self::CrossSiteRequest,
        Self::NotJson,
        Self::NotAudio,
    ];

    /// The status to answer with.
    pub fn status(self) -> StatusCode {
        match self {
            Self::NotJson | Self::NotAudio => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::ForeignHost | Self::ForeignOrigin | Self::CrossSiteRequest => {
                StatusCode::FORBIDDEN
            }
        }
    }

    /// The sentence of the refusal, without an allocation (the `Display` text).
    pub fn message(self) -> &'static str {
        match self {
            Self::ForeignHost => "requests are accepted only for a local host name",
            Self::ForeignOrigin => "requests are accepted only from a page on this machine",
            Self::CrossSiteRequest => "requests are not accepted from another site",
            Self::NotJson => "changes are accepted only as JSON (Content-Type: application/json)",
            Self::NotAudio => {
                "a recording is accepted only as audio/L16 (Content-Type: audio/L16;rate=16000)"
            }
        }
    }
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for GuardError {}

/// The host name inside an authority such as `127.0.0.1:7878`, `localhost` or `[::1]:7878`.
fn host_name(authority: &str) -> &str {
    if let Some(bracketed) = authority.strip_prefix('[') {
        return bracketed.split(']').next().unwrap_or("");
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(name, _port)| name)
}

fn is_loopback_name(name: &str) -> bool {
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

/// The host of an `Origin` header such as `http://127.0.0.1:7878`. `null` and anything that is not
/// a URL give `None`.
fn origin_host(origin: &str) -> Option<&str> {
    let (_scheme, rest) = origin.split_once("://")?;
    let authority = rest.split('/').next()?;
    Some(host_name(authority))
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Applies the rules to one request for the route at `path`.
pub fn check(method: &Method, path: &str, headers: &HeaderMap) -> Result<(), GuardError> {
    let host = header_text(headers, header::HOST).ok_or(GuardError::ForeignHost)?;
    if !is_loopback_name(host_name(host)) {
        return Err(GuardError::ForeignHost);
    }
    if method == Method::GET || method == Method::HEAD {
        return Ok(());
    }
    let body = BodyType::of(path);
    let sent_as_body = header_text(headers, header::CONTENT_TYPE).is_some_and(|content_type| {
        let media_type = content_type.split(';').next().unwrap_or("").trim();
        media_type.eq_ignore_ascii_case(body.media_type())
    });
    if !sent_as_body {
        return Err(body.refusal());
    }
    if let Some(origin) = header_text(headers, header::ORIGIN)
        && !origin_host(origin).is_some_and(is_loopback_name)
    {
        return Err(GuardError::ForeignOrigin);
    }
    if let Some(site) = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        && !matches!(site, "same-origin" | "none")
    {
        return Err(GuardError::CrossSiteRequest);
    }
    Ok(())
}
