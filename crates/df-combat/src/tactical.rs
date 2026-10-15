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
    Action, AttackSpend, AttackUse, Battle, Cell, CreatureKind, DrawBinding, RuleError, Side,
    Weapon, resolve,
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
/// Current policy takes each legal opportunity attack with a natural or held melee weapon.
pub fn choose(
    battle: &Battle,
    actor: u32,
    context: TacticContext<'_>,
) -> Result<Option<Action>, TacticError> {
    choose_with_policy(battle, actor, context, false, false)
}

/// Policy revision that approaches standing heroes and follows the source-data downed-target rule.
pub fn choose_latest(
    battle: &Battle,
    actor: u32,
    context: TacticContext<'_>,
) -> Result<Option<Action>, TacticError> {
    choose_with_policy(battle, actor, context, false, true)
}

/// Preserve the historical policy for fights whose stored tactical source revision predates
/// opportunity-attack reactions.
pub fn choose_legacy(
    battle: &Battle,
    actor: u32,
    context: TacticContext<'_>,
) -> Result<Option<Action>, TacticError> {
    choose_with_policy(battle, actor, context, true, false)
}

fn choose_with_policy(
    battle: &Battle,
    actor: u32,
    context: TacticContext<'_>,
    legacy: bool,
    latest: bool,
) -> Result<Option<Action>, TacticError> {
    let proposed = if legacy {
        propose_legacy(battle, actor)
    } else if latest {
        propose_latest(battle, actor)
    } else {
        propose(battle, actor)
    }
    .map_err(TacticError::Rules)?;
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
    propose_with_shield_policy(battle, id, true)
}

fn propose_with_shield_policy(
    battle: &Battle,
    id: u32,
    doff_shield_for_ranged: bool,
) -> Result<Option<Action>, RuleError> {
    let actor = battle.actor(id)?;
    if battle.reaction_offers().first().copied() == Some(id) {
        let weapon = opportunity_weapon(&actor.spec().weapons, actor.held_weapons());
        return Ok(Some(Action::Opportunity {
            attack: Some(weapon_use(weapon, false)),
        }));
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
        if doff_shield_for_ranged
            && actor.shield_equipped()
            && !actor.budget().action_used
            && actor.arrows() > 0
        {
            return Ok(Some(Action::DoffShield));
        }
        let usage = weapon_use(weapon, true);
        // Occupancy-aware reachable cells; seek cover and remain beyond melee proximity.
        if actor.movement_remaining_ft() > 0
            && (!actor.shield_equipped()
                || actor.spec().kind == CreatureKind::Goblin
                    && opponents
                        .iter()
                        .any(|enemy| enemy.cell().distance(actor.cell()) <= 5))
        {
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

/// Current enemy action policy; `propose` remains the RUL-02 policy for stored fights.
pub fn propose_latest(battle: &Battle, id: u32) -> Result<Option<Action>, RuleError> {
    let actor = battle.actor(id)?;
    let action = propose_with_shield_policy(battle, id, actor.side() == Side::Heroes)?;
    if actor.spec().kind == CreatureKind::Goblin
        && !actor.budget().bonus_action_used
        && !actor.budget().disengaged
        && let Some(Action::Move(path)) = &action
        && battle.combatants().iter().any(|enemy| {
            enemy.side() != actor.side()
                && !enemy.health().incapacitated()
                && !enemy.budget().reaction_used
                && enemy.cell().distance(actor.cell()) <= 5
                && path.iter().any(|cell| enemy.cell().distance(*cell) > 5)
        })
    {
        // SRD 5.2.1 p. 290: Nimble Escape makes Disengage a Bonus Action.
        return Ok(Some(Action::NimbleDisengage));
    }
    if matches!(&action, Some(Action::Dash)) && actor.speed_ft() == 0 {
        return Ok(Some(if actor.side() == Side::Enemies {
            Action::EndTurn
        } else {
            Action::Dodge
        }));
    }
    if actor.side() != Side::Enemies {
        return Ok(action);
    }

    if !actor.budget().action_used
        && !matches!(&action, Some(Action::Attack { .. } | Action::Move(_)))
        && attacks_unconscious_target(actor.spec().kind)?
    {
        let mut dying_heroes: Vec<_> = battle
            .combatants()
            .iter()
            .filter(|target| {
                target.side() == Side::Heroes
                    && target.health().hit_points == 0
                    && target.health().unconscious
                    && !target.health().dead
                    && actor.cell().distance(target.cell()) <= 5
            })
            .collect();
        dying_heroes.sort_by_key(|target| (actor.cell().distance(target.cell()), target.id()));
        for target in dying_heroes {
            for &weapon in &actor.spec().weapons {
                if weapon.ranged_weapon() {
                    continue;
                }
                let use_ = weapon_use(weapon, false);
                if battle.legal_targets(id, use_)?.contains(&target.id()) {
                    return Ok(Some(Action::Attack {
                        target: target.id(),
                        weapon: use_,
                        spend: AttackSpend::Action,
                    }));
                }
            }
        }
    }

    if matches!(&action, Some(Action::Dodge | Action::EndTurn))
        && let Some(approach) = approach_target(battle, id)?
    {
        return Ok(Some(approach));
    }
    if actor.side() == Side::Enemies && matches!(&action, Some(Action::Dodge)) {
        // SRD 5.2.1 p. 14: an enemy spends its turn on a declared action. Do not
        // leave an enemy taking Dodge forever when no legal attack or purposeful
        // movement exists; EndTurn records the no-target outcome for the caller.
        return Ok(Some(Action::EndTurn));
    }
    Ok(action)
}

fn approach_target(battle: &Battle, id: u32) -> Result<Option<Action>, RuleError> {
    let actor = battle.actor(id)?;
    let mut targets: Vec<_> = battle
        .combatants()
        .iter()
        .filter(|target| target.side() != actor.side() && !target.health().incapacitated())
        .collect();
    targets.sort_by_key(|target| (actor.cell().distance(target.cell()), target.id()));
    if targets.is_empty() {
        return Ok(None);
    }

    let (cols, rows) = battle.map().dimensions();
    let mut paths = Vec::new();
    for target in targets {
        let current_distance = actor.cell().distance(target.cell());
        for col in 0..cols {
            for row in 0..rows {
                let cell = Cell(col, row);
                if cell == actor.cell() || cell.distance(target.cell()) >= current_distance {
                    continue;
                }
                let Ok(path) = battle.shortest_path(id, cell, None) else {
                    continue;
                };
                let cost = battle.path_cost(id, &path)?;
                paths.push((cell.distance(target.cell()), cost, target.id(), path));
            }
        }
    }
    paths.sort();
    let Some((_, full_cost, _, path)) = paths.first() else {
        return Ok(None);
    };
    if *full_cost > u32::from(actor.movement_remaining_ft()) && !actor.budget().action_used {
        return Ok(Some(Action::Dash));
    }

    let mut prefix = Vec::new();
    for &step in path {
        let mut candidate = prefix.clone();
        candidate.push(step);
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
    if !actor.budget().action_used {
        Ok(Some(Action::Dash))
    } else {
        Ok(None)
    }
}

fn attacks_unconscious_target(kind: CreatureKind) -> Result<bool, RuleError> {
    let id = kind.srd_id().ok_or(RuleError::InvalidCombatant)?;
    let source = df_rules::SRD_521_CREATURES
        .iter()
        .find(|creature| creature.id == id)
        .ok_or(RuleError::UnsupportedCreature)?;
    Ok(source.attacks_unconscious)
}

fn opportunity_weapon(weapons: &[Weapon], held: &[Weapon]) -> Weapon {
    weapons
        .iter()
        .copied()
        .find(|weapon| {
            !weapon.ranged_weapon()
                && (matches!(weapon, Weapon::Bite | Weapon::Slam) || held.contains(weapon))
        })
        .unwrap_or(Weapon::UnarmedStrike)
}
/// The previous policy deliberately declined optional opportunity reactions.
pub fn propose_legacy(battle: &Battle, id: u32) -> Result<Option<Action>, RuleError> {
    battle.actor(id)?;
    if battle.reaction_offers().first().copied() == Some(id) {
        return Ok(Some(Action::Opportunity { attack: None }));
    }
    propose(battle, id)
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
        hands: if weapon == Weapon::UnarmedStrike {
            0
        } else if weapon.two_handed() {
            2
        } else {
            1
        },
        finesse_dexterity: weapon.finesse(),
        knockout: false,
        savage_attacker: false,
        // Stat-block actions do not grant Fighter weapon masteries.
        use_mastery: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{attacks_unconscious_target, opportunity_weapon};
    use df_rules::tactical::{CreatureKind, Weapon};

    /// SRD 5.2.1 pp. 14, 176: a Reaction uses a weapon in hand or an Unarmed Strike.
    #[test]
    fn enemy_with_only_a_held_ranged_weapon_uses_an_unarmed_strike_for_its_reaction() {
        assert_eq!(
            opportunity_weapon(&[Weapon::Longbow], &[Weapon::Longbow]),
            Weapon::UnarmedStrike
        );
        assert_eq!(
            opportunity_weapon(&[Weapon::Javelin], &[]),
            Weapon::UnarmedStrike
        );
    }

    /// SRD 5.2.1 p. 18: damage at 0 HP is permitted; the table's targeting choice is creature data.
    #[test]
    fn downed_target_policy_uses_the_creature_record() {
        for kind in [
            CreatureKind::Zombie,
            CreatureKind::Wolf,
            CreatureKind::GiantRat,
            CreatureKind::GiantWeasel,
            CreatureKind::Mastiff,
        ] {
            assert!(attacks_unconscious_target(kind).unwrap());
        }
        for kind in [
            CreatureKind::Bandit,
            CreatureKind::Cultist,
            CreatureKind::Guard,
            CreatureKind::Goblin,
            CreatureKind::Kobold,
            CreatureKind::Skeleton,
            CreatureKind::Scout,
        ] {
            assert!(!attacks_unconscious_target(kind).unwrap());
        }
    }
}
