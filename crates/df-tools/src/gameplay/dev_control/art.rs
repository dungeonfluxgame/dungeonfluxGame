//! `dev art`: the generated depictions the hub holds (rooms, cast photographs, health faces,
//! plates), and clearing or regenerating one subject's depiction. A cleared place is made again
//! the next time a view needs it; a cleared character is requested again at once.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use df_render::art_contract::{Subject, SubjectKind};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{Outcome, Shared, authorized, bad_request};

#[derive(Deserialize)]
pub(super) struct ListQuery {
    #[serde(default)]
    subject: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct EditRequest {
    op: String,
    subject: String,
}

/// `character:<id>` or `place:<id>`, the form `Subject::key` writes.
fn parse_subject(text: &str) -> Option<Subject> {
    let (kind, id) = text.split_once(':')?;
    if id.is_empty() || id.chars().any(char::is_control) || id.chars().count() > 200 {
        return None;
    }
    match kind {
        "character" => Some(Subject::character(id)),
        "place" => Some(Subject::place(id)),
        _ => None,
    }
}

/// `GET /dev/art[?subject=…]`: every hub resource, or only those whose subject starts with the
/// given text, with state, handle, attempts and last error.
pub(super) async fn list(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let filter = query.subject.unwrap_or_default();
    let rows: Vec<Value> = state
        .service
        .living
        .list_resources(None)
        .into_iter()
        .filter(|row| row.subject.starts_with(&filter))
        .map(|row| {
            json!({
                "key": row.key.as_str(),
                "kind": row.kind.label(),
                "subject": row.subject,
                "state": row.state,
                "handle": row.handle,
                "attempts": row.attempts,
                "last_error": row.last_error,
            })
        })
        .collect();
    Ok((
        StatusCode::OK,
        Json(json!({"count": rows.len(), "resources": rows})),
    ))
}

/// `POST /dev/art/edit {op: clear|regenerate, subject: character:<id>|place:<id>}`.
pub(super) async fn edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<EditRequest>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let subject = parse_subject(&body.subject)
        .ok_or_else(|| bad_request("subject must be character:<id> or place:<id>"))?;
    let living = &state.service.living;
    let document = match body.op.as_str() {
        "clear" => json!({
            "status": "cleared",
            "subject": subject.key(),
            "entries": living.clear_depiction(&subject),
            "next": "made again the next time a view needs it",
        }),
        "regenerate" => match subject.kind {
            SubjectKind::Character => json!({
                "status": if living.regenerate_depiction(&subject) { "requested" } else { "not_in_cast" },
                "subject": subject.key(),
            }),
            // A place is generated when a view needs it: clearing it makes the next view ask.
            SubjectKind::Place => json!({
                "status": "cleared",
                "subject": subject.key(),
                "entries": living.clear_depiction(&subject),
                "next": "requested again by the next view of that place",
            }),
        },
        _ => return Err(bad_request("art op must be clear or regenerate")),
    };
    Ok((StatusCode::OK, Json(document)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_parse_only_in_the_key_form_the_hub_writes() {
        assert_eq!(
            parse_subject("place:place-mareks-cloth-shop").map(|s| s.key()),
            Some("place:place-mareks-cloth-shop".to_owned())
        );
        assert_eq!(
            parse_subject("character:merchant").map(|s| s.key()),
            Some("character:merchant".to_owned())
        );
        assert!(parse_subject("merchant").is_none());
        assert!(parse_subject("prop:crates").is_none());
        assert!(parse_subject("place:").is_none());
    }
}
