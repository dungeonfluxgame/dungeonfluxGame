//! Who may talk to the developer API.
//!
//! The API is for the person at this machine, so every request must name a loopback host, which
//! stops a web page that has been pointed at 127.0.0.1 by DNS from reading the canvas. A request
//! that changes something must also be JSON and must not come from another site, which stops a
//! web page from posting a form to it. The routes that take a recording (`RECORDING_ROUTES`: the
//! dev composer's and a seat's) require `audio/L16` instead of JSON: that is not a content type a
//! cross-site form can send without a preflight, which the origin checks refuse.
//! There is no password: a request without a browser can
//! name any host it likes, so on a shared machine any local process can drive the canvas.
//!
//! The player listener (`player`) answers phones on the network with the same rules, for its own
//! audience: a request must name one of this machine's addresses (or a loopback name) on the
//! player port, so a page elsewhere cannot use DNS to reach it under another name, and a request
//! that changes something must be same-origin JSON.
//!
//! It owns these rules as plain functions over headers, and never touches the canvas.

use std::net::IpAddr;

use axum::http::{HeaderMap, Method, StatusCode, header};
use serde::{Deserialize, Serialize};

use crate::speech::routes::path::SPEECH_TRANSCRIBE;

/// A route whose body is not JSON: what it takes, and the largest body it reads.
pub(crate) struct BodyRoute {
    pub(crate) path: &'static str,
    body: BodyType,
    pub(crate) limit: usize,
}

/// The routes that take a recording (`POST`, `audio/L16`), with the speech service's bound on a
/// recording. Every other route that changes something takes JSON.
pub(crate) const RECORDING_ROUTES: [BodyRoute; 2] = [
    BodyRoute {
        path: SPEECH_TRANSCRIBE,
        body: BodyType::Audio,
        limit: crate::speech::limits::MAXIMUM_ADMITTED_AUDIO_BYTES,
    },
    BodyRoute {
        path: df_playground::path::PLAY_SEAT_TRANSCRIBE,
        body: BodyType::Audio,
        limit: crate::speech::limits::MAXIMUM_ADMITTED_AUDIO_BYTES,
    },
];

/// The largest body the recording route `path` reads.
pub(crate) fn recording_limit(path: &str) -> usize {
    RECORDING_ROUTES
        .iter()
        .find(|route| route.path == path)
        .map_or(
            crate::speech::limits::MAXIMUM_ADMITTED_AUDIO_BYTES,
            |route| route.limit,
        )
}

/// Why a request was refused: a typed route error like every route's, answered as a `DevReply`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum GuardError {
    ForeignHost,
    ForeignOrigin,
    CrossSiteRequest,
    NotJson,
    /// A recording route was not sent `audio/L16`.
    NotAudio,
    /// A developer header (`seat::key::DEV_HEADER_PREFIX`: acting as a seat) came to the player
    /// listener, which never takes one.
    DevHeader,
}

/// What a route that changes something takes as its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyType {
    Json,
    /// `audio/L16`, with any parameters (the recording's `rate=16000`).
    Audio,
}

impl BodyType {
    /// The body type of the route at `path`, as the route table declares it: JSON unless listed.
    fn of(path: &str) -> Self {
        RECORDING_ROUTES
            .iter()
            .find(|route| route.path == path)
            .map_or(Self::Json, |route| route.body)
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
    pub const ALL: [Self; 6] = [
        Self::ForeignHost,
        Self::ForeignOrigin,
        Self::CrossSiteRequest,
        Self::NotJson,
        Self::NotAudio,
        Self::DevHeader,
    ];

    /// The status to answer with.
    pub fn status(self) -> StatusCode {
        match self {
            Self::NotJson | Self::NotAudio => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::ForeignHost | Self::ForeignOrigin | Self::CrossSiteRequest | Self::DevHeader => {
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
            Self::DevHeader => "developer headers are accepted only on the developer listener",
        }
    }
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for GuardError {}

impl crate::dev_http::RouteError for GuardError {
    fn status(&self) -> StatusCode {
        (*self).status()
    }
}

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

/// The port an `http` authority names when it names none.
const HTTP_PORT: u16 = 80;

/// The port inside an authority such as `192.168.1.20:7880` or `[::1]:7880`, or `HTTP_PORT` when it
/// names none. A port that is not a number gives `None`.
fn port_of(authority: &str) -> Option<u16> {
    let port = match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split_once(']')?.1.strip_prefix(':'),
        None => authority.rsplit_once(':').map(|(_name, port)| port),
    };
    port.map_or(Some(HTTP_PORT), |port| port.parse().ok())
}

/// The authority of an `Origin` header such as `http://127.0.0.1:7878`. `null` and anything that
/// is not a URL give `None`.
fn origin_authority(origin: &str) -> Option<&str> {
    let (_scheme, rest) = origin.split_once("://")?;
    rest.split('/').next()
}

/// Who a listener answers: the host names a request may give and an origin may have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audience {
    /// The developer API: a loopback name, on any port.
    Developer,
    /// The player listener: one of `addresses` (this machine's addresses on the network) or a
    /// loopback name, on `port` only.
    Players { addresses: Vec<IpAddr>, port: u16 },
}

impl Audience {
    /// Whether a request for `authority` (a `Host` header, or an origin's authority) is for this
    /// listener.
    fn accepts(&self, authority: &str) -> bool {
        let name = host_name(authority);
        match self {
            Self::Developer => is_loopback_name(name),
            Self::Players { addresses, port } => {
                let ours = is_loopback_name(name)
                    || name
                        .parse::<IpAddr>()
                        .is_ok_and(|address| addresses.contains(&address));
                ours && port_of(authority) == Some(*port)
            }
        }
    }
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Applies the developer API's rules to one request for the route at `path`.
pub fn check(method: &Method, path: &str, headers: &HeaderMap) -> Result<(), GuardError> {
    check_for(&Audience::Developer, method, path, headers)
}

/// Applies the rules of the listener that answers `audience` to one request for the route at
/// `path`.
pub fn check_for(
    audience: &Audience,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Result<(), GuardError> {
    let host = header_text(headers, header::HOST).ok_or(GuardError::ForeignHost)?;
    if !audience.accepts(host) {
        return Err(GuardError::ForeignHost);
    }
    // Acting as a seat by its number is the developer's (the `dev` command line), never a phone's.
    if matches!(audience, Audience::Players { .. })
        && headers.keys().any(|name| {
            name.as_str()
                .starts_with(crate::seat::key::DEV_HEADER_PREFIX)
        })
    {
        return Err(GuardError::DevHeader);
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
        && !origin_authority(origin).is_some_and(|authority| audience.accepts(authority))
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
