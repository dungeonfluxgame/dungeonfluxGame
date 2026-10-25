#![cfg(not(target_arch = "wasm32"))]

mod support;

use axum::{Router, extract::WebSocketUpgrade, response::IntoResponse, routing::get};
use df_rpc_bridge::{
    ConnectionMetrics, ConnectionSnapshot, FRAME_BYTES, NativeIncoming, RECEIVE_BYTES, TunnelStream,
};
use futures::{SinkExt, StreamExt};
use std::{
    future::Future,
    io,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
    task::JoinHandle,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

type ClientSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct NativeCase {
    socket: ClientSocket,
    stream: TunnelStream,
    result: mpsc::Receiver<io::Result<()>>,
    metrics: Arc<ConnectionMetrics>,
    listener: JoinHandle<()>,
}

async fn accepted_case() -> NativeCase {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("owned listener");
    let address = listener.local_addr().expect("owned address");
    let (admission, mut incoming) =
        NativeIncoming::bounded(NonZeroUsize::new(1).expect("nonzero capacity"))
            .expect("bounded admission");
    let metrics = Arc::new(ConnectionMetrics::new(false));
    let (result_sender, result) = mpsc::channel(1);
    let app = Router::new().route(
        "/",
        get({
            let metrics = metrics.clone();
            move |upgrade: WebSocketUpgrade| {
                let metrics = metrics.clone();
                let result_sender = result_sender.clone();
                let permit = admission.try_reserve().expect("one admitted connection");
                async move {
                    upgrade
                        .on_upgrade(move |socket| async move {
                            let result = permit.accept_websocket_measured(socket, metrics).await;
                            let _ = result_sender.send(result).await;
                        })
                        .into_response()
                }
            }
        }),
    );
    let listener = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("owned Axum server");
    });
    let (socket, _) = connect_async(format!("ws://{address}/"))
        .await
        .expect("real WebSocket connect");
    let stream = tokio::time::timeout(Duration::from_secs(2), incoming.next())
        .await
        .expect("admitted stream wait")
        .expect("admitted stream")
        .expect("admitted stream error");
    NativeCase {
        socket,
        stream,
        result,
        metrics,
        listener,
    }
}

async fn pump_result(result: &mut mpsc::Receiver<io::Result<()>>) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(2), result.recv())
        .await
        .expect("native pump close bound")
        .expect("native pump result missing")
}

async fn finish(
    case: NativeCase,
    result: io::Result<()>,
    expected_error: Option<io::ErrorKind>,
) -> (String, ConnectionSnapshot) {
    let outcome = match &result {
        Ok(()) => "Ok".to_owned(),
        Err(error) => format!("{:?}", error.kind()),
    };
    match expected_error {
        Some(kind) => assert_eq!(result.expect_err("expected transport failure").kind(), kind),
        None => result.expect("clean native transport close"),
    }
    drop(case.stream);
    case.listener.abort();
    let _ = case.listener.await;
    let snapshot = case.metrics.snapshot().expect("native resource snapshot");
    assert!(snapshot.closed, "connection owner must close");
    assert_eq!(snapshot.pipe_owners, 0, "both byte pipes must release");
    assert_eq!(snapshot.websocket_receive.current, 0);
    assert_eq!(snapshot.pipe_to_grpc.current, 0);
    assert_eq!(snapshot.pipe_to_websocket.current, 0);
    assert_eq!(snapshot.rejected, expected_error.is_some());
    (outcome, snapshot)
}

#[derive(Default)]
struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn poll_pending<F: Future>(future: Pin<&mut F>, wakes: &Arc<WakeCount>) -> bool {
    let waker = Waker::from(wakes.clone());
    let mut context = Context::from_waker(&waker);
    matches!(future.poll(&mut context), Poll::Pending)
}

struct Witness {
    case: &'static str,
    pending_before_close: bool,
    woken_after_close: bool,
    buffered_before_close: bool,
    prefix_bytes: usize,
    terminal: String,
    pump: String,
    snapshot: ConnectionSnapshot,
}
impl Witness {
    fn json(&self) -> String {
        format!(
            concat!(
                "    {{\"case\":\"{}\",\"pending_before_close\":{},",
                "\"woken_after_close\":{},\"buffered_before_close\":{},",
                "\"prefix_bytes\":{},\"terminal\":\"{}\",\"pump\":\"{}\",",
                "\"closed\":{},\"rejected\":{},\"pipe_owners\":{},",
                "\"pipe_to_grpc_current\":{},\"pipe_to_websocket_current\":{},",
                "\"websocket_receive_current\":{}}}"
            ),
            self.case,
            self.pending_before_close,
            self.woken_after_close,
            self.buffered_before_close,
            self.prefix_bytes,
            self.terminal,
            self.pump,
            self.snapshot.closed,
            self.snapshot.rejected,
            self.snapshot.pipe_owners,
            self.snapshot.pipe_to_grpc.current,
            self.snapshot.pipe_to_websocket.current,
            self.snapshot.websocket_receive.current,
        )
    }
}

async fn idle_close() -> Witness {
    let mut case = accepted_case().await;
    let wakes = Arc::new(WakeCount::default());
    let mut byte = [0; 1];
    let mut read = Box::pin(case.stream.read(&mut byte));
    let pending = poll_pending(read.as_mut(), &wakes);
    assert!(pending, "idle read must be pending before WebSocket close");
    case.socket.close(None).await.expect("owned client close");
    let result = pump_result(&mut case.result).await;
    let woken = wakes.0.load(Ordering::Relaxed) > 0;
    assert!(woken, "close must wake the pending idle reader");
    let count = tokio::time::timeout(Duration::from_secs(2), read.as_mut())
        .await
        .expect("idle reader wake")
        .expect("idle reader result");
    assert_eq!(count, 0);
    drop(read);
    let (pump, snapshot) = finish(case, result, None).await;
    Witness {
        case: "idle",
        pending_before_close: pending,
        woken_after_close: woken,
        buffered_before_close: false,
        prefix_bytes: count,
        terminal: "Eof".to_owned(),
        pump,
        snapshot,
    }
}

async fn partial_close() -> Witness {
    let mut case = accepted_case().await;
    case.socket
        .send(Message::Binary(b"PRI * HT".to_vec().into()))
        .await
        .expect("partial real WebSocket message");
    let mut read = [0; 8];
    tokio::time::timeout(Duration::from_secs(2), case.stream.read_exact(&mut read))
        .await
        .expect("partial preface read bound")
        .expect("partial preface read");
    assert_eq!(&read, b"PRI * HT");
    let wakes = Arc::new(WakeCount::default());
    let mut tail = [0; 1];
    let mut pending_read = Box::pin(case.stream.read(&mut tail));
    let pending = poll_pending(pending_read.as_mut(), &wakes);
    assert!(
        pending,
        "partial read must be pending before WebSocket close"
    );
    case.socket.close(None).await.expect("owned client close");
    let result = pump_result(&mut case.result).await;
    let woken = wakes.0.load(Ordering::Relaxed) > 0;
    assert!(woken, "close must wake the pending partial reader");
    let count = tokio::time::timeout(Duration::from_secs(2), pending_read.as_mut())
        .await
        .expect("blocked reader wake")
        .expect("blocked reader result");
    assert_eq!(count, 0);
    drop(pending_read);
    let (pump, snapshot) = finish(case, result, Some(io::ErrorKind::UnexpectedEof)).await;
    Witness {
        case: "partial_preface",
        pending_before_close: pending,
        woken_after_close: woken,
        buffered_before_close: false,
        prefix_bytes: read.len(),
        terminal: "Eof".to_owned(),
        pump,
        snapshot,
    }
}

async fn pending_writer_close() -> Witness {
    let mut case = accepted_case().await;
    // Valid HEADERS frames exercise the passive observer without consuming DATA credit.
    // A single poll fills the bounded 1 MiB pipe before the pump gets scheduled.
    let mut framed = Vec::with_capacity(RECEIVE_BYTES + FRAME_BYTES * 2);
    while framed.len() <= RECEIVE_BYTES {
        framed.extend_from_slice(&[0, 0x40, 0, 1, 0, 0, 0, 0, 1]);
        framed.resize(framed.len() + FRAME_BYTES, 0);
    }
    let wakes = Arc::new(WakeCount::default());
    let mut write = Box::pin(case.stream.write_all(&framed));
    let pending = poll_pending(write.as_mut(), &wakes);
    assert!(
        pending,
        "bounded tunnel writer must be pending before close"
    );
    let buffered = case
        .metrics
        .snapshot()
        .expect("pending writer snapshot")
        .pipe_to_websocket
        .current
        > 0;
    assert!(buffered, "pending write must own buffered pipe bytes");
    case.socket.close(None).await.expect("owned client close");
    let result = pump_result(&mut case.result).await;
    let woken = wakes.0.load(Ordering::Relaxed) > 0;
    assert!(woken, "close must wake the pending writer");
    let error = tokio::time::timeout(Duration::from_secs(2), write.as_mut())
        .await
        .expect("pending writer wake")
        .expect_err("pending writer must not report full delivery after close");
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    let terminal = format!("{:?}", error.kind());
    drop(write);
    let (pump, snapshot) = finish(case, result, None).await;
    Witness {
        case: "pending_writer",
        pending_before_close: pending,
        woken_after_close: woken,
        buffered_before_close: buffered,
        prefix_bytes: 0,
        terminal,
        pump,
        snapshot,
    }
}

#[tokio::test]
async fn native_close_outcomes_match_consumed_result_derived_json() {
    let idle = idle_close().await;
    let partial = partial_close().await;
    let writer = pending_writer_close().await;
    let mut actual = String::from(
        "{\n  \"contract_version\": 2,\n  \"decision\": \"Only observed gRPC terminal trailers authorize a domain outcome; preterminal transport loss remains Unknown.\",\n  \"alternatives\": [\"Do not infer drain from WebSocket Close\", \"Do not replay an uncertain mutating RPC\", \"Do not classify by gRPC code text alone\"],\n  \"unsupported\": [\"Production operation-ID query/resync\", \"Authentication and permitted-view recovery\", \"Physical-device receive-memory qualification\"],\n  \"native_cases\": [\n",
    );
    actual.push_str(&idle.json());
    actual.push_str(",\n");
    actual.push_str(&partial.json());
    actual.push_str(",\n");
    actual.push_str(&writer.json());
    actual.push_str("\n  ]\n}\n");
    println!("RESULT_DERIVED_NATIVE_JSON={actual}");
    assert_eq!(actual, include_str!("close_reconnect_contract.json"));
}

#[tokio::test]
async fn fault_proxy_preserves_bytes_and_applies_seeded_latency() {
    use support::fault_proxy::{FaultProfile, FaultProxy};

    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("echo listener");
    let upstream_address = upstream.local_addr().expect("echo address");
    let echo = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = upstream.accept().await.expect("echo accept");
            let mut buffer = [0; 1024];
            loop {
                let count = stream.read(&mut buffer).await.expect("echo read");
                if count == 0 {
                    break;
                }
                stream
                    .write_all(&buffer[..count])
                    .await
                    .expect("echo write");
            }
        }
    });
    let baseline = FaultProxy::start(upstream_address, FaultProfile::default())
        .await
        .expect("baseline proxy");
    let measured = FaultProxy::start(
        upstream_address,
        FaultProfile {
            latency: Duration::from_millis(20),
            jitter: Duration::from_millis(5),
            seed: 42,
            ..FaultProfile::default()
        },
    )
    .await
    .expect("delayed proxy");
    let payload = vec![0x5a; 512];
    let baseline_ms = round_trip(baseline.address(), &payload).await;
    let delayed_ms = round_trip(measured.address(), &payload).await;
    println!("NET_PROXY_MEASURE bytes=512 baseline_ms={baseline_ms} injected_ms={delayed_ms}");
    assert!(delayed_ms >= baseline_ms + 30);
    echo.abort();
}

async fn round_trip(address: std::net::SocketAddr, payload: &[u8]) -> u128 {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("proxy connect");
    let started = std::time::Instant::now();
    stream.write_all(payload).await.expect("send payload");
    let mut received = vec![0; payload.len()];
    stream
        .read_exact(&mut received)
        .await
        .expect("receive payload");
    assert_eq!(received, payload);
    started.elapsed().as_millis()
}
