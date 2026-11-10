//! The routes behind `rules check` and `rules roll`: `POST /dev/engine/rules/check` and
//! `POST /dev/engine/rules/roll`.
//!
//! It owns the two handlers, merged into the dev engine's router so they sit behind the same guard.
//! It holds the check kit only when the table runs the `df-rules` backend; with the scripted
//! backend it refuses with `RulesCheckError::ScriptedBackend`. It answers through the shared dev
//! reply (`dev_http::reply`), records one audit line per answered or refused call (a refusal's
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
use df_rules::roll::RollRequest;

use crate::dev_http::{parse_body, reply};
use crate::engine::report::UnreadableBodyCause;
use crate::engine::routes::path;
use crate::state::AppState;

use super::dev_check::{
    RulesCheckError, RulesCheckKit, RulesCheckReport, RulesCheckRequest, RulesRollReport,
    audit_line, refusal_audit_line, roll_audit_line, roll_refusal_audit_line,
};

#[derive(Clone)]
struct CheckRoute {
    app: Arc<AppState>,
    kit: Option<Arc<RulesCheckKit>>,
}

fn readable(bytes: Result<Bytes, BytesRejection>) -> Result<Bytes, RulesCheckError> {
    bytes.map_err(|rejection| RulesCheckError::UnreadableBody {
        cause: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            UnreadableBodyCause::TooLarge
        } else {
            UnreadableBodyCause::Interrupted
        },
    })
}

/// Reads the request and runs the check. The kit's work is short, in-memory rules arithmetic and
/// recorded draws, so it runs on the request's task.
fn run_check(
    route: &CheckRoute,
    bytes: Result<Bytes, BytesRejection>,
) -> Result<RulesCheckReport, RulesCheckError> {
    let request: RulesCheckRequest = parse_body(&readable(bytes)?)?;
    let kit = route.kit.as_ref().ok_or(RulesCheckError::ScriptedBackend)?;
    let report = kit.run(&request)?;
    route.app.record_line(audit_line(&request, &report));
    Ok(report)
}

/// Reads the request and rolls it, in the same way.
fn run_roll(
    route: &CheckRoute,
    bytes: Result<Bytes, BytesRejection>,
) -> Result<RulesRollReport, RulesCheckError> {
    let request: RollRequest = parse_body(&readable(bytes)?)?;
    let kit = route.kit.as_ref().ok_or(RulesCheckError::ScriptedBackend)?;
    let mut report = kit.roll(&request)?;
    let line = roll_audit_line(&request, &report);
    report.audit.push(line.clone());
    route.app.record_line(line);
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

async fn rules_roll(
    State(route): State<CheckRoute>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = run_roll(&route, bytes);
    if let Err(error) = &result {
        route.app.record_line(roll_refusal_audit_line(
            error,
            started.elapsed().as_millis(),
        ));
    }
    reply(result)
}

/// The routes of `rules check` and `rules roll`, answering from `kit` when there is one.
pub fn router(app: Arc<AppState>, kit: Option<Arc<RulesCheckKit>>) -> Router {
    Router::new()
        .route(path::RULES_CHECK, post(rules_check))
        .route(path::RULES_ROLL, post(rules_roll))
        .with_state(CheckRoute { app, kit })
}
