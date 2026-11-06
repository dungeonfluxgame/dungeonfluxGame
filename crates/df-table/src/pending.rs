//! Pending resolutions held in table state, the windows that carry their offers, and answers by
//! pending id (RES26, C.4, M2-D4 to M2-D7).
//!
//! It owns `PendingResolution` (what is waiting: its kind, who opened it and in which operation,
//! the continuation to resume, the dice already drawn and the `ResolutionWindow` of offers),
//! `PendingBook` (the open ones, bounded, indexed and serialisable) and, in the submodules, the
//! window, the rulings and the digest hasher. A pending is committed together with the turn that
//! opened it (phase one of C.4). The dice drawn so far are stored in order, so a restart resumes
//! with them and never draws again (coding-style section 8, pattern 5); an answer is phase two,
//! which the pipeline runs against the state as it is then. `PendingBook::to_record` and
//! `from_record` are the saved form; a record newer than this engine or with an impossible shape
//! is refused with a typed `PendingLoadError`, never loaded wrongly (H.12). The wait of a pending
//! is saved as an age and put back against the new process's clock, because a monotonic reading
//! means nothing to a later process.
//!
//! The book keeps three indexes (by operation, by scene and by the seat each pending still waits
//! on) and an exclusive-or digest of every pending's logical state, all updated where an entry
//! changes, so a lookup or the digest costs the same however many pendings are open. The digest
//! leaves out clock readings and the nudge flag. Waiting spends nothing, rolls nothing and invents
//! nothing: nothing here reads a clock or draws a die, and a window never times out into a decline
//! (`window_timeout` none, RES26). The nudge sweep only reports which pendings are due. It never
//! decides what an answer means (the rules owner does, through the pipeline) and never changes a
//! machine's state. A book that is full refuses a new pending with `PendingError::TooManyOpen`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::envelope::PendingId;
use crate::identity::{OperationId, SceneId, SeatId, TableId, TurnSeq};
use crate::limits::MAXIMUM_OPEN_PENDINGS;
use crate::listed::listed_enum;
use crate::machine::slot::{LoadCause, RecordKind};
use crate::outcome::PendingKind;
use crate::ports::{ActionId, ConfirmationReason, Draw, RulingRequest};

pub(crate) mod bounded;
pub(crate) mod digest;
pub(crate) mod ruling;
pub(crate) mod window;

use digest::{Fnv, mix};
pub use ruling::{RulingBook, RulingLoadError, RulingRecord, RulingSource, ScopedRuling};
pub use window::{AnswerError, Completion, ResolutionWindow, SeatOffer, WindowError};

/// The schema version of `PendingRecord`. Raised when the saved shape changes.
pub(crate) const PENDING_SCHEMA_VERSION: u32 = 1;

/// One die as saved: the sides and the value, checked when it is read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredDraw {
    sides: u16,
    value: u16,
}

impl StoredDraw {
    pub fn of(draw: Draw) -> Self {
        Self {
            sides: draw.sides().get(),
            value: draw.value(),
        }
    }

    pub fn sides(self) -> u16 {
        self.sides
    }

    pub fn value(self) -> u16 {
        self.value
    }

    /// The die, unless the record names an impossible one.
    pub fn restore(self) -> Result<Draw, PendingLoadError> {
        NonZeroU16::new(self.sides)
            .and_then(|sides| Draw::new(sides, self.value))
            .ok_or(PendingLoadError::Malformed(MalformedPending::ImpossibleDie))
    }
}

listed_enum! {
    /// Where a pending stands. A ruling waits for an adjudicator from the moment it opens.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PendingStatus {
        Open,
        /// A pending ruling waiting for the host's tap (RES26): a state of the ruling, not of a
        /// machine.
        AwaitingAdjudicator,
    }
}

impl PendingStatus {
    pub fn key(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::AwaitingAdjudicator => "awaiting_adjudicator",
        }
    }
}

/// What to do when the pending is answered.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "continuation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Continuation {
    /// The seat's own action waits for its one-tap yes (C.5); the machine has not seen it.
    /// `reason` is why the rules owner asked, for the words of the confirmation card.
    Confirm {
        seat: SeatId,
        action: ActionId,
        reason: ConfirmationReason,
    },
    /// A resolution of the rules owner waits for answers or a roll; the machine waits on the id.
    /// `roll` holds the sides of the dice a `Roll` makes when answered.
    Resolve {
        seat: SeatId,
        action: ActionId,
        nesting: u32,
        roll: Vec<u16>,
    },
    /// The seat's action waits for a ruling; the machine has not seen it. `reuse` is true when
    /// the question was answered before and only the once-a-scene confirmation is asked (RES48).
    Ruling {
        seat: SeatId,
        action: ActionId,
        request: RulingRequest,
        reuse: bool,
    },
}

impl Continuation {
    /// The seat whose action this is.
    pub fn seat(&self) -> SeatId {
        match self {
            Self::Confirm { seat, .. } | Self::Resolve { seat, .. } | Self::Ruling { seat, .. } => {
                *seat
            }
        }
    }

    pub fn action(&self) -> &ActionId {
        match self {
            Self::Confirm { action, .. }
            | Self::Resolve { action, .. }
            | Self::Ruling { action, .. } => action,
        }
    }
}

/// Everything needed to open a pending.
#[derive(Debug, Clone)]
pub struct PendingOpening {
    pub id: PendingId,
    pub kind: PendingKind,
    pub opened_by: SeatId,
    pub operation: OperationId,
    pub turn: TurnSeq,
    pub now_ms: u64,
    pub scene: Option<SceneId>,
    pub continuation: Continuation,
    pub draws: Vec<StoredDraw>,
    pub window: ResolutionWindow,
}

/// One resolution waiting for an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingResolution {
    id: u64,
    kind: PendingKind,
    status: PendingStatus,
    opened_by: SeatId,
    opened_by_operation: OperationId,
    opened_turn: TurnSeq,
    /// A reading of the clock the table runs on; negative when the pending was opened before the
    /// origin of the current process's clock. In the saved record it is an age (see
    /// `PendingBook::to_record`), which is why the key says so.
    #[serde(rename = "opened_age_ms")]
    opened_ms: i64,
    scene: Option<SceneId>,
    continuation: Continuation,
    draws: Vec<StoredDraw>,
    window: ResolutionWindow,
    nudged: bool,
    held_seats: Vec<SeatId>,
}

impl PendingResolution {
    pub fn open(opening: PendingOpening) -> Self {
        let status = match opening.kind {
            PendingKind::Ruling => PendingStatus::AwaitingAdjudicator,
            PendingKind::Choice | PendingKind::Roll | PendingKind::Reaction => PendingStatus::Open,
        };
        Self {
            id: opening.id.get(),
            kind: opening.kind,
            status,
            opened_by: opening.opened_by,
            opened_by_operation: opening.operation,
            opened_turn: opening.turn,
            opened_ms: i64::try_from(opening.now_ms).unwrap_or(i64::MAX),
            scene: opening.scene,
            continuation: opening.continuation,
            draws: opening.draws,
            window: opening.window,
            nudged: false,
            held_seats: Vec::new(),
        }
    }

    pub fn id(&self) -> PendingId {
        PendingId::new(self.id)
    }

    pub fn kind(&self) -> PendingKind {
        self.kind
    }

    pub fn status(&self) -> PendingStatus {
        self.status
    }

    pub fn opened_by(&self) -> SeatId {
        self.opened_by
    }

    pub fn opened_by_operation(&self) -> OperationId {
        self.opened_by_operation
    }

    pub fn scene(&self) -> Option<SceneId> {
        self.scene
    }

    pub fn continuation(&self) -> &Continuation {
        &self.continuation
    }

    pub fn window(&self) -> &ResolutionWindow {
        &self.window
    }

    /// The dice drawn for this resolution so far, oldest first.
    pub fn draws(&self) -> &[StoredDraw] {
        &self.draws
    }

    pub fn is_nudged(&self) -> bool {
        self.nudged
    }

    /// Whether a seat the pending waits on is away (a held turn, C.4).
    pub fn is_held(&self) -> bool {
        !self.held_seats.is_empty()
    }

    /// How long it has waited, in milliseconds.
    pub fn waited_ms(&self, now_ms: u64) -> u64 {
        let waited = i128::from(now_ms) - i128::from(self.opened_ms);
        u64::try_from(waited.max(0)).unwrap_or(u64::MAX)
    }

    /// The dice read back from the record, or why the record is impossible.
    pub fn restored_draws(&self) -> Result<Vec<Draw>, PendingLoadError> {
        self.draws.iter().map(|draw| draw.restore()).collect()
    }

    /// A copy with another window.
    pub fn with_window(&self, window: ResolutionWindow) -> Self {
        Self {
            window,
            ..self.clone()
        }
    }

    /// A copy with more dice appended, in the order they were drawn.
    pub fn with_more_draws(&self, more: impl IntoIterator<Item = StoredDraw>) -> Self {
        let mut next = self.clone();
        next.draws.extend(more);
        next
    }

    /// A copy waiting on the next stage: a new kind, continuation and window, the dice kept. A
    /// nudge is shown again for the new stage.
    pub fn advanced(
        &self,
        kind: PendingKind,
        continuation: Continuation,
        window: ResolutionWindow,
    ) -> Self {
        Self {
            kind,
            continuation,
            window,
            nudged: false,
            ..self.clone()
        }
    }

    /// A hash of what the pending is, leaving out the clock reading and the nudge flag, which are
    /// observations of time and not state a replay must agree on.
    fn logical_hash(&self) -> u64 {
        let mut hasher = Fnv::new();
        self.id.hash(&mut hasher);
        self.kind.hash(&mut hasher);
        self.status.hash(&mut hasher);
        self.opened_by.hash(&mut hasher);
        self.opened_by_operation.hash(&mut hasher);
        self.opened_turn.hash(&mut hasher);
        self.scene.hash(&mut hasher);
        self.continuation.hash(&mut hasher);
        self.draws.hash(&mut hasher);
        self.window.hash(&mut hasher);
        self.held_seats.hash(&mut hasher);
        hasher.finish()
    }
}

/// Why a pending could not be opened or replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingError {
    /// The book holds its maximum already.
    TooManyOpen,
    /// The id is not the next one the book hands out, or is in use.
    IdNotNext,
    /// The operation already opened a pending.
    OperationAlreadyOpened,
    Unknown,
}

impl fmt::Display for PendingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyOpen => formatter.write_str("too many pending resolutions are open"),
            Self::IdNotNext => formatter.write_str("the pending id is not the next one"),
            Self::OperationAlreadyOpened => {
                formatter.write_str("the operation already opened a pending resolution")
            }
            Self::Unknown => formatter.write_str("no such pending resolution is open"),
        }
    }
}

impl std::error::Error for PendingError {}

/// What is wrong with a saved record that parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedPending {
    ImpossibleDie,
    DuplicateId,
    IdNotBelowNext,
    DuplicateOperation,
    /// More pendings than the book's maximum.
    TooManyForMaximum,
}

/// Why a saved pending record was refused (H.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingLoadError {
    UnsupportedVersion { found: u32, expected: u32 },
    Malformed(MalformedPending),
}

impl PendingLoadError {
    pub fn cause(&self) -> LoadCause {
        match self {
            Self::UnsupportedVersion { .. } => LoadCause::UnsupportedVersion,
            Self::Malformed(_) => LoadCause::Malformed,
        }
    }

    /// The `load.refused` event of refusing the record when the host loads `table`.
    pub fn event(&self, table: TableId) -> crate::event::TableEvent {
        let version = match self {
            Self::UnsupportedVersion { found, .. } => u64::from(*found),
            Self::Malformed(_) => u64::from(PENDING_SCHEMA_VERSION),
        };
        crate::event::TableEvent::LoadRefused {
            table,
            record: RecordKind::PendingBook,
            machine: None,
            version,
            expected: u64::from(PENDING_SCHEMA_VERSION),
            cause: self.cause(),
        }
    }
}

impl fmt::Display for PendingLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found, expected } => write!(
                formatter,
                "a pending record at version {found} was refused; this engine reads {expected}"
            ),
            Self::Malformed(reason) => {
                write!(formatter, "a pending record is malformed: {reason:?}")
            }
        }
    }
}

impl std::error::Error for PendingLoadError {}

/// The saved form of a `PendingBook`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingRecord {
    schema_version: u32,
    next_id: u64,
    pendings: Vec<PendingResolution>,
}

/// A pending that is due its nudge line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nudged {
    pub pending: PendingId,
    pub kind: PendingKind,
    pub waited_ms: u64,
}

/// A change to the marks of a pending, which the commit makes after the book's own changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingMark {
    /// The seat is away: every pending it has not answered is held.
    Hold(SeatId),
    /// The seat is back: what was held for it is released.
    Release(SeatId),
    /// The pending was shown its nudge line.
    Nudged(PendingId),
}

/// The open pending resolutions of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingBook {
    next_id: u64,
    open: BTreeMap<u64, PendingResolution>,
    by_operation: BTreeMap<OperationId, u64>,
    by_scene: BTreeMap<Option<SceneId>, usize>,
    awaiting: BTreeMap<SeatId, BTreeSet<u64>>,
    maximum: usize,
    digest: u64,
}

impl Default for PendingBook {
    fn default() -> Self {
        Self::holding_at_most(MAXIMUM_OPEN_PENDINGS)
    }
}

impl PendingBook {
    /// An empty book that holds at most `maximum` open pendings.
    pub fn holding_at_most(maximum: usize) -> Self {
        Self {
            next_id: 1,
            open: BTreeMap::new(),
            by_operation: BTreeMap::new(),
            by_scene: BTreeMap::new(),
            awaiting: BTreeMap::new(),
            maximum,
            digest: 0,
        }
    }

    /// The id the next pending will get. Asking changes nothing, so a turn that fails after
    /// asking leaves no gap.
    pub fn next_id(&self) -> PendingId {
        PendingId::new(self.next_id)
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// A digest of the logical state of the book: every open pending, the next id and the bound.
    /// It is kept up to date where an entry changes, so asking costs nothing.
    pub fn digest(&self) -> u64 {
        self.digest ^ mix(self.next_id) ^ mix(self.maximum as u64)
    }

    pub fn get(&self, id: PendingId) -> Option<&PendingResolution> {
        self.open.get(&id.get())
    }

    /// The pending an operation opened, so the replay of that input after a restart finds it
    /// instead of drawing again.
    pub fn opened_by(&self, operation: OperationId) -> Option<&PendingResolution> {
        self.by_operation
            .get(&operation)
            .and_then(|id| self.open.get(id))
    }

    /// The open pendings, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &PendingResolution> {
        self.open.values()
    }

    /// How many open pendings belong to the scene: a hand-off is declined `pending_open` while
    /// any exists (RES26).
    pub fn open_in_scene(&self, scene: Option<SceneId>) -> usize {
        self.by_scene.get(&scene).copied().unwrap_or(0)
    }

    fn index_in(&mut self, pending: &PendingResolution) {
        self.by_operation
            .insert(pending.opened_by_operation, pending.id);
        *self.by_scene.entry(pending.scene).or_insert(0) += 1;
        for seat in pending.window.awaiting() {
            self.awaiting.entry(seat).or_default().insert(pending.id);
        }
        self.digest ^= pending.logical_hash();
    }

    fn index_out(&mut self, pending: &PendingResolution) {
        self.by_operation.remove(&pending.opened_by_operation);
        if let Some(count) = self.by_scene.get_mut(&pending.scene) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.by_scene.remove(&pending.scene);
            }
        }
        for seat in pending.window.awaiting() {
            if let Some(ids) = self.awaiting.get_mut(&seat) {
                ids.remove(&pending.id);
                if ids.is_empty() {
                    self.awaiting.remove(&seat);
                }
            }
        }
        self.digest ^= pending.logical_hash();
    }

    /// Whether the changes of one commit can be made: room for what opens after what closes, the
    /// ids the next ones the book hands out, and every operation new. Checking changes nothing,
    /// so a commit that is refused leaves the book as it was.
    pub fn can_apply(
        &self,
        close: &[PendingId],
        replace: &[PendingResolution],
        open: &[PendingResolution],
    ) -> Result<(), PendingError> {
        if replace
            .iter()
            .any(|pending| !self.open.contains_key(&pending.id))
        {
            return Err(PendingError::Unknown);
        }
        let closing = close
            .iter()
            .filter(|id| self.open.contains_key(&id.get()))
            .count();
        if self.open.len() - closing + open.len() > self.maximum {
            return Err(PendingError::TooManyOpen);
        }
        let mut expected = self.next_id;
        let mut operations = BTreeSet::new();
        for pending in open {
            if pending.id != expected {
                return Err(PendingError::IdNotNext);
            }
            let taken = self.by_operation.contains_key(&pending.opened_by_operation);
            if taken || !operations.insert(pending.opened_by_operation) {
                return Err(PendingError::OperationAlreadyOpened);
            }
            expected = expected.checked_add(1).ok_or(PendingError::TooManyOpen)?;
        }
        Ok(())
    }

    /// Makes the changes `can_apply` allowed: closes, then replaces, then opens.
    pub fn apply_changes(
        &mut self,
        close: &[PendingId],
        replace: Vec<PendingResolution>,
        open: Vec<PendingResolution>,
    ) {
        for id in close {
            self.remove(*id);
        }
        for pending in replace {
            self.replace_unchecked(pending);
        }
        for pending in open {
            self.insert_unchecked(pending);
        }
    }

    fn insert_unchecked(&mut self, pending: PendingResolution) {
        self.next_id = pending.id.saturating_add(1).max(self.next_id);
        self.index_in(&pending);
        self.open.insert(pending.id, pending);
    }

    fn replace_unchecked(&mut self, pending: PendingResolution) {
        if let Some(old) = self.open.remove(&pending.id) {
            self.index_out(&old);
        }
        self.index_in(&pending);
        self.open.insert(pending.id, pending);
    }

    /// Adds a pending, which must carry the id `next_id` gave out.
    pub fn insert(&mut self, pending: PendingResolution) -> Result<(), PendingError> {
        self.can_apply(&[], &[], std::slice::from_ref(&pending))?;
        self.insert_unchecked(pending);
        Ok(())
    }

    /// Replaces an open pending with a changed copy of itself.
    pub fn replace(&mut self, pending: PendingResolution) -> Result<(), PendingError> {
        self.can_apply(&[], std::slice::from_ref(&pending), &[])?;
        self.replace_unchecked(pending);
        Ok(())
    }

    /// Closes a pending, which is resolved or cancelled.
    pub fn remove(&mut self, id: PendingId) -> Option<PendingResolution> {
        let removed = self.open.remove(&id.get())?;
        self.index_out(&removed);
        Some(removed)
    }

    /// The offers still waiting for this seat, by pending id: what a reconnecting phone shows
    /// again (C.4).
    pub fn offers_awaiting(&self, seat: SeatId) -> Vec<(PendingId, &SeatOffer)> {
        let Some(ids) = self.awaiting.get(&seat) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| {
                let pending = self.open.get(id)?;
                Some((pending.id(), pending.window.offer_for(seat)?))
            })
            .collect()
    }

    /// The pendings a seat's drop would newly hold: those it has not answered and that are not
    /// held for it yet.
    pub fn would_hold(&self, seat: SeatId) -> Vec<PendingId> {
        let Some(ids) = self.awaiting.get(&seat) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| self.open.get(id))
            .filter(|pending| !pending.held_seats.contains(&seat))
            .map(PendingResolution::id)
            .collect()
    }

    /// The pendings held for `seat`, which its return would release.
    pub fn would_release(&self, seat: SeatId) -> Vec<PendingId> {
        let Some(ids) = self.awaiting.get(&seat) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| self.open.get(id))
            .filter(|pending| pending.held_seats.contains(&seat))
            .map(PendingResolution::id)
            .collect()
    }

    /// Which pendings have waited their nudge time and were not nudged yet: a ruling after
    /// `ruling_nudge_ms`, any other pending after `held_nudge_ms`. Reading only; the nudge is
    /// marked by the commit (`PendingMark::Nudged`).
    pub fn would_nudge(
        &self,
        now_ms: u64,
        ruling_nudge_ms: u64,
        held_nudge_ms: u64,
    ) -> Vec<Nudged> {
        digest::walked(self.open.len());
        self.open
            .values()
            .filter_map(|pending| {
                let limit = match pending.kind {
                    PendingKind::Ruling => ruling_nudge_ms,
                    PendingKind::Choice | PendingKind::Roll | PendingKind::Reaction => {
                        held_nudge_ms
                    }
                };
                let waited_ms = pending.waited_ms(now_ms);
                (!pending.nudged && waited_ms >= limit).then_some(Nudged {
                    pending: pending.id(),
                    kind: pending.kind,
                    waited_ms,
                })
            })
            .collect()
    }

    /// Makes one mark. A mark for a pending or seat that is gone changes nothing.
    pub fn mark(&mut self, mark: PendingMark) {
        match mark {
            PendingMark::Hold(seat) => self.change_held(seat, true),
            PendingMark::Release(seat) => self.change_held(seat, false),
            PendingMark::Nudged(id) => {
                if let Some(pending) = self.open.get_mut(&id.get()) {
                    pending.nudged = true;
                }
            }
        }
    }

    fn change_held(&mut self, seat: SeatId, hold: bool) {
        let ids = if hold {
            self.would_hold(seat)
        } else {
            self.would_release(seat)
        };
        for id in ids {
            let Some(mut pending) = self.open.remove(&id.get()) else {
                continue;
            };
            self.index_out(&pending);
            if hold {
                pending.held_seats.push(seat);
            } else {
                pending.held_seats.retain(|held| *held != seat);
            }
            self.index_in(&pending);
            self.open.insert(pending.id, pending);
        }
    }

    /// The saved form, with the dice already drawn. The wait of each pending is saved as its age
    /// at `now_ms`.
    pub fn to_record(&self, now_ms: u64) -> PendingRecord {
        PendingRecord {
            schema_version: PENDING_SCHEMA_VERSION,
            next_id: self.next_id,
            pendings: self
                .open
                .values()
                .map(|pending| PendingResolution {
                    opened_ms: i64::try_from(pending.waited_ms(now_ms)).unwrap_or(i64::MAX),
                    ..pending.clone()
                })
                .collect(),
        }
    }

    /// A book from a saved record, holding at most `maximum`, with every wait put back against
    /// `now_ms`. A record that is newer than this engine, impossible or too large for `maximum`
    /// is refused.
    pub fn from_record(
        record: PendingRecord,
        now_ms: u64,
        maximum: usize,
    ) -> Result<Self, PendingLoadError> {
        if record.schema_version != PENDING_SCHEMA_VERSION {
            return Err(PendingLoadError::UnsupportedVersion {
                found: record.schema_version,
                expected: PENDING_SCHEMA_VERSION,
            });
        }
        if record.pendings.len() > maximum {
            return Err(PendingLoadError::Malformed(
                MalformedPending::TooManyForMaximum,
            ));
        }
        let mut book = Self::holding_at_most(maximum);
        book.next_id = record.next_id;
        for mut pending in record.pendings {
            pending.restored_draws()?;
            if pending.id >= record.next_id {
                return Err(PendingLoadError::Malformed(
                    MalformedPending::IdNotBelowNext,
                ));
            }
            if book.by_operation.contains_key(&pending.opened_by_operation) {
                return Err(PendingLoadError::Malformed(
                    MalformedPending::DuplicateOperation,
                ));
            }
            if book.open.contains_key(&pending.id) {
                return Err(PendingLoadError::Malformed(MalformedPending::DuplicateId));
            }
            pending.opened_ms = i64::try_from(now_ms)
                .unwrap_or(i64::MAX)
                .saturating_sub(pending.opened_ms);
            book.index_in(&pending);
            book.open.insert(pending.id, pending);
        }
        Ok(book)
    }

    /// The digest worked out again from the pendings, to prove the kept one is right.
    #[cfg(test)]
    pub(crate) fn recomputed_digest(&self) -> u64 {
        digest::walked(self.open.len());
        let folded = self
            .open
            .values()
            .fold(0, |folded, pending| folded ^ pending.logical_hash());
        folded ^ mix(self.next_id) ^ mix(self.maximum as u64)
    }
}
