#![cfg(not(target_arch = "wasm32"))]

use std::{
    io,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Router,
    extract::ws::{WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
};
use bytes::{Bytes, BytesMut};
use df_rpc_bridge::{
    AdmissionError, ConnectionMetrics, FRAME_BYTES, MESSAGE_BYTES, NativeIncoming,
};
use futures::{Sink, SinkExt, Stream, StreamExt};
use http::{Request, Response as HttpResponse, StatusCode};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    net::TcpListener,
    task::JoinHandle,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

type ClientWebSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const MAX_COALESCED_WRITE_BYTES: usize = 64 * 1024;
const FIRST_BODY_MARKER: &[u8] = b"first-body-marker";
const LAST_BODY_MARKER: &[u8] = b"last-body-marker";

struct WebSocketIo {
    socket: ClientWebSocket,
    read_buffer: BytesMut,
    write_limit: usize,
    sent_messages: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl WebSocketIo {
    fn new(
        socket: ClientWebSocket,
        write_limit: usize,
        sent_messages: Arc<Mutex<Vec<Vec<u8>>>>,
    ) -> Self {
        Self {
            socket,
            read_buffer: BytesMut::new(),
            write_limit,
            sent_messages,
        }
    }
}

impl AsyncRead for WebSocketIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.as_mut().get_mut();
        loop {
            if !this.read_buffer.is_empty() {
                let count = buffer.remaining().min(this.read_buffer.len());
                buffer.put_slice(&this.read_buffer.split_to(count));
                return Poll::Ready(Ok(()));
            }

            match Pin::new(&mut this.socket).poll_next(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Err(error))) => {
                    return Poll::Ready(Err(io::Error::other(error)));
                }
                Poll::Ready(Some(Ok(Message::Binary(bytes)))) => {
                    this.read_buffer.extend_from_slice(&bytes);
                }
                Poll::Ready(Some(Ok(Message::Close(_)))) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Ok(_))) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected a binary RPC tunnel message",
                    )));
                }
            }
        }
    }
}

impl AsyncWrite for WebSocketIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let this = self.as_mut().get_mut();
        let count = bytes.len().min(this.write_limit);
        match Pin::new(&mut this.socket).poll_ready(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(io::Error::other(error))),
            Poll::Ready(Ok(())) => {
                let message = Message::Binary(bytes[..count].to_vec().into());
                match Pin::new(&mut this.socket).start_send(message) {
                    Ok(()) => {
                        this.sent_messages
                            .lock()
                            .expect("WebSocket message observations should not be poisoned")
                            .push(bytes[..count].to_vec());
                        Poll::Ready(Ok(count))
                    }
                    Err(error) => Poll::Ready(Err(io::Error::other(error))),
                }
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.as_mut().get_mut().socket)
            .poll_flush(context)
            .map_err(io::Error::other)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.as_mut().poll_flush(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => Pin::new(&mut self.as_mut().get_mut().socket)
                .poll_close(context)
                .map_err(io::Error::other),
        }
    }
}

trait ClientTransport: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ClientTransport for T {}

async fn pump_coalesced_websocket(
    socket: ClientWebSocket,
    pipe: DuplexStream,
    sent_messages: Arc<Mutex<Vec<Vec<u8>>>>,
    coalesce_body: Arc<AtomicBool>,
    response_verified: Arc<AtomicBool>,
    client_h2_completed: Arc<AtomicBool>,
) {
    let (mut socket_sink, mut socket_stream) = socket.split();
    let (mut pipe_reader, mut pipe_writer) = tokio::io::split(pipe);

    let outbound = async {
        let mut pending = BytesMut::new();
        let mut chunk = [0_u8; FRAME_BYTES];
        loop {
            let count = pipe_reader
                .read(&mut chunk)
                .await
                .expect("the bounded client byte pipe should remain readable");
            if count == 0 {
                assert!(
                    pending.is_empty(),
                    "the coalesced request body must be complete"
                );
                socket_sink
                    .close()
                    .await
                    .expect("the owned client WebSocket should close");
                break;
            }

            assert!(
                pending.len().saturating_add(count) <= MAX_COALESCED_WRITE_BYTES,
                "the coalescing WebSocket pump must stay within its byte bound"
            );
            pending.extend_from_slice(&chunk[..count]);
            let collecting = coalesce_body.load(Ordering::Acquire);
            if collecting
                && !pending
                    .windows(LAST_BODY_MARKER.len())
                    .any(|window| window == LAST_BODY_MARKER)
            {
                continue;
            }
            if collecting {
                assert!(
                    pending
                        .windows(FIRST_BODY_MARKER.len())
                        .any(|window| window == FIRST_BODY_MARKER),
                    "the grouped body must include its first marker"
                );
            }

            let message = std::mem::take(&mut pending).freeze();
            socket_sink
                .send(Message::Binary(message.clone()))
                .await
                .expect("the owned client WebSocket should send h2 bytes");
            sent_messages
                .lock()
                .expect("WebSocket message observations should not be poisoned")
                .push(message.to_vec());
            if collecting {
                coalesce_body.store(false, Ordering::Release);
            }
        }
    };

    let inbound = async {
        while let Some(message) = socket_stream.next().await {
            match message.expect("the owned client WebSocket should receive h2 bytes") {
                Message::Binary(bytes) => {
                    if let Err(error) = pipe_writer.write_all(&bytes).await {
                        assert!(
                            error.kind() == io::ErrorKind::BrokenPipe
                                && response_verified.load(Ordering::Acquire)
                                && client_h2_completed.load(Ordering::Acquire),
                            "unexpected client byte pipe failure: {error}"
                        );
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => panic!("expected a binary RPC tunnel message"),
            }
        }
        pipe_writer
            .shutdown()
            .await
            .expect("the bounded client byte pipe should close");
    };

    tokio::join!(outbound, inbound);
}

async fn echo_one_http2_request(
    mut incoming: NativeIncoming,
    response_read: tokio::sync::oneshot::Receiver<()>,
    server_ready: tokio::sync::oneshot::Sender<()>,
) {
    let tunnel = incoming
        .next()
        .await
        .expect("native admission should yield a tunnel")
        .expect("native admission should not yield an error");
    let mut connection = h2::server::handshake(tunnel)
        .await
        .expect("the admitted byte stream should complete the HTTP/2 handshake");
    server_ready
        .send(())
        .expect("the client should retain the server readiness receiver");
    let (request, mut respond) = connection
        .accept()
        .await
        .expect("the client should open one HTTP/2 request")
        .expect("HTTP/2 should accept the request");
    let mut body = request.into_body();
    let mut payload = BytesMut::new();
    let mut response_read = response_read;
    loop {
        tokio::select! {
            chunk = body.data() => match chunk {
                Some(Ok(chunk)) => {
                    let length = chunk.len();
                    payload.extend_from_slice(&chunk);
                    body.flow_control()
                        .release_capacity(length)
                        .expect("the echo server should release consumed capacity");
                }
                Some(Err(error)) => panic!("HTTP/2 request data should remain valid: {error}"),
                None => break,
            },
            next_request = connection.accept() => match next_request {
                Some(Ok(_)) => panic!("the contract fixture accepts exactly one request"),
                Some(Err(error)) => panic!("HTTP/2 should keep the request body alive: {error}"),
                None => panic!("HTTP/2 closed before the request body completed"),
            }
        }
    }

    let mut response = respond
        .send_response(
            HttpResponse::builder()
                .status(StatusCode::OK)
                .body(())
                .expect("the response should be valid"),
            false,
        )
        .expect("the HTTP/2 response should be accepted");
    response
        .send_data(payload.freeze(), true)
        .expect("the echo body should fit the stream window");
    tokio::select! {
        response_result = &mut response_read => {
            response_result.expect("the client should read the complete echo response");
            connection.graceful_shutdown();
            match connection.accept().await {
                Some(Ok(_)) => panic!("the contract fixture accepts exactly one request"),
                Some(Err(error)) => panic!("HTTP/2 should finish its graceful shutdown: {error}"),
                None => (),
            }
        },
        next_request = connection.accept() => match next_request {
            Some(Ok(_)) => panic!("the contract fixture accepts exactly one request"),
            Some(Err(error)) => panic!("HTTP/2 should drive the response: {error}"),
            None => (),
        }
    }
}

async fn run_http2_exchange(write_limit: usize, coalesce_writes: bool) {
    let (admission, incoming) =
        NativeIncoming::bounded(NonZeroUsize::new(1).expect("capacity is nonzero"))
            .expect("one pending connection should be valid");
    let metrics = Arc::new(ConnectionMetrics::new(false));
    let (finished_sender, finished_receiver) = tokio::sync::oneshot::channel();
    let finished_sender = Arc::new(Mutex::new(Some(finished_sender)));

    let route_admission = admission.clone();
    let route_metrics = metrics.clone();
    let route_finished = finished_sender.clone();
    let app = Router::new().route(
        "/",
        get(move |upgrade: WebSocketUpgrade| {
            let admission = route_admission.clone();
            let metrics = route_metrics.clone();
            let finished = route_finished.clone();
            async move {
                let permit = match admission.try_reserve() {
                    Ok(permit) => permit,
                    Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
                };
                upgrade
                    .read_buffer_size(FRAME_BYTES)
                    .write_buffer_size(FRAME_BYTES)
                    .max_write_buffer_size(MESSAGE_BYTES)
                    .max_message_size(MESSAGE_BYTES)
                    .max_frame_size(MESSAGE_BYTES)
                    .on_upgrade(move |socket: WebSocket| async move {
                        let result = permit.accept_websocket_measured(socket, metrics).await;
                        let mut sender = finished
                            .lock()
                            .expect("WebSocket pump completion owner should not be poisoned");
                        if let Some(sender) = sender.take() {
                            sender
                                .send(result)
                                .expect("the test should retain the pump completion receiver");
                        }
                    })
                    .into_response()
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener should bind");
    let address = listener
        .local_addr()
        .expect("the listener should be addressed");
    let listener_task: JoinHandle<()> =
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve loopback") });
    let (response_read_sender, response_read_receiver) = tokio::sync::oneshot::channel();
    let (server_ready_sender, server_ready_receiver) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(echo_one_http2_request(
        incoming,
        response_read_receiver,
        server_ready_sender,
    ));

    let (socket, _) = connect_async(format!("ws://{address}/"))
        .await
        .expect("the WebSocket should connect to the owned loopback listener");
    let sent_messages = Arc::new(Mutex::new(Vec::new()));
    let coalesce_body = Arc::new(AtomicBool::new(false));
    let response_verified = Arc::new(AtomicBool::new(false));
    let client_h2_completed = Arc::new(AtomicBool::new(false));
    let (transport, pump_task): (Box<dyn ClientTransport>, Option<JoinHandle<()>>) =
        if coalesce_writes {
            let (client_pipe, pump_pipe) = tokio::io::duplex(MAX_COALESCED_WRITE_BYTES);
            let pump_task = tokio::spawn(pump_coalesced_websocket(
                socket,
                pump_pipe,
                sent_messages.clone(),
                coalesce_body.clone(),
                response_verified.clone(),
                client_h2_completed.clone(),
            ));
            (Box::new(client_pipe), Some(pump_task))
        } else {
            (
                Box::new(WebSocketIo::new(socket, write_limit, sent_messages.clone())),
                None,
            )
        };
    let (mut client, connection) = h2::client::handshake(transport)
        .await
        .expect("the HTTP/2 client should handshake through the WebSocket tunnel");
    let completion = client_h2_completed.clone();
    let mut client_task = tokio::spawn(async move {
        let mut connection = Box::pin(connection);
        let result = connection.as_mut().await;
        completion.store(result.is_ok(), Ordering::Release);
        drop(connection);
        result
    });
    server_ready_receiver
        .await
        .expect("the native HTTP/2 server should complete its handshake");
    if coalesce_writes {
        coalesce_body.store(true, Ordering::Release);
    }

    let mut payload_bytes = vec![b'x'; FRAME_BYTES * 2 + 64];
    payload_bytes[..FIRST_BODY_MARKER.len()].copy_from_slice(FIRST_BODY_MARKER);
    let last_marker_start = payload_bytes.len() - LAST_BODY_MARKER.len();
    payload_bytes[last_marker_start..].copy_from_slice(LAST_BODY_MARKER);
    let payload = Bytes::from(payload_bytes);
    let request = Request::builder()
        .method("POST")
        .uri("https://loopback.invalid/echo")
        .body(())
        .expect("the HTTP/2 request should be valid");
    let (response, mut request_body) = client
        .send_request(request, false)
        .expect("the client should start an HTTP/2 request");
    request_body
        .send_data(payload.clone(), true)
        .expect("the request body should fit the initial stream window");

    let response = response
        .await
        .expect("the server should return an HTTP/2 response");
    assert_eq!(response.status(), StatusCode::OK);
    let mut response_body = response.into_body();
    let mut echoed = BytesMut::new();
    while let Some(chunk) = response_body.data().await {
        let chunk = chunk.expect("the echoed HTTP/2 body should be valid");
        let length = chunk.len();
        echoed.extend_from_slice(&chunk);
        response_body
            .flow_control()
            .release_capacity(length)
            .expect("the client should release consumed response capacity");
    }
    assert_eq!(echoed.freeze(), payload);
    {
        let observed_messages = sent_messages
            .lock()
            .expect("WebSocket message observations should not be poisoned");
        if coalesce_writes {
            let coalesced_http2_data = observed_messages.iter().any(|message| {
                let first = message
                    .windows(FIRST_BODY_MARKER.len())
                    .position(|window| window == FIRST_BODY_MARKER);
                let last = message
                    .windows(LAST_BODY_MARKER.len())
                    .position(|window| window == LAST_BODY_MARKER);
                matches!((first, last), (Some(first), Some(last)) if last - first > FRAME_BYTES)
            });
            assert!(
                coalesced_http2_data,
                "one binary WebSocket message must contain separated payload bytes spanning multiple HTTP/2 DATA frames"
            );
        } else {
            assert!(observed_messages.len() > 4);
            assert!(
                observed_messages
                    .iter()
                    .all(|message| !message.is_empty() && message.len() <= write_limit)
            );
            let mut wire = Vec::new();
            for message in observed_messages.iter() {
                wire.extend_from_slice(message);
            }
            assert!(wire.starts_with(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"));
        }
    }
    response_verified.store(true, Ordering::Release);
    response_read_sender
        .send(())
        .expect("the HTTP/2 server should still own its response driver");

    drop(client);
    match tokio::time::timeout(Duration::from_secs(3), &mut client_task).await {
        Ok(joined) => {
            joined
                .expect("the HTTP/2 client driver should not panic")
                .expect("the HTTP/2 client connection should close cleanly");
        }
        Err(_) => {
            client_task.abort();
            let reaped = client_task.await;
            assert!(
                matches!(reaped, Err(error) if error.is_cancelled()),
                "a timed-out client driver must be cancelled and reaped"
            );
            panic!("the HTTP/2 client driver exceeded its close bound");
        }
    }
    let mut server_task = server_task;
    match tokio::time::timeout(Duration::from_secs(3), &mut server_task).await {
        Ok(joined) => joined.expect("the HTTP/2 server task should not panic"),
        Err(_) => {
            server_task.abort();
            let reaped = server_task.await;
            assert!(
                matches!(reaped, Err(error) if error.is_cancelled()),
                "a timed-out server task must be cancelled and reaped"
            );
            panic!("the HTTP/2 server task exceeded its completion bound");
        }
    }
    let finished = tokio::time::timeout(Duration::from_secs(3), finished_receiver)
        .await
        .expect("the WebSocket pump should finish with its owner")
        .expect("the WebSocket pump completion should be reported");
    finished.expect("the clean HTTP/2 close should not reject the byte stream");
    if let Some(mut pump_task) = pump_task {
        match tokio::time::timeout(Duration::from_secs(3), &mut pump_task).await {
            Ok(joined) => joined.expect("the owned coalescing WebSocket pump should not panic"),
            Err(_) => {
                pump_task.abort();
                let reaped = pump_task.await;
                assert!(
                    matches!(reaped, Err(error) if error.is_cancelled()),
                    "a timed-out coalescing WebSocket pump must be cancelled and reaped"
                );
                panic!("the coalescing WebSocket pump exceeded its close bound");
            }
        }
    }

    let snapshot = metrics
        .snapshot()
        .expect("the public connection metrics should remain readable");
    assert!(
        snapshot.closed,
        "the adapter should close the connection owner"
    );
    assert_eq!(
        snapshot.pipe_owners, 0,
        "the tunnel pipes should be released"
    );
    listener_task.abort();
    let listener_result = listener_task.await;
    assert!(
        matches!(listener_result, Err(error) if error.is_cancelled()),
        "the owned loopback listener should be cancelled and reaped"
    );
}

#[tokio::test]
async fn websocket_messages_are_only_chunks_of_the_http2_byte_stream() {
    tokio::time::timeout(Duration::from_secs(15), async {
        // Tiny writes force the HTTP/2 preface and frames across many WebSocket messages.
        run_http2_exchange(7, false).await;
        // The owned byte-pipe pump groups consecutive HTTP/2 DATA writes into one message.
        run_http2_exchange(usize::MAX, true).await;
    })
    .await
    .expect("the real native HTTP/2 loopback should finish within its bound");
}

#[test]
fn native_admission_reports_full_and_closed_as_typed_refusals() {
    let (admission, incoming) =
        NativeIncoming::bounded(NonZeroUsize::new(1).expect("capacity is nonzero"))
            .expect("one pending connection should be valid");
    let permit = admission
        .try_reserve()
        .expect("the first pending connection should reserve capacity");
    assert!(matches!(admission.try_reserve(), Err(AdmissionError::Full)));
    drop(permit);

    let permit = admission
        .try_reserve()
        .expect("dropping a reservation should release capacity");
    drop(incoming);
    assert!(matches!(
        admission.try_reserve(),
        Err(AdmissionError::Closed)
    ));
    drop(permit);
}
