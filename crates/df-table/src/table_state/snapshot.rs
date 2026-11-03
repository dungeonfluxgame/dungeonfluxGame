//! An immutable snapshot of the table, which projection and the rules read.
//!
//! It owns `TableSnapshot`, a handle on the facts as they were at one commit. Taking one is a
//! pointer copy, and a later commit never changes it: a commit builds new facts and shares what it
//! did not touch (`facts.rs`), so a view call built from a snapshot never sees a half-applied
//! commit and never blocks the owner (RES30, H.14). The reads here are the whole question set of
//! the table's composition view: seats, scenes, cast, time and status. It never changes anything.

use std::sync::Arc;

use crate::element::ElementId;
use crate::identity::{SceneId, SeatId, SessionRevision};
use crate::table_state::cast::{CastEntry, CastKind};
use crate::table_state::digest::Digest;
use crate::table_state::facts::Facts;
use crate::table_state::scene::{ActiveMachine, HandOffRecord, SceneRecord};
use crate::table_state::seat::{SeatState, Seats};
use crate::table_state::status::{SplitPolicy, TableStatus};

/// The table as it was at one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSnapshot {
    pub(super) facts: Arc<Facts>,
}

impl TableSnapshot {
    pub(super) fn of(facts: &Arc<Facts>) -> Self {
        Self {
            facts: Arc::clone(facts),
        }
    }

    /// The digest of the facts alone, as `TableState::digest` without the operation memory.
    pub fn digest(&self) -> u64 {
        let mut digest = Digest::new();
        self.facts.digest_into(&mut digest);
        digest.finish()
    }

    /// The digest of the facts without the commit stamps (the revision and the revision a pause
    /// was stamped at), for comparing tables that differ only in how commits were grouped.
    pub fn digest_without_stamps(&self) -> u64 {
        let mut digest = Digest::new();
        self.facts.digest_without_stamps_into(&mut digest);
        digest.finish()
    }

    pub fn seat_capacity(&self) -> usize {
        self.facts.seat_capacity
    }

    pub fn seats(&self) -> Seats<'_> {
        Seats {
            states: &self.facts.seats,
        }
    }

    pub fn seat(&self, seat: SeatId) -> Option<&SeatState> {
        self.facts.seats.get(&seat)
    }

    /// The open scenes, in scene order.
    pub fn scenes(&self) -> impl Iterator<Item = (SceneId, &SceneRecord)> {
        self.facts
            .scenes
            .iter()
            .map(|(scene, record)| (*scene, record))
    }

    pub fn scene(&self, scene: SceneId) -> Option<&SceneRecord> {
        self.facts.scenes.get(&scene)
    }

    /// The scene the shared screen shows: the one the party last entered, or the lowest open one
    /// when that closed. The K.10 rule for choosing among several scenes by dwell time is not
    /// built; it belongs with the tempo owner.
    pub fn focus_scene(&self) -> Option<SceneId> {
        self.facts.focus
    }

    pub fn world_time_s(&self) -> u64 {
        self.facts.world_time_s
    }

    pub fn status(&self) -> TableStatus {
        self.facts.status
    }

    pub fn split_policy(&self) -> SplitPolicy {
        self.facts.split_policy
    }

    /// The revision of the last commit applied; before the first it is the genesis revision
    /// (the first epoch, sequence zero), so a view always has a revision the wire accepts.
    pub fn revision(&self) -> SessionRevision {
        self.facts.revision
    }

    /// The table-scope machine (the lobby), which runs while no scene has a group machine.
    pub fn table_machine(&self) -> Option<&ActiveMachine> {
        self.facts.table_machine.as_ref()
    }

    /// The machine a view names: the focus scene's active machine, else the table machine.
    pub fn focus_machine(&self) -> Option<&ActiveMachine> {
        self.facts
            .focus
            .and_then(|scene| self.facts.scenes.get(&scene))
            .and_then(SceneRecord::active)
            .or(self.facts.table_machine.as_ref())
    }

    /// The revision of the commit that last paused the table, if one ever did.
    pub fn last_paused_at(&self) -> Option<SessionRevision> {
        self.facts.last_paused_at
    }

    pub fn last_hand_off(&self) -> Option<&HandOffRecord> {
        self.facts.last_hand_off.as_ref()
    }

    /// Every cast entry of every scene, in id order.
    pub fn cast(&self) -> impl Iterator<Item = &CastEntry> {
        self.facts.cast.values()
    }

    pub fn cast_len(&self) -> usize {
        self.facts.cast.len()
    }

    pub fn cast_entry(&self, id: &ElementId) -> Option<&CastEntry> {
        self.facts.cast.get(id)
    }

    /// The party entries (heroes) of every scene, in id order.
    pub fn heroes(&self) -> impl Iterator<Item = &CastEntry> {
        self.cast().filter(|entry| entry.kind() == CastKind::Party)
    }

    /// The hero a seat plays: its one party entry (D.4).
    pub fn hero_of(&self, seat: SeatId) -> Option<&CastEntry> {
        let id = self.facts.seats.get(&seat)?.hero()?;
        self.facts.cast.get(id)
    }

    /// Whether `other` is the same facts in memory, which tests use to prove a commit shared what
    /// it did not change.
    #[cfg(test)]
    pub(crate) fn shares_cast_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.facts.cast, &other.facts.cast)
    }

    #[cfg(test)]
    pub(crate) fn shares_scenes_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.facts.scenes, &other.facts.scenes)
    }

    #[cfg(test)]
    pub(crate) fn shares_seats_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.facts.seats, &other.facts.seats)
    }
}
