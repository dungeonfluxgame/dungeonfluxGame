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
    place_move::{DevTarget, MoveError, MoveTarget, resolve_target},
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
    #[serde(default)]
    travel: bool,
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
    if target.is_empty() || target.chars().count() > 512 || target.chars().any(char::is_control) {
        return Err(bad_request(
            "place go/travel needs a destination, 1 to 512 characters",
        ));
    }
    let current = state.service.updates.borrow().clone();
    let resolved = resolve_move(&current, target, body.travel).map_err(bad_request)?;
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
    let pre_jump_entries = super::jumps::pre_jump_moment(&state);
    let result = state
        .service
        .move_party_toward(credential, operation, resolved, target)
        .await;
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
        guarded.set_redo(pre_jump_entries, revision);
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

fn resolve_move(
    current: &df_model::checkpoint::Checkpoint,
    target: &str,
    travel: bool,
) -> Result<MoveTarget, String> {
    if travel {
        return Ok(MoveTarget::New);
    }
    if let Some(name) = target.strip_prefix("landmark:") {
        let scene = crate::gameplay::beat_script_runtime::scene(current.state())
            .map_err(|_| "current scene unavailable")?;
        let here = crate::gameplay::place::whereabouts(current.state(), &scene)
            .map_err(|_| "current place unavailable")?
            .here;
        return crate::gameplay::place::landmarks_here(current.state(), &here)
            .map_err(|_| "landmarks unavailable".to_owned())?
            .into_iter()
            .find(|landmark| landmark.name.eq_ignore_ascii_case(name.trim()))
            .map(MoveTarget::Landmark)
            .ok_or_else(|| format!("no current landmark matches {name:?}"));
    }
    match resolve_target(current, target)? {
        DevTarget::Exit(exit) => Ok(MoveTarget::Exit(exit)),
        DevTarget::Known(place) => Ok(MoveTarget::Known(place)),
    }
}
