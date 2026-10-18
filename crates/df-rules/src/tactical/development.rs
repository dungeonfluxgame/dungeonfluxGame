//! Bounded development edits to a detached battle. The session owner must commit the result.
use super::{Battle, Cell, CombatantSpec, RuleError, Side, state::Effect};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DevelopmentEdit {
    HitPoints(u16),
    Condition { name: String, present: bool },
    Cell(Cell),
    Resource { name: String, value: u16 },
    Turn,
    Initiative(i32),
    Restore,
    Remove,
}

impl Battle {
    /// Apply an explicitly authorized development edit, retaining snapshot invariants.
    /// Unknown conditions/resources, occupied cells and out-of-profile values are refused.
    pub fn development_edit(&self, id: u32, edit: &DevelopmentEdit) -> Result<Self, RuleError> {
        let mut next = self.clone();
        next.actor(id)?;
        if next.pending.is_some() {
            return Err(RuleError::PendingReaction);
        }
        match edit {
            DevelopmentEdit::Cell(cell) => {
                if !next.map.contains(*cell) {
                    return Err(RuleError::OutOfBounds);
                }
                if next.map.is_blocked(*cell) {
                    return Err(RuleError::Blocked);
                }
                if next
                    .actors
                    .iter()
                    .any(|actor| actor.id() != id && actor.cell() == *cell && !actor.health.dead)
                {
                    return Err(RuleError::Occupied);
                }
                next.actor_mut(id)?.spec.cell = *cell;
            }
            DevelopmentEdit::Turn => {
                if next.actor(id)?.health.dead {
                    return Err(RuleError::Incapacitated);
                }
                let previous = next.active_actor().ok_or(RuleError::UnknownActor)?;
                super::resolve::finish_turn(&mut next, previous)?;
                next.active = next
                    .order
                    .iter()
                    .position(|candidate| *candidate == id)
                    .ok_or(RuleError::UnknownActor)?;
                next.round = next.round.max(
                    next.actor(id)?
                        .turn_count
                        .checked_add(1)
                        .ok_or(RuleError::Capacity)?,
                );
                super::resolve::begin_turn(&mut next, id)?;
            }
            DevelopmentEdit::Initiative(value) => {
                let actor = next.actor(id)?;
                let modifier = crate::local_journey::ability_modifier(actor.spec.abilities[1]);
                if !(1 + modifier..=20 + modifier).contains(value) {
                    return Err(RuleError::InvalidDraw);
                }
                let active = next.active_actor();
                next.initiatives.insert(id, *value);
                next.order.sort_by_key(|candidate| {
                    std::cmp::Reverse(next.initiatives.get(candidate).copied().unwrap_or(0))
                });
                if let Some(active) = active {
                    next.active = next
                        .order
                        .iter()
                        .position(|candidate| *candidate == active)
                        .ok_or(RuleError::UnknownActor)?;
                }
            }
            DevelopmentEdit::Remove => {
                let side = next.actor(id)?.side();
                if next
                    .actors
                    .iter()
                    .filter(|actor| actor.side() == side)
                    .count()
                    <= 1
                {
                    return Err(RuleError::InvalidCombatant);
                }
                let active = next.active_actor().filter(|active| *active != id);
                next.actors.retain(|actor| actor.id() != id);
                next.order.retain(|candidate| *candidate != id);
                next.initiatives.remove(&id);
                next.helped_attacks.retain(|effect| {
                    effect.helper != id && effect.ally != id && effect.target != id
                });
                for actor in &mut next.actors {
                    actor.slow.retain(|effect| effect.origin != id);
                    actor.sap.retain(|effect| effect.origin != id);
                    actor.vex.retain(|(origin, _)| *origin != id);
                }
                next.active = active
                    .and_then(|active| next.order.iter().position(|candidate| *candidate == active))
                    .unwrap_or(next.active.min(next.order.len() - 1));
                next.needs_death_save = next.actor(next.order[next.active])?.health.hit_points == 0;
            }
            _ => {
                let actor = next.actor_mut(id)?;
                match edit {
                    DevelopmentEdit::HitPoints(value) => {
                        if *value > actor.health.maximum {
                            return Err(RuleError::InvalidCombatant);
                        }
                        actor.health.hit_points = *value;
                        actor.health.unconscious = *value == 0;
                        actor.health.prone |= *value == 0;
                        actor.health.dead = *value == 0 && actor.side() == Side::Enemies;
                        actor.health.knocked_out = false;
                        actor.health.stable = false;
                        actor.health.death_successes = 0;
                        actor.health.death_failures = 0;
                    }
                    DevelopmentEdit::Restore => {
                        actor.health.hit_points = actor.health.maximum;
                        actor.health.prone = false;
                        actor.health.unconscious = false;
                        actor.health.dead = false;
                        actor.health.knocked_out = false;
                        actor.health.stable = false;
                        actor.health.death_successes = 0;
                        actor.health.death_failures = 0;
                    }
                    DevelopmentEdit::Condition { name, present } => match name.as_str() {
                        "prone" if !*present && actor.health.unconscious => {
                            return Err(RuleError::Incapacitated);
                        }
                        "prone" => actor.health.prone = *present,
                        "unconscious"
                            if !*present && (actor.health.hit_points == 0 || actor.health.dead) =>
                        {
                            return Err(RuleError::Incapacitated);
                        }
                        "unconscious" => {
                            actor.health.unconscious = *present;
                            actor.health.prone |= *present;
                            actor.health.knocked_out &= *present;
                        }
                        "dead"
                            if (*present
                                && (actor.health.hit_points != 0 || actor.health.stable))
                                || (!*present
                                    && actor.health.dead
                                    && actor.health.hit_points == 0) =>
                        {
                            return Err(RuleError::InvalidCombatant);
                        }
                        "dead" => actor.health.dead = *present,
                        "stable"
                            if *present && (actor.health.hit_points != 0 || actor.health.dead) =>
                        {
                            return Err(RuleError::InvalidCombatant);
                        }
                        "stable" => {
                            actor.health.stable = *present;
                            if *present {
                                actor.health.death_successes = 0;
                                actor.health.death_failures = 0;
                            }
                        }
                        "dodging" => {
                            actor.budget.dodging = *present;
                            actor.budget.action_used |= *present;
                        }
                        "disengaged" => actor.budget.disengaged = *present,
                        "slowed" | "sapped" => {
                            let effect = Effect {
                                origin: id,
                                expires_turn: actor.turn_count + 1,
                            };
                            let effects = if name == "slowed" {
                                &mut actor.slow
                            } else {
                                &mut actor.sap
                            };
                            effects.clear();
                            if *present {
                                effects.push(effect);
                            }
                        }
                        _ => return Err(RuleError::InvalidCombatant),
                    },
                    DevelopmentEdit::Resource { name, value } => match name.as_str() {
                        "second-wind" => {
                            let cap = actor
                                .spec
                                .fighter_level()
                                .map_or(0, |level| level.second_wind_maximum());
                            if *value > u16::from(cap) {
                                return Err(RuleError::InvalidCombatant);
                            }
                            actor.spec.second_wind_uses = *value as u8;
                        }
                        "action-surge" => {
                            if *value > 1
                                || actor
                                    .spec
                                    .fighter_level()
                                    .is_none_or(|level| level.level() < 2)
                            {
                                return Err(RuleError::InvalidCombatant);
                            }
                            actor.action_surge_spent = *value == 0;
                            actor.additional_actions = 0;
                            actor.extra_attacks = 0;
                            actor.additional_dash = false;
                        }
                        "ammo" => {
                            let maximum = 1000;
                            if *value > maximum {
                                return Err(RuleError::InvalidCombatant);
                            }
                            actor.spec.arrows = *value;
                            if actor.spec.weapons.contains(&super::Weapon::LightCrossbow) {
                                actor.spec.bolts = *value;
                            }
                        }
                        _ => return Err(RuleError::InvalidCombatant),
                    },
                    _ => return Err(RuleError::InvalidCombatant),
                }
            }
        }
        let previous_active = self.active_actor();
        let previously_dying = previous_active
            .map(|active| self.actor(active))
            .transpose()?
            .is_some_and(|actor| {
                actor.side() == Side::Heroes
                    && actor.health.hit_points == 0
                    && !actor.health.dead
                    && !actor.health.stable
                    && !actor.health.knocked_out
            });
        let active = next.active_actor();
        let eligible = active
            .map(|active| next.actor(active))
            .transpose()?
            .is_some_and(|actor| {
                actor.side() == Side::Heroes
                    && actor.health.hit_points == 0
                    && !actor.health.dead
                    && !actor.health.stable
                    && !actor.health.knocked_out
            });
        next.needs_death_save = eligible
            && (matches!(edit, DevelopmentEdit::Turn)
                || self.needs_death_save
                || active != previous_active
                || !previously_dying);
        Self::from_snapshot(next.map.clone(), &next.snapshot()?)
    }

    /// Add a source-validated enemy at a legal free cell; existing actors and budgets persist.
    pub fn development_add(
        &self,
        spec: CombatantSpec,
        initiative_total: i32,
    ) -> Result<Self, RuleError> {
        if self.pending.is_some() {
            return Err(RuleError::PendingReaction);
        }
        if spec.side != Side::Enemies
            || self
                .actors
                .iter()
                .filter(|actor| actor.side() == Side::Enemies)
                .count()
                >= 4
        {
            return Err(RuleError::Capacity);
        }
        if self.actors.iter().any(|actor| actor.id() == spec.id) {
            return Err(RuleError::DuplicateId);
        }
        if self
            .actors
            .iter()
            .any(|actor| actor.cell() == spec.cell && !actor.health.dead)
        {
            return Err(RuleError::Occupied);
        }
        let hero = self
            .actors
            .iter()
            .find(|actor| actor.side() == Side::Heroes)
            .ok_or(RuleError::InvalidCombatant)?;
        let mut hero_profile = hero.spec.clone();
        hero_profile.arrows = 20;
        hero_profile.second_wind_uses = hero_profile
            .fighter_level()
            .ok_or(RuleError::InvalidBuild)?
            .second_wind_maximum();
        let template = super::state::begin_faces(
            self.map.clone(),
            vec![hero_profile, spec.clone()],
            &[10, 10],
        )?;
        let added = template.actor(spec.id)?.clone();
        let mut next = self.clone();
        let active = next.active_actor();
        next.order.push(spec.id);
        let modifier = crate::local_journey::ability_modifier(spec.abilities[1]);
        if !(1 + modifier..=20 + modifier).contains(&initiative_total) {
            return Err(RuleError::InvalidDraw);
        }
        next.initiatives.insert(spec.id, initiative_total);
        next.actors.push(added);
        next.order
            .sort_by_key(|id| std::cmp::Reverse(next.initiatives.get(id).copied().unwrap_or(0)));
        if let Some(active) = active {
            next.active = next
                .order
                .iter()
                .position(|id| *id == active)
                .ok_or(RuleError::UnknownActor)?;
        }
        Self::from_snapshot(next.map.clone(), &next.snapshot()?)
    }

    /// Attach the historical committed initiative totals to an older decoded snapshot.
    /// This does not reroll, reorder actors or start another turn.
    pub fn with_development_initiatives(
        mut self,
        totals: &[(u32, i32)],
    ) -> Result<Self, RuleError> {
        for (id, total) in totals {
            let actor = self.actor(*id)?;
            let modifier = crate::local_journey::ability_modifier(actor.spec.abilities[1]);
            if !(1 + modifier..=20 + modifier).contains(total) {
                return Err(RuleError::InvalidDraw);
            }
            self.initiatives.entry(*id).or_insert(*total);
        }
        Ok(self)
    }
    /// Committed initiative totals, when present in this snapshot revision.
    pub fn development_initiative(&self, id: u32) -> Option<i32> {
        self.initiatives.get(&id).copied()
    }
}
