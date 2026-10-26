//! The customer account settings screen.

use super::submit_control;
use crate::view::{
    ButtonKind, InputKind, TextField, button, checkbox, error_summary, fact, facts, failed, form,
    page, remote, status_line, submit_button, text_field,
};
use crate::{
    copy, money,
    node::{Element, Node, el},
    state::{Field, Form, Intent, Model, Remote, SettingsScreen},
    wire::{ApiError, ClosureBlocker, Refusal, SettingsView},
};

pub(in crate::view) fn settings(screen: &SettingsScreen, model: &Model) -> Node {
    page("Account settings", None)
        .child(remote(
            &screen.settings,
            "Loading your settings",
            |settings| settings_content(screen, settings, model),
        ))
        .into()
}

fn settings_content(screen: &SettingsScreen, settings: &SettingsView, model: &Model) -> Node {
    el("div")
        .class("settings")
        .child(account_details_section(settings, model.offset_east_minutes))
        .child(sign_in_methods(settings, model))
        .child(profile_section(screen, settings))
        .child(crate::view::link(
            "Developer tokens",
            crate::route::Route::DeveloperTokens,
            "button",
        ))
        .child(
            if settings
                .sign_in_methods
                .contains(&crate::wire::SignInMethod::Password)
            {
                email_section(screen)
            } else {
                el("div")
            },
        )
        .child(
            if settings
                .sign_in_methods
                .contains(&crate::wire::SignInMethod::Password)
            {
                password_section(screen)
            } else {
                el("div")
            },
        )
        .child(preferences_section(screen))
        .child(export_section(screen, settings))
        .child(devices_section(screen, settings, model.offset_east_minutes))
        .child(close_section(screen, settings))
        .into()
}

fn account_details_section(settings: &SettingsView, offset: i32) -> Element {
    let consent = |label: &str, value: String| fact(label, value);
    let facts_list = vec![
        fact("Email address", settings.email.clone()),
        fact(
            "Display name",
            settings
                .display_name
                .as_deref()
                .unwrap_or("Not set")
                .to_owned(),
        ),
        fact(
            "Account created",
            if settings.created_at_ms == 0 {
                "Not recorded".to_owned()
            } else {
                money::date_time(settings.created_at_ms, offset)
            },
        ),
        fact(
            "Email verified",
            if settings.is_verified {
                "Yes"
            } else {
                "Not yet"
            }
            .to_owned(),
        ),
        consent(
            "Terms accepted",
            match (
                settings.terms_version.as_deref(),
                settings.terms_accepted_at_ms,
            ) {
                (Some(version), Some(at)) => {
                    format!("{version}, {}", money::date_time(at, offset))
                }
                (Some(version), None) => version.to_owned(),
                _ => "Not recorded".to_owned(),
            },
        ),
        consent(
            "18 or older confirmed",
            settings.adult_attested_at_ms.map_or_else(
                || "Not recorded".to_owned(),
                |at| money::date_time(at, offset),
            ),
        ),
        fact(
            "Account state",
            match settings.closure_state {
                crate::wire::ClosureState::Open => "Open",
                crate::wire::ClosureState::Closing => "Closing",
            }
            .to_owned(),
        ),
        fact(
            "Sign-in methods",
            if settings.sign_in_methods.is_empty() {
                "Password".to_owned()
            } else {
                settings
                    .sign_in_methods
                    .iter()
                    .map(|method| match method {
                        crate::wire::SignInMethod::Password => "Password",
                        crate::wire::SignInMethod::Google => "Google",
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        ),
    ];
    el("section")
        .class("section settings-section")
        .attr("aria-labelledby", "account-details-title")
        .child(
            el("h2")
                .attr("id", "account-details-title")
                .text("Your account"),
        )
        .child(facts(facts_list))
}

fn sign_in_methods(settings: &SettingsView, model: &Model) -> Element {
    use crate::view::google;
    use crate::wire::SignInMethod;
    let linked = settings.sign_in_methods.contains(&SignInMethod::Google);
    let password = settings.sign_in_methods.contains(&SignInMethod::Password);
    let mut section = el("section")
        .class("section settings-section")
        .attr("aria-labelledby", "sign-in-methods-title")
        .child(
            el("h2")
                .attr("id", "sign-in-methods-title")
                .text("Sign-in methods"),
        );
    if linked {
        section = section.child(el("p").text("Google is linked to your account."));
        if google::enabled(model) {
            section = section.child(google::form(model, "reauthenticate")
                .child(el("p").text("Confirm with Google before exporting data or closing your account. Close within five minutes; leave the password field empty."))
                .child(google::button(false)));
        }
        if password {
            section = section.child(
                el("form")
                    .class("form google-form")
                    .attr("method", "post")
                    .attr("action", "/app/api/oauth/google/unlink")
                    .attr("data-df-oauth", "google")
                    .attr("aria-label", "Unlink Google")
                    .child(google::hidden("csrf", model.csrf()))
                    .child(google::password("google-unlink-password"))
                    .child(
                        el("button")
                            .class("btn btn-outline")
                            .attr("type", "submit")
                            .text("Unlink Google"),
                    ),
            );
        } else {
            section = section.child(
                el("p").text("Google is your only sign-in method, so it cannot be removed."),
            );
        }
    } else if google::enabled(model) && password {
        section = section.child(
            google::form(model, "link")
                .child(el("p").text(
                    "Enter your current password, then confirm your Google account to link it.",
                ))
                .child(google::password("google-link-password"))
                .child(google::button(false)),
        );
    }
    section
}

fn form_section(
    id: &str,
    title: &str,
    intro: &str,
    success: Option<&str>,
    form_node: Node,
) -> Element {
    let mut section = el("section")
        .class("section settings-section")
        .attr("aria-labelledby", id.to_owned())
        .child(el("h2").attr("id", id.to_owned()).text(title))
        .child(el("p").text(intro));
    if let Some(success) = success {
        section = section.child(status_line(success));
    }
    section.child(form_node)
}

fn email_section(screen: &SettingsScreen) -> Element {
    form_section(
        "email-title",
        "Change email address",
        "We send a confirmation to the new address. Your address changes only after you confirm it there.",
        screen
            .is_email_change_sent
            .then_some("Check the new address and confirm the change there."),
        form(Form::ChangeEmail, "Change email address")
            .child(error_summary(screen.email_change.error()))
            .child(text_field(
                TextField {
                    field: Field::NewEmail,
                    label: "New email address",
                    value: &screen.new_email,
                    kind: InputKind::Email,
                    autocomplete: "email",
                    hint: None,
                },
                screen.email_change.error(),
            ))
            .child(text_field(
                TextField {
                    field: Field::EmailPassword,
                    label: "Your password",
                    value: &screen.email_password,
                    kind: InputKind::Password,
                    autocomplete: "current-password",
                    hint: Some("We ask for it again so nobody else can change your address."),
                },
                screen.email_change.error(),
            ))
            .child(submit_button(
                "Send confirmation",
                "Sending confirmation",
                screen.email_change.is_sending(),
            ))
            .into(),
    )
}

fn password_section(screen: &SettingsScreen) -> Element {
    form_section(
        "password-title",
        "Change password",
        "Changing your password signs out your other devices.",
        screen
            .is_password_changed
            .then_some("Password changed. Other devices are signed out."),
        form(Form::ChangePassword, "Change password")
            .child(error_summary(screen.password_change.error()))
            .child(text_field(
                TextField {
                    field: Field::CurrentPassword,
                    label: "Current password",
                    value: &screen.current_password,
                    kind: InputKind::Password,
                    autocomplete: "current-password",
                    hint: None,
                },
                screen.password_change.error(),
            ))
            .child(text_field(
                TextField {
                    field: Field::Password,
                    label: "New password",
                    value: &screen.new_password,
                    kind: InputKind::Password,
                    autocomplete: "new-password",
                    hint: Some(
                        "At least 12 characters. A short sentence you will remember works well.",
                    ),
                },
                screen.password_change.error(),
            ))
            .child(text_field(
                TextField {
                    field: Field::Confirmation,
                    label: "New password again",
                    value: &screen.new_confirmation,
                    kind: InputKind::Password,
                    autocomplete: "new-password",
                    hint: None,
                },
                screen.password_change.error(),
            ))
            .child(submit_button(
                "Change password",
                "Changing password",
                screen.password_change.is_sending(),
            ))
            .into(),
    )
}

fn preferences_section(screen: &SettingsScreen) -> Element {
    form_section(
        "preferences-title",
        "Email choices",
        "Receipts, security and other transactional mail always arrives; you cannot turn it off.",
        screen
            .is_preferences_saved
            .then_some("Email choices saved."),
        form(Form::Preferences, "Email choices")
            .child(error_summary(screen.preferences.error()))
            .child(checkbox(
                Field::ProductNews,
                "Product news",
                screen.product_news,
                screen.preferences.error(),
            ))
            .child(checkbox(
                Field::SessionReminders,
                "Session reminders",
                screen.session_reminders,
                screen.preferences.error(),
            ))
            .child(submit_button(
                "Save email choices",
                "Saving email choices",
                screen.preferences.is_sending(),
            ))
            .into(),
    )
}

fn export_section(screen: &SettingsScreen, settings: &SettingsView) -> Element {
    form_section(
        "export-title",
        "Download my data",
        "We prepare a copy of your account data for you to download.",
        screen.export_view.as_ref().map(|view| match view.status {
            crate::wire::ExportStatus::NotRequested => "No download requested yet.",
            crate::wire::ExportStatus::Requested => "Your download is being prepared.",
            crate::wire::ExportStatus::Ready => "Your download is ready.",
        }),
        form(Form::Export, "Download my data")
            .child(error_summary(screen.export.error()))
            .child(if settings.sign_in_methods.contains(&crate::wire::SignInMethod::Password) { text_field(
                TextField {
                    field: Field::ExportPassword,
                    label: "Your password",
                    value: &screen.export_password,
                    kind: InputKind::Password,
                    autocomplete: "current-password",
                    hint: None,
                },
                screen.export.error(),
            ) } else { el("p").text("Confirm with Google in Sign-in methods above before requesting your data.").into() })
            .child(submit_button(
                "Request download",
                "Requesting download",
                screen.export.is_sending(),
            ))
            .into(),
    )
}

fn profile_section(screen: &SettingsScreen, settings: &SettingsView) -> Element {
    let mut profile = el("section")
        .class("section settings-section")
        .attr("aria-labelledby", "profile-title")
        .child(el("h2").attr("id", "profile-title").text("Profile"))
        .child(facts(vec![fact("Email address", settings.email.clone())]));
    if screen.is_profile_saved {
        profile = profile.child(status_line("Display name saved."));
    }
    profile.child(
        form(Form::Profile, "Profile")
            .child(error_summary(screen.profile.error()))
            .child(text_field(
                TextField {
                    field: Field::DisplayName,
                    label: "Display name",
                    value: &screen.display_name,
                    kind: InputKind::Text,
                    autocomplete: "nickname",
                    hint: Some("The people at your tables see this name."),
                },
                screen.profile.error(),
            ))
            .child(submit_control(
                &screen.profile_operation,
                "Save display name",
                "Saving display name",
                screen.profile.is_sending(),
            )),
    )
}

fn devices_section(
    screen: &SettingsScreen,
    settings: &SettingsView,
    offset_east_minutes: i32,
) -> Element {
    let others = settings
        .sessions
        .iter()
        .filter(|device| !device.is_current)
        .count();
    let rows = settings.sessions.iter().map(|device| {
        let mut title = el("span")
            .class("row-title")
            .text(copy::device_name(device.device));
        if device.is_current {
            title = title
                .text(" ")
                .child(el("span").class("pill pill-ok").text("This device"));
        }
        let sign_out_label = "Sign out this device";
        el("li")
            .child(el("div").class("row-main").child(title).child(
                el("span").class("row-meta").text(format!(
                    "Signed in {}. Last seen {}.",
                    money::date_time(device.signed_in_at_ms, offset_east_minutes),
                    money::date_time(device.last_seen_at_ms, offset_east_minutes)
                )),
            ))
            .child(el("div").class("row-actions").child(button(
                sign_out_label,
                Some(Intent::RevokeSession {
                    session_id: device.session_id.clone(),
                }),
                ButtonKind::Destructive,
                device.session_id.is_empty(),
            )))
            .into()
    });
    let mut devices = el("section")
        .class("section settings-section")
        .attr("aria-labelledby", "devices-title")
        .child(
            el("h2")
                .attr("id", "devices-title")
                .text("Signed-in devices"),
        )
        .child(error_summary(screen.revoke.error()));
    devices = if settings.sessions.is_empty() {
        devices.child(el("p").text("No devices are listed."))
    } else {
        devices.child(el("ul").class("rows").children(rows.collect::<Vec<Node>>()))
    };
    if let Some(count) = screen.revoked {
        devices = devices.child(status_line(match count {
            0 => "No other devices were signed in.".to_owned(),
            1 => "Signed out 1 other device.".to_owned(),
            count => format!("Signed out {count} other devices."),
        }));
    }
    if others > 0 {
        let is_sending = screen.revoke.is_sending();
        devices = devices.child(
            el("p")
                .class("actions")
                .child(button(
                    if is_sending {
                        "Signing out other devices"
                    } else {
                        "Sign out other devices"
                    },
                    Some(Intent::RevokeOtherSessions),
                    ButtonKind::Destructive,
                    is_sending,
                ))
                .child(button(
                    "Sign out everywhere",
                    Some(Intent::RevokeAllSessions),
                    ButtonKind::Destructive,
                    is_sending,
                )),
        );
    }
    devices
}

fn blocker_words(blocker: &ClosureBlocker) -> String {
    match blocker {
        ClosureBlocker::HostsActiveCampaign { campaign_name } => format!(
            "You host an active campaign: {campaign_name}. Hand it over or finish it first."
        ),
        ClosureBlocker::PurchasedBalance {
            amount_micro_dollars,
        } => format!(
            "You still have {} of purchased credit. Use it, or ask for a refund, first.",
            money::balance(*amount_micro_dollars)
        ),
        ClosureBlocker::PendingHolds {
            amount_micro_dollars,
        } => format!(
            "{} is held for pending work. Wait for it to settle first.",
            money::balance(*amount_micro_dollars)
        ),
        ClosureBlocker::OpenDispute => {
            "A payment dispute is open. It must be resolved first.".to_owned()
        }
    }
}

fn close_section(screen: &SettingsScreen, settings: &SettingsView) -> Element {
    let error = screen.close.error();
    let mut close = el("section")
        .class("panel panel-warn settings-section settings-close")
        .attr("aria-labelledby", "close-title")
        .child(el("h2").attr("id", "close-title").text("Close your account"))
        .child(el("p").text("Before you decide, this is what closing does:"))
        .child(
            el("ul")
                .class("plain-list")
                .child(el("li").text("It signs you out on every device."))
                .child(el("li").text(
                    "Your personal data is deleted after a grace period. If you sign in before the deletion date, your account reopens.",
                ))
                .child(el("li").text("Any free play credit is forfeited."))
                .child(el("li").text(
                    "Purchased credit must be refunded or used before you can close.",
                )),
        );
    if !settings.closure_blockers.is_empty() {
        close = close
            .child(
                el("p")
                    .class("problem-what")
                    .text("Before you can close your account:"),
            )
            .child(
                el("ul").class("plain-list blockers").children(
                    settings
                        .closure_blockers
                        .iter()
                        .map(|blocker| el("li").text(blocker_words(blocker)).into())
                        .collect::<Vec<Node>>(),
                ),
            );
    }
    close.child(
        form(Form::Close, "Close your account")
            .child(close_summary(error))
            .child(checkbox(
                Field::CloseUnderstood,
                "I understand that closing signs me out everywhere and starts the deletion of my personal data",
                screen.close_understood,
                error,
            ))
            .child(if settings.sign_in_methods.contains(&crate::wire::SignInMethod::Password) { close_password_field(screen, error) } else { el("p").text("Confirm with Google in Sign-in methods above, then close within five minutes.").into() })
            .child(match &screen.close_operation {
                Remote::Failed(error) => failed(*error),
                Remote::Loading => button("Close my account", None, ButtonKind::Destructive, true),
                Remote::Ready(_) => {
                    let is_sending = screen.close.is_sending();
                    button(
                        if is_sending {
                            "Closing your account"
                        } else {
                            "Close my account"
                        },
                        None,
                        ButtonKind::Destructive,
                        is_sending,
                    )
                }
            }),
    )
}

fn is_wrong_password(error: Option<ApiError>) -> bool {
    error.is_some_and(|error| error.refusal == Some(Refusal::WrongPassword))
}

/// A wrong password is refused against the sign-in password field in `copy::refusal_field`, but
/// this form's field is `close-password`, so the link and the inline message are built here.
fn close_summary(error: Option<ApiError>) -> Node {
    let Some(refused) = error.filter(|error| is_wrong_password(Some(*error))) else {
        return error_summary(error);
    };
    let problem = copy::problem(refused);
    el("div")
        .class("error-summary")
        .attr("role", "alert")
        .attr("tabindex", "-1")
        .attr("data-df-focus", "error-summary")
        .child(el("h2").text("There is a problem"))
        .child(
            el("p").child(
                el("a")
                    .attr("href", format!("#{}", Field::ClosePassword.name()))
                    .text(problem.what),
            ),
        )
        .into()
}

fn close_password_field(screen: &SettingsScreen, error: Option<ApiError>) -> Node {
    let field = text_field(
        TextField {
            field: Field::ClosePassword,
            label: "Your password",
            value: &screen.close_password,
            kind: InputKind::Password,
            autocomplete: "current-password",
            hint: Some("We ask for it again so nobody else can close your account."),
        },
        error,
    );
    if !is_wrong_password(error) {
        return field;
    }
    el("div")
        .class("has-error")
        .child(field)
        .child(
            el("p")
                .class("field-error")
                .attr("id", "close-password-error")
                .text(copy::refusal_message(Refusal::WrongPassword)),
        )
        .into()
}
