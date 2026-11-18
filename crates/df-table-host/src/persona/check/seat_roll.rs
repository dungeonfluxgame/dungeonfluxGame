//! A seat's own roll (`PLAY_SEAT_ROLL`): the player taps a check of their hero's sheet on the
//! phone's Roll page, and the host runs it as any check. The check judge is asked at the
//! narrator's moment with the tapped test as the intent, so it sets the difficulty from the scene
//! (or finds the test does not fit it); the host's dice roll it against the seat's derived sheet
//! (`creation::sheet_file`) through the rules kit, the dungeon master calls and reads it on the
//! table, and the result line joins the story. The phone names the check, never a number. A seat's
//! roll waits for the table's turn like any check, and the judge is told the tests this hero
//! already rolled at this place, so a check is not rolled again until something changes.
//!
//! The route is on the player listener and the loopback listener alike (merged with the seat's
//! routes), keyed by the seat the request's key holds (`SeatCaller`); the `dev` command line calls
//! it as a seat (`x-df-dev-seat`). It reaches the check runner the persona routes started through
//! the host state (`AppState::check_rolls`).
//!
//! Audit lines: `SEAT rolled seat= check= total= dc= outcome=`, or `SEAT roll_skipped seat=
//! check= reason=` when the judge calls no roll; a refusal is `PERSONA refused verb=seat_roll
//! error=<key>`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::post;
use df_playground::fixture::{DieRole, RollView};
use df_playground::path::PLAY_SEAT_ROLL;
use df_playground::seat::wire::{SeatCheck, SeatRoll, SeatRolled};

use super::flow::{Pacer, RunClock, run_check};
use super::request::{
    CheckError, CheckJudgeRequest, CheckKind, CheckRouteError, CheckRunRequest, CheckSheet,
    CheckTest, Initiator,
};
use super::routes::CheckRoutes;
use super::sheet::{ability_of, read_sheet, skill_of};
use crate::blocking::run_owned;
use crate::dev_http::{parse_body, reply};
use crate::seat::key::SeatCaller;
use crate::state::AppState;

fn refused(cause: CheckError) -> CheckRouteError {
    CheckRouteError::BadRequest { cause }
}

/// The test `check` names, checked against the rules' closed lists.
fn test_of(check: &SeatCheck) -> Result<CheckTest, CheckRouteError> {
    let unknown = |key: &str| {
        refused(CheckError::UnknownCheck {
            key: key.to_owned(),
        })
    };
    Ok(match check {
        SeatCheck::Ability { ability } => CheckTest {
            kind: CheckKind::AbilityCheck,
            ability: Some(ability_of(ability).map_err(|_| unknown(ability))?),
            skill: None,
        },
        SeatCheck::Skill { skill } => CheckTest {
            kind: CheckKind::AbilityCheck,
            ability: None,
            skill: Some(skill_of(skill).map_err(|_| unknown(skill))?),
        },
        SeatCheck::Save { ability } => CheckTest {
            kind: CheckKind::SavingThrow,
            ability: Some(ability_of(ability).map_err(|_| unknown(ability))?),
            skill: None,
        },
    })
}

/// The check as the log names it: `skill:athletics`, `ability:str`, `save:con`.
fn logged_check(check: &SeatCheck) -> String {
    let (kind, key) = match check {
        SeatCheck::Ability { ability } => ("ability", ability),
        SeatCheck::Skill { skill } => ("skill", skill),
        SeatCheck::Save { ability } => ("save", ability),
    };
    format!("{kind}:{}", crate::world::logged_id(key))
}

/// The numbers of a rolled `view` as the phone shows them: the test die kept, what is added to it,
/// the total, the difficulty and the verdict.
fn rolled_of(view: &RollView) -> SeatRolled {
    SeatRolled::Rolled {
        roll: view
            .dice
            .iter()
            .find(|die| die.kept && die.role == DieRole::Test)
            .map_or(0, |die| i32::from(die.face)),
        modifier: view.modifiers.iter().map(|modifier| modifier.value).sum(),
        total: view.total,
        dc: view.target.as_ref().map(|target| target.value),
        outcome: view.outcome,
    }
}

/// Rolls `ask` for the seat `seat`, whose hero has the derived `sheet`: the check judge sets the
/// difficulty from the active place and what is offered there (the thing `about` names first), and
/// the whole check runs on the table (`run_check`).
pub(super) async fn roll_for_seat<P: Pacer>(
    routes: &CheckRoutes,
    pacer: &P,
    (seat, sheet): (u64, CheckSheet),
    ask: &SeatRoll,
) -> Result<SeatRolled, CheckRouteError> {
    let app = &routes.persona.app;
    let test = test_of(&ask.check)?;
    let mut offered = crate::world::place_items(app);
    if let Some(about) = &ask.about {
        let at = offered
            .iter()
            .position(|thing| &thing.id == about)
            .ok_or_else(|| refused(CheckError::UnknownThing { id: about.clone() }))?;
        // The thing the roll is about is the first thing in sight.
        let thing = offered.remove(at);
        offered.insert(0, thing);
    }
    // One check at a time on the table; what this hero already rolled here is read in its turn.
    let _turn = routes.table_turn().await;
    let place = crate::world::active_place_name(app);
    // The hero's character id is its sheet's (`hero-<id>`).
    let character = sheet.id.clone();
    let judge = CheckJudgeRequest {
        character: character.clone(),
        sheet,
        moment: Initiator::Narrator,
        action: None,
        station: None,
        place: place.clone(),
        in_sight: offered.into_iter().map(|thing| thing.name).collect(),
        checked: routes.seat_checked(&character, place.as_deref()),
        intent: Some(test),
        model: None,
    };
    let request = CheckRunRequest {
        judge,
        speak: true,
        inspiration: None,
        told: None,
        dc: None,
        tool: None,
    };
    let run = run_check(routes, pacer, &request).await?;
    let check = logged_check(&ask.check);
    let Some(rolled) = &run.rolled else {
        let reason = run.judged.verdict.reason.key();
        app.record_line(format!(
            "SEAT roll_skipped seat={seat} check={check} reason={reason}"
        ));
        return Ok(SeatRolled::NotCalled {
            reason: reason.to_owned(),
        });
    };
    // The test as called (a skill with its ability), as the judge compares what was rolled here.
    if let Some(call) = &run.judged.verdict.call {
        let called = CheckTest {
            kind: call.kind,
            ability: call.ability,
            skill: call.skill,
        };
        routes.keep_seat_checked(&character, place.as_deref(), called);
    }
    let view = &rolled.view;
    let answer = rolled_of(view);
    app.record_line(format!(
        "SEAT rolled seat={seat} check={check} total={} dc={} outcome={}",
        view.total,
        view.target
            .as_ref()
            .map_or_else(|| "-".to_owned(), |target| target.value.to_string()),
        view.outcome.map_or("-", |outcome| outcome.key())
    ));
    Ok(answer)
}

/// The seat's sheet as the check reads it: the seat's derived sheet from creation.
async fn seat_sheet(app: &AppState, seat: u64) -> Result<CheckSheet, CheckRouteError> {
    let file = crate::creation::sheet_file(app, seat)
        .await
        .map_err(|error| refused(CheckError::SeatSheet(Box::new(error))))?;
    read_sheet(&file.to_string()).map_err(|detail| refused(CheckError::SheetUnreadable { detail }))
}

async fn post_seat_roll(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let result = async {
        let routes = app
            .check_rolls()
            .map(Arc::clone)
            .ok_or_else(|| refused(CheckError::ChecksNotStarted))?;
        let seat = SeatCaller::of(&app, &headers)
            .await
            .map_err(|error| refused(CheckError::Seat(error)))?
            .seat();
        let ask: SeatRoll = parse_body(&bytes).map_err(CheckRouteError::InvalidBody)?;
        let sheet = seat_sheet(&app, seat).await?;
        run_owned(
            async move { roll_for_seat(&routes, &RunClock::start(), (seat, sheet), &ask).await },
            CheckRouteError::Stopped,
        )
        .await
    }
    .await;
    if let Err(error) = &result {
        app.record_line(format!(
            "PERSONA refused verb=seat_roll error={}",
            error.key()
        ));
    }
    reply(result)
}

/// The seat's roll route, for the player listener and the loopback listener alike.
pub(crate) fn seat_router(app: Arc<AppState>) -> Router {
    Router::new()
        .route(PLAY_SEAT_ROLL, post(post_seat_roll))
        .with_state(app)
}
