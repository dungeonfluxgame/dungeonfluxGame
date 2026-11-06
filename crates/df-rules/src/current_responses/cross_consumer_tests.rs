use super::current_responses_tests::{command_limits, request, waiting};
use super::fixtures::*;
use super::{
    ResponseError, ResponsePreparationError, current_pending_source, offered_current_commands,
    prepare_current_response,
};
use crate::command_handler::{InvocationError, RulesCommandHandler, RulesCommandInput};
use crate::dispatch::{DispatchError, DispatchRegistry, HandlerRegistration};
use crate::preconditions::{
    CurrentRuleContext, PreconditionError, PreconditionLimits, PreconditionedCommandHandler,
    PreconditionedRejection, RuleDependency, RulePreconditions,
};
use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_model::checkpoint::*;
use df_model::commands::CommandError;
use df_types::{OperationId, RevisionLabel};
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyntheticRefusal {
    Refused,
    InvalidInput,
    InvalidCheckpoint,
}

// This fixture proves invocation and checkpoint binding only; it implements no D&D mechanic.
struct SyntheticHandler {
    pins: CheckpointPins,
    called: Cell<usize>,
    inputs: RefCell<Vec<GameInput>>,
    draws: RefCell<Vec<Vec<ActualDraw>>>,
    draw_pointers: RefCell<Vec<usize>>,
    reject: bool,
    append_foreign: bool,
}
impl SyntheticHandler {
    fn new() -> Self {
        Self {
            pins: pins(),
            called: Cell::new(0),
            inputs: RefCell::new(vec![]),
            draws: RefCell::new(vec![]),
            draw_pointers: RefCell::new(vec![]),
            reject: false,
            append_foreign: false,
        }
    }
}
impl RulesCommandHandler for SyntheticHandler {
    type Rejection = SyntheticRefusal;
    fn pins(&self) -> &CheckpointPins {
        &self.pins
    }
    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        self.called.set(self.called.get() + 1);
        self.inputs.borrow_mut().push(input.command.clone());
        self.draws.borrow_mut().push(input.supplied_draws.to_vec());
        self.draw_pointers
            .borrow_mut()
            .push(input.supplied_draws.as_ptr() as usize);
        if self.reject {
            return Err(SyntheticRefusal::Refused);
        }
        let operation = match input.command {
            GameInput::Game(input) => input.operation,
            GameInput::Host(input) => input.operation,
            _ => return Err(SyntheticRefusal::InvalidInput),
        };
        let mut next = current.basis();
        next.revision = next
            .revision
            .next_sequence()
            .map_err(|_| SyntheticRefusal::InvalidCheckpoint)?;
        let mut state = current.state().clone();
        state.pending.clear();
        let mut accepted_facts = vec![];
        let first_fact_ordinal = match state
            .facts
            .iter()
            .filter(|fact| fact.operation == operation)
            .map(|fact| fact.ordinal)
            .next_back()
        {
            Some(ordinal) => ordinal
                .checked_add(1)
                .ok_or(SyntheticRefusal::InvalidCheckpoint)?,
            None => 0,
        };
        for (index, draw) in input.supplied_draws.iter().enumerate() {
            let ordinal_offset =
                u32::try_from(index).map_err(|_| SyntheticRefusal::InvalidCheckpoint)?;
            let fact_ordinal = first_fact_ordinal
                .checked_add(ordinal_offset)
                .ok_or(SyntheticRefusal::InvalidCheckpoint)?;
            let id_byte = u8::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(30))
                .ok_or(SyntheticRefusal::InvalidCheckpoint)?;
            let id = FactId::from_bytes(&[id_byte; 16])
                .map_err(|_| SyntheticRefusal::InvalidCheckpoint)?;
            state.facts.push(GameFact {
                id,
                revision: next.revision,
                operation,
                ordinal: fact_ordinal,
                cause: None,
                audience: AudienceScope::Shared,
                value: FactValue::DrawAccepted {
                    operation,
                    ordinal: draw.ordinal,
                },
            });
            accepted_facts.push(id);
        }
        state.draws.extend_from_slice(input.supplied_draws);
        let accepted_draws = state
            .draws
            .iter()
            .filter(|draw| draw.operation == operation)
            .map(|draw| draw.ordinal)
            .collect();
        state.decisions.push(AcceptedDecision {
            operation,
            revision: next.revision,
            facts: accepted_facts,
            draws: accepted_draws,
            effects: vec![],
            source_policy: label("fixture-handler-policy"),
            semantic_output: None,
        });
        if self.append_foreign {
            state.decisions.push(AcceptedDecision {
                operation: OperationId::from_bytes(&[99; 16])
                    .map_err(|_| SyntheticRefusal::InvalidCheckpoint)?,
                revision: next.revision,
                facts: vec![],
                draws: vec![],
                effects: vec![],
                source_policy: label("fixture-foreign-policy"),
                semantic_output: None,
            });
        }
        Checkpoint::new(
            CHECKPOINT_SCHEMA,
            next,
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
        .map_err(|_| SyntheticRefusal::InvalidCheckpoint)
    }
}

fn without_draws(command: &GameInput) -> RulesCommandInput<'_> {
    RulesCommandInput {
        command,
        supplied_draws: &[],
    }
}

fn supplied_roll_fixture() -> ActualDraw {
    ActualDraw {
        operation: request(false).operation,
        ordinal: 0,
        resolution: pending().id,
        window: pending().window.id,
        sides: 20,
        value: 17,
        source: rule(),
    }
}

fn with_pipeline<R>(
    prepared: &Checkpoint,
    current: &Checkpoint,
    handler: &SyntheticHandler,
    dependencies: &[RuleDependency],
    call: impl FnOnce(
        &DispatchRegistry<'_, PreconditionedCommandHandler<'_, SyntheticHandler>>,
        CurrentRuleContext<'_>,
        &RevisionLabel,
    ) -> R,
) -> R {
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
    call(&registry, context(), &selector)
}

fn checkpoint_at(mut state: GameState, basis: Basis) -> Checkpoint {
    for pending in &mut state.pending {
        pending.basis = basis;
    }
    Checkpoint::new(
        CHECKPOINT_SCHEMA,
        basis,
        pins(),
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

#[test]
fn current_offers_and_submitted_commands_invoke_the_same_registered_dependency_guard() {
    for reaction in [false, true] {
        let current = waiting(reaction, |_| {});
        let before = current.clone();
        let handler = SyntheticHandler::new();
        with_pipeline(
            &current,
            &current,
            &handler,
            &[RuleDependency::PendingResolution(pending().id)],
            |registry, context, selector| {
                let offered = offered_current_commands(
                    &request(reaction),
                    context,
                    registry,
                    selector,
                    1024 * 1024,
                )
                .unwrap();
                assert_eq!(offered.len(), 1);
                let selected = offered.first().unwrap();
                let candidate = with_pipeline(
                    &current,
                    &current,
                    &handler,
                    &[RuleDependency::PendingResolution(pending().id)],
                    |registry, context, selector| {
                        prepare_current_response(
                            without_draws(selected),
                            context,
                            registry,
                            selector,
                            1024 * 1024,
                        )
                        .unwrap()
                    },
                );
                assert_eq!(
                    candidate.state().decisions.first().unwrap().operation,
                    request(reaction).operation
                );
                assert_eq!(handler.called.get(), 2);
                assert!(handler.draws.borrow().iter().all(Vec::is_empty));
                let inputs = handler.inputs.borrow();
                assert_eq!(inputs.first(), inputs.get(1));
            },
        );
        assert_eq!(current, before);
    }
}

#[test]
fn unchanged_relevant_response_survives_new_current_revision_and_older_client_observation() {
    let prepared = waiting(false, |_| {});
    let mut next = prepared.basis();
    next.revision = next.revision.next_sequence().unwrap();
    let current = checkpoint_at(prepared.state().clone(), next);
    let handler = SyntheticHandler::new();
    let submitted = GameInput::Game(request(false));
    let offered = with_pipeline(
        &prepared,
        &current,
        &handler,
        &[RuleDependency::PendingResolution(pending().id)],
        |registry, context, selector| {
            offered_current_commands(&request(false), context, registry, selector, 1024 * 1024)
                .unwrap()
        },
    );
    let from_old = with_pipeline(
        &prepared,
        &current,
        &handler,
        &[RuleDependency::PendingResolution(pending().id)],
        |registry, context, selector| {
            prepare_current_response(
                without_draws(&submitted),
                context,
                registry,
                selector,
                1024 * 1024,
            )
            .unwrap()
        },
    );
    let from_fresh = with_pipeline(
        &prepared,
        &current,
        &handler,
        &[RuleDependency::PendingResolution(pending().id)],
        |registry, context, selector| {
            prepare_current_response(
                without_draws(offered.first().unwrap()),
                context,
                registry,
                selector,
                1024 * 1024,
            )
            .unwrap()
        },
    );
    assert_eq!(from_old, from_fresh);
    let GameInput::Game(fresh) = offered.first().unwrap() else {
        panic!("game offer");
    };
    let GameInput::Game(old) = &submitted else {
        panic!("game selection");
    };
    assert_eq!(fresh.command, old.command);
    assert_eq!(fresh.member, old.member);
    assert_eq!(fresh.operation, old.operation);
    assert_eq!(fresh.basis, current.basis());
    assert_eq!(old.basis, prepared.basis());
}

#[test]
fn changed_relevant_resource_refuses_query_and_submit_before_selected_mechanic_runs() {
    let prepared = waiting(true, |_| {});
    let mut state = prepared.state().clone();
    state.resources.first_mut().unwrap().value = 3;
    let current = checkpoint_at(state, prepared.basis());
    let handler = SyntheticHandler::new();
    let dependency = [RuleDependency::Resource {
        owner: entity(4),
        resource: label("fixture-resource-1"),
    }];
    let expected = ResponsePreparationError::Invocation(InvocationError::Handler(
        PreconditionedRejection::Precondition(PreconditionError::StaleResource),
    ));
    let offered = with_pipeline(
        &prepared,
        &current,
        &handler,
        &dependency,
        |registry, context, selector| {
            offered_current_commands(&request(true), context, registry, selector, 1024 * 1024)
        },
    );
    let submitted = with_pipeline(
        &prepared,
        &current,
        &handler,
        &dependency,
        |registry, context, selector| {
            prepare_current_response(
                without_draws(&GameInput::Game(request(true))),
                context,
                registry,
                selector,
                1024 * 1024,
            )
        },
    );
    assert_eq!(offered, Err(expected.clone()));
    assert_eq!(submitted, Err(expected));
    assert_eq!(handler.called.get(), 0);
}

#[test]
fn missing_registry_binding_and_handler_refusal_do_not_publish_usable_partial_offers() {
    let current = waiting(false, |_| {});
    let mut handler = SyntheticHandler::new();
    let missing = with_pipeline(&current, &current, &handler, &[], |registry, context, _| {
        offered_current_commands(
            &request(false),
            context,
            registry,
            &label("unknown-handler"),
            1024 * 1024,
        )
    });
    assert_eq!(
        missing,
        Err(ResponsePreparationError::Invocation(
            InvocationError::Dispatch(DispatchError::UnknownHandler)
        ))
    );
    assert_eq!(handler.called.get(), 0);
    handler.reject = true;
    let refused = with_pipeline(
        &current,
        &current,
        &handler,
        &[],
        |registry, context, selector| {
            offered_current_commands(&request(false), context, registry, selector, 1024 * 1024)
        },
    );
    assert_eq!(
        refused,
        Err(ResponsePreparationError::Invocation(
            InvocationError::Handler(PreconditionedRejection::Handler(SyntheticRefusal::Refused))
        ))
    );
    assert_eq!(handler.called.get(), 1);
}

#[test]
fn roll_and_host_ruling_resolve_exact_canonical_pending_source_and_stage_once() {
    for ruling in [false, true] {
        let current = waiting(false, |state| {
            let pending = state.pending.first_mut().unwrap();
            pending.next = if ruling {
                PendingInput::Ruling {
                    permitted: vec![OfferedResponse {
                        participant: member(3),
                        offer: label("fixture-offer-1"),
                        options: vec![label("fixture-option-1")],
                        source: rule(),
                    }],
                    source: rule(),
                }
            } else {
                PendingInput::Roll {
                    participant: member(3),
                    sides: vec![20],
                    source: rule(),
                }
            };
        });
        let input = if ruling {
            GameInput::Host(HostInput {
                basis: basis(),
                operation: request(false).operation,
                host: member(3),
                command: HostCommand::ResolveRuling {
                    resolution: pending().id,
                    window: pending().window.id,
                    offer: label("fixture-offer-1"),
                    option: label("fixture-option-1"),
                },
            })
        } else {
            let mut input = request(false);
            input.command = GameCommand::SubmitRoll {
                resolution: pending().id,
                window: pending().window.id,
            };
            GameInput::Game(input)
        };
        assert_eq!(current_pending_source(&input, &current).unwrap(), &rule());
        let handler = SyntheticHandler::new();
        // The test owner explicitly supplies the recorded outcome; no adapter invents a roll.
        let supplied = if ruling {
            vec![]
        } else {
            vec![supplied_roll_fixture()]
        };
        let before = supplied.clone();
        let candidate = with_pipeline(
            &current,
            &current,
            &handler,
            &[RuleDependency::PendingResolution(pending().id)],
            |registry, context, selector| {
                prepare_current_response(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &supplied,
                    },
                    context,
                    registry,
                    selector,
                    1024 * 1024,
                )
                .unwrap()
            },
        );
        assert_eq!(handler.called.get(), 1);
        assert_eq!(supplied, before);
        assert_eq!(handler.draws.borrow().first().unwrap(), &supplied);
        assert_eq!(
            *handler.draw_pointers.borrow().first().unwrap(),
            supplied.as_ptr() as usize
        );
        assert_eq!(candidate.state().draws, supplied);
    }
}

#[test]
fn changed_current_participant_refuses_registered_submission_before_handler_invocation() {
    let prepared = waiting(false, |_| {});
    let current = waiting(false, |state| {
        state.members.push(MembershipLink {
            member: member(12),
            character: None,
        });
        if let PendingInput::Choice { remaining } = &mut state.pending.first_mut().unwrap().next {
            remaining.first_mut().unwrap().participant = member(12);
        }
    });
    let handler = SyntheticHandler::new();
    let expected = ResponsePreparationError::Response(ResponseError::Admission(
        CommandError::UnofferedResponse,
    ));
    let offered = with_pipeline(
        &prepared,
        &current,
        &handler,
        &[],
        |registry, context, selector| {
            offered_current_commands(&request(false), context, registry, selector, 1024 * 1024)
        },
    );
    let submitted = with_pipeline(
        &prepared,
        &current,
        &handler,
        &[],
        |registry, context, selector| {
            prepare_current_response(
                without_draws(&GameInput::Game(request(false))),
                context,
                registry,
                selector,
                1024 * 1024,
            )
        },
    );
    assert_eq!(offered, Err(expected.clone()));
    assert_eq!(submitted, Err(expected));
    assert_eq!(handler.called.get(), 0);
}

#[test]
fn roll_without_trusted_actual_outcomes_is_refused_without_inventing_draws() {
    let current = waiting(false, |state| {
        state.pending.first_mut().unwrap().next = PendingInput::Roll {
            participant: member(3),
            sides: vec![20],
            source: rule(),
        };
    });
    let mut request = request(false);
    request.command = GameCommand::SubmitRoll {
        resolution: pending().id,
        window: pending().window.id,
    };
    let input = GameInput::Game(request);
    let handler = SyntheticHandler::new();
    let before = current.clone();
    let result = with_pipeline(
        &current,
        &current,
        &handler,
        &[RuleDependency::PendingResolution(pending().id)],
        |registry, context, selector| {
            prepare_current_response(
                without_draws(&input),
                context,
                registry,
                selector,
                1024 * 1024,
            )
        },
    );
    assert_eq!(
        result,
        Err(ResponsePreparationError::Invocation(
            InvocationError::RollInputMismatch
        ))
    );
    assert_eq!(handler.called.get(), 0);
    assert_eq!(current, before);
    assert!(handler.draws.borrow().is_empty());
}

fn current_with_atomic_history(reaction: bool, with_history: bool) -> Checkpoint {
    waiting(reaction, |state| {
        if with_history {
            state.decisions.push(AcceptedDecision {
                operation: OperationId::from_bytes(&[98; 16]).unwrap(),
                revision: basis().revision,
                facts: vec![],
                draws: vec![],
                effects: vec![],
                source_policy: label("fixture-retained-policy"),
                semantic_output: Some("retained-outcome".into()),
            });
        }
    })
}

#[test]
fn requested_plus_foreign_decision_is_refused_by_current_offer_and_submit() {
    for reaction in [false, true] {
        for with_history in [false, true] {
            let current = current_with_atomic_history(reaction, with_history);
            let before = current.clone();
            let mut handler = SyntheticHandler::new();
            handler.append_foreign = true;
            let dependencies = [RuleDependency::PendingResolution(pending().id)];
            let offered = with_pipeline(
                &current,
                &current,
                &handler,
                &dependencies,
                |registry, context, selector| {
                    offered_current_commands(
                        &request(reaction),
                        context,
                        registry,
                        selector,
                        1024 * 1024,
                    )
                },
            );
            let input = GameInput::Game(request(reaction));
            let submitted = with_pipeline(
                &current,
                &current,
                &handler,
                &dependencies,
                |registry, context, selector| {
                    prepare_current_response(
                        without_draws(&input),
                        context,
                        registry,
                        selector,
                        1024 * 1024,
                    )
                },
            );
            let expected = ResponsePreparationError::Invocation(
                InvocationError::CandidateDecisionHistoryMismatch,
            );

            assert_eq!(offered, Err(expected.clone()));
            assert_eq!(submitted, Err(expected));
            assert_eq!(handler.called.get(), 2);
            assert!(handler.draws.borrow().iter().all(Vec::is_empty));
            assert_eq!(current, before);
        }
    }
}

#[test]
fn one_atomic_decision_preserves_history_through_current_offer_and_submit() {
    for reaction in [false, true] {
        let current = current_with_atomic_history(reaction, true);
        let before = current.clone();
        let handler = SyntheticHandler::new();
        let dependencies = [RuleDependency::PendingResolution(pending().id)];
        let offered = with_pipeline(
            &current,
            &current,
            &handler,
            &dependencies,
            |registry, context, selector| {
                offered_current_commands(
                    &request(reaction),
                    context,
                    registry,
                    selector,
                    1024 * 1024,
                )
            },
        )
        .unwrap();
        assert_eq!(offered.len(), 1);
        let candidate = with_pipeline(
            &current,
            &current,
            &handler,
            &dependencies,
            |registry, context, selector| {
                prepare_current_response(
                    without_draws(offered.first().unwrap()),
                    context,
                    registry,
                    selector,
                    1024 * 1024,
                )
            },
        )
        .unwrap();

        assert!(
            candidate
                .state()
                .decisions
                .starts_with(&current.state().decisions)
        );
        assert_eq!(
            candidate.state().decisions.len(),
            current.state().decisions.len() + 1
        );
        let appended = candidate.state().decisions.last().unwrap();
        assert_eq!(appended.operation, request(reaction).operation);
        assert_eq!(
            appended.revision,
            current.basis().revision.next_sequence().unwrap()
        );
        assert!(appended.draws.is_empty());
        assert_eq!(candidate.state().facts, current.state().facts);
        assert_eq!(candidate.state().draws, current.state().draws);
        assert_eq!(handler.called.get(), 2);
        assert!(handler.draws.borrow().iter().all(Vec::is_empty));
        assert_eq!(current, before);
    }
}
