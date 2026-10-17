//! `dev music`: the server's current music plan for the scene on screen, as the display receives
//! it. This build has no place music spec and no generated-track cache, so the document says the
//! track is not built; `regenerate` and `clear` answer unavailable until those exist.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use df_protocol::common as rpc;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{Outcome, Shared, authorized, write_unavailable};

/// The music document around one plan. `plan` is null when no scene is on screen.
fn music_document(plan: Option<Value>) -> Value {
    json!({
        "plan": plan,
        "track_status": "not_built",
        "scope": "the current music plan the display receives; no place music spec or generated-track cache exists in this build",
    })
}

/// `GET /dev/music`: the current plan for the scene on screen.
pub(super) async fn read(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let (_, view) = super::display_view(&state).await?;
    let plan = match view.audience {
        Some(rpc::view_message::Audience::Display(display)) => display
            .scene
            .as_ref()
            .zip(display.journey.as_ref())
            .map(|(scene, journey)| super::reads::music_plan_summary(scene, journey)),
        _ => None,
    };
    Ok((StatusCode::OK, Json(music_document(plan))))
}

/// `POST /dev/music/edit`: no track is regenerated or cleared by this surface.
pub(super) async fn unsupported_edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Outcome {
    authorized(&headers, &state.token)?;
    write_unavailable(
        "music",
        "regenerate and clear need a place music spec and a generated-track cache, and neither exists in this build",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_read_passes_the_current_plan_through_and_says_no_track_is_built() {
        let shown = music_document(Some(json!({"phase": "anticipation", "intensity": 700})));
        assert_eq!(shown["plan"]["phase"], "anticipation");
        assert_eq!(shown["track_status"], "not_built");

        let idle = music_document(None);
        assert!(idle["plan"].is_null());
    }
}
