#[path = "support/pending_fixtures.rs"]
mod pending_fixtures;
use df_engine::command_entry::CommandRejection;
use df_engine::pending_resumption::{ResumeError, ResumeFence};
use df_model::checkpoint::*;
use df_model::commands::CommandError;
use df_rules::current_responses::ResponseError;
use df_rules::preconditions::{PreconditionError, PreconditionedRejection};
use df_rules::{InvocationError, RulesCommandInput};
use df_types::OperationId;
use pending_fixtures::*;
use std::cell::Cell;
use std::rc::Rc;

const CHECKPOINT_BYTES: usize = 1024 * 1024;
fn handler(current: &Checkpoint, input: RulesCommandInput<'_>) -> SuppliedHandler {
    SuppliedHandler {
        candidate: supplied_candidate(current, input),
        pins: current.pins().clone(),
        calls: Rc::new(Cell::new(0)),
        refuse: false,
        observed_draw_pointer: Rc::new(Cell::new(std::ptr::null())),
    }
}

#[test]
fn all_canonical_pending_kinds_use_exact_registered_handler_once() {
    for kind in 0..4 {
        let mut current = waiting(kind);
        let input = response(kind);
        let supplied = if kind == 2 {
            let mut state = current.state().clone();
            let mut prior = native_roll_draw(0);
            prior.operation = OperationId::from_bytes(&[30; 16]).unwrap();
            state.draws.push(prior.clone());
            state.pending[0]
                .draw_ordinals
                .push((prior.operation, prior.ordinal));
            let prior_fact = FactId::from_bytes(&[39; 16]).unwrap();
            state.facts.push(GameFact {
                id: prior_fact,
                revision: current.basis().revision,
                operation: prior.operation,
                ordinal: 0,
                cause: None,
                audience: AudienceScope::Shared,
                value: FactValue::DrawAccepted {
                    operation: prior.operation,
                    ordinal: 0,
                },
            });
            state.decisions.push(AcceptedDecision {
                operation: prior.operation,
                revision: current.basis().revision,
                facts: vec![prior_fact],
                draws: vec![0],
                effects: vec![],
                source_policy: label("prior-source"),
                semantic_output: None,
            });
            current = checkpoint(state).unwrap();
            vec![native_roll_draw(0)]
        } else {
            vec![]
        };
        let original = current.clone();
        let envelope = RulesCommandInput {
            command: &input,
            supplied_draws: &supplied,
        };
        let handler = handler(&current, envelope);
        let result = invoke(
            envelope,
            &current,
            &current,
            &handler,
            fence(),
            CHECKPOINT_BYTES,
        )
        .unwrap();
        assert_eq!(result, handler.candidate);
        assert_eq!(handler.calls.get(), 1);
        assert!(std::ptr::eq(
            handler.observed_draw_pointer.get(),
            supplied.as_ptr()
        ));
        assert_eq!(current, original);
        assert_eq!(
            result.state().pending[0].spent,
            current.state().pending[0].spent
        );
        assert!(result.state().draws.starts_with(&current.state().draws));
        assert_eq!(
            &result.state().draws[current.state().draws.len()..],
            supplied.as_slice()
        );
        if kind == 2 {
            assert_eq!(result.state().decisions.last().unwrap().draws, vec![0]);
            assert!(
                result
                    .state()
                    .decisions
                    .starts_with(&current.state().decisions)
            );
            assert_eq!(
                invoke(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &[]
                    },
                    &current,
                    &current,
                    &handler,
                    fence(),
                    CHECKPOINT_BYTES
                ),
                Err(ResumeError::Command(CommandRejection::Invocation(
                    InvocationError::RollInputMismatch
                )))
            );
            let mut foreign = supplied.clone();
            foreign[0].operation = OperationId::from_bytes(&[99; 16]).unwrap();
            assert_eq!(
                invoke(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &foreign
                    },
                    &current,
                    &current,
                    &handler,
                    fence(),
                    CHECKPOINT_BYTES
                ),
                Err(ResumeError::Command(CommandRejection::Invocation(
                    InvocationError::DrawOperationMismatch
                )))
            );
            let mut invalid = supplied.clone();
            invalid[0].value = 0;
            assert_eq!(
                invoke(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &invalid
                    },
                    &current,
                    &current,
                    &handler,
                    fence(),
                    CHECKPOINT_BYTES
                ),
                Err(ResumeError::Command(CommandRejection::Invocation(
                    InvocationError::InvalidDrawInput
                )))
            );
            let mut foreign_window = supplied.clone();
            foreign_window[0].window = WindowId::from_bytes(&[99; 16]).unwrap();
            assert_eq!(
                invoke(
                    RulesCommandInput {
                        command: &input,
                        supplied_draws: &foreign_window
                    },
                    &current,
                    &current,
                    &handler,
                    fence(),
                    CHECKPOINT_BYTES
                ),
                Err(ResumeError::Command(CommandRejection::Invocation(
                    InvocationError::RollInputMismatch
                )))
            );
            assert_eq!(
                invoke(
                    envelope,
                    &result,
                    &result,
                    &handler,
                    fence(),
                    CHECKPOINT_BYTES
                ),
                Err(ResumeError::Command(CommandRejection::Invocation(
                    InvocationError::AlreadyAccepted
                )))
            );
            assert_eq!(handler.calls.get(), 1);
        }
    }
}

#[test]
fn stale_generation_cancel_and_bounds_refuse_before_handler_invocation() {
    let current = waiting(1);
    let input = response(1);
    let handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    for (owner_fence, bytes, expected) in [
        (
            ResumeFence {
                current_generation: 4,
                ..fence()
            },
            CHECKPOINT_BYTES,
            ResumeError::StaleGeneration,
        ),
        (
            ResumeFence {
                current_generation: 0,
                admitted_generation: 0,
                ..fence()
            },
            CHECKPOINT_BYTES,
            ResumeError::StaleGeneration,
        ),
        (
            ResumeFence {
                cancel_before_admission: true,
                ..fence()
            },
            CHECKPOINT_BYTES,
            ResumeError::CancelledBeforeAdmission,
        ),
        (fence(), 1, ResumeError::Capacity),
    ] {
        assert_eq!(
            invoke(
                RulesCommandInput {
                    command: &input,
                    supplied_draws: &[]
                },
                &current,
                &current,
                &handler,
                owner_fence,
                bytes
            ),
            Err(expected)
        );
    }
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn shared_resource_dependencies_reject_changed_current_basis_before_handler() {
    let prepared = waiting(1);
    let mut changed = prepared.state().clone();
    changed.resources[0].value -= 1;
    let mut current_basis = prepared.basis();
    current_basis.revision = current_basis.revision.next_sequence().unwrap();
    for pending in &mut changed.pending {
        pending.basis = current_basis;
    }
    let current = Checkpoint::new(
        CHECKPOINT_SCHEMA,
        current_basis,
        prepared.pins().clone(),
        changed,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &resource_constraints(),
            assets: &[],
        },
        limits(),
    )
    .unwrap();
    let input = response(1);
    let handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    assert_eq!(
        invoke(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current,
            &prepared,
            &handler,
            fence(),
            CHECKPOINT_BYTES
        ),
        Err(ResumeError::Command(CommandRejection::Invocation(
            InvocationError::Handler(PreconditionedRejection::Precondition(
                PreconditionError::StaleResource
            ))
        )))
    );
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn shared_source_resolver_refuses_replaced_window_unoffered_response_and_wrong_kind() {
    let current = waiting(1);
    let mut input = response(1);
    if let GameInput::Game(command) = &mut input
        && let GameCommand::SelectReaction { option, .. } = &mut command.command
    {
        *option = label("fixture-unoffered");
    }
    let handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    assert_eq!(
        invoke(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current,
            &current,
            &handler,
            fence(),
            CHECKPOINT_BYTES
        ),
        Err(ResumeError::Response(ResponseError::Admission(
            CommandError::UnofferedResponse
        )))
    );
    let input = response(0);
    assert_eq!(
        invoke(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current,
            &current,
            &handler,
            fence(),
            CHECKPOINT_BYTES
        ),
        Err(ResumeError::Response(ResponseError::Admission(
            CommandError::WrongPendingKind
        )))
    );
    let mut changed = current.state().clone();
    changed.pending.clear();
    let replaced = checkpoint(changed).unwrap();
    let input = response(1);
    assert_eq!(
        invoke(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &replaced,
            &replaced,
            &handler,
            fence(),
            CHECKPOINT_BYTES
        ),
        Err(ResumeError::Response(ResponseError::Admission(
            CommandError::StaleWindow
        )))
    );
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn source_handler_refusal_remains_typed_without_candidate_or_input_mutation() {
    let current = waiting(1);
    let original = current.clone();
    let input = response(1);
    let mut handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    handler.refuse = true;
    assert_eq!(
        invoke(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current,
            &current,
            &handler,
            fence(),
            CHECKPOINT_BYTES
        ),
        Err(ResumeError::Command(CommandRejection::Invocation(
            InvocationError::Handler(PreconditionedRejection::Handler(
                FixtureRejection::Unsupported
            ))
        )))
    );
    assert_eq!(handler.calls.get(), 1);
    assert_eq!(current, original);
}

#[test]
fn retained_ordered_draws_choices_spends_rulings_and_offered_decline_are_preserved() {
    let mut state = waiting(3).state().clone();
    let prior = OperationId::from_bytes(&[30; 16]).unwrap();
    state.draws.push(ActualDraw {
        operation: prior,
        ordinal: 0,
        resolution: state.pending[0].id,
        window: state.pending[0].window.id,
        sides: 20,
        value: 11,
        source: rule(),
    });
    state.pending[0].draw_ordinals.push((prior, 0));
    state.pending[0].choices.push(AcceptedChoice {
        participant: member(3),
        offer: label("prior-offer"),
        selected: label("prior-option"),
        source: rule(),
    });
    state.pending[0].rulings.push(ScopedRuling {
        adjudicator: member(3),
        selected: label("prior-ruling"),
        source: rule(),
        audience: AudienceScope::Shared,
    });
    let PendingInput::Ruling { permitted, .. } = &mut state.pending[0].next else {
        unreachable!()
    };
    permitted[0].options.push(label("fixture-decline"));
    let current = checkpoint(state).unwrap();
    let original = current.clone();
    let mut input = response(3);
    if let GameInput::Host(command) = &mut input
        && let HostCommand::ResolveRuling { option, .. } = &mut command.command
    {
        *option = label("fixture-decline");
    }
    let handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    let result = invoke(
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
        &current,
        &current,
        &handler,
        fence(),
        CHECKPOINT_BYTES,
    )
    .unwrap();
    assert_eq!(result.state().draws, current.state().draws);
    assert_eq!(
        result.state().pending[0].choices,
        current.state().pending[0].choices
    );
    assert_eq!(
        result.state().pending[0].rulings,
        current.state().pending[0].rulings
    );
    assert_eq!(
        result.state().pending[0].spent,
        current.state().pending[0].spent
    );
    assert_eq!(current, original);
}

#[test]
fn exact_source_handler_pins_are_required_before_any_mechanics() {
    let current = waiting(1);
    let input = response(1);
    let mut handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    handler.pins.rules.handler_digest = ContentDigest([99; 32]);
    assert_eq!(
        invoke(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current,
            &current,
            &handler,
            fence(),
            CHECKPOINT_BYTES
        ),
        Err(ResumeError::Command(CommandRejection::Invocation(
            InvocationError::HandlerRulesMismatch
        )))
    );
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn surviving_older_observation_is_admitted_but_retired_epoch_is_refused() {
    let current = waiting(1);
    let mut input = response(1);
    if let GameInput::Game(command) = &mut input {
        command.basis.revision = revision(2, 7);
        command.observed_revision = revision(2, 7);
    }
    let handler = handler(
        &current,
        RulesCommandInput {
            command: &input,
            supplied_draws: &[],
        },
    );
    assert!(
        invoke(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current,
            &current,
            &handler,
            fence(),
            CHECKPOINT_BYTES
        )
        .is_ok()
    );
    if let GameInput::Game(command) = &mut input {
        command.basis.revision = revision(1, 7);
    }
    assert_eq!(
        invoke(
            RulesCommandInput {
                command: &input,
                supplied_draws: &[]
            },
            &current,
            &current,
            &handler,
            fence(),
            CHECKPOINT_BYTES
        ),
        Err(ResumeError::Command(CommandRejection::Structural(
            CommandError::StaleRevision
        )))
    );
    assert_eq!(handler.calls.get(), 1);
}
