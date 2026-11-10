//! The roll a validated call makes: the `df-rules` request built from the character sheet.
//!
//! The ability modifier, the Proficiency Bonus, Expertise, the weapon's damage and the Champion's
//! critical range are the sheet's; the target is the judge's band on the SRD ladder; the mode is
//! the judge's grounded one. Nothing here invents a modifier or decides how the roll comes out.
//! Contested rolls are the two plain tests the SRD has (pages 183 and 190): escaping a grab is a
//! check against the escape difficulty, finding a hidden creature a check against its Stealth
//! total.

use df_rules::roll::{
    Ability, D20Request, D20Test, DiceExpression, DiceRequest, DieRole, DieSize, HeroicInspiration,
    ModifierLabel, PassivePerceptionRequest, Proficiency, RollRequest, Skill, Target, TargetLabel,
};

use super::request::{AnswerField, CheckCall, CheckError, CheckKind, CheckSheet};

fn need<T>(value: Option<T>, field: AnswerField) -> Result<T, CheckError> {
    value.ok_or(CheckError::MissingField { field })
}

/// How the Proficiency Bonus applies to `skill`.
fn skill_proficiency(sheet: &CheckSheet, skill: Skill) -> Proficiency {
    let bonus = sheet.proficiency_bonus;
    if sheet.skill_expertise.contains(&skill) {
        Proficiency::Expertise { bonus }
    } else if sheet.skill_proficiencies.contains(&skill) {
        Proficiency::Proficient { bonus }
    } else {
        Proficiency::None
    }
}

fn difficulty(call: &CheckCall, label: TargetLabel) -> Result<Target, CheckError> {
    let band = need(call.dc_band, AnswerField::DcBand)?;
    Ok(Target {
        label,
        value: i16::from(band.value()),
    })
}

fn d20(
    sheet: &CheckSheet,
    call: &CheckCall,
    test: D20Test,
    ability: Option<Ability>,
    proficiency: Proficiency,
    target: Option<Target>,
    inspiration: Option<u8>,
) -> Result<RollRequest, CheckError> {
    let ability_modifier = match ability {
        Some(ability) => sheet.modifier(ability)?,
        None => 0,
    };
    Ok(RollRequest::D20(D20Request {
        test,
        mode: call.mode,
        ability_modifier,
        proficiency,
        target,
        heroic_inspiration: inspiration.map(|replaces_die| HeroicInspiration { replaces_die }),
    }))
}

fn dice(
    role: DieRole,
    expression: DiceExpression,
    label: ModifierLabel,
    inspiration: Option<u8>,
) -> RollRequest {
    RollRequest::Dice(DiceRequest {
        role,
        expression,
        modifier_label: label,
        critical: false,
        heroic_inspiration: inspiration.map(|replaces_die| HeroicInspiration { replaces_die }),
    })
}

/// The `df-rules` request `call` makes for the character of `sheet`, with Heroic Inspiration spent
/// on the die at `inspiration` when given.
pub(super) fn roll_request(
    call: &CheckCall,
    sheet: &CheckSheet,
    inspiration: Option<u8>,
) -> Result<RollRequest, CheckError> {
    let kind = call.kind;
    if inspiration.is_some() && matches!(kind, CheckKind::Passive | CheckKind::Table) {
        return Err(CheckError::InspirationNotPossible { kind });
    }
    let weapon = || {
        let id = need(call.uses.as_deref(), AnswerField::Uses)?;
        sheet
            .attack(id)
            .ok_or_else(|| CheckError::UnknownUses { id: id.to_owned() })
    };
    match kind {
        CheckKind::AbilityCheck => {
            let (test, ability, proficiency) = match call.skill {
                Some(skill) => (
                    D20Test::SkillCheck(skill),
                    skill.ability(),
                    skill_proficiency(sheet, skill),
                ),
                None => {
                    let ability = need(call.ability, AnswerField::Ability)?;
                    (D20Test::AbilityCheck(ability), ability, Proficiency::None)
                }
            };
            let target = difficulty(call, TargetLabel::DifficultyClass)?;
            d20(
                sheet,
                call,
                test,
                Some(ability),
                proficiency,
                Some(target),
                inspiration,
            )
        }
        CheckKind::SavingThrow => {
            let ability = need(call.ability, AnswerField::Ability)?;
            let proficiency = if sheet.saving_throw_proficiencies.contains(&ability) {
                Proficiency::Proficient {
                    bonus: sheet.proficiency_bonus,
                }
            } else {
                Proficiency::None
            };
            let target = difficulty(call, TargetLabel::DifficultyClass)?;
            let test = D20Test::SavingThrow(ability);
            d20(
                sheet,
                call,
                test,
                Some(ability),
                proficiency,
                Some(target),
                inspiration,
            )
        }
        CheckKind::AttackRoll => {
            let attack = weapon()?;
            let test = D20Test::Attack {
                ability: attack.ability,
                kind: attack.kind,
                critical_from: sheet.critical_from,
            };
            let proficiency = Proficiency::Proficient {
                bonus: sheet.proficiency_bonus,
            };
            let target = difficulty(call, TargetLabel::ArmorClass)?;
            d20(
                sheet,
                call,
                test,
                Some(attack.ability),
                proficiency,
                Some(target),
                inspiration,
            )
        }
        CheckKind::Initiative => {
            let proficiency = if sheet.initiative_proficient {
                Proficiency::Proficient {
                    bonus: sheet.proficiency_bonus,
                }
            } else {
                Proficiency::None
            };
            d20(
                sheet,
                call,
                D20Test::Initiative,
                Some(Ability::Dexterity),
                proficiency,
                None,
                inspiration,
            )
        }
        CheckKind::DeathSave => d20(
            sheet,
            call,
            D20Test::DeathSave,
            None,
            Proficiency::None,
            None,
            inspiration,
        ),
        CheckKind::Passive => Ok(RollRequest::PassivePerception(PassivePerceptionRequest {
            ability_modifier: sheet.modifier(Ability::Wisdom)?,
            proficiency: skill_proficiency(sheet, Skill::Perception),
            mode: call.mode,
        })),
        CheckKind::Contested => {
            let skill = need(call.skill, AnswerField::Skill)?;
            let label = if skill == Skill::Perception {
                TargetLabel::StealthTotal
            } else {
                TargetLabel::EscapeDc
            };
            let target = difficulty(call, label)?;
            d20(
                sheet,
                call,
                D20Test::SkillCheck(skill),
                Some(skill.ability()),
                skill_proficiency(sheet, skill),
                Some(target),
                inspiration,
            )
        }
        CheckKind::Damage => {
            let attack = weapon()?;
            Ok(dice(
                DieRole::Damage,
                attack.damage,
                ModifierLabel::ability(attack.ability),
                inspiration,
            ))
        }
        CheckKind::Healing => {
            let id = need(call.uses.as_deref(), AnswerField::Uses)?;
            let healing = sheet
                .healing_of(id)
                .ok_or_else(|| CheckError::UnknownUses { id: id.to_owned() })?;
            Ok(dice(
                DieRole::Healing,
                healing.dice,
                healing.bonus_label,
                inspiration,
            ))
        }
        CheckKind::Table => Ok(dice(
            DieRole::Table,
            DiceExpression {
                count: 1,
                die: DieSize::D100,
                modifier: 0,
            },
            ModifierLabel::Flat,
            None,
        )),
    }
}
