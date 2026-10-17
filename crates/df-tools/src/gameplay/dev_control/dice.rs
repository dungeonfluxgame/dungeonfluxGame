//! `dev dice`: the forced-face queue for the next server-side die draws. `dev dice next <face…>
//! --die d20` appends faces to that die size's first-in, first-out queue; `dev dice clear` empties
//! every queue. Each forced face is written to the audit log as it is used.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::super::dice as forced_dice;
use super::{Outcome, Shared, authorized, bad_request};

fn document() -> Value {
    let queue: Vec<Value> = forced_dice::snapshot()
        .into_iter()
        .map(|(sides, faces)| json!({"die": format!("d{sides}"), "sides": sides, "faces": faces}))
        .collect();
    json!({"queue": queue})
}

/// `GET /dev/dice`: the faces waiting for each die size, in the order they will be used.
pub(super) async fn read(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((StatusCode::OK, Json(document())))
}

#[derive(Deserialize)]
pub(super) struct NextRequest {
    sides: u32,
    faces: Vec<u32>,
}

/// `POST /dev/dice/next {sides, faces}`: appends the faces in order. A face outside 1..=sides
/// refuses the whole request, so nothing is queued.
pub(super) async fn next(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<NextRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if body.faces.is_empty() {
        return Err(bad_request("dice next needs at least one face"));
    }
    forced_dice::enqueue(body.sides, &body.faces).map_err(|error| bad_request(error.message()))?;
    Ok((StatusCode::OK, Json(document())))
}

/// `POST /dev/dice/clear`: empties the queue of every die size.
pub(super) async fn clear(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    forced_dice::clear();
    Ok((StatusCode::OK, Json(document())))
}
