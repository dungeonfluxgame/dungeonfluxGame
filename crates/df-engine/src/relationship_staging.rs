//! Recomputed categorical reactions attached to an actual source-selected command transition.
//! The session alone commits the complete checkpoint; private proposals never become facts.

use df_interaction::reactions::{
    ReactionEntry, ReactionError, ReactionLimits, ReactionOutcome, ReactionPolicy, ReactionRequest,
    ReactionSourceOwner, react,
};
use df_interaction::relationships::{
    RelationshipAxes, RelationshipAxis, RelationshipAxisValue, RelationshipChange,
    RelationshipOwner, RelationshipRefusal, RelationshipView, propose_relationship_change,
};
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, CheckpointLimits, CheckpointPins, CommandInput, EntityId,
    GameCommand, GameInput, ReferenceInventory, Relationship, RelationshipAxisProvenance,
    RelationshipAxisState,
};
use df_rules::{InvocationError, RulesCommandHandler, RulesCommandInput, stage_handler};
use df_types::RevisionLabel;

/// Native admission for this exact command and social consequence pass, distinct from entry
/// validation. Implementations must check current source rights, controlled actor, permitted
/// requests and complete authored entries under the full checkpoint pins. They perform no I/O.
/// Inventory membership, client selections and a ReactionSourceOwner alone grant no control.
pub trait RelationshipApplicationOwner: ReactionSourceOwner {
    type Refusal;

    fn admit_application(
        &self,
        current: &Checkpoint,
        command: &CommandInput,
        requests: &[ReactionRequest],
        entries: &[ReactionEntry],
    ) -> Result<(), Self::Refusal>;
}

/// Count and retained-capacity bounds for this detached pass. maximum_work reserves the
/// reaction budgets and local actor/directional scans; native owner/inner handler work and
/// canonical validation retain their own bounded contracts and are not charged as reaction work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelationshipStagingLimits {
    pub maximum_reactions: usize,
    pub maximum_relationships: usize,
    pub maximum_work: usize,
    pub maximum_inventory_records: usize,
    pub maximum_checkpoint_bytes: usize,
    pub maximum_pass_bytes: usize,
    pub checkpoint: CheckpointLimits,
    pub reaction: ReactionLimits,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RelationshipStagingError<CommandRefusal, SourceRefusal> {
    Capacity,
    Snapshot(CheckpointError),
    UnsupportedInput,
    StaleCommand,
    PendingResolution,
    WrongActor,
    DuplicateDirection,
    Source(SourceRefusal),
    Command(InvocationError<CommandRefusal>),
    Reaction(ReactionError),
    RelationshipConflict,
    InvalidCandidate(CheckpointError),
}

/// Private selected checkpoint and exact recomputed outcomes for the native caller. NoReaction
/// remains typed and does not invalidate the source-admitted base command. Evidence for the
/// categorical projection remains transient; the independent seven-axis values keep their own
/// source and accepted-fact provenance. Snapshot reload and exact operation receipts are supported.
pub struct RelationshipTransition {
    pub checkpoint: Checkpoint,
    pub reactions: Vec<ReactionOutcome>,
}

/// One source-admitted change to one directional axis in a staged checkpoint candidate.
pub struct RelationshipAxisChange {
    pub subject: EntityId,
    pub target: EntityId,
    pub axis: RelationshipAxis,
    pub replacement: RelationshipAxisState,
}

/// The native owner proves that this accepted event/policy may change the exact relationship.
/// The durable checkpoint constructor independently checks ledger and pinned-reference lineage.
pub trait RelationshipAxisOwner:
    RelationshipOwner<
        Subject = EntityId,
        Basis = Basis,
        Value = RevisionLabel,
        Provenance = RelationshipAxisProvenance,
    >
{
    fn admit_axis_change(
        &self,
        candidate: &Checkpoint,
        change: &RelationshipAxisChange,
    ) -> Result<(), Self::Refusal>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum RelationshipAxisStagingError<Refusal> {
    Capacity,
    Snapshot(CheckpointError),
    DuplicateDirection,
    MissingRelationship,
    Source(Refusal),
    Proposal(RelationshipRefusal<Refusal>),
    InvalidCandidate(CheckpointError),
}

/// Bounds for a detached axis pass and its canonical checkpoint reconstruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelationshipAxisStagingLimits {
    pub maximum_changes: usize,
    pub maximum_inventory_records: usize,
    pub maximum_checkpoint_bytes: usize,
    pub maximum_pass_bytes: usize,
    pub checkpoint: CheckpointLimits,
}

/// Applies bounded, source-admitted axis proposals to a command-staged candidate.
/// The caller supplies the captured full basis independently of the candidate. The input
/// checkpoint remains untouched; canonical construction rechecks every accepted source link.
pub fn apply_relationship_axis_changes<Owner: RelationshipAxisOwner>(
    owner: &Owner,
    expected_basis: Basis,
    candidate: &Checkpoint,
    admitted_pins: &CheckpointPins,
    inventory: ReferenceInventory<'_>,
    changes: &[RelationshipAxisChange],
    limits: RelationshipAxisStagingLimits,
) -> Result<Checkpoint, RelationshipAxisStagingError<Owner::Refusal>> {
    candidate
        .validate_resume(expected_basis, admitted_pins)
        .map_err(RelationshipAxisStagingError::Snapshot)?;
    check_axis_capacity(candidate, &inventory, changes, limits)?;
    candidate
        .validate_admitted(expected_basis, admitted_pins, inventory, limits.checkpoint)
        .map_err(RelationshipAxisStagingError::Snapshot)?;
    for (index, change) in changes.iter().enumerate() {
        if changes.iter().take(index).any(|prior| {
            prior.subject == change.subject
                && prior.target == change.target
                && prior.axis == change.axis
        }) {
            return Err(RelationshipAxisStagingError::DuplicateDirection);
        }
    }

    let mut state = candidate.state().clone();
    for change in changes {
        owner
            .admit_axis_change(candidate, change)
            .map_err(RelationshipAxisStagingError::Source)?;
        let index = state
            .relationships
            .iter()
            .position(|record| record.subject == change.subject && record.object == change.target)
            .ok_or(RelationshipAxisStagingError::MissingRelationship)?;
        let original = state.relationships[index].clone();
        let axes = axes_from_relationship(&original);
        let proposal = propose_relationship_change(
            owner,
            RelationshipView {
                subject: &original.subject,
                target: &original.object,
                basis: &expected_basis,
                axes: &axes,
            },
            RelationshipChange {
                subject: &change.subject,
                target: &change.target,
                expected_basis: &expected_basis,
                axis: change.axis,
                replacement: RelationshipAxisValue::new(
                    change.replacement.value.clone(),
                    change.replacement.provenance.clone(),
                ),
            },
        )
        .map_err(RelationshipAxisStagingError::Proposal)?;
        apply_axes(&mut state.relationships[index], proposal.axes());
    }

    Checkpoint::new(
        candidate.schema(),
        candidate.basis(),
        candidate.pins().clone(),
        state,
        inventory,
        limits.checkpoint,
    )
    .map_err(RelationshipAxisStagingError::InvalidCandidate)
}

fn check_axis_capacity<R>(
    candidate: &Checkpoint,
    inventory: &ReferenceInventory<'_>,
    changes: &[RelationshipAxisChange],
    limits: RelationshipAxisStagingLimits,
) -> Result<(), RelationshipAxisStagingError<R>> {
    if limits.maximum_changes == 0
        || limits.maximum_inventory_records == 0
        || limits.maximum_checkpoint_bytes == 0
        || limits.maximum_pass_bytes == 0
        || limits.checkpoint.maximum_retained_bytes == 0
        || limits.checkpoint.maximum_retained_bytes > limits.maximum_checkpoint_bytes
    {
        return Err(RelationshipAxisStagingError::Capacity);
    }
    let maximum_axes = candidate
        .state()
        .relationships
        .len()
        .checked_mul(7)
        .ok_or(RelationshipAxisStagingError::Capacity)?;
    if changes.len() > maximum_axes
        || changes.len() > limits.maximum_changes
        || changes.len() > limits.checkpoint.maximum_records
    {
        return Err(RelationshipAxisStagingError::Capacity);
    }
    let inventory_records = [
        inventory.rules.len(),
        inventory.content.len(),
        inventory.resources.len(),
        inventory.assets.len(),
    ]
    .into_iter()
    .try_fold(0usize, |sum, count| sum.checked_add(count))
    .ok_or(RelationshipAxisStagingError::Capacity)?;
    let checkpoint_bytes = candidate
        .retained_bytes()
        .ok_or(RelationshipAxisStagingError::Capacity)?;
    let mut change_bytes = changes
        .len()
        .checked_mul(std::mem::size_of::<RelationshipAxisChange>())
        .ok_or(RelationshipAxisStagingError::Capacity)?;
    for change in changes {
        change_bytes = change_bytes
            .checked_add(change.replacement.value.retained_heap_bytes())
            .ok_or(RelationshipAxisStagingError::Capacity)?;
        let rule_reference = match &change.replacement.provenance {
            RelationshipAxisProvenance::AuthoredBaseline { source } => source,
            RelationshipAxisProvenance::AcceptedFact {
                source,
                source_policy,
                ..
            } => {
                change_bytes = change_bytes
                    .checked_add(source_policy.retained_heap_bytes())
                    .ok_or(RelationshipAxisStagingError::Capacity)?;
                source
            }
        };
        change_bytes = change_bytes
            .checked_add(rule_reference.package.retained_heap_bytes())
            .and_then(|total| total.checked_add(rule_reference.entry.retained_heap_bytes()))
            .ok_or(RelationshipAxisStagingError::Capacity)?;
    }
    let pass_bytes = limits
        .maximum_checkpoint_bytes
        .checked_mul(3)
        .and_then(|scratch| scratch.checked_add(checkpoint_bytes))
        .and_then(|total| total.checked_add(change_bytes))
        .ok_or(RelationshipAxisStagingError::Capacity)?;
    if checkpoint_bytes > limits.maximum_checkpoint_bytes
        || inventory_records > limits.maximum_inventory_records
        || pass_bytes > limits.maximum_pass_bytes
    {
        return Err(RelationshipAxisStagingError::Capacity);
    }
    Ok(())
}

fn axes_from_relationship(
    relationship: &Relationship,
) -> RelationshipAxes<RevisionLabel, RelationshipAxisProvenance> {
    RelationshipAxes::new([
        axis_value(&relationship.trust),
        axis_value(&relationship.affection),
        axis_value(&relationship.respect),
        axis_value(&relationship.fear),
        axis_value(&relationship.suspicion),
        axis_value(&relationship.debt),
        axis_value(&relationship.familiarity),
    ])
}

fn axis_value(
    state: &RelationshipAxisState,
) -> RelationshipAxisValue<RevisionLabel, RelationshipAxisProvenance> {
    RelationshipAxisValue::new(state.value.clone(), state.provenance.clone())
}

fn apply_axes(
    relationship: &mut Relationship,
    axes: &RelationshipAxes<RevisionLabel, RelationshipAxisProvenance>,
) {
    relationship.trust = axis_state(axes, RelationshipAxis::Trust);
    relationship.affection = axis_state(axes, RelationshipAxis::Affection);
    relationship.respect = axis_state(axes, RelationshipAxis::Respect);
    relationship.fear = axis_state(axes, RelationshipAxis::Fear);
    relationship.suspicion = axis_state(axes, RelationshipAxis::Suspicion);
    relationship.debt = axis_state(axes, RelationshipAxis::Debt);
    relationship.familiarity = axis_state(axes, RelationshipAxis::Familiarity);
}

fn axis_state(
    axes: &RelationshipAxes<RevisionLabel, RelationshipAxisProvenance>,
    axis: RelationshipAxis,
) -> RelationshipAxisState {
    let value = axes.axis(axis);
    RelationshipAxisState {
        value: value.value().clone(),
        provenance: value.provenance().clone(),
    }
}

/// Compose with a real source-selected command handler and invoke through decide_registered_command.
/// Requests and entries are native supplied, never client authored. Reactions use the unchanged
/// current checkpoint: this pass cannot claim that a newly staged event was already committed.
/// The base command must retain existing relationships. Every replacement is rechecked against
/// its original; conflicting siblings or a last reaction/constructor failure discard the pass.
pub struct RelationshipReactionHandler<'a, Handler, Owner> {
    pub command_handler: &'a Handler,
    pub owner: &'a Owner,
    pub current_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub requests: &'a [ReactionRequest],
    pub entries: &'a [ReactionEntry],
    pub limits: RelationshipStagingLimits,
}

impl<Handler, Owner> RelationshipReactionHandler<'_, Handler, Owner>
where
    Handler: RulesCommandHandler,
    Owner: RelationshipApplicationOwner,
{
    /// Return a fully validated next-revision checkpoint together with private typed outcomes.
    /// Admission and the actual command handler run before any policy validation or react call.
    /// Structural client-command validation and registry selector admission are supplied by
    /// decide_registered_command, which must wrap production use of this handler.
    pub fn stage_with_outcomes(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<RelationshipTransition, RelationshipStagingError<Handler::Rejection, Owner::Refusal>>
    {
        self.check_capacity(current, input.command)?;
        current
            .validate_resume(self.current_basis, self.admitted_pins)
            .map_err(RelationshipStagingError::Snapshot)?;
        let GameInput::Game(command) = input.command else {
            return Err(RelationshipStagingError::UnsupportedInput);
        };
        if command.basis != self.current_basis
            || command.observed_revision != self.current_basis.revision
            || self
                .requests
                .iter()
                .any(|request| request.expected_basis != self.current_basis)
        {
            return Err(RelationshipStagingError::StaleCommand);
        }
        if !current.state().pending.is_empty() {
            return Err(RelationshipStagingError::PendingResolution);
        }
        let actor = match &command.command {
            GameCommand::ProposeAction { actor, .. } => *actor,
            _ => return Err(RelationshipStagingError::UnsupportedInput),
        };
        if !current
            .state()
            .members
            .iter()
            .any(|link| link.member == command.member && link.character == Some(actor))
            || !current
                .state()
                .entities
                .iter()
                .any(|entity| entity.id == actor)
            || !current
                .state()
                .characters
                .iter()
                .any(|character| character.entity == actor && character.owner == command.member)
        {
            return Err(RelationshipStagingError::WrongActor);
        }
        self.owner
            .admit_application(current, command, self.requests, self.entries)
            .map_err(RelationshipStagingError::Source)?;
        let staged = stage_handler(
            self.command_handler,
            self.admitted_pins,
            input,
            current,
            self.limits.maximum_checkpoint_bytes,
        )
        .map_err(RelationshipStagingError::Command)?;
        if staged.state().relationships != current.state().relationships {
            return Err(RelationshipStagingError::RelationshipConflict);
        }
        for (index, relationship) in current.state().relationships.iter().enumerate() {
            if current
                .state()
                .relationships
                .iter()
                .take(index)
                .any(|prior| {
                    prior.subject == relationship.subject && prior.object == relationship.object
                })
            {
                return Err(RelationshipStagingError::DuplicateDirection);
            }
        }
        for (index, request) in self.requests.iter().enumerate() {
            if self
                .requests
                .iter()
                .take(index)
                .any(|prior| prior.npc == request.npc && prior.target == request.target)
            {
                return Err(RelationshipStagingError::DuplicateDirection);
            }
        }
        let policy = ReactionPolicy::new(
            current,
            self.entries,
            self.inventory(),
            self.owner,
            self.limits.reaction,
        )
        .map_err(RelationshipStagingError::Reaction)?;
        let mut reactions = Vec::new();
        reactions
            .try_reserve_exact(self.requests.len())
            .map_err(|_| RelationshipStagingError::Capacity)?;
        for request in self.requests {
            reactions.push(
                react(current, *request, &policy).map_err(RelationshipStagingError::Reaction)?,
            );
        }
        let mut state = staged.state().clone();
        for outcome in &reactions {
            if let ReactionOutcome::Proposed(proposal) = outcome {
                let record = state
                    .relationships
                    .iter_mut()
                    .find(|relationship| {
                        relationship.subject == proposal.original.subject
                            && relationship.object == proposal.original.object
                    })
                    .ok_or(RelationshipStagingError::RelationshipConflict)?;
                if *record != proposal.original {
                    return Err(RelationshipStagingError::RelationshipConflict);
                }
                *record = proposal.proposed.clone();
            }
        }
        let checkpoint = Checkpoint::new(
            staged.schema(),
            staged.basis(),
            staged.pins().clone(),
            state,
            self.inventory(),
            self.limits.checkpoint,
        )
        .map_err(RelationshipStagingError::InvalidCandidate)?;
        Ok(RelationshipTransition {
            checkpoint,
            reactions,
        })
    }

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
    ) -> Result<(), RelationshipStagingError<Handler::Rejection, Owner::Refusal>> {
        let limits = self.limits;
        let relationships = current.state().relationships.len();
        if limits.maximum_reactions == 0
            || limits.maximum_relationships == 0
            || limits.maximum_inventory_records == 0
            || limits.checkpoint.maximum_retained_bytes == 0
            || limits.checkpoint.maximum_retained_bytes > limits.maximum_checkpoint_bytes
            || self.requests.len() > limits.maximum_reactions
            || relationships > limits.maximum_relationships
            || self.entries.len() > limits.reaction.maximum_entries
        {
            return Err(RelationshipStagingError::Capacity);
        }
        let records = [
            self.inventory.rules.len(),
            self.inventory.content.len(),
            self.inventory.resources.len(),
            self.inventory.assets.len(),
        ]
        .into_iter()
        .try_fold(0usize, |count, next| count.checked_add(next))
        .ok_or(RelationshipStagingError::Capacity)?;
        // Reserve the complete policy scan and every react call, plus directional uniqueness.
        let work = self
            .requests
            .len()
            .checked_add(1)
            .and_then(|count| count.checked_mul(limits.reaction.maximum_work))
            .and_then(|work| work.checked_add(relationships.checked_mul(relationships)?))
            .and_then(|work| {
                work.checked_add(self.requests.len().checked_mul(self.requests.len())?)
            })
            .and_then(|work| work.checked_add(current.state().members.len()))
            .and_then(|work| work.checked_add(current.state().entities.len()))
            .and_then(|work| work.checked_add(current.state().characters.len()))
            .and_then(|work| work.checked_add(self.requests.len()))
            .ok_or(RelationshipStagingError::Capacity)?;
        let bytes = current
            .retained_bytes()
            .ok_or(RelationshipStagingError::Capacity)?;
        let pass = limits
            .maximum_checkpoint_bytes
            .checked_mul(3)
            .and_then(|scratch| scratch.checked_add(bytes))
            .and_then(|total| total.checked_add(input.retained_bytes()?))
            .and_then(|total| total.checked_add(limits.reaction.maximum_policy_bytes))
            .and_then(|total| {
                total.checked_add(
                    self.requests
                        .len()
                        .checked_mul(limits.reaction.maximum_proposal_bytes)?,
                )
            })
            .and_then(|total| {
                total.checked_add(
                    self.requests
                        .len()
                        .checked_mul(std::mem::size_of::<ReactionRequest>())?,
                )
            })
            .ok_or(RelationshipStagingError::Capacity)?;
        if records > limits.maximum_inventory_records
            || work > limits.maximum_work
            || bytes > limits.maximum_checkpoint_bytes
            || pass > limits.maximum_pass_bytes
        {
            return Err(RelationshipStagingError::Capacity);
        }
        Ok(())
    }
}

impl<Handler, Owner> RulesCommandHandler for RelationshipReactionHandler<'_, Handler, Owner>
where
    Handler: RulesCommandHandler,
    Owner: RelationshipApplicationOwner,
{
    type Rejection = RelationshipStagingError<Handler::Rejection, Owner::Refusal>;

    fn pins(&self) -> &CheckpointPins {
        self.admitted_pins
    }

    fn bound_source(&self) -> Option<&df_model::checkpoint::RuleReference> {
        self.command_handler.bound_source()
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        Ok(self.stage_with_outcomes(input, current)?.checkpoint)
    }
}
/// Native current-command admission, separate from accepted consequence admission.
pub trait RelationshipAxisApplicationOwner: RelationshipAxisOwner {
    fn admit_axis_application(
        &self,
        current: &Checkpoint,
        command: &CommandInput,
        changes: &[RelationshipAxisChange],
    ) -> Result<(), Self::Refusal>;
}

/// Refusals publish neither mechanics nor any earlier axis in the detached pass.
#[derive(Debug, Eq, PartialEq)]
pub enum RelationshipAxisCommandError<CommandRefusal, SourceRefusal> {
    Snapshot(CheckpointError),
    UnsupportedInput,
    StaleCommand,
    PendingResolution,
    WrongActor,
    Source(SourceRefusal),
    Command(InvocationError<CommandRefusal>),
    RelationshipConflict,
    UnacceptedConsequence,
    Axis(RelationshipAxisStagingError<SourceRefusal>),
}

/// Register this borrowed wrapper with the source handler's exact selector/clause.
/// An accepted next-revision fact from this operation is required for every axis.
/// Categorical reaction state is retained; the inner handler cannot rewrite axes,
/// directional identity or policy. No source qualification, commit or publication
/// authority follows from constructing this value.
pub struct RelationshipAxisHandler<'a, Handler, Owner> {
    pub command_handler: &'a Handler,
    pub owner: &'a Owner,
    pub current_basis: Basis,
    pub admitted_pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub changes: &'a [RelationshipAxisChange],
    pub limits: RelationshipAxisStagingLimits,
}

impl<Handler, Owner> RulesCommandHandler for RelationshipAxisHandler<'_, Handler, Owner>
where
    Handler: RulesCommandHandler,
    Owner: RelationshipAxisApplicationOwner,
{
    type Rejection = RelationshipAxisCommandError<Handler::Rejection, Owner::Refusal>;

    fn pins(&self) -> &CheckpointPins {
        self.admitted_pins
    }

    fn bound_source(&self) -> Option<&df_model::checkpoint::RuleReference> {
        self.command_handler.bound_source()
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        current: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        current
            .validate_resume(self.current_basis, self.admitted_pins)
            .map_err(RelationshipAxisCommandError::Snapshot)?;
        check_axis_capacity(current, &self.inventory, self.changes, self.limits)
            .map_err(RelationshipAxisCommandError::Axis)?;
        current
            .validate_admitted(
                self.current_basis,
                self.admitted_pins,
                self.inventory,
                self.limits.checkpoint,
            )
            .map_err(RelationshipAxisCommandError::Snapshot)?;
        let GameInput::Game(command) = input.command else {
            return Err(RelationshipAxisCommandError::UnsupportedInput);
        };
        if command.basis != self.current_basis
            || command.observed_revision != self.current_basis.revision
        {
            return Err(RelationshipAxisCommandError::StaleCommand);
        }
        if !current.state().pending.is_empty() {
            return Err(RelationshipAxisCommandError::PendingResolution);
        }
        let GameCommand::ProposeAction { actor, .. } = &command.command else {
            return Err(RelationshipAxisCommandError::UnsupportedInput);
        };
        if !current
            .state()
            .members
            .iter()
            .any(|link| link.member == command.member && link.character == Some(*actor))
            || !current
                .state()
                .characters
                .iter()
                .any(|character| character.entity == *actor && character.owner == command.member)
        {
            return Err(RelationshipAxisCommandError::WrongActor);
        }
        self.owner
            .admit_axis_application(current, command, self.changes)
            .map_err(RelationshipAxisCommandError::Source)?;
        let staged = stage_handler(
            self.command_handler,
            self.admitted_pins,
            input,
            current,
            self.limits.maximum_checkpoint_bytes,
        )
        .map_err(RelationshipAxisCommandError::Command)?;
        if !staged.state().pending.is_empty() {
            return Err(RelationshipAxisCommandError::PendingResolution);
        }
        if staged.state().relationships.len() != current.state().relationships.len()
            || !staged.state().relationships.iter().all(|next| {
                current.state().relationships.iter().any(|prior| {
                    next.subject == prior.subject
                        && next.object == prior.object
                        && next.policy == prior.policy
                        && next.trust == prior.trust
                        && next.affection == prior.affection
                        && next.respect == prior.respect
                        && next.fear == prior.fear
                        && next.suspicion == prior.suspicion
                        && next.debt == prior.debt
                        && next.familiarity == prior.familiarity
                })
            })
        {
            return Err(RelationshipAxisCommandError::RelationshipConflict);
        }
        for change in self.changes {
            let RelationshipAxisProvenance::AcceptedFact {
                fact,
                source_policy,
                ..
            } = &change.replacement.provenance
            else {
                return Err(RelationshipAxisCommandError::UnacceptedConsequence);
            };
            let accepted = staged.state().facts.iter().any(|record| {
                record.id == *fact
                    && record.operation == command.operation
                    && record.revision == staged.basis().revision
                    && staged.state().decisions.iter().any(|decision| {
                        decision.operation == command.operation
                            && decision.revision == record.revision
                            && decision.source_policy == *source_policy
                            && usize::try_from(record.ordinal)
                                .ok()
                                .and_then(|ordinal| decision.facts.get(ordinal))
                                == Some(fact)
                    })
            });
            if !accepted {
                return Err(RelationshipAxisCommandError::UnacceptedConsequence);
            }
        }
        apply_relationship_axis_changes(
            self.owner,
            staged.basis(),
            &staged,
            self.admitted_pins,
            self.inventory,
            self.changes,
            self.limits,
        )
        .map_err(RelationshipAxisCommandError::Axis)
    }
}
