//! `dev place`: where the party stands, its exits and every place the run knows; `dev place go`
//! moves the party through the same path a hero's line takes (the place is created when the exit
//! leads somewhere the run does not have yet) or straight to a place the run has. A move is a
//! write: it is audited with the `PLACE change` line, told to the table, and marks the run edited.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use df_protocol::common as rpc;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Outcome, Shared, authorized, bad_request, failed};
use crate::gameplay::{
    model,
    place_move::{DevTarget, MoveError, resolve_target},
};

fn report(state: &Shared) -> Result<Value, (StatusCode, Json<Value>)> {
    state
        .service
        .place_report()
        .map_err(|status| failed(status.message()))
}

/// `GET /dev/place`: the current place, its exits (and where each leads) and the known places.
pub(super) async fn read(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let mut document = report(&state)?;
    if let Some(object) = document.as_object_mut() {
        object.remove("moves");
    }
    Ok((StatusCode::OK, Json(document)))
}

/// `GET /dev/place/list`: every known place with its people, and every committed move.
pub(super) async fn list(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let document = report(&state)?;
    Ok((
        StatusCode::OK,
        Json(json!({"known": document["known"], "moves": document["moves"]})),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GoBody {
    target: String,
}

/// `POST /dev/place/go {"target"}`: an exit of the current place (id, name or alias), else a place
/// the run has (id or name).
pub(super) async fn go(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<GoBody>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let target = body.target.trim();
    if target.is_empty() || target.chars().count() > 96 || target.chars().any(char::is_control) {
        return Err(bad_request(
            "place go needs an exit or a place, 1 to 96 characters",
        ));
    }
    let current = state.service.updates.borrow().clone();
    let resolved = resolve_target(&current, target).map_err(bad_request)?;
    let credential = {
        let seats = state.seats.lock().await;
        super::jumps::table_credential(&seats)?
    };
    let operation = loop {
        let bytes = model::random::<16>().map_err(|_| failed("no operation id available"))?;
        if let Ok(operation) = df_types::OperationId::from_bytes(&bytes) {
            break operation;
        }
    };
    let pre = super::jumps::pre_jump_moment(&state);
    let result = match &resolved {
        DevTarget::Exit(exit) => {
            state
                .service
                .move_party_through(credential, operation, exit)
                .await
        }
        DevTarget::Known(place) => {
            state
                .service
                .move_party_to_known(credential, operation, place)
                .await
        }
    };
    let moved = match result {
        Ok(moved) => moved,
        Err(MoveError::Refused(reason)) => {
            return Ok((
                StatusCode::OK,
                Json(json!({
                    "status": "refused",
                    "target": target,
                    "reason": reason.label(),
                    "edited": false,
                })),
            ));
        }
        Err(MoveError::Status(status)) => return Err(failed(status.message())),
    };
    state.table.push(
        "Narrator",
        rpc::TableRole::Narrator,
        0,
        rpc::TableKind::Action,
        &moved.line,
    );
    let revision = (moved.revision.epoch().get(), moved.revision.sequence());
    state.overrides.lock().await.push(json!({
        "kind": "place-go",
        "target": target,
        "before": moved.from,
        "after": moved.to,
        "revision": {"epoch": revision.0, "sequence": revision.1},
    }));
    if let Ok(mut guarded) = state.history.lock() {
        guarded.set_redo(pre, revision);
    }
    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "moved",
            "target": target,
            "from": moved.from,
            "to": moved.to,
            "place": moved.place_name,
            "exit": moved.exit,
            "created": moved.created,
            "line": moved.line,
            "narration": moved.narration,
            "revision": {"epoch": revision.0, "sequence": revision.1},
            "edited": true,
        })),
    ))
}
