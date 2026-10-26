//! Sign up, sign in, sign out, email verification and password reset, all through
//! `CustomerService`. Verification and reset tokens arrive in the JSON body only, never in a path
//! or query (they come from the URL fragment of the emailed link). Answers that could reveal
//! whether an address has an account are the same for both.

use axum::extract::State;
use df_account_app::wire::{
    Accepted, ApiError, CSRF_FORM, ErrorCode, ForgotPasswordRequest, Refusal, ResetOutcome,
    ResetPasswordRequest, SessionView, SignInRequest, SignUpRequest, VerifyEmailRequest,
    VerifyOutcome,
};
use df_customer::{
    AccountStore, EmailRefusal, EmailSender, LoginOutcome, PasswordRefusal, ResendOutcome,
    ResetConfirmOutcome, ResetRequestOutcome, SignupForm, SignupOutcome, SignupRefusal,
    VerifyWithPasswordOutcome,
};

use super::{
    guard::{
        ApiBody, ApiFailure, ApiResult, Nothing, Reply, SameSite, SessionPost, VisitorPost,
        clear_secret_cookie, presession_for_visitor, reply, secret_cookie,
    },
    ids,
    ports::Ports,
    session::signed_in_view,
    state::AppApiState,
};
use crate::security::{ClientInfo, PRESESSION_COOKIE, SESSION_COOKIE};

const fn email_refusal(refusal: EmailRefusal) -> Refusal {
    match refusal {
        EmailRefusal::Empty => Refusal::EmailEmpty,
        EmailRefusal::TooLong => Refusal::EmailTooLong,
        EmailRefusal::Malformed => Refusal::EmailMalformed,
    }
}

const fn password_refusal(refusal: PasswordRefusal) -> Refusal {
    match refusal {
        PasswordRefusal::TooShort => Refusal::PasswordTooShort,
        PasswordRefusal::TooLong => Refusal::PasswordTooLong,
        PasswordRefusal::MatchesEmail => Refusal::PasswordMatchesEmail,
        PasswordRefusal::Common => Refusal::PasswordCommon,
    }
}

const fn refused<T>(refusal: Refusal) -> ApiResult<T> {
    Err(ApiFailure::new(ApiError::refused(refusal)))
}

const ACCEPTED: Accepted = Accepted { accepted: true };

pub(super) async fn sign_up<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    _guard: VisitorPost,
    ClientInfo(client): ClientInfo,
    ApiBody(body): ApiBody<SignUpRequest>,
) -> ApiResult<Accepted> {
    if !body.is_adult {
        return refused(Refusal::AdultRequired);
    }
    if !body.accepts_terms {
        return refused(Refusal::TermsRequired);
    }
    let operation = ids::operation(&body.operation)?;
    let form = SignupForm {
        email: &body.email,
        password: &body.password,
        operation,
        adult_attested: body.is_adult,
        terms_version: body.terms_version,
    };
    match state.service.signup(form, client).await? {
        SignupOutcome::CheckYourEmail => reply(ACCEPTED),
        SignupOutcome::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
        SignupOutcome::Refused(SignupRefusal::Email(refusal)) => refused(email_refusal(refusal)),
        SignupOutcome::Refused(SignupRefusal::Password(refusal)) => {
            refused(password_refusal(refusal))
        }
        SignupOutcome::Refused(SignupRefusal::PasswordRejected) => refused(Refusal::PasswordCommon),
        SignupOutcome::Refused(SignupRefusal::StaleForm) => {
            Err(ApiFailure::of(ErrorCode::StaleForm))
        }
        SignupOutcome::Refused(SignupRefusal::AdultAttestationRequired) => {
            refused(Refusal::AdultRequired)
        }
        SignupOutcome::Refused(SignupRefusal::TermsVersionChanged) => {
            refused(Refusal::TermsRequired)
        }
    }
}

/// Signs in with a new session and a new CSRF token; an earlier session of the caller ends, and
/// the spent pre-session cookie is cleared so a token from before sign-in is never promoted.
pub(super) async fn sign_in<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    guard: VisitorPost,
    ClientInfo(client): ClientInfo,
    ApiBody(body): ApiBody<SignInRequest>,
) -> ApiResult<SessionView> {
    let issued = match state
        .service
        .login(&body.email, &body.password, client)
        .await?
    {
        LoginOutcome::SignedIn(issued) => issued,
        LoginOutcome::Refused => return refused(Refusal::WrongPassword),
        LoginOutcome::VerificationRequired => {
            return Err(ApiFailure::of(ErrorCode::NotVerified));
        }
        LoginOutcome::Throttled => return Err(ApiFailure::of(ErrorCode::Throttled)),
    };
    let token = issued.token.text();
    let Some(fresh) = state.service.authenticate(&token).await? else {
        return Err(ApiFailure::of(ErrorCode::Unavailable));
    };
    if let Some(previous) = guard.caller
        && previous.session != fresh.session
        && let Err(error) = state.service.logout(&previous, client).await
    {
        df_observe::record_audit_line(
            "app_api.sign_in",
            format!(
                "APP_API sign_in previous_session_kept reason={}",
                error.label()
            ),
        );
    }
    let view = signed_in_view(&state, &fresh).await?;
    Ok(Reply::new(view)
        .with_cookie(secret_cookie(
            SESSION_COOKIE,
            &token,
            issued.max_age_s,
            SameSite::Lax,
        ))
        .with_cookie(clear_secret_cookie(PRESESSION_COOKIE, SameSite::Lax)))
}

/// Ends the session and answers with the anonymous view and a fresh pre-session token.
pub(super) async fn sign_out<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ClientInfo(client): ClientInfo,
    ApiBody(_nothing): ApiBody<Nothing>,
) -> ApiResult<SessionView> {
    state.service.logout(&caller, client).await?;
    let (secret, cookie) = presession_for_visitor(None)?;
    let mut answer = Reply::new(SessionView {
        google_enabled: state.service.google_enabled(),
        csrf: secret.form_token(CSRF_FORM),
        account: None,
        dev_sign_in: None,
    })
    .with_cookie(clear_secret_cookie(SESSION_COOKIE, SameSite::Lax));
    if let Some(cookie) = cookie {
        answer = answer.with_cookie(cookie);
    }
    Ok(answer)
}

pub(super) async fn verify_email<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    _guard: VisitorPost,
    ClientInfo(client): ClientInfo,
    ApiBody(body): ApiBody<VerifyEmailRequest>,
) -> ApiResult<VerifyOutcome> {
    if body.password.is_empty() {
        return refused(Refusal::PasswordTooShort);
    }
    match state
        .service
        .verify_email_and_set_password(&body.token, &body.password, &body.confirmation, client)
        .await?
    {
        VerifyWithPasswordOutcome::Verified(issued) => {
            let token = issued.token.text();
            Ok(Reply::new(VerifyOutcome::Verified)
                .with_cookie(secret_cookie(
                    SESSION_COOKIE,
                    &token,
                    issued.max_age_s,
                    SameSite::Lax,
                ))
                .with_cookie(clear_secret_cookie(PRESESSION_COOKIE, SameSite::Lax)))
        }
        VerifyWithPasswordOutcome::AlreadyVerified => reply(VerifyOutcome::AlreadyVerified),
        VerifyWithPasswordOutcome::InvalidLink => reply(VerifyOutcome::InvalidLink),
        VerifyWithPasswordOutcome::Mismatch => refused(Refusal::PasswordMismatch),
        VerifyWithPasswordOutcome::Refused(refusal) => refused(password_refusal(refusal)),
        VerifyWithPasswordOutcome::ProviderRejectedPassword => refused(Refusal::PasswordCommon),
        VerifyWithPasswordOutcome::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
    }
}

pub(super) async fn resend_verification<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(_nothing): ApiBody<Nothing>,
) -> ApiResult<Accepted> {
    match state.service.resend_verification(&caller).await? {
        ResendOutcome::Sent | ResendOutcome::AlreadyVerified => reply(ACCEPTED),
        ResendOutcome::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
    }
}

pub(super) async fn forgot_password<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    _guard: VisitorPost,
    ClientInfo(client): ClientInfo,
    ApiBody(body): ApiBody<ForgotPasswordRequest>,
) -> ApiResult<Accepted> {
    match state
        .service
        .request_password_reset(&body.email, client)
        .await?
    {
        ResetRequestOutcome::Accepted => reply(ACCEPTED),
        ResetRequestOutcome::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
    }
}

pub(super) async fn reset_password<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    _guard: VisitorPost,
    ClientInfo(client): ClientInfo,
    ApiBody(body): ApiBody<ResetPasswordRequest>,
) -> ApiResult<ResetOutcome> {
    let outcome = state
        .service
        .confirm_password_reset(&body.token, &body.password, &body.confirmation, client)
        .await?;
    match outcome {
        ResetConfirmOutcome::Reset => reply(ResetOutcome::Reset),
        ResetConfirmOutcome::InvalidLink => reply(ResetOutcome::InvalidLink),
        ResetConfirmOutcome::Mismatch => refused(Refusal::PasswordMismatch),
        ResetConfirmOutcome::Refused(refusal) => refused(password_refusal(refusal)),
        ResetConfirmOutcome::ProviderRejectedPassword => refused(Refusal::PasswordCommon),
        ResetConfirmOutcome::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
    }
}
