//! The decisions the page makes when the host's answers arrive.

use crate::decisions::{controls_effect, fixture_load_needed, is_animating, scene_changed};

#[test]
fn another_scenario_the_first_answer_and_a_new_run_of_the_host_read_the_scenario() {
    assert!(controls_effect(Some("a.json"), Some("b.json"), false, false).fixture_changed);
    assert!(controls_effect(None, Some("a.json"), true, false).fixture_changed);
    assert!(controls_effect(None, None, true, false).fixture_changed);
    assert!(
        controls_effect(Some("a.json"), Some("a.json"), false, true).fixture_changed,
        "the same name may be another file after a restart"
    );
    assert!(controls_effect(Some("a.json"), None, false, false).fixture_changed);
}

#[test]
fn the_same_scenario_in_the_same_run_is_not_read_again() {
    assert!(!controls_effect(Some("a.json"), Some("a.json"), false, false).fixture_changed);
    assert!(!controls_effect(None, None, false, false).fixture_changed);
}

#[test]
fn a_scenario_that_is_chosen_and_not_loaded_is_read_and_one_that_is_loaded_is_not() {
    assert!(fixture_load_needed(true, true, true));
    assert!(
        fixture_load_needed(false, false, true),
        "a failed read cleared it"
    );
    assert!(!fixture_load_needed(false, true, true));
    assert!(!fixture_load_needed(false, false, false), "none is chosen");
}

#[test]
fn only_a_different_scenario_after_the_first_is_a_change_of_scene() {
    assert!(scene_changed(Some("a.json"), "b.json"));
    assert!(!scene_changed(None, "a.json"));
    assert!(!scene_changed(Some("a.json"), "a.json"));
}

#[test]
fn an_element_is_animating_when_its_style_says_so() {
    assert!(is_animating(
        "left: 5%; animation: card-slide 5000ms linear forwards;"
    ));
    assert!(is_animating("transform: x; animation-delay: -5ms;"));
    assert!(!is_animating("left: 85%;"));
    assert!(!is_animating(
        "transform: translate(0%, 0%) scale(1); transition: transform 90ms linear;"
    ));
}
