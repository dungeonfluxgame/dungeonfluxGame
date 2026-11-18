//! A seat's roll: the tapped check reaches the judge as the narrator's intent, the host rolls it
//! with the seat's sheet, and the phone is answered with the numbers, or with why none was called.
//!
//! TODO(DEV-12): the route over a seat with a built hero (`creation::sheet_file`) on both
//! listeners, and a roll `about` a thing the place offers.

use std::sync::Arc;

use df_playground::fixture::RollOutcome;
use df_playground::path::PLAY_SEAT_ROLL;
use df_playground::seat::wire::{SeatCheck, SeatRoll, SeatRolled};
use serde_json::{Value, json};

use super::fixtures::{
    VirtualClock, athletics_answer, dice_showing, line_answer, routes_over, sheet, table,
    trivial_answer, with,
};
use super::request::{CheckError, CheckRouteError};
use super::seat_roll::{roll_for_seat, seat_router};
use crate::dev_client::post_json;
use crate::dev_http::DevReply;

fn narrator(answer: Value, reason: &str) -> Value {
    with(answer, json!({"initiator": "narrator", "reason": reason}))
}

fn athletics() -> SeatRoll {
    SeatRoll {
        check: SeatCheck::Skill {
            skill: "athletics".to_owned(),
        },
        about: None,
    }
}

#[tokio::test]
async fn a_tapped_check_is_judged_as_the_narrators_intent_and_rolled_by_the_host() {
    let table = table(vec![
        narrator(athletics_answer(), "narrator_obstacle"),
        line_answer("The wall is slick.", "You pull yourself over."),
        line_answer("The wall is slick.", "You pull yourself over."),
    ])
    .await;
    let routes = routes_over(&table, dice_showing(&[14]));
    let clock = VirtualClock::over(table.app());
    let rolled = roll_for_seat(&routes, &clock, (3, sheet()), &athletics())
        .await
        .expect("the roll runs");
    assert_eq!(
        rolled,
        SeatRolled::Rolled {
            roll: 14,
            modifier: 5,
            total: 19,
            dc: Some(15),
            outcome: Some(RollOutcome::Success),
        }
    );
    let judged = table.facts_of(0);
    assert_eq!(judged["narrator_intent"]["skill"], "athletics", "{judged}");
    assert_eq!(
        table.lines_starting_with("SEAT rolled"),
        ["SEAT rolled seat=3 check=skill:athletics total=19 dc=15 outcome=success"]
    );
}

#[tokio::test]
async fn a_check_that_does_not_fit_the_scene_is_answered_as_not_called() {
    let table = table(vec![narrator(trivial_answer(), "intent_unsuitable")]).await;
    let routes = routes_over(&table, dice_showing(&[]));
    let clock = VirtualClock::over(table.app());
    let save = SeatRoll {
        check: SeatCheck::Save {
            ability: "con".to_owned(),
        },
        about: None,
    };
    let rolled = roll_for_seat(&routes, &clock, (3, sheet()), &save)
        .await
        .expect("the judge answers");
    assert_eq!(
        rolled,
        SeatRolled::NotCalled {
            reason: "intent_unsuitable".to_owned()
        }
    );
    assert_eq!(
        table.lines_starting_with("SEAT roll_skipped"),
        ["SEAT roll_skipped seat=3 check=save:con reason=intent_unsuitable"]
    );
}

#[tokio::test]
async fn a_check_the_rules_do_not_have_or_a_thing_not_offered_is_refused_before_any_call() {
    let table = table(Vec::new()).await;
    let routes = routes_over(&table, dice_showing(&[]));
    let clock = VirtualClock::over(table.app());
    let unknown = SeatRoll {
        check: SeatCheck::Skill {
            skill: "juggling".to_owned(),
        },
        about: None,
    };
    let refused = roll_for_seat(&routes, &clock, (3, sheet()), &unknown).await;
    assert_eq!(
        refused,
        Err(CheckRouteError::BadRequest {
            cause: CheckError::UnknownCheck {
                key: "juggling".to_owned()
            }
        })
    );
    let elsewhere = SeatRoll {
        about: Some("dragon".to_owned()),
        ..athletics()
    };
    let refused = roll_for_seat(&routes, &clock, (3, sheet()), &elsewhere).await;
    assert_eq!(
        refused,
        Err(CheckRouteError::BadRequest {
            cause: CheckError::UnknownThing {
                id: "dragon".to_owned()
            }
        })
    );
    assert_eq!(table.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_roll_before_the_check_runner_started_is_refused_as_not_open() {
    // A host state whose persona routes never started: no check runner.
    let directory = tempfile::TempDir::new().expect("makes a directory");
    let log = Arc::new(crate::event_log::EventLog::default());
    let state = Arc::new(crate::state::AppState::new(
        df_table::canvas::Canvas::new(df_table::testkit::registry_from(&[])),
        crate::observe::HostEventSink::new(log, Arc::new(crate::observe::WallClock)),
        None,
        directory.path().to_path_buf(),
        crate::playground::PlaygroundFiles::new(
            directory.path().join("assets"),
            directory.path().join("pkg"),
        ),
    ));
    let listener = crate::server::bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = seat_router(state);
    tokio::spawn(async move {
        // The server runs until the test's runtime stops; its end is the test's end.
        let _ = axum::serve(listener, app).await;
    });
    let reply: DevReply<SeatRolled, CheckRouteError> =
        post_json(port, PLAY_SEAT_ROLL, &athletics()).await.unwrap();
    let DevReply::Rejected { error, message } = reply else {
        panic!("a roll with no runner was answered: {reply:?}");
    };
    assert_eq!(
        error,
        CheckRouteError::BadRequest {
            cause: CheckError::ChecksNotStarted
        }
    );
    assert!(message.contains("Rolls are not open yet."), "{message}");
}

#[tokio::test]
async fn the_same_check_again_at_the_same_place_reaches_the_judge_as_already_rolled() {
    let table = table(vec![
        narrator(athletics_answer(), "narrator_obstacle"),
        line_answer("The wall is slick.", "You pull yourself over."),
        line_answer("The wall is slick.", "You pull yourself over."),
        narrator(trivial_answer(), "already_checked"),
    ])
    .await;
    let routes = routes_over(&table, dice_showing(&[3]));
    let clock = VirtualClock::over(table.app());
    roll_for_seat(&routes, &clock, (3, sheet()), &athletics())
        .await
        .expect("the first roll runs");
    let again = roll_for_seat(&routes, &clock, (3, sheet()), &athletics())
        .await
        .expect("the judge answers");
    assert_eq!(
        again,
        SeatRolled::NotCalled {
            reason: "already_checked".to_owned()
        }
    );
    let told = table.facts_of(3);
    assert_eq!(told["already_checked"][0]["skill"], "athletics", "{told}");
}
