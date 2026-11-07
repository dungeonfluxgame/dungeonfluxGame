//! The world routes. Each runs its file work on the blocking pool and answers a `WorldReply`; a
//! refusal is a typed [`WorldError`] and one `WORLD refused verb=.. error=<key>` audit line, and a
//! change of the active place is `WORLD place place=<id|none>`.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use df_content::world_bible::{WorldBible, is_valid_id};

use super::facts::place_facts;
use super::request::{
    Listed, PlaceRequest, PlaceSet, QuoteQuery, Quoted, Validated, WorldError, WorldView,
};
use super::store::{read_bible, read_place, world_directory, write_place};
use crate::blocking::run_blocking;
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;

/// The addresses of the world dev API.
pub mod path {
    /// The bible's contents and the active place (GET).
    pub const WORLD: &str = "/dev/world";
    /// Sets or clears the active place (POST a `PlaceRequest`).
    pub const WORLD_PLACE: &str = "/dev/world/place";
    /// The price quote of an item at the active place (GET with `?item=&qty=`).
    pub const WORLD_QUOTE: &str = "/dev/world/quote";
    /// Reads the bible now and counts what it holds (GET).
    pub const WORLD_VALIDATE: &str = "/dev/world/validate";
}

/// Answers `result`, recording a refusal first as `WORLD refused verb=.. error=<key>`.
fn answer<T: serde::Serialize>(
    app: &AppState,
    verb: &str,
    result: Result<T, WorldError>,
) -> Response {
    if let Err(error) = &result {
        app.record_line(format!("WORLD refused verb={verb} error={}", error.key()));
    }
    reply::<T, WorldError>(result)
}

/// The bible's contents and the active place with its facts.
pub(super) fn view(directory: &Path) -> Result<WorldView, WorldError> {
    let bible = read_bible(directory)?;
    let place = read_place(directory)?;
    let facts = place
        .as_deref()
        .and_then(|id| bible.place(id))
        .map_or(serde_json::Value::Null, |place| {
            place_facts(&bible, place, None)
        });
    Ok(WorldView {
        place,
        items: bible.items.iter().map(|item| item.id.clone()).collect(),
        places: bible
            .places
            .iter()
            .map(|place| Listed {
                id: place.id.clone(),
                text: place.name.clone(),
            })
            .collect(),
        voice_packs: bible
            .voice_packs
            .iter()
            .map(|pack| Listed {
                id: pack.id.clone(),
                text: pack.description.clone(),
            })
            .collect(),
        facts,
    })
}

/// Makes `place` the active one after checking the bible holds it; none clears it.
pub(super) fn set_place(directory: &Path, place: Option<String>) -> Result<PlaceSet, WorldError> {
    if let Some(id) = &place
        && (!is_valid_id(id) || read_bible(directory)?.place(id).is_none())
    {
        return Err(WorldError::UnknownPlace { id: id.clone() });
    }
    write_place(directory, place.as_deref())?;
    Ok(PlaceSet { place })
}

/// The quote of `query.item` at the active place.
pub(super) fn quote_here(directory: &Path, query: &QuoteQuery) -> Result<Quoted, WorldError> {
    let place = read_place(directory)?.ok_or(WorldError::NoActivePlace)?;
    let quote = read_bible(directory)?
        .quote_at(&place, &query.item, query.qty)
        .map_err(|cause| WorldError::Quote { cause })?;
    Ok(Quoted {
        place,
        item: query.item.clone(),
        quote,
    })
}

fn counted(bible: &WorldBible) -> Validated {
    Validated {
        items: bible.items.len(),
        places: bible.places.len(),
        stalls: bible.places.iter().map(|place| place.stalls.len()).sum(),
        voice_packs: bible.voice_packs.len(),
    }
}

async fn show(State(app): State<Arc<AppState>>) -> Response {
    let directory = world_directory(&app);
    let result = run_blocking(move || view(&directory), WorldError::Stopped).await;
    answer(&app, "show", result)
}

async fn place(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: PlaceRequest = parse_body(&bytes)?;
        let directory = world_directory(&app);
        run_blocking(
            move || set_place(&directory, request.place),
            WorldError::Stopped,
        )
        .await
    }
    .await;
    if let Ok(set) = &result {
        app.record_line(format!(
            "WORLD place place={}",
            set.place.as_deref().unwrap_or("none")
        ));
    }
    answer(&app, "place", result)
}

async fn quote(
    State(app): State<Arc<AppState>>,
    query: Result<Query<QuoteQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let Query(query) = query.map_err(|rejection| WorldError::InvalidQuery {
            detail: rejection.body_text(),
        })?;
        let directory = world_directory(&app);
        run_blocking(move || quote_here(&directory, &query), WorldError::Stopped).await
    }
    .await;
    answer(&app, "quote", result)
}

async fn validate(State(app): State<Arc<AppState>>) -> Response {
    let directory = world_directory(&app);
    let result = run_blocking(
        move || read_bible(&directory).map(|bible| counted(&bible)),
        WorldError::Stopped,
    )
    .await;
    answer(&app, "validate", result)
}

/// The world routes.
pub fn router_with(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::WORLD, get(show))
        .route(path::WORLD_PLACE, post(place))
        .route(path::WORLD_QUOTE, get(quote))
        .route(path::WORLD_VALIDATE, get(validate))
        .with_state(app)
}
