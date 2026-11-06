//! The normal, single-d20 ability-check subset of SRD 5.2.1, page 6.
//! The authority supplies the draw, applicable modifiers, and GM-selected DC.
//! Advantage, Disadvantage, conditions, and other adjustments are not supported here.

/// Pinned source clause of this check subset.
pub const SOURCE_REVISION: &str = "SRD-5.2.1:page-6:D20-Tests:Ability-Checks";

/// The supplied d20 face, modifiers and GM-selected DC of one ability check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbilityCheckInput {
    pub die: u8,
    pub ability_modifier: i8,
    pub proficiency_bonus: u8,
    pub difficulty_class: u8,
}

/// A resolved check: the inputs, the total and whether it meets the DC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbilityCheckResult {
    pub die: u8,
    pub ability_modifier: i8,
    pub proficiency_bonus: u8,
    pub total: i16,
    pub difficulty_class: u8,
    pub succeeded: bool,
}

/// Which input falls outside the supported normal-check subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AbilityCheckError {
    InvalidDie,
    InvalidAbilityModifier,
    UnsupportedProficiencyBonus,
    InvalidDifficultyClass,
}

/// Resolve the explicitly selected normal-check subset, without I/O or state changes.
/// A proficiency bonus of zero means the character lacks the relevant proficiency.
/// Natural 1 and 20 do not override the total-versus-DC comparison for an ability check.
pub fn resolve(input: AbilityCheckInput) -> Result<AbilityCheckResult, AbilityCheckError> {
    if !(1..=20).contains(&input.die) {
        return Err(AbilityCheckError::InvalidDie);
    }
    if !(-5..=10).contains(&input.ability_modifier) {
        return Err(AbilityCheckError::InvalidAbilityModifier);
    }
    if input.proficiency_bonus != 0 && !(2..=6).contains(&input.proficiency_bonus) {
        return Err(AbilityCheckError::UnsupportedProficiencyBonus);
    }
    if input.difficulty_class == 0 {
        return Err(AbilityCheckError::InvalidDifficultyClass);
    }
    let total = i16::from(input.die)
        + i16::from(input.ability_modifier)
        + i16::from(input.proficiency_bonus);
    Ok(AbilityCheckResult {
        die: input.die,
        ability_modifier: input.ability_modifier,
        proficiency_bonus: input.proficiency_bonus,
        total,
        difficulty_class: input.difficulty_class,
        succeeded: total >= i16::from(input.difficulty_class),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(die: u8) -> AbilityCheckInput {
        AbilityCheckInput {
            die,
            ability_modifier: 2,
            proficiency_bonus: 2,
            difficulty_class: 15,
        }
    }

    #[test]
    fn matching_dc_succeeds_and_one_lower_fails() {
        let success = resolve(input(11)).unwrap();
        let failure = resolve(input(10)).unwrap();
        assert_eq!(success.total, 15);
        assert!(success.succeeded);
        assert_eq!(failure.total, 14);
        assert!(!failure.succeeded);
    }

    #[test]
    fn natural_die_extremes_do_not_override_ability_check_total() {
        let low = resolve(AbilityCheckInput {
            die: 1,
            ability_modifier: 10,
            proficiency_bonus: 6,
            difficulty_class: 15,
        })
        .unwrap();
        let high = resolve(AbilityCheckInput {
            die: 20,
            ability_modifier: -5,
            proficiency_bonus: 0,
            difficulty_class: 20,
        })
        .unwrap();
        assert!(low.succeeded);
        assert!(!high.succeeded);
    }

    #[test]
    fn rejects_inputs_outside_the_supported_subset() {
        assert_eq!(resolve(input(0)), Err(AbilityCheckError::InvalidDie));
        assert_eq!(resolve(input(21)), Err(AbilityCheckError::InvalidDie));
        assert_eq!(
            resolve(AbilityCheckInput {
                ability_modifier: -6,
                ..input(11)
            }),
            Err(AbilityCheckError::InvalidAbilityModifier)
        );
        assert_eq!(
            resolve(AbilityCheckInput {
                proficiency_bonus: 1,
                ..input(11)
            }),
            Err(AbilityCheckError::UnsupportedProficiencyBonus)
        );
        assert_eq!(
            resolve(AbilityCheckInput {
                difficulty_class: 0,
                ..input(11)
            }),
            Err(AbilityCheckError::InvalidDifficultyClass)
        );
    }
}
