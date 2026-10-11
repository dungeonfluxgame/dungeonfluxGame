#[path = "support/pending_fixtures.rs"]
mod pending_fixtures;

use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::command_entry::{CommandEntryContext, CommandEntryLimits, CommandRejection};
use df_engine::pending_resumption::{ResumeError, ResumeFence, ResumeLimits};
use df_engine::semantic_candidate::{
    DialogueResponseAdmission, DialogueResponseError, DialogueResponseOutcome,
    SemanticResponseAdmission, SemanticResponseError, stage_dialogue_response,
    stage_semantic_response,
};
use df_intent::candidate::{CandidateError, CandidateOwner};
use df_intent::dialogue::{DialogueError, DialogueLimits};
use df_model::checkpoint::*;
use df_model::commands::{CommandError, CommandLimits};
use df_model::intent::{DiscourseContext, InputFinality, IntentDisposition, RawInputRef};
use df_rules::preconditions::{
    CurrentRuleContext, PreconditionError, PreconditionLimits, PreconditionedCommandHandler,
    PreconditionedRejection, RuleDependency, RulePreconditions,
};
use df_rules::{DispatchRegistry, HandlerRegistration, InvocationError, RulesCommandInput};
use df_types::{MemberId, OperationId, RevisionLabel};
use pending_fixtures::*;
use std::cell::Cell;
use std::rc::Rc;

const BYTES: usize = 1024 * 1024;

struct Admission {
    member: MemberId,
    operation: OperationId,
    basis: Basis,
    pins: CheckpointPins,
    rules: Vec<RuleReference>,
    selector: RevisionLabel,
    fence: ResumeFence,
    command: CommandLimits,
    maximum_bytes: usize,
}

fn admission(current: &Checkpoint) -> Admission {
    Admission {
        member: member(3),
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        basis: current.basis(),
        pins: current.pins().clone(),
        rules: vec![rule()],
        selector: label("fixture-continuation-position-1"),
        fence: fence(),
        command: command_limits(),
        maximum_bytes: BYTES,
    }
}

fn handler(current: &Checkpoint, input: RulesCommandInput<'_>) -> SuppliedHandler {
    SuppliedHandler {
        candidate: supplied_candidate(current, input),
        pins: current.pins().clone(),
        calls: Rc::new(Cell::new(0)),
        refuse: false,
        observed_draw_pointer: Rc::new(Cell::new(std::ptr::null())),
    }
}

// Synthetic source-controlled registry/dependency fixture; no D&D mechanic is implemented.
fn invoke_semantic(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    prepared: &Checkpoint,
    handler: &SuppliedHandler,
    admission: Admission,
) -> Result<Checkpoint, SemanticResponseError<FixtureRejection>> {
    let contents = [content()];
    let resources = resource_constraints();
    let dependencies = [
        RuleDependency::Resource {
            owner: entity(4),
            resource: label("fixture-resource-1"),
        },
        RuleDependency::PendingResolutions,
    ];
    let source = rule();
    let registered_selector = label("fixture-continuation-position-1");
    let inventory = || ReferenceInventory {
        rules: &admission.rules,
        content: &contents,
        resources: &resources,
        assets: &[],
    };
    let wrapper = PreconditionedCommandHandler::new(
        handler,
        &source,
        CurrentRuleContext {
            checkpoint: current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: inventory(),
            command_limits: admission.command,
        },
        RulePreconditions {
            prepared,
            sources: &admission.rules,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 8,
            maximum_comparisons: BYTES,
            maximum_checkpoint_bytes: BYTES,
        },
    );
    let entries = [CatalogEntry::new(&source, b"synthetic-clause")];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-publication",
        &entries,
        CatalogLimits {
            max_complete_bytes: 128,
            max_entries: 4,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let registrations = [HandlerRegistration::new(
        &registered_selector,
        &source,
        &wrapper,
    )];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    stage_semantic_response(
        input,
        current,
        CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory: inventory(),
            limits: CommandEntryLimits {
                command: admission.command,
                maximum_staged_bytes: admission.maximum_bytes,
            },
        },
        SemanticResponseAdmission {
            owner: CandidateOwner {
                basis: admission.basis,
                pins: &admission.pins,
                member: admission.member,
                operation: admission.operation,
            },
            fence: admission.fence,
            limits: ResumeLimits {
                maximum_checkpoint_bytes: admission.maximum_bytes,
            },
        },
        &registry,
        &admission.selector,
    )
}

#[test]
fn offered_choice_reaction_and_roll_use_existing_registered_entry_without_mutation() {
    for kind in 0..3 {
        let current = waiting(kind);
        let original = current.clone();
        let input = response(kind);
        let draws = if kind == 2 {
            vec![native_roll_draw(0)]
        } else {
            vec![]
        };
        let envelope = RulesCommandInput {
            command: &input,
            supplied_draws: &draws,
        };
        let handler = handler(&current, envelope);
        let staged =
            invoke_semantic(envelope, &current, &current, &handler, admission(&current)).unwrap();
        assert_eq!(staged, handler.candidate);
        assert_eq!(handler.calls.get(), 1);
        assert!(std::ptr::eq(
            handler.observed_draw_pointer.get(),
            draws.as_ptr()
        ));
        assert_eq!(current, original);
        assert_eq!(staged.state().resources, current.state().resources);
        assert_eq!(staged.state().draws, draws);
        assert_eq!(
            staged.state().pending[0].spent,
            current.state().pending[0].spent
        );
        assert_eq!(
            invoke(envelope, &current, &current, &handler, fence(), BYTES).unwrap(),
            staged
        );
    }
}

#[test]
fn trusted_member_operation_basis_and_pins_are_independent_of_suggestion() {
    let current = waiting(0);
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let handler = handler(&current, envelope);
    let mut foreign_member = admission(&current);
    foreign_member.member = member(99);
    let mut foreign_operation = admission(&current);
    foreign_operation.operation = OperationId::from_bytes(&[99; 16]).unwrap();
    let mut stale_basis = admission(&current);
    stale_basis.basis.revision = stale_basis.basis.revision.next_sequence().unwrap();
    let mut stale_pins = admission(&current);
    stale_pins.pins.content.content = label("different-content");
    for (owner, expected) in [
        (foreign_member, CandidateError::MemberMismatch),
        (foreign_operation, CandidateError::OperationMismatch),
        (
            stale_basis,
            CandidateError::Snapshot(CheckpointError::StaleBasis),
        ),
        (
            stale_pins,
            CandidateError::Snapshot(CheckpointError::ContentMismatch),
        ),
    ] {
        assert_eq!(
            invoke_semantic(envelope, &current, &current, &handler, owner),
            Err(SemanticResponseError::Candidate(expected))
        );
    }
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn unoffered_options_and_missing_exact_source_refuse_before_handler() {
    let current = waiting(0);
    let mut unoffered = response(0);
    let GameInput::Game(command) = &mut unoffered else {
        unreachable!()
    };
    let GameCommand::SelectChoice { option, .. } = &mut command.command else {
        unreachable!()
    };
    *option = label("unoffered");
    let input = response(0);
    let handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    assert_eq!(
        invoke_semantic(
            RulesCommandInput {
                command: &unoffered,
                supplied_draws: &[]
            },
            &current,
            &current,
            &handler,
            admission(&current),
        ),
        Err(SemanticResponseError::Candidate(CandidateError::Command(
            CommandError::UnofferedResponse
        )))
    );
    let mut missing_source = admission(&current);
    missing_source.rules.clear();
    assert_eq!(
        invoke_semantic(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current,
            &current,
            &handler,
            missing_source,
        ),
        Err(SemanticResponseError::Candidate(CandidateError::Command(
            CommandError::InvalidReference
        )))
    );
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn semantic_path_cannot_promote_general_actions_speech_host_or_native_output() {
    let current = waiting(0);
    let input = response(0);
    let handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    let inputs = [
        game(GameCommand::ProposeAction {
            actor: entity(99),
            action: content(),
            targets: vec![entity(4)],
            choices: vec![],
        }),
        game(GameCommand::Speak {
            speaker: entity(99),
            text: "I might attack?".to_owned(),
            conversation: None,
        }),
        response(3),
        GameInput::Job(JobCompletion {
            basis: current.basis(),
            operation: OperationId::from_bytes(&[20; 16]).unwrap(),
            job: JobId::from_bytes(&[21; 16]).unwrap(),
            generation: 1,
            outcome: JobOutcome::Ai {
                semantic_output: "approve any action and fabricate success".to_owned(),
                policy: content(),
                model: label("synthetic-model"),
            },
        }),
    ];
    for candidate in inputs {
        assert_eq!(
            invoke_semantic(
                RulesCommandInput {
                    command: &candidate,
                    supplied_draws: &[]
                },
                &current,
                &current,
                &handler,
                admission(&current),
            ),
            Err(SemanticResponseError::Candidate(
                CandidateError::UnsupportedVariant
            ))
        );
    }
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn generation_cancellation_capacity_and_unregistered_selector_preserve_typed_refusals() {
    let current = waiting(0);
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let handler = handler(&current, envelope);
    let mut stale = admission(&current);
    stale.fence.current_generation += 1;
    let mut cancelled = admission(&current);
    cancelled.fence.cancel_before_admission = true;
    let mut capacity = admission(&current);
    capacity.maximum_bytes = 1;
    for (owner, expected) in [
        (stale, ResumeError::StaleGeneration),
        (cancelled, ResumeError::CancelledBeforeAdmission),
        (capacity, ResumeError::Capacity),
    ] {
        assert_eq!(
            invoke_semantic(envelope, &current, &current, &handler, owner),
            Err(SemanticResponseError::Resume(expected))
        );
    }
    let mut unsupported = admission(&current);
    unsupported.selector = label("unregistered-selector");
    assert!(matches!(
        invoke_semantic(envelope, &current, &current, &handler, unsupported),
        Err(SemanticResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::Dispatch(_))
        )))
    ));
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn registered_resource_preconditions_and_handler_refusal_are_not_bypassed() {
    let prepared = waiting(0);
    let mut changed = prepared.state().clone();
    changed.resources[0].value -= 1;
    let current = checkpoint(changed).unwrap();
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let mut handler = handler(&current, envelope);
    assert_eq!(
        invoke_semantic(envelope, &current, &prepared, &handler, admission(&current)),
        Err(SemanticResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Precondition(PreconditionError::StaleResource)
            ))
        )))
    );
    assert_eq!(handler.calls.get(), 0);
    handler.refuse = true;
    assert_eq!(
        invoke_semantic(envelope, &current, &current, &handler, admission(&current)),
        Err(SemanticResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Handler(FixtureRejection::Unsupported)
            ))
        )))
    );
    assert_eq!(handler.calls.get(), 1);
}

#[test]
fn explicit_native_draws_are_checked_without_generation_substitution_or_replay() {
    let current = waiting(2);
    let input = response(2);
    let draws = [native_roll_draw(0)];
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &draws,
    };
    let handler = handler(&current, envelope);
    let mut foreign_operation = draws.to_vec();
    foreign_operation[0].operation = OperationId::from_bytes(&[99; 16]).unwrap();
    let mut invalid_value = draws.to_vec();
    invalid_value[0].value = 0;
    let mut foreign_window = draws.to_vec();
    foreign_window[0].window = WindowId::from_bytes(&[99; 16]).unwrap();
    for (supplied, expected) in [
        (&[][..], InvocationError::RollInputMismatch),
        (
            foreign_operation.as_slice(),
            InvocationError::DrawOperationMismatch,
        ),
        (invalid_value.as_slice(), InvocationError::InvalidDrawInput),
        (
            foreign_window.as_slice(),
            InvocationError::RollInputMismatch,
        ),
    ] {
        assert_eq!(
            invoke_semantic(
                RulesCommandInput {
                    command: &input,
                    supplied_draws: supplied
                },
                &current,
                &current,
                &handler,
                admission(&current),
            ),
            Err(SemanticResponseError::Resume(ResumeError::Command(
                CommandRejection::Invocation(expected)
            )))
        );
    }
    assert_eq!(handler.calls.get(), 0);
    let staged =
        invoke_semantic(envelope, &current, &current, &handler, admission(&current)).unwrap();
    assert!(std::ptr::eq(
        handler.observed_draw_pointer.get(),
        draws.as_ptr()
    ));
    assert_eq!(handler.calls.get(), 1);
    assert_eq!(
        invoke_semantic(envelope, &staged, &staged, &handler, admission(&staged)),
        Err(SemanticResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::AlreadyAccepted)
        )))
    );
    assert_eq!(handler.calls.get(), 1);
}

#[cfg(not(target_arch = "wasm32"))]
mod durable_session {
    use super::*;
    use df_observe::OperationContext;
    use df_session::inbox::{ActorInput, AdmissionSequence, Reducer};
    use df_session::submission::*;
    use df_types::SessionId;
    use std::cell::RefCell;

    // Controlled native repository/ledger boundary; actual PostgreSQL qualification is separate.
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
        engines: usize,
        handlers: usize,
        commits: usize,
        publications: usize,
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
            if scope.principal != member(3) || scope.session() != basis().session {
                return Err(RepositoryError::Unauthorized);
            }
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
            Ok(self.database.borrow().checkpoint.clone())
        }
    }

    struct Engine {
        database: Rc<RefCell<Database>>,
    }

    impl SessionEngine<Scope> for Engine {
        fn decide(
            &mut self,
            current: &Checkpoint,
            scope: &Scope,
            input: &GameInput,
        ) -> Result<Checkpoint, RepositoryError> {
            self.database.borrow_mut().engines += 1;
            let envelope = RulesCommandInput {
                command: input,
                supplied_draws: &[],
            };
            let handler = handler(current, envelope);
            let mut trusted = admission(current);
            trusted.member = scope.principal;
            trusted.operation = scope.operation();
            let result = invoke_semantic(envelope, current, current, &handler, trusted);
            self.database.borrow_mut().handlers += handler.calls.get();
            result.map_err(|_| RepositoryError::InvalidCandidate)
        }

        fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
            current
                .validate_resume(current.basis(), &pins())
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
            assert!(db.ledger.iter().any(|(committed, _)| committed == scope));
            db.publications += 1;
            Ok(())
        }

        fn wake_committed_intents(&mut self, _: &Scope) -> Result<(), DeliveryError> {
            Ok(())
        }
    }

    type Owner = DurableOwner<Repository, Engine, Publication>;

    fn setup(failure: Failure) -> (Owner, Rc<RefCell<Database>>, Scope) {
        let current = waiting(0);
        let scope = Scope {
            input: response(0),
            principal: member(3),
        };
        let database = Rc::new(RefCell::new(Database {
            checkpoint: current.clone(),
            ledger: vec![],
            failure,
            engines: 0,
            handlers: 0,
            commits: 0,
            publications: 0,
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

    fn submit(owner: &mut Owner, scope: &Scope) -> SubmissionOutcome {
        let (input, wait) = OwnedInput::new(
            OperationContext {
                trace_parent: String::new(),
                build: "semantic-pending-fixture".to_owned(),
            },
            scope.clone(),
            scope.input.clone(),
        );
        owner.reduce(AdmissionSequence(1), input);
        wait.try_recv().unwrap()
    }

    #[test]
    fn durable_owner_calls_registered_semantic_engine_once_and_retries_exact_receipt() {
        let (mut owner, database, scope) = setup(Failure::None);
        let original = owner.checkpoint().clone();
        let first = submit(&mut owner, &scope);
        assert!(matches!(first, SubmissionOutcome::Confirmed(_)));
        assert_eq!(submit(&mut owner, &scope), first);
        let db = database.borrow();
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_eq!(
            db.checkpoint.basis().revision,
            original.basis().revision.next_sequence().unwrap()
        );
        assert_eq!(db.checkpoint.state().resources, original.state().resources);
        assert_eq!(db.checkpoint.state().facts, original.state().facts);
        assert_eq!(db.checkpoint.state().decisions.len(), 1);
        assert_eq!(
            (
                db.engines,
                db.handlers,
                db.commits,
                db.publications,
                db.ledger.len()
            ),
            (1, 1, 1, 1, 1)
        );
    }

    #[test]
    fn semantic_or_principal_refusal_never_calls_handler_commits_or_publishes() {
        for refusal in 0..3 {
            let (mut owner, database, mut scope) = setup(Failure::None);
            let original = owner.checkpoint().clone();
            if refusal == 0 {
                scope.principal = member(99);
            } else {
                let GameInput::Game(command) = &mut scope.input else {
                    unreachable!()
                };
                if refusal == 1 {
                    command.member = member(99);
                } else {
                    command.command = GameCommand::ProposeAction {
                        actor: entity(99),
                        action: content(),
                        targets: vec![entity(4)],
                        choices: vec![],
                    };
                }
            }
            assert_eq!(
                submit(&mut owner, &scope),
                SubmissionOutcome::Refused(if refusal == 0 {
                    RepositoryError::Unauthorized
                } else {
                    RepositoryError::InvalidCandidate
                })
            );
            let db = database.borrow();
            assert_eq!(owner.checkpoint(), &original);
            assert_eq!(db.checkpoint, original);
            assert_eq!(
                (db.handlers, db.commits, db.publications, db.ledger.len()),
                (0, 0, 0, 0)
            );
        }
    }

    #[test]
    fn known_commit_failure_keeps_original_checkpoint_and_allows_exact_retry() {
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
            (
                db.engines,
                db.handlers,
                db.commits,
                db.publications,
                db.ledger.len()
            ),
            (2, 2, 2, 1, 1)
        );
    }

    #[test]
    fn lost_commit_ack_uses_ledger_lookup_without_reexecuting_semantic_handler() {
        let (mut owner, database, scope) = setup(Failure::LostAck);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::LookupRequired
        );
        assert!(owner.has_uncertain_operation());
        assert_eq!(owner.checkpoint(), &original);
        assert_eq!(database.borrow().publications, 0);
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        let db = database.borrow();
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_eq!(
            (
                db.engines,
                db.handlers,
                db.commits,
                db.publications,
                db.ledger.len()
            ),
            (1, 1, 1, 0, 1)
        );
    }

    #[test]
    fn unknown_unrecorded_commit_never_reexecutes_and_conflicting_receipt_is_refused() {
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
        assert_eq!(owner.checkpoint(), &original);
        let db = database.borrow();
        assert_eq!(
            (
                db.engines,
                db.handlers,
                db.commits,
                db.publications,
                db.ledger.len()
            ),
            (1, 1, 1, 0, 0)
        );
        drop(db);

        let (mut owner, database, mut scope) = setup(Failure::None);
        assert!(matches!(
            submit(&mut owner, &scope),
            SubmissionOutcome::Confirmed(_)
        ));
        let GameInput::Game(command) = &mut scope.input else {
            unreachable!()
        };
        command.command = GameCommand::Speak {
            speaker: entity(4),
            text: "different exact input".to_owned(),
            conversation: None,
        };
        assert_eq!(
            submit(&mut owner, &scope),
            SubmissionOutcome::OperationConflict
        );
        let db = database.borrow();
        assert_eq!(
            (
                db.engines,
                db.handlers,
                db.commits,
                db.publications,
                db.ledger.len()
            ),
            (1, 1, 1, 1, 1)
        );
    }
}

fn invoke_dialogue(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    prepared: &Checkpoint,
    handler: &SuppliedHandler,
    admission: Admission,
    dialogue: DialogueResponseAdmission<'_>,
) -> Result<DialogueResponseOutcome, DialogueResponseError<FixtureRejection>> {
    let contents = [content()];
    let resources = resource_constraints();
    let dependencies = [
        RuleDependency::Resource {
            owner: entity(4),
            resource: label("fixture-resource-1"),
        },
        RuleDependency::PendingResolutions,
    ];
    let source = rule();
    let registered_selector = label("fixture-continuation-position-1");
    let inventory = || ReferenceInventory {
        rules: &admission.rules,
        content: &contents,
        resources: &resources,
        assets: &[],
    };
    let wrapper = PreconditionedCommandHandler::new(
        handler,
        &source,
        CurrentRuleContext {
            checkpoint: current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: inventory(),
            command_limits: admission.command,
        },
        RulePreconditions {
            prepared,
            sources: &admission.rules,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 8,
            maximum_comparisons: BYTES,
            maximum_checkpoint_bytes: BYTES,
        },
    );
    let entries = [CatalogEntry::new(&source, b"synthetic-clause")];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-publication",
        &entries,
        CatalogLimits {
            max_complete_bytes: 128,
            max_entries: 4,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let registrations = [HandlerRegistration::new(
        &registered_selector,
        &source,
        &wrapper,
    )];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    stage_dialogue_response(
        input,
        current,
        CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory: inventory(),
            limits: CommandEntryLimits {
                command: admission.command,
                maximum_staged_bytes: admission.maximum_bytes,
            },
        },
        dialogue,
        SemanticResponseAdmission {
            owner: CandidateOwner {
                basis: admission.basis,
                pins: &admission.pins,
                member: admission.member,
                operation: admission.operation,
            },
            fence: admission.fence,
            limits: ResumeLimits {
                maximum_checkpoint_bytes: admission.maximum_bytes,
            },
        },
        &registry,
        &admission.selector,
    )
}

fn captured_dialogue(current: &Checkpoint) -> RawInputRef {
    RawInputRef {
        input: OperationId::from_bytes(&[30; 16]).unwrap(),
        member: member(3),
        basis: current.basis(),
        pins: current.pins().clone(),
        finality: InputFinality::Final,
    }
}

fn dialogue_admission<'a>(
    captured: &'a RawInputRef,
    confirmed: Option<&'a RawInputRef>,
    discourse: DiscourseContext,
    text: &'a str,
) -> DialogueResponseAdmission<'a> {
    DialogueResponseAdmission {
        captured,
        confirmed,
        discourse,
        text,
        limits: DialogueLimits {
            maximum_text_bytes: 64,
        },
    }
}

#[test]
fn nonaction_discourse_and_partials_never_enter_registered_mechanics() {
    let current = waiting(2);
    let original = current.clone();
    let input = response(2);
    let draws = [native_roll_draw(0)];
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &draws,
    };
    let handler = handler(&current, envelope);
    for (discourse, expected) in [
        (DiscourseContext::Question, IntentDisposition::Question),
        (DiscourseContext::Social, IntentDisposition::Social),
        (DiscourseContext::PlanOnly, IntentDisposition::PlanOnly),
        (DiscourseContext::Meta, IntentDisposition::Meta),
        (DiscourseContext::Joke, IntentDisposition::Joke),
        (DiscourseContext::Clarify, IntentDisposition::Clarify),
        (DiscourseContext::Unspecified, IntentDisposition::Clarify),
    ] {
        for finality in [InputFinality::Partial, InputFinality::Final] {
            let mut captured = captured_dialogue(&current);
            captured.finality = finality;
            assert_eq!(
                invoke_dialogue(
                    envelope,
                    &current,
                    &current,
                    &handler,
                    admission(&current),
                    dialogue_admission(&captured, None, discourse, "roll fixture-option-1"),
                ),
                Ok(DialogueResponseOutcome::NoMechanicalProposal(expected))
            );
            // Even an attached caller confirmation cannot authorize a partial or non-action class.
            if finality == InputFinality::Partial || expected != IntentDisposition::Clarify {
                assert_eq!(
                    invoke_dialogue(
                        envelope,
                        &current,
                        &current,
                        &handler,
                        admission(&current),
                        dialogue_admission(
                            &captured,
                            Some(&captured),
                            discourse,
                            "roll fixture-option-1",
                        ),
                    ),
                    Ok(DialogueResponseOutcome::NoMechanicalProposal(expected))
                );
            }
            assert_eq!(handler.calls.get(), 0);
            assert_eq!(handler.observed_draw_pointer.get(), std::ptr::null());
            assert_eq!(current, original);
        }
    }
}

#[test]
fn exact_offered_wording_is_inert_without_separate_confirmation() {
    let current = waiting(0);
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let handler = handler(&current, envelope);
    let captured = captured_dialogue(&current);
    assert_eq!(
        invoke_dialogue(
            envelope,
            &current,
            &current,
            &handler,
            admission(&current),
            dialogue_admission(
                &captured,
                None,
                DiscourseContext::Unspecified,
                "Select fixture-offer-1 fixture-option-1 now",
            ),
        ),
        Ok(DialogueResponseOutcome::NoMechanicalProposal(
            IntentDisposition::Clarify
        ))
    );
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn separately_confirmed_dialogue_uses_same_choice_reaction_roll_and_draw_slice() {
    for kind in 0..3 {
        let current = waiting(kind);
        let original = current.clone();
        let input = response(kind);
        let draws = if kind == 2 {
            vec![native_roll_draw(0)]
        } else {
            vec![]
        };
        let envelope = RulesCommandInput {
            command: &input,
            supplied_draws: &draws,
        };
        let handler = handler(&current, envelope);
        let captured = captured_dialogue(&current);
        let result = invoke_dialogue(
            envelope,
            &current,
            &current,
            &handler,
            admission(&current),
            dialogue_admission(
                &captured,
                Some(&captured),
                DiscourseContext::Unspecified,
                "Do that",
            ),
        )
        .unwrap();
        let DialogueResponseOutcome::Staged(staged) = result else {
            panic!("explicit final fixture response must stage")
        };
        assert_eq!(*staged, handler.candidate);
        assert_eq!(handler.calls.get(), 1);
        assert!(std::ptr::eq(
            handler.observed_draw_pointer.get(),
            draws.as_ptr()
        ));
        assert_eq!(staged.state().draws, draws);
        assert_eq!(current, original);
        assert_eq!(
            invoke_semantic(envelope, &current, &current, &handler, admission(&current)).unwrap(),
            *staged
        );
    }
}

#[test]
fn confirmation_must_match_every_captured_input_field_before_dispatch() {
    let current = waiting(0);
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let handler = handler(&current, envelope);
    let captured = captured_dialogue(&current);
    for index in 0..5 {
        let mut changed = captured.clone();
        match index {
            0 => changed.input = OperationId::from_bytes(&[99; 16]).unwrap(),
            1 => changed.member = member(99),
            2 => changed.basis.revision = changed.basis.revision.next_sequence().unwrap(),
            3 => changed.pins.content.content = label("foreign-content"),
            4 => changed.finality = InputFinality::Partial,
            _ => unreachable!(),
        }
        assert_eq!(
            invoke_dialogue(
                envelope,
                &current,
                &current,
                &handler,
                admission(&current),
                dialogue_admission(
                    &captured,
                    Some(&changed),
                    DiscourseContext::Unspecified,
                    "Do that",
                ),
            ),
            Err(DialogueResponseError::ConfirmationMismatch)
        );
    }
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn confirmed_dialogue_keeps_exact_current_owner_and_distinct_operation_bindings() {
    let current = waiting(0);
    let original = current.clone();
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let handler = handler(&current, envelope);
    let captured = captured_dialogue(&current);
    for index in 0..5 {
        let mut owner = admission(&current);
        let expected = match index {
            0 => {
                owner.member = member(99);
                DialogueResponseError::Dialogue(DialogueError::MemberMismatch)
            }
            1 => {
                owner.operation = OperationId::from_bytes(&[99; 16]).unwrap();
                DialogueResponseError::Semantic(SemanticResponseError::Candidate(
                    CandidateError::OperationMismatch,
                ))
            }
            2 => {
                owner.basis.revision = owner.basis.revision.next_sequence().unwrap();
                DialogueResponseError::InputBindingMismatch
            }
            3 => {
                owner.pins.rules.handler = label("foreign-handler");
                DialogueResponseError::InputBindingMismatch
            }
            4 => {
                owner.operation = captured.input;
                DialogueResponseError::ConfirmationOperationReuse
            }
            _ => unreachable!(),
        };
        assert_eq!(
            invoke_dialogue(
                envelope,
                &current,
                &current,
                &handler,
                owner,
                dialogue_admission(
                    &captured,
                    Some(&captured),
                    DiscourseContext::Unspecified,
                    "Do that",
                ),
            ),
            Err(expected)
        );
    }
    let mut older_command = input.clone();
    let GameInput::Game(command) = &mut older_command else {
        unreachable!()
    };
    command.observed_revision = revision(2, 1);
    assert_eq!(
        invoke_dialogue(
            RulesCommandInput {
                command: &older_command,
                supplied_draws: &[],
            },
            &current,
            &current,
            &handler,
            admission(&current),
            dialogue_admission(
                &captured,
                Some(&captured),
                DiscourseContext::Unspecified,
                "Do that",
            ),
        ),
        Err(DialogueResponseError::InputBindingMismatch)
    );
    for index in 0..3 {
        let mut stale = captured.clone();
        match index {
            0 => stale.member = member(99),
            1 => stale.basis.revision = stale.basis.revision.next_sequence().unwrap(),
            2 => stale.pins.rules.handler = label("foreign-handler"),
            _ => unreachable!(),
        }
        assert!(matches!(
            invoke_dialogue(
                envelope,
                &current,
                &current,
                &handler,
                admission(&current),
                dialogue_admission(
                    &stale,
                    Some(&stale),
                    DiscourseContext::Unspecified,
                    "Do that",
                ),
            ),
            Err(DialogueResponseError::Dialogue(_))
        ));
    }
    assert_eq!(handler.calls.get(), 0);
    assert_eq!(current, original);
}

#[test]
fn confirmed_dialogue_revalidates_offers_capacity_generation_and_cancellation() {
    let current = waiting(0);
    let original = current.clone();
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let handler = handler(&current, envelope);
    let captured = captured_dialogue(&current);
    for index in 0..3 {
        let mut owner = admission(&current);
        let expected = match index {
            0 => {
                owner.fence.current_generation += 1;
                ResumeError::StaleGeneration
            }
            1 => {
                owner.fence.cancel_before_admission = true;
                ResumeError::CancelledBeforeAdmission
            }
            2 => {
                owner.maximum_bytes = 1;
                ResumeError::Capacity
            }
            _ => unreachable!(),
        };
        assert_eq!(
            invoke_dialogue(
                envelope,
                &current,
                &current,
                &handler,
                owner,
                dialogue_admission(
                    &captured,
                    Some(&captured),
                    DiscourseContext::Unspecified,
                    "Do that",
                ),
            ),
            Err(DialogueResponseError::Semantic(
                SemanticResponseError::Resume(expected)
            ))
        );
    }
    let mut unoffered = input.clone();
    let GameInput::Game(command) = &mut unoffered else {
        unreachable!()
    };
    let GameCommand::SelectChoice { option, .. } = &mut command.command else {
        unreachable!()
    };
    *option = label("not-offered");
    assert_eq!(
        invoke_dialogue(
            RulesCommandInput {
                command: &unoffered,
                supplied_draws: &[],
            },
            &current,
            &current,
            &handler,
            admission(&current),
            dialogue_admission(
                &captured,
                Some(&captured),
                DiscourseContext::Unspecified,
                "Do that",
            ),
        ),
        Err(DialogueResponseError::Semantic(
            SemanticResponseError::Candidate(CandidateError::Command(
                CommandError::UnofferedResponse
            ))
        ))
    );
    assert_eq!(handler.calls.get(), 0);
    assert_eq!(current, original);
}

#[test]
fn dialogue_text_limits_and_source_refusal_stay_explicit_without_hidden_effects() {
    let current = waiting(0);
    let original = current.clone();
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let mut handler = handler(&current, envelope);
    let captured = captured_dialogue(&current);
    for (text, maximum, expected) in [
        (" ", 64, DialogueError::EmptyInput),
        ("Do that", 0, DialogueError::Capacity),
        ("Do that", 2, DialogueError::Capacity),
    ] {
        let mut dialogue = dialogue_admission(
            &captured,
            Some(&captured),
            DiscourseContext::Unspecified,
            text,
        );
        dialogue.limits.maximum_text_bytes = maximum;
        assert_eq!(
            invoke_dialogue(
                envelope,
                &current,
                &current,
                &handler,
                admission(&current),
                dialogue,
            ),
            Err(DialogueResponseError::Dialogue(expected))
        );
    }
    let mut missing_source = admission(&current);
    missing_source.rules.clear();
    assert!(matches!(
        invoke_dialogue(
            envelope,
            &current,
            &current,
            &handler,
            missing_source,
            dialogue_admission(
                &captured,
                Some(&captured),
                DiscourseContext::Unspecified,
                "Do that",
            ),
        ),
        Err(DialogueResponseError::Semantic(
            SemanticResponseError::Candidate(CandidateError::Command(
                CommandError::InvalidReference
            ))
        ))
    ));
    assert_eq!(handler.calls.get(), 0);
    handler.refuse = true;
    assert!(matches!(
        invoke_dialogue(
            envelope,
            &current,
            &current,
            &handler,
            admission(&current),
            dialogue_admission(
                &captured,
                Some(&captured),
                DiscourseContext::Unspecified,
                "Do that",
            ),
        ),
        Err(DialogueResponseError::Semantic(
            SemanticResponseError::Resume(ResumeError::Command(CommandRejection::Invocation(
                InvocationError::Handler(PreconditionedRejection::Handler(
                    FixtureRejection::Unsupported
                ))
            )))
        ))
    ));
    assert_eq!(handler.calls.get(), 1);
    assert_eq!(current, original);
}
