//! Typed campaign authoring inputs; adventure-generator §§13–18 govern validation.
use super::{
    Tables, TablesError,
    schema::{Table, TextEntry},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::Read,
    path::Path,
};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Premise {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) family: String,
    pub(super) archetype: String,
    pub(super) titles: Vec<String>,
    pub(super) pitch: String,
    pub(super) clock: String,
    pub(super) villain_types: Vec<String>,
    pub(super) region_tags: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) name: String,
    pub(super) short: String,
    pub(super) form: String,
    pub(super) power: String,
    pub(super) hunger: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RegionName {
    pub(super) faces: [u8; 2],
    pub(super) name: String,
    pub(super) short: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SiteRole {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) text: String,
    pub(super) prefer: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VillainType {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) label: String,
    pub(super) gender: String,
    pub(super) families: Vec<String>,
    pub(super) voice: String,
    pub(super) art: String,
    pub(super) lieutenant_pref: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scheme {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) families: Vec<String>,
    pub(super) text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Weakness {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) text: String,
    pub(super) clues: [String; 2],
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Lieutenant {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) antagonist: String,
    pub(super) title: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Faction {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) name: String,
    pub(super) goal: String,
    pub(super) asset: String,
    pub(super) fits: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Attitude {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Beat {
    pub(super) hook_kind: String,
    pub(super) stake_tags: Vec<String>,
    pub(super) antagonist_role: String,
    pub(super) location_role: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChapterType {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) label: String,
    pub(super) intended: [u8; 3],
    pub(super) needs: Vec<String>,
    pub(super) v1: String,
    pub(super) beat: Option<Beat>,
    pub(super) asks: Vec<String>,
    pub(super) helps: Vec<String>,
    pub(super) stands: Vec<String>,
    pub(super) lens: String,
    pub(super) on_victory: Vec<String>,
    pub(super) on_defeat: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Slot {
    pub(super) act: String,
    pub(super) role: String,
    pub(super) allowed: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Act {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) name: String,
    pub(super) purpose: String,
    pub(super) stakes: String,
    pub(super) milestone: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChapterCount {
    pub(super) faces: [u8; 2],
    pub(super) chapters: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Node {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) label: String,
    pub(super) needed_by: String,
    pub(super) count: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LieutenantFate {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) text: String,
    pub(super) effect: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Clock {
    pub(super) die: Option<u8>,
    pub(super) steps: Vec<ClockStep>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClockStep {
    pub(super) step: u8,
    pub(super) text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Twist {
    pub(super) faces: [u8; 2],
    pub(super) id: String,
    pub(super) families: Vec<String>,
    pub(super) reveal: String,
    pub(super) hints: [String; 2],
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Clue {
    pub(super) faces: [u8; 2],
    pub(super) p: String,
    pub(super) r: String,
    pub(super) c: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PremiseTables {
    pub(super) premises: Table<Premise>,
    pub(super) artifacts: Table<Artifact>,
    pub(super) themes: Table<TextEntry>,
    pub(super) tones: Table<TextEntry>,
    pub(super) region_names: Table<RegionName>,
    pub(super) site_roles: Table<SiteRole>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VillainTables {
    pub(super) types: Table<VillainType>,
    pub(super) motives: Table<TextEntry>,
    pub(super) schemes: Table<Scheme>,
    pub(super) methods: Table<TextEntry>,
    pub(super) resources: Table<TextEntry>,
    pub(super) weaknesses: Table<Weakness>,
    pub(super) signatures: Table<TextEntry>,
    pub(super) lieutenants: Table<Lieutenant>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FactionTables {
    pub(super) factions: Table<Faction>,
    pub(super) attitude_toward_party: Table<Attitude>,
    pub(super) attitude_toward_villain: Table<Attitude>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChapterTables {
    pub(super) chapter_types: Table<ChapterType>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActTables {
    pub(super) acts: Table<Act>,
    pub(super) chapter_count: Table<ChapterCount>,
    pub(super) nodes: Table<Node>,
    pub(super) lieutenant_fate: Table<LieutenantFate>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClockTables {
    pub(super) clocks: BTreeMap<String, Clock>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TwistTables {
    pub(super) twists: Table<Twist>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClueTables {
    pub(super) clues: BTreeMap<String, Table<Clue>>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RewardTables {
    pub(super) chapter_lines: Table<TextEntry>,
    pub(super) milestones: Table<TextEntry>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Overlay {
    pub(super) chapter_title_prefix: Vec<String>,
    pub(super) room_title: Vec<String>,
    pub(super) recap_lead: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PremiseFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: PremiseTables,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VillainFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: VillainTables,
    pub(super) title_nouns: BTreeMap<String, String>,
    pub(super) name_tables: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FactionFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: FactionTables,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChapterFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: ChapterTables,
    pub(super) capability_names: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: ActTables,
    pub(super) slots: BTreeMap<String, Vec<Slot>>,
    pub(super) family_hooks: BTreeMap<String, Vec<String>>,
    pub(super) minion_rule: String,
    pub(super) rules: Vec<String>,
    pub(super) overlay: Overlay,
    pub(super) recap_budget: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClockFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: ClockTables,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TwistFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: TwistTables,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClueFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: ClueTables,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RewardFile {
    schema: String,
    file: String,
    description: String,
    pub(super) tables: RewardTables,
}

pub(crate) struct CampaignTables {
    pub(super) chapter: Tables,
    pub(super) premises: PremiseFile,
    pub(super) villains: VillainFile,
    pub(super) factions: FactionFile,
    pub(super) types: ChapterFile,
    pub(super) acts: ActFile,
    pub(super) clocks: ClockFile,
    pub(super) twists: TwistFile,
    pub(super) clues: ClueFile,
    pub(super) rewards: RewardFile,
}
#[derive(Debug)]
pub(crate) enum CampaignTablesError {
    Chapter(TablesError),
    Invalid {
        file: String,
        path: String,
        reason: String,
    },
    Io {
        file: String,
        kind: std::io::ErrorKind,
    },
}
impl fmt::Display for CampaignTablesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chapter(e) => e.fmt(f),
            Self::Invalid { file, path, reason } => {
                write!(f, "campaign table {file} at {path}: {reason}")
            }
            Self::Io { file, kind } => write!(f, "campaign table {file}: {kind}"),
        }
    }
}
impl std::error::Error for CampaignTablesError {}
fn invalid(file: &str, path: &str, reason: impl Into<String>) -> CampaignTablesError {
    CampaignTablesError::Invalid {
        file: file.into(),
        path: path.into(),
        reason: reason.into(),
    }
}
fn load<T: DeserializeOwned>(directory: &Path, file: &str) -> Result<T, CampaignTablesError> {
    let mut bytes = Vec::new();
    fs::File::open(directory.join(file))
        .and_then(|f| f.take(1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| CampaignTablesError::Io {
            file: file.into(),
            kind: e.kind(),
        })?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid(file, "$", "exceeds one MiB"));
    }
    let mut parser = serde_json::Deserializer::from_slice(&bytes);
    let value: T = serde_path_to_error::deserialize(&mut parser)
        .map_err(|e| invalid(file, &format!("$.{}", e.path()), e.inner().to_string()))?;
    parser
        .end()
        .map_err(|e| invalid(file, "$", e.to_string()))?;
    let tree: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| invalid(file, "$", e.to_string()))?;
    if tree.get("schema").and_then(|v| v.as_str()) != Some("df-adventure-tables/1")
        || tree.get("file").and_then(|v| v.as_str()) != Some(file.trim_end_matches(".json"))
    {
        return Err(invalid(file, "$.schema", "unsupported schema/file"));
    }
    lint(file, "$", &tree)?;
    Ok(value)
}
fn lint(file: &str, path: &str, value: &serde_json::Value) -> Result<(), CampaignTablesError> {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            if let Some(entries) = map.get("entries").and_then(Value::as_array) {
                let die = map
                    .get("die")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid(file, path, "missing die"))?;
                let expected = [1, 4, 6, 8, 10, 12, 20]
                    .into_iter()
                    .find(|d| *d >= entries.len() as u64)
                    .ok_or_else(|| invalid(file, path, "too many entries"))?;
                if entries.is_empty() || die != expected {
                    return Err(invalid(
                        file,
                        &format!("{path}.die"),
                        "entry count does not match named die",
                    ));
                }
                let mut next = 1;
                let mut ids = BTreeSet::new();
                for (n, entry) in entries.iter().enumerate() {
                    let faces = entry
                        .get("faces")
                        .and_then(Value::as_array)
                        .ok_or_else(|| invalid(file, path, "missing faces"))?;
                    let first = faces.first().and_then(Value::as_u64);
                    let last = faces
                        .get(1)
                        .and_then(Value::as_u64)
                        .ok_or_else(|| invalid(file, path, "missing last face"))?;
                    if first != Some(next) || last < next || last > die {
                        return Err(invalid(
                            file,
                            &format!("{path}.entries[{n}].faces"),
                            "faces do not partition die",
                        ));
                    }
                    next = last + 1;
                    if let Some(id) = entry.get("id").and_then(Value::as_str)
                        && !ids.insert(id)
                    {
                        return Err(invalid(file, path, "duplicate id"));
                    }
                }
                if next != die + 1 {
                    return Err(invalid(file, path, "faces do not cover die"));
                }
            }
            for (key, value) in map {
                lint(file, &format!("{path}.{key}"), value)?;
            }
        }
        Value::Array(values) => {
            for (n, value) in values.iter().enumerate() {
                lint(file, &format!("{path}[{n}]"), value)?;
            }
        }
        Value::String(text) => {
            let bound = if path.contains("description")
                || path.contains("rules")
                || path.contains("minion_rule")
            {
                1000
            } else {
                400
            };
            if text.chars().count() > bound || text.chars().any(|c| c.is_control() && c != '\n') {
                return Err(invalid(
                    file,
                    path,
                    "text exceeds bound or contains control characters",
                ));
            }
            let context = super::campaign_render::placeholder_context();
            super::render::render(text, &context)
                .map_err(|e| invalid(file, path, e.to_string()))?;
        }
        _ => {}
    }
    Ok(())
}
impl CampaignTables {
    pub(crate) fn load(directory: &Path) -> Result<Self, CampaignTablesError> {
        let tables = Self {
            chapter: Tables::load(directory).map_err(CampaignTablesError::Chapter)?,
            premises: load(directory, "campaign-premises.json")?,
            villains: load(directory, "villains.json")?,
            factions: load(directory, "factions.json")?,
            types: load(directory, "chapter-types.json")?,
            acts: load(directory, "act-structure.json")?,
            clocks: load(directory, "clocks.json")?,
            twists: load(directory, "twists.json")?,
            clues: load(directory, "clues.json")?,
            rewards: load(directory, "rewards.json")?,
        };
        tables.validate()?;
        Ok(tables)
    }
    fn validate(&self) -> Result<(), CampaignTablesError> {
        let require = |file: &str, path: &str, valid: bool| {
            if valid {
                Ok(())
            } else {
                Err(invalid(
                    file,
                    path,
                    "unresolved reference or incompatible contract",
                ))
            }
        };
        let types = &self.types.tables.chapter_types.entries;
        let families: BTreeSet<_> = self
            .premises
            .tables
            .premises
            .entries
            .iter()
            .map(|p| p.family.as_str())
            .collect();
        let tags: BTreeSet<_> = self
            .chapter
            .locations
            .entries
            .iter()
            .flat_map(|l| l.tags.iter().map(String::as_str))
            .collect();
        let stake_tags: BTreeSet<_> = self
            .chapter
            .stakes
            .entries
            .iter()
            .flat_map(|l| l.tags.iter().map(String::as_str))
            .collect();
        for (n, role) in self.premises.tables.site_roles.entries.iter().enumerate() {
            require(
                "campaign-premises.json",
                &format!("$.tables.site_roles.entries[{n}].prefer"),
                role.prefer.iter().all(|t| tags.contains(t.as_str())),
            )?;
        }
        for (n, premise) in self.premises.tables.premises.entries.iter().enumerate() {
            require(
                "campaign-premises.json",
                &format!("$.tables.premises.entries[{n}].region_tags"),
                !premise.region_tags.is_empty()
                    && premise
                        .region_tags
                        .iter()
                        .all(|t| tags.contains(t.as_str())),
            )?;
        }
        for (n, faction) in self.factions.tables.factions.entries.iter().enumerate() {
            require(
                "factions.json",
                &format!("$.tables.factions.entries[{n}].fits"),
                !faction.fits.is_empty() && faction.fits.iter().all(|t| tags.contains(t.as_str())),
            )?;
        }
        for (file, entries) in [
            (
                "villains.json",
                self.villains
                    .tables
                    .types
                    .entries
                    .iter()
                    .map(|v| &v.families)
                    .collect::<Vec<_>>(),
            ),
            (
                "villains.json",
                self.villains
                    .tables
                    .schemes
                    .entries
                    .iter()
                    .map(|v| &v.families)
                    .collect(),
            ),
            (
                "twists.json",
                self.twists
                    .tables
                    .twists
                    .entries
                    .iter()
                    .map(|v| &v.families)
                    .collect(),
            ),
        ] {
            for (n, refs) in entries.iter().enumerate() {
                require(
                    file,
                    &format!("$.tables.entries[{n}].families"),
                    !refs.is_empty() && refs.iter().all(|f| families.contains(f.as_str())),
                )?;
            }
        }
        for family in self.acts.family_hooks.keys() {
            require(
                "act-structure.json",
                &format!("$.family_hooks.{family}"),
                families.contains(family.as_str()),
            )?;
        }
        require(
            "villains.json",
            "$.name_tables",
            self.villains.name_tables
                == "names.json: villain_he, villain_she (given name), villain_adjective (title adjective). Title = adjective + type noun."
                && ["villain_he", "villain_she", "villain_adjective"]
                    .iter()
                    .all(|name| self.chapter.names.contains_key(*name)),
        )?;

        for (n, premise) in self.premises.tables.premises.entries.iter().enumerate() {
            require(
                "campaign-premises.json",
                &format!("$.tables.premises.entries[{n}].clock"),
                self.clocks.tables.clocks.contains_key(&premise.clock),
            )?;
            require(
                "campaign-premises.json",
                &format!("$.tables.premises.entries[{n}].villain_types"),
                !premise.villain_types.is_empty()
                    && premise.villain_types.iter().all(|id| {
                        self.villains
                            .tables
                            .types
                            .entries
                            .iter()
                            .any(|v| v.id == *id)
                    }),
            )?;
            require(
                "campaign-premises.json",
                &format!("$.tables.premises.entries[{n}].family"),
                self.acts.family_hooks.contains_key(&premise.family)
                    && self
                        .villains
                        .tables
                        .schemes
                        .entries
                        .iter()
                        .any(|s| s.families.contains(&premise.family))
                    && self
                        .twists
                        .tables
                        .twists
                        .entries
                        .iter()
                        .filter(|t| t.families.contains(&premise.family))
                        .count()
                        >= 2
                    && premise.titles.len() == 4,
            )?;
        }
        for (n, v) in self.villains.tables.types.entries.iter().enumerate() {
            require(
                "villains.json",
                &format!("$.tables.types.entries[{n}]"),
                self.villains.title_nouns.contains_key(&v.id)
                    && ["he", "she"].contains(&v.gender.as_str())
                    && self.chapter.art.contains_key(&v.art)
                    && v.lieutenant_pref
                        .iter()
                        .all(|id| self.chapter.antagonists.entries.iter().any(|a| a.id == *id)),
            )?;
        }
        for (n, l) in self.villains.tables.lieutenants.entries.iter().enumerate() {
            require(
                "villains.json",
                &format!("$.tables.lieutenants.entries[{n}].antagonist"),
                self.chapter
                    .antagonists
                    .entries
                    .iter()
                    .any(|a| a.id == l.antagonist),
            )?;
        }
        for (n, t) in types.iter().enumerate() {
            let path = format!("$.tables.chapter_types.entries[{n}]");
            require(
                "chapter-types.json",
                &path,
                ["native", "faked", "no"].contains(&t.v1.as_str())
                    && t.intended.iter().map(|p| u16::from(*p)).sum::<u16>() == 100
                    && t.needs
                        .iter()
                        .all(|need| self.types.capability_names.contains(need))
                    && t.on_victory
                        .iter()
                        .chain(&t.on_defeat)
                        .all(|id| types.iter().any(|t| t.id == *id)),
            )?;
            require(
                "chapter-types.json",
                &path,
                if t.v1 == "no" {
                    t.beat.is_none()
                } else {
                    t.beat.as_ref().is_some_and(|b| {
                        self.chapter
                            .hooks
                            .entries
                            .iter()
                            .any(|h| h.kind == b.hook_kind)
                            && !b.stake_tags.is_empty()
                    }) && t.asks.len() == 2
                        && t.helps.len() == 2
                        && t.stands.len() == 2
                },
            )?;

            if let Some(beat) = &t.beat {
                require(
                    "chapter-types.json",
                    &format!("{path}.beat.location_role"),
                    beat.location_role == "faction-seat"
                        || self
                            .premises
                            .tables
                            .site_roles
                            .entries
                            .iter()
                            .any(|role| role.id == beat.location_role),
                )?;
                require(
                    "chapter-types.json",
                    &format!("{path}.beat.stake_tags"),
                    !beat.stake_tags.is_empty()
                        && beat
                            .stake_tags
                            .iter()
                            .all(|tag| stake_tags.contains(tag.as_str())),
                )?;
                require(
                    "chapter-types.json",
                    &format!("{path}.beat.antagonist_role"),
                    ["minion", "guardian", "beast", "lieutenant", "champion"]
                        .contains(&beat.antagonist_role.as_str()),
                )?;
            }
            for (key, labels) in [
                ("asks", &t.asks),
                ("helps", &t.helps),
                ("stands", &t.stands),
            ] {
                require(
                    "chapter-types.json",
                    &format!("{path}.{key}"),
                    labels.iter().all(|text| text.chars().count() <= 40),
                )?;
            }
        }
        for count in 6..=10 {
            let slots =
                self.acts.slots.get(&count.to_string()).ok_or_else(|| {
                    invalid("act-structure.json", "$.slots", "missing chapter count")
                })?;
            require(
                "act-structure.json",
                &format!("$.slots.{count}"),
                slots.len() == count
                    && slots.last().is_some_and(|s| s.role == "finale")
                    && slots
                        .iter()
                        .filter(|s| s.allowed.contains(&"lieutenant".into()))
                        .count()
                        == if count >= 9 { 2 } else { 1 },
            )?;
            for (n, s) in slots.iter().enumerate() {
                require(
                    "act-structure.json",
                    &format!("$.slots.{count}[{n}]"),
                    self.acts.tables.acts.entries.iter().any(|a| a.id == s.act)
                        && !s.allowed.is_empty()
                        && s.allowed.iter().all(|id| types.iter().any(|t| t.id == *id)),
                )?;
            }
        }
        for (family, hooks) in &self.acts.family_hooks {
            require(
                "act-structure.json",
                &format!("$.family_hooks.{family}"),
                hooks
                    .iter()
                    .all(|id| self.chapter.hooks.entries.iter().any(|h| h.id == *id)),
            )?;
        }
        for (n, node) in self.acts.tables.nodes.entries.iter().enumerate() {
            require(
                "act-structure.json",
                &format!("$.tables.nodes.entries[{n}]"),
                node.count == 3
                    && self.clues.tables.clues.contains_key(&node.id)
                    && self
                        .acts
                        .slots
                        .values()
                        .all(|s| s.iter().any(|s| s.role == node.needed_by)),
            )?;
        }
        for (id, clock) in &self.clocks.tables.clocks {
            require(
                "clocks.json",
                &format!("$.tables.clocks.{id}"),
                clock.die.is_none()
                    && clock.steps.len() == 7
                    && clock
                        .steps
                        .iter()
                        .enumerate()
                        .all(|(n, s)| usize::from(s.step) == n + 1),
            )?;
        }
        Ok(())
    }
}
