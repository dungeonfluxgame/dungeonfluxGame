//! `dev memory`: the story memory of this run. Reads: `status`, `search` (the same recall path the
//! generators use), `entity`, `event`. Maintenance: `supersede`, `reindex`, `delete-run-index`
//! and `erase`. Nothing here prints an embedding vector or a key; an answer that has no memory to
//! read says so instead of failing.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

use super::{Outcome, Shared, authorized, bad_request, write_unavailable};
use crate::gameplay::memory::{self, Audience, EventKind, StoryMemory};

fn current() -> Result<Arc<StoryMemory>, Outcome> {
    memory::current().ok_or_else(|| {
        write_unavailable(
            "memory",
            "the story memory is off for this run: see the MEMORY start-up line for the reason",
        )
    })
}

/// `--audience public|party|member:<hero>`; party when absent.
pub(super) fn parse_audience(text: Option<&String>) -> Result<Audience, String> {
    match text {
        None => Ok(Audience::Party),
        Some(text) => Audience::parse(text.trim().to_lowercase().as_str())
            .ok_or_else(|| format!("audience {text:?} must be public, party or member:<hero>")),
    }
}

/// A `--kind` against the closed event kinds.
pub(super) fn parse_kind(text: &str) -> Result<EventKind, String> {
    EventKind::parse(text.trim().to_lowercase().as_str()).ok_or_else(|| {
        let known: Vec<&str> = EventKind::ALL.iter().map(|kind| kind.label()).collect();
        format!("kind {text:?} must be one of {}", known.join(", "))
    })
}

/// A `--limit` of related events: 1 to 32, 8 when absent.
pub(super) fn parse_limit(text: Option<&String>) -> Result<usize, String> {
    match text {
        None => Ok(8),
        Some(text) => match text.trim().parse::<usize>() {
            Ok(limit) if (1..=32).contains(&limit) => Ok(limit),
            _ => Err(format!("limit {text:?} must be 1 to 32")),
        },
    }
}

fn parse_id(text: Option<&String>, field_name: &str) -> Result<i64, String> {
    text.and_then(|text| text.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| format!("{field_name} must be an event id (a positive number)"))
}

fn json_ok(value: Value) -> Outcome {
    Ok((StatusCode::OK, Json(value)))
}

/// `GET /dev/memory`: counts by kind, the embedding backlog and model, the spend, the queue.
pub(super) async fn status(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    match current() {
        Ok(memory) => json_ok(memory.status_json().await),
        Err(unavailable) => unavailable,
    }
}

/// `GET /dev/memory/search?q=<text>[&entity=<id>][&kind=<kind>][&audience=<a>][&limit=<n>]`.
pub(super) async fn search(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let query = params
        .get("q")
        .filter(|query| !query.trim().is_empty())
        .ok_or_else(|| bad_request("memory search needs ?q=<text>"))?;
    let audience = parse_audience(params.get("audience")).map_err(bad_request)?;
    let kind = params
        .get("kind")
        .map(|kind| parse_kind(kind))
        .transpose()
        .map_err(bad_request)?;
    let limit = parse_limit(params.get("limit")).map_err(bad_request)?;
    match current() {
        Ok(memory) => json_ok(
            memory
                .search_json(
                    query,
                    params.get("entity").map(String::as_str),
                    kind,
                    audience,
                    limit,
                )
                .await,
        ),
        Err(unavailable) => unavailable,
    }
}

/// `GET /dev/memory/entity?id=<entity>[&audience=<a>][&superseded=true][&refresh=true]`.
pub(super) async fn entity(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let id = params
        .get("id")
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| bad_request("memory entity needs ?id=<entity id>"))?;
    let audience = parse_audience(params.get("audience")).map_err(bad_request)?;
    let is_requested = |name: &str| params.get(name).is_some_and(|value| value == "true");
    match current() {
        Ok(memory) => json_ok(
            memory
                .entity_json(
                    id,
                    audience,
                    is_requested("superseded"),
                    is_requested("refresh"),
                )
                .await,
        ),
        Err(unavailable) => unavailable,
    }
}

/// `GET /dev/memory/event?id=<n>`: one event whatever its audience, with its supersession links.
pub(super) async fn event(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let id = parse_id(params.get("id"), "id").map_err(bad_request)?;
    match current() {
        Ok(memory) => json_ok(memory.event_view_json(id).await),
        Err(unavailable) => unavailable,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Supersede {
    old: i64,
    by: i64,
}

/// `POST /dev/memory/supersede {"old", "by"}`: marks `old` superseded by `by`. Nothing is deleted.
pub(super) async fn supersede(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Supersede>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if body.old <= 0 || body.by <= 0 || body.old == body.by {
        return Err(bad_request("supersede needs two different event ids"));
    }
    match current() {
        Ok(memory) => match memory.supersede(body.old, body.by).await {
            Ok(changed) => json_ok(serde_json::json!({
                "status": if changed { "superseded" } else { "unchanged" },
                "old": body.old,
                "by": body.by,
                "deleted": false,
            })),
            Err(reason) => Err(bad_request(reason)),
        },
        Err(unavailable) => unavailable,
    }
}

/// `POST /dev/memory/reindex`: writes the rebuildable sources again and fills in the vectors.
pub(super) async fn reindex(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    match current() {
        Ok(memory) => json_ok(memory.reindex_json().await),
        Err(unavailable) => unavailable,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteIndex {
    #[serde(default)]
    all: bool,
}

/// `POST /dev/memory/delete-run-index {"all": bool}`: drops the rebuildable events (and with
/// `all`, every event and recall record of the run).
pub(super) async fn delete_run_index(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<DeleteIndex>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    match current() {
        Ok(memory) => json_ok(memory.delete_run_index_json(body.all).await),
        Err(unavailable) => unavailable,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Erase {
    hero: String,
}

/// `POST /dev/memory/erase {"hero"}`: erases one hero's own words and what was derived from them.
pub(super) async fn erase(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Erase>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if body.hero.trim().is_empty() {
        return Err(bad_request("erase needs a hero name"));
    }
    match current() {
        Ok(memory) => json_ok(memory.erase_hero_json(&body.hero).await),
        Err(unavailable) => unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audiences_kinds_and_limits_are_parsed_against_closed_sets() {
        assert_eq!(parse_audience(None), Ok(Audience::Party));
        assert_eq!(
            parse_audience(Some(&"PUBLIC".to_owned())),
            Ok(Audience::Public)
        );
        assert_eq!(
            parse_audience(Some(&"member:Brynn".to_owned())),
            Ok(Audience::Member("brynn".to_owned()))
        );
        assert!(parse_audience(Some(&"everyone".to_owned())).is_err());
        assert_eq!(parse_kind("Promise_Thread"), Ok(EventKind::PromiseThread));
        assert!(parse_kind("gossip").unwrap_err().contains("hero_speech"));
        assert_eq!(parse_limit(None), Ok(8));
        assert_eq!(parse_limit(Some(&"32".to_owned())), Ok(32));
        assert!(parse_limit(Some(&"33".to_owned())).is_err());
        assert!(parse_limit(Some(&"0".to_owned())).is_err());
        assert_eq!(parse_id(Some(&" 12 ".to_owned()), "id"), Ok(12));
        assert!(parse_id(Some(&"-3".to_owned()), "id").is_err());
        assert!(parse_id(None, "id").is_err());
    }
}
