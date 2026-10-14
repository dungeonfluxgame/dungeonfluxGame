//! Read and write the owner-held party notebook through the loopback development tool.
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use df_types::MemberId;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use super::{Outcome, Shared, authorized, bad_request, failed, resolve_hero};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AddNote {
    hero: String,
    text: String,
}

/// `GET /dev/notes?hero=<name>` returns the same audience-filtered entries as that hero's phone.
pub(super) async fn read(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let requested_hero = params
        .get("hero")
        .map(String::as_str)
        .ok_or_else(|| bad_request("notes needs ?hero=<name>"))?;
    let (hero, member) = {
        let seats = state.seats.lock().await;
        let index = resolve_hero(&seats, requested_hero).map_err(bad_request)?;
        let seat = &seats.seats[index];
        let hero = seat
            .hero
            .clone()
            .ok_or_else(|| bad_request("hero has not joined"))?;
        let member = seat
            .member
            .as_deref()
            .and_then(|bytes| MemberId::from_bytes(bytes).ok())
            .ok_or_else(|| bad_request("hero member binding invalid; rejoin the seat"))?;
        (hero, member)
    };
    let checkpoint = state.service.updates.borrow().clone();
    let mut view = super::super::wire::journey_view(
        &checkpoint,
        df_persistence::local_demo_scope::LocalDemoRole::Player,
        member,
    )
    .map_err(|_| failed("current hero view unavailable"))?;
    state.social.attach(&mut view, member);
    let standing = match view.audience {
        Some(df_protocol::common::view_message::Audience::Player(player)) => player
            .journey
            .map(|journey| journey.standing)
            .unwrap_or_default(),
        _ => return Err(failed("current hero view unavailable")),
    };
    let entries = state
        .notebook
        .visible_to(member)
        .into_iter()
        .map(|entry| {
            let attitude = (entry.kind == super::super::notebook::Kind::Person)
                .then(|| {
                    standing
                        .iter()
                        .find(|item| item.name.eq_ignore_ascii_case(&entry.title))
                        .and_then(|item| match item.attitude {
                            1 => Some("hostile"),
                            2 => Some("indifferent"),
                            3 => Some("friendly"),
                            _ => None,
                        })
                })
                .flatten();
            json!({
                "kind": entry.kind.label(),
                "title": entry.title,
                "text": entry.text,
                "scene": entry.scene,
                "order": entry.order,
                "owner": entry.owner,
                "author": entry.author,
                "attitude": attitude,
            })
        })
        .collect::<Vec<Value>>();
    Ok((
        StatusCode::OK,
        Json(json!({"hero": hero, "entries": entries})),
    ))
}

/// `POST /dev/note {"hero": "Brakka", "text": "..."}` adds a shared party note.
pub(super) async fn add(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<AddNote>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let (hero, member) = {
        let seats = state.seats.lock().await;
        let index = resolve_hero(&seats, &body.hero).map_err(bad_request)?;
        let seat = &seats.seats[index];
        let hero = seat
            .hero
            .clone()
            .ok_or_else(|| bad_request("hero has not joined"))?;
        let member = seat
            .member
            .as_deref()
            .and_then(|bytes| MemberId::from_bytes(bytes).ok())
            .ok_or_else(|| bad_request("hero member binding invalid; rejoin the seat"))?;
        (hero, member)
    };
    let checkpoint = state.service.updates.borrow().clone();
    let scene_title = super::super::beat_script_runtime::scene(checkpoint.state())
        .map_err(|_| failed("current scene unavailable"))?
        .title;
    let order = state
        .notebook
        .add_note(member, &hero, &body.text, &scene_title)
        .map_err(|error| match error {
            super::super::notebook::NotebookError::Bound("text") => {
                bad_request("text must be 1 to 4096 characters")
            }
            super::super::notebook::NotebookError::Io(_) => failed("notebook state unavailable"),
            _ => bad_request("party note rejected"),
        })?;
    state.table.push(
        "table",
        df_protocol::common::TableRole::Narrator,
        0,
        df_protocol::common::TableKind::Action,
        &format!("NOTE added kind=note title=\"Party note\" scene={scene_title}"),
    );
    Ok((
        StatusCode::OK,
        Json(json!({"status": "added", "hero": hero, "scene": scene_title, "order": order})),
    ))
}
