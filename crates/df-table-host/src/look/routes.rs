//! The look studio's routes: the phone's (`df_playground::path::PLAY_LOOK*`, on the player
//! listener and the loopback listener) and the dev routes (`path`, loopback only).
//!
//! A phone route takes the seat from the key in its header, never from the body; on the loopback
//! listener the `dev` command line names the seat instead (`seat::key::SeatCaller`). Both call the
//! one studio the services started (`AppState::look`). The work runs on a task of its own, so a
//! caller that leaves does not cancel a reading or a picture already paid for. A refusal writes
//! one `LOOK refused` line.

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, post};
use df_playground::look::wire::{
    LookAgainAsk, LookDetailsAsk, LookLockAsk, LookPackAsk, LookRefineAsk, LookRollAsk,
    LookStartAsk, LookVersionAsk,
};
use df_playground::path::{
    PLAY_LOOK, PLAY_LOOK_AGAIN, PLAY_LOOK_CONTROLS, PLAY_LOOK_DETAILS, PLAY_LOOK_DISCARD,
    PLAY_LOOK_KEEP, PLAY_LOOK_LOCK, PLAY_LOOK_REFINE, PLAY_LOOK_ROLL, PLAY_LOOK_START,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::LookStudio;
use super::request::{
    CharacterSheet, HeroBind, HeroQuery, LookError, PoseCandidatesAsk, PoseChoose, ReferenceQuery,
};
use crate::blocking::run_owned;
use crate::dev_http::{parse_body, read_query, reply};
use crate::imagegen::ImageGenerator;
use crate::seat::key::{SeatCaller, dev_wait};
use crate::state::AppState;
use crate::text::TextRuntime;

/// The addresses of the look studio's dev routes (loopback only).
pub mod path {
    /// The frames' chosen poses (GET).
    pub const POSES: &str = "/dev/look/poses";
    /// POST a `PoseCandidatesAsk`: candidates of a frame's base pose or sheet.
    pub const POSE_CANDIDATES: &str = "/dev/look/pose/candidates";
    /// POST a `PoseChoose`: a candidate becomes a frame's base pose or sheet.
    pub const POSE_CHOOSE: &str = "/dev/look/pose/choose";
    /// POST a `HeroBind`: the hero a seat plays; or GET `?seat=` the kept look of the hero it
    /// plays (a `HeroLook`).
    pub const HERO: &str = "/dev/look/hero";
    /// POST a `LookPackAsk` as the seat `seat::key::DEV_SEAT_HEADER` names: make a pack item of
    /// its hero again, or every item not made. A seat's studio itself is the phone's routes
    /// (`path::PLAY_LOOK*`), which the dev command line calls as the seat.
    pub const PACK: &str = "/dev/look/pack";
    /// The reference sheet of a character of a picture (GET `?character=`): the one lookup every
    /// scene picture uses, for a persona and a seated player's hero alike.
    pub const REFERENCE: &str = "/dev/look/reference";
}

/// The studio the services started, or `StudioClosed`.
fn studio(state: &AppState) -> Result<Arc<LookStudio>, LookError> {
    state.look().cloned().ok_or(LookError::StudioClosed)
}

/// Where the studio file went wrong, for the audit line of a refusal over it: the owner's own
/// data file, so its reader's sentence names the bad field.
fn studio_detail(error: &LookError) -> String {
    match error {
        LookError::StudioInvalid { cause } => format!(
            " line={} column={} detail={:?}",
            cause.line, cause.column, cause.detail
        ),
        LookError::StudioRejected { cause } => format!(" detail={:?}", cause.to_string()),
        _ => String::new(),
    }
}

/// Writes `LOOK refused route=<route> error=<key>` for a refusal, then answers.
fn answer<T: Serialize>(state: &AppState, route: &str, result: Result<T, LookError>) -> Response {
    if let Err(error) = &result {
        state.record_line(format!(
            "LOOK refused route={route} error={}{}",
            error.audit_key(),
            studio_detail(error)
        ));
    }
    reply(result)
}

/// The seat the phone's key holds.
async fn seat_of(state: &AppState, headers: &HeaderMap) -> Result<u64, LookError> {
    Ok(SeatCaller::of(state, headers).await?.seat())
}

/// Runs `work` for `seat` with `ask` on a task of its own.
async fn run<T, R, Work, Done>(
    state: Arc<AppState>,
    seat: u64,
    ask: T,
    wait: bool,
    work: Work,
) -> Result<R, LookError>
where
    T: Send + 'static,
    R: Send + 'static,
    Work: FnOnce(Arc<LookStudio>, Arc<AppState>, u64, T, bool) -> Done + Send + 'static,
    Done: Future<Output = Result<R, LookError>> + Send + 'static,
{
    let studio = studio(&state)?;
    run_owned(work(studio, state, seat, ask, wait), LookError::Stopped).await
}

/// A phone's POST: the seat from its key, the body as `T`.
async fn phone<T, R, Work, Done>(
    state: Arc<AppState>,
    route: &'static str,
    headers: HeaderMap,
    bytes: Bytes,
    work: Work,
) -> Response
where
    T: DeserializeOwned + Send + 'static,
    R: Serialize + Send + 'static,
    Work: FnOnce(Arc<LookStudio>, Arc<AppState>, u64, T, bool) -> Done + Send + 'static,
    Done: Future<Output = Result<R, LookError>> + Send + 'static,
{
    let result = async {
        let seat = seat_of(&state, &headers).await?;
        let ask: T = parse_body(&bytes)?;
        // Only the developer's command line waits for a picture (`DEV_WAIT_HEADER`, refused on
        // the player listener); a phone never does.
        run(Arc::clone(&state), seat, ask, dev_wait(&headers), work).await
    }
    .await;
    answer(&state, route, result)
}

async fn details(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    ask: LookDetailsAsk,
    _: bool,
) -> Result<df_playground::look::wire::LookDetails, LookError> {
    studio.details(&state, seat, &ask.words).await
}

async fn refine(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    ask: LookRefineAsk,
    wait: bool,
) -> Result<df_playground::look::wire::LookVersion, LookError> {
    studio.refine(&state, seat, ask, wait).await
}

async fn keep(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    ask: LookVersionAsk,
    _: bool,
) -> Result<df_playground::look::wire::LookStudioView, LookError> {
    studio.keep_version(&state, seat, ask.version).await
}

async fn discard(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    ask: LookVersionAsk,
    _: bool,
) -> Result<df_playground::look::wire::LookStudioView, LookError> {
    studio.discard_version(&state, seat, ask.version).await
}

async fn again(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    LookAgainAsk {}: LookAgainAsk,
    _: bool,
) -> Result<df_playground::look::wire::LookStudioView, LookError> {
    studio.start_again(&state, seat).await
}

async fn roll(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    LookRollAsk {}: LookRollAsk,
    _: bool,
) -> Result<df_playground::look::wire::LookDraft, LookError> {
    studio.roll(&state, seat).await
}

async fn pack(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    ask: LookPackAsk,
    _: bool,
) -> Result<df_playground::look::wire::HeroPack, LookError> {
    studio.remake_pack(&state, seat, ask).await
}

async fn reference(
    State(state): State<Arc<AppState>>,
    query: Result<Query<ReferenceQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let query = read_query(query).map_err(LookError::InvalidQuery)?;
        let sheet = LookStudio::reference_of(&state, &query.character).await?;
        Ok(CharacterSheet { sheet })
    }
    .await;
    answer(&state, "reference", result)
}

async fn start(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    ask: LookStartAsk,
    _: bool,
) -> Result<df_playground::look::wire::LookStudioView, LookError> {
    studio.start_hero(&state, seat, ask).await
}

async fn start_options(State(state): State<Arc<AppState>>) -> Response {
    let result = async { studio(&state)?.start_options(&state).await }.await;
    answer(&state, "start_options", result)
}

async fn lock(
    studio: Arc<LookStudio>,
    state: Arc<AppState>,
    seat: u64,
    ask: LookLockAsk,
    wait: bool,
) -> Result<df_playground::look::wire::LookStudioView, LookError> {
    studio.lock(&state, seat, ask, wait).await
}

async fn phone_view(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let result = async {
        let seat = seat_of(&state, &headers).await?;
        studio(&state)?.view(&state, seat).await
    }
    .await;
    answer(&state, "view", result)
}

async fn phone_controls(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let result = async {
        let seat = seat_of(&state, &headers).await?;
        studio(&state)?.controls(&state, seat).await
    }
    .await;
    answer(&state, "controls", result)
}

async fn poses(State(state): State<Arc<AppState>>) -> Response {
    let result = async { studio(&state)?.poses(&state).await }.await;
    answer(&state, "poses", result)
}

async fn pose_candidates(State(state): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let ask: PoseCandidatesAsk = parse_body(&bytes)?;
        let studio = studio(&state)?;
        let owner = Arc::clone(&state);
        run_owned(
            async move { studio.pose_candidates(&owner, ask).await },
            LookError::Stopped,
        )
        .await
    }
    .await;
    answer(&state, "pose_candidates", result)
}

async fn pose_choose(State(state): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let ask: PoseChoose = parse_body(&bytes)?;
        studio(&state)?.choose_pose(&state, ask).await
    }
    .await;
    answer(&state, "pose_choose", result)
}

async fn hero_look(
    State(state): State<Arc<AppState>>,
    query: Result<Query<HeroQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let HeroQuery { seat } = read_query(query).map_err(LookError::InvalidQuery)?;
        LookStudio::hero_look(&state, seat).await
    }
    .await;
    answer(&state, "hero_look", result)
}

async fn hero(State(state): State<Arc<AppState>>, bytes: Bytes) -> Response {
    let result = async {
        let bind: HeroBind = parse_body(&bytes)?;
        LookStudio::bind_hero(&state, bind).await
    }
    .await;
    answer(&state, "hero", result)
}

/// The phone's look routes, for the player listener and the loopback listener.
pub(crate) fn player_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(PLAY_LOOK, get(phone_view))
        .route(PLAY_LOOK_CONTROLS, get(phone_controls))
        .route(
            PLAY_LOOK_DETAILS,
            post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "details", headers, bytes, details)
                },
            ),
        )
        .route(
            PLAY_LOOK_REFINE,
            post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "refine", headers, bytes, refine)
                },
            ),
        )
        .route(
            PLAY_LOOK_KEEP,
            post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "keep", headers, bytes, keep)
                },
            ),
        )
        .route(
            PLAY_LOOK_DISCARD,
            post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "discard", headers, bytes, discard)
                },
            ),
        )
        .route(
            PLAY_LOOK_AGAIN,
            post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "again", headers, bytes, again)
                },
            ),
        )
        .route(
            PLAY_LOOK_LOCK,
            post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "lock", headers, bytes, lock)
                },
            ),
        )
        .route(
            PLAY_LOOK_ROLL,
            post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "roll", headers, bytes, roll)
                },
            ),
        )
        .route(
            PLAY_LOOK_START,
            get(start_options).post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "start", headers, bytes, start)
                },
            ),
        )
        .with_state(state)
}

/// The look studio's dev routes, and the phone's on the loopback listener, over one studio made
/// from `generator` and `text` and kept on `app` for the player listener.
pub fn router_with(
    app: Arc<AppState>,
    text: Arc<TextRuntime>,
    generator: Arc<ImageGenerator>,
) -> Router {
    app.set_look(Arc::new(LookStudio::new(generator, text)));
    Router::new()
        .route(path::POSES, get(poses))
        .route(path::POSE_CANDIDATES, post(pose_candidates))
        .route(path::POSE_CHOOSE, post(pose_choose))
        .route(path::HERO, get(hero_look).post(hero))
        .route(
            path::PACK,
            post(
                |State(state): State<Arc<AppState>>, headers: HeaderMap, bytes: Bytes| {
                    phone(state, "pack", headers, bytes, pack)
                },
            ),
        )
        .route(path::REFERENCE, get(reference))
        .with_state(Arc::clone(&app))
        .merge(player_router(app))
}
