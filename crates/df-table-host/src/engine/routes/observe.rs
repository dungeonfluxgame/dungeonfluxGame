//! The routes of the event and settings verbs: `history`, `trace`, `replay` and `config`.
//!
//! They are the same shape as every route of the engine API: a read is a GET, a change is a POST
//! of JSON, and every answer is an `EngineReply`. A replay that is not built answers 501 with the
//! typed `not_supported_yet`, never a 200 with nothing in it.

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::response::Response;
use tokio::time::Instant;

use crate::engine::request::{ConfigControl, HistoryRequest, ReplayRequest, TraceQuery};

use super::{EngineRoutes, answer, body, engine, query};

pub(super) async fn history(
    State(routes): State<EngineRoutes>,
    history_query: Result<Query<HistoryRequest>, QueryRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let request = query(history_query)?;
        engine(&routes)?.history(request).await
    }
    .await;
    answer(&routes, "history", started, result)
}

pub(super) async fn trace(
    State(routes): State<EngineRoutes>,
    trace_query: Result<Query<TraceQuery>, QueryRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let TraceQuery { operation } = query(trace_query)?;
        engine(&routes)?.trace(operation).await
    }
    .await;
    answer(&routes, "trace", started, result)
}

pub(super) async fn replay(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let request: ReplayRequest = body(bytes)?;
        engine(&routes)?.replay(request).await
    }
    .await;
    answer(&routes, "replay", started, result)
}

pub(super) async fn config_show(State(routes): State<EngineRoutes>) -> Response {
    let started = Instant::now();
    let result = async { engine(&routes)?.config(ConfigControl::Show).await }.await;
    answer(&routes, "config", started, result)
}

pub(super) async fn config_set(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let control: ConfigControl = body(bytes)?;
        engine(&routes)?.config(control).await
    }
    .await;
    answer(&routes, "config", started, result)
}
