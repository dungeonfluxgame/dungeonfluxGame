//! The kit's shared pieces: text cleaning, the steps of a run, and the closed keys.

use std::fmt::Debug;

use serde::Serialize;

use crate::fixtures::assert_keys_are_serde_names;
use crate::kit::{KitPanels, PanelTone, Rarity, Step, current_index, shown, steps};
use crate::overlay::Progress;

/// Asserts that each of `values` serializes to its CSS word `key`, through the shared key check
/// (the kit's words are written into classes only, never read back).
pub(crate) fn assert_css_words_are_serde_names<T: Serialize + Copy + PartialEq + Debug>(
    values: &[T],
    key: impl Fn(T) -> &'static str,
) {
    assert_keys_are_serde_names(values, &key, |word| {
        values.iter().copied().find(|value| key(*value) == word)
    });
}

fn progress(index: u16, total: u16) -> Progress {
    Progress {
        index,
        total,
        title: None,
    }
}

#[test]
fn shown_trims_and_drops_blank_text() {
    assert_eq!(shown(Some("  Kael ")), Some("Kael"));
    assert_eq!(shown(Some("   ")), None);
    assert_eq!(shown(None), None);
}

#[test]
fn steps_mark_past_current_and_future_and_hold_the_index_inside_the_run() {
    assert_eq!(
        steps(&progress(2, 4)),
        vec![Step::Past, Step::Current, Step::Future, Step::Future]
    );
    assert_eq!(current_index(&progress(0, 3)), 1);
    assert_eq!(current_index(&progress(9, 3)), 3);
    assert_eq!(steps(&progress(9, 3)).last(), Some(&Step::Current));
}

#[test]
fn a_run_of_fewer_than_two_scenes_has_no_steps() {
    assert!(steps(&progress(1, 1)).is_empty());
    assert!(steps(&progress(1, 0)).is_empty());
}

#[test]
fn the_tone_and_rarity_keys_are_their_serde_names() {
    assert_css_words_are_serde_names(&[PanelTone::Gold, PanelTone::Danger], PanelTone::key);
    assert_css_words_are_serde_names(
        &[
            Rarity::Common,
            Rarity::Uncommon,
            Rarity::Rare,
            Rarity::VeryRare,
            Rarity::Legendary,
        ],
        Rarity::key,
    );
    assert_eq!(Rarity::VeryRare.word(), "Very Rare");
}

#[test]
fn the_section_label_stands_under_a_drawn_journal_in_its_column() {
    let panels = |json: &str| -> KitPanels { serde_json::from_str(json).expect("panels read") };
    let section = r#""section":{"title":"Journal / Quests"}"#;
    assert!(
        panels(&format!(
            r#"{{"journal":{{"entries":[{{"id":"a","title":"The Missing Caravan"}}]}},{section}}}"#
        ))
        .section_under_journal(),
        "a drawn book and its label share one column, so the book never grows over the label"
    );
    assert!(
        !panels(&format!(r#"{{"journal":{{"entries":[]}},{section}}}"#)).section_under_journal(),
        "an empty journal is not drawn, so the label stays in the bottom stack"
    );
    assert!(
        !panels(r#"{"journal":{"entries":[{"id":"a","title":"Rats"}]}}"#).section_under_journal()
    );
}
