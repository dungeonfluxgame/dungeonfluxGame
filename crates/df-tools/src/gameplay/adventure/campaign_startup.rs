//! Campaign-to-chapter startup binding. Database bytes stay immutable on resume.
use super::super::{
    model,
    startup_configuration::{AdventureSettings, adventure_seed, content_rating},
};
use super::{
    GenerateOptions,
    campaign::{CampaignError, CampaignRecord, digest, generate_frame, next_chapter},
    campaign_render,
    campaign_store::{CampaignStore, StoreError, database, validate_id},
    campaign_tables::{CampaignTables, CampaignTablesError},
    record,
    startup::{PreparedAdventure, StartupError},
};
use df_persistence::local_demo_scope::{ADVENTURE_DOCUMENT_BYTES, AdventureStartup, DemoAdventure};
use std::{
    fmt,
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub(crate) struct CampaignRun {
    pub(crate) root: PathBuf,
    pub(crate) id: String,
    pub(crate) number: u8,
    pub(crate) frame_digest: String,
}
#[derive(Debug)]
pub(crate) enum CampaignStartupError {
    Store(StoreError),
    Tables(CampaignTablesError),
    Campaign(CampaignError),
    Input(&'static str),
    Run(&'static str),
}
impl fmt::Display for CampaignStartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => e.fmt(f),
            Self::Tables(e) => e.fmt(f),
            Self::Campaign(e) => e.fmt(f),
            Self::Input(e) => write!(f, "campaign environment refused: {e}"),
            Self::Run(e) => write!(f, "campaign startup refused: {e}"),
        }
    }
}
impl std::error::Error for CampaignStartupError {}
impl From<StoreError> for CampaignStartupError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}
impl From<CampaignError> for CampaignStartupError {
    fn from(e: CampaignError) -> Self {
        Self::Campaign(e)
    }
}
pub(crate) fn frame(
    root: &Path,
    id: &str,
    tables: &Path,
    seed: Option<u64>,
    options: GenerateOptions,
) -> Result<CampaignRecord, CampaignStartupError> {
    let store = CampaignStore::new(root, id)?;
    if let Some(stored) = store.load()? {
        return Ok(stored.record);
    }
    let tables = CampaignTables::load(tables).map_err(CampaignStartupError::Tables)?;
    let seed = match seed {
        Some(seed) => seed,
        None => u64::from_le_bytes(
            model::random::<8>().map_err(|_| CampaignStartupError::Input("OS seed unavailable"))?,
        ),
    };
    let record = CampaignRecord::new(generate_frame(&tables, seed, options)?)?;
    store.save(&record, None)?;
    Ok(record)
}
pub(crate) fn current_database(
    id: &str,
    record: &CampaignRecord,
) -> Result<String, CampaignStartupError> {
    if let Some(current) = record.chapters.last().filter(|c| c.outcome.is_none()) {
        return Ok(current.run_database.clone());
    }
    let number = record
        .frame
        .outline
        .iter()
        .find(|slot| !slot.skipped && !record.chapters.iter().any(|c| c.plan.number == slot.number))
        .ok_or(CampaignError::Complete)?
        .number;
    Ok(database(id, number)?)
}
pub(crate) fn select(
    settings: &AdventureSettings,
    root: &Path,
    run_database: &str,
    state: AdventureStartup,
) -> Result<PreparedAdventure, StartupError> {
    let (id, seed) = (|| {
        let id = std::env::var("DF_GAMEPLAY_CAMPAIGN")
            .map_err(|_| CampaignStartupError::Input("DF_GAMEPLAY_CAMPAIGN is required"))?;
        validate_id(&id)?;
        let seed = match std::env::var("DF_GAMEPLAY_CAMPAIGN_SEED") {
            Ok(value) => Some(adventure_seed(&value).map_err(|_| {
                CampaignStartupError::Input("DF_GAMEPLAY_CAMPAIGN_SEED must be a u64")
            })?),
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => {
                return Err(CampaignStartupError::Input(
                    "DF_GAMEPLAY_CAMPAIGN_SEED must be Unicode",
                ));
            }
        };
        Ok::<_, CampaignStartupError>((id, seed))
    })()
    .map_err(StartupError::Campaign)?;
    select_campaign(settings, root, id, seed, run_database, state)
}
pub(crate) fn select_campaign(
    settings: &AdventureSettings,
    root: &Path,
    id: String,
    seed: Option<u64>,
    run_database: &str,
    state: AdventureStartup,
) -> Result<PreparedAdventure, StartupError> {
    let result = (|| {
        frame(
            root,
            &id,
            &settings.tables,
            seed,
            GenerateOptions {
                rules_mode: settings.rules_mode,
                rating: content_rating(),
            },
        )?;
        let store = CampaignStore::new(root, &id)?;
        let stored = store.load()?.ok_or(CampaignStartupError::Run(
            "authority missing after creation",
        ))?;
        let mut record = stored.record;
        if current_database(&id, &record)? != run_database {
            return Err(CampaignStartupError::Run(
                "use the campaign's current chapter database; unfinished chapters must resume",
            ));
        }
        let directory = root.join("development/runs").join(run_database);
        let (documents, fresh, number) = match state {
            AdventureStartup::Stored(documents) => {
                let chapter = record
                    .chapters
                    .last()
                    .filter(|c| c.outcome.is_none())
                    .ok_or(CampaignStartupError::Run(
                        "the next chapter requires a fresh database",
                    ))?;
                if digest(&documents.record) != chapter.adventure_digest {
                    return Err(CampaignStartupError::Run(
                        "adventure digest disagrees with campaign index",
                    ));
                }
                record::verify_cache(&directory, &documents).map_err(|_| {
                    CampaignStartupError::Run("resume cache disagrees with database")
                })?;
                (documents, false, chapter.plan.number)
            }
            AdventureStartup::Fresh => {
                if let Some(chapter) = record.chapters.last().filter(|c| c.outcome.is_none()) {
                    // Recover a crash between campaign binding and the first database transaction.
                    let read = |name| {
                        {
                            use std::io::Read;
                            let file = std::fs::File::open(directory.join(name));
                            file.and_then(|file| {
                                let mut bytes = Vec::new();
                                file.take(ADVENTURE_DOCUMENT_BYTES as u64 + 1)
                                    .read_to_end(&mut bytes)?;
                                if bytes.len() > ADVENTURE_DOCUMENT_BYTES {
                                    return Err(std::io::Error::new(
                                        std::io::ErrorKind::InvalidData,
                                        "chapter file bound",
                                    ));
                                }
                                Ok(bytes)
                            })
                        }
                        .map_err(|_| {
                            CampaignStartupError::Run("pending chapter immutable cache is missing")
                        })
                    };
                    let documents = DemoAdventure {
                        pack: read("pack.json")?,
                        tuning: read("tuning.json")?,
                        record: read("adventure.json")?,
                    };
                    if digest(&documents.record) != chapter.adventure_digest {
                        return Err(CampaignStartupError::Run("pending chapter digest mismatch"));
                    }
                    record::reload(&documents)
                        .map_err(|_| CampaignStartupError::Run("pending chapter invalid"))?;
                    (documents, true, chapter.plan.number)
                } else {
                    let tables = CampaignTables::load(&settings.tables)
                        .map_err(CampaignStartupError::Tables)?;
                    let mut plan = next_chapter(&tables, &record.frame, &record)?;
                    let mut generated =
                        campaign_render::generate_chapter(&tables, &record.frame, &mut plan)?;
                    generated.pack.campaign = Some(super::super::pack::CampaignIdentity {
                        id: id.clone(),
                        chapter: plan.number,
                        frame_sha256: record.frame_digest.clone(),
                    });
                    let documents = generated
                        .documents(true)
                        .map_err(|_| CampaignStartupError::Run("chapter encoding"))?;
                    create_campaign_cache(&directory, &documents)?;
                    let number = plan.number;
                    record.begin(plan, run_database.into(), digest(&documents.record))?;
                    store.save(&record, Some(&stored.digest))?;
                    (documents, true, number)
                }
            }
        };
        let (loaded, adventure_record) =
            record::reload(&documents).map_err(|_| CampaignStartupError::Run("chapter reload"))?;
        let identity = loaded
            .pack
            .campaign
            .as_ref()
            .ok_or(CampaignStartupError::Run(
                "missing persisted campaign binding",
            ))?;
        if identity.id != id
            || identity.chapter != number
            || identity.frame_sha256 != record.frame_digest
        {
            return Err(CampaignStartupError::Run(
                "persisted campaign binding disagrees with authority",
            ));
        }
        let chapter = record
            .chapters
            .iter()
            .find(|chapter| chapter.plan.number == number)
            .ok_or(CampaignStartupError::Run("chapter binding missing"))?;
        let context = df_observe::OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.into(),
        };
        let mut span = df_observe::begin(&context, "gameplay.campaign.chapter");
        span.event(
            "gameplay.campaign.chapter",
            &[
                ("campaign.id", id.clone()),
                ("campaign.chapter", number.to_string()),
                ("campaign.frame_sha256", record.frame_digest.clone()),
                ("campaign.adventure_sha256", digest(&documents.record)),
                ("campaign.clock", record.clock_position.to_string()),
            ],
        );
        span.finish_unmeasured(if fresh { "generated" } else { "resumed" });
        println!(
            "CAMPAIGN id={} seed=0x{:016x} title={:?} act={} chapter={}/{} type={}",
            id,
            record.frame.seed,
            record.frame.title,
            chapter.plan.act,
            number,
            record.frame.outline.len(),
            chapter.plan.chapter_type
        );
        let prose = adventure_record.prose_summary();
        Ok(PreparedAdventure {
            directory,
            loaded,
            documents,
            record: adventure_record,
            fresh,
            prose,
            campaign: Some(CampaignRun {
                root: root.into(),
                id,
                number,
                frame_digest: record.frame_digest.clone(),
            }),
        })
    })();
    result.map_err(StartupError::Campaign)
}

/// New caches publish as whole directories. Only unbound, matching partial orphans
/// from an interrupted older startup may be repaired, never divergent bytes.
pub(crate) fn create_campaign_cache(
    directory: &Path,
    documents: &DemoAdventure,
) -> Result<(), CampaignStartupError> {
    let parent = directory
        .parent()
        .ok_or(CampaignStartupError::Run("cache parent"))?;
    std::fs::create_dir_all(parent).map_err(|_| CampaignStartupError::Run("cache parent"))?;
    let pairs = [
        ("pack.json", &documents.pack),
        ("tuning.json", &documents.tuning),
        ("adventure.json", &documents.record),
    ];
    let verify_partial = |name: &str, expected: &[u8]| -> Result<bool, CampaignStartupError> {
        use std::io::Read;
        let path = directory.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
                return Err(CampaignStartupError::Run("orphan cache file type"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err(CampaignStartupError::Run("orphan cache metadata")),
        };
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|file| {
                file.take(ADVENTURE_DOCUMENT_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
            })
            .map_err(|_| CampaignStartupError::Run("orphan cache read"))?;
        if bytes.len() > expected.len() || !expected.starts_with(&bytes) {
            return Err(CampaignStartupError::Run(
                "orphan cache differs from deterministic chapter",
            ));
        }
        Ok(bytes == expected)
    };
    let existing = directory.exists();
    if existing {
        let metadata = std::fs::symlink_metadata(directory)
            .map_err(|_| CampaignStartupError::Run("cache metadata"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(CampaignStartupError::Run("cache directory type"));
        }
        for entry in
            std::fs::read_dir(directory).map_err(|_| CampaignStartupError::Run("cache entries"))?
        {
            let entry = entry.map_err(|_| CampaignStartupError::Run("cache entry"))?;
            if !pairs.iter().any(|(name, _)| entry.file_name() == *name) {
                return Err(CampaignStartupError::Run("unexpected orphan cache file"));
            }
        }
        let mut complete = true;
        for (name, bytes) in &pairs {
            complete &= verify_partial(name, bytes)?;
        }
        if complete {
            return Ok(());
        }
    }
    let nonce =
        model::random::<8>().map_err(|_| CampaignStartupError::Input("cache staging entropy"))?;
    let staging = parent.join(format!(
        ".campaign-stage-{}-{}",
        std::process::id(),
        nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    record::create_cache(&staging, documents)
        .map_err(|_| CampaignStartupError::Run("stage chapter cache"))?;
    if existing {
        for (name, bytes) in &pairs {
            if !verify_partial(name, bytes)? {
                std::fs::rename(staging.join(name), directory.join(name))
                    .map_err(|_| CampaignStartupError::Run("repair matching orphan cache"))?;
            }
        }
        std::fs::remove_dir_all(&staging)
            .map_err(|_| CampaignStartupError::Run("remove owned staging"))?;
    } else if std::fs::rename(&staging, directory).is_err() {
        record::verify_cache(directory, documents)
            .map_err(|_| CampaignStartupError::Run("concurrent chapter cache differs"))?;
        std::fs::remove_dir_all(&staging)
            .map_err(|_| CampaignStartupError::Run("remove owned staging"))?;
    }
    // Directory handles cannot be flushed this way on Windows, where this is a
    // documented no-op; on Unix it is unchanged.
    crate::file_identity::sync_directory(parent)
        .map_err(|_| CampaignStartupError::Run("sync cache parent"))?;
    Ok(())
}
