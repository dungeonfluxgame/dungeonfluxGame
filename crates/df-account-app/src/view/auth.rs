//! The free-play landing page and the sign-up, verification, sign-in and reset screens.

use super::{
    ButtonKind, InputKind, TextField, button, checkbox, error_summary, failed, form, heading, link,
    page, submit_button, text_field,
};
use crate::{
    node::{Node, el},
    route::Route,
    state::{
        Field, ForgotForm, Form, Intent, Model, Remote, ResetForm, SignInForm, SignUpForm,
        VerifyConfirm, VerifyPending,
    },
    wire::VerifyOutcome,
};

const PASSWORD_HINT: &str =
    "At least 12 characters. A short sentence you will remember works well.";

pub(super) fn landing() -> Node {
    el("div")
        .class("landing")
        .child(
            el("section")
                .class("hero")
                .attr("aria-labelledby", "landing-title")
                .child(
                    el("img")
                        .class("hero-art")
                        .attr("src", "/app/assets/landing.webp")
                        .attr("alt", "")
                        .attr("width", "1600")
                        .attr("height", "900"),
                )
                .child(el("div").class("hero-scrim"))
                .child(
                    el("div")
                        .class("hero-inner")
                        .child(el("p").class("subtitle-who").text("Your Dungeon Master"))
                        .child(
                            heading("Get $10 of free play")
                                .attr("id", "landing-title")
                                .class("subtitle-line"),
                        )
                        .child(el("span").class("subtitle-rule").attr("aria-hidden", "true"))
                        .child(el("p").class("hero-lede").text(
                            "Create a free account and verify your email, and we add $10 of play \
                             credit for your table. No card needed.",
                        ))
                        .child(
                            el("ul")
                                .class("choices-cta")
                                .child(el("li").child(link(
                                    "Create a free account",
                                    Route::SignUp,
                                    "choice-cta is-lit",
                                )))
                                .child(el("li").child(link(
                                    "I already have an account",
                                    Route::SignIn,
                                    "choice-cta",
                                ))),
                        )
                        .child(
                            el("p").class("hero-fine").child(link(
                                "Invited to a table? Enter a room code",
                                Route::Join { token: None },
                                "hero-link",
                            )),
                        ),
                ),
        )
        .child(
            el("section")
                .class("page trial-steps")
                .attr("aria-labelledby", "trial-steps-title")
                .child(el("h2").attr("id", "trial-steps-title").text("How the free play works"))
                .child(
                    el("ol")
                        .class("steps")
                        .child(step(
                            "Create your account",
                            "An email address and a password. Only the host of a table needs one to pay; invited players never do.",
                        ))
                        .child(step(
                            "Verify your email",
                            "Open the link we send. That is when the $10 arrives, once per person.",
                        ))
                        .child(step(
                            "Set a pace and play",
                            "Choose how much the table may spend in an hour. Free credit is spent first, and an empty balance never deletes your story.",
                        )),
                )
                .child(el("p").class("fine").text(
                    "Free play credit has no cash value and cannot be refunded or transferred.",
                )),
        )
        .into()
}

fn step(title: &str, body: &str) -> Node {
    el("li")
        .class("step")
        .child(el("h3").text(title))
        .child(el("p").text(body))
        .into()
}

pub(super) fn sign_up(form_state: &SignUpForm, model: &Model) -> Node {
    let error = form_state.submit.error();
    let body = match &form_state.operation {
        Remote::Failed(error) => failed(*error),
        Remote::Loading | Remote::Ready(_) => form(Form::SignUp, "Create your account")
            .child(error_summary(error))
            .child(text_field(
                TextField {
                    field: Field::Email,
                    label: "Email address",
                    value: &form_state.email,
                    kind: InputKind::Email,
                    autocomplete: "email",
                    hint: Some("We send a link here to verify it."),
                },
                error,
            ))
            .child(text_field(
                TextField {
                    field: Field::Password,
                    label: "Password",
                    value: &form_state.password,
                    kind: InputKind::Password,
                    autocomplete: "new-password",
                    hint: Some(PASSWORD_HINT),
                },
                error,
            ))
            .child(checkbox(
                Field::IsAdult,
                "I am 18 or older",
                form_state.is_adult,
                error,
            ))
            .child(terms_checkbox(form_state.accepts_terms, error))
            .child(submit_button(
                "Create account",
                "Creating your account",
                form_state.submit.is_sending() || form_state.operation.ready().is_none(),
            ))
            .into(),
    };
    let mut section = page(
        "Create your account",
        Some("Verify your email afterwards and we add $10 of free play."),
    )
    .child(body);
    if super::google::enabled(model) {
        section = section.child(
            super::google::form(model, "sign_up")
                .child(super::google::hidden(
                    "is_adult",
                    if form_state.is_adult { "true" } else { "false" },
                ))
                .child(super::google::hidden(
                    "accepts_terms",
                    if form_state.accepts_terms {
                        "true"
                    } else {
                        "false"
                    },
                ))
                .child(super::google::hidden(
                    "terms_version",
                    crate::wire::CURRENT_TERMS_VERSION,
                ))
                .child(super::google::button(
                    !form_state.is_adult || !form_state.accepts_terms,
                )),
        );
    }
    section
        .child(
            el("p")
                .class("aside")
                .text("Already have an account? ")
                .child(link("Sign in", Route::SignIn, "")),
        )
        .into()
}

fn terms_checkbox(is_checked: bool, error: Option<crate::wire::ApiError>) -> Node {
    let base = checkbox(
        Field::AcceptsTerms,
        "I accept the terms of service and the privacy notice",
        is_checked,
        error,
    );
    el("div")
        .class("terms")
        .child(base)
        .child(
            el("p")
                .class("hint")
                .text("Read the ")
                .child(
                    el("a")
                        .attr("href", "/legal/terms/")
                        .text("terms of service"),
                )
                .text(" and the ")
                .child(
                    el("a")
                        .attr("href", "/legal/privacy/")
                        .text("privacy notice"),
                )
                .text(" (drafts until counsel approves them)."),
        )
        .into()
}

pub(super) fn sign_in(form_state: &SignInForm, model: &Model) -> Node {
    let error = form_state.submit.error();
    let mut section = page("Sign in", None).child(
        form(Form::SignIn, "Sign in")
            .child(error_summary(error))
            .child(text_field(
                TextField {
                    field: Field::Email,
                    label: "Email address",
                    value: &form_state.email,
                    kind: InputKind::Email,
                    autocomplete: "email",
                    hint: None,
                },
                error,
            ))
            .child(text_field(
                TextField {
                    field: Field::Password,
                    label: "Password",
                    value: &form_state.password,
                    kind: InputKind::Password,
                    autocomplete: "current-password",
                    hint: None,
                },
                error,
            ))
            .child(submit_button(
                "Sign in",
                "Signing in",
                form_state.submit.is_sending(),
            ))
            .child(el("p").class("aside").child(link(
                "Forgot your password?",
                Route::ForgotPassword,
                "",
            ))),
    );
    if super::google::enabled(model) {
        section = section
            .child(super::google::form(model, "sign_in").child(super::google::button(false)));
    }
    if !form_state.dev_sign_in.is_empty() {
        let choices = form_state
            .dev_sign_in
            .iter()
            .enumerate()
            .map(|(index, account)| {
                button(
                    &format!("Development: sign in as {}", account.label),
                    Some(Intent::UseDevSignIn(index)),
                    ButtonKind::Quiet,
                    form_state.selected_dev_sign_in == Some(index),
                )
            });
        section = section.child(
            el("div")
                .class("dev-sign-in")
                .attr("aria-label", "Development sign-in choices")
                .children(choices.collect::<Vec<Node>>()),
        );
    }
    section
        .child(el("p").class("aside").text("New here? ").child(link(
            "Create a free account",
            Route::SignUp,
            "",
        )))
        .into()
}

pub(super) fn verify_pending(screen: &VerifyPending, model: &Model) -> Node {
    let account = model.account();
    if account.is_some_and(|account| account.is_verified) {
        return page(
            "Your email is verified",
            Some("Your free play credit is on your dashboard."),
        )
        .child(el("p").class("actions").child(link(
            "Go to your dashboard",
            Route::Dashboard,
            "btn btn-gilt",
        )))
        .into();
    }
    let sent = match &screen.sent_to {
        Some(address) => format!(
            "We sent a link to {address}. Open it to verify your address and get your $10 of free play. The link works for 24 hours."
        ),
        None => "Open the link in the email we sent you to verify your address and get your $10 of free play. The link works for 24 hours.".to_owned(),
    };
    let mut section =
        page("Check your email", Some(&sent)).child(error_summary(screen.resend.error()));
    section = if account.is_some() {
        if screen.is_resent {
            section.child(super::status_line(
                "A new link is on its way. Older links no longer work.",
            ))
        } else {
            section.child(
                el("p")
                    .class("aside")
                    .text("Nothing arrived? Check your spam folder, or ")
                    .child(button(
                        if screen.resend.is_sending() {
                            "Sending a new link"
                        } else {
                            "send a new link"
                        },
                        Some(Intent::ResendVerification),
                        ButtonKind::Quiet,
                        screen.resend.is_sending(),
                    )),
            )
        }
    } else {
        section.child(
            el("p")
                .class("aside")
                .text("Nothing arrived? Check your spam folder, or ")
                .child(link("sign in", Route::SignIn, ""))
                .text(" and ask for a new link."),
        )
    };
    section.into()
}

pub(super) fn verify_confirm(screen: &VerifyConfirm, model: &Model) -> Node {
    let next = if model.account().is_some() {
        link("Go to your dashboard", Route::Dashboard, "btn btn-gilt")
    } else {
        link("Sign in", Route::SignIn, "btn btn-gilt")
    };
    match screen.outcome {
        Some(VerifyOutcome::Verified) => page(
            "Your email is verified",
            Some("Your $10 of free play is ready."),
        )
        .child(el("p").class("actions").child(next))
        .into(),
        Some(VerifyOutcome::AlreadyVerified) => page(
            "This address is already verified",
            Some("There is nothing more to do."),
        )
        .child(el("p").class("actions").child(next))
        .into(),
        Some(VerifyOutcome::InvalidLink) => page(
            "This link has expired or was already used",
            Some("Verification links work once, for 24 hours. Sign in to ask for a new one."),
        )
        .child(el("p").class("actions").child(next))
        .into(),
        None => page(
            "Verify your email",
            Some("Set a password so only the person who owns this email controls the account."),
        )
        .child(
            form(Form::VerifyEmail, "Verify your email")
                .child(error_summary(screen.submit.error()))
                .child(text_field(
                    TextField {
                        field: crate::state::Field::Password,
                        label: "New password",
                        value: &screen.password,
                        kind: InputKind::Password,
                        autocomplete: "new-password",
                        hint: Some(PASSWORD_HINT),
                    },
                    screen.submit.error(),
                ))
                .child(text_field(
                    TextField {
                        field: crate::state::Field::Confirmation,
                        label: "New password again",
                        value: &screen.confirmation,
                        kind: InputKind::Password,
                        autocomplete: "new-password",
                        hint: None,
                    },
                    screen.submit.error(),
                ))
                .child(submit_button(
                    "Verify my email",
                    "Verifying",
                    screen.submit.is_sending(),
                )),
        )
        .into(),
    }
}

pub(super) fn forgot(form_state: &ForgotForm) -> Node {
    if form_state.is_sent {
        return page(
            "Check your email",
            Some("If an account uses that address, we have sent it a link to choose a new password. The link works once, for 15 minutes."),
        )
        .child(el("p").class("actions").child(link("Back to sign in", Route::SignIn, "btn btn-outline")))
        .into();
    }
    let error = form_state.submit.error();
    page(
        "Reset your password",
        Some("Enter the address you signed up with and we will email you a link."),
    )
    .child(
        form(Form::Forgot, "Reset your password")
            .child(error_summary(error))
            .child(text_field(
                TextField {
                    field: Field::Email,
                    label: "Email address",
                    value: &form_state.email,
                    kind: InputKind::Email,
                    autocomplete: "email",
                    hint: None,
                },
                error,
            ))
            .child(submit_button(
                "Email me a reset link",
                "Sending",
                form_state.submit.is_sending(),
            )),
    )
    .into()
}

pub(super) fn reset(form_state: &ResetForm) -> Node {
    if form_state.token.is_none() {
        return page(
            "This reset link has expired or was already used",
            Some("Reset links work once, for 15 minutes. Ask for a new one."),
        )
        .child(el("p").class("actions").child(link(
            "Ask for a new link",
            Route::ForgotPassword,
            "btn btn-gilt",
        )))
        .into();
    }
    let error = form_state.submit.error();
    page(
        "Choose a new password",
        Some("Every device signed in to your account will be signed out."),
    )
    .child(
        form(Form::Reset, "Choose a new password")
            .child(error_summary(error))
            .child(text_field(
                TextField {
                    field: Field::Password,
                    label: "New password",
                    value: &form_state.password,
                    kind: InputKind::Password,
                    autocomplete: "new-password",
                    hint: Some(PASSWORD_HINT),
                },
                error,
            ))
            .child(text_field(
                TextField {
                    field: Field::Confirmation,
                    label: "New password again",
                    value: &form_state.confirmation,
                    kind: InputKind::Password,
                    autocomplete: "new-password",
                    hint: None,
                },
                error,
            ))
            .child(submit_button(
                "Save new password",
                "Saving",
                form_state.submit.is_sending(),
            )),
    )
    .into()
}

pub(super) fn signed_out() -> Node {
    page(
        "You are signed out",
        Some("This device no longer has access to your account."),
    )
    .child(
        el("p")
            .class("actions")
            .child(link("Sign in again", Route::SignIn, "btn btn-gilt"))
            .child(
                el("a")
                    .class("btn btn-outline")
                    .attr("href", "/")
                    .text("Back to DungeonFlux"),
            ),
    )
    .into()
}
