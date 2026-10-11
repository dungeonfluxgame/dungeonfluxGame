//! Source-bound attributed belief updates staged through the canonical checkpoint owner.

use df_knowledge::beliefs::{
    BeliefBasis, BeliefUpdate, BeliefValidationError, validate_belief_update,
};
use df_model::checkpoint::{
    AcceptedDecision, AttributedClaim, Basis, Checkpoint, CheckpointError, CheckpointLimits,
    CheckpointPins, CommandInput, ContentReference, EntityId, FactId, GameCommand, GameInput,
    ReferenceInventory, RuleReference,
};
use df_rules::{RulesCommandHandler, RulesCommandInput};
use df_types::RevisionLabel;

/// A caller-owned proposal that binds an attributed claim to the current world basis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttributedBeliefUpdate {
    pub basis: Basis,
    pub claim: AttributedClaim,
}

impl BeliefUpdate for AttributedBeliefUpdate {
    type Basis = Basis;
    type Actor = EntityId;
    type Subject = EntityId;
    type Claim = String;
    type Evidence = Vec<FactId>;

    fn basis(&self) -> &Self::Basis {
        &self.basis
    }

    fn actor(&self) -> &Self::Actor {
        &self.claim.holder
    }

    fn subject(&self) -> &Self::Subject {
        &self.claim.subject
    }

    fn claim(&self) -> &Self::Claim {
        &self.claim.claim
    }

    fn evidence(&self) -> &Self::Evidence {
        &self.claim.evidence
    }
}

/// Immutable current-source registration for one action that may record a belief.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeliefRegistration {
    pub action: ContentReference,
    pub source: RuleReference,
    pub claim_source: ContentReference,
    pub decision_policy: RevisionLabel,
}

/// Trusted native observation and command owner for a belief update.
///
/// The inherited `BeliefBasis` checks authorize only this claim's attributed provenance.
/// They must not compare the claim to canonical truth or grant disclosure to an audience.
pub trait BeliefSourceOwner: BeliefBasis<AttributedBeliefUpdate> {
    type Refusal;

    fn admit(
        &self,
        current: &Checkpoint,
        registration: &BeliefRegistration,
        command: &CommandInput,
        _update: &AttributedBeliefUpdate,
    ) -> Result<(), Self::Refusal>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeliefStagingLimits {
    pub maximum_checkpoint_bytes: usize,
    pub maximum_pass_bytes: usize,
    pub maximum_inventory_records: usize,
    pub checkpoint: CheckpointLimits,
}

#[derive(Debug, Eq, PartialEq)]
pub enum BeliefStagingError<R> {
    Capacity,
    Snapshot(CheckpointError),
    UnsupportedInput,
    StaleCommand,
    PendingResolution,
    AlreadyAccepted,
    MissingProducer,
    InvalidRegistration,
    WrongCommand,
    DuplicateClaim,
    Source(R),
    Belief(BeliefValidationError),
    RevisionExhausted,
    InvalidCandidate(CheckpointError),
}

/// A trusted candidate captured outside client-controlled command data.
///
/// Run through the registered rules/session boundary. Only the session's durable owner
/// commits or publishes the resulting checkpoint. `None` refuses a missing producer.
pub struct BeliefStagingHandler<'a, Owner> {
    pub owner: &'a Owner,
    pub current_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub registration: &'a BeliefRegistration,
    pub update: Option<&'a AttributedBeliefUpdate>,
    pub limits: BeliefStagingLimits,
}

impl<Owner: BeliefSourceOwner> RulesCommandHandler for BeliefStagingHandler<'_, Owner> {
    type Rejection = BeliefStagingError<Owner::Refusal>;

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
            .map_err(BeliefStagingError::Snapshot)?;

        let GameInput::Game(command) = input.command else {
            return Err(BeliefStagingError::UnsupportedInput);
        };
        if !input.supplied_draws.is_empty() {
            return Err(BeliefStagingError::UnsupportedInput);
        }
        if command.basis != self.current_basis
            || command.observed_revision != self.current_basis.revision
        {
            return Err(BeliefStagingError::StaleCommand);
        }
        if !current.state().pending.is_empty() {
            return Err(BeliefStagingError::PendingResolution);
        }
        if current
            .state()
            .decisions
            .iter()
            .any(|decision| decision.operation == command.operation)
        {
            return Err(BeliefStagingError::AlreadyAccepted);
        }

        let update = self.update.ok_or(BeliefStagingError::MissingProducer)?;
        let registration = self.registration;
        if *self.owner.current_basis() != current.basis() || update.basis != current.basis() {
            return Err(BeliefStagingError::Belief(
                BeliefValidationError::StaleBasis,
            ));
        }
        if registration.source.catalog != current.pins().rules.catalog
            || !self.inventory.rules.contains(&registration.source)
            || [&registration.action, &registration.claim_source]
                .into_iter()
                .any(|reference| {
                    reference.package != current.pins().content.package
                        || !self.inventory.content.contains(reference)
                })
            || update.claim.source != registration.claim_source
        {
            return Err(BeliefStagingError::InvalidRegistration);
        }

        let GameCommand::ProposeAction { actor, action, .. } = &command.command else {
            return Err(BeliefStagingError::WrongCommand);
        };
        if action != &registration.action
            || update.claim.holder != *actor
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
            return Err(BeliefStagingError::WrongCommand);
        }
        if current
            .state()
            .beliefs
            .iter()
            .any(|claim| claim.id == update.claim.id)
        {
            return Err(BeliefStagingError::DuplicateClaim);
        }
        if update.claim.evidence.iter().any(|evidence| {
            !current
                .state()
                .facts
                .iter()
                .any(|fact| fact.id == *evidence)
        }) {
            return Err(BeliefStagingError::Belief(
                BeliefValidationError::UndisclosedEvidence,
            ));
        }

        self.owner
            .admit(current, registration, command, update)
            .map_err(BeliefStagingError::Source)?;
        let validated = validate_belief_update(self.owner, update.clone())
            .map_err(BeliefStagingError::Belief)?;

        let mut next = self.current_basis;
        next.revision = next
            .revision
            .next_sequence()
            .map_err(|_| BeliefStagingError::RevisionExhausted)?;
        let mut state = current.state().clone();
        state
            .beliefs
            .try_reserve_exact(1)
            .map_err(|_| BeliefStagingError::Capacity)?;
        state.beliefs.push(validated.into_update().claim);
        state
            .decisions
            .try_reserve_exact(1)
            .map_err(|_| BeliefStagingError::Capacity)?;
        state.decisions.push(AcceptedDecision {
            operation: command.operation,
            revision: next.revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: registration.decision_policy.clone(),
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
        .map_err(BeliefStagingError::InvalidCandidate)
    }
}

impl<Owner: BeliefSourceOwner> BeliefStagingHandler<'_, Owner> {
    fn check_capacity(
        &self,
        current: &Checkpoint,
        input: &GameInput,
    ) -> Result<(), BeliefStagingError<Owner::Refusal>> {
        let limits = self.limits;
        let records = [
            self.inventory.rules.len(),
            self.inventory.content.len(),
            self.inventory.resources.len(),
            self.inventory.assets.len(),
        ]
        .into_iter()
        .try_fold(0usize, |count, next| count.checked_add(next))
        .ok_or(BeliefStagingError::Capacity)?;
        if limits.maximum_checkpoint_bytes == 0
            || limits.maximum_pass_bytes == 0
            || limits.maximum_inventory_records == 0
            || limits.checkpoint.maximum_retained_bytes == 0
            || limits.checkpoint.maximum_retained_bytes > limits.maximum_checkpoint_bytes
            || records > limits.maximum_inventory_records
        {
            return Err(BeliefStagingError::Capacity);
        }
        let update = self.update.ok_or(BeliefStagingError::MissingProducer)?;
        let text_bytes = update.claim.claim.capacity();
        let evidence_capacity = update.claim.evidence.capacity();
        let audience_capacity = match &update.claim.audience {
            df_model::checkpoint::AudienceScope::Members(members) => members.capacity(),
            df_model::checkpoint::AudienceScope::Shared
            | df_model::checkpoint::AudienceScope::Host => 0,
        };
        if text_bytes > limits.checkpoint.maximum_text_bytes
            || update.claim.evidence.len() > limits.checkpoint.maximum_records
            || evidence_capacity > limits.checkpoint.maximum_records
            || audience_capacity > limits.checkpoint.maximum_records
        {
            return Err(BeliefStagingError::Capacity);
        }
        let audience_bytes = audience_capacity
            .checked_mul(std::mem::size_of::<df_types::MemberId>())
            .filter(|bytes| *bytes <= limits.maximum_pass_bytes)
            .ok_or(BeliefStagingError::Capacity)?;

        let mut request_bytes = std::mem::size_of::<BeliefRegistration>()
            .checked_add(std::mem::size_of::<AttributedBeliefUpdate>())
            .and_then(|bytes| bytes.checked_add(text_bytes))
            .and_then(|bytes| {
                evidence_capacity
                    .checked_mul(std::mem::size_of::<FactId>())
                    .and_then(|evidence| bytes.checked_add(evidence))
            })
            .and_then(|bytes| bytes.checked_add(audience_bytes))
            .ok_or(BeliefStagingError::Capacity)?;
        let mut add = |bytes: usize| -> Result<(), BeliefStagingError<Owner::Refusal>> {
            request_bytes = request_bytes
                .checked_add(bytes)
                .filter(|bytes| *bytes <= limits.maximum_pass_bytes)
                .ok_or(BeliefStagingError::Capacity)?;
            Ok(())
        };
        for label in [
            &self.registration.action.package,
            &self.registration.action.entry,
            &self.registration.source.catalog,
            &self.registration.source.source,
            &self.registration.source.entry,
            &self.registration.source.clause,
            &self.registration.claim_source.package,
            &self.registration.claim_source.entry,
            &self.registration.decision_policy,
            &update.claim.source.package,
            &update.claim.source.entry,
        ] {
            add(label.retained_heap_bytes())?;
        }
        let current_bytes = current
            .retained_bytes()
            .ok_or(BeliefStagingError::Capacity)?;
        let total = limits
            .maximum_checkpoint_bytes
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(current_bytes))
            .and_then(|bytes| bytes.checked_add(request_bytes))
            .and_then(|bytes| bytes.checked_add(input.retained_bytes()?))
            .ok_or(BeliefStagingError::Capacity)?;
        if current_bytes > limits.maximum_checkpoint_bytes || total > limits.maximum_pass_bytes {
            return Err(BeliefStagingError::Capacity);
        }
        Ok(())
    }
}

/// Select journal entries from the checkpoint supplied by the Session/projection
/// owner, using Knowledge's exact source and observer checks. A checkpoint returned
/// by BeliefStagingHandler is still a staged proposal; selecting its journal does
/// not commit it or authorize publication. Revalidate authoritative state before
/// serialization. No claim, fact, decision, or intent is changed.
pub fn select_journal<'a>(
    current: &'a Checkpoint,
    expected: Basis,
    admitted: &CheckpointPins,
    observer: df_knowledge::perception::ObserverScope,
    maximum_scan_records: usize,
    maximum_record_comparisons: usize,
    maximum_selected_entries: usize,
) -> Result<Vec<&'a df_model::checkpoint::JournalEntry>, df_knowledge::perception::PerceptionError>
{
    df_knowledge::perception::perceive_journal(
        current,
        expected,
        admitted,
        observer,
        maximum_scan_records,
        maximum_record_comparisons,
        maximum_selected_entries,
    )
}
