//! The inventory routes under `/dev/world/inventory`. Each runs its file work on the blocking pool
//! and answers an `InventoryReply`; a refusal is a typed [`InventoryError`] and one `WORLD refused
//! verb=inventory_<verb> error=<key>` audit line. A change is one `WORLD inventory_changed
//! owner=.. item=.. kind=.. before=.. after=..` line for each item that moved, and a set one
//! `WORLD inventory_set owner=.. kind=.. items=..` line: ids and counts only, never a reason, which
//! is the model's or the caller's words. The history route reads those lines back. The show and
//! hide routes put one owner's inventory on the shared screen and take it away (`screen`), and
//! every route that changes an owner's inventory brings that panel up to date.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use serde::Serialize;

use super::operations::{change, get as get_owner, history, list, move_between, price, set};
use super::request::{
    AddRequest, BatchRequest, Changed, HistoryQuery, InventoryError, OwnerQuery, PayRequest,
    PriceRequest, SetRequest, TakeRequest, TillRequest, TransferRequest,
};
use super::screen::{self, ShowRequest, follow};
use super::store::inventory_directory;
use super::{InventoryChange, OwnerKind};
use crate::blocking::run_blocking;
use crate::dev_http::{parse_body, read_query, reply};
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
    /// POST a `PriceRequest`: an item without a price gets one.
    pub const INVENTORY_PRICE: &str = "/dev/world/inventory/price";
    /// Takes an item from an owner (POST a `TakeRequest`).
    pub const INVENTORY_REMOVE: &str = "/dev/world/inventory/remove";
    /// Uses an item of an owner up (POST a `TakeRequest`).
    pub const INVENTORY_CONSUME: &str = "/dev/world/inventory/consume";
    /// Several changes to one owner, all or none (POST a `BatchRequest`).
    pub const INVENTORY_BATCH: &str = "/dev/world/inventory/batch";
    /// An item from one owner to another, both or neither (POST a `TransferRequest`).
    pub const INVENTORY_TRANSFER: &str = "/dev/world/inventory/transfer";
    /// Coins from one owner to another with exact change, both or neither (POST a `PayRequest`).
    pub const INVENTORY_PAY: &str = "/dev/world/inventory/pay";
    /// `POST`: an owner keeps a till that can make change.
    pub const INVENTORY_TILL: &str = "/dev/world/inventory/till";
    /// The change lines of the host's log (GET with `?owner=&limit=`).
    pub const INVENTORY_HISTORY: &str = "/dev/world/inventory/history";
    /// Shows one owner's inventory on the shared screen (POST a `ShowRequest`).
    pub const INVENTORY_SHOW: &str = "/dev/world/inventory/show";
    /// Takes the inventory off the shared screen (POST, no body).
    pub const INVENTORY_HIDE: &str = "/dev/world/inventory/hide";
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

/// Brings the shared screen's inventory panel up to date for each owner `changed` left.
async fn follow_changed(app: &AppState, changed: &Changed) {
    for view in &changed.owners {
        follow(
            app,
            &view.inventory.owner,
            &view.inventory.items,
            &changed.applied,
        )
        .await;
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
        let query = read_query(query).map_err(InventoryError::InvalidQuery)?;
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
        record_set(&app, changed).await;
    }
    answer(&app, "set", result)
}

/// One `WORLD inventory_set owner= kind= items=` line for each owner a set made, and the shared
/// screen's panel brought up to date.
async fn record_set(app: &AppState, changed: &Changed) {
    for view in &changed.owners {
        app.record_line(format!(
            "WORLD inventory_set owner={} kind={} items={}",
            view.inventory.owner,
            view.inventory.kind.key(),
            view.inventory.items.len()
        ));
    }
    follow_changed(app, changed).await;
}

/// Makes `owner`'s whole inventory an empty one of `kind` through the inventory's one set (as
/// `world inventory set <owner> <kind>` with no items does), made when it is new: a fresh
/// journey's bag, before its starting kit goes in. One `INVENTORY reset owner= kind=
/// items_before=` line (the items held before, none for a new owner), and the set's `WORLD
/// inventory_set` line.
pub(crate) async fn reset_owner(
    app: &AppState,
    owner: &str,
    kind: OwnerKind,
) -> Result<(), InventoryError> {
    let directory = inventory_directory(app);
    let request = SetRequest {
        owner: owner.to_owned(),
        kind,
        items: Vec::new(),
    };
    let (before, changed) = run_blocking(
        move || {
            let before = match get_owner(&directory, &request.owner) {
                Ok(view) => view.inventory.held().count(),
                Err(InventoryError::UnknownOwner { .. }) => 0,
                Err(error) => return Err(error),
            };
            set(&directory, request).map(|changed| (before, changed))
        },
        InventoryError::Stopped,
    )
    .await?;
    app.record_line(format!(
        "INVENTORY reset owner={} kind={} items_before={before}",
        super::logged_id(owner),
        kind.key()
    ));
    record_set(app, &changed).await;
    Ok(())
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
        move || change(&directory, &owner, &changes, None, None),
        InventoryError::Stopped,
    )
    .await;
    if let Ok(changed) = &result {
        record_changed(app, changed);
        follow_changed(app, changed).await;
    }
    answer(app, verb, result)
}

/// Prices an item that has no price yet (a painting to sell from the Bag): one `WORLD
/// inventory_priced owner= item= price_cp=` line, with `kept=yes` when a price it had stays.
async fn priced(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: PriceRequest = parse_body(&bytes)?;
        let directory = inventory_directory(&app);
        let asked = request.clone();
        let priced =
            run_blocking(move || price(&directory, &asked), InventoryError::Stopped).await?;
        app.record_line(format!(
            "WORLD inventory_priced owner={} item={} price_cp={}{}",
            request.owner,
            request.item,
            priced.price_cp,
            if priced.set { "" } else { " kept=yes" }
        ));
        Ok(priced)
    }
    .await;
    answer(&app, "price", result)
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
                price_cp: request.price_cp,
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
        follow_changed(&app, changed).await;
    }
    answer(&app, "transfer", result)
}

/// Pays coins with exact change (`bag::pay_coins`, which writes its own `INVENTORY` line).
async fn pay(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: PayRequest = parse_body(&bytes)?;
        super::bag::pay_coins(
            &app,
            (&request.from, &request.to),
            request.amount_cp,
            &request.why,
        )
        .await
    }
    .await;
    if let Ok(changed) = &result {
        follow_changed(&app, changed).await;
    }
    answer(&app, "pay", result)
}

async fn till(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: TillRequest = parse_body(&bytes)?;
        let giving = Arc::clone(&app);
        run_blocking(
            move || super::give_till(&giving, &request.owner),
            InventoryError::Stopped,
        )
        .await
    }
    .await;
    if let Ok(changed) = &result {
        follow_changed(&app, changed).await;
    }
    answer(&app, "till", result)
}

async fn show(State(app): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let request: ShowRequest = parse_body(&bytes)?;
        screen::show(&app, &request.owner).await
    }
    .await;
    answer(&app, "show", result)
}

async fn hide(State(app): State<Arc<AppState>>) -> Response {
    let result = screen::hide(&app).await;
    answer(&app, "hide", result)
}

async fn changes(
    State(app): State<Arc<AppState>>,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Response {
    let result = read_query(query)
        .map_err(InventoryError::InvalidQuery)
        .and_then(|query| history(app.log(), &query));
    answer(&app, "history", result)
}

/// The inventory routes.
pub fn router_with(app: Arc<AppState>) -> Router {
    Router::new()
        .route(path::INVENTORY, get(owners))
        .route(path::INVENTORY_OWNER, get(owner))
        .route(path::INVENTORY_SET, post(seed))
        .route(path::INVENTORY_ADD, post(add))
        .route(path::INVENTORY_PRICE, post(priced))
        .route(path::INVENTORY_REMOVE, post(remove))
        .route(path::INVENTORY_CONSUME, post(consume))
        .route(path::INVENTORY_BATCH, post(batch))
        .route(path::INVENTORY_TRANSFER, post(transfer))
        .route(path::INVENTORY_PAY, post(pay))
        .route(path::INVENTORY_TILL, post(till))
        .route(path::INVENTORY_HISTORY, get(changes))
        .route(path::INVENTORY_SHOW, post(show))
        .route(path::INVENTORY_HIDE, post(hide))
        .with_state(app)
}
