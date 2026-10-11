//! Source-admitted existing encounter objectives through the registered session boundary.

use df_encounter::objectives::{
    AuthoredObjectiveTransition, ObjectivePolicyOwner, ObjectiveProposalError,
    ObjectiveProposalLimits, ObjectiveProposalRequest, propose_objective_transitions,
};
use df_model::checkpoint::{
    AcceptedDecision, Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins,
    CommandInput, ContentReference, GameCommand, GameInput, ReferenceInventory, RuleReference,
};
use df_rules::{RulesCommandHandler, RulesCommandInput};
use df_types::RevisionLabel;

/// Compiled action/source registration and the accepted application's policy revision.
/// The source owner admits the mapping; these labels alone confer no authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectiveRegistration {
    pub action: ContentReference,
    pub source: RuleReference,
    pub policy: RevisionLabel,
}

/// Native source authority over this exact command and its complete authored batch.
///
/// In addition to each `ObjectivePolicyOwner` mapping/cause/rights check, admission must
/// check the current registration, authenticated member's actor control and application
/// policy, including source-admitted terminal turn clearing. All checks are pure,
/// deterministic and bounded. No inventory entry, client
/// action or retained proposal supplies permission. Session authenticates ingress first.
pub trait ObjectiveSourceOwner: ObjectivePolicyOwner {
    type Refusal;

    fn admit_application(
        &self,
        current: &Checkpoint,
        registration: &ObjectiveRegistration,
        command: &CommandInput,
        transitions: &[AuthoredObjectiveTransition<'_>],
    ) -> Result<(), Self::Refusal>;
}

/// Explicit memory/work bounds; production values remain native owner admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectiveStagingLimits {
    pub maximum_checkpoint_bytes: usize,
    pub maximum_pass_bytes: usize,
    pub checkpoint: CheckpointLimits,
    pub objectives: ObjectiveProposalLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObjectiveStagingError<R> {
    Capacity,
    Snapshot(CheckpointError),
    UnsupportedInput,
    StaleCommand,
    PendingResolution,
    AlreadyAccepted,
    MissingProducer,
    InvalidRegistration,
    WrongCommand,
    Source(R),
    Objective(ObjectiveProposalError),
    RevisionExhausted,
    InvalidCandidate(CheckpointError),
}

/// Source-owned native batch; no client payload supplies objective mappings or cause facts.
///
/// Register behind `PreconditionedCommandHandler` with source-reviewed dependencies and
/// invoke `decide_registered_command` inside `SessionEngine`. The returned checkpoint is
/// the existing canonical transition candidate; only `DurableOwner` may commit/publish it.
/// Scoped operation lookup must precede staging. Confirmed retries return the original
/// receipt without restaging; unknown commits require lookup/reload, never reapplication.
/// `None` means missing source producer; `Some([])` is an admitted no-change application.
pub struct ObjectiveTransitionHandler<'a, Owner> {
    pub owner: &'a Owner,
    pub current_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub registration: &'a ObjectiveRegistration,
    pub transitions: Option<&'a [AuthoredObjectiveTransition<'a>]>,
    pub limits: ObjectiveStagingLimits,
}

impl<Owner: ObjectiveSourceOwner> RulesCommandHandler for ObjectiveTransitionHandler<'_, Owner> {
    type Rejection = ObjectiveStagingError<Owner::Refusal>;

    fn pins(&self) -> &CheckpointPins {
        self.admitted_pins
    }

    fn bound_source(&self) -> Option<&RuleReference> {
        Some(&self.registration.source)
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        self.check_capacity(current, input.command)?;
        current
            .validate_resume(self.current_basis, self.admitted_pins)
            .map_err(ObjectiveStagingError::Snapshot)?;
        let GameInput::Game(command) = input.command else {
            return Err(ObjectiveStagingError::UnsupportedInput);
        };
        if !input.supplied_draws.is_empty() {
            return Err(ObjectiveStagingError::UnsupportedInput);
        }
        if command.basis != self.current_basis
            || command.observed_revision != self.current_basis.revision
        {
            return Err(ObjectiveStagingError::StaleCommand);
        }
        if !current.state().pending.is_empty() {
            return Err(ObjectiveStagingError::PendingResolution);
        }
        if current
            .state()
            .decisions
            .iter()
            .any(|decision| decision.operation == command.operation)
        {
            return Err(ObjectiveStagingError::AlreadyAccepted);
        }
        let transitions = self
            .transitions
            .ok_or(ObjectiveStagingError::MissingProducer)?;
        let registration = self.registration;
        if registration.source.catalog != current.pins().rules.catalog
            || !self.inventory.rules.contains(&registration.source)
            || registration.action.package != current.pins().content.package
            || !self.inventory.content.contains(&registration.action)
            || transitions
                .iter()
                .any(|transition| transition.source != &registration.source)
        {
            return Err(ObjectiveStagingError::InvalidRegistration);
        }
        let GameCommand::ProposeAction {
            actor,
            action,
            targets,
            choices,
        } = &command.command
        else {
            return Err(ObjectiveStagingError::WrongCommand);
        };
        // Objective slots are canonical record positions, not client-selected entity targets.
        if action != &registration.action
            || !targets.is_empty()
            || !choices.is_empty()
            || transitions
                .iter()
                .any(|transition| transition.actor != *actor)
            || !current
                .state()
                .members
                .iter()
                .any(|link| link.member == command.member && link.character == Some(*actor))
            || !current
                .state()
                .entities
                .iter()
                .any(|entity| entity.id == *actor)
        {
            return Err(ObjectiveStagingError::WrongCommand);
        }
        // The real proposer admits all request bytes before invoking its source policy owner.
        // Objective replacements and source-admitted turn termination share one detached result.
        // It remains private if application admission or the final tail fails.
        let proposal = propose_objective_transitions(
            self.owner,
            ObjectiveProposalRequest {
                current,
                expected_basis: self.current_basis,
                admitted_pins: self.admitted_pins,
                inventory: self.inventory(),
                transitions,
                checkpoint_limits: self.limits.checkpoint,
                limits: self.limits.objectives,
            },
        )
        .map_err(ObjectiveStagingError::Objective)?;
        self.owner
            .admit_application(current, registration, command, transitions)
            .map_err(ObjectiveStagingError::Source)?;
        let mut next = self.current_basis;
        next.revision = next
            .revision
            .next_sequence()
            .map_err(|_| ObjectiveStagingError::RevisionExhausted)?;
        let mut state = proposal.checkpoint.state().clone();
        state
            .decisions
            .try_reserve_exact(1)
            .map_err(|_| ObjectiveStagingError::Capacity)?;
        // The new operation owns application, not the already committed causal declarations.
        // Do not duplicate their facts/draws/effects or classify new mechanical outcomes.
        state.decisions.push(AcceptedDecision {
            operation: command.operation,
            revision: next.revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: registration.policy.clone(),
            semantic_output: None,
        });
        Checkpoint::new(
            current.schema(),
            next,
            current.pins().clone(),
            state,
            self.inventory(),
            self.limits.checkpoint,
        )
        .map_err(ObjectiveStagingError::InvalidCandidate)
    }
}

impl<Owner: ObjectiveSourceOwner> ObjectiveTransitionHandler<'_, Owner> {
    fn inventory(&self) -> ReferenceInventory<'_> {
        ReferenceInventory {
            rules: self.inventory.rules,
            content: self.inventory.content,
            resources: self.inventory.resources,
            assets: self.inventory.assets,
        }
    }

    fn check_capacity(
        &self,
        current: &Checkpoint,
        input: &GameInput,
    ) -> Result<(), ObjectiveStagingError<Owner::Refusal>> {
        let limits = self.limits;
        let proposal = limits.objectives;
        if limits.maximum_checkpoint_bytes == 0
            || limits.checkpoint.maximum_retained_bytes == 0
            || limits.checkpoint.maximum_retained_bytes > limits.maximum_checkpoint_bytes
            || proposal.maximum_records == 0
            || proposal.maximum_changes == 0
            || proposal.maximum_changes > proposal.maximum_records
            || proposal.maximum_comparisons == 0
            || proposal.maximum_input_bytes == 0
            || proposal.maximum_output_bytes == 0
            || self
                .transitions
                .is_some_and(|changes| changes.len() > proposal.maximum_changes)
            || [
                self.inventory.rules.len(),
                self.inventory.content.len(),
                self.inventory.resources.len(),
                self.inventory.assets.len(),
            ]
            .into_iter()
            .any(|count| count > proposal.maximum_records)
        {
            return Err(ObjectiveStagingError::Capacity);
        }
        let mut registration_bytes = std::mem::size_of::<ObjectiveRegistration>();
        for label in [
            &self.registration.action.package,
            &self.registration.action.entry,
            &self.registration.source.catalog,
            &self.registration.source.source,
            &self.registration.source.entry,
            &self.registration.source.clause,
            &self.registration.policy,
        ] {
            if label.as_str().len() > limits.checkpoint.maximum_text_bytes {
                return Err(ObjectiveStagingError::Capacity);
            }
            registration_bytes = registration_bytes
                .checked_add(label.retained_heap_bytes())
                .ok_or(ObjectiveStagingError::Capacity)?;
        }
        let current_bytes = current
            .retained_bytes()
            .filter(|bytes| *bytes <= limits.maximum_checkpoint_bytes)
            .ok_or(ObjectiveStagingError::Capacity)?;
        // Reserve proposer traversal/output and both final constructor working copies up front.
        let total = limits
            .maximum_checkpoint_bytes
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(current_bytes))
            .and_then(|bytes| bytes.checked_add(proposal.maximum_input_bytes))
            .and_then(|bytes| bytes.checked_add(proposal.maximum_output_bytes))
            .and_then(|bytes| bytes.checked_add(registration_bytes))
            .and_then(|bytes| bytes.checked_add(input.retained_bytes()?))
            .ok_or(ObjectiveStagingError::Capacity)?;
        if total > limits.maximum_pass_bytes {
            return Err(ObjectiveStagingError::Capacity);
        }
        Ok(())
    }
}
