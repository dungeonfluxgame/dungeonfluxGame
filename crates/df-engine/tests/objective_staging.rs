#[path = "support/fixture_model.rs"]
pub mod fixture_model;
#[path = "support/objective_staging.rs"]
pub mod support;

use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_encounter::objectives::{
    AuthoredObjectiveTransition, ObjectivePolicyError, ObjectivePolicyOwner,
    ObjectiveProposalError, ObjectiveProposalLimits,
};
use df_engine::command_entry::{
    CommandEntryContext, CommandEntryLimits, CommandRejection, decide_registered_command,
};
use df_engine::objective_staging::*;
use df_model::checkpoint::*;
use df_model::commands::CommandLimits;
use df_rules::preconditions::*;
use df_rules::{
    DispatchRegistry, HandlerRegistration, InvocationError, RulesCommandHandler, RulesCommandInput,
};
use df_types::OperationId;
use fixture_model as fixture;
use std::cell::Cell;
use support::*;

fn rejected(error: ObjectiveStagingError<SourceRefusal>) -> Rejection {
    CommandRejection::Invocation(InvocationError::Handler(PreconditionedRejection::Handler(
        error,
    )))
}

#[test]
fn registered_objective_batch_preserves_private_accepted_causes_and_every_sibling() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    let current = fixture.current();
    let before = current.clone();
    let input = fixture.input(current.basis());
    let next = registered(&fixture, &source, &current, &input).unwrap();
    assert_preserved(&fixture, &current, &next);
    assert_eq!(current, before);
    assert_eq!((source.calls.get(), source.applications.get()), (2, 1));
    let repeated = registered(&fixture, &source, &current, &input).unwrap();
    assert_eq!(next, repeated);
    assert_eq!(
        next.state().facts[0].audience,
        AudienceScope::Members(vec![fixture::member(3)])
    );
}

#[test]
fn stale_objective_conflicting_tail_and_denied_tail_discard_the_complete_batch() {
    let fixture = Fixture::new();
    for case in ["stale", "conflict", "rights", "missing-slot"] {
        let source = SourceOwner::new(&fixture);
        let mut state = fixture.state();
        if case == "stale" {
            state.encounters[0].objectives[1] = fixture.replacements[1].clone();
        }
        let current = fixture.checkpoint(fixture::basis(), state);
        let before = current.clone();
        let mut changes = fixture.transitions(&current);
        if case == "conflict" {
            let mut duplicate = fixture.transitions(&current);
            changes.push(duplicate.remove(0));
        }
        if case == "rights" {
            source.refused_index.set(Some(1));
        }
        if case == "missing-slot" {
            // Source owner refuses an unadmitted mapping before any slot can be applied.
            changes[1].objective_index = usize::MAX;
        }
        let handler = fixture.handler(&source, current.basis(), Some(&changes));
        let expected = match case {
            "stale" => ObjectiveProposalError::StaleObjective,
            "conflict" => ObjectiveProposalError::ConflictingChanges,
            "rights" => ObjectiveProposalError::Policy(ObjectivePolicyError::RightsDenied),
            _ => ObjectiveProposalError::Policy(ObjectivePolicyError::PolicyMismatch),
        };
        assert_eq!(
            registered_handler(
                &fixture,
                &current,
                &current,
                &fixture.input(current.basis()),
                &handler
            ),
            Err(rejected(ObjectiveStagingError::Objective(expected)))
        );
        assert_eq!(current, before);
        assert_eq!(source.applications.get(), 0);
    }
}

#[test]
fn accepted_cause_policy_audience_and_subjects_are_revalidated_from_current_state() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    for case in ["policy", "audience", "subjects"] {
        let mut state = fixture.state();
        match case {
            "policy" => state.decisions[0].source_policy = fixture::label("different-cause-policy"),
            "audience" => state.facts[1].audience = AudienceScope::Shared,
            _ => {
                let FactValue::ContentEvent { subjects, .. } = &mut state.facts[1].value else {
                    unreachable!()
                };
                subjects.clear();
            }
        }
        let current = fixture.checkpoint(fixture::basis(), state);
        let before = current.clone();
        assert_eq!(
            registered(&fixture, &source, &current, &fixture.input(current.basis())),
            Err(rejected(ObjectiveStagingError::Objective(
                ObjectiveProposalError::Policy(ObjectivePolicyError::PolicyMismatch)
            )))
        );
        assert_eq!(current, before);
    }
}

#[test]
fn source_withdrawal_and_missing_producer_never_become_successful_empty_batches() {
    let fixture = Fixture::new();
    let current = fixture.current();
    let before = current.clone();
    let source = SourceOwner::new(&fixture);
    source.refusal.set(Some(SourceRefusal::Withdrawn));
    assert_eq!(
        registered(&fixture, &source, &current, &fixture.input(current.basis())),
        Err(rejected(ObjectiveStagingError::Source(
            SourceRefusal::Withdrawn
        )))
    );
    let missing = fixture.handler(&source, current.basis(), None);
    assert_eq!(
        registered_handler(
            &fixture,
            &current,
            &current,
            &fixture.input(current.basis()),
            &missing
        ),
        Err(rejected(ObjectiveStagingError::MissingProducer))
    );
    assert_eq!(current, before);
    let mut state = fixture.state();
    state
        .continuity
        .recovery
        .unavailable_sources
        .push(fixture::rule().source);
    let unavailable = fixture.checkpoint(current.basis(), state);
    let source = SourceOwner::new(&fixture);
    assert!(
        registered(
            &fixture,
            &source,
            &unavailable,
            &fixture.input(unavailable.basis())
        )
        .is_err()
    );
    assert_eq!((source.calls.get(), source.applications.get()), (0, 0));
}

#[test]
fn mismatched_native_pins_basis_prepared_encounter_and_actor_refuse_before_policy() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    let current = fixture.current();
    let changes = fixture.transitions(&current);
    for case in ["pins", "basis", "observed", "actor", "targets"] {
        let mut handler = fixture.handler(&source, current.basis(), Some(&changes));
        let mut pins = fixture.pins.clone();
        pins.rules.source_manifest_digest = ContentDigest([9; 32]);
        let mut input = fixture.input(current.basis());
        let GameInput::Game(command) = &mut input else {
            unreachable!()
        };
        match case {
            "pins" => handler.admitted_pins = &pins,
            "basis" => handler.current_basis.revision = fixture::revision(2, 7),
            "observed" => command.observed_revision = fixture::revision(2, 7),
            _ => {
                let GameCommand::ProposeAction { actor, targets, .. } = &mut command.command else {
                    unreachable!()
                };
                if case == "actor" {
                    *actor = fixture::entity(9);
                } else {
                    targets.push(fixture::entity(4));
                }
            }
        }
        assert!(
            handler
                .stage(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &[]
                    },
                    &current
                )
                .is_err()
        );
    }
    let mut state = fixture.state();
    state.encounters[0].objectives[1] = fixture.replacements[1].clone();
    let changed = fixture.checkpoint(current.basis(), state);
    let changes = fixture.transitions(&changed);
    let handler = fixture.handler(&source, changed.basis(), Some(&changes));
    assert_eq!(
        registered_handler(
            &fixture,
            &current,
            &changed,
            &fixture.input(changed.basis()),
            &handler
        ),
        Err(CommandRejection::Invocation(InvocationError::Handler(
            PreconditionedRejection::Precondition(PreconditionError::StaleEncounter)
        )))
    );
    assert_eq!((source.calls.get(), source.applications.get()), (0, 0));
    assert_eq!(current, fixture.current());
}

#[test]
fn proposer_capacity_and_final_constructor_failure_return_no_partial_transition() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    let current = fixture.current();
    let changes = fixture.transitions(&current);
    let input = fixture.input(current.basis());
    for case in ["pass", "input", "work"] {
        let mut handler = fixture.handler(&source, current.basis(), Some(&changes));
        match case {
            "pass" => handler.limits.maximum_pass_bytes = 1,
            "input" => handler.limits.objectives.maximum_input_bytes = 1,
            _ => handler.limits.objectives.maximum_comparisons = 1,
        }
        let result = handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[],
            },
            &current,
        );
        assert!(result.is_err());
        assert_eq!(current, fixture.current());
    }
    assert_eq!(source.applications.get(), 0);
    let mut handler = fixture.handler(&source, current.basis(), Some(&changes));
    // The complete original fixture has 25 root/nested records. Objective replacement
    // fits that bound; adding its accepted application decision is the failing final tail.
    handler.limits.checkpoint.maximum_records = 25;
    assert_eq!(
        handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current
        ),
        Err(ObjectiveStagingError::InvalidCandidate(
            CheckpointError::Capacity
        ))
    );
    assert_eq!(source.applications.get(), 1);
    assert_eq!(current, fixture.current());
}

#[test]
fn member_link_cannot_substitute_for_native_actor_control_admission() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    let mut state = fixture.state();
    state.characters[0].owner = fixture::member(8);
    state.members.push(MembershipLink {
        member: fixture::member(8),
        character: Some(fixture::entity(4)),
    });
    let current = fixture.checkpoint(fixture::basis(), state);
    assert_eq!(
        registered(&fixture, &source, &current, &fixture.input(current.basis())),
        Err(rejected(ObjectiveStagingError::Source(
            SourceRefusal::ActorControl
        )))
    );
    assert_eq!(current.state().decisions.len(), 1);
}

#[test]
fn supplied_draws_and_current_pending_rules_do_not_enter_objective_policy() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    let current = fixture.current();
    let changes = fixture.transitions(&current);
    let handler = fixture.handler(&source, current.basis(), Some(&changes));
    let input = fixture.input(current.basis());
    let draw = ActualDraw {
        operation: operation(50),
        ordinal: 0,
        resolution: ResolutionId::from_bytes(&[9; 16]).unwrap(),
        window: WindowId::from_bytes(&[10; 16]).unwrap(),
        sides: 20,
        value: 1,
        source: fixture::rule(),
    };
    assert_eq!(
        handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[draw]
            },
            &current
        ),
        Err(ObjectiveStagingError::UnsupportedInput)
    );
    let mut state = fixture.state();
    state.pending.push(PendingResolution {
        id: ResolutionId::from_bytes(&[9; 16]).unwrap(),
        basis: current.basis(),
        continuation: fixture::label("existing-objective-rules-continuation"),
        window: ResolutionWindow {
            id: WindowId::from_bytes(&[10; 16]).unwrap(),
            causal_fact: state.facts[0].id,
            source: fixture::rule(),
            phase: TriggerPhase::BeforeDraw,
            timer: None,
        },
        next: PendingInput::Choice {
            remaining: vec![OfferedResponse {
                participant: fixture::member(3),
                offer: fixture::label("existing-objective-rules-offer"),
                options: vec![fixture::label("existing-objective-rules-option")],
                source: fixture::rule(),
            }],
        },
        choices: vec![],
        draw_ordinals: vec![],
        spent: vec![],
        rulings: vec![],
    });
    let pending = fixture.checkpoint(current.basis(), state);
    let before = pending.clone();
    let changes = fixture.transitions(&pending);
    let handler = fixture.handler(&source, pending.basis(), Some(&changes));
    assert_eq!(
        registered_handler(
            &fixture,
            &pending,
            &pending,
            &fixture.input(pending.basis()),
            &handler
        ),
        Err(rejected(ObjectiveStagingError::PendingResolution))
    );
    assert_eq!((source.calls.get(), source.applications.get()), (0, 0));
    assert_eq!(pending, before);
}

#[test]
fn exact_forged_cause_and_wrong_registered_source_are_never_applied() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    let current = fixture.current();
    let input = fixture.input(current.basis());
    let mut forged = current.state().facts[1].clone();
    forged.audience = AudienceScope::Shared;
    let mut changes = fixture.transitions(&current);
    changes[1].cause = &forged;
    let handler = fixture.handler(&source, current.basis(), Some(&changes));
    assert_eq!(
        registered_handler(&fixture, &current, &current, &input, &handler),
        Err(rejected(ObjectiveStagingError::Objective(
            ObjectiveProposalError::Policy(ObjectivePolicyError::PolicyMismatch)
        )))
    );
    let wrong_source = RuleReference {
        clause: fixture::label("other-clause"),
        ..fixture::rule()
    };
    let mut changes = fixture.transitions(&current);
    changes[1].source = &wrong_source;
    let handler = fixture.handler(&source, current.basis(), Some(&changes));
    assert_eq!(
        handler.stage(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current
        ),
        Err(ObjectiveStagingError::InvalidRegistration)
    );
    assert_eq!(current, fixture.current());
}

#[test]
fn authored_input_order_preserves_canonical_slot_order_and_admitted_no_change_is_explicit() {
    let fixture = Fixture::new();
    let source = SourceOwner::new(&fixture);
    let current = fixture.current();
    let input = fixture.input(current.basis());
    let mut changes = fixture.transitions(&current);
    changes.reverse();
    let handler = fixture.handler(&source, current.basis(), Some(&changes));
    let next = registered_handler(&fixture, &current, &current, &input, &handler).unwrap();
    assert_preserved(&fixture, &current, &next);
    let handler = fixture.handler(&source, current.basis(), Some(&[]));
    let next = registered_handler(&fixture, &current, &current, &input, &handler).unwrap();
    let mut expected = current.state().clone();
    expected
        .decisions
        .push(next.state().decisions.last().unwrap().clone());
    assert_eq!(next.state(), &expected);
    assert_eq!(next.state().encounters, current.state().encounters);
}

// These synthetic reviewed mappings exercise source admission, not a rulebook catalog.
fn end_state_fixture(outcome: &str) -> Fixture {
    let mut fixture = Fixture::new();
    fixture.replacements = [
        content(&format!("{outcome}-first")),
        content(&format!("{outcome}-second")),
    ];
    fixture.content.extend(fixture.replacements.iter().cloned());
    fixture
}

struct EndStateSource {
    source: SourceOwner,
    target: EncounterState,
    replacements: [ContentReference; 2],
    terminal: bool,
    refusal: Cell<Option<ObjectivePolicyError>>,
    calls: Cell<usize>,
}

impl EndStateSource {
    fn new(fixture: &Fixture, terminal: bool) -> Self {
        Self {
            source: SourceOwner::new(fixture),
            target: fixture.current().state().encounters[0].clone(),
            replacements: fixture.replacements.clone(),
            terminal,
            refusal: Cell::new(None),
            calls: Cell::new(0),
        }
    }
}

impl ObjectivePolicyOwner for EndStateSource {
    fn admit_transition(
        &self,
        current: &Checkpoint,
        transition: &AuthoredObjectiveTransition<'_>,
    ) -> Result<(), ObjectivePolicyError> {
        self.source.admit_transition(current, transition)
    }

    fn admit_end_state(
        &self,
        _: &Checkpoint,
        target: &EncounterState,
        changes: &[AuthoredObjectiveTransition<'_>],
    ) -> Result<bool, ObjectivePolicyError> {
        self.calls.set(self.calls.get() + 1);
        if let Some(refusal) = self.refusal.get() {
            return Err(refusal);
        }
        if target != &self.target
            || changes.len() != 2
            || !self
                .replacements
                .iter()
                .enumerate()
                .all(|(index, replacement)| {
                    changes.iter().any(|change| {
                        change.encounter == target.id
                            && change.objective_index == index
                            && change.replacement == replacement
                    })
                })
        {
            return Err(ObjectivePolicyError::PolicyMismatch);
        }
        Ok(self.terminal)
    }
}

impl ObjectiveSourceOwner for EndStateSource {
    type Refusal = SourceRefusal;

    fn admit_application(
        &self,
        current: &Checkpoint,
        registration: &ObjectiveRegistration,
        command: &CommandInput,
        transitions: &[AuthoredObjectiveTransition<'_>],
    ) -> Result<(), Self::Refusal> {
        self.source
            .admit_application(current, registration, command, transitions)
    }
}

fn registered_end_state(
    fixture: &Fixture,
    source: &EndStateSource,
    current: &Checkpoint,
    input: &GameInput,
) -> Result<Checkpoint, Rejection> {
    let changes = fixture.transitions(current);
    let base = fixture.handler(&source.source, current.basis(), Some(&changes));
    let handler = ObjectiveTransitionHandler {
        owner: source,
        current_basis: base.current_basis,
        admitted_pins: base.admitted_pins,
        inventory: fixture.inventory(),
        registration: base.registration,
        transitions: Some(&changes),
        limits: base.limits,
    };
    registered_end_state_handler(fixture, current, current, input, &handler)
}

fn assert_end_state_preserved(fixture: &Fixture, original: &Checkpoint, next: &Checkpoint) {
    let mut expected = original.state().clone();
    let target = &mut expected.encounters[0];
    target.objectives = fixture.replacements.to_vec();
    target.turn_order.clear();
    target.active_turn = None;
    expected.decisions.push(AcceptedDecision {
        operation: operation(50),
        revision: original.basis().revision.next_sequence().unwrap(),
        facts: vec![],
        draws: vec![],
        effects: vec![],
        source_policy: fixture.registration.policy.clone(),
        semantic_output: None,
    });
    assert_eq!(next.state(), &expected);
    assert_eq!(next.pins(), original.pins());
    let mut basis = original.basis();
    basis.revision = basis.revision.next_sequence().unwrap();
    assert_eq!(next.basis(), basis);
}

#[test]
fn registered_success_failure_escape_are_source_admitted_and_continuation_preserves_turns() {
    for outcome in ["reviewed-success", "reviewed-failure", "reviewed-escape"] {
        let fixture = end_state_fixture(outcome);
        let current = fixture.current();
        let before = current.clone();
        let input = fixture.input(current.basis());
        let source = EndStateSource::new(&fixture, true);
        let next = registered_end_state(&fixture, &source, &current, &input).unwrap();
        assert_end_state_preserved(&fixture, &current, &next);
        assert_eq!(source.calls.get(), 1);
        assert_eq!(current, before);
        let continuing = EndStateSource::new(&fixture, false);
        let next = registered_end_state(&fixture, &continuing, &current, &input).unwrap();
        assert_preserved(&fixture, &current, &next);
        assert_eq!(continuing.calls.get(), 1);
    }
}

#[test]
fn registered_end_state_denial_has_no_candidate_or_application_admission() {
    let fixture = end_state_fixture("reviewed-escape");
    let current = fixture.current();
    let before = current.clone();
    let source = EndStateSource::new(&fixture, true);
    source.refusal.set(Some(ObjectivePolicyError::RightsDenied));
    assert_eq!(
        registered_end_state(&fixture, &source, &current, &fixture.input(current.basis())),
        Err(rejected(ObjectiveStagingError::Objective(
            ObjectiveProposalError::Policy(ObjectivePolicyError::RightsDenied)
        )))
    );
    assert_eq!(source.source.calls.get(), 2);
    assert_eq!(source.calls.get(), 1);
    assert_eq!(source.source.applications.get(), 0);
    assert_eq!(current, before);
}

fn registered_end_state_handler(
    fixture: &Fixture,
    prepared: &Checkpoint,
    current: &Checkpoint,
    input: &GameInput,
    handler: &ObjectiveTransitionHandler<'_, EndStateSource>,
) -> Result<Checkpoint, Rejection> {
    let dependencies = [
        RuleDependency::Entity(fixture::entity(4)),
        RuleDependency::Encounter(encounter(12)),
        RuleDependency::PendingResolutions,
    ];
    let sources = [fixture.registration.source.clone()];
    let guarded = PreconditionedCommandHandler::new(
        handler,
        &fixture.registration.source,
        CurrentRuleContext {
            checkpoint: current,
            basis: handler.current_basis,
            pins: handler.admitted_pins,
            inventory: fixture.inventory(),
            command_limits: command_limits(),
        },
        RulePreconditions {
            prepared,
            sources: &sources,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 4,
            maximum_comparisons: 10_000_000,
            maximum_checkpoint_bytes: 1024 * 1024,
        },
    );
    let selector = fixture::label("compiled-objective-application");
    let entries = [CatalogEntry::new(
        &fixture.registration.source,
        b"exact synthetic authored objective source",
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
        &guarded,
    )];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    decide_registered_command(
        RulesCommandInput {
            command: input,
            supplied_draws: &[],
        },
        current,
        CommandEntryContext {
            current_basis: handler.current_basis,
            admitted_pins: handler.admitted_pins,
            inventory: fixture.inventory(),
            limits: CommandEntryLimits {
                command: command_limits(),
                maximum_staged_bytes: 1024 * 1024,
            },
        },
        &registry,
        &selector,
        &fixture.registration.source,
    )
}

#[cfg(not(target_arch = "wasm32"))]
mod durable_session {
    use super::*;
    use df_observe::OperationContext;
    use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
    use df_session::submission::*;
    use df_types::{MemberId, SessionId};
    use std::cell::RefCell;
    use std::rc::Rc;

    // Existing controlled repository pattern executes the real DurableOwner. Physical PG
    // transaction/lease proof remains the persistence gate, not this deterministic fixture.
    #[derive(Clone, Eq, PartialEq)]
    struct Scope {
        input: GameInput,
        principal: MemberId,
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
            if &self.input != input {
                return Err(RepositoryError::InputBinding);
            }
            let GameInput::Game(command) = input else {
                return Err(RepositoryError::InputBinding);
            };
            if command.member != self.principal || self.principal != fixture::member(3) {
                return Err(RepositoryError::Unauthorized);
            }
            Ok(())
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
        Publication,
        SourceWithdrawn,
        StaleFence,
        RevisionConflict,
        DeniedTail,
        DeniedEndState,
    }
    struct Database {
        checkpoint: Checkpoint,
        replacements: [ContentReference; 2],
        end_state_admissions: usize,
        ledger: Vec<(Scope, DecisionReceipt)>,
        failure: Failure,
        commits: usize,
        decisions: usize,
        publications: usize,
        wakes: usize,
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
            scope.validate_input(&scope.input)?;
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
            scope.validate_input(&scope.input)?;
            let mut db = self.database.borrow_mut();
            db.commits += 1;
            if db.failure == Failure::StaleFence {
                return Err(RepositoryError::StaleFence);
            }
            if expected != db.checkpoint.basis() || db.failure == Failure::RevisionConflict {
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
            // One atomic fixture write covers both objective slots, checkpoint and exact receipt.
            assert_eq!(candidate.state().encounters[0].objectives, db.replacements);
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
            scope: &Scope,
            _: &OperationContext,
        ) -> Result<Checkpoint, RepositoryError> {
            scope.validate_input(&scope.input)?;
            Ok(self.database.borrow().checkpoint.clone())
        }
    }
    struct Engine {
        fixture: Fixture,
        source: EndStateSource,
        database: Rc<RefCell<Database>>,
    }
    impl SessionEngine<Scope> for Engine {
        fn decide(
            &mut self,
            current: &Checkpoint,
            scope: &Scope,
            input: &GameInput,
        ) -> Result<Checkpoint, RepositoryError> {
            scope.validate_input(input)?;
            self.database.borrow_mut().decisions += 1;
            if self.database.borrow().failure == Failure::SourceWithdrawn {
                self.source
                    .source
                    .refusal
                    .set(Some(SourceRefusal::Withdrawn));
            }
            if self.database.borrow().failure == Failure::DeniedTail {
                self.source.source.refused_index.set(Some(1));
            }
            if self.database.borrow().failure == Failure::DeniedEndState {
                self.source
                    .refusal
                    .set(Some(ObjectivePolicyError::RightsDenied));
            }
            let before = self.source.calls.get();
            let result = registered_end_state(&self.fixture, &self.source, current, input)
                .map_err(|_| RepositoryError::InvalidCandidate);
            self.database.borrow_mut().end_state_admissions += self.source.calls.get() - before;
            result
        }
        fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
            current
                .validate_resume(current.basis(), &self.fixture.pins)
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
            scope: &Scope,
            checkpoint: &Checkpoint,
        ) -> Result<(), DeliveryError> {
            let mut db = self.database.borrow_mut();
            assert_eq!(checkpoint, &db.checkpoint);
            assert!(db.ledger.iter().any(|(prior, receipt)| prior == scope && receipt.basis() == checkpoint.basis()));
            assert_eq!(checkpoint.state().encounters[0].objectives, db.replacements);
            db.publications += 1;
            if db.failure == Failure::Publication {
                return Err(DeliveryError::Unavailable);
            }
            Ok(())
        }
        fn wake_committed_intents(&mut self, scope: &Scope) -> Result<(), DeliveryError> {
            let mut db = self.database.borrow_mut();
            assert!(db.ledger.iter().any(|(prior, _)| prior == scope));
            assert!(db.publications > 0);
            assert!(db.checkpoint.state().intents.is_empty());
            db.wakes += 1;
            Ok(())
        }
    }
    type Owner = DurableOwner<Repository, Engine, Publication>;
    fn setup(failure: Failure) -> (Owner, Rc<RefCell<Database>>, Scope) {
        setup_with_fixture(failure, Fixture::new(), false)
    }

    fn setup_with_fixture(
        failure: Failure,
        fixture: Fixture,
        terminal: bool,
    ) -> (Owner, Rc<RefCell<Database>>, Scope) {
        let current = fixture.current();
        let scope = Scope {
            input: fixture.input(current.basis()),
            principal: fixture::member(3),
        };
        let source = EndStateSource::new(&fixture, terminal);
        let db = Rc::new(RefCell::new(Database {
            checkpoint: current.clone(),
            replacements: fixture.replacements.clone(),
            end_state_admissions: 0,
            ledger: vec![],
            failure,
            commits: 0,
            decisions: 0,
            publications: 0,
            wakes: 0,
        }));
        let owner = DurableOwner::new(
            Repository {
                database: Rc::clone(&db),
            },
            Engine {
                fixture,
                source,
                database: Rc::clone(&db),
            },
            Publication {
                database: Rc::clone(&db),
            },
            current,
            64 * 1024,
        )
        .unwrap();
        (owner, db, scope)
    }
    fn submit(owner: &mut Owner, scope: &Scope) -> SubmissionOutcome {
        let (input, wait) = OwnedInput::new(
            OperationContext {
                trace_parent: String::new(),
                build: "objective-fixture".to_owned(),
            },
            scope.clone(),
            scope.input.clone(),
        );
        owner.reduce(AdmissionSequence(1), input);
        wait.try_recv().unwrap()
    }
    fn counts(db: &Database) -> (usize, usize, usize, usize, usize) {
        (
            db.decisions,
            db.commits,
            db.publications,
            db.wakes,
            db.ledger.len(),
        )
    }
    #[test]
    fn atomic_two_objective_ack_publishes_once_and_exact_retry_returns_original_receipt() {
        let (mut owner, db, scope) = setup(Failure::None);
        let original = owner.checkpoint().clone();
        let receipt = submit(&mut owner, &scope);
        assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
        db.borrow_mut().failure = Failure::SourceWithdrawn;
        assert_eq!(submit(&mut owner, &scope), receipt);
        let db = db.borrow();
        assert_preserved(&Fixture::new(), &original, &db.checkpoint);
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_eq!(counts(&db), (1, 1, 1, 1, 1));
    }
    #[test]
    fn precommit_rollback_preserves_both_objectives_and_safe_retry_applies_one_whole_batch() {
        let (mut owner, db, scope) = setup(Failure::BeforeCommit);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::Unavailable)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.borrow().checkpoint, original);
        assert_eq!(counts(&db.borrow()), (1, 1, 0, 0, 0));
        db.borrow_mut().failure = Failure::None;
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        assert_preserved(&Fixture::new(), &original, &db.borrow().checkpoint);
        assert_eq!(counts(&db.borrow()), (2, 2, 1, 1, 1));
    }
    #[test]
    fn unknown_absent_commit_blocks_reexecution_publication_and_unrelated_operations() {
        let (mut owner, db, scope) = setup(Failure::UnknownNotCommitted);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        db.borrow_mut().failure = Failure::None;
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
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
        assert!(owner.has_uncertain_operation());
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.borrow().checkpoint, original);
        assert_eq!(counts(&db.borrow()), (1, 1, 0, 0, 0));
    }
    #[test]
    fn lost_ack_reloads_exact_committed_objectives_without_restage_or_early_publication() {
        let (mut owner, db, scope) = setup(Failure::LostAck);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_preserved(&Fixture::new(), &original, &db.borrow().checkpoint);
        assert_eq!(counts(&db.borrow()), (1, 1, 0, 0, 1));
        db.borrow_mut().failure = Failure::SourceWithdrawn;
        let receipt = submit(&mut owner, &scope);
        assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
        assert!(owner.is_current());
        assert_eq!(owner.checkpoint(), &db.borrow().checkpoint);
        assert_eq!(submit(&mut owner, &scope), receipt);
        assert_eq!(counts(&db.borrow()), (1, 1, 0, 0, 1));
    }
    #[test]
    fn stale_fence_and_revision_conflict_refuse_complete_candidate_and_fence_the_cache() {
        for (failure, error) in [
            (Failure::StaleFence, RepositoryError::StaleFence),
            (Failure::RevisionConflict, RepositoryError::RevisionConflict),
        ] {
            let (mut owner, db, scope) = setup(failure);
            let original = owner.checkpoint().clone();
            assert_eq!(
                submit(&mut owner, &scope),
                SubmissionOutcome::Refused(error)
            );
            assert_eq!(owner.checkpoint(), &original);
            assert_eq!(db.borrow().checkpoint, original);
            db.borrow_mut().failure = Failure::None;
            assert_eq!(
                submit(&mut owner, &scope),
                SubmissionOutcome::LookupRequired
            );
            assert_eq!(counts(&db.borrow()), (1, 1, 0, 0, 0));
        }
    }
    #[test]
    fn source_and_tail_refusal_reach_no_repository_commit_publication_or_wake() {
        for failure in [Failure::SourceWithdrawn, Failure::DeniedTail] {
            let (mut owner, db, scope) = setup(failure);
            let original = owner.checkpoint().clone();
            assert_eq!(
                submit(&mut owner, &scope),
                SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
            );
            assert_eq!(owner.checkpoint(), &original);
            assert_eq!(db.borrow().checkpoint, original);
            assert_eq!(counts(&db.borrow()), (1, 0, 0, 0, 0));
        }
    }
    #[test]
    fn publication_failure_retains_durable_ack_and_does_not_repeat_objective_application() {
        let (mut owner, db, scope) = setup(Failure::Publication);
        let original = owner.checkpoint().clone();
        let receipt = submit(&mut owner, &scope);
        assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
        assert_eq!(submit(&mut owner, &scope), receipt);
        assert_preserved(&Fixture::new(), &original, &db.borrow().checkpoint);
        assert_eq!(owner.checkpoint(), &db.borrow().checkpoint);
        assert_eq!(counts(&db.borrow()), (1, 1, 1, 1, 1));
    }
    #[test]
    fn principal_and_exact_operation_input_conflicts_cannot_restage_an_objective_batch() {
        let (mut owner, db, scope) = setup(Failure::None);
        let original = owner.checkpoint().clone();
        let mut forged = scope.clone();
        forged.principal = fixture::member(9);
        assert_eq!(
            submit(&mut owner, &forged),
            SubmissionOutcome::Refused(RepositoryError::Unauthorized)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(counts(&db.borrow()), (0, 0, 0, 0, 0));
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        let mut conflict = scope.clone();
        let GameInput::Game(command) = &mut conflict.input else {
            unreachable!()
        };
        command.command = GameCommand::Speak {
            speaker: fixture::entity(4),
            text: "different exact operation input".to_owned(),
            conversation: None,
        };
        assert_eq!(
            submit(&mut owner, &conflict),
            SubmissionOutcome::OperationConflict
        );
        assert_eq!(counts(&db.borrow()), (1, 1, 1, 1, 1));
    }
    #[test]
    fn durable_terminal_success_failure_escape_commit_once_and_exact_retry_never_restages() {
        for outcome in ["reviewed-success", "reviewed-failure", "reviewed-escape"] {
            let fixture = end_state_fixture(outcome);
            let (mut owner, db, scope) =
                setup_with_fixture(Failure::None, end_state_fixture(outcome), true);
            let original = owner.checkpoint().clone();
            let receipt = submit(&mut owner, &scope);
            assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
            assert_end_state_preserved(&fixture, &original, owner.checkpoint());
            db.borrow_mut().failure = Failure::DeniedEndState;
            assert_eq!(submit(&mut owner, &scope), receipt);
            let db = db.borrow();
            assert_eq!(counts(&db), (1, 1, 1, 1, 1));
            assert_eq!(db.end_state_admissions, 1);
            assert_eq!(owner.checkpoint(), &db.checkpoint);
        }
    }

    #[test]
    fn terminal_lost_ack_reloads_both_objectives_and_cleared_turns_without_reexecution() {
        let fixture = end_state_fixture("reviewed-escape");
        let (mut owner, db, scope) =
            setup_with_fixture(Failure::LostAck, end_state_fixture("reviewed-escape"), true);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_end_state_preserved(&fixture, &original, &db.borrow().checkpoint);
        assert_eq!(counts(&db.borrow()), (1, 1, 0, 0, 1));
        db.borrow_mut().failure = Failure::DeniedEndState;
        let receipt = submit(&mut owner, &scope);
        assert!(matches!(receipt, SubmissionOutcome::Confirmed(_)));
        assert_eq!(submit(&mut owner, &scope), receipt);
        assert_end_state_preserved(&fixture, &original, owner.checkpoint());
        assert_eq!(counts(&db.borrow()), (1, 1, 0, 0, 1));
        assert_eq!(db.borrow().end_state_admissions, 1);
    }

    #[test]
    fn terminal_precommit_rollback_preserves_turns_and_retry_commits_the_whole_candidate() {
        let fixture = end_state_fixture("reviewed-failure");
        let (mut owner, db, scope) = setup_with_fixture(
            Failure::BeforeCommit,
            end_state_fixture("reviewed-failure"),
            true,
        );
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::Unavailable)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.borrow().checkpoint, original);
        assert_eq!(counts(&db.borrow()), (1, 1, 0, 0, 0));
        db.borrow_mut().failure = Failure::None;
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        assert_end_state_preserved(&fixture, &original, owner.checkpoint());
        assert_eq!(counts(&db.borrow()), (2, 2, 1, 1, 1));
        assert_eq!(db.borrow().end_state_admissions, 2);
    }

    #[test]
    fn terminal_admission_denial_reaches_no_commit_publication_or_wake() {
        let (mut owner, db, scope) = setup_with_fixture(
            Failure::DeniedEndState,
            end_state_fixture("reviewed-success"),
            true,
        );
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(db.borrow().checkpoint, original);
        assert_eq!(counts(&db.borrow()), (1, 0, 0, 0, 0));
        assert_eq!(db.borrow().end_state_admissions, 1);
    }
}
