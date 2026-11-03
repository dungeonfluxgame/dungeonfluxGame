//! The routes of the event and settings verbs: `history`, `trace`, `replay` and `config`.
//!
//! They are the same shape as every route of the engine API: a read is a GET, a change is a POST
//! of JSON, and every answer is an `EngineReply`. A replay that is not built answers 501 with the
//! typed `not_supported_yet`, never a 200 with nothing in it.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::response::Response;
use serde::Deserialize;

use crate::engine::report::EngineError;
use crate::engine::request::{ConfigControl, HistoryRequest, ReplayRequest};

use super::{EngineRoutes, OPERATION_PARAMETER, body, engine, reply};

pub(super) async fn history(
    State(routes): State<EngineRoutes>,
    Query(request): Query<HistoryRequest>,
) -> Response {
    let result = async { engine(&routes)?.history(request).await }.await;
    reply(&routes, result)
}

#[derive(Debug, Deserialize)]
pub(super) struct TraceQuery {
    operation: Option<String>,
}

pub(super) async fn trace(
    State(routes): State<EngineRoutes>,
    Query(query): Query<TraceQuery>,
) -> Response {
    let result = async {
        let operation = query.operation.ok_or_else(|| EngineError::BadRequest {
            reason: format!("give ?{OPERATION_PARAMETER}=N or 32 hexadecimal digits"),
        })?;
        engine(&routes)?.trace(operation).await
    }
    .await;
    reply(&routes, result)
}

pub(super) async fn replay(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let request: ReplayRequest = body(&bytes)?;
        engine(&routes)?.replay(request).await
    }
    .await;
    reply(&routes, result)
}

pub(super) async fn config_show(State(routes): State<EngineRoutes>) -> Response {
    let result = async { engine(&routes)?.config(ConfigControl::Show).await }.await;
    reply(&routes, result)
}

pub(super) async fn config_set(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let control: ConfigControl = body(&bytes)?;
        engine(&routes)?.config(control).await
    }
    .await;
    reply(&routes, result)
}
