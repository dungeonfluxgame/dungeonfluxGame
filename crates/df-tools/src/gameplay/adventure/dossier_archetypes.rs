//! Versioned, reusable old-world roles and narrative-only character flavor.
//!
//! Role profiles supply grounded occupational habits to authored NPC dossiers. Flavor axes are
//! selected from independent seeded streams, so changing this layer does not shift any existing
//! dossier roll. Every selected value is copied into the dossier for stable reloads.
use super::{
    Roll, RollStatus,
    dossier::{DossierError, ResolvedArchetype, ResolvedFlavor},
    rng::Dice,
    schema::Table,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const FILE: &str = "npc-archetypes.json";
const SCHEMA: &str = "df-npc-archetypes/1";
const VERSION: u16 = 1;
const ARCHETYPE_STREAM_TAG: u64 = 0x4152_4348_4554_5950;
const DICE: [u8; 6] = [4, 6, 8, 10, 12, 20];

#[derive(Debug)]
pub(super) struct ArchetypeLibrary {
    version: u16,
    profiles: Vec<RoleProfile>,
    by_key: BTreeMap<String, usize>,
    by_label: BTreeMap<String, usize>,
    flavors: FlavorAxes,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchetypeDocument {
    schema: String,
    version: u16,
    description: String,
    archetypes: Vec<RoleProfile>,
    flavors: FlavorAxes,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleProfile {
    id: String,
    category: String,
    label: String,
    aliases: Vec<String>,
    instincts: Vec<String>,
    priorities: Vec<String>,
    boundaries: Vec<String>,
    knowledge_domains: Vec<String>,
    speech_habits: Vec<String>,
    typical_actions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FlavorAxes {
    experience: Table<FlavorEntry>,
    competence: Table<FlavorEntry>,
    temperament: Table<FlavorEntry>,
    motive: Table<FlavorEntry>,
    style: Table<FlavorEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FlavorEntry {
    faces: [u8; 2],
    id: String,
    label: String,
    guidance: String,
}

impl ArchetypeLibrary {
    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let document: ArchetypeDocument = parse(bytes)?;
        if document.schema != SCHEMA || document.version != VERSION {
            return Err("unsupported archetype schema or version".to_owned());
        }
        if document.description.trim().is_empty() {
            return Err("description is empty".to_owned());
        }
        if document.archetypes.is_empty() {
            return Err("archetype library must contain at least one profile".to_owned());
        }

        let mut profile_ids = BTreeSet::new();
        let mut by_key = BTreeMap::new();
        let mut by_label = BTreeMap::new();
        for (index, profile) in document.archetypes.iter().enumerate() {
            let path = format!("archetypes[{index}]");
            if !valid_id(&profile.id) || !profile_ids.insert(profile.id.as_str()) {
                return Err(format!("{path}.id must be unique kebab-case"));
            }
            check_text(&format!("{path}.category"), &profile.category, 48)?;
            check_text(&format!("{path}.label"), &profile.label, 64)?;
            let label_key = normalize_key(&profile.label);
            if !valid_id(&label_key) {
                return Err(format!("{path}.label must normalize to a role key"));
            }
            if by_label.insert(label_key, index).is_some() {
                return Err(format!("{path}.label normalizes to a duplicate role label"));
            }
            for (field, values) in [
                ("instincts", &profile.instincts),
                ("priorities", &profile.priorities),
                ("boundaries", &profile.boundaries),
                ("knowledge_domains", &profile.knowledge_domains),
                ("speech_habits", &profile.speech_habits),
                ("typical_actions", &profile.typical_actions),
            ] {
                if !(1..=3).contains(&values.len()) {
                    return Err(format!("{path}.{field} must have one to three cues"));
                }
                for (cue_index, cue) in values.iter().enumerate() {
                    check_text(&format!("{path}.{field}[{cue_index}]"), cue, 180)?;
                }
            }
            let mut keys = profile.aliases.clone();
            keys.push(profile.id.clone());
            let mut local_keys = BTreeSet::new();
            for key in &keys {
                if !valid_id(key) || !local_keys.insert(key.to_owned()) {
                    return Err(format!(
                        "{path}.aliases contains an invalid or duplicate key"
                    ));
                }
            }
            for key in keys {
                if by_key.get(&key).is_some_and(|other| *other != index) {
                    return Err(format!("{path}.aliases conflicts with another role key"));
                }
                by_key.insert(key, index);
            }
        }
        validate_axis("experience", &document.flavors.experience)?;
        validate_axis("competence", &document.flavors.competence)?;
        validate_axis("temperament", &document.flavors.temperament)?;
        validate_axis("motive", &document.flavors.motive)?;
        validate_axis("style", &document.flavors.style)?;

        Ok(Self {
            version: document.version,
            profiles: document.archetypes,
            by_key,
            by_label,
            flavors: document.flavors,
        })
    }

    /// Resolves by the generator's stable archetype id first, then by the normalized role label.
    /// Each flavor axis has its own NPC- and axis-keyed stream; the main dossier stream is untouched.
    pub(super) fn compose(
        &self,
        archetype_id: &str,
        role_label: &str,
        seed: u64,
        npc_id: &str,
    ) -> Result<Option<(ResolvedArchetype, Vec<Roll>)>, DossierError> {
        let role_key = normalize_key(role_label);
        let index = self.by_key.get(&normalize_key(archetype_id)).or_else(|| {
            self.by_label
                .get(&role_key)
                .or_else(|| self.by_key.get(&role_key))
        });
        let Some(index) = index else {
            return Ok(None);
        };
        let profile = self
            .profiles
            .get(*index)
            .ok_or_else(|| DossierError::NoCompatibleEntry {
                npc_id: npc_id.to_owned(),
                table: "archetype.profile".to_owned(),
            })?;
        let mut rolls = Vec::with_capacity(5);
        let experience = pick_flavor(
            &self.flavors.experience,
            "experience",
            seed,
            npc_id,
            &mut rolls,
        )?;
        let competence = pick_flavor(
            &self.flavors.competence,
            "competence",
            seed,
            npc_id,
            &mut rolls,
        )?;
        let temperament = pick_flavor(
            &self.flavors.temperament,
            "temperament",
            seed,
            npc_id,
            &mut rolls,
        )?;
        let motive = pick_flavor(&self.flavors.motive, "motive", seed, npc_id, &mut rolls)?;
        let style = pick_flavor(&self.flavors.style, "style", seed, npc_id, &mut rolls)?;
        Ok(Some((
            ResolvedArchetype {
                library_version: self.version,
                source_id: profile.id.clone(),
                category: profile.category.clone(),
                label: profile.label.clone(),
                instincts: profile.instincts.clone(),
                priorities: profile.priorities.clone(),
                boundaries: profile.boundaries.clone(),
                knowledge_domains: profile.knowledge_domains.clone(),
                speech_habits: profile.speech_habits.clone(),
                typical_actions: profile.typical_actions.clone(),
                experience,
                competence,
                temperament,
                motive,
                style,
            },
            rolls,
        )))
    }

    #[cfg(test)]
    pub(super) fn archetype_count(&self) -> usize {
        self.profiles.len()
    }
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut parser)
        .map_err(|error| format!("{}: {}", error.path(), error.inner()))?;
    parser.end().map_err(|error| error.to_string())?;
    Ok(value)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn check_text(path: &str, value: &str, maximum: usize) -> Result<(), String> {
    let length = value.chars().count();
    if length == 0
        || length > maximum
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "{path} must contain 1 to {maximum} plain characters"
        ));
    }
    Ok(())
}

fn validate_axis(name: &str, axis: &Table<FlavorEntry>) -> Result<(), String> {
    if !DICE.contains(&axis.die) || !(8..=usize::from(axis.die)).contains(&axis.entries.len()) {
        return Err(format!(
            "flavors.{name} must have 8 or more entries on a standard die"
        ));
    }
    let mut next = 1u8;
    let mut ids = BTreeSet::new();
    for (index, entry) in axis.entries.iter().enumerate() {
        let [first, last] = entry.faces;
        if first != next || last < first || last > axis.die {
            return Err(format!(
                "flavors.{name}.entries[{index}].faces must tile the die without gaps"
            ));
        }
        next = last.saturating_add(1);
        if !valid_id(&entry.id) || !ids.insert(entry.id.as_str()) {
            return Err(format!("flavors.{name} has an invalid or duplicate id"));
        }
        check_text(
            &format!("flavors.{name}.entries[{index}].label"),
            &entry.label,
            64,
        )?;
        check_text(
            &format!("flavors.{name}.entries[{index}].guidance"),
            &entry.guidance,
            180,
        )?;
    }
    if next != axis.die + 1 {
        return Err(format!("flavors.{name} does not cover its die"));
    }
    Ok(())
}

fn normalize_key(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !normalized.is_empty() {
                normalized.push('-');
            }
            normalized.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    normalized
}

fn pick_flavor(
    axis: &Table<FlavorEntry>,
    name: &str,
    seed: u64,
    npc_id: &str,
    rolls: &mut Vec<Roll>,
) -> Result<ResolvedFlavor, DossierError> {
    let mut dice = Dice::new(seed ^ ARCHETYPE_STREAM_TAG ^ hash(npc_id) ^ hash(name));
    let natural = dice.roll(axis.die);
    let entry = axis
        .entries
        .iter()
        .find(|entry| entry.faces[0] <= natural && natural <= entry.faces[1])
        .ok_or_else(|| DossierError::NoCompatibleEntry {
            npc_id: npc_id.to_owned(),
            table: format!("archetype.{name}"),
        })?;
    rolls.push(Roll {
        n: 0,
        table: format!("dossier.archetype.{name}"),
        die: axis.die,
        natural,
        entry_id: entry.id.clone(),
        status: RollStatus::Ok,
    });
    Ok(ResolvedFlavor {
        id: entry.id.clone(),
        label: entry.label.clone(),
        guidance: entry.guidance.clone(),
    })
}

fn hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY: &str =
        include_str!("../../../../../content/adventure-tables/npc-archetypes.json");

    fn library() -> ArchetypeLibrary {
        ArchetypeLibrary::from_bytes(LIBRARY.as_bytes()).unwrap()
    }

    #[test]
    fn library_is_broad_and_has_composable_flavor_axes() {
        let library = library();
        assert!(library.archetype_count() >= 200);
        for seed in 0..128 {
            let profile = library
                .compose("merchant", "cloth merchant", seed, "patron:merchant")
                .unwrap()
                .unwrap()
                .0;
            assert_eq!(profile.source_id, "cloth-merchant");
            assert!(!profile.knowledge_domains.is_empty());
            assert!(!profile.experience.guidance.is_empty());
            assert!(!profile.competence.guidance.is_empty());
        }
    }

    #[test]
    fn occupations_have_distinct_cues_and_resolve_by_id_before_label() {
        let library = library();
        let herb_gatherer = library
            .compose("herbwife", "archivist", 4, "patron:herbwife")
            .unwrap()
            .unwrap()
            .0;
        let archivist = library
            .compose("archivist", "herb gatherer", 4, "contact:archivist")
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(herb_gatherer.source_id, "herb-gatherer");
        assert_eq!(archivist.source_id, "archivist");
        assert_ne!(herb_gatherer.instincts, archivist.instincts);
        assert_ne!(herb_gatherer.knowledge_domains, archivist.knowledge_domains);
        assert_ne!(herb_gatherer.typical_actions, archivist.typical_actions);
        let adventurer = library
            .compose("adventurer", "adventurer", 4, "bystander:shop-adventurer")
            .unwrap()
            .unwrap()
            .0;
        let soldier = library
            .compose("soldier", "soldier", 4, "bystander:shop-soldier")
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(adventurer.source_id, "adventurer");
        assert_eq!(soldier.source_id, "soldier");
        assert_ne!(adventurer.priorities, soldier.priorities);
        assert_ne!(adventurer.knowledge_domains, soldier.knowledge_domains);
        let ranger_by_label = library
            .compose("unmapped-cast-id", "ranger and guide", 4, "contact:ranger")
            .unwrap()
            .unwrap()
            .0;
        assert_eq!(ranger_by_label.source_id, "ranger-guide");
    }

    #[test]
    fn experience_and_competence_are_independent_narrative_flavor() {
        let library = library();
        let mut experience = BTreeSet::new();
        let mut competence = BTreeSet::new();
        for seed in 0..128 {
            let profile = library
                .compose(
                    "watch-sergeant",
                    "watch sergeant",
                    seed,
                    "contact:watch-sergeant",
                )
                .unwrap()
                .unwrap()
                .0;
            experience.insert(profile.experience.id);
            competence.insert(profile.competence.id);
        }
        assert!(experience.contains("new-to-the-work"));
        assert!(experience.contains("veteran"));
        assert!(competence.contains("struggling"));
        assert!(competence.contains("exceptional-craft"));
    }

    #[test]
    fn axis_streams_are_stable_per_npc_and_do_not_follow_call_order() {
        let library = library();
        let first = library
            .compose(
                "watch-sergeant",
                "watch sergeant",
                17,
                "contact:watch-sergeant",
            )
            .unwrap()
            .unwrap();
        let repeat = library
            .compose(
                "watch-sergeant",
                "watch sergeant",
                17,
                "contact:watch-sergeant",
            )
            .unwrap()
            .unwrap();
        let other = library
            .compose(
                "watch-sergeant",
                "watch sergeant",
                17,
                "bystander:watch-sergeant",
            )
            .unwrap()
            .unwrap();
        assert_eq!(first, repeat);
        assert_ne!(first.0, other.0);
        assert_eq!(first.1.len(), 5);
        assert!(
            first
                .1
                .iter()
                .all(|roll| roll.table.starts_with("dossier.archetype."))
        );
    }
}
