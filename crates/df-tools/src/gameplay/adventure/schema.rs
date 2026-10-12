use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Table<T> {
    pub(super) die: u8,
    pub(super) entries: Vec<T>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TextEntry {
    pub(super) faces: [u8; 2],
    pub(super) text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PatternTable {
    pub(super) die: Option<u8>,
    pub(super) variants: Vec<TextEntry>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document<T> {
    pub(super) schema: String,
    pub(super) file: String,
    pub(super) description: String,
    pub(super) tables: BTreeMap<String, T>,
    #[serde(default)]
    pub(super) attribution: Option<String>,
    #[serde(default)]
    pub(super) source: Option<Source>,
    #[serde(default)]
    pub(super) hero_baseline: Option<HeroBaseline>,
    #[serde(default)]
    pub(super) mechanical_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(super) limits: Option<BTreeMap<String, usize>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    pub(super) file: String,
    pub(super) sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HeroBaseline {
    pub(super) armor_class: u8,
    pub(super) hit_points: u8,
    pub(super) attack_bonus: i8,
    pub(super) weapon: String,
    pub(super) count: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtPair {
    pub(super) arrival: String,
    pub(super) interior: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Location {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) name: String,
    pub(super) title: String,
    pub(super) short: String,
    pub(super) tags: Vec<String>,
    pub(super) art: ArtPair,
    pub(super) atmosphere: Table<TextEntry>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NpcArchetype {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) role: String,
    pub(super) desc: String,
    pub(super) gender: String,
    /// One of `young`, `adult`, `elder`; agrees with `voice` and the portrait.
    pub(super) age: String,
    pub(super) voice: String,
    pub(super) fits: Vec<String>,
    pub(super) art: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Hook {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) fits: Vec<String>,
    pub(super) stake_tags: Vec<String>,
    pub(super) title: String,
    pub(super) entrance: String,
    pub(super) pitch: String,
    pub(super) answer: String,
    pub(super) accept: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Stake {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) full: String,
    pub(super) short: String,
    pub(super) title: String,
    pub(super) tags: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SrdCitation {
    pub(super) name: String,
    pub(super) section: String,
    pub(super) page: u16,
    pub(super) cr: String,
    pub(super) block: String,
    pub(super) license: String,
    pub(super) pdf_sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WeaponStats {
    pub(super) label: String,
    pub(super) damage_dice: u8,
    pub(super) damage_sides: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreatureStats {
    pub(super) hit_points: u8,
    pub(super) armor_class: u8,
    pub(super) abilities: super::super::pack::AbilityScores,
    pub(super) attack_bonus: i8,
    pub(super) weapon: WeaponStats,
    pub(super) damage_modifier: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Antagonist {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) label: String,
    pub(super) srd: SrdCitation,
    pub(super) stats: CreatureStats,
    pub(super) fidelity: String,
    pub(super) fidelity_note: String,
    pub(super) klass: String,
    pub(super) speaks: bool,
    pub(super) gender: String,
    /// One of `young`, `adult`, `elder`.
    pub(super) age: String,
    pub(super) voice: String,
    pub(super) fits: Vec<String>,
    pub(super) name_form: String,
    pub(super) the: String,
    pub(super) noun: String,
    pub(super) art: String,
    pub(super) arrivals: PatternTable,
    pub(super) warnings: Option<PatternTable>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Complication {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    #[serde(rename = "for")]
    pub(super) groups: Vec<String>,
    pub(super) text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Secret {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) kinds: Vec<String>,
    pub(super) fact: String,
    pub(super) confirm: String,
    /// Phrases that give the secret away; the first occurs verbatim in the rendered fact.
    /// A listener who has not been told the secret must never hear them.
    #[serde(default)]
    pub(super) terms: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Destination {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) name: String,
    pub(super) title: String,
    pub(super) short: String,
    pub(super) arrival: String,
}
