//! The small books the pipeline keeps beside the pending book: the resource requests that are out,
//! the reservations to release once a commit is durable, and how operations ended.
//!
//! `RequestBook` holds every request a machine's step asked the broker for: the machine and the
//! key it chose, the `ResultStamp` it was made under (scene, generation, revision, so a late
//! result for an earlier scene is rejected by the table's own judgement, D.2, RES42) and the
//! handle the broker gave once the host issued it. A request is recorded as an intent in the
//! commit and issued after it, so a refused batch leaves nothing at the broker; a restart keeps
//! the intents that were not issued, and issuing is idempotent per key. `ResultBook` holds, for
//! the most recent committed operations, how each ended, so a duplicate operation id after a
//! restart finds its stored result before any port is asked and no die is drawn again (H.2). The
//! result book keeps what the seat was told (the readout, the decline, the clarification, the
//! routed kind), so a duplicate delivery gets the same answer and writes nothing to the views.
//! An operation id means something only together with its sender, so the result book is keyed by
//! `ResultKey`: the sender, the membership of that sender (a seat's changes each time it joins,
//! the owner queue's rule) and the id. Another seat that reuses an id, or a seat that left and
//! came back, finds nothing and its input runs; the sender, membership and input kind are saved
//! with each result. A record saved before results were kept per sender cannot say who sent each
//! one, so it is refused on load with a typed `BookLoadError` (the host records `load.refused`,
//! H.12) and never guessed.
//! `ReleaseBook` holds the reservations a cancelled pending gave up (`RulesOwner::abandon`): they
//! are recorded with the commit and told to the rules owner after it, like a broker request, so a
//! refused batch leaves the reservation where it was.
//!
//! Both are bounded (`BoundedMap`, the oldest forgotten first), serialisable with a version and
//! checked on load, and keep an exclusive-or digest of their logical state (the broker handle is
//! not part of it, only whether one was bound). They never call a port and never read a clock.

use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::envelope::{InputKind, Sender};
use crate::identity::{
    Correlation, MachineId, OperationId, RecoveryEpoch, SceneId, SeatId, SessionRevision,
};
use crate::machine::RequestKey;
use crate::outcome::{DeclineRecord, PendingKind};
use crate::pending::bounded::BoundedMap;
use crate::pending::digest::{Fnv, mix};
use crate::pipeline::events::ClarifyCause;
use crate::ports::{ActionId, NonActionKind, PurposeName};
use crate::table_state::{ResultStamp, SceneGeneration};

/// The schema version of the request and release records.
pub const BOOK_SCHEMA_VERSION: u32 = 1;

/// The schema version of the result record: 2 keeps each result under its sender and the
/// membership of that sender and records the input kind; version 1 had the operation id alone.
pub const RESULT_SCHEMA_VERSION: u32 = 2;

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
    /// The input that asked, so the broker's work joins back to it (O.4).
    asked_by: Correlation,
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

    /// The correlation of the input that asked for this request.
    pub fn asked_by(&self) -> Correlation {
        self.asked_by
    }

    /// Hashes what the request is and whether it was issued, never the handle itself.
    fn logical_hash(&self) -> u64 {
        let mut hasher = Fnv::new();
        self.machine.hash(&mut hasher);
        self.key.hash(&mut hasher);
        self.purpose.hash(&mut hasher);
        self.stamp.hash(&mut hasher);
        self.asked_by.hash(&mut hasher);
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
    /// A result that does not say who sent it, so whose answer it is cannot be told: refused, never
    /// guessed.
    SenderMissing,
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
            Self::SenderMissing => {
                formatter.write_str("a result record does not say who sent the operation")
            }
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
    /// How many entries have no broker handle yet, so issuing costs nothing when none is due.
    unissued: usize,
    digest: u64,
}

impl RequestBook {
    pub fn holding_at_most(maximum: usize) -> Self {
        Self {
            entries: BoundedMap::holding_at_most(maximum),
            by_handle: std::collections::BTreeMap::new(),
            unissued: 0,
            digest: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    /// How many requests were recorded and not issued to the broker yet.
    pub fn unissued_count(&self) -> usize {
        self.unissued
    }

    pub fn digest(&self) -> u64 {
        self.digest ^ mix(self.entries.maximum() as u64)
    }

    fn forget(&mut self, entry: &RequestEntry) {
        self.digest ^= entry.logical_hash();
        match entry.handle {
            Some(handle) => {
                self.by_handle.remove(&handle);
            }
            None => self.unissued = self.unissued.saturating_sub(1),
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
        asked_by: Correlation,
    ) {
        let entry = RequestEntry {
            machine: machine.clone(),
            key: key.get(),
            purpose,
            stamp: stamp.map(StoredStamp::of),
            asked_by,
            handle: None,
        };
        self.digest ^= entry.logical_hash();
        self.unissued += 1;
        let removed = self.entries.insert((machine, key.get()), entry);
        for old in removed.replaced.iter().chain(removed.evicted.iter()) {
            self.forget(old);
        }
    }

    /// The requests recorded and not issued to the broker yet, oldest first.
    pub fn unissued(&self) -> Vec<RequestEntry> {
        if self.unissued == 0 {
            return Vec::new();
        }
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
        let was_unissued = entry.handle.is_none();
        entry.handle = Some(handle);
        self.digest ^= entry.logical_hash();
        if was_unissued {
            self.unissued = self.unissued.saturating_sub(1);
        }
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
        check_header(
            BOOK_SCHEMA_VERSION,
            record.schema_version,
            record.maximum,
            record.entries.len(),
        )?;
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
            match entry.handle {
                Some(handle) => {
                    if book.by_handle.insert(handle, slot.clone()).is_some() {
                        return Err(BookLoadError::HandleReused);
                    }
                }
                None => book.unissued += 1,
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

fn check_header(
    expected: u32,
    version: u32,
    maximum: usize,
    held: usize,
) -> Result<(), BookLoadError> {
    if version != expected {
        return Err(BookLoadError::UnsupportedVersion {
            found: version,
            expected,
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

/// How an operation ended, as the result book keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "ended", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredEnding {
    Applied,
    Pending { pending: u64, kind: PendingKind },
    Declined,
    NeedsClarification,
}

/// The buttons a seat was asked to pick from, as saved.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredClarification {
    pub options: Vec<ActionId>,
    pub cause: ClarifyCause,
    pub keeps_draft: bool,
}

/// What an operation came to and what its seat was told, so a duplicate delivery of the same
/// operation id gets the same answer and no port is asked (H.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredOutcome {
    pub ending: StoredEnding,
    /// The structured line the seat was shown.
    pub readout: String,
    /// The decline of a declined operation, or of the step that stopped a plan after a valid
    /// prefix was applied.
    pub decline: Option<DeclineRecord>,
    pub clarification: Option<StoredClarification>,
    /// The kind of a line that was not an action and was routed to its handler.
    pub routed: Option<NonActionKind>,
}

/// Who sent an operation, as saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredSender {
    Engine,
    Seat { seat: u64 },
}

impl StoredSender {
    fn of(sender: Sender) -> Self {
        match sender {
            Sender::Engine => Self::Engine,
            Sender::Seat(seat) => Self::Seat { seat: seat.get() },
        }
    }

    fn restore(self) -> Sender {
        match self {
            Self::Engine => Sender::Engine,
            Self::Seat { seat } => Sender::Seat(SeatId::new(seat)),
        }
    }
}

/// What a result is remembered under: the sender, the membership of that sender (zero for the
/// engine; a seat's changes each time it joins, so a rejoined seat, or another player who took its
/// seat id, never meets the result of the one before) and the id the sender gave the operation.
/// The owner queue finds duplicates under the same identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResultKey {
    sender: Sender,
    membership: u64,
    operation: OperationId,
}

impl ResultKey {
    pub fn new(sender: Sender, membership: u64, operation: OperationId) -> Self {
        Self {
            sender,
            membership,
            operation,
        }
    }
}

/// Who sent a result's operation, which membership of that sender and what kind of input it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredOrigin {
    sender: StoredSender,
    membership: u64,
    kind: InputKind,
}

/// One operation and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredResult {
    operation: OperationId,
    /// Absent only in a record saved before version 2, which is refused on load.
    #[serde(default)]
    origin: Option<StoredOrigin>,
    outcome: StoredOutcome,
}

impl StoredResult {
    fn logical_hash(&self) -> u64 {
        let mut hasher = Fnv::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    fn key(&self) -> Option<ResultKey> {
        let origin = self.origin?;
        Some(ResultKey::new(
            origin.sender.restore(),
            origin.membership,
            self.operation,
        ))
    }

    /// The kind of input that made this result.
    pub fn kind(&self) -> Option<InputKind> {
        self.origin.map(|origin| origin.kind)
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

/// How the most recent committed operations ended, per sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultBook {
    entries: BoundedMap<ResultKey, StoredResult>,
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

    /// How the operation ended, if it was committed, is still remembered and was sent by the same
    /// sender under the same membership. Another sender's id, or the id of the seat's earlier
    /// membership, is not found.
    pub fn find(&self, key: ResultKey) -> Option<StoredOutcome> {
        self.entries.get(&key).map(|stored| stored.outcome.clone())
    }

    /// The saved record of the operation, with the kind of input that made it.
    pub fn find_stored(&self, key: ResultKey) -> Option<&StoredResult> {
        self.entries.get(&key)
    }

    /// Remembers how an operation ended, and what kind of input it was.
    pub fn record(&mut self, key: ResultKey, kind: InputKind, outcome: StoredOutcome) {
        let stored = StoredResult {
            operation: key.operation,
            origin: Some(StoredOrigin {
                sender: StoredSender::of(key.sender),
                membership: key.membership,
                kind,
            }),
            outcome,
        };
        self.insert(key, stored);
    }

    fn insert(&mut self, key: ResultKey, stored: StoredResult) {
        self.digest ^= stored.logical_hash();
        let removed = self.entries.insert(key, stored);
        for old in removed.replaced.iter().chain(removed.evicted.iter()) {
            self.digest ^= old.logical_hash();
        }
    }

    pub fn to_record(&self) -> ResultRecord {
        ResultRecord {
            schema_version: RESULT_SCHEMA_VERSION,
            maximum: self.entries.maximum(),
            entries: self.entries.oldest_first().into_iter().cloned().collect(),
        }
    }

    pub fn from_record(record: ResultRecord) -> Result<Self, BookLoadError> {
        check_header(
            RESULT_SCHEMA_VERSION,
            record.schema_version,
            record.maximum,
            record.entries.len(),
        )?;
        let mut book = Self::remembering(record.maximum);
        for stored in record.entries {
            let key = stored.key().ok_or(BookLoadError::SenderMissing)?;
            if book.entries.contains(&key) {
                return Err(BookLoadError::Duplicate);
            }
            book.insert(key, stored);
        }
        Ok(book)
    }
}

/// One reservation a cancelled pending gave up, waiting to be told to the rules owner.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseEntry {
    pending: u64,
    seat: u64,
    action: ActionId,
    /// The input that cancelled the pending.
    asked_by: Correlation,
}

impl ReleaseEntry {
    pub fn pending(&self) -> u64 {
        self.pending
    }

    pub fn seat(&self) -> SeatId {
        SeatId::new(self.seat)
    }

    pub fn action(&self) -> &ActionId {
        &self.action
    }

    pub fn asked_by(&self) -> Correlation {
        self.asked_by
    }

    fn logical_hash(&self) -> u64 {
        let mut hasher = Fnv::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

/// The saved form of a `ReleaseBook`, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRecord {
    schema_version: u32,
    maximum: usize,
    entries: Vec<ReleaseEntry>,
}

/// The reservations to tell the rules owner about once the commit that cancelled their pendings is
/// durable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseBook {
    entries: BoundedMap<u64, ReleaseEntry>,
    digest: u64,
}

impl ReleaseBook {
    pub fn holding_at_most(maximum: usize) -> Self {
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

    /// Records that the pending's reservation is to be released.
    pub fn record(&mut self, pending: u64, seat: SeatId, action: ActionId, asked_by: Correlation) {
        let entry = ReleaseEntry {
            pending,
            seat: seat.get(),
            action,
            asked_by,
        };
        self.digest ^= entry.logical_hash();
        let removed = self.entries.insert(pending, entry);
        for old in removed.replaced.iter().chain(removed.evicted.iter()) {
            self.digest ^= old.logical_hash();
        }
    }

    /// The releases due, oldest first.
    pub fn due(&self) -> Vec<ReleaseEntry> {
        self.entries.oldest_first().into_iter().cloned().collect()
    }

    /// Forgets a release that was told to the rules owner.
    pub fn done(&mut self, pending: u64) {
        if let Some(old) = self.entries.remove(&pending) {
            self.digest ^= old.logical_hash();
        }
    }

    pub fn to_record(&self) -> ReleaseRecord {
        ReleaseRecord {
            schema_version: BOOK_SCHEMA_VERSION,
            maximum: self.entries.maximum(),
            entries: self.entries.oldest_first().into_iter().cloned().collect(),
        }
    }

    pub fn from_record(record: ReleaseRecord) -> Result<Self, BookLoadError> {
        check_header(
            BOOK_SCHEMA_VERSION,
            record.schema_version,
            record.maximum,
            record.entries.len(),
        )?;
        let mut book = Self::holding_at_most(record.maximum);
        for entry in record.entries {
            if book.entries.contains(&entry.pending) {
                return Err(BookLoadError::Duplicate);
            }
            book.digest ^= entry.logical_hash();
            book.entries.insert(entry.pending, entry);
        }
        Ok(book)
    }
}
