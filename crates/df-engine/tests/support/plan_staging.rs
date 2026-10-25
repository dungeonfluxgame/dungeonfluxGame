use crate::pending_fixtures::*;
use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::command_entry::{CommandEntryContext, CommandEntryLimits};
use df_engine::pending_resumption::{ResumeFence, ResumeLimits};
use df_engine::plan_staging::{PlanHeadAdmission, PlanHeadError, StagedPlanHead, stage_plan_head};
use df_intent::candidate::CandidateOwner;
use df_intent::plan::PlanLimits;
use df_model::checkpoint::*;
use df_model::commands::CommandLimits;
use df_rules::preconditions::{
    CurrentRuleContext, PreconditionLimits, PreconditionedCommandHandler, RuleDependency,
    RulePreconditions,
};
use df_rules::{DispatchRegistry, HandlerRegistration, RulesCommandHandler, RulesCommandInput};
use df_types::{MemberId, OperationId, RevisionLabel};
use std::cell::Cell;

pub(super) const BYTES: usize = 1024 * 1024;

pub(super) struct Admission {
    pub member: MemberId,
    pub operation: OperationId,
    pub basis: Basis,
    pub pins: CheckpointPins,
    pub selector: RevisionLabel,
    pub fence: ResumeFence,
    pub plan: PlanLimits,
    pub engine_command: CommandLimits,
    pub maximum_bytes: usize,
}

pub(super) fn operation(input: &GameInput) -> OperationId {
    let GameInput::Game(command) = input else {
        panic!("fixture game command only")
    };
    command.operation
}

pub(super) fn admission(current: &Checkpoint, first: &GameInput) -> Admission {
    Admission {
        member: member(3),
        operation: operation(first),
        basis: current.basis(),
        pins: current.pins().clone(),
        selector: label("fixture-continuation-position-1"),
        fence: fence(),
        plan: PlanLimits {
            maximum_steps: 4,
            maximum_total_input_bytes: BYTES,
            maximum_comparisons: 256,
            commands: command_limits(),
        },
        engine_command: command_limits(),
        maximum_bytes: BYTES,
    }
}

pub(super) fn two_steps(kind: u8) -> Vec<GameInput> {
    let first = response(kind);
    let mut second = first.clone();
    let GameInput::Game(command) = &mut second else {
        unreachable!()
    };
    command.operation = OperationId::from_bytes(&[21; 16]).unwrap();
    vec![first, second]
}

// Source-controlled synthetic checkpoint outcomes exercise wiring, not D&D mechanics.
pub(super) struct Handler {
    pub candidate: Checkpoint,
    pub calls: Cell<usize>,
    pub refuse: bool,
    pub command_pointer: Cell<*const GameInput>,
    pub draw_pointer: Cell<*const ActualDraw>,
}

impl RulesCommandHandler for Handler {
    type Rejection = FixtureRejection;

    fn pins(&self) -> &CheckpointPins {
        self.candidate.pins()
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        _: &Checkpoint,
    ) -> Result<Checkpoint, FixtureRejection> {
        self.calls.set(self.calls.get() + 1);
        self.command_pointer.set(input.command);
        self.draw_pointer.set(input.supplied_draws.as_ptr());
        if self.refuse {
            Err(FixtureRejection::Unsupported)
        } else {
            Ok(self.candidate.clone())
        }
    }
}

pub(super) fn handler(current: &Checkpoint, input: &GameInput, draws: &[ActualDraw]) -> Handler {
    Handler {
        candidate: supplied_candidate(
            current,
            RulesCommandInput {
                command: input,
                supplied_draws: draws,
            },
        ),
        calls: Cell::new(0),
        refuse: false,
        command_pointer: Cell::new(std::ptr::null()),
        draw_pointer: Cell::new(std::ptr::null()),
    }
}

pub(super) fn rebuild(current: &Checkpoint, state: GameState) -> Checkpoint {
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        current.basis(),
        current.pins().clone(),
        state,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap()
}

pub(super) fn invoke<'a>(
    steps: &'a [GameInput],
    draws: &[ActualDraw],
    current: &Checkpoint,
    prepared: &Checkpoint,
    handler: &Handler,
    admission: Admission,
) -> Result<StagedPlanHead<'a>, PlanHeadError<FixtureRejection>> {
    let rules = [rule()];
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
    let selector = label("fixture-continuation-position-1");
    let inventory = || ReferenceInventory {
        rules: &rules,
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
            command_limits: admission.engine_command,
        },
        RulePreconditions {
            prepared,
            sources: &rules,
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
    let registrations = [HandlerRegistration::new(&selector, &source, &wrapper)];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    stage_plan_head(
        steps,
        draws,
        current,
        CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory: inventory(),
            limits: CommandEntryLimits {
                command: admission.engine_command,
                maximum_staged_bytes: admission.maximum_bytes,
            },
        },
        PlanHeadAdmission {
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
            plan_limits: admission.plan,
        },
        &registry,
        &admission.selector,
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) mod durable {
    use super::*;
    use df_observe::OperationContext;
    use df_session::inbox::{ActorInput, AdmissionSequence, bounded_inbox};
    use df_session::submission::*;
    use df_types::SessionId;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Eq, PartialEq)]
    pub(crate) struct Scope {
        pub input: GameInput,
        pub principal: MemberId,
    }

    impl ActorInput for Scope {
        fn retained_heap_bytes(&self) -> Option<usize> {
            self.input.retained_heap_bytes()
        }
    }

    impl OperationScope for Scope {
        type UncertaintyKey = Scope;

        fn capture_uncertainty_key(
            &self,
            maximum_retained_bytes: usize,
        ) -> Result<Scope, RepositoryError> {
            if self
                .input
                .retained_bytes()
                .and_then(|retained_bytes| retained_bytes.checked_add(std::mem::size_of::<Scope>()))
                .is_none_or(|retained_bytes| retained_bytes > maximum_retained_bytes)
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
            operation(&self.input)
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
    pub(crate) enum Failure {
        None,
        BeforeCommit,
        LostAck,
        UnknownNotCommitted,
    }

    #[derive(Clone, Copy, Eq, PartialEq)]
    pub(crate) enum Outcome {
        Preserve,
        SpendResource,
        CloseWindow,
    }

    // Controlled native repository, exact head ledger and ephemeral list submission fixture.
    // This is not a PostgreSQL adapter, persisted plan, source qualification or D&D rule.
    pub(crate) struct Database {
        pub checkpoint: Checkpoint,
        pub ledger: Vec<(Scope, DecisionReceipt)>,
        pub failure: Failure,
        pub outcome: Outcome,
        pub prepared: Option<Checkpoint>,
        pub ephemeral_steps: Vec<GameInput>,
        pub engines: usize,
        pub handlers: usize,
        pub commits: usize,
        pub publications: usize,
        pub observed_bases: Vec<Basis>,
        pub refusals: Vec<PlanHeadError<FixtureRejection>>,
        pub returned_tail: Vec<GameInput>,
    }

    pub(crate) struct Repository {
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

    pub(crate) struct Engine {
        database: Rc<RefCell<Database>>,
    }

    impl SessionEngine<Scope> for Engine {
        fn decide(
            &mut self,
            current: &Checkpoint,
            scope: &Scope,
            input: &GameInput,
        ) -> Result<Checkpoint, RepositoryError> {
            let (steps, prepared, outcome) = {
                let mut db = self.database.borrow_mut();
                db.engines += 1;
                db.observed_bases.push(current.basis());
                (
                    db.ephemeral_steps.clone(),
                    db.prepared.clone().unwrap_or_else(|| current.clone()),
                    db.outcome,
                )
            };
            if steps.first() != Some(input) {
                return Err(RepositoryError::InputBinding);
            }
            let mut handler = handler(current, input, &[]);
            let mut state = handler.candidate.state().clone();
            match outcome {
                Outcome::Preserve => {}
                Outcome::SpendResource => state.resources[0].value -= 1,
                Outcome::CloseWindow => state.pending.clear(),
            }
            handler.candidate = rebuild(&handler.candidate, state);
            // Each attempted head receives actual current native basis/pins/member/operation.
            let mut trusted = admission(current, input);
            trusted.member = scope.principal;
            trusted.operation = scope.operation();
            let result = invoke(&steps, &[], current, &prepared, &handler, trusted);
            let mut db = self.database.borrow_mut();
            db.handlers += handler.calls.get();
            match result {
                Ok(staged) => {
                    assert_eq!(handler.command_pointer.get(), &steps[0] as *const GameInput);
                    db.returned_tail = staged.requires_revalidation().to_vec();
                    Ok(staged.into_parts().0)
                }
                Err(error) => {
                    db.refusals.push(error);
                    Err(RepositoryError::InvalidCandidate)
                }
            }
        }

        fn validate_recovery(&mut self, current: &Checkpoint) -> Result<(), RepositoryError> {
            current
                .validate_resume(current.basis(), &pins())
                .map(|_| ())
                .map_err(|_| RepositoryError::InvalidCandidate)
        }
    }

    pub(crate) struct Publication {
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

    pub(crate) type Owner = DurableOwner<Repository, Engine, Publication>;

    pub(crate) fn setup(failure: Failure) -> (Owner, Rc<RefCell<Database>>, Vec<GameInput>) {
        let current = waiting(0);
        let steps = two_steps(0);
        let database = Rc::new(RefCell::new(Database {
            checkpoint: current.clone(),
            ledger: vec![],
            failure,
            outcome: Outcome::Preserve,
            prepared: None,
            ephemeral_steps: steps.clone(),
            engines: 0,
            handlers: 0,
            commits: 0,
            publications: 0,
            observed_bases: vec![],
            refusals: vec![],
            returned_tail: vec![],
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
        (owner, database, steps)
    }

    pub(crate) fn submit(owner: &mut Owner, input: &GameInput) -> SubmissionOutcome {
        submit_scope(
            owner,
            Scope {
                input: input.clone(),
                principal: member(3),
            },
        )
    }

    pub(crate) fn submit_scope(owner: &mut Owner, scope: Scope) -> SubmissionOutcome {
        let (input, wait) = OwnedInput::new(
            OperationContext {
                trace_parent: String::new(),
                build: "bounded-plan-head-fixture".to_owned(),
            },
            scope.clone(),
            scope.input.clone(),
        );
        // Exercise actual Session inbox admission and serialized DurableOwner reduction.
        let (inbox, actor) = bounded_inbox();
        assert!(inbox.try_submit(input).is_ok());
        inbox.stop().unwrap();
        let drained = actor.run(owner).unwrap();
        assert_eq!(drained.reduced_inputs, 1);
        assert_eq!(drained.last_sequence, Some(AdmissionSequence(1)));
        wait.try_recv().unwrap()
    }
}
