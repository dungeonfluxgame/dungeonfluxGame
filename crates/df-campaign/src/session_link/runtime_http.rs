//! One loopback runtime, with bounded HTTP exchanges and an owned opaque WebSocket relay.
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
        Ok(upgrade
            .max_message_size(MAXIMUM_TUNNEL_BYTES)
            .max_frame_size(MAXIMUM_TUNNEL_BYTES)
            .on_upgrade(move |client| async move {
                let outcome = relay(client, socket, deadline, relay_config).await;
                crate::audit::AuditLine::new("SESSION", "tunnel")
                    .reason(match outcome {
                        Ok(RelayEnd::ClientClosed) => "client_closed",
                        Ok(RelayEnd::UpstreamClosed) => "upstream_closed",
                        Ok(RelayEnd::IdleTimeout) => "idle_timeout",
                        Ok(RelayEnd::LifetimeLimit) => "lifetime_limit",
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
}

async fn relay(
    mut client: WebSocket,
    mut upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    deadline: Duration,
    config: RelayConfig,
) -> Result<RelayEnd, ()> {
    let started = tokio::time::Instant::now();
    let lifetime_deadline = started + config.maximum_lifetime;
    let mut idle_deadline = started + config.idle_timeout;
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
                tokio::time::timeout(deadline, client.send(message)).await.map_err(|_| ())?.map_err(|_| ())?;
                if application_data {
                    idle_deadline = tokio::time::Instant::now() + config.idle_timeout;
                }
            }
        }
    }
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
}
