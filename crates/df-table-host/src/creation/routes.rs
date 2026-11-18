//! The creation routes. A seat's own (`df_playground::path::PLAY_CREATION*`) are on
//! the player listener and the loopback listener alike, keyed by the seat the request's key holds
//! (`SeatCaller`); the `dev` command line calls the same routes as a seat (`x-df-dev-seat`). The
//! dev routes (`path`, loopback only) list the offers, show the table, give a seat's sheet file
//! and start play.
//!
//! Each answers a `CreationReply` of its value or a typed `CreationRouteError`, with one
//! `CREATION refused route=<route> error=<key>` line for a refusal.
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use df_playground::pages::creation::{BuildChoose, BuildStep};
use df_playground::path::{
    PLAY_CREATION, PLAY_CREATION_CHOOSE, PLAY_CREATION_REOPEN, PLAY_CREATION_SUBMIT,
    PLAY_CREATION_SURPRISE,
};
use df_rules::hero::hero_offers;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::desk;
use super::request::{CreationRouteError, SeatAsk, TableAsk};
use crate::dev_http::{parse_body, read_query, reply};
use crate::seat::key::SeatCaller;
use crate::state::AppState;

/// The addresses of the creation dev routes (loopback only).
pub mod path {
    /// Every core species, class and background with its status (GET).
    pub const OFFERS: &str = "/dev/creation/offers";
    /// Every seat as the TV sees it (GET).
    pub const TABLE: &str = "/dev/creation/table";
    /// A seat's derived sheet in the shape the check judge reads (GET `?seat=`).
    pub const SHEET: &str = "/dev/creation/sheet";
    /// POST a `TableAsk`: hand the table to play once every seat is ready.
    pub const START: &str = "/dev/creation/start";
}

type AppRoutes = State<Arc<AppState>>;

/// Writes the refusal line of a refused route, then answers.
fn answer<T: Serialize>(
    app: &AppState,
    route: &str,
    result: Result<T, CreationRouteError>,
) -> Response {
    if let Err(error) = &result {
        app.record_line(format!(
            "CREATION refused route={route} error={}",
            error.key()
        ));
    }
    reply(result)
}

fn body<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, CreationRouteError> {
    parse_body(bytes).map_err(CreationRouteError::InvalidBody)
}

/// The seat the request's key holds.
async fn seat_of(app: &AppState, headers: &HeaderMap) -> Result<u64, CreationRouteError> {
    Ok(SeatCaller::of(app, headers).await?.seat())
}

async fn build(State(app): AppRoutes, headers: HeaderMap) -> Response {
    let result = async {
        let seat = seat_of(&app, &headers).await?;
        desk::view(&app, seat).await
    }
    .await;
    answer(&app, "build", result)
}

async fn choose(State(app): AppRoutes, headers: HeaderMap, bytes: Bytes) -> Response {
    let result = async {
        let seat = seat_of(&app, &headers).await?;
        let choose: BuildChoose = body(&bytes)?;
        desk::choose(&app, seat, choose).await
    }
    .await;
    answer(&app, "choose", result)
}

async fn surprise(State(app): AppRoutes, headers: HeaderMap, bytes: Bytes) -> Response {
    let result = async {
        let seat = seat_of(&app, &headers).await?;
        let BuildStep {} = body(&bytes)?;
        desk::surprise(&app, seat).await
    }
    .await;
    answer(&app, "surprise", result)
}

async fn submit(State(app): AppRoutes, headers: HeaderMap, bytes: Bytes) -> Response {
    let result = async {
        let seat = seat_of(&app, &headers).await?;
        let BuildStep {} = body(&bytes)?;
        desk::submit(&app, seat).await
    }
    .await;
    answer(&app, "submit", result)
}

async fn reopen(State(app): AppRoutes, headers: HeaderMap, bytes: Bytes) -> Response {
    let result = async {
        let seat = seat_of(&app, &headers).await?;
        let BuildStep {} = body(&bytes)?;
        desk::reopen(&app, seat).await
    }
    .await;
    answer(&app, "reopen", result)
}

async fn offers() -> Response {
    reply::<_, CreationRouteError>(Ok(hero_offers()))
}

async fn table(State(app): AppRoutes) -> Response {
    let result = desk::table(&app).await;
    answer(&app, "table", result)
}

async fn sheet(State(app): AppRoutes, query: Result<Query<SeatAsk>, QueryRejection>) -> Response {
    let result = async {
        let ask = read_query(query).map_err(CreationRouteError::InvalidQuery)?;
        desk::sheet_file(&app, ask.seat).await
    }
    .await;
    answer(&app, "sheet", result)
}

async fn start(State(app): AppRoutes, bytes: Bytes) -> Response {
    let result = async {
        let TableAsk {} = body(&bytes)?;
        desk::start(&app).await
    }
    .await;
    answer(&app, "start", result)
}

/// A seat's creation routes, for the player listener and the loopback listener alike.
pub(crate) fn player_router(app: Arc<AppState>) -> Router {
    Router::new()
        .route(PLAY_CREATION, get(build))
        .route(PLAY_CREATION_CHOOSE, post(choose))
        .route(PLAY_CREATION_SURPRISE, post(surprise))
        .route(PLAY_CREATION_SUBMIT, post(submit))
        .route(PLAY_CREATION_REOPEN, post(reopen))
        .with_state(app)
}

/// The creation dev routes (loopback only).
pub fn router_with(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::OFFERS, get(offers))
        .route(path::TABLE, get(table))
        .route(path::SHEET, get(sheet))
        .route(path::START, post(start))
        .with_state(app)
}
