//! SRD 5.2.1 pp.84-87: source choices and the implemented species combat subset.
use crate::local_journey::{HeroSpecies, JourneyRuleError, SpeciesCombatTraits, SpeciesSize};
use crate::tactical::DamageType;
use serde::{Deserialize, Serialize};

/// A required creation group, offered only for its owning species.
#[derive(Clone, Copy, Debug)]
pub struct SpeciesChoiceGroup {
    pub id: &'static str,
    pub label: &'static str,
    pub options: &'static [&'static str],
}
const DRAGONS: &[&str] = &[
    "black", "blue", "brass", "bronze", "copper", "gold", "green", "red", "silver", "white",
];
const SKILLS: &[&str] = &[
    "acrobatics",
    "animal-handling",
    "arcana",
    "athletics",
    "deception",
    "history",
    "insight",
    "intimidation",
    "investigation",
    "medicine",
    "nature",
    "perception",
    "performance",
    "persuasion",
    "religion",
    "sleight-of-hand",
    "stealth",
    "survival",
];
const SPELL_ABILITIES: &[&str] = &["intelligence", "wisdom", "charisma"];
const SIZE_OPTIONS: &[&str] = &["medium", "small"];
const fn group(
    id: &'static str,
    label: &'static str,
    options: &'static [&'static str],
) -> SpeciesChoiceGroup {
    SpeciesChoiceGroup { id, label, options }
}
/// Spellcasting choices are retained, but their spells are explicitly not in play yet.
/// Tough is the only currently implemented Human Origin feat; Soldier already grants
/// nonrepeatable Savage Attacker (SRD p.87), so it cannot be selected a second time.
pub fn choice_groups(species: HeroSpecies) -> Vec<SpeciesChoiceGroup> {
    use HeroSpecies::*;
    match species {
        Dragonborn => vec![group("draconic-ancestry", "Draconic Ancestry", DRAGONS)],
        Elf => vec![
            group(
                "elven-lineage",
                "Elven Lineage",
                &["drow", "high-elf", "wood-elf"],
            ),
            group(
                "keen-senses",
                "Keen Senses",
                &["insight", "perception", "survival"],
            ),
            group(
                "species-spellcasting",
                "Lineage spellcasting ability (spells not in play yet)",
                SPELL_ABILITIES,
            ),
        ],
        Gnome => vec![
            group(
                "gnomish-lineage",
                "Gnomish Lineage",
                &["forest-gnome", "rock-gnome"],
            ),
            group(
                "species-spellcasting",
                "Lineage spellcasting ability (spells not in play yet)",
                SPELL_ABILITIES,
            ),
        ],
        Goliath => vec![group(
            "giant-ancestry",
            "Giant Ancestry",
            &["cloud", "fire", "frost", "hill", "stone", "storm"],
        )],
        Human => vec![
            group("species-size", "Size", SIZE_OPTIONS),
            group("human-skill", "Skillful", SKILLS),
            group(
                "versatile",
                "Versatile (implemented Origin feats)",
                &["tough"],
            ),
        ],
        Tiefling => vec![
            group(
                "fiendish-legacy",
                "Fiendish Legacy",
                &["abyssal", "chthonic", "infernal"],
            ),
            group("species-size", "Size", SIZE_OPTIONS),
            group(
                "species-spellcasting",
                "Legacy spellcasting ability (spells not in play yet)",
                SPELL_ABILITIES,
            ),
        ],
        Dwarf | Halfling | Orc => vec![],
    }
}
/// Recognizes the source catalog only; applicability and completeness are checked separately.
pub fn choice_option_is_known(id: &str, option: &str) -> bool {
    [
        HeroSpecies::Dragonborn,
        HeroSpecies::Elf,
        HeroSpecies::Gnome,
        HeroSpecies::Goliath,
        HeroSpecies::Human,
        HeroSpecies::Tiefling,
    ]
    .into_iter()
    .flat_map(choice_groups)
    .any(|group| group.id == id && group.options.contains(&option))
}
pub fn is_choice_group(id: &str) -> bool {
    [
        "draconic-ancestry",
        "elven-lineage",
        "keen-senses",
        "species-spellcasting",
        "gnomish-lineage",
        "giant-ancestry",
        "species-size",
        "human-skill",
        "versatile",
        "fiendish-legacy",
    ]
    .contains(&id)
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum DraconicAncestry {
    Black,
    Blue,
    Brass,
    Bronze,
    Copper,
    Gold,
    Green,
    Red,
    Silver,
    White,
}
impl DraconicAncestry {
    pub fn damage_type(self) -> DamageType {
        use DraconicAncestry::*;
        match self {
            Black | Copper => DamageType::Acid,
            Blue | Bronze => DamageType::Lightning,
            Brass | Gold | Red => DamageType::Fire,
            Green => DamageType::Poison,
            Silver | White => DamageType::Cold,
        }
    }
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElvenLineage {
    Drow,
    HighElf,
    WoodElf,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum GnomishLineage {
    Forest,
    Rock,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum GiantAncestry {
    Cloud,
    Fire,
    Frost,
    Hill,
    Stone,
    Storm,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum FiendishLegacy {
    Abyssal,
    Chthonic,
    Infernal,
}
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ancestry {
    Draconic(DraconicAncestry),
    Elven(ElvenLineage),
    Gnomish(GnomishLineage),
    Giant(GiantAncestry),
    Fiendish(FiendishLegacy),
}
/// Source-validated mechanical choices, never independently client-derived bonuses.
#[derive(Default, Deserialize, Serialize, Clone, Copy, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SpeciesOptions {
    pub ancestry: Option<Ancestry>,
    pub size: Option<SpeciesSize>,
    pub tough: bool,
}
impl SpeciesOptions {
    pub fn from_choices(
        species: HeroSpecies,
        choices: &[(&str, &str)],
    ) -> Result<Self, JourneyRuleError> {
        let groups = choice_groups(species);
        for group in &groups {
            let selected = choices
                .iter()
                .filter(|(id, _)| *id == group.id)
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Err(JourneyRuleError::MissingChoice);
            }
            if selected.len() != 1 {
                return Err(JourneyRuleError::DuplicateChoice);
            }
            if !group.options.contains(&selected[0].1) {
                return Err(JourneyRuleError::InvalidChoice);
            }
        }
        if choices
            .iter()
            .any(|(id, _)| is_choice_group(id) && !groups.iter().any(|group| group.id == *id))
        {
            return Err(JourneyRuleError::InvalidChoice);
        }
        let value = |id: &str| {
            choices
                .iter()
                .find(|(key, _)| *key == id)
                .map(|(_, value)| *value)
        };
        // SRD pp.84,86: skill grants are additional proficiencies, not duplicate grants.
        if let Some(skill) = value("human-skill").or_else(|| value("keen-senses"))
            && (matches!(skill, "athletics" | "intimidation")
                || value("skill-1") == Some(skill)
                || value("skill-2") == Some(skill))
        {
            return Err(JourneyRuleError::InvalidChoice);
        }
        let ancestry = match species {
            HeroSpecies::Dragonborn => Some(Ancestry::Draconic(match value("draconic-ancestry") {
                Some("black") => DraconicAncestry::Black,
                Some("blue") => DraconicAncestry::Blue,
                Some("brass") => DraconicAncestry::Brass,
                Some("bronze") => DraconicAncestry::Bronze,
                Some("copper") => DraconicAncestry::Copper,
                Some("gold") => DraconicAncestry::Gold,
                Some("green") => DraconicAncestry::Green,
                Some("red") => DraconicAncestry::Red,
                Some("silver") => DraconicAncestry::Silver,
                Some("white") => DraconicAncestry::White,
                _ => return Err(JourneyRuleError::InvalidChoice),
            })),
            HeroSpecies::Elf => Some(Ancestry::Elven(match value("elven-lineage") {
                Some("drow") => ElvenLineage::Drow,
                Some("high-elf") => ElvenLineage::HighElf,
                Some("wood-elf") => ElvenLineage::WoodElf,
                _ => return Err(JourneyRuleError::InvalidChoice),
            })),
            HeroSpecies::Gnome => Some(Ancestry::Gnomish(match value("gnomish-lineage") {
                Some("forest-gnome") => GnomishLineage::Forest,
                Some("rock-gnome") => GnomishLineage::Rock,
                _ => return Err(JourneyRuleError::InvalidChoice),
            })),
            HeroSpecies::Goliath => Some(Ancestry::Giant(match value("giant-ancestry") {
                Some("cloud") => GiantAncestry::Cloud,
                Some("fire") => GiantAncestry::Fire,
                Some("frost") => GiantAncestry::Frost,
                Some("hill") => GiantAncestry::Hill,
                Some("stone") => GiantAncestry::Stone,
                Some("storm") => GiantAncestry::Storm,
                _ => return Err(JourneyRuleError::InvalidChoice),
            })),
            HeroSpecies::Tiefling => Some(Ancestry::Fiendish(match value("fiendish-legacy") {
                Some("abyssal") => FiendishLegacy::Abyssal,
                Some("chthonic") => FiendishLegacy::Chthonic,
                Some("infernal") => FiendishLegacy::Infernal,
                _ => return Err(JourneyRuleError::InvalidChoice),
            })),
            _ => None,
        };
        Ok(Self {
            ancestry,
            size: value("species-size").map(|size| {
                if size == "small" {
                    SpeciesSize::Small
                } else {
                    SpeciesSize::Medium
                }
            }),
            tough: value("versatile") == Some("tough"),
        })
    }
    pub fn valid_for(self, species: HeroSpecies) -> bool {
        matches!(
            (species, self.ancestry),
            (HeroSpecies::Dragonborn, Some(Ancestry::Draconic(_)))
                | (HeroSpecies::Elf, Some(Ancestry::Elven(_)))
                | (HeroSpecies::Gnome, Some(Ancestry::Gnomish(_)))
                | (HeroSpecies::Goliath, Some(Ancestry::Giant(_)))
                | (HeroSpecies::Tiefling, Some(Ancestry::Fiendish(_)))
                | (
                    HeroSpecies::Dwarf
                        | HeroSpecies::Halfling
                        | HeroSpecies::Human
                        | HeroSpecies::Orc,
                    None
                )
        ) && self.size.is_some() == matches!(species, HeroSpecies::Human | HeroSpecies::Tiefling)
            && self.tough == (species == HeroSpecies::Human)
    }
    /// SRD pp.84-86: Wood Elf speed, Drow sight and chosen Human/Tiefling size.
    pub fn combat_traits(self, species: HeroSpecies) -> SpeciesCombatTraits {
        let mut traits = species.combat_traits();
        if let Some(size) = self.size {
            traits.size = size;
        }
        if self.ancestry == Some(Ancestry::Elven(ElvenLineage::WoodElf)) {
            traits.speed_ft = 35;
        }
        if self.ancestry == Some(Ancestry::Elven(ElvenLineage::Drow)) {
            traits.darkvision_ft = Some(120);
        }
        // SRD p.87 Tough: +2 maximum HP per character level.
        if self.tough {
            traits.hit_points_per_level += 2;
        }
        traits
    }
    /// SRD Dragonborn Damage Resistance, Dwarven Resilience and Fiendish Legacies.
    pub fn resistance(self, species: HeroSpecies) -> Option<DamageType> {
        if species.combat_traits().poison_resistance {
            return Some(DamageType::Poison);
        }
        match self.ancestry {
            Some(Ancestry::Draconic(ancestry)) => Some(ancestry.damage_type()),
            Some(Ancestry::Fiendish(legacy)) => Some(match legacy {
                FiendishLegacy::Abyssal => DamageType::Poison,
                FiendishLegacy::Chthonic => DamageType::Necrotic,
                FiendishLegacy::Infernal => DamageType::Fire,
            }),
            _ => None,
        }
    }
}

/// Explicit gaps are presentation data, never grants or a substitute for a rules handler.
pub fn not_in_play(species: HeroSpecies, options: SpeciesOptions) -> Vec<&'static str> {
    use HeroSpecies::*;
    match species {
        Dragonborn => vec!["Draconic Flight (level 5)"],
        Dwarf => vec![
            "Dwarven Resilience: advantage against Poisoned",
            "Stonecunning: Tremorsense",
        ],
        Elf => vec![
            "Elven Lineage: cantrips and spells",
            "Fey Ancestry: advantage against Charmed",
            "Trance",
        ],
        Gnome => vec![
            "Gnomish Cunning: mental saving throw advantage",
            "Gnomish Lineage: cantrips, spells and devices",
        ],
        Goliath => {
            let mut gaps = vec![
                "Large Form (level 5)",
                "Powerful Build: grapple escape and carrying capacity",
            ];
            if options.ancestry == Some(Ancestry::Giant(GiantAncestry::Cloud)) {
                gaps.push("Cloud's Jaunt: teleportation");
            }
            gaps
        }
        Halfling => vec![
            "Brave: advantage against Frightened",
            "Naturally Stealthy: Hide",
        ],
        Human => vec!["Resourceful: Heroic Inspiration"],
        Orc => vec!["Adrenaline Rush: Bonus Action Dash and Temporary Hit Points"],
        Tiefling => vec![
            "Fiendish Legacy: cantrips and spells",
            "Otherworldly Presence: Thaumaturgy",
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn srd_521_species_choices_keep_every_source_option_and_reject_cross_species_grants() {
        assert_eq!(choice_groups(HeroSpecies::Dragonborn)[0].options.len(), 10);
        assert_eq!(choice_groups(HeroSpecies::Goliath)[0].options.len(), 6);
        assert_eq!(choice_groups(HeroSpecies::Human)[1].options.len(), 18);
        assert_eq!(choice_groups(HeroSpecies::Human)[2].options, ["tough"]);
        assert!(
            SpeciesOptions::from_choices(HeroSpecies::Dwarf, &[("draconic-ancestry", "red")])
                .is_err()
        );
        assert!(SpeciesOptions::from_choices(HeroSpecies::Dragonborn, &[]).is_err());
        assert!(
            SpeciesOptions::from_choices(
                HeroSpecies::Dragonborn,
                &[("draconic-ancestry", "red"), ("draconic-ancestry", "blue")]
            )
            .is_err()
        );
        assert!(
            SpeciesOptions::from_choices(
                HeroSpecies::Dragonborn,
                &[("draconic-ancestry", "purple")]
            )
            .is_err()
        );
        assert!(HeroSpecies::from_choice("aasimar").is_none());
    }
    #[test]
    fn srd_521_draconic_ancestry_and_fiendish_legacy_resistances_match_source_tables() {
        for (colour, damage) in [
            ("black", DamageType::Acid),
            ("blue", DamageType::Lightning),
            ("brass", DamageType::Fire),
            ("bronze", DamageType::Lightning),
            ("copper", DamageType::Acid),
            ("gold", DamageType::Fire),
            ("green", DamageType::Poison),
            ("red", DamageType::Fire),
            ("silver", DamageType::Cold),
            ("white", DamageType::Cold),
        ] {
            let options = SpeciesOptions::from_choices(
                HeroSpecies::Dragonborn,
                &[("draconic-ancestry", colour)],
            )
            .unwrap();
            assert_eq!(options.resistance(HeroSpecies::Dragonborn), Some(damage));
        }
        for (legacy, damage) in [
            ("abyssal", DamageType::Poison),
            ("chthonic", DamageType::Necrotic),
            ("infernal", DamageType::Fire),
        ] {
            let options = SpeciesOptions::from_choices(
                HeroSpecies::Tiefling,
                &[
                    ("fiendish-legacy", legacy),
                    ("species-size", "small"),
                    ("species-spellcasting", "wisdom"),
                ],
            )
            .unwrap();
            assert_eq!(options.resistance(HeroSpecies::Tiefling), Some(damage));
            assert_eq!(
                options.combat_traits(HeroSpecies::Tiefling).size,
                SpeciesSize::Small
            );
        }
        assert_eq!(
            SpeciesOptions::default().resistance(HeroSpecies::Dwarf),
            Some(DamageType::Poison)
        );
    }
    #[test]
    fn srd_521_elven_lineage_sight_and_speed_and_human_toughness_are_not_cosmetic() {
        for (lineage, speed, sight) in [
            ("drow", 30, 120),
            ("high-elf", 30, 60),
            ("wood-elf", 35, 60),
        ] {
            let options = SpeciesOptions::from_choices(
                HeroSpecies::Elf,
                &[
                    ("elven-lineage", lineage),
                    ("keen-senses", "insight"),
                    ("species-spellcasting", "intelligence"),
                ],
            )
            .unwrap();
            let traits = options.combat_traits(HeroSpecies::Elf);
            assert_eq!(
                (traits.speed_ft, traits.darkvision_ft),
                (speed, Some(sight))
            );
        }
        let options = SpeciesOptions::from_choices(
            HeroSpecies::Human,
            &[
                ("species-size", "small"),
                ("human-skill", "arcana"),
                ("versatile", "tough"),
            ],
        )
        .unwrap();
        assert_eq!(
            options
                .combat_traits(HeroSpecies::Human)
                .hit_points_per_level,
            2
        );
        assert!(
            SpeciesOptions::from_choices(
                HeroSpecies::Human,
                &[
                    ("species-size", "small"),
                    ("human-skill", "arcana"),
                    ("versatile", "savage-attacker")
                ]
            )
            .is_err()
        );
        assert!(
            SpeciesOptions::from_choices(
                HeroSpecies::Human,
                &[
                    ("species-size", "small"),
                    ("human-skill", "athletics"),
                    ("versatile", "tough")
                ]
            )
            .is_err()
        );
    }
}
