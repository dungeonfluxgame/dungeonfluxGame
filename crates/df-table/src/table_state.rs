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

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::Arc;

use crate::effects::{BeatName, EffectError, TableEffect};
use crate::event::TableEvent;
use crate::identity::{Correlation, OperationId, SceneId, SeatId, TableScope};
use crate::owner_queue::{MAXIMUM_REMEMBERED_OPERATIONS, MAXIMUM_SEATS};

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

/// The operation ids already applied, newest last; the oldest is forgotten first once `maximum`
/// newer ones were applied, after which an old id is new again (the owner queue's rule).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedOperations {
    order: VecDeque<OperationId>,
    members: BTreeSet<OperationId>,
    maximum: usize,
}

impl AppliedOperations {
    /// The order the operations were applied in and how many are remembered; the membership set
    /// is derived from the order.
    fn digest_into(&self, digest: &mut Digest) {
        let Self {
            order,
            members: _,
            maximum,
        } = self;
        digest.count(*maximum);
        digest.count(order.len());
        for operation in order {
            digest.bytes(operation.as_bytes());
        }
    }

    fn remember(&mut self, operation: OperationId) {
        if self.maximum == 0 || !self.members.insert(operation) {
            return;
        }
        self.order.push_back(operation);
        while self.order.len() > self.maximum {
            if let Some(oldest) = self.order.pop_front() {
                self.members.remove(&oldest);
            }
        }
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
}

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
        let mut facts = Facts::empty(seat_capacity);
        facts.seats = Arc::new(taken);
        Ok(Self {
            facts: Arc::new(facts),
            applied: AppliedOperations {
                order: VecDeque::new(),
                members: BTreeSet::new(),
                maximum: MAXIMUM_REMEMBERED_OPERATIONS,
            },
        })
    }

    /// Remembers at most `maximum` applied operation ids; zero remembers none.
    pub fn remembering_operations(mut self, maximum: usize) -> Self {
        self.applied.maximum = maximum;
        self
    }

    /// A table whose party may split across scenes (RES25, question 60). Tables start with the
    /// split declined, which is the first release's rule.
    pub fn with_split_policy(mut self, policy: SplitPolicy) -> Self {
        Arc::make_mut(&mut self.facts).split_policy = policy;
        self
    }

    /// A deterministic digest of every fact and of the applied-operation memory: equal tables give
    /// equal digests in every run, and any commit that changes the world or the memory changes it
    /// (#1682). The walk reads no clock, so golden sessions and replay may compare it.
    pub fn digest(&self) -> u64 {
        let mut digest = Digest::new();
        self.facts.digest_into(&mut digest);
        self.applied.digest_into(&mut digest);
        digest.finish()
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

    /// Applies one commit's effects, all or nothing. A commit whose operation id was applied
    /// before changes nothing and says so. A refused effect leaves the table as it was. An applied
    /// commit, even an empty one, moves the revision stamp to its own.
    pub fn apply(
        &mut self,
        scope: &TableScope,
        correlation: Correlation,
        effects: &[TableEffect],
    ) -> Result<ApplyOutcome, EffectError> {
        if self.applied.members.contains(&correlation.operation_id) {
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
        self.applied.remember(correlation.operation_id);
        Ok(ApplyOutcome::Applied(events))
    }
}
