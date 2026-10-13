use super::tests::{input, opening_story};
use super::*;
use df_observe::OperationContext;
use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
use df_session::submission::*;
use df_types::SessionId;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

fn combat() -> Checkpoint {
    let first = MemberId::from_bytes(&MEMBERS[0]).unwrap();
    let opening = opening_story();
    let escorted = stage_with_supplier(
        &opening,
        &input(&opening, first, 6, "escort-courier", vec![]),
        &mut |_| panic!("escort cannot draw"),
    )
    .unwrap();
    let current = stage_with_supplier(
        &escorted,
        &input(&escorted, first, 7, "defend-courier", vec![]),
        &mut |sides| {
            assert_eq!(sides, 20);
            Ok(10)
        },
    )
    .unwrap();
    assert_eq!(phase(&current).unwrap(), rpc::JourneyPhase::Combat);
    assert_eq!(
        current.state().encounters[0].active_turn,
        Some(entity(ENTITIES[0]).unwrap())
    );
    current
}

fn attack_input(current: &Checkpoint, operation: u8, savage: bool, graze: bool) -> GameInput {
    input(
        current,
        MemberId::from_bytes(&MEMBERS[0]).unwrap(),
        operation,
        "greatsword-attack",
        vec![
            (
                model::label("savage-attacker").unwrap(),
                model::label(if savage { "yes" } else { "no" }).unwrap(),
            ),
            (
                model::label("graze").unwrap(),
                model::label(if graze { "yes" } else { "no" }).unwrap(),
            ),
        ],
    )
}

struct ExpectedAttack {
    damage: u32,
    hit_points: u32,
    hit: bool,
    critical: bool,
    knocked_out: bool,
    grazed: bool,
}

#[test]
fn registered_player_greatsword_uses_existing_source_outcomes_and_action_economy() {
    for (faces, savage, graze, expected) in [
        (
            &[10, 1, 2][..],
            false,
            false,
            ExpectedAttack {
                damage: 6,
                hit_points: 5,
                hit: true,
                critical: false,
                knocked_out: false,
                grazed: false,
            },
        ),
        (
            &[1][..],
            false,
            false,
            ExpectedAttack {
                damage: 0,
                hit_points: 11,
                hit: false,
                critical: false,
                knocked_out: false,
                grazed: false,
            },
        ),
        (
            &[1][..],
            false,
            true,
            ExpectedAttack {
                damage: 3,
                hit_points: 8,
                hit: false,
                critical: false,
                knocked_out: false,
                grazed: true,
            },
        ),
        (
            &[20, 1, 1, 1, 1][..],
            false,
            false,
            ExpectedAttack {
                damage: 7,
                hit_points: 4,
                hit: true,
                critical: true,
                knocked_out: false,
                grazed: false,
            },
        ),
        (
            &[10, 1, 1, 3, 3][..],
            true,
            false,
            ExpectedAttack {
                damage: 9,
                hit_points: 2,
                hit: true,
                critical: false,
                knocked_out: false,
                grazed: false,
            },
        ),
        (
            &[20, 1, 1, 1, 1, 2, 2, 2, 2][..],
            true,
            false,
            ExpectedAttack {
                damage: 11,
                hit_points: 1,
                hit: true,
                critical: true,
                knocked_out: true,
                grazed: false,
            },
        ),
    ] {
        let current = combat();
        let before = current.clone();
        let command = attack_input(&current, 8, savage, graze);
        let mut supplied = faces.iter().copied();
        let mut calls = Vec::new();
        let candidate = stage_with_supplier(&current, &command, &mut |sides| {
            let face = supplied.next().unwrap();
            calls.push((sides, face));
            Ok(face)
        })
        .unwrap();
        assert!(supplied.next().is_none());
        assert_eq!(calls.len(), faces.len());
        assert_eq!(calls[0].0, 20);
        assert!(calls[1..].iter().all(|(sides, _)| *sides == 6));
        let decision = candidate.state().decisions.last().unwrap();
        let receipt = accepted(decision).unwrap();
        let [outcome] = receipt.combat.as_slice() else {
            panic!("one existing attack")
        };
        assert_eq!(outcome.actor_id, ENTITIES[0]);
        assert_eq!(outcome.target_id, BANDIT);
        assert_eq!(outcome.attack_die, faces[0]);
        assert_eq!(outcome.attack_modifier, 5);
        assert_eq!(outcome.target_armor_class, 12);
        assert_eq!(outcome.damage_dice, faces[1..]);
        assert_eq!(outcome.damage, expected.damage);
        assert_eq!(outcome.target_hit_points, expected.hit_points);
        assert_eq!(outcome.hit, expected.hit);
        assert_eq!(outcome.critical, expected.critical);
        assert_eq!(outcome.knocked_out, expected.knocked_out);
        assert_eq!(outcome.grazed, expected.grazed);
        assert_eq!(outcome.source_revision, rules::SOURCE_REVISION);
        assert_eq!(
            value(
                candidate.state(),
                entity(ENTITIES[0]).unwrap(),
                "action-used"
            )
            .unwrap(),
            1
        );
        assert_eq!(
            value(candidate.state(), entity(BANDIT).unwrap(), "hit-points").unwrap(),
            expected.hit_points
        );
        let expected_phase = if expected.knocked_out {
            rpc::JourneyPhase::Complete
        } else {
            rpc::JourneyPhase::Combat
        };
        assert_eq!(phase(&candidate).unwrap(), expected_phase);
        assert_eq!(receipt.phase, expected_phase as i32);
        assert_eq!(
            candidate
                .state()
                .narrative
                .open_threads
                .contains(&model::content(THREAT_THREAD).unwrap()),
            !expected.knocked_out
        );
        assert!(
            candidate
                .state()
                .narrative
                .open_threads
                .contains(&model::content(PACKET_THREAD).unwrap())
        );
        assert_eq!(
            candidate.state().draws.len(),
            before.state().draws.len() + faces.len()
        );
        for (ordinal, face) in faces.iter().enumerate() {
            let draw = &candidate.state().draws[before.state().draws.len() + ordinal];
            assert_eq!(draw.operation, command_operation(&command));
            assert_eq!(draw.ordinal as usize, ordinal);
            assert_eq!(draw.value, *face);
            assert_eq!(draw.source, rule().unwrap());
        }
        assert_eq!(
            &candidate.state().facts[..before.state().facts.len()],
            before.state().facts
        );
        assert_eq!(current, before);
        let repeat_action = attack_input(&candidate, 9, savage, graze);
        assert!(
            stage_with_supplier(&candidate, &repeat_action, &mut |_| panic!(
                "spent/complete attack cannot draw"
            ))
            .is_err()
        );
    }
}

#[test]
fn combat_command_from_older_revision_is_refused_before_drawing() {
    let current = combat();
    let before = current.state().clone();
    let GameInput::Game(mut command) = attack_input(&current, 8, false, false) else {
        panic!("player command")
    };
    let stale_revision = df_types::SessionRevision::new(
        command.basis.revision.epoch(),
        command.basis.revision.sequence() - 1,
    );
    command.basis.revision = stale_revision;
    command.observed_revision = stale_revision;
    let mut state = current.state().clone();
    let mut draws = vec![];
    let mut outcomes = vec![];

    assert_eq!(
        combat_transition::stage(
            &current,
            &command,
            &mut state,
            &mut draws,
            &mut outcomes,
            &mut |_| panic!("stale attack cannot draw")
        ),
        Err(RepositoryError::InvalidCandidate)
    );
    assert_eq!(state, before);
    assert!(draws.is_empty());
    assert!(outcomes.is_empty());
}

fn unavailable(current: &Checkpoint, case: usize) -> Checkpoint {
    let mut state = current.state().clone();
    let actor = entity(ENTITIES[0]).unwrap();
    let weapon = held_weapon_id(actor).unwrap();
    match case {
        0 => {
            let held = state
                .entities
                .iter_mut()
                .find(|entity| entity.id == weapon)
                .unwrap();
            held.location = Some(room_entity().unwrap());
            held.position = Some(Position { x: 0, y: 5, z: 0 });
        }
        1 => set(&mut state, actor, "held-weapon", 0).unwrap(),
        2 => {
            state
                .entities
                .iter_mut()
                .find(|entry| entry.id == entity(BANDIT).unwrap())
                .unwrap()
                .position = Some(Position { x: 0, y: 10, z: 0 })
        }
        3 => {
            state
                .entities
                .iter_mut()
                .find(|entry| entry.id == entity(BANDIT).unwrap())
                .unwrap()
                .location = None
        }
        4 => {
            state
                .inventory
                .iter_mut()
                .find(|item| item.item == weapon)
                .unwrap()
                .owner = entity(ENTITIES[1]).unwrap()
        }
        5 => {
            state
                .inventory
                .iter_mut()
                .find(|item| item.item == weapon)
                .unwrap()
                .origin = model::content("flail").unwrap()
        }
        6 => {
            state
                .inventory
                .iter_mut()
                .find(|item| item.item == weapon)
                .unwrap()
                .source = model::rule().unwrap()
        }
        7 => {
            state
                .inventory
                .iter_mut()
                .find(|item| item.item == weapon)
                .unwrap()
                .quantity = 2
        }
        8 => {
            state.inventory.retain(|item| item.item != weapon);
            state.entities.retain(|entity| entity.id != weapon);
        }
        _ => unreachable!(),
    }
    model::checkpoint(current.basis(), state).unwrap()
}

#[test]
fn registered_player_attack_unavailable_weapon_or_target_never_spends_or_draws() {
    let current = combat();
    let actor = entity(ENTITIES[0]).unwrap();
    assert_eq!(combat_transition::available(&current, actor), Ok(true));
    for case in 0..9 {
        let changed = unavailable(&current, case);
        let before = changed.clone();
        assert!(
            matches!(
                combat_transition::available(&changed, actor),
                Ok(false) | Err(RepositoryError::InvalidCandidate)
            ),
            "case {case}"
        );
        assert!(
            stage_with_supplier(
                &changed,
                &attack_input(&changed, 8, false, false),
                &mut |_| panic!("unavailable attack cannot draw")
            )
            .is_err(),
            "case {case}"
        );
        assert_eq!(changed, before);
        assert_eq!(value(changed.state(), actor, "action-used").unwrap(), 0);
        assert_eq!(changed.state().draws, current.state().draws);
        assert_eq!(changed.state().facts, current.state().facts);
        assert_eq!(changed.state().decisions, current.state().decisions);
    }
}

#[test]
fn source_candidate_is_revalidated_against_working_copy_before_existing_attack() {
    let current = combat();
    let actor = entity(ENTITIES[0]).unwrap();
    assert_eq!(combat_transition::available(&current, actor), Ok(true));
    let changed = unavailable(&current, 0);
    let mut state = changed.state().clone();
    let before = state.clone();
    let command = attack_input(&current, 8, false, false);
    let GameInput::Game(command) = command else {
        panic!("player command")
    };
    let mut draws = vec![];
    let mut outcomes = vec![];
    assert_eq!(
        combat_transition::stage(
            &current,
            &command,
            &mut state,
            &mut draws,
            &mut outcomes,
            &mut |_| panic!("dropped weapon cannot draw")
        ),
        Err(RepositoryError::InvalidCandidate)
    );
    assert_eq!(state, before);
    assert!(draws.is_empty());
    assert!(outcomes.is_empty());
}

// Classified in-memory receipt/commit ports exercise the real DurableOwner and registered
// Engine transition. This is not physical PostgreSQL, authentication or service qualification.
#[derive(Clone, Eq, PartialEq)]
struct Scope {
    input: GameInput,
}
impl ActorInput for Scope {
    fn retained_heap_bytes(&self) -> Option<usize> {
        self.input.retained_heap_bytes()
    }
}
impl OperationScope for Scope {
    type UncertaintyKey = Scope;
    fn capture_uncertainty_key(&self, maximum: usize) -> Result<Self, RepositoryError> {
        if self
            .input
            .retained_bytes()
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .is_none_or(|bytes| bytes > maximum)
        {
            return Err(RepositoryError::Capacity);
        }
        Ok(self.clone())
    }
    fn session(&self) -> SessionId {
        let GameInput::Game(command) = &self.input else {
            panic!("player command")
        };
        command.basis.session
    }
    fn operation(&self) -> OperationId {
        command_operation(&self.input)
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
fn command_operation(input: &GameInput) -> OperationId {
    let GameInput::Game(command) = input else {
        panic!("player command")
    };
    command.operation
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Failure {
    None,
    BeforeCommit,
    LostAck,
    UnknownNotCommitted,
}
struct Stored {
    checkpoint: Checkpoint,
    ledger: Vec<(Scope, DecisionReceipt)>,
    failure: Failure,
    faces: VecDeque<u32>,
    samples: Vec<(u32, u32)>,
    decisions: usize,
    commits: usize,
    publications: usize,
    refuse_reload: bool,
}
struct Repository(Rc<RefCell<Stored>>);
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
        let mut db = self.0.borrow_mut();
        db.commits += 1;
        if expected != db.checkpoint.basis() {
            return Err(RepositoryError::RevisionConflict);
        }
        if db.failure == Failure::BeforeCommit {
            return Err(RepositoryError::Unavailable);
        }
        if db.failure == Failure::UnknownNotCommitted {
            return Ok(CommitOutcome::Indeterminate);
        }
        let decision = candidate
            .state()
            .decisions
            .iter()
            .find(|decision| decision.operation == scope.operation())
            .unwrap()
            .clone();
        let receipt = DecisionReceipt::new(candidate.basis(), decision, 64 * 1024)?;
        db.checkpoint = candidate.clone();
        db.ledger.push((scope.clone(), receipt.clone()));
        Ok(if db.failure == Failure::LostAck {
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
        let db = self.0.borrow();
        if db.refuse_reload {
            Err(RepositoryError::Unavailable)
        } else {
            Ok(db.checkpoint.clone())
        }
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
        self.0.borrow_mut().decisions += 1;
        stage_with_supplier(current, input, &mut |sides| {
            let mut db = self.0.borrow_mut();
            let face = db
                .faces
                .pop_front()
                .ok_or(RepositoryError::InvalidCandidate)?;
            db.samples.push((sides, face));
            Ok(face)
        })
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        phase(checkpoint)?;
        Ok(())
    }
}
struct Publication(Rc<RefCell<Stored>>);
impl PublicationOwner<Scope> for Publication {
    fn publish_committed(
        &mut self,
        _: &Scope,
        checkpoint: &Checkpoint,
    ) -> Result<(), DeliveryError> {
        let mut db = self.0.borrow_mut();
        assert_eq!(checkpoint, &db.checkpoint);
        db.publications += 1;
        Ok(())
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        Ok(())
    }
}
type Owner = DurableOwner<Repository, Engine, Publication>;
fn setup(
    current: Checkpoint,
    failure: Failure,
    faces: &[u32],
) -> (Owner, Rc<RefCell<Stored>>, Scope) {
    let scope = Scope {
        input: attack_input(&current, 8, false, false),
    };
    let db = Rc::new(RefCell::new(Stored {
        checkpoint: current.clone(),
        ledger: vec![],
        failure,
        faces: faces.iter().copied().collect(),
        samples: vec![],
        decisions: 0,
        commits: 0,
        publications: 0,
        refuse_reload: false,
    }));
    let owner = DurableOwner::new(
        Repository(Rc::clone(&db)),
        Engine(Rc::clone(&db)),
        Publication(Rc::clone(&db)),
        current,
        64 * 1024,
    )
    .unwrap();
    (owner, db, scope)
}
fn submit(owner: &mut Owner, scope: &Scope, deliver: bool) -> Option<SubmissionOutcome> {
    let (input, wait) = OwnedInput::new(
        OperationContext {
            trace_parent: String::new(),
            build: "native-greatsword-consumer".to_owned(),
        },
        scope.clone(),
        scope.input.clone(),
    );
    if !deliver {
        drop(wait);
        owner.reduce(AdmissionSequence(1), input);
        return None;
    }
    owner.reduce(AdmissionSequence(1), input);
    Some(wait.try_recv().unwrap())
}
fn committed_hit(before: &Checkpoint, after: &Checkpoint) {
    assert_eq!(
        value(after.state(), entity(BANDIT).unwrap(), "hit-points").unwrap(),
        5
    );
    assert_eq!(
        value(after.state(), entity(ENTITIES[0]).unwrap(), "action-used").unwrap(),
        1
    );
    assert_eq!(phase(after).unwrap(), rpc::JourneyPhase::Combat);
    assert_eq!(after.state().draws.len(), before.state().draws.len() + 3);
    assert_eq!(
        &after.state().facts[..before.state().facts.len()],
        before.state().facts
    );
    assert_eq!(
        &after.state().decisions[..before.state().decisions.len()],
        before.state().decisions
    );
    let receipt = accepted(after.state().decisions.last().unwrap()).unwrap();
    assert_eq!(receipt.combat.len(), 1);
    assert_eq!(receipt.combat[0].damage, 6);
    assert_eq!(receipt.combat[0].damage_dice, [1, 2]);
}

#[test]
fn registered_greatsword_session_commit_exact_retry_and_cancelled_receipt_do_not_redraw() {
    for deliver in [true, false] {
        let (mut owner, db, scope) = setup(combat(), Failure::None, &[10, 1, 2]);
        let before = owner.checkpoint().clone();
        let first = submit(&mut owner, &scope, deliver);
        let replay = submit(&mut owner, &scope, true).unwrap();
        assert!(matches!(replay, SubmissionOutcome::Confirmed(_)));
        if deliver {
            assert_eq!(Some(replay), first);
        }
        let stored = db.borrow();
        committed_hit(&before, &stored.checkpoint);
        assert_eq!(owner.checkpoint(), &stored.checkpoint);
        assert_eq!(
            (
                stored.decisions,
                stored.commits,
                stored.publications,
                stored.samples.len(),
                stored.ledger.len()
            ),
            (1, 1, 1, 3, 1)
        );
        let accepted_state = stored.checkpoint.clone();
        drop(stored);
        let mut conflict = scope.clone();
        let GameInput::Game(command) = &mut conflict.input else {
            panic!("player command")
        };
        let GameCommand::ProposeAction { choices, .. } = &mut command.command else {
            panic!("attack")
        };
        choices[0].1 = model::label("yes").unwrap();
        assert_eq!(
            submit(&mut owner, &conflict, true),
            Some(SubmissionOutcome::OperationConflict)
        );
        assert_eq!(db.borrow().checkpoint, accepted_state);
        assert_eq!(db.borrow().decisions, 1);
        assert_eq!(db.borrow().samples.len(), 3);
    }
}

#[test]
fn registered_greatsword_session_lost_ack_is_resolved_only_by_receipt_and_reload() {
    let (mut owner, db, scope) = setup(combat(), Failure::LostAck, &[10, 1, 2]);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    assert!(owner.has_uncertain_operation());
    assert_eq!(owner.checkpoint(), &before);
    committed_hit(&before, &db.borrow().checkpoint);
    let accepted_state = db.borrow().checkpoint.clone();
    let mut other = scope.clone();
    let GameInput::Game(command) = &mut other.input else {
        panic!("player command")
    };
    command.operation = OperationId::from_bytes(&[9; 16]).unwrap();
    assert_eq!(
        submit(&mut owner, &other, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    db.borrow_mut().refuse_reload = true;
    let receipt = submit(&mut owner, &scope, true).unwrap();
    assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
    assert_eq!(owner.checkpoint(), &before);
    assert!(owner.has_uncertain_operation());
    db.borrow_mut().refuse_reload = false;
    assert_eq!(submit(&mut owner, &scope, true), Some(receipt));
    assert!(owner.is_current());
    assert_eq!(owner.checkpoint(), &accepted_state);
    let stored = db.borrow();
    assert_eq!(stored.checkpoint, accepted_state);
    assert_eq!(
        (
            stored.decisions,
            stored.commits,
            stored.publications,
            stored.samples.len(),
            stored.ledger.len()
        ),
        (1, 1, 0, 3, 1)
    );
}

#[test]
fn registered_greatsword_session_refused_commit_keeps_attack_unapplied() {
    let (mut owner, db, scope) = setup(combat(), Failure::BeforeCommit, &[10, 1, 2, 10, 1, 2]);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Refused(RepositoryError::Unavailable))
    );
    assert_eq!(owner.checkpoint(), &before);
    assert_eq!(db.borrow().checkpoint, before);
    assert_eq!(db.borrow().publications, 0);
    db.borrow_mut().failure = Failure::None;
    assert!(matches!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Confirmed(_))
    ));
    committed_hit(&before, &db.borrow().checkpoint);
    assert_eq!(
        (
            db.borrow().decisions,
            db.borrow().commits,
            db.borrow().samples.len(),
            db.borrow().ledger.len()
        ),
        (2, 2, 6, 1)
    );
}

#[test]
fn registered_greatsword_session_unknown_commit_never_consumes_a_second_attack() {
    let (mut owner, db, scope) = setup(combat(), Failure::UnknownNotCommitted, &[10, 1, 2]);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    db.borrow_mut().failure = Failure::None;
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    assert!(owner.has_uncertain_operation());
    assert_eq!(owner.checkpoint(), &before);
    let stored = db.borrow();
    assert_eq!(stored.checkpoint, before);
    assert_eq!(
        (
            stored.decisions,
            stored.commits,
            stored.publications,
            stored.samples.len(),
            stored.ledger.len()
        ),
        (1, 1, 0, 3, 0)
    );
}

#[test]
fn registered_greatsword_session_unavailable_attack_has_no_commit_or_draw() {
    let current = combat();
    for case in [0, 2] {
        let changed = unavailable(&current, case);
        let (mut owner, db, scope) = setup(changed.clone(), Failure::None, &[]);
        assert_eq!(
            submit(&mut owner, &scope, true),
            Some(SubmissionOutcome::Refused(
                RepositoryError::InvalidCandidate
            ))
        );
        assert_eq!(owner.checkpoint(), &changed);
        let stored = db.borrow();
        assert_eq!(stored.checkpoint, changed);
        assert_eq!(
            (
                stored.decisions,
                stored.commits,
                stored.publications,
                stored.samples.len(),
                stored.ledger.len()
            ),
            (1, 0, 0, 0, 0)
        );
    }
}
