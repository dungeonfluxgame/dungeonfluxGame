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
        run_seed: "hs69-fixture-run".to_owned(),
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
    json!({"market":true,"name":"Market Street","short":"a cobbled market for northbound merchants","kind":"market-street","narration":"Three canvas stalls line Market Street.","greeting":"Mind the wheels, friends!","speaker":"Hobb","wares":[{"seller":"Wenna","name":"saffron pinch","description":"A saffron twist.","price_cp":10},{"seller":"Wenna","name":"pepper pinch","description":"A pepper twist.","price_cp":5},{"seller":"Hobb","name":"oats","description":"A small sack.","price_cp":5},{"seller":"Hobb","name":"barley","description":"A small sack.","price_cp":4},{"seller":"Yara","name":"herring","description":"A fresh fish.","price_cp":2},{"seller":"Yara","name":"trout","description":"A river fish.","price_cp":3}],"services":[{"provider":"Hobb","name":"wagon directions","description":"Directions to the north road.","price_cp":10}],"exits":[{"name":"north alley","position":"between two stalls","beyond":"the north gate alley"},{"name":"Carriage lane","position":"past the wagons","beyond":"the carriage stand"}],"landmarks":[{"name":"the Bell Tower","direction":"up the hill"},{"name":"Saint Ivo's Temple","direction":"two streets east"}],"stalls":[{"name":"cart stall","keeper":"Hobb","position":"beside the wagon"},{"name":"spice stall","keeper":"Wenna","position":"on the east side"},{"name":"fish stall","keeper":"Yara","position":"by the fountain"}],"mentioned_stalls":["spice stall","cart stall"]})
}

pub(in crate::gameplay) fn people_slot() -> Value {
    json!([{"name":"Hobb","purse_cp":2400,"role":"carter","description":"A broad carter in a dusty coat.","position":"beside the wagon","gender":"he","age":"adult","wants":"Load before dusk.","attitude":"friendly","stat_block":"commoner","knowledge":["Wagons load here.","Bell Tower uphill.","I drive wagons."]},{"name":"Wenna","purse_cp":5000,"role":"spice seller","description":"A quick-handed adult weighs saffron.","position":"at the spice stall","gender":"she","age":"adult","wants":"Sell spices.","attitude":"indifferent","stat_block":"commoner","knowledge":["Spices are sold here.","Bell Tower uphill.","I sell spices."]},{"name":"Yara","purse_cp":500,"role":"fish seller","description":"A brisk adult in an apron.","position":"by the fountain","gender":"she","age":"adult","wants":"Sell fish.","attitude":"friendly","stat_block":"commoner","knowledge":["Fish came today.","Bell Tower uphill.","I keep this stall."]}])
}

pub(in crate::gameplay) fn things_slot() -> Value {
    json!([{"name":"cart stall","detail":"A handcart holds grain. Its handle is smooth.","position":"by the curb","portable":false},{"name":"spice stall","detail":"A canvas stall holds spices. Its jars stand in rows.","position":"on the east side","portable":false},{"name":"fish stall","detail":"A stall holds fish on wet boards. A fountain stands behind it.","position":"by the fountain","portable":false},{"name":"lamp post","detail":"An iron post holds a lantern. Its wick is unlit.","position":"at the corner","portable":false}])
}

pub(in crate::gameplay) fn answer(place: &Value, people: &Value, things: &Value) -> Value {
    let mut place = place.clone();
    let mut people = people.clone();
    // Legacy fake adults predate explicit voice-age bands and purses; production admission
    // still requires both fields, and invalid-input tests bypass this fixture serializer.
    for person in people.as_array_mut().unwrap() {
        person
            .as_object_mut()
            .unwrap()
            .entry("age")
            .or_insert(json!("adult"));
        person
            .as_object_mut()
            .unwrap()
            .entry("purse_cp")
            .or_insert(json!(500));
    }
    // Callers reusing this market template for a gate/other destination replace its cast.
    // Such fake destinations have no inherited trading stalls or seller bindings.
    if place["stalls"].as_array().is_some_and(|stalls| {
        stalls.iter().any(|stall| {
            !people
                .as_array()
                .unwrap()
                .iter()
                .any(|person| person["name"] == stall["keeper"])
        })
    }) {
        place["market"] = json!(false);
        place["stalls"] = json!([]);
        place["mentioned_stalls"] = json!([]);
    }
    // Generated output now declares all sellers explicitly; legacy stored builds stay valid.
    place["traders"] = json!(
        place["wares"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|ware| ware["seller"].as_str())
            .filter(|seller| people
                .as_array()
                .unwrap()
                .iter()
                .any(|person| person["name"] == *seller))
            .collect::<std::collections::BTreeSet<_>>()
    );
    json!({
        "place": place.to_string(),
        "people": people.to_string(),
        "things": things.to_string(),
    })
}

pub(in crate::gameplay) fn persona_answer(prompt: &str) -> Value {
    let facts: Value = prompt
        .lines()
        .find_map(|line| line.strip_prefix("FACTS: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .unwrap();
    let temperament = &facts["drawn_temperament"];
    let role = facts["person"]["role"].as_str().unwrap();
    let (experience, signature) = match role {
        "carter" => (
            "guiding heavy wagons through crowded streets",
            "Easy on the axle.",
        ),
        "spice seller" => (
            "weighing delicate spices for impatient customers",
            "A pinch tells the tale.",
        ),
        "fish seller" => (
            "sorting the dawn catch beside the cold river",
            "Fresh with the tide.",
        ),
        _ => (
            "learning the daily work from experienced neighbours",
            "A little care carries far.",
        ),
    };
    let axis = |name| temperament[name].as_str().unwrap();
    let (energy_cause, pace) = match axis("energy") {
        "low" => ("saving strength through long shifts", "Slow, measured"),
        "high" => ("quick responses to bustling trade", "Quick, animated"),
        _ => ("an even rhythm through daily routine", "Even, unhurried"),
    };
    let (nerve_cause, delivery) = match axis("nerve") {
        "nervous" => (
            "caution after a costly mistake",
            "hedges and verbal fidgets",
        ),
        "bold" => ("confidence from solving hard problems", "direct assertions"),
        _ => ("composure through familiar duties", "composed answers"),
    };
    let warmth_cause = match axis("warmth") {
        "cool" => "Difficult customers taught reserve",
        "warm" => "Regulars became friends",
        _ => "Fair dealing taught courtesy",
    };
    let talk_cause = if axis("talk") == "terse" {
        "busy hours taught brevity"
    } else {
        "long waits encouraged stories"
    };
    let humour_cause = match axis("humour") {
        "dry" => "mishaps invited wry understatement",
        "jokey" => "jokes eased daily fatigue",
        _ => "honest craft discouraged mockery",
    };
    let backstory = format!(
        "Work as a {role}, {experience}, taught {energy_cause} and {nerve_cause}. {warmth_cause}, {talk_cause}, and {humour_cause}.",
    );
    let speech = format!(
        "{pace} pace; {delivery}, {} trade vocabulary, {} phrasing and {} humour.",
        axis("warmth"),
        axis("talk"),
        axis("humour")
    );
    json!({"persona": json!({"temperament": temperament, "backstory":backstory,
        "speech":speech, "catchphrase":signature}).to_string()})
}

pub(in crate::gameplay) fn persona_replies(count: usize) -> Vec<Reply> {
    (0..count)
        .map(|_| Reply::Compute(std::sync::Arc::new(persona_answer)))
        .collect()
}

pub(in crate::gameplay) fn accepting_persona_judge(count: usize) -> FakeEndpoint {
    FakeEndpoint::start((0..count).map(|_| choice("o1")).collect())
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

#[tokio::test]
async fn local_reference_judges_a_plausible_unrecorded_stall_under_a_schema() {
    let mut scene = super::super::pack::scene(super::super::pack::SceneKey::Opening);
    scene.setting = Some(super::super::adventure::setting::SceneSetting {
        place_id: "place-gullwatch-market-street".into(),
        name: "Gullwatch Market Street".into(),
        short: "an open street lined with ordinary market stalls".into(),
        kind: SettingKind::MarketStreet,
        exterior: true,
        art_key: "market-street".into(),
        model: None,
    });
    let jev_endpoint = FakeEndpoint::start(vec![choice("o1"), choice("o1")]);
    let story = FakeEndpoint::start(vec![Reply::Output(json!({"spot":json!({
        "name":"fruit stand", "look":"A canvas stall with baskets of red apples."
    }).to_string()}))]);
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS40/plausible-reference")
        .join(std::process::id().to_string());
    let storyteller = teller(story.address, 0, Duration::from_secs(5), &cache);
    let result = judge_local_reference(
        &storyteller,
        &jev(&jev_endpoint),
        &scene,
        "I go over to a fruit seller and see what is on offer.",
        &[],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        result,
        LocalReference::Spot(LandmarkDraft {
            name: "fruit stand".into(),
            direction: "A canvas stall with baskets of red apples.".into()
        })
    );
    assert_eq!(jev_endpoint.connections(), 2);
    assert!(
        story.bodies()[0]["input"]
            .as_str()
            .unwrap()
            .contains("output_schema")
    );
    assert!(
        jev_endpoint.bodies()[1]
            .to_string()
            .contains("avoid duplicating")
    );
}

#[test]
fn every_stall_belongs_to_its_bound_keeper() {
    let record = built();
    assert!(!record.stalls.is_empty());
    let model = record.expand().unwrap().model;
    for stall in &record.stalls {
        let key = crate::gameplay::place::name_key(&stall.name);
        let thing = model
            .things
            .iter()
            .find(|thing| crate::gameplay::place::name_key(&thing.name) == key)
            .expect("the stall is a thing in the place");
        let keeper = model
            .people
            .iter()
            .find(|person| person.name == stall.keeper)
            .expect("the keeper is a person in the place");
        assert_eq!(thing.owner_id.as_deref(), Some(keeper.id.as_str()));
    }
}

#[test]
fn place_creation_admits_and_preserves_generated_working_capital_and_requires_it_for_new_people() {
    let record = built();
    assert_eq!(record.people[1].purse_cp, 5000);
    assert_eq!(record.expand().unwrap().model.people[1].coin_cp, 5000);
    let stored = serde_json::to_string(&record).unwrap();
    let restored: PlaceRecord = serde_json::from_str(&stored).unwrap();
    assert_eq!(restored.expand().unwrap().model.people[1].coin_cp, 5000);
    for value in [
        json!(-1),
        json!(1.5),
        json!(place::MAXIMUM_PURSE_CP + 1),
        Value::Null,
    ] {
        let mut people = people_slot();
        people[1]["purse_cp"] = value;
        assert!(
            build(
                &request(),
                &place_slot().to_string(),
                &people.to_string(),
                &things_slot().to_string()
            )
            .is_err()
        );
    }
    let mut people = people_slot();
    people[1].as_object_mut().unwrap().remove("purse_cp");
    assert_eq!(
        build(
            &request(),
            &place_slot().to_string(),
            &people.to_string(),
            &things_slot().to_string()
        ),
        Err(CreationError::Invalid("purse_cp"))
    );
    let mut penniless = people_slot();
    penniless[1]["purse_cp"] = json!(0);
    assert_eq!(
        build(
            &request(),
            &place_slot().to_string(),
            &penniless.to_string(),
            &things_slot().to_string()
        ),
        Err(CreationError::Invalid("merchant_purse"))
    );
    let mut legacy = serde_json::to_value(record).unwrap();
    legacy["people"][1]
        .as_object_mut()
        .unwrap()
        .remove("purse_cp");
    let legacy: PlaceRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.expand().unwrap().model.people[1].coin_cp, 0);
}

#[test]
fn a_valid_answer_becomes_a_place_with_a_way_back_that_expands_to_a_valid_scene_model() {
    let record = built();
    assert_eq!(record.place_id, "place-market-street");
    assert_eq!(record.kind, SettingKind::MarketStreet);
    assert!(record.stored_bytes() <= place::MAXIMUM_PLACE_BYTES);
    let expanded = record.expand().expect("expands");
    assert!(expanded.model.validate_structure().is_ok());
    assert_eq!(expanded.model.offers.len(), 6);
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
        ["Hobb", "Wenna", "Yara"]
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
fn created_people_choose_only_supported_srd_person_blocks_and_old_records_default_to_commoners() {
    use crate::gameplay::place::PersonStatBlock;
    use df_rules::tactical::{Cell, CombatantSpec};

    let allowed = output_schema()["people"]["items"]["properties"]["stat_block"]["enum"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(allowed.len(), PersonStatBlock::ALL.len());
    for block in PersonStatBlock::ALL {
        let id = block.srd_id();
        assert!(allowed.contains(&json!(id)));
        let record = with(|_, people, _| people[0]["stat_block"] = json!(id)).unwrap();
        let cast = record.expand().unwrap().cast;
        assert_eq!(cast[0].stat_block.as_deref(), Some(id));
        let spec = CombatantSpec::creature(3, id, Cell(0, 0), 0).unwrap();
        assert_eq!(spec.kind.srd_id(), Some(id));
    }
    for unsupported in ["noble", "veteran", "wolf", "invented-guard", "Guard", ""] {
        assert_eq!(
            with(|_, people, _| people[0]["stat_block"] = json!(unsupported)).unwrap_err(),
            CreationError::Invalid("people_json"),
            "{unsupported} cannot enter the person schema"
        );
    }
    for missing in [Value::Null, json!(false), json!(15)] {
        assert!(with(|_, people, _| people[0]["stat_block"] = missing).is_err());
    }
    assert_eq!(
        with(|_, people, _| {
            people[0].as_object_mut().unwrap().remove("stat_block");
        })
        .unwrap_err(),
        CreationError::Invalid("stat_block")
    );

    // Recovery reads the original record shape, without asking a model to supply new rules.
    let mut old = serde_json::to_value(built()).unwrap();
    for person in old["people"].as_array_mut().unwrap() {
        person.as_object_mut().unwrap().remove("stat_block");
    }
    let record: PlaceRecord = serde_json::from_value(old.clone()).unwrap();
    record.validate().unwrap();
    assert!(
        record
            .people
            .iter()
            .all(|person| person.stat_block.is_none())
    );
    assert!(
        record
            .expand()
            .unwrap()
            .cast
            .iter()
            .all(|person| { person.stat_block.as_deref() == Some("commoner") })
    );
    assert_eq!(serde_json::to_value(record).unwrap(), old);
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
        // Character bounds do not bound UTF-8 bytes; exercise the current journal byte limit.
        let wide = "\u{1f9f5}";
        place["name"] = json!("N".repeat(40));
        place["short"] = json!(wide.repeat(80));
        place["narration"] = json!(wide.repeat(260));
        place["greeting"] = json!(wide.repeat(120));
        place["exits"] = json!([
            {"name": "e".repeat(32), "position": wide.repeat(32), "beyond": wide.repeat(100)},
            {"name": "f".repeat(32), "position": wide.repeat(32), "beyond": wide.repeat(100)},
        ]);
        for person in people.as_array_mut().unwrap() {
            person["role"] = json!(wide.repeat(32));
            person["description"] = json!(wide.repeat(110));
            person["position"] = json!(wide.repeat(32));
            person["wants"] = json!(wide.repeat(60));
            person["knowledge"] = json!([wide.repeat(80), wide.repeat(80), wide.repeat(80)]);
        }
        for thing in things.as_array_mut().unwrap() {
            thing["detail"] = json!(format!("{}. {}.", wide.repeat(100), wide.repeat(100)));
            thing["position"] = json!(wide.repeat(32));
        }
    });
    // Every field is within its own bound, yet the whole is more than the checkpoint holds.
    assert_eq!(result.unwrap_err(), CreationError::Invalid("too_large"));
}

pub(in crate::gameplay) fn jev(endpoint: &FakeEndpoint) -> ApproachModel {
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
    // Redundant landmarks are dropped; the destination remains valid even with one left.
    let cleaned = with(|place, _, _| {
        place["landmarks"][0]["name"] = json!("Marek's Cloth Shop");
    })
    .unwrap();
    assert_eq!(cleaned.landmarks.len(), 1);
    assert_eq!(cleaned.landmarks[0].name, "Saint Ivo's Temple");
    assert!(cleaned.expand().is_ok());
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
        things[0]["name"] = json!("Cart stall");
        place["wares"][0]["name"] = json!("Saffron pinch");
        place["services"][0]["name"] = json!("Wagon directions");
    })
    .expect("a valid answer");
    assert_eq!(record.things[0].name, "cart stall");
    assert_eq!(record.wares[0].name, "saffron pinch");
    assert_eq!(record.services[0].name, "wagon directions");
}

#[tokio::test(flavor = "current_thread")]
async fn creation_gives_the_storyteller_the_canon_and_the_table_and_retries_once_naming_the_rule() {
    let mut wrong = place_slot();
    wrong["speaker"] = json!("Nobody");
    let endpoint = FakeEndpoint::start(
        [
            vec![
                Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
                Reply::Output(answer(&place_slot(), &people_slot(), &things_slot())),
            ],
            persona_replies(3),
        ]
        .into_iter()
        .flatten()
        .collect(),
    );
    let cache = cache_directory("place-creation-retry");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &cache);
    let judge = accepting_persona_judge(3);
    let record = create(&storyteller, &jev(&judge), &request())
        .await
        .expect("place");
    assert_eq!(record.name, "Market Street");
    assert_eq!(record.origin.id, "place-mareks-cloth-shop");
    assert_eq!(endpoint.connections(), 5);
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
    let facts = |input: &str| -> Value {
        input
            .lines()
            .find_map(|line| line.strip_prefix("FACTS: "))
            .map(|line| serde_json::from_str(line).unwrap())
            .unwrap()
    };
    assert!(facts(first).get("previous_answer_problem").is_none());
    assert!(
        facts(second)["previous_answer_problem"]
            .as_str()
            .unwrap()
            .contains("invalid_place")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_place_the_storyteller_cannot_make_is_not_made() {
    let off = Storyteller::disabled();
    assert_eq!(
        create(&off, &ApproachModel::default(), &request())
            .await
            .unwrap_err(),
        CreationError::Off
    );
    let mut wrong = place_slot();
    wrong["kind"] = json!("dragon-lair");
    let endpoint = FakeEndpoint::start(vec![
        Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
        Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
        Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
    ]);
    let cache = cache_directory("place-creation-refused");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &cache);
    assert_eq!(
        create(&storyteller, &ApproachModel::default(), &request())
            .await
            .unwrap_err(),
        CreationError::Invalid("kind")
    );
    assert_eq!(endpoint.connections(), 3, "two retries, then it stops");
    let down = FakeEndpoint::start(
        (0..3)
            .map(|_| Reply::Status(500, json!({"error": "down"})))
            .collect(),
    );
    let storyteller = teller(
        down.address,
        0,
        Duration::from_secs(5),
        &cache_directory("place-creation-down"),
    );
    assert_eq!(
        create(&storyteller, &ApproachModel::default(), &request())
            .await
            .unwrap_err(),
        CreationError::Model
    );
    assert_eq!(down.connections(), 3, "provider failures are retried twice");
}

#[test]
fn generated_people_require_typed_voice_presentation_and_age() {
    for (field, value) in [
        ("gender", json!("male")),
        ("gender", json!("it")),
        ("age", json!("ancient")),
    ] {
        let mut people = people_slot();
        people[0][field] = value;
        assert!(
            build(
                &request(),
                &place_slot().to_string(),
                &people.to_string(),
                &things_slot().to_string()
            )
            .is_err()
        );
    }
    let mut people = people_slot();
    people[0].as_object_mut().unwrap().remove("age");
    assert_eq!(
        build(
            &request(),
            &place_slot().to_string(),
            &people.to_string(),
            &things_slot().to_string()
        ),
        Err(CreationError::Invalid("age"))
    );
}

#[test]
fn generated_voice_bands_survive_storage_and_legacy_people_remain_adult() {
    let mut people = people_slot();
    people[0]["age"] = json!("elder");
    people[1]["age"] = json!("young");
    let place = build(
        &request(),
        &place_slot().to_string(),
        &people.to_string(),
        &things_slot().to_string(),
    )
    .unwrap();
    let stored = serde_json::to_value(&place).unwrap();
    let restored: PlaceRecord = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(
        restored.expand().unwrap().cast[0].age,
        Some(super::super::pack::Age::Elder)
    );
    assert_eq!(
        restored.expand().unwrap().cast[1].age,
        Some(super::super::pack::Age::Young)
    );
    let mut legacy = stored;
    for person in legacy["people"].as_array_mut().unwrap() {
        person.as_object_mut().unwrap().remove("age");
    }
    let restored: PlaceRecord = serde_json::from_value(legacy).unwrap();
    assert!(
        restored
            .expand()
            .unwrap()
            .cast
            .iter()
            .all(|person| person.age == Some(super::super::pack::Age::Adult))
    );
}
#[test]
fn market_stalls_bind_distinct_keepers_purses_visible_positions_and_multiple_wares() {
    let market = built();
    assert_eq!(market.stalls.len(), 3);
    let expanded = market.expand().unwrap();
    for stall in &market.stalls {
        let keeper = expanded
            .model
            .people
            .iter()
            .find(|person| person.name == stall.keeper)
            .unwrap();
        assert!(keeper.coin_cp > 0);
        assert!(keeper.position.contains(&stall.name));
        assert!(
            expanded
                .model
                .things
                .iter()
                .any(|thing| thing.name == stall.name)
        );
        assert!(
            (2..=4).contains(
                &expanded
                    .model
                    .offers
                    .iter()
                    .filter(|offer| offer.seller_id == keeper.id)
                    .count()
            )
        );
    }
    let mut unknown = place_slot();
    unknown["mentioned_stalls"] = json!(["west jewel stall"]);
    assert_eq!(
        build(
            &request(),
            &unknown.to_string(),
            &people_slot().to_string(),
            &things_slot().to_string()
        )
        .unwrap_err(),
        CreationError::Invalid("mentioned_stall")
    );
    let mut orphan = place_slot();
    orphan["stalls"][0]["keeper"] = json!("Nobody");
    assert!(
        build(
            &request(),
            &orphan.to_string(),
            &people_slot().to_string(),
            &things_slot().to_string()
        )
        .is_err()
    );
    let mut incomplete = place_slot();
    incomplete["wares"]
        .as_array_mut()
        .unwrap()
        .retain(|ware| ware["name"] != "trout");
    assert!(
        build(
            &request(),
            &incomplete.to_string(),
            &people_slot().to_string(),
            &things_slot().to_string()
        )
        .is_err()
    );
}

#[tokio::test]
async fn generated_market_retries_a_single_vendor_layout_then_admits_several_stalls() {
    let mut missing = place_slot();
    missing["stalls"] = json!([missing["stalls"][1].clone()]);
    missing["mentioned_stalls"] = json!(["spice stall"]);
    missing["wares"]
        .as_array_mut()
        .unwrap()
        .retain(|ware| ware["seller"] == "Wenna");
    let endpoint = FakeEndpoint::start(
        [
            vec![
                Reply::Output(answer(&missing, &people_slot(), &things_slot())),
                Reply::Output(answer(&place_slot(), &people_slot(), &things_slot())),
            ],
            persona_replies(3),
        ]
        .into_iter()
        .flatten()
        .collect(),
    );
    let cache = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS40/market-generation")
        .join(std::process::id().to_string());
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &cache);
    let judge = accepting_persona_judge(3);
    let market = create(&storyteller, &jev(&judge), &request())
        .await
        .unwrap();
    assert_eq!(market.stalls.len(), 3);
    assert_eq!(endpoint.connections(), 5);
    assert!(
        endpoint.bodies()[1]["input"]
            .as_str()
            .unwrap()
            .contains("invalid_place")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn hs69_seeded_personas_are_distinct_persisted_and_old_record_migration_is_idempotent() {
    let asked = request();
    let mut record = built();
    let old_json = serde_json::to_string(&record).unwrap();
    assert!(!old_json.contains("temperament"));
    record = serde_json::from_str(&old_json).unwrap();
    let endpoint = FakeEndpoint::start(persona_replies(3));
    let judge = accepting_persona_judge(3);
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS69/persona-migration")
        .join(std::process::id().to_string());
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    author_personas(&storyteller, &jev(&judge), &asked, &mut record)
        .await
        .unwrap();
    for (index, person) in record.people.iter().enumerate() {
        let persona = person.persona.as_ref().unwrap();
        assert!(persona.validate());
        assert!(record.people.iter().take(index).all(|other| {
            persona
                .temperament
                .differences(other.persona.as_ref().unwrap().temperament)
                >= 2
        }));
        println!(
            "HS69 PERSONA {} {}",
            person.name,
            serde_json::to_string(persona).unwrap()
        );
    }
    let saved = serde_json::to_string(&record).unwrap();
    let mut loaded: PlaceRecord = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded, record);
    author_personas(
        &Storyteller::disabled(),
        &ApproachModel::default(),
        &asked,
        &mut loaded,
    )
    .await
    .unwrap();
    assert_eq!(loaded, record);
    let expanded = loaded.expand().unwrap();
    for (draft, person) in loaded.people.iter().zip(&expanded.model.people) {
        assert_eq!(draft.persona, person.persona);
    }
    assert_eq!(endpoint.connections(), 3);
    assert_eq!(judge.connections(), 3);
    let mut reordered = built();
    reordered.people.reverse();
    let reordered_endpoint = FakeEndpoint::start(persona_replies(3));
    let reordered_judge = accepting_persona_judge(3);
    let reordered_teller = teller(
        reordered_endpoint.address,
        0,
        Duration::from_secs(5),
        &directory.join("reordered"),
    );
    author_personas(
        &reordered_teller,
        &jev(&reordered_judge),
        &asked,
        &mut reordered,
    )
    .await
    .unwrap();
    for person in &reordered.people {
        assert_eq!(
            person.persona,
            record
                .people
                .iter()
                .find(|other| other.name == person.name)
                .unwrap()
                .persona
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn hs69_jev_rejection_regenerates_only_the_rejected_person() {
    let mut record = built();
    let place_before = record.clone();
    let endpoint = FakeEndpoint::start(persona_replies(4));
    let judge = FakeEndpoint::start(vec![choice("o2"), choice("o1"), choice("o1"), choice("o1")]);
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS69/persona-retry")
        .join(std::process::id().to_string());
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    author_personas(&storyteller, &jev(&judge), &request(), &mut record)
        .await
        .unwrap();
    let bodies = endpoint.bodies();
    let facts: Vec<Value> = bodies
        .iter()
        .map(|body| {
            body["input"]
                .as_str()
                .unwrap()
                .lines()
                .find_map(|line| line.strip_prefix("FACTS: "))
                .map(|line| serde_json::from_str(line).unwrap())
                .unwrap()
        })
        .collect();
    assert_eq!(
        facts
            .iter()
            .map(|facts| facts["person"]["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Hobb", "Hobb", "Wenna", "Yara"]
    );
    assert_eq!(facts[0]["drawn_temperament"], facts[1]["drawn_temperament"]);
    assert!(
        !facts[1]["previous_answer_problem"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    for person in &mut record.people {
        person.persona = None;
    }
    assert_eq!(record, place_before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_retry_supplies_job_and_logs_the_jev_rejection_reason() {
    let mut record = built();
    record.place_id = "place-roamstart-retry".into();
    let endpoint = FakeEndpoint::start(persona_replies(4));
    let judge = FakeEndpoint::start(vec![choice("o2"), choice("o1"), choice("o1"), choice("o1")]);
    let directory = cache_directory("roamstart-retry");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);

    author_personas(&storyteller, &jev(&judge), &request(), &mut record)
        .await
        .unwrap();

    let bodies = endpoint.bodies();
    let facts: Vec<Value> = bodies
        .iter()
        .map(|body| {
            body["input"]
                .as_str()
                .unwrap()
                .lines()
                .find_map(|line| line.strip_prefix("FACTS: "))
                .map(|line| serde_json::from_str(line).unwrap())
                .unwrap()
        })
        .collect();
    assert_eq!(facts[0]["person_job"], "carter");
    assert_eq!(facts[0]["place"]["kind"], "market-street");
    assert_eq!(facts[0]["place"]["person_position"], "beside the wagon");
    assert_eq!(facts[0]["drawn_temperament"], facts[1]["drawn_temperament"]);
    assert!(
        facts[1]["previous_answer_problem"]
            .as_str()
            .unwrap()
            .contains("job")
    );
    let lines = super::super::actor::audit_lines_containing("PERSONA place=place-roamstart-retry");
    assert!(lines.iter().any(|line| {
        line.contains("person=Hobb attempt=0 outcome=rejected reason=job_or_history")
    }));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("person=Hobb attempt=1 outcome=accepted"))
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_encoded_forbidden_name_is_audited_with_its_own_reason() {
    use std::sync::Arc;

    let mut record = built();
    record.place_id = "place-roamstart-forbidden-name".into();
    let rejected = Arc::new(|prompt: &str| {
        let mut answer = persona_answer(prompt);
        let mut persona: Value = serde_json::from_str(answer["persona"].as_str().unwrap()).unwrap();
        persona["backstory"] = json!("Brynn worked there.");
        let encoded = persona.to_string().replace("Brynn", r"\u0042rynn");
        answer["persona"] = json!(encoded);
        answer
    });
    let endpoint = FakeEndpoint::start(
        [vec![Reply::Compute(rejected)], persona_replies(3)]
            .into_iter()
            .flatten()
            .collect(),
    );
    let judge = accepting_persona_judge(3);
    let directory = cache_directory("roamstart-forbidden-name");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);

    author_personas(&storyteller, &jev(&judge), &request(), &mut record)
        .await
        .unwrap();

    let lines =
        super::super::actor::audit_lines_containing("PERSONA place=place-roamstart-forbidden-name");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("outcome=rejected reason=forbidden_hero_name"))
    );
    assert!(lines.iter().all(|line| !line.contains("Brynn")));
    assert_eq!(endpoint.connections(), 4);
    assert_eq!(judge.connections(), 3);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_wrong_temperament_is_audited_and_retried() {
    use std::sync::Arc;

    let mut record = built();
    record.place_id = "place-roamstart-wrong-draw".into();
    let rejected = Arc::new(|prompt: &str| {
        let mut answer = persona_answer(prompt);
        let mut persona: Value = serde_json::from_str(answer["persona"].as_str().unwrap()).unwrap();
        persona["temperament"]["energy"] = if persona["temperament"]["energy"] == "low" {
            json!("high")
        } else {
            json!("low")
        };
        answer["persona"] = json!(persona.to_string());
        answer
    });
    let endpoint = FakeEndpoint::start(
        [vec![Reply::Compute(rejected)], persona_replies(3)]
            .into_iter()
            .flatten()
            .collect(),
    );
    let judge = accepting_persona_judge(3);
    let directory = cache_directory("roamstart-wrong-draw");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);

    author_personas(&storyteller, &jev(&judge), &request(), &mut record)
        .await
        .unwrap();

    let lines =
        super::super::actor::audit_lines_containing("PERSONA place=place-roamstart-wrong-draw");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("outcome=rejected reason=shape_or_draw"))
    );
    assert_eq!(endpoint.connections(), 4);
    assert_eq!(judge.connections(), 3);
    std::fs::remove_dir_all(directory).unwrap();
}

fn roamstart_city_answer(prompt: &str) -> Value {
    use sha2::{Digest, Sha256};

    let facts: Value = prompt
        .lines()
        .find_map(|line| line.strip_prefix("FACTS: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .unwrap();
    if facts["task"] == "person_persona" {
        let mut answer = persona_answer(prompt);
        let mut persona: Value = serde_json::from_str(answer["persona"].as_str().unwrap()).unwrap();
        persona["backstory"] = json!("Rejected private sentinel.");
        answer["persona"] = json!(persona.to_string());
        return answer;
    }
    if facts["task"] == "free_roam_city" {
        return json!({
            "name": "Roamstead", "character": "a canal city of pale stone and busy workshops",
            "mood": "unhurried civic life", "time_of_day": "afternoon", "weather": "dry and cool",
        });
    }
    let gate = facts["origin"]["name"] == "Road outside Roamstead";
    let identity = format!("{:x}", Sha256::digest(facts.to_string().as_bytes()));
    let identity = &identity[..8];
    let mut place = place_slot();
    place["name"] = json!(if gate {
        "Roamstead Main Gate".to_owned()
    } else {
        format!("Roamstead Walk {identity}")
    });
    place["short"] = json!("a public street within Roamstead's walls");
    place["narration"] = json!("You stand inside the city walls as carts roll past the arch.");
    place["speaker"] = json!("");
    place["greeting"] = json!("");
    place["wares"] = json!([]);
    place["services"] = json!([]);
    let roles = if gate {
        vec!["guard", "carter", "hawker", "traveller"]
    } else {
        vec!["potter", "gardener"]
    };
    let people = roles
        .iter()
        .enumerate()
        .map(|(index, role)| {
            json!({
                "name": format!("Local{identity}{index}"), "role": role,
                "description": format!("An adult {role} in a plain coat."),
                "position": format!("beside post {index}"), "gender": "they",
                "wants": "Finish the day's work.", "attitude": "indifferent",
                "stat_block": if *role == "guard" { "guard" } else { "commoner" },
                "knowledge": ["This is Roamstead.", "The Bell Tower is uphill.", "I work here."],
            })
        })
        .collect::<Vec<_>>();
    answer(&place, &json!(people), &things_slot())
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_city_and_lookahead_start_when_every_persona_is_rejected() {
    use std::sync::Arc;

    let compute = Arc::new(roamstart_city_answer);
    let endpoint = FakeEndpoint::start((0..64).map(|_| Reply::Compute(compute.clone())).collect());
    let judge = FakeEndpoint::start((0..48).map(|_| choice("o2")).collect());
    let directory = cache_directory("roamstart-rejected-city");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);

    let generated = super::super::adventure::free_roam::generate(
        3_917_472,
        super::super::adventure::RulesMode::Standard,
        super::super::startup_configuration::ContentRating::default(),
        &storyteller,
        &jev(&judge),
    )
    .await
    .unwrap();
    let world = generated
        .pack
        .beat_script
        .as_ref()
        .unwrap()
        .free_roam
        .as_ref()
        .unwrap();
    assert_eq!(world.gate.people.len(), 4);
    assert_eq!(world.ahead.len(), 2);
    let documents = generated.documents(true).unwrap();
    let (loaded, _) = super::super::adventure::record::reload(&documents).unwrap();
    assert_eq!(
        loaded.pack.beat_script.as_ref().unwrap().free_roam.as_ref(),
        Some(world)
    );
    let mut saw_rejection = false;
    for place in std::iter::once(&world.gate).chain(world.ahead.iter().map(|item| &item.place)) {
        place.validate().unwrap();
        for person in &place.people {
            let persona = person.persona.as_ref().unwrap();
            assert!(persona.validate());
            assert!(persona.speech.contains(person.role.trim()));
            assert!(!format!("{persona:?}").contains("Rejected private sentinel"));
        }
        let lines = super::super::actor::audit_lines_containing(&format!(
            "PERSONA place={}",
            place.place_id
        ));
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("outcome=fallback"))
                .count(),
            place.people.len()
        );
        saw_rejection |= lines
            .iter()
            .any(|line| line.contains("outcome=rejected reason=job_or_history"));
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("Rejected private sentinel"))
        );
    }
    assert!(saw_rejection);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_five_rejections_exhaust_the_retry_limit_and_use_fallback() {
    let mut record = built();
    record.place_id = "place-roamstart-five-rejections".into();
    let person = &record.people[0];
    let asked = request();
    let temperament =
        place::Temperament::draw(&asked.run_seed, &record.place_id, &person.name, &[]).unwrap();
    let endpoint = FakeEndpoint::start(persona_replies(5));
    let judge = FakeEndpoint::start((0..5).map(|_| choice("o2")).collect());
    let directory = cache_directory("roamstart-five-rejections");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);

    let persona = author_persona(
        &storyteller,
        &jev(&judge),
        &asked,
        &record,
        person,
        temperament,
        Duration::from_secs(120),
    )
    .await;

    assert!(persona.validate());
    assert!(persona.speech.contains(person.role.trim()));
    assert_eq!(endpoint.connections(), 5);
    assert_eq!(judge.connections(), 5);
    let lines = super::super::actor::audit_lines_containing(
        "PERSONA place=place-roamstart-five-rejections",
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("outcome=rejected reason=job_or_history"))
            .count(),
        5
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("outcome=fallback reason=attempts_exhausted"))
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_disabled_storyteller_uses_valid_job_and_drawn_temperament() {
    let mut record = built();
    record.place_id = "place-roamstart-disabled".into();
    record.people[0].role = "   Dr. gate! guard?   ".into();
    let job = record.people[0].role.trim().to_owned();
    let expected = place::Temperament::draw(
        &request().run_seed,
        &record.place_id,
        &record.people[0].name,
        &[],
    )
    .unwrap();

    author_personas(
        &Storyteller::disabled(),
        &ApproachModel::default(),
        &request(),
        &mut record,
    )
    .await
    .unwrap();

    let persona = record.people[0].persona.as_ref().unwrap();
    assert_eq!(persona.temperament, expected);
    assert!(persona.validate());
    assert!(persona.speech.contains(&job));
    let traits = serde_json::to_value(expected).unwrap();
    assert!(
        persona
            .backstory
            .contains(traits["energy"].as_str().unwrap())
    );
    for axis in ["nerve", "warmth", "talk", "humour"] {
        assert!(persona.speech.contains(traits[axis].as_str().unwrap()));
    }
    assert!(
        record
            .people
            .iter()
            .all(|person| person.persona.as_ref().unwrap().validate())
    );
    let lines =
        super::super::actor::audit_lines_containing("PERSONA place=place-roamstart-disabled");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("outcome=fallback reason=storyteller_off"))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_unavailable_jev_uses_fallback_without_copying_model_text() {
    let mut record = built();
    record.place_id = "place-roamstart-no-jev".into();
    let endpoint = FakeEndpoint::start(persona_replies(3));
    let directory = cache_directory("roamstart-no-jev");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);

    author_personas(
        &storyteller,
        &ApproachModel::default(),
        &request(),
        &mut record,
    )
    .await
    .unwrap();

    assert!(record.people.iter().all(|person| {
        let persona = person.persona.as_ref().unwrap();
        persona.validate() && persona.speech.contains(person.role.trim())
    }));
    let lines = super::super::actor::audit_lines_containing("PERSONA place=place-roamstart-no-jev");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("outcome=fallback reason=jev_provider_unavailable"))
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_storyteller_failure_uses_fallback_without_logging_provider_content() {
    let mut record = built();
    record.place_id = "place-roamstart-provider-failure".into();
    let endpoint = FakeEndpoint::start(
        (0..12)
            .map(|_| {
                Reply::Status(
                    500,
                    json!({"error": {"message": "private provider sentinel"}}),
                )
            })
            .collect(),
    );
    let directory = cache_directory("roamstart-provider-failure");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);

    author_personas(
        &storyteller,
        &ApproachModel::default(),
        &request(),
        &mut record,
    )
    .await
    .unwrap();

    assert!(record.people.iter().all(|person| {
        let persona = person.persona.as_ref().unwrap();
        persona.validate() && persona.speech.contains(person.role.trim())
    }));
    let lines = super::super::actor::audit_lines_containing(
        "PERSONA place=place-roamstart-provider-failure",
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("outcome=fallback reason=storyteller_provider_failure"))
            .count(),
        record.people.len()
    );
    assert!(
        lines
            .iter()
            .all(|line| !line.contains("private provider sentinel"))
    );
    assert!(super::super::actor::audit_lines_containing("private provider sentinel").is_empty());
    if directory.exists() {
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_expired_persona_deadline_uses_safe_fallback() {
    assert_eq!(PERSONA_DEADLINE, Duration::from_secs(20));
    let mut record = built();
    record.place_id = "place-roamstart-deadline".into();
    let person = &record.people[0];
    let asked = request();
    let temperament =
        place::Temperament::draw(&asked.run_seed, &record.place_id, &person.name, &[]).unwrap();
    let endpoint = FakeEndpoint::start(vec![Reply::Hang]);
    let judge = accepting_persona_judge(1);
    let directory = cache_directory("roamstart-deadline");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);

    let persona = author_persona(
        &storyteller,
        &jev(&judge),
        &asked,
        &record,
        person,
        temperament,
        Duration::ZERO,
    )
    .await;

    assert_eq!(persona.temperament, temperament);
    assert!(persona.validate());
    assert!(persona.speech.contains(person.role.trim()));
    assert_eq!(judge.connections(), 0);
    let lines =
        super::super::actor::audit_lines_containing("PERSONA place=place-roamstart-deadline");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("outcome=fallback reason=deadline"))
    );
    if directory.exists() {
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn hs69_draw_uses_run_place_and_name_and_persona_bounds_are_strict() {
    let draw = |run: &str, place: &str, name: &str| {
        place::Temperament::draw(run, place, name, &[]).unwrap()
    };
    let first = draw("run-a", "place-a", "Hobb");
    assert_eq!(first, draw("run-a", "place-a", "Hobb"));
    assert!((0..20).any(|index| draw(&format!("run-{index}"), "place-a", "Hobb") != first));
    assert!((0..20).any(|index| draw("run-a", &format!("place-{index}"), "Hobb") != first));
    assert!((0..20).any(|index| draw("run-a", "place-a", &format!("Local{index}")) != first));
    let mut persona = place::PersonPersona {
        temperament: first,
        backstory: "Cart work taught a patient manner.".into(),
        speech: "Slow, plain trade words.".into(),
        catchphrase: "Easy on the axle.".into(),
    };
    assert!(persona.validate());
    persona.catchphrase = "x".repeat(65);
    assert!(!persona.validate());
    persona.catchphrase = "Easy.".into();
    persona.speech = format!("{}Slow.{}", " ".repeat(100), " ".repeat(100));
    assert!(!persona.validate());
    persona.speech = "Slow.".into();
    persona.backstory = "One. Two. Three.".into();
    assert!(!persona.validate());
}

#[tokio::test(flavor = "current_thread")]
async fn hs69_identity_pass_cannot_supply_an_unjudged_persona() {
    let malicious = json!({"temperament":{"energy":"high","nerve":"bold","warmth":"cool","talk":"terse","humour":"dry"},
        "backstory":"A different trade taught a different manner.","speech":"An unrelated voice.","catchphrase":"Unjudged."});
    let mut people = people_slot();
    for person in people.as_array_mut().unwrap() {
        person["persona"] = malicious.clone();
    }
    let endpoint = FakeEndpoint::start(
        [
            vec![Reply::Output(answer(
                &place_slot(),
                &people,
                &things_slot(),
            ))],
            persona_replies(3),
        ]
        .into_iter()
        .flatten()
        .collect(),
    );
    let judge = accepting_persona_judge(3);
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS69/persona-identity")
        .join(std::process::id().to_string());
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    let record = create(&storyteller, &jev(&judge), &request())
        .await
        .unwrap();
    assert!(
        record
            .people
            .iter()
            .all(|person| person.persona.as_ref().unwrap().catchphrase != "Unjudged.")
    );
    assert_eq!(judge.connections(), 3);
    assert_eq!(endpoint.connections(), 4);
    std::fs::remove_dir_all(directory).unwrap();
}

pub(in crate::gameplay) fn roamstart_round2_endpoint(
    rejected_gate: bool,
    rejected_neighbour_attempts: usize,
) -> FakeEndpoint {
    use std::sync::Arc;

    let compute = Arc::new(|prompt: &str| {
        let facts: Value = prompt
            .lines()
            .find_map(|line| line.strip_prefix("FACTS: "))
            .map(|line| serde_json::from_str(line).unwrap())
            .unwrap();
        if facts["task"] == "person_persona" {
            persona_answer(prompt)
        } else {
            roamstart_city_answer(prompt)
        }
    });
    let mut replies = vec![Reply::Compute(compute.clone())];
    if !rejected_gate {
        replies.extend((0..5).map(|_| Reply::Compute(compute.clone())));
    }
    for _ in 0..(if rejected_gate {
        3
    } else {
        rejected_neighbour_attempts
    }) {
        for _ in 0..2 {
            replies.push(Reply::Compute(compute.clone()));
            replies.push(Reply::Output(json!({
                "decision": "published_non_srd", "flagged_span": ""
            })));
        }
    }
    replies.extend((0..64).map(|_| Reply::Compute(compute.clone())));
    FakeEndpoint::start(replies)
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_round2_refused_neighbour_is_retried_with_its_content_category() {
    let endpoint = roamstart_round2_endpoint(false, 1);
    let judge = accepting_persona_judge(8);
    let directory = cache_directory("roamstart-round2-retry");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    let generated = super::super::adventure::free_roam::generate(
        4_017_472,
        super::super::adventure::RulesMode::Standard,
        super::super::startup_configuration::ContentRating::default(),
        &storyteller,
        &jev(&judge),
    )
    .await
    .expect("a revised neighbour permits the city to start");
    let world = generated
        .pack
        .beat_script
        .as_ref()
        .unwrap()
        .free_roam
        .as_ref()
        .unwrap();
    assert_eq!(world.ahead.len(), 2);
    let bodies = endpoint.bodies();
    assert!(bodies.iter().any(|body| {
        body["input"]
            .as_str()
            .is_some_and(|input| input.contains("the server reports the previous refusal"))
    }));
    let retry = bodies
        .iter()
        .find_map(|body| {
            let input = body["input"].as_str()?;
            let facts: Value = input
                .lines()
                .find_map(|line| line.strip_prefix("FACTS: "))
                .and_then(|line| serde_json::from_str(line).ok())?;
            (facts["task"] == "create_place" && facts["attempt"] == 1).then_some(facts)
        })
        .expect("a second place expression was sent");
    let feedback = retry["previous_answer_problem"].as_str().unwrap();
    assert!(feedback.contains("published_non_srd_material"));
    assert!(feedback.contains("original") && feedback.contains("SRD 5.2.1"));
    assert!(!feedback.contains("Rejected private sentinel"));
    assert!(
        super::super::actor::audit_lines_containing("PLACE generation")
            .iter()
            .any(|line| line.contains("published_non_srd_material"))
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_round2_permanently_refused_neighbour_is_deferred_and_other_place_survives() {
    let endpoint = roamstart_round2_endpoint(false, 3);
    let judge = accepting_persona_judge(6);
    let directory = cache_directory("roamstart-round2-deferred");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    let generated = super::super::adventure::free_roam::generate(
        4_017_473,
        super::super::adventure::RulesMode::Standard,
        super::super::startup_configuration::ContentRating::default(),
        &storyteller,
        &jev(&judge),
    )
    .await
    .expect("a speculative neighbour cannot prevent the city from starting");
    let world = generated
        .pack
        .beat_script
        .as_ref()
        .unwrap()
        .free_roam
        .as_ref()
        .unwrap();
    assert_eq!(world.gate.people.len(), 4);
    assert_eq!(world.ahead.len(), 1);
    let mut malformed = world.clone();
    assert!(malformed.gate.exits.pop().is_some());
    malformed.ahead.clear();
    malformed.gate.validate().unwrap();
    assert!(malformed.validate(&generated.pack).is_err());
    let scene = generated
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == super::super::pack::SceneKey::Opening)
        .unwrap();
    let ways = &scene
        .setting
        .as_ref()
        .unwrap()
        .model
        .as_ref()
        .unwrap()
        .ways_out;
    let missing = ways
        .iter()
        .find(|way| way.id != world.ahead[0].exit)
        .unwrap();
    assert!(
        missing.open,
        "the deferred exit remains available for on-demand creation"
    );
    let (loaded, _) =
        super::super::adventure::record::reload(&generated.documents(true).unwrap()).unwrap();
    assert_eq!(
        loaded.pack.beat_script.as_ref().unwrap().free_roam.as_ref(),
        Some(world)
    );
    let expected = format!(
        "FREE_ROAM generation stage=look_ahead outcome=deferred place={}",
        missing.id
    );
    assert!(!super::super::actor::audit_lines_containing(&expected).is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_round2_refused_starting_gate_still_reports_start_failure() {
    let endpoint = roamstart_round2_endpoint(true, 0);
    let directory = cache_directory("roamstart-round2-failed-gate");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    let error = super::super::adventure::free_roam::generate(
        4_017_474,
        super::super::adventure::RulesMode::Standard,
        super::super::startup_configuration::ContentRating::default(),
        &storyteller,
        &ApproachModel::default(),
    )
    .await
    .expect_err("a refused starting gate fails the start");
    assert_eq!(error, CreationError::Model);
    assert!(
        !super::super::actor::audit_lines_containing(
            "FREE_ROAM generation stage=gate outcome=failed place=main-gate"
        )
        .is_empty()
    );
    assert!(
        !super::super::actor::audit_lines_containing(
            "FREE_ROAM generation stage=city_and_gate outcome=failed place=main-gate"
        )
        .is_empty()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_round2_both_refused_neighbours_leave_both_exits_available() {
    let endpoint = roamstart_round2_endpoint(false, 6);
    let judge = accepting_persona_judge(4);
    let directory = cache_directory("roamstart-round2-both-deferred");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    let generated = super::super::adventure::free_roam::generate(
        4_017_475,
        super::super::adventure::RulesMode::Standard,
        super::super::startup_configuration::ContentRating::default(),
        &storyteller,
        &jev(&judge),
    )
    .await
    .unwrap();
    let world = generated
        .pack
        .beat_script
        .as_ref()
        .unwrap()
        .free_roam
        .as_ref()
        .unwrap();
    assert!(world.ahead.is_empty());
    let opening = generated
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == super::super::pack::SceneKey::Opening)
        .unwrap();
    let ways = &opening
        .setting
        .as_ref()
        .unwrap()
        .model
        .as_ref()
        .unwrap()
        .ways_out;
    assert_eq!(ways.len(), 2);
    assert!(ways.iter().all(|way| way.open));
    let (loaded, _) =
        super::super::adventure::record::reload(&generated.documents(true).unwrap()).unwrap();
    assert_eq!(
        loaded.pack.beat_script.as_ref().unwrap().free_roam.as_ref(),
        Some(world)
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_round2_provider_failure_is_retried_with_a_safe_category() {
    let endpoint = FakeEndpoint::start(
        [
            vec![
                Reply::Status(500, json!({"error": "private provider sentinel"})),
                Reply::Output(answer(&place_slot(), &people_slot(), &things_slot())),
            ],
            persona_replies(3),
        ]
        .into_iter()
        .flatten()
        .collect(),
    );
    let judge = accepting_persona_judge(3);
    let directory = cache_directory("roamstart-round2-provider-retry");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    let record = create(&storyteller, &jev(&judge), &request())
        .await
        .unwrap();
    assert_eq!(record.name, "Market Street");
    let bodies = endpoint.bodies();
    let retry = bodies[1]["input"].as_str().unwrap();
    assert!(retry.contains("provider_failure"));
    assert!(!retry.contains("private provider sentinel"));
    assert_eq!(endpoint.connections(), 5);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn roamstart_round2_expired_place_deadline_does_not_send_an_attempt() {
    let endpoint = FakeEndpoint::start(vec![Reply::Hang]);
    let directory = cache_directory("roamstart-round2-expired-deadline");
    let storyteller = teller(endpoint.address, 0, Duration::from_secs(5), &directory);
    assert_eq!(
        express_place(&storyteller, &request(), false, false, Duration::ZERO)
            .await
            .unwrap_err(),
        CreationError::Model
    );
    assert_eq!(endpoint.connections(), 0);
}
