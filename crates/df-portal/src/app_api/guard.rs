//! The request guard and the JSON envelope of the account app's API.
//!
//! A mutating request must prove it came from our own page: a JSON content type, no cross-site
//! or same-site fetch metadata, an `Origin` (when present) equal to the configured origin, and
//! the CSRF token of the caller's session or pre-session in `x-df-csrf`. The display device's
//! redeem and renew calls use the stricter `DevicePost` instead of a token (S1 8.5 step 4).
//! Every refusal answers `csrf_refused` and changes nothing. Campaign routes mounted by S1 use
//! these extractors through `GuardState`.

use std::future::Future;

use axum::{
    body::Bytes,
    extract::{FromRequest, FromRequestParts, Request},
    http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use df_account_app::wire::{ApiError, CSRF_FORM, CSRF_HEADER, DEVICE_HEADER, ErrorCode};
use df_customer::{Authenticated, CsrfSecret, ServiceError};
use serde::{Serialize, de::DeserializeOwned};

use crate::security::{PRESESSION_COOKIE, SESSION_COOKIE, cookie, presession};

/// The `SameSite=Strict` cookie that identifies a pairing display (S1 8.5).
pub const DEVICE_COOKIE: &str = "__Host-df_display_device";
/// The display ticket cookie, issued once at redemption.
pub const TICKET_COOKIE: &str = "__Host-df_display_ticket";
/// The invitation remembered across sign in and sign up (S1 8.2).
pub const JOIN_COOKIE: &str = "__Host-df_join";
/// Largest accepted JSON body.
pub const MAXIMUM_BODY_BYTES: usize = 16 * 1024;

/// What the guard needs from the application state. S1's own router implements it for its state.
pub trait GuardState: Clone + Send + Sync + 'static {
    /// The configured canonical origin, `scheme://host[:port]`.
    fn origin(&self) -> &str;

    fn authenticate(
        &self,
        session_token: &str,
    ) -> impl Future<Output = Result<Option<Authenticated>, ServiceError>> + Send;
}

pub const fn status_of(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::SignedOut => StatusCode::UNAUTHORIZED,
        ErrorCode::Forbidden | ErrorCode::NotVerified | ErrorCode::CsrfRefused => {
            StatusCode::FORBIDDEN
        }
        ErrorCode::Throttled => StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
        ErrorCode::StaleForm => StatusCode::CONFLICT,
        ErrorCode::Unavailable | ErrorCode::Busy => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::UnknownOutcome => StatusCode::GATEWAY_TIMEOUT,
    }
}

/// Headers every API response carries, so no reply is cached, sniffed or leaks a referrer.
pub fn apply_headers(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}

fn json_response(status: StatusCode, body: &impl Serialize, cookies: Vec<HeaderValue>) -> Response {
    let Ok(text) = serde_json::to_string(body) else {
        return ApiFailure::of(ErrorCode::Unavailable).into_response();
    };
    let mut response = (status, [(header::CONTENT_TYPE, "application/json")], text).into_response();
    let headers = response.headers_mut();
    apply_headers(headers);
    for cookie in cookies {
        headers.append(header::SET_COOKIE, cookie);
    }
    response
}

/// A refusal in the API envelope, with the cookies to set or clear alongside it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiFailure {
    pub error: ApiError,
    cookies: Vec<HeaderValue>,
}

impl ApiFailure {
    pub const fn new(error: ApiError) -> Self {
        Self {
            error,
            cookies: Vec::new(),
        }
    }

    pub const fn of(code: ErrorCode) -> Self {
        Self::new(ApiError::of(code))
    }

    /// Adds a `Set-Cookie` to the refusal (a failed display redemption clears the device cookie).
    pub fn with_cookie(mut self, cookie: HeaderValue) -> Self {
        self.cookies.push(cookie);
        self
    }
}

impl From<ApiError> for ApiFailure {
    fn from(error: ApiError) -> Self {
        Self::new(error)
    }
}

impl From<ServiceError> for ApiFailure {
    fn from(error: ServiceError) -> Self {
        Self::of(match error {
            ServiceError::Unknown => ErrorCode::UnknownOutcome,
            ServiceError::Busy => ErrorCode::Busy,
            ServiceError::Unavailable
            | ServiceError::InvalidRecord
            | ServiceError::RandomUnavailable => ErrorCode::Unavailable,
        })
    }
}

impl From<super::ports::PortError> for ApiFailure {
    fn from(error: super::ports::PortError) -> Self {
        Self::new(error.api_error())
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        json_response(status_of(self.error.code), &self.error, self.cookies)
    }
}

/// A 200 reply with a JSON body and optional cookies.
pub struct Reply<T> {
    body: T,
    cookies: Vec<HeaderValue>,
}

impl<T> Reply<T> {
    pub const fn new(body: T) -> Self {
        Self {
            body,
            cookies: Vec::new(),
        }
    }

    pub fn with_cookie(mut self, cookie: HeaderValue) -> Self {
        self.cookies.push(cookie);
        self
    }
}

impl<T: Serialize> IntoResponse for Reply<T> {
    fn into_response(self) -> Response {
        json_response(StatusCode::OK, &self.body, self.cookies)
    }
}

pub type ApiResult<T> = Result<Reply<T>, ApiFailure>;

pub fn reply<T>(body: T) -> ApiResult<T> {
    Ok(Reply::new(body))
}

/// The header middleware for routes that answer without going through `Reply` or `ApiFailure`.
pub async fn api_headers(request: Request, next: axum::middleware::Next) -> Response {
    let mut response = next.run(request).await;
    apply_headers(response.headers_mut());
    response
}

/// Why a request failed the shape checks. Only for tests and audit; the caller sees `csrf_refused`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refused {
    ContentType,
    FetchSite,
    Origin,
    Token,
    DeviceCookie,
    DeviceHeader,
}

impl From<Refused> for ApiFailure {
    fn from(refused: Refused) -> Self {
        df_observe::record_audit_line(
            "app_api.csrf_refused",
            format!("APP_API csrf_refused reason={refused:?}"),
        );
        Self::of(ErrorCode::CsrfRefused)
    }
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    header_text(headers, "content-type")
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}

/// A present `Origin` must be exactly the configured origin, once.
fn origin_matches(headers: &HeaderMap, origin: &str) -> Option<bool> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => None,
        (Some(value), None) => Some(value.as_bytes() == origin.as_bytes()),
        _ => Some(false),
    }
}

/// The checks every token-guarded mutation shares.
pub fn check_shape(headers: &HeaderMap, origin: &str) -> Result<(), Refused> {
    if !has_json_content_type(headers) {
        return Err(Refused::ContentType);
    }
    if matches!(
        header_text(headers, "sec-fetch-site"),
        Some("cross-site" | "same-site")
    ) {
        return Err(Refused::FetchSite);
    }
    if origin_matches(headers, origin) == Some(false) {
        return Err(Refused::Origin);
    }
    Ok(())
}

fn check_token(secret: &CsrfSecret, headers: &HeaderMap) -> Result<(), Refused> {
    match header_text(headers, CSRF_HEADER) {
        Some(token) if secret.accepts(CSRF_FORM, token) => Ok(()),
        _ => Err(Refused::Token),
    }
}

/// The checks for the display device's calls: no token, but the Strict device cookie, a present
/// matching `Origin`, `Sec-Fetch-Site: same-origin` exactly, a JSON content type and the custom
/// header that forces a preflight no other origin can pass.
pub fn check_device(headers: &HeaderMap, origin: &str) -> Result<(), Refused> {
    if !has_json_content_type(headers) {
        return Err(Refused::ContentType);
    }
    if header_text(headers, "sec-fetch-site") != Some("same-origin") {
        return Err(Refused::FetchSite);
    }
    if origin_matches(headers, origin) != Some(true) {
        return Err(Refused::Origin);
    }
    if header_text(headers, DEVICE_HEADER).is_none_or(str::is_empty) {
        return Err(Refused::DeviceHeader);
    }
    Ok(())
}

async fn session_of<G: GuardState>(
    state: &G,
    headers: &HeaderMap,
) -> Result<Option<Authenticated>, ApiFailure> {
    match cookie(headers, SESSION_COOKIE) {
        Some(token) => Ok(state.authenticate(token).await?),
        None => Ok(None),
    }
}

/// A signed-in caller on a read.
pub struct SignedIn(pub Authenticated);

impl<G: GuardState> FromRequestParts<G> for SignedIn {
    type Rejection = ApiFailure;

    async fn from_request_parts(parts: &mut Parts, state: &G) -> Result<Self, Self::Rejection> {
        session_of(state, &parts.headers)
            .await?
            .map(Self)
            .ok_or_else(|| ApiFailure::of(ErrorCode::SignedOut))
    }
}

/// A read that works signed in or not: the session, or the pre-session cookie's secret.
pub struct Visitor {
    pub caller: Option<Authenticated>,
    pub presession: Option<CsrfSecret>,
}

impl<G: GuardState> FromRequestParts<G> for Visitor {
    type Rejection = ApiFailure;

    async fn from_request_parts(parts: &mut Parts, state: &G) -> Result<Self, Self::Rejection> {
        Ok(Self {
            caller: session_of(state, &parts.headers).await?,
            presession: presession(&parts.headers),
        })
    }
}

/// A token-guarded mutation by a signed-in caller.
pub struct SessionPost(pub Authenticated);

impl<G: GuardState> FromRequestParts<G> for SessionPost {
    type Rejection = ApiFailure;

    async fn from_request_parts(parts: &mut Parts, state: &G) -> Result<Self, Self::Rejection> {
        check_shape(&parts.headers, state.origin())?;
        let caller = session_of(state, &parts.headers)
            .await?
            .ok_or_else(|| ApiFailure::of(ErrorCode::SignedOut))?;
        check_token(&caller.csrf, &parts.headers)?;
        Ok(Self(caller))
    }
}

/// A token-guarded mutation by anyone: the token is the session's when one is valid, else the
/// pre-session's. A pre-session cookie alone authorizes nothing beyond the anonymous forms.
pub struct VisitorPost {
    pub caller: Option<Authenticated>,
}

impl<G: GuardState> FromRequestParts<G> for VisitorPost {
    type Rejection = ApiFailure;

    async fn from_request_parts(parts: &mut Parts, state: &G) -> Result<Self, Self::Rejection> {
        check_shape(&parts.headers, state.origin())?;
        let caller = session_of(state, &parts.headers).await?;
        let secret = match &caller {
            Some(caller) => Some(caller.csrf.clone()),
            None => presession(&parts.headers),
        };
        let secret = secret.ok_or(Refused::Token)?;
        check_token(&secret, &parts.headers)?;
        Ok(Self { caller })
    }
}

/// The display device's redeem and renew calls: the device cookie and the strict header checks.
pub struct DevicePost {
    pub device_secret: String,
    pub ticket: Option<String>,
}

impl<G: GuardState> FromRequestParts<G> for DevicePost {
    type Rejection = ApiFailure;

    async fn from_request_parts(parts: &mut Parts, state: &G) -> Result<Self, Self::Rejection> {
        check_device(&parts.headers, state.origin())?;
        let device_secret = cookie(&parts.headers, DEVICE_COOKIE)
            .filter(|secret| !secret.is_empty())
            .ok_or(Refused::DeviceCookie)?;
        Ok(Self {
            device_secret: device_secret.to_owned(),
            ticket: cookie(&parts.headers, TICKET_COOKIE).map(str::to_owned),
        })
    }
}

/// The request body, strictly deserialized; anything else is `invalid`.
pub struct ApiBody<T>(pub T);

impl<G: Send + Sync, T: DeserializeOwned> FromRequest<G> for ApiBody<T> {
    type Rejection = ApiFailure;

    async fn from_request(request: Request, state: &G) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|_| ApiFailure::of(ErrorCode::Invalid))?;
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|_| ApiFailure::of(ErrorCode::Invalid))
    }
}

/// An empty JSON object, the body of calls that carry nothing.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nothing {}

/// `Set-Cookie` for a host-only secret cookie with the given `SameSite` mode.
pub fn secret_cookie(name: &str, value: &str, max_age_s: u64, same_site: SameSite) -> HeaderValue {
    // The value is our own hexadecimal token; the header is always valid.
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; Secure; HttpOnly; SameSite={}; Max-Age={max_age_s}",
        same_site.label()
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("__Host-df_invalid=; Path=/; Secure"))
}

pub fn clear_secret_cookie(name: &str, same_site: SameSite) -> HeaderValue {
    secret_cookie(name, "", 0, same_site)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SameSite {
    Lax,
    Strict,
}

impl SameSite {
    const fn label(self) -> &'static str {
        match self {
            Self::Lax => "Lax",
            Self::Strict => "Strict",
        }
    }
}

/// The pre-session cookie, created when the visitor has none.
pub fn presession_for_visitor(
    existing: Option<CsrfSecret>,
) -> Result<(CsrfSecret, Option<HeaderValue>), ApiFailure> {
    if let Some(secret) = existing {
        return Ok((secret, None));
    }
    let secret = CsrfSecret::generate().map_err(|_| ApiFailure::of(ErrorCode::Unavailable))?;
    let cookie = secret_cookie(
        PRESESSION_COOKIE,
        &secret.text(),
        crate::security::PRESESSION_MAX_AGE_S,
        SameSite::Lax,
    );
    Ok((secret, Some(cookie)))
}
