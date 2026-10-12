//! Cross-run authority: canonical record envelope, optimistic digest and crash-safe file locks.
use super::campaign::{CampaignError, CampaignRecord, ChapterOutcome, canonical_bytes};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
#[derive(Debug)]
pub(crate) enum StoreError {
    Id,
    Busy,
    Conflict,
    Corrupt(&'static str),
    Campaign(CampaignError),
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
    },
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id => f.write_str(
                "campaign id must contain 1–41 ASCII letters, digits, dash or underscore",
            ),
            Self::Busy => f.write_str(
                "campaign is locked by another writer; retry after it releases the lock",
            ),
            Self::Conflict => f.write_str("campaign previous digest does not match"),
            Self::Corrupt(reason) => write!(f, "campaign authority refused: {reason}"),
            Self::Campaign(e) => e.fmt(f),
            Self::Io { operation, kind } => write!(f, "campaign {operation}: {kind}"),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<CampaignError> for StoreError {
    fn from(e: CampaignError) -> Self {
        Self::Campaign(e)
    }
}
fn ioe(operation: &'static str, error: io::Error) -> StoreError {
    StoreError::Io {
        operation,
        kind: error.kind(),
    }
}
pub(crate) fn validate_id(id: &str) -> Result<(), StoreError> {
    if id.is_empty()
        || id.len() > 41
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Err(StoreError::Id)
    } else {
        Ok(())
    }
}
pub(crate) fn database(id: &str, chapter: u8) -> Result<String, StoreError> {
    validate_id(id)?;
    if !(1..=10).contains(&chapter) {
        return Err(StoreError::Corrupt("chapter number"));
    }
    Ok(format!("df_gameplay_demo_{id}_ch{chapter:02}"))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    digest: String,
    record: CampaignRecord,
}
#[derive(Serialize)]
struct ChapterIndex<'a> {
    number: u8,
    database: &'a str,
    seed: u64,
    adventure_digest: &'a str,
    outcome: Option<&'a ChapterOutcome>,
}
pub(crate) struct CampaignStore {
    directory: PathBuf,
    id: String,
}
pub(crate) struct StoredCampaign {
    pub(crate) record: CampaignRecord,
    pub(crate) digest: String,
}
impl CampaignStore {
    pub(crate) fn new(root: &Path, id: &str) -> Result<Self, StoreError> {
        validate_id(id)?;
        Ok(Self {
            directory: root.join("development/campaigns").join(id),
            id: id.into(),
        })
    }
    fn locked(&self) -> Result<File, StoreError> {
        fs::create_dir_all(&self.directory).map_err(|e| ioe("create directory", e))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join("write.lock"))
            .map_err(|e| ioe("open lock", e))?;
        file.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => StoreError::Busy,
            std::fs::TryLockError::Error(e) => ioe("lock", e),
        })?;
        Ok(file)
    }
    fn read(&self) -> Result<Option<StoredCampaign>, StoreError> {
        let mut file = match File::open(self.directory.join("campaign.json")) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(ioe("read", e)),
        };
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| ioe("read", e))?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(StoreError::Corrupt("size bound"));
        }
        let envelope: Envelope =
            serde_json::from_slice(&bytes).map_err(|_| StoreError::Corrupt("JSON schema"))?;
        if canonical_bytes(&envelope)? != bytes {
            return Err(StoreError::Corrupt("noncanonical encoding"));
        }
        envelope.record.validate()?;
        self.identities(&envelope.record)?;
        if envelope.record.digest()? != envelope.digest {
            return Err(StoreError::Corrupt("digest"));
        }
        Ok(Some(StoredCampaign {
            record: envelope.record,
            digest: envelope.digest,
        }))
    }
    pub(crate) fn load(&self) -> Result<Option<StoredCampaign>, StoreError> {
        let _lock = self.locked()?;
        let stored = self.read()?;
        if let Some(stored) = &stored {
            self.index(&stored.record)?;
        }
        Ok(stored)
    }
    pub(crate) fn save(
        &self,
        record: &CampaignRecord,
        previous: Option<&str>,
    ) -> Result<String, StoreError> {
        let _lock = self.locked()?;
        let current = self.read()?;
        if current.as_ref().map(|c| c.digest.as_str()) != previous {
            return Err(StoreError::Conflict);
        }
        if let Some(current) = &current {
            if current.record.frame_digest != record.frame_digest
                || record.chapters.len() < current.record.chapters.len()
            {
                return Err(StoreError::Corrupt(
                    "immutable campaign frame or chapter prefix changed",
                ));
            }
            for (old, new) in [
                (
                    canonical_bytes(&current.record.facts)?,
                    canonical_bytes(
                        &record
                            .facts
                            .get(..current.record.facts.len())
                            .ok_or(StoreError::Corrupt("fact history shortened"))?,
                    )?,
                ),
                (
                    canonical_bytes(&current.record.clues)?,
                    canonical_bytes(
                        &record
                            .clues
                            .get(..current.record.clues.len())
                            .ok_or(StoreError::Corrupt("clue history shortened"))?,
                    )?,
                ),
                (
                    canonical_bytes(&current.record.rolls)?,
                    canonical_bytes(
                        &record
                            .rolls
                            .get(..current.record.rolls.len())
                            .ok_or(StoreError::Corrupt("roll history shortened"))?,
                    )?,
                ),
            ] {
                if old != new {
                    return Err(StoreError::Corrupt("established history changed"));
                }
            }
            for (old, new) in current.record.chapters.iter().zip(&record.chapters) {
                if canonical_bytes(&old.plan)? != canonical_bytes(&new.plan)?
                    || old.run_database != new.run_database
                    || old.adventure_digest != new.adventure_digest
                    || (old.outcome.is_some()
                        && (old.outcome != new.outcome
                            || old.lieutenant_fate != new.lieutenant_fate))
                {
                    return Err(StoreError::Corrupt(
                        "committed chapter binding or outcome changed",
                    ));
                }
            }
        }
        record.validate()?;
        self.identities(record)?;
        let digest = record.digest()?;
        let bytes = canonical_bytes(&Envelope {
            record: record.clone(),
            digest: digest.clone(),
        })?;
        self.atomic("campaign.json", &bytes)?;
        self.index(record)?;
        Ok(digest)
    }
    fn atomic(&self, name: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let target = self.directory.join(name);
        let parent = target.parent().ok_or(StoreError::Corrupt("file parent"))?;
        fs::create_dir_all(parent).map_err(|e| ioe("create index", e))?;
        let staging = target.with_extension(format!("tmp.{}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&staging)
            .map_err(|e| ioe("stage", e))?;
        file.write_all(bytes).map_err(|e| ioe("write", e))?;
        file.sync_all().map_err(|e| ioe("sync file", e))?;
        drop(file);
        fs::rename(&staging, &target).map_err(|e| ioe("rename", e))?;
        // Directory handles cannot be flushed this way on Windows, where this
        // is a documented no-op; on Unix it is unchanged.
        crate::file_identity::sync_directory(parent).map_err(|e| ioe("sync directory", e))
    }
    fn identities(&self, record: &CampaignRecord) -> Result<(), StoreError> {
        for chapter in &record.chapters {
            if chapter.run_database != database(&self.id, chapter.plan.number)?
                || chapter.adventure_digest.len() != 64
                || !chapter
                    .adventure_digest
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit())
            {
                return Err(StoreError::Corrupt(
                    "chapter database or adventure digest identity",
                ));
            }
        }
        Ok(())
    }
    fn index(&self, record: &CampaignRecord) -> Result<(), StoreError> {
        for chapter in &record.chapters {
            let index = ChapterIndex {
                number: chapter.plan.number,
                database: &chapter.run_database,
                seed: chapter.plan.seed,
                adventure_digest: &chapter.adventure_digest,
                outcome: chapter.outcome.as_ref(),
            };
            let bytes = canonical_bytes(&index)?;
            let name = format!("chapters/{:02}.json", chapter.plan.number);
            match fs::read(self.directory.join(&name)) {
                Ok(existing) if existing == bytes => {}
                Ok(_) => self.atomic(&name, &bytes)?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => self.atomic(&name, &bytes)?,
                Err(e) => return Err(ioe("read index", e)),
            }
        }
        Ok(())
    }
}
