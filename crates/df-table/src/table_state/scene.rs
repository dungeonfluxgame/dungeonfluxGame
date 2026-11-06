//! A scene: one place with one group of seats, its generation, its beat and its hand-off stack.
//!
//! It owns `SceneGeneration` (RES42's name for D.2's `scene_epoch`, kept apart from
//! `RecoveryEpoch`), `SceneRecord`, `HandOffFrame` (a machine's saved slot waiting on the stack)
//! and `HandOffRecord` (the last hand-off the table made). A generation is issued by the table
//! from one counter that only goes up, so a scene entered again is never confused with its
//! earlier self. It never issues a generation itself: `apply` does, in the commit that changes the
//! scene.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::effects::{BeatName, ConversationOutcome};
use crate::identity::{MachineId, SceneId, SessionRevision};
use crate::machine::slot::MachineSlot;
use crate::table_state::digest::Digest;

/// The scene's epoch of D.2: it rises in the same commit as a scene change, and every plate job,
/// resource request and result carries it, so a result for an earlier scene is set aside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SceneGeneration(u64);

impl SceneGeneration {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SceneGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Where an active machine lives: the table itself (the lobby, before any scene has a group
/// machine) or one scene (RES20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case", deny_unknown_fields)]
pub enum MachineHome {
    Table,
    Scene { scene: SceneId },
}

/// The machine that runs a scene, with its saved state: the one home of the active slot (RES27,
/// O.3). The pipeline reads it from the table, runs `step` on it and commits the new slot with an
/// `ActiveSlotUpdated` effect, so no second copy can disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMachine {
    pub(super) machine: MachineId,
    pub(super) slot: MachineSlot,
}

impl ActiveMachine {
    pub fn machine(&self) -> &MachineId {
        &self.machine
    }

    pub fn slot(&self) -> &MachineSlot {
        &self.slot
    }

    pub(super) fn digest_into(&self, digest: &mut Digest) {
        let Self { machine, slot } = self;
        digest.machine(machine);
        digest.slot(slot);
    }
}

/// A machine waiting below the active one on a scene's hand-off stack (RES27). `pushed_at` is the
/// revision of the commit that pushed it: the basis for the machine's own check, when the table
/// returns to it, that the world still fits what it was doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandOffFrame {
    pub(super) machine: MachineId,
    pub(super) slot: MachineSlot,
    pub(super) pushed_at: SessionRevision,
}

impl HandOffFrame {
    pub fn machine(&self) -> &MachineId {
        &self.machine
    }

    pub fn slot(&self) -> &MachineSlot {
        &self.slot
    }

    pub fn pushed_at(&self) -> SessionRevision {
        self.pushed_at
    }

    pub(super) fn digest_into(&self, digest: &mut Digest) {
        let Self {
            machine,
            slot,
            pushed_at,
        } = self;
        digest.machine(machine);
        digest.slot(slot);
        digest.revision(*pushed_at);
    }
}

/// What a hand-off recorded: where the table came from, where it went and how much it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandOffRecord {
    pub from: MachineId,
    pub to: MachineId,
    pub carried: usize,
}

impl HandOffRecord {
    pub(super) fn digest_into(&self, digest: &mut Digest) {
        let Self { from, to, carried } = self;
        digest.machine(from);
        digest.machine(to);
        digest.count(*carried);
    }
}

/// One open scene (D.1). A scene is closed in the commit in which its last seat leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneRecord {
    pub(super) generation: SceneGeneration,
    pub(super) beat: Option<BeatName>,
    pub(super) conversation: Option<ConversationOutcome>,
    pub(super) active: Option<ActiveMachine>,
    pub(super) stack: Vec<HandOffFrame>,
}

impl SceneRecord {
    pub(super) fn opened(generation: SceneGeneration) -> Self {
        Self {
            generation,
            beat: None,
            conversation: None,
            active: None,
            stack: Vec::new(),
        }
    }

    pub fn generation(&self) -> SceneGeneration {
        self.generation
    }

    /// The authored beat that was played through last, if any.
    pub fn beat(&self) -> Option<&BeatName> {
        self.beat.as_ref()
    }

    /// How the scene's conversation ended, once it did.
    pub fn conversation(&self) -> Option<ConversationOutcome> {
        self.conversation
    }

    /// The scene's one active group machine and its saved state, once one was set.
    pub fn active(&self) -> Option<&ActiveMachine> {
        self.active.as_ref()
    }

    /// The id of the active machine, once one was set.
    pub fn active_machine(&self) -> Option<&MachineId> {
        self.active.as_ref().map(ActiveMachine::machine)
    }

    /// The machines waiting below the active one, oldest first.
    pub fn hand_off_stack(&self) -> &[HandOffFrame] {
        &self.stack
    }

    pub(super) fn digest_into(&self, digest: &mut Digest) {
        let Self {
            generation,
            beat,
            conversation,
            active,
            stack,
        } = self;
        digest.number(generation.get());
        digest.option(beat.as_ref(), |digest, beat| digest.beat(beat));
        digest.option(conversation.as_ref(), |digest, outcome| {
            digest.text(outcome.key())
        });
        digest.option(active.as_ref(), |digest, active| active.digest_into(digest));
        digest.count(stack.len());
        for frame in stack {
            frame.digest_into(digest);
        }
    }
}
