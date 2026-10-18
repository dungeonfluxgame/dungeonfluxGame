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
        tests::{FakeEndpoint, Reply, cache_directory, teller},
    },
    table_log::TableLog,
    voice::Voice,
};
use df_persistence::local_demo_scope::LocalDemoRole;
use df_session::inbox::{ActorLoop, AdmissionSequence, Reducer, bounded_inbox};
use std::sync::Arc;
use tokio::sync::{Semaphore, watch};

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

fn operation(number: u8) -> OperationId {
    OperationId::from_bytes(&[number; 16]).unwrap()
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
        assert_eq!(names(&scene), ["Hobb", "Wenna"]);
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
        assert_eq!(model.offers.len(), 1);
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
        assert_eq!(names(&display_scene(&again)), ["Hobb", "Wenna"]);
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
            ["Hobb", "Wenna"]
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
        };
        let owner = std::thread::spawn(move || {
            pack::with_test_pack(loaded, || {
                let mut owner = Owner {
                    checkpoint: current,
                    updates: updates_in,
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
        assert_eq!(
            told,
            "Brynn leads the party through the street door to Market Street."
        );
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
        assert_eq!(exits.connections(), 1);
        assert!(
            exits.bodies()[0]
                .to_string()
                .contains("I follow Brakka out of the shop")
        );
        assert_eq!(storyteller.connections(), 1);
        let asked = storyteller.bodies()[0]["input"]
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
        assert!(
            changes
                .iter()
                .any(|line| line.ends_with("exit=street-door created=true"))
        );
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
                "Brynn leads the party through the street door to Market Street.",
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
                "Brynn leads the party through the street door to Market Street.",
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
        assert_eq!(storyteller.connections(), 1, "the street was made once");
        assert_eq!(table.lines().len(), 3);
    });
}

#[test]
fn narration_never_claims_a_change_that_did_not_happen() {
    with_shop(|opening, loaded| {
        let scene = runtime::scene(opening.state()).unwrap();
        // Jev finds no exit in the line: the party stays, the table is told nothing, no place is made.
        // The shop's choices are its street door, a journey elsewhere, and staying (o3).
        let exits = FakeEndpoint::start(vec![choice("o3")]);
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
        assert_eq!(storyteller.connections(), 0);

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
        assert_eq!(unasked.connections(), 0);
        assert!(table.lines().is_empty());

        // Jev names the door but the storyteller cannot make the place beyond it: the party stays
        // where it is, nothing is committed, nothing is told, and the refusal is audited.
        let exits = FakeEndpoint::start(vec![choice("o1")]);
        let mut wrong = place_slot();
        wrong["kind"] = json!("dragon-lair");
        let storyteller = FakeEndpoint::start(vec![
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
        assert!(
            run(table.service.leave_by_freeform(
                [1; 32],
                &line(&table, 82, "I go out through the front door."),
                &scene,
                "Brynn",
                0,
                &performed_move(true),
            ))
            .is_none()
        );
        assert_eq!(table.current(), opening);
        assert!(table.lines().is_empty());
        assert!(
            !actor::audit_lines_containing(
                "PLACE refused from=place-mareks-cloth-shop to=none exit=street-door reason=kind"
            )
            .is_empty()
        );

        // Without Jev nothing is guessed from the words.
        let table = Table::start(opening.clone(), loaded, Arc::new(NpcVoice::silent()));
        assert!(
            run(table.service.leave_by_freeform(
                [1; 32],
                &line(&table, 83, "I leave."),
                &scene,
                "Brynn",
                0,
                &performed_move(true),
            ))
            .is_none()
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
        assert_eq!(exits.connections(), 0);
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
        assert_eq!(storyteller.connections(), 1);
        assert_eq!(exits.connections(), 0);
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
    place["wares"][0]["seller"] = json!("Mira Fen");
    place["services"][0]["provider"] = json!("Oren Vale");
    place["landmarks"] = json!([
        {"name": "the Clock Arch", "direction": "at the end of the street"},
        {"name": "the Old Quay", "direction": "down the steps"},
    ]);
    let mut people = people_slot();
    people[0]["name"] = json!("Oren Vale");
    people[1]["name"] = json!("Mira Fen");
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
        assert_eq!(exits.connections(), 1);
        let asked = storyteller.bodies()[0]["input"]
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
            .any(|line| line.ends_with("exit=journey created=true"))
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
        let criteria = exits.bodies()[0]["questions"]["q1"]["criteria"].to_string();
        assert!(criteria.contains("Marek") && !criteria.contains("journey to Market Street"));
        assert_eq!(storyteller.connections(), 0, "nothing is made");
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
        let asked = storyteller.bodies()[0]["input"]
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
            jev.bodies()[0]
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
        assert_eq!(storyteller.connections(), 1);
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
                .ride_after_service([1; 32], &request, &scene, service))
            .is_none()
        );
        assert_eq!(jev.connections(), 1);
        assert!(jev.bodies()[0].to_string().contains("wagon directions"));
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
        let (told, revision) = run(table
            .service
            .ride_after_service([1; 32], &request, &scene, service))
        .expect("the journey");
        assert!(
            told.ends_with("Brynn leads the party on to The City Centre."),
            "{told}"
        );
        assert_eq!(revision, table.current().basis().revision);
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
        let room = target_named(&scene, "the room");
        let response = run(table
            .service
            .plain_observation(&request, &scene, &room, "Brynn", 0))
        .expect("a plain look");
        let told = response.npc_reply;
        assert!(
            told.starts_with("Brynn looks around Market Street."),
            "{told}"
        );
        assert!(
            told.contains("Hobb, the carter, is beside the wagon."),
            "{told}"
        );
        assert!(
            told.contains("On sale: saffron pinch from Wenna for 1 sp."),
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
            .plain_observation(&request, &scene, &wenna, "Brynn", 0))
        .expect("a plain look")
        .npc_reply;
        assert!(told.starts_with("Brynn looks at Wenna."), "{told}");
        assert!(told.contains("Wenna sells saffron pinch (1 sp)"), "{told}");
        // A search for what is hidden keeps its check.
        assert!(
            run(table
                .service
                .plain_observation(&request, &scene, &wenna, "Brynn", 0))
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
            told.starts_with("Brynn looks at Hobb. Hobb is the carter here, beside the wagon."),
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
