//! The active machines' report: which machine runs where, in which state, and what its own
//! `inspect` says (RES27, H.16 `dump`).
//!
//! Each line is one saved slot of the table: the table machine, each scene's active machine and
//! every frame of each scene's hand-off stack. The state name and schema version come from the
//! machine's own `inspect`, never from the slot's text, so a machine that cannot report is a
//! `ReportFailed` line with the closed cause, and a slot whose machine is not registered and
//! enabled is `NotAvailable`: neither is dropped or shown as healthy. The saved payload is
//! operator-only, because a machine keeps its own hidden facts in it. The host sees every line
//! without the payload, and a seat sees the one machine its screen names, without the stack and
//! without the payload. It never decodes a payload or changes a slot.

use serde::Serialize;
use serde_json::Value;

use crate::identity::{MachineId, SessionRevision};
use crate::inspect::{Inspect, InspectView, Needs, Visible};
use crate::machine::MachineFailureCause;
use crate::machine_slot::MachineSlot;
use crate::machine_spec::StateName;
use crate::registry::Registry;
use crate::table_state::{MachineHome, TableSnapshot};

/// What the machine itself reports about one slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MachineStanding {
    Reported {
        state: StateName,
        schema_version: u32,
        /// The saved state as data.
        payload: Visible<Value>,
    },
    /// The machine's `inspect` failed, which the conformance suite forbids.
    ReportFailed { cause: MachineFailureCause },
    /// No enabled machine of this id is registered.
    NotAvailable,
}

/// Where the slot sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Frame {
    /// The machine that runs now in its home.
    Active,
    /// A machine a hand-off pushed below the active one; `depth` counts from the oldest frame.
    Stacked {
        depth: usize,
        pushed_at: SessionRevision,
    },
}

/// One slot of the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActiveMachineLine {
    pub home: MachineHome,
    pub frame: Frame,
    pub machine: MachineId,
    /// The one the shared screen names (the focus scene's, else the table's).
    pub focus: bool,
    pub standing: MachineStanding,
}

/// The slots of the table, the table machine first, then each scene in scene order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachinesReport {
    pub machines: Vec<ActiveMachineLine>,
}

/// The registry and a snapshot of the table: what is needed to read each slot with its machine.
#[derive(Clone, Copy)]
pub struct MachinesOf<'a> {
    pub registry: &'a Registry,
    pub snapshot: &'a TableSnapshot,
}

impl MachinesOf<'_> {
    fn standing(
        &self,
        view: InspectView,
        machine: &MachineId,
        slot: &MachineSlot,
    ) -> MachineStanding {
        let Some(registered) = self.registry.machine(machine) else {
            return MachineStanding::NotAvailable;
        };
        match registered.inspect(slot) {
            Ok(report) => MachineStanding::Reported {
                state: report.state,
                schema_version: report.schema_version,
                payload: Visible::when(view, Needs::Operator, || slot.payload().clone()),
            },
            Err(error) => MachineStanding::ReportFailed {
                cause: error.cause(),
            },
        }
    }

    fn line(
        &self,
        view: InspectView,
        home: MachineHome,
        frame: Frame,
        machine: &MachineId,
        slot: &MachineSlot,
    ) -> ActiveMachineLine {
        let focus_scene = self.snapshot.focus_scene();
        let focus = matches!(frame, Frame::Active)
            && match home {
                MachineHome::Table => focus_scene.is_none_or(|scene| {
                    self.snapshot
                        .scene(scene)
                        .is_none_or(|record| record.active().is_none())
                }),
                MachineHome::Scene { scene } => focus_scene == Some(scene),
            };
        ActiveMachineLine {
            home,
            frame,
            machine: machine.clone(),
            focus,
            standing: self.standing(view, machine, slot),
        }
    }
}

impl Inspect for MachinesOf<'_> {
    type Report = MachinesReport;

    fn inspect(&self, view: InspectView) -> MachinesReport {
        let mut machines = Vec::new();
        if let Some(active) = self.snapshot.table_machine() {
            machines.push(self.line(
                view,
                MachineHome::Table,
                Frame::Active,
                active.machine(),
                active.slot(),
            ));
        }
        for (scene, record) in self.snapshot.scenes() {
            let home = MachineHome::Scene { scene };
            for (depth, frame) in record.hand_off_stack().iter().enumerate() {
                machines.push(self.line(
                    view,
                    home,
                    Frame::Stacked {
                        depth,
                        pushed_at: frame.pushed_at(),
                    },
                    frame.machine(),
                    frame.slot(),
                ));
            }
            if let Some(active) = record.active() {
                machines.push(self.line(
                    view,
                    home,
                    Frame::Active,
                    active.machine(),
                    active.slot(),
                ));
            }
        }
        if !view.audience.administers() {
            machines.retain(|line| line.focus);
        }
        MachinesReport { machines }
    }
}
