//! Who may talk to the developer API.
//!
//! The API is for the person at this machine, so every request must name a loopback host, which
//! stops a web page that has been pointed at 127.0.0.1 by DNS from reading the canvas. A request
//! that changes something must also be JSON and must not come from another site, which stops a
//! web page from posting a form to it. There is no password: a request without a browser can
//! name any host it likes, so on a shared machine any local process can drive the canvas.
//!
//! It owns these rules as plain functions over headers, and never touches the canvas.

use axum::http::{HeaderMap, Method, StatusCode, header};

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    ForeignHost,
    ForeignOrigin,
    CrossSiteRequest,
    NotJson,
}

impl Refusal {
    /// The status to answer with.
    pub fn status(self) -> StatusCode {
        match self {
            Self::NotJson => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::ForeignHost | Self::ForeignOrigin | Self::CrossSiteRequest => {
                StatusCode::FORBIDDEN
            }
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::ForeignHost => "requests are accepted only for a local host name",
            Self::ForeignOrigin => "requests are accepted only from a page on this machine",
            Self::CrossSiteRequest => "requests are not accepted from another site",
            Self::NotJson => "changes are accepted only as JSON (Content-Type: application/json)",
        }
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

/// Applies the rules to one request.
pub fn check(method: &Method, headers: &HeaderMap) -> Result<(), Refusal> {
    let host = header_text(headers, header::HOST).ok_or(Refusal::ForeignHost)?;
    if !is_loopback_name(host_name(host)) {
        return Err(Refusal::ForeignHost);
    }
    if method == Method::GET || method == Method::HEAD {
        return Ok(());
    }
    let is_json = header_text(headers, header::CONTENT_TYPE).is_some_and(|content_type| {
        content_type
            .to_ascii_lowercase()
            .starts_with("application/json")
    });
    if !is_json {
        return Err(Refusal::NotJson);
    }
    if let Some(origin) = header_text(headers, header::ORIGIN)
        && !origin_host(origin).is_some_and(is_loopback_name)
    {
        return Err(Refusal::ForeignOrigin);
    }
    if let Some(site) = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        && !matches!(site, "same-origin" | "none")
    {
        return Err(Refusal::CrossSiteRequest);
    }
    Ok(())
}
