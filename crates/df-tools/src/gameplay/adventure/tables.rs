use super::{GenerateError, schema::*};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::Path,
};

#[derive(Debug)]
pub(crate) enum TablesError {
    Io {
        file: String,
        kind: std::io::ErrorKind,
    },
    Invalid {
        file: String,
        path: String,
        reason: String,
    },
}
impl fmt::Display for TablesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { file, kind } => write!(f, "adventure table {file}: {kind}"),
            Self::Invalid { file, path, reason } => {
                write!(f, "adventure table {file} at {path}: {reason}")
            }
        }
    }
}
impl std::error::Error for TablesError {}
fn invalid(file: &str, path: &str, reason: impl Into<String>) -> TablesError {
    TablesError::Invalid {
        file: file.into(),
        path: path.into(),
        reason: reason.into(),
    }
}
fn read(directory: &Path, file: &str) -> Result<Vec<u8>, TablesError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(directory.join(file))
        .and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| TablesError::Io {
            file: file.into(),
            kind: e.kind(),
        })?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid(file, ".", "file exceeds one MiB"));
    }
    Ok(bytes)
}
fn parse<T: DeserializeOwned>(file: &str, bytes: &[u8]) -> Result<T, TablesError> {
    let mut parser = serde_json::Deserializer::from_slice(bytes);
    let result = serde_path_to_error::deserialize(&mut parser)
        .map_err(|e| invalid(file, &e.path().to_string(), e.inner().to_string()))?;
    parser
        .end()
        .map_err(|e| invalid(file, ".", e.to_string()))?;
    Ok(result)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Art {
    pub(crate) art_key: String,
    pub(crate) kind: String,
    pub(crate) file_stem: String,
    pub(crate) aspect: String,
    pub(crate) used_by: String,
    pub(crate) prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) depicts: Vec<super::setting::SettingKind>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: String,
    file: String,
    schema_version: u8,
    tables_version: String,
    description: String,
    files: Vec<String>,
    rng: RngSpec,
    selection: SelectionSpec,
    rules_modes: BTreeMap<String, String>,
    roll_order: Vec<RollSpec>,
    limits: BTreeMap<String, usize>,
    campaign: CampaignSpec,
    counts: BTreeMap<String, usize>,
    compat_tags: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RngSpec {
    algorithm: String,
    seed_bits: u8,
    init: String,
    next: String,
    roll: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionSpec {
    faces: String,
    incompatible: String,
    max_attempts: u8,
    fallback: String,
    determinism: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CampaignSpec {
    stream: String,
    frame_roll_order: Vec<String>,
    chapter_seed: String,
    chapter_generation: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RollSpec {
    n: u8,
    table: String,
    file: String,
    die: DieSpec,
    filter: Option<String>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum DieSpec {
    Number(u8),
    Description(String),
}
/// Validated chapter tables; campaign files are included in the revision but never interpreted.
pub(crate) struct Tables {
    pub(super) revision: String,
    pub(super) locations: Table<Location>,
    pub(super) hooks: Table<Hook>,
    pub(super) patrons: Table<NpcArchetype>,
    pub(super) stakes: Table<Stake>,
    pub(super) antagonists: Table<Antagonist>,
    pub(super) complications: Table<Complication>,
    pub(super) secrets: Table<Secret>,
    pub(super) destinations: BTreeMap<String, Table<Destination>>,
    pub(super) contacts: Table<NpcArchetype>,
    pub(super) rewards: Table<TextEntry>,
    pub(super) names: BTreeMap<String, Table<TextEntry>>,
    pub(super) traits: BTreeMap<String, Table<TextEntry>>,
    pub(super) prose: BTreeMap<String, PatternTable>,
    pub(super) art: BTreeMap<String, String>,
    pub(super) depicts: BTreeMap<String, Vec<super::setting::SettingKind>>,
    pub(super) places: super::setting::Places,
}
fn document<T: DeserializeOwned + Serialize>(
    bytes: &BTreeMap<String, Vec<u8>>,
    file: &str,
) -> Result<Document<T>, TablesError> {
    let doc: Document<T> = parse(
        file,
        bytes
            .get(file)
            .ok_or_else(|| invalid(file, ".", "not indexed"))?,
    )?;
    if doc.schema != "df-adventure-tables/1" || doc.file != file.trim_end_matches(".json") {
        return Err(invalid(file, "schema", "unsupported schema or file marker"));
    }
    let tree =
        serde_json::to_value(&doc.tables).map_err(|_| invalid(file, ".", "serialization"))?;
    lint_tree(file, "tables", &tree)?;
    Ok(doc)
}
fn take<T>(mut tables: BTreeMap<String, T>, file: &str, key: &str) -> Result<T, TablesError> {
    tables
        .remove(key)
        .ok_or_else(|| invalid(file, "tables", format!("missing {key}")))
}
impl Tables {
    pub(crate) fn load(directory: &Path) -> Result<Self, TablesError> {
        let index: Index = parse("tables.json", &read(directory, "tables.json")?)?;
        if index.schema != "df-adventure-tables/1"
            || index.schema_version != 1
            || index.rng.algorithm != "splitmix64"
            || index.rng.seed_bits != 64
            || index.selection.max_attempts != 64
        {
            return Err(invalid("tables.json", ".", "unsupported table protocol"));
        }
        // These descriptive fields are admitted as typed metadata, never executable instructions.
        let _metadata = (
            &index.file,
            &index.tables_version,
            &index.description,
            &index.rng.init,
            &index.rng.next,
            &index.rng.roll,
            &index.selection.faces,
            &index.selection.incompatible,
            &index.selection.fallback,
            &index.selection.determinism,
            &index.rules_modes,
            &index.limits,
            &index.campaign.stream,
            &index.campaign.frame_roll_order,
            &index.campaign.chapter_seed,
            &index.campaign.chapter_generation,
            &index.counts,
        );
        for spec in &index.roll_order {
            let _ = (&spec.n, &spec.table, &spec.file, &spec.filter);
            match &spec.die {
                DieSpec::Number(n) if *n == 0 => {
                    return Err(invalid("tables.json", "roll_order.die", "zero die"));
                }
                DieSpec::Description(s) if s.is_empty() => {
                    return Err(invalid("tables.json", "roll_order.die", "empty die"));
                }
                _ => {}
            }
        }
        let mut digest = Sha256::new();
        let mut files = BTreeSet::new();
        let mut bytes = BTreeMap::new();
        for file in &index.files {
            if !file.ends_with(".json")
                || file.contains('/')
                || file.contains('\\')
                || !files.insert(file)
            {
                return Err(invalid("tables.json", "files", "invalid or duplicate file"));
            }
            let content = read(directory, file)?;
            digest.update(&content);
            bytes.insert(file.clone(), content);
        }
        for file in [
            "locations.json",
            "hooks.json",
            "patrons.json",
            "macguffins.json",
            "antagonists.json",
            "complications.json",
            "secrets.json",
            "destinations.json",
            "contacts.json",
            "rewards.json",
            "names.json",
            "npc-traits.json",
            "prose-patterns.json",
            "art-manifest.json",
            "places.json",
        ] {
            if !files.contains(&file.to_owned()) {
                return Err(invalid("tables.json", "files", format!("missing {file}")));
            }
        }
        let art_entries: Vec<Art> = parse(
            "art-manifest.json",
            bytes
                .get("art-manifest.json")
                .ok_or_else(|| invalid("tables.json", "files", "missing art manifest"))?,
        )?;
        let mut art = BTreeMap::new();
        let mut depicts = BTreeMap::new();
        let mut stems = BTreeSet::new();
        for entry in art_entries {
            if entry.kind == "scene" && entry.depicts.is_empty() {
                return Err(invalid(
                    "art-manifest.json",
                    "depicts",
                    "scene depiction required",
                ));
            }
            depicts.insert(entry.art_key.clone(), entry.depicts.clone());
            if !entry
                .file_stem
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                || !stems.insert(entry.file_stem.clone())
                || art
                    .insert(
                        entry.art_key,
                        format!("assets/ui/generated/{}", entry.file_stem),
                    )
                    .is_some()
            {
                return Err(invalid(
                    "art-manifest.json",
                    "file_stem",
                    "invalid or duplicate art identity",
                ));
            }
        }
        let result = Self {
            revision: format!("{:x}", digest.finalize()),
            locations: take(
                document(&bytes, "locations.json")?.tables,
                "locations.json",
                "locations",
            )?,
            hooks: take(
                document(&bytes, "hooks.json")?.tables,
                "hooks.json",
                "hooks",
            )?,
            patrons: take(
                document(&bytes, "patrons.json")?.tables,
                "patrons.json",
                "patrons",
            )?,
            stakes: take(
                document(&bytes, "macguffins.json")?.tables,
                "macguffins.json",
                "stakes",
            )?,
            antagonists: take(
                document(&bytes, "antagonists.json")?.tables,
                "antagonists.json",
                "antagonists",
            )?,
            complications: take(
                document(&bytes, "complications.json")?.tables,
                "complications.json",
                "complications",
            )?,
            secrets: take(
                document(&bytes, "secrets.json")?.tables,
                "secrets.json",
                "secrets",
            )?,
            destinations: document(&bytes, "destinations.json")?.tables,
            contacts: take(
                document(&bytes, "contacts.json")?.tables,
                "contacts.json",
                "contacts",
            )?,
            rewards: take(
                document(&bytes, "rewards.json")?.tables,
                "rewards.json",
                "chapter_lines",
            )?,
            names: document(&bytes, "names.json")?.tables,
            traits: document(&bytes, "npc-traits.json")?.tables,
            prose: document(&bytes, "prose-patterns.json")?.tables,
            art,
            depicts,
            places: parse(
                "places.json",
                bytes
                    .get("places.json")
                    .ok_or_else(|| invalid("places.json", ".", "not indexed"))?,
            )?,
        };
        lint_tree(
            "places.json",
            ".",
            &serde_json::to_value(&result.places)
                .map_err(|_| invalid("places.json", ".", "serialization"))?,
        )?;
        result.places.validate(&result)?;
        result.compatibility(&index.compat_tags)?;
        super::bounds::validate(&result)?;
        Ok(result)
    }
    fn compatibility(&self, tags: &[String]) -> Result<(), TablesError> {
        // Rules/outcome language is compiled authority, never editable expression data.
        for (key, expected) in [
            (
                "combat_mech",
                "Both heroes are within 5 feet of {ant.the}, and everyone fights in the open: no surprise, cover or movement. This encounter uses nonlethal melee attacks.",
            ),
            (
                "victory_mech",
                "{ant.short|cap} falls unconscious with 1 HP. {story} {ant.pos|cap} Short Rest has begun; campaign time has not completed it.",
            ),
            (
                "victory_rested",
                "{ant.short|cap} was defeated. An uninterrupted hour has passed without Hit Point Dice or healing. Knocked-out creatures are awake with 1 HP, still Prone; their held weapons remain dropped. The original victory and {patron.first}'s unresolved journey remain.",
            ),
            (
                "defeat_mech",
                "Both heroes are unconscious with 1 HP after nonlethal attacks. {story} Their Short Rests have begun; campaign time has not completed them.",
            ),
            (
                "defeat_rested",
                "Both heroes were defeated by nonlethal attacks. An uninterrupted hour has passed without Hit Point Dice or healing. They awaken with 1 HP, still Prone, and their held weapons remain dropped. That rest does not change the original defeat.",
            ),
        ] {
            let table = self.prose.get(key).ok_or_else(|| {
                invalid(
                    "prose-patterns.json",
                    &format!("tables.{key}"),
                    "required mechanical template missing",
                )
            })?;
            if table.die.is_some()
                || table.variants.len() != 1
                || table.variants.first().is_none_or(|v| v.text != expected)
            {
                return Err(invalid(
                    "prose-patterns.json",
                    &format!("tables.{key}"),
                    "mechanical template differs from compiled rules authority",
                ));
            }
        }
        for key in [
            "begin_label",
            "room_title",
            "room_narration",
            "opening_narration",
            "foreshadow_speaker",
            "foreshadow_beast",
            "foreshadow_undead",
            "ask_label",
            "help_label",
            "stand_label",
            "continue_label",
            "ask_title",
            "ask_narration",
            "help_title",
            "help_narration",
            "route_deliver",
            "route_escort",
            "route_warn",
            "route_retrieve",
            "clue",
            "combat_title",
            "combat_open_speaker",
            "combat_open_beast",
            "combat_open_undead",
            "combat_mech",
            "victory_title",
            "victory_lead",
            "victory_mech",
            "victory_rested",
            "victory_dialogue",
            "defeat_title",
            "defeat_deliver",
            "defeat_escort",
            "defeat_warn",
            "defeat_retrieve",
            "defeat_mech",
            "defeat_rested",
            "inn_title",
            "inn_intro",
            "inn_dialogue",
            "escort_support_suffix",
        ] {
            if !self.prose.contains_key(key) {
                return Err(invalid(
                    "prose-patterns.json",
                    &format!("tables.{key}"),
                    "required pattern table missing",
                ));
            }
        }
        for key in ["he", "she", "surnames"] {
            let distinct = self.names.get(key).map_or(0, |table| {
                table
                    .entries
                    .iter()
                    .map(|e| &e.text)
                    .collect::<BTreeSet<_>>()
                    .len()
            });
            if distinct < if key == "surnames" { 2 } else { 3 } {
                return Err(invalid(
                    "names.json",
                    &format!("tables.{key}.entries"),
                    "insufficient distinct names for exclusions",
                ));
            }
        }
        for location in &self.locations.entries {
            if location
                .atmosphere
                .entries
                .iter()
                .map(|e| &e.text)
                .collect::<BTreeSet<_>>()
                .len()
                < 3
            {
                return Err(invalid(
                    "locations.json",
                    &format!("tables.locations.{}.atmosphere", location.id),
                    "three distinct atmosphere entries required",
                ));
            }
            if location.tags.iter().any(|tag| !tags.contains(tag)) {
                return Err(invalid(
                    "locations.json",
                    "tags",
                    "unknown compatibility tag",
                ));
            }
            if !self.destinations.contains_key(&location.id) {
                return Err(invalid(
                    "destinations.json",
                    "tables",
                    "location missing destination table",
                ));
            }
            for (file, choices) in [
                (
                    "hooks.json",
                    self.hooks
                        .entries
                        .iter()
                        .map(|e| &e.fits)
                        .collect::<Vec<_>>(),
                ),
                (
                    "patrons.json",
                    self.patrons.entries.iter().map(|e| &e.fits).collect(),
                ),
                (
                    "contacts.json",
                    self.contacts.entries.iter().map(|e| &e.fits).collect(),
                ),
                (
                    "antagonists.json",
                    self.antagonists
                        .entries
                        .iter()
                        .filter(|e| e.fidelity == "exact")
                        .map(|e| &e.fits)
                        .collect(),
                ),
            ] {
                if choices
                    .iter()
                    .any(|fits| fits.iter().any(|tag| !tags.contains(tag)))
                    || !choices.iter().any(|fits| intersects(fits, &location.tags))
                {
                    return Err(invalid(
                        file,
                        "tables.entries.fits",
                        "unknown tag or uncovered location",
                    ));
                }
            }
            for key in [&location.art.arrival, &location.art.interior] {
                if !self.art.contains_key(key) {
                    return Err(invalid("locations.json", "art", "unknown art key"));
                }
            }
        }
        let kinds = ["deliver", "escort", "warn", "retrieve"];
        let stake_tags = ["carry", "message", "heirloom", "remedy"];
        for stake in &self.stakes.entries {
            if stake.tags.is_empty() || stake.tags.iter().any(|t| !stake_tags.contains(&t.as_str()))
            {
                return Err(invalid(
                    "macguffins.json",
                    "tables.stakes.entries.tags",
                    "unknown or empty stake tags",
                ));
            }
        }
        for secret in &self.secrets.entries {
            if secret.kinds.is_empty() || secret.kinds.iter().any(|k| !kinds.contains(&k.as_str()))
            {
                return Err(invalid(
                    "secrets.json",
                    "tables.secrets.entries.kinds",
                    "unknown or empty hook kinds",
                ));
            }
        }
        for complication in &self.complications.entries {
            if complication.groups.is_empty()
                || complication
                    .groups
                    .iter()
                    .any(|g| !["speaker", "beast", "undead"].contains(&g.as_str()))
            {
                return Err(invalid(
                    "complications.json",
                    "tables.complications.entries.for",
                    "unknown or empty antagonist groups",
                ));
            }
        }
        for hook in &self.hooks.entries {
            if hook.stake_tags.is_empty()
                || hook
                    .stake_tags
                    .iter()
                    .any(|t| !stake_tags.contains(&t.as_str()))
            {
                return Err(invalid(
                    "hooks.json",
                    "tables.hooks.entries.stake_tags",
                    "unknown or empty stake tags",
                ));
            }
            if !["deliver", "escort", "warn", "retrieve"].contains(&hook.kind.as_str())
                || !self
                    .stakes
                    .entries
                    .iter()
                    .any(|s| intersects(&s.tags, &hook.stake_tags))
                || !self
                    .secrets
                    .entries
                    .iter()
                    .any(|s| s.kinds.contains(&hook.kind))
            {
                return Err(invalid(
                    "hooks.json",
                    "tables.entries",
                    "uncovered hook kind/stake/secret",
                ));
            }
        }
        for ant in &self.antagonists.entries {
            let row = df_rules::SRD_521_CREATURES
                .iter()
                .find(|c| c.id == ant.id)
                .ok_or_else(|| {
                    invalid(
                        "antagonists.json",
                        &format!("tables.antagonists.{}.id", ant.id),
                        "unknown compiled SRD creature",
                    )
                })?;
            let a = ant.stats.abilities;
            if ant.stats.hit_points != row.rules.hit_points
                || ant.stats.armor_class != row.rules.armor_class
                || [
                    a.strength,
                    a.dexterity,
                    a.constitution,
                    a.intelligence,
                    a.wisdom,
                    a.charisma,
                ] != row.rules.abilities
                || i32::from(ant.stats.attack_bonus) != row.rules.attack_bonus
                || i32::from(ant.stats.damage_modifier) != row.rules.damage_modifier
                || ant.stats.weapon.label != row.weapon_label
                || ant.stats.weapon.damage_dice != row.rules.weapon.damage_dice
                || ant.stats.weapon.damage_sides != row.rules.weapon.damage_sides
                || (ant.fidelity == "exact") != row.exact
                || ant.srd.page != row.page
            {
                return Err(invalid(
                    "antagonists.json",
                    &format!("tables.antagonists.{}.stats", ant.id),
                    "numbers, fidelity or citation differ from compiled SRD authority",
                ));
            }
            if ant.fits.iter().any(|t| !tags.contains(t))
                || !["person", "goblinoid", "beast", "undead"].contains(&ant.klass.as_str())
                || !["person", "goblinoid", "creature", "undead"].contains(&ant.name_form.as_str())
                || !["he", "she", "it"].contains(&ant.gender.as_str())
                || ant.speaks != ant.warnings.is_some()
            {
                return Err(invalid(
                    "antagonists.json",
                    "tables.antagonists.entries",
                    "unknown tag, class, name form, gender or inconsistent speech",
                ));
            }
            if !["exact", "simplified"].contains(&ant.fidelity.as_str())
                || !self
                    .complications
                    .entries
                    .iter()
                    .any(|c| c.groups.iter().any(|g| g == group(ant)))
            {
                return Err(invalid(
                    "antagonists.json",
                    "tables.entries",
                    "uncovered antagonist group or fidelity",
                ));
            }
            if !self.art.contains_key(&ant.art) {
                return Err(invalid("antagonists.json", "art", "unknown art key"));
            }
        }
        for npc in self.patrons.entries.iter().chain(&self.contacts.entries) {
            if !self.names.contains_key(&npc.gender) || !self.art.contains_key(&npc.art) {
                return Err(invalid(
                    "patrons.json",
                    "gender/art",
                    "unknown gender or art",
                ));
            }
        }
        for key in ["surnames", "goblin", "kobold", "creature_adj", "undead_adj"] {
            if !self.names.contains_key(key) {
                return Err(invalid("names.json", "tables", format!("missing {key}")));
            }
        }
        for key in ["mannerisms", "speech", "menace"] {
            if !self.traits.contains_key(key) {
                return Err(invalid(
                    "npc-traits.json",
                    "tables",
                    format!("missing {key}"),
                ));
            }
        }
        Ok(())
    }
    pub(super) fn asset(&self, key: &str) -> Result<String, GenerateError> {
        self.art
            .get(key)
            .cloned()
            .ok_or_else(|| GenerateError::MissingTable(format!("art.{key}")))
    }
}
pub(super) fn intersects(a: &[String], b: &[String]) -> bool {
    a.iter().any(|v| b.contains(v))
}
pub(super) fn group(ant: &Antagonist) -> &str {
    if ant.speaks {
        "speaker"
    } else if ant.klass == "undead" {
        "undead"
    } else {
        "beast"
    }
}
fn lint_tree(file: &str, path: &str, value: &serde_json::Value) -> Result<(), TablesError> {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            if let Some(entries) = map
                .get("entries")
                .or_else(|| map.get("variants"))
                .and_then(Value::as_array)
            {
                let die = map.get("die").and_then(Value::as_u64).unwrap_or(1);
                if ![1, 4, 6, 8, 10, 12, 20].contains(&die) || entries.is_empty() {
                    return Err(invalid(file, path, "invalid die or empty entries"));
                }
                let n = entries.len() as u64;
                let expected_die = if n == 1 {
                    1
                } else {
                    *[4u64, 6, 8, 10, 12, 20]
                        .iter()
                        .find(|d| **d >= n)
                        .ok_or_else(|| invalid(file, path, "too many entries"))?
                };
                if die != expected_die {
                    return Err(invalid(
                        file,
                        path,
                        "entry count does not match smallest named die",
                    ));
                }
                let mut next = 1;
                let mut ids = BTreeSet::new();
                for (ordinal, entry) in entries.iter().enumerate() {
                    let faces = entry
                        .get("faces")
                        .and_then(Value::as_array)
                        .ok_or_else(|| invalid(file, path, "missing faces"))?;
                    let first = faces.first().and_then(Value::as_u64);
                    let last = faces.get(1).and_then(Value::as_u64);
                    if first != Some(next) || last.is_none_or(|n| n < next || n > die) {
                        return Err(invalid(file, path, "faces do not partition die"));
                    }
                    if last.unwrap_or(0) - next + 1
                        != die / n + u64::from((ordinal as u64) < die % n)
                    {
                        return Err(invalid(
                            file,
                            path,
                            "face spread differs from table protocol",
                        ));
                    }
                    next = last.unwrap_or(0) + 1;
                    if let Some(id) = entry.get("id").and_then(Value::as_str)
                        && !ids.insert(id)
                    {
                        return Err(invalid(file, path, "duplicate entry id"));
                    }
                }
                if next != die + 1 {
                    return Err(invalid(file, path, "faces do not cover die"));
                }
            }
            for (key, value) in map {
                if file == "rewards.json" && key == "milestones" {
                    continue;
                }
                lint_tree(file, &format!("{path}.{key}"), value)?;
            }
        }
        Value::Array(entries) => {
            for (i, value) in entries.iter().enumerate() {
                lint_tree(file, &format!("{path}[{i}]"), value)?;
            }
        }
        Value::String(text) => {
            let known = [
                "place",
                "place_short",
                "place_detail",
                "loc.name",
                "loc.title",
                "loc.short",
                "loc.atmos_a",
                "loc.atmos_b",
                "loc.atmos_c",
                "patron.first",
                "patron.full",
                "patron.role",
                "patron.desc",
                "patron.sub",
                "patron.obj",
                "patron.pos",
                "patron.self",
                "patron.mannerism",
                "contact.first",
                "contact.full",
                "contact.role",
                "contact.desc",
                "contact.sub",
                "contact.obj",
                "contact.pos",
                "contact.self",
                "contact.mannerism",
                "ant.short",
                "ant.full",
                "ant.the",
                "ant.sub",
                "ant.obj",
                "ant.pos",
                "ant.self",
                "stake.short",
                "stake.full",
                "stake.title",
                "dest.name",
                "dest.short",
                "dest.title",
                "secret.fact",
                "secret.confirm",
                "reward.line",
                "entrance",
                "foreshadow",
                "arrival",
                "route",
                "story",
                "name",
                "pos",
            ];
            let slots = known
                .into_iter()
                .map(|k| (k.into(), "value".into()))
                .collect();
            super::render::render(text, &slots).map_err(|e| invalid(file, path, e.to_string()))?;
        }
        _ => {}
    }
    Ok(())
}
