use super::*;
use df_observe::OperationContext;
use df_protocol::common as rpc;
use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
use df_session::submission::*;
use df_types::{OperationId, SessionId};
use std::cell::RefCell;
use std::rc::Rc;

// Controlled transaction/publication ports drive the real DurableOwner and registered
// native journey. They model acknowledgement states, not physical PostgreSQL authority.
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
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
            .is_none_or(|bytes| bytes > maximum)
        {
            return Err(RepositoryError::Capacity);
        }
        Ok(self.clone())
    }
    fn session(&self) -> SessionId {
        let GameInput::Game(command) = &self.input else {
            panic!("command")
        };
        command.basis.session
    }
    fn operation(&self) -> OperationId {
        let GameInput::Game(command) = &self.input else {
            panic!("command")
        };
        command.operation
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
            return Err(RepositoryError::Unavailable);
        }
        Ok(db.checkpoint.clone())
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
        stage_with_supplier(current, input, &mut |sides| {
            assert_eq!(sides, 20);
            self.database.borrow_mut().samples += 1;
            Ok(10)
        })
    }
    fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
        super::super::super::phase(checkpoint)?;
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
        assert_eq!(reaction_facts(checkpoint).len(), 1);
        assert_eq!(
            relation(checkpoint, entity(ENTITIES[0]).unwrap())
                .state
                .as_str(),
            ESCORT_SUPPORTED
        );
        assert_eq!(
            relation(checkpoint, entity(ENTITIES[1]).unwrap())
                .state
                .as_str(),
            UNFAMILIAR
        );
        db.publications += 1;
        Ok(())
    }
    fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
        Ok(())
    }
}
type Owner = DurableOwner<Repository, Engine, Publication>;
fn setup(failure: Failure) -> (Owner, Rc<RefCell<Database>>, Scope) {
    let current = escorted();
    let scope = Scope {
        input: defend_input(&current, 0, 7),
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
            build: "courier-relationship-consumer".to_owned(),
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
fn assert_exact_committed_change(before: &Checkpoint, after: &Checkpoint) {
    assert_eq!(reaction_facts(after).len(), 1);
    assert_eq!(
        &after.state().facts[..before.state().facts.len()],
        before.state().facts
    );
    assert_eq!(
        &after.state().decisions[..before.state().decisions.len()],
        before.state().decisions
    );
    assert_eq!(
        relation(after, entity(ENTITIES[0]).unwrap()).state.as_str(),
        ESCORT_SUPPORTED
    );
    assert_eq!(
        relation(after, entity(ENTITIES[1]).unwrap()),
        relation(before, entity(ENTITIES[1]).unwrap())
    );
    assert_eq!(after.state().continuity, before.state().continuity);
    assert_eq!(after.state().knowledge, before.state().knowledge);
    assert_eq!(after.state().draws.len(), before.state().draws.len() + 3);
}

#[test]
fn courier_relationship_session_ack_exact_retry_and_cancelled_receipt_apply_one_consequence() {
    for deliver in [false, true] {
        let (mut owner, database, scope) = setup(Failure::None);
        let original = owner.checkpoint().clone();
        let first = submit(&mut owner, &scope, deliver);
        let retry = submit(&mut owner, &scope, true).unwrap();
        assert!(matches!(retry, SubmissionOutcome::Confirmed(_)));
        if deliver {
            assert_eq!(Some(retry), first);
        }
        let db = database.borrow();
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_exact_committed_change(&original, &db.checkpoint);
        assert_eq!(
            (
                db.decisions,
                db.commits,
                db.publications,
                db.samples,
                db.ledger.len()
            ),
            (1, 1, 1, 3, 1)
        );
        let mut conflict = scope.clone();
        let GameInput::Game(command) = &mut conflict.input else {
            panic!("command")
        };
        command.observed_revision = command.observed_revision.next_sequence().unwrap();
        drop(db);
        assert_eq!(
            submit(&mut owner, &conflict, true),
            Some(SubmissionOutcome::OperationConflict)
        );
        assert_eq!(database.borrow().decisions, 1);
    }
}

#[test]
fn courier_relationship_session_commit_rollback_retains_canonical_state_and_safe_retry_applies_once()
 {
    let (mut owner, database, scope) = setup(Failure::BeforeCommit);
    let original = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Refused(RepositoryError::Unavailable))
    );
    assert_eq!(owner.checkpoint(), &original);
    assert_eq!(database.borrow().checkpoint, original);
    assert_eq!(database.borrow().publications, 0);
    assert!(reaction_facts(owner.checkpoint()).is_empty());
    database.borrow_mut().failure = Failure::None;
    assert!(matches!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Confirmed(_))
    ));
    let db = database.borrow();
    assert_eq!(owner.checkpoint(), &db.checkpoint);
    assert_exact_committed_change(&original, &db.checkpoint);
    assert_eq!(
        (
            db.decisions,
            db.commits,
            db.publications,
            db.samples,
            db.ledger.len()
        ),
        (2, 2, 1, 6, 1)
    );
}

#[test]
fn courier_relationship_session_lost_ack_fences_other_input_until_exact_receipt_and_validated_reload()
 {
    let (mut owner, database, scope) = setup(Failure::LostAck);
    let original = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    assert!(owner.has_uncertain_operation());
    assert_eq!(owner.checkpoint(), &original);
    assert_exact_committed_change(&original, &database.borrow().checkpoint);
    let mut other = scope.clone();
    let GameInput::Game(command) = &mut other.input else {
        panic!("command")
    };
    command.operation = OperationId::from_bytes(&[8; 16]).unwrap();
    assert_eq!(
        submit(&mut owner, &other, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    database.borrow_mut().refuse_reload = true;
    let receipt = submit(&mut owner, &scope, true).unwrap();
    assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
    assert!(owner.has_uncertain_operation());
    assert_eq!(owner.checkpoint(), &original);
    database.borrow_mut().refuse_reload = false;
    assert_eq!(submit(&mut owner, &scope, true), Some(receipt));
    assert!(owner.is_current());
    let db = database.borrow();
    assert_eq!(owner.checkpoint(), &db.checkpoint);
    assert_exact_committed_change(&original, &db.checkpoint);
    assert_eq!(
        (
            db.decisions,
            db.commits,
            db.publications,
            db.samples,
            db.ledger.len()
        ),
        (1, 1, 0, 3, 1)
    );
}

#[test]
fn courier_relationship_session_unknown_not_recorded_never_reexecutes_or_publishes_reaction() {
    let (mut owner, database, scope) = setup(Failure::UnknownNotCommitted);
    let original = owner.checkpoint().clone();
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
    assert_eq!(owner.checkpoint(), &original);
    assert_eq!(db.checkpoint, original);
    assert!(reaction_facts(&db.checkpoint).is_empty());
    assert_eq!(
        (
            db.decisions,
            db.commits,
            db.publications,
            db.samples,
            db.ledger.len()
        ),
        (1, 1, 0, 3, 0)
    );
}

fn trigger_fact(checkpoint: &Checkpoint) -> &GameFact {
    let facts = checkpoint
        .state()
        .facts
        .iter()
        .filter(|fact| {
            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "defend-courier")
        })
        .collect::<Vec<_>>();
    let [fact] = facts.as_slice() else {
        panic!("one committed authored Defend source")
    };
    fact
}

fn assert_trigger_occurrence(
    before: &Checkpoint,
    after: &Checkpoint,
    scope: &Scope,
    receipt: &DecisionReceipt,
) {
    let fact = trigger_fact(after);
    let state = after.state();
    let decision = receipt.decision();
    assert_eq!(receipt.basis(), after.basis());
    assert_eq!(
        receipt.basis().revision,
        before.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(decision.operation, scope.operation());
    assert_eq!(decision.revision, fact.revision);
    assert_eq!(decision.operation, fact.operation);
    assert_eq!(decision.source_policy.as_str(), THREAD_POLICY);
    assert_eq!(decision.facts.last(), Some(&fact.id));
    assert_eq!(decision.facts.get(fact.ordinal as usize), Some(&fact.id));
    assert_eq!(decision.draws, [0, 1, 2]);
    assert!(decision.effects.is_empty());
    let owners = state
        .decisions
        .iter()
        .filter(|record| record.operation == fact.operation)
        .collect::<Vec<_>>();
    assert_eq!(owners, [decision]);
    let index = state
        .facts
        .iter()
        .position(|record| record.id == fact.id)
        .unwrap();
    let preceding = &state.facts[index - 1];
    assert_eq!(fact.cause, Some(preceding.id));
    assert_eq!(preceding.operation, fact.operation);
    assert_eq!(preceding.revision, fact.revision);
    assert_eq!(preceding.ordinal + 1, fact.ordinal);
    assert_eq!(fact.audience, AudienceScope::Shared);
    assert!(
        matches!(&fact.value, FactValue::ContentEvent { definition, subjects }
        if *definition == model::content("defend-courier").unwrap() && subjects.is_empty())
    );
    assert_eq!(
        &state.narrative.accepted_facts[..before.state().narrative.accepted_facts.len()],
        before.state().narrative.accepted_facts
    );
    assert_eq!(
        &state.narrative.accepted_facts[before.state().narrative.accepted_facts.len()..],
        [fact.id]
    );
    let [encounter] = state.encounters.as_slice() else {
        panic!("one committed encounter")
    };
    assert_eq!(encounter.id.as_bytes(), &super::super::super::ENCOUNTER);
    assert_eq!(encounter.definition, model::content("combat").unwrap());
    assert_eq!(
        encounter.objectives,
        [model::content("defend-courier").unwrap()]
    );
    assert_eq!(
        encounter.combat_policy,
        model::content("normal-nonlethal-melee").unwrap()
    );
    assert_eq!(
        state
            .entities
            .iter()
            .filter(|record| record.id.as_bytes() == &super::super::super::BANDIT)
            .count(),
        1
    );
    assert_eq!(
        super::super::super::phase(after).unwrap(),
        rpc::JourneyPhase::Combat
    );
    assert_exact_committed_change(before, after);
}

fn database_activity_counts(
    database: &Rc<RefCell<Database>>,
) -> (usize, usize, usize, usize, usize) {
    let db = database.borrow();
    (
        db.decisions,
        db.commits,
        db.publications,
        db.samples,
        db.ledger.len(),
    )
}

fn confirmed_receipt(outcome: SubmissionOutcome) -> DecisionReceipt {
    let SubmissionOutcome::Confirmed(receipt) = outcome else {
        panic!("actual durable acknowledgement required")
    };
    receipt
}

fn owner_at(
    database: &Rc<RefCell<Database>>,
    checkpoint: Checkpoint,
) -> Result<Owner, RepositoryError> {
    DurableOwner::new(
        Repository {
            database: Rc::clone(database),
        },
        Engine {
            database: Rc::clone(database),
        },
        Publication {
            database: Rc::clone(database),
        },
        checkpoint,
        64 * 1024,
    )
}

#[test]
fn encounter_trigger_occurrence_commits_one_canonical_source() {
    let (mut owner, database, scope) = setup(Failure::None);
    let before = owner.checkpoint().clone();
    assert!(before.state().encounters.is_empty());
    let receipt = confirmed_receipt(submit(&mut owner, &scope, true).unwrap());
    let db = database.borrow();
    assert_eq!(owner.checkpoint(), &db.checkpoint);
    assert!(db.ledger[0].0 == scope);
    assert_eq!(db.ledger[0].1, receipt);
    assert_trigger_occurrence(&before, &db.checkpoint, &scope, &receipt);
    drop(db);
    assert_eq!(database_activity_counts(&database), (1, 1, 1, 3, 1));
}

#[test]
fn encounter_trigger_occurrence_exact_old_basis_retry_is_receipt_only() {
    for deliver in [false, true] {
        let (mut owner, database, scope) = setup(Failure::None);
        let before = owner.checkpoint().clone();
        let first = submit(&mut owner, &scope, deliver);
        let committed = database.borrow().checkpoint.clone();
        let retained = database.borrow().ledger[0].1.clone();
        if deliver {
            assert_eq!(first, Some(SubmissionOutcome::Confirmed(retained.clone())));
        }
        assert!(before.basis().revision < committed.basis().revision);
        // Keep the original input bytes and older observed basis. Receipt lookup
        // precedes the current offer/phase check and every supplier call.
        for _ in 0..2 {
            assert_eq!(
                submit(&mut owner, &scope, true),
                Some(SubmissionOutcome::Confirmed(retained.clone()))
            );
        }
        assert_eq!(owner.checkpoint(), &committed);
        assert_eq!(database.borrow().checkpoint, committed);
        assert_trigger_occurrence(&before, &committed, &scope, &retained);
        assert_eq!(database_activity_counts(&database), (1, 1, 1, 3, 1));
    }
}

#[test]
fn encounter_trigger_occurrence_fresh_operation_is_refused() {
    let (mut owner, database, scope) = setup(Failure::None);
    let before = owner.checkpoint().clone();
    let receipt = confirmed_receipt(submit(&mut owner, &scope, true).unwrap());
    let committed = owner.checkpoint().clone();
    let fresh = Scope {
        input: defend_input(&committed, 0, 8),
    };
    assert_eq!(
        submit(&mut owner, &fresh, true),
        Some(SubmissionOutcome::Refused(
            RepositoryError::InvalidCandidate
        ))
    );
    assert_eq!(owner.checkpoint(), &committed);
    assert_eq!(database.borrow().checkpoint, committed);
    assert_trigger_occurrence(&before, &committed, &scope, &receipt);
    assert_eq!(database_activity_counts(&database), (2, 1, 1, 3, 1));
    // Direct stale input is illegal at the source caller even though the Session
    // can truthfully return the original committed receipt for the exact key.
    assert_eq!(
        stage_with_supplier(&committed, &scope.input, &mut |_| {
            panic!("stale source retry cannot sample")
        }),
        Err(RepositoryError::InvalidCandidate)
    );
}

#[test]
fn encounter_trigger_occurrence_lost_ack_reloads_same_receipt() {
    let (mut owner, database, scope) = setup(Failure::LostAck);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    let committed = database.borrow().checkpoint.clone();
    let receipt = database.borrow().ledger[0].1.clone();
    assert_trigger_occurrence(&before, &committed, &scope, &receipt);
    assert_eq!(owner.checkpoint(), &before);
    let other = Scope {
        input: defend_input(&before, 0, 8),
    };
    assert_eq!(
        submit(&mut owner, &other, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    // A truthful receipt alone does not make a failed restored cache current.
    let mut damaged = committed.state().clone();
    damaged
        .narrative
        .accepted_facts
        .retain(|id| *id != trigger_fact(&committed).id);
    database.borrow_mut().checkpoint = model::checkpoint(committed.basis(), damaged).unwrap();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Confirmed(receipt.clone()))
    );
    assert!(owner.has_uncertain_operation());
    assert_eq!(owner.checkpoint(), &before);
    database.borrow_mut().checkpoint = committed.clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Confirmed(receipt.clone()))
    );
    assert!(owner.is_current());
    assert_eq!(owner.checkpoint(), &committed);
    assert_eq!(database_activity_counts(&database), (1, 1, 0, 3, 1));
}

#[test]
fn encounter_trigger_occurrence_known_rollback_allows_one_commit() {
    let (mut owner, database, scope) = setup(Failure::BeforeCommit);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Refused(RepositoryError::Unavailable))
    );
    assert_eq!(owner.checkpoint(), &before);
    assert_eq!(database.borrow().checkpoint, before);
    assert_eq!(database_activity_counts(&database), (1, 1, 0, 3, 0));
    database.borrow_mut().failure = Failure::None;
    let receipt = confirmed_receipt(submit(&mut owner, &scope, true).unwrap());
    assert_trigger_occurrence(&before, owner.checkpoint(), &scope, &receipt);
    let committed = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::Confirmed(receipt))
    );
    assert_eq!(owner.checkpoint(), &committed);
    // Two known transaction attempts/samplings, exactly one retained commit.
    assert_eq!(database_activity_counts(&database), (2, 2, 1, 6, 1));
}

#[test]
fn encounter_trigger_occurrence_unknown_not_recorded_never_reexecutes() {
    let (mut owner, database, scope) = setup(Failure::UnknownNotCommitted);
    let before = owner.checkpoint().clone();
    assert_eq!(
        submit(&mut owner, &scope, true),
        Some(SubmissionOutcome::LookupRequired)
    );
    database.borrow_mut().failure = Failure::None;
    for _ in 0..2 {
        assert_eq!(
            submit(&mut owner, &scope, true),
            Some(SubmissionOutcome::LookupRequired)
        );
    }
    assert!(owner.has_uncertain_operation());
    assert_eq!(owner.checkpoint(), &before);
    assert_eq!(database.borrow().checkpoint, before);
    assert!(before.state().encounters.is_empty());
    assert!(!before.state().facts.iter().any(
        |fact| matches!(&fact.value, FactValue::ContentEvent { definition, .. }
            if definition.entry.as_str() == "defend-courier")
    ));
    assert_eq!(database_activity_counts(&database), (1, 1, 0, 3, 0));
}

#[test]
fn encounter_trigger_occurrence_canonical_restore_and_source_refusals() {
    let (mut owner, database, scope) = setup(Failure::None);
    let before = owner.checkpoint().clone();
    let receipt = confirmed_receipt(submit(&mut owner, &scope, true).unwrap());
    let committed = owner.checkpoint().clone();
    drop(owner);
    let restored = model::checkpoint(committed.basis(), committed.state().clone()).unwrap();
    let mut recovered = owner_at(&database, restored).unwrap();
    assert_eq!(
        submit(&mut recovered, &scope, true),
        Some(SubmissionOutcome::Confirmed(receipt.clone()))
    );
    assert_trigger_occurrence(&before, recovered.checkpoint(), &scope, &receipt);
    assert_eq!(database_activity_counts(&database), (1, 1, 1, 3, 1));
    for damage in 0..3 {
        let mut state = committed.state().clone();
        match damage {
            0 => state
                .narrative
                .accepted_facts
                .retain(|id| *id != trigger_fact(&committed).id),
            1 => state.encounters.clear(),
            2 => {
                state.narrative.active_beats =
                    vec![model::content("courier-answer-escort").unwrap()]
            }
            _ => unreachable!(),
        }
        // Structural model admission is established separately from native recovery.
        let restored = model::checkpoint(committed.basis(), state).unwrap();
        assert!(matches!(
            owner_at(&database, restored.clone()),
            Err(RepositoryError::InvalidCandidate)
        ));
        let command = defend_input(&restored, 0, 8);
        assert_eq!(
            stage_with_supplier(&restored, &command, &mut |_| {
                panic!("damaged restoration cannot sample")
            }),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(database.borrow().checkpoint, committed);
        assert_eq!(database_activity_counts(&database), (1, 1, 1, 3, 1));
    }
    // The actual prerequisite event cannot be an orphan, wrong-policy, wrong-
    // revision, incorrectly caused or unconsumed source occurrence.
    let source_operation = before.state().decisions.last().unwrap().operation;
    for damage in 0..5 {
        let mut state = before.state().clone();
        let source = state
            .facts
            .iter()
            .find(|fact| {
                matches!(&fact.value, FactValue::ContentEvent { definition, .. }
                if definition.entry.as_str() == "escort-courier")
            })
            .unwrap()
            .id;
        match damage {
            0 => state
                .decisions
                .retain(|decision| decision.operation != source_operation),
            1 => {
                state.decisions.last_mut().unwrap().source_policy =
                    model::label("wrong-source-policy").unwrap()
            }
            2 => {
                let earlier = state
                    .decisions
                    .iter()
                    .find(|decision| {
                        decision.operation != source_operation
                            && decision.facts.iter().any(|id| {
                                state.facts.iter().any(|fact| fact.id == *id &&
                            matches!(&fact.value, FactValue::ContentEvent { definition, .. }
                                if definition.entry.as_str() == "begin-story"))
                            })
                    })
                    .unwrap()
                    .revision;
                state.decisions.last_mut().unwrap().revision = earlier;
                for fact in state
                    .facts
                    .iter_mut()
                    .filter(|fact| fact.operation == source_operation)
                {
                    fact.revision = earlier;
                }
            }
            3 => {
                state
                    .facts
                    .iter_mut()
                    .find(|fact| fact.id == source)
                    .unwrap()
                    .cause = None
            }
            4 => state.narrative.accepted_facts.retain(|id| *id != source),
            _ => unreachable!(),
        }
        let restored = model::checkpoint(before.basis(), state).unwrap();
        assert!(matches!(
            owner_at(&database, restored.clone()),
            Err(RepositoryError::InvalidCandidate)
        ));
        assert_eq!(
            stage_with_supplier(&restored, &defend_input(&restored, 0, 8), &mut |_| {
                panic!("unowned or unconsumed source cannot sample")
            }),
            Err(RepositoryError::InvalidCandidate)
        );
        assert_eq!(database_activity_counts(&database), (1, 1, 1, 3, 1));
    }
}

#[test]
fn encounter_trigger_occurrence_changed_payload_conflicts() {
    let (mut owner, database, scope) = setup(Failure::None);
    let before = owner.checkpoint().clone();
    let mut wrong_actor = scope.clone();
    let GameInput::Game(command) = &mut wrong_actor.input else {
        panic!("command")
    };
    let GameCommand::ProposeAction { actor, .. } = &mut command.command else {
        panic!("action")
    };
    *actor = entity(ENTITIES[1]).unwrap();
    assert_eq!(
        submit(&mut owner, &wrong_actor, true),
        Some(SubmissionOutcome::Refused(RepositoryError::Unauthorized))
    );
    assert_eq!(database_activity_counts(&database), (1, 0, 0, 0, 0));
    assert_eq!(owner.checkpoint(), &before);
    let receipt = confirmed_receipt(submit(&mut owner, &scope, true).unwrap());
    let committed = owner.checkpoint().clone();
    let mut changed = scope.clone();
    let GameInput::Game(command) = &mut changed.input else {
        panic!("command")
    };
    command.observed_revision = committed.basis().revision;
    assert_eq!(
        submit(&mut owner, &changed, true),
        Some(SubmissionOutcome::OperationConflict)
    );
    assert_eq!(
        submit(&mut owner, &wrong_actor, true),
        Some(SubmissionOutcome::OperationConflict)
    );
    assert_eq!(owner.checkpoint(), &committed);
    assert_trigger_occurrence(&before, &committed, &scope, &receipt);
    assert_eq!(database_activity_counts(&database), (2, 1, 1, 3, 1));
}
