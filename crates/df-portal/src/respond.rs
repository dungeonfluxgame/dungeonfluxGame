//! Response helpers shared by the route modules: pages, redirects, refusals and failures, and
//! the caller's session or pre-session.

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use df_customer::{AccountStore, Authenticated, CsrfSecret, EmailSender, ServiceError};

use crate::{
    PortalState,
    html::{Markup, markup},
    layout::{Navigation, NoticeKind, notice, page},
    security::{
        FormRefusal, PRESESSION_COOKIE, PRESESSION_MAX_AGE_S, SESSION_COOKIE, check_csrf,
        check_post_origin, cookie, presession, set_cookie,
    },
};

pub fn html(status: StatusCode, body: Markup) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body.into_string(),
    )
        .into_response()
}

pub fn ok_page(title: &str, navigation: Navigation, main: Markup) -> Response {
    html(StatusCode::OK, page(title, navigation, main))
}

/// `303 See Other` to a path on this origin.
pub fn redirect(location: &'static str) -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response()
}

pub fn form_refused(refusal: FormRefusal) -> Response {
    df_observe::record_audit_line(
        "account.csrf_refused",
        format!("ACCOUNT csrf_refused reason={}", refusal.label()),
    );
    html(
        StatusCode::FORBIDDEN,
        page(
            "Form expired",
            Navigation::Visitor,
            markup!(
                "<h1>This form has expired</h1><p>For your safety we could not accept it. Go ",
                "back, reload the page and send the form again.</p>"
            ),
        ),
    )
}

/// A failure the person can only wait out. `Unknown` says to look before repeating.
pub fn service_failed(error: ServiceError) -> Response {
    df_observe::record_audit_line(
        "account.failed",
        format!("ACCOUNT failed reason={}", error.label()),
    );
    let message = match error {
        ServiceError::Busy => "We are very busy right now. Wait a minute and try again.",
        ServiceError::Unknown => {
            "We could not confirm whether that went through. Reload your account page to check \
             before trying again."
        }
        ServiceError::Unavailable
        | ServiceError::InvalidRecord
        | ServiceError::RandomUnavailable => {
            "Something went wrong on our side and nothing was changed. Try again in a few minutes."
        }
    };
    html(
        StatusCode::SERVICE_UNAVAILABLE,
        page(
            "Try again",
            Navigation::Visitor,
            markup!(
                "<h1>Please try again</h1>",
                notice(NoticeKind::Warning, message)
            ),
        ),
    )
}

pub fn throttled() -> Response {
    html(
        StatusCode::TOO_MANY_REQUESTS,
        page(
            "Too many attempts",
            Navigation::Visitor,
            markup!(
                "<h1>Too many attempts</h1><p>For your safety we have paused this for a little ",
                "while. Wait a few minutes and try again.</p>"
            ),
        ),
    )
}

/// The caller's current session, if its cookie is valid.
pub async fn signed_in<S: AccountStore, M: EmailSender>(
    state: &PortalState<S, M>,
    headers: &HeaderMap,
) -> Result<Option<Authenticated>, ServiceError> {
    match cookie(headers, SESSION_COOKIE) {
        Some(token) => state.service.authenticate(token).await,
        None => Ok(None),
    }
}

/// The pre-session secret for an anonymous form, creating one (with its cookie) if absent.
pub fn presession_for_form(
    headers: &HeaderMap,
) -> Result<(CsrfSecret, Option<HeaderValue>), ServiceError> {
    if let Some(secret) = presession(headers) {
        return Ok((secret, None));
    }
    let secret = CsrfSecret::generate().map_err(|_| ServiceError::RandomUnavailable)?;
    let cookie = set_cookie(PRESESSION_COOKIE, &secret.text(), PRESESSION_MAX_AGE_S);
    Ok((secret, Some(cookie)))
}

/// Adds a `Set-Cookie` to a response when one is given.
pub fn with_cookie(mut response: Response, cookie: Option<HeaderValue>) -> Response {
    if let Some(cookie) = cookie {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

/// Checks an anonymous form: origin, then the pre-session CSRF token.
pub fn check_anonymous_post(
    headers: &HeaderMap,
    origin: &str,
    form: &str,
    token: &str,
) -> Result<CsrfSecret, FormRefusal> {
    check_post_origin(headers, origin)?;
    let secret = presession(headers);
    check_csrf(secret.as_ref(), form, token)?;
    secret.ok_or(FormRefusal::BadToken)
}

/// Checks a signed-in form: origin, then the session's CSRF token.
pub fn check_session_post(
    headers: &HeaderMap,
    origin: &str,
    caller: &Authenticated,
    form: &str,
    token: &str,
) -> Result<(), FormRefusal> {
    check_post_origin(headers, origin)?;
    check_csrf(Some(&caller.csrf), form, token)
}
