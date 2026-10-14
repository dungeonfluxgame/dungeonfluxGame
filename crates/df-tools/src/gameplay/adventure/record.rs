//! Exact immutable bytes are authority on resume; changed tables are never consulted.
use super::super::pack::{self, CastRole, LoadedPack, SpeakerProfile};
use super::{GenerateError, GeneratedAdventure, Roll, RollStatus, RulesMode, dossier::Dossier};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    campaign: Option<pack::CampaignIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) settings: Option<super::setting::ChapterSettings>,
    /// Copies of the pack's listener-safety data, so a review of the record shows them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    secret_terms: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    speakers: Vec<SpeakerProfile>,
    /// Who is on stage in each scene, copied from the pack.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scene_cast: Vec<SceneCastRecord>,
    /// Backstories of the speaking cast (section 19). Absent in adventures saved before they
    /// existed, which keep playing without them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) dossiers: Vec<Dossier>,
    /// The bounded structured expression facts, retained so the notebook can cite only
    /// entities the generated scene actually names. Older adventures continue without them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) fact_sheet: Vec<super::FactSheet>,
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
    #[serde(default)]
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
            campaign: self.pack.campaign.clone(),
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
            settings: Some(self.settings.clone()),
            dossiers: self.dossiers.clone(),
            // Only what the notebook reads at run time (the scene, who speaks, the named
            // facts and what the text must mention). The voice, delivery, tone, forbidden terms
            // and baseline text are generation inputs: stored whole they took the record past
            // the storage limit for one document and the server would not start.
            fact_sheet: notebook_fact_sheets(&self.fact_sheet),
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
/// The facts of a scene's sheet the party notebook reads (`gameplay/notebook.rs`): who the named
/// people are and where they are found, the places named, and the reward. Each sheet repeats
/// about ninety facts; storing them all for fifty-seven sheets was 210 KiB of one record.
fn notebook_reads(key: &str) -> bool {
    matches!(
        key,
        "setting.meeting.name"
            | "setting.destination.name"
            | "setting.confrontation.name"
            | "place_short"
            | "place_detail"
            | "loc.name"
            | "loc.short"
            | "dest.name"
            | "reward.line"
    ) || ["patron.", "contact.", "ant."].iter().any(|prefix| {
        key.strip_prefix(prefix)
            .is_some_and(|rest| matches!(rest, "full" | "first" | "short" | "role" | "the"))
    })
}
fn notebook_fact_sheets(sheets: &[super::FactSheet]) -> Vec<super::FactSheet> {
    let mut stored = Vec::<super::FactSheet>::new();
    for sheet in sheets {
        let Some(scene) = sheet.scene.as_deref() else {
            continue;
        };
        let stored_sheet = if let Some(stored_sheet) = stored
            .iter_mut()
            .find(|stored_sheet| stored_sheet.scene.as_deref() == Some(scene))
        {
            stored_sheet
        } else {
            stored.push(super::FactSheet {
                scene: Some(scene.to_owned()),
                beat: String::new(),
                slot: String::new(),
                speaker: String::new(),
                voice: String::new(),
                delivery: String::new(),
                facts: sheet
                    .facts
                    .iter()
                    .filter(|(key, _)| notebook_reads(key))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                must_mention: Vec::new(),
                forbidden: Vec::new(),
                bounds: 0,
                baseline: String::new(),
                tone: String::new(),
            });
            stored.last_mut().expect("the sheet was just stored")
        };
        for mention in &sheet.must_mention {
            if stored_sheet
                .facts
                .values()
                .any(|fact| fact.eq_ignore_ascii_case(mention))
                && !stored_sheet.must_mention.contains(mention)
            {
                stored_sheet.must_mention.push(mention.clone());
            }
        }
    }
    stored
}
/// Validate exact stored content/record without reading any table or drawing entropy.
pub(crate) fn reload(
    documents: &DemoAdventure,
) -> Result<(LoadedPack, AdventureRecord), RecordError> {
    if [&documents.pack, &documents.tuning, &documents.record]
        .iter()
        .any(|bytes| bytes.is_empty() || bytes.len() > 256 * 1024)
    {
        // Say which document and by how much: "document size" alone cost a debugging session.
        eprintln!(
            "ADVENTURE RECORD refused: document size pack={} tuning={} record={} limit={}",
            documents.pack.len(),
            documents.tuning.len(),
            documents.record.len(),
            256 * 1024
        );
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
        || (!record.fact_sheet.is_empty() && !valid_hash(&record.expression.fact_sheet_sha256))
        || record.pack_sha256 != hash(&documents.pack)
        || record.tuning_sha256 != hash(&documents.tuning)
        || loaded.pack.id != format!("adv-{seed:016x}")
        || record.campaign != loaded.pack.campaign
    {
        return Err(RecordError::Invalid(
            "schema, version, seed or byte revisions",
        ));
    }
    if !record.fact_sheet.is_empty()
        && (record.fact_sheet.len() > 128
            || record.fact_sheet.iter().any(|sheet| {
                sheet.scene.as_deref().is_some_and(|scene| {
                    !pack::SceneKey::ALL.iter().any(|key| {
                        serde_json::to_value(key)
                            .ok()
                            .and_then(|value| value.as_str().map(|value| value == scene))
                            == Some(true)
                    })
                }) || sheet.slot.len() > 96
                    || sheet.beat.len() > 96
                    || sheet.speaker.len() > 96
                    || sheet.voice.len() > 256
                    || sheet.delivery.len() > 256
                    || sheet.tone.len() > 256
                    || sheet.baseline.len() > 4096
                    || sheet.bounds > 4096
                    || sheet.facts.len() > 128
                    || sheet.must_mention.len() > 32
                    || sheet.forbidden.len() > 128
                    || sheet
                        .facts
                        .iter()
                        .any(|(key, value)| key.len() > 96 || value.len() > 4096)
                    || sheet.must_mention.iter().any(|value| value.len() > 256)
                    || sheet.forbidden.iter().any(|value| value.len() > 4096)
            }))
    {
        return Err(RecordError::Invalid("fact sheet"));
    }
    let mut output = Sha256::new();
    output.update(&documents.pack);
    output.update(&documents.tuning);
    if record.expression.output_sha256 != format!("{:x}", output.finalize()) {
        return Err(RecordError::Invalid("expression output hash"));
    }
    match record.setting_state() {
        SettingState::LegacyNoSettings
            if loaded.pack.scenes.iter().any(|s| s.setting.is_some()) =>
        {
            return Err(RecordError::Invalid("legacy record with scene settings"));
        }
        SettingState::Defined(settings)
            if loaded
                .pack
                .scenes
                .iter()
                .any(|s| s.setting.as_ref() != Some(settings.scene(s.key))) =>
        {
            return Err(RecordError::Invalid("record scene settings binding"));
        }
        _ => {}
    }
    // Stored backstories are spoken from, so a repeated id or a leak in their public tiers
    // refuses the record like any other tampered document.
    let mut npc_ids = BTreeSet::new();
    if record.dossiers.len() > 20
        || record.dossiers.iter().any(|dossier| {
            !npc_ids.insert(dossier.npc_id.as_str())
                || dossier.leak().is_some()
                || dossier.emotional_profile.as_ref().is_some_and(|profile| {
                    super::super::npc_dossier::validate_emotional_profile(dossier, profile).is_err()
                })
        })
    {
        return Err(RecordError::Invalid("dossiers"));
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
        || record.rules_mode == RulesMode::Standard
            && loaded.departures.iter().any(|departure| {
                departure != "tactical.cover-centre-line" || loaded.tuning.tactical.is_none()
            })
    {
        return Err(RecordError::Invalid("antagonist or standard rules binding"));
    }
    Ok((loaded, record))
}
pub(crate) enum SettingState<'a> {
    LegacyNoSettings,
    Defined(&'a super::setting::ChapterSettings),
}
impl AdventureRecord {
    pub(crate) fn setting_state(&self) -> SettingState<'_> {
        match &self.settings {
            Some(settings) => SettingState::Defined(settings),
            None => SettingState::LegacyNoSettings,
        }
    }
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

#[cfg(test)]
mod emotional_profile_tests {
    use super::super::{
        FactSheet, RulesMode,
        dossier::{
            Attitude, Binding, Dossier, Effect, FeelingKind, GivenTraits, GroundedFeeling,
            KnowledgeTiers, Length, NpcRole, Reaction, SpeechStyle, StoryTie, Temperament, Trait,
        },
    };
    use super::{AdventureRecord, Expression, GENERATOR_VERSION, RECORD_SCHEMA};

    fn dossier() -> Dossier {
        let reaction = Reaction {
            id: "steady".to_owned(),
            text: "keeps their composure".to_owned(),
            answer: "I hear you.".to_owned(),
            tags: Vec::new(),
            effect: Effect::Steady,
        };
        Dossier {
            npc_id: "patron:test".to_owned(),
            name: "Ari".to_owned(),
            role: NpcRole::Patron,
            role_label: "patron".to_owned(),
            tables_revision: "revision".to_owned(),
            given: GivenTraits {
                mannerism: None,
                delivery: None,
                voice: None,
                tags: Vec::new(),
                note: None,
            },
            origin: trait_value("origin", "grew up in a mill town"),
            past_trade: trait_value("trade", "worked as a baker"),
            formative_event: trait_value("event", "lost a friend in a fire"),
            ideal: trait_value("ideal", "protects the vulnerable"),
            bond: trait_value("bond", "keeps a sibling safe"),
            flaw: trait_value("flaw", "acts too quickly"),
            want: trait_value("want", "finds a new home"),
            fear: trait_value("fear", "fears the open sea"),
            talent: trait_value("talent", "reads old maps"),
            story_tie: StoryTie {
                id: "stake".to_owned(),
                binds: Binding::Stake,
                bound_fact: "a brass key".to_owned(),
                bound_cast: None,
                public: true,
                text: "wants the brass key found".to_owned(),
                speak: "I need the brass key found.".to_owned(),
            },
            motive: None,
            stand_down: None,
            knowledge: KnowledgeTiers {
                says_freely: Vec::new(),
                says_when_trusted: Vec::new(),
                never_says: Vec::new(),
            },
            emotional_profile: None,
            temperament: Temperament {
                flattery: reaction.clone(),
                flirting: reaction.clone(),
                threats: reaction.clone(),
                lies: reaction.clone(),
                bribes: reaction,
            },
            starting_attitude: Attitude::Indifferent,
            speech_style: SpeechStyle {
                id: "plain".to_owned(),
                length: Length::Medium,
                vocabulary: "plain".to_owned(),
                tic: String::new(),
                opener: String::new(),
                closer: String::new(),
                tags: Vec::new(),
            },
            prepared_answers: Default::default(),
            subject_answers: Default::default(),
            curt_answers: Vec::new(),
            rolls: Vec::new(),
        }
    }

    fn trait_value(id: &str, text: &str) -> Trait {
        Trait {
            id: id.to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_bounded_emotional_profile_fits_the_existing_record_document_limit() {
        let mut dossier = dossier();
        let mut profile = dossier.emotional_profile();
        let evidence: String = dossier.fear.text.chars().take(280).collect();
        profile.feelings = (0..8)
            .map(|_| GroundedFeeling {
                kind: FeelingKind::Fear,
                text: "x".repeat(120),
                because: "fear".to_owned(),
                evidence: evidence.chars().take(120).collect(),
            })
            .collect();
        profile.tells = (0..5).map(|_| "t".repeat(80)).collect();
        dossier.emotional_profile = Some(profile);
        let record = AdventureRecord {
            schema: RECORD_SCHEMA.to_owned(),
            seed: "17".to_owned(),
            tables_revision: "0".repeat(64),
            generator_version: GENERATOR_VERSION.to_owned(),
            rules_mode: RulesMode::Standard,
            rolls: Vec::new(),
            expression: Expression {
                source: "patterns".to_owned(),
                fact_sheet_sha256: String::new(),
                output_sha256: "0".repeat(64),
                model: None,
            },
            pack_sha256: "0".repeat(64),
            tuning_sha256: "0".repeat(64),
            seeded_dice: false,
            campaign: None,
            settings: None,
            secret_terms: Vec::new(),
            speakers: Vec::new(),
            scene_cast: Vec::new(),
            dossiers: vec![dossier],
            fact_sheet: (0..50)
                .map(|_| FactSheet {
                    scene: None,
                    beat: "b".to_owned(),
                    slot: "s".to_owned(),
                    speaker: "n".to_owned(),
                    voice: String::new(),
                    delivery: String::new(),
                    facts: Default::default(),
                    must_mention: Vec::new(),
                    forbidden: Vec::new(),
                    bounds: 0,
                    baseline: "b".repeat(4096),
                    tone: String::new(),
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&record).unwrap();
        assert!(bytes.len() <= 256 * 1024);
    }
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
