//! The WebSocket tunnel's connection cap: a slot is released when its socket ends, however it
//! ends, so reloading a page any number of times never exhausts the cap.

use std::{sync::Arc, time::Duration};

use axum::{Router, routing::get};
use df_rpc_bridge::NativeIncoming;
use futures::{SinkExt, StreamExt};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, http::HeaderValue};

use super::{PageState, socket};

// Run the actual browser's admission and wait policies on native too, with controlled
// stream/deadline signals instead of browser timers or a gameplay database.
#[path = "../gameplay_browser/live_tv_status.rs"]
mod live_tv;
#[expect(
    clippy::duplicate_mod,
    reason = "native tests run the browser delivery policy; music_score loads the same file for its own tests"
)]
#[path = "../gameplay_browser/view_delivery.rs"]
mod view_delivery;

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
        splat_lod: 1,
        battle_grid: false,
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

#[tokio::test]
async fn presentation_timing_stalled_upload_retries_immutable_batch_and_reports_overflow() {
    use df_observe::presentation::{MAX_BUFFER_EVENTS, TimingBuffer, TimingEvent};
    use futures::channel::oneshot;

    let mut buffer = TimingBuffer::default();
    buffer
        .push(TimingEvent::lifecycle("scene", "ready", 1, 1, 0))
        .unwrap();
    let pending = buffer.batch().unwrap();
    let (expire, expired) = oneshot::channel::<()>();
    let upload = view_delivery::bounded_view_wait(futures::future::pending::<()>(), async {
        expired.await.unwrap();
    });
    futures::pin_mut!(upload);
    assert!(futures::poll!(upload.as_mut()).is_pending());
    for at_ms in 1..=MAX_BUFFER_EVENTS + 4 {
        let _overflow = buffer.push(TimingEvent::lifecycle("scene", "ready", 1, 1, at_ms as u64));
    }
    expire.send(()).unwrap();
    assert_eq!(upload.await, Err(view_delivery::ViewWaitExpired));
    assert_eq!(
        buffer.batch().unwrap(),
        pending,
        "retry identity and bytes survive timeout"
    );
    assert!(buffer.acknowledge(pending.sequence));
    let next = buffer.batch().unwrap();
    assert!(next.sequence > pending.sequence);
    assert_eq!(next.dropped, 5);
    assert!(
        !buffer.acknowledge(pending.sequence),
        "late receipts cannot release a newer batch"
    );
    assert!(buffer.acknowledge(next.sequence));
    assert!(
        buffer
            .push(TimingEvent::lifecycle("scene", "ready", 2, 2, 1_000))
            .is_ok()
    );
    let mut drained = 0;
    while let Ok(batch) = buffer.batch() {
        drained += 1;
        assert!(
            drained <= 4,
            "recovery must drain rather than retry an acknowledged prefix"
        );
        assert_eq!(batch.dropped, 5);
        assert!(buffer.acknowledge(batch.sequence));
    }
    assert!(drained > 0);
}
