//! The choice panel's drawn choices, their dimming and order.

use crate::choices::ChoiceListView;
use crate::fixture::{Choice, ChoiceGroup, UnavailableChoice};
use crate::kit::choices::{ChoiceLayout, ChoiceLook, ChoicePanel};
use crate::kit::kit_tests::assert_css_words_are_serde_names;

fn panel(json: &str) -> ChoicePanel {
    serde_json::from_str(json).expect("a panel reads")
}

#[test]
fn choices_keep_the_server_order_and_drop_blank_labels() {
    let panel = panel(
        r#"{"prompt":" Which path? ","choices":[
            {"id":"a","label":"Take the pass","look":"lit"},
            {"id":"b","label":" "},
            {"id":"c","label":"Follow the river","description":" "}
        ]}"#,
    );
    assert_eq!(panel.prompt(), Some("Which path?"));
    assert_eq!(panel.layout, ChoiceLayout::Rows);
    let drawn = panel.drawn();
    let ids: Vec<&str> = drawn
        .iter()
        .map(|choice| choice.choice.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "c"]);
    assert_eq!(drawn[1].order, 1);
    assert_eq!(drawn[1].description, None);
    assert!(drawn.iter().all(|choice| !choice.dimmed));
}

#[test]
fn once_a_choice_is_chosen_the_others_dim() {
    let panel = panel(
        r#"{"layout":"cards","choices":[
            {"id":"a","label":"Negotiate","look":"chosen"},
            {"id":"b","label":"Investigate"}
        ]}"#,
    );
    let drawn = panel.drawn();
    assert!(!drawn[0].dimmed);
    assert!(drawn[1].dimmed);
    assert_eq!(drawn[1].choice.look, ChoiceLook::Idle);
}

#[test]
fn a_list_keeps_an_icon_column_when_any_choice_has_an_icon() {
    let mixed = panel(
        r#"{"choices":[
            {"id":"a","label":"Negotiate","icon":"scales"},
            {"id":"b","label":"Investigate"},
            {"id":"c","label":"Rest","icon":"masks"}
        ]}"#,
    );
    assert!(mixed.reserves_icon_column());
    let plain = panel(r#"{"choices":[{"id":"a","label":"Negotiate"},{"id":"b","label":"Wait"}]}"#);
    assert!(!plain.reserves_icon_column());
    let blank_icon_only =
        panel(r#"{"choices":[{"id":"a","label":" ","icon":"scales"},{"id":"b","label":"Wait"}]}"#);
    assert!(
        !blank_icon_only.reserves_icon_column(),
        "a choice that is not drawn reserves nothing"
    );
}

#[test]
fn rows_with_an_icon_start_their_labels_at_one_x_and_plain_rows_centre() {
    let iconed = panel(
        r#"{"choices":[
            {"id":"a","label":"Buy the chain shirt","icon":"coins"},
            {"id":"b","label":"Walk on"}
        ]}"#,
    );
    assert_eq!(
        iconed.class(),
        "kit-choices kit-choices-rows kit-choices-iconed"
    );
    let plain = panel(
        r#"{"layout":"cards","choices":[
            {"id":"a","label":"Negotiate"},
            {"id":"b","label":"Wait"}
        ]}"#,
    );
    assert_eq!(plain.class(), "kit-choices kit-choices-cards");
}

#[test]
fn the_closed_keys_are_their_serde_names() {
    assert_css_words_are_serde_names(
        &[ChoiceLayout::Rows, ChoiceLayout::Cards],
        ChoiceLayout::key,
    );
    assert_css_words_are_serde_names(
        &[
            ChoiceLook::Idle,
            ChoiceLook::Lit,
            ChoiceLook::Chosen,
            ChoiceLook::Unavailable,
        ],
        ChoiceLook::key,
    );
}

#[test]
fn the_projected_list_becomes_the_panel_in_its_order_and_looks() {
    let suggested = [Choice {
        label: "Take the pass".to_owned(),
        explain: Some("The quick way.".to_owned()),
    }];
    let talk = ChoiceGroup {
        name: "Talk".to_owned(),
        choices: vec![Choice {
            label: "Ask the guide".to_owned(),
            explain: None,
        }],
    };
    let unavailable = [UnavailableChoice {
        label: "Wait for nightfall".to_owned(),
        reason: "The gate shuts at dusk.".to_owned(),
    }];
    let list = ChoiceListView {
        suggested: &suggested,
        left_out: 0,
        groups: vec![&talk],
        unavailable: &unavailable,
        warnings: Vec::new(),
    };
    let panel = ChoicePanel::from_list(&list, Some("Which path?"));
    assert_eq!(panel.prompt(), Some("Which path?"));
    assert_eq!(panel.layout, ChoiceLayout::Rows);
    let drawn: Vec<(&str, &str, ChoiceLook, Option<&str>)> = panel
        .drawn()
        .iter()
        .map(|drawn| {
            (
                drawn.choice.id.as_str(),
                drawn.label,
                drawn.choice.look,
                drawn.description,
            )
        })
        .collect();
    assert_eq!(
        drawn,
        vec![
            (
                "suggested-0",
                "Take the pass",
                ChoiceLook::Lit,
                Some("The quick way.")
            ),
            ("Talk-0", "Ask the guide", ChoiceLook::Idle, None),
            (
                "unavailable-0",
                "Wait for nightfall",
                ChoiceLook::Unavailable,
                Some("The gate shuts at dusk.")
            ),
        ]
    );
}
