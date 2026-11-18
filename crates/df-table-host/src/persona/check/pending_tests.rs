//! A seated hero rolls their own dice: the call is said, the roll waits on the seat with no
//! deadline, and only the seat's tap rolls it; a seat that leaves withdraws it unrolled. A
//! character no seat plays rolls at once, as before.
//!
//! TODO(DEV-12): two check runs asked at once take the table's turn one after the other.

use std::sync::Arc;
use std::time::Duration;

use super::fixtures::{
    Table, VirtualClock, WARRIOR, athletics_answer, dice_showing, judge_request, line_answer,
    routes_over, table,
};
use super::flow::run_check;
use super::pending::PendingRollError;
use super::request::{CheckRun, CheckRunRequest, Initiator};
use super::routes::CheckRoutes;
use crate::seat::book::Holder;
use crate::seat::key::KeyHash;

fn answers() -> Vec<serde_json::Value> {
    let line = line_answer("The rock is cold.", "The ledge holds.");
    vec![athletics_answer(), line.clone(), line]
}

fn request() -> CheckRunRequest {
    CheckRunRequest {
        judge: judge_request(Initiator::Player, Some("I climb the ledge.")),
        speak: true,
        inspiration: None,
        told: None,
        dc: None,
        tool: None,
    }
}

/// Seat 2 plays the warrior.
async fn seat_the_warrior(table: &Table) {
    let mut open = table.app().seats().open().await.unwrap();
    let books = open.books();
    let key = KeyHash::of("seat-key-for-the-pending-roll-test");
    books.seat(&key, 2, Holder::Guest, "Ann", 0);
    let card = books.add_hero(2, "Corwen", "human", "fighter").unwrap();
    books.choose_hero(&key, card.id).unwrap();
    books.bind_persona(2, Some(WARRIOR)).unwrap();
}

async fn run_with(routes: &CheckRoutes, table: &Table) -> CheckRun {
    let clock = VirtualClock::over(table.app());
    run_check(routes, &clock, &request())
        .await
        .expect("the check runs")
}

#[tokio::test]
async fn a_seated_heros_roll_waits_for_the_players_tap_and_then_the_host_rolls() {
    let table = table(answers()).await;
    seat_the_warrior(&table).await;
    let routes = Arc::new(routes_over(&table, dice_showing(&[14])));
    let tapper = Arc::clone(&routes);
    let tap = tokio::spawn(async move {
        for _ in 0..200 {
            if let Some(pending) = tapper.pending.of(2) {
                assert_eq!(pending.title, "Athletics, Strength");
                assert_eq!(pending.skill.as_deref(), Some("athletics"));
                assert_eq!(pending.target.as_ref().map(|target| target.value), Some(15));
                assert_eq!(
                    tapper.pending.answer(2, "a-roll-long-gone"),
                    Err(PendingRollError::StaleRoll {
                        roll_id: "a-roll-long-gone".to_owned()
                    })
                );
                tapper.pending.answer(2, &pending.roll_id).unwrap();
                return pending.roll_id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no roll waited for seat 2");
    });
    let run = run_with(&routes, &table).await;
    let roll_id = tap.await.unwrap();
    let rolled = run.rolled.expect("the roll was called");
    assert_eq!(
        rolled.view.id, roll_id,
        "the card is the roll the seat tapped"
    );
    assert_eq!(rolled.view.total, 19);
    let checks: Vec<String> = table
        .lines_starting_with("CHECK ")
        .iter()
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_owned))
        .collect();
    let at = |kind: &str| checks.iter().position(|seen| seen == kind).unwrap();
    assert!(at("called") < at("awaiting_roll") && at("roll_tapped") < at("rolled"));
    assert!(
        table.lines_starting_with("CHECK awaiting_roll")[0]
            .starts_with(&format!("CHECK awaiting_roll seat=2 roll_id={roll_id}"))
    );
    assert!(
        table.lines_starting_with("CHECK roll_tapped")[0]
            .starts_with("CHECK roll_tapped seat=2 waited_ms=")
    );
    assert_eq!(routes.pending.of(2), None, "the roll no longer waits");
}

#[tokio::test]
async fn a_roll_nobody_taps_keeps_waiting_and_the_host_never_rolls_for_the_player() {
    let table = table(answers()).await;
    seat_the_warrior(&table).await;
    let mut routes = routes_over(&table, dice_showing(&[14]));
    routes.roll_watch = Duration::from_millis(20);
    let routes = Arc::new(routes);
    let tapper = Arc::clone(&routes);
    // Many watches pass with no tap: the roll still waits, and only the tap rolls it.
    let tap = async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let pending = tapper.pending.of(2).expect("the roll still waits");
        tapper.pending.answer(2, &pending.roll_id).unwrap();
    };
    let (run, ()) = tokio::join!(run_with(&routes, &table), tap);
    assert_eq!(run.rolled.expect("the roll was called").view.total, 19);
    let waited = table.lines_starting_with("CHECK roll_waited ");
    assert!(waited.len() >= 3, "{waited:?}");
    assert!(waited[0].starts_with("CHECK roll_waited seat=2 roll_id="));
    assert!(
        table
            .lines_starting_with("CHECK roll_waited_out")
            .is_empty()
    );
    assert_eq!(table.lines_starting_with("CHECK roll_tapped").len(), 1);
}

#[tokio::test]
async fn a_seat_that_leaves_withdraws_its_roll_and_nobody_rolls_in_its_place() {
    let table = table(answers()).await;
    seat_the_warrior(&table).await;
    let mut routes = routes_over(&table, dice_showing(&[14]));
    routes.roll_watch = Duration::from_millis(20);
    let leave = async {
        while routes.pending.of(2).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let mut open = table.app().seats().open().await.unwrap();
        open.books().free(2);
    };
    let clock = VirtualClock::over(table.app());
    let asked = request();
    let (run, ()) = tokio::join!(run_check(&routes, &clock, &asked), leave);
    let Err(refused) = run else {
        panic!("a check whose roller left was rolled");
    };
    assert!(refused.to_string().contains("nobody rolls"), "{refused}");
    assert!(
        table.lines_starting_with("CHECK roll_withdrawn")[0]
            .starts_with("CHECK roll_withdrawn seat=2 roll_id=")
    );
    assert!(table.lines_starting_with("CHECK rolled").is_empty());
    assert_eq!(routes.pending.of(2), None, "the roll no longer waits");
}

#[tokio::test]
async fn a_character_no_seat_plays_is_rolled_at_once() {
    let table = table(answers()).await;
    let routes = routes_over(&table, dice_showing(&[14]));
    run_with(&routes, &table).await;
    assert!(table.lines_starting_with("CHECK awaiting_roll").is_empty());
    assert_eq!(
        routes.pending.answer(2, "any"),
        Err(PendingRollError::NoPendingRoll { seat: 2 })
    );
}
