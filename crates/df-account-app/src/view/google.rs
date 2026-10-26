//! Native same-origin forms let the server redirect the top-level browser to Google.

use crate::{
    node::{Element, Node, el},
    state::Model,
};

pub(super) fn enabled(model: &Model) -> bool {
    model
        .session
        .ready()
        .is_some_and(|session| session.google_enabled)
}

pub(super) fn hidden(name: &'static str, value: &str) -> Node {
    el("input")
        .attr("type", "hidden")
        .attr("name", name)
        .attr("value", value)
        .into()
}

pub(super) fn form(model: &Model, intent: &str) -> Element {
    el("form")
        .class("form google-form")
        .attr("method", "post")
        .attr("action", "/app/api/oauth/google/start")
        .attr("data-df-oauth", "google")
        .attr("aria-label", "Continue with Google")
        .child(hidden("csrf", model.csrf()))
        .child(hidden("intent", intent))
}

pub(super) fn button(disabled: bool) -> Node {
    el("button")
        .class("google-button")
        .attr("type", "submit")
        .flag("disabled", disabled)
        .child(
            el("img")
                .attr("src", "/app/assets/google-g.svg")
                .attr("alt", "")
                .attr("width", "18")
                .attr("height", "18")
                .attr("aria-hidden", "true"),
        )
        .child(el("span").text("Continue with Google"))
        .into()
}

pub(super) fn password(id: &str) -> Node {
    el("label")
        .class("field")
        .attr("for", id)
        .text("Your current password")
        .child(
            el("input")
                .attr("id", id)
                .attr("name", "current_password")
                .attr("type", "password")
                .attr("autocomplete", "current-password")
                .attr("required", ""),
        )
        .into()
}

#[cfg(test)]
mod tests {
    use crate::{
        api::Reply,
        route::Route,
        state::{Field, Model, Msg, Notice, Screen},
        wire::{AccountView, SessionView},
    };

    fn opened(route: Route, enabled: bool, signed_in: bool) -> Model {
        let mut model = Model::default();
        model.update(Msg::Started {
            route,
            offset_east_minutes: 0,
        });
        model.update(Msg::Replied {
            generation: model.generation,
            reply: Reply::Session(Ok(SessionView {
                google_enabled: enabled,
                csrf: "test-csrf".to_owned(),
                dev_sign_in: None,
                account: signed_in.then(|| AccountView {
                    email: "google@example.test".to_owned(),
                    display_name: None,
                    is_verified: true,
                    operator: None,
                }),
            })),
        });
        model
    }

    #[test]
    fn google_buttons_are_native_forms_and_hidden_when_server_is_not_configured() {
        for route in [Route::SignIn, Route::SignUp] {
            let enabled = opened(route.clone(), true, false);
            let node = crate::view::view(&enabled);
            let forms: Vec<_> = node
                .elements()
                .into_iter()
                .filter(|element| element.attribute("data-df-oauth") == Some("google"))
                .collect();
            assert_eq!(forms.len(), 1);
            assert_eq!(forms[0].attribute("method"), Some("post"));
            assert_eq!(
                forms[0].attribute("action"),
                Some("/app/api/oauth/google/start")
            );
            assert!(forms[0].on_submit.is_none());
            assert!(node.text_content().contains("Continue with Google"));
            assert!(
                !crate::view::view(&opened(route, false, false))
                    .text_content()
                    .contains("Continue with Google")
            );
        }
    }

    #[test]
    fn google_signup_reuses_both_checkboxes_and_sends_explicit_boolean_consent() {
        let mut model = opened(Route::SignUp, true, false);
        let disabled = |node: &crate::node::Node| {
            node.elements()
                .into_iter()
                .find(|element| element.attribute("class") == Some("google-button"))
                .unwrap()
                .attribute("disabled")
                .is_some()
        };
        assert!(disabled(&crate::view::view(&model)));
        model.update(Msg::Toggle(Field::IsAdult, true));
        model.update(Msg::Toggle(Field::AcceptsTerms, true));
        let node = crate::view::view(&model);
        assert!(!disabled(&node));
        for name in ["is_adult", "accepts_terms"] {
            assert_eq!(
                node.elements()
                    .into_iter()
                    .find(|element| element.attribute("name") == Some(name))
                    .unwrap()
                    .attribute("value"),
                Some("true")
            );
        }
        assert_eq!(
            node.elements()
                .into_iter()
                .filter(|element| element.tag == "input"
                    && element.attribute("type") == Some("checkbox"))
                .count(),
            2
        );
    }

    #[test]
    fn google_signup_welcome_reuses_verified_dashboard_notice_and_cleans_path() {
        let route = Route::parse("/app/dashboard", "welcome=1", "");
        assert_eq!(route, Route::GoogleWelcome);
        assert_eq!(route.path(), "/app/dashboard");
        let model = opened(route, true, true);
        assert_eq!(model.notice, Some(Notice::EmailVerified));
        assert!(matches!(model.screen, Screen::Dashboard(_)));
        assert!(
            crate::view::view(&model)
                .text_content()
                .contains("Your $10 of free play is ready.")
        );
    }
}
