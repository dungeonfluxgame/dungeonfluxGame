//! The world routes. Each runs its file work on the blocking pool and answers a `WorldReply`; a
//! refusal is a typed [`WorldError`] and one `WORLD refused verb=.. error=<key>` audit line, and a
//! change of the active place is `WORLD place place=<id|none>`, or `WORLD arrived place=<id>
//! things=<n>` for a place designed in play. A thing's kept close-up is `WORLD thing_picture
//! place=.. item=..`. The town is `town`; the inventory routes are `inventory::routes`.

use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use df_content::world_bible::{Place, WorldBible, is_valid_id};
use df_playground::scene::SceneChange;
use df_table::ports::Clock;

use super::facts::place_facts;
use super::inventory::stock_shop;
use super::request::{
    ArriveRequest, Listed, PlaceRequest, PlaceSet, QuoteQuery, Quoted, ThingAdd, ThingNote,
    ThingPicture, Validated, WorldError, WorldView,
};
use super::store::{
    keep_thing_picture, read_active_place, read_bible, read_place, read_town, revisit,
    world_directory, write_designed, write_place,
};
use super::town::TownView;
use super::walk_note::{WalkNoted, WalkStageNote};
use crate::blocking::run_blocking;
use crate::dev_http::{parse_body, read_query, reply};
use crate::observe::WallClock;
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
    /// Adds a thing in sight to the active place designed in play (POST a `ThingAdd`).
    pub const WORLD_THING_ADD: &str = "/dev/world/things/add";
    /// Notes a thing of the active place (POST a `ThingNote`).
    pub const WORLD_THING_NOTE: &str = "/dev/world/things/note";
    /// The price quote of an item at the active place (GET with `?item=&qty=`).
    pub const WORLD_QUOTE: &str = "/dev/world/quote";
    /// Reads the bible now and counts what it holds (GET).
    pub const WORLD_VALIDATE: &str = "/dev/world/validate";
    /// The town the party is in, if any (GET).
    pub const WORLD_TOWN: &str = "/dev/world/town";
    /// Notes a stage of a move's walk in the audit log (POST a `WalkStageNote`).
    pub const WORLD_WALK: &str = "/dev/world/walk";
    /// The quests offered and taken (GET).
    pub const WORLD_QUESTS: &str = "/dev/world/quests";
    /// Puts quests on the board (POST a `QuestOffer`).
    pub const WORLD_QUEST_OFFER: &str = "/dev/world/quests/offer";
    /// The party takes an offered quest (POST a `QuestAccept`).
    pub const WORLD_QUEST_ACCEPT: &str = "/dev/world/quests/accept";
    /// The stop the party reached on the quest's way (POST a `QuestProgress`).
    pub const WORLD_QUEST_PROGRESS: &str = "/dev/world/quests/progress";
    /// The quest is done and its reward is paid (POST a `QuestComplete`).
    pub const WORLD_QUEST_COMPLETE: &str = "/dev/world/quests/complete";
    /// The region the party is in (GET), or a crossing into another (POST a `Region`).
    pub const WORLD_REGION: &str = "/dev/world/region";
    /// Takes the crossing's title card off the shared screen (POST, no body).
    pub const WORLD_REGION_CARD_CLEAR: &str = "/dev/world/region/card/clear";
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
        // A place designed in play the party has been to is gone back to as it was left, in the
        // same town; the places visited stay.
        if is_valid_id(id) && revisit(directory, id)?.is_some() {
            return Ok(PlaceSet { place });
        }
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

/// How long the TV's party strip shows after the party reaches a new place, before it fades out
/// (owner request 2026-11-16).
pub(crate) const PARTY_STRIP_SHOWN: Duration = Duration::from_secs(15);

/// The party reached `place`, a new place on the screen: the TV's party strip shows
/// (`SceneChange::PartyHidden`), and fades out `PARTY_STRIP_SHOWN` later unless another new place
/// comes first, which shows it again and starts the wait anew. Nothing else brings it back. One
/// `PLAYGROUND party_strip shown place= at_ms=` line, and `PLAYGROUND party_strip hidden at_ms=
/// after_ms=` when it fades (at_ms on the wall clock), so the hold can be measured.
async fn show_party_strip(app: &Arc<AppState>, place: &str) {
    app.party_strip_rest().cancel();
    let shown = vec![SceneChange::PartyHidden { hidden: false }];
    // A refusal is recorded by the scene path.
    if crate::playground::apply_scene_changes(app, shown)
        .await
        .is_err()
    {
        return;
    }
    let shown_ms = WallClock.now_ms();
    app.record_line(format!(
        "PLAYGROUND party_strip shown place={} at_ms={shown_ms}",
        crate::world::logged_id(place)
    ));
    // The task holds the host weakly, so the host that owns the task can be dropped.
    let host: Weak<AppState> = Arc::downgrade(app);
    app.party_strip_rest().keep(tokio::spawn(async move {
        tokio::time::sleep(PARTY_STRIP_SHOWN).await;
        let Some(app) = host.upgrade() else {
            return;
        };
        let hidden = vec![SceneChange::PartyHidden { hidden: true }];
        if crate::playground::apply_scene_changes(&app, hidden)
            .await
            .is_ok()
        {
            let hidden_ms = WallClock.now_ms();
            app.record_line(format!(
                "PLAYGROUND party_strip hidden at_ms={hidden_ms} after_ms={}",
                hidden_ms.saturating_sub(shown_ms)
            ));
        }
    }));
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
        if let Some(place) = &set.place {
            show_party_strip(&app, place).await;
        }
    }
    answer(&app, "place", result)
}

async fn arrive(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let mut things = 0;
    let mut shops = Vec::new();
    let result = async {
        let request: ArriveRequest = parse_body(&bytes)?;
        things = request.place.things.len();
        shops = request.shops;
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
        show_party_strip(&app, id).await;
        // Terms put at the place left behind are not open at the new one.
        app.deals().leave(&app);
        // The place's traders have their stock from the party's first arrival on.
        let stocking = Arc::clone(&app);
        let stocked = run_blocking(
            move || {
                for shop in shops {
                    stock_shop(&stocking, shop);
                }
                Ok::<_, ()>(())
            },
            (),
        )
        .await;
        if stocked.is_err() {
            app.record_line("WORLD refused verb=inventory_stock error=stopped".to_owned());
        }
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

/// A thing the runner saw in the picture becomes a real thing of the place, with an id the look
/// path resolves (`world::add_things`, which writes its own lines).
async fn thing_add(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let ThingAdd { id, name, look } = parse_body(&bytes)?;
        let thing = df_content::world_bible::Thing {
            id,
            name,
            look,
            picture: None,
        };
        let place = super::put_thing(&app, "runner", thing).await?;
        Ok::<_, WorldError>(PlaceSet { place: Some(place) })
    }
    .await;
    reply(result)
}

/// A note on a thing of the active place (`world::note_thing`, which writes its own lines).
async fn thing_note(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let ThingNote {
            id,
            label,
            text,
            name,
        } = parse_body(&bytes)?;
        let place = super::note_thing(&app, &id, &label, (&text, name.as_deref())).await?;
        Ok::<_, WorldError>(PlaceSet { place: Some(place) })
    }
    .await;
    reply(result)
}

async fn quote(
    State(app): State<Arc<AppState>>,
    query: Result<Query<QuoteQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let query = read_query(query).map_err(WorldError::InvalidQuery)?;
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
        .route(path::WORLD_THING_ADD, post(thing_add))
        .route(path::WORLD_THING_NOTE, post(thing_note))
        .route(path::WORLD_QUOTE, get(quote))
        .route(path::WORLD_VALIDATE, get(validate))
        .route(path::WORLD_TOWN, get(town))
        .route(path::WORLD_WALK, post(walk))
        .with_state(Arc::clone(&app))
        .merge(super::memory::routes::router_with(Arc::clone(&app)))
        .merge(super::quest::router_with(Arc::clone(&app)))
        .merge(super::region::router_with(Arc::clone(&app)))
        .merge(super::inventory::routes::router_with(app))
}
