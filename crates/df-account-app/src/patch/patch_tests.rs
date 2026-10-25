use super::*;
use crate::{
    node::{el, text},
    route::Route,
};

fn page(children: Vec<Node>) -> Node {
    el("div").children(children).into()
}

fn paragraph(words: &str) -> Node {
    el("p").text(words).into()
}

fn properties_at(ops: &[Op], at: &[usize]) -> Vec<Property> {
    ops.iter()
        .filter_map(|op| match op {
            Op::SetProperty { path, property } if path == at => Some(property.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn an_unchanged_tree_produces_no_operations() {
    let tree = page(vec![
        el("h1").text("Sign in").into(),
        el("section").child(paragraph("Hello")).class("page").into(),
        el("button").text("Go").flag("disabled", false).into(),
    ]);

    assert_eq!(diff(&tree, &tree.clone()), Vec::new());
}

#[test]
fn a_changed_text_node_sets_its_text_at_its_path() {
    let old = page(vec![
        paragraph("one"),
        el("section").child(paragraph("two")).into(),
    ]);
    let new = page(vec![
        paragraph("one"),
        el("section").child(paragraph("three")).into(),
    ]);

    assert_eq!(
        diff(&old, &new),
        vec![Op::SetText {
            path: vec![1, 0, 0],
            text: "three".to_owned()
        }]
    );
}

#[test]
fn an_added_changed_and_removed_attribute_each_produce_one_operation() {
    let old = page(vec![
        el("a")
            .attr("href", "/app/a")
            .attr("class", "old")
            .attr("aria-busy", "true")
            .into(),
    ]);
    let new = page(vec![
        el("a")
            .attr("href", "/app/a")
            .attr("class", "new")
            .attr("role", "link")
            .into(),
    ]);

    assert_eq!(
        diff(&old, &new),
        vec![
            Op::SetAttribute {
                path: vec![0],
                name: "class",
                value: "new".to_owned()
            },
            Op::SetAttribute {
                path: vec![0],
                name: "role",
                value: "link".to_owned()
            },
            Op::RemoveAttribute {
                path: vec![0],
                name: "aria-busy"
            },
        ]
    );
}

#[test]
fn a_repeated_attribute_takes_its_last_value_like_the_dom_would() {
    let old = el("p").attr("class", "a").into();
    let new = el("p").attr("class", "b").attr("class", "a").into();

    assert_eq!(diff(&old, &new), Vec::new());
}

#[test]
fn extra_new_children_are_appended_to_their_parent() {
    let old = page(vec![paragraph("one")]);
    let new = page(vec![paragraph("one"), paragraph("two"), text("three")]);

    assert_eq!(
        diff(&old, &new),
        vec![
            Op::Append {
                parent: vec![],
                node: paragraph("two")
            },
            Op::Append {
                parent: vec![],
                node: text("three")
            },
        ]
    );
}

#[test]
fn children_beyond_the_new_length_are_removed_last_first() {
    let old = page(vec![
        paragraph("a"),
        paragraph("b"),
        paragraph("c"),
        paragraph("d"),
    ]);
    let new = page(vec![paragraph("a"), paragraph("b")]);

    assert_eq!(
        diff(&old, &new),
        vec![Op::Remove { path: vec![3] }, Op::Remove { path: vec![2] }]
    );
}

#[test]
fn a_different_tag_replaces_the_node_and_keeps_its_siblings() {
    let old = page(vec![paragraph("keep"), el("p").text("loading").into()]);
    let replacement: Node = el("div").class("problem").text("failed").into();
    let new = page(vec![paragraph("keep"), replacement.clone()]);

    assert_eq!(
        diff(&old, &new),
        vec![Op::Replace {
            path: vec![1],
            node: replacement
        }]
    );
}

#[test]
fn a_text_node_against_an_element_is_replaced_either_way() {
    let old = page(vec![text("plain")]);
    let new = page(vec![paragraph("wrapped")]);

    assert_eq!(
        diff(&old, &new),
        vec![Op::Replace {
            path: vec![0],
            node: paragraph("wrapped")
        }]
    );
    assert_eq!(
        diff(&new, &old),
        vec![Op::Replace {
            path: vec![0],
            node: text("plain")
        }]
    );
}

#[test]
fn a_different_root_tag_replaces_the_root() {
    let old: Node = el("div").into();
    let new: Node = el("main").into();

    assert_eq!(
        diff(&old, &new),
        vec![Op::Replace {
            path: vec![],
            node: new.clone()
        }]
    );
}

#[test]
fn an_input_with_the_same_tag_is_patched_in_place_so_focus_survives() {
    let old = page(vec![
        el("input")
            .attr("type", "text")
            .attr("class", "a")
            .attr("value", "ab")
            .into(),
    ]);
    let new = page(vec![
        el("input")
            .attr("type", "text")
            .attr("class", "b")
            .attr("value", "abc")
            .into(),
    ]);

    let ops = diff(&old, &new);

    assert!(!ops.iter().any(|op| matches!(op, Op::Replace { .. })));
    assert!(ops.contains(&Op::SetAttribute {
        path: vec![0],
        name: "class",
        value: "b".to_owned()
    }));
}

#[test]
fn a_bound_input_carries_its_value_as_a_property_never_an_attribute() {
    let input = |value: &str| {
        page(vec![
            el("input")
                .attr("type", "text")
                .attr("value", value)
                .bind(Field::Email)
                .into(),
        ])
    };

    let changed = diff(&input("a"), &input("ab"));
    let repeated = diff(&input("ab"), &input("ab"));

    assert_eq!(
        changed,
        vec![Op::SetProperty {
            path: vec![0],
            property: Property::Value("ab".to_owned())
        }]
    );
    assert_eq!(
        repeated, changed,
        "the value is repeated so the runtime can compare it with the live property"
    );
}

#[test]
fn a_cleared_value_attribute_empties_the_input() {
    let old = page(vec![
        el("input").attr("type", "text").attr("value", "x").into(),
    ]);
    let new = page(vec![el("input").attr("type", "text").into()]);

    assert_eq!(
        diff(&old, &new),
        vec![Op::SetProperty {
            path: vec![0],
            property: Property::Value(String::new())
        }]
    );
}

#[test]
fn a_radio_value_is_an_attribute_and_its_checked_state_a_property() {
    let radio = |value: &str, is_checked: bool| {
        page(vec![
            el("input")
                .attr("type", "radio")
                .attr("value", value)
                .flag("checked", is_checked)
                .bind(Field::PacePreset)
                .into(),
        ])
    };

    let ops = diff(&radio("a", false), &radio("b", true));

    assert_eq!(
        ops,
        vec![
            Op::SetAttribute {
                path: vec![0],
                name: "value",
                value: "b".to_owned()
            },
            Op::SetProperty {
                path: vec![0],
                property: Property::Checked(true)
            },
        ]
    );
}

#[test]
fn an_unbound_checkbox_changes_its_checked_property_only_when_the_tree_says_so() {
    let checkbox = |is_checked: bool| {
        page(vec![
            el("input")
                .attr("type", "checkbox")
                .flag("checked", is_checked)
                .into(),
        ])
    };

    assert_eq!(diff(&checkbox(false), &checkbox(false)), Vec::new());
    assert_eq!(
        properties_at(&diff(&checkbox(true), &checkbox(false)), &[0]),
        vec![Property::Checked(false)]
    );
}

#[test]
fn a_bound_checkbox_repeats_its_state_every_render() {
    let checkbox = page(vec![
        el("input")
            .attr("type", "checkbox")
            .flag("checked", true)
            .bind(Field::IsAdult)
            .into(),
    ]);

    assert_eq!(
        properties_at(&diff(&checkbox, &checkbox), &[0]),
        vec![Property::Checked(true)]
    );
}

#[test]
fn a_selected_option_sets_its_property_and_a_deselected_one_clears_it() {
    let select = |chosen: usize| {
        let options = (1..=3).map(|seats| {
            el("option")
                .attr("value", seats.to_string())
                .flag("selected", seats == chosen)
                .text(seats.to_string())
                .into()
        });
        page(vec![
            el("select").bind(Field::SeatCount).children(options).into(),
        ])
    };

    let ops = diff(&select(1), &select(2));

    assert_eq!(
        properties_at(&ops, &[0, 0]),
        vec![Property::Selected(false)]
    );
    assert_eq!(properties_at(&ops, &[0, 1]), vec![Property::Selected(true)]);
    assert!(properties_at(&ops, &[0, 2]).is_empty());
}

#[test]
fn a_select_value_is_set_after_its_options_changed() {
    let select = |values: &[&str]| {
        page(vec![
            el("select")
                .attr("value", "b")
                .children(
                    values
                        .iter()
                        .map(|value| el("option").attr("value", *value).text(*value).into()),
                )
                .into(),
        ])
    };

    let ops = diff(&select(&["a"]), &select(&["a", "b"]));

    let append = ops.iter().position(|op| matches!(op, Op::Append { .. }));
    let value = ops.iter().position(|op| {
        matches!(
            op,
            Op::SetProperty {
                property: Property::Value(_),
                ..
            }
        )
    });
    assert!(append < value);
}

#[test]
fn a_disabled_flag_toggles_the_property_of_a_control_both_ways() {
    let button = |is_busy: bool| {
        page(vec![
            el("button").flag("disabled", is_busy).text("Save").into(),
        ])
    };

    assert_eq!(
        diff(&button(false), &button(true)),
        vec![Op::SetProperty {
            path: vec![0],
            property: Property::Disabled(true)
        }]
    );
    assert_eq!(
        diff(&button(true), &button(false)),
        vec![Op::SetProperty {
            path: vec![0],
            property: Property::Disabled(false)
        }]
    );
}

#[test]
fn disabled_stays_an_attribute_on_elements_that_have_no_such_property() {
    let old = page(vec![el("a").into()]);
    let new = page(vec![el("a").flag("disabled", true).into()]);

    assert_eq!(
        diff(&old, &new),
        vec![Op::SetAttribute {
            path: vec![0],
            name: "disabled",
            value: String::new()
        }]
    );
}

#[test]
fn a_built_element_splits_into_plain_attributes_and_creation_properties() {
    let input = el("input")
        .attr("id", "email")
        .attr("type", "text")
        .attr("value", "a@b.c")
        .flag("disabled", true)
        .attr("id", "mail");

    assert_eq!(
        plain_attributes(&input),
        vec![("id", "mail"), ("type", "text")]
    );
    assert_eq!(
        creation_properties(&input),
        vec![
            Property::Value("a@b.c".to_owned()),
            Property::Disabled(true)
        ]
    );
    let radio = el("input")
        .attr("type", "radio")
        .attr("value", "x")
        .flag("checked", true);
    assert_eq!(
        plain_attributes(&radio),
        vec![("type", "radio"), ("value", "x")]
    );
    assert_eq!(creation_properties(&radio), vec![Property::Checked(true)]);
}

fn form_tree() -> Node {
    page(vec![
        el("form")
            .on_submit(Intent::Retry)
            .child(el("label").child(el("span").text("Email")))
            .child(el("input").bind(Field::Email))
            .child(
                el("a")
                    .attr("href", "/app/sign-in")
                    .on_click(Intent::Go(Route::SignIn))
                    .child(el("strong").text("Sign in")),
            )
            .child(el("button").on_click(Intent::SignOut).text("Out"))
            .into(),
    ])
}

#[test]
fn a_click_on_text_inside_a_link_finds_the_links_intent() {
    let tree = form_tree();

    let hit = click_target(&tree, &[0, 2, 0, 0]);

    let hit = hit.expect("the link carries a click intent");
    assert_eq!(hit.value, Intent::Go(Route::SignIn));
    assert_eq!(hit.element.tag, "a");
    assert_eq!(hit.path, vec![0, 2]);
}

#[test]
fn a_click_with_no_intent_on_the_way_up_finds_nothing() {
    let tree = form_tree();

    assert!(click_target(&tree, &[0, 0, 0]).is_none());
    assert!(click_target(&tree, &[0, 1]).is_none());
}

#[test]
fn the_nearest_intent_wins_over_a_farther_ancestor() {
    let tree: Node = el("div")
        .on_click(Intent::Retry)
        .child(el("button").on_click(Intent::SignOut).text("Out"))
        .child(el("p").text("outside"))
        .into();

    assert_eq!(
        click_target(&tree, &[0, 0]).map(|hit| hit.value),
        Some(Intent::SignOut)
    );
    assert_eq!(
        click_target(&tree, &[1, 0]).map(|hit| hit.value),
        Some(Intent::Retry)
    );
}

#[test]
fn a_submit_event_on_a_field_finds_the_forms_submit_intent() {
    let tree = form_tree();

    let hit = submit_target(&tree, &[0, 1]);

    let hit = hit.expect("the form carries a submit intent");
    assert_eq!(hit.value, Intent::Retry);
    assert_eq!(hit.path, vec![0]);
}

#[test]
fn an_input_event_finds_the_bound_field_and_its_element_path() {
    let tree = form_tree();

    let hit = field_target(&tree, &[0, 1]);

    let hit = hit.expect("the input is bound");
    assert_eq!(hit.value, Field::Email);
    assert_eq!(hit.path, vec![0, 1]);
    assert!(field_target(&tree, &[0, 3]).is_none());
}

#[test]
fn a_path_that_does_not_exist_or_runs_through_text_resolves_to_nothing() {
    let tree = form_tree();

    assert!(click_target(&tree, &[0, 9]).is_none());
    assert!(click_target(&tree, &[0, 0, 0, 0, 0]).is_none());
    assert!(click_target(&tree, &[7]).is_none());
}

#[test]
fn the_root_itself_can_carry_the_intent() {
    let tree: Node = el("div").on_click(Intent::Retry).into();

    assert_eq!(
        click_target(&tree, &[]).map(|hit| hit.value),
        Some(Intent::Retry)
    );
}

#[test]
fn a_control_is_read_by_its_tag_and_type() {
    assert_eq!(control(&el("select")), Control::Select);
    assert_eq!(
        control(&el("input").attr("type", "checkbox")),
        Control::Checkbox
    );
    assert_eq!(control(&el("input").attr("type", "radio")), Control::Radio);
    assert_eq!(
        control(&el("input").attr("type", "password")),
        Control::Text
    );
    assert_eq!(control(&el("textarea")), Control::Text);
}

#[test]
fn the_app_may_leave_for_its_own_paths_and_the_hosted_checkout_only() {
    assert!(is_allowed_destination("/app/dashboard"));
    assert!(is_allowed_destination("/"));
    assert!(is_allowed_destination(
        "https://checkout.stripe.com/c/pay/cs_test_1#frag"
    ));

    assert!(!is_allowed_destination(""));
    assert!(!is_allowed_destination("//evil.example/app"));
    assert!(!is_allowed_destination("/\\evil.example"));
    assert!(!is_allowed_destination("/\t/evil.example"));
    assert!(!is_allowed_destination("/app/a b"));
    assert!(!is_allowed_destination("https://evil.example/"));
    assert!(!is_allowed_destination("http://checkout.stripe.com/c/pay"));
    assert!(!is_allowed_destination(
        "https://checkout.stripe.com.evil.example/"
    ));
    assert!(!is_allowed_destination("https://checkout.stripe.com/"));
    assert!(!is_allowed_destination("javascript:alert(1)"));
    assert!(!is_allowed_destination("app/dashboard"));
}
