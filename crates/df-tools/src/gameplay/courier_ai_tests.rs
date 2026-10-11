use super::*;
use crate::gameplay::{actor, wire};
use df_ai::admission::RecordInputError;
use df_observe::OperationContext;
use df_persistence::local_demo_scope::{LocalDemoRole, PLAYER};
use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
use df_session::submission::*;
use std::{cell::RefCell, rc::Rc};

fn command(
    current: &Checkpoint,
    member: MemberId,
    operation: u8,
    entry: &str,
    choices: Vec<(RevisionLabel, RevisionLabel)>,
) -> GameInput {
    GameInput::Game(CommandInput {
        basis: current.basis(),
        operation: OperationId::from_bytes(&[operation; 16]).unwrap(),
        member,
        observed_revision: current.basis().revision,
        command: GameCommand::ProposeAction {
            actor: if member.as_bytes() == &PLAYER {
                journey::room_entity().unwrap()
            } else {
                journey::player_entity(member, current).unwrap()
            },
            action: model::content(entry).unwrap(),
            targets: vec![],
            choices,
        },
    })
}
fn build(name: &str) -> Vec<(RevisionLabel, RevisionLabel)> {
    [
        ("name", journey::hex(name.as_bytes())),
        ("species", "dwarf".to_owned()),
        ("class", "fighter-1".to_owned()),
        ("background", "soldier".to_owned()),
        ("array", "stalwart".to_owned()),
        ("alignment", "lawful-good".to_owned()),
        ("language-1", "dwarvish".to_owned()),
        ("language-2", "elvish".to_owned()),
        ("skill-1", "perception".to_owned()),
        ("skill-2", "survival".to_owned()),
        ("gaming-set", "dice".to_owned()),
        ("equipment", "fighter-a-soldier-a".to_owned()),
        (
            "style-masteries",
            "defense-greatsword-flail-javelin".to_owned(),
        ),
        ("allied-ties", "join-order".to_owned()),
    ]
    .into_iter()
    .map(|(key, value)| (model::label(key).unwrap(), model::label(&value).unwrap()))
    .collect()
}
fn before_answer() -> (Checkpoint, MemberId, MemberId) {
    let mut current = journey::initial().unwrap();
    let bootstrap = MemberId::from_bytes(&PLAYER).unwrap();
    for operation in [1, 2] {
        current = journey::stage_join(
            &current,
            &command(&current, bootstrap, operation, "join-room", vec![]),
        )
        .unwrap();
    }
    let members = current
        .state()
        .members
        .iter()
        .filter(|link| link.member != bootstrap)
        .map(|link| link.member)
        .collect::<Vec<_>>();
    let (first, second) = (members[0], members[1]);
    current = journey::stage(
        &current,
        &command(&current, first, 3, "create-character", build("Brynn")),
    )
    .unwrap();
    current = journey::stage(
        &current,
        &command(&current, second, 4, "create-character", build("Vale")),
    )
    .unwrap();
    current = journey::stage(
        &current,
        &command(&current, first, 5, "begin-story", vec![]),
    )
    .unwrap();
    (current, first, second)
}
fn pending() -> (Checkpoint, MemberId, MemberId) {
    let (current, first, second) = before_answer();
    (
        journey::stage(
            &current,
            &command(&current, first, 6, "ask-courier", vec![]),
        )
        .unwrap(),
        first,
        second,
    )
}
fn intent(current: &Checkpoint) -> &DurableIntent {
    current.state().intents.first().unwrap()
}
fn player_clue(current: &Checkpoint, member: MemberId) -> String {
    let view = wire::journey_view(current, LocalDemoRole::Player, member).unwrap();
    let Some(crate::gameplay::rpc::view_message::Audience::Player(player)) = view.audience else {
        panic!("player view")
    };
    player.private_clue
}

#[test]
fn actual_source_producer_retains_reveal_and_declares_only_one_pending_ai_effect() {
    let (before, first, _) = before_answer();
    let current =
        journey::stage(&before, &command(&before, first, 6, "ask-courier", vec![])).unwrap();
    let effect = intent(&current);
    assert_eq!(effect.kind, EffectKind::RunAi);
    assert_eq!(effect.status, DurableStatus::Pending);
    assert_eq!(recipient(&current, effect), Ok(first));
    assert_eq!(
        df_engine::effect_emission::declare_accepted_effects(
            &current,
            current.basis(),
            current.pins(),
            effect.operation,
            df_engine::effect_emission::EffectEmissionLimits {
                maximum_scan_records: 512,
                maximum_effects: 2,
                maximum_comparisons: 8192,
                maximum_retained_bytes: 8192
            }
        )
        .unwrap(),
        vec![effect.clone()]
    );
    assert!(player_clue(&current, first).is_empty());
    assert_eq!(current.state().draws, before.state().draws);
    assert_eq!(current.state().resources, before.state().resources);
    assert_eq!(
        current.state().narrative.open_threads,
        before.state().narrative.open_threads
    );
}

#[test]
fn actual_prepared_record_completion_and_safe_view_preserve_sibling_state() {
    let (current, first, second) = pending();
    let original = current.clone();
    let effect = intent(&current);
    let completion = execute(&current, effect).unwrap();
    assert_eq!(completion, expected_completion(effect).unwrap());
    let staged =
        stage_completion(&current, &completion, completion_operation(effect).unwrap()).unwrap();
    assert_eq!(current, original);
    assert_eq!(staged.state().intents[0].status, DurableStatus::Completed);
    assert_eq!(staged.state().facts, current.state().facts);
    assert_eq!(staged.state().draws, current.state().draws);
    assert_eq!(staged.state().resources, current.state().resources);
    assert_eq!(staged.state().narrative, current.state().narrative);
    assert_eq!(staged.state().logical_time, current.state().logical_time);
    assert_eq!(player_clue(&staged, first), RESPONSE);
    assert!(player_clue(&staged, second).is_empty());
    let display = wire::journey_view(
        &staged,
        LocalDemoRole::Display,
        MemberId::from_bytes(&df_persistence::local_demo_scope::DISPLAY).unwrap(),
    )
    .unwrap();
    let Some(crate::gameplay::rpc::view_message::Audience::Display(view)) = display.audience else {
        panic!("display")
    };
    // Encoding an actual display projection must not include the private response bytes.
    use prost::Message;
    assert!(
        !view
            .encode_to_vec()
            .windows(RESPONSE.len())
            .any(|bytes| bytes == RESPONSE.as_bytes())
    );
    assert!(stage_completion(&staged, &completion, completion_operation(effect).unwrap()).is_err());
}

fn completed_answer() -> (Checkpoint, MemberId, MemberId) {
    let (current, first, second) = pending();
    let effect = intent(&current);
    let completion = execute(&current, effect).unwrap();
    (
        stage_completion(&current, &completion, completion_operation(effect).unwrap()).unwrap(),
        first,
        second,
    )
}

#[test]
fn restored_response_requires_completion_strictly_after_request_revision() {
    let (completed, first, _) = completed_answer();
    let original = completed.clone();
    let effect = intent(&completed);
    let operation = completion_operation(effect).unwrap();
    let origin = effect.basis.revision;
    let earlier =
        df_types::SessionRevision::new(origin.epoch(), origin.sequence().checked_sub(1).unwrap());
    for revision in [earlier, origin] {
        let mut state = completed.state().clone();
        state
            .decisions
            .iter_mut()
            .find(|decision| decision.operation == operation)
            .unwrap()
            .revision = revision;
        // Structural restore succeeds: it cannot establish this owner's causal contract.
        let restored = model::checkpoint(completed.basis(), state).unwrap();
        assert_eq!(
            saved_response(&restored, first),
            Err(RepositoryError::InvalidCandidate)
        );
        assert!(matches!(
            wire::journey_view(&restored, LocalDemoRole::Player, first),
            Err(RepositoryError::InvalidCandidate)
        ));
        assert_eq!(completed, original);
    }
}

#[test]
fn restored_response_refuses_nonempty_terminal_receipt_vectors_with_valid_references() {
    let (completed, first, _) = completed_answer();
    let original = completed.clone();
    let operation = completion_operation(intent(&completed)).unwrap();
    let revision = completed.basis().revision;
    for case in 0..3 {
        let mut state = completed.state().clone();
        match case {
            0 => {
                let id = FactId::from_bytes(&[0xa1; 16]).unwrap();
                state.facts.push(GameFact {
                    id,
                    revision,
                    operation,
                    ordinal: 0,
                    cause: None,
                    audience: AudienceScope::Members(vec![first]),
                    value: FactValue::ContentEvent {
                        definition: model::content("harbor").unwrap(),
                        subjects: vec![],
                    },
                });
                state
                    .decisions
                    .iter_mut()
                    .find(|decision| decision.operation == operation)
                    .unwrap()
                    .facts
                    .push(id);
            }
            1 => {
                state.draws.push(ActualDraw {
                    operation,
                    ordinal: 0,
                    resolution: ResolutionId::from_bytes(&[0xa2; 16]).unwrap(),
                    window: WindowId::from_bytes(&[0xa3; 16]).unwrap(),
                    sides: 20,
                    value: 1,
                    source: model::rule().unwrap(),
                });
                state
                    .decisions
                    .iter_mut()
                    .find(|decision| decision.operation == operation)
                    .unwrap()
                    .draws
                    .push(0);
            }
            2 => {
                let id = EffectId::from_bytes(&[0xa4; 16]).unwrap();
                state.intents.push(DurableIntent {
                    id,
                    basis: completed.basis(),
                    operation,
                    slot: 0,
                    kind: EffectKind::PublishPresentation,
                    job: None,
                    timer: None,
                    generation: 1,
                    status: DurableStatus::Pending,
                    definition: model::content("harbor").unwrap(),
                });
                state
                    .decisions
                    .iter_mut()
                    .find(|decision| decision.operation == operation)
                    .unwrap()
                    .effects
                    .push(id);
            }
            _ => unreachable!(),
        }
        // Each forged vector has matching canonical records, rather than dangling fixture IDs.
        let restored = model::checkpoint(completed.basis(), state).unwrap();
        assert_eq!(
            saved_response(&restored, first),
            Err(RepositoryError::InvalidCandidate),
            "case {case}"
        );
        assert!(matches!(
            wire::journey_view(&restored, LocalDemoRole::Player, first),
            Err(RepositoryError::InvalidCandidate)
        ));
        assert_eq!(completed, original);
    }
}

#[test]
fn restored_response_preserves_existing_terminal_identity_policy_and_text_refusals() {
    let (completed, first, _) = completed_answer();
    let operation = completion_operation(intent(&completed)).unwrap();
    for case in 0..5 {
        let mut state = completed.state().clone();
        if case == 0 {
            state
                .decisions
                .retain(|decision| decision.operation != operation);
        } else {
            let terminal = state
                .decisions
                .iter_mut()
                .find(|decision| decision.operation == operation)
                .unwrap();
            match case {
                1 => terminal.operation = OperationId::from_bytes(&[0xb1; 16]).unwrap(),
                2 => terminal.source_policy = model::label("unadmitted-response-policy").unwrap(),
                3 => terminal.semantic_output = None,
                4 => terminal.semantic_output = Some("The courier guarantees victory.".to_owned()),
                _ => unreachable!(),
            }
        }
        let restored = model::checkpoint(completed.basis(), state).unwrap();
        assert_eq!(
            saved_response(&restored, first),
            Err(RepositoryError::InvalidCandidate),
            "case {case}"
        );
        assert!(matches!(
            wire::journey_view(&restored, LocalDemoRole::Player, first),
            Err(RepositoryError::InvalidCandidate)
        ));
    }
}

fn defend_with_retained_initiative(current: &Checkpoint, member: MemberId) -> Checkpoint {
    assert!(
        journey::offered(current, member)
            .unwrap()
            .iter()
            .any(|(kind, _)| *kind == crate::gameplay::rpc::GameplayActionKind::DefendCourier)
    );
    let input = command(current, member, 7, "defend-courier", vec![]);
    let mut faces = [20, 19, 1].into_iter();
    let staged = journey::stage_with_supplier(current, &input, &mut |sides| {
        assert_eq!(sides, 20);
        faces.next().ok_or(RepositoryError::InvalidCandidate)
    })
    .unwrap();
    assert!(faces.next().is_none());
    let GameInput::Game(command) = input else {
        unreachable!()
    };
    assert_eq!(
        staged
            .state()
            .draws
            .iter()
            .filter(|draw| draw.operation == command.operation)
            .map(|draw| (draw.sides, draw.value))
            .collect::<Vec<_>>(),
        [(20, 20), (20, 19), (20, 1)]
    );
    staged
}

#[test]
fn genuine_completion_survives_intervening_actions_and_later_restored_current_revision() {
    // Asking selects private Dialogue; its advertised next action is Defend, not Escort.
    // Explicit source-legal initiative faces are retained by the actual registered transition.
    for defend_before_completion in [true, false] {
        let (mut current, first, second) = pending();
        let origin = intent(&current).basis.revision;
        if defend_before_completion {
            current = defend_with_retained_initiative(&current, first);
        }
        let effect = intent(&current).clone();
        let completion = execute(&current, &effect).unwrap();
        let operation = completion_operation(&effect).unwrap();
        current = stage_completion(&current, &completion, operation).unwrap();
        let terminal_revision = current.basis().revision;
        if defend_before_completion {
            assert!(terminal_revision > origin.next_sequence().unwrap());
        } else {
            current = defend_with_retained_initiative(&current, first);
            assert!(current.basis().revision > terminal_revision);
        }
        let restored = model::checkpoint(current.basis(), current.state().clone()).unwrap();
        assert_eq!(saved_response(&restored, first).unwrap(), Some(RESPONSE));
        assert_eq!(player_clue(&restored, first), RESPONSE);
        assert!(player_clue(&restored, second).is_empty());
        assert_eq!(
            stage_completion(&restored, &completion, operation),
            Err(CourierError::Stale)
        );
        assert_eq!(
            execute(&restored, intent(&restored)),
            Err(CourierError::Stale)
        );
        assert_eq!(restored, current);
        let terminal = restored
            .state()
            .decisions
            .iter()
            .find(|decision| decision.operation == operation)
            .unwrap();
        assert_eq!(terminal.revision, terminal_revision);
        assert!(terminal.facts.is_empty());
        assert!(terminal.draws.is_empty());
        assert!(terminal.effects.is_empty());
    }
}

#[test]
fn every_completion_binding_and_final_response_refusal_preserves_original_checkpoint() {
    let (current, _, _) = pending();
    let original = current.clone();
    let effect = intent(&current);
    let accepted = execute(&current, effect).unwrap();
    for case in 0..9 {
        let mut changed = accepted.clone();
        let mut operation = completion_operation(effect).unwrap();
        match case {
            0 => changed.generation += 1,
            1 => changed.job = JobId::from_bytes(&[99; 16]).unwrap(),
            2 => changed.operation = OperationId::from_bytes(&[99; 16]).unwrap(),
            3 => changed.basis.run = df_types::RunId::from_bytes(&[99; 16]).unwrap(),
            4 => changed.basis.session = df_types::SessionId::from_bytes(&[99; 16]).unwrap(),
            5 => operation = effect.operation,
            6 => {
                if let JobOutcome::Ai { policy, .. } = &mut changed.outcome {
                    *policy = model::content("harbor").unwrap();
                }
            }
            7 => {
                if let JobOutcome::Ai { model, .. } = &mut changed.outcome {
                    *model = model::label("unadmitted-model").unwrap();
                }
            }
            8 => {
                if let JobOutcome::Ai {
                    semantic_output, ..
                } = &mut changed.outcome
                {
                    *semantic_output = "The courier guarantees a successful battle.".to_owned();
                }
            }
            _ => unreachable!(),
        }
        assert!(
            stage_completion(&current, &changed, operation).is_err(),
            "case {case}"
        );
        assert_eq!(current, original);
    }
}

#[test]
fn changed_source_and_unapproved_modes_never_execute_or_change_truth() {
    let (current, _, _) = pending();
    for case in 0..6 {
        let mut state = current.state().clone();
        match case {
            0 => state.intents[0].definition = model::content("harbor").unwrap(),
            1 => state.mode = ExecutionMode::Live,
            2 => state.mode = ExecutionMode::Replay,
            3 => {
                let source = state
                    .facts
                    .iter_mut()
                    .find(|fact| {
                        fact.operation == intent(&current).operation
                            && matches!(fact.value, FactValue::ContentEvent { .. })
                    })
                    .unwrap();
                source.value = FactValue::ContentEvent {
                    definition: model::content("escort-courier").unwrap(),
                    subjects: vec![],
                };
            }
            4 => {
                state
                    .decisions
                    .iter_mut()
                    .find(|decision| decision.operation == intent(&current).operation)
                    .unwrap()
                    .source_policy = model::label("unadmitted-source-policy").unwrap()
            }
            5 => state.intents[0].status = DurableStatus::Cancelled,
            _ => unreachable!(),
        }
        let changed = model::checkpoint(current.basis(), state).unwrap();
        let original = changed.clone();
        assert!(execute(&changed, intent(&changed)).is_err());
        assert_eq!(changed, original);
    }
}

#[test]
fn truncated_cancelled_mismatched_and_trailing_streams_publish_nothing() {
    let (current, first, _) = pending();
    let original = current.clone();
    let effect = intent(&current);
    let complete = || RecordEvent::Complete {
        identity: RecordIdentity {
            key: effect.job.unwrap(),
            basis: effect.basis,
            operation: effect.operation,
        },
        byte_length: RESPONSE.len() as u64,
        sha256: Sha256::digest(RESPONSE.as_bytes()).into(),
    };
    let streams = [
        vec![Ok(RecordEvent::Chunk(RESPONSE.as_bytes().to_vec()))],
        vec![
            Ok(RecordEvent::Chunk(RESPONSE.as_bytes()[..10].to_vec())),
            Ok(complete()),
        ],
        vec![
            Ok(RecordEvent::Chunk(RESPONSE.as_bytes().to_vec())),
            Err(RecordInputError::Cancelled),
        ],
        vec![
            Ok(RecordEvent::Chunk(RESPONSE.as_bytes().to_vec())),
            Ok(complete()),
            Ok(RecordEvent::Chunk(vec![])),
        ],
        vec![
            Ok(RecordEvent::Chunk(vec![b'x'; RESPONSE.len()])),
            Ok(complete()),
        ],
    ];
    for events in streams {
        assert_eq!(
            admit_record(&current, effect, events),
            Err(CourierError::Record)
        );
        assert_eq!(current, original);
        assert_eq!(saved_response(&current, first).unwrap(), None);
    }
}

#[test]
fn final_candidate_capacity_failure_does_not_escape_completed_status_or_response() {
    let (mut current, first, _) = pending();
    // Fill only existing canonical history records. Every prior fixture snapshot
    // passes the real constructor; the next response must cross the same bound.
    for ordinal in 0_u64..512 {
        let mut state = current.state().clone();
        let mut bytes = [0x90; 16];
        bytes[8..].copy_from_slice(&ordinal.to_be_bytes());
        state.decisions.push(AcceptedDecision {
            operation: OperationId::from_bytes(&bytes).unwrap(),
            revision: current.basis().revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: model::label("fixture-history").unwrap(),
            semantic_output: None,
        });
        match model::checkpoint(current.basis(), state) {
            Ok(next) => current = next,
            Err(_) => break,
        }
    }
    let original = current.clone();
    let effect = intent(&current);
    let completion = execute(&current, effect).unwrap();
    assert_eq!(
        stage_completion(&current, &completion, completion_operation(effect).unwrap()),
        Err(CourierError::Capacity)
    );
    assert_eq!(current, original);
    assert_eq!(intent(&current).status, DurableStatus::Pending);
    assert_eq!(saved_response(&current, first).unwrap(), None);
}

// The repository below is a classified in-memory commit fixture, not PostgreSQL,
// authentication or native issuer qualification. It exercises the actual session
// lookup/commit gates with the source producer and recording/response consumers.
#[derive(Eq, PartialEq)]
struct Key(OperationId, Vec<u8>);
impl ActorInput for Key {
    fn retained_heap_bytes(&self) -> Option<usize> {
        Some(self.1.capacity())
    }
}
struct Scope {
    operation: OperationId,
    input: GameInput,
    fingerprint: Vec<u8>,
}
impl ActorInput for Scope {
    fn retained_heap_bytes(&self) -> Option<usize> {
        self.input
            .retained_bytes()?
            .checked_add(self.fingerprint.capacity())
    }
}
impl OperationScope for Scope {
    type UncertaintyKey = Key;
    fn capture_uncertainty_key(&self, maximum: usize) -> Result<Key, RepositoryError> {
        if maximum < std::mem::size_of::<Key>() + self.fingerprint.len() {
            return Err(RepositoryError::Capacity);
        }
        Ok(Key(self.operation, self.fingerprint.clone()))
    }
    fn session(&self) -> df_types::SessionId {
        match &self.input {
            GameInput::Game(input) => input.basis.session,
            GameInput::Job(input) => input.basis.session,
            _ => unreachable!(),
        }
    }
    fn operation(&self) -> OperationId {
        self.operation
    }
    fn validate_input(&self, input: &GameInput) -> Result<(), RepositoryError> {
        if input == &self.input {
            Ok(())
        } else {
            Err(RepositoryError::InputBinding)
        }
    }
    fn is_lookup_only(&self) -> bool {
        false
    }
}
#[derive(Clone, Copy)]
enum Commit {
    Confirm,
    Refuse,
    LostAcknowledgement,
}
struct Stored {
    checkpoint: Checkpoint,
    receipts: Vec<(OperationId, Vec<u8>, DecisionReceipt)>,
    commit: Commit,
    events: Vec<&'static str>,
    unavailable_lookups: usize,
}
struct Repository(Rc<RefCell<Stored>>);
impl SessionRepository for Repository {
    type Scope = Scope;
    fn lookup_operation(
        &mut self,
        scope: &Scope,
        _: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        let mut stored = self.0.borrow_mut();
        if stored.unavailable_lookups > 0 {
            stored.unavailable_lookups -= 1;
            return Err(RepositoryError::Unavailable);
        }
        Ok(
            match stored
                .receipts
                .iter()
                .find(|(operation, _, _)| *operation == scope.operation)
            {
                Some((_, fingerprint, receipt)) if fingerprint == &scope.fingerprint => {
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
        let mut stored = self.0.borrow_mut();
        if stored.checkpoint.basis() != expected {
            return Err(RepositoryError::RevisionConflict);
        }
        if matches!(stored.commit, Commit::Refuse) {
            return Err(RepositoryError::Unavailable);
        }
        let receipt = DecisionReceipt::new(
            candidate.basis(),
            candidate
                .state()
                .decisions
                .iter()
                .find(|decision| decision.operation == scope.operation)
                .unwrap()
                .clone(),
            4096,
        )?;
        stored.checkpoint = candidate.clone();
        stored
            .receipts
            .push((scope.operation, scope.fingerprint.clone(), receipt.clone()));
        stored.events.push("commit");
        Ok(if matches!(stored.commit, Commit::LostAcknowledgement) {
            CommitOutcome::Indeterminate
        } else {
            CommitOutcome::Confirmed(receipt)
        })
    }
    fn load_current(
        &mut self,
        _: &Scope,
        _: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        Ok(self.0.borrow().checkpoint.clone())
    }
}
struct Engine(Rc<RefCell<Stored>>);
impl SessionEngine<Scope> for Engine {
    fn decide(
        &mut self,
        current: &Checkpoint,
        scope: &Scope,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        scope.validate_input(input)?;
        match input {
            GameInput::Game(_) => journey::stage(current, input),
            GameInput::Job(completion) => {
                self.0.borrow_mut().events.push("execute");
                let admitted = execute(current, intent(current))
                    .map_err(|_| RepositoryError::InvalidCandidate)?;
                if admitted != *completion {
                    return Err(RepositoryError::InputBinding);
                }
                stage_completion(current, completion, scope.operation)
                    .map_err(|_| RepositoryError::InvalidCandidate)
            }
            _ => Err(RepositoryError::InvalidCandidate),
        }
    }
    fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
        current
            .validate_resume(current.basis(), &model::pins()?)
            .map(|_| ())
            .map_err(|_| RepositoryError::InvalidCandidate)
    }
}
struct Publication(Rc<RefCell<Stored>>, actor::Publication);
impl PublicationOwner<Scope> for Publication {
    fn publish_committed(&mut self, _: &Scope, current: &Checkpoint) -> Result<(), DeliveryError> {
        self.1.publish(current);
        Ok(())
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        self.0.borrow_mut().events.push("wake");
        self.1.wake()
    }
}
type Owner = DurableOwner<Repository, Engine, Publication>;
fn owner(
    stored: &Rc<RefCell<Stored>>,
    current: Checkpoint,
) -> (Owner, std::sync::mpsc::Receiver<()>) {
    let (updates, _) = tokio::sync::watch::channel(current.clone());
    let (publication, notifications) = actor::Publication::new(updates);
    (
        DurableOwner::new(
            Repository(stored.clone()),
            Engine(stored.clone()),
            Publication(stored.clone(), publication),
            current,
            4096,
        )
        .unwrap(),
        notifications,
    )
}
fn submit(owner: &mut Owner, input: &GameInput, operation: OperationId) -> SubmissionOutcome {
    let fingerprint = if let GameInput::Job(completion) = input {
        completion_fingerprint(completion).unwrap().to_vec()
    } else {
        operation.as_bytes().to_vec()
    };
    let scope = Scope {
        operation,
        input: input.clone(),
        fingerprint,
    };
    let (item, receiver) = OwnedInput::new(
        OperationContext {
            trace_parent: String::new(),
            build: "courier-connection-test".to_owned(),
        },
        scope,
        input.clone(),
    );
    owner.reduce(AdmissionSequence(0), item);
    receiver.try_recv().unwrap()
}

#[test]
fn source_commit_precedes_wake_and_execution_and_refused_commit_never_wakes() {
    let (current, first, _) = before_answer();
    let input = command(&current, first, 6, "ask-courier", vec![]);
    let operation = if let GameInput::Game(input) = &input {
        input.operation
    } else {
        unreachable!()
    };
    let stored = Rc::new(RefCell::new(Stored {
        checkpoint: current.clone(),
        receipts: vec![],
        commit: Commit::Refuse,
        events: vec![],
        unavailable_lookups: 0,
    }));
    let (mut session, notifications) = owner(&stored, current.clone());
    assert_eq!(
        submit(&mut session, &input, operation),
        SubmissionOutcome::Refused(RepositoryError::Unavailable)
    );
    assert_eq!(stored.borrow().checkpoint, current);
    assert!(notifications.try_recv().is_err());
    assert!(stored.borrow().events.is_empty());
    stored.borrow_mut().commit = Commit::Confirm;
    assert!(matches!(
        submit(&mut session, &input, operation),
        SubmissionOutcome::Confirmed(_)
    ));
    assert_eq!(stored.borrow().events, ["commit", "wake"]);
    notifications.try_recv().unwrap();
    let effect = intent(session.checkpoint()).clone();
    let completion = GameInput::Job(expected_completion(&effect).unwrap());
    assert!(matches!(
        submit(
            &mut session,
            &completion,
            completion_operation(&effect).unwrap()
        ),
        SubmissionOutcome::Confirmed(_)
    ));
    assert_eq!(
        stored.borrow().events,
        ["commit", "wake", "execute", "commit", "wake"]
    );
    assert_eq!(player_clue(session.checkpoint(), first), RESPONSE);
}

#[test]
fn lost_completion_ack_exact_retry_and_restart_accept_response_only_once() {
    let (current, first, second) = pending();
    let effect = intent(&current).clone();
    let input = GameInput::Job(expected_completion(&effect).unwrap());
    let operation = completion_operation(&effect).unwrap();
    let stored = Rc::new(RefCell::new(Stored {
        checkpoint: current.clone(),
        receipts: vec![],
        commit: Commit::LostAcknowledgement,
        events: vec![],
        unavailable_lookups: 0,
    }));
    let (mut session, notifications) = owner(&stored, current.clone());
    assert_eq!(
        submit(&mut session, &input, operation),
        SubmissionOutcome::LookupRequired
    );
    assert_eq!(session.checkpoint(), &current);
    assert!(notifications.try_recv().is_err());
    stored.borrow_mut().unavailable_lookups = 2;
    for _ in 0..2 {
        assert_eq!(
            submit(&mut session, &input, operation),
            SubmissionOutcome::Refused(RepositoryError::Unavailable)
        );
        assert!(session.has_uncertain_operation());
        assert_eq!(session.checkpoint(), &current);
        assert_eq!(stored.borrow().events, ["execute", "commit"]);
    }
    let changed_operation = OperationId::from_bytes(&[0x99; 16]).unwrap();
    assert_eq!(
        submit(&mut session, &input, changed_operation),
        SubmissionOutcome::LookupRequired
    );
    assert!(session.has_uncertain_operation());
    assert_eq!(stored.borrow().events, ["execute", "commit"]);
    struct IdleRecovery(Owner);
    impl Reducer<GameInput> for IdleRecovery {
        fn reduce(&mut self, _: AdmissionSequence, _: GameInput) {
            panic!("receipt recovery must not require a player input");
        }
    }
    let (sender, inbox) = df_session::inbox::bounded_inbox::<GameInput>();
    let mut recovery = IdleRecovery(session);
    let due = std::time::Instant::now();
    let mut wakes = 0;
    let drained = inbox
        .run_with_owner_wake(
            &mut recovery,
            |_| Some(due),
            |recovery| {
                wakes += 1;
                assert!(matches!(
                    submit(&mut recovery.0, &input, operation),
                    SubmissionOutcome::Confirmed(_)
                ));
                assert!(recovery.0.is_current());
                assert!(!recovery.0.has_uncertain_operation());
                sender.stop().unwrap();
            },
        )
        .unwrap();
    assert_eq!(wakes, 1);
    assert_eq!(drained.reduced_inputs, 0);
    let session = recovery.0;
    assert_eq!(stored.borrow().events, ["execute", "commit"]);
    assert_eq!(player_clue(session.checkpoint(), first), RESPONSE);
    assert!(player_clue(session.checkpoint(), second).is_empty());
    let saved = stored.borrow().checkpoint.clone();
    let (mut restarted, _) = owner(&stored, saved.clone());
    assert!(matches!(
        submit(&mut restarted, &input, operation),
        SubmissionOutcome::Confirmed(_)
    ));
    assert_eq!(stored.borrow().events, ["execute", "commit"]);
    assert_eq!(restarted.checkpoint(), &saved);
    assert_eq!(
        saved
            .state()
            .decisions
            .iter()
            .filter(|decision| decision.source_policy.as_str() == POLICY)
            .count(),
        1
    );
}

#[test]
fn capacity_one_wake_coalesces_and_disconnected_executor_is_explicit() {
    let (current, _, _) = pending();
    let (updates, _) = tokio::sync::watch::channel(current);
    let (mut publication, notifications) = actor::Publication::new(updates);
    assert_eq!(publication.wake(), Ok(()));
    assert_eq!(publication.wake(), Ok(()));
    notifications.try_recv().unwrap();
    assert!(notifications.try_recv().is_err());
    drop(notifications);
    assert_eq!(publication.wake(), Err(DeliveryError::Unavailable));
}

#[path = "cache_replay_locale_contract.rs"]
mod cache_replay_locale_contract;

#[test]
fn edited_courier_clue_is_recorded_and_only_revealed_to_its_recipient() {
    let mut content: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/harbor-journey-pack-baseline.json"))
            .unwrap();
    let clue = "The courier whispers: find Mira at the east pier.";
    content["courier_clue"] = clue.into();
    let loaded = crate::gameplay::pack::load_from_bytes(
        &serde_json::to_vec(&content).unwrap(),
        include_bytes!("../../../../content/harbor-journey/tuning.json"),
    )
    .unwrap();
    crate::gameplay::pack::with_test_pack(loaded, || {
        let (current, first, second) = pending();
        let effect = intent(&current);
        let completion = execute(&current, effect).unwrap();
        let staged =
            stage_completion(&current, &completion, completion_operation(effect).unwrap()).unwrap();
        assert_eq!(player_clue(&staged, first), clue);
        assert!(player_clue(&staged, second).is_empty());
        let display = wire::journey_view(
            &staged,
            LocalDemoRole::Display,
            MemberId::from_bytes(&df_persistence::local_demo_scope::DISPLAY).unwrap(),
        )
        .unwrap();
        use prost::Message;
        assert!(
            !display
                .encode_to_vec()
                .windows(clue.len())
                .any(|bytes| bytes == clue.as_bytes())
        );
    });
}
