//! Process-isolated gate tests using real handlers and validated local owner candidates.

use super::*;
use crate::gameplay::{
    Service,
    adventure::{RulesMode, record, shop},
    beat_script_variety_tests as native, journey,
    living_scenes::LivingScenes,
    model,
    npc_reply::NpcVoice,
    pack,
    startup_configuration::{ContentRating, VoiceOff},
    table_log::TableLog,
    wire,
};
use axum::extract::Query;
use df_model::checkpoint::{Checkpoint, ReferenceInventory};
use df_persistence::local_demo_scope::LocalDemoRole;
use df_session::inbox::{AdmissionSequence, Reducer, bounded_inbox};
use df_types::OperationId;
use std::{
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Semaphore;

const DISPLAY: [u8; 32] = [0x72; 32];
const PLAYER: [u8; 32] = [0x61; 32];
const CHILD: &str = "DF_K1_OVERRIDE_TEST_CHILD";

struct TestOwner {
    current: Checkpoint,
    updates: watch::Sender<Checkpoint>,
    commits: Arc<AtomicUsize>,
    next_operation: u8,
    table: Arc<TableLog>,
}

impl TestOwner {
    fn commit(&mut self, order: commands::DevOrder) -> commands::DevResult {
        let operation = OperationId::from_bytes(&[self.next_operation; 16]).unwrap();
        self.next_operation += 1;
        let change = match &order.job {
            commands::DevJob::NpcTurn { npc, act } => candidate::Change::NpcTurn {
                turn: Box::new(
                    journey::tactical::npc_turn(&self.current, npc, act, operation)
                        .unwrap_or_else(|error| panic!("{}: {error}", order.job.note())),
                ),
            },
            commands::DevJob::Budget { value, cap } => candidate::Change::Budget {
                value: *value,
                cap: *cap,
            },
            _ => panic!("unexpected gate test order"),
        };
        let rules = [model::rule().unwrap(), journey::rule().unwrap()];
        let content = model::contents().unwrap();
        let resources = journey::resources_for(self.current.state()).unwrap();
        let mut built = candidate::build(
            &self.current,
            &change,
            self.current.basis().revision.next_sequence().unwrap(),
            operation,
            self.current
                .state()
                .decisions
                .last()
                .unwrap()
                .source_policy
                .clone(),
            order.job.note(),
            ReferenceInventory {
                rules: &rules,
                content: &content,
                resources: &resources,
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        built.candidate =
            journey::living_scene::stage_fight_completion(&self.current, built.candidate).unwrap();
        // The fixture publishes the same checkpoint projections a native owner must admit.
        wire::journey_view(&built.candidate, LocalDemoRole::Display, native::first()).unwrap();
        wire::journey_view(&built.candidate, LocalDemoRole::Player, native::first()).unwrap();
        let revision = built.candidate.basis().revision;
        self.current = built.candidate.clone();
        self.updates.send_replace(built.candidate);
        self.commits.fetch_add(1, Ordering::SeqCst);
        commands::DevResult {
            epoch: revision.epoch().get(),
            sequence: revision.sequence(),
            before: built.before,
            after: built.after,
        }
    }
}

impl Reducer<actor::Call> for TestOwner {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        match call {
            actor::Call::Dev {
                credential,
                order,
                reply,
            } => {
                assert_eq!(credential, PLAYER);
                assert!(reply.send(Ok(self.commit(*order))).is_ok());
            }
            actor::Call::View {
                credential,
                request,
                reply,
            } => {
                assert_eq!(
                    df_api::session_id(request.session_id.as_ref()).unwrap(),
                    self.current.basis().session
                );
                assert_eq!(
                    df_api::run_id(request.run_id.as_ref()).unwrap(),
                    self.current.basis().run
                );
                let role = if credential == DISPLAY {
                    LocalDemoRole::Display
                } else {
                    assert_eq!(credential, PLAYER);
                    LocalDemoRole::Player
                };
                let mut view = wire::journey_view(&self.current, role, native::first()).unwrap();
                self.table.attach(&mut view);
                reply.send(Ok((role, view))).unwrap();
            }
            _ => panic!("unexpected gate test owner call"),
        }
    }
}

struct Fixture {
    state: Arc<Shared>,
    owner: Option<std::thread::JoinHandle<()>>,
    commits: Arc<AtomicUsize>,
}

impl Fixture {
    fn new(current: Checkpoint, loaded: pack::LoadedPack) -> Self {
        let (updates_send, updates) = watch::channel(current.clone());
        let (sender, inbox) = bounded_inbox();
        let commits = Arc::new(AtomicUsize::new(0));
        let owner_commits = commits.clone();
        let table = TableLog::new();
        let owner_table = table.clone();
        let owner = std::thread::spawn(move || {
            pack::with_test_pack(loaded, || {
                inbox
                    .run(&mut TestOwner {
                        current,
                        updates: updates_send,
                        commits: owner_commits,
                        next_operation: 100,
                        table: owner_table,
                    })
                    .unwrap();
            })
        });
        let current = updates.borrow().clone();
        let social = crate::gameplay::social::SocialLedger::none();
        let layout = layout::LayoutState::new();
        let service = Service {
            look_ahead: Default::default(),
            portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
            actor: sender,
            display_credential: DISPLAY,
            updates,
            streams: Arc::new(Semaphore::new(2)),
            voice: crate::gameplay::voice::Voice::disabled(VoiceOff::Qualification),
            npc_voice: Arc::new(NpcVoice::silent()),
            table_log: table.clone(),
            social: social.clone(),
            living: LivingScenes::disabled(),
            layout: layout.clone(),
        };
        let (shutdown, _) = watch::channel(false);
        let hero = journey::name(current.state(), current.state().characters[0].entity).unwrap();
        let seats = Seats {
            session_id: Some(current.basis().session.as_bytes().to_vec()),
            run_id: Some(current.basis().run.as_bytes().to_vec()),
            seats: vec![
                Seat {
                    reclaim: None,
                    credential: Some(PLAYER),
                    member: Some(native::first().as_bytes().to_vec()),
                    hero: Some(hero),
                },
                Seat::empty(),
            ],
        };
        Self {
            state: Arc::new(Shared {
                service,
                display_credential: DISPLAY,
                token: Arc::from("override-test-token"),
                screen_tickets: Arc::new(Mutex::new(super::screen_preview::Tickets::default())),
                seat_file: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../artifacts/tmp/lanes/K1/gate-test-seats"),
                shutdown,
                seats: Arc::new(Mutex::new(seats)),
                overrides: Arc::new(Mutex::new(Vec::new())),
                layout,
                recovery: df_persistence::NativeRecoverySource {
                    rules: vec![model::rule().unwrap(), journey::rule().unwrap()],
                    content: model::contents().unwrap(),
                    resources: journey::resources_for(current.state()).unwrap(),
                    resource_resolver: None,
                    asset_resolver: None,
                    assets: Vec::new(),
                    limits: model::limits(),
                },
                history: Arc::new(std::sync::Mutex::new(history::History::new())),
                table,
                social,
                notebook: crate::gameplay::notebook::NotebookLedger::none(),
                lore: Arc::new(crate::gameplay::npc_reply::local_lore::LocalLore::in_memory()),
            }),
            owner: Some(owner),
            commits,
        }
    }

    fn current(&self) -> Checkpoint {
        self.state.service.updates.borrow().clone()
    }

    async fn battle(&self, body: Value) -> Value {
        let before = self.current().basis().revision;
        let (_, Json(answer)) = fight::edit(
            State(self.state.clone()),
            headers(),
            Json(serde_json::from_value(body).unwrap()),
        )
        .await
        .unwrap();
        assert_eq!(answer["status"], "acted", "{answer}");
        assert_eq!(
            self.current().basis().revision,
            before.next_sequence().unwrap()
        );
        answer
    }

    async fn override_value(&self, body: Value) {
        let before = self.current().basis().revision;
        let (_, Json(answer)) =
            jumps::apply_override(State(self.state.clone()), headers(), Json(body))
                .await
                .unwrap();
        assert!(
            answer["status"] == "acted" || answer["status"] == "overridden",
            "{answer}"
        );
        assert_eq!(
            self.current().basis().revision,
            before.next_sequence().unwrap()
        );
    }

    async fn reads(&self, hero: &str) {
        let (_, Json(document)) = fight::read(State(self.state.clone()), headers())
            .await
            .unwrap();
        assert!(document["open"].is_boolean());
        let (_, Json(document)) = fight::log(
            State(self.state.clone()),
            headers(),
            Query(serde_json::from_value(json!({"last":50})).unwrap()),
        )
        .await
        .unwrap();
        assert!(document["outcomes"].is_array());
        let (_, Json(document)) = fight::initiative(State(self.state.clone()), headers())
            .await
            .unwrap();
        assert!(document["initiative"].is_array());
        let (_, Json(document)) = fight::offers(
            State(self.state.clone()),
            headers(),
            Query(serde_json::from_value(json!({"hero":hero})).unwrap()),
        )
        .await
        .unwrap();
        assert!(document["offers"].is_array());
        let (_, Json(document)) = reads::game_state(State(self.state.clone()), headers())
            .await
            .unwrap();
        assert!(document["phase"].is_string());
        assert_eq!(
            document["revision"]["sequence"],
            self.current().basis().revision.sequence()
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.state.service.actor.stop().unwrap();
        let joined = self.owner.take().unwrap().join();
        if !std::thread::panicking() {
            assert!(joined.is_ok(), "test owner failed");
        }
        npc::set_hold("all", false);
        crate::gameplay::dice::clear();
    }
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer override-test-token".parse().unwrap(),
    );
    headers
}

fn refused(outcome: Outcome) {
    let (_, Json(answer)) =
        outcome.expect_err("disabled mutation must refuse before owner admission");
    assert_eq!(answer["status"], "refused");
    assert_eq!(answer["edited"], false);
    assert!(
        answer["reason"]
            .as_str()
            .unwrap()
            .contains("DF_GAMEPLAY_DEV_OVERRIDES=on")
    );
}

async fn disabled(fixture: &Fixture, hero: &str) {
    let before = fixture.current();
    for body in [
        json!({"op":"start","foe":"bandit","count":1}),
        json!({"op":"end","outcome":"victory"}),
        json!({"op":"restore","person":hero}),
        json!({"op":"add","foe":"wolf"}),
        json!({"op":"remove","name":"Bandit 1"}),
        json!({"op":"scene","scene":"salt-quay"}),
        json!({"op":"edit","name":hero,"kind":"hp","value":1}),
    ] {
        refused(
            fight::edit(
                State(fixture.state.clone()),
                headers(),
                Json(serde_json::from_value(body).unwrap()),
            )
            .await,
        );
    }
    for body in [
        json!({"kind":"budget","value":0}),
        json!({"kind":"hp","hero":hero,"value":1}),
        json!({"kind":"condition","hero":hero,"condition":"prone","present":true}),
        json!({"kind":"cell","hero":hero,"cell":"2,3"}),
        json!({"kind":"resource","hero":hero,"resource":"second-wind","value":0}),
        json!({"kind":"turn","hero":hero}),
        json!({"kind":"initiative","hero":hero,"value":15}),
        json!({"kind":"unknown"}),
    ] {
        refused(jumps::apply_override(State(fixture.state.clone()), headers(), Json(body)).await);
    }
    refused(
        npc::say(
            State(fixture.state.clone()),
            headers(),
            Json(serde_json::from_value(json!({"npc":"merchant","text":"fixture line"})).unwrap()),
        )
        .await,
    );
    for on in [true, false] {
        refused(
            npc::hold(
                State(fixture.state.clone()),
                headers(),
                Json(serde_json::from_value(json!({"npc":"all","on":on})).unwrap()),
            )
            .await,
        );
    }
    refused(
        npc::act(
            State(fixture.state.clone()),
            headers(),
            Json(serde_json::from_value(json!({"npc":"Bandit 1","action":"end-turn"})).unwrap()),
        )
        .await,
    );
    assert_eq!(fixture.current().state(), before.state());
    assert_eq!(fixture.current().basis(), before.basis());
    assert_eq!(fixture.commits.load(Ordering::SeqCst), 0);
    assert!(fixture.state.overrides.lock().await.is_empty());
    assert!(fixture.state.table.lines().is_empty());
    assert!(!npc::held("all"));
}

fn detached_battle(current: &Checkpoint) -> df_rules::tactical::Battle {
    let saved = current.state().encounters[0].tactical.as_ref().unwrap();
    let map =
        df_rules::tactical::BattleMap::from_json(&saved.scene, saved.layout.as_str()).unwrap();
    df_rules::tactical::Battle::from_snapshot(map, saved.rules_state()).unwrap()
}

fn actor_name(current: &Checkpoint, id: u32) -> String {
    let saved = current.state().encounters[0].tactical.as_ref().unwrap();
    let entity = saved
        .actors
        .iter()
        .find(|(actor, _)| *actor == id)
        .unwrap()
        .1;
    journey::name(current.state(), entity).unwrap()
}

async fn enabled(fixture: &Fixture, hero: &str) {
    let scene = crate::gameplay::beat_script_runtime::scene(fixture.current().state()).unwrap();
    let speaker = scene.cast.first().unwrap();
    let (_, Json(answer)) = npc::say(
        State(fixture.state.clone()),
        headers(),
        Json(serde_json::from_value(json!({"npc":speaker.id,"text":"fixture line"})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(answer["status"], "said");
    assert_eq!(
        fixture.state.table.lines().last().unwrap().text,
        "fixture line"
    );
    let (_, Json(answer)) = npc::hold(
        State(fixture.state.clone()),
        headers(),
        Json(serde_json::from_value(json!({"npc":"all","on":true})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(answer["status"], "held");
    assert!(npc::held("all"));
    fixture
        .override_value(json!({"kind":"budget","value":0}))
        .await;
    assert_eq!(fixture.current().state().narrative.remaining_budget, 0);
    fixture
        .battle(json!({"op":"scene","scene":"salt-quay","layout":"open-quay"}))
        .await;
    fixture
        .battle(json!({"op":"start","foe":"bandit","count":2,"level":1}))
        .await;
    fixture.reads(hero).await;
    fixture
        .override_value(json!({"kind":"hp","hero":hero,"value":3}))
        .await;
    assert_eq!(
        detached_battle(&fixture.current())
            .actor(1)
            .unwrap()
            .health()
            .hit_points,
        3
    );
    fixture
        .override_value(json!({"kind":"condition","hero":hero,"condition":"prone","present":true}))
        .await;
    assert!(
        detached_battle(&fixture.current())
            .actor(1)
            .unwrap()
            .health()
            .prone
    );
    fixture
        .override_value(json!({"kind":"condition","hero":hero,"condition":"prone","present":false}))
        .await;
    assert!(
        !detached_battle(&fixture.current())
            .actor(1)
            .unwrap()
            .health()
            .prone
    );
    let current = fixture.current();
    let battle = detached_battle(&current);
    let (cols, rows) = battle.map().dimensions();
    let empty = (0..cols)
        .flat_map(|col| (0..rows).map(move |row| df_rules::tactical::Cell(col, row)))
        .find(|cell| {
            !battle.map().is_blocked(*cell)
                && battle
                    .combatants()
                    .iter()
                    .all(|actor| actor.cell() != *cell)
        })
        .unwrap();
    fixture
        .override_value(json!({"kind":"cell","hero":hero,"cell":format!("{},{}",empty.0,empty.1)}))
        .await;
    assert_eq!(
        detached_battle(&fixture.current()).actor(1).unwrap().cell(),
        empty
    );
    fixture
        .override_value(json!({"kind":"resource","hero":hero,"resource":"second-wind","value":0}))
        .await;
    fixture
        .override_value(json!({"kind":"turn","hero":hero}))
        .await;
    assert_eq!(detached_battle(&fixture.current()).active_actor(), Some(1));
    fixture
        .override_value(json!({"kind":"initiative","hero":hero,"value":15}))
        .await;
    fixture
        .battle(json!({"op":"edit","name":hero,"kind":"hp","value":2}))
        .await;
    fixture.battle(json!({"op":"restore","person":hero})).await;
    fixture.battle(json!({"op":"add","foe":"wolf"})).await;
    let current = fixture.current();
    let battle = detached_battle(&current);
    assert_eq!(battle.combatants().len(), 5);
    let added = battle.combatants().last().unwrap().id();
    let added = actor_name(&current, added);
    fixture.battle(json!({"op":"remove","name":added})).await;
    assert_eq!(detached_battle(&fixture.current()).combatants().len(), 4);
    let current = fixture.current();
    let battle = detached_battle(&current);
    let enemy = battle
        .combatants()
        .iter()
        .find(|actor| actor.side() == df_rules::tactical::Side::Enemies)
        .unwrap()
        .id();
    let enemy = actor_name(&current, enemy);
    fixture
        .override_value(json!({"kind":"turn","hero":enemy}))
        .await;
    let before = fixture.current().basis().revision;
    let (_, Json(answer)) = npc::act(
        State(fixture.state.clone()),
        headers(),
        Json(serde_json::from_value(json!({"npc":enemy,"action":"end-turn"})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(answer["status"], "acted");
    assert_eq!(
        fixture.current().basis().revision,
        before.next_sequence().unwrap()
    );
    for outcome in ["victory", "defeat", "flee"] {
        fixture.battle(json!({"op":"end","outcome":outcome})).await;
        assert!(!journey::tactical::present(fixture.current().state()));
        fixture.reads(hero).await;
        if outcome != "flee" {
            if outcome == "defeat" {
                let current = fixture.current();
                for character in &current.state().characters {
                    let person = journey::name(current.state(), character.entity).unwrap();
                    fixture
                        .battle(json!({"op":"restore","person":person}))
                        .await;
                }
            }
            fixture
                .battle(json!({"op":"start","foe":"bandit","count":1,"level":1}))
                .await;
        }
    }
    let (_, Json(answer)) = npc::hold(
        State(fixture.state.clone()),
        headers(),
        Json(serde_json::from_value(json!({"npc":"all","on":false})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(answer["status"], "released");
    assert!(!npc::held("all"));
}

#[test]
fn gameplay_override_handlers_obey_the_process_opt_in() {
    let _process_state =
        crate::gameplay::journey::tactical::development_tests::process_state_test_scope();
    if std::env::var_os(CHILD).is_none() {
        for setting in [None, Some("off"), Some("on")] {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "gameplay::dev_control::override_tests::gameplay_override_handlers_obey_the_process_opt_in", "--nocapture"])
                .env(CHILD, "1");
            if let Some(setting) = setting {
                child.env("DF_GAMEPLAY_DEV_OVERRIDES", setting);
            } else {
                child.env_remove("DF_GAMEPLAY_DEV_OVERRIDES");
            }
            let result = child.output().unwrap();
            assert!(
                result.status.success(),
                "setting={setting:?}\n{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            println!("override handler matrix setting={setting:?}: passed");
        }
        return;
    }
    let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    let owner_pack = loaded.clone();
    pack::with_test_pack(loaded, || {
        npc::set_hold("all", false);
        let current = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let hero = journey::name(current.state(), current.state().characters[0].entity).unwrap();
        let fixture = Fixture::new(current, owner_pack);
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                fixture.reads(&hero).await;
                if std::env::var("DF_GAMEPLAY_DEV_OVERRIDES").as_deref() == Ok("on") {
                    enabled(&fixture, &hero).await;
                } else {
                    disabled(&fixture, &hero).await;
                }
                fixture.reads(&hero).await;
            });
    });
}

#[test]
fn development_fight_start_and_end_keep_state_fight_and_display_reads_responsive() {
    let _process_state =
        crate::gameplay::journey::tactical::development_tests::process_state_test_scope();
    const CHILD: &str = "DF_K2_FIGHT_READ_TEST_CHILD";
    const TEST: &str = "gameplay::dev_control::override_tests::development_fight_start_and_end_keep_state_fight_and_display_reads_responsive";
    if std::env::var_os(CHILD).is_none() {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "1")
            .env("DF_GAMEPLAY_DEV_OVERRIDES", "on")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    // A synchronous projection deadlock also blocks Tokio's timeout polling. Keep an
    // independent watchdog outside the runtime so that such a regression fails boundedly.
    let (finished, wait) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
        let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
        // The child owns the global pack too: runtime worker threads do not inherit
        // with_test_pack's thread-local override.
        pack::install(loaded.clone()).unwrap();
        let owner_pack = loaded.clone();
        pack::with_test_pack(loaded, || {
            npc::set_hold("all", false);
            crate::gameplay::dice::clear();
            let current = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let mut fixture = Fixture::new(current, owner_pack);
            fixture.state.layout.enable();
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_time()
                .build()
                .unwrap()
                .block_on(async {
                    let shared = Arc::get_mut(&mut fixture.state).unwrap();
                    shared.service.living = LivingScenes::start(
                        crate::gameplay::imagery::Imagery::disabled(
                            crate::gameplay::startup_configuration::ImageryOff::Qualification,
                        ),
                        crate::gameplay::imagery::ReferenceStore::new(
                            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                                .join("../../artifacts/tmp/lanes/K2/references"),
                        ),
                        shared.social.clone(),
                        ContentRating::default(),
                        crate::gameplay::living_scenes::Timing::from_interval(
                            std::time::Duration::from_secs(60),
                        ),
                        true,
                        shared.service.updates.clone(),
                    );
                    let basis = fixture.current().basis();
                    let mut request = tonic::Request::new(rpc::WatchViewRequest {
                        session_id: Some(rpc::SessionId {
                            value: Some(basis.session.as_bytes().to_vec()),
                        }),
                        run_id: Some(rpc::RunId {
                            value: Some(basis.run.as_bytes().to_vec()),
                        }),
                        client_binding_id: Some(rpc::ClientBindingId {
                            value: Some(vec![0x72; 16]),
                        }),
                        after_revision: None,
                    });
                    request.metadata_mut().insert(
                        "x-df-local-binding",
                        crate::gameplay::hex(&DISPLAY).parse().unwrap(),
                    );
                    let mut display_stream = rpc::session_service_server::SessionService::watch(
                        &fixture.state.service,
                        request,
                    )
                    .await
                    .unwrap()
                    .into_inner();
                    use futures::StreamExt;
                    assert!(display_stream.next().await.unwrap().is_ok());
                    // Fix initiative at a hero turn so ending the development fight does not
                    // depend on a randomly opened ordinary opportunity-reaction window.
                    crate::gameplay::dice::enqueue(20, &[20, 20, 1, 1, 1, 1]).unwrap();
                    fixture
                        .battle(json!({"op":"start","foe":"skeleton","count":4,"level":5,"scene":"scan-village"}))
                        .await;
                    for expected_open in [true, false] {
                        let (_, Json(state)) = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            reads::game_state(State(fixture.state.clone()), headers()),
                        )
                        .await
                        .expect("state read deadline")
                        .unwrap();
                        assert_eq!(state["phase"], if expected_open { "combat" } else { "opening" });
                        let (_, Json(fight)) = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            fight::read(State(fixture.state.clone()), headers()),
                        )
                        .await
                        .expect("fight read deadline")
                        .unwrap();
                        assert_eq!(fight["open"], expected_open);
                        let (_, display) = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            display_view(&fixture.state),
                        )
                        .await
                        .expect("display view deadline")
                        .unwrap();
                        assert_eq!(
                            reads::display_journey(&display)["phase"],
                            if expected_open { "combat" } else { "opening" }
                        );
                        tokio::time::timeout(std::time::Duration::from_secs(2), async {
                            loop {
                                let display = display_stream.next().await.unwrap().unwrap();
                                if reads::display_journey(&display)["phase"]
                                    == if expected_open { "combat" } else { "opening" }
                                {
                                    assert_eq!(
                                        display.revision,
                                        Some(wire::revision(fixture.current().basis().revision))
                                    );
                                    break;
                                }
                            }
                        })
                        .await
                        .expect("display watch update deadline");
                        if expected_open {
                            fixture.battle(json!({"op":"end","outcome":"victory"})).await;
                        }
                    }
                });
        });
        finished.send(()).unwrap();
    });
    wait.recv_timeout(std::time::Duration::from_secs(15))
        .expect("development fight/read worker exceeded deadline");
    worker.join().unwrap();
}

#[test]
fn roleplay_routes_inspect_without_committing_or_registering_voice() {
    use tonic::codegen::Service;
    let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, record) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded.clone(), || {
        let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let mut fixture = Fixture::new(opening, loaded);
        let social =
            crate::gameplay::social::SocialLedger::open(record.dossiers, &record.seed, None)
                .unwrap();
        let state = Arc::get_mut(&mut fixture.state).unwrap();
        state.social = social.clone();
        state.service.social = social;
        fixture.state.table.push(
            "Brynn",
            rpc::TableRole::Hero,
            0,
            rpc::TableKind::Speech,
            "Hello.",
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut router = roleplay::routes()
                .route("/dev/timeline", axum::routing::get(reads::timeline))
                .route("/dev/audit", axum::routing::get(audit_log::audit))
                .route("/dev/decisions", axum::routing::get(audit_log::decisions))
                .route(
                    "/dev/screens/ticket",
                    axum::routing::post(screen_preview::issue),
                )
                .route("/dev/place/go", axum::routing::post(place::go))
                .route("/dev/music", axum::routing::get(music::read))
                .route("/dev/music/edit", axum::routing::post(music::edit))
                .route("/dev/presentation", axum::routing::get(roleplay::present))
                .with_state(fixture.state.clone());
            for (path, key) in [
                ("/dev/transcript", "array"),
                ("/dev/timeline?since=0", "next_since"),
                ("/dev/audit?since=0&npc=merchant", "next_since"),
                ("/dev/decisions?since=0&npc=merchant", "next_since"),
                ("/dev/trade-terms?hero=Brynn", "array"),
                ("/dev/npc/show?name=merchant", "dossier"),
                ("/dev/npc/last?name=merchant", "npc_id"),
                ("/dev/narrative", "budget"),
                ("/dev/voice", "array"),
                ("/dev/mood", "faces"),
                ("/dev/presentation", "presentation"),
            ] {
                let request = axum::http::Request::builder()
                    .uri(path)
                    .header("authorization", "Bearer override-test-token")
                    .body(axum::body::Body::empty())
                    .unwrap();
                let response = router.clone().as_service().call(request).await.unwrap();
                let status = response.status();
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                assert_eq!(
                    status,
                    StatusCode::OK,
                    "{path}: {}",
                    String::from_utf8_lossy(&bytes)
                );
                let document: Value = serde_json::from_slice(&bytes).unwrap();
                if key == "array" {
                    assert!(document.is_array(), "{path}: {document}");
                } else {
                    assert!(document.get(key).is_some(), "{path}: {document}");
                }
            }
            for path in [
                "/dev/npc/show?name=unknown",
                "/dev/trade-terms?hero=unknown",
                "/dev/transcript?since=bad",
            ] {
                let request = axum::http::Request::builder()
                    .uri(path)
                    .header("authorization", "Bearer override-test-token")
                    .body(axum::body::Body::empty())
                    .unwrap();
                assert_eq!(
                    router
                        .clone()
                        .as_service()
                        .call(request)
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::BAD_REQUEST,
                    "{path}"
                );
            }
            for (path, body) in [
                ("/dev/screens/ticket", json!({"seat":"7"})),
                ("/dev/place/go", json!({"target":""})),
                ("/dev/place/go", json!({"target":"landmark:unknown"})),
                ("/dev/music/edit", json!({"op":"invalid","place":"shop"})),
            ] {
                let request = axum::http::Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", "Bearer override-test-token")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap();
                assert_eq!(
                    router
                        .clone()
                        .as_service()
                        .call(request)
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::BAD_REQUEST
                );
            }
            {
                let mut seats = fixture.state.seats.lock().await;
                seats.seats.get_mut(1).unwrap().hero = Some("Brynn".into());
            }
            let request = axum::http::Request::builder()
                .uri("/dev/trade-terms?hero=Brynn")
                .header("authorization", "Bearer override-test-token")
                .body(axum::body::Body::empty())
                .unwrap();
            let response = router.clone().as_service().call(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let refusal: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(refusal["error"].as_str().unwrap().contains("ambiguous"));
            let response = router
                .as_service()
                .call(
                    axum::http::Request::builder()
                        .uri("/dev/transcript")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(fixture.commits.load(Ordering::SeqCst), 0);
            assert_eq!(fixture.state.service.voice.development_report(), json!([]));
        });
    });
}

#[test]
fn authenticated_write_commands_have_one_boundary_audit_without_body_or_token() {
    const NAME: &str = "gameplay::dev_control::override_tests::authenticated_write_commands_have_one_boundary_audit_without_body_or_token";
    if std::env::var("DF_HS17_AUDIT_CHILD").as_deref() != Ok("on") {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture"])
            .env("DF_HS17_AUDIT_CHILD", "on")
            .env("DF_GAMEPLAY_DEV_CONTROL", "on")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use tonic::codegen::Service;
    let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded.clone(), || {
        let fixture = Fixture::new(native::prepared(), loaded);
        fixture.state.layout.enable();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let router = Router::new()
                    .route("/dev/presentation", axum::routing::post(presentation_edit))
                    .layer(axum::middleware::from_fn_with_state(
                        fixture.state.clone(),
                        audit_command,
                    ))
                    .with_state(fixture.state.clone());
                let request = axum::http::Request::builder()
                    .method("POST")
                    .uri("/dev/presentation")
                    .header("authorization", "Bearer override-test-token")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"op":"pause"}"#))
                    .unwrap();
                assert_eq!(
                    router
                        .clone()
                        .as_service()
                        .call(request)
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::OK
                );
                let audit = timeline::audit(100, Some("DEV_CONTROL"));
                assert_eq!(audit["returned"], 1);
                let line = audit["events"][0]["line"].as_str().unwrap();
                assert!(line.contains(
                    "route=/dev/presentation method=POST status=200 outcome=answered ms="
                ));
                assert!(!line.contains("override-test-token"));
                assert!(!line.contains("pause"));
                let before = timeline::audit(100, Some("DEV_CONTROL"))["matched"].clone();
                let commits_before_read = fixture.commits.load(Ordering::SeqCst);
                let (_, Json(view)) = reads::view_as(
                    State(fixture.state.clone()),
                    headers(),
                    Query(std::collections::HashMap::from([(
                        "as".into(),
                        "tv".into(),
                    )])),
                )
                .await
                .unwrap();
                assert_eq!(view["projection"], "live_display");
                assert_eq!(fixture.commits.load(Ordering::SeqCst), commits_before_read);
                assert_eq!(timeline::audit(100, Some("DEV_CONTROL"))["matched"], before);
                // The live display projection registers cue metadata even with voice off.
                // Off entries have no synthesis job, selected voice, or playback duration.
                let voices = fixture.state.service.voice.development_report();
                assert!(voices.as_array().unwrap().iter().all(|speaker| {
                    speaker["voice_id"].is_null()
                        && speaker["lines"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|line| line["state"] == "off" && line["ms"].is_null())
                }));
                let request = axum::http::Request::builder()
                    .method("POST")
                    .uri("/dev/presentation")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"op":"pause"}"#))
                    .unwrap();
                assert_eq!(
                    router
                        .clone()
                        .as_service()
                        .call(request)
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::UNAUTHORIZED
                );
                let audit = timeline::audit(100, Some("DEV_CONTROL"));
                assert_eq!(audit["returned"], 3);
                let lines = audit["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|event| event["line"].as_str().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    lines
                        .iter()
                        .filter(|line| line.starts_with("DEV_CONTROL route="))
                        .count(),
                    2
                );
                let refusals = lines
                    .iter()
                    .filter(|line| line.starts_with("DEV_CONTROL refused reason=token"))
                    .collect::<Vec<_>>();
                assert_eq!(refusals.len(), 1);
                let refusal_line = refusals[0];
                assert!(refusal_line.contains("route=/dev/presentation"));
                assert!(!refusal_line.contains("override-test-token"));
                assert!(!refusal_line.contains("pause"));
            });
    });
}
