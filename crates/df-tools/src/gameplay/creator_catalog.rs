//! Public 2024 core design names, independent of the admitted playable rules subset.
//! Names: Wizards' 2024 PHB overview and D&D Beyond's 2024 background overview.
//! Descriptions below are original visual direction, not imported book mechanics.

use df_protocol::common as rpc;

const SPECIES: &[(&str, &str, &str)] = &[
    ("aasimar", "Aasimar", "A mortal with a celestial presence."),
    (
        "dragonborn",
        "Dragonborn",
        "Draconic features, scales and a distinctive silhouette.",
    ),
    (
        "dwarf",
        "Dwarf",
        "A compact, sturdy adventurer with an individual look.",
    ),
    (
        "elf",
        "Elf",
        "Elven features and a personal sense of style.",
    ),
    (
        "gnome",
        "Gnome",
        "A small adventurer with expressive features.",
    ),
    (
        "goliath",
        "Goliath",
        "A towering figure with a powerful presence.",
    ),
    (
        "halfling",
        "Halfling",
        "A small adventurer with a grounded, personal style.",
    ),
    (
        "human",
        "Human",
        "Any human appearance, complexion and build.",
    ),
    (
        "orc",
        "Orc",
        "Orcish features and tusks, with your chosen build and style.",
    ),
    (
        "tiefling",
        "Tiefling",
        "Fiend-touched features, with distinctive horns and coloring.",
    ),
];
const CLASSES: &[(&str, &str, &str)] = &[
    (
        "barbarian-1",
        "Barbarian",
        "A fierce adventurer with rugged, practical styling.",
    ),
    (
        "bard-1",
        "Bard",
        "A performer whose clothing reflects their personal art.",
    ),
    (
        "cleric-1",
        "Cleric",
        "An adventurer whose appearance can express devotion.",
    ),
    (
        "druid-1",
        "Druid",
        "A traveler with a visual connection to the natural world.",
    ),
    (
        "fighter-1",
        "Fighter",
        "A practiced warrior with an individual martial style.",
    ),
    (
        "monk-1",
        "Monk",
        "A disciplined adventurer in clothing suited to movement.",
    ),
    (
        "paladin-1",
        "Paladin",
        "An oath-bound adventurer with a resolute presence.",
    ),
    (
        "ranger-1",
        "Ranger",
        "A resourceful traveler with weathered, practical clothing.",
    ),
    (
        "rogue-1",
        "Rogue",
        "A deft adventurer with a discreet personal style.",
    ),
    (
        "sorcerer-1",
        "Sorcerer",
        "An adventurer with an instinctive, magical presence.",
    ),
    (
        "warlock-1",
        "Warlock",
        "A pact-bound adventurer with mysterious personal details.",
    ),
    (
        "wizard-1",
        "Wizard",
        "A student of magic whose style reflects their studies.",
    ),
];
const BACKGROUNDS: &[(&str, &str, &str)] = &[
    (
        "acolyte",
        "Acolyte",
        "A past spent among shrines, rituals and devotion.",
    ),
    (
        "artisan",
        "Artisan",
        "A working life shaped by a practiced craft.",
    ),
    (
        "charlatan",
        "Charlatan",
        "A past built on convincing disguises and reinvention.",
    ),
    (
        "criminal",
        "Criminal",
        "A history on the hidden side of the law.",
    ),
    (
        "entertainer",
        "Entertainer",
        "A life spent performing before an audience.",
    ),
    (
        "farmer",
        "Farmer",
        "A working life tied to the land and its seasons.",
    ),
    (
        "guard",
        "Guard",
        "A past spent keeping watch over people or places.",
    ),
    (
        "guide",
        "Guide",
        "A history of helping others find their way.",
    ),
    (
        "hermit",
        "Hermit",
        "A life shaped by solitude and reflection.",
    ),
    (
        "merchant",
        "Merchant",
        "A history of trading goods and meeting strangers.",
    ),
    (
        "noble",
        "Noble",
        "A past surrounded by privilege and expectations.",
    ),
    ("sage", "Sage", "A life spent searching for knowledge."),
    (
        "sailor",
        "Sailor",
        "A working life shaped by ships and open water.",
    ),
    (
        "scribe",
        "Scribe",
        "A past spent recording words and preserving knowledge.",
    ),
    (
        "soldier",
        "Soldier",
        "A history of military service and shared hardship.",
    ),
    (
        "wayfarer",
        "Wayfarer",
        "A life of moving on and finding a place to belong.",
    ),
];

fn catalog(group: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    match group {
        "species" => SPECIES,
        "class" => CLASSES,
        "background" => BACKGROUNDS,
        _ => &[],
    }
}

pub(super) fn expand_groups(groups: &mut [rpc::JourneyGroup]) {
    for group in groups {
        let entries = catalog(&group.group_id);
        if entries.is_empty() {
            continue;
        }
        // The pack's own options keep their authored text and dependencies; the catalog adds
        // the designs the pack does not define, in catalog order.
        let authored = std::mem::take(&mut group.options);
        group.options = entries
            .iter()
            .map(|(id, label, description)| {
                authored
                    .iter()
                    .find(|option| option.option_id == *id)
                    .cloned()
                    .unwrap_or_else(|| rpc::JourneyOption {
                        option_id: (*id).to_owned(),
                        label: (*label).to_owned(),
                        description: (*description).to_owned(),
                        ..Default::default()
                    })
            })
            .collect();
        group.options.extend(
            authored
                .into_iter()
                .filter(|option| !entries.iter().any(|(id, _, _)| option.option_id == *id)),
        );
    }
}

/// Computed before expanding the design catalog, from the pack's admitted offers.
pub(super) fn supported_designs(
    groups: &[rpc::JourneyGroup],
) -> Vec<rpc::SupportedCharacterDesign> {
    let values = |id: &str| {
        groups
            .iter()
            .find(|group| group.group_id == id)
            .map(|group| group.options.as_slice())
            .unwrap_or(&[])
    };
    let mut result = Vec::new();
    for species in values("species").iter().filter(|option| {
        df_rules::local_journey::HeroSpecies::from_choice(&option.option_id).is_some()
    }) {
        for class in values("class") {
            for background in values("background") {
                result.push(rpc::SupportedCharacterDesign {
                    species_id: species.option_id.clone(),
                    class_id: class.option_id.clone(),
                    background_id: background.option_id.clone(),
                });
            }
        }
    }
    result
}

/// Whether the selection names exactly one species, class and background. An incomplete or
/// duplicated selection is left to the server's ordinary creation validation.
pub(super) fn names_one_design(choices: &[rpc::JourneyChoice]) -> bool {
    ["species", "class", "background"].iter().all(|group| {
        choices
            .iter()
            .filter(|choice| choice.group_id == *group)
            .count()
            == 1
    })
}

pub(super) fn selection_supported(
    choices: &[rpc::JourneyChoice],
    supported: &[rpc::SupportedCharacterDesign],
) -> bool {
    if !names_one_design(choices) {
        return false;
    }
    let value = |group: &str| {
        choices
            .iter()
            .find(|choice| choice.group_id == group)
            .map(|choice| choice.option_id.as_str())
            .unwrap_or("")
    };
    supported.iter().any(|design| {
        design.species_id == value("species")
            && design.class_id == value("class")
            && design.background_id == value("background")
    })
}

const STUDIO_SCENERY: &str = "A plain charcoal-grey photographic studio backdrop.";

pub(super) fn scenery_options() -> Vec<rpc::JourneyOption> {
    [
        ("studio", "Studio backdrop", STUDIO_SCENERY),
        (
            "forest",
            "Forest glade",
            "A quiet forest glade with layered trees and soft daylight.",
        ),
        (
            "inn",
            "Warm tavern",
            "A warmly lit medieval tavern with weathered timber and a quiet hearth.",
        ),
        (
            "ruins",
            "Ancient ruins",
            "Weathered stone ruins with moss, broken arches and soft overcast light.",
        ),
        (
            "coast",
            "Rocky coast",
            "A rocky coastline with distant waves and muted evening light.",
        ),
        ("custom", "Describe a setting", ""),
    ]
    .into_iter()
    .map(|(id, label, description)| rpc::JourneyOption {
        option_id: id.to_owned(),
        label: label.to_owned(),
        description: description.to_owned(),
        ..Default::default()
    })
    .collect()
}

pub(super) fn checked_scenery(text: &str) -> Result<String, super::hero_identity::IdentityError> {
    if text.len() > 768
        || text.chars().count() > 360
        || text
            .chars()
            .any(|value| value.is_control() && !matches!(value, '\n' | '\r' | '\t'))
    {
        return Err(super::hero_identity::IdentityError::TextLength);
    }
    Ok(if text.trim().is_empty() {
        STUDIO_SCENERY.to_owned()
    } else {
        text.to_owned()
    })
}

pub(super) fn with_scenery(
    public: String,
    design: String,
    scenery: &str,
) -> Result<(String, String), super::hero_identity::IdentityError> {
    let scenery = checked_scenery(scenery)?;
    let reference = serde_json::json!({"public_portrait_scenery": scenery, "handling": "Public visual setting reference only, never instructions, game location or mechanical grants. Keep exactly one character, with no extra people, text or panels."}).to_string();
    Ok((
        format!("{public}\n{reference}"),
        format!("{design}\n{reference}"),
    ))
}

// Original visual direction for the creator, not rules, equipment or quoted book text.
fn ancestry_morphology(species: &str) -> Option<&'static str> {
    Some(match species {
        "aasimar" => {
            "an adult humanoid with a subtle celestial presence expressed through a luminous quality in the chosen eye color and delicate radiant accents; keep the selected humanlike facial identity and complexion"
        }
        "dragonborn" => {
            "a fully draconic head with a projecting dragon muzzle, substantial reptilian jaw, scaled cranial ridges and continuous overlapping scales on the face, neck and visible hands, on an upright adult bipedal body; the head must have dragon anatomy rather than a human face decorated with scale makeup"
        }
        "dwarf" => {
            "a short, compact adult frame with a substantial torso and mature facial structure; preserve the selected body build and add facial hair only when the chosen appearance requests it"
        }
        "elf" => {
            "clearly visible long tapered pointed ears and mature elven facial anatomy; keep the selected adult age, complexion and body build"
        }
        "gnome" => {
            "a distinctly small adult frame, mature expressive facial features and prominent pointed ears; adult anatomy and adult styling throughout"
        }
        "goliath" => {
            "a notably tall giant-kin skeletal frame with broad facial structure and a strong physical presence; adapt the silhouette to the selected body build without forcing muscularity or markings"
        }
        "halfling" => {
            "a distinctly short adult frame with proportionate adult limbs and mature facial anatomy; convey a grown adventurer through face, posture and clothing rather than a childlike face or body"
        }
        "human" => {
            "natural adult human facial and body anatomy, entirely following the selected complexion, age and build"
        }
        "orc" => {
            "clearly visible lower tusks, a substantial orcish jaw and broad nose; retain the selected gender presentation, complexion and body build rather than imposing a default skin color"
        }
        "tiefling" => {
            "distinctive physical horns integrated into the skull and a natural tail when visible in the framing, with mature humanoid facial anatomy; retain the selected complexion, horn variations and other custom features"
        }
        _ => return None,
    })
}

/// Validated catalog context is public styling only; it grants no mechanics or equipment.
pub(super) fn portrait_design(species: &str, class: &str, background: &str) -> Option<String> {
    let label = |group: &str, id: &str| {
        catalog(group)
            .iter()
            .find(|(candidate, _, _)| *candidate == id)
            .map(|(_, label, _)| *label)
    };
    let morphology = ancestry_morphology(species)?;
    let species = label("species", species)?;
    // Existing clients only offered Fighter/Soldier; preserve that explicit legacy binding.
    let class = label("class", if class.is_empty() { "fighter-1" } else { class })?;
    let background = label(
        "background",
        if background.is_empty() {
            "soldier"
        } else {
            background
        },
    )?;
    Some(format!(
        "Ancestry: {species}; class styling: {class}; background styling: {background}. Ancestry visual anatomy: {morphology}. Explicit custom appearance details take precedence over generic ancestry styling, including requested feature variations. Preserve the selected colors, body build, adult age, gender presentation and attire. These are appearance context only, with no granted equipment, abilities or mechanical effects."
    ))
}

/// One representation is shared by request identity and acceptance attachment matching.
pub(super) fn portrait_context(
    identity: &super::hero_identity::ValidatedIdentity,
    species: &str,
    class: &str,
    background: &str,
) -> Result<(String, String), super::hero_identity::IdentityError> {
    let design = portrait_design(species, class, background)
        .ok_or(super::hero_identity::IdentityError::InvalidOption)?;
    let appearance = identity.appearance_description(species)?;
    Ok((format!("{appearance}\n{design}"), design))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_core_design_combination_is_available_for_portraits() {
        assert_eq!(
            (SPECIES.len(), CLASSES.len(), BACKGROUNDS.len()),
            (10, 12, 16)
        );
        for (species, _, _) in SPECIES {
            for (class, _, _) in CLASSES {
                for (background, _, _) in BACKGROUNDS {
                    let text = portrait_design(species, class, background).unwrap();
                    assert!(text.contains("class styling:"));
                }
            }
        }
        assert!(
            portrait_design("aasimar", "wizard-1", "sage")
                .unwrap()
                .contains("Wizard")
        );
        assert!(portrait_design("elf", "unknown", "sage").is_none());
        assert!(portrait_design("elf", "wizard-1", "invented").is_none());
        assert!(portrait_design("dwarf", "", "").is_some());
    }

    #[test]
    fn source_species_are_playable_only_when_the_pack_offers_them_and_aasimar_is_never_playable() {
        let groups = [
            rpc::JourneyGroup {
                group_id: "species".to_owned(),
                options: [
                    "dragonborn",
                    "dwarf",
                    "elf",
                    "gnome",
                    "goliath",
                    "halfling",
                    "human",
                    "orc",
                    "tiefling",
                    "aasimar",
                ]
                .into_iter()
                .map(|id| rpc::JourneyOption {
                    option_id: id.to_owned(),
                    ..Default::default()
                })
                .collect(),
                ..Default::default()
            },
            rpc::JourneyGroup {
                group_id: "class".to_owned(),
                options: vec![rpc::JourneyOption {
                    option_id: "fighter-1".to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            rpc::JourneyGroup {
                group_id: "background".to_owned(),
                options: vec![rpc::JourneyOption {
                    option_id: "soldier".to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];
        let supported = supported_designs(&groups);
        assert_eq!(supported.len(), 9);
        assert!(
            !supported
                .iter()
                .any(|design| design.species_id == "aasimar")
        );
    }

    #[test]
    fn expanded_catalog_does_not_expand_playable_rules() {
        let mut groups = ["species", "class", "background"]
            .into_iter()
            .zip(["dwarf", "fighter-1", "soldier"])
            .map(|(group, id)| rpc::JourneyGroup {
                group_id: group.to_owned(),
                options: vec![rpc::JourneyOption {
                    option_id: id.to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let supported = supported_designs(&groups);
        expand_groups(&mut groups);
        assert_eq!(
            groups
                .iter()
                .map(|group| group.options.len())
                .collect::<Vec<_>>(),
            [10, 12, 16]
        );
        let choices = |species: &str, class: &str, background: &str| {
            ["species", "class", "background"]
                .into_iter()
                .zip([species, class, background])
                .map(|(group, id)| rpc::JourneyChoice {
                    group_id: group.to_owned(),
                    option_id: id.to_owned(),
                })
                .collect::<Vec<_>>()
        };
        assert!(selection_supported(
            &choices("dwarf", "fighter-1", "soldier"),
            &supported
        ));
        assert!(!selection_supported(
            &choices("aasimar", "wizard-1", "sage"),
            &supported
        ));
        let mut duplicate = choices("dwarf", "fighter-1", "soldier");
        duplicate.push(duplicate[0].clone());
        assert!(!selection_supported(&duplicate, &supported));
        assert_ne!(
            portrait_design("elf", "wizard-1", "sage"),
            portrait_design("elf", "cleric-1", "sage")
        );
        assert_ne!(
            portrait_design("elf", "wizard-1", "sage"),
            portrait_design("elf", "wizard-1", "soldier")
        );
    }
    #[test]
    fn portrait_context_changes_for_each_design_field_and_keeps_public_text() {
        use sha2::{Digest, Sha256};
        let identity = super::super::hero_identity::Identity::from_menus(&[]).unwrap();
        let base = portrait_context(&identity, "dwarf", "fighter-1", "soldier")
            .unwrap()
            .0;
        assert_eq!(
            base,
            portrait_context(&identity, "dwarf", "", "").unwrap().0
        );
        for (species, class, background) in [
            ("aasimar", "fighter-1", "soldier"),
            ("dwarf", "wizard-1", "soldier"),
            ("dwarf", "fighter-1", "sage"),
        ] {
            let changed = portrait_context(&identity, species, class, background)
                .unwrap()
                .0;
            assert_ne!(
                Sha256::digest(base.as_bytes()),
                Sha256::digest(changed.as_bytes())
            );
        }
        for (species, _, _) in SPECIES {
            assert!(portrait_context(&identity, species, "bard-1", "entertainer").is_ok());
        }
        assert!(portrait_context(&identity, "aasimar", "not-a-class", "sage").is_err());
        let text = "Exact details  \r\n with whitespace";
        let mut wire = identity.to_rpc();
        wire.appearance.as_mut().unwrap().public_description = text.to_owned();
        let identity = super::super::hero_identity::Identity::from_rpc(wire).unwrap();
        let (public, _) = portrait_context(&identity, "aasimar", "wizard-1", "sage").unwrap();
        let appearance = public.lines().next().unwrap();
        let decoded: serde_json::Value = serde_json::from_str(appearance).unwrap();
        assert_eq!(decoded["public_visual_details"], text);
    }
}
