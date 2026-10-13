use crate::{
    CONCURRENT_STREAMS, ConnectionMetrics, ConnectionSnapshot, FRAME_BYTES, MESSAGE_BYTES,
    RECEIVE_BYTES,
    resources::ReceiveCredit,
    upload::{ConnectionCancellation, ConnectionClosed, UploadCompletion, UploadResult},
};
use bytes::Bytes;
use futures::{
    FutureExt,
    channel::oneshot,
    future::{AbortHandle, Abortable, poll_fn},
};
use gloo_timers::callback::Timeout;
use http_body::{Body, Frame};
use http_body_util::BodyExt;
use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    io,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tower_service::Service;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Event, MessageEvent, WebSocket};

struct ReceiptOwner {
    metrics: Arc<ConnectionMetrics>,
    bytes: usize,
}
impl Drop for ReceiptOwner {
    fn drop(&mut self) {
        if self.metrics.socket_receive(self.bytes, false).is_err() {
            self.metrics.reject();
        }
    }
}
struct ReceiveState {
    messages: VecDeque<Bytes>,
    bytes: usize,
    open: bool,
    closed: bool,
    failed: bool,
    waker: Option<Waker>,
    read_budget: usize,
}
impl ReceiveState {
    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
    fn clear_queued(&mut self, metrics: &ConnectionMetrics) {
        for bytes in self.messages.drain(..) {
            if metrics.queue(bytes.len(), false, true).is_err() {
                metrics.reject();
            }
            drop(bytes);
        }
        self.bytes = 0;
    }
}
struct WebSocketIo {
    socket: WebSocket,
    state: Rc<RefCell<ReceiveState>>,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _open: Closure<dyn FnMut(Event)>,
    _close: Closure<dyn FnMut(Event)>,
    _error: Closure<dyn FnMut(Event)>,
    write_wakeup: Option<Timeout>,
    read_wakeup: Option<Timeout>,
    metrics: Arc<ConnectionMetrics>,
}
impl WebSocketIo {
    async fn connect(url: &str, metrics: Arc<ConnectionMetrics>) -> io::Result<Self> {
        let socket =
            WebSocket::new(url).map_err(|_| io::Error::other("WebSocket creation failed"))?;
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let state = Rc::new(RefCell::new(ReceiveState {
            messages: VecDeque::new(),
            bytes: 0,
            open: false,
            closed: false,
            failed: false,
            waker: None,
            read_budget: 64 * 1024,
        }));
        let message_metrics = metrics.clone();
        let message_state = state.clone();
        let message_socket = socket.clone();
        let message = Closure::wrap(Box::new(move |event: MessageEvent| {
            let mut state = message_state.borrow_mut();
            let Ok(array) = event.data().dyn_into::<js_sys::ArrayBuffer>() else {
                message_metrics.reject();
                state.failed = true;
                state.wake();
                let _ = message_socket.close();
                return;
            };
            let size = array.byte_length() as usize;
            if message_metrics.socket_receive(size, true).is_err() {
                state.failed = true;
                state.wake();
                let _ = message_socket.close();
                return;
            }
            // The engine already allocated this ArrayBuffer; this owner covers its callback reference.
            let _receipt = ReceiptOwner {
                metrics: message_metrics.clone(),
                bytes: size,
            };
            if size == 0 {
                return;
            }
            if size > MESSAGE_BYTES
                || state.bytes.saturating_add(size) > RECEIVE_BYTES
                || state.messages.len() >= 256
            {
                message_metrics.reject();
                state.failed = true;
                state.clear_queued(&message_metrics);
                state.wake();
                let _ = message_socket.close();
                return;
            }
            let bytes = js_sys::Uint8Array::new(&array).to_vec();
            let copied = match message_metrics.browser_callback_from_vec(bytes) {
                Ok(copied) => copied,
                Err(_) => {
                    message_metrics.reject();
                    state.failed = true;
                    state.clear_queued(&message_metrics);
                    state.wake();
                    let _ = message_socket.close();
                    return;
                }
            };
            if message_metrics.queue(copied.len(), true, false).is_err() {
                message_metrics.reject();
                state.failed = true;
                state.clear_queued(&message_metrics);
                state.wake();
                let _ = message_socket.close();
                return;
            }
            state.bytes += copied.len();
            state.messages.push_back(copied);
            state.wake();
        }) as Box<dyn FnMut(MessageEvent)>);
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        let open_state = state.clone();
        let open = Closure::wrap(Box::new(move |_: Event| {
            let mut state = open_state.borrow_mut();
            state.open = true;
            state.wake();
        }) as Box<dyn FnMut(Event)>);
        socket.set_onopen(Some(open.as_ref().unchecked_ref()));
        let close_state = state.clone();
        let close = Closure::wrap(Box::new(move |_: Event| {
            let mut state = close_state.borrow_mut();
            state.closed = true;
            state.wake();
        }) as Box<dyn FnMut(Event)>);
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        let error_state = state.clone();
        let error = Closure::wrap(Box::new(move |_: Event| {
            let mut state = error_state.borrow_mut();
            state.failed = true;
            state.wake();
        }) as Box<dyn FnMut(Event)>);
        socket.set_onerror(Some(error.as_ref().unchecked_ref()));
        let io = Self {
            socket,
            state,
            _message: message,
            _open: open,
            _close: close,
            _error: error,
            write_wakeup: None,
            read_wakeup: None,
            metrics,
        };
        const CONNECTION_TIMEOUT_MS: u32 = 10_000;
        let wait_for_open = poll_fn(|context| {
            let mut state = io.state.borrow_mut();
            if state.failed || state.closed {
                return Poll::Ready(Err(io::Error::other("WebSocket admission failed")));
            }
            if state.open {
                return Poll::Ready(Ok(()));
            }
            state.waker = Some(context.waker().clone());
            Poll::Pending
        })
        .boxed_local();
        let deadline = gloo_timers::future::TimeoutFuture::new(CONNECTION_TIMEOUT_MS).boxed_local();
        match futures::future::select(wait_for_open, deadline).await {
            futures::future::Either::Left((result, _)) => result?,
            futures::future::Either::Right(((), _)) => {
                let _ = io.socket.close();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "WebSocket connection timed out",
                ));
            }
        }
        Ok(io)
    }
    fn wait_for_write(&mut self, context: &Context<'_>) {
        let waker = context.waker().clone();
        self.write_wakeup = Some(Timeout::new(5, move || waker.wake()));
    }
}
impl Drop for WebSocketIo {
    fn drop(&mut self) {
        self.socket.set_onmessage(None);
        self.socket.set_onopen(None);
        self.socket.set_onclose(None);
        self.socket.set_onerror(None);
        let _ = self.socket.close();
        self.state.borrow_mut().clear_queued(&self.metrics);
        self.metrics.close();
    }
}

/// Fixture-only observation through the real callback adapter, without an h2 reader.
/// Owns the private socket for at most three seconds; never exposes unpolled I/O.
#[cfg(all(debug_assertions, feature = "browser-callback-overflow-fixture"))]
pub async fn observe_callback_overflow_fixture(
    url: &str,
) -> (
    io::Result<()>,
    Option<ConnectionSnapshot>,
    io::Result<ConnectionSnapshot>,
) {
    use futures::future::{Either, select};
    use gloo_timers::future::TimeoutFuture;
    use tokio::io::AsyncWriteExt;

    let metrics = Arc::new(ConnectionMetrics::new(true));
    let observed = Rc::new(RefCell::new(None));
    let work = async {
        let mut socket = WebSocketIo::connect(url, metrics.clone()).await?;
        // Release the existing finite producer with a real HTTP/2 client preface.
        // Incoming reads remain unpolled: only actual callback queue bounds can reject.
        socket
            .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
            .await?;
        loop {
            let snapshot = metrics.snapshot()?;
            let rejected = snapshot.rejected;
            *observed.borrow_mut() = Some(snapshot);
            if rejected {
                return Ok(());
            }
            if socket.state.borrow().failed || socket.state.borrow().closed {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "callback fixture closed before queue rejection",
                ));
            }
            TimeoutFuture::new(5).await;
        }
    };
    let result = match select(work.boxed_local(), TimeoutFuture::new(3000).boxed_local()).await {
        Either::Left((result, _)) => result,
        Either::Right(((), pending)) => {
            drop(pending);
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "callback fixture observation exceeded 3000ms",
            ))
        }
    };
    // Completion/cancellation drops the socket and detaches all four callbacks.
    TimeoutFuture::new(20).await;
    let before_close = observed.borrow_mut().take();
    (result, before_close, metrics.snapshot())
}

impl AsyncRead for WebSocketIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut state = self.state.borrow_mut();
        if state.failed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WebSocket receive bound or transport failure",
            )));
        }
        if state.read_budget == 0 {
            drop(state);
            let state = self.state.clone();
            let waker = context.waker().clone();
            self.read_wakeup = Some(Timeout::new(0, move || {
                state.borrow_mut().read_budget = 64 * 1024;
                waker.wake();
            }));
            if let Err(error) = self.metrics.yield_decode() {
                return Poll::Ready(Err(error));
            }
            return Poll::Pending;
        }
        if let Some(mut bytes) = state.messages.pop_front() {
            let original_length = bytes.len();
            let count = buffer
                .remaining()
                .min(bytes.len())
                .min(FRAME_BYTES)
                .min(state.read_budget);
            let chunk = bytes.split_to(count);
            if let Err(error) = self
                .metrics
                .observe(true, &chunk)
                .and_then(|()| self.metrics.queue(count, false, bytes.is_empty()))
            {
                state.failed = true;
                if self.metrics.queue(original_length, false, true).is_err() {
                    self.metrics.reject();
                }
                state.clear_queued(&self.metrics);
                let _ = self.socket.close();
                return Poll::Ready(Err(error));
            }
            buffer.put_slice(&chunk);
            state.read_budget -= count;
            state.bytes -= count;
            if !bytes.is_empty() {
                state.messages.push_front(bytes);
            }
            return Poll::Ready(Ok(()));
        }
        if state.closed {
            return Poll::Ready(Ok(()));
        }
        state.waker = Some(context.waker().clone());
        Poll::Pending
    }
}
impl AsyncWrite for WebSocketIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.socket.ready_state() != WebSocket::OPEN {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "WebSocket closed",
            )));
        }
        let buffered = self.socket.buffered_amount() as usize;
        if let Err(error) = self.metrics.backlog(buffered) {
            return Poll::Ready(Err(error));
        }
        if buffered >= MESSAGE_BYTES {
            self.wait_for_write(context);
            return Poll::Pending;
        }
        let count = bytes.len().min(FRAME_BYTES).min(MESSAGE_BYTES - buffered);
        Poll::Ready(
            self.socket
                .send_with_u8_array(&bytes[..count])
                .map_err(|_| io::Error::other("WebSocket send failed"))
                .and_then(|()| {
                    self.metrics.observe(false, &bytes[..count])?;
                    self.metrics
                        .backlog(self.socket.buffered_amount() as usize)?;
                    Ok(count)
                }),
        )
    }
    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(error) = self.metrics.backlog(self.socket.buffered_amount() as usize) {
            return Poll::Ready(Err(error));
        }
        if self.socket.buffered_amount() == 0 {
            Poll::Ready(Ok(()))
        } else {
            self.wait_for_write(context);
            Poll::Pending
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(
            self.socket
                .close()
                .map_err(|_| io::Error::other("WebSocket close failed")),
        )
    }
}

struct AbortOnDrop(AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Owns the local driver lifetime. Drop closes the socket and releases callbacks.
pub struct BrowserConnection {
    driver: AbortOnDrop,
    metrics: Arc<ConnectionMetrics>,
    cancellation: Rc<ConnectionCancellation>,
}
impl Drop for BrowserConnection {
    fn drop(&mut self) {
        self.close();
    }
}
impl BrowserConnection {
    /// Connect one HTTP/2 session with bounded frame, header, DATA credit and concurrency.
    pub async fn connect(url: &str) -> io::Result<(Self, BrowserChannel)> {
        let metrics = Arc::new(ConnectionMetrics::new(true));
        let socket = WebSocketIo::connect(url, metrics.clone()).await?;
        let (sender, connection) = h2::client::Builder::new()
            .initial_window_size(MESSAGE_BYTES as u32)
            .initial_connection_window_size(RECEIVE_BYTES as u32)
            .max_frame_size(FRAME_BYTES as u32)
            .max_header_list_size(FRAME_BYTES as u32)
            .max_send_buffer_size(MESSAGE_BYTES)
            .initial_max_send_streams(CONCURRENT_STREAMS as usize)
            .enable_push(false)
            .handshake(socket)
            .await
            .map_err(io::Error::other)?;
        let (abort, registration) = AbortHandle::new_pair();
        let driver_metrics = metrics.clone();
        let cancellation = Rc::new(ConnectionCancellation::new());
        let driver_cancellation = cancellation.clone();
        let context = df_observe::OperationContext {
            trace_parent: String::new(),
            build: option_env!("DF_FIXTURE_BUILD")
                .unwrap_or("unregistered-cargo-build")
                .to_owned(),
        };
        let mut span = df_observe::begin(&context, "bridge.driver");
        wasm_bindgen_futures::spawn_local(async move {
            let result = Abortable::new(connection, registration).await;
            driver_cancellation.cancel();
            let status = match &result {
                Ok(Ok(())) => "closed",
                Ok(Err(_)) => "failed",
                Err(_) => "cancelled",
            };
            driver_metrics.driver_finished(matches!(result, Ok(Err(_))));
            match driver_metrics.snapshot() {
                Ok(value) => span.finish(status, value.received_bytes + value.sent_bytes),
                Err(_) => span.finish_unmeasured("measurement_failed"),
            }
        });
        Ok((
            Self {
                driver: AbortOnDrop(abort),
                metrics: metrics.clone(),
                cancellation: cancellation.clone(),
            },
            BrowserChannel {
                sender,
                credit: ReceiveCredit::new(metrics),
                closed: cancellation.subscribe(),
            },
        ))
    }
    /// Current and peak resource measurements owned by this connection.
    pub fn snapshot(&self) -> io::Result<ConnectionSnapshot> {
        self.metrics.snapshot()
    }
    /// Explicit cancellation is idempotent and owned by the fixture run.
    pub fn close(&self) {
        self.cancellation.cancel();
        self.driver.0.abort();
    }
}

/// Generated tonic-client compatible service, with a local future and local h2 driver.
#[derive(Clone)]
pub struct BrowserChannel {
    sender: h2::client::SendRequest<Bytes>,
    credit: ReceiveCredit,
    closed: ConnectionClosed,
}
impl Service<http::Request<tonic::body::Body>> for BrowserChannel {
    type Response = http::Response<ReceiveBody>;
    type Error = tonic::Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;
    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.sender.poll_ready(context).map_err(transport_status)
    }
    fn call(&mut self, request: http::Request<tonic::body::Body>) -> Self::Future {
        let credit = self.credit.clone();
        let closed = self.closed.clone();
        let (mut parts, body) = request.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map(|value| value.as_str())
            .unwrap_or("/");
        let uri = format!("http://fixture{path}").parse();
        let Ok(uri) = uri else {
            return futures::future::ready(Err(transport_status(
                h2::Reason::PROTOCOL_ERROR.into(),
            )))
            .boxed_local();
        };
        parts.uri = uri;
        parts.version = http::Version::HTTP_2;
        let result = self
            .sender
            .send_request(http::Request::from_parts(parts, ()), false);
        async move {
            let (response, mut send) = result.map_err(transport_status)?;
            let (abort, registration) = AbortHandle::new_pair();
            let guard = AbortOnDrop(abort);
            let (completion, receiver) = oneshot::channel();
            let mut upload = UploadCompletion::new(receiver);
            wasm_bindgen_futures::spawn_local(async move {
                let upload = async {
                    let mut body = body;
                    while let Some(frame) = body.frame().await {
                        let frame = frame?;
                        match frame.into_data() {
                            Ok(mut bytes) => {
                                while !bytes.is_empty() {
                                    send.reserve_capacity(bytes.len().min(FRAME_BYTES));
                                    let capacity = poll_fn(|context| send.poll_capacity(context))
                                        .await
                                        .ok_or_else(|| transport_status(h2::Reason::CANCEL.into()))?
                                        .map_err(transport_status)?;
                                    let count = capacity.min(bytes.len()).min(FRAME_BYTES);
                                    if count != 0 {
                                        send.send_data(bytes.split_to(count), false)
                                            .map_err(transport_status)?;
                                    }
                                }
                            }
                            Err(frame) => {
                                let trailers = frame.into_trailers().map_err(|_| {
                                    transport_status(h2::Reason::PROTOCOL_ERROR.into())
                                })?;
                                send.send_trailers(trailers).map_err(transport_status)?;
                                return Ok(());
                            }
                        }
                    }
                    send.send_data(Bytes::new(), true)
                        .map_err(transport_status)?;
                    Ok::<(), tonic::Status>(())
                };
                let result = {
                    let mut upload = Box::pin(Abortable::new(upload, registration));
                    let mut closed = closed;
                    poll_fn(|context| {
                        // A ready original result wins over simultaneous connection closure.
                        if let Poll::Ready(result) = upload.as_mut().poll(context) {
                            return Poll::Ready(match result {
                                Ok(Ok(())) => UploadResult::Completed,
                                Ok(Err(status)) => UploadResult::Failed(status),
                                Err(_) => UploadResult::Cancelled,
                            });
                        }
                        if Pin::new(&mut closed).poll(context).is_ready() {
                            return Poll::Ready(UploadResult::Cancelled);
                        }
                        Poll::Pending
                    })
                    .await
                };
                let failed = matches!(result, UploadResult::Failed(_));
                // Publish the original failure before reset can wake the response future.
                // A missing receiver means its response owner has already cancelled/dropped.
                let _ = completion.send(result);
                if failed {
                    send.send_reset(h2::Reason::INTERNAL_ERROR);
                }
            });
            let mut response = Box::pin(response);
            let response = poll_fn(|context| {
                if let Poll::Ready(Err(status)) = upload.poll_completion(context, false) {
                    return Poll::Ready(Err(status));
                }
                response.as_mut().poll(context).map_err(transport_status)
            })
            .await?;
            let (parts, mut receive) = response.into_parts();
            let id = credit
                .register(receive.flow_control().clone())
                .map_err(transport_status)?;
            Ok(http::Response::from_parts(
                parts,
                ReceiveBody {
                    receive,
                    upload_guard: guard,
                    upload,
                    terminal_received: false,
                    terminal_trailers: None,
                    data_finished: false,
                    finished: false,
                    previous_frame: 0,
                    credit,
                    id: Some(id),
                },
            ))
        }
        .boxed_local()
    }
}

/// DATA and trailers remain distinct HTTP/2 frames. Credit follows body consumption.
pub struct ReceiveBody {
    receive: h2::RecvStream,
    upload_guard: AbortOnDrop,
    upload: UploadCompletion,
    terminal_received: bool,
    terminal_trailers: Option<http::HeaderMap>,
    data_finished: bool,
    finished: bool,
    previous_frame: usize,
    credit: ReceiveCredit,
    id: Option<h2::StreamId>,
}
impl ReceiveBody {
    fn finish(&mut self) -> Result<(), h2::Error> {
        if let Some(id) = self.id.take() {
            self.credit.finish(id)?;
        }
        Ok(())
    }
}
impl Drop for ReceiveBody {
    fn drop(&mut self) {
        // The coordinator records a failed release; Drop cannot return it.
        let _ = self.finish();
    }
}
impl Body for ReceiveBody {
    type Data = Bytes;
    type Error = tonic::Status;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.finished {
            return Poll::Ready(None);
        }
        let terminal_received = self.terminal_received;
        if let Poll::Ready(Err(status)) = self.upload.poll_completion(context, terminal_received) {
            self.finished = true;
            self.upload_guard.0.abort();
            // Credit release records its own failure; it must not erase the upload status.
            let _ = self.finish();
            return Poll::Ready(Some(Err(status)));
        }
        if !self.data_finished {
            let previous_frame = std::mem::take(&mut self.previous_frame);
            let Some(id) = self.id else {
                self.finished = true;
                self.upload_guard.0.abort();
                return Poll::Ready(Some(Err(transport_status(
                    h2::Reason::INTERNAL_ERROR.into(),
                ))));
            };
            if let Err(error) = self.credit.consumed(id, previous_frame) {
                self.finished = true;
                self.upload_guard.0.abort();
                let _ = self.finish();
                return Poll::Ready(Some(Err(transport_status(error))));
            }
            match self.receive.poll_data(context) {
                Poll::Ready(Some(Ok(bytes))) => {
                    self.previous_frame = bytes.len();
                    return Poll::Ready(Some(Ok(Frame::data(bytes))));
                }
                Poll::Ready(Some(Err(error))) => {
                    self.finished = true;
                    self.upload_guard.0.abort();
                    return Poll::Ready(Some(Err(transport_status(
                        self.finish().err().unwrap_or(error),
                    ))));
                }
                Poll::Ready(None) => {
                    self.data_finished = true;
                    if let Err(error) = self.finish() {
                        self.finished = true;
                        self.upload_guard.0.abort();
                        return Poll::Ready(Some(Err(transport_status(error))));
                    }
                }
                Poll::Pending => {
                    if let Err(error) = self.credit.pending(id) {
                        self.finished = true;
                        self.upload_guard.0.abort();
                        let _ = self.finish();
                        return Poll::Ready(Some(Err(transport_status(error))));
                    }
                    return Poll::Pending;
                }
            }
        }
        if !self.terminal_received {
            match self.receive.poll_trailers(context) {
                Poll::Ready(Ok(trailers)) => {
                    self.terminal_received = true;
                    self.terminal_trailers = trailers;
                    // A server may finish while client/bidi input is still open. Cancel its
                    // owned upload, then observe completion before exposing terminal frames.
                    self.upload_guard.0.abort();
                }
                Poll::Ready(Err(error)) => {
                    self.finished = true;
                    self.upload_guard.0.abort();
                    return Poll::Ready(Some(Err(transport_status(error))));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        match self.upload.poll_completion(context, true) {
            Poll::Ready(Ok(())) => {
                self.finished = true;
                Poll::Ready(
                    self.terminal_trailers
                        .take()
                        .map(|trailers| Ok(Frame::trailers(trailers))),
                )
            }
            Poll::Ready(Err(status)) => {
                self.finished = true;
                Poll::Ready(Some(Err(status)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
    fn is_end_stream(&self) -> bool {
        self.finished
    }
}

fn transport_status(error: h2::Error) -> tonic::Status {
    tonic::Status::from_error(Box::new(error))
}
