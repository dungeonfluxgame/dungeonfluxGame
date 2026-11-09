//! The plate crossfade's state machine and its length.

use crate::controls::{Transition, TransitionStyle};
use crate::plate_fade::*;

#[test]
fn a_picture_fades_only_after_its_image_has_loaded() {
    let mut fade = PlateFade::default();
    assert_eq!(fade.begin("b.png"), FadeStage::Idle);
    assert_eq!(fade.stage(), FadeStage::Loading);
    assert!(fade.loaded("b.png"));
    assert_eq!(fade.stage(), FadeStage::Fading);
    assert!(!fade.loaded("b.png"), "a load is reported once");
    assert!(fade.finished("b.png"));
    assert_eq!(fade.stage(), FadeStage::Idle);
}

#[test]
fn an_event_of_a_replaced_picture_is_ignored() {
    let mut fade = PlateFade::default();
    fade.begin("b.png");
    assert_eq!(fade.begin("c.png"), FadeStage::Loading);
    assert!(!fade.loaded("b.png"));
    assert!(!fade.finished("b.png"));
    assert!(fade.loaded("c.png"));
    assert_eq!(fade.incoming(), Some("c.png"));
}

#[test]
fn a_change_in_the_middle_of_a_fade_says_the_fade_was_under_way() {
    let mut fade = PlateFade::default();
    fade.begin("b.png");
    fade.loaded("b.png");
    assert_eq!(fade.begin("c.png"), FadeStage::Fading);
    assert_eq!(fade.stage(), FadeStage::Loading);
    assert!(
        !fade.finished("b.png"),
        "the old fade's end changes nothing"
    );
    assert_eq!(fade.stage(), FadeStage::Loading);
}

#[test]
fn a_failed_image_ends_the_wait_and_abandoning_clears_it() {
    let mut fade = PlateFade::default();
    fade.begin("b.png");
    assert!(fade.finished("b.png"));
    fade.begin("c.png");
    assert_eq!(fade.abandon(), FadeStage::Loading);
    assert_eq!(fade.stage(), FadeStage::Idle);
    assert_eq!(fade.abandon(), FadeStage::Idle);
}

#[test]
fn a_cut_or_an_instant_transition_does_not_fade() {
    let transition = |style, duration_ms| Transition { style, duration_ms };
    assert_eq!(
        crossfade_ms(transition(TransitionStyle::Fade, 1500)),
        Some(1500)
    );
    assert_eq!(
        crossfade_ms(transition(TransitionStyle::Wipe, 700)),
        Some(700)
    );
    assert_eq!(crossfade_ms(transition(TransitionStyle::Cut, 700)), None);
    assert_eq!(crossfade_ms(transition(TransitionStyle::Fade, 0)), None);
}
