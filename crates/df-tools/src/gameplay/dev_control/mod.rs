//! Development-only control surface for agents (DEVCTL-001, milestone 1).
//!
//! When the server starts with `DF_GAMEPLAY_DEV_CONTROL=on`, this module mounts
//! a loopback-only JSON API under `/dev/*`. Every game action (join, create,
//! choose, move, attack, say) travels through the same session-owner admission,
//! validation, rules and commit path a phone uses, as a seated player: the
//! handlers submit [`actor::Call`]s to the single owner thread and report the
//! owner's answer faithfully. Reading (`state`, `view`, `log`) and waiting are
//! served from the same views the screens receive.
//!
//! With the setting off, [`setup`] returns [`None`] and no route, token file
//! or behaviour changes: the router merge, the startup line and the shutdown
//! hook below are all gated on it.
//!
//! [`actor::Call`]: super::actor::Call

use std::{io, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
};
use df_protocol::common as rpc;
use serde_json::{Value, json};
use tokio::sync::{Mutex, oneshot, watch};

use super::{actor, timing};
use crate::BUILD_ID;

mod actions;
pub(super) mod direct;
pub(super) mod directions;
pub(super) mod candidate;
pub(crate) mod commands;
pub(super) mod config;
pub(crate) mod history;
mod jumps;
mod parse;
mod reads;

/// The development history bundle, wired only when the surface is on: the
/// back/forward ring plus the actor-side state the checkpoint does not carry
/// (the table log for save-time floors, the social ledger for snapshots), so
/// a rewind restores all of it or none.
#[derive(Clone)]
pub(super) struct Bundle {
    pub history: Arc<std::sync::Mutex<history::History>>,
    pub table: Arc<super::table_log::TableLog>,
    pub social: Arc<super::social::SocialLedger>,
}

/// Shared development control state, cloned into every route handler.
#[derive(Clone)]
struct Shared {
    service: super::Service,
    display_credential: [u8; 32],
    token: Arc<str>,
    seat_file: std::path::PathBuf,
    shutdown: watch::Sender<bool>,
    seats: Arc<Mutex<Seats>>,
    overrides: Arc<Mutex<Vec<Value>>>,
    /// Admitted references for candidate rebuilds (milestones 2/3).
    recovery: df_persistence::NativeRecoverySource,
    /// Back/forward ring shared with the owner's publication hook.
    history: Arc<std::sync::Mutex<history::History>>,
    /// The public table log (for save-time floors and rewinds).
    table: Arc<super::table_log::TableLog>,
    /// The NPC social ledger (for save-time snapshots and rewinds).
    social: Arc<super::social::SocialLedger>,
}

/// The two joinable seats and the session scope learned from the first join.
#[derive(serde::Serialize, serde::Deserialize)]
struct Seats {
    session_id: Option<Vec<u8>>,
    run_id: Option<Vec<u8>>,
    seats: [Seat; 2],
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Seat {
    #[serde(default)]
    reclaim: Option<PendingReclaim>,
    credential: Option<[u8; 32]>,
    member: Option<Vec<u8>>,
    hero: Option<String>,
}

/// Admitted-reference inventory plus history handle live in [`Shared`]; named
/// snapshots live in the shared [`history::History`] instead.
/// The session/run scope every owner call needs, or why no seat can act yet.
type Scope = Result<(Vec<u8>, Vec<u8>), (StatusCode, Json<Value>)>;

/// What [`setup`] hands back to the server: the routes to merge (only when the
/// surface is on), a shutdown receiver for the serve loop, and the one startup
/// line the server prints.
pub(super) struct Control {
    /// Routes to merge into the game router. `None` keeps the game unchanged.
    pub router: Option<Router>,
    /// Fires when an authenticated `POST /dev/shutdown` asks for a clean stop.
    pub shutdown: watch::Receiver<bool>,
    /// The single startup line, printed only when the surface is on.
    pub startup_line: Option<String>,
}

/// Starts the control surface when `DF_GAMEPLAY_DEV_CONTROL=on`.
///
/// Reads the setting, refuses a non-loopback `listen` without the second
/// explicit setting, writes a fresh bearer token to
/// `development/runs/<database>/dev-control-token` and returns the routes.
/// Whether the surface would start for this listener. Pure environment read,
/// so the serve loop can wire the history hook before the routes exist.
pub(super) fn setting_on(listen: SocketAddr) -> Result<bool, io::Error> {
    Ok(config::setting(
        std::env::var("DF_GAMEPLAY_DEV_CONTROL"),
        std::env::var("DF_GAMEPLAY_DEV_CONTROL_PUBLIC"),
        listen,
    )? == config::Setting::On)
}

/// Starts the control surface when `DF_GAMEPLAY_DEV_CONTROL=on`.
///
/// Reads the setting, refuses a non-loopback `listen` without the second
/// explicit setting, writes a fresh control file to
/// `development/runs/<database>/dev-control-token` and returns the routes.
/// With the setting off, returns no routes and touches nothing.
pub(super) fn setup(
    service: super::Service,
    display_credential: [u8; 32],
    listen: SocketAddr,
    database: &tokio_postgres::Config,
    run_root: &std::path::Path,
    recovery: df_persistence::NativeRecoverySource,
    bundle: Bundle,
) -> Result<Control, io::Error> {
    let (shutdown_send, shutdown_receive) = watch::channel(false);
    if !setting_on(listen)? {
        return Ok(Control {
            router: None,
            shutdown: shutdown_receive,
            startup_line: None,
        });
    }
    let database_name = database.get_dbname().unwrap_or_default().to_owned();
    if database_name.is_empty() {
        return Err(io::Error::other("dev control needs a named database"));
    }
    let run_dir = run_root
        .join("development")
        .join("runs")
        .join(&database_name);
    let token = config::fresh_token()?;
    let token_file = config::write_token_file(&run_dir, &token)?;
    let seat_file = run_dir.join("dev-control-seats");
    let shared = Shared {
        service,
        display_credential,
        token: Arc::from(token),
        seats: Arc::new(Mutex::new(load_seats(&seat_file)?)),
        seat_file,
        shutdown: shutdown_send,
        overrides: Arc::new(Mutex::new(Vec::new())),
        recovery,
        history: bundle.history,
        table: bundle.table,
        social: bundle.social,
    };
    let state = Arc::new(shared);
    let router = Router::new()
        .route("/dev/health", get(health))
        .route("/dev/join", post(actions::join_seat))
        .route("/dev/reclaim", post(actions::reclaim_seat))
        .route("/dev/create", post(actions::create_hero))
        .route("/dev/act", post(actions::act))
        .route("/dev/say", post(actions::say))
        .route("/dev/direct", post(direct::direct))
        .route("/dev/direct/{key}", get(direct::status))
        .route("/dev/state", get(reads::game_state))
        .route("/dev/view", get(reads::view_as))
        .route("/dev/log", get(reads::game_log))
        .route("/dev/checkpoint", post(reads::save_checkpoint))
        .route("/dev/checkpoints", get(reads::list_checkpoints))
        .route("/dev/restore", post(jumps::restore_named))
        .route("/dev/back", post(jumps::step_back))
        .route("/dev/forward", post(jumps::step_forward))
        .route("/dev/override", post(jumps::apply_override))
        .route("/dev/shutdown", post(shutdown))
        .route("/dev/describe", get(reads::describe))
        .with_state(state);
    Ok(Control {
        router: Some(router),
        shutdown: shutdown_receive,
        startup_line: Some(format!(
            "DEV CONTROL on url=http://{listen} token_file={} (token never printed)",
            token_file.display(),
        )),
    })
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct PendingReclaim {
    operation: [u8; 16],
    token: [u8; 32],
}

impl Seat {
    fn empty() -> Self {
        Self {
            reclaim: None,
            credential: None,
            member: None,
            hero: None,
        }
    }
}

fn load_seats(path: &std::path::Path) -> Result<Seats, io::Error> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| io::Error::other("dev control seat state invalid")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Seats {
            session_id: None,
            run_id: None,
            seats: [Seat::empty(), Seat::empty()],
        }),
        Err(error) => Err(error),
    }
}

fn persist_seats(state: &Shared, seats: &Seats) -> Result<(), io::Error> {
    let bytes = serde_json::to_vec(seats)
        .map_err(|_| io::Error::other("dev control seat state invalid"))?;
    std::fs::write(&state.seat_file, bytes)
}

/// Parses `1`, `2`, `seat1` or `seat2` into a seat index.
fn seat_index(text: &str) -> Result<usize, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "seat1" | "hero1" => Ok(0),
        "2" | "seat2" | "hero2" => Ok(1),
        _ => Err(format!("seat {text:?} must be 1 or 2")),
    }
}

/// Resolves a hero reference (seat label or hero name, case-insensitive) to a
/// seat index. Names win when they match; ambiguous names are refused.
fn resolve_hero(seats: &Seats, query: &str) -> Result<usize, String> {
    if let Ok(index) = seat_index(query) {
        return Ok(index);
    }
    let mut found = None;
    for (index, seat) in seats.seats.iter().enumerate() {
        if seat
            .hero
            .as_deref()
            .is_some_and(|hero| hero.eq_ignore_ascii_case(query))
        {
            if found.is_some() {
                return Err(format!("hero {query:?} is ambiguous"));
            }
            found = Some(index);
        }
    }
    found.ok_or_else(|| format!("unknown hero {query:?}; join and create first (see dev describe)"))
}

fn authorized(headers: &HeaderMap, token: &str) -> Result<(), (StatusCode, Json<Value>)> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if config::bearer_authorized(header, token) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "dev control bearer token required"})),
        ))
    }
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": message.into()})),
    )
}

fn failed(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({"error": message.into()})),
    )
}

type Outcome = Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)>;

async fn health(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let revision = current_revision(&state).await?;
    Ok((
        StatusCode::OK,
        Json(json!({"status": "ok", "build": BUILD_ID, "revision": revision})),
    ))
}

/// The owner's current revision, read from the display view without side effects.
async fn current_revision(state: &Shared) -> Result<Value, (StatusCode, Json<Value>)> {
    let (_, view) = display_view(state).await?;
    Ok(revision_json(view.revision.as_ref()))
}

fn revision_json(revision: Option<&rpc::SessionRevision>) -> Value {
    match revision {
        Some(revision) => {
            let epoch = revision
                .epoch
                .as_ref()
                .and_then(|epoch| epoch.value)
                .unwrap_or(0);
            json!({"epoch": epoch, "sequence": revision.sequence.unwrap_or(0)})
        }
        None => json!({"epoch": 0, "sequence": 0}),
    }
}

/// Calls the owner for the exact display view the TV is being sent.
async fn display_view(
    state: &Shared,
) -> Result<(rpc::SessionRevision, rpc::ViewMessage), (StatusCode, Json<Value>)> {
    let basis = state.service.updates.borrow().basis();
    let (session_id, run_id) = display_scope(basis);
    let request = rpc::WatchViewRequest {
        session_id: Some(rpc::SessionId {
            value: Some(session_id),
        }),
        run_id: Some(rpc::RunId {
            value: Some(run_id),
        }),
        client_binding_id: Some(rpc::ClientBindingId {
            value: Some(vec![0x72; 16]),
        }),
        after_revision: None,
    };
    owner_view(state, state.display_credential, request).await
}

/// Calls the owner for the exact player view one seat is being sent.
async fn seat_view(
    state: &Shared,
    seat: usize,
) -> Result<(rpc::SessionRevision, rpc::ViewMessage), (StatusCode, Json<Value>)> {
    let seats = state.seats.lock().await;
    let (session_id, run_id) = scope_of(&seats)?;
    let member = seats.seats[seat]
        .member
        .clone()
        .ok_or_else(|| bad_request(format!("seat{} has not joined yet", seat + 1)))?;
    let credential = seats.seats[seat]
        .credential
        .ok_or_else(|| bad_request(format!("seat{} has not joined yet", seat + 1)))?;
    drop(seats);
    let request = rpc::WatchViewRequest {
        session_id: Some(rpc::SessionId {
            value: Some(session_id),
        }),
        run_id: Some(rpc::RunId {
            value: Some(run_id),
        }),
        client_binding_id: Some(rpc::ClientBindingId {
            value: Some(member),
        }),
        after_revision: None,
    };
    owner_view(state, credential, request).await
}

fn display_scope(basis: df_model::checkpoint::Basis) -> (Vec<u8>, Vec<u8>) {
    (
        basis.session.as_bytes().to_vec(),
        basis.run.as_bytes().to_vec(),
    )
}

fn scope_of(seats: &Seats) -> Scope {
    match (seats.session_id.clone(), seats.run_id.clone()) {
        (Some(session), Some(run)) => Ok((session, run)),
        _ => Err(bad_request("no seat has joined yet")),
    }
}

/// The same owner round trip the phone-facing view service performs.
async fn owner_view(
    state: &Shared,
    credential: [u8; 32],
    request: rpc::WatchViewRequest,
) -> Result<(rpc::SessionRevision, rpc::ViewMessage), (StatusCode, Json<Value>)> {
    let (reply, wait) = oneshot::channel();
    state
        .service
        .actor
        .try_submit(actor::Call::View {
            credential,
            request,
            reply,
        })
        .map_err(|_| failed("gameplay actor queue full"))?;
    let (role, mut view) = tokio::time::timeout(timing::active().reply, wait)
        .await
        .map_err(|_| failed("gameplay view deadline"))?
        .map_err(|_| failed("gameplay actor stopped"))?
        .map_err(|status| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": status.message()})),
            )
        })?;
    let _ = role;
    // Presentation-only decoration the Service layer applies to every served
    // view is intentionally left out here (see the handoff); the journey
    // projection below is exactly what the owner committed.
    let revision = view.revision.unwrap_or_default();
    super::home_video::apply(&mut view);
    state.service.living.observe(&view);
    state.service.living.apply(&mut view);
    Ok((revision, view))
}

/// Submits one player action through the owner's full admission path and maps
/// the owner's answer to JSON without rewording it.
async fn owner_submit_action(
    state: &Shared,
    credential: [u8; 32],
    request: rpc::SubmitActionRequest,
) -> Outcome {
    let (reply, wait) = oneshot::channel();
    state
        .service
        .actor
        .try_submit(actor::Call::Submit {
            credential,
            request,
            reply,
        })
        .map_err(|_| failed("gameplay actor queue full"))?;
    let response = tokio::time::timeout(timing::active().reply, wait)
        .await
        .map_err(|_| failed("gameplay submit deadline"))?
        .map_err(|_| failed("gameplay actor stopped"))?
        .map_err(|status| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": status.message()})),
            )
        })?;
    Ok((StatusCode::OK, Json(submit_json(response))))
}

fn submit_json(response: rpc::SubmitActionResponse) -> Value {
    match response.outcome {
        Some(rpc::submit_action_response::Outcome::CommittedDecision(receipt)) => {
            receipt_json(*receipt)
        }
        Some(rpc::submit_action_response::Outcome::OperationObservation(observation)) => {
            json!({"status": "observation", "code": rejection_name(observation.code)})
        }
        None => json!({"status": "error", "error": "empty submit answer"}),
    }
}

fn receipt_json(receipt: rpc::DecisionReceipt) -> Value {
    let revision = revision_json(receipt.revision.as_ref());
    match receipt.outcome {
        Some(rpc::decision_receipt::Outcome::Accepted(accepted)) => {
            json!({
                "status": "accepted",
                "replayed": receipt.replayed,
                "revision": revision,
                "phase": phase_name(accepted.phase),
                "accepted": accepted_json(*accepted),
            })
        }
        Some(rpc::decision_receipt::Outcome::Rejected(rejected)) => {
            json!({
                "status": "refused",
                "replayed": receipt.replayed,
                "revision": revision,
                "code": rejection_name(rejected.code),
            })
        }
        None => json!({"status": "error", "revision": revision, "error": "empty receipt"}),
    }
}

fn accepted_json(accepted: rpc::AcceptedAction) -> Value {
    json!({
        "scene": accepted.scene.map(|scene| json!({
            "title": scene.title,
            "place": scene.place,
        })),
        "character": accepted.character.map(|sheet| sheet_json(&sheet)),
        "check": accepted.check.map(|check| json!({
            "die": check.die,
            "total": check.total,
            "succeeded": check.succeeded,
        })),
        "combat": accepted.combat.iter().map(combat_json).collect::<Vec<_>>(),
    })
}

fn combat_json(outcome: &rpc::CombatOutcome) -> Value {
    json!({
        "kind": outcome.kind,
        "actor": outcome.actor_name,
        "target": outcome.target_name,
        "attack_total": outcome.attack_total,
        "hit": outcome.hit,
        "critical": outcome.critical,
        "damage": outcome.damage,
        "target_hit_points": outcome.target_hit_points,
        "knocked_out": outcome.knocked_out,
        "weapon": outcome.weapon,
    })
}

fn sheet_json(sheet: &rpc::CharacterSheet) -> Value {
    json!({
        "name": sheet.name,
        "hit_points": sheet.hit_points,
        "maximum_hit_points": sheet.maximum_hit_points,
        "armor_class": sheet.armor_class,
        "facts": sheet.facts.iter().map(|fact| json!({"label": fact.label, "value": fact.value})).collect::<Vec<_>>(),
    })
}

fn phase_name(phase: i32) -> &'static str {
    match rpc::JourneyPhase::try_from(phase) {
        Ok(rpc::JourneyPhase::Room) => "room",
        Ok(rpc::JourneyPhase::CharacterCreation) => "creation",
        Ok(rpc::JourneyPhase::Opening) => "opening",
        Ok(rpc::JourneyPhase::Dialogue) => "dialogue",
        Ok(rpc::JourneyPhase::Combat) => "combat",
        Ok(rpc::JourneyPhase::Complete) => "complete",
        _ => "unknown",
    }
}

fn rejection_name(code: i32) -> &'static str {
    match rpc::RejectionCode::try_from(code) {
        Ok(rpc::RejectionCode::StaleOffer) => "stale-offer",
        Ok(rpc::RejectionCode::WrongTurn) => "wrong-turn",
        Ok(rpc::RejectionCode::InvalidSelection) => "invalid-selection",
        Ok(rpc::RejectionCode::Paused) => "paused",
        Ok(rpc::RejectionCode::ResourceMissing) => "resource-missing",
        Ok(rpc::RejectionCode::CapabilityUnavailable) => "capability-unavailable",
        Ok(rpc::RejectionCode::RecoveryRequired) => "recovery-required",
        Ok(rpc::RejectionCode::RevisionConflict) => "revision-conflict",
        Ok(rpc::RejectionCode::OperationConflict) => "operation-conflict",
        Ok(rpc::RejectionCode::OperationExpired) => "operation-expired",
        _ => "unknown",
    }
}

async fn shutdown(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    state
        .shutdown
        .send(true)
        .map_err(|_| failed("shutdown signal refused"))?;
    Ok((
        StatusCode::OK,
        Json(
            json!({"status": "shutting-down", "note": "the server releases its owner lease, so the database resumes immediately"}),
        ),
    ))
}

#[cfg(test)]
mod display_scope_tests {
    use super::*;
    use df_types::{RecoveryEpoch, RunId, SessionId, SessionRevision};

    #[test]
    fn display_scope_uses_the_checkpoint_session_and_run() {
        let basis = df_model::checkpoint::Basis {
            session: SessionId::from_bytes(&[1; 16]).expect("valid session id"),
            run: RunId::from_bytes(&[2; 16]).expect("valid run id"),
            revision: SessionRevision::new(RecoveryEpoch::new(1).expect("valid epoch"), 0),
        };

        assert_eq!(display_scope(basis), (vec![1; 16], vec![2; 16]));
    }

    #[test]
    fn dev_control_seat_state_round_trips_credentials_and_hero_names() {
        let seats = Seats {
            session_id: Some(vec![1; 16]),
            run_id: Some(vec![2; 16]),
            seats: [
                Seat {
                    reclaim: Some(PendingReclaim {
                        operation: [5; 16],
                        token: [6; 32],
                    }),
                    credential: Some([3; 32]),
                    member: Some(vec![4; 16]),
                    hero: Some("Sigrun".to_owned()),
                },
                Seat::empty(),
            ],
        };

        let bytes = serde_json::to_vec(&seats).expect("seat state serializes");
        let restored: Seats = serde_json::from_slice(&bytes).expect("seat state restores");

        assert_eq!(restored.session_id, Some(vec![1; 16]));
        assert_eq!(restored.run_id, Some(vec![2; 16]));
        assert_eq!(restored.seats[0].credential, Some([3; 32]));
        assert_eq!(restored.seats[0].member, Some(vec![4; 16]));
        assert_eq!(restored.seats[0].hero.as_deref(), Some("Sigrun"));
        assert!(restored.seats[1].credential.is_none());
        let pending = restored.seats[0]
            .reclaim
            .expect("uncertain reclaim proof restored");
        assert_eq!(pending.operation, [5; 16]);
        assert_eq!(pending.token, [6; 32]);
    }
}
