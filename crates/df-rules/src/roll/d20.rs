//! D20 Tests: ability checks, skill checks, saving throws, attack rolls, initiative, Death Saving
//! Throws and passive scores (SRD 5.2.1 pages 6 to 9, 13, 17 to 18, 22, 49 and 186).
//!
//! It owns the arithmetic and the natural-roll rules of each test type. The caller supplies every
//! face, every modifier and the target; it never rolls, never judges whether a test is called for
//! and never decides Advantage or Disadvantage. A natural 1 or 20 decides only an attack roll and a
//! Death Saving Throw, never an ability check or a saving throw (page 6, page 7).
//!
//! Contested rolls: the SRD has no opposed-check rule. Its two opposed-looking cases are plain
//! tests against a target number, so they use `TargetLabel::EscapeDc` and `StealthTotal`.
use super::abilities::{Ability, Skill};
use super::dice::{DiePlace, DieRole, DieSize, RolledDie};
use super::result::{
    AttackKind, Modifier, ModifierLabel, Natural, RollOutcome, RollResult, RollSubject, Target,
    TargetLabel,
};
use super::{Faces, RollError};
use crate::influence::RollMode;
use serde::{Deserialize, Serialize};

/// The lowest ability modifier of a score (page 5, score 1).
pub const MINIMUM_ABILITY_MODIFIER: i8 = -5;
/// The highest ability modifier of a score (page 5, score 30).
pub const MAXIMUM_ABILITY_MODIFIER: i8 = 10;
/// The lowest Proficiency Bonus (page 8, level or CR up to 4).
pub const MINIMUM_PROFICIENCY_BONUS: u8 = 2;
/// The highest Proficiency Bonus (page 8, level or CR 29 to 30).
pub const MAXIMUM_PROFICIENCY_BONUS: u8 = 9;
/// A Death Saving Throw of this or more succeeds (page 17).
pub const DEATH_SAVE_DC: i16 = 10;
/// The d20 face that always hits and is a Critical Hit without Improved Critical (page 7).
pub const NATURAL_TWENTY: u8 = 20;
/// The d20 face that always misses on an attack roll (page 7).
pub const NATURAL_ONE: u8 = 1;
/// What Advantage or Disadvantage adds to or takes from a passive score (page 186).
pub const PASSIVE_MODE_STEP: i16 = 5;
/// The base of a passive score (page 22).
pub const PASSIVE_BASE: i16 = 10;

/// How the Proficiency Bonus applies to a test (pages 8 and 182).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Proficiency {
    None,
    Proficient {
        bonus: u8,
    },
    /// The bonus is doubled for the test (Expertise, page 182).
    Expertise {
        bonus: u8,
    },
}

/// The test a D20 roll is, with what only that test needs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum D20Test {
    AbilityCheck(Ability),
    SkillCheck(Skill),
    SavingThrow(Ability),
    /// `critical_from` is the lowest d20 face that is a Critical Hit: 20, or 19 with Champion's
    /// Improved Critical, or 18 with Superior Critical (page 49).
    Attack {
        ability: Ability,
        kind: AttackKind,
        critical_from: u8,
    },
    Initiative,
    DeathSave,
}

/// Heroic Inspiration spent to reroll one die of the roll (pages 8 and 183). Which die is the
/// player's choice; the replacement face is the last face supplied.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeroicInspiration {
    /// The place of the replaced die among the dice of the roll, from 0.
    pub replaces_die: u8,
}

/// One D20 Test to resolve.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct D20Request {
    pub test: D20Test,
    pub mode: RollMode,
    pub ability_modifier: i8,
    pub proficiency: Proficiency,
    /// A Difficulty Class or other target for a check or save, an Armor Class for an attack; none
    /// for initiative and a Death Saving Throw, whose target is fixed.
    pub target: Option<Target>,
    pub heroic_inspiration: Option<HeroicInspiration>,
}

/// One passive Wisdom (Perception) score (pages 22 and 186).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PassivePerceptionRequest {
    pub ability_modifier: i8,
    pub proficiency: Proficiency,
    pub mode: RollMode,
}

/// The DC of an Unarmed Strike grapple or shove and of escaping that grapple: 8 plus the Strength
/// modifier and the Proficiency Bonus (page 190). Use it as the target of the target's save, or as
/// `TargetLabel::EscapeDc` for the escape check.
pub const fn unarmed_strike_dc(strength_modifier: i8, proficiency_bonus: u8) -> i16 {
    8 + strength_modifier as i16 + proficiency_bonus as i16
}

fn dice_count(request: &D20Request) -> usize {
    match request.mode {
        RollMode::Normal => 1,
        RollMode::Advantage | RollMode::Disadvantage => 2,
    }
}

fn validate_modifiers(ability_modifier: i8, proficiency: Proficiency) -> Result<(), RollError> {
    if !(MINIMUM_ABILITY_MODIFIER..=MAXIMUM_ABILITY_MODIFIER).contains(&ability_modifier) {
        return Err(RollError::InvalidAbilityModifier);
    }
    match proficiency {
        Proficiency::None => Ok(()),
        Proficiency::Proficient { bonus } | Proficiency::Expertise { bonus } => {
            if (MINIMUM_PROFICIENCY_BONUS..=MAXIMUM_PROFICIENCY_BONUS).contains(&bonus) {
                Ok(())
            } else {
                Err(RollError::InvalidProficiencyBonus)
            }
        }
    }
}

fn validate_target(request: &D20Request) -> Result<(), RollError> {
    let target = match (request.test, request.target) {
        (D20Test::Initiative | D20Test::DeathSave, None) => return Ok(()),
        (D20Test::Initiative | D20Test::DeathSave, Some(_)) => {
            return Err(RollError::UnexpectedTarget);
        }
        (_, None) => return Err(RollError::MissingTarget),
        (_, Some(target)) => target,
    };
    let is_armor_class = target.label == TargetLabel::ArmorClass;
    if is_armor_class != matches!(request.test, D20Test::Attack { .. }) {
        return Err(RollError::MismatchedTarget);
    }
    if target.label != TargetLabel::StealthTotal && target.value < 1 {
        return Err(RollError::InvalidTarget);
    }
    Ok(())
}

fn validate(request: &D20Request) -> Result<(), RollError> {
    if request.test == D20Test::DeathSave
        && (request.ability_modifier != 0 || request.proficiency != Proficiency::None)
    {
        return Err(RollError::UnexpectedModifiers);
    }
    validate_modifiers(request.ability_modifier, request.proficiency)?;
    validate_target(request)?;
    if let D20Test::Attack { critical_from, .. } = request.test
        && !(2..=NATURAL_TWENTY).contains(&critical_from)
    {
        return Err(RollError::InvalidCriticalRange);
    }
    if let Some(inspiration) = request.heroic_inspiration
        && usize::from(inspiration.replaces_die) >= dice_count(request)
    {
        return Err(RollError::InvalidReroll);
    }
    Ok(())
}

/// The dice a D20 request draws, in order: its d20s, then the replacement if Heroic Inspiration
/// is spent.
pub(super) fn d20_plan(request: &D20Request) -> Result<Vec<DieSize>, RollError> {
    validate(request)?;
    let rerolls = usize::from(request.heroic_inspiration.is_some());
    Ok(vec![DieSize::D20; dice_count(request) + rerolls])
}

/// Which of the rolled d20s counts: the higher with Advantage, the lower with Disadvantage, the
/// first on a tie (page 8).
fn kept_index(mode: RollMode, dice: &[RolledDie]) -> Option<usize> {
    dice.iter()
        .enumerate()
        .reduce(|best, candidate| {
            let better = match mode {
                RollMode::Normal => false,
                RollMode::Advantage => candidate.1.face > best.1.face,
                RollMode::Disadvantage => candidate.1.face < best.1.face,
            };
            if better { candidate } else { best }
        })
        .map(|(index, _)| index)
}

fn proficiency_modifier(proficiency: Proficiency) -> Option<Modifier> {
    match proficiency {
        Proficiency::None => None,
        Proficiency::Proficient { bonus } => Some(Modifier {
            label: ModifierLabel::Proficiency,
            value: i16::from(bonus),
        }),
        Proficiency::Expertise { bonus } => Some(Modifier {
            label: ModifierLabel::Expertise,
            value: 2 * i16::from(bonus),
        }),
    }
}

fn test_ability(test: D20Test) -> Option<Ability> {
    match test {
        D20Test::AbilityCheck(ability) | D20Test::SavingThrow(ability) => Some(ability),
        D20Test::Attack { ability, .. } => Some(ability),
        D20Test::SkillCheck(skill) => Some(skill.ability()),
        D20Test::Initiative => Some(Ability::Dexterity),
        D20Test::DeathSave => None,
    }
}

fn subject(test: D20Test) -> RollSubject {
    match test {
        D20Test::AbilityCheck(ability) => RollSubject::AbilityCheck(ability),
        D20Test::SkillCheck(skill) => RollSubject::SkillCheck(skill),
        D20Test::SavingThrow(ability) => RollSubject::SavingThrow(ability),
        D20Test::Attack { kind, .. } => RollSubject::Attack(kind),
        D20Test::Initiative => RollSubject::Initiative,
        D20Test::DeathSave => RollSubject::DeathSave,
    }
}

struct Judged {
    outcome: RollOutcome,
    natural: Natural,
    target: Option<Target>,
}

fn natural_of(face: u16) -> Natural {
    match face {
        face if face == u16::from(NATURAL_ONE) => Natural::One,
        face if face == u16::from(NATURAL_TWENTY) => Natural::Twenty,
        _ => Natural::None,
    }
}

fn rolled_without_target() -> Judged {
    Judged {
        outcome: RollOutcome::Rolled,
        natural: Natural::None,
        target: None,
    }
}

fn judge_attack(critical_from: u8, target: Target, face: u16, total: i16) -> Judged {
    let critical = face >= u16::from(critical_from);
    let hit = face != u16::from(NATURAL_ONE) && (critical || total >= target.value);
    let outcome = match (critical, hit) {
        (true, _) => RollOutcome::CriticalHit,
        (false, true) => RollOutcome::Hit,
        (false, false) => RollOutcome::Miss,
    };
    Judged {
        outcome,
        natural: natural_of(face),
        target: Some(target),
    }
}

fn judge_death_save(face: u16, total: i16) -> Judged {
    let outcome = match face {
        face if face == u16::from(NATURAL_TWENTY) => RollOutcome::DeathSaveRevived,
        face if face == u16::from(NATURAL_ONE) => RollOutcome::DeathSaveTwoFailures,
        _ if total >= DEATH_SAVE_DC => RollOutcome::DeathSaveSuccess,
        _ => RollOutcome::DeathSaveFailure,
    };
    Judged {
        outcome,
        natural: natural_of(face),
        target: Some(Target {
            label: TargetLabel::DifficultyClass,
            value: DEATH_SAVE_DC,
        }),
    }
}

fn judge(request: &D20Request, face: u16, total: i16) -> Result<Judged, RollError> {
    match request.test {
        D20Test::Initiative => Ok(rolled_without_target()),
        D20Test::DeathSave => Ok(judge_death_save(face, total)),
        D20Test::Attack { critical_from, .. } => {
            let target = request.target.ok_or(RollError::MissingTarget)?;
            Ok(judge_attack(critical_from, target, face, total))
        }
        D20Test::AbilityCheck(_) | D20Test::SkillCheck(_) | D20Test::SavingThrow(_) => {
            let target = request.target.ok_or(RollError::MissingTarget)?;
            let outcome = if total >= target.value {
                RollOutcome::Success
            } else {
                RollOutcome::Failure
            };
            Ok(Judged {
                outcome,
                natural: Natural::None,
                target: Some(target),
            })
        }
    }
}

/// Resolves a D20 request from the faces it drew.
pub(super) fn resolve_d20(
    request: &D20Request,
    faces: &mut Faces<'_>,
) -> Result<RollResult, RollError> {
    validate(request)?;
    let mut dice = Vec::with_capacity(dice_count(request));
    for _ in 0..dice_count(request) {
        let face = faces.take(DieSize::D20)?;
        dice.push(RolledDie::new(
            DieSize::D20,
            face,
            DieRole::Test,
            DiePlace::Whole,
        ));
    }
    let mut inspiration_reroll = None;
    if let Some(inspiration) = request.heroic_inspiration {
        let replacement = faces.take(DieSize::D20)?;
        let place = usize::from(inspiration.replaces_die);
        let replaced = dice.get_mut(place).ok_or(RollError::InvalidReroll)?;
        inspiration_reroll = Some(RolledDie {
            kept: false,
            ..*replaced
        });
        replaced.face = replacement;
    }
    let kept = kept_index(request.mode, &dice).ok_or(RollError::MissingDraw {
        sides: DieSize::D20.sides(),
    })?;
    for (index, die) in dice.iter_mut().enumerate() {
        die.kept = index == kept;
    }
    let face =
        dice.iter()
            .find(|die| die.kept)
            .map(|die| die.face)
            .ok_or(RollError::MissingDraw {
                sides: DieSize::D20.sides(),
            })?;
    let mut modifiers = Vec::with_capacity(2);
    if let Some(ability) = test_ability(request.test) {
        modifiers.push(Modifier {
            label: ModifierLabel::ability(ability),
            value: i16::from(request.ability_modifier),
        });
    }
    modifiers.extend(proficiency_modifier(request.proficiency));
    let natural_total = i16::try_from(face).map_err(|_| RollError::FaceOutOfRange {
        sides: DieSize::D20.sides(),
        face,
    })?;
    let total = modifiers
        .iter()
        .fold(natural_total, |sum, modifier| sum + modifier.value);
    let judged = judge(request, face, total)?;
    Ok(RollResult {
        subject: subject(request.test),
        mode: request.mode,
        dice,
        modifiers,
        total,
        target: judged.target,
        outcome: judged.outcome,
        natural: judged.natural,
        inspiration_reroll,
    })
}

/// Resolves a passive Wisdom (Perception) score: no dice, 10 plus the Perception modifier, plus 5
/// with Advantage or minus 5 with Disadvantage (page 186).
pub(super) fn resolve_passive(request: &PassivePerceptionRequest) -> Result<RollResult, RollError> {
    validate_modifiers(request.ability_modifier, request.proficiency)?;
    let mut modifiers = vec![
        Modifier {
            label: ModifierLabel::Base,
            value: PASSIVE_BASE,
        },
        Modifier {
            label: ModifierLabel::Wisdom,
            value: i16::from(request.ability_modifier),
        },
    ];
    modifiers.extend(proficiency_modifier(request.proficiency));
    match request.mode {
        RollMode::Normal => {}
        RollMode::Advantage => modifiers.push(Modifier {
            label: ModifierLabel::Advantage,
            value: PASSIVE_MODE_STEP,
        }),
        RollMode::Disadvantage => modifiers.push(Modifier {
            label: ModifierLabel::Disadvantage,
            value: -PASSIVE_MODE_STEP,
        }),
    }
    let total = modifiers.iter().map(|modifier| modifier.value).sum();
    Ok(RollResult {
        subject: RollSubject::PassivePerception,
        mode: request.mode,
        dice: Vec::new(),
        modifiers,
        total,
        target: None,
        outcome: RollOutcome::Score,
        natural: Natural::None,
        inspiration_reroll: None,
    })
}
