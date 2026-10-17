//! Whole current Intent boundary contract; no new public or persistent shapes.
use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_intent::candidate::{
    CandidateError, CandidateOwner, CandidatePreparationError, prepare_semantic_candidate,
    validate_semantic_candidate,
};
use df_intent::dialogue::{DialogueError, DialogueLimits, classify_dialogue, confirm_final_input};
use df_intent::plan::{PlanError, PlanLimits, validate_plan_steps};
use df_intent::targets::{TargetResolutionError, TargetResolutionLimits, resolve_target};
use df_model::affordance::{Affordance, AffordanceSet, TargetResolution, TargetSelection};
use df_model::checkpoint::*;
use df_model::commands::{CommandError, CommandLimits};
use df_model::intent::{DiscourseContext, InputFinality, IntentDisposition, RawInputRef};
use df_rules::current_responses::ResponsePreparationError;
use df_rules::preconditions::{
    CurrentRuleContext, PreconditionError, PreconditionLimits, PreconditionedCommandHandler,
    PreconditionedRejection, RuleDependency, RulePreconditions,
};
use df_rules::{
    DispatchError, DispatchRegistry, HandlerRegistration, InvocationError, RulesCommandHandler,
    RulesCommandInput,
};
use df_types::OperationId;
use std::cell::Cell;

#[path = "../src/partial_progress.rs"]
#[allow(dead_code)]
mod partial_progress;

use partial_progress::candidate_fixture::*;

fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).unwrap()
}

fn command_limits() -> CommandLimits {
    CommandLimits {
        maximum_records: 32,
        maximum_text_bytes: 64,
        maximum_retained_bytes: 8192,
    }
}

fn plan_limits() -> PlanLimits {
    PlanLimits {
        maximum_steps: 3,
        maximum_total_input_bytes: 8192,
        maximum_comparisons: 1024,
        commands: command_limits(),
    }
}

fn owner(current: &Checkpoint) -> CandidateOwner<'_> {
    CandidateOwner {
        basis: current.basis(),
        pins: current.pins(),
        member: member(3),
        operation: operation(31),
    }
}

fn response(kind: u8, request: u8) -> GameInput {
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
        operation: operation(request),
        member: member(3),
        command,
    })
}

fn waiting(kind: u8) -> Checkpoint {
    let mut supplied = state();
    supplied.facts.push(fact(7, 0));
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

fn command(input: &mut GameInput) -> &mut CommandInput {
    let GameInput::Game(command) = input else {
        unreachable!()
    };
    command
}

fn validate<'a>(
    input: &'a GameInput,
    current: &Checkpoint,
) -> Result<&'a CommandInput, CandidateError> {
    validate_semantic_candidate(
        input,
        current,
        owner(current),
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        command_limits(),
    )
}

fn validate_steps<'a>(
    inputs: &'a [GameInput],
    current: &'a Checkpoint,
    limits: PlanLimits,
) -> Result<df_intent::plan::PlanAdmission<'a>, PlanError> {
    validate_plan_steps(
        inputs,
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

struct RecordedDrawHandler<'a> {
    pins: &'a CheckpointPins,
    expected: RulesCommandInput<'a>,
    calls: Cell<usize>,
}

impl RulesCommandHandler for RecordedDrawHandler<'_> {
    type Rejection = CheckpointError;

    fn pins(&self) -> &CheckpointPins {
        self.pins
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        self.calls.set(self.calls.get() + 1);
        assert!(std::ptr::eq(input.command, self.expected.command));
        assert!(std::ptr::eq(
            input.supplied_draws,
            self.expected.supplied_draws
        ));
        let GameInput::Game(command) = input.command else {
            unreachable!()
        };
        let mut next_basis = current.basis();
        next_basis.revision = next_basis.revision.next_sequence().unwrap();
        let mut staged = current.state().clone();
        for pending in &mut staged.pending {
            pending.basis = next_basis;
        }
        let mut accepted = Vec::new();
        for (index, draw) in input.supplied_draws.iter().enumerate() {
            let id =
                FactId::from_bytes(&[32_u8.checked_add(index.try_into().unwrap()).unwrap(); 16])
                    .unwrap();
            staged.facts.push(GameFact {
                id,
                revision: next_basis.revision,
                operation: command.operation,
                ordinal: index.try_into().unwrap(),
                cause: None,
                audience: AudienceScope::Shared,
                value: FactValue::DrawAccepted {
                    operation: draw.operation,
                    ordinal: draw.ordinal,
                },
            });
            accepted.push(id);
        }
        staged.draws.extend_from_slice(input.supplied_draws);
        staged.decisions.push(AcceptedDecision {
            operation: command.operation,
            revision: next_basis.revision,
            facts: accepted,
            draws: input
                .supplied_draws
                .iter()
                .map(|draw| draw.ordinal)
                .collect(),
            effects: vec![],
            source_policy: label("fixture-handler-policy"),
            semantic_output: None,
        });
        checkpoint_at_basis(staged, &[rule()], next_basis, self.pins.clone())
    }
}

fn prepare(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    prepared: &Checkpoint,
    dependencies: &[RuleDependency],
    selector: &df_types::RevisionLabel,
) -> (
    Result<Checkpoint, CandidatePreparationError<CheckpointError>>,
    usize,
) {
    let sources = [rule()];
    let content_entries = [content()];
    let resources = resource_constraints();
    let context = || CurrentRuleContext {
        checkpoint: current,
        basis: current.basis(),
        pins: current.pins(),
        inventory: ReferenceInventory {
            rules: &sources,
            content: &content_entries,
            resources: &resources,
            assets: &[],
        },
        command_limits: command_limits(),
    };
    let handler = RecordedDrawHandler {
        pins: current.pins(),
        expected: input,
        calls: Cell::new(0),
    };
    let guarded = PreconditionedCommandHandler::new(
        &handler,
        &sources[0],
        context(),
        RulePreconditions {
            prepared,
            sources: &sources,
            dependencies,
        },
        PreconditionLimits {
            maximum_dependencies: 8,
            maximum_comparisons: 1_000_000,
            maximum_checkpoint_bytes: 1_000_000,
        },
    );
    let registered = label("fixture-compiled-handler");
    let registrations = [HandlerRegistration::new(&registered, &sources[0], &guarded)];
    let entries = [CatalogEntry::new(&sources[0], b"synthetic-clause")];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-catalog",
        &entries,
        CatalogLimits {
            max_complete_bytes: 64,
            max_entries: 1,
            max_item_bytes: 64,
            max_total_item_bytes: 64,
        },
    )
    .unwrap();
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    let result = prepare_semantic_candidate(
        input,
        context(),
        owner(current),
        &registry,
        selector,
        1_000_000,
    );
    (result, handler.calls.get())
}

/// One composed pure boundary, using existing canonical inputs and registered preparation.
/// The already-accepted checkpoints below are fixture inputs, not Session commit evidence.
#[test]
fn intent_boundary_preserves_classification_provenance_and_uncommitted_tail() {
    let current = waiting(1);
    let before = current.clone();
    let private_text = "Maybe I attack the other one? private-speech-marker";
    let mut raw = RawInputRef {
        input: operation(90),
        member: member(3),
        basis: current.basis(),
        pins: current.pins().clone(),
        finality: InputFinality::Partial,
    };
    let text_limits = DialogueLimits {
        maximum_text_bytes: 64,
    };
    for (context, expected) in [
        (DiscourseContext::Question, IntentDisposition::Question),
        (DiscourseContext::Social, IntentDisposition::Social),
        (DiscourseContext::PlanOnly, IntentDisposition::PlanOnly),
        (DiscourseContext::Meta, IntentDisposition::Meta),
        (DiscourseContext::Joke, IntentDisposition::Joke),
        (DiscourseContext::Unspecified, IntentDisposition::Clarify),
        (DiscourseContext::Clarify, IntentDisposition::Clarify),
    ] {
        assert_eq!(
            classify_dialogue(
                private_text,
                context,
                &raw,
                &current,
                member(3),
                text_limits
            ),
            Ok(expected)
        );
        assert_eq!(current, before);
    }
    assert_eq!(
        confirm_final_input(&raw, &current, member(3)),
        Err(DialogueError::PartialInput)
    );
    raw.finality = InputFinality::Final;
    assert_eq!(
        confirm_final_input(&raw, &current, member(3)),
        Ok(IntentDisposition::Action)
    );
    assert_eq!(
        classify_dialogue(
            " ",
            DiscourseContext::Question,
            &raw,
            &current,
            member(3),
            text_limits
        ),
        Err(DialogueError::EmptyInput)
    );
    assert_eq!(
        classify_dialogue(
            private_text,
            DiscourseContext::Question,
            &raw,
            &current,
            member(3),
            DialogueLimits {
                maximum_text_bytes: 1
            }
        ),
        Err(DialogueError::Capacity)
    );
    for mutation in 0..3 {
        let mut wrong = raw.clone();
        match mutation {
            0 => wrong.member = member(5),
            1 => wrong.basis.revision = wrong.basis.revision.next_sequence().unwrap(),
            _ => wrong.pins.rules.handler_digest.0[0] ^= 1,
        }
        assert!(confirm_final_input(&wrong, &current, member(3)).is_err());
        assert!(
            classify_dialogue(
                private_text,
                DiscourseContext::Question,
                &wrong,
                &current,
                member(3),
                text_limits
            )
            .is_err()
        );
        assert_eq!(current, before);
    }

    // A final disposition cannot supply the independent typed native request identity.
    let candidate = response(1, 31);
    assert!(validate(&candidate, &current).is_ok());
    assert_eq!(
        validate(&response(1, 32), &current),
        Err(CandidateError::OperationMismatch)
    );
    let mut wrong = candidate.clone();
    command(&mut wrong).member = member(5);
    assert_eq!(
        validate(&wrong, &current),
        Err(CandidateError::MemberMismatch)
    );
    command(&mut wrong).member = member(3);
    command(&mut wrong).command = GameCommand::Speak {
        speaker: entity(4),
        text: private_text.to_owned(),
        conversation: None,
    };
    assert_eq!(
        validate(&wrong, &current),
        Err(CandidateError::UnsupportedVariant)
    );
    assert_eq!(
        validate_semantic_candidate(
            &candidate,
            &current,
            owner(&current),
            ReferenceInventory {
                rules: &[],
                content: &[],
                resources: &[],
                assets: &[]
            },
            command_limits()
        ),
        Err(CandidateError::Command(CommandError::InvalidReference))
    );

    let mut tail = response(1, 32);
    let GameCommand::SelectReaction { option, .. } = &mut command(&mut tail).command else {
        unreachable!()
    };
    *option = label("fixture-option-next");
    let steps = [candidate.clone(), tail];
    let steps_before = steps.clone();
    {
        let admitted = validate_steps(&steps, &current, plan_limits()).unwrap();
        let GameInput::Game(first) = &steps[0] else {
            unreachable!()
        };
        assert!(std::ptr::eq(admitted.first(), first));
        assert!(std::ptr::eq(admitted.requires_revalidation(), &steps[1..]));
        let diagnostic = format!("{admitted:?}");
        assert!(!diagnostic.contains("fixture-option-next"));
        assert!(!diagnostic.contains(private_text));
    }
    assert_eq!(steps, steps_before);
    assert_eq!(current, before);
    for mut bounds in [plan_limits(); 3].into_iter().enumerate() {
        match bounds.0 {
            0 => bounds.1.maximum_steps = 1,
            1 => bounds.1.maximum_total_input_bytes = 1,
            _ => bounds.1.maximum_comparisons = 0,
        }
        assert_eq!(
            validate_steps(&steps, &current, bounds.1).map(|_| ()),
            Err(PlanError::Capacity)
        );
    }
    assert_eq!(
        validate_steps(
            &[candidate.clone(), candidate.clone()],
            &current,
            plan_limits()
        )
        .map(|_| ()),
        Err(PlanError::DuplicateOperation)
    );
    let tail_owner = CandidateOwner {
        operation: operation(32),
        ..owner(&current)
    };
    assert_eq!(
        validate_semantic_candidate(
            &steps[1],
            &current,
            tail_owner,
            ReferenceInventory {
                rules: &[rule()],
                content: &[],
                resources: &[],
                assets: &[]
            },
            command_limits()
        ),
        Err(CandidateError::Command(CommandError::UnofferedResponse))
    );
    let mut next_basis = current.basis();
    next_basis.revision = next_basis.revision.next_sequence().unwrap();
    let mut next_state = current.state().clone();
    let resolution = next_state.pending.first_mut().unwrap();
    resolution.basis = next_basis;
    let PendingInput::Reaction { remaining } = &mut resolution.next else {
        unreachable!()
    };
    remaining[0].options = vec![label("fixture-option-next")];
    let next = checkpoint_at_basis(next_state, &[rule()], next_basis, pins()).unwrap();
    let fresh_owner = CandidateOwner {
        basis: next.basis(),
        pins: next.pins(),
        member: member(3),
        operation: operation(32),
    };
    assert!(
        validate_semantic_candidate(
            &steps[1],
            &next,
            fresh_owner,
            ReferenceInventory {
                rules: &[rule()],
                content: &[],
                resources: &[],
                assets: &[]
            },
            command_limits()
        )
        .is_ok()
    );
    assert_eq!(
        validate_semantic_candidate(
            &steps[1],
            &next,
            CandidateOwner {
                operation: operation(32),
                ..owner(&current)
            },
            ReferenceInventory {
                rules: &[rule()],
                content: &[],
                resources: &[],
                assets: &[]
            },
            command_limits()
        ),
        Err(CandidateError::Snapshot(CheckpointError::StaleBasis))
    );

    // Retained work is a lookup fence in every terminal or uncertain state.
    for status in [
        DurableStatus::Pending,
        DurableStatus::SentUnknown,
        DurableStatus::Completed,
        DurableStatus::Failed,
        DurableStatus::Cancelled,
    ] {
        let mut supplied = current.state().clone();
        supplied.intents.push(DurableIntent {
            id: EffectId::from_bytes(&[30; 16]).unwrap(),
            basis: current.basis(),
            operation: operation(32),
            slot: 0,
            kind: EffectKind::PublishPresentation,
            job: None,
            timer: None,
            generation: 17,
            status,
            definition: content(),
        });
        let retained = checkpoint(supplied).unwrap();
        let retained_before = retained.clone();
        assert_eq!(
            validate_steps(&steps, &retained, plan_limits()).map(|_| ()),
            Err(PlanError::LookupRequired {
                operation: operation(32)
            })
        );
        assert_eq!(retained, retained_before);
        assert_eq!(steps, steps_before);
        assert_eq!(retained.state().intents[0].generation, 17);
        assert_eq!(retained.state().intents[0].status, status);
    }

    let roll_current = waiting(2);
    let roll_before = roll_current.clone();
    let roll = response(2, 31);
    let draws = [ActualDraw {
        operation: operation(31),
        ordinal: 0,
        resolution: pending().id,
        window: pending().window.id,
        sides: 20,
        value: 11,
        source: rule(),
    }];
    let input = RulesCommandInput {
        command: &roll,
        supplied_draws: &draws,
    };
    let (staged, calls) = prepare(
        input,
        &roll_current,
        &roll_current,
        &[RuleDependency::PendingResolution(pending().id)],
        &label("fixture-compiled-handler"),
    );
    let staged = staged.unwrap();
    assert_eq!(calls, 1);
    assert_eq!(staged.state().draws, draws);
    assert_eq!(
        staged.state().decisions.last().unwrap().operation,
        operation(31)
    );
    assert_eq!(staged.state().resources, roll_current.state().resources);
    assert_eq!(roll_current, roll_before);
    assert_eq!(
        validate_steps(std::slice::from_ref(&roll), &staged, plan_limits()).map(|_| ()),
        Err(PlanError::LookupRequired {
            operation: operation(31)
        })
    );
    let (unknown, calls) = prepare(
        input,
        &roll_current,
        &roll_current,
        &[],
        &label("unknown-handler"),
    );
    assert_eq!(calls, 0);
    assert_eq!(
        unknown,
        Err(CandidatePreparationError::Rules(
            ResponsePreparationError::Invocation(InvocationError::Dispatch(
                DispatchError::UnknownHandler
            ))
        ))
    );
    let mut changed = roll_current.state().clone();
    changed.resources[0].value = 3;
    let changed = checkpoint(changed).unwrap();
    let changed_before = changed.clone();
    let (stale, calls) = prepare(
        input,
        &changed,
        &roll_current,
        &[RuleDependency::Resource {
            owner: entity(4),
            resource: label("fixture-resource-1"),
        }],
        &label("fixture-compiled-handler"),
    );
    assert_eq!(calls, 0);
    assert_eq!(
        stale,
        Err(CandidatePreparationError::Rules(
            ResponsePreparationError::Invocation(InvocationError::Handler(
                PreconditionedRejection::Precondition(PreconditionError::StaleResource)
            ))
        ))
    );
    assert_eq!(changed, changed_before);

    let mut targets_state = current.state().clone();
    let mut other = targets_state.entities[0].clone();
    other.id = entity(5);
    targets_state.entities.push(other);
    let targets_current = checkpoint(targets_state).unwrap();
    let targets_before = targets_current.clone();
    let mut set = AffordanceSet {
        basis: targets_current.basis(),
        pins: targets_current.pins(),
        actor: entity(4),
        action: content(),
        matches: targets_current
            .state()
            .entities
            .iter()
            .rev()
            .map(|target| Affordance {
                actor: entity(4),
                action: content(),
                source: rule(),
                target: Some(target.clone()),
            })
            .collect(),
    };
    let bounds = TargetResolutionLimits {
        input_records: 128,
        candidates: 16,
        output_bytes: 16384,
    };
    let sources = [rule()];
    let contents = [content()];
    let inventory = ReferenceInventory {
        rules: &sources,
        content: &contents,
        resources: &[],
        assets: &[],
    };
    assert_eq!(
        resolve_target(
            &targets_current,
            &set,
            TargetSelection::Unspecified,
            inventory,
            bounds
        ),
        Ok(TargetResolution::NeedsClarification(vec![
            entity(4),
            entity(5)
        ]))
    );
    let selected = resolve_target(
        &targets_current,
        &set,
        TargetSelection::Explicit(entity(5)),
        inventory,
        bounds,
    )
    .unwrap();
    let TargetResolution::Selected(selected) = selected else {
        panic!("exact target selection")
    };
    assert_eq!(selected.target.as_ref().unwrap().id, entity(5));
    assert_eq!(
        resolve_target(
            &targets_current,
            &set,
            TargetSelection::Explicit(entity(99)),
            inventory,
            bounds
        ),
        Ok(TargetResolution::UnsupportedTarget)
    );
    assert_eq!(
        resolve_target(
            &targets_current,
            &set,
            TargetSelection::Unspecified,
            inventory,
            TargetResolutionLimits {
                candidates: 1,
                ..bounds
            }
        ),
        Err(TargetResolutionError::Capacity)
    );
    set.basis.revision = revision(2, 7);
    assert_eq!(
        resolve_target(
            &targets_current,
            &set,
            TargetSelection::Unspecified,
            inventory,
            bounds
        ),
        Err(TargetResolutionError::Snapshot(CheckpointError::StaleBasis))
    );
    assert_eq!(targets_current, targets_before);
    set.basis = targets_current.basis();
    let mut changed_pins = targets_current.pins().clone();
    changed_pins.rules.handler_digest.0[0] ^= 1;
    set.pins = &changed_pins;
    assert_eq!(
        resolve_target(
            &targets_current,
            &set,
            TargetSelection::Unspecified,
            inventory,
            bounds
        ),
        Err(TargetResolutionError::Snapshot(
            CheckpointError::RulesMismatch
        ))
    );
    assert_eq!(targets_current, targets_before);
    set.pins = targets_current.pins();
    set.matches[0].source.clause = label("foreign-clause");
    assert_eq!(
        resolve_target(
            &targets_current,
            &set,
            TargetSelection::Unspecified,
            inventory,
            bounds
        ),
        Err(TargetResolutionError::InvalidAffordance)
    );
    set.matches.clear();
    assert_eq!(
        resolve_target(
            &targets_current,
            &set,
            TargetSelection::Unspecified,
            inventory,
            bounds
        ),
        Ok(TargetResolution::NeedsRuling)
    );
    assert_eq!(targets_current, targets_before);

    let accepted = partial_progress::checkpoint_after_one_accepted_step();
    let accepted_before = accepted.clone();
    let proposal = [
        partial_progress::response(7, "fixture-option-1"),
        partial_progress::response(8, "fixture-option-1"),
    ];
    {
        let admitted = partial_progress::validate(&proposal, &accepted, 7).unwrap();
        assert!(std::ptr::eq(
            admitted.requires_revalidation(),
            &proposal[1..]
        ));
    }
    assert_eq!(accepted, accepted_before);
    assert_eq!(
        accepted.state().decisions[0].operation,
        partial_progress::operation(6)
    );
    let later = partial_progress::checkpoint_after_two_accepted_steps();
    let later_before = later.clone();
    assert!(partial_progress::validate(&proposal[1..], &later, 8).is_ok());
    assert_eq!(
        partial_progress::validate(
            &[partial_progress::response(7, "fixture-option-1")],
            &later,
            7
        )
        .map(|_| ()),
        Err(PlanError::LookupRequired {
            operation: partial_progress::operation(7)
        })
    );
    assert_eq!(later, later_before);
    println!(
        "Intent boundary: declared non-action classification, current provenance, registered preparation, target clarification, bounded head/uncommitted tail, and accepted-progress preservation observed; persistent plan/durable executor unresolved. Ledger: {}",
        include_str!("fixtures/intent_boundary_contract.json")
    );
}
