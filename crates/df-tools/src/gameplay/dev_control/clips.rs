//! Read and explicitly request the character-clip hub resources.
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::json;

use super::{Outcome, Shared, authorized, bad_request, failed};
use crate::gameplay::character_clips::{self, ClipKind};

#[derive(Deserialize)]
pub(super) struct ListQuery {
    name: String,
}

#[derive(Deserialize)]
pub(super) struct Request {
    name: String,
    #[serde(default)]
    kind: Option<String>,
}

fn requested_kind(kind: Option<&str>) -> Result<Option<ClipKind>, &'static str> {
    kind.map(|kind| ClipKind::parse(kind).ok_or("kind must be idle, attack or last-hit"))
        .transpose()
}

pub(super) async fn list(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let service =
        character_clips::service().ok_or_else(|| failed("character clips service unavailable"))?;
    let fighter = service
        .find(state.service.updates.borrow().state(), &query.name)
        .map_err(bad_request)?;
    Ok((StatusCode::OK, Json(service.list(&fighter))))
}

pub(super) async fn request(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Request>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let kind = requested_kind(body.kind.as_deref()).map_err(bad_request)?;
    let service =
        character_clips::service().ok_or_else(|| failed("character clips service unavailable"))?;
    let fighter = service
        .find(state.service.updates.borrow().state(), &body.name)
        .map_err(bad_request)?;
    service.request(fighter.clone(), kind, true);
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "status": "requested", "identity": fighter.identity, "name": fighter.name,
            "kind": kind, "fallback": "existing cutout motion",
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_commands_accept_only_the_three_kinds_and_absent_means_all() {
        assert_eq!(requested_kind(None), Ok(None));
        for (text, kind) in [
            ("idle", ClipKind::Idle),
            ("attack", ClipKind::Attack),
            ("last-hit", ClipKind::LastHit),
        ] {
            assert_eq!(requested_kind(Some(text)), Ok(Some(kind)));
        }
        assert!(requested_kind(Some("last_hit")).is_err());
        assert!(requested_kind(Some("")).is_err());
    }
}
