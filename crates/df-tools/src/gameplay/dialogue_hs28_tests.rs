//! RP-03 service boundaries with closed-choice loopback fakes and a deterministic owner.
use super::*;
use crate::gameplay::{
    actor::place_actor::{MoveOutcome, Moved},
    approach_model::tests::{answer, model as jev},
    beat_script_runtime::{self as runtime, opening_social_tests},
    pack, place,
    place_creation::tests::{answer as place_answer, people_slot, place_slot, things_slot},
    social::SocialLedger,
    storyteller::{
        Storyteller,
        tests::{FakeEndpoint, Reply, teller},
    },
    table_log::TableLog,
};
use df_model::checkpoint::Checkpoint;
use df_session::inbox::{AdmissionSequence, Reducer, bounded_inbox};
use std::collections::VecDeque;
use tokio::sync::{Semaphore, watch};

struct Owner {
    current: Checkpoint,
    updates: watch::Sender<Checkpoint>,
    faces: VecDeque<[u8; 2]>,
    refuse_social: bool,
}

impl Reducer<actor::Call> for Owner {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        match call {
            actor::Call::SpeechMember { credential, reply } => {
                let member = self
                    .current
                    .state()
                    .characters
                    .get(usize::from(credential[0] - 1))
                    .map(|hero| hero.owner)
                    .ok_or_else(|| Status::permission_denied("seat absent"));
                assert!(reply.send(member).is_ok());
            }
            actor::Call::Dialogue { request, reply, .. } => {
                let result = match request {
                    DialogueRequest::PendingMerchantQuote => DialogueReply::PendingMerchantQuote {
                        hero_id: "Brynn".to_owned(),
                        quote: None,
                    },
                    DialogueRequest::PartyHeroes => DialogueReply::PartyHeroes(vec![]),
                    _ => panic!("unexpected dialogue request"),
                };
                assert!(reply.send(Ok(result)).is_ok());
            }
            actor::Call::OpeningSocial {
                credential,
                proposal,
                reply,
            } => {
                validate_opening_social_request(&proposal.request, self.current.basis()).unwrap();
                assert_eq!(proposal.expected_basis, self.current.basis());
                if self.refuse_social {
                    assert!(
                        reply
                            .send(Err(Status::unavailable("outcome unavailable")))
                            .is_ok()
                    );
                    return;
                }
                let member = self.current.state().characters[usize::from(credential[0] - 1)].owner;
                let faces = self
                    .faces
                    .pop_front()
                    .expect("only admitted requests draw dice");
                let mut draws = faces.into_iter();
                let staged = actor::stage_opening_social_proposal(
                    &self.current,
                    member,
                    &SocialLedger::none(),
                    &proposal,
                    &mut |sides| {
                        assert_eq!(sides, 20);
                        Ok(u32::from(
                            draws
                                .next()
                                .expect("the actor draws exactly the source dice"),
                        ))
                    },
                )
                .unwrap();
                self.current = staged.checkpoint.clone();
                self.updates.send_replace(self.current.clone());
                assert!(reply.send(Ok(staged)).is_ok());
            }
            actor::Call::PlaceMove {
                proposal, reply, ..
            } => {
                assert_eq!(proposal.expected_basis, self.current.basis());
                let planned = place::plan(
                    &self.current,
                    "Brynn",
                    proposal.operation,
                    &proposal.from,
                    &proposal.exit,
                    &proposal.to,
                    proposal.created.as_ref(),
                )
                .unwrap_or_else(|_| panic!("canonical travel must stage"));
                self.current = planned.candidate;
                self.updates.send_replace(self.current.clone());
                assert!(
                    reply
                        .send(Ok(MoveOutcome::Moved(Box::new(Moved {
                            revision: self.current.basis().revision,
                            from: proposal.from,
                            to: proposal.to,
                            place_name: planned.to_name,
                            exit: proposal.exit,
                            created: proposal.created.is_some(),
                            line: planned.entry.line,
                            narration: planned.arrival,
                        }))))
                        .is_ok()
                );
            }
            _ => panic!("unexpected owner call"),
        }
    }
}

struct Table {
    service: Service,
    owner: Option<std::thread::JoinHandle<()>>,
}
impl Table {
    fn start(
        current: Checkpoint,
        loaded: pack::LoadedPack,
        npc_voice: NpcVoice,
        faces: Vec<[u8; 2]>,
        refuse_social: bool,
    ) -> Self {
        let (updates_in, updates) = watch::channel(current.clone());
        let (sender, inbox) = bounded_inbox();
        let service = Service {
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
            npc_voice: Arc::new(npc_voice),
            table_log: TableLog::new(),
            social: SocialLedger::none(),
            living: crate::gameplay::living_scenes::LivingScenes::disabled(),
        };
        let owner = std::thread::spawn(move || {
            pack::with_test_pack(loaded, || {
                inbox
                    .run(&mut Owner {
                        current,
                        updates: updates_in,
                        faces: faces.into(),
                        refuse_social,
                    })
                    .unwrap();
            })
        });
        Self {
            service,
            owner: Some(owner),
        }
    }
    fn current(&self) -> Checkpoint {
        self.service.updates.borrow().clone()
    }
    fn line(&self, id: u8, text: &str) -> rpc::RawDialogueRequest {
        let basis = self.current().basis();
        rpc::RawDialogueRequest {
            session_id: Some(rpc::SessionId {
                value: Some(basis.session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(basis.run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(basis.revision)),
            input_id: Some(rpc::OperationId {
                value: Some(vec![id; 16]),
            }),
            text: text.to_owned(),
            finality: rpc::DialogueFinality::Final as i32,
            ..Default::default()
        }
    }
    fn travel(&self, id: u8) -> rpc::SubmitActionRequest {
        let basis = self.current().basis();
        let offer = wire::travel_offer(&self.current()).unwrap().unwrap();
        assert!(
            offer.input_groups[0]
                .options
                .iter()
                .any(|option| option.option_id == "exit:market-road")
        );
        rpc::SubmitActionRequest {
            session_id: Some(rpc::SessionId {
                value: Some(basis.session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(basis.run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(basis.revision)),
            operation_id: Some(rpc::OperationId {
                value: Some(vec![id; 16]),
            }),
            offer_id: offer.offer_id,
            action_kind: rpc::GameplayActionKind::Travel as i32,
            action_choices: vec![rpc::JourneyChoice {
                group_id: "travel".to_owned(),
                option_id: "exit:market-road".to_owned(),
            }],
            ..Default::default()
        }
    }
}
impl Drop for Table {
    fn drop(&mut self) {
        self.service.actor.stop().unwrap();
        self.owner.take().unwrap().join().unwrap();
    }
}
fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
fn with_harbor(run: impl FnOnce(pack::LoadedPack)) {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded.clone(), || run(loaded));
}
fn voice(endpoint: &FakeEndpoint, storyteller: Storyteller) -> NpcVoice {
    NpcVoice::new(Arc::new(storyteller)).with_approach_model(jev(endpoint))
}
fn silent_voice(endpoint: &FakeEndpoint) -> NpcVoice {
    voice(endpoint, Storyteller::disabled())
}

#[test]
fn hs28_talk_down_one_four_six_heroes_commits_before_line_roll_and_offers_travel() {
    for count in [1, 4, 6] {
        with_harbor(|loaded| {
            let jev = FakeEndpoint::start(vec![answer("o1"), answer("o3"), answer("o2")]);
            let table = Table::start(
                opening_social_tests::opening(count),
                loaded,
                silent_voice(&jev),
                vec![[20, 20]],
                false,
            );
            assert_eq!(
                wire::opening_talk_offer(&table.current())
                    .unwrap()
                    .unwrap()
                    .label,
                "Talk to Dain Rusk"
            );
            let request = table.line(
                90,
                "Dain, let Finn pass. We can bring customers to your next market.",
            );
            let reply = run(table.service.submit_line([1; 32], request)).unwrap();
            let DialogueReply::Raw(response, None) = reply else {
                panic!("committed social reply")
            };
            assert!(response.npc_reply.contains("steps aside"));
            assert!(runtime::opening_threat_resolved(table.current().state()).unwrap());
            assert!(table.current().state().encounters.is_empty());
            assert_eq!(table.current().state().characters.len(), count);
            assert!(
                wire::opening_talk_offer(&table.current())
                    .unwrap()
                    .is_none()
            );
            assert!(wire::travel_offer(&table.current()).unwrap().is_some());
            let current = table.current();
            let projected = wire::journey_view(
                &current,
                df_persistence::local_demo_scope::LocalDemoRole::Player,
                current.state().characters[0].owner,
            )
            .unwrap();
            let Some(rpc::view_message::Audience::Player(player)) = projected.audience else {
                panic!("phone view")
            };
            let travel = player
                .offers
                .iter()
                .find(|offer| offer.action_kind == rpc::GameplayActionKind::Travel as i32)
                .expect("the real phone projection includes reachable travel");
            assert!(
                travel.input_groups[0]
                    .options
                    .iter()
                    .any(|option| option.option_id == "exit:market-road")
            );

            let lines = table.service.table_log.lines();
            assert_eq!(
                lines.iter().filter(|line| line.speaker == "Roll").count(),
                1
            );
            assert_eq!(
                lines
                    .iter()
                    .filter(|line| line.speaker == "Narrator")
                    .count(),
                1
            );
            let previous = runtime::previous_opening_social(&table.current())
                .unwrap()
                .unwrap();
            assert!(previous.1.unwrap().contains("bring customers"));
            assert_eq!(jev.connections(), 3);
            let social_schema = jev.bodies()[1].to_string();
            assert!(social_schema.contains("previous_failed_request"));
            assert!(social_schema.contains("materially"));
        });
    }
}

#[test]
fn hs28_failure_same_urging_does_not_reroll_changed_offer_can_resolve() {
    with_harbor(|loaded| {
        let jev = FakeEndpoint::start(vec![
            answer("o1"),
            answer("o3"),
            answer("o2"),
            answer("o1"),
            answer("o2"),
            answer("o1"),
            answer("o5"),
            answer("o2"),
        ]);
        let table = Table::start(
            opening_social_tests::opening(4),
            loaded,
            silent_voice(&jev),
            vec![[1, 1], [20, 20]],
            false,
        );
        run(table.service.submit_line(
            [1; 32],
            table.line(90, "Let the courier through for our good word."),
        ))
        .unwrap();
        assert!(!runtime::opening_threat_resolved(table.current().state()).unwrap());
        assert_eq!(
            wire::opening_talk_offer(&table.current())
                .unwrap()
                .unwrap()
                .label,
            "Try another offer"
        );
        assert!(wire::travel_offer(&table.current()).unwrap().is_none());
        let failed = table.current();
        run(table.service.submit_line(
            [2; 32],
            table.line(91, "Please let the courier through for our good word."),
        ))
        .unwrap();
        assert_eq!(
            table.current(),
            failed,
            "another hero's same urging cannot reroll"
        );
        run(table.service.submit_line(
            [3; 32],
            table.line(92, "Release Finn or we report your extortion to the watch."),
        ))
        .unwrap();
        assert!(runtime::opening_threat_resolved(table.current().state()).unwrap());
        assert_eq!(
            table
                .current()
                .state()
                .decisions
                .iter()
                .filter(|decision| decision.source_policy.as_str() == "opening-social-1")
                .count(),
            2
        );
        assert_eq!(
            table
                .service
                .table_log
                .lines()
                .iter()
                .filter(|line| line.speaker == "Roll")
                .count(),
            2
        );
        assert!(jev.bodies()[4].to_string().contains("our good word"));
        assert_eq!(jev.connections(), 8);
    });
}

#[test]
fn hs28_uncertain_social_owner_publishes_no_line_or_roll() {
    with_harbor(|loaded| {
        let jev = FakeEndpoint::start(vec![answer("o1"), answer("o3"), answer("o2")]);
        let opening = opening_social_tests::opening(1);
        let table = Table::start(opening.clone(), loaded, silent_voice(&jev), vec![], true);
        assert!(
            run(table.service.submit_line(
                [1; 32],
                table.line(90, "Dain, let Finn pass for our introduction.")
            ))
            .is_err()
        );
        assert_eq!(table.current(), opening);
        assert!(table.service.table_log.lines().is_empty());
    });
}

#[test]
fn hs28_phone_travel_dispatch_rechecks_canonical_selection_and_publishes_one_arrival() {
    use rpc::action_service_server::ActionService;
    with_harbor(|loaded| {
        let jev = FakeEndpoint::start(vec![answer("o1"), answer("o3"), answer("o2")]);
        let mut market = place_slot();
        market["name"] = serde_json::json!("Market Square");
        let teller_endpoint = FakeEndpoint::start(vec![Reply::Output(place_answer(
            &market,
            &people_slot(),
            &things_slot(),
        ))]);
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/lanes/HS28/market-cache-{}",
            std::process::id()
        ));
        let table = Table::start(
            opening_social_tests::opening(6),
            loaded,
            voice(
                &jev,
                teller(teller_endpoint.address, 0, Duration::from_secs(5), &cache),
            ),
            vec![[20, 20]],
            false,
        );
        run(table.service.submit_line(
            [1; 32],
            table.line(90, "Dain, let Finn pass for our introduction."),
        ))
        .unwrap();
        let request = table.travel(91);
        let mut stale = request.clone();
        stale.observed_revision = Some(wire::revision(
            table.current().basis().revision.next_sequence().unwrap(),
        ));
        let stale_result = run(table.service.travel_submit([1; 32], stale)).unwrap();
        assert!(
            matches!(stale_result.outcome, Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt))
            if matches!(receipt.outcome, Some(rpc::decision_receipt::Outcome::Rejected(rejection))
                if rejection.code == rpc::RejectionCode::StaleOffer as i32))
        );
        assert_eq!(teller_endpoint.connections(), 0);
        let mut invalid = request.clone();
        invalid.action_choices[0].option_id = "exit:forged".to_owned();
        let before = table.current();
        let refused = run(table.service.travel_submit([1; 32], invalid)).unwrap();
        assert!(
            matches!(refused.outcome, Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) if matches!(receipt.outcome, Some(rpc::decision_receipt::Outcome::Rejected(_))))
        );
        assert_eq!(table.current(), before);
        let mut rpc_request = Request::new(request.clone());
        rpc_request.metadata_mut().insert(
            "x-df-local-binding",
            crate::gameplay::hex(&[1; 32]).parse().unwrap(),
        );
        run(table.service.submit(rpc_request)).unwrap();
        assert_eq!(
            runtime::scene(table.current().state()).unwrap().title,
            "Market Square"
        );
        assert_eq!(place::entries(table.current().state()).unwrap().len(), 1);
        let arrivals = table
            .service
            .table_log
            .lines()
            .iter()
            .filter(|line| line.text.contains("Market Square") && line.speaker == "Narrator")
            .count();
        assert_eq!(arrivals, 1);
        let replay = run(table.service.travel_submit([1; 32], request)).unwrap();
        assert!(matches!(
            replay.outcome,
            Some(rpc::submit_action_response::Outcome::OperationObservation(
                _
            ))
        ));
        assert_eq!(place::entries(table.current().state()).unwrap().len(), 1);
        assert_eq!(teller_endpoint.connections(), 1);
    });
}

#[test]
fn hs28_typed_travel_refused_during_threat_and_owner_plan_rechecks_all_move_paths() {
    with_harbor(|loaded| {
        let jev = FakeEndpoint::start(vec![answer("o15")]);
        let opening = opening_social_tests::opening(1);
        let table = Table::start(opening.clone(), loaded, silent_voice(&jev), vec![], false);
        let reply = run(table.service.submit_line(
            [1; 32],
            table.line(90, "Courier, you walk with us to the market square."),
        ))
        .unwrap();
        let DialogueReply::Raw(response, None) = reply else {
            panic!("honest refusal")
        };
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Rejected as i32
        );
        assert!(response.npc_reply.contains("enforcer"));
        assert_eq!(table.current(), opening);
        let scene = runtime::scene(opening.state()).unwrap();
        let party = place::whereabouts(opening.state(), &scene).unwrap();
        let plan = place::plan(
            &opening,
            "Brynn",
            OperationId::from_bytes(&[99; 16]).unwrap(),
            &party.here,
            "market-road",
            "place-market-square",
            None,
        );
        assert!(matches!(
            plan,
            Err(place::PlaceFailure::Refused(
                place::PlaceRefusal::TravelBlocked(_)
            ))
        ));
        assert_eq!(jev.connections(), 1);
    });
}

#[test]
fn hs39_dialogue_social_useful_offer_eases_hostile_to_indifferent_before_owner_roll() {
    with_harbor(|loaded| {
        let jev = FakeEndpoint::start(vec![answer("o1"), answer("o3"), answer("o4")]);
        let table = Table::start(
            opening_social_tests::opening(6),
            loaded,
            silent_voice(&jev),
            vec![[15, 1]],
            false,
        );
        let request = table.line(
            191,
            "Dain, let Finn pass and I will regrind that nicked blade at my forge.",
        );
        let reply = run(table.service.submit_line([1; 32], request)).unwrap();
        assert!(matches!(reply, DialogueReply::Raw(_, None)));
        let rows = table.service.table_log.lines();
        let roll = rows
            .iter()
            .find_map(|line| line.roll.as_ref())
            .expect("owner publishes actual Influence roll");
        assert_eq!(
            roll.roll_mode, 0,
            "Indifferent removes Hostile disadvantage"
        );
        assert_eq!(roll.faces, [15]);
        let receipt = table
            .current()
            .state()
            .decisions
            .last()
            .unwrap()
            .semantic_output
            .clone()
            .unwrap();
        assert!(
            receipt.contains("indifferent"),
            "effective attitude survives checkpoint replay"
        );
        assert!(
            crate::gameplay::actor::audit_lines_containing("OPENING_SOCIAL_ATTITUDE")
                .iter()
                .any(|line| line.contains(
                    "before=hostile for_request=indifferent roleplay_softened=true mode=Normal"
                ))
        );
    });
}

#[test]
fn hs39_dialogue_social_consecutive_refusals_vary_without_retrying_the_same_urging() {
    with_harbor(|loaded| {
        let jev = FakeEndpoint::start(vec![
            answer("o1"),
            answer("o3"),
            answer("o2"),
            answer("o1"),
            answer("o2"),
        ]);
        let table = Table::start(
            opening_social_tests::opening(2),
            loaded,
            silent_voice(&jev),
            vec![[1, 1]],
            false,
        );
        let first = run(table.service.submit_line(
            [1; 32],
            table.line(192, "Dain, let Finn pass for an introduction to buyers."),
        ))
        .unwrap();
        let second = run(table.service.submit_line(
            [1; 32],
            table.line(193, "Dain, let him pass for that same introduction."),
        ))
        .unwrap();
        let (DialogueReply::Raw(first, None), DialogueReply::Raw(second, None)) = (first, second)
        else {
            panic!("two in-fiction replies")
        };
        assert!(!first.npc_reply.is_empty());
        assert!(!second.npc_reply.is_empty());
        assert_ne!(first.npc_reply, second.npc_reply);
        assert_eq!(
            table
                .service
                .table_log
                .lines()
                .iter()
                .filter(|line| line.roll.is_some())
                .count(),
            1
        );
    });
}

#[test]
fn hs39_dialogue_trade_exhausted_exchange_judgement_is_in_fiction_not_status_error() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
    let mut document: pack::PackFile = serde_json::from_slice(include_bytes!(
        "../../../../content/harbor-journey/pack.json"
    ))
    .unwrap();
    document.creation.pick_lists.skill = vec!["perception".into(), "survival".into()];
    for scene in &mut document.scenes {
        scene.setting = Some(generated.settings.scene(scene.key).clone());
        let source = generated
            .pack
            .scenes
            .iter()
            .find(|source| source.key == scene.key)
            .unwrap();
        scene.asset = source.asset.clone();
        scene.cast = source.cast.clone();
        scene.speaker = source.speaker.clone();
    }
    let loaded = pack::load_from_bytes(
        &serde_json::to_vec(&document).unwrap(),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded.clone(), || {
        let current = opening_social_tests::opening(1);
        let jev = FakeEndpoint::start(vec![answer("o8")]);
        let endpoint = FakeEndpoint::start(vec![
            Reply::Output(serde_json::json!({"exchange":"{\"offer\":"})),
            Reply::Output(serde_json::json!({"exchange":"{\"offer\":"})),
        ]);
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS39/terminal-trade-failure")
            .join(journey::hex(&model::random::<16>().unwrap()));
        let table = Table::start(
            current.clone(),
            loaded,
            voice(
                &jev,
                teller(endpoint.address, 0, Duration::from_secs(5), &cache),
            ),
            vec![],
            false,
        );
        let reply = run(table.service.submit_line(
            [1; 32],
            table.line(
                194,
                "Mistress Venn, would you buy a dozen forged harness rings?",
            ),
        ))
        .expect("a merchant unable to answer must speak in fiction instead of an error toast");
        let DialogueReply::Raw(response, None) = reply else {
            panic!("in-fiction answer")
        };
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Social as i32
        );
        assert!(response.npc_reply.contains("Give me a moment to think"));
        assert_eq!(table.current().basis(), current.basis());
        assert_eq!(endpoint.connections(), 2);
    });
}
