//! The Roll panel's words: the button, the advantage pill and the sum.

use crate::fixture::{RollMode, RollView};
use crate::phone_kit::roll::{RollPanel, RollState, kept_text, sum_text};

const TWO_DICE: &str =
    r#"[{"sides":20,"face":14,"role":"test"},{"sides":20,"face":6,"role":"test","kept":false}]"#;

fn view(dice: &str, modifiers: &str, total: i32) -> RollView {
    serde_json::from_str(&format!(
        r#"{{"id":"t","title":"Perception","kind":"skill_check","dice":{dice},
        "modifiers":{modifiers},"total":{total}}}"#
    ))
    .expect("the roll reads")
}

fn panel(mode: RollMode, state: RollState) -> RollPanel {
    RollPanel {
        check: "Perception check".to_owned(),
        basis: None,
        stakes: "Spot the watcher".to_owned(),
        mode,
        mode_reason: None,
        state,
    }
}

#[test]
fn the_button_names_the_mode_and_only_a_ready_roll_can_be_pressed() {
    assert_eq!(
        panel(RollMode::Normal, RollState::Ready).button_label(),
        "Roll"
    );
    assert_eq!(
        panel(RollMode::Advantage, RollState::Ready).button_label(),
        "Roll with advantage"
    );
    assert_eq!(
        panel(RollMode::Disadvantage, RollState::Ready).button_label(),
        "Roll with disadvantage"
    );
    assert!(panel(RollMode::Normal, RollState::Ready).can_roll());
    assert!(!panel(RollMode::Normal, RollState::Rolling).can_roll());
    let done = panel(
        RollMode::Normal,
        RollState::Done(view(r#"[{"sides":20,"face":10,"role":"test"}]"#, "[]", 10)),
    );
    assert!(!done.can_roll());
    assert_eq!(done.button_label(), "Rolled");
}

#[test]
fn a_normal_roll_has_no_mode_pill() {
    assert_eq!(panel(RollMode::Normal, RollState::Ready).mode_pill(), None);
    assert_eq!(
        panel(RollMode::Advantage, RollState::Ready)
            .mode_pill()
            .as_deref(),
        Some("Advantage: roll two, keep the higher")
    );
}

#[test]
fn the_sum_is_the_kept_dice_and_the_modifiers() {
    let rolled = view(
        TWO_DICE,
        r#"[{"label":"Dexterity","value":4},{"label":"Proficiency","value":2}]"#,
        20,
    );
    assert_eq!(sum_text(&rolled), "14 + 4 + 2 = 20");
    let negative = view(
        r#"[{"sides":20,"face":9,"role":"test"}]"#,
        r#"[{"label":"Cursed","value":-1}]"#,
        8,
    );
    assert_eq!(sum_text(&negative), "9 - 1 = 8");
}

#[test]
fn two_dice_say_which_one_counted_and_one_die_says_nothing() {
    let two = view(TWO_DICE, "[]", 14);
    assert_eq!(kept_text(&two).as_deref(), Some("Rolled 14 and 6, kept 14"));
    let one = view(r#"[{"sides":20,"face":14,"role":"test"}]"#, "[]", 14);
    assert_eq!(kept_text(&one), None);
}
