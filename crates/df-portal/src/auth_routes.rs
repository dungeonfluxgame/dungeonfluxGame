//! Anonymous pages: sign up, verify email and choose a password, log in, reset a password. They
//! serve whichever credential source the service has (local passwords for development and
//! self-hosting, or the identity provider); confirming an email change is a local-passwords
//! page only. Every form carries a pre-session CSRF token; answers that could reveal whether an
//! address has an account are the same page for both (plan 6.3).

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use df_customer::{
    AccountStore, EmailChangeDecision, EmailConfirmResult, EmailRefusal, EmailSender, LoginOutcome,
    PasswordRefusal, ResetConfirmOutcome, ResetRequestOutcome, SignupForm, SignupOutcome,
    SignupRefusal, VerifyWithPasswordOutcome,
};
use df_types::OperationId;
use serde::Deserialize;

use crate::{
    PortalState,
    html::{Markup, hex, markup, text},
    layout::{
        ButtonStyle, Field, FieldError, InputKind, Navigation, button, error_summary, field,
        form_start, hidden, page,
    },
    respond::{
        check_anonymous_post, form_refused, html, ok_page, presession_for_form, redirect,
        service_failed, throttled, with_cookie,
    },
    security::{
        ClientInfo, FormRefusal, PRESESSION_COOKIE, SESSION_COOKIE, clear_cookie, set_cookie,
    },
};

#[derive(Deserialize)]
pub struct TokenQuery {
    #[serde(default)]
    token: String,
}

pub fn email_refusal_message(refusal: EmailRefusal) -> &'static str {
    match refusal {
        EmailRefusal::Empty => "Enter your email address.",
        EmailRefusal::TooLong => "That email address is too long.",
        EmailRefusal::Malformed => "Enter an email address like name@example.com.",
    }
}

pub fn password_refusal_message(refusal: PasswordRefusal) -> &'static str {
    match refusal {
        PasswordRefusal::TooShort => "Use at least 12 characters.",
        PasswordRefusal::TooLong => "Use at most 128 characters.",
        PasswordRefusal::MatchesEmail => "Choose a password that is not your email address.",
        PasswordRefusal::Common => "That password is too common. Choose something less guessable.",
    }
}

/// The hidden-field form of a form's operation id.
pub fn operation_text(operation: &OperationId) -> String {
    hex(operation.as_bytes())
}

const PASSWORD_HINT: &str = "12 to 128 characters. A phrase of a few unusual words works well.";

fn email_field<'a>(value: &'a str, autocomplete: &'static str) -> Field<'a> {
    Field {
        name: "email",
        label: "Email address",
        kind: InputKind::Email,
        autocomplete,
        value,
        hint: None,
        required: true,
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

/// Signs the browser in: the session cookie, and the spent pre-session cookie cleared so a
/// token from before login is never promoted.
pub fn signed_in_redirect(location: &'static str, token: &str, max_age_s: u64) -> Response {
    let mut response = redirect(location);
    let headers = response.headers_mut();
    headers.append(
        header::SET_COOKIE,
        set_cookie(SESSION_COOKIE, token, max_age_s),
    );
    headers.append(header::SET_COOKIE, clear_cookie(PRESESSION_COOKIE));
    response
}

// ---- Sign up

#[derive(Deserialize)]
pub struct SignupBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    operation: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    adult_attested: bool,
    #[serde(default)]
    terms_version: String,
}

fn signup_page(
    csrf: &str,
    operation: &str,
    email: &str,
    terms_version: &str,
    errors: &[FieldError],
) -> Markup {
    page(
        "Create your account",
        Navigation::Visitor,
        markup!(
            "<h1>Create your DungeonFlux account</h1><p class=\"lead\">Every player signs in; ",
            "only the people who choose to fund a table pay.</p>",
            error_summary(errors),
            form_start("/signup", csrf, Some(operation)),
            field(&email_field(email, "email"), errors),
            field(
                &password_field("password", "Password", "new-password", Some(PASSWORD_HINT)),
                errors
            ),
            "<label><input type=\"checkbox\" name=\"terms_version\" value=\"",
            text(terms_version),
            "\" required> I agree to the current terms and privacy notice.</label>",
            "<label><input type=\"checkbox\" name=\"adult_attested\" value=\"true\" required> I confirm I am an adult.</label>",
            button("Create account", ButtonStyle::Primary),
            "</form><p>Already have an account? <a href=\"/login\">Log in</a></p>"
        ),
    )
}

pub async fn signup_form<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    headers: HeaderMap,
) -> Response {
    let (secret, cookie) = match presession_for_form(&headers) {
        Ok(found) => found,
        Err(error) => return service_failed(error),
    };
    let operation = match state.service.new_operation() {
        Ok(operation) => operation,
        Err(error) => return service_failed(error),
    };
    let body = signup_page(
        &secret.form_token("signup"),
        &operation_text(&operation),
        "",
        state.service.current_terms_version(),
        &[],
    );
    with_cookie(html(StatusCode::OK, body), cookie)
}

pub async fn signup<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<SignupBody>,
) -> Response {
    let secret = match check_anonymous_post(&headers, state.origin.as_str(), "signup", &body.csrf) {
        Ok(secret) => secret,
        Err(refusal) => return form_refused(refusal),
    };
    let Ok(operation) = OperationId::from_hex(&body.operation) else {
        return form_refused(FormRefusal::BadToken);
    };
    let outcome = state
        .service
        .signup(
            SignupForm {
                email: &body.email,
                password: &body.password,
                operation,
                adult_attested: body.adult_attested,
                terms_version: body.terms_version.clone(),
            },
            client,
        )
        .await;
    let error = match outcome {
        Ok(SignupOutcome::CheckYourEmail) => {
            return ok_page(
                "Check your email",
                Navigation::Visitor,
                markup!(
                    "<h1>Check your email</h1><p>We have sent a message to <strong>",
                    text(body.email.trim()),
                    "</strong>. Open the link in it to verify your address; it works for 24 ",
                    "hours.</p><p>Nothing arrived? Check your spam folder, or <a href=\"/login\">",
                    "log in</a> and ask for another link.</p>"
                ),
            );
        }
        Ok(SignupOutcome::Throttled) => return throttled(),
        Ok(SignupOutcome::Refused(SignupRefusal::Email(refusal))) => FieldError {
            field: "email",
            message: email_refusal_message(refusal),
        },
        Ok(SignupOutcome::Refused(SignupRefusal::Password(refusal))) => FieldError {
            field: "password",
            message: password_refusal_message(refusal),
        },
        Ok(SignupOutcome::Refused(SignupRefusal::PasswordRejected)) => FieldError {
            field: "password",
            message: "Our sign-in service did not accept that password. Choose a longer or \
                      less common one.",
        },
        Ok(SignupOutcome::Refused(SignupRefusal::AdultAttestationRequired)) => FieldError {
            field: "adult_attested",
            message: "Confirm that you are an adult to create an account.",
        },
        Ok(SignupOutcome::Refused(SignupRefusal::TermsVersionChanged)) => FieldError {
            field: "terms_version",
            message: "Review and accept the current terms before creating an account.",
        },
        Ok(SignupOutcome::Refused(SignupRefusal::StaleForm)) => {
            return form_refused(FormRefusal::BadToken);
        }
        Err(error) => return service_failed(error),
    };
    let fresh = match state.service.new_operation() {
        Ok(operation) => operation,
        Err(error) => return service_failed(error),
    };
    html(
        StatusCode::UNPROCESSABLE_ENTITY,
        signup_page(
            &secret.form_token("signup"),
            &operation_text(&fresh),
            &body.email,
            state.service.current_terms_version(),
            &[error],
        ),
    )
}

// ---- Verify email and choose a password

fn verify_page(csrf: &str, token: &str, errors: &[FieldError]) -> Markup {
    page(
        "Verify your email",
        Navigation::Visitor,
        markup!(
            "<h1>Verify your email address</h1><p>Choose the password for this account. It ",
            "replaces any password set when the account was created, and every device signed ",
            "in before now is signed out.</p>",
            error_summary(errors),
            form_start("/verify-email", csrf, None),
            hidden("token", token),
            field(
                &password_field("password", "Password", "new-password", Some(PASSWORD_HINT)),
                errors
            ),
            field(
                &password_field("confirmation", "Password again", "new-password", None),
                errors
            ),
            button("Verify my email", ButtonStyle::Primary),
            "</form>"
        ),
    )
}

pub async fn verify_form<S: AccountStore, M: EmailSender>(
    State(_state): State<PortalState<S, M>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response {
    let (secret, cookie) = match presession_for_form(&headers) {
        Ok(found) => found,
        Err(error) => return service_failed(error),
    };
    let body = verify_page(&secret.form_token("verify-email"), &query.token, &[]);
    with_cookie(html(StatusCode::OK, body), cookie)
}

#[derive(Deserialize)]
pub struct VerifyBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    confirmation: String,
}

pub async fn verify<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<VerifyBody>,
) -> Response {
    let secret =
        match check_anonymous_post(&headers, state.origin.as_str(), "verify-email", &body.csrf) {
            Ok(secret) => secret,
            Err(refusal) => return form_refused(refusal),
        };
    let outcome = state
        .service
        .verify_email_and_set_password(&body.token, &body.password, &body.confirmation, client)
        .await;
    let error = match outcome {
        Ok(VerifyWithPasswordOutcome::Verified | VerifyWithPasswordOutcome::AlreadyVerified) => {
            return ok_page(
                "Email verified",
                Navigation::Visitor,
                markup!(
                    "<h1>Your email address is verified</h1><p><a class=\"button primary\" ",
                    "href=\"/login\">Log in</a></p>"
                ),
            );
        }
        Ok(VerifyWithPasswordOutcome::InvalidLink) => {
            return html(
                StatusCode::BAD_REQUEST,
                page(
                    "Link expired",
                    Navigation::Visitor,
                    markup!(
                        "<h1>This link has expired or was already used</h1><p><a href=\"/login\">",
                        "Log in</a> and ask for a new verification link from your account page.</p>"
                    ),
                ),
            );
        }
        Ok(VerifyWithPasswordOutcome::Throttled) => return throttled(),
        Ok(VerifyWithPasswordOutcome::Mismatch) => FieldError {
            field: "confirmation",
            message: "The two passwords are different.",
        },
        Ok(VerifyWithPasswordOutcome::Refused(refusal)) => FieldError {
            field: "password",
            message: password_refusal_message(refusal),
        },
        Ok(VerifyWithPasswordOutcome::ProviderRejectedPassword) => FieldError {
            field: "password",
            message: "Our sign-in service did not accept that password. Choose a longer or \
                      less common one.",
        },
        Err(error) => return service_failed(error),
    };
    html(
        StatusCode::UNPROCESSABLE_ENTITY,
        verify_page(&secret.form_token("verify-email"), &body.token, &[error]),
    )
}

// ---- Log in

#[derive(Deserialize)]
pub struct LoginBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
}

fn login_page(csrf: &str, email: &str, errors: &[FieldError]) -> Markup {
    page(
        "Log in",
        Navigation::Visitor,
        markup!(
            "<h1>Log in</h1>",
            error_summary(errors),
            form_start("/login", csrf, None),
            field(&email_field(email, "username"), errors),
            field(
                &password_field("password", "Password", "current-password", None),
                errors
            ),
            button("Log in", ButtonStyle::Primary),
            "</form><p><a href=\"/password-reset\">Forgot your password?</a></p>",
            "<p>New here? <a href=\"/signup\">Create an account</a></p>"
        ),
    )
}

pub async fn login_form<S: AccountStore, M: EmailSender>(
    State(_state): State<PortalState<S, M>>,
    headers: HeaderMap,
) -> Response {
    let (secret, cookie) = match presession_for_form(&headers) {
        Ok(found) => found,
        Err(error) => return service_failed(error),
    };
    with_cookie(
        html(
            StatusCode::OK,
            login_page(&secret.form_token("login"), "", &[]),
        ),
        cookie,
    )
}

pub async fn login<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<LoginBody>,
) -> Response {
    let secret = match check_anonymous_post(&headers, state.origin.as_str(), "login", &body.csrf) {
        Ok(secret) => secret,
        Err(refusal) => return form_refused(refusal),
    };
    match state
        .service
        .login(&body.email, &body.password, client)
        .await
    {
        Ok(LoginOutcome::SignedIn(issued)) => {
            signed_in_redirect(state.home, &issued.token.text(), issued.max_age_s)
        }
        Ok(LoginOutcome::Refused) => html(
            StatusCode::UNPROCESSABLE_ENTITY,
            login_page(
                &secret.form_token("login"),
                &body.email,
                &[FieldError {
                    field: "email",
                    message: "Email or password is incorrect.",
                }],
            ),
        ),
        Ok(LoginOutcome::VerificationRequired) => html(
            StatusCode::UNPROCESSABLE_ENTITY,
            login_page(
                &secret.form_token("login"),
                &body.email,
                &[FieldError {
                    field: "email",
                    message: "Verify your email address first: open the link we emailed you.",
                }],
            ),
        ),
        Ok(LoginOutcome::Throttled) => throttled(),
        Err(error) => service_failed(error),
    }
}

// ---- Reset a password

#[derive(Deserialize)]
pub struct ResetRequestBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    email: String,
}

pub async fn reset_request_form<S: AccountStore, M: EmailSender>(
    State(_state): State<PortalState<S, M>>,
    headers: HeaderMap,
) -> Response {
    let (secret, cookie) = match presession_for_form(&headers) {
        Ok(found) => found,
        Err(error) => return service_failed(error),
    };
    let body = page(
        "Reset your password",
        Navigation::Visitor,
        markup!(
            "<h1>Reset your password</h1><p>Enter the address you signed up with. We will ",
            "email it a link to choose a new password.</p>",
            form_start(
                "/password-reset",
                &secret.form_token("password-reset"),
                None
            ),
            field(&email_field("", "email"), &[]),
            button("Email me a reset link", ButtonStyle::Primary),
            "</form>"
        ),
    );
    with_cookie(html(StatusCode::OK, body), cookie)
}

pub async fn reset_request<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<ResetRequestBody>,
) -> Response {
    if let Err(refusal) = check_anonymous_post(
        &headers,
        state.origin.as_str(),
        "password-reset",
        &body.csrf,
    ) {
        return form_refused(refusal);
    }
    match state
        .service
        .request_password_reset(&body.email, client)
        .await
    {
        Ok(ResetRequestOutcome::Accepted) => ok_page(
            "Check your email",
            Navigation::Visitor,
            markup!(
                "<h1>Check your email</h1><p>If an account uses that address, we have sent it a ",
                "link to choose a new password. The link works once, for 15 minutes.</p>"
            ),
        ),
        Ok(ResetRequestOutcome::Throttled) => throttled(),
        Err(error) => service_failed(error),
    }
}

#[derive(Deserialize)]
pub struct ResetConfirmBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    confirmation: String,
}

fn reset_confirm_page(csrf: &str, token: &str, errors: &[FieldError]) -> Markup {
    page(
        "Choose a new password",
        Navigation::Visitor,
        markup!(
            "<h1>Choose a new password</h1><p>Every device signed in to your account will be ",
            "signed out.</p>",
            error_summary(errors),
            form_start("/password-reset/confirm", csrf, None),
            hidden("token", token),
            field(
                &password_field(
                    "password",
                    "New password",
                    "new-password",
                    Some(PASSWORD_HINT)
                ),
                errors
            ),
            field(
                &password_field("confirmation", "New password again", "new-password", None),
                errors
            ),
            button("Save new password", ButtonStyle::Primary),
            "</form>"
        ),
    )
}

pub async fn reset_confirm_form<S: AccountStore, M: EmailSender>(
    State(_state): State<PortalState<S, M>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response {
    let (secret, cookie) = match presession_for_form(&headers) {
        Ok(found) => found,
        Err(error) => return service_failed(error),
    };
    with_cookie(
        html(
            StatusCode::OK,
            reset_confirm_page(
                &secret.form_token("password-reset-confirm"),
                &query.token,
                &[],
            ),
        ),
        cookie,
    )
}

pub async fn reset_confirm<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<ResetConfirmBody>,
) -> Response {
    let secret = match check_anonymous_post(
        &headers,
        state.origin.as_str(),
        "password-reset-confirm",
        &body.csrf,
    ) {
        Ok(secret) => secret,
        Err(refusal) => return form_refused(refusal),
    };
    let error = match state
        .service
        .confirm_password_reset(&body.token, &body.password, &body.confirmation, client)
        .await
    {
        Ok(ResetConfirmOutcome::Reset) => {
            return ok_page(
                "Password changed",
                Navigation::Visitor,
                markup!(
                    "<h1>Your password is changed</h1><p>Every device was signed out. ",
                    "<a class=\"button primary\" href=\"/login\">Log in</a></p>"
                ),
            );
        }
        Ok(ResetConfirmOutcome::InvalidLink) => {
            return html(
                StatusCode::BAD_REQUEST,
                page(
                    "Link expired",
                    Navigation::Visitor,
                    markup!(
                        "<h1>This reset link has expired or was already used</h1><p>Links work ",
                        "once, for 15 minutes. <a href=\"/password-reset\">Ask for a new ",
                        "link</a>.</p>"
                    ),
                ),
            );
        }
        Ok(ResetConfirmOutcome::Throttled) => return throttled(),
        Ok(ResetConfirmOutcome::Mismatch) => FieldError {
            field: "confirmation",
            message: "The two passwords are different.",
        },
        Ok(ResetConfirmOutcome::Refused(refusal)) => FieldError {
            field: "password",
            message: password_refusal_message(refusal),
        },
        Ok(ResetConfirmOutcome::ProviderRejectedPassword) => FieldError {
            field: "password",
            message: "Our sign-in service did not accept that password. Choose a longer or \
                      less common one.",
        },
        Err(error) => return service_failed(error),
    };
    html(
        StatusCode::UNPROCESSABLE_ENTITY,
        reset_confirm_page(
            &secret.form_token("password-reset-confirm"),
            &body.token,
            &[error],
        ),
    )
}

// ---- Confirm or cancel an email change

pub async fn email_change_form<S: AccountStore, M: EmailSender>(
    State(_state): State<PortalState<S, M>>,
    headers: HeaderMap,
    Query(query): Query<TokenQuery>,
) -> Response {
    let (secret, cookie) = match presession_for_form(&headers) {
        Ok(found) => found,
        Err(error) => return service_failed(error),
    };
    let body = page(
        "Email change",
        Navigation::Visitor,
        markup!(
            "<h1>Change of email address</h1><p>Confirm the change, or cancel it if you did ",
            "not ask for it.</p>",
            form_start(
                "/email-change/confirm",
                &secret.form_token("email-change"),
                None
            ),
            hidden("token", &query.token),
            "<button type=\"submit\" class=\"button primary\" name=\"decision\" ",
            "value=\"confirm\">Confirm the change</button> ",
            "<button type=\"submit\" class=\"button danger\" name=\"decision\" ",
            "value=\"cancel\">Cancel the change</button></form>"
        ),
    );
    with_cookie(html(StatusCode::OK, body), cookie)
}

#[derive(Deserialize)]
pub struct EmailChangeBody {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    decision: String,
}

pub async fn email_change<S: AccountStore, M: EmailSender>(
    State(state): State<PortalState<S, M>>,
    ClientInfo(client): ClientInfo,
    headers: HeaderMap,
    axum::Form(body): axum::Form<EmailChangeBody>,
) -> Response {
    if let Err(refusal) =
        check_anonymous_post(&headers, state.origin.as_str(), "email-change", &body.csrf)
    {
        return form_refused(refusal);
    }
    let decision = match body.decision.as_str() {
        "confirm" => EmailChangeDecision::Confirm,
        "cancel" => EmailChangeDecision::Cancel,
        _ => return form_refused(FormRefusal::BadToken),
    };
    let (title, message) = match state
        .service
        .confirm_email_change(&body.token, decision, client)
        .await
    {
        Ok(EmailConfirmResult::AwaitingOtherAddress) => (
            "Confirmed",
            "Thank you. The change completes when the other address confirms too.",
        ),
        Ok(EmailConfirmResult::Changed) => (
            "Email changed",
            "Your email address is changed. Every device was signed out; log in with the new \
             address.",
        ),
        Ok(EmailConfirmResult::Cancelled) => (
            "Change cancelled",
            "The change is cancelled; your address stays as it was. If you did not ask for it, \
             change your password now.",
        ),
        Ok(EmailConfirmResult::AddressTaken) => (
            "Change not possible",
            "That address now belongs to another account, so the change was not made.",
        ),
        Ok(EmailConfirmResult::InvalidLink) => {
            ("Link expired", "This link has expired or was already used.")
        }
        Ok(EmailConfirmResult::Throttled) => return throttled(),
        Err(error) => return service_failed(error),
    };
    ok_page(
        title,
        Navigation::Visitor,
        markup!(
            "<h1>",
            text(title),
            "</h1><p>",
            text(message),
            "</p><p><a href=\"/login\">Log in</a></p>"
        ),
    )
}
