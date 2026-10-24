//! Source-pinned creation cards. These describe SRD choices; only the rules engine can admit one.
//! SRD 5.2.1 printed pp.83-86 (backgrounds and species); pp.87-88 (Origin feats).

#[cfg(test)]
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[cfg(test)]
pub(crate) struct Catalog {
    pub(crate) source: String,
    pub(crate) species: Vec<Species>,
    pub(crate) backgrounds: Vec<Background>,
    pub(crate) items: Vec<Item>,
}

/// A gear entry. Only packs use `contents`; `kit_item` names the pack by the id the hero's
/// starting kit uses when that id differs from the catalog id.
#[derive(Debug, Deserialize)]
#[cfg(test)]
pub(crate) struct Item {
    pub(crate) id: String,
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) kit_item: Option<String>,
    #[serde(default)]
    pub(crate) contents: Vec<PackContent>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[cfg(test)]
pub(crate) struct PackContent {
    pub(crate) item: String,
    pub(crate) quantity: u8,
}

#[derive(Debug, Deserialize)]
#[cfg(test)]
pub(crate) struct Species {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) page: u16,
    pub(crate) traits: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn catalog() -> Result<Catalog, serde_json::Error> {
    serde_json::from_str(include_str!(
        "../../../../content/hero-tables/catalog-srd.json"
    ))
}

/// Species-dependent source choices. The wire owner appends these to the creation offer.
pub(crate) fn species_choice_groups() -> Vec<df_protocol::common::JourneyGroup> {
    use df_protocol::common as rpc;
    use df_rules::local_journey::HeroSpecies;
    let mut groups: Vec<rpc::JourneyGroup> = Vec::new();
    for species in [
        HeroSpecies::Dragonborn,
        HeroSpecies::Elf,
        HeroSpecies::Gnome,
        HeroSpecies::Goliath,
        HeroSpecies::Human,
        HeroSpecies::Tiefling,
    ] {
        for group in df_rules::species::choice_groups(species) {
            if let Some(existing) = groups
                .iter_mut()
                .find(|existing| existing.group_id == group.id)
            {
                for option in &mut existing.options {
                    option.requires_options.push(species.choice_id().to_owned());
                }
            } else {
                groups.push(rpc::JourneyGroup {
                    group_id: group.id.to_owned(),
                    label: group.label.to_owned(),
                    options: group
                        .options
                        .iter()
                        .map(|id| rpc::JourneyOption {
                            option_id: (*id).to_owned(),
                            label: id.replace('-', " "),
                            description: if *id == "cloud" {
                                "Not in play yet: Cloud's Jaunt".to_owned()
                            } else {
                                String::new()
                            },
                            requires_group: "species".to_owned(),
                            requires_options: vec![species.choice_id().to_owned()],
                        })
                        .collect(),
                });
            }
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_choice_groups_require_their_species_and_human_origin_feat_is_tough() {
        let groups = species_choice_groups();
        assert_eq!(
            groups
                .iter()
                .find(|group| group.group_id == "versatile")
                .unwrap()
                .options
                .iter()
                .map(|option| option.option_id.as_str())
                .collect::<Vec<_>>(),
            ["tough"]
        );
        for group in &groups {
            assert!(!group.options.is_empty());
            for option in &group.options {
                assert_eq!(option.requires_group, "species");
                assert!(!option.requires_options.is_empty());
                assert!(
                    !option
                        .requires_options
                        .iter()
                        .any(|species| species == "aasimar")
                );
            }
        }
        let spell = groups
            .iter()
            .find(|group| group.group_id == "species-spellcasting")
            .unwrap();
        assert_eq!(
            spell.options[0].requires_options,
            ["elf", "gnome", "tiefling"]
        );
    }

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

    #[test]
    fn every_srd_pack_lists_contents_that_exist_in_the_catalog_from_pages_96_to_99() {
        let catalog = catalog().expect("source-backed catalog");
        let gear: Vec<&str> = catalog.items.iter().map(|item| item.id.as_str()).collect();
        let packs: Vec<&Item> = catalog
            .items
            .iter()
            .filter(|item| item.kind == "pack")
            .collect();
        assert_eq!(packs.len(), 7);
        for pack in packs {
            assert!(!pack.contents.is_empty(), "{}", pack.id);
            for content in &pack.contents {
                assert!(
                    gear.contains(&content.item.as_str()),
                    "{}: {}",
                    pack.id,
                    content.item
                );
                assert!(content.quantity > 0, "{}: {}", pack.id, content.item);
            }
        }
    }

    #[test]
    fn the_dungeoneers_pack_kit_row_names_the_pack_that_holds_rope_torches_and_a_waterskin() {
        let catalog = catalog().expect("source-backed catalog");
        let pack = catalog
            .items
            .iter()
            .find(|item| item.kit_item.as_deref() == Some("dungeoneer-pack"))
            .expect("the starting kit's dungeoneer pack is a catalog pack");
        assert_eq!(pack.id, "dungeoneers-pack");
        let quantity_of = |item: &str| {
            pack.contents
                .iter()
                .find(|content| content.item == item)
                .map(|content| content.quantity)
        };
        assert_eq!(quantity_of("waterskin"), Some(1));
        assert_eq!(quantity_of("rope"), Some(1));
        assert_eq!(quantity_of("rations"), Some(10));
        assert_eq!(quantity_of("torch"), Some(10));
        assert_eq!(quantity_of("tinderbox"), Some(1));
    }
}
