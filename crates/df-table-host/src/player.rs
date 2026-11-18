//! The player listener: an opt-in second listener (`serve --players N`) on every network address,
//! so a phone on the home network loads the phone page, stays live and comes back by itself.
//!
//! It owns that listener: its router, its guard, the watch over the open pages and the address it
//! announces. The router serves an explicit allow-list of handlers the dev router also serves (one
//! handler each, never a copy): the phone page at `df_playground::path::PLAY`, its files, the frame
//! long poll, the media and dice files, what the playground shows, the page's own report, and the
//! player routes of the seats (`seat::routes`), the seat's live pages (`seat_pages`) and the look
//! studio (`look::routes`). Anything else answers 404, so no `/dev/` route but the report is
//! reachable from the network. A phone changes only what its own seat key holds: it joins or
//! leaves a seat, chooses a hero and makes its look (the look's refines are paid image calls), says
//! ready or not ready, starts the game (which runs the table's scenario, when it has one), says its
//! hero's lines when the table asks, keeps notes in its own notebook and sends its page's report.
//! It never changes the shown scene directly. The guard (`guard::Audience::Players`) accepts only
//! this machine's own addresses (and loopback) on the player port and refuses a cross-site change;
//! the dev API's guard middleware (`server::guard_requests`) runs it, so refusals are written
//! alike.
//!
//! At startup it finds the address this machine reaches the network from, with no packet sent, and
//! writes `HOST player_listener url=http://<address>:<port>/play port=<port>` (or, when there is
//! none, `url=http://127.0.0.1:<port>/play port=<port> lan=none reason=..`). While it serves, its
//! watch writes the `CLIENT link state=quiet` line of a phone gone quiet and the `CLIENT gone` line
//! of a page gone, even when nothing else asks, and the `PLAYER traffic` line of what it sent.
//! Beside its seat's view, the phone still sees the same dev-mode phone view as the playground's
//! phone pane, which is not a private per-seat view.
//!
//! The loopback dev listener also serves `path::PHONE_LINK`, the phone page's address in large
//! text and as a QR code with the friends waiting for approval (`phone_link`), so the owner can
//! scan it from the PC's screen, and the seat routes and their dev routes.

mod phone_link;
mod traffic;

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::get;
use tokio::net::{TcpListener, UdpSocket};

use crate::guard::Audience;
use crate::limits::{CLIENT_WATCH_INTERVAL, MAXIMUM_PLAYER_REQUEST_BYTES, PLAYER_TRAFFIC_WINDOW};
use crate::server::{ListenerGuard, guard_requests};
use crate::state::AppState;
use crate::wire::route;

pub(crate) use self::traffic::PlayerTraffic;

/// The addresses of the player routes the dev listener serves.
pub mod path {
    /// The phone page's address in large text and as a QR code (GET, loopback dev listener only).
    pub const PHONE_LINK: &str = "/playground/phone-link";
}

/// A documentation address (RFC 5737) a socket is pointed at to learn which local address the
/// network route leaves from. Pointing a UDP socket sends nothing.
const ROUTE_PROBE: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9);

/// The player listener, bound and not yet serving, with the addresses it answers to.
#[derive(Debug)]
pub struct PlayerListener {
    listener: TcpListener,
    port: u16,
    addresses: Vec<IpAddr>,
    /// Why no network address was found, when none was.
    missing_address: Option<io::ErrorKind>,
}

impl PlayerListener {
    /// The phone page's address for each network address, or the loopback address when there is
    /// none (the page then opens on this machine only).
    pub fn links(&self) -> Vec<String> {
        if self.addresses.is_empty() {
            return vec![phone_page_link(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)];
        }
        self.addresses
            .iter()
            .map(|address| phone_page_link(*address, self.port))
            .collect()
    }

    /// Who this listener answers.
    pub fn audience(&self) -> Audience {
        Audience::Players {
            addresses: self.addresses.clone(),
            port: self.port,
        }
    }

    /// The audit lines that say where the phone page is.
    fn listening_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .links()
            .into_iter()
            .map(|link| format!("HOST player_listener url={link} port={}", self.port))
            .collect();
        if let (Some(kind), Some(line)) = (self.missing_address, lines.first_mut()) {
            line.push_str(&format!(" lan=none reason={}", network_cause_key(kind)));
        }
        lines
    }
}

/// A closed word for a socket failure, for an audit line.
fn network_cause_key(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NetworkUnreachable | io::ErrorKind::HostUnreachable => "network_unreachable",
        io::ErrorKind::AddrNotAvailable => "no_network_address",
        io::ErrorKind::PermissionDenied => "permission_denied",
        _ => "other",
    }
}

/// The phone page's address at `address` on `port`.
pub fn phone_page_link(address: IpAddr, port: u16) -> String {
    let host = match address {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    format!("http://{host}:{port}{}", df_playground::path::PLAY)
}

/// The local address the network route leaves from: a UDP socket pointed at `ROUTE_PROBE` picks
/// it, and sends nothing. A loopback or unspecified answer is no network address.
async fn network_address() -> io::Result<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.connect(ROUTE_PROBE).await?;
    let address = socket.local_addr()?.ip();
    if address.is_loopback() || address.is_unspecified() {
        return Err(io::Error::from(io::ErrorKind::AddrNotAvailable));
    }
    Ok(address)
}

/// Binds every network address on `port` (port zero picks a free port) and finds this machine's
/// network address. A machine with no network still listens, for loopback only.
pub async fn listen(port: u16) -> io::Result<PlayerListener> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    let port = listener.local_addr()?.port();
    let (addresses, missing_address) = match network_address().await {
        Ok(address) => (vec![address], None),
        Err(error) => (Vec::new(), Some(error.kind())),
    };
    Ok(PlayerListener {
        listener,
        port,
        addresses,
        missing_address,
    })
}

/// The player listener's routes: the allow-list, behind its guard. Any other address answers 404.
pub fn router(state: Arc<AppState>, audience: Audience) -> Router {
    routes(state, audience, Arc::default())
}

/// The player listener's routes, counting what they send in `traffic`: a JSON answer a phone
/// already has is a bodiless 304 (`traffic`).
pub(crate) fn routes(
    state: Arc<AppState>,
    audience: Audience,
    traffic: Arc<PlayerTraffic>,
) -> Router {
    let guard = ListenerGuard::new(Arc::clone(&state), audience);
    let reads = Router::new()
        // A phone is sent the frame as a phone draws it, never the shared screen's whole frame.
        .route(
            route::PLAYGROUND_FRAME,
            get(crate::server::player_frame_json),
        )
        // A phone is served only the media its answers gave it signed links to.
        .route(
            route::PLAYGROUND_MEDIA_FILE,
            get(crate::media_links::signed_media_file),
        )
        .with_state(Arc::clone(&state));
    crate::playground::player_router(Arc::clone(&state))
        .merge(crate::seat::routes::player_router(Arc::clone(&state)))
        .merge(crate::seat_pages::player_router(Arc::clone(&state)))
        .merge(crate::look::routes::player_router(Arc::clone(&state)))
        .merge(reads)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            crate::media_links::sign_answers,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&traffic),
            traffic::conditional_answers,
        ))
        .layer(DefaultBodyLimit::max(MAXIMUM_PLAYER_REQUEST_BYTES))
        // A phone's own change (a line, a note, a lobby action) wakes the pages waiting for a
        // frame, as a change sent to the dev API does.
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            crate::server::announce_changes,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&traffic),
            traffic::bounded_requests,
        ))
        .layer(middleware::from_fn_with_state(guard, guard_requests))
}

/// The routes of the player listener the loopback dev listener serves: the phone link page, the
/// phone's seat and live-page routes (so the live phone the playground shows beside the shared
/// screen joins and plays) and the seat dev routes.
pub(crate) fn dev_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(path::PHONE_LINK, get(phone_link::page))
        .with_state(Arc::clone(&state))
        .merge(crate::seat::routes::player_router(Arc::clone(&state)))
        .merge(crate::seat_pages::player_router(Arc::clone(&state)))
        .merge(crate::seat::routes::dev_router(state))
}

/// The player listener's tasks while it serves: abort both to stop it.
#[derive(Debug)]
pub struct PlayerTasks {
    pub serving: tokio::task::JoinHandle<()>,
    pub watch: tokio::task::JoinHandle<()>,
}

/// Records where the phone page is, remembers its links for the dev routes and starts serving and
/// watching. Each connection's address reaches the routes, so a phone's report says where it came
/// from. A listener that fails while serving writes `HOST player_listener_stopped cause=`.
pub fn start(listener: PlayerListener, state: &Arc<AppState>) -> PlayerTasks {
    for line in listener.listening_lines() {
        state.record_line(line);
    }
    state.set_player_links(listener.links());
    state.echo_new_lines();
    let traffic = Arc::new(PlayerTraffic::default());
    let routes = routes(Arc::clone(state), listener.audience(), Arc::clone(&traffic))
        .into_make_service_with_connect_info::<SocketAddr>();
    let serving_state = Arc::clone(state);
    let serving = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener.listener, routes).await {
            serving_state.record_line_and_echo(format!(
                "HOST player_listener_stopped cause={}",
                network_cause_key(error.kind())
            ));
        }
    });
    let watch = spawn_client_watch(Arc::clone(state), traffic);
    PlayerTasks { serving, watch }
}

/// Looks at the open pages every `CLIENT_WATCH_INTERVAL`, so a phone gone quiet and a page gone
/// are written while nothing else asks, and writes what the listener sent (`traffic`) every
/// `PLAYER_TRAFFIC_WINDOW`. It reads tokio's clock for the interval and the window only; the pages'
/// silence is measured on the playground's clock. Abort the handle to stop it.
fn spawn_client_watch(
    state: Arc<AppState>,
    traffic: Arc<PlayerTraffic>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLIENT_WATCH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut window_start = tokio::time::Instant::now();
        loop {
            interval.tick().await;
            crate::playground::sweep_clients(&state);
            let window = window_start.elapsed();
            if window >= PLAYER_TRAFFIC_WINDOW {
                window_start = tokio::time::Instant::now();
                if let Some(line) = traffic.take().line(window.as_secs()) {
                    state.record_line(line);
                }
            }
            state.echo_new_lines();
        }
    })
}
