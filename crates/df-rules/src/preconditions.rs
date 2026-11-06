//! Source-handler-declared structural dependencies over canonical model state.
use crate::command_handler::{RulesCommandHandler, RulesCommandInput, is_same_epoch_observation};
use df_model::checkpoint::{
    ActiveEffect, Basis, Checkpoint, CheckpointError, CheckpointPins, ContentReference, EntityId,
    GameInput, PendingInput, PendingResolution, RecordId, ReferenceInventory, ResolutionId,
    RuleReference, TimerId,
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
#[derive(Clone, Copy)]
pub struct CurrentRuleContext<'a> {
    pub checkpoint: &'a Checkpoint,
    pub basis: Basis,
    pub pins: &'a CheckpointPins,
    pub inventory: ReferenceInventory<'a>,
    pub command_limits: CommandLimits,
}

/// Which declared dependency, bound or reference failed revalidation.
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
    validate_prepared_snapshot(preconditions.prepared, current_basis, pins)?;
    validate_declared_sources(preconditions.sources, pins, &inventory)?;
    let references = DeclaredReferences {
        sources: preconditions.sources,
        pins,
        inventory,
    };
    let snapshots = Snapshots {
        prepared: preconditions.prepared,
        current,
    };
    for (index, dependency) in preconditions.dependencies.iter().enumerate() {
        if preconditions.dependencies[..index].contains(dependency) {
            return Err(PreconditionError::DuplicateDependency);
        }
        validate_dependency(dependency, snapshots, &references)?;
    }
    validate_client_command(input, current, inventory, command_limits)
        .map_err(PreconditionError::Command)
}

/// The prepared snapshot is a valid same-epoch observation of the current one under the same
/// rules, content and build pins.
fn validate_prepared_snapshot(
    prepared: &Checkpoint,
    current_basis: Basis,
    pins: &CheckpointPins,
) -> Result<(), PreconditionError> {
    let basis = prepared.basis();
    prepared
        .validate_resume(basis, prepared.pins())
        .map_err(PreconditionError::PreparedCheckpoint)?;
    if !is_same_epoch_observation(basis, current_basis) {
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
    Ok(())
}

/// Every declared source is admitted under the pinned catalog and declared once.
fn validate_declared_sources(
    sources: &[RuleReference],
    pins: &CheckpointPins,
    inventory: &ReferenceInventory<'_>,
) -> Result<(), PreconditionError> {
    for (index, source) in sources.iter().enumerate() {
        if source.catalog != pins.rules.catalog || !inventory.rules.contains(source) {
            return Err(PreconditionError::InvalidReference);
        }
        if sources[..index].contains(source) {
            return Err(PreconditionError::DuplicateDependency);
        }
    }
    Ok(())
}

/// The prepared observation and the current serialized-owner snapshot it is checked against.
#[derive(Clone, Copy)]
struct Snapshots<'a> {
    prepared: &'a Checkpoint,
    current: &'a Checkpoint,
}

/// Whether the record `matches` selects is the same, or equally absent, in both collections;
/// also returns the prepared record so its references can be checked.
fn same_record<'a, T: PartialEq>(
    prepared: &'a [T],
    current: &[T],
    matches: impl Fn(&T) -> bool,
) -> (bool, Option<&'a T>) {
    let expected = prepared.iter().find(|record| matches(record));
    let actual = current.iter().find(|record| matches(record));
    (expected == actual, expected)
}

/// The references a dependency's records may name: the declared sources and admitted content.
struct DeclaredReferences<'a> {
    sources: &'a [RuleReference],
    pins: &'a CheckpointPins,
    inventory: ReferenceInventory<'a>,
}

impl DeclaredReferences<'_> {
    fn require_source(&self, source: &RuleReference) -> Result<(), PreconditionError> {
        if self.sources.contains(source) {
            Ok(())
        } else {
            Err(PreconditionError::InvalidReference)
        }
    }

    fn require_content(&self, reference: &ContentReference) -> Result<(), PreconditionError> {
        if reference.package == self.pins.content.package
            && self.inventory.content.contains(reference)
        {
            Ok(())
        } else {
            Err(PreconditionError::InvalidReference)
        }
    }

    fn require_effect_sources(&self, effect: &ActiveEffect) -> Result<(), PreconditionError> {
        self.require_source(&effect.source)?;
        for choice in &effect.choices {
            self.require_source(&choice.source)?;
        }
        Ok(())
    }
}

/// One declared dependency is unchanged between the snapshots and names only declared sources
/// and admitted content.
fn validate_dependency(
    dependency: &RuleDependency,
    snapshots: Snapshots<'_>,
    references: &DeclaredReferences<'_>,
) -> Result<(), PreconditionError> {
    let prepared = snapshots.prepared.state();
    let current = snapshots.current.state();
    match dependency {
        RuleDependency::Entity(id) => {
            let (unchanged, entity) =
                same_record(&prepared.entities, &current.entities, |record| {
                    record.id == *id
                });
            if !unchanged {
                return Err(PreconditionError::StaleEntity);
            }
            if let Some(entity) = entity {
                references.require_content(&entity.definition)?;
            }
        }
        RuleDependency::Encounter(id) => {
            let (unchanged, encounter) =
                same_record(&prepared.encounters, &current.encounters, |record| {
                    record.id == *id
                });
            if !unchanged {
                return Err(PreconditionError::StaleEncounter);
            }
            if let Some(encounter) = encounter {
                references.require_content(&encounter.definition)?;
                references.require_content(&encounter.combat_policy)?;
                for objective in &encounter.objectives {
                    references.require_content(objective)?;
                }
            }
        }
        RuleDependency::Environment(location) => {
            let prepared_rows = prepared
                .continuity
                .environment
                .iter()
                .filter(|record| record.location == *location);
            let current_rows = current
                .continuity
                .environment
                .iter()
                .filter(|record| record.location == *location);
            if !prepared_rows.clone().eq(current_rows) {
                return Err(PreconditionError::StaleEnvironment);
            }
            for environment in prepared_rows {
                references.require_content(&environment.definition)?;
            }
        }
        RuleDependency::Resource { owner, resource } => {
            let expected = prepared
                .resources
                .iter()
                .find(|record| record.owner == *owner && record.resource == *resource)
                .ok_or(PreconditionError::InvalidReference)?;
            references.require_source(&expected.source)?;
            if !references.inventory.resources.iter().any(|constraint| {
                constraint.owner == expected.owner
                    && constraint.resource == expected.resource
                    && constraint.source == expected.source
                    && constraint.minimum == expected.minimum
                    && constraint.maximum == expected.maximum
            }) {
                return Err(PreconditionError::InvalidReference);
            }
            if !current.resources.iter().any(|record| record == expected) {
                return Err(PreconditionError::StaleResource);
            }
        }
        RuleDependency::LogicalTime => {
            if prepared.logical_time != current.logical_time {
                return Err(PreconditionError::StaleTime);
            }
        }
        RuleDependency::ActiveEffect(id) => {
            let (unchanged, effect) = same_record(
                &prepared.active_effects,
                &current.active_effects,
                |record| record.id == *id,
            );
            if !unchanged {
                return Err(PreconditionError::StaleActiveEffects);
            }
            if let Some(effect) = effect {
                references.require_effect_sources(effect)?;
            }
        }
        RuleDependency::PendingResolution(id) => {
            let expected = prepared.pending.iter().find(|record| record.id == *id);
            let actual = current.pending.iter().find(|record| record.id == *id);
            if !same_pending(expected, actual) {
                return Err(PreconditionError::StalePending);
            }
            if let Some(pending) = expected {
                pending_sources(pending, references)?;
            }
        }
        RuleDependency::Timer(id) => {
            let (unchanged, timer) =
                same_record(&prepared.timers, &current.timers, |record| record.id == *id);
            if !unchanged {
                return Err(PreconditionError::StaleTimers);
            }
            if let Some(timer) = timer {
                references.require_source(&timer.source)?;
            }
        }
        RuleDependency::ActiveEffects => {
            if prepared.active_effects != current.active_effects {
                return Err(PreconditionError::StaleActiveEffects);
            }
            for effect in &prepared.active_effects {
                references.require_effect_sources(effect)?;
            }
        }
        RuleDependency::PendingResolutions => {
            if prepared.pending.len() != current.pending.len()
                || !prepared
                    .pending
                    .iter()
                    .zip(&current.pending)
                    .all(|(expected, actual)| same_pending(Some(expected), Some(actual)))
            {
                return Err(PreconditionError::StalePending);
            }
            for pending in &prepared.pending {
                pending_sources(pending, references)?;
            }
        }
        RuleDependency::Timers => {
            if prepared.timers != current.timers {
                return Err(PreconditionError::StaleTimers);
            }
            for timer in &prepared.timers {
                references.require_source(&timer.source)?;
            }
        }
    }
    Ok(())
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
    references: &DeclaredReferences<'_>,
) -> Result<(), PreconditionError> {
    let required = |source: &RuleReference| references.require_source(source);
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

/// A refusal from the dependency guard, its source binding or the wrapped handler.
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
    /// Binds a handler to its source clause, owner context, preparation and limits.
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
        validate_rule_preconditions(
            input.command,
            CurrentRuleContext {
                checkpoint,
                ..self.context
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
