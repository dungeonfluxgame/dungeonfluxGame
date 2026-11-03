//! The route behind `rules check`: `POST /dev/engine/rules/check`.
//!
//! It owns the one handler and its reply, merged into the dev engine's router so it sits behind
//! the same guard. It holds the check kit only when the table runs the `df-rules` backend; with
//! the scripted backend it answers `NotAvailable`, as the other dev controls do for a table
//! without them. It records one audit line per answered check and changes no table state.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};

use crate::engine::report::{EngineError, EngineReply};
use crate::engine::routes::path;
use crate::state::AppState;

use super::dev_check::{RulesCheckKit, RulesCheckReport, RulesCheckRequest, audit_line};

#[derive(Clone)]
struct CheckRoute {
    app: Arc<AppState>,
    kit: Option<Arc<RulesCheckKit>>,
}

fn run_check(route: &CheckRoute, bytes: &Bytes) -> Result<RulesCheckReport, EngineError> {
    let request: RulesCheckRequest =
        serde_json::from_slice(bytes).map_err(|error| EngineError::BadRequest {
            reason: format!("the request body is not valid: {error}"),
        })?;
    let kit = route
        .kit
        .as_ref()
        .ok_or_else(|| EngineError::NotAvailable {
            what: "a rules check (this table runs the scripted rules backend)".to_owned(),
        })?;
    let report = kit.run(&request)?;
    route.app.record_line(audit_line(&request, &report));
    Ok(report)
}

async fn rules_check(State(route): State<CheckRoute>, bytes: Bytes) -> Response {
    let result = run_check(&route, &bytes);
    route.app.echo_new_lines();
    let status = result
        .as_ref()
        .err()
        .map_or(StatusCode::OK, |_| StatusCode::BAD_REQUEST);
    (status, Json(EngineReply::from_result(result))).into_response()
}

/// The route of `rules check`, answering from `kit` when there is one.
pub fn router(app: Arc<AppState>, kit: Option<Arc<RulesCheckKit>>) -> Router {
    Router::new()
        .route(path::RULES_CHECK, post(rules_check))
        .with_state(CheckRoute { app, kit })
}
