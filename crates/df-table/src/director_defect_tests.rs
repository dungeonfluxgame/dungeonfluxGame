//! The Director's review defects (#1756 to #1770), each with a controlled clock: a hidden target's
//! id never reaches the shared audience, `end` honours the inbox and is keyed to the element, the
//! area rule keeps the card and the spacing and holds across ticks, a failed apply is told back,
//! a camera moved elsewhere is seen, and the size limits and the log bound hold.

use crate::director::error::DirectorError;
use crate::director::log::{Cause, DirectorReport, DropReason, Outcome};
use crate::director::reason::{Ends, Reason, ReasonSpec, ReasonTable, WideReason};
use crate::director::view::{BeatHold, BeatKind, PlateView, StageView, TargetView};
use crate::director::{
    Director, DirectorSetting, DirectorSettings, Receipt, Request, StageDirection,
};
use crate::director_tests::{ask, director, focus_of, id, tavern, tick, view_of, wide_of};
use crate::inspect::InspectAudience;

const SENTINEL: &str = "lurker";

/// Everything a shared reader of this Director could be shown after the target was hidden: the
/// report as JSON and every line, except the lines written while the target was still visible
/// (a request queued or a focus applied then may name it).
fn shared_text(director: &Director, view: &StageView) -> String {
    let mut report: DirectorReport = director.report(InspectAudience::Host, view, usize::MAX);
    report.log.retain(|entry| {
        matches!(entry.outcome, Outcome::Dropped { .. }) || matches!(entry.cause, Cause::Wide(_))
    });
    let lines: Vec<String> = report
        .log
        .iter()
        .flat_map(|entry| entry.lines(InspectAudience::Host))
        .collect();
    let json = serde_json::to_string(&report).expect("serialises");
    format!("{json}\n{}", lines.join("\n"))
}

fn dropped_for(director: &Director, view: &StageView) -> Vec<DropReason> {
    director
        .report(InspectAudience::Operator, view, usize::MAX)
        .log
        .iter()
        .filter_map(|entry| match entry.outcome {
            Outcome::Dropped { why } => Some(why),
            _ => None,
        })
        .collect()
}

fn lurker_seen() -> StageView {
    view_of(&[("lurker", TargetView::SEEN), ("guard", TargetView::SEEN)])
}

fn lurker_hidden() -> StageView {
    view_of(&[("lurker", TargetView::HIDDEN), ("guard", TargetView::SEEN)])
}

fn lurker_card() -> StageView {
    let mut view = lurker_hidden();
    view.plate = PlateView::Card;
    view
}

#[test]
fn a_hidden_targets_id_never_reaches_the_shared_audience_under_any_drop_reason() {
    let mut reached = Vec::new();
    let mut check = |name: &str, director: &Director, view: &StageView| {
        let text = shared_text(director, view);
        assert!(!text.contains(SENTINEL), "{name} leaked the id:\n{text}");
        reached.extend(dropped_for(director, view));
    };
    let ask_for_lurker = |director: &mut Director, view: &StageView| {
        director
            .request(10, view, Request::focus(Reason::Speaker, id(SENTINEL)))
            .expect("well formed")
    };

    let mut paused = director();
    paused.set_paused(true);
    let view = lurker_hidden();
    assert_eq!(
        ask_for_lurker(&mut paused, &view),
        Receipt::Dropped(DropReason::Paused)
    );
    check("pause", &paused, &view);

    let mut auto_off = director();
    auto_off.set_auto_camera(false);
    assert_eq!(
        ask_for_lurker(&mut auto_off, &view),
        Receipt::Dropped(DropReason::AutoOff)
    );
    check("auto off", &auto_off, &view);

    let mut card = director();
    let card_view = lurker_card();
    assert_eq!(
        ask_for_lurker(&mut card, &card_view),
        Receipt::Dropped(DropReason::NoTarget)
    );
    check("card", &card, &card_view);

    let mut hidden = director();
    assert_eq!(
        ask_for_lurker(&mut hidden, &view),
        Receipt::Dropped(DropReason::TargetHidden)
    );
    check("hidden", &hidden, &view);

    let mut unknown = director();
    let nobody = view_of(&[("guard", TargetView::SEEN)]);
    assert_eq!(
        ask_for_lurker(&mut unknown, &nobody),
        Receipt::Dropped(DropReason::UnknownTarget)
    );
    check("unknown", &unknown, &nobody);

    // Seen when asked, hidden before the end of the line.
    let mut ended = director();
    ask_for_lurker(&mut ended, &lurker_seen());
    ended
        .end(20, Reason::Speaker, &id(SENTINEL))
        .expect("the clock moves on");
    check("end", &ended, &view);

    // Seen when asked, hidden before the scene starts.
    let mut scene = director();
    ask_for_lurker(&mut scene, &lurker_seen());
    scene.start_scene(20).expect("the clock moves on");
    check("scene start", &scene, &view);

    // Seen when asked, then hidden and the camera set to manual: the tick drops it as auto_off.
    let mut late = director();
    ask_for_lurker(&mut late, &lurker_seen());
    late.set_auto_camera(false);
    tick(&mut late, 30, &view);
    check("tick after hiding", &late, &view);

    // A focus that became hidden and then a new scene: the pull-back entry names no one.
    let mut pulled = director();
    ask(
        &mut pulled,
        0,
        &lurker_seen(),
        Request::focus(Reason::Speaker, id(SENTINEL)),
    );
    pulled.start_scene(50).expect("the clock moves on");
    check("scene start wide", &pulled, &view);

    // The operator's pull-back of a focus that became hidden since the last tick.
    let mut area = director();
    ask(
        &mut area,
        0,
        &lurker_seen(),
        Request::focus(Reason::Speaker, id(SENTINEL)),
    );
    area.pull_back(10, &view, None).expect("the clock moves on");
    check("operator wide", &area, &view);

    for why in [
        DropReason::Paused,
        DropReason::AutoOff,
        DropReason::NoTarget,
        DropReason::TargetHidden,
        DropReason::UnknownTarget,
        DropReason::ReasonEnded,
        DropReason::SceneChanged,
    ] {
        assert!(reached.contains(&why), "no scenario produced {why:?}");
    }

    // The operator still reads the id in the same entries.
    let operator = serde_json::to_string(&paused.report(InspectAudience::Operator, &view, 10))
        .expect("serialises");
    assert!(operator.contains(SENTINEL));
}

#[test]
fn a_request_still_in_the_inbox_is_voided_when_its_reason_ends() {
    let mut director = director();
    let view = tavern();
    director
        .request(0, &view, Request::focus(Reason::Speaker, id("finn")))
        .expect("well formed");
    director
        .end(0, Reason::Speaker, &id("finn"))
        .expect("the clock moves on");
    assert!(tick(&mut director, 0, &view).is_empty());
    assert!(!director.has_work());
    let dropped = dropped_for(&director, &view);
    assert_eq!(dropped, [DropReason::ReasonEnded]);
}

#[test]
fn end_is_keyed_to_the_element_so_line_as_end_never_cuts_line_b() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    // Line B waits behind the spacing; then the engine reports that line A ended.
    ask(
        &mut director,
        100,
        &view,
        Request::focus(Reason::Speaker, id("sigrun")),
    );
    director
        .end(300, Reason::Speaker, &id("finn"))
        .expect("the clock moves on");
    let report = director.report(InspectAudience::Operator, &view, 10);
    assert_eq!(
        report.waiting.map(|waiting| waiting.element),
        Some(Some(id("sigrun"))),
        "line B is still waiting"
    );
    assert_eq!(
        wide_of(&tick(&mut director, 300, &view)[0]).0,
        WideReason::ReasonEnded
    );
    assert!(tick(&mut director, 1_099, &view).is_empty());
    assert_eq!(
        focus_of(&tick(&mut director, 1_100, &view)[0]),
        ("sigrun", Reason::Speaker, 3)
    );
    // The end of line A does not touch line B once it is the focus, and B's own end does.
    director
        .end(2_000, Reason::Speaker, &id("finn"))
        .expect("the clock moves on");
    assert!(tick(&mut director, 5_000, &view).is_empty());
    assert!(director.is_focused_on(&id("sigrun")));
    director
        .end(5_100, Reason::Speaker, &id("sigrun"))
        .expect("the clock moves on");
    assert_eq!(
        wide_of(&tick(&mut director, 5_100, &view)[0]).0,
        WideReason::ReasonEnded
    );
    // A request waiting for line B's end is void only for B.
    ask(
        &mut director,
        6_000,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    ask(
        &mut director,
        6_100,
        &view,
        Request::focus(Reason::Speaker, id("sigrun")),
    );
    director
        .end(6_200, Reason::Speaker, &id("sigrun"))
        .expect("the clock moves on");
    assert_eq!(
        director
            .report(InspectAudience::Operator, &view, 10)
            .waiting,
        None
    );
    assert!(director.is_focused_on(&id("finn")));
}

fn result_card_until(until_ms: u64) -> BeatHold {
    BeatHold {
        kind: BeatKind::ResultCard,
        until_ms,
        subject: None,
    }
}

#[test]
fn an_area_hit_waits_for_the_result_card_and_for_the_spacing() {
    let mut director = director();
    let mut view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    view.beat = Some(result_card_until(3_000));
    let area = Request::area(Reason::Hit, vec![id("finn"), id("sigrun")]);
    assert!(ask(&mut director, 500, &view, area).is_empty());
    assert!(tick(&mut director, 2_999, &view).is_empty());
    assert!(director.is_focused_on(&id("guard")));
    let moves = tick(&mut director, 3_000, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::Area);

    // A focus started at 0 and an area at 300 ms waits until 800.
    let mut director = crate::director_tests::director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    let area = Request::area(Reason::Hit, vec![id("finn"), id("sigrun")]);
    assert!(ask(&mut director, 300, &view, area).is_empty());
    assert!(tick(&mut director, 799, &view).is_empty());
    assert_eq!(
        wide_of(&tick(&mut director, 800, &view)[0]).0,
        WideReason::Area
    );
}

#[test]
fn heroes_of_one_result_arriving_in_different_ticks_are_still_an_area() {
    let mut director = director();
    let view = tavern();
    let mut moves = Vec::new();
    for (now, hero) in [(0, "finn"), (100, "sigrun"), (200, "brenna")] {
        moves.extend(ask(
            &mut director,
            now,
            &view,
            Request::focus(Reason::Hit, id(hero)).with_result(9),
        ));
    }
    for now in [400, 799, 800, 1_000] {
        moves.extend(tick(&mut director, now, &view));
    }
    assert_eq!(moves.len(), 2, "one focus and one pull-back: {moves:?}");
    assert_eq!(focus_of(&moves[0]), ("finn", Reason::Hit, 1));
    assert_eq!(wide_of(&moves[1]).0, WideReason::Area);
    for hero in ["finn", "sigrun", "brenna"] {
        assert!(!director.is_focused_on(&id(hero)), "{hero} is focused");
    }
    // A later hero of the same result is part of the same area, and another result is not.
    assert!(
        ask(
            &mut director,
            5_000,
            &view,
            Request::focus(Reason::Hit, id("guard")).with_result(9)
        )
        .is_empty()
    );
    let moves = ask(
        &mut director,
        6_000,
        &view,
        Request::focus(Reason::Hit, id("guard")).with_result(10),
    );
    assert_eq!(focus_of(&moves[0]).0, "guard");
}

fn focused_on(name: &str) -> (Director, StageView) {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id(name)),
    );
    (director, view)
}

#[test]
fn a_failed_focus_is_forgotten_and_the_same_request_is_not_swallowed_as_already_focused() {
    let (mut director, view) = focused_on("finn");
    director.end(10, Reason::Speaker, &id("finn")).expect("ok");
    tick(&mut director, 10, &view);
    let moves = ask(
        &mut director,
        2_000,
        &view,
        Request::focus(Reason::Speaker, id("sigrun")),
    );
    director.apply_failed(&moves[0]);
    assert!(!director.is_focused_on(&id("sigrun")));
    let moves = ask(
        &mut director,
        3_000,
        &view,
        Request::focus(Reason::Speaker, id("sigrun")),
    );
    assert_eq!(focus_of(&moves[0]), ("sigrun", Reason::Speaker, 4));
}

#[test]
fn a_failed_wide_is_made_again_on_the_next_tick_and_a_failed_focus_over_a_focus_pulls_back() {
    let (mut director, view) = focused_on("finn");
    director.end(10, Reason::Speaker, &id("finn")).expect("ok");
    let moves = tick(&mut director, 10, &view);
    assert_eq!(wide_of(&moves[0]), (WideReason::ReasonEnded, 1_200, 2));
    director.apply_failed(&moves[0]);
    assert!(director.has_work());
    let again = tick(&mut director, 110, &view);
    assert_eq!(wide_of(&again[0]), (WideReason::ReasonEnded, 1_200, 3));
    assert!(!director.has_work());
    assert!(tick(&mut director, 210, &view).is_empty());

    let (mut director, view) = focused_on("finn");
    let moves = ask(
        &mut director,
        100,
        &view,
        Request::focus(Reason::Hit, id("sigrun")),
    );
    assert_eq!(focus_of(&moves[0]).0, "sigrun");
    director.apply_failed(&moves[0]);
    let moves = tick(&mut director, 200, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::ApplyFailed);

    // A direction that a later move already replaced is not retried.
    let (mut director, view) = focused_on("finn");
    director.end(10, Reason::Speaker, &id("finn")).expect("ok");
    let stale = tick(&mut director, 10, &view);
    let moves = ask(
        &mut director,
        1_000,
        &view,
        Request::focus(Reason::Speaker, id("sigrun")),
    );
    assert_eq!(focus_of(&moves[0]).2, 3);
    director.apply_failed(&stale[0]);
    assert!(tick(&mut director, 1_100, &view).is_empty());
    assert!(director.is_focused_on(&id("sigrun")));
}

#[test]
fn a_camera_moved_elsewhere_is_forgotten_so_the_next_focus_makes_a_move() {
    let (mut director, view) = focused_on("finn");
    assert!(
        !director
            .camera_moved_elsewhere(100, Some(&id("finn")))
            .expect("ok"),
        "the camera still looks at finn"
    );
    assert!(director.camera_moved_elsewhere(200, None).expect("ok"));
    assert!(!director.is_focused_on(&id("finn")));
    let moves = ask(
        &mut director,
        2_000,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    assert_eq!(focus_of(&moves[0]), ("finn", Reason::Speaker, 2));
}

#[test]
fn the_director_has_work_for_a_request_that_waits_in_the_inbox() {
    let mut director = director();
    let view = tavern();
    assert!(!director.has_work());
    director
        .request(0, &view, Request::focus(Reason::Speaker, id("finn")))
        .expect("well formed");
    assert!(director.has_work());
    tick(&mut director, 0, &view);
    assert!(director.has_work(), "the focus is held");
}

fn small(setting: DirectorSetting, value: u64) -> Director {
    let settings = DirectorSettings::default()
        .with(setting, value)
        .expect("small settings are valid");
    Director::new(settings)
}

#[test]
fn the_size_limits_are_settings_and_each_gives_its_typed_error() {
    let view = tavern();
    let mut director = small(DirectorSetting::MaximumTargets, 2);
    let three = Request::area(Reason::Hit, vec![id("finn"), id("sigrun"), id("brenna")]);
    assert_eq!(
        director.request(0, &view, three),
        Err(DirectorError::TooManyTargets { limit: 2 })
    );

    let mut director = small(DirectorSetting::MaximumWaiting, 2);
    for who in ["finn", "sigrun"] {
        director
            .request(0, &view, Request::focus(Reason::Speaker, id(who)))
            .expect("fits");
    }
    assert_eq!(
        director.request(0, &view, Request::focus(Reason::Speaker, id("brenna"))),
        Err(DirectorError::TooManyWaiting { limit: 2 })
    );
    tick(&mut director, 0, &view);
    director
        .request(100, &view, Request::focus(Reason::Hit, id("brenna")))
        .expect("the tick emptied the inbox");

    let mut director = small(DirectorSetting::MaximumWaitingTargets, 3);
    let two = || Request::area(Reason::Hit, vec![id("finn"), id("sigrun")]);
    director.request(0, &view, two()).expect("fits");
    assert_eq!(
        director.request(0, &view, two()),
        Err(DirectorError::TooManyWaitingTargets { limit: 3 })
    );
    director
        .request(0, &view, Request::focus(Reason::Hit, id("finn")))
        .expect("one more name fits");
    assert_eq!(
        director.request(0, &view, Request::focus(Reason::Hit, id("finn"))),
        Err(DirectorError::TooManyWaitingTargets { limit: 3 })
    );
    // Voiding the waiting requests frees their names.
    director.start_scene(1).expect("ok");
    director.request(2, &view, two()).expect("fits again");

    for setting in [
        DirectorSetting::MaximumTargets,
        DirectorSetting::MaximumWaiting,
        DirectorSetting::MaximumWaitingTargets,
        DirectorSetting::MaximumLogEntries,
    ] {
        assert_eq!(
            DirectorSettings::default().with(setting, 0).err(),
            Some(DirectorError::InvalidSetting { setting, value: 0 })
        );
    }
}

#[test]
fn a_small_log_evicts_the_oldest_entry_and_unread_returns_the_newest() {
    let mut director = small(DirectorSetting::MaximumLogEntries, 3);
    let view = tavern();
    for round in 1..=5_u64 {
        director
            .request(round, &view, Request::focus(Reason::Speaker, id("lurker")))
            .expect("well formed");
    }
    let report = director.report(InspectAudience::Operator, &view, usize::MAX);
    let sequences: Vec<u64> = report.log.iter().map(|entry| entry.sequence).collect();
    assert_eq!(sequences, [3, 4, 5]);
    assert_eq!(report.dropped, 5, "the counts keep what the log forgot");
    let unread: Vec<u64> = director
        .take_unread()
        .iter()
        .map(|entry| entry.sequence)
        .collect();
    assert_eq!(unread, [3, 4, 5]);
    assert_eq!(director.take_unread(), []);
    director
        .request(6, &view, Request::focus(Reason::Speaker, id("lurker")))
        .expect("well formed");
    let unread: Vec<u64> = director
        .take_unread()
        .iter()
        .map(|entry| entry.sequence)
        .collect();
    assert_eq!(unread, [6]);
}

#[test]
fn an_operator_wide_is_a_cut_under_reduced_effects_and_animates_otherwise() {
    let mut director = director();
    let mut view = tavern();
    let plain = director.pull_back(0, &view, None).expect("ok");
    assert_eq!(wide_of(&plain), (WideReason::Operator, 1_200, 1));
    view.reduced_effects = true;
    let cut = director.pull_back(1, &view, None).expect("ok");
    assert_eq!(wide_of(&cut), (WideReason::Operator, 0, 2));
    let asked = director.pull_back(2, &view, Some(500)).expect("ok");
    assert_eq!(wide_of(&asked).1, 500, "a time that was asked for stands");
}

#[test]
fn every_reason_round_trips_through_the_table_and_the_table_holds_one_spec_each() {
    let mut table = ReasonTable::default();
    assert_eq!(table.specs().len(), Reason::ALL.len());
    for (index, reason) in Reason::ALL.iter().enumerate() {
        let zoom = 110 + u16::try_from(index).expect("a few reasons");
        let spec = ReasonSpec::new(zoom, 0, Ends::AfterHold).expect("in range");
        table = table.with_spec(*reason, spec);
        assert_eq!(table.spec(*reason), spec);
    }
    let zooms: Vec<u16> = table.specs().iter().map(ReasonSpec::zoom_percent).collect();
    let mut distinct = zooms.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), Reason::ALL.len(), "{zooms:?}");
    assert_eq!(
        ReasonSpec::new(99, 0, Ends::AfterHold),
        Err(DirectorError::ZoomOutOfRange { zoom_percent: 99 })
    );
}

#[test]
fn a_direction_the_director_never_made_is_ignored_by_apply_failed() {
    let direction = StageDirection::Wide {
        reason: WideReason::Timeout,
        over_ms: 0,
        serial: 99,
        driven: false,
    };
    let mut director = director();
    director.apply_failed(&direction);
    assert!(
        !director.has_work(),
        "a serial the Director never made is ignored"
    );
}
