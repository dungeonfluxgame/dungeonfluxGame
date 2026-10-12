//! The WebSocket tunnel's connection cap: a slot is released when its socket ends, however it
//! ends, so reloading a page any number of times never exhausts the cap.

use std::{sync::Arc, time::Duration};

use axum::{Router, routing::get};
use df_rpc_bridge::NativeIncoming;
use futures::{SinkExt, StreamExt};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, http::HeaderValue};

use super::{PageState, socket};

const CAP: usize = 2;
const ORIGIN: &str = "http://127.0.0.1";

type Client =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A tunnel server with `CAP` connection slots whose game side keeps every tunnel it accepts, as
/// a gRPC server holding its connections would, so only the page's socket decides when a slot
/// is free.
async fn tunnel() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let (admission, mut incoming) =
        NativeIncoming::bounded(std::num::NonZeroUsize::new(4).unwrap()).unwrap();
    let state = PageState {
        run_identity: Arc::from(""),
        incoming: admission,
        connections: Arc::new(Semaphore::new(CAP)),
        allowed_origins: vec![ORIGIN.to_owned()].into(),
        display: [0; 32],
        glue: bytes::Bytes::new(),
        wasm: bytes::Bytes::new(),
        battle_scene: None,
        battle_fps: 30,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/gameplay/tunnel", get(socket))
        .with_state(state);
    let server = tokio::spawn(async move {
        tokio::select! {
            _ = axum::serve(listener, app) => {}
            _ = async {
                let mut accepted = Vec::new();
                while let Some(connection) = incoming.next().await {
                    accepted.push(connection);
                }
            } => {}
        }
    });
    (address, server)
}

async fn connect(address: std::net::SocketAddr) -> Result<Client, tungstenite::Error> {
    let mut request = format!("ws://{address}/gameplay/tunnel")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", HeaderValue::from_static(ORIGIN));
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(client, _)| client)
}

/// Connects, allowing the server a moment to release the slot of the socket just ended.
async fn connect_soon(address: std::net::SocketAddr) -> Client {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match connect(address).await {
            Ok(client) => return client,
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _refused = error;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => panic!("no connection slot was ever released: {error}"),
        }
    }
}

#[tokio::test]
async fn more_sockets_than_the_cap_can_come_and_go_in_sequence() {
    let (address, server) = tunnel().await;
    for round in 0..(CAP * 6) {
        let mut client = connect_soon(address).await;
        if round % 2 == 0 {
            // A page that closes its socket properly.
            client
                .send(tungstenite::Message::Close(None))
                .await
                .unwrap();
            while client.next().await.is_some() {}
        } else {
            // A page that is reloaded: the connection drops without a closing handshake.
            drop(client);
        }
    }
    let last = connect_soon(address).await;
    drop(last);
    server.abort();
}

#[tokio::test]
async fn sockets_held_open_at_the_cap_are_refused_until_one_ends() {
    let (address, server) = tunnel().await;
    let held: Vec<Client> = vec![connect_soon(address).await, connect_soon(address).await];
    assert!(
        connect(address).await.is_err(),
        "the cap admitted another socket"
    );
    drop(held);
    let _again = connect_soon(address).await;
    server.abort();
}
