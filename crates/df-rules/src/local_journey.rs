//! Selected SRD 5.2.1 creation and normal adjacent melee mechanics.
//! This is an explicitly bounded playable subset, not full 2024 rules coverage.
use serde::{Deserialize, Serialize};

pub const SOURCE_REVISION: &str = "SRD5.2.1-2024-local-journey-3";
pub const SOURCE: &str = "Wizards of the Coast LLC SRD 5.2.1 English; https://media.dndbeyond.com/compendium-images/srd/5.2/SRD_CC_v5.2.1.pdf; CC BY4.0; PDF SHA256 8974902d109d6e63672d7c490bde9ccf052410503d9cfa768237154fbc5e3d87; creation pages19-22; Fighter47-48; Soldier83; species84-86; Origin feats83,87-88; Defense88; Graze90; weapons91; armor92; initiative13; attack7,14; damage16; knockout17; ShortRest187; Unconscious191; Bandit261. Selected level1 Dwarf, Halfling, or Orc Fighter/Soldier; Defense; Greatsword/Flail/Javelin or archer equipment; standard arrays; normal adjacent attack; SecondWind; nonlethal melee encounter; uninterrupted after-combat ShortRest without HitPointDice; no full-rest or general rest catalog.";

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
    InvalidRules,
    InvalidRest,
    RestIncomplete,
    RestInterrupted,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeroSpecies {
    Dragonborn,
    Dwarf,
    Elf,
    Gnome,
    Goliath,
    Halfling,
    Human,
    Orc,
    Tiefling,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeciesSize {
    Small,
    Medium,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeciesCombatTraits {
    pub size: SpeciesSize,
    pub speed_ft: u16,
    pub darkvision_ft: Option<u16>,
    pub hit_points_per_level: u8,
    pub poison_resistance: bool,
    pub reroll_natural_ones: bool,
    pub relentless_endurance: bool,
}

impl HeroSpecies {
    pub fn from_choice(value: &str) -> Option<Self> {
        Some(match value {
            "dragonborn" => Self::Dragonborn,
            "dwarf" => Self::Dwarf,
            "elf" => Self::Elf,
            "gnome" => Self::Gnome,
            "goliath" => Self::Goliath,
            "halfling" => Self::Halfling,
            "human" => Self::Human,
            "orc" => Self::Orc,
            "tiefling" => Self::Tiefling,
            _ => return None,
        })
    }

    pub fn choice_id(self) -> &'static str {
        match self {
            Self::Dragonborn => "dragonborn",
            Self::Dwarf => "dwarf",
            Self::Elf => "elf",
            Self::Gnome => "gnome",
            Self::Goliath => "goliath",
            Self::Halfling => "halfling",
            Self::Human => "human",
            Self::Orc => "orc",
            Self::Tiefling => "tiefling",
        }
    }

    /// Level-one combat traits from SRD 5.2.1 pp. 84-86. Lineage choices and
    /// combat features without an implemented rules handler remain unavailable.
    pub const fn combat_traits(self) -> SpeciesCombatTraits {
        use SpeciesSize::{Medium, Small};
        match self {
            Self::Dragonborn => SpeciesCombatTraits {
                size: Medium,
                speed_ft: 30,
                darkvision_ft: Some(60),
                hit_points_per_level: 0,
                poison_resistance: false,
                reroll_natural_ones: false,
                relentless_endurance: false,
            },
            // SRD 5.2.1 p. 84: "You have Darkvision with a range of 120 feet."
            // "Your Hit Point maximum increases by 1, and it increases by 1 again whenever you gain a level."
            Self::Dwarf => SpeciesCombatTraits {
                size: Medium,
                speed_ft: 30,
                darkvision_ft: Some(120),
                hit_points_per_level: 1,
                poison_resistance: true,
                reroll_natural_ones: false,
                relentless_endurance: false,
            },
            Self::Elf => SpeciesCombatTraits {
                size: Medium,
                speed_ft: 30,
                darkvision_ft: Some(60),
                hit_points_per_level: 0,
                poison_resistance: false,
                reroll_natural_ones: false,
                relentless_endurance: false,
            },
            Self::Gnome => SpeciesCombatTraits {
                size: Small,
                speed_ft: 30,
                darkvision_ft: Some(60),
                hit_points_per_level: 0,
                poison_resistance: false,
                reroll_natural_ones: false,
                relentless_endurance: false,
            },
            Self::Goliath => SpeciesCombatTraits {
                size: Medium,
                speed_ft: 35,
                darkvision_ft: None,
                hit_points_per_level: 0,
                poison_resistance: false,
                reroll_natural_ones: false,
                relentless_endurance: false,
            },
            // SRD 5.2.1 p. 86: "When you roll a 1 on the d20 of a D20 Test, you can reroll the die, and you must use the new roll."
            Self::Halfling => SpeciesCombatTraits {
                size: Small,
                speed_ft: 30,
                darkvision_ft: None,
                hit_points_per_level: 0,
                poison_resistance: false,
                reroll_natural_ones: true,
                relentless_endurance: false,
            },
            Self::Human => SpeciesCombatTraits {
                size: Medium,
                speed_ft: 30,
                darkvision_ft: None,
                hit_points_per_level: 0,
                poison_resistance: false,
                reroll_natural_ones: false,
                relentless_endurance: false,
            },
            // SRD 5.2.1 p. 86: "You have Darkvision with a range of 120 feet."
            Self::Orc => SpeciesCombatTraits {
                size: Medium,
                speed_ft: 30,
                darkvision_ft: Some(120),
                hit_points_per_level: 0,
                poison_resistance: false,
                reroll_natural_ones: false,
                relentless_endurance: true,
            },
            Self::Tiefling => SpeciesCombatTraits {
                size: Medium,
                speed_ft: 30,
                darkvision_ft: Some(60),
                hit_points_per_level: 0,
                poison_resistance: false,
                reroll_natural_ones: false,
                relentless_endurance: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginFeat {
    Alert,
    MagicInitiate,
    SavageAttacker,
}

/// SRD 5.2.1 p. 83: Acolyte/Sage list "Feat: Magic Initiate," Criminal "Feat: Alert," and Soldier "Feat: Savage Attacker."
pub fn background_origin_feat(background: &str) -> Option<OriginFeat> {
    Some(match background {
        "acolyte" | "sage" => OriginFeat::MagicInitiate,
        "criminal" => OriginFeat::Alert,
        "soldier" => OriginFeat::SavageAttacker,
        _ => return None,
    })
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChallengeRating {
    Zero,
    Eighth,
    Quarter,
    Half,
    Whole(u8),
}

/// SRD 5.2.1 p. 256: "The number of Experience Points (XP) a monster is worth is based on its CR." CR 0 is 0 or 10 XP.
pub fn experience_points_for_challenge_rating(cr: ChallengeRating) -> Option<u32> {
    const WHOLE_CR_XP: [u32; 30] = [
        200, 450, 700, 1_100, 1_800, 2_300, 2_900, 3_900, 5_000, 5_900, 7_200, 8_400, 10_000,
        11_500, 13_000, 15_000, 18_000, 20_000, 22_000, 25_000, 33_000, 41_000, 50_000, 62_000,
        75_000, 90_000, 105_000, 120_000, 135_000, 155_000,
    ];
    Some(match cr {
        ChallengeRating::Zero => return None,
        ChallengeRating::Eighth => 25,
        ChallengeRating::Quarter => 50,
        ChallengeRating::Half => 100,
        ChallengeRating::Whole(value) => *WHOLE_CR_XP.get(usize::from(value.checked_sub(1)?))?,
    })
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
/// Fixed supported Fighter/Champion progression (SRD 5.2.1 pp. 23, 47-49, 87).
/// Level 4 selects +2 STR for melee or +2 DEX for archers; later HP use fixed 6.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FighterProgression {
    level: u8,
}
impl FighterProgression {
    pub fn new(level: u8) -> Result<Self, JourneyRuleError> {
        if !(1..=5).contains(&level) {
            return Err(JourneyRuleError::InvalidChoice);
        }
        Ok(Self { level })
    }
    pub fn level(self) -> u8 {
        self.level
    }
    pub fn proficiency_bonus(self) -> u8 {
        if self.level >= 5 { 3 } else { 2 }
    }
    pub fn second_wind_maximum(self) -> u8 {
        if self.level >= 4 { 3 } else { 2 }
    }
    pub fn champion(self) -> bool {
        self.level >= 3
    }
    pub fn hit_points(self, constitution: u8, species: HeroSpecies) -> u16 {
        let per_level = ability_modifier(constitution)
            + i32::from(species.combat_traits().hit_points_per_level);
        (10 + 6 * i32::from(self.level - 1) + per_level * i32::from(self.level)) as u16
    }
    /// Advance an accepted level-one receipt using this build's fixed source-legal ASI.
    pub fn apply(self, character: &mut LegalCharacter) -> Result<(), JourneyRuleError> {
        let species = character
            .choices
            .iter()
            .find(|(key, _)| key == "species")
            .and_then(|(_, value)| HeroSpecies::from_choice(value))
            .ok_or(JourneyRuleError::InvalidChoice)?;
        let archer = character
            .choices
            .iter()
            .any(|(key, value)| key == "array" && value == "dexterous");
        if self.level >= 4 {
            let ability = if archer { 1 } else { 0 };
            if character.abilities[ability] > 18 {
                return Err(JourneyRuleError::InvalidChoice);
            }
            character.abilities[ability] += 2;
        }
        character.hit_points = u32::from(self.hit_points(character.abilities[2], species));
        character.proficiency_bonus = u32::from(self.proficiency_bonus());
        character.second_wind_uses = u32::from(self.second_wind_maximum());
        if archer {
            character.armor_class = u32::try_from(12 + character.dexterity_modifier())
                .map_err(|_| JourneyRuleError::InvalidRules)?;
        }
        Ok(())
    }
    /// Tactical Mind follows a failed check; a still-failed check does not spend Second Wind.
    pub fn tactical_mind(
        self,
        total: i32,
        dc: i32,
        die: u32,
        uses: u8,
    ) -> Result<(i32, u8), JourneyRuleError> {
        if self.level < 2 || total >= dc {
            return Err(JourneyRuleError::InvalidChoice);
        }
        if uses == 0 || uses > self.second_wind_maximum() {
            return Err(JourneyRuleError::ResourceMissing);
        }
        if !(1..=10).contains(&die) {
            return Err(JourneyRuleError::InvalidDie);
        }
        let improved = total
            .checked_add(die as i32)
            .ok_or(JourneyRuleError::InvalidRules)?;
        Ok((improved, uses - u8::from(improved >= dc)))
    }
    /// Champion has Advantage on Strength (Athletics) checks (p.49).
    pub fn athletics_advantage(self) -> bool {
        self.champion()
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

/// A compiled supported array identifier and its unmodified STR/DEX/CON/INT/WIS/CHA scores.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbilityArray<'a> {
    pub id: &'a str,
    pub scores: [u8; 6],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackgroundBonus {
    pub strength: u8,
    pub constitution: u8,
}
/// Creation numbers; array IDs remain within the compiled supported offer set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreationRules<'a> {
    pub ability_arrays: &'a [AbilityArray<'a>],
    pub background_bonus: BackgroundBonus,
    pub base_hit_points: u8,
    pub armor_class: u8,
    pub proficiency_bonus: u8,
    pub second_wind_uses_max: u8,
}
impl CreationRules<'_> {
    pub const fn srd_521() -> Self {
        SRD_521_BASELINE.creation
    }
    /// Reject out-of-bounds numbers, unsupported/duplicate IDs and nonpositive derived HP.
    pub fn validate(&self) -> Result<(), JourneyRuleError> {
        if !(1..=60).contains(&self.base_hit_points)
            || !(10..=30).contains(&self.armor_class)
            || !(2..=6).contains(&self.proficiency_bonus)
            || self.second_wind_uses_max > 5
            || self.background_bonus.strength > 4
            || self.background_bonus.constitution > 4
            || self.ability_arrays.is_empty()
        {
            return Err(JourneyRuleError::InvalidRules);
        }
        for (index, array) in self.ability_arrays.iter().enumerate() {
            if !matches!(array.id, "stalwart" | "vanguard")
                || self.ability_arrays[..index]
                    .iter()
                    .any(|previous| previous.id == array.id)
                || array.scores.iter().any(|score| !(3..=18).contains(score))
                || array.scores[0] + self.background_bonus.strength > 20
                || array.scores[2] + self.background_bonus.constitution > 20
                || i32::from(self.base_hit_points)
                    + ability_modifier(array.scores[2] + self.background_bonus.constitution)
                    <= 0
            {
                return Err(JourneyRuleError::InvalidRules);
            }
        }
        Ok(())
    }
}
/// Combat numbers shared with the native caller; d20 and dice-doubling mechanics stay fixed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CombatRules {
    pub critical_threshold: u8,
    pub round_ticks: u32,
    pub combat_log_entries: u8,
}
impl CombatRules {
    pub const fn srd_521() -> Self {
        SRD_521_BASELINE.combat
    }
    pub fn validate(&self) -> Result<(), JourneyRuleError> {
        if !(15..=20).contains(&self.critical_threshold)
            || !(1..=600).contains(&self.round_ticks)
            || !(1..=32).contains(&self.combat_log_entries)
        {
            return Err(JourneyRuleError::InvalidRules);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecondWindRules {
    pub uses_max: u8,
    pub die_sides: u8,
    pub flat_bonus: u8,
    pub recharge_per_short_rest: u8,
}
impl SecondWindRules {
    pub const fn srd_521() -> Self {
        SRD_521_BASELINE.second_wind
    }
    pub fn validate(&self) -> Result<(), JourneyRuleError> {
        if self.uses_max > 5
            || !permitted_damage_sides(self.die_sides)
            || self.flat_bonus > 10
            || self.recharge_per_short_rest > self.uses_max
        {
            return Err(JourneyRuleError::InvalidRules);
        }
        Ok(())
    }
}
/// Rest duration in seconds of admitted campaign time, never wall-clock time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestRules {
    pub short_rest_seconds: u32,
}
impl RestRules {
    pub const fn srd_521() -> Self {
        SRD_521_BASELINE.rest
    }
    pub fn validate(&self) -> Result<(), JourneyRuleError> {
        if !(60..=86400).contains(&self.short_rest_seconds) {
            return Err(JourneyRuleError::InvalidRules);
        }
        Ok(())
    }
}
/// SRD 5.2.1 p47: the Fighter's Hit Point Die is a d10.
pub const LEVEL_ONE_FIGHTER_HIT_POINT_DIE_SIDES: u32 = 10;
/// SRD 5.2.1 p187: heal by the die roll plus Constitution (minimum 1); p17 caps HP at its maximum.
pub fn heal_hit_point_die(
    die: u32,
    constitution_modifier: i32,
    current: u32,
    maximum: u32,
) -> Result<(u32, u32), JourneyRuleError> {
    if !(1..=LEVEL_ONE_FIGHTER_HIT_POINT_DIE_SIDES).contains(&die) {
        return Err(JourneyRuleError::InvalidDie);
    }
    if maximum == 0 || current == 0 || current >= maximum {
        return Err(JourneyRuleError::InvalidTarget);
    }
    let rolled = (i64::from(die) + i64::from(constitution_modifier)).max(1) as u32;
    let healed = rolled.min(maximum - current);
    Ok((current + healed, healed))
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WeaponRules {
    pub damage_dice: u8,
    pub damage_sides: u8,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BanditRules {
    pub hit_points: u8,
    pub armor_class: u8,
    pub abilities: [u8; 6],
    pub attack_bonus: i32,
    pub damage_modifier: i32,
    pub weapon: WeaponRules,
}
/// Compiled defaults for every standard-sensitive tuning/pack field (§3).
/// A loader compares against these values; input data cannot declare itself standard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Srd521Baseline {
    pub creation: CreationRules<'static>,
    pub combat: CombatRules,
    pub second_wind: SecondWindRules,
    pub rest: RestRules,
    pub hero_weapon: WeaponRules,
    pub bandit: BanditRules,
    pub equipment: &'static [(&'static str, u8)],
    pub fighter_skills: &'static [&'static str],
    pub standard_languages: &'static [&'static str],
    pub alignments: &'static [&'static str],
    pub gaming_sets: &'static [&'static str],
}
pub const SRD_521_BASELINE: Srd521Baseline = Srd521Baseline {
    creation: CreationRules {
        ability_arrays: &[
            AbilityArray {
                id: "stalwart",
                scores: [15, 13, 14, 10, 12, 8],
            },
            AbilityArray {
                id: "vanguard",
                scores: [15, 14, 13, 8, 10, 12],
            },
        ],
        background_bonus: BackgroundBonus {
            strength: 2,
            constitution: 1,
        },
        base_hit_points: 10,
        armor_class: 17,
        proficiency_bonus: 2,
        second_wind_uses_max: 2,
    },
    combat: CombatRules {
        critical_threshold: 20,
        round_ticks: 6,
        combat_log_entries: 6,
    },
    second_wind: SecondWindRules {
        uses_max: 2,
        die_sides: 10,
        flat_bonus: 1,
        recharge_per_short_rest: 1,
    },
    rest: RestRules {
        short_rest_seconds: 3600,
    },
    hero_weapon: WeaponRules {
        damage_dice: 2,
        damage_sides: 6,
    },
    bandit: BanditRules {
        hit_points: 11,
        armor_class: 12,
        abilities: [11, 12, 12, 10, 10, 10],
        attack_bonus: 3,
        damage_modifier: 1,
        weapon: WeaponRules {
            damage_dice: 1,
            damage_sides: 6,
        },
    },
    equipment: &[
        ("chain-mail", 1),
        ("greatsword", 1),
        ("flail", 1),
        ("javelin", 8),
        ("dungeoneer-pack", 1),
        ("spear", 1),
        ("shortbow", 1),
        ("arrow", 20),
        ("gaming-set", 1),
        ("healers-kit", 1),
        ("quiver", 1),
        ("travelers-clothes", 1),
        ("gold-piece", 18),
    ],
    fighter_skills: FIGHTER_SKILLS,
    standard_languages: STANDARD_LANGUAGES,
    alignments: ALIGNMENTS,
    gaming_sets: GAMING_SETS,
};

/// Longest hero name a player can type, in characters.
pub const MAXIMUM_CHARACTER_NAME_CHARACTERS: usize = 32;
const MAXIMUM_CHARACTER_NAME_BYTES: usize = 48;
const ATTACK_DIE_SIDES: u32 = 20;
const NATURAL_MISS: u32 = 1;
const MINIMUM_WEAPON_DICE: usize = 1;
const MAXIMUM_WEAPON_DICE: usize = 2;
const KNOCKOUT_HIT_POINTS: u32 = 1;

const fn permitted_damage_sides(sides: u8) -> bool {
    matches!(sides, 4 | 6 | 8 | 10 | 12)
}

/// A hero name is one or more words of letters from any alphabet. Single spaces, apostrophes, hyphens,
/// periods and underscores may join letters ("Vix Ashlantern", "D'Artagnan", "Jean-Luc", "St. Brynn");
/// the name starts with a letter and ends with a letter or a period.
pub fn valid_character_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > MAXIMUM_CHARACTER_NAME_BYTES
        || name.chars().count() > MAXIMUM_CHARACTER_NAME_CHARACTERS
    {
        return false;
    }
    let mut previous: Option<char> = None;
    for character in name.chars() {
        if !character.is_alphabetic() {
            let joins = matches!(character, ' ' | '\'' | '\u{2019}' | '-' | '.' | '_');
            let follows = match previous {
                Some(letter) if letter.is_alphabetic() => true,
                Some('.') => character == ' ',
                _ => false,
            };
            if !joins || !follows {
                return false;
            }
        }
        previous = Some(character);
    }
    previous.is_some_and(|last| last.is_alphabetic() || last == '.')
}

/// Validate the explicit supported offers, never accept a client-derived stat or roll.
pub fn validate_character(
    rules: &CreationRules<'_>,
    name: &str,
    choices: &[(&str, &str)],
) -> Result<LegalCharacter, JourneyRuleError> {
    rules.validate()?;
    if !valid_character_name(name) {
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
    let species =
        HeroSpecies::from_choice(choice("species")?).ok_or(JourneyRuleError::InvalidChoice)?;
    if !matches!(
        species,
        HeroSpecies::Dwarf | HeroSpecies::Halfling | HeroSpecies::Orc
    ) {
        return Err(JourneyRuleError::InvalidChoice);
    }
    for (group, expected) in [
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
    let array_id = choice("array")?;
    let mut abilities = rules
        .ability_arrays
        .iter()
        .find(|array| array.id == array_id)
        .map(|array| array.scores)
        .ok_or(JourneyRuleError::InvalidChoice)?;
    abilities[0] += rules.background_bonus.strength;
    abilities[2] += rules.background_bonus.constitution;
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
        // SRD 5.2.1 pp. 47, 84: Fighter's d10 maximum plus Constitution and Dwarven Toughness.
        hit_points: (i32::from(rules.base_hit_points)
            + ability_modifier(abilities[2])
            + i32::from(species.combat_traits().hit_points_per_level)) as u32,
        armor_class: u32::from(rules.armor_class),
        proficiency_bonus: u32::from(rules.proficiency_bonus),
        second_wind_uses: u32::from(rules.second_wind_uses_max),
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
    pub critical_threshold: u8,
    pub damage_sides: u8,
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
    if !(15..=20).contains(&input.critical_threshold) || !permitted_damage_sides(input.damage_sides)
    {
        return Err(JourneyRuleError::InvalidRules);
    }
    if !(1..=ATTACK_DIE_SIDES).contains(&input.die) {
        return Err(JourneyRuleError::InvalidDie);
    }
    if input.target_hit_points == 0
        || !(MINIMUM_WEAPON_DICE..=MAXIMUM_WEAPON_DICE).contains(&input.weapon_dice)
    {
        return Err(JourneyRuleError::InvalidTarget);
    }
    let total = input.die as i32 + input.modifier;
    let critical = input.die >= u32::from(input.critical_threshold);
    let hit = input.die != NATURAL_MISS && (critical || total >= input.armor_class as i32);
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
        || input
            .damage_dice
            .iter()
            .any(|die| !(1..=u32::from(input.damage_sides)).contains(die))
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
            KNOCKOUT_HIT_POINTS
        } else {
            input.target_hit_points - damage
        },
        knocked_out,
        grazed,
    })
}
pub fn heal_second_wind(
    rules: &SecondWindRules,
    die: u32,
    current: u32,
    maximum: u32,
    uses: u32,
) -> Result<(u32, u32), JourneyRuleError> {
    rules.validate()?;
    if !(1..=u32::from(rules.die_sides)).contains(&die) {
        return Err(JourneyRuleError::InvalidDie);
    }
    if uses == 0 {
        return Err(JourneyRuleError::ResourceMissing);
    }
    if maximum == 0 || current == 0 || current > maximum {
        return Err(JourneyRuleError::InvalidTarget);
    }
    let healed = u64::from(current) + u64::from(die) + u64::from(rules.flat_bonus);
    Ok((u64::from(maximum).min(healed) as u32, uses - 1))
}

/// Configured rest duration, expressed in admitted campaign ticks.
/// This does not advance time or supply healing/dice.
pub fn short_rest_due(
    started: u64,
    ticks_per_second: u32,
    short_rest_seconds: u32,
) -> Result<u64, JourneyRuleError> {
    RestRules { short_rest_seconds }.validate()?;
    if ticks_per_second == 0 {
        return Err(JourneyRuleError::InvalidRest);
    }
    started
        .checked_add(u64::from(ticks_per_second) * u64::from(short_rest_seconds))
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
/// Selected no-Hit-Dice rest: unchanged HP, configured capped Second Wind recharge,
/// and source p17 knockout unconsciousness ends. Prone/equipment stay with the caller.
pub fn complete_no_hit_dice_short_rest(
    rules: &RestRules,
    input: NoHitDiceShortRest,
    uses_max: u8,
    recharge: u8,
) -> Result<ShortRestBenefits, JourneyRuleError> {
    rules.validate()?;
    if uses_max > 5 || recharge > uses_max {
        return Err(JourneyRuleError::InvalidRules);
    }
    if input.interrupted {
        return Err(JourneyRuleError::RestInterrupted);
    }
    if input.hit_points == 0
        || input
            .fighter_second_wind_uses
            .is_some_and(|uses| uses > u32::from(uses_max))
    {
        return Err(JourneyRuleError::InvalidRest);
    }
    if input.finished
        < short_rest_due(
            input.started,
            input.ticks_per_second,
            rules.short_rest_seconds,
        )?
    {
        return Err(JourneyRuleError::RestIncomplete);
    }
    Ok(ShortRestBenefits {
        hit_points: input.hit_points,
        fighter_second_wind_uses: input
            .fighter_second_wind_uses
            .map(|uses| u32::from(uses_max).min(uses + u32::from(recharge))),
        ends_knockout_unconscious: input.knockout_unconscious,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn srd_p17_p47_p187_short_rest_hit_point_die_adds_modifier_minimum_and_cap() {
        assert_eq!(heal_hit_point_die(5, 2, 3, 13), Ok((10, 7)));
        assert_eq!(heal_hit_point_die(1, -5, 7, 13), Ok((8, 1)));
        assert_eq!(heal_hit_point_die(10, 5, 12, 13), Ok((13, 1)));
    }
    #[test]
    fn srd_p47_p187_short_rest_hit_point_die_rejects_zero_hp_full_hp_and_invalid_faces() {
        assert_eq!(
            heal_hit_point_die(5, 2, 0, 13),
            Err(JourneyRuleError::InvalidTarget)
        );
        assert_eq!(
            heal_hit_point_die(5, 2, 13, 13),
            Err(JourneyRuleError::InvalidTarget)
        );
        assert_eq!(
            heal_hit_point_die(0, 2, 3, 13),
            Err(JourneyRuleError::InvalidDie)
        );
        assert_eq!(
            heal_hit_point_die(11, 2, 3, 13),
            Err(JourneyRuleError::InvalidDie)
        );
    }
    struct ExpectedSrdCreatureCore {
        id: &'static str,
        weapon_label: &'static str,
        page: u16,
        size: crate::tactical::Size,
        speed_ft: u16,
        weapons: &'static [crate::tactical::Weapon],
        shield: bool,
        hit_points: u8,
        armor_class: u8,
        abilities: [u8; 6],
        attack_bonus: i32,
        damage_modifier: i32,
        damage_dice: (u8, u8),
    }

    fn assert_srd_creature_core(expected: ExpectedSrdCreatureCore) {
        let creature = crate::SRD_521_CREATURES
            .iter()
            .find(|creature| creature.id == expected.id)
            .expect("compiled SRD creature");
        let tactical = crate::tactical::CombatantSpec::creature(
            3,
            expected.id,
            crate::tactical::Cell(2, 1),
            0,
        )
        .expect("compiled tactical creature");
        assert_eq!(creature.weapon_label, expected.weapon_label);
        assert_eq!(creature.page, expected.page);
        assert_eq!(tactical.size, expected.size);
        assert_eq!(tactical.speed_ft, expected.speed_ft);
        assert_eq!(tactical.weapons.as_slice(), expected.weapons);
        assert_eq!(tactical.shield, expected.shield);
        assert_eq!(creature.rules.hit_points, expected.hit_points);
        assert_eq!(creature.rules.armor_class, expected.armor_class);
        assert_eq!(creature.rules.abilities, expected.abilities);
        assert_eq!(creature.rules.attack_bonus, expected.attack_bonus);
        assert_eq!(creature.rules.damage_modifier, expected.damage_modifier);
        assert_eq!(
            (
                creature.rules.weapon.damage_dice,
                creature.rules.weapon.damage_sides,
            ),
            expected.damage_dice
        );
    }

    // SRD 5.2.1, p. 261. Damage modifier is the Scimitar's +1.
    #[test]
    fn bandit_core_stat_block_values_srd_521_p261() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "bandit",
            weapon_label: "scimitar",
            page: 261,
            size: crate::tactical::Size::Medium,
            speed_ft: 30,
            weapons: &[
                crate::tactical::Weapon::Scimitar,
                crate::tactical::Weapon::LightCrossbow,
            ],
            shield: false,
            hit_points: 11,
            armor_class: 12,
            abilities: [11, 12, 12, 10, 10, 10],
            attack_bonus: 3,
            damage_modifier: 1,
            damage_dice: (1, 6),
        });
    }

    // SRD 5.2.1, p. 278. The code folds the Sickle's +1 Slashing and +1 Necrotic into +2.
    #[test]
    fn cultist_core_stat_block_values_srd_521_p278() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "cultist",
            weapon_label: "ritual sickle",
            page: 278,
            size: crate::tactical::Size::Medium,
            speed_ft: 30,
            weapons: &[crate::tactical::Weapon::Sickle],
            shield: false,
            hit_points: 9,
            armor_class: 12,
            abilities: [11, 12, 10, 10, 11, 10],
            attack_bonus: 3,
            damage_modifier: 2,
            damage_dice: (1, 4),
        });
    }

    // SRD 5.2.1, p. 296. The compiled melee action is the Spear.
    #[test]
    fn guard_core_stat_block_values_srd_521_p296() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "guard",
            weapon_label: "spear",
            page: 296,
            size: crate::tactical::Size::Medium,
            speed_ft: 30,
            weapons: &[crate::tactical::Weapon::Spear],
            shield: true,
            hit_points: 11,
            armor_class: 16,
            abilities: [13, 12, 12, 10, 11, 10],
            attack_bonus: 3,
            damage_modifier: 1,
            damage_dice: (1, 6),
        });
    }

    // SRD 5.2.1, p. 290. The compiled melee action is the Scimitar.
    #[test]
    fn goblin_warrior_core_stat_block_values_srd_521_p290() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "goblin-warrior",
            weapon_label: "scimitar",
            page: 290,
            size: crate::tactical::Size::Small,
            speed_ft: 30,
            weapons: &[
                crate::tactical::Weapon::Scimitar,
                crate::tactical::Weapon::Shortbow,
            ],
            shield: true,
            hit_points: 10,
            armor_class: 15,
            abilities: [8, 15, 10, 10, 8, 8],
            attack_bonus: 4,
            damage_modifier: 2,
            damage_dice: (1, 6),
        });
    }

    // SRD 5.2.1, p. 302. The compiled melee action is the Dagger.
    #[test]
    fn kobold_warrior_core_stat_block_values_srd_521_p302() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "kobold-warrior",
            weapon_label: "dagger",
            page: 302,
            size: crate::tactical::Size::Small,
            speed_ft: 30,
            weapons: &[crate::tactical::Weapon::Dagger],
            shield: false,
            hit_points: 7,
            armor_class: 14,
            abilities: [7, 15, 9, 8, 7, 8],
            attack_bonus: 4,
            damage_modifier: 2,
            damage_dice: (1, 4),
        });
    }

    // SRD 5.2.1, p. 325. The compiled melee action is the Shortsword.
    #[test]
    fn skeleton_core_stat_block_values_srd_521_p325() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "skeleton",
            weapon_label: "shortsword",
            page: 325,
            size: crate::tactical::Size::Medium,
            speed_ft: 30,
            weapons: &[
                crate::tactical::Weapon::Shortsword,
                crate::tactical::Weapon::Shortbow,
            ],
            shield: false,
            hit_points: 13,
            armor_class: 14,
            abilities: [10, 16, 15, 6, 8, 5],
            attack_bonus: 5,
            damage_modifier: 3,
            damage_dice: (1, 6),
        });
    }

    // SRD 5.2.1, p. 343. The compiled action is Slam.
    #[test]
    fn zombie_core_stat_block_values_srd_521_p343() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "zombie",
            weapon_label: "slam",
            page: 343,
            size: crate::tactical::Size::Medium,
            speed_ft: 20,
            weapons: &[crate::tactical::Weapon::Slam],
            shield: false,
            hit_points: 15,
            armor_class: 8,
            abilities: [13, 6, 16, 3, 6, 5],
            attack_bonus: 3,
            damage_modifier: 1,
            damage_dice: (1, 8),
        });
    }

    // SRD 5.2.1, p. 364. The compiled action is Bite.
    #[test]
    fn wolf_core_stat_block_values_srd_521_p364() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "wolf",
            weapon_label: "bite",
            page: 364,
            size: crate::tactical::Size::Medium,
            speed_ft: 40,
            weapons: &[crate::tactical::Weapon::Bite],
            shield: false,
            hit_points: 11,
            armor_class: 12,
            abilities: [14, 15, 12, 3, 12, 6],
            attack_bonus: 4,
            damage_modifier: 2,
            damage_dice: (1, 6),
        });
    }

    // SRD 5.2.1, p. 353. The compiled action is Bite.
    #[test]
    fn giant_rat_core_stat_block_values_srd_521_p353() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "giant-rat",
            weapon_label: "bite",
            page: 353,
            size: crate::tactical::Size::Small,
            speed_ft: 30,
            weapons: &[crate::tactical::Weapon::Bite],
            shield: false,
            hit_points: 7,
            armor_class: 13,
            abilities: [7, 16, 11, 2, 10, 4],
            attack_bonus: 5,
            damage_modifier: 3,
            damage_dice: (1, 4),
        });
    }

    // SRD 5.2.1, p. 355. The compiled action is Bite.
    #[test]
    fn giant_weasel_core_stat_block_values_srd_521_p355() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "giant-weasel",
            weapon_label: "bite",
            page: 355,
            size: crate::tactical::Size::Medium,
            speed_ft: 40,
            weapons: &[crate::tactical::Weapon::Bite],
            shield: false,
            hit_points: 9,
            armor_class: 13,
            abilities: [11, 17, 10, 4, 12, 5],
            attack_bonus: 5,
            damage_modifier: 3,
            damage_dice: (1, 4),
        });
    }

    // SRD 5.2.1, p. 357. The compiled action is Bite.
    #[test]
    fn mastiff_core_stat_block_values_srd_521_p357() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "mastiff",
            weapon_label: "bite",
            page: 357,
            size: crate::tactical::Size::Medium,
            speed_ft: 40,
            weapons: &[crate::tactical::Weapon::Bite],
            shield: false,
            hit_points: 5,
            armor_class: 12,
            abilities: [13, 14, 12, 3, 12, 7],
            attack_bonus: 3,
            damage_modifier: 1,
            damage_dice: (1, 6),
        });
    }

    // SRD 5.2.1, p. 322. The compiled legacy attack is Shortsword.
    #[test]
    fn scout_core_stat_block_values_srd_521_p322() {
        assert_srd_creature_core(ExpectedSrdCreatureCore {
            id: "scout",
            weapon_label: "shortsword",
            page: 322,
            size: crate::tactical::Size::Medium,
            speed_ft: 30,
            weapons: &[
                crate::tactical::Weapon::Shortsword,
                crate::tactical::Weapon::Longbow,
            ],
            shield: false,
            hit_points: 16,
            armor_class: 13,
            abilities: [11, 14, 12, 11, 13, 11],
            attack_bonus: 4,
            damage_modifier: 2,
            damage_dice: (1, 6),
        });
    }

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
        let built = validate_character(&CreationRules::srd_521(), "Brynn", &values)
            .expect("source-legal selection");
        assert_eq!(built.abilities, [17, 13, 15, 10, 12, 8]);
        assert_eq!(
            (built.hit_points, built.armor_class, built.proficiency_bonus),
            (13, 17, 2)
        );
        values[8].1 = "perception";
        assert_eq!(
            validate_character(&CreationRules::srd_521(), "Brynn", &values),
            Err(JourneyRuleError::InvalidChoice)
        );
        values[8].1 = "athletics"; // Soldier already provides it.
        assert_eq!(
            validate_character(&CreationRules::srd_521(), "Brynn", &values),
            Err(JourneyRuleError::InvalidChoice)
        );
    }
    #[test]
    fn critical_doubles_weapon_dice_and_knockout_precedes_zero() {
        let result = resolve_melee(NormalMeleeAttack {
            critical_threshold: CombatRules::srd_521().critical_threshold,
            damage_sides: SRD_521_BASELINE.hero_weapon.damage_sides,
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
            critical_threshold: CombatRules::srd_521().critical_threshold,
            damage_sides: SRD_521_BASELINE.hero_weapon.damage_sides,
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
        assert_eq!(
            heal_second_wind(&SecondWindRules::srd_521(), 10, 5, 13, 2),
            Ok((13, 1))
        );
        assert_eq!(
            heal_second_wind(&SecondWindRules::srd_521(), 1, 13, 13, 0),
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
            complete_no_hit_dice_short_rest(
                &RestRules::srd_521(),
                NoHitDiceShortRest {
                    finished: 3605,
                    ..rest
                },
                SecondWindRules::srd_521().uses_max,
                SecondWindRules::srd_521().recharge_per_short_rest
            ),
            Err(JourneyRuleError::RestIncomplete)
        );
        assert_eq!(
            complete_no_hit_dice_short_rest(
                &RestRules::srd_521(),
                NoHitDiceShortRest {
                    interrupted: true,
                    ..rest
                },
                SecondWindRules::srd_521().uses_max,
                SecondWindRules::srd_521().recharge_per_short_rest
            ),
            Err(JourneyRuleError::RestInterrupted)
        );
        assert_eq!(
            complete_no_hit_dice_short_rest(
                &RestRules::srd_521(),
                NoHitDiceShortRest {
                    hit_points: 0,
                    ..rest
                },
                SecondWindRules::srd_521().uses_max,
                SecondWindRules::srd_521().recharge_per_short_rest
            ),
            Err(JourneyRuleError::InvalidRest)
        );
        let result = complete_no_hit_dice_short_rest(
            &RestRules::srd_521(),
            rest,
            SecondWindRules::srd_521().uses_max,
            SecondWindRules::srd_521().recharge_per_short_rest,
        )
        .unwrap();
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
                complete_no_hit_dice_short_rest(
                    &RestRules::srd_521(),
                    NoHitDiceShortRest {
                        fighter_second_wind_uses: Some(uses),
                        ..rest
                    },
                    SecondWindRules::srd_521().uses_max,
                    SecondWindRules::srd_521().recharge_per_short_rest
                )
                .unwrap()
                .fighter_second_wind_uses,
                Some(2)
            );
        }
        assert_eq!(
            short_rest_due(6, 2, RestRules::srd_521().short_rest_seconds),
            Ok(7206)
        );
        assert_eq!(
            short_rest_due(u64::MAX, 1, RestRules::srd_521().short_rest_seconds),
            Err(JourneyRuleError::InvalidRest)
        );
        assert_eq!(
            short_rest_due(0, 0, RestRules::srd_521().short_rest_seconds),
            Err(JourneyRuleError::InvalidRest)
        );
    }
    #[test]
    fn baseline_arrays_preserve_both_character_sheets_and_owned_choices() {
        let mut selected = choices();
        for (array, abilities) in [
            ("stalwart", [17, 13, 15, 10, 12, 8]),
            ("vanguard", [17, 14, 14, 8, 10, 12]),
        ] {
            selected[3].1 = array;
            assert_eq!(
                validate_character(&CreationRules::srd_521(), "Brynn", &selected),
                Ok(LegalCharacter {
                    name: "Brynn".to_owned(),
                    abilities,
                    choices: selected
                        .iter()
                        .map(|(group, option)| (group.to_string(), option.to_string()))
                        .collect(),
                    hit_points: 13,
                    armor_class: 17,
                    proficiency_bonus: 2,
                    second_wind_uses: 2,
                })
            );
        }
    }
    #[test]
    fn admitted_species_apply_their_own_level_one_toughness_and_unsupported_species_refuse() {
        let mut selected = choices();
        for (species, expected_hp) in [("dwarf", 13), ("halfling", 12), ("orc", 12)] {
            selected[0].1 = species;
            let character = validate_character(&CreationRules::srd_521(), "Brynn", &selected)
                .expect("admitted species uses its own traits");
            assert_eq!(character.hit_points, expected_hp, "{species}");
        }
        selected[0].1 = "dragonborn";
        assert_eq!(
            validate_character(&CreationRules::srd_521(), "Brynn", &selected),
            Err(JourneyRuleError::InvalidChoice)
        );
    }
    #[test]
    fn background_origin_feats_match_the_srd_choices() {
        assert_eq!(
            background_origin_feat("acolyte"),
            Some(OriginFeat::MagicInitiate)
        );
        assert_eq!(background_origin_feat("criminal"), Some(OriginFeat::Alert));
        assert_eq!(
            background_origin_feat("sage"),
            Some(OriginFeat::MagicInitiate)
        );
        assert_eq!(
            background_origin_feat("soldier"),
            Some(OriginFeat::SavageAttacker)
        );
        assert_eq!(background_origin_feat("unknown"), None);
    }
    #[test]
    fn srd_521_page_256_challenge_rating_xp_table_is_exact_and_cr_zero_stays_ambiguous() {
        for (cr, xp) in [
            (ChallengeRating::Eighth, 25),
            (ChallengeRating::Quarter, 50),
            (ChallengeRating::Half, 100),
            (ChallengeRating::Whole(1), 200),
            (ChallengeRating::Whole(3), 700),
            (ChallengeRating::Whole(10), 5_900),
            (ChallengeRating::Whole(30), 155_000),
        ] {
            assert_eq!(experience_points_for_challenge_rating(cr), Some(xp));
        }
        assert_eq!(
            experience_points_for_challenge_rating(ChallengeRating::Zero),
            None
        );
        assert_eq!(
            experience_points_for_challenge_rating(ChallengeRating::Whole(31)),
            None
        );
    }
    #[test]
    fn creation_numbers_and_array_subset_change_the_character() {
        let arrays = [AbilityArray {
            id: "stalwart",
            scores: [16, 13, 16, 10, 12, 8],
        }];
        let rules = CreationRules {
            ability_arrays: &arrays,
            background_bonus: BackgroundBonus {
                strength: 1,
                constitution: 2,
            },
            base_hit_points: 20,
            armor_class: 19,
            proficiency_bonus: 4,
            second_wind_uses_max: 3,
        };
        let mut selected = choices();
        let built = validate_character(&rules, "Brynn", &selected).unwrap();
        assert_eq!(built.abilities, [17, 13, 18, 10, 12, 8]);
        assert_eq!(
            (
                built.hit_points,
                built.armor_class,
                built.proficiency_bonus,
                built.second_wind_uses
            ),
            (25, 19, 4, 3)
        );
        selected[3].1 = "vanguard";
        assert_eq!(
            validate_character(&rules, "Brynn", &selected),
            Err(JourneyRuleError::InvalidChoice)
        );
    }
    #[test]
    fn creation_rejects_invalid_parameter_bounds_and_array_ids() {
        let baseline = CreationRules::srd_521();
        for rules in [
            CreationRules {
                base_hit_points: 0,
                ..baseline
            },
            CreationRules {
                base_hit_points: 61,
                ..baseline
            },
            CreationRules {
                armor_class: 9,
                ..baseline
            },
            CreationRules {
                armor_class: 31,
                ..baseline
            },
            CreationRules {
                proficiency_bonus: 1,
                ..baseline
            },
            CreationRules {
                proficiency_bonus: 7,
                ..baseline
            },
            CreationRules {
                second_wind_uses_max: 6,
                ..baseline
            },
            CreationRules {
                background_bonus: BackgroundBonus {
                    strength: 5,
                    constitution: 1,
                },
                ..baseline
            },
            CreationRules {
                background_bonus: BackgroundBonus {
                    strength: 2,
                    constitution: 5,
                },
                ..baseline
            },
            CreationRules {
                ability_arrays: &[],
                ..baseline
            },
        ] {
            assert_eq!(
                validate_character(&rules, "Brynn", &choices()),
                Err(JourneyRuleError::InvalidRules)
            );
        }
        for arrays in [
            vec![AbilityArray {
                id: "unknown",
                scores: [10; 6],
            }],
            vec![
                AbilityArray {
                    id: "stalwart",
                    scores: [10; 6]
                };
                2
            ],
            vec![AbilityArray {
                id: "stalwart",
                scores: [2, 10, 10, 10, 10, 10],
            }],
            vec![AbilityArray {
                id: "stalwart",
                scores: [19, 10, 10, 10, 10, 10],
            }],
            vec![AbilityArray {
                id: "stalwart",
                scores: [255; 6],
            }],
            vec![AbilityArray {
                id: "stalwart",
                scores: [18; 6],
            }],
        ] {
            let rules = CreationRules {
                ability_arrays: &arrays,
                background_bonus: BackgroundBonus {
                    strength: 4,
                    constitution: 4,
                },
                ..baseline
            };
            assert_eq!(rules.validate(), Err(JourneyRuleError::InvalidRules));
        }
        let low_constitution = [AbilityArray {
            id: "stalwart",
            scores: [10, 10, 3, 10, 10, 10],
        }];
        assert_eq!(
            CreationRules {
                ability_arrays: &low_constitution,
                base_hit_points: 1,
                ..baseline
            }
            .validate(),
            Err(JourneyRuleError::InvalidRules)
        );
    }
    #[test]
    fn baseline_character_rejections_keep_their_typed_errors() {
        let rules = CreationRules::srd_521();
        for name in [
            "",
            "123",
            "R2-D2",
            " Vix",
            "Vix ",
            "Vix  Ash",
            "Vix--Ash",
            "-Vix",
            "Vix!",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFG",
        ] {
            assert_eq!(
                validate_character(&rules, name, &choices()),
                Err(JourneyRuleError::InvalidName),
                "{name:?}"
            );
        }
    }
    #[test]
    fn hero_names_may_be_simple_or_several_words_in_any_alphabet() {
        let rules = CreationRules::srd_521();
        for name in [
            "Vix",
            "Vix Ashlantern",
            "Thorin Oakenshield II",
            "D'Artagnan",
            "Ka\u{2019}rel",
            "Jean-Luc",
            "St. Brynn",
            "Brynn Jr.",
            "Åsa",
            "Ælfwyn of the Red_Hill",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEF",
        ] {
            assert!(
                validate_character(&rules, name, &choices()).is_ok(),
                "{name:?}"
            );
        }
        assert_eq!(
            validate_character(&rules, "Brynn", &[]),
            Err(JourneyRuleError::MissingChoice)
        );
        let mut selected = choices();
        selected[1] = selected[0];
        assert_eq!(
            validate_character(&rules, "Brynn", &selected),
            Err(JourneyRuleError::DuplicateChoice)
        );
        selected = choices();
        selected[0].1 = "human";
        assert_eq!(
            validate_character(&rules, "Brynn", &selected),
            Err(JourneyRuleError::InvalidChoice)
        );
        selected = choices();
        selected[6].1 = selected[5].1;
        assert_eq!(
            validate_character(&rules, "Brynn", &selected),
            Err(JourneyRuleError::InvalidChoice)
        );
    }
    fn ordinary_attack() -> NormalMeleeAttack<'static> {
        NormalMeleeAttack {
            critical_threshold: CombatRules::srd_521().critical_threshold,
            damage_sides: SRD_521_BASELINE.hero_weapon.damage_sides,
            die: 15,
            modifier: 5,
            armor_class: 12,
            damage_dice: &[6, 5],
            weapon_dice: 2,
            damage_modifier: 3,
            savage_attacker_take_higher: false,
            graze: false,
            target_hit_points: 20,
        }
    }
    #[test]
    fn baseline_melee_preserves_the_entire_outcome_and_invalid_inputs() {
        assert_eq!(
            resolve_melee(ordinary_attack()),
            Ok(AttackOutcome {
                die: 15,
                modifier: 5,
                total: 20,
                armor_class: 12,
                hit: true,
                critical: false,
                damage_dice: vec![6, 5],
                damage: 14,
                target_hit_points: 6,
                knocked_out: false,
                grazed: false,
            })
        );
        for die in [0, 21] {
            assert_eq!(
                resolve_melee(NormalMeleeAttack {
                    die,
                    ..ordinary_attack()
                }),
                Err(JourneyRuleError::InvalidDie)
            );
        }
        for weapon_dice in [0, 3] {
            assert_eq!(
                resolve_melee(NormalMeleeAttack {
                    weapon_dice,
                    ..ordinary_attack()
                }),
                Err(JourneyRuleError::InvalidTarget)
            );
        }
        for damage_dice in [&[0, 5][..], &[7, 5], &[6]] {
            assert_eq!(
                resolve_melee(NormalMeleeAttack {
                    damage_dice,
                    ..ordinary_attack()
                }),
                Err(JourneyRuleError::InvalidDamageDice)
            );
        }
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                target_hit_points: 0,
                ..ordinary_attack()
            }),
            Err(JourneyRuleError::InvalidTarget)
        );
    }
    #[test]
    fn configured_weapon_faces_counts_and_critical_threshold_change_damage() {
        for damage_sides in [4, 6, 8, 10, 12] {
            let dice = [u32::from(damage_sides)];
            let result = resolve_melee(NormalMeleeAttack {
                damage_sides,
                damage_dice: &dice,
                weapon_dice: 1,
                ..ordinary_attack()
            })
            .unwrap();
            assert_eq!(result.damage, u32::from(damage_sides) + 3);
        }
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                damage_sides: 8,
                damage_dice: &[8, 7],
                ..ordinary_attack()
            })
            .unwrap()
            .damage,
            18
        );
        assert_eq!(
            resolve_melee(NormalMeleeAttack {
                damage_dice: &[8, 7],
                ..ordinary_attack()
            }),
            Err(JourneyRuleError::InvalidDamageDice)
        );
        let result = resolve_melee(NormalMeleeAttack {
            die: 19,
            critical_threshold: 19,
            damage_dice: &[6, 5, 4, 3],
            armor_class: 100,
            ..ordinary_attack()
        })
        .unwrap();
        assert!(result.hit && result.critical);
        assert_eq!(result.damage, 21);
        for critical_threshold in [14, 21] {
            assert_eq!(
                resolve_melee(NormalMeleeAttack {
                    critical_threshold,
                    ..ordinary_attack()
                }),
                Err(JourneyRuleError::InvalidRules)
            );
        }
        for damage_sides in [0, 5, 20] {
            assert_eq!(
                resolve_melee(NormalMeleeAttack {
                    damage_sides,
                    ..ordinary_attack()
                }),
                Err(JourneyRuleError::InvalidRules)
            );
        }
    }
    #[test]
    fn second_wind_die_bonus_and_boundaries_are_explicit() {
        let baseline = SecondWindRules::srd_521();
        let changed = SecondWindRules {
            die_sides: 6,
            flat_bonus: 4,
            ..baseline
        };
        assert_eq!(heal_second_wind(&baseline, 6, 1, 20, 2), Ok((8, 1)));
        assert_eq!(heal_second_wind(&changed, 6, 1, 20, 2), Ok((11, 1)));
        assert_eq!(
            heal_second_wind(&changed, 7, 1, 20, 2),
            Err(JourneyRuleError::InvalidDie)
        );
        for rules in [
            SecondWindRules {
                uses_max: 6,
                ..baseline
            },
            SecondWindRules {
                die_sides: 20,
                ..baseline
            },
            SecondWindRules {
                flat_bonus: 11,
                ..baseline
            },
            SecondWindRules {
                recharge_per_short_rest: 3,
                ..baseline
            },
        ] {
            assert_eq!(
                heal_second_wind(&rules, 1, 1, 20, 2),
                Err(JourneyRuleError::InvalidRules)
            );
        }
        for (current, maximum) in [(0, 13), (14, 13), (1, 0)] {
            assert_eq!(
                heal_second_wind(&baseline, 1, current, maximum, 2),
                Err(JourneyRuleError::InvalidTarget)
            );
        }
        assert_eq!(
            heal_second_wind(&baseline, 0, 1, 13, 2),
            Err(JourneyRuleError::InvalidDie)
        );
        assert_eq!(
            heal_second_wind(&baseline, 11, 1, 13, 2),
            Err(JourneyRuleError::InvalidDie)
        );
        assert_eq!(
            heal_second_wind(&baseline, 10, u32::MAX, u32::MAX, 2),
            Ok((u32::MAX, 1))
        );
    }
    #[test]
    fn configured_rest_duration_and_recharge_change_only_selected_benefits() {
        let rules = RestRules {
            short_rest_seconds: 60,
        };
        let rest = NoHitDiceShortRest {
            started: 6,
            finished: 66,
            ticks_per_second: 1,
            interrupted: false,
            hit_points: 1,
            fighter_second_wind_uses: Some(1),
            knockout_unconscious: true,
        };
        assert_eq!(short_rest_due(6, 2, rules.short_rest_seconds), Ok(126));
        assert_eq!(
            complete_no_hit_dice_short_rest(&RestRules::srd_521(), rest, 2, 1),
            Err(JourneyRuleError::RestIncomplete)
        );
        assert_eq!(
            complete_no_hit_dice_short_rest(&rules, rest, 4, 2),
            Ok(ShortRestBenefits {
                hit_points: 1,
                fighter_second_wind_uses: Some(3),
                ends_knockout_unconscious: true,
            })
        );
        assert_eq!(
            complete_no_hit_dice_short_rest(&rules, rest, 2, 0)
                .unwrap()
                .fighter_second_wind_uses,
            Some(1)
        );
        assert_eq!(
            complete_no_hit_dice_short_rest(
                &rules,
                NoHitDiceShortRest {
                    fighter_second_wind_uses: None,
                    ..rest
                },
                2,
                1
            )
            .unwrap()
            .fighter_second_wind_uses,
            None
        );
        for seconds in [59, 86401] {
            assert_eq!(
                short_rest_due(0, 1, seconds),
                Err(JourneyRuleError::InvalidRules)
            );
        }
        for (uses_max, recharge) in [(6, 1), (2, 3)] {
            assert_eq!(
                complete_no_hit_dice_short_rest(&rules, rest, uses_max, recharge),
                Err(JourneyRuleError::InvalidRules)
            );
        }
        assert_eq!(
            complete_no_hit_dice_short_rest(
                &rules,
                NoHitDiceShortRest {
                    fighter_second_wind_uses: Some(3),
                    ..rest
                },
                2,
                1
            ),
            Err(JourneyRuleError::InvalidRest)
        );
    }
    #[test]
    fn combat_configuration_rejects_each_out_of_bounds_field() {
        let baseline = CombatRules::srd_521();
        for rules in [
            CombatRules {
                critical_threshold: 14,
                ..baseline
            },
            CombatRules {
                critical_threshold: 21,
                ..baseline
            },
            CombatRules {
                round_ticks: 0,
                ..baseline
            },
            CombatRules {
                round_ticks: 601,
                ..baseline
            },
            CombatRules {
                combat_log_entries: 0,
                ..baseline
            },
            CombatRules {
                combat_log_entries: 33,
                ..baseline
            },
        ] {
            assert_eq!(rules.validate(), Err(JourneyRuleError::InvalidRules));
        }
        assert_eq!(baseline.validate(), Ok(()));
        assert_eq!(SecondWindRules::srd_521().validate(), Ok(()));
        assert_eq!(RestRules::srd_521().validate(), Ok(()));
    }
}
