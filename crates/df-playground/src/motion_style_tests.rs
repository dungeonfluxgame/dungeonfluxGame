//! The styles of a slide and a camera: numbers only, and when they must be written. `theme` checks
//! the same motion in the page's style sheet.

mod theme;

use crate::camera::Framing;
use crate::continuity::CameraPlan;
use crate::fixture::Slide;
use crate::motion_style::{
    CameraAction, CameraDrawn, camera_action, camera_style, frozen_card_style, frozen_world_style,
    resting_camera_style, slide_style,
};

fn slide(duration_ms: u64) -> Slide {
    Slide {
        from_percent: 15,
        to_percent: 85,
        duration_ms,
        serial: 1,
        began_ms: 0,
        elapsed_ms: 0,
    }
}

fn zoomed() -> Framing {
    Framing {
        scale: 2.0,
        x_percent: -50.0,
        y_percent: -20.0,
    }
}

fn moving(serial: u64, to: Framing) -> CameraPlan {
    CameraPlan {
        from: Framing::WHOLE_STAGE,
        to,
        elapsed_ms: 250.0,
        duration_ms: 1_000,
        moving: true,
        serial,
    }
}

fn resting(serial: u64, to: Framing) -> CameraPlan {
    CameraPlan {
        from: to,
        to,
        elapsed_ms: 0.0,
        duration_ms: 0,
        moving: false,
        serial,
    }
}

fn numbers_and_fixed_words_only(style: &str) {
    assert!(
        style.chars().all(|character| matches!(
            character,
            '0'..='9' | 'a'..='z' | '%' | '(' | ')' | ',' | ':' | ';' | '-' | ' ' | '.'
        )),
        "{style}"
    );
}

#[test]
fn a_slide_under_way_starts_part_way_and_a_finished_one_is_only_its_place() {
    let style = slide_style(&slide(5_000), 1_500.0);
    assert_eq!(
        style,
        "left: 85%; --slide-from: 15%; --slide-to: 85%; animation: card-slide 5000ms linear forwards; animation-delay: -1500ms;"
    );
    assert_eq!(slide_style(&slide(5_000), 5_000.0), "left: 85%;");
    assert_eq!(slide_style(&slide(5_000), 9_000.0), "left: 85%;");
}

#[test]
fn a_slide_never_writes_a_negative_elapsed_that_the_browser_would_refuse() {
    let style = slide_style(&slide(5_000), -5.0);
    assert!(style.contains("animation-delay: -0ms;"), "{style}");
    assert!(!style.contains("--5"), "{style}");
}

#[test]
fn a_camera_at_rest_glides_to_its_framing_and_a_move_runs_between_two_framings() {
    assert_eq!(
        resting_camera_style(&zoomed()),
        "transform: translate(-50.000%, -20.000%) scale(2.0000); --cam-scale: 2.0000; transition: transform 90ms linear, --cam-scale 90ms linear;"
    );
    let style = camera_style(&moving(2, zoomed()));
    assert_eq!(
        style,
        "transform: translate(-50.000%, -20.000%) scale(2.0000); --cam-scale: 2.0000; --cam-from: translate(0.000%, 0.000%) scale(1.0000); --cam-to: translate(-50.000%, -20.000%) scale(2.0000); --cam-scale-from: 1.0000; --cam-scale-to: 2.0000; animation: camera-move 1000ms linear forwards; animation-delay: -250ms;"
    );
    assert_eq!(
        camera_style(&resting(2, zoomed())),
        resting_camera_style(&zoomed())
    );
    numbers_and_fixed_words_only(&style);
}

#[test]
fn no_text_of_a_scene_reaches_any_style_made_here() {
    for style in [
        slide_style(&slide(5_000), 10.0),
        camera_style(&moving(1, zoomed())),
        resting_camera_style(&Framing::WHOLE_STAGE),
        frozen_world_style("matrix(1, 0, 0, 1, -40.5, 10)"),
        frozen_card_style("123.5px"),
    ] {
        numbers_and_fixed_words_only(&style);
    }
}

#[test]
fn a_redraw_of_the_same_framing_writes_nothing() {
    let plan = moving(2, zoomed());
    let drawn = CameraDrawn::of(&plan);
    assert_eq!(camera_action(Some(&drawn), &plan), CameraAction::Unchanged);
    let rest = resting(0, Framing::WHOLE_STAGE);
    assert_eq!(
        camera_action(Some(&CameraDrawn::of(&rest)), &rest),
        CameraAction::Unchanged
    );
}

#[test]
fn a_move_that_is_under_way_is_aimed_at_its_new_end_and_not_restarted() {
    let before = CameraDrawn::of(&moving(2, zoomed()));
    let target_moved = Framing {
        x_percent: -45.0,
        ..zoomed()
    };
    assert_eq!(
        camera_action(Some(&before), &moving(2, target_moved)),
        CameraAction::Retarget
    );
}

#[test]
fn a_new_move_a_first_draw_and_a_move_that_finished_write_the_whole_style() {
    let before = CameraDrawn::of(&moving(2, zoomed()));
    assert_eq!(
        camera_action(None, &moving(2, zoomed())),
        CameraAction::Restyle
    );
    assert_eq!(
        camera_action(Some(&before), &moving(3, zoomed())),
        CameraAction::Restyle
    );
    assert_eq!(
        camera_action(Some(&before), &resting(2, zoomed())),
        CameraAction::Restyle
    );
    let rest = CameraDrawn::of(&resting(2, zoomed()));
    assert_eq!(
        camera_action(Some(&rest), &moving(2, zoomed())),
        CameraAction::Restyle
    );
}

#[test]
fn a_leaving_copy_is_frozen_where_the_original_stood_with_no_animation() {
    assert_eq!(
        frozen_world_style("matrix(2, 0, 0, 2, -300.25, -90)"),
        "transform: matrix(2, 0, 0, 2, -300.25, -90); animation: none; transition: none;"
    );
    assert_eq!(
        frozen_card_style("412.5px"),
        "left: 412.5px; animation: none;"
    );
    for style in [
        frozen_world_style("matrix(2, 0, 0, 2, -300.25, -90)"),
        frozen_card_style("412.5px"),
    ] {
        assert!(style.contains("animation: none"), "{style}");
        assert!(!style.contains("animation-delay") && !style.contains("card-slide"));
    }
}

#[test]
fn a_computed_value_that_is_not_plainly_a_number_is_not_trusted_and_the_copy_is_just_stopped() {
    for hostile in [
        "none",
        "",
        "matrix(1,0,0,1,0,0);background:url(x)",
        "url(x)",
        "calc(1px)",
    ] {
        assert_eq!(
            frozen_world_style(hostile),
            "animation: none; transition: none;",
            "{hostile}"
        );
    }
    for hostile in ["auto", "", "12px;color:red", "50%"] {
        assert_eq!(frozen_card_style(hostile), "animation: none;", "{hostile}");
    }
}
