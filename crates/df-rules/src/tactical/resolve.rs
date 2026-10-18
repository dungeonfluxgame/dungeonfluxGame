use super::state::CombatantSpec;
use super::state::{Effect, HelpEffect, PendingMove};
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
pub enum HelpPurpose {
    Attack { ally: u32, target: u32 },
    Stabilize { target: u32 },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StabilizationOutcome {
    Medicine {
        die: u32,
        modifier: i16,
        total: i16,
        success: bool,
    },
    HealersKit {
        uses_remaining: u8,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StabilizationSummary {
    pub helper: u32,
    pub target: u32,
    pub outcome: StabilizationOutcome,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HelpSummary {
    pub helper: u32,
    pub ally: u32,
    pub target: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum D20Test {
    Attack,
    DeathSave,
    AbilityCheck,
    Initiative,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct D20Reroll {
    pub actor: u32,
    pub test: D20Test,
    pub original: u32,
    pub replacement: u32,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Move(Vec<Cell>),
    Stand,
    DropProne,
    Dash,
    Disengage,
    Dodge,
    Help {
        purpose: HelpPurpose,
    },
    UtilizeHealersKit {
        target: u32,
    },
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
    /// Fighter 2: one additional non-Magic action, once per Short or Long Rest (p.48).
    ActionSurge,
    /// Full Long Rest admission stays with the caller; eight uninterrupted hours (p.183).
    LongRest {
        elapsed_seconds: u32,
    },
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
    /// SRD 5.2.1 p16: an attacker that cannot see its target attacks with Disadvantage.
    UnseenTarget,
    /// SRD 5.2.1 p16: attacks against an unseen attacker have Advantage.
    UnseenAttacker,
    Help,
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
    /// -1 disadvantage, 0 normal, +1 advantage; faces are post-reroll, and Resolution.rerolls records Halfling Luck draws.
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
    pub helps: Vec<HelpSummary>,
    pub stabilizations: Vec<StabilizationSummary>,
    pub rerolls: Vec<D20Reroll>,
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

pub(crate) fn take_d20(
    spec: &CombatantSpec,
    actor: u32,
    test: D20Test,
    draws: &mut Draws<'_>,
    rerolls: &mut Vec<D20Reroll>,
) -> Result<u32, RuleError> {
    let original = draws.take(20)?;
    // SRD 5.2.1 p. 86: "When you roll a 1 on the d20 of a D20 Test, you can reroll the die, and you must use the new roll."
    if original == 1
        && spec
            .species
            .is_some_and(|species| species.combat_traits().reroll_natural_ones)
    {
        let replacement = draws.take(20)?;
        rerolls.push(D20Reroll {
            actor,
            test,
            original,
            replacement,
        });
        Ok(replacement)
    } else {
        Ok(original)
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
    let mut helps = Vec::new();
    let mut stabilizations = Vec::new();
    let mut rerolls = Vec::new();
    if let Action::Opportunity { attack } = action {
        react(
            &mut next,
            actor,
            *attack,
            &mut draws,
            &mut attacks,
            &mut moves,
            &mut rerolls,
        )?;
    } else {
        if next.pending.is_some() {
            return Err(RuleError::PendingReaction);
        }
        match action {
            Action::LongRest { elapsed_seconds } => {
                if *elapsed_seconds < 28_800 || !next.finished() {
                    return Err(RuleError::RecoveryUnavailable);
                }
                let combatant = next.actor_mut(actor)?;
                if combatant.health.dead || combatant.health.hit_points == 0 {
                    return Err(RuleError::RecoveryUnavailable);
                }
                let progression = combatant
                    .spec
                    .fighter_level()
                    .ok_or(RuleError::InvalidCombatant)?;
                combatant.health.hit_points = combatant.health.maximum;
                combatant.spec.second_wind_uses = progression.second_wind_maximum();
                combatant.action_surge_spent = false;
                combatant.additional_actions = 0;
                combatant.additional_dash = false;
                combatant.extra_attacks = 0;
                combatant.feature_movement_ft = 0;
            }
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
                        combatant.spec.second_wind_uses.saturating_add(1).min(
                            combatant
                                .spec
                                .fighter_level()
                                .ok_or(RuleError::InvalidCombatant)?
                                .second_wind_maximum(),
                        );
                    combatant.action_surge_spent = false;
                    combatant.additional_actions = 0;
                    combatant.additional_dash = false;
                    combatant.extra_attacks = 0;
                    combatant.feature_movement_ft = 0;
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
                death_saves.push(death_save(&mut next, actor, &mut draws, &mut rerolls)?);
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
                if !matches!(action, Action::Move(_)) {
                    next.actor_mut(actor)?.feature_movement_ft = 0;
                }
                match action {
                    Action::ActionSurge => {
                        let combatant = next.actor_mut(actor)?;
                        if !combatant.action_surge_available() {
                            return Err(RuleError::RecoveryUnavailable);
                        }
                        combatant.action_surge_spent = true;
                        combatant.additional_actions = 1;
                    }
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
                    Action::Help { purpose } => match purpose {
                        HelpPurpose::Attack { ally, target } => {
                            helps.push(help_attack(&mut next, actor, *ally, *target)?);
                        }
                        HelpPurpose::Stabilize { target } => {
                            stabilizations.push(stabilize_with_medicine(
                                &mut next,
                                actor,
                                *target,
                                &mut draws,
                                &mut rerolls,
                            )?);
                        }
                    },
                    Action::UtilizeHealersKit { target } => {
                        stabilizations.push(stabilize_with_healers_kit(&mut next, actor, *target)?);
                    }
                    Action::Dash | Action::Disengage | Action::Dodge | Action::DoffShield => {
                        let combatant = next.actor_mut(actor)?;
                        if *action == Action::DoffShield && !combatant.shield_equipped {
                            return Err(RuleError::InvalidHands);
                        }
                        combatant.spend_action()?;
                        match action {
                            Action::Dash => {
                                if combatant.budget.dash {
                                    combatant.additional_dash = true;
                                }
                                combatant.budget.dash = true;
                            }
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
                            &mut next,
                            actor,
                            *target,
                            *weapon,
                            *spend,
                            &mut draws,
                            &mut rerolls,
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
                            &crate::local_journey::SecondWindRules {
                                flat_bonus: combatant
                                    .spec
                                    .fighter_level()
                                    .ok_or(RuleError::InvalidBuild)?
                                    .level(),
                                uses_max: combatant
                                    .spec
                                    .fighter_level()
                                    .ok_or(RuleError::InvalidBuild)?
                                    .second_wind_maximum(),
                                ..crate::local_journey::SecondWindRules::srd_521()
                            },
                            die,
                            u32::from(combatant.health.hit_points),
                            u32::from(combatant.health.maximum),
                            u32::from(combatant.spec.second_wind_uses),
                        )
                        .map_err(|_| RuleError::InvalidCombatant)?;
                        combatant.health.hit_points = hp as u16;
                        combatant.spec.second_wind_uses = uses as u8;
                        combatant.budget.bonus_action_used = true;
                        if combatant
                            .spec
                            .fighter_level()
                            .is_some_and(|progression| progression.level() >= 5)
                        {
                            combatant.feature_movement_ft = combatant.speed_ft() / 2;
                        }
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
                            combatant.spend_action()?;
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
                    | Action::LongRest { .. }
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
        helps,
        stabilizations,
        rerolls,
    })
}

fn help_attack(
    battle: &mut Battle,
    helper: u32,
    ally: u32,
    target: u32,
) -> Result<HelpSummary, RuleError> {
    let helper_actor = battle.actor(helper)?;
    let ally_actor = battle.actor(ally)?;
    let target_actor = battle.actor(target)?;
    if helper == ally
        || helper_actor.side() != ally_actor.side()
        || helper_actor.side() == target_actor.side()
        || ally_actor.health().incapacitated()
        || target_actor.health().incapacitated()
    {
        return Err(RuleError::InvalidTarget);
    }
    // SRD 5.2.1 p. 182: "The creature you aid has Advantage on the next attack roll it makes against a creature within 5 feet of you."
    if helper_actor.cell().distance(target_actor.cell()) > 5 {
        return Err(RuleError::OutOfRange);
    }
    if helper_actor.budget().action_used && helper_actor.additional_actions == 0 {
        return Err(RuleError::ActionSpent);
    }
    if battle.helped_attacks.len() >= 64 {
        return Err(RuleError::Capacity);
    }
    let helper_turn_count = helper_actor.turn_count;
    battle.actor_mut(helper)?.spend_action()?;
    battle.helped_attacks.push(HelpEffect {
        helper,
        helper_turn_count,
        ally,
        target,
    });
    Ok(HelpSummary {
        helper,
        ally,
        target,
    })
}

fn validate_stabilization_target(
    battle: &Battle,
    helper: u32,
    target: u32,
) -> Result<(), RuleError> {
    let helper_actor = battle.actor(helper)?;
    let target_actor = battle.actor(target)?;
    if target_actor.health().hit_points != 0
        || target_actor.health().dead
        || target_actor.health().stable
        || !target_actor.health().unconscious
    {
        return Err(RuleError::InvalidTarget);
    }
    // The session's disclosed adjudication keeps stabilization actions within 5 feet.
    if helper_actor.cell().distance(target_actor.cell()) > 5 {
        return Err(RuleError::OutOfRange);
    }
    if helper_actor.budget().action_used && helper_actor.additional_actions == 0 {
        return Err(RuleError::ActionSpent);
    }
    Ok(())
}

fn stabilize_with_medicine(
    battle: &mut Battle,
    helper: u32,
    target: u32,
    draws: &mut Draws<'_>,
    rerolls: &mut Vec<D20Reroll>,
) -> Result<StabilizationSummary, RuleError> {
    // SRD 5.2.1 p. 18: "You can take the Help action to try to stabilize a creature
    // with 0 Hit Points, which requires a successful DC 10 Wisdom (Medicine) check."
    validate_stabilization_target(battle, helper, target)?;
    let helper_spec = battle.actor(helper)?.spec();
    let wisdom = helper_spec.abilities[4];
    let modifier = crate::local_journey::ability_modifier(wisdom) as i16;
    let die = take_d20(helper_spec, helper, D20Test::AbilityCheck, draws, rerolls)?;
    let total = die as i16 + modifier;
    let success = total >= 10;
    battle.actor_mut(helper)?.spend_action()?;
    if success {
        battle.actor_mut(target)?.health.stable = true;
    }
    Ok(StabilizationSummary {
        helper,
        target,
        outcome: StabilizationOutcome::Medicine {
            die,
            modifier,
            total,
            success,
        },
    })
}

fn stabilize_with_healers_kit(
    battle: &mut Battle,
    helper: u32,
    target: u32,
) -> Result<StabilizationSummary, RuleError> {
    // SRD 5.2.1 p. 97: "As a Utilize action, you can expend one of its uses to
    // stabilize an Unconscious creature that has 0 Hit Points without needing to
    // make a Wisdom (Medicine) check."
    validate_stabilization_target(battle, helper, target)?;
    let uses_remaining = battle
        .actor(helper)?
        .spec
        .healers_kit_uses
        .checked_sub(1)
        .ok_or(RuleError::HealersKitMissing)?;
    battle.actor_mut(helper)?.spend_action()?;
    battle.actor_mut(helper)?.spec.healers_kit_uses = uses_remaining;
    battle.actor_mut(target)?.health.stable = true;
    Ok(StabilizationSummary {
        helper,
        target,
        outcome: StabilizationOutcome::HealersKit { uses_remaining },
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
        let attacker = self.actor(actor)?;
        let target = self.actor(target)?;
        let creatures = self.cover_cells_for(target);
        self.map.cover_between_for_size(
            attacker.cell(),
            target.cell(),
            &creatures,
            target.spec.size,
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
        let creatures = self.cover_cells_for(combatant);
        let cover = self.map.cover_between_for_size(
            origin,
            combatant.cell(),
            &creatures,
            combatant.spec.size,
        )?;
        Ok((
            cover.bonus(),
            combatant.budget.dodging
                && combatant.speed_ft() != 0
                && !combatant.health.incapacitated(),
            combatant.health.unconscious,
        ))
    }

    fn cover_cells_for(&self, target: &super::Combatant) -> Vec<Cell> {
        self.actors
            .iter()
            .filter(|creature| {
                // SRD 5.2.1 p. 15: a creature offers Half Cover only when it covers
                // at least half the target. A prone body lies below this grid's
                // representative Small and Medium target heights.
                !creature.health.dead
                    && !creature.health.prone
                    && u32::from(super::map::target_height_dm(creature.spec.size)) * 2
                        >= u32::from(super::map::target_height_dm(target.spec.size))
            })
            .map(super::Combatant::cell)
            .collect()
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
    // SRD 5.2.1 p. 290 lists "Shield, Shortbow" in Goblin Warrior Gear and
    // "Shortbow. Ranged Attack Roll: +4, range 80/320 ft." as an Action.
    let listed_creature_attack = actor.spec.kind != CreatureKind::Hero
        && usage.weapon.two_handed()
        && actor.spec.weapons.contains(&usage.weapon);
    if (usage.hands == 0
        && !matches!(
            usage.weapon,
            Weapon::UnarmedStrike | Weapon::Bite | Weapon::Slam
        ))
        || usage.hands > 2
        || (actor.shield_equipped && usage.hands > 1 && !listed_creature_attack)
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
    let cover = battle.attack_cover(id, target.id())?;
    if !usage.ranged && !battle.map.step_clear(actor.cell(), target.cell()) && cover != Cover::Total
    {
        return Err(RuleError::Blocked);
    }
    if cover == Cover::Total {
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
            actor.spend_action()?;
            actor.extra_attacks += u8::from(
                actor
                    .spec
                    .fighter_level()
                    .is_some_and(|progression| progression.level() >= 5)
                    || (actor.spec.kind == CreatureKind::Scout
                        && matches!(weapon, Weapon::Shortsword | Weapon::Longbow)),
            );
            if weapon.light() {
                actor.light_first = Some(weapon);
            }
        }
        AttackSpend::Multiattack => {
            if actor.extra_attacks == 0
                || !(actor
                    .spec
                    .fighter_level()
                    .is_some_and(|progression| progression.level() >= 5)
                    || (actor.spec.kind == CreatureKind::Scout
                        && matches!(weapon, Weapon::Shortsword | Weapon::Longbow)))
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
    // Vision and Light, SRD 5.2.1 p16: darkness and observer-specific
    // darkvision determine whether an attack is seen; the attack remains legal.
    if !battle.can_see_actor(actor, target) {
        disadvantage.push(AttackReason::UnseenTarget);
    }
    if !battle.can_see_actor(target, actor) {
        advantage.push(AttackReason::UnseenAttacker);
    }
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
        && battle.can_see_actor(target, actor)
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
                && battle.can_see_actor(other, actor)
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
    if battle
        .helped_attacks
        .iter()
        .any(|effect| effect.ally == id && effect.target == target.id())
    {
        advantage.push(AttackReason::Help);
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
    rerolls: &mut Vec<D20Reroll>,
) -> Result<AttackSummary, RuleError> {
    let (advantage, disadvantage, modifier, ability_modifier) =
        modifiers(battle, id, target, usage)?;
    let roll_mode = match (advantage.is_empty(), disadvantage.is_empty()) {
        (false, true) => 1,
        (true, false) => -1,
        _ => 0,
    };
    let actor_spec = battle.actor(id)?.spec();
    let mut faces = vec![take_d20(actor_spec, id, D20Test::Attack, draws, rerolls)?];
    let first = faces[0];
    let selected = if roll_mode == 0 {
        first
    } else {
        let second = take_d20(actor_spec, id, D20Test::Attack, draws, rerolls)?;
        faces.push(second);
        if roll_mode > 0 {
            first.max(second)
        } else {
            first.min(second)
        }
    };
    battle
        .helped_attacks
        .retain(|effect| !(effect.ally == id && effect.target == target));
    let cover = battle.attack_cover(id, target)?;
    let armor_class = battle.actor(target)?.spec.armor_class + cover.bonus() as u16;
    let critical_roll = selected
        >= if battle
            .actor(id)?
            .spec
            .fighter_level()
            .is_some_and(|progression| progression.champion())
        {
            19
        } else {
            20
        };
    let hit = selected != 1 && (critical_roll || selected as i16 + modifier >= armor_class as i16);
    let critical = hit
        && (critical_roll
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
        // SRD 5.2.1 pp. 16-17, 278: preserve the Ritual Sickle's Slashing
        // and Necrotic amounts separately through target defenses.
        if kind == CreatureKind::Cultist && usage.weapon == Weapon::Sickle {
            necrotic_damage = 1;
        }
    }
    [damage, necrotic_damage] = apply_typed_defenses(
        [
            (usage.weapon.damage_type(), damage),
            (DamageType::Necrotic, necrotic_damage),
        ],
        source_damage_defenses(battle.actor(target)?.spec()),
    );
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
                // SRD 5.2.1 p. 90: Slow can "reduce its Speed by 10 feet until the start of your
                // next turn"; p. 91 lists Slow as the Javelin's mastery.
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
    if critical
        && battle
            .actor(id)?
            .spec
            .fighter_level()
            .is_some_and(|progression| progression.champion())
    {
        let actor = battle.actor_mut(id)?;
        actor.feature_movement_ft = actor.speed_ft() / 2;
    }
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
struct DamageDefenses<'a> {
    resistances: &'a [DamageType],
    immunities: &'a [DamageType],
    vulnerabilities: &'a [DamageType],
}

fn source_damage_defenses(spec: &CombatantSpec) -> DamageDefenses<'static> {
    // SRD 5.2.1 p. 84: "You have Resistance to Poison damage." (Dwarven Resilience.)
    if spec.kind == CreatureKind::Hero
        && spec.species == Some(crate::local_journey::HeroSpecies::Dwarf)
    {
        return DamageDefenses {
            resistances: &[DamageType::Poison],
            immunities: &[],
            vulnerabilities: &[],
        };
    }
    // SRD 5.2.1 pp. 325, 343: Skeletons are vulnerable to Bludgeoning;
    // Skeletons and Zombies are immune to Poison. No admitted target resists
    // or is immune to the Ritual Sickle's Slashing or Necrotic damage.
    match spec.kind {
        CreatureKind::Skeleton => DamageDefenses {
            resistances: &[],
            immunities: &[DamageType::Poison],
            vulnerabilities: &[DamageType::Bludgeoning],
        },
        CreatureKind::Zombie => DamageDefenses {
            resistances: &[],
            immunities: &[DamageType::Poison],
            vulnerabilities: &[],
        },
        _ => DamageDefenses {
            resistances: &[],
            immunities: &[],
            vulnerabilities: &[],
        },
    }
}

fn apply_typed_defenses(parts: [(DamageType, u16); 2], defenses: DamageDefenses<'_>) -> [u16; 2] {
    parts.map(|(kind, amount)| {
        if defenses.immunities.contains(&kind) {
            0
        } else {
            let amount = if defenses.vulnerabilities.contains(&kind) {
                amount * 2
            } else {
                amount
            };
            if defenses.resistances.contains(&kind) {
                amount / 2
            } else {
                amount
            }
        }
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
    // SRD 5.2.1 p. 86: "When you are reduced to 0 Hit Points but not killed outright, you can drop to 1 Hit Point instead."
    if hp > 0
        && actor.spec.kind == CreatureKind::Hero
        && actor.spec.relentless_endurance_available
        && damage - hp < actor.health.maximum
    {
        actor.health.hit_points = 1;
        actor.spec.relentless_endurance_available = false;
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
    rerolls: &mut Vec<D20Reroll>,
) -> Result<DeathSaveSummary, RuleError> {
    let spec = battle.actor(id)?.spec();
    let health = battle.actor(id)?.health();
    if health.hit_points != 0 || health.dead || health.stable || spec.kind != CreatureKind::Hero {
        return Err(RuleError::InvalidTarget);
    }
    let die = take_d20(spec, id, D20Test::DeathSave, draws, rerolls)?;
    let actor = battle.actor_mut(id)?;
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
    actor.extra_attacks = 0;
    actor.additional_actions = 0;
    actor.feature_movement_ft = 0;
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
    actor.additional_actions = 0;
    actor.feature_movement_ft = 0;
    actor.additional_dash = false;
    battle
        .helped_attacks
        .retain(|effect| effect.helper != next || effect.helper_turn_count >= turn);
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
        let cost = battle.step_cost(actor, actor.cell(), to)?;
        if !skip_trigger && !actor.budget.disengaged && actor.feature_movement_ft < cost {
            for &id in &battle.order {
                let other = battle.actor(id)?;
                if other.side() != actor.side()
                    && !other.health.incapacitated()
                    && !other.budget.reaction_used
                    && other.cell().distance(actor.cell()) <= 5
                    && other.cell().distance(to) > 5
                    && battle.map.step_clear(other.cell(), actor.cell())
                    && battle.can_see_actor(other, actor)
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
        let free = cost.min(actor.feature_movement_ft);
        actor.feature_movement_ft -= free;
        actor.budget.movement_spent_ft += cost - free;
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
    rerolls: &mut Vec<D20Reroll>,
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
            rerolls,
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

#[cfg(test)]
mod typed_damage_tests {
    use super::*;

    fn defenses_for(id: &str) -> DamageDefenses<'static> {
        let spec = CombatantSpec::creature(1, id, Cell(1, 1), 0).unwrap();
        source_damage_defenses(&spec)
    }

    #[test]
    fn srd_521_p16_p278_ritual_sickle_parts_meet_defenses_independently() {
        let parts = [(DamageType::Slashing, 5), (DamageType::Necrotic, 1)];
        assert_eq!(
            apply_typed_defenses(
                parts,
                DamageDefenses {
                    resistances: &[DamageType::Slashing],
                    immunities: &[],
                    vulnerabilities: &[],
                }
            ),
            [2, 1]
        );
        assert_eq!(
            apply_typed_defenses(
                parts,
                DamageDefenses {
                    resistances: &[],
                    immunities: &[DamageType::Necrotic],
                    vulnerabilities: &[],
                }
            ),
            [5, 0]
        );
        assert_eq!(
            apply_typed_defenses(parts, defenses_for("skeleton")),
            [5, 1]
        );
        assert_eq!(
            apply_typed_defenses(
                parts,
                DamageDefenses {
                    resistances: &[DamageType::Slashing],
                    immunities: &[],
                    vulnerabilities: &[DamageType::Slashing],
                }
            ),
            [5, 1]
        );
    }

    #[test]
    fn srd_521_p325_p343_catalog_defenses_cover_bludgeoning_and_poison() {
        let parts = [(DamageType::Bludgeoning, 3), (DamageType::Poison, 1)];
        assert_eq!(
            apply_typed_defenses(parts, defenses_for("skeleton")),
            [6, 0]
        );
        assert_eq!(apply_typed_defenses(parts, defenses_for("zombie")), [3, 0]);
    }

    #[test]
    fn srd_521_p84_dwarven_resilience_halves_poison_damage() {
        let mut dwarf =
            crate::tactical::CombatantSpec::creature(1, "giant-weasel", Cell(1, 1), 0).unwrap();
        dwarf.kind = CreatureKind::Hero;
        dwarf.species = Some(crate::local_journey::HeroSpecies::Dwarf);

        assert_eq!(
            apply_typed_defenses(
                [(DamageType::Poison, 5), (DamageType::Necrotic, 1)],
                source_damage_defenses(&dwarf),
            ),
            [2, 1]
        );
    }
}
