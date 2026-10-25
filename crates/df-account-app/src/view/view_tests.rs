//! Rules every screen obeys, checked on the screen each route opens to (its loading state, or its
//! form). The per-screen files check the populated states.

use std::collections::HashSet;

use super::view;
use crate::{
    node::{Element, Node},
    route::{AdminRoute, Route},
    state::{Model, Msg, Screen},
    wire::{AccountView, DevSignIn, OperatorCapability, OperatorView, SessionView},
};

fn session(is_signed_in: bool, is_operator: bool) -> SessionView {
    SessionView {
        csrf: "csrf".to_owned(),
        account: is_signed_in.then(|| AccountView {
            email: "ana@example.test".to_owned(),
            display_name: Some("Ana".to_owned()),
            is_verified: true,
            operator: is_operator.then(|| OperatorView {
                capabilities: vec![
                    OperatorCapability::SupportInspect,
                    OperatorCapability::Refund,
                    OperatorCapability::ContentReview,
                ],
                has_second_factor: true,
            }),
        }),
        dev_sign_in: None,
    }
}

fn opened(route: Route, is_signed_in: bool, is_operator: bool) -> Model {
    let mut model = Model::default();
    model.update(Msg::Started {
        route,
        offset_east_minutes: 0,
    });
    let generation = model.generation;
    model.update(Msg::Replied {
        generation,
        reply: crate::api::Reply::Session(Ok(session(is_signed_in, is_operator))),
    });
    model
}

fn customer_routes() -> Vec<Route> {
    vec![
        Route::Dashboard,
        Route::AddCredit,
        Route::CreditReturn {
            checkout: Some("0a".to_owned()),
        },
        Route::Funding,
        Route::Statements,
        Route::Settings,
        Route::NewCampaign,
        Route::Campaign {
            campaign_id: "0c".to_owned(),
        },
        Route::Lobby {
            session_id: "5e".to_owned(),
        },
    ]
}

fn public_routes() -> Vec<Route> {
    vec![
        Route::Landing,
        Route::SignUp,
        Route::SignIn,
        Route::VerifyEmail { token: None },
        Route::VerifyEmail {
            token: Some("0a".to_owned()),
        },
        Route::ForgotPassword,
        Route::ResetPassword { token: None },
        Route::ResetPassword {
            token: Some("0a".to_owned()),
        },
        Route::SignedOut,
        Route::Join { token: None },
        Route::Join {
            token: Some("0a".to_owned()),
        },
        Route::Display,
        Route::NotFound,
    ]
}

fn admin_routes() -> Vec<Route> {
    [
        AdminRoute::Summary,
        AdminRoute::Accounts,
        AdminRoute::Account {
            account_id: "00ff".to_owned(),
        },
        AdminRoute::Disputes,
        AdminRoute::Reconciliation,
        AdminRoute::Providers,
        AdminRoute::Review,
        AdminRoute::Audit,
    ]
    .into_iter()
    .map(Route::Admin)
    .collect()
}

/// Every screen a person can open: (route, who is signed in, operator).
fn every_screen() -> Vec<(Route, Model)> {
    let mut screens = Vec::new();
    for route in public_routes() {
        screens.push((route.clone(), opened(route, false, false)));
    }
    for route in customer_routes() {
        screens.push((route.clone(), opened(route, true, false)));
    }
    for route in admin_routes() {
        screens.push((route.clone(), opened(route, true, true)));
    }
    screens
}

fn elements(tree: &Node) -> Vec<&Element> {
    tree.elements()
}

fn names(tree: &Node, tag: &str) -> usize {
    elements(tree)
        .into_iter()
        .filter(|element| element.tag == tag)
        .count()
}

#[test]
fn every_screen_has_exactly_one_focusable_main_heading() {
    for (route, model) in every_screen() {
        let tree = view(&model);

        assert_eq!(names(&tree, "h1"), 1, "{route:?}");
        let heading = elements(&tree)
            .into_iter()
            .find(|element| element.tag == "h1")
            .expect("an h1");
        assert_eq!(heading.attribute("tabindex"), Some("-1"), "{route:?}");
        assert_eq!(
            heading.attribute("data-df-focus"),
            Some("heading"),
            "{route:?}"
        );
    }
}

#[test]
fn every_screen_has_a_skip_link_a_main_landmark_and_a_labelled_navigation() {
    for (route, model) in every_screen() {
        let tree = view(&model);

        assert_eq!(names(&tree, "main"), 1, "{route:?}");
        assert!(
            elements(&tree)
                .iter()
                .any(|element| element.attribute("href") == Some("#main")),
            "{route:?}"
        );
        for nav in elements(&tree)
            .into_iter()
            .filter(|element| element.tag == "nav")
        {
            assert!(nav.attribute("aria-label").is_some(), "{route:?}");
        }
    }
}

#[test]
fn ids_are_unique_and_every_reference_to_an_id_points_at_one() {
    for (route, model) in every_screen() {
        let tree = view(&model);
        let mut ids = HashSet::new();
        for element in elements(&tree) {
            if let Some(id) = element.attribute("id") {
                assert!(ids.insert(id.to_owned()), "{route:?}: duplicate id {id}");
            }
        }
        for element in elements(&tree) {
            for attribute in ["for", "aria-labelledby"] {
                for target in element
                    .attribute(attribute)
                    .into_iter()
                    .flat_map(str::split_whitespace)
                {
                    assert!(ids.contains(target), "{route:?}: {attribute}={target}");
                }
            }
            for target in element
                .attribute("aria-describedby")
                .into_iter()
                .flat_map(str::split_whitespace)
            {
                assert!(ids.contains(target), "{route:?}: aria-describedby={target}");
            }
        }
    }
}

#[test]
fn every_input_has_a_label_and_every_button_has_text_and_a_type() {
    for (route, model) in every_screen() {
        let tree = view(&model);
        let labelled: HashSet<&str> = elements(&tree)
            .into_iter()
            .filter(|element| element.tag == "label")
            .filter_map(|label| label.attribute("for"))
            .collect();
        for element in elements(&tree) {
            match element.tag {
                "input" | "select" | "textarea" => {
                    let id = element.attribute("id").unwrap_or_default();
                    assert!(labelled.contains(id), "{route:?}: input {id} has no label");
                }
                "button" => {
                    assert!(
                        matches!(element.attribute("type"), Some("button" | "submit")),
                        "{route:?}"
                    );
                    assert!(
                        !Node::Element(element.clone()).text_content().is_empty(),
                        "{route:?}: a button without text"
                    );
                }
                "a" => assert!(element.attribute("href").is_some(), "{route:?}"),
                "img" => assert!(element.attribute("alt").is_some(), "{route:?}"),
                _ => {}
            }
        }
    }
}

#[test]
fn the_sign_in_screen_hides_development_choices_when_the_server_omits_them() {
    let model = opened(Route::SignIn, false, false);
    let Screen::SignIn(form) = &model.screen else {
        panic!("sign-in screen remains open");
    };

    assert!(form.email.is_empty());
    assert!(form.password.is_empty());
    assert!(
        !view(&model)
            .text_content()
            .contains("Development: sign in as")
    );
}

#[test]
fn the_sign_in_screen_prefills_and_offers_only_server_supplied_development_accounts() {
    let mut session = session(false, false);
    session.dev_sign_in = Some(vec![
        DevSignIn {
            label: "Customer".to_owned(),
            email: "dev@example.test".to_owned(),
            password: "development password".to_owned(),
        },
        DevSignIn {
            label: "Admin".to_owned(),
            email: "dev-admin@example.test".to_owned(),
            password: "development password".to_owned(),
        },
    ]);
    let mut model = Model::default();
    model.update(Msg::Started {
        route: Route::SignIn,
        offset_east_minutes: 0,
    });
    let generation = model.generation;
    model.update(Msg::Replied {
        generation,
        reply: crate::api::Reply::Session(Ok(session)),
    });
    let Screen::SignIn(form) = &model.screen else {
        panic!("sign-in screen remains open");
    };

    assert_eq!(form.email, "dev@example.test");
    assert_eq!(form.password, "development password");
    assert!(
        view(&model)
            .text_content()
            .contains("Development: sign in as Admin")
    );
}

#[test]
fn customer_and_public_screens_never_say_fee_multiplier_markup_or_provider() {
    let forbidden = ["table fee", "multiplier", "markup", "provider", "margin"];
    for (route, model) in every_screen()
        .into_iter()
        .filter(|(route, _)| !matches!(route, Route::Admin(_)))
    {
        let text = view(&model).text_content().to_lowercase();
        for word in forbidden {
            assert!(!text.contains(word), "{route:?} says {word}");
        }
    }
}

#[test]
fn no_attribute_or_text_of_a_join_page_holds_the_invitation_token() {
    let model = opened(
        Route::Join {
            token: Some("deadbeef0123".to_owned()),
        },
        false,
        false,
    );
    let tree = view(&model);

    assert!(!tree.text_content().contains("deadbeef0123"));
    for element in elements(&tree) {
        for (_, value) in &element.attributes {
            assert!(!value.contains("deadbeef0123"));
        }
    }
}

#[test]
fn only_operators_see_the_console_link() {
    let customer = view(&opened(Route::Dashboard, true, false)).text_content();
    let operator = view(&opened(Route::Dashboard, true, true)).text_content();

    assert!(!customer.contains("Operator console"));
    assert!(operator.contains("Operator console"));
}

#[test]
fn the_bar_offers_sign_in_and_sign_up_to_visitors_and_sign_out_to_members() {
    let visitor = view(&opened(Route::Landing, false, false)).text_content();
    let member = view(&opened(Route::Dashboard, true, false)).text_content();

    assert!(visitor.contains("Sign in") && visitor.contains("Create a free account"));
    assert!(member.contains("Sign out") && !member.contains("Create a free account"));
}

#[test]
fn the_current_section_is_marked_in_the_navigation() {
    let tree = view(&opened(Route::Statements, true, false));

    let current: Vec<_> = elements(&tree)
        .into_iter()
        .filter(|element| element.attribute("aria-current") == Some("page"))
        .map(|element| Node::Element(element.clone()).text_content())
        .collect();

    assert_eq!(current, vec!["Statements".to_owned()]);
}
