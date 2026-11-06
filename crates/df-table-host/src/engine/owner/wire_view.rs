//! The wire view of an audience: what a TV or a phone is served as a `MachineView` (O.16 H3).
//!
//! It is the committed table's real projection mapped by `machine_wire` and checked by the wire's
//! own guard, so a view is never blank and a shared screen never carries a choice. One case has no
//! projection: a seat that is in the queue but not yet at the table (a phone that has not pressed
//! join). `df_table::projection` refuses such a seat with a typed error and never builds an empty
//! view (C5), so the owner answers it here with a guest view. Everything but two things is the
//! shared screen's own view (the machine, the revision, the state label and status word a seated
//! phone is also given, the reason): what the whole room may see. The two are what only the guest
//! is told: the prompt the running machine says to an unseated seat in front of what it says to
//! everyone (the lobby's "join the table"), and the actions its spec declares for an unseated seat
//! (the join button). The readout is the shared screen's with that prompt before it. No seated
//! player's name, narration, notice or offer is read, so none can reach the guest, whatever a
//! machine writes for a seated audience. A guest view is never saved.
//!
//! Like every read it is of the committed world, one audited `projection` line.

use df_protocol::common as wire;
use df_table::identity::SeatId;
use df_table::pipeline::PipelineTable;
use df_table::ports::Audience;
use df_table::projection::ProjectionError;

use crate::engine::audit::EngineAudit;
use crate::engine::projection_adapter::{ViewError, project_machine_view};
use crate::engine::report::{EngineError, ViewCause};
use crate::engine::request::AudienceSpec;

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

/// What a machine says to an unseated seat in front of what it says to everyone, or `None` when
/// it says nothing more or says it in another shape.
fn join_prompt<'a>(to_the_seat: &'a str, to_everyone: &str) -> Option<&'a str> {
    to_the_seat
        .strip_suffix(to_everyone)
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
}

/// A view the projection refused, with the projection's sentence.
fn unavailable(error: impl ToString) -> EngineError {
    EngineError::ViewUnavailable {
        cause: ViewCause::Projection,
        detail: error.to_string(),
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
            Err(error) => return Err(unavailable(error)),
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
        let public = project_machine_view(table, Audience::Public).map_err(unavailable)?;
        let snapshot = table.facts().snapshot();
        let active = snapshot
            .focus_machine()
            .ok_or(EngineError::NoMachineRunning)?;
        let machine =
            self.registry
                .machine(active.machine())
                .ok_or_else(|| EngineError::UnknownMachine {
                    machine: active.machine().to_string(),
                })?;
        let said_to_the_seat = machine
            .project(table.facts(), active.slot(), Audience::Seat(seat))
            .map_err(unavailable)?;
        let said_to_everyone = machine
            .project(table.facts(), active.slot(), Audience::Public)
            .map_err(unavailable)?;
        // The prompt is only what the machine puts in front of its public text for this seat; a
        // text of another shape is not used, so nothing but the prompt can come from it.
        let readout = match join_prompt(said_to_the_seat.as_str(), said_to_everyone.as_str()) {
            Some(prompt) => format!("{prompt} {}", public.readout),
            None => public.readout.clone(),
        };
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
            revision: public.revision,
            machine_id: public.machine_id,
            state_label: public.state_label,
            status: public.status,
            reason: public.reason,
            readout: readout.clone(),
            audience: Some(wire::machine_view::Audience::Seat(wire::MachineSeatView {
                hero: guest_name(seat),
                goal: None,
                narration: vec![readout],
                note: None,
                image: None,
                offers,
            })),
        };
        df_api::machine::check_machine_view(&view).map_err(|error| {
            EngineError::ViewUnavailable {
                cause: ViewCause::WireCheck,
                detail: format!("{error:?}"),
            }
        })?;
        Ok(view)
    }
}
