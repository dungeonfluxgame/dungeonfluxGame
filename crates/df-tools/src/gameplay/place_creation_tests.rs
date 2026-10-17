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
        exit: street_door(),
        named: None,
        taken_ids: vec!["place-mareks-cloth-shop".to_owned()],
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
        "exits": [{
            "name": "north alley",
            "position": "between two stalls",
            "beyond": "a narrow alley toward the north gate",
        }],
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
    let ways: Vec<&str> = expanded
        .model
        .ways_out
        .iter()
        .map(|way| way.id.as_str())
        .collect();
    assert_eq!(ways, ["way-back", "north-alley"]);
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
    let leaves = FakeEndpoint::start(vec![choice("o1")]);
    let chosen = judge_exit(
        &jev(&leaves),
        "I follow Brakka out of the shop into the market street.",
        "Sigrun",
        "Marek's Cloth Shop",
        "a small cloth shop",
        &exits,
    )
    .await;
    assert_eq!(chosen, Ok(Some(0)));
    let body = &leaves.bodies()[0];
    // The line reaches Jev as quoted data, with every exit and a last option for staying.
    assert!(body.to_string().contains("I follow Brakka out of the shop"));
    assert_eq!(
        body["questions"]["q1"]["criteria"]
            .as_object()
            .unwrap()
            .len(),
        3
    );

    let stays = FakeEndpoint::start(vec![choice("o3")]);
    assert_eq!(
        judge_exit(
            &jev(&stays),
            "I walk up to the counter.",
            "Sigrun",
            "Shop",
            "",
            &exits
        )
        .await,
        Ok(None)
    );
    let second = FakeEndpoint::start(vec![choice("o2")]);
    assert_eq!(
        judge_exit(
            &jev(&second),
            "I climb out of the window.",
            "Sigrun",
            "Shop",
            "",
            &exits
        )
        .await,
        Ok(Some(1))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn without_jev_no_exit_is_chosen_and_the_reason_is_returned() {
    let door = street_door();
    let outcome = judge_exit(
        &ApproachModel::default(),
        "I leave.",
        "Sigrun",
        "Shop",
        "",
        &[&door],
    )
    .await;
    assert_eq!(outcome, Err(JevUnavailable::ProviderUnavailable));
    // With nothing to leave by, nothing is asked.
    let endpoint = FakeEndpoint::start(vec![choice("o1")]);
    assert_eq!(
        judge_exit(&jev(&endpoint), "I leave.", "Sigrun", "Shop", "", &[]).await,
        Ok(None)
    );
    assert_eq!(endpoint.connections(), 0);
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
