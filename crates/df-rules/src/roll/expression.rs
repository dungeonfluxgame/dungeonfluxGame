//! Dice expressions rolled for damage, healing, hit dice, bonus dice and tables (SRD 5.2.1 pages
//! 16 to 17, 26 to 27 and 183).
//!
//! It owns the draw plan and the sum of an `NdS+M` roll. A Critical Hit rolls the damage dice
//! twice and adds the modifier once (page 16). The caller supplies every face; it never rolls, and
//! it does not apply Resistance, Hit Point maximums or any other rule to the total.
use super::abilities::Ability;
use super::d20::HeroicInspiration;
use super::dice::{
    DiceExpression, DiePlace, DieRole, DieSize, MAXIMUM_DICE_PER_EXPRESSION, MAXIMUM_FLAT_MODIFIER,
    RolledDie, percentile_value,
};
use super::result::{Modifier, ModifierLabel, Natural, RollOutcome, RollResult, RollSubject};
use super::{Faces, RollError};
use crate::influence::RollMode;
use serde::{Deserialize, Serialize};

/// One dice expression to roll.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiceRequest {
    /// Any role except `Test`, which belongs to a D20 Test.
    pub role: DieRole,
    pub expression: DiceExpression,
    /// What the flat modifier is: an ability modifier added to damage (page 22), or a plain number.
    pub modifier_label: ModifierLabel,
    /// A Critical Hit: roll all the damage dice twice (page 16). Damage only.
    pub critical: bool,
    pub heroic_inspiration: Option<HeroicInspiration>,
}

impl DiceRequest {
    /// A damage roll that adds an ability modifier, as a weapon attack does (page 22).
    pub const fn damage(expression: DiceExpression, ability: Ability, critical: bool) -> Self {
        Self {
            role: DieRole::Damage,
            expression,
            modifier_label: ModifierLabel::ability(ability),
            critical,
            heroic_inspiration: None,
        }
    }
}

fn validate(request: &DiceRequest) -> Result<(), RollError> {
    if request.role == DieRole::Test {
        return Err(RollError::InvalidRole);
    }
    if request.critical && request.role != DieRole::Damage {
        return Err(RollError::CriticalNeedsDamage);
    }
    let expression = request.expression;
    if !(1..=MAXIMUM_DICE_PER_EXPRESSION).contains(&expression.count) {
        return Err(RollError::InvalidDiceCount);
    }
    if !(-MAXIMUM_FLAT_MODIFIER..=MAXIMUM_FLAT_MODIFIER).contains(&expression.modifier) {
        return Err(RollError::InvalidDiceModifier);
    }
    Ok(())
}

/// The physical dice of the expression with their place in a percentile pair, in the order they
/// are drawn.
fn expression_dice(request: &DiceRequest) -> Vec<(DieSize, DiePlace)> {
    let die = request.expression.die;
    let repeats = if request.critical { 2 } else { 1 };
    let logical = usize::from(request.expression.count) * repeats;
    (0..logical)
        .flat_map(|_| {
            die.physical()
                .iter()
                .enumerate()
                .map(move |(index, size)| (*size, place_of(die, index)))
        })
        .collect()
}

/// The dice a dice request draws, in order: the expression's dice (twice for a Critical Hit), then
/// the replacement if Heroic Inspiration is spent.
pub(super) fn dice_plan(request: &DiceRequest) -> Result<Vec<DieSize>, RollError> {
    validate(request)?;
    let slots = expression_dice(request);
    let mut plan: Vec<DieSize> = slots.iter().map(|(size, _)| *size).collect();
    if let Some(inspiration) = request.heroic_inspiration {
        let (replaced, _) = slots
            .get(usize::from(inspiration.replaces_die))
            .ok_or(RollError::InvalidReroll)?;
        plan.push(*replaced);
    }
    Ok(plan)
}

fn place_of(die: DieSize, physical_index: usize) -> DiePlace {
    match (die, physical_index) {
        (DieSize::D100, 0) => DiePlace::Tens,
        (DieSize::D100, _) => DiePlace::Ones,
        _ => DiePlace::Whole,
    }
}

/// The sum of the dice: a percentile pair is one value from 1 to 100, any other die its face.
fn sum_of(die: DieSize, dice: &[RolledDie]) -> i16 {
    let width = die.physical().len();
    dice.chunks(width)
        .map(|chunk| match chunk {
            [tens, ones] if die == DieSize::D100 => percentile_value(*tens, *ones),
            _ => chunk.iter().map(|die| die.face).sum(),
        })
        .fold(0_i16, |sum, value| sum.saturating_add_unsigned(value))
}

/// Resolves a dice request from the faces it drew.
pub(super) fn resolve_dice(
    request: &DiceRequest,
    faces: &mut Faces<'_>,
) -> Result<RollResult, RollError> {
    validate(request)?;
    let slots = expression_dice(request);
    let mut dice = Vec::with_capacity(slots.len());
    for (size, place) in &slots {
        let face = faces.take(*size)?;
        dice.push(RolledDie::new(*size, face, request.role, *place));
    }
    let mut inspiration_reroll = None;
    if let Some(inspiration) = request.heroic_inspiration {
        let place = usize::from(inspiration.replaces_die);
        let (size, _) = slots.get(place).ok_or(RollError::InvalidReroll)?;
        let replacement = faces.take(*size)?;
        let replaced = dice.get_mut(place).ok_or(RollError::InvalidReroll)?;
        inspiration_reroll = Some(RolledDie {
            kept: false,
            ..*replaced
        });
        replaced.face = replacement;
    }
    let modifier = request.expression.modifier;
    let modifiers = if modifier == 0 {
        Vec::new()
    } else {
        vec![Modifier {
            label: request.modifier_label,
            value: modifier,
        }]
    };
    Ok(RollResult {
        subject: RollSubject::Dice(request.role),
        mode: RollMode::Normal,
        total: sum_of(request.expression.die, &dice).saturating_add(modifier),
        dice,
        modifiers,
        target: None,
        outcome: RollOutcome::Rolled,
        natural: Natural::None,
        inspiration_reroll,
    })
}
