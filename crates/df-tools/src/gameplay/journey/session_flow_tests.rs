use super::*;
use crate::gameplay::{actor, courier_ai};
use df_model::transition::TransitionResult;
use df_observe::OperationContext;
use df_persistence::local_demo_scope::{DISPLAY, LocalDemoRole};
use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
use df_session::submission::*;
use df_types::SessionId;
use std::{cell::RefCell, rc::Rc};
use tokio::sync::watch;

// These controlled transaction ports exercise DurableOwner and the registered native
// journey. They do not qualify physical storage, credentials, or a broader rules catalog.
#[derive(Clone, Eq, PartialEq)]
struct Scope {
    session: SessionId,
    operation: OperationId,
    input: GameInput,
}
impl ActorInput for Scope {
    fn retained_heap_bytes(&self) -> Option<usize> {
        self.input.retained_heap_bytes()
    }
}
impl OperationScope for Scope {
    type UncertaintyKey = Self;
    fn capture_uncertainty_key(&self, maximum: usize) -> Result<Self, RepositoryError> {
        if self
            .input
            .retained_bytes()
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
            .is_none_or(|bytes| bytes > maximum)
        {
            return Err(RepositoryError::Capacity);
        }
        Ok(self.clone())
    }
    fn session(&self) -> SessionId {
        self.session
    }
    fn operation(&self) -> OperationId {
        self.operation
    }
    fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
        if &self.input == input {
            Ok(())
        } else {
            Err(RepositoryError::InputBinding)
        }
    }
    fn is_lookup_only(&self) -> bool {
        false
    }
}
impl Scope {
    fn command(input: GameInput) -> Self {
        let GameInput::Game(command) = &input else {
            panic!("game command")
        };
        Self {
            session: command.basis.session,
            operation: command.operation,
            input,
        }
    }
}

struct Database {
    checkpoint: Checkpoint,
    ledger: Vec<(Scope, DecisionReceipt)>,
    refuse_commit: bool,
    damage: u32,
    decisions: usize,
    commits: usize,
    publications: usize,
    wakes: usize,
    samples: usize,
}
struct Repository(Rc<RefCell<Database>>);
impl SessionRepository for Repository {
    type Scope = Scope;
    fn lookup_operation(
        &mut self,
        scope: &Scope,
        _: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        let db = self.0.borrow();
        Ok(
            match db
                .ledger
                .iter()
                .find(|(prior, _)| prior.operation == scope.operation)
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
        let mut db = self.0.borrow_mut();
        db.commits += 1;
        if expected != db.checkpoint.basis() {
            return Err(RepositoryError::RevisionConflict);
        }
        if db.refuse_commit {
            return Err(RepositoryError::Unavailable);
        }
        let decision = candidate
            .state()
            .decisions
            .iter()
            .find(|decision| decision.operation == scope.operation)
            .unwrap()
            .clone();
        let receipt = DecisionReceipt::new(candidate.basis(), decision, 64 * 1024)?;
        db.checkpoint = candidate.clone();
        db.ledger.push((scope.clone(), receipt.clone()));
        Ok(CommitOutcome::Confirmed(receipt))
    }
    fn load_current(
        &mut self,
        _: &Scope,
        _: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        Ok(self.0.borrow().checkpoint.clone())
    }
}
struct Engine(Rc<RefCell<Database>>);
impl SessionEngine<Scope> for Engine {
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &Scope,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        scope.validate_input(input)?;
        self.0.borrow_mut().decisions += 1;
        if let GameInput::Job(completion) = input {
            let intent = current
                .state()
                .intents
                .iter()
                .find(|intent| intent.job == Some(completion.job))
                .ok_or(RepositoryError::InvalidCandidate)?;
            let admitted = courier_ai::execute(current, intent)
                .map_err(|_| RepositoryError::InvalidCandidate)?;
            if &admitted != completion {
                return Err(RepositoryError::InputBinding);
            }
            return courier_ai::stage_completion(current, completion, scope.operation)
                .map_err(|_| RepositoryError::InvalidCandidate);
        }
        if matches!(input, GameInput::Game(CommandInput {
            command: GameCommand::ProposeAction { action, .. }, ..
        }) if action.entry.as_str() == "join-room")
        {
            return stage_join(current, input);
        }
        // Reuse the legal supplied faces of narrative_runtime_tests::victory. The
        // registered handler consumes the recorded draws; no random/provider path runs.
        stage_with_supplier(current, input, &mut |sides| {
            // d6 damage, the d10 Second Wind die, and d20 checks.
            assert!(matches!(sides, 6 | 10 | 20));
            let mut db = self.0.borrow_mut();
            db.samples += 1;
            Ok(if sides == 20 { 10 } else { db.damage })
        })
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        phase(checkpoint)?;
        Ok(())
    }
}
struct Publication {
    database: Rc<RefCell<Database>>,
    native: actor::Publication,
}
impl PublicationOwner<Scope> for Publication {
    fn publish_committed(
        &mut self,
        _: &Scope,
        checkpoint: &Checkpoint,
    ) -> Result<(), DeliveryError> {
        let mut db = self.database.borrow_mut();
        assert_eq!(checkpoint, &db.checkpoint);
        db.publications += 1;
        self.native.publish(checkpoint);
        Ok(())
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        self.native.wake()?;
        self.database.borrow_mut().wakes += 1;
        Ok(())
    }
}
type Owner = DurableOwner<Repository, Engine, Publication>;
struct Runtime {
    owner: Owner,
    database: Rc<RefCell<Database>>,
    published: watch::Receiver<Checkpoint>,
    notifications: std::sync::mpsc::Receiver<()>,
}
impl Runtime {
    fn restore(database: Rc<RefCell<Database>>) -> Self {
        let checkpoint = database.borrow().checkpoint.clone();
        let (updates, published) = watch::channel(checkpoint.clone());
        let (native, notifications) = actor::Publication::new(updates);
        let owner = DurableOwner::new(
            Repository(Rc::clone(&database)),
            Engine(Rc::clone(&database)),
            Publication {
                database: Rc::clone(&database),
                native,
            },
            checkpoint,
            64 * 1024,
        )
        .unwrap();
        Self {
            owner,
            database,
            published,
            notifications,
        }
    }
    fn submit(&mut self, scope: &Scope) -> SubmissionOutcome {
        let (input, wait) = OwnedInput::new(
            OperationContext {
                trace_parent: String::new(),
                build: "integrated-native-session-flow".to_owned(),
            },
            scope.clone(),
            scope.input.clone(),
        );
        self.owner.reduce(AdmissionSequence(1), input);
        wait.try_recv().unwrap()
    }
    fn accept(&mut self, scope: &Scope) -> SubmissionOutcome {
        let before = self.owner.checkpoint().clone();
        let samples = self.database.borrow().samples;
        let outcome = self.submit(scope);
        assert!(matches!(outcome, SubmissionOutcome::Confirmed(_)));
        let checkpoint = self.owner.checkpoint();
        assert_eq!(&*self.published.borrow(), checkpoint);
        assert_eq!(&self.database.borrow().checkpoint, checkpoint);
        assert_eq!(
            &checkpoint.state().facts[..before.state().facts.len()],
            before.state().facts
        );
        assert_eq!(
            &checkpoint.state().decisions[..before.state().decisions.len()],
            before.state().decisions
        );
        assert_eq!(
            &checkpoint.state().draws[..before.state().draws.len()],
            before.state().draws
        );
        assert_eq!(
            checkpoint.state().draws.len() - before.state().draws.len(),
            self.database.borrow().samples - samples
        );
        assert!(self.notifications.try_recv().is_ok());
        let stable = self.owner.checkpoint().clone();
        let counts = activity(&self.database.borrow());
        assert_eq!(self.submit(scope), outcome);
        assert_eq!(self.owner.checkpoint(), &stable);
        assert_eq!(activity(&self.database.borrow()), counts);
        assert!(self.notifications.try_recv().is_err());
        outcome
    }
}
fn activity(db: &Database) -> (usize, usize, usize, usize, usize, usize) {
    (
        db.decisions,
        db.commits,
        db.publications,
        db.wakes,
        db.samples,
        db.ledger.len(),
    )
}
fn member(index: usize) -> MemberId {
    MemberId::from_bytes(&MEMBERS[index]).unwrap()
}
fn views(current: &Checkpoint) -> [Vec<u8>; 3] {
    [
        wire::journey_view(current, LocalDemoRole::Player, member(0))
            .unwrap()
            .encode_to_vec(),
        wire::journey_view(current, LocalDemoRole::Player, member(1))
            .unwrap()
            .encode_to_vec(),
        wire::journey_view(
            current,
            LocalDemoRole::Display,
            MemberId::from_bytes(&DISPLAY).unwrap(),
        )
        .unwrap()
        .encode_to_vec(),
    ]
}
fn request(
    current: &Checkpoint,
    operation: u8,
    kind: rpc::GameplayActionKind,
) -> rpc::SubmitActionRequest {
    let basis = current.basis();
    rpc::SubmitActionRequest {
        session_id: Some(rpc::SessionId {
            value: Some(basis.session.as_bytes().to_vec()),
        }),
        run_id: Some(rpc::RunId {
            value: Some(basis.run.as_bytes().to_vec()),
        }),
        operation_id: Some(rpc::OperationId {
            value: Some(vec![operation; 16]),
        }),
        observed_revision: Some(wire::revision(basis.revision)),
        offer_id: offer_id(current, kind),
        action_kind: kind as i32,
        destination: if kind == rpc::GameplayActionKind::ChooseHarborScene {
            rpc::HarborDestination::HarborInn as i32
        } else {
            rpc::HarborDestination::Unspecified as i32
        },
        ..Default::default()
    }
}
fn action(
    current: &Checkpoint,
    index: usize,
    operation: u8,
    kind: rpc::GameplayActionKind,
) -> Scope {
    assert!(
        offered(current, member(index))
            .unwrap()
            .iter()
            .any(|(offered, _)| *offered == kind)
    );
    let mut request = request(current, operation, kind);
    if kind == rpc::GameplayActionKind::CreateCharacter {
        request.character = Some(rpc::CharacterSelection {
            name: ["Brynn", "Vale"][index].to_owned(),
            choices: tests::build(["Brynn", "Vale"][index])
                .into_iter()
                .filter(|(key, _)| key.as_str() != "name")
                .chain([(
                    model::label("portrait").expect("portrait group"),
                    model::label(if index == 0 {
                        "portrait-hero-1"
                    } else {
                        "portrait-hero-2"
                    })
                    .expect("accepted quickstart portrait"),
                )])
                .map(|(key, value)| rpc::JourneyChoice {
                    group_id: key.as_str().to_owned(),
                    option_id: value.as_str().to_owned(),
                })
                .collect(),
            ..Default::default()
        });
    }
    assert!(!wire::unrelated_payload(&request, kind));
    Scope::command(wire::journey_input(&request, member(index), current).unwrap())
}
fn refuses(runtime: &mut Runtime, scope: &Scope, expected: SubmissionOutcome) {
    let before = runtime.owner.checkpoint().clone();
    let private = views(&before);
    let db = runtime.database.borrow();
    let visible_effects = (db.ledger.len(), db.publications, db.wakes, db.samples);
    drop(db);
    assert_eq!(runtime.submit(scope), expected);
    assert_eq!(runtime.owner.checkpoint(), &before);
    assert_eq!(runtime.database.borrow().checkpoint, before);
    assert_eq!(&*runtime.published.borrow(), &before);
    assert_eq!(views(runtime.owner.checkpoint()), private);
    let db = runtime.database.borrow();
    assert_eq!(
        (db.ledger.len(), db.publications, db.wakes, db.samples),
        visible_effects
    );
    assert!(runtime.notifications.try_recv().is_err());
}
fn assert_private_answer(current: &Checkpoint, asked: bool) {
    let first = wire::journey_view(current, LocalDemoRole::Player, member(0)).unwrap();
    let Some(rpc::view_message::Audience::Player(player)) = first.audience else {
        panic!("player view")
    };
    assert_eq!(
        player.private_clue,
        if asked { courier_ai::RESPONSE } else { "" }
    );
    let projected = views(current);
    for view in &projected[1..] {
        assert!(
            !view
                .windows(courier_ai::RESPONSE.len())
                .any(|bytes| bytes == courier_ai::RESPONSE.as_bytes())
        );
    }
}

#[test]
fn local_journey_engine_exposes_staged_acceptance_and_bound_refusal() {
    let current = initial().unwrap();
    let database = Rc::new(RefCell::new(Database {
        checkpoint: current.clone(),
        ledger: vec![],
        refuse_commit: false,
        damage: 1,
        decisions: 0,
        commits: 0,
        publications: 0,
        wakes: 0,
        samples: 0,
    }));
    let scope = Scope::command(tests::input(
        &current,
        bootstrap_member().unwrap(),
        1,
        "join-room",
        vec![],
    ));
    let mut engine = Engine(Rc::clone(&database));
    let result = engine.decide_transition(&current, &scope, &scope.input);
    assert!(matches!(result, TransitionResult::Accepted(_)));
    assert_eq!(current, database.borrow().checkpoint);
    assert_eq!(database.borrow().commits, 0);
    let mut foreign = scope.clone();
    foreign.operation = OperationId::from_bytes(&[0xf1; 16]).unwrap();
    assert_eq!(
        engine.decide_transition(&current, &foreign, &scope.input),
        TransitionResult::Rejected(RepositoryError::InvalidCandidate)
    );
    let mut mismatched = scope.clone();
    mismatched.input = tests::input(
        &current,
        bootstrap_member().unwrap(),
        2,
        "join-room",
        vec![],
    );
    assert_eq!(
        engine.decide_transition(&current, &mismatched, &scope.input),
        TransitionResult::Rejected(RepositoryError::InputBinding)
    );
    assert_eq!(database.borrow().commits, 0);
}

#[test]
fn durable_registered_journey_retains_each_legal_opening_branch_through_inn_and_late_retries() {
    use rpc::GameplayActionKind as Action;
    for opening in [Action::AskCourier, Action::EscortCourier] {
        let database = Rc::new(RefCell::new(Database {
            checkpoint: initial().unwrap(),
            ledger: vec![],
            refuse_commit: false,
            damage: 1,
            decisions: 0,
            commits: 0,
            publications: 0,
            wakes: 0,
            samples: 0,
        }));
        let mut runtime = Runtime::restore(Rc::clone(&database));
        let mut accepted = Vec::new();
        for operation in [1, 2] {
            let scope = Scope::command(tests::input(
                runtime.owner.checkpoint(),
                bootstrap_member().unwrap(),
                operation,
                "join-room",
                vec![],
            ));
            let receipt = runtime.accept(&scope);
            accepted.push((scope, receipt));
        }
        assert_eq!(participants(runtime.owner.checkpoint()).len(), 2);
        for (index, operation) in [(0, 3), (1, 4)] {
            let scope = action(
                runtime.owner.checkpoint(),
                index,
                operation,
                Action::CreateCharacter,
            );
            let receipt = runtime.accept(&scope);
            accepted.push((scope, receipt));
        }
        assert_eq!(runtime.owner.checkpoint().state().characters.len(), 2);
        assert_eq!(
            runtime.owner.checkpoint().state().continuity.creation.len(),
            2
        );
        let scope = action(runtime.owner.checkpoint(), 0, 5, Action::BeginStory);
        let receipt = runtime.accept(&scope);
        accepted.push((scope, receipt));
        assert_eq!(
            phase(runtime.owner.checkpoint()).unwrap(),
            rpc::JourneyPhase::Opening
        );

        let scope = action(runtime.owner.checkpoint(), 0, 6, opening);
        database.borrow_mut().refuse_commit = true;
        refuses(
            &mut runtime,
            &scope,
            SubmissionOutcome::Refused(RepositoryError::Unavailable),
        );
        assert!(runtime.owner.checkpoint().state().intents.is_empty());
        database.borrow_mut().refuse_commit = false;
        let receipt = runtime.accept(&scope);
        accepted.push((scope.clone(), receipt));
        assert_eq!(
            phase(runtime.owner.checkpoint()).unwrap(),
            rpc::JourneyPhase::Dialogue
        );
        assert_private_answer(runtime.owner.checkpoint(), false);
        let alternate = if opening == Action::AskCourier {
            Action::EscortCourier
        } else {
            Action::AskCourier
        };
        assert!(
            !offered(runtime.owner.checkpoint(), member(1))
                .unwrap()
                .iter()
                .any(|(kind, _)| *kind == alternate)
        );
        let refused_request = request(runtime.owner.checkpoint(), 30, alternate);
        assert!(!wire::unrelated_payload(&refused_request, alternate));
        let refused = Scope::command(
            wire::journey_input(&refused_request, member(1), runtime.owner.checkpoint()).unwrap(),
        );
        refuses(
            &mut runtime,
            &refused,
            SubmissionOutcome::Refused(RepositoryError::InvalidCandidate),
        );

        if opening == Action::AskCourier {
            let current = runtime.owner.checkpoint();
            assert_eq!(current.state().intents.len(), 1);
            let intent = &current.state().intents[0];
            let completion = Scope {
                session: current.basis().session,
                operation: courier_ai::completion_operation(intent).unwrap(),
                input: GameInput::Job(courier_ai::execute(current, intent).unwrap()),
            };
            let receipt = runtime.accept(&completion);
            accepted.push((completion, receipt));
            assert_eq!(
                runtime.owner.checkpoint().state().intents[0].status,
                DurableStatus::Completed
            );
        } else {
            assert!(runtime.owner.checkpoint().state().intents.is_empty());
        }
        let asked = opening == Action::AskCourier;
        assert_private_answer(runtime.owner.checkpoint(), asked);
        let mut stale = scope;
        stale.operation = OperationId::from_bytes(&[31; 16]).unwrap();
        let GameInput::Game(command) = &mut stale.input else {
            panic!("game command")
        };
        command.operation = stale.operation;
        refuses(
            &mut runtime,
            &stale,
            SubmissionOutcome::Refused(RepositoryError::InvalidCandidate),
        );

        // The first hero also spends Second Wind, so the aftermath rest
        // below has a use to regain (the machine refuses a rest nobody
        // gains from).
        for (index, operation, kind) in [
            (0, 7, Action::DefendCourier),
            (0, 8, Action::GreatswordAttack),
            (0, 9, Action::SecondWind),
            (0, 10, Action::EndTurn),
            (1, 11, Action::GreatswordAttack),
        ] {
            if operation == 11 {
                database.borrow_mut().damage = 6;
            }
            let scope = action(runtime.owner.checkpoint(), index, operation, kind);
            let receipt = runtime.accept(&scope);
            accepted.push((scope, receipt));
            assert_private_answer(runtime.owner.checkpoint(), asked);
        }
        let victory = runtime.owner.checkpoint().clone();
        assert_eq!(phase(&victory).unwrap(), rpc::JourneyPhase::Complete);
        assert!(combat_victory(victory.state()).unwrap());
        assert_eq!(
            victory.state().narrative.open_threads,
            vec![model::content(PACKET_THREAD).unwrap()]
        );
        let knowledge = victory.state().knowledge.clone();
        let round = combat_round(&victory).unwrap();
        let scope = action(&victory, 0, 12, Action::ShortRest);
        let receipt = runtime.accept(&scope);
        accepted.push((scope, receipt));
        let after_rest = runtime.owner.checkpoint().state().logical_time;
        assert_eq!(
            after_rest.ticks_per_second,
            victory.state().logical_time.ticks_per_second
        );
        assert_eq!(
            after_rest.ticks - victory.state().logical_time.ticks,
            3_600 * u64::from(after_rest.ticks_per_second)
        );
        assert_eq!(combat_round(runtime.owner.checkpoint()).unwrap(), round);
        let rested = runtime.owner.checkpoint().clone();
        let scope = action(&rested, 1, 13, Action::ChooseHarborScene);
        let receipt = runtime.accept(&scope);
        accepted.push((scope, receipt));
        let arrived = runtime.owner.checkpoint().clone();
        assert_eq!(arrived.state().logical_time, rested.state().logical_time);
        assert_eq!(arrived.state().characters, rested.state().characters);
        assert_eq!(arrived.state().resources, rested.state().resources);
        assert_eq!(arrived.state().encounters, rested.state().encounters);
        assert_eq!(arrived.state().knowledge, knowledge);
        assert_eq!(
            arrived.state().narrative.active_beats,
            vec![model::content("harbor-inn").unwrap()]
        );
        assert_eq!(
            arrived.state().narrative.open_threads,
            vec![model::content(PACKET_THREAD).unwrap()]
        );
        assert_eq!(combat_round(&arrived).unwrap(), round);
        assert_private_answer(&arrived, asked);
        let final_views = views(&arrived);
        for (role, principal) in [
            (LocalDemoRole::Player, member(0)),
            (LocalDemoRole::Player, member(1)),
            (
                LocalDemoRole::Display,
                MemberId::from_bytes(&DISPLAY).unwrap(),
            ),
        ] {
            let view = wire::journey_view(&arrived, role, principal).unwrap();
            let scene = match view.audience.unwrap() {
                rpc::view_message::Audience::Player(view) => view.scene.unwrap(),
                rpc::view_message::Audience::Display(view) => view.scene.unwrap(),
            };
            assert_eq!(scene.title, "The Harbor Inn");
            assert_eq!(scene.destination, rpc::HarborDestination::HarborInn as i32);
        }
        let final_activity = activity(&database.borrow());
        for (scope, receipt) in &accepted {
            assert_eq!(runtime.submit(scope), *receipt);
            assert_eq!(runtime.owner.checkpoint(), &arrived);
            assert_eq!(views(runtime.owner.checkpoint()), final_views);
            assert_eq!(activity(&database.borrow()), final_activity);
        }
        let mut conflict = accepted[0].0.clone();
        let GameInput::Game(command) = &mut conflict.input else {
            panic!("join command")
        };
        command.observed_revision = command.observed_revision.next_sequence().unwrap();
        refuses(
            &mut runtime,
            &conflict,
            SubmissionOutcome::OperationConflict,
        );
        let restored = model::checkpoint(arrived.basis(), arrived.state().clone()).unwrap();
        assert_eq!(restored, arrived);
        database.borrow_mut().checkpoint = restored;
        let mut recovered = Runtime::restore(Rc::clone(&database));
        assert_eq!(views(recovered.owner.checkpoint()), final_views);
        for (scope, receipt) in &accepted {
            assert_eq!(recovered.submit(scope), *receipt);
            assert_eq!(recovered.owner.checkpoint(), &arrived);
            assert_eq!(views(recovered.owner.checkpoint()), final_views);
            assert_eq!(activity(&database.borrow()), final_activity);
        }
        assert!(recovered.notifications.try_recv().is_err());
    }
}
