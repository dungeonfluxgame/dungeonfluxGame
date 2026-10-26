//! One loopback runtime, with bounded HTTP exchanges and an owned opaque WebSocket relay.
use super::routes::{TunnelAuthorization, TunnelRevalidator};
use super::{
    Audience, Placement, Room, RuntimeError, RuntimeLauncher, Surface, runtime_protocol::*,
};
use axum::{
    body::Body,
    extract::{
        FromRequestParts, Request, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use http::{HeaderValue, StatusCode, header};
use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

const MAXIMUM_PAGE_BYTES: usize = 256 * 1024;
const MAXIMUM_ALLOCATION_BYTES: usize = 4096;
const MAXIMUM_CONNECTIONS: usize = 32;
const MAXIMUM_TUNNEL_BYTES: usize = 256 * 1024;
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_TUNNEL_LIFETIME: Duration = Duration::from_secs(6 * 60 * 60);
const MAXIMUM_TUNNELS_PER_PRINCIPAL: usize = 2;
const REVOCATION_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const MAXIMUM_AUTHORITY_CHECK: Duration = Duration::from_millis(500);
#[cfg(test)]
const AUTHORITY_ACTIVE: u8 = 0;
#[cfg(test)]
const AUTHORITY_REVOKED: u8 = 1;
#[cfg(test)]
const AUTHORITY_UNAVAILABLE: u8 = 2;

/// Bounds the lifetime and concurrency of each upgraded session relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelayConfig {
    pub idle_timeout: Duration,
    pub maximum_lifetime: Duration,
    pub maximum_per_principal: usize,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            maximum_lifetime: DEFAULT_TUNNEL_LIFETIME,
            maximum_per_principal: MAXIMUM_TUNNELS_PER_PRINCIPAL,
        }
    }
}

impl RelayConfig {
    fn is_valid(self) -> bool {
        !self.idle_timeout.is_zero()
            && !self.maximum_lifetime.is_zero()
            && self.maximum_per_principal > 0
    }
}

#[derive(Clone)]
pub struct HttpRuntime {
    address: SocketAddr,
    secret: LaunchSecret,
    client: reqwest::Client,
    deadline: Duration,
    connections: std::sync::Arc<tokio::sync::Semaphore>,
    principal_connections: std::sync::Arc<std::sync::Mutex<HashMap<[u8; 17], usize>>>,
    relay_config: RelayConfig,
}

impl HttpRuntime {
    /// `address` must be literal loopback; no DNS, redirects, proxies or remote runtime admission.
    pub fn new(
        address: SocketAddr,
        secret: LaunchSecret,
        deadline: Duration,
    ) -> Result<Self, RuntimeError> {
        Self::new_with_relay_config(address, secret, deadline, RelayConfig::default())
    }

    /// Use explicit idle, absolute lifetime, and per-principal relay bounds.
    pub fn new_with_relay_config(
        address: SocketAddr,
        secret: LaunchSecret,
        deadline: Duration,
        relay_config: RelayConfig,
    ) -> Result<Self, RuntimeError> {
        if !address.ip().is_loopback() || address.port() == 0 || deadline.is_zero() {
            return Err(RuntimeError::Unavailable);
        }
        if !relay_config.is_valid() {
            return Err(RuntimeError::InvalidReply);
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(deadline)
            .timeout(deadline)
            .build()
            .map_err(|_| RuntimeError::Unavailable)?;
        Ok(Self {
            address,
            secret,
            client,
            deadline,
            connections: std::sync::Arc::new(tokio::sync::Semaphore::new(MAXIMUM_CONNECTIONS)),
            principal_connections: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            relay_config,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    fn display_ticket(&self, placement: &Placement) -> [u8; 16] {
        let mut identity = [0; 32];
        identity[..16].copy_from_slice(placement.campaign.as_bytes());
        identity[16..].copy_from_slice(placement.session.as_bytes());
        let digest = self.secret.derive(b"campaign display ticket v1", &identity);
        let mut ticket = [0; 16];
        ticket.copy_from_slice(&digest[..16]);
        ticket
    }

    fn token(
        &self,
        placement: &Placement,
        room: &Room,
        audience: Audience,
    ) -> Result<String, RuntimeError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::Unavailable)?
            .as_secs();
        self.secret
            .sign(&Entry {
                session: *placement.session.as_bytes(),
                campaign: *placement.campaign.as_bytes(),
                audience: match audience {
                    Audience::Player(member) => EntryAudience::Player(*member.as_bytes()),
                    Audience::Display => EntryAudience::Display(self.display_ticket(placement)),
                },
                generation: room.run_identity.clone(),
                expires_at_unix_seconds: now
                    .checked_add(ENTRY_LIFETIME_SECONDS)
                    .ok_or(RuntimeError::Unavailable)?,
            })
            .ok_or(RuntimeError::InvalidReply)
    }

    async fn tunnel(
        &self,
        placement: &Placement,
        token: &str,
        audience: Audience,
        request: Request,
    ) -> Result<Response, RuntimeError> {
        let permit = self.connections.clone().try_acquire_owned().map_err(|_| ());
        let Ok(permit) = permit else {
            record_tunnel_refusal();
            return Ok(StatusCode::TOO_MANY_REQUESTS.into_response());
        };
        let principal_key = match audience {
            Audience::Player(member) => {
                let mut key = [0; 17];
                key[0] = 1;
                key[1..].copy_from_slice(member.as_bytes());
                key
            }
            Audience::Display => {
                let mut key = [0; 17];
                key[0] = 2;
                key[1..].copy_from_slice(&self.display_ticket(placement));
                key
            }
        };
        let principal_permit = match PrincipalPermit::acquire(
            std::sync::Arc::clone(&self.principal_connections),
            principal_key,
            self.relay_config.maximum_per_principal,
        ) {
            Ok(permit) => permit,
            Err(()) => {
                record_tunnel_refusal();
                return Ok(StatusCode::TOO_MANY_REQUESTS.into_response());
            }
        };
        let (mut parts, _) = request.into_parts();
        let Some(revalidator) = parts.extensions.remove::<TunnelRevalidator>() else {
            return Err(RuntimeError::InvalidReply);
        };
        let upgrade = WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .map_err(|_| RuntimeError::InvalidReply)?;
        let mut upstream = format!("ws://{}{ENTRY_PATH}/tunnel", self.address)
            .into_client_request()
            .map_err(|_| RuntimeError::InvalidReply)?;
        upstream.headers_mut().insert(
            SECRET_HEADER,
            HeaderValue::from_bytes(self.secret.header_value())
                .map_err(|_| RuntimeError::Unavailable)?,
        );
        upstream.headers_mut().insert(
            ENTRY_HEADER,
            HeaderValue::from_str(token).map_err(|_| RuntimeError::InvalidReply)?,
        );
        if let Some(origin) = parts.headers.get(header::ORIGIN) {
            upstream
                .headers_mut()
                .insert(header::ORIGIN, origin.clone());
        }
        let config = tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAXIMUM_TUNNEL_BYTES))
            .max_frame_size(Some(MAXIMUM_TUNNEL_BYTES));
        let (socket, _) = tokio::time::timeout(
            self.deadline,
            tokio_tungstenite::connect_async_with_config(upstream, Some(config), false),
        )
        .await
        .map_err(|_| RuntimeError::Deadline)?
        .map_err(|_| RuntimeError::Unavailable)?;
        let deadline = self.deadline;
        let relay_config = self.relay_config;
        let campaign = placement.campaign;
        let session = placement.session;
        Ok(upgrade
            .max_message_size(MAXIMUM_TUNNEL_BYTES)
            .max_frame_size(MAXIMUM_TUNNEL_BYTES)
            .on_upgrade(move |client| async move {
                let outcome = relay(client, socket, deadline, relay_config, revalidator).await;
                crate::audit::AuditLine::new("SESSION", "tunnel")
                    .campaign(campaign)
                    .session(session)
                    .reason(match outcome {
                        Ok(RelayEnd::ClientClosed) => "client_closed",
                        Ok(RelayEnd::UpstreamClosed) => "upstream_closed",
                        Ok(RelayEnd::IdleTimeout) => "idle_timeout",
                        Ok(RelayEnd::LifetimeLimit) => "lifetime_limit",
                        Ok(RelayEnd::AccessRevoked) => "access_revoked",
                        Ok(RelayEnd::AuthorityUnavailable) => "authority_unavailable",
                        Err(()) => "transport_failed",
                    })
                    .emit("session.tunnel");
                drop(permit);
                drop(principal_permit);
            })
            .into_response())
    }
}

fn record_tunnel_refusal() {
    crate::audit::AuditLine::new("SESSION", "tunnel")
        .reason("capacity_refused")
        .emit("session.tunnel");
}

impl RuntimeLauncher for HttpRuntime {
    async fn allocate(&self, placement: &Placement) -> Result<Room, RuntimeError> {
        let mut allocation = Allocation::from(placement);
        allocation
            .display_tickets
            .push(self.display_ticket(placement));
        let response = self
            .client
            .post(self.url(ALLOCATION_PATH))
            .header(SECRET_HEADER, self.secret.header_value())
            .json(&allocation)
            .send()
            .await
            .map_err(|_| RuntimeError::Unavailable)?;
        let status = response.status();
        let bytes = bounded_body(response, MAXIMUM_ALLOCATION_BYTES).await?;
        if !status.is_success() {
            return match serde_json::from_slice::<Refusal>(&bytes) {
                Ok(Refusal::Occupied) => Err(RuntimeError::Occupied),
                Ok(Refusal::InvalidPlacement) => Err(RuntimeError::InvalidReply),
                _ => Err(RuntimeError::Unavailable),
            };
        }
        let room: Allocated =
            serde_json::from_slice(&bytes).map_err(|_| RuntimeError::InvalidReply)?;
        Ok(Room {
            code: room.code,
            run_identity: room.generation,
        })
    }

    async fn serve(
        &self,
        placement: &Placement,
        room: &Room,
        audience: Audience,
        surface: Surface,
        request: Request,
    ) -> Result<Response, RuntimeError> {
        let token = self.token(placement, room, audience)?;
        if surface == Surface::Tunnel {
            return self.tunnel(placement, &token, audience, request).await;
        }
        let path = match surface {
            Surface::Play => "play",
            Surface::Display => "display",
            Surface::Tunnel => return Err(RuntimeError::InvalidReply),
        };
        let response = self
            .client
            .get(self.url(&format!("{ENTRY_PATH}/{path}")))
            .header(SECRET_HEADER, self.secret.header_value())
            .header(ENTRY_HEADER, token)
            .send()
            .await
            .map_err(|_| RuntimeError::Unavailable)?;
        if response.status() != StatusCode::OK {
            return Err(RuntimeError::Unavailable);
        }
        let body = bounded_body(response, MAXIMUM_PAGE_BYTES).await?;
        Ok((
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
                (header::REFERRER_POLICY, "no-referrer"),
            ],
            Body::from(body),
        )
            .into_response())
    }
}

async fn bounded_body(
    mut response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>, RuntimeError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| RuntimeError::Unavailable)?
    {
        if chunk.len() > maximum.saturating_sub(body.len()) {
            return Err(RuntimeError::InvalidReply);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

struct PrincipalPermit {
    connections: std::sync::Arc<std::sync::Mutex<HashMap<[u8; 17], usize>>>,
    key: [u8; 17],
}

impl PrincipalPermit {
    fn acquire(
        connections: std::sync::Arc<std::sync::Mutex<HashMap<[u8; 17], usize>>>,
        key: [u8; 17],
        maximum: usize,
    ) -> Result<Self, ()> {
        let admitted = {
            let mut active = match connections.lock() {
                Ok(active) => active,
                Err(poisoned) => poisoned.into_inner(),
            };
            let count = active.entry(key).or_default();
            if *count >= maximum {
                false
            } else {
                *count += 1;
                true
            }
        };
        if !admitted {
            return Err(());
        }
        Ok(Self { connections, key })
    }
}

impl Drop for PrincipalPermit {
    fn drop(&mut self) {
        let mut active = match self.connections.lock() {
            Ok(active) => active,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(count) = active.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                active.remove(&self.key);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum RelayEnd {
    ClientClosed,
    UpstreamClosed,
    IdleTimeout,
    LifetimeLimit,
    AccessRevoked,
    AuthorityUnavailable,
}

async fn relay(
    mut client: WebSocket,
    mut upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    deadline: Duration,
    config: RelayConfig,
    revalidator: TunnelRevalidator,
) -> Result<RelayEnd, ()> {
    let started = tokio::time::Instant::now();
    let lifetime_deadline = started + config.maximum_lifetime;
    let mut idle_deadline = started + config.idle_timeout;
    let mut revocation_check = tokio::time::interval_at(
        started + REVOCATION_CHECK_INTERVAL,
        REVOCATION_CHECK_INTERVAL,
    );
    revocation_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(idle_deadline.min(lifetime_deadline)) => {
                close_both(&mut client, &mut upstream, deadline).await;
                return Ok(if lifetime_deadline <= idle_deadline {
                    RelayEnd::LifetimeLimit
                } else {
                    RelayEnd::IdleTimeout
                });
            }
            _ = revocation_check.tick() => {
                match check_tunnel_access(&revalidator, deadline).await {
                    TunnelAuthorization::Authorized => {}
                    TunnelAuthorization::Revoked => {
                        close_both(
                            &mut client,
                            &mut upstream,
                            deadline.min(MAXIMUM_AUTHORITY_CHECK),
                        )
                        .await;
                        return Ok(RelayEnd::AccessRevoked);
                    }
                    TunnelAuthorization::Unavailable => {
                        close_both(
                            &mut client,
                            &mut upstream,
                            deadline.min(MAXIMUM_AUTHORITY_CHECK),
                        )
                        .await;
                        return Ok(RelayEnd::AuthorityUnavailable);
                    }
                }
            }
            message = client.next() => {
                let message = match message {
                    Some(Ok(Message::Text(text))) => (tungstenite::Message::Text(text.to_string().into()), true),
                    Some(Ok(Message::Binary(bytes))) => (tungstenite::Message::Binary(bytes), true),
                    Some(Ok(Message::Ping(bytes))) => (tungstenite::Message::Ping(bytes), false),
                    Some(Ok(Message::Pong(bytes))) => (tungstenite::Message::Pong(bytes), false),
                    Some(Ok(Message::Close(_))) | None => {
                        close_both(&mut client, &mut upstream, deadline).await;
                        return Ok(RelayEnd::ClientClosed);
                    }
                    _ => return Err(()),
                };
                let (message, application_data) = message;
                match check_tunnel_access(&revalidator, deadline).await {
                    TunnelAuthorization::Authorized => {}
                    TunnelAuthorization::Revoked => {
                        close_both(
                            &mut client,
                            &mut upstream,
                            deadline.min(MAXIMUM_AUTHORITY_CHECK),
                        )
                        .await;
                        return Ok(RelayEnd::AccessRevoked);
                    }
                    TunnelAuthorization::Unavailable => {
                        close_both(
                            &mut client,
                            &mut upstream,
                            deadline.min(MAXIMUM_AUTHORITY_CHECK),
                        )
                        .await;
                        return Ok(RelayEnd::AuthorityUnavailable);
                    }
                }
                tokio::time::timeout(deadline, upstream.send(message)).await.map_err(|_| ())?.map_err(|_| ())?;
                if application_data {
                    idle_deadline = tokio::time::Instant::now() + config.idle_timeout;
                }
            }
            message = upstream.next() => {
                let message = match message {
                    Some(Ok(tungstenite::Message::Text(text))) => (Message::Text(text.to_string().into()), true),
                    Some(Ok(tungstenite::Message::Binary(bytes))) => (Message::Binary(bytes), true),
                    Some(Ok(tungstenite::Message::Ping(bytes))) => (Message::Ping(bytes), false),
                    Some(Ok(tungstenite::Message::Pong(bytes))) => (Message::Pong(bytes), false),
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        close_both(&mut client, &mut upstream, deadline).await;
                        return Ok(RelayEnd::UpstreamClosed);
                    }
                    _ => return Err(()),
                };
                let (message, application_data) = message;
                match check_tunnel_access(&revalidator, deadline).await {
                    TunnelAuthorization::Authorized => {}
                    TunnelAuthorization::Revoked => {
                        close_both(
                            &mut client,
                            &mut upstream,
                            deadline.min(MAXIMUM_AUTHORITY_CHECK),
                        )
                        .await;
                        return Ok(RelayEnd::AccessRevoked);
                    }
                    TunnelAuthorization::Unavailable => {
                        close_both(
                            &mut client,
                            &mut upstream,
                            deadline.min(MAXIMUM_AUTHORITY_CHECK),
                        )
                        .await;
                        return Ok(RelayEnd::AuthorityUnavailable);
                    }
                }
                tokio::time::timeout(deadline, client.send(message)).await.map_err(|_| ())?.map_err(|_| ())?;
                if application_data {
                    idle_deadline = tokio::time::Instant::now() + config.idle_timeout;
                }
            }
        }
    }
}

async fn check_tunnel_access(
    revalidator: &TunnelRevalidator,
    deadline: Duration,
) -> TunnelAuthorization {
    tokio::time::timeout(deadline.min(MAXIMUM_AUTHORITY_CHECK), revalidator.check())
        .await
        .unwrap_or(TunnelAuthorization::Unavailable)
}

async fn close_both(
    client: &mut WebSocket,
    upstream: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    deadline: Duration,
) {
    let client_close = Message::Close(Some(axum::extract::ws::CloseFrame {
        code: 1000,
        reason: "Reconnect required".into(),
    }));
    let upstream_close = tungstenite::Message::Close(Some(tungstenite::protocol::CloseFrame {
        code: tungstenite::protocol::frame::coding::CloseCode::Normal,
        reason: "Reconnect required".into(),
    }));
    let _client_result = tokio::time::timeout(deadline, client.send(client_close)).await;
    let _upstream_result = tokio::time::timeout(deadline, upstream.send(upstream_close)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::{Path, State},
        routing::get,
    };
    use tokio::{
        net::TcpListener,
        sync::{broadcast, mpsc, oneshot},
        task::JoinHandle,
    };

    type ClientSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    #[derive(Clone)]
    struct UpstreamState {
        outbound: broadcast::Sender<String>,
        events: mpsc::UnboundedSender<String>,
    }

    #[derive(Clone)]
    struct FrontState {
        runtime: HttpRuntime,
        placement: Placement,
        room: Room,
        authority: std::sync::Arc<std::sync::atomic::AtomicU8>,
    }

    struct RelayHarness {
        address: SocketAddr,
        outbound: broadcast::Sender<String>,
        events: mpsc::UnboundedReceiver<String>,
        authority: std::sync::Arc<std::sync::atomic::AtomicU8>,
        front_shutdown: Option<oneshot::Sender<()>>,
        upstream_shutdown: Option<oneshot::Sender<()>>,
        front_task: JoinHandle<()>,
        upstream_task: JoinHandle<()>,
    }

    impl RelayHarness {
        async fn start(config: RelayConfig) -> Self {
            let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream_address = upstream_listener.local_addr().unwrap();
            let (outbound, _) = broadcast::channel(32);
            let (events_tx, mut events) = mpsc::unbounded_channel();
            let upstream_router = Router::new()
                .route("/gameplay/internal/entry/tunnel", get(upstream_entry))
                .with_state(UpstreamState {
                    outbound: outbound.clone(),
                    events: events_tx,
                });
            let (upstream_shutdown, upstream_shutdown_rx) = oneshot::channel();
            let upstream_task = tokio::spawn(async move {
                axum::serve(upstream_listener, upstream_router)
                    .with_graceful_shutdown(async {
                        let _ = upstream_shutdown_rx.await;
                    })
                    .await
                    .unwrap();
            });
            tokio::task::yield_now().await;
            let (mut upstream_probe, _) = tokio_tungstenite::connect_async(format!(
                "ws://{upstream_address}/gameplay/internal/entry/tunnel"
            ))
            .await
            .expect("loopback upstream WebSocket fixture failed its own handshake");
            assert_eq!(events.recv().await.as_deref(), Some("ready"));
            upstream_probe.close(None).await.unwrap();
            assert_eq!(events.recv().await.as_deref(), Some("closed"));

            let secret =
                LaunchSecret::new(&format!("relay-test-secret-{}", "x".repeat(24))).unwrap();
            let runtime = HttpRuntime::new_with_relay_config(
                upstream_address,
                secret,
                Duration::from_secs(60),
                config,
            )
            .unwrap();
            let front_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = front_listener.local_addr().unwrap();
            let authority = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(AUTHORITY_ACTIVE));
            let front_router = Router::new()
                .route("/tunnel/{seat}", get(front_entry))
                .with_state(FrontState {
                    runtime,
                    placement: Placement {
                        campaign: df_types::CampaignId::from_bytes(&[1; 16]).unwrap(),
                        session: df_types::SessionId::from_bytes(&[2; 16]).unwrap(),
                        roster: Vec::new(),
                    },
                    room: Room {
                        code: "RELAY".to_owned(),
                        run_identity: "relay-test-run".to_owned(),
                    },
                    authority: std::sync::Arc::clone(&authority),
                });
            let (front_shutdown, front_shutdown_rx) = oneshot::channel();
            let front_task = tokio::spawn(async move {
                axum::serve(front_listener, front_router)
                    .with_graceful_shutdown(async {
                        let _ = front_shutdown_rx.await;
                    })
                    .await
                    .unwrap();
            });
            tokio::task::yield_now().await;

            Self {
                address,
                outbound,
                events,
                authority,
                front_shutdown: Some(front_shutdown),
                upstream_shutdown: Some(upstream_shutdown),
                front_task,
                upstream_task,
            }
        }

        async fn connect(&self, seat: u8) -> ClientSocket {
            let (socket, response) =
                tokio_tungstenite::connect_async(format!("ws://{}/tunnel/{seat}", self.address))
                    .await
                    .unwrap_or_else(|error| {
                        panic!("seat {seat} WebSocket upgrade failed: {error}")
                    });
            assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
            socket
        }

        async fn expect_upstream_ready(&mut self) {
            let event = tokio::time::timeout(Duration::from_secs(2), self.events.recv())
                .await
                .expect("upstream handshake did not reach its WebSocket handler");
            assert_eq!(event.as_deref(), Some("ready"));
        }

        async fn close(self) {
            let _ = self.front_shutdown.unwrap().send(());
            let _ = self.upstream_shutdown.unwrap().send(());
            self.front_task.await.unwrap();
            self.upstream_task.await.unwrap();
        }
    }

    async fn upstream_entry(
        upgrade: WebSocketUpgrade,
        State(state): State<UpstreamState>,
    ) -> Response {
        upgrade
            .on_upgrade(move |socket| run_upstream(socket, state))
            .into_response()
    }

    async fn run_upstream(mut socket: WebSocket, state: UpstreamState) {
        let mut outbound = state.outbound.subscribe();
        let _ = state.events.send("ready".to_owned());
        loop {
            tokio::select! {
                incoming = socket.next() => match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let _ = state.events.send(text.to_string());
                    }
                    Some(Ok(Message::Ping(bytes))) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                },
                outgoing = outbound.recv() => match outgoing {
                    Ok(message) => {
                        if socket.send(Message::Text(message.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                },
            }
        }
        let _ = state.events.send("closed".to_owned());
    }

    async fn front_entry(
        State(state): State<FrontState>,
        Path(seat): Path<u8>,
        request: Request,
    ) -> Response {
        let member = df_types::MemberId::from_bytes(&[seat + 1; 16]).unwrap();
        let authority = std::sync::Arc::clone(&state.authority);
        let (mut parts, body) = request.into_parts();
        parts.extensions.insert(TunnelRevalidator::new(move || {
            let authority = std::sync::Arc::clone(&authority);
            Box::pin(async move {
                match authority.load(std::sync::atomic::Ordering::SeqCst) {
                    AUTHORITY_ACTIVE => TunnelAuthorization::Authorized,
                    AUTHORITY_REVOKED => TunnelAuthorization::Revoked,
                    _ => TunnelAuthorization::Unavailable,
                }
            })
        }));
        let request = Request::from_parts(parts, body);
        let audience = if seat == 0 {
            Audience::Display
        } else {
            Audience::Player(member)
        };
        match state
            .runtime
            .serve(
                &state.placement,
                &state.room,
                audience,
                Surface::Tunnel,
                request,
            )
            .await
        {
            Ok(response) => response,
            Err(RuntimeError::Deadline) => StatusCode::GATEWAY_TIMEOUT.into_response(),
            Err(RuntimeError::Unavailable) => StatusCode::BAD_GATEWAY.into_response(),
            Err(RuntimeError::Occupied | RuntimeError::InvalidReply) => {
                StatusCode::BAD_REQUEST.into_response()
            }
        }
    }

    async fn expect_close(socket: &mut ClientSocket) {
        let close = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match socket.next().await {
                    Some(Ok(tungstenite::Message::Close(_))) | None => break true,
                    Some(Err(_)) => break true,
                    Some(Ok(_)) => {}
                }
            }
        })
        .await;
        assert!(
            close.is_ok(),
            "relay did not close before the bounded timeout"
        );
    }

    async fn expect_upstream_data(harness: &mut RelayHarness, expected: &str) {
        let event = tokio::time::timeout(Duration::from_secs(2), harness.events.recv())
            .await
            .expect("client data did not reach the upstream WebSocket");
        assert_eq!(event.as_deref(), Some(expected));
    }

    async fn expect_upstream_closed(harness: &mut RelayHarness, forbidden: &str) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match harness.events.recv().await.as_deref() {
                    Some("closed") => break,
                    Some(event) => assert_ne!(event, forbidden),
                    None => panic!("upstream event channel closed before the socket"),
                }
            }
        })
        .await
        .expect("upstream tunnel did not close within the revocation bound");
    }

    async fn expect_close_without(socket: &mut ClientSocket, forbidden: &str) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match socket.next().await {
                    Some(Ok(tungstenite::Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(tungstenite::Message::Text(text))) => assert_ne!(text, forbidden),
                    Some(Ok(_)) => {}
                }
            }
        })
        .await
        .expect("client tunnel did not close before the bounded timeout");
    }

    #[test]
    fn relay_configuration_defaults_bound_idle_lifetime_and_principal_count() {
        assert_eq!(
            RelayConfig::default(),
            RelayConfig {
                idle_timeout: Duration::from_secs(120),
                maximum_lifetime: Duration::from_secs(6 * 60 * 60),
                maximum_per_principal: 2,
            }
        );
    }

    #[test]
    fn principal_capacity_refuses_a_third_tunnel_and_releases_every_exit() {
        let connections = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        let seat = [1; 17];
        let other = [2; 17];
        let first = PrincipalPermit::acquire(std::sync::Arc::clone(&connections), seat, 2).unwrap();
        let second =
            PrincipalPermit::acquire(std::sync::Arc::clone(&connections), seat, 2).unwrap();
        assert!(PrincipalPermit::acquire(std::sync::Arc::clone(&connections), seat, 2).is_err());
        let other_seat =
            PrincipalPermit::acquire(std::sync::Arc::clone(&connections), other, 2).unwrap();

        drop(first);
        let replacement =
            PrincipalPermit::acquire(std::sync::Arc::clone(&connections), seat, 2).unwrap();
        drop((second, other_seat, replacement));
        assert!(connections.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn idle_relays_close_and_return_global_and_principal_permits() {
        let harness = RelayHarness::start(RelayConfig {
            idle_timeout: Duration::from_secs(5),
            maximum_lifetime: Duration::from_secs(60),
            maximum_per_principal: 2,
        })
        .await;
        let mut sockets = vec![harness.connect(0).await, harness.connect(0).await];
        for seat in 1..=30 {
            sockets.push(harness.connect(seat).await);
        }

        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::time::resume();
        for socket in &mut sockets {
            expect_close(socket).await;
        }

        let _first_replacement = harness.connect(0).await;
        let _second_replacement = harness.connect(0).await;
        drop((_first_replacement, _second_replacement, sockets));
        harness.close().await;
    }

    #[tokio::test]
    async fn ping_only_traffic_does_not_refresh_the_idle_window() {
        let mut harness = RelayHarness::start(RelayConfig {
            idle_timeout: Duration::from_secs(5),
            maximum_lifetime: Duration::from_secs(60),
            maximum_per_principal: 2,
        })
        .await;
        let mut socket = harness.connect(1).await;
        harness.expect_upstream_ready().await;

        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(4)).await;
        tokio::time::resume();
        socket
            .send(tungstenite::Message::Ping(vec![1].into()))
            .await
            .unwrap();
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            tungstenite::Message::Pong(_)
        ));
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::time::resume();
        expect_close(&mut socket).await;
        drop(socket);
        harness.close().await;
    }

    #[tokio::test]
    async fn client_data_refreshes_the_idle_window() {
        let mut harness = RelayHarness::start(RelayConfig {
            idle_timeout: Duration::from_secs(5),
            maximum_lifetime: Duration::from_secs(60),
            maximum_per_principal: 2,
        })
        .await;
        let mut socket = harness.connect(1).await;
        harness.expect_upstream_ready().await;

        tokio::time::pause();
        for _ in 0..2 {
            tokio::time::advance(Duration::from_secs(4)).await;
            tokio::time::resume();
            socket
                .send(tungstenite::Message::Text("client-data".into()))
                .await
                .unwrap();
            expect_upstream_data(&mut harness, "client-data").await;
            tokio::time::pause();
        }
        tokio::time::resume();
        drop(socket);
        harness.close().await;
    }

    #[tokio::test]
    async fn upstream_data_refreshes_the_idle_window() {
        let mut harness = RelayHarness::start(RelayConfig {
            idle_timeout: Duration::from_secs(5),
            maximum_lifetime: Duration::from_secs(60),
            maximum_per_principal: 2,
        })
        .await;
        let mut socket = harness.connect(1).await;
        harness.expect_upstream_ready().await;

        tokio::time::pause();
        for _ in 0..2 {
            tokio::time::advance(Duration::from_secs(4)).await;
            tokio::time::resume();
            harness.outbound.send("upstream-data".to_owned()).unwrap();
            assert!(matches!(
                socket.next().await.unwrap().unwrap(),
                tungstenite::Message::Text(text) if text == "upstream-data"
            ));
            tokio::time::pause();
        }
        tokio::time::resume();
        drop(socket);
        harness.close().await;
    }

    #[tokio::test]
    async fn active_relays_close_at_the_absolute_lifetime_limit() {
        let mut harness = RelayHarness::start(RelayConfig {
            idle_timeout: Duration::from_secs(5),
            maximum_lifetime: Duration::from_secs(12),
            maximum_per_principal: 2,
        })
        .await;
        let mut socket = harness.connect(1).await;
        harness.expect_upstream_ready().await;

        tokio::time::pause();
        for elapsed in [4, 4, 3] {
            tokio::time::advance(Duration::from_secs(elapsed)).await;
            tokio::time::resume();
            socket
                .send(tungstenite::Message::Text("keepalive-data".into()))
                .await
                .unwrap();
            expect_upstream_data(&mut harness, "keepalive-data").await;
            tokio::time::pause();
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::time::resume();
        expect_close(&mut socket).await;
        drop(socket);
        harness.close().await;
    }

    #[tokio::test]
    async fn a_third_tunnel_is_refused_per_seat_while_another_seat_connects() {
        let harness = RelayHarness::start(RelayConfig::default()).await;
        let _first = harness.connect(1).await;
        let _second = harness.connect(1).await;
        let refused =
            tokio_tungstenite::connect_async(format!("ws://{}/tunnel/1", harness.address)).await;
        assert!(matches!(
            refused,
            Err(tokio_tungstenite::tungstenite::Error::Http(response))
                if response.status() == StatusCode::TOO_MANY_REQUESTS
        ));
        let _other_seat = harness.connect(2).await;
        drop((_first, _second, _other_seat));
        harness.close().await;
    }

    #[tokio::test]
    async fn revoking_an_admitted_member_closes_both_tunnel_peers_before_more_frames_cross() {
        let mut harness = RelayHarness::start(RelayConfig::default()).await;
        let mut socket = harness.connect(1).await;
        harness.expect_upstream_ready().await;
        socket
            .send(tungstenite::Message::Text("before-revocation".into()))
            .await
            .unwrap();
        expect_upstream_data(&mut harness, "before-revocation").await;

        harness
            .outbound
            .send("private-before-revocation".to_owned())
            .unwrap();
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            tungstenite::Message::Text(text) if text == "private-before-revocation"
        ));

        harness
            .authority
            .store(AUTHORITY_REVOKED, std::sync::atomic::Ordering::SeqCst);
        socket
            .send(tungstenite::Message::Text("after-revocation".into()))
            .await
            .unwrap();
        expect_close(&mut socket).await;
        expect_upstream_closed(&mut harness, "after-revocation").await;
        assert!(
            harness
                .outbound
                .send("private-after-revocation".to_owned())
                .is_err()
        );
        drop(socket);
        harness.close().await;
    }

    #[tokio::test]
    async fn an_authority_outage_closes_both_peers_without_forwarding_input() {
        let mut harness = RelayHarness::start(RelayConfig::default()).await;
        let mut socket = harness.connect(1).await;
        harness.expect_upstream_ready().await;
        harness
            .authority
            .store(AUTHORITY_UNAVAILABLE, std::sync::atomic::Ordering::SeqCst);
        socket
            .send(tungstenite::Message::Text("during-authority-outage".into()))
            .await
            .unwrap();
        expect_close(&mut socket).await;
        expect_upstream_closed(&mut harness, "during-authority-outage").await;
        drop(socket);
        let replacement = harness.connect(1).await;
        drop(replacement);
        harness.close().await;
    }

    #[tokio::test]
    async fn a_revoked_display_tunnel_does_not_deliver_private_output() {
        let mut harness = RelayHarness::start(RelayConfig::default()).await;
        let mut socket = harness.connect(0).await;
        harness.expect_upstream_ready().await;
        harness
            .authority
            .store(AUTHORITY_REVOKED, std::sync::atomic::Ordering::SeqCst);
        harness
            .outbound
            .send("private-after-display-revocation".to_owned())
            .unwrap();
        expect_close_without(&mut socket, "private-after-display-revocation").await;
        expect_upstream_closed(&mut harness, "private-after-display-revocation").await;
        drop(socket);
        harness.close().await;
    }

    #[tokio::test]
    async fn an_archived_campaign_closes_an_admitted_member_tunnel_before_private_output() {
        let mut harness = RelayHarness::start(RelayConfig::default()).await;
        let mut socket = harness.connect(1).await;
        harness.expect_upstream_ready().await;
        harness
            .authority
            .store(AUTHORITY_REVOKED, std::sync::atomic::Ordering::SeqCst);
        harness
            .outbound
            .send("private-after-campaign-archive".to_owned())
            .unwrap();
        expect_close_without(&mut socket, "private-after-campaign-archive").await;
        expect_upstream_closed(&mut harness, "private-after-campaign-archive").await;
        drop(socket);
        harness.close().await;
    }

    #[tokio::test]
    async fn an_idle_tunnel_detects_member_revocation_within_the_poll_bound() {
        let mut harness = RelayHarness::start(RelayConfig::default()).await;
        let mut socket = harness.connect(1).await;
        harness.expect_upstream_ready().await;
        harness
            .authority
            .store(AUTHORITY_REVOKED, std::sync::atomic::Ordering::SeqCst);
        tokio::time::pause();
        tokio::time::advance(REVOCATION_CHECK_INTERVAL * 2).await;
        tokio::time::resume();
        expect_close(&mut socket).await;
        expect_upstream_closed(&mut harness, "").await;
        drop(socket);
        harness.close().await;
    }
}
