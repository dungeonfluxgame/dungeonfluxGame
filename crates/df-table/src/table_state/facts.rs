//! The facts of a table and the small changes that keep them consistent.
//!
//! It owns `Facts`, the one structure a commit changes and a snapshot shares. Its collections sit
//! behind `Arc`, so a copy of the facts shares every collection a commit did not touch and a
//! snapshot costs one pointer (O.7, RES30): the first write to a shared collection copies that
//! collection and no other. It also owns the invariants every effect leaves true:
//!
//! - a scene closes when its last seat leaves it, and a seat is in at most one scene;
//! - a party cast entry is controlled by a seated seat and stands in that seat's scene;
//! - the focus scene is open whenever any scene is.
//!
//! It never decides whether an effect may be applied: `apply` checks, then uses these changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::effects::EffectRefusal;
use crate::element::ElementId;
use crate::identity::{RecoveryEpoch, SceneId, SeatId, SessionRevision};
use crate::table_state::cast::{CastEntry, Controller};
use crate::table_state::scene::{ActiveMachine, HandOffRecord, SceneGeneration, SceneRecord};
use crate::table_state::seat::SeatState;
use crate::table_state::status::{SplitPolicy, TableStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Facts {
    pub(super) seat_capacity: usize,
    pub(super) seats: Arc<BTreeMap<SeatId, SeatState>>,
    pub(super) scenes: Arc<BTreeMap<SceneId, SceneRecord>>,
    pub(super) cast: Arc<BTreeMap<ElementId, CastEntry>>,
    pub(super) focus: Option<SceneId>,
    /// The generation issued last; zero before the first scene.
    pub(super) last_generation: u64,
    pub(super) world_time_s: u64,
    pub(super) status: TableStatus,
    pub(super) last_paused_at: Option<SessionRevision>,
    /// The revision of the last commit applied; before the first it is the genesis revision, which
    /// the wire accepts (sequence zero is a valid value).
    pub(super) revision: SessionRevision,
    /// The table-scope machine (the lobby) that runs while no scene has a group machine.
    pub(super) table_machine: Option<ActiveMachine>,
    pub(super) last_hand_off: Option<HandOffRecord>,
    pub(super) split_policy: SplitPolicy,
}

impl Facts {
    pub(super) fn empty(seat_capacity: usize) -> Self {
        Self {
            seat_capacity,
            seats: Arc::new(BTreeMap::new()),
            scenes: Arc::new(BTreeMap::new()),
            cast: Arc::new(BTreeMap::new()),
            focus: None,
            last_generation: 0,
            world_time_s: 0,
            status: TableStatus::Active,
            last_paused_at: None,
            revision: SessionRevision::new(RecoveryEpoch::FIRST, 0),
            table_machine: None,
            last_hand_off: None,
            split_policy: SplitPolicy::Declined,
        }
    }

    /// Whether `other` has the same world: everything except the bookkeeping of the last
    /// hand-off, the revision stamp, the pause history and the generation counter.
    pub(super) fn same_world_as(&self, other: &Self) -> bool {
        self.seat_capacity == other.seat_capacity
            && self.seats == other.seats
            && self.scenes == other.scenes
            && self.table_machine == other.table_machine
            && self.cast == other.cast
            && self.focus == other.focus
            && self.world_time_s == other.world_time_s
            && self.status == other.status
            && self.split_policy == other.split_policy
    }

    /// The next scene generation, one above every one issued before.
    pub(super) fn issue_generation(&mut self) -> Result<SceneGeneration, EffectRefusal> {
        let next = self
            .last_generation
            .checked_add(1)
            .ok_or(EffectRefusal::GenerationOverflow)?;
        self.last_generation = next;
        Ok(SceneGeneration::new(next))
    }

    /// Opens the scene with a fresh generation unless it is open already.
    pub(super) fn open_scene(&mut self, scene: SceneId) -> Result<(), EffectRefusal> {
        if self.scenes.contains_key(&scene) {
            return Ok(());
        }
        let generation = self.issue_generation()?;
        Arc::make_mut(&mut self.scenes).insert(scene, SceneRecord::opened(generation));
        Ok(())
    }

    /// Whether any seat other than `except` stands in `scene`.
    pub(super) fn has_other_seat_in(&self, scene: SceneId, except: Option<SeatId>) -> bool {
        self.seats
            .iter()
            .any(|(seat, state)| Some(*seat) != except && state.scene == Some(scene))
    }

    /// Closes the scene and drops its creatures that are not heroes, if no seat is in it.
    pub(super) fn close_scene_if_empty(&mut self, scene: SceneId) {
        if self.has_other_seat_in(scene, None) || !self.scenes.contains_key(&scene) {
            return;
        }
        Arc::make_mut(&mut self.scenes).remove(&scene);
        let in_scene = |entry: &CastEntry| entry.scene() == scene;
        if self.cast.values().any(in_scene) {
            Arc::make_mut(&mut self.cast).retain(|_, entry| !in_scene(entry));
        }
    }

    /// Keeps the focus on an open scene: the focus stays while it is open, and otherwise moves to
    /// the lowest open scene, or to none.
    pub(super) fn refocus(&mut self) {
        let still_open = self
            .focus
            .is_some_and(|scene| self.scenes.contains_key(&scene));
        if !still_open {
            self.focus = self.scenes.keys().next().copied();
        }
    }

    /// Whether any other seat is at the table: the company a seat would split from.
    pub(super) fn has_company(&self, seat: SeatId) -> bool {
        self.seats.keys().any(|other| *other != seat)
    }

    /// Whether `seat` standing in `scene` would leave some other seat in a different scene: the one
    /// rule for a split, which both the applier and `judge_split` ask (#1690).
    pub(super) fn parts_company(&self, seat: SeatId, scene: SceneId) -> bool {
        self.seats
            .iter()
            .any(|(other, state)| *other != seat && state.scene != Some(scene))
    }

    /// Every cast entry a seat controls: its hero and its mounts, companions, summons and
    /// hirelings.
    pub(super) fn controlled_by(&self, seat: SeatId) -> impl Iterator<Item = &ElementId> {
        self.cast.iter().filter_map(move |(id, entry)| {
            (entry.controller() == (Controller::Seat { seat })).then_some(id)
        })
    }
}
