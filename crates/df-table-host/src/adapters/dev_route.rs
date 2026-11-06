//! The route behind `rules check`: `POST /dev/engine/rules/check`.
//!
//! It owns the one handler, merged into the dev engine's router so it sits behind the same guard.
//! It holds the check kit only when the table runs the `df-rules` backend; with the scripted
//! backend it refuses with `RulesCheckError::ScriptedBackend`. It answers through the shared dev
//! reply (`dev_http::reply`), records one audit line per answered or refused check (a refusal's
//! line has the engine routes' `duration_ms`), and changes no table state. A body over the host's
//! limit is a typed `unreadable_body` refusal like on every other engine POST.

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::BytesRejection;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;

use crate::dev_http::{parse_body, reply};
use crate::engine::report::UnreadableBodyCause;
use crate::engine::routes::path;
use crate::state::AppState;

use super::dev_check::{
    RulesCheckError, RulesCheckKit, RulesCheckReport, RulesCheckRequest, audit_line,
    refusal_audit_line,
};

#[derive(Clone)]
struct CheckRoute {
    app: Arc<AppState>,
    kit: Option<Arc<RulesCheckKit>>,
}

/// Reads the request and runs the check. The kit's work is short, in-memory rules arithmetic and
/// recorded draws, so it runs on the request's task.
fn run_check(
    route: &CheckRoute,
    bytes: Result<Bytes, BytesRejection>,
) -> Result<RulesCheckReport, RulesCheckError> {
    let bytes = bytes.map_err(|rejection| RulesCheckError::UnreadableBody {
        cause: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            UnreadableBodyCause::TooLarge
        } else {
            UnreadableBodyCause::Interrupted
        },
    })?;
    let request: RulesCheckRequest = parse_body(&bytes)?;
    let kit = route.kit.as_ref().ok_or(RulesCheckError::ScriptedBackend)?;
    let report = kit.run(&request)?;
    route.app.record_line(audit_line(&request, &report));
    Ok(report)
}

async fn rules_check(
    State(route): State<CheckRoute>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = run_check(&route, bytes);
    if let Err(error) = &result {
        route
            .app
            .record_line(refusal_audit_line(error, started.elapsed().as_millis()));
    }
    reply(result)
}

/// The route of `rules check`, answering from `kit` when there is one.
pub fn router(app: Arc<AppState>, kit: Option<Arc<RulesCheckKit>>) -> Router {
    Router::new()
        .route(path::RULES_CHECK, post(rules_check))
        .with_state(CheckRoute { app, kit })
}
