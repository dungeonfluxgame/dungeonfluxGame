use axum::{
    Router,
    extract::{Path, RawQuery, State, WebSocketUpgrade, ws::Message},
    http::HeaderMap,
    response::{Html, IntoResponse},
    routing::get,
};
use df_observe::{FixtureTelemetry, OperationContext};
use df_protocol::transport_fixture::{
    Sample,
    sample::Behavior,
    transport_fixture_server::{TransportFixture, TransportFixtureServer},
};
use df_rpc_bridge::{
    CONCURRENT_STREAMS, ConnectionMetrics, FRAME_BYTES, MESSAGE_BYTES, NativeAdmission,
    NativeIncoming, RPC_MESSAGE_BYTES,
};
use futures::{SinkExt, Stream, StreamExt};
use std::{
    collections::VecDeque,
    io,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use tokio::sync::Semaphore;
use tonic::{Code, Request, Response, Status, Streaming, metadata::MetadataMap};

type ResponseStream = Pin<Box<dyn Stream<Item = Result<Sample, Status>> + Send>>;
#[derive(Default)]
struct Statistics {
    active: AtomicU32,
    cancelled: AtomicU32,
    completed: AtomicU32,
    deadlines: AtomicU32,
}
struct CallScope {
    statistics: Arc<Statistics>,
    span: df_observe::OperationSpan,
    status: Code,
    complete: bool,
    payload_bytes: usize,
}
impl CallScope {
    fn finish(&mut self) {
        self.complete = true;
        self.status = Code::Ok;
    }
}
impl CallScope {
    fn finish_with(&mut self, status: Code) {
        self.complete = true;
        self.status = status;
    }
}
impl Drop for CallScope {
    fn drop(&mut self) {
        self.statistics.active.fetch_sub(1, Ordering::Relaxed);
        let status = if self.complete {
            self.statistics.completed.fetch_add(1, Ordering::Relaxed);
            "complete"
        } else {
            self.statistics.cancelled.fetch_add(1, Ordering::Relaxed);
            "cancelled"
        };
        self.span
            .finish(&format!("{status}:{:?}", self.status), self.payload_bytes);
    }
}
#[derive(Clone)]
struct FixtureService {
    statistics: Arc<Statistics>,
    telemetry: Arc<FixtureTelemetry>,
}
impl FixtureService {
    fn scope<T>(&self, request: &Request<T>, method: &'static str) -> CallScope {
        self.statistics.active.fetch_add(1, Ordering::Relaxed);
        if request.metadata().get("grpc-timeout").is_some() {
            self.statistics.deadlines.fetch_add(1, Ordering::Relaxed);
        }
        let trace_parent = request
            .metadata()
            .get("traceparent")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        CallScope {
            statistics: self.statistics.clone(),
            span: self.telemetry.begin(
                &OperationContext {
                    trace_parent,
                    build: crate::BUILD_ID.to_owned(),
                },
                method,
            ),
            status: Code::Cancelled,
            complete: false,
            payload_bytes: 0,
        }
    }
}
fn terminal(code: Code) -> Status {
    let mut metadata = MetadataMap::new();
    metadata.insert(
        "fixture-terminal",
        tonic::metadata::MetadataValue::from_static("observed"),
    );
    Status::with_metadata(code, "synthetic terminal", metadata)
}
fn check(sample: &Sample) -> Result<(), Status> {
    if sample.behavior == Behavior::Reject as i32 {
        return Err(terminal(Code::InvalidArgument));
    }
    Ok(())
}
#[tonic::async_trait]
impl TransportFixture for FixtureService {
    async fn unary(&self, request: Request<Sample>) -> Result<Response<Sample>, Status> {
        let mut scope = self.scope(&request, "fixture.unary");
        let sample = request.into_inner();
        scope.payload_bytes += sample.payload.len();
        if let Err(error) = check(&sample) {
            scope.finish_with(error.code());
            return Err(error);
        }
        if sample.behavior == Behavior::Wait as i32 {
            return futures::future::pending().await;
        }
        let sample = if sample.payload == b"stats" {
            Sample {
                sequence: self.statistics.cancelled.load(Ordering::Relaxed),
                payload: vec![],
                behavior: Behavior::Echo as i32,
                active_calls: self.statistics.active.load(Ordering::Relaxed),
                deadline_calls: self.statistics.deadlines.load(Ordering::Relaxed),
            }
        } else {
            sample
        };
        scope.payload_bytes += sample.payload.len();
        let mut response = Response::new(sample);
        response.metadata_mut().insert(
            "fixture-server",
            tonic::metadata::MetadataValue::from_static("native-tonic"),
        );
        scope.finish();
        Ok(response)
    }
    type ServerStreamStream = ResponseStream;
    async fn server_stream(
        &self,
        request: Request<Sample>,
    ) -> Result<Response<Self::ServerStreamStream>, Status> {
        let mut scope = self.scope(&request, "fixture.server_stream");
        let sample = request.into_inner();
        scope.payload_bytes += sample.payload.len();
        if let Err(status) = check(&sample) {
            scope.finish_with(status.code());
            return Err(status);
        }
        let stream = futures::stream::unfold(
            (0, sample, scope),
            |(sequence, sample, mut scope)| async move {
                if sample.behavior == Behavior::Wait as i32 && sequence > 0 {
                    return futures::future::pending().await;
                }
                let pressure = sample.payload.starts_with(b"pressure:");
                let limit = if pressure {
                    sample.sequence.clamp(1, 128)
                } else {
                    3
                };
                if sequence < limit {
                    let output = if pressure {
                        let paced = sample.payload == b"pressure:paced";
                        if paced {
                            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
                        }
                        Sample {
                            sequence,
                            payload: vec![0x5a; if paced { 8 * 1024 } else { 48 * 1024 }],
                            ..Sample::default()
                        }
                    } else {
                        Sample {
                            sequence,
                            ..sample.clone()
                        }
                    };
                    scope.payload_bytes += output.payload.len();
                    return Some((Ok(output), (sequence + 1, sample, scope)));
                }
                if sequence == limit {
                    scope.finish();
                    return Some((Err(terminal(Code::Ok)), (limit + 1, sample, scope)));
                }
                None
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }
    async fn client_stream(
        &self,
        request: Request<Streaming<Sample>>,
    ) -> Result<Response<Sample>, Status> {
        let mut scope = self.scope(&request, "fixture.client_stream");
        let mut incoming = request.into_inner();
        let mut count = 0;
        while let Some(sample) = incoming.message().await? {
            scope.payload_bytes += sample.payload.len();
            if let Err(status) = check(&sample) {
                scope.finish_with(status.code());
                return Err(status);
            }
            count += 1;
            if count > 16 {
                scope.finish_with(Code::ResourceExhausted);
                return Err(terminal(Code::ResourceExhausted));
            }
        }
        scope.payload_bytes += b"half-close observed".len();
        scope.finish();
        Ok(Response::new(Sample {
            sequence: count,
            payload: b"half-close observed".to_vec(),
            behavior: Behavior::Echo as i32,
            ..Sample::default()
        }))
    }
    type BidiStream = ResponseStream;
    async fn bidi(
        &self,
        request: Request<Streaming<Sample>>,
    ) -> Result<Response<Self::BidiStream>, Status> {
        let scope = self.scope(&request, "fixture.bidi");
        let stream = futures::stream::unfold(
            (request.into_inner(), scope, 0, false),
            |(mut incoming, mut scope, count, finished)| async move {
                if finished {
                    return None;
                }
                match incoming.message().await {
                    Ok(Some(sample)) if count < 16 => {
                        scope.payload_bytes += sample.payload.len();
                        let result = check(&sample).map(|()| {
                            scope.payload_bytes += sample.payload.len();
                            sample
                        });
                        let finished = result.is_err();
                        if let Err(status) = &result {
                            scope.finish_with(status.code());
                        }
                        Some((result, (incoming, scope, count + 1, finished)))
                    }
                    Ok(Some(_)) => {
                        scope.finish_with(Code::ResourceExhausted);
                        Some((
                            Err(terminal(Code::ResourceExhausted)),
                            (incoming, scope, count, true),
                        ))
                    }
                    Ok(None) => {
                        scope.finish();
                        Some((Err(terminal(Code::Ok)), (incoming, scope, count, true)))
                    }
                    Err(status) => Some((Err(status), (incoming, scope, count, true))),
                }
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }
}
#[derive(Clone)]
struct PreviewState {
    incoming: NativeAdmission,
    telemetry: Arc<FixtureTelemetry>,
    statistics: Arc<Statistics>,
    connections: Arc<Semaphore>,
    resources: Arc<Mutex<VecDeque<Arc<ConnectionMetrics>>>>,
    origin: Arc<str>,
    reports: Arc<Mutex<ReportState>>,
    shapes: Arc<Mutex<VecDeque<ShapeWitness>>>,
}
#[derive(Clone)]
struct ShapeWitness {
    id: u64,
    mode: String,
    messages: usize,
    frames: usize,
    total_bytes: usize,
    domain_calls: u32,
    closed: bool,
    failed: bool,
}
fn record_shape(state: &PreviewState, witness: ShapeWitness) {
    if let Ok(mut shapes) = state.shapes.lock() {
        if shapes.len() == 20 {
            shapes.pop_front();
        }
        shapes.push_back(witness);
    }
}
async fn shape_report(
    State(state): State<PreviewState>,
    Path(id): Path<u64>,
) -> axum::response::Response {
    let Ok(shapes) = state.shapes.lock() else {
        return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    match shapes.iter().rev().find(|shape| shape.id == id) {
        Some(shape) => format!(
            "mode={} messages={} frames={} bytes={} domain_calls={} closed={} failed={} build={}",
            shape.mode,
            shape.messages,
            shape.frames,
            shape.total_bytes,
            shape.domain_calls,
            shape.closed,
            shape.failed,
            crate::BUILD_ID,
        )
        .into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}
async fn read_h2_frame(
    reader: &mut tokio::io::ReadHalf<tokio::io::DuplexStream>,
) -> io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut header = [0; 9];
    reader.read_exact(&mut header).await?;
    let length =
        (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
    if length > FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "shape frame exceeds bound",
        ));
    }
    let mut frame = header.to_vec();
    frame.resize(9 + length, 0);
    reader.read_exact(&mut frame[9..]).await?;
    Ok(frame)
}
// Test-only source-bound delivery shapes. The browser still runs its production
// WebSocketIo/h2/generated-client stack; this fixture owns the peer's WS writes.
async fn byte_shape(
    State(state): State<PreviewState>,
    Path((mode, id)): Path<(String, u64)>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    if headers.get("origin").and_then(|value| value.to_str().ok()) != Some(state.origin.as_ref()) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    if !matches!(
        mode.as_str(),
        "split" | "coalesced" | "text" | "loss" | "pending-loss"
    ) {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    let Ok(permit) = state.connections.clone().try_acquire_owned() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    upgrade
        .read_buffer_size(FRAME_BYTES)
        .write_buffer_size(FRAME_BYTES)
        .max_write_buffer_size(MESSAGE_BYTES)
        .max_message_size(MESSAGE_BYTES)
        .max_frame_size(MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            let mut witness = ShapeWitness {
                id,
                mode: mode.clone(),
                messages: 0,
                frames: 0,
                total_bytes: 0,
                domain_calls: 0,
                closed: false,
                failed: false,
            };
            if mode == "text" {
                let (mut outgoing, mut incoming) = socket.split();
                // Require real browser h2 preface activity before injecting the unsupported frame.
                let result =
                    tokio::time::timeout(std::time::Duration::from_secs(2), incoming.next()).await;
                if result.is_ok_and(|message| message.is_some_and(|message| message.is_ok())) {
                    witness.messages = 1;
                    witness.total_bytes = 16;
                    witness.failed = outgoing
                        .send(Message::Text("unsupported text".into()))
                        .await
                        .is_err();
                } else {
                    witness.failed = true;
                }
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(2), incoming.next()).await;
                witness.closed = true;
                record_shape(&state, witness);
                return;
            }
            let statistics = Arc::new(Statistics::default());
            let service = TransportFixtureServer::new(FixtureService {
                statistics: statistics.clone(),
                telemetry: state.telemetry.clone(),
            })
            .max_decoding_message_size(RPC_MESSAGE_BYTES)
            .max_encoding_message_size(RPC_MESSAGE_BYTES);
            let (peer, grpc_end) = tokio::io::duplex(64 * 1024);
            let server = tokio::spawn(async move {
                tonic::transport::Server::builder()
                    .initial_stream_window_size(MESSAGE_BYTES as u32)
                    .initial_connection_window_size(df_rpc_bridge::RECEIVE_BYTES as u32)
                    .max_frame_size(FRAME_BYTES as u32)
                    .http2_max_header_list_size(FRAME_BYTES as u32)
                    .max_concurrent_streams(CONCURRENT_STREAMS)
                    .add_service(service)
                    .serve_with_incoming(futures::stream::once(async move {
                        Ok::<_, io::Error>(grpc_end)
                    }))
                    .await
            });
            let (mut outgoing, mut incoming) = socket.split();
            let (mut reader, mut writer) = tokio::io::split(peer);
            let receive = async {
                use tokio::io::AsyncWriteExt;
                while let Some(message) = incoming.next().await {
                    match message.map_err(io::Error::other)? {
                        Message::Binary(bytes) if bytes.len() <= MESSAGE_BYTES => {
                            writer.write_all(&bytes).await?;
                        }
                        Message::Close(_) => return Ok::<(), io::Error>(()),
                        Message::Ping(_) | Message::Pong(_) => {}
                        _ => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "nonbinary shape input",
                            ));
                        }
                    }
                }
                Ok(())
            };
            let send = async {
                use tokio::io::AsyncReadExt;
                let first = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    read_h2_frame(&mut reader),
                )
                .await
                .map_err(io::Error::other)??;
                if mode == "split" {
                    for byte in &first {
                        outgoing
                            .send(Message::Binary(vec![*byte].into()))
                            .await
                            .map_err(io::Error::other)?;
                    }
                    witness.messages = first.len();
                    witness.frames = 1;
                    witness.total_bytes = first.len();
                } else if mode == "coalesced" {
                    let second = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        read_h2_frame(&mut reader),
                    )
                    .await
                    .map_err(io::Error::other)??;
                    let mut combined = first;
                    combined.extend(second);
                    outgoing
                        .send(Message::Binary(combined.clone().into()))
                        .await
                        .map_err(io::Error::other)?;
                    witness.messages = 1;
                    witness.frames = 2;
                    witness.total_bytes = combined.len();
                } else {
                    outgoing
                        .send(Message::Binary(first.clone().into()))
                        .await
                        .map_err(io::Error::other)?;
                    witness.messages = 1;
                    witness.frames = 1;
                    witness.total_bytes = first.len();
                }
                let mut buffer = [0; FRAME_BYTES];
                loop {
                    let count = reader.read(&mut buffer).await?;
                    if count == 0 {
                        return Ok::<(), io::Error>(());
                    }
                    outgoing
                        .send(Message::Binary(buffer[..count].to_vec().into()))
                        .await
                        .map_err(io::Error::other)?;
                }
            };
            let result = if matches!(mode.as_str(), "loss" | "pending-loss") {
                let wait_for_dispatch = async {
                    tokio::time::timeout(std::time::Duration::from_secs(2), async {
                        loop {
                            let observed = if mode == "pending-loss" {
                                statistics.completed.load(Ordering::Relaxed)
                            } else {
                                statistics.active.load(Ordering::Relaxed)
                            };
                            if observed != 0 {
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                        }
                    })
                    .await
                    .map_err(io::Error::other)?;
                    Ok::<(), io::Error>(())
                };
                let result = tokio::select! {
                    result = receive => result,
                    result = send => result,
                    result = wait_for_dispatch => result,
                };
                // Pending loss records the actual completed trigger; ordinary loss keeps
                // its existing preterminal active-call witness.
                witness.domain_calls = if mode == "pending-loss" {
                    statistics.completed.load(Ordering::Relaxed)
                } else {
                    statistics.active.load(Ordering::Relaxed)
                };
                result
            } else {
                tokio::select! { result = receive => result, result = send => result }
            };
            witness.failed = result.is_err() || witness.frames == 0;
            witness.closed = true;
            if !matches!(mode.as_str(), "loss" | "pending-loss") {
                witness.domain_calls = statistics.completed.load(Ordering::Relaxed)
                    + statistics.cancelled.load(Ordering::Relaxed);
            }
            server.abort();
            let _ = server.await;
            record_shape(&state, witness);
        })
}
struct ReportState {
    slots: [String; 5],
    credit_order: Option<(u64, u8)>,
}
impl ReportState {
    fn write(&mut self, index: usize, report: String, order: Option<(u64, u8)>) -> bool {
        if index == 3 {
            let Some(order) = order else {
                return false;
            };
            if self.credit_order.is_some_and(|current| order <= current) {
                return false;
            }
            self.credit_order = Some(order);
        }
        self.slots[index] = report;
        true
    }
}
fn credit_report_order(query: Option<&str>) -> Option<(u64, u8)> {
    let (generation, phase) = query?.split_once("&phase=")?;
    let generation = generation.strip_prefix("generation=")?.parse().ok()?;
    let phase = phase.parse().ok()?;
    (generation > 0 && phase <= 2).then_some((generation, phase))
}
async fn websocket(
    State(state): State<PreviewState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    if headers.get("origin").and_then(|value| value.to_str().ok()) != Some(state.origin.as_ref()) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "synthetic loopback origin required",
        )
            .into_response();
    }
    let Ok(permit) = state.connections.clone().try_acquire_owned() else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "fixture connection limit reached",
        )
            .into_response();
    };
    let admission = match state.incoming.try_reserve() {
        Ok(admission) => admission,
        Err(_) => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "fixture incoming unavailable",
            )
                .into_response();
        }
    };
    let metrics = Arc::new(ConnectionMetrics::new(false));
    match state.resources.lock() {
        Ok(mut resources) => {
            if resources.len() >= 20 {
                if let Some(index) = resources.iter().position(|value| {
                    value
                        .snapshot()
                        .is_ok_and(|value| value.closed && value.pipe_owners == 0)
                }) {
                    resources.remove(index);
                } else {
                    return (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        "metrics owner capacity reached",
                    )
                        .into_response();
                }
            }
            resources.push_back(metrics.clone());
        }
        Err(_) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "metrics unavailable",
            )
                .into_response();
        }
    }
    upgrade
        .read_buffer_size(FRAME_BYTES)
        .write_buffer_size(FRAME_BYTES)
        .max_write_buffer_size(MESSAGE_BYTES)
        .max_message_size(MESSAGE_BYTES)
        .max_frame_size(MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let context = OperationContext {
                trace_parent: String::new(),
                build: crate::BUILD_ID.to_owned(),
            };
            let result = admission
                .accept_websocket_measured(socket, metrics.clone())
                .await;
            drop(permit);
            let mut span = state.telemetry.begin(&context, "bridge.connection");
            match metrics.snapshot() {
                Ok(value) => span.finish(
                    if result.is_ok() { "closed" } else { "failed" },
                    value.received_bytes + value.sent_bytes,
                ),
                Err(_) => span.finish_unmeasured("measurement_failed"),
            }
        })
}
async fn health(State(state): State<PreviewState>) -> impl IntoResponse {
    match state.telemetry.count() {
        Ok(spans) => format!(
            "S00 experimental; active={}; completed={}; cancelled={}; otel_spans={spans}; durable_export=false; dropped_spans={}; deadline_headers={}",
            state.statistics.active.load(Ordering::Relaxed),
            state.statistics.completed.load(Ordering::Relaxed),
            state.statistics.cancelled.load(Ordering::Relaxed),
            state.telemetry.dropped(),
            state.statistics.deadlines.load(Ordering::Relaxed)
        ),
        Err(error) => format!("telemetry failure: {error}"),
    }
}
async fn fixture_telemetry(State(state): State<PreviewState>) -> impl IntoResponse {
    match state.telemetry.snapshot() {
        Ok(spans) => spans,
        Err(error) => format!("telemetry export failure: {error}"),
    }
}
async fn fixture_resources(State(state): State<PreviewState>) -> impl IntoResponse {
    match state.resources.lock() {
        Ok(resources) => resources
            .iter()
            .map(|metrics| format!("{:?}", metrics.snapshot()))
            .collect::<Vec<_>>()
            .join("\n"),
        Err(error) => format!("resource snapshot failure: {error}"),
    }
}
fn report_index(kind: &str) -> Option<usize> {
    match kind {
        "semantics" => Some(0),
        "qualification" => Some(1),
        "callback-capacity" => Some(2),
        "connection-credit" => Some(3),
        "close-reconnect" => Some(4),
        _ => None,
    }
}
async fn read_report(
    State(state): State<PreviewState>,
    axum::extract::Path(kind): axum::extract::Path<String>,
) -> axum::response::Response {
    let Some(index) = report_index(&kind) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    match state.reports.lock() {
        Ok(reports) => reports.slots[index].clone().into_response(),
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
async fn write_report(
    State(state): State<PreviewState>,
    axum::extract::Path(kind): axum::extract::Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    report: String,
) -> axum::response::Response {
    if headers.get("origin").and_then(|value| value.to_str().ok()) != Some(state.origin.as_ref()) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    let Some(index) = report_index(&kind) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    if report.len() > 64 * 1024 || report.is_empty() {
        return axum::http::StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let order = if index == 3 {
        match credit_report_order(query.as_deref()) {
            Some(order) => Some(order),
            None => return axum::http::StatusCode::BAD_REQUEST.into_response(),
        }
    } else {
        None
    };
    match state.reports.lock() {
        Ok(mut reports) => {
            if reports.write(index, report, order) {
                axum::http::StatusCode::NO_CONTENT.into_response()
            } else {
                axum::http::StatusCode::CONFLICT.into_response()
            }
        }
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
// Finite synthetic malicious server inputs. No user/provider data or unbounded sender.
async fn malicious(
    State(state): State<PreviewState>,
    axum::extract::Path(kind): axum::extract::Path<String>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    if headers.get("origin").and_then(|value| value.to_str().ok()) != Some(state.origin.as_ref()) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = state.connections.clone().try_acquire_owned() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    upgrade
        .read_buffer_size(FRAME_BYTES)
        .write_buffer_size(FRAME_BYTES)
        .max_write_buffer_size(MESSAGE_BYTES * 2)
        .max_message_size(MESSAGE_BYTES)
        .max_frame_size(MESSAGE_BYTES)
        .on_upgrade(move |mut socket| async move {
            let _permit = permit;
            #[cfg(all(debug_assertions, feature = "browser-callback-overflow-fixture"))]
            if kind == "callback-overflow" {
                let Ok(Some(Ok(axum::extract::ws::Message::Binary(preface)))) =
                    tokio::time::timeout(std::time::Duration::from_secs(1), socket.next()).await
                else {
                    return;
                };
                if preface.as_ref() != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" {
                    return;
                }
                // Exactly five independent engine callbacks. No SETTINGS prefix:
                // the private fixture adapter never polls the incoming byte stream.
                let payload = bytes::Bytes::from(vec![0; MESSAGE_BYTES]);
                let sent = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    for _ in 0..5 {
                        socket
                            .send(axum::extract::ws::Message::Binary(payload.clone()))
                            .await?;
                    }
                    Ok::<(), axum::Error>(())
                })
                .await;
                if sent.is_ok_and(|result| result.is_ok()) {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                        while socket.next().await.is_some() {}
                    })
                    .await;
                }
                return;
            }
            // Wait for the real h2 client's preface before emitting valid SETTINGS followed by abuse.
            if tokio::time::timeout(std::time::Duration::from_secs(1), socket.next())
                .await
                .is_err()
            {
                return;
            }
            let mut wire = vec![0, 0, 0, 4, 0, 0, 0, 0, 0];
            match kind.as_str() {
                "websocket" => {
                    wire.resize(MESSAGE_BYTES + 1, 0);
                }
                "frame" => {
                    wire.extend([0, 0x40, 1, 1, 4, 0, 0, 0, 1]);
                    wire.resize(wire.len() + FRAME_BYTES + 1, 0);
                }
                "header" => {
                    // Small HPACK block, decoded list exceeds the advertised 16KiB limit.
                    let mut block = vec![0x88];
                    for _ in 0..300 {
                        block.extend([0, 1, b'x', 32]);
                        block.extend([b'a'; 32]);
                    }
                    let length = block.len();
                    wire.extend([0, (length >> 8) as u8, length as u8, 1, 4, 0, 0, 0, 1]);
                    wire.extend(block);
                }
                "continuation" => {
                    wire.extend([0, 0, 1, 1, 0, 0, 0, 0, 1, 0x82]);
                    for _ in 0..128 {
                        wire.extend([0, 0, 1, 9, 0, 0, 0, 0, 1, 0x82]);
                    }
                }
                "flood" => {
                    for _ in 0..2049 {
                        wire.extend([0, 0, 0, 4, 0, 0, 0, 0, 0]);
                    }
                }
                "control-rate" => {
                    // The prefix already contains initial SETTINGS: exactly 100
                    // more controls make total frame 101 the first rejection.
                    for _ in 0..100 {
                        wire.extend([0, 0, 0, 4, 1, 0, 0, 0, 0]);
                    }
                }
                _ => return,
            }
            if tokio::time::timeout(
                std::time::Duration::from_secs(2),
                socket.send(axum::extract::ws::Message::Binary(wire.into())),
            )
            .await
            .is_ok_and(|result| result.is_ok())
            {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    while socket.next().await.is_some() {}
                })
                .await;
            }
        })
}
const HTML: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>DungeonFlux S00 transport laboratory</title><style>body{margin:0;background:#10141c;color:#e6eaf3;font:17px system-ui,sans-serif}main{max-width:880px;margin:64px auto;padding:0 24px}h1{font-size:38px;line-height:1.1}p{color:#b7c3d4;line-height:1.5}button{background:#88dfb5;color:#10141c;border:0;border-radius:8px;font:600 17px system-ui;padding:14px 22px;cursor:pointer}button:disabled{opacity:.5}pre{white-space:pre-wrap;padding:20px;background:#1a2331;line-height:1.7;border-radius:10px}.tag{color:#88dfb5;letter-spacing:2px;font-size:13px}a{color:#88dfb5}</style></head><body><main><div class="tag">DUNGEONFLUX / EXPERIMENTAL S00</div><h1>Rust transport laboratory</h1><p>Generated protobuf calls travel as native HTTP/2 gRPC bytes over one binary WebSocket. The browser driver and this fixture interface are Rust compiled to single-threaded WebAssembly.</p><p>This starts the execution foundation. Gameplay, production authentication, durable telemetry and physical device qualification remain pending.</p><button id="run" disabled>Loading Rust/WASM…</button><button id="qualify" disabled>Loading qualification…</button><button id="callback-capacity" disabled>Loading callback observation…</button><button id="connection-credit" disabled>Loading connection-credit observation…</button><button id="close-reconnect" disabled>Loading close/reconnect observation…</button><pre id="close-reconnect-report" role="status" aria-live="polite">Close/reconnect observation has not run.</pre><pre id="connection-credit-report" role="status" aria-live="polite">Connection-credit stall observation has not run.</pre><pre id="callback-capacity-report" role="status" aria-live="polite">Callback capacity observation has not run.</pre><pre id="qualification" role="status" aria-live="polite">Desktop pressure qualification has not run.</pre><pre id="results" role="status" aria-live="polite">Loading generated WebAssembly bindings…</pre><p><a href="/fixture-health">Native fixture diagnostics</a></p><p id="build"></p></main><script type="module">import init from '/pkg/df_tools.js';await init();</script></body></html>"#;

/// Start only a synthetic loopback preview. The caller owns process and output directory.
pub async fn serve(
    build: Arc<crate::build_set::ReadyBuild>,
) -> Result<(), Box<dyn std::error::Error>> {
    let preview_configuration = crate::preview::PreviewConfiguration::from_environment()?;
    let port = build.port;
    let web_root = build.web_root();
    let set_id = build.set_id();
    let mut metadata = format!("<meta name=\"df-build-set\" content=\"{set_id}\">");
    for revision in [
        df_types::BuildRevision::Source,
        df_types::BuildRevision::Native,
        df_types::BuildRevision::Wasm,
        df_types::BuildRevision::Configuration,
        df_types::BuildRevision::Content,
    ] {
        // RevisionLabel contains only bounded ASCII letters/digits and . _ - : /.
        metadata.push_str(&format!(
            "<meta name=\"df-build-{revision:?}\" content=\"{}\">",
            build.identity().revision(revision).as_str()
        ));
    }
    let html = HTML
        .replace("</head>", &format!("{metadata}</head>"))
        .replace(
            "'/pkg/df_tools.js'",
            &format!("'/build-sets/{set_id}/pkg/df_tools.js'"),
        );
    let asset_build = Arc::clone(&build);
    let telemetry = Arc::new(FixtureTelemetry::default());
    let statistics = Arc::new(Statistics::default());
    let (sender, incoming) =
        NativeIncoming::bounded(NonZeroUsize::new(4).ok_or("invalid fixture incoming capacity")?)?;
    let service = TransportFixtureServer::new(FixtureService {
        statistics: statistics.clone(),
        telemetry: telemetry.clone(),
    })
    .max_decoding_message_size(RPC_MESSAGE_BYTES)
    .max_encoding_message_size(RPC_MESSAGE_BYTES);
    let grpc = tonic::transport::Server::builder()
        .initial_stream_window_size(MESSAGE_BYTES as u32)
        .initial_connection_window_size(df_rpc_bridge::RECEIVE_BYTES as u32)
        .max_frame_size(FRAME_BYTES as u32)
        .http2_max_header_list_size(FRAME_BYTES as u32)
        .max_concurrent_streams(CONCURRENT_STREAMS)
        .add_service(service)
        .serve_with_incoming(incoming);
    let state = PreviewState {
        incoming: sender,
        telemetry: telemetry.clone(),
        statistics,
        connections: Arc::new(Semaphore::new(4)),
        resources: Arc::new(Mutex::new(VecDeque::new())),
        shapes: Arc::new(Mutex::new(VecDeque::new())),
        reports: Arc::new(Mutex::new(ReportState {
            slots: std::array::from_fn(|_| String::new()),
            credit_order: None,
        })),
        origin: format!("http://127.0.0.1:{port}").into(),
    };
    let router = Router::new()
        .route(
            "/",
            get(move || {
                let html = html.clone();
                async move { Html(html) }
            }),
        )
        .route("/tunnel", get(websocket))
        .route("/byte-shape/{mode}/{id}", get(byte_shape))
        .route("/byte-shape-report/{id}", get(shape_report))
        .route("/fixture-health", get(health))
        .route("/fixture-telemetry", get(fixture_telemetry))
        .route("/fixture-resources", get(fixture_resources))
        .route(
            "/fixture-report/{kind}",
            get(read_report).post(write_report),
        )
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .route("/malicious/{kind}", get(malicious))
        .route(
            "/build-sets/{id}/pkg/{*path}",
            get(move |Path((id, path)): Path<(String, String)>| {
                let build = Arc::clone(&asset_build);
                async move {
                    match build.asset(&id, &path) {
                        Some(bytes) => {
                            let mime = if path.ends_with(".js") {
                                "text/javascript"
                            } else if path.ends_with(".wasm") {
                                "application/wasm"
                            } else if path.ends_with(".txt") || path.ends_with(".ts") {
                                "text/plain"
                            } else if path.ends_with(".png") {
                                "image/png"
                            } else if path.ends_with(".webp") {
                                "image/webp"
                            } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
                                "image/jpeg"
                            } else {
                                "application/octet-stream"
                            };
                            (
                                [("content-type", mime), ("cache-control", "no-store")],
                                bytes,
                            )
                                .into_response()
                        }
                        None => axum::http::StatusCode::NOT_FOUND.into_response(),
                    }
                }
            }),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let registration = match preview_configuration {
        Some(configuration) => Some(
            configuration
                .register(web_root, listener.local_addr()?)
                .await?,
        ),
        None => None,
    };
    println!(
        "DungeonFlux S00 fixture ready at http://127.0.0.1:{port} · build {} · set {}",
        crate::BUILD_ID,
        set_id
    );
    // Leaving this scope drops both serving futures and the owned listener before
    // the capability is consumed. Errors also take the explicit release path.
    let server_result: Result<(), Box<dyn std::error::Error>> = {
        tokio::select! {
            result = grpc => result.map_err(Into::into),
            result = axum::serve(listener, router) => result.map_err(Into::into),
            result = tokio::signal::ctrl_c() => result.map_err(Into::into),
        }
    };
    let release_result = match registration {
        Some(registration) => registration.release_after_listener_stopped().await,
        None => Ok(()),
    };
    let telemetry_result = telemetry.shutdown();
    if server_result.is_err() || release_result.is_err() || telemetry_result.is_err() {
        return Err(io::Error::other(format!(
            "fixture shutdown: server={server_result:?}; ownership={release_result:?}; telemetry={telemetry_result:?}"
        )).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_protocol::transport_fixture::transport_fixture_client::TransportFixtureClient;
    use futures::StreamExt;

    #[test]
    fn credit_report_rejects_late_run_and_late_success_after_deadline() {
        let mut reports = ReportState {
            slots: std::array::from_fn(|_| String::new()),
            credit_order: None,
        };
        assert_eq!(
            credit_report_order(Some("generation=10&phase=0")),
            Some((10, 0))
        );
        assert_eq!(credit_report_order(Some("generation=10&phase=3")), None);
        assert!(reports.write(3, "starting".to_owned(), Some((10, 0))));
        assert!(reports.write(3, "deadline".to_owned(), Some((10, 2))));
        assert!(!reports.write(3, "late success".to_owned(), Some((10, 1))));
        assert_eq!(reports.slots[3], "deadline");
        assert!(reports.write(3, "new run".to_owned(), Some((11, 0))));
        assert!(!reports.write(3, "late old run".to_owned(), Some((10, 2))));
        assert_eq!(reports.slots[3], "new run");
    }
    #[tokio::test]
    async fn rejected_unary_preserves_terminal_metadata() {
        let service = FixtureService {
            statistics: Arc::new(Statistics::default()),
            telemetry: Arc::new(FixtureTelemetry::default()),
        };
        let error = service
            .unary(Request::new(Sample {
                sequence: 0,
                payload: vec![],
                behavior: Behavior::Reject as i32,
                ..Sample::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        assert_eq!(
            error.metadata().get("fixture-terminal").unwrap(),
            "observed"
        );
    }
    #[tokio::test]
    async fn cancellation_releases_stream_owner() {
        let statistics = Arc::new(Statistics::default());
        let service = FixtureService {
            statistics: statistics.clone(),
            telemetry: Arc::new(FixtureTelemetry::default()),
        };
        let mut stream = service
            .server_stream(Request::new(Sample {
                sequence: 0,
                payload: vec![],
                behavior: Behavior::Wait as i32,
                ..Sample::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(stream.next().await.unwrap().is_ok());
        drop(stream);
        assert_eq!(statistics.active.load(Ordering::Relaxed), 0);
        assert_eq!(statistics.cancelled.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn native_rpc_modes_record_on_the_service_owner() {
        let telemetry = Arc::new(FixtureTelemetry::default());
        let statistics = Arc::new(Statistics::default());
        let service = TransportFixtureServer::new(FixtureService {
            statistics: statistics.clone(),
            telemetry: telemetry.clone(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_shutdown(address, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });
        let endpoint = format!("http://{address}");
        let channel = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match tonic::transport::Endpoint::from_shared(endpoint.clone())
                    .unwrap()
                    .connect()
                    .await
                {
                    Ok(channel) => break channel,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
                }
            }
        })
        .await
        .unwrap();
        let mut client = TransportFixtureClient::new(channel);
        let sample = || Sample {
            sequence: 0,
            payload: b"owner".to_vec(),
            behavior: Behavior::Echo as i32,
            ..Sample::default()
        };

        client.unary(Request::new(sample())).await.unwrap();
        let mut server_stream = client
            .server_stream(Request::new(sample()))
            .await
            .unwrap()
            .into_inner();
        while server_stream.message().await.unwrap().is_some() {}
        let client_input = futures::stream::iter([sample(), sample()]);
        client
            .client_stream(Request::new(client_input))
            .await
            .unwrap();
        let bidi_input = futures::stream::iter([sample(), sample()]);
        let mut bidi = client
            .bidi(Request::new(bidi_input))
            .await
            .unwrap()
            .into_inner();
        while bidi.message().await.unwrap().is_some() {}

        let mut cancelled = client
            .server_stream(Request::new(Sample {
                behavior: Behavior::Wait as i32,
                ..sample()
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(cancelled.message().await.unwrap().is_some());
        drop(cancelled);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while statistics.cancelled.load(Ordering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let spans = telemetry.snapshot().unwrap();
        for name in [
            "fixture.unary",
            "fixture.server_stream",
            "fixture.client_stream",
            "fixture.bidi",
        ] {
            assert!(spans.contains(name), "missing {name}: {spans}");
        }
        assert!(spans.contains("cancelled"), "missing cancellation: {spans}");
        assert_eq!(telemetry.count().unwrap(), 5);
        let _ = shutdown_tx.send(());
        server.await.unwrap();
    }
    struct IncomingTestTasks(Vec<tokio::task::JoinHandle<io::Result<()>>>);
    impl Drop for IncomingTestTasks {
        fn drop(&mut self) {
            for task in &self.0 {
                task.abort();
            }
        }
    }
    impl IncomingTestTasks {
        async fn stop(&mut self) {
            for task in &self.0 {
                task.abort();
            }
            for task in self.0.drain(..) {
                match task.await {
                    Ok(result) => result.unwrap(),
                    Err(error) => assert!(error.is_cancelled()),
                }
            }
        }
    }
    async fn incoming_listener(
        admission: NativeAdmission,
    ) -> (std::net::SocketAddr, PreviewState, IncomingTestTasks) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = PreviewState {
            incoming: admission,
            telemetry: Arc::new(FixtureTelemetry::default()),
            statistics: Arc::new(Statistics::default()),
            connections: Arc::new(Semaphore::new(4)),
            resources: Arc::new(Mutex::new(VecDeque::new())),
            shapes: Arc::new(Mutex::new(VecDeque::new())),
            origin: format!("http://{address}").into(),
            reports: Arc::new(Mutex::new(ReportState {
                slots: std::array::from_fn(|_| String::new()),
                credit_order: None,
            })),
        };
        let router = Router::new()
            .route("/tunnel", get(websocket))
            .route("/byte-shape/{mode}/{id}", get(byte_shape))
            .with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        (address, state, IncomingTestTasks(vec![task]))
    }
    fn tunnel_request(
        address: std::net::SocketAddr,
        origin: &str,
        path: &str,
    ) -> tokio_tungstenite::tungstenite::http::Request<()> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://{address}{path}")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        request
    }
    async fn generated_client_over_websocket(
        address: std::net::SocketAddr,
        origin: Arc<str>,
        path: &'static str,
        tasks: &mut IncomingTestTasks,
    ) -> TransportFixtureClient<tonic::transport::Channel> {
        // Test-only TCP connector preserves generated native client behavior. Its
        // unchanged bytes cross the real WebSocket listener and NativeIncoming.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = listener.local_addr().unwrap();
        tasks.0.push(tokio::spawn(async move {
            use futures::SinkExt;
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            use tokio_tungstenite::tungstenite::Message;
            let (tcp, _) = listener.accept().await?;
            let (socket, _) =
                tokio_tungstenite::connect_async(tunnel_request(address, &origin, path))
                    .await
                    .map_err(io::Error::other)?;
            let (mut websocket_writer, mut websocket_reader) = socket.split();
            let (mut tcp_reader, mut tcp_writer) = tcp.into_split();
            let upload = async {
                let mut buffer = vec![0; FRAME_BYTES];
                loop {
                    let count = tcp_reader.read(&mut buffer).await?;
                    if count == 0 {
                        websocket_writer
                            .send(Message::Close(None))
                            .await
                            .map_err(io::Error::other)?;
                        return Ok::<_, io::Error>(());
                    }
                    websocket_writer
                        .send(Message::Binary(buffer[..count].to_vec().into()))
                        .await
                        .map_err(io::Error::other)?;
                }
            };
            let download = async {
                while let Some(message) = websocket_reader.next().await {
                    match message.map_err(io::Error::other)? {
                        Message::Binary(bytes) => tcp_writer.write_all(&bytes).await?,
                        Message::Ping(_) | Message::Pong(_) => {}
                        Message::Close(_) => return Ok::<_, io::Error>(()),
                        _ => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "nonbinary test tunnel",
                            ));
                        }
                    }
                }
                Ok(())
            };
            tokio::select! { result = upload => result, result = download => result }
        }));
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{proxy_address}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        TransportFixtureClient::new(channel)
    }
    fn start_generated_server(
        incoming: NativeIncoming,
        state: &PreviewState,
        tasks: &mut IncomingTestTasks,
    ) {
        let service = TransportFixtureServer::new(FixtureService {
            statistics: state.statistics.clone(),
            telemetry: state.telemetry.clone(),
        })
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES);
        tasks.0.push(tokio::spawn(async move {
            tonic::transport::Server::builder()
                .initial_stream_window_size(MESSAGE_BYTES as u32)
                .initial_connection_window_size(df_rpc_bridge::RECEIVE_BYTES as u32)
                .max_frame_size(FRAME_BYTES as u32)
                .http2_max_header_list_size(FRAME_BYTES as u32)
                .max_concurrent_streams(CONCURRENT_STREAMS)
                .add_service(service)
                .serve_with_incoming(incoming)
                .await
                .map_err(io::Error::other)
        }));
    }
    async fn wait_for_released_tunnels(state: &PreviewState) {
        loop {
            let released = state.resources.lock().unwrap().iter().all(|metrics| {
                let snapshot = metrics.snapshot().unwrap();
                snapshot.closed && snapshot.pipe_owners == 0 && snapshot.envelope_within_limit()
            });
            if released {
                return;
            }
            tokio::task::yield_now().await;
        }
    }
    #[tokio::test]
    async fn generated_four_modes_use_websocket_native_incoming_and_preserve_status_trailers() {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let (admission, incoming) =
                NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
            let (address, state, mut tasks) = incoming_listener(admission).await;
            start_generated_server(incoming, &state, &mut tasks);
            let mut client = generated_client_over_websocket(
                address,
                state.origin.clone(),
                "/tunnel",
                &mut tasks,
            )
            .await;
            let sample = |sequence| Sample {
                sequence,
                payload: b"native-incoming".to_vec(),
                behavior: Behavior::Echo as i32,
                ..Sample::default()
            };
            let response = client.unary(Request::new(sample(7))).await.unwrap();
            assert_eq!(
                response.metadata().get("fixture-server").unwrap(),
                "native-tonic"
            );
            assert_eq!(response.into_inner(), sample(7));
            let mut stream = client
                .server_stream(Request::new(sample(9)))
                .await
                .unwrap()
                .into_inner();
            for sequence in 0..3 {
                assert_eq!(stream.message().await.unwrap().unwrap(), sample(sequence));
            }
            assert!(stream.message().await.unwrap().is_none());
            assert_eq!(
                stream
                    .trailers()
                    .await
                    .unwrap()
                    .unwrap()
                    .get("fixture-terminal")
                    .unwrap(),
                "observed"
            );
            let response = client
                .client_stream(Request::new(futures::stream::iter([sample(0), sample(1)])))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(response.sequence, 2);
            assert_eq!(response.payload, b"half-close observed");
            let mut bidi = client
                .bidi(Request::new(futures::stream::iter([sample(3), sample(4)])))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(bidi.message().await.unwrap().unwrap(), sample(3));
            assert_eq!(bidi.message().await.unwrap().unwrap(), sample(4));
            assert!(bidi.message().await.unwrap().is_none());
            assert_eq!(
                bidi.trailers()
                    .await
                    .unwrap()
                    .unwrap()
                    .get("fixture-terminal")
                    .unwrap(),
                "observed"
            );
            let status = client
                .unary(Request::new(Sample {
                    behavior: Behavior::Reject as i32,
                    ..sample(0)
                }))
                .await
                .unwrap_err();
            assert_eq!(status.code(), Code::InvalidArgument);
            assert_eq!(
                status.metadata().get("fixture-terminal").unwrap(),
                "observed"
            );
            assert_eq!(
                client
                    .unary(Request::new(sample(8)))
                    .await
                    .unwrap()
                    .into_inner(),
                sample(8)
            );
            assert_eq!(
                state.resources.lock().unwrap().len(),
                1,
                "RPCs must reuse one physical tunnel"
            );
            assert_eq!(state.statistics.completed.load(Ordering::Relaxed), 6);
            assert_eq!(state.statistics.active.load(Ordering::Relaxed), 0);
            drop(client);
            tasks.stop().await;
            wait_for_released_tunnels(&state).await;
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn full_closed_and_origin_refused_incoming_fail_before_websocket_upgrade() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (admission, incoming) =
                NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
            let reservation = admission.try_reserve().unwrap();
            let (address, state, mut tasks) = incoming_listener(admission).await;
            for (origin, expected) in [
                (
                    "http://untrusted.invalid",
                    axum::http::StatusCode::FORBIDDEN,
                ),
                (
                    state.origin.as_ref(),
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                ),
            ] {
                let error =
                    tokio_tungstenite::connect_async(tunnel_request(address, origin, "/tunnel"))
                        .await
                        .unwrap_err();
                let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
                    panic!("expected HTTP refusal");
                };
                assert_eq!(response.status(), expected);
            }
            drop(reservation);
            drop(incoming);
            let error =
                tokio_tungstenite::connect_async(tunnel_request(address, &state.origin, "/tunnel"))
                    .await
                    .unwrap_err();
            let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
                panic!("expected closed-server HTTP refusal");
            };
            assert_eq!(
                response.status(),
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            );
            assert!(state.resources.lock().unwrap().is_empty());
            assert_eq!(state.statistics.active.load(Ordering::Relaxed), 0);
            assert_eq!(state.connections.available_permits(), 4);
            tasks.stop().await;
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn malformed_tunnel_releases_incoming_connection_without_domain_service_dispatch() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            use futures::SinkExt;
            let (admission, incoming) =
                NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
            let (address, state, mut tasks) = incoming_listener(admission).await;
            start_generated_server(incoming, &state, &mut tasks);
            let (mut socket, _) =
                tokio_tungstenite::connect_async(tunnel_request(address, &state.origin, "/tunnel"))
                    .await
                    .unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    "invalid".into(),
                ))
                .await
                .unwrap();
            wait_for_released_tunnels(&state).await;
            let metrics = state.resources.lock().unwrap().front().unwrap().clone();
            assert!(metrics.snapshot().unwrap().rejected);
            assert_eq!(state.statistics.active.load(Ordering::Relaxed), 0);
            assert_eq!(state.statistics.completed.load(Ordering::Relaxed), 0);
            drop(socket);
            let mut client = generated_client_over_websocket(
                address,
                state.origin.clone(),
                "/tunnel",
                &mut tasks,
            )
            .await;
            assert_eq!(
                client
                    .unary(Request::new(Sample {
                        payload: b"healthy-after-failure".to_vec(),
                        behavior: Behavior::Echo as i32,
                        ..Sample::default()
                    }))
                    .await
                    .unwrap()
                    .into_inner()
                    .payload,
                b"healthy-after-failure"
            );
            drop(client);
            tasks.stop().await;
            wait_for_released_tunnels(&state).await;
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn shaped_websocket_messages_preserve_generated_rpc_fields_and_terminal_ownership() {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let (admission, _incoming) =
                NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
            let (address, state, mut tasks) = incoming_listener(admission).await;
            let sample = |sequence| Sample {
                sequence,
                payload: b"byte-shape".to_vec(),
                behavior: Behavior::Echo as i32,
                ..Sample::default()
            };
            for (mode, id, path) in [
                ("split", 1, "/byte-shape/split/1"),
                ("coalesced", 2, "/byte-shape/coalesced/2"),
            ] {
                let mut client = generated_client_over_websocket(
                    address,
                    state.origin.clone(),
                    path,
                    &mut tasks,
                )
                .await;
                let reply = client.unary(Request::new(sample(43))).await.unwrap();
                assert_eq!(
                    reply.metadata().get("fixture-server").unwrap(),
                    "native-tonic"
                );
                assert_eq!(reply.into_inner(), sample(43));
                let mut stream = client
                    .server_stream(Request::new(sample(0)))
                    .await
                    .unwrap()
                    .into_inner();
                for sequence in 0..3 {
                    assert_eq!(stream.message().await.unwrap(), Some(sample(sequence)));
                }
                assert!(stream.message().await.unwrap().is_none());
                assert_eq!(
                    stream
                        .trailers()
                        .await
                        .unwrap()
                        .unwrap()
                        .get("fixture-terminal")
                        .unwrap(),
                    "observed"
                );
                drop(stream);
                drop(client);
                let proxy = tasks.0.pop().expect("owned test proxy missing");
                tokio::time::timeout(std::time::Duration::from_secs(2), proxy)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        let witness = state
                            .shapes
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|w| w.id == id)
                            .cloned();
                        if let Some(witness) = witness.filter(|w| w.closed) {
                            assert_eq!(witness.mode, mode);
                            assert!(!witness.failed);
                            assert_eq!(witness.domain_calls, 2);
                            if mode == "split" {
                                assert!(witness.messages >= 9);
                                assert_eq!(witness.frames, 1);
                            } else {
                                assert_eq!(witness.messages, 1);
                                assert_eq!(witness.frames, 2);
                            }
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
            }
            let (mut socket, _) = tokio_tungstenite::connect_async(tunnel_request(
                address,
                &state.origin,
                "/byte-shape/text/3",
            ))
            .await
            .unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Binary(
                    b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec().into(),
                ))
                .await
                .unwrap();
            let message = socket.next().await.unwrap().unwrap();
            assert!(matches!(
                message,
                tokio_tungstenite::tungstenite::Message::Text(_)
            ));
            socket.close(None).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let witness = state
                        .shapes
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|w| w.id == 3)
                        .cloned();
                    if let Some(witness) = witness.filter(|w| w.closed) {
                        assert_eq!(witness.messages, 1);
                        assert_eq!(witness.domain_calls, 0);
                        assert!(!witness.failed);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            tasks.stop().await;
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn pending_loss_waits_for_completed_trigger_with_retained_pending_bidi() {
        use std::future::Future;
        tokio::time::timeout(std::time::Duration::from_secs(6), async {
            let (admission, _incoming) =
                NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
            let (address, state, mut tasks) = incoming_listener(admission).await;
            let mut client = generated_client_over_websocket(
                address,
                state.origin.clone(),
                "/byte-shape/pending-loss/5",
                &mut tasks,
            )
            .await;
            let sample = |sequence| Sample {
                sequence,
                behavior: Behavior::Echo as i32,
                ..Sample::default()
            };
            let (pending, observed_pending) = futures::channel::oneshot::channel();
            let mut pending = Some(pending);
            let mut first = Some(sample(77));
            let input = futures::stream::poll_fn(move |_| {
                if let Some(first) = first.take() {
                    return std::task::Poll::Ready(Some(first));
                }
                if let Some(pending) = pending.take() {
                    let _ = pending.send(());
                }
                std::task::Poll::Pending
            });
            let mut response = client.bidi(Request::new(input)).await.unwrap().into_inner();
            assert_eq!(response.message().await.unwrap(), Some(sample(77)));
            observed_pending.await.unwrap();
            let mut reader = Box::pin(response.message());
            assert!(
                futures::future::poll_fn(|context| {
                    std::task::Poll::Ready(reader.as_mut().poll(context).is_pending())
                })
                .await
            );
            assert!(
                state
                    .shapes
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|witness| witness.id != 5),
                "pending peer closed before a native RPC completed"
            );
            let mut trigger = client.clone();
            // Its handler completion arms peer loss; its response may be lost.
            let _ = trigger.unary(Request::new(sample(78))).await;
            let terminal = reader.await.expect_err("peer loss fabricated normal End");
            assert!(terminal.metadata().get("fixture-terminal").is_none());
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let witness = state
                        .shapes
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|witness| witness.id == 5)
                        .cloned();
                    if let Some(witness) = witness.filter(|witness| witness.closed) {
                        assert_eq!(witness.mode, "pending-loss");
                        assert_eq!(witness.domain_calls, 1);
                        assert!(!witness.failed);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            drop(response);
            drop(client);
            drop(trigger);
            let proxy = tasks.0.pop().expect("owned pending-loss proxy missing");
            tokio::time::timeout(std::time::Duration::from_secs(2), proxy)
                .await
                .expect("pending-loss proxy close bound")
                .expect("pending-loss proxy join")
                .expect_err("peer loss must terminate its actual proxy");
            tasks.stop().await;
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn forced_websocket_loss_after_generated_wait_has_no_domain_terminal_or_replay() {
        tokio::time::timeout(std::time::Duration::from_secs(6), async {
            let (admission, incoming) =
                NativeIncoming::bounded(NonZeroUsize::new(1).unwrap()).unwrap();
            let (address, state, mut tasks) = incoming_listener(admission).await;
            start_generated_server(incoming, &state, &mut tasks);
            let mut lost = generated_client_over_websocket(
                address,
                state.origin.clone(),
                "/byte-shape/loss/4",
                &mut tasks,
            )
            .await;
            let transport = lost
                .unary(Request::new(Sample {
                    sequence: 62,
                    behavior: Behavior::Wait as i32,
                    ..Sample::default()
                }))
                .await
                .unwrap_err();
            assert_ne!(transport.code(), Code::InvalidArgument);
            assert!(transport.metadata().get("fixture-terminal").is_none());
            drop(lost);
            let proxy = tasks.0.pop().expect("owned forced-loss proxy missing");
            let proxy_error = tokio::time::timeout(std::time::Duration::from_secs(2), proxy)
                .await
                .expect("forced-loss proxy close bound")
                .expect("forced-loss proxy join")
                .expect_err("forced WebSocket loss must terminate its proxy");
            assert!(
                proxy_error
                    .get_ref()
                    .is_some_and(|source| source.is::<tokio_tungstenite::tungstenite::Error>())
            );
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let witness = state
                        .shapes
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|witness| witness.id == 4)
                        .cloned();
                    if let Some(witness) = witness.filter(|witness| witness.closed) {
                        assert_eq!(witness.mode, "loss");
                        assert_eq!(witness.domain_calls, 1);
                        assert_eq!(witness.messages, 1);
                        assert_eq!(witness.frames, 1);
                        assert!(!witness.failed);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            let mut fresh = generated_client_over_websocket(
                address,
                state.origin.clone(),
                "/tunnel",
                &mut tasks,
            )
            .await;
            let reply = fresh
                .unary(Request::new(Sample {
                    sequence: 64,
                    behavior: Behavior::Echo as i32,
                    ..Sample::default()
                }))
                .await
                .unwrap();
            assert_eq!(reply.into_inner().sequence, 64);
            assert_eq!(
                state
                    .shapes
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|witness| witness.id == 4)
                    .unwrap()
                    .domain_calls,
                1
            );
            drop(fresh);
            tasks.stop().await;
            wait_for_released_tunnels(&state).await;
        })
        .await
        .unwrap();
    }
}
