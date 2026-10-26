//! Opening ingress tests use authenticated fake seats and the native staging boundaries.
use super::*;
use crate::gameplay::{
    approach_model::tests::{answer, model as jev},
    beat_script_runtime::{self as runtime, OpeningSkill},
    pack,
    social::SocialLedger,
    storyteller::{Storyteller, tests::FakeEndpoint},
    table_log::TableLog,
};
use df_model::checkpoint::Checkpoint;
use df_session::inbox::{AdmissionSequence, Reducer, bounded_inbox};
use tokio::sync::{Semaphore, watch};

struct TestOwner {
    current: Checkpoint,
    updates: watch::Sender<Checkpoint>,
}

impl TestOwner {
    fn member(&self, credential: &[u8; 32]) -> MemberId {
        let index = if credential == &[1; 32] {
            0
        } else if credential == &[2; 32] {
            1
        } else {
            panic!("unknown fake seat")
        };
        self.current.state().characters[index].owner
    }
}

impl Reducer<actor::Call> for TestOwner {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        match call {
            actor::Call::SpeechMember { credential, reply } => {
                reply.send(Ok(self.member(&credential))).unwrap();
            }
            actor::Call::Dialogue {
                credential,
                request,
                reply,
            } => {
                let member = self.member(&credential);
                let response = match request {
                    DialogueRequest::PendingMerchantQuote => DialogueReply::PendingMerchantQuote {
                        hero_id: journey::name(
                            self.current.state(),
                            journey::player_entity(member, &self.current).unwrap(),
                        )
                        .unwrap(),
                        quote: None,
                    },
                    DialogueRequest::PartyHeroes => DialogueReply::PartyHeroes(Vec::new()),
                    DialogueRequest::RawTarget { bytes, actor_id } => {
                        let request =
                            rpc::RawDialogueRequest::decode(bytes.bytes.as_slice()).unwrap();
                        assert_eq!(request.context, rpc::DialogueContext::Social as i32);
                        validate_opening_social_request(&request, self.current.basis()).unwrap();
                        let projection =
                            wire::project_journey(&self.current, LocalDemoRole::Player, member)
                                .unwrap();
                        let context = NpcReplyContext::for_target(
                            &projection.view,
                            member,
                            &request.text,
                            &pack::with_content(StoryFacts::from_pack),
                            &SocialLedger::none(),
                            "Opening",
                            Some(&actor_id),
                        )
                        .expect("the actual present NPC has a spoken reply context");
                        DialogueReply::Raw(
                            rpc::DialogueResponse {
                                npc_actor_id: context.actor_id.clone(),
                                npc_speaker: context.speaker.clone(),
                                input_id: request.input_id,
                                ..Default::default()
                            },
                            Some(Box::new(SpokenTurn {
                                context,
                                hero_trade: HeroTradeIntent::not_requested(),
                                cleanup: None,
                                typed_line_shown: false,
                            })),
                        )
                    }
                    _ => panic!("unexpected opening dialogue request"),
                };
                assert!(reply.send(Ok(response)).is_ok());
            }
            actor::Call::OpeningHelp {
                credential,
                request,
                expected_basis,
                beneficiary,
                skill,
                reply,
            } => {
                let result = actor::stage_opening_help_request(
                    &self.current,
                    self.member(&credential),
                    &request,
                    expected_basis,
                    beneficiary,
                    skill,
                );
                if let Ok(current) = &result {
                    self.current = current.clone();
                    self.updates.send_replace(current.clone());
                }
                assert!(reply.send(result).is_ok());
            }
            actor::Call::OpeningSocial {
                credential,
                proposal,
                reply,
            } => {
                let result = actor::stage_opening_social_proposal(
                    &self.current,
                    self.member(&credential),
                    &SocialLedger::none(),
                    &proposal,
                    &mut |_| Ok(1),
                );
                if let Ok(outcome) = &result {
                    self.current = outcome.checkpoint.clone();
                    self.updates.send_replace(self.current.clone());
                }
                assert!(reply.send(result).is_ok());
            }
            _ => panic!("unexpected opening owner call"),
        }
    }
}

struct Table {
    service: Service,
    owner: Option<std::thread::JoinHandle<()>>,
}

impl Table {
    fn new(current: Checkpoint, loaded: pack::LoadedPack, endpoint: &FakeEndpoint) -> Self {
        let (updates_in, updates) = watch::channel(current.clone());
        let (sender, inbox) = bounded_inbox();
        let owner = std::thread::spawn(move || {
            pack::with_test_pack(loaded, || {
                inbox
                    .run(&mut TestOwner {
                        current,
                        updates: updates_in,
                    })
                    .unwrap();
            })
        });
        Self {
            service: Service {
                look_ahead: Default::default(),
                portraits: crate::gameplay::creator_portraits::Portraits::disabled(),
                display_credential: [0x82; 32],
                layout: crate::gameplay::dev_control::layout::LayoutState::new(),
                actor: sender,
                updates,
                streams: Arc::new(Semaphore::new(2)),
                voice: crate::gameplay::voice::Voice::disabled(
                    crate::gameplay::startup_configuration::VoiceOff::Qualification,
                ),
                npc_voice: Arc::new(
                    NpcVoice::new(Arc::new(Storyteller::disabled()))
                        .with_approach_model(jev(endpoint)),
                ),
                table_log: TableLog::new(),
                social: SocialLedger::none(),
                living: crate::gameplay::living_scenes::LivingScenes::disabled(),
            },
            owner: Some(owner),
        }
    }

    fn line(&self, operation: u8, words: &str) -> rpc::RawDialogueRequest {
        request(&self.service.updates.borrow(), operation, words)
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        let stopped = self.service.actor.stop();
        let joined = self.owner.take().map(std::thread::JoinHandle::join);
        // A failing test is already unwinding: panicking again here (on a stopped actor or the
        // owner thread's own panic) would abort the whole test process instead of failing it.
        if !std::thread::panicking() {
            stopped.unwrap();
            assert!(
                joined.is_some_and(|joined| joined.is_ok()),
                "the owner thread panicked"
            );
        }
    }
}

fn request(current: &Checkpoint, operation: u8, words: &str) -> rpc::RawDialogueRequest {
    let basis = current.basis();
    rpc::RawDialogueRequest {
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        observed_revision: Some(wire::revision(basis.revision)),
        input_id: Some(rpc::OperationId {
            value: Some(vec![operation; 16]),
        }),
        finality: rpc::DialogueFinality::Final as i32,
        text: words.to_owned(),
        ..Default::default()
    }
}

fn with_harbor(run: impl FnOnce(pack::LoadedPack)) {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded.clone(), || run(loaded));
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn opening_social_actor_party_appraisal_preserves_personal_exception_and_changes_the_other_hero() {
    let mut loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    loaded
        .pack
        .scenes
        .iter_mut()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap()
        .setting
        .as_mut()
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .people
        .iter_mut()
        .find(|person| person.id == "bandit")
        .unwrap()
        .attitudes
        .insert(
            "Brynn".to_owned(),
            crate::gameplay::adventure::scene_model::StartingAttitude::Indifferent,
        );
    pack::with_test_pack(loaded, || {
        let current = runtime::opening_social_tests::opening(2);
        let member = current.state().characters[0].owner;
        let other = current.state().characters[1].owner;
        let proposal = OpeningSocialProposal {
            request: request(
                &current,
                179,
                "I will regrind your chipped scimitar if Finn can pass.",
            ),
            expected_basis: current.basis(),
            skill: OpeningSkill::Persuasion,
            attitude: df_rules::influence::Attitude::Indifferent,
            emotional: None,
            willingness: df_rules::influence::Willingness::Hesitant,
            roleplay_softened: true,
            approved_party_easing: Some(df_rules::influence::Attitude::Hostile),
            changed_after: None,
        };
        let result = actor::stage_opening_social_proposal(
            &current,
            member,
            &SocialLedger::none(),
            &proposal,
            &mut |_| Ok(1),
        )
        .unwrap();
        assert_eq!(result.result.mode, df_rules::influence::RollMode::Normal);
        assert_eq!(
            SocialLedger::none()
                .opening_attitude(&result.checkpoint, member)
                .unwrap(),
            df_rules::influence::Attitude::Indifferent
        );
        assert_eq!(
            SocialLedger::none()
                .opening_attitude(&result.checkpoint, other)
                .unwrap(),
            df_rules::influence::Attitude::Indifferent
        );
        assert!(
            SocialLedger::none()
                .development_opening_social(&result.checkpoint, "Dain Rusk", Some("Brynn"))
                .unwrap()
                .unwrap()["relationships"][0]["exception"]
                .as_bool()
                .unwrap()
        );
    });
}

#[test]
fn opening_social_authenticated_helper_reaches_actor_and_advantage_is_consumed_once() {
    with_harbor(|loaded| {
        let current = runtime::opening_social_tests::opening(2);
        let member = current.state().characters[0].owner;
        let current = actor::stage_opening_social_proposal(
            &current,
            member,
            &SocialLedger::none(),
            &OpeningSocialProposal {
                request: request(
                    &current,
                    180,
                    "I will regrind your chipped scimitar if Finn can pass.",
                ),
                expected_basis: current.basis(),
                skill: OpeningSkill::Persuasion,
                attitude: df_rules::influence::Attitude::Hostile,
                emotional: None,
                willingness: df_rules::influence::Willingness::Hesitant,
                roleplay_softened: true,
                approved_party_easing: Some(df_rules::influence::Attitude::Hostile),
                changed_after: None,
            },
            &mut |_| Ok(1),
        )
        .unwrap()
        .checkpoint;
        let endpoint = crate::gameplay::npc_reply::tests::ref069_existing_facts(vec![
            answer("o1"),
            answer("o6"),
            answer("o2"),
            answer("o3"),
            answer("o1"),
            answer("o5"),
            answer("o2"),
            answer("o1"),
            answer("o5"),
            answer("o2"),
            answer("o1"),
            answer("o2"),
        ]);
        let table = Table::new(current, loaded, &endpoint);
        let help = table.line(
            181,
            "Vale, I will stand at your shoulder and block Dain's retreat while you warn him.",
        );
        let result = run(table.service.submit_line([1; 32], help)).unwrap();
        assert!(matches!(result, DialogueReply::Raw(_, None)));
        let helped = table.service.updates.borrow().clone();
        let beneficiary = helped.state().characters[1].owner;
        assert!(
            runtime::opening_help::source_for(&helped, beneficiary, OpeningSkill::Intimidation)
                .unwrap()
                .is_some()
        );
        let first = table.line(
            182,
            "Dain, step aside or we will take you to the watch for frightening the boy.",
        );
        run(table.service.submit_line([2; 32], first)).unwrap();
        let after_first = table.service.updates.borrow().clone();
        assert!(
            runtime::opening_help::source_for(
                &after_first,
                beneficiary,
                OpeningSkill::Intimidation
            )
            .unwrap()
            .is_none()
        );
        let checks: Vec<_> = table
            .service
            .table_log
            .lines()
            .into_iter()
            .filter_map(|line| line.roll)
            .collect();
        assert_eq!(checks.last().unwrap().roll_mode, 1);
        assert_eq!(checks.last().unwrap().faces.len(), 2);
        let second = table.line(
            183,
            "Dain, step aside or I will call your employer to account in front of the watch.",
        );
        run(table.service.submit_line([2; 32], second)).unwrap();
        let checks: Vec<_> = table
            .service
            .table_log
            .lines()
            .into_iter()
            .filter_map(|line| line.roll)
            .collect();
        assert_eq!(checks.last().unwrap().roll_mode, 0);
        assert_eq!(checks.last().unwrap().faces.len(), 1);
        let before_weak = table.service.updates.borrow().clone();
        let weak = table.line(184, "Dain, please, step aside, I already asked.");
        run(table.service.submit_line([2; 32], weak)).unwrap();
        assert_eq!(table.service.updates.borrow().basis(), before_weak.basis());
        assert_eq!(
            table
                .service
                .table_log
                .lines()
                .into_iter()
                .filter(|line| line.roll.is_some())
                .count(),
            checks.len()
        );
        assert_eq!(
            SocialLedger::none()
                .opening_attitude(&before_weak, member)
                .unwrap(),
            df_rules::influence::Attitude::Indifferent
        );
    });
}

#[test]
fn opening_social_invitation_reaches_influence_before_travel_and_departure_stays_blocked() {
    with_harbor(|loaded| {
        let endpoint = crate::gameplay::npc_reply::tests::ref069_existing_facts(vec![
            answer("o15"),
            answer("o2"),
            answer("o2"),
            answer("o4"),
            answer("o15"),
            answer("o1"),
        ]);
        let table = Table::new(runtime::opening_social_tests::opening(2), loaded, &endpoint);
        let line = table.line(185, "Dain, walk with us to the Harbor Inn, first round's mine, and tell your employer the packet went to the watch.");
        run(table.service.submit_line([1; 32], line)).unwrap();
        assert_eq!(
            table
                .service
                .table_log
                .lines()
                .into_iter()
                .filter(|line| line.roll.is_some())
                .count(),
            1
        );
        assert_eq!(
            runtime::opening_party_attitude(table.service.updates.borrow().state())
                .unwrap()
                .unwrap()
                .0,
            df_rules::influence::Attitude::Indifferent
        );
        let before = table.service.updates.borrow().basis();
        let leave = table.line(186, "We leave Salt Quay now and head for the Harbor Inn.");
        run(table.service.submit_line([1; 32], leave)).unwrap();
        assert_eq!(table.service.updates.borrow().basis(), before);
        assert!(
            table
                .service
                .table_log
                .lines()
                .iter()
                .any(|line| line.text.contains("Settle the threat"))
        );
    });
}

#[test]
fn opening_social_information_questions_reach_dain_and_finn_spoken_contexts() {
    for (actor_id, speaker, words) in [
        ("bandit", "Dain Rusk", "Dain, who wants that packet?"),
        (
            "courier",
            "Finn Alder",
            "Finn, sweetheart, who sealed that packet, and who broke the seal before the ship reached port?",
        ),
    ] {
        with_harbor(|loaded| {
            let opening = runtime::opening_social_tests::opening(2);
            // This is the obstructing courier-answer phase, with both NPCs on the cast.
            let member = opening.state().characters[0].owner;
            let current = crate::gameplay::beat_script_variety_tests::act(
                &opening,
                member,
                188,
                "escort-courier",
            );
            let scene = runtime::scene(current.state()).unwrap();
            let cast = wire::scene_cast(&scene);
            let index = cast.iter().position(|actor| actor.id == actor_id).unwrap();
            let label = format!("o{}", index + 4);
            let endpoint = FakeEndpoint::start(vec![answer("o1"), answer("o1"), answer(&label)]);
            let table = Table::new(current, loaded, &endpoint);
            let line = table.line(187, words);
            let reply = run(table.service.submit_line([1; 32], line)).unwrap();
            let DialogueReply::Raw(response, Some(turn)) = reply else {
                panic!("a present NPC must answer")
            };
            assert_eq!(response.npc_actor_id, actor_id);
            assert_eq!(response.npc_speaker, speaker);
            assert_eq!(turn.context.actor_id, actor_id);
            assert_eq!(turn.context.speaker, speaker);
            let expected = if actor_id == "bandit" {
                "I was only told to collect it."
            } else {
                "I did not see who broke the seal."
            };
            let output =
                FakeEndpoint::start(vec![crate::gameplay::storyteller::tests::Reply::Output(
                    serde_json::json!({"reply": expected}),
                )]);
            let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/tmp/lanes/HS47")
                .join(format!(
                    "question-publication-{}-{actor_id}",
                    std::process::id()
                ));
            let fact_jev = crate::gameplay::npc_reply::tests::ref069_existing_facts(vec![]);
            let voice = NpcVoice::new(Arc::new(crate::gameplay::storyteller::tests::teller(
                output.address,
                1000,
                Duration::from_secs(5),
                &cache,
            )))
            .with_approach_model(crate::gameplay::approach_model::tests::model(&fact_jev));
            let mut context = turn.context;
            context.watch_publication(table.service.updates.clone());
            let exchange = run(voice.converse_report(context, &table.service.table_log)).unwrap();
            assert_eq!(exchange.text, expected);
            let lines = table.service.table_log.lines();
            let spoken = lines.iter().find(|line| line.text == expected).unwrap();
            assert_eq!(spoken.role, rpc::TableRole::Npc as i32);
            assert_eq!(spoken.kind, rpc::TableKind::Speech as i32);
            assert_eq!(spoken.actor_id, actor_id);
            assert_eq!(spoken.speaker, speaker);
            let display = rpc::DisplayGameplayView {
                journey: Some(rpc::JourneyView {
                    table_log: lines,
                    ..Default::default()
                }),
                ..Default::default()
            };
            let voices = crate::voice_line::display_lines(&display);
            assert_eq!(voices.last().unwrap().actor_id(), Some(actor_id));
            assert_eq!(voices.last().unwrap().speaker(), Some(speaker));
            if cache.exists() {
                std::fs::remove_dir_all(&cache).unwrap();
            }
            assert!(
                !table
                    .service
                    .table_log
                    .lines()
                    .iter()
                    .any(|line| line.role == rpc::TableRole::Narrator as i32)
            );
        });
    }
}
