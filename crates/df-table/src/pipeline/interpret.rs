//! Stage 1, INTERPRET: a free line of text to a registered plan, or to a question, never a guess
//! (C, C.7, C.5, C.8).
//!
//! Only typed text and spoken lines come here; a tap is already an intent (C.1). The interpreter
//! port answers under a closed schema, and this stage checks the answer: the plan has between one
//! and `MAXIMUM_PLAN_STEPS` steps (C.0), every step is a registered action (the active machine
//! declares it, the rules owner offers it, or the machine accepts it now), and an ambiguity names
//! at least two candidates the seat may take now (they become the buttons, C.2). An answer that breaks the schema is a failure with cause
//! `text_invalid`, exactly like a time out or an unavailable model (C.7: output that is the wrong
//! shape is a failure), and all of them take the same path: the interpreter is asked once more
//! with the legal choices only, and if that fails too the failure is recorded once, the table says
//! "I could not read that. Pick one:" with the seat's legal choices as buttons that need no
//! model, and the player's draft is kept. A refusal is never retried (coding-style section 8,
//! pattern 11). A model that declines the line (`Rejected`) is a valid answer: it ends
//! `intent_invalid` and records no failure.
//!
//! A line the model reads as not an action (a question, a social line, a plan note, a rules
//! question, a joke) is returned as such: the pipeline routes it to its handler and runs nothing
//! (C.8). Nothing in this file looks at the words of a line: no keyword, regex or word list
//! (coding-style section 8, pattern 1).

use crate::event::{FailureCause, PipelineStep};
use crate::identity::SeatId;
use crate::machine::slot::MachineSlot;
use crate::machine::{Machine, MachineInput};
use crate::outcome::{Decline, DeclineCode, OutcomeKind};
use crate::ports::{ActionId, Interpretation, NonActionKind, PortError, PortName};
use crate::private_data::PlayerText;

use super::events::ClarifyCause;
use super::table_access::PipelineTable;
use super::turn::Turn;
use super::{Clarification, MAXIMUM_PLAN_STEPS};

pub(super) enum Interpreted {
    Plan(Vec<ActionId>),
    Clarify(Clarification),
    /// Not an action: routed to the handler of its kind and nothing is run (C.8).
    Routed(NonActionKind),
    Declined(Decline),
}

pub(super) fn interpret(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    seat: SeatId,
    text: &PlayerText,
) -> Interpreted {
    let started = turn.now();
    let interpreted = read_line(turn, table, seat, text, started);
    let ended = match &interpreted {
        Interpreted::Plan(_) => None,
        Interpreted::Clarify(_) => Some(OutcomeKind::NeedsClarification),
        Interpreted::Routed(_) => Some(OutcomeKind::Applied),
        Interpreted::Declined(_) => Some(OutcomeKind::Declined),
    };
    turn.stage_done(PipelineStep::Interpret, started, ended);
    interpreted
}

fn read_line(
    turn: &Turn<'_, '_>,
    table: &PipelineTable,
    seat: SeatId,
    text: &PlayerText,
    started: u64,
) -> Interpreted {
    let ports = &turn.pipeline.ports;
    let snapshot = table.facts().snapshot();
    let Some((machine, running)) = turn.pipeline.active_machine(table, &snapshot) else {
        return Interpreted::Declined(Decline::with_default_reason(DeclineCode::NotViable));
    };
    let legal = match ports.rules.legal_choices(turn.correlation, seat) {
        Ok(legal) => legal,
        Err(error) => return port_failed(turn, error, started),
    };
    let known = Known {
        machine: machine.as_ref(),
        slot: running.slot,
        table,
        seat,
        legal: &legal,
    };
    let first = known.checked(
        ports
            .interpreter
            .interpret(turn.correlation, seat, text, &legal),
    );
    let answered = match first {
        Err(error) if error.cause != FailureCause::Refused => known.checked(
            ports
                .interpreter
                .retry_with_choices_only(turn.correlation, seat, text, &legal),
        ),
        other => other,
    };
    let interpretation = match answered {
        Ok(interpretation) => interpretation,
        Err(error) => {
            turn.boundary_failed(error.port, error.cause, started);
            return Interpreted::Clarify(Clarification {
                options: legal,
                cause: ClarifyCause::Unreadable,
                keeps_draft: true,
            });
        }
    };
    match interpretation {
        Interpretation::Action(action) => Interpreted::Plan(vec![action]),
        Interpretation::Plan(steps) => Interpreted::Plan(steps),
        Interpretation::NeedsClarification => Interpreted::Clarify(Clarification {
            options: legal,
            cause: ClarifyCause::Unclear,
            keeps_draft: false,
        }),
        Interpretation::Ambiguous { candidates } => Interpreted::Clarify(Clarification {
            // Only the candidates the seat may take now are offered as buttons (C.2): a button
            // the rules refuse is a tap that is declined.
            options: candidates
                .into_iter()
                .filter(|candidate| legal.contains(candidate))
                .collect(),
            cause: ClarifyCause::Ambiguous,
            keeps_draft: false,
        }),
        Interpretation::NonAction(kind) => Interpreted::Routed(kind),
        Interpretation::Rejected => {
            Interpreted::Declined(Decline::with_default_reason(DeclineCode::IntentInvalid))
        }
    }
}

/// What the stage checks an interpreted answer against.
struct Known<'a> {
    machine: &'a dyn Machine,
    slot: &'a MachineSlot,
    table: &'a PipelineTable,
    seat: SeatId,
    legal: &'a [ActionId],
}

impl Known<'_> {
    /// Registered means the active machine declares it, the rules owner offers it, or the machine
    /// accepts it now. Whether it is legal now is CHECK's question, not this one.
    fn is_registered(&self, action: &ActionId) -> bool {
        let spec = self.machine.spec();
        spec.rules_actions().contains(action)
            || spec.unseated_actions().contains(action)
            || self.legal.contains(action)
            || self.machine.accepts(
                self.table.facts(),
                &MachineInput::Intent {
                    seat: self.seat,
                    action: action.clone(),
                },
                self.slot,
            )
    }

    /// An answer that fits the closed schema, or the failure it is: a port error as it came, or
    /// `text_invalid` for an answer of the wrong shape.
    fn checked(
        &self,
        answer: Result<Interpretation, PortError>,
    ) -> Result<Interpretation, PortError> {
        let interpretation = answer?;
        let well_formed = match &interpretation {
            Interpretation::Action(action) => self.is_registered(action),
            Interpretation::Plan(steps) => {
                !steps.is_empty()
                    && steps.len() <= MAXIMUM_PLAN_STEPS
                    && steps.iter().all(|step| self.is_registered(step))
            }
            Interpretation::Ambiguous { candidates } => {
                candidates.iter().all(|step| self.is_registered(step))
                    && candidates
                        .iter()
                        .filter(|candidate| self.legal.contains(candidate))
                        .count()
                        >= 2
            }
            Interpretation::NeedsClarification
            | Interpretation::NonAction(_)
            | Interpretation::Rejected => true,
        };
        if well_formed {
            Ok(interpretation)
        } else {
            Err(PortError::new(
                PortName::Interpreter,
                FailureCause::TextInvalid,
            ))
        }
    }
}

fn port_failed(turn: &Turn<'_, '_>, error: PortError, started: u64) -> Interpreted {
    turn.boundary_failed(error.port, error.cause, started);
    Interpreted::Declined(Decline::with_default_reason(DeclineCode::NotSupportedYet))
}
