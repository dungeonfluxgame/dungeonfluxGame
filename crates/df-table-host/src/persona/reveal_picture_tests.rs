//! `@show <picture> | <caption> | <seconds>` and `persona show-picture`: a picture the library
//! already holds cut to as it is. The screen's rules are tested with short real slots (the screen
//! keeps the host's own clock); the whole show runs against a host's scene routes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::cli::{PersonaCommand, parse};
use super::frame::{
    FrameMoment, FrameReason, FrameTimingReply, FrameTimingRequest, PERSONA_FRAME_TIMING,
    RevealRefusal, RevealedTiming,
};
use super::frame_watch::{DEFAULT_REVEAL_HOLD_SECONDS, WatchInput, show_picture};
use super::plate::{
    SharedScreen, begin_declaration, claim_after_reveal, claim_declared, claim_reveal, claim_room,
    queue_reveal,
};
use crate::dev_client::{get_json, post_json};
use crate::dev_http::DevReply;
use crate::test_support::{Running, STARTER_FILES, layout_of_cast, start};
use crate::wire::{ControlsReply, route};

const SLOT: Duration = Duration::from_millis(300);

#[test]
fn a_show_input_is_a_picture_a_caption_and_the_seconds() {
    let show = |entry| WatchInput::parse(entry);
    assert_eq!(
        show("@show painting-3.png | Study of a blue door | 12"),
        WatchInput::Show {
            picture: "painting-3.png",
            caption: "Study of a blue door",
            hold: Some(12)
        }
    );
    assert_eq!(
        show("@show painting-3.png | Study of a blue door"),
        WatchInput::Show {
            picture: "painting-3.png",
            caption: "Study of a blue door",
            hold: None
        },
        "no seconds: the default, {DEFAULT_REVEAL_HOLD_SECONDS} s, is the watch's"
    );
    assert_eq!(
        show("@show a.png | one | two | 5"),
        WatchInput::Show {
            picture: "a.png",
            caption: "one | two",
            hold: Some(5)
        },
        "a caption may hold the separator"
    );
    assert_eq!(
        show("@show a.png | soon | later"),
        WatchInput::Show {
            picture: "a.png",
            caption: "soon | later",
            hold: None
        },
        "a last field that is no number is part of the caption"
    );
    for odd in ["@show ", "@show a.png", "@show  | caption | 3"] {
        assert_eq!(show(odd), WatchInput::Unreadable, "{odd}");
    }
}

#[test]
fn persona_show_picture_reads_the_picture_the_room_the_caption_and_the_hold() {
    let words = |text: &str| -> Vec<String> { text.split(' ').map(str::to_owned).collect() };
    let PersonaCommand::ShowPicture {
        picture,
        room,
        caption,
        hold,
    } = parse(&words("show-picture study.png room=wall.png hold=4")).unwrap()
    else {
        panic!("a show-picture command");
    };
    assert_eq!(
        (picture.as_str(), room.as_str(), hold),
        ("study.png", "wall.png", 4)
    );
    assert_eq!(caption, None);
    let PersonaCommand::ShowPicture { hold, caption, .. } = parse(&[
        "show-picture".to_owned(),
        "study.png".to_owned(),
        "room=wall.png".to_owned(),
        "caption=A small study".to_owned(),
    ])
    .unwrap() else {
        panic!("a show-picture command");
    };
    assert_eq!(hold, DEFAULT_REVEAL_HOLD_SECONDS);
    assert_eq!(caption.as_deref(), Some("A small study"));
    for odd in [
        "show-picture",
        "show-picture study.png",
        "show-picture a.png room=b.png x",
    ] {
        assert!(parse(&words(odd)).is_err(), "{odd}");
    }
}

#[test]
fn a_revealed_picture_is_an_insert_of_its_own_reason() {
    assert_eq!(FrameReason::Reveal.key(), "reveal");
    assert!(FrameReason::Reveal.is_insert());
    assert_eq!(RevealRefusal::UnknownPicture.key(), "unknown_picture");
    assert_eq!(RevealRefusal::InvalidName.key(), "invalid_name");
}

// The screen.

#[tokio::test]
async fn a_revealed_picture_is_shown_at_once_holds_the_screen_and_drops_no_waiting_plate() {
    let screen = SharedScreen::default();
    screen.lock().unwrap().room_every = SLOT;
    assert!(claim_room(&screen, 1, false).await);
    // A plate is ready and waits for its slot.
    let waiting = {
        let screen = screen.clone();
        tokio::spawn(async move {
            let began = Instant::now();
            let up = claim_room(&screen, 2, false).await;
            (up, began.elapsed())
        })
    };
    tokio::time::sleep(SLOT / 6).await;
    let began = Instant::now();
    let mut ticket = queue_reveal(&screen);
    claim_reveal(&screen, &mut ticket, Duration::from_millis(500)).await;
    assert!(
        began.elapsed() < SLOT / 2,
        "no wait for the plate's slot: {:?}",
        began.elapsed()
    );
    let (up, waited) = waiting.await.unwrap();
    assert!(
        up,
        "a plate waiting for its slot is not dropped: it is no new picture of the talk"
    );
    assert!(
        waited >= Duration::from_millis(450),
        "it waits out the revealed picture's hold: {waited:?}"
    );
}

#[tokio::test]
async fn a_revealed_picture_still_waits_the_least_time_the_picture_before_it_stays_up() {
    let screen = SharedScreen::default();
    screen.lock().unwrap().minimum_shown = Duration::from_millis(250);
    assert!(claim_room(&screen, 1, false).await);
    let began = Instant::now();
    let mut ticket = queue_reveal(&screen);
    claim_reveal(&screen, &mut ticket, Duration::from_millis(50)).await;
    assert!(
        began.elapsed() >= Duration::from_millis(200),
        "nothing changes faster than a person can follow: {:?}",
        began.elapsed()
    );
}

// The whole show, against a host's scene routes.

async fn host_with_the_pictures() -> (Running, u16) {
    let running = start(&STARTER_FILES).await;
    running.write_asset("media/painting.png", "painting.png");
    running.write_asset("media/room.png", "room.png");
    crate::locate::write_layout(
        &running.playground_path("assets/layouts"),
        "room.png",
        &layout_of_cast("room.png", &["danny"]),
    )
    .unwrap();
    // A scene to change: a fixture whose backdrop is the room.
    running.write_asset(
        "fixtures/one.json",
        concat!(
            r#"{"name": "Roadside", "description": "A lane.", "#,
            r#""tv": {"place": "Here", "status": {"kind": "ok"}, "elements": [], "#,
            r#""backdrop": {"media": "room.png", "caption": null}}, "#,
            r#""phone": {"hero": "Pip", "status": {"kind": "ok"}}}"#
        ),
    );
    let app = crate::server::router(Arc::clone(&running.state));
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        // The server runs until the test's runtime stops.
        let _ = axum::serve(listener, app).await;
    });
    (running, port)
}

/// The picture behind the scene and the caption bar's words, as the scene says now.
async fn on_screen(port: u16) -> (Option<String>, Option<String>) {
    let controls: ControlsReply = get_json(port, route::PLAYGROUND_CONTROLS).await.unwrap();
    (
        controls
            .scene
            .backdrop
            .set_value()
            .map(|image| image.media.clone()),
        controls
            .scene
            .roleplay
            .set_value()
            .and_then(|card| card.caption.clone()),
    )
}

fn lines_of(running: &Running, start: &str) -> Vec<String> {
    running
        .state
        .log()
        .after_sequence(0, usize::MAX)
        .into_iter()
        .map(|line| line.text)
        .filter(|text| text.starts_with(start))
        .collect()
}

#[tokio::test]
async fn a_picture_is_shown_with_its_caption_for_its_hold_and_the_room_returns() {
    let (running, port) = host_with_the_pictures().await;
    let began = Instant::now();
    let show = tokio::spawn(async move {
        show_picture(
            port,
            "painting.png",
            "room.png",
            Some("A study of a door"),
            Duration::from_millis(900),
        )
        .await
    });
    // While it is held: the picture is behind the scene and the caption is on the bar.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (backdrop, caption) = on_screen(port).await;
    assert_eq!(backdrop.as_deref(), Some("painting.png"), "{backdrop:?}");
    assert_eq!(caption.as_deref(), Some("A study of a door"));
    let said = show.await.unwrap().expect("shown");
    assert!(
        began.elapsed() >= Duration::from_millis(900),
        "held for its whole hold"
    );
    assert!(said.contains("the room is back"), "{said}");
    // After: the room is back and the caption that was there (none) with it.
    let (backdrop, caption) = on_screen(port).await;
    assert_eq!(backdrop.as_deref(), Some("room.png"));
    assert_eq!(caption.as_deref().unwrap_or(""), "");
    let timing = lines_of(&running, "PERSONA frame_timing ");
    let moment = |name: &str| {
        timing.iter().any(|line| {
            line.starts_with(&format!("PERSONA frame_timing moment={name} "))
                && line.contains(" reason=reveal ")
        })
    };
    assert!(
        moment("started") && moment("shown") && moment("ended"),
        "{timing:?}"
    );
    assert!(
        lines_of(&running, "PERSONA revealed ")
            .iter()
            .any(|line| line == "PERSONA revealed picture=painting.png hold_ms=900"),
        "{:?}",
        lines_of(&running, "PERSONA")
    );
}

#[tokio::test]
async fn a_picture_the_library_does_not_hold_is_refused_with_a_typed_reason_and_logged() {
    let (running, port) = host_with_the_pictures().await;
    let refused = show_picture(
        port,
        "nowhere.png",
        "room.png",
        Some("x"),
        Duration::from_millis(100),
    )
    .await
    .expect_err("an unknown picture is refused");
    assert!(refused.to_string().contains("unknown_picture"), "{refused}");
    let invalid = show_picture(port, "../x y", "room.png", None, Duration::from_millis(100)).await;
    assert!(invalid.is_err());
    assert_eq!(on_screen(port).await.0, None, "nothing changed on screen");
    let lines = lines_of(&running, "PERSONA");
    assert!(
        lines
            .iter()
            .any(|line| line
                == "PERSONA refused verb=reveal picture=nowhere.png error=unknown_picture"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(
            |line| line.starts_with("PERSONA refused verb=reveal picture=- ")
                && line.ends_with(" error=invalid_name")
        ),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(
            |line| line.starts_with("PERSONA frame_timing moment=failed ")
                && line.contains(" reason=reveal ")
        ),
        "{lines:?}"
    );
    assert!(lines_of(&running, "PERSONA revealed ").is_empty());
}

#[tokio::test]
async fn the_timing_route_writes_the_revealed_line_at_the_shown_moment_only() {
    let (running, port) = host_with_the_pictures().await;
    let timing = |moment| FrameTimingRequest {
        frame: "show-1".to_owned(),
        moment,
        reason: FrameReason::Reveal,
        took_ms: 5,
        declared: None,
        revealed: Some(RevealedTiming {
            picture: "painting.png".to_owned(),
            hold_ms: 10_000,
            refused: None,
            behind: 2,
        }),
        dropped: None,
    };
    for moment in [FrameMoment::Started, FrameMoment::Shown, FrameMoment::Ended] {
        let timed: FrameTimingReply = post_json(port, PERSONA_FRAME_TIMING, &timing(moment))
            .await
            .unwrap();
        assert!(matches!(timed, DevReply::Done { .. }));
    }
    assert_eq!(
        lines_of(&running, "PERSONA revealed "),
        ["PERSONA revealed picture=painting.png hold_ms=10000"]
    );
    assert_eq!(
        lines_of(&running, "PERSONA reveal_queued "),
        ["PERSONA reveal_queued picture=painting.png behind=2"],
        "said once, when it is asked for behind others"
    );
}

// The sequence of revealed pictures.

/// When each of `holds`, asked for in this order on `screen`, was granted the screen, with the
/// ticket's `behind`.
async fn granted(screen: &SharedScreen, holds: &[u64]) -> Vec<(Instant, usize)> {
    let tickets: Vec<_> = holds.iter().map(|_| queue_reveal(screen)).collect();
    let tasks: Vec<_> = tickets
        .into_iter()
        .zip(holds)
        .map(|(mut ticket, hold)| {
            let (screen, hold) = (screen.clone(), Duration::from_millis(*hold));
            let behind = ticket.behind;
            tokio::spawn(async move {
                claim_reveal(&screen, &mut ticket, hold).await;
                (Instant::now(), behind)
            })
        })
        .collect();
    let mut grants = Vec::new();
    for task in tasks {
        grants.push(task.await.unwrap());
    }
    grants
}

#[tokio::test]
async fn two_revealed_pictures_in_a_row_hold_their_whole_times_in_order_and_the_room_waits_no_more()
{
    let screen = SharedScreen::default();
    // The least time a generated picture stays up is longer than the second hold (the owner's 8 s
    // against a 6 s hold, scaled).
    screen.lock().unwrap().minimum_shown = Duration::from_millis(400);
    assert!(claim_room(&screen, 1, false).await);
    let began = Instant::now();
    let grants = granted(&screen, &[500, 300]).await;
    assert_eq!(
        grants.iter().map(|grant| grant.1).collect::<Vec<_>>(),
        [0, 1]
    );
    let (first, second) = (grants[0].0, grants[1].0);
    assert!(
        first - began >= Duration::from_millis(380),
        "the first replaces a generated picture after its least time: {:?}",
        first - began
    );
    let between = second - first;
    assert!(
        between >= Duration::from_millis(500) && between < Duration::from_millis(600),
        "the second waits for the first's whole 500 ms hold, no more, no less: {between:?}"
    );
    // The first does not put the room back over the second.
    assert!(!claim_after_reveal(&screen, first).await);
    // The second was asked to hold 300 ms, shorter than the least time: the room is back at 300 ms.
    tokio::time::sleep(
        (second + Duration::from_millis(300)).saturating_duration_since(Instant::now()),
    )
    .await;
    let back = Instant::now();
    assert!(claim_after_reveal(&screen, second).await);
    assert!(
        back.elapsed() < Duration::from_millis(60),
        "an explicit short hold is not stretched to the least time: {:?}",
        back.elapsed()
    );
    assert!(
        back - second < Duration::from_millis(400),
        "the room is back before the least time of a generated picture would allow"
    );
}

#[tokio::test]
async fn three_queued_revealed_pictures_are_shown_one_after_another_in_order() {
    let screen = SharedScreen::default();
    let grants = granted(&screen, &[250, 200, 150]).await;
    assert_eq!(
        grants.iter().map(|grant| grant.1).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    let gaps = [grants[1].0 - grants[0].0, grants[2].0 - grants[1].0];
    assert!(
        gaps[0] >= Duration::from_millis(250) && gaps[0] < Duration::from_millis(350),
        "{gaps:?}"
    );
    assert!(
        gaps[1] >= Duration::from_millis(200) && gaps[1] < Duration::from_millis(300),
        "{gaps:?}"
    );
}

#[tokio::test]
async fn twelve_queued_revealed_pictures_are_all_taken_in_order_each_held_its_whole_time() {
    let screen = SharedScreen::default();
    let hold = Duration::from_millis(40);
    let grants = granted(&screen, &[40; 12]).await;
    assert_eq!(
        grants.iter().map(|grant| grant.1).collect::<Vec<_>>(),
        (0..12).collect::<Vec<_>>(),
        "every one is given a place, each behind all the ones before it"
    );
    for pair in grants.windows(2) {
        let between = pair[1].0 - pair[0].0;
        assert!(
            between >= hold && between < hold + Duration::from_millis(60),
            "each is held its whole time, in order: {between:?}"
        );
    }
}

#[tokio::test]
async fn a_revealed_picture_given_up_without_being_served_holds_none_behind_it() {
    let screen = SharedScreen::default();
    let mut tickets: Vec<_> = (0..6).map(|_| queue_reveal(&screen)).collect();
    drop(tickets.remove(0));
    for mut ticket in tickets {
        claim_reveal(&screen, &mut ticket, Duration::ZERO).await;
    }
}

#[tokio::test]
async fn an_emergency_plate_goes_ahead_of_older_room_plates_but_after_twelve_revealed_pictures() {
    let screen = SharedScreen::default();
    let tickets: Vec<_> = (0..12).map(|_| queue_reveal(&screen)).collect();
    let began = Instant::now();
    let room = {
        let screen = screen.clone();
        tokio::spawn(async move { claim_room(&screen, 3, false).await })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    begin_declaration(&screen, 5, "danny-plate");
    let declared = {
        let screen = screen.clone();
        tokio::spawn(async move {
            let up = claim_declared(&screen, 5, Duration::from_millis(20)).await;
            (up, began.elapsed())
        })
    };
    for mut ticket in tickets {
        claim_reveal(&screen, &mut ticket, Duration::from_millis(30)).await;
    }
    assert!(
        !room.await.unwrap(),
        "the room plate started before the declaration gives way to it"
    );
    let (up, waited) = declared.await.unwrap();
    assert!(
        up && waited >= Duration::from_millis(350),
        "it takes its turn after the whole sequence, the last hold included: {waited:?}"
    );
}

#[tokio::test]
async fn a_declared_plate_and_a_room_plate_wait_for_the_whole_sequence_of_revealed_pictures() {
    let screen = SharedScreen::default();
    let mut first = queue_reveal(&screen);
    let mut second = queue_reveal(&screen);
    let began = Instant::now();
    let declared = {
        let screen = screen.clone();
        tokio::spawn(async move {
            let up = claim_declared(&screen, 5, Duration::from_millis(50)).await;
            (up, began.elapsed())
        })
    };
    let room = {
        let screen = screen.clone();
        tokio::spawn(async move {
            let up = claim_room(&screen, 6, false).await;
            (up, began.elapsed())
        })
    };
    claim_reveal(&screen, &mut first, Duration::from_millis(300)).await;
    claim_reveal(&screen, &mut second, Duration::from_millis(300)).await;
    let (up, waited) = declared.await.unwrap();
    assert!(up);
    assert!(
        waited >= Duration::from_millis(580),
        "the emergency plate takes its turn after both pictures' whole holds: {waited:?}"
    );
    let (up, waited) = room.await.unwrap();
    assert!(up && waited >= Duration::from_millis(580), "{waited:?}");
}
