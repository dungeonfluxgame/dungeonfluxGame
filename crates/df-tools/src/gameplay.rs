//! A finite, explicitly local gameplay slice. Authentication is preconfigured development
//! membership, not production bootstrap. The action and its outcome use generated RPC.
mod actor;
mod adventure;
mod beat_script;
mod beat_script_runtime;
#[cfg(test)]
mod beat_script_tests;
#[cfg(test)]
mod beat_script_variety_tests;
#[cfg(test)]
mod connection_tests;
#[cfg(test)]
mod content_rating_live_tests;
mod courier_ai;
mod courier_npc_qualification;
mod courier_process_qualification;
mod dialogue;
#[cfg(test)]
mod dialogue_tests;
mod health_faces;
mod home_theme;
#[cfg(test)]
mod home_theme_tests;
mod imagery;
mod inn_qualification;
mod join_link;
mod journey;
mod living_scenes;
mod model;
mod music_assets;
mod npc_dossier;
#[cfg(test)]
mod npc_dossier_live_tests;
#[cfg(test)]
mod npc_dossier_tests;
mod npc_facts;
mod npc_reply;
#[cfg(test)]
mod npc_talk_live_tests;
#[cfg(test)]
mod npc_talk_rules;
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
mod room_code;
mod scene_mood;
mod session_recovery;
mod social;
mod speech_input;
mod speech_text;
mod splat_assets;
mod startup_configuration;
#[cfg(test)]
mod startup_contract;
mod storyteller;
mod table_log;
#[cfg(test)]
mod tempo_clock;
mod timing;
mod voice;
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
use df_persistence::{NativeCodecLimits, NativeRepositoryOptions, PostgresRepository};
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
    voice: voice::Voice,
    /// Speaks for the scene's NPC when the storyteller is on; silent otherwise.
    npc_voice: Arc<npc_reply::NpcVoice>,
    /// The public feed of who said and did what; a new line publishes a fresh view.
    table_log: Arc<table_log::TableLog>,
    /// Follows the story's mood with generated scene art; presentation only.
    living: living_scenes::LivingScenes,
}
fn credential<T>(request: &Request<T>) -> Result<[u8; 32], Status> {
    let value = request
        .metadata()
        .get("x-df-local-binding")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Status::unauthenticated("local binding required"))?;
    parse_binding(value).ok_or_else(|| Status::unauthenticated("local binding invalid"))
}
/// The 32 bytes of a lowercase hexadecimal room binding, if `value` is exactly that.
fn parse_binding(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut result = [0; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |byte| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        result[index] = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Some(result)
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
        let viewed = tokio::time::timeout(timing::active().reply, wait)
            .await
            .map_err(|_| Status::deadline_exceeded("gameplay view deadline"))?
            .map_err(|_| Status::unavailable("gameplay actor stopped"))?;
        // Every view leaves the server through here, so this is where spoken lines are
        // registered; only the display audience is voiced.
        if let Ok((_, message)) = &viewed {
            self.voice.register_view(message);
        }
        // Living scenes watch the display's view and then add their published art to it; the
        // art is presentation only and nothing else in the view changes.
        viewed.map(|(role, mut message)| {
            self.living.observe(&message);
            self.living.apply(&mut message);
            (role, message)
        })
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
        let outcome = tokio::time::timeout(timing::active().reply, wait)
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
        let mut table_lines = self.table_log.subscribe();
        table_lines.borrow_and_update();
        // A published scene image wakes a watcher the way a new table line does.
        let mut art = self.living.subscribe();
        if let Some(art) = art.as_mut() {
            art.borrow_and_update();
        }
        let (_, first) = self.view(credential, request.clone()).await?;
        // The display shows the generated scene art and every screen shows the heroes' health
        // faces, so every stream wakes when published art changes.
        let service = self.clone();
        let stream = futures::stream::unfold(
            (
                Some(first),
                updates,
                table_lines,
                art,
                service,
                request,
                credential,
                permit,
            ),
            |(
                first,
                mut updates,
                mut table_lines,
                mut art,
                service,
                request,
                credential,
                permit,
            )| async move {
                let next = if let Some(first) = first {
                    Ok(first)
                } else {
                    // A checkpoint change, a new table line or a newly published scene image each
                    // publish a fresh view.
                    let ended = tokio::select! {
                        changed = updates.changed() => changed.is_err(),
                        changed = table_lines.changed() => changed.is_err(),
                        () = scene_art_changed(&mut art) => false,
                    };
                    if ended {
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
                        table_lines,
                        art,
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
/// Completes when the published scene art changes; a service without living scenes (or whose
/// channel has closed) never completes.
async fn scene_art_changed(art: &mut Option<watch::Receiver<u64>>) {
    if let Some(receiver) = art
        && receiver.changed().await.is_ok()
    {
        return;
    }
    std::future::pending::<()>().await
}
#[derive(Clone)]
struct PageState {
    run_identity: Arc<str>,
    incoming: NativeAdmission,
    connections: Arc<Semaphore>,
    allowed_origins: Arc<[String]>,
    display: [u8; 32],
    glue: bytes::Bytes,
    wasm: bytes::Bytes,
    /// `DF_GAMEPLAY_BATTLE_SCENE`: a scene id the display uses for every fight, when set.
    battle_scene: Option<Arc<str>>,
    /// `DF_GAMEPLAY_BATTLE_FPS`: the battlefield's fixed presentation rate (24 or 30).
    battle_fps: u32,
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
/// Who speaks in the installed pack, for voice casting. A speaker whose pack entry carries no
/// gender is left out, so it is voiced by the roster's neutral fallback and never guessed.
fn speaker_castings() -> Vec<voice::SpeakerCasting> {
    pack::speakers()
        .into_iter()
        .filter_map(|speaker| {
            Some(voice::SpeakerCasting {
                name: speaker.name,
                gender: match speaker.gender? {
                    pack::Gender::He => voice::Gender::He,
                    pack::Gender::She => voice::Gender::She,
                    pack::Gender::They => voice::Gender::They,
                    pack::Gender::It => voice::Gender::It,
                },
                age: speaker.age.map(|age| match age {
                    pack::Age::Young => voice::Age::Young,
                    pack::Age::Adult => voice::Age::Adult,
                    pack::Age::Elder => voice::Age::Elder,
                }),
                tone: speaker.voice,
            })
        })
        .collect()
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
    page(
        false,
        Some(state.display),
        state.battle_scene.as_deref(),
        state.battle_fps,
    )
}
async fn player() -> Html<String> {
    page(true, None, None, df_render::battle::BattleFps::DEFAULT.hz())
}
async fn display(State(state): State<PageState>) -> Html<String> {
    page(
        false,
        Some(state.display),
        state.battle_scene.as_deref(),
        state.battle_fps,
    )
}
fn page(
    player: bool,
    display: Option<[u8; 32]>,
    battle_scene: Option<&str>,
    battle_fps: u32,
) -> Html<String> {
    // The browser waits a little longer than the server for any one request; both come from the
    // same setting (see `timing`).
    let wait = timing::active().browser_request().as_millis();
    let roots = if player {
        format!(
            "<section class=\"client player\" id=\"player\" data-room=\"{}\" data-request-timeout-ms=\"{wait}\"><div class=\"view\"></div></section>",
            journey::ROOM_CODE
        )
    } else {
        display.map(|credential|format!("<section class=\"client display\" id=\"display\" data-local-binding=\"{}\" data-request-timeout-ms=\"{wait}\" data-battle-fps=\"{battle_fps}\"{}><div class=\"view\"></div></section>",hex(&credential),battle_scene.map(|scene|format!(" data-battle-scene=\"{scene}\"")).unwrap_or_default())).unwrap_or_default()
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
/// A minimal shell: the Rust client mounts and styles everything (see `live_theme`).
const HTML: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><base href="/"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"><title>DungeonFlux</title><style>*{box-sizing:border-box}html,body{margin:0;background:#05090f;color:#efe6ce;font-family:Georgia,serif}</style></head><body><main class="__LAYOUT__">__ROOTS__</main><script type="module">import init from '/pkg/df_tools.js'; await init();</script></body></html>"#;

/// Attribution reachable from the player's menu only. The SRD wording is unchanged.
const CREDITS: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>DungeonFlux credits</title><style>*{box-sizing:border-box}body{margin:0;background:#05090f;color:#efe6ce;font:17px/1.6 "Iowan Old Style","Palatino Linotype",Palatino,Georgia,serif}main{max-width:44rem;margin:0 auto;padding:32px 20px 64px}h1{font-weight:400;font-size:2rem;color:#e3c68f;margin:0 0 4px}p{margin:16px 0}a{color:#d9e6be}.back{display:inline-block;margin-top:24px;min-height:44px;padding:10px 0}</style></head><body><main><h1>Credits</h1><p>Local browser journey · player-created Dwarf Fighter / Soldier · selected SRD 5.2.1 rules. Other character options and mechanics remain in development.</p><p>Authored concept art illustrates the setting. Illustrative artwork includes AI-generated images (the wordmark, frames, icons, portraits and scene paintings); it adds no gameplay requirements. The concept art comes from the ShellHacks 2026 DungeonFlux prototype.</p><p>This work includes material from the System Reference Document 5.2.1 (“SRD 5.2.1”) by Wizards of the Coast LLC, available at <a href="https://www.dndbeyond.com/srd">dndbeyond.com/srd</a>. The SRD 5.2.1 is licensed under the <a href="https://creativecommons.org/licenses/by/4.0/legalcode">Creative Commons Attribution 4.0 International License</a>.</p><a class="back" href="/gameplay/player">Back to the game</a></main></body></html>"#;
async fn credits() -> Html<&'static str> {
    Html(CREDITS)
}

/// Run the gameplay demonstration using an already owned empty PG cluster.
///
/// A plain `--gameplay-demo <web> <assets>` run is interactive: it listens and accepts
/// browser origins as configured by the `DF_GAMEPLAY_*` variables read by
/// [`startup_configuration::session_settings`], runs until interrupted unless a session
/// length is set, and prints one `READY` line. A qualification flag as the fourth
/// argument keeps the fixed loopback address, 290-second limit, and small connection,
/// stream, and call bounds the qualification harnesses were written against.
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(command) = std::env::args_os().nth(1)
        && (command == "--campaign-run" || command == "--campaign-status")
    {
        return adventure::cli::campaign_run(
            &std::env::args_os().skip(2).collect::<Vec<_>>(),
            command == "--campaign-status",
        );
    }
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--roll-adventure")
    {
        return adventure::cli::run(&std::env::args_os().skip(2).collect::<Vec<_>>());
    }
    if let Some(command) = std::env::args_os().nth(1)
        && (command == "--roll-campaign" || command == "--simulate-campaign")
    {
        return adventure::cli::campaign(
            &std::env::args_os().skip(2).collect::<Vec<_>>(),
            command == "--simulate-campaign",
        );
    }
    let interactive = std::env::args().nth(4).is_none();
    let adventure_settings = if interactive {
        Some(startup_configuration::adventure_settings(
            startup_configuration::AdventureInputs::from_environment(),
            &std::env::current_dir()?,
        )?)
    } else {
        None
    };
    let mut generated_adventure: Option<adventure::startup::PreparedAdventure> = None;
    // Names the server run for pages that keep a seat; empty (no check) in qualification.
    let mut run_identity = String::new();
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
        combat: combat_bytes,
    } = startup_configuration::load_assets(
        std::path::Path::new(&web_root),
        std::path::Path::new(&asset_root),
    )?;
    home_theme::install(std::path::Path::new(&asset_root))?;
    let splat_directory = startup_configuration::splat_directory(
        std::env::var("DF_GAMEPLAY_SPLAT_DIR"),
        &std::path::Path::new(&asset_root).join("../splat"),
        &std::env::current_dir()?,
    )?;
    let battle_scene =
        startup_configuration::battle_scene(std::env::var("DF_GAMEPLAY_BATTLE_SCENE"))?;
    let battle_fps = startup_configuration::battle_fps(std::env::var("DF_GAMEPLAY_BATTLE_FPS"))?;
    if interactive {
        timing::install(timing::Timing::from_environment()?);
        let timing = timing::active();
        df_persistence::local_demo_scope::set_lease_seconds(timing.lease.as_secs());
        println!(
            "TIMING commit={}s reply={}s lease={}s browser={}s",
            timing.commit.as_secs(),
            timing.reply.as_secs(),
            timing.lease.as_secs(),
            timing.browser_request().as_secs()
        );
    }
    let settings = if interactive {
        startup_configuration::session_settings(
            startup_configuration::SessionInputs::from_environment(),
        )?
    } else {
        startup_configuration::SessionSettings::qualification()
    };
    join_link::install(&settings.join_origin)?;
    if interactive {
        let rating = startup_configuration::content_rating_setting(std::env::var(
            "DF_GAMEPLAY_CONTENT_RATING",
        ))?;
        startup_configuration::install_content_rating(rating)?;
        println!("{}", rating.startup_line());
    }
    let storyteller = std::sync::Arc::new(if interactive {
        let mode = startup_configuration::storyteller_mode(
            startup_configuration::StorytellerInputs::from_environment(),
            &std::env::current_dir()?,
        )?;
        match &mode {
            startup_configuration::StorytellerMode::On(settings) => println!(
                "STORYTELLER on model={} budget={}",
                settings.model, settings.token_budget
            ),
            startup_configuration::StorytellerMode::Off(reason) => {
                println!("STORYTELLER off ({reason})");
            }
        }
        storyteller::Storyteller::from_mode(mode)
    } else {
        storyteller::Storyteller::disabled()
    });
    if interactive {
        let settings = adventure_settings
            .as_ref()
            .ok_or("adventure settings absent")?;
        let (directory, loaded) = match settings.mode {
            startup_configuration::AdventureMode::Harbor => {
                let directory = startup_configuration::pack_directory(
                    std::env::var("DF_GAMEPLAY_PACK_DIR"),
                    &std::env::current_dir()?,
                )?;
                let loaded =
                    pack::load(&directory).map_err(|error| io::Error::other(error.to_string()))?;
                (directory, loaded)
            }
            startup_configuration::AdventureMode::Random
            | startup_configuration::AdventureMode::Campaign => {
                let mut prepared = adventure::startup::prepare(
                    settings,
                    &database_config()?,
                    &std::env::current_dir()?,
                    &storyteller,
                )
                .await?;
                adventure::startup::substitute_missing_art(
                    &mut prepared.loaded,
                    &prepared.record,
                    std::path::Path::new(&asset_root),
                );
                let opening = prepared
                    .loaded
                    .pack
                    .scenes
                    .iter()
                    .find(|scene| scene.key == pack::SceneKey::Opening)
                    .ok_or("generated opening absent")?;
                println!(
                    "ADVENTURE seed={} title={:?} location={} patron={} antagonist={} rolls={}",
                    prepared.record.seed,
                    opening.title,
                    prepared.record.entry("location")?,
                    prepared.record.entry("patron")?,
                    prepared.record.entry("antagonist")?,
                    prepared.record.rolls.len()
                );
                println!("ADVENTURE {}", prepared.prose);
                let directory = prepared.directory.clone();
                let loaded = prepared.loaded.clone();
                generated_adventure = Some(prepared);
                (directory, loaded)
            }
        };
        println!(
            "GAMEPLAY PACK directory={} pack={} tuning={} {}",
            directory.display(),
            &loaded.pack_revision.to_hex()[..16],
            &loaded.tuning_revision.to_hex()[..16],
            if loaded.departures.is_empty() {
                "standard SRD 5.2.1 values".to_owned()
            } else {
                format!("CUSTOM RULES: {}", loaded.departures.join(", "))
            }
        );
        let context = OperationContext {
            trace_parent: String::new(),
            build: crate::BUILD_ID.to_owned(),
        };
        let mut span = df_observe::begin(&context, "gameplay.startup");
        span.event(
            "gameplay.pack.loaded",
            &[
                (
                    "game.run",
                    hex(model::basis()
                        .map_err(|_| io::Error::other("gameplay run invalid"))?
                        .run
                        .as_bytes()),
                ),
                ("gameplay.pack_revision", loaded.pack_revision.to_hex()),
                ("gameplay.tuning_revision", loaded.tuning_revision.to_hex()),
                ("gameplay.departures", loaded.departures.join(", ")),
            ],
        );
        if let Some(prepared) = &generated_adventure
            && prepared.fresh
        {
            span.event(
                "gameplay.adventure.generated",
                &[
                    ("gameplay.adventure.seed", prepared.record.seed.clone()),
                    (
                        "gameplay.adventure.tables_revision",
                        prepared.record.tables_revision().to_owned(),
                    ),
                    (
                        "gameplay.adventure.rules_mode",
                        match prepared.record.rules_mode() {
                            adventure::RulesMode::Standard => "standard",
                            adventure::RulesMode::CustomAllowed => "custom_allowed",
                        }
                        .to_owned(),
                    ),
                    (
                        "gameplay.adventure.pack_sha256",
                        loaded.pack_revision.to_hex(),
                    ),
                    (
                        "gameplay.adventure.tuning_sha256",
                        loaded.tuning_revision.to_hex(),
                    ),
                    (
                        "gameplay.adventure.roll_count",
                        prepared.record.rolls.len().to_string(),
                    ),
                    (
                        "gameplay.adventure.seeded_dice",
                        prepared.record.seeded_dice.to_string(),
                    ),
                ],
            );
        }
        span.finish_unmeasured("loaded");
        // Every run has its own room code, derived from its database so a restart keeps it.
        let mut loaded = loaded;
        let run = database_config()?
            .get_dbname()
            .unwrap_or_default()
            .to_owned();
        loaded.pack.room_code = room_code::select(
            room_code::pinned(std::env::var("DF_GAMEPLAY_ROOM_CODE"))?,
            &run,
        );
        run_identity = room_code::run_identity(&run, &loaded.pack_revision.to_hex());
        println!("ROOM {}", loaded.pack.room_code);
        pack::install(loaded)?;
    }
    // The join code encodes the room code the pack above has fixed, before anything is started.
    let join_qr = join_link::JoinQr::for_room(&journey::room_code())
        .map_err(|_| io::Error::other("join QR code refused"))?;
    let voice = voice::Voice::start(
        if interactive {
            startup_configuration::voice_settings(
                startup_configuration::VoiceInputs::from_environment(),
            )?
        } else {
            startup_configuration::VoiceSettings::Off(
                startup_configuration::VoiceOff::Qualification,
            )
        },
        &speaker_castings(),
        generated_adventure
            .as_ref()
            .map_or("harbor", |prepared| prepared.record.seed.as_str()),
    )?;
    let speech = speech_input::SpeechInput::start(if interactive {
        speech_input::speech_settings(speech_input::SpeechInputs::from_environment())?
    } else {
        speech_input::SpeechSettings::Off(speech_input::SpeechOff::Qualification)
    })?;
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
    if interactive {
        let diagnostic = tokio::time::timeout(
            Duration::from_secs(2),
            df_persistence::local_demo_scope::stored_startup_configuration(
                &grant_client,
                &cold,
                codec,
            ),
        )
        .await;
        let checked = match diagnostic {
            Ok(Ok(Some(stored))) => model::validate_file_revisions(
                &stored,
                cold.pins()
                    .build
                    .revision(df_types::BuildRevision::Configuration),
            ),
            Ok(Ok(None)) => Ok(()),
            Ok(Err(error)) => Err(io::Error::other(format!(
                "stored gameplay provenance refused: {error:?}"
            ))),
            Err(_) => Err(io::Error::other("stored gameplay provenance deadline")),
        };
        if let Err(error) = checked {
            drop(grant_client);
            join_grant_driver(grant_driver).await?;
            return Err(error.into());
        }
    }
    let initialization = tokio::time::timeout(
        Duration::from_secs(5),
        df_persistence::local_demo_scope::admit_startup_with_adventure(
            &mut grant_client,
            &cold,
            df_persistence::local_demo_scope::DemoStartupIdentity {
                fence,
                player_credential,
                display_credential,
            },
            codec,
            &recovery,
            generated_adventure
                .as_ref()
                .map(|prepared| &prepared.documents),
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
        // Generous by default so a commit that is slow on a busy machine finishes instead of
        // leaving its outcome unknown and the session fenced (see `timing`).
        transaction_bounds: timing::active().transaction_bounds(),
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
    let (campaign_observer, campaign_worker) = if let Some(run) = generated_adventure
        .as_ref()
        .and_then(|prepared| prepared.campaign.clone())
    {
        let (observer, worker) = adventure::campaign_completion::worker(run)?;
        observer.observe(&initial);
        (Some(observer), Some(worker))
    } else {
        (None, None)
    };
    let (publication, intent_notifications) = actor::Publication::new(updates.clone());
    let publication = publication.with_campaign(campaign_observer.clone());
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
    let table_log = if interactive {
        table_log::TableLog::new()
    } else {
        table_log::TableLog::disabled()
    };
    // A generated adventure's NPCs have backstories and feelings about each hero; the harbor
    // pack and adventures saved before dossiers existed have none and keep today's behaviour.
    let social = match &generated_adventure {
        Some(prepared) if !prepared.record.dossiers.is_empty() => {
            println!(
                "ADVENTURE npc dossiers={} (attitude saved in {})",
                prepared.record.dossiers.len(),
                prepared.directory.display()
            );
            social::SocialLedger::open(
                prepared.record.dossiers.clone(),
                &prepared.record.seed,
                Some(prepared.directory.clone()),
            )
            .map_err(|error| io::Error::other(error.to_string()))?
        }
        _ => social::SocialLedger::none(),
    };
    // Living scenes: server-generated scene art that follows the story's mood, under the
    // server's one content rating.
    let rating = startup_configuration::content_rating();
    let imagery_settings = if interactive {
        startup_configuration::imagery_settings(
            startup_configuration::ImageryInputs::from_environment(),
            &std::env::current_dir()?,
            rating,
        )?
    } else {
        startup_configuration::ImagerySettings::Off(
            startup_configuration::ImageryOff::Qualification,
        )
    };
    let living_interval = match &imagery_settings {
        startup_configuration::ImagerySettings::On(configuration) => configuration.interval,
        startup_configuration::ImagerySettings::Off(_) => Duration::from_secs(30),
    };
    let imagery = imagery::Imagery::start(
        imagery_settings,
        generated_adventure
            .as_ref()
            .map(|prepared| prepared.directory.as_path()),
    )?;
    let living = living_scenes::LivingScenes::start(
        imagery.clone(),
        imagery::ReferenceStore::new(std::path::Path::new(&asset_root).join("generated")),
        social.clone(),
        rating,
        living_scenes::Timing::from_interval(living_interval),
    );
    let tempo = if std::env::args().nth(4).is_some() {
        None
    } else {
        let loaded =
            pack::active().ok_or_else(|| io::Error::other("tempo requires loaded tuning"))?;
        let tempo = actor::tempo::TempoOwner::start(
            owner.checkpoint(),
            loaded.tuning.tempo,
            std::time::Instant::now(),
        )
        .map_err(|error| io::Error::other(format!("tempo startup refused: {error:?}")))?;
        println!(
            "TEMPO on profile={} thresholds={:?}",
            loaded.tuning_revision, loaded.tuning.tempo.phase_thresholds_permille
        );
        Some(tempo)
    };
    let actor = actor::Actor {
        tempo,
        owner,
        table_log: table_log.clone(),
        social,
        dialogue: dialogue::DialogueState::default(),
        bootstrap_credential: player_credential,
        issuer,
        codec,
        fenced: false,
        recovery_wakeup: updates,
        campaign_observer,
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
        session: Default::default(),
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
        voice: voice.clone(),
        npc_voice: Arc::new(npc_reply::NpcVoice::new(storyteller)),
        table_log,
        living,
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
        run_identity: run_identity.into(),
        incoming: incoming_sender,
        connections: Arc::new(Semaphore::new(settings.max_connections)),
        allowed_origins: settings.allowed_origins.clone().into(),
        display: display_credential,
        glue: glue_bytes,
        wasm: wasm_bytes,
        battle_scene: battle_scene.map(Arc::from),
        battle_fps,
    };
    let router = Router::new()
        .route("/gameplay", get(both))
        .route("/gameplay/player", get(room::player_page))
        .route("/gameplay/display", get(display))
        .route("/gameplay/credits", get(credits))
        .route("/gameplay/tunnel", get(socket))
        .route("/pkg/df_tools.js", get(glue))
        .route("/pkg/df_tools_bg.wasm", get(wasm))
        .with_state(state)
        .merge(public_assets::router(public_assets::PublicAssets {
            art: art_bytes,
            campfire: campfire_bytes,
            portrait: portrait_bytes,
            inn: inn_bytes,
            combat: combat_bytes,
        }))
        .merge(public_assets::extra_router(
            startup_configuration::load_extra_assets(std::path::Path::new(&asset_root))?,
        ))
        .merge(music_assets::router(
            &std::path::Path::new(&asset_root).join("../music"),
        )?)
        .merge(splat_assets::router(&splat_directory)?)
        .merge(voice::router(voice.clone()))
        .merge(imagery::router(imagery.clone()))
        .merge(speech_input::router(
            speech.clone(),
            speech_input::actor_identity(sender.clone()),
            settings.allowed_origins.clone().into(),
        ))
        .merge(join_link::router(join_qr));
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
        println!("{}", voice.startup_line());
        println!("{}", imagery.startup_line());
        println!("{}", speech.startup_line());
        // PLAY-Q01: the join link and its QR code, from the public base address when one is set.
        println!(
            "JOIN url={} qr={}",
            join_link::join_url(&journey::room_code()),
            join_link::QR_PATH
        );
        if settings.join_loopback {
            println!(
                "JOIN phones on the network cannot reach {} (loopback only): start with development/run-gameplay.sh --lan",
                settings.join_origin
            );
        }
        println!(
            "READY display={origin}/gameplay/display player={origin}/gameplay/player?room={} room={}",
            journey::room_code(),
            journey::room_code()
        );
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
    if let Some(worker) = campaign_worker {
        tokio::task::spawn_blocking(move || drop(worker)).await?;
    }
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

/// The local demonstration database. `DF_GAMEPLAY_DATABASE_PORT` (1024 to 65535) selects another
/// PostgreSQL instance than the launcher's, so tests and harness runs use their own.
fn database_config() -> Result<tokio_postgres::Config, io::Error> {
    let launcher =
        startup_configuration::database_config(std::env::var("DF_GAMEPLAY_DEMO_DATABASE"))?;
    let port = match std::env::var("DF_GAMEPLAY_DATABASE_PORT") {
        Ok(port) => port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port >= 1024)
            .ok_or_else(|| io::Error::other("DF_GAMEPLAY_DATABASE_PORT refused"))?,
        Err(std::env::VarError::NotPresent) => return Ok(launcher),
        Err(_) => return Err(io::Error::other("DF_GAMEPLAY_DATABASE_PORT refused")),
    };
    // A configuration's port list only grows, so the instance's own is built afresh.
    let mut configuration = tokio_postgres::Config::new();
    configuration.host("127.0.0.1").port(port);
    if let Some(user) = launcher.get_user() {
        configuration.user(user);
    }
    if let Some(name) = launcher.get_dbname() {
        configuration.dbname(name);
    }
    Ok(configuration)
}
#[cfg(test)]
mod battle_scene_page_tests {
    use super::page;

    #[test]
    fn the_display_page_names_the_forced_battle_scene_and_the_player_page_never_does() {
        let credential = Some([7u8; 32]);
        let forced = page(false, credential, Some("scan-village"), 24).0;
        assert!(forced.contains("data-battle-scene=\"scan-village\""));
        assert!(forced.contains("data-battle-fps=\"24\""));
        let default = page(false, credential, None, 30).0;
        assert!(!default.contains("data-battle-scene"));
        assert!(default.contains("data-battle-fps=\"30\""));
        let player = page(true, None, Some("scan-village"), 24).0;
        assert!(!player.contains("data-battle-scene"));
        assert!(!player.contains("data-battle-fps"));
    }
}
