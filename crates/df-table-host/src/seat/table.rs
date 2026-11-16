//! What the seat routes ask the table's engine: a seat's view, and a seat's lobby action
//! submitted through the wire intent's one path (`engine::routes::play::submit_intent`). The
//! seat is always the one the key gave; the lobby machine decides what happens.

use df_protocol::common as wire;
use df_table::lobby::{self, LobbyAction};

use super::request::SeatError;
use crate::engine::EngineHandle;
use crate::engine::machine_io::{WireIntentBody, WireIntentKind};
use crate::engine::request::AudienceSpec;
use crate::engine::routes::play::submit_intent;

/// What the table did with one of a seat's actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActionResult {
    Applied,
    /// Declined, with the sentence the seat is shown.
    Declined(String),
    /// Anything else (a question back, a pending answer, an observation of the operation).
    NotApplied,
}

/// The seat's view, read without a view line of its own (the action that follows writes one).
pub(crate) async fn quiet_view(
    engine: &EngineHandle,
    seat: u64,
) -> Result<wire::MachineView, SeatError> {
    Ok(engine.intent_view(AudienceSpec::Seat { seat }).await?)
}

/// Whether the view offers the seat the lobby action `action`.
pub(crate) fn offers(view: &wire::MachineView, action: LobbyAction) -> bool {
    let Some(wire::machine_view::Audience::Seat(seat)) = &view.audience else {
        return false;
    };
    seat.offers
        .iter()
        .any(|offer| offer.offer_id == action.key())
}

/// Whether the view is the lobby's: the table still gathers.
pub(crate) fn is_gathering(view: &wire::MachineView) -> bool {
    view.machine_id == lobby::MACHINE_ID
}

/// Whether the view says the seat said it is ready: the lobby offers it `unready`.
pub(crate) fn is_ready(view: &wire::MachineView) -> bool {
    offers(view, LobbyAction::Unready)
}

/// Submits the lobby action `action` as `seat` and reads what the table did.
pub(crate) async fn act(
    engine: &EngineHandle,
    seat: u64,
    action: LobbyAction,
) -> Result<ActionResult, SeatError> {
    let intent = WireIntentBody {
        seat,
        intent: WireIntentKind::Tap {
            offer: action.key().to_owned(),
        },
        operation: None,
        machine: None,
        observed: None,
    };
    let response = submit_intent(engine, &intent).await?;
    use wire::machine_receipt::Result as Outcome;
    use wire::submit_machine_intent_response::Outcome as Answer;
    let Some(Answer::Receipt(receipt)) = &response.outcome else {
        return Ok(ActionResult::NotApplied);
    };
    Ok(match &receipt.result {
        Some(Outcome::Applied(_)) => ActionResult::Applied,
        Some(Outcome::Declined(decline)) => ActionResult::Declined(decline.reason.clone()),
        _ => ActionResult::NotApplied,
    })
}

/// Whether the table has `seat` now: only an unseated seat is offered `join`, and a table that has
/// moved on from the lobby carried its seats with it.
pub(crate) fn is_seated(view: &wire::MachineView) -> bool {
    !offers(view, LobbyAction::Join)
}

/// The lobby's `join` as `seat`: a decline (the table is full) is `TableFull`.
async fn join(engine: &EngineHandle, seat: u64) -> Result<(), SeatError> {
    match act(engine, seat, LobbyAction::Join).await? {
        ActionResult::Applied => Ok(()),
        ActionResult::Declined(_) => Err(SeatError::TableFull),
        ActionResult::NotApplied => Err(SeatError::NotApplied),
    }
}

/// Seats `seat` at the table through the lobby's `join`, unless the table already has it.
pub(crate) async fn sit_down(engine: &EngineHandle, seat: u64) -> Result<(), SeatError> {
    let view = quiet_view(engine, seat).await?;
    if is_seated(&view) {
        return Ok(());
    }
    join(engine, seat).await
}

/// The first seat of `free` the table offers a join, seated through the lobby. A free seat the
/// table already has (a dev command seated it) is passed over; a table that has moved on from the
/// lobby takes no new seat (`GameStarted`).
pub(crate) async fn take_free_seat(engine: &EngineHandle, free: &[u64]) -> Result<u64, SeatError> {
    for seat in free {
        let view = quiet_view(engine, *seat).await?;
        if !is_gathering(&view) {
            return Err(SeatError::GameStarted);
        }
        if !is_seated(&view) {
            join(engine, *seat).await?;
            return Ok(*seat);
        }
    }
    Err(SeatError::TableFull)
}
