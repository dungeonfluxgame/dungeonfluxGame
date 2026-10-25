use super::*;
use crate::wire::{
    Accepted, ForgotPasswordRequest, Refusal, ResetOutcome, ResetPasswordRequest, SignInRequest,
    SignUpRequest, VerifyEmailRequest, VerifyOutcome,
};

fn token_route(token: &str) -> Route {
    Route::VerifyEmail {
        token: Some(token.to_owned()),
    }
}

fn filled_sign_up() -> Model {
    let mut model = booted(Route::SignUp, anonymous());
    issue(&mut model, OperationSlot::Primary, "op-signup");
    type_into(&mut model, Field::Email, "  ana@example.test ");
    type_into(&mut model, Field::Password, "a long enough passphrase");
    tick(&mut model, Field::IsAdult, true);
    tick(&mut model, Field::AcceptsTerms, true);
    model
}

#[test]
fn an_anonymous_visitor_sees_the_landing_page_and_a_member_goes_to_the_dashboard() {
    let anonymous_model = booted(Route::Landing, anonymous());
    let member_model = booted(Route::Landing, signed_in(true));

    assert_eq!(anonymous_model.screen, Screen::Landing);
    assert!(matches!(member_model.screen, Screen::Dashboard(_)));
}

#[test]
fn a_private_page_sends_an_anonymous_visitor_to_sign_in_and_back_afterwards() {
    let mut model = booted(Route::Settings, anonymous());
    assert!(matches!(model.screen, Screen::SignIn(_)));
    assert_eq!(model.notice, Some(Notice::SignInToContinue));
    type_into(&mut model, Field::Email, "ana@example.test");
    type_into(&mut model, Field::Password, "secret");

    let effects = submit(&mut model, Form::SignIn);
    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::SignIn(SignInRequest {
            email: "ana@example.test".to_owned(),
            password: "secret".to_owned(),
        }))
    );
    let effects = deliver(&mut model, Reply::SignedIn(Ok(signed_in(true))));

    assert!(matches!(model.screen, Screen::Settings(_)));
    assert!(has_effect(
        &effects,
        &Effect::PushUrl("/app/settings".to_owned())
    ));
    assert_eq!(model.notice, None);
}

#[test]
fn a_signed_in_member_never_sees_the_sign_in_or_sign_up_forms() {
    for route in [Route::SignIn, Route::SignUp, Route::ForgotPassword] {
        let model = booted(route, signed_in(true));
        assert!(matches!(model.screen, Screen::Dashboard(_)));
    }
}

#[test]
fn sign_up_refuses_a_missing_box_before_asking_the_server() {
    let mut model = filled_sign_up();
    tick(&mut model, Field::IsAdult, false);

    let effects = submit(&mut model, Form::SignUp);

    assert!(calls(&effects).is_empty());
    let Screen::SignUp(form) = &model.screen else {
        panic!("still the sign-up form");
    };
    assert_eq!(
        form.submit,
        Submit::Refused(ApiError::refused(Refusal::AdultRequired))
    );
    assert!(has_effect(
        &effects,
        &Effect::Focus(FocusTarget::ErrorSummary)
    ));
}

#[test]
fn sign_up_sends_the_trimmed_address_once_with_the_issued_operation_id() {
    let mut model = filled_sign_up();

    let effects = submit(&mut model, Form::SignUp);
    let again = submit(&mut model, Form::SignUp);

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::SignUp(SignUpRequest {
            email: "ana@example.test".to_owned(),
            password: "a long enough passphrase".to_owned(),
            is_adult: true,
            accepts_terms: true,
            terms_version: crate::wire::CURRENT_TERMS_VERSION.to_owned(),
            operation: "op-signup".to_owned(),
        }))
    );
    assert!(calls(&again).is_empty(), "a second press sends nothing");
}

#[test]
fn switching_to_the_development_admin_runs_password_sign_in_and_opens_the_console() {
    let mut session = anonymous();
    session.dev_sign_in = Some(vec![
        crate::wire::DevSignIn {
            label: "Customer".to_owned(),
            email: "dev@example.test".to_owned(),
            password: "development password".to_owned(),
        },
        crate::wire::DevSignIn {
            label: "Admin".to_owned(),
            email: "dev-admin@example.test".to_owned(),
            password: "development password".to_owned(),
        },
    ]);
    let mut model = booted(Route::SignIn, session);

    model.update(Msg::Intent(Intent::UseDevSignIn(1)));
    let effects = submit(&mut model, Form::SignIn);

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::SignIn(SignInRequest {
            email: "dev-admin@example.test".to_owned(),
            password: "development password".to_owned(),
        }))
    );
    deliver(&mut model, Reply::SignedIn(Ok(operator(true))));

    assert_eq!(model.route, Route::Admin(crate::route::AdminRoute::Summary));
}

#[test]
fn the_prefilled_development_customer_runs_password_sign_in_and_opens_the_dashboard() {
    let mut session = anonymous();
    session.dev_sign_in = Some(vec![crate::wire::DevSignIn {
        label: "Customer".to_owned(),
        email: "dev@example.test".to_owned(),
        password: "development password".to_owned(),
    }]);
    let mut model = booted(Route::SignIn, session);

    let effects = submit(&mut model, Form::SignIn);

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::SignIn(SignInRequest {
            email: "dev@example.test".to_owned(),
            password: "development password".to_owned(),
        }))
    );
    deliver(&mut model, Reply::SignedIn(Ok(signed_in(true))));

    assert_eq!(model.route, Route::Dashboard);
}

#[test]
fn sign_up_waits_for_its_operation_id_instead_of_sending_without_one() {
    let mut model = booted(Route::SignUp, anonymous());
    type_into(&mut model, Field::Email, "ana@example.test");
    type_into(&mut model, Field::Password, "a long enough passphrase");
    tick(&mut model, Field::IsAdult, true);
    tick(&mut model, Field::AcceptsTerms, true);

    assert!(calls(&submit(&mut model, Form::SignUp)).is_empty());
}

#[test]
fn a_created_account_goes_to_check_your_email_naming_the_address() {
    let mut model = filled_sign_up();
    submit(&mut model, Form::SignUp);

    deliver(&mut model, Reply::SignedUp(Ok(Accepted { accepted: true })));

    let Screen::VerifyPending(screen) = &model.screen else {
        panic!("the check-your-email screen");
    };
    assert_eq!(screen.sent_to.as_deref(), Some("ana@example.test"));
}

#[test]
fn a_stale_sign_up_form_gets_a_new_operation_id_and_keeps_what_was_typed() {
    let mut model = filled_sign_up();
    submit(&mut model, Form::SignUp);

    let effects = deliver(
        &mut model,
        Reply::SignedUp(Err(ApiError::of(ErrorCode::StaleForm))),
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::IssueOperation {
            slot: OperationSlot::Primary,
            ..
        }
    )));
    let Screen::SignUp(form) = &model.screen else {
        panic!("still the form");
    };
    assert_eq!(form.email, "  ana@example.test ");
    assert_eq!(form.operation, Remote::Loading);
}

#[test]
fn a_wrong_password_clears_the_password_and_keeps_the_email() {
    let mut model = booted(Route::SignIn, anonymous());
    type_into(&mut model, Field::Email, "ana@example.test");
    type_into(&mut model, Field::Password, "wrong");
    submit(&mut model, Form::SignIn);

    let effects = deliver(
        &mut model,
        Reply::SignedIn(Err(ApiError::refused(Refusal::WrongPassword))),
    );

    let Screen::SignIn(form) = &model.screen else {
        panic!("still the sign-in form");
    };
    assert_eq!(form.email, "ana@example.test");
    assert_eq!(form.password, "");
    assert_eq!(
        form.submit,
        Submit::Refused(ApiError::refused(Refusal::WrongPassword))
    );
    assert!(has_effect(
        &effects,
        &Effect::Focus(FocusTarget::ErrorSummary)
    ));
}

#[test]
fn a_verification_link_does_nothing_until_the_button_is_pressed() {
    let mut model = booted(token_route("0a1b"), anonymous());

    assert!(matches!(model.screen, Screen::VerifyConfirm(_)));
    // No password yet: nothing is sent.
    assert!(calls(&submit(&mut model, Form::VerifyEmail)).is_empty());
    type_into(&mut model, Field::Password, "a long enough passphrase");
    type_into(&mut model, Field::Confirmation, "a long enough passphrase");
    let effects = submit(&mut model, Form::VerifyEmail);

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::VerifyEmail(VerifyEmailRequest {
            token: "0a1b".to_owned(),
            password: "a long enough passphrase".to_owned(),
            confirmation: "a long enough passphrase".to_owned(),
        }))
    );
}

#[test]
fn verifying_with_different_passwords_is_refused_before_any_call() {
    let mut model = booted(token_route("0a1b"), anonymous());
    type_into(&mut model, Field::Password, "a long enough passphrase");
    type_into(
        &mut model,
        Field::Confirmation,
        "a different passphrase here",
    );
    assert!(calls(&submit(&mut model, Form::VerifyEmail)).is_empty());
    let Screen::VerifyConfirm(screen) = &model.screen else {
        panic!("the confirmation");
    };
    assert_eq!(
        screen.submit,
        Submit::Refused(ApiError::refused(Refusal::PasswordMismatch))
    );
}

fn filled_verify(model: &mut Model) {
    type_into(model, Field::Password, "a long enough passphrase");
    type_into(model, Field::Confirmation, "a long enough passphrase");
}

#[test]
fn verifying_while_signed_in_refreshes_the_session_so_the_trial_shows() {
    let mut model = booted(token_route("0a1b"), signed_in(false));
    filled_verify(&mut model);
    submit(&mut model, Form::VerifyEmail);

    let effects = deliver(&mut model, Reply::Verified(Ok(VerifyOutcome::Verified)));

    assert_eq!(first_call(&effects), Some(&ApiCall::Session));
    let Screen::VerifyConfirm(screen) = &model.screen else {
        panic!("the confirmation");
    };
    assert_eq!(screen.outcome, Some(VerifyOutcome::Verified));
}

#[test]
fn a_verification_cannot_be_sent_twice() {
    let mut model = booted(token_route("0a1b"), anonymous());
    filled_verify(&mut model);
    submit(&mut model, Form::VerifyEmail);
    assert!(calls(&submit(&mut model, Form::VerifyEmail)).is_empty());

    deliver(&mut model, Reply::Verified(Ok(VerifyOutcome::InvalidLink)));

    assert!(calls(&submit(&mut model, Form::VerifyEmail)).is_empty());
}

#[test]
fn resending_the_verification_link_sends_one_request_and_then_says_so() {
    let mut model = booted(Route::VerifyEmail { token: None }, signed_in(false));

    let effects = click(&mut model, Intent::ResendVerification);
    let again = click(&mut model, Intent::ResendVerification);
    deliver(&mut model, Reply::Resent(Ok(Accepted { accepted: true })));

    assert_eq!(first_call(&effects), Some(&ApiCall::ResendVerification));
    assert!(calls(&again).is_empty());
    let Screen::VerifyPending(screen) = &model.screen else {
        panic!("check your email");
    };
    assert!(screen.is_resent);
}

#[test]
fn forgot_password_says_the_same_thing_for_every_address() {
    let mut model = booted(Route::ForgotPassword, anonymous());
    type_into(&mut model, Field::Email, "nobody@example.test");

    let effects = submit(&mut model, Form::Forgot);
    deliver(
        &mut model,
        Reply::ForgotSent(Ok(Accepted { accepted: true })),
    );

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::ForgotPassword(ForgotPasswordRequest {
            email: "nobody@example.test".to_owned()
        }))
    );
    let Screen::Forgot(form) = &model.screen else {
        panic!("forgot form");
    };
    assert!(form.is_sent);
}

#[test]
fn a_reset_without_a_matching_confirmation_is_refused_locally() {
    let mut model = booted(
        Route::ResetPassword {
            token: Some("0f".to_owned()),
        },
        anonymous(),
    );
    type_into(&mut model, Field::Password, "one long passphrase");
    type_into(&mut model, Field::Confirmation, "another long passphrase");

    let effects = submit(&mut model, Form::Reset);

    assert!(calls(&effects).is_empty());
    assert_eq!(
        crate::copy::refused_form_error(&model),
        Some(ApiError::refused(Refusal::PasswordMismatch))
    );
}

#[test]
fn a_reset_sends_the_token_only_in_the_request_and_ends_every_session() {
    let mut model = booted(
        Route::ResetPassword {
            token: Some("0f".to_owned()),
        },
        anonymous(),
    );
    type_into(&mut model, Field::Password, "one long passphrase");
    type_into(&mut model, Field::Confirmation, "one long passphrase");

    let effects = submit(&mut model, Form::Reset);
    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::ResetPassword(ResetPasswordRequest {
            token: "0f".to_owned(),
            password: "one long passphrase".to_owned(),
            confirmation: "one long passphrase".to_owned(),
        }))
    );
    let effects = deliver(&mut model, Reply::Reset(Ok(ResetOutcome::Reset)));

    assert_eq!(model.notice, Some(Notice::PasswordChanged));
    assert_eq!(first_call(&effects), Some(&ApiCall::Session));
    assert_eq!(model.route, Route::SignIn);
}

#[test]
fn an_expired_reset_link_offers_a_new_one_and_sends_nothing() {
    let mut model = booted(Route::ResetPassword { token: None }, anonymous());
    let Screen::Reset(form) = &model.screen else {
        panic!("reset form");
    };
    assert_eq!(form.token, None);
    assert!(calls(&submit(&mut model, Form::Reset)).is_empty());
}

#[test]
fn signing_out_ends_on_the_signed_out_page_with_a_fresh_anonymous_session() {
    let mut model = booted(Route::Dashboard, signed_in(true));

    let effects = click(&mut model, Intent::SignOut);
    assert_eq!(first_call(&effects), Some(&ApiCall::SignOut));
    deliver(&mut model, Reply::SignedOut(Ok(anonymous())));

    assert_eq!(model.screen, Screen::SignedOut);
    assert!(model.account().is_none());
    assert_eq!(model.csrf(), "csrf-anonymous");
}

#[test]
fn a_session_the_server_no_longer_accepts_returns_to_sign_in_and_remembers_the_page() {
    let mut model = booted(Route::Statements, signed_in(true));

    let effects = deliver(
        &mut model,
        Reply::Statements {
            is_continuation: false,
            result: Err(ApiError::of(ErrorCode::SignedOut)),
        },
    );

    assert_eq!(effects.first(), Some(&Effect::CancelPending));
    assert_eq!(model.notice, Some(Notice::SessionEnded));
    assert_eq!(model.route, Route::SignIn);
    // The session is read again, and the sign-in form comes up with the anonymous token.
    deliver(&mut model, Reply::Session(Ok(anonymous())));
    assert!(matches!(model.screen, Screen::SignIn(_)));
    deliver(&mut model, Reply::SignedIn(Ok(signed_in(true))));
    assert!(matches!(model.screen, Screen::Statements(_)));
}

#[test]
fn a_sign_out_that_could_not_be_confirmed_keeps_the_screen_and_says_so() {
    let mut model = booted(Route::Dashboard, signed_in(true));
    click(&mut model, Intent::SignOut);

    let effects = deliver(
        &mut model,
        Reply::SignedOut(Err(ApiError::of(ErrorCode::Unavailable))),
    );

    assert!(calls(&effects).is_empty());
    assert!(matches!(model.screen, Screen::Dashboard(_)));
    assert_eq!(model.notice, Some(Notice::SignOutFailed));
    assert!(model.account().is_some());
}
