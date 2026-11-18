//! A seat action is done once by its `said` id: a copy sent while it is being done is refused
//! `in_flight` without running, and a request cut off mid-way still finishes and keeps its outcome
//! for the phone's retry.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use df_playground::pages::wire::BuyDone;
use df_playground::seat::wire::{SeatAction, SeatOutcome};
use tokio::sync::Notify;

use super::once;
use crate::seat::request::{SeatActionError, SeatError};
use crate::state::AppState;
use crate::test_support::{STARTER_FILES, start};

fn buy_action() -> SeatAction {
    SeatAction::Buy {
        said: 7,
        shop: "smith".to_owned(),
        item: "dagger".to_owned(),
        qty: 1,
    }
}

fn bought() -> SeatOutcome {
    SeatOutcome::Bought {
        buy: BuyDone {
            item: "dagger".to_owned(),
            qty: 1,
            price: "2 gp".to_owned(),
            weight: "1 lb".to_owned(),
            capacity: "150 lb".to_owned(),
        },
    }
}

/// A purchase that says when it starts, waits for `go`, then charges once.
fn purchase(
    charges: &Arc<AtomicU32>,
    started: &Arc<Notify>,
    go: &Arc<Notify>,
) -> impl Future<Output = Result<SeatOutcome, SeatActionError>> + Send + 'static {
    let (charges, started, go) = (Arc::clone(charges), Arc::clone(started), Arc::clone(go));
    async move {
        started.notify_one();
        go.notified().await;
        charges.fetch_add(1, Ordering::SeqCst);
        Ok(bought())
    }
}

/// A purchase that must never run: a repeat or a copy is answered without it.
fn never_run(
    charges: &Arc<AtomicU32>,
) -> impl Future<Output = Result<SeatOutcome, SeatActionError>> + Send + 'static {
    let charges = Arc::clone(charges);
    async move {
        charges.fetch_add(100, Ordering::SeqCst);
        Ok(bought())
    }
}

/// `seat`'s answer to a retry of `action`, once the copy being done has finished.
async fn retried(state: &Arc<AppState>, charges: &Arc<AtomicU32>) -> SeatOutcome {
    for _ in 0..1_000 {
        match once(state, 1, buy_action(), never_run(charges)).await {
            Err(SeatActionError::Seat(SeatError::InFlight { said: 7 })) => {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            answer => return answer.expect("the stored outcome"),
        }
    }
    panic!("the purchase never finished");
}

#[tokio::test]
async fn two_copies_of_one_purchase_sent_at_once_charge_once() {
    let running = start(&STARTER_FILES).await;
    let state = Arc::clone(&running.state);
    let (charges, started, go) = (
        Arc::new(AtomicU32::new(0)),
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
    );
    let first = {
        let (state, work) = (Arc::clone(&state), purchase(&charges, &started, &go));
        tokio::spawn(async move { once(&state, 1, buy_action(), work).await })
    };
    started.notified().await;
    // The copy comes while the first is being done: refused, and it never runs.
    let copy = once(&state, 1, buy_action(), never_run(&charges)).await;
    assert_eq!(
        copy,
        Err(SeatActionError::Seat(SeatError::InFlight { said: 7 }))
    );
    go.notify_one();
    assert_eq!(first.await.expect("the first finishes"), Ok(bought()));
    // A repeat after it is answered as before.
    assert_eq!(retried(&state, &charges).await, bought());
    assert_eq!(charges.load(Ordering::SeqCst), 1);
    let log = running
        .state
        .log()
        .newest(1_000)
        .iter()
        .map(|event| event.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        log.contains("SEAT action seat=1 action=buy repeat=no"),
        "{log}"
    );
    assert!(
        log.contains("SEAT action seat=1 action=buy repeat=yes"),
        "{log}"
    );
}

#[tokio::test]
async fn a_purchase_cut_off_at_its_deadline_finishes_and_its_retry_gets_the_stored_outcome() {
    let running = start(&STARTER_FILES).await;
    let state = Arc::clone(&running.state);
    let (charges, started, go) = (
        Arc::new(AtomicU32::new(0)),
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
    );
    // The player listener's deadline drops the request while the purchase is being done.
    let cut = tokio::time::timeout(
        Duration::from_millis(20),
        once(&state, 1, buy_action(), purchase(&charges, &started, &go)),
    )
    .await;
    assert!(cut.is_err(), "the request was cut off");
    started.notified().await;
    go.notify_one();
    assert_eq!(retried(&state, &charges).await, bought());
    assert_eq!(charges.load(Ordering::SeqCst), 1);
}
