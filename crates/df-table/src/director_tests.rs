//! The Director's camera stage with a controlled clock: every call passes the time, so no test
//! sleeps. Rules of focus, the view, the zoom ceiling, returns and the log (O.14 acceptance cases
//! that need no engine events). Pace, priority and bursts are in `director_pace_tests`.

use crate::director::{
    DEFAULT_FOCUS_HOLD_MS, Director, DirectorSettings, Receipt, Request, StageDirection,
    UNENDED_FOCUS_LIMIT_MS,
};
use crate::director_error::DirectorError;
use crate::director_log::{Cause, DropReason, EndsReport, Outcome};
use crate::director_reason::{
    Ends, FOCUS_MOVE_MAXIMUM_MS, FOCUS_MOVE_MINIMUM_MS, MAXIMUM_CAMERA_MS, MAXIMUM_ZOOM_PERCENT,
    Reason, ReasonSpec, ReasonTable, WideReason,
};
use crate::director_view::{
    BeatHold, BeatKind, PlateView, StageView, TargetView, clamped_zoom, zoom_ceiling_percent,
};
use crate::element::ElementId;
use crate::inspect::InspectAudience;

pub(crate) fn id(name: &str) -> ElementId {
    ElementId::new(name).expect("a valid element id")
}

/// A plate of unknown width with these targets, each as given.
pub(crate) fn view_of(targets: &[(&str, TargetView)]) -> StageView {
    let mut view = StageView::new(PlateView::Plate {
        generation: 1,
        width_px: None,
    });
    for (name, target) in targets {
        view = view.with_target(id(name), *target);
    }
    view
}

/// Finn and Sigrun (heroes), a guard and a lurker (hidden).
pub(crate) fn tavern() -> StageView {
    view_of(&[
        ("finn", TargetView::HERO),
        ("sigrun", TargetView::HERO),
        ("brenna", TargetView::HERO),
        ("guard", TargetView::SEEN),
        ("lurker", TargetView::HIDDEN),
    ])
}

pub(crate) fn director() -> Director {
    Director::new(DirectorSettings::defaults()).expect("default settings are valid")
}

/// Asks, ticks at the same time, and returns what the tick moved.
pub(crate) fn ask(
    director: &mut Director,
    now_ms: u64,
    view: &StageView,
    request: Request,
) -> Vec<StageDirection> {
    director
        .request(now_ms, view, request)
        .expect("the request is well formed");
    director.tick(now_ms, view).expect("the clock moves on")
}

pub(crate) fn tick(director: &mut Director, now_ms: u64, view: &StageView) -> Vec<StageDirection> {
    director.tick(now_ms, view).expect("the clock moves on")
}

pub(crate) fn focus_of(direction: &StageDirection) -> (&str, Reason, u64) {
    match direction {
        StageDirection::Focus {
            element,
            reason,
            serial,
            ..
        } => (element.as_str(), *reason, *serial),
        other => panic!("expected a focus, got {other:?}"),
    }
}

pub(crate) fn wide_of(direction: &StageDirection) -> (WideReason, u32, u64) {
    match direction {
        StageDirection::Wide {
            reason,
            over_ms,
            serial,
            ..
        } => (*reason, *over_ms, *serial),
        other => panic!("expected a pull-back, got {other:?}"),
    }
}

pub(crate) fn lines(director: &mut Director, audience: InspectAudience) -> Vec<String> {
    director
        .take_unread()
        .iter()
        .flat_map(|entry| entry.lines(audience))
        .collect()
}

#[test]
fn the_reasons_are_ranked_in_the_order_of_the_plan_and_their_defaults_are_the_tables() {
    let order: Vec<&str> = {
        let mut reasons = Reason::ALL.to_vec();
        reasons.sort_by_key(|reason| std::cmp::Reverse(reason.rank()));
        reasons.iter().map(|reason| reason.key()).collect()
    };
    assert_eq!(
        order,
        [
            "hit",
            "roll",
            "reveal",
            "active_creature",
            "speaker",
            "arrival",
            "spotlight",
            "requested"
        ]
    );
    let table = ReasonTable::defaults();
    let numbers = |reason| {
        let spec = table.spec(reason);
        (spec.zoom_percent, spec.over_ms)
    };
    assert_eq!(numbers(Reason::Speaker), (200, 1_200));
    assert_eq!(numbers(Reason::ActiveCreature), (160, 900));
    assert_eq!(numbers(Reason::Hit), (220, 700));
    assert_eq!(numbers(Reason::Reveal), (250, 1_500));
    assert_eq!(numbers(Reason::Roll), (200, 900));
    assert_eq!(numbers(Reason::Arrival), (180, 1_200));
    assert_eq!(numbers(Reason::Spotlight), (180, 1_500));
    assert_eq!(numbers(Reason::Requested), (180, 1_200));
    assert_eq!(table.spec(Reason::Speaker).ends, Ends::WhenReasonEnds);
    assert_eq!(table.spec(Reason::Reveal).ends, Ends::AfterHold);
    assert_eq!(DEFAULT_FOCUS_HOLD_MS, 8_000);
    assert_eq!(Reason::from_key("hit"), Some(Reason::Hit));
    assert_eq!(Reason::from_key("wide"), None);
}

#[test]
fn a_speaker_focus_is_one_numbered_move_with_the_reasons_numbers_and_an_audit_line() {
    let mut director = director();
    let view = tavern();
    let moves = ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    let [
        StageDirection::Focus {
            zoom_percent,
            over_ms,
            clamped_from,
            driven,
            ..
        },
    ] = moves.as_slice()
    else {
        panic!("expected one focus, got {moves:?}");
    };
    assert_eq!(
        (*zoom_percent, *over_ms, *clamped_from, *driven),
        (200, 1_200, None, false)
    );
    assert_eq!(focus_of(&moves[0]), ("guard", Reason::Speaker, 1));
    assert_eq!(
        lines(&mut director, InspectAudience::Operator),
        [
            "CAMERA queued direction=speaker element=guard",
            "CAMERA focus element=guard zoom=200 over=1200 reason=speaker serial=1"
        ]
    );
}

#[test]
fn a_driven_request_says_so_in_its_line_and_its_numbers_can_be_given() {
    let mut director = director();
    let view = tavern();
    let request = Request::focus(Reason::Speaker, id("finn"))
        .zoom(250)
        .over(1_500)
        .driven();
    let moves = ask(&mut director, 0, &view, request);
    assert!(matches!(
        moves.as_slice(),
        [StageDirection::Focus {
            zoom_percent: 250,
            over_ms: 1_500,
            driven: true,
            ..
        }]
    ));
    let lines = lines(&mut director, InspectAudience::Operator);
    assert_eq!(
        lines.last().map(String::as_str),
        Some("CAMERA focus element=finn zoom=250 over=1500 reason=speaker serial=1 driven=true")
    );
}

#[test]
fn a_hidden_creature_is_refused_as_a_target_and_a_hero_is_always_focusable() {
    let mut director = director();
    let view = tavern();
    assert_eq!(
        director
            .request(0, &view, Request::focus(Reason::Speaker, id("lurker")))
            .expect("well formed"),
        Receipt::Dropped(DropReason::TargetHidden)
    );
    assert!(tick(&mut director, 0, &view).is_empty());
    // A hero the scene marks as not visible is still a hero (K.13).
    let hero_view = view_of(&[(
        "finn",
        TargetView {
            visible: false,
            hero: true,
            anchored: true,
        },
    )]);
    let moves = ask(
        &mut director,
        1_000,
        &hero_view,
        Request::focus(Reason::Spotlight, id("finn")),
    );
    assert_eq!(focus_of(&moves[0]).0, "finn");
    // A name that is not on the stage is its own refusal.
    assert_eq!(
        director
            .request(2_000, &view, Request::focus(Reason::Speaker, id("nobody")))
            .expect("well formed"),
        Receipt::Dropped(DropReason::UnknownTarget)
    );
}

#[test]
fn a_creature_that_becomes_hidden_is_pulled_back_in_the_next_tick_through_spacing_and_a_result_card()
 {
    let mut director = director();
    let mut view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    // A result card about someone else, and the spacing from the move just made, both hold.
    view.beat = Some(BeatHold {
        kind: BeatKind::ResultCard,
        until_ms: 10_000,
        subject: Some(id("finn")),
    });
    view.targets.insert(id("guard"), TargetView::HIDDEN);
    let moves = tick(&mut director, 10, &view);
    assert_eq!(moves.len(), 1);
    assert_eq!(wide_of(&moves[0]), (WideReason::TargetHidden, 1_200, 2));
    assert!(!director.is_focused_on(&id("guard")));
    // The shared audience's line names no element; the operator's log keeps the id.
    let entries = director.take_unread();
    let pulled = entries.last().expect("the pull-back is logged");
    assert_eq!(
        pulled.lines(InspectAudience::Host),
        ["CAMERA wide reason=target_hidden serial=2 over=1200"]
    );
    assert_eq!(
        pulled.for_audience(InspectAudience::Operator).element,
        Some(id("guard"))
    );
    assert_eq!(pulled.for_audience(InspectAudience::Host).element, None);
}

#[test]
fn a_target_that_left_the_stage_pulls_the_camera_back() {
    let mut director = director();
    let mut view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Arrival, id("guard")),
    );
    view.targets.remove(&id("guard"));
    let moves = tick(&mut director, 50, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::TargetGone);
}

#[test]
fn while_a_card_shows_every_focus_is_declined_no_target_and_a_card_pulls_a_held_focus_wide() {
    let mut director = director();
    let mut view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    view.plate = PlateView::Card;
    assert_eq!(
        director
            .request(100, &view, Request::focus(Reason::Hit, id("finn")))
            .expect("well formed"),
        Receipt::Dropped(DropReason::NoTarget)
    );
    let moves = tick(&mut director, 100, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::NoAnchor);
}

#[test]
fn a_swap_inside_a_scene_aims_at_the_same_creatures_anchor_or_goes_wide() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    // The next plate has the guard: one new move at the same creature within the dissolve.
    let mut swapped = tavern();
    swapped.plate = PlateView::Plate {
        generation: 2,
        width_px: None,
    };
    let moves = tick(&mut director, 5_000, &swapped);
    assert_eq!(moves.len(), 1);
    assert_eq!(focus_of(&moves[0]), ("guard", Reason::Speaker, 2));
    assert!(matches!(
        moves[0],
        StageDirection::Focus {
            over_ms: 1_500,
            zoom_percent: 200,
            ..
        }
    ));
    // The plate after it has the guard but gives it no anchor: wide, never the old position.
    let mut unanchored = swapped.clone();
    unanchored.plate = PlateView::Plate {
        generation: 3,
        width_px: None,
    };
    unanchored.targets.insert(
        id("guard"),
        TargetView {
            visible: true,
            hero: false,
            anchored: false,
        },
    );
    let moves = tick(&mut director, 9_000, &unanchored);
    assert_eq!(wide_of(&moves[0]).0, WideReason::NoAnchor);
    // A request for a creature with no anchor is declined, a narrator leaves the camera alone.
    assert_eq!(
        director
            .request(
                10_000,
                &unanchored,
                Request::focus(Reason::Speaker, id("guard"))
            )
            .expect("well formed"),
        Receipt::Dropped(DropReason::NoAnchor)
    );
}

#[test]
fn a_collapsed_beat_keeps_the_camera_wide_for_the_active_creature_only() {
    let mut director = director();
    let mut view = tavern();
    view.collapsed_beat = true;
    assert_eq!(
        director
            .request(
                0,
                &view,
                Request::focus(Reason::ActiveCreature, id("guard"))
            )
            .expect("well formed"),
        Receipt::Dropped(DropReason::CollapsedBeat)
    );
    let moves = ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    assert_eq!(moves.len(), 1);
}

#[test]
fn a_moment_focus_returns_wide_when_its_hold_ends_and_a_reason_focus_when_the_engine_ends_it() {
    let mut director = director();
    let view = tavern();
    // `reveal` holds `focus_hold_s` (8 s) from its start.
    let moves = ask(
        &mut director,
        1_000,
        &view,
        Request::focus(Reason::Reveal, id("guard")),
    );
    assert_eq!(focus_of(&moves[0]).2, 1);
    assert!(tick(&mut director, 8_999, &view).is_empty());
    let moves = tick(&mut director, 9_000, &view);
    assert_eq!(wide_of(&moves[0]), (WideReason::Timeout, 1_200, 2));
    assert_eq!(
        lines(&mut director, InspectAudience::Operator)
            .last()
            .map(String::as_str),
        Some("CAMERA wide reason=timeout serial=2 over=1200")
    );
    // `speaker` stays until the line's dwell ends, then returns on the next tick.
    ask(
        &mut director,
        20_000,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    assert!(tick(&mut director, 60_000, &view).is_empty());
    director
        .end(61_000, Reason::Speaker, &id("finn"))
        .expect("the clock moves on");
    let moves = tick(&mut director, 61_000, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::ReasonEnded);
}

#[test]
fn a_focus_nobody_ends_returns_at_the_safety_limit_so_a_zoom_is_never_stuck() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    assert!(tick(&mut director, UNENDED_FOCUS_LIMIT_MS - 1, &view).is_empty());
    let moves = tick(&mut director, UNENDED_FOCUS_LIMIT_MS, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::Timeout);
}

#[test]
fn a_new_scene_and_combat_start_wide_and_forget_what_was_waiting() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    // A request inside the spacing is waiting.
    director
        .request(100, &view, Request::focus(Reason::Speaker, id("finn")))
        .expect("well formed");
    tick(&mut director, 100, &view);
    let direction = director
        .start_scene(200)
        .expect("the clock moves on")
        .expect("a held focus is pulled back");
    assert_eq!(wide_of(&direction), (WideReason::SceneStart, 0, 2));
    // Nothing waits any more, and the new scene is not held to the old scene's spacing.
    assert!(tick(&mut director, 1_000, &view).is_empty());
    let moves = ask(
        &mut director,
        1_001,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    assert_eq!(focus_of(&moves[0]).2, 3);
    let direction = director
        .start_combat(5_000, &view)
        .expect("the clock moves on")
        .expect("combat starts wide");
    assert_eq!(wide_of(&direction).0, WideReason::CombatStart);
    assert_eq!(director.start_combat(6_000, &view).expect("ok"), None);
    assert_eq!(director.start_scene(7_000).expect("ok"), None);
}

#[test]
fn a_move_during_a_result_card_waits_unless_the_card_is_about_its_target() {
    let mut director = director();
    let mut view = tavern();
    view.beat = Some(BeatHold {
        kind: BeatKind::ResultCard,
        until_ms: 4_000,
        subject: Some(id("finn")),
    });
    // The card is about Finn: the camera may go to Finn during it.
    let moves = ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Roll, id("finn")),
    );
    assert_eq!(focus_of(&moves[0]).0, "finn");
    // A speaker elsewhere waits for the card (and the spacing) to pass.
    director
        .request(1_000, &view, Request::focus(Reason::Roll, id("sigrun")))
        .expect("well formed");
    assert!(tick(&mut director, 1_000, &view).is_empty());
    assert!(tick(&mut director, 3_999, &view).is_empty());
    let moves = tick(&mut director, 4_000, &view);
    assert_eq!(focus_of(&moves[0]).0, "sigrun");
}

#[test]
fn a_return_waits_for_a_result_card_that_is_not_about_the_target() {
    let mut director = director();
    let mut view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Reveal, id("guard")).hold(1_000),
    );
    view.beat = Some(BeatHold {
        kind: BeatKind::ResultCard,
        until_ms: 3_000,
        subject: Some(id("finn")),
    });
    assert!(tick(&mut director, 1_000, &view).is_empty());
    assert!(tick(&mut director, 2_999, &view).is_empty());
    let moves = tick(&mut director, 3_000, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::Timeout);
}

#[test]
fn a_reveal_beat_holds_every_move_but_the_reveals() {
    let mut director = director();
    let mut view = tavern();
    view.beat = Some(BeatHold {
        kind: BeatKind::RevealBeat,
        until_ms: 5_000,
        subject: Some(id("guard")),
    });
    director
        .request(0, &view, Request::focus(Reason::Speaker, id("finn")))
        .expect("well formed");
    assert!(tick(&mut director, 0, &view).is_empty());
    // The reveal belongs to the beat and starts at once; the speaker it outranked is void.
    let moves = ask(
        &mut director,
        100,
        &view,
        Request::focus(Reason::Reveal, id("sigrun")),
    );
    assert_eq!(focus_of(&moves[0]), ("sigrun", Reason::Reveal, 1));
}

#[test]
fn reduced_effects_make_every_move_a_cut() {
    let mut director = director();
    let mut view = tavern();
    view.reduced_effects = true;
    let moves = ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    assert!(matches!(moves[0], StageDirection::Focus { over_ms: 0, .. }));
    director
        .end(1_000, Reason::Speaker, &id("guard"))
        .expect("ok");
    let moves = tick(&mut director, 1_000, &view);
    assert_eq!(wide_of(&moves[0]).1, 0);
}

#[test]
fn a_zoom_is_held_to_the_plates_ceiling_and_the_cut_is_logged_not_refused() {
    let mut director = director();
    let mut view = tavern();
    // An 800 px picture on a 400 px stage bears 200 percent.
    view.plate = PlateView::Plate {
        generation: 1,
        width_px: Some(800),
    };
    view.stage_width_px = 400;
    assert_eq!(view.zoom_ceiling_percent(), 200);
    let moves = ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")).zoom(400),
    );
    assert!(matches!(
        moves[0],
        StageDirection::Focus {
            zoom_percent: 200,
            clamped_from: Some(400),
            ..
        }
    ));
    assert!(
        lines(&mut director, InspectAudience::Operator)
            .contains(&"CAMERA clamped requested=400 allowed=200 serial=1".to_owned())
    );
    assert_eq!(
        director
            .report(InspectAudience::Operator, &view, 10)
            .ceiling_percent,
        200
    );
}

#[test]
fn the_ceiling_is_the_least_of_the_models_largest_the_plate_and_never_below_the_whole_stage() {
    assert_eq!(zoom_ceiling_percent(None, 1_920), MAXIMUM_ZOOM_PERCENT);
    assert_eq!(zoom_ceiling_percent(Some(4_000), 0), MAXIMUM_ZOOM_PERCENT);
    assert_eq!(zoom_ceiling_percent(Some(4_000), 1_000), 400);
    assert_eq!(zoom_ceiling_percent(Some(1_000), 4_000), 100);
    assert_eq!(
        zoom_ceiling_percent(Some(u32::MAX), 1),
        MAXIMUM_ZOOM_PERCENT
    );
    assert_eq!(clamped_zoom(400, 210), (210, Some(210)));
    assert_eq!(clamped_zoom(150, 210), (150, None));
}

#[test]
fn a_request_or_setting_out_of_range_is_a_typed_error_naming_the_range() {
    let mut director = director();
    let view = tavern();
    let focus = |zoom: Option<u16>, over: Option<u32>, hold: Option<u64>| {
        let mut request = Request::focus(Reason::Speaker, id("guard"));
        request.zoom_percent = zoom;
        request.over_ms = over;
        request.hold_ms = hold;
        request
    };
    let refused = |director: &mut Director, request: Request| {
        director
            .request(0, &view, request)
            .expect_err("the request is refused")
    };
    assert_eq!(
        refused(&mut director, focus(Some(99), None, None)),
        DirectorError::ZoomOutOfRange { zoom_percent: 99 }
    );
    assert_eq!(
        refused(
            &mut director,
            focus(Some(MAXIMUM_ZOOM_PERCENT + 1), None, None)
        ),
        DirectorError::ZoomOutOfRange {
            zoom_percent: MAXIMUM_ZOOM_PERCENT + 1
        }
    );
    for over in [1, FOCUS_MOVE_MINIMUM_MS - 1, FOCUS_MOVE_MAXIMUM_MS + 1] {
        assert_eq!(
            refused(&mut director, focus(None, Some(over), None)),
            DirectorError::FocusMoveOutOfRange { over_ms: over }
        );
    }
    for over in [0, FOCUS_MOVE_MINIMUM_MS, FOCUS_MOVE_MAXIMUM_MS] {
        assert!(
            director
                .request(0, &view, focus(None, Some(over), None))
                .is_ok()
        );
    }
    assert_eq!(
        refused(&mut director, focus(None, None, Some(0))),
        DirectorError::HoldOutOfRange { hold_ms: 0 }
    );
    let mut nobody = Request::focus(Reason::Speaker, id("guard"));
    nobody.targets.clear();
    assert_eq!(refused(&mut director, nobody), DirectorError::NoTargets);
    assert_eq!(
        director.pull_back(0, &tavern(), Some(MAXIMUM_CAMERA_MS + 1)),
        Err(DirectorError::WideMoveOutOfRange {
            over_ms: MAXIMUM_CAMERA_MS + 1
        })
    );
    let message = DirectorError::FocusMoveOutOfRange { over_ms: 3 }.to_string();
    assert!(message.contains("600 to 2500"), "{message}");
    let mut settings = DirectorSettings::defaults();
    settings.focus_hold_ms = 0;
    assert!(matches!(
        Director::new(settings),
        Err(DirectorError::BadSetting {
            name: "focus_hold_ms",
            ..
        })
    ));
    let mut table = ReasonTable::defaults();
    let bad = ReasonSpec {
        zoom_percent: 50,
        over_ms: 900,
        ends: Ends::AfterHold,
    };
    assert_eq!(
        table.set(Reason::Hit, bad),
        Err(DirectorError::ZoomOutOfRange { zoom_percent: 50 })
    );
    assert_eq!(table.spec(Reason::Hit).zoom_percent, 220);
}

#[test]
fn a_clock_that_runs_backwards_is_refused_and_changes_nothing() {
    let mut director = director();
    let view = tavern();
    tick(&mut director, 500, &view);
    assert_eq!(
        director.tick(499, &view),
        Err(DirectorError::ClockWentBack {
            now_ms: 499,
            last_ms: 500
        })
    );
    assert_eq!(director.now_ms(), 500);
}

#[test]
fn pause_and_auto_hold_automatic_directions_but_not_a_driven_one() {
    let mut director = director();
    let view = tavern();
    director.set_paused(true);
    assert_eq!(
        director
            .request(0, &view, Request::focus(Reason::Speaker, id("guard")))
            .expect("well formed"),
        Receipt::Dropped(DropReason::Paused)
    );
    let moves = ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")).driven(),
    );
    assert_eq!(moves.len(), 1);
    // Nothing expires while a person drives by hand ...
    assert!(tick(&mut director, 700_000, &view).is_empty());
    director.set_paused(false);
    // ... and the held focus goes home as soon as the Director is let go.
    let moves = tick(&mut director, 700_001, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::Timeout);
    director.set_auto_camera(false);
    assert_eq!(
        director
            .request(701_000, &view, Request::focus(Reason::Speaker, id("guard")))
            .expect("well formed"),
        Receipt::Dropped(DropReason::AutoOff)
    );
    assert!(!director.auto_camera());
    director.set_auto_camera(true);
}

#[test]
fn the_operators_pull_back_is_driven_whatever_the_camera_held() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Hit, id("finn")),
    );
    let direction = director.pull_back(10, &view, Some(500)).expect("ok");
    assert_eq!(wide_of(&direction), (WideReason::Operator, 500, 2));
    assert_eq!(
        lines(&mut director, InspectAudience::Operator)
            .last()
            .map(String::as_str),
        Some("CAMERA wide reason=operator serial=2 over=500 driven=true")
    );
}

#[test]
fn director_show_matches_the_log_and_the_shared_audience_never_learns_what_is_hidden() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    director
        .request(10, &view, Request::focus(Reason::Speaker, id("lurker")))
        .expect("well formed");
    director
        .request(20, &view, Request::focus(Reason::Speaker, id("nobody")))
        .expect("well formed");
    let operator = director.report(InspectAudience::Operator, &view, 100);
    let shared = director.report(InspectAudience::Host, &view, 100);
    assert_eq!(
        (operator.queued, operator.applied, operator.dropped),
        (1, 1, 2)
    );
    assert_eq!(operator.serial, 1);
    assert_eq!(
        operator.active.as_ref().map(|active| active.reason),
        Some(Reason::Speaker)
    );
    assert_eq!(
        operator.active.as_ref().map(|active| active.ends),
        Some(EndsReport::WhenReasonEnds(UNENDED_FOCUS_LIMIT_MS))
    );
    // The operator reads the ids; the shared audience reads `target_hidden` and no id, for the
    // hidden creature and for a name that is not on the stage alike.
    let hidden_rows = |report: &crate::director_log::DirectorReport| {
        report
            .log
            .iter()
            .filter_map(|entry| match entry.outcome {
                Outcome::Dropped { why } => Some((why, entry.element.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        hidden_rows(&operator),
        [
            (DropReason::TargetHidden, Some(id("lurker"))),
            (DropReason::UnknownTarget, Some(id("nobody")))
        ]
    );
    assert_eq!(
        hidden_rows(&shared),
        [
            (DropReason::TargetHidden, None),
            (DropReason::TargetHidden, None)
        ]
    );
    let shared_text = serde_json::to_string(&shared).expect("serialises");
    assert!(!shared_text.contains("lurker") && !shared_text.contains("nobody"));
    // The lines are the same rows rendered, in order.
    let from_log: Vec<String> = director
        .take_unread()
        .iter()
        .flat_map(|entry| entry.lines(InspectAudience::Host))
        .collect();
    assert!(from_log.iter().all(|line| !line.contains("lurker")));
    assert!(from_log.contains(&"CAMERA dropped reason=target_hidden direction=speaker".to_owned()));
    assert_eq!(director.take_unread(), []);
}

#[test]
fn the_report_withholds_the_name_of_a_focus_that_just_became_hidden() {
    let mut director = director();
    let mut view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    view.targets.insert(id("guard"), TargetView::HIDDEN);
    let shared = director.report(InspectAudience::Host, &view, 10);
    assert_eq!(shared.active.expect("still held").element, None);
    let operator = director.report(InspectAudience::Operator, &view, 10);
    assert_eq!(
        operator.active.expect("still held").element,
        Some(id("guard"))
    );
}

#[test]
fn the_log_is_bounded_and_keeps_the_newest_entries() {
    let mut director = director();
    let view = tavern();
    for round in 0..50_u64 {
        director
            .request(round, &view, Request::focus(Reason::Speaker, id("lurker")))
            .expect("well formed");
    }
    let report = director.report(InspectAudience::Operator, &view, 10);
    assert_eq!(report.log.len(), 10);
    assert_eq!(report.log.first().map(|entry| entry.sequence), Some(41));
    assert_eq!(report.log.last().map(|entry| entry.sequence), Some(50));
    assert_eq!(director.take_unread().len(), 50);
    let cause = Cause::Focus(Reason::Hit);
    assert_eq!(cause.key(), "hit");
}
