//! Exact immutable bytes are authority on resume; changed tables are never consulted.
use super::super::pack::{self, CastRole, LoadedPack, SpeakerProfile};
use super::{GenerateError, GeneratedAdventure, Roll, RollStatus, RulesMode};
use df_persistence::local_demo_scope::DemoAdventure;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fmt,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};
const GENERATOR_VERSION: &str = "splitmix64-patterns-1";
const RECORD_SCHEMA: &str = "df-adventure-record/1";
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdventureRecord {
    schema: String,
    pub(crate) seed: String,
    tables_revision: String,
    generator_version: String,
    rules_mode: RulesMode,
    pub(crate) rolls: Vec<Roll>,
    expression: Expression,
    pack_sha256: String,
    tuning_sha256: String,
    pub(crate) seeded_dice: bool,
    /// Copies of the pack's listener-safety data, so a review of the record shows them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    secret_terms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    speakers: Vec<SpeakerProfile>,
    /// Who is on stage in each scene, copied from the pack.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scene_cast: Vec<SceneCastRecord>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SceneCastRecord {
    scene: String,
    id: String,
    name: String,
    role: CastRole,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Expression {
    /// `patterns` or `model`; `model` carries the details below.
    source: String,
    fact_sheet_sha256: String,
    output_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<ModelExpression>,
}
/// What the language model wrote: which slots took its text and which kept the pattern text.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelExpression {
    pub(crate) model: String,
    pub(crate) model_slots: Vec<String>,
    pub(crate) pattern_slots: Vec<String>,
    /// Why a pattern slot kept its pattern text, as `slot: reason`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) refused: Vec<String>,
    /// Supplier-reported tokens of the calls made.
    pub(crate) tokens: u64,
    /// Wall-clock milliseconds the whole expression step took.
    #[serde(default)]
    pub(crate) elapsed_ms: u64,
}
impl ModelExpression {
    pub(crate) fn summary(&self) -> String {
        format!(
            "prose=model slots={}/{} tokens={}",
            self.model_slots.len(),
            self.model_slots.len() + self.pattern_slots.len(),
            self.tokens
        )
    }
}
#[derive(Debug)]
pub(crate) enum RecordError {
    Pack(pack::PackError),
    Invalid(&'static str),
    Io {
        file: &'static str,
        kind: io::ErrorKind,
    },
}
impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pack(e) => e.fmt(f),
            Self::Invalid(reason) => write!(f, "adventure record refused: {reason}"),
            Self::Io { file, kind } => write!(f, "adventure cache {file}: {kind}"),
        }
    }
}
impl std::error::Error for RecordError {}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl GeneratedAdventure {
    /// The documents of an adventure whose text is entirely the pattern text.
    #[cfg(test)]
    pub(crate) fn documents(&self, seeded_dice: bool) -> Result<DemoAdventure, GenerateError> {
        self.documents_expressed(seeded_dice, None)
    }
    /// The documents, recording how the language model took part when it did.
    pub(crate) fn documents_expressed(
        &self,
        seeded_dice: bool,
        model: Option<ModelExpression>,
    ) -> Result<DemoAdventure, GenerateError> {
        let pack =
            serde_json::to_vec_pretty(&self.pack).map_err(|_| GenerateError::Serialization)?;
        let tuning =
            serde_json::to_vec_pretty(&self.tuning).map_err(|_| GenerateError::Serialization)?;
        let mut output = Sha256::new();
        output.update(&pack);
        output.update(&tuning);
        let record = AdventureRecord {
            schema: RECORD_SCHEMA.into(),
            seed: format!("0x{:016x}", self.seed),
            tables_revision: self.tables_revision.clone(),
            generator_version: GENERATOR_VERSION.into(),
            rules_mode: self.rules_mode,
            rolls: self.rolls.clone(),
            expression: Expression {
                source: if model.is_some() { "model" } else { "patterns" }.into(),
                model,
                fact_sheet_sha256: hash(
                    &serde_json::to_vec(&self.fact_sheet)
                        .map_err(|_| GenerateError::Serialization)?,
                ),
                output_sha256: format!("{:x}", output.finalize()),
            },
            pack_sha256: hash(&pack),
            tuning_sha256: hash(&tuning),
            seeded_dice,
            secret_terms: self.pack.secret_terms.clone(),
            speakers: self.pack.speakers.clone(),
            scene_cast: self
                .pack
                .scenes
                .iter()
                .flat_map(|scene| {
                    let key = serde_json::to_value(scene.key)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned))
                        .unwrap_or_default();
                    scene.cast.iter().map(move |actor| SceneCastRecord {
                        scene: key.clone(),
                        id: actor.id.clone(),
                        name: actor.name.clone(),
                        role: actor.role,
                    })
                })
                .collect(),
        };
        let record =
            serde_json::to_vec_pretty(&record).map_err(|_| GenerateError::Serialization)?;
        Ok(DemoAdventure {
            pack,
            tuning,
            record,
        })
    }
}
/// Validate exact stored content/record without reading any table or drawing entropy.
pub(crate) fn reload(
    documents: &DemoAdventure,
) -> Result<(LoadedPack, AdventureRecord), RecordError> {
    if [&documents.pack, &documents.tuning, &documents.record]
        .iter()
        .any(|bytes| bytes.is_empty() || bytes.len() > 256 * 1024)
    {
        return Err(RecordError::Invalid("document size"));
    }
    let loaded =
        pack::load_from_bytes(&documents.pack, &documents.tuning).map_err(RecordError::Pack)?;
    let record: AdventureRecord = serde_json::from_slice(&documents.record)
        .map_err(|_| RecordError::Invalid("adventure.json schema"))?;
    let seed = super::super::startup_configuration::adventure_seed(&record.seed)
        .map_err(|_| RecordError::Invalid("seed"))?;
    let valid_hash =
        |value: &str| value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit());
    if record.schema != RECORD_SCHEMA
        || record.generator_version != GENERATOR_VERSION
        || !matches!(
            (record.expression.source.as_str(), &record.expression.model),
            ("patterns", None) | ("model", Some(_))
        )
        || !valid_hash(&record.tables_revision)
        || !valid_hash(&record.expression.fact_sheet_sha256)
        || record.pack_sha256 != hash(&documents.pack)
        || record.tuning_sha256 != hash(&documents.tuning)
        || loaded.pack.id != format!("adv-{seed:016x}")
    {
        return Err(RecordError::Invalid(
            "schema, version, seed or byte revisions",
        ));
    }
    let mut output = Sha256::new();
    output.update(&documents.pack);
    output.update(&documents.tuning);
    if record.expression.output_sha256 != format!("{:x}", output.finalize()) {
        return Err(RecordError::Invalid("expression output hash"));
    }
    let mut story = BTreeSet::new();
    for (index, roll) in record.rolls.iter().enumerate() {
        if roll.n != index + 1
            || ![4, 6, 8, 10, 12, 20].contains(&roll.die)
            || roll.entry_id.is_empty()
            || match roll.status {
                RollStatus::Ok | RollStatus::Rejected => {
                    roll.natural == 0 || roll.natural > roll.die
                }
                RollStatus::Fallback | RollStatus::Forced => roll.natural != 0,
            }
        {
            return Err(RecordError::Invalid("DM roll ordering or face"));
        }
        if matches!(
            roll.status,
            RollStatus::Ok | RollStatus::Fallback | RollStatus::Forced
        ) {
            story.insert(roll.table.as_str());
        }
    }
    for table in [
        "location",
        "hook",
        "patron",
        "stake",
        "antagonist",
        "complication",
        "secret",
        "destination",
        "contact",
        "reward",
    ] {
        if !story.contains(table) {
            return Err(RecordError::Invalid("missing story roll"));
        }
    }
    let antagonist = loaded
        .pack
        .bandit
        .srd_id
        .as_deref()
        .ok_or(RecordError::Invalid("missing antagonist identity"))?;
    if record.entry("antagonist")? != antagonist
        || record.rules_mode == RulesMode::Standard && !loaded.departures.is_empty()
    {
        return Err(RecordError::Invalid("antagonist or standard rules binding"));
    }
    Ok((loaded, record))
}
impl AdventureRecord {
    pub(crate) fn entry(&self, table: &str) -> Result<&str, RecordError> {
        self.rolls
            .iter()
            .find(|r| {
                r.table == table
                    && matches!(
                        r.status,
                        RollStatus::Ok | RollStatus::Fallback | RollStatus::Forced
                    )
            })
            .map(|r| r.entry_id.as_str())
            .ok_or(RecordError::Invalid("missing selected story entry"))
    }
    /// The startup summary of how the story text was written.
    pub(crate) fn prose_summary(&self) -> String {
        match &self.expression.model {
            Some(model) => model.summary(),
            None => "prose=patterns (stored adventure uses the pattern text)".to_owned(),
        }
    }
    pub(crate) fn tables_revision(&self) -> &str {
        &self.tables_revision
    }
    pub(crate) fn rules_mode(&self) -> RulesMode {
        self.rules_mode
    }
}
fn documents(data: &DemoAdventure) -> [(&'static str, &[u8]); 3] {
    [
        ("pack.json", &data.pack),
        ("tuning.json", &data.tuning),
        ("adventure.json", &data.record),
    ]
}
/// Creates the run cache once before bootstrap. Existing staging is never overwritten.
pub(crate) fn create_cache(directory: &Path, data: &DemoAdventure) -> Result<(), RecordError> {
    let parent = directory
        .parent()
        .ok_or(RecordError::Invalid("run directory"))?;
    fs::create_dir_all(parent).map_err(|e| RecordError::Io {
        file: "run directory",
        kind: e.kind(),
    })?;
    fs::create_dir(directory).map_err(|e| RecordError::Io {
        file: "run directory (existing staging refuses regeneration)",
        kind: e.kind(),
    })?;
    for (name, bytes) in documents(data) {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
            .map_err(|e| RecordError::Io {
                file: name,
                kind: e.kind(),
            })?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| RecordError::Io {
                file: name,
                kind: e.kind(),
            })?;
    }
    Ok(())
}
/// A cache never overrides PostgreSQL. Missing or edited local bytes refuse resume explicitly.
pub(crate) fn verify_cache(directory: &Path, data: &DemoAdventure) -> Result<(), RecordError> {
    use std::io::Read;
    for (name, expected) in documents(data) {
        let mut actual = Vec::new();
        fs::File::open(directory.join(name))
            .and_then(|file| file.take(256 * 1024 + 1).read_to_end(&mut actual))
            .map_err(|e| RecordError::Io {
                file: name,
                kind: e.kind(),
            })?;
        if actual != expected {
            return Err(RecordError::Invalid(
                "cache differs from authoritative stored bytes",
            ));
        }
    }
    Ok(())
}
