//! Every screen as an element tree, built from the model alone (no clock, no I/O).
//!
//! Shared rules: one `h1` per screen, focusable for navigation; every control has a visible text
//! label; a refused form shows an error summary that names the problem, the fix and links to the
//! field; loading, empty and failed states each say what is happening and what to do.

mod admin;
mod auth;
mod campaign;
mod customer;
mod google;
mod integration;

use crate::{
    copy,
    node::{Element, Node, el},
    route::{AdminRoute, Route},
    state::{Field, Form, Intent, Model, Notice, Remote, Screen},
    wire::ApiError,
};

/// The whole page for the current model.
pub fn view(model: &Model) -> Node {
    let is_operator_screen = matches!(model.screen, Screen::Admin(_));
    el("div")
        .class(if is_operator_screen {
            "shell shell-operator"
        } else {
            "shell"
        })
        .child(
            el("a")
                .class("skip")
                .attr("href", "#main")
                .text("Skip to content"),
        )
        .child(top_bar(model))
        .child(
            el("main")
                .attr("id", "main")
                .class("main")
                .child(notice(model))
                .child(screen(model)),
        )
        .into()
}

fn screen(model: &Model) -> Node {
    match &model.screen {
        Screen::Integration(screen) => integration::view(screen),
        Screen::Booting => page_loading("Opening your account"),
        Screen::BootFailed(error) => failed_page("We could not reach DungeonFlux", *error),
        Screen::Landing => auth::landing(),
        Screen::SignUp(form) => auth::sign_up(form, model),
        Screen::SignIn(form) => auth::sign_in(form, model),
        Screen::VerifyPending(screen) => auth::verify_pending(screen, model),
        Screen::VerifyConfirm(screen) => auth::verify_confirm(screen, model),
        Screen::Forgot(form) => auth::forgot(form),
        Screen::Reset(form) => auth::reset(form),
        Screen::SignedOut => auth::signed_out(),
        Screen::Dashboard(screen) => customer::dashboard(screen, model),
        Screen::AddCredit(form) => customer::add_credit(form),
        Screen::CreditReturn(screen) => customer::credit_return(screen),
        Screen::Funding(screen) => customer::funding(screen),
        Screen::Statements(screen) => customer::statements(screen, model.offset_east_minutes),
        Screen::PlaySessions(screen) => customer::play_sessions(screen, model.offset_east_minutes),
        Screen::Assets(screen) => customer::assets(screen, model.offset_east_minutes),
        Screen::Settings(screen) => customer::settings(screen, model),
        Screen::NewCampaign(screen) => campaign::wizard(screen, model),
        Screen::Campaign(screen) => campaign::overview(screen, model.offset_east_minutes),
        Screen::Lobby(screen) => campaign::lobby(screen, model.offset_east_minutes),
        Screen::Play(screen) => campaign::play(screen),
        Screen::Join(screen) => campaign::join(screen, model),
        Screen::Admin(screen) => admin::console(screen, model),
        Screen::NotPermitted => message_page(
            "This page is for DungeonFlux operators",
            "Your account does not have operator access. Your own tables and credit are on your dashboard.",
            Some(("Go to your dashboard", Route::Dashboard)),
        ),
        Screen::NeedsSecondFactor => message_page(
            "Confirm it is you with your second step",
            "The operator console needs the second sign-in step for this session. Sign out, then sign in again and complete the second step.",
            Some(("Go to your dashboard", Route::Dashboard)),
        ),
        Screen::NotFound => message_page(
            "We could not find that page",
            "The link may be old or mistyped.",
            Some(("Go to the start", Route::Landing)),
        ),
    }
}

fn top_bar(model: &Model) -> Node {
    let account = model.account();
    let mut nav = el("nav").class("bar-nav").attr("aria-label", "Account");
    if let Some(account) = account {
        let current = &model.route;
        let items = [
            ("Overview", Route::Dashboard),
            ("Play sessions", Route::PlaySessions),
            ("Assets", Route::Assets),
            ("Funds", Route::Funding),
            ("Statements", Route::Statements),
            ("Your account", Route::Settings),
            ("Security", Route::Security),
        ];
        for (label, route) in items {
            nav = nav.child(nav_link(label, route, current));
        }
        if account.operator.is_some() {
            nav = nav.child(nav_link(
                "Operator console",
                Route::Admin(AdminRoute::Summary),
                current,
            ));
        }
    }
    let actions = match account {
        Some(account) => el("div")
            .class("bar-actions")
            .child(
                el("span").class("bar-who").text(
                    account
                        .display_name
                        .clone()
                        .unwrap_or_else(|| account.email.clone()),
                ),
            )
            .child(button(
                "Sign out",
                Some(Intent::SignOut),
                ButtonKind::Quiet,
                false,
            )),
        None => el("div")
            .class("bar-actions")
            .child(link("Sign in", Route::SignIn, "bar-link"))
            .child(link(
                "Create a free account",
                Route::SignUp,
                "btn btn-gilt btn-small",
            )),
    };
    el("header")
        .class("bar")
        .child(
            el("div")
                .class("bar-inner")
                .child(brand())
                .child(nav)
                .child(actions),
        )
        .into()
}

fn brand() -> Node {
    el("a")
        .class("mark")
        .attr("href", "/")
        .attr("aria-label", "DungeonFlux home")
        .child(el("span").class("mark-dungeon").text("Dungeon"))
        .child(el("span").class("mark-flux").text("Flux"))
        .into()
}

fn nav_link(label: &str, route: Route, current: &Route) -> Node {
    let is_current = same_section(&route, current);
    let mut anchor = el("a")
        .class(if is_current {
            "nav-link is-current"
        } else {
            "nav-link"
        })
        .attr("href", route.path())
        .on_click(Intent::Go(route))
        .text(label);
    if is_current {
        anchor = anchor.attr("aria-current", "page");
    }
    anchor.into()
}

fn same_section(link: &Route, current: &Route) -> bool {
    match (link, current) {
        (Route::Admin(_), Route::Admin(_)) => true,
        (Route::Funding, Route::AddCredit | Route::CreditReturn { .. }) => true,
        (Route::AddCredit, Route::CreditReturn { .. }) => true,
        (
            Route::Dashboard,
            Route::Campaign { .. } | Route::Lobby { .. } | Route::NewCampaign | Route::Join { .. },
        ) => true,
        _ => link == current,
    }
}

fn notice(model: &Model) -> Node {
    let Some(notice) = model.notice else {
        return el("div").class("notice-slot").into();
    };
    let message = match notice {
        Notice::SignInToContinue => "Sign in to continue to that page.".to_owned(),
        Notice::SessionEnded => {
            "Your session ended, so you were signed out. Sign in again to carry on.".to_owned()
        }
        Notice::EmailVerified => "Email verified. Your $10 of free play is ready.".to_owned(),
        Notice::SignOutFailed => {
            "We could not confirm that you were signed out. Check your connection and press Sign out again."
                .to_owned()
        }
        Notice::PasswordChanged => {
            "Your password is changed and every device was signed out. Sign in with the new password."
                .to_owned()
        }
        Notice::AccountClosed { delete_after_ms } => format!(
            "Your account is closed. Signing in before {} reopens it; after that your personal data is deleted.",
            crate::money::date(delete_after_ms, model.offset_east_minutes)
        ),
    };
    el("div")
        .class("notice-slot")
        .child(
            el("p")
                .class("notice")
                .attr("role", "status")
                .attr("tabindex", "-1")
                .attr("data-df-focus", "notice")
                .text(message),
        )
        .into()
}

/// A link that the app handles without a page load.
pub(crate) fn link(label: &str, route: Route, class: &str) -> Node {
    el("a")
        .class(class)
        .attr("href", route.path())
        .on_click(Intent::Go(route))
        .text(label)
        .into()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ButtonKind {
    /// The one main action of a screen or form (gilt).
    Primary,
    Secondary,
    /// Removes, closes or revokes something.
    Destructive,
    Quiet,
}

impl ButtonKind {
    const fn class(self) -> &'static str {
        match self {
            Self::Primary => "btn btn-gilt",
            Self::Secondary => "btn btn-outline",
            Self::Destructive => "btn btn-danger",
            Self::Quiet => "btn btn-quiet",
        }
    }
}

/// A `type=button` control with a click intent, or `type=submit` when `intent` is `None`.
pub(crate) fn button(label: &str, intent: Option<Intent>, kind: ButtonKind, is_busy: bool) -> Node {
    let mut control = el("button")
        .class(kind.class())
        .attr("type", if intent.is_some() { "button" } else { "submit" })
        .flag("disabled", is_busy)
        .text(label);
    if is_busy {
        control = control.attr("aria-busy", "true");
    }
    if let Some(intent) = intent {
        control = control.on_click(intent);
    }
    control.into()
}

/// The submit button: while sending it says so and cannot be pressed twice (the server still
/// enforces idempotency; plan 11).
pub(crate) fn submit_button(label: &str, sending_label: &str, is_sending: bool) -> Node {
    button(
        if is_sending { sending_label } else { label },
        None,
        ButtonKind::Primary,
        is_sending,
    )
}

pub(crate) fn form(form: Form, label: &str) -> Element {
    el("form")
        .class("form")
        .attr("novalidate", "")
        .attr("aria-label", label)
        .on_submit(Intent::Submit(form))
}

pub(crate) fn heading(title: &str) -> Element {
    el("h1")
        .attr("tabindex", "-1")
        .attr("data-df-focus", "heading")
        .text(title)
}

pub(crate) fn page(title: &str, lede: Option<&str>) -> Element {
    titled_page("page", title, lede)
}

/// A page whose content (tables, columns) uses the full width instead of the form measure.
pub(crate) fn page_wide(title: &str, lede: Option<&str>) -> Element {
    titled_page("page is-wide", title, lede)
}

fn titled_page(class: &str, title: &str, lede: Option<&str>) -> Element {
    let mut section = el("section").class(class).child(heading(title));
    if let Some(lede) = lede {
        section = section.child(el("p").class("lede").text(lede));
    }
    section
}

pub(crate) fn page_loading(what: &str) -> Node {
    page(what, None).child(loading(what)).into()
}

pub(crate) fn loading(what: &str) -> Node {
    el("p")
        .class("loading")
        .attr("role", "status")
        .text(format!("{what}..."))
        .into()
}

/// A failed read: what happened, what to do, and a retry.
pub(crate) fn failed(error: ApiError) -> Node {
    let problem = copy::problem(error);
    el("div")
        .class("problem")
        .attr("role", "alert")
        .child(el("p").class("problem-what").text(problem.what))
        .child(el("p").text(problem.fix))
        .child(button(
            "Try again",
            Some(Intent::Retry),
            ButtonKind::Secondary,
            false,
        ))
        .into()
}

fn failed_page(title: &str, error: ApiError) -> Node {
    page(title, None).child(failed(error)).into()
}

pub(crate) fn message_page(title: &str, body: &str, action: Option<(&str, Route)>) -> Node {
    let mut section = page(title, Some(body));
    if let Some((label, route)) = action {
        section = section.child(
            el("p")
                .class("actions")
                .child(link(label, route, "btn btn-gilt")),
        );
    }
    section.into()
}

/// Renders a remote value: loading line, failure with retry, or the ready content.
pub(crate) fn remote<T>(value: &Remote<T>, what: &str, ready: impl FnOnce(&T) -> Node) -> Node {
    match value {
        Remote::Loading => loading(what),
        Remote::Failed(error) => failed(*error),
        Remote::Ready(value) => ready(value),
    }
}

/// The error summary of a refused form, focused after the refusal.
pub(crate) fn error_summary(error: Option<ApiError>) -> Node {
    let Some(error) = error else {
        return el("div").class("error-slot").into();
    };
    let problem = copy::problem(error);
    let mut summary = el("div")
        .class("error-summary")
        .attr("role", "alert")
        .attr("tabindex", "-1")
        .attr("data-df-focus", "error-summary")
        .child(el("h2").text("There is a problem"));
    let field = error.refusal.and_then(copy::refusal_field);
    summary = match field {
        Some(field) => summary.child(
            el("p").child(
                el("a")
                    .attr("href", format!("#{}", field.name()))
                    .text(problem.what),
            ),
        ),
        None => summary.child(el("p").text(problem.what)),
    };
    if !problem.fix.is_empty() {
        summary = summary.child(el("p").text(problem.fix));
    }
    summary.into()
}

/// The refusal message for `field`, when the form's error is about it.
fn field_error(field: Field, error: Option<ApiError>) -> Option<&'static str> {
    let refusal = error?.refusal?;
    (copy::refusal_field(refusal) == Some(field)).then(|| copy::refusal_message(refusal))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputKind {
    Text,
    Email,
    Password,
    /// A dollar amount typed as text (`inputmode=decimal`).
    Dollars,
}

/// A labelled input with an optional hint and its inline error.
pub(crate) struct TextField<'a> {
    pub field: Field,
    pub label: &'a str,
    pub value: &'a str,
    pub kind: InputKind,
    pub autocomplete: &'a str,
    pub hint: Option<&'a str>,
}

pub(crate) fn text_field(spec: TextField<'_>, error: Option<ApiError>) -> Node {
    let id = spec.field.name();
    let message = field_error(spec.field, error);
    let mut described = Vec::new();
    let mut wrapper = el("div").class(if message.is_some() {
        "field has-error"
    } else {
        "field"
    });
    wrapper = wrapper.child(el("label").attr("for", id).text(spec.label));
    if let Some(hint) = spec.hint {
        described.push(format!("{id}-hint"));
        wrapper = wrapper.child(
            el("p")
                .class("hint")
                .attr("id", format!("{id}-hint"))
                .text(hint),
        );
    }
    if let Some(message) = message {
        described.push(format!("{id}-error"));
        wrapper = wrapper.child(
            el("p")
                .class("field-error")
                .attr("id", format!("{id}-error"))
                .text(message),
        );
    }
    let (kind, input_mode) = match spec.kind {
        InputKind::Text => ("text", None),
        InputKind::Email => ("email", None),
        InputKind::Password => ("password", None),
        InputKind::Dollars => ("text", Some("decimal")),
    };
    let mut input = el("input")
        .attr("id", id)
        .attr("name", id)
        .attr("type", kind)
        .attr("autocomplete", spec.autocomplete)
        .attr("value", spec.value)
        .bind(spec.field);
    if let Some(mode) = input_mode {
        input = input.attr("inputmode", mode);
    }
    if !described.is_empty() {
        input = input.attr("aria-describedby", described.join(" "));
    }
    if message.is_some() {
        input = input.attr("aria-invalid", "true");
    }
    wrapper.child(input).into()
}

pub(crate) fn checkbox(
    field: Field,
    label: &str,
    is_checked: bool,
    error: Option<ApiError>,
) -> Node {
    let id = field.name();
    let message = field_error(field, error);
    let mut input = el("input")
        .attr("id", id)
        .attr("name", id)
        .attr("type", "checkbox")
        .flag("checked", is_checked)
        .bind(field);
    if message.is_some() {
        input = input
            .attr("aria-invalid", "true")
            .attr("aria-describedby", format!("{id}-error"));
    }
    let mut wrapper = el("div")
        .class(if message.is_some() {
            "check has-error"
        } else {
            "check"
        })
        .child(input)
        .child(el("label").attr("for", id).text(label));
    if let Some(message) = message {
        wrapper = wrapper.child(
            el("p")
                .class("field-error")
                .attr("id", format!("{id}-error"))
                .text(message),
        );
    }
    wrapper.into()
}

/// One radio choice with a title and a description line.
pub(crate) struct RadioChoice<'a> {
    pub field: Field,
    pub value: &'a str,
    pub title: String,
    pub detail: String,
    pub is_checked: bool,
}

pub(crate) fn radio_group(
    legend: &str,
    field: Field,
    choices: Vec<RadioChoice<'_>>,
    error: Option<ApiError>,
) -> Node {
    let message = field_error(field, error);
    let mut set = el("fieldset")
        .class(if message.is_some() {
            "choices has-error"
        } else {
            "choices"
        })
        .attr("id", field.name())
        .child(el("legend").text(legend));
    if let Some(message) = message {
        set = set.child(el("p").class("field-error").text(message));
    }
    for choice in choices {
        let id = format!("{}-{}", choice.field.name(), choice.value);
        set = set.child(
            el("div")
                .class(if choice.is_checked {
                    "choice is-checked"
                } else {
                    "choice"
                })
                .child(
                    el("input")
                        .attr("id", id.clone())
                        .attr("type", "radio")
                        .attr("name", choice.field.name())
                        .attr("value", choice.value)
                        .flag("checked", choice.is_checked)
                        .bind(choice.field),
                )
                .child(
                    el("label")
                        .attr("for", id)
                        .child(el("span").class("choice-title").text(choice.title))
                        .child(el("span").class("choice-detail").text(choice.detail)),
                ),
        );
    }
    set.into()
}

/// A definition row: a label and its value.
pub(crate) fn fact(label: &str, value: impl Into<String>) -> Node {
    el("div")
        .class("fact")
        .child(el("dt").text(label))
        .child(el("dd").text(value))
        .into()
}

pub(crate) fn facts(rows: Vec<Node>) -> Node {
    el("dl").class("facts").children(rows).into()
}

pub(crate) fn status_line(message: impl Into<String>) -> Node {
    el("p")
        .class("success")
        .attr("role", "status")
        .text(message)
        .into()
}

#[cfg(test)]
mod view_tests;
