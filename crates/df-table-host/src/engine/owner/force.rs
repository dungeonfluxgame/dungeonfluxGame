//! `force`: a driven change that puts the table in a machine and state the operator names.
//!
//! It is an operator tool of local and test builds (H.3, H.16), and it is as honest as any input:
//! the machine is entered through its own `enter` (or its saved record is read through `load`, which
//! refuses what it cannot read), the state reached must be the one named, the machine is started
//! at its home through the table's own committed effect, and the change is committed through the
//! same `commit_world` as a batch (of no inputs), so a forced table is saved like any other and a
//! failed commit leaves the table as it was. It never edits the table's shared facts; a machine
//! that needs a scene or seats it does not have refuses to enter, and the refusal is the answer.
//! What it audits is only validated: a name that is not a valid name is recorded as
//! `invalid_name` and never as the text it was.

use df_table::effects::TableState;
use df_table::identity::MachineId;
use df_table::machine::{CarryItem, Entry, Scope, StateName, entered_event};
use df_table::machine_slot::MachineSlot;
use df_table::table_state::MachineHome;

use crate::engine::audit::EngineAudit;
use crate::engine::report::{CommitDisposition, EngineError, ForceReport};
use crate::engine::request::ForceRequest;

use super::Owner;

fn invalid_name(text: &str) -> EngineError {
    EngineError::InvalidName {
        what: text.to_owned(),
    }
}

fn refused(reason: impl ToString) -> EngineError {
    EngineError::ForceRefused {
        reason: reason.to_string(),
    }
}

impl Owner {
    pub(super) fn force(&mut self, request: ForceRequest) -> Result<ForceReport, EngineError> {
        let result = self.force_machine(&request);
        if let Err(error) = &result {
            self.audit(EngineAudit::ForceRefused {
                machine: MachineId::new(&request.machine).ok(),
                cause: refusal_key(error),
            });
        }
        result
    }

    fn force_machine(&mut self, request: &ForceRequest) -> Result<ForceReport, EngineError> {
        let id = MachineId::new(&request.machine).map_err(|_| invalid_name(&request.machine))?;
        let machine =
            self.registry
                .machine(&id)
                .cloned()
                .ok_or_else(|| EngineError::UnknownMachine {
                    machine: request.machine.clone(),
                })?;
        let state = StateName::new(&request.state).map_err(|_| invalid_name(&request.state))?;
        if !machine.spec().states().contains(&state) {
            return Err(EngineError::UnknownState {
                machine: request.machine.clone(),
                state: request.state.clone(),
            });
        }
        let mut working = self.committed.checkpoint();
        let bad_payload = |reason: String| EngineError::BadPayload {
            machine: request.machine.clone(),
            reason,
        };
        let slot = match &request.payload {
            Some(payload) => {
                let saved = MachineSlot::from_parts(
                    id.clone(),
                    machine.spec().schema_version(),
                    payload.clone(),
                );
                machine
                    .load(&saved)
                    .map_err(|error| bad_payload(error.to_string()))?
            }
            None => machine
                .enter(working.facts(), &entry_of(working.facts()))
                .map_err(refused)?,
        };
        let reached = machine
            .state_name(&slot)
            .map_err(|error| bad_payload(error.to_string()))?;
        if reached != state {
            return Err(bad_payload(format!(
                "it starts in '{reached}', not '{state}'; give the saved state as field=value words"
            )));
        }
        let home = match (machine.spec().scope(), working.facts().active_scene()) {
            (Scope::Table, _) => MachineHome::Table,
            (Scope::Group, Some(scene)) => MachineHome::Scene { scene },
            (Scope::Group, None) => return Err(refused("no scene is active for a group machine")),
            (Scope::Seat, _) => return Err(refused("a seat lane is not forced")),
        };
        let operation = self.mint_engine_operation();
        working
            .start_machine(self.correlation_for(operation), home, slot)
            .map_err(refused)?;
        self.commit_forced(working, id, state)
    }

    /// Commits the forced table as a batch of no inputs.
    fn commit_forced(
        &mut self,
        working: df_table::pipeline::PipelineTable,
        machine: MachineId,
        state: StateName,
    ) -> Result<ForceReport, EngineError> {
        let operation = self.mint_engine_operation();
        let correlation = self.correlation_for(operation);
        let last_turn = self.queue.last_turn_seq();
        match self.commit_world(working, 0, 0, last_turn) {
            Ok(saved) => {
                self.sink.record(&entered_event(
                    &self.committed.scope(),
                    correlation,
                    state.clone(),
                ));
                self.audit(EngineAudit::Force {
                    machine: machine.clone(),
                    state: state.clone(),
                    revision: saved.commit.get(),
                });
                Ok(ForceReport {
                    machine: machine.to_string(),
                    state: state.to_string(),
                    revision: saved.commit.get(),
                    commit: CommitDisposition::Committed {
                        id: saved.commit.get(),
                    },
                })
            }
            Err(error) => Err(refused(error)),
        }
    }
}

/// What the machine is entered with: the active scene and every seat at the table.
fn entry_of(facts: &TableState) -> Entry {
    let scene = facts.active_scene().map(CarryItem::Scene);
    let seats = facts.seats().iter().copied().map(CarryItem::Seat);
    Entry::carrying(scene.into_iter().chain(seats).collect())
}

fn refusal_key(error: &EngineError) -> &'static str {
    match error {
        EngineError::InvalidName { .. } => "invalid_name",
        EngineError::UnknownMachine { .. } => "unknown_machine",
        EngineError::UnknownState { .. } => "unknown_state",
        EngineError::BadPayload { .. } => "bad_payload",
        _ => "refused",
    }
}
