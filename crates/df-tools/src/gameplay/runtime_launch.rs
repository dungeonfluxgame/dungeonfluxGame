//! Single-room campaign attachment. The game owner issues every seat grant; routing is disposable.
use super::{PageState, actor, room};
use axum::{
    Json, Router,
    extract::{
        ConnectInfo, DefaultBodyLimit, FromRequestParts, Path, Request, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use df_campaign::session_link::runtime_protocol::*;
use df_protocol::common as rpc;
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const MAXIMUM_ALLOCATION_BYTES: usize = 4096;
const MAXIMUM_DISPLAY_TICKETS: usize = 16;

struct AssignedSeat {
    member: [u8; 16],
    request: rpc::JoinRoomRequest,
    joined: Option<rpc::JoinedRoom>,
}
struct Slot {
    allocation: Allocation,
    seats: Vec<AssignedSeat>,
}

#[derive(Clone)]
struct LaunchState {
    actor: super::InboxHandle<actor::Call>,
    page: PageState,
    secret: LaunchSecret,
    generation: Arc<str>,
    code: Arc<str>,
    slot: Arc<Mutex<Option<Slot>>>,
}

pub(super) fn secret_from_environment() -> Result<Option<LaunchSecret>, std::io::Error> {
    secret_for_profile(
        std::env::var("DF_RUNTIME_LAUNCH_SECRET").ok().as_deref(),
        std::env::var("DF_PROFILE").ok().as_deref(),
        cfg!(debug_assertions),
    )
}

fn secret_for_profile(
    secret: Option<&str>,
    profile: Option<&str>,
    debug_build: bool,
) -> Result<Option<LaunchSecret>, std::io::Error> {
    match secret {
        Some(secret) => LaunchSecret::new(secret)
            .map(Some)
            .map_err(|_| std::io::Error::other("invalid DF_RUNTIME_LAUNCH_SECRET")),
        None if profile == Some("development") || (profile.is_none() && debug_build) => Ok(None),
        None => Err(std::io::Error::other(
            "DF_RUNTIME_LAUNCH_SECRET required outside development",
        )),
    }
}

pub(super) fn router(
    actor: super::InboxHandle<actor::Call>,
    page: PageState,
    secret: Option<LaunchSecret>,
) -> Result<Router, std::io::Error> {
    let Some(secret) = secret else {
        return Ok(Router::new());
    };
    let state = LaunchState {
        actor,
        page,
        secret,
        generation: super::hex(&super::model::random::<16>().map_err(|error| {
            actor::record_audit_line(
                "session.runtime_refused",
                format!("SESSION runtime_refused reason=generation error={error:?}"),
            );
            std::io::Error::other("runtime generation unavailable")
        })?)
        .into(),
        code: super::journey::room_code().into(),
        slot: Arc::new(Mutex::new(None)),
    };
    Ok(routes(state))
}

fn routes(state: LaunchState) -> Router {
    Router::new()
        .route(ALLOCATION_PATH, post(allocate))
        .route(&format!("{ENTRY_PATH}/{{surface}}"), get(entry))
        .layer(DefaultBodyLimit::max(MAXIMUM_ALLOCATION_BYTES))
        .with_state(state)
}

fn authenticated(
    state: &LaunchState,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: &HeaderMap,
) -> bool {
    let offered = headers
        .get(SECRET_HEADER)
        .map(|value| value.as_bytes())
        .unwrap_or_default();
    let secret_matches = state.secret.authenticates(offered);
    secret_matches && peer.is_some_and(|peer| peer.0.ip().is_loopback())
}

fn valid_allocation(allocation: &Allocation) -> bool {
    allocation.session != [0; 16]
        && allocation.campaign != [0; 16]
        && !allocation.roster.is_empty()
        && allocation.roster.len() <= df_campaign::session_link::MAXIMUM_DEMO_SEATS
        && allocation.display_tickets.len() <= MAXIMUM_DISPLAY_TICKETS
        && allocation
            .display_tickets
            .iter()
            .all(|ticket| *ticket != [0; 16])
        && allocation.roster.iter().enumerate().all(|(index, seat)| {
            seat.account != [0; 16]
                && seat.member != [0; 16]
                && !allocation
                    .roster
                    .iter()
                    .skip(index + 1)
                    .any(|other| other.account == seat.account || other.member == seat.member)
        })
}

async fn allocate(
    State(state): State<LaunchState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(mut allocation): Json<Allocation>,
) -> Response {
    if !authenticated(&state, Some(peer), &headers) {
        return concealed();
    }
    if !valid_allocation(&allocation) {
        return refusal(Refusal::InvalidPlacement);
    }
    allocation.roster.sort_by_key(|seat| seat.member);
    allocation.display_tickets.sort();
    // This lock serializes attachment only, never the game owner. A cancelled request retains
    // the reserved slot and exact join proofs, so a retry reconciles the same owner operations.
    let mut slot = state.slot.lock().await;
    if let Some(existing) = slot.as_ref() {
        if existing.allocation != allocation {
            return refusal(Refusal::Occupied);
        }
    } else {
        let seats = allocation
            .roster
            .iter()
            .map(|seat| {
                let mut identity = Vec::new();
                identity.extend_from_slice(&allocation.session);
                identity.extend_from_slice(&seat.member);
                identity.extend_from_slice(state.generation.as_bytes());
                let operation = state
                    .secret
                    .derive(b"campaign join operation v1", &identity);
                AssignedSeat {
                    member: seat.member,
                    request: rpc::JoinRoomRequest {
                        room_code: state.code.to_string(),
                        operation_id: Some(rpc::OperationId {
                            value: Some(operation[..16].to_vec()),
                        }),
                        join_secret: state
                            .secret
                            .derive(b"campaign join secret v1", &identity)
                            .to_vec(),
                    },
                    joined: None,
                }
            })
            .collect();
        *slot = Some(Slot { allocation, seats });
    }
    let Some(slot) = slot.as_mut() else {
        return refusal(Refusal::Unavailable);
    };
    for seat in &mut slot.seats {
        if seat.joined.is_none() {
            match join(&state.actor, seat.request.clone()).await {
                Ok(joined) => seat.joined = Some(joined),
                Err(_) => return refusal(Refusal::Unavailable),
            }
        }
    }
    actor::record_audit_line(
        "session.runtime_attached",
        format!(
            "SESSION runtime_attached session={} seats={} generation={}",
            super::hex(&slot.allocation.session),
            slot.seats.len(),
            state.generation
        ),
    );
    private(
        Json(Allocated {
            code: state.code.to_string(),
            generation: state.generation.to_string(),
        })
        .into_response(),
    )
}

async fn join(
    actor: &super::InboxHandle<actor::Call>,
    request: rpc::JoinRoomRequest,
) -> Result<rpc::JoinedRoom, tonic::Status> {
    let (reply, wait) = tokio::sync::oneshot::channel();
    actor
        .try_submit(actor::Call::Join { request, reply })
        .map_err(|_| tonic::Status::unavailable("room owner busy"))?;
    let response = tokio::time::timeout(super::timing::active().reply, wait)
        .await
        .map_err(|_| tonic::Status::deadline_exceeded("room join deadline"))?
        .map_err(|_| tonic::Status::unavailable("room owner stopped"))??;
    match response.outcome {
        Some(rpc::join_room_response::Outcome::Joined(joined)) => Ok(*joined),
        _ => Err(tonic::Status::unavailable("room admission refused")),
    }
}

async fn entry(
    State(state): State<LaunchState>,
    Path(surface): Path<String>,
    request: Request,
) -> Response {
    let (mut parts, _) = request.into_parts();
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().cloned();
    let headers = parts.headers.clone();
    if !authenticated(&state, peer, &headers) {
        return concealed();
    }
    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(now) => now.as_secs(),
        Err(_) => return refusal(Refusal::Unavailable),
    };
    let Some(entry) = headers
        .get(ENTRY_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|token| state.secret.verify(token, now))
    else {
        return concealed();
    };
    let joined = {
        let slot = state.slot.lock().await;
        let Some(slot) = slot.as_ref() else {
            return concealed();
        };
        if entry.session != slot.allocation.session
            || entry.campaign != slot.allocation.campaign
            || entry.generation != state.generation.as_ref()
        {
            return concealed();
        }
        match entry.audience {
            EntryAudience::Player(member) => match slot
                .seats
                .iter()
                .find(|seat| seat.member == member)
                .and_then(|seat| seat.joined.clone())
            {
                Some(joined) => Some(joined),
                None => return concealed(),
            },
            EntryAudience::Display(ticket) => {
                if !slot.allocation.display_tickets.contains(&ticket) {
                    return concealed();
                }
                None
            }
        }
    };
    if surface == "tunnel" {
        let Ok(upgrade) = WebSocketUpgrade::from_request_parts(&mut parts, &()).await else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        return super::socket(State(state.page), headers, upgrade).await;
    }
    entry_page(&state, &entry, &surface, joined).await
}

async fn entry_page(
    state: &LaunchState,
    entry: &Entry,
    surface: &str,
    joined: Option<rpc::JoinedRoom>,
) -> Response {
    let session_path = format!("/sessions/{}", super::hex(&entry.session));
    let (Html(body), page_path) = match (surface, joined) {
        ("play", Some(joined)) => {
            let Html(body) =
                room::player_page(state.actor.clone(), state.page.run_identity.clone()).await;
            let Some(scope) = player_scope(&joined, &state.page.run_identity) else {
                return refusal(Refusal::Unavailable);
            };
            (
                Html(body.replace(
                    "id=\"player\"",
                    &format!("id=\"player\" data-campaign-scope=\"{scope}\""),
                )),
                format!("{session_path}/play"),
            )
        }
        ("display", None) => match super::display(State(state.page.clone())).await {
            Ok(body) => (body, format!("{session_path}/display")),
            Err(_) => return refusal(Refusal::Unavailable),
        },
        _ => return concealed(),
    };
    actor::record_audit_line(
        "session.runtime_entry",
        format!(
            "SESSION runtime_entry session={} surface={surface} generation={}",
            super::hex(&entry.session),
            state.generation
        ),
    );
    private(
        Html(
            body.replace(
                "data-tunnel-path=\"/gameplay/tunnel\"",
                &format!("data-tunnel-path=\"{session_path}/tunnel\""),
            )
            .replace(
                "data-page-path=\"/gameplay/player\"",
                &format!("data-page-path=\"{page_path}\""),
            )
            .replace(
                "data-page-path=\"/gameplay/display\"",
                &format!("data-page-path=\"{page_path}\""),
            ),
        )
        .into_response(),
    )
}

fn player_scope(joined: &rpc::JoinedRoom, run_identity: &str) -> Option<String> {
    Some(format!(
        "v2.{}.{}.{}.{}.{}",
        joined.local_binding,
        super::hex(joined.session_id.as_ref()?.value.as_ref()?),
        super::hex(joined.run_id.as_ref()?.value.as_ref()?),
        super::hex(joined.client_binding_id.as_ref()?.value.as_ref()?),
        run_identity
    ))
}

fn refusal(reason: Refusal) -> Response {
    actor::record_audit_line(
        "session.runtime_refused",
        format!("SESSION runtime_refused reason={reason:?}"),
    );
    private(
        (
            match reason {
                Refusal::Occupied => StatusCode::CONFLICT,
                Refusal::InvalidPlacement => StatusCode::BAD_REQUEST,
                Refusal::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            },
            Json(reason),
        )
            .into_response(),
    )
}

fn concealed() -> Response {
    actor::record_audit_line(
        "session.runtime_refused",
        "SESSION runtime_refused reason=authorization".to_owned(),
    );
    private(StatusCode::NOT_FOUND.into_response())
}

fn private(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response
}

#[cfg(test)]
mod harness;
#[cfg(test)]
mod tests;
