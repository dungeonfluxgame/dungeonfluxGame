use super::state::{Effect, PendingMove};
use super::{Battle, Cell, Cover, CreatureKind, DamageType, Mastery, RuleError, Weapon};
use df_model::checkpoint::{ActualDraw, ResolutionId, RuleReference, WindowId};
use df_types::OperationId;

/// Bind actual supplied records to their caller-owned durable pending operation/window.
#[derive(Clone, Copy)]
pub struct DrawBinding<'a> {
    pub operation: OperationId,
    pub resolution: ResolutionId,
    pub window: WindowId,
    pub source: &'a RuleReference,
    pub next_ordinal: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttackSpend {
    Action,
    LightBonus,
    Nick,
    Multiattack,
}
/// Explicit wielding choice: Finesse may select Dexterity, Versatile may use two hands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttackUse {
    pub weapon: Weapon,
    pub ranged: bool,
    pub hands: u8,
    pub finesse_dexterity: bool,
    pub knockout: bool,
    pub savage_attacker: bool,
    /// Opt in to the available unlocked property for this attack. False declines it;
    /// this never grants an unchosen mastery. Nick spend also requires this opt-in.
    pub use_mastery: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Move(Vec<Cell>),
    Stand,
    DropProne,
    Dash,
    Disengage,
    Dodge,
    NimbleDisengage,
    Attack {
        target: u32,
        weapon: AttackUse,
        spend: AttackSpend,
    },
    /// Only offered pending reactors may choose; None explicitly declines without spending.
    Opportunity {
        attack: Option<AttackUse>,
    },
    SecondWind,
    EndTurn,
    DeathSave,
    /// SRD Short Rest: this slice spends no Hit Dice; Fighters starting with at least 1 HP recharge one use.
    ShortRest {
        elapsed_seconds: u32,
    },
    RecoverAmmunition {
        elapsed_seconds: u32,
    },
    /// SRD 5.2.1 p18: a Stable creature that is not healed regains 1 Hit Point after 1d4 hours.
    /// The one recorded d4 is the number of hours that pass; the caller advances its clock.
    StableRecovery,
    /// Shield don/doff takes the Utilize action (SRD armor); changing gear is explicit.
    DoffShield,
    PickUpWeapon(Weapon),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttackReason {
    LongRange,
    HostileNearby,
    Heavy,
    ProneAttacker,
    ProneTarget,
    UnconsciousTarget,
    Dodge,
    PackTactics,
    /// Kobold Warrior in sunlight (SRD 5.2.1 p302): Disadvantage on attack rolls.
    SunlightSensitivity,
    Vex,
    Sap,
}
/// Reasons combine by presence, never by counting; both signs cancel even with unequal counts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttackSummary {
    pub actor: u32,
    pub target: u32,
    pub weapon: Weapon,
    pub ranged: bool,
    pub distance_ft: u32,
    pub cover: Cover,
    pub armor_class: u16,
    pub modifier: i16,
    pub advantage: Vec<AttackReason>,
    pub disadvantage: Vec<AttackReason>,
    /// -1 disadvantage, 0 normal, +1 advantage; the chosen face and both actual draws retained.
    pub roll_mode: i8,
    pub faces: Vec<u32>,
    pub selected: u32,
    pub hit: bool,
    pub critical: bool,
    pub damage_faces: Vec<u32>,
    pub damage_modifier: i16,
    pub damage: u16,
    pub damage_type: DamageType,
    pub necrotic_damage: u16,
    pub graze: bool,
    pub knocked_out: bool,
    pub killed: bool,
    pub mastery: Option<Mastery>,
}
/// Squares a mover actually entered in one resolution, in order, and what they cost in feet
/// (SRD p13-14: every square 5 ft, Difficult Terrain and crawling add 5 ft each). A move that
/// pauses for an Opportunity Attack offer reports only the squares entered before the pause.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoveSummary {
    pub actor: u32,
    pub from: Cell,
    pub path: Vec<Cell>,
    pub cost_ft: u16,
    /// Movement left this turn after the squares above (Speed, Dash and Slow already applied).
    pub remaining_ft: u16,
}
/// How a Death Saving Throw changed the dying hero (SRD p17-18).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeathSaveResult {
    Success,
    Failure,
    /// Natural 1: two failures.
    CriticalFailure,
    /// Third success: stable at 0 Hit Points.
    Stabilised,
    /// Third failure: dead.
    Died,
    /// Natural 20: back to 1 Hit Point.
    Revived,
}
/// One Death Saving Throw: the d20 face and the running counts after it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeathSaveSummary {
    pub actor: u32,
    pub die: u32,
    pub successes: u8,
    pub failures: u8,
    pub result: DeathSaveResult,
}
/// A Stable hero back on 1 Hit Point after the hours the d4 gave.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoverySummary {
    pub actor: u32,
    pub hours: u32,
}
/// A staged successor and exact consumed records. Caller commits state and draws together.
/// Any rejection leaves caller state and supplied immutable draws unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resolution {
    pub battle: Battle,
    pub draws: Vec<ActualDraw>,
    pub attacks: Vec<AttackSummary>,
    pub moves: Vec<MoveSummary>,
    pub death_saves: Vec<DeathSaveSummary>,
    pub recoveries: Vec<RecoverySummary>,
}

pub(crate) struct Draws<'a> {
    binding: DrawBinding<'a>,
    records: &'a [ActualDraw],
    consumed: usize,
}
impl<'a> Draws<'a> {
    pub(crate) fn new(
        binding: DrawBinding<'a>,
        records: &'a [ActualDraw],
    ) -> Result<Self, RuleError> {
        if records.len() > 128 {
            return Err(RuleError::Capacity);
        }
        for (index, record) in records.iter().enumerate() {
            let ordinal = binding
                .next_ordinal
                .checked_add(u32::try_from(index).map_err(|_| RuleError::Capacity)?)
                .ok_or(RuleError::StaleDraw)?;
            if record.operation != binding.operation
                || record.resolution != binding.resolution
                || record.window != binding.window
                || record.source != *binding.source
                || record.ordinal != ordinal
            {
                return Err(RuleError::StaleDraw);
            }
            if record.sides == 0 || record.value == 0 || record.value > record.sides {
                return Err(RuleError::InvalidDraw);
            }
        }
        Ok(Self {
            binding,
            records,
            consumed: 0,
        })
    }
    pub(crate) fn take(&mut self, sides: u32) -> Result<u32, RuleError> {
        let record = self
            .records
            .get(self.consumed)
            .ok_or(RuleError::MissingDraw { sides })?;
        if record.sides != sides {
            return Err(RuleError::InvalidDraw);
        }
        self.binding
            .next_ordinal
            .checked_add(u32::try_from(self.consumed + 1).map_err(|_| RuleError::Capacity)?)
            .ok_or(RuleError::StaleDraw)?;
        self.consumed += 1;
        Ok(record.value)
    }
    pub(crate) fn finish(self) -> Result<Vec<ActualDraw>, RuleError> {
        if self.consumed != self.records.len() {
            return Err(RuleError::UnusedDraws);
        }
        Ok(self.records.to_vec())
    }
}

/// Resolve one currently legal input. Moves pause at optional Opportunity Attack windows,
/// before the provoking square is left. No RNG, I/O, timer or mutation of input occurs.
pub fn resolve(
    battle: &Battle,
    actor: u32,
    action: &Action,
    binding: DrawBinding<'_>,
    records: &[ActualDraw],
) -> Result<Resolution, RuleError> {
    let mut draws = Draws::new(binding, records)?;
    let mut next = battle.clone();
    let mut attacks = Vec::new();
    let mut moves = Vec::new();
    let mut death_saves = Vec::new();
    let mut recoveries = Vec::new();
    if let Action::Opportunity { attack } = action {
        react(
            &mut next,
            actor,
            *attack,
            &mut draws,
            &mut attacks,
            &mut moves,
        )?;
    } else {
        if next.pending.is_some() {
            return Err(RuleError::PendingReaction);
        }
        match action {
            Action::ShortRest { elapsed_seconds } => {
                if *elapsed_seconds < 3600 {
                    return Err(RuleError::RecoveryUnavailable);
                }
                let combatant = next.actor_mut(actor)?;
                if combatant.health.dead || combatant.health.hit_points == 0 {
                    return Err(RuleError::RecoveryUnavailable);
                }
                if combatant.spec.kind == CreatureKind::Hero {
                    combatant.spec.second_wind_uses =
                        combatant.spec.second_wind_uses.saturating_add(1).min(2);
                }
            }
            Action::RecoverAmmunition { elapsed_seconds } => {
                if !next.finished() || *elapsed_seconds < 60 {
                    return Err(RuleError::RecoveryUnavailable);
                }
                let combatant = next.actor_mut(actor)?;
                if combatant.health.incapacitated() {
                    return Err(RuleError::Incapacitated);
                }
                if combatant.recovered {
                    return Err(RuleError::RecoveryUnavailable);
                }
                combatant.spec.arrows += combatant.ammunition_used / 2;
                combatant.spec.bolts += combatant.bolts_used / 2;
                combatant.recovered = true;
            }
            Action::StableRecovery => {
                let combatant = next.actor_mut(actor)?;
                if combatant.spec.kind != CreatureKind::Hero
                    || combatant.health.hit_points != 0
                    || combatant.health.dead
                    || !combatant.health.stable
                {
                    return Err(RuleError::RecoveryUnavailable);
                }
                let hours = draws.take(4)?;
                combatant.health.hit_points = 1;
                combatant.health.stable = false;
                combatant.health.unconscious = false;
                combatant.health.death_successes = 0;
                combatant.health.death_failures = 0;
                recoveries.push(RecoverySummary { actor, hours });
            }
            Action::DeathSave => {
                if next.active_actor() != Some(actor) || !next.needs_death_save {
                    return Err(RuleError::NotYourTurn);
                }
                death_saves.push(death_save(&mut next, actor, &mut draws)?);
                next.needs_death_save = false;
            }
            _ => {
                if next.finished() && *action != Action::EndTurn {
                    return Err(RuleError::Finished);
                }
                if next.active_actor() != Some(actor) {
                    return Err(RuleError::NotYourTurn);
                }
                if next.needs_death_save {
                    return Err(RuleError::NeedDeathSave);
                }
                if next.actor(actor)?.health.incapacitated() && *action != Action::EndTurn {
                    return Err(RuleError::Incapacitated);
                }
                match action {
                    Action::Move(path) => {
                        next.validate_path(actor, path)?;
                        next.pending = Some(PendingMove {
                            actor,
                            path: path.clone(),
                            next: 0,
                            eligible: vec![],
                        });
                        let from = next.actor(actor)?.cell();
                        let mut steps = Vec::new();
                        continue_move(&mut next, false, &mut steps)?;
                        moves.extend(move_summary(&next, actor, from, steps)?);
                    }
                    Action::Stand => {
                        let combatant = next.actor_mut(actor)?;
                        let cost = combatant.speed_ft() / 2;
                        if !combatant.health.prone || combatant.speed_ft() == 0 {
                            return Err(RuleError::InvalidPath);
                        }
                        if cost > combatant.movement_remaining_ft() {
                            return Err(RuleError::MovementSpent);
                        }
                        combatant.budget.movement_spent_ft += cost;
                        combatant.health.prone = false;
                    }
                    Action::DropProne => {
                        let combatant = next.actor_mut(actor)?;
                        if combatant.speed_ft() == 0 {
                            return Err(RuleError::MovementSpent);
                        }
                        combatant.health.prone = true;
                    }
                    Action::Dash | Action::Disengage | Action::Dodge | Action::DoffShield => {
                        let combatant = next.actor_mut(actor)?;
                        if combatant.budget.action_used {
                            return Err(RuleError::ActionSpent);
                        }
                        if *action == Action::DoffShield && !combatant.shield_equipped {
                            return Err(RuleError::InvalidHands);
                        }
                        combatant.budget.action_used = true;
                        match action {
                            Action::Dash => combatant.budget.dash = true,
                            Action::Disengage => combatant.budget.disengaged = true,
                            Action::Dodge => combatant.budget.dodging = true,
                            Action::DoffShield => {
                                combatant.shield_equipped = false;
                                combatant.spec.armor_class -= 2;
                            }
                            _ => {}
                        }
                    }
                    Action::NimbleDisengage => {
                        let combatant = next.actor_mut(actor)?;
                        if combatant.spec.kind != CreatureKind::Goblin {
                            return Err(RuleError::InvalidCombatant);
                        }
                        if combatant.budget.bonus_action_used {
                            return Err(RuleError::BonusActionSpent);
                        }
                        combatant.budget.bonus_action_used = true;
                        combatant.budget.disengaged = true;
                    }
                    Action::Attack {
                        target,
                        weapon,
                        spend,
                    } => {
                        if *spend == AttackSpend::Nick && !weapon.use_mastery {
                            return Err(RuleError::InvalidExtraAttack);
                        }
                        admit_attack(&next, actor, *target, *weapon)?;
                        spend_attack(&mut next, actor, weapon.weapon, *spend)?;
                        attacks.push(attack(
                            &mut next, actor, *target, *weapon, *spend, &mut draws,
                        )?);
                    }
                    Action::SecondWind => {
                        let combatant = next.actor_mut(actor)?;
                        if combatant.spec.kind != CreatureKind::Hero
                            || combatant.spec.second_wind_uses == 0
                        {
                            return Err(RuleError::InvalidCombatant);
                        }
                        if combatant.budget.bonus_action_used {
                            return Err(RuleError::BonusActionSpent);
                        }
                        let die = draws.take(10)?;
                        let (hp, uses) = crate::local_journey::heal_second_wind(
                            &crate::local_journey::SecondWindRules::srd_521(),
                            die,
                            u32::from(combatant.health.hit_points),
                            u32::from(combatant.health.maximum),
                            u32::from(combatant.spec.second_wind_uses),
                        )
                        .map_err(|_| RuleError::InvalidCombatant)?;
                        combatant.health.hit_points = hp as u16;
                        combatant.spec.second_wind_uses = uses as u8;
                        combatant.budget.bonus_action_used = true;
                    }
                    Action::EndTurn => end_turn(&mut next, actor)?,
                    Action::PickUpWeapon(weapon) => {
                        let combatant = next.actor_mut(actor)?;
                        let index = combatant
                            .dropped
                            .iter()
                            .position(|(item, cell)| item == weapon && *cell == combatant.cell())
                            .ok_or(RuleError::WeaponMissing)?;
                        if combatant.budget.object_interaction_used {
                            if combatant.budget.action_used {
                                return Err(RuleError::ActionSpent);
                            }
                            combatant.budget.action_used = true;
                        }
                        combatant.budget.object_interaction_used = true;
                        if !combatant.held.contains(weapon)
                            && (combatant.held.len() == 2
                                || (weapon.two_handed() && combatant.shield_equipped))
                        {
                            return Err(RuleError::InvalidHands);
                        }
                        combatant.dropped.remove(index);
                        let quantity = combatant
                            .weapon_counts
                            .get_mut(weapon)
                            .ok_or(RuleError::WeaponMissing)?;
                        *quantity = quantity.checked_add(1).ok_or(RuleError::Capacity)?;
                        if weapon.two_handed() {
                            combatant.held.clear();
                        }
                        if !combatant.held.contains(weapon) {
                            combatant.held.push(*weapon);
                        }
                    }
                    Action::Opportunity { .. }
                    | Action::DeathSave
                    | Action::ShortRest { .. }
                    | Action::StableRecovery
                    | Action::RecoverAmmunition { .. } => return Err(RuleError::InvalidReaction),
                }
            }
        }
    }
    Ok(Resolution {
        battle: next,
        draws: draws.finish()?,
        attacks,
        moves,
        death_saves,
        recoveries,
    })
}
fn move_summary(
    battle: &Battle,
    actor: u32,
    from: Cell,
    steps: Vec<(Cell, u16)>,
) -> Result<Option<MoveSummary>, RuleError> {
    if steps.is_empty() {
        return Ok(None);
    }
    Ok(Some(MoveSummary {
        actor,
        from,
        cost_ft: steps.iter().map(|(_, cost)| *cost).sum(),
        path: steps.into_iter().map(|(cell, _)| cell).collect(),
        remaining_ft: battle.actor(actor)?.movement_remaining_ft(),
    }))
}

/// Read-only authoritative range/cover/weapon legality, before any budget or draw spends.
impl Battle {
    pub fn legal_targets(&self, actor: u32, weapon: AttackUse) -> Result<Vec<u32>, RuleError> {
        self.actor(actor)?;
        Ok(self
            .actors
            .iter()
            .filter(|target| admit_attack(self, actor, target.id(), weapon).is_ok())
            .map(|target| target.id())
            .collect())
    }
    /// Check exact weapon/target legality at a proposed free map cell. This does not admit
    /// the path, turn or resource spend; callers separately validate the route and action.
    /// A detached position change reuses the same source attack owner, including cover,
    /// occupancy, hands, range and ammunition, without duplicating rules in tactical policy.
    pub fn attack_from(
        &self,
        actor: u32,
        origin: Cell,
        target: u32,
        weapon: AttackUse,
    ) -> Result<(), RuleError> {
        if !self.map.contains(origin) {
            return Err(RuleError::OutOfBounds);
        }
        if self.map.is_blocked(origin) {
            return Err(RuleError::Blocked);
        }
        if self
            .actors
            .iter()
            .any(|other| other.id() != actor && other.cell() == origin && !other.health.dead)
        {
            return Err(RuleError::Occupied);
        }
        let mut projected = self.clone();
        projected.actor_mut(actor)?.spec.cell = origin;
        admit_attack(&projected, actor, target, weapon)
    }
    /// Range, cover and advantage reasons for an admitted attack, without spending or drawing.
    pub fn attack_reasons(
        &self,
        actor: u32,
        target: u32,
        usage: AttackUse,
    ) -> Result<(Vec<AttackReason>, Vec<AttackReason>), RuleError> {
        admit_attack(self, actor, target, usage)?;
        let (advantage, disadvantage, _, _) = modifiers(self, actor, target, usage)?;
        Ok((advantage, disadvantage))
    }
    pub fn attack_cover(&self, actor: u32, target: u32) -> Result<Cover, RuleError> {
        self.map.cover_between(
            self.actor(actor)?.cell(),
            self.actor(target)?.cell(),
            &self.cells(),
        )
    }
    /// Dodge saving throw advantage and cover bonus for future save handlers; Unconscious
    /// automatically fails Dexterity saves. This function does not invent an actual save.
    pub fn dexterity_save_adjustments(
        &self,
        actor: u32,
        origin: Cell,
    ) -> Result<(i16, bool, bool), RuleError> {
        let combatant = self.actor(actor)?;
        let cover = self
            .map
            .cover_between(origin, combatant.cell(), &self.cells())?;
        Ok((
            cover.bonus(),
            combatant.budget.dodging
                && combatant.speed_ft() != 0
                && !combatant.health.incapacitated(),
            combatant.health.unconscious,
        ))
    }
}
fn admit_attack(battle: &Battle, id: u32, target: u32, usage: AttackUse) -> Result<(), RuleError> {
    let actor = battle.actor(id)?;
    let target = battle.actor(target)?;
    if actor.health.incapacitated() {
        return Err(RuleError::Incapacitated);
    }
    if actor.id() == target.id() || actor.side() == target.side() || target.health.dead {
        return Err(RuleError::InvalidTarget);
    }
    if usage.weapon != Weapon::UnarmedStrike && actor.weapon_count(usage.weapon) == 0 {
        return Err(RuleError::WeaponMissing);
    }
    if (usage.hands == 0
        && !matches!(
            usage.weapon,
            Weapon::UnarmedStrike | Weapon::Bite | Weapon::Slam
        ))
        || usage.hands > 2
        || (actor.shield_equipped && usage.hands > 1)
        || (usage.weapon.two_handed() && usage.hands != 2)
        || (usage.hands == 2 && !usage.weapon.two_handed() && usage.weapon != Weapon::Spear)
    {
        return Err(RuleError::InvalidHands);
    }
    if usage.use_mastery && !actor.spec.masteries.contains(&usage.weapon) {
        return Err(RuleError::InvalidWeapon);
    }
    if usage.finesse_dexterity && !usage.weapon.finesse() {
        return Err(RuleError::InvalidWeapon);
    }
    if usage.knockout && usage.ranged {
        return Err(RuleError::InvalidTarget);
    }
    if usage.savage_attacker
        && (usage.weapon == Weapon::UnarmedStrike
            || !actor.spec.savage_attacker
            || actor.savage_used)
    {
        return Err(RuleError::InvalidExtraAttack);
    }
    if usage.ranged {
        let (_, long) = usage.weapon.range().ok_or(RuleError::InvalidWeapon)?;
        if actor.cell().distance(target.cell()) > u32::from(long) {
            return Err(RuleError::OutOfRange);
        }
        if usage.weapon.ammunition()
            && (if usage.weapon == Weapon::LightCrossbow {
                actor.spec.bolts
            } else {
                actor.spec.arrows
            }) == 0
        {
            return Err(RuleError::AmmunitionMissing);
        }
    } else if usage.weapon.ranged_weapon() {
        return Err(RuleError::InvalidWeapon);
    } else if actor.cell().distance(target.cell()) > 5 {
        return Err(RuleError::OutOfRange);
    }
    if battle.attack_cover(id, target.id())? == Cover::Total {
        return Err(RuleError::TotalCover);
    }
    Ok(())
}
fn spend_attack(
    battle: &mut Battle,
    id: u32,
    weapon: Weapon,
    spend: AttackSpend,
) -> Result<(), RuleError> {
    let actor = battle.actor_mut(id)?;
    match spend {
        AttackSpend::Action => {
            if actor.budget.action_used {
                return Err(RuleError::ActionSpent);
            }
            actor.budget.action_used = true;
            actor.extra_attacks = if actor.spec.kind == CreatureKind::Scout
                && matches!(weapon, Weapon::Shortsword | Weapon::Longbow)
            {
                1
            } else {
                0
            };
            if weapon.light() {
                actor.light_first = Some(weapon);
            }
        }
        AttackSpend::Multiattack => {
            if actor.spec.kind != CreatureKind::Scout
                || actor.extra_attacks == 0
                || !matches!(weapon, Weapon::Shortsword | Weapon::Longbow)
            {
                return Err(RuleError::InvalidExtraAttack);
            }
            actor.extra_attacks -= 1;
        }
        AttackSpend::LightBonus | AttackSpend::Nick => {
            if actor.spec.kind != CreatureKind::Hero
                || actor.light_used
                || !weapon.light()
                || actor.light_first.is_none_or(|first| first == weapon)
            {
                return Err(RuleError::InvalidExtraAttack);
            }
            if spend == AttackSpend::Nick {
                if weapon.mastery() != Some(Mastery::Nick)
                    || !actor.spec.masteries.contains(&weapon)
                {
                    return Err(RuleError::InvalidExtraAttack);
                }
            } else {
                if actor.budget.bonus_action_used {
                    return Err(RuleError::BonusActionSpent);
                }
                actor.budget.bonus_action_used = true;
            }
            actor.light_used = true;
        }
    }
    Ok(())
}
fn modifiers(
    battle: &Battle,
    id: u32,
    target: u32,
    usage: AttackUse,
) -> Result<(Vec<AttackReason>, Vec<AttackReason>, i16, i16), RuleError> {
    let actor = battle.actor(id)?;
    let target = battle.actor(target)?;
    let mut advantage = Vec::new();
    let mut disadvantage = Vec::new();
    let ability = if (usage.ranged && usage.weapon.ranged_weapon()) || usage.finesse_dexterity {
        1
    } else {
        0
    };
    let damage_modifier =
        crate::local_journey::ability_modifier(actor.spec.abilities[ability]) as i16;
    let attack_modifier = if actor.spec.kind == CreatureKind::Hero {
        actor
            .spec
            .hero_attack_modifiers(usage.weapon, usage.ranged, usage.finesse_dexterity)?
            .0
    } else if usage.weapon == Weapon::UnarmedStrike {
        damage_modifier
            + 2
            + if usage.weapon.ranged_weapon() && actor.spec.archery {
                2
            } else {
                0
            }
    } else {
        match actor.spec.kind {
            CreatureKind::Goblin
            | CreatureKind::Kobold
            | CreatureKind::Wolf
            | CreatureKind::Scout => 4,
            CreatureKind::Skeleton | CreatureKind::GiantRat | CreatureKind::GiantWeasel => 5,
            _ => 3,
        }
    };
    if actor.health.prone {
        disadvantage.push(AttackReason::ProneAttacker);
    }
    if target.health.prone {
        if actor.cell().distance(target.cell()) <= 5 {
            advantage.push(AttackReason::ProneTarget);
        } else {
            disadvantage.push(AttackReason::ProneTarget);
        }
    }
    if target.health.unconscious {
        advantage.push(AttackReason::UnconsciousTarget);
    }
    if target.budget.dodging
        && target.speed_ft() != 0
        && !target.health.incapacitated()
        && battle.can_see(target.cell(), actor.cell())
    {
        disadvantage.push(AttackReason::Dodge);
    }
    if !actor.sap.is_empty() {
        disadvantage.push(AttackReason::Sap);
    }
    if usage.weapon.heavy()
        && actor.spec.abilities[if usage.weapon.ranged_weapon() { 1 } else { 0 }] < 13
    {
        disadvantage.push(AttackReason::Heavy);
    }
    if usage.ranged {
        if usage
            .weapon
            .range()
            .is_some_and(|(normal, _)| actor.cell().distance(target.cell()) > u32::from(normal))
        {
            disadvantage.push(AttackReason::LongRange);
        }
        if battle.actors.iter().any(|other| {
            other.side() != actor.side()
                && !other.health.incapacitated()
                && other.cell().distance(actor.cell()) <= 5
                && battle.can_see(other.cell(), actor.cell())
        }) {
            disadvantage.push(AttackReason::HostileNearby);
        }
    }
    if matches!(
        actor.spec.kind,
        CreatureKind::Wolf | CreatureKind::Kobold | CreatureKind::GiantRat
    ) && battle.actors.iter().any(|ally| {
        ally.id() != id
            && ally.side() == actor.side()
            && !ally.health.incapacitated()
            && ally.cell().distance(target.cell()) <= 5
    }) {
        advantage.push(AttackReason::PackTactics);
    }
    if actor.spec.kind == CreatureKind::Kobold && battle.map.lighting() == super::Lighting::Sunlight
    {
        disadvantage.push(AttackReason::SunlightSensitivity);
    }
    if actor.vex.iter().any(|(marked, _)| *marked == target.id()) {
        advantage.push(AttackReason::Vex);
    }
    Ok((advantage, disadvantage, attack_modifier, damage_modifier))
}
fn attack(
    battle: &mut Battle,
    id: u32,
    target: u32,
    usage: AttackUse,
    spend: AttackSpend,
    draws: &mut Draws<'_>,
) -> Result<AttackSummary, RuleError> {
    let (advantage, disadvantage, modifier, ability_modifier) =
        modifiers(battle, id, target, usage)?;
    let roll_mode = match (advantage.is_empty(), disadvantage.is_empty()) {
        (false, true) => 1,
        (true, false) => -1,
        _ => 0,
    };
    let mut faces = vec![draws.take(20)?];
    let first = faces[0];
    let selected = if roll_mode == 0 {
        first
    } else {
        let second = draws.take(20)?;
        faces.push(second);
        if roll_mode > 0 {
            first.max(second)
        } else {
            first.min(second)
        }
    };
    let cover = battle.attack_cover(id, target)?;
    let armor_class = battle.actor(target)?.spec.armor_class + cover.bonus() as u16;
    let hit = selected != 1 && (selected == 20 || selected as i16 + modifier >= armor_class as i16);
    let critical = hit
        && (selected == 20
            || (battle.actor(target)?.health.unconscious
                && battle
                    .actor(id)?
                    .cell()
                    .distance(battle.actor(target)?.cell())
                    <= 5));
    let kind = battle.actor(id)?.spec.kind;
    let mastery = if usage.use_mastery && battle.actor(id)?.spec.masteries.contains(&usage.weapon) {
        usage.weapon.mastery()
    } else {
        None
    };
    let thrown_landing = if usage.ranged && !usage.weapon.ammunition() {
        Some(battle.actor(target)?.cell())
    } else {
        None
    };
    let graze = !hit && mastery == Some(Mastery::Graze);
    let multiplier = if critical { 2 } else { 1 };
    let (mut count, mut sides) = usage
        .weapon
        .dice(kind == CreatureKind::Hero && usage.hands == 2 && !usage.ranged);
    if usage.weapon == Weapon::Bite
        && matches!(kind, CreatureKind::GiantRat | CreatureKind::GiantWeasel)
    {
        count = 1;
        sides = 4;
    }
    // Monster weapons use their stat block modifier, including finesse-like sickle attacks.
    let damage_modifier = if kind == CreatureKind::Hero || usage.weapon == Weapon::UnarmedStrike {
        ability_modifier
    } else {
        match kind {
            CreatureKind::Goblin
            | CreatureKind::Kobold
            | CreatureKind::Wolf
            | CreatureKind::Scout => 2,
            CreatureKind::Skeleton | CreatureKind::GiantRat | CreatureKind::GiantWeasel => 3,
            _ => 1,
        }
    };
    let damage_modifier = if matches!(spend, AttackSpend::LightBonus | AttackSpend::Nick) {
        damage_modifier.min(0)
    } else {
        damage_modifier
    };
    let mut damage = if graze {
        ability_modifier.max(0) as u16
    } else {
        0
    };
    let mut necrotic_damage = 0;
    let mut damage_faces = Vec::new();
    if hit {
        let mut weapon_damage = if usage.weapon == Weapon::UnarmedStrike {
            1_u16
        } else {
            0_u16
        };
        for _ in 0..count * multiplier {
            let face = draws.take(sides)?;
            damage_faces.push(face);
            weapon_damage += face as u16;
        }
        if usage.savage_attacker {
            let mut second = 0_u16;
            for _ in 0..count * multiplier {
                let face = draws.take(sides)?;
                damage_faces.push(face);
                second += face as u16;
            }
            weapon_damage = weapon_damage.max(second);
        }
        damage = (weapon_damage as i16 + damage_modifier).max(0) as u16;
        if kind == CreatureKind::Goblin
            && roll_mode > 0
            && matches!(usage.weapon, Weapon::Scimitar | Weapon::Shortbow)
        {
            for _ in 0..multiplier {
                let face = draws.take(4)?;
                damage_faces.push(face);
                damage += face as u16;
            }
        }
        if kind == CreatureKind::Cultist && usage.weapon == Weapon::Sickle {
            necrotic_damage = 1;
        }
    }
    if battle.actor(target)?.spec.kind == CreatureKind::Skeleton
        && usage.weapon.damage_type() == DamageType::Bludgeoning
    {
        damage *= 2;
    }
    {
        let actor = battle.actor_mut(id)?;
        if !matches!(
            usage.weapon,
            Weapon::UnarmedStrike | Weapon::Bite | Weapon::Slam
        ) && usage.hands == 2
        {
            actor.held = vec![usage.weapon];
        } else if !matches!(
            usage.weapon,
            Weapon::UnarmedStrike | Weapon::Bite | Weapon::Slam
        ) {
            actor.held.retain(|weapon| !weapon.two_handed());
            if !actor.held.contains(&usage.weapon) {
                actor.held.push(usage.weapon);
            }
            if actor.held.len() > 2 {
                actor.held.remove(0);
            }
        }
        actor.sap.clear();
        // Vex applies to the next attack against its marked creature, not the next arbitrary target.
        actor.vex.retain(|(marked, _)| *marked != target);
        if hit && usage.savage_attacker {
            actor.savage_used = true;
        }
        if usage.ranged && usage.weapon.ammunition() {
            if usage.weapon == Weapon::LightCrossbow {
                actor.spec.bolts -= 1;
                actor.bolts_used += 1;
            } else {
                actor.spec.arrows -= 1;
                actor.ammunition_used += 1;
            }
        } else if usage.ranged {
            let quantity = actor
                .weapon_counts
                .get_mut(&usage.weapon)
                .ok_or(RuleError::WeaponMissing)?;
            *quantity -= 1;
            actor.dropped.push((
                usage.weapon,
                thrown_landing.ok_or(RuleError::WeaponMissing)?,
            ));
            if *quantity == 0 {
                actor.held.retain(|held| *held != usage.weapon);
            }
        }
    }
    let effect = Effect {
        origin: id,
        expires_turn: battle
            .actor(id)?
            .turn_count
            .checked_add(1)
            .ok_or(RuleError::Capacity)?,
    };
    if hit && damage + necrotic_damage > 0 {
        match mastery {
            Some(Mastery::Slow) => {
                let target = battle.actor_mut(target)?;
                if !target.slow.contains(&effect) {
                    target.slow.push(effect);
                }
            }
            Some(Mastery::Vex) => {
                let actor = battle.actor_mut(id)?;
                actor.vex.retain(|(marked, _)| *marked != target);
                actor.vex.push((target, effect.expires_turn));
            }
            _ => {}
        }
    }
    if hit && mastery == Some(Mastery::Sap) {
        let target = battle.actor_mut(target)?;
        if !target.sap.contains(&effect) {
            target.sap.push(effect);
        }
    }
    if hit
        && usage.weapon == Weapon::Bite
        && matches!(kind, CreatureKind::Wolf | CreatureKind::Mastiff)
        && matches!(
            battle.actor(target)?.spec.size,
            super::Size::Small | super::Size::Medium
        )
    {
        battle.actor_mut(target)?.health.prone = true;
    }
    apply_damage(
        battle,
        target,
        damage + necrotic_damage,
        critical,
        usage.knockout,
        draws,
    )?;
    let target_actor = battle.actor(target)?;
    Ok(AttackSummary {
        actor: id,
        target,
        weapon: usage.weapon,
        ranged: usage.ranged,
        distance_ft: battle.actor(id)?.cell().distance(target_actor.cell()),
        cover,
        armor_class,
        modifier,
        advantage,
        disadvantage,
        roll_mode,
        faces,
        selected,
        hit,
        critical,
        damage_faces,
        damage_modifier,
        damage,
        damage_type: usage.weapon.damage_type(),
        necrotic_damage,
        graze,
        knocked_out: target_actor.health.knocked_out,
        killed: target_actor.health.dead,
        mastery,
    })
}
fn apply_damage(
    battle: &mut Battle,
    target: u32,
    damage: u16,
    critical: bool,
    knockout: bool,
    draws: &mut Draws<'_>,
) -> Result<(), RuleError> {
    if damage == 0 {
        return Ok(());
    }
    let actor = battle.actor_mut(target)?;
    let hp = actor.health.hit_points;
    if hp > 0 && damage < hp {
        actor.health.hit_points -= damage;
        return Ok(());
    }
    if hp > 0 && knockout {
        actor.health.hit_points = 1;
        actor.health.knocked_out = true;
        actor.health.unconscious = true;
        actor.health.prone = true;
        drop_held(actor)?;
        return Ok(());
    }
    if hp > 0 && actor.spec.kind == CreatureKind::Zombie && !critical {
        let die = draws.take(20)?;
        if die as u16 + 3 >= 5 + damage {
            actor.health.hit_points = 1;
            return Ok(());
        }
    }
    actor.health.hit_points = 0;
    actor.health.unconscious = true;
    actor.health.prone = true;
    drop_held(actor)?;
    if actor.spec.kind != CreatureKind::Hero
        || (hp > 0 && damage - hp >= actor.health.maximum)
        || (hp == 0 && damage >= actor.health.maximum)
    {
        actor.health.dead = true;
    } else if hp == 0 {
        actor.health.stable = false;
        actor.health.death_failures = actor
            .health
            .death_failures
            .saturating_add(if critical { 2 } else { 1 })
            .min(3);
        if actor.health.death_failures >= 3 {
            actor.health.dead = true;
        }
    }
    Ok(())
}
fn death_save(
    battle: &mut Battle,
    id: u32,
    draws: &mut Draws<'_>,
) -> Result<DeathSaveSummary, RuleError> {
    let actor = battle.actor_mut(id)?;
    if actor.health.hit_points != 0
        || actor.health.dead
        || actor.health.stable
        || actor.spec.kind != CreatureKind::Hero
    {
        return Err(RuleError::InvalidTarget);
    }
    let die = draws.take(20)?;
    let mut result = if die == 20 {
        actor.health.hit_points = 1;
        actor.health.unconscious = false;
        actor.health.death_failures = 0;
        actor.health.death_successes = 0;
        DeathSaveResult::Revived
    } else if die < 10 {
        actor.health.death_failures = actor
            .health
            .death_failures
            .saturating_add(if die == 1 { 2 } else { 1 })
            .min(3);
        actor.health.dead = actor.health.death_failures >= 3;
        if actor.health.dead {
            DeathSaveResult::Died
        } else if die == 1 {
            DeathSaveResult::CriticalFailure
        } else {
            DeathSaveResult::Failure
        }
    } else {
        actor.health.death_successes += 1;
        DeathSaveResult::Success
    };
    let (successes, failures) = (actor.health.death_successes, actor.health.death_failures);
    if result == DeathSaveResult::Success && actor.health.death_successes == 3 {
        actor.health.stable = true;
        actor.health.death_successes = 0;
        actor.health.death_failures = 0;
        result = DeathSaveResult::Stabilised;
    }
    Ok(DeathSaveSummary {
        actor: id,
        die,
        successes,
        failures,
        result,
    })
}
fn end_turn(battle: &mut Battle, id: u32) -> Result<(), RuleError> {
    let actor = battle.actor_mut(id)?;
    actor.vex.retain(|(_, expiry)| actor.turn_count < *expiry);
    // Reset once-per-turn Savage Attacker for every actor, including reactions on others' turns.
    for actor in &mut battle.actors {
        actor.savage_used = false;
    }
    battle.active += 1;
    if battle.active == battle.order.len() {
        battle.active = 0;
        battle.round = battle.round.checked_add(1).ok_or(RuleError::Capacity)?;
    }
    let next = battle.active_actor().ok_or(RuleError::UnknownActor)?;
    let actor = battle.actor_mut(next)?;
    actor.turn_count = actor.turn_count.checked_add(1).ok_or(RuleError::Capacity)?;
    let turn = actor.turn_count;
    actor.budget = super::Budget {
        action_used: false,
        bonus_action_used: false,
        reaction_used: false,
        object_interaction_used: false,
        movement_spent_ft: 0,
        dash: false,
        disengaged: false,
        dodging: false,
    };
    actor.light_first = None;
    actor.light_used = false;
    actor.extra_attacks = 0;
    for actor in &mut battle.actors {
        actor
            .slow
            .retain(|effect| effect.origin != next || effect.expires_turn > turn);
        actor
            .sap
            .retain(|effect| effect.origin != next || effect.expires_turn > turn);
    }
    let actor = battle.actor(next)?;
    battle.needs_death_save =
        actor.health.hit_points == 0 && !actor.health.dead && !actor.health.stable;
    Ok(())
}
fn continue_move(
    battle: &mut Battle,
    approved_step: bool,
    steps: &mut Vec<(Cell, u16)>,
) -> Result<(), RuleError> {
    let Some(mut movement) = battle.pending.take() else {
        return Ok(());
    };
    let mut skip_trigger = approved_step;
    while let Some(&to) = movement.path.get(movement.next) {
        let actor = battle.actor(movement.actor)?;
        if actor.health.incapacitated() || battle.finished() {
            return Ok(());
        }
        if !skip_trigger && !actor.budget.disengaged {
            for &id in &battle.order {
                let other = battle.actor(id)?;
                if other.side() != actor.side()
                    && !other.health.incapacitated()
                    && !other.budget.reaction_used
                    && other.cell().distance(actor.cell()) <= 5
                    && other.cell().distance(to) > 5
                    && battle.can_see(other.cell(), actor.cell())
                {
                    movement.eligible.push(id);
                }
            }
            if !movement.eligible.is_empty() {
                battle.pending = Some(movement);
                return Ok(());
            }
        }
        let cost = battle.step_cost(actor, actor.cell(), to)?;
        if cost > actor.movement_remaining_ft() {
            return Ok(());
        } // a legal Slow hit can shorten remaining movement
        let actor = battle.actor_mut(movement.actor)?;
        actor.budget.movement_spent_ft += cost;
        actor.spec.cell = to;
        steps.push((to, cost));
        movement.next += 1;
        skip_trigger = false;
    }
    Ok(())
}
fn react(
    battle: &mut Battle,
    reactor: u32,
    usage: Option<AttackUse>,
    draws: &mut Draws<'_>,
    attacks: &mut Vec<AttackSummary>,
    moves: &mut Vec<MoveSummary>,
) -> Result<(), RuleError> {
    let pending = battle
        .pending
        .as_ref()
        .ok_or(RuleError::NoPendingReaction)?;
    if pending.eligible.first().copied() != Some(reactor) {
        return Err(RuleError::InvalidReaction);
    }
    let mover = pending.actor;
    if let Some(usage) = usage {
        if usage.ranged {
            return Err(RuleError::InvalidReaction);
        }
        if usage.weapon != Weapon::UnarmedStrike
            && !battle.actor(reactor)?.held.contains(&usage.weapon)
            && !matches!(usage.weapon, Weapon::Bite | Weapon::Slam)
        {
            return Err(RuleError::InvalidWeapon);
        }
        admit_attack(battle, reactor, mover, usage)?;
        if battle.actor(reactor)?.budget.reaction_used {
            return Err(RuleError::ReactionSpent);
        }
        battle.actor_mut(reactor)?.budget.reaction_used = true;
        attacks.push(attack(
            battle,
            reactor,
            mover,
            usage,
            AttackSpend::Action,
            draws,
        )?);
    }
    if battle.actor(mover)?.health.incapacitated() || battle.finished() {
        battle.pending = None;
        return Ok(());
    }
    let pending = battle
        .pending
        .as_mut()
        .ok_or(RuleError::NoPendingReaction)?;
    pending.eligible.remove(0);
    if pending.eligible.is_empty() {
        let from = battle.actor(mover)?.cell();
        let mut steps = Vec::new();
        continue_move(battle, true, &mut steps)?;
        moves.extend(move_summary(battle, mover, from, steps)?);
    }
    Ok(())
}

fn drop_held(actor: &mut super::Combatant) -> Result<(), RuleError> {
    for weapon in actor.held.drain(..) {
        let quantity = actor
            .weapon_counts
            .get_mut(&weapon)
            .ok_or(RuleError::WeaponMissing)?;
        *quantity = quantity.checked_sub(1).ok_or(RuleError::WeaponMissing)?;
        actor.dropped.push((weapon, actor.spec.cell));
    }
    Ok(())
}
