//! Authenticated, audited local hero library inspection and removal.
use super::{Outcome, Shared, authorized, failed};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;
fn audit(action: &str, id: Option<i64>, status: &str) {
    super::super::actor::record_audit_line(
        "dev.heroes",
        format!("DEV heroes action={action} hero={id:?} status={status}"),
    );
}
fn authorize_command(
    headers: &HeaderMap,
    token: &str,
    action: &str,
    hero_id: Option<i64>,
) -> Result<(), (StatusCode, Json<Value>)> {
    if let Err(error) = authorized(headers, token) {
        audit(action, hero_id, "unauthorized");
        return Err(error);
    }
    if hero_id.is_some_and(|hero_id| hero_id <= 0) {
        audit(action, hero_id, "invalid_id");
        return Err(super::bad_request("Hero id must be positive"));
    }
    Ok(())
}
fn hero_metadata(row: &df_persistence::hero_library::HeroRevision) -> Value {
    json!({"id":row.reference.hero_id,"revision":row.reference.revision,"name":row.record.name,"species":row.record.species,"class":row.record.character_class,"design":{"species":row.record.species,"class":row.record.character_class},"portrait_bytes":row.record.portrait.len(),"portrait_media_type":row.record.portrait_media_type})
}
pub(super) async fn list(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorize_command(&headers, &state.token, "list", None)?;
    let result = async {
        super::super::hero_library::open()
            .await?
            .admin_list()
            .await
            .map_err(|_| tonic::Status::unavailable("Library unavailable"))
    }
    .await;
    audit(
        "list",
        None,
        if result.is_ok() { "ok" } else { "unavailable" },
    );
    let rows = result.map_err(|_| failed("Hero library unavailable"))?;
    Ok((
        StatusCode::OK,
        Json(json!({"heroes":rows.iter().map(hero_metadata).collect::<Vec<_>>()})),
    ))
}
async fn read(state: Arc<Shared>, headers: HeaderMap, id: i64, history: bool) -> Outcome {
    authorize_command(
        &headers,
        &state.token,
        if history { "revisions" } else { "show" },
        Some(id),
    )?;
    let result = async {
        super::super::hero_library::open()
            .await?
            .revisions(id)
            .await
            .map_err(|_| tonic::Status::unavailable("Library unavailable"))
    }
    .await;
    audit(
        if history { "revisions" } else { "show" },
        Some(id),
        if result.is_ok() { "ok" } else { "unavailable" },
    );
    let rows = result.map_err(|_| failed("Hero library unavailable"))?;
    if history {
        return Ok((
            StatusCode::OK,
            Json(json!({"revisions":rows.iter().map(hero_metadata).collect::<Vec<_>>()})),
        ));
    }
    let row = rows.first().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Hero unavailable"})),
        )
    })?;
    use prost::Message;
    let identity = df_protocol::common::LibraryHeroIdentity::decode(row.record.encoded.as_slice())
        .map_err(|_| failed("Hero record invalid"))?;
    let mut value = hero_metadata(row);
    value["public_look"] = json!(
        identity
            .identity
            .as_ref()
            .and_then(|identity| identity.appearance.as_ref())
            .map(|look| &look.public_description)
    );
    value["backstory"] = json!(
        identity
            .identity
            .as_ref()
            .map(|identity| &identity.backstory)
    );
    // Private/sealed prose remains excluded from routine diagnostic output.
    value["private_backstory_present"] = json!(!identity.private_backstory.is_empty());
    value["sealed_backstory_present"] = json!(!identity.sealed_backstory.is_empty());
    Ok((StatusCode::OK, Json(value)))
}
pub(super) async fn show(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Outcome {
    read(state, headers, id, false).await
}
pub(super) async fn revisions(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Outcome {
    read(state, headers, id, true).await
}
pub(super) async fn delete(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Outcome {
    authorize_command(&headers, &state.token, "delete", Some(id))?;
    let result = super::super::hero_library::delete_saved_hero(None, id).await;
    match &result {
        Ok(removed) => super::super::actor::record_audit_line(
            "dev.heroes",
            format!(
                "DEV_CONTROL DeleteHero hero_id={id} removed={removed} outcome={}",
                if *removed { "answered" } else { "not_found" },
            ),
        ),
        Err(_) => audit("delete", Some(id), "unavailable"),
    }
    Ok((
        StatusCode::OK,
        Json(json!({"deleted":result.map_err(|_|failed("Hero library unavailable"))?,"id":id})),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heroes_commands_require_the_dev_token_and_a_positive_id() {
        let mut headers = HeaderMap::new();
        for action in ["list", "show", "delete"] {
            assert_eq!(
                authorize_command(&headers, "synthetic-token", action, Some(42))
                    .unwrap_err()
                    .0,
                StatusCode::UNAUTHORIZED
            );
        }
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer synthetic-token".parse().unwrap(),
        );
        assert!(authorize_command(&headers, "synthetic-token", "list", None).is_ok());
        assert!(authorize_command(&headers, "synthetic-token", "show", Some(42)).is_ok());
        assert_eq!(
            authorize_command(&headers, "synthetic-token", "delete", Some(0))
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}
