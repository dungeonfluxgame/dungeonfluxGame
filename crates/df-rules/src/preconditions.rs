//! Source-handler-declared structural dependencies over canonical model state.
use crate::command_handler::{RulesCommandHandler, RulesCommandInput};
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, CheckpointPins, ContentReference, EntityId, GameInput,
    PendingInput, PendingResolution, RecordId, ReferenceInventory, ResolutionId, RuleReference,
    TimerId,
};
use df_model::commands::{CommandError, CommandLimits, validate_client_command};
use df_types::RevisionLabel;

/// A trusted compiled handler's dependency, never a client assertion of mechanical legality.
/// Keyed records compare presence as well as values. Whole-collection variants explicitly
/// require absence/addition detection across that collection; they are not global revision locks.
/// Source review must establish that the complete list covers this handler's prerequisites.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleDependency {
    Entity(EntityId),
    Encounter(RecordId),
    /// All canonical environmental rows at this location, including the empty set.
    Environment(EntityId),
    Resource {
        owner: EntityId,
        resource: RevisionLabel,
    },
    LogicalTime,
    ActiveEffect(RecordId),
    PendingResolution(ResolutionId),
    Timer(TimerId),
    ActiveEffects,
    PendingResolutions,
    Timers,
}

/// Retained preparation and its immutable caller-owned, source-reviewed dependency list.
/// This wrapper preserves existing canonical records; it defines no mechanical state schema.
pub struct RulePreconditions<'a> {
    pub prepared: &'a Checkpoint,
    pub sources: &'a [RuleReference],
    pub dependencies: &'a [RuleDependency],
}

/// Positive caller-selected work bounds, independent of tabletop timing or resource rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreconditionLimits {
    pub maximum_dependencies: usize,
    pub maximum_comparisons: usize,
    pub maximum_checkpoint_bytes: usize,
}

/// Current serialized-owner context, independent of every client-observed field.
pub struct CurrentRuleContext<'a> {
    pub checkpoint: &'a Checkpoint,
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub command_limits: CommandLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreconditionError {
    CurrentCheckpoint(CheckpointError),
    PreparedCheckpoint(CheckpointError),
    Command(CommandError),
    StalePreparedBasis,
    SourceMismatch,
    ContentMismatch,
    BuildMismatch,
    InvalidReference,
    DuplicateDependency,
    StaleEntity,
    StaleEncounter,
    StaleEnvironment,
    StaleResource,
    StaleTime,
    StaleActiveEffects,
    StalePending,
    StaleTimers,
    Capacity,
}

/// Validates declared dependencies immediately before the selected compiled handler runs.
/// Older same-epoch observations survive unselected changes. Source handlers still own legal
/// costs, geometry, turns, exceptions and outcomes; this guard never draws, spends or mutates.
/// The session authenticates/authorizes and resolves prior operation receipts before this path.
pub fn validate_rule_preconditions(
    input: &GameInput,
    context: CurrentRuleContext<'_>,
    preconditions: &RulePreconditions<'_>,
    limits: PreconditionLimits,
) -> Result<(), PreconditionError> {
    let CurrentRuleContext {
        checkpoint: current,
        basis: current_basis,
        pins,
        inventory,
        command_limits,
    } = context;
    current
        .validate_resume(current_basis, pins)
        .map_err(PreconditionError::CurrentCheckpoint)?;
    validate_bounds(current, preconditions, &inventory, limits)?;
    let prepared = preconditions.prepared;
    let basis = prepared.basis();
    prepared
        .validate_resume(basis, prepared.pins())
        .map_err(PreconditionError::PreparedCheckpoint)?;
    if basis.session != current_basis.session
        || basis.run != current_basis.run
        || basis.revision.epoch() != current_basis.revision.epoch()
        || basis.revision > current_basis.revision
    {
        return Err(PreconditionError::StalePreparedBasis);
    }
    if prepared.pins().rules != pins.rules {
        return Err(PreconditionError::SourceMismatch);
    }
    if prepared.pins().content != pins.content {
        return Err(PreconditionError::ContentMismatch);
    }
    if prepared.pins().build != pins.build {
        return Err(PreconditionError::BuildMismatch);
    }
    for (index, source) in preconditions.sources.iter().enumerate() {
        if source.catalog != pins.rules.catalog || !inventory.rules.contains(source) {
            return Err(PreconditionError::InvalidReference);
        }
        if preconditions
            .sources
            .iter()
            .take(index)
            .any(|earlier| earlier == source)
        {
            return Err(PreconditionError::DuplicateDependency);
        }
    }
    let required_source = |source: &RuleReference| {
        if preconditions.sources.contains(source) {
            Ok(())
        } else {
            Err(PreconditionError::InvalidReference)
        }
    };
    let required_content = |reference: &ContentReference| {
        if reference.package == pins.content.package && inventory.content.contains(reference) {
            Ok(())
        } else {
            Err(PreconditionError::InvalidReference)
        }
    };
    for (index, dependency) in preconditions.dependencies.iter().enumerate() {
        if preconditions
            .dependencies
            .iter()
            .take(index)
            .any(|earlier| earlier == dependency)
        {
            return Err(PreconditionError::DuplicateDependency);
        }
        match dependency {
            RuleDependency::Entity(id) => {
                let expected = prepared
                    .state()
                    .entities
                    .iter()
                    .find(|record| record.id == *id);
                let actual = current
                    .state()
                    .entities
                    .iter()
                    .find(|record| record.id == *id);
                if expected != actual {
                    return Err(PreconditionError::StaleEntity);
                }
                if let Some(entity) = expected {
                    required_content(&entity.definition)?;
                }
            }
            RuleDependency::Encounter(id) => {
                let expected = prepared
                    .state()
                    .encounters
                    .iter()
                    .find(|record| record.id == *id);
                let actual = current
                    .state()
                    .encounters
                    .iter()
                    .find(|record| record.id == *id);
                if expected != actual {
                    return Err(PreconditionError::StaleEncounter);
                }
                if let Some(encounter) = expected {
                    required_content(&encounter.definition)?;
                    required_content(&encounter.combat_policy)?;
                    for objective in &encounter.objectives {
                        required_content(objective)?;
                    }
                }
            }
            RuleDependency::Environment(location) => {
                let expected = prepared
                    .state()
                    .continuity
                    .environment
                    .iter()
                    .filter(|record| record.location == *location);
                let actual = current
                    .state()
                    .continuity
                    .environment
                    .iter()
                    .filter(|record| record.location == *location);
                if !expected.eq(actual) {
                    return Err(PreconditionError::StaleEnvironment);
                }
                for environment in prepared
                    .state()
                    .continuity
                    .environment
                    .iter()
                    .filter(|record| record.location == *location)
                {
                    required_content(&environment.definition)?;
                }
            }
            RuleDependency::Resource { owner, resource } => {
                let expected = prepared
                    .state()
                    .resources
                    .iter()
                    .find(|record| record.owner == *owner && record.resource == *resource)
                    .ok_or(PreconditionError::InvalidReference)?;
                required_source(&expected.source)?;
                if !inventory.resources.iter().any(|constraint| {
                    constraint.owner == expected.owner
                        && constraint.resource == expected.resource
                        && constraint.source == expected.source
                        && constraint.minimum == expected.minimum
                        && constraint.maximum == expected.maximum
                }) {
                    return Err(PreconditionError::InvalidReference);
                }
                if !current
                    .state()
                    .resources
                    .iter()
                    .any(|record| record == expected)
                {
                    return Err(PreconditionError::StaleResource);
                }
            }
            RuleDependency::LogicalTime => {
                if prepared.state().logical_time != current.state().logical_time {
                    return Err(PreconditionError::StaleTime);
                }
            }
            RuleDependency::ActiveEffect(id) => {
                let expected = prepared
                    .state()
                    .active_effects
                    .iter()
                    .find(|record| record.id == *id);
                let actual = current
                    .state()
                    .active_effects
                    .iter()
                    .find(|record| record.id == *id);
                if expected != actual {
                    return Err(PreconditionError::StaleActiveEffects);
                }
                if let Some(effect) = expected {
                    required_source(&effect.source)?;
                    for choice in &effect.choices {
                        required_source(&choice.source)?;
                    }
                }
            }
            RuleDependency::PendingResolution(id) => {
                let expected = prepared
                    .state()
                    .pending
                    .iter()
                    .find(|record| record.id == *id);
                let actual = current
                    .state()
                    .pending
                    .iter()
                    .find(|record| record.id == *id);
                if !same_pending(expected, actual) {
                    return Err(PreconditionError::StalePending);
                }
                if let Some(pending) = expected {
                    pending_sources(pending, &required_source)?;
                }
            }
            RuleDependency::Timer(id) => {
                let expected = prepared
                    .state()
                    .timers
                    .iter()
                    .find(|record| record.id == *id);
                let actual = current
                    .state()
                    .timers
                    .iter()
                    .find(|record| record.id == *id);
                if expected != actual {
                    return Err(PreconditionError::StaleTimers);
                }
                if let Some(timer) = expected {
                    required_source(&timer.source)?;
                }
            }
            RuleDependency::ActiveEffects => {
                if prepared.state().active_effects != current.state().active_effects {
                    return Err(PreconditionError::StaleActiveEffects);
                }
                for effect in &prepared.state().active_effects {
                    required_source(&effect.source)?;
                    for choice in &effect.choices {
                        required_source(&choice.source)?;
                    }
                }
            }
            RuleDependency::PendingResolutions => {
                if prepared.state().pending.len() != current.state().pending.len()
                    || !prepared
                        .state()
                        .pending
                        .iter()
                        .zip(&current.state().pending)
                        .all(|(expected, actual)| same_pending(Some(expected), Some(actual)))
                {
                    return Err(PreconditionError::StalePending);
                }
                for pending in &prepared.state().pending {
                    pending_sources(pending, &required_source)?;
                }
            }
            RuleDependency::Timers => {
                if prepared.state().timers != current.state().timers {
                    return Err(PreconditionError::StaleTimers);
                }
                for timer in &prepared.state().timers {
                    required_source(&timer.source)?;
                }
            }
        }
    }
    validate_client_command(input, current, inventory, command_limits)
        .map_err(PreconditionError::Command)
}

fn same_pending(expected: Option<&PendingResolution>, actual: Option<&PendingResolution>) -> bool {
    match (expected, actual) {
        (None, None) => true,
        (Some(expected), Some(actual)) => {
            // Constructor binds pending to each snapshot's revision; that bookkeeping sequence
            // advances on unrelated changes without altering a surviving source-defined window.
            expected.id == actual.id
                && expected.continuation == actual.continuation
                && expected.window == actual.window
                && expected.next == actual.next
                && expected.choices == actual.choices
                && expected.draw_ordinals == actual.draw_ordinals
                && expected.spent == actual.spent
                && expected.rulings == actual.rulings
        }
        _ => false,
    }
}

fn pending_sources(
    pending: &PendingResolution,
    required: &impl Fn(&RuleReference) -> Result<(), PreconditionError>,
) -> Result<(), PreconditionError> {
    required(&pending.window.source)?;
    for choice in &pending.choices {
        required(&choice.source)?;
    }
    for spent in &pending.spent {
        required(&spent.source)?;
    }
    for ruling in &pending.rulings {
        required(&ruling.source)?;
    }
    match &pending.next {
        PendingInput::Choice { remaining }
        | PendingInput::Reaction { remaining }
        | PendingInput::Ruling {
            permitted: remaining,
            ..
        } => {
            for offered in remaining {
                required(&offered.source)?;
            }
            if let PendingInput::Ruling { source, .. } = &pending.next {
                required(source)?;
            }
        }
        PendingInput::Roll { source, .. } => required(source)?,
    }
    Ok(())
}

fn validate_bounds(
    current: &Checkpoint,
    preconditions: &RulePreconditions<'_>,
    inventory: &ReferenceInventory<'_>,
    limits: PreconditionLimits,
) -> Result<(), PreconditionError> {
    if limits.maximum_dependencies == 0
        || limits.maximum_comparisons == 0
        || limits.maximum_checkpoint_bytes == 0
    {
        return Err(PreconditionError::Capacity);
    }
    let prepared = preconditions.prepared;
    let mut checkpoint_bytes = 0usize;
    for checkpoint in [prepared, current] {
        let retained_checkpoint_bytes = checkpoint
            .retained_bytes()
            .ok_or(PreconditionError::Capacity)?;
        if retained_checkpoint_bytes > limits.maximum_checkpoint_bytes {
            return Err(PreconditionError::Capacity);
        }
        checkpoint_bytes = checkpoint_bytes
            .checked_add(retained_checkpoint_bytes)
            .ok_or(PreconditionError::Capacity)?;
    }
    let source_and_dependency_count = preconditions
        .sources
        .len()
        .checked_add(preconditions.dependencies.len())
        .ok_or(PreconditionError::Capacity)?;
    if source_and_dependency_count > limits.maximum_dependencies {
        return Err(PreconditionError::Capacity);
    }
    // Retained bytes conservatively bound nested snapshot records and source comparisons.
    let scan_units_per_pass = [
        source_and_dependency_count,
        inventory.rules.len(),
        inventory.content.len(),
        inventory.resources.len(),
        checkpoint_bytes,
    ]
    .into_iter()
    .try_fold(0usize, |total, amount| total.checked_add(amount))
    .ok_or(PreconditionError::Capacity)?;
    let comparison_count = source_and_dependency_count
        .checked_add(1)
        .and_then(|pass_count| pass_count.checked_mul(scan_units_per_pass))
        .and_then(|comparison_count| {
            inventory
                .content
                .len()
                .checked_add(1)
                .and_then(|content_count| comparison_count.checked_mul(content_count))
        })
        .ok_or(PreconditionError::Capacity)?;
    if comparison_count > limits.maximum_comparisons {
        return Err(PreconditionError::Capacity);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreconditionedRejection<R> {
    Precondition(PreconditionError),
    HandlerSourceMismatch,
    HandlerPinsMismatch,
    Handler(R),
}

/// A concrete registrable handler value binding a reviewed implementation to its preparation
/// and immutable dependency list. Registration/selection uses I01's canonical registry.
/// The supplied list is caller-owned source evidence, not automatically inferred mechanics.
/// This adapter invokes the inner handler only after successful structural revalidation.
/// It borrows the canonical command for validation and forwards the identical trusted supplied
/// draw envelope. It neither consumes draws nor constructs, substitutes or reorders outcomes.
pub struct PreconditionedCommandHandler<'a, H> {
    handler: &'a H,
    source: &'a RuleReference,
    context: CurrentRuleContext<'a>,
    preconditions: RulePreconditions<'a>,
    limits: PreconditionLimits,
}

impl<'a, H: RulesCommandHandler> PreconditionedCommandHandler<'a, H> {
    pub fn new(
        handler: &'a H,
        source: &'a RuleReference,
        context: CurrentRuleContext<'a>,
        dependencies: RulePreconditions<'a>,
        limits: PreconditionLimits,
    ) -> Self {
        Self {
            handler,
            source,
            context,
            preconditions: dependencies,
            limits,
        }
    }

    /// Exact clause that the owner must bind to its I01 registration and I03 response source.
    pub fn source(&self) -> &RuleReference {
        self.source
    }
}

impl<H: RulesCommandHandler> RulesCommandHandler for PreconditionedCommandHandler<'_, H> {
    type Rejection = PreconditionedRejection<H::Rejection>;

    fn pins(&self) -> &CheckpointPins {
        self.handler.pins()
    }

    fn bound_source(&self) -> Option<&RuleReference> {
        Some(self.source)
    }

    fn stage(
        &self,
        input: RulesCommandInput<'_>,
        checkpoint: &Checkpoint,
    ) -> Result<Checkpoint, Self::Rejection> {
        if self
            .handler
            .bound_source()
            .is_some_and(|source| source != self.source)
        {
            return Err(PreconditionedRejection::HandlerSourceMismatch);
        }
        if self.handler.pins() != self.context.pins {
            return Err(PreconditionedRejection::HandlerPinsMismatch);
        }
        if !self.preconditions.sources.contains(self.source) {
            return Err(PreconditionedRejection::Precondition(
                PreconditionError::InvalidReference,
            ));
        }
        let inventory = &self.context.inventory;
        validate_rule_preconditions(
            input.command,
            CurrentRuleContext {
                checkpoint,
                basis: self.context.basis,
                pins: self.context.pins,
                inventory: ReferenceInventory {
                    rules: inventory.rules,
                    content: inventory.content,
                    resources: inventory.resources,
                    assets: inventory.assets,
                },
                command_limits: self.context.command_limits,
            },
            &self.preconditions,
            self.limits,
        )
        .map_err(PreconditionedRejection::Precondition)?;
        self.handler
            .stage(input, checkpoint)
            .map_err(PreconditionedRejection::Handler)
    }
}

#[cfg(test)]
#[path = "preconditions_tests.rs"]
mod tests;
