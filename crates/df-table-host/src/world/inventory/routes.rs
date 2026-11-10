//! The inventory routes under `/dev/world/inventory`. Each runs its file work on the blocking pool
//! and answers an `InventoryReply`; a refusal is a typed [`InventoryError`] and one `WORLD refused
//! verb=inventory_<verb> error=<key>` audit line. A change is one `WORLD inventory_changed
//! owner=.. item=.. kind=.. before=.. after=..` line for each item that moved, and a set one
//! `WORLD inventory_set owner=.. kind=.. items=..` line: ids and counts only, never a reason, which
//! is the model's or the caller's words. The history route reads those lines back.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use serde::Serialize;

use super::InventoryChange;
use super::operations::{change, get as get_owner, history, list, move_between, set};
use super::request::{
    AddRequest, BatchRequest, Changed, HistoryQuery, InventoryError, OwnerQuery, SetRequest,
    TakeRequest, TransferRequest,
};
use super::store::inventory_directory;
use crate::blocking::run_blocking;
use crate::dev_http::{parse_body, reply};
use crate::state::AppState;

/// The addresses of the inventory dev API.
pub mod path {
    /// Every owner (GET).
    pub const INVENTORY: &str = "/dev/world/inventory";
    /// One owner's inventory (GET with `?owner=`).
    pub const INVENTORY_OWNER: &str = "/dev/world/inventory/owner";
    /// Seeds or replaces an owner's whole inventory (POST a `SetRequest`).
    pub const INVENTORY_SET: &str = "/dev/world/inventory/set";
    /// Adds to an owner's item (POST an `AddRequest`).
    pub const INVENTORY_ADD: &str = "/dev/world/inventory/add";
    /// Takes an item from an owner (POST a `TakeRequest`).
    pub const INVENTORY_REMOVE: &str = "/dev/world/inventory/remove";
    /// Uses an item of an owner up (POST a `TakeRequest`).
    pub const INVENTORY_CONSUME: &str = "/dev/world/inventory/consume";
    /// Several changes to one owner, all or none (POST a `BatchRequest`).
    pub const INVENTORY_BATCH: &str = "/dev/world/inventory/batch";
    /// An item from one owner to another, both or neither (POST a `TransferRequest`).
    pub const INVENTORY_TRANSFER: &str = "/dev/world/inventory/transfer";
    /// The change lines of the host's log (GET with `?owner=&limit=`).
    pub const INVENTORY_HISTORY: &str = "/dev/world/inventory/history";
}

/// Records a refusal of `verb` as `WORLD refused verb=inventory_<verb> error=<key>`.
pub(super) fn record_refused(app: &AppState, verb: &str, error: &InventoryError) {
    app.record_line(format!(
        "WORLD refused verb=inventory_{verb} error={}",
        error.key()
    ));
}

/// Records each change `changed` came to, one line for each item that moved.
pub(super) fn record_changed(app: &AppState, changed: &Changed) {
    for taken in &changed.applied {
        app.record_line(format!(
            "WORLD inventory_changed owner={} item={} kind={} before={} after={}",
            taken.owner,
            taken.item,
            taken.kind.key(),
            taken.before,
            taken.after
        ));
    }
}

fn answer<T: Serialize>(app: &AppState, verb: &str, result: Result<T, InventoryError>) -> Response {
    if let Err(error) = &result {
        record_refused(app, verb, error);
    }
    reply::<T, InventoryError>(result)
}

async fn owners(State(app): State<Arc<AppState>>) -> Response {
    let directory = inventory_directory(&app);
    let result = run_blocking(move || list(&directory), InventoryError::Stopped).await;
    answer(&app, "list", result)
}

async fn owner(
    State(app): State<Arc<AppState>>,
    query: Result<Query<OwnerQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let Query(query) = query.map_err(|rejection| InventoryError::InvalidQuery {
            detail: rejection.body_text(),
        })?;
        let directory = inventory_directory(&app);
        run_blocking(
            move || get_owner(&directory, &query.owner),
            InventoryError::Stopped,
        )
        .await
    }
    .await;
    answer(&app, "get", result)
}

async fn seed(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: SetRequest = parse_body(&bytes)?;
        let directory = inventory_directory(&app);
        run_blocking(move || set(&directory, request), InventoryError::Stopped).await
    }
    .await;
    if let Ok(changed) = &result {
        for view in &changed.owners {
            app.record_line(format!(
                "WORLD inventory_set owner={} kind={} items={}",
                view.inventory.owner,
                view.inventory.kind.key(),
                view.inventory.items.len()
            ));
        }
    }
    answer(&app, "set", result)
}

/// Runs `changes` on the owner and records what they came to.
async fn changing(
    app: &Arc<AppState>,
    verb: &str,
    owner: String,
    changes: Vec<InventoryChange>,
) -> Response {
    let directory = inventory_directory(app);
    let result = run_blocking(
        move || change(&directory, &owner, &changes),
        InventoryError::Stopped,
    )
    .await;
    if let Ok(changed) = &result {
        record_changed(app, changed);
    }
    answer(app, verb, result)
}

async fn add(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    match parse_body::<AddRequest>(&bytes) {
        Ok(request) => {
            let change = InventoryChange::Add {
                item: request.item,
                count: request.count,
                name: request.name,
                look: request.look,
                usage: request.usage,
                why: request.why,
            };
            changing(&app, "add", request.owner, vec![change]).await
        }
        Err(error) => answer::<Changed>(&app, "add", Err(error.into())),
    }
}

async fn remove(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    match parse_body::<TakeRequest>(&bytes) {
        Ok(request) => {
            let change = InventoryChange::Remove {
                item: request.item,
                count: request.count,
                why: request.why,
            };
            changing(&app, "remove", request.owner, vec![change]).await
        }
        Err(error) => answer::<Changed>(&app, "remove", Err(error.into())),
    }
}

async fn consume(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    match parse_body::<TakeRequest>(&bytes) {
        Ok(request) => {
            let change = InventoryChange::Consume {
                item: request.item,
                count: request.count,
                why: request.why,
            };
            changing(&app, "consume", request.owner, vec![change]).await
        }
        Err(error) => answer::<Changed>(&app, "consume", Err(error.into())),
    }
}

async fn batch(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    match parse_body::<BatchRequest>(&bytes) {
        Ok(request) => changing(&app, "batch", request.owner, request.changes).await,
        Err(error) => answer::<Changed>(&app, "batch", Err(error.into())),
    }
}

async fn transfer(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: TransferRequest = parse_body(&bytes)?;
        let directory = inventory_directory(&app);
        run_blocking(
            move || move_between(&directory, &request),
            InventoryError::Stopped,
        )
        .await
    }
    .await;
    if let Ok(changed) = &result {
        record_changed(&app, changed);
    }
    answer(&app, "transfer", result)
}

async fn changes(
    State(app): State<Arc<AppState>>,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let result = match query {
        Ok(Query(query)) => history(app.log(), &query),
        Err(rejection) => Err(InventoryError::InvalidQuery {
            detail: rejection.body_text(),
        }),
    };
    answer(&app, "history", result)
}

/// The inventory routes.
pub fn router_with(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::INVENTORY, get(owners))
        .route(path::INVENTORY_OWNER, get(owner))
        .route(path::INVENTORY_SET, post(seed))
        .route(path::INVENTORY_ADD, post(add))
        .route(path::INVENTORY_REMOVE, post(remove))
        .route(path::INVENTORY_CONSUME, post(consume))
        .route(path::INVENTORY_BATCH, post(batch))
        .route(path::INVENTORY_TRANSFER, post(transfer))
        .route(path::INVENTORY_HISTORY, get(changes))
        .with_state(app)
}
