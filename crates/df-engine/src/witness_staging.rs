//! Current source-admitted witnesses through the registered rules/session boundary.

use df_knowledge::witness::{
    WitnessEligibility, WitnessError, WitnessLimits, WitnessRoute, propose_witness_grants,
};
use df_model::checkpoint::{
    AcceptedDecision, Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins,
    CommandInput, ContentReference, GameCommand, GameInput, KnowledgeGrant, ReferenceInventory,
    RuleReference,
};
use df_rules::{RulesCommandHandler, RulesCommandInput};
use df_types::RevisionLabel;

/// Compiled action/source mapping and exact witness policy. References grant no authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WitnessRegistration {
    pub action: ContentReference,
    pub source: RuleReference,
    pub witness_policy: ContentReference,
    pub decision_policy: RevisionLabel,
}

/// Independently admit actual current perception/contact and source rights for the entire batch.
/// Implementations are pure and bounded. They bind each native eligibility's route, recipient's
/// current member-character link and exact canonical witness to this registration/current basis,
/// together with current actor control and the command-to-batch binding.
/// Relationship delivery needs an actual directional contact and the source's permitted state;
/// a score, proximity, global fact, client flag, belief or memory cannot supply this authority.
/// This crate does not produce visibility/contact or qualify an authored source policy.
pub trait WitnessSourceOwner {
    type Refusal;

    fn admit(
        &self,
        current: &Checkpoint,
        registration: &WitnessRegistration,
        command: &CommandInput,
        eligible: &[WitnessEligibility<'_>],
    ) -> Result<(), Self::Refusal>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WitnessStagingLimits {
    pub maximum_checkpoint_bytes: usize,
    pub maximum_pass_bytes: usize,
    pub maximum_inventory_records: usize,
    pub checkpoint: CheckpointLimits,
    pub witness: WitnessLimits,
}

#[derive(Debug, Eq, PartialEq)]
pub enum WitnessStagingError<R> {
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
    Witness(WitnessError),
    RevisionExhausted,
    InvalidCandidate(CheckpointError),
}

/// Trusted native batch, captured independently of client input. Invoke via
/// `decide_registered_command` inside `SessionEngine`; only `DurableOwner` commits/publishes.
/// The source owner revalidates the complete batch immediately before proposing grants.
/// `None` refuses a missing producer; `Some([])` explicitly admits a no-change decision.
pub struct WitnessGrantHandler<'a, Owner> {
    pub owner: &'a Owner,
    pub current_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub registration: &'a WitnessRegistration,
    pub eligible: Option<&'a [WitnessEligibility<'a>]>,
    pub limits: WitnessStagingLimits,
}

impl<Owner: WitnessSourceOwner> RulesCommandHandler for WitnessGrantHandler<'_, Owner> {
    type Rejection = WitnessStagingError<Owner::Refusal>;

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
            .map_err(WitnessStagingError::Snapshot)?;
        let GameInput::Game(command) = input.command else {
            return Err(WitnessStagingError::UnsupportedInput);
        };
        if !input.supplied_draws.is_empty() {
            return Err(WitnessStagingError::UnsupportedInput);
        }
        if command.basis != self.current_basis
            || command.observed_revision != self.current_basis.revision
        {
            return Err(WitnessStagingError::StaleCommand);
        }
        if !current.state().pending.is_empty() {
            return Err(WitnessStagingError::PendingResolution);
        }
        if current
            .state()
            .decisions
            .iter()
            .any(|decision| decision.operation == command.operation)
        {
            return Err(WitnessStagingError::AlreadyAccepted);
        }
        let eligible = self.eligible.ok_or(WitnessStagingError::MissingProducer)?;
        let registration = self.registration;
        if registration.source.catalog != current.pins().rules.catalog
            || !self.inventory.rules.contains(&registration.source)
            || [&registration.action, &registration.witness_policy]
                .into_iter()
                .any(|reference| {
                    reference.package != current.pins().content.package
                        || !self.inventory.content.contains(reference)
                })
        {
            return Err(WitnessStagingError::InvalidRegistration);
        }
        let GameCommand::ProposeAction { actor, action, .. } = &command.command else {
            return Err(WitnessStagingError::WrongCommand);
        };
        if action != &registration.action
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
            return Err(WitnessStagingError::WrongCommand);
        }
        self.owner
            .admit(current, registration, command, eligible)
            .map_err(WitnessStagingError::Source)?;
        let proposal = propose_witness_grants(
            current,
            self.current_basis,
            self.admitted_pins,
            &registration.witness_policy,
            eligible,
            self.limits.witness,
        )
        .map_err(WitnessStagingError::Witness)?;
        let mut next_basis = self.current_basis;
        next_basis.revision = next_basis
            .revision
            .next_sequence()
            .map_err(|_| WitnessStagingError::RevisionExhausted)?;
        let mut state = current.state().clone();
        state
            .knowledge
            .try_reserve_exact(proposal.grants().len())
            .map_err(|_| WitnessStagingError::Capacity)?;
        state.knowledge.extend_from_slice(proposal.grants());
        state
            .decisions
            .try_reserve_exact(1)
            .map_err(|_| WitnessStagingError::Capacity)?;
        // Applying knowledge does not redeclare the original accepted fact or rewrite its cause.
        state.decisions.push(AcceptedDecision {
            operation: command.operation,
            revision: next_basis.revision,
            facts: vec![],
            draws: vec![],
            effects: vec![],
            source_policy: registration.decision_policy.clone(),
            semantic_output: None,
        });
        Checkpoint::new(
            current.schema(),
            next_basis,
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
        .map_err(WitnessStagingError::InvalidCandidate)
    }
}

impl<Owner: WitnessSourceOwner> WitnessGrantHandler<'_, Owner> {
    fn check_capacity(
        &self,
        current: &Checkpoint,
        input: &GameInput,
    ) -> Result<(), WitnessStagingError<Owner::Refusal>> {
        let limits = self.limits;
        let eligible = self.eligible.unwrap_or(&[]);
        if limits.maximum_checkpoint_bytes == 0
            || limits.maximum_inventory_records == 0
            || limits.checkpoint.maximum_retained_bytes == 0
            || limits.checkpoint.maximum_retained_bytes > limits.maximum_checkpoint_bytes
            || limits.witness.maximum_candidates > 4096
            || eligible.len() > limits.witness.maximum_candidates
            || limits.witness.maximum_grants > limits.witness.maximum_candidates
        {
            return Err(WitnessStagingError::Capacity);
        }
        let records = [
            self.inventory.rules.len(),
            self.inventory.content.len(),
            self.inventory.resources.len(),
            self.inventory.assets.len(),
        ]
        .into_iter()
        .try_fold(0usize, |record_count, collection_len| {
            record_count.checked_add(collection_len)
        })
        .ok_or(WitnessStagingError::Capacity)?;
        if records > limits.maximum_inventory_records {
            return Err(WitnessStagingError::Capacity);
        }
        let mut request_bytes = eligible
            .len()
            .checked_mul(std::mem::size_of::<WitnessEligibility<'_>>())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<WitnessRegistration>()))
            .ok_or(WitnessStagingError::Capacity)?;
        let mut add_request_bytes =
            |bytes: usize| -> Result<(), WitnessStagingError<Owner::Refusal>> {
                request_bytes = request_bytes
                    .checked_add(bytes)
                    .filter(|bytes| *bytes <= limits.maximum_pass_bytes)
                    .ok_or(WitnessStagingError::Capacity)?;
                Ok(())
            };
        for label in [
            &self.registration.action.package,
            &self.registration.action.entry,
            &self.registration.source.catalog,
            &self.registration.source.source,
            &self.registration.source.entry,
            &self.registration.source.clause,
            &self.registration.witness_policy.package,
            &self.registration.witness_policy.entry,
            &self.registration.decision_policy,
        ] {
            add_request_bytes(label.retained_heap_bytes())?;
        }
        for candidate in eligible {
            if let WitnessRoute::Relationship { permitted_state } = candidate.route {
                add_request_bytes(permitted_state.retained_heap_bytes())?;
            }
        }
        let current_bytes = current
            .retained_bytes()
            .ok_or(WitnessStagingError::Capacity)?;
        let grant_bytes = limits
            .witness
            .maximum_grants
            .checked_mul(std::mem::size_of::<KnowledgeGrant>())
            .ok_or(WitnessStagingError::Capacity)?;
        let total = limits
            .maximum_checkpoint_bytes
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(current_bytes))
            .and_then(|bytes| bytes.checked_add(grant_bytes))
            .and_then(|bytes| bytes.checked_add(request_bytes))
            .and_then(|bytes| bytes.checked_add(input.retained_bytes()?))
            .ok_or(WitnessStagingError::Capacity)?;
        if current_bytes > limits.maximum_checkpoint_bytes || total > limits.maximum_pass_bytes {
            return Err(WitnessStagingError::Capacity);
        }
        Ok(())
    }
}
