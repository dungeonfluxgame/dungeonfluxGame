//! Pure, source-bound initiative for the existing three-actor local journey subset.
use df_encounter::participants::ValidatedParticipants;
use df_model::checkpoint::{
    Basis, Checkpoint, CheckpointError, EntityId, GameInput, GameState, ResolutionId,
    RuleReference, WindowId,
};
use df_model::commands::{CommandError, validate_client_command};
use df_rules::local_journey::{JourneyRuleError, initiative_order};

use crate::command_entry::{CommandEntryContext, RulesCommandInput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncounterStartError {
    Checkpoint(CheckpointError),
    Command(CommandError),
    InvalidParticipants,
    InvalidSource,
    InvalidDraws,
    InvalidDexterity,
    Rules(JourneyRuleError),
}

/// Order the exact admitted local party using three already-observed d20 draws.
///
/// The caller admits the working state and participants through their canonical
/// owner and supplies a trusted source inventory. This neither authenticates a
/// sender nor commits an encounter. It validates the current basis, pins, source,
/// draw identities and source-backed Dexterity resources without mutation or I/O.
/// Equal totals retain the caller's authored participant order; this is the local
/// journey decision, not a general 2024 tie-policy implementation.
pub fn local_journey_turn_order(
    input: RulesCommandInput<'_>,
    current: &Checkpoint,
    context: CommandEntryContext<'_>,
    admitted: &ValidatedParticipants<'_, EntityId, Basis>,
    staged: &GameState,
    source: &RuleReference,
) -> Result<[EntityId; 3], EncounterStartError> {
    current
        .validate_resume(context.current_basis, context.admitted_pins)
        .map_err(EncounterStartError::Checkpoint)?;
    validate_client_command(
        input.command,
        current,
        context.inventory,
        context.limits.command,
    )
    .map_err(EncounterStartError::Command)?;
    let GameInput::Game(command) = input.command else {
        return Err(EncounterStartError::InvalidParticipants);
    };
    if *admitted.current_basis() != context.current_basis || command.basis != context.current_basis
    {
        return Err(EncounterStartError::InvalidParticipants);
    }
    let actors: &[EntityId; 3] = admitted
        .actors()
        .try_into()
        .map_err(|_| EncounterStartError::InvalidParticipants)?;
    if source.catalog != context.admitted_pins.rules.catalog
        || !context.inventory.rules.contains(source)
    {
        return Err(EncounterStartError::InvalidSource);
    }
    let draws: &[_; 3] = input
        .supplied_draws
        .try_into()
        .map_err(|_| EncounterStartError::InvalidDraws)?;
    let resolution = ResolutionId::from_bytes(command.operation.as_bytes())
        .map_err(|_| EncounterStartError::InvalidDraws)?;
    let window = WindowId::from_bytes(command.operation.as_bytes())
        .map_err(|_| EncounterStartError::InvalidDraws)?;
    // The admitted inventory bounds the local resource scan, including new Bandit resources.
    if staged.resources.len() > context.inventory.resources.len() {
        return Err(EncounterStartError::InvalidDexterity);
    }
    let mut supplied = [(0, 0); 3];
    for (ordinal, ((values, actor), draw)) in supplied.iter_mut().zip(actors).zip(draws).enumerate()
    {
        if draw.operation != command.operation
            || draw.ordinal != ordinal as u32
            || draw.resolution != resolution
            || draw.window != window
            || draw.source != *source
            || draw.sides != 20
            || !(1..=20).contains(&draw.value)
        {
            return Err(EncounterStartError::InvalidDraws);
        }
        let mut resources = staged.resources.iter().filter(|resource| {
            resource.owner == *actor && resource.resource.as_str() == "dexterity"
        });
        let resource = resources
            .next()
            .ok_or(EncounterStartError::InvalidDexterity)?;
        if resources.next().is_some() {
            return Err(EncounterStartError::InvalidDexterity);
        }
        let mut constraints = context.inventory.resources.iter().filter(|constraint| {
            constraint.owner == *actor && constraint.resource == resource.resource
        });
        let constraint = constraints
            .next()
            .ok_or(EncounterStartError::InvalidDexterity)?;
        if constraints.next().is_some()
            || constraint.source != *source
            || resource.source != constraint.source
            || constraint.minimum != 1
            || constraint.maximum != 20
            || resource.minimum != constraint.minimum
            || resource.maximum != constraint.maximum
            || !(constraint.minimum..=constraint.maximum).contains(&resource.value)
        {
            return Err(EncounterStartError::InvalidDexterity);
        }
        let dexterity =
            u8::try_from(resource.value).map_err(|_| EncounterStartError::InvalidDexterity)?;
        *values = (draw.value, dexterity);
    }
    let order = initiative_order(supplied).map_err(EncounterStartError::Rules)?;
    // Rules returns a permutation of this fixed three-element array.
    Ok(order.map(|index| actors[index]))
}
