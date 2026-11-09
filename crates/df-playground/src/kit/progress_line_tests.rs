//! The progress line's words and segments.

use crate::kit::Step;
use crate::kit::progress_line::ProgressLine;
use crate::overlay::Progress;

fn line(index: u16, total: u16, title: Option<&str>) -> ProgressLine {
    ProgressLine {
        progress: Progress {
            index,
            total,
            title: title.map(str::to_owned),
        },
        show_play: true,
        continue_label: None,
    }
}

#[test]
fn the_line_counts_the_scene_and_names_it_apart() {
    let line = line(1, 5, Some(" Introduction "));
    assert!(line.is_drawn());
    assert_eq!(line.count_text(), "Scene 1 of 5");
    assert_eq!(line.title(), Some("Introduction"));
    assert_eq!(line.segments().len(), 5);
    assert_eq!(line.segments()[0], Step::Current);
}

#[test]
fn an_index_outside_the_run_is_held_inside_it() {
    assert_eq!(line(7, 5, None).count_text(), "Scene 5 of 5");
    assert_eq!(line(0, 5, None).count_text(), "Scene 1 of 5");
}

#[test]
fn a_run_of_one_scene_draws_no_line() {
    assert!(!line(1, 1, None).is_drawn());
}

#[test]
fn show_play_defaults_to_off() {
    let line: ProgressLine =
        serde_json::from_str(r#"{"progress":{"index":1,"total":2,"title":null}}"#)
            .expect("a line reads");
    assert!(!line.show_play);
    assert_eq!(line.continue_label(), None);
}

#[test]
fn a_blank_continue_word_draws_no_capsule() {
    let mut line = line(1, 5, None);
    line.continue_label = Some("  ".to_owned());
    assert_eq!(line.continue_label(), None);
    line.continue_label = Some(" Continue ".to_owned());
    assert_eq!(line.continue_label(), Some("Continue"));
}
