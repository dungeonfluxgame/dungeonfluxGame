//! The reads that reach into what the owner holds: `inspect <report>`, `dump table`,
//! `dump <machine>` and `chaos` with no words.
//!
//! `inspect` builds `df_table::inspect`'s report of one module of the committed world for one
//! audience; the pure crate redacts while it builds, so a value the audience may not see is never
//! read. A dump names the table or one machine and prints its saved state, which only the
//! operator's local build serves. Each read is one audited line and changes nothing.

use df_table::identity::{MachineId, SceneId};
use df_table::inspect::{Inspect, InspectAudience, InspectView, MachinesOf, WiredPorts};
use df_table::machine::slot::MachineSlot;
use serde::Serialize;

use crate::engine::audit::{DumpOf, EngineAudit};
use crate::engine::report::reach::{DumpReport, InspectReport, MachineDump};
use crate::engine::report::{ChaosReport, EngineError, ViewCause};
use crate::engine::request::{DUMP_TABLE_WORD, InspectName, inspect_audience_word};
use crate::engine::setup::WIRED_PORTS;

use super::Owner;
use super::reads::table_dump;

/// A report as JSON, or the host fault of one that cannot be written.
fn as_json(report: &impl Serialize) -> Result<serde_json::Value, EngineError> {
    serde_json::to_value(report).map_err(|error| EngineError::ViewUnavailable {
        cause: ViewCause::Report,
        detail: error.to_string(),
    })
}

impl Owner {
    pub(super) fn inspect(
        &self,
        report: InspectName,
        audience: InspectAudience,
    ) -> Result<InspectReport, EngineError> {
        if let Some(seat) = audience.seat() {
            self.seat_is_at_the_table(seat.get())?;
        }
        let now_ms = self.now_ms();
        let view = InspectView::new(audience, now_ms);
        let table = &self.committed;
        let value = match report {
            InspectName::Table => as_json(&table.facts().inspect(view)),
            InspectName::Machines => {
                let snapshot = table.facts().snapshot();
                let machines = MachinesOf {
                    registry: &self.registry,
                    snapshot: &snapshot,
                };
                as_json(&machines.inspect(view))
            }
            InspectName::Pending => as_json(&table.pending().inspect(view)),
            InspectName::Rulings => as_json(&table.rulings().inspect(view)),
            InspectName::Requests => as_json(&table.requests().inspect(view)),
            InspectName::Results => as_json(&table.results().inspect(view)),
            InspectName::Pipeline => as_json(&table.inspect(view)),
            InspectName::Queue => as_json(&self.queue.inspect(view)),
            InspectName::Registry => as_json(&self.registry.inspect(view)),
            InspectName::Ports => {
                let ports = WiredPorts {
                    registry: &self.registry,
                    wired: &WIRED_PORTS,
                };
                as_json(&ports.inspect(view))
            }
            InspectName::Settings => as_json(&self.config.settings.inspect(view)),
        }?;
        self.audit(EngineAudit::Inspect { report, audience });
        Ok(InspectReport {
            report,
            audience: inspect_audience_word(audience),
            now_ms,
            revision: self.revision(),
            value,
        })
    }

    /// `dump table` or `dump <machine>`, of the committed world.
    pub(super) fn dump_named(&self, target: &str) -> Result<DumpReport, EngineError> {
        let (revision, commit) = (self.revision(), self.commit_id.get());
        if target == DUMP_TABLE_WORD {
            self.audit(EngineAudit::Dump {
                revision,
                commit,
                of: DumpOf::Table,
            });
            return Ok(DumpReport::Table {
                revision,
                commit,
                table: table_dump(&self.committed),
            });
        }
        let machine = MachineId::new(target).map_err(|_| EngineError::InvalidName {
            what: target.to_owned(),
        })?;
        if self.registry.machine(&machine).is_none() {
            return Err(EngineError::UnknownMachine {
                machine: target.to_owned(),
            });
        }
        let places = self.places_of(&machine);
        if places.is_empty() {
            return Err(EngineError::MachineNotOnTable {
                machine: target.to_owned(),
            });
        }
        self.audit(EngineAudit::Dump {
            revision,
            commit,
            of: DumpOf::Machine(machine),
        });
        Ok(DumpReport::Machine {
            revision,
            commit,
            places,
        })
    }

    /// Every place `machine` holds on the committed table, the focus first.
    fn places_of(&self, machine: &MachineId) -> Vec<MachineDump> {
        let snapshot = self.committed.facts().snapshot();
        let focus = snapshot
            .focus_machine()
            .map(|active| active.machine().clone());
        let mut places = Vec::new();
        let mut place = |scene: Option<SceneId>, stacked: bool, slot: &MachineSlot| {
            places.push(MachineDump {
                machine: machine.to_string(),
                state: self.state_of(machine, slot),
                scene: scene.map(SceneId::get),
                stacked,
                focus: !stacked && focus.as_ref() == Some(machine),
                schema_version: slot.schema_version(),
                slot: slot.payload().clone(),
            });
        };
        if let Some(active) = snapshot.table_machine()
            && active.machine() == machine
        {
            place(None, false, active.slot());
        }
        for (scene, record) in snapshot.scenes() {
            if let Some(active) = record.active()
                && active.machine() == machine
            {
                place(Some(scene), false, active.slot());
            }
            for frame in record.hand_off_stack() {
                if frame.machine() == machine {
                    place(Some(scene), true, frame.slot());
                }
            }
        }
        places.sort_by_key(|place| !place.focus);
        places
    }

    fn state_of(&self, machine: &MachineId, slot: &MachineSlot) -> String {
        self.registry
            .machine(machine)
            .and_then(|spec| spec.state_name(slot).ok())
            .map_or_else(|| "unknown".to_owned(), |state| state.to_string())
    }

    /// The faults in force: the scripted commits that wait and whether the interpreter is down,
    /// for each control this table has.
    pub(super) fn chaos_show(&self) -> Vec<ChaosReport> {
        let controls = &self.adapters.controls;
        let mut faults = Vec::new();
        if let Some(store) = controls.commit.as_ref() {
            faults.push(ChaosReport {
                fault: "commit".to_owned(),
                in_force: format!("{} scripted writes wait", store.scripted_remaining()),
            });
        }
        if let Some(interpreter) = controls.interpreter.as_ref() {
            faults.push(ChaosReport {
                fault: "interpreter".to_owned(),
                in_force: if interpreter.is_down() { "down" } else { "up" }.to_owned(),
            });
        }
        let in_force = controls
            .commit
            .as_ref()
            .map_or(0, |store| usize::from(store.scripted_remaining() > 0))
            + controls
                .interpreter
                .as_ref()
                .map_or(0, |interpreter| usize::from(interpreter.is_down()));
        self.audit(EngineAudit::ChaosShow { in_force });
        faults
    }
}
