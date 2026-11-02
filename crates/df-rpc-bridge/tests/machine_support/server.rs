//! The sample service, the gRPC server that carries it and the `/tunnel` route that feeds it.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    response::{IntoResponse, Response},
    routing::get,
};
use df_api::machine::{
    AdmittedMachineIntent, AdmittedPayload, RpcStatusClass, admit_machine_intent,
    check_machine_response, check_machine_view, sample,
};
use df_protocol::common as wire;
use df_protocol::common::{
    action_service_server::{ActionService, ActionServiceServer},
    session_service_server::{SessionService, SessionServiceServer},
};
use df_rpc_bridge::{
    CONCURRENT_STREAMS, ConnectionMetrics, FRAME_BYTES, MESSAGE_BYTES, NativeAdmission,
    NativeIncoming, RECEIVE_BYTES, RPC_MESSAGE_BYTES,
};
use futures::Stream;
use tokio::sync::watch;
use tonic::{Request, Response as RpcResponse, Status};

type ViewStream = Pin<Box<dyn Stream<Item = Result<wire::MachineView, Status>> + Send>>;

/// The gRPC status of a refusal, from the class `df-api` assigns it: no service invents its own.
fn status_of(class: RpcStatusClass, error: &dyn std::fmt::Debug) -> Status {
    match class {
        RpcStatusClass::InvalidArgument => Status::invalid_argument(format!("refused: {error:?}")),
        RpcStatusClass::Internal => Status::internal(format!("refused: {error:?}")),
    }
}

/// One seat's machine: the view it shows and the intents it has admitted.
#[derive(Clone)]
pub struct SampleMachineService {
    view: Arc<watch::Sender<wire::MachineView>>,
    admitted: Arc<Mutex<Vec<AdmittedMachineIntent>>>,
}

impl SampleMachineService {
    /// The seat's machine: a phone's view, with offers, that a tap moves on.
    pub fn new() -> Self {
        Self::showing(sample::seat_view())
    }

    /// The shared screen's machine: the view the TV gets, which has no choices.
    // The round-trip test serves it; the example server shows a seat.
    #[allow(dead_code)]
    pub fn shared_screen() -> Self {
        Self::showing(sample::shared_screen_view())
    }

    fn showing(view: wire::MachineView) -> Self {
        Self {
            view: Arc::new(watch::channel(view).0),
            admitted: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The intents that passed admission, in order.
    // The round-trip test reads it; the example server shares this file and does not.
    #[allow(dead_code)]
    pub fn admitted(&self) -> Vec<AdmittedMachineIntent> {
        match self.admitted.lock() {
            Ok(admitted) => admitted.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn record(&self, intent: &AdmittedMachineIntent) {
        match self.admitted.lock() {
            Ok(mut admitted) => admitted.push(intent.clone()),
            Err(poisoned) => poisoned.into_inner().push(intent.clone()),
        }
    }

    /// Moves the seat to the next revision with one more line of narration.
    fn advance(&self) {
        self.view.send_modify(|view| {
            if let Some(revision) = view.revision.as_mut() {
                revision.sequence = revision.sequence.map(|sequence| sequence + 1);
            }
            if let Some(wire::machine_view::Audience::Seat(seat)) = view.audience.as_mut() {
                seat.narration.push(sample::TAP_NARRATION.to_owned());
            }
        });
    }

    fn revision_now(&self) -> Option<wire::SessionRevision> {
        self.view.borrow().revision
    }
}

fn receipt(
    intent: &AdmittedMachineIntent,
    revision: Option<wire::SessionRevision>,
    utterance_id: Option<u64>,
    result: wire::machine_receipt::Result,
) -> wire::SubmitMachineIntentResponse {
    wire::SubmitMachineIntentResponse {
        outcome: Some(wire::submit_machine_intent_response::Outcome::Receipt(
            wire::MachineReceipt {
                operation_id: Some(wire::OperationId {
                    value: Some(intent.operation.as_bytes().to_vec()),
                }),
                revision,
                replayed: false,
                utterance_id,
                result: Some(result),
            },
        )),
    }
}

fn decline(code: wire::RejectionCode, reason: &str) -> wire::machine_receipt::Result {
    wire::machine_receipt::Result::Declined(wire::MachineDecline {
        code: code as i32,
        reason: reason.to_owned(),
        missing: None,
        alternatives: Vec::new(),
    })
}

#[tonic::async_trait]
impl ActionService for SampleMachineService {
    async fn submit(
        &self,
        _: Request<wire::SubmitActionRequest>,
    ) -> Result<RpcResponse<wire::SubmitActionResponse>, Status> {
        Err(Status::unimplemented("not part of the machine sample"))
    }

    async fn preview_hero_identity(
        &self,
        _: Request<wire::PreviewHeroIdentityRequest>,
    ) -> Result<RpcResponse<wire::PreviewHeroIdentityResponse>, Status> {
        Err(Status::unimplemented("not part of the machine sample"))
    }

    async fn request_hero_portrait(
        &self,
        _: Request<wire::HeroPortraitRequest>,
    ) -> Result<RpcResponse<wire::HeroPortraitResponse>, Status> {
        Err(Status::unimplemented("not part of the machine sample"))
    }

    async fn get_hero_portrait(
        &self,
        _: Request<wire::HeroPortraitRequest>,
    ) -> Result<RpcResponse<wire::HeroPortraitResponse>, Status> {
        Err(Status::unimplemented("not part of the machine sample"))
    }

    /// Admits the request, then answers by kind: a tap on the sample offer applies and moves the
    /// view on, a line is declined as not the seat's turn (by utterance id, never its words), a
    /// tap on any other offer is stale, and a pending answer has no public decline class.
    async fn submit_machine_intent(
        &self,
        request: Request<wire::SubmitMachineIntentRequest>,
    ) -> Result<RpcResponse<wire::SubmitMachineIntentResponse>, Status> {
        let intent = admit_machine_intent(request.get_ref())
            .map_err(|error| status_of(error.rpc_status(), &error))?;
        self.record(&intent);
        let response = match &intent.payload {
            AdmittedPayload::Tap { offer_id } if offer_id == sample::OFFER_ID => {
                self.advance();
                receipt(
                    &intent,
                    self.revision_now(),
                    None,
                    wire::machine_receipt::Result::Applied(wire::MachineApplied {}),
                )
            }
            AdmittedPayload::Tap { .. } => receipt(
                &intent,
                self.revision_now(),
                None,
                decline(
                    wire::RejectionCode::StaleOffer,
                    "That choice is no longer offered.",
                ),
            ),
            AdmittedPayload::SubmitText(line) | AdmittedPayload::Say(line) => receipt(
                &intent,
                self.revision_now(),
                Some(line.utterance_id),
                decline(wire::RejectionCode::WrongTurn, "It isn't your turn."),
            ),
            AdmittedPayload::SafetyCard => receipt(
                &intent,
                self.revision_now(),
                None,
                wire::machine_receipt::Result::Applied(wire::MachineApplied {}),
            ),
            AdmittedPayload::PendingAnswer { .. } => receipt(
                &intent,
                self.revision_now(),
                None,
                decline(wire::RejectionCode::Unspecified, "I can't run that yet."),
            ),
        };
        check_machine_response(&response, &intent)
            .map_err(|error| status_of(error.rpc_status(), &error))?;
        Ok(RpcResponse::new(response))
    }
}

/// The current view, then each later one, every one checked before it leaves.
fn view_stream(receiver: watch::Receiver<wire::MachineView>) -> ViewStream {
    Box::pin(futures::stream::unfold(
        (receiver, true),
        |(mut receiver, first)| async move {
            if !first && receiver.changed().await.is_err() {
                return None;
            }
            let view = receiver.borrow_and_update().clone();
            let item = check_machine_view(&view)
                .map(|()| view)
                .map_err(|error| status_of(error.rpc_status(), &error));
            Some((item, (receiver, false)))
        },
    ))
}

#[tonic::async_trait]
impl SessionService for SampleMachineService {
    type WatchStream = Pin<Box<dyn Stream<Item = Result<wire::ViewMessage, Status>> + Send>>;
    type WatchMachineStream = ViewStream;

    async fn watch(
        &self,
        _: Request<wire::WatchViewRequest>,
    ) -> Result<RpcResponse<Self::WatchStream>, Status> {
        Err(Status::unimplemented("not part of the machine sample"))
    }

    async fn report_presented(
        &self,
        _: Request<wire::PresentedRequest>,
    ) -> Result<RpcResponse<wire::PresentedResponse>, Status> {
        Err(Status::unimplemented("not part of the machine sample"))
    }

    async fn watch_machine(
        &self,
        _: Request<wire::WatchViewRequest>,
    ) -> Result<RpcResponse<Self::WatchMachineStream>, Status> {
        Ok(RpcResponse::new(view_stream(self.view.subscribe())))
    }
}

/// The gRPC server over the bridge's incoming connections, with the bridge's message limits.
pub fn serve_grpc(
    incoming: NativeIncoming,
    service: SampleMachineService,
) -> impl Future<Output = Result<(), tonic::transport::Error>> {
    tonic::transport::Server::builder()
        .initial_stream_window_size(MESSAGE_BYTES as u32)
        .initial_connection_window_size(RECEIVE_BYTES as u32)
        .max_frame_size(FRAME_BYTES as u32)
        .http2_max_header_list_size(FRAME_BYTES as u32)
        .max_concurrent_streams(CONCURRENT_STREAMS)
        .add_service(
            ActionServiceServer::new(service.clone())
                .max_decoding_message_size(RPC_MESSAGE_BYTES)
                .max_encoding_message_size(RPC_MESSAGE_BYTES),
        )
        .add_service(
            SessionServiceServer::new(service)
                .max_decoding_message_size(RPC_MESSAGE_BYTES)
                .max_encoding_message_size(RPC_MESSAGE_BYTES),
        )
        .serve_with_incoming(incoming)
}

async fn tunnel(State(admission): State<NativeAdmission>, upgrade: WebSocketUpgrade) -> Response {
    let permit = match admission.try_reserve() {
        Ok(permit) => permit,
        Err(error) => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                error.to_string(),
            )
                .into_response();
        }
    };
    upgrade
        .read_buffer_size(FRAME_BYTES)
        .write_buffer_size(FRAME_BYTES)
        .max_write_buffer_size(MESSAGE_BYTES)
        .max_message_size(MESSAGE_BYTES)
        .max_frame_size(MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            // The connection's outcome is read from its metrics; there is no caller to tell.
            drop(
                permit
                    .accept_websocket_measured(socket, Arc::new(ConnectionMetrics::new(false)))
                    .await,
            );
        })
}

/// The one route that turns a WebSocket upgrade into a gRPC connection.
pub fn tunnel_router(admission: NativeAdmission) -> Router {
    Router::new()
        .route("/tunnel", get(tunnel))
        .with_state(admission)
}
