//! The routes of the event and settings verbs: `history`, `trace`, `replay` and `config`.
//!
//! They are the same shape as every route of the engine API: a read is a GET, a change is a POST
//! of JSON, and every answer is an `EngineReply`. A replay that is not built answers 501 with the
//! typed `not_supported_yet`, never a 200 with nothing in it.

use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;

use crate::dev_http::{parse_body, reply};
use crate::engine::request::{ConfigControl, HistoryRequest, ReplayRequest, TraceQuery};

use super::{EngineRoutes, engine, query};

pub(super) async fn history(
    State(routes): State<EngineRoutes>,
    history_query: Result<Query<HistoryRequest>, QueryRejection>,
) -> Response {
    let result = async {
        let request = query(history_query)?;
        engine(&routes)?.history(request).await
    }
    .await;
    reply(result)
}

pub(super) async fn trace(
    State(routes): State<EngineRoutes>,
    trace_query: Result<Query<TraceQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let TraceQuery { operation } = query(trace_query)?;
        engine(&routes)?.trace(operation).await
    }
    .await;
    reply(result)
}

pub(super) async fn replay(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: ReplayRequest = parse_body(&bytes)?;
        engine(&routes)?.replay(request).await
    }
    .await;
    reply(result)
}

pub(super) async fn config_show(State(routes): State<EngineRoutes>) -> Response {
    reply(async { engine(&routes)?.config(ConfigControl::Show).await }.await)
}

pub(super) async fn config_set(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let control: ConfigControl = parse_body(&bytes)?;
        engine(&routes)?.config(control).await
    }
    .await;
    reply(result)
}
