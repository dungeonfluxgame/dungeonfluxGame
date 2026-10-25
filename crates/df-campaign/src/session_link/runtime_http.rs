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
    net::SocketAddr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

const MAXIMUM_PAGE_BYTES: usize = 256 * 1024;
const MAXIMUM_ALLOCATION_BYTES: usize = 4096;
const MAXIMUM_CONNECTIONS: usize = 32;
const MAXIMUM_TUNNEL_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub struct HttpRuntime {
    address: SocketAddr,
    secret: LaunchSecret,
    client: reqwest::Client,
    deadline: Duration,
    connections: std::sync::Arc<tokio::sync::Semaphore>,
}

impl HttpRuntime {
    /// `address` must be literal loopback; no DNS, redirects, proxies or remote runtime admission.
    pub fn new(
        address: SocketAddr,
        secret: LaunchSecret,
        deadline: Duration,
    ) -> Result<Self, RuntimeError> {
        if !address.ip().is_loopback() || address.port() == 0 || deadline.is_zero() {
            return Err(RuntimeError::Unavailable);
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

    async fn tunnel(&self, token: &str, request: Request) -> Result<Response, RuntimeError> {
        let permit = self
            .connections
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::Unavailable)?;
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
        Ok(upgrade
            .max_message_size(MAXIMUM_TUNNEL_BYTES)
            .max_frame_size(MAXIMUM_TUNNEL_BYTES)
            .on_upgrade(move |client| async move {
                let outcome = relay(client, socket, deadline).await;
                crate::audit::AuditLine::new("SESSION", "tunnel")
                    .reason(if outcome.is_ok() {
                        "closed"
                    } else {
                        "transport_failed"
                    })
                    .emit("session.tunnel");
                drop(permit);
            })
            .into_response())
    }
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
            return self.tunnel(&token, request).await;
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

async fn relay(
    mut client: WebSocket,
    mut upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    deadline: Duration,
) -> Result<(), ()> {
    loop {
        tokio::select! {
            message = client.next() => {
                let message = match message {
                    Some(Ok(Message::Binary(bytes))) => tungstenite::Message::Binary(bytes),
                    Some(Ok(Message::Ping(bytes))) => tungstenite::Message::Ping(bytes),
                    Some(Ok(Message::Pong(bytes))) => tungstenite::Message::Pong(bytes),
                    Some(Ok(Message::Close(_))) | None => {
                        tokio::time::timeout(deadline, upstream.close(None)).await.map_err(|_| ())?.map_err(|_| ())?;
                        return Ok(());
                    }
                    _ => return Err(()),
                };
                tokio::time::timeout(deadline, upstream.send(message)).await.map_err(|_| ())?.map_err(|_| ())?;
            }
            message = upstream.next() => {
                let message = match message {
                    Some(Ok(tungstenite::Message::Binary(bytes))) => Message::Binary(bytes),
                    Some(Ok(tungstenite::Message::Ping(bytes))) => Message::Ping(bytes),
                    Some(Ok(tungstenite::Message::Pong(bytes))) => Message::Pong(bytes),
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        tokio::time::timeout(deadline, client.close()).await.map_err(|_| ())?.map_err(|_| ())?;
                        return Ok(());
                    }
                    _ => return Err(()),
                };
                tokio::time::timeout(deadline, client.send(message)).await.map_err(|_| ())?.map_err(|_| ())?;
            }
        }
    }
}
