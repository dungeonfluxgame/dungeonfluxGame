//! Complete versioned detached state. Decode requires the caller's admitted immutable map.
//! This codec grants no source, actor or persistence authority.
use super::{
    Battle, BattleMap, RuleError,
    state::{Combatant, HelpEffect, PendingMove},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const VERSION: u16 = 2;
const MAXIMUM_BYTES: usize = 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u16,
    source: String,
    layout: String,
    actors: Vec<Combatant>,
    order: Vec<u32>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    initiatives: std::collections::BTreeMap<u32, i32>,
    active: usize,
    round: u32,
    pending: Option<PendingMove>,
    #[serde(default)]
    species_damage: Vec<super::species::SpeciesDamage>,
    helped_attacks: Vec<HelpEffect>,
    needs_death_save: bool,
}

impl Battle {
    /// Preserve all health, equipment, expiries, budgets and optional movement/reaction state.
    /// Map bytes/hash and actor-to-entity bindings remain the session owner's pinned records.
    pub fn snapshot(&self) -> Result<Vec<u8>, RuleError> {
        let bytes = serde_json::to_vec(&Snapshot {
            version: VERSION,
            source: if self.actors.iter().any(|actor| {
                actor
                    .spec
                    .fighter_level()
                    .is_some_and(|progression| progression.level() > 1)
            }) {
                super::FIGHTER_SOURCE_REVISION
            } else {
                super::SOURCE_REVISION
            }
            .to_owned(),
            layout: self.map.id().to_owned(),
            actors: self.actors.clone(),
            order: self.order.clone(),
            initiatives: self.initiatives.clone(),
            active: self.active,
            round: self.round,
            pending: self.pending.clone(),
            species_damage: self.species_damage.clone(),
            helped_attacks: self.helped_attacks.clone(),
            needs_death_save: self.needs_death_save,
        })
        .map_err(|_| RuleError::SnapshotData)?;
        if bytes.len() > MAXIMUM_BYTES {
            return Err(RuleError::Capacity);
        }
        Ok(bytes)
    }

    /// Refuse incompatible or structurally invalid state; never manufacture missing fields.
    /// Caller must separately admit the immutable map revision and canonical actor bindings.
    pub fn from_snapshot(map: BattleMap, bytes: &[u8]) -> Result<Self, RuleError> {
        if bytes.len() > MAXIMUM_BYTES {
            return Err(RuleError::Capacity);
        }
        let state: Snapshot = serde_json::from_slice(bytes).map_err(|_| RuleError::SnapshotData)?;
        if state.version != VERSION
            || state.source
                != if state.actors.iter().any(|actor| {
                    actor
                        .spec
                        .fighter_level()
                        .is_some_and(|progression| progression.level() > 1)
                }) {
                    super::FIGHTER_SOURCE_REVISION
                } else {
                    super::SOURCE_REVISION
                }
            || state.layout != map.id()
            || state.round == 0
            || !(2..=64).contains(&state.actors.len())
            || state.order.len() != state.actors.len()
            || state.active >= state.order.len()
            || state.helped_attacks.len() > 64
        {
            return Err(RuleError::SnapshotData);
        }
        let mut ids = BTreeSet::new();
        let mut living_cells = BTreeSet::new();
        for actor in &state.actors {
            // A pending mover is passing through this cell and has not ended its move here.
            let in_transit = state.pending.as_ref().is_some_and(|pending| {
                pending.actor == actor.id()
                    && pending.next > 0
                    && pending.path.get(pending.next - 1) == Some(&actor.cell())
            });
            if actor.id() == 0
                || !ids.insert(actor.id())
                || !map.contains(actor.cell())
                || map.is_blocked(actor.cell())
                || actor.turn_count > state.round
                || actor.health.maximum != actor.spec.hit_points
                || actor.health.hit_points > actor.health.maximum
                || actor.health.death_successes > 3
                || actor.health.death_failures > 3
                || actor.spec.second_wind_uses
                    > actor
                        .spec
                        .fighter_level()
                        .map_or(0, |progression| progression.second_wind_maximum())
                || actor.spec.arrows > 1000
                || actor.spec.bolts > 1000
                || actor.held.len() > 2
                || actor.dropped.len() > 12
                || actor.weapon_counts.len() > 12
                || actor.extra_attacks
                    > if actor
                        .spec
                        .fighter_level()
                        .is_some_and(|progression| progression.level() >= 5)
                    {
                        if actor.action_surge_spent { 2 } else { 1 }
                    } else {
                        actor.spec.multiattack_count() - 1
                    }
                || (actor.extra_attacks > 0 && !actor.budget.action_used)
                || actor.additional_actions > 1
                || (actor.additional_actions > 0 && !actor.action_surge_spent)
                || (actor.action_surge_spent
                    && actor
                        .spec
                        .fighter_level()
                        .is_none_or(|progression| progression.level() < 2))
                || actor.feature_movement_ft > actor.spec.speed_ft / 2
                || (actor.feature_movement_ft > 0
                    && actor
                        .spec
                        .fighter_level()
                        .is_none_or(|progression| !progression.champion()))
                || (actor.additional_dash && (!actor.action_surge_spent || !actor.budget.dash))
                || (!actor.health.dead
                    && !in_transit
                    && actor
                        .spec
                        .space_cells(actor.cell())
                        .iter()
                        .any(|cell| map.is_blocked(*cell) || !living_cells.insert(*cell)))
                || actor.budget.movement_spent_ft > 240
                || (actor.budget.dash && !actor.budget.action_used)
                || (actor.budget.dodging && !actor.budget.action_used)
                || (actor.health.knocked_out
                    && (actor.health.hit_points != 1
                        || !actor.health.unconscious
                        || actor.health.dead))
                || (actor.regeneration_inhibited && actor.spec.kind != super::CreatureKind::Troll)
                || actor.frost_chill.len() > 64
                || actor.species_uses_spent
                    > actor
                        .spec
                        .fighter_level()
                        .map_or(0, |level| level.proficiency_bonus())
                || (actor.species_uses_spent > 0
                    && !matches!(
                        actor.spec.species,
                        Some(super::HeroSpecies::Dragonborn | super::HeroSpecies::Goliath)
                    ))
                || actor.slow.len() > 64
                || actor.sap.len() > 64
                || actor.vex.len() > 64
                || (actor.health.dead && actor.health.hit_points != 0)
                || (actor.health.unconscious && !actor.health.prone)
            {
                return Err(RuleError::SnapshotData);
            }
            if (!actor.spec.shield && actor.shield_equipped)
                || actor.held.iter().copied().collect::<BTreeSet<_>>().len() != actor.held.len()
                || actor
                    .held
                    .iter()
                    .any(|weapon| actor.weapon_count(*weapon) == 0)
                || actor.weapon_counts.keys().copied().collect::<BTreeSet<_>>()
                    != actor.spec.weapons.iter().copied().collect::<BTreeSet<_>>()
                || actor.weapon_counts.iter().any(|(weapon, count)| {
                    usize::from(*count)
                        + actor
                            .dropped
                            .iter()
                            .filter(|(dropped, _)| dropped == weapon)
                            .count()
                        > usize::from(
                            if actor.spec.kind == super::CreatureKind::Ogre
                                && *weapon == super::Weapon::Javelin
                            {
                                3_u16
                            } else if *weapon == super::Weapon::Javelin {
                                8_u16
                            } else if actor.spec.kind == super::CreatureKind::Kobold
                                && *weapon == super::Weapon::Dagger
                            {
                                3
                            } else {
                                1
                            },
                        )
                })
            {
                return Err(RuleError::SnapshotData);
            }
            let mut original = actor.spec.clone();
            // Source creation is fixed; resources can only decrease/recover through resolve.
            if original.kind == super::CreatureKind::Hero {
                original.arrows = 20;
                original.second_wind_uses = original
                    .fighter_level()
                    .ok_or(RuleError::SnapshotData)?
                    .second_wind_maximum();
            }
            if actor.spec.shield && !actor.shield_equipped {
                original.armor_class = original
                    .armor_class
                    .checked_add(2)
                    .ok_or(RuleError::SnapshotData)?;
            }
            super::state::validate_source_profile(&original)?;
            if actor
                .held
                .iter()
                .chain(actor.dropped.iter().map(|(weapon, _)| weapon))
                .any(|weapon| !actor.spec.weapons.contains(weapon))
                || actor.dropped.iter().any(|(_, cell)| !map.contains(*cell))
            {
                return Err(RuleError::SnapshotData);
            }
        }
        if !state.initiatives.is_empty()
            && (state.initiatives.len() != state.actors.len()
                || state.actors.iter().any(|actor| {
                    let modifier = actor.spec.initiative_modifier();
                    state
                        .initiatives
                        .get(&actor.id())
                        .is_none_or(|total| !(1 + modifier..=20 + modifier).contains(total))
                }))
        {
            return Err(RuleError::SnapshotData);
        }
        for effect in &state.helped_attacks {
            let helper = state
                .actors
                .iter()
                .find(|actor| actor.id() == effect.helper)
                .ok_or(RuleError::SnapshotData)?;
            let ally = state
                .actors
                .iter()
                .find(|actor| actor.id() == effect.ally)
                .ok_or(RuleError::SnapshotData)?;
            let target = state
                .actors
                .iter()
                .find(|actor| actor.id() == effect.target)
                .ok_or(RuleError::SnapshotData)?;
            if effect.helper == effect.ally
                || helper.side() != ally.side()
                || helper.side() == target.side()
                || helper.turn_count != effect.helper_turn_count
                || !helper.budget.action_used
            {
                return Err(RuleError::SnapshotData);
            }
        }
        if state.order.iter().copied().collect::<BTreeSet<_>>() != ids {
            return Err(RuleError::SnapshotData);
        }
        for actor in &state.actors {
            if actor
                .slow
                .iter()
                .chain(&actor.sap)
                .chain(&actor.frost_chill)
                .any(|effect| !ids.contains(&effect.origin) || effect.expires_turn == 0)
                || actor
                    .vex
                    .iter()
                    .any(|(target, until)| !ids.contains(target) || *until == 0)
            {
                return Err(RuleError::SnapshotData);
            }
        }
        if let Some(pending) = &state.pending
            && (state.order.get(state.active) != Some(&pending.actor)
                || pending.path.is_empty()
                || pending.path.len() > 1024
                || pending.next >= pending.path.len()
                || (pending.eligible.is_empty() && state.species_damage.is_empty())
                || pending.eligible.len() > 64
                || pending
                    .eligible
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != pending.eligible.len()
                || pending
                    .eligible
                    .iter()
                    .any(|id| !ids.contains(id) || *id == pending.actor)
                || pending.path.iter().any(|cell| !map.contains(*cell)))
        {
            return Err(RuleError::SnapshotData);
        }
        let battle = Self {
            map,
            actors: state.actors,
            order: state.order,
            initiatives: state.initiatives,
            active: state.active,
            round: state.round,
            pending: state.pending,
            species_damage: state.species_damage,
            helped_attacks: state.helped_attacks,
            needs_death_save: state.needs_death_save,
        };
        if battle.species_damage.len() > 64
            || battle.species_damage.iter().any(|event| {
                event.source == event.target
                    || battle.actor(event.source).is_err()
                    || battle.actor(event.target).is_err()
            })
            || (!battle.species_damage.is_empty() && battle.species_reaction_offer().is_none())
        {
            return Err(RuleError::SnapshotData);
        }
        if let Some(pending) = &battle.pending {
            let mover = battle.actor(pending.actor)?;
            let destination = pending.path[pending.next];
            let mut from = mover.cell();
            let remaining_path_is_clear = pending.path[pending.next..].iter().all(|to| {
                let clear = battle.map.step_clear(from, *to);
                from = *to;
                clear
            });
            if mover.health.incapacitated()
                || mover.budget.disengaged
                || mover.cell().distance(destination) != 5
                || battle.map.is_blocked(destination)
                || !remaining_path_is_clear
                || pending
                    .path
                    .windows(2)
                    .any(|cells| cells[0].distance(cells[1]) != 5)
                || pending.path.iter().any(|cell| battle.map.is_blocked(*cell))
                || (pending.next > 0 && pending.path[pending.next - 1] != mover.cell())
                || pending.eligible.iter().any(|id| {
                    battle.actor(*id).is_ok_and(|reactor| {
                        reactor.side() == mover.side()
                            || reactor.health.incapacitated()
                            || reactor.budget.reaction_used
                            || reactor.cell().distance(mover.cell()) > 5
                            || reactor.cell().distance(destination) <= 5
                            || !battle.can_see_actor(reactor, mover)
                    })
                })
            {
                return Err(RuleError::SnapshotData);
            }
        }
        if battle.needs_death_save
            && battle.active_actor().is_none_or(|id| {
                battle.actor(id).map_or(true, |actor| {
                    actor.health.hit_points != 0
                        || actor.health.dead
                        || actor.health.stable
                        || actor.health.knocked_out
                })
            })
        {
            return Err(RuleError::SnapshotData);
        }
        Ok(battle)
    }
}
