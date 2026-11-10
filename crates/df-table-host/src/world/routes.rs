//! The world routes. Each runs its file work on the blocking pool and answers a `WorldReply`; a
//! refusal is a typed [`WorldError`] and one `WORLD refused verb=.. error=<key>` audit line, and a
//! change of the active place is `WORLD place place=<id|none>`, or `WORLD arrived place=<id>
//! things=<n>` for a place designed in play. A thing's kept close-up is `WORLD thing_picture
//! place=.. item=..`. The town is `town`; the inventory routes are `inventory::routes`.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use df_content::world_bible::{Place, WorldBible, is_valid_id};

use super::facts::place_facts;
use super::request::{
    ArriveRequest, Listed, PlaceRequest, PlaceSet, QuoteQuery, Quoted, ThingPicture, Validated,
    WorldError, WorldView,
};
use super::store::{
    keep_thing_picture, read_active_place, read_bible, read_place, read_town, world_directory,
    write_designed, write_place,
};
use super::town::TownView;
use super::walk_note::{WalkNoted, WalkStageNote};
use crate::blocking::run_blocking;
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;

/// The addresses of the world dev API.
pub mod path {
    /// The bible's contents and the active place (GET).
    pub const WORLD: &str = "/dev/world";
    /// Sets or clears the active place (POST a `PlaceRequest`).
    pub const WORLD_PLACE: &str = "/dev/world/place";
    /// Makes a place designed in play the active one (POST an `ArriveRequest`).
    pub const WORLD_ARRIVE: &str = "/dev/world/arrive";
    /// Keeps a close-up as a thing's reference sheet (POST a `ThingPicture`).
    pub const WORLD_PICTURE: &str = "/dev/world/picture";
    /// The price quote of an item at the active place (GET with `?item=&qty=`).
    pub const WORLD_QUOTE: &str = "/dev/world/quote";
    /// Reads the bible now and counts what it holds (GET).
    pub const WORLD_VALIDATE: &str = "/dev/world/validate";
    /// The town the party is in, if any (GET).
    pub const WORLD_TOWN: &str = "/dev/world/town";
    /// Notes a stage of a move's walk in the audit log (POST a `WalkStageNote`).
    pub const WORLD_WALK: &str = "/dev/world/walk";
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
    let facts = read_active_place(directory, &bible)?.map_or(serde_json::Value::Null, |place| {
        place_facts(&bible, &place, None)
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

/// Makes `place`, designed in play, the active one after checking it as the bible's places are.
pub(super) fn arrive_at(
    directory: &Path,
    place: Place,
    feature: Option<String>,
) -> Result<PlaceSet, WorldError> {
    read_bible(directory)?
        .check_place(&place)
        .map_err(|cause| WorldError::PlaceInvalid { cause })?;
    let id = place.id.clone();
    write_designed(directory, place, feature)?;
    Ok(PlaceSet { place: Some(id) })
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

async fn arrive(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let mut things = 0;
    let result = async {
        let request: ArriveRequest = parse_body(&bytes)?;
        things = request.place.things.len();
        let directory = world_directory(&app);
        run_blocking(
            move || arrive_at(&directory, request.place, request.feature),
            WorldError::Stopped,
        )
        .await
    }
    .await;
    if let Ok(PlaceSet { place: Some(id) }) = &result {
        app.record_line(format!("WORLD arrived place={id} things={things}"));
    }
    answer(&app, "arrive", result)
}

async fn picture(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: ThingPicture = parse_body(&bytes)?;
        let directory = world_directory(&app);
        run_blocking(
            move || {
                keep_thing_picture(&directory, &request.item, &request.picture)
                    .map(|place| (place, request.item))
            },
            WorldError::Stopped,
        )
        .await
    }
    .await;
    if let Ok((place, item)) = &result {
        app.record_line(format!("WORLD thing_picture place={place} item={item}"));
    }
    answer(
        &app,
        "picture",
        result.map(|(place, _)| PlaceSet { place: Some(place) }),
    )
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

async fn town(State(app): State<Arc<AppState>>) -> Response {
    let directory = world_directory(&app);
    let result = run_blocking(
        move || read_town(&directory).map(|town| TownView { town }),
        WorldError::Stopped,
    )
    .await;
    answer(&app, "town", result)
}

async fn walk(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = parse_body::<WalkStageNote>(&bytes)
        .map_err(WorldError::from)
        .and_then(|note| {
            let invalid = [&note.place, &note.stage]
                .into_iter()
                .find(|id| !is_valid_id(id));
            match invalid {
                Some(id) => Err(WorldError::InvalidWalkStage { id: id.clone() }),
                None => Ok(note),
            }
        })
        .map(|note| {
            let at_ms = app.playground().clock_ms();
            app.record_line(format!(
                "WORLD walk_stage place={} stage={} number={} of={} outcome={} at_ms={at_ms} \
                 up_ms={}",
                note.place,
                note.stage,
                note.number,
                note.of,
                note.outcome.key(),
                note.up_ms
                    .map_or_else(|| "-".to_owned(), |up_ms| up_ms.to_string())
            ));
            WalkNoted { at_ms }
        });
    answer(&app, "walk", result)
}

/// The world routes.
pub fn router_with(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::WORLD, get(show))
        .route(path::WORLD_PLACE, post(place))
        .route(path::WORLD_ARRIVE, post(arrive))
        .route(path::WORLD_PICTURE, post(picture))
        .route(path::WORLD_QUOTE, get(quote))
        .route(path::WORLD_VALIDATE, get(validate))
        .route(path::WORLD_TOWN, get(town))
        .route(path::WORLD_WALK, post(walk))
        .with_state(Arc::clone(&app))
        .merge(super::memory::routes::router_with(Arc::clone(&app)))
        .merge(super::inventory::routes::router_with(app))
}
