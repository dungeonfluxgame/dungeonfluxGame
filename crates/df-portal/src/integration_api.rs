//! Bearer-only, customer read API. No cookie fallback, CORS, writes or operator authority.
mod limits;
mod reads;
use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use df_customer::{
    AccountStore, CustomerService, EmailSender,
    integration::{IntegrationPrincipal, Peppers, PersonalTokenStore, Scope, TokenId},
};
use limits::Limits;
pub use limits::{InvalidRatePolicy, RatePolicy};
pub use reads::{
    CampaignSummary, FundingSummary, IntegrationReads, Page, ReadFailure, SessionSummary,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct ApiState<S, M, R> {
    service: CustomerService<S, M>,
    peppers: Arc<Peppers>,
    reads: Arc<R>,
    limits: Arc<Mutex<Limits>>,
}
impl<S, M, R> Clone for ApiState<S, M, R> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            peppers: Arc::clone(&self.peppers),
            reads: Arc::clone(&self.reads),
            limits: Arc::clone(&self.limits),
        }
    }
}

pub fn router<S: AccountStore + PersonalTokenStore, M: EmailSender, R: IntegrationReads>(
    service: CustomerService<S, M>,
    peppers: Arc<Peppers>,
    reads: Arc<R>,
    policy: RatePolicy,
) -> Router {
    let api=Router::new()
        .route("/account", get(account::<S, M, R>))
        .route("/auth-sessions", get(auth_sessions::<S, M, R>))
        .route("/campaigns", get(reads::campaigns::<S, M, R>))
        .route("/play-sessions", get(reads::sessions::<S, M, R>))
        .route("/funding", get(reads::funding::<S, M, R>))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(axum::middleware::from_fn(deadline))
        .with_state(ApiState {
            service,
            peppers,
            reads,
            limits: Arc::new(Mutex::new(Limits::new(policy))),
        });
    Router::new().nest("/api/v1",api)
}

async fn not_found()->Failure{Failure::new(StatusCode::NOT_FOUND,"not_found",&request_id())}
async fn method_not_allowed()->Response{
    let mut response=Failure::new(StatusCode::METHOD_NOT_ALLOWED,"invalid_request",&request_id()).into_response();
    response.headers_mut().insert(header::ALLOW,HeaderValue::from_static("GET, HEAD"));response
}

async fn deadline(request:axum::extract::Request,next:axum::middleware::Next)->Response{
    match tokio::time::timeout(std::time::Duration::from_secs(30),next.run(request)).await{
        Ok(response)=>response,
        Err(_)=>Failure::unavailable(&request_id()).into_response(),
    }
}

struct Failure {
    status: StatusCode,
    code: &'static str,
    request: String,
    scope: Option<Scope>,
}
impl Failure {
    fn new(status: StatusCode, code: &'static str, request: &str) -> Self {
        Self {
            status,
            code,
            request: request.to_owned(),
            scope: None,
        }
    }
    fn unavailable(request: &str) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable", request)
    }
    fn invalid(request: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", request)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let mut error = json!({"code":self.code,"message":self.code.replace('_'," "),"request_id":self.request});
        if let Some(scope) = self.scope {
            error["required_scope"] = json!(scope.name());
        }
        let mut response = axum::Json(json!({"error":error})).into_response();
        *response.status_mut() = self.status;
        headers(response.headers_mut());
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}
fn headers(h: &mut HeaderMap) {
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}

fn request_id() -> String {
    use opentelemetry::trace::TraceContextExt;
    let context=opentelemetry::Context::current();
    let span=context.span();
    if span.span_context().is_valid(){return span.span_context().trace_id().to_string();}
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    format!(
        "api-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

struct Authorization {
    principal: IntegrationPrincipal,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl std::ops::Deref for Authorization {
    type Target = IntegrationPrincipal;
    fn deref(&self) -> &Self::Target {
        &self.principal
    }
}

async fn authorize<S: AccountStore + PersonalTokenStore, M: EmailSender, R>(
    state: &ApiState<S, M, R>,
    headers: &HeaderMap,
    scope: Scope,
    request: &str,
) -> Result<Authorization, Failure> {
    let now = state.service.now_ms();
    if !state
        .limits
        .lock()
        .map_err(|_| Failure::unavailable(request))?
        .global(now)
    {
        return Err(Failure::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            request,
        ));
    }
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let bearer = match (values.next(), values.next()) {
        (Some(value), None) => value.to_str().ok().and_then(|v| v.strip_prefix("Bearer ")),
        _ => None,
    }
    .ok_or_else(|| Failure::new(StatusCode::UNAUTHORIZED, "invalid_token", request))?;
    let principal = state
        .service
        .integration_principal(bearer, &state.peppers)
        .await
        .map_err(|_| Failure::unavailable(request))?
        .ok_or_else(|| Failure::new(StatusCode::UNAUTHORIZED, "invalid_token", request))?;
    let permit = state
        .limits
        .lock()
        .map_err(|_| Failure::unavailable(request))?
        .token(principal.token.id.0, *principal.account.id.as_bytes(), now)
        .ok_or_else(|| Failure::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", request))?;
    if !principal.token.scopes.contains(&scope) {
        return Err(Failure {
            status: StatusCode::FORBIDDEN,
            code: "insufficient_scope",
            request: request.to_owned(),
            scope: Some(scope),
        });
    }
    Ok(Authorization {
        principal,
        _permit: permit,
    })
}

fn respond(
    data: Value,
    request: &str,
    page: Option<Option<String>>,
    incoming: &HeaderMap,
    sensitive: bool,
) -> Response {
    use base64::Engine;
    use sha2::Digest;
    let mut body = json!({"data":data});
    if let Some(cursor) = page {
        body["page"] = json!({"next_cursor":cursor});
    }
    let Ok(bytes) = serde_json::to_vec(&body) else {
        return Failure::unavailable(request).into_response();
    };
    let etag = format!(
        "W/\"api-v1-{}\"",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(bytes))
    );
    let matches = incoming
        .get(header::IF_NONE_MATCH)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| {
            h.split(',')
                .any(|value| value.trim() == etag || value.trim() == "*")
        });
    body["meta"] = json!({"request_id":request});
    let mut response = if matches {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        axum::Json(body).into_response()
    };
    headers(response.headers_mut());
    if !sensitive {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-cache"),
        );
    }
    if let Ok(etag) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, etag);
    }
    response
}

pub fn timestamp(ms: u64) -> String {
    format!(
        "{}:{:02}.{:03}Z",
        crate::time_text::utc(ms)
            .replace(" UTC", "")
            .replace(' ', "T"),
        ms / 1000 % 60,
        ms % 1000
    )
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Paging {
    limit: Option<u32>,
    cursor: Option<String>,
}
impl Paging {
    fn checked(
        &self,
        peppers: &Peppers,
        account: &[u8; 16],
        endpoint: &str,
        now: u64,
        request: &str,
    ) -> Result<(u32, Option<TokenId>), Failure> {
        let limit = self.limit.unwrap_or(50);
        if !(1..=100).contains(&limit) {
            return Err(Failure::invalid(request));
        }
        let row = self
            .cursor
            .as_deref()
            .map(|c| {
                peppers
                    .cursor_row(account, endpoint, c, now)
                    .ok_or_else(|| Failure::invalid(request))
            })
            .transpose()?;
        Ok((limit, row))
    }
}

async fn account<S: AccountStore + PersonalTokenStore, M: EmailSender, R: IntegrationReads>(
    State(state): State<ApiState<S, M, R>>,
    h: HeaderMap,
) -> Result<Response, Failure> {
    let request = request_id();
    let principal = authorize(&state, &h, Scope::Account, &request).await?;
    let account = &principal.account;
    let mut data = json!({"account_id":TokenId(*account.id.as_bytes()).text(),"display_name":account.display_name.as_ref().map(|n| n.as_str()),"verified":account.is_verified,"standing":"active","created_at":timestamp(account.created_at_ms)});
    if principal.token.scopes.contains(&Scope::Email) && account.is_verified {
        data["email"] = json!(account.email.as_ref().map(|e| e.display()));
    }
    Ok(respond(data, &request, None, &h, false))
}

async fn auth_sessions<
    S: AccountStore + PersonalTokenStore,
    M: EmailSender,
    R: IntegrationReads,
>(
    State(state): State<ApiState<S, M, R>>,
    h: HeaderMap,
    query: Result<Query<Paging>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Failure> {
    let request = request_id();
    let principal = authorize(&state, &h, Scope::AuthSessions, &request).await?;
    let Query(paging) = query.map_err(|_| Failure::invalid(&request))?;
    let now = state.service.now_ms();
    let (limit, after) = paging.checked(
        &state.peppers,
        principal.account.id.as_bytes(),
        "auth-sessions",
        now,
        &request,
    )?;
    let mut rows = state
        .service
        .integration_sessions(principal.account.id, after, limit + 1)
        .await
        .map_err(|_| Failure::unavailable(&request))?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if has_more {
        rows.last().map(|r| {
            state
                .peppers
                .cursor(principal.account.id.as_bytes(), "auth-sessions", r.id, now)
        })
    } else {
        None
    };
    let data = rows.into_iter().map(|r| json!({"session_id":r.id.text(),"device_label":r.device_label,"created_at":timestamp(r.created_at_ms),"last_seen_at":timestamp(r.last_seen_at_ms),"expires_at":timestamp(r.expires_at_ms),"revoked":r.revoked})).collect::<Vec<_>>();
    Ok(respond(json!(data), &request, Some(next), &h, true))
}

#[cfg(test)]
mod tests;
