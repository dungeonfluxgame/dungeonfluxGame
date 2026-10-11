#[path = "support/pending_fixtures.rs"]
mod pending_fixtures;
#[path = "support/plan_staging.rs"]
mod plan_fixtures;

use df_engine::command_entry::CommandRejection;
use df_engine::pending_resumption::ResumeError;
use df_engine::plan_staging::PlanHeadError;
use df_engine::semantic_candidate::SemanticResponseError;
use df_intent::candidate::CandidateError;
use df_intent::plan::PlanError;
use df_model::checkpoint::*;
use df_model::commands::CommandError;
use df_rules::InvocationError;
use df_rules::RulesCommandInput;
use df_rules::preconditions::{PreconditionError, PreconditionedRejection};
use df_types::{OperationId, RunId, SessionId};
use pending_fixtures::{
    FixtureRejection, content, entity, label, member, native_roll_draw, response, revision, waiting,
};
use plan_fixtures::*;

#[test]
fn exact_borrowed_head_and_draw_slice_reach_handler_while_tail_is_untouched() {
    for kind in 0..3 {
        let current = waiting(kind);
        let original = current.clone();
        let steps = two_steps(kind);
        let draws = if kind == 2 {
            vec![native_roll_draw(0)]
        } else {
            vec![]
        };
        let handler = handler(&current, &steps[0], &draws);
        let staged = invoke(
            &steps,
            &draws,
            &current,
            &current,
            &handler,
            admission(&current, &steps[0]),
        )
        .unwrap();
        assert!(std::ptr::eq(staged.first(), &steps[0]));
        assert!(std::ptr::eq(
            staged.requires_revalidation().as_ptr(),
            steps[1..].as_ptr()
        ));
        assert_eq!(staged.requires_revalidation(), &steps[1..]);
        assert_eq!(handler.command_pointer.get(), &steps[0] as *const GameInput);
        assert_eq!(handler.draw_pointer.get(), draws.as_ptr());
        assert_eq!(handler.calls.get(), 1);
        assert_eq!(staged.checkpoint(), &handler.candidate);
        assert_eq!(current, original);
        assert_eq!(staged.checkpoint().state().draws, draws);
        assert_eq!(staged.checkpoint().state().decisions.len(), 1);
        let (checkpoint, tail) = staged.into_parts();
        assert_eq!(checkpoint, handler.candidate);
        assert_eq!(tail, &steps[1..]);
    }
}

#[test]
fn tail_not_currently_offered_is_returned_without_approving_it() {
    let current = waiting(0);
    let mut steps = two_steps(0);
    let GameInput::Game(command) = &mut steps[1] else {
        unreachable!()
    };
    let GameCommand::SelectChoice { option, .. } = &mut command.command else {
        unreachable!()
    };
    *option = label("unoffered-tail");
    let handler = handler(&current, &steps[0], &[]);
    let staged = invoke(
        &steps,
        &[],
        &current,
        &current,
        &handler,
        admission(&current, &steps[0]),
    )
    .unwrap();
    assert_eq!(staged.requires_revalidation(), &steps[1..]);
    let next = staged.checkpoint();
    let tail_handler = plan_fixtures::handler(next, &steps[1], &[]);
    assert_eq!(
        invoke(
            staged.requires_revalidation(),
            &[],
            next,
            next,
            &tail_handler,
            admission(next, &steps[1]),
        )
        .err(),
        Some(PlanHeadError::Plan(PlanError::Candidate(
            CandidateError::Command(CommandError::UnofferedResponse)
        )))
    );
    assert_eq!(tail_handler.calls.get(), 0);
    assert_eq!(next.state().decisions.len(), 1);
}

#[test]
fn whole_list_duplicate_foreign_member_run_epoch_and_unsupported_action_refuse() {
    let current = waiting(0);
    let clean = two_steps(0);
    let handler = handler(&current, &clean[0], &[]);
    for case in 0..7 {
        let mut steps = clean.clone();
        let GameInput::Game(command) = &mut steps[1] else {
            unreachable!()
        };
        let expected = match case {
            0 => {
                command.operation = operation(&clean[0]);
                PlanError::DuplicateOperation
            }
            1 => {
                command.member = member(99);
                PlanError::MemberMismatch { index: 1 }
            }
            2 => {
                command.basis.run = RunId::from_bytes(&[99; 16]).unwrap();
                PlanError::ScopeMismatch { index: 1 }
            }
            3 => {
                command.basis.revision = revision(1, 8);
                PlanError::ScopeMismatch { index: 1 }
            }
            4 => {
                command.observed_revision = revision(1, 8);
                PlanError::ScopeMismatch { index: 1 }
            }
            5 => {
                command.basis.session = SessionId::from_bytes(&[99; 16]).unwrap();
                PlanError::ScopeMismatch { index: 1 }
            }
            _ => {
                command.command = GameCommand::ProposeAction {
                    actor: entity(4),
                    action: content(),
                    targets: vec![],
                    choices: vec![],
                };
                PlanError::UnsupportedStep { index: 1 }
            }
        };
        assert_eq!(
            invoke(
                &steps,
                &[],
                &current,
                &current,
                &handler,
                admission(&current, &clean[0]),
            )
            .err(),
            Some(PlanHeadError::Plan(expected))
        );
    }
    let unsupported_head = [response(3)];
    assert_eq!(
        invoke(
            &unsupported_head,
            &[],
            &current,
            &current,
            &handler,
            admission(&current, &clean[0]),
        )
        .err(),
        Some(PlanHeadError::Plan(PlanError::UnsupportedStep { index: 0 }))
    );
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn recorded_head_requires_lookup_before_another_handler_or_tail_admission() {
    let current = waiting(0);
    let steps = two_steps(0);
    let handler = handler(&current, &steps[0], &[]);
    let staged = invoke(
        &steps,
        &[],
        &current,
        &current,
        &handler,
        admission(&current, &steps[0]),
    )
    .unwrap();
    let next = staged.checkpoint();
    assert_eq!(
        invoke(
            &steps,
            &[],
            next,
            next,
            &handler,
            admission(next, &steps[0]),
        )
        .err(),
        Some(PlanHeadError::Plan(PlanError::LookupRequired {
            operation: operation(&steps[0]),
        }))
    );
    assert_eq!(handler.calls.get(), 1);
}

#[test]
fn bounds_and_native_generation_cancellation_and_operation_are_enforced() {
    let current = waiting(0);
    let steps = two_steps(0);
    let handler = handler(&current, &steps[0], &[]);
    for case in 0..9 {
        let mut trusted = admission(&current, &steps[0]);
        let expected = match case {
            0 => {
                trusted.plan.maximum_steps = 1;
                PlanHeadError::Plan(PlanError::Capacity)
            }
            1 => {
                trusted.plan.maximum_total_input_bytes = 1;
                PlanHeadError::Plan(PlanError::Capacity)
            }
            2 => {
                trusted.plan.maximum_comparisons = 0;
                PlanHeadError::Plan(PlanError::Capacity)
            }
            3 => {
                trusted.fence.current_generation += 1;
                PlanHeadError::Semantic(SemanticResponseError::Resume(ResumeError::StaleGeneration))
            }
            4 => {
                trusted.fence.cancel_before_admission = true;
                PlanHeadError::Semantic(SemanticResponseError::Resume(
                    ResumeError::CancelledBeforeAdmission,
                ))
            }
            5 => {
                trusted.maximum_bytes = 1;
                PlanHeadError::Semantic(SemanticResponseError::Resume(ResumeError::Capacity))
            }
            6 => {
                trusted.operation = OperationId::from_bytes(&[99; 16]).unwrap();
                PlanHeadError::Plan(PlanError::Candidate(CandidateError::OperationMismatch))
            }
            7 => {
                trusted.pins.content.content = label("different-native-content");
                PlanHeadError::Plan(PlanError::Candidate(CandidateError::Snapshot(
                    CheckpointError::ContentMismatch,
                )))
            }
            _ => {
                trusted.engine_command.maximum_retained_bytes = 1;
                PlanHeadError::Semantic(SemanticResponseError::Candidate(CandidateError::Command(
                    CommandError::Capacity,
                )))
            }
        };
        assert_eq!(
            invoke(&steps, &[], &current, &current, &handler, trusted).err(),
            Some(expected)
        );
    }
    assert_eq!(
        invoke(
            &[],
            &[],
            &current,
            &current,
            &handler,
            admission(&current, &steps[0]),
        )
        .err(),
        Some(PlanHeadError::Plan(PlanError::EmptyPlan))
    );
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn native_roll_and_registered_resource_refusals_reach_existing_typed_boundary() {
    let current = waiting(2);
    let steps = two_steps(2);
    let draws = [native_roll_draw(0)];
    let handler = handler(&current, &steps[0], &draws);
    let mut foreign_draws = draws.clone();
    foreign_draws[0].operation = OperationId::from_bytes(&[99; 16]).unwrap();
    for (supplied, expected) in [
        (&[][..], InvocationError::RollInputMismatch),
        (&foreign_draws[..], InvocationError::DrawOperationMismatch),
    ] {
        assert_eq!(
            invoke(
                &steps,
                supplied,
                &current,
                &current,
                &handler,
                admission(&current, &steps[0]),
            )
            .err(),
            Some(PlanHeadError::Semantic(SemanticResponseError::Resume(
                ResumeError::Command(CommandRejection::Invocation(expected))
            )))
        );
    }
    let mut changed = current.state().clone();
    changed.resources[0].value -= 1;
    let changed = rebuild(&current, changed);
    assert_eq!(
        invoke(
            &steps,
            &draws,
            &changed,
            &current,
            &handler,
            admission(&changed, &steps[0]),
        )
        .err(),
        Some(PlanHeadError::Semantic(SemanticResponseError::Resume(
            ResumeError::Command(CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Precondition(PreconditionError::StaleResource)
            )))
        )))
    );
    assert_eq!(handler.calls.get(), 0);
}

#[test]
fn missing_registered_selector_and_real_handler_refusal_cannot_become_a_plan_success() {
    let current = waiting(0);
    let steps = two_steps(0);
    let mut handler = handler(&current, &steps[0], &[]);
    let mut trusted = admission(&current, &steps[0]);
    trusted.selector = label("unregistered-selector");
    assert!(matches!(
        invoke(&steps, &[], &current, &current, &handler, trusted).err(),
        Some(PlanHeadError::Semantic(SemanticResponseError::Resume(
            ResumeError::Command(CommandRejection::Invocation(InvocationError::Dispatch(_)))
        )))
    ));
    assert_eq!(handler.calls.get(), 0);
    handler.refuse = true;
    assert_eq!(
        invoke(
            &steps,
            &[],
            &current,
            &current,
            &handler,
            admission(&current, &steps[0]),
        )
        .err(),
        Some(PlanHeadError::Semantic(SemanticResponseError::Resume(
            ResumeError::Command(CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Handler(FixtureRejection::Unsupported)
            )))
        )))
    );
    assert_eq!(handler.calls.get(), 1);
}

#[test]
fn plan_head_stages_same_candidate_as_existing_pending_entry() {
    use std::cell::Cell;
    use std::rc::Rc;

    let current = waiting(0);
    let steps = two_steps(0);
    let handler = handler(&current, &steps[0], &[]);
    let staged = invoke(
        &steps,
        &[],
        &current,
        &current,
        &handler,
        admission(&current, &steps[0]),
    )
    .unwrap();
    let existing = pending_fixtures::SuppliedHandler {
        candidate: handler.candidate.clone(),
        pins: current.pins().clone(),
        calls: Rc::new(Cell::new(0)),
        refuse: false,
        observed_draw_pointer: Rc::new(Cell::new(std::ptr::null())),
    };
    assert_eq!(
        pending_fixtures::invoke(
            RulesCommandInput {
                command: &steps[0],
                supplied_draws: &[],
            },
            &current,
            &current,
            &existing,
            pending_fixtures::fence(),
            BYTES,
        )
        .unwrap(),
        *staged.checkpoint()
    );
    assert_eq!(existing.calls.get(), 1);
}

#[cfg(not(target_arch = "wasm32"))]
mod durable_session {
    use super::*;
    use df_session::submission::{RepositoryError, SubmissionOutcome};
    use plan_fixtures::durable::*;

    #[test]
    fn actual_session_inbox_commits_heads_separately_against_new_native_checkpoint() {
        let (mut owner, database, steps) = setup(Failure::None);
        let original = owner.checkpoint().clone();
        let first = submit(&mut owner, &steps[0]);
        assert!(matches!(first, SubmissionOutcome::Confirmed(_)));
        let prefix = owner.checkpoint().clone();
        assert_eq!(database.borrow().returned_tail, steps[1..]);
        assert_eq!(submit(&mut owner, &steps[0]), first);
        database.borrow_mut().ephemeral_steps = steps[1..].to_vec();
        let second = submit(&mut owner, &steps[1]);
        assert!(matches!(second, SubmissionOutcome::Confirmed(_)));
        assert_eq!(submit(&mut owner, &steps[1]), second);
        let db = database.borrow();
        assert_eq!(db.observed_bases, vec![original.basis(), prefix.basis()]);
        assert_eq!(owner.checkpoint(), &db.checkpoint);
        assert_eq!(
            db.checkpoint.basis().revision,
            prefix.basis().revision.next_sequence().unwrap()
        );
        assert!(
            db.checkpoint
                .state()
                .decisions
                .starts_with(&prefix.state().decisions)
        );
        assert_eq!(db.checkpoint.state().decisions.len(), 2);
        assert!(db.returned_tail.is_empty());
        assert_eq!(
            (db.engines, db.handlers, db.commits, db.publications),
            (2, 2, 2, 2)
        );
        assert_eq!(db.ledger.len(), 2);
    }

    #[test]
    fn later_resource_or_window_refusal_preserves_already_committed_head() {
        for outcome in [Outcome::SpendResource, Outcome::CloseWindow] {
            let (mut owner, database, steps) = setup(Failure::None);
            let original = owner.checkpoint().clone();
            database.borrow_mut().outcome = outcome;
            assert!(matches!(
                submit(&mut owner, &steps[0]),
                SubmissionOutcome::Confirmed(_)
            ));
            let prefix = owner.checkpoint().clone();
            {
                let mut db = database.borrow_mut();
                db.ephemeral_steps = db.returned_tail.clone();
                db.outcome = Outcome::Preserve;
                if outcome == Outcome::SpendResource {
                    // The source's previously prepared resource expectation is now invalid.
                    db.prepared = Some(original.clone());
                }
            }
            assert_eq!(
                submit(&mut owner, &steps[1]),
                SubmissionOutcome::Refused(RepositoryError::InvalidCandidate)
            );
            assert_eq!(owner.checkpoint(), &prefix);
            let db = database.borrow();
            assert_eq!(db.checkpoint, prefix);
            assert_eq!(db.observed_bases, vec![original.basis(), prefix.basis()]);
            assert_eq!(
                (db.engines, db.handlers, db.commits, db.publications),
                (2, 1, 1, 1)
            );
            assert_eq!(db.ledger.len(), 1);
            assert_eq!(db.checkpoint.state().decisions.len(), 1);
            let expected = if outcome == Outcome::SpendResource {
                PlanHeadError::Semantic(SemanticResponseError::Resume(ResumeError::Command(
                    CommandRejection::Invocation(InvocationError::Handler(
                        PreconditionedRejection::Precondition(PreconditionError::StaleResource),
                    )),
                )))
            } else {
                PlanHeadError::Plan(PlanError::Candidate(CandidateError::Command(
                    CommandError::StaleWindow,
                )))
            };
            assert_eq!(db.refusals, vec![expected]);
        }
    }

    #[test]
    fn known_failed_commit_has_no_publication_and_same_head_retry_uses_same_operation() {
        let (mut owner, database, steps) = setup(Failure::BeforeCommit);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &steps[0]),
            SubmissionOutcome::Refused(RepositoryError::Unavailable)
        );
        assert_eq!(owner.checkpoint(), &original);
        assert!(!owner.has_uncertain_operation());
        assert_eq!(database.borrow().checkpoint, original);
        assert_eq!(database.borrow().publications, 0);
        assert!(database.borrow().ledger.is_empty());
        database.borrow_mut().failure = Failure::None;
        assert!(matches!(
            submit(&mut owner, &steps[0]),
            SubmissionOutcome::Confirmed(_)
        ));
        let db = database.borrow();
        assert_eq!(db.observed_bases, vec![original.basis(), original.basis()]);
        assert_eq!(
            (db.engines, db.handlers, db.commits, db.publications),
            (2, 2, 2, 1)
        );
        assert_eq!(db.ledger.len(), 1);
        assert_eq!(
            db.checkpoint.state().decisions[0].operation,
            operation(&steps[0])
        );
    }

    #[test]
    fn lost_ack_fences_tail_until_exact_lookup_then_new_head_uses_reloaded_checkpoint() {
        let (mut owner, database, steps) = setup(Failure::LostAck);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &steps[0]),
            SubmissionOutcome::LookupRequired
        );
        assert!(owner.has_uncertain_operation());
        assert_eq!(owner.checkpoint(), &original);
        database.borrow_mut().ephemeral_steps = steps[1..].to_vec();
        assert_eq!(
            submit(&mut owner, &steps[1]),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(database.borrow().engines, 1);
        assert!(matches!(
            submit(&mut owner, &steps[0]),
            SubmissionOutcome::Confirmed(_)
        ));
        let prefix = owner.checkpoint().clone();
        assert_eq!(prefix, database.borrow().checkpoint);
        assert!(!owner.has_uncertain_operation());
        assert_eq!(database.borrow().handlers, 1);
        assert_eq!(database.borrow().publications, 0);
        database.borrow_mut().failure = Failure::None;
        assert!(matches!(
            submit(&mut owner, &steps[1]),
            SubmissionOutcome::Confirmed(_)
        ));
        let db = database.borrow();
        assert_eq!(db.observed_bases, vec![original.basis(), prefix.basis()]);
        assert_eq!(
            (db.engines, db.handlers, db.commits, db.publications),
            (2, 2, 2, 1)
        );
        assert_eq!(db.ledger.len(), 2);
        assert!(
            db.checkpoint
                .state()
                .decisions
                .starts_with(&prefix.state().decisions)
        );
    }

    #[test]
    fn unknown_not_recorded_never_replays_head_or_executes_tail() {
        let (mut owner, database, steps) = setup(Failure::UnknownNotCommitted);
        let original = owner.checkpoint().clone();
        assert_eq!(
            submit(&mut owner, &steps[0]),
            SubmissionOutcome::LookupRequired
        );
        database.borrow_mut().failure = Failure::None;
        assert_eq!(
            submit(&mut owner, &steps[0]),
            SubmissionOutcome::LookupRequired
        );
        database.borrow_mut().ephemeral_steps = steps[1..].to_vec();
        assert_eq!(
            submit(&mut owner, &steps[1]),
            SubmissionOutcome::LookupRequired
        );
        assert_eq!(owner.checkpoint(), &original);
        assert!(owner.has_uncertain_operation());
        let db = database.borrow();
        assert_eq!(db.checkpoint, original);
        assert_eq!(
            (db.engines, db.handlers, db.commits, db.publications),
            (1, 1, 1, 0)
        );
        assert!(db.ledger.is_empty());
    }

    #[test]
    fn committed_head_lookup_checks_exact_input_and_authenticated_principal() {
        let (mut owner, database, steps) = setup(Failure::None);
        assert!(matches!(
            submit(&mut owner, &steps[0]),
            SubmissionOutcome::Confirmed(_)
        ));
        let prefix = owner.checkpoint().clone();
        let mut changed = steps[0].clone();
        let GameInput::Game(command) = &mut changed else {
            unreachable!()
        };
        command.command = GameCommand::Speak {
            speaker: entity(4),
            text: "different exact input".to_owned(),
            conversation: None,
        };
        assert_eq!(
            submit(&mut owner, &changed),
            SubmissionOutcome::OperationConflict
        );
        assert_eq!(
            submit_scope(
                &mut owner,
                Scope {
                    input: steps[0].clone(),
                    principal: member(99),
                },
            ),
            SubmissionOutcome::Refused(RepositoryError::Unauthorized)
        );
        assert_eq!(owner.checkpoint(), &prefix);
        let db = database.borrow();
        assert_eq!(
            (db.engines, db.handlers, db.commits, db.publications),
            (1, 1, 1, 1)
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[path = "compound_discretionary_contract.rs"]
mod compound_discretionary_contract;
