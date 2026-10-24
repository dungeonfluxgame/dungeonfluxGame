use super::*;
use crate::gameplay::{
    actor::place_actor::{MoveOutcome, Moved},
    approach_model::tests::{answer, model as jev},
    beat_script_runtime as runtime, beat_script_variety_tests as native,
    journey::checkpoint_fight_tests::{fight, with_created_place},
    pack, place,
    place_creation::tests::{answer as place_answer, people_slot, place_slot, things_slot},
    storyteller::{
        Storyteller,
        tests::{FakeEndpoint, Reply, cache_directory, teller},
    },
    table_log::TableLog,
};
use df_model::checkpoint::Checkpoint;
use df_session::inbox::{AdmissionSequence, Reducer, bounded_inbox};
use tokio::sync::{Semaphore, watch};

struct Owner {
    current: Checkpoint,
    updates: watch::Sender<Checkpoint>,
    story_replies: bool,
}

impl Reducer<actor::Call> for Owner {
    fn reduce(&mut self, _: AdmissionSequence, call: actor::Call) {
        match call {
            actor::Call::SpeechMember { reply, .. } => {
                reply.send(Ok(native::first())).unwrap();
            }
            actor::Call::PlaceMove {
                proposal, reply, ..
            } => {
                let hero = journey::name(
                    self.current.state(),
                    self.current.state().characters[0].entity,
                )
                .unwrap();
                let planned = place::plan(
                    &self.current,
                    &hero,
                    proposal.operation,
                    &proposal.from,
                    &proposal.exit,
                    &proposal.to,
                    proposal.created.as_ref(),
                )
                .unwrap_or_else(|_| panic!("the real place owner accepts the departure"));
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
            actor::Call::Dialogue {
                credential,
                request,
                reply,
                ..
            } => {
                if self.story_replies {
                    let index = usize::from(credential[0] - 1);
                    let character = &self.current.state().characters[index];
                    let member = character.owner;
                    let hero = journey::name(self.current.state(), character.entity).unwrap();
                    if matches!(request, DialogueRequest::PendingMerchantQuote) {
                        let result =
                            journey::living_scene::pending_merchant_quote(&self.current, &hero)
                                .map(|quote| DialogueReply::PendingMerchantQuote {
                                    hero_id: hero,
                                    quote,
                                })
                                .map_err(actor::unavailable);
                        assert!(reply.send(result).is_ok());
                        return;
                    }
                    let raw = match &request {
                        DialogueRequest::Raw(bytes) => Some((bytes, None)),
                        DialogueRequest::RawTarget { bytes, actor_id } => {
                            Some((bytes, Some(actor_id.as_str())))
                        }
                        _ => None,
                    };
                    if let Some((bytes, target)) = raw {
                        let line = rpc::RawDialogueRequest::decode(bytes.as_slice()).unwrap();
                        let view = wire::journey_view(
                            &self.current,
                            df_persistence::local_demo_scope::LocalDemoRole::Player,
                            member,
                        )
                        .unwrap();
                        let facts = pack::with_content(StoryFacts::from_pack);
                        let context = NpcReplyContext::for_target(
                            &view,
                            member,
                            &line.text,
                            &facts,
                            &crate::gameplay::social::SocialLedger::none(),
                            "opening",
                            target,
                        )
                        .expect("a typed line reaches the opening NPC's storyteller context");
                        let response = DialogueReply::Raw(
                            rpc::DialogueResponse {
                                input_id: line.input_id,
                                revision: Some(wire::revision(self.current.basis().revision)),
                                disposition: rpc::DialogueDisposition::Social as i32,
                                ..Default::default()
                            },
                            Some(SpokenTurn {
                                context,
                                hero_trade: HeroTradeIntent::not_requested(),
                                cleanup: None,
                                typed_line_shown: false,
                            }),
                        );
                        assert!(reply.send(Ok(response)).is_ok());
                        return;
                    }
                }
                let response = match request {
                    DialogueRequest::PartyHeroes => DialogueReply::PartyHeroes(Vec::new()),
                    DialogueRequest::PendingMerchantQuote => DialogueReply::PendingMerchantQuote {
                        hero_id: "Brynn".to_owned(),
                        quote: None,
                    },
                    DialogueRequest::Raw(bytes) | DialogueRequest::RawTarget { bytes, .. } => {
                        let request = rpc::RawDialogueRequest::decode(bytes.as_slice()).unwrap();
                        DialogueReply::Raw(
                            rpc::DialogueResponse {
                                input_id: request.input_id,
                                revision: Some(wire::revision(self.current.basis().revision)),
                                disposition: rpc::DialogueDisposition::Social as i32,
                                ..Default::default()
                            },
                            None,
                        )
                    }
                    _ => panic!("unexpected dialogue call"),
                };
                assert!(reply.send(Ok(response)).is_ok());
            }
            _ => panic!("unexpected routing owner call"),
        }
    }
}

struct Table {
    service: Service,
    owner: Option<std::thread::JoinHandle<()>>,
}

impl Table {
    fn start(current: Checkpoint, loaded: pack::LoadedPack, npc_voice: NpcVoice) -> Self {
        Self::start_with_story_replies(current, loaded, npc_voice, false)
    }

    fn start_with_story_replies(
        current: Checkpoint,
        loaded: pack::LoadedPack,
        npc_voice: NpcVoice,
        story_replies: bool,
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
            social: crate::gameplay::social::SocialLedger::none(),
            living: crate::gameplay::living_scenes::LivingScenes::disabled(),
        };
        let owner = std::thread::spawn(move || {
            pack::with_test_pack(loaded, || {
                inbox
                    .run(&mut Owner {
                        current,
                        updates: updates_in,
                        story_replies,
                    })
                    .unwrap();
            });
        });
        Self {
            service,
            owner: Some(owner),
        }
    }

    fn line(&self, text: &str) -> rpc::RawDialogueRequest {
        let current = self.service.updates.borrow();
        rpc::RawDialogueRequest {
            session_id: Some(rpc::SessionId {
                value: Some(current.basis().session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(current.basis().run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(current.basis().revision)),
            input_id: Some(rpc::OperationId {
                value: Some(vec![91; 16]),
            }),
            context: rpc::DialogueContext::Question as i32,
            finality: rpc::DialogueFinality::Final as i32,
            text: text.to_owned(),
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

#[test]
fn n1_spoken_departure_commits_the_exit_move_and_shows_arrival() {
    with_created_place(|home, _, loaded| {
        // Ordinary merchant meaning, then travel intent and the committed street-door.
        let endpoint = FakeEndpoint::start(vec![answer("o10"), answer("o15"), answer("o1")]);
        let storyteller = FakeEndpoint::start(vec![Reply::Output(place_answer(
            &place_slot(),
            &people_slot(),
            &things_slot(),
        ))]);
        let cache = cache_directory("n1-spoken-departure");
        let voice = NpcVoice::new(Arc::new(teller(
            storyteller.address,
            0,
            std::time::Duration::from_secs(5),
            &cache,
        )))
        .with_approach_model(jev(&endpoint));
        let table = Table::start(home.clone(), loaded, voice);
        let request = table.line(
            "Come on, Brakka. Let's get out of this shop before the watch has questions. Out into the street.",
        );
        let DialogueReply::Raw(response, None) =
            run(table.service.submit_line([1; 32], request.clone())).unwrap()
        else {
            panic!("departure resolves without an NPC reply");
        };
        let current = table.service.updates.borrow().clone();
        let arrived = runtime::scene(current.state()).unwrap();
        assert_eq!(arrived.setting.unwrap().place_id, "place-market-street");
        assert_eq!(arrived.title, "Market Street");
        assert_ne!(current.basis().revision, home.basis().revision);
        assert_eq!(
            response.revision,
            Some(wire::revision(current.basis().revision))
        );
        assert_eq!(
            response.disposition,
            rpc::DialogueDisposition::Action as i32
        );
        assert_eq!(response.input_id, request.input_id);
        assert_eq!(
            response.npc_reply,
            place::arrival_line("Brynn", "Market Street")
        );
        let rows = table.service.table_log.lines();
        assert_eq!(
            rows.len(),
            2,
            "the hero's departure, then the committed arrival"
        );
        assert_eq!(rows[0].role, rpc::TableRole::Hero as i32);
        assert_eq!(rows[0].text, request.text);
        assert_eq!(rows[1].role, rpc::TableRole::Narrator as i32);
        assert_eq!(rows[1].text, response.npc_reply);
        assert_eq!(endpoint.connections(), 3);
        assert_eq!(storyteller.connections(), 1);
        let bodies = endpoint.bodies();
        assert!(
            bodies[1]["questions"]["q1"]["criteria"]["o15"]
                .as_str()
                .unwrap()
                .starts_with("travel:")
        );
        let criteria = bodies[2]["questions"]["q1"]["criteria"]
            .as_object()
            .unwrap();
        assert_eq!(criteria.len(), 3, "the open exit, elsewhere, and stay");
        assert!(criteria["o1"].as_str().unwrap().contains("street-door"));
        assert!(criteria["o3"].as_str().unwrap().starts_with("none:"));
    });
}

#[test]
fn n1_jev_stay_and_unavailable_do_not_infer_departure_from_words() {
    with_created_place(|opening, _, loaded| {
        let scene = runtime::scene(opening.state()).unwrap();
        let endpoint = FakeEndpoint::start(vec![answer("o3")]);
        let voice =
            NpcVoice::new(Arc::new(Storyteller::disabled())).with_approach_model(jev(&endpoint));
        let table = Table::start(opening.clone(), loaded.clone(), voice);
        let line = table.line("Let's get out into the street.");
        assert!(run(table.service.travel_by_line([1; 32], &line, &scene)).is_err());
        assert_eq!(*table.service.updates.borrow(), opening);
        assert!(table.service.table_log.lines().is_empty());
        assert_eq!(endpoint.connections(), 1);
        let table = Table::start(opening.clone(), loaded, NpcVoice::silent());
        assert!(run(table.service.travel_by_line([1; 32], &line, &scene)).is_err());
        assert_eq!(*table.service.updates.borrow(), opening);
    });
}

#[test]
fn n1_closed_exits_are_not_offered_to_jev() {
    with_created_place(|opening, _, mut loaded| {
        for scene in &mut loaded.pack.scenes {
            if let Some(model) = scene
                .setting
                .as_mut()
                .and_then(|setting| setting.model.as_mut())
            {
                for exit in &mut model.ways_out {
                    exit.open = false;
                }
            }
        }
        if let Some(script) = loaded.pack.beat_script.as_mut() {
            for scene in script.scenes.values_mut() {
                if let Some(model) = scene
                    .setting
                    .as_mut()
                    .and_then(|setting| setting.model.as_mut())
                {
                    for exit in &mut model.ways_out {
                        exit.open = false;
                    }
                }
            }
        }
        pack::with_test_pack(loaded.clone(), || {
            let scene = runtime::scene(opening.state()).unwrap();
            let endpoint = FakeEndpoint::start(vec![answer("o2")]);
            let voice = NpcVoice::new(Arc::new(Storyteller::disabled()))
                .with_approach_model(jev(&endpoint));
            let table = Table::start(opening.clone(), loaded.clone(), voice);
            let line = table.line("Let's get out into the street.");
            assert!(run(table.service.travel_by_line([1; 32], &line, &scene)).is_err());
            assert_eq!(endpoint.connections(), 1);
            let body = endpoint.bodies()[0].to_string();
            assert!(
                !body.contains("street-door"),
                "closed exits never enter the heading options"
            );
            assert_eq!(*table.service.updates.borrow(), opening);
            assert!(table.service.table_log.lines().is_empty());
        });
    });
}

#[test]
fn n1_unresolvable_final_line_gets_one_narrator_row() {
    with_created_place(|opening, _, loaded| {
        let table = Table::start(opening, loaded, NpcVoice::silent());
        let line = table.line("Can anything answer this?");
        let DialogueReply::Raw(response, None) =
            run(table.service.submit_line([1; 32], line)).unwrap()
        else {
            panic!("unresolvable line gets a narrator response");
        };
        assert_eq!(response.npc_reply, EMPTY_META_NARRATION);
        let rows = table.service.table_log.lines();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, rpc::TableRole::Narrator as i32);
        assert_eq!(rows[0].text, response.npc_reply);
    });
}

#[test]
fn n1_private_aside_feedback_does_not_publish_private_words() {
    with_created_place(|opening, _, loaded| {
        let table = Table::start(opening, loaded, NpcVoice::silent());
        let line = table.line("A private aside to Brakka.");
        let DialogueReply::Raw(response, None) = table
            .service
            .narrated_addressee_reply(&line, PRIVATE_ASIDE_UNAVAILABLE_NARRATION)
        else {
            panic!("private feedback returns to the phone");
        };
        assert_eq!(response.npc_reply, PRIVATE_ASIDE_UNAVAILABLE_NARRATION);
        assert!(table.service.table_log.lines().is_empty());
    });
}

#[test]
fn n1_fled_and_dead_people_are_excluded_from_the_jev_addressee_choices() {
    with_created_place(|_, street, loaded| {
        let original = runtime::scene(street.state()).unwrap();
        assert!(original.cast.len() >= 2);
        let initialized = fight(&street);
        let mut state = street.state().clone();
        state.entities = initialized.state().entities.clone();
        state.continuity.npcs = initialized.state().continuity.npcs.clone();
        state.resources = initialized.state().resources.clone();
        let fled = &original.cast[0];
        let dead = &original.cast[1];
        let fled_entity = journey::scene_fight::person_entity(&fled.id).unwrap();
        state
            .entities
            .iter_mut()
            .find(|entity| entity.id == fled_entity)
            .unwrap()
            .location = None;
        let dead_entity = journey::scene_fight::person_entity(&dead.id).unwrap();
        for resource in &mut state.resources {
            if resource.owner == dead_entity {
                match resource.resource.as_str() {
                    "unconscious" => resource.value = 1,
                    "hit-points" => resource.value = 0,
                    _ => {}
                }
            }
        }
        let current = model::checkpoint(street.basis(), state).unwrap();
        let scene = runtime::scene(current.state()).unwrap();
        let cast = wire::scene_cast(&scene);
        assert!(super::super::npc_target::current(&cast, &fled.id).is_none());
        assert!(super::super::npc_target::current(&cast, &dead.id).is_none());
        let endpoint = FakeEndpoint::start(vec![answer("o1")]);
        let decision = run(jev(&endpoint).resolve_addressee(
            &format!("{}, are you there?", fled.name),
            &cast,
            &fled.name,
        ));
        assert!(matches!(
            addressee_route(&decision, &cast, &[]),
            AddresseeRoute::Unmatched { .. }
        ));
        let present = endpoint.bodies()[0]["state"]["present_cast"].to_string();
        assert!(!present.contains(&fled.id));
        assert!(!present.contains(&dead.id));
        let table = Table::start(current, loaded, NpcVoice::silent());
        let DialogueReply::Raw(response, None) = table.service.narrated_addressee_reply(
            &table.line(&format!("{}, are you there?", fled.name)),
            NAME_NOT_HERE_NARRATION,
        ) else {
            panic!("absent addressee takes the narrator path");
        };
        assert_eq!(response.npc_reply, NAME_NOT_HERE_NARRATION);
        let rows = table.service.table_log.lines();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].role, rpc::TableRole::Narrator as i32);
    });
}

#[test]
fn story_input_one_four_and_six_mixed_heroes_reach_the_storyteller_after_opening() {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../content/harbor-journey/pack.json"),
        include_bytes!("fixtures/harbor-journey-tuning-baseline.json"),
    )
    .unwrap();
    for count in [1, 4, 6] {
        pack::with_test_pack(loaded.clone(), || {
            let mut current = journey::initial().unwrap();
            for index in 0..count {
                current = journey::stage_join(
                    &current,
                    &crate::gameplay::beat_script_tests::input(
                        &current,
                        journey::bootstrap_member().unwrap(),
                        index as u8 + 1,
                        "join-room",
                        vec![],
                    ),
                )
                .unwrap();
            }
            let identities = [
                ("Rurik", "dwarf", vec![]),
                (
                    "Lethiel",
                    "elf",
                    vec![
                        ("elven-lineage", "wood-elf"),
                        ("keen-senses", "insight"),
                        ("species-spellcasting", "wisdom"),
                    ],
                ),
                ("Pip", "halfling", vec![]),
                ("Arjhan", "dragonborn", vec![("draconic-ancestry", "red")]),
                (
                    "Nemeia",
                    "tiefling",
                    vec![
                        ("fiendish-legacy", "infernal"),
                        ("species-size", "medium"),
                        ("species-spellcasting", "charisma"),
                    ],
                ),
                ("Gauthak", "goliath", vec![("giant-ancestry", "fire")]),
            ];
            let members = journey::participants(&current)
                .into_iter()
                .map(|link| link.member)
                .collect::<Vec<_>>();
            for (index, (name, species, extra)) in identities.into_iter().take(count).enumerate() {
                let mut choices = crate::gameplay::beat_script_tests::build(name);
                choices
                    .iter_mut()
                    .find(|(key, _)| key.as_str() == "species")
                    .unwrap()
                    .1 = model::label(species).unwrap();
                choices.extend(extra.into_iter().map(|(key, value)| {
                    (model::label(key).unwrap(), model::label(value).unwrap())
                }));
                current = journey::stage_with_supplier(
                    &current,
                    &crate::gameplay::beat_script_tests::input(
                        &current,
                        members[index],
                        index as u8 + 10,
                        "create-character",
                        choices,
                    ),
                    &mut |_| panic!("creation cannot draw"),
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "{species}: {error:?}; {:?}",
                        crate::gameplay::refusal_detail::take()
                    )
                });
            }
            current = journey::stage_with_supplier(
                &current,
                &crate::gameplay::beat_script_tests::input(
                    &current,
                    members[0],
                    26,
                    "begin-story",
                    vec![],
                ),
                &mut |_| panic!("opening cannot draw"),
            )
            .unwrap();
            assert_eq!(
                journey::phase(&current).unwrap(),
                rpc::JourneyPhase::Opening
            );
            let scene = runtime::scene(current.state()).unwrap();
            assert!(
                journey::living_scene::snapshot(current.state(), &scene)
                    .unwrap()
                    .is_some(),
                "HS23 authored a canonical living model for the harbor opening"
            );
            let lines = (0..count)
                .map(|index| {
                    format!(
                        "Courier, my name is {}. Tell us about the broken seal.",
                        journey::name(current.state(), current.state().characters[index].entity)
                            .unwrap()
                    )
                })
                .collect::<Vec<_>>();
            // Character speech, question form, explicitly addressed to the present Courier.
            let endpoint = FakeEndpoint::start(
                (0..count)
                    .flat_map(|_| {
                        [
                            answer("o1"),  // character speech
                            answer("o1"),  // not a request to settle the obstruction
                            answer("o15"), // ordinary conversation, no scene interaction
                            answer("o4"), // courier is the candidate service provider (no listed services)
                            answer("o1"), // a question
                            answer("o4"), // the present courier
                        ]
                    })
                    .collect(),
            );
            let storyteller = FakeEndpoint::start(
                lines
                    .iter()
                    .map(|line| Reply::Output(serde_json::json!({"clean": line})))
                    .collect(),
            );
            let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../artifacts/tmp/lanes/HS28/story-input-{count}-{}",
                std::process::id()
            ));
            let voice = NpcVoice::new(Arc::new(teller(
                storyteller.address,
                0,
                std::time::Duration::from_secs(5),
                &cache,
            )))
            .with_approach_model(jev(&endpoint));
            let table =
                Table::start_with_story_replies(current.clone(), loaded.clone(), voice, true);
            for (index, words) in lines.iter().enumerate() {
                let mut request = table.line(words);
                request.input_id = Some(rpc::OperationId {
                    value: Some(vec![80 + index as u8; 16]),
                });
                request.context = rpc::DialogueContext::Social as i32;
                let result = run(async {
                    let reply = table
                        .service
                        .submit_line([index as u8 + 1; 32], request)
                        .await?;
                    let DialogueReply::Raw(_, Some(mut turn)) = reply else {
                        panic!("seat {index} must reach the storyteller reply path");
                    };
                    assert_eq!(turn.context.typed_words, *words);
                    let line = turn
                        .cleanup
                        .take()
                        .expect("spoken line cleanup starts")
                        .finish()
                        .await
                        .unwrap();
                    assert_eq!(line.text, *words);
                    Ok::<_, tonic::Status>(())
                });
                result.unwrap_or_else(|error| panic!("{count}-hero seat {index}: {error:?}"));
            }
            assert_eq!(endpoint.connections(), 6 * count);
            assert_eq!(storyteller.connections(), count);
            assert_eq!(
                *table.service.updates.borrow(),
                current,
                "speech does not mutate game rules"
            );
        });
    }
}
