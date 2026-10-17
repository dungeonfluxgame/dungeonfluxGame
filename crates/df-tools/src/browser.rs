use df_observe::OperationContext;
use df_protocol::transport_fixture::{
    Sample, sample::Behavior, transport_fixture_client::TransportFixtureClient,
};
use df_rpc_bridge::{BrowserChannel, BrowserConnection, ConnectionSnapshot, RPC_MESSAGE_BYTES};
use futures::{
    FutureExt, SinkExt, Stream,
    channel::{mpsc, oneshot},
    future::{Either, select},
};
use gloo_timers::future::TimeoutFuture;
use std::{
    cell::Cell,
    error::Error,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
use tonic::{Code, Request, Status};
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{Document, HtmlButtonElement};

pub(super) type FixtureClient = TransportFixtureClient<BrowserChannel>;
pub(super) fn sample(sequence: u32) -> Sample {
    Sample {
        sequence,
        payload: b"synthetic protobuf".to_vec(),
        behavior: Behavior::Echo as i32,
        ..Sample::default()
    }
}
pub(super) fn request<T>(body: T) -> Result<Request<T>, String> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "traceparent",
        "00-11111111111111111111111111111111-2222222222222222-01"
            .parse()
            .map_err(|error: tonic::metadata::errors::InvalidMetadataValue| error.to_string())?,
    );
    Ok(request)
}
pub(super) fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}
fn show(document: &Document, text: &str) {
    if let Some(element) = document.get_element_by_id("results") {
        element.set_text_content(Some(text));
    }
}
fn append(document: &Document, lines: &mut String, line: &str) {
    lines.push_str(line);
    lines.push('\n');
    show(document, lines);
}
async fn finish(mut stream: tonic::Streaming<Sample>, expected: &[u32]) -> Result<(), String> {
    let mut sequences = vec![];
    while let Some(sample) = stream.message().await.map_err(|error| error.to_string())? {
        sequences.push(sample.sequence);
    }
    require(sequences == expected, "stream order or count mismatch")?;
    let trailers = stream
        .trailers()
        .await
        .map_err(|error| error.to_string())?
        .ok_or("terminal trailers missing")?;
    require(
        trailers
            .get("fixture-terminal")
            .is_some_and(|value| value == "observed"),
        "custom terminal trailer missing",
    )
}
async fn run(document: Document) -> Result<(), String> {
    let mut lines = String::new();
    append(
        &document,
        &mut lines,
        "CONNECTING · binary WebSocket / native HTTP/2",
    );
    let (connection, channel) = BrowserConnection::connect(&tunnel_url("/tunnel")?)
        .await
        .map_err(|error| error.to_string())?;
    let mut client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    crate::generated_browser_bindings::verify(&mut client).await?;
    append(
        &document,
        &mut lines,
        "PASS · Generated Rust browser bindings: exact protobuf fields in four modes, metadata/status/trailers/EOF, half-close/cancel and client message bounds",
    );
    let reply = client
        .unary(request(sample(7))?)
        .await
        .map_err(|error| error.to_string())?;
    require(
        reply
            .metadata()
            .get("fixture-server")
            .is_some_and(|value| value == "native-tonic"),
        "native response metadata missing",
    )?;
    require(reply.into_inner().sequence == 7, "unary mismatch")?;
    append(&document, &mut lines, "PASS · Unary + response metadata");
    let stream = client
        .server_stream(request(sample(0))?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    finish(stream, &[0, 1, 2]).await?;
    append(
        &document,
        &mut lines,
        "PASS · Server streaming + ordered DATA + OK trailers",
    );
    let response = client
        .client_stream(request(futures::stream::iter([
            sample(0),
            sample(1),
            sample(2),
        ]))?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    require(
        response.sequence == 3 && response.payload == b"half-close observed",
        "client half-close not observed",
    )?;
    append(
        &document,
        &mut lines,
        "PASS · Client streaming + request half-close",
    );
    let (mut sender, receiver) = mpsc::channel(1);
    sender
        .send(sample(10))
        .await
        .map_err(|error| error.to_string())?;
    let mut bidi = client
        .bidi(request(receiver)?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    require(
        bidi.message()
            .await
            .map_err(|error| error.to_string())?
            .is_some_and(|sample| sample.sequence == 10),
        "bidi did not respond before half-close",
    )?;
    sender
        .send(sample(11))
        .await
        .map_err(|error| error.to_string())?;
    drop(sender);
    finish(bidi, &[11]).await?;
    append(
        &document,
        &mut lines,
        "PASS · Bidirectional streaming + response before half-close",
    );
    let reject = Sample {
        behavior: Behavior::Reject as i32,
        ..sample(0)
    };
    let status = client
        .unary(request(reject)?)
        .await
        .err()
        .ok_or("rejection was accepted")?;
    require(
        status.code() == Code::InvalidArgument
            && status.metadata().get("fixture-terminal").is_some(),
        "error status/trailer mismatch",
    )?;
    append(
        &document,
        &mut lines,
        "PASS · INVALID_ARGUMENT + error trailers",
    );
    rejection_modes(&mut client).await?;
    append(
        &document,
        &mut lines,
        "PASS · Error statuses + trailers in all four RPC modes",
    );
    let waiting = Sample {
        behavior: Behavior::Wait as i32,
        ..sample(0)
    };
    let baseline_stats = client
        .unary(request(Sample {
            payload: b"stats".to_vec(),
            ..sample(0)
        })?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    let baseline = baseline_stats.sequence;
    let mut stream = client
        .server_stream(request(waiting.clone())?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    require(
        stream
            .message()
            .await
            .map_err(|error| error.to_string())?
            .is_some(),
        "cancellable stream never started",
    )?;
    drop(stream);
    wait_for_cancellation(&mut client, baseline + 1).await?;
    append(
        &document,
        &mut lines,
        "PASS · Cancellation resets stream + server owner released",
    );
    let mut deadline_request = request(waiting)?;
    deadline_request.set_timeout(std::time::Duration::from_millis(40));
    let (deadline_status, deadline_source) = match select(
        client.unary(deadline_request).boxed_local(),
        TimeoutFuture::new(40).boxed_local(),
    )
    .await
    {
        Either::Left((result, _)) => (
            result.err().ok_or("waiting RPC unexpectedly completed")?,
            "native terminal status",
        ),
        Either::Right(((), rpc)) => {
            drop(rpc);
            (
                Status::deadline_exceeded("browser monotonic deadline elapsed"),
                "local browser deadline",
            )
        }
    };
    require(
        matches!(
            deadline_status.code(),
            Code::DeadlineExceeded | Code::Cancelled
        ),
        "deadline not enforced",
    )?;
    wait_for_cancellation(&mut client, baseline + 2).await?;
    let deadline_stats = client
        .unary(request(Sample {
            payload: b"stats".to_vec(),
            ..sample(0)
        })?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    require(
        deadline_stats.deadline_calls > baseline_stats.deadline_calls,
        "native grpc-timeout metadata was not observed",
    )?;
    append(
        &document,
        &mut lines,
        &format!(
            "PASS · Deadline ({deadline_source}: {:?}) + native grpc-timeout + cleanup",
            deadline_status.code()
        ),
    );
    let oversized = Sample {
        payload: vec![0; RPC_MESSAGE_BYTES + 1],
        ..sample(0)
    };
    let status = client
        .unary(request(oversized)?)
        .await
        .err()
        .ok_or("oversized message was accepted")?;
    require(
        status.code() == Code::OutOfRange || status.code() == Code::ResourceExhausted,
        "bounded rejection status mismatch",
    )?;
    append(
        &document,
        &mut lines,
        "PASS · Oversized protobuf rejected at native decoding boundary",
    );
    let mut other_client = client.clone();
    let stream = client
        .server_stream(request(sample(0))?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    let unary = other_client.unary(request(sample(99))?);
    let (stream_result, unary_result) = futures::join!(finish(stream, &[0, 1, 2]), unary);
    stream_result?;
    require(
        unary_result
            .map_err(|error| error.to_string())?
            .into_inner()
            .sequence
            == 99,
        "simultaneous unary failed",
    )?;
    append(
        &document,
        &mut lines,
        "PASS · Simultaneous streams share the same connection",
    );
    byte_shape_checks(&document, &mut lines).await?;
    let wire = connection.snapshot().map_err(|error| error.to_string())?;
    connection.close();
    let context = OperationContext {
        trace_parent: "00-11111111111111111111111111111111-2222222222222222-01".to_owned(),
        build: crate::BUILD_ID.to_owned(),
    };
    df_observe::record(
        &context,
        "browser.fixture",
        "complete",
        wire.received_bytes + wire.sent_bytes,
    );
    append(
        &document,
        &mut lines,
        "COMPLETE · Generated Rust browser binding and transport checks passed\nG02 INCONCLUSIVE · physical devices / adversarial receive memory pending",
    );
    crate::qualification::save_report("semantics", &format!("Build: {}\n{lines}", crate::BUILD_ID))
        .await?;
    Ok(())
}
async fn shape_report(id: u64) -> Result<String, String> {
    let window = web_sys::window().ok_or("window unavailable")?;
    for _ in 0..50 {
        let response = wasm_bindgen_futures::JsFuture::from(
            window.fetch_with_str(&format!("/byte-shape-report/{id}")),
        )
        .await
        .map_err(|_| "byte shape report request failed")?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "byte shape report response unavailable")?;
        if response.status() == 200 {
            let text = wasm_bindgen_futures::JsFuture::from(
                response
                    .text()
                    .map_err(|_| "byte shape report was not text")?,
            )
            .await
            .map_err(|_| "byte shape report text failed")?
            .as_string()
            .ok_or("byte shape report text missing")?;
            if text.contains("closed=true") {
                return Ok(text);
            }
        }
        TimeoutFuture::new(10).await;
    }
    Err("byte shape endpoint did not reach terminal cleanup".to_owned())
}
fn show_close(document: &Document, report: &str) {
    if let Some(element) = document.get_element_by_id("close-reconnect-report") {
        element.set_text_content(Some(report));
    }
}
fn append_close(document: &Document, lines: &mut String, line: &str) {
    lines.push_str(line);
    lines.push('\n');
    show_close(document, lines);
}
async fn closed_browser(connection: &BrowserConnection) -> Result<ConnectionSnapshot, String> {
    for _ in 0..100 {
        let snapshot = connection.snapshot().map_err(|error| error.to_string())?;
        if snapshot.closed
            && snapshot.callback_bytes.current == 0
            && snapshot.callback_items.current == 0
            && snapshot.browser_callback_vec_capacity.current == 0
        {
            return Ok(snapshot);
        }
        TimeoutFuture::new(5).await;
    }
    Err("browser connection owners did not close within 500ms".to_owned())
}
struct PendingInput {
    first: Option<Sample>,
    pending: Option<oneshot::Sender<()>>,
    dropped: Option<oneshot::Sender<()>>,
    drop_count: Arc<AtomicUsize>,
}
impl Stream for PendingInput {
    type Item = Sample;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Sample>> {
        if let Some(first) = self.first.take() {
            return Poll::Ready(Some(first));
        }
        if let Some(pending) = self.pending.take() {
            let _ = pending.send(());
        }
        Poll::Pending
    }
}
impl Drop for PendingInput {
    fn drop(&mut self) {
        self.drop_count.fetch_add(1, Ordering::SeqCst);
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}
fn pending_input(
    sequence: u32,
) -> (
    PendingInput,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    Arc<AtomicUsize>,
) {
    let (pending, observed_pending) = oneshot::channel();
    let (dropped, observed_drop) = oneshot::channel();
    let drop_count = Arc::new(AtomicUsize::new(0));
    (
        PendingInput {
            first: Some(sample(sequence)),
            pending: Some(pending),
            dropped: Some(dropped),
            drop_count: drop_count.clone(),
        },
        observed_pending,
        observed_drop,
        drop_count,
    )
}
async fn close_bound<T>(future: impl Future<Output = T>, label: &str) -> Result<T, String> {
    match select(future.boxed_local(), TimeoutFuture::new(500).boxed_local()).await {
        Either::Left((result, _)) => Ok(result),
        Either::Right(((), pending)) => {
            drop(pending);
            Err(format!("{label} exceeded owned 500ms observation bound"))
        }
    }
}
async fn observed_input_drop(
    dropped: oneshot::Receiver<()>,
    count: &AtomicUsize,
) -> Result<(), String> {
    close_bound(dropped, "pending input Drop")
        .await?
        .map_err(|_| "pending input Drop witness lost")?;
    require(
        count.load(Ordering::SeqCst) == 1,
        "pending input was not dropped exactly once",
    )
}
async fn owned_close_checks(document: &Document, lines: &mut String) -> Result<(), String> {
    let (connection, channel) = BrowserConnection::connect(&tunnel_url("/tunnel")?)
        .await
        .map_err(|error| error.to_string())?;
    let mut client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let (input, pending, dropped, count) = pending_input(71);
    let mut response = client
        .bidi(request(input)?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    require(
        close_bound(response.message(), "first bidi echo")
            .await?
            .map_err(|error| error.to_string())?
            == Some(sample(71)),
        "owned close bidi first echo mismatch",
    )?;
    close_bound(pending, "actual pending input poll")
        .await?
        .map_err(|_| "pending input poll witness lost")?;
    let mut reader = response.message().boxed_local();
    let pending_reader =
        futures::future::poll_fn(|context| Poll::Ready(reader.as_mut().poll(context).is_pending()))
            .await;
    require(
        pending_reader,
        "response reader was not Pending before close",
    )?;
    require(
        count.load(Ordering::SeqCst) == 0,
        "input dropped before close",
    )?;
    connection.close();
    connection.close();
    let terminal = close_bound(reader, "retained response reader termination")
        .await?
        .err()
        .ok_or("closed retained reader fabricated normal End")?;
    require(
        terminal.metadata().get("fixture-terminal").is_none(),
        "connection cancellation fabricated authoritative domain trailers",
    )?;
    // Keep the generated response alive until the independently pending input is gone.
    observed_input_drop(dropped, &count).await?;
    let snapshot = closed_browser(&connection).await?;
    require(
        snapshot.websocket_receive.current == 0 && snapshot.websocket_receive_items.current == 0,
        "closed connection retained callback receipt owners",
    )?;
    append_close(
        document,
        lines,
        "PASS · Owned close: input_pending=true reader_pending=true repeated_close=true input_drop_before_response_drop=1 reader_terminal=error callback_owners=0",
    );
    drop(response);
    drop(client);

    let (connection, channel) = BrowserConnection::connect(&tunnel_url("/tunnel")?)
        .await
        .map_err(|error| error.to_string())?;
    let mut client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let (input, pending, dropped, count) = pending_input(72);
    let mut response = client
        .bidi(request(input)?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    require(
        close_bound(response.message(), "response Drop first echo")
            .await?
            .map_err(|error| error.to_string())?
            == Some(sample(72)),
        "response Drop first echo mismatch",
    )?;
    close_bound(pending, "response Drop pending input")
        .await?
        .map_err(|_| "response Drop pending witness lost")?;
    drop(response);
    observed_input_drop(dropped, &count).await?;
    require(
        client
            .unary(request(sample(73))?)
            .await
            .map_err(|error| error.to_string())?
            .into_inner()
            == sample(73),
        "per-response cancellation affected current connection",
    )?;
    require(
        !connection
            .snapshot()
            .map_err(|error| error.to_string())?
            .closed,
        "response Drop closed unrelated connection owner",
    )?;
    connection.close();
    closed_browser(&connection).await?;
    append_close(
        document,
        lines,
        "PASS · Owned response Drop: input_pending=true input_drop=1 current_unary=preserved",
    );

    let (connection, channel) = BrowserConnection::connect(&tunnel_url("/tunnel")?)
        .await
        .map_err(|error| error.to_string())?;
    let mut client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let (input, pending, dropped, count) = pending_input(74);
    let mut call = client.client_stream(request(input)?).boxed_local();
    let pending_call =
        futures::future::poll_fn(|context| Poll::Ready(call.as_mut().poll(context).is_pending()))
            .await;
    require(
        pending_call,
        "client-stream call did not block before response headers",
    )?;
    match select(
        call,
        close_bound(pending, "pre-response pending input").boxed_local(),
    )
    .await
    {
        Either::Left((_result, _)) => {
            return Err("client-stream call completed before pending input witness".to_owned());
        }
        Either::Right((observed, call)) => {
            observed?.map_err(|_| "pre-response input witness lost")?;
            drop(call);
        }
    }
    observed_input_drop(dropped, &count).await?;
    require(
        client
            .unary(request(sample(75))?)
            .await
            .map_err(|error| error.to_string())?
            .into_inner()
            == sample(75),
        "cancelled pre-response call affected current connection",
    )?;
    connection.close();
    closed_browser(&connection).await?;
    append_close(
        document,
        lines,
        "PASS · Owned call Drop: response_pending=true input_pending=true input_drop=1 current_unary=preserved",
    );

    let owner_drop_id = js_sys::Date::now() as u64;
    let (connection, channel) = BrowserConnection::connect(&tunnel_url(&format!(
        "/byte-shape/coalesced/{owner_drop_id}"
    ))?)
    .await
    .map_err(|error| error.to_string())?;
    let mut client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let (input, pending, dropped, count) = pending_input(76);
    let mut response = client
        .bidi(request(input)?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    require(
        close_bound(response.message(), "owner Drop first echo")
            .await?
            .map_err(|error| error.to_string())?
            == Some(sample(76)),
        "owner Drop first echo mismatch",
    )?;
    close_bound(pending, "owner Drop pending input")
        .await?
        .map_err(|_| "owner Drop pending witness lost")?;
    let mut reader = response.message().boxed_local();
    require(
        futures::future::poll_fn(|context| Poll::Ready(reader.as_mut().poll(context).is_pending()))
            .await,
        "retained reader was not Pending before owner Drop",
    )?;
    require(
        count.load(Ordering::SeqCst) == 0,
        "input dropped before owner Drop",
    )?;
    drop(connection);
    // No response poll or Drop may supply the cancellation being witnessed here.
    observed_input_drop(dropped, &count).await?;
    let terminal = close_bound(reader, "owner Drop retained reader termination")
        .await?
        .err()
        .ok_or("owner Drop fabricated normal End")?;
    require(
        terminal.metadata().get("fixture-terminal").is_none(),
        "owner Drop fabricated authoritative domain trailers",
    )?;
    let witness = shape_report(owner_drop_id).await?;
    require(
        witness.contains("mode=coalesced")
            && witness.contains("closed=true")
            && witness.contains("failed=false")
            && witness.contains(&format!("build={}", crate::BUILD_ID)),
        "owner Drop lacked the actual owned peer terminal witness",
    )?;
    append_close(
        document,
        lines,
        &format!(
            "PASS · Owned connection Drop: input_pending=true reader_pending=true input_drop_before_response_poll=1 retained_reader_terminal=error peer_closed=true {witness}"
        ),
    );
    drop(response);
    drop(client);

    let natural_end_id = owner_drop_id + 1;
    let (connection, channel) = BrowserConnection::connect(&tunnel_url(&format!(
        "/byte-shape/pending-loss/{natural_end_id}"
    ))?)
    .await
    .map_err(|error| error.to_string())?;
    let mut client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let (input, pending, dropped, count) = pending_input(77);
    let mut response = client
        .bidi(request(input)?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    require(
        close_bound(response.message(), "natural end first echo")
            .await?
            .map_err(|error| error.to_string())?
            == Some(sample(77)),
        "natural end first echo mismatch",
    )?;
    close_bound(pending, "natural end pending input")
        .await?
        .map_err(|_| "natural end pending witness lost")?;
    let mut reader = response.message().boxed_local();
    require(
        futures::future::poll_fn(|context| Poll::Ready(reader.as_mut().poll(context).is_pending()))
            .await,
        "retained reader was not Pending before natural end trigger",
    )?;
    require(
        count.load(Ordering::SeqCst) == 0,
        "input dropped before natural end trigger",
    )?;
    let mut trigger_client = client.clone();
    let trigger = trigger_client.unary(request(sample(78))?).boxed_local();
    let input_drop = observed_input_drop(dropped, &count).boxed_local();
    // The completed native handler arms peer loss; its reply can be lost legitimately.
    // Drive that real RPC without polling the retained response or cancelling its owner.
    match select(
        input_drop,
        close_bound(trigger, "natural end native trigger").boxed_local(),
    )
    .await
    {
        Either::Left((observed, trigger)) => {
            observed?;
            drop(trigger);
        }
        Either::Right((trigger_result, observed)) => {
            let _ = trigger_result?;
            observed.await?;
        }
    }
    let terminal = close_bound(reader, "natural end retained reader termination")
        .await?
        .err()
        .ok_or("natural driver end fabricated normal End")?;
    require(
        terminal.metadata().get("fixture-terminal").is_none(),
        "natural driver end fabricated authoritative domain trailers",
    )?;
    let snapshot = closed_browser(&connection).await?;
    require(
        snapshot.websocket_receive.current == 0 && snapshot.websocket_receive_items.current == 0,
        "natural driver end retained callback receipt owners",
    )?;
    let witness = shape_report(natural_end_id).await?;
    require(
        witness.contains("mode=pending-loss")
            && witness.contains("domain_calls=1")
            && witness.contains("closed=true")
            && witness.contains("failed=false")
            && witness.contains(&format!("build={}", crate::BUILD_ID)),
        "natural driver end lacked its actual completed native trigger and peer cleanup",
    )?;
    append_close(
        document,
        lines,
        &format!(
            "PASS · Natural driver end: input_pending=true reader_pending=true input_drop_before_response_poll=1 retained_reader_terminal=error explicit_close=false callback_owners=0 completed_native_trigger=1 {witness}"
        ),
    );
    drop(response);
    drop(client);
    drop(trigger_client);
    drop(connection);

    Ok(())
}

async fn run_close_reconnect(document: Document) -> Result<(), String> {
    let mut lines = String::new();
    append_close(
        &document,
        &mut lines,
        "CONNECTING · close/drain/reconnect contract",
    );

    let (idle, _) = BrowserConnection::connect(&tunnel_url("/tunnel")?)
        .await
        .map_err(|error| error.to_string())?;
    idle.close();
    let idle_snapshot = closed_browser(&idle).await?;
    require(
        !idle_snapshot.driver_failed,
        "idle close failed the browser driver",
    )?;
    append_close(
        &document,
        &mut lines,
        "PASS · Idle close released browser callback owners",
    );

    owned_close_checks(&document, &mut lines).await?;

    let (drained, channel) = BrowserConnection::connect(&tunnel_url("/tunnel")?)
        .await
        .map_err(|error| error.to_string())?;
    let mut client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let rejected = Sample {
        behavior: Behavior::Reject as i32,
        ..sample(60)
    };
    let domain = client
        .unary(request(rejected)?)
        .await
        .err()
        .ok_or("domain refusal unexpectedly succeeded")?;
    require(
        domain.code() == Code::InvalidArgument
            && domain.message() == "synthetic terminal"
            && domain
                .metadata()
                .get("fixture-terminal")
                .is_some_and(|value| value == "observed")
            && domain.source().is_none(),
        "authoritative domain status/trailer/source mismatch",
    )?;
    append_close(
        &document,
        &mut lines,
        "PASS · Domain refusal: InvalidArgument, exact terminal trailer, no transport source",
    );
    let response = client
        .unary(request(sample(61))?)
        .await
        .map_err(|error| error.to_string())?;
    require(
        response.into_inner() == sample(61),
        "drained unary mismatch",
    )?;
    finish(
        client
            .server_stream(request(sample(0))?)
            .await
            .map_err(|error| error.to_string())?
            .into_inner(),
        &[0, 1, 2],
    )
    .await?;
    drop(client);
    drained.close();
    let drained_snapshot = closed_browser(&drained).await?;
    require(
        !drained_snapshot.driver_failed && drained_snapshot.received_frames > 0,
        "after-terminal close lost the drained response",
    )?;
    append_close(
        &document,
        &mut lines,
        "PASS · Close after observed terminal trailers preserved authority and released owners",
    );

    let id = js_sys::Date::now() as u64;
    let (lost, channel) =
        BrowserConnection::connect(&tunnel_url(&format!("/byte-shape/loss/{id}"))?)
            .await
            .map_err(|error| error.to_string())?;
    let mut old_client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let transport = old_client
        .unary(request(Sample {
            behavior: Behavior::Wait as i32,
            ..sample(62)
        })?)
        .await
        .err()
        .ok_or("forced WebSocket loss produced a domain success")?;
    require(
        transport.source().is_some()
            && transport.metadata().get("fixture-terminal").is_none()
            && transport.code() != Code::InvalidArgument,
        "forced WebSocket loss was mistaken for authoritative domain refusal",
    )?;
    lost.close();
    let lost_snapshot = closed_browser(&lost).await?;
    let witness = shape_report(id).await?;
    require(
        witness.contains("mode=loss")
            && witness.contains("domain_calls=1")
            && witness.contains("closed=true")
            && witness.contains("failed=false")
            && witness.contains(&format!("build={}", crate::BUILD_ID)),
        "forced loss lacks actual preterminal dispatch and peer close witness",
    )?;
    append_close(
        &document,
        &mut lines,
        &format!(
            "PASS · Preterminal WS loss: transport source present, no domain trailer, code={:?}, old_driver_failed={}, {witness}",
            transport.code(),
            lost_snapshot.driver_failed
        ),
    );
    let stale = old_client
        .unary(request(sample(63))?)
        .await
        .err()
        .ok_or("closed old generation accepted a new RPC")?;
    require(
        stale.metadata().get("fixture-terminal").is_none(),
        "closed old generation acquired an authoritative status",
    )?;
    let old_snapshot = lost.snapshot().map_err(|error| error.to_string())?;

    let (fresh, channel) = BrowserConnection::connect(&tunnel_url("/tunnel")?)
        .await
        .map_err(|error| error.to_string())?;
    let mut fresh_client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let reply = fresh_client
        .unary(request(sample(64))?)
        .await
        .map_err(|error| error.to_string())?;
    require(
        reply
            .metadata()
            .get("fixture-server")
            .is_some_and(|value| value == "native-tonic")
            && reply.into_inner() == sample(64),
        "fresh connection did not deliver exact generated RPC",
    )?;
    require(
        lost.snapshot()
            .map_err(|error| error.to_string())?
            .received_bytes
            == old_snapshot.received_bytes
            && shape_report(id).await?.contains("domain_calls=1"),
        "old generation received late bytes or replayed the pending operation",
    )?;
    drop(fresh_client);
    fresh.close();
    closed_browser(&fresh).await?;
    append_close(
        &document,
        &mut lines,
        "PASS · Fresh connection served new RPC; old generation stayed closed without replay",
    );
    append_close(
        &document,
        &mut lines,
        "COMPLETE · Close/reconnect contract passed; preterminal operation outcome UNKNOWN, external operation-ID resync unimplemented",
    );
    crate::qualification::save_report(
        "close-reconnect",
        &format!("Build: {}\n{lines}", crate::BUILD_ID),
    )
    .await?;
    Ok(())
}
async fn shaped_mode(mode: &str, id: u64) -> Result<String, String> {
    let (connection, channel) =
        BrowserConnection::connect(&tunnel_url(&format!("/byte-shape/{mode}/{id}"))?)
            .await
            .map_err(|error| format!("{mode} browser connection: {error}"))?;
    let mut client = FixtureClient::new(channel)
        .max_decoding_message_size(RPC_MESSAGE_BYTES)
        .max_encoding_message_size(RPC_MESSAGE_BYTES * 2);
    let reply = client
        .unary(request(sample(43))?)
        .await
        .map_err(|error| format!("{mode} unary: {error}"))?;
    require(
        reply
            .metadata()
            .get("fixture-server")
            .is_some_and(|value| value == "native-tonic")
            && reply.into_inner() == sample(43),
        "shaped unary metadata or exact protobuf fields mismatch",
    )?;
    let mut stream = client
        .server_stream(request(sample(0))?)
        .await
        .map_err(|error| format!("{mode} server stream: {error}"))?
        .into_inner();
    for sequence in 0..3 {
        require(
            stream.message().await.map_err(|error| error.to_string())? == Some(sample(sequence)),
            "shaped server stream fields/order mismatch",
        )?;
    }
    require(
        stream
            .message()
            .await
            .map_err(|error| error.to_string())?
            .is_none()
            && stream
                .trailers()
                .await
                .map_err(|error| error.to_string())?
                .is_some_and(|trailers| {
                    trailers
                        .get("fixture-terminal")
                        .is_some_and(|value| value == "observed")
                }),
        "shaped server stream EOF/trailer mismatch",
    )?;
    let response = client
        .client_stream(request(futures::stream::iter([sample(1), sample(2)]))?)
        .await
        .map_err(|error| format!("{mode} client stream: {error}"))?
        .into_inner();
    require(
        response.sequence == 2 && response.payload == b"half-close observed",
        "shaped client half-close mismatch",
    )?;
    let mut bidi = client
        .bidi(request(futures::stream::iter([sample(3), sample(4)]))?)
        .await
        .map_err(|error| format!("{mode} bidi: {error}"))?
        .into_inner();
    require(
        bidi.message().await.map_err(|error| error.to_string())? == Some(sample(3))
            && bidi.message().await.map_err(|error| error.to_string())? == Some(sample(4))
            && bidi
                .message()
                .await
                .map_err(|error| error.to_string())?
                .is_none()
            && bidi
                .trailers()
                .await
                .map_err(|error| error.to_string())?
                .is_some_and(|trailers| {
                    trailers
                        .get("fixture-terminal")
                        .is_some_and(|value| value == "observed")
                }),
        "shaped bidi fields/order/EOF/trailer mismatch",
    )?;
    drop(client);
    connection.close();
    let snapshot = connection.snapshot().map_err(|error| error.to_string())?;
    require(
        snapshot.received_frames > 0 && snapshot.received_bytes > 0,
        "shaped browser adapter did not receive HTTP2 bytes",
    )?;
    let report = shape_report(id).await?;
    require(
        report.contains(&format!("mode={mode}"))
            && report.contains("domain_calls=4")
            && report.contains("closed=true")
            && report.contains("failed=false")
            && report.contains(&format!("build={}", crate::BUILD_ID)),
        "shaped endpoint source, RPC dispatch or cleanup mismatch",
    )?;
    if mode == "split" {
        let messages = report
            .split_whitespace()
            .find_map(|part| part.strip_prefix("messages="))
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or("split message count missing")?;
        require(
            messages >= 9 && report.contains("frames=1"),
            "one HTTP2 frame was not split into many real WebSocket messages",
        )?;
    } else {
        require(
            report.contains("messages=1") && report.contains("frames=2"),
            "two HTTP2 frames were not coalesced into one real WebSocket message",
        )?;
    }
    Ok(report)
}
async fn byte_shape_checks(document: &Document, lines: &mut String) -> Result<(), String> {
    let id = js_sys::Date::now() as u64;
    let split = shaped_mode("split", id).await?;
    append(
        document,
        lines,
        &format!("PASS · Browser HTTP2 frame split across real WS messages: {split}"),
    );
    let coalesced = shaped_mode("coalesced", id + 1).await?;
    append(
        document,
        lines,
        &format!("PASS · Browser receives two HTTP2 frames in one real WS message: {coalesced}"),
    );
    let text_id = id + 2;
    let attempted =
        BrowserConnection::connect(&tunnel_url(&format!("/byte-shape/text/{text_id}"))?).await;
    match attempted {
        Ok((connection, channel)) => {
            let mut client = FixtureClient::new(channel);
            let status = client
                .unary(request(sample(0))?)
                .await
                .err()
                .ok_or("unsupported WS text produced accepted RPC")?;
            require(
                status.code() != Code::InvalidArgument,
                "unsupported WS text was misclassified as a domain rejection",
            )?;
            connection.close();
            TimeoutFuture::new(20).await;
            let snapshot = connection.snapshot().map_err(|error| error.to_string())?;
            require(
                snapshot.rejected && snapshot.closed,
                "browser text rejection did not close owned adapter",
            )?;
        }
        Err(error) => require(
            error.kind() == std::io::ErrorKind::Other
                || error.kind() == std::io::ErrorKind::InvalidData,
            "unsupported WS text did not produce typed transport failure",
        )?,
    }
    let report = shape_report(text_id).await?;
    require(
        report.contains("mode=text")
            && report.contains("messages=1")
            && report.contains("domain_calls=0")
            && report.contains("closed=true")
            && report.contains("failed=false")
            && report.contains(&format!("build={}", crate::BUILD_ID)),
        "unsupported WS text did not produce source-bound refusal and cleanup",
    )?;
    append(
        document,
        lines,
        &format!("PASS · Browser rejects unsupported WS text before domain dispatch: {report}"),
    );
    Ok(())
}
pub(super) async fn wait_for_cancellation(
    client: &mut FixtureClient,
    expected: u32,
) -> Result<(), String> {
    for _ in 0..100 {
        let stats = client
            .unary(request(Sample {
                payload: b"stats".to_vec(),
                ..sample(0)
            })?)
            .await
            .map_err(|error| error.to_string())?
            .into_inner();
        if stats.sequence >= expected && stats.active_calls == 1 {
            return Ok(());
        }
        TimeoutFuture::new(5).await;
    }
    Err("server cancellation cleanup not observed within 500ms".to_owned())
}
#[cfg_attr(not(feature = "public-scene-delivery-fixture"), wasm_bindgen(start))]
#[cfg_attr(feature = "public-scene-delivery-fixture", wasm_bindgen)]
pub fn start() -> Result<(), JsValue> {
    // A wasm panic otherwise reaches the console only as "unreachable", with no message or
    // location; this logs both once, before the abort, so a client panic can be diagnosed.
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&format!("client panic: {info}")));
    }));
    if web_sys::window().is_some_and(|window| {
        window
            .location()
            .pathname()
            .is_ok_and(|path| path.starts_with("/gameplay"))
    }) {
        return crate::gameplay_browser::start();
    }
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("document unavailable"))?;
    let button = document
        .get_element_by_id("run")
        .ok_or_else(|| JsValue::from_str("fixture button unavailable"))?
        .dyn_into::<HtmlButtonElement>()?;
    button.set_disabled(false);
    button.set_text_content(Some("Run transport checks"));
    show(
        &document,
        "READY · Rust/WASM initialized. Run all four RPC modes and failure checks.",
    );
    if let Some(build) = document.get_element_by_id("build") {
        build.set_text_content(Some(&format!("Candidate: S00 transport / G02 qualification · Rust 1.98.1 · tonic 0.14.6 · h2 0.4.19 + browser clock · build {}", crate::BUILD_ID)));
    }
    let active = Rc::new(Cell::new(false));
    crate::qualification::install(&document, active.clone())?;
    install_close_reconnect(&document, active.clone())?;
    let callback_button = button.clone();
    let callback = Closure::wrap(Box::new(move || {
        if active.replace(true) {
            return;
        }
        callback_button.set_disabled(true);
        let document = document.clone();
        let button = callback_button.clone();
        let active = active.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let timed = select(
                run(document.clone()).boxed_local(),
                TimeoutFuture::new(15000).boxed_local(),
            )
            .await;
            let result = match timed {
                Either::Left((result, _)) => result,
                Either::Right(((), future)) => {
                    drop(future);
                    Err("fixture exceeded its 15-second run deadline".to_owned())
                }
            };
            if let Err(error) = result {
                show(
                    &document,
                    &format!("FAIL · {error}\nG02 INCONCLUSIVE · fixture did not complete"),
                );
            }
            active.set(false);
            button.set_disabled(false);
            button.set_text_content(Some("Run transport checks again"));
        });
    }) as Box<dyn FnMut()>);
    button.set_onclick(Some(callback.as_ref().unchecked_ref()));
    // Page-lifetime owner: browser drops the document and WASM instance together on navigation.
    callback.forget();
    Ok(())
}

fn install_close_reconnect(document: &Document, active: Rc<Cell<bool>>) -> Result<(), JsValue> {
    let button = document
        .get_element_by_id("close-reconnect")
        .ok_or_else(|| JsValue::from_str("close/reconnect button unavailable"))?
        .dyn_into::<HtmlButtonElement>()?;
    button.set_disabled(false);
    button.set_text_content(Some("Run close/reconnect observation"));
    let document = document.clone();
    let callback_button = button.clone();
    let callback = Closure::wrap(Box::new(move || {
        if active.replace(true) {
            return;
        }
        callback_button.set_disabled(true);
        let document = document.clone();
        let button = callback_button.clone();
        let active = active.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = match select(
                run_close_reconnect(document.clone()).boxed_local(),
                TimeoutFuture::new(15000).boxed_local(),
            )
            .await
            {
                Either::Left((result, _)) => result,
                Either::Right(((), future)) => {
                    drop(future);
                    Err("close/reconnect exceeded its 15-second deadline".to_owned())
                }
            };
            if let Err(error) = result {
                show_close(
                    &document,
                    &format!("FAIL · {error}\nClose/reconnect INCONCLUSIVE"),
                );
            }
            active.set(false);
            button.set_disabled(false);
            button.set_text_content(Some("Run close/reconnect observation again"));
        });
    }) as Box<dyn FnMut()>);
    button.set_onclick(Some(callback.as_ref().unchecked_ref()));
    callback.forget();
    Ok(())
}

async fn rejection_modes(client: &mut FixtureClient) -> Result<(), String> {
    let rejected = Sample {
        behavior: Behavior::Reject as i32,
        ..sample(0)
    };
    let server_error = client
        .server_stream(request(rejected.clone())?)
        .await
        .err()
        .ok_or("server stream rejection accepted")?;
    let client_error = client
        .client_stream(request(futures::stream::iter([rejected.clone()]))?)
        .await
        .err()
        .ok_or("client stream rejection accepted")?;
    let mut bidi = client
        .bidi(request(futures::stream::iter([rejected]))?)
        .await
        .map_err(|error| error.to_string())?
        .into_inner();
    let bidi_error = bidi
        .message()
        .await
        .err()
        .ok_or("bidi rejection accepted")?;
    for error in [server_error, client_error, bidi_error] {
        require(
            error.code() == Code::InvalidArgument
                && error
                    .metadata()
                    .get("fixture-terminal")
                    .is_some_and(|value| value == "observed"),
            "stream rejection status or trailers mismatch",
        )?;
    }
    Ok(())
}

pub(super) fn tunnel_url(path: &str) -> Result<String, String> {
    let location = web_sys::window().ok_or("window unavailable")?.location();
    let host = location.host().map_err(|_| "location host unavailable")?;
    Ok(format!("ws://{host}{path}"))
}
