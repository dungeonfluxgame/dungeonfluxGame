//! Sign up, verify, sign in, forgot and reset password (plan 9.3; decisions 3 and 28).
//!
//! The app checks only what it can see (an empty field, an unticked box) so the person hears
//! about it at once; the server checks everything again and is the only authority.

use super::{
    Effect, Field, FocusTarget, Form, History, Model, OperationSlot, Remote, Screen, Submit,
};
use crate::{
    api::{ApiCall, Reply},
    route::Route,
    wire::{
        ApiError, CURRENT_TERMS_VERSION, ForgotPasswordRequest, Refusal, ResetOutcome,
        ResetPasswordRequest, SignInRequest, SignUpRequest, VerifyEmailRequest, VerifyOutcome,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignUpForm {
    pub email: String,
    pub password: String,
    pub is_adult: bool,
    pub accepts_terms: bool,
    pub operation: Remote<String>,
    pub submit: Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignInForm {
    pub email: String,
    pub password: String,
    pub dev_sign_in: Vec<crate::wire::DevSignIn>,
    pub selected_dev_sign_in: Option<usize>,
    pub submit: Submit,
}

/// "Check your email": the address the link went to, when this visit sent it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyPending {
    pub sent_to: Option<String>,
    pub resend: Submit,
    pub is_resent: bool,
}

/// The emailed link opened: the mailbox owner chooses the account password, then verifies
/// (link scanners never submit the form).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyConfirm {
    pub token: String,
    pub password: String,
    pub confirmation: String,
    pub submit: Submit,
    pub outcome: Option<VerifyOutcome>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForgotForm {
    pub email: String,
    pub submit: Submit,
    pub is_sent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResetForm {
    pub token: Option<String>,
    pub password: String,
    pub confirmation: String,
    pub submit: Submit,
}

impl Model {
    pub(super) fn enter_auth(&mut self, route: Route) -> Vec<Effect> {
        match route {
            Route::SignUp => {
                self.screen = Screen::SignUp(SignUpForm {
                    email: String::new(),
                    password: String::new(),
                    is_adult: false,
                    accepts_terms: false,
                    operation: Remote::Loading,
                    submit: Submit::Idle,
                });
                vec![self.issue_operation(OperationSlot::Primary)]
            }
            Route::SignIn => {
                let dev_sign_in = self
                    .session
                    .ready()
                    .and_then(|session| session.dev_sign_in.clone())
                    .unwrap_or_default();
                let first = dev_sign_in.first();
                self.screen = Screen::SignIn(SignInForm {
                    email: first.map_or_else(String::new, |account| account.email.clone()),
                    password: first.map_or_else(String::new, |account| account.password.clone()),
                    selected_dev_sign_in: (!dev_sign_in.is_empty()).then_some(0),
                    dev_sign_in,
                    submit: Submit::Idle,
                });
                Vec::new()
            }
            Route::VerifyEmail { token: Some(token) } => {
                self.screen = Screen::VerifyConfirm(VerifyConfirm {
                    token,
                    password: String::new(),
                    confirmation: String::new(),
                    submit: Submit::Idle,
                    outcome: None,
                });
                Vec::new()
            }
            Route::VerifyEmail { token: None } => {
                let sent_to = self.sent_to.take();
                self.screen = Screen::VerifyPending(VerifyPending {
                    sent_to,
                    resend: Submit::Idle,
                    is_resent: false,
                });
                Vec::new()
            }
            Route::ForgotPassword => {
                self.screen = Screen::Forgot(ForgotForm {
                    email: String::new(),
                    submit: Submit::Idle,
                    is_sent: false,
                });
                Vec::new()
            }
            Route::ResetPassword { token } => {
                self.screen = Screen::Reset(ResetForm {
                    token,
                    password: String::new(),
                    confirmation: String::new(),
                    submit: Submit::Idle,
                });
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Returns whether the field belonged to an auth form.
    pub(super) fn edit_auth(&mut self, field: Field, value: &str) -> bool {
        let slot = match (&mut self.screen, field) {
            (Screen::SignUp(form), Field::Email) => &mut form.email,
            (Screen::SignUp(form), Field::Password) => &mut form.password,
            (Screen::SignIn(form), Field::Email) => &mut form.email,
            (Screen::SignIn(form), Field::Password) => &mut form.password,
            (Screen::Forgot(form), Field::Email) => &mut form.email,
            (Screen::VerifyConfirm(form), Field::Password) => &mut form.password,
            (Screen::VerifyConfirm(form), Field::Confirmation) => &mut form.confirmation,
            (Screen::Reset(form), Field::Password) => &mut form.password,
            (Screen::Reset(form), Field::Confirmation) => &mut form.confirmation,
            _ => return false,
        };
        value.clone_into(slot);
        true
    }

    pub(super) fn use_dev_sign_in(&mut self, index: usize) {
        let Screen::SignIn(form) = &mut self.screen else {
            return;
        };
        let Some(account) = form.dev_sign_in.get(index) else {
            return;
        };
        form.email.clone_from(&account.email);
        form.password.clone_from(&account.password);
        form.selected_dev_sign_in = Some(index);
        form.submit = Submit::Idle;
    }

    pub(super) fn submit_auth(&mut self, form: Form) -> Vec<Effect> {
        let generation = self.generation;
        let call = match (&mut self.screen, form) {
            (Screen::SignUp(form), Form::SignUp) => {
                if form.submit.is_sending() {
                    return Vec::new();
                }
                let missing = if form.email.trim().is_empty() {
                    Some(Refusal::EmailEmpty)
                } else if form.password.is_empty() {
                    Some(Refusal::PasswordTooShort)
                } else if !form.is_adult {
                    Some(Refusal::AdultRequired)
                } else if !form.accepts_terms {
                    Some(Refusal::TermsRequired)
                } else {
                    None
                };
                if let Some(refusal) = missing {
                    form.submit = Submit::Refused(ApiError::refused(refusal));
                    return Vec::new();
                }
                let Some(operation) = form.operation.ready().cloned() else {
                    return Vec::new();
                };
                form.submit = Submit::Sending;
                ApiCall::SignUp(SignUpRequest {
                    email: form.email.trim().to_owned(),
                    password: form.password.clone(),
                    is_adult: form.is_adult,
                    accepts_terms: form.accepts_terms,
                    terms_version: CURRENT_TERMS_VERSION.to_owned(),
                    operation,
                })
            }
            (Screen::SignIn(form), Form::SignIn) => {
                if form.submit.is_sending() {
                    return Vec::new();
                }
                if form.email.trim().is_empty() {
                    form.submit = Submit::Refused(ApiError::refused(Refusal::EmailEmpty));
                    return Vec::new();
                }
                form.submit = Submit::Sending;
                ApiCall::SignIn(SignInRequest {
                    email: form.email.trim().to_owned(),
                    password: form.password.clone(),
                })
            }
            (Screen::VerifyConfirm(screen), Form::VerifyEmail) => {
                if screen.submit.is_sending() || screen.outcome.is_some() {
                    return Vec::new();
                }
                if screen.password.is_empty() {
                    screen.submit = Submit::Refused(ApiError::refused(Refusal::PasswordTooShort));
                    return Vec::new();
                }
                if screen.password != screen.confirmation {
                    screen.submit = Submit::Refused(ApiError::refused(Refusal::PasswordMismatch));
                    return Vec::new();
                }
                screen.submit = Submit::Sending;
                ApiCall::VerifyEmail(VerifyEmailRequest {
                    token: screen.token.clone(),
                    password: screen.password.clone(),
                    confirmation: screen.confirmation.clone(),
                })
            }
            (Screen::Forgot(form), Form::Forgot) => {
                if form.submit.is_sending() {
                    return Vec::new();
                }
                if form.email.trim().is_empty() {
                    form.submit = Submit::Refused(ApiError::refused(Refusal::EmailEmpty));
                    return Vec::new();
                }
                form.submit = Submit::Sending;
                ApiCall::ForgotPassword(ForgotPasswordRequest {
                    email: form.email.trim().to_owned(),
                })
            }
            (Screen::Reset(form), Form::Reset) => {
                let Some(token) = form.token.clone() else {
                    return Vec::new();
                };
                if form.submit.is_sending() {
                    return Vec::new();
                }
                if form.password != form.confirmation {
                    form.submit = Submit::Refused(ApiError::refused(Refusal::PasswordMismatch));
                    return Vec::new();
                }
                form.submit = Submit::Sending;
                ApiCall::ResetPassword(ResetPasswordRequest {
                    token,
                    password: form.password.clone(),
                    confirmation: form.confirmation.clone(),
                })
            }
            _ => return Vec::new(),
        };
        vec![Effect::Api { generation, call }]
    }

    pub(super) fn resend_verification(&mut self) -> Vec<Effect> {
        let resend = match &mut self.screen {
            Screen::VerifyPending(screen) => &mut screen.resend,
            Screen::Dashboard(screen) => &mut screen.resend,
            _ => return Vec::new(),
        };
        if resend.is_sending() {
            return Vec::new();
        }
        *resend = Submit::Sending;
        vec![self.api(ApiCall::ResendVerification)]
    }

    pub(super) fn auth_reply(&mut self, reply: Reply) -> Vec<Effect> {
        match (reply, &mut self.screen) {
            (Reply::SignedUp(Ok(_)), Screen::SignUp(form)) => {
                self.sent_to = Some(form.email.trim().to_owned());
                self.navigate(Route::VerifyEmail { token: None }, History::Push)
            }
            (Reply::SignedUp(Err(error)), Screen::SignUp(form)) => {
                form.submit = Submit::Refused(error);
                if error.code == crate::wire::ErrorCode::StaleForm {
                    form.operation = Remote::Loading;
                    return vec![
                        self.issue_operation(OperationSlot::Primary),
                        Effect::Focus(FocusTarget::ErrorSummary),
                    ];
                }
                vec![Effect::Focus(FocusTarget::ErrorSummary)]
            }
            (Reply::SignedIn(Ok(session)), Screen::SignIn(_)) => {
                let default_target = if session
                    .account
                    .as_ref()
                    .is_some_and(|account| account.operator.is_some())
                {
                    Route::Admin(crate::route::AdminRoute::Summary)
                } else {
                    Route::Dashboard
                };
                self.session = Remote::Ready(session);
                let target = self.after_sign_in.take().unwrap_or(default_target);
                self.notice = None;
                self.navigate(target, History::Push)
            }
            (Reply::SignedIn(Err(error)), Screen::SignIn(form)) => {
                form.password.clear();
                form.submit = Submit::Refused(error);
                vec![Effect::Focus(FocusTarget::ErrorSummary)]
            }
            (Reply::Verified(result), Screen::VerifyConfirm(screen)) => match result {
                Ok(outcome) => {
                    screen.submit = Submit::Idle;
                    screen.outcome = Some(outcome);
                    self.session = super::Remote::Loading;
                    vec![
                        Effect::Focus(FocusTarget::Heading),
                        self.api(ApiCall::Session),
                    ]
                }
                Err(error) => {
                    screen.submit = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::Resent(result), Screen::VerifyPending(screen)) => resent(
                &mut screen.resend,
                &mut screen.is_resent,
                result.map(|_| ()),
            ),
            (Reply::Resent(result), Screen::Dashboard(screen)) => resent(
                &mut screen.resend,
                &mut screen.is_resent,
                result.map(|_| ()),
            ),
            (Reply::ForgotSent(result), Screen::Forgot(form)) => match result {
                Ok(_) => {
                    form.submit = Submit::Idle;
                    form.is_sent = true;
                    vec![Effect::Focus(FocusTarget::Heading)]
                }
                Err(error) => {
                    form.submit = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            (Reply::Reset(result), Screen::Reset(form)) => match result {
                Ok(ResetOutcome::Reset) => {
                    self.notice = Some(super::Notice::PasswordChanged);
                    // Every session was revoked; the session must be read again before sign-in.
                    self.session = Remote::Loading;
                    self.route = Route::SignIn;
                    self.screen = Screen::Booting;
                    vec![
                        Effect::PushUrl(Route::SignIn.path()),
                        self.api(ApiCall::Session),
                    ]
                }
                Ok(ResetOutcome::InvalidLink) => {
                    form.token = None;
                    form.submit = Submit::Idle;
                    vec![Effect::Focus(FocusTarget::Heading)]
                }
                Err(error) => {
                    form.submit = Submit::Refused(error);
                    vec![Effect::Focus(FocusTarget::ErrorSummary)]
                }
            },
            _ => Vec::new(),
        }
    }
}

fn resent(resend: &mut Submit, is_resent: &mut bool, result: Result<(), ApiError>) -> Vec<Effect> {
    match result {
        Ok(()) => {
            *resend = Submit::Idle;
            *is_resent = true;
            Vec::new()
        }
        Err(error) => {
            *resend = Submit::Refused(error);
            vec![Effect::Focus(FocusTarget::ErrorSummary)]
        }
    }
}
