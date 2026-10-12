//! Pure, server-owned chapter generation; governing design: adventure-generator §§1–12.
#[cfg(test)]
mod beat_generation_tests;
mod beat_tables;
mod bounds;
pub(crate) mod campaign;
pub(crate) mod campaign_completion;
pub(crate) mod campaign_render;
pub(crate) mod campaign_startup;
pub(crate) mod campaign_store;
#[cfg(test)]
mod campaign_store_tests;
pub(crate) mod campaign_tables;
#[cfg(test)]
mod campaign_tests;
pub(crate) mod cli;
pub(crate) mod dossier;
mod dossier_cast;
pub(crate) mod dossier_render;
pub(crate) mod dossier_tables;
#[cfg(test)]
mod dossier_tests;
mod expression;
mod generate;
mod listener;
mod mature_overlay;
pub(crate) mod record;
mod render;
mod rng;
mod schema;
pub(crate) mod setting;
pub(crate) mod startup;
mod tables;
#[cfg(test)]
mod tests;

use super::{
    pack::{PackFile, TuningFile},
    startup_configuration::ContentRating,
};
pub(crate) use generate::{generate, generate_with_prose};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};
pub(crate) use tables::{Tables, TablesError};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RulesMode {
    #[default]
    Standard,
    CustomAllowed,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GenerateOptions {
    pub(crate) rules_mode: RulesMode,
    /// Which content-rating variant of the tables rolls the story text and the NPC backstories.
    pub(crate) rating: ContentRating,
}
/// Keys are recorded table names; values are entry IDs, name text or "variant N".
/// Original variant text is also accepted, for authoring tools that already hold the table.
/// Forced values still pass coherence filters, consume no entropy and are recorded as forced.
pub(crate) type ForcedEntries = BTreeMap<String, String>;
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RollStatus {
    Ok,
    Rejected,
    Fallback,
    Forced,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Roll {
    pub(crate) n: usize,
    pub(crate) table: String,
    pub(crate) die: u8,
    pub(crate) natural: u8,
    pub(crate) entry_id: String,
    pub(crate) status: RollStatus,
}
/// One bounded expression request. Mechanical text bypasses this interface entirely.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct FactSheet {
    pub(crate) beat: String,
    pub(crate) slot: String,
    pub(crate) speaker: String,
    pub(crate) voice: String,
    pub(crate) delivery: String,
    pub(crate) facts: BTreeMap<String, String>,
    pub(crate) must_mention: Vec<String>,
    pub(crate) forbidden: Vec<String>,
    pub(crate) bounds: usize,
    pub(crate) baseline: String,
    pub(crate) tone: String,
}
/// Substitutes expression only, after dice fix facts and before output bytes are frozen.
/// Implementations must preserve request facts; returned prose is checked against the slot bound.
pub(crate) trait ProseSource {
    fn express(&self, sheet: &FactSheet) -> Result<String, GenerateError>;
    /// Fail closed until a model source implements all section 7 semantic validators.
    /// Changing expression never grants authority to alter the rolled facts or mechanics.
    fn validate(&self, sheet: &FactSheet, text: &str) -> bool {
        text == sheet.baseline
    }
}
pub(crate) struct PatternProse;
impl ProseSource for PatternProse {
    fn express(&self, sheet: &FactSheet) -> Result<String, GenerateError> {
        Ok(sheet.baseline.clone())
    }
}
#[derive(Clone, Debug)]
pub(crate) struct GeneratedAdventure {
    pub(crate) seed: u64,
    pub(crate) tables_revision: String,
    pub(crate) rules_mode: RulesMode,
    pub(crate) rating: ContentRating,
    pub(crate) facts: BTreeMap<String, String>,
    pub(crate) fact_sheet: Vec<FactSheet>,
    pub(crate) rolls: Vec<Roll>,
    pub(crate) pack: PackFile,
    pub(crate) tuning: TuningFile,
    pub(crate) settings: setting::ChapterSettings,
    /// The speaking cast's backstories, rolled on their own streams (section 19).
    pub(crate) dossiers: Vec<dossier::Dossier>,
}
#[derive(Clone, Debug)]
pub(crate) enum GenerateError {
    MissingTable(String),
    Incompatible(String),
    UnknownForced(String),
    Pattern(String),
    Bounds {
        slot: String,
        actual: usize,
        maximum: usize,
    },
    Pack(super::pack::PackError),
    Dossier(dossier::DossierError),
    Serialization,
}
impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTable(table) => write!(f, "adventure table missing: {table}"),
            Self::Incompatible(table) => {
                write!(f, "adventure table has no compatible entry: {table}")
            }
            Self::UnknownForced(table) => write!(
                f,
                "adventure forced entry is unknown or incompatible: {table}"
            ),
            Self::Pattern(path) => write!(f, "adventure pattern has unknown placeholder: {path}"),
            Self::Bounds {
                slot,
                actual,
                maximum,
            } => write!(f, "adventure {slot}: {actual} characters exceeds {maximum}"),
            Self::Pack(error) => error.fmt(f),
            Self::Dossier(error) => error.fmt(f),
            Self::Serialization => f.write_str("adventure serialization failed"),
        }
    }
}
impl std::error::Error for GenerateError {}
/// Manifest-pinned generated route paths; never accepts arbitrary stems or filesystem paths.
pub(crate) fn generated_asset(path: &str) -> bool {
    generated_stems()
        .iter()
        .any(|stem| path == format!("assets/ui/generated/{stem}"))
}
pub(crate) fn generated_stems() -> &'static [String] {
    static STEMS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    STEMS.get_or_init(|| {
        // Internal invariant: compiled manifest parsed and duplicate/path checked by tests.
        let art: Vec<tables::Art> = serde_json::from_slice(include_bytes!(
            "../../../../../content/adventure-tables/art-manifest.json"
        ))
        .expect("compiled art manifest");
        art.into_iter().map(|entry| entry.file_stem).collect()
    })
}
