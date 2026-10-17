//! Test-only fixtures for bounded plan admission and partial progress.
//!
//! This module is mounted by `tests/partial_progress.rs`. Canonical checkpoints in
//! these fixtures represent state already accepted by the session owner; this file
//! does not implement session commits or cancellation receipts.

#[path = "../tests/support/candidate_fixture.rs"]
#[allow(dead_code)]
pub(crate) mod candidate_fixture;

use candidate_fixture::{
    basis, checkpoint_at_basis, content, fact, label, member, pending, pins, rule, state,
};
use df_intent::candidate::CandidateOwner;
use df_intent::plan::{PlanAdmission, PlanError, PlanLimits, validate_plan_steps};
use df_model::checkpoint::{
    AcceptedDecision, Basis, Checkpoint, FactId, FactValue, GameFact, GameInput, ReferenceInventory,
};
use df_model::commands::CommandLimits;
use df_types::OperationId;

pub(super) fn operation(value: u8) -> OperationId {
    OperationId::from_bytes(&[value; 16]).expect("fixture operation identity is valid")
}

pub(super) fn response(operation_id: u8, option: &str) -> GameInput {
    let current = pending();
    GameInput::Game(df_model::checkpoint::CommandInput {
        basis: basis(),
        observed_revision: basis().revision,
        operation: operation(operation_id),
        member: member(3),
        command: df_model::checkpoint::GameCommand::SelectReaction {
            resolution: current.id,
            window: current.window.id,
            offer: label("fixture-offer-1"),
            option: label(option),
        },
    })
}

pub(super) fn owner(current: &Checkpoint, operation_id: u8) -> CandidateOwner<'_> {
    CandidateOwner {
        basis: current.basis(),
        pins: current.pins(),
        member: member(3),
        operation: operation(operation_id),
    }
}

pub(super) fn limits() -> PlanLimits {
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

pub(super) fn validate<'a>(
    steps: &'a [GameInput],
    current: &'a Checkpoint,
    operation_id: u8,
) -> Result<PlanAdmission<'a>, PlanError> {
    validate_with_owner(steps, current, owner(current, operation_id))
}

pub(super) fn validate_with_owner<'a>(
    steps: &'a [GameInput],
    current: &'a Checkpoint,
    owner: CandidateOwner<'_>,
) -> Result<PlanAdmission<'a>, PlanError> {
    validate_plan_steps(
        steps,
        current,
        owner,
        ReferenceInventory {
            rules: &[rule()],
            content: &[content()],
            resources: &[],
            assets: &[],
        },
        limits(),
    )
}

/// A canonical current checkpoint after the session accepted operation 6 and its fact.
pub(super) fn checkpoint_after_one_accepted_step() -> Checkpoint {
    let mut supplied = state();
    let accepted_fact = fact(7, 0);
    let accepted_fact_id = accepted_fact.id;
    let accepted_revision = basis().revision;
    supplied.facts.push(accepted_fact);
    supplied.decisions.push(AcceptedDecision {
        operation: operation(6),
        revision: accepted_revision,
        facts: vec![accepted_fact_id],
        draws: vec![],
        effects: vec![],
        source_policy: label("fixture-policy-1"),
        semantic_output: None,
    });
    let next_basis = Basis {
        revision: accepted_revision
            .next_sequence()
            .expect("fixture revision advances"),
        ..basis()
    };
    let mut resolution = pending();
    resolution.basis = next_basis;
    supplied.pending.push(resolution);
    checkpoint_at_basis(supplied, &[rule()], next_basis, pins())
        .expect("accepted fixture decision and fact form a valid checkpoint")
}

/// A later canonical checkpoint after the session also accepted operation 7.
pub(super) fn checkpoint_after_two_accepted_steps() -> Checkpoint {
    let previous = checkpoint_after_one_accepted_step();
    let mut supplied = previous.state().clone();
    let accepted_revision = previous.basis().revision;
    let accepted_fact_id = FactId::from_bytes(&[8; 16]).expect("fixture fact identity is valid");
    supplied.facts.push(GameFact {
        id: accepted_fact_id,
        revision: accepted_revision,
        operation: operation(7),
        ordinal: 0,
        cause: Some(FactId::from_bytes(&[7; 16]).expect("prior fixture fact is valid")),
        audience: df_model::checkpoint::AudienceScope::Shared,
        value: FactValue::ContentEvent {
            definition: content(),
            subjects: vec![candidate_fixture::entity(4)],
        },
    });
    supplied.decisions.push(AcceptedDecision {
        operation: operation(7),
        revision: accepted_revision,
        facts: vec![accepted_fact_id],
        draws: vec![],
        effects: vec![],
        source_policy: label("fixture-policy-1"),
        semantic_output: None,
    });
    let next_basis = Basis {
        revision: accepted_revision
            .next_sequence()
            .expect("fixture revision advances"),
        ..basis()
    };
    if let Some(resolution) = supplied.pending.first_mut() {
        resolution.basis = next_basis;
        resolution.window.causal_fact = accepted_fact_id;
    }
    checkpoint_at_basis(supplied, &[rule()], next_basis, pins())
        .expect("two accepted fixture decisions and facts form a valid checkpoint")
}
