//! Deterministic policy over PLAY-T01 rules offers. Policy ordering is custom AI behaviour,
//! never a mechanical bonus. Canonical admission/ranking reuse caller-provided source pins.
use crate::candidates::{
    CandidateContext, CandidateError, CandidateLimits, LegalOfferOwner, enumerate_candidates,
};
use crate::ranking::{
    RankingObservation, TacticalKnowledge, UtilityAssignment, UtilityLimits, UtilityPolicy,
    UtilityRankingError, rank_tactics,
};
use df_rules::tactical::{
    Action, AttackSpend, AttackUse, Battle, Cell, CreatureKind, DrawBinding, RuleError, Weapon,
    resolve,
};

/// Current canonical source/basis/policy and actor perception, supplied by engine/session.
/// These values are never manufactured from tactical positions or a display projection.
pub struct TacticContext<'a> {
    pub candidates: CandidateContext<'a>,
    pub observation: &'a RankingObservation<'a>,
    pub knowledge: &'a TacticalKnowledge<'a>,
    pub policy: &'a UtilityPolicy<'a>,
    pub draws: DrawBinding<'a>,
}
#[derive(Debug, Eq, PartialEq)]
pub enum TacticError {
    Rules(RuleError),
    Candidates(CandidateError<RuleError>),
    Ranking(UtilityRankingError),
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Offer {
    action: Action,
}
struct Authority<'a> {
    battle: &'a Battle,
    actor: u32,
    context: CandidateContext<'a>,
    draws: DrawBinding<'a>,
}
impl LegalOfferOwner for Authority<'_> {
    type Observation = u32;
    type Offer = Offer;
    type Error = RuleError;
    fn validate_observation(
        &self,
        actor: &u32,
        context: CandidateContext<'_>,
    ) -> Result<(), RuleError> {
        if *actor != self.actor
            || context.basis != self.context.basis
            || context.pins != self.context.pins
        {
            return Err(RuleError::NotYourTurn);
        }
        self.battle.actor(*actor)?;
        Ok(())
    }
    fn is_current_legal(
        &self,
        _: &u32,
        _: CandidateContext<'_>,
        offer: &Offer,
    ) -> Result<bool, RuleError> {
        // MissingDraw is a legal prepared roll requirement, never a fabricated outcome.
        match resolve(self.battle, self.actor, &offer.action, self.draws, &[]) {
            Ok(_) | Err(RuleError::MissingDraw { .. }) => Ok(true),
            Err(_) => Ok(false),
        }
    }
    fn same_offer(&self, left: &Offer, right: &Offer) -> Result<bool, RuleError> {
        Ok(left == right)
    }
    fn offer_bytes(&self, offer: &Offer) -> Result<usize, RuleError> {
        Ok(std::mem::size_of::<Offer>()
            + match &offer.action {
                Action::Move(path) => path.len() * std::mem::size_of::<Cell>(),
                _ => 0,
            })
    }
}

/// Propose one legal deterministic next step, then admit/rank it with existing combat APIs.
/// Empty contributions preserve the explicit table policy order and introduce no fictitious
/// evidence or utility bonus. Real caller pins/knowledge/policy must agree or ranking rejects.
/// Opportunity windows are explicit: this policy declines its own optional reaction.
pub fn choose(
    battle: &Battle,
    actor: u32,
    context: TacticContext<'_>,
) -> Result<Option<Action>, TacticError> {
    let proposed = propose(battle, actor).map_err(TacticError::Rules)?;
    let offers: Vec<Offer> = proposed
        .into_iter()
        .map(|action| Offer { action })
        .collect();
    let authority = Authority {
        battle,
        actor,
        context: context.candidates,
        draws: context.draws,
    };
    let admitted = enumerate_candidates(
        &authority,
        &actor,
        context.candidates,
        context.candidates,
        &offers,
        CandidateLimits {
            max_offers: 1,
            max_identity_comparisons: 0,
            max_candidates: 1,
            max_candidate_bytes: 8192,
        },
    )
    .map_err(TacticError::Candidates)?;
    let assignments: Vec<_> = admitted
        .offers()
        .iter()
        .map(|offer| UtilityAssignment {
            offer: *offer,
            contributions: &[],
        })
        .collect();
    let ranking = rank_tactics(
        context.observation,
        &admitted,
        &assignments,
        context.knowledge,
        context.policy,
        UtilityLimits {
            candidates: 1,
            contributions: 0,
            knowledge_records: 4096,
            criteria: 32,
        },
    )
    .map_err(TacticError::Ranking)?;
    Ok(ranking
        .preferred()
        .map(|tactic| tactic.offer().action.clone()))
}

/// The bounded table policy, independently useful before canonical engine integration.
/// Rules must re-admit this staged proposal. Source stat blocks supply weapons; no rolls,
/// combat resources, damage, conditions or positions are applied by the policy itself.
pub fn propose(battle: &Battle, id: u32) -> Result<Option<Action>, RuleError> {
    let actor = battle.actor(id)?;
    if battle.reaction_offers().first().copied() == Some(id) {
        return Ok(Some(Action::Opportunity { attack: None }));
    }
    if !battle.reaction_offers().is_empty() {
        return Err(RuleError::PendingReaction);
    }
    if battle.active_actor() != Some(id) {
        return Err(RuleError::NotYourTurn);
    }
    if battle.needs_death_save() {
        return Ok(Some(Action::DeathSave));
    }
    if battle.finished() || actor.health().incapacitated() {
        return Ok(Some(Action::EndTurn));
    }
    if actor.health().prone
        && actor.speed_ft() > 0
        && actor.movement_remaining_ft() >= actor.speed_ft() / 2
    {
        return Ok(Some(Action::Stand));
    }
    let spend = if actor.budget().action_used {
        if actor.spec().kind == CreatureKind::Scout && actor.extra_attacks_available() {
            AttackSpend::Multiattack
        } else {
            AttackSpend::Action
        }
    } else {
        AttackSpend::Action
    };
    let can_attack = !actor.budget().action_used
        || actor.spec().kind == CreatureKind::Scout && actor.extra_attacks_available();
    let mut opponents: Vec<_> = battle
        .combatants()
        .iter()
        .filter(|other| other.side() != actor.side() && !other.health().incapacitated())
        .collect();
    opponents.sort_by_key(|other| (actor.cell().distance(other.cell()), other.id()));
    if let Some(&weapon) = actor
        .spec()
        .weapons
        .iter()
        .filter(|_| actor.spec().kind != CreatureKind::Hero || actor.spec().archery)
        .find(|weapon| weapon.ranged_weapon() && actor.weapon_count(**weapon) > 0)
    {
        if actor.shield_equipped() && !actor.budget().action_used && actor.arrows() > 0 {
            return Ok(Some(Action::DoffShield));
        }
        let usage = weapon_use(weapon, true);
        // Occupancy-aware reachable cells; seek cover and remain beyond melee proximity.
        if actor.movement_remaining_ft() > 0 && !actor.shield_equipped() {
            let (normal, _) = weapon.range().ok_or(RuleError::InvalidWeapon)?;
            let mut destinations = Vec::new();
            for cell in battle.reachable_cells(id)? {
                for target in &opponents {
                    let distance = cell.distance(target.cell());
                    if distance <= 5 || distance > u32::from(normal) {
                        continue;
                    }
                    if battle.attack_from(id, cell, target.id(), usage).is_err() {
                        continue;
                    }
                    let safety = opponents
                        .iter()
                        .map(|enemy| cell.distance(enemy.cell()))
                        .min()
                        .unwrap_or(0);
                    if safety <= 5 {
                        continue;
                    }
                    let protection = battle.map().cover_between(target.cell(), cell, &[])?;
                    let path =
                        battle.shortest_path(id, cell, Some(actor.movement_remaining_ft()))?;
                    let cost = if path.is_empty() {
                        0
                    } else {
                        battle.validate_path(id, &path)?
                    };
                    destinations.push((
                        std::cmp::Reverse(protection),
                        std::cmp::Reverse(safety),
                        cost,
                        target.id(),
                        cell,
                        path,
                    ));
                }
            }
            destinations.sort();
            if let Some((_, _, _, _, cell, path)) = destinations.first()
                && *cell != actor.cell()
            {
                return Ok(Some(Action::Move(path.clone())));
            }
        }
        let ranged_targets = battle.legal_targets(id, usage)?;
        if !can_attack && !ranged_targets.is_empty() {
            return Ok(Some(Action::EndTurn));
        }
        let melee_available = actor.spec().weapons.iter().any(|&weapon| {
            !weapon.ranged_weapon()
                && battle
                    .legal_targets(id, weapon_use(weapon, false))
                    .is_ok_and(|targets| !targets.is_empty())
        });
        for target in &opponents {
            if can_attack && !melee_available && ranged_targets.contains(&target.id()) {
                return Ok(Some(Action::Attack {
                    target: target.id(),
                    weapon: usage,
                    spend,
                }));
            }
        }
    }
    // Melee is the fallback when a ranged actor cannot obtain a legal firing position.
    for target in &opponents {
        if can_attack && actor.cell().distance(target.cell()) <= 5 {
            for &weapon in &actor.spec().weapons {
                if weapon.ranged_weapon() {
                    continue;
                }
                let usage = weapon_use(weapon, false);
                if battle.legal_targets(id, usage)?.contains(&target.id()) {
                    return Ok(Some(Action::Attack {
                        target: target.id(),
                        weapon: usage,
                        spend,
                    }));
                }
            }
        }
    }
    // Select nearest actually reachable attack square, prioritizing targets reachable now.
    let mut paths = Vec::new();
    for target in &opponents {
        let (cols, rows) = battle.map().dimensions();
        for col in
            target.cell().0.saturating_sub(1)..=target.cell().0.saturating_add(1).min(cols - 1)
        {
            for row in
                target.cell().1.saturating_sub(1)..=target.cell().1.saturating_add(1).min(rows - 1)
            {
                let cell = Cell(col, row);
                if cell == target.cell() {
                    continue;
                }
                if !actor.spec().weapons.iter().any(|&weapon| {
                    !weapon.ranged_weapon()
                        && battle
                            .attack_from(id, cell, target.id(), weapon_use(weapon, false))
                            .is_ok()
                }) {
                    continue;
                }
                let Ok(path) = battle.shortest_path(id, cell, None) else {
                    continue;
                };
                let full_cost = battle.path_cost(id, &path)?;
                paths.push((
                    full_cost > u32::from(actor.movement_remaining_ft()),
                    full_cost,
                    target.id(),
                    cell,
                    path,
                ));
            }
        }
    }
    paths.sort();
    // SRD 5.2.1 stat blocks (Kobold Warrior Dagger p302, Guard Spear p296) are "Melee or Ranged
    // Attack Roll": reach 5 ft or range 20/60 ft. A creature that cannot reach a melee square this
    // turn throws at normal range, from where it stands or after moving within its Speed.
    let melee_reachable = paths.first().is_some_and(|(too_far, ..)| !too_far);
    if can_attack
        && !melee_reachable
        && let Some(action) = thrown_attack(battle, id, &opponents, spend)?
    {
        return Ok(Some(action));
    }
    if let Some((_, full_cost, _, _, path)) = paths.first() {
        if *full_cost > u32::from(actor.movement_remaining_ft()) && !actor.budget().action_used {
            return Ok(Some(Action::Dash));
        }
        let mut prefix = Vec::new();
        for &step in path {
            let mut candidate = prefix.clone();
            candidate.push(step);
            // Allies can be intermediate squares; occupied prefixes cannot be committed endpoints.
            if battle.path_cost(id, &candidate)? > u32::from(actor.movement_remaining_ft()) {
                break;
            }
            prefix = candidate;
        }
        while !prefix.is_empty() && battle.validate_path(id, &prefix).is_err() {
            prefix.pop();
        }
        if !prefix.is_empty() {
            return Ok(Some(Action::Move(prefix)));
        }
    }
    if !actor.budget().action_used {
        Ok(Some(Action::Dodge))
    } else {
        Ok(Some(Action::EndTurn))
    }
}
/// The thrown weapon a stat-block creature carries: a melee weapon with a 20/60 style range
/// and at least one left. Heroes choose their own throws, so only enemies use this policy.
fn thrown_weapon(battle: &Battle, id: u32) -> Result<Option<Weapon>, RuleError> {
    let actor = battle.actor(id)?;
    if actor.spec().kind == CreatureKind::Hero {
        return Ok(None);
    }
    Ok(actor.spec().weapons.iter().copied().find(|weapon| {
        !weapon.ranged_weapon() && weapon.range().is_some() && actor.weapon_count(*weapon) > 0
    }))
}
/// Throw now if a target is within normal range with a clear line, otherwise move to the
/// cheapest reachable square from which one is, never beside a hostile creature that could
/// see the thrower (that would impose Disadvantage, SRD 5.2.1 p16).
fn thrown_attack(
    battle: &Battle,
    id: u32,
    opponents: &[&df_rules::tactical::Combatant],
    spend: AttackSpend,
) -> Result<Option<Action>, RuleError> {
    let Some(weapon) = thrown_weapon(battle, id)? else {
        return Ok(None);
    };
    let actor = battle.actor(id)?;
    let (normal, _) = weapon.range().ok_or(RuleError::InvalidWeapon)?;
    let usage = weapon_use(weapon, true);
    let safe = |cell: Cell| {
        opponents
            .iter()
            .all(|enemy| cell.distance(enemy.cell()) > 5)
    };
    let in_range = |cell: Cell, enemy: &df_rules::tactical::Combatant| {
        cell.distance(enemy.cell()) <= u32::from(normal)
            && battle.attack_from(id, cell, enemy.id(), usage).is_ok()
    };
    if safe(actor.cell())
        && let Some(target) = opponents.iter().find(|enemy| in_range(actor.cell(), enemy))
    {
        return Ok(Some(Action::Attack {
            target: target.id(),
            weapon: usage,
            spend,
        }));
    }
    if actor.movement_remaining_ft() == 0 {
        return Ok(None);
    }
    let mut destinations = Vec::new();
    for cell in battle.reachable_cells(id)? {
        if cell == actor.cell() || !safe(cell) {
            continue;
        }
        if opponents.iter().any(|enemy| in_range(cell, enemy)) {
            let path = battle.shortest_path(id, cell, Some(actor.movement_remaining_ft()))?;
            destinations.push((battle.path_cost(id, &path)?, cell, path));
        }
    }
    destinations.sort();
    Ok(destinations
        .into_iter()
        .next()
        .map(|(_, _, path)| Action::Move(path)))
}
fn weapon_use(weapon: Weapon, ranged: bool) -> AttackUse {
    AttackUse {
        weapon,
        ranged,
        hands: if weapon.two_handed() { 2 } else { 1 },
        finesse_dexterity: weapon.finesse(),
        knockout: false,
        savage_attacker: false,
        // Stat-block actions do not grant Fighter weapon masteries.
        use_mastery: false,
    }
}
