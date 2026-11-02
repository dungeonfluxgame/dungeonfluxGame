//! The camera: what it frames, held on the screen, and how a move is blended.

use crate::camera::{Framing, framing, progress};
use crate::fixture::{Camera, ElementShape, StageElement, StagePivot};

fn figure(id: &str, x: f32, y: f32) -> StageElement {
    StageElement {
        id: id.to_owned(),
        kind: "avatar".to_owned(),
        label: id.to_owned(),
        x,
        y,
        color: None,
        shape: ElementShape::Circle,
        width_units: 9.0,
        height_units: 9.0,
        fields: serde_json::Map::new(),
        rotation_degrees: 0,
        pivot: StagePivot::default(),
    }
}

fn camera(target: Option<&str>, zoom_percent: u16) -> Camera {
    Camera {
        target: target.map(str::to_owned),
        zoom_percent,
        duration_ms: 1_000,
        serial: 1,
    }
}

#[test]
fn no_camera_a_missing_target_or_no_zoom_shows_the_whole_stage() {
    let stage = [figure("finn", 50.0, 28.125)];
    assert_eq!(framing(None, &stage), Framing::WHOLE_STAGE);
    assert_eq!(
        framing(Some(&camera(None, 300)), &stage),
        Framing::WHOLE_STAGE
    );
    assert_eq!(
        framing(Some(&camera(Some("nobody"), 300)), &stage),
        Framing::WHOLE_STAGE
    );
    assert_eq!(
        framing(Some(&camera(Some("finn"), 100)), &stage),
        Framing::WHOLE_STAGE
    );
}

#[test]
fn a_target_in_the_middle_of_the_stage_is_kept_in_the_middle_of_the_screen() {
    let stage = [figure("finn", 50.0, 28.125)];
    let view = framing(Some(&camera(Some("finn"), 200)), &stage);
    assert_eq!(
        (view.scale, view.x_percent, view.y_percent),
        (2.0, -50.0, -50.0)
    );
}

#[test]
fn a_target_off_centre_is_centred_when_the_edges_allow() {
    // 60 percent across, 40 percent down at three times: 50 - 180 and 50 - 120.
    let stage = [figure("sigrun", 60.0, 22.5)];
    let view = framing(Some(&camera(Some("sigrun"), 300)), &stage);
    assert!((view.x_percent - -130.0).abs() < 0.001, "{view:?}");
    assert!((view.y_percent - -70.0).abs() < 0.001, "{view:?}");
}

#[test]
fn a_target_near_an_edge_is_held_so_the_picture_still_fills_the_screen() {
    let stage = [figure("brakka", 5.0, 2.0), figure("far", 98.0, 55.0)];
    let near = framing(Some(&camera(Some("brakka"), 200)), &stage);
    assert_eq!((near.x_percent, near.y_percent), (0.0, 0.0));
    let far = framing(Some(&camera(Some("far"), 200)), &stage);
    assert_eq!((far.x_percent, far.y_percent), (-100.0, -100.0));
}

#[test]
fn the_transform_is_numbers_only() {
    let view = Framing {
        scale: 2.5,
        x_percent: -75.0,
        y_percent: -12.5,
    };
    assert_eq!(
        view.transform(),
        "translate(-75.000%, -12.500%) scale(2.5000)"
    );
    assert_eq!(
        Framing::WHOLE_STAGE.transform(),
        "translate(0.000%, 0.000%) scale(1.0000)"
    );
}

#[test]
fn a_blend_moves_each_number_evenly_from_one_framing_to_the_next() {
    let from = Framing::WHOLE_STAGE;
    let to = Framing {
        scale: 3.0,
        x_percent: -100.0,
        y_percent: -60.0,
    };
    assert_eq!(from.blend(&to, 0.0), from);
    assert_eq!(from.blend(&to, 1.0), to);
    let half = from.blend(&to, 0.5);
    assert_eq!(
        (half.scale, half.x_percent, half.y_percent),
        (2.0, -50.0, -30.0)
    );
    assert_eq!(from.blend(&to, 7.0), to, "past the end stays at the end");
    assert_eq!(from.blend(&to, -1.0), from);
}

#[test]
fn progress_runs_from_nothing_to_all_and_a_move_of_no_length_is_done() {
    assert_eq!(progress(0.0, 1_000), 0.0);
    assert_eq!(progress(250.0, 1_000), 0.25);
    assert_eq!(progress(5_000.0, 1_000), 1.0);
    assert_eq!(progress(0.0, 0), 1.0);
    assert_eq!(progress(-10.0, 1_000), 0.0);
}

mod overlay {
    use crate::fixture::{Camera, Fixture, StageElement};
    use crate::scene::{MAXIMUM_CAMERA_MS, SceneChange, SceneError, SceneOverlay};

    const FIXTURE: &str = r#"{
        "name": "Base",
        "tv": {"place": "Here", "status": {"kind": "ok"}, "elements": "live"},
        "phone": {"hero": "Pip", "status": {"kind": "ok"}}
    }"#;

    fn base() -> Fixture {
        Fixture::parse(FIXTURE).expect("the fixture reads")
    }

    fn look(target: Option<&str>, zoom_percent: u16, duration_ms: u32) -> SceneChange {
        SceneChange::Camera {
            target: target.map(str::to_owned),
            zoom_percent,
            duration_ms,
        }
    }

    fn drawn(overlay: &SceneOverlay) -> Option<Camera> {
        let mut fixture = base();
        overlay.apply_to(&mut fixture);
        fixture.tv.camera
    }

    #[test]
    fn a_camera_move_is_drawn_on_the_fixture_with_the_next_number() {
        let mut overlay = SceneOverlay::default();
        overlay
            .apply(look(Some("finn"), 250, 1_500), &base())
            .expect("applies");
        let camera = drawn(&overlay).expect("a camera");
        assert_eq!(
            (
                camera.target.as_deref(),
                camera.zoom_percent,
                camera.duration_ms,
                camera.serial
            ),
            (Some("finn"), 250, 1_500, 1)
        );
        overlay
            .apply(look(Some("sigrun"), 250, 1_500), &base())
            .expect("applies again");
        assert_eq!(drawn(&overlay).expect("a camera").serial, 2);
        assert_eq!(overlay.camera_count, 2);
    }

    #[test]
    fn the_whole_stage_is_a_move_too_so_the_camera_can_pull_back() {
        let mut overlay = SceneOverlay::default();
        overlay
            .apply(look(Some("finn"), 300, 800), &base())
            .expect("applies");
        overlay
            .apply(look(None, 100, 800), &base())
            .expect("applies");
        let camera = drawn(&overlay).expect("a camera");
        assert_eq!(
            (camera.target, camera.zoom_percent, camera.serial),
            (None, 100, 2)
        );
    }

    #[test]
    fn a_zoom_or_time_outside_the_limits_is_refused_and_changes_nothing() {
        for (zoom, ms) in [
            (99, 100),
            (401, 100),
            (0, 100),
            (200, MAXIMUM_CAMERA_MS + 1),
        ] {
            let mut overlay = SceneOverlay::default();
            let refused = overlay.apply(look(Some("finn"), zoom, ms), &base());
            assert!(
                matches!(
                    refused,
                    Err(SceneError::BadZoom(_) | SceneError::BadCameraTime(_))
                ),
                "{zoom} {ms}"
            );
            assert!(overlay.is_empty() && overlay.camera_count == 0);
        }
        assert!(SceneError::BadZoom(500).to_string().contains("100 to 400"));
        assert!(
            SceneError::BadCameraTime(1)
                .to_string()
                .contains("0 to 30000")
        );
    }

    #[test]
    fn the_limits_themselves_are_accepted() {
        let mut overlay = SceneOverlay::default();
        assert!(overlay.apply(look(Some("a"), 100, 0), &base()).is_ok());
        assert!(
            overlay
                .apply(look(Some("a"), 400, MAXIMUM_CAMERA_MS), &base())
                .is_ok()
        );
    }

    #[test]
    fn an_empty_or_over_long_target_is_refused() {
        let mut overlay = SceneOverlay::default();
        assert_eq!(
            overlay.apply(look(Some(" "), 200, 0), &base()),
            Err(SceneError::Empty("a camera target"))
        );
        let long = "x".repeat(81);
        assert!(matches!(
            overlay.apply(look(Some(&long), 200, 0), &base()),
            Err(SceneError::TooLong { .. })
        ));
    }

    #[test]
    fn a_reset_drops_the_camera_but_the_count_only_rises() {
        let mut overlay = SceneOverlay::default();
        overlay
            .apply(look(Some("finn"), 200, 100), &base())
            .expect("applies");
        overlay
            .apply(SceneChange::Reset {}, &base())
            .expect("resets");
        assert!(overlay.is_empty());
        assert_eq!(drawn(&overlay), None);
        assert_eq!(overlay.camera_count, 1);
        overlay
            .apply(look(Some("finn"), 200, 100), &base())
            .expect("applies");
        assert_eq!(drawn(&overlay).expect("a camera").serial, 2);
    }

    #[test]
    fn a_scene_file_may_start_with_a_camera_and_an_older_file_without_one_still_reads() {
        let with = r#"{"target": "finn", "zoom_percent": 300, "duration_ms": 900}"#;
        let camera: Camera = serde_json::from_str(with).expect("reads");
        assert_eq!(camera.serial, 0, "the host sets the number");
        assert!(serde_json::from_str::<Camera>(r#"{"target": "finn"}"#).is_err());
        assert_eq!(base().tv.camera, None);
    }

    #[test]
    fn a_figure_has_a_portrait_only_when_its_portrait_field_names_one() {
        let parse = |fields: &str| -> StageElement {
            serde_json::from_str(&format!(
                r#"{{"id":"a","kind":"avatar","label":"A","x":1,"y":1,"color":null,"shape":"circle","width_units":9,"height_units":9{fields}}}"#
            ))
            .expect("reads")
        };
        assert_eq!(parse("").portrait(), None, "an older frame has no fields");
        assert_eq!(parse(r#","fields":{"name":"A"}"#).portrait(), None);
        assert_eq!(parse(r#","fields":{"portrait":""}"#).portrait(), None);
        assert_eq!(parse(r#","fields":{"portrait":7}"#).portrait(), None);
        assert_eq!(
            parse(r#","fields":{"portrait":"finn-alder.webp"}"#).portrait(),
            Some("finn-alder.webp")
        );
    }
}
