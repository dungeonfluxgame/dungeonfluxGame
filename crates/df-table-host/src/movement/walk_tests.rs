//! Tests of `walk`: the order and the times of a walk's pictures on a controlled clock, the lines
//! that say what happened, and the options a move is given.

use std::time::Duration;

use tokio::sync::mpsc::unbounded_channel;
use tokio::time::sleep;

use super::types::TownRole;
use super::walk::{
    DEFAULT_HOLD, MINIMUM_HOLD, Made, Moment, PLACE_TO_SIGHT, Screen, Sequencer, Shot, Skip, Step,
    WalkFailure, WalkOptions, arrived_line, conduct, shown_line, skipped_line,
};

const HOLD_MS: u64 = 6_000;

fn sequencer(stages: usize) -> Sequencer {
    Sequencer::new(stages, Duration::from_millis(HOLD_MS))
}

#[test]
fn a_hold_is_never_under_the_least_a_sight_can_be_taken_in() {
    let mut walk = Sequencer::new(1, Duration::from_millis(100));
    walk.made_stage(0, true);
    walk.made_arrival();

    assert_eq!(walk.step(0), Step::Stage(0));
    assert_eq!(walk.step(4_999), Step::Wait(Some(5_000)));
    assert_eq!(walk.step(5_000), Step::Arrival);
    assert_eq!(walk.step(5_000), Step::Done);
    assert!(DEFAULT_HOLD >= MINIMUM_HOLD);
}

#[test]
fn sights_go_up_in_order_each_after_the_one_before_has_been_up_its_hold() {
    let mut walk = sequencer(2);
    walk.made_stage(1, true);
    walk.made_stage(0, true);
    walk.made_arrival();

    assert_eq!(walk.step(1_000), Step::Stage(0));
    assert_eq!(walk.step(1_500), Step::Wait(Some(1_000 + HOLD_MS)));
    assert_eq!(walk.step(1_000 + HOLD_MS), Step::Stage(1));
    assert_eq!(
        walk.step(1_000 + HOLD_MS),
        Step::Wait(Some(1_000 + 2 * HOLD_MS))
    );
    assert_eq!(walk.step(1_000 + 2 * HOLD_MS), Step::Arrival);
}

#[test]
fn nothing_is_shown_until_a_plate_is_ready() {
    let mut walk = sequencer(2);
    assert_eq!(walk.step(0), Step::Wait(None));

    walk.made_stage(1, true);
    assert_eq!(
        walk.step(0),
        Step::Wait(None),
        "the first sight comes first"
    );
}

#[test]
fn the_place_is_never_up_before_a_sight_that_is_ready() {
    let mut walk = sequencer(2);
    walk.made_arrival();
    walk.made_stage(0, true);

    assert_eq!(walk.step(0), Step::Stage(0));
    assert_eq!(
        walk.step(10),
        Step::Wait(Some(HOLD_MS)),
        "the place waits for the sight that is up"
    );
    assert_eq!(
        walk.step(HOLD_MS),
        Step::Skipped {
            index: 1,
            why: Skip::Late
        }
    );
    assert_eq!(walk.step(HOLD_MS), Step::Arrival);
}

#[test]
fn a_sight_not_ready_when_the_place_is_ready_is_passed_over_rather_than_waited_for() {
    let mut walk = sequencer(2);
    walk.made_arrival();

    assert_eq!(
        walk.step(0),
        Step::Skipped {
            index: 0,
            why: Skip::Late
        }
    );
    assert_eq!(
        walk.step(0),
        Step::Skipped {
            index: 1,
            why: Skip::Late
        }
    );
    assert_eq!(walk.step(0), Step::Arrival);
}

#[test]
fn a_sight_that_comes_in_while_the_one_before_is_up_still_goes_up() {
    let mut walk = sequencer(2);
    walk.made_stage(0, true);
    walk.made_arrival();
    assert_eq!(walk.step(0), Step::Stage(0));

    assert_eq!(
        walk.step(2_000),
        Step::Wait(Some(HOLD_MS)),
        "the late sight has until the screen is free"
    );
    walk.made_stage(1, true);
    assert_eq!(walk.step(HOLD_MS), Step::Stage(1));
}

#[test]
fn a_sight_that_could_not_be_made_is_passed_over_at_once() {
    let mut walk = sequencer(2);
    walk.made_stage(0, false);
    walk.made_stage(1, true);

    assert_eq!(
        walk.step(0),
        Step::Skipped {
            index: 0,
            why: Skip::Failed
        }
    );
    assert_eq!(walk.step(0), Step::Stage(1));
}

#[test]
fn a_walk_with_no_sights_waits_for_the_place_and_shows_it() {
    let mut walk = sequencer(0);
    assert_eq!(walk.step(0), Step::Wait(None));
    walk.made_arrival();
    assert_eq!(walk.step(7), Step::Arrival);
}

#[test]
fn sights_that_land_together_while_the_place_is_pending_are_spread_over_the_expected_wait() {
    // The sights took 72 s to make, so the place is expected at 144 s: four sights share the 72 s.
    let first_up = 72_000;
    assert_eq!(first_up * PLACE_TO_SIGHT, 144_000);
    let mut walk = sequencer(4);
    for index in 0..4 {
        walk.made_stage(index, true);
    }

    assert_eq!(walk.step(first_up), Step::Stage(0));
    assert_eq!(walk.step(first_up + 1), Step::Wait(Some(90_000)));
    assert_eq!(walk.step(90_000), Step::Stage(1));
    assert_eq!(walk.step(108_000), Step::Stage(2));
    assert_eq!(walk.step(126_000), Step::Stage(3));
    assert_eq!(
        walk.step(144_000),
        Step::Wait(None),
        "the last sight stays up until the place is made"
    );
}

#[test]
fn a_place_that_comes_in_sooner_only_waits_the_plain_hold_of_the_sight_that_is_up() {
    let mut walk = sequencer(2);
    walk.made_stage(0, true);
    walk.made_stage(1, true);
    assert_eq!(walk.step(72_000), Step::Stage(0));
    assert_eq!(walk.step(73_000), Step::Wait(Some(108_000)));

    walk.made_arrival();

    assert_eq!(
        walk.step(73_000),
        Step::Wait(Some(72_000 + HOLD_MS)),
        "the stretched hold ends when the place is made"
    );
    assert_eq!(walk.step(72_000 + HOLD_MS), Step::Stage(1));
    assert_eq!(walk.step(72_000 + 2 * HOLD_MS), Step::Arrival);
}

#[test]
fn a_sight_that_lands_after_the_place_was_expected_is_held_only_its_plain_hold() {
    let mut walk = sequencer(3);
    walk.made_stage(0, true);
    assert_eq!(walk.step(72_000), Step::Stage(0));
    walk.made_stage(1, true);
    walk.made_stage(2, true);

    assert_eq!(walk.step(144_000), Step::Stage(1));
    assert_eq!(walk.step(144_001), Step::Wait(Some(144_000 + HOLD_MS)));
}

#[test]
fn sights_that_all_land_at_the_start_are_not_stretched() {
    let mut walk = sequencer(2);
    walk.made_stage(0, true);
    walk.made_stage(1, true);

    assert_eq!(walk.step(0), Step::Stage(0));
    assert_eq!(walk.step(1), Step::Wait(Some(HOLD_MS)));
    assert_eq!(walk.step(HOLD_MS), Step::Stage(1));
}

/// A screen that keeps what was put up and every moment noted.
#[derive(Default)]
struct Recorder {
    shown: Vec<(Shot, &'static str)>,
    moments: Vec<Moment>,
    fail: Option<Shot>,
}

impl Screen for Recorder {
    type Plate = &'static str;

    async fn show(&mut self, shot: Shot, plate: &'static str) -> Result<(), WalkFailure> {
        if self.fail == Some(shot) {
            return Err(WalkFailure {
                key: "backdrop".to_owned(),
                message: "the scene refused".to_owned(),
            });
        }
        self.shown.push((shot, plate));
        Ok(())
    }

    async fn note(&mut self, moment: Moment) {
        self.moments.push(moment);
    }
}

fn plates_failed(key: &str) -> WalkFailure {
    WalkFailure {
        key: key.to_owned(),
        message: "no picture".to_owned(),
    }
}

fn hold() -> Duration {
    Duration::from_millis(HOLD_MS)
}

fn ms(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[tokio::test(start_paused = true)]
async fn the_sights_go_up_as_they_are_ready_and_the_place_after_the_last_of_them() {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder::default();
    let plates = async {
        sleep(ms(10)).await;
        let _ = sender.send(Made::Stage(0, Ok("fountain")));
        sleep(ms(2)).await;
        let _ = sender.send(Made::Stage(1, Ok("shopfronts")));
        sleep(ms(18)).await;
        let _ = sender.send(Made::Arrival(Ok("square")));
    };

    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 2, hold()), plates);

    conducted.expect("the walk ends with the place");
    assert_eq!(
        screen.shown,
        [
            (Shot::Stage(0), "fountain"),
            (Shot::Stage(1), "shopfronts"),
            (Shot::Arrival, "square")
        ]
    );
    assert_eq!(
        screen.moments,
        [
            Moment::Shown {
                shot: Shot::Stage(0),
                at_ms: 10_000
            },
            Moment::Ended {
                stage: 0,
                up_ms: HOLD_MS
            },
            Moment::Shown {
                shot: Shot::Stage(1),
                at_ms: 16_000
            },
            Moment::Made { at_ms: 30_000 },
            Moment::Ended {
                stage: 1,
                up_ms: 14_000
            },
            Moment::Shown {
                shot: Shot::Arrival,
                at_ms: 30_000
            },
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn sights_that_land_together_are_spread_over_a_slow_place_and_none_is_up_under_the_least() {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder::default();
    let plates = async {
        sleep(ms(40)).await;
        for (index, plate) in ["lane", "archway", "cottages"].into_iter().enumerate() {
            let _ = sender.send(Made::Stage(index, Ok(plate)));
        }
        sleep(ms(160)).await;
        let _ = sender.send(Made::Arrival(Ok("square")));
    };

    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 3, hold()), plates);

    conducted.expect("the walk ends with the place");
    let up: Vec<(Shot, u64)> = screen
        .moments
        .iter()
        .filter_map(|moment| match moment {
            Moment::Shown { shot, at_ms } => Some((*shot, *at_ms)),
            _ => None,
        })
        .collect();
    assert_eq!(
        up,
        [
            (Shot::Stage(0), 40_000),
            (Shot::Stage(1), 53_333),
            (Shot::Stage(2), 66_666),
            (Shot::Arrival, 200_000)
        ],
        "three sights share the 40 s from their landing to the expected place at 80 s"
    );
    let held: Vec<u64> = screen
        .moments
        .iter()
        .filter_map(|moment| match moment {
            Moment::Ended { up_ms, .. } => Some(*up_ms),
            _ => None,
        })
        .collect();
    assert_eq!(held, [13_333, 13_333, 133_334]);
    assert!(held.iter().all(|up_ms| *up_ms >= HOLD_MS));
}

#[tokio::test(start_paused = true)]
async fn a_place_that_comes_in_while_sights_are_spread_goes_up_after_the_plain_holds() {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder::default();
    let plates = async {
        sleep(ms(40)).await;
        for (index, plate) in ["lane", "archway", "cottages"].into_iter().enumerate() {
            let _ = sender.send(Made::Stage(index, Ok(plate)));
        }
        sleep(ms(10)).await;
        let _ = sender.send(Made::Arrival(Ok("square")));
    };

    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 3, hold()), plates);

    conducted.expect("the walk ends with the place");
    let up: Vec<(Shot, u64)> = screen
        .moments
        .iter()
        .filter_map(|moment| match moment {
            Moment::Shown { shot, at_ms } => Some((*shot, *at_ms)),
            _ => None,
        })
        .collect();
    assert_eq!(
        up,
        [
            (Shot::Stage(0), 40_000),
            (Shot::Stage(1), 50_000),
            (Shot::Stage(2), 56_000),
            (Shot::Arrival, 62_000)
        ],
        "the place made at 50 s ends the spreading: the sights go up on their plain holds"
    );
}

#[tokio::test(start_paused = true)]
async fn the_place_waits_for_a_ready_sight_to_be_up_its_whole_hold() {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder::default();
    let plates = async {
        sleep(ms(1)).await;
        let _ = sender.send(Made::Stage(0, Ok("fountain")));
        sleep(ms(1)).await;
        let _ = sender.send(Made::Arrival(Ok("square")));
    };

    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 1, hold()), plates);

    conducted.unwrap();
    let times: Vec<(Shot, u64)> = screen
        .moments
        .iter()
        .filter_map(|moment| match moment {
            Moment::Shown { shot, at_ms } => Some((*shot, *at_ms)),
            _ => None,
        })
        .collect();
    assert_eq!(
        times,
        [(Shot::Stage(0), 1_000), (Shot::Arrival, 1_000 + HOLD_MS)],
        "the place is ready at 2 s and goes up when the sight has been up its hold"
    );
}

#[tokio::test(start_paused = true)]
async fn a_late_sight_is_skipped_and_never_keeps_the_place_from_arriving() {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder::default();
    let plates = async {
        sleep(ms(10)).await;
        let _ = sender.send(Made::Stage(0, Ok("fountain")));
        sleep(ms(10)).await;
        let _ = sender.send(Made::Arrival(Ok("square")));
        sleep(ms(20)).await;
        let _ = sender.send(Made::Stage(1, Ok("too late")));
    };

    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 2, hold()), plates);

    conducted.unwrap();
    assert_eq!(
        screen.shown,
        [(Shot::Stage(0), "fountain"), (Shot::Arrival, "square")]
    );
    assert!(screen.moments.contains(&Moment::Skipped {
        index: 1,
        why: Skip::Late,
        detail: String::new()
    }));
    assert!(screen.moments.contains(&Moment::Shown {
        shot: Shot::Arrival,
        at_ms: 20_000
    }));
}

#[tokio::test(start_paused = true)]
async fn a_sight_that_failed_is_skipped_with_what_failed_and_the_walk_goes_on() {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder::default();
    let plates = async {
        sleep(ms(5)).await;
        let _ = sender.send(Made::Stage(0, Err(plates_failed("generate"))));
        let _ = sender.send(Made::Arrival(Ok("square")));
    };

    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 1, hold()), plates);

    conducted.unwrap();
    assert_eq!(screen.shown, [(Shot::Arrival, "square")]);
    assert!(screen.moments.contains(&Moment::Skipped {
        index: 0,
        why: Skip::Failed,
        detail: "generate: no picture".to_owned()
    }));
}

#[tokio::test(start_paused = true)]
async fn a_sight_the_scene_would_not_take_is_skipped_and_the_walk_goes_on() {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder {
        fail: Some(Shot::Stage(0)),
        ..Recorder::default()
    };
    let plates = async {
        let _ = sender.send(Made::Stage(0, Ok("fountain")));
        sleep(ms(1)).await;
        let _ = sender.send(Made::Arrival(Ok("square")));
    };

    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 1, hold()), plates);

    conducted.unwrap();
    assert_eq!(screen.shown, [(Shot::Arrival, "square")]);
    assert!(matches!(
        screen.moments.first(),
        Some(Moment::Skipped {
            index: 0,
            why: Skip::Failed,
            ..
        })
    ));
}

#[tokio::test(start_paused = true)]
async fn a_place_that_cannot_be_made_ends_the_walk_with_its_failure() {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder::default();
    let plates = async {
        sleep(ms(1)).await;
        let _ = sender.send(Made::Stage(0, Ok("fountain")));
        sleep(ms(1)).await;
        let _ = sender.send(Made::Arrival(Err(plates_failed("generate"))));
    };

    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 1, hold()), plates);

    assert_eq!(conducted, Err(plates_failed("generate")));
    assert_eq!(screen.shown, [(Shot::Stage(0), "fountain")]);
}

#[tokio::test(start_paused = true)]
async fn plates_that_stop_before_the_place_is_made_end_the_walk() {
    let (sender, receiver) = unbounded_channel::<Made<&'static str>>();
    drop(sender);
    let mut screen = Recorder::default();

    let conducted = conduct(&mut screen, receiver, 1, hold()).await;

    assert_eq!(conducted.unwrap_err().key, "plates_stopped");
}

#[test]
fn the_lines_say_which_sight_when_and_for_how_long() {
    assert_eq!(
        shown_line("stage 1/2 a stone fountain", Some(61_250), 6_020),
        "walk stage 1/2 a stone fountain shown_at_ms=61250 up_ms=6020"
    );
    assert_eq!(
        shown_line("stage 1/2 a stone fountain", None, 6_020),
        "walk stage 1/2 a stone fountain shown_at_ms=- up_ms=6020"
    );
    assert_eq!(
        skipped_line("stage 2/2 a row of shopfronts", Skip::Late, ""),
        "walk stage 2/2 a row of shopfronts skipped reason=late"
    );
    assert_eq!(
        skipped_line(
            "stage 2/2 a row of shopfronts",
            Skip::Failed,
            "generate: no picture"
        ),
        "walk stage 2/2 a row of shopfronts skipped reason=failed (generate: no picture)"
    );
    assert_eq!(
        arrived_line(1, 2, 6_100),
        "walk arrived stages=1/2 walk_ms=6100"
    );
}

#[test]
fn a_move_asks_a_walk_only_when_it_has_sights_or_a_town_role() {
    assert_eq!(WalkOptions::default().ask(Some("roam-01")), None);

    let sights = WalkOptions {
        stages: 2,
        ..WalkOptions::default()
    };
    let ask = sights.ask(Some("Roam-01")).unwrap();
    assert_eq!(
        (ask.stages, ask.town, ask.move_id.as_str()),
        (2, TownRole::Within, "roam-01")
    );

    let leaving = WalkOptions {
        town: Some(TownRole::Leaves),
        ..WalkOptions::default()
    };
    assert_eq!(leaving.ask(Some("roam-02")).unwrap().town, TownRole::Leaves);
}

#[test]
fn a_move_with_no_plate_is_only_looked_at_and_changes_no_town() {
    let seeding = WalkOptions {
        stages: 2,
        town: Some(TownRole::Seeds),
        ..WalkOptions::default()
    };

    let ask = seeding.ask(None).unwrap();

    assert_eq!(ask.town, TownRole::Within);
    assert_eq!(ask.stages, 2);
}

/// Runs a two-sight walk where each plate comes in at the given second, in the given order, and
/// the channel closes when the last has come (as the live producers drop theirs).
async fn run_two(
    events: Vec<(u64, Made<&'static str>)>,
) -> (Result<(), WalkFailure>, Recorder, Vec<(Shot, u64)>) {
    let (sender, receiver) = unbounded_channel();
    let mut screen = Recorder::default();
    let plates = async move {
        let mut now = 0;
        for (at, made) in events {
            sleep(ms(at - now)).await;
            now = at;
            let _ = sender.send(made);
        }
    };
    let (conducted, ()) = tokio::join!(conduct(&mut screen, receiver, 2, hold()), plates);
    let shown = screen
        .moments
        .iter()
        .filter_map(|moment| match moment {
            Moment::Shown { shot, at_ms } => Some((*shot, *at_ms)),
            _ => None,
        })
        .collect();
    (conducted, screen, shown)
}

#[tokio::test(start_paused = true)]
async fn the_walk_waits_out_the_hold_after_every_plate_has_come_and_the_stream_is_closed() {
    // The live failure: both sights and the place come in, the senders are gone, and the sight
    // that is up still has most of its hold to run.
    let (conducted, screen, shown) = run_two(vec![
        (1, Made::Stage(0, Ok("walk1"))),
        (1, Made::Stage(1, Ok("walk2"))),
        (1, Made::Arrival(Ok("place"))),
    ])
    .await;

    conducted.expect("a closed stream with every plate in is no failure");
    assert_eq!(
        shown,
        [
            (Shot::Stage(0), 1_000),
            (Shot::Stage(1), 1_000 + HOLD_MS),
            (Shot::Arrival, 1_000 + 2 * HOLD_MS)
        ],
        "each sight is up its whole hold and the walk ends on the place"
    );
    assert_eq!(screen.shown.last(), Some(&(Shot::Arrival, "place")));
}

#[tokio::test(start_paused = true)]
async fn a_later_sight_made_before_an_earlier_one_waits_its_turn() {
    // Run 7 move 5: walk2 stored before walk1, the place last.
    let (conducted, _, shown) = run_two(vec![
        (3, Made::Stage(1, Ok("walk2"))),
        (4, Made::Stage(0, Ok("walk1"))),
        (6, Made::Arrival(Ok("place"))),
    ])
    .await;

    conducted.unwrap();
    assert_eq!(
        shown,
        [
            (Shot::Stage(0), 4_000),
            (Shot::Stage(1), 4_000 + HOLD_MS),
            (Shot::Arrival, 4_000 + 2 * HOLD_MS)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_place_made_before_the_first_sight_still_ends_the_walk() {
    // The place comes in first and the sights a second later: the late sights are passed over
    // and the place is shown, never dropped.
    let (conducted, screen, shown) = run_two(vec![
        (1, Made::Arrival(Ok("place"))),
        (2, Made::Stage(1, Ok("walk2"))),
        (2, Made::Stage(0, Ok("walk1"))),
    ])
    .await;

    conducted.unwrap();
    assert_eq!(shown, [(Shot::Arrival, 1_000)]);
    assert_eq!(screen.shown, [(Shot::Arrival, "place")]);
}

#[tokio::test(start_paused = true)]
async fn sights_and_the_place_that_come_in_together_are_all_taken_in_before_any_is_passed_over() {
    let (conducted, screen, shown) = run_two(vec![
        (5, Made::Arrival(Ok("place"))),
        (5, Made::Stage(1, Ok("walk2"))),
        (5, Made::Stage(0, Ok("walk1"))),
    ])
    .await;

    conducted.unwrap();
    assert_eq!(
        shown,
        [
            (Shot::Stage(0), 5_000),
            (Shot::Stage(1), 5_000 + HOLD_MS),
            (Shot::Arrival, 5_000 + 2 * HOLD_MS)
        ]
    );
    assert!(
        !screen
            .moments
            .iter()
            .any(|moment| matches!(moment, Moment::Skipped { .. }))
    );
}

#[tokio::test(start_paused = true)]
async fn the_first_sight_is_up_its_whole_hold_though_the_second_and_the_place_are_ready() {
    let (conducted, screen, _) = run_two(vec![
        (2, Made::Stage(0, Ok("walk1"))),
        (2, Made::Stage(1, Ok("walk2"))),
        (3, Made::Arrival(Ok("place"))),
    ])
    .await;

    conducted.unwrap();
    assert!(screen.moments.contains(&Moment::Ended {
        stage: 0,
        up_ms: HOLD_MS
    }));
    assert!(screen.moments.contains(&Moment::Ended {
        stage: 1,
        up_ms: HOLD_MS
    }));
}

#[tokio::test(start_paused = true)]
async fn a_sight_that_never_comes_is_passed_over_when_the_stream_closes_and_the_place_arrives() {
    let (conducted, screen, shown) = run_two(vec![
        (1, Made::Stage(0, Ok("walk1"))),
        (2, Made::Arrival(Ok("place"))),
    ])
    .await;

    conducted.unwrap();
    assert_eq!(
        shown,
        [(Shot::Stage(0), 1_000), (Shot::Arrival, 1_000 + HOLD_MS)]
    );
    assert!(screen.moments.contains(&Moment::Skipped {
        index: 1,
        why: Skip::Failed,
        detail: "no plate came for it".to_owned()
    }));
}

#[tokio::test(start_paused = true)]
async fn a_stream_that_closes_before_the_place_says_which_plate_is_in_which_state() {
    let (conducted, screen, _) = run_two(vec![(1, Made::Stage(0, Ok("walk1")))]).await;

    let failure = conducted.unwrap_err();
    assert_eq!(failure.key, "plates_stopped");
    assert!(
        failure
            .message
            .ends_with("stage 1 passed, stage 2 pending, place pending"),
        "{}",
        failure.message
    );
    assert_eq!(screen.shown, [(Shot::Stage(0), "walk1")]);
}

#[test]
fn the_sequencer_reports_where_each_plate_stands() {
    let mut walk = sequencer(3);
    walk.made_stage(0, true);
    walk.made_stage(1, false);
    assert_eq!(walk.step(0), Step::Stage(0));
    assert_eq!(
        walk.report(),
        "stage 1 passed, stage 2 failed, stage 3 pending, place pending"
    );
    assert_eq!(walk.give_up_stages(), [2]);
    walk.made_arrival();
    assert!(walk.arrival_ready());
}
