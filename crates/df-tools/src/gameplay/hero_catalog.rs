//! Source-pinned creation cards. These describe SRD choices; only the rules engine can admit one.
//! SRD 5.2.1 printed pp.83-86 (backgrounds and species); pp.87-88 (Origin feats).

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Catalog {
    pub(crate) source: String,
    pub(crate) species: Vec<Species>,
    pub(crate) backgrounds: Vec<Background>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Species {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) page: u16,
    pub(crate) traits: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Background {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) page: u16,
    pub(crate) ability_scores: Vec<String>,
    pub(crate) origin_feat: serde_json::Value,
    pub(crate) skill_proficiencies: Vec<String>,
    pub(crate) tool_proficiency: serde_json::Value,
    pub(crate) equipment_a: serde_json::Value,
    pub(crate) equipment_b_gp: u16,
}

pub(crate) fn catalog() -> Result<Catalog, serde_json::Error> {
    serde_json::from_str(include_str!(
        "../../../../content/hero-tables/catalog-srd.json"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_srd_species_card_has_its_real_traits_and_printed_page() {
        let catalog = catalog().expect("source-backed catalog");
        assert_eq!(catalog.source, "srd-5.2.1");
        let expected = [
            ("dragonborn", 84, "Breath Weapon"),
            ("dwarf", 84, "Dwarven Toughness"),
            ("elf", 84, "Trance"),
            ("gnome", 85, "Gnomish Cunning"),
            ("goliath", 85, "Powerful Build"),
            ("halfling", 86, "Luck"),
            ("human", 86, "Versatile"),
            ("orc", 86, "Relentless Endurance"),
            ("tiefling", 86, "Fiendish Legacy"),
        ];
        assert_eq!(catalog.species.len(), expected.len());
        for (id, page, trait_name) in expected {
            let species = catalog.species.iter().find(|entry| entry.id == id).unwrap();
            assert!(!species.name.is_empty());
            assert_eq!(species.page, page, "{id}");
            assert!(
                species
                    .traits
                    .iter()
                    .any(|text| text.starts_with(trait_name)),
                "{id}"
            );
            assert!(
                species.traits.iter().any(|text| text.starts_with("Speed:")),
                "{id}"
            );
        }
    }

    #[test]
    fn four_srd_backgrounds_keep_feats_skills_tools_and_equipment_from_page_83() {
        let catalog = catalog().expect("source-backed catalog");
        assert_eq!(catalog.backgrounds.len(), 4);
        for (id, feat, skills, tool, gold) in [
            (
                "acolyte",
                "magic-initiate",
                ["insight", "religion"],
                "calligraphers-supplies",
                8,
            ),
            (
                "criminal",
                "alert",
                ["sleight-of-hand", "stealth"],
                "thieves-tools",
                16,
            ),
            (
                "sage",
                "magic-initiate",
                ["arcana", "history"],
                "calligraphers-supplies",
                8,
            ),
            (
                "soldier",
                "savage-attacker",
                ["athletics", "intimidation"],
                "gaming-set",
                14,
            ),
        ] {
            let background = catalog
                .backgrounds
                .iter()
                .find(|entry| entry.id == id)
                .unwrap();
            assert!(!background.name.is_empty());
            assert_eq!(background.page, 83, "{id}");
            assert_eq!(background.ability_scores.len(), 3, "{id}");
            assert_eq!(background.origin_feat["id"], feat, "{id}");
            assert_eq!(background.skill_proficiencies, skills, "{id}");
            let tool_id = if id == "soldier" { "category" } else { "id" };
            assert_eq!(background.tool_proficiency[tool_id], tool, "{id}");
            assert_eq!(background.equipment_a["gp"], gold, "{id}");
            assert_eq!(background.equipment_b_gp, 50, "{id}");
            assert!(
                !background.equipment_a["items"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
