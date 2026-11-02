//! What the table knows about each seat: whether it is there, and where in the story it stands.
//!
//! It owns `Presence` (connected, sleeping, disconnected or voluntarily away, as `remote-play.md`
//! names them: distinct from membership, which is whether a seat is in the table at all),
//! `SeatState` (a seat's presence and scene) and `Seats`, the read view of the seated seats that
//! the machines use (`contains`, `len`, `iter`). It never decides what a presence means for a rule:
//! a voluntary-AFK character never acting alone in risk (D.4) is the machines' concern.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_map::Keys;

use serde::{Deserialize, Serialize};

use crate::element::ElementId;
use crate::identity::{SceneId, SeatId};
use crate::listed::listed_enum;
use crate::table_state::digest::Digest;

listed_enum! {
    /// Whether a seat's player is there (D.4). It is a deliberate fact: a sleeping or
    /// away-from-keyboard seat is never punished for it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Presence {
        Connected,
        Sleeping,
        Disconnected,
        /// The player chose to step away; the host decides whether the hero follows or waits.
        VoluntaryAfk,
    }
}

impl Presence {
    pub fn key(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Sleeping => "sleeping",
            Self::Disconnected => "disconnected",
            Self::VoluntaryAfk => "voluntary_afk",
        }
    }
}

/// One seat at the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatState {
    pub(super) presence: Presence,
    pub(super) scene: Option<SceneId>,
    pub(super) hero: Option<ElementId>,
}

impl SeatState {
    pub fn presence(&self) -> Presence {
        self.presence
    }

    /// The scene the seat is in: `None` until the first scene starts (D.1: a seat belongs to one
    /// scene).
    pub fn scene(&self) -> Option<SceneId> {
        self.scene
    }

    /// The seat's one hero, the party entry it plays (D.4: one active character per seat).
    pub fn hero(&self) -> Option<&ElementId> {
        self.hero.as_ref()
    }

    pub(super) fn digest_into(&self, digest: &mut Digest) {
        let Self {
            presence,
            scene,
            hero,
        } = self;
        digest.text(presence.key());
        digest.option(scene, |digest, scene| digest.number(scene.get()));
        digest.option(hero, |digest, hero| digest.text(hero.as_str()));
    }
}

/// A read view of the seats at the table, in seat order.
#[derive(Debug, Clone, Copy)]
pub struct Seats<'a> {
    pub(super) states: &'a BTreeMap<SeatId, SeatState>,
}

impl<'a> Seats<'a> {
    pub fn contains(&self, seat: &SeatId) -> bool {
        self.states.contains_key(seat)
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    pub fn iter(&self) -> Keys<'a, SeatId, SeatState> {
        self.states.keys()
    }

    /// The seats as an owned set.
    pub fn to_set(&self) -> BTreeSet<SeatId> {
        self.states.keys().copied().collect()
    }
}

impl<'a> IntoIterator for Seats<'a> {
    type Item = &'a SeatId;
    type IntoIter = Keys<'a, SeatId, SeatState>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
