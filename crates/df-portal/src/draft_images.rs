//! Authenticated delivery of account-private draft images.
//!
//! `GET /app/api/draft-images/{asset}` serves one buffered private image to its owning account.
//! The route authenticates the session cookie through `CustomerService`, buffers bytes through
//! the injected narrow adapter, re-authenticates the same cookie, and only then asks the adapter
//! for the final disclosure decision. That final adapter call is the disclosure linearization
//! point; this module builds the response synchronously right after it returns, with no further
//! storage await. `HEAD` shares the authorization and omits the bytes; a `Range` request still
//! receives the full bounded 200 response.
//!
//! Denials carry no body and no reason: missing, forbidden, stale or tombstoned reads answer
//! 404, while authority or storage failures answer 503.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, Uri, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use df_assets::{
    DRAFT_IMAGE_OPERATION_TIMEOUT, Deadline, DeliveryBasis, DraftDeliveryError,
    DraftImageCandidate, DraftImageVariant,
};
use df_customer::{AccountStore, Authenticated, CustomerService, EmailSender};
use df_types::{AccountId, WebSessionId};

use crate::{SESSION_COOKIE, security::cookie};

/// How long one draft image request may take end to end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DraftImagePolicy {
    pub operation_timeout: Duration,
}

impl Default for DraftImagePolicy {
    fn default() -> Self {
        Self {
            operation_timeout: DRAFT_IMAGE_OPERATION_TIMEOUT,
        }
    }
}

/// The narrow delivery adapter the route is generic over. `df-hosted` implements it over the
/// production catalog, object store and account session storage; tests use a fake. The adapter
/// never sees cookies or digests: it receives only the authenticated account, session and the
/// exact catalog basis the buffered bytes were verified against.
pub trait DraftImageSource: Send + Sync + 'static {
    fn buffer_candidate(
        &self,
        owner: AccountId,
        asset: df_assets::AssetId,
        variant: DraftImageVariant,
        deadline: Deadline,
    ) -> impl Future<Output = Result<DraftImageCandidate, DraftDeliveryError>> + Send;

    /// The disclosure linearization point: rechecks the live session, account and exact catalog
    /// basis in one snapshot after the bytes are buffered. A denial retracts the buffer.
    /// The propagated deadline bounds the final snapshot; no phase resets it.
    fn authorize_release(
        &self,
        owner: AccountId,
        session: WebSessionId,
        basis: DeliveryBasis,
        deadline: Deadline,
    ) -> impl Future<Output = Result<(), DraftDeliveryError>> + Send;
}

struct DraftImageState<S, M, A> {
    service: CustomerService<S, M>,
    source: Arc<A>,
    policy: DraftImagePolicy,
}

impl<S, M, A> Clone for DraftImageState<S, M, A> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            source: Arc::clone(&self.source),
            policy: self.policy,
        }
    }
}

/// Mounts the authenticated draft image route. The host merges this alongside
/// `app_api::compose`, before the common middleware.
pub fn router<S, M, A>(
    service: CustomerService<S, M>,
    source: Arc<A>,
    policy: DraftImagePolicy,
) -> Router
where
    S: AccountStore,
    M: EmailSender,
    A: DraftImageSource,
{
    let state = DraftImageState {
        service,
        source,
        policy,
    };
    Router::new()
        .route(
            "/app/api/draft-images/{asset}",
            get(handler::<S, M, A>).head(handler::<S, M, A>),
        )
        .layer(axum::middleware::from_fn(privacy_layer))
        .with_state(state)
}

async fn privacy_layer(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    if response.status() == StatusCode::BAD_REQUEST {
        return denied(StatusCode::NOT_FOUND);
    }
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    response
}

fn privacy_headers(headers: &mut HeaderMap) {
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
}

/// A denial with no body and no reason. Concealed failures answer 404; authority failures 503.
fn denied(status: StatusCode) -> Response {
    Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header("cross-origin-resource-policy", "same-origin")
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::SERVICE_UNAVAILABLE.into_response())
}

fn status_of(error: DraftDeliveryError) -> StatusCode {
    if error.concealed() {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn caller<S: AccountStore, M: EmailSender>(
    service: &CustomerService<S, M>,
    headers: &HeaderMap,
) -> Result<Option<Authenticated>, Response> {
    let Some(token) = cookie(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    match service.authenticate(token).await {
        Ok(caller) => Ok(caller),
        Err(_) => Err(denied(StatusCode::SERVICE_UNAVAILABLE)),
    }
}

fn same_caller(first: &Authenticated, second: &Authenticated) -> bool {
    second.session == first.session
        && second.account.id == first.account.id
        && second.account.credential_generation == first.account.credential_generation
}

fn parse_variant(query: Option<&str>) -> Option<DraftImageVariant> {
    let Some(query) = query else {
        return Some(DraftImageVariant::Display);
    };
    if query.is_empty() {
        return Some(DraftImageVariant::Display);
    }
    let mut variant: Option<DraftImageVariant> = None;
    let mut seen_variant = false;
    for part in query.split('&') {
        let (key, value) = part.split_once('=')?;
        if key == "account" {
            continue;
        }
        if key != "variant" {
            return None;
        }
        if seen_variant {
            return None;
        }
        seen_variant = true;
        variant = match value {
            "display" => Some(DraftImageVariant::Display),
            "master" => Some(DraftImageVariant::Master),
            _ => return None,
        };
    }
    Some(variant.unwrap_or(DraftImageVariant::Display))
}

async fn handler<S, M, A>(
    State(state): State<DraftImageState<S, M, A>>,
    method: Method,
    headers: HeaderMap,
    Path(asset_text): Path<String>,
    uri: Uri,
) -> Response
where
    S: AccountStore,
    M: EmailSender,
    A: DraftImageSource,
{
    let deadline = Deadline::after(state.policy.operation_timeout);
    let remaining = deadline
        .instant()
        .saturating_duration_since(std::time::Instant::now());
    let served = serve::<S, M, A>(state, method, headers, asset_text, uri, deadline);
    match tokio::time::timeout(remaining, served).await {
        Ok(response) => response,
        Err(_) => denied(StatusCode::SERVICE_UNAVAILABLE),
    }
}

async fn serve<S, M, A>(
    state: DraftImageState<S, M, A>,
    method: Method,
    headers: HeaderMap,
    asset_text: String,
    uri: Uri,
    deadline: Deadline,
) -> Response
where
    S: AccountStore,
    M: EmailSender,
    A: DraftImageSource,
{
    if deadline.has_passed() {
        return denied(status_of(DraftDeliveryError::DeadlineExceeded));
    }
    let first = match caller(&state.service, &headers).await {
        Ok(Some(caller)) => caller,
        Ok(None) => return denied(StatusCode::NOT_FOUND),
        Err(response) => return response,
    };
    let Ok(asset) = df_assets::AssetId::from_hex(&asset_text) else {
        return denied(StatusCode::NOT_FOUND);
    };
    let Some(variant) = parse_variant(uri.query()) else {
        return denied(StatusCode::NOT_FOUND);
    };
    if deadline.has_passed() {
        return denied(status_of(DraftDeliveryError::DeadlineExceeded));
    }
    let candidate = match state
        .source
        .buffer_candidate(first.account.id, asset, variant, deadline)
        .await
    {
        Ok(candidate) => candidate,
        Err(error) => return denied(status_of(error)),
    };
    let second = match caller(&state.service, &headers).await {
        Ok(Some(caller)) => caller,
        Ok(None) => return denied(StatusCode::NOT_FOUND),
        Err(response) => return response,
    };
    if !same_caller(&first, &second) {
        return denied(StatusCode::NOT_FOUND);
    }
    if deadline.has_passed() {
        return denied(status_of(DraftDeliveryError::DeadlineExceeded));
    }
    if let Err(error) = state
        .source
        .authorize_release(
            first.account.id,
            first.session,
            candidate.basis().clone(),
            deadline,
        )
        .await
    {
        return denied(status_of(error));
    }
    if deadline.has_passed() {
        return denied(status_of(DraftDeliveryError::DeadlineExceeded));
    }
    let (mime, byte_len) = (candidate.mime().to_owned(), candidate.byte_len());
    let (Ok(content_type), Ok(content_length)) = (
        HeaderValue::from_str(&mime),
        HeaderValue::from_str(&byte_len.to_string()),
    ) else {
        return denied(StatusCode::SERVICE_UNAVAILABLE);
    };
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, content_length)
        .body(if method == Method::HEAD {
            Body::empty()
        } else {
            Body::new(candidate.into_body())
        })
        .unwrap_or_else(|_| StatusCode::SERVICE_UNAVAILABLE.into_response());
    privacy_headers(response.headers_mut());
    response
}
