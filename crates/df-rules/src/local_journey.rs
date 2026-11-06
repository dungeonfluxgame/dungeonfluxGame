//! Selected SRD 5.2.1 creation and normal adjacent melee mechanics.
//! This is an explicitly bounded playable subset, not full 2024 rules coverage.
use serde::{Deserialize, Serialize};

/// Most heroes in one party; encounter staging admits up to this many heroes plus one foe.
pub const MAXIMUM_PARTY: usize = 6;

/// SRD 5.2.1, Equipment, "Selling Equipment", printed page 89. Equipment sells
/// for half its cost; trade goods and valuables retain full value. Fractions of CP round down.
pub const fn selling_value_cp(cost_cp: u32, retains_full_value: bool) -> u32 {
    if retains_full_value {
        cost_cp
    } else {
        cost_cp / 2
    }
}

/// Pinned revision of this creation and melee subset.
pub const SOURCE_REVISION: &str = "SRD5.2.1-2024-local-journey-3";
/// Source attribution, licence, PDF checksum and the pages this subset implements.
pub const SOURCE: &str = "Wizards of the Coast LLC SRD 5.2.1 English; \
    https://media.dndbeyond.com/compendium-images/srd/5.2/SRD_CC_v5.2.1.pdf; CC BY4.0; PDF SHA256 \
    8974902d109d6e63672d7c490bde9ccf052410503d9cfa768237154fbc5e3d87; creation pages19-22; \
    Fighter47-48; Soldier83; species84-86; Origin feats83,87-88; Defense88; Graze90; weapons91; \
    armor92; initiative13; attack7,14; damage16; knockout17; ShortRest187; Unconscious191; \
    Bandit261. Selected level1 SRD species Fighter/Soldier; Defense; Greatsword/Flail/Javelin or \
    archer equipment; standard arrays; normal adjacent attack; SecondWind; nonlethal melee \
    encounter; uninterrupted after-combat ShortRest without HitPointDice; no full-rest or general \
    rest catalog.";

/// Why a creation, melee, healing or rest request was refused.
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

/// The nine SRD 5.2.1 species a hero may have (pp. 84-86).
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

/// The size a species grants or a Human or Tiefling chooses.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeciesSize {
    Small,
    Medium,
}

/// The species traits combat reads, after any lineage or size choice.
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
    /// Every species a creator may pick, in listing order.
    pub(crate) const ALL: [HeroSpecies; 9] = [
        Self::Dragonborn,
        Self::Dwarf,
        Self::Elf,
        Self::Gnome,
        Self::Goliath,
        Self::Halfling,
        Self::Human,
        Self::Orc,
        Self::Tiefling,
    ];

    /// The species a creation choice id names, if any.
    pub fn from_choice(choice_id: &str) -> Option<Self> {
        Some(match choice_id {
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

    /// The creation choice id of this species.
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

    /// Base combat traits from SRD 5.2.1 pp. 84-86. SpeciesOptions applies source
    /// lineage and size choices; features without handlers remain explicit coverage gaps.
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
            // "Your Hit Point maximum increases by 1, and it increases by 1 again whenever you gain
            // a level."
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
            // SRD 5.2.1 p. 86: "When you roll a 1 on the d20 of a D20 Test, you can reroll the die,
            // and you must use the new roll."
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

/// The Origin feats a background grants (SRD p. 83).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginFeat {
    Alert,
    MagicInitiate,
    SavageAttacker,
}

/// SRD 5.2.1 p. 83: Acolyte/Sage list "Feat: Magic Initiate," Criminal "Feat: Alert," and Soldier
/// "Feat: Savage Attacker."
pub fn background_origin_feat(background: &str) -> Option<OriginFeat> {
    Some(match background {
        "acolyte" | "sage" => OriginFeat::MagicInitiate,
        "criminal" => OriginFeat::Alert,
        "soldier" => OriginFeat::SavageAttacker,
        _ => return None,
    })
}

/// A monster Challenge Rating: the three fractions or a whole number.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChallengeRating {
    Zero,
    Eighth,
    Quarter,
    Half,
    Whole(u8),
}

/// SRD 5.2.1 p. 256: "The number of Experience Points (XP) a monster is worth is based on its CR."
/// CR 0 is 0 or 10 XP.
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
        ChallengeRating::Whole(whole_challenge_rating) => {
            *WHOLE_CR_XP.get(usize::from(whole_challenge_rating.checked_sub(1)?))?
        }
    })
}
/// A hero admitted by `validate_character`, with its derived sheet numbers.
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
    /// The progression at `level`; only levels 1 to 5 are supported.
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
    /// Whether the Champion subclass applies (level 3 and up).
    pub fn champion(self) -> bool {
        self.level >= 3
    }
    /// Maximum hit points at this level with the species' base bonus (no Tough).
    pub fn hit_points(self, constitution: u8, species: HeroSpecies) -> u16 {
        maximum_hit_points(
            self.level,
            FIGHTER_FIRST_LEVEL_HIT_POINTS,
            constitution,
            species.combat_traits().hit_points_per_level,
        ) as u16
    }
    /// Advance an accepted level-one receipt using this build's fixed source-legal ASI.
    pub fn apply(self, character: &mut LegalCharacter) -> Result<(), JourneyRuleError> {
        let species = character
            .choices
            .iter()
            .find(|(key, _)| key == "species")
            .and_then(|(_, species_id)| HeroSpecies::from_choice(species_id))
            .ok_or(JourneyRuleError::InvalidChoice)?;
        let archer = character
            .choices
            .iter()
            .any(|(key, choice_value)| key == "array" && choice_value == "dexterous");
        if self.level >= 4 {
            let ability = if archer { 1 } else { 0 };
            if character.abilities[ability] > 18 {
                return Err(JourneyRuleError::InvalidChoice);
            }
            character.abilities[ability] += 2;
        }
        let choices = character
            .choices
            .iter()
            .map(|(key, choice_value)| (key.as_str(), choice_value.as_str()))
            .collect::<Vec<_>>();
        let options = crate::species::SpeciesOptions::from_choices(species, &choices)?;
        character.hit_points = maximum_hit_points(
            self.level,
            FIGHTER_FIRST_LEVEL_HIT_POINTS,
            character.abilities[2],
            options.combat_traits(species).hit_points_per_level,
        );
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

/// The Fighter skill choices (SRD p. 47) a creator picks two from.
pub const FIGHTER_SKILLS: &[&str] = &[
    "acrobatics",
    "animal-handling",
    "history",
    "insight",
    "persuasion",
    "perception",
    "survival",
];
/// The standard languages (SRD p. 20) a creator picks two from.
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
/// The alignments a creator may pick (SRD p. 21).
pub const ALIGNMENTS: &[&str] = &[
    "lawful-good",
    "neutral-good",
    "chaotic-good",
    "lawful-neutral",
    "neutral",
    "chaotic-neutral",
];
/// The gaming sets the Soldier background offers.
pub const GAMING_SETS: &[&str] = &["dice", "dragonchess", "playing-cards", "three-dragon-ante"];
/// Skills the Soldier background already grants (SRD p.83); a second grant of either is refused.
pub(crate) const SOLDIER_SKILLS: [&str; 2] = ["athletics", "intimidation"];
/// The single first-release offers (SRD pp.47, 83, 88); `validate_character` admits exactly these.
pub(crate) const CLASS_OFFER: &str = "fighter-1";
pub(crate) const BACKGROUND_OFFER: &str = "soldier";
pub(crate) const EQUIPMENT_OFFER: &str = "fighter-a-soldier-a";
pub(crate) const STYLE_OFFER: &str = "defense-greatsword-flail-javelin";
pub(crate) const TIES_OFFER: &str = "join-order";
/// Each build key with a single first-release offer, paired with that offer.
const FIXED_OFFERS: [(&str, &str); 5] = [
    ("class", CLASS_OFFER),
    ("background", BACKGROUND_OFFER),
    ("equipment", EQUIPMENT_OFFER),
    ("style-masteries", STYLE_OFFER),
    ("allied-ties", TIES_OFFER),
];
/// The build keys every creation request must fill, besides species choice groups.
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
/// The background's ability score increases to Strength and Constitution.
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
/// Second Wind numbers shared with the native caller.
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
/// A weapon's damage dice: how many and how many sides.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WeaponRules {
    pub damage_dice: u8,
    pub damage_sides: u8,
}
/// The stat-block numbers a single-opponent encounter reads for one creature.
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
/// The compiled SRD 5.2.1 values every tuning or pack field is compared against.
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

/// A hero name is one or more words of letters from any alphabet. Single spaces, apostrophes,
/// hyphens, periods and underscores may join letters ("Vix Ashlantern", "D'Artagnan", "Jean-Luc",
/// "St. Brynn"); the name starts with a letter and ends with a letter or a period.
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
    if choices.len() < BUILD_GROUPS.len() {
        return Err(JourneyRuleError::MissingChoice);
    }
    for (index, (group, _)) in choices.iter().enumerate() {
        if !BUILD_GROUPS.contains(group) && !crate::species::is_choice_group(group) {
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
    let species_options = crate::species::SpeciesOptions::from_choices(species, choices)?;
    for (group, expected) in FIXED_OFFERS {
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
        hit_points: maximum_hit_points(
            1,
            rules.base_hit_points,
            abilities[2],
            species_options.combat_traits(species).hit_points_per_level,
        ),
        armor_class: u32::from(rules.armor_class),
        proficiency_bonus: u32::from(rules.proficiency_bonus),
        second_wind_uses: u32::from(rules.second_wind_uses_max),
    })
}
/// A Fighter's d10 maximum, the first-level base the creation rules start from.
pub const FIGHTER_FIRST_LEVEL_HIT_POINTS: u8 = 10;

/// The one maximum hit point figure (SRD 5.2.1 pp. 47, 84, 87): the first-level base, the fixed 6
/// for each further Fighter level, and per level the Constitution modifier plus the species bonus
/// (Dwarven Toughness 1, Tough 2). Creation, level advancement and every bound derive from it.
pub fn maximum_hit_points(
    level: u8,
    first_level_base: u8,
    constitution: u8,
    species_bonus_per_level: u8,
) -> u32 {
    let per_level = ability_modifier(constitution) + i32::from(species_bonus_per_level);
    let advanced = 6 * i32::from(level.saturating_sub(1));
    (i32::from(first_level_base) + advanced + per_level * i32::from(level)) as u32
}
/// The ability modifier of a score: (score - 10) / 2, rounded down.
pub const fn ability_modifier(score: u8) -> i32 {
    (score as i32 - 10).div_euclid(2)
}

/// Order this local participant subset from supplied d20 faces and Dexterity scores.
/// Initiative uses the cited SRD d20 + Dexterity modifier; equal totals retain the
/// existing authored participant order. General player/DM tie choices remain outside
/// this subset. No draw, identity, state mutation or implicit random input occurs here.
pub fn initiative_order<const N: usize>(
    participants: [(u32, u8); N],
) -> Result<[usize; N], JourneyRuleError> {
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
    let mut order = std::array::from_fn(|index| index);
    // All indices are internal members of the fixed participant array.
    order.sort_by_key(|index| (std::cmp::Reverse(totals[*index]), *index));
    Ok(order)
}

/// Which of the turn's action and Bonus Action are spent.
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
/// Everything one resolved melee attack decided, for the table and the log.
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
/// The supplied faces and numbers of one normal melee attack.
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
    // The die is at most ATTACK_DIE_SIDES, checked above, so it fits in i32.
    let total = (input.die as i32)
        .checked_add(input.modifier)
        .ok_or(JourneyRuleError::InvalidRules)?;
    let critical = input.die >= u32::from(input.critical_threshold);
    let hit =
        input.die != NATURAL_MISS && (critical || i64::from(total) >= i64::from(input.armor_class));
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
        // Weapon dice total at most 4 x 255, so adding any i32 modifier cannot reach u32::MAX;
        // a negative total clamps to zero damage.
        weapon_damage.saturating_add_signed(input.damage_modifier)
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
/// Second Wind healing: the die plus the flat bonus, capped at the maximum; spends one use.
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
/// One Short Rest request in admitted campaign ticks, without Hit Point Dice.
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
/// What a completed no-Hit-Dice Short Rest grants.
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
#[path = "local_journey_tests.rs"]
mod tests;
