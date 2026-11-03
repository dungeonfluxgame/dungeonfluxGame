//! The wire view of an audience: what a TV or a phone is served as a `MachineView` (O.16 H3).
//!
//! It is the committed table's real projection mapped by `machine_wire` and checked by the wire's
//! own guard, so a view is never blank and a shared screen never carries a choice. One case has no
//! projection: a seat that is in the queue but not yet at the table (a phone that has not pressed
//! join). `df_table::projection` refuses such a seat with a typed error and never builds an empty
//! view (C5), so the owner answers it here with a guest view made only of what the running machine
//! says for an unseated seat: its own readout for that audience and the actions its spec declares
//! for an unseated seat (the lobby's join). A guest view holds no private data and is never saved.
//!
//! Like every read it is of the committed world, one audited `projection` line.

use df_protocol::common as wire;
use df_table::identity::SeatId;
use df_table::pipeline::PipelineTable;
use df_table::ports::Audience;
use df_table::projection::ProjectionError;

use crate::engine::audit::EngineAudit;
use crate::engine::projection_adapter::{ViewError, project_machine_view};
use crate::engine::report::EngineError;
use crate::engine::request::AudienceSpec;
use crate::machine_wire::wire_revision;

use super::Owner;

/// The hero name of a seat that has not sat down.
fn guest_name(seat: SeatId) -> String {
    format!("Seat {seat}")
}

/// A name in words, with a capital: `join` is "Join".
fn words(name: &str) -> String {
    let spaced = name.replace(['_', '-'], " ");
    let mut letters = spaced.chars();
    match letters.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), letters.as_str()),
        None => spaced,
    }
}

fn unwritable(what: impl ToString) -> EngineError {
    EngineError::Unwritable {
        what: what.to_string(),
    }
}

impl Owner {
    /// The committed table's `MachineView` for `spec`.
    pub(super) fn wire_view(&self, spec: AudienceSpec) -> Result<wire::MachineView, EngineError> {
        let audience = spec.audience();
        if let AudienceSpec::Seat { seat } = spec {
            self.seat_is_at_the_table(seat)?;
        }
        let view = match project_machine_view(&self.committed, audience) {
            Ok(view) => view,
            Err(ViewError::Projection(ProjectionError::SeatNotAtTable { seat })) => {
                self.guest_view(&self.committed, seat)?
            }
            Err(error) => return Err(unwritable(error)),
        };
        self.audit(EngineAudit::Projection {
            audience,
            revision: self.revision(),
        });
        Ok(view)
    }

    /// The view of a seat that is in the queue and not at the table.
    fn guest_view(
        &self,
        table: &PipelineTable,
        seat: SeatId,
    ) -> Result<wire::MachineView, EngineError> {
        let snapshot = table.facts().snapshot();
        let active = snapshot
            .focus_machine()
            .ok_or_else(|| EngineError::UnknownMachine {
                machine: "none".to_owned(),
            })?;
        let machine =
            self.registry
                .machine(active.machine())
                .ok_or_else(|| EngineError::UnknownMachine {
                    machine: active.machine().to_string(),
                })?;
        let readout = machine
            .project(table.facts(), active.slot(), Audience::Seat(seat))
            .map_err(unwritable)?;
        let state = machine.state_name(active.slot()).map_err(unwritable)?;
        let offers = machine
            .spec()
            .unseated_actions()
            .iter()
            .map(|action| wire::MachineOffer {
                offer_id: action.as_str().to_owned(),
                label: words(action.as_str()),
                explain: None,
                unavailable_reason: None,
                group: None,
            })
            .collect();
        let view = wire::MachineView {
            revision: Some(wire_revision(self.session_revision())),
            machine_id: active.machine().to_string(),
            state_label: state.to_string(),
            status: Some(wire::MachineStatus {
                kind: wire::MachineStatusKind::Ok as i32,
                waiting_who: None,
                waiting_what: None,
            }),
            reason: None,
            readout: readout.as_str().to_owned(),
            audience: Some(wire::machine_view::Audience::Seat(wire::MachineSeatView {
                hero: guest_name(seat),
                goal: None,
                narration: vec![readout.as_str().to_owned()],
                note: None,
                image: None,
                offers,
            })),
        };
        df_api::machine::check_machine_view(&view)
            .map_err(|error| unwritable(format!("{error:?}")))?;
        Ok(view)
    }
}
