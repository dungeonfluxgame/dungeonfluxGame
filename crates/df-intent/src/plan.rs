//! Ephemeral admission of bounded ordered requests, not a persisted ActionPlan schema.

use crate::candidate::{CandidateError, CandidateOwner, validate_semantic_candidate};
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointPins, CommandInput, GameCommand, GameInput, ReferenceInventory,
};
use df_model::commands::CommandLimits;
use df_types::OperationId;

/// Bounds apply to the entire borrowed request list, without copying its private payloads.
/// The existing candidate limits separately bound first-step current-response validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanLimits {
    pub maximum_steps: usize,
    pub maximum_total_input_bytes: usize,
    pub maximum_comparisons: usize,
    pub commands: CommandLimits,
}

/// Typed refusals contain only identities and classifications, never request text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError {
    EmptyPlan,
    Capacity,
    UnsupportedStep { index: usize },
    MemberMismatch { index: usize },
    ScopeMismatch { index: usize },
    DuplicateOperation,
    LookupRequired { operation: OperationId },
    Candidate(CandidateError),
}

/// Only `first` has passed current-response admission. The tail requires revalidation.
/// This borrowed view issues no plan identity, continuation lifetime or commit receipt.
/// Its checkpoint pins are exact current provenance, not predicted future source approval.
pub struct PlanAdmission<'a> {
    first: &'a CommandInput,
    requires_revalidation: &'a [GameInput],
    checkpoint: &'a Checkpoint,
}

impl<'a> PlanAdmission<'a> {
    pub fn first(&self) -> &'a CommandInput {
        self.first
    }

    /// Unchanged later requests; none is admitted against the initial checkpoint.
    pub fn requires_revalidation(&self) -> &'a [GameInput] {
        self.requires_revalidation
    }

    pub fn basis(&self) -> Basis {
        self.checkpoint.basis()
    }

    pub fn pins(&self) -> &'a CheckpointPins {
        self.checkpoint.pins()
    }
}

impl std::fmt::Debug for PlanAdmission<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlanAdmission")
            .field("basis", &self.basis())
            .field("first_operation", &self.first.operation)
            .field("revalidation_count", &self.requires_revalidation.len())
            .finish_non_exhaustive()
    }
}

/// Admit the first currently offered Choice/Reaction/Roll request in a bounded ordered list.
///
/// Every request must retain the same authenticated member/session/run/recovery epoch,
/// nonfuture client observations and a distinct operation identity. Only the first goes through
/// `validate_semantic_candidate`; later windows, costs and targets are deliberately not assumed
/// legal now or after the first result. Resume by validating the remainder against the owner's
/// new checkpoint. Current native basis and complete pins must match exactly.
///
/// Any recorded decision or durable intent for a requested operation requires native operation
/// lookup before further admission, regardless of job status. This boundary does not resolve
/// uncertainty, replay work, stage mechanics, draw dice, consume resources or cancel commits.
/// Persisted plan policy, confirmation, authorization, source rights and durable commit remain
/// separate owner contracts. Disclosure of these borrowed inputs also requires authorization.
pub fn validate_plan_steps<'a>(
    steps: &'a [GameInput],
    current: &'a Checkpoint,
    owner: CandidateOwner<'_>,
    inventory: ReferenceInventory<'_>,
    limits: PlanLimits,
) -> Result<PlanAdmission<'a>, PlanError> {
    current
        .validate_resume(owner.basis, owner.pins)
        .map_err(|error| PlanError::Candidate(CandidateError::Snapshot(error)))?;
    let (first, remaining) = steps.split_first().ok_or(PlanError::EmptyPlan)?;
    if steps.len() > limits.maximum_steps || limits.maximum_total_input_bytes == 0 {
        return Err(PlanError::Capacity);
    }
    let records = current
        .state()
        .decisions
        .len()
        .checked_add(current.state().intents.len())
        .ok_or(PlanError::Capacity)?;
    let comparisons = comparison_bound(steps.len(), records)?;
    if comparisons > limits.maximum_comparisons {
        return Err(PlanError::Capacity);
    }
    let mut retained_bytes = 0_usize;
    for (index, input) in steps.iter().enumerate() {
        let command = response(input).ok_or(PlanError::UnsupportedStep { index })?;
        if command.member != owner.member {
            return Err(PlanError::MemberMismatch { index });
        }
        if command.basis.session != owner.basis.session
            || command.basis.run != owner.basis.run
            || command.basis.revision.epoch() != owner.basis.revision.epoch()
            || command.basis.revision > owner.basis.revision
            || command.observed_revision.epoch() != owner.basis.revision.epoch()
            || command.observed_revision > owner.basis.revision
        {
            return Err(PlanError::ScopeMismatch { index });
        }
        if steps.iter().take(index).any(|earlier| {
            response(earlier).is_some_and(|earlier| earlier.operation == command.operation)
        }) {
            return Err(PlanError::DuplicateOperation);
        }
        retained_bytes = retained_bytes
            .checked_add(input.retained_bytes().ok_or(PlanError::Capacity)?)
            .ok_or(PlanError::Capacity)?;
        if retained_bytes > limits.maximum_total_input_bytes {
            return Err(PlanError::Capacity);
        }
        if current
            .state()
            .decisions
            .iter()
            .any(|record| record.operation == command.operation)
            || current
                .state()
                .intents
                .iter()
                .any(|record| record.operation == command.operation)
        {
            return Err(PlanError::LookupRequired {
                operation: command.operation,
            });
        }
    }
    let first = validate_semantic_candidate(first, current, owner, inventory, limits.commands)
        .map_err(PlanError::Candidate)?;
    Ok(PlanAdmission {
        first,
        requires_revalidation: remaining,
        checkpoint: current,
    })
}

fn response(input: &GameInput) -> Option<&CommandInput> {
    let GameInput::Game(command) = input else {
        return None;
    };
    match command.command {
        GameCommand::SelectChoice { .. }
        | GameCommand::SelectReaction { .. }
        | GameCommand::SubmitRoll { .. } => Some(command),
        _ => None,
    }
}

fn comparison_bound(steps: usize, records: usize) -> Result<usize, PlanError> {
    steps
        .checked_mul(steps.checked_sub(1).ok_or(PlanError::Capacity)?)
        .map(|count| count / 2)
        .and_then(|count| count.checked_add(steps.checked_mul(records)?))
        .ok_or(PlanError::Capacity)
}

#[cfg(test)]
mod tests {
    use super::{PlanError, comparison_bound};

    #[test]
    fn comparison_overflow_refuses_without_allocating_impossible_input_lists() {
        assert_eq!(comparison_bound(usize::MAX, 0), Err(PlanError::Capacity));
        assert_eq!(comparison_bound(2, usize::MAX), Err(PlanError::Capacity));
        assert_eq!(comparison_bound(2, 2), Ok(5));
    }
}
