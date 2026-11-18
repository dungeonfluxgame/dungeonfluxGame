//! The 2024 core options a hero is built from, and the SRD 5.2.1 facts each one carries.
//!
//! Owns the closed lists of the core species, classes and backgrounds (the SRD 5.2.1 ones with
//! their creation facts, and the core ones the SRD does not print, by name only), the level 1
//! facts of each class (Core Traits tables, SRD pp. 28-77), the four SRD backgrounds (p. 83), the
//! weapons and armour of the starting kits (pp. 91-92), and the tools a build may name (pp. 93-95).
//! Never does: derive a number, read a choice, or offer an option the SRD does not print. A name
//! with no admitted source carries no facts (`HeroBackground::srd` and `CoreSpecies::srd` are
//! `None`), so nothing can be derived from it.
use crate::listed::listed_enum;
use crate::local_journey::{HeroSpecies, OriginFeat};
use crate::roll::{Ability, DieSize, Skill};

listed_enum! {
    keyed
    /// The species of the 2024 core rules. Aasimar is a core species the SRD 5.2.1 does not print.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum CoreSpecies {
        Aasimar,
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
}

impl CoreSpecies {
    pub fn key(self) -> &'static str {
        match self {
            Self::Aasimar => "aasimar",
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

    pub fn title(self) -> &'static str {
        match self {
            Self::Aasimar => "Aasimar",
            Self::Dragonborn => "Dragonborn",
            Self::Dwarf => "Dwarf",
            Self::Elf => "Elf",
            Self::Gnome => "Gnome",
            Self::Goliath => "Goliath",
            Self::Halfling => "Halfling",
            Self::Human => "Human",
            Self::Orc => "Orc",
            Self::Tiefling => "Tiefling",
        }
    }

    /// The SRD 5.2.1 species (pp. 84-86), or `None` for a species the SRD does not print.
    pub fn srd(self) -> Option<HeroSpecies> {
        HeroSpecies::from_key(self.key())
    }
}

listed_enum! {
    keyed
    /// The twelve classes of the 2024 core rules, all printed in SRD 5.2.1 (pp. 28-82).
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum HeroClass {
        Barbarian,
        Bard,
        Cleric,
        Druid,
        Fighter,
        Monk,
        Paladin,
        Ranger,
        Rogue,
        Sorcerer,
        Warlock,
        Wizard,
    }
}

/// The armour a class is trained with (Core Traits tables).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArmorTraining {
    pub light: bool,
    pub medium: bool,
    pub heavy: bool,
    pub shields: bool,
}

const NO_ARMOR: ArmorTraining = ArmorTraining {
    light: false,
    medium: false,
    heavy: false,
    shields: false,
};
const LIGHT: ArmorTraining = ArmorTraining {
    light: true,
    ..NO_ARMOR
};
const LIGHT_SHIELDS: ArmorTraining = ArmorTraining {
    shields: true,
    ..LIGHT
};
const LIGHT_MEDIUM_SHIELDS: ArmorTraining = ArmorTraining {
    medium: true,
    ..LIGHT_SHIELDS
};
const ALL_ARMOR: ArmorTraining = ArmorTraining {
    heavy: true,
    ..LIGHT_MEDIUM_SHIELDS
};

/// The tool proficiencies a class grants at level 1 (Core Traits tables).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassTools {
    None,
    /// One named tool (Druid: Herbalism Kit; Rogue: Thieves' Tools).
    Fixed(Tool),
    /// Three Musical Instruments of the player's choice (Bard).
    ThreeInstruments,
    /// One kind of Artisan's Tools or Musical Instrument of the player's choice (Monk).
    ArtisanOrInstrument,
}

impl HeroClass {
    pub fn key(self) -> &'static str {
        match self {
            Self::Barbarian => "barbarian",
            Self::Bard => "bard",
            Self::Cleric => "cleric",
            Self::Druid => "druid",
            Self::Fighter => "fighter",
            Self::Monk => "monk",
            Self::Paladin => "paladin",
            Self::Ranger => "ranger",
            Self::Rogue => "rogue",
            Self::Sorcerer => "sorcerer",
            Self::Warlock => "warlock",
            Self::Wizard => "wizard",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Barbarian => "Barbarian",
            Self::Bard => "Bard",
            Self::Cleric => "Cleric",
            Self::Druid => "Druid",
            Self::Fighter => "Fighter",
            Self::Monk => "Monk",
            Self::Paladin => "Paladin",
            Self::Ranger => "Ranger",
            Self::Rogue => "Rogue",
            Self::Sorcerer => "Sorcerer",
            Self::Warlock => "Warlock",
            Self::Wizard => "Wizard",
        }
    }

    /// The printed SRD page of the class's Core Traits table.
    pub fn page(self) -> u16 {
        match self {
            Self::Barbarian => 28,
            Self::Bard => 31,
            Self::Cleric => 36,
            Self::Druid => 41,
            Self::Fighter => 47,
            Self::Monk => 49,
            Self::Paladin => 53,
            Self::Ranger => 57,
            Self::Rogue => 61,
            Self::Sorcerer => 64,
            Self::Warlock => 70,
            Self::Wizard => 77,
        }
    }

    /// The Hit Point Die (Core Traits; the level 1 maximum is its largest face, SRD p. 23).
    pub fn hit_die(self) -> DieSize {
        match self {
            Self::Barbarian => DieSize::D12,
            Self::Fighter | Self::Paladin | Self::Ranger => DieSize::D10,
            Self::Bard | Self::Cleric | Self::Druid | Self::Monk | Self::Rogue | Self::Warlock => {
                DieSize::D8
            }
            Self::Sorcerer | Self::Wizard => DieSize::D6,
        }
    }

    pub fn saving_throws(self) -> [Ability; 2] {
        use Ability::{Charisma, Constitution, Dexterity, Intelligence, Strength, Wisdom};
        match self {
            Self::Barbarian | Self::Fighter => [Strength, Constitution],
            Self::Bard => [Dexterity, Charisma],
            Self::Cleric | Self::Paladin | Self::Warlock => [Wisdom, Charisma],
            Self::Druid | Self::Wizard => [Intelligence, Wisdom],
            Self::Monk | Self::Ranger => [Strength, Dexterity],
            Self::Rogue => [Dexterity, Intelligence],
            Self::Sorcerer => [Constitution, Charisma],
        }
    }

    /// How many skills the class lets the player choose.
    pub fn skill_count(self) -> usize {
        match self {
            Self::Bard | Self::Ranger => 3,
            Self::Rogue => 4,
            _ => 2,
        }
    }

    /// The skills the class's choice is made from (the Bard chooses any skill).
    pub fn skill_options(self) -> &'static [Skill] {
        use Skill::*;
        match self {
            Self::Barbarian => &[
                AnimalHandling,
                Athletics,
                Intimidation,
                Nature,
                Perception,
                Survival,
            ],
            Self::Bard => Skill::ALL,
            Self::Cleric => &[History, Insight, Medicine, Persuasion, Religion],
            Self::Druid => &[
                AnimalHandling,
                Arcana,
                Insight,
                Medicine,
                Nature,
                Perception,
                Religion,
                Survival,
            ],
            Self::Fighter => &[
                Acrobatics,
                AnimalHandling,
                Athletics,
                History,
                Insight,
                Intimidation,
                Persuasion,
                Perception,
                Survival,
            ],
            Self::Monk => &[Acrobatics, Athletics, History, Insight, Religion, Stealth],
            Self::Paladin => &[
                Athletics,
                Insight,
                Intimidation,
                Medicine,
                Persuasion,
                Religion,
            ],
            Self::Ranger => &[
                AnimalHandling,
                Athletics,
                Insight,
                Investigation,
                Nature,
                Perception,
                Stealth,
                Survival,
            ],
            Self::Rogue => &[
                Acrobatics,
                Athletics,
                Deception,
                Insight,
                Intimidation,
                Investigation,
                Perception,
                Persuasion,
                SleightOfHand,
                Stealth,
            ],
            Self::Sorcerer => &[
                Arcana,
                Deception,
                Insight,
                Intimidation,
                Persuasion,
                Religion,
            ],
            Self::Warlock => &[
                Arcana,
                Deception,
                History,
                Intimidation,
                Investigation,
                Nature,
                Religion,
            ],
            Self::Wizard => &[
                Arcana,
                History,
                Insight,
                Investigation,
                Medicine,
                Nature,
                Religion,
            ],
        }
    }

    pub fn armor_training(self) -> ArmorTraining {
        match self {
            Self::Barbarian | Self::Cleric | Self::Ranger => LIGHT_MEDIUM_SHIELDS,
            Self::Bard | Self::Rogue | Self::Warlock => LIGHT,
            Self::Druid => LIGHT_SHIELDS,
            Self::Fighter | Self::Paladin => ALL_ARMOR,
            Self::Monk | Self::Sorcerer | Self::Wizard => NO_ARMOR,
        }
    }

    pub fn tools(self) -> ClassTools {
        match self {
            Self::Bard => ClassTools::ThreeInstruments,
            Self::Druid => ClassTools::Fixed(Tool::HerbalismKit),
            Self::Monk => ClassTools::ArtisanOrInstrument,
            Self::Rogue => ClassTools::Fixed(Tool::ThievesTools),
            _ => ClassTools::None,
        }
    }

    /// The level 1 class features, as the class's features table names them.
    pub fn level_one_features(self) -> &'static [&'static str] {
        match self {
            Self::Barbarian => &["Rage", "Unarmored Defense", "Weapon Mastery"],
            Self::Bard => &["Bardic Inspiration", "Spellcasting"],
            Self::Cleric => &["Spellcasting", "Divine Order"],
            Self::Druid => &["Spellcasting", "Druidic", "Primal Order"],
            Self::Fighter => &["Fighting Style", "Second Wind", "Weapon Mastery"],
            Self::Monk => &["Martial Arts", "Unarmored Defense"],
            Self::Paladin => &["Lay On Hands", "Spellcasting", "Weapon Mastery"],
            Self::Ranger => &["Spellcasting", "Favored Enemy", "Weapon Mastery"],
            Self::Rogue => &[
                "Expertise",
                "Sneak Attack",
                "Thieves' Cant",
                "Weapon Mastery",
            ],
            Self::Sorcerer => &["Spellcasting", "Innate Sorcery"],
            Self::Warlock => &["Eldritch Invocations", "Pact Magic"],
            Self::Wizard => &["Spellcasting", "Ritual Adept", "Arcane Recovery"],
        }
    }

    /// The class's row of the Standard Array by Class table (SRD p. 21): where the rules suggest
    /// the standard array goes, in the order of `Ability::ALL`.
    pub fn suggested_array(self) -> [u8; 6] {
        match self {
            Self::Barbarian => [15, 13, 14, 10, 12, 8],
            Self::Bard => [8, 14, 12, 13, 10, 15],
            Self::Cleric => [14, 8, 13, 10, 15, 12],
            Self::Druid => [8, 12, 14, 13, 15, 10],
            Self::Fighter => [15, 14, 13, 8, 10, 12],
            Self::Monk => [12, 15, 13, 10, 14, 8],
            Self::Paladin => [15, 10, 13, 8, 12, 14],
            Self::Ranger => [12, 15, 13, 8, 14, 10],
            Self::Rogue => [12, 15, 13, 14, 10, 8],
            Self::Sorcerer => [10, 13, 14, 8, 12, 15],
            Self::Warlock => [8, 14, 13, 12, 10, 15],
            Self::Wizard => [8, 12, 13, 15, 14, 10],
        }
    }

    /// The starting equipment choices, in the order the table gives them (A, B, then C).
    pub fn kits(self) -> &'static [Kit] {
        match self {
            Self::Barbarian => BARBARIAN_KITS,
            Self::Bard => BARD_KITS,
            Self::Cleric => CLERIC_KITS,
            Self::Druid => DRUID_KITS,
            Self::Fighter => FIGHTER_KITS,
            Self::Monk => MONK_KITS,
            Self::Paladin => PALADIN_KITS,
            Self::Ranger => RANGER_KITS,
            Self::Rogue => ROGUE_KITS,
            Self::Sorcerer => SORCERER_KITS,
            Self::Warlock => WARLOCK_KITS,
            Self::Wizard => WIZARD_KITS,
        }
    }
}

listed_enum! {
    keyed
    /// The sixteen backgrounds of the 2024 core rules. Only Acolyte, Criminal, Sage and Soldier are
    /// printed in SRD 5.2.1 (p. 83); the others are names until their source is admitted.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum HeroBackground {
        Acolyte,
        Artisan,
        Charlatan,
        Criminal,
        Entertainer,
        Farmer,
        Guard,
        Guide,
        Hermit,
        Merchant,
        Noble,
        Sage,
        Sailor,
        Scribe,
        Soldier,
        Wayfarer,
    }
}

/// The tool proficiency a background grants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundTool {
    Fixed(Tool),
    /// One kind of Gaming Set of the player's choice (Soldier).
    GamingSet,
}

/// What an SRD 5.2.1 background grants (p. 83).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SrdBackground {
    pub abilities: [Ability; 3],
    pub feat: OriginFeat,
    /// The spell list of a Magic Initiate feat, as the background names it.
    pub feat_list: Option<&'static str>,
    pub skills: [Skill; 2],
    pub tool: BackgroundTool,
    /// Equipment choice A; choice B is `BACKGROUND_GOLD_GP` gold pieces.
    pub kit: Kit,
}

/// The gold of a background's equipment choice B (p. 83).
pub const BACKGROUND_GOLD_GP: u32 = 50;

impl HeroBackground {
    pub fn key(self) -> &'static str {
        match self {
            Self::Acolyte => "acolyte",
            Self::Artisan => "artisan",
            Self::Charlatan => "charlatan",
            Self::Criminal => "criminal",
            Self::Entertainer => "entertainer",
            Self::Farmer => "farmer",
            Self::Guard => "guard",
            Self::Guide => "guide",
            Self::Hermit => "hermit",
            Self::Merchant => "merchant",
            Self::Noble => "noble",
            Self::Sage => "sage",
            Self::Sailor => "sailor",
            Self::Scribe => "scribe",
            Self::Soldier => "soldier",
            Self::Wayfarer => "wayfarer",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Acolyte => "Acolyte",
            Self::Artisan => "Artisan",
            Self::Charlatan => "Charlatan",
            Self::Criminal => "Criminal",
            Self::Entertainer => "Entertainer",
            Self::Farmer => "Farmer",
            Self::Guard => "Guard",
            Self::Guide => "Guide",
            Self::Hermit => "Hermit",
            Self::Merchant => "Merchant",
            Self::Noble => "Noble",
            Self::Sage => "Sage",
            Self::Sailor => "Sailor",
            Self::Scribe => "Scribe",
            Self::Soldier => "Soldier",
            Self::Wayfarer => "Wayfarer",
        }
    }

    /// What the background grants, or `None` for a background SRD 5.2.1 does not print.
    pub fn srd(self) -> Option<SrdBackground> {
        use Ability::{Charisma, Constitution, Dexterity, Intelligence, Strength, Wisdom};
        Some(match self {
            Self::Acolyte => SrdBackground {
                abilities: [Intelligence, Wisdom, Charisma],
                feat: OriginFeat::MagicInitiate,
                feat_list: Some("Cleric"),
                skills: [Skill::Insight, Skill::Religion],
                tool: BackgroundTool::Fixed(Tool::CalligraphersSupplies),
                kit: Kit {
                    armor: None,
                    shield: false,
                    weapons: &[],
                    gear: &[
                        ("Calligrapher's Supplies", 1),
                        ("Book (prayers)", 1),
                        ("Holy Symbol", 1),
                        ("Parchment (sheets)", 10),
                        ("Robe", 1),
                    ],
                    gold_gp: 8,
                },
            },
            Self::Criminal => SrdBackground {
                abilities: [Dexterity, Constitution, Intelligence],
                feat: OriginFeat::Alert,
                feat_list: None,
                skills: [Skill::SleightOfHand, Skill::Stealth],
                tool: BackgroundTool::Fixed(Tool::ThievesTools),
                kit: Kit {
                    armor: None,
                    shield: false,
                    weapons: &[(Weapon::Dagger, 2)],
                    gear: &[
                        ("Thieves' Tools", 1),
                        ("Crowbar", 1),
                        ("Pouch", 2),
                        ("Traveler's Clothes", 1),
                    ],
                    gold_gp: 16,
                },
            },
            Self::Sage => SrdBackground {
                abilities: [Constitution, Intelligence, Wisdom],
                feat: OriginFeat::MagicInitiate,
                feat_list: Some("Wizard"),
                skills: [Skill::Arcana, Skill::History],
                tool: BackgroundTool::Fixed(Tool::CalligraphersSupplies),
                kit: Kit {
                    armor: None,
                    shield: false,
                    weapons: &[(Weapon::Quarterstaff, 1)],
                    gear: &[
                        ("Calligrapher's Supplies", 1),
                        ("Book (history)", 1),
                        ("Parchment (sheets)", 8),
                        ("Robe", 1),
                    ],
                    gold_gp: 8,
                },
            },
            Self::Soldier => SrdBackground {
                abilities: [Strength, Dexterity, Constitution],
                feat: OriginFeat::SavageAttacker,
                feat_list: None,
                skills: [Skill::Athletics, Skill::Intimidation],
                tool: BackgroundTool::GamingSet,
                kit: Kit {
                    armor: None,
                    shield: false,
                    weapons: &[(Weapon::Spear, 1), (Weapon::Shortbow, 1)],
                    gear: &[
                        ("Arrows", 20),
                        (CHOSEN_TOOL_GEAR, 1),
                        ("Healer's Kit", 1),
                        ("Quiver", 1),
                        ("Traveler's Clothes", 1),
                    ],
                    gold_gp: 14,
                },
            },
            _ => return None,
        })
    }
}

/// One starting equipment choice: its armour, shield, weapons (with counts), other gear (by name,
/// with counts) and gold. A choice that is only gold has nothing else.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Kit {
    pub armor: Option<Armor>,
    pub shield: bool,
    pub weapons: &'static [(Weapon, u16)],
    pub gear: &'static [(&'static str, u16)],
    pub gold_gp: u32,
}

impl Kit {
    /// What the kit holds, in one line: "Chain Mail, Greatsword, Javelin (8), 4 GP".
    pub fn summary(&self) -> String {
        let counted = |title: &str, count: u16| {
            if count == 1 {
                title.to_owned()
            } else {
                format!("{title} ({count})")
            }
        };
        let mut parts = Vec::new();
        if let Some(armor) = self.armor {
            parts.push(armor.facts().title.to_owned());
        }
        if self.shield {
            parts.push("Shield".to_owned());
        }
        for (weapon, count) in self.weapons {
            parts.push(counted(weapon.facts().title, *count));
        }
        for (gear, count) in self.gear {
            let title = if *gear == CHOSEN_TOOL_GEAR {
                "the tool you chose"
            } else {
                gear
            };
            parts.push(counted(title, *count));
        }
        parts.push(format!("{} GP", self.gold_gp));
        parts.join(", ")
    }
}

const fn gold(gold_gp: u32) -> Kit {
    Kit {
        armor: None,
        shield: false,
        weapons: &[],
        gear: &[],
        gold_gp,
    }
}

/// The kit gear that using uses up: ammunition and writing paper. Every other item of a kit
/// (armour, weapons, tools, packs, clothes) is kept when used.
pub const CONSUMABLE_GEAR: [&str; 2] = ["Arrows", "Parchment (sheets)"];

/// The place in a kit of the tool the player chose for the proficiency: the Bard's first
/// instrument, the Monk's tool, the Soldier's gaming set ("same as above").
pub const CHOSEN_TOOL_GEAR: &str = "(the tool chosen for the proficiency)";

const BARBARIAN_KITS: &[Kit] = &[
    Kit {
        armor: None,
        shield: false,
        weapons: &[(Weapon::Greataxe, 1), (Weapon::Handaxe, 4)],
        gear: &[("Explorer's Pack", 1)],
        gold_gp: 15,
    },
    gold(75),
];
const BARD_KITS: &[Kit] = &[
    Kit {
        armor: Some(Armor::Leather),
        shield: false,
        weapons: &[(Weapon::Dagger, 2)],
        gear: &[(CHOSEN_TOOL_GEAR, 1), ("Entertainer's Pack", 1)],
        gold_gp: 19,
    },
    gold(90),
];
const CLERIC_KITS: &[Kit] = &[
    Kit {
        armor: Some(Armor::ChainShirt),
        shield: true,
        weapons: &[(Weapon::Mace, 1)],
        gear: &[("Holy Symbol", 1), ("Priest's Pack", 1)],
        gold_gp: 7,
    },
    gold(110),
];
const DRUID_KITS: &[Kit] = &[
    Kit {
        armor: Some(Armor::Leather),
        shield: true,
        weapons: &[(Weapon::Sickle, 1), (Weapon::Quarterstaff, 1)],
        gear: &[("Explorer's Pack", 1), ("Herbalism Kit", 1)],
        gold_gp: 9,
    },
    gold(50),
];
const FIGHTER_KITS: &[Kit] = &[
    Kit {
        armor: Some(Armor::ChainMail),
        shield: false,
        weapons: &[
            (Weapon::Greatsword, 1),
            (Weapon::Flail, 1),
            (Weapon::Javelin, 8),
        ],
        gear: &[("Dungeoneer's Pack", 1)],
        gold_gp: 4,
    },
    Kit {
        armor: Some(Armor::StuddedLeather),
        shield: false,
        weapons: &[
            (Weapon::Scimitar, 1),
            (Weapon::Shortsword, 1),
            (Weapon::Longbow, 1),
        ],
        gear: &[("Arrows", 20), ("Quiver", 1), ("Dungeoneer's Pack", 1)],
        gold_gp: 11,
    },
    gold(155),
];
const MONK_KITS: &[Kit] = &[
    Kit {
        armor: None,
        shield: false,
        weapons: &[(Weapon::Spear, 1), (Weapon::Dagger, 5)],
        gear: &[(CHOSEN_TOOL_GEAR, 1), ("Explorer's Pack", 1)],
        gold_gp: 11,
    },
    gold(50),
];
const PALADIN_KITS: &[Kit] = &[
    Kit {
        armor: Some(Armor::ChainMail),
        shield: true,
        weapons: &[(Weapon::Longsword, 1), (Weapon::Javelin, 6)],
        gear: &[("Holy Symbol", 1), ("Priest's Pack", 1)],
        gold_gp: 9,
    },
    gold(150),
];
const RANGER_KITS: &[Kit] = &[
    Kit {
        armor: Some(Armor::StuddedLeather),
        shield: false,
        weapons: &[
            (Weapon::Scimitar, 1),
            (Weapon::Shortsword, 1),
            (Weapon::Longbow, 1),
        ],
        gear: &[
            ("Arrows", 20),
            ("Quiver", 1),
            ("Druidic Focus (sprig of mistletoe)", 1),
            ("Explorer's Pack", 1),
        ],
        gold_gp: 7,
    },
    gold(150),
];
const ROGUE_KITS: &[Kit] = &[
    Kit {
        armor: Some(Armor::Leather),
        shield: false,
        weapons: &[
            (Weapon::Dagger, 2),
            (Weapon::Shortsword, 1),
            (Weapon::Shortbow, 1),
        ],
        gear: &[
            ("Arrows", 20),
            ("Quiver", 1),
            ("Thieves' Tools", 1),
            ("Burglar's Pack", 1),
        ],
        gold_gp: 8,
    },
    gold(100),
];
const SORCERER_KITS: &[Kit] = &[
    Kit {
        armor: None,
        shield: false,
        weapons: &[(Weapon::Spear, 1), (Weapon::Dagger, 2)],
        gear: &[("Arcane Focus (crystal)", 1), ("Dungeoneer's Pack", 1)],
        gold_gp: 28,
    },
    gold(50),
];
const WARLOCK_KITS: &[Kit] = &[
    Kit {
        armor: Some(Armor::Leather),
        shield: false,
        weapons: &[(Weapon::Sickle, 1), (Weapon::Dagger, 2)],
        gear: &[
            ("Arcane Focus (orb)", 1),
            ("Book (occult lore)", 1),
            ("Scholar's Pack", 1),
        ],
        gold_gp: 15,
    },
    gold(100),
];
const WIZARD_KITS: &[Kit] = &[
    Kit {
        armor: None,
        shield: false,
        weapons: &[(Weapon::Dagger, 2), (Weapon::Quarterstaff, 1)],
        gear: &[("Robe", 1), ("Spellbook", 1), ("Scholar's Pack", 1)],
        gold_gp: 5,
    },
    gold(55),
];

/// Whether a weapon is swung or shot (Weapons table, p. 91).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WeaponReach {
    Melee,
    /// A melee weapon with the Thrown property.
    MeleeOrThrown,
    Ranged,
}

/// The weapon mastery properties (SRD p. 90).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MasteryProperty {
    Cleave,
    Graze,
    Nick,
    Push,
    Sap,
    Slow,
    Topple,
    Vex,
}

impl MasteryProperty {
    pub fn title(self) -> &'static str {
        match self {
            Self::Cleave => "Cleave",
            Self::Graze => "Graze",
            Self::Nick => "Nick",
            Self::Push => "Push",
            Self::Sap => "Sap",
            Self::Slow => "Slow",
            Self::Topple => "Topple",
            Self::Vex => "Vex",
        }
    }

    /// What the property does, in a sentence of our own (SRD p. 90).
    pub fn effect(self) -> &'static str {
        match self {
            Self::Cleave => {
                "On a melee hit, one more attack at a second creature within 5 feet, once a turn."
            }
            Self::Graze => {
                "On a miss, the target still takes damage equal to your ability modifier."
            }
            Self::Nick => {
                "The Light weapon's extra attack is part of the Attack action, once a turn."
            }
            Self::Push => "On a hit, push a Large or smaller creature up to 10 feet away.",
            Self::Sap => "On a hit, the target has Disadvantage on its next attack roll.",
            Self::Slow => "On a damaging hit, the target's Speed drops by 10 feet until your turn.",
            Self::Topple => "On a hit, the target makes a Constitution save or falls Prone.",
            Self::Vex => {
                "On a damaging hit, Advantage on your next attack roll against the target."
            }
        }
    }
}

listed_enum! {
    /// The weapons of the starting kits and those a Rogue may master (Weapons table, p. 91).
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Weapon {
        Club,
        Dagger,
        Dart,
        Flail,
        Greataxe,
        Greatclub,
        Greatsword,
        HandCrossbow,
        Handaxe,
        Javelin,
        LightCrossbow,
        LightHammer,
        Longbow,
        Longsword,
        Mace,
        Quarterstaff,
        Rapier,
        Scimitar,
        Shortbow,
        Shortsword,
        Sickle,
        Sling,
        Spear,
        Whip,
    }
}

/// One weapon's row of the Weapons table that a sheet reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WeaponFacts {
    pub id: &'static str,
    pub title: &'static str,
    pub dice: u8,
    pub die: DieSize,
    pub damage_type: &'static str,
    pub reach: WeaponReach,
    pub finesse: bool,
}

impl Weapon {
    /// Whether the weapon has the Two-Handed property (Weapons table, p. 91).
    pub fn two_handed(self) -> bool {
        matches!(
            self,
            Self::Greataxe
                | Self::Greatclub
                | Self::Greatsword
                | Self::LightCrossbow
                | Self::Longbow
                | Self::Shortbow
        )
    }

    /// Whether the weapon has the Light property (Weapons table, p. 91).
    pub fn light(self) -> bool {
        matches!(
            self,
            Self::Club
                | Self::Dagger
                | Self::HandCrossbow
                | Self::Handaxe
                | Self::LightHammer
                | Self::Scimitar
                | Self::Shortsword
                | Self::Sickle
        )
    }

    /// Whether the weapon is a Martial weapon; every other weapon here is Simple (p. 91).
    pub fn martial(self) -> bool {
        matches!(
            self,
            Self::Flail
                | Self::Greataxe
                | Self::Greatsword
                | Self::HandCrossbow
                | Self::Longbow
                | Self::Longsword
                | Self::Rapier
                | Self::Scimitar
                | Self::Shortsword
                | Self::Whip
        )
    }

    /// The weapon's mastery property (Weapons table, p. 91).
    pub fn mastery(self) -> MasteryProperty {
        use MasteryProperty::{Cleave, Graze, Nick, Push, Sap, Slow, Topple, Vex};
        match self {
            Self::Club
            | Self::Javelin
            | Self::LightCrossbow
            | Self::Longbow
            | Self::Sling
            | Self::Whip => Slow,
            Self::Dagger | Self::LightHammer | Self::Scimitar | Self::Sickle => Nick,
            Self::Dart
            | Self::HandCrossbow
            | Self::Handaxe
            | Self::Rapier
            | Self::Shortbow
            | Self::Shortsword => Vex,
            Self::Flail | Self::Longsword | Self::Mace | Self::Spear => Sap,
            Self::Greataxe => Cleave,
            Self::Greatclub => Push,
            Self::Greatsword => Graze,
            Self::Quarterstaff => Topple,
        }
    }

    pub fn facts(self) -> WeaponFacts {
        use WeaponReach::{Melee, MeleeOrThrown, Ranged};
        let row = |id, title, dice, die, damage_type, reach, finesse| WeaponFacts {
            id,
            title,
            dice,
            die,
            damage_type,
            reach,
            finesse,
        };
        match self {
            Self::Club => row("club", "Club", 1, DieSize::D4, "bludgeoning", Melee, false),
            Self::Dart => row("dart", "Dart", 1, DieSize::D4, "piercing", Ranged, true),
            Self::Greatclub => row(
                "greatclub",
                "Greatclub",
                1,
                DieSize::D8,
                "bludgeoning",
                Melee,
                false,
            ),
            Self::HandCrossbow => row(
                "hand_crossbow",
                "Hand Crossbow",
                1,
                DieSize::D6,
                "piercing",
                Ranged,
                false,
            ),
            Self::LightCrossbow => row(
                "light_crossbow",
                "Light Crossbow",
                1,
                DieSize::D8,
                "piercing",
                Ranged,
                false,
            ),
            Self::LightHammer => row(
                "light_hammer",
                "Light Hammer",
                1,
                DieSize::D4,
                "bludgeoning",
                MeleeOrThrown,
                false,
            ),
            Self::Rapier => row("rapier", "Rapier", 1, DieSize::D8, "piercing", Melee, true),
            Self::Sling => row(
                "sling",
                "Sling",
                1,
                DieSize::D4,
                "bludgeoning",
                Ranged,
                false,
            ),
            Self::Whip => row("whip", "Whip", 1, DieSize::D4, "slashing", Melee, true),
            Self::Dagger => row(
                "dagger",
                "Dagger",
                1,
                DieSize::D4,
                "piercing",
                MeleeOrThrown,
                true,
            ),
            Self::Flail => row(
                "flail",
                "Flail",
                1,
                DieSize::D8,
                "bludgeoning",
                Melee,
                false,
            ),
            Self::Greataxe => row(
                "greataxe",
                "Greataxe",
                1,
                DieSize::D12,
                "slashing",
                Melee,
                false,
            ),
            Self::Greatsword => row(
                "greatsword",
                "Greatsword",
                2,
                DieSize::D6,
                "slashing",
                Melee,
                false,
            ),
            Self::Handaxe => row(
                "handaxe",
                "Handaxe",
                1,
                DieSize::D6,
                "slashing",
                MeleeOrThrown,
                false,
            ),
            Self::Javelin => row(
                "javelin",
                "Javelin",
                1,
                DieSize::D6,
                "piercing",
                MeleeOrThrown,
                false,
            ),
            Self::Longbow => row(
                "longbow",
                "Longbow",
                1,
                DieSize::D8,
                "piercing",
                Ranged,
                false,
            ),
            Self::Longsword => row(
                "longsword",
                "Longsword",
                1,
                DieSize::D8,
                "slashing",
                Melee,
                false,
            ),
            Self::Mace => row("mace", "Mace", 1, DieSize::D6, "bludgeoning", Melee, false),
            Self::Quarterstaff => row(
                "quarterstaff",
                "Quarterstaff",
                1,
                DieSize::D6,
                "bludgeoning",
                Melee,
                false,
            ),
            Self::Scimitar => row(
                "scimitar",
                "Scimitar",
                1,
                DieSize::D6,
                "slashing",
                Melee,
                true,
            ),
            Self::Shortbow => row(
                "shortbow",
                "Shortbow",
                1,
                DieSize::D6,
                "piercing",
                Ranged,
                false,
            ),
            Self::Shortsword => row(
                "shortsword",
                "Shortsword",
                1,
                DieSize::D6,
                "piercing",
                Melee,
                true,
            ),
            Self::Sickle => row("sickle", "Sickle", 1, DieSize::D4, "slashing", Melee, false),
            Self::Spear => row(
                "spear",
                "Spear",
                1,
                DieSize::D6,
                "piercing",
                MeleeOrThrown,
                false,
            ),
        }
    }
}

/// The armour of the starting kits (Armor table, p. 92).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Armor {
    Leather,
    StuddedLeather,
    ChainShirt,
    ChainMail,
}

/// One armour's row of the Armor table that a sheet reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArmorFacts {
    pub title: &'static str,
    pub base: u8,
    /// The most Dexterity modifier the armour adds; `None` adds all of it, `Some(0)` none.
    pub dexterity_cap: Option<u8>,
    /// The Strength score under which the armour reduces Speed by 10 feet.
    pub strength_needed: Option<u8>,
    /// Whether the wearer has Disadvantage on Dexterity (Stealth) checks.
    pub stealth_disadvantage: bool,
}

impl Armor {
    pub fn facts(self) -> ArmorFacts {
        match self {
            Self::Leather => ArmorFacts {
                title: "Leather Armor",
                base: 11,
                dexterity_cap: None,
                strength_needed: None,
                stealth_disadvantage: false,
            },
            Self::StuddedLeather => ArmorFacts {
                title: "Studded Leather Armor",
                base: 12,
                dexterity_cap: None,
                strength_needed: None,
                stealth_disadvantage: false,
            },
            Self::ChainShirt => ArmorFacts {
                title: "Chain Shirt",
                base: 13,
                dexterity_cap: Some(2),
                strength_needed: None,
                stealth_disadvantage: false,
            },
            Self::ChainMail => ArmorFacts {
                title: "Chain Mail",
                base: 16,
                dexterity_cap: Some(0),
                strength_needed: Some(13),
                stealth_disadvantage: true,
            },
        }
    }
}

/// The Shield's bonus to Armor Class (Armor table, p. 92).
pub const SHIELD_BONUS: i32 = 2;
/// The base Armor Class without armour or a shield (SRD p. 23).
pub const UNARMORED_BASE: i32 = 10;

listed_enum! {
    keyed
    /// The tools a build may name (Tools, SRD pp. 93-95): the Artisan's Tools, the Musical
    /// Instruments and the other tools a class or background grants.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Tool {
        AlchemistsSupplies,
        BrewersSupplies,
        CalligraphersSupplies,
        CarpentersTools,
        CartographersTools,
        CobblersTools,
        CooksUtensils,
        GlassblowersTools,
        JewelersTools,
        LeatherworkersTools,
        MasonsTools,
        PaintersSupplies,
        PottersTools,
        SmithsTools,
        TinkersTools,
        WeaversTools,
        WoodcarversTools,
        Bagpipes,
        Drum,
        Dulcimer,
        Flute,
        Horn,
        Lute,
        Lyre,
        PanFlute,
        Shawm,
        Viol,
        HerbalismKit,
        ThievesTools,
    }
}

/// What kind of tool a tool is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolKind {
    Artisan,
    Instrument,
    Other,
}

impl Tool {
    pub fn key(self) -> &'static str {
        match self {
            Self::AlchemistsSupplies => "alchemists_supplies",
            Self::BrewersSupplies => "brewers_supplies",
            Self::CalligraphersSupplies => "calligraphers_supplies",
            Self::CarpentersTools => "carpenters_tools",
            Self::CartographersTools => "cartographers_tools",
            Self::CobblersTools => "cobblers_tools",
            Self::CooksUtensils => "cooks_utensils",
            Self::GlassblowersTools => "glassblowers_tools",
            Self::JewelersTools => "jewelers_tools",
            Self::LeatherworkersTools => "leatherworkers_tools",
            Self::MasonsTools => "masons_tools",
            Self::PaintersSupplies => "painters_supplies",
            Self::PottersTools => "potters_tools",
            Self::SmithsTools => "smiths_tools",
            Self::TinkersTools => "tinkers_tools",
            Self::WeaversTools => "weavers_tools",
            Self::WoodcarversTools => "woodcarvers_tools",
            Self::Bagpipes => "bagpipes",
            Self::Drum => "drum",
            Self::Dulcimer => "dulcimer",
            Self::Flute => "flute",
            Self::Horn => "horn",
            Self::Lute => "lute",
            Self::Lyre => "lyre",
            Self::PanFlute => "pan_flute",
            Self::Shawm => "shawm",
            Self::Viol => "viol",
            Self::HerbalismKit => "herbalism_kit",
            Self::ThievesTools => "thieves_tools",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::AlchemistsSupplies => "Alchemist's Supplies",
            Self::BrewersSupplies => "Brewer's Supplies",
            Self::CalligraphersSupplies => "Calligrapher's Supplies",
            Self::CarpentersTools => "Carpenter's Tools",
            Self::CartographersTools => "Cartographer's Tools",
            Self::CobblersTools => "Cobbler's Tools",
            Self::CooksUtensils => "Cook's Utensils",
            Self::GlassblowersTools => "Glassblower's Tools",
            Self::JewelersTools => "Jeweler's Tools",
            Self::LeatherworkersTools => "Leatherworker's Tools",
            Self::MasonsTools => "Mason's Tools",
            Self::PaintersSupplies => "Painter's Supplies",
            Self::PottersTools => "Potter's Tools",
            Self::SmithsTools => "Smith's Tools",
            Self::TinkersTools => "Tinker's Tools",
            Self::WeaversTools => "Weaver's Tools",
            Self::WoodcarversTools => "Woodcarver's Tools",
            Self::Bagpipes => "Bagpipes",
            Self::Drum => "Drum",
            Self::Dulcimer => "Dulcimer",
            Self::Flute => "Flute",
            Self::Horn => "Horn",
            Self::Lute => "Lute",
            Self::Lyre => "Lyre",
            Self::PanFlute => "Pan Flute",
            Self::Shawm => "Shawm",
            Self::Viol => "Viol",
            Self::HerbalismKit => "Herbalism Kit",
            Self::ThievesTools => "Thieves' Tools",
        }
    }

    pub fn kind(self) -> ToolKind {
        match self {
            Self::Bagpipes
            | Self::Drum
            | Self::Dulcimer
            | Self::Flute
            | Self::Horn
            | Self::Lute
            | Self::Lyre
            | Self::PanFlute
            | Self::Shawm
            | Self::Viol => ToolKind::Instrument,
            Self::HerbalismKit | Self::ThievesTools => ToolKind::Other,
            _ => ToolKind::Artisan,
        }
    }
}
