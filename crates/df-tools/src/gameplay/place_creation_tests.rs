use super::*;
use crate::gameplay::{
    adventure::scene_model::SceneWayOut,
    startup_configuration::ApproachSettings,
    storyteller::tests::{FakeEndpoint, Reply, cache_directory, teller},
};
use serde_json::json;
use std::time::Duration;

pub(in crate::gameplay) fn street_door() -> SceneWayOut {
    SceneWayOut {
        id: "street-door".to_owned(),
        name: "street door".to_owned(),
        aliases: vec!["door".to_owned(), "front door".to_owned()],
        position: "at the front of the shop".to_owned(),
        beyond_place_id: place::UNNAMED.to_owned(),
        beyond: "the market street outside".to_owned(),
        open: true,
    }
}

pub(in crate::gameplay) fn request() -> CreationRequest {
    CreationRequest {
        adventure: "Marek's Cloth Shop".to_owned(),
        origin_id: "place-mareks-cloth-shop".to_owned(),
        origin_name: "Marek's Cloth Shop".to_owned(),
        origin_short: "a small cloth shop with a wooden counter".to_owned(),
        origin_narration: "You are inside Marek's cloth shop.".to_owned(),
        time_of_day: "afternoon".to_owned(),
        weather: "dry and cool outside".to_owned(),
        origin_people: vec![
            ("Marek".to_owned(), "cloth merchant".to_owned()),
            ("Corin".to_owned(), "adventurer".to_owned()),
            ("Jessa".to_owned(), "off-duty soldier".to_owned()),
        ],
        statements: vec!["Marek: I run this shop and mend clothing.".to_owned()],
        heroes: vec!["Brynn".to_owned(), "Vale".to_owned()],
        exit: Some(street_door()),
        goal: "I follow Brakka out of the shop into the market street.".to_owned(),
        landmark: None,
        named: None,
        taken_ids: vec!["place-mareks-cloth-shop".to_owned()],
        place_names: vec!["Marek's Cloth Shop".to_owned()],
        people_names: vec!["Tavin Reed".to_owned()],
        canon: json!({"facts": [{
            "kind": "custom",
            "name": "northbound wagons",
            "content": "Merchants heading north gather at the market before dawn.",
            "established_by": "Marek",
        }]}),
        recent_table: vec!["Marek: Merchants heading north gather at the market.".to_owned()],
        recalled_memory: String::new(),
    }
}

pub(in crate::gameplay) fn place_slot() -> Value {
    json!({
        "name": "Market Street",
        "short": "a cobbled street where northbound merchants load wagons",
        "kind": "market-street",
        "narration": "You step out onto Market Street. Carts crowd the cobbles, and merchants load wagons for the northern road.",
        "greeting": "Mind the wheels, friends!",
        "speaker": "Hobb",
        "wares": [{
            "seller": "Wenna",
            "name": "saffron pinch",
            "description": "A paper twist of saffron for a pot of stew.",
            "price_cp": 10,
        }],
        "services": [{
            "provider": "Hobb",
            "name": "wagon directions",
            "description": "Hobb points out the northern road and its toll gate.",
            "price_cp": 10,
        }],
        "exits": [{
            "name": "north alley",
            "position": "between two stalls",
            "beyond": "a narrow alley toward the north gate",
        }, {
            "name": "Carriage lane",
            "position": "past the wagons",
            "beyond": "the carriage stand where coaches leave for the city centre",
        }],
        "landmarks": [
            {"name": "the Bell Tower", "direction": "up the hill past the well"},
            {"name": "Saint Ivo's Temple", "direction": "two streets east"},
        ],
    })
}

pub(in crate::gameplay) fn people_slot() -> Value {
    json!([
        {
            "name": "Hobb",
            "role": "carter",
            "description": "A broad carter in a dusty coat checks the straps on a wagon.",
            "position": "beside the wagon",
            "gender": "he",
            "wants": "Load the wagon before dusk.",
            "attitude": "friendly",
            "knowledge": [
                "The northern wagons leave from this street.",
                "Merchants gather here before dawn.",
                "I have driven this road for years.",
            ],
        },
        {
            "name": "Wenna",
            "role": "spice seller",
            "description": "A quick-handed woman weighs saffron at a small stall.",
            "position": "at the spice stall",
            "gender": "she",
            "wants": "Sell the saffron before it spoils.",
            "attitude": "indifferent",
            "knowledge": [
                "Spice stalls line the east side.",
                "The watch walks the street at noon.",
                "My stall is older than the well.",
            ],
        },
    ])
}

pub(in crate::gameplay) fn things_slot() -> Value {
    json!([
        {"name": "handcart", "detail": "A two-wheeled handcart stacked with bolts of wool. Its handle is worn smooth.", "position": "by the curb", "portable": false},
        {"name": "spice stall", "detail": "A canvas-roofed stall with jars of saffron and pepper. The jars are lined up by size.", "position": "on the east side", "portable": false},
        {"name": "water trough", "detail": "A long stone trough fed by a lead pipe. Mule hoofprints surround it.", "position": "near the wagons", "portable": false},
        {"name": "lamp post", "detail": "An iron lamp post with a glass lantern. Moths circle the unlit wick.", "position": "at the corner", "portable": false},
    ])
}

pub(in crate::gameplay) fn answer(place: &Value, people: &Value, things: &Value) -> Value {
    json!({
        "place": place.to_string(),
        "people": people.to_string(),
        "things": things.to_string(),
    })
}

fn built() -> PlaceRecord {
    build(
        &request(),
        &place_slot().to_string(),
        &people_slot().to_string(),
        &things_slot().to_string(),
    )
    .expect("a valid answer")
}

#[test]
fn a_valid_answer_becomes_a_place_with_a_way_back_that_expands_to_a_valid_scene_model() {
    let record = built();
    assert_eq!(record.place_id, "place-market-street");
    assert_eq!(record.kind, SettingKind::MarketStreet);
    assert!(record.stored_bytes() <= place::MAXIMUM_PLACE_BYTES);
    let expanded = record.expand().expect("expands");
    assert!(expanded.model.validate_structure().is_ok());
    assert_eq!(expanded.model.offers.len(), 1);
    assert_eq!(expanded.model.offers[0].seller_id, "market-street-wenna");
    assert_eq!(expanded.model.offers[0].name, "saffron pinch");
    assert_eq!(expanded.model.offers[0].cost_cp, 10);
    assert!(expanded.model.inventory().iter().any(|item| {
        item.location == crate::gameplay::adventure::scene_model::SceneInventoryLocation::Stock
            && item.cost_cp == Some(10)
    }));
    assert_eq!(expanded.model.services.len(), 1);
    assert_eq!(expanded.model.services[0].provider_id, "market-street-hobb");
    assert_eq!(expanded.model.services[0].price_cp, 10);
    let ways: Vec<&str> = expanded
        .model
        .ways_out
        .iter()
        .map(|way| way.id.as_str())
        .collect();
    assert_eq!(ways, ["way-back", "north-alley", "carriage-lane"]);
    // A sentence-case common noun is stored in lower case, so the narration gives it an article.
    assert_eq!(expanded.model.ways_out[2].name, "carriage lane");
    assert_eq!(record.landmarks.len(), 2);
    assert_eq!(record.landmarks[1].name, "Saint Ivo's Temple");
    let back = &expanded.model.ways_out[0];
    assert_eq!(back.beyond_place_id, "place-mareks-cloth-shop");
    assert!(back.open);
    assert_eq!(
        expanded
            .cast
            .iter()
            .map(|member| member.name.as_str())
            .collect::<Vec<_>>(),
        ["Hobb", "Wenna"]
    );
    assert_eq!(expanded.model.time_of_day, "afternoon");
    assert_eq!(expanded.model.weather, "dry and cool outside");
    assert!(
        expanded
            .model
            .people
            .iter()
            .all(|person| person.id.starts_with("market-street-") && person.knowledge.len() == 3)
    );
    // The expansion is deterministic: the same record always reads back the same place.
    assert_eq!(record.expand().unwrap().model, expanded.model);
}

#[test]
fn trade_entries_must_name_a_created_person_and_have_bounded_positive_copper_prices() {
    assert_eq!(
        with(|place, _, _| place["wares"] = json!([{
            "seller": "Nobody",
            "name": "saffron pinch",
            "description": "A small twist of saffron.",
            "price_cp": 10,
        }]))
        .unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
    assert_eq!(
        with(|place, _, _| place["services"] = json!([{
            "provider": "Hobb",
            "name": "directions",
            "description": "Points out the northern road.",
            "price_cp": 0,
        }]))
        .unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
    assert_eq!(
        with(|place, _, _| place["wares"] = json!([{
            "seller": "Wenna",
            "name": "saffron pinch",
            "description": "A small twist of saffron.",
            "price_cp": 100_001,
        }]))
        .unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
}

fn with<F: FnOnce(&mut Value, &mut Value, &mut Value)>(
    change: F,
) -> Result<PlaceRecord, CreationError> {
    let (mut place, mut people, mut things) = (place_slot(), people_slot(), things_slot());
    change(&mut place, &mut people, &mut things);
    build(
        &request(),
        &place.to_string(),
        &people.to_string(),
        &things.to_string(),
    )
}

#[test]
fn an_answer_that_breaks_a_rule_of_a_place_is_refused_and_names_the_rule() {
    assert_eq!(
        with(|place, _, _| place["kind"] = json!("dragon-lair")).unwrap_err(),
        CreationError::Invalid("kind")
    );
    assert_eq!(
        with(|place, _, _| place["speaker"] = json!("Nobody")).unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
    assert_eq!(
        with(|_, people, _| people[0]["name"] = json!("Marek")).unwrap_err(),
        CreationError::Invalid("person_name")
    );
    assert_eq!(
        with(|_, people, _| people[0]["name"] = json!("Brynn")).unwrap_err(),
        CreationError::Invalid("person_name")
    );
    assert_eq!(
        with(|_, _, things| things[0]["detail"] = json!("Only one sentence here.")).unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
    assert_eq!(
        with(|_, _, things| {
            things.as_array_mut().unwrap().pop();
        })
        .unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
    assert_eq!(
        with(|place, _, _| place["narration"] = json!("x".repeat(300))).unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
    assert_eq!(
        with(|place, _, _| place["exits"][0]["name"] = json!("way back")).unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
    assert_eq!(
        with(|_, people, _| people[1]["attitude"] = json!("eager")).unwrap_err(),
        CreationError::Invalid("invalid_place")
    );
    // An unknown field is a different shape, not an extra the server ignores.
    assert_eq!(
        with(|place, _, _| place["secret_plan"] = json!("x")).unwrap_err(),
        CreationError::Invalid("place_json")
    );
    let mut taken = request();
    taken.taken_ids.push("place-market-street".to_owned());
    taken.named = Some("place-market-street".to_owned());
    assert_eq!(
        build(
            &taken,
            &place_slot().to_string(),
            &people_slot().to_string(),
            &things_slot().to_string()
        )
        .unwrap_err(),
        CreationError::Invalid("place_id")
    );
}

#[test]
fn a_place_that_would_not_fit_the_checkpoint_text_is_refused_not_truncated() {
    let result = with(|place, people, things| {
        place["name"] = json!("N".repeat(40));
        place["short"] = json!("s".repeat(80));
        place["narration"] = json!("n".repeat(260));
        place["greeting"] = json!("g".repeat(120));
        place["exits"] = json!([
            {"name": "e".repeat(32), "position": "p".repeat(32), "beyond": "b".repeat(100)},
            {"name": "f".repeat(32), "position": "p".repeat(32), "beyond": "b".repeat(100)},
        ]);
        for person in people.as_array_mut().unwrap() {
            person["role"] = json!("r".repeat(32));
            person["description"] = json!("a".repeat(110));
            person["position"] = json!("p".repeat(32));
            person["wants"] = json!("b".repeat(60));
            person["knowledge"] = json!(["c".repeat(80), "d".repeat(80), "e".repeat(80)]);
        }
        for thing in things.as_array_mut().unwrap() {
            thing["detail"] = json!(format!("{}. {}.", "f".repeat(100), "g".repeat(100)));
            thing["position"] = json!("p".repeat(32));
        }
    });
    // Every field is within its own bound, yet the whole is more than the checkpoint holds.
    assert_eq!(result.unwrap_err(), CreationError::Invalid("too_large"));
}

fn jev(endpoint: &FakeEndpoint) -> ApproachModel {
    ApproachModel::configured(Some(ApproachSettings {
        credential: "jev-fixture-sentinel".to_owned(),
        base_url: format!("http://{}", endpoint.address),
    }))
}

fn choice(option: &str) -> Reply {
    Reply::Status(
        200,
        json!({
            "answers": {"q1": {"type": "choice", "choice": option, "confidence": 0.99}},
            "model": "jev-1.13.0",
            "usage": {"input_tokens": 10, "output_tokens": 1}
        }),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn jev_chooses_an_open_exit_for_a_leaving_line_and_none_when_the_hero_stays() {
    let door = street_door();
    let window = SceneWayOut {
        id: "side-window".to_owned(),
        name: "side window".to_owned(),
        aliases: vec!["window".to_owned()],
        position: "on the side wall".to_owned(),
        beyond_place_id: place::UNNAMED.to_owned(),
        beyond: "the alley".to_owned(),
        open: true,
    };
    let exits = [&door, &window];
    let choices = HeadingChoices {
        exits: &exits,
        known: &[],
        landmarks: &[],
    };
    let leaves = FakeEndpoint::start(vec![choice("o1")]);
    let chosen = judge_heading(
        &jev(&leaves),
        "I follow Brakka out of the shop into the market street.",
        "Sigrun",
        "Marek's Cloth Shop",
        "a small cloth shop",
        &choices,
    )
    .await;
    assert_eq!(chosen, Ok(Heading::Exit(0)));
    let body = &leaves.bodies()[0];
    // The line reaches Jev as quoted data, with every exit, a journey elsewhere and staying.
    assert!(body.to_string().contains("I follow Brakka out of the shop"));
    assert_eq!(
        body["questions"]["q1"]["criteria"]
            .as_object()
            .unwrap()
            .len(),
        4
    );

    let stays = FakeEndpoint::start(vec![choice("o4")]);
    assert_eq!(
        judge_heading(
            &jev(&stays),
            "I walk up to the counter.",
            "Sigrun",
            "Shop",
            "",
            &choices
        )
        .await,
        Ok(Heading::Stay)
    );
    let second = FakeEndpoint::start(vec![choice("o2")]);
    assert_eq!(
        judge_heading(
            &jev(&second),
            "I climb out of the window.",
            "Sigrun",
            "Shop",
            "",
            &choices
        )
        .await,
        Ok(Heading::Exit(1))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_stated_destination_is_a_known_place_a_landmark_or_a_journey_elsewhere() {
    let door = street_door();
    let exits = [&door];
    let known = [KnownPlace {
        id: "place-mareks-cloth-shop".to_owned(),
        name: "Marek's Cloth Shop".to_owned(),
        short: "a small cloth shop".to_owned(),
    }];
    let landmarks = [LandmarkDraft {
        name: "the Bell Tower".to_owned(),
        direction: "up the hill".to_owned(),
    }];
    let choices = HeadingChoices {
        exits: &exits,
        known: &known,
        landmarks: &landmarks,
    };
    let endpoint = FakeEndpoint::start(vec![choice("o2"), choice("o3"), choice("o4")]);
    let model = jev(&endpoint);
    for (words, expected) in [
        ("We head back to Marek's shop.", Heading::Known(0)),
        ("We climb up to the bell tower.", Heading::Landmark(0)),
        (
            "We follow the street on toward the carriage stand.",
            Heading::Elsewhere,
        ),
    ] {
        assert_eq!(
            judge_heading(
                &model,
                words,
                "Brakka",
                "Market Street",
                "a street",
                &choices
            )
            .await,
            Ok(expected),
            "{words}"
        );
    }
    let bodies = endpoint.bodies();
    let criteria = bodies[0]["questions"]["q1"]["criteria"].to_string();
    assert!(criteria.contains("Marek's Cloth Shop") && criteria.contains("the Bell Tower"));
    assert!(criteria.contains("elsewhere") && criteria.contains("none"));
    assert_eq!(
        Heading::Known(0).label(&choices),
        "journey:place-mareks-cloth-shop"
    );
    assert_eq!(Heading::Elsewhere.label(&choices), "journey:new");
    assert_eq!(Heading::Exit(0).label(&choices), "street-door");
}

#[tokio::test(flavor = "current_thread")]
async fn without_jev_nothing_is_chosen_and_the_reason_is_returned() {
    let door = street_door();
    let exits = [&door];
    let choices = HeadingChoices {
        exits: &exits,
        known: &[],
        landmarks: &[],
    };
    let outcome = judge_heading(
        &ApproachModel::default(),
        "I leave.",
        "Sigrun",
        "Shop",
        "",
        &choices,
    )
    .await;
    assert_eq!(outcome, Err(JevUnavailable::ProviderUnavailable));
    assert_eq!(
        judge_ride(&ApproachModel::default(), "Driver!", "Sigrun", "Shop", None).await,
        Err(JevUnavailable::ProviderUnavailable)
    );
    // With no service to choose among, nothing is asked.
    let endpoint = FakeEndpoint::start(vec![choice("o1")]);
    assert_eq!(
        judge_ride_service(&jev(&endpoint), "A carriage!", &[]).await,
        Ok(None)
    );
    assert_eq!(endpoint.connections(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn jev_says_whether_a_line_hails_a_ride_and_which_listed_service_is_that_ride() {
    let endpoint =
        FakeEndpoint::start(vec![choice("o2"), choice("o1"), choice("o2"), choice("o3")]);
    let model = jev(&endpoint);
    let hail =
        "I wave down a passing carriage and ask the driver to take us into the heart of the city.";
    assert_eq!(
        judge_ride(&model, hail, "Brakka", "The Market Yard", None).await,
        Ok(true)
    );
    assert_eq!(
        judge_ride(
            &model,
            "Is there a carriage stand nearby?",
            "Brakka",
            "The Market Yard",
            None
        )
        .await,
        Ok(false)
    );
    let services = [
        ServiceOffer {
            name: "harness mending",
            description: "Stitches a torn harness.",
            provider: "Mira Fen",
        },
        ServiceOffer {
            name: "carriage hire",
            description: "A coach and driver into the city centre.",
            provider: "Oren Vale",
        },
    ];
    assert_eq!(
        judge_ride_service(&model, hail, &services).await,
        Ok(Some(1))
    );
    assert_eq!(judge_ride_service(&model, hail, &services).await, Ok(None));
    let bodies = endpoint.bodies();
    assert!(
        bodies[0]
            .to_string()
            .contains("wave down a passing carriage")
    );
    assert!(bodies[2].to_string().contains("carriage hire"));
}

#[test]
fn a_travel_beat_falls_back_to_a_plain_line_that_names_the_hero_and_both_places() {
    let walk = TravelBeat {
        hero: "Brakka",
        words: "We walk to the temple.",
        from: "Market Street",
        to: "Saint Ivo's Temple",
        to_short: "a domed temple",
        ride: false,
        hired: None,
    };
    assert_eq!(
        walk.fallback(),
        "Brakka sets off with the party from Market Street toward Saint Ivo's Temple."
    );
    let hailed = TravelBeat { ride: true, ..walk };
    assert!(hailed.fallback().starts_with("Brakka hails a passing ride"));
    let hired = TravelBeat {
        ride: true,
        hired: Some(("carriage hire", "Oren Vale")),
        ..walk
    };
    assert_eq!(
        hired.fallback(),
        "Oren Vale drives Brakka and the party away from Market Street, through the busy streets toward Saint Ivo's Temple."
    );
}

#[test]
fn a_new_place_never_repeats_a_place_or_a_person_the_run_has() {
    // The real shop under another spelling is not made a second time.
    assert_eq!(
        with(|place, _, _| place["name"] = json!("Marek\u{2019}s cloth shop")).unwrap_err(),
        CreationError::Invalid("place_name")
    );
    let mut known = request();
    known.place_names.push("The Open Market".to_owned());
    assert_eq!(
        build(
            &known,
            &{
                let mut place = place_slot();
                place["name"] = json!("the open market");
                place
            }
            .to_string(),
            &people_slot().to_string(),
            &things_slot().to_string(),
        )
        .unwrap_err(),
        CreationError::Invalid("place_name")
    );
    // Nor a second Marek, a Marek with a surname, or someone from another made place.
    for name in ["Marek Dorn", "MAREK", "Tavin", "Tavin Reed"] {
        assert_eq!(
            with(|_, people, _| people[0]["name"] = json!(name)).unwrap_err(),
            CreationError::Invalid("person_name"),
            "{name}"
        );
    }
    // A landmark that is a place the run has is refused, as is a missing second exit.
    assert_eq!(
        with(|place, _, _| place["landmarks"][0]["name"] = json!("Marek's Cloth Shop"))
            .unwrap_err(),
        CreationError::Invalid("landmarks")
    );
    assert_eq!(
        with(|place, _, _| {
            place["landmarks"].as_array_mut().unwrap().pop();
        })
        .unwrap_err(),
        CreationError::Invalid("landmarks")
    );
    assert_eq!(
        with(|place, _, _| {
            place["exits"].as_array_mut().unwrap().pop();
        })
        .unwrap_err(),
        CreationError::Invalid("exits")
    );
    // A journey to a landmark makes that landmark, under its own name.
    let mut tower = request();
    tower.exit = None;
    tower.landmark = Some(LandmarkDraft {
        name: "the Bell Tower".to_owned(),
        direction: "up the hill".to_owned(),
    });
    let wrong = build(
        &tower,
        &place_slot().to_string(),
        &people_slot().to_string(),
        &things_slot().to_string(),
    );
    assert_eq!(
        wrong.unwrap_err(),
        CreationError::Invalid("destination_name")
    );
    let mut named = place_slot();
    named["name"] = json!("The Bell Tower");
    named["landmarks"][0]["name"] = json!("the Clock Arch");
    let made = build(
        &tower,
        &named.to_string(),
        &people_slot().to_string(),
        &things_slot().to_string(),
    )
    .expect("the tower");
    assert_eq!(made.place_id, "place-the-bell-tower");
}

#[test]
fn common_nouns_are_stored_in_lower_case_and_proper_names_keep_their_capitals() {
    let proper = vec!["Wenna".to_owned(), "Saint Ivo's Temple".to_owned()];
    for (written, stored) in [
        ("Harnesses", "harnesses"),
        ("Water trough", "water trough"),
        ("Travel cloak", "travel cloak"),
        ("water trough", "water trough"),
        ("Wenna's scale", "Wenna's scale"),
        ("Wenna\u{2019}s scale", "Wenna\u{2019}s scale"),
        ("Saint Ivo's lamp", "Saint Ivo's lamp"),
        ("Old Quay Crane", "Old Quay Crane"),
        ("OK sign", "OK sign"),
    ] {
        assert_eq!(common_noun(written, &proper), stored, "{written}");
    }
    let record = with(|place, _, things| {
        things[0]["name"] = json!("Handcart");
        place["wares"][0]["name"] = json!("Saffron pinch");
        place["services"][0]["name"] = json!("Wagon directions");
    })
    .expect("a valid answer");
    assert_eq!(record.things[0].name, "handcart");
    assert_eq!(record.wares[0].name, "saffron pinch");
    assert_eq!(record.services[0].name, "wagon directions");
}

#[tokio::test(flavor = "current_thread")]
async fn creation_gives_the_storyteller_the_canon_and_the_table_and_retries_once_naming_the_rule() {
    let mut wrong = place_slot();
    wrong["speaker"] = json!("Nobody");
    let endpoint = FakeEndpoint::start(vec![
        Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
        Reply::Output(answer(&place_slot(), &people_slot(), &things_slot())),
    ]);
    let cache = cache_directory("place-creation-retry");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &cache);
    let record = create(&storyteller, &request()).await.expect("place");
    assert_eq!(record.name, "Market Street");
    assert_eq!(record.origin.id, "place-mareks-cloth-shop");
    assert_eq!(endpoint.connections(), 2);
    let bodies = endpoint.bodies();
    let first = bodies[0]["input"].as_str().unwrap();
    let second = bodies[1]["input"].as_str().unwrap();
    // The run's canon and the table's own words reach the storyteller, as the request's facts.
    assert!(first.contains("Merchants heading north gather at the market before dawn"));
    assert!(first.contains("Merchants heading north gather at the market."));
    assert!(first.contains("Brynn") && first.contains("street door"));
    assert!(first.contains("wares") && first.contains("services"));
    assert!(first.contains("price_cp") && first.contains("100000 copper pieces"));
    // The hero's stated destination and every place and person the run has reach it too.
    assert!(first.contains("I follow Brakka out of the shop into the market street."));
    assert!(first.contains("place_names_in_use") && first.contains("Tavin Reed"));
    assert!(first.contains("landmarks"));
    assert!(!first.contains("previous_answer_problem"));
    assert!(second.contains("previous_answer_problem"));
    assert!(second.contains("invalid_place"));
}

#[tokio::test(flavor = "current_thread")]
async fn a_place_the_storyteller_cannot_make_is_not_made() {
    let off = Storyteller::disabled();
    assert_eq!(
        create(&off, &request()).await.unwrap_err(),
        CreationError::Off
    );
    let mut wrong = place_slot();
    wrong["kind"] = json!("dragon-lair");
    let endpoint = FakeEndpoint::start(vec![
        Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
        Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
    ]);
    let cache = cache_directory("place-creation-refused");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &cache);
    assert_eq!(
        create(&storyteller, &request()).await.unwrap_err(),
        CreationError::Invalid("kind")
    );
    assert_eq!(endpoint.connections(), 2, "one retry, then it stops");
    let down = FakeEndpoint::start(vec![Reply::Status(500, json!({"error": "down"}))]);
    let storyteller = teller(
        down.address,
        0,
        Duration::from_secs(5),
        &cache_directory("place-creation-down"),
    );
    assert_eq!(
        create(&storyteller, &request()).await.unwrap_err(),
        CreationError::Model
    );
}
