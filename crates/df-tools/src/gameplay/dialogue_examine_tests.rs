use super::*;
use crate::gameplay::{
    adventure::{RulesMode, record, shop},
    approach_model::tests::{answer, model as jev},
    beat_script_runtime as runtime, beat_script_variety_tests as native,
    journey::living_scene,
    pack,
    place::{self, PlaceFailure},
    place_creation::tests::{answer as place_answer, people_slot, place_slot, things_slot},
    social::SocialLedger,
    startup_configuration::ContentRating,
    storyteller::{
        Storyteller,
        tests::{FakeEndpoint, Reply, teller},
    },
    table_log::TableLog,
};
use df_model::checkpoint::Checkpoint;
use df_session::inbox::{AdmissionSequence, Reducer, bounded_inbox};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, watch};

const WORDS: &str =
    "I wander over to the pottery stall and look closely at the glazes on the bowls.";
const LOOK: &str = "The bowls have a glossy blue glaze, pooling dark around the rims. Sella Voss tends the pottery stall.";

fn operation(byte: u8) -> OperationId {
    OperationId::from_bytes(&[byte; 16]).unwrap()
}

fn cache(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS71")
        .join(format!("{name}-{}", std::process::id()))
}

fn market_slots() -> (Value, Value, Value) {
    let mut place = place_slot();
    place["speaker"] = json!("Sella Voss");
    place["stalls"][0] =
        json!({"name":"pottery stall","keeper":"Sella Voss","position":"beside the wagon"});
    place["mentioned_stalls"] = json!(["pottery stall", "spice stall"]);
    for ware in place["wares"].as_array_mut().unwrap() {
        if ware["seller"] == "Hobb" {
            ware["seller"] = json!("Sella Voss");
            ware["name"] = json!(if ware["name"] == "oats" {
                "bowl"
            } else {
                "cup"
            });
            ware["description"] =
                json!("Clay with glossy blue glaze, pooling dark around the rims.");
        }
    }
    place["services"][0]["provider"] = json!("Sella Voss");
    let mut people = people_slot();
    people[0]["name"] = json!("Sella Voss");
    people[0]["role"] = json!("potter");
    people[0]["position"] = json!("at the pottery stall");
    people[0]["description"] = json!("An adult potter in a clay-stained apron.");
    let mut things = things_slot();
    things[0]["name"] = json!("pottery stall");
    things[0]["detail"] =
        json!("A low table displays clay cups and bowls. It serves as Sella Voss's pottery stall.");
    (place, people, things)
}

fn with_market(test: impl FnOnce(Checkpoint, pack::LoadedPack)) {
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded.clone(), || {
        let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let (place, people, things) = market_slots();
        let mut market = crate::gameplay::place_creation::build(
            &crate::gameplay::place_creation::tests::request(),
            &place.to_string(),
            &people.to_string(),
            &things.to_string(),
        )
        .unwrap();
        // Match a currently generated place: personas are admitted before this record is kept.
        // The pure builder alone also accepts old saves that predate personas.
        run(async {
            let endpoint =
                FakeEndpoint::start(crate::gameplay::place_creation::tests::persona_replies(3));
            let judge = crate::gameplay::place_creation::tests::accepting_persona_judge(3);
            let storyteller = teller(
                endpoint.address,
                0,
                Duration::from_secs(5),
                &cache("market-personas"),
            );
            crate::gameplay::place_creation::author_personas(
                &storyteller,
                &jev(&judge),
                &crate::gameplay::place_creation::tests::request(),
                &mut market,
            )
            .await
            .unwrap();
        });
        let scene = runtime::scene(opening.state()).unwrap();
        let planned = place::plan(
            &opening,
            "Brynn",
            operation(201),
            &scene.setting.unwrap().place_id,
            "street-door",
            &market.place_id,
            Some(&market),
        )
        .unwrap_or_else(|_| panic!("market arrival"));
        test(planned.candidate, loaded);
    });
}

fn run(test: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}

struct Owner {
    current: Checkpoint,
    updates: watch::Sender<Checkpoint>,
    table: Arc<TableLog>,
    registered: std::sync::mpsc::Receiver<()>,
}

impl Owner {
    fn publish(&self) {
        self.updates.send_replace(self.current.clone());
        self.registered.recv().unwrap();
    }
}

impl Reducer<actor::Call> for Owner {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        match call {
            actor::Call::Dialogue {
                request: DialogueRequest::Raw(mut bytes),
                reply,
                ..
            } => {
                let proposal = bytes
                    .prepared_examine
                    .take()
                    .expect("server-prepared inspection");
                let (operation, fingerprint) = scene_examine_identity(&proposal.request).unwrap();
                let line = if let Some(line) = living_scene::examine_narration_replay(
                    &self.current,
                    native::first(),
                    operation,
                    &fingerprint,
                )
                .unwrap()
                {
                    line
                } else {
                    assert_eq!(proposal.expected_basis, self.current.basis());
                    self.current = living_scene::commit_examine_with_story(
                        &self.current,
                        native::first(),
                        operation,
                        living_scene::ExamineNarration {
                            target: &proposal.target,
                            words: &proposal.request.text,
                            fingerprint: &fingerprint,
                            text: &proposal.narration,
                        },
                        proposal.item_story,
                    )
                    .unwrap();
                    self.publish();
                    if proposal.publish {
                        freeform_action::publish_hero_line(
                            &self.table,
                            "Brynn",
                            0,
                            &proposal.request.text,
                        );
                        freeform_action::publish_outcome(
                            &self.table,
                            0,
                            None::<crate::gameplay::table_log::RollRow>,
                            &proposal.narration,
                        );
                    }
                    proposal.narration
                };
                assert!(
                    reply
                        .send(Ok(DialogueReply::Raw(
                            rpc::DialogueResponse {
                                input_id: proposal.request.input_id,
                                disposition: rpc::DialogueDisposition::Action as i32,
                                revision: Some(wire::revision(self.current.basis().revision)),
                                npc_reply: line,
                                ..Default::default()
                            },
                            None
                        )))
                        .is_ok()
                );
            }
            actor::Call::SpeechMember { reply, .. } => {
                reply.send(Ok(native::first())).unwrap();
            }
            actor::Call::ObservationFinding {
                expected_basis,
                record,
                detail,
                reply,
                ..
            } => {
                let previous = self.current.basis();
                self.current = living_scene::commit_observation_with_detail(
                    &self.current,
                    expected_basis,
                    record.clone(),
                    detail,
                )
                .unwrap();
                if self.current.basis() != previous {
                    self.publish();
                    freeform_action::publish_outcome(
                        &self.table,
                        0,
                        None::<crate::gameplay::table_log::RollRow>,
                        &record.text,
                    );
                }
                reply.send(Ok(record)).unwrap();
            }
            actor::Call::PlaceMove {
                proposal, reply, ..
            } => {
                assert_eq!(proposal.expected_basis, self.current.basis());
                let result = match place::plan(
                    &self.current,
                    "Brynn",
                    proposal.operation,
                    &proposal.from,
                    &proposal.exit,
                    &proposal.to,
                    proposal.created.as_ref(),
                ) {
                    Ok(plan) => {
                        self.current = plan.candidate;
                        self.publish();
                        Ok(actor::place_actor::MoveOutcome::Moved(Box::new(
                            actor::place_actor::Moved {
                                revision: self.current.basis().revision,
                                from: proposal.from,
                                to: proposal.to,
                                place_name: plan.to_name,
                                exit: proposal.exit,
                                created: proposal.created.is_some(),
                                line: plan.entry.line,
                                narration: plan.arrival,
                            },
                        )))
                    }
                    Err(PlaceFailure::Refused(reason)) => {
                        Ok(actor::place_actor::MoveOutcome::Refused(reason))
                    }
                    Err(PlaceFailure::State(error)) => Err(actor::unavailable(error)),
                };
                assert!(
                    reply.send(result).is_ok(),
                    "move reply receiver must remain live"
                );
            }
            _ => panic!("unexpected examine owner call"),
        }
    }
}

struct Table {
    service: Service,
    owner: Option<std::thread::JoinHandle<()>>,
    registration: tokio::task::JoinHandle<()>,
}

impl Table {
    fn start(current: Checkpoint, loaded: pack::LoadedPack, voice: NpcVoice) -> Self {
        let (updates, receiver) = watch::channel(current.clone());
        // Production's registry is shared; tests isolate it per thread. Mirror the owner's
        // publication on this current-thread runtime before acknowledging each fake commit.
        living_scene::publish(&current).unwrap();
        let mut registry_updates = receiver.clone();
        let (registered, registration_wait) = std::sync::mpsc::sync_channel(1);
        let registration = tokio::spawn(async move {
            while registry_updates.changed().await.is_ok() {
                living_scene::publish(&registry_updates.borrow().clone()).unwrap();
                if registered.send(()).is_err() {
                    break;
                }
            }
        });
        let (actor, inbox) = bounded_inbox();
        let table = TableLog::new();
        let owner_table = table.clone();
        let owner = std::thread::spawn(move || {
            pack::with_test_pack(loaded, || {
                inbox
                    .run(&mut Owner {
                        current,
                        updates,
                        table: owner_table,
                        registered: registration_wait,
                    })
                    .unwrap();
            })
        });
        Self {
            service: Service {
                actor,
                display_credential: [0x82; 32],
                updates: receiver,
                streams: Arc::new(Semaphore::new(2)),
                voice: crate::gameplay::voice::Voice::disabled(
                    crate::gameplay::startup_configuration::VoiceOff::Qualification,
                ),
                npc_voice: Arc::new(voice),
                table_log: table,
                social: SocialLedger::none(),
                living: crate::gameplay::living_scenes::LivingScenes::disabled(),
                look_ahead: Arc::default(),
                layout: crate::gameplay::dev_control::layout::LayoutState::new(),
                portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
            },
            owner: Some(owner),
            registration,
        }
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        self.service.actor.stop().unwrap();
        self.owner.take().unwrap().join().unwrap();
        self.registration.abort();
    }
}

fn request(current: &Checkpoint, words: &str) -> rpc::RawDialogueRequest {
    rpc::RawDialogueRequest {
        session_id: Some(rpc::SessionId {
            value: Some(current.basis().session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(current.basis().run.as_bytes().to_vec()),
        }),
        observed_revision: Some(wire::revision(current.basis().revision)),
        input_id: Some(rpc::OperationId {
            value: Some(operation(202).as_bytes().to_vec()),
        }),
        text: words.into(),
        finality: rpc::DialogueFinality::Final as i32,
        context: rpc::DialogueContext::Unspecified as i32,
        ..Default::default()
    }
}

fn stall(current: &Checkpoint) -> (crate::gameplay::adventure::scene_model::SceneModel, String) {
    let scene = runtime::scene(current.state()).unwrap();
    let model = living_scene::snapshot(current.state(), &scene)
        .unwrap()
        .unwrap()
        .model;
    let target = model
        .things
        .iter()
        .find(|thing| thing.name == "pottery stall")
        .unwrap()
        .id
        .clone();
    (model, target)
}

#[test]
fn hs59_living_scene_examine_fallback_names_the_present_vendor_without_claiming_ownership() {
    with_market(|current, _| {
        let (mut model, target) = stall(&current);
        // Generated stalls bind their keeper as owner. Remove that legal claim only in this
        // detached fixture, preserving the separate unknown-owner/known-attendant contract.
        model
            .things
            .iter_mut()
            .find(|thing| thing.id == target)
            .unwrap()
            .owner_id = None;
        let thing = model
            .things
            .iter()
            .find(|thing| thing.id == target)
            .unwrap();
        assert_eq!(thing.owner_id, None);
        let line = living_scene::examine_fallback(&model, thing, "Brynn");
        assert!(line.contains("Sella Voss tends the pottery stall."));
        assert!(!line.contains("Nothing shows who owns"));
        assert!(!line.contains("Sella Voss owns"));
    });
}

#[test]
fn hs59_scene_examine_narration_answers_the_glaze_from_recorded_wares() {
    with_market(|current, loaded| {
        run(async {
            let story = FakeEndpoint::start(vec![Reply::Output(json!({"narration":LOOK}))]);
            let judge = FakeEndpoint::start(vec![
                answer("o2"),
                answer("o1"),
                answer("o1"),
                answer("o1"),
                answer("o1"),
            ]);
            let voice = NpcVoice::new(Arc::new(teller(
                story.address,
                0,
                Duration::from_secs(5),
                &cache("recorded"),
            )))
            .with_approach_model(jev(&judge));
            let table = Table::start(current.clone(), loaded, voice);
            let (_, target) = stall(&current);
            let result = table
                .service
                .interaction_call(
                    [1; 32],
                    &request(&current, "I look closely at the glazes on the bowls."),
                    &target,
                    "examine",
                    None,
                    None,
                )
                .await
                .unwrap();
            let DialogueReply::Raw(response, None) = result else {
                panic!("narrator reply")
            };
            assert_eq!(response.npc_reply, LOOK);
            assert_ne!(table.service.updates.borrow().basis(), current.basis());
            let committed = table.service.updates.borrow().clone();
            let before_replay = table.service.table_log.lines().len();
            let replay = table
                .service
                .submit_line(
                    [1; 32],
                    request(&current, "I look closely at the glazes on the bowls."),
                )
                .await
                .unwrap();
            let DialogueReply::Raw(replay, None) = replay else {
                panic!("retained narration")
            };
            assert_eq!(replay.npc_reply, LOOK);
            assert_eq!(table.service.updates.borrow().basis(), committed.basis());
            assert_eq!(table.service.table_log.lines().len(), before_replay);
            assert_eq!(story.connections(), 1);
            assert_eq!(
                judge.connections(),
                5,
                "one relevance decision is retained across replay"
            );
            let prompt = story.bodies()[0]["input"].as_str().unwrap().to_owned();
            assert!(prompt.contains("glazes on the bowls") && prompt.contains("glossy blue glaze"));
            assert!(prompt.contains("Sella Voss") && prompt.contains("visible_picture"));
            assert!(
                table
                    .service
                    .table_log
                    .lines()
                    .iter()
                    .any(|line| line.text == LOOK)
            );
            assert!(
                !table
                    .service
                    .table_log
                    .lines()
                    .iter()
                    .any(|line| line.text.contains("Nothing shows who owns"))
            );
        })
    });
}

#[test]
fn hs59_scene_examine_keeps_a_recorded_owner_separate_from_the_attendant() {
    with_market(|current, _| {
        run(async {
            let (mut model, target) = stall(&current);
            model
                .things
                .iter_mut()
                .find(|thing| thing.id == target)
                .unwrap()
                .owner_id = None;
            let owner_id = model
                .people
                .iter()
                .find(|person| person.name == "Sella Voss")
                .unwrap()
                .id
                .clone();
            let thing = model
                .things
                .iter()
                .find(|thing| thing.id == target)
                .unwrap();
            let unknown = scene_examine_facts(&model, thing, "Whose stall is this?");
            assert!(unknown["owner"].is_null());
            assert_eq!(unknown["attendant"]["name"], "Sella Voss");
            model
                .things
                .iter_mut()
                .find(|thing| thing.id == target)
                .unwrap()
                .owner_id = Some(owner_id);
            let thing = model
                .things
                .iter()
                .find(|thing| thing.id == target)
                .unwrap();
            let facts = scene_examine_facts(&model, thing, "Whose stall is this?");
            assert_eq!(facts["owner"]["name"], "Sella Voss");
            let line = "Sella Voss owns the pottery stall.";
            let endpoint = FakeEndpoint::start(vec![Reply::Output(json!({"narration":line}))]);
            let story = teller(
                endpoint.address,
                0,
                Duration::from_secs(5),
                &cache("known-owner"),
            );
            assert_eq!(narrate_scene_examine(&story, &facts).await.unwrap(), line);
            assert!(
                endpoint.bodies()[0]["input"]
                    .as_str()
                    .unwrap()
                    .contains("owner")
            );
        })
    });
}

#[test]
fn hs59_scene_examine_storyteller_off_retains_the_template_path() {
    with_market(|current, loaded| {
        run(async {
            let judge = FakeEndpoint::start(vec![answer("o2")]);
            let voice =
                NpcVoice::new(Arc::new(Storyteller::disabled())).with_approach_model(jev(&judge));
            let table = Table::start(current.clone(), loaded, voice);
            let (_, target) = stall(&current);
            let reply = table
                .service
                .examine_scene_thing_call([1; 32], &request(&current, WORDS), &target)
                .await
                .unwrap();
            assert!(
                reply.is_none(),
                "caller must use the existing journalled examine template"
            );
            let (model, target) = stall(&current);
            assert!(
                living_scene::examine_fallback(
                    &model,
                    model
                        .things
                        .iter()
                        .find(|thing| thing.id == target)
                        .unwrap(),
                    "Brynn"
                )
                .contains("A low table displays clay cups and bowls.")
            );
            assert!(table.service.table_log.lines().is_empty());
            let command = crate::gameplay::beat_script_tests::input(
                &current,
                native::first(),
                203,
                "script-choice",
                vec![
                    (
                        model::label("scene-target").unwrap(),
                        model::label(&target).unwrap(),
                    ),
                    (
                        model::label("scene-option").unwrap(),
                        model::label("examine").unwrap(),
                    ),
                ],
            );
            let examined = journey::stage_with_supplier(&current, &command, &mut |_| {
                panic!("visible inspection never rolls")
            })
            .unwrap();
            let line = living_scene::line(examined.state(), operation(203)).unwrap();
            assert!(line.contains("A low table displays clay cups and bowls."));
            assert!(
                line.contains("Sella Voss owns"),
                "the generated stall has a recorded keeper-owner"
            );
        })
    });
}

#[tokio::test]
async fn hs59_scene_examine_approach_uses_jev_meaning_instead_of_player_word_matching() {
    let facts = json!({"hero_words":"I wander over to it.","thing":{"portable":false}});
    let judge = FakeEndpoint::start(vec![answer("o2"), answer("o1"), answer("o4")]);
    assert_eq!(
        judge_examine_approach(&jev(&judge), &facts).await,
        ExamineFraming::Look
    );
    let facts = json!({"hero_words":"I cross the square to inspect those bowls.","thing":{"portable":false}});
    assert_eq!(
        judge_examine_approach(&jev(&judge), &facts).await,
        ExamineFraming::Approach
    );
    let facts = json!({"hero_words":"I cross to the stall and lift a bowl to look at it.","thing":{"portable":true}});
    assert_eq!(
        judge_examine_approach(&jev(&judge), &facts).await,
        ExamineFraming::ApproachInHands
    );
    assert!(judge.bodies()[0].to_string().contains("approach_examine"));
}

#[test]
fn hs59_scene_examine_backfills_missing_visual_detail_before_a_later_vendor_answer() {
    with_market(|current, loaded| {
        run(async {
            let fact =
                "Sella Voss's bowls have a faint crackled texture beneath their glossy blue glaze.";
            let look = "A faint crackled texture shows beneath the bowls' glossy blue glaze.";
            let story = FakeEndpoint::start(vec![
                Reply::Output(
                    json!({"questions":json!(["What texture is beneath Sella Voss's bowls' blue glaze?"]).to_string()}),
                ),
                Reply::Output(json!({"fact":json!({"name":"glaze texture","content":fact,
                "answer":"My bowls have a faint crackled texture beneath their glossy blue glaze."}).to_string()})),
                Reply::Output(json!({"narration":look})),
            ]);
            let judge = FakeEndpoint::start(vec![
                answer("o2"),
                answer("o2"),
                answer("o1"),
                answer("o1"),
                answer("o1"),
                answer("o1"),
                answer("o1"),
                answer("o2"),
            ]);
            let lore = Arc::new(crate::gameplay::npc_reply::local_lore::LocalLore::in_memory());
            let voice = NpcVoice::new(Arc::new(teller(
                story.address,
                0,
                Duration::from_secs(5),
                &cache("backfill"),
            )))
            .with_approach_model(jev(&judge))
            .with_local_lore(lore.clone());
            let table = Table::start(current.clone(), loaded, voice);
            let (model, target) = stall(&current);
            let result = table
                .service
                .interaction_call(
                    [1; 32],
                    &request(
                        &current,
                        "I look at the texture beneath the bowls' glossy blue glaze.",
                    ),
                    &target,
                    "examine",
                    None,
                    None,
                )
                .await
                .unwrap();
            let DialogueReply::Raw(response, None) = result else {
                panic!("inspection answer")
            };
            assert_eq!(response.npc_reply, look);
            assert!(
                lore.development_facts(None)
                    .iter()
                    .flat_map(|(_, facts)| facts)
                    .any(|kept| kept.content == fact)
            );
            let thing = model
                .things
                .iter()
                .find(|thing| thing.id == target)
                .unwrap();
            let attendant = living_scene::examine_attendant(&model, thing).unwrap();
            let current = table.service.updates.borrow().clone();
            let projection =
                wire::project_journey(&current, LocalDemoRole::Player, native::first()).unwrap();
            let mut context = NpcReplyContext::for_target(
                &projection.view,
                native::first(),
                "Sella, what texture do these glazes have?",
                &pack::with_content(StoryFacts::from_pack),
                &table.service.social,
                &format!("{:?}", projection.scene),
                Some(&attendant.id),
            )
            .unwrap();
            context.with_engine_context(current.state());
            context.watch_publication(table.service.updates.clone());
            let prepared = table
                .service
                .npc_voice
                .prepare_fact_reply(&context)
                .await
                .unwrap();
            assert_eq!(prepared["narrative_fact_decision"]["fact"]["content"], fact);
            assert_eq!(
                story.connections(),
                3,
                "the vendor reuses the recorded detail"
            );
            let prompt = story.bodies()[2]["input"].as_str().unwrap().to_owned();
            assert!(
                prompt.contains("narrative_fact_decision") && prompt.contains("crackled texture")
            );
        })
    });
}

fn spot_reply() -> Value {
    let (mut place, mut people, things) = market_slots();
    place["name"] = json!("pottery stall");
    place["speaker"] = json!("");
    place["greeting"] = json!("");
    place["exits"] = json!([]);
    place["landmarks"] = json!([]);
    place["market"] = json!(false);
    place["stalls"] = json!([]);
    place["mentioned_stalls"] = json!([]);
    place["narration"] = json!(
        "You gather at the pottery stall beside Sella Voss; Market Street stays in view behind you."
    );
    // The spot builder retains the original keeper and wares. The other fixture passers-by
    // have new identities, so a generated destination cannot duplicate an existing person.
    people[1]["name"] = json!("Mira Fen");
    people[2]["name"] = json!("Tess Brook");
    for ware in place["wares"].as_array_mut().unwrap() {
        if ware["seller"] == "Wenna" {
            ware["seller"] = json!("Mira Fen");
        }
        if ware["seller"] == "Yara" {
            ware["seller"] = json!("Tess Brook");
        }
    }
    place_answer(&place, &people, &things)
}

#[test]
fn hs59_scene_examine_rejected_narration_never_becomes_vendor_speech() {
    with_market(|current, loaded| {
        run(async {
            let story = FakeEndpoint::start(vec![Reply::Output(
                json!({"narration":"Sella Voss says the glaze grants you luck."}),
            )]);
            let judge = FakeEndpoint::start(vec![
                answer("o2"),
                answer("o1"),
                answer("o1"),
                answer("o1"),
                answer("o2"),
            ]);
            let voice = NpcVoice::new(Arc::new(teller(
                story.address,
                0,
                Duration::from_secs(5),
                &cache("rejected"),
            )))
            .with_approach_model(jev(&judge));
            let table = Table::start(current.clone(), loaded, voice);
            let (_, target) = stall(&current);
            let result = table
                .service
                .interaction_call(
                    [1; 32],
                    &request(&current, WORDS),
                    &target,
                    "examine",
                    None,
                    None,
                )
                .await
                .unwrap();
            let DialogueReply::Raw(response, None) = result else {
                panic!("narrator refusal")
            };
            assert_eq!(
                response.npc_reply,
                "You cannot make out that detail just now."
            );
            assert_eq!(
                story.connections(),
                1,
                "no NPC repair call for narrator speech"
            );
            assert!(
                !table
                    .service
                    .table_log
                    .lines()
                    .iter()
                    .any(|line| line.text.contains("grants you luck"))
            );
        })
    });
}

#[test]
fn hs59_scene_examine_unavailable_hero_never_reaches_a_model() {
    with_market(|_, loaded| {
        run(async {
            let current = native::prepared();
            let story = FakeEndpoint::start(vec![]);
            let judge = FakeEndpoint::start(vec![]);
            let voice = NpcVoice::new(Arc::new(teller(
                story.address,
                0,
                Duration::from_secs(5),
                &cache("unavailable"),
            )))
            .with_approach_model(jev(&judge));
            let table = Table::start(current.clone(), loaded, voice);
            let result = table
                .service
                .examine_scene_thing_call([1; 32], &request(&current, WORDS), "pottery-stall")
                .await;
            assert!(result.is_err());
            assert_eq!(judge.connections(), 0);
            assert_eq!(story.connections(), 0);
            assert!(table.service.table_log.lines().is_empty());
        })
    });
}

#[test]
fn hs59_scene_examine_resumes_after_the_move_committed_without_moving_twice() {
    with_market(|current, loaded| {
        run(async {
            let story = FakeEndpoint::start(
                [
                    vec![Reply::Output(spot_reply())],
                    crate::gameplay::place_creation::tests::persona_replies(2),
                    vec![Reply::Output(json!({"narration":LOOK}))],
                ]
                .into_iter()
                .flatten()
                .collect(),
            );
            let judge = FakeEndpoint::start((0..6).map(|_| answer("o1")).collect());
            let voice = NpcVoice::new(Arc::new(teller(
                story.address,
                0,
                Duration::from_secs(5),
                &cache("resumed"),
            )))
            .with_approach_model(jev(&judge));
            let table = Table::start(current.clone(), loaded, voice);
            let (_, target) = stall(&current);
            let original = request(&current, WORDS);
            table
                .service
                .move_party_toward(
                    [1; 32],
                    operation(202),
                    crate::gameplay::place_move::MoveTarget::Spot(place::LandmarkDraft {
                        name: "pottery stall".into(),
                        direction: "A low table of bowls tended by Sella Voss.".into(),
                    }),
                    WORDS,
                )
                .await
                .unwrap_or_else(|_| panic!("first move commits"));
            let after_move = table.service.updates.borrow().clone();
            let (_, scoped_target) = stall(&after_move);
            assert_ne!(
                scoped_target, target,
                "the arrived spot scopes its thing IDs"
            );
            let moves = place::entries(after_move.state()).unwrap().len();
            let result = table
                .service
                .interaction_call([1; 32], &original, &scoped_target, "examine", None, None)
                .await
                .unwrap();
            let DialogueReply::Raw(response, None) = result else {
                panic!("resumed look")
            };
            assert!(response.npc_reply.contains("glossy blue glaze"));
            assert_eq!(
                place::entries(table.service.updates.borrow().state())
                    .unwrap()
                    .len(),
                moves
            );
            assert_eq!(
                story.connections(),
                4,
                "the destination is not generated again"
            );
            assert_eq!(
                judge.connections(),
                6,
                "the committed approach is not judged again; relevance is judged once"
            );
        })
    });
}

#[test]
fn hs59_scene_examine_compound_approach_commits_the_spot_and_requests_its_plate() {
    with_market(|current, loaded| {
        run(async {
            let story = FakeEndpoint::start(
                [
                    vec![
                        Reply::Output(json!({"narration":LOOK})),
                        Reply::Output(spot_reply()),
                    ],
                    crate::gameplay::place_creation::tests::persona_replies(2),
                ]
                .into_iter()
                .flatten()
                .collect(),
            );
            let judge = FakeEndpoint::start((0..7).map(|_| answer("o1")).collect());
            let voice = NpcVoice::new(Arc::new(teller(
                story.address,
                0,
                Duration::from_secs(5),
                &cache("approach"),
            )))
            .with_approach_model(jev(&judge));
            let mut table = Table::start(current.clone(), loaded, voice);
            let images = crate::gameplay::imagery::tests::DirectionFixture::photographic(&cache(
                "approach-images",
            ));
            table.service.living = crate::gameplay::living_scenes::LivingScenes::start(
                images.imagery.clone(),
                crate::gameplay::imagery::ReferenceStore::new(cache("references")),
                table.service.social.clone(),
                ContentRating::Restrained,
                crate::gameplay::living_scenes::Timing::from_interval(Duration::from_secs(30)),
                true,
                table.service.updates.clone(),
            );
            let mut wake = table.service.living.subscribe().unwrap();
            // A real display has already registered the wide market before the hero approaches.
            let old_view =
                wire::journey_view(&current, LocalDemoRole::Display, native::first()).unwrap();
            table.service.living.observe(&old_view);
            let (_, target) = stall(&current);
            let result = table
                .service
                .interaction_call(
                    [1; 32],
                    &request(&current, WORDS),
                    &target,
                    "examine",
                    None,
                    None,
                )
                .await
                .unwrap();
            let DialogueReply::Raw(response, None) = result else {
                panic!("approach and look")
            };
            let after = table.service.updates.borrow().clone();
            assert_ne!(after.basis().revision, current.basis().revision);
            let arrival = runtime::scene(after.state()).unwrap();
            assert_eq!(arrival.title, "pottery stall");
            assert!(response.npc_reply.contains("glossy blue glaze"));
            let snapshot = living_scene::snapshot(after.state(), &arrival)
                .unwrap()
                .unwrap();
            assert!(
                snapshot
                    .model
                    .people
                    .iter()
                    .any(|person| person.name == "Sella Voss")
            );
            assert!(
                snapshot
                    .model
                    .offers
                    .iter()
                    .any(|ware| ware.description.contains("glossy blue glaze"))
            );
            let creation = story.bodies()[1]["input"].as_str().unwrap().to_owned();
            assert!(creation.contains("local_spot") && creation.contains("Sella Voss"));
            let framed = snapshot
                .model
                .things
                .iter()
                .find(|thing| thing.name == "pottery stall")
                .unwrap();
            let expected = format!("plate:{}", arrival.setting.as_ref().unwrap().place_id);
            let view = wire::journey_view(&after, LocalDemoRole::Display, native::first()).unwrap();
            let (image_key, image_bytes) = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    table.service.living.observe(&view);
                    if let Some(edit) = table
                        .service
                        .living
                        .list_resources(Some(crate::gameplay::resources::ResourceKind::PlateEdit))
                        .into_iter()
                        .find(|edit| edit.subject == expected && edit.state == "ready")
                        && let crate::gameplay::imagery::Lookup::Image { bytes, .. } =
                            images.imagery.image(&edit.handle).await
                    {
                        break (edit.handle, bytes);
                    }
                    wake.changed().await.unwrap();
                }
            })
            .await
            .expect("the committed stall and vendor must reach the plate-edit provider");
            assert!(!image_bytes.is_empty());
            image::load_from_memory(&image_bytes).expect("the framed stall image must decode");
            std::fs::write(
                cache("plate-evidence").with_extension("json"),
                json!({
                    "stall_id":framed.id,"vendor":"Sella Voss","image_key":image_key,
                    "plate_edit_subject":expected,"state":"ready",
                    "completed_image_bytes":image_bytes.len(),
                })
                .to_string(),
            )
            .unwrap();
            assert!(images.calls() > 0);
            assert_eq!(
                response.revision,
                Some(wire::revision(after.basis().revision))
            );
        })
    });
}

fn cup_target(model: &crate::gameplay::adventure::scene_model::SceneModel) -> (String, usize) {
    let visible_things = model
        .things
        .iter()
        .filter(|thing| thing.in_scene && thing.publicly_visible)
        .count();
    let (index, offer) = model
        .offers
        .iter()
        .filter(|offer| model.offer_available(&offer.id))
        .enumerate()
        .find(|(_, offer)| offer.name == "cup")
        .unwrap();
    (offer.canonical_item_id(), visible_things + index)
}

#[test]
fn hs59_examine_target_jev_selects_the_cup_instead_of_its_named_vendor() {
    with_market(|current, loaded| {
        run(async {
            let (model, _) = stall(&current);
            let (cup, index) = cup_target(&model);
            let vendor_index = model
                .things
                .iter()
                .filter(|thing| thing.in_scene && thing.publicly_visible)
                .count()
                + model
                    .offers
                    .iter()
                    .filter(|offer| model.offer_available(&offer.id))
                    .count()
                + model
                    .people
                    .iter()
                    .position(|person| person.name == "Sella Voss")
                    .unwrap();
            let judge = FakeEndpoint::start(vec![
                answer("o12"),
                answer(&format!("o{}", index + 1)),
                answer(&format!("o{}", vendor_index + 1)),
            ]);
            let voice =
                NpcVoice::new(Arc::new(Storyteller::disabled())).with_approach_model(jev(&judge));
            let table = Table::start(current.clone(), loaded, voice);
            let scene = runtime::scene(current.state()).unwrap();
            let words =
                "I pick up one of Sella Voss's clay cups and take a closer look at the glaze.";
            let interaction = table
                .service
                .judge_scene_interaction(words, &scene, &[], None)
                .await
                .unwrap();
            assert_eq!(interaction, (cup, "examine"));
            assert_eq!(
                interaction_gate(
                    false,
                    super::super::approach_model::LineIntent::Examine,
                    true,
                    Some("examine"),
                    interaction_names_person(&scene, &interaction.0)
                ),
                InteractionGate::Keep
            );
            let (_, jev) = table.service.npc_voice.hero_identity_models();
            let person = judge_examine_target(jev, &model, "I study Sella Voss's face.")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                person,
                model
                    .people
                    .iter()
                    .find(|person| person.name == "Sella Voss")
                    .unwrap()
                    .id
            );
            let prompt = judge.bodies()[1].to_string();
            assert!(
                prompt.contains(words)
                    && prompt.contains("Ware cup")
                    && prompt.contains("Person Sella Voss")
            );
        })
    });
}

#[test]
fn hs59_examine_target_none_does_not_fall_back_to_the_named_vendor() {
    with_market(|current, loaded| {
        run(async {
            let (model, _) = stall(&current);
            let none_index = model
                .things
                .iter()
                .filter(|thing| thing.in_scene && thing.publicly_visible)
                .count()
                + model
                    .offers
                    .iter()
                    .filter(|offer| model.offer_available(&offer.id))
                    .count()
                + model.people.len()
                + model.ways_out.len()
                + model.services.len();
            let judge =
                FakeEndpoint::start(vec![answer("o12"), answer(&format!("o{}", none_index + 1))]);
            let voice =
                NpcVoice::new(Arc::new(Storyteller::disabled())).with_approach_model(jev(&judge));
            let table = Table::start(current.clone(), loaded, voice);
            let scene = runtime::scene(current.state()).unwrap();
            assert!(
                table
                    .service
                    .judge_scene_interaction(
                        "I examine Sella Voss's unlisted vase.",
                        &scene,
                        &[],
                        None
                    )
                    .await
                    .is_none()
            );
            assert_eq!(table.service.updates.borrow().basis(), current.basis());
        })
    });
}

#[test]
fn hs59_pick_up_examine_narrates_the_ware_and_queues_its_public_close_up_without_taking_it() {
    with_market(|current, loaded| {
        run(async {
            let words =
                "I pick up one of Sella Voss's clay cups and take a closer look at the glaze.";
            let look = "You turn the clay cup under the light; its glossy blue glaze pools dark around the rim. It remains Sella Voss's ware.";
            let story = FakeEndpoint::start(vec![Reply::Output(json!({"narration":look}))]);
            let judge = FakeEndpoint::start(vec![
                answer("o3"),
                answer("o1"),
                answer("o1"),
                answer("o1"),
                answer("o1"),
            ]);
            let voice = NpcVoice::new(Arc::new(teller(
                story.address,
                0,
                Duration::from_secs(5),
                &cache("cup-look"),
            )))
            .with_approach_model(jev(&judge));
            let mut table = Table::start(current.clone(), loaded, voice);
            let images = crate::gameplay::imagery::tests::DirectionFixture::photographic(&cache(
                "cup-images",
            ));
            table.service.living = crate::gameplay::living_scenes::LivingScenes::start(
                images.imagery.clone(),
                crate::gameplay::imagery::ReferenceStore::new(cache("cup-references")),
                table.service.social.clone(),
                ContentRating::Restrained,
                crate::gameplay::living_scenes::Timing::from_interval(Duration::from_secs(30)),
                true,
                table.service.updates.clone(),
            );
            let mut wake = table.service.living.subscribe().unwrap();
            let old_view =
                wire::journey_view(&current, LocalDemoRole::Display, native::first()).unwrap();
            table.service.living.observe(&old_view);
            let (before, _) = stall(&current);
            let (cup, _) = cup_target(&before);
            let result = table
                .service
                .interaction_call(
                    [1; 32],
                    &request(&current, words),
                    &cup,
                    "examine",
                    None,
                    None,
                )
                .await
                .unwrap();
            let DialogueReply::Raw(response, None) = result else {
                panic!("ware narration")
            };
            assert_eq!(response.npc_reply, look);
            let after = table.service.updates.borrow().clone();
            let (model, _) = stall(&after);
            assert_eq!(
                model, before,
                "handling cannot transfer custody or consume stock"
            );
            assert!(model.offer_available(&cup));
            assert!(!model.things.iter().any(|thing| thing.id == cup));
            assert_eq!(
                place::entries(after.state()).unwrap().len(),
                place::entries(current.state()).unwrap().len()
            );
            let prompt = story.bodies()[0]["input"].as_str().unwrap().to_owned();
            assert!(
                prompt.contains(words)
                    && prompt.contains("glossy blue glaze")
                    && prompt.contains(&cup)
            );
            let mut view =
                wire::journey_view(&after, LocalDemoRole::Display, native::first()).unwrap();
            table.service.living.apply(&mut view);
            let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
                panic!("display")
            };
            let card = display
                .scene
                .unwrap()
                .item
                .expect("next sync exposes the inspected public ware");
            assert_eq!(card.thing_id, cup);
            assert_ne!(card.holder_id, "Brynn");
            assert!(card.detail.contains("glossy blue glaze"));
            let image_bytes = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Some(item) = table
                        .service
                        .living
                        .list_resources(Some(crate::gameplay::resources::ResourceKind::ItemCloseUp))
                        .into_iter()
                        .find(|item| {
                            item.subject.starts_with(&format!("item:{cup}:"))
                                && item.state == "ready"
                        })
                        && let crate::gameplay::imagery::Lookup::Image { bytes, .. } =
                            images.imagery.image(&item.handle).await
                    {
                        break bytes;
                    }
                    wake.changed().await.unwrap();
                }
            })
            .await
            .expect("the inspected cup close-up must complete through the image provider");
            assert!(!image_bytes.is_empty());
            image::load_from_memory(&image_bytes).unwrap();
        })
    });
}

#[test]
fn hs59_ware_storyteller_off_retains_the_offer_template_without_taking_stock() {
    with_market(|current, loaded| {
        run(async {
            let judge = FakeEndpoint::start(vec![answer("o2")]);
            let voice =
                NpcVoice::new(Arc::new(Storyteller::disabled())).with_approach_model(jev(&judge));
            let table = Table::start(current.clone(), loaded, voice);
            let (before, _) = stall(&current);
            let (cup, _) = cup_target(&before);
            assert!(
                table
                    .service
                    .examine_scene_thing_call(
                        [1; 32],
                        &request(
                            &current,
                            "I lift Sella's cup for a closer look at its glaze."
                        ),
                        &cup
                    )
                    .await
                    .unwrap()
                    .is_none()
            );
            let command = crate::gameplay::beat_script_tests::input(
                &current,
                native::first(),
                203,
                "script-choice",
                vec![
                    (
                        model::label("scene-target").unwrap(),
                        model::label(&cup).unwrap(),
                    ),
                    (
                        model::label("scene-option").unwrap(),
                        model::label("examine").unwrap(),
                    ),
                ],
            );
            let examined = journey::stage_with_supplier(&current, &command, &mut |_| {
                panic!("inspection never rolls")
            })
            .unwrap();
            let offer = before.offer(&cup).unwrap();
            assert_eq!(
                living_scene::line(examined.state(), operation(203)).unwrap(),
                format!("{} It costs {} CP.", offer.description, offer.cost_cp)
            );
            assert_eq!(stall(&examined).0, before);
        })
    });
}

#[test]
fn hs59_examine_target_preserves_public_exit_and_service_choices() {
    with_market(|current, _| {
        run(async {
            let (model, _) = stall(&current);
            let first_exit = model
                .things
                .iter()
                .filter(|thing| thing.in_scene && thing.publicly_visible)
                .count()
                + model
                    .offers
                    .iter()
                    .filter(|offer| model.offer_available(&offer.id))
                    .count()
                + model.people.len();
            let first_service = first_exit + model.ways_out.len();
            let way = model.ways_out.first().expect("public exit fixture");
            let service = model.services.first().expect("public service fixture");
            let judge = FakeEndpoint::start(vec![
                answer(&format!("o{}", first_exit + 1)),
                answer(&format!("o{}", first_service + 1)),
            ]);
            assert_eq!(
                judge_examine_target(&jev(&judge), &model, "I inspect the way back out.")
                    .await
                    .unwrap(),
                Some(way.id.clone())
            );
            assert_eq!(
                judge_examine_target(&jev(&judge), &model, "I look over the displayed service.")
                    .await
                    .unwrap(),
                Some(service.id.clone())
            );
            let choices = judge.bodies()[0].to_string();
            assert!(choices.contains("Exit ") && choices.contains("Service "));
        })
    });
}

fn roll_item_inspection(current: &Checkpoint, target: &str, step: u8, die: u32) -> Checkpoint {
    let offered = runtime::view_offers(current, native::first())
        .unwrap()
        .into_iter()
        .find(|(kind, _, _)| *kind == rpc::GameplayActionKind::ExamineHarborSeal)
        .unwrap()
        .2;
    let selected = runtime::submitted_id(&offered).unwrap();
    let command = crate::gameplay::beat_script_tests::input(
        current,
        native::first(),
        step,
        "script-choice",
        vec![
            (
                model::label(runtime::CHOICE_GROUP).unwrap(),
                model::label(selected).unwrap(),
            ),
            (
                model::label("attempt-skill").unwrap(),
                model::label("investigation").unwrap(),
            ),
            (
                model::label("attempt-thing").unwrap(),
                model::label(&format!("thing-{target}")).unwrap(),
            ),
        ],
    );
    journey::stage_with_supplier(current, &command, &mut |_| Ok(die)).unwrap_or_else(|error| {
        panic!(
            "{error:?}; refusal={:?}",
            crate::gameplay::refusal_detail::take()
        )
    })
}

#[test]
fn hs71_item_decision_replays_once_and_only_a_successful_bound_roll_publishes_its_hidden_detail() {
    with_market(|current, loaded| {
        let (_, target) = stall(&current);
        item_second_look_fixture(current, loaded, target);
    });
}

#[test]
fn hs71_authored_place_service_completion_schedules_only_a_successful_second_focus() {
    let generated = shop::generate(71, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded.clone(), || {
        let current = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let source = runtime::scene(current.state()).unwrap();
        let target = source
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .things[0]
            .id
            .clone();
        item_second_look_fixture(current, loaded, target);
    });
}

fn item_second_look_fixture(current: Checkpoint, loaded: pack::LoadedPack, target: String) {
    use crate::gameplay::investigation::{ItemConnection, ItemHiddenDetail, ItemStoryDecision};
    run(async {
        let source = runtime::scene(current.state()).unwrap();
        let model = living_scene::snapshot(current.state(), &source)
            .unwrap()
            .unwrap()
            .model;
        let public = "A patch of old blue wax clings to the edge.";
        let hidden = "A maker's gull mark is scratched beneath the base.";
        let story = ItemStoryDecision {
            item: target.clone(),
            connection: ItemConnection::Clue,
            surface: Some(public.into()),
            hidden: Some(ItemHiddenDetail {
                detail: hidden.into(),
                skill: pack::FindingSkill::Investigation,
                dc: 15,
            }),
            authored_finding: None,
            authored_source: None,
        };
        let thing = model
            .things
            .iter()
            .find(|thing| thing.id == target)
            .unwrap();
        let mut current = living_scene::commit_examine_with_story(
            &current,
            native::first(),
            operation(220),
            living_scene::ExamineNarration {
                target: &target,
                words: "I take a look.",
                fingerprint: "hs71-first-look",
                text: &thing.detail,
            },
            Some(story.clone()),
        )
        .unwrap();
        let first_scene = runtime::scene(current.state()).unwrap();
        let first_model = living_scene::snapshot(current.state(), &first_scene)
            .unwrap()
            .unwrap()
            .model;
        let focus_thing = first_model
            .things
            .iter()
            .find(|thing| thing.id == target)
            .unwrap();
        assert!(focus_thing.detail.contains(public));
        assert!(!focus_thing.detail.contains(hidden));
        assert!(!first_model.public_contents().unwrap().contains(hidden));
        assert_eq!(
            living_scene::item_story_for(current.state(), "another-place", &target).unwrap(),
            Some(story.clone())
        );
        let first_look = current.clone();
        for (step, die, succeeds) in [(221, 1, false), (223, 20, true)] {
            current = first_look.clone();
            current = roll_item_inspection(&current, &target, step, die);
            let check = journey::accepted(current.state().decisions.last().unwrap())
                .unwrap()
                .check
                .unwrap();
            assert_eq!(check.succeeded, succeeds);
            assert_eq!(check.difficulty_class, 15);
            let mut table = Table::start(current.clone(), loaded.clone(), NpcVoice::silent());
            let images = crate::gameplay::imagery::tests::DirectionFixture::photographic(&cache(
                &format!("second-look-{step}"),
            ));
            table.service.living = crate::gameplay::living_scenes::LivingScenes::start(
                images.imagery.clone(),
                crate::gameplay::imagery::ReferenceStore::new(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../assets/ui/generated"),
                ),
                table.service.social.clone(),
                ContentRating::Restrained,
                crate::gameplay::living_scenes::Timing::from_interval(Duration::from_secs(30)),
                true,
                table.service.updates.clone(),
            );
            let mut wake = table.service.living.subscribe().unwrap();
            let initial_view =
                wire::journey_view(&current, LocalDemoRole::Display, native::first()).unwrap();
            table.service.living.observe(&initial_view);
            tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let mut view = initial_view.clone();
                    table.service.living.apply(&mut view);
                    if let Some(rpc::view_message::Audience::Display(display)) = view.audience
                        && display
                            .scene
                            .as_ref()
                            .is_some_and(|scene| scene.plate_state == rpc::PlateState::Ready as i32)
                    {
                        break;
                    }
                    wake.changed().await.unwrap();
                }
            })
            .await
            .expect("a real fake-provider base plate is presented before the second look");
            let action = rpc::SubmitActionRequest {
                operation_id: Some(rpc::OperationId {
                    value: Some(operation(step).as_bytes().to_vec()),
                }),
                action_kind: rpc::GameplayActionKind::ExamineHarborSeal as i32,
                ..Default::default()
            };
            let response = rpc::SubmitActionResponse {
                outcome: Some(rpc::submit_action_response::Outcome::CommittedDecision(
                    Box::new(rpc::DecisionReceipt {
                        outcome: Some(rpc::decision_receipt::Outcome::Accepted(Box::new(
                            journey::accepted(current.state().decisions.last().unwrap()).unwrap(),
                        ))),
                        ..Default::default()
                    }),
                )),
                ..Default::default()
            };
            let record = table
                .service
                .complete_observation_for_question(
                    [1; 32],
                    &action,
                    &response,
                    "I search beneath the display for its maker's mark.",
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(record.text.contains(hidden), succeeds);
            assert!(!record.unavailable);
            current = table.service.updates.borrow().clone();
            if succeeds {
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let rows = table.service.living.list_resources(Some(
                            crate::gameplay::resources::ResourceKind::PlateEdit,
                        ));
                        if let Some(focus) = rows
                            .iter()
                            .find(|row| row.subject.starts_with(&format!("item-focus:{target}:")))
                            && focus.state == "ready"
                        {
                            break;
                        }
                        wake.changed().await.unwrap();
                    }
                })
                .await
                .expect("successful observation commits and queues the second foreground focus");
                assert!(
                    images.prompt().contains(hidden),
                    "the second focus prompt shows the committed hidden detail"
                );
                assert_eq!(images.image_count(), 2);
                table
                    .service
                    .complete_observation_for_question([1; 32], &action, &response, "I look again.")
                    .await
                    .unwrap();
            }
            let focus_count = table
                .service
                .living
                .list_resources(Some(crate::gameplay::resources::ResourceKind::PlateEdit))
                .iter()
                .filter(|row| row.subject.starts_with(&format!("item-focus:{target}:")))
                .count();
            assert_eq!(
                focus_count,
                usize::from(succeeds),
                "failure queues no hidden focus and replay shares the same focus"
            );
            let scene = runtime::scene(current.state()).unwrap();
            assert_eq!(
                living_scene::released_item_detail(current.state(), &scene, &target)
                    .unwrap()
                    .is_some(),
                succeeds
            );
        }
        let after = runtime::scene(current.state()).unwrap();
        assert!(
            living_scene::snapshot(current.state(), &after)
                .unwrap()
                .unwrap()
                .model
                .things
                .iter()
                .find(|thing| thing.id == target)
                .unwrap()
                .detail
                .contains(hidden)
        );
        current = living_scene::commit_examine_with_story(
            &current,
            native::first(),
            operation(225),
            living_scene::ExamineNarration {
                target: &target,
                words: "I look again.",
                fingerprint: "hs71-repeat",
                text: public,
            },
            Some(story),
        )
        .unwrap();
        assert_eq!(
            living_scene::item_story_journal(current.state())
                .unwrap()
                .len(),
            1
        );
    });
}

#[test]
fn hs71_authored_clue_sources_survive_successful_reveal_repeat_and_a_different_place_projection() {
    use crate::gameplay::investigation;
    for clue_fact_only in [false, true] {
        let generated = shop::generate(71, RulesMode::Standard, ContentRating::default()).unwrap();
        let (mut loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
        let hidden = "A gull-shaped maker's mark is cut beneath the base.";
        for scene in loaded.pack.scenes.iter_mut().chain(
            loaded
                .pack
                .beat_script
                .iter_mut()
                .flat_map(|script| script.scenes.values_mut()),
        ) {
            if let Some(thing) = scene
                .setting
                .as_mut()
                .and_then(|setting| setting.model.as_mut())
                .and_then(|model| model.things.first_mut())
            {
                thing.closer_look = hidden.into();
                thing.skill = pack::FindingSkill::Investigation;
                thing.dc = 15;
                thing.finding_id = (!clue_fact_only).then(|| "hs71-authored-marker".into());
                thing.clue_fact_id = clue_fact_only.then(|| "secret.id".into());
                if !clue_fact_only {
                    scene.findings.push(pack::Finding {
                        id: "hs71-authored-marker".into(),
                        sentence: hidden.into(),
                        skill: pack::FindingSkill::Investigation,
                        dc: 15,
                        private: false,
                        consequence: None,
                    });
                }
            }
        }
        pack::with_test_pack(loaded, || {
            run(async {
                let opened = native::act(&native::prepared(), native::first(), 5, "begin-story");
                let source = runtime::scene(opened.state()).unwrap();
                let thing = source
                    .setting
                    .as_ref()
                    .unwrap()
                    .model
                    .as_ref()
                    .unwrap()
                    .things[0]
                    .clone();
                let story = investigation::decide_item_story(
                    &Storyteller::disabled(),
                    &crate::gameplay::approach_model::ApproachModel::default(),
                    opened.state(),
                    &source,
                    &thing,
                    json!({}),
                )
                .await
                .unwrap();
                let retained = story.authored_source.clone().unwrap();
                assert_eq!(retained.sentence, hidden);
                let first = living_scene::commit_examine_with_story(
                    &opened,
                    native::first(),
                    operation(230),
                    living_scene::ExamineNarration {
                        target: &thing.id,
                        words: "I take a look.",
                        fingerprint: "hs71-authored-look",
                        text: &thing.detail,
                    },
                    Some(story),
                )
                .unwrap();
                let rolled = roll_item_inspection(&first, &thing.id, 231, 20);
                let check = journey::accepted(rolled.state().decisions.last().unwrap())
                    .unwrap()
                    .check
                    .unwrap();
                let mut scene = runtime::scene(rolled.state()).unwrap();
                investigation::project_item_story_findings(rolled.state(), &mut scene).unwrap();
                let (ids, text, unavailable) = investigation::complete_finding(
                    &Storyteller::disabled(),
                    &crate::gameplay::approach_model::ApproachModel::default(),
                    &scene,
                    &check,
                    "Brynn",
                )
                .await;
                assert_eq!(ids, vec![retained.id.clone()]);
                assert!(text.contains(hidden));
                let op = journey::hex(operation(231).as_bytes());
                let receipt = living_scene::ObservationFinding {
                    id: format!("observation-{op}"),
                    operation: op,
                    actor: journey::hex(rolled.state().characters[0].entity.as_bytes()),
                    hero: "Brynn".into(),
                    audience: "shared".into(),
                    check: journey::hex(&check.encode_to_vec()),
                    target: format!("thing-{}", thing.id),
                    source_revision: check.source_revision.clone(),
                    source_epoch: rolled.basis().revision.epoch().get(),
                    source_sequence: rolled.basis().revision.sequence(),
                    source_hash: living_scene::observation_source_hash(&scene).unwrap(),
                    scene: scene.key,
                    selected_source_ids: ids,
                    text,
                    unavailable,
                };
                let after = living_scene::commit_observation_with_detail(
                    &rolled,
                    rolled.basis(),
                    receipt,
                    None,
                )
                .unwrap();
                let repeated = runtime::scene(after.state()).unwrap();
                assert_eq!(
                    living_scene::released_item_detail(after.state(), &repeated, &thing.id)
                        .unwrap()
                        .as_deref(),
                    Some(hidden)
                );
                assert!(
                    living_scene::snapshot(after.state(), &repeated)
                        .unwrap()
                        .unwrap()
                        .model
                        .things
                        .iter()
                        .find(|entry| entry.id == thing.id)
                        .unwrap()
                        .detail
                        .contains(hidden)
                );
                let mut destination = source.clone();
                destination.setting.as_mut().unwrap().place_id = "hs71-other-place".into();
                destination.findings.clear();
                assert_eq!(
                    living_scene::released_item_detail(after.state(), &destination, &thing.id)
                        .unwrap()
                        .as_deref(),
                    Some(hidden)
                );
                investigation::project_item_story_findings(after.state(), &mut destination)
                    .unwrap();
                assert!(
                    destination.findings.contains(&retained),
                    "destination restores the exact authored source"
                );
                assert!(
                    living_scene::snapshot(after.state(), &destination)
                        .unwrap()
                        .unwrap()
                        .model
                        .things
                        .iter()
                        .find(|entry| entry.id == thing.id)
                        .unwrap()
                        .detail
                        .contains(hidden)
                );
            })
        });
    }
}

#[test]
fn hs71_item_surface_capacity_is_checked_before_the_owner_can_commit_it() {
    use crate::gameplay::investigation::{ItemConnection, ItemStoryDecision};
    let generated = shop::generate(72, RulesMode::Standard, ContentRating::default()).unwrap();
    let (mut loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    for scene in loaded.pack.scenes.iter_mut().chain(
        loaded
            .pack
            .beat_script
            .iter_mut()
            .flat_map(|script| script.scenes.values_mut()),
    ) {
        if let Some(thing) = scene
            .setting
            .as_mut()
            .and_then(|setting| setting.model.as_mut())
            .and_then(|model| model.things.first_mut())
        {
            thing.finding_id = None;
            thing.clue_fact_id = None;
            thing.detail = format!("A {}. A {}.", "worn ".repeat(40), "smooth ".repeat(35));
        }
    }
    pack::with_test_pack(loaded, || {
        let current = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let scene = runtime::scene(current.state()).unwrap();
        let thing = &scene
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .things[0];
        assert!(
            living_scene::snapshot(current.state(), &scene)
                .unwrap()
                .is_some()
        );
        let story = ItemStoryDecision {item:thing.id.clone(),connection:ItemConnection::Flavor,
            surface:Some("A harmless decorative detail follows the old local fashion in its curves and weathered finish.".into()),
            hidden:None,authored_finding:None,authored_source:None};
        assert!(story.valid());
        assert!(
            thing.detail.chars().count() + story.surface.as_ref().unwrap().chars().count() + 1
                > 480
        );
        let result = living_scene::commit_examine_with_story(
            &current,
            native::first(),
            operation(240),
            living_scene::ExamineNarration {
                target: &thing.id,
                words: "I take a look.",
                fingerprint: "hs71-overflow",
                text: &thing.detail,
            },
            Some(story),
        );
        assert!(
            result.is_err(),
            "a candidate that would invalidate its public snapshot never reaches persistence"
        );
        assert!(
            living_scene::item_story_journal(current.state())
                .unwrap()
                .is_empty()
        );
    });
}
