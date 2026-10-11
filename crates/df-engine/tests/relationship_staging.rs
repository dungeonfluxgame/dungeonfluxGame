use std::cell::Cell;

#[path = "support/fixture_model.rs"]
pub mod fixture_model;
#[path = "support/relationship_staging.rs"]
pub mod support;

use df_engine::obligation_fulfillment::FulfillmentError;
use df_engine::relationship_staging::*;
use df_interaction::reactions::*;
use df_interaction::relationships::{RelationshipAxis, RelationshipAxisValue, RelationshipOwner};
use df_model::checkpoint::*;
use df_rules::{RulesCommandHandler, RulesCommandInput, stage_handler};
use df_types::RevisionLabel;
use fixture_model as model;
use support::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AxisOwnerRefusal {
    UnacceptedSource,
}

#[derive(Default)]
struct AxisOwner {
    admissions: Cell<usize>,
    validations: Cell<usize>,
}

impl RelationshipOwner for AxisOwner {
    type Subject = EntityId;
    type Basis = Basis;
    type Value = RevisionLabel;
    type Provenance = RelationshipAxisProvenance;
    type Refusal = AxisOwnerRefusal;

    fn validate_axis_change(
        &self,
        _: RelationshipAxis,
        _: &RelationshipAxisValue<Self::Value, Self::Provenance>,
        _: &RelationshipAxisValue<Self::Value, Self::Provenance>,
    ) -> Result<(), Self::Refusal> {
        self.validations.set(self.validations.get() + 1);
        Ok(())
    }
}

impl RelationshipAxisOwner for AxisOwner {
    fn admit_axis_change(
        &self,
        candidate: &Checkpoint,
        change: &RelationshipAxisChange,
    ) -> Result<(), Self::Refusal> {
        self.admissions.set(self.admissions.get() + 1);
        let RelationshipAxisProvenance::AcceptedFact {
            source,
            fact,
            source_policy,
            witness,
        } = &change.replacement.provenance
        else {
            return Err(AxisOwnerRefusal::UnacceptedSource);
        };
        let Some(record) = candidate
            .state()
            .facts
            .iter()
            .find(|record| record.id == *fact)
        else {
            return Err(AxisOwnerRefusal::UnacceptedSource);
        };
        let admitted = candidate.state().decisions.iter().any(|decision| {
            decision.operation == record.operation
                && decision.revision == record.revision
                && decision.facts.contains(fact)
                && decision.source_policy == *source_policy
                && decision.source_policy == model::label("original-delivery-policy")
        });
        let subjects_match = matches!(
            &record.value,
            FactValue::ContentEvent { definition, subjects }
                if definition == &content("accepted-delivery")
                    && subjects.as_slice() == [change.subject, change.target]
        );
        if admitted
            && subjects_match
            && source == &content("directional-policy")
            && witness.is_none()
        {
            Ok(())
        } else {
            Err(AxisOwnerRefusal::UnacceptedSource)
        }
    }
}

fn axis_staging_limits() -> RelationshipAxisStagingLimits {
    RelationshipAxisStagingLimits {
        maximum_changes: 16,
        maximum_inventory_records: 64,
        maximum_checkpoint_bytes: 1024 * 1024,
        maximum_pass_bytes: 8 * 1024 * 1024,
        checkpoint: model::limits(),
    }
}

fn delivery_axis_change() -> RelationshipAxisChange {
    RelationshipAxisChange {
        subject: model::entity(4),
        target: model::entity(5),
        axis: RelationshipAxis::Affection,
        replacement: RelationshipAxisState {
            value: model::label("affection-after-delivery"),
            provenance: RelationshipAxisProvenance::AcceptedFact {
                source: content("directional-policy"),
                fact: fact(30),
                source_policy: model::label("original-delivery-policy"),
                witness: None,
            },
        },
    }
}

fn axis_candidate(fixture: &Fixture) -> (Checkpoint, Basis) {
    let current = fixture.current();
    let mut expected = current.basis();
    expected.revision = expected.revision.next_sequence().unwrap();
    let owner = SourceOwner::new(fixture);
    let input = fixture.input(&current);
    let staged = stage_handler(
        &fixture.inner(&owner, &current),
        &fixture.pins,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
        &current,
        1024 * 1024,
    )
    .unwrap();
    (staged, expected)
}

#[test]
fn registered_real_completion_applies_only_exact_direction_and_retains_all_mechanical_siblings() {
    let fixture = Fixture::new();
    let owner = SourceOwner::new(&fixture);
    let current = fixture.current();
    let original = current.clone();
    let input = fixture.input(&current);
    let mut base = stage_handler(
        &fixture.inner(&owner, &current),
        &fixture.pins,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
        &current,
        1024 * 1024,
    )
    .unwrap()
    .state()
    .clone();
    let candidate = registered(&fixture, &owner, &current, &input).unwrap();
    base.relationships[0].state = model::label("receptive");
    assert_eq!(candidate.state(), &base);
    assert_eq!(
        candidate.state().relationships[1],
        current.state().relationships[1]
    );
    assert_eq!(
        [
            &candidate.state().relationships[0].trust,
            &candidate.state().relationships[0].affection,
            &candidate.state().relationships[0].respect,
            &candidate.state().relationships[0].fear,
            &candidate.state().relationships[0].suspicion,
            &candidate.state().relationships[0].debt,
            &candidate.state().relationships[0].familiarity,
        ],
        [
            &current.state().relationships[0].trust,
            &current.state().relationships[0].affection,
            &current.state().relationships[0].respect,
            &current.state().relationships[0].fear,
            &current.state().relationships[0].suspicion,
            &current.state().relationships[0].debt,
            &current.state().relationships[0].familiarity,
        ]
    );
    assert_eq!(candidate.pins(), current.pins());
    assert_eq!(
        candidate.basis().revision,
        current.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(candidate.state().facts.last().unwrap().id, fact(50));
    assert_eq!(
        candidate.state().facts.len(),
        current.state().facts.len() + 1
    );
    assert_eq!(current.state().draws.len(), 1);
    assert_eq!(candidate.state().draws, current.state().draws);
    assert_eq!(
        candidate
            .state()
            .decisions
            .iter()
            .find(|decision| decision.operation == operation(30))
            .unwrap()
            .draws,
        [0]
    );
    assert_eq!(
        candidate.state().decisions.last().unwrap().source_policy,
        fixture.registration.policy
    );
    assert_eq!(current, original);
}

#[test]
fn accepted_axis_change_updates_one_directional_axis_and_keeps_all_siblings() {
    let fixture = Fixture::new();
    let source_owner = SourceOwner::new(&fixture);
    let current = fixture.current();
    let mut expected_basis = current.basis();
    expected_basis.revision = expected_basis.revision.next_sequence().unwrap();
    let input = fixture.input(&current);
    let staged = stage_handler(
        &fixture.inner(&source_owner, &current),
        &fixture.pins,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
        &current,
        1024 * 1024,
    )
    .unwrap();
    let original = staged.state().relationships[1].clone();
    let reverse = staged.state().relationships[0].clone();
    let change = RelationshipAxisChange {
        subject: model::entity(4),
        target: model::entity(5),
        axis: RelationshipAxis::Affection,
        replacement: RelationshipAxisState {
            value: model::label("affection-after-delivery"),
            provenance: RelationshipAxisProvenance::AcceptedFact {
                source: content("directional-policy"),
                fact: fact(30),
                source_policy: model::label("original-delivery-policy"),
                witness: None,
            },
        },
    };

    let candidate = apply_relationship_axis_changes(
        &AxisOwner::default(),
        expected_basis,
        &staged,
        &fixture.pins,
        fixture.inventory(),
        &[change],
        axis_staging_limits(),
    )
    .unwrap();
    let replayed = apply_relationship_axis_changes(
        &AxisOwner::default(),
        expected_basis,
        &staged,
        &fixture.pins,
        fixture.inventory(),
        &[RelationshipAxisChange {
            subject: model::entity(4),
            target: model::entity(5),
            axis: RelationshipAxis::Affection,
            replacement: RelationshipAxisState {
                value: model::label("affection-after-delivery"),
                provenance: RelationshipAxisProvenance::AcceptedFact {
                    source: content("directional-policy"),
                    fact: fact(30),
                    source_policy: model::label("original-delivery-policy"),
                    witness: None,
                },
            },
        }],
        axis_staging_limits(),
    )
    .unwrap();

    assert_eq!(staged.state().relationships[1], original);
    assert_eq!(replayed, candidate);
    assert_eq!(candidate.state().relationships[0], reverse);
    let updated = &candidate.state().relationships[1];
    assert_eq!(updated.state, original.state);
    assert_eq!(
        updated.affection.value,
        model::label("affection-after-delivery")
    );
    assert_eq!(
        updated.affection.provenance,
        RelationshipAxisProvenance::AcceptedFact {
            source: content("directional-policy"),
            fact: fact(30),
            source_policy: model::label("original-delivery-policy"),
            witness: None,
        }
    );
    assert_eq!(updated.trust, original.trust);
    assert_eq!(updated.respect, original.respect);
    assert_eq!(updated.fear, original.fear);
    assert_eq!(updated.suspicion, original.suspicion);
    assert_eq!(updated.debt, original.debt);
    assert_eq!(updated.familiarity, original.familiarity);
}

#[test]
fn independent_axis_basis_and_current_capacity_refuse_before_owner_work() {
    let fixture = Fixture::new();
    let (candidate, expected) = axis_candidate(&fixture);
    let original = candidate.clone();
    assert_eq!(expected.revision, model::revision(2, 9));
    let mut session = expected;
    session.session = df_types::SessionId::from_bytes(&[71; 16]).unwrap();
    let mut run = expected;
    run.run = df_types::RunId::from_bytes(&[72; 16]).unwrap();
    let mut epoch = expected;
    epoch.revision = model::revision(3, 9);
    let mut sequence = expected;
    sequence.revision = model::revision(2, 10);
    for (stale, expected_error) in [
        (session, CheckpointError::WrongSession),
        (run, CheckpointError::WrongRun),
        (epoch, CheckpointError::StaleBasis),
        (sequence, CheckpointError::StaleBasis),
    ] {
        let owner = AxisOwner::default();
        assert_eq!(
            apply_relationship_axis_changes(
                &owner,
                stale,
                &candidate,
                &fixture.pins,
                fixture.inventory(),
                &[delivery_axis_change()],
                axis_staging_limits(),
            ),
            Err(RelationshipAxisStagingError::Snapshot(expected_error))
        );
        assert_eq!((owner.admissions.get(), owner.validations.get()), (0, 0));
        assert_eq!(candidate, original);
    }

    let mut changes = axis_staging_limits();
    changes.maximum_changes = 0;
    let mut inventory = axis_staging_limits();
    inventory.maximum_inventory_records = 1;
    let mut pass = axis_staging_limits();
    pass.maximum_pass_bytes = 1;
    let mut records = axis_staging_limits();
    records.checkpoint.maximum_records = 1;
    let mut retained = axis_staging_limits();
    retained.checkpoint.maximum_retained_bytes = 1;
    for (limits, expected_error) in [
        (changes, RelationshipAxisStagingError::Capacity),
        (inventory, RelationshipAxisStagingError::Capacity),
        (pass, RelationshipAxisStagingError::Capacity),
        (
            records,
            RelationshipAxisStagingError::Snapshot(CheckpointError::Capacity),
        ),
        (
            retained,
            RelationshipAxisStagingError::Snapshot(CheckpointError::Capacity),
        ),
    ] {
        let owner = AxisOwner::default();
        assert_eq!(
            apply_relationship_axis_changes(
                &owner,
                expected,
                &candidate,
                &fixture.pins,
                fixture.inventory(),
                &[delivery_axis_change()],
                limits,
            ),
            Err(expected_error)
        );
        assert_eq!((owner.admissions.get(), owner.validations.get()), (0, 0));
        assert_eq!(candidate, original);
    }
}

#[test]
fn malformed_axis_provenance_duplicates_and_late_refusal_discard_the_whole_pass() {
    let fixture = Fixture::new();
    let (candidate, expected) = axis_candidate(&fixture);
    let original = candidate.clone();
    for dimension in ["source", "policy", "fact", "missing-fact", "witness"] {
        let mut change = delivery_axis_change();
        let RelationshipAxisProvenance::AcceptedFact {
            source,
            source_policy,
            fact: cause,
            witness,
        } = &mut change.replacement.provenance
        else {
            unreachable!()
        };
        match dimension {
            "source" => *source = content("authored-reaction"),
            "policy" => *source_policy = model::label("wrong-policy"),
            "fact" => *cause = fact(29),
            "missing-fact" => *cause = fact(99),
            "witness" => *witness = Some(record(31)),
            _ => unreachable!(),
        }
        let owner = AxisOwner::default();
        assert_eq!(
            apply_relationship_axis_changes(
                &owner,
                expected,
                &candidate,
                &fixture.pins,
                fixture.inventory(),
                &[change],
                axis_staging_limits(),
            ),
            Err(RelationshipAxisStagingError::Source(
                AxisOwnerRefusal::UnacceptedSource
            ))
        );
        assert_eq!(owner.admissions.get(), 1);
        assert_eq!(owner.validations.get(), 0);
        assert_eq!(candidate, original);
    }
    let owner = AxisOwner::default();
    assert_eq!(
        apply_relationship_axis_changes(
            &owner,
            expected,
            &candidate,
            &fixture.pins,
            fixture.inventory(),
            &[delivery_axis_change(), delivery_axis_change()],
            axis_staging_limits(),
        ),
        Err(RelationshipAxisStagingError::DuplicateDirection)
    );
    assert_eq!((owner.admissions.get(), owner.validations.get()), (0, 0));
    assert_eq!(candidate, original);

    let mut last = delivery_axis_change();
    last.axis = RelationshipAxis::Trust;
    let RelationshipAxisProvenance::AcceptedFact { source_policy, .. } =
        &mut last.replacement.provenance
    else {
        unreachable!()
    };
    *source_policy = model::label("late-unaccepted-policy");
    let owner = AxisOwner::default();
    assert_eq!(
        apply_relationship_axis_changes(
            &owner,
            expected,
            &candidate,
            &fixture.pins,
            fixture.inventory(),
            &[delivery_axis_change(), last],
            axis_staging_limits(),
        ),
        Err(RelationshipAxisStagingError::Source(
            AxisOwnerRefusal::UnacceptedSource
        ))
    );
    assert_eq!((owner.admissions.get(), owner.validations.get()), (2, 1));
    assert_eq!(candidate, original);
}

#[test]
fn ignorance_beliefs_memory_and_wrong_witness_return_typed_no_reaction_without_social_change() {
    for missing in ["known", "witness", "wrong-observer", "wrong-event"] {
        let fixture = Fixture::new();
        let owner = SourceOwner::new(&fixture);
        let mut state = fixture.state();
        let expected = if missing == "known" {
            state.continuity.npcs[0].known_facts.clear();
            state.beliefs.push(AttributedClaim {
                id: record(70),
                holder: model::entity(5),
                subject: model::entity(4),
                claim: "Private claimed perception".to_owned(),
                evidence: vec![fact(30)],
                audience: AudienceScope::Host,
                source: model::content(),
            });
            state.continuity.npcs[0].beliefs.push(record(70));
            state.memories.push(MemoryEpisode {
                id: record(71),
                holder: model::entity(5),
                source_facts: vec![fact(30)],
                retained_text: "Private memory claiming witness".to_owned(),
                audience: AudienceScope::Host,
                source_revision: model::basis().revision,
            });
            NoReactionReason::NotKnown
        } else {
            match missing {
                "witness" => state.continuity.witnesses.clear(),
                "wrong-observer" => state.continuity.witnesses[0].observer = model::entity(4),
                "wrong-event" => {
                    let mut fact_record = state
                        .facts
                        .iter()
                        .find(|record| record.id == fact(30))
                        .unwrap()
                        .clone();
                    fact_record.id = fact(32);
                    fact_record.ordinal = 2;
                    state.facts.push(fact_record);
                    state
                        .decisions
                        .iter_mut()
                        .find(|decision| decision.operation == operation(30))
                        .unwrap()
                        .facts
                        .push(fact(32));
                    state.continuity.witnesses[0].fact = fact(32);
                }
                _ => unreachable!(),
            }
            NoReactionReason::NotWitnessed
        };
        let current = fixture.checkpoint(state);
        let original = current.clone();
        let result = stage(
            &fixture,
            &owner,
            &current,
            &fixture.input(&current),
            &[fixture.request(&current)],
            limits(),
        )
        .unwrap();
        assert_eq!(result.reactions, [ReactionOutcome::NoReaction(expected)]);
        assert_eq!(
            result.checkpoint.state().relationships,
            current.state().relationships
        );
        assert_eq!(result.checkpoint.state().beliefs, current.state().beliefs);
        assert_eq!(result.checkpoint.state().memories, current.state().memories);
        assert_eq!(
            result.checkpoint.state().continuity,
            current.state().continuity
        );
        assert_eq!(current, original);
    }
}

#[test]
fn forged_actor_and_revoked_native_admission_fail_before_reaction_entry_access() {
    for failure in [
        "actor",
        "member",
        "owner",
        "withdrawn",
        "control",
        "command",
    ] {
        let fixture = Fixture::new();
        let owner = SourceOwner::new(&fixture);
        let mut state = fixture.state();
        if failure == "owner" {
            state.members.push(MembershipLink {
                member: model::member(9),
                character: Some(model::entity(4)),
            });
            state.characters[0].owner = model::member(9);
        }
        let current = fixture.checkpoint(state);
        let mut input = fixture.input(&current);
        let GameInput::Game(command) = &mut input else {
            unreachable!()
        };
        match failure {
            "actor" => {
                let GameCommand::ProposeAction { actor, .. } = &mut command.command else {
                    unreachable!()
                };
                *actor = model::entity(5);
            }
            "member" => command.member = model::member(9),
            "withdrawn" => owner.withdrawn.set(true),
            "control" => owner.controlled.set(false),
            "command" => {
                let GameCommand::ProposeAction { targets, .. } = &mut command.command else {
                    unreachable!()
                };
                targets.clear();
            }
            _ => {}
        }
        let original = current.clone();
        assert!(
            stage(
                &fixture,
                &owner,
                &current,
                &input,
                &[fixture.request(&current)],
                limits()
            )
            .is_err()
        );
        assert_eq!(owner.entry_calls.get(), 0);
        assert_eq!(current, original);
    }
}

#[test]
fn newly_staged_completion_fact_is_never_treated_as_a_committed_perception() {
    let fixture = Fixture::new();
    let owner = SourceOwner::new(&fixture);
    let current = fixture.current();
    let original = current.clone();
    let mut request = fixture.request(&current);
    request.event = fact(50);
    assert!(matches!(
        stage(
            &fixture,
            &owner,
            &current,
            &fixture.input(&current),
            &[request],
            limits()
        ),
        Err(RelationshipStagingError::Reaction(
            ReactionError::UnknownEvent
        ))
    ));
    assert_eq!(current, original);
    assert_eq!(owner.command_calls.get(), 1);
}

#[test]
fn duplicate_or_contradictory_authored_entries_and_duplicate_directions_refuse_whole_pass() {
    for contradiction in [false, true] {
        let mut fixture = Fixture::new();
        let mut entry = fixture.entries[0].clone();
        if contradiction {
            entry.to_state = model::label("suspicious");
        }
        fixture.entries.push(entry);
        let owner = SourceOwner::new(&fixture);
        let current = fixture.current();
        assert!(matches!(
            stage(
                &fixture,
                &owner,
                &current,
                &fixture.input(&current),
                &[fixture.request(&current)],
                limits()
            ),
            Err(RelationshipStagingError::Reaction(
                ReactionError::AmbiguousPolicy
            ))
        ));
        fixture.entries.reverse();
        let owner = SourceOwner::new(&fixture);
        assert!(matches!(
            stage(
                &fixture,
                &owner,
                &current,
                &fixture.input(&current),
                &[fixture.request(&current)],
                limits()
            ),
            Err(RelationshipStagingError::Reaction(
                ReactionError::AmbiguousPolicy
            ))
        ));
    }
    let fixture = Fixture::new();
    let owner = SourceOwner::new(&fixture);
    let current = fixture.current();
    let request = fixture.request(&current);
    assert!(matches!(
        stage(
            &fixture,
            &owner,
            &current,
            &fixture.input(&current),
            &[request, request],
            limits()
        ),
        Err(RelationshipStagingError::DuplicateDirection)
    ));
    let mut state = fixture.state();
    let mut conflicting = state.relationships[0].clone();
    conflicting.state = model::label("conflicting-direction-state");
    state.relationships.push(conflicting);
    let original = state.clone();
    for reverse in [false, true] {
        let mut supplied = state.clone();
        if reverse {
            supplied.relationships.reverse();
        }
        let before = supplied.clone();
        assert_eq!(
            fixture.try_checkpoint(supplied.clone()),
            Err(CheckpointError::DuplicateIdentity)
        );
        assert_eq!(supplied, before);
    }
    assert_eq!(state, original);
}

#[cfg(not(target_arch = "wasm32"))]
mod session {
    use super::*;
    use df_observe::OperationContext;
    use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
    use df_session::submission::*;
    use df_types::{OperationId, SessionId};
    use std::cell::RefCell;
    use std::rc::Rc;

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
        fn capture_uncertainty_key(&self, maximum: usize) -> Result<Scope, RepositoryError> {
            if self
                .input
                .retained_bytes()
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Scope>()))
                .is_none_or(|bytes| bytes > maximum)
            {
                return Err(RepositoryError::Capacity);
            }
            Ok(self.clone())
        }
        fn session(&self) -> SessionId {
            let GameInput::Game(command) = &self.input else {
                unreachable!()
            };
            command.basis.session
        }
        fn operation(&self) -> OperationId {
            let GameInput::Game(command) = &self.input else {
                unreachable!()
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
    // Controlled durable-port fixture; physical PostgreSQL/fence qualification is separate.
    struct Database {
        checkpoint: Checkpoint,
        ledger: Vec<(Scope, DecisionReceipt)>,
        failure: Failure,
        commits: usize,
        decisions: usize,
        publications: usize,
        refuse_reload: bool,
        command_stages: usize,
        entry_checks: usize,
        with_axes: bool,
        axis_admissions: usize,
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
        axis: super::RegisteredAxisOwner,
        fixture: Fixture,
        source: SourceOwner,
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
            let with_axes = self.database.borrow().with_axes;
            let result = if with_axes {
                super::registered_completed_axis(
                    &self.fixture,
                    &self.source,
                    &self.axis,
                    current,
                    input,
                )
                .map_err(|_| RepositoryError::InvalidCandidate)
            } else {
                registered(&self.fixture, &self.source, current, input)
                    .map_err(|_| RepositoryError::InvalidCandidate)
            };
            let mut db = self.database.borrow_mut();
            db.command_stages = self.source.command_calls.get();
            db.entry_checks = self.source.entry_calls.get();
            db.axis_admissions = self.axis.admissions.get();
            result
        }
        fn validate_recovery(&mut self, checkpoint: &Checkpoint) -> Result<(), RepositoryError> {
            checkpoint
                .validate_resume(checkpoint.basis(), &self.fixture.pins)
                .map(|_| ())
                .map_err(|_| RepositoryError::InvalidCandidate)
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
            assert_eq!(
                checkpoint.state().relationships[0].state,
                model::label("receptive")
            );
            assert_eq!(
                checkpoint.state().relationships[1].state,
                model::label("independent-reverse")
            );
            assert_eq!(
                checkpoint.state().obligations[0].status,
                ObligationStatus::Fulfilled
            );
            if db.with_axes {
                assert_eq!(
                    checkpoint.state().relationships[1].affection,
                    super::completed_axis_change(RelationshipAxis::Affection).replacement
                );
            }
            db.publications += 1;
            Ok(())
        }
        fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
            Ok(())
        }
    }
    type Owner = DurableOwner<Repository, Engine, Publication>;
    fn setup(failure: Failure) -> (Owner, Rc<RefCell<Database>>, Scope) {
        setup_fixture(Fixture::new(), failure)
    }
    fn setup_fixture(fixture: Fixture, failure: Failure) -> (Owner, Rc<RefCell<Database>>, Scope) {
        setup_fixture_with_axes(fixture, failure, false)
    }
    fn setup_fixture_with_axes(
        fixture: Fixture,
        failure: Failure,
        with_axes: bool,
    ) -> (Owner, Rc<RefCell<Database>>, Scope) {
        let current = fixture.current();
        let scope = Scope {
            input: fixture.input(&current),
        };
        let source = SourceOwner::new(&fixture);
        let database = Rc::new(RefCell::new(Database {
            checkpoint: current.clone(),
            ledger: vec![],
            failure,
            commits: 0,
            decisions: 0,
            publications: 0,
            refuse_reload: false,
            command_stages: 0,
            entry_checks: 0,
            with_axes,
            axis_admissions: 0,
        }));
        let owner = DurableOwner::new(
            Repository {
                database: Rc::clone(&database),
            },
            Engine {
                axis: super::RegisteredAxisOwner::default(),
                fixture,
                source,
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
    fn submit(owner: &mut Owner, scope: &Scope) -> SubmissionOutcome {
        let (input, wait) = OwnedInput::new(
            OperationContext {
                trace_parent: String::new(),
                build: "relationship-fixture".to_owned(),
            },
            scope.clone(),
            scope.input.clone(),
        );
        owner.reduce(AdmissionSequence(1), input);
        wait.try_recv().unwrap()
    }

    #[test]
    fn accepted_axis_and_fulfillment_commit_together_and_exact_recovery_keeps_lineage() {
        for failure in [Failure::None, Failure::LostAck] {
            let (mut owner, database, scope) =
                setup_fixture_with_axes(Fixture::new(), failure, true);
            let original = owner.checkpoint().clone();
            let first = submit(&mut owner, &scope);
            let receipt = if failure == Failure::LostAck {
                assert_eq!(first, SubmissionOutcome::LookupRequired);
                assert_eq!(owner.checkpoint(), &original);
                assert_eq!(database.borrow().publications, 0);
                database.borrow_mut().refuse_reload = true;
                let receipt = submit(&mut owner, &scope);
                assert!(matches!(&receipt, SubmissionOutcome::Confirmed(_)));
                assert!(owner.has_uncertain_operation());
                assert_eq!(owner.checkpoint(), &original);
                database.borrow_mut().refuse_reload = false;
                assert_eq!(submit(&mut owner, &scope), receipt);
                receipt
            } else {
                assert!(matches!(&first, SubmissionOutcome::Confirmed(_)));
                first
            };
            let committed = database.borrow().checkpoint.clone();
            assert_eq!(submit(&mut owner, &scope), receipt);
            assert!(owner.is_current());
            assert_eq!(owner.checkpoint(), &committed);
            let db = database.borrow();
            assert_eq!(db.checkpoint, committed);
            let mut expected = original.state().relationships[1].clone();
            expected.affection =
                super::completed_axis_change(RelationshipAxis::Affection).replacement;
            assert_eq!(committed.state().relationships[1], expected);
            let mut reverse = original.state().relationships[0].clone();
            reverse.state = model::label("receptive");
            assert_eq!(committed.state().relationships[0], reverse);
            assert_eq!(
                committed.state().obligations[0].status,
                ObligationStatus::Fulfilled
            );
            assert!(committed.state().obligations[0].transition.is_some());
            assert_eq!(committed.state().draws, original.state().draws);
            assert_eq!(
                committed.state().facts.len(),
                original.state().facts.len() + 1
            );
            assert_eq!(
                (
                    db.decisions,
                    db.commits,
                    db.ledger.len(),
                    db.axis_admissions
                ),
                (1, 1, 1, 1)
            );
            assert_eq!(db.publications, usize::from(failure == Failure::None));
        }
    }

    #[test]
    fn refused_axis_commit_preserves_all_state_and_safe_retry_commits_once() {
        let (mut owner, database, scope) =
            setup_fixture_with_axes(Fixture::new(), Failure::BeforeCommit, true);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::Unavailable)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(database.borrow().checkpoint, original);
        assert_eq!(database.borrow().publications, 0);
        assert!(database.borrow().ledger.is_empty());
        database.borrow_mut().failure = Failure::None;
        let receipt = submit(&mut owner, &scope);
        assert!(matches!(&receipt, SubmissionOutcome::Confirmed(_)));
        let committed = owner.checkpoint().clone();
        assert_eq!(submit(&mut owner, &scope), receipt);
        assert_eq!(owner.checkpoint(), &committed);
        let db = database.borrow();
        assert_eq!(db.checkpoint, committed);
        assert_eq!(
            committed.state().relationships[1].affection,
            super::completed_axis_change(RelationshipAxis::Affection).replacement
        );
        assert_eq!(
            committed.state().obligations[0].status,
            ObligationStatus::Fulfilled
        );
        assert_eq!(
            (
                db.decisions,
                db.commits,
                db.publications,
                db.ledger.len(),
                db.axis_admissions
            ),
            (2, 2, 1, 1, 2)
        );
    }

    #[test]
    fn admitted_registered_reaction_commits_only_after_ack_and_exact_retry_returns_original_receipt()
     {
        let (mut owner, database, scope) = setup(Failure::None);
        let original = owner.checkpoint().clone();
        let first = submit(&mut owner, &scope);
        assert!(matches!(first, SubmissionOutcome::Confirmed(_)));
        assert_eq!(submit(&mut owner, &scope), first);
        let db = database.borrow();
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_eq!(
            db.checkpoint.state().relationships[0].state,
            model::label("receptive")
        );
        assert_eq!(
            db.checkpoint.state().relationships[1],
            original.state().relationships[1]
        );
        assert_eq!(
            db.checkpoint.state().facts.len(),
            original.state().facts.len() + 1
        );
        assert_eq!(original.state().draws.len(), 1);
        assert_eq!(db.checkpoint.state().draws, original.state().draws);
        assert_eq!(
            db.checkpoint
                .state()
                .decisions
                .iter()
                .find(|decision| decision.operation == operation(30))
                .unwrap()
                .draws,
            [0]
        );
        assert_eq!(
            (db.decisions, db.commits, db.publications, db.ledger.len()),
            (1, 1, 1, 1)
        );
        let mut conflict = scope.clone();
        let GameInput::Game(command) = &mut conflict.input else {
            unreachable!()
        };
        command.observed_revision = model::revision(2, 7);
        drop(db);
        assert_eq!(
            submit(&mut owner, &conflict),
            SubmissionOutcome::OperationConflict
        );
    }

    #[test]
    fn failed_commit_preserves_social_and_mechanical_state_and_safe_retry_applies_once() {
        let (mut owner, database, scope) = setup(Failure::BeforeCommit);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::Unavailable)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(database.borrow().checkpoint, original);
        assert_eq!(database.borrow().publications, 0);
        database.borrow_mut().failure = Failure::None;
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        let db = database.borrow();
        assert_eq!(
            (db.decisions, db.commits, db.publications, db.ledger.len()),
            (2, 2, 1, 1)
        );
        assert_eq!(
            db.checkpoint.state().relationships[0].state,
            model::label("receptive")
        );
    }

    #[test]
    fn lost_ack_and_failed_reload_fence_other_input_until_exact_receipt_and_validated_snapshot() {
        let (mut owner, database, scope) = setup(Failure::LostAck);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        assert!(owner.has_uncertain_operation());
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(
            database.borrow().checkpoint.state().relationships[0].state,
            model::label("receptive")
        );
        let mut other = scope.clone();
        let GameInput::Game(command) = &mut other.input else {
            unreachable!()
        };
        command.operation = operation(51);
        assert_eq!(
            submit(&mut owner, &other),
            SubmissionOutcome::LookupRequired
        );
        database.borrow_mut().refuse_reload = true;
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        assert!(owner.has_uncertain_operation());
        assert_eq!(owner.checkpoint(), &original);
        database.borrow_mut().refuse_reload = false;
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        assert!(owner.is_current());
        let db = database.borrow();
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_eq!(
            (db.decisions, db.commits, db.publications, db.ledger.len()),
            (1, 1, 0, 1)
        );
    }

    #[test]
    fn unknown_absent_commit_never_reexecutes_reaction_even_on_not_recorded_exact_lookup() {
        let (mut owner, database, scope) = setup(Failure::UnknownNotCommitted);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        database.borrow_mut().failure = Failure::None;
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        assert!(owner.has_uncertain_operation());
        let db = database.borrow();
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.checkpoint, original);
        assert_eq!(
            (db.decisions, db.commits, db.publications, db.ledger.len()),
            (1, 1, 0, 0)
        );
    }

    #[test]
    fn admitted_second_reaction_failure_never_commits_or_publishes_first_reaction_or_mechanical_siblings()
     {
        let mut fixture = Fixture::new();
        fixture.reaction_tail = true;
        let (mut owner, database, scope) = setup_fixture(fixture, Failure::None);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
        );
        let db = database.borrow();
        // One policy construction, a successful first react, then second react's policy recheck.
        assert_eq!((db.command_stages, db.entry_checks), (1, 3));
        assert_eq!(
            (db.decisions, db.commits, db.publications, db.ledger.len()),
            (1, 0, 0, 0)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.checkpoint, original);
        assert_eq!(db.checkpoint.state().draws.len(), 1);
        assert_eq!(
            db.checkpoint.state().relationships[0].state,
            model::label("reserved")
        );
        assert_eq!(
            db.checkpoint.state().relationships[2].state,
            model::label("reserved")
        );
        assert_eq!(
            db.checkpoint.state().obligations[0].status,
            ObligationStatus::Active
        );
    }
}

#[test]
fn last_reaction_source_and_constructor_failure_discard_all_tentative_mechanics_and_relationships()
{
    for failure in [
        "tail",
        "source",
        "constructor",
        "budget",
        "work",
        "proposal",
    ] {
        let mut fixture = Fixture::new();
        fixture.reaction_tail = failure == "tail";
        let owner = SourceOwner::new(&fixture);
        let current = fixture.current();
        let original = current.clone();
        let mut requests = vec![fixture.request(&current)];
        let mut bounds = limits();
        match failure {
            "tail" => {
                requests = fixture.requests(&current);
            }
            "source" => owner.withdraw_after_entry.set(Some(2)),
            "constructor" => bounds.checkpoint.maximum_records = 1,
            "budget" => bounds.maximum_pass_bytes = 1,
            "work" => bounds.maximum_work = 1,
            "proposal" => bounds.reaction.maximum_proposal_bytes = 1,
            _ => unreachable!(),
        }
        if failure == "tail" {
            let first = stage(
                &fixture,
                &owner,
                &current,
                &fixture.input(&current),
                &requests[..1],
                bounds,
            )
            .unwrap();
            assert!(matches!(&first.reactions[0], ReactionOutcome::Proposed(_)));
            assert_eq!(
                first.checkpoint.state().relationships[0].state,
                model::label("receptive")
            );
            assert_eq!(
                first.checkpoint.state().relationships[2],
                current.state().relationships[2]
            );
            assert_eq!(first.checkpoint.state().draws, current.state().draws);
            let input = fixture.input(&current);
            let sibling = stage_handler(
                &fixture.inner(&owner, &current),
                &fixture.pins,
                RulesCommandInput {
                    command: &input,
                    supplied_draws: &[],
                },
                &current,
                1024 * 1024,
            )
            .unwrap();
            let sibling_original = sibling.clone();
            let entries_before = owner.entry_calls.get();
            let commands_before = owner.command_calls.get();
            assert!(matches!(
                stage(&fixture, &owner, &current, &input, &requests, bounds),
                Err(RelationshipStagingError::Reaction(
                    ReactionError::UnknownEvent
                ))
            ));
            assert_eq!(owner.entry_calls.get() - entries_before, 3);
            assert_eq!(owner.command_calls.get() - commands_before, 1);
            assert_eq!(sibling, sibling_original);
            assert_eq!(sibling.state().relationships, current.state().relationships);
            assert_eq!(sibling.state().draws, current.state().draws);
        } else {
            assert!(
                stage(
                    &fixture,
                    &owner,
                    &current,
                    &fixture.input(&current),
                    &requests,
                    bounds
                )
                .is_err()
            );
        }
        assert_eq!(current, original);
    }
}

#[test]
fn stale_request_full_pins_and_unavailable_checkpoint_refuse_without_policy_access() {
    let fixture = Fixture::new();
    let owner = SourceOwner::new(&fixture);
    let current = fixture.current();
    let mut request = fixture.request(&current);
    request.expected_basis.revision = model::revision(2, 7);
    assert!(matches!(
        stage(
            &fixture,
            &owner,
            &current,
            &fixture.input(&current),
            &[request],
            limits()
        ),
        Err(RelationshipStagingError::StaleCommand)
    ));
    let mut state = fixture.state();
    state
        .continuity
        .recovery
        .unavailable_sources
        .push(model::label("withdrawn-source"));
    let unavailable = fixture.checkpoint(state);
    assert!(matches!(
        stage(
            &fixture,
            &owner,
            &unavailable,
            &fixture.input(&unavailable),
            &[fixture.request(&unavailable)],
            limits()
        ),
        Err(RelationshipStagingError::Snapshot(
            CheckpointError::UnavailableCheckpoint
        ))
    ));
    let mut other_pins = fixture.pins.clone();
    other_pins.content.content_digest = ContentDigest([99; 32]);
    let changed = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        current.basis(),
        other_pins,
        current.state().clone(),
        fixture.inventory(),
        model::limits(),
    )
    .unwrap();
    assert!(matches!(
        stage(
            &fixture,
            &owner,
            &changed,
            &fixture.input(&changed),
            &[fixture.request(&changed)],
            limits()
        ),
        Err(RelationshipStagingError::Snapshot(
            CheckpointError::ContentMismatch
        ))
    ));
    assert_eq!(owner.entry_calls.get(), 0);
}

#[derive(Clone, Copy)]
enum SiblingChange {
    Relationship,
    Knowledge,
}
struct ChangedSibling<'a> {
    fixture: &'a Fixture,
    owner: &'a SourceOwner,
    change: SiblingChange,
}
impl RulesCommandHandler for ChangedSibling<'_> {
    type Rejection = FulfillmentError<SourceRefusal>;
    fn pins(&self) -> &CheckpointPins {
        &self.fixture.pins
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        let base = self
            .fixture
            .inner(self.owner, current)
            .stage(input, current)?;
        let mut state = base.state().clone();
        match self.change {
            SiblingChange::Relationship => {
                state.relationships[0].state = model::label("conflicting")
            }
            SiblingChange::Knowledge => state.continuity.npcs[0].known_facts.push(fact(30)),
        }
        Ok(Checkpoint::new(
            base.schema(),
            base.basis(),
            base.pins().clone(),
            state,
            self.fixture.inventory(),
            model::limits(),
        )
        .unwrap())
    }
}

#[test]
fn changed_sibling_relationship_refuses_and_newly_staged_knowledge_cannot_authorize_reaction() {
    let fixture = Fixture::new();
    let owner = SourceOwner::new(&fixture);
    for change in [SiblingChange::Relationship, SiblingChange::Knowledge] {
        let mut state = fixture.state();
        let knowledge = matches!(change, SiblingChange::Knowledge);
        if knowledge {
            state.continuity.npcs[0].known_facts.clear();
        }
        let current = fixture.checkpoint(state);
        let original = current.clone();
        let sibling = ChangedSibling {
            fixture: &fixture,
            owner: &owner,
            change,
        };
        let requests = [fixture.request(&current)];
        let handler = RelationshipReactionHandler {
            command_handler: &sibling,
            owner: &owner,
            current_basis: current.basis(),
            admitted_pins: &fixture.pins,
            inventory: fixture.inventory(),
            requests: &requests,
            entries: &fixture.entries,
            limits: limits(),
        };
        let input = fixture.input(&current);
        let result = handler.stage_with_outcomes(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[],
            },
            &current,
        );
        if knowledge {
            let result = result.unwrap();
            assert_eq!(
                result.reactions,
                [ReactionOutcome::NoReaction(NoReactionReason::NotKnown)]
            );
            assert_eq!(
                result.checkpoint.state().relationships,
                current.state().relationships
            );
            assert_eq!(
                result.checkpoint.state().continuity.npcs[0].known_facts,
                [fact(30)]
            );
        } else {
            assert!(matches!(
                result,
                Err(RelationshipStagingError::RelationshipConflict)
            ));
        }
        assert_eq!(current, original);
    }
}

#[test]
fn exact_event_acceptance_and_prior_cause_acceptance_cannot_be_replaced_by_fact_presence() {
    for missing in ["event", "cause"] {
        let fixture = Fixture::new();
        let owner = SourceOwner::new(&fixture);
        let mut state = fixture.state();
        let mut event = state
            .facts
            .iter()
            .find(|record| record.id == fact(30))
            .unwrap()
            .clone();
        event.id = fact(32);
        event.ordinal = 2;
        event.cause = Some(fact(30));
        if missing == "cause" {
            let mut cause = state
                .facts
                .iter()
                .find(|record| record.id == fact(30))
                .unwrap()
                .clone();
            cause.id = fact(34);
            cause.operation = operation(33);
            cause.ordinal = 0;
            state.facts.push(cause);
            event.cause = Some(fact(34));
            state
                .decisions
                .iter_mut()
                .find(|decision| decision.operation == operation(30))
                .unwrap()
                .facts
                .push(fact(32));
        }
        state.facts.push(event);
        state.continuity.npcs[0].known_facts.push(fact(32));
        state.continuity.witnesses[0].fact = fact(32);
        let current = fixture.checkpoint(state);
        let original = current.clone();
        let mut request = fixture.request(&current);
        request.event = fact(32);
        assert!(matches!(
            stage(
                &fixture,
                &owner,
                &current,
                &fixture.input(&current),
                &[request],
                limits()
            ),
            Err(RelationshipStagingError::Reaction(
                ReactionError::UnacceptedEvent
            ))
        ));
        assert_eq!(current, original);
    }
}

#[derive(Default)]
struct RegisteredAxisOwner {
    application_calls: Cell<usize>,
    admissions: Cell<usize>,
    validations: Cell<usize>,
    withdrawn: Cell<bool>,
    refuse_at: Cell<Option<usize>>,
}

impl RelationshipOwner for RegisteredAxisOwner {
    type Subject = EntityId;
    type Basis = Basis;
    type Value = RevisionLabel;
    type Provenance = RelationshipAxisProvenance;
    type Refusal = AxisOwnerRefusal;

    fn validate_axis_change(
        &self,
        _: RelationshipAxis,
        _: &RelationshipAxisValue<Self::Value, Self::Provenance>,
        _: &RelationshipAxisValue<Self::Value, Self::Provenance>,
    ) -> Result<(), Self::Refusal> {
        self.validations.set(self.validations.get() + 1);
        Ok(())
    }
}

impl RelationshipAxisApplicationOwner for RegisteredAxisOwner {
    fn admit_axis_application(
        &self,
        current: &Checkpoint,
        command: &CommandInput,
        changes: &[RelationshipAxisChange],
    ) -> Result<(), Self::Refusal> {
        self.application_calls.set(self.application_calls.get() + 1);
        if self.withdrawn.get()
            || command.member != model::member(3)
            || command.operation != operation(50)
            || !matches!(&command.command,
                GameCommand::ProposeAction { actor, action, targets, choices }
                    if *actor == model::entity(4)
                        && action == &content("explicit-agreement")
                        && targets.as_slice() == [model::entity(5)]
                        && choices.is_empty())
            || changes.iter().any(|change| {
                change.subject != model::entity(4)
                    || change.target != model::entity(5)
                    || !current.state().relationships.iter().any(|relationship| {
                        relationship.subject == change.subject
                            && relationship.object == change.target
                            && relationship.policy == content("directional-policy")
                    })
            })
        {
            return Err(AxisOwnerRefusal::UnacceptedSource);
        }
        Ok(())
    }
}

impl RelationshipAxisOwner for RegisteredAxisOwner {
    fn admit_axis_change(
        &self,
        candidate: &Checkpoint,
        change: &RelationshipAxisChange,
    ) -> Result<(), Self::Refusal> {
        let calls = self.admissions.get() + 1;
        self.admissions.set(calls);
        if self.withdrawn.get() || self.refuse_at.get() == Some(calls) {
            return Err(AxisOwnerRefusal::UnacceptedSource);
        }
        let RelationshipAxisProvenance::AcceptedFact {
            source,
            fact: id,
            source_policy,
            witness,
        } = &change.replacement.provenance
        else {
            return Err(AxisOwnerRefusal::UnacceptedSource);
        };
        let admitted = candidate.state().facts.iter().any(|record| {
            record.id == *id
                && matches!(&record.value, FactValue::ContentEvent { definition, subjects }
                    if definition == &content("agreement-completed")
                        && subjects.as_slice() == [change.subject, change.target])
        });
        if admitted
            && source == &content("directional-policy")
            && source_policy == &model::label("compiled-completion-policy")
            && witness.is_none()
        {
            Ok(())
        } else {
            Err(AxisOwnerRefusal::UnacceptedSource)
        }
    }
}

fn completed_axis_change(axis: RelationshipAxis) -> RelationshipAxisChange {
    RelationshipAxisChange {
        subject: model::entity(4),
        target: model::entity(5),
        axis,
        replacement: RelationshipAxisState {
            value: model::label("axis-after-completion"),
            provenance: RelationshipAxisProvenance::AcceptedFact {
                source: content("directional-policy"),
                fact: fact(50),
                source_policy: model::label("compiled-completion-policy"),
                witness: None,
            },
        },
    }
}

fn registered_axis<H: RulesCommandHandler>(
    fixture: &Fixture,
    current: &Checkpoint,
    input: RulesCommandInput<'_>,
    handler: &H,
) -> Result<Checkpoint, df_engine::command_entry::CommandRejection<H::Rejection>> {
    use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
    use df_engine::command_entry::{
        CommandEntryContext, CommandEntryLimits, decide_registered_command,
    };
    use df_model::commands::CommandLimits;
    use df_rules::{DispatchRegistry, HandlerRegistration};

    let selector = model::label("actual-completion-with-axes");
    let entries = [CatalogEntry::new(
        &fixture.registration.source,
        b"complete synthetic source clause",
    )];
    let catalog = CatalogSnapshot::from_published(
        &fixture.pins.rules.catalog,
        &fixture.pins,
        b"complete synthetic catalog",
        &entries,
        CatalogLimits {
            max_complete_bytes: 64,
            max_entries: 1,
            max_item_bytes: 64,
            max_total_item_bytes: 64,
        },
    )
    .unwrap();
    let registrations = [HandlerRegistration::new(
        &selector,
        &fixture.registration.source,
        handler,
    )];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    decide_registered_command(
        input,
        current,
        CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: &fixture.pins,
            inventory: fixture.inventory(),
            limits: CommandEntryLimits {
                command: CommandLimits {
                    maximum_records: 100,
                    maximum_text_bytes: 256,
                    maximum_retained_bytes: 1024 * 1024,
                },
                maximum_staged_bytes: 1024 * 1024,
            },
        },
        &registry,
        &selector,
        &fixture.registration.source,
    )
}

fn registered_completed_axis(
    fixture: &Fixture,
    source: &SourceOwner,
    axis_owner: &RegisteredAxisOwner,
    current: &Checkpoint,
    input: &GameInput,
) -> Result<
    Checkpoint,
    df_engine::command_entry::CommandRejection<
        RelationshipAxisCommandError<Error, AxisOwnerRefusal>,
    >,
> {
    let inner = fixture.inner(source, current);
    let requests = fixture.requests(current);
    let reactions = RelationshipReactionHandler {
        command_handler: &inner,
        owner: source,
        current_basis: current.basis(),
        admitted_pins: &fixture.pins,
        inventory: fixture.inventory(),
        requests: &requests,
        entries: &fixture.entries,
        limits: limits(),
    };
    let changes = [completed_axis_change(RelationshipAxis::Affection)];
    let axes = RelationshipAxisHandler {
        command_handler: &reactions,
        owner: axis_owner,
        current_basis: current.basis(),
        admitted_pins: &fixture.pins,
        inventory: fixture.inventory(),
        changes: &changes,
        limits: axis_staging_limits(),
    };
    registered_axis(
        fixture,
        current,
        RulesCommandInput {
            command: input,
            supplied_draws: &[],
        },
        &axes,
    )
}

#[test]
fn registered_axis_completion_preserves_categorical_reaction_and_exact_replay() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let input = fixture.input(&current);
    let source = SourceOwner::new(&fixture);
    let owner = RegisteredAxisOwner::default();
    let original = current.clone();
    let staged = registered(&fixture, &source, &current, &input).unwrap();
    let mut expected = staged.state().clone();
    expected.relationships[1].affection =
        completed_axis_change(RelationshipAxis::Affection).replacement;
    let candidate = registered_completed_axis(&fixture, &source, &owner, &current, &input).unwrap();
    assert_eq!(candidate.state(), &expected);
    assert_eq!(
        candidate.state().relationships[0].state,
        model::label("receptive")
    );
    assert_eq!(
        candidate.state().relationships[1].affection,
        completed_axis_change(RelationshipAxis::Affection).replacement
    );
    assert_eq!(current, original);
    assert_eq!(
        registered_completed_axis(&fixture, &source, &owner, &current, &input).unwrap(),
        candidate
    );
}

#[test]
fn registered_axis_handler_admits_all_seven_axes_without_touching_siblings() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let original = current.clone();
    let input = fixture.input(&current);
    for axis in [
        RelationshipAxis::Trust,
        RelationshipAxis::Affection,
        RelationshipAxis::Respect,
        RelationshipAxis::Fear,
        RelationshipAxis::Suspicion,
        RelationshipAxis::Debt,
        RelationshipAxis::Familiarity,
    ] {
        let source = SourceOwner::new(&fixture);
        let owner = RegisteredAxisOwner::default();
        let inner = fixture.inner(&source, &current);
        let changes = [completed_axis_change(axis)];
        let handler = RelationshipAxisHandler {
            command_handler: &inner,
            owner: &owner,
            current_basis: current.basis(),
            admitted_pins: &fixture.pins,
            inventory: fixture.inventory(),
            changes: &changes,
            limits: axis_staging_limits(),
        };
        let staged = stage_handler(
            &inner,
            &fixture.pins,
            RulesCommandInput {
                command: &input,
                supplied_draws: &[],
            },
            &current,
            1024 * 1024,
        )
        .unwrap();
        let candidate = registered_axis(
            &fixture,
            &current,
            RulesCommandInput {
                command: &input,
                supplied_draws: &[],
            },
            &handler,
        )
        .unwrap();
        let mut expected = staged.state().clone();
        let record = &mut expected.relationships[1];
        let destination = match axis {
            RelationshipAxis::Trust => &mut record.trust,
            RelationshipAxis::Affection => &mut record.affection,
            RelationshipAxis::Respect => &mut record.respect,
            RelationshipAxis::Fear => &mut record.fear,
            RelationshipAxis::Suspicion => &mut record.suspicion,
            RelationshipAxis::Debt => &mut record.debt,
            RelationshipAxis::Familiarity => &mut record.familiarity,
        };
        *destination = completed_axis_change(axis).replacement;
        assert_eq!(candidate.state(), &expected);
        assert_eq!(candidate.pins(), &fixture.pins);
        assert_eq!(
            (
                owner.application_calls.get(),
                owner.admissions.get(),
                owner.validations.get()
            ),
            (1, 1, 1)
        );
        assert_eq!(current, original);
    }
}

#[test]
fn axis_command_stale_actor_source_and_pending_refuse_before_mechanics() {
    let fixture = Fixture::new();
    let mut pending = fixture.state();
    pending.pending.push(PendingResolution {
        id: ResolutionId::from_bytes(&[71; 16]).unwrap(),
        basis: model::basis(),
        continuation: model::label("continuation"),
        window: ResolutionWindow {
            id: WindowId::from_bytes(&[72; 16]).unwrap(),
            phase: TriggerPhase::BeforeConsequence,
            causal_fact: fact(30),
            source: model::rule(),
            timer: None,
        },
        next: PendingInput::Roll {
            participant: model::member(3),
            sides: vec![6],
            source: model::rule(),
        },
        choices: vec![],
        draw_ordinals: vec![],
        spent: vec![],
        rulings: vec![],
    });
    let pending = fixture.checkpoint(pending);
    let base = fixture.current();
    for dimension in ["basis", "revision", "actor", "source", "pending", "pins"] {
        let current = if dimension == "pending" {
            &pending
        } else {
            &base
        };
        let original = current.clone();
        let source = SourceOwner::new(&fixture);
        let owner = RegisteredAxisOwner::default();
        let mut input = fixture.input(current);
        let GameInput::Game(command) = &mut input else {
            unreachable!()
        };
        let mut pins = fixture.pins.clone();
        match dimension {
            "basis" => command.basis.revision = model::revision(2, 7),
            "revision" => command.observed_revision = model::revision(2, 7),
            "actor" => {
                let GameCommand::ProposeAction { actor, .. } = &mut command.command else {
                    unreachable!()
                };
                *actor = model::entity(5);
            }
            "source" => owner.withdrawn.set(true),
            "pins" => pins.content.content = model::label("stale-content"),
            "pending" => {}
            _ => unreachable!(),
        }
        let inner = fixture.inner(&source, current);
        let changes = [completed_axis_change(RelationshipAxis::Affection)];
        let handler = RelationshipAxisHandler {
            command_handler: &inner,
            owner: &owner,
            current_basis: current.basis(),
            admitted_pins: &pins,
            inventory: fixture.inventory(),
            changes: &changes,
            limits: axis_staging_limits(),
        };
        assert!(
            handler
                .stage(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &[]
                    },
                    current,
                )
                .is_err(),
            "{dimension}"
        );
        assert_eq!(source.command_calls.get(), 0, "{dimension}");
        assert_eq!(owner.admissions.get(), 0, "{dimension}");
        assert_eq!(current, &original);
    }
}

#[test]
fn axis_command_requires_current_operation_fact_and_exact_source_policy() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let original = current.clone();
    let input = fixture.input(&current);
    for dimension in [
        "historical",
        "missing",
        "policy",
        "source",
        "witness",
        "baseline",
    ] {
        let source = SourceOwner::new(&fixture);
        let owner = RegisteredAxisOwner::default();
        let inner = fixture.inner(&source, &current);
        let mut change = completed_axis_change(RelationshipAxis::Affection);
        let RelationshipAxisProvenance::AcceptedFact {
            source: source_ref,
            fact: id,
            source_policy,
            witness,
        } = &mut change.replacement.provenance
        else {
            unreachable!()
        };
        match dimension {
            "historical" => *id = fact(30),
            "missing" => *id = fact(99),
            "policy" => *source_policy = model::label("unaccepted-policy"),
            "source" => *source_ref = content("authored-reaction"),
            "witness" => *witness = Some(record(31)),
            "baseline" => {
                change.replacement.provenance = RelationshipAxisProvenance::AuthoredBaseline {
                    source: content("directional-policy"),
                }
            }
            _ => unreachable!(),
        }
        let handler = RelationshipAxisHandler {
            command_handler: &inner,
            owner: &owner,
            current_basis: current.basis(),
            admitted_pins: &fixture.pins,
            inventory: fixture.inventory(),
            changes: &[change],
            limits: axis_staging_limits(),
        };
        assert!(
            handler
                .stage(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &[]
                    },
                    &current,
                )
                .is_err(),
            "{dimension}"
        );
        assert_eq!(current, original);
        assert_eq!(owner.validations.get(), 0);
    }
}

#[test]
fn axis_command_duplicate_and_last_source_refusal_publish_no_partial_candidate() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let original = current.clone();
    let input = fixture.input(&current);
    for duplicate in [true, false] {
        let source = SourceOwner::new(&fixture);
        let owner = RegisteredAxisOwner::default();
        if !duplicate {
            owner.refuse_at.set(Some(2));
        }
        let inner = fixture.inner(&source, &current);
        let changes = [
            completed_axis_change(RelationshipAxis::Affection),
            completed_axis_change(if duplicate {
                RelationshipAxis::Affection
            } else {
                RelationshipAxis::Trust
            }),
        ];
        let handler = RelationshipAxisHandler {
            command_handler: &inner,
            owner: &owner,
            current_basis: current.basis(),
            admitted_pins: &fixture.pins,
            inventory: fixture.inventory(),
            changes: &changes,
            limits: axis_staging_limits(),
        };
        let result = handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[],
            },
            &current,
        );
        assert_eq!(
            result,
            Err(RelationshipAxisCommandError::Axis(if duplicate {
                RelationshipAxisStagingError::DuplicateDirection
            } else {
                RelationshipAxisStagingError::Source(AxisOwnerRefusal::UnacceptedSource)
            }))
        );
        assert_eq!(current, original);
        assert_eq!(owner.validations.get(), usize::from(!duplicate));
    }
}

#[test]
fn axis_command_capacity_refuses_before_native_owner_or_inner_stage() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let original = current.clone();
    let input = fixture.input(&current);
    let mut count = axis_staging_limits();
    count.maximum_changes = 0;
    let mut inventory = axis_staging_limits();
    inventory.maximum_inventory_records = 1;
    let mut retained = axis_staging_limits();
    retained.maximum_checkpoint_bytes = 1;
    let mut pass = axis_staging_limits();
    pass.maximum_pass_bytes = 1;
    let mut records = axis_staging_limits();
    records.checkpoint.maximum_records = 1;
    for limits in [count, inventory, retained, pass, records] {
        let source = SourceOwner::new(&fixture);
        let owner = RegisteredAxisOwner::default();
        let inner = fixture.inner(&source, &current);
        let changes = [completed_axis_change(RelationshipAxis::Affection)];
        let handler = RelationshipAxisHandler {
            command_handler: &inner,
            owner: &owner,
            current_basis: current.basis(),
            admitted_pins: &fixture.pins,
            inventory: fixture.inventory(),
            changes: &changes,
            limits,
        };
        assert!(
            handler
                .stage(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &[]
                    },
                    &current,
                )
                .is_err()
        );
        assert_eq!(
            (
                owner.application_calls.get(),
                owner.admissions.get(),
                source.command_calls.get()
            ),
            (0, 0, 0)
        );
        assert_eq!(current, original);
    }
}

struct ObservedAxisInner<'a> {
    fixture: &'a Fixture,
    owner: &'a SourceOwner,
    command: Cell<usize>,
    draws: Cell<usize>,
    corrupt_axis: bool,
}
impl RulesCommandHandler for ObservedAxisInner<'_> {
    type Rejection = FulfillmentError<SourceRefusal>;
    fn pins(&self) -> &CheckpointPins {
        &self.fixture.pins
    }
    fn bound_source(&self) -> Option<&RuleReference> {
        Some(&self.fixture.registration.source)
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        self.command.set(input.command as *const GameInput as usize);
        self.draws.set(input.supplied_draws.as_ptr() as usize);
        let staged = self
            .fixture
            .inner(self.owner, current)
            .stage(input, current)?;
        if !self.corrupt_axis {
            return Ok(staged);
        }
        let mut state = staged.state().clone();
        state.relationships[0].trust.value = model::label("conflicting-inner-axis");
        Ok(Checkpoint::new(
            staged.schema(),
            staged.basis(),
            staged.pins().clone(),
            state,
            self.fixture.inventory(),
            model::limits(),
        )
        .unwrap())
    }
}

#[test]
fn axis_command_preserves_borrowed_input_source_and_rejects_inner_axis_conflict() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let original = current.clone();
    let input = fixture.input(&current);
    let supplied: [ActualDraw; 0] = [];
    for corrupt_axis in [false, true] {
        let source = SourceOwner::new(&fixture);
        let owner = RegisteredAxisOwner::default();
        let inner = ObservedAxisInner {
            fixture: &fixture,
            owner: &source,
            command: Cell::new(0),
            draws: Cell::new(0),
            corrupt_axis,
        };
        let changes = [completed_axis_change(RelationshipAxis::Affection)];
        let handler = RelationshipAxisHandler {
            command_handler: &inner,
            owner: &owner,
            current_basis: current.basis(),
            admitted_pins: &fixture.pins,
            inventory: fixture.inventory(),
            changes: &changes,
            limits: axis_staging_limits(),
        };
        assert_eq!(handler.bound_source(), Some(&fixture.registration.source));
        let result = registered_axis(
            &fixture,
            &current,
            RulesCommandInput {
                command: &input,
                supplied_draws: &supplied,
            },
            &handler,
        );
        assert_eq!(result.is_err(), corrupt_axis);
        assert_eq!(inner.command.get(), &input as *const GameInput as usize);
        assert_eq!(inner.draws.get(), supplied.as_ptr() as usize);
        assert_eq!(current, original);
    }
}
