//! The shared language of change: `TableEffect`, its applier and the minimal table state.
//!
//! It owns `TableEffect`, a versioned closed enum that machines and the engine use to change facts
//! the whole table shares (RES18, RES32, O.5), `TableEffectKind` (its payload-free list, which
//! carries the version each effect was introduced in), `EffectRecord` (an effect as saved, refused
//! when it is newer than this engine, H.12), and `TableState`, the minimal table state that holds
//! those facts until `table_state` (C6) exists: the seats, the active scene and its beat, and the
//! last hand-off. `TableState::apply` is the one applier: all or nothing, one `EffectApplied`
//! event per effect, and idempotent per operation id, so replaying a committed operation changes
//! nothing (coding-style section 8, pattern 5).
//!
//! Adding an effect is a new variant of `TableEffect` and `TableEffectKind` and an arm in `kind`,
//! `introduced_in`, `key` and `apply_one`; each is an exhaustive `match`, so a half-added effect
//! does not compile. Effects are added and never changed (Q44). It never decides whether a machine
//! should propose an effect, and it holds no machine state: a machine's own state lives in its slot.

use std::collections::{BTreeSet, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::element::name_type;
use crate::event::TableEvent;
use crate::identity::{Correlation, MachineId, OperationId, SceneId, SeatId, TableScope};
use crate::listed::listed_enum;
use crate::owner_queue::{MAXIMUM_REMEMBERED_OPERATIONS, MAXIMUM_SEATS};

name_type!(
    /// The id of an authored beat of a scene, such as `locked_gate`.
    BeatName
);

/// The newest effect schema this engine writes and reads. Raised when an effect is added.
pub const TABLE_EFFECT_SCHEMA_VERSION: u32 = 1;

listed_enum! {
    /// Every effect without its payload: the list tests walk and events name.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum TableEffectKind {
        SeatJoined,
        SeatLeft,
        SceneEntered,
        BeatResolved,
        HandOffRecorded,
    }
}

impl TableEffectKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::SeatJoined => "seat_joined",
            Self::SeatLeft => "seat_left",
            Self::SceneEntered => "scene_entered",
            Self::BeatResolved => "beat_resolved",
            Self::HandOffRecorded => "hand_off_recorded",
        }
    }

    /// The schema version that first had this effect. An effect never changes after that.
    pub fn introduced_in(self) -> u32 {
        match self {
            Self::SeatJoined
            | Self::SeatLeft
            | Self::SceneEntered
            | Self::BeatResolved
            | Self::HandOffRecorded => 1,
        }
    }
}

/// A change to the facts the table shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case", deny_unknown_fields)]
pub enum TableEffect {
    SeatJoined {
        seat: SeatId,
    },
    SeatLeft {
        seat: SeatId,
    },
    /// A scene became the table's active scene; its beat starts empty.
    SceneEntered {
        scene: SceneId,
    },
    /// The active scene's authored beat was played through.
    BeatResolved {
        scene: SceneId,
        beat: BeatName,
    },
    /// The engine moved the table from one machine to another, carrying `carried` items.
    HandOffRecorded {
        from: MachineId,
        to: MachineId,
        carried: usize,
    },
}

impl TableEffect {
    pub fn kind(&self) -> TableEffectKind {
        match self {
            Self::SeatJoined { .. } => TableEffectKind::SeatJoined,
            Self::SeatLeft { .. } => TableEffectKind::SeatLeft,
            Self::SceneEntered { .. } => TableEffectKind::SceneEntered,
            Self::BeatResolved { .. } => TableEffectKind::BeatResolved,
            Self::HandOffRecorded { .. } => TableEffectKind::HandOffRecorded,
        }
    }
}

/// An effect as saved: stamped with the schema version it was written under (H.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectRecord {
    schema_version: u32,
    effect: TableEffect,
}

/// A saved effect this engine cannot use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordRefused {
    pub found: u32,
    pub expected: u32,
}

impl fmt::Display for RecordRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "an effect record has schema version {}, this engine reads up to {}",
            self.found, self.expected
        )
    }
}

impl std::error::Error for RecordRefused {}

impl EffectRecord {
    /// The effect stamped with the current schema version.
    pub fn stamp(effect: TableEffect) -> Self {
        Self {
            schema_version: TABLE_EFFECT_SCHEMA_VERSION,
            effect,
        }
    }

    /// The effect, unless the record is newer than this engine or claims a version older than the
    /// one that introduced the effect.
    pub fn open(self) -> Result<TableEffect, RecordRefused> {
        let introduced = self.effect.kind().introduced_in();
        if self.schema_version > TABLE_EFFECT_SCHEMA_VERSION || self.schema_version < introduced {
            return Err(RecordRefused {
                found: self.schema_version,
                expected: TABLE_EFFECT_SCHEMA_VERSION,
            });
        }
        Ok(self.effect)
    }
}

listed_enum! {
    /// Why an effect was not applied.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum EffectRefusal {
        SeatAlreadySeated,
        SeatNotSeated,
        TableFull,
        /// The effect names a scene that is not the table's active scene.
        SceneNotActive,
    }
}

impl EffectRefusal {
    pub fn key(self) -> &'static str {
        match self {
            Self::SeatAlreadySeated => "seat_already_seated",
            Self::SeatNotSeated => "seat_not_seated",
            Self::TableFull => "table_full",
            Self::SceneNotActive => "scene_not_active",
        }
    }
}

/// An effect of a commit was refused, so none of the commit was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectError {
    pub effect: TableEffectKind,
    pub cause: EffectRefusal,
}

impl EffectError {
    /// The class A event that records the refusal, once, where the commit handles it.
    pub fn event(&self, scope: &TableScope, correlation: Correlation) -> TableEvent {
        TableEvent::EffectRefused {
            scope: scope.clone(),
            correlation,
            effect: self.effect,
            cause: self.cause,
        }
    }
}

impl fmt::Display for EffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the effect {} was refused: {}",
            self.effect.key(),
            self.cause.key()
        )
    }
}

impl std::error::Error for EffectError {}

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

/// What a hand-off recorded: where the table came from, where it went and how much it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandOffRecord {
    pub from: MachineId,
    pub to: MachineId,
    pub carried: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Facts {
    seat_capacity: usize,
    seats: BTreeSet<SeatId>,
    active_scene: Option<SceneId>,
    active_beat: Option<BeatName>,
    last_hand_off: Option<HandOffRecord>,
}

/// The operation ids already applied, newest last; the oldest is forgotten first once `maximum`
/// newer ones were applied, after which an old id is new again (the owner queue's rule).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedOperations {
    order: VecDeque<OperationId>,
    members: BTreeSet<OperationId>,
    maximum: usize,
}

impl AppliedOperations {
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

/// The facts the whole table shares, minimal until `table_state` (C6) replaces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableState {
    facts: Facts,
    applied: AppliedOperations,
}

impl TableState {
    /// An empty table with room for `seat_capacity` seats.
    pub fn new(seat_capacity: usize) -> Result<Self, TableStateError> {
        Self::seated(seat_capacity, &[])
    }

    /// A table with room for `seat_capacity` seats and these seats already taken.
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
        let mut taken = BTreeSet::new();
        for seat in seats {
            if !taken.insert(*seat) {
                return Err(TableStateError::DuplicateSeat(*seat));
            }
        }
        Ok(Self {
            facts: Facts {
                seat_capacity,
                seats: taken,
                active_scene: None,
                active_beat: None,
                last_hand_off: None,
            },
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

    pub fn seat_capacity(&self) -> usize {
        self.facts.seat_capacity
    }

    pub fn seats(&self) -> &BTreeSet<SeatId> {
        &self.facts.seats
    }

    pub fn active_scene(&self) -> Option<SceneId> {
        self.facts.active_scene
    }

    pub fn active_beat(&self) -> Option<&BeatName> {
        self.facts.active_beat.as_ref()
    }

    pub fn last_hand_off(&self) -> Option<&HandOffRecord> {
        self.facts.last_hand_off.as_ref()
    }

    /// Whether `other` has the same seats, scene and beat: the facts a machine's entry and exit
    /// must leave as they found them. A hand-off record is the one fact an exit may change.
    pub fn same_world_as(&self, other: &Self) -> bool {
        self.facts.seat_capacity == other.facts.seat_capacity
            && self.facts.seats == other.facts.seats
            && self.facts.active_scene == other.facts.active_scene
            && self.facts.active_beat == other.facts.active_beat
    }

    /// Applies one commit's effects, all or nothing. A commit whose operation id was applied
    /// before changes nothing and says so. A refused effect leaves the table as it was.
    pub fn apply(
        &mut self,
        scope: &TableScope,
        correlation: Correlation,
        effects: &[TableEffect],
    ) -> Result<ApplyOutcome, EffectError> {
        if self.applied.members.contains(&correlation.operation_id) {
            return Ok(ApplyOutcome::AlreadyApplied);
        }
        let mut next = self.facts.clone();
        let mut events = Vec::with_capacity(effects.len());
        for effect in effects {
            next.apply_one(effect).map_err(|cause| EffectError {
                effect: effect.kind(),
                cause,
            })?;
            events.push(TableEvent::EffectApplied {
                scope: scope.clone(),
                correlation,
                effect: effect.kind(),
            });
        }
        self.facts = next;
        self.applied.remember(correlation.operation_id);
        Ok(ApplyOutcome::Applied(events))
    }
}

impl Facts {
    fn apply_one(&mut self, effect: &TableEffect) -> Result<(), EffectRefusal> {
        match effect {
            TableEffect::SeatJoined { seat } => {
                if self.seats.contains(seat) {
                    return Err(EffectRefusal::SeatAlreadySeated);
                }
                if self.seats.len() >= self.seat_capacity {
                    return Err(EffectRefusal::TableFull);
                }
                self.seats.insert(*seat);
            }
            TableEffect::SeatLeft { seat } => {
                if !self.seats.remove(seat) {
                    return Err(EffectRefusal::SeatNotSeated);
                }
            }
            TableEffect::SceneEntered { scene } => {
                self.active_scene = Some(*scene);
                self.active_beat = None;
            }
            TableEffect::BeatResolved { scene, beat } => {
                if self.active_scene != Some(*scene) {
                    return Err(EffectRefusal::SceneNotActive);
                }
                self.active_beat = Some(beat.clone());
            }
            TableEffect::HandOffRecorded { from, to, carried } => {
                self.last_hand_off = Some(HandOffRecord {
                    from: from.clone(),
                    to: to.clone(),
                    carried: *carried,
                });
            }
        }
        Ok(())
    }
}
