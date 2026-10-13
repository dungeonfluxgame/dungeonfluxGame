use super::*;
use df_observe::OperationContext;
use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
use df_session::submission::*;
use df_types::SessionId;
use std::cell::RefCell;
use std::rc::Rc;

fn first() -> MemberId {
    MemberId::from_bytes(&MEMBERS[0]).unwrap()
}

fn before_outcome(victory: bool) -> Checkpoint {
    let opening = tests::opening_story();
    let dialogue = stage_with_supplier(
        &opening,
        &tests::input(&opening, first(), 6, "escort-courier", vec![]),
        &mut |_| panic!("dialogue has no draws"),
    )
    .unwrap();
    let mut initiative = 0;
    stage_with_supplier(
        &dialogue,
        &tests::input(&dialogue, first(), 7, "defend-courier", vec![]),
        &mut |sides| {
            if sides == 20 && initiative < 3 {
                initiative += 1;
                Ok(
                    if (victory && initiative == 1) || (!victory && initiative == 3) {
                        20
                    } else {
                        1
                    },
                )
            } else {
                Ok(if sides == 20 { 20 } else { 6 })
            }
        },
    )
    .unwrap()
}

fn outcome_input(before: &Checkpoint, victory: bool) -> GameInput {
    let member = MemberId::from_bytes(&MEMBERS[usize::from(!victory)]).unwrap();
    tests::input(
        before,
        member,
        8,
        if victory {
            "greatsword-attack"
        } else {
            "end-turn"
        },
        if victory {
            vec![
                (
                    model::label("savage-attacker").unwrap(),
                    model::label("no").unwrap(),
                ),
                (model::label("graze").unwrap(), model::label("no").unwrap()),
            ]
        } else {
            vec![]
        },
    )
}

fn completed(victory: bool) -> (Checkpoint, GameInput, Checkpoint) {
    let before = before_outcome(victory);
    let input = outcome_input(&before, victory);
    let after = stage_with_supplier(&before, &input, &mut |sides| {
        Ok(if sides == 20 { 20 } else { 6 })
    })
    .unwrap();
    (before, input, after)
}

fn terminal_fact(current: &Checkpoint) -> &GameFact {
    current
        .state()
        .facts
        .iter()
        .find(|fact| {
            matches!(&fact.value,
        FactValue::ContentEvent { definition, .. }
        if ["combat-victory", "combat-defeat"].contains(&definition.entry.as_str()))
        })
        .unwrap()
}

fn assert_terminal(before: &Checkpoint, input: &GameInput, after: &Checkpoint, victory: bool) {
    let command = command(input).unwrap();
    let GameCommand::ProposeAction { actor, action, .. } = &command.command else {
        panic!("action")
    };
    let cause = terminal_fact(after);
    let decision = after.state().decisions.last().unwrap();
    let last = after.state().facts.last().unwrap();
    assert_eq!(
        after.state().encounters[0].objectives,
        [model::content(if victory {
            "combat-victory"
        } else {
            "combat-defeat"
        })
        .unwrap()]
    );
    assert_eq!(after.state().encounters[0].active_turn, None);
    assert_eq!(
        cause.value,
        FactValue::ContentEvent {
            definition: model::content(if victory {
                "combat-victory"
            } else {
                "combat-defeat"
            })
            .unwrap(),
            subjects: vec![*actor],
        }
    );
    assert_eq!(cause.audience, AudienceScope::Shared);
    assert_eq!(cause.operation, command.operation);
    assert_eq!(cause.revision, after.basis().revision);
    assert_eq!(last.ordinal, cause.ordinal + 1);
    assert_eq!(last.cause, Some(cause.id));
    assert_eq!(
        last.value,
        FactValue::ContentEvent {
            definition: action.clone(),
            subjects: vec![]
        }
    );
    assert_eq!(decision.facts.get(cause.ordinal as usize), Some(&cause.id));
    assert_eq!(decision.facts.last(), Some(&last.id));
    assert_eq!(decision.operation, command.operation);
    assert_eq!(decision.revision, after.basis().revision);
    assert_eq!(decision.source_policy.as_str(), THREAD_POLICY);
    assert_eq!(
        after.basis().revision,
        before.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(
        after.state().decisions.len(),
        before.state().decisions.len() + 1
    );
    assert_eq!(
        &after.state().facts[..before.state().facts.len()],
        before.state().facts
    );
    assert_eq!(
        &after.state().decisions[..before.state().decisions.len()],
        before.state().decisions
    );
    assert_eq!(after.state().members, before.state().members);
    assert_eq!(after.state().characters, before.state().characters);
    assert_eq!(after.state().knowledge, before.state().knowledge);
    let mut continuity = before.state().continuity.clone();
    continuity.catch_up = after.state().continuity.catch_up.clone();
    assert_eq!(after.state().continuity, continuity);
    if victory {
        assert_eq!(after.state().logical_time, before.state().logical_time);
    } else {
        assert_eq!(
            after.state().logical_time.ticks,
            before.state().logical_time.ticks + 6
        );
        assert_eq!(
            after.state().logical_time.ticks_per_second,
            before.state().logical_time.ticks_per_second
        );
    }
    assert_eq!(decision.effects, []);
    assert!(!after.state().narrative.accepted_facts.contains(&cause.id));
    assert_eq!(
        after.state().narrative.accepted_facts.last(),
        Some(&last.id)
    );
    assert_eq!(combat_victory(after.state()), Ok(victory));
    assert_eq!(phase(after), Ok(rpc::JourneyPhase::Complete));
    assert_eq!(
        combat_round(after).unwrap(),
        combat_round(before).unwrap() + u32::from(!victory)
    );
}

#[test]
fn objective_nonterminal_registered_round_keeps_the_active_slot_and_has_no_terminal_event() {
    let before = before_outcome(true);
    let member = first();
    let action = tests::input(
        &before,
        member,
        8,
        "greatsword-attack",
        vec![
            (
                model::label("savage-attacker").unwrap(),
                model::label("no").unwrap(),
            ),
            (model::label("graze").unwrap(), model::label("no").unwrap()),
        ],
    );
    let after = stage_with_supplier(&before, &action, &mut |sides| {
        Ok(if sides == 20 { 10 } else { 1 })
    })
    .unwrap();
    assert_eq!(phase(&after), Ok(rpc::JourneyPhase::Combat));
    assert_eq!(
        after.state().encounters[0].objectives,
        [model::content("defend-courier").unwrap()]
    );
    assert!(after.state().facts.iter().all(|fact| !matches!(&fact.value,
        FactValue::ContentEvent { definition, .. }
        if ["combat-victory", "combat-defeat"].contains(&definition.entry.as_str()))));
    assert_eq!(
        &after.state().facts[..before.state().facts.len()],
        before.state().facts
    );
    assert_eq!(
        after.state().decisions.len(),
        before.state().decisions.len() + 1
    );
    assert_eq!(
        after.state().narrative.accepted_facts.last(),
        after.state().facts.last().map(|fact| &fact.id)
    );
}

#[test]
fn objective_registered_victory_and_npc_defeat_share_the_single_combat_decision() {
    for victory in [true, false] {
        let (before, input, after) = completed(victory);
        assert_terminal(&before, &input, &after, victory);
        let receipt = accepted(after.state().decisions.last().unwrap()).unwrap();
        if !victory {
            let GameCommand::ProposeAction { actor, .. } = &command(&input).unwrap().command else {
                panic!("action")
            };
            assert_ne!(*actor, entity(BANDIT).unwrap());
            assert!(
                receipt
                    .combat
                    .iter()
                    .all(|outcome| outcome.actor_id.as_slice() == BANDIT.as_slice())
            );
        }
    }
}

#[test]
fn objective_registered_rest_and_inn_preserve_the_terminal_and_original_history() {
    for victory in [true, false] {
        let (_, _, complete) = completed(victory);
        let cause = terminal_fact(&complete).clone();
        // Spend a Second Wind use on paper so the rest has a use to
        // regain; the played fight leaves both heroes untouched.
        let mut spent = complete.state().clone();
        let hero = spent.characters[0].entity;
        set(&mut spent, hero, "second-wind", 1).unwrap();
        let complete = model::checkpoint(complete.basis(), spent).unwrap();
        let input = tests::input(&complete, first(), 9, "short-rest", vec![]);
        let rest =
            stage_with_supplier(&complete, &input, &mut |_| panic!("rest cannot sample")).unwrap();
        assert_eq!(terminal_fact(&rest), &cause);
        assert_eq!(
            rest.state().encounters[0].objectives,
            complete.state().encounters[0].objectives
        );
        assert_eq!(
            &rest.state().facts[..complete.state().facts.len()],
            complete.state().facts
        );
        assert_eq!(phase(&rest), Ok(rpc::JourneyPhase::Complete));
        assert_eq!(combat_victory(rest.state()), Ok(victory));
        let restored = model::checkpoint(rest.basis(), rest.state().clone()).unwrap();
        assert_eq!(phase(&restored), Ok(rpc::JourneyPhase::Complete));
        if victory {
            for current in [&complete, &restored] {
                let arrived = stage_with_supplier(
                    current,
                    &tests::inn_command(current, first(), 10),
                    &mut |_| panic!("Inn cannot sample"),
                )
                .unwrap();
                assert_eq!(terminal_fact(&arrived), &cause);
                assert_eq!(arrived.state().resources, current.state().resources);
                assert_eq!(arrived.state().logical_time, current.state().logical_time);
                assert_eq!(arrived.state().knowledge, current.state().knowledge);
                assert_eq!(phase(&arrived), Ok(rpc::JourneyPhase::Complete));
            }
        } else {
            assert!(
                !offered(&restored, first())
                    .unwrap()
                    .iter()
                    .any(|(kind, _)| *kind == rpc::GameplayActionKind::ChooseHarborScene)
            );
        }
    }
}

// Reconstruct the baseline's committed pair representation and native op/ordinal IDs,
// not a migration of different build/source pins. No live historical record is rewritten.
fn legacy(current: &Checkpoint, victory: bool) -> Checkpoint {
    use sha2::{Digest, Sha256};
    let mut state = current.state().clone();
    let cause = terminal_fact(current);
    let action = state.facts.last_mut().unwrap();
    let old_id = action.id;
    action.ordinal -= 1;
    let digest = Sha256::digest(
        [
            action.operation.as_bytes().as_slice(),
            action.ordinal.to_le_bytes().as_slice(),
        ]
        .concat(),
    );
    action.id = FactId::from_bytes(&digest[..16]).unwrap();
    action.cause = cause.cause;
    let new_id = action.id;
    state.facts.retain(|fact| fact.id != cause.id);
    let decision = state.decisions.last_mut().unwrap();
    decision.facts.retain(|id| *id != cause.id);
    *decision.facts.last_mut().unwrap() = new_id;
    *state
        .narrative
        .accepted_facts
        .iter_mut()
        .find(|id| **id == old_id)
        .unwrap() = new_id;
    state.encounters[0].objectives = vec![
        model::content("defend-courier").unwrap(),
        model::content(if victory {
            "combat-victory"
        } else {
            "combat-defeat"
        })
        .unwrap(),
    ];
    model::checkpoint(current.basis(), state).unwrap()
}

#[test]
fn objective_legacy_committed_pairs_remain_readable_without_a_new_terminal_event() {
    for victory in [true, false] {
        let (_, _, complete) = completed(victory);
        let old = legacy(&complete, victory);
        assert_eq!(phase(&old), Ok(rpc::JourneyPhase::Complete));
        assert_eq!(combat_victory(old.state()), Ok(victory));
        assert_eq!(combat_round(&old), combat_round(&complete));
        // Spend a Second Wind use on paper so the rest has a use to regain.
        let mut spent = old.state().clone();
        let hero = spent.characters[0].entity;
        set(&mut spent, hero, "second-wind", 1).unwrap();
        let old = model::checkpoint(old.basis(), spent).unwrap();
        let before = old.clone();
        let rest = stage_with_supplier(
            &old,
            &tests::input(&old, first(), 9, "short-rest", vec![]),
            &mut |_| panic!("legacy rest cannot sample"),
        )
        .unwrap();
        assert_eq!(
            rest.state().encounters[0].objectives,
            old.state().encounters[0].objectives
        );
        assert_eq!(
            &rest.state().facts[..old.state().facts.len()],
            old.state().facts
        );
        assert_eq!(phase(&rest), Ok(rpc::JourneyPhase::Complete));
        if victory {
            let arrived =
                stage_with_supplier(&rest, &tests::inn_command(&rest, first(), 10), &mut |_| {
                    panic!("legacy Inn cannot sample")
                })
                .unwrap();
            assert_eq!(
                arrived.state().encounters[0].objectives,
                old.state().encounters[0].objectives
            );
        }
        assert_eq!(old, before);
    }
}

fn refuses_without_sampling(current: &Checkpoint) {
    let original = current.clone();
    let input = tests::input(current, first(), 30, "short-rest", vec![]);
    assert_eq!(
        stage_with_supplier(current, &input, &mut |_| panic!("refused before sampling")),
        Err(RepositoryError::InvalidCandidate)
    );
    assert_eq!(current, &original);
}

#[test]
fn objective_forged_terminal_actor_member_cause_and_receipt_refuse_before_side_effects() {
    for victory in [true, false] {
        let (_, _, complete) = completed(victory);
        for case in 0..11 {
            let mut state = complete.state().clone();
            let id = terminal_fact(&complete).id;
            match case {
                0 => {
                    state
                        .facts
                        .iter_mut()
                        .find(|fact| fact.id == id)
                        .unwrap()
                        .cause = None
                }
                1 => {
                    state
                        .facts
                        .iter_mut()
                        .find(|fact| fact.id == id)
                        .unwrap()
                        .audience = AudienceScope::Host
                }
                2 | 3 => {
                    let FactValue::ContentEvent { subjects, .. } = &mut state
                        .facts
                        .iter_mut()
                        .find(|fact| fact.id == id)
                        .unwrap()
                        .value
                    else {
                        panic!("terminal")
                    };
                    *subjects = vec![
                        entity(if case == 2 {
                            BANDIT
                        } else {
                            ENTITIES[usize::from(victory)]
                        })
                        .unwrap(),
                    ];
                }
                4 => {
                    let first = state
                        .members
                        .iter()
                        .position(|link| link.member.as_bytes() == &MEMBERS[0])
                        .unwrap();
                    let second = state
                        .members
                        .iter()
                        .position(|link| link.member.as_bytes() == &MEMBERS[1])
                        .unwrap();
                    let first_actor = state.members[first].character;
                    state.members[first].character = state.members[second].character;
                    state.members[second].character = first_actor;
                    for character in &mut state.characters {
                        character.owner = state
                            .members
                            .iter()
                            .find(|link| link.character == Some(character.entity))
                            .unwrap()
                            .member;
                        for choice in &mut character.choices {
                            choice.participant = character.owner;
                        }
                    }
                }
                5 => {
                    state.decisions.last_mut().unwrap().source_policy =
                        model::label("room").unwrap()
                }
                6 => {
                    let decision = state.decisions.last_mut().unwrap();
                    let mut receipt = accepted(decision).unwrap();
                    receipt.combat.clear();
                    decision.semantic_output = Some(hex(&receipt.encode_to_vec()));
                }
                7 => {
                    let fact = state.facts.iter_mut().find(|fact| fact.operation == complete.state().decisions.last().unwrap().operation
                        && matches!(&fact.value, FactValue::ResourceChanged { resource, .. } if resource.as_str() == "unconscious")).unwrap();
                    let FactValue::ResourceChanged { source, .. } = &mut fact.value else {
                        panic!("knockout")
                    };
                    *source = model::rule().unwrap();
                }
                8 => {
                    let FactValue::ContentEvent { definition, .. } = &mut state
                        .facts
                        .iter_mut()
                        .find(|fact| fact.id == id)
                        .unwrap()
                        .value
                    else {
                        panic!("terminal")
                    };
                    *definition = model::content("short-rest-complete").unwrap();
                }
                9 => {
                    let FactValue::ContentEvent { definition, .. } = &mut state
                        .facts
                        .iter_mut()
                        .find(|fact| fact.id == id)
                        .unwrap()
                        .value
                    else {
                        panic!("terminal")
                    };
                    *definition = model::content(if victory {
                        "combat-defeat"
                    } else {
                        "combat-victory"
                    })
                    .unwrap();
                }
                _ => {
                    let original = state.facts.iter_mut().find(|fact| matches!(&fact.value, FactValue::ContentEvent { definition, .. } if definition.entry.as_str() == "defend-courier")).unwrap();
                    let FactValue::ContentEvent { definition, .. } = &mut original.value else {
                        panic!("defend")
                    };
                    *definition = model::content("end-turn").unwrap();
                }
            }
            let forged = model::checkpoint(complete.basis(), state).unwrap();
            refuses_without_sampling(&forged);
        }
        let old = legacy(&complete, victory);
        let mut state = old.state().clone();
        state.encounters[0].objectives.remove(0);
        refuses_without_sampling(&model::checkpoint(old.basis(), state).unwrap());
    }
}

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
        command(&self.input).unwrap().basis.session
    }
    fn operation(&self) -> OperationId {
        command(&self.input).unwrap().operation
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
#[derive(Clone, Copy, Eq, PartialEq)]
enum Failure {
    None,
    BeforeCommit,
    LostAck,
    UnknownNotCommitted,
}
struct Database {
    checkpoint: Checkpoint,
    ledger: Vec<(Scope, DecisionReceipt)>,
    failure: Failure,
    decisions: usize,
    commits: usize,
    publications: usize,
    samples: usize,
    refuse_reload: bool,
    rewrite_prior: bool,
}
struct Repository {
    database: Rc<RefCell<Database>>,
}
impl SessionRepository for Repository {
    type Scope = Scope;
    fn lookup_operation(
        &mut self,
        scope: &Scope,
        _: &OperationContext,
    ) -> Result<OperationLookup, RepositoryError> {
        let db = self.database.borrow();
        if let Some((prior, receipt)) = db
            .ledger
            .iter()
            .find(|(prior, _)| prior.operation() == scope.operation())
        {
            return Ok(if prior == scope {
                OperationLookup::Committed(receipt.clone())
            } else {
                OperationLookup::Conflict
            });
        }
        Ok(OperationLookup::NotRecorded)
    }
    fn commit_decision(
        &mut self,
        scope: &Scope,
        candidate: &Checkpoint,
        expected: Basis,
        _: &OperationContext,
    ) -> Result<CommitOutcome, RepositoryError> {
        let mut db = self.database.borrow_mut();
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
        let decision = candidate.state().decisions.last().unwrap().clone();
        assert_eq!(decision.operation, scope.operation());
        let receipt = DecisionReceipt::new(candidate.basis(), decision, 64 * 1024)?;
        db.checkpoint = candidate.clone();
        db.ledger.push((scope.clone(), receipt.clone()));
        if db.failure == Failure::LostAck {
            Ok(CommitOutcome::Indeterminate)
        } else {
            Ok(CommitOutcome::Confirmed(receipt))
        }
    }
    fn load_current(
        &mut self,
        _: &Scope,
        _: &OperationContext,
    ) -> Result<Checkpoint, RepositoryError> {
        let db = self.database.borrow();
        if db.refuse_reload {
            Err(RepositoryError::Unavailable)
        } else {
            Ok(db.checkpoint.clone())
        }
    }
}
struct Engine {
    database: Rc<RefCell<Database>>,
}
impl SessionEngine<Scope> for Engine {
    fn decide(
        &mut self,
        current: &Checkpoint,
        _: &Scope,
        input: &GameInput,
    ) -> Result<Checkpoint, RepositoryError> {
        self.database.borrow_mut().decisions += 1;
        let candidate = stage_with_supplier(current, input, &mut |sides| {
            self.database.borrow_mut().samples += 1;
            Ok(if sides == 20 { 20 } else { 6 })
        })?;
        if self.database.borrow().rewrite_prior {
            let mut state = candidate.state().clone();
            state.facts.first_mut().unwrap().audience = AudienceScope::Host;
            model::checkpoint(candidate.basis(), state)
        } else {
            Ok(candidate)
        }
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        phase(checkpoint)?;
        Ok(())
    }
}
struct Publication {
    database: Rc<RefCell<Database>>,
}
impl PublicationOwner<Scope> for Publication {
    fn publish_committed(
        &mut self,
        _: &Scope,
        checkpoint: &Checkpoint,
    ) -> Result<(), DeliveryError> {
        let mut db = self.database.borrow_mut();
        assert_eq!(checkpoint, &db.checkpoint);
        assert_eq!(phase(checkpoint), Ok(rpc::JourneyPhase::Complete));
        assert_eq!(checkpoint.state().encounters[0].objectives.len(), 1);
        db.publications += 1;
        Ok(())
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        Ok(())
    }
}
type Owner = DurableOwner<Repository, Engine, Publication>;
fn setup(victory: bool, failure: Failure) -> (Owner, Rc<RefCell<Database>>, Scope) {
    let current = before_outcome(victory);
    let scope = Scope {
        input: outcome_input(&current, victory),
    };
    let database = Rc::new(RefCell::new(Database {
        checkpoint: current.clone(),
        ledger: vec![],
        failure,
        decisions: 0,
        commits: 0,
        publications: 0,
        samples: 0,
        refuse_reload: false,
        rewrite_prior: false,
    }));
    let owner = DurableOwner::new(
        Repository {
            database: Rc::clone(&database),
        },
        Engine {
            database: Rc::clone(&database),
        },
        Publication {
            database: Rc::clone(&database),
        },
        current,
        64 * 1024,
    )
    .unwrap();
    (owner, database, scope)
}
fn submit(owner: &mut Owner, scope: &Scope, deliver: bool) -> Option<SubmissionOutcome> {
    let (input, wait) = OwnedInput::new(
        OperationContext {
            trace_parent: String::new(),
            build: "native-objective-consumer".to_owned(),
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

#[test]
fn objective_session_confirmed_and_cancelled_receipts_retry_without_deciding_or_sampling() {
    for victory in [true, false] {
        for deliver in [true, false] {
            let (mut owner, database, scope) = setup(victory, Failure::None);
            let before = owner.checkpoint().clone();
            let first = submit(&mut owner, &scope, deliver);
            let repeated = submit(&mut owner, &scope, true).unwrap();
            assert!(matches!(repeated, SubmissionOutcome::Confirmed(_)));
            if deliver {
                assert_eq!(Some(repeated), first);
            }
            let db = database.borrow();
            assert_terminal(&before, &scope.input, &db.checkpoint, victory);
            assert_eq!(owner.checkpoint(), &db.checkpoint);
            assert_eq!(
                (db.decisions, db.commits, db.publications, db.ledger.len()),
                (1, 1, 1, 1)
            );
            assert_eq!(
                db.samples,
                db.checkpoint.state().draws.len() - before.state().draws.len()
            );
            drop(db);
            let mut conflict = scope.clone();
            let GameInput::Game(command) = &mut conflict.input else {
                panic!("command")
            };
            command.member = MemberId::from_bytes(&MEMBERS[usize::from(victory)]).unwrap();
            assert_eq!(
                submit(&mut owner, &conflict, true),
                Some(SubmissionOutcome::OperationConflict)
            );
            assert_eq!(database.borrow().decisions, 1);
        }
    }
}

#[test]
fn objective_session_rollback_publishes_nothing_and_safe_retry_commits_once() {
    let (mut owner, database, scope) = setup(true, Failure::BeforeCommit);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Refused(RepositoryError::Unavailable))
    );
    assert_eq!(owner.checkpoint(), &before);
    assert_eq!(database.borrow().checkpoint, before);
    assert_eq!(database.borrow().publications, 0);
    database.borrow_mut().failure = Failure::None;
    assert!(matches!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Confirmed(_))
    ));
    let db = database.borrow();
    assert_terminal(&before, &scope.input, &db.checkpoint, true);
    assert_eq!(
        (db.decisions, db.commits, db.publications, db.ledger.len()),
        (2, 2, 1, 1)
    );
}

#[test]
fn objective_session_lost_ack_requires_exact_lookup_and_qualified_reload_without_reproposal() {
    let (mut owner, database, scope) = setup(false, Failure::LostAck);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    assert!(owner.has_uncertain_operation());
    assert_eq!(owner.checkpoint(), &before);
    assert_terminal(&before, &scope.input, &database.borrow().checkpoint, false);
    let mut other = scope.clone();
    let GameInput::Game(command) = &mut other.input else {
        panic!("command")
    };
    command.operation = OperationId::from_bytes(&[30; 16]).unwrap();
    assert_eq!(
        submit(&mut owner, &other, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    database.borrow_mut().refuse_reload = true;
    let receipt = submit(&mut owner, &scope, true).unwrap();
    assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
    assert!(owner.has_uncertain_operation());
    assert_eq!(owner.checkpoint(), &before);
    database.borrow_mut().refuse_reload = false;
    assert_eq!(submit(&mut owner, &scope, true), Some(receipt));
    assert!(owner.is_current());
    let db = database.borrow();
    assert_eq!(owner.checkpoint(), &db.checkpoint);
    assert_eq!(
        (db.decisions, db.commits, db.publications, db.ledger.len()),
        (1, 1, 0, 1)
    );
}

#[test]
fn objective_session_unknown_not_recorded_never_reexecutes_or_publishes() {
    let (mut owner, database, scope) = setup(true, Failure::UnknownNotCommitted);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    database.borrow_mut().failure = Failure::None;
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    assert!(owner.has_uncertain_operation());
    let db = database.borrow();
    assert_eq!(owner.checkpoint(), &before);
    assert_eq!(db.checkpoint, before);
    assert_eq!(
        (db.decisions, db.commits, db.publications, db.ledger.len()),
        (1, 1, 0, 0)
    );
}

#[test]
fn objective_session_wrong_current_member_and_actor_refuse_before_sampler_commit_and_publication() {
    for victory in [true, false] {
        for wrong_member in [true, false] {
            let (mut owner, database, mut scope) = setup(victory, Failure::None);
            let before = owner.checkpoint().clone();
            let GameInput::Game(command) = &mut scope.input else {
                panic!("command")
            };
            let GameCommand::ProposeAction { actor, .. } = &mut command.command else {
                panic!("action")
            };
            if wrong_member {
                command.member = MemberId::from_bytes(&MEMBERS[usize::from(victory)]).unwrap();
                *actor = player_entity(command.member, &before).unwrap();
            } else {
                *actor = entity(BANDIT).unwrap();
            }
            assert!(matches!(
                submit(&mut owner, &scope, true),
                Some(SubmissionOutcome::Refused(_))
            ));
            let db = database.borrow();
            assert_eq!(owner.checkpoint(), &before);
            assert_eq!(db.checkpoint, before);
            assert_eq!(
                (
                    db.decisions,
                    db.commits,
                    db.publications,
                    db.samples,
                    db.ledger.len()
                ),
                (1, 0, 0, 0, 0)
            );
        }
    }
}

#[test]
fn objective_session_rejects_any_rewrite_of_accepted_history_before_commit() {
    let (mut owner, database, scope) = setup(true, Failure::None);
    let before = owner.checkpoint().clone();
    database.borrow_mut().rewrite_prior = true;
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Refused(
            RepositoryError::InvalidCandidate
        ))
    );
    let db = database.borrow();
    assert_eq!(owner.checkpoint(), &before);
    assert_eq!(db.checkpoint, before);
    assert_eq!(
        (db.decisions, db.commits, db.publications, db.ledger.len()),
        (1, 0, 0, 0)
    );
}

fn before_proposal(after: &Checkpoint, victory: bool) -> Checkpoint {
    let mut state = after.state().clone();
    state.encounters[0].objectives = vec![model::content("defend-courier").unwrap()];
    assert_eq!(
        state.narrative.accepted_facts.pop(),
        state.facts.last().map(|fact| fact.id)
    );
    if victory {
        state
            .narrative
            .open_threads
            .push(model::content(THREAT_THREAD).unwrap());
    }
    model::checkpoint(after.basis(), state).unwrap()
}

#[test]
fn objective_current_source_owner_admits_only_exact_current_command_and_unmutated_receipt() {
    for victory in [true, false] {
        let (before, input, after) = completed(victory);
        let proposal_input = before_proposal(&after, victory);
        let command = command(&input).unwrap();
        let admitted = objective_outcome::stage(&before, proposal_input.clone(), command).unwrap();
        let mut expected = proposal_input.state().clone();
        expected.encounters[0].objectives = after.state().encounters[0].objectives.clone();
        assert_eq!(admitted.state(), &expected);
        assert_eq!(admitted.basis(), proposal_input.basis());
        assert_eq!(admitted.pins(), proposal_input.pins());
        for case in 0..4 {
            let mut wrong = command.clone();
            let GameCommand::ProposeAction { actor, .. } = &mut wrong.command else {
                panic!("command")
            };
            match case {
                0 => wrong.operation = OperationId::from_bytes(&[30; 16]).unwrap(),
                1 => {
                    wrong.member = MemberId::from_bytes(&MEMBERS[usize::from(victory)]).unwrap();
                    *actor = player_entity(wrong.member, &before).unwrap();
                }
                2 => *actor = entity(BANDIT).unwrap(),
                _ => wrong.observed_revision = wrong.observed_revision.next_sequence().unwrap(),
            }
            let original = proposal_input.clone();
            assert_eq!(
                objective_outcome::stage(&before, proposal_input.clone(), &wrong),
                Err(RepositoryError::InvalidCandidate)
            );
            assert_eq!(proposal_input, original);
        }
        let mut state = proposal_input.state().clone();
        state.facts.first_mut().unwrap().audience = AudienceScope::Host;
        let rewritten = model::checkpoint(proposal_input.basis(), state).unwrap();
        assert_eq!(
            objective_outcome::stage(&before, rewritten, command),
            Err(RepositoryError::InvalidCandidate)
        );
    }
}

#[test]
fn objective_source_pin_changes_and_injected_terminal_events_never_create_a_candidate() {
    let (before, input, after) = completed(true);
    let pending = before_proposal(&after, true);
    let mut pins = before.pins().clone();
    pins.rules.source_manifest_digest = ContentDigest([99; 32]);
    let stale = Checkpoint::new(
        before.schema(),
        before.basis(),
        pins,
        before.state().clone(),
        ReferenceInventory {
            rules: &[model::rule().unwrap(), rule().unwrap()],
            content: &model::contents().unwrap(),
            resources: &resources().unwrap(),
            assets: &[],
        },
        model::limits(),
    )
    .unwrap();
    assert_eq!(
        stage_with_supplier(&stale, &input, &mut |_| panic!(
            "source changed before sampling"
        )),
        Err(RepositoryError::InvalidCandidate)
    );
    assert_eq!(
        objective_outcome::stage(&stale, pending, command(&input).unwrap()),
        Err(RepositoryError::InvalidCandidate)
    );
    let mut state = after.state().clone();
    let previous = state.facts.last().unwrap();
    let extra = GameFact {
        id: FactId::from_bytes(&[99; 16]).unwrap(),
        revision: previous.revision,
        operation: previous.operation,
        ordinal: previous.ordinal + 1,
        cause: Some(previous.id),
        audience: AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: model::content("combat-victory").unwrap(),
            subjects: vec![entity(ENTITIES[0]).unwrap()],
        },
    };
    state.decisions.last_mut().unwrap().facts.push(extra.id);
    state.facts.push(extra);
    refuses_without_sampling(&model::checkpoint(after.basis(), state).unwrap());
}

const BATTLEFIELD_DECISION_FIXTURE: &str = include_str!("battlefield_objective_contract.json");

fn battlefield_refuses_without_sampling(current: &Checkpoint, input: &GameInput) {
    let original = current.clone();
    let calls = std::cell::Cell::new(0usize);
    assert_eq!(
        stage_with_supplier(current, input, &mut |_| {
            calls.set(calls.get() + 1);
            Ok(20)
        }),
        Err(RepositoryError::InvalidCandidate)
    );
    assert_eq!(calls.get(), 0);
    assert_eq!(current, &original);
}

#[test]
fn battlefield_registered_defend_is_the_exact_authored_escalation() {
    let opening = tests::opening_story();
    let unoffered = tests::input(&opening, first(), 6, "defend-courier", vec![]);
    battlefield_refuses_without_sampling(&opening, &unoffered);

    let dialogue = stage_with_supplier(
        &opening,
        &tests::input(&opening, first(), 6, "escort-courier", vec![]),
        &mut |_| panic!("escort has no rules draws"),
    )
    .unwrap();
    assert!(dialogue.state().encounters.is_empty());
    let input = tests::input(&dialogue, first(), 7, "defend-courier", vec![]);
    let mut initiative = 0;
    let combat = stage_with_supplier(&dialogue, &input, &mut |sides| {
        assert_eq!(sides, 20);
        initiative += 1;
        Ok(if initiative == 1 { 20 } else { 1 })
    })
    .unwrap();

    assert_eq!(initiative, 3);
    let [encounter] = combat.state().encounters.as_slice() else {
        panic!("the authored escalation creates one encounter")
    };
    assert_eq!(
        encounter.participants,
        [
            entity(ENTITIES[0]).unwrap(),
            entity(ENTITIES[1]).unwrap(),
            entity(BANDIT).unwrap()
        ]
    );
    assert_eq!(
        encounter.objectives,
        [model::content("defend-courier").unwrap()]
    );
    assert_eq!(
        encounter.combat_policy,
        model::content("normal-nonlethal-melee").unwrap()
    );
    assert_eq!(
        encounter.active_turn,
        Some(player_entity(first(), &combat).unwrap())
    );
    let bandits = combat
        .state()
        .entities
        .iter()
        .filter(|record| record.definition == model::content("bandit").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(bandits.len(), 1);
    assert_eq!(bandits[0].location, Some(room_entity().unwrap()));
    assert_eq!(bandits[0].position, Some(Position { x: 0, y: 0, z: 0 }));
    for previous in &dialogue.state().entities {
        assert_eq!(
            combat
                .state()
                .entities
                .iter()
                .find(|record| record.id == previous.id),
            Some(previous)
        );
    }
    let operation = command(&input).unwrap().operation;
    let draws = combat
        .state()
        .draws
        .iter()
        .filter(|draw| draw.operation == operation)
        .collect::<Vec<_>>();
    assert_eq!(draws.len(), 3);
    assert!(draws.iter().all(|draw| draw.source == rule().unwrap()));
    let repeated = tests::input(&combat, first(), 8, "defend-courier", vec![]);
    battlefield_refuses_without_sampling(&combat, &repeated);
}

#[test]
fn battlefield_objective_waits_for_the_actual_rules_terminal_cause() {
    let before = before_outcome(true);
    let input = outcome_input(&before, true);
    let after = stage_with_supplier(&before, &input, &mut |sides| {
        Ok(if sides == 20 { 10 } else { 1 })
    })
    .unwrap();

    assert_eq!(phase(&after), Ok(rpc::JourneyPhase::Combat));
    assert_eq!(after.state().entities, before.state().entities);
    assert_eq!(
        after.state().encounters[0].participants,
        before.state().encounters[0].participants
    );
    assert_eq!(
        after.state().encounters[0].combat_policy,
        before.state().encounters[0].combat_policy
    );
    assert_eq!(
        after.state().encounters[0].objectives,
        [model::content("defend-courier").unwrap()]
    );
    assert!(after.state().facts.iter().all(|fact| !matches!(
        &fact.value,
        FactValue::ContentEvent { definition, .. }
            if ["combat-victory", "combat-defeat"].contains(&definition.entry.as_str())
    )));
    assert_eq!(
        &after.state().facts[..before.state().facts.len()],
        before.state().facts.as_slice()
    );
}

#[test]
fn battlefield_terminal_objective_changes_only_the_admitted_slot() {
    for victory in [true, false] {
        let (before, input, after) = completed(victory);
        assert_terminal(&before, &input, &after, victory);
        let proposal_input = before_proposal(&after, victory);
        let original = proposal_input.clone();
        let admitted =
            objective_outcome::stage(&before, proposal_input.clone(), command(&input).unwrap())
                .unwrap();
        let mut expected = original.state().clone();
        expected.encounters[0].objectives = vec![
            model::content(if victory {
                "combat-victory"
            } else {
                "combat-defeat"
            })
            .unwrap(),
        ];

        assert_eq!(admitted.state(), &expected);
        assert_eq!(admitted.basis(), original.basis());
        assert_eq!(admitted.pins(), original.pins());
        assert_eq!(admitted.state().entities, original.state().entities);
        assert_eq!(admitted.state().schedules, original.state().schedules);
        assert_eq!(admitted.state().logical_time, original.state().logical_time);
        assert_eq!(proposal_input, original);
    }
}

#[test]
fn battlefield_complete_pin_mismatches_refuse_before_sampling() {
    use df_types::BuildIdentity;

    let before = before_outcome(true);
    for case in 0..6 {
        let mut pins = before.pins().clone();
        match case {
            0 => pins.rules.catalog_digest = ContentDigest([91; 32]),
            1 => pins.rules.source_manifest_digest = ContentDigest([92; 32]),
            2 => pins.rules.handler_digest = ContentDigest([93; 32]),
            3 => pins.content.content_digest = ContentDigest([94; 32]),
            4 => pins.content.package_digest = ContentDigest([95; 32]),
            _ => {
                pins.build = BuildIdentity::new(
                    Some("unadmitted-source"),
                    Some("unadmitted-native"),
                    Some("unadmitted-wasm"),
                    Some("unadmitted-config"),
                    Some("unadmitted-content"),
                )
                .unwrap();
            }
        }
        let changed = Checkpoint::new(
            before.schema(),
            before.basis(),
            pins,
            before.state().clone(),
            ReferenceInventory {
                rules: &[model::rule().unwrap(), rule().unwrap()],
                content: &model::contents().unwrap(),
                resources: &resources().unwrap(),
                assets: &[],
            },
            model::limits(),
        )
        .unwrap();
        let input = outcome_input(&changed, true);
        battlefield_refuses_without_sampling(&changed, &input);
    }
}

#[test]
fn battlefield_incompatible_canonical_geometry_removes_attack_admission() {
    let before = before_outcome(true);
    let actor = player_entity(first(), &before).unwrap();
    for case in 0..4 {
        let mut state = before.state().clone();
        let id = if case % 2 == 0 {
            actor
        } else {
            entity(BANDIT).unwrap()
        };
        let record = state
            .entities
            .iter_mut()
            .find(|record| record.id == id)
            .unwrap();
        if case < 2 {
            record.position = Some(Position { x: 0, y: 10, z: 0 });
        } else {
            record.location = None;
        }
        let changed = model::checkpoint(before.basis(), state).unwrap();
        assert_eq!(combat_transition::available(&changed, actor), Ok(false));
        let input = outcome_input(&changed, true);
        battlefield_refuses_without_sampling(&changed, &input);
    }
}

#[test]
fn battlefield_unmapped_escalation_cannot_add_enemies_or_transform_terrain() {
    let before = before_outcome(true);
    for entry in [
        "unsupported-reinforcement-request",
        "unsupported-terrain-request",
    ] {
        let reference = model::content(entry).unwrap();
        assert!(!model::contents().unwrap().contains(&reference));
        let input = tests::input(&before, first(), 8, entry, vec![]);
        battlefield_refuses_without_sampling(&before, &input);
    }
    assert_eq!(before.state().encounters.len(), 1);
    assert_eq!(
        before.state().encounters[0].participants,
        [
            entity(ENTITIES[0]).unwrap(),
            entity(ENTITIES[1]).unwrap(),
            entity(BANDIT).unwrap()
        ]
    );
    assert_eq!(
        before.state().encounters[0].objectives,
        [model::content("defend-courier").unwrap()]
    );
}

#[test]
fn battlefield_catalog_presence_and_policy_labels_do_not_authorize_outcomes() {
    let before = before_outcome(true);
    for change_objective in [true, false] {
        let mut state = before.state().clone();
        let present_but_unmapped = model::content("short-rest").unwrap();
        assert!(model::contents().unwrap().contains(&present_but_unmapped));
        if change_objective {
            state.encounters[0].objectives = vec![present_but_unmapped];
        } else {
            state.encounters[0].combat_policy = present_but_unmapped;
        }
        let changed = model::checkpoint(before.basis(), state).unwrap();
        let input = outcome_input(&changed, true);
        battlefield_refuses_without_sampling(&changed, &input);
    }
}

#[test]
fn battlefield_executed_native_witness_binds_the_decision_fixture() {
    use sha2::{Digest, Sha256};

    let (before, input, after) = completed(true);
    assert_terminal(&before, &input, &after, true);
    assert_eq!(before.pins(), &model::pins().unwrap());
    assert_eq!(after.pins(), before.pins());
    assert!(!BATTLEFIELD_DECISION_FIXTURE.is_empty());
    println!(
        "COMBAT_D03_NATIVE_WITNESS fixture_sha256={:x} source_manifest_sha256={:x} pins={:?}",
        Sha256::digest(BATTLEFIELD_DECISION_FIXTURE.as_bytes()),
        Sha256::digest(model::source_manifest()),
        after.pins()
    );
}
