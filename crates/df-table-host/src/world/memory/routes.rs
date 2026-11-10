//! The memory routes under `/dev/world/memory`. Each runs its file work on the blocking pool and
//! answers a `MemoryReply`; a refusal is a typed [`MemoryError`] and one `WORLD refused
//! verb=memory_<verb> error=<key>` audit line. A put or capture is one `WORLD memory_put owner=..
//! id=.. kind=.. place=.. seq=.. pictures=.. created=..` line, a delete one `WORLD memory_deleted
//! owner=.. id=.. kind=..` line and a compose one `WORLD memory_composed owner=.. picture=..
//! members=.. columns=.. rows=..` line: ids and counts only, never a title or a description, which
//! are words. The history route reads those lines back.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use serde::Serialize;

use super::operations::{
    Paths, capture, delete, get as get_memory, history, inspect, keep_members, list, list_owners,
    prepare_compose, put, references,
};
use super::request::{
    CaptureRequest, ComposeRequest, Composed, DeleteRequest, GetQuery, HistoryQuery, InspectQuery,
    ListQuery, MemoryError, ReferencesRequest,
};
use super::store::memory_directory;
use super::{MemoryDraft, Put};
use crate::blocking::run_blocking;
use crate::dev_http::{parse_body, reply};
use crate::media_library::{media_directory, store_bytes};
use crate::state::AppState;

/// The addresses of the memory dev API.
pub mod path {
    /// Every owner that keeps memories (GET).
    pub const MEMORY: &str = "/dev/world/memory";
    /// A page of one owner's memories (GET with `?owner=&kind=&place=&tag=&limit=&after=`).
    pub const MEMORY_LIST: &str = "/dev/world/memory/list";
    /// One memory (GET with `?owner=&id=`).
    pub const MEMORY_GET: &str = "/dev/world/memory/get";
    /// Creates or replaces a memory (POST a `MemoryDraft`).
    pub const MEMORY_PUT: &str = "/dev/world/memory/put";
    /// Makes a memory of an existing picture (POST a `CaptureRequest`).
    pub const MEMORY_CAPTURE: &str = "/dev/world/memory/capture";
    /// Deletes a memory, not its pictures (POST a `DeleteRequest`).
    pub const MEMORY_DELETE: &str = "/dev/world/memory/delete";
    /// The pictures to attach to a plate for a selection (POST a `ReferencesRequest`).
    pub const MEMORY_REFERENCES: &str = "/dev/world/memory/references";
    /// Tiles a selection into one gallery sheet picture (POST a `ComposeRequest`).
    pub const MEMORY_COMPOSE: &str = "/dev/world/memory/compose";
    /// Whether a picture of the media library is near blank (GET with `?picture=`).
    pub const MEMORY_INSPECT: &str = "/dev/world/memory/inspect";
    /// The memory lines of the host's log (GET with `?owner=&limit=`).
    pub const MEMORY_HISTORY: &str = "/dev/world/memory/history";
}

/// Records a refusal of `verb` as `WORLD refused verb=memory_<verb> error=<key>`.
fn record_refused(app: &AppState, verb: &str, error: &MemoryError) {
    app.record_line(format!(
        "WORLD refused verb=memory_{verb} error={}",
        error.key()
    ));
}

fn answer<T: Serialize>(app: &AppState, verb: &str, result: Result<T, MemoryError>) -> Response {
    if let Err(error) = &result {
        record_refused(app, verb, error);
    }
    reply::<T, MemoryError>(result)
}

pub(super) fn paths(app: &AppState) -> Paths {
    Paths {
        memory: memory_directory(app),
        media: media_directory(app.playground().files()),
    }
}

pub(super) fn record_put(app: &AppState, put: &Put) {
    let memory = &put.memory;
    app.record_line(format!(
        "WORLD memory_put owner={} id={} kind={} place={} seq={} pictures={} created={}",
        memory.owner,
        memory.id,
        memory.kind,
        memory.place.as_deref().unwrap_or("-"),
        memory.seq,
        memory.pictures.len(),
        put.created
    ));
}

async fn owners(State(app): State<Arc<AppState>>) -> Response {
    let directory = memory_directory(&app);
    let result = run_blocking(move || list_owners(&directory), MemoryError::Stopped).await;
    answer(&app, "list", result)
}

async fn listing(
    State(app): State<Arc<AppState>>,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let Query(query) = query.map_err(|rejection| MemoryError::InvalidQuery {
            detail: rejection.body_text(),
        })?;
        let directory = memory_directory(&app);
        run_blocking(move || list(&directory, &query), MemoryError::Stopped).await
    }
    .await;
    answer(&app, "list", result)
}

async fn one(
    State(app): State<Arc<AppState>>,
    query: Result<Query<GetQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let Query(query) = query.map_err(|rejection| MemoryError::InvalidQuery {
            detail: rejection.body_text(),
        })?;
        let directory = memory_directory(&app);
        run_blocking(move || get_memory(&directory, &query), MemoryError::Stopped).await
    }
    .await;
    answer(&app, "get", result)
}

async fn keep(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let draft: MemoryDraft = parse_body(&bytes)?;
        let paths = paths(&app);
        run_blocking(move || put(&paths, draft), MemoryError::Stopped).await
    }
    .await;
    if let Ok(put) = &result {
        record_put(&app, put);
    }
    answer(&app, "put", result)
}

async fn snap(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: CaptureRequest = parse_body(&bytes)?;
        let paths = paths(&app);
        run_blocking(move || capture(&paths, request), MemoryError::Stopped).await
    }
    .await;
    if let Ok(put) = &result {
        record_put(&app, put);
    }
    answer(&app, "capture", result)
}

async fn remove(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: DeleteRequest = parse_body(&bytes)?;
        let directory = memory_directory(&app);
        run_blocking(
            move || delete(&directory, &request.owner, &request.id),
            MemoryError::Stopped,
        )
        .await
    }
    .await;
    if let Ok(deleted) = &result {
        let memory = &deleted.memory;
        app.record_line(format!(
            "WORLD memory_deleted owner={} id={} kind={}",
            memory.owner, memory.id, memory.kind
        ));
    }
    answer(&app, "delete", result)
}

async fn attach(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: ReferencesRequest = parse_body(&bytes)?;
        let directory = memory_directory(&app);
        run_blocking(
            move || references(&directory, &request),
            MemoryError::Stopped,
        )
        .await
    }
    .await;
    answer(&app, "references", result)
}

async fn compose(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: ComposeRequest = parse_body(&bytes)?;
        let paths = paths(&app);
        let (media, directory) = (paths.media.clone(), paths.memory.clone());
        let prepared = run_blocking(
            move || prepare_compose(&paths, &request),
            MemoryError::Stopped,
        )
        .await?;
        store_bytes(media, &format!("{}.png", prepared.name), &prepared.sheet)
            .await
            .map_err(|failure| MemoryError::PictureNotStored {
                detail: failure.to_string(),
            })?;
        run_blocking(
            move || keep_members(&directory, &prepared).map(|()| prepared),
            MemoryError::Stopped,
        )
        .await
        .map(|prepared| Composed {
            picture: format!("{}.png", prepared.name),
            owner: prepared.owner,
            members: prepared.members,
            pictures: prepared.pictures,
            columns: prepared.columns,
            rows: prepared.rows,
        })
    }
    .await;
    if let Ok(composed) = &result {
        app.record_line(format!(
            "WORLD memory_composed owner={} picture={} members={} columns={} rows={}",
            composed.owner,
            composed.picture,
            composed.members.len(),
            composed.columns,
            composed.rows
        ));
    }
    answer(&app, "compose", result)
}

async fn looked_at(
    State(app): State<Arc<AppState>>,
    query: Result<Query<InspectQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let Query(query) = query.map_err(|rejection| MemoryError::InvalidQuery {
            detail: rejection.body_text(),
        })?;
        let media = media_directory(app.playground().files());
        run_blocking(move || inspect(&media, &query), MemoryError::Stopped).await
    }
    .await;
    answer(&app, "inspect", result)
}

async fn changes(
    State(app): State<Arc<AppState>>,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let result = match query {
        Ok(Query(query)) => history(app.log(), &query),
        Err(rejection) => Err(MemoryError::InvalidQuery {
            detail: rejection.body_text(),
        }),
    };
    answer(&app, "history", result)
}

/// The memory routes.
pub fn router_with(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::MEMORY, get(owners))
        .route(path::MEMORY_LIST, get(listing))
        .route(path::MEMORY_GET, get(one))
        .route(path::MEMORY_PUT, post(keep))
        .route(path::MEMORY_CAPTURE, post(snap))
        .route(path::MEMORY_DELETE, post(remove))
        .route(path::MEMORY_REFERENCES, post(attach))
        .route(path::MEMORY_COMPOSE, post(compose))
        .route(path::MEMORY_INSPECT, get(looked_at))
        .route(path::MEMORY_HISTORY, get(changes))
        .with_state(app)
}
