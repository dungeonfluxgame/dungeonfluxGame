//! Boundary tests for changing place on the authored cloth shop: the exits of the shop, a place
//! created from the street door, its picture-bearing scene and cast on the display view, the return
//! to the shop and the street, and the table being told only what was committed. The model is a
//! loopback fake; no live provider is called and no server is started.

use super::*;
use crate::gameplay::{
    adventure::setting::SettingKind,
    adventure::{RulesMode, record, shop},
    approach_model::ApproachModel,
    beat_script_runtime as runtime, beat_script_variety_tests as native,
    freeform_action::{Performance, StrikeMeans},
    investigation,
    living_scenes::LivingScenes,
    npc_reply::{NpcVoice, local_lore},
    place::{PlaceFailure, PlaceRecord, PlaceRefusal},
    place_creation::tests::{answer, people_slot, place_slot, request, things_slot},
    social::SocialLedger,
    startup_configuration::{ApproachSettings, ContentRating, VoiceOff},
    storyteller::{
        Storyteller,
        tests::{FakeEndpoint as RawEndpoint, Reply, cache_directory, teller},
    },
    table_log::TableLog,
    voice::Voice,
};
use df_persistence::local_demo_scope::LocalDemoRole;
use df_session::inbox::{ActorLoop, AdmissionSequence, Reducer, bounded_inbox};
use std::sync::Arc;
use tokio::sync::{Semaphore, watch};

/// Existing move scripts describe routing, place identity and travel narration. Persona calls
/// are a separate stage: serve their fixtures without consuming a scripted route/travel answer.
struct FakeEndpoint {
    endpoint: RawEndpoint,
    address: std::net::SocketAddr,
}

fn persona_request(body: &Value) -> bool {
    body["state"]["task"] == "person_persona_judgement"
        || body["input"]
            .as_str()
            .and_then(|prompt| prompt.lines().find_map(|line| line.strip_prefix("FACTS: ")))
            .and_then(|facts| serde_json::from_str::<Value>(facts).ok())
            .is_some_and(|facts| facts["task"] == "person_persona")
}

fn completed_fixture(body: &Value, output: String) -> Value {
    json!({"object":"response","id":"resp_hs69_fixture","status":"completed",
        "model":body["model"], "metadata":body["metadata"], "text":body["text"],
        "error":null,"incomplete_details":null,
        "output":[{"type":"message","role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":output,"annotations":[]}]}],
        "usage":{"input_tokens":100,"output_tokens":40,"total_tokens":140,
            "input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}}})
}

impl FakeEndpoint {
    fn start(script: Vec<Reply>) -> Self {
        let script = std::sync::Mutex::new(std::collections::VecDeque::from(script));
        let respond = Arc::new(move |body: &Value| {
            if body["state"]["task"] == "person_persona_judgement" {
                let Reply::Status(status, response) = choice("o1") else {
                    unreachable!()
                };
                return (status, response);
            }
            if persona_request(body) {
                let output = crate::gameplay::place_creation::tests::persona_answer(
                    body["input"].as_str().unwrap(),
                );
                return (200, completed_fixture(body, output.to_string()));
            }
            let Some(reply) = script.lock().unwrap().pop_front() else {
                return (503, json!({"error":"primary fixture script exhausted"}));
            };
            match reply {
                Reply::Status(status, response) => (status, response),
                Reply::Output(output) => (200, completed_fixture(body, output.to_string())),
                Reply::OutputText(output) => (200, completed_fixture(body, output)),
                Reply::Compute(compute) => (
                    200,
                    completed_fixture(body, compute(body["input"].as_str().unwrap()).to_string()),
                ),
                Reply::ComputeResponse(compute) => compute(body),
                _ => panic!("unsupported primary script in the persona-aware move fixture"),
            }
        });
        let endpoint = RawEndpoint::start(
            (0..128)
                .map(|_| Reply::ComputeResponse(respond.clone()))
                .collect(),
        );
        let address = endpoint.address;
        Self { endpoint, address }
    }

    fn primary_bodies(&self) -> Vec<Value> {
        self.endpoint
            .bodies()
            .into_iter()
            .filter(|body| !persona_request(body))
            .collect()
    }

    fn primary_connections(&self) -> usize {
        self.primary_bodies().len()
    }
}

const HOME: &str = "place-mareks-cloth-shop";
const STREET: &str = "place-market-street";

/// The shop in freeplay with both heroes created, after "begin story".
fn with_shop<R>(run: impl FnOnce(Checkpoint, pack::LoadedPack) -> R) -> R {
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let documents = generated.documents(true).unwrap();
    let (loaded, _) = record::reload(&documents).unwrap();
    let kept = loaded.clone();
    pack::with_test_pack(loaded, || {
        let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
        run(opening, kept)
    })
}

fn street(shop_scene_model: &SceneModel) -> PlaceRecord {
    let mut asked = request();
    asked.time_of_day = shop_scene_model.time_of_day.clone();
    asked.weather = shop_scene_model.weather.clone();
    asked.origin_name = "Marek\u{2019}s Cloth Shop".to_owned();
    place_creation::build(
        &asked,
        &place_slot().to_string(),
        &people_slot().to_string(),
        &things_slot().to_string(),
    )
    .expect("the fixture street is valid")
}

/// A newly admitted market already has personas; legacy None records remain in `street()`
/// for the separate serialization/migration cases.
fn admitted_market(shop_scene_model: &SceneModel) -> PlaceRecord {
    let mut record = street(shop_scene_model);
    let mut drawn = Vec::new();
    for person in &mut record.people {
        let temperament = place::Temperament::draw(
            "hs69-market-fixture",
            &record.place_id,
            &person.name,
            &drawn,
        )
        .unwrap();
        drawn.push(temperament);
        let answer = place_creation::tests::persona_answer(&format!(
            "FACTS: {}",
            json!({
                "person":person, "drawn_temperament":temperament,
            })
        ));
        person.persona = Some(serde_json::from_str(answer["persona"].as_str().unwrap()).unwrap());
    }
    record.validate().unwrap();
    record
}

fn operation(number: u8) -> OperationId {
    OperationId::from_bytes(&[number; 16]).unwrap()
}

fn fruit_picture(plate_key: &str) -> journey::living_scene::PictureFeatures {
    journey::living_scene::PictureFeatures {
        place: STREET.into(),
        plate_key: plate_key.into(),
        seen_in_picture: vec![journey::living_scene::VisibleFeature {
            name: "fruit stand".into(),
            look: "A canvas stall with red apples in shallow baskets.".into(),
            region: [650, 220, 920, 600],
            recorded_id: None,
        }],
    }
}

#[test]
fn established_observation_detail_commits_with_its_answer_and_survives_replay() {
    use prost::Message;
    with_shop(|opening, _| {
        let choices = [
            ("attempt-skill", "investigation"),
            ("attempt-thing", "thing-shop-counter"),
        ]
        .into_iter()
        .map(|(group, value)| rpc::JourneyChoice {
            group_id: group.into(),
            option_id: value.into(),
        })
        .collect();
        let (kind, _, offer_id) = runtime::view_offers(&opening, native::first())
            .unwrap()
            .into_iter()
            .find(|(kind, _, _)| *kind == rpc::GameplayActionKind::ExamineHarborSeal)
            .unwrap();
        let basis = opening.basis();
        let request = rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(basis.session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(basis.run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(basis.revision)),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![77; 16]),
            }),
            offer_id,
            action_kind: kind as i32,
            action_choices: choices,
            ..Default::default()
        };
        let input = wire::journey_input(&request, native::first(), &opening).unwrap();
        let checked = journey::stage_with_supplier(&opening, &input, &mut |_| Ok(18)).unwrap();
        let operation = checked.state().decisions.last().unwrap().operation;
        let check = journey::accepted(checked.state().decisions.last().unwrap())
            .unwrap()
            .check
            .unwrap();
        let scene = runtime::scene(checked.state()).unwrap();
        let who = journey::player_entity(native::first(), &checked).unwrap();
        let source_hash = journey::living_scene::observation_source_hash(&scene).unwrap();
        let detail = journey::living_scene::EstablishedDetail {
            target: "thing-shop-counter".into(),
            question: "Does the counter wobble?".into(),
            detail: "Its legs sit flat on the boards.".into(),
            source_hash: source_hash.clone(),
            consistency_approved: true,
        };
        let record = journey::living_scene::ObservationFinding {
            id: format!("observation-{}", journey::hex(operation.as_bytes())),
            operation: journey::hex(operation.as_bytes()),
            actor: journey::hex(who.as_bytes()),
            hero: journey::name(checked.state(), who).unwrap(),
            audience: "shared".into(),
            check: journey::hex(&check.encode_to_vec()),
            target: detail.target.clone(),
            source_revision: check.source_revision.clone(),
            source_epoch: checked.basis().revision.epoch().get(),
            source_sequence: checked.basis().revision.sequence(),
            source_hash: source_hash.clone(),
            scene: scene.key,
            selected_source_ids: vec![],
            text: "Brynn finds the counter steady on its legs.".into(),
            unavailable: false,
        };
        let mut denied = detail.clone();
        denied.consistency_approved = false;
        assert!(
            journey::living_scene::commit_observation_with_detail(
                &checked,
                checked.basis(),
                record.clone(),
                Some(denied)
            )
            .is_err()
        );
        let completed = journey::living_scene::commit_observation_with_detail(
            &checked,
            checked.basis(),
            record.clone(),
            Some(detail.clone()),
        )
        .unwrap();
        assert_eq!(
            journey::living_scene::established_detail_for(completed.state(), HOME, &detail.target)
                .unwrap(),
            Some(detail.clone())
        );
        assert_eq!(
            journey::living_scene::observation_journal(completed.state()).unwrap(),
            vec![record]
        );
        let after = runtime::scene(completed.state()).unwrap();
        assert_eq!(
            journey::living_scene::observation_source_hash(&after).unwrap(),
            source_hash
        );
        assert_eq!(
            journey::living_scene::established_detail_for(
                &completed.state().clone(),
                HOME,
                &detail.target
            )
            .unwrap(),
            Some(detail)
        );
    });
}

#[test]
fn picture_features_are_place_bound_deduplicated_and_survive_later_plates_and_returns() {
    with_shop(|opening, _| {
        let made = street(&model_of(&opening));
        let street = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let record = fruit_picture("approved-plate-one");
        let seen = journey::living_scene::commit_picture_features(
            &street,
            street.basis(),
            operation(41),
            record.clone(),
        )
        .unwrap();
        let duplicate = journey::living_scene::commit_picture_features(
            &seen,
            seen.basis(),
            operation(42),
            record,
        )
        .unwrap();
        assert_eq!(duplicate, seen);
        let later = journey::living_scene::commit_picture_features(
            &seen,
            seen.basis(),
            operation(43),
            fruit_picture("approved-plate-two"),
        )
        .unwrap();
        let all = journey::living_scene::picture_features(later.state(), STREET).unwrap();
        assert_eq!(
            all.iter()
                .flat_map(|record| &record.seen_in_picture)
                .count(),
            1
        );
        assert_eq!(all[0].plate_key, "approved-plate-one");
        let back = commit(&later, 44, place::WAY_BACK, HOME, None).unwrap();
        assert!(
            journey::living_scene::picture_features(back.state(), HOME)
                .unwrap()
                .is_empty()
        );
        let returned = commit(&back, 45, "street-door", STREET, None).unwrap();
        assert_eq!(
            journey::living_scene::picture_features(returned.state(), STREET).unwrap(),
            all
        );
        assert!(
            journey::living_scene::commit_picture_features(
                &returned,
                street.basis(),
                operation(46),
                fruit_picture("stale")
            )
            .is_err()
        );
        let mut invalid = fruit_picture("bad-region");
        invalid.seen_in_picture[0].region = [900, 200, 600, 500];
        assert!(
            journey::living_scene::commit_picture_features(
                &returned,
                returned.basis(),
                operation(47),
                invalid
            )
            .is_err()
        );
    });
}

#[test]
fn seen_picture_reference_moves_to_a_generated_spot_and_return_reuses_it() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let street = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let seen = journey::living_scene::commit_picture_features(
            &street,
            street.basis(),
            operation(41),
            fruit_picture("approved-street-plate"),
        )
        .unwrap();
        let mut spot = named_place("fruit stand");
        let mut place: Value = serde_json::from_str(spot["place"].as_str().unwrap()).unwrap();
        place["speaker"] = json!("");
        place["greeting"] = json!("");
        place["narration"] = json!(
            "You gather at the fruit stand. Mira offers crisp apples in shallow baskets; Market Street stays in view behind you."
        );
        place["wares"] = json!([{"seller":"Mira Fen", "name":"apple", "description":"A crisp red apple from the basket.", "price_cp":2}]);
        place["traders"] = json!(["Mira Fen"]);
        place["exits"] = json!([]);
        place["landmarks"] = json!([]);
        place["market"] = json!(false);
        place["stalls"] = json!([]);
        place["mentioned_stalls"] = json!([]);
        spot["place"] = json!(place.to_string());
        let (choices, story, voice) = travel_voice(
            vec![choice("o4"), choice("o1")],
            vec![Reply::Output(spot)],
            "HS40-picture-spot",
        );
        let table = Table::start(seen, loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let response = run(table.service.local_reference_call([1;32], &line(&table, 42,
            "I wander over to the fruit stand in the background and take a good look. What's on offer?"), &scene, false)).unwrap().unwrap();
        assert!(response.npc_reply.contains("apples"));
        let at_spot = table.current();
        let spot_id = runtime::scene(at_spot.state())
            .unwrap()
            .setting
            .unwrap()
            .place_id;
        assert_ne!(spot_id, STREET);
        assert_eq!(model_of(&at_spot).ways_out[0].beyond_place_id, STREET);
        assert_eq!(model_of(&at_spot).offers[0].cost_cp, 2);
        assert_eq!(place::created(at_spot.state()).unwrap().len(), 2);
        run(table
            .service
            .move_party_through([1; 32], operation(43), place::WAY_BACK))
        .unwrap_or_else(|_| panic!("return"));
        let returned = table.current();
        let scene = runtime::scene(returned.state()).unwrap();
        run(table.service.local_reference_call(
            [1; 32],
            &line(&table, 44, "I go back to that apple stall."),
            &scene,
            false,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            runtime::scene(table.current().state())
                .unwrap()
                .setting
                .unwrap()
                .place_id,
            spot_id
        );
        assert_eq!(place::created(table.current().state()).unwrap().len(), 2);
        assert_eq!(story.primary_connections(), 1);
        assert_eq!(choices.primary_connections(), 2);
        let request = story.primary_bodies()[0]["input"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(request.contains("local_spot") && request.contains("arrival_question"));
        assert!(request.contains("red apples") && request.contains("Market Street"));
    });
}

#[test]
fn an_impossible_local_reference_is_refused_specifically_in_fiction_without_a_move() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let street = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let text = "There is no royal vault on this open street; Wenna's spice stall stands at the corner.";
        let (_choices, story, voice) = travel_voice(
            vec![choice("o5")],
            vec![Reply::Output(json!({"refusal":text}))],
            "HS40-impossible-spot",
        );
        let table = Table::start(street.clone(), loaded, voice);
        let scene = runtime::scene(street.state()).unwrap();
        let response = run(table.service.local_reference_call(
            [1; 32],
            &line(&table, 41, "I examine the royal vault beside us."),
            &scene,
            false,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(response.npc_reply, text);
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Rejected as i32
        );
        assert_eq!(table.current(), street);
        assert_eq!(story.primary_connections(), 1);
    });
}

/// What the owner does with a proposal: checks it, builds the candidate and commits it.
fn commit(
    current: &Checkpoint,
    number: u8,
    exit: &str,
    to: &str,
    created: Option<&PlaceRecord>,
) -> Result<Checkpoint, PlaceRefusal> {
    let scene = runtime::scene(current.state()).unwrap();
    let from = scene.setting.as_ref().unwrap().place_id.clone();
    match place::plan(
        current,
        "Brynn",
        operation(number),
        &from,
        exit,
        to,
        created,
    ) {
        Ok(planned) => Ok(planned.candidate),
        Err(PlaceFailure::Refused(reason)) => Err(reason),
        Err(PlaceFailure::State(error)) => panic!("state failure {error:?}"),
    }
}

fn model_of(current: &Checkpoint) -> SceneModel {
    let scene = runtime::scene(current.state()).unwrap();
    journey::living_scene::snapshot(current.state(), &scene)
        .unwrap()
        .unwrap()
        .model
}

fn display_scene(current: &Checkpoint) -> rpc::GameplayScene {
    let view = wire::journey_view(current, LocalDemoRole::Display, native::first()).unwrap();
    match view.audience.unwrap() {
        rpc::view_message::Audience::Display(display) => display.scene.unwrap(),
        _ => panic!("display"),
    }
}

fn player_scene(current: &Checkpoint, member: df_types::MemberId) -> rpc::GameplayScene {
    let view = wire::journey_view(current, LocalDemoRole::Player, member).unwrap();
    match view.audience.unwrap() {
        rpc::view_message::Audience::Player(player) => player.scene.unwrap(),
        _ => panic!("player"),
    }
}

fn names(scene: &rpc::GameplayScene) -> Vec<String> {
    scene.cast.iter().map(|actor| actor.name.clone()).collect()
}

#[test]
fn the_shop_lists_its_street_door_and_it_leads_somewhere_the_run_does_not_have_yet() {
    with_shop(|opening, _| {
        let model = model_of(&opening);
        let door = model
            .ways_out
            .iter()
            .find(|way| way.id == "street-door")
            .expect("the shop's door to the market street");
        assert!(door.open);
        let scene = runtime::scene(opening.state()).unwrap();
        let party = place::whereabouts(opening.state(), &scene).unwrap();
        assert_eq!((party.here.as_str(), party.home.as_str()), (HOME, HOME));
        assert_eq!(
            place::destination(opening.state(), &party.home, &party.here, door).unwrap(),
            Destination::Create { named: None }
        );
        let report = report(&opening).unwrap();
        assert_eq!(report["place"]["id"], HOME);
        assert_eq!(report["place"]["home"], true);
        assert_eq!(report["place"]["created"], false);
        assert_eq!(report["exits"][0]["id"], "street-door");
        assert_eq!(report["exits"][0]["leads_to"]["status"], "new");
        assert_eq!(report["known"][0]["id"], HOME);
        assert_eq!(report["moves"].as_array().unwrap().len(), 0);
    });
}

#[test]
fn leaving_through_the_door_moves_the_party_and_the_display_view_names_the_new_scene_and_cast() {
    with_shop(|opening, _| {
        let before = display_scene(&opening);
        assert_eq!(before.title, "Marek’s Cloth Shop");
        assert_eq!(names(&before), ["Marek", "Corin", "Jessa"]);
        let made = street(&model_of(&opening));
        let moved = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        // The scene, its cast and its place are the street's; the shop's are not on screen.
        let scene = display_scene(&moved);
        assert_eq!(scene.title, "Market Street");
        assert_eq!(scene.place, "Market Street");
        assert_eq!(scene.location_id, STREET);
        assert_eq!(scene.scene_asset, "live-room-photo");
        assert_eq!(names(&scene), ["Hobb", "Wenna", "Yara"]);
        assert!(
            scene
                .cast
                .iter()
                .all(|actor| actor.sprite == "live-canonical-photo")
        );
        // The picture is the place's own: the room photo is made from the street's scene model.
        let contents: Value = serde_json::from_str(&scene.scene_contents_json).unwrap();
        let things: Vec<&str> = contents["things"]
            .as_array()
            .unwrap()
            .iter()
            .map(|thing| thing["name"].as_str().unwrap())
            .collect();
        assert!(things.contains(&"spice stall") && !things.contains(&"wooden counter"));
        assert_eq!(
            runtime::scene(moved.state()).unwrap().narration,
            made.narration
        );
        // The party moves together: the other hero's own view shows the street too.
        let other = player_scene(&moved, native::second());
        assert_eq!(
            (other.title.as_str(), other.location_id.as_str()),
            ("Market Street", STREET)
        );
        // Its scene model is the street's, with the party in it and a way back to the shop.
        let model = model_of(&moved);
        assert!(model.people.iter().any(|person| person.name == "Brynn"));
        assert!(model.people.iter().any(|person| person.name == "Vale"));
        assert_eq!(model.offers.len(), 6);
        assert_eq!(model.offers[0].seller_id, "market-street-wenna");
        assert_eq!(model.offers[0].cost_cp, 10);
        assert_eq!(model.services.len(), 1);
        assert_eq!(model.services[0].provider_id, "market-street-hobb");
        assert_eq!(model.services[0].price_cp, 10);
        assert_eq!(model.ways_out[0].id, "way-back");
        assert_eq!(model.ways_out[0].beyond_place_id, HOME);
        assert_eq!(report(&moved).unwrap()["place"]["created"], true);
    });
}

#[test]
fn a_created_place_is_reused_on_return_and_the_shop_stays_in_the_world() {
    with_shop(|opening, _| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        // Back through the street's way back: the shop, exactly as it was left.
        let back = commit(&out, 41, "way-back", HOME, None).unwrap();
        let scene = display_scene(&back);
        assert_eq!(scene.title, "Marek’s Cloth Shop");
        assert_eq!(names(&scene), ["Marek", "Corin", "Jessa"]);
        assert_eq!(scene.location_id, HOME);
        assert_eq!(
            model_of(&back).things.len(),
            model_of(&opening).things.len()
        );
        // The door now leads to the street the run has: nothing is created again, and asking for
        // it to be created again is refused.
        let state = back.state();
        let door = model_of(&back)
            .ways_out
            .into_iter()
            .find(|way| way.id == "street-door")
            .unwrap();
        let shop_scene = runtime::scene(state).unwrap();
        let party = place::whereabouts(state, &shop_scene).unwrap();
        match place::destination(state, &party.home, &party.here, &door).unwrap() {
            Destination::Created(known) => assert_eq!(*known, made),
            other => panic!("the street is already made, not {other:?}"),
        }
        assert_eq!(
            commit(&back, 42, "street-door", STREET, Some(&made)).err(),
            Some(PlaceRefusal::InvalidPlace)
        );
        let again = commit(&back, 43, "street-door", STREET, None).unwrap();
        assert_eq!(display_scene(&again).title, "Market Street");
        assert_eq!(names(&display_scene(&again)), ["Hobb", "Wenna", "Yara"]);
        assert_eq!(model_of(&again).offers, model_of(&out).offers);
        assert_eq!(model_of(&again).services, model_of(&out).services);
        assert_eq!(place::created(again.state()).unwrap().len(), 1);
        let moves = place::entries(again.state()).unwrap();
        assert_eq!(moves.len(), 3);
        assert_eq!(
            moves
                .iter()
                .map(|(_, entry)| entry.change.created.is_some())
                .collect::<Vec<_>>(),
            [true, false, false]
        );
        let listed = report(&again).unwrap();
        assert_eq!(listed["place"]["id"], STREET);
        assert_eq!(listed["known"].as_array().unwrap().len(), 2);
        assert_eq!(listed["exits"][0]["leads_to"]["status"], "home");
    });
}

#[test]
fn the_screens_find_a_created_place_where_the_owner_publishes_it() {
    with_shop(|opening, _| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let shown = display_scene(&out);
        let key = pack::SceneKey::Opening;
        // The street is not in the pack: until the owner publishes the scene nothing finds it.
        assert!(place::registered(&shown.title, &shown.scene_asset, key).is_none());
        journey::living_scene::publish(&out).unwrap();
        let found =
            place::registered(&shown.title, &shown.scene_asset, key).expect("the published street");
        let setting = found.setting.as_ref().unwrap();
        assert_eq!(setting.place_id, STREET);
        assert_eq!(setting.kind, SettingKind::MarketStreet);
        assert!(!setting.exterior || setting.kind.is_exterior());
        assert_eq!(
            found
                .cast
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>(),
            ["Hobb", "Wenna", "Yara"]
        );
        // What the place music reads about the place comes from the same publication.
        let published = place::registered_setting(STREET).unwrap();
        assert_eq!(published.short, made.short);
        assert_eq!(published.name, made.name);
    });
}

#[test]
fn a_move_is_a_journal_entry_that_survives_persistence_and_replays_the_same_scene() {
    with_shop(|opening, _| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let decision = out.state().decisions.last().unwrap();
        let text = decision.semantic_output.as_deref().unwrap();
        // The checkpoint codec keeps every stored text to 4,096 bytes.
        assert!(text.len() <= 4_096, "{} bytes", text.len());
        assert_eq!(decision.source_policy.as_str(), place::POLICY);
        assert!(decision.draws.is_empty());
        let recovered = native::persisted(&out);
        assert_eq!(
            runtime::scene(recovered.state()).unwrap(),
            runtime::scene(out.state()).unwrap()
        );
        // Going back and forth keeps replaying: the whole journal validates after every move.
        let back = native::persisted(&commit(&recovered, 41, "way-back", HOME, None).unwrap());
        let again = native::persisted(&commit(&back, 42, "street-door", STREET, None).unwrap());
        assert_eq!(display_scene(&again).title, "Market Street");
    });
}

#[test]
fn the_owner_refuses_every_move_it_cannot_check_and_nothing_changes() {
    with_shop(|opening, _| {
        let made = street(&model_of(&opening));
        let refuse = |current: &Checkpoint, exit: &str, to: &str, created: Option<&PlaceRecord>| {
            commit(current, 50, exit, to, created).err()
        };
        assert_eq!(
            refuse(&opening, "no-such-exit", STREET, Some(&made)),
            Some(PlaceRefusal::NoExit)
        );
        // A place that is not the one the exit leads to, or a new place with no record.
        assert_eq!(
            refuse(&opening, "street-door", HOME, None),
            Some(PlaceRefusal::InvalidPlace)
        );
        assert_eq!(
            refuse(&opening, "street-door", STREET, None),
            Some(PlaceRefusal::InvalidPlace)
        );
        let mut renamed = made.clone();
        renamed.place_id = "place-somewhere-else".to_owned();
        assert_eq!(
            refuse(&opening, "street-door", STREET, Some(&renamed)),
            Some(PlaceRefusal::AlreadyKnown)
        );
        // A place that breaks a rule of a place is never committed.
        let mut broken = made.clone();
        broken.speaker = "Nobody".to_owned();
        assert_eq!(
            refuse(&opening, "street-door", STREET, Some(&broken)),
            Some(PlaceRefusal::InvalidPlace)
        );
        // The shop's own place cannot be created, and a development move needs a place the run has.
        let mut shop_again = made.clone();
        shop_again.place_id = HOME.to_owned();
        assert_eq!(
            refuse(&opening, "street-door", HOME, Some(&shop_again)),
            Some(PlaceRefusal::AlreadyKnown)
        );
        assert_eq!(
            refuse(&opening, place::DEV_EXIT, STREET, None),
            Some(PlaceRefusal::NoExit)
        );
        // The party must stand where the move leaves from.
        let scene = runtime::scene(opening.state()).unwrap();
        assert_eq!(scene.setting.as_ref().unwrap().place_id, HOME);
        match place::plan(
            &opening,
            "Brynn",
            operation(51),
            STREET,
            "way-back",
            HOME,
            None,
        ) {
            Err(PlaceFailure::Refused(PlaceRefusal::NotHere)) => {}
            _ => panic!("a move from a place the party is not in"),
        }
        // A shut exit does not open by being walked at, and one operation changes the place once.
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        match place::plan(&out, "Brynn", operation(40), STREET, "way-back", HOME, None) {
            Err(PlaceFailure::Refused(PlaceRefusal::Duplicate)) => {}
            _ => panic!("the same operation twice"),
        }
        // Nothing was committed by any refusal.
        assert!(place::entries(opening.state()).unwrap().is_empty());
        assert_eq!(display_scene(&opening).title, "Marek’s Cloth Shop");
    });
}

#[test]
fn the_run_stops_making_places_at_its_bound_and_a_story_place_is_not_walked_to() {
    with_shop(|opening, _| {
        // Walk out and back with distinct made places until the run is full: each exit of a made
        // place that names no place makes a new one.
        let mut current = opening.clone();
        let mut made_ids = vec![];
        for index in 0..place::MAXIMUM_CREATED_PLACES {
            let door = if index == 0 {
                "street-door"
            } else {
                "north-alley"
            };
            let model = model_of(&current);
            let mut record = street(&model);
            if index > 0 {
                record.place_id = format!("place-street-{index}");
                record.origin.id = made_ids.last().cloned().unwrap();
                record.name = format!("Street {index}");
            }
            let to = record.place_id.clone();
            current = commit(&current, 60 + index as u8, door, &to, Some(&record)).unwrap();
            made_ids.push(to);
        }
        assert_eq!(
            place::created(current.state()).unwrap().len(),
            place::MAXIMUM_CREATED_PLACES
        );
        let mut extra = street(&model_of(&current));
        extra.place_id = "place-one-too-many".to_owned();
        extra.name = "One Too Many".to_owned();
        extra.origin.id = made_ids.last().cloned().unwrap();
        assert_eq!(
            commit(
                &current,
                99,
                "north-alley",
                "place-one-too-many",
                Some(&extra)
            )
            .err(),
            Some(PlaceRefusal::RunFull)
        );
    });
}

#[test]
fn development_targets_read_an_exit_by_id_name_or_alias_then_a_place_the_run_has() {
    with_shop(|opening, _| {
        for wanted in ["street-door", "Street Door", "front door", "door"] {
            assert_eq!(
                resolve_target(&opening, wanted),
                Ok(DevTarget::Exit("street-door".to_owned())),
                "{wanted}"
            );
        }
        assert_eq!(
            resolve_target(&opening, "Marek’s Cloth Shop"),
            Ok(DevTarget::Known(HOME.to_owned()))
        );
        let failure = resolve_target(&opening, "the moon").unwrap_err();
        assert!(
            failure.contains("street-door") && failure.contains(HOME),
            "{failure}"
        );
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        assert_eq!(
            resolve_target(&out, "market street"),
            Ok(DevTarget::Known(STREET.to_owned()))
        );
        assert_eq!(
            resolve_target(&out, "way back to Marek’s Cloth Shop"),
            Ok(DevTarget::Exit("way-back".to_owned()))
        );
        // A development move goes straight to a place the run has, and never makes one.
        let direct = commit(&out, 41, place::DEV_EXIT, HOME, None).unwrap();
        assert_eq!(display_scene(&direct).title, "Marek’s Cloth Shop");
        let again = commit(&direct, 42, place::DEV_EXIT, STREET, None).unwrap();
        assert_eq!(display_scene(&again).title, "Market Street");
    });
}

// ---- The service route: Jev chooses the exit, the storyteller makes the place, the owner commits.

/// A stand-in for the session owner that runs the owner's own checks (`place::plan`) over an
/// in-memory checkpoint: the same code the real owner runs before it commits.
struct Owner {
    checkpoint: Checkpoint,
    updates: watch::Sender<Checkpoint>,
    // When set, the next committed move is made but its reply never reaches the caller.
    lose_reply: Arc<std::sync::atomic::AtomicBool>,
    // When set, the next move is not made: the revision advances for another reason and the reply is lost.
    advance_only: Arc<std::sync::atomic::AtomicBool>,
}

impl Reducer<actor::Call> for Owner {
    fn reduce(&mut self, _sequence: AdmissionSequence, input: actor::Call) {
        let (proposal, reply) = match input {
            actor::Call::PlaceMove {
                proposal, reply, ..
            } => (proposal, reply),
            // Every seat in these tests is the first hero's.
            actor::Call::SpeechMember { reply, .. } => {
                let _ = reply.send(Ok(native::first()));
                return;
            }
            _ => return,
        };
        if self
            .advance_only
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            // Another action advances the revision; this move is not made and its reply is lost.
            let mut basis = self.checkpoint.basis();
            basis.revision = basis.revision.next_sequence().unwrap();
            let advanced =
                crate::gameplay::model::checkpoint(basis, self.checkpoint.state().clone()).unwrap();
            self.checkpoint = advanced.clone();
            self.updates.send_replace(advanced);
            return;
        }
        let state = self.checkpoint.state();
        let hero = journey::name(state, state.characters[0].entity).unwrap();
        let outcome = match place::plan(
            &self.checkpoint,
            &hero,
            proposal.operation,
            &proposal.from,
            &proposal.exit,
            &proposal.to,
            proposal.created.as_ref(),
        ) {
            Ok(planned) => {
                let created = proposal.created.is_some();
                place::audit_changed(&proposal.from, &proposal.to, &proposal.exit, created);
                self.checkpoint = planned.candidate.clone();
                self.updates.send_replace(planned.candidate.clone());
                if self
                    .lose_reply
                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                {
                    return;
                }
                Ok(MoveOutcome::Moved(Box::new(Moved {
                    revision: planned.candidate.basis().revision,
                    from: proposal.from,
                    to: proposal.to,
                    place_name: planned.to_name,
                    exit: proposal.exit,
                    created,
                    line: planned.entry.line,
                    narration: planned.arrival,
                })))
            }
            Err(PlaceFailure::Refused(reason)) => {
                place::audit_refused(&proposal.from, &proposal.to, &proposal.exit, reason.label());
                Ok(MoveOutcome::Refused(reason))
            }
            Err(PlaceFailure::State(error)) => Err(actor::unavailable(error)),
        };
        let _ = reply.send(outcome);
    }
}

struct Table {
    lose_reply: Arc<std::sync::atomic::AtomicBool>,
    advance_only: Arc<std::sync::atomic::AtomicBool>,
    service: Service,
    table: Arc<TableLog>,
    updates: watch::Receiver<Checkpoint>,
    owner: Option<std::thread::JoinHandle<()>>,
    sender: df_session::inbox::InboxHandle<actor::Call>,
}

impl Table {
    fn start(current: Checkpoint, loaded: pack::LoadedPack, npc_voice: Arc<NpcVoice>) -> Self {
        let (updates_in, updates) = watch::channel(current.clone());
        let (sender, inbox): (_, ActorLoop<actor::Call>) = bounded_inbox();
        let table = TableLog::new();
        let service = Service {
            portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
            display_credential: [0x82; 32],
            layout: crate::gameplay::dev_control::layout::LayoutState::new(),
            actor: sender.clone(),
            updates: updates.clone(),
            streams: Arc::new(Semaphore::new(2)),
            voice: Voice::disabled(VoiceOff::Qualification),
            npc_voice,
            table_log: table.clone(),
            social: SocialLedger::none(),
            living: LivingScenes::disabled(),
            look_ahead: Arc::default(),
        };
        let lose_reply = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let owner_lose_reply = lose_reply.clone();
        let advance_only = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let owner_advance_only = advance_only.clone();
        let owner = std::thread::spawn(move || {
            pack::with_test_pack(loaded, || {
                let mut owner = Owner {
                    checkpoint: current,
                    updates: updates_in,
                    lose_reply: owner_lose_reply,
                    advance_only: owner_advance_only,
                };
                let _ = inbox.run(&mut owner);
            });
        });
        Self {
            service,
            table,
            updates,
            owner: Some(owner),
            sender,
            lose_reply,
            advance_only,
        }
    }

    fn current(&self) -> Checkpoint {
        self.updates.borrow().clone()
    }

    fn lines(&self) -> Vec<(String, String)> {
        self.table
            .lines()
            .into_iter()
            .map(|line| (line.speaker, line.text))
            .collect()
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        let _ = self.sender.stop();
        if let Some(owner) = self.owner.take() {
            let _ = owner.join();
        }
    }
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

fn voice_of(jev_endpoint: &FakeEndpoint, story: Storyteller) -> Arc<NpcVoice> {
    Arc::new(NpcVoice::new(Arc::new(story)).with_approach_model(jev(jev_endpoint)))
}

fn performed_move(succeeded: bool) -> Resolution {
    Resolution {
        hero: "Brynn".to_owned(),
        act: Some(ActKind::Move),
        sense: None,
        means: StrikeMeans::Unarmed,
        target: None,
        spell: None,
        recipient: None,
        notices: Vec::new(),
        verdict: Verdict::Performed(Performance {
            check: None,
            succeeded,
            revealed: None,
        }),
        consequences: Vec::new(),
    }
}

fn line(table: &Table, number: u8, text: &str) -> rpc::RawDialogueRequest {
    let basis = table.current().basis();
    rpc::RawDialogueRequest {
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        observed_revision: Some(wire::revision(basis.revision)),
        input_id: Some(rpc::OperationId {
            value: Some(vec![number; 16]),
        }),
        text: text.to_owned(),
        context: rpc::DialogueContext::Unspecified as i32,
        finality: rpc::DialogueFinality::Final as i32,
        offer_id: String::new(),
    }
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn replied(reply: Option<DialogueReply>) -> String {
    match reply {
        Some(DialogueReply::Raw(response, _)) => response.npc_reply,
        _ => panic!("a raw reply"),
    }
}

#[test]
fn a_leaving_line_goes_through_jev_and_the_storyteller_to_a_committed_move_the_table_is_told() {
    with_shop(|opening, loaded| {
        let exits = FakeEndpoint::start(vec![choice("o1")]);
        let storyteller = FakeEndpoint::start(vec![Reply::Output(answer(
            &place_slot(),
            &people_slot(),
            &things_slot(),
        ))]);
        let cache = cache_directory("place-service-leave");
        // What the town's people have said: the canon Marek established, and a line at the table.
        let lore = Arc::new(local_lore::LocalLore::in_memory());
        lore.propose(
            "Opening",
            "custom|northbound wagons|Merchants heading north gather at the market before dawn.",
            "Marek",
            &local_lore::Names {
                heroes: &[],
                game_state: &[],
            },
        )
        .unwrap()
        .unwrap();
        let voice = Arc::new(
            NpcVoice::new(Arc::new(teller(
                storyteller.address,
                0,
                Duration::from_secs(5),
                &cache,
            )))
            .with_approach_model(jev(&exits))
            .with_local_lore(lore),
        );
        let table = Table::start(opening.clone(), loaded, voice);
        table.table.push_actor(
            "merchant",
            "Marek",
            rpc::TableRole::Npc,
            0,
            rpc::TableKind::Speech,
            "Wagons for the north road load at the market, friend.",
        );
        let scene = runtime::scene(opening.state()).unwrap();
        let said = "I follow Brakka out of the shop into the market street.";
        let reply = run(table.service.leave_by_freeform(
            [1; 32],
            &line(&table, 70, said),
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        ));
        let told = replied(reply);
        assert_eq!(told, "Brynn leads the party to Market Street.");
        // The state changed, and only then did the table hear of it.
        let now = table.current();
        assert_eq!(runtime::scene(now.state()).unwrap().title, "Market Street");
        assert_eq!(
            table.lines().last(),
            Some(&("Narrator".to_owned(), told.clone()))
        );
        assert_eq!(
            table.lines().len(),
            2,
            "Marek's line, then the committed move"
        );
        assert_eq!(place::created(now.state()).unwrap().len(), 1);
        // Jev was asked once, with the line as quoted data; the storyteller once, with the
        // town's canon and the table's own words in the facts it was given.
        assert_eq!(exits.primary_connections(), 1);
        assert!(
            exits.primary_bodies()[0]
                .to_string()
                .contains("I follow Brakka out of the shop")
        );
        assert_eq!(storyteller.primary_connections(), 1);
        let asked = storyteller.primary_bodies()[0]["input"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(asked.contains("Merchants heading north gather at the market before dawn"));
        assert!(asked.contains("Wagons for the north road load at the market"));
        assert!(asked.contains("street door") && asked.contains("Brynn"));
        assert!(asked.contains("Corin") && asked.contains("Jessa"));
        // One audit line per change, and the judgement before it.
        let changes = actor::audit_lines_containing(
            "PLACE change from=place-mareks-cloth-shop to=place-market-street",
        );
        assert!(changes.iter().any(|line| {
            // The shared audit path appends correlation fields after the original event.
            line.split_once(" op=")
                .is_some_and(|(event, _)| event.ends_with("exit=street-door created=true"))
        }));
        assert!(
            !actor::audit_lines_containing(
                "PLACE judged from=place-mareks-cloth-shop source=jev exit=street-door"
            )
            .is_empty()
        );
    });
}

#[test]
fn the_party_comes_back_through_the_way_back_and_a_second_trip_makes_nothing_new() {
    with_shop(|opening, loaded| {
        // Out (exit o1 of one; the place is made), back (way-back is o1 of two), out again (o1).
        let exits = FakeEndpoint::start(vec![choice("o1"), choice("o1"), choice("o1")]);
        let storyteller = FakeEndpoint::start(vec![Reply::Output(answer(
            &place_slot(),
            &people_slot(),
            &things_slot(),
        ))]);
        let cache = cache_directory("place-service-return");
        let voice = voice_of(
            &exits,
            teller(storyteller.address, 0, Duration::from_secs(5), &cache),
        );
        let table = Table::start(opening.clone(), loaded, voice);
        for (number, said, told) in [
            (
                71,
                "I walk out of the shop into the street.",
                "Brynn leads the party to Market Street.",
            ),
            // The way back names the place once (P6-170).
            (
                72,
                "We go back inside the shop.",
                "Brynn leads the party back to Marek\u{2019}s Cloth Shop.",
            ),
            (
                73,
                "I step out into the street again.",
                "Brynn leads the party to Market Street.",
            ),
        ] {
            let scene = runtime::scene(table.current().state()).unwrap();
            let reply = run(table.service.leave_by_freeform(
                [1; 32],
                &line(&table, number, said),
                &scene,
                "Brynn",
                0,
                &performed_move(true),
            ));
            assert_eq!(replied(reply), told);
        }
        let now = table.current();
        assert_eq!(runtime::scene(now.state()).unwrap().title, "Market Street");
        assert_eq!(place::entries(now.state()).unwrap().len(), 3);
        assert_eq!(place::created(now.state()).unwrap().len(), 1);
        assert_eq!(
            storyteller.primary_connections(),
            1,
            "the street was made once"
        );
        assert_eq!(table.lines().len(), 3);
    });
}

#[test]
fn narration_never_claims_a_change_that_did_not_happen() {
    with_shop(|opening, loaded| {
        let scene = runtime::scene(opening.state()).unwrap();
        // Jev finds no exit in the line: the party stays, the table is told nothing, no place is made.
        // The shop's choices are its street door, a journey elsewhere, and staying (o3).
        let exits = FakeEndpoint::start(vec![choice("o3"), choice("o3")]);
        let storyteller = FakeEndpoint::start(vec![]);
        let cache = cache_directory("place-service-stays");
        let voice = voice_of(
            &exits,
            teller(storyteller.address, 0, Duration::from_secs(5), &cache),
        );
        let table = Table::start(opening.clone(), loaded.clone(), voice);
        let reply = run(table.service.leave_by_freeform(
            [1; 32],
            &line(&table, 80, "I walk up to the counter."),
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        ));
        assert!(reply.is_none());
        assert!(table.lines().is_empty());
        assert_eq!(table.current(), opening);
        assert_eq!(storyteller.primary_connections(), 0);

        // A move that failed its own check goes nowhere, and Jev is not even asked.
        let unasked = FakeEndpoint::start(vec![choice("o1")]);
        let voice = voice_of(
            &unasked,
            teller(
                storyteller.address,
                0,
                Duration::from_secs(5),
                &cache_directory("place-service-failed-move"),
            ),
        );
        let table = Table::start(opening.clone(), loaded.clone(), voice);
        assert!(
            run(table.service.leave_by_freeform(
                [1; 32],
                &line(&table, 81, "I leap through the door."),
                &scene,
                "Brynn",
                0,
                &performed_move(false),
            ))
            .is_none()
        );
        assert_eq!(unasked.primary_connections(), 0);
        assert!(table.lines().is_empty());

        // Jev names the door but the storyteller cannot make the place beyond it: the party stays
        // where it is, nothing is committed, and an explicit refusal is told and audited.
        let exits = FakeEndpoint::start(vec![choice("o1")]);
        let mut wrong = place_slot();
        wrong["kind"] = json!("dragon-lair");
        let storyteller = FakeEndpoint::start(vec![
            Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
            Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
            Reply::Output(answer(&wrong, &people_slot(), &things_slot())),
        ]);
        let voice = voice_of(
            &exits,
            teller(
                storyteller.address,
                0,
                Duration::from_secs(5),
                &cache_directory("place-service-cannot-make"),
            ),
        );
        let table = Table::start(opening.clone(), loaded.clone(), voice);
        let Some(DialogueReply::Raw(response, _)) = run(table.service.leave_by_freeform(
            [1; 32],
            &line(&table, 82, "I go out through the front door."),
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        )) else {
            panic!("failed generation returns an explicit refusal");
        };
        let refusal = actor::place_actor::travel_refusal_line(PlaceRefusal::CreationUnavailable);
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Rejected as i32
        );
        assert_eq!(response.npc_reply, refusal);
        assert_eq!(table.current(), opening);
        assert!(place::created(table.current().state()).unwrap().is_empty());
        let rows = table.table.lines();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, rpc::TableRole::Narrator as i32);
        assert_eq!(rows[0].kind, rpc::TableKind::Action as i32);
        assert_eq!(rows[0].text, refusal);
        assert!(
            !actor::audit_lines_containing(
                "PLACE refused from=place-mareks-cloth-shop to=none exit=street-door reason=kind"
            )
            .is_empty()
        );

        // Without Jev nothing is guessed from the words.
        let table = Table::start(opening.clone(), loaded, Arc::new(NpcVoice::silent()));
        let Some(DialogueReply::Raw(response, _)) = run(table.service.leave_by_freeform(
            [1; 32],
            &line(&table, 83, "I leave."),
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        )) else {
            panic!("unavailable local navigation is answered in fiction");
        };
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Rejected as i32
        );
        assert_eq!(
            response.npc_reply,
            "The party stays together; the way to that spot isn't clear yet."
        );
        assert_eq!(table.current(), opening);
    });
}

#[test]
fn only_a_succeeded_move_act_is_put_to_the_exits() {
    with_shop(|opening, loaded| {
        let scene = runtime::scene(opening.state()).unwrap();
        let exits = FakeEndpoint::start(vec![choice("o1")]);
        let table = Table::start(
            opening.clone(),
            loaded,
            voice_of(&exits, Storyteller::disabled()),
        );
        let mut sense = performed_move(true);
        sense.act = Some(ActKind::Sense);
        assert!(
            run(table.service.leave_by_freeform(
                [1; 32],
                &line(&table, 90, "I look at the door."),
                &scene,
                "Brynn",
                0,
                &sense,
            ))
            .is_none()
        );
        let mut refused = performed_move(true);
        refused.verdict = Verdict::Unjudged;
        assert!(
            run(table.service.leave_by_freeform(
                [1; 32],
                &line(&table, 91, "I leave."),
                &scene,
                "Brynn",
                0,
                &refused,
            ))
            .is_none()
        );
        assert_eq!(exits.primary_connections(), 0);
        assert_eq!(table.current(), opening);
    });
}

#[test]
fn the_dev_route_moves_to_an_exit_and_to_a_known_place_through_the_same_owner() {
    with_shop(|opening, loaded| {
        let exits = FakeEndpoint::start(vec![]);
        let storyteller = FakeEndpoint::start(vec![Reply::Output(answer(
            &place_slot(),
            &people_slot(),
            &things_slot(),
        ))]);
        let voice = voice_of(
            &exits,
            teller(
                storyteller.address,
                0,
                Duration::from_secs(5),
                &cache_directory("place-service-dev"),
            ),
        );
        let table = Table::start(opening, loaded, voice);
        let moved =
            match run(table
                .service
                .move_party_through([1; 32], operation(100), "street-door"))
            {
                Ok(moved) => moved,
                Err(_) => panic!("the dev move through the door"),
            };
        assert!(moved.created);
        assert_eq!((moved.from.as_str(), moved.to.as_str()), (HOME, STREET));
        assert_eq!(
            runtime::scene(table.current().state()).unwrap().title,
            "Market Street"
        );
        // Straight back to a place the run has, with no exit and nothing made.
        let home = match run(table
            .service
            .move_party_to_known([1; 32], operation(101), HOME))
        {
            Ok(moved) => moved,
            Err(_) => panic!("the dev move home"),
        };
        assert!(!home.created);
        assert_eq!(
            runtime::scene(table.current().state()).unwrap().title,
            "Marek’s Cloth Shop"
        );
        assert_eq!(storyteller.primary_connections(), 1);
        assert_eq!(exits.primary_connections(), 0);
        // A place the run does not have is refused, and the report says where each exit leads.
        assert!(matches!(
            run(table
                .service
                .move_party_to_known([1; 32], operation(102), "place-nowhere")),
            Err(MoveError::Refused(_))
        ));
        let listed = table.service.place_report().unwrap();
        assert_eq!(listed["place"]["id"], HOME);
        assert_eq!(listed["exits"][0]["leads_to"]["status"], "created");
        assert_eq!(listed["moves"].as_array().unwrap().len(), 2);
    });
}

// ---- Destinations, journeys, rides and sightseeing (lane TRAVEL) ----------------------------

/// The fixture answer for a place under another name, with people and landmarks of its own so
/// nothing in it repeats a place or a person the run already has.
fn named_place(name: &str) -> Value {
    let mut place = place_slot();
    place["name"] = json!(name);
    place["speaker"] = json!("Oren Vale");
    for ware in place["wares"].as_array_mut().unwrap() {
        ware["seller"] = json!(match ware["seller"].as_str().unwrap() {
            "Hobb" => "Oren Vale",
            "Wenna" => "Mira Fen",
            _ => "Tess Brook",
        });
    }
    for stall in place["stalls"].as_array_mut().unwrap() {
        stall["keeper"] = json!(match stall["keeper"].as_str().unwrap() {
            "Hobb" => "Oren Vale",
            "Wenna" => "Mira Fen",
            _ => "Tess Brook",
        });
    }
    place["services"][0]["provider"] = json!("Oren Vale");
    place["landmarks"] = json!([
        {"name": "the Clock Arch", "direction": "at the end of the street"},
        {"name": "the Old Quay", "direction": "down the steps"},
    ]);
    let mut people = people_slot();
    people[0]["name"] = json!("Oren Vale");
    people[1]["name"] = json!("Mira Fen");
    people[2]["name"] = json!("Tess Brook");
    answer(&place, &people, &things_slot())
}

fn travel_voice(
    jev_replies: Vec<Reply>,
    story: Vec<Reply>,
    cache: &str,
) -> (FakeEndpoint, FakeEndpoint, Arc<NpcVoice>) {
    let exits = FakeEndpoint::start(jev_replies);
    let storyteller = FakeEndpoint::start(story);
    let voice = voice_of(
        &exits,
        teller(
            storyteller.address,
            0,
            Duration::from_secs(5),
            &cache_directory(cache),
        ),
    );
    (exits, storyteller, voice)
}

fn rename_slot_person(answer: &mut Value, index: usize, name: &str) {
    let mut people: Value = serde_json::from_str(answer["people"].as_str().unwrap()).unwrap();
    let previous = people[index]["name"].as_str().unwrap().to_owned();
    people[index]["name"] = json!(name);
    let mut place: Value = serde_json::from_str(answer["place"].as_str().unwrap()).unwrap();
    for (collection, field) in [
        ("wares", "seller"),
        ("services", "provider"),
        ("stalls", "keeper"),
    ] {
        for entry in place[collection].as_array_mut().unwrap() {
            if entry[field] == previous {
                entry[field] = json!(name);
            }
        }
    }
    for trader in place["traders"].as_array_mut().unwrap() {
        if *trader == previous {
            *trader = json!(name);
        }
    }
    answer["place"] = json!(place.to_string());
    answer["people"] = json!(people.to_string());
}

#[test]
fn a_stated_destination_no_exit_leads_to_is_made_from_the_heros_words_and_reached_by_a_journey() {
    with_shop(|opening, loaded| {
        // The shop's choices: the street door (o1), a journey elsewhere (o2), staying (o3).
        let (exits, storyteller, voice) = travel_voice(
            vec![choice("o2")],
            vec![Reply::Output(named_place("The Carriage Stand"))],
            "place-service-journey-new",
        );
        let table = Table::start(opening.clone(), loaded, voice);
        let scene = runtime::scene(opening.state()).unwrap();
        let said = "We follow the street on toward the carriage stand where coaches leave for the city centre.";
        let told = replied(run(table.service.leave_by_freeform(
            [1; 32],
            &line(&table, 110, said),
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        )));
        assert_eq!(told, "Brynn leads the party on to The Carriage Stand.");
        let now = table.current();
        assert_eq!(
            runtime::scene(now.state()).unwrap().title,
            "The Carriage Stand"
        );
        // The way back leads to the shop the party left; the real shop is not made again.
        assert_eq!(model_of(&now).ways_out[0].beyond_place_id, HOME);
        assert_eq!(place::created(now.state()).unwrap().len(), 1);
        // The storyteller was given the hero's own destination and the places and people in use.
        assert_eq!(exits.primary_connections(), 1);
        let asked = storyteller.primary_bodies()[0]["input"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(asked.contains("toward the carriage stand where coaches leave"));
        assert!(asked.contains("place_names_in_use") && asked.contains("Cloth Shop"));
        assert!(asked.contains("Marek") && asked.contains("Corin"));
        assert!(
            !actor::audit_lines_containing(
                "PLACE judged from=place-mareks-cloth-shop source=jev exit=journey:new"
            )
            .is_empty()
        );
        assert!(
            actor::audit_lines_containing(
                "PLACE change from=place-mareks-cloth-shop to=place-the-carriage-stand"
            )
            .iter()
            .any(|line| {
                // The shared audit path appends correlation fields after the original event.
                line.split_once(" op=")
                    .is_some_and(|(event, _)| event.ends_with("exit=journey created=true"))
            })
        );
    });
}

#[test]
fn a_journey_reaches_a_place_the_run_has_by_name_and_makes_nothing() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let mut further = street(&model_of(&out));
        further.place_id = "place-street-1".to_owned();
        further.name = "Street 1".to_owned();
        further.origin.id = STREET.to_owned();
        further.origin.name = "Market Street".to_owned();
        let deeper = commit(&out, 41, "north-alley", "place-street-1", Some(&further)).unwrap();
        // Street 1's choices: way back (to the street), north alley, carriage lane; the shop by
        // name (o4; the street is already reached by the way back); two landmarks; elsewhere;
        // staying.
        let (exits, storyteller, voice) =
            travel_voice(vec![choice("o4")], vec![], "place-service-journey-known");
        let table = Table::start(deeper.clone(), loaded, voice);
        let scene = runtime::scene(deeper.state()).unwrap();
        let told = replied(run(table.service.leave_by_freeform(
            [1; 32],
            &line(&table, 111, "We head back to Marek's shop."),
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        )));
        assert_eq!(
            told,
            "Brynn leads the party on to Marek\u{2019}s Cloth Shop."
        );
        assert_eq!(
            runtime::scene(table.current().state()).unwrap().title,
            "Marek\u{2019}s Cloth Shop"
        );
        let criteria = exits.primary_bodies()[0]["questions"]["q1"]["criteria"].to_string();
        assert!(criteria.contains("Marek") && !criteria.contains("journey to Market Street"));
        assert_eq!(storyteller.primary_connections(), 0, "nothing is made");
        assert_eq!(place::created(table.current().state()).unwrap().len(), 2);
        // The journey replays like any move.
        let recovered = native::persisted(&table.current());
        assert_eq!(
            runtime::scene(recovered.state()).unwrap().title,
            "Marek\u{2019}s Cloth Shop"
        );
    });
}

#[test]
fn a_landmark_of_a_place_becomes_a_destination_made_under_its_own_name() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        assert_eq!(
            place::landmarks_here(out.state(), STREET)
                .unwrap()
                .iter()
                .map(|landmark| landmark.name.as_str())
                .collect::<Vec<_>>(),
            ["the Bell Tower", "Saint Ivo's Temple"]
        );
        // The street's choices: three ways out, then its two landmarks (o4, o5).
        let (_jev, storyteller, voice) = travel_voice(
            vec![choice("o4")],
            vec![Reply::Output(named_place("The Bell Tower"))],
            "place-service-journey-landmark",
        );
        let table = Table::start(out.clone(), loaded, voice);
        let scene = runtime::scene(out.state()).unwrap();
        let told = replied(run(table.service.leave_by_freeform(
            [1; 32],
            &line(
                &table,
                112,
                "We climb up to the bell tower Hobb told us about.",
            ),
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        )));
        assert_eq!(told, "Brynn leads the party on to The Bell Tower.");
        let asked = storyteller.primary_bodies()[0]["input"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(asked.contains("direction_from_origin") && asked.contains("the Bell Tower"));
        // Once made, the landmark is a known place, not a landmark to make again.
        let now = table.current();
        assert_eq!(place::landmarks_here(now.state(), STREET).unwrap().len(), 1);
    });
}

#[test]
fn a_hailed_carriage_becomes_a_journey_with_a_travel_beat_told_after_the_move() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        // A ride (o2); no listed service is it (o2 after Hobb's one); a journey elsewhere (o6).
        let (jev, _storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o2"), choice("o6")],
            vec![
                Reply::Output(named_place("The City Centre")),
                Reply::Output(json!({
                    "travel": "Brynn waves down a passing carriage, and it rattles away from Market Street toward the heart of the city."
                })),
            ],
            "place-service-hail",
        );
        let table = Table::start(out.clone(), loaded, voice);
        let scene = runtime::scene(out.state()).unwrap();
        let said = "I wave down a passing carriage and ask the driver to take us into the heart of the city.";
        let request = line(&table, 113, said);
        let ride = run(table.service.hailed_ride([1; 32], &request, &scene)).expect("a ride");
        assert!(ride.fare.is_none());
        // Nothing has moved until the ride is taken.
        assert_eq!(table.current(), out);
        let told = replied(run(table.service.take_ride([1; 32], &request, ride, false)));
        assert_eq!(
            told,
            "Brynn waves down a passing carriage, and it rattles away from Market Street toward the heart of the city. Brynn leads the party on to The City Centre."
        );
        assert_eq!(
            runtime::scene(table.current().state()).unwrap().title,
            "The City Centre"
        );
        let lines = table.lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], ("Brynn".to_owned(), said.to_owned()));
        assert!(
            lines[1]
                .1
                .starts_with("Brynn waves down a passing carriage")
        );
        assert_eq!(lines[2].1, "Brynn leads the party on to The City Centre.");
        assert!(
            jev.primary_bodies()[0]
                .to_string()
                .contains("wave down a passing carriage")
        );
        assert!(
            !actor::audit_lines_containing(
                "PLACE ride from=place-market-street source=jev fare=none"
            )
            .is_empty()
        );
    });
}

#[test]
fn a_ride_through_a_listed_service_names_the_fare_and_moves_under_its_own_operation() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let (_jev, storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o1"), choice("o6")],
            vec![
                Reply::Output(named_place("The City Centre")),
                Reply::Status(500, json!({"error": "down"})),
            ],
            "place-service-hire",
        );
        let table = Table::start(out.clone(), loaded, voice);
        let scene = runtime::scene(out.state()).unwrap();
        let request = line(&table, 114, "Hobb, take us into the city on your wagon.");
        let ride = run(table.service.hailed_ride([1; 32], &request, &scene)).expect("a ride");
        let fare = ride.fare.clone().expect("the listed service is the fare");
        assert_eq!(
            fare.service_id,
            "market-street-hobb-service-wagon-directions"
        );
        assert_eq!((fare.price_cp, fare.provider.as_str()), (10, "Hobb"));
        // The place is made before any fare is paid, so a paid fare never goes nowhere.
        assert_eq!(storyteller.primary_connections(), 1);
        let told = replied(run(table.service.take_ride([1; 32], &request, ride, true)));
        assert!(
            told.starts_with("Hobb drives Brynn and the party away from Market Street"),
            "{told}"
        );
        // The purchase showed the hero's line; the ride adds the beat and the arrival only.
        assert_eq!(table.lines().len(), 2);
        let operation = df_api::operation_id(request.input_id.as_ref()).unwrap();
        assert!(
            table
                .current()
                .state()
                .decisions
                .iter()
                .all(|decision| decision.operation != operation),
            "the move does not take the fare's operation"
        );
        let moves = place::entries(table.current().state()).unwrap();
        assert_eq!(moves.last().unwrap().1.change.exit, place::JOURNEY_EXIT);
    });
}

#[test]
fn buying_a_ride_service_starts_the_journey_and_any_other_service_does_not() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let service = "market-street-hobb-service-wagon-directions";
        // Not a ride: nothing more is asked and nothing moves.
        let (jev, _storyteller, voice) =
            travel_voice(vec![choice("o1")], vec![], "place-service-not-ride");
        let table = Table::start(out.clone(), loaded.clone(), voice);
        let scene = runtime::scene(out.state()).unwrap();
        let request = line(&table, 115, "Hobb, which way is the north road?");
        assert!(
            run(table
                .service
                .prepare_service_ride([1; 32], &request, &scene, service))
            .is_none()
        );
        assert_eq!(jev.primary_connections(), 1);
        assert!(
            jev.primary_bodies()[0]
                .to_string()
                .contains("wagon directions")
        );
        assert_eq!(table.current(), out);
        // A ride: the journey starts at once.
        let (_jev, _storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o6")],
            vec![
                Reply::Output(named_place("The City Centre")),
                Reply::Status(500, json!({"error": "down"})),
            ],
            "place-service-ride",
        );
        let table = Table::start(out.clone(), loaded, voice);
        let request = line(
            &table,
            116,
            "I hire Hobb's wagon to take us to the city centre.",
        );
        // Judged and prepared before the purchase; the purchase itself is the caller's step.
        let ride = run(table
            .service
            .prepare_service_ride([1; 32], &request, &scene, service))
        .expect("the hire is a ride");
        assert_eq!(
            ride.fare.as_ref().map(|fare| fare.service_id.as_str()),
            Some(service)
        );
        let RideOutcome::Travelled {
            line: told,
            revision,
        } = run(table
            .service
            .take_ride_outcome([1; 32], &request, ride, true))
        else {
            panic!("the journey travels");
        };
        assert!(
            told.ends_with("Brynn leads the party on to The City Centre."),
            "{told}"
        );
        assert_eq!(revision, table.current().basis().revision);
        assert_eq!(
            table.lines().last().map(|line| line.1.clone()),
            Some("Brynn leads the party on to The City Centre.".to_owned())
        );
    });
}

fn target_named(scene: &pack::Scene, name: &str) -> investigation::Target {
    investigation::targets(scene, &[])
        .into_iter()
        .find(|target| target.name == name)
        .unwrap_or_else(|| panic!("{name} is a target"))
}

#[test]
fn a_plain_look_at_a_created_place_needs_no_roll_and_tells_its_people_and_goods() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let scene = runtime::scene(out.state()).unwrap();
        // Jev reads the look as plain (o2); the storyteller is off, so the rules narrate.
        let exits = FakeEndpoint::start(vec![choice("o2"), choice("o2"), choice("o1")]);
        let table = Table::start(
            out.clone(),
            loaded,
            voice_of(&exits, Storyteller::disabled()),
        );
        let request = line(
            &table,
            117,
            "I look around the market: the stalls, the people, what's being sold.",
        );
        let room = target_named(&scene, "Market Street");
        assert_eq!(room.id, "room");
        let response = run(table
            .service
            .plain_observation([1; 32], &request, &scene, &room, "Brynn", 0))
        .expect("authenticated observation")
        .expect("a plain look");
        let told = response.npc_reply;
        assert!(
            told.starts_with("Brynn looks around Market Street."),
            "{told}"
        );
        assert!(
            told.contains("Hobb, the carter, is at the cart stall, beside the wagon."),
            "{told}"
        );
        assert!(
            told.contains("Wenna at the spice stall, on the east side sells saffron pinch (1 sp)"),
            "{told}"
        );
        assert!(
            !told.contains("the room") && !told.contains("You "),
            "{told}"
        );
        assert_eq!(
            table.lines().len(),
            2,
            "the hero's line, then the look; no roll"
        );
        // A look at a stallholder shows what they sell.
        let wenna = target_named(&scene, "Wenna");
        let told = run(table
            .service
            .plain_observation([1; 32], &request, &scene, &wenna, "Brynn", 0))
        .expect("authenticated observation")
        .expect("a plain look")
        .npc_reply;
        assert!(told.starts_with("Brynn looks at Wenna."), "{told}");
        assert!(told.contains("Wenna sells saffron pinch (1 sp)"), "{told}");
        // A search for what is hidden keeps its check.
        assert!(
            run(table
                .service
                .plain_observation([1; 32], &request, &scene, &wenna, "Brynn", 0))
            .expect("authenticated observation")
            .is_none()
        );
        assert!(
            !actor::audit_lines_containing(
                "OBSERVATION plain=true target=\"Wenna\" source=jev roll=none"
            )
            .is_empty()
        );
    });
}

#[test]
fn a_check_the_owner_refuses_is_answered_with_what_is_plainly_there() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let scene = runtime::scene(out.state()).unwrap();
        let table = Table::start(out, loaded, Arc::new(NpcVoice::silent()));
        let request = line(&table, 118, "I watch Hobb for a moment.");
        let hobb = target_named(&scene, "Hobb");
        let told = run(table
            .service
            .unrolled_observation(&request, &scene, &hobb, "Brynn", 0))
        .npc_reply;
        assert!(
            told.starts_with(
                "Brynn looks at Hobb. Hobb is the carter here, at the cart stall, beside the wagon."
            ),
            "{told}"
        );
        assert!(
            told.contains("Hobb offers wagon directions (1 sp)"),
            "{told}"
        );
        assert_eq!(table.lines().last().map(|line| line.1.clone()), Some(told));
    });
}

#[test]
fn a_replay_reads_the_place_the_party_stood_in_when_a_decision_was_made() {
    with_shop(|opening, _| {
        let base = runtime::scene(opening.state()).unwrap();
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let state = out.state();
        let party = place::whereabouts(state, &runtime::scene(state).unwrap()).unwrap();
        let moved_at = place::entries(state).unwrap()[0].0;
        let before = opening.basis().revision;
        let after = out.basis().revision.next_sequence().unwrap();
        assert!(before < moved_at && moved_at < after);
        assert_eq!(
            place::scene_at(state, &party.beat, base.clone(), before)
                .unwrap()
                .title,
            base.title
        );
        let then = place::scene_at(state, &party.beat, base, after).unwrap();
        assert_eq!(then.title, "Market Street");
        // A person of the street is a target of the scene the replay reads.
        assert!(
            investigation::targets(&then, &[])
                .iter()
                .any(|target| target.name == "Wenna")
        );
    });
}

#[path = "arena_opening_tests.rs"]
mod arena_opening_tests;

#[test]
fn likely_exit_prepares_one_destination_and_photo_then_opening_reuses_it_without_moving() {
    use crate::gameplay::{fight_foresight, imagery, living_scenes};
    struct LikelyExit;
    impl fight_foresight::Jev for LikelyExit {
        fn forecast<'a>(
            &'a self,
            _: &'a Value,
            _: &'a [String],
        ) -> fight_foresight::ChoiceFuture<'a> {
            Box::pin(async { Ok(2) })
        }
    }
    with_shop(|opening, loaded| {
        run(async move {
            let storyteller = FakeEndpoint::start(vec![Reply::Output(answer(
                &place_slot(),
                &people_slot(),
                &things_slot(),
            ))]);
            let lane = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/tmp/lanes/P1/arena-preparation-test")
                .join(std::process::id().to_string());
            let judge = FakeEndpoint::start(vec![]);
            let voice = voice_of(
                &judge,
                teller(
                    storyteller.address,
                    0,
                    Duration::from_secs(5),
                    &lane.join("text"),
                ),
            );
            let mut table = Table::start(opening.clone(), loaded, voice);
            let fixture = imagery::tests::DirectionFixture::photographic(&lane.join("imagery"));
            table.service.living = LivingScenes::start(
                fixture.imagery.clone(),
                imagery::ReferenceStore::new(
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../assets/ui/generated"),
                ),
                SocialLedger::none(),
                ContentRating::Restrained,
                living_scenes::Timing::from_interval(Duration::from_secs(30)),
                false,
                table.updates.clone(),
            );
            let world_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let requested = world_requests.clone();
            let photograph = table.service.living.clone();
            let director = Arc::new(
                fight_foresight::FightForesight::new(Box::new(LikelyExit)).with_world_requester(
                    move |spec| {
                        assert_eq!(spec.place_id(), STREET);
                        assert!(
                            photograph.canonical_room_ready(STREET),
                            "world must wait for its photograph"
                        );
                        requested.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        true
                    },
                ),
            );
            fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = Some(director.clone()));
            fight_foresight::bind_service(table.service.clone());
            let model = model_of(&opening);
            let exit = model
                .ways_out
                .iter()
                .find(|exit| exit.id == "street-door")
                .unwrap();
            let place = crate::gameplay::table_log::LinePlace {
                id: HOME.to_owned(),
                facts: vec!["A small cloth shop".to_owned()],
                exits: vec![crate::gameplay::table_log::LineExit {
                    id: exit.id.clone(),
                    place_id: exit.beyond_place_id.clone(),
                    description: exit.beyond.clone(),
                }],
            };
            for _ in 0..3 {
                director.after_line(&place, &[], |_| false).await;
                director.prepare_after_line(&opening, HOME).await;
            }
            let destination = fight_foresight::proposed_destination(HOME).expect("prepared street");
            let mut changes = table.service.living.subscribe().unwrap();
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if table
                        .service
                        .living
                        .canonical_room_ready(&destination.place_id)
                    {
                        break;
                    }
                    changes.changed().await.unwrap();
                }
            })
            .await
            .expect("fake destination photograph becomes ready");
            assert_eq!(
                world_requests.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "photo completion requests the world before the fight"
            );
            assert_eq!(
                storyteller.primary_connections(),
                1,
                "one place/people/things creation"
            );
            assert!(
                fixture
                    .imagery
                    .hub()
                    .for_subject(&format!("place:{}", destination.place_id))
                    .iter()
                    .any(|row| row.state == "ready")
            );
            table
                .service
                .prepare_fight_arena(&opening, operation(0x78))
                .await
                .unwrap_or_else(|_| panic!("opening reuses the prepared arena"));
            assert_eq!(
                storyteller.primary_connections(),
                1,
                "opening must not create another record"
            );
            assert_eq!(
                world_requests.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "opening must reuse the already requested world"
            );
            assert_eq!(
                fight_foresight::proposed_destination(HOME),
                Some(destination)
            );
            assert_eq!(
                table.current().basis(),
                opening.basis(),
                "preparation never moves the party"
            );
            assert!(place::created(table.current().state()).unwrap().is_empty());
            fight_foresight::TEST_DIRECTOR.with(|slot| *slot.borrow_mut() = None);
        })
    });
}

#[test]
fn place_move_travel_line_reaches_market_square_for_one_four_and_six_heroes() {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    for count in [1, 4, 6] {
        pack::with_test_pack(loaded.clone(), || {
            use crate::gameplay::beat_script_runtime::{
                OpeningSkill, OpeningSocialAttempt, opening_social_tests, stage_opening_social,
            };
            use df_rules::influence::{Attitude, Willingness};
            let opening = opening_social_tests::opening(count);
            let member = journey::participants(&opening)[0].member;
            let settled = stage_opening_social(
                &opening,
                operation(90),
                OpeningSocialAttempt {
                    member,
                    skill: OpeningSkill::Persuasion,
                    attitude: Attitude::Hostile,
                    willingness: Willingness::Hesitant,
                    faces: &[20, 20],
                    changed_after: None,
                },
            )
            .unwrap()
            .checkpoint;
            let exits = FakeEndpoint::start(vec![choice("o2")]);
            let mut market = place_slot();
            market["name"] = json!("Market Square");
            market["narration"] =
                json!("Market stalls ring the square. Carters make room for the arriving party.");
            let storyteller = FakeEndpoint::start(vec![Reply::Output(answer(
                &market,
                &people_slot(),
                &things_slot(),
            ))]);
            let cache = cache_directory("hs26-market-travel");
            let voice = voice_of(
                &exits,
                teller(storyteller.address, 0, Duration::from_secs(5), &cache),
            );
            let table = Table::start(settled.clone(), loaded.clone(), voice);
            let scene = runtime::scene(settled.state()).unwrap();
            let request = line(&table, 91, "Let's go to the market square.");
            let reply = run(table.service.travel_by_line([1; 32], &request, &scene)).unwrap();
            let DialogueReply::Raw(response, None) = reply else {
                panic!("travel returns committed raw response")
            };
            assert_eq!(
                response.disposition,
                rpc::DialogueDisposition::Action as i32
            );
            assert!(response.npc_reply.contains("Market Square"));
            assert_eq!(
                runtime::scene(table.current().state()).unwrap().title,
                "Market Square"
            );
            assert_eq!(table.current().state().characters.len(), count);
            assert!(table.current().state().encounters.is_empty());
            assert_eq!(
                table
                    .lines()
                    .iter()
                    .filter(|(speaker, _)| speaker == "Narrator")
                    .count(),
                1
            );
            assert_eq!(exits.primary_connections(), 1);
            assert!(
                exits.primary_bodies()[0]
                    .to_string()
                    .contains("Let's go to the market square.")
            );
        });
    }
}

#[test]
fn place_move_travel_refuses_the_unresolved_enforcer_and_unstarted_room() {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded.clone(), || {
        let room = journey::initial().unwrap();
        assert!(travel_blocker(&room).unwrap().unwrap().contains("heroes"));
        assert!(travel_options(&room).unwrap().is_empty());
        for count in [1, 4, 6] {
            let opening = runtime::opening_social_tests::opening(count);
            let scene = runtime::scene(opening.state()).unwrap();
            let table = Table::start(
                opening.clone(),
                loaded.clone(),
                Arc::new(NpcVoice::silent()),
            );
            let request = line(
                &table,
                92,
                "Courier, you walk with us up to the market square.",
            );
            let reply = run(table.service.travel_by_line([1; 32], &request, &scene)).unwrap();
            let DialogueReply::Raw(response, None) = reply else {
                panic!("fictional refusal")
            };
            assert_eq!(
                response.disposition,
                rpc::DialogueDisposition::Rejected as i32
            );
            assert!(response.npc_reply.contains("enforcer"));
            assert_eq!(table.current(), opening);
            assert!(travel_options(&opening).unwrap().is_empty());
        }
    });
}
#[test]
fn market_departure_to_a_recorded_stall_keeps_its_keeper_and_stock_and_reuses_the_spot() {
    with_shop(|opening, loaded| {
        let market = admitted_market(&model_of(&opening));
        let current = commit(&opening, 120, "street-door", STREET, Some(&market)).unwrap();
        let mut spot = named_place("fish stall");
        let mut draft: Value = serde_json::from_str(spot["place"].as_str().unwrap()).unwrap();
        draft["speaker"] = json!("");
        draft["greeting"] = json!("");
        draft["exits"] = json!([]);
        draft["landmarks"] = json!([]);
        draft["market"] = json!(false);
        draft["stalls"] = json!([]);
        draft["mentioned_stalls"] = json!([]);
        draft["narration"] = json!(
            "The party gathers at Yara's fish stall. Herring costs 2 cp and trout 3 cp; Market Street stays behind them."
        );
        spot["place"] = json!(draft.to_string());
        rename_slot_person(&mut spot, 2, "Yara");
        // The departure judge says stay: this is inside the place. The local judge picks fish.
        let (_choices, story, voice) = travel_voice(
            vec![choice("o7"), choice("o3"), choice("o1")],
            vec![Reply::Output(spot)],
            "HS40-market-stall",
        );
        let table = Table::start(current, loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let reply = run(table.service.travel_by_line([1;32], &line(&table, 121,
            "I walk over to the fish stall by the fountain. What do they sell and what are the prices?"), &scene)).unwrap();
        let told = replied(Some(reply));
        assert!(told.contains("Herring costs 2 cp and trout 3 cp"));
        assert!(!told.contains("clear destination"));
        let close = table.current();
        let model = model_of(&close);
        let keeper = model
            .people
            .iter()
            .find(|person| person.name == "Yara")
            .unwrap();
        let original = market.expand().unwrap();
        let original_keeper = original
            .model
            .people
            .iter()
            .find(|person| person.name == "Yara")
            .unwrap();
        assert_eq!(keeper.id, original_keeper.id);
        assert_eq!(keeper.coin_cp, original_keeper.coin_cp);
        assert_eq!(keeper.description, original_keeper.description);
        assert_eq!(keeper.persona, original_keeper.persona);
        let goods = model
            .offers
            .iter()
            .filter(|offer| offer.seller_id == keeper.id)
            .map(|offer| (offer.name.as_str(), offer.cost_cp))
            .collect::<Vec<_>>();
        assert_eq!(goods, [("herring", 2), ("trout", 3)]);
        assert_eq!(model.ways_out[0].beyond_place_id, STREET);
        let close_id = runtime::scene(close.state())
            .unwrap()
            .setting
            .unwrap()
            .place_id;
        run(table
            .service
            .move_party_through([1; 32], operation(122), place::WAY_BACK))
        .unwrap_or_else(|_| panic!("recorded way back"));
        let returned = runtime::scene(table.current().state()).unwrap();
        run(table.service.local_reference_call(
            [1; 32],
            &line(&table, 123, "I return to that fish stall."),
            &returned,
            false,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            runtime::scene(table.current().state())
                .unwrap()
                .setting
                .unwrap()
                .place_id,
            close_id
        );
        assert_eq!(story.primary_connections(), 1);
        let sent = story.primary_bodies()[0]["input"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            sent.contains("local_stall")
                && sent.contains("arrival_question")
                && sent.contains("Yara")
        );
    });
}

#[test]
fn walking_to_a_stall_leaves_out_people_the_market_already_has_instead_of_failing() {
    with_shop(|opening, loaded| {
        let market = admitted_market(&model_of(&opening));
        let current = commit(&opening, 130, "street-door", STREET, Some(&market)).unwrap();
        let mut spot = named_place("fish stall");
        let mut draft: Value = serde_json::from_str(spot["place"].as_str().unwrap()).unwrap();
        draft["speaker"] = json!("");
        draft["greeting"] = json!("");
        draft["exits"] = json!([]);
        draft["landmarks"] = json!([]);
        draft["market"] = json!(false);
        draft["stalls"] = json!([]);
        draft["mentioned_stalls"] = json!([]);
        draft["narration"] = json!("The party gathers at Yara's fish stall by the fountain.");
        spot["place"] = json!(draft.to_string());
        // The generator stands Hobb (who already keeps the cart stall) at the fish stall too, and
        // gives the keeper an out-of-range purse; both are the market's own people.
        rename_slot_person(&mut spot, 0, "Hobb");
        rename_slot_person(&mut spot, 2, "Yara");
        let mut people: Value = serde_json::from_str(spot["people"].as_str().unwrap()).unwrap();
        people[2]["purse_cp"] = json!(u64::from(place::MAXIMUM_PURSE_CP) + 1);
        spot["people"] = json!(people.to_string());
        let (_choices, _story, voice) = travel_voice(
            vec![choice("o7"), choice("o3"), choice("o1")],
            vec![Reply::Output(spot)],
            "walk-to-stall-known-people",
        );
        let table = Table::start(current, loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let reply = run(table.service.travel_by_line(
            [1; 32],
            &line(
                &table,
                131,
                "I walk over to the fish stall by the fountain.",
            ),
            &scene,
        ))
        .unwrap();
        let told = replied(Some(reply));
        assert!(!told.contains("clear destination"), "{told}");
        let model = model_of(&table.current());
        let original = market.expand().unwrap().model;
        let yara = |people: &[crate::gameplay::adventure::scene_model::ScenePerson]| {
            people
                .iter()
                .find(|person| person.name == "Yara")
                .map(|person| (person.id.clone(), person.persona.clone()))
        };
        assert_eq!(yara(&model.people), yara(&original.people));
        assert_eq!(
            model
                .people
                .iter()
                .filter(|person| person.name == "Hobb")
                .count(),
            0,
            "Hobb stays the market's own person, not a second copy at the stall"
        );
    });
}

#[test]
fn market_look_lists_distinct_stalls_and_their_goods_for_the_next_choice() {
    with_shop(|opening, _| {
        let market = street(&model_of(&opening));
        let current = commit(&opening, 124, "street-door", STREET, Some(&market)).unwrap();
        let scene = runtime::scene(current.state()).unwrap();
        let facts = investigation::place_facts(&scene).unwrap().join(" ");
        assert!(facts.contains("Stalls in view:"));
        for (stall, goods) in [
            ("cart stall", "oats"),
            ("spice stall", "saffron pinch"),
            ("fish stall", "herring"),
        ] {
            assert!(facts.contains(stall), "{facts}");
            assert!(facts.contains(goods), "{facts}");
        }
        let room = target_named(&scene, "Market Street");
        let line = investigation::plain_look(&scene, &room, "Pip")
            .unwrap()
            .fallback();
        assert!(
            line.contains("fish stall") && line.contains("herring"),
            "{line}"
        );
    });
}

fn with_hs57_free_roam(run: impl FnOnce(Table, PlaceRecord)) {
    // The production prepared-place cache is process-wide; synthetic worlds cannot overlap.
    static WORLD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _world = WORLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    with_shop(|opening, mut loaded| {
        let scene = runtime::scene(opening.state()).unwrap();
        let destination = street(&model_of(&opening));
        let mut gate = destination.clone();
        gate.place_id = HOME.to_owned();
        gate.name = scene.setting.as_ref().unwrap().name.clone();
        gate.speaker.clear();
        gate.greeting.clear();
        static NEXT_WORLD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let number = NEXT_WORLD.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        loaded.pack.beat_script.as_mut().unwrap().free_roam =
            Some(crate::gameplay::adventure::free_roam::World {
                city: crate::gameplay::adventure::free_roam::City {
                    name: format!("HS57 City {number}"),
                    character: "a city of public streets".into(),
                    mood: "quiet".into(),
                    time_of_day: destination.origin.time_of_day.clone(),
                    weather: destination.origin.weather.clone(),
                },
                gate,
                ahead: vec![crate::gameplay::adventure::free_roam::PreparedPlace {
                    from: HOME.into(),
                    exit: "street-door".into(),
                    place: destination.clone(),
                }],
            });
        pack::with_test_pack(loaded.clone(), || {
            let table = Table::start(
                opening,
                loaded,
                Arc::new(NpcVoice::new(Arc::new(Storyteller::disabled()))),
            );
            run(table, destination);
        });
    });
}

struct Hs57Fake {
    release: Arc<Semaphore>,
    entered: Arc<Semaphore>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    cancelled: Arc<std::sync::atomic::AtomicUsize>,
}

impl Hs57Fake {
    fn install(service: &Service, prototype: PlaceRecord, clash: bool) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let fake = Self {
            release: Arc::new(Semaphore::new(0)),
            entered: Arc::new(Semaphore::new(0)),
            calls: Arc::new(AtomicUsize::new(0)),
            cancelled: Arc::new(AtomicUsize::new(0)),
        };
        let (release, entered, calls, cancelled) = (
            fake.release.clone(),
            fake.entered.clone(),
            fake.calls.clone(),
            fake.cancelled.clone(),
        );
        *service.look_ahead.generator.lock().unwrap() = Some(Arc::new(move |request| {
            let count = calls.fetch_add(1, Ordering::SeqCst);
            let mut record = prototype.clone();
            let exit = request.exit.as_ref().unwrap();
            record.name = if clash && count < 2 {
                "Shared Square".into()
            } else {
                format!("Prepared {} {}", request.origin_name, exit.id)
            };
            record.place_id = place::place_id(&record.name, &request.taken_ids);
            record.origin.id = request.origin_id.clone();
            record.origin.name = request.origin_name.clone();
            record.origin.time_of_day = request.time_of_day.clone();
            record.origin.weather = request.weather.clone();
            record.speaker.clear();
            record.greeting.clear();
            record.narration = format!("You arrive at {}.", record.name);
            for (index, person) in record.people.iter_mut().enumerate() {
                let previous = person.name.clone();
                person.name = format!("Citizen{count}x{index}");
                for ware in &mut record.wares {
                    if ware.seller == previous {
                        ware.seller = person.name.clone();
                    }
                }
                for service in &mut record.services {
                    if service.provider == previous {
                        service.provider = person.name.clone();
                    }
                }
                for stall in &mut record.stalls {
                    if stall.keeper == previous {
                        stall.keeper = person.name.clone();
                    }
                }
            }
            record.validate().unwrap();
            let (release, entered, cancelled) =
                (release.clone(), entered.clone(), cancelled.clone());
            Box::pin(async move {
                struct Pending(Arc<AtomicUsize>, bool);
                impl Drop for Pending {
                    fn drop(&mut self) {
                        if self.1 {
                            self.0.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
                let mut pending = Pending(cancelled, true);
                entered.add_permits(1);
                release.acquire().await.unwrap().forget();
                pending.1 = false;
                Ok(record)
            })
        }));
        fake
    }

    async fn wait_for_calls(&self, count: u32) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.acquire_many(count))
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
}

#[test]
fn hs57_prepared_move_commits_while_two_slow_neighbours_are_pending() {
    with_hs57_free_roam(|table, destination| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let narration = destination.narration.clone();
        let fake = Hs57Fake::install(&table.service, destination, false);
        runtime.block_on(async {
            let started = Instant::now();
            let moved = tokio::time::timeout(
                Duration::from_secs(1),
                table
                    .service
                    .move_party_through([1; 32], operation(200), "street-door"),
            )
            .await
            .unwrap()
            .unwrap_or_else(|_| panic!("prepared destination moves"));
            let move_ms = started.elapsed().as_millis();
            assert_eq!(moved.to, STREET);
            assert_eq!(moved.line, narration);
            fake.wait_for_calls(2).await;
            assert_eq!(fake.release.available_permits(), 0);
            assert!(
                crate::gameplay::adventure::free_roam::prepared(&table.current())
                    .iter()
                    .all(|ready| ready.from != STREET)
            );
            println!("HS57 prepared_move_ms={move_ms} pending_neighbours=2");
            table.service.look_ahead.cancel();
            tokio::task::yield_now().await;
            assert_eq!(fake.cancelled.load(std::sync::atomic::Ordering::SeqCst), 2);
        });
    });
}

#[test]
fn hs57_second_move_cancels_stale_neighbours_and_preserves_prepared_memory() {
    with_hs57_free_roam(|table, destination| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fake = Hs57Fake::install(&table.service, destination, false);
        runtime.block_on(async {
            table
                .service
                .move_party_through([1; 32], operation(201), "street-door")
                .await
                .unwrap_or_else(|_| panic!("prepared destination moves"));
            fake.wait_for_calls(2).await;
            let started = Instant::now();
            table
                .service
                .move_party_to_known([1; 32], operation(202), HOME)
                .await
                .unwrap_or_else(|_| panic!("known return moves"));
            tokio::task::yield_now().await;
            assert_eq!(fake.cancelled.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert!(
                crate::gameplay::adventure::free_roam::prepared(&table.current())
                    .iter()
                    .all(|ready| ready.from != STREET)
            );
            println!(
                "HS57 cancel_on_second_move_ms={} cancelled=2",
                started.elapsed().as_millis()
            );
            table.service.look_ahead.cancel();
        });
    });
}

#[test]
fn hs57_concurrent_neighbours_regenerate_a_name_clash_before_remembering() {
    with_hs57_free_roam(|table, destination| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fake = Hs57Fake::install(&table.service, destination, true);
        runtime.block_on(async {
            table
                .service
                .move_party_through([1; 32], operation(203), "street-door")
                .await
                .unwrap_or_else(|_| panic!("prepared destination moves"));
            fake.wait_for_calls(2).await;
            let started = Instant::now();
            fake.release.add_permits(3);
            fake.wait_for_calls(1).await;
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if crate::gameplay::adventure::free_roam::prepared(&table.current())
                        .iter()
                        .filter(|ready| ready.from == STREET)
                        .count()
                        == 2
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let prepared = crate::gameplay::adventure::free_roam::prepared(&table.current());
            let neighbours: Vec<_> = prepared
                .iter()
                .filter(|ready| ready.from == STREET)
                .collect();
            assert_ne!(neighbours[0].place.name, neighbours[1].place.name);
            assert_ne!(neighbours[0].place.place_id, neighbours[1].place.place_id);
            assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
            println!(
                "HS57 concurrent_clash_ms={} initial_in_flight=2 regenerations=1",
                started.elapsed().as_millis()
            );
        });
    });
}

#[test]
fn hs57_move_into_pending_neighbour_reuses_only_its_generation() {
    with_hs57_free_roam(|table, destination| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fake = Hs57Fake::install(&table.service, destination, false);
        runtime.block_on(async {
            table.service.move_party_through([1; 32], operation(204), "street-door")
                .await.unwrap_or_else(|_| panic!("prepared destination moves"));
            fake.wait_for_calls(2).await;
            let exit = model_of(&table.current()).ways_out.into_iter()
                .find(|exit| exit.id != place::WAY_BACK).unwrap();
            let mut moving = Box::pin(table.service.move_party_through([1; 32], operation(205), &exit.id));
            tokio::select! {
                _ = &mut moving => panic!("pending destination must wait for its existing generation"),
                () = tokio::task::yield_now() => {}
            }
            assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            fake.release.add_permits(1);
            let started = Instant::now();
            let moved = tokio::time::timeout(Duration::from_secs(2), moving).await.unwrap()
                .unwrap_or_else(|_| panic!("in-flight destination moves when released"));
            assert_eq!(moved.exit, exit.id);
            assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 2,
                "the sibling stays pending; travel must not request a duplicate destination");
            println!("HS57 reuse_after_release_ms={} generation_calls=2", started.elapsed().as_millis());
            table.service.look_ahead.cancel();
        });
    });
}

#[test]
fn hs57_startup_warmup_returns_before_generating_neighbours() {
    with_hs57_free_roam(|table, destination| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fake = Hs57Fake::install(&table.service, destination, false);
        runtime.block_on(async {
            table
                .service
                .move_party_through([1; 32], operation(206), "street-door")
                .await
                .unwrap_or_else(|_| panic!("prepared destination moves"));
            table.service.look_ahead.cancel();
            let started = Instant::now();
            tokio::time::timeout(
                Duration::from_secs(1),
                table.service.prepare_free_roam_current(),
            )
            .await
            .unwrap()
            .unwrap();
            println!(
                "HS57 startup_warmup_return_ms={}",
                started.elapsed().as_millis()
            );
            fake.wait_for_calls(2).await;
            assert_eq!(fake.release.available_permits(), 0);
            table.service.look_ahead.cancel();
        });
    });
}

#[test]
fn hs57_last_service_owner_drop_aborts_pending_lookahead() {
    with_hs57_free_roam(|table, destination| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fake = Hs57Fake::install(&table.service, destination, false);
        let owner = Arc::downgrade(&table.service.look_ahead);
        runtime.block_on(async move {
            table
                .service
                .move_party_through([1; 32], operation(207), "street-door")
                .await
                .unwrap_or_else(|_| panic!("prepared destination moves"));
            fake.wait_for_calls(2).await;
            drop(table);
            tokio::task::yield_now().await;
            assert!(
                owner.upgrade().is_none(),
                "task data must not keep its owner alive"
            );
            assert_eq!(fake.cancelled.load(std::sync::atomic::Ordering::SeqCst), 2);
        });
    });
}

#[test]
fn hs57_failed_generation_does_not_remember_a_destination() {
    with_hs57_free_roam(|table, _| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        *table.service.look_ahead.generator.lock().unwrap() = Some(Arc::new(|_| {
            Box::pin(async { Err(place_creation::CreationError::Model) })
        }));
        runtime.block_on(async {
            table
                .service
                .move_party_through([1; 32], operation(208), "street-door")
                .await
                .unwrap_or_else(|_| panic!("prepared destination moves"));
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let complete = table
                        .service
                        .look_ahead
                        .lock()
                        .as_ref()
                        .is_some_and(|batch| {
                            batch
                                .exits
                                .iter()
                                .all(|exit| matches!(*exit.outcome.borrow(), AheadOutcome::Failed))
                        });
                    if complete {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(
                crate::gameplay::adventure::free_roam::prepared(&table.current())
                    .iter()
                    .all(|ready| ready.from != STREET)
            );
        });
    });
}

#[test]
fn hs57_dropped_move_caller_audits_cancellation_without_duplicate_generation() {
    with_hs57_free_roam(|table, destination| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fake = Hs57Fake::install(&table.service, destination, false);
        runtime.block_on(async {
            table
                .service
                .move_party_through([1; 32], operation(209), "street-door")
                .await
                .unwrap_or_else(|_| panic!("prepared destination moves"));
            fake.wait_for_calls(2).await;
            let exit = model_of(&table.current())
                .ways_out
                .into_iter()
                .find(|exit| exit.id != place::WAY_BACK)
                .unwrap();
            let mut moving = Box::pin(table.service.move_party_through(
                [1; 32],
                operation(210),
                &exit.id,
            ));
            tokio::select! {
                _ = &mut moving => panic!("destination still pending"),
                () = tokio::task::yield_now() => {}
            }
            drop(moving);
            assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert_eq!(
                runtime::scene(table.current().state())
                    .unwrap()
                    .setting
                    .unwrap()
                    .place_id,
                STREET
            );
            assert!(
                actor::audit_lines_containing("PLACE move outcome=cancelled ms=")
                    .iter()
                    .any(|line| line.contains(&format!(
                        "operation_hash={}",
                        df_observe::operation_hash(operation(210).as_bytes())
                    )))
            );
            table.service.look_ahead.cancel();
        });
    });
}

#[test]
fn a_move_with_an_unreadable_input_identity_is_answered_and_nothing_moves() {
    with_shop(|opening, loaded| {
        let scene = runtime::scene(opening.state()).unwrap();
        let exits = FakeEndpoint::start(vec![choice("o1")]);
        let storyteller = FakeEndpoint::start(vec![]);
        let voice = voice_of(
            &exits,
            teller(
                storyteller.address,
                0,
                Duration::from_secs(5),
                &cache_directory("place-move-unreadable-identity"),
            ),
        );
        let table = Table::start(opening.clone(), loaded, voice);
        let mut request = line(&table, 84, "I go out through the front door.");
        request.input_id = Some(rpc::OperationId {
            value: Some(vec![84; 3]),
        });
        let Some(DialogueReply::Raw(response, _)) = run(table.service.leave_by_freeform(
            [1; 32],
            &request,
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        )) else {
            panic!("a move with an unreadable identity is answered in fiction");
        };
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Rejected as i32
        );
        assert_eq!(response.npc_reply, MOVE_NOT_RECORDED);
        assert_eq!(
            table.lines().last().map(|told| told.1.clone()),
            Some(MOVE_NOT_RECORDED.to_owned())
        );
        assert_eq!(table.current(), opening);
        assert!(place::created(table.current().state()).unwrap().is_empty());
    });
}

#[test]
fn a_move_the_session_owner_cannot_take_is_answered_and_nothing_moves() {
    with_shop(|opening, loaded| {
        let scene = runtime::scene(opening.state()).unwrap();
        let exits = FakeEndpoint::start(vec![choice("o1")]);
        let storyteller = FakeEndpoint::start(vec![Reply::Output(answer(
            &place_slot(),
            &people_slot(),
            &things_slot(),
        ))]);
        let voice = voice_of(
            &exits,
            teller(
                storyteller.address,
                0,
                Duration::from_secs(5),
                &cache_directory("place-move-owner-stopped"),
            ),
        );
        let table = Table::start(opening.clone(), loaded, voice);
        let _ = table.sender.stop();
        let Some(DialogueReply::Raw(response, _)) = run(table.service.leave_by_freeform(
            [1; 32],
            &line(&table, 85, "I go out through the front door."),
            &scene,
            "Brynn",
            0,
            &performed_move(true),
        )) else {
            panic!("a move the owner cannot take is answered in fiction");
        };
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Rejected as i32
        );
        assert_eq!(response.npc_reply, MOVE_NOT_RECORDED);
        assert_eq!(
            table.lines().last().map(|told| told.1.clone()),
            Some(MOVE_NOT_RECORDED.to_owned())
        );
        assert_eq!(table.current(), opening);
    });
}

#[test]
fn a_ride_whose_departure_has_gone_is_not_taken_and_its_paid_fare_is_said_to_stand() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let (_jev, _storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o2"), choice("o6")],
            vec![Reply::Output(named_place("The City Centre"))],
            "place-ride-departure-gone",
        );
        let table = Table::start(out, loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let request = line(
            &table,
            113,
            "I wave down a passing carriage and ask the driver to take us into the heart of the city.",
        );
        let mut ride = run(table.service.hailed_ride([1; 32], &request, &scene)).expect("a ride");
        ride.fare = Some(RideFare {
            service_id: "service-carriage".to_owned(),
            price_cp: 5,
            name: "Carriage ride".to_owned(),
            provider: "Driver".to_owned(),
        });
        // The party leaves by another route after the ride was prepared and before it is taken.
        run(table
            .service
            .move_party_to_known([1; 32], operation(114), HOME))
        .unwrap_or_else(|_| panic!("the dev move home"));
        let before = table.current();
        let Some(DialogueReply::Raw(response, _)) =
            run(table.service.take_ride([1; 32], &request, ride, false))
        else {
            panic!("a ride that cannot start is answered in fiction");
        };
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Rejected as i32
        );
        assert_eq!(
            response.npc_reply,
            format!(
                "{} {FARE_KEPT}",
                actor::place_actor::travel_refusal_line(PlaceRefusal::NotHere)
            )
        );
        assert_eq!(table.current(), before);
    });
}

#[test]
fn a_paid_service_ride_whose_journey_cannot_start_is_not_told_as_travel() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let (_jev, _storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o2"), choice("o6")],
            vec![Reply::Output(named_place("The City Centre"))],
            "place-service-ride-not-taken",
        );
        let table = Table::start(out, loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let request = line(
            &table,
            117,
            "I hire the wagon to take us into the heart of the city.",
        );
        let mut ride = run(table.service.hailed_ride([1; 32], &request, &scene)).expect("a ride");
        ride.fare = Some(RideFare {
            service_id: "market-street-hobb-service-wagon-directions".to_owned(),
            price_cp: 5,
            name: "Wagon ride".to_owned(),
            provider: "Hobb".to_owned(),
        });
        // The party leaves by another route after the ride was prepared, so the journey is stale.
        run(table
            .service
            .move_party_to_known([1; 32], operation(118), HOME))
        .unwrap_or_else(|_| panic!("the dev move home"));
        let before = table.current();
        let RideOutcome::NotTaken { line, revision } = run(table
            .service
            .take_ride_outcome([1; 32], &request, ride, false))
        else {
            panic!("a journey that cannot start is not reported as travel");
        };
        let expected = format!(
            "{} {FARE_KEPT}",
            actor::place_actor::travel_refusal_line(PlaceRefusal::NotHere)
        );
        assert_eq!(line, expected);
        assert_eq!(revision, before.basis().revision);
        assert_eq!(table.current(), before);
        assert_eq!(
            table.lines().last().map(|told| told.1.clone()),
            Some(expected)
        );
        assert!(
            !actor::audit_lines_containing(&format!(
                "PLACE ride outcome=fare_kept service=market-street-hobb-service-wagon-directions price_cp=5 reason=not_here key={}",
                ride_key(117)
            ))
            .is_empty()
        );
        assert!(
            !actor::audit_lines_containing(&format!(
                "PLACE ride outcome=not_taken reason=not_here key={}",
                ride_key(117)
            ))
            .is_empty()
        );
    });
}

/// The harbor journey as it stands in its opening threat: departure is blocked until it is settled.
fn with_harbor<R>(run: impl FnOnce(pack::LoadedPack) -> R) -> R {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded.clone(), || run(loaded))
}

#[test]
fn a_hailed_ride_during_a_blocker_takes_no_fare_and_is_refused_in_fiction() {
    with_harbor(|loaded| {
        let current = runtime::opening_social_tests::opening(2);
        let blocker = travel_blocker(&current)
            .unwrap()
            .expect("the opening threat blocks departure");
        let scene = runtime::scene(current.state()).unwrap();
        let (jev, storyteller, voice) =
            travel_voice(vec![choice("o2")], vec![], "place-hail-during-blocker");
        let table = Table::start(current.clone(), loaded, voice);
        let request = line(
            &table,
            121,
            "I hail that carriage and tell the driver to take us to the city centre.",
        );
        let ride = run(table.service.hailed_ride([1; 32], &request, &scene))
            .expect("the hail is judged a ride");
        assert!(ride.fare.is_none(), "a blocked ride has no fare to buy");
        let Some(DialogueReply::Raw(response, _)) =
            run(table.service.take_ride([1; 32], &request, ride, false))
        else {
            panic!("a blocked ride is answered in fiction");
        };
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Rejected as i32
        );
        assert_eq!(response.npc_reply, blocker);
        assert_eq!(table.current(), current);
        // Judged as a ride once; no destination was judged or made.
        assert_eq!(jev.primary_connections(), 1);
        assert_eq!(storyteller.primary_connections(), 0);
        let lines = table.lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0].1, request.text);
        assert_eq!(lines[1].1, blocker);
        assert!(
            !actor::audit_lines_containing(&format!(
                "PLACE ride outcome=not_taken reason=travel_blocked key={}",
                ride_key(121)
            ))
            .is_empty()
        );
    });
}

#[test]
fn a_ride_whose_reply_is_lost_after_the_move_is_told_as_the_arrival() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let (_jev, _storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o2"), choice("o6")],
            vec![
                Reply::Output(named_place("The City Centre")),
                Reply::Output(json!({"travel": "Brynn waves down a passing carriage."})),
            ],
            "place-ride-lost-reply",
        );
        let table = Table::start(out, loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let request = line(
            &table,
            122,
            "I wave down a passing carriage and ask the driver to take us into the heart of the city.",
        );
        let ride = run(table.service.hailed_ride([1; 32], &request, &scene)).expect("a ride");
        // The owner commits the move, then its reply is lost on the way back.
        table
            .lose_reply
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let RideOutcome::Travelled {
            line: told,
            revision,
        } = run(table
            .service
            .take_ride_outcome([1; 32], &request, ride, false))
        else {
            panic!("the party travelled, so the lost reply is still an arrival");
        };
        assert!(
            told.ends_with("Brynn leads the party on to The City Centre."),
            "{told}"
        );
        assert_eq!(revision, table.current().basis().revision);
        assert_eq!(
            runtime::scene(table.current().state()).unwrap().title,
            "The City Centre"
        );
        assert_eq!(
            table.lines().last().map(|told| told.1.clone()),
            Some("Brynn leads the party on to The City Centre.".to_owned())
        );
        assert!(
            !actor::audit_lines_containing(&format!(
                "PLACE ride outcome=arrived_after_lost_reply key={}",
                ride_key(122)
            ))
            .is_empty()
        );
    });
}

#[test]
fn a_ride_the_owner_cannot_take_is_told_as_not_recorded_and_nothing_moves() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let (_jev, _storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o2"), choice("o6")],
            vec![Reply::Output(named_place("The City Centre"))],
            "place-ride-owner-gone",
        );
        let table = Table::start(out, loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let request = line(
            &table,
            123,
            "I wave down a passing carriage and ask the driver to take us into the heart of the city.",
        );
        let ride = run(table.service.hailed_ride([1; 32], &request, &scene)).expect("a ride");
        let before = table.current();
        let _ = table.sender.stop();
        let RideOutcome::NotTaken {
            line: told,
            revision,
        } = run(table
            .service
            .take_ride_outcome([1; 32], &request, ride, false))
        else {
            panic!("a ride the owner cannot take is not reported as travel");
        };
        assert_eq!(told, MOVE_NOT_RECORDED);
        assert_eq!(revision, before.basis().revision);
        assert_eq!(table.current(), before);
        assert_eq!(
            table.lines().last().map(|told| told.1.clone()),
            Some(MOVE_NOT_RECORDED.to_owned())
        );
        assert!(
            !actor::audit_lines_containing(&format!(
                "PLACE ride outcome=not_taken reason=ResourceExhausted key={}",
                ride_key(123)
            ))
            .is_empty()
        );
    });
}

/// The audit key of a ride whose line has input `input` (see `line`).
fn ride_key(input: u8) -> u64 {
    df_observe::operation_hash(&[input; 16])
}

#[test]
fn a_revision_that_advances_for_another_reason_is_not_taken_as_an_arrival() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let (_jev, _storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o2"), choice("o6")],
            vec![Reply::Output(named_place("The City Centre"))],
            "place-ride-unrelated-revision",
        );
        let table = Table::start(out, loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let request = line(
            &table,
            124,
            "I wave down a passing carriage and ask the driver to take us into the heart of the city.",
        );
        let ride = run(table.service.hailed_ride([1; 32], &request, &scene)).expect("a ride");
        let before = table.current();
        // The revision advances, but nothing moves: the owner drops the move's reply.
        table
            .advance_only
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let RideOutcome::NotTaken {
            line: told,
            revision,
        } = run(table
            .service
            .take_ride_outcome([1; 32], &request, ride, false))
        else {
            panic!("a revision change alone is not an arrival");
        };
        assert_eq!(told, MOVE_NOT_RECORDED);
        assert_ne!(revision, before.basis().revision);
        assert_eq!(revision, table.current().basis().revision);
        assert_eq!(
            runtime::scene(table.current().state()).unwrap().title,
            runtime::scene(before.state()).unwrap().title
        );
        assert_eq!(
            table.lines().last().map(|told| told.1.clone()),
            Some(MOVE_NOT_RECORDED.to_owned())
        );
        assert!(
            !actor::audit_lines_containing(&format!(
                "PLACE ride outcome=not_taken reason=Unavailable key={}",
                ride_key(124)
            ))
            .is_empty()
        );
    });
}

#[test]
fn the_restriction_helper_refuses_a_blocked_party_and_passes_a_clear_one() {
    with_harbor(|loaded| {
        let blocked = runtime::opening_social_tests::opening(2);
        let blocker = travel_blocker(&blocked)
            .unwrap()
            .expect("the opening threat blocks departure");
        let table = Table::start(blocked, loaded, Arc::new(NpcVoice::silent()));
        let ride = table
            .service
            .blocked_ride("Dain".to_owned(), 0)
            .expect("a blocked party refuses the ride");
        assert!(ride.fare.is_none());
        let RideJourney::Blocked(refusal) = ride.journey else {
            panic!("the refusal carries the restriction");
        };
        assert_eq!(refusal.line, blocker);
        assert_eq!(refusal.reason, "travel_blocked");
    });
    with_shop(|opening, loaded| {
        let table = Table::start(opening, loaded, Arc::new(NpcVoice::silent()));
        assert!(table.service.blocked_ride("Brynn".to_owned(), 0).is_none());
    });
}

#[test]
fn a_clear_ride_keeps_its_fare_at_the_last_check_and_still_travels() {
    with_shop(|opening, loaded| {
        let made = street(&model_of(&opening));
        let out = commit(&opening, 40, "street-door", STREET, Some(&made)).unwrap();
        let (_jev, _storyteller, voice) = travel_voice(
            vec![choice("o2"), choice("o2"), choice("o6")],
            vec![Reply::Output(named_place("The City Centre"))],
            "place-ride-last-check",
        );
        let table = Table::start(out.clone(), loaded, voice);
        let scene = runtime::scene(table.current().state()).unwrap();
        let request = line(
            &table,
            125,
            "I wave down a passing carriage and ask the driver to take us into the heart of the city.",
        );
        // A clear ride keeps its fare and still travels.
        let mut clear = run(table.service.hailed_ride([1; 32], &request, &scene)).expect("a ride");
        clear.fare = Some(RideFare {
            service_id: "market-street-hobb-service-wagon-directions".to_owned(),
            price_cp: 5,
            name: "Wagon ride".to_owned(),
            provider: "Hobb".to_owned(),
        });
        let clear = table.service.recheck_before_fare(clear);
        assert!(clear.fare.is_some());
        let RideOutcome::Travelled { .. } = run(table
            .service
            .take_ride_outcome([1; 32], &request, clear, false))
        else {
            panic!("a clear ride travels");
        };
    });
}

#[test]
fn a_ride_whose_restriction_holds_at_the_last_check_loses_its_fare_before_purchase() {
    with_harbor(|loaded| {
        let blocked_state = runtime::opening_social_tests::opening(2);
        let blocker = travel_blocker(&blocked_state)
            .unwrap()
            .expect("the opening threat blocks departure");
        let table = Table::start(blocked_state, loaded, Arc::new(NpcVoice::silent()));
        let mut late = table
            .service
            .blocked_ride("Dain".to_owned(), 0)
            .expect("blocked");
        late.fare = Some(RideFare {
            service_id: "market-street-hobb-service-wagon-directions".to_owned(),
            price_cp: 5,
            name: "Wagon ride".to_owned(),
            provider: "Hobb".to_owned(),
        });
        let checked = table.service.recheck_before_fare(late);
        assert!(checked.fare.is_none(), "no fare is bought");
        let RideJourney::Blocked(refusal) = checked.journey else {
            panic!("the last check refuses the ride");
        };
        assert_eq!(refusal.line, blocker);
    });
}
