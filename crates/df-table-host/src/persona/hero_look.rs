//! The look in words of a character that is a seated hero (`hero-<id>`, or the persona a seat
//! plays): the whole public look of the hero's locked design as the look studio words it
//! (`LookStudio::locked_look`: species and class, each control, each detail such as an aura or a
//! scar). Every plate that draws such a character takes it as the character's look in its brief
//! (the move plate, the walk's sights, a declared action), in place of the persona's job. Never a
//! name.
//!
//! `POST /dev/persona/looks` answers the looks of the characters asked for, by id; a character who
//! is no seated hero, or whose hero has no locked design, has none. Audit lines: `PERSONA
//! hero_look id= chars=` for each look given, `PERSONA hero_look_unavailable id= error=` when the
//! seat books or the look studio cannot be read.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use serde::{Deserialize, Serialize};

use super::request::PersonaError;
use super::routes::{PersonaRoutes, path};
use crate::dev::DevError;
use crate::dev_client::post_json;
use crate::dev_http::{DevReply, parse_body, reply};
use crate::seat::book::HeroRecord;
use crate::seat::request::SeatError;
use crate::state::AppState;

/// The body of `POST /dev/persona/looks`: the characters whose looks a plate needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LooksRequest {
    pub ids: Vec<String>,
}

/// The look in words of `id` when it is a seated hero with a locked design: the seat whose hero's
/// character id (`hero-<id>`) or played persona is `id`. None for anyone else.
pub(crate) async fn hero_look(app: &AppState, id: &str) -> Option<String> {
    let unavailable = |key: &str| {
        app.record_line(format!(
            "PERSONA hero_look_unavailable id={} error={key}",
            crate::world::logged_id(id)
        ));
    };
    let hero = match seated_hero(app, id).await {
        Ok(hero) => hero?.1,
        Err(error) => {
            unavailable(error.key());
            return None;
        }
    };
    match crate::look::LookStudio::locked_look(app, &hero).await {
        Ok(look) => look,
        Err(error) => {
            unavailable(error.key());
            None
        }
    }
}

/// The seat that plays `id` and its hero, when `id` is a seated hero: the seat whose hero's
/// character id (`hero-<id>`) or played persona is `id`. None for anyone else.
pub(crate) async fn seated_hero(
    app: &AppState,
    id: &str,
) -> Result<Option<(u64, HeroRecord)>, SeatError> {
    let mut open = app.seats().open().await?;
    let books = open.books();
    let Some((seat, hero)) = books.records().iter().find_map(|record| {
        let hero = record.hero?;
        let plays =
            record.persona.as_deref() == Some(id) || crate::creation::rules::sheet_id(hero) == id;
        plays.then_some((record.seat, hero))
    }) else {
        return Ok(None);
    };
    Ok(books.hero(hero).cloned().map(|hero| (seat, hero)))
}

/// The looks of the seated heroes among `ids`, by id.
pub(super) async fn hero_looks(app: &AppState, ids: &[String]) -> BTreeMap<String, String> {
    let mut looks = BTreeMap::new();
    for id in ids {
        if let Some(look) = hero_look(app, id)
            .await
            .filter(|look| !look.trim().is_empty())
        {
            app.record_line(format!(
                "PERSONA hero_look id={} chars={}",
                crate::world::logged_id(id),
                look.chars().count()
            ));
            looks.insert(id.clone(), look);
        }
    }
    looks
}

pub(super) async fn looks(State(routes): State<Arc<PersonaRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<LooksRequest>(&bytes) {
        Ok(request) => Ok(hero_looks(&routes.app, &request.ids).await),
        Err(error) => Err(PersonaError::InvalidBody(error)),
    };
    reply(result)
}

/// The looks of the seated heroes among `ids`, by id, from the host on `port`: what a plate's brief
/// takes as their look (`LookSheets::from_looks`).
pub(crate) async fn hero_looks_from(
    port: u16,
    ids: Vec<String>,
) -> Result<BTreeMap<String, String>, DevError> {
    let reply: DevReply<BTreeMap<String, String>, PersonaError> =
        post_json(port, path::PERSONA_LOOKS, &LooksRequest { ids }).await?;
    crate::dev_client::reply_value(reply)
}
