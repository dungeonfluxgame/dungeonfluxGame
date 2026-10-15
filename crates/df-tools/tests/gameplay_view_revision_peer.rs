#![cfg(not(target_arch = "wasm32"))]

use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
};
use df_protocol::common as rpc;
use df_rpc_bridge::{FRAME_BYTES, MESSAGE_BYTES, NativeAdmission, NativeIncoming};
use futures::Stream;
use prost::Message;
use std::{
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, broadcast};
use tonic::{Request, Response, Status};

const PLAYER: &str = "5151515151515151515151515151515151515151515151515151515151515151";
const DISPLAY: &str = "5252525252525252525252525252525252525252525252525252525252525252";

#[derive(Clone)]
struct Service {
    events: broadcast::Sender<FixtureFrame>,
    current: Arc<Mutex<FixtureSnapshot>>,
    last_binding: Arc<Mutex<Option<Vec<u8>>>>,
    watches: Arc<AtomicUsize>,
    stale_sent: Arc<AtomicUsize>,
    input: Arc<Mutex<Option<(u64, u64)>>>,
    streams: Arc<Semaphore>,
}

#[derive(Clone, Copy)]
struct FixtureSnapshot {
    epoch: u64,
    sequence: u64,
    creation: bool,
}

#[derive(Clone, Copy)]
enum FixtureFrame {
    Current(FixtureSnapshot),
    Stale,
    ConflictingDuplicate(FixtureSnapshot),
}

fn revision(epoch: u64, sequence: u64) -> rpc::SessionRevision {
    rpc::SessionRevision {
        epoch: Some(rpc::RecoveryEpoch { value: Some(epoch) }),
        sequence: Some(sequence),
    }
}

#[derive(Clone, PartialEq, Message)]
struct OldViewMessage {
    #[prost(message, optional, tag = "3")]
    display: Option<OldDisplayGameplayView>,
}

#[derive(Clone, PartialEq, Message)]
struct OldDisplayGameplayView {
    #[prost(string, tag = "1")]
    narration: String,
    #[prost(string, tag = "2")]
    scene_asset: String,
    #[prost(bytes = "vec", optional, tag = "3")]
    scene: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "4")]
    journey: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
struct OldGameplayScene {
    #[prost(string, tag = "2")]
    title: String,
    #[prost(string, tag = "20")]
    live_plate: String,
}

#[test]
fn appended_item_card_and_scene_contents_are_ignored_by_an_old_scene_peer() {
    let current = rpc::GameplayScene {
        title: "A room with intent".into(),
        scene_contents_json: r#"{"people":[],"things":[]}"#.into(),
        item: Some(rpc::SceneItemCard {
            name: "Pewter tankard".into(),
            detail: "A dented tankard with green wax on its rim.".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let old = OldGameplayScene::decode(current.encode_to_vec().as_slice()).unwrap();
    assert_eq!(old.title, current.title);
    assert!(old.live_plate.is_empty());
}

#[derive(Clone, PartialEq, Message)]
struct OldCombatOutcome {
    #[prost(string, tag = "1")]
    actor_name: String,
    #[prost(uint32, repeated, tag = "9")]
    damage_dice: Vec<u32>,
    #[prost(uint32, tag = "10")]
    damage: u32,
}

#[test]
fn appended_scene_graph_is_ignored_by_an_old_display_client() {
    let current = rpc::ViewMessage {
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                narration: "The scene remains readable.".to_owned(),
                scene_asset: "assets/ui/scenes/harbor.webp".to_owned(),
                scene: Some(rpc::GameplayScene {
                    title: "Lantern Wharf".to_owned(),
                    live_plate: "/gameplay/imagery/plate".to_owned(),
                    scene_video_state: rpc::HomeVideoState::Ready as i32,
                    scene_video_path: "/gameplay/video/clip".to_owned(),
                    ..Default::default()
                }),
                scene_graph: Some(rpc::DisplaySceneGraph {
                    revision: 42,
                    scene_key: "opening".to_owned(),
                    ..Default::default()
                }),
                live_plate_graph_revision: 42,
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let old = OldViewMessage::decode(current.encode_to_vec().as_slice())
        .expect("an old client skips appended display fields");
    let display = old.display.expect("display audience is retained");
    assert_eq!(display.narration, "The scene remains readable.");
    assert_eq!(display.scene_asset, "assets/ui/scenes/harbor.webp");
    let scene = OldGameplayScene::decode(display.scene.unwrap().as_slice())
        .expect("an old client skips appended scene clip fields");
    assert_eq!(scene.title, "Lantern Wharf");
    assert_eq!(scene.live_plate, "/gameplay/imagery/plate");
}

#[test]
fn appended_damage_roll_fields_are_ignored_by_an_old_client() {
    let outcome = rpc::CombatOutcome {
        actor_name: "Brakka".to_owned(),
        damage_dice: vec![6, 5],
        damage: 14,
        damage_rolls: vec![
            rpc::DamageRoll {
                dice: vec![6, 4],
                kept: false,
            },
            rpc::DamageRoll {
                dice: vec![6, 5],
                kept: true,
            },
        ],
        damage_modifier: 3,
        ..Default::default()
    };
    let old = OldCombatOutcome::decode(outcome.encode_to_vec().as_slice())
        .expect("an old client skips appended damage-roll fields");
    assert_eq!(old.actor_name, "Brakka");
    assert_eq!(old.damage_dice, [6, 5]);
    assert_eq!(old.damage, 14);
}

fn role<T>(request: &Request<T>) -> Result<bool, Status> {
    match request
        .metadata()
        .get("x-df-local-binding")
        .and_then(|value| value.to_str().ok())
    {
        Some(PLAYER) => Ok(true),
        Some(DISPLAY) => Ok(false),
        _ => Err(Status::unauthenticated("fixture binding required")),
    }
}

fn view(player: bool, frame: FixtureFrame) -> rpc::ViewMessage {
    let stale = matches!(frame, FixtureFrame::Stale);
    let conflicting = matches!(frame, FixtureFrame::ConflictingDuplicate(_));
    let snapshot = match frame {
        FixtureFrame::Current(snapshot) | FixtureFrame::ConflictingDuplicate(snapshot) => snapshot,
        FixtureFrame::Stale => FixtureSnapshot {
            epoch: 1,
            sequence: 99,
            creation: false,
        },
    };
    let sequence = snapshot.sequence;
    let scene = rpc::GameplayScene {
        destination: rpc::HarborDestination::LanternWharf as i32,
        title: if conflicting {
            "CONFLICTING duplicate must never appear".to_owned()
        } else if stale {
            "STALE retired epoch must never appear".to_owned()
        } else if snapshot.epoch == 2 && sequence == 1 {
            "Recovered epoch 2 — Lantern Wharf".to_owned()
        } else {
            format!(
                "Current epoch {} sequence {sequence} — Lantern Wharf",
                snapshot.epoch
            )
        },
        scene_asset: "assets/ui/scenes/mara-harbor-v4.png".to_owned(),
        speaker_portrait: String::new(),
        ..Default::default()
    };
    let narration = if stale {
        "This delayed snapshot belongs to epoch 1, sequence 99.".to_owned()
    } else if conflicting {
        "Conflicting same-revision data must not replace the retained view.".to_owned()
    } else {
        format!(
            "Current permitted view: epoch {}, sequence {sequence}. The recovered scene must remain after delayed old traffic.",
            snapshot.epoch
        )
    };
    let journey = rpc::JourneyView {
        phase: if snapshot.creation {
            rpc::JourneyPhase::CharacterCreation
        } else {
            rpc::JourneyPhase::Opening
        } as i32,
        room_code: "LANTERN".to_owned(),
        own_character: player.then_some(rpc::CharacterSheet {
            name: "Brakka".to_owned(),
            hit_points: 12,
            maximum_hit_points: 12,
            armor_class: 14,
            source_revision: "synthetic-notebook-fixture-v1".to_owned(),
            ..Default::default()
        }),
        creation: if player && snapshot.creation {
            Some(rpc::CharacterCreationOffer {
                description: if conflicting {
                    "CONFLICTING duplicate creation must never appear".to_owned()
                } else {
                    format!(
                        "Creation fixture · stable binding 53 · epoch {}. Enter a draft and select an offered option before reconnecting.",
                        snapshot.epoch
                    )
                },
                source_revision: "synthetic-creation-contract-v1".to_owned(),
                groups: vec![rpc::JourneyGroup {
                    group_id: "fixture-choice".to_owned(),
                    label: "Draft fixture choice".to_owned(),
                    options: vec![
                        rpc::JourneyOption {
                            option_id: "lantern".to_owned(),
                            label: "Lantern".to_owned(),
                            description: "First synthetic offered choice for draft preservation."
                                .to_owned(),
                            ..Default::default()
                        },
                        rpc::JourneyOption {
                            option_id: "compass".to_owned(),
                            label: "Compass".to_owned(),
                            description: "Second synthetic offered choice for draft preservation."
                                .to_owned(),
                            ..Default::default()
                        },
                    ],
                }],
                ..Default::default()
            })
        } else {
            None
        },
        notebook: if player {
            vec![
                rpc::NotebookEntry {
                    kind: rpc::NotebookKind::Person as i32,
                    title: "Tomas Greaves".to_owned(),
                    text: "Mapmaker at Tin Street Tea House.".to_owned(),
                    scene: "opening".to_owned(),
                    order: 1,
                    ..Default::default()
                },
                rpc::NotebookEntry {
                    kind: rpc::NotebookKind::Clue as i32,
                    title: "A folded letter".to_owned(),
                    text: "Jorun says the letter is hidden beneath the ferry desk.".to_owned(),
                    scene: "opening".to_owned(),
                    order: 2,
                    owner: Some("Brakka".to_owned()),
                    author: Some("Brakka".to_owned()),
                    ..Default::default()
                },
            ]
        } else {
            vec![rpc::NotebookEntry {
                kind: rpc::NotebookKind::Person as i32,
                title: "Tomas Greaves".to_owned(),
                text: "Mapmaker at Tin Street Tea House.".to_owned(),
                scene: "opening".to_owned(),
                order: 1,
                ..Default::default()
            }]
        },
        ..Default::default()
    };
    let audience = if player {
        rpc::view_message::Audience::Player(rpc::PlayerGameplayView {
            narration,
            scene: Some(scene),
            journey: Some(journey),
            offers: vec![rpc::GameplayActionOffer {
                offer_id: if snapshot.creation {
                    "creation-basis-check"
                } else {
                    "recovery-basis-check"
                }
                .to_owned(),
                action_kind: if snapshot.creation {
                    rpc::GameplayActionKind::CreateCharacter
                } else {
                    rpc::GameplayActionKind::ExamineHarborSeal
                } as i32,
                label: "Check recovered view basis".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        })
    } else {
        rpc::view_message::Audience::Display(rpc::DisplayGameplayView {
            narration,
            scene: Some(scene),
            journey: Some(journey),
            ..Default::default()
        })
    };
    rpc::ViewMessage {
        revision: Some(if stale {
            revision(1, 99)
        } else {
            revision(snapshot.epoch, sequence)
        }),
        audience: Some(audience),
    }
}

type ViewStream = Pin<Box<dyn Stream<Item = Result<rpc::ViewMessage, Status>> + Send>>;

#[tonic::async_trait]
impl rpc::session_service_server::SessionService for Service {
    type WatchStream = ViewStream;
    async fn watch(
        &self,
        request: Request<rpc::WatchViewRequest>,
    ) -> Result<Response<ViewStream>, Status> {
        let player = role(&request)?;
        let request = request.into_inner();
        if request
            .session_id
            .as_ref()
            .and_then(|id| id.value.as_deref())
            != Some([0x41; 16].as_slice())
            || request.run_id.as_ref().and_then(|id| id.value.as_deref())
                != Some([0x42; 16].as_slice())
        {
            return Err(Status::invalid_argument("fixture session/run required"));
        }
        let expected_binding = if player { 0x53 } else { 0x72 };
        let binding = request
            .client_binding_id
            .as_ref()
            .and_then(|id| id.value.as_deref());
        if binding != Some([expected_binding; 16].as_slice()) {
            return Err(Status::permission_denied("fixture watch binding denied"));
        }
        *self
            .last_binding
            .lock()
            .map_err(|_| Status::internal("fixture binding record unavailable"))? =
            binding.map(Vec::from);
        let permit = self
            .streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("fixture stream bound"))?;
        self.watches.fetch_add(1, Ordering::SeqCst);
        let events = self.events.subscribe();
        let sent = self.stale_sent.clone();
        let snapshot = *self
            .current
            .lock()
            .map_err(|_| Status::internal("fixture snapshot unavailable"))?;
        let stream = futures::stream::unfold(
            (true, events, permit, false),
            move |(first, mut events, permit, ended)| {
                let sent = sent.clone();
                async move {
                    if ended {
                        return None;
                    }
                    let result = if first {
                        Ok(view(player, FixtureFrame::Current(snapshot)))
                    } else {
                        match events.recv().await {
                            Ok(frame) => {
                                if matches!(frame, FixtureFrame::Stale) {
                                    sent.fetch_add(1, Ordering::SeqCst);
                                }
                                Ok(view(player, frame))
                            }
                            Err(error) => Err(Status::unavailable(format!(
                                "fixture stream ended: {error}"
                            ))),
                        }
                    };
                    let ended = result.is_err();
                    Some((result, (false, events, permit, ended)))
                }
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }

    /// The fixture has no pacing gate: a presentation report is answered with the
    /// sequence it carried, as a fully released table would.
    async fn report_presented(
        &self,
        request: Request<rpc::PresentedRequest>,
    ) -> Result<Response<rpc::PresentedResponse>, Status> {
        Ok(Response::new(rpc::PresentedResponse {
            released_sequence: request.into_inner().sequence,
        }))
    }
}

#[tonic::async_trait]
impl rpc::room_service_server::RoomService for Service {
    async fn current_adventure(
        &self,
        _: Request<rpc::CurrentAdventureRequest>,
    ) -> Result<Response<rpc::CurrentAdventureResponse>, Status> {
        Ok(Response::new(rpc::CurrentAdventureResponse {
            room_code: "LANTERN".to_owned(),
            run_identity: String::new(),
            preparing: false,
        }))
    }

    async fn reclaim(
        &self,
        _request: Request<rpc::ReclaimRoomRequest>,
    ) -> Result<Response<rpc::ReclaimRoomResponse>, Status> {
        Err(Status::unimplemented(
            "reclaim is outside the revision-order fixture",
        ))
    }

    async fn join(
        &self,
        request: Request<rpc::JoinRoomRequest>,
    ) -> Result<Response<rpc::JoinRoomResponse>, Status> {
        let body = request.into_inner();
        if body.room_code != "LANTERN"
            || body.join_secret.len() != 32
            || body
                .operation_id
                .as_ref()
                .and_then(|id| id.value.as_ref())
                .is_none_or(|id| id.len() != 16)
        {
            return Err(Status::invalid_argument("bounded fixture join required"));
        }
        let session_id = Some(rpc::SessionId {
            value: Some(vec![0x41; 16]),
        });
        let run_id = Some(rpc::RunId {
            value: Some(vec![0x42; 16]),
        });
        Ok(Response::new(rpc::JoinRoomResponse {
            outcome: Some(rpc::join_room_response::Outcome::Joined(Box::new(
                rpc::JoinedRoom {
                    receipt: Some(rpc::DecisionReceipt {
                        operation_id: body.operation_id,
                        session_id: session_id.clone(),
                        run_id: run_id.clone(),
                        revision: Some(revision(2, 1)),
                        outcome: Some(rpc::decision_receipt::Outcome::Accepted(Default::default())),
                        replayed: false,
                    }),
                    local_binding: PLAYER.to_owned(),
                    session_id,
                    run_id,
                    client_binding_id: Some(rpc::ClientBindingId {
                        value: Some(vec![0x53; 16]),
                    }),
                },
            ))),
        }))
    }
}

#[tonic::async_trait]
impl rpc::action_service_server::ActionService for Service {
    async fn preview_hero_identity(
        &self,
        _request: Request<rpc::PreviewHeroIdentityRequest>,
    ) -> Result<Response<rpc::PreviewHeroIdentityResponse>, Status> {
        Err(Status::unimplemented(
            "the revision peer does not read hero descriptions",
        ))
    }

    async fn submit(
        &self,
        request: Request<rpc::SubmitActionRequest>,
    ) -> Result<Response<rpc::SubmitActionResponse>, Status> {
        if !role(&request)? {
            return Err(Status::permission_denied("display cannot submit"));
        }
        let body = request.into_inner();
        let observed = body
            .observed_revision
            .ok_or_else(|| Status::invalid_argument("revision required"))?;
        let epoch = observed
            .epoch
            .and_then(|epoch| epoch.value)
            .ok_or_else(|| Status::invalid_argument("epoch required"))?;
        let sequence = observed
            .sequence
            .ok_or_else(|| Status::invalid_argument("sequence required"))?;
        *self
            .input
            .lock()
            .map_err(|_| Status::internal("fixture input record unavailable"))? =
            Some((epoch, sequence));
        Ok(Response::new(rpc::SubmitActionResponse {
            outcome: Some(rpc::submit_action_response::Outcome::CommittedDecision(
                Box::new(rpc::DecisionReceipt {
                    operation_id: body.operation_id,
                    session_id: body.session_id,
                    run_id: body.run_id,
                    revision: Some(revision(2, 1)),
                    replayed: false,
                    outcome: Some(rpc::decision_receipt::Outcome::Rejected(rpc::Rejection {
                        code: rpc::RejectionCode::CapabilityUnavailable as i32,
                    })),
                }),
            )),
        }))
    }
}

#[derive(Clone)]
struct PageState {
    service: Service,
    admission: NativeAdmission,
    origin: String,
    permits: Arc<Semaphore>,
    html: String,
}

async fn page(State(state): State<PageState>) -> Html<String> {
    Html(state.html)
}
async fn stale(State(state): State<PageState>) -> impl IntoResponse {
    match state.service.events.send(FixtureFrame::Stale) {
        Ok(receivers) => (
            StatusCode::OK,
            format!("stale snapshot queued for {receivers} watchers"),
        ),
        Err(_) => (StatusCode::CONFLICT, "no active watcher".to_owned()),
    }
}
async fn newer(State(state): State<PageState>) -> impl IntoResponse {
    let snapshot = match state.service.current.lock() {
        Ok(mut current) => {
            let Some(sequence) = current.sequence.checked_add(1) else {
                return (
                    StatusCode::CONFLICT,
                    "fixture sequence exhausted".to_owned(),
                );
            };
            current.sequence = sequence;
            *current
        }
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "fixture snapshot unavailable".to_owned(),
            );
        }
    };
    match state.service.events.send(FixtureFrame::Current(snapshot)) {
        Ok(receivers) => (
            StatusCode::OK,
            format!("newer snapshot queued for {receivers} watchers"),
        ),
        Err(_) => (StatusCode::CONFLICT, "no active watcher".to_owned()),
    }
}
async fn conflicting_duplicate(State(state): State<PageState>) -> impl IntoResponse {
    let snapshot = match state.service.current.lock() {
        Ok(current) => *current,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "fixture snapshot unavailable".to_owned(),
            );
        }
    };
    match state
        .service
        .events
        .send(FixtureFrame::ConflictingDuplicate(snapshot))
    {
        Ok(receivers) => (
            StatusCode::OK,
            format!("conflicting duplicate queued for {receivers} watchers"),
        ),
        Err(_) => (StatusCode::CONFLICT, "no active watcher".to_owned()),
    }
}
async fn creation(State(state): State<PageState>) -> impl IntoResponse {
    publish_creation(&state, false)
}
async fn creation_new_epoch(State(state): State<PageState>) -> impl IntoResponse {
    publish_creation(&state, true)
}
fn publish_creation(state: &PageState, new_epoch: bool) -> (StatusCode, String) {
    let snapshot = match state.service.current.lock() {
        Ok(mut current) => {
            let next = if new_epoch {
                current.epoch.checked_add(1).map(|epoch| (epoch, 0))
            } else {
                current
                    .sequence
                    .checked_add(1)
                    .map(|sequence| (current.epoch, sequence))
            };
            let Some((epoch, sequence)) = next else {
                return (
                    StatusCode::CONFLICT,
                    "fixture revision exhausted".to_owned(),
                );
            };
            *current = FixtureSnapshot {
                epoch,
                sequence,
                creation: true,
            };
            *current
        }
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "fixture snapshot unavailable".to_owned(),
            );
        }
    };
    match state.service.events.send(FixtureFrame::Current(snapshot)) {
        Ok(receivers) => (
            StatusCode::OK,
            format!(
                "creation epoch {} sequence {} queued for {receivers} watchers",
                snapshot.epoch, snapshot.sequence
            ),
        ),
        Err(_) => (
            StatusCode::CONFLICT,
            "no active watcher; current creation snapshot retained".to_owned(),
        ),
    }
}
async fn fixture_status(State(state): State<PageState>) -> Result<String, StatusCode> {
    let input = *state
        .service
        .input
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let current = *state
        .service
        .current
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let last_binding = state
        .service
        .last_binding
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let binding = last_binding.as_ref().map(|bytes| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    });
    Ok(format!(
        "watches={} stale_sent={} current_epoch={} current_sequence={} creation={} last_binding={binding:?} last_input={input:?}",
        state.service.watches.load(Ordering::SeqCst),
        state.service.stale_sent.load(Ordering::SeqCst),
        current.epoch,
        current.sequence,
        current.creation
    ))
}
async fn socket(
    State(state): State<PageState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    if headers
        .get("origin")
        .and_then(|origin| origin.to_str().ok())
        != Some(state.origin.as_str())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = state.permits.try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(admission) = state.admission.try_reserve() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    upgrade
        .read_buffer_size(FRAME_BYTES)
        .write_buffer_size(FRAME_BYTES)
        .max_write_buffer_size(MESSAGE_BYTES)
        .max_message_size(MESSAGE_BYTES)
        .max_frame_size(MESSAGE_BYTES)
        .on_upgrade(move |mut socket| async move {
            if socket
                .send(axum::extract::ws::Message::Binary(Vec::new().into()))
                .await
                .is_err()
            {
                drop(permit);
                return;
            }
            let _result = admission
                .accept_websocket_measured(
                    socket,
                    Arc::new(df_rpc_bridge::ConnectionMetrics::new(false)),
                )
                .await;
            drop(permit);
        })
}

/// Temporary independent-evaluation harness only. The browser loads actual candidate
/// df_tools WASM/glue and production gameplay HTML; only the generated RPC peer is fake.
#[tokio::test]
#[ignore = "requires coordinator runtime/browser lease and candidate WASM"]
async fn serve_actual_gameplay_client_with_late_old_epoch() -> Result<(), Box<dyn std::error::Error>>
{
    let web = std::env::var("DF_REVISION_FIXTURE_WEB")?;
    let assets = std::env::var("DF_REVISION_FIXTURE_ASSETS")?;
    let port: u16 = std::env::var("DF_REVISION_FIXTURE_PORT")?.parse()?;
    if port == 0 {
        return Err("nonzero owned port required".into());
    }
    let html = std::fs::read_to_string(std::env::var("DF_REVISION_FIXTURE_HTML")?)?;
    let roots = format!(
        "<section class=\"client player\" id=\"player\" data-room=\"LANTERN\"><div class=\"connection\" role=\"status\"></div><div class=\"view\"></div></section><section class=\"client display\" id=\"display\" data-local-binding=\"{DISPLAY}\"><div class=\"connection\" role=\"status\"></div><div class=\"view\"></div></section>"
    );
    let html = html
        .replace("__ROOTS__", &roots)
        .replace("__LAYOUT__", "clients");
    let creation_mode = match std::env::var("DF_REVISION_FIXTURE_MODE") {
        Ok(mode) if mode == "creation" => true,
        Ok(mode) if mode == "opening" => false,
        Err(std::env::VarError::NotPresent) => false,
        _ => return Err("fixture mode must be opening or creation".into()),
    };
    let (events, _) = broadcast::channel(8);
    let service = Service {
        events,
        current: Arc::new(Mutex::new(FixtureSnapshot {
            epoch: 2,
            sequence: 1,
            creation: creation_mode,
        })),
        last_binding: Arc::new(Mutex::new(None)),
        watches: Arc::new(AtomicUsize::new(0)),
        stale_sent: Arc::new(AtomicUsize::new(0)),
        input: Arc::new(Mutex::new(None)),
        streams: Arc::new(Semaphore::new(8)),
    };
    let (admission, incoming) =
        NativeIncoming::bounded(NonZeroUsize::new(4).ok_or("connection bound")?)?;
    let grpc = tonic::transport::Server::builder()
        .initial_stream_window_size(MESSAGE_BYTES as u32)
        .initial_connection_window_size(df_rpc_bridge::RECEIVE_BYTES as u32)
        .max_frame_size(FRAME_BYTES as u32)
        .http2_max_header_list_size(FRAME_BYTES as u32)
        .max_concurrent_streams(df_rpc_bridge::CONCURRENT_STREAMS)
        .add_service(
            rpc::session_service_server::SessionServiceServer::new(service.clone())
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        )
        .add_service(
            rpc::action_service_server::ActionServiceServer::new(service.clone())
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        )
        .add_service(
            rpc::room_service_server::RoomServiceServer::new(service.clone())
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        )
        .serve_with_incoming(incoming);
    let state = PageState {
        service,
        admission,
        origin: format!("http://127.0.0.1:{port}"),
        permits: Arc::new(Semaphore::new(4)),
        html,
    };
    let router = Router::new()
        .route("/gameplay", get(page))
        .route("/gameplay/tunnel", get(socket))
        .route("/fixture/stale", get(stale))
        .route("/fixture/newer", get(newer))
        .route("/fixture/conflicting-duplicate", get(conflicting_duplicate))
        .route("/fixture/creation", get(creation))
        .route("/fixture/creation-new-epoch", get(creation_new_epoch))
        .route("/fixture/status", get(fixture_status))
        .nest_service("/pkg", tower_http::services::ServeDir::new(web))
        .nest_service("/assets", tower_http::services::ServeDir::new(assets))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!("revision-fixture ready http://127.0.0.1:{port}/gameplay; finite 170-second lifetime");
    tokio::select! {
        result = grpc => result?,
        result = axum::serve(listener, router) => result?,
        () = tokio::time::sleep(Duration::from_secs(170)) => {},
    }
    Ok(())
}
