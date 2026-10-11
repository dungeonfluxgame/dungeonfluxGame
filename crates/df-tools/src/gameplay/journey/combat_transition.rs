//! One source-selected player Greatsword candidate in the authored adjacent encounter.
use df_combat::candidates::{
    CandidateContext, CandidateError, CandidateLimits, LegalOfferOwner, enumerate_candidates,
};
use df_model::checkpoint::{
    ActualDraw, Checkpoint, CommandInput, ContentReference, EntityId, GameCommand, GameState,
    Position, RuleReference,
};
use df_protocol::common as rpc;
use df_session::submission::RepositoryError;

use super::{
    AttackOptions, BANDIT, DiceSource, ENTITIES, attack, choice, entity, finish_combat,
    held_weapon_id, model, phase, room_entity, rule, rules, set, value,
};

#[derive(Eq, PartialEq)]
struct GreatswordAttack {
    actor: EntityId,
    target: EntityId,
    weapon: EntityId,
    definition: ContentReference,
    source: RuleReference,
}

struct CurrentPlayerAttack<'a> {
    current: &'a Checkpoint,
    state: &'a GameState,
}

impl CurrentPlayerAttack<'_> {
    fn resource(&self, actor: EntityId, key: &str) -> Result<u32, RepositoryError> {
        let resource = self
            .state
            .resources
            .iter()
            .find(|resource| resource.owner == actor && resource.resource.as_str() == key)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if resource.source != rule()? {
            return Err(RepositoryError::InvalidCandidate);
        }
        u32::try_from(resource.value).map_err(|_| RepositoryError::InvalidCandidate)
    }
}

impl LegalOfferOwner for CurrentPlayerAttack<'_> {
    type Observation = EntityId;
    type Offer = GreatswordAttack;
    type Error = RepositoryError;

    fn validate_observation(
        &self,
        actor: &EntityId,
        context: CandidateContext<'_>,
    ) -> Result<(), RepositoryError> {
        if *context.basis != self.current.basis() {
            return Err(RepositoryError::RevisionConflict);
        }
        if context.pins != self.current.pins() || context.pins != &model::pins()? {
            return Err(RepositoryError::InvalidCandidate);
        }
        if phase(self.current)? != rpc::JourneyPhase::Combat {
            return Err(RepositoryError::InvalidCandidate);
        }
        let [encounter] = self.state.encounters.as_slice() else {
            return Err(RepositoryError::InvalidCandidate);
        };
        let build = model::content("dwarf-fighter-soldier")?;
        let character = self
            .state
            .characters
            .iter()
            .find(|character| character.entity == *actor && character.build == build)
            .ok_or(RepositoryError::InvalidCandidate)?;
        if !ENTITIES.contains(actor.as_bytes())
            || !self
                .state
                .members
                .iter()
                .any(|link| link.member == character.owner && link.character == Some(*actor))
            || encounter.definition != model::content("combat")?
            || encounter.combat_policy != model::content("normal-nonlethal-melee")?
            || encounter.objectives != [model::content("defend-courier")?]
            || !encounter.participants.contains(actor)
            || !encounter.participants.contains(&entity(BANDIT)?)
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        Ok(())
    }

    fn is_current_legal(
        &self,
        actor: &EntityId,
        _: CandidateContext<'_>,
        candidate: &GreatswordAttack,
    ) -> Result<bool, RepositoryError> {
        if candidate.actor != *actor
            || candidate.target != entity(BANDIT)?
            || candidate.weapon != held_weapon_id(*actor)?
            || candidate.definition != model::content(&super::pack::hero_weapon_item())?
            || candidate.source != rule()?
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let find = |id| self.state.entities.iter().find(|entity| entity.id == id);
        let actor_entity = find(*actor).ok_or(RepositoryError::InvalidCandidate)?;
        let target = find(candidate.target).ok_or(RepositoryError::InvalidCandidate)?;
        let Some(weapon) = find(candidate.weapon) else {
            return Ok(false);
        };
        let Some(item) = self
            .state
            .inventory
            .iter()
            .find(|item| item.item == candidate.weapon)
        else {
            return Ok(false);
        };
        let [encounter] = self.state.encounters.as_slice() else {
            return Err(RepositoryError::InvalidCandidate);
        };
        if weapon.definition != candidate.definition
            || weapon.identity_revision != model::label("source-starting-equipment-1")?
            || item.owner != *actor
            || item.quantity != super::pack::hero_weapon_quantity()
            || item.source != candidate.source
            || item.origin != candidate.definition
            || item.attunement_owner.is_some()
            || actor_entity.definition != model::content("joined-player")?
            || actor_entity.identity_revision != model::label("joined-player-1")?
            || target.definition != model::content("bandit")?
            || target.identity_revision != model::label("srd521-bandit-1")?
        {
            return Err(RepositoryError::InvalidCandidate);
        }
        let actor_position = if *actor == entity(ENTITIES[0])? {
            Position { x: 0, y: 5, z: 0 }
        } else {
            Position { x: 5, y: 0, z: 0 }
        };
        let economy = rules::ActionEconomy {
            action_used: self.resource(*actor, "action-used")? != 0,
            bonus_action_used: self.resource(*actor, "bonus-used")? != 0,
        };
        // These cells are the existing selected adjacent profile. Other positioning,
        // Prone interactions, reach, advantage and movement remain outside this slice.
        Ok(encounter.active_turn == Some(*actor)
            && actor_entity.location == Some(room_entity()?)
            && actor_entity.position == Some(actor_position)
            && target.location == Some(room_entity()?)
            && target.position == Some(Position { x: 0, y: 0, z: 0 })
            && weapon.location == Some(*actor)
            && weapon.position.is_none()
            && self.resource(*actor, "held-weapon")? == 1
            && self.resource(*actor, "unconscious")? == 0
            && self.resource(*actor, "prone")? == 0
            && self.resource(*actor, "hit-points")? != 0
            && self.resource(candidate.target, "unconscious")? == 0
            && self.resource(candidate.target, "prone")? == 0
            && self.resource(candidate.target, "hit-points")? != 0
            && economy.attack().is_ok())
    }

    fn same_offer(
        &self,
        left: &GreatswordAttack,
        right: &GreatswordAttack,
    ) -> Result<bool, RepositoryError> {
        Ok(left == right)
    }

    fn offer_bytes(&self, offer: &GreatswordAttack) -> Result<usize, RepositoryError> {
        [
            &offer.definition.package,
            &offer.definition.entry,
            &offer.source.catalog,
            &offer.source.source,
            &offer.source.entry,
            &offer.source.clause,
        ]
        .into_iter()
        .try_fold(size_of::<GreatswordAttack>(), |total, label| {
            total
                .checked_add(label.retained_heap_bytes())
                .ok_or(RepositoryError::Capacity)
        })
    }
}

fn admit(
    current: &Checkpoint,
    state: &GameState,
    actor: EntityId,
) -> Result<Option<GreatswordAttack>, RepositoryError> {
    if state.entities.len() > 512
        || state.resources.len() > 512
        || state.inventory.len() > 512
        || state.members.len() > 3
        || state.characters.len() > 2
    {
        return Err(RepositoryError::Capacity);
    }
    let candidate = GreatswordAttack {
        actor,
        target: entity(BANDIT)?,
        weapon: held_weapon_id(actor)?,
        definition: model::content(&super::pack::hero_weapon_item())?,
        source: rule()?,
    };
    let basis = current.basis();
    let context = CandidateContext {
        basis: &basis,
        pins: current.pins(),
    };
    let owner = CurrentPlayerAttack { current, state };
    let offered = [candidate];
    let admitted = enumerate_candidates(
        &owner,
        &actor,
        context,
        context,
        &offered,
        CandidateLimits {
            max_offers: 1,
            max_identity_comparisons: 0,
            max_candidates: 1,
            max_candidate_bytes: 4096,
        },
    )
    .map_err(|error| match error {
        CandidateError::Owner(error) => error,
        CandidateError::StaleBasis => RepositoryError::RevisionConflict,
        CandidateError::StalePins | CandidateError::Duplicate { .. } => {
            RepositoryError::InvalidCandidate
        }
        CandidateError::OfferCapacity { .. }
        | CandidateError::ComparisonCapacity { .. }
        | CandidateError::ComparisonCountOverflow
        | CandidateError::CandidateCapacity { .. }
        | CandidateError::ByteCapacity { .. }
        | CandidateError::AllocationCapacity => RepositoryError::Capacity,
    })?;
    let legal = !admitted.offers().is_empty();
    drop(admitted);
    let [candidate] = offered;
    Ok(legal.then_some(candidate))
}

pub(super) fn available(current: &Checkpoint, actor: EntityId) -> Result<bool, RepositoryError> {
    Ok(admit(current, current.state(), actor)?.is_some())
}

pub(super) fn stage(
    current: &Checkpoint,
    command: &CommandInput,
    state: &mut GameState,
    draws: &mut Vec<ActualDraw>,
    outcomes: &mut Vec<rpc::CombatOutcome>,
    supplier: DiceSource<'_>,
) -> Result<(), RepositoryError> {
    let GameCommand::ProposeAction { actor, choices, .. } = &command.command else {
        return Err(RepositoryError::InvalidCandidate);
    };
    // The registered journey binds member, command and current offer before this call.
    // Re-admit against the current working copy before any action spend or actual draw.
    let candidate = admit(current, state, *actor)?.ok_or(RepositoryError::InvalidCandidate)?;
    let options = AttackOptions {
        savage: choice(choices, "savage-attacker")? == "yes",
        graze: choice(choices, "graze")? == "yes",
    };
    let economy = rules::ActionEconomy {
        action_used: value(state, candidate.actor, "action-used")? != 0,
        bonus_action_used: value(state, candidate.actor, "bonus-used")? != 0,
    }
    .attack()
    .map_err(super::bad)?;
    outcomes.push(attack(
        state,
        command.operation,
        candidate.actor,
        candidate.target,
        options,
        draws,
        supplier,
    )?);
    set(
        state,
        candidate.actor,
        "action-used",
        u32::from(economy.action_used),
    )?;
    finish_combat(state)?;
    Ok(())
}
