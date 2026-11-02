//! What the page remembers between draws: slides, camera moves and host runs (O.13 and O.14
//! acceptance for continuity).

use crate::camera::Framing;
use crate::continuity::{Admit, CameraClock, Continuity, ReplyGate, SlideClock, canvas_accepts};
use crate::controls::{Controls, ControlsReply};
use crate::fixture::{Camera, ElementShape, StageElement, StagePivot};
use crate::limits::{MAXIMUM_REMEMBERED_SLIDES, MAXIMUM_ZOOM_PERCENT};
use crate::scene::{ImageSlot, Override, SceneChange, SceneOverlay};

fn element(id: &str, x: f32, y: f32) -> StageElement {
    StageElement {
        id: id.to_owned(),
        kind: "character".to_owned(),
        label: id.to_owned(),
        x,
        y,
        color: None,
        shape: ElementShape::Circle,
        width_units: 5.0,
        height_units: 5.0,
        fields: serde_json::Map::new(),
        rotation_degrees: 0,
        pivot: StagePivot::default(),
    }
}

fn camera(target: Option<&str>, serial: u64, duration_ms: u32, elapsed_ms: u64) -> Camera {
    Camera {
        target: target.map(str::to_owned),
        zoom_percent: 200,
        duration_ms,
        serial,
        began_ms: 0,
        elapsed_ms,
    }
}

fn slide(serial: u64, duration_ms: u32, elapsed_ms: u64) -> crate::fixture::Slide {
    crate::fixture::Slide {
        from_percent: 15,
        to_percent: 85,
        duration_ms,
        serial,
        began_ms: 0,
        elapsed_ms,
    }
}

fn finn() -> Vec<StageElement> {
    vec![element("finn", 50.0, 28.125)]
}

#[test]
fn an_answer_is_taken_when_newer_and_never_when_older_in_one_run() {
    let mut gate = ReplyGate::default();
    assert_eq!(
        gate.admit("b1", 5),
        Admit::Take { new_run: false },
        "the first answer"
    );
    assert_eq!(gate.admit("b1", 6), Admit::Take { new_run: false });
    assert_eq!(
        gate.admit("b1", 6),
        Admit::Refuse { older_run: false },
        "the same answer again"
    );
    assert_eq!(
        gate.admit("b1", 5),
        Admit::Refuse { older_run: false },
        "a slow answer that was overtaken"
    );
    assert_eq!(gate.boot_id(), Some("b1"));
}

#[test]
fn a_newer_run_of_the_host_is_taken_from_the_start_and_a_late_answer_of_the_older_one_is_not() {
    let mut gate = ReplyGate::default();
    gate.admit("0001", 6);
    assert_eq!(
        gate.admit("0002", 1),
        Admit::Take { new_run: true },
        "a restarted host counts from the start"
    );
    // B then a late A: the old run's slow poll resolves after the new run's frame was taken.
    assert_eq!(gate.admit("0001", 7), Admit::Refuse { older_run: true });
    assert_eq!(
        gate.admit("0002", 2),
        Admit::Take { new_run: false },
        "the new run goes on"
    );
    assert_eq!(gate.boot_id(), Some("0002"));
}

#[test]
fn a_run_that_keeps_answering_with_an_older_id_is_the_real_one_after_three_in_a_row() {
    let mut gate = ReplyGate::default();
    gate.admit("0009", 4);
    assert_eq!(gate.admit("0003", 1), Admit::Refuse { older_run: true });
    assert_eq!(gate.admit("0003", 2), Admit::Refuse { older_run: true });
    assert_eq!(
        gate.admit("0003", 3),
        Admit::Take { new_run: true },
        "the host restarted with its clock set back"
    );
    assert_eq!(gate.boot_id(), Some("0003"));
    // Strikes of different older runs do not add up, and an answer of the followed run resets them.
    let mut gate = ReplyGate::default();
    gate.admit("0009", 4);
    gate.admit("0001", 1);
    gate.admit("0002", 1);
    assert_eq!(gate.admit("0001", 2), Admit::Refuse { older_run: true });
    gate.admit("0001", 3);
    gate.admit("0009", 5);
    assert_eq!(gate.admit("0001", 9), Admit::Refuse { older_run: true });
}

#[test]
fn a_host_that_does_not_name_its_run_is_one_run() {
    let mut gate = ReplyGate::default();
    assert_eq!(gate.admit("", 1), Admit::Take { new_run: false });
    assert_eq!(gate.admit("", 2), Admit::Take { new_run: false });
    assert_eq!(gate.admit("", 2), Admit::Refuse { older_run: false });
}

#[test]
fn a_canvas_is_taken_only_when_newer_in_its_run_or_from_a_newer_run() {
    assert!(canvas_accepts(None, "a", 3));
    assert!(canvas_accepts(Some(("a", 3)), "a", 4));
    assert!(!canvas_accepts(Some(("a", 4)), "a", 4), "the same canvas");
    assert!(
        !canvas_accepts(Some(("a", 9)), "a", 4),
        "a slow read after a newer frame"
    );
    assert!(
        canvas_accepts(Some(("a", 9)), "b", 1),
        "a newer run starts again"
    );
    assert!(
        !canvas_accepts(Some(("b", 1)), "a", 50),
        "a late canvas of the older run"
    );
}
#[test]
fn the_reply_carries_the_boot_id_and_reads_without_one() {
    let reply = ControlsReply {
        revision: 4,
        boot_id: "run-1".to_owned(),
        controls: Controls::default(),
        scene: SceneOverlay::default(),
    };
    let text = serde_json::to_string(&reply).expect("writes");
    assert!(text.contains("\"boot_id\":\"run-1\""));
    let read: ControlsReply = serde_json::from_str(&text).expect("reads");
    assert_eq!(read, reply);
    let old: ControlsReply = serde_json::from_str(&text.replace("\"boot_id\":\"run-1\",", ""))
        .expect("a host that does not say reads as an empty boot id");
    assert_eq!(old.boot_id, "");
}

#[test]
fn a_reply_says_how_long_ago_each_move_began_by_the_hosts_clock() {
    let base = crate::fixture::Fixture::parse(
        r#"{"name":"B","tv":{"place":"P","status":{"kind":"ok"},"elements":[]},"phone":{"hero":"H","status":{"kind":"ok"}}}"#,
    )
    .expect("reads");
    let mut overlay = SceneOverlay::default();
    overlay
        .apply_at(
            SceneChange::Camera {
                target: Some("finn".to_owned()),
                zoom_percent: 250,
                duration_ms: 4_000,
            },
            &base,
            10_000,
        )
        .expect("applies");
    overlay
        .apply_at(
            SceneChange::Image {
                slot: ImageSlot::TvCard,
                media: "a.png".to_owned(),
                caption: None,
                rotation_degrees: 0,
                slide: Some(slide(0, 5_000, 0)),
            },
            &base,
            10_500,
        )
        .expect("applies");
    let reply = ControlsReply {
        revision: 2,
        boot_id: "run".to_owned(),
        controls: Controls::default(),
        scene: overlay.clone(),
    };
    let later = reply.at(13_000);
    assert_eq!(
        later.scene.camera.as_ref().map(|camera| camera.elapsed_ms),
        Some(3_000)
    );
    let Some(Override::Set(card)) = later.scene.card else {
        panic!("a card");
    };
    assert_eq!(card.slide.map(|slide| slide.elapsed_ms), Some(2_500));
    assert_eq!(
        reply.scene.camera.as_ref().map(|camera| camera.elapsed_ms),
        Some(0),
        "what the host keeps is not changed by being read"
    );
    assert_eq!(
        reply.at(5_000).scene.camera.map(|camera| camera.elapsed_ms),
        Some(0),
        "never negative"
    );
}

mod slides {
    use super::*;

    #[test]
    fn a_redraw_in_the_middle_of_a_slide_carries_on_and_does_not_start_again() {
        let mut clock = SlideClock::default();
        let slide = slide(3, 5_000, 0);
        assert_eq!(clock.elapsed(1_000.0, &slide), 0.0);
        assert_eq!(clock.elapsed(2_500.0, &slide), 1_500.0, "carries on");
        assert_eq!(
            clock.elapsed(6_500.0, &slide),
            5_500.0,
            "past its end it is finished"
        );
    }

    #[test]
    fn a_new_command_is_a_new_slide_that_starts_from_its_beginning() {
        let mut clock = SlideClock::default();
        assert_eq!(clock.elapsed(0.0, &slide(1, 5_000, 0)), 0.0);
        assert_eq!(clock.elapsed(3_000.0, &slide(1, 5_000, 0)), 3_000.0);
        assert_eq!(
            clock.elapsed(3_000.0, &slide(2, 5_000, 0)),
            0.0,
            "the next serial"
        );
    }

    #[test]
    fn a_page_that_opens_mid_slide_continues_from_what_the_host_says() {
        let mut clock = SlideClock::default();
        assert_eq!(clock.elapsed(100.0, &slide(4, 5_000, 3_000)), 3_000.0);
        assert_eq!(
            clock.elapsed(600.0, &slide(4, 5_000, 3_500)),
            3_500.0,
            "afterwards the page's own clock decides, and the host's later reading is ignored"
        );
    }

    #[test]
    fn a_slide_of_a_scenario_file_has_no_number_and_is_told_by_what_it_says() {
        let mut clock = SlideClock::default();
        assert_eq!(clock.elapsed(0.0, &slide(0, 5_000, 0)), 0.0);
        assert_eq!(clock.elapsed(1_000.0, &slide(0, 5_000, 0)), 1_000.0);
        let other = crate::fixture::Slide {
            to_percent: 40,
            ..slide(0, 5_000, 0)
        };
        assert_eq!(clock.elapsed(1_000.0, &other), 0.0);
    }

    #[test]
    fn a_scene_change_forgets_every_slide() {
        let mut clock = SlideClock::default();
        clock.elapsed(0.0, &slide(1, 5_000, 0));
        clock.forget();
        assert!(clock.is_empty());
        assert_eq!(clock.elapsed(9_000.0, &slide(1, 5_000, 0)), 0.0);
    }

    #[test]
    fn a_clock_that_goes_backwards_never_gives_a_negative_elapsed() {
        let mut clock = SlideClock::default();
        clock.elapsed(5_000.0, &slide(1, 5_000, 0));
        assert_eq!(clock.elapsed(4_000.0, &slide(1, 5_000, 0)), 0.0);
    }

    #[test]
    fn the_slides_remembered_are_bounded_and_the_oldest_goes_first() {
        let mut clock = SlideClock::default();
        for serial in 1..=(MAXIMUM_REMEMBERED_SLIDES as u64 + 5) {
            clock.elapsed(serial as f64, &slide(serial, 5_000, 0));
        }
        assert_eq!(clock.len(), MAXIMUM_REMEMBERED_SLIDES);
        assert_eq!(
            clock.elapsed(10_000.0, &slide(1, 5_000, 0)),
            0.0,
            "the oldest was let go"
        );
        assert!(
            clock.elapsed(
                10_000.0,
                &slide(MAXIMUM_REMEMBERED_SLIDES as u64 + 5, 5_000, 0)
            ) > 0.0
        );
    }
}

mod camera_moves {
    use super::*;

    fn plan(
        clock: &mut CameraClock,
        now: f64,
        camera: &Camera,
        elements: &[StageElement],
    ) -> crate::continuity::CameraPlan {
        clock.plan(now, Some(camera), Some(elements))
    }

    #[test]
    fn a_new_move_runs_from_the_whole_stage_to_the_framing() {
        let mut clock = CameraClock::default();
        let elements = finn();
        let first = plan(
            &mut clock,
            1_000.0,
            &camera(Some("finn"), 1, 1_000, 0),
            &elements,
        );
        assert!(first.moving);
        assert_eq!(first.from, Framing::WHOLE_STAGE);
        assert_eq!(first.elapsed_ms, 0.0);
        assert_eq!(first.serial, 1);
        assert_ne!(first.to, Framing::WHOLE_STAGE);
    }

    #[test]
    fn a_redraw_of_the_same_move_keeps_its_start_and_its_clock() {
        let mut clock = CameraClock::default();
        let elements = finn();
        let look = camera(Some("finn"), 1, 1_000, 0);
        let first = plan(&mut clock, 1_000.0, &look, &elements);
        let again = plan(&mut clock, 1_400.0, &look, &elements);
        assert_eq!(again.from, first.from);
        assert_eq!(again.elapsed_ms, 400.0);
        let done = plan(&mut clock, 2_500.0, &look, &elements);
        assert!(!done.moving, "finished");
        assert_eq!(done.to, first.to);
    }

    #[test]
    fn a_move_that_is_interrupted_pans_on_from_where_it_had_got_to_and_does_not_pull_out() {
        let mut clock = CameraClock::default();
        let elements = vec![element("finn", 20.0, 20.0), element("brakka", 80.0, 40.0)];
        let toward_finn = plan(
            &mut clock,
            0.0,
            &camera(Some("finn"), 1, 1_000, 0),
            &elements,
        );
        let half_way = Framing::WHOLE_STAGE.blend(&toward_finn.to, 0.5);
        let toward_brakka = plan(
            &mut clock,
            500.0,
            &camera(Some("brakka"), 2, 1_000, 0),
            &elements,
        );
        assert_eq!(toward_brakka.from, half_way);
        assert_ne!(toward_brakka.from, Framing::WHOLE_STAGE);
        assert_eq!(toward_brakka.elapsed_ms, 0.0);
        assert!(toward_brakka.moving);
    }

    #[test]
    fn a_camera_asking_for_no_time_is_there_at_once() {
        let mut clock = CameraClock::default();
        let instant = plan(&mut clock, 0.0, &camera(Some("finn"), 1, 0, 0), &finn());
        assert!(!instant.moving);
        assert_ne!(instant.to, Framing::WHOLE_STAGE);
    }

    #[test]
    fn a_page_that_opens_mid_move_continues_it_and_does_not_replay_the_zoom() {
        let mut clock = CameraClock::default();
        let late = plan(
            &mut clock,
            10_000.0,
            &camera(Some("finn"), 7, 4_000, 1_000),
            &finn(),
        );
        assert!(late.moving);
        assert_eq!(
            late.elapsed_ms, 1_000.0,
            "a quarter of the way, not the start"
        );
        let over = plan(
            &mut CameraClock::default(),
            10_000.0,
            &camera(Some("finn"), 7, 4_000, 9_000),
            &finn(),
        );
        assert!(
            !over.moving,
            "a move that ended before the page opened is simply there"
        );
    }

    #[test]
    fn a_clock_reading_before_the_start_never_gives_a_negative_elapsed() {
        let mut clock = CameraClock::default();
        let look = camera(Some("finn"), 1, 1_000, 0);
        plan(&mut clock, 5_000.0, &look, &finn());
        let earlier = plan(&mut clock, 4_000.0, &look, &finn());
        assert_eq!(earlier.elapsed_ms, 0.0);
    }

    #[test]
    fn a_camera_with_no_target_or_none_at_all_shows_the_whole_stage_and_forgets_the_move() {
        let mut clock = CameraClock::default();
        plan(&mut clock, 0.0, &camera(Some("finn"), 1, 1_000, 0), &finn());
        let none = clock.plan(10.0, None, Some(&finn()));
        assert_eq!(
            (none.to, none.moving, none.serial),
            (Framing::WHOLE_STAGE, false, 0)
        );
        let again = plan(
            &mut clock,
            20.0,
            &camera(Some("finn"), 1, 1_000, 0),
            &finn(),
        );
        assert_eq!(again.elapsed_ms, 0.0, "serial 1 again is a new move");
    }

    #[test]
    fn while_the_stage_is_not_known_the_whole_stage_shows_and_nothing_is_remembered() {
        let mut clock = CameraClock::default();
        let unknown = clock.plan(0.0, Some(&camera(Some("finn"), 1, 1_000, 0)), None);
        assert_eq!((unknown.to, unknown.moving), (Framing::WHOLE_STAGE, false));
        let known = plan(
            &mut clock,
            500.0,
            &camera(Some("finn"), 1, 1_000, 0),
            &finn(),
        );
        assert!(known.moving && known.elapsed_ms == 0.0);
    }

    #[test]
    fn a_target_that_goes_away_shows_the_whole_stage_and_the_same_frame_always_draws_the_same_picture()
     {
        let mut clock = CameraClock::default();
        let look = camera(Some("finn"), 3, 1_000, 0);
        let zoomed = plan(&mut clock, 0.0, &look, &finn());
        assert_ne!(zoomed.to, Framing::WHOLE_STAGE);
        let gone = plan(&mut clock, 2_000.0, &look, &[]);
        assert_eq!((gone.to, gone.moving), (Framing::WHOLE_STAGE, false));
        let back = plan(&mut clock, 3_000.0, &look, &finn());
        assert_eq!(
            back.to, zoomed.to,
            "the frame names a camera on finn and finn is there, so every page draws the zoom"
        );
    }

    #[test]
    fn two_pages_that_saw_different_frames_before_agree_on_the_frame_they_both_have_now() {
        let look = camera(Some("finn"), 3, 1_000, 0);
        let mut watched = CameraClock::default();
        plan(&mut watched, 0.0, &look, &finn());
        plan(&mut watched, 500.0, &look, &[]);
        let mut late = CameraClock::default();
        // The host stamps every frame with how long ago the move began, so both pages are told 5 s.
        let frame_now = camera(Some("finn"), 3, 1_000, 5_000);
        let now_watched = plan(&mut watched, 5_000.0, &frame_now, &finn());
        let now_late = plan(&mut late, 5_000.0, &frame_now, &finn());
        assert_eq!(
            (now_watched.to, now_watched.moving),
            (now_late.to, now_late.moving)
        );
    }

    #[test]
    fn a_target_that_is_not_on_the_stage_yet_shows_the_whole_stage_and_the_zoom_when_it_arrives() {
        let mut clock = CameraClock::default();
        let look = camera(Some("finn"), 5, 1_000, 0);
        assert_eq!(plan(&mut clock, 0.0, &look, &[]).to, Framing::WHOLE_STAGE);
        assert_ne!(
            plan(&mut clock, 100.0, &look, &finn()).to,
            Framing::WHOLE_STAGE
        );
    }

    #[test]
    fn a_zoom_over_what_the_picture_can_bear_is_drawn_at_the_ceiling() {
        let mut clock = CameraClock::default();
        let wide = Camera {
            zoom_percent: 400,
            ..camera(Some("finn"), 1, 0, 0)
        };
        let ceiling = |percent| {
            let mut clock = CameraClock::default();
            clock
                .plan_within(0.0, Some(&wide), Some(&finn()), percent)
                .to
                .scale
        };
        assert_eq!(ceiling(400), 4.0);
        assert_eq!(ceiling(210), 2.1);
        assert_eq!(ceiling(100), 1.0, "never less than the whole stage");
        assert_eq!(plan(&mut clock, 0.0, &wide, &finn()).to.scale, 4.0);
    }

    #[test]
    fn a_camera_of_a_scenario_file_follows_its_target_whenever_it_is_there() {
        let mut clock = CameraClock::default();
        let look = camera(Some("finn"), 0, 0, 0);
        assert_eq!(plan(&mut clock, 0.0, &look, &[]).to, Framing::WHOLE_STAGE);
        assert_ne!(
            plan(&mut clock, 1.0, &look, &finn()).to,
            Framing::WHOLE_STAGE
        );
    }

    #[test]
    fn a_target_that_travels_is_followed_without_restarting_the_move() {
        let mut clock = CameraClock::default();
        let look = camera(Some("finn"), 1, 10_000, 0);
        let first = plan(&mut clock, 0.0, &look, &[element("finn", 30.0, 28.0)]);
        let later = plan(&mut clock, 100.0, &look, &[element("finn", 40.0, 28.0)]);
        assert_eq!(later.from, first.from);
        assert_eq!(later.elapsed_ms, 100.0);
        assert_ne!(later.to, first.to, "the end follows the target");
        assert!(later.moving);
    }
}

mod host_runs {
    use super::*;

    #[test]
    fn serial_one_from_a_restarted_host_is_a_new_camera_move_and_not_the_old_one() {
        let mut page = Continuity::default();
        assert!(!page.note_boot("run-a"), "the first run is not a change");
        let elements = finn();
        let first = page.camera_plan(
            0.0,
            Some(&camera(Some("finn"), 1, 1_000, 0)),
            Some(&elements),
            MAXIMUM_ZOOM_PERCENT,
        );
        let carried = page.camera_plan(
            900.0,
            Some(&camera(Some("finn"), 1, 1_000, 0)),
            Some(&elements),
            MAXIMUM_ZOOM_PERCENT,
        );
        assert_eq!(carried.elapsed_ms, 900.0, "the same move in the same run");
        assert!(
            page.note_boot("run-b"),
            "another run drops everything remembered"
        );
        let restarted = page.camera_plan(
            950.0,
            Some(&camera(Some("finn"), 1, 1_000, 0)),
            Some(&elements),
            MAXIMUM_ZOOM_PERCENT,
        );
        assert!(restarted.moving, "a pan, not a glide that is already over");
        assert_eq!(restarted.elapsed_ms, 0.0);
        assert_eq!(
            restarted.from,
            Framing::WHOLE_STAGE,
            "not blended from the old run's move"
        );
        assert_eq!(first.serial, restarted.serial);
    }

    #[test]
    fn the_first_slide_after_a_restart_is_not_drawn_already_finished() {
        let mut page = Continuity::default();
        page.note_boot("run-a");
        let slide = slide(1, 5_000, 0);
        page.slide_elapsed(0.0, &slide);
        assert_eq!(
            page.slide_elapsed(6_000.0, &slide),
            6_000.0,
            "finished in the old run"
        );
        page.note_boot("run-b");
        assert_eq!(page.slide_elapsed(6_001.0, &slide), 0.0);
    }

    #[test]
    fn the_same_run_again_changes_nothing() {
        let mut page = Continuity::default();
        page.note_boot("run-a");
        page.slide_elapsed(0.0, &slide(1, 5_000, 0));
        assert!(!page.note_boot("run-a"));
        assert_eq!(page.slide_elapsed(1_000.0, &slide(1, 5_000, 0)), 1_000.0);
    }

    #[test]
    fn a_scene_change_forgets_the_slides_and_the_camera_together() {
        let mut page = Continuity::default();
        let elements = finn();
        page.note_boot("run-a");
        page.slide_elapsed(0.0, &slide(1, 5_000, 0));
        page.camera_plan(
            0.0,
            Some(&camera(Some("finn"), 1, 1_000, 0)),
            Some(&elements),
            MAXIMUM_ZOOM_PERCENT,
        );
        page.forget();
        assert_eq!(page.slide_elapsed(2_000.0, &slide(1, 5_000, 0)), 0.0);
        let plan = page.camera_plan(
            2_000.0,
            Some(&camera(Some("finn"), 1, 1_000, 0)),
            Some(&elements),
            MAXIMUM_ZOOM_PERCENT,
        );
        assert_eq!((plan.elapsed_ms, plan.from), (0.0, Framing::WHOLE_STAGE));
    }
}

#[test]
fn strays_of_an_older_run_are_not_added_up_across_answers_of_the_run_being_followed() {
    let mut gate = ReplyGate::default();
    gate.admit("0009", 4);
    for revision in 1..=6 {
        assert_eq!(
            gate.admit("0003", revision),
            Admit::Refuse { older_run: true },
            "a stray in between"
        );
        assert_eq!(
            gate.admit("0009", 4),
            Admit::Refuse { older_run: false },
            "the followed run speaks again"
        );
    }
    assert_eq!(gate.boot_id(), Some("0009"));
}
