//! A finite, explicitly local gameplay slice. Authentication is preconfigured development
//! membership, not production bootstrap. The action and its outcome use generated RPC.
mod actor;
mod courier_ai;
mod courier_npc_qualification;
mod courier_process_qualification;
mod dialogue;
#[cfg(test)]
mod dialogue_tests;
mod inn_qualification;
mod journey;
mod model;
mod pack;
#[cfg(test)]
mod pack_tests;
mod public_assets;
mod qualification;
#[cfg(test)]
mod recovery_qualification;
mod rest_qualification;
mod restart_qualification;
mod room;
mod startup_configuration;
#[cfg(test)]
mod startup_contract;
#[cfg(test)]
mod tempo_clock;
mod wire;

use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    http::HeaderMap,
    response::{Html, IntoResponse},
    routing::get,
};
use df_observe::OperationContext;
use df_persistence::local_demo_scope::{LocalDemoAuthority, LocalDemoRole, LocalDemoScopeIssuer};
use df_persistence::{
    NativeCodecLimits, NativeRepositoryOptions, NativeTransactionBounds, PostgresRepository,
};
use df_protocol::common as rpc;
use df_rpc_bridge::{
    CONCURRENT_STREAMS, FRAME_BYTES, MESSAGE_BYTES, NativeAdmission, NativeIncoming,
};
use df_session::inbox::{InboxHandle, bounded_inbox};
use df_session::submission::DurableOwner;
use futures::Stream;
use std::{io, num::NonZeroUsize, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::{Semaphore, oneshot, watch};
use tonic::{Request, Response, Status};

#[derive(Clone)]
struct Service {
    actor: InboxHandle<actor::Call>,
    updates: watch::Receiver<df_model::checkpoint::Checkpoint>,
    streams: Arc<Semaphore>,
}
fn credential<T>(request: &Request<T>) -> Result<[u8; 32], Status> {
    let value = request
        .metadata()
        .get("x-df-local-binding")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Status::unauthenticated("local binding required"))?;
    if value.len() != 64 {
        return Err(Status::unauthenticated("local binding invalid"));
    }
    let mut result = [0; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |byte| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        let high =
            digit(pair[0]).ok_or_else(|| Status::unauthenticated("local binding invalid"))?;
        let low = digit(pair[1]).ok_or_else(|| Status::unauthenticated("local binding invalid"))?;
        result[index] = (high << 4) | low;
    }
    Ok(result)
}
impl Service {
    async fn view(
        &self,
        credential: [u8; 32],
        request: rpc::WatchViewRequest,
    ) -> Result<(LocalDemoRole, rpc::ViewMessage), Status> {
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::View {
                credential,
                request,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("gameplay actor queue full"))?;
        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .map_err(|_| Status::deadline_exceeded("gameplay view deadline"))?
            .map_err(|_| Status::unavailable("gameplay actor stopped"))?
    }
}
#[tonic::async_trait]
impl rpc::action_service_server::ActionService for Service {
    async fn submit(
        &self,
        request: Request<rpc::SubmitActionRequest>,
    ) -> Result<Response<rpc::SubmitActionResponse>, Status> {
        let credential = credential(&request)?;
        let request = request.into_inner();
        if request.offer_id.len() > 96
            || request.character.as_ref().is_some_and(|character| {
                character.name.len() > 48
                    || character.choices.len() > 16
                    || character
                        .choices
                        .iter()
                        .any(|choice| choice.group_id.len() > 64 || choice.option_id.len() > 64)
            })
        {
            return Err(Status::invalid_argument("bounded action fields required"));
        }
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::Submit {
                credential,
                request,
                reply,
            })
            .map_err(|_| Status::resource_exhausted("gameplay actor queue full"))?;
        let outcome = tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .map_err(|_| Status::deadline_exceeded("retain your gameplay operation for lookup"))?
            .map_err(|_| Status::unavailable("gameplay actor stopped"))??;
        Ok(Response::new(outcome))
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
        let credential = credential(&request)?;
        let request = request.into_inner();
        let permit = self
            .streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("gameplay watch limit"))?;
        let mut updates = self.updates.clone();
        updates.borrow_and_update();
        let (_, first) = self.view(credential, request.clone()).await?;
        let service = self.clone();
        let stream = futures::stream::unfold(
            (Some(first), updates, service, request, credential, permit),
            |(first, mut updates, service, request, credential, permit)| async move {
                let next = if let Some(first) = first {
                    Ok(first)
                } else {
                    if updates.changed().await.is_err() {
                        return None;
                    }
                    service
                        .view(credential, request.clone())
                        .await
                        .map(|(_, view)| view)
                };
                let failed = next.is_err();
                Some((
                    next,
                    (
                        None,
                        if failed {
                            let (_, closed) = watch::channel(updates.borrow().clone());
                            closed
                        } else {
                            updates
                        },
                        service,
                        request,
                        credential,
                        permit,
                    ),
                ))
            },
        );
        Ok(Response::new(Box::pin(stream)))
    }
}
#[derive(Clone)]
struct PageState {
    incoming: NativeAdmission,
    connections: Arc<Semaphore>,
    allowed_origins: Arc<[String]>,
    display: [u8; 32],
    glue: bytes::Bytes,
    wasm: bytes::Bytes,
}
async fn socket(
    State(state): State<PageState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> axum::response::Response {
    if !headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|origin| {
            state
                .allowed_origins
                .iter()
                .any(|allowed| allowed == origin)
        })
    {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = state.connections.clone().try_acquire_owned() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(admission) = state.incoming.try_reserve() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    upgrade
        .read_buffer_size(FRAME_BYTES)
        .write_buffer_size(FRAME_BYTES)
        .max_write_buffer_size(MESSAGE_BYTES)
        .max_message_size(MESSAGE_BYTES)
        .max_frame_size(MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let metrics = Arc::new(df_rpc_bridge::ConnectionMetrics::new(false));
            let result = admission.accept_websocket_measured(socket, metrics).await;
            let context = OperationContext {
                trace_parent: String::new(),
                build: crate::BUILD_ID.to_owned(),
            };
            let mut span = df_observe::begin(&context, "gameplay.local_connection");
            span.finish_unmeasured(if result.is_ok() {
                "connection_closed"
            } else {
                "connection_failed"
            });
            drop(permit);
        })
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
async fn glue(State(state): State<PageState>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        state.glue,
    )
}
async fn wasm(State(state): State<PageState>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/wasm")],
        state.wasm,
    )
}
fn bounded_asset(path: std::path::PathBuf, maximum: u64) -> Result<bytes::Bytes, io::Error> {
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(io::Error::other("owned asset bound refused"));
    }
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(file, maximum + 1), &mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(io::Error::other("owned asset bound refused"));
    }
    Ok(bytes.into())
}
async fn both(State(state): State<PageState>) -> Html<String> {
    page(false, Some(state.display))
}
async fn player() -> Html<String> {
    page(true, None)
}
async fn display(State(state): State<PageState>) -> Html<String> {
    page(false, Some(state.display))
}
fn page(player: bool, display: Option<[u8; 32]>) -> Html<String> {
    let roots = if player {
        format!(
            "<section class=\"client player\" id=\"player\" data-room=\"{}\"><div class=\"connection\" role=\"status\">Your adventure awaits.</div><div class=\"view\"></div></section>",
            journey::ROOM_CODE
        )
    } else {
        display.map(|credential|format!("<section class=\"client display\" id=\"display\" data-local-binding=\"{}\"><div class=\"connection\" role=\"status\">Connecting to the room…</div><div class=\"view\"></div></section>",hex(&credential))).unwrap_or_default()
    };
    Html(HTML.replace("__ROOTS__", &roots).replace(
        "__LAYOUT__",
        if player {
            "clients player-only"
        } else {
            "clients display-only"
        },
    ))
}
const HTML: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><base href="/"><meta name="viewport" content="width=device-width, initial-scale=1"><title>DungeonFlux — The Broken Seal</title><style>
*{box-sizing:border-box}body{margin:0;background:#080f17;color:#efe6ce;font-family:Georgia,serif}header{padding:22px 5%;display:flex;justify-content:space-between;align-items:center;border-bottom:1px solid #bd96653b;background:#0b1720}header strong{font-size:24px;letter-spacing:3px;color:#eac488}header span,.eyebrow{font:11px system-ui;letter-spacing:2px;text-transform:uppercase;color:#aeb7b4}.clients{display:grid;grid-template-columns:minmax(320px,400px) 1fr;min-height:calc(100vh - 76px);gap:24px;padding:24px 4%}.clients.display-only{grid-template-columns:1fr}.clients.player-only{grid-template-columns:minmax(0,400px);justify-content:center}.client{position:relative;border:1px solid #cba46d4d;border-radius:18px;background:#10202ae8;overflow:hidden;box-shadow:0 15px 50px #0008}.connection{padding:10px 18px;background:#081219;color:#a8c9ba;font:12px system-ui;letter-spacing:.4px}.art{width:100%;height:260px;object-fit:cover;object-position:44% 48%;display:block}.display .art{height:calc(100vh - 200px);min-height:400px;object-position:45% 45%}.scene-copy{padding:24px}.display .scene-copy{position:absolute;bottom:0;right:0;left:0;padding:70px 40px 32px;background:linear-gradient(transparent,#061118f7 45%)}h1,h2{margin:8px 0 14px;font-weight:400;line-height:1.1}h1{font-size:54px}.player h2{font-size:30px}.narration{font-size:17px;line-height:1.5;color:#d1d3c6;margin:12px 0 20px}.display .narration{max-width:640px;font-size:22px}.offered{display:block;background:linear-gradient(135deg,#355447,#20392e);border:1px solid #8cb498;color:#f5eedf;border-radius:10px;width:100%;padding:17px 18px;min-height:56px;text-align:left;font:18px Georgia;cursor:pointer;box-shadow:0 5px 16px #0004}.offered small{display:block;color:#bed0bd;font:12px system-ui;margin-top:8px}.offered:disabled{opacity:.5;cursor:default}.tools{display:flex;flex-wrap:wrap;gap:8px;margin:18px 0 0}.tools button{background:#132530;border:1px solid #7c8a825e;border-radius:7px;padding:11px;color:#d7ddd1;min-height:44px;cursor:pointer}.result{border:1px solid #c59b5f77;border-radius:12px;background:#162630;margin:20px 0;padding:20px}.dice{font-size:40px;color:#f2ce90}.check-label{font:12px system-ui;letter-spacing:1px;color:#9fc1ae}.clue{border-left:2px solid #cbaa67;padding-left:16px;color:#eed69a;line-height:1.6}.receipt{font:11px system-ui;color:#8cb5a0;line-height:1.6}.feedback{font:13px system-ui;color:#d9b681;margin-top:12px}.scope{font:11px system-ui;color:#9ba9a3;line-height:1.5;margin-top:20px}footer{padding:18px 4%;font:11px system-ui;color:#899995;line-height:1.5}footer a{color:#aebfb2}button:focus-visible{outline:3px solid #f1c782;outline-offset:3px}@media(max-width:900px){.clients{grid-template-columns:1fr;max-width:680px;margin:auto}.display .art{height:380px;min-height:0}.display .scene-copy{padding:60px 24px 24px}h1{font-size:36px}.display .narration{font-size:17px}}@media(max-width:420px){header{padding:16px}header strong{font-size:18px}header span{font-size:9px;max-width:110px;text-align:right}.clients{padding:14px 10px;gap:14px}.art{height:185px}.scene-copy{padding:18px}.player h2{font-size:27px}.narration{font-size:15px;margin-bottom:16px}.offered{padding:14px;font-size:17px}.result{padding:16px}.display .art{height:330px}.tools{gap:6px}}@media(prefers-reduced-motion:reduce){*{scroll-behavior:auto}}

.room-code{font-size:38px;letter-spacing:8px;color:#f2ce90;margin:16px 0}.room-share{border:1px solid #caa56c70;background:#071922d9;border-radius:14px;padding:18px;max-width:450px}.join-link{color:#d9e6be}.party{display:flex;gap:8px;flex-wrap:wrap}.party-member{padding:8px 12px;border:1px solid #78968670;border-radius:999px;font:12px system-ui;background:#122c27}.hero-summary{display:grid;gap:8px;padding:16px;border:1px solid #bd966580;border-radius:12px;background:#102c26}.hero-summary strong{font-size:24px;color:#ead2a0}.hero-summary span,.sheet-facts{font:13px system-ui;line-height:1.6}.dialogue{border:1px solid #c6a87970;border-radius:14px;background:#12222ee8;padding:18px;margin:20px 0;min-height:94px}.speaker-portrait{width:60px;height:60px;object-fit:cover;border-radius:50%;float:left;margin:0 14px 8px 0}.dialogue blockquote{margin:8px 0;font-style:italic;font-size:20px;line-height:1.4}.combat-roster{display:flex;gap:8px;flex-wrap:wrap}.combatant{display:grid;gap:8px;padding:12px;background:#12202de8;border:1px solid #778e8050;border-radius:10px;flex:1;min-width:100px;font:12px system-ui}.combatant.active{border-color:#e7bf6f;box-shadow:0 0 20px #e7bf6f22}.hp{width:100%;height:10px;accent-color:#7eb38a}.battle{margin-top:22px}.combat-history{display:grid;gap:10px;margin:16px 0}.combat-result{position:relative;padding:16px 16px 16px 64px;border:1px solid #bba27460;border-radius:12px;background:#10242eea;font:13px system-ui;animation:reveal-result .65s ease-out}.combat-result p{line-height:1.6}.roll-number{position:absolute;left:12px;top:18px;font:30px Georgia;color:#ecc278;animation:reveal-die .7s ease-out}.attack-option{display:grid;grid-template-columns:1fr 90px;gap:12px;margin:12px 0;font:13px system-ui;color:#bfd0b9}.attack-option select{padding:8px;background:#13272b;color:#efe6ce;border:1px solid #a88e66;border-radius:6px}.offered{margin-top:10px}.display .battle{max-width:860px}.display .scene-copy{max-height:75vh;overflow:auto}.display .art{min-height:650px}.player .view>[data-character-phase]{width:100%}.player .character-reference{display:none}@keyframes reveal-result{from{opacity:0;transform:translateY(10px)}to{opacity:1;transform:none}}@keyframes reveal-die{from{transform:rotate(-18deg) scale(.8)}to{transform:none}}@media(prefers-reduced-motion:reduce){.roll-number,.combat-result{animation:none}}
.dialogue-recovery{margin:20px 0;padding:16px;border:1px solid #c59b5f77;border-radius:12px;background:#162630}.dialogue-recovery h3{margin:0 0 6px;font-size:20px;font-weight:400;color:#eac488}.dialogue-recovery .scope{margin:0 0 12px}.dialogue-draft{display:block;width:100%;min-height:96px;padding:12px;background:#13272b;color:#efe6ce;border:1px solid #a88e66;border-radius:8px;font:16px/1.45 Georgia,serif;resize:vertical}.dialogue-recovery select{display:block;width:100%;min-height:44px;margin-top:10px;padding:8px 10px;background:#13272b;color:#efe6ce;border:1px solid #a88e66;border-radius:6px;font:16px system-ui}.dialogue-recovery select:disabled,.dialogue-controls button:disabled{opacity:.5;cursor:default}.dialogue-controls{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}.dialogue-controls button{flex:1 1 140px;min-height:44px;padding:10px 12px;background:#132530;border:1px solid #7c8a825e;border-radius:7px;color:#d7ddd1;font:14px system-ui;cursor:pointer}.dialogue-controls button:first-child{background:linear-gradient(135deg,#355447,#20392e);border-color:#8cb498;color:#f5eedf}.dialogue-feedback{margin:10px 0 0;font:13px system-ui;line-height:1.5;color:#d9b681}.dialogue-feedback:empty{display:none}.dialogue-draft:focus-visible,.dialogue-recovery select:focus-visible{outline:3px solid #f1c782;outline-offset:2px}.recovery-needed{margin:18px 0 0;padding:14px;border:1px solid #d9b68199;border-radius:12px;background:#2a2417}.recovery-needed p{margin:0 0 10px;font:13px system-ui;line-height:1.5;color:#eed69a}.recovery-needed .tools{margin:0}.recovery-needed .tools button{background:linear-gradient(135deg,#355447,#20392e);border-color:#8cb498;color:#f5eedf}.trouble{margin:22px 0 0;font:13px system-ui;color:#9ba9a3}.trouble summary{cursor:pointer;padding:12px 0}.trouble .tools{margin-top:0}
</style></head><body><header><strong>DUNGEONFLUX</strong><span>The Lantern Wharf · your shared adventure</span></header><main class="__LAYOUT__">__ROOTS__</main><footer>Local browser journey · player-created Dwarf Fighter / Soldier · selected SRD 5.2.1 rules. Other character options and mechanics remain in development. Authored concept art illustrates the setting; character artwork is not generated here. This work includes material from the System Reference Document 5.2.1 (“SRD 5.2.1”) by Wizards of the Coast LLC, available at <a href="https://www.dndbeyond.com/srd">dndbeyond.com/srd</a>. The SRD 5.2.1 is licensed under the <a href="https://creativecommons.org/licenses/by/4.0/legalcode">Creative Commons Attribution 4.0 International License</a>.</footer><script type="module">import init from '/pkg/df_tools.js'; await init();</script></body></html>"#;

/// Run the gameplay demonstration using an already owned empty PG cluster.
///
/// A plain `--gameplay-demo <web> <assets>` run is interactive: it listens and accepts
/// browser origins as configured by the `DF_GAMEPLAY_*` variables read by
/// [`startup_configuration::session_settings`], runs until interrupted unless a session
/// length is set, and prints one `READY` line. A qualification flag as the fourth
/// argument keeps the fixed loopback address, 290-second limit, and small connection,
/// stream, and call bounds the qualification harnesses were written against.
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let web_root = std::env::args()
        .nth(2)
        .ok_or("generated gameplay web directory required")?;
    let asset_root = std::env::args()
        .nth(3)
        .ok_or("fixed gameplay asset directory required")?;
    let startup_configuration::StartupAssets {
        glue: glue_bytes,
        wasm: wasm_bytes,
        art: art_bytes,
        campfire: campfire_bytes,
        portrait: portrait_bytes,
        inn: inn_bytes,
    } = startup_configuration::load_assets(
        std::path::Path::new(&web_root),
        std::path::Path::new(&asset_root),
    )?;
    let interactive = std::env::args().nth(4).is_none();
    let settings = if interactive {
        startup_configuration::session_settings(
            startup_configuration::SessionInputs::from_environment(),
        )?
    } else {
        startup_configuration::SessionSettings::qualification()
    };
    let npc_phase = courier_npc_qualification::phase();
    let npc_budget = npc_phase
        .map(courier_npc_qualification::call_budget)
        .transpose()?;
    let inn_phase = inn_qualification::phase();
    let inn_budget = inn_phase.map(inn_qualification::call_budget).transpose()?;
    let courier_phase = courier_process_qualification::phase();
    let courier_budget = courier_phase
        .map(courier_process_qualification::call_budget)
        .transpose()?;
    let restart_phase = restart_qualification::phase();
    let restart_budget = restart_phase
        .map(restart_qualification::call_budget)
        .transpose()?;
    let rest_phase = rest_qualification::phase();
    let rest_budget = rest_phase
        .map(rest_qualification::call_budget)
        .transpose()?;
    let cold =
        journey::initial().map_err(|_| io::Error::other("gameplay baseline validation failed"))?;
    let codec = NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 1024,
        maximum_text_bytes: 4096,
    };
    let recovery =
        model::recovery().map_err(|_| io::Error::other("source recovery inventory invalid"))?;
    let fence = model::random::<16>().map_err(|_| io::Error::other("native fence unavailable"))?;
    let player_credential =
        model::random::<32>().map_err(|_| io::Error::other("local grant unavailable"))?;
    let display_credential =
        model::random::<32>().map_err(|_| io::Error::other("local grant unavailable"))?;
    let database = database_config()?;
    let (mut grant_client, grant_connection) = tokio::time::timeout(
        Duration::from_secs(2),
        database.connect(tokio_postgres::NoTls),
    )
    .await
    .map_err(|_| io::Error::other("grant connect handshake deadline"))??;
    let grant_driver = tokio::spawn(grant_connection);
    let initialization = tokio::time::timeout(
        Duration::from_secs(5),
        df_persistence::local_demo_scope::admit_startup(
            &mut grant_client,
            &cold,
            df_persistence::local_demo_scope::DemoStartupIdentity {
                fence,
                player_credential,
                display_credential,
            },
            codec,
            &recovery,
            |checkpoint| {
                df_session::submission::SessionEngine::<
                    df_persistence::NativeScope<LocalDemoAuthority>,
                >::validate_recovery(&mut model::HarborEngine, checkpoint)
            },
        ),
    )
    .await;
    let admitted = match initialization {
        Ok(Ok(admitted)) => admitted,
        failure => {
            let refusal_report = if restart_phase == Some(restart_qualification::Phase::Contender)
                && let Ok(Err(error)) = &failure
            {
                restart_qualification::refusal(error)
            } else if rest_phase == Some(rest_qualification::Phase::Contender)
                && let Ok(Err(error)) = &failure
            {
                rest_qualification::refusal(error)
            } else {
                Ok(())
            };
            drop(grant_client);
            join_grant_driver(grant_driver).await?;
            refusal_report?;
            return Err(io::Error::other(format!(
                "owned gameplay admission refused: {}",
                match failure {
                    Ok(Err(error)) => format!("{} {:?}", error.stage, error.class),
                    _ => "startup_timeout".to_owned(),
                }
            ))
            .into());
        }
    };
    if let Some(phase) = restart_phase {
        let expected = phase == restart_qualification::Phase::B;
        if admitted.restored != expected {
            let released = df_persistence::local_demo_scope::release_owner(
                &grant_client,
                admitted.checkpoint.basis().session,
                fence,
            )
            .await;
            drop(grant_client);
            join_grant_driver(grant_driver).await?;
            released.map_err(|_| io::Error::other("wrong-phase exact-fence release failed"))?;
            return Err(io::Error::other("restart phase admission kind refused").into());
        }
    }
    if let Some(phase) = rest_phase
        && admitted.restored != (phase == rest_qualification::Phase::B)
    {
        let released = df_persistence::local_demo_scope::release_owner(
            &grant_client,
            admitted.checkpoint.basis().session,
            fence,
        )
        .await;
        drop(grant_client);
        join_grant_driver(grant_driver).await?;
        released.map_err(|_| io::Error::other("rest wrong-phase exact-fence release failed"))?;
        return Err(io::Error::other("rest phase admission kind refused").into());
    }
    if let Some(phase) = courier_phase
        && admitted.restored != (phase != courier_process_qualification::Phase::A)
    {
        let released = df_persistence::local_demo_scope::release_owner(
            &grant_client,
            admitted.checkpoint.basis().session,
            fence,
        )
        .await;
        drop(grant_client);
        join_grant_driver(grant_driver).await?;
        released.map_err(|_| io::Error::other("courier wrong-phase exact-fence release failed"))?;
        return Err(io::Error::other("courier phase admission kind refused").into());
    }
    if let Some(phase) = npc_phase
        && admitted.restored != (phase == courier_npc_qualification::Phase::B)
    {
        let released = df_persistence::local_demo_scope::release_owner(
            &grant_client,
            admitted.checkpoint.basis().session,
            fence,
        )
        .await;
        drop(grant_client);
        join_grant_driver(grant_driver).await?;
        released
            .map_err(|_| io::Error::other("courier NPC wrong-phase exact-fence release failed"))?;
        return Err(io::Error::other("courier NPC phase admission kind refused").into());
    }
    if let Some(phase) = inn_phase
        && admitted.restored != (phase == inn_qualification::Phase::B)
    {
        let released = df_persistence::local_demo_scope::release_owner(
            &grant_client,
            admitted.checkpoint.basis().session,
            fence,
        )
        .await;
        drop(grant_client);
        join_grant_driver(grant_driver).await?;
        released.map_err(|_| io::Error::other("inn wrong-phase exact-fence release failed"))?;
        return Err(io::Error::other("inn phase admission kind refused").into());
    }
    let initial = admitted.checkpoint;
    let player_credential = admitted.player_credential;
    let display_credential = admitted.display_credential;
    let runtime = tokio::runtime::Handle::current();
    let (client, connection) = match tokio::time::timeout(
        Duration::from_secs(2),
        database.connect(tokio_postgres::NoTls),
    )
    .await
    {
        Ok(Ok(connected)) => connected,
        failure => {
            let released = df_persistence::local_demo_scope::release_owner(
                &grant_client,
                initial.basis().session,
                fence,
            )
            .await;
            drop(grant_client);
            join_grant_driver(grant_driver).await?;
            released.map_err(|_| io::Error::other("setup exact-fence release failed"))?;
            return Err(io::Error::other(match failure {
                Ok(Err(_)) => "repository connect handshake refused",
                _ => "repository connect handshake deadline",
            })
            .into());
        }
    };
    let context = OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let repository_options = NativeRepositoryOptions {
        transaction_bounds: NativeTransactionBounds {
            transaction: Duration::from_secs(2),
            rollback: Duration::from_millis(250),
            driver_join: Duration::from_secs(2),
        },
        codec_limits: codec,
        maximum_receipt_bytes: 4096,
        verifier: Some(df_persistence::local_demo_scope::verifier()),
        recovery: Some(recovery),
    };
    let repository_setup = PostgresRepository::<LocalDemoAuthority>::from_connected_no_tls(
        runtime.clone(),
        client,
        connection,
        repository_options.clone(),
        &context,
    )
    .await;
    let mut repository = match repository_setup {
        Ok(repository) => repository,
        Err(mut failure) => {
            let closed = failure.close().await;
            let released = df_persistence::local_demo_scope::release_owner(
                &grant_client,
                initial.basis().session,
                fence,
            )
            .await;
            drop(grant_client);
            join_grant_driver(grant_driver).await?;
            closed.map_err(|_| io::Error::other("native setup driver close failed"))?;
            released.map_err(|_| io::Error::other("setup exact-fence release failed"))?;
            return Err(io::Error::other("native repository setup refused").into());
        }
    };
    if repository
        .configure_reconnect(database.clone(), repository_options)
        .is_err()
    {
        let closed = repository.close_owned().await;
        let released = df_persistence::local_demo_scope::release_owner(
            &grant_client,
            initial.basis().session,
            fence,
        )
        .await;
        drop(grant_client);
        join_grant_driver(grant_driver).await?;
        closed.map_err(|_| io::Error::other("native setup driver close failed"))?;
        released.map_err(|_| io::Error::other("setup exact-fence release failed"))?;
        return Err(io::Error::other("native recovery configuration refused").into());
    }
    let listener = match tokio::net::TcpListener::bind(settings.listen).await {
        Ok(listener) => listener,
        Err(error) => {
            let closed = repository.close_owned().await;
            let released = df_persistence::local_demo_scope::release_owner(
                &grant_client,
                initial.basis().session,
                fence,
            )
            .await;
            drop(grant_client);
            join_grant_driver(grant_driver).await?;
            closed.map_err(|_| io::Error::other("native listener failure driver close failed"))?;
            released
                .map_err(|_| io::Error::other("listener failure exact-fence release failed"))?;
            return Err(error.into());
        }
    };
    let mut issuer = match LocalDemoScopeIssuer::new(
        runtime.clone(),
        grant_client,
        initial.basis().session,
        fence,
    ) {
        Ok(issuer) => issuer,
        Err(_) => {
            // The rejected constructor drops its client; observe its original driver
            // and the repository even though startup has already checked the fence.
            let closed = repository.close_owned().await;
            let joined = join_grant_driver(grant_driver).await;
            closed.map_err(|_| io::Error::other("issuer construction repository close failed"))?;
            joined?;
            return Err(io::Error::other("local issuer construction refused").into());
        }
    };
    if let Err((_, driver)) = issuer.configure_reconnect(database.clone(), grant_driver) {
        let released = issuer.release_owner_owned().await;
        let closed = repository.close_owned().await;
        let grants = issuer.close_owned().await;
        join_grant_driver(driver).await?;
        released.map_err(|_| io::Error::other("issuer setup exact-fence release failed"))?;
        closed.map_err(|_| io::Error::other("issuer setup repository close failed"))?;
        grants.map_err(|_| io::Error::other("issuer setup grant close failed"))?;
        return Err(io::Error::other("issuer recovery configuration refused").into());
    }

    let (updates, receiver) = watch::channel(initial.clone());
    let (publication, intent_notifications) = actor::Publication::new(updates.clone());
    let courier_reductions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let owner = match DurableOwner::new(
        repository,
        actor::Engine(courier_phase.map(|_| courier_reductions.clone())),
        publication,
        initial,
        4096,
    ) {
        Ok(owner) => owner,
        Err(_) => {
            let released = issuer.release_owner_owned().await;
            let closed = issuer.close_owned().await;
            released.map_err(|_| io::Error::other("owner setup exact-fence release failed"))?;
            closed.map_err(|_| io::Error::other("owner setup grant close failed"))?;
            return Err(io::Error::other("canonical owner setup refused").into());
        }
    };
    let (sender, inbox) = bounded_inbox::<actor::Call>();
    let call_budget = npc_budget
        .or(inn_budget)
        .or(courier_budget)
        .or(restart_budget)
        .or(rest_budget)
        .or(settings.call_budget);
    let actor = actor::Actor {
        owner,
        dialogue: dialogue::DialogueState::default(),
        bootstrap_credential: player_credential,
        issuer,
        codec,
        fenced: false,
        recovery_wakeup: updates,
        intent_notifications,
        calls_remaining: call_budget.unwrap_or(u16::MAX),
        calls_unbounded: call_budget.is_none(),
        qualification_joins: if npc_phase.is_some()
            || inn_phase.is_some()
            || courier_phase.is_some()
            || rest_phase.is_some()
            || restart_phase.is_some()
            || std::env::args().nth(4).as_deref() == Some("--qualification")
        {
            Some(Vec::new())
        } else {
            None
        },
        completion_retry: None,
        #[cfg(test)]
        completion_observer: None,
        qualification_inputs: if npc_phase.is_some()
            || inn_phase.is_some()
            || courier_phase.is_some()
            || rest_phase.is_some()
            || restart_phase.is_some()
            || std::env::args().nth(4).as_deref() == Some("--qualification")
        {
            Some(Vec::new())
        } else {
            None
        },
    };
    let (startup_send, startup_receive) = std::sync::mpsc::sync_channel::<actor::Actor>(1);
    let courier_database = database.clone();
    let courier_runtime = runtime.clone();
    let courier_thread_reductions = courier_reductions.clone();
    let thread_setup = std::thread::Builder::new()
        .name("df-real-gameplay-owner".to_owned())
        .spawn(move || {
            let mut actor = startup_receive
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| io::Error::other("actor startup handoff deadline"))?;
            actor.run_committed_intents();
            let drained = inbox.run_with_owner_wake(
                &mut actor,
                actor::Actor::next_completion_wake,
                actor::Actor::wake_completion,
            );
            let courier_committed = if drained.is_ok() {
                courier_phase
                    .map(|phase| {
                        courier_process_qualification::after_drained(
                            phase,
                            &mut actor,
                            &courier_database,
                            &courier_runtime,
                            courier_thread_reductions.as_ref(),
                        )
                    })
                    .transpose()
                    .map(|_| ())
            } else {
                Err(io::Error::other("courier requires drained inbox"))
            };
            let released = if drained.is_ok() {
                actor.issuer.release_owner()
            } else {
                Err(df_session::submission::RepositoryError::Unavailable)
            };
            let remaining = actor.calls_remaining;
            let mut repository = actor.owner.into_repository();
            let closed = repository.close();
            let grant_closed = actor.issuer.close();
            drop(actor.issuer);
            Ok::<_, io::Error>((
                drained,
                released,
                closed,
                grant_closed,
                remaining,
                courier_committed,
            ))
        });
    let thread = match thread_setup {
        Ok(thread) => thread,
        Err(error) => {
            close_unstarted_actor(actor).await?;
            return Err(error.into());
        }
    };
    if let Err(error) = startup_send.send(actor) {
        let closed = close_unstarted_actor(error.0).await;
        let joined = tokio::task::spawn_blocking(move || thread.join()).await?;
        closed?;
        if joined
            .map_err(|_| io::Error::other("failed actor startup thread join failed"))?
            .is_ok()
        {
            return Err(io::Error::other("failed handoff unexpectedly ran actor").into());
        }
        return Err(io::Error::other("actor startup handoff refused").into());
    }
    let service = Service {
        actor: sender.clone(),
        updates: receiver,
        streams: Arc::new(Semaphore::new(settings.max_watch_streams)),
    };
    let (incoming_sender, incoming) =
        NativeIncoming::bounded(NonZeroUsize::new(4).ok_or("invalid native incoming bound")?)?;
    let grpc = tonic::transport::Server::builder()
        .initial_stream_window_size(MESSAGE_BYTES as u32)
        .initial_connection_window_size(df_rpc_bridge::RECEIVE_BYTES as u32)
        .max_frame_size(FRAME_BYTES as u32)
        .http2_max_header_list_size(FRAME_BYTES as u32)
        .max_concurrent_streams(CONCURRENT_STREAMS)
        .add_service(
            rpc::dialogue_service_server::DialogueServiceServer::new(service.clone())
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        )
        .add_service(
            rpc::action_service_server::ActionServiceServer::new(service.clone())
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        )
        .add_service(
            rpc::session_service_server::SessionServiceServer::new(service.clone())
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
        incoming: incoming_sender,
        connections: Arc::new(Semaphore::new(settings.max_connections)),
        allowed_origins: settings.allowed_origins.clone().into(),
        display: display_credential,
        glue: glue_bytes,
        wasm: wasm_bytes,
    };
    let router = Router::new()
        .route("/gameplay", get(both))
        .route("/gameplay/player", get(player))
        .route("/gameplay/display", get(display))
        .route("/gameplay/tunnel", get(socket))
        .route("/pkg/df_tools.js", get(glue))
        .route("/pkg/df_tools_bg.wasm", get(wasm))
        .with_state(state)
        .merge(public_assets::router(public_assets::PublicAssets {
            art: art_bytes,
            campfire: campfire_bytes,
            portrait: portrait_bytes,
            inn: inn_bytes,
        }))
        .merge(public_assets::extra_router(
            startup_configuration::load_extra_assets(std::path::Path::new(&asset_root))?,
        ));
    let npc = npc_phase.map(|phase| {
        tokio::spawn(courier_npc_qualification::run(
            service.clone(),
            display_credential,
            codec,
            database.clone(),
            phase,
            fence,
        ))
    });
    let inn = inn_phase.map(|phase| {
        tokio::spawn(inn_qualification::run(
            service.clone(),
            display_credential,
            codec,
            database.clone(),
            phase,
            fence,
        ))
    });
    let courier = courier_phase.map(|phase| {
        tokio::spawn(courier_process_qualification::run(
            service.clone(),
            display_credential,
            codec,
            database.clone(),
            phase,
            courier_reductions.clone(),
        ))
    });
    let restart = restart_phase.map(|phase| {
        tokio::spawn(restart_qualification::run(
            service.clone(),
            display_credential,
            codec,
            database.clone(),
            phase,
            fence,
        ))
    });
    let rest = rest_phase.map(|phase| {
        tokio::spawn(rest_qualification::run(
            service.clone(),
            display_credential,
            codec,
            database.clone(),
            phase,
            fence,
        ))
    });
    let qualification = if std::env::args().nth(4).as_deref() == Some("--qualification") {
        let (cancel, cancelled) = oneshot::channel();
        Some((
            cancel,
            tokio::spawn(qualification::run(
                service,
                display_credential,
                cancelled,
                database,
            )),
        ))
    } else {
        None
    };
    if interactive {
        let origin = &settings.allowed_origins[0];
        println!("READY display={origin}/gameplay/display player={origin}/gameplay/player");
    }
    let outcome: Result<(), Box<dyn std::error::Error>> = {
        tokio::select! {
            result=grpc=>result.map_err(Into::into),result=axum::serve(listener,router)=>result.map_err(Into::into),
            result=tokio::signal::ctrl_c()=>result.map_err(Into::into),
            _=async{match settings.session{Some(session)=>tokio::time::sleep(session).await,None=>std::future::pending().await}}=>Ok(()),
        }
    };
    let qualification_outcome = if let Some((cancel, task)) = qualification {
        let _ = cancel.send(());
        match task.await {
            Ok(result) => result,
            Err(_) => Err(io::Error::other("qualification consumer did not finish")),
        }
    } else {
        Ok(())
    };
    let courier_outcome = if let Some(mut task) = courier {
        match tokio::time::timeout(Duration::from_secs(47), &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(io::Error::other("courier process consumer panicked")),
            Err(_) => {
                task.abort();
                let joined = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
                Err(io::Error::other(if joined.is_ok() {
                    "courier consumer deadline; aborted task joined"
                } else {
                    "courier consumer deadline; join pending"
                }))
            }
        }
    } else {
        Ok(())
    };
    let restart_outcome = if let Some(mut task) = restart {
        match tokio::time::timeout(Duration::from_secs(47), &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(io::Error::other(
                "restart consumer panicked; cleanup required",
            )),
            Err(_) => {
                task.abort();
                let joined = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
                Err(io::Error::other(if joined.is_ok() {
                    "restart consumer deadline; aborted task joined"
                } else {
                    "restart consumer deadline; join pending"
                }))
            }
        }
    } else {
        Ok(())
    };
    let rest_outcome = if let Some(mut task) = rest {
        match tokio::time::timeout(Duration::from_secs(47), &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(io::Error::other("rest consumer panicked; cleanup required")),
            Err(_) => {
                task.abort();
                let joined = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
                Err(io::Error::other(if joined.is_ok() {
                    "rest consumer deadline; aborted task joined"
                } else {
                    "rest consumer deadline; join pending"
                }))
            }
        }
    } else {
        Ok(())
    };
    let npc_outcome = if let Some(mut task) = npc {
        match tokio::time::timeout(Duration::from_secs(47), &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(io::Error::other(
                "courier NPC consumer panicked; cleanup required",
            )),
            Err(_) => {
                task.abort();
                let joined = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
                Err(io::Error::other(if joined.is_ok() {
                    "courier NPC consumer deadline; aborted task joined"
                } else {
                    "courier NPC consumer deadline; join pending"
                }))
            }
        }
    } else {
        Ok(())
    };
    let inn_outcome = if let Some(mut task) = inn {
        match tokio::time::timeout(Duration::from_secs(47), &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(io::Error::other("inn consumer panicked; cleanup required")),
            Err(_) => {
                task.abort();
                let joined = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
                Err(io::Error::other(if joined.is_ok() {
                    "inn consumer deadline; aborted task joined"
                } else {
                    "inn consumer deadline; join pending"
                }))
            }
        }
    } else {
        Ok(())
    };
    sender
        .stop()
        .map_err(|_| io::Error::other("gameplay inbox stop failed"))?;
    let (drained, released, closed, grant_closed, remaining, courier_committed) =
        tokio::task::spawn_blocking(move || thread.join())
            .await?
            .map_err(|_| io::Error::other("gameplay actor join failed"))??;
    drained.map_err(|_| io::Error::other("gameplay actor drain failed"))?;
    released.map_err(|_| io::Error::other("gameplay exact-fence release failed"))?;
    closed.map_err(|_| io::Error::other("gameplay repository close failed"))?;
    grant_closed.map_err(|_| io::Error::other("gameplay grant close failed"))?;
    courier_committed?;
    courier_outcome?;
    qualification_outcome?;
    restart_outcome?;
    rest_outcome?;
    npc_outcome?;
    inn_outcome?;
    if let Some(phase) = npc_phase {
        courier_npc_qualification::closed(phase, remaining)?;
    }
    if let Some(phase) = inn_phase {
        inn_qualification::closed(phase, remaining)?;
    }
    if let Some(phase) = restart_phase {
        restart_qualification::closed(phase, remaining)?;
    }
    if let Some(phase) = rest_phase {
        rest_qualification::closed(phase, remaining)?;
    }
    if let Some(phase) = courier_phase {
        courier_process_qualification::closed(
            phase,
            remaining,
            courier_reductions.load(std::sync::atomic::Ordering::SeqCst),
        )?;
    }
    outcome
}

async fn close_unstarted_actor(mut actor: actor::Actor) -> Result<(), io::Error> {
    let released = actor.issuer.release_owner_owned().await;
    let mut repository = actor.owner.into_repository();
    let closed = repository.close_owned().await;
    let joined = actor.issuer.close_owned().await;
    released.map_err(|_| io::Error::other("unstarted actor exact-fence release failed"))?;
    closed.map_err(|_| io::Error::other("unstarted actor repository close failed"))?;
    joined.map_err(|_| io::Error::other("unstarted actor grant close failed"))
}

async fn join_grant_driver(
    mut driver: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) -> Result<(), io::Error> {
    match tokio::time::timeout(Duration::from_secs(2), &mut driver).await {
        Ok(result) => result
            .map_err(|_| io::Error::other("grant driver join failed"))?
            .map_err(|_| io::Error::other("grant driver protocol failed")),
        Err(_) => {
            driver.abort();
            let joined = tokio::time::timeout(Duration::from_secs(2), &mut driver).await;
            Err(io::Error::other(if joined.is_ok() {
                "grant driver deadline; aborted task joined"
            } else {
                "grant driver deadline; join pending"
            }))
        }
    }
}

fn database_config() -> Result<tokio_postgres::Config, io::Error> {
    startup_configuration::database_config(std::env::var("DF_GAMEPLAY_DEMO_DATABASE"))
}
