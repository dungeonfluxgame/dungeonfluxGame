//! Selected SRD 5.2.1 creation and normal adjacent melee mechanics.
//! This is an explicitly bounded playable subset, not full 2024 rules coverage.
pub const SOURCE_REVISION: &str = "SRD5.2.1-2024-local-journey-2";
pub const SOURCE: &str = "Wizards of the Coast LLC SRD 5.2.1 English; https://media.dndbeyond.com/compendium-images/srd/5.2/SRD_CC_v5.2.1.pdf; CC BY4.0; PDF SHA256 8974902d109d6e63672d7c490bde9ccf052410503d9cfa768237154fbc5e3d87; creation pages19-22; Fighter47-48; Soldier83; Dwarf84; SavageAttacker87; Defense88; Graze90; weapons91; armor92; initiative13; attack7,14; damage16; knockout17; ShortRest187; Unconscious191; Bandit261. Selected level1 Dwarf Fighter/Soldier, Defense, Greatsword/Flail/Javelin masteries, two standard arrays, normal adjacent attack, SecondWind, nonlethal melee encounter and uninterrupted after-combat ShortRest without HitPointDice; no full-rest or general rest catalog.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JourneyRuleError {
    MissingChoice,
    DuplicateChoice,
    InvalidChoice,
    InvalidName,
    InvalidDie,
    InvalidDamageDice,
    InvalidTarget,
    ActionSpent,
    BonusActionSpent,
    ResourceMissing,
    InvalidRest,
    RestIncomplete,
    RestInterrupted,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegalCharacter {
    pub name: String,
    pub abilities: [u8; 6],
    pub choices: Vec<(String, String)>,
    pub hit_points: u32,
    pub armor_class: u32,
    pub proficiency_bonus: u32,
    pub second_wind_uses: u32,
}
impl LegalCharacter {
    pub fn strength_modifier(&self) -> i32 {
        ability_modifier(self.abilities[0])
    }
    pub fn dexterity_modifier(&self) -> i32 {
        ability_modifier(self.abilities[1])
    }
}
pub const FIGHTER_SKILLS: &[&str] = &[
    "acrobatics",
    "animal-handling",
    "history",
    "insight",
    "persuasion",
    "perception",
    "survival",
];
pub const STANDARD_LANGUAGES: &[&str] = &[
    "common-sign",
    "draconic",
    "dwarvish",
    "elvish",
    "giant",
    "gnomish",
    "goblin",
    "halfling",
    "orc",
];
pub const ALIGNMENTS: &[&str] = &[
    "lawful-good",
    "neutral-good",
    "chaotic-good",
    "lawful-neutral",
    "neutral",
    "chaotic-neutral",
];
pub const GAMING_SETS: &[&str] = &["dice", "dragonchess", "playing-cards", "three-dragon-ante"];
pub const BUILD_GROUPS: &[&str] = &[
    "species",
    "class",
    "background",
    "array",
    "alignment",
    "language-1",
    "language-2",
    "skill-1",
    "skill-2",
    "gaming-set",
    "equipment",
    "style-masteries",
    "allied-ties",
];

/// Validate the explicit supported offers, never accept a client-derived stat or roll.
pub fn validate_character(
    name: &str,
    choices: &[(&str, &str)],
) -> Result<LegalCharacter, JourneyRuleError> {
    if name.is_empty()
        || name.len() > 24
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || matches!(byte, b'-' | b'_'))
    {
        return Err(JourneyRuleError::InvalidName);
    }
    if choices.len() != BUILD_GROUPS.len() {
        return Err(JourneyRuleError::MissingChoice);
    }
    for (index, (group, _)) in choices.iter().enumerate() {
        if !BUILD_GROUPS.contains(group) {
            return Err(JourneyRuleError::InvalidChoice);
        }
        if choices[..index]
            .iter()
            .any(|(previous, _)| previous == group)
        {
            return Err(JourneyRuleError::DuplicateChoice);
        }
    }
    let choice = |group: &str| {
        choices
            .iter()
            .find(|(key, _)| *key == group)
            .map(|(_, value)| *value)
            .ok_or(JourneyRuleError::MissingChoice)
    };
    for (group, expected) in [
        ("species", "dwarf"),
        ("class", "fighter-1"),
        ("background", "soldier"),
        ("equipment", "fighter-a-soldier-a"),
        ("style-masteries", "defense-greatsword-flail-javelin"),
        ("allied-ties", "join-order"),
    ] {
        if choice(group)? != expected {
            return Err(JourneyRuleError::InvalidChoice);
        }
    }
    let mut abilities = match choice("array")? {
        "stalwart" => [15, 13, 14, 10, 12, 8],
        "vanguard" => [15, 14, 13, 8, 10, 12],
        _ => return Err(JourneyRuleError::InvalidChoice),
    };
    // Soldier permits +2 to Strength and +1 to a distinct Constitution.
    abilities[0] += 2;
    abilities[2] += 1;
    if !ALIGNMENTS.contains(&choice("alignment")?) || !GAMING_SETS.contains(&choice("gaming-set")?)
    {
        return Err(JourneyRuleError::InvalidChoice);
    }
    for (first, second, options) in [
        ("language-1", "language-2", STANDARD_LANGUAGES),
        ("skill-1", "skill-2", FIGHTER_SKILLS),
    ] {
        let left = choice(first)?;
        let right = choice(second)?;
        if left == right || !options.contains(&left) || !options.contains(&right) {
            return Err(JourneyRuleError::InvalidChoice);
        }
    }
    Ok(LegalCharacter {
        name: name.to_owned(),
        abilities,
        choices: choices
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        // Fighter maximum first die + Constitution modifier + Dwarven Toughness.
        hit_points: (11 + ability_modifier(abilities[2])) as u32,
        // Chain Mail16, Defense +1, no shield (Greatsword occupies both hands).
        armor_class: 17,
        proficiency_bonus: 2,
        second_wind_uses: 2,
    })
}
pub const fn ability_modifier(score: u8) -> i32 {
    (score as i32 - 10).div_euclid(2)
}

/// Order this local three-participant subset from supplied d20 faces and Dexterity scores.
/// Initiative uses the cited SRD d20 + Dexterity modifier; equal totals retain the
/// existing authored participant order. General player/DM tie choices remain outside
/// this subset. No draw, identity, state mutation or implicit random input occurs here.
pub fn initiative_order(participants: [(u32, u8); 3]) -> Result<[usize; 3], JourneyRuleError> {
    if participants.iter().any(|(die, _)| !(1..=20).contains(die)) {
        return Err(JourneyRuleError::InvalidDie);
    }
    if participants
        .iter()
        .any(|(_, dexterity)| !(1..=20).contains(dexterity))
    {
        return Err(JourneyRuleError::InvalidChoice);
    }
    let totals = participants.map(|(die, dexterity)| die as i32 + ability_modifier(dexterity));
    let mut order = [0, 1, 2];
    // All indices are internal members of the fixed participant array.
    order.sort_by_key(|index| (std::cmp::Reverse(totals[*index]), *index));
    Ok(order)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionEconomy {
    pub action_used: bool,
    pub bonus_action_used: bool,
}
impl ActionEconomy {
    pub fn attack(self) -> Result<Self, JourneyRuleError> {
        if self.action_used {
            return Err(JourneyRuleError::ActionSpent);
        }
        Ok(Self {
            action_used: true,
            ..self
        })
    }
    pub fn second_wind(self, uses: u32) -> Result<Self, JourneyRuleError> {
        if self.bonus_action_used {
            return Err(JourneyRuleError::BonusActionSpent);
        }
        if uses == 0 {
            return Err(JourneyRuleError::ResourceMissing);
        }
        Ok(Self {
            bonus_action_used: true,
            ..self
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttackOutcome {
    pub die: u32,
    pub modifier: i32,
    pub total: i32,
    pub armor_class: u32,
    pub hit: bool,
    pub critical: bool,
    pub damage_dice: Vec<u32>,
    pub damage: u32,
    pub target_hit_points: u32,
    pub knocked_out: bool,
    pub grazed: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct NormalMeleeAttack<'a> {
    pub die: u32,
    pub modifier: i32,
    pub armor_class: u32,
    pub damage_dice: &'a [u32],
    pub weapon_dice: usize,
    pub damage_modifier: i32,
    pub savage_attacker_take_higher: bool,
    pub graze: bool,
    pub target_hit_points: u32,
}
/// Exact normal melee subset: no advantage, cover, surprise, resistance, or hidden target.
/// Both combatants deliberately select source-defined knockout for this encounter.
pub fn resolve_melee(input: NormalMeleeAttack<'_>) -> Result<AttackOutcome, JourneyRuleError> {
    if !(1..=20).contains(&input.die) {
        return Err(JourneyRuleError::InvalidDie);
    }
    if input.target_hit_points == 0 || !(1..=2).contains(&input.weapon_dice) {
        return Err(JourneyRuleError::InvalidTarget);
    }
    let total = input.die as i32 + input.modifier;
    let critical = input.die == 20;
    let hit = input.die != 1 && (critical || total >= input.armor_class as i32);
    let count = input.weapon_dice * if critical { 2 } else { 1 };
    let expected = if hit {
        count
            * if input.savage_attacker_take_higher {
                2
            } else {
                1
            }
    } else {
        0
    };
    if input.damage_dice.len() != expected
        || input.damage_dice.iter().any(|die| !(1..=6).contains(die))
    {
        return Err(JourneyRuleError::InvalidDamageDice);
    }
    let weapon_damage = if hit {
        let first: u32 = input.damage_dice[..count].iter().sum();
        if input.savage_attacker_take_higher {
            first.max(input.damage_dice[count..].iter().sum())
        } else {
            first
        }
    } else {
        0
    };
    let grazed = !hit && input.graze;
    let damage = if hit {
        (weapon_damage as i32 + input.damage_modifier).max(0) as u32
    } else if grazed {
        input.damage_modifier.max(0) as u32
    } else {
        0
    };
    let knocked_out = damage >= input.target_hit_points;
    Ok(AttackOutcome {
        die: input.die,
        modifier: input.modifier,
        total,
        armor_class: input.armor_class,
        hit,
        critical,
        damage_dice: input.damage_dice.to_vec(),
        damage,
        target_hit_points: if knocked_out {
            1
        } else {
            input.target_hit_points - damage
        },
        knocked_out,
        grazed,
    })
}
pub fn heal_second_wind(
    die: u32,
    current: u32,
    maximum: u32,
    uses: u32,
) -> Result<(u32, u32), JourneyRuleError> {
    if !(1..=10).contains(&die) {
        return Err(JourneyRuleError::InvalidDie);
    }
    if uses == 0 {
        return Err(JourneyRuleError::ResourceMissing);
    }
    if maximum == 0 || current == 0 || current > maximum {
        return Err(JourneyRuleError::InvalidTarget);
    }
    Ok((maximum.min(current + die + 1), uses - 1))
}

/// Source p187 fixed one-hour duration, expressed in admitted campaign ticks.
/// This does not advance time or supply healing/dice.
pub fn short_rest_due(started: u64, ticks_per_second: u32) -> Result<u64, JourneyRuleError> {
    if ticks_per_second == 0 {
        return Err(JourneyRuleError::InvalidRest);
    }
    started
        .checked_add(u64::from(ticks_per_second) * 3600)
        .ok_or(JourneyRuleError::InvalidRest)
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NoHitDiceShortRest {
    pub started: u64,
    pub finished: u64,
    pub ticks_per_second: u32,
    pub interrupted: bool,
    pub hit_points: u32,
    pub fighter_second_wind_uses: Option<u32>,
    pub knockout_unconscious: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShortRestBenefits {
    pub hit_points: u32,
    pub fighter_second_wind_uses: Option<u32>,
    pub ends_knockout_unconscious: bool,
}
/// Selected no-Hit-Dice rest: unchanged HP, Fighter p48 +one use capped2,
/// and source p17 knockout unconsciousness ends. Prone/equipment stay with the caller.
pub fn complete_no_hit_dice_short_rest(
    input: NoHitDiceShortRest,
) -> Result<ShortRestBenefits, JourneyRuleError> {
    if input.interrupted {
        return Err(JourneyRuleError::RestInterrupted);
    }
    if input.hit_points == 0 || input.fighter_second_wind_uses.is_some_and(|uses| uses > 2) {
        return Err(JourneyRuleError::InvalidRest);
    }
    if input.finished < short_rest_due(input.started, input.ticks_per_second)? {
        return Err(JourneyRuleError::RestIncomplete);
    }
    Ok(ShortRestBenefits {
        hit_points: input.hit_points,
        fighter_second_wind_uses: input.fighter_second_wind_uses.map(|uses| 2.min(uses + 1)),
        ends_knockout_unconscious: input.knockout_unconscious,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn choices() -> Vec<(&'static str, &'static str)> {
        vec![
            ("species", "dwarf"),
            ("class", "fighter-1"),
            ("background", "soldier"),
            ("array", "stalwart"),
            ("alignment", "neutral-good"),
            ("language-1", "dwarvish"),
            ("language-2", "elvish"),
            ("skill-1", "perception"),
            ("skill-2", "survival"),
            ("gaming-set", "dice"),
            ("equipment", "fighter-a-soldier-a"),
            ("style-masteries", "defense-greatsword-flail-javelin"),
            ("allied-ties", "join-order"),
        ]
    }

    #[test]
    fn initiative_uses_supplied_faces_and_modifier_and_retains_authored_tie_order() {
        assert_eq!(
            initiative_order([(10, 13), (9, 14), (10, 12)]),
            Ok([0, 1, 2])
        );
        assert_eq!(
            initiative_order([(8, 13), (14, 14), (4, 12)]),
            Ok([1, 0, 2])
        );
        assert_eq!(
            initiative_order([(1, 1), (20, 20), (10, 10)]),
            Ok([1, 2, 0])
        );
    }
    #[test]
    fn initiative_rejects_invalid_local_faces_and_ability_scores() {
        for position in 0..3 {
            for invalid in [0, 21] {
                let mut supplied = [(10, 12); 3];
                supplied[position].0 = invalid;
                assert_eq!(
                    initiative_order(supplied),
                    Err(JourneyRuleError::InvalidDie)
                );
                supplied[position] = (10, invalid as u8);
                assert_eq!(
                    initiative_order(supplied),
                    Err(JourneyRuleError::InvalidChoice)
                );
            }
        }
    }
    #[test]
    fn legal_build_and_illegal_duplicate_skill() {
        let mut values = choices();
        let built = validate_character("Brynn", &values).expect("source-legal selection");
        assert_eq!(built.abilities, [17, 13, 15, 10, 12, 8]);
        assert_eq!(
            (built.hit_points, built.armor_class, built.proficiency_bonus),
            (13, 17, 2)
        );
        values[8].1 = "perception";
        assert_eq!(
            validate_character("Brynn", &values),
            Err(JourneyRuleError::InvalidChoice)
        );
        values[8].1 = "athletics"; // Soldier already provides it.
        assert_eq!(
            validate_character("Brynn", &values),
            Err(JourneyRuleError::InvalidChoice)
        );
    }
    #[test]
    fn critical_doubles_weapon_dice_and_knockout_precedes_zero() {
        let result = resolve_melee(NormalMeleeAttack {
            die: 20,
            modifier: 5,
            armor_class: 12,
            damage_dice: &[6, 5, 4, 3, 1, 1, 1, 1],
            weapon_dice: 2,
            damage_modifier: 3,
            savage_attacker_take_higher: true,
            graze: true,
            target_hit_points: 11,
        })
        .expect("critical explicit dice");
        assert!(result.hit && result.critical && result.knocked_out);
        assert_eq!((result.damage, result.target_hit_points), (21, 1));
        assert_eq!(result.damage_dice.len(), 8);
    }
    #[test]
    fn natural_one_graze_and_bad_dice_are_explicit() {
        let attack = NormalMeleeAttack {
            die: 1,
            modifier: 50,
            armor_class: 12,
            damage_dice: &[],
            weapon_dice: 2,
            damage_modifier: 3,
            savage_attacker_take_higher: true,
            graze: true,
            target_hit_points: 11,
        };
        let result = resolve_melee(attack).expect("miss with Graze");
        assert!(!result.hit && result.grazed);
        assert_eq!((result.damage, result.target_hit_points), (3, 8));
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                damage_dice: &[6],
                ..attack
            }),
            Err(JourneyRuleError::InvalidDamageDice)
        );
    }
    #[test]
    fn action_and_bonus_economy_and_healing_do_not_reset_action() {
        let start = ActionEconomy {
            action_used: false,
            bonus_action_used: false,
        };
        let attack = start.attack().expect("one action");
        assert_eq!(attack.attack(), Err(JourneyRuleError::ActionSpent));
        let healed = attack.second_wind(2).expect("one bonus action");
        assert!(healed.action_used && healed.bonus_action_used);
        assert_eq!(
            healed.second_wind(1),
            Err(JourneyRuleError::BonusActionSpent)
        );
        assert_eq!(heal_second_wind(10, 5, 13, 2), Ok((13, 1)));
        assert_eq!(
            heal_second_wind(1, 13, 13, 0),
            Err(JourneyRuleError::ResourceMissing)
        );
    }
    #[test]
    fn no_hit_dice_rest_requires_uninterrupted_hour_and_caps_one_recovered_use() {
        let rest = NoHitDiceShortRest {
            started: 6,
            finished: 3606,
            ticks_per_second: 1,
            interrupted: false,
            hit_points: 1,
            fighter_second_wind_uses: Some(0),
            knockout_unconscious: true,
        };
        assert_eq!(
            complete_no_hit_dice_short_rest(NoHitDiceShortRest {
                finished: 3605,
                ..rest
            }),
            Err(JourneyRuleError::RestIncomplete)
        );
        assert_eq!(
            complete_no_hit_dice_short_rest(NoHitDiceShortRest {
                interrupted: true,
                ..rest
            }),
            Err(JourneyRuleError::RestInterrupted)
        );
        assert_eq!(
            complete_no_hit_dice_short_rest(NoHitDiceShortRest {
                hit_points: 0,
                ..rest
            }),
            Err(JourneyRuleError::InvalidRest)
        );
        let result = complete_no_hit_dice_short_rest(rest).unwrap();
        assert_eq!(
            result,
            ShortRestBenefits {
                hit_points: 1,
                fighter_second_wind_uses: Some(1),
                ends_knockout_unconscious: true
            }
        );
        for uses in [1, 2] {
            assert_eq!(
                complete_no_hit_dice_short_rest(NoHitDiceShortRest {
                    fighter_second_wind_uses: Some(uses),
                    ..rest
                })
                .unwrap()
                .fighter_second_wind_uses,
                Some(2)
            );
        }
        assert_eq!(short_rest_due(6, 2), Ok(7206));
        assert_eq!(
            short_rest_due(u64::MAX, 1),
            Err(JourneyRuleError::InvalidRest)
        );
        assert_eq!(short_rest_due(0, 0), Err(JourneyRuleError::InvalidRest));
    }
}
