//! Signed-in pages: overview, profile, security, sessions, and your data (export and closure).
//! Each page needs a valid session; each form, the session's CSRF token. When an identity
//! provider holds the passwords the password and email forms are absent (it manages them);
//! sensitive actions ask for the password either way.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use df_customer::{
    AccountStore, Authenticated, CloseResult, DisplayNameRefusal, EmailChangeResult, EmailSender,
    EventKind, ExportResult, PasswordChangeResult, ProfileResult, Reauthentication, ResendOutcome,
    RevokeScope,
};
use df_types::{OperationId, WebSessionId};
use serde::Deserialize;

use crate::{
    PortalState,
    auth_routes::{email_refusal_message, operation_text, password_refusal_message},
    html::{Markup, hex, markup, text},
    layout::{
        ButtonStyle, Field, FieldError, InputKind, Navigation, NoticeKind, button, error_summary,
        field, form_start, hidden, notice, page,
    },
    respond::{
        check_session_post, form_refused, html, redirect, service_failed, signed_in, throttled,
    },
    security::{ClientInfo, FormRefusal, SESSION_COOKIE, clear_cookie, set_cookie},
    time_text::utc,
};

/// The caller, or the response to send instead (a redirect to log in, or a failure page).
/// Boxed: a response is large and this is the uncommon path.
async fn caller<S: AccountStore, M: EmailSender>(
    state: &PortalState<S, M>,
    headers: &HeaderMap,
) -> Result<Authenticated, Box<Response>> {
    match signed_in(state, headers).await {
        Ok(Some(caller)) => Ok(caller),
        Ok(None) => Err(Box::new(redirect("/login"))),
        Err(error) => Err(Box::new(service_failed(error))),
    }
}

/// The caller of a `POST`, after its origin and CSRF checks.
async fn posting_caller<S: AccountStore, M: EmailSender>(
    state: &PortalState<S, M>,
    headers: &HeaderMap,
    form: &str,
    token: &str,
) -> Result<Authenticated, Box<Response>> {
    let caller = caller(state, headers).await?;
    check_session_post(headers, state.origin.as_str(), &caller, form, token)
        .map_err(|refusal| Box::new(form_refused(refusal)))?;
    Ok(caller)
}

fn signed_in_page(status: StatusCode, title: &str, main: Markup) -> Response {
    html(status, page(title, Navigation::SignedIn, main))
}

fn email_of(caller: &Authenticated) -> &str {
    caller
        .account
        .email
        .as_ref()
        .map_or("", |email| email.display())
}

#[derive(Deserialize)]
pub struct CsrfOnly {
    #[serde(default)]
    csrf: String,
}

// ---- Overview

pub async fn overview<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
) -> Response {
    let caller = match caller(&state, &headers).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    let verification = if caller.account.is_verified {
        Markup::default()
    } else {
        markup!(
            notice(
                NoticeKind::Warning,
                "Your email address is not verified yet. Open the link we emailed you; you need \
                 a verified address to add funds."
            ),
            form_start(
                "/verify-email/resend",
                &caller.csrf.form_token("verify-email-resend"),
                None
            ),
            button("Send the verification link again", ButtonStyle::Secondary),
            "</form>"
        )
    };
    let name = caller
        .account
        .display_name
        .as_ref()
        .map_or("not set", |name| name.as_str());
    signed_in_page(
        StatusCode::OK,
        "Your account",
        markup!(
            "<h1>Your account</h1>",
            verification,
            "<dl class=\"summary\"><dt>Email</dt><dd>",
            text(email_of(&caller)),
            "</dd><dt>Display name</dt><dd>",
            text(name),
            " <a href=\"/account/profile\">Change</a></dd><dt>Member since</dt><dd>",
            text(&utc(caller.account.created_at_ms)),
            "</dd></dl><ul class=\"links\"><li><a href=\"/account/security\">Sign-in and ",
            "security</a></li><li><a href=\"/account/sessions\">Signed-in devices</a></li>",
            "<li><a href=\"/account/data\">Download or close your account</a></li></ul>",
            form_start("/logout", &caller.csrf.form_token("logout"), None),
            button("Log out", ButtonStyle::Secondary),
            "</form>"
        ),
    )
}

pub async fn resend_verification<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
    axum::Form(body): axum::Form<CsrfOnly>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "verify-email-resend", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    let message = match state.service.resend_verification(&caller).await {
        Ok(ResendOutcome::Sent) => "We have sent a new verification link. It works for 24 hours.",
        Ok(ResendOutcome::AlreadyVerified) => "Your email address is already verified.",
        Ok(ResendOutcome::Throttled) => return throttled(),
        Err(error) => return service_failed(error),
    };
    signed_in_page(
        StatusCode::OK,
        "Verification",
        markup!(
            "<h1>Verification</h1>",
            notice(NoticeKind::Success, message),
            "<p><a href=\"/account\">Back to your account</a></p>"
        ),
    )
}

pub async fn logout<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<CsrfOnly>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "logout", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    if let Err(error) = state.service.logout(&caller, client).await {
        return service_failed(error);
    }
    let mut response = redirect("/");
    response
        .headers_mut()
        .append(header::SET_COOKIE, clear_cookie(SESSION_COOKIE));
    response
}

// ---- Profile

#[derive(Deserialize)]
pub struct ProfileBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    display_name: String,
}

fn profile_page(
    caller: &Authenticated,
    value: &str,
    banner: Markup,
    errors: &[FieldError],
) -> Markup {
    page(
        "Profile",
        Navigation::SignedIn,
        markup!(
            "<h1>Profile</h1>",
            banner,
            error_summary(errors),
            form_start("/account/profile", &caller.csrf.form_token("profile"), None),
            field(
                &Field {
                    name: "display_name",
                    label: "Display name",
                    kind: InputKind::Text,
                    autocomplete: "nickname",
                    value,
                    hint: Some(
                        "Shown on your account pages. Up to 60 characters; leave empty to remove it."
                    ),
                    required: false,
                },
                errors
            ),
            button("Save", ButtonStyle::Primary),
            "</form>"
        ),
    )
}

pub async fn profile_form<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
) -> Response {
    let caller = match caller(&state, &headers).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    let current = caller
        .account
        .display_name
        .as_ref()
        .map_or("", |name| name.as_str());
    html(
        StatusCode::OK,
        profile_page(&caller, current, Markup::default(), &[]),
    )
}

pub async fn profile<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
    axum::Form(body): axum::Form<ProfileBody>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "profile", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    let message = match state
        .service
        .update_profile(&caller, &body.display_name)
        .await
    {
        Ok(ProfileResult::Saved) => {
            return html(
                StatusCode::OK,
                profile_page(
                    &caller,
                    body.display_name.trim(),
                    notice(NoticeKind::Success, "Saved."),
                    &[],
                ),
            );
        }
        Ok(ProfileResult::Refused(DisplayNameRefusal::TooLong)) => "Use at most 60 characters.",
        Ok(ProfileResult::Refused(DisplayNameRefusal::ControlCharacter)) => {
            "Use letters, numbers, spaces and punctuation only."
        }
        Ok(ProfileResult::Throttled) => return throttled(),
        Err(error) => return service_failed(error),
    };
    html(
        StatusCode::UNPROCESSABLE_ENTITY,
        profile_page(
            &caller,
            &body.display_name,
            Markup::default(),
            &[FieldError {
                field: "display_name",
                message,
            }],
        ),
    )
}

// ---- Security: sign-in, password and email

fn event_label(kind: EventKind) -> &'static str {
    match kind {
        EventKind::SignedUp => "Account created",
        EventKind::Verified => "Email verified",
        EventKind::LoggedIn => "Signed in",
        EventKind::LoginRefused => "Failed sign-in attempt",
        EventKind::LoggedOut => "Signed out",
        EventKind::PasswordChanged => "Password changed",
        EventKind::PasswordReset => "Password reset by email",
        EventKind::EmailChangeRequested => "Email change requested",
        EventKind::EmailChanged => "Email address changed",
        EventKind::SessionsRevoked => "Devices signed out",
        EventKind::DataExported => "Data downloaded",
        EventKind::Closed => "Account closed",
        EventKind::Reopened => "Account reopened",
    }
}

fn password_field(
    name: &'static str,
    label: &'static str,
    autocomplete: &'static str,
    hint: Option<&'static str>,
) -> Field<'static> {
    Field {
        name,
        label,
        kind: InputKind::Password,
        autocomplete,
        value: "",
        hint,
        required: true,
    }
}

fn credential_forms(
    caller: &Authenticated,
    operation: &str,
    password_errors: &[FieldError],
    email_errors: &[FieldError],
) -> Markup {
    markup!(
        "<section><h2>Change your password</h2><p>Every other device will be signed out.</p>",
        error_summary(password_errors),
        form_start(
            "/account/security/password",
            &caller.csrf.form_token("password"),
            None
        ),
        field(
            &password_field("current", "Current password", "current-password", None),
            password_errors
        ),
        field(
            &password_field(
                "password",
                "New password",
                "new-password",
                Some("12 to 128 characters.")
            ),
            password_errors
        ),
        field(
            &password_field("confirmation", "New password again", "new-password", None),
            password_errors
        ),
        button("Change password", ButtonStyle::Primary),
        "</form></section><section><h2>Change your email address</h2>",
        error_summary(email_errors),
        form_start(
            "/account/security/email",
            &caller.csrf.form_token("email"),
            Some(operation)
        ),
        field(
            &Field {
                name: "new_email",
                label: "New email address",
                kind: InputKind::Email,
                autocomplete: "email",
                value: "",
                hint: Some("We email both addresses; the change happens when both confirm."),
                required: true,
            },
            email_errors
        ),
        field(
            &password_field(
                "email_password",
                "Current password",
                "current-password",
                None
            ),
            email_errors
        ),
        button("Change email address", ButtonStyle::Primary),
        "</form></section>"
    )
}

async fn security_response<S: AccountStore, M: EmailSender>(
    state: &PortalState<S, M>,
    caller: &Authenticated,
    status: StatusCode,
    banner: Markup,
    password_errors: &[FieldError],
    email_errors: &[FieldError],
) -> Response {
    let (overview, operation) = match (
        state.service.security_overview(caller).await,
        state.service.new_operation(),
    ) {
        (Ok(overview), Ok(operation)) => (overview, operation),
        (Err(error), _) | (_, Err(error)) => return service_failed(error),
    };
    let rows = Markup::concat(overview.events.iter().map(|event| {
        markup!(
            "<tr><td>",
            text(event_label(event.kind)),
            "</td><td>",
            text(&utc(event.at_ms)),
            "</td><td>",
            text(event.device.label()),
            "</td></tr>"
        )
    }));
    let credentials = if state.service.uses_local_passwords() {
        credential_forms(
            caller,
            &operation_text(&operation),
            password_errors,
            email_errors,
        )
    } else {
        markup!(
            "<section><h2>Password</h2><p>Your password is managed by our sign-in service. To ",
            "change it, sign out and choose \"Forgot password\" on the sign-in page.</p></section>"
        )
    };
    signed_in_page(
        status,
        "Security",
        markup!(
            "<h1>Sign-in and security</h1>",
            banner,
            "<p>Signed in as <strong>",
            text(email_of(caller)),
            "</strong>.</p>",
            credentials,
            "<section><h2>Recent security activity</h2><table><thead><tr>",
            "<th scope=\"col\">What</th><th scope=\"col\">When</th><th scope=\"col\">Device</th>",
            "</tr></thead><tbody>",
            rows,
            "</tbody></table></section>"
        ),
    )
}

pub async fn security<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
) -> Response {
    match caller(&state, &headers).await {
        Ok(caller) => {
            security_response(&state, &caller, StatusCode::OK, Markup::default(), &[], &[]).await
        }
        Err(response) => *response,
    }
}

#[derive(Deserialize)]
pub struct PasswordBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    current: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    confirmation: String,
}

pub async fn change_password<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<PasswordBody>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "password", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    let error = match state
        .service
        .change_password(
            &caller,
            &body.current,
            &body.password,
            &body.confirmation,
            client,
        )
        .await
    {
        Ok(PasswordChangeResult::Changed(issued)) => {
            // The old session is revoked; the browser gets the replacement in the same response.
            let mut response = redirect("/account");
            response.headers_mut().append(
                header::SET_COOKIE,
                set_cookie(SESSION_COOKIE, &issued.token.text(), issued.max_age_s),
            );
            return response;
        }
        Ok(PasswordChangeResult::WrongPassword) => FieldError {
            field: "current",
            message: "That is not your current password.",
        },
        Ok(PasswordChangeResult::Mismatch) => FieldError {
            field: "confirmation",
            message: "The two new passwords are different.",
        },
        Ok(PasswordChangeResult::Refused(refusal)) => FieldError {
            field: "password",
            message: password_refusal_message(refusal),
        },
        Ok(PasswordChangeResult::ProviderManaged) => FieldError {
            field: "current",
            message: "Your password is managed by our sign-in service; use \"Forgot password\".",
        },
        Ok(PasswordChangeResult::Throttled) => return throttled(),
        Err(error) => return service_failed(error),
    };
    security_response(
        &state,
        &caller,
        StatusCode::UNPROCESSABLE_ENTITY,
        Markup::default(),
        &[error],
        &[],
    )
    .await
}

#[derive(Deserialize)]
pub struct EmailBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    operation: String,
    #[serde(default)]
    new_email: String,
    #[serde(default)]
    email_password: String,
}

pub async fn change_email<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
    axum::Form(body): axum::Form<EmailBody>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "email", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    let Ok(operation) = OperationId::from_hex(&body.operation) else {
        return form_refused(FormRefusal::BadToken);
    };
    let error = match state
        .service
        .request_email_change(&caller, &body.email_password, &body.new_email, operation)
        .await
    {
        Ok(EmailChangeResult::ConfirmationSent) => {
            return security_response(
                &state,
                &caller,
                StatusCode::OK,
                notice(
                    NoticeKind::Success,
                    "We have emailed both addresses. The change happens when both confirm \
                     within 24 hours.",
                ),
                &[],
                &[],
            )
            .await;
        }
        Ok(EmailChangeResult::WrongPassword) => FieldError {
            field: "email_password",
            message: "That is not your current password.",
        },
        Ok(EmailChangeResult::Refused(refusal)) => FieldError {
            field: "new_email",
            message: email_refusal_message(refusal),
        },
        Ok(EmailChangeResult::SameAddress) => FieldError {
            field: "new_email",
            message: "That is already your email address.",
        },
        Ok(EmailChangeResult::ProviderManaged) => FieldError {
            field: "new_email",
            message: "Changing your address is not available yet for your account.",
        },
        Ok(EmailChangeResult::StaleForm) => return form_refused(FormRefusal::BadToken),
        Ok(EmailChangeResult::Throttled) => return throttled(),
        Err(error) => return service_failed(error),
    };
    security_response(
        &state,
        &caller,
        StatusCode::UNPROCESSABLE_ENTITY,
        Markup::default(),
        &[],
        &[error],
    )
    .await
}

// ---- Sessions

pub async fn sessions<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
) -> Response {
    let caller = match caller(&state, &headers).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    let overview = match state.service.security_overview(&caller).await {
        Ok(overview) => overview,
        Err(error) => return service_failed(error),
    };
    let revoke_token = caller.csrf.form_token("revoke");
    let rows = Markup::concat(overview.sessions.iter().map(|session| {
        let action = if session.id == caller.session {
            markup!("<strong>This device</strong>")
        } else {
            markup!(
                form_start("/account/sessions/revoke", &revoke_token, None),
                hidden("session", &hex(session.id.as_bytes())),
                button("Sign out this device", ButtonStyle::Secondary),
                "</form>"
            )
        };
        markup!(
            "<tr><td>",
            text(session.device.label()),
            "</td><td>",
            text(&utc(session.created_at_ms)),
            "</td><td>",
            text(&utc(session.last_seen_at_ms)),
            "</td><td>",
            action,
            "</td></tr>"
        )
    }));
    signed_in_page(
        StatusCode::OK,
        "Signed-in devices",
        markup!(
            "<h1>Signed-in devices</h1><table><thead><tr><th scope=\"col\">Device</th>",
            "<th scope=\"col\">Signed in</th><th scope=\"col\">Last used</th>",
            "<th scope=\"col\"><span class=\"visually-hidden\">Action</span></th></tr></thead>",
            "<tbody>",
            rows,
            "</tbody></table><div class=\"actions\">",
            form_start(
                "/account/sessions/revoke-others",
                &caller.csrf.form_token("revoke-others"),
                None
            ),
            button("Sign out all other devices", ButtonStyle::Secondary),
            "</form>",
            form_start(
                "/account/sessions/revoke-all",
                &caller.csrf.form_token("revoke-all"),
                None
            ),
            button("Sign out everywhere, including here", ButtonStyle::Danger),
            "</form></div>"
        ),
    )
}

#[derive(Deserialize)]
pub struct RevokeBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    session: String,
}

pub async fn revoke_one<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<RevokeBody>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "revoke", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    // An unknown or another account's id revokes nothing and answers the same way.
    if let Ok(session) = WebSessionId::from_hex(&body.session)
        && let Err(error) = state
            .service
            .revoke_sessions(&caller, RevokeScope::One(session), client)
            .await
    {
        return service_failed(error);
    }
    redirect("/account/sessions")
}

pub async fn revoke_others<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<CsrfOnly>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "revoke-others", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    match state
        .service
        .revoke_sessions(&caller, RevokeScope::AllExcept(caller.session), client)
        .await
    {
        Ok(_) => redirect("/account/sessions"),
        Err(error) => service_failed(error),
    }
}

pub async fn revoke_all<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<CsrfOnly>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "revoke-all", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    match state
        .service
        .revoke_sessions(&caller, RevokeScope::All, client)
        .await
    {
        Ok(_) => {
            let mut response = redirect("/login");
            response
                .headers_mut()
                .append(header::SET_COOKIE, clear_cookie(SESSION_COOKIE));
            response
        }
        Err(error) => service_failed(error),
    }
}

// ---- Your data: export and closure

/// The re-authentication part of a sensitive form: the password field.
fn reauthentication_field(name: &'static str, errors: &[FieldError]) -> Markup {
    field(
        &password_field(name, "Your password", "current-password", None),
        errors,
    )
}

fn data_page(
    caller: &Authenticated,
    export_errors: &[FieldError],
    close_errors: &[FieldError],
) -> Markup {
    page(
        "Your data",
        Navigation::SignedIn,
        markup!(
            "<h1>Your data</h1>",
            "<section><h2>Download your data</h2><p>A file with your account details and ",
            "security history. It never includes game content or other people's data.</p>",
            error_summary(export_errors),
            form_start(
                "/account/data/export",
                &caller.csrf.form_token("export"),
                None
            ),
            reauthentication_field("export_password", export_errors),
            button("Download my data", ButtonStyle::Primary),
            "</form></section><section><h2>Close your account</h2><p>Closing signs out every ",
            "device. You can reopen the account by signing in within 30 days; after that its ",
            "personal data is deleted and cannot be recovered.</p>",
            error_summary(close_errors),
            form_start("/account/close", &caller.csrf.form_token("close"), None),
            reauthentication_field("close_password", close_errors),
            button("Close my account", ButtonStyle::Danger),
            "</form></section>"
        ),
    )
}

pub async fn data<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
) -> Response {
    match caller(&state, &headers).await {
        Ok(caller) => html(StatusCode::OK, data_page(&caller, &[], &[])),
        Err(response) => *response,
    }
}

fn proof(password: &str) -> Reauthentication<'_> {
    Reauthentication::Password(password)
}

/// The answer to a failed re-authentication: a field error on the password.
fn not_reauthenticated(
    caller: &Authenticated,
    export: bool,
    password_field_name: &'static str,
) -> Response {
    let errors = [FieldError {
        field: password_field_name,
        message: "That is not your password.",
    }];
    let (export_errors, close_errors): (&[FieldError], &[FieldError]) = if export {
        (&errors, &[])
    } else {
        (&[], &errors)
    };
    html(
        StatusCode::UNPROCESSABLE_ENTITY,
        data_page(caller, export_errors, close_errors),
    )
}

#[derive(Deserialize)]
pub struct ExportBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    export_password: String,
}

pub async fn export<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<ExportBody>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "export", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    match state
        .service
        .export(&caller, proof(&body.export_password), client)
        .await
    {
        Ok(ExportResult::Document(document)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/json; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"dungeonflux-account.json\"",
                ),
            ],
            document,
        )
            .into_response(),
        Ok(ExportResult::NotReauthenticated) => {
            not_reauthenticated(&caller, true, "export_password")
        }
        Ok(ExportResult::Throttled) => throttled(),
        Err(error) => service_failed(error),
    }
}

/// The campaigns that stop a closure, and the archived ones the owner may still settle.
fn blockers_list(check: &df_customer::ClosureCheck) -> Markup {
    let blocking = Markup::concat(check.blocking.iter().map(|campaign| {
        let why = match campaign.reason {
            df_customer::BlockReason::Owns => "you own it; transfer it or archive it",
            df_customer::BlockReason::Hosts => "you host it; hand over hosting",
            df_customer::BlockReason::OwnsAndHosts => {
                "you own and host it; transfer it or archive it"
            }
        };
        markup!("<li>", text(&campaign.name), " (", why, ")</li>")
    }));
    let archived = Markup::concat(check.owned_archived.iter().map(|campaign| {
        markup!(
            "<li>",
            text(&campaign.name),
            " (archived; transfer it or delete it, or it is deleted after the closing period)</li>"
        )
    }));
    markup!("<ul>", blocking, archived, "</ul>")
}

#[derive(Deserialize)]
pub struct CloseBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    close_password: String,
}

pub async fn close<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
    axum::Form(body): axum::Form<CloseBody>,
) -> Response {
    let caller = match posting_caller(&state, &headers, "close", &body.csrf).await {
        Ok(caller) => caller,
        Err(response) => return *response,
    };
    match state
        .service
        .close(&caller, proof(&body.close_password))
        .await
    {
        Ok(CloseResult::Closed { delete_after_ms }) => {
            let mut response = html(
                StatusCode::OK,
                page(
                    "Account closed",
                    Navigation::Visitor,
                    markup!(
                        "<h1>Your account is closed</h1><p>Every device was signed out. Sign in ",
                        "before ",
                        text(&utc(delete_after_ms)),
                        " to reopen it; after that its personal data is deleted.</p>"
                    ),
                ),
            );
            response
                .headers_mut()
                .append(header::SET_COOKIE, clear_cookie(SESSION_COOKIE));
            response
        }
        Ok(CloseResult::Blocked(check)) => html(
            StatusCode::CONFLICT,
            page(
                "Settle these first",
                Navigation::SignedIn,
                markup!(
                    "<h1>Settle these first</h1><p>Your account was not closed. These campaigns ",
                    "need you first.</p>",
                    blockers_list(&check),
                    "<p><a href=\"/app\">Open your campaigns</a></p>"
                ),
            ),
        ),
        Ok(CloseResult::NotReauthenticated) => {
            not_reauthenticated(&caller, false, "close_password")
        }
        Ok(CloseResult::Throttled) => throttled(),
        Err(error) => service_failed(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostile_value_in_any_table_cell_is_escaped() {
        // Device labels are a closed set today; the cell escapes them anyway (security review).
        let cell = markup!("<td>", text("<script>alert(1)</script>"), "</td>");
        assert_eq!(
            cell.as_str(),
            "<td>&lt;script&gt;alert(1)&lt;/script&gt;</td>"
        );
    }
}
