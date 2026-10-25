//! Exact source-admitted obligation lifecycle actions staged through the registered rules boundary.
//! A prior committed cause is required. The session ledger and commit owner resolve retries.

use df_interaction::debts::{
    DebtAction, DebtAuthorization, DebtStatus, DebtTransitionOutcome, DebtTransitionRefusal,
    DebtTransitionRequest, ObligationView, propose_debt_transition,
};
use df_model::checkpoint::{
    AcceptedDecision, Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins,
    CommandInput, ContentReference, EntityId, FactId, FactValue, GameCommand, GameFact, GameInput,
    Obligation, ObligationAction, ObligationStatus, ObligationTransition, RecordId,
    ReferenceInventory, RuleReference,
};
use df_rules::{RulesCommandHandler, RulesCommandInput};
use df_types::{OperationId, RevisionLabel, SessionRevision};

/// Compiled source-owner mapping. Possession of these references grants no authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FulfillmentRegistration {
    pub obligation_definition: ContentReference,
    pub cause_definition: ContentReference,
    pub completion_definition: ContentReference,
    pub source: RuleReference,
    pub policy: RevisionLabel,
}

/// Exact prior accepted fact, independently selected by the trusted native owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedFulfillmentCause {
    pub fact: FactId,
    pub operation: OperationId,
    pub revision: SessionRevision,
}

/// Exact lifecycle action and authored definition selected by the native owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FulfillmentAction<'a> {
    pub kind: DebtAction,
    pub definition: &'a ContentReference,
}

/// Current source and actor-control admission, separate from structural membership checks.
/// Implementations must be pure and bounded, validate the complete registration under current
/// pins/content/source rights, and authorize this action for the exact obligation and member.
/// No client-provided registration, trace identity or content-reference presence grants authority.
pub trait FulfillmentSourceOwner {
    type Refusal;

    fn admit(
        &self,
        current: &Checkpoint,
        registration: &FulfillmentRegistration,
        action: FulfillmentAction<'_>,
        command: &CommandInput,
        obligation: &Obligation,
        cause: &GameFact,
    ) -> Result<(), Self::Refusal>;
}

/// Explicit bounds for all checkpoint, inventory and admitted record work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FulfillmentLimits {
    pub maximum_checkpoint_bytes: usize,
    pub maximum_pass_bytes: usize,
    pub maximum_inventory_records: usize,
    pub checkpoint: CheckpointLimits,
}

#[derive(Debug, Eq, PartialEq)]
pub enum FulfillmentError<R> {
    Capacity,
    Snapshot(CheckpointError),
    UnsupportedInput,
    StaleCommand,
    PendingResolution,
    AlreadyAccepted,
    UnknownObligation,
    InvalidRegistration,
    MissingAcceptedCause,
    WrongCause,
    WrongSubjects,
    WrongCommand,
    DuplicateFact,
    Source(R),
    Lifecycle(DebtTransitionRefusal),
    RevisionExhausted,
    InvalidCandidate(CheckpointError),
}

/// A bounded exact request captured independently of client input. The output fact identity
/// is supplied by the native owner and cannot collide with retained history. References and
/// source admission are rechecked on every stage call. This handler performs no I/O or commit.
pub struct ObligationFulfillmentHandler<'a, Owner> {
    pub owner: &'a Owner,
    pub current_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub registration: &'a FulfillmentRegistration,
    pub obligation: RecordId,
    pub action: DebtAction,
    pub action_definition: &'a ContentReference,
    pub cause: AcceptedFulfillmentCause,
    pub action_fact: FactId,
    pub limits: FulfillmentLimits,
}

impl<Owner: FulfillmentSourceOwner> RulesCommandHandler
    for ObligationFulfillmentHandler<'_, Owner>
{
    type Rejection = FulfillmentError<Owner::Refusal>;

    fn pins(&self) -> &CheckpointPins {
        self.admitted_pins
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        self.check_capacity(current, input.command)?;
        current
            .validate_resume(self.current_basis, self.admitted_pins)
            .map_err(FulfillmentError::Snapshot)?;
        let GameInput::Game(command) = input.command else {
            return Err(FulfillmentError::UnsupportedInput);
        };
        if !input.supplied_draws.is_empty() {
            return Err(FulfillmentError::UnsupportedInput);
        }
        if command.basis != self.current_basis
            || command.observed_revision != self.current_basis.revision
        {
            return Err(FulfillmentError::StaleCommand);
        }
        // Rebasing an unrelated interrupted mechanic is not an obligation transition.
        if !current.state().pending.is_empty() {
            return Err(FulfillmentError::PendingResolution);
        }
        if current
            .state()
            .decisions
            .iter()
            .any(|decision| decision.operation == command.operation)
        {
            return Err(FulfillmentError::AlreadyAccepted);
        }
        let obligation = current
            .state()
            .obligations
            .iter()
            .find(|obligation| obligation.id == self.obligation)
            .ok_or(FulfillmentError::UnknownObligation)?;
        self.validate_registration(current, obligation)?;
        if self.action == DebtAction::Fulfill
            && self.action_definition != &self.registration.completion_definition
        {
            return Err(FulfillmentError::InvalidRegistration);
        }
        if self.action_definition.package != current.pins().content.package
            || !self.inventory.content.contains(self.action_definition)
        {
            return Err(FulfillmentError::InvalidRegistration);
        }
        let cause = current
            .state()
            .facts
            .iter()
            .find(|fact| fact.id == self.cause.fact)
            .ok_or(FulfillmentError::MissingAcceptedCause)?;
        if cause.operation != self.cause.operation
            || cause.revision != self.cause.revision
            || !current.state().decisions.iter().any(|decision| {
                decision.operation == self.cause.operation
                    && decision.revision == self.cause.revision
                    && decision.facts.contains(&self.cause.fact)
            })
        {
            return Err(FulfillmentError::MissingAcceptedCause);
        }
        let FactValue::ContentEvent {
            definition,
            subjects,
        } = &cause.value
        else {
            return Err(FulfillmentError::WrongCause);
        };
        if definition != &self.registration.cause_definition {
            return Err(FulfillmentError::WrongCause);
        }
        if subjects.as_slice() != [obligation.obligor, obligation.beneficiary] {
            return Err(FulfillmentError::WrongSubjects);
        }
        let GameCommand::ProposeAction {
            actor,
            action,
            targets,
            choices,
        } = &command.command
        else {
            return Err(FulfillmentError::WrongCommand);
        };
        if *actor != obligation.obligor
            || action != &self.registration.obligation_definition
            || targets.as_slice() != [obligation.beneficiary]
            || !choices.is_empty()
            || !current
                .state()
                .entities
                .iter()
                .any(|entity| entity.id == obligation.obligor)
            || !current
                .state()
                .entities
                .iter()
                .any(|entity| entity.id == obligation.beneficiary)
            || !current
                .state()
                .members
                .iter()
                .any(|link| link.member == command.member)
        {
            return Err(FulfillmentError::WrongCommand);
        }
        if current
            .state()
            .facts
            .iter()
            .any(|fact| fact.id == self.action_fact)
        {
            return Err(FulfillmentError::DuplicateFact);
        }
        self.owner
            .admit(
                current,
                self.registration,
                FulfillmentAction {
                    kind: self.action,
                    definition: self.action_definition,
                },
                command,
                obligation,
                cause,
            )
            .map_err(FulfillmentError::Source)?;
        let view = CanonicalObligation {
            record: obligation,
            basis: &self.current_basis,
        };
        let transition = propose_debt_transition(
            &view,
            DebtTransitionRequest {
                obligation_id: &self.obligation,
                expected_basis: &self.current_basis,
                at: &current.state().logical_time.ticks,
                action: self.action,
                authorization: DebtAuthorization::Approved,
                action_provenance: self.action_definition,
            },
        );
        let proposal = match transition {
            DebtTransitionOutcome::Proposed(proposal) => proposal,
            DebtTransitionOutcome::Refused(reason) => {
                return Err(FulfillmentError::Lifecycle(reason));
            }
        };
        let mut next = self.current_basis;
        next.revision = next
            .revision
            .next_sequence()
            .map_err(|_| FulfillmentError::RevisionExhausted)?;
        let mut state = current.state().clone();
        let record = state
            .obligations
            .iter_mut()
            .find(|record| record.id == proposal.obligation_id)
            .ok_or(FulfillmentError::UnknownObligation)?;
        record.status = match proposal.status {
            DebtStatus::Fulfilled => ObligationStatus::Fulfilled,
            DebtStatus::Broken => ObligationStatus::Broken,
            DebtStatus::Expired => ObligationStatus::Expired,
            DebtStatus::Cancelled => ObligationStatus::Cancelled,
            DebtStatus::Active => return Err(FulfillmentError::WrongCommand),
        };
        record.transition = Some(ObligationTransition {
            action: match proposal.action {
                DebtAction::Fulfill => ObligationAction::Fulfill,
                DebtAction::Break => ObligationAction::Break,
                DebtAction::Expire => ObligationAction::Expire,
                DebtAction::Cancel => ObligationAction::Cancel,
            },
            source: self.action_definition.clone(),
            fact: self.action_fact,
            source_policy: self.registration.policy.clone(),
            at: current.state().logical_time,
        });
        state
            .facts
            .try_reserve_exact(1)
            .map_err(|_| FulfillmentError::Capacity)?;
        state
            .decisions
            .try_reserve_exact(1)
            .map_err(|_| FulfillmentError::Capacity)?;
        state.facts.push(GameFact {
            id: self.action_fact,
            revision: next.revision,
            operation: command.operation,
            ordinal: 0,
            cause: Some(cause.id),
            // Completion may not widen disclosure beyond the committed cause.
            audience: cause.audience.clone(),
            value: FactValue::ContentEvent {
                definition: self.action_definition.clone(),
                subjects: vec![obligation.obligor, obligation.beneficiary],
            },
        });
        state.decisions.push(AcceptedDecision {
            operation: command.operation,
            revision: next.revision,
            facts: vec![self.action_fact],
            draws: vec![],
            effects: vec![],
            source_policy: self.registration.policy.clone(),
            semantic_output: None,
        });
        Checkpoint::new(
            current.schema(),
            next,
            current.pins().clone(),
            state,
            ReferenceInventory {
                rules: self.inventory.rules,
                content: self.inventory.content,
                resources: self.inventory.resources,
                assets: self.inventory.assets,
            },
            self.limits.checkpoint,
        )
        .map_err(FulfillmentError::InvalidCandidate)
    }
}

impl<Owner: FulfillmentSourceOwner> ObligationFulfillmentHandler<'_, Owner> {
    fn check_capacity(
        &self,
        current: &Checkpoint,
        input: &GameInput,
    ) -> Result<(), FulfillmentError<Owner::Refusal>> {
        let limits = self.limits;
        if limits.maximum_checkpoint_bytes == 0
            || limits.maximum_inventory_records == 0
            || limits.checkpoint.maximum_retained_bytes == 0
            || limits.checkpoint.maximum_retained_bytes > limits.maximum_checkpoint_bytes
        {
            return Err(FulfillmentError::Capacity);
        }
        let current_bytes = current.retained_bytes().ok_or(FulfillmentError::Capacity)?;
        let registration = self.registration;
        let mut registration_bytes = std::mem::size_of::<FulfillmentRegistration>();
        for label in [
            &registration.obligation_definition.package,
            &registration.obligation_definition.entry,
            &registration.cause_definition.package,
            &registration.cause_definition.entry,
            &registration.completion_definition.package,
            &registration.completion_definition.entry,
            &registration.source.catalog,
            &registration.source.source,
            &registration.source.entry,
            &registration.source.clause,
            &registration.policy,
            &self.action_definition.package,
            &self.action_definition.entry,
        ] {
            registration_bytes = registration_bytes
                .checked_add(label.retained_heap_bytes())
                .ok_or(FulfillmentError::Capacity)?;
        }
        let pass_bytes = limits
            .maximum_checkpoint_bytes
            .checked_mul(2)
            .and_then(|scratch| scratch.checked_add(current_bytes))
            .and_then(|total| total.checked_add(input.retained_bytes()?))
            .and_then(|total| total.checked_add(registration_bytes))
            .ok_or(FulfillmentError::Capacity)?;
        let inventory_records = [
            self.inventory.rules.len(),
            self.inventory.content.len(),
            self.inventory.resources.len(),
            self.inventory.assets.len(),
        ]
        .into_iter()
        .try_fold(0usize, |record_count, next_collection_records| {
            record_count.checked_add(next_collection_records)
        })
        .ok_or(FulfillmentError::Capacity)?;
        if current_bytes > limits.maximum_checkpoint_bytes
            || pass_bytes > limits.maximum_pass_bytes
            || inventory_records > limits.maximum_inventory_records
        {
            return Err(FulfillmentError::Capacity);
        }
        Ok(())
    }

    fn validate_registration(
        &self,
        current: &Checkpoint,
        obligation: &Obligation,
    ) -> Result<(), FulfillmentError<Owner::Refusal>> {
        let registration = self.registration;
        if obligation.definition != registration.obligation_definition
            || obligation.agreement.source != registration.obligation_definition
            || registration.source.catalog != current.pins().rules.catalog
            || !self.inventory.rules.contains(&registration.source)
            || [
                &registration.obligation_definition,
                &registration.cause_definition,
                &registration.completion_definition,
            ]
            .into_iter()
            .any(|definition| {
                definition.package != current.pins().content.package
                    || !self.inventory.content.contains(definition)
            })
        {
            return Err(FulfillmentError::InvalidRegistration);
        }
        Ok(())
    }
}

/// Complete uses the current canonical tick coordinate without introducing a deadline or
/// converting units. Agreement provenance remains the exact original admitted terms reference;
/// the canonical completion fact separately retains the accepted action fact identity.
struct CanonicalObligation<'a> {
    record: &'a Obligation,
    basis: &'a Basis,
}

impl ObligationView for CanonicalObligation<'_> {
    type Id = RecordId;
    type Party = EntityId;
    type Terms = ContentReference;
    type Basis = Basis;
    type LogicalTime = u64;
    type Provenance = ContentReference;

    fn obligation_id(&self) -> &RecordId {
        &self.record.id
    }
    fn debtor(&self) -> &EntityId {
        &self.record.obligor
    }
    fn creditor(&self) -> &EntityId {
        &self.record.beneficiary
    }
    fn terms(&self) -> &ContentReference {
        &self.record.terms
    }
    fn basis(&self) -> &Basis {
        self.basis
    }
    fn status(&self) -> DebtStatus {
        match self.record.status {
            ObligationStatus::Active => DebtStatus::Active,
            ObligationStatus::Fulfilled => DebtStatus::Fulfilled,
            ObligationStatus::Broken => DebtStatus::Broken,
            ObligationStatus::Expired => DebtStatus::Expired,
            ObligationStatus::Cancelled => DebtStatus::Cancelled,
        }
    }
    fn last_transition_at(&self) -> &u64 {
        &self.record.agreement.at.ticks
    }
    fn due_at(&self) -> Option<&u64> {
        self.record.due.as_ref().map(|time| &time.ticks)
    }
    fn agreement_provenance(&self) -> &ContentReference {
        &self.record.agreement.source
    }
}
