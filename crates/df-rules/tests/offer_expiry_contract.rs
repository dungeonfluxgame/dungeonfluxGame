//! Synthetic canonical records prove current-window and source-dependency binding, not D&D.
#[path = "../src/offer_expiry_contract.rs"]
mod contract;
include!("../src/tests/fixtures.rs");

use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_model::commands::{CommandError, CommandLimits};
use df_rules::current_responses::{ResponseError, ResponsePreparationError, current_responses};
use df_rules::preconditions::{
    CurrentRuleContext, PreconditionError, PreconditionLimits, PreconditionedCommandHandler,
    PreconditionedRejection, RuleDependency, RulePreconditions,
};
use df_rules::{
    DispatchRegistry, HandlerRegistration, InvocationError, RulesCommandHandler, RulesCommandInput,
};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Refusal {
    InvalidInput,
    InvalidCandidate,
}
struct Handler {
    pins: CheckpointPins,
    calls: Cell<usize>,
}
impl Handler {
    fn new() -> Self {
        Self {
            pins: pins(),
            calls: Cell::new(0),
        }
    }
}
impl RulesCommandHandler for Handler {
    type Rejection = Refusal;
    fn pins(&self) -> &CheckpointPins {
        &self.pins
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Refusal> {
        self.calls.set(self.calls.get() + 1);
        let GameInput::Game(request) = input.command else {
            return Err(Refusal::InvalidInput);
        };
        let mut basis = current.basis();
        basis.revision = basis
            .revision
            .next_sequence()
            .map_err(|_| Refusal::InvalidCandidate)?;
        let mut state = current.state().clone();
        state.pending.clear();
        state.decisions.push(AcceptedDecision {
            operation: request.operation,
            revision: basis.revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-handler-policy"),
            semantic_output: None,
        });
        make_checkpoint(state, basis).map_err(|_| Refusal::InvalidCandidate)
    }
}
fn command_limits() -> CommandLimits {
    CommandLimits {
        maximum_records: 8,
        maximum_text_bytes: 64,
        maximum_retained_bytes: 8192,
    }
}
fn request(reaction: bool) -> CommandInput {
    let pending_resolution = pending();
    CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: OperationId::from_bytes(&[6; 16]).unwrap(),
        member: member(3),
        command: if reaction {
            GameCommand::SelectReaction {
                resolution: pending_resolution.id,
                window: pending_resolution.window.id,
                offer: label("fixture-offer-1"),
                option: label("fixture-option-1"),
            }
        } else {
            GameCommand::SelectChoice {
                resolution: pending_resolution.id,
                window: pending_resolution.window.id,
                offer: label("fixture-offer-1"),
                option: label("fixture-option-1"),
            }
        },
    }
}
fn waiting(reaction: bool) -> Checkpoint {
    let mut game_state = state();
    game_state.facts.push(fact(7, 0));
    let mut pending_resolution = pending();
    if !reaction && let PendingInput::Reaction { remaining } = pending_resolution.next {
        pending_resolution.next = PendingInput::Choice { remaining };
    }
    game_state.pending.push(pending_resolution);
    checkpoint(game_state).unwrap()
}
fn make_checkpoint(mut game_state: GameState, basis: Basis) -> Result<Checkpoint, CheckpointError> {
    for pending_resolution in &mut game_state.pending {
        pending_resolution.basis = basis;
    }
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        pins(),
        game_state,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
}
fn updated(previous_checkpoint: &Checkpoint, change: impl FnOnce(&mut GameState)) -> Checkpoint {
    let mut game_state = previous_checkpoint.state().clone();
    change(&mut game_state);
    let mut basis = previous_checkpoint.basis();
    basis.revision = basis.revision.next_sequence().unwrap();
    make_checkpoint(game_state, basis).unwrap()
}
type Error = ResponsePreparationError<Refusal>;
fn with_pipeline<T>(
    prepared: &Checkpoint,
    current: &Checkpoint,
    handler: &Handler,
    dependencies: &[RuleDependency],
    invoke: impl FnOnce(
        &DispatchRegistry<'_, PreconditionedCommandHandler<'_, Handler>>,
        CurrentRuleContext<'_>,
        &RevisionLabel,
    ) -> T,
) -> T {
    let source = rule();
    let sources = [source.clone()];
    let contents = [content()];
    let resources = resource_constraints();
    let context = || CurrentRuleContext {
        checkpoint: current,
        basis: current.basis(),
        pins: current.pins(),
        inventory: ReferenceInventory {
            rules: &sources,
            content: &contents,
            resources: &resources,
            assets: &[],
        },
        command_limits: command_limits(),
    };
    let wrapped = PreconditionedCommandHandler::new(
        handler,
        &source,
        context(),
        RulePreconditions {
            prepared,
            sources: &sources,
            dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 16,
            maximum_comparisons: 8 * 1024 * 1024,
            maximum_checkpoint_bytes: 1024 * 1024,
        },
    );
    let selector = pending().continuation;
    let registrations = [HandlerRegistration::new(&selector, &source, &wrapped)];
    let entries = [CatalogEntry::new(&source, b"synthetic structural fixture")];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic structural fixture",
        &entries,
        CatalogLimits {
            max_complete_bytes: 1024,
            max_entries: 8,
            max_item_bytes: 1024,
            max_total_item_bytes: 1024,
        },
    )
    .unwrap();
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 8).unwrap();
    invoke(&registry, context(), &selector)
}
fn submit(
    prepared: &Checkpoint,
    current: &Checkpoint,
    handler: &Handler,
    dependencies: &[RuleDependency],
    request: CommandInput,
) -> Result<Checkpoint, Error> {
    let input = GameInput::Game(request);
    with_pipeline(
        prepared,
        current,
        handler,
        dependencies,
        |registry, context, selector| {
            contract::submit(
                RulesCommandInput {
                    command: &input,
                    supplied_draws: &[],
                },
                context,
                registry,
                selector,
                1024 * 1024,
            )
        },
    )
}
fn enumerate(
    prepared: &Checkpoint,
    current: &Checkpoint,
    handler: &Handler,
    dependencies: &[RuleDependency],
    request: &CommandInput,
) -> Result<Vec<GameInput>, Error> {
    with_pipeline(
        prepared,
        current,
        handler,
        dependencies,
        |registry, context, selector| {
            contract::enumerate(request, context, registry, selector, 1024 * 1024)
        },
    )
}
fn admission(error: CommandError) -> Error {
    Error::Response(ResponseError::Admission(error))
}
fn precondition(error: PreconditionError) -> Error {
    Error::Invocation(InvocationError::Handler(
        PreconditionedRejection::Precondition(error),
    ))
}

#[test]
fn current_choice_and_reaction_enumeration_and_submission_agree_without_spending_or_drawing() {
    for reaction in [false, true] {
        let current = waiting(reaction);
        let before = current.clone();
        let handler = Handler::new();
        let dependencies = [
            RuleDependency::Entity(entity(4)),
            RuleDependency::PendingResolution(pending().id),
        ];
        let options = enumerate(
            &current,
            &current,
            &handler,
            &dependencies,
            &request(reaction),
        )
        .unwrap();
        assert_eq!(options.len(), 1);
        let GameInput::Game(selected) = &options[0] else {
            panic!("game selection");
        };
        let candidate = submit(
            &current,
            &current,
            &handler,
            &dependencies,
            selected.clone(),
        )
        .unwrap();
        assert_eq!(candidate.state().resources, current.state().resources);
        assert_eq!(candidate.state().draws, current.state().draws);
        assert_eq!(current, before);
        assert_eq!(handler.calls.get(), 2);
    }
}
#[test]
fn exact_surviving_offer_admits_old_sequence_after_unrelated_revision_movement() {
    for reaction in [false, true] {
        let prepared = waiting(reaction);
        let current = updated(&prepared, |game_state| game_state.resources[0].value = 3);
        let handler = Handler::new();
        let dependencies = [
            RuleDependency::Entity(entity(4)),
            RuleDependency::PendingResolution(pending().id),
        ];
        let old = submit(
            &prepared,
            &current,
            &handler,
            &dependencies,
            request(reaction),
        )
        .unwrap();
        let fresh = enumerate(
            &prepared,
            &current,
            &handler,
            &dependencies,
            &request(reaction),
        )
        .unwrap();
        let GameInput::Game(fresh) = &fresh[0] else {
            panic!("game selection");
        };
        assert_eq!(fresh.basis, current.basis());
        assert_eq!(
            old,
            submit(&prepared, &current, &handler, &dependencies, fresh.clone()).unwrap()
        );
    }
}
#[test]
fn expired_removed_or_replaced_window_refuses_both_paths_before_handler() {
    for reaction in [false, true] {
        let prepared = waiting(reaction);
        for replaced in [false, true] {
            let current = updated(&prepared, |game_state| {
                if replaced {
                    game_state.pending[0].window.id = WindowId::from_bytes(&[11; 16]).unwrap();
                } else {
                    game_state.pending.clear();
                }
            });
            let handler = Handler::new();
            assert_eq!(
                enumerate(&prepared, &current, &handler, &[], &request(reaction)),
                Err(admission(CommandError::StaleWindow))
            );
            assert_eq!(
                submit(&prepared, &current, &handler, &[], request(reaction)),
                Err(admission(CommandError::StaleWindow))
            );
            assert_eq!(handler.calls.get(), 0);
        }
    }
}
#[test]
fn withdrawn_offer_or_option_cannot_be_submitted_and_enumeration_uses_current_records() {
    for reaction in [false, true] {
        let prepared = waiting(reaction);
        for option in [false, true] {
            let current = updated(&prepared, |game_state| {
                let remaining = match &mut game_state.pending[0].next {
                    PendingInput::Choice { remaining } | PendingInput::Reaction { remaining } => {
                        remaining
                    }
                    _ => unreachable!(),
                };
                if option {
                    remaining[0].options = vec![label("replacement-option")];
                } else {
                    remaining[0].offer = label("replacement-offer");
                }
            });
            let handler = Handler::new();
            assert_eq!(
                submit(&prepared, &current, &handler, &[], request(reaction)),
                Err(admission(CommandError::UnofferedResponse))
            );
            assert_eq!(handler.calls.get(), 0);
            let fresh = enumerate(&current, &current, &handler, &[], &request(reaction)).unwrap();
            let GameInput::Game(fresh) = &fresh[0] else {
                panic!("game selection");
            };
            assert!(submit(&current, &current, &handler, &[], fresh.clone()).is_ok());
        }
    }
}
#[test]
fn changed_target_basis_refuses_query_and_submit_without_invoking_mechanics() {
    for reaction in [false, true] {
        let prepared = waiting(reaction);
        let current = updated(&prepared, |game_state| {
            game_state.entities[0].position = Some(Position { x: 1, y: 0, z: 0 })
        });
        let before = current.clone();
        let handler = Handler::new();
        let dependencies = [RuleDependency::Entity(entity(4))];
        assert_eq!(
            enumerate(
                &prepared,
                &current,
                &handler,
                &dependencies,
                &request(reaction)
            ),
            Err(precondition(PreconditionError::StaleEntity))
        );
        assert_eq!(
            submit(
                &prepared,
                &current,
                &handler,
                &dependencies,
                request(reaction)
            ),
            Err(precondition(PreconditionError::StaleEntity))
        );
        assert_eq!(handler.calls.get(), 0);
        assert_eq!(current, before);
    }
}
#[test]
fn source_declared_time_expiry_refuses_both_paths_without_wall_clock_or_countdown() {
    for reaction in [false, true] {
        let prepared = waiting(reaction);
        let current = updated(&prepared, |game_state| game_state.logical_time.ticks += 1);
        let handler = Handler::new();
        let dependencies = [RuleDependency::LogicalTime];
        assert_eq!(
            enumerate(
                &prepared,
                &current,
                &handler,
                &dependencies,
                &request(reaction)
            ),
            Err(precondition(PreconditionError::StaleTime))
        );
        assert_eq!(
            submit(
                &prepared,
                &current,
                &handler,
                &dependencies,
                request(reaction)
            ),
            Err(precondition(PreconditionError::StaleTime))
        );
        assert_eq!(handler.calls.get(), 0);
    }
}
#[test]
fn foreign_session_run_epoch_member_and_future_revision_refuse_before_handler() {
    for reaction in [false, true] {
        let current = waiting(reaction);
        for variant in 0..6 {
            let mut selected = request(reaction);
            let expected = match variant {
                0 => {
                    selected.basis.session = SessionId::from_bytes(&[20; 16]).unwrap();
                    CommandError::WrongSession
                }
                1 => {
                    selected.basis.run = RunId::from_bytes(&[20; 16]).unwrap();
                    CommandError::WrongRun
                }
                2 => {
                    selected.basis.revision = revision(1, 8);
                    CommandError::StaleRevision
                }
                3 => {
                    selected.observed_revision = revision(3, 0);
                    CommandError::StaleRevision
                }
                4 => {
                    selected.member = member(20);
                    CommandError::UnknownMember
                }
                _ => {
                    selected.observed_revision = revision(2, 9);
                    CommandError::StaleRevision
                }
            };
            let handler = Handler::new();
            // Enumeration filters participant records before validating generated commands.
            let enumerated = if variant == 4 {
                CommandError::UnofferedResponse
            } else {
                expected
            };
            assert_eq!(
                enumerate(&current, &current, &handler, &[], &selected),
                Err(admission(enumerated))
            );
            assert_eq!(
                submit(&current, &current, &handler, &[], selected),
                Err(admission(expected))
            );
            assert_eq!(handler.calls.get(), 0);
        }
    }
}
#[test]
fn current_offer_for_another_member_is_not_the_requesting_members_offer() {
    for reaction in [false, true] {
        let prepared = waiting(reaction);
        let current = updated(&prepared, |game_state| {
            game_state.members.push(MembershipLink {
                member: member(12),
                character: None,
            });
            let remaining = match &mut game_state.pending[0].next {
                PendingInput::Choice { remaining } | PendingInput::Reaction { remaining } => {
                    remaining
                }
                _ => unreachable!(),
            };
            remaining[0].participant = member(12);
        });
        let handler = Handler::new();
        assert_eq!(
            enumerate(&prepared, &current, &handler, &[], &request(reaction)),
            Err(admission(CommandError::UnofferedResponse))
        );
        assert_eq!(
            submit(&prepared, &current, &handler, &[], request(reaction)),
            Err(admission(CommandError::UnofferedResponse))
        );
        assert_eq!(handler.calls.get(), 0);
    }
}
#[test]
fn structural_list_is_not_source_handler_legality_for_changed_targets() {
    let prepared = waiting(false);
    let current = updated(&prepared, |game_state| {
        game_state.entities[0].identity_revision = label("changed-target")
    });
    let template = request(false);
    assert_eq!(
        current_responses(
            &template,
            &current,
            current.basis(),
            current.pins(),
            ReferenceInventory {
                rules: &[rule()],
                content: &[content()],
                resources: &resource_constraints(),
                assets: &[]
            },
            command_limits()
        )
        .unwrap()
        .len(),
        1
    );
    let handler = Handler::new();
    assert_eq!(
        enumerate(
            &prepared,
            &current,
            &handler,
            &[RuleDependency::Entity(entity(4))],
            &template
        ),
        Err(precondition(PreconditionError::StaleEntity))
    );
    assert_eq!(handler.calls.get(), 0);
}
