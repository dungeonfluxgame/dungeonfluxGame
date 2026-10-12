use crate::{ConnectionMetrics, FRAME_BYTES, MESSAGE_BYTES, RECEIVE_BYTES};
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, Stream, StreamExt};
use std::{
    io,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    sync::mpsc,
};

/// How often an idle tunnel is pinged, and how long it may stay silent before it is dropped.
/// A browser answers pings by itself, so a live page never trips this; a page that vanished
/// without closing its socket (a crashed or suspended tab) releases its connection slot and
/// watch stream within [`KeepAlive::idle_limit`] instead of holding them until the process ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KeepAlive {
    pub(crate) interval: Duration,
    pub(crate) idle_limit: Duration,
}

pub(crate) const KEEP_ALIVE: KeepAlive = KeepAlive {
    interval: Duration::from_secs(15),
    idle_limit: Duration::from_secs(45),
};

/// Failure to reserve a bounded incoming slot before a WebSocket upgrade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    InvalidCapacity,
    Full,
    Closed,
}
impl std::fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCapacity => "incoming capacity exceeds channel limit",
            Self::Full => "incoming capacity exhausted",
            Self::Closed => "incoming server closed",
        })
    }
}
impl std::error::Error for AdmissionError {}

/// Bounded pending connections consumed directly by tonic's incoming server.
/// Live connection limits remain the listener owner's responsibility.
pub struct NativeIncoming {
    receiver: mpsc::Receiver<Result<TunnelStream, io::Error>>,
    accepting: Arc<Mutex<bool>>,
}
impl NativeIncoming {
    /// Create a queue with an explicit nonzero pending connection bound.
    /// Returns InvalidCapacity rather than panicking above Tokio's channel limit.
    pub fn bounded(capacity: NonZeroUsize) -> Result<(NativeAdmission, Self), AdmissionError> {
        if capacity.get() > tokio::sync::Semaphore::MAX_PERMITS {
            return Err(AdmissionError::InvalidCapacity);
        }
        let (sender, receiver) = mpsc::channel(capacity.get());
        let accepting = Arc::new(Mutex::new(true));
        Ok((
            NativeAdmission {
                sender,
                accepting: accepting.clone(),
            },
            Self {
                receiver,
                accepting,
            },
        ))
    }
}
impl Drop for NativeIncoming {
    fn drop(&mut self) {
        // An owned Tokio permit may send after receiver closure. Serialize the
        // final transfer with closure and drain while no reservation can publish.
        let mut accepting = match self.accepting.lock() {
            Ok(accepting) => accepting,
            // Poison recovery is used only to close; reserve/transfer fail closed.
            Err(poisoned) => poisoned.into_inner(),
        };
        *accepting = false;
        self.receiver.close();
        while let Ok(connection) = self.receiver.try_recv() {
            drop(connection);
        }
    }
}
impl Stream for NativeIncoming {
    type Item = Result<TunnelStream, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(context)
    }
}

/// Listener-side admission handle. This reserves capacity without waiting or allocating pipes.
#[derive(Clone)]
pub struct NativeAdmission {
    sender: mpsc::Sender<Result<TunnelStream, io::Error>>,
    accepting: Arc<Mutex<bool>>,
}
impl NativeAdmission {
    /// Reserve before returning an HTTP upgrade response. Dropping a reservation releases its slot.
    pub fn try_reserve(&self) -> Result<NativeConnectionPermit, AdmissionError> {
        let accepting = self.accepting.lock().map_err(|_| AdmissionError::Closed)?;
        if !*accepting {
            return Err(AdmissionError::Closed);
        }
        self.sender
            .clone()
            .try_reserve_owned()
            .map(|permit| NativeConnectionPermit {
                permit,
                accepting: self.accepting.clone(),
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => AdmissionError::Full,
                mpsc::error::TrySendError::Closed(_) => AdmissionError::Closed,
            })
    }
}

/// Single-use owned reservation transferred to the HTTP upgrade's connection task.
pub struct NativeConnectionPermit {
    permit: mpsc::OwnedPermit<Result<TunnelStream, io::Error>>,
    accepting: Arc<Mutex<bool>>,
}
impl NativeConnectionPermit {
    /// Forward the upgraded socket into tonic and own both byte directions until termination.
    /// Receiver closure after reservation returns BrokenPipe; transport errors propagate unchanged.
    pub async fn accept_websocket_measured(
        self,
        socket: WebSocket,
        metrics: Arc<ConnectionMetrics>,
    ) -> io::Result<()> {
        pump_websocket(socket, metrics, KEEP_ALIVE, |connection| {
            let accepting = self
                .accepting
                .lock()
                .map_err(|_| io::Error::other("incoming admission state unavailable"))?;
            if !*accepting {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "incoming server closed",
                ));
            }
            drop(self.permit.send(Ok(connection)));
            Ok(())
        })
        .await
    }
}

/// Connection owned by tonic. Dropping it releases the associated WebSocket pump.
pub struct TunnelStream(PipeIo);
struct PipeIo {
    stream: DuplexStream,
    metrics: Arc<ConnectionMetrics>,
    grpc_end: bool,
}
impl Drop for PipeIo {
    fn drop(&mut self) {
        self.metrics.pipe_drop();
    }
}
impl AsyncRead for PipeIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let metrics = self.metrics.clone();
        let to_grpc = self.grpc_end;
        let result = metrics.pipe(to_grpc, |counter| {
            let before = buffer.filled().len();
            match Pin::new(&mut self.stream).poll_read(context, buffer) {
                Poll::Ready(Ok(())) => {
                    Poll::Ready(counter.transferred(buffer.filled().len() - before, false))
                }
                other => other,
            }
        });
        result.unwrap_or_else(|error| Poll::Ready(Err(error)))
    }
}
impl AsyncWrite for PipeIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let metrics = self.metrics.clone();
        let to_grpc = !self.grpc_end;
        let result = metrics.pipe(to_grpc, |counter| {
            match Pin::new(&mut self.stream).poll_write(context, bytes) {
                Poll::Ready(Ok(count)) => {
                    Poll::Ready(counter.transferred(count, true).map(|()| count))
                }
                other => other,
            }
        });
        result.unwrap_or_else(|error| Poll::Ready(Err(error)))
    }
    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}
struct MessageScope {
    metrics: Arc<ConnectionMetrics>,
    bytes: usize,
}
impl Drop for MessageScope {
    fn drop(&mut self) {
        if self.metrics.socket_receive(self.bytes, false).is_err() {
            self.metrics.reject();
        }
    }
}
struct ConnectionScope(Arc<ConnectionMetrics>);
impl Drop for ConnectionScope {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl tonic::transport::server::Connected for TunnelStream {
    type ConnectInfo = ();
    fn connect_info(&self) {}
}
impl AsyncRead for TunnelStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}
impl AsyncWrite for TunnelStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(context, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(context)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}

/// Run both byte directions in the upgrade's scope. Admission never queues without a bound.
pub async fn accept_websocket(
    socket: WebSocket,
    incoming: mpsc::Sender<Result<TunnelStream, io::Error>>,
) -> io::Result<()> {
    accept_websocket_measured(socket, incoming, Arc::new(ConnectionMetrics::new(false))).await
}
/// Own the real pipe counters and observe both HTTP/2 wire directions.
pub async fn accept_websocket_measured(
    socket: WebSocket,
    incoming: mpsc::Sender<Result<TunnelStream, io::Error>>,
    metrics: Arc<ConnectionMetrics>,
) -> io::Result<()> {
    accept_websocket_kept(socket, incoming, metrics, KEEP_ALIVE).await
}
async fn accept_websocket_kept(
    socket: WebSocket,
    incoming: mpsc::Sender<Result<TunnelStream, io::Error>>,
    metrics: Arc<ConnectionMetrics>,
    keep_alive: KeepAlive,
) -> io::Result<()> {
    pump_websocket(socket, metrics, keep_alive, |connection| {
        incoming
            .try_send(Ok(connection))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    io::Error::other("connection capacity exhausted")
                }
                mpsc::error::TrySendError::Closed(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "incoming server closed")
                }
            })
    })
    .await
}
async fn pump_websocket(
    socket: WebSocket,
    metrics: Arc<ConnectionMetrics>,
    keep_alive: KeepAlive,
    admit: impl FnOnce(TunnelStream) -> io::Result<()>,
) -> io::Result<()> {
    let _scope = ConnectionScope(metrics.clone());
    let (grpc, tunnel) = tokio::io::duplex(RECEIVE_BYTES);
    let grpc = PipeIo {
        stream: grpc,
        metrics: metrics.clone(),
        grpc_end: true,
    };
    let tunnel = PipeIo {
        stream: tunnel,
        metrics: metrics.clone(),
        grpc_end: false,
    };
    if let Err(error) = admit(TunnelStream(grpc)) {
        metrics.reject();
        return Err(error);
    }
    let (mut outgoing, mut incoming_socket) = socket.split();
    let (mut reader, mut writer) = tokio::io::split(tunnel);
    // Milliseconds after `opened` at which the last frame (of any kind) was heard.
    let opened = Instant::now();
    let last_heard = AtomicU64::new(0);
    let receive = async {
        while let Some(message) = incoming_socket.next().await {
            last_heard.store(
                u64::try_from(opened.elapsed().as_millis()).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
            match message.map_err(io::Error::other)? {
                Message::Binary(bytes) if bytes.len() <= MESSAGE_BYTES => {
                    metrics.socket_receive(bytes.len(), true)?;
                    let _message_owner = MessageScope {
                        metrics: metrics.clone(),
                        bytes: bytes.len(),
                    };
                    metrics.observe(true, &bytes)?;
                    for chunk in bytes.chunks(FRAME_BYTES) {
                        writer.write_all(chunk).await?;
                        metrics.yield_decode()?;
                        tokio::task::yield_now().await;
                    }
                }
                Message::Close(_) => return metrics.receive_eof(),
                Message::Ping(_) | Message::Pong(_) => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "binary tunnel message rejected",
                    ));
                }
            }
            metrics.yield_decode()?;
            tokio::task::yield_now().await;
        }
        metrics.receive_eof()
    };
    let send = async {
        let mut buffer = vec![0; FRAME_BYTES];
        let mut ticker = tokio::time::interval(keep_alive.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick completes at once; pings start one interval after the upgrade.
        ticker.tick().await;
        loop {
            tokio::select! {
                read = reader.read(&mut buffer) => {
                    let count = read?;
                    if count == 0 {
                        return Ok(());
                    }
                    metrics.observe(false, &buffer[..count])?;
                    metrics.backlog(count)?;
                    outgoing
                        .send(Message::Binary(buffer[..count].to_vec().into()))
                        .await
                        .map_err(io::Error::other)?;
                    metrics.backlog(0)?;
                }
                _ = ticker.tick() => {
                    let silent = opened.elapsed().saturating_sub(Duration::from_millis(
                        last_heard.load(Ordering::Relaxed),
                    ));
                    if silent > keep_alive.idle_limit {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "websocket tunnel idle: no frame or pong heard",
                        ));
                    }
                    outgoing
                        .send(Message::Ping(Vec::new().into()))
                        .await
                        .map_err(io::Error::other)?;
                }
            }
        }
    };
    let result = tokio::select! { result = receive => result, result = send => result };
    if result.is_err() {
        metrics.reject();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::WebSocketUpgrade, routing::get};
    use bytes::Bytes;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn oversized_binary_websocket_is_rejected_at_gateway() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (incoming, mut receiver) = mpsc::channel(1);
        let (result_sender, result_receiver) = oneshot::channel();
        let result_sender = std::sync::Arc::new(std::sync::Mutex::new(Some(result_sender)));
        let app = Router::new().route(
            "/",
            get(move |upgrade: WebSocketUpgrade| {
                let incoming = incoming.clone();
                let result_sender = result_sender.clone();
                async move {
                    upgrade
                        .read_buffer_size(FRAME_BYTES)
                        .write_buffer_size(FRAME_BYTES)
                        .max_write_buffer_size(MESSAGE_BYTES)
                        .max_message_size(MESSAGE_BYTES)
                        .max_frame_size(MESSAGE_BYTES)
                        .on_upgrade(move |socket| async move {
                            let result = accept_websocket(socket, incoming).await;
                            if let Some(sender) = result_sender.lock().unwrap().take() {
                                sender.send(result).unwrap();
                            }
                        })
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
            .await
            .unwrap();
        let connection = receiver.recv().await.unwrap().unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Binary(
                vec![0; MESSAGE_BYTES + 1].into(),
            ))
            .await
            .unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), result_receiver)
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.is_err(),
            "gateway accepted an oversized tunnel message"
        );
        drop(connection);
        drop(socket);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
    /// A tunnel whose peer vanished without closing (a crashed or suspended tab) is dropped
    /// after the idle limit, so its connection slot is released, while a peer that answers
    /// pings is never dropped however quiet the tunnel is.
    #[tokio::test]
    async fn a_silent_peer_is_dropped_and_an_answering_one_is_kept() {
        let keep_alive = KeepAlive {
            interval: Duration::from_millis(40),
            idle_limit: Duration::from_millis(250),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (incoming, mut receiver) = mpsc::channel(4);
        let (finished_sender, mut finished) = mpsc::unbounded_channel();
        let app = Router::new().route(
            "/",
            get(move |upgrade: WebSocketUpgrade| {
                let incoming = incoming.clone();
                let finished_sender = finished_sender.clone();
                async move {
                    upgrade.on_upgrade(move |socket| async move {
                        let metrics = Arc::new(ConnectionMetrics::new(false));
                        let result =
                            accept_websocket_kept(socket, incoming, metrics, keep_alive).await;
                        let _ = finished_sender.send(result);
                    })
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        // The silent peer connects and is never polled again, so it never answers a ping.
        let (silent, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
            .await
            .unwrap();
        let _silent_tunnel = receiver.recv().await.unwrap().unwrap();
        let verdict = tokio::time::timeout(Duration::from_secs(3), finished.recv())
            .await
            .expect("the silent tunnel was never dropped")
            .unwrap();
        assert_eq!(verdict.unwrap_err().kind(), io::ErrorKind::TimedOut);
        drop(silent);
        // A peer that keeps reading answers pings automatically and stays well past the limit.
        let (mut answering, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
            .await
            .unwrap();
        let _answering_tunnel = receiver.recv().await.unwrap().unwrap();
        let reader = tokio::spawn(async move { while answering.next().await.is_some() {} });
        assert!(
            tokio::time::timeout(Duration::from_millis(900), finished.recv())
                .await
                .is_err(),
            "a peer that answers pings was dropped"
        );
        reader.abort();
        server.abort();
    }

    struct OwnedListener(tokio::task::JoinHandle<io::Result<()>>);
    impl Drop for OwnedListener {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    impl OwnedListener {
        async fn stop(&mut self) {
            self.0.abort();
            assert!((&mut self.0).await.unwrap_err().is_cancelled());
        }
    }
    type ClientSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;
    async fn native_boundary(
        incoming: mpsc::Sender<Result<TunnelStream, io::Error>>,
    ) -> (
        ClientSocket,
        oneshot::Receiver<io::Result<()>>,
        Arc<ConnectionMetrics>,
        OwnedListener,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let metrics = Arc::new(ConnectionMetrics::new(false));
        let observed = metrics.clone();
        let (sender, receiver) = oneshot::channel();
        let sender = Arc::new(std::sync::Mutex::new(Some(sender)));
        let app = Router::new().route(
            "/",
            get(move |upgrade: WebSocketUpgrade| {
                let incoming = incoming.clone();
                let metrics = observed.clone();
                let sender = sender.clone();
                async move {
                    upgrade
                        .read_buffer_size(FRAME_BYTES)
                        .write_buffer_size(FRAME_BYTES)
                        .max_write_buffer_size(MESSAGE_BYTES)
                        .max_message_size(MESSAGE_BYTES)
                        .max_frame_size(MESSAGE_BYTES)
                        .on_upgrade(move |socket| async move {
                            let result = accept_websocket_measured(socket, incoming, metrics).await;
                            if let Some(sender) = sender.lock().unwrap().take() {
                                let _ = sender.send(result);
                            }
                        })
                }
            }),
        );
        let owner = OwnedListener(tokio::spawn(
            async move { axum::serve(listener, app).await },
        ));
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
            .await
            .unwrap();
        (socket, receiver, metrics, owner)
    }
    fn ordered_wire(client: bool) -> Vec<u8> {
        let mut wire = if client {
            b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec()
        } else {
            Vec::new()
        };
        wire.extend([0, 0, 0, 4, 0, 0, 0, 0, 0]);
        for length in [FRAME_BYTES, FRAME_BYTES, 97] {
            wire.extend([0, (length >> 8) as u8, length as u8, 0, 0, 0, 0, 0, 1]);
            wire.extend((0..length).map(|index| ((index * 31 + length) % 251) as u8));
        }
        wire
    }
    fn assert_released(metrics: &ConnectionMetrics) {
        let snapshot = metrics.snapshot().unwrap();
        assert!(snapshot.closed);
        assert_eq!(snapshot.pipe_owners, 0);
        assert_eq!(snapshot.pipe_to_grpc.current, 0);
        assert_eq!(snapshot.pipe_to_websocket.current, 0);
        assert_eq!(snapshot.websocket_receive.current, 0);
        assert_eq!(snapshot.websocket_receive_items.current, 0);
        assert!(snapshot.envelope_within_limit());
    }
    #[tokio::test]
    async fn native_partial_reads_preserve_bytes_across_websocket_message_boundaries() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (incoming, mut receiver) = mpsc::channel(1);
            let (mut socket, finished, metrics, mut owner) = native_boundary(incoming).await;
            let mut connection = receiver.recv().await.unwrap().unwrap();
            let wire = ordered_wire(true);
            let send = async {
                // Empty messages do not manufacture EOF or an extra byte.
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Binary(
                        Vec::new().into(),
                    ))
                    .await
                    .unwrap();
                let mut cursor = 0;
                for length in [1, 2, 5, 19, FRAME_BYTES + 3, 7, FRAME_BYTES + 1] {
                    let end = (cursor + length).min(wire.len());
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                            wire[cursor..end].to_vec().into(),
                        ))
                        .await
                        .unwrap();
                    cursor = end;
                }
                if cursor != wire.len() {
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                            wire[cursor..].to_vec().into(),
                        ))
                        .await
                        .unwrap();
                }
            };
            let receive = async {
                let mut actual = Vec::with_capacity(wire.len());
                let mut buffer = [0u8; 113];
                while actual.len() != wire.len() {
                    let length = (wire.len() - actual.len()).min(buffer.len());
                    let count = connection.read(&mut buffer[..length]).await.unwrap();
                    assert!(count != 0, "unexpected EOF before all tunnel bytes");
                    actual.extend_from_slice(&buffer[..count]);
                }
                assert_eq!(actual, wire);
            };
            tokio::join!(send, receive);
            assert_eq!(metrics.snapshot().unwrap().received_bytes, wire.len());
            socket.close(None).await.unwrap();
            finished.await.unwrap().unwrap();
            let mut byte = [0u8; 1];
            assert_eq!(connection.read(&mut byte).await.unwrap(), 0);
            drop(connection);
            assert_released(&metrics);
            owner.stop().await;
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn native_idle_websocket_close_is_clean_and_releases_owners() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (incoming, mut receiver) = mpsc::channel(1);
            let (mut socket, finished, metrics, mut owner) = native_boundary(incoming).await;
            let mut connection = receiver.recv().await.unwrap().unwrap();
            socket.close(None).await.unwrap();
            finished.await.unwrap().unwrap();
            let mut bytes = Vec::new();
            connection.read_to_end(&mut bytes).await.unwrap();
            assert!(bytes.is_empty());
            assert!(!metrics.snapshot().unwrap().rejected);
            drop(connection);
            drop(socket);
            assert_released(&metrics);
            owner.stop().await;
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn native_websocket_close_rejects_truncated_http2_bytes_and_releases_owners() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let wire = ordered_wire(true);
            // Partial preface, frame header, and DATA payload, including a header
            // received in separate WebSocket messages from its truncated payload.
            for length in [1, 23, 28, 24 + 9 + 9 + 7] {
                let (incoming, mut receiver) = mpsc::channel(1);
                let (mut socket, finished, metrics, mut owner) = native_boundary(incoming).await;
                let mut connection = receiver.recv().await.unwrap().unwrap();
                for chunk in wire[..length].chunks(7) {
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                            chunk.to_vec().into(),
                        ))
                        .await
                        .unwrap();
                }
                socket.close(None).await.unwrap();
                let mut actual = Vec::new();
                connection.read_to_end(&mut actual).await.unwrap();
                assert_eq!(actual, wire[..length]);
                let error = finished.await.unwrap().unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
                assert!(metrics.snapshot().unwrap().rejected);
                drop(connection);
                drop(socket);
                assert_released(&metrics);
                owner.stop().await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn native_partial_writes_preserve_bytes_in_bounded_websocket_chunks() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (incoming, mut receiver) = mpsc::channel(1);
            let (mut socket, finished, metrics, mut owner) = native_boundary(incoming).await;
            let mut connection = receiver.recv().await.unwrap().unwrap();
            let wire = ordered_wire(false);
            let send = async {
                for chunk in wire.chunks(FRAME_BYTES + 11) {
                    connection.write_all(chunk).await.unwrap();
                }
                connection.flush().await.unwrap();
            };
            let receive = async {
                let mut actual = Vec::with_capacity(wire.len());
                while actual.len() != wire.len() {
                    let message = socket.next().await.unwrap().unwrap();
                    let tokio_tungstenite::tungstenite::Message::Binary(bytes) = message else {
                        panic!("adapter emitted nonbinary tunnel payload");
                    };
                    assert!(!bytes.is_empty());
                    assert!(bytes.len() <= FRAME_BYTES);
                    assert!(actual.len() + bytes.len() <= wire.len());
                    actual.extend_from_slice(&bytes);
                }
                assert_eq!(actual, wire);
            };
            tokio::join!(send, receive);
            assert_eq!(metrics.snapshot().unwrap().sent_bytes, wire.len());
            connection.shutdown().await.unwrap();
            finished.await.unwrap().unwrap();
            drop(connection);
            drop(socket);
            assert_released(&metrics);
            owner.stop().await;
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn full_native_admission_fails_without_replacing_queued_connection() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (incoming, mut receiver) = mpsc::channel(1);
            assert!(
                incoming
                    .try_send(Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "retained admission marker"
                    )))
                    .is_ok()
            );
            let (socket, finished, metrics, mut owner) = native_boundary(incoming).await;
            assert_eq!(
                finished.await.unwrap().unwrap_err().kind(),
                io::ErrorKind::Other
            );
            assert_eq!(
                receiver.recv().await.unwrap().err().unwrap().kind(),
                io::ErrorKind::PermissionDenied
            );
            assert!(matches!(
                receiver.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
            drop(socket);
            assert_released(&metrics);
            owner.stop().await;
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn pipe_partial_write_reports_only_transferred_bytes_and_preserves_tail() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let metrics = Arc::new(ConnectionMetrics::new(false));
            let (grpc, tunnel) = tokio::io::duplex(7);
            let mut grpc = PipeIo {
                stream: grpc,
                metrics: metrics.clone(),
                grpc_end: true,
            };
            let mut tunnel = PipeIo {
                stream: tunnel,
                metrics: metrics.clone(),
                grpc_end: false,
            };
            let wire: Vec<u8> = (0..31).collect();
            let first = grpc.write(&wire).await.unwrap();
            assert_eq!(first, 7);
            assert_eq!(metrics.snapshot().unwrap().pipe_to_websocket.current, first);
            let mut actual = Vec::with_capacity(wire.len());
            let mut prefix = [0u8; 7];
            tunnel.read_exact(&mut prefix).await.unwrap();
            actual.extend(prefix);
            let send = async {
                grpc.write_all(&wire[first..]).await.unwrap();
                grpc.shutdown().await.unwrap();
            };
            let receive = async {
                let mut buffer = [0u8; 3];
                loop {
                    let count = tunnel.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    actual.extend_from_slice(&buffer[..count]);
                }
            };
            tokio::join!(send, receive);
            assert_eq!(actual, wire);
            let snapshot = metrics.snapshot().unwrap();
            assert_eq!(snapshot.pipe_to_websocket.current, 0);
            assert_eq!(snapshot.pipe_to_websocket.total, wire.len());
            assert!(snapshot.pipe_to_websocket.peak <= 7);
        })
        .await
        .unwrap();
    }
    enum Rejection {
        Adapter,
        Connection,
        Stream,
    }
    async fn reject_wire(mut wire: Vec<u8>, rejection: Rejection) -> crate::ConnectionSnapshot {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (incoming, mut receiver) = mpsc::channel(1);
        let metrics = Arc::new(ConnectionMetrics::new(false));
        let (result_sender, result_receiver) = oneshot::channel();
        let result_sender = Arc::new(std::sync::Mutex::new(Some(result_sender)));
        let observed = metrics.clone();
        let app = Router::new().route(
            "/",
            get(move |upgrade: WebSocketUpgrade| {
                let incoming = incoming.clone();
                let metrics = observed.clone();
                let sender = result_sender.clone();
                async move {
                    upgrade
                        .read_buffer_size(FRAME_BYTES)
                        .write_buffer_size(FRAME_BYTES)
                        .max_write_buffer_size(MESSAGE_BYTES)
                        .max_message_size(MESSAGE_BYTES)
                        .max_frame_size(MESSAGE_BYTES)
                        .on_upgrade(move |socket| async move {
                            let result = accept_websocket_measured(socket, incoming, metrics).await;
                            if let Some(sender) = sender.lock().unwrap().take() {
                                let _ = sender.send(result);
                            }
                        })
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
            .await
            .unwrap();
        let connection = receiver.recv().await.unwrap().unwrap();
        let codec = tokio::spawn(async move {
            let mut connection = h2::server::Builder::new()
                .max_header_list_size(FRAME_BYTES as u32)
                .max_frame_size(FRAME_BYTES as u32)
                .handshake::<_, Bytes>(connection)
                .await?;
            while let Some(request) = connection.accept().await {
                request?;
            }
            Ok::<_, h2::Error>(())
        });
        let mut preface = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec();
        preface.extend([0, 0, 0, 4, 0, 0, 0, 0, 0]);
        preface.append(&mut wire);
        socket
            .send(tokio_tungstenite::tungstenite::Message::Binary(
                preface.into(),
            ))
            .await
            .unwrap();
        if matches!(rejection, Rejection::Stream) {
            let reset = async {
                let mut bytes = Vec::new();
                let mut header_431 = false;
                while let Some(message) = socket.next().await {
                    if let tokio_tungstenite::tungstenite::Message::Binary(chunk) = message.unwrap()
                    {
                        bytes.extend(chunk);
                        assert!(bytes.len() <= 64 * 1024, "unbounded rejection response");
                        while bytes.len() >= 9 {
                            let length = ((bytes[0] as usize) << 16)
                                | ((bytes[1] as usize) << 8)
                                | bytes[2] as usize;
                            if bytes.len() < 9 + length {
                                break;
                            }
                            if bytes[3] == 1 {
                                header_431 = true;
                            }
                            if bytes[3] == 3 && bytes[5..9] == [0, 0, 0, 1] {
                                assert_eq!(
                                    &bytes[9..13],
                                    &[0, 0, 0, 1],
                                    "h2 oversized headers use PROTOCOL_ERROR reset"
                                );
                                assert!(
                                    header_431,
                                    "native h2 did not emit header-list-specific response before reset"
                                );
                                return;
                            }
                            bytes.drain(..9 + length);
                        }
                    }
                }
                panic!("no native header-list stream reset");
            };
            tokio::time::timeout(std::time::Duration::from_secs(2), reset)
                .await
                .unwrap();
            socket.close(None).await.unwrap();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), result_receiver)
                .await
                .unwrap()
                .unwrap();
            codec.abort();
            let _ = codec.await;
        } else if matches!(rejection, Rejection::Connection) {
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(2), codec)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err(),
                "h2 did not reject continuation/header pressure"
            );
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), result_receiver)
                .await
                .unwrap()
                .unwrap();
        } else {
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(2), result_receiver)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
            assert!(metrics.snapshot().unwrap().rejected);
            codec.abort();
            let _ = codec.await;
        }
        drop(socket);
        server.abort();
        let _ = server.await;
        let snapshot = metrics.snapshot().unwrap();
        assert!(snapshot.envelope_within_limit());
        assert_eq!(snapshot.pipe_owners, 0);
        snapshot
    }
    #[tokio::test]
    async fn oversized_http2_frame_rejected_before_pipe_admission() {
        let mut wire = vec![0, 0x40, 1, 1, 4, 0, 0, 0, 1];
        wire.resize(wire.len() + FRAME_BYTES + 1, 0);
        reject_wire(wire, Rejection::Adapter).await;
    }
    #[tokio::test]
    async fn tiny_http2_control_flood_rejected_at_native_boundary() {
        let mut wire = vec![];
        for _ in 0..2049 {
            wire.extend([0, 0, 0, 4, 0, 0, 0, 0, 0]);
        }
        reject_wire(wire, Rejection::Adapter).await;
    }
    #[tokio::test]
    async fn native_incoming_control_rate_rejects_exact_total_101_and_releases_owners() {
        // reject_wire prepends the client's initial SETTINGS, so these 100
        // additional frames make precisely 101 controls on the physical tunnel.
        let mut wire = vec![];
        for _ in 0..100 {
            wire.extend([0, 0, 0, 4, 1, 0, 0, 0, 0]);
        }
        let snapshot = reject_wire(wire, Rejection::Adapter).await;
        assert_eq!(snapshot.received_control_frames, 100);
        assert_eq!(snapshot.received_frames, 101);
        assert_eq!(snapshot.incoming_control_window.len(), 100);
        assert_eq!(snapshot.rejected_control.unwrap().kind, 4);
        assert!(snapshot.closed);
    }
    #[tokio::test]
    async fn continuation_chain_rejected_by_established_h2_limit() {
        let mut wire = vec![0, 0, 1, 1, 0, 0, 0, 0, 1, 0x82];
        for _ in 0..128 {
            wire.extend([0, 0, 1, 9, 0, 0, 0, 0, 1, 0x82]);
        }
        reject_wire(wire, Rejection::Connection).await;
    }
    #[tokio::test]
    async fn oversized_hpack_header_list_rejected_at_native_boundary() {
        // Small encoded block, oversized decoded list (300 literal x headers).
        let mut block = vec![0x82, 0x86, 0x84, 0x01, 9];
        block.extend(b"localhost");
        for _ in 0..300 {
            block.extend([0, 1, b'x', 32]);
            block.extend([b'a'; 32]);
        }
        let length = block.len();
        let mut wire = vec![0, (length >> 8) as u8, length as u8, 1, 4, 0, 0, 0, 1];
        wire.extend(block);
        reject_wire(wire, Rejection::Stream).await;
    }
    #[test]
    fn native_reservations_bound_pending_admission_and_release_when_dropped() {
        let (admission, incoming) = NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
        let reservation = admission.try_reserve().unwrap();
        assert!(matches!(admission.try_reserve(), Err(AdmissionError::Full)));
        drop(reservation);
        let reservation = admission.try_reserve().unwrap();
        drop(reservation);
        drop(incoming);
        assert!(matches!(
            admission.try_reserve(),
            Err(AdmissionError::Closed)
        ));
        assert!(matches!(
            NativeIncoming::bounded(
                NonZeroUsize::new(tokio::sync::Semaphore::MAX_PERMITS + 1).unwrap()
            ),
            Err(AdmissionError::InvalidCapacity)
        ));
    }
    #[tokio::test]
    async fn native_incoming_forwards_errors_and_ends_after_admission_owner_drop() {
        let (admission, mut incoming) =
            NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
        assert!(
            admission
                .sender
                .try_send(Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "synthetic listener failure",
                )))
                .is_ok()
        );
        assert_eq!(
            incoming.next().await.unwrap().err().unwrap().kind(),
            io::ErrorKind::ConnectionAborted
        );
        drop(admission);
        assert!(incoming.next().await.is_none());
    }
    #[tokio::test]
    async fn closed_native_server_rejects_upgraded_socket_and_releases_pipe_owners() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (incoming, receiver) = mpsc::channel(1);
            drop(receiver);
            let (socket, finished, metrics, mut owner) = native_boundary(incoming).await;
            assert_eq!(
                finished.await.unwrap().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            drop(socket);
            assert_released(&metrics);
            owner.stop().await;
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn receiver_closed_after_reservation_rejects_owned_upgrade_and_releases_pipes() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (admission, incoming) =
                NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
            let reservation = admission.try_reserve().unwrap();
            drop(incoming);
            let reservation = Arc::new(std::sync::Mutex::new(Some(reservation)));
            let metrics = Arc::new(ConnectionMetrics::new(false));
            let observed = metrics.clone();
            let (finished_tx, finished_rx) = oneshot::channel();
            let finished_tx = Arc::new(std::sync::Mutex::new(Some(finished_tx)));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let app = Router::new().route(
                "/",
                get(move |upgrade: WebSocketUpgrade| {
                    let reservation = reservation.lock().unwrap().take().unwrap();
                    let finished_tx = finished_tx.lock().unwrap().take().unwrap();
                    let metrics = observed.clone();
                    async move {
                        upgrade.on_upgrade(move |socket| async move {
                            let result =
                                reservation.accept_websocket_measured(socket, metrics).await;
                            let _ = finished_tx.send(result);
                        })
                    }
                }),
            );
            let mut owner = OwnedListener(tokio::spawn(
                async move { axum::serve(listener, app).await },
            ));
            let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
                .await
                .unwrap();
            assert_eq!(
                finished_rx.await.unwrap().unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
            assert!(metrics.snapshot().unwrap().rejected);
            drop(socket);
            assert_released(&metrics);
            owner.stop().await;
        })
        .await
        .unwrap();
    }
}
