use std::sync::Mutex;

use super::*;

struct FakeJev(Result<usize, JevUnavailable>);

impl Jev for FakeJev {
    fn forecast<'a>(
        &'a self,
        state: &'a serde_json::Value,
        choices: &'a [String],
    ) -> ChoiceFuture<'a> {
        assert_eq!(state["place_id"], "shop");
        assert_eq!(choices.len(), 3);
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
async fn likely_requests_here_once_per_place_per_run() {
    let foresight = FightForesight::new(Box::new(FakeJev(Ok(1))));
    let requests = Mutex::new(Vec::new());
    for _ in 0..3 {
        foresight
            .after_line(&shop(), &[], |spec| {
                requests.lock().unwrap().push(spec.place_id().to_owned());
                true
            })
            .await;
    }
    assert_eq!(*requests.lock().unwrap(), ["shop"]);
    assert_eq!(foresight.proposed_arena("shop").as_deref(), Some("shop"));
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
            .after_line(&shop(), &[], |_| {
                tries += 1;
                false
            })
            .await;
    }
    assert_eq!(tries, 1);
    assert!(foresight.state.lock().unwrap().requested.contains("shop"));
}
