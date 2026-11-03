//! The Director's pace and priority rules with a controlled clock (O.14 camera rule 5): spacing,
//! the order of reasons, bursts, areas and the "look at me" request.

use crate::director::{Receipt, Request, StageDirection};
use crate::director_log::{DropReason, Outcome};
use crate::director_reason::{Reason, WideReason};
use crate::director_tests::{ask, director, focus_of, id, lines, tavern, tick, wide_of};
use crate::inspect::InspectAudience;

#[test]
fn finn_then_sigrun_back_to_back_pans_on_without_pulling_out() {
    let mut director = director();
    let view = tavern();
    let first = ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("finn"))
            .zoom(250)
            .over(1_500),
    );
    assert_eq!(focus_of(&first[0]), ("finn", Reason::Speaker, 1));
    // The next speaker comes at once: it waits out the 800 ms, then pans straight on.
    director
        .request(
            100,
            &view,
            Request::focus(Reason::Speaker, id("sigrun"))
                .zoom(250)
                .over(1_500),
        )
        .expect("well formed");
    let mut moves = Vec::new();
    for now in [100, 400, 799, 800] {
        moves.extend(tick(&mut director, now, &view));
    }
    assert_eq!(
        moves.len(),
        1,
        "one move, and no pull-back between: {moves:?}"
    );
    assert_eq!(focus_of(&moves[0]), ("sigrun", Reason::Speaker, 2));
    assert!(
        moves
            .iter()
            .all(|direction| matches!(direction, StageDirection::Focus { .. }))
    );
}

#[test]
fn at_most_one_new_move_in_800_ms_unless_the_new_reason_outranks_the_focus() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Arrival, id("guard")),
    );
    // A speaker outranks an arrival, so it may start at once.
    let moves = ask(
        &mut director,
        50,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    assert_eq!(focus_of(&moves[0]), ("finn", Reason::Speaker, 2));
    // An equal reason inside the spacing waits for it ...
    let moves = ask(
        &mut director,
        100,
        &view,
        Request::focus(Reason::Speaker, id("sigrun")),
    );
    assert!(moves.is_empty());
    assert!(tick(&mut director, 849, &view).is_empty());
    let moves = tick(&mut director, 850, &view);
    assert_eq!(focus_of(&moves[0]), ("sigrun", Reason::Speaker, 3));
    // ... and the pull-back of a hold's end does not wait for spacing.
    director
        .end(900, Reason::Speaker, &id("sigrun"))
        .expect("ok");
    let moves = tick(&mut director, 900, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::ReasonEnded);
}

#[test]
fn a_hit_interrupts_a_speaker_focus_and_a_speaker_never_interrupts_a_hit() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    let moves = ask(
        &mut director,
        10,
        &view,
        Request::focus(Reason::Hit, id("finn")),
    );
    assert_eq!(focus_of(&moves[0]), ("finn", Reason::Hit, 2));
    assert_eq!(
        director
            .request(20, &view, Request::focus(Reason::Speaker, id("guard")))
            .expect("well formed"),
        Receipt::Dropped(DropReason::LowerPriority)
    );
    // The hold of the hit stays on Finn whatever happens below it.
    assert!(tick(&mut director, 5_000, &view).is_empty());
    assert!(director.is_focused_on(&id("finn")));
    // Once the result card is over, the focus ends and a speaker may have the camera.
    director.end(5_100, Reason::Hit, &id("finn")).expect("ok");
    assert_eq!(
        wide_of(&tick(&mut director, 5_100, &view)[0]).0,
        WideReason::ReasonEnded
    );
    let moves = ask(
        &mut director,
        6_000,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    assert_eq!(focus_of(&moves[0]).0, "guard");
}

#[test]
fn a_burst_of_three_directions_in_one_tick_leaves_one_move_to_the_highest_reason() {
    let mut director = director();
    let view = tavern();
    for (reason, who) in [
        (Reason::Arrival, "guard"),
        (Reason::Speaker, "finn"),
        (Reason::Spotlight, "sigrun"),
    ] {
        director
            .request(0, &view, Request::focus(reason, id(who)))
            .expect("well formed");
    }
    let moves = tick(&mut director, 0, &view);
    assert_eq!(moves.len(), 1);
    assert_eq!(focus_of(&moves[0]), ("finn", Reason::Speaker, 1));
    // The two that lost are dropped with a reason, in the log.
    let dropped: Vec<DropReason> = director
        .take_unread()
        .iter()
        .filter_map(|entry| match entry.outcome {
            Outcome::Dropped { why } => Some(why),
            _ => None,
        })
        .collect();
    assert_eq!(
        dropped,
        [DropReason::LowerPriority, DropReason::LowerPriority]
    );
}

#[test]
fn among_equal_reasons_in_a_burst_the_latest_wins_and_one_spread_over_ticks_waits_its_turn() {
    let mut director = director();
    let view = tavern();
    for who in ["finn", "sigrun", "brenna"] {
        director
            .request(0, &view, Request::focus(Reason::Speaker, id(who)))
            .expect("well formed");
    }
    let moves = tick(&mut director, 0, &view);
    assert_eq!(focus_of(&moves[0]), ("brenna", Reason::Speaker, 1));
    let coalesced = director
        .take_unread()
        .iter()
        .filter(|entry| {
            matches!(
                entry.outcome,
                Outcome::Dropped {
                    why: DropReason::Coalesced
                }
            )
        })
        .count();
    assert_eq!(coalesced, 2);
    // Three more spread over 600 ms (inside the spacing): the latest of them moves at 800.
    for (now, who) in [(100, "finn"), (300, "sigrun"), (600, "guard")] {
        director
            .request(now, &view, Request::focus(Reason::Speaker, id(who)))
            .expect("well formed");
        assert!(tick(&mut director, now, &view).is_empty());
    }
    let moves = tick(&mut director, 800, &view);
    assert_eq!(moves.len(), 1);
    assert_eq!(focus_of(&moves[0]), ("guard", Reason::Speaker, 2));
}

#[test]
fn a_later_equal_direction_replaces_the_one_waiting_and_only_one_ever_waits() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    ask(
        &mut director,
        100,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    // The same reason arriving while one waits replaces it: the latest of equals.
    ask(
        &mut director,
        200,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    let report = director.report(InspectAudience::Operator, &view, 100);
    assert_eq!(
        report.waiting.as_ref().map(|waiting| waiting.reason),
        Some(Reason::Speaker)
    );
    assert_eq!(
        report.waiting.and_then(|waiting| waiting.element),
        Some(id("finn"))
    );
}

#[test]
fn three_heroes_hit_by_one_area_leave_the_camera_wide() {
    let mut director = director();
    let view = tavern();
    // On a stage the camera has not zoomed on, three hits of one result move nothing.
    for who in ["finn", "sigrun", "brenna"] {
        director
            .request(0, &view, Request::focus(Reason::Hit, id(who)).result(9))
            .expect("well formed");
    }
    assert!(tick(&mut director, 0, &view).is_empty());
    assert_eq!(director.serial(), 0);
    let areas = director
        .take_unread()
        .iter()
        .filter(|entry| {
            matches!(
                entry.outcome,
                Outcome::Dropped {
                    why: DropReason::Area
                }
            )
        })
        .count();
    assert_eq!(areas, 3);
    // Held on a speaker, the camera pulls out and stays out.
    ask(
        &mut director,
        5_000,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    for who in ["finn", "sigrun", "brenna"] {
        director
            .request(
                6_000,
                &view,
                Request::focus(Reason::Hit, id(who)).result(10),
            )
            .expect("well formed");
    }
    let moves = tick(&mut director, 6_000, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::Area);
    assert!(!director.is_focused_on(&id("guard")));
    // One request that names three heroes is the same area.
    ask(
        &mut director,
        9_000,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    let area = Request::area(Reason::Hit, vec![id("finn"), id("sigrun")]);
    let moves = ask(&mut director, 9_900, &view, area);
    assert_eq!(wide_of(&moves[0]).0, WideReason::Area);
    // Two hits of one result on the same hero are not an area.
    let one = Request::focus(Reason::Hit, id("finn")).result(11);
    director
        .request(12_000, &view, one.clone())
        .expect("well formed");
    director.request(12_000, &view, one).expect("well formed");
    assert_eq!(focus_of(&tick(&mut director, 12_000, &view)[0]).0, "finn");
}

#[test]
fn look_at_me_during_a_speaker_focus_is_dropped_and_on_a_quiet_stage_it_focuses_the_hero() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    assert_eq!(
        director
            .request(2_000, &view, Request::focus(Reason::Requested, id("finn")))
            .expect("well formed"),
        Receipt::Dropped(DropReason::LowerPriority)
    );
    let logged = lines(&mut director, InspectAudience::Operator);
    assert_eq!(
        logged.last().map(String::as_str),
        Some("CAMERA dropped reason=lower_priority direction=requested element=finn")
    );
    director
        .end(3_000, Reason::Speaker, &id("guard"))
        .expect("ok");
    tick(&mut director, 3_000, &view);
    let moves = ask(
        &mut director,
        4_000,
        &view,
        Request::focus(Reason::Requested, id("finn")),
    );
    assert_eq!(focus_of(&moves[0]), ("finn", Reason::Requested, 3));
    assert!(matches!(
        moves[0],
        StageDirection::Focus {
            zoom_percent: 180,
            over_ms: 1_200,
            ..
        }
    ));
}

#[test]
fn the_same_creature_for_the_same_reason_renews_the_hold_and_makes_no_new_move() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Reveal, id("guard")),
    );
    let moves = ask(
        &mut director,
        7_000,
        &view,
        Request::focus(Reason::Reveal, id("guard")),
    );
    assert!(moves.is_empty());
    assert_eq!(director.serial(), 1);
    // The hold now runs 8 s from the renewal.
    assert!(tick(&mut director, 14_999, &view).is_empty());
    assert_eq!(
        wide_of(&tick(&mut director, 15_000, &view)[0]).0,
        WideReason::Timeout
    );
    let dropped = director.take_unread().iter().any(|entry| {
        matches!(
            entry.outcome,
            Outcome::Dropped {
                why: DropReason::AlreadyFocused
            }
        )
    });
    assert!(dropped);
}

#[test]
fn a_direction_waiting_for_a_reason_that_ended_is_void() {
    let mut director = director();
    let view = tavern();
    ask(
        &mut director,
        0,
        &view,
        Request::focus(Reason::Speaker, id("guard")),
    );
    ask(
        &mut director,
        100,
        &view,
        Request::focus(Reason::Speaker, id("finn")),
    );
    assert_eq!(
        director
            .report(InspectAudience::Operator, &view, 10)
            .waiting
            .map(|waiting| waiting.reason),
        Some(Reason::Speaker)
    );
    // The line ended before the move began: the waiting speaker is dropped, and the held one goes home.
    director.end(300, Reason::Speaker, &id("finn")).expect("ok");
    director
        .end(300, Reason::Speaker, &id("guard"))
        .expect("ok");
    assert_eq!(
        director
            .report(InspectAudience::Operator, &view, 10)
            .waiting,
        None
    );
    let moves = tick(&mut director, 300, &view);
    assert_eq!(wide_of(&moves[0]).0, WideReason::ReasonEnded);
    let ended = director.take_unread().iter().any(|entry| {
        matches!(
            entry.outcome,
            Outcome::Dropped {
                why: DropReason::ReasonEnded
            }
        )
    });
    assert!(ended);
}

#[test]
fn a_move_is_never_made_by_a_clock_the_director_reads_itself() {
    // Every call carries the time; two directors given the same calls make the same moves.
    let run = || {
        let mut director = director();
        let view = tavern();
        let mut out = Vec::new();
        for (now, reason, who) in [
            (0, Reason::Speaker, "guard"),
            (30, Reason::Hit, "finn"),
            (900, Reason::Arrival, "sigrun"),
        ] {
            out.extend(ask(
                &mut director,
                now,
                &view,
                Request::focus(reason, id(who)),
            ));
        }
        out.extend(tick(&mut director, 20_000, &view));
        out
    };
    assert_eq!(run(), run());
    assert!(!run().is_empty());
}
