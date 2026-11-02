//! The two small books the pipeline keeps beside the pending book: the resource requests that are
//! out, and the results of the operations it committed.
//!
//! `RequestBook` holds every request a machine's step asked the broker for: the machine and the
//! key it chose, the `ResultStamp` it was made under (scene, generation, revision, so a late
//! result for an earlier scene is rejected by the table's own judgement, D.2, RES42) and the
//! handle the broker gave once the host issued it. A request is recorded as an intent in the
//! commit and issued after it, so a refused batch leaves nothing at the broker; a restart keeps
//! the intents that were not issued, and issuing is idempotent per key. `ResultBook` holds, for
//! the most recent committed operations, how each ended, so a duplicate operation id after a
//! restart finds its stored result before any port is asked and no die is drawn again (H.2).
//!
//! Both are bounded (`BoundedMap`, the oldest forgotten first), serialisable with a version and
//! checked on load, and keep an exclusive-or digest of their logical state (the broker handle is
//! not part of it, only whether one was bound). They never call a port and never read a clock.

use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::identity::{MachineId, OperationId, RecoveryEpoch, SceneId, SessionRevision};
use crate::machine::RequestKey;
use crate::outcome::{DeclineCode, PendingKind};
use crate::pending::bounded::BoundedMap;
use crate::pending::digest::{Fnv, mix};
use crate::ports::PurposeName;
use crate::table_state::{ResultStamp, SceneGeneration};

/// The schema version of the request and result records.
pub const BOOK_SCHEMA_VERSION: u32 = 1;

/// A `ResultStamp` as saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredStamp {
    scene: u64,
    generation: u64,
    epoch: u64,
    sequence: u64,
}

impl StoredStamp {
    pub fn of(stamp: ResultStamp) -> Self {
        Self {
            scene: stamp.scene.get(),
            generation: stamp.generation.get(),
            epoch: stamp.revision.epoch().get(),
            sequence: stamp.revision.sequence(),
        }
    }

    /// The stamp, unless the record holds an epoch of zero.
    pub fn restore(self) -> Option<ResultStamp> {
        let epoch = RecoveryEpoch::new(self.epoch).ok()?;
        Some(ResultStamp {
            scene: SceneId::new(self.scene),
            generation: SceneGeneration::new(self.generation),
            revision: SessionRevision::new(epoch, self.sequence),
        })
    }
}

/// One request a machine asked for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestEntry {
    machine: MachineId,
    key: u64,
    purpose: PurposeName,
    stamp: Option<StoredStamp>,
    /// The broker's handle, once the host issued the request.
    handle: Option<u64>,
}

impl RequestEntry {
    pub fn machine(&self) -> &MachineId {
        &self.machine
    }

    pub fn key(&self) -> RequestKey {
        RequestKey::new(self.key)
    }

    pub fn purpose(&self) -> &PurposeName {
        &self.purpose
    }

    pub fn is_issued(&self) -> bool {
        self.handle.is_some()
    }

    /// Hashes what the request is and whether it was issued, never the handle itself.
    fn logical_hash(&self) -> u64 {
        let mut hasher = Fnv::new();
        self.machine.hash(&mut hasher);
        self.key.hash(&mut hasher);
        self.purpose.hash(&mut hasher);
        self.stamp.hash(&mut hasher);
        self.handle.is_some().hash(&mut hasher);
        hasher.finish()
    }
}

type RequestSlot = (MachineId, u64);

/// What a finished request was made for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenRequest {
    pub machine: MachineId,
    pub key: RequestKey,
    pub stamp: Option<ResultStamp>,
}

/// Why a saved book was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookLoadError {
    UnsupportedVersion {
        found: u32,
        expected: u32,
    },
    MaximumIsZero,
    TooManyForMaximum,
    /// The same request or operation twice.
    Duplicate,
    /// A handle bound to two requests.
    HandleReused,
    ImpossibleStamp,
}

impl fmt::Display for BookLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found, expected } => write!(
                formatter,
                "a record at version {found} was refused; this engine reads {expected}"
            ),
            Self::MaximumIsZero => formatter.write_str("a record needs a maximum above zero"),
            Self::TooManyForMaximum => formatter.write_str("a record holds more than its maximum"),
            Self::Duplicate => formatter.write_str("a record holds the same entry twice"),
            Self::HandleReused => formatter.write_str("a record binds one handle twice"),
            Self::ImpossibleStamp => formatter.write_str("a record holds an impossible stamp"),
        }
    }
}

impl std::error::Error for BookLoadError {}

/// The saved form of a `RequestBook`, oldest request first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestRecord {
    schema_version: u32,
    maximum: usize,
    entries: Vec<RequestEntry>,
}

/// The requests that are out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestBook {
    entries: BoundedMap<RequestSlot, RequestEntry>,
    by_handle: std::collections::BTreeMap<u64, RequestSlot>,
    digest: u64,
}

impl RequestBook {
    pub fn holding_at_most(maximum: usize) -> Self {
        Self {
            entries: BoundedMap::holding_at_most(maximum),
            by_handle: std::collections::BTreeMap::new(),
            digest: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    pub fn digest(&self) -> u64 {
        self.digest ^ mix(self.entries.maximum() as u64)
    }

    fn forget(&mut self, entry: &RequestEntry) {
        self.digest ^= entry.logical_hash();
        if let Some(handle) = entry.handle {
            self.by_handle.remove(&handle);
        }
    }

    /// Records that a machine's step asked for `purpose` under `key`. A key asked for again
    /// replaces the earlier request.
    pub fn record_intent(
        &mut self,
        machine: MachineId,
        key: RequestKey,
        purpose: PurposeName,
        stamp: Option<ResultStamp>,
    ) {
        let entry = RequestEntry {
            machine: machine.clone(),
            key: key.get(),
            purpose,
            stamp: stamp.map(StoredStamp::of),
            handle: None,
        };
        self.digest ^= entry.logical_hash();
        let removed = self.entries.insert((machine, key.get()), entry);
        for old in removed.replaced.iter().chain(removed.evicted.iter()) {
            self.forget(old);
        }
    }

    /// The requests recorded and not issued to the broker yet, oldest first.
    pub fn unissued(&self) -> Vec<RequestEntry> {
        self.entries
            .oldest_first()
            .into_iter()
            .filter(|entry| !entry.is_issued())
            .cloned()
            .collect()
    }

    /// Binds the broker's handle to a recorded request; whether the request was there.
    pub fn bind_handle(&mut self, machine: &MachineId, key: RequestKey, handle: u64) -> bool {
        let slot = (machine.clone(), key.get());
        let Some(entry) = self.entries.get_mut(&slot) else {
            return false;
        };
        self.digest ^= entry.logical_hash();
        entry.handle = Some(handle);
        self.digest ^= entry.logical_hash();
        self.by_handle.insert(handle, slot);
        true
    }

    /// What a handle was issued for, without taking the request.
    pub fn peek_by_handle(&self, handle: u64) -> Option<TakenRequest> {
        let slot = self.by_handle.get(&handle)?;
        let entry = self.entries.get(slot)?;
        Some(TakenRequest {
            machine: entry.machine.clone(),
            key: entry.key(),
            stamp: entry.stamp.and_then(StoredStamp::restore),
        })
    }

    /// Takes the request a finished handle was issued for, forgetting it.
    pub fn take_by_handle(&mut self, handle: u64) -> Option<TakenRequest> {
        let slot = self.by_handle.remove(&handle)?;
        let entry = self.entries.remove(&slot)?;
        self.digest ^= entry.logical_hash();
        Some(TakenRequest {
            machine: entry.machine.clone(),
            key: entry.key(),
            stamp: entry.stamp.and_then(StoredStamp::restore),
        })
    }

    pub fn to_record(&self) -> RequestRecord {
        RequestRecord {
            schema_version: BOOK_SCHEMA_VERSION,
            maximum: self.entries.maximum(),
            entries: self.entries.oldest_first().into_iter().cloned().collect(),
        }
    }

    pub fn from_record(record: RequestRecord) -> Result<Self, BookLoadError> {
        check_header(record.schema_version, record.maximum, record.entries.len())?;
        let mut book = Self::holding_at_most(record.maximum);
        for entry in record.entries {
            let slot = (entry.machine.clone(), entry.key);
            if book.entries.contains(&slot) {
                return Err(BookLoadError::Duplicate);
            }
            if entry.stamp.is_some_and(|stamp| stamp.restore().is_none()) {
                return Err(BookLoadError::ImpossibleStamp);
            }
            book.digest ^= entry.logical_hash();
            if let Some(handle) = entry.handle
                && book.by_handle.insert(handle, slot.clone()).is_some()
            {
                return Err(BookLoadError::HandleReused);
            }
            book.entries.insert(slot, entry);
        }
        Ok(book)
    }

    #[cfg(test)]
    pub(crate) fn recomputed_digest(&self) -> u64 {
        crate::pending::digest::walked(self.entries.len());
        let folded = self
            .entries
            .values()
            .fold(0, |folded, entry| folded ^ entry.logical_hash());
        folded ^ mix(self.entries.maximum() as u64)
    }
}

fn check_header(version: u32, maximum: usize, held: usize) -> Result<(), BookLoadError> {
    if version != BOOK_SCHEMA_VERSION {
        return Err(BookLoadError::UnsupportedVersion {
            found: version,
            expected: BOOK_SCHEMA_VERSION,
        });
    }
    if maximum == 0 {
        return Err(BookLoadError::MaximumIsZero);
    }
    if held > maximum {
        return Err(BookLoadError::TooManyForMaximum);
    }
    Ok(())
}

/// How a committed operation ended, as the result book keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "ended", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredOutcome {
    Applied,
    Pending { pending: u64, kind: PendingKind },
    Declined { code: DeclineCode },
}

/// One committed operation and how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredResult {
    operation: OperationId,
    outcome: StoredOutcome,
}

impl StoredResult {
    fn logical_hash(&self) -> u64 {
        let mut hasher = Fnv::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

/// The saved form of a `ResultBook`, oldest operation first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultRecord {
    schema_version: u32,
    maximum: usize,
    entries: Vec<StoredResult>,
}

/// How the most recent committed operations ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultBook {
    entries: BoundedMap<OperationId, StoredResult>,
    digest: u64,
}

impl ResultBook {
    pub fn remembering(maximum: usize) -> Self {
        Self {
            entries: BoundedMap::holding_at_most(maximum),
            digest: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    pub fn digest(&self) -> u64 {
        self.digest ^ mix(self.entries.maximum() as u64)
    }

    /// How the operation ended, if it was committed and is still remembered.
    pub fn find(&self, operation: OperationId) -> Option<StoredOutcome> {
        self.entries.get(&operation).map(|stored| stored.outcome)
    }

    /// Remembers how a committed operation ended.
    pub fn record(&mut self, operation: OperationId, outcome: StoredOutcome) {
        let stored = StoredResult { operation, outcome };
        self.digest ^= stored.logical_hash();
        let removed = self.entries.insert(operation, stored);
        for old in removed.replaced.iter().chain(removed.evicted.iter()) {
            self.digest ^= old.logical_hash();
        }
    }

    pub fn to_record(&self) -> ResultRecord {
        ResultRecord {
            schema_version: BOOK_SCHEMA_VERSION,
            maximum: self.entries.maximum(),
            entries: self.entries.oldest_first().into_iter().copied().collect(),
        }
    }

    pub fn from_record(record: ResultRecord) -> Result<Self, BookLoadError> {
        check_header(record.schema_version, record.maximum, record.entries.len())?;
        let mut book = Self::remembering(record.maximum);
        for stored in record.entries {
            if book.entries.contains(&stored.operation) {
                return Err(BookLoadError::Duplicate);
            }
            book.record(stored.operation, stored.outcome);
        }
        Ok(book)
    }
}
