//! Native OAuth navigation; the browser receives only fixed local outcomes and opaque cookies.

use super::{
    guard::{self, ApiFailure, SameSite, Visitor},
    ports::Ports,
    state::AppApiState,
};
use crate::security::{self, ClientInfo, PRESESSION_COOKIE, SESSION_COOKIE};
use axum::{
    extract::{
        Form, Path, Query, Request, State,
        rejection::{FormRejection, QueryRejection},
    },
    http::{HeaderMap, header},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use df_account_app::wire::CSRF_FORM;
use df_customer::{
    AccountStore, EmailSender, OAuthIntent, OAuthOutcome, OAuthRefusal, OAuthStartForm,
};
use serde::Deserialize;

const ATTEMPT_COOKIE: &str = "__Host-df_oauth";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartForm {
    csrf: String,
    intent: String,
    #[serde(default)]
    is_adult: bool,
    #[serde(default)]
    accepts_terms: bool,
    #[serde(default)]
    terms_version: String,
    #[serde(default)]
    current_password: String,
}

fn form_is_authorized<S, M, P>(
    state: &AppApiState<S, M, P>,
    visitor: &Visitor,
    headers: &HeaderMap,
    csrf: &str,
) -> bool {
    if guard::check_form_shape(headers, state.origin.as_str()).is_err() {
        return false;
    }
    let secret = visitor
        .caller
        .as_ref()
        .map(|caller| &caller.csrf)
        .or(visitor.presession.as_ref());
    secret.is_some_and(|secret| secret.accepts(CSRF_FORM, csrf))
}

fn failure(reason: OAuthRefusal) -> Response {
    Redirect::to(&format!("/app/oauth/error/{}", reason.label())).into_response()
}

fn visitor_or_failure(visitor: Result<Visitor, ApiFailure>) -> Result<Visitor, OAuthRefusal> {
    visitor.map_err(|_error| {
        df_observe::record_audit_line(
            "account.oauth",
            "ACCOUNT oauth_session_unavailable".to_owned(),
        );
        OAuthRefusal::Unavailable
    })
}

pub(super) async fn start<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    visitor: Result<Visitor, ApiFailure>,
    headers: HeaderMap,
    ClientInfo(client): ClientInfo,
    form: Result<Form<StartForm>, FormRejection>,
) -> Response {
    let visitor = match visitor_or_failure(visitor) {
        Ok(visitor) => visitor,
        Err(reason) => return failure(reason),
    };
    let Ok(Form(form)) = form else {
        return failure(OAuthRefusal::ConsentRequired);
    };
    if !form_is_authorized(&state, &visitor, &headers, &form.csrf) {
        df_observe::record_audit_line(
            "account.oauth",
            "ACCOUNT oauth_start_refused outcome=csrf_refused".to_owned(),
        );
        return failure(OAuthRefusal::Expired);
    }
    let Some(intent) = OAuthIntent::parse(&form.intent) else {
        return failure(OAuthRefusal::Expired);
    };
    let outcome = state
        .service
        .start_google(
            OAuthStartForm {
                intent,
                adult_attested: form.is_adult && form.accepts_terms,
                terms_version: &form.terms_version,
                current_password: &form.current_password,
            },
            visitor.caller.as_ref(),
            client,
        )
        .await;
    match outcome {
        Ok(Ok(start)) => {
            let mut response = Redirect::to(start.authorization_url.expose()).into_response();
            response.headers_mut().append(
                header::SET_COOKIE,
                guard::secret_cookie(ATTEMPT_COOKIE, &start.cookie.text(), 600, SameSite::Lax),
            );
            response
        }
        Ok(Err(reason)) => failure(reason),
        Err(_) => failure(OAuthRefusal::Unavailable),
    }
}

#[derive(Deserialize)]
pub(super) struct Callback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

/// Runs outside extractors, so malformed queries and visitor/store failures also clear the cookie.
pub(super) async fn callback_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    if response.status().is_server_error() {
        response = failure(OAuthRefusal::Unavailable);
    }
    guard::apply_headers(response.headers_mut());
    response.headers_mut().append(
        header::SET_COOKIE,
        guard::clear_secret_cookie(ATTEMPT_COOKIE, SameSite::Lax),
    );
    response
}

pub(super) async fn callback<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    visitor: Result<Visitor, ApiFailure>,
    headers: HeaderMap,
    ClientInfo(client): ClientInfo,
    query: Result<Query<Callback>, QueryRejection>,
) -> Response {
    let visitor = match visitor_or_failure(visitor) {
        Ok(visitor) => visitor,
        Err(reason) => return failure(reason),
    };
    let Ok(Query(query)) = query else {
        df_observe::record_audit_line(
            "account.oauth",
            "ACCOUNT oauth_callback attempt=none outcome=invalid_query".to_owned(),
        );
        return failure(OAuthRefusal::Expired);
    };
    let outcome = state
        .service
        .finish_google(
            query.state.as_deref().unwrap_or(""),
            security::cookie(&headers, ATTEMPT_COOKIE).unwrap_or(""),
            query.code.as_deref(),
            query.error.is_some(),
            visitor.caller.as_ref(),
            client,
        )
        .await;
    let (issued, path) = match outcome {
        Ok(OAuthOutcome::SignedUp(issued)) => (issued, "/app/dashboard?welcome=1"),
        Ok(OAuthOutcome::SignedIn(issued)) => (
            issued,
            if visitor.caller.is_some() {
                "/app/settings"
            } else {
                "/app/dashboard"
            },
        ),
        Ok(OAuthOutcome::Linked) => return Redirect::to("/app/settings").into_response(),
        Ok(OAuthOutcome::Refused(OAuthRefusal::Canceled))
            if query.error.as_deref() != Some("access_denied") =>
        {
            return failure(OAuthRefusal::Unavailable);
        }
        Ok(OAuthOutcome::Refused(reason)) => return failure(reason),
        Err(_) => return failure(OAuthRefusal::Unavailable),
    };
    if let Some(previous) = visitor.caller
        && let Err(error) = state.service.logout(&previous, client).await
    {
        df_observe::record_audit_line(
            "account.oauth",
            format!(
                "ACCOUNT oauth_previous_session_kept outcome={}",
                error.label()
            ),
        );
    }
    let mut response = Redirect::to(path).into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        guard::secret_cookie(
            SESSION_COOKIE,
            &issued.token.text(),
            issued.max_age_s,
            SameSite::Lax,
        ),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        guard::clear_secret_cookie(PRESESSION_COOKIE, SameSite::Lax),
    );
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UnlinkForm {
    csrf: String,
    current_password: String,
}

pub(super) async fn unlink<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    visitor: Result<Visitor, ApiFailure>,
    headers: HeaderMap,
    form: Result<Form<UnlinkForm>, FormRejection>,
) -> Response {
    let visitor = match visitor_or_failure(visitor) {
        Ok(visitor) => visitor,
        Err(reason) => return failure(reason),
    };
    let Ok(Form(form)) = form else {
        return failure(OAuthRefusal::Expired);
    };
    if !form_is_authorized(&state, &visitor, &headers, &form.csrf) {
        return failure(OAuthRefusal::Expired);
    }
    let Some(caller) = visitor.caller else {
        return failure(OAuthRefusal::ReauthenticationRequired);
    };
    match state
        .service
        .unlink_google(&caller, &form.current_password)
        .await
    {
        Ok(Ok(())) => Redirect::to("/app/settings").into_response(),
        Ok(Err(reason)) => failure(reason),
        Err(_) => failure(OAuthRefusal::Unavailable),
    }
}

pub(super) async fn error(Path(reason): Path<String>) -> Response {
    let message = match reason.as_str() {
        "canceled" => "Google sign-in was canceled. Try again or return to sign in.",
        "email_not_verified" => {
            "Google could not verify your email. Verify it with Google and try again, or use your password."
        }
        "password_sign_in_required" => {
            "Sign in with your password first, then link Google in Settings > Sign-in methods."
        }
        "no_account" => "Create an account first, or sign in using your existing sign-in method.",
        "consent_required" => {
            "Return to sign up and confirm that you are 18 or older and accept the current terms and privacy notice."
        }
        "account_unavailable" => {
            "This account is closing, closed or unavailable. Use your existing account recovery options."
        }
        "conflict" => {
            "Google could not be linked. Use your existing sign-in method or contact support."
        }
        "reauthentication_required" => {
            "Confirm it is you again with Google before continuing. A fresh authentication within five minutes is required."
        }
        "last_sign_in_method" => "Google is your only sign-in method, so it cannot be removed.",
        "unavailable" | "disabled" | "throttled" => {
            "Google sign-in is temporarily unavailable. Please try again later."
        }
        _ => "This sign-in link expired or was already used. Start again.",
    };
    let mut response = Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Google sign-in | DungeonFlux</title><link rel=\"stylesheet\" href=\"/app/assets/app.css\"></head><body><main class=\"page\"><h1>Google sign-in</h1><p role=\"alert\">{message}</p><p><a href=\"/app/sign-in\">Return to sign in</a> <a href=\"/app/sign-up\">Return to sign up</a> <a href=\"/app/settings\">Settings</a></p></main></body></html>"
    )).into_response();
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static("default-src 'none'; style-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"));
    response
}
