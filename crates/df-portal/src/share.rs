//! Curated public pages and cookie/CSRF management. Public pages have no cookie authority.
mod render;
mod limits;
pub use limits::{PublicSharePolicy,InvalidPublicSharePolicy};
use limits::PublicLimits;
mod tokens;
use crate::app_api::guard::{
    ApiBody, ApiFailure, ApiResult, GuardState, SessionPost, SignedIn, reply,
};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use df_account_app::{
    wire::{ErrorCode, RevokedView},
    wire_integration::SecretReveal,
    wire_share::{CreateShare, ShareConsent, ShareDraft, ShareIssued, ShareLinkView, SharePreview},
};
use df_customer::{AccountStore, CustomerService, EmailSender};
use df_types::AccountId;
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
pub use tokens::LinkToken;

/// Public output cannot represent account ids, private content or money.
pub struct PublicSummary {
    pub title: String,
    pub highlights: Vec<String>,
    pub names: Vec<String>,
    pub date: String,
    pub duration_minutes: u64,
    pub allow_indexing: bool,
    pub allow_embeds: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum ShareFailure {
    NotFound,
    ConsentRequired,
    Invalid,
    Unavailable,
    Unknown,
}
pub trait Sharing: Send + Sync + 'static {
    fn preview(
        &self,
        account: AccountId,
        session: &str,
    ) -> impl Future<Output = Result<SharePreview, ShareFailure>> + Send;
    fn save_preview(
        &self,
        account: AccountId,
        session: &str,
        draft: ShareDraft,
    ) -> impl Future<Output = Result<SharePreview, ShareFailure>> + Send;
    fn consent(
        &self,
        account: AccountId,
        session: &str,
        consent: ShareConsent,
    ) -> impl Future<Output = Result<(), ShareFailure>> + Send;
    fn create(
        &self,
        account: AccountId,
        session: &str,
        choices: CreateShare,
        digest: [u8; 32],
    ) -> impl Future<Output = Result<ShareLinkView, ShareFailure>> + Send;
    fn list(
        &self,
        account: AccountId,
        session: &str,
    ) -> impl Future<Output = Result<Vec<ShareLinkView>, ShareFailure>> + Send;
    fn revoke(
        &self,
        account: AccountId,
        session: &str,
        link: &str,
    ) -> impl Future<Output = Result<(), ShareFailure>> + Send;
    fn public(
        &self,
        digest: [u8; 32],
    ) -> impl Future<Output = Result<PublicSummary, ShareFailure>> + Send;
}

struct ShareState<S, M, P> {
    service: CustomerService<S, M>,
    origin: crate::PortalOrigin,
    port: Arc<P>,
    window: Arc<Mutex<PublicLimits>>,
}
impl<S, M, P> Clone for ShareState<S, M, P> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            origin: self.origin.clone(),
            port: Arc::clone(&self.port),
            window: Arc::clone(&self.window),
        }
    }
}
impl<S: AccountStore, M: EmailSender, P: Send + Sync + 'static> GuardState for ShareState<S, M, P> {
    fn origin(&self) -> &str {
        self.origin.as_str()
    }
    async fn authenticate(
        &self,
        token: &str,
    ) -> Result<Option<df_customer::Authenticated>, df_customer::ServiceError> {
        self.service.authenticate(token).await
    }
}
pub fn router<S: AccountStore, M: EmailSender, P: Sharing>(
    service: CustomerService<S, M>,
    origin: crate::PortalOrigin,
    port: Arc<P>,
    policy: PublicSharePolicy,
) -> Router {
    Router::new()
        .route(
            "/app/api/sessions/{session}/share-preview",
            get(preview::<S, M, P>).post(save::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session}/share-consent",
            post(consent::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session}/shares",
            get(list::<S, M, P>).post(create::<S, M, P>),
        )
        .route(
            "/app/api/sessions/{session}/shares/{link}/revoke",
            post(revoke::<S, M, P>),
        )
        .route("/s/{token}", get(page::<S, M, P>))
        .route("/embed/s/{token}", get(embed::<S, M, P>))
        .route(
            "/share.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    render::STYLE,
                )
            }),
        )
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
        .layer(axum::middleware::from_fn(deadline))
        .with_state(ShareState {
            service,
            origin,
            port,
            window: Arc::new(Mutex::new(PublicLimits::new(policy))),
        })
}
fn failure(error: ShareFailure) -> ApiFailure {
    if matches!(error, ShareFailure::ConsentRequired) {
        return ApiFailure::new(df_account_app::wire::ApiError::refused(df_account_app::wire::Refusal::ConsentRequired));
    }
    ApiFailure::of(match error {
        ShareFailure::NotFound => ErrorCode::NotFound,
        ShareFailure::ConsentRequired => ErrorCode::Forbidden,
        ShareFailure::Invalid => ErrorCode::Invalid,
        ShareFailure::Unavailable => ErrorCode::Unavailable,
        ShareFailure::Unknown => ErrorCode::UnknownOutcome,
    })
}

async fn deadline(request:axum::extract::Request,next:axum::middleware::Next)->Response{
    let mutation=!matches!(*request.method(),axum::http::Method::GET|axum::http::Method::HEAD);
    match tokio::time::timeout(std::time::Duration::from_secs(15),next.run(request)).await{
        Ok(mut response)=>{render::ensure_headers(&mut response);response},
        Err(_)=>failure(if mutation{ShareFailure::Unknown}else{ShareFailure::Unavailable}).into_response(),
    }
}
async fn preview<S: AccountStore, M: EmailSender, P: Sharing>(
    State(s): State<ShareState<S, M, P>>,
    SignedIn(a): SignedIn,
    Path(session): Path<String>,
) -> ApiResult<SharePreview> {
    reply(
        s.port
            .preview(a.account.id, &session)
            .await
            .map_err(failure)?,
    )
}
async fn save<S: AccountStore, M: EmailSender, P: Sharing>(
    State(s): State<ShareState<S, M, P>>,
    SessionPost(a): SessionPost,
    Path(session): Path<String>,
    ApiBody(body): ApiBody<ShareDraft>,
) -> ApiResult<SharePreview> {
    reply(
        s.port
            .save_preview(a.account.id, &session, body)
            .await
            .map_err(failure)?,
    )
}
async fn consent<S: AccountStore, M: EmailSender, P: Sharing>(
    State(s): State<ShareState<S, M, P>>,
    SessionPost(a): SessionPost,
    Path(session): Path<String>,
    ApiBody(body): ApiBody<ShareConsent>,
) -> ApiResult<RevokedView> {
    s.port
        .consent(a.account.id, &session, body)
        .await
        .map_err(failure)?;
    reply(RevokedView { revoked: 0 })
}
async fn list<S: AccountStore, M: EmailSender, P: Sharing>(
    State(s): State<ShareState<S, M, P>>,
    SignedIn(a): SignedIn,
    Path(session): Path<String>,
) -> ApiResult<Vec<ShareLinkView>> {
    reply(s.port.list(a.account.id, &session).await.map_err(failure)?)
}
async fn create<S: AccountStore, M: EmailSender, P: Sharing>(
    State(s): State<ShareState<S, M, P>>,
    SessionPost(a): SessionPost,
    Path(session): Path<String>,
    ApiBody(body): ApiBody<CreateShare>,
) -> ApiResult<ShareIssued> {
    let token = LinkToken::generate()?;
    let link = s
        .port
        .create(a.account.id, &session, body, token.digest())
        .await
        .map_err(failure)?;
    reply(ShareIssued {
        link,
        url: SecretReveal(format!("{}/s/{}", s.origin.as_str(), token.text())),
    })
}
async fn revoke<S: AccountStore, M: EmailSender, P: Sharing>(
    State(s): State<ShareState<S, M, P>>,
    SessionPost(a): SessionPost,
    Path((session, link)): Path<(String, String)>,
    ApiBody(_): ApiBody<crate::app_api::guard::Nothing>,
) -> ApiResult<RevokedView> {
    s.port
        .revoke(a.account.id, &session, &link)
        .await
        .map_err(failure)?;
    reply(RevokedView { revoked: 1 })
}

#[derive(Default, serde::Deserialize)]
struct Theme {
    theme: Option<String>,
}
async fn page<S: AccountStore, M: EmailSender, P: Sharing>(
    State(s): State<ShareState<S, M, P>>,
    Path(token): Path<String>,
    crate::security::ClientInfo(client): crate::security::ClientInfo,
) -> Response {
    public(s, token, false, "auto", client.ip).await
}
async fn embed<S: AccountStore, M: EmailSender, P: Sharing>(
    State(s): State<ShareState<S, M, P>>,
    Path(token): Path<String>,
    Query(theme): Query<Theme>,
    crate::security::ClientInfo(client): crate::security::ClientInfo,
) -> Response {
    public(s, token, true, theme.theme.as_deref().unwrap_or("auto"), client.ip).await
}
async fn public<S: AccountStore, M: EmailSender, P: Sharing>(
    state: ShareState<S, M, P>,
    token: String,
    embed: bool,
    theme: &str,
    ip: Option<std::net::IpAddr>,
) -> Response {
    let now = state.service.now_ms();
    let allowed=state.window.lock().map(|mut limits|limits.admit(ip,now)).unwrap_or(false);
    if !allowed {
        let mut r = render::missing(StatusCode::TOO_MANY_REQUESTS);
        r.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        return r;
    }
    let Some(parsed) = LinkToken::parse(&token) else {
        return render::missing(StatusCode::NOT_FOUND);
    };
    match state.port.public(parsed.digest()).await {
        Ok(summary) if !embed || summary.allow_embeds => {
            render::page(&summary, state.origin.as_str(), &token, embed, theme)
        }
        _ => render::missing(StatusCode::NOT_FOUND),
    }
}
