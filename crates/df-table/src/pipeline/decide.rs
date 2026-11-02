//! Stage 3, DECIDE: draw the dice at the one draw point and let the rules owner resolve (C, H.3).
//!
//! Dice come only from the `Dice` port and are used only here and when a pending is answered. The
//! dice the rules owner prepared are drawn first, then it resolves; if it needs more dice (a stage
//! only a hit reaches) it says so and they are drawn and it is asked again with every die so far,
//! up to `maximum_draw_rounds` and never past `decide_deadline_s` on the clock, because the rules
//! owner is a call-out and a count alone cannot bound a loop that calls out. A resolution that awaits answers (a window, the roller's choice, a
//! roll) ends the stage as `Await`; everything drawn so far is kept so that a restart resumes with
//! it and never draws again. A ruling that applies is passed to the rules owner with the resume
//! call, which validates it (RES48).
//!
//! It never opens a pending itself (the action runner does, with the machine's report) and never
//! changes state. A port that fails is recorded once and ends the input declined
//! `not_supported_yet`: the dice already drawn are lost with the input, and nothing is committed.

use crate::event::{FailureCause, PipelineStep};
use crate::identity::SeatId;
use crate::outcome::{Decline, DeclineCode, OutcomeKind};
use crate::pending::StoredDraw;
use crate::ports::{
    ActionId, Awaiting, Draw, DrawRequest, PortName, Resolution, Resolved, Resumption,
    RulingAnswer, WindowAnswer,
};

use super::turn::Turn;

/// What DECIDE settled.
pub(super) enum Decided {
    Applied(Resolved),
    Declined(Decline),
    Await(Awaiting),
}

/// How the first resolution call of a stage is made.
#[derive(Clone, Copy)]
pub(super) enum Entry<'a> {
    /// A fresh action: `resolve`.
    Fresh,
    /// A pending is being finished: `resume` with the answers and the ruling that apply.
    Resume {
        answers: &'a [WindowAnswer],
        ruling: Option<&'a RulingAnswer>,
    },
}

/// Draws the prepared dice, resolves, and ends the stage.
pub(super) fn decide(
    turn: &mut Turn<'_, '_>,
    seat: SeatId,
    action: &ActionId,
    prepared: &[DrawRequest],
    ruling: Option<&RulingAnswer>,
    drawn: &mut Vec<Draw>,
) -> Decided {
    let started = turn.now();
    let entry = match ruling {
        Some(ruling) => Entry::Resume {
            answers: &[],
            ruling: Some(ruling),
        },
        None => Entry::Fresh,
    };
    let decided = match draw_all(turn, prepared, drawn) {
        Err(decline) => Decided::Declined(decline),
        Ok(()) => match resolve_loop(turn, seat, action, drawn, entry) {
            Ok(Resolution::Applied(resolved)) => Decided::Applied(resolved),
            Ok(Resolution::Declined(decline)) => Decided::Declined(decline),
            Ok(Resolution::Await(awaiting)) => Decided::Await(awaiting),
            Ok(Resolution::NeedsDraws(_)) => {
                // `resolve_loop` never returns this; refuse rather than guess if it ever does.
                Decided::Declined(Decline::with_default_reason(DeclineCode::NotSupportedYet))
            }
            Err(decline) => Decided::Declined(decline),
        },
    };
    let ended = match &decided {
        Decided::Applied(_) => None,
        Decided::Declined(_) => Some(OutcomeKind::Declined),
        Decided::Await(_) => Some(OutcomeKind::Pending),
    };
    turn.stage_done(PipelineStep::Decide, started, ended);
    decided
}

/// Draws every requested die in order, adding each to `drawn` (everything the rules owner is
/// given) and to the turn's own record.
pub(super) fn draw_all(
    turn: &mut Turn<'_, '_>,
    requests: &[DrawRequest],
    drawn: &mut Vec<Draw>,
) -> Result<(), Decline> {
    for request in requests {
        let started = turn.now();
        match turn.pipeline.ports.dice.draw(turn.correlation, *request) {
            Ok(draw) => {
                drawn.push(draw);
                turn.draws.push(StoredDraw::of(draw));
            }
            Err(error) => {
                turn.boundary_failed(error.port, error.cause, started);
                return Err(Decline::with_default_reason(DeclineCode::NotSupportedYet));
            }
        }
    }
    Ok(())
}

/// Asks the rules owner until it stops asking for more dice. The result is never `NeedsDraws`.
pub(super) fn resolve_loop(
    turn: &mut Turn<'_, '_>,
    seat: SeatId,
    action: &ActionId,
    drawn: &mut Vec<Draw>,
    entry: Entry<'_>,
) -> Result<Resolution, Decline> {
    let rules = turn.pipeline.ports.rules;
    let rounds = turn.pipeline.limits.maximum_draw_rounds;
    let deadline_ms = u64::from(turn.pipeline.settings.decide_deadline_s()).saturating_mul(1_000);
    for _ in 0..rounds {
        let started = turn.now();
        if turn.elapsed_since(turn.started_ms) > deadline_ms {
            turn.boundary_failed(PortName::RulesOwner, FailureCause::TimedOut, started);
            return Err(Decline::with_default_reason(DeclineCode::NotSupportedYet));
        }
        let answered = match entry {
            Entry::Fresh => rules.resolve(turn.correlation, seat, action, drawn),
            Entry::Resume { answers, ruling } => rules.resume(
                turn.correlation,
                seat,
                action,
                &Resumption {
                    draws: drawn,
                    answers,
                    ruling,
                },
            ),
        };
        match answered {
            Ok(Resolution::NeedsDraws(requests)) => draw_all(turn, &requests, drawn)?,
            Ok(settled) => return Ok(settled),
            Err(error) => {
                turn.boundary_failed(error.port, error.cause, started);
                return Err(Decline::with_default_reason(DeclineCode::NotSupportedYet));
            }
        }
    }
    turn.boundary_failed(
        PortName::RulesOwner,
        FailureCause::InvalidAnswer,
        turn.started_ms,
    );
    Err(Decline::with_default_reason(DeclineCode::NotSupportedYet))
}
