use std::sync::Mutex;

use super::*;
use crate::gameplay::table_log::LineExit as Exit;

struct FakeJev(Result<usize, JevUnavailable>);

impl Jev for FakeJev {
    fn forecast<'a>(
        &'a self,
        state: &'a serde_json::Value,
        choices: &'a [String],
    ) -> ChoiceFuture<'a> {
        assert!(state["place_id"].is_string());
        assert_eq!(
            choices.last().unwrap(),
            "likely | no large arena (flat view)"
        );
        Box::pin(async { self.0 })
    }
}

fn shop() -> Place {
    Place {
        id: "shop".to_owned(),
        facts: vec!["A small cloth shop".to_owned()],
        exits: vec![Exit {
            id: "door".to_owned(),
            place_id: "street".to_owned(),
            description: "The street outside the shop".to_owned(),
        }],
    }
}

#[tokio::test]
async fn large_hall_requests_here_once_per_place_per_run() {
    let foresight = FightForesight::new(Box::new(FakeJev(Ok(1))));
    let requests = Mutex::new(Vec::new());
    for _ in 0..3 {
        foresight
            .after_line(&hall(), &[], |spec| {
                requests.lock().unwrap().push(spec.place_id().to_owned());
                true
            })
            .await;
    }
    assert_eq!(*requests.lock().unwrap(), ["hall"]);
    assert_eq!(foresight.proposed_arena("hall").as_deref(), Some("hall"));
}

#[tokio::test]
async fn likely_outside_requests_only_the_chosen_exit_destination() {
    let foresight = FightForesight::new(Box::new(FakeJev(Ok(2))));
    let mut requests = Vec::new();
    foresight
        .after_line(&shop(), &[], |spec| {
            requests.push(spec.place_id().to_owned());
            true
        })
        .await;
    assert_eq!(requests, ["street"]);
    assert_eq!(foresight.proposed_arena("shop").as_deref(), Some("street"));
}

#[tokio::test]
async fn an_unnamed_large_exit_never_spends_on_a_placeholder_place_id() {
    let foresight = FightForesight::new(Box::new(FakeJev(Ok(2))));
    let mut place = shop();
    place.exits[0].place_id = crate::gameplay::place::UNNAMED.to_owned();
    foresight
        .after_line(&place, &[], |_| {
            panic!("a canonical destination is required")
        })
        .await;
    assert_eq!(foresight.proposed_arena("shop"), None);
    assert_eq!(
        foresight
            .state
            .lock()
            .unwrap()
            .selected_exits
            .get("shop")
            .map(String::as_str),
        Some("door")
    );
    assert!(
        !foresight
            .state
            .lock()
            .unwrap()
            .requested
            .contains(crate::gameplay::place::UNNAMED)
    );
}

#[tokio::test]
async fn unlikely_unavailable_and_invalid_choices_never_request_worlds() {
    for verdict in [
        Ok(0),
        Err(JevUnavailable::ProviderUnavailable),
        Err(JevUnavailable::Timeout),
        Ok(99),
    ] {
        let foresight = FightForesight::new(Box::new(FakeJev(verdict)));
        foresight
            .after_line(&shop(), &[], |_| panic!("must not request"))
            .await;
        assert!(foresight.proposed_arena("shop").is_none());
    }
}

#[tokio::test]
async fn unavailable_resource_service_is_attempted_only_once_per_run() {
    let foresight = FightForesight::new(Box::new(FakeJev(Ok(1))));
    let mut tries = 0;
    for _ in 0..2 {
        foresight
            .after_line(&hall(), &[], |_| {
                tries += 1;
                false
            })
            .await;
    }
    assert_eq!(tries, 1);
    assert!(foresight.state.lock().unwrap().requested.contains("hall"));
}

fn hall() -> Place {
    Place {
        id: "hall".to_owned(),
        facts: vec!["A spacious assembly hall with ample floor for combat".to_owned()],
        exits: Vec::new(),
    }
}

#[tokio::test]
async fn small_room_without_a_large_exit_uses_flat_and_revokes_an_old_proposal() {
    let foresight = FightForesight::new(Box::new(FakeJev(Ok(2))));
    foresight.after_line(&shop(), &[], |_| true).await;
    assert_eq!(foresight.proposed_arena("shop").as_deref(), Some("street"));
    let mut closed = shop();
    closed.exits.clear();
    // With no exits, index 2 is the explicit no-large-arena verdict.
    foresight
        .after_line(&closed, &[], |_| {
            panic!("small rooms never request a world")
        })
        .await;
    assert_eq!(foresight.proposed_arena("shop"), None);
}

struct PendingJev(Arc<std::sync::atomic::AtomicUsize>);

impl Jev for PendingJev {
    fn forecast<'a>(&'a self, state: &'a serde_json::Value, _: &'a [String]) -> ChoiceFuture<'a> {
        assert_eq!(state["fight_starting"], true);
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn an_unforecast_opening_times_out_once_and_never_dispatches_a_world() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let foresight = FightForesight::new(Box::new(PendingJev(calls.clone())));
    assert_eq!(
        foresight
            .opening(&shop(), |_| panic!("no world after timeout"))
            .await,
        None
    );
    assert_eq!(
        foresight
            .opening(&shop(), |_| panic!("no retry world"))
            .await,
        None
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unforecast_opening_asks_for_an_arena_once_and_existing_forecast_is_reused() {
    let foresight = FightForesight::new(Box::new(FakeJev(Ok(2))));
    let mut requested = Vec::new();
    for _ in 0..2 {
        assert_eq!(
            foresight
                .opening(&shop(), |spec| {
                    requested.push(spec.place_id().to_owned());
                    true
                })
                .await
                .as_deref(),
            Some("street")
        );
    }
    assert_eq!(requested, ["street"]);
}

struct RecordingJev(tokio::sync::mpsc::Sender<serde_json::Value>);

impl Jev for RecordingJev {
    fn forecast<'a>(
        &'a self,
        state: &'a serde_json::Value,
        _choices: &'a [String],
    ) -> ChoiceFuture<'a> {
        self.0.try_send(state.clone()).unwrap();
        Box::pin(async { Ok(0) })
    }
}

#[tokio::test]
async fn coalesced_table_rows_keep_their_committed_place_after_a_move() {
    use crate::gameplay::{
        adventure::{RulesMode, record, shop},
        beat_script_variety_tests as native, pack, place, place_creation,
        startup_configuration::ContentRating,
    };
    use df_protocol::common as rpc;

    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    let (worker, mut judgments, log, updates) = pack::with_test_pack(loaded, || {
        let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let (updates, checkpoints) = watch::channel(opening.clone());
        let log = TableLog::new();
        let (judgments, received) = tokio::sync::mpsc::channel(4);
        let director = Arc::new(FightForesight::new(Box::new(RecordingJev(judgments))));
        let worker = subscribe(director, log.clone(), checkpoints);
        log.push_speech("Brynn", 0, "We are still in the shop.");
        let reserved = log.reserve();
        let made = place_creation::build(
            &place_creation::tests::request(),
            &place_creation::tests::place_slot().to_string(),
            &place_creation::tests::people_slot().to_string(),
            &place_creation::tests::things_slot().to_string(),
        )
        .unwrap();
        let moved = place::plan(
            &opening,
            "Brynn",
            df_types::OperationId::from_bytes(&[0x44; 16]).unwrap(),
            "place-mareks-cloth-shop",
            "street-door",
            "place-market-street",
            Some(&made),
        )
        .unwrap_or_else(|_| panic!("the shop door admits the fixture street"))
        .candidate;
        updates.send_replace(moved);
        // It reserved a position in the shop, but commits while the party is on the street.
        log.push_reserved(
            reserved,
            "Brynn",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Speech,
            "We have reached the street.",
        );
        log.push_speech("Vale", 1, "I am here too.");
        // No await has let the subscriber read any row before the move.
        (worker, received, log, updates)
    });
    let states = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut states = Vec::new();
        for _ in 0..3 {
            states.push(judgments.recv().await.unwrap());
        }
        states
    })
    .await
    .expect("the subscription judges all committed rows");
    assert_eq!(states[0]["place_id"], "place-mareks-cloth-shop");
    assert_eq!(states[1]["place_id"], "place-market-street");
    assert_eq!(states[2]["place_id"], "place-market-street");
    assert_eq!(states[0]["recent_table_lines"].as_array().unwrap().len(), 1);
    assert_eq!(states[1]["recent_table_lines"].as_array().unwrap().len(), 1);
    assert_eq!(states[2]["recent_table_lines"].as_array().unwrap().len(), 2);
    assert_eq!(states[0]["place"][0], "Marek\u{2019}s Cloth Shop");
    assert_eq!(states[1]["place"][0], "Market Street");
    assert_eq!(
        log.committed_line_at_sequence(1).unwrap().1.place.id,
        "place-mareks-cloth-shop"
    );
    drop(updates);
    drop(worker);
}
