use super::*;
use crate::gameplay::{
    adventure::{RulesMode, record, shop},
    approach_model::JevUnavailable,
    beat_script_tests::{build, input},
    beat_script_variety_tests as native,
    social::{SocialLedger, fight_witness},
    startup_configuration::ContentRating,
    storyteller::tests::{FakeEndpoint, Reply, teller},
};
use df_observe::OperationContext;
use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
use df_session::submission::*;
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

#[derive(Clone, Eq, PartialEq)]
struct Scope(GameInput);
impl ActorInput for Scope {
    fn retained_heap_bytes(&self) -> Option<usize> {
        self.0.retained_heap_bytes()
    }
}
impl OperationScope for Scope {
    type UncertaintyKey = Self;
    fn capture_uncertainty_key(&self, maximum: usize) -> Result<Self, RepositoryError> {
        if self.0.retained_bytes().is_none_or(|bytes| bytes > maximum) {
            return Err(RepositoryError::Capacity);
        }
        Ok(self.clone())
    }
    fn session(&self) -> df_types::SessionId {
        let GameInput::Game(command) = &self.0 else {
            panic!("game input")
        };
        command.basis.session
    }
    fn operation(&self) -> OperationId {
        let GameInput::Game(command) = &self.0 else {
            panic!("game input")
        };
        command.operation
    }
    fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
        if self.0 == *input {
            Ok(())
        } else {
            Err(RepositoryError::InputBinding)
        }
    }
    fn is_lookup_only(&self) -> bool {
        false
    }
}
struct Database {
    current: Checkpoint,
    receipts: Vec<(Scope, DecisionReceipt)>,
}
struct Repository(Rc<RefCell<Database>>);
impl SessionRepository for Repository {
    type Scope = Scope;
    fn lookup_operation(
        &mut self,
        scope: &Scope,
        _: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        Ok(
            match self
                .0
                .borrow()
                .receipts
                .iter()
                .find(|(prior, _)| prior.operation() == scope.operation())
            {
                Some((prior, receipt)) if prior == scope => {
                    OperationLookup::Committed(receipt.clone())
                }
                Some(_) => OperationLookup::Conflict,
                None => OperationLookup::NotRecorded,
            },
        )
    }
    fn commit_decision(
        &mut self,
        scope: &Scope,
        candidate: &Checkpoint,
        expected: Basis,
        _: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        let mut database = self.0.borrow_mut();
        if database.current.basis() != expected {
            return Err(RepositoryError::RevisionConflict);
        }
        let receipt = DecisionReceipt::new(
            candidate.basis(),
            candidate.state().decisions.last().unwrap().clone(),
            256 * 1024,
        )?;
        database.current = candidate.clone();
        database.receipts.push((scope.clone(), receipt.clone()));
        Ok(CommitOutcome::Confirmed(receipt))
    }
    fn load_current(
        &mut self,
        _: &Scope,
        _: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        Ok(round_trip(&self.0.borrow().current))
    }
}
struct Engine;
impl SessionEngine<Scope> for Engine {
    fn decide(
        &mut self,
        current: &Checkpoint,
        _: &Scope,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        let mut opening_faces = [20, 19, 1, 1, 1, 1].into_iter();
        let opening = matches!(input, GameInput::Game(CommandInput { command: GameCommand::ProposeAction { action, .. }, .. }) if action.entry.as_str() == super::super::scene_fight::ACTION);
        let candidate = super::super::stage_with_supplier(current, input, &mut |sides| {
            Ok(if sides == 20 {
                if opening {
                    opening_faces.next().unwrap()
                } else {
                    15
                }
            } else {
                sides
            })
        })?;
        // This is the same complete-close hook used by actor::Engine, with supplied dice.
        stage_fight_completion(current, candidate)
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        crate::gameplay::beat_script::validate_resume(checkpoint)
            .map_err(|_| RepositoryError::InvalidCandidate)
    }
}
struct Publication;
impl PublicationOwner<Scope> for Publication {
    fn publish_committed(&mut self, _: &Scope, current: &Checkpoint) -> Result<(), DeliveryError> {
        publish(current).map_err(|_| DeliveryError::Unavailable)
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        Ok(())
    }
}
type Owner = DurableOwner<Repository, Engine, Publication>;
fn new_owner(current: Checkpoint) -> Owner {
    DurableOwner::new(
        Repository(Rc::new(RefCell::new(Database {
            current: current.clone(),
            receipts: Vec::new(),
        }))),
        Engine,
        Publication,
        current,
        256 * 1024,
    )
    .unwrap()
}
fn submit(owner: &mut Owner, request: rpc::SubmitActionRequest, member: MemberId) {
    let input = crate::gameplay::wire::journey_input(&request, member, owner.checkpoint())
        .unwrap_or_else(|error| {
            panic!(
                "wire request={request:?}: {error}; step={:?}",
                refusal_detail::take()
            )
        });
    submit_input(owner, input);
}

fn submit_input(owner: &mut Owner, input: GameInput) {
    let (item, wait) = OwnedInput::new(
        OperationContext {
            trace_parent: String::new(),
            build: "S2-aftermath".into(),
        },
        Scope(input.clone()),
        input,
    );
    owner.reduce(AdmissionSequence(0), item);
    let outcome = wait.try_recv().unwrap();
    assert!(
        matches!(outcome, SubmissionOutcome::Confirmed(_)),
        "{outcome:?}; step={:?}",
        refusal_detail::take()
    );
}
fn request(
    current: &Checkpoint,
    op: u8,
    kind: rpc::GameplayActionKind,
    choices: &[(&str, &str)],
) -> rpc::SubmitActionRequest {
    rpc::SubmitActionRequest {
        session_id: Some(rpc::SessionId {
            value: Some(current.basis().session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(current.basis().run.as_bytes().to_vec()),
        }),
        observed_revision: Some(crate::gameplay::wire::revision(current.basis().revision)),
        operation_id: Some(rpc::OperationId {
            value: Some(vec![op; 16]),
        }),
        action_kind: kind as i32,
        offer_id: if kind == rpc::GameplayActionKind::SceneAssault {
            super::super::scene_fight::offer_id(current)
        } else if kind == rpc::GameplayActionKind::ExamineHarborSeal {
            offer_id(current)
        } else {
            super::super::offer_id(current, kind)
        },
        action_choices: choices
            .iter()
            .map(|(group, option)| rpc::JourneyChoice {
                group_id: (*group).into(),
                option_id: (*option).into(),
            })
            .collect(),
        ..Default::default()
    }
}
fn round_trip(current: &Checkpoint) -> Checkpoint {
    let codec = df_persistence::NativeCodecLimits {
        maximum_document_bytes: 4 * 1024 * 1024,
        maximum_allocated_bytes: 8 * 1024 * 1024,
        maximum_collection_items: 4096,
        maximum_text_bytes: model::limits().maximum_text_bytes,
    };
    let bytes =
        df_persistence::local_demo_scope::encode_owned_demo_checkpoint(current, codec).unwrap();
    let recovered = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
        &bytes,
        current.basis(),
        current.pins(),
        &model::recovery().unwrap(),
        codec,
    )
    .unwrap();
    crate::gameplay::beat_script::validate_resume(&recovered).unwrap();
    assert_eq!(&recovered, current);
    recovered
}
struct FakeJev(std::sync::atomic::AtomicUsize);
impl fight_witness::Jev for FakeJev {
    fn reaction<'a>(
        &'a self,
        facts: &'a Value,
        choices: &'a [String],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<usize, JevUnavailable>> + Send + 'a>,
    > {
        assert_eq!(facts["observation"]["deaths"][0]["person_name"], "Jessa");
        // Brakka starts this fight, but Sigrun lands the accepted lethal blow.
        assert_eq!(facts["observation"]["deaths"][0]["killer_name"], "Sigrun");
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let choice = choices
            .iter()
            .position(|choice| choice.starts_with("Grieving;") && choice.ends_with("none"))
            .unwrap();
        Box::pin(async move { Ok(choice) })
    }
}
struct UnavailableJev;
impl fight_witness::Jev for UnavailableJev {
    fn reaction<'a>(
        &'a self,
        _: &'a Value,
        _: &'a [String],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<usize, JevUnavailable>> + Send + 'a>,
    > {
        Box::pin(async { Err(JevUnavailable::ProviderUnavailable) })
    }
}

#[test]
fn s2_jessa_fight_owner_journals_body_custody_witness_and_recovers_without_jev() {
    use rpc::GameplayActionKind as K;
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded, || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut story = super::super::initial().unwrap();
        for op in [1, 2] {
            story = super::super::stage_join(
                &story,
                &input(&story, bootstrap_member().unwrap(), op, "join-room", vec![]),
            )
            .unwrap();
        }
        for (op, member, hero) in [
            (3, native::first(), "Brakka"),
            (4, native::second(), "Sigrun"),
        ] {
            let mut choices = build(hero);
            choices.push((
                model::label("portrait").unwrap(),
                model::label(if member == native::first() {
                    "portrait-hero-1"
                } else {
                    "portrait-hero-2"
                })
                .unwrap(),
            ));
            if !crate::gameplay::beat_script::with_script(|script| script.is_canonical()) {
                choices.push((
                    model::label(crate::gameplay::beat_script_runtime::CHOICE_GROUP).unwrap(),
                    model::label("create-character").unwrap(),
                ));
            }
            story = super::super::stage_with_supplier(
                &story,
                &input(&story, member, op, "create-character", choices),
                &mut |_| panic!("creation dice"),
            )
            .unwrap_or_else(|error| panic!("hero {hero}: {error:?}; {:?}", refusal_detail::take()));
        }
        story = native::act(&story, native::first(), 5, "begin-story");
        let scene = crate::gameplay::beat_script_runtime::scene(story.state()).unwrap();
        let endpoint = FakeEndpoint::start(
            (0..3)
                .map(|_| {
                    Reply::Compute(Arc::new(|prompt| {
                        if prompt.contains("Jessa") {
                            json!({"items": "[\"plain clothes\",\"scuffed boots\"]"})
                        } else {
                            json!({"items": "[]"})
                        }
                    }))
                })
                .collect(),
        );
        let cache = std::env::temp_dir().join(format!("s2-identity-{}", std::process::id()));
        let storyteller = teller(endpoint.address, 20_000, Duration::from_secs(5), &cache);
        let preparation = runtime
            .block_on(prepare_fight(
                &story,
                OperationId::from_bytes(&[7; 16]).unwrap(),
                &storyteller,
            ))
            .unwrap();
        let prepared = commit_fight_preparation(
            &story,
            OperationId::from_bytes(&[6; 16]).unwrap(),
            preparation,
        )
        .unwrap();
        assert_eq!(endpoint.connections(), 3);
        runtime
            .block_on(prepare_fight(
                &round_trip(&prepared),
                OperationId::from_bytes(&[7; 16]).unwrap(),
                &storyteller,
            ))
            .unwrap();
        assert_eq!(
            endpoint.connections(),
            3,
            "accepted identity extraction is reused"
        );
        let mut owner = new_owner(prepared);
        let start = request(
            owner.checkpoint(),
            7,
            K::SceneAssault,
            &[
                ("scene-target", "shop-soldier"),
                ("scene-roster", "out:shop-adventurer/flee:merchant"),
            ],
        );
        submit(&mut owner, start, native::first());
        for op in 8..108 {
            let current = owner.checkpoint();
            if !super::super::scene_fight::open(current.state()) {
                break;
            }
            let member = [native::first(), native::second()]
                .into_iter()
                .find(|member| {
                    crate::gameplay::wire::current_combat_turn(current, *member).unwrap()
                })
                .unwrap();
            let battle = tactical::read(current.state()).unwrap();
            let attack = tactical::groups(current, member, K::GreatswordAttack)
                .unwrap()
                .first()
                .and_then(|group| group.options.first())
                .map(|option| option.option_id.clone());
            let foes = battle
                .combatants()
                .iter()
                .filter(|actor| {
                    actor.side() == df_rules::tactical::Side::Enemies
                        && !actor.health().incapacitated()
                })
                .map(|actor| actor.cell())
                .collect::<Vec<_>>();
            let movement = tactical::groups(current, member, K::Move)
                .unwrap()
                .first()
                .and_then(|group| {
                    group.options.iter().min_by_key(|option| {
                        let (x, y) = option.option_id.split_once('-').unwrap();
                        let cell = df_rules::tactical::Cell(x.parse().unwrap(), y.parse().unwrap());
                        foes.iter().map(|foe| cell.distance(*foe)).min()
                    })
                })
                .map(|option| option.option_id.clone());
            let (kind, choice) = if !battle.reaction_offers().is_empty() {
                (K::DeclineReaction, None)
            } else if attack.is_some() {
                (K::GreatswordAttack, attack)
            } else if movement.is_some() {
                (K::Move, movement)
            } else {
                (K::EndTurn, None)
            };
            let mut choices = choice
                .as_deref()
                .map(|choice| vec![("tactical-choice", choice)])
                .unwrap_or_default();
            if kind == K::GreatswordAttack {
                choices.push(("knock-out", "no"));
            }
            let action = request(current, op, kind, &choices);
            submit(&mut owner, action, member);
        }
        assert!(!super::super::scene_fight::open(owner.checkpoint().state()));
        let closed = round_trip(owner.checkpoint());
        assert_eq!(
            stage_fight_completion(&closed, closed.clone()).unwrap(),
            closed,
            "a closed-to-closed replay never stages a second outcome batch"
        );
        let now = snapshot(closed.state(), &scene).unwrap().unwrap();
        assert!(!now.model.people.iter().any(|person| person.name == "Marek"));
        let jessa = now
            .model
            .people
            .iter()
            .find(|person| person.name == "Jessa")
            .unwrap();
        assert_eq!(
            jessa.description,
            "Jessa lies dead beside the cloak rail on the right."
        );
        let social_directory =
            std::env::temp_dir().join(format!("s2-social-{}", std::process::id()));
        std::fs::create_dir_all(&social_directory).unwrap();
        let ledger = SocialLedger::open(
            generated.dossiers.clone(),
            "s2-owner",
            Some(social_directory.clone()),
        )
        .unwrap();
        assert!(
            runtime
                .block_on(resume_fight_reactions(
                    closed.state(),
                    &ledger,
                    &UnavailableJev
                ))
                .is_err()
        );
        assert!(
            ledger
                .fight_witness_record(&hex(&[7; 16]), "shop-adventurer")
                .is_none(),
            "an unavailable judgement never commits an unmoved reaction"
        );
        let jev = FakeJev(std::sync::atomic::AtomicUsize::new(0));
        assert_eq!(
            runtime
                .block_on(resume_fight_reactions(closed.state(), &ledger, &jev))
                .unwrap(),
            1
        );
        assert_eq!(jev.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let mut owner = new_owner(round_trip(&closed));
        let target = crate::gameplay::investigation::targets(&scene, &[])
            .into_iter()
            .find(|target| target.id == "person-shop-soldier")
            .unwrap();
        assert_eq!(
            crate::gameplay::investigation::fallen_search_target(closed.state(), &scene, &target)
                .unwrap()
                .as_deref(),
            Some("shop-soldier")
        );
        let draws = closed.state().draws.clone();
        let search = request(
            owner.checkpoint(),
            120,
            K::ExamineHarborSeal,
            &[
                ("scene-target", "shop-soldier"),
                ("scene-option", "search-body"),
            ],
        );
        let retained_search =
            crate::gameplay::wire::journey_input(&search, native::first(), owner.checkpoint())
                .unwrap();
        submit_input(&mut owner, retained_search.clone());
        let searched = round_trip(owner.checkpoint());
        assert_eq!(searched.state().draws, draws);
        let held = full_snapshot(searched.state(), &scene).unwrap().unwrap();
        for item in [
            "chain shirt",
            "shield",
            "spear",
            "plain clothes",
            "scuffed boots",
        ] {
            assert!(
                held.model.things.iter().any(|thing| thing.name == item
                    && thing.holder_id.as_deref() == Some("Brakka")
                    && thing.owner_id.as_deref() == Some("shop-soldier")),
                "{item}"
            );
        }
        assert!(
            held.model
                .people
                .iter()
                .any(|person| person.name == "Marek"),
            "full canonical identities retain the absent owner"
        );
        // The owner receives the exact admitted input again; it looks up the operation before
        // evaluating current offers. Raw old-offer decoding is a separate wire-owner request.
        submit_input(&mut owner, retained_search);
        assert_eq!(
            owner.checkpoint(),
            &searched,
            "lost-reply replay transfers nothing twice"
        );
        drop(ledger);
        let recovered_ledger = SocialLedger::open(
            generated.dossiers.clone(),
            "s2-owner",
            Some(social_directory),
        )
        .unwrap();
        let mut recovered_owner = new_owner(round_trip(&searched));
        assert_eq!(
            runtime
                .block_on(resume_fight_reactions(
                    recovered_owner.checkpoint().state(),
                    &recovered_ledger,
                    &UnavailableJev
                ))
                .unwrap(),
            0
        );
        let corin = generated
            .dossiers
            .iter()
            .find(|dossier| dossier.name == "Corin")
            .unwrap();
        let lines = recovered_ledger.fight_witness_lines(native::first(), corin);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Sigrun killed Jessa") && line.contains("grieving"))
        );
        recovered_ledger
            .install_rooms(&generated.settings, &generated.pack.scenes)
            .unwrap();
        recovered_ledger
            .sync_room(
                "Opening",
                snapshot(searched.state(), &scene).unwrap().unwrap(),
            )
            .unwrap();
        let reply_endpoint = FakeEndpoint::start(vec![Reply::Output(json!({
            "reply": "I saw her fall. Give me a moment.", "approach": "neutral", "local_fact": "none"
        }))]);
        let voice = crate::gameplay::npc_reply::NpcVoice::new(Arc::new(teller(
            reply_endpoint.address,
            20_000,
            Duration::from_secs(5),
            &std::env::temp_dir().join(format!("s2-reply-{}", std::process::id())),
        )));
        let view = crate::gameplay::npc_dossier_tests::view(&generated, scene.key, "Brakka");
        let context = crate::gameplay::npc_reply::NpcReplyContext::for_target(
            &view,
            native::first(),
            "I saw what happened.",
            &crate::gameplay::npc_reply::StoryFacts::from_pack(&generated.pack),
            &recovered_ledger,
            "Opening",
            Some("shop-adventurer"),
        )
        .unwrap();
        assert!(
            runtime
                .block_on(
                    voice.converse_in_character(
                        context,
                        &crate::gameplay::table_log::TableLog::new()
                    )
                )
                .is_some()
        );
        let bodies = reply_endpoint.bodies();
        let prompt = bodies.first().unwrap()["input"].as_str().unwrap();
        assert!(prompt.contains("witnessed_fights"));
        assert!(prompt.contains("Sigrun killed Jessa") && prompt.contains("grieving"));
        assert_eq!(
            snapshot(recovered_owner.checkpoint().state(), &scene)
                .unwrap()
                .unwrap(),
            snapshot(searched.state(), &scene).unwrap().unwrap()
        );
        let again = request(
            recovered_owner.checkpoint(),
            121,
            K::ExamineHarborSeal,
            &[
                ("scene-target", "shop-soldier"),
                ("scene-option", "search-body"),
            ],
        );
        submit(&mut recovered_owner, again, native::first());
        assert!(
            crate::gameplay::freeform_action::search_body(
                recovered_owner.checkpoint().state(),
                &held.model,
                "shop-soldier",
                "Brakka"
            )
            .unwrap()
            .is_empty()
        );
    });
}

#[test]
fn s2_room_projection_accepts_multi_body_inventory_and_keeps_the_full_change_history() {
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded, || {
        let current = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let base = snapshot(current.state(), &scene).unwrap().unwrap();
        let target = base.model.things.first().unwrap().id.clone();
        let changes = (0..80)
            .map(|index| SceneChange {
                entity_id: target.clone(),
                description: format!("The object was examined at step {index}."),
                kind: SceneChangeKind::Searched {
                    hero_id: "Brynn".into(),
                },
            })
            .collect();
        let entry = Interaction {
            place: scene.setting.as_ref().unwrap().place_id.clone(),
            changes,
            accepted: hex(&rpc::AcceptedAction {
                phase: phase(&current).unwrap() as i32,
                ..Default::default()
            }
            .encode_to_vec()),
            ..Interaction::default()
        };
        let candidate =
            finish_interaction(&current, OperationId::from_bytes(&[6; 16]).unwrap(), entry)
                .unwrap();
        let mut projected = snapshot(candidate.state(), &scene).unwrap().unwrap();
        assert_eq!(projected.changes.len(), 80);
        assert_eq!(scene_change_journal(&candidate).unwrap().len(), 80);
        let item = projected.model.things[0].clone();
        for index in 0..64 {
            let mut carried = item.clone();
            carried.id = format!("fight-gear-{index}");
            projected.model.things.push(carried);
        }
        let ledger = SocialLedger::open(generated.dossiers.clone(), "s2-bounds", None).unwrap();
        ledger
            .install_rooms(&generated.settings, &generated.pack.scenes)
            .unwrap();
        ledger.sync_room("Opening", projected.clone()).unwrap();
        assert_eq!(ledger.scene_now("Opening").unwrap(), projected);
        while projected.model.things.len() <= MAXIMUM_ROOM_THINGS {
            projected.model.things.push(item.clone());
        }
        assert!(
            ledger.sync_room("Opening", projected).is_err(),
            "overflow is explicit"
        );
        assert_eq!(
            ledger.scene_now("Opening").unwrap().changes.len(),
            80,
            "a refusal preserves the previous projection"
        );
    });
}

#[test]
fn s2_fight_outcome_is_not_overwritten_by_older_movement_prose() {
    use crate::gameplay::adventure::scene_model::SceneFightOutcome;
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded, || {
        let current = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let scene = crate::gameplay::beat_script_runtime::scene(current.state()).unwrap();
        let entry = Interaction {
            place: scene.setting.as_ref().unwrap().place_id.clone(),
            changes: vec![
                SceneChange {
                    entity_id: "shop-soldier".into(),
                    description: "Jessa returns to the cloak rail.".into(),
                    kind: SceneChangeKind::PersonMoved {
                        position: "beside the cloak rail on the right".into(),
                    },
                },
                SceneChange {
                    entity_id: "shop-soldier".into(),
                    description: "Jessa falls dead.".into(),
                    kind: SceneChangeKind::FightEnded {
                        outcome: SceneFightOutcome::Dead,
                        possessions: Vec::new(),
                    },
                },
            ],
            accepted: hex(&rpc::AcceptedAction {
                phase: phase(&current).unwrap() as i32,
                ..Default::default()
            }
            .encode_to_vec()),
            ..Interaction::default()
        };
        let candidate =
            finish_interaction(&current, OperationId::from_bytes(&[6; 16]).unwrap(), entry)
                .unwrap();
        let now = snapshot(candidate.state(), &scene).unwrap().unwrap();
        assert_eq!(
            now.model
                .people
                .iter()
                .find(|person| person.id == "shop-soldier")
                .unwrap()
                .description,
            "Jessa lies dead beside the cloak rail on the right."
        );
    });
}
