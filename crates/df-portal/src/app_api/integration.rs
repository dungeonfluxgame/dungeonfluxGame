//! Cookie + CSRF protected PAT lifecycle, separate from bearer-only reads.
use super::guard::{ApiBody, ApiFailure, ApiResult, GuardState, SessionPost, SignedIn, reply};
use axum::{
    Router,
    extract::{Path, State},
    routing::get,
};
use df_account_app::{
    wire::{ErrorCode, RevokedView},
    wire_integration::{
        CreatePersonalToken, PersonalTokenIssued, PersonalTokenView, RevokePersonalToken,
        SecretReveal,
    },
};
use df_customer::{
    AccountStore, CustomerService, EmailSender, Reauthentication,
    integration::{
        IssueToken, Peppers, PersonalTokenStore, Scope, TokenId, TokenMetadata, TokenRefusal,
    },
};
use std::sync::Arc;

pub struct TokenState<S, M> {
    service: CustomerService<S, M>,
    origin: crate::PortalOrigin,
    peppers: Arc<Peppers>,
}
impl<S, M> Clone for TokenState<S, M> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            origin: self.origin.clone(),
            peppers: Arc::clone(&self.peppers),
        }
    }
}
impl<S: AccountStore, M: EmailSender> GuardState for TokenState<S, M> {
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

pub fn router<S: AccountStore + PersonalTokenStore, M: EmailSender>(
    service: CustomerService<S, M>,
    origin: crate::PortalOrigin,
    peppers: Arc<Peppers>,
) -> Router {
    Router::new()
        .route(
            "/app/api/developer-tokens",
            get(list::<S, M>).post(create::<S, M>),
        )
        .route(
            "/app/api/developer-tokens/{token_id}/revoke",
            axum::routing::post(revoke::<S, M>),
        )
        .layer(axum::extract::DefaultBodyLimit::max(
            super::guard::MAXIMUM_BODY_BYTES,
        ))
        .with_state(TokenState {
            service,
            origin,
            peppers,
        })
}

pub(crate) fn token_view(m: TokenMetadata) -> PersonalTokenView {
    PersonalTokenView {
        token_id: m.id.text(),
        label: m.label,
        scopes: m.scopes.iter().map(|s| s.name().to_owned()).collect(),
        created_at_ms: m.created_at_ms,
        expires_at_ms: m.expires_at_ms,
        revoked_at_ms: m.revoked_at_ms,
        last_used_at_ms: m.last_used_at_ms,
    }
}
fn proof(password: Option<&SecretReveal>) -> Reauthentication<'_> {
    password.map_or(Reauthentication::RecentSignIn, |p| {
        Reauthentication::Password(&p.0)
    })
}
fn refused(reason: TokenRefusal) -> ApiFailure {
    if matches!(reason,TokenRefusal::ReauthenticationRequired) {
        return ApiFailure::new(df_account_app::wire::ApiError::refused(df_account_app::wire::Refusal::ReauthenticationRequired));
    }
    ApiFailure::of(match reason {
        TokenRefusal::ReauthenticationRequired | TokenRefusal::InactiveAccount => {
            ErrorCode::Forbidden
        }
        TokenRefusal::LimitReached => ErrorCode::Throttled,
        _ => ErrorCode::Invalid,
    })
}
async fn list<S: AccountStore + PersonalTokenStore, M: EmailSender>(
    State(state): State<TokenState<S, M>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<Vec<PersonalTokenView>> {
    reply(
        state
            .service
            .personal_tokens(&caller)
            .await?
            .into_iter()
            .map(token_view)
            .collect(),
    )
}
async fn create<S: AccountStore + PersonalTokenStore, M: EmailSender>(
    State(state): State<TokenState<S, M>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<CreatePersonalToken>,
) -> ApiResult<PersonalTokenIssued> {
    let scopes = body
        .scopes
        .iter()
        .map(|s| Scope::parse(s).ok_or_else(|| ApiFailure::of(ErrorCode::Invalid)))
        .collect::<Result<Vec<_>, _>>()?;
    match state
        .service
        .issue_personal_token(
            &caller,
            proof(body.password.as_ref()),
            &state.peppers,
            &body.label,
            scopes,
            body.expiry_days,
        )
        .await?
    {
        IssueToken::Issued(issued) => reply(PersonalTokenIssued {
            token: token_view(issued.metadata),
            secret: SecretReveal(issued.secret.reveal()),
        }),
        IssueToken::Refused(reason) => Err(refused(reason)),
    }
}
async fn revoke<S: AccountStore + PersonalTokenStore, M: EmailSender>(
    State(state): State<TokenState<S, M>>,
    SessionPost(caller): SessionPost,
    Path(id): Path<String>,
    ApiBody(body): ApiBody<RevokePersonalToken>,
) -> ApiResult<RevokedView> {
    let id = TokenId::parse(&id).ok_or_else(|| ApiFailure::of(ErrorCode::NotFound))?;
    let revoked = state
        .service
        .revoke_personal_token(&caller, proof(body.password.as_ref()), id)
        .await?
        .map_err(refused)?;
    if !revoked {
        return Err(ApiFailure::of(ErrorCode::NotFound));
    }
    reply(RevokedView { revoked: 1 })
}
