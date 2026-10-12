//! Canonical current-target admission for the actual registered native journey.

use super::*;
use df_intent::targets::{TargetResolutionError, TargetResolutionLimits, resolve_target};
use df_model::affordance::{Affordance, AffordanceSet, TargetResolution, TargetSelection};
use df_world::affordance::{
    AffordanceLookupError, AffordanceLookupLimits, AffordanceQuery, lookup_affordances,
};

pub(in crate::gameplay) struct CurrentOffer<'a> {
    pub kind: rpc::GameplayActionKind,
    pub label: String,
    pub set: AffordanceSet<'a>,
}

#[derive(Debug, Eq, PartialEq)]
pub(in crate::gameplay) enum NativeTargetError {
    State(RepositoryError),
    Lookup(AffordanceLookupError),
    Resolution(TargetResolutionError),
    UnsupportedAction,
    UnsupportedTarget,
    NeedsClarification(Vec<EntityId>),
    NeedsRuling,
}

fn inventory<'a>(
    rules: &'a [RuleReference],
    contents: &'a [ContentReference],
) -> ReferenceInventory<'a> {
    ReferenceInventory {
        rules,
        content: contents,
        resources: &[],
        assets: &[],
    }
}

fn resolution_limits() -> TargetResolutionLimits {
    TargetResolutionLimits {
        input_records: 4096,
        candidates: 32,
        output_bytes: 8192,
    }
}

/// Every binding here comes from an existing authored action, current eligible source path,
/// and canonical identity. It introduces no entity, mechanics, geometry or action permission.
pub(in crate::gameplay) fn current(
    checkpoint: &Checkpoint,
    member: MemberId,
) -> Result<Vec<CurrentOffer<'_>>, NativeTargetError> {
    if super::tactical::present(checkpoint.state())
        && super::phase(checkpoint).map_err(NativeTargetError::State)? == rpc::JourneyPhase::Combat
    {
        return Err(NativeTargetError::UnsupportedAction);
    }
    let actor = player_entity(member, checkpoint).map_err(NativeTargetError::State)?;
    let rules = [
        model::rule().map_err(NativeTargetError::State)?,
        rule().map_err(NativeTargetError::State)?,
    ];
    let contents = model::contents().map_err(NativeTargetError::State)?;
    let mut result = Vec::new();
    for (kind, label) in authored_offered(checkpoint, member).map_err(NativeTargetError::State)? {
        let entry = wire::action_entry(kind).ok_or(NativeTargetError::UnsupportedAction)?;
        let action = model::content(entry).map_err(NativeTargetError::State)?;
        let target = match kind {
            rpc::GameplayActionKind::AskCourier
            | rpc::GameplayActionKind::EscortCourier
            | rpc::GameplayActionKind::DefendCourier
            | rpc::GameplayActionKind::ChooseHarborScene => {
                Some(courier_reaction::courier().map_err(NativeTargetError::State)?)
            }
            rpc::GameplayActionKind::GreatswordAttack => {
                Some(entity(BANDIT).map_err(NativeTargetError::State)?)
            }
            rpc::GameplayActionKind::SecondWind | rpc::GameplayActionKind::ShortRest => Some(actor),
            rpc::GameplayActionKind::ExamineHarborSeal
            | rpc::GameplayActionKind::CreateCharacter
            | rpc::GameplayActionKind::BeginStory
            | rpc::GameplayActionKind::EndTurn => None,
            _ => return Err(NativeTargetError::UnsupportedAction),
        };
        let target = target
            .map(|target| {
                checkpoint
                    .state()
                    .entities
                    .iter()
                    .find(|entity| entity.id == target)
                    .cloned()
                    .ok_or(NativeTargetError::UnsupportedTarget)
            })
            .transpose()?;
        let records = [Affordance {
            actor,
            action: action.clone(),
            source: rule().map_err(NativeTargetError::State)?,
            target,
        }];
        let set = lookup_affordances(
            checkpoint,
            AffordanceQuery {
                basis: checkpoint.basis(),
                pins: checkpoint.pins(),
                actor,
                action: &action,
            },
            &records,
            inventory(&rules, &contents),
            AffordanceLookupLimits {
                input_records: 4096,
                candidates: 32,
                output_bytes: 8192,
            },
        )
        .map_err(NativeTargetError::Lookup)?;
        result.push(CurrentOffer { kind, label, set });
    }
    Ok(result)
}

pub(in crate::gameplay) fn offered(
    checkpoint: &Checkpoint,
    member: MemberId,
) -> Result<Vec<(rpc::GameplayActionKind, String)>, NativeTargetError> {
    let rules = [
        model::rule().map_err(NativeTargetError::State)?,
        rule().map_err(NativeTargetError::State)?,
    ];
    let contents = model::contents().map_err(NativeTargetError::State)?;
    let mut result = Vec::new();
    for offer in current(checkpoint, member)? {
        match resolve_target(
            checkpoint,
            &offer.set,
            TargetSelection::Unspecified,
            inventory(&rules, &contents),
            resolution_limits(),
        )
        .map_err(NativeTargetError::Resolution)?
        {
            TargetResolution::Selected(_) => result.push((offer.kind, offer.label)),
            TargetResolution::NeedsClarification(targets) => {
                return Err(NativeTargetError::NeedsClarification(targets));
            }
            TargetResolution::UnsupportedTarget => {
                return Err(NativeTargetError::UnsupportedTarget);
            }
            TargetResolution::NeedsRuling => return Err(NativeTargetError::NeedsRuling),
        }
    }
    Ok(result)
}

/// Revalidation runs before any staging/draw and feeds the unchanged registered mechanics.
/// Multiple supplied IDs cannot masquerade as confirmation of one supported native target.
pub(in crate::gameplay) fn validate(
    checkpoint: &Checkpoint,
    member: MemberId,
    action: &ContentReference,
    targets: &[EntityId],
) -> Result<(), NativeTargetError> {
    if super::tactical::present(checkpoint.state())
        && (super::phase(checkpoint).map_err(NativeTargetError::State)?
            == rpc::JourneyPhase::Combat
            || super::tactical::dying(checkpoint.state()).map_err(NativeTargetError::State)?)
    {
        if super::phase(checkpoint).map_err(NativeTargetError::State)?
            == rpc::JourneyPhase::Complete
            && targets.is_empty()
            && action.entry.as_str() == "end-turn"
        {
            return Ok(());
        }
        return super::tactical::validate_input(checkpoint, member, action, targets)
            .map_err(NativeTargetError::State);
    }
    let selection = match targets {
        [] => TargetSelection::Unspecified,
        [target] => TargetSelection::Explicit(*target),
        _ => return Err(NativeTargetError::UnsupportedTarget),
    };
    let rules = [
        model::rule().map_err(NativeTargetError::State)?,
        rule().map_err(NativeTargetError::State)?,
    ];
    let contents = model::contents().map_err(NativeTargetError::State)?;
    let offers = current(checkpoint, member)?;
    let offer = offers
        .iter()
        .find(|offer| offer.set.action == *action)
        .ok_or(NativeTargetError::UnsupportedAction)?;
    match resolve_target(
        checkpoint,
        &offer.set,
        selection,
        inventory(&rules, &contents),
        resolution_limits(),
    )
    .map_err(NativeTargetError::Resolution)?
    {
        TargetResolution::Selected(_) => Ok(()),
        TargetResolution::NeedsClarification(targets) => {
            Err(NativeTargetError::NeedsClarification(targets))
        }
        TargetResolution::UnsupportedTarget => Err(NativeTargetError::UnsupportedTarget),
        TargetResolution::NeedsRuling => Err(NativeTargetError::NeedsRuling),
    }
}
