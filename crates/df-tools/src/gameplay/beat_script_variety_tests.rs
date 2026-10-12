use super::{
    beat_script_runtime as runtime,
    beat_script_tests::{build, input},
    journey, model, pack, wire,
};
use df_model::checkpoint::*;
use df_protocol::common as rpc;
use df_types::MemberId;
use prost::Message;
const PACK: &[u8] = include_bytes!("../../../../content/harbor-journey/pack.json");
const TUNING: &[u8] = include_bytes!("../../../../content/harbor-journey/tuning.json");
pub(super) fn first() -> MemberId {
    MemberId::from_bytes(&[0x61; 16]).unwrap()
}
pub(super) fn second() -> MemberId {
    MemberId::from_bytes(&[0x62; 16]).unwrap()
}
fn loaded(bytes: &[u8]) -> pack::LoadedPack {
    let mut content: pack::PackFile = serde_json::from_slice(PACK).unwrap();
    content.beat_script = Some(serde_json::from_slice(bytes).unwrap());
    pack::load_from_bytes(&serde_json::to_vec(&content).unwrap(), TUNING).unwrap()
}
pub(super) fn persisted(current: &Checkpoint) -> Checkpoint {
    let codec = df_persistence::NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 4096,
        maximum_text_bytes: 4096,
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
    super::beat_script::validate_resume(&recovered).unwrap();
    assert_eq!(&recovered, current);
    recovered
}
fn assert_campaign_completion(current: &Checkpoint, expected_member: MemberId, asked: bool) {
    use super::adventure::{campaign::*, campaign_tables::CampaignTables};
    let tables = CampaignTables::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/adventure-tables")
            .as_path(),
    )
    .unwrap();
    let frame = generate_frame(&tables, 1, super::adventure::GenerateOptions::default()).unwrap();
    let mut record = CampaignRecord::new(frame.clone()).unwrap();
    let plan = next_chapter(&tables, &frame, &record).unwrap();
    let leads = plan.leads.clone();
    let number = plan.number;
    record
        .begin(plan, "beat-script-run".into(), "a".repeat(64))
        .unwrap();
    let outcome = super::adventure::campaign_completion::terminal_outcome(current)
        .unwrap()
        .unwrap();
    assert_eq!(outcome.result, ChapterResult::Completed);
    assert_eq!(outcome.member, journey::hex(expected_member.as_bytes()));
    assert!(!outcome.rested && !outcome.antagonist_spared && outcome.destination_reached);
    assert_eq!(outcome.campaign_time, 0);
    assert_eq!(outcome.approach == Approach::Asked, asked);
    assert!(record.complete(number, outcome.clone()).unwrap());
    let once = record.clone();
    assert!(!record.complete(number, outcome).unwrap());
    assert_eq!(
        record.canonical_bytes().unwrap(),
        once.canonical_bytes().unwrap()
    );
    record.validate().unwrap();
    assert!(
        record
            .clock_history
            .iter()
            .all(|advance| advance.reason != "defeat")
    );
    assert!(record.chapters[0].lieutenant_fate.is_none());
    for lead in leads {
        let clue = record
            .clues
            .iter()
            .find(|clue| clue.clue_id == lead.clue_id)
            .unwrap();
        assert_eq!(
            clue.delivered,
            match clue.vehicle.as_str() {
                "private" => asked,
                "contact" => true,
                "victory" => false,
                _ => panic!("vehicle"),
            }
        );
        if asked && clue.vehicle == "private" {
            assert_eq!(
                clue.audience,
                Audience::Private(journey::hex(expected_member.as_bytes()))
            );
        }
    }
    let next = next_chapter(&tables, &frame, &record).unwrap();
    assert!(next.number > number);
    let recap = next.recap;
    assert!(
        serde_json::to_string(&recap)
            .unwrap()
            .contains("resolved the chapter")
    );
}
fn request(
    current: &Checkpoint,
    member: MemberId,
    operation: u8,
    id: &str,
) -> rpc::SubmitActionRequest {
    let selected = super::beat_script::with_script(|script| {
        script
            .current(current.state())
            .unwrap()
            .offers
            .iter()
            .find(|offer| offer.id == id)
            .unwrap()
            .clone()
    });
    let (kind, _, offer_id) = runtime::view_offers(current, member)
        .unwrap()
        .into_iter()
        .find(|(kind, _, opaque)| {
            runtime::submitted_id(opaque) == Some(id)
                || (super::beat_script::with_script(|script| script.is_canonical())
                    && selected.effect.action() == *kind)
        })
        .unwrap();
    let basis = current.basis();
    rpc::SubmitActionRequest {
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        observed_revision: Some(wire::revision(basis.revision)),
        operation_id: Some(rpc::OperationId {
            value: Some(vec![operation; 16]),
        }),
        offer_id,
        action_kind: kind as i32,
        destination: if kind == rpc::GameplayActionKind::ChooseHarborScene {
            rpc::HarborDestination::HarborInn as i32
        } else {
            0
        },
        character: if kind == rpc::GameplayActionKind::CreateCharacter {
            Some(rpc::CharacterSelection {
                name: if member == first() { "Brynn" } else { "Vale" }.into(),
                choices: build(if member == first() { "Brynn" } else { "Vale" })
                    .into_iter()
                    .filter(|(group, _)| group.as_str() != "name")
                    .map(|(group, option)| rpc::JourneyChoice {
                        group_id: group.as_str().into(),
                        option_id: option.as_str().into(),
                    })
                    .collect(),
            })
        } else {
            None
        },
        savage_attacker: false,
        graze: false,
    }
}
pub(super) fn act(current: &Checkpoint, member: MemberId, operation: u8, id: &str) -> Checkpoint {
    let raw = request(current, member, operation, id);
    assert!(
        runtime::matches_id(
            current,
            rpc::GameplayActionKind::try_from(raw.action_kind).unwrap(),
            &raw.offer_id
        )
        .unwrap()
    );
    let raw = rpc::SubmitActionRequest::decode(raw.encode_to_vec().as_slice()).unwrap();
    let input = wire::journey_input(&raw, member, current).unwrap();
    journey::stage_with_supplier(current, &input, &mut |sides| Ok(sides))
        .unwrap_or_else(|error| panic!("action {id} operation {operation}: {error:?}"))
}
pub(super) fn prepared() -> Checkpoint {
    let mut current = journey::initial().unwrap();
    for operation in [1, 2] {
        current = journey::stage_join(
            &current,
            &input(
                &current,
                journey::bootstrap_member().unwrap(),
                operation,
                "join-room",
                vec![],
            ),
        )
        .unwrap();
    }
    current = act(&current, first(), 3, "create-character");
    act(&current, second(), 4, "create-character")
}
fn narration(current: &Checkpoint) -> String {
    let view = wire::journey_view(
        current,
        df_persistence::local_demo_scope::LocalDemoRole::Display,
        journey::bootstrap_member().unwrap(),
    )
    .unwrap();
    match view.audience.unwrap() {
        rpc::view_message::Audience::Display(view) => view.narration,
        _ => panic!("display"),
    }
}
#[test]
fn three_authored_approaches_have_distinct_public_narration_and_reach_the_same_real_encounter() {
    pack::with_test_pack(
        loaded(include_bytes!("fixtures/third-approach-beat-script.json")),
        || {
            let opening = act(&prepared(), first(), 5, "begin-story");
            let mut public = std::collections::BTreeSet::new();
            for id in ["ask-courier", "escort-courier", "third-approach"] {
                let dialogue = act(&opening, first(), 6, id);
                public.insert(narration(&dialogue));
                let combat = act(&dialogue, first(), 7, "defend-courier");
                assert_eq!(journey::phase(&combat).unwrap(), rpc::JourneyPhase::Combat);
                assert_eq!(combat.state().encounters[0].participants.len(), 3);
                assert_eq!(combat.state().draws.len(), 3);
            }
            assert_eq!(public.len(), 3);
        },
    );
}
#[test]
fn no_fight_script_completes_by_native_actions_without_encounter_dice_or_time() {
    pack::with_test_pack(
        loaded(include_bytes!("fixtures/no-fight-beat-script.json")),
        || {
            let mut current = prepared();
            for (operation, id) in [
                (5, "begin-story"),
                (6, "escort-courier"),
                (7, "continue-conversation"),
                (8, "choose-harbor-scene"),
            ] {
                current = act(&current, first(), operation, id);
                assert!(current.state().encounters.is_empty());
                assert!(current.state().draws.is_empty());
                assert_eq!(current.state().logical_time.ticks, 0);
            }
            assert_eq!(
                journey::phase(&current).unwrap(),
                rpc::JourneyPhase::Complete
            );
            assert!(runtime::view_offers(&current, first()).unwrap().is_empty());
            assert_campaign_completion(&persisted(&current), first(), false);
        },
    );
}
#[test]
fn conversation_branch_reaches_two_distinct_authored_destination_views() {
    pack::with_test_pack(
        loaded(include_bytes!("fixtures/branch-beat-script.json")),
        || {
            let opening = act(&prepared(), first(), 5, "begin-story");
            let mut destinations = std::collections::BTreeSet::new();
            for (approach, expected) in [
                ("ask-courier", "East Destination"),
                ("escort-courier", "West Destination"),
            ] {
                let dialogue = act(&opening, first(), 6, approach);
                let travel = act(&dialogue, first(), 7, "continue-conversation");
                let destination = act(&persisted(&travel), second(), 8, "choose-harbor-scene");
                let destination = persisted(&destination);
                let scene = runtime::scene(destination.state()).unwrap();
                assert_eq!(scene.title, expected);
                destinations.insert(narration(&destination));
                let view = wire::journey_view(
                    &destination,
                    df_persistence::local_demo_scope::LocalDemoRole::Player,
                    first(),
                )
                .unwrap();
                let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
                    panic!("player")
                };
                assert_eq!(player.scene.unwrap().title, expected);
                assert!(destination.state().encounters.is_empty());
            }
            assert_eq!(destinations.len(), 2);
        },
    );
}
#[test]
fn repeated_look_choices_share_a_proto_kind_but_keep_exact_ids_replay_and_member_private_clue() {
    pack::with_test_pack(
        loaded(include_bytes!("fixtures/investigate-beat-script.json")),
        || {
            let mut current = act(&prepared(), first(), 5, "begin-story");
            let options = runtime::view_offers(&current, first()).unwrap();
            let looks = options
                .iter()
                .filter(|(kind, _, _)| *kind == rpc::GameplayActionKind::ExamineHarborSeal)
                .collect::<Vec<_>>();
            assert_eq!(looks.len(), 2);
            assert_ne!(looks[0].2, looks[1].2);
            let stale = request(&current, first(), 6, "look-crates");
            let proposed = wire::journey_input(&stale, first(), &current).unwrap();
            let replay = journey::stage_with_supplier(&current, &proposed, &mut |_| {
                panic!("observation never draws")
            })
            .unwrap();
            assert_eq!(
                replay,
                journey::stage_with_supplier(&current, &proposed, &mut |_| panic!(
                    "replay never draws"
                ))
                .unwrap()
            );
            current = replay;
            assert!(
                !runtime::matches_id(
                    &current,
                    rpc::GameplayActionKind::ExamineHarborSeal,
                    &stale.offer_id
                )
                .unwrap()
            );
            let completed = current.state().narrative.completed_beats.clone();
            current = act(&current, first(), 7, "look-crates");
            assert_eq!(current.state().narrative.completed_beats, completed);
            current = act(&current, first(), 8, "look-desk");
            assert!(narration(&current).contains("shipping list"));
            current = act(&current, second(), 9, "ask-courier");
            let mut wrong_holder = current.state().clone();
            wrong_holder.facts.iter_mut().find(|fact| matches!(&fact.value, FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "private-courier-note")).unwrap()
                .audience = AudienceScope::Members(vec![first()]);
            assert!(
                journey::phase(&model::checkpoint(current.basis(), wrong_holder).unwrap()).is_err()
            );
            let effect = current.state().intents.first().unwrap().clone();
            let result = super::courier_ai::execute(&current, &effect).unwrap();
            current = super::courier_ai::stage_completion(
                &current,
                &result,
                super::courier_ai::completion_operation(&effect).unwrap(),
            )
            .unwrap();
            for member in [first(), second()] {
                let view = wire::journey_view(
                    &current,
                    df_persistence::local_demo_scope::LocalDemoRole::Player,
                    member,
                )
                .unwrap();
                let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
                    panic!("player")
                };
                assert_eq!(!player.private_clue.is_empty(), member == second());
            }
            assert!(
                !narration(&current)
                    .contains(&pack::with_content(|content| content.courier_clue.clone()))
            );
            current = act(&current, second(), 10, "continue-conversation");
            current = act(&current, second(), 11, "choose-harbor-scene");
            assert!(current.state().encounters.is_empty());
            assert_campaign_completion(&persisted(&current), second(), true);
        },
    );
}

#[test]
fn investigation_after_begin_story_is_admitted_through_the_engine_entry() {
    // The live server admits through `HarborEngine::decide_admitted`, not `act`'s direct
    // staging call. An investigation carries the `script-choice` entry and must reach the
    // registered journey even after decisions exist.
    pack::with_test_pack(
        loaded(include_bytes!("fixtures/investigate-beat-script.json")),
        || {
            let mut current = act(&prepared(), first(), 5, "begin-story");
            for (operation, id, text) in [
                (6, "look-crates", "Fresh rope binds the crates"),
                (7, "look-desk", "shipping list"),
            ] {
                let raw = request(&current, first(), operation, id);
                let input = wire::journey_input(&raw, first(), &current).unwrap();
                let GameInput::Game(command) = &input else {
                    panic!("game input")
                };
                let staged = model::HarborEngine
                    .decide_admitted(&current, &input, command.operation)
                    .unwrap_or_else(|error| panic!("look {id}: {error:?}"));
                assert!(narration(&staged).contains(text));
                current = staged;
            }
        },
    );
}
#[test]
fn player_choice_cycle_revisits_two_durable_beats_and_resumes_without_duplicate_completion() {
    use super::beat_script::*;
    let mut fixture: BeatScript =
        serde_json::from_slice(include_bytes!("fixtures/no-fight-beat-script.json")).unwrap();
    fixture
        .labels
        .insert("look-around".into(), "Look around".into());
    let original = fixture.find("peaceful-travel").unwrap().clone();
    let mut exit = original.clone();
    exit.id = "cycle-exit".into();
    exit.state.durable = "cycle-exit".into();
    let look = BeatOffer {
        id: "look-around".into(),
        label_slot: "look-around".into(),
        effect: BeatEffect::Observe,
    };
    let leave = BeatOffer {
        id: "leave-cycle".into(),
        label_slot: "continue-label".into(),
        effect: BeatEffect::Continue,
    };
    let first_beat = fixture
        .beats
        .iter_mut()
        .find(|beat| beat.id == original.id)
        .unwrap();
    first_beat.kind = BeatKind::Investigate;
    first_beat.offers = vec![look.clone(), leave.clone()];
    first_beat.transitions = vec![
        BeatTransition {
            trigger: BeatTrigger::Offer {
                offer: look.id.clone(),
            },
            target: "second-observation".into(),
        },
        BeatTransition {
            trigger: BeatTrigger::Offer {
                offer: leave.id.clone(),
            },
            target: exit.id.clone(),
        },
    ];
    let mut second_beat = first_beat.clone();
    second_beat.id = "second-observation".into();
    second_beat.state.durable = second_beat.id.clone();
    second_beat.transitions[0].target = original.id;
    fixture.beats.extend([second_beat, exit]);
    pack::with_test_pack(loaded(&serde_json::to_vec(&fixture).unwrap()), || {
        let mut current = prepared();
        for (operation, id) in [
            (5, "begin-story"),
            (6, "escort-courier"),
            (7, "continue-conversation"),
            (8, "look-around"),
            (9, "look-around"),
            (10, "look-around"),
        ] {
            current = act(&current, first(), operation, id);
            current = persisted(&current);
        }
        let completed = &current.state().narrative.completed_beats;
        assert_eq!(
            completed.len(),
            completed
                .iter()
                .map(|beat| beat.entry.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );
        current = act(&current, first(), 11, "leave-cycle");
        current = act(&current, second(), 12, "choose-harbor-scene");
        assert_campaign_completion(&persisted(&current), first(), false);
    });
}

// Controlled durable transaction/publication ports exercise the session owner without
// replacing the registered native action path or requiring a physical PostgreSQL server.
mod session {
    use super::*;
    use df_observe::OperationContext;
    use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
    use df_session::submission::*;
    use df_types::{OperationId, SessionId};
    use std::{cell::RefCell, rc::Rc};
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
        fn session(&self) -> SessionId {
            let GameInput::Game(command) = &self.0 else {
                panic!("fixture command")
            };
            command.basis.session
        }
        fn operation(&self) -> OperationId {
            let GameInput::Game(command) = &self.0 else {
                panic!("fixture command")
            };
            command.operation
        }
        fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
            if &self.0 == input {
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
        ledger: Vec<(Scope, DecisionReceipt)>,
        decisions: usize,
        publications: usize,
    }
    struct Repository(Rc<RefCell<Database>>);
    impl SessionRepository for Repository {
        type Scope = Scope;
        fn lookup_operation(
            &mut self,
            scope: &Scope,
            _: &OperationContext,
        ) -> Result<OperationLookup, RepositoryError> {
            let database = self.0.borrow();
            Ok(
                match database
                    .ledger
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
            if expected != database.current.basis() {
                return Err(RepositoryError::RevisionConflict);
            }
            let decision = candidate
                .state()
                .decisions
                .iter()
                .find(|decision| decision.operation == scope.operation())
                .unwrap()
                .clone();
            let receipt = DecisionReceipt::new(candidate.basis(), decision, 64 * 1024)?;
            database.current = candidate.clone();
            database.ledger.push((scope.clone(), receipt));
            // The durable transaction committed, but its acknowledgement was lost.
            Ok(CommitOutcome::Indeterminate)
        }
        fn load_current(
            &mut self,
            _: &Scope,
            _: &OperationContext,
        ) -> Result<Checkpoint, RepositoryError> {
            Ok(persisted(&self.0.borrow().current))
        }
    }
    struct Engine(Rc<RefCell<Database>>);
    impl SessionEngine<Scope> for Engine {
        fn decide(
            &mut self,
            current: &Checkpoint,
            _: &Scope,
            input: &GameInput,
        ) -> Result<Checkpoint, RepositoryError> {
            self.0.borrow_mut().decisions += 1;
            journey::stage_with_supplier(current, input, &mut |_| panic!("observation cannot draw"))
        }
        fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
            journey::phase(checkpoint).map(|_| ())
        }
    }
    struct Publication(Rc<RefCell<Database>>);
    impl PublicationOwner<Scope> for Publication {
        fn publish_committed(
            &mut self,
            _: &Scope,
            current: &Checkpoint,
        ) -> Result<(), DeliveryError> {
            let mut database = self.0.borrow_mut();
            assert_eq!(&database.current, current);
            database.publications += 1;
            Ok(())
        }
        fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
            Ok(())
        }
    }
    type Owner = DurableOwner<Repository, Engine, Publication>;
    fn submit(owner: &mut Owner, scope: &Scope) -> SubmissionOutcome {
        let (input, wait) = OwnedInput::new(
            OperationContext {
                trace_parent: String::new(),
                build: "beat-script-native-session".into(),
            },
            scope.clone(),
            scope.0.clone(),
        );
        owner.reduce(AdmissionSequence(1), input);
        wait.try_recv().unwrap()
    }
    #[test]
    fn lost_ack_retry_and_collision_preserve_the_exact_second_look_choice_once() {
        pack::with_test_pack(
            loaded(include_bytes!("fixtures/investigate-beat-script.json")),
            || {
                let current = act(&prepared(), first(), 5, "begin-story");
                let raw = request(&current, first(), 6, "look-desk");
                let scope = Scope(wire::journey_input(&raw, first(), &current).unwrap());
                let database = Rc::new(RefCell::new(Database {
                    current: current.clone(),
                    ledger: vec![],
                    decisions: 0,
                    publications: 0,
                }));
                let mut owner = DurableOwner::new(
                    Repository(database.clone()),
                    Engine(database.clone()),
                    Publication(database.clone()),
                    current.clone(),
                    64 * 1024,
                )
                .unwrap();
                assert!(matches!(
                    submit(&mut owner, &scope),
                    SubmissionOutcome::LookupRequired
                ));
                let confirmed = submit(&mut owner, &scope);
                assert!(
                    matches!(confirmed, SubmissionOutcome::Confirmed(_)),
                    "{confirmed:?}"
                );
                assert_eq!(submit(&mut owner, &scope), confirmed);
                assert_eq!(database.borrow().decisions, 1);
                assert_eq!(database.borrow().ledger.len(), 1);
                assert_eq!(database.borrow().publications, 0);
                assert!(owner.is_current());
                assert!(narration(owner.checkpoint()).contains("shipping list"));
                let collision = Scope(
                    wire::journey_input(
                        &request(&current, super::first(), 6, "look-crates"),
                        super::first(),
                        &current,
                    )
                    .unwrap(),
                );
                assert!(matches!(
                    submit(&mut owner, &collision),
                    SubmissionOutcome::OperationConflict
                ));
                assert_eq!(database.borrow().decisions, 1);
                let stale = Scope(
                    wire::journey_input(
                        &request(&current, super::first(), 7, "look-crates"),
                        super::first(),
                        &current,
                    )
                    .unwrap(),
                );
                assert!(matches!(
                    submit(&mut owner, &stale),
                    SubmissionOutcome::Refused(_)
                ));
                assert_eq!(database.borrow().ledger.len(), 1);
            },
        );
    }
}

#[test]
fn script_owned_text_offer_identity_and_native_character_provenance_are_validated() {
    use super::beat_script::*;
    let base: BeatScript =
        serde_json::from_slice(include_bytes!("fixtures/investigate-beat-script.json")).unwrap();
    let content: pack::PackFile = serde_json::from_slice(PACK).unwrap();
    let mut duplicate = base.clone();
    duplicate
        .beats
        .iter_mut()
        .find(|beat| beat.id == "observed-desk")
        .unwrap()
        .offers[0]
        .label_slot = "look-desk-label".into();
    assert_eq!(
        duplicate.validate(&content).unwrap_err().reason,
        ScriptErrorReason::DuplicateOffer
    );
    let mut text = base.clone();
    text.labels
        .insert("look-desk-label".into(), "Look\nthrough the desk".into());
    assert!(text.validate(&content).is_err());
    let mut long = base.clone();
    long.beats[1].offers[0].id = "x".repeat(47);
    assert_eq!(
        long.validate(&content).unwrap_err().reason,
        ScriptErrorReason::Bounds
    );
    pack::with_test_pack(loaded(&serde_json::to_vec(&base).unwrap()), || {
        let current = act(&prepared(), first(), 5, "begin-story");
        for damage in 0..4 {
            let mut state = current.state().clone();
            match damage {
                0 => state.characters[0].owner = second(),
                1 => {
                    state
                        .resources
                        .iter_mut()
                        .find(|resource| resource.resource.as_str() == "strength")
                        .unwrap()
                        .value -= 1
                }
                2 => state.inventory[0].quantity += 1,
                3 => {
                    let decision = state
                        .decisions
                        .iter_mut()
                        .find(|decision| {
                            journey::accepted(decision)
                                .is_ok_and(|action| action.character.is_some())
                        })
                        .unwrap();
                    let mut accepted = journey::accepted(decision).unwrap();
                    accepted.character.as_mut().unwrap().name = "Forged creation receipt".into();
                    decision.semantic_output = Some(journey::hex(&accepted.encode_to_vec()));
                }
                _ => unreachable!(),
            }
            match model::checkpoint(current.basis(), state) {
                Ok(forged) => assert!(journey::phase(&forged).is_err(), "forgery {damage}"),
                Err(error) => assert_eq!(
                    error,
                    df_session::submission::RepositoryError::InvalidCandidate
                ),
            }
        }
    });
}

#[test]
fn impossible_choice_predicates_rest_and_native_prefix_reentry_refuse_at_load() {
    let fixtures: std::collections::BTreeMap<String, super::beat_script::BeatScript> =
        serde_json::from_slice(include_bytes!("fixtures/invalid-variety-beat-scripts.json"))
            .unwrap();
    let content: pack::PackFile = serde_json::from_slice(PACK).unwrap();
    assert_eq!(fixtures.len(), 5);
    for (name, mut script) in fixtures {
        let error = script.validate(&content).unwrap_err();
        assert!(!error.beat.is_empty(), "{name}");
        assert!(
            matches!(
                error.reason,
                super::beat_script::ScriptErrorReason::HandlerWiring
                    | super::beat_script::ScriptErrorReason::RestWiring
                    | super::beat_script::ScriptErrorReason::UnsupportedEffect
            ),
            "{name}: {error}"
        );
        if name == "public-look-returns-to-one-shot-gather"
            || name == "return-opening-repeats-private-source"
        {
            // Also reach the prefix validator with a supported generic handler.
            let beat = script
                .beats
                .iter_mut()
                .find(|beat| beat.id == "peaceful-travel")
                .unwrap();
            beat.kind = super::beat_script::BeatKind::Investigate;
            for offer in &mut beat.offers {
                if offer.effect == super::beat_script::BeatEffect::ChooseHarborScene {
                    offer.effect = super::beat_script::BeatEffect::Continue;
                }
            }
            assert_eq!(
                script.validate(&content).unwrap_err().reason,
                super::beat_script::ScriptErrorReason::HandlerWiring
            );
        }
    }
    let mut fight: super::beat_script::BeatScript =
        serde_json::from_slice(include_bytes!("fixtures/third-approach-beat-script.json")).unwrap();
    fight
        .beats
        .iter_mut()
        .find(|beat| beat.id == "victory")
        .unwrap()
        .state
        .chosen_offer = Some("greatsword-attack".into());
    assert!(fight.validate(&content).is_err());
    for variant in [false, true] {
        let mut destination = super::beat_script::canonical().clone();
        if variant {
            destination.scenes.insert(
                "alternate-inn".into(),
                content
                    .scenes
                    .iter()
                    .find(|scene| scene.key == pack::SceneKey::Inn)
                    .unwrap()
                    .clone(),
            );
        }
        let inn = destination
            .beats
            .iter_mut()
            .find(|beat| beat.id == "harbor-inn")
            .unwrap();
        if variant {
            inn.scene_variant = Some("alternate-inn".into());
        } else {
            inn.scene = pack::SceneKey::Opening;
        }
        assert_eq!(
            destination.validate(&content).unwrap_err().reason,
            super::beat_script::ScriptErrorReason::HandlerWiring
        );
    }
}
#[test]
fn authored_destination_on_legacy_inn_durable_id_keeps_its_scene_and_receipt() {
    let mut script: super::beat_script::BeatScript =
        serde_json::from_slice(include_bytes!("fixtures/no-fight-beat-script.json")).unwrap();
    let content: pack::PackFile = serde_json::from_slice(PACK).unwrap();
    let mut scene = content
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Inn)
        .unwrap()
        .clone();
    scene.title = "Authored Arrival".into();
    script.scenes.insert("authored-arrival".into(), scene);
    let destination = script
        .beats
        .iter_mut()
        .find(|beat| beat.id == "peaceful-destination")
        .unwrap();
    destination.state.durable = "harbor-inn".into();
    destination.state.chosen_offer = Some("choose-harbor-scene".into());
    destination.scene_variant = Some("authored-arrival".into());
    pack::with_test_pack(loaded(&serde_json::to_vec(&script).unwrap()), || {
        let mut current = prepared();
        for (operation, id) in [
            (5, "begin-story"),
            (6, "escort-courier"),
            (7, "continue-conversation"),
            (8, "choose-harbor-scene"),
        ] {
            current = act(&current, first(), operation, id);
        }
        let current = persisted(&current);
        let view = wire::journey_view(
            &current,
            df_persistence::local_demo_scope::LocalDemoRole::Player,
            first(),
        )
        .unwrap();
        let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
            panic!("player")
        };
        assert_eq!(player.scene.unwrap().title, "Authored Arrival");
        assert_eq!(
            journey::accepted(current.state().decisions.last().unwrap())
                .unwrap()
                .scene
                .unwrap()
                .title,
            "Authored Arrival"
        );
    });
}
