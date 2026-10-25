//! Cookies, CSRF, origin checks, the client's address and device, and the security headers
//! (plan 6.1; `bootstrap-origin-policy.md`).
//!
//! Cookies are host-only (`__Host-` prefix: `Secure`, `Path=/`, no `Domain`), `HttpOnly` and
//! `SameSite=Lax`. Every `POST` must carry the form's CSRF token for the caller's session or
//! pre-session, must not be cross-site, and a present `Origin` must equal the configured origin.

use std::net::SocketAddr;

use crate::client_ip::{ClientIpRefusal, TrustedProxies, client_ip};

use axum::{
    extract::{ConnectInfo, FromRequestParts, MatchedPath, Request},
    http::{HeaderMap, HeaderValue, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use df_customer::{Client, CsrfSecret, DeviceLabel};

pub const SESSION_COOKIE: &str = "__Host-df_session";
pub const PRESESSION_COOKIE: &str = "__Host-df_presession";
/// The pre-session (anonymous form) cookie lives one hour.
pub const PRESESSION_MAX_AGE_S: u64 = 60 * 60;

const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; style-src 'self'; img-src 'self'; \
    form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

/// Reads one cookie's value from the request.
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then_some(value)
        })
}

/// `Set-Cookie` for a host-only secret cookie.
pub fn set_cookie(name: &str, value: &str, max_age_s: u64) -> HeaderValue {
    // The value is our own hex token; the header is always valid.
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age_s}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("__Host-df_invalid=; Path=/; Secure"))
}

/// `Set-Cookie` that deletes the cookie.
pub fn clear_cookie(name: &str) -> HeaderValue {
    set_cookie(name, "", 0)
}

/// Why a `POST` was refused before it reached its handler's logic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormRefusal {
    CrossSite,
    WrongOrigin,
    BadToken,
}

impl FormRefusal {
    pub const fn label(self) -> &'static str {
        match self {
            Self::CrossSite => "cross_site",
            Self::WrongOrigin => "wrong_origin",
            Self::BadToken => "bad_token",
        }
    }
}

/// The request-shape checks every `POST` shares.
pub fn check_post_origin(headers: &HeaderMap, origin: &str) -> Result<(), FormRefusal> {
    if headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        == Some("cross-site")
    {
        return Err(FormRefusal::CrossSite);
    }
    let mut origins = headers.get_all(header::ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => Ok(()),
        (Some(value), None) if value.as_bytes() == origin.as_bytes() => Ok(()),
        _ => Err(FormRefusal::WrongOrigin),
    }
}

/// Checks a form's CSRF token against the secret it was rendered with.
pub fn check_csrf(
    secret: Option<&CsrfSecret>,
    form: &str,
    submitted: &str,
) -> Result<(), FormRefusal> {
    match secret {
        Some(secret) if secret.accepts(form, submitted) => Ok(()),
        _ => Err(FormRefusal::BadToken),
    }
}

/// The pre-session secret of an anonymous request, if its cookie is present and well formed.
pub fn presession(headers: &HeaderMap) -> Option<CsrfSecret> {
    cookie(headers, PRESESSION_COOKIE).and_then(CsrfSecret::parse)
}

/// How the client's address is decided for every request the router serves, installed as a
/// request extension by the router layers. With this policy present, a request whose address
/// cannot be determined (no connection info, or a trusted proxy that forwards badly) is refused;
/// without it (a bare handler under test) the peer address is used when there is one.
#[derive(Clone, Debug, Default)]
pub struct ClientAddressPolicy {
    pub proxies: TrustedProxies,
}

/// The response when the client's address is unknown: nothing is counted in a shared bucket and
/// nothing runs.
pub struct ClientAddressRefused(ClientIpRefusal);

impl IntoResponse for ClientAddressRefused {
    fn into_response(self) -> Response {
        df_observe::record_audit_line(
            "portal.client_address_refused",
            format!("PORTAL client_address_refused reason={}", self.0.label()),
        );
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            [(header::CACHE_CONTROL, "no-store")],
            "Service unavailable",
        )
            .into_response()
    }
}

/// The caller's address (as the policy decides it) and coarse device class.
pub struct ClientInfo(pub Client);

/// A coarse device class from the user agent: shown on the sessions page, never a decision input.
fn device(headers: &HeaderMap) -> DeviceLabel {
    let Some(agent) = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
    else {
        return DeviceLabel::Unknown;
    };
    if agent.contains("iPad") || agent.contains("Tablet") {
        DeviceLabel::Tablet
    } else if agent.contains("Mobi") {
        DeviceLabel::Phone
    } else {
        DeviceLabel::Desktop
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientInfo {
    type Rejection = ClientAddressRefused;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip());
        // Forwarding headers count only from a configured proxy network (`client_ip`).
        let ip = match parts.extensions.get::<ClientAddressPolicy>() {
            Some(policy) => {
                let mut forwarded = Vec::new();
                for value in parts.headers.get_all("x-forwarded-for") {
                    match value.to_str() {
                        Ok(value) => forwarded.push(value),
                        Err(_) => {
                            return Err(ClientAddressRefused(ClientIpRefusal::MalformedForwarding));
                        }
                    }
                }
                Some(client_ip(peer, &forwarded, &policy.proxies).map_err(ClientAddressRefused)?)
            }
            None => peer,
        };
        Ok(Self(Client {
            ip,
            device: device(&parts.headers),
        }))
    }
}

/// The route templates spans are named after (a span name must be static).
const ROUTES: &[&str] = &[
    "/",
    "/how-it-plays",
    "/pricing",
    "/faq",
    "/terms",
    "/privacy",
    "/portal/{asset}",
    "/signup",
    "/verify-email",
    "/verify-email/resend",
    "/login",
    "/logout",
    "/password-reset",
    "/password-reset/confirm",
    "/email-change/confirm",
    "/account",
    "/account/profile",
    "/account/security",
    "/account/security/password",
    "/account/security/email",
    "/account/sessions",
    "/account/sessions/revoke",
    "/account/sessions/revoke-others",
    "/account/sessions/revoke-all",
    "/account/data",
    "/account/data/export",
    "/account/close",
    "/auth/callback",
];

fn route_name(path: Option<&MatchedPath>) -> &'static str {
    path.and_then(|path| ROUTES.iter().find(|route| **route == path.as_str()))
        .copied()
        .unwrap_or("unmatched")
}

/// Adds the security headers, keeps account pages out of caches, and opens one span per request
/// named by its route template. Query strings, bodies and cookies are never recorded.
pub async fn headers_and_span(request: Request, next: Next) -> Response {
    let route = route_name(request.extensions().get::<MatchedPath>());
    let is_asset = route == "/portal/{asset}";
    let mut span = df_observe::begin(
        &df_observe::OperationContext {
            trace_parent: String::new(),
            build: env!("CARGO_PKG_VERSION").to_owned(),
        },
        "portal.request",
    );
    span.event("route", &[("route", route.to_owned())]);
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if !is_asset {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    span.finish_unmeasured(response.status().as_str());
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cookie_is_found_among_several_and_a_prefix_match_is_not_enough() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; __Host-df_session_x=2; __Host-df_session=abc"),
        );
        assert_eq!(cookie(&headers, SESSION_COOKIE), Some("abc"));
        assert_eq!(cookie(&headers, "missing"), None);
    }

    #[test]
    fn the_session_cookie_is_host_only_secure_http_only_and_lax() {
        assert_eq!(
            set_cookie(SESSION_COOKIE, "ab12", 60).to_str().unwrap(),
            "__Host-df_session=ab12; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=60"
        );
    }

    #[test]
    fn a_cross_site_or_foreign_origin_post_is_refused() {
        let origin = "http://127.0.0.1:8099";
        let mut headers = HeaderMap::new();
        assert_eq!(check_post_origin(&headers, origin), Ok(()));
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert_eq!(
            check_post_origin(&headers, origin),
            Err(FormRefusal::CrossSite)
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, HeaderValue::from_static("http://evil.test"));
        assert_eq!(
            check_post_origin(&headers, origin),
            Err(FormRefusal::WrongOrigin)
        );
        let mut headers = HeaderMap::new();
        headers.append(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:8099"),
        );
        headers.append(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:8099"),
        );
        assert_eq!(
            check_post_origin(&headers, origin),
            Err(FormRefusal::WrongOrigin),
            "a duplicated Origin is refused"
        );
    }
}
