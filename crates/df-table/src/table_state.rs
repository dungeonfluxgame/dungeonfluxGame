//! The composition view of the owners' state: seats, scenes, cast, time, status and the stack.
//!
//! It owns `TableState`, the facts the whole table shares and the one applier that changes them
//! (D.1 to D.7, RES25, RES42), and the reads and rules around it. The facts are the commit
//! revision, the world time and table status (active, idle or paused), `scenes[]` each with its
//! own generation (`scene_epoch` of D.2, named `scene_generation` in code by RES42), beat, active
//! machine and hand-off stack, the seats with their presence and scene, and the cast of heroes and
//! creatures. Knowledge facts stay with the knowledge owner and reach projection as
//! audience-tagged attachments, so private text never sits in a table effect.
//!
//! Every change goes through `TableState::apply` with a list of `TableEffect`s, all or nothing,
//! one `EffectApplied` event per effect, and idempotent per operation id, so replaying a
//! committed operation changes nothing (coding-style section 8, pattern 5). `snapshot` hands
//! out an immutable `TableSnapshot` for projection in one pointer copy: a commit builds new facts
//! that share every collection it did not touch, so a snapshot never sees a later commit.
//!
//! The questions the table answers without changing anything are on the snapshot and live in
//! `judge`: the split-party decline, the play-input and idle rules, and stale-result rejection by
//! scene generation. It never reads a clock or a die (time arrives as an argument), never owns a
//! pending resolution (the pending handler does) and never decides whether a machine should propose
//! an effect. It fails with a typed `EffectError`, and a failed commit changes nothing.
//!
//! `effects` re-exports `TableState`, `TableStateError`, `ApplyOutcome` and `HandOffRecord`, the
//! names the machines import, so moving the applier here changed no caller.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::effects::{BeatName, EffectError, TableEffect};
use crate::envelope::Sender;
use crate::event::TableEvent;
use crate::identity::{Correlation, OperationId, SceneId, SeatId, SessionRevision, TableScope};
use crate::owner_queue::{MAXIMUM_REMEMBERED_OPERATIONS, MAXIMUM_SEATS};
use crate::pending::bounded::BoundedMap;
use crate::pending::digest::{Fnv, mix};

mod apply;
pub mod bounds;
mod cast;
mod digest;
mod events;
mod facts;
mod judge;
mod lanes;
mod scene;
mod seat;
mod snapshot;
mod status;

pub use cast::{
    CastChange, CastEntry, CastKind, ConditionName, Controller, DisplayName, DisplayNameError,
    HeroColor, HeroColorError, MediaName, MediaNameError, Placement, PlacementError, Visibility,
    Vitals, VitalsError,
};
pub use events::TableStateEvent;
pub use judge::{
    IdleClock, IdleVerdict, InFlight, PlayInput, ResultStamp, StaleCause, StaleResult,
    split_party_decline,
};
pub use lanes::{LaneChange, reconcile_lanes};
pub use scene::{
    ActiveMachine, HandOffFrame, HandOffRecord, MachineHome, SceneGeneration, SceneRecord,
};
pub use seat::{Presence, SeatState, Seats};
pub use snapshot::TableSnapshot;
pub use status::{SplitPolicy, TableStatus};

use digest::Digest;
use facts::Facts;

/// Why a table state could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableStateError {
    ZeroCapacity,
    CapacityTooLarge,
    TooManySeatsForCapacity,
    DuplicateSeat(SeatId),
}

impl fmt::Display for TableStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => formatter.write_str("a table has at least one seat"),
            Self::CapacityTooLarge => {
                write!(formatter, "a table has at most {MAXIMUM_SEATS} seats")
            }
            Self::TooManySeatsForCapacity => {
                formatter.write_str("more seats were given than the table holds")
            }
            Self::DuplicateSeat(seat) => write!(formatter, "seat {seat} was given twice"),
        }
    }
}

impl std::error::Error for TableStateError {}

/// What an applied operation is remembered under: its sender, the membership of that sender (zero
/// for the engine; a seat's membership changes each time it joins) and the id the sender gave it.
/// An id means something only together with its sender (the owner queue's rule), so one seat's id
/// never stands in for another's, and a seat that left and came back, or another player who took
/// its seat id, meets none of the history of the one before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct AppliedKey {
    sender: Sender,
    membership: u64,
    operation: OperationId,
}

/// The operations already applied, newest last; the oldest is forgotten first once `maximum`
/// newer ones were applied, after which an old id is new again (the owner queue's rule). Each is
/// remembered per sender and per membership of that sender (`AppliedKey`). It also holds the
/// membership each seated seat joined under, which is what makes that stamp.
///
/// It is a `BoundedMap`, so a copy of the table shares it (a checkpoint costs a few pointer copies,
/// not one copy of up to a million ids) and a lookup is one bucket access. Its digest is the
/// exclusive or of the hash of each remembered key, kept up to date where a key enters or leaves,
/// so asking for the table's digest costs the same however many ids are remembered. A commit's own
/// id is remembered here and a host's commit stamp never is (`TableState::stamp_commit`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedOperations {
    members: BoundedMap<AppliedKey, AppliedKey>,
    digest: u64,
    /// The membership of each seated seat: the number its join was given. A seat that left has
    /// none, so an input it sends is stamped zero, which no seated seat holds.
    memberships: BTreeMap<SeatId, u64>,
    /// The membership the next join is given; it only grows, so a rejoined seat never repeats one.
    next_membership: u64,
}

impl AppliedOperations {
    fn remembering(maximum: usize) -> Self {
        Self {
            members: BoundedMap::holding_at_most(maximum),
            digest: 0,
            memberships: BTreeMap::new(),
            next_membership: 0,
        }
    }

    /// The same memberships and a new, empty memory of at most `maximum` operations.
    fn forgetting_operations(self, maximum: usize) -> Self {
        Self {
            memberships: self.memberships,
            next_membership: self.next_membership,
            ..Self::remembering(maximum)
        }
    }

    fn hash_of(key: &AppliedKey) -> u64 {
        let mut hasher = Fnv::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    /// The bound, the digest of the remembered keys (which does not depend on the order they were
    /// applied in) and the memberships, in seat order.
    fn digest_into(&self, digest: &mut Digest) {
        digest.count(self.members.maximum());
        digest.count(self.members.len());
        digest.number(self.digest ^ mix(self.members.maximum() as u64));
        digest.number(self.next_membership);
        digest.count(self.memberships.len());
        for (seat, membership) in &self.memberships {
            digest.number(seat.get());
            digest.number(*membership);
        }
    }

    fn membership_of(&self, sender: Sender) -> u64 {
        match sender {
            Sender::Engine => 0,
            Sender::Seat(seat) => self.memberships.get(&seat).copied().unwrap_or(0),
        }
    }

    fn key_of(&self, sender: Sender, operation: OperationId) -> AppliedKey {
        AppliedKey {
            sender,
            membership: self.membership_of(sender),
            operation,
        }
    }

    fn contains(&self, key: &AppliedKey) -> bool {
        self.members.contains(key)
    }

    fn remember(&mut self, key: AppliedKey) {
        if self.members.contains(&key) {
            return;
        }
        let removed = self.members.insert(key, key);
        self.digest ^= Self::hash_of(&key);
        // A replaced or evicted key leaves the digest again; with a bound of zero the key just
        // given is the one evicted, so it enters and leaves and the digest is unchanged.
        for old in removed.replaced.iter().chain(removed.evicted.iter()) {
            self.digest ^= Self::hash_of(old);
        }
    }

    /// Gives a seat a membership of its own: a join starts a new one.
    fn seat_joined(&mut self, seat: SeatId) {
        self.next_membership = self.next_membership.saturating_add(1);
        self.memberships.insert(seat, self.next_membership);
    }

    fn seat_left(&mut self, seat: SeatId) {
        self.memberships.remove(&seat);
    }
}

/// What a commit did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Every effect was applied; one `EffectApplied` event per effect, in order.
    Applied(Vec<TableEvent>),
    /// The operation id was applied before, so nothing changed.
    AlreadyApplied,
}

/// The facts the whole table shares, and the one applier that changes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableState {
    facts: Arc<Facts>,
    applied: AppliedOperations,
    memo: DigestMemo,
}

/// The last digest worked out, kept until the table changes, so asking twice costs one walk. It
/// is a cache and never part of what two tables are compared by. A digest of zero is worked out
/// again, which costs a walk and is never wrong.
#[derive(Debug, Default)]
pub(crate) struct DigestMemo(AtomicU64);

impl DigestMemo {
    pub(crate) fn empty() -> Self {
        Self(AtomicU64::new(0))
    }

    pub(crate) fn get(&self) -> Option<u64> {
        // Relaxed: a cache of a pure function of the table, read and written by whoever holds it.
        match self.0.load(Ordering::Relaxed) {
            0 => None,
            known => Some(known),
        }
    }

    pub(crate) fn set(&self, digest: u64) {
        self.0.store(digest, Ordering::Relaxed);
    }

    pub(crate) fn forget(&self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

impl Clone for DigestMemo {
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.0.load(Ordering::Relaxed)))
    }
}

impl PartialEq for DigestMemo {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for DigestMemo {}

impl TableState {
    /// An empty table with room for `seat_capacity` seats.
    pub fn new(seat_capacity: usize) -> Result<Self, TableStateError> {
        Self::seated(seat_capacity, &[])
    }

    /// A table with room for `seat_capacity` seats and these seats already taken, all connected
    /// and in no scene yet.
    pub fn seated(seat_capacity: usize, seats: &[SeatId]) -> Result<Self, TableStateError> {
        if seat_capacity == 0 {
            return Err(TableStateError::ZeroCapacity);
        }
        if seat_capacity > MAXIMUM_SEATS {
            return Err(TableStateError::CapacityTooLarge);
        }
        if seats.len() > seat_capacity {
            return Err(TableStateError::TooManySeatsForCapacity);
        }
        let mut taken = BTreeMap::new();
        for seat in seats {
            let state = SeatState {
                presence: Presence::Connected,
                scene: None,
                hero: None,
            };
            if taken.insert(*seat, state).is_some() {
                return Err(TableStateError::DuplicateSeat(*seat));
            }
        }
        let mut applied = AppliedOperations::remembering(MAXIMUM_REMEMBERED_OPERATIONS);
        for seat in taken.keys() {
            applied.seat_joined(*seat);
        }
        let mut facts = Facts::empty(seat_capacity);
        facts.seats = Arc::new(taken);
        Ok(Self {
            facts: Arc::new(facts),
            applied,
            memo: DigestMemo::empty(),
        })
    }

    /// Remembers at most `maximum` applied operations; zero remembers none. The seats keep the
    /// memberships they were seated with.
    pub fn remembering_operations(mut self, maximum: usize) -> Self {
        self.applied = self.applied.forgetting_operations(maximum);
        self.memo.forget();
        self
    }

    /// A table whose party may split across scenes (RES25, question 60). Tables start with the
    /// split declined, which is the first release's rule.
    pub fn with_split_policy(mut self, policy: SplitPolicy) -> Self {
        Arc::make_mut(&mut self.facts).split_policy = policy;
        self.memo.forget();
        self
    }

    /// A deterministic digest of every fact and of the applied-operation memory: equal tables give
    /// equal digests in every run, and any commit that changes the world or the memory changes it
    /// (#1682). The walk reads no clock, so golden sessions and replay may compare it.
    pub fn digest(&self) -> u64 {
        if let Some(known) = self.memo.get() {
            return known;
        }
        let mut digest = Digest::new();
        self.facts.digest_into(&mut digest);
        self.applied.digest_into(&mut digest);
        let worked_out = digest.finish();
        self.memo.set(worked_out);
        worked_out
    }

    /// The digest without the commit stamps: the facts as `TableSnapshot::digest_without_stamps`
    /// reads them and the memory of applied operations, whose digest does not depend on the order
    /// the operations arrived in. Two tables that applied the same operations in one batch or in
    /// several are equal here, whatever commits the host stamped between them (S1).
    pub fn digest_without_stamps(&self) -> u64 {
        let mut digest = Digest::new();
        self.facts.digest_without_stamps_into(&mut digest);
        self.applied.digest_into(&mut digest);
        digest.finish()
    }

    /// Whether `sender`, under the membership it holds now, has an operation with this id
    /// remembered as applied. Another sender's id, and the id of a seat's earlier membership, is
    /// not found: an id means something only together with its sender.
    pub fn has_applied(&self, sender: Sender, operation: &OperationId) -> bool {
        self.applied
            .contains(&self.applied.key_of(sender, *operation))
    }

    /// The membership of a sender: zero for the engine, the number a seat's last join was given
    /// for a seated seat (it changes each time the seat joins, as the owner queue's does), and
    /// zero for a seat that is not at the table.
    pub fn membership_of(&self, sender: Sender) -> u64 {
        self.applied.membership_of(sender)
    }

    /// Moves the revision to the one the host stamped for its commit, if it is later. A commit
    /// stamp is not an operation: its id is not remembered, so commit ids never use up the memory
    /// of real operations and the digest does not depend on how many commits a run took.
    pub fn stamp_commit(&mut self, revision: SessionRevision) {
        if revision > self.facts.revision {
            Arc::make_mut(&mut self.facts).revision = revision;
            self.memo.forget();
        }
    }

    /// The table as it is now, immutable and cheap to hold (one pointer copy).
    pub fn snapshot(&self) -> TableSnapshot {
        TableSnapshot::of(&self.facts)
    }

    pub fn seat_capacity(&self) -> usize {
        self.facts.seat_capacity
    }

    pub fn seats(&self) -> Seats<'_> {
        Seats {
            states: &self.facts.seats,
        }
    }

    /// The scene the shared screen shows: the focus scene of the snapshot.
    pub fn active_scene(&self) -> Option<SceneId> {
        self.facts.focus
    }

    /// The beat last played through in the active scene.
    pub fn active_beat(&self) -> Option<&BeatName> {
        self.facts
            .focus
            .and_then(|scene| self.facts.scenes.get(&scene))
            .and_then(SceneRecord::beat)
    }

    pub fn last_hand_off(&self) -> Option<&HandOffRecord> {
        self.facts.last_hand_off.as_ref()
    }

    /// Whether `other` has the same world: the seats and their presence and scene, the scenes,
    /// the cast, the focus, the time and the status. A machine's entry and exit must leave these as
    /// they found them; the last hand-off, the revision stamp and the pause history are
    /// bookkeeping an exit may change.
    pub fn same_world_as(&self, other: &Self) -> bool {
        self.facts.same_world_as(&other.facts)
    }

    /// Applies the engine's own commit (the host's, a lobby's), as `apply_from` the engine.
    pub fn apply(
        &mut self,
        scope: &TableScope,
        correlation: Correlation,
        effects: &[TableEffect],
    ) -> Result<ApplyOutcome, EffectError> {
        self.apply_from(Sender::Engine, scope, correlation, effects)
    }

    /// Applies one commit's effects, all or nothing. A commit whose operation id `sender` applied
    /// before, under the membership it holds now, changes nothing and says so; the same id from
    /// another sender is a different operation. A refused effect leaves the table as it was. An
    /// applied commit, even an empty one, moves the revision stamp to its own.
    pub fn apply_from(
        &mut self,
        sender: Sender,
        scope: &TableScope,
        correlation: Correlation,
        effects: &[TableEffect],
    ) -> Result<ApplyOutcome, EffectError> {
        let key = self.applied.key_of(sender, correlation.operation_id);
        if self.applied.contains(&key) {
            return Ok(ApplyOutcome::AlreadyApplied);
        }
        let mut next = (*self.facts).clone();
        let mut events = Vec::with_capacity(effects.len());
        for effect in effects {
            let scenes_before = Arc::clone(&next.scenes);
            next.apply_one(effect, correlation.revision)
                .map_err(|cause| EffectError {
                    effect: effect.kind(),
                    cause,
                })?;
            events.push(TableEvent::EffectApplied {
                scope: scope.clone(),
                correlation,
                effect: effect.kind(),
            });
            for (scene, record) in scenes_before.iter() {
                let kept = next.scenes.get(scene).map(SceneRecord::generation);
                if kept != Some(record.generation()) {
                    events.push(TableEvent::TableState(TableStateEvent::SceneClosed {
                        scope: scope.clone(),
                        correlation,
                        scene: *scene,
                        generation: record.generation().get(),
                        machine: record.active_machine().cloned(),
                        stack_depth: record.hand_off_stack().len(),
                    }));
                }
            }
        }
        next.revision = next.revision.max(correlation.revision);
        self.facts = Arc::new(next);
        self.applied.remember(key);
        for effect in effects {
            match effect {
                TableEffect::SeatJoined { seat } => self.applied.seat_joined(*seat),
                TableEffect::SeatLeft { seat } => self.applied.seat_left(*seat),
                _ => {}
            }
        }
        self.memo.forget();
        Ok(ApplyOutcome::Applied(events))
    }
}
