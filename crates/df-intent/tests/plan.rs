use df_intent::candidate::{CandidateError, CandidateOwner};
use df_intent::plan::{PlanAdmission, PlanError, PlanLimits, validate_plan_steps};
use df_model::checkpoint::*;
use df_model::commands::{CommandError, CommandLimits};
use df_types::OperationId;

#[path = "support/candidate_fixture.rs"]
mod candidate_fixture;
use candidate_fixture::*;

fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}

fn bounds() -> PlanLimits {
    PlanLimits {
        maximum_steps: 4,
        maximum_total_input_bytes: 8192,
        maximum_comparisons: 128,
        commands: CommandLimits {
            maximum_records: 32,
            maximum_text_bytes: 64,
            maximum_retained_bytes: 8192,
        },
    }
}

fn response(kind: u8, id: u8) -> GameInput {
    let pending = pending();
    let command = match kind {
        0 => GameCommand::SelectChoice {
            resolution: pending.id,
            window: pending.window.id,
            offer: label("fixture-offer-1"),
            option: label("fixture-option-1"),
        },
        1 => GameCommand::SelectReaction {
            resolution: pending.id,
            window: pending.window.id,
            offer: label("fixture-offer-1"),
            option: label("fixture-option-1"),
        },
        2 => GameCommand::SubmitRoll {
            resolution: pending.id,
            window: pending.window.id,
        },
        _ => unreachable!(),
    };
    GameInput::Game(CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: operation(id),
        member: member(3),
        command,
    })
}

fn waiting(kind: u8) -> Checkpoint {
    let mut supplied = state();
    supplied.facts.push(fact(7, 0));
    supplied.members.push(MembershipLink {
        member: member(5),
        character: None,
    });
    let mut resolution = pending();
    let offered = vec![OfferedResponse {
        participant: member(3),
        offer: label("fixture-offer-1"),
        options: vec![label("fixture-option-1")],
        source: rule(),
    }];
    resolution.next = match kind {
        0 => PendingInput::Choice { remaining: offered },
        1 => PendingInput::Reaction { remaining: offered },
        2 => PendingInput::Roll {
            participant: member(3),
            sides: vec![20],
            source: rule(),
        },
        _ => unreachable!(),
    };
    supplied.pending.push(resolution);
    checkpoint(supplied).unwrap()
}

fn owner(current: &Checkpoint) -> CandidateOwner<'_> {
    CandidateOwner {
        basis: current.basis(),
        pins: current.pins(),
        member: member(3),
        operation: operation(20),
    }
}

fn validate<'a>(
    steps: &'a [GameInput],
    current: &'a Checkpoint,
    limits: PlanLimits,
) -> Result<PlanAdmission<'a>, PlanError> {
    validate_plan_steps(
        steps,
        current,
        owner(current),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        limits,
    )
}

fn command(input: &mut GameInput) -> &mut CommandInput {
    let GameInput::Game(command) = input else {
        unreachable!()
    };
    command
}

#[test]
fn admits_first_current_choice_reaction_or_roll_and_retains_exact_unvalidated_tail() {
    for kind in 0..3 {
        let current = waiting(kind);
        let before = current.clone();
        let steps = [response(kind, 20), response((kind + 1) % 3, 21)];
        let before_steps = steps.clone();
        let admitted = validate(&steps, &current, bounds()).unwrap();
        let GameInput::Game(first) = &steps[0] else {
            unreachable!()
        };
        assert!(std::ptr::eq(admitted.first(), first));
        assert!(std::ptr::eq(
            admitted.requires_revalidation().as_ptr(),
            steps[1..].as_ptr()
        ));
        assert_eq!(admitted.requires_revalidation(), &steps[1..]);
        assert_eq!(admitted.basis(), current.basis());
        assert!(std::ptr::eq(admitted.pins(), current.pins()));
        // The second request is a different pending kind and cannot be admitted now.
        assert_eq!(
            validate(&steps[1..], &current, bounds()).unwrap_err(),
            PlanError::Candidate(CandidateError::OperationMismatch)
        );
        let mut tail_owner = owner(&current);
        tail_owner.operation = operation(21);
        assert_eq!(
            validate_plan_steps(
                &steps[1..],
                &current,
                tail_owner,
                ReferenceInventory {
                    rules: &[rule()],
                    content: &[content()],
                    resources: &[],
                    assets: &[],
                },
                bounds()
            )
            .unwrap_err(),
            PlanError::Candidate(CandidateError::Command(CommandError::WrongPendingKind))
        );
        assert_eq!(current, before);
        assert_eq!(steps, before_steps);
        assert!(current.state().draws.is_empty());
        assert!(current.state().decisions.is_empty());
    }
}

#[test]
fn tail_is_readmitted_only_against_a_new_current_checkpoint_offer() {
    let initial = waiting(1);
    let steps = [response(1, 20), response(0, 21)];
    let admitted = validate(&steps, &initial, bounds()).unwrap();
    let mut later = waiting(0).state().clone();
    let next_basis = Basis {
        revision: basis().revision.next_sequence().unwrap(),
        ..basis()
    };
    later.pending[0].basis = next_basis;
    let current = checkpoint_at_basis(later, &[rule()], next_basis, pins()).unwrap();
    let before = current.clone();
    let mut later_owner = owner(&current);
    later_owner.operation = operation(21);
    let next = validate_plan_steps(
        admitted.requires_revalidation(),
        &current,
        later_owner,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        bounds(),
    )
    .unwrap();
    assert_eq!(next.first().operation, operation(21));
    assert!(next.requires_revalidation().is_empty());
    assert_eq!(next.basis(), next_basis);
    assert_eq!(current, before);
    assert!(current.state().draws.is_empty());
}

#[test]
fn an_unoffered_first_response_refuses_instead_of_skipping_to_a_later_step() {
    let current = waiting(1);
    let mut first = response(1, 20);
    let GameCommand::SelectReaction { option, .. } = &mut command(&mut first).command else {
        unreachable!()
    };
    *option = label("unoffered");
    let steps = [first, response(1, 21)];
    assert_eq!(
        validate(&steps, &current, bounds()).unwrap_err(),
        PlanError::Candidate(CandidateError::Command(CommandError::UnofferedResponse))
    );
}

#[test]
fn tail_variant_and_membership_cannot_bypass_structural_plan_admission() {
    let current = waiting(1);
    let mut tail = response(1, 21);
    command(&mut tail).command = GameCommand::Speak {
        speaker: entity(4),
        text: "private unsupported speech".into(),
        conversation: None,
    };
    assert_eq!(
        validate(&[response(1, 20), tail], &current, bounds()).unwrap_err(),
        PlanError::UnsupportedStep { index: 1 }
    );
    let mut tail = response(1, 21);
    command(&mut tail).member = member(5);
    assert_eq!(
        validate(&[response(1, 20), tail], &current, bounds()).unwrap_err(),
        PlanError::MemberMismatch { index: 1 }
    );
}

#[test]
fn unsupported_host_and_general_action_steps_refuse_even_in_the_tail() {
    let current = waiting(1);
    let host = GameInput::Host(HostInput {
        basis: basis(),
        operation: operation(21),
        host: member(3),
        command: HostCommand::RequestCheckpoint,
    });
    assert_eq!(
        validate(&[response(1, 20), host], &current, bounds()).unwrap_err(),
        PlanError::UnsupportedStep { index: 1 }
    );
    let mut action = response(1, 20);
    command(&mut action).command = GameCommand::ProposeAction {
        actor: entity(4),
        action: content(),
        targets: vec![],
        choices: vec![],
    };
    assert_eq!(
        validate(&[action], &current, bounds()).unwrap_err(),
        PlanError::UnsupportedStep { index: 0 }
    );
}

#[test]
fn mixed_session_run_epoch_or_future_observations_refuse_before_any_admission() {
    let current = waiting(1);
    let changes = [
        Basis {
            session: df_types::SessionId::from_bytes(&[99; 16]).unwrap(),
            ..basis()
        },
        Basis {
            run: df_types::RunId::from_bytes(&[99; 16]).unwrap(),
            ..basis()
        },
        Basis {
            revision: revision(1, 100),
            ..basis()
        },
        Basis {
            revision: revision(2, 9),
            ..basis()
        },
    ];
    for change in changes {
        let mut tail = response(1, 21);
        command(&mut tail).basis = change;
        assert_eq!(
            validate(&[response(1, 20), tail], &current, bounds()).unwrap_err(),
            PlanError::ScopeMismatch { index: 1 }
        );
    }
    for change in [revision(1, 8), revision(2, 9)] {
        let mut tail = response(1, 21);
        command(&mut tail).observed_revision = change;
        assert_eq!(
            validate(&[response(1, 20), tail], &current, bounds()).unwrap_err(),
            PlanError::ScopeMismatch { index: 1 }
        );
    }
}

#[test]
fn native_basis_and_complete_pins_are_independent_of_older_client_observations() {
    let current = waiting(1);
    let mut steps = [response(1, 20), response(0, 21)];
    for input in &mut steps {
        command(input).basis.revision = revision(2, 7);
        command(input).observed_revision = revision(2, 7);
    }
    assert!(validate(&steps, &current, bounds()).is_ok());
    let mut stale = owner(&current);
    stale.basis.revision = revision(2, 7);
    assert_eq!(
        validate_plan_steps(
            &steps,
            &current,
            stale,
            ReferenceInventory {
                rules: &[rule()],
                content: &[content()],
                resources: &[],
                assets: &[],
            },
            bounds()
        )
        .unwrap_err(),
        PlanError::Candidate(CandidateError::Snapshot(CheckpointError::StaleBasis))
    );
    let mut changed = pins();
    changed.rules.handler_digest = ContentDigest([99; 32]);
    let stale = CandidateOwner {
        pins: &changed,
        ..owner(&current)
    };
    assert_eq!(
        validate_plan_steps(
            &steps,
            &current,
            stale,
            ReferenceInventory {
                rules: &[rule()],
                content: &[content()],
                resources: &[],
                assets: &[],
            },
            bounds()
        )
        .unwrap_err(),
        PlanError::Candidate(CandidateError::Snapshot(CheckpointError::RulesMismatch))
    );
}

#[test]
fn duplicate_operation_identity_refuses_even_when_request_meaning_differs() {
    let current = waiting(1);
    assert_eq!(
        validate(&[response(1, 20), response(0, 20)], &current, bounds()).unwrap_err(),
        PlanError::DuplicateOperation
    );
}

#[test]
fn recorded_first_or_later_operation_requires_native_lookup_without_replay() {
    for recorded in [20, 21] {
        let mut supplied = waiting(1).state().clone();
        supplied.decisions.push(AcceptedDecision {
            operation: operation(recorded),
            revision: basis().revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: label("fixture-original-policy"),
            semantic_output: Some("private original".into()),
        });
        let current = checkpoint(supplied).unwrap();
        let before = current.clone();
        assert_eq!(
            validate(&[response(1, 20), response(0, 21)], &current, bounds()).unwrap_err(),
            PlanError::LookupRequired {
                operation: operation(recorded)
            }
        );
        assert_eq!(current, before);
    }
}

#[test]
fn durable_unknown_completed_failed_or_cancelled_work_is_not_reissued() {
    for status in [
        DurableStatus::Pending,
        DurableStatus::SentUnknown,
        DurableStatus::Completed,
        DurableStatus::Failed,
        DurableStatus::Cancelled,
    ] {
        let mut supplied = waiting(1).state().clone();
        supplied.intents.push(DurableIntent {
            id: EffectId::from_bytes(&[30; 16]).unwrap(),
            basis: basis(),
            operation: operation(21),
            slot: 0,
            kind: EffectKind::PublishPresentation,
            job: None,
            timer: None,
            generation: 17,
            status,
            definition: content(),
        });
        let current = checkpoint(supplied).unwrap();
        let before = current.clone();
        assert_eq!(
            validate(&[response(1, 20), response(0, 21)], &current, bounds()).unwrap_err(),
            PlanError::LookupRequired {
                operation: operation(21)
            }
        );
        assert_eq!(current, before);
        assert_eq!(current.state().intents[0].generation, 17);
        assert_eq!(current.state().intents[0].status, status);
    }
}

#[test]
fn empty_count_byte_and_comparison_boundaries_are_explicit() {
    let current = waiting(1);
    let steps = [response(1, 20), response(0, 21)];
    assert_eq!(
        validate(&[], &current, bounds()).unwrap_err(),
        PlanError::EmptyPlan
    );
    let retained_bytes = steps
        .iter()
        .map(|input| input.retained_bytes().unwrap())
        .sum::<usize>();
    for limits in [
        PlanLimits {
            maximum_steps: 1,
            ..bounds()
        },
        PlanLimits {
            maximum_steps: 0,
            ..bounds()
        },
        PlanLimits {
            maximum_total_input_bytes: retained_bytes - 1,
            ..bounds()
        },
        PlanLimits {
            maximum_total_input_bytes: 0,
            ..bounds()
        },
        PlanLimits {
            maximum_comparisons: 0,
            ..bounds()
        },
    ] {
        assert_eq!(
            validate(&steps, &current, limits).unwrap_err(),
            PlanError::Capacity
        );
    }
    assert!(
        validate(
            &steps,
            &current,
            PlanLimits {
                maximum_steps: 2,
                maximum_total_input_bytes: retained_bytes,
                maximum_comparisons: 1,
                ..bounds()
            }
        )
        .is_ok()
    );
    let mut supplied = current.state().clone();
    supplied.decisions.push(AcceptedDecision {
        operation: operation(90),
        revision: basis().revision,
        facts: vec![],
        draws: vec![],
        effects: vec![],
        source_policy: label("fixture-other-policy"),
        semantic_output: None,
    });
    let current = checkpoint(supplied).unwrap();
    assert_eq!(
        validate(
            &steps,
            &current,
            PlanLimits {
                maximum_comparisons: 2,
                ..bounds()
            }
        )
        .unwrap_err(),
        PlanError::Capacity
    );
    assert!(
        validate(
            &steps,
            &current,
            PlanLimits {
                maximum_comparisons: 3,
                ..bounds()
            }
        )
        .is_ok()
    );
}

#[test]
fn admission_debug_contains_counts_without_private_options_or_source_payloads() {
    let current = waiting(1);
    let steps = [response(1, 20), response(0, 21)];
    let admitted = validate(&steps, &current, bounds()).unwrap();
    let debug = format!("{admitted:?}");
    assert!(debug.contains("revalidation_count: 1"));
    for private in ["fixture-option-1", "fixture-offer-1", "fixture-source-1"] {
        assert!(!debug.contains(private));
    }
}
