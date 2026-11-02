//! The typed events of the table state that are not already a variant of `TableEvent`.
//!
//! It owns `TableStateEvent`, carried by `TableEvent::TableState` so the table state's vocabulary
//! has one home and `event.rs` keeps to its size: a result set aside because the scene or the
//! revision it was started under is no longer current (D.2, `plate.rejected_epoch`). Applied and
//! refused effects already have their events (`EffectApplied`, `EffectRefused`). Every field is an
//! `EventField` (ids, counts, closed keys), the event is class A and files under its own audit
//! area, and the rendering is an exhaustive `match`. It never decides what to record: the code that
//! handles a stale result does, once.

use std::fmt::{self, Write};

use serde::{Deserialize, Serialize};

use crate::event::{EventClass, write_context};
use crate::event_field::LineWriter;
use crate::identity::{Correlation, MachineId, SceneId, TableScope};
use crate::table_state::judge::StaleCause;

/// One thing the table state did that explains a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "what", rename_all = "snake_case", deny_unknown_fields)]
pub enum TableStateEvent {
    /// A result started under an earlier scene generation or revision was set aside. `generation`
    /// is the one it carried and `current` the scene's generation now, when the scene is open.
    ResultRejected {
        scope: TableScope,
        correlation: Correlation,
        cause: StaleCause,
        generation: u64,
        current: Option<u64>,
    },
    /// A scene closed in this commit, or was replaced by a fresh one: it took its active machine
    /// and its hand-off stack with it, and a pending resolution of that scene must be closed by
    /// whoever holds it (RES26).
    SceneClosed {
        scope: TableScope,
        correlation: Correlation,
        scene: SceneId,
        generation: u64,
        machine: Option<MachineId>,
        stack_depth: usize,
    },
}

impl TableStateEvent {
    pub fn class(&self) -> EventClass {
        match self {
            Self::ResultRejected { .. } | Self::SceneClosed { .. } => EventClass::Always,
        }
    }

    pub fn area(&self) -> &'static str {
        match self {
            Self::ResultRejected { .. } => "RESULT",
            Self::SceneClosed { .. } => "SCENE",
        }
    }

    pub(crate) fn write_audit_body<W: Write>(&self, out: &mut W) -> fmt::Result {
        match self {
            Self::ResultRejected {
                scope,
                correlation,
                cause,
                generation,
                current,
            } => {
                let mut line = LineWriter::start(out, "rejected")?;
                write_context(&mut line, scope, correlation)?;
                line.field("cause", cause)?;
                line.field("generation", generation)?;
                line.field_if_present("current", current)
            }
            Self::SceneClosed {
                scope,
                correlation,
                scene,
                generation,
                machine,
                stack_depth,
            } => {
                let mut line = LineWriter::start(out, "closed")?;
                write_context(&mut line, scope, correlation)?;
                line.field("closed_scene", scene)?;
                line.field("generation", generation)?;
                line.field_if_present("active", machine)?;
                line.field("stack_depth", stack_depth)
            }
        }
    }
}
