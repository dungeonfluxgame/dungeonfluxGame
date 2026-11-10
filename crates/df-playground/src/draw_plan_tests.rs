//! What the page redraws when something changed: a table of changes and the pieces each touches.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::controls::{Controls, ControlsChange, Device};
use crate::copy::ReadingPace;
use crate::draw_plan::{DrawPlan, Part, Snapshot, frame_action, wants_live};
use crate::fixture::{Fixture, FixtureTitle, PlateKind};
use crate::scene::{ImageSlot, SceneChange, SceneOverlay, Target};

const SCENE: &str = r##"{
    "name": "Quay",
    "description": "A wet quay.",
    "tv": {
        "place": "The quay",
        "status": {"kind": "ok"},
        "caption": {"speaker": null, "text": "Rain."},
        "backdrop": {"media": "quay.png", "caption": null},
        "elements": "live"
    },
    "phone": {
        "hero": "Pip",
        "status": {"kind": "ok"},
        "suggested": [{"label": "Wait", "explain": null}]
    }
}"##;

fn fixture() -> Fixture {
    Fixture::parse(SCENE).expect("reads")
}

fn snapshot(fixture: &Fixture) -> Snapshot {
    Snapshot {
        names: vec![FixtureTitle {
            file: "quay.json".to_owned(),
            title: Some("The quay".to_owned()),
        }],
        controls: Controls {
            fixture: Some("quay.json".to_owned()),
            ..Controls::default()
        },
        loaded_name: Some("quay.json".to_owned()),
        shown: Some(Rc::new(Ok(fixture.clone()))),
        connection_problem: None,
        live_changes: false,
        audio_unlocked: false,
    }
}

fn shown_with(base: &Snapshot, change: SceneChange, at_ms: u64) -> Snapshot {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply_at(change, &fixture(), at_ms)
        .expect("applies");
    let mut next = fixture();
    overlay.apply_to(&mut next);
    Snapshot {
        shown: Some(Rc::new(Ok(next))),
        ..base.clone()
    }
}

fn parts(plan: &DrawPlan) -> BTreeSet<Part> {
    plan.parts.clone()
}

#[test]
fn the_first_draw_draws_everything() {
    let new = snapshot(&fixture());
    assert_eq!(DrawPlan::between(None, &new), DrawPlan::everything());
}

#[test]
fn nothing_changed_draws_nothing() {
    let old = snapshot(&fixture());
    assert!(DrawPlan::between(Some(&old), &old.clone()).is_empty());
}

#[test]
fn a_sound_music_or_a_transition_setting_draws_nothing() {
    let old = snapshot(&fixture());
    for change in [
        SceneChange::Sound {
            media: "bell.wav".to_owned(),
            target: Target::Both,
            volume_percent: 80,
        },
        SceneChange::Music {
            media: "theme.ogg".to_owned(),
            target: Target::Both,
            volume_percent: 80,
            looping: true,
        },
        SceneChange::StopMusic {},
    ] {
        let new = shown_with(&old, change.clone(), 0);
        assert!(DrawPlan::between(Some(&old), &new).is_empty(), "{change:?}");
    }
    let mut controls = old.clone();
    controls.controls.transition.duration_ms = 900;
    assert!(DrawPlan::between(Some(&old), &controls).is_empty());
}

#[test]
fn a_caption_redraws_only_the_caption() {
    let old = snapshot(&fixture());
    let new = shown_with(
        &old,
        SceneChange::Caption {
            speaker: None,
            text: "The bell rings.".to_owned(),
        },
        0,
    );
    let plan = DrawPlan::between(Some(&old), &new);
    assert_eq!(parts(&plan), BTreeSet::from([Part::TvCaption]));
    assert!(!plan.toolbar && !plan.devices && !plan.notes && !plan.stage);
}

#[test]
fn a_party_change_a_picture_and_a_choice_each_redraw_their_own_piece() {
    let old = snapshot(&fixture());
    let party = shown_with(
        &old,
        SceneChange::PartyAdd {
            member: crate::fixture::PartyMember {
                name: "Pip".to_owned(),
                hit_points: 3,
                maximum_hit_points: 9,
                conditions: Vec::new(),
                color: None,
            },
        },
        0,
    );
    assert_eq!(
        parts(&DrawPlan::between(Some(&old), &party)),
        BTreeSet::from([Part::TvParty])
    );
    let card = shown_with(
        &old,
        SceneChange::Image {
            slot: ImageSlot::TvCard,
            media: "map.png".to_owned(),
            caption: None,
            rotation_degrees: 0,
            slide: None,
            layout: None,
            plate: PlateKind::Room,
        },
        0,
    );
    assert_eq!(
        parts(&DrawPlan::between(Some(&old), &card)),
        BTreeSet::from([Part::TvCard])
    );
    let choices = shown_with(
        &old,
        SceneChange::Suggested {
            choices: vec![crate::fixture::Choice {
                label: "Run".to_owned(),
                explain: None,
            }],
        },
        0,
    );
    let plan = DrawPlan::between(Some(&old), &choices);
    assert_eq!(parts(&plan), BTreeSet::from([Part::PhoneChoices]));
    assert!(!plan.notes, "the same warnings stay");
}

#[test]
fn a_slide_that_is_only_further_along_is_not_a_new_picture() {
    let old = snapshot(&fixture());
    let slide = crate::fixture::Slide {
        from_percent: 15,
        to_percent: 85,
        duration_ms: 5_000,
        serial: 0,
        began_ms: 0,
        elapsed_ms: 0,
    };
    let sliding = |at_ms: u64, now: u64| {
        let mut overlay = SceneOverlay::default();
        overlay
            .apply_at(
                SceneChange::Image {
                    slot: ImageSlot::TvCard,
                    media: "map.png".to_owned(),
                    caption: None,
                    rotation_degrees: 0,
                    slide: Some(slide),
                    layout: None,
                    plate: PlateKind::Room,
                },
                &fixture(),
                at_ms,
            )
            .expect("applies");
        let overlay = overlay.at(now);
        let mut next = fixture();
        overlay.apply_to(&mut next);
        Snapshot {
            shown: Some(Rc::new(Ok(next))),
            ..old.clone()
        }
    };
    let early = sliding(100, 200);
    let later = sliding(100, 2_000);
    assert!(DrawPlan::between(Some(&early), &later).is_empty());
}

#[test]
fn a_new_slide_is_a_new_picture() {
    let old = snapshot(&fixture());
    let slide = |serial_apply: u32| {
        let mut overlay = SceneOverlay::default();
        for _ in 0..serial_apply {
            overlay
                .apply(
                    SceneChange::Image {
                        slot: ImageSlot::TvCard,
                        media: "map.png".to_owned(),
                        caption: None,
                        rotation_degrees: 0,
                        layout: None,
                        plate: PlateKind::Room,
                        slide: Some(crate::fixture::Slide {
                            from_percent: 0,
                            to_percent: 100,
                            duration_ms: 1_000,
                            serial: 0,
                            began_ms: 0,
                            elapsed_ms: 0,
                        }),
                    },
                    &fixture(),
                )
                .expect("applies");
        }
        let mut next = fixture();
        overlay.apply_to(&mut next);
        Snapshot {
            shown: Some(Rc::new(Ok(next))),
            ..old.clone()
        }
    };
    let plan = DrawPlan::between(Some(&slide(1)), &slide(2));
    assert_eq!(parts(&plan), BTreeSet::from([Part::TvCard]));
}

#[test]
fn another_scene_device_or_a_scene_that_cannot_be_read_rebuilds_the_devices() {
    let old = snapshot(&fixture());
    let mut other_scene = old.clone();
    other_scene.loaded_name = Some("tavern.json".to_owned());
    let plan = DrawPlan::between(Some(&old), &other_scene);
    assert!(plan.devices && plan.notes && plan.stage);
    let mut phone_only = old.clone();
    phone_only.controls.device = Device::Phone;
    assert!(DrawPlan::between(Some(&old), &phone_only).devices);
    let mut broken = old.clone();
    broken.shown = Some(Rc::new(Err("line 3, column 4: bad".to_owned())));
    assert!(DrawPlan::between(Some(&old), &broken).devices);
}

#[test]
fn a_changed_error_message_redraws_the_notes_and_the_same_one_nothing() {
    let mut old = snapshot(&fixture());
    old.shown = Some(Rc::new(Err("line 3: bad".to_owned())));
    let same = old.clone();
    assert!(DrawPlan::between(Some(&old), &same).is_empty());
    let mut other = old.clone();
    other.shown = Some(Rc::new(Err("line 9: worse".to_owned())));
    let plan = DrawPlan::between(Some(&old), &other);
    assert!(plan.notes && !plan.devices);
}

#[test]
fn a_toolbar_control_redraws_the_toolbar_and_a_cli_scene_change_does_not() {
    let old = snapshot(&fixture());
    let mut paced = old.clone();
    paced.controls.pace = ReadingPace::Fast;
    let plan = DrawPlan::between(Some(&old), &paced);
    assert!(plan.toolbar);
    assert_eq!(
        parts(&plan),
        BTreeSet::from([Part::TvCaption]),
        "the dwell is in the caption"
    );
    let caption = shown_with(
        &old,
        SceneChange::Caption {
            speaker: None,
            text: "x".to_owned(),
        },
        0,
    );
    assert!(!DrawPlan::between(Some(&old), &caption).toolbar);
    let mut sound_on = old.clone();
    sound_on.audio_unlocked = true;
    let plan = DrawPlan::between(Some(&old), &sound_on);
    assert!(plan.toolbar && plan.parts.is_empty());
    for change in [
        ControlsChange::Sound(false),
        ControlsChange::Music(true),
        ControlsChange::Volume(30),
        ControlsChange::Tts(true),
    ] {
        let mut switched = old.clone();
        switched.controls.apply(change.clone()).expect("applies");
        let plan = DrawPlan::between(Some(&old), &switched);
        assert!(
            plan.toolbar,
            "{change:?} redraws the toolbar so its state is shown"
        );
        assert!(!plan.devices, "{change:?}");
    }
    let mut names = old.clone();
    names.names.push(FixtureTitle {
        file: "tavern.json".to_owned(),
        title: None,
    });
    assert!(DrawPlan::between(Some(&old), &names).toolbar);
    let mut retitled = old.clone();
    retitled.names[0].title = Some("The quay at night".to_owned());
    assert!(DrawPlan::between(Some(&old), &retitled).toolbar);
}

#[test]
fn the_grid_a_status_and_a_connection_problem_redraw_their_own_piece() {
    let old = snapshot(&fixture());
    let mut grid = old.clone();
    grid.controls.show_grid = true;
    assert_eq!(
        parts(&DrawPlan::between(Some(&old), &grid)),
        BTreeSet::from([Part::TvGrid])
    );
    let mut status = old.clone();
    status.controls.status_key = "rolling".to_owned();
    let plan = DrawPlan::between(Some(&old), &status);
    assert_eq!(
        parts(&plan),
        BTreeSet::from([Part::TvStatus, Part::PhoneStatus])
    );
    let mut down = old.clone();
    down.connection_problem = Some("could not reach".to_owned());
    let plan = DrawPlan::between(Some(&old), &down);
    assert!(plan.notes && plan.parts.is_empty() && !plan.devices);
}

#[test]
fn a_status_that_stops_input_redraws_the_composer() {
    let old = snapshot(&fixture());
    let mut waiting = old.clone();
    waiting.controls.status_key = "read_only".to_owned();
    let plan = DrawPlan::between(Some(&old), &waiting);
    assert!(plan.parts.contains(&Part::PhoneComposer));
    let mut reading = old.clone();
    reading.controls.status_key = "reading".to_owned();
    let plan = DrawPlan::between(Some(&old), &reading);
    assert!(
        !plan.parts.contains(&Part::PhoneComposer),
        "reading does not stop input"
    );
}

#[test]
fn a_camera_move_asks_for_the_stage_to_be_framed_again_and_a_place_is_a_part() {
    let old = snapshot(&fixture());
    let camera = shown_with(
        &old,
        SceneChange::Camera {
            target: Some("finn".to_owned()),
            zoom_percent: 200,
            duration_ms: 1_000,
        },
        0,
    );
    let plan = DrawPlan::between(Some(&old), &camera);
    assert!(plan.stage && plan.parts.is_empty() && !plan.devices);
}

#[test]
fn the_same_camera_move_read_again_later_does_not_ask_for_anything() {
    let old = snapshot(&fixture());
    let first = shown_with(
        &old,
        SceneChange::Camera {
            target: None,
            zoom_percent: 100,
            duration_ms: 0,
        },
        50,
    );
    let mut later = first.clone();
    if let Some(shown) = later.shown.take()
        && let Ok(mut fixture) = (*shown).clone()
    {
        if let Some(camera) = &mut fixture.tv.camera {
            camera.elapsed_ms = 9_999;
        }
        later.shown = Some(Rc::new(Ok(fixture)));
    }
    assert!(DrawPlan::between(Some(&first), &later).is_empty());
}

#[test]
fn a_frame_decides_between_a_redraw_a_stage_update_and_nothing() {
    // (was_down, controls_changed, reloaded, canvas_changed, live_on_tv)
    let table = [
        ((false, false, false, false, false), (false, false)),
        ((false, false, false, true, true), (false, true)),
        ((false, false, false, true, false), (false, false)),
        ((false, true, false, false, true), (true, false)),
        ((false, true, false, true, true), (true, true)),
        ((true, false, false, false, false), (true, false)),
        ((false, false, true, false, false), (true, false)),
    ];
    for ((down, controls, reloaded, canvas, live), (redraw, stage)) in table {
        let action = frame_action(down, controls, reloaded, canvas, live);
        assert_eq!(
            (action.redraw, action.stage),
            (redraw, stage),
            "{down} {controls} {reloaded} {canvas} {live}"
        );
    }
}

#[test]
fn a_frame_that_changed_only_the_canvas_keeps_the_scenes_camera() {
    // The regression of #1620: the camera is part of what is shown, read from the overlay.
    let old = snapshot(&fixture());
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::Camera {
                target: Some("finn".to_owned()),
                zoom_percent: 250,
                duration_ms: 1_000,
            },
            &fixture(),
        )
        .expect("applies");
    let mut shown = fixture();
    overlay.apply_to(&mut shown);
    let action = frame_action(
        false,
        false,
        false,
        true,
        wants_live(Some(&Ok(shown.clone()))),
    );
    assert!(!action.redraw && action.stage);
    assert_eq!(
        shown.tv.camera.as_ref().map(|camera| camera.zoom_percent),
        Some(250)
    );
    assert!(old.shown.is_some());
}

#[test]
fn only_a_scene_with_the_live_canvas_wants_it() {
    assert!(wants_live(Some(&Ok(fixture()))));
    assert!(!wants_live(None));
    assert!(!wants_live(Some(&Err("bad".to_owned()))));
}

mod transitions {
    use crate::controls::{Transition, TransitionStyle};
    use crate::draw_plan::{is_leaving, leaving_class, transition_for};

    fn fade(ms: u64) -> Transition {
        Transition {
            style: TransitionStyle::Fade,
            duration_ms: ms,
        }
    }

    #[test]
    fn another_scenario_leaves_with_the_chosen_transition_and_nothing_else_does() {
        assert_eq!(
            transition_for(Some("a.json"), "b.json", fade(900)),
            Some(fade(900))
        );
        assert_eq!(
            transition_for(None, "a.json", fade(900)),
            None,
            "the first one"
        );
        assert_eq!(
            transition_for(Some("a.json"), "a.json", fade(900)),
            None,
            "read again"
        );
        assert_eq!(
            transition_for(Some("a.json"), "b.json", fade(0)),
            None,
            "no time"
        );
        let cut = Transition {
            style: TransitionStyle::Cut,
            duration_ms: 900,
        };
        assert_eq!(transition_for(Some("a.json"), "b.json", cut), None, "a cut");
    }

    #[test]
    fn a_wipe_wipes_and_a_fade_fades_and_only_a_leaving_copy_is_taken_away() {
        assert!(leaving_class(TransitionStyle::Wipe).contains("scene-leaving-wipe"));
        assert!(leaving_class(TransitionStyle::Fade).contains("scene-leaving-fade"));
        assert!(leaving_class(TransitionStyle::Cut).contains("scene-leaving-fade"));
        for style in [
            TransitionStyle::Wipe,
            TransitionStyle::Fade,
            TransitionStyle::Cut,
        ] {
            assert!(is_leaving(leaving_class(style)));
        }
        assert!(!is_leaving("devices"));
        assert!(!is_leaving("tv-figures-leaving"));
    }
}

#[test]
fn a_command_during_a_transition_redraws_a_piece_and_never_the_devices_the_copy_lies_beside() {
    let first = snapshot(&fixture());
    let mut tavern = first.clone();
    tavern.loaded_name = Some("tavern.json".to_owned());
    assert!(
        DrawPlan::between(Some(&first), &tavern).devices,
        "the scene change draws the copy"
    );
    for change in [
        SceneChange::Caption {
            speaker: None,
            text: "The bell rings.".to_owned(),
        },
        SceneChange::Sound {
            media: "bell.wav".to_owned(),
            target: Target::Both,
            volume_percent: 80,
        },
        SceneChange::Camera {
            target: None,
            zoom_percent: 100,
            duration_ms: 0,
        },
    ] {
        let next = shown_with(&tavern, change.clone(), 0);
        let plan = DrawPlan::between(Some(&tavern), &next);
        assert!(!plan.devices && !plan.toolbar, "{change:?}");
    }
}

#[test]
fn live_changes_starting_or_ending_redraw_the_notes() {
    let quiet = snapshot(&fixture());
    let live = Snapshot {
        live_changes: true,
        ..quiet.clone()
    };
    assert!(DrawPlan::between(Some(&quiet), &live).notes);
    assert!(DrawPlan::between(Some(&live), &quiet).notes);
    assert!(!DrawPlan::between(Some(&live), &live).notes);
}
