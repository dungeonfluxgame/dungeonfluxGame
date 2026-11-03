//! Stage 2, CHECK: may this seat do this now, and does it need asking first (C, C.2, C.5, RES15).
//!
//! The rules owner answers; this stage only routes the answer. An action the active machine does
//! not send to the rules owner is the machine's alone. For one that it does, the rules owner
//! prepares the action: a decline travels whole, with its reason, its missing piece and its
//! alternatives, because only the rules owner knows what is missing and what would be legal (the
//! pipeline never writes rules text). Then, for a reading that was not a direct tap on an offered
//! choice, the rules owner says whether the action needs the seat's one-tap confirmation (it
//! harms an ally or a neutral, or cannot be undone, C.5); then whether the source leaves a real
//! choice open, which is a ruling to ask for (RES15). The prepared dice list travels on to
//! DECIDE and nothing is rolled here.
//!
//! A step of a plan is checked against what the plan's earlier steps already did: the rules owner
//! is given the steps applied so far and the effects they staged (`PlanSoFar`), so it can decline
//! the second Attack of a hero with one Action `economy_spent` (C.2).
//!
//! CHECK always asks the rules owner afresh, so an input drawn from an older view is re-validated
//! against the table as it is (H.2). A port that fails is recorded once and the input is declined
//! `not_supported_yet`, never guessed at. It never reads the text of a line.

use crate::identity::SeatId;
use crate::outcome::{Decline, DeclineCode};
use crate::ports::{
    ActionId, ConfirmationReason, DrawRequest, PlanSoFar, Preparation, RulingRequest,
};

use super::turn::Turn;

/// What CHECK made of one action.
pub(super) enum Checked {
    /// The rules owner has the action, with the dice DECIDE draws for it.
    Ready(Vec<DrawRequest>),
    Declined(Decline),
    /// The seat is asked to confirm first.
    Confirm(ConfirmationReason),
    /// The source leaves a choice open; the dice are kept for after the ruling.
    Ruling {
        request: RulingRequest,
        draws: Vec<DrawRequest>,
    },
}

/// What the stage may skip because it was already settled.
#[derive(Debug, Clone, Copy)]
pub(super) struct Settled {
    /// The reading was not a direct tap, so the harm check applies (C.5).
    pub(super) harm_check: bool,
    /// The seat confirmed already.
    pub(super) confirmed: bool,
    /// A ruling already answers the open question.
    pub(super) ruled: bool,
}

/// Asks the rules owner. The caller records `step.finished` for CHECK once it knows how the stage
/// ended: a ruling that is reused or settled automatically passes the input on, while one that
/// opens a pending ends it.
pub(super) fn check(
    turn: &Turn<'_, '_>,
    seat: SeatId,
    action: &ActionId,
    settled: Settled,
    so_far: &PlanSoFar<'_>,
    started: u64,
) -> Checked {
    let rules = turn.pipeline.ports.rules;
    let failed = |error: crate::ports::PortError| {
        turn.boundary_failed(error.port, error.cause, started);
        Checked::Declined(Decline::with_default_reason(DeclineCode::NotSupportedYet))
    };
    let draws = match rules.prepare_in_plan(turn.correlation, seat, action, so_far) {
        Ok(Preparation::Ready { draws }) => draws,
        Ok(Preparation::Declined(decline)) => {
            return Checked::Declined(turn.pipeline.with_legal_alternatives(turn, decline));
        }
        Err(error) => return failed(error),
    };
    if settled.harm_check && !settled.confirmed {
        match rules.confirmation_needed(turn.correlation, seat, action) {
            Ok(Some(reason)) => return Checked::Confirm(reason),
            Ok(None) => {}
            Err(error) => return failed(error),
        }
    }
    if !settled.ruled {
        match rules.ruling_needed(turn.correlation, seat, action) {
            Ok(Some(request)) => return Checked::Ruling { request, draws },
            Ok(None) => {}
            Err(error) => return failed(error),
        }
    }
    Checked::Ready(draws)
}
