//! Opens a pending resolution and holds the machine on it (C.3 step 3, C.4, C.5, RES26, K.5).
//!
//! Every kind of pending is opened the same way: the machine is stepped with the id of the pending
//! it will wait on and reports `Waiting{pending_id}`, the pending is staged with that new state in
//! one commit, and the plan ends there (a plan outlives neither its turn nor its bound, C.2). So a
//! seat's one-tap confirmation (C.5), a ruling asked of the host (RES15, K.4), a reaction window,
//! a choice and a roll are all held in the pending, with the action as its continuation, and the
//! machine holds only the id. While it waits the machine declines every other intent, so no second
//! pending for the same action can stack; the exits are the answer, the seat's own cancel, the
//! host's refusal and `cancel_pending` (a seat that left for good, a scene that changed, the host's
//! decision), and each of them returns the machine to its normal state.
//!
//! A ruling question is settled here too: a stored ruling with an equal key and scope is reused, a
//! reviewed automatic policy settles it at once, and otherwise the host is asked (RES15, RES48).
//! It never reads player text and never changes anything but through `commit`.

use crate::effects::TableState;
use crate::event::FailureCause;
use crate::identity::SeatId;
use crate::machine::{Decision, Machine, MachineInput, StepInput, StepOutcome};
use crate::outcome::{Decline, DeclineCode, Outcome, PendingKind};
use crate::pending::window::{Completion, ResolutionWindow};
use crate::pending::{
    Continuation, PendingOpening, PendingResolution, RulingSource, ScopedRuling, StoredDraw,
};
use crate::ports::{
    ActionId, Awaiting, AwaitingKind, ConfirmationReason, Draw, Offer, PortName, RulingAnswer,
    RulingOption, RulingRequest,
};

use super::action::{self, ActionEnd, PlanState, stop_declined, stop_pending};
use super::events::PipelineEvent;
use super::table_access::PipelineTable;
use super::turn::Turn;

/// What a ruling question came to.
pub(super) enum RulingFlow {
    /// Go on with this answer; `store` is a ruling to keep once the action is applied.
    Use {
        answer: RulingAnswer,
        store: Option<ScopedRuling>,
    },
    Opened,
}

/// The machine broke the contract of reporting `Waiting` for the pending it was given.
fn broke_contract(turn: &Turn<'_, '_>) -> Decline {
    action::machine_broke_contract_decline(turn)
}

/// An `Awaiting` the pending book cannot hold is the rules owner breaking its contract: recorded
/// once, never guessed at.
pub(super) fn rules_gave_garbage(turn: &Turn<'_, '_>) -> ActionEnd {
    turn.boundary_failed(
        PortName::RulesOwner,
        FailureCause::InvalidAnswer,
        turn.started_ms,
    );
    stop_declined(Decline::with_default_reason(DeclineCode::NotSupportedYet))
}

/// Opens a pending and stages it with the commit.
#[allow(
    clippy::too_many_arguments,
    reason = "everything a pending is opened with, none of it optional"
)]
fn open_pending(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    state: &mut PlanState<'_>,
    seat: SeatId,
    kind: PendingKind,
    continuation: Continuation,
    window: ResolutionWindow,
    draws: Vec<StoredDraw>,
) {
    let id = table.pending().next_id();
    let offers = window.offers().len();
    let pending = PendingResolution::open(PendingOpening {
        id,
        kind,
        opened_by: seat,
        operation: turn.correlation.operation_id,
        turn: turn.correlation.turn_seq,
        now_ms: turn.now(),
        scene: table.active_scene(),
        continuation,
        draws,
        window,
    });
    turn.pending = Some(id);
    turn.on_commit.push(
        turn.pipeline_event(|scope, correlation| PipelineEvent::PendingOpened {
            scope,
            correlation,
            pending: id,
            kind,
            offers,
        }),
    );
    state.staged.open.push(pending);
}

/// Steps the machine with the pending it will wait on, then stages the pending with the machine's
/// `Waiting` state (C.4 phase one). The one place a pending is opened.
#[allow(
    clippy::too_many_arguments,
    reason = "the stage context of one action plus the pending it opens"
)]
fn hold(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    facts: &TableState,
    machine: &dyn Machine,
    state: &mut PlanState<'_>,
    (seat, action): (SeatId, &ActionId),
    (kind, window, continuation): (PendingKind, ResolutionWindow, Continuation),
    draws: Vec<StoredDraw>,
) -> Result<(), Decline> {
    let id = table.pending().next_id();
    let decision = match kind {
        PendingKind::Roll => Decision::RollPending(id),
        PendingKind::Choice | PendingKind::Reaction | PendingKind::Ruling => Decision::Pending(id),
    };
    let step = StepInput {
        correlation: turn.correlation,
        now_ms: turn.now(),
        table: facts,
        input: MachineInput::Intent {
            seat,
            action: action.clone(),
        },
        decision: Some(decision),
    };
    let result = match machine.step(&state.slot, &step) {
        Ok(result) => result,
        Err(error) => {
            turn.emit(&error.event(&turn.scope, turn.correlation));
            return Err(Decline::with_default_reason(DeclineCode::NotViable));
        }
    };
    match &result.outcome {
        StepOutcome::Waiting(reported) if *reported == id => {}
        StepOutcome::Settled(Outcome::Declined(decline)) => return Err(decline.clone()),
        StepOutcome::Waiting(_) | StepOutcome::Settled(_) => return Err(broke_contract(turn)),
    }
    action::absorb(turn, machine, state, result);
    open_pending(turn, table, state, seat, kind, continuation, window, draws);
    Ok(())
}

/// The seat's own yes is asked before the rules owner resolves anything (C.5, RES26).
pub(super) fn open_confirmation(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    (facts, machine): (&TableState, &dyn Machine),
    state: &mut PlanState<'_>,
    (seat, action): (SeatId, &ActionId),
    reason: ConfirmationReason,
) -> ActionEnd {
    let choices = &turn.pipeline.choices;
    let offer = Offer {
        seat,
        choices: vec![choices.confirm.clone(), choices.cancel.clone()],
    };
    let Ok(window) = ResolutionWindow::new(vec![offer], Completion::AnyOffer) else {
        return rules_gave_garbage(turn);
    };
    let continuation = Continuation::Confirm {
        seat,
        action: action.clone(),
        reason,
    };
    match hold(
        turn,
        table,
        facts,
        machine,
        state,
        (seat, action),
        (PendingKind::Choice, window, continuation),
        Vec::new(),
    ) {
        Ok(()) => stop_pending(PendingKind::Choice),
        Err(decline) => stop_declined(decline),
    }
}

/// Phase one of a window, a choice or a roll (C.4): the machine is stepped with the pending it
/// waits on, and the pending is committed with the machine's `Waiting` state.
#[allow(
    clippy::too_many_arguments,
    reason = "the stage context of one action plus what it awaits"
)]
pub(super) fn await_answers(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    (facts, machine): (&TableState, &dyn Machine),
    state: &mut PlanState<'_>,
    (seat, action): (SeatId, &ActionId),
    awaiting: Awaiting,
    drawn: &[Draw],
) -> ActionEnd {
    let (kind, completion, offers, roll) = match awaiting.kind {
        AwaitingKind::Roll => {
            if awaiting.draws.is_empty() {
                return rules_gave_garbage(turn);
            }
            let offer = Offer {
                seat,
                choices: vec![turn.pipeline.choices.roll.clone()],
            };
            let sides = awaiting
                .draws
                .iter()
                .map(|request| request.sides.get())
                .collect();
            (PendingKind::Roll, Completion::AnyOffer, vec![offer], sides)
        }
        AwaitingKind::Choice => (
            PendingKind::Choice,
            Completion::EveryOffer,
            awaiting.offers,
            Vec::new(),
        ),
        AwaitingKind::Reaction => (
            PendingKind::Reaction,
            Completion::EveryOffer,
            awaiting.offers,
            Vec::new(),
        ),
    };
    let Ok(window) = ResolutionWindow::new(offers, completion) else {
        return rules_gave_garbage(turn);
    };
    let continuation = Continuation::Resolve {
        seat,
        action: action.clone(),
        nesting: 0,
        roll,
    };
    let stored: Vec<StoredDraw> = drawn.iter().copied().map(StoredDraw::of).collect();
    match hold(
        turn,
        table,
        facts,
        machine,
        state,
        (seat, action),
        (kind, window, continuation),
        stored,
    ) {
        Ok(()) => stop_pending(kind),
        Err(decline) => stop_declined(decline),
    }
}

/// A ruling question: reuse a stored ruling with an equal key and scope, settle it by a reviewed
/// automatic policy, or ask the host (RES15, RES48, K.4).
pub(super) fn settle_ruling(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    (facts, machine): (&TableState, &dyn Machine),
    state: &mut PlanState<'_>,
    (seat, action): (SeatId, &ActionId),
    request: &RulingRequest,
) -> Result<RulingFlow, Decline> {
    let scene = table.active_scene();
    match table.rulings().find(&request.key, request.scope, scene) {
        Some(stored) if !stored.needs_reconfirmation(scene) => {
            let answer = stored.answer().clone();
            turn.ruling_note = Some(answer.clone());
            let (key, tier) = (request.key.clone(), stored.tier());
            turn.on_commit
                .push(
                    turn.pipeline_event(|scope, correlation| PipelineEvent::RulingReused {
                        scope,
                        correlation,
                        key,
                        tier,
                    }),
                );
            Ok(RulingFlow::Use {
                answer,
                store: None,
            })
        }
        Some(stored) => {
            let mut asked = request.clone();
            asked.suggested = RulingOption {
                answer: stored.answer().clone(),
                tier: stored.tier(),
            };
            open_ruling(
                turn,
                table,
                (facts, machine),
                state,
                (seat, action),
                asked,
                true,
            )?;
            Ok(RulingFlow::Opened)
        }
        None if request.reviewed_automatic => {
            let answer = request.suggested.answer.clone();
            let ruling = ScopedRuling::new(
                request.key.clone(),
                request.scope,
                scene,
                answer.clone(),
                request.suggested.tier,
                RulingSource::ReviewedAutomatic,
                action.clone(),
            );
            Ok(RulingFlow::Use {
                answer,
                store: Some(ruling),
            })
        }
        None => {
            open_ruling(
                turn,
                table,
                (facts, machine),
                state,
                (seat, action),
                request.clone(),
                false,
            )?;
            Ok(RulingFlow::Opened)
        }
    }
}

/// Opens a `pending{ruling}`: the host answers with a tap, the requester may cancel.
fn open_ruling(
    turn: &mut Turn<'_, '_>,
    table: &PipelineTable,
    (facts, machine): (&TableState, &dyn Machine),
    state: &mut PlanState<'_>,
    (seat, action): (SeatId, &ActionId),
    request: RulingRequest,
    reuse: bool,
) -> Result<(), Decline> {
    let choices = &turn.pipeline.choices;
    let mut host_choices = vec![
        choices.accept_suggestion.clone(),
        choices.decline_ruling.clone(),
    ];
    // A ruling answer is a name under the same rule as an action, so none is ever skipped.
    host_choices.extend(
        request
            .alternatives
            .iter()
            .filter_map(|option| ActionId::new(option.answer.as_str()).ok()),
    );
    let mut offers = Vec::new();
    match table.host() {
        Some(host) if host == seat => {
            host_choices.push(choices.cancel_ruling.clone());
            offers.push(Offer {
                seat: host,
                choices: host_choices,
            });
        }
        Some(host) => {
            offers.push(Offer {
                seat: host,
                choices: host_choices,
            });
            offers.push(Offer {
                seat,
                choices: vec![choices.cancel_ruling.clone()],
            });
        }
        None => offers.push(Offer {
            seat,
            choices: vec![choices.cancel_ruling.clone()],
        }),
    }
    let window = ResolutionWindow::new(offers, Completion::AnyOffer).map_err(|_| {
        turn.boundary_failed(
            PortName::RulesOwner,
            FailureCause::InvalidAnswer,
            turn.started_ms,
        );
        Decline::with_default_reason(DeclineCode::NotSupportedYet)
    })?;
    let continuation = Continuation::Ruling {
        seat,
        action: action.clone(),
        request,
        reuse,
    };
    hold(
        turn,
        table,
        facts,
        machine,
        state,
        (seat, action),
        (PendingKind::Ruling, window, continuation),
        Vec::new(),
    )
}
