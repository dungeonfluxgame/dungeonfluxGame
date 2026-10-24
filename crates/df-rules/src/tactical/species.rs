//! SRD 5.2.1 Species: optional ancestry triggers and Dragonborn saving-throw area damage.
use super::resolve::{D20Reroll, D20Test, Draws, take_d20};
use super::{Battle, Cell, DamageType, RuleError, Size};
use crate::species::{Ancestry, GiantAncestry};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BreathShape {
    Cone,
    Line,
}
/// One of eight grid directions. Validation rejects zero or nonunit components.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BreathDirection {
    pub column: i8,
    pub row: i8,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SavingThrowSummary {
    pub target: u32,
    pub die: u32,
    pub modifier: i16,
    pub dc: i16,
    pub success: bool,
    pub damage: u16,
    pub damage_type: DamageType,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeciesReaction {
    StoneEndurance,
    StormThunder,
}
#[derive(Deserialize, Serialize, Clone, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpeciesDamage {
    pub source: u32,
    pub target: u32,
    pub amount: u16,
    pub critical: bool,
    pub knockout: bool,
    pub applied: bool,
}
impl Battle {
    /// Current source-required optional reaction; all normal actions wait for this decision.
    pub fn species_reaction_offer(&self) -> Option<(u32, SpeciesReaction)> {
        let event = self.species_damage.first()?;
        let actor = self.actor(event.target).ok()?;
        if event.amount == 0
            || actor.health.incapacitated()
            || actor.budget.reaction_used
            || actor.species_uses_remaining() == 0
        {
            return None;
        }
        match actor.spec.species_options.ancestry {
            Some(Ancestry::Giant(GiantAncestry::Stone)) if !event.applied => {
                Some((event.target, SpeciesReaction::StoneEndurance))
            }
            Some(Ancestry::Giant(GiantAncestry::Storm))
                if event.applied && actor.distance_to(self.actor(event.source).ok()?) <= 60 =>
            {
                Some((event.target, SpeciesReaction::StormThunder))
            }
            _ => None,
        }
    }
}
pub(super) fn queue_damage(
    battle: &mut Battle,
    source: u32,
    target: u32,
    amount: u16,
    critical: bool,
    knockout: bool,
) -> Result<(), RuleError> {
    if battle.species_damage.len() >= 64 {
        return Err(RuleError::Capacity);
    }
    battle.species_damage.push(SpeciesDamage {
        source,
        target,
        amount,
        critical,
        knockout,
        applied: false,
    });
    Ok(())
}
pub(super) fn flush_damage(battle: &mut Battle, draws: &mut Draws<'_>) -> Result<(), RuleError> {
    while let Some(event) = battle.species_damage.first().cloned() {
        if battle.species_reaction_offer().is_some() {
            return Ok(());
        }
        if !event.applied {
            super::resolve::apply_damage(
                battle,
                event.target,
                event.amount,
                event.critical,
                event.knockout,
                draws,
            )?;
            battle.species_damage[0].applied = true;
            if battle.species_reaction_offer().is_some() {
                return Ok(());
            }
        }
        battle.species_damage.remove(0);
    }
    Ok(())
}
pub(super) fn react_damage(
    battle: &mut Battle,
    actor: u32,
    use_trait: bool,
    draws: &mut Draws<'_>,
) -> Result<(), RuleError> {
    let (target, kind) = battle
        .species_reaction_offer()
        .ok_or(RuleError::NoPendingReaction)?;
    if actor != target {
        return Err(RuleError::InvalidReaction);
    }
    let mut event = battle.species_damage.remove(0);
    if use_trait {
        let actor = battle.actor_mut(actor)?;
        actor.budget.reaction_used = true;
        actor.species_uses_spent += 1;
        match kind {
            // SRD p.85 Stone's Endurance: reaction reduces the triggering damage by d12 + CON.
            SpeciesReaction::StoneEndurance => {
                let reduction = (draws.take(12)? as i32
                    + crate::local_journey::ability_modifier(actor.spec.abilities[2]))
                .max(0) as u16;
                event.amount = event.amount.saturating_sub(reduction);
            }
            // SRD p.86 Storm's Thunder: after damage, retaliate against its source within 60 ft.
            SpeciesReaction::StormThunder => {
                let amount = super::resolve::defended_damage(
                    battle.actor(event.source)?.spec(),
                    DamageType::Thunder,
                    draws.take(8)? as u16,
                );
                queue_damage(battle, event.target, event.source, amount, false, false)?;
            }
        }
    }
    if !event.applied {
        super::resolve::apply_damage(
            battle,
            event.target,
            event.amount,
            event.critical,
            event.knockout,
            draws,
        )?;
    }
    flush_damage(battle, draws)
}
/// A preselected optional boon only spends a use on a damaging hit. A miss never spends.
pub(super) fn attack_boon(
    battle: &mut Battle,
    actor: u32,
    target: u32,
    critical: bool,
    draws: &mut Draws<'_>,
) -> Result<(u16, Option<DamageType>), RuleError> {
    let ancestry = battle.actor(actor)?.spec.species_options.ancestry;
    let effect = super::state::Effect {
        origin: actor,
        expires_turn: battle
            .actor(actor)?
            .turn_count
            .checked_add(1)
            .ok_or(RuleError::Capacity)?,
    };
    // SRD p.16 Critical Hits: roll all of the attack's damage dice twice.
    let mut roll_bonus = |sides| -> Result<u16, RuleError> {
        let mut total = draws.take(sides)? as u16;
        if critical {
            total += draws.take(sides)? as u16;
        }
        Ok(total)
    };
    let amount = match ancestry {
        // SRD p.85 Fire's Burn and Frost's Chill: added damage on a damaging attack roll hit.
        Some(Ancestry::Giant(GiantAncestry::Fire)) => super::resolve::defended_damage(
            battle.actor(target)?.spec(),
            DamageType::Fire,
            roll_bonus(10)?,
        ),
        Some(Ancestry::Giant(GiantAncestry::Frost)) => {
            if !battle.actor(target)?.frost_chill.contains(&effect) {
                battle.actor_mut(target)?.frost_chill.push(effect);
            }
            super::resolve::defended_damage(
                battle.actor(target)?.spec(),
                DamageType::Cold,
                roll_bonus(6)?,
            )
        }
        // SRD p.85 Hill's Tumble has no save and affects only Large or smaller creatures.
        Some(Ancestry::Giant(GiantAncestry::Hill))
            if battle.actor(target)?.spec.size != Size::Huge =>
        {
            battle.actor_mut(target)?.health.prone = true;
            0
        }
        _ => return Err(RuleError::InvalidTarget),
    };
    battle.actor_mut(actor)?.species_uses_spent += 1;
    Ok((
        amount,
        match ancestry {
            Some(Ancestry::Giant(GiantAncestry::Fire)) => Some(DamageType::Fire),
            Some(Ancestry::Giant(GiantAncestry::Frost)) => Some(DamageType::Cold),
            _ => None,
        },
    ))
}
pub(super) fn validate_boon(battle: &Battle, actor: u32) -> Result<(), RuleError> {
    let actor = battle.actor(actor)?;
    if actor.species_uses_remaining() == 0
        || !matches!(
            actor.spec.species_options.ancestry,
            Some(Ancestry::Giant(
                GiantAncestry::Fire | GiantAncestry::Frost | GiantAncestry::Hill
            ))
        )
    {
        return Err(RuleError::RecoveryUnavailable);
    }
    Ok(())
}
pub(super) fn breath(
    battle: &mut Battle,
    actor: u32,
    shape: BreathShape,
    direction: BreathDirection,
    spend: super::AttackSpend,
    draws: &mut Draws<'_>,
    rerolls: &mut Vec<D20Reroll>,
) -> Result<Vec<SavingThrowSummary>, RuleError> {
    let source = battle.actor(actor)?;
    let Some(Ancestry::Draconic(ancestry)) = source.spec.species_options.ancestry else {
        return Err(RuleError::InvalidBuild);
    };
    if source.species_uses_remaining() == 0 {
        return Err(RuleError::RecoveryUnavailable);
    }
    let dx = i32::from(direction.column);
    let dy = i32::from(direction.row);
    if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dy) || (dx == 0 && dy == 0) {
        return Err(RuleError::InvalidTarget);
    }
    if !matches!(
        spend,
        super::AttackSpend::Action | super::AttackSpend::Multiattack
    ) {
        return Err(RuleError::InvalidExtraAttack);
    }
    let origin = source.cell();
    let level = source.spec.fighter_level().ok_or(RuleError::InvalidBuild)?;
    let dc = 8
        + crate::local_journey::ability_modifier(source.spec.abilities[2]) as i16
        + i16::from(level.proficiency_bonus());
    // SRD p.84 Breath Weapon: all creatures, including allies, in 15-ft cone or 30-by-5-ft line.
    // Cell-centre inclusion and eight grid directions are explicit flat-grid geometry policy.
    let targets = battle
        .combatants()
        .iter()
        .filter(|target| target.id() != actor && !target.health.dead)
        .filter(|target| {
            target.spec.space_cells(target.cell()).iter().any(|cell| {
                in_breath(origin, *cell, shape, dx, dy)
                    && battle
                        .map
                        .cover_between_for_size(origin, *cell, &[], target.spec.size)
                        .is_ok_and(|cover| cover != super::Cover::Total)
            })
        })
        .map(|target| target.id())
        .collect::<Vec<_>>();
    super::resolve::spend_attack(battle, actor, super::Weapon::UnarmedStrike, spend)?;
    battle.actor_mut(actor)?.species_uses_spent += 1;
    let mut damage = 0;
    // SRD p.84: 1d10 at 1st level, 2d10 at 5th. Shared roll for the area (Damage Rolls p.16).
    for _ in 0..if level.level() >= 5 { 2 } else { 1 } {
        damage += draws.take(10)? as u16;
    }
    let mut summaries = Vec::new();
    for target in targets {
        let spec = battle.actor(target)?.spec();
        // SRD stat blocks MOD/SAVE columns: admitted creatures have untrained Dexterity saves.
        // Fighters save proficiency is STR/CON; no Dexterity proficiency is invented here.
        let modifier = crate::local_journey::ability_modifier(spec.abilities[1]) as i16;
        let automatic_failure = battle.actor(target)?.health.unconscious;
        let die = if automatic_failure {
            0
        } else {
            take_d20(spec, target, D20Test::SavingThrow, draws, rerolls)?
        };
        // SRD p.9 Dodge grants Dexterity-save Advantage; only its attack-roll benefit requires sight.
        let die = if !automatic_failure && battle.actor(target)?.budget.dodging {
            die.max(take_d20(
                spec,
                target,
                D20Test::SavingThrow,
                draws,
                rerolls,
            )?)
        } else {
            die
        };
        let cover_bonus = match battle.attack_cover(actor, target)? {
            super::Cover::Half => 2,
            super::Cover::ThreeQuarters => 5,
            _ => 0,
        };
        let modifier = modifier + cover_bonus;
        let success = !automatic_failure && die as i16 + modifier >= dc;
        let damage = super::resolve::defended_damage(
            spec,
            ancestry.damage_type(),
            if success { damage / 2 } else { damage },
        );
        super::resolve::mark_regeneration_inhibition(
            battle,
            target,
            ancestry.damage_type(),
            damage,
        )?;
        queue_damage(battle, actor, target, damage, false, false)?;
        summaries.push(SavingThrowSummary {
            target,
            die,
            modifier,
            dc,
            success,
            damage,
            damage_type: ancestry.damage_type(),
        });
    }
    flush_damage(battle, draws)?;
    Ok(summaries)
}
fn in_breath(origin: Cell, target: Cell, shape: BreathShape, dx: i32, dy: i32) -> bool {
    let x = i64::from(target.0) - i64::from(origin.0);
    let dx = i64::from(dx);
    let dy = i64::from(dy);
    let y = i64::from(target.1) - i64::from(origin.1);
    let forward = x * dx + y * dy;
    let cross = (x * dy - y * dx).abs();
    let norm = dx * dx + dy * dy;
    let length = match shape {
        BreathShape::Cone => 3,
        BreathShape::Line => 6,
    };
    forward > 0
        && forward * forward <= length * length * norm
        && match shape {
            BreathShape::Cone => cross * 2 <= forward,
            BreathShape::Line => cross == 0,
        }
}
