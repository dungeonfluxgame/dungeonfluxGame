#[path = "support/pending_fixtures.rs"]
mod pending_fixtures;

use df_combat::candidates::{CandidateContext, CandidateLimits};
use df_combat::current_response::{
    CurrentResponseRanking, ResponseUtility, stage_preferred_response,
};
use df_combat::ranking::{KnowledgeObserver, RankingObservation, UtilityLimits, UtilityPolicy};
use df_combat::registered_candidates::{
    RegisteredCandidateLimits, RegisteredCandidateRequest, enumerate_registered_responses,
};
use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::combat_staging::{
    TacticalResponseAdmission, TacticalResponseError, stage_tactical_response,
};
use df_engine::command_entry::{CommandEntryContext, CommandEntryLimits, CommandRejection};
use df_engine::pending_resumption::{ResumeError, ResumeFence, ResumeLimits};
use df_model::checkpoint::*;
use df_model::commands::CommandLimits;
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
    rules: Vec<RuleReference>,
    selector: RevisionLabel,
    fence: ResumeFence,
    command: CommandLimits,
    maximum_bytes: usize,
}

fn admission(_: &Checkpoint) -> Admission {
    Admission {
        member: member(3),
        operation: OperationId::from_bytes(&[20; 16]).unwrap(),
        rules: vec![rule()],
        selector: label("fixture-continuation-position-1"),
        fence: fence(),
        command: command_limits(),
        maximum_bytes: BYTES,
    }
}

// Synthetic registered continuation: retire the completed pending window. No D&D mechanic.
fn handler(current: &Checkpoint, input: RulesCommandInput<'_>) -> SuppliedHandler {
    let supplied = supplied_candidate(current, input);
    let mut state = supplied.state().clone();
    state.pending.clear();
    let candidate = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        supplied.basis(),
        supplied.pins().clone(),
        state,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    SuppliedHandler {
        candidate,
        pins: current.pins().clone(),
        calls: Rc::new(Cell::new(0)),
        refuse: false,
        observed_draw_pointer: Rc::new(Cell::new(std::ptr::null())),
    }
}

/// Exercise real Combat enumeration/selection before the engine adapter.
/// Preview owns a separate source fixture; no preview checkpoint can become the committed result.
fn invoke_tactical(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    preview: &Checkpoint,
    prepared: &Checkpoint,
    final_handler: &SuppliedHandler,
    admission: Admission,
) -> Result<Checkpoint, TacticalResponseError<FixtureRejection>> {
    let kind = match &preview.state().pending[0].next {
        PendingInput::Choice { .. } => 0,
        PendingInput::Reaction { .. } => 1,
        _ => unreachable!(),
    };
    let template_input = response(kind);
    let GameInput::Game(template) = &template_input else {
        unreachable!()
    };
    let contents = [content()];
    let resources = resource_constraints();
    let source = rule();
    let selector = label("fixture-continuation-position-1");
    let preview_rules = [rule()];
    let dependencies = [
        RuleDependency::Resource {
            owner: entity(4),
            resource: label("fixture-resource-1"),
        },
        RuleDependency::PendingResolutions,
    ];
    let inventory = || ReferenceInventory {
        rules: &preview_rules,
        content: &contents,
        resources: &resources,
        assets: &[],
    };
    let context = || CurrentRuleContext {
        checkpoint: preview,
        basis: preview.basis(),
        pins: preview.pins(),
        inventory: inventory(),
        command_limits: command_limits(),
    };
    let preview_handler = handler(
        preview,
        RulesCommandInput {
            command: &template_input,
            supplied_draws: &[],
        },
    );
    let preview_wrapper = PreconditionedCommandHandler::new(
        &preview_handler,
        &source,
        context(),
        RulePreconditions {
            prepared: preview,
            sources: &preview_rules,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 8,
            maximum_comparisons: BYTES,
            maximum_checkpoint_bytes: BYTES,
        },
    );
    let entries = [CatalogEntry::new(&source, b"synthetic-clause")];
    let catalog = || {
        CatalogSnapshot::from_published(
            &preview.pins().rules.catalog,
            preview.pins(),
            b"synthetic-publication",
            &entries,
            CatalogLimits {
                max_complete_bytes: 128,
                max_entries: 4,
                max_item_bytes: 64,
                max_total_item_bytes: 128,
            },
        )
        .unwrap()
    };
    let registrations = [HandlerRegistration::new(
        &selector,
        &source,
        &preview_wrapper,
    )];
    let registry = DispatchRegistry::from_catalog(catalog(), &registrations, 1).unwrap();
    let basis = preview.basis();
    let admitted = enumerate_registered_responses(
        context(),
        &registry,
        RegisteredCandidateRequest {
            template,
            current: CandidateContext {
                basis: &basis,
                pins: preview.pins(),
            },
            member: member(3),
            operation: template.operation,
            selector: &selector,
        },
        RegisteredCandidateLimits {
            candidates: CandidateLimits {
                max_offers: 8,
                max_identity_comparisons: 28,
                max_candidates: 8,
                max_candidate_bytes: BYTES,
            },
            maximum_checkpoint_bytes: BYTES,
            maximum_inventory_records: 8,
            maximum_rules_preparations: 8,
            maximum_staged_bytes: BYTES,
        },
    )
    .unwrap();
    let assignments: Vec<_> = admitted
        .offers()
        .iter()
        .map(|response| ResponseUtility {
            response,
            option: &response.options[0],
            contributions: &[],
        })
        .collect();
    let observation = RankingObservation {
        basis: &basis,
        pins: preview.pins(),
        observer: KnowledgeObserver::Member(member(3)),
        logical_time: preview.state().logical_time,
        cause: template.operation,
        policy: &contents[0],
    };
    let selected = stage_preferred_response(
        CurrentResponseRanking {
            template,
            observation: &observation,
            assignments: &assignments,
            policy: &UtilityPolicy {
                basis: &basis,
                pins: preview.pins(),
                definition: &contents[0],
                criteria: &[],
            },
            limits: UtilityLimits {
                candidates: 8,
                contributions: 8,
                knowledge_records: 8,
                criteria: 8,
            },
            maximum_staged_bytes: BYTES,
        },
        &admitted,
        context(),
        &registry,
        &selector,
    )
    .unwrap()
    .unwrap();
    assert_eq!(preview_handler.calls.get(), 2);
    assert_eq!(final_handler.calls.get(), 0);

    let references = || ReferenceInventory {
        rules: &admission.rules,
        content: &contents,
        resources: &resources,
        assets: &[],
    };
    let wrapper = PreconditionedCommandHandler::new(
        final_handler,
        &source,
        CurrentRuleContext {
            checkpoint: current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: references(),
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
    let final_registrations = [HandlerRegistration::new(&selector, &source, &wrapper)];
    let final_registry =
        DispatchRegistry::from_catalog(catalog(), &final_registrations, 1).unwrap();
    // Successful cases use the actual selected command. Overrides exercise untrusted input refusal.
    let command = if input.command == selected.input() {
        selected.input()
    } else {
        input.command
    };
    stage_tactical_response(
        RulesCommandInput {
            command,
            supplied_draws: input.supplied_draws,
        },
        current,
        CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory: references(),
            limits: CommandEntryLimits {
                command: admission.command,
                maximum_staged_bytes: admission.maximum_bytes,
            },
        },
        TacticalResponseAdmission {
            candidates: &admitted,
            member: admission.member,
            operation: admission.operation,
            fence: admission.fence,
            limits: ResumeLimits {
                maximum_checkpoint_bytes: admission.maximum_bytes,
            },
        },
        &final_registry,
        &admission.selector,
    )
}

#[test]
fn registered_choice_and_reaction_selection_enter_engine_without_applying_preview() {
    for kind in 0..2 {
        let current = waiting(kind);
        let original = current.clone();
        let input = response(kind);
        let envelope = RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        };
        let handler = handler(&current, envelope);
        let staged = invoke_tactical(
            envelope,
            &current,
            &current,
            &current,
            &handler,
            admission(&current),
        )
        .unwrap();
        assert_eq!(staged, handler.candidate);
        assert_eq!(handler.calls.get(), 1);
        assert_eq!(current, original);
        assert!(staged.state().pending.is_empty());
        assert_eq!(staged.state().resources, current.state().resources);
        assert_eq!(staged.state().facts, current.state().facts);
        assert_eq!(staged.state().logical_time, current.state().logical_time);
        assert_eq!(
            invoke(envelope, &current, &current, &handler, fence(), BYTES).unwrap(),
            staged
        );
    }
}

#[test]
fn independently_admitted_member_and_operation_cannot_come_from_selected_labels() {
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
    for (owner, expected) in [
        (foreign_member, TacticalResponseError::MemberMismatch),
        (foreign_operation, TacticalResponseError::OperationMismatch),
    ] {
        assert_eq!(
            invoke_tactical(envelope, &current, &current, &current, &handler, owner),
            Err(expected)
        );
    }
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn equal_detached_pending_snapshot_is_not_the_exact_admitted_response() {
    let preview = waiting(0);
    let detached = preview.clone();
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let handler = handler(&detached, envelope);
    assert_eq!(
        invoke_tactical(
            envelope,
            &detached,
            &preview,
            &detached,
            &handler,
            admission(&detached)
        ),
        Err(TacticalResponseError::UnadmittedResponse)
    );
    assert_eq!(handler.calls.get(), 0);
    assert_eq!(detached, preview);
}

#[test]
fn previously_selected_input_is_stale_after_current_revision_or_pin_change() {
    let preview = waiting(0);
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let first_handler = handler(&preview, envelope);
    let changed_revision = first_handler.candidate.clone();
    assert_eq!(
        invoke_tactical(
            envelope,
            &changed_revision,
            &preview,
            &changed_revision,
            &first_handler,
            admission(&changed_revision)
        ),
        Err(TacticalResponseError::StaleBasis)
    );
    let mut changed_pins = preview.pins().clone();
    changed_pins.build = df_types::BuildIdentity::new(
        Some("other-source"),
        Some("other-native"),
        Some("other-wasm"),
        Some("other-config"),
        Some("other-content"),
    )
    .unwrap();
    let changed = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        preview.basis(),
        changed_pins,
        preview.state().clone(),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    assert_eq!(
        invoke_tactical(
            envelope,
            &changed,
            &preview,
            &changed,
            &first_handler,
            admission(&changed)
        ),
        Err(TacticalResponseError::StalePins)
    );
    assert_eq!(first_handler.calls.get(), 0);
}

#[test]
fn unoffered_options_actions_rolls_and_host_inputs_cannot_become_tactics() {
    let current = waiting(0);
    let canonical = response(0);
    let handler = handler(
        &current,
        RulesCommandInput {
            command: &canonical,
            supplied_draws: &[],
        },
    );
    let mut unoffered = canonical.clone();
    let GameInput::Game(command) = &mut unoffered else {
        unreachable!()
    };
    let GameCommand::SelectChoice { option, .. } = &mut command.command else {
        unreachable!()
    };
    *option = label("unoffered");
    assert_eq!(
        invoke_tactical(
            RulesCommandInput {
                command: &unoffered,
                supplied_draws: &[]
            },
            &current,
            &current,
            &current,
            &handler,
            admission(&current)
        ),
        Err(TacticalResponseError::UnadmittedResponse)
    );
    for input in [
        response(2),
        response(3),
        game(GameCommand::ProposeAction {
            actor: entity(99),
            action: content(),
            targets: vec![],
            choices: vec![],
        }),
    ] {
        assert_eq!(
            invoke_tactical(
                RulesCommandInput {
                    command: &input,
                    supplied_draws: &[]
                },
                &current,
                &current,
                &current,
                &handler,
                admission(&current)
            ),
            Err(TacticalResponseError::UnsupportedCommand)
        );
    }
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn current_source_selector_dependencies_fence_and_capacity_are_rechecked_after_preview() {
    let current = waiting(0);
    let input = response(0);
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &[],
    };
    let mut handler = handler(&current, envelope);
    let mut stale = admission(&current);
    stale.fence.current_generation += 1;
    let mut cancelled = admission(&current);
    cancelled.fence.cancel_before_admission = true;
    let mut capacity = admission(&current);
    capacity.maximum_bytes = 1;
    for (owner, expected) in [
        (
            stale,
            TacticalResponseError::Resume(ResumeError::StaleGeneration),
        ),
        (
            cancelled,
            TacticalResponseError::Resume(ResumeError::CancelledBeforeAdmission),
        ),
        (capacity, TacticalResponseError::Capacity),
    ] {
        assert_eq!(
            invoke_tactical(envelope, &current, &current, &current, &handler, owner),
            Err(expected)
        );
    }
    let before_revocation = current.clone();
    let mut revoked = admission(&current);
    revoked.rules.clear();
    assert_eq!(
        invoke_tactical(envelope, &current, &current, &current, &handler, revoked),
        Err(TacticalResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Precondition(PreconditionError::InvalidReference)
            ))
        )))
    );
    assert_eq!(handler.calls.get(), 0);
    assert_eq!(current, before_revocation);
    let mut unavailable = admission(&current);
    unavailable.selector = label("unregistered");
    assert!(matches!(
        invoke_tactical(
            envelope,
            &current,
            &current,
            &current,
            &handler,
            unavailable
        ),
        Err(TacticalResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::Dispatch(_))
        )))
    ));
    let mut changed_state = current.state().clone();
    changed_state.resources[0].value -= 1;
    let prepared = checkpoint(changed_state).unwrap();
    assert_eq!(
        invoke_tactical(
            envelope,
            &current,
            &current,
            &prepared,
            &handler,
            admission(&current)
        ),
        Err(TacticalResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Precondition(PreconditionError::StaleResource)
            ))
        )))
    );
    assert_eq!(handler.calls.get(), 0);
    let original = current.clone();
    handler.refuse = true;
    assert_eq!(
        invoke_tactical(
            envelope,
            &current,
            &current,
            &current,
            &handler,
            admission(&current)
        ),
        Err(TacticalResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Handler(FixtureRejection::Unsupported)
            ))
        )))
    );
    assert_eq!(handler.calls.get(), 1);
    assert_eq!(current, original);
}

#[test]
fn native_draws_reach_final_handler_unchanged_and_invalid_draws_never_run_it() {
    let current = waiting(1);
    let input = response(1);
    let draws = [native_roll_draw(0)];
    let envelope = RulesCommandInput {
        command: &input,
        supplied_draws: &draws,
    };
    let handler = handler(&current, envelope);
    let mut invalid = draws.to_vec();
    invalid[0].operation = OperationId::from_bytes(&[99; 16]).unwrap();
    assert_eq!(
        invoke_tactical(
            RulesCommandInput {
                command: &input,
                supplied_draws: &invalid
            },
            &current,
            &current,
            &current,
            &handler,
            admission(&current)
        ),
        Err(TacticalResponseError::Resume(ResumeError::Command(
            CommandRejection::Invocation(InvocationError::DrawOperationMismatch)
        )))
    );
    assert_eq!(handler.calls.get(), 0);
    let staged = invoke_tactical(
        envelope,
        &current,
        &current,
        &current,
        &handler,
        admission(&current),
    )
    .unwrap();
    assert!(std::ptr::eq(
        handler.observed_draw_pointer.get(),
        draws.as_ptr()
    ));
    assert_eq!(staged.state().draws, draws);
    assert_eq!(staged.state().decisions[0].draws, vec![0]);
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
            let result = invoke_tactical(envelope, current, current, current, &handler, trusted);
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
                build: "tactical-pending-fixture".to_owned(),
            },
            scope.clone(),
            scope.input.clone(),
        );
        owner.reduce(AdmissionSequence(1), input);
        wait.try_recv().unwrap()
    }

    #[test]
    fn durable_owner_calls_registered_tactical_engine_once_and_retries_exact_receipt() {
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
    fn tactical_or_principal_refusal_never_calls_handler_commits_or_publishes() {
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
    fn lost_commit_ack_uses_ledger_lookup_without_reexecuting_tactical_handler() {
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

use df_combat::registered_candidates::{RegisteredActionRequest, enumerate_registered_actions};
use df_engine::combat_staging::{
    TacticalActionAdmission, TacticalActionError, stage_tactical_action,
};

fn tactical_action_checkpoint() -> Checkpoint {
    let mut state = waiting(2).state().clone();
    for id in [5, 6] {
        state.entities.push(WorldEntity {
            id: entity(id),
            definition: content(),
            location: None,
            position: None,
            identity_revision: label("fixture-enemy"),
        });
    }
    state.encounters.push(EncounterState {
        id: RecordId::from_bytes(&[15; 16]).unwrap(),
        definition: content(),
        participants: vec![entity(5), entity(4), entity(6)],
        turn_order: vec![entity(5), entity(4), entity(6)],
        active_turn: Some(entity(5)),
        objectives: vec![content()],
        combat_policy: content(),
    });
    checkpoint(state).unwrap()
}

fn tactical_action_input(target: u8) -> GameInput {
    game(GameCommand::ProposeAction {
        actor: entity(5),
        action: content(),
        targets: vec![entity(target)],
        choices: vec![],
    })
}

struct ActionAuthority {
    native: Admission,
    actor: EntityId,
    source: RuleReference,
    registered: bool,
}

fn action_authority() -> ActionAuthority {
    ActionAuthority {
        native: admission(&tactical_action_checkpoint()),
        actor: entity(5),
        source: rule(),
        registered: true,
    }
}

/// Real Combat admission -> source-bound receipt -> actual registered Engine staging.
/// Both source handlers are explicit boundary fixtures, not source-producing enemy mechanics.
fn invoke_action(
    current: &Checkpoint,
    preview: &Checkpoint,
    prepared: &Checkpoint,
    offers: &[GameInput],
    input: RulesCommandInput<'_>,
    final_handler: &SuppliedHandler,
    authority: ActionAuthority,
) -> Result<Checkpoint, TacticalActionError<FixtureRejection>> {
    let source = rule();
    let selector = label("fixture-continuation-position-1");
    let preview_rules = [rule()];
    let contents = [content()];
    let resources = resource_constraints();
    let dependencies = [
        RuleDependency::Resource {
            owner: entity(4),
            resource: label("fixture-resource-1"),
        },
        RuleDependency::LogicalTime,
        RuleDependency::Entity(entity(5)),
        RuleDependency::Encounter(RecordId::from_bytes(&[15; 16]).unwrap()),
        RuleDependency::PendingResolutions,
    ];
    let preview_inventory = || ReferenceInventory {
        rules: &preview_rules,
        content: &contents,
        resources: &resources,
        assets: &[],
    };
    let preview_context = || CurrentRuleContext {
        checkpoint: preview,
        basis: preview.basis(),
        pins: preview.pins(),
        inventory: preview_inventory(),
        command_limits: command_limits(),
    };
    let preview_handler = handler(
        preview,
        RulesCommandInput {
            command: offers.first().unwrap_or(input.command),
            supplied_draws: &[],
        },
    );
    let wrapper = PreconditionedCommandHandler::new(
        &preview_handler,
        &source,
        preview_context(),
        RulePreconditions {
            prepared: preview,
            sources: &preview_rules,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 16,
            maximum_comparisons: 8 * BYTES,
            maximum_checkpoint_bytes: BYTES,
        },
    );
    let entries = [CatalogEntry::new(&source, b"synthetic-action-clause")];
    let catalog = CatalogSnapshot::from_published(
        &preview.pins().rules.catalog,
        preview.pins(),
        b"synthetic-action-publication",
        &entries,
        CatalogLimits {
            max_complete_bytes: 128,
            max_entries: 4,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let registrations = [HandlerRegistration::new(&selector, &source, &wrapper)];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    let basis = preview.basis();
    let receipt = enumerate_registered_actions(
        preview_context(),
        &registry,
        RegisteredActionRequest {
            current: CandidateContext {
                basis: &basis,
                pins: preview.pins(),
            },
            actor: entity(5),
            member: member(3),
            operation: OperationId::from_bytes(&[20; 16]).unwrap(),
            source: &source,
            selector: &selector,
        },
        offers,
        RegisteredCandidateLimits {
            candidates: CandidateLimits {
                max_offers: 8,
                max_identity_comparisons: 28,
                max_candidates: 8,
                max_candidate_bytes: BYTES,
            },
            maximum_checkpoint_bytes: BYTES,
            maximum_inventory_records: 8,
            maximum_rules_preparations: 8,
            maximum_staged_bytes: BYTES,
        },
    )
    .unwrap();
    assert_eq!(preview_handler.calls.get(), offers.len());
    assert_eq!(final_handler.calls.get(), 0);
    let current_inventory = || ReferenceInventory {
        rules: &authority.native.rules,
        content: &contents,
        resources: &resources,
        assets: &[],
    };
    let current_wrapper = PreconditionedCommandHandler::new(
        final_handler,
        &authority.source,
        CurrentRuleContext {
            checkpoint: current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: current_inventory(),
            command_limits: authority.native.command,
        },
        RulePreconditions {
            prepared,
            sources: &authority.native.rules,
            dependencies: &dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 16,
            maximum_comparisons: 8 * BYTES,
            maximum_checkpoint_bytes: BYTES,
        },
    );
    let current_entries = [CatalogEntry::new(
        &authority.source,
        b"synthetic-action-clause",
    )];
    let current_catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-action-publication",
        &current_entries,
        CatalogLimits {
            max_complete_bytes: 128,
            max_entries: 4,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let final_registrations = [HandlerRegistration::new(
        &authority.native.selector,
        &authority.source,
        &current_wrapper,
    )];
    let final_registry = DispatchRegistry::from_catalog(
        current_catalog,
        if authority.registered {
            &final_registrations
        } else {
            &[]
        },
        1,
    )
    .unwrap();
    if authority.registered {
        assert!(
            final_registry
                .select(
                    current.pins(),
                    &authority.native.selector,
                    &authority.source
                )
                .is_ok()
        );
    }
    stage_tactical_action(
        input,
        current,
        CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory: current_inventory(),
            limits: CommandEntryLimits {
                command: authority.native.command,
                maximum_staged_bytes: authority.native.maximum_bytes,
            },
        },
        TacticalActionAdmission {
            candidates: &receipt,
            actor: authority.actor,
            member: authority.native.member,
            operation: authority.native.operation,
            source: &authority.source,
            fence: authority.native.fence,
            limits: ResumeLimits {
                maximum_checkpoint_bytes: authority.native.maximum_bytes,
            },
        },
        &final_registry,
        &authority.native.selector,
    )
}

#[test]
fn registered_actions_reach_engine_as_exact_inputs_and_never_apply_preview() {
    let current = tactical_action_checkpoint();
    let original = current.clone();
    let offers = [tactical_action_input(4), tactical_action_input(6)];
    let input = RulesCommandInput {
        command: &offers[1],
        supplied_draws: &[],
    };
    let final_handler = handler(&current, input);
    let staged = invoke_action(
        &current,
        &current,
        &current,
        &offers,
        input,
        &final_handler,
        action_authority(),
    )
    .unwrap();
    assert_eq!(final_handler.calls.get(), 1);
    assert_eq!(
        staged.basis().revision,
        current.basis().revision.next_sequence().unwrap()
    );
    assert_eq!(staged.state().resources, current.state().resources);
    assert_eq!(staged.state().facts, current.state().facts);
    assert_eq!(staged.state().decisions.len(), 1);
    assert!(staged.state().pending.is_empty());
    assert_eq!(current, original);
}

#[test]
fn registered_action_receipt_binds_independent_native_actor_member_and_operation() {
    let current = tactical_action_checkpoint();
    let offers = [tactical_action_input(4)];
    let input = RulesCommandInput {
        command: &offers[0],
        supplied_draws: &[],
    };
    for case in 0..3 {
        let mut authority = action_authority();
        let expected = match case {
            0 => {
                authority.native.member = member(99);
                TacticalActionError::MemberMismatch
            }
            1 => {
                authority.native.operation = OperationId::from_bytes(&[99; 16]).unwrap();
                TacticalActionError::OperationMismatch
            }
            2 => {
                authority.actor = entity(4);
                TacticalActionError::ActorMismatch
            }
            _ => unreachable!(),
        };
        let final_handler = handler(&current, input);
        assert_eq!(
            invoke_action(
                &current,
                &current,
                &current,
                &offers,
                input,
                &final_handler,
                authority
            ),
            Err(expected)
        );
        assert_eq!(final_handler.calls.get(), 0);
    }
}

#[test]
fn action_receipt_rejects_other_current_registered_source_or_selector() {
    let current = tactical_action_checkpoint();
    let offers = [tactical_action_input(4)];
    let input = RulesCommandInput {
        command: &offers[0],
        supplied_draws: &[],
    };
    for change_source in [false, true] {
        let mut authority = action_authority();
        let expected = if change_source {
            authority.source.clause = label("other-current-compiled-clause");
            authority.native.rules.push(authority.source.clone());
            TacticalActionError::SourceMismatch
        } else {
            authority.native.selector = label("other-current-compiled-selector");
            TacticalActionError::SelectorMismatch
        };
        let final_handler = handler(&current, input);
        assert_eq!(
            invoke_action(
                &current,
                &current,
                &current,
                &offers,
                input,
                &final_handler,
                authority
            ),
            Err(expected)
        );
        assert_eq!(final_handler.calls.get(), 0);
    }
}

#[test]
fn equal_detached_changed_empty_and_unsupported_action_inputs_cannot_use_receipt() {
    let current = tactical_action_checkpoint();
    let offers = [tactical_action_input(4)];
    for case in 0..6 {
        let mut detached = offers[0].clone();
        if case == 1 {
            let GameInput::Game(command) = &mut detached else {
                unreachable!()
            };
            let GameCommand::ProposeAction { targets, .. } = &mut command.command else {
                unreachable!()
            };
            *targets = vec![entity(6)];
        } else if case == 2 {
            detached = response(0);
        } else if case == 3 {
            detached = host(HostCommand::RequestCheckpoint);
        } else if case == 4 {
            detached = GameInput::Timer(TimerExpiry {
                basis: basis(),
                timer: TimerId::from_bytes(&[21; 16]).unwrap(),
                generation: 1,
                observed_time: current.state().logical_time,
            });
        }
        let selected = if case == 5 { &offers[0] } else { &detached };
        let input = RulesCommandInput {
            command: selected,
            supplied_draws: &[],
        };
        let final_handler = handler(&current, input);
        let expected = if (2..=4).contains(&case) {
            TacticalActionError::UnsupportedCommand
        } else {
            TacticalActionError::UnadmittedAction
        };
        let inventory = if case == 5 { &[][..] } else { &offers[..] };
        assert_eq!(
            invoke_action(
                &current,
                &current,
                &current,
                inventory,
                input,
                &final_handler,
                action_authority()
            ),
            Err(expected)
        );
        assert_eq!(final_handler.calls.get(), 0);
    }
}

#[test]
fn action_receipt_current_basis_and_pins_are_rechecked_after_enumeration() {
    let preview = tactical_action_checkpoint();
    let offers = [tactical_action_input(4)];
    let input = RulesCommandInput {
        command: &offers[0],
        supplied_draws: &[],
    };
    for change_pins in [false, true] {
        let mut basis = preview.basis();
        let mut pins = preview.pins().clone();
        if change_pins {
            pins.rules.handler = label("changed-handler-pin");
        } else {
            basis.revision = basis.revision.next_sequence().unwrap();
        }
        let current = Checkpoint::new(
            CHECKPOINT_SCHEMA,
            basis,
            pins,
            preview.state().clone(),
            ReferenceInventory {
                rules: &[rule()],
                content: &[content()],
                resources: &resource_constraints(),
                assets: &[],
            },
            limits(),
        )
        .unwrap();
        let final_handler = handler(&current, input);
        assert_eq!(
            invoke_action(
                &current,
                &preview,
                &preview,
                &offers,
                input,
                &final_handler,
                action_authority()
            ),
            Err(if change_pins {
                TacticalActionError::StalePins
            } else {
                TacticalActionError::StaleBasis
            })
        );
        assert_eq!(final_handler.calls.get(), 0);
    }
}

#[test]
fn current_action_source_handler_generation_cancellation_and_capacity_are_rechecked() {
    let current = tactical_action_checkpoint();
    let original = current.clone();
    let offers = [tactical_action_input(4)];
    let input = RulesCommandInput {
        command: &offers[0],
        supplied_draws: &[],
    };
    for case in 0..6 {
        let mut authority = action_authority();
        let expected = match case {
            0 => {
                authority.native.fence.current_generation += 1;
                Some(TacticalActionError::StaleGeneration)
            }
            1 => {
                authority.native.fence.current_generation = 0;
                Some(TacticalActionError::StaleGeneration)
            }
            2 => {
                authority.native.fence.cancel_before_admission = true;
                Some(TacticalActionError::CancelledBeforeAdmission)
            }
            3 => {
                authority.native.maximum_bytes = 1;
                Some(TacticalActionError::Capacity)
            }
            4 => {
                authority.native.rules.clear();
                Some(TacticalActionError::SourceMismatch)
            }
            5 => {
                authority.registered = false;
                None
            }
            _ => unreachable!(),
        };
        let final_handler = handler(&current, input);
        let result = invoke_action(
            &current,
            &current,
            &current,
            &offers,
            input,
            &final_handler,
            authority,
        );
        if let Some(expected) = expected {
            assert_eq!(result, Err(expected));
        } else {
            assert!(matches!(
                result,
                Err(TacticalActionError::Command(CommandRejection::Invocation(
                    InvocationError::Dispatch(_)
                )))
            ));
        }
        assert_eq!(final_handler.calls.get(), 0);
        assert_eq!(current, original);
    }
}

#[test]
fn complete_action_dependencies_and_handler_refusal_are_rechecked_before_final_staging() {
    let prepared = tactical_action_checkpoint();
    let offers = [tactical_action_input(4)];
    let input = RulesCommandInput {
        command: &offers[0],
        supplied_draws: &[],
    };
    for case in 0..6 {
        let mut state = prepared.state().clone();
        let dependency = match case {
            0 => {
                state.resources[0].value -= 1;
                Some(PreconditionError::StaleResource)
            }
            1 => {
                state.logical_time.ticks += 1;
                Some(PreconditionError::StaleTime)
            }
            2 => {
                state.entities[1].identity_revision = label("changed-identity");
                Some(PreconditionError::StaleEntity)
            }
            3 => {
                state.encounters[0].active_turn = Some(entity(4));
                Some(PreconditionError::StaleEncounter)
            }
            4 => {
                state.pending[0].continuation = label("changed-continuation");
                Some(PreconditionError::StalePending)
            }
            5 => None,
            _ => unreachable!(),
        };
        let current = checkpoint(state).unwrap();
        let original = current.clone();
        let mut final_handler = handler(&current, input);
        final_handler.refuse = case == 5;
        let rejection = if let Some(dependency) = dependency {
            PreconditionedRejection::Precondition(dependency)
        } else {
            PreconditionedRejection::Handler(FixtureRejection::Unsupported)
        };
        assert_eq!(
            invoke_action(
                &current,
                &prepared,
                &prepared,
                &offers,
                input,
                &final_handler,
                action_authority()
            ),
            Err(TacticalActionError::Command(CommandRejection::Invocation(
                InvocationError::Handler(rejection)
            )))
        );
        assert_eq!(final_handler.calls.get(), usize::from(case == 5));
        assert_eq!(current, original);
    }
}

#[test]
fn original_native_action_draw_slice_is_preserved_and_invalid_outcome_never_runs_handler() {
    let current = tactical_action_checkpoint();
    let original = current.clone();
    let offers = [tactical_action_input(4)];
    let draws = [native_roll_draw(0)];
    let input = RulesCommandInput {
        command: &offers[0],
        supplied_draws: &draws,
    };
    let final_handler = handler(&current, input);
    let mut invalid = draws.to_vec();
    invalid[0].operation = OperationId::from_bytes(&[99; 16]).unwrap();
    assert_eq!(
        invoke_action(
            &current,
            &current,
            &current,
            &offers,
            RulesCommandInput {
                command: &offers[0],
                supplied_draws: &invalid
            },
            &final_handler,
            action_authority(),
        ),
        Err(TacticalActionError::Command(CommandRejection::Invocation(
            InvocationError::DrawOperationMismatch
        )))
    );
    assert_eq!(final_handler.calls.get(), 0);
    let staged = invoke_action(
        &current,
        &current,
        &current,
        &offers,
        input,
        &final_handler,
        action_authority(),
    )
    .unwrap();
    assert!(std::ptr::eq(
        final_handler.observed_draw_pointer.get(),
        draws.as_ptr()
    ));
    assert_eq!(staged.state().draws, draws);
    assert_eq!(staged.state().decisions[0].draws, vec![0]);
    assert_eq!(final_handler.calls.get(), 1);
    assert_eq!(current, original);
}
