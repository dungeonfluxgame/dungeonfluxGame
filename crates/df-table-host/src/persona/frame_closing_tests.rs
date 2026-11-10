//! Every frame that starts ends with exactly one closing moment (`shown`, `failed` or `dropped`),
//! and the plate timer the page reads counts the frames in flight, so the toolbar's "generating"
//! never counts a frame that is over (the owner's "the timer is broken").

use super::frame::{
    DeclaredTiming, DroppedWhy, FrameMoment, FrameReason, FrameTimingReply, FrameTimingRequest,
    PERSONA_FRAME_SCHEDULE, PERSONA_FRAME_TIMING,
};
use super::plate::{RoomTurn, SharedScreen, begin_declaration, claim_room, not_shown};
use crate::dev_client::post_json;
use crate::dev_http::DevReply;
use crate::test_support::{Running, STARTER_FILES, start};
use df_playground::controls::PlateTimer;

async fn moment(
    running: &Running,
    frame: &str,
    moment: FrameMoment,
    reason: FrameReason,
    dropped: Option<DroppedWhy>,
) {
    let request = FrameTimingRequest {
        frame: frame.to_owned(),
        moment,
        reason,
        took_ms: 10,
        declared: None,
        revealed: None,
        dropped,
    };
    let timed: FrameTimingReply = post_json(running.port, PERSONA_FRAME_TIMING, &request)
        .await
        .unwrap();
    assert!(matches!(timed, DevReply::Done { .. }));
}

fn plate(running: &Running) -> PlateTimer {
    running.state.playground().snapshot().controls.plate
}

fn lines(running: &Running, containing: &str) -> Vec<String> {
    running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with("PERSONA frame_timing") && text.contains(containing))
        .collect()
}

#[tokio::test]
async fn frames_made_together_count_two_then_one_then_none() {
    let running = start(&STARTER_FILES).await;
    assert!(plate(&running).in_flight.is_empty() && !plate(&running).making);
    moment(
        &running,
        "talk-1",
        FrameMoment::Started,
        FrameReason::Declared,
        None,
    )
    .await;
    assert_eq!(plate(&running).in_flight, ["talk-1"]);
    moment(
        &running,
        "talk-2",
        FrameMoment::Started,
        FrameReason::Line,
        None,
    )
    .await;
    assert_eq!(
        plate(&running).in_flight,
        ["talk-1", "talk-2"],
        "two in flight"
    );
    assert!(plate(&running).making);
    // The first goes up: the second is still being made, so the timer still says so (a single
    // boolean cleared it here).
    moment(
        &running,
        "talk-1",
        FrameMoment::Shown,
        FrameReason::Declared,
        None,
    )
    .await;
    assert_eq!(plate(&running).in_flight, ["talk-2"], "then one");
    assert!(plate(&running).making);
    // A frame closing twice, or a frame that never started closing, changes nothing.
    moment(
        &running,
        "talk-1",
        FrameMoment::Shown,
        FrameReason::Declared,
        None,
    )
    .await;
    moment(
        &running,
        "talk-9",
        FrameMoment::Dropped,
        FrameReason::Line,
        Some(DroppedWhy::Overtaken),
    )
    .await;
    assert_eq!(plate(&running).in_flight, ["talk-2"]);
    // The second finishes without going up: it closes with `dropped`, and the timer is idle.
    moment(
        &running,
        "talk-2",
        FrameMoment::Dropped,
        FrameReason::Line,
        Some(DroppedWhy::Overtaken),
    )
    .await;
    let after = plate(&running);
    assert!(after.in_flight.is_empty() && !after.making, "then none");
    assert_eq!(
        after.last.map(|last| (last.frame, last.moment)),
        Some(("talk-2".to_owned(), FrameMoment::Dropped)),
        "the newest moment of any frame"
    );
    let dropped = lines(&running, "moment=dropped frame=talk-2 ");
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(
        dropped[0].contains(" reason=line ") && dropped[0].contains(" why=overtaken"),
        "{dropped:?}"
    );
}

#[tokio::test]
async fn a_dropped_or_redone_frame_closes_once_and_the_page_is_told_how_many() {
    let running = start(&STARTER_FILES).await;
    let schedule = |running_now: bool| df_json_schedule(running_now);
    let told: FrameTimingReply = post_json(running.port, PERSONA_FRAME_SCHEDULE, &schedule(true))
        .await
        .unwrap();
    assert!(matches!(told, DevReply::Done { .. }));
    // A declared plate superseded by a later declaration closes with `dropped`.
    moment(
        &running,
        "talk-3",
        FrameMoment::Started,
        FrameReason::Line,
        None,
    )
    .await;
    moment(
        &running,
        "talk-4",
        FrameMoment::Started,
        FrameReason::Declared,
        None,
    )
    .await;
    assert!(
        plate(&running).shown_text(0).contains("2 in flight"),
        "{}",
        plate(&running).shown_text(0)
    );
    moment(
        &running,
        "talk-3",
        FrameMoment::Dropped,
        FrameReason::Line,
        Some(DroppedWhy::Superseded),
    )
    .await;
    // A declared plate whose first attempt showed too many people and was made again is one frame:
    // it started once and closes once, with the second attempt.
    let redone = FrameTimingRequest {
        frame: "talk-4".to_owned(),
        moment: FrameMoment::Shown,
        reason: FrameReason::Declared,
        took_ms: 130_000,
        declared: Some(DeclaredTiming {
            who: "danny".to_owned(),
            superseded: 1,
            queue_wait_ms: 20,
            made_ms: 64_000,
            base: "fresh".to_owned(),
            reason: "extra_figures".to_owned(),
            attempt: 2,
            first_made_ms: Some(65_000),
            rejected: None,
        }),
        revealed: None,
        dropped: None,
    };
    let timed: FrameTimingReply = post_json(running.port, PERSONA_FRAME_TIMING, &redone)
        .await
        .unwrap();
    assert!(matches!(timed, DevReply::Done { .. }));
    assert!(
        plate(&running).in_flight.is_empty(),
        "{:?}",
        plate(&running).in_flight
    );
    for frame in ["talk-3", "talk-4"] {
        let closing: Vec<String> = ["shown", "failed", "dropped"]
            .iter()
            .flat_map(|moment| lines(&running, &format!("moment={moment} frame={frame} ")))
            .collect();
        assert_eq!(closing.len(), 1, "{frame} closes once: {closing:?}");
    }
    assert!(
        lines(&running, "moment=dropped frame=talk-3 ")[0].ends_with(" why=superseded"),
        "{:?}",
        lines(&running, "moment=dropped")
    );
    // A watch that stops clears whatever was left.
    moment(
        &running,
        "talk-5",
        FrameMoment::Started,
        FrameReason::Line,
        None,
    )
    .await;
    let told: FrameTimingReply = post_json(running.port, PERSONA_FRAME_SCHEDULE, &schedule(false))
        .await
        .unwrap();
    assert!(matches!(told, DevReply::Done { .. }));
    assert!(plate(&running).in_flight.is_empty() && !plate(&running).making);
}

fn df_json_schedule(running: bool) -> super::frame::FrameScheduleRequest {
    super::frame::FrameScheduleRequest {
        running,
        every_ms: 60_000,
        due_in_ms: running.then_some(60_000),
        missed: 0,
    }
}

#[tokio::test]
async fn a_plate_that_finishes_without_going_up_says_why() {
    let screen = SharedScreen::default();
    assert!(claim_room(&screen, 1, false).await);
    let turn = |number| RoomTurn {
        number,
        screen: screen.clone(),
        cast_from: None,
        cued: false,
        emergency: None,
    };
    // A newer plate took its slot: overtaken.
    let overtaken = not_shown(&turn(1), "talk-1", true, None);
    assert!(!overtaken.on_screen);
    assert_eq!(overtaken.dropped, Some(DroppedWhy::Overtaken));
    // A declaration made after it started: superseded, and the declared plate itself is not.
    begin_declaration(&screen, 3, "talk-3");
    let superseded = not_shown(&turn(2), "talk-2", true, None);
    assert_eq!(superseded.dropped, Some(DroppedWhy::Superseded));
    assert!(superseded.summary.ends_with("superseded by talk-3"));
}
