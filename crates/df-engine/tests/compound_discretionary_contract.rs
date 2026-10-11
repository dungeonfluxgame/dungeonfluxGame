//! Whole Original Design contract for source-qualified compound heads; no general ruling executor.
#![cfg(not(target_arch = "wasm32"))]

use super::*;
use df_content::catalog::{CatalogEntry, CatalogLimits, CatalogSnapshot};
use df_engine::command_entry::{CommandEntryContext, CommandEntryLimits};
use df_engine::pending_resumption::ResumeLimits;
use df_engine::plan_staging::{PlanHeadAdmission, StagedPlanHead, stage_plan_head};
use df_intent::candidate::CandidateOwner;
use df_rules::preconditions::{
    CurrentRuleContext, PreconditionLimits, PreconditionedCommandHandler, RuleDependency,
    RulePreconditions,
};
use df_rules::{DispatchError, DispatchRegistry, HandlerRegistration};

const _: &str = include_str!("fixtures/compound_discretionary_contract.json");

#[derive(Clone, Copy)]
enum SourceSelection {
    Exact,
    OtherClause,
}

// Published synthetic clauses qualify source-selection wiring, not tabletop mechanics or rights.
fn registered_head<'a>(
    steps: &'a [GameInput],
    draws: &[ActualDraw],
    current: &Checkpoint,
    prepared: &Checkpoint,
    inner: &plan_fixtures::Handler,
    selection: SourceSelection,
) -> Result<StagedPlanHead<'a>, PlanHeadError<FixtureRejection>> {
    let trusted = admission(current, &steps[0]);
    let source = pending_fixtures::rule();
    let mut other = source.clone();
    other.clause = label("different-published-clause");
    let registered_source = match selection {
        SourceSelection::Exact => &source,
        SourceSelection::OtherClause => &other,
    };
    let rules = [source.clone(), other.clone()];
    let contents = [content()];
    let resources = pending_fixtures::resource_constraints();
    let dependencies = [
        RuleDependency::Resource {
            owner: entity(4),
            resource: label("fixture-resource-1"),
        },
        RuleDependency::PendingResolutions,
    ];
    let inventory = || ReferenceInventory {
        rules: &rules,
        content: &contents,
        resources: &resources,
        assets: &[],
    };
    let wrapper = PreconditionedCommandHandler::new(
        inner,
        registered_source,
        CurrentRuleContext {
            checkpoint: current,
            basis: current.basis(),
            pins: current.pins(),
            inventory: inventory(),
            command_limits: trusted.engine_command,
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
    let entries = [
        CatalogEntry::new(&source, b"synthetic-G11-source-clause"),
        CatalogEntry::new(&other, b"synthetic-other-source-clause"),
    ];
    let catalog = CatalogSnapshot::from_published(
        &current.pins().rules.catalog,
        current.pins(),
        b"synthetic-G11-publication",
        &entries,
        CatalogLimits {
            max_complete_bytes: 128,
            max_entries: 2,
            max_item_bytes: 64,
            max_total_item_bytes: 128,
        },
    )
    .unwrap();
    let selector = label("fixture-continuation-position-1");
    let registrations = [HandlerRegistration::new(
        &selector,
        registered_source,
        &wrapper,
    )];
    let registry = DispatchRegistry::from_catalog(catalog, &registrations, 1).unwrap();
    if matches!(selection, SourceSelection::Exact) {
        let (selected, explanation) = registry
            .select_with_provenance(current.pins(), &selector, &source)
            .unwrap();
        assert!(std::ptr::eq(selected, &wrapper));
        assert_eq!(explanation.rules_pins(), &current.pins().rules);
        assert_eq!(explanation.content_pins(), &current.pins().content);
        assert_eq!(explanation.build(), &current.pins().build);
        assert_eq!(explanation.source(), &source);
        assert_eq!(explanation.selector(), &selector);
        assert_eq!(explanation.source_bytes(), b"synthetic-G11-source-clause");
    }
    stage_plan_head(
        steps,
        draws,
        current,
        CommandEntryContext {
            current_basis: current.basis(),
            admitted_pins: current.pins(),
            inventory: inventory(),
            limits: CommandEntryLimits {
                command: trusted.engine_command,
                maximum_staged_bytes: trusted.maximum_bytes,
            },
        },
        PlanHeadAdmission {
            owner: CandidateOwner {
                basis: trusted.basis,
                pins: &trusted.pins,
                member: trusted.member,
                operation: trusted.operation,
            },
            fence: trusted.fence,
            limits: ResumeLimits {
                maximum_checkpoint_bytes: trusted.maximum_bytes,
            },
            plan_limits: trusted.plan,
        },
        &registry,
        &selector,
    )
}

#[test]
fn published_source_provenance_and_exact_head_reach_registered_handler() {
    for kind in 0..3 {
        let current = waiting(kind);
        let original = current.clone();
        let steps = two_steps(kind);
        let draws = if kind == 2 {
            vec![native_roll_draw(0)]
        } else {
            vec![]
        };
        let inner = handler(&current, &steps[0], &draws);
        let staged = registered_head(
            &steps,
            &draws,
            &current,
            &current,
            &inner,
            SourceSelection::Exact,
        )
        .unwrap();
        assert_eq!(staged.checkpoint(), &inner.candidate);
        assert!(std::ptr::eq(staged.first(), &steps[0]));
        assert!(std::ptr::eq(
            staged.requires_revalidation().as_ptr(),
            steps[1..].as_ptr()
        ));
        assert_eq!(inner.command_pointer.get(), &steps[0] as *const GameInput);
        assert_eq!(inner.draw_pointer.get(), draws.as_ptr());
        assert_eq!(inner.calls.get(), 1);
        assert_eq!(current, original);
        assert_eq!(staged.checkpoint().state().draws, draws);
        assert_eq!(staged.checkpoint().state().decisions.len(), 1);
    }
}

#[test]
fn same_selector_registered_for_other_clause_cannot_approve_current_source() {
    for kind in 0..3 {
        let current = waiting(kind);
        let original = current.clone();
        let steps = two_steps(kind);
        let draws = if kind == 2 {
            vec![native_roll_draw(0)]
        } else {
            vec![]
        };
        let inner = handler(&current, &steps[0], &draws);
        assert_eq!(
            registered_head(
                &steps,
                &draws,
                &current,
                &current,
                &inner,
                SourceSelection::OtherClause,
            )
            .err(),
            Some(PlanHeadError::Semantic(SemanticResponseError::Resume(
                ResumeError::Command(CommandRejection::Invocation(InvocationError::Dispatch(
                    DispatchError::UnsupportedSource,
                ))),
            )))
        );
        assert_eq!(inner.calls.get(), 0);
        assert_eq!(current, original);
    }
}

#[test]
fn host_discretionary_ruling_in_any_plan_position_refuses_before_first_handler() {
    for position in 0..2 {
        let current = waiting(if position == 0 { 3 } else { 0 });
        let original = current.clone();
        let clean = two_steps(0);
        let inner = handler(&current, &clean[0], &[]);
        let trusted = admission(&current, &clean[0]);
        let mut steps = clean;
        steps[position] = response(3);
        assert_eq!(
            invoke(&steps, &[], &current, &current, &inner, trusted).err(),
            Some(PlanHeadError::Plan(PlanError::UnsupportedStep {
                index: position,
            }))
        );
        assert_eq!(inner.calls.get(), 0);
        assert_eq!(current, original);
    }
}

#[test]
fn creative_physical_action_in_any_plan_position_has_no_automatic_mechanics() {
    for position in 0..2 {
        let current = waiting(0);
        let original = current.clone();
        let mut steps = two_steps(0);
        let inner = handler(&current, &steps[0], &[]);
        let trusted = admission(&current, &steps[0]);
        let GameInput::Game(command) = &mut steps[position] else {
            unreachable!()
        };
        command.command = GameCommand::ProposeAction {
            actor: entity(4),
            action: content(),
            targets: vec![],
            choices: vec![],
        };
        assert_eq!(
            invoke(&steps, &[], &current, &current, &inner, trusted).err(),
            Some(PlanHeadError::Plan(PlanError::UnsupportedStep {
                index: position,
            }))
        );
        assert_eq!(inner.calls.get(), 0);
        assert_eq!(current, original);
    }
}

#[test]
fn native_source_handler_catalog_and_content_changes_refuse_each_new_head() {
    for changed_pin in 0..3 {
        let current = waiting(0);
        let original = current.clone();
        let steps = two_steps(0);
        let inner = handler(&current, &steps[0], &[]);
        let mut trusted = admission(&current, &steps[0]);
        let expected = match changed_pin {
            0 => {
                trusted.pins.rules.handler = label("withdrawn-source-handler");
                CheckpointError::RulesMismatch
            }
            1 => {
                trusted.pins.rules.catalog = label("different-source-catalog");
                CheckpointError::RulesMismatch
            }
            _ => {
                trusted.pins.content.content = label("different-source-content");
                CheckpointError::ContentMismatch
            }
        };
        assert_eq!(
            invoke(&steps, &[], &current, &current, &inner, trusted).err(),
            Some(PlanHeadError::Plan(PlanError::Candidate(
                CandidateError::Snapshot(expected),
            )))
        );
        assert_eq!(inner.calls.get(), 0);
        assert_eq!(current, original);
    }
}

#[test]
fn currently_offered_response_cannot_bypass_source_declared_resource_costs() {
    let prepared = waiting(0);
    let mut state = prepared.state().clone();
    state.resources[0].value -= 1;
    let current = rebuild(&prepared, state);
    let original = current.clone();
    let steps = two_steps(0);
    let inner = handler(&current, &steps[0], &[]);
    assert_eq!(
        registered_head(
            &steps,
            &[],
            &current,
            &prepared,
            &inner,
            SourceSelection::Exact,
        )
        .err(),
        Some(PlanHeadError::Semantic(SemanticResponseError::Resume(
            ResumeError::Command(CommandRejection::Invocation(InvocationError::Handler(
                PreconditionedRejection::Precondition(PreconditionError::StaleResource),
            ))),
        )))
    );
    assert_eq!(inner.calls.get(), 0);
    assert_eq!(current, original);
}

#[test]
fn later_discretionary_refusal_preserves_prior_accepted_staged_prefix() {
    let current = waiting(0);
    let original = current.clone();
    let steps = two_steps(0);
    let inner = handler(&current, &steps[0], &[]);
    let staged = registered_head(
        &steps,
        &[],
        &current,
        &current,
        &inner,
        SourceSelection::Exact,
    )
    .unwrap();
    let prefix = staged.checkpoint().clone();
    let tail = staged.requires_revalidation();
    let next_inner = handler(&prefix, &tail[0], &[]);
    let mut unsupported = tail.to_vec();
    unsupported[0] = response(3);
    let trusted = admission(&prefix, &tail[0]);
    assert_eq!(
        invoke(&unsupported, &[], &prefix, &prefix, &next_inner, trusted).err(),
        Some(PlanHeadError::Plan(PlanError::UnsupportedStep { index: 0 }))
    );
    let mut unoffered = tail.to_vec();
    let GameInput::Game(command) = &mut unoffered[0] else {
        unreachable!()
    };
    let GameCommand::SelectChoice { option, .. } = &mut command.command else {
        unreachable!()
    };
    *option = label("not-authorized-at-next-head");
    assert_eq!(
        invoke(
            &unoffered,
            &[],
            &prefix,
            &prefix,
            &next_inner,
            admission(&prefix, &tail[0]),
        )
        .err(),
        Some(PlanHeadError::Plan(PlanError::Candidate(
            CandidateError::Command(CommandError::UnofferedResponse),
        )))
    );
    assert_eq!(next_inner.calls.get(), 0);
    assert_eq!(staged.checkpoint(), &prefix);
    assert_eq!(prefix.state().decisions.len(), 1);
    assert_eq!(prefix.state().decisions[0].operation, operation(&steps[0]));
    assert_eq!(current, original);
}
