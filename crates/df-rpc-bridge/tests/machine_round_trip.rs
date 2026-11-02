#![cfg(not(target_arch = "wasm32"))]
//! One machine view and one machine intent through the native gRPC-over-WebSocket bridge
//! (plan O.16 W1): generated tonic clients speak HTTP/2 inside binary WebSocket messages to the
//! generated server behind `NativeIncoming`. The browser half of the same exchange is the
//! `machine_browser` example.

mod machine_support;

use std::{
    future::Future,
    net::SocketAddr,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    time::Duration,
};

use df_api::machine::{AdmittedPayload, check_machine_view, sample};
use df_protocol::common as wire;
use df_protocol::common::{
    action_service_client::ActionServiceClient, session_service_client::SessionServiceClient,
};
use df_rpc_bridge::{FRAME_BYTES, MESSAGE_BYTES, NativeIncoming, RPC_MESSAGE_BYTES};
use futures::{SinkExt, StreamExt};
use machine_support::server::{SampleMachineService, serve_grpc, tunnel_router};
use prost::Message;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::Message as SocketMessage;

/// Longest any step of the exchange may take before the test calls it hung.
const STEP_LIMIT: Duration = Duration::from_secs(30);
/// What every HTTP/2 client sends first (RFC 9113 section 3.4).
const HTTP2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

async fn within<T>(step: &str, future: impl Future<Output = T>) -> T {
    match tokio::time::timeout(STEP_LIMIT, future).await {
        Ok(value) => value,
        Err(_) => panic!("{step} did not finish within {STEP_LIMIT:?}"),
    }
}

/// What the WebSocket carried toward the server: one entry per binary message.
type UploadLog = Arc<Mutex<Vec<Vec<u8>>>>;

struct Bridge {
    service: SampleMachineService,
    uploads: UploadLog,
    actions: ActionServiceClient<tonic::transport::Channel>,
    sessions: SessionServiceClient<tonic::transport::Channel>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Relays one TCP connection (the generated client's HTTP/2 bytes) over one WebSocket to the
/// bridge, as binary messages, and back, recording what went up.
fn relay(listener: TcpListener, address: SocketAddr, uploads: UploadLog) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let Ok((socket, _)) =
            tokio_tungstenite::connect_async(format!("ws://{address}/tunnel")).await
        else {
            return;
        };
        let (mut socket_writer, mut socket_reader) = socket.split();
        let (mut tcp_reader, mut tcp_writer) = tcp.into_split();
        let upload = async {
            let mut buffer = vec![0; FRAME_BYTES];
            loop {
                let Ok(count) = tcp_reader.read(&mut buffer).await else {
                    return;
                };
                if count == 0 {
                    drop(socket_writer.send(SocketMessage::Close(None)).await);
                    return;
                }
                if let Ok(mut log) = uploads.lock() {
                    log.push(buffer[..count].to_vec());
                }
                if socket_writer
                    .send(SocketMessage::Binary(buffer[..count].to_vec().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        };
        let download = async {
            while let Some(Ok(message)) = socket_reader.next().await {
                match message {
                    SocketMessage::Binary(bytes) => {
                        if tcp_writer.write_all(&bytes).await.is_err() {
                            return;
                        }
                    }
                    SocketMessage::Ping(_) | SocketMessage::Pong(_) => {}
                    _ => return,
                }
            }
        };
        tokio::select! { () = upload => {}, () = download => {} }
    })
}

async fn start() -> Bridge {
    start_with(SampleMachineService::new()).await
}

async fn start_with(service: SampleMachineService) -> Bridge {
    let (admission, incoming) =
        NativeIncoming::bounded(NonZeroUsize::new(2).expect("two is nonzero"))
            .expect("two pending connections are within the channel limit");
    let grpc = tokio::spawn({
        let service = service.clone();
        async move {
            drop(serve_grpc(incoming, service).await);
        }
    });
    let web = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("an ephemeral loopback port");
    let web_address = web.local_addr().expect("the bound address");
    let server = tokio::spawn(async move {
        drop(axum::serve(web, tunnel_router(admission)).await);
    });
    // One TCP connection, one WebSocket: every call below shares it.
    let proxy_listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("an ephemeral loopback port");
    let proxy_address = proxy_listener.local_addr().expect("the bound address");
    let uploads = UploadLog::default();
    let proxy = relay(proxy_listener, web_address, uploads.clone());
    let channel = within(
        "connecting the generated client",
        tonic::transport::Endpoint::from_shared(format!("http://{proxy_address}"))
            .expect("a valid endpoint")
            .connect(),
    )
    .await
    .expect("the bridge accepts the connection");
    Bridge {
        service,
        uploads,
        actions: ActionServiceClient::new(channel.clone())
            .max_decoding_message_size(RPC_MESSAGE_BYTES)
            .max_encoding_message_size(RPC_MESSAGE_BYTES),
        sessions: SessionServiceClient::new(channel)
            .max_decoding_message_size(RPC_MESSAGE_BYTES)
            .max_encoding_message_size(RPC_MESSAGE_BYTES),
        tasks: vec![grpc, server, proxy],
    }
}

fn watch_request() -> wire::WatchViewRequest {
    wire::WatchViewRequest {
        session_id: Some(wire::SessionId {
            value: Some(sample::SESSION_BYTES.to_vec()),
        }),
        run_id: Some(wire::RunId {
            value: Some(sample::RUN_BYTES.to_vec()),
        }),
        client_binding_id: None,
        after_revision: None,
    }
}

fn result_of(response: wire::SubmitMachineIntentResponse) -> wire::MachineReceipt {
    match response.outcome {
        Some(wire::submit_machine_intent_response::Outcome::Receipt(receipt)) => receipt,
        other => panic!("expected a receipt, found {other:?}"),
    }
}

fn narration_of(view: &wire::MachineView) -> Vec<String> {
    match &view.audience {
        Some(wire::machine_view::Audience::Seat(seat)) => seat.narration.clone(),
        other => panic!("expected a seat view, found {other:?}"),
    }
}

#[tokio::test]
async fn a_view_and_an_intent_cross_the_bridge_and_the_next_view_follows() {
    let mut bridge = start().await;

    // The view: the first message of the watch is the seat's current view, whole and valid.
    let mut views = within(
        "opening WatchMachine",
        bridge.sessions.watch_machine(watch_request()),
    )
    .await
    .expect("the watch opens")
    .into_inner();
    let first = within("the first view", views.message())
        .await
        .expect("a view, not an error")
        .expect("the stream is not empty");
    assert_eq!(first, sample::seat_view());
    assert_eq!(check_machine_view(&first), Ok(()));

    // The intent: a tap on the sample offer is admitted as typed values and applied.
    let receipt = result_of(
        within(
            "the tap",
            bridge.actions.submit_machine_intent(sample::tap_request()),
        )
        .await
        .expect("the call succeeds")
        .into_inner(),
    );
    assert_eq!(
        receipt.result,
        Some(wire::machine_receipt::Result::Applied(
            wire::MachineApplied {}
        ))
    );
    assert_eq!(
        receipt.operation_id,
        Some(wire::OperationId {
            value: Some(sample::OPERATION_BYTES.to_vec())
        })
    );
    let admitted = bridge.service.admitted();
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].machine_id, sample::MACHINE_ID);
    assert_eq!(admitted[0].observed_revision.sequence(), sample::SEQUENCE);
    assert_eq!(
        admitted[0].payload,
        AdmittedPayload::Tap {
            offer_id: sample::OFFER_ID.to_owned()
        }
    );

    // The view that follows: one revision on, with the new line, on the same stream.
    let next = within("the next view", views.message())
        .await
        .expect("a view, not an error")
        .expect("the stream is still open");
    assert_eq!(check_machine_view(&next), Ok(()));
    let revision = next.revision.expect("a revision");
    assert_eq!(revision.sequence, Some(sample::SEQUENCE + 1));
    assert_eq!(receipt.revision, Some(revision));
    let lines = narration_of(&next);
    assert_eq!(lines.len(), narration_of(&first).len() + 1);
    assert_eq!(
        lines.last().map(String::as_str),
        Some(sample::TAP_NARRATION)
    );

    // What crossed was native HTTP/2 in bounded binary WebSocket messages on one connection.
    let uploads = bridge.uploads.lock().expect("the upload log").clone();
    assert!(
        uploads
            .first()
            .is_some_and(|message| message.starts_with(HTTP2_PREFACE)),
        "the first bytes up are not the HTTP/2 preface"
    );
    assert!(uploads.iter().all(|message| message.len() <= MESSAGE_BYTES));
}

#[tokio::test]
async fn a_declined_line_is_named_by_id_and_a_malformed_intent_is_a_call_failure() {
    let mut bridge = start().await;

    // A spoken line is declined with a public class, and the answer never echoes the words.
    let response = within(
        "the line",
        bridge.actions.submit_machine_intent(sample::say_request()),
    )
    .await
    .expect("the call succeeds")
    .into_inner();
    assert!(
        !response
            .encode_to_vec()
            .windows(sample::SAY_TEXT.len())
            .any(|window| window == sample::SAY_TEXT.as_bytes()),
        "the decline repeats the player's words"
    );
    let receipt = result_of(response);
    assert_eq!(receipt.utterance_id, Some(sample::UTTERANCE_ID));
    let Some(wire::machine_receipt::Result::Declined(declined)) = receipt.result else {
        panic!("expected a decline, found {:?}", receipt.result);
    };
    assert_eq!(
        wire::RejectionCode::try_from(declined.code),
        Ok(wire::RejectionCode::WrongTurn)
    );
    assert!(!declined.reason.trim().is_empty());

    // A tap on an offer the view does not list is stale.
    let mut stale = sample::tap_request();
    if let Some(intent) = stale.intent.as_mut() {
        intent.offer_id = Some("not-offered".to_owned());
    }
    let receipt = result_of(
        within("the stale tap", bridge.actions.submit_machine_intent(stale))
            .await
            .expect("the call succeeds")
            .into_inner(),
    );
    let Some(wire::machine_receipt::Result::Declined(declined)) = receipt.result else {
        panic!("expected a decline, found {:?}", receipt.result);
    };
    assert_eq!(
        wire::RejectionCode::try_from(declined.code),
        Ok(wire::RejectionCode::StaleOffer)
    );

    // A request the edge refuses is an RPC failure, not a receipt, and nothing is recorded.
    let mut malformed = sample::tap_request();
    if let Some(intent) = malformed.intent.as_mut() {
        intent.machine_id.clear();
    }
    let status = within(
        "the malformed intent",
        bridge.actions.submit_machine_intent(malformed),
    )
    .await
    .expect_err("a blank machine id is refused");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    // Only the line and the stale tap were admitted.
    assert_eq!(bridge.service.admitted().len(), 2);

    // A method the sample does not serve answers with its own status on the same connection.
    let status = within(
        "an unserved method",
        bridge
            .actions
            .submit(tonic::Request::new(wire::SubmitActionRequest::default())),
    )
    .await
    .expect_err("the sample does not serve Submit");
    assert_eq!(status.code(), tonic::Code::Unimplemented);
}

#[tokio::test]
async fn the_shared_screens_view_crosses_the_bridge_with_no_choices() {
    let mut bridge = start_with(SampleMachineService::shared_screen()).await;
    let mut views = within(
        "opening WatchMachine",
        bridge.sessions.watch_machine(watch_request()),
    )
    .await
    .expect("the watch opens")
    .into_inner();
    let view = within("the first view", views.message())
        .await
        .expect("a view, not an error")
        .expect("the stream is not empty");
    assert_eq!(view, sample::shared_screen_view());
    assert_eq!(check_machine_view(&view), Ok(()));
    assert!(matches!(
        view.audience,
        Some(wire::machine_view::Audience::SharedScreen(_))
    ));
}

// The bound `df-api` checks (it cannot name the bridge's constant) must leave room under it.
const _: () = assert!(df_api::machine::MAXIMUM_MESSAGE_BYTES < RPC_MESSAGE_BYTES);

#[tokio::test]
async fn the_pause_card_is_applied_and_the_message_bound_sits_below_the_bridge_limit() {
    let mut bridge = start().await;
    let receipt = result_of(
        within(
            "the pause card",
            bridge
                .actions
                .submit_machine_intent(sample::safety_card_request()),
        )
        .await
        .expect("the call succeeds")
        .into_inner(),
    );
    assert_eq!(
        receipt.result,
        Some(wire::machine_receipt::Result::Applied(
            wire::MachineApplied {}
        ))
    );
    assert_eq!(
        bridge
            .service
            .admitted()
            .last()
            .map(|intent| intent.payload.clone()),
        Some(AdmittedPayload::SafetyCard)
    );
}
