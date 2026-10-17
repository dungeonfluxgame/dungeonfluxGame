//! `dev fx`: the TV's screen-space effects and whether each is on; `dev fx <effect> on|off`
//! switches one for every display (it reaches a display within its next view, at most 5 s).

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::super::display_effects;
use super::{Outcome, Shared, authorized, bad_request};

fn document() -> Value {
    let effects: Vec<Value> = display_effects::states()
        .into_iter()
        .map(|(effect, on)| json!({"effect": effect, "on": on}))
        .collect();
    json!({"effects": effects})
}

/// `GET /dev/fx`: every effect and whether it is on.
pub(super) async fn read(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((StatusCode::OK, Json(document())))
}

#[derive(Deserialize)]
pub(super) struct EditRequest {
    effect: String,
    on: bool,
}

/// `POST /dev/fx/edit {effect, on}`: switches one effect for every display.
pub(super) async fn edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<EditRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    display_effects::set(&body.effect, body.on).map_err(|_| {
        bad_request(format!(
            "effect must be one of {}",
            display_effects::EFFECTS.join(", ")
        ))
    })?;
    Ok((StatusCode::OK, Json(document())))
}
