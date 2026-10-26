//! `GET session` (who is signed in, plus the CSRF token for this session or anonymous visit) and
//! `GET operation` (a fresh idempotency key).

use axum::extract::State;
use df_account_app::wire::{AccountView, CSRF_FORM, OperationView, OperatorView, SessionView};
use df_customer::{AccountStore, Authenticated, EmailSender};

use super::{
    guard::{ApiFailure, ApiResult, Reply, Visitor, presession_for_visitor, reply},
    ids::hex,
    ports::{OperatorDirectory, Ports},
    state::AppApiState,
};

/// The session view of a signed-in caller, with the operator's capabilities when it has any.
pub(super) async fn signed_in_view<S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    caller: &Authenticated,
) -> Result<SessionView, ApiFailure> {
    let operator = state
        .ports
        .operators()
        .grant(caller.account.id)
        .await?
        .map(|grant| OperatorView {
            capabilities: grant.capabilities,
            has_second_factor: grant.has_second_factor,
        });
    Ok(SessionView {
        google_enabled: state.service.google_enabled(),
        csrf: caller.csrf.form_token(CSRF_FORM),
        account: Some(AccountView {
            email: caller
                .account
                .email
                .as_ref()
                .map_or_else(String::new, |email| email.display().to_owned()),
            display_name: caller
                .account
                .display_name
                .as_ref()
                .map(|name| name.as_str().to_owned()),
            is_verified: caller.account.is_verified,
            operator,
        }),
        dev_sign_in: None,
    })
}

pub(super) async fn get_session<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    visitor: Visitor,
) -> ApiResult<SessionView> {
    if let Some(caller) = &visitor.caller {
        return reply(signed_in_view(&state, caller).await?);
    }
    let (secret, cookie) = presession_for_visitor(visitor.presession)?;
    let view = SessionView {
        google_enabled: state.service.google_enabled(),
        csrf: secret.form_token(CSRF_FORM),
        account: None,
        dev_sign_in: None,
    };
    let mut answer = Reply::new(view);
    if let Some(cookie) = cookie {
        answer = answer.with_cookie(cookie);
    }
    Ok(answer)
}

pub(super) async fn get_operation<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
) -> ApiResult<OperationView> {
    let operation = state.service.new_operation()?;
    reply(OperationView {
        operation: hex(operation.as_bytes()),
    })
}
