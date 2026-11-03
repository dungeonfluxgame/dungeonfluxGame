//! The table state's report: revision, status, time, scenes, seats and the cast as one audience
//! may see them (D.1 to D.7, K.13).
//!
//! A creature the party cannot see (hidden, invisible or only sensed) is left out of the cast of
//! the host and of every seat, whole: not named, not counted, not marked, because a count that
//! changes when a hidden creature moves would say it moved (the paired-state rule of projection).
//! A seat still sees its own hero when it is hidden. The hit points of a creature that is not a
//! party member are the operator's, since the shared screen does not show them. The digest of the
//! table moves with every hidden fact, so it is the operator's too. Seats, scenes and presence are
//! public: the party list is on the shared screen. A scene's beat is story progress, which the
//! shared screen does not show, so it is the host's.
//!
//! It reads the snapshot only and never changes the table.

use serde::Serialize;

use crate::effects::BeatName;
use crate::element::ElementId;
use crate::identity::{MachineId, SceneId, SeatId, SessionRevision};
use crate::inspect::{Inspect, InspectView, Needs, Visible};
use crate::table_state::{
    CastEntry, CastKind, ConditionName, Controller, DisplayName, Presence, TableSnapshot,
    TableState, TableStatus, Visibility,
};

/// The hit points of a creature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VitalsLine {
    pub hit_points: u32,
    pub maximum_hit_points: u32,
}

/// One creature of the cast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CastLine {
    pub id: ElementId,
    pub kind: CastKind,
    pub scene: SceneId,
    pub name: DisplayName,
    pub controller: Controller,
    pub visibility: Visibility,
    /// Shown for the party; the operator's for everyone else.
    pub vitals: Visible<VitalsLine>,
    pub conditions: Vec<ConditionName>,
}

/// One open scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SceneLine {
    pub scene: SceneId,
    pub generation: u64,
    /// The beat last played through: story progress the shared screen does not show.
    pub beat: Visible<Option<BeatName>>,
    pub active_machine: Option<MachineId>,
    pub stack_depth: usize,
}

/// One seat at the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeatLine {
    pub seat: SeatId,
    pub presence: Presence,
    pub scene: Option<SceneId>,
    pub hero: Option<ElementId>,
}

/// The table state as one audience may see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableReport {
    pub revision: SessionRevision,
    pub status: TableStatus,
    pub world_time_s: u64,
    pub seat_capacity: usize,
    pub split_policy: &'static str,
    pub focus_scene: Option<SceneId>,
    pub scenes: Vec<SceneLine>,
    pub seats: Vec<SeatLine>,
    /// Only the creatures this audience may know are there.
    pub cast: Vec<CastLine>,
    /// A digest of every fact and of the applied-operation memory (`TableState::digest`).
    pub digest: Visible<u64>,
}

impl Inspect for TableState {
    type Report = TableReport;

    fn inspect(&self, view: InspectView) -> TableReport {
        let snapshot = self.snapshot();
        TableReport {
            revision: snapshot.revision(),
            status: snapshot.status(),
            world_time_s: snapshot.world_time_s(),
            seat_capacity: snapshot.seat_capacity(),
            split_policy: snapshot.split_policy().key(),
            focus_scene: snapshot.focus_scene(),
            scenes: snapshot
                .scenes()
                .map(|(scene, record)| SceneLine {
                    scene,
                    generation: record.generation().get(),
                    beat: Visible::when(view, Needs::Host, || record.beat().cloned()),
                    active_machine: record.active_machine().cloned(),
                    stack_depth: record.hand_off_stack().len(),
                })
                .collect(),
            seats: snapshot
                .seats()
                .iter()
                .filter_map(|seat| {
                    snapshot.seat(*seat).map(|state| SeatLine {
                        seat: *seat,
                        presence: state.presence(),
                        scene: state.scene(),
                        hero: state.hero().cloned(),
                    })
                })
                .collect(),
            cast: cast_lines(&snapshot, view),
            digest: Visible::when(view, Needs::Operator, || self.digest()),
        }
    }
}

fn cast_lines(snapshot: &TableSnapshot, view: InspectView) -> Vec<CastLine> {
    let own_hero = view
        .audience
        .seat()
        .and_then(|seat| snapshot.hero_of(seat))
        .map(CastEntry::id);
    snapshot
        .cast()
        .filter(|entry| {
            view.audience.is_operator()
                || entry.visibility() == Visibility::Seen
                || own_hero == Some(entry.id())
        })
        .map(|entry| CastLine {
            id: entry.id().clone(),
            kind: entry.kind(),
            scene: entry.scene(),
            name: entry.name().clone(),
            controller: entry.controller(),
            visibility: entry.visibility(),
            vitals: Visible::gated(
                view.audience.is_operator() || entry.kind() == CastKind::Party,
                Needs::Operator,
                || VitalsLine {
                    hit_points: entry.vitals().hit_points(),
                    maximum_hit_points: entry.vitals().maximum_hit_points(),
                },
            ),
            conditions: entry.conditions().to_vec(),
        })
        .collect()
}
