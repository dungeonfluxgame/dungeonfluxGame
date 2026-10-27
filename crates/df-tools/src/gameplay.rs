//! A finite, explicitly local gameplay slice. Authentication is preconfigured development
//! membership, not production bootstrap. The action and its outcome use generated RPC.
mod actor;
mod adventure;
mod approach_model;
mod beat_script;
mod beat_script_runtime;
#[cfg(test)]
mod beat_script_tests;
#[cfg(test)]
mod beat_script_variety_tests;
mod character_clips;
mod codex_cli;
#[cfg(test)]
mod connection_tests;
#[cfg(test)]
mod content_rating_live_tests;
mod cost;
mod courier_ai;
mod courier_npc_qualification;
mod courier_process_qualification;
mod dialogue;
#[cfg(test)]
mod dialogue_tests;
mod dice;
mod display_effects;
mod display_tuning;
mod freeform_action;
// DEVCTL-001: development-only agent control surface (off by default).
mod creator_catalog;
mod creator_classification;
mod creator_portrait_startup;
mod creator_portraits;
mod dev_control;
mod fight_foresight;
mod fight_lines;
mod health_faces;
mod hero_catalog;
mod hero_identity;
mod hero_library;
mod home_theme;
#[cfg(test)]
mod home_theme_tests;
mod home_video;
#[cfg(test)]
mod home_video_tests;
mod imagery;
mod inn_qualification;
mod investigation;
mod join_link;
mod journey;
mod launcher_status;
mod line_cleanup;
mod living_scenes;
mod media_grant;
mod media_policy;
mod memory;
mod model;
mod music_assets;
mod narrative_focus;
#[cfg(test)]
mod new_adventure_tests;
mod notebook;
#[cfg(test)]
mod notebook_tests;
mod npc_dossier;
#[cfg(test)]
mod npc_dossier_live_tests;
#[cfg(test)]
mod npc_dossier_tests;
mod npc_event_classifier;
mod npc_facts;
mod npc_reply;
#[cfg(test)]
mod npc_talk_live_tests;
#[cfg(test)]
mod npc_talk_rules;
mod npc_target;
mod pack;
mod pack_contents;
#[cfg(test)]
mod pack_tests;
mod place;
mod place_creation;
mod place_move;
mod place_music;
mod plate_segments;
#[path = "gameplay_browser/presentation_ingress.rs"]
mod presentation_ingress;
mod public_assets;
mod qualification;
#[cfg(test)]
mod recovery_qualification;
mod refusal_detail;
mod resource_adapters;
mod resources;
mod rest_qualification;
mod restart_qualification;
mod room;
mod room_code;
mod run_restart;
mod runtime_launch;
mod scene_mood;
mod session_recovery;
mod shot;
mod social;
mod speech_input;
mod speech_text;
mod splat_assets;
mod splat_world;
mod staging;
mod startup_configuration;
#[cfg(test)]
mod startup_contract;
mod storyteller;
mod table_log;
mod table_settings;
#[cfg(test)]
mod tempo_clock;
mod timing;
mod voice;
#[cfg(test)]
mod weapon_swap_audit;
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
    display_credential: [u8; 32],
    updates: watch::Receiver<df_model::checkpoint::Checkpoint>,
    streams: Arc<Semaphore>,
    voice: voice::Voice,
    /// Speaks for the scene's NPC when the storyteller is on; silent otherwise.
    npc_voice: Arc<npc_reply::NpcVoice>,
    /// The public feed of who said and did what; a new line publishes a fresh view.
    table_log: Arc<table_log::TableLog>,
    /// The current, server-owned standing and dossiers for each hero and character.
    social: Arc<social::SocialLedger>,
    /// Follows the story's mood with generated scene art; presentation only.
    living: living_scenes::LivingScenes,
    look_ahead: Arc<place_move::LookAhead>,
    layout: dev_control::layout::LayoutState,
    portraits: creator_portraits::Portraits,
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
    pub(super) async fn commit_npc_event(
        &self,
        credential: [u8; 32],
        proposal: npc_event_classifier::Proposal,
    ) -> Result<actor::NpcEventOutcome, Status> {
        let audit = service_audit_context(self, Some(&credential));
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::NpcEvent {
                credential,
                proposal,
                reply,
            })
            .map_err(|_| {
                ingress_failure(
                    &audit,
                    "gameplay.npc_event_refused",
                    "NPC_EVENT commit_refused",
                    "actor-boundary",
                    Status::resource_exhausted("gameplay actor queue full"),
                )
            })?;
        tokio::time::timeout(timing::active().reply, wait)
            .await
            .map_err(|_| {
                ingress_failure(
                    &audit,
                    "gameplay.npc_event_refused",
                    "NPC_EVENT commit_refused",
                    "actor-boundary",
                    Status::deadline_exceeded("NPC event commit timed out"),
                )
            })?
            .map_err(|_| {
                ingress_failure(
                    &audit,
                    "gameplay.npc_event_refused",
                    "NPC_EVENT commit_refused",
                    "actor-boundary",
                    Status::unavailable("NPC event actor response dropped"),
                )
            })?
    }

    async fn view(
        &self,
        credential: [u8; 32],
        request: rpc::WatchViewRequest,
    ) -> Result<(LocalDemoRole, rpc::ViewMessage), Status> {
        // A full queue is waited out within the reply deadline, so a burst of watchers (a new
        // adventure starting) does not close every stream with ResourceExhausted (DEV-09).
        let viewed = room::actor_view(&self.actor, credential, request.clone()).await;
        // Every view leaves the server through here, so this is where spoken lines are
        // registered; only the display audience is voiced.
        if let Ok((_, message)) = &viewed {
            self.voice
                .register_people(self.updates.borrow().state())
                .map_err(actor::unavailable)?;
            if !crate::fake_profile::is_enabled() {
                self.voice.register_view(message);
            }
        }
        // Living scenes watch the display's view and then add their published art to it; the
        // art is presentation only and nothing else in the view changes.
        let (role, mut message) = viewed?;
        // Release the checkpoint watch before the living-scenes lock: holding its read borrow
        // across that lock blocks the owner's publish behind any thread waiting on the lock.
        let checkpoint = self.updates.borrow().clone();
        self.living.observe_checkpoint(&checkpoint);
        self.living.observe(&message);
        self.living.apply(&mut message);
        if self
            .admit_picture_features(credential, role, &checkpoint, &mut message)
            .await
        {
            // Evidence admission advances the revision. Rebuild offers with that same basis.
            let (_, refreshed) = room::actor_view(&self.actor, credential, request).await?;
            message = refreshed;
            let checkpoint = self.updates.borrow().clone();
            self.living.observe_checkpoint(&checkpoint);
            self.living.observe(&message);
            self.living.apply(&mut message);
        }
        // PLAY-V10: the home loop's state, read on every view so a loop that lands wakes it.
        home_video::apply(&mut message);
        place_music::apply(&mut message);
        display_effects::apply(&mut message);
        self.layout
            .attach(&mut message)
            .await
            .map_err(|error| Status::internal(error.to_string()))?;
        if crate::fake_profile::is_enabled() {
            // Final presentation must follow living-scene/portrait overlays. Register the
            // resulting canonical fake text before attaching its signed local audio URLs.
            crate::fake_profile::FakeProfile.project(&mut message);
            self.voice.register_view(&message);
        }
        voice::attach_urls(&mut message);
        media_grant::sign_view(&mut message, role == LocalDemoRole::Display, &credential);
        wire::validate_view_size(&message).map_err(actor::unavailable)?;
        Ok((role, message))
    }
}
fn service_audit_context(
    service: &Service,
    credential: Option<&[u8; 32]>,
) -> df_observe::AuditContext {
    let basis = service.updates.borrow().basis();
    df_observe::AuditContext {
        operation: None,
        seat: credential.map(|credential| df_observe::seat_hash(credential)),
        room: Some(crate::client_diagnostics::operation_hash(
            basis.session.as_bytes(),
        )),
        revision: Some(format!(
            "{}:{}",
            basis.revision.epoch().get(),
            basis.revision.sequence()
        )),
    }
}

fn submit_audit_context(
    request: &rpc::SubmitActionRequest,
    basis: df_model::checkpoint::Basis,
) -> df_observe::AuditContext {
    df_observe::AuditContext {
        operation: request
            .operation_id
            .as_ref()
            .and_then(|id| id.value.as_deref())
            .map(crate::client_diagnostics::operation_hash),
        seat: None,
        room: Some(crate::client_diagnostics::operation_hash(
            basis.session.as_bytes(),
        )),
        revision: Some(format!(
            "{}:{}",
            basis.revision.epoch().get(),
            basis.revision.sequence()
        )),
    }
}

fn validate_submit_shape(request: &rpc::SubmitActionRequest) -> Result<(), Status> {
    let fail = |field: &'static str, actual: usize, limit: usize| {
        actor::record_ingress_refusal(
            request.action_kind,
            "submit-shape",
            &format!("Capacity field={field} actual={actual} limit={limit}"),
            tonic::Code::InvalidArgument,
        );
        Status::invalid_argument("bounded action fields required")
    };
    if request.offer_id.len() > 96 {
        return Err(fail("offer_id_bytes", request.offer_id.len(), 96));
    }
    if let Some(character) = request.character.as_ref() {
        if character.name.len() > 48 {
            return Err(fail("character_name_bytes", character.name.len(), 48));
        }
        if character.choices.len() > 16 {
            return Err(fail("character_choices", character.choices.len(), 16));
        }
        for choice in &character.choices {
            if choice.group_id.len() > 64 {
                return Err(fail("choice_group_bytes", choice.group_id.len(), 64));
            }
            if choice.option_id.len() > 64 {
                return Err(fail("choice_option_bytes", choice.option_id.len(), 64));
            }
        }
        if let Some(appearance) = character
            .identity
            .as_ref()
            .and_then(|identity| identity.appearance.as_ref())
        {
            hero_identity::validate_public_description(&appearance.public_description).map_err(
                |error| {
                    actor::record_ingress_refusal(
                        request.action_kind,
                        "submit-public-description",
                        &format!(
                            "{error:?} bytes={} chars={}",
                            appearance.public_description.len(),
                            appearance.public_description.chars().count()
                        ),
                        tonic::Code::InvalidArgument,
                    );
                    Status::invalid_argument(
                        "Public appearance must fit 2000 characters and 4096 bytes",
                    )
                },
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod submit_diagnostic_tests {
    use super::*;
    #[test]
    fn oversize_creator_ingress_is_logged_before_actor_admission_without_player_text() {
        let basis = model::initial().unwrap().basis();
        let operation = vec![0x37; 16];
        let request = rpc::SubmitActionRequest {
            operation_id: Some(rpc::OperationId {
                value: Some(operation.clone()),
            }),
            session_id: Some(rpc::SessionId {
                value: Some(basis.session.as_bytes().to_vec()),
            }),
            run_id: Some(rpc::RunId {
                value: Some(basis.run.as_bytes().to_vec()),
            }),
            observed_revision: Some(wire::revision(basis.revision)),
            action_kind: rpc::GameplayActionKind::CreateCharacter as i32,
            character: Some(rpc::CharacterSelection {
                name: "secret-player-name".repeat(4),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut audit = submit_audit_context(&request, basis);
        let seat = df_observe::seat_hash(&[0x38; 32]);
        audit.seat = Some(seat);
        let _scope = df_observe::audit_scope(audit);
        assert_eq!(
            validate_submit_shape(&request).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
        let lines = actor::audit_lines_containing(
            "step=submit-shape outer=ingress cause=Capacity field=character_name_bytes actual=72 limit=48",
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains(&format!(
            "op={}",
            crate::client_diagnostics::operation_hash(&operation)
        )));
        assert!(lines[0].contains(&format!("seat={seat}")));
        assert!(lines[0].contains(&format!(
            "room={}",
            crate::client_diagnostics::operation_hash(basis.session.as_bytes())
        )));
        assert!(lines[0].contains(&format!(
            "rev={}:{}",
            basis.revision.epoch().get(),
            basis.revision.sequence()
        )));
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("secret-player-name"))
        );
    }
}

#[tonic::async_trait]
impl rpc::action_service_server::ActionService for Service {
    async fn submit(
        &self,
        request: Request<rpc::SubmitActionRequest>,
    ) -> Result<Response<rpc::SubmitActionResponse>, Status> {
        let mut audit = submit_audit_context(request.get_ref(), self.updates.borrow().basis());
        let action_kind = request.get_ref().action_kind;
        let credential = credential(&request).inspect_err(|status| {
            actor::record_ingress_refusal_scoped(
                &audit,
                action_kind,
                "submit-credential",
                "CredentialInvalid",
                status.code(),
            );
        })?;
        audit.seat = Some(df_observe::seat_hash(&credential));
        let request = request.into_inner();
        {
            let _scope = df_observe::audit_scope(audit.clone());
            validate_submit_shape(&request)?;
        }
        if request.action_kind == rpc::GameplayActionKind::Travel as i32 {
            return self
                .travel_submit(credential, request)
                .await
                .map(Response::new);
        }
        // A scene assault starts a fight through the one entry point every route shares.
        if request.action_kind == rpc::GameplayActionKind::SceneAssault as i32 {
            return self
                .scene_assault_submit(credential, request)
                .await
                .inspect_err(|status| {
                    actor::record_ingress_refusal_scoped(
                        &audit,
                        action_kind,
                        "scene-assault-submit",
                        "SceneAssaultRefused",
                        status.code(),
                    )
                })
                .map(Response::new);
        }
        let committed = df_api::operation_id(request.operation_id.as_ref())
            .ok()
            .is_some_and(|operation| {
                self.updates
                    .borrow()
                    .state()
                    .decisions
                    .iter()
                    .any(|decision| decision.operation == operation)
            });
        let portrait = if !committed
            && request.action_kind == rpc::GameplayActionKind::CreateCharacter as i32
        {
            self.portraits
                .accepted(credential, request.character.as_ref())
                .await
                .inspect_err(|status| {
                    actor::record_ingress_refusal_scoped(
                        &audit,
                        action_kind,
                        "portrait-attachment",
                        "PortraitAttachmentRefused",
                        status.code(),
                    )
                })?
                .or_else(|| hero_library::loaded_portrait(credential, request.character.as_ref()))
        } else {
            None
        };
        let observation_request = request.clone();
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::SubmitAppearance {
                portrait,
                credential,
                request,
                reply,
            })
            .map_err(|_| {
                actor::record_ingress_refusal_scoped(
                    &audit,
                    action_kind,
                    "submit-actor-admission",
                    "QueueFull",
                    tonic::Code::ResourceExhausted,
                );
                Status::resource_exhausted("gameplay actor queue full")
            })?;
        let outcome = tokio::time::timeout(timing::active().reply, wait)
            .await
            .map_err(|_| {
                actor::record_ingress_refusal_scoped(
                    &audit,
                    action_kind,
                    "submit-actor-reply",
                    "ReplyDeadlineOutcomeUnknown",
                    tonic::Code::DeadlineExceeded,
                );
                Status::deadline_exceeded("retain your gameplay operation for lookup")
            })?
            .map_err(|_| {
                actor::record_ingress_refusal_scoped(
                    &audit,
                    action_kind,
                    "submit-actor-reply",
                    "ActorStoppedOutcomeUnknown",
                    tonic::Code::Unavailable,
                );
                Status::unavailable("gameplay actor stopped")
            })??;
        self.complete_observation(credential, &observation_request, &outcome)
            .await?;
        Ok(Response::new(outcome))
    }

    /// Reads a typed description of a hero's looks and story into a proposed identity. Worker C01
    /// builds the reader (`gameplay/hero_identity.rs`); until it is connected the call says so.
    async fn request_hero_portrait(
        &self,
        request: Request<rpc::HeroPortraitRequest>,
    ) -> Result<Response<rpc::HeroPortraitResponse>, Status> {
        creator_portraits::handle(self, request, true)
            .await
            .map(Response::new)
    }
    async fn get_hero_portrait(
        &self,
        request: Request<rpc::HeroPortraitRequest>,
    ) -> Result<Response<rpc::HeroPortraitResponse>, Status> {
        creator_portraits::handle(self, request, false)
            .await
            .map(Response::new)
    }

    async fn preview_hero_identity(
        &self,
        request: Request<rpc::PreviewHeroIdentityRequest>,
    ) -> Result<Response<rpc::PreviewHeroIdentityResponse>, Status> {
        hero_identity::preview(self, request).await
    }
}
fn ingress_failure(
    audit: &df_observe::AuditContext,
    event: &'static str,
    key: &'static str,
    step: &'static str,
    status: Status,
) -> Status {
    let _scope = df_observe::audit_scope(audit.clone());
    actor::record_audit_line(event, format!("{key} step={step} code={:?}", status.code()));
    status
}

struct WatchAudit {
    audit: df_observe::AuditContext,
    role: &'static str,
    seat: u64,
    frames: u64,
    started: std::time::Instant,
    reason: &'static str,
}
impl WatchAudit {
    fn end(&mut self, reason: &'static str) {
        self.reason = reason;
    }
}
impl Drop for WatchAudit {
    fn drop(&mut self) {
        let _scope = df_observe::audit_scope(self.audit.clone());
        actor::record_audit_line(
            "gameplay.watch_ended",
            format!(
                "WATCH ended reason={} role={} seat={} frames={} duration_ms={} ms={}",
                self.reason,
                self.role,
                self.seat,
                self.frames,
                self.started.elapsed().as_millis(),
                self.started.elapsed().as_millis(),
            ),
        );
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
        let mut audit = service_audit_context(self, None);
        let credential = credential(&request).map_err(|status| {
            ingress_failure(
                &audit,
                "gameplay.watch_refused",
                "WATCH refused",
                "credential",
                status,
            )
        })?;
        audit.seat = Some(df_observe::seat_hash(&credential));
        let request = request.into_inner();
        let permit = self.streams.clone().try_acquire_owned().map_err(|_| {
            ingress_failure(
                &audit,
                "gameplay.watch_refused",
                "WATCH refused",
                "limit",
                Status::resource_exhausted("gameplay watch limit"),
            )
        })?;
        let mut updates = self.updates.clone();
        updates.borrow_and_update();
        let mut table_lines = self.table_log.subscribe();
        table_lines.borrow_and_update();
        // A published scene image wakes a watcher the way a new table line does.
        // PLAY-V10: a stream without living art still wakes when a home loop lands.
        let mut art = Some(
            self.living
                .subscribe()
                .unwrap_or_else(home_video::wake_subscribe),
        );
        let mut layout = self.layout.subscribe();
        layout.borrow_and_update();
        if let Some(art) = art.as_mut() {
            art.borrow_and_update();
        }
        let (role, first) = self
            .view(credential, request.clone())
            .await
            .map_err(|status| {
                ingress_failure(
                    &audit,
                    "gameplay.watch_refused",
                    "WATCH refused",
                    "first_view",
                    status,
                )
            })?;
        let role = if role == LocalDemoRole::Player {
            "player"
        } else {
            "display"
        };
        let seat = df_observe::seat_hash(&credential);
        let revision = self.updates.borrow().basis().revision;
        audit.revision = Some(format!(
            "{}:{}",
            revision.epoch().get(),
            revision.sequence()
        ));
        let open_scope = df_observe::audit_scope(audit.clone());
        actor::record_audit_line(
            "gameplay.watch_opened",
            format!(
                "WATCH opened role={role} seat={seat} rev={}:{}",
                revision.epoch().get(),
                revision.sequence()
            ),
        );
        drop(open_scope);
        let permit = (
            permit,
            WatchAudit {
                audit: audit.clone(),
                role,
                seat,
                frames: 0,
                started: std::time::Instant::now(),
                reason: "caller-gone",
            },
        );
        // The display shows the generated scene art and every screen shows the heroes' health
        // faces, so every stream wakes when published art changes.
        let service = self.clone();
        let stream = futures::stream::unfold(
            (
                Some(first),
                updates,
                table_lines,
                art,
                layout,
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
                mut layout,
                service,
                request,
                credential,
                mut permit,
            )| async move {
                let next = if let Some(first) = first {
                    Ok(first)
                } else {
                    // A checkpoint change, a new table line or a newly published scene image each
                    // publish a fresh view.
                    let ended = tokio::select! {
                        changed = updates.changed() => changed.is_err(),
                        changed = table_lines.changed() => changed.is_err(),
                        changed = layout.changed() => changed.is_err(),
                        () = scene_art_changed(&mut art) => false,
                        () = tokio::time::sleep(Duration::from_secs(5)) => false,
                    };
                    if ended {
                        permit.1.end("source-closed");
                        return None;
                    }
                    service
                        .view(credential, request.clone())
                        .await
                        .map(|(_, view)| view)
                };
                if let Err(status) = &next {
                    let _scope = df_observe::audit_scope(permit.1.audit.clone());
                    actor::record_audit_line(
                        "gameplay.watch_failed",
                        format!(
                            "WATCH failed code={:?} role={} seat={}",
                            status.code(),
                            permit.1.role,
                            permit.1.seat
                        ),
                    );
                    permit.1.end("view-failed");
                }
                permit.1.frames = permit.1.frames.saturating_add(1);
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
                        layout,
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

    /// The shared display reports the last combat beat it finished presenting. The
    /// report can only advance the pacing gate's released sequence; the actor ignores
    /// reports from any credential but the display's.
    async fn report_presented(
        &self,
        request: Request<rpc::PresentedRequest>,
    ) -> Result<Response<rpc::PresentedResponse>, Status> {
        let mut audit = service_audit_context(self, None);
        let credential = credential(&request).map_err(|status| {
            ingress_failure(
                &audit,
                "gameplay.display_refused",
                "DISPLAY report_refused",
                "credential",
                status,
            )
        })?;
        audit.seat = Some(df_observe::seat_hash(&credential));
        let request = request.into_inner();
        if !request.layout_readback_json.is_empty() {
            if credential != self.display_credential {
                return Err(ingress_failure(
                    &audit,
                    "gameplay.display_refused",
                    "DISPLAY report_refused",
                    "authorization",
                    Status::permission_denied("layout report requires display authorization"),
                ));
            }
            self.layout
                .readback(request.display_instance, &request.layout_readback_json)
                .await
                .map_err(|error| {
                    ingress_failure(
                        &audit,
                        "gameplay.display_refused",
                        "DISPLAY report_refused",
                        "layout-readback",
                        Status::invalid_argument(error.to_string()),
                    )
                })?;
            // Metadata-only reports never enter the gameplay actor or release a combat beat.
            if request.sequence == 0 {
                return Ok(Response::new(rpc::PresentedResponse {
                    released_sequence: 0,
                }));
            }
        }
        let (reply, wait) = oneshot::channel();
        self.actor
            .try_submit(actor::Call::Presented {
                credential,
                sequence: request.sequence,
                display_instance: request.display_instance,
                reply,
            })
            .map_err(|_| {
                ingress_failure(
                    &audit,
                    "gameplay.display_refused",
                    "DISPLAY report_refused",
                    "actor-boundary",
                    Status::resource_exhausted("gameplay actor queue full"),
                )
            })?;
        let released_sequence = tokio::time::timeout(timing::active().reply, wait)
            .await
            .map_err(|_| {
                ingress_failure(
                    &audit,
                    "gameplay.display_refused",
                    "DISPLAY report_refused",
                    "actor-boundary",
                    Status::deadline_exceeded("gameplay presented deadline"),
                )
            })?
            .map_err(|_| {
                ingress_failure(
                    &audit,
                    "gameplay.display_refused",
                    "DISPLAY report_refused",
                    "actor-boundary",
                    Status::unavailable("gameplay actor stopped"),
                )
            })??;
        Ok(Response::new(rpc::PresentedResponse { released_sequence }))
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
    /// `DF_GAMEPLAY_SPLAT_LOD`: the tier of a streamed scan the battlefield draws as its finest.
    splat_lod: u32,
    /// `DF_GAMEPLAY_BATTLE_GRID`: whether the TV draws the cell grid (off by default).
    battle_grid: bool,
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
        actor::record_audit_line(
            "gameplay.stream_refused",
            "STREAM refused reason=origin".to_owned(),
        );
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = state.connections.clone().try_acquire_owned() else {
        actor::record_audit_line(
            "gameplay.stream_refused",
            "STREAM refused reason=connection_limit".to_owned(),
        );
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(admission) = state.incoming.try_reserve() else {
        actor::record_audit_line(
            "gameplay.stream_refused",
            "STREAM refused reason=admission_full".to_owned(),
        );
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
            let started = std::time::Instant::now();
            let result = admission.accept_websocket_measured(socket, metrics).await;
            actor::record_audit_line(
                "gameplay.stream_closed",
                format!(
                    "STREAM closed outcome={} duration_ms={} ms={}",
                    if result.is_ok() { "closed" } else { "failed" },
                    started.elapsed().as_millis(),
                    started.elapsed().as_millis()
                ),
            );
            let context = OperationContext {
                trace_parent: String::new(),
                build: crate::BUILD_ID.to_owned(),
            };
            let mut span = df_observe::begin(&context, "gameplay.local_connection");
            if let Err(error) = &result {
                // One stream failure for one member: say which error ended the tunnel, so a phone
                // that loses its connection leaves a diagnostic instead of a silent close.
                actor::record_audit_line(
                    "gameplay.stream_failed",
                    format!("STREAM ended reason=transport kind={:?}", error.kind()),
                );
                span.event(
                    "gameplay.stream_failed",
                    &[("stream.reason", format!("{error}"))],
                );
            }
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
                actor_ids: pack::with_content(|pack| {
                    pack.scenes
                        .iter()
                        .flat_map(|scene| &scene.cast)
                        .filter(|actor| actor.name == speaker.name)
                        .map(|actor| actor.id.clone())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect()
                }),
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
        None,
        state.battle_fps,
        state.splat_lod,
        state.battle_grid,
    )
}
async fn player(join_scene_asset: Option<&str>) -> Html<String> {
    let opening_scene_asset = pack::scene(pack::SceneKey::Opening).asset;
    page(
        true,
        None,
        None,
        Some(join_scene_asset.unwrap_or(&opening_scene_asset)),
        df_render::battle::BattleFps::DEFAULT.hz(),
        0,
        false,
    )
}
async fn display(State(state): State<PageState>) -> Result<Html<String>, axum::http::StatusCode> {
    let Html(body) = page(
        false,
        Some(state.display),
        state.battle_scene.as_deref(),
        None,
        state.battle_fps,
        state.splat_lod,
        state.battle_grid,
    );
    let run_id = hex(model::basis()
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?
        .run
        .as_bytes());
    // Hand-tuned TV values persist in a stylesheet beside the play data (no rebuild to change).
    let body = body.replace(
        "</head>",
        &format!("{}</head>", display_tuning::style_element().await),
    );
    Ok(Html(body.replace(
        "id=\"display\"",
        &format!(
            "id=\"display\" data-run=\"{}\" data-run-id=\"{run_id}\" data-room=\"{}\"",
            state.run_identity,
            journey::room_code()
        ),
    )))
}
fn page(
    player: bool,
    display: Option<[u8; 32]>,
    battle_scene: Option<&str>,
    join_scene_asset: Option<&str>,
    battle_fps: u32,
    splat_lod: u32,
    battle_grid: bool,
) -> Html<String> {
    // The browser waits a little longer than the server for any one request; both come from the
    // same setting (see `timing`).
    let wait = timing::active().browser_request().as_millis();
    let roots = if player {
        let join_scene_asset = join_scene_asset.map(escape_attribute).unwrap_or_default();
        format!(
            "<section class=\"client player\" id=\"player\" data-tunnel-path=\"/gameplay/tunnel\" data-page-path=\"/gameplay/player\" data-asset-base=\"/\" data-room=\"{}\" data-join-scene-asset=\"{join_scene_asset}\" data-request-timeout-ms=\"{wait}\"><div class=\"view\"></div></section>",
            journey::ROOM_CODE,
        )
    } else {
        display.map(|credential|format!("<section class=\"client display\" id=\"display\" data-tunnel-path=\"/gameplay/tunnel\" data-page-path=\"/gameplay/display\" data-asset-base=\"/\" data-local-binding=\"{}\" data-request-timeout-ms=\"{wait}\" data-battle-fps=\"{battle_fps}\" data-battle-splat-lod=\"{splat_lod}\"{}{}><div class=\"view\"></div></section>",hex(&credential),battle_scene.map(|scene|format!(" data-battle-scene=\"{scene}\"")).unwrap_or_default(),if battle_grid{" data-battle-grid=\"on\""}else{""})).unwrap_or_default()
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
fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&#39;")
}
/// A minimal shell: the Rust client mounts and styles everything (see `live_theme`).
const HTML: &str = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><base href="/"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"><title>DungeonFlux</title><style>*{box-sizing:border-box}html,body{margin:0;background:#05090f;color:#efe6ce;font-family:Georgia,serif}.df-boot{position:fixed;inset:0;display:grid;place-content:center;gap:18px;text-align:center;background:radial-gradient(ellipse at 50% 40%,#16202c 0%,#05090f 70%);transition:opacity .6s ease,visibility .6s}.df-boot-title{font-size:clamp(28px,7vw,48px);letter-spacing:.06em;color:#e3c68f;font-weight:400}.df-boot-line{font-size:17px;color:#b9b2a0;font-style:italic}.df-boot-bar{justify-self:center;width:160px;height:2px;background:#1d2733;overflow:hidden}.df-boot-bar::after{content:"";display:block;width:40%;height:100%;background:#e3c68f;animation:df-boot 1.6s ease-in-out infinite}@keyframes df-boot{0%{transform:translateX(-100%)}100%{transform:translateX(250%)}}body:has(.view>*) .df-boot{opacity:0;visibility:hidden;pointer-events:none}</style></head><body><main class="__LAYOUT__">__ROOTS__</main><div class="df-boot" role="status" aria-live="polite"><div class="df-boot-title">DungeonFlux</div><div class="df-boot-line">Setting the table…</div><div class="df-boot-bar"></div></div><script type="module">import init from '/pkg/df_tools.js'; await init();</script></body></html>"#;

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
pub async fn serve(
    #[cfg(windows)] console_stop: tokio::sync::watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    match std::env::var("DF_GAMEPLAY_DISABLE_OPENAI_API") {
        Ok(value) if value == "on" || value == "off" => {}
        Err(std::env::VarError::NotPresent) => {}
        _ => {
            return Err(
                std::io::Error::other("DF_GAMEPLAY_DISABLE_OPENAI_API must be on or off").into(),
            );
        }
    }
    codex_cli::validate_startup().await?;
    df_observe::install_audit_sink(actor::record_audit_line);
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
    run_restart::serve(
        #[cfg(windows)]
        console_stop,
    )
    .await
}

/// Jev's credential and endpoint when the Jev gate allows it and its key is set.
fn jev_settings() -> Result<Option<startup_configuration::ApproachSettings>, io::Error> {
    if !media_policy::gates().allows(media_policy::Capability::Jev) {
        return Ok(None);
    }
    startup_configuration::approach_settings(
        std::env::var("DF_JEV_API_KEY"),
        std::env::var("DF_JEV_BASE_URL"),
    )
}

/// The live store's codec bounds: they admit every checkpoint the model's own limits admit
/// (records, per-text bytes and retained bytes, all larger for a tactical pack), so a state the
/// rules accept can always be saved. A fixed 1,024-item bound refused the first hero created in
/// a tactical harbor game once creation carried the player's full character sheet.
fn live_codec_limits() -> NativeCodecLimits {
    let model = model::limits();
    NativeCodecLimits {
        maximum_document_bytes: model.maximum_retained_bytes,
        maximum_allocated_bytes: 2 * model.maximum_retained_bytes,
        maximum_collection_items: model.maximum_records,
        maximum_text_bytes: model.maximum_text_bytes,
    }
}
struct AuditStage {
    key: &'static str,
    step: Option<&'static str>,
    started: std::time::Instant,
}
impl AuditStage {
    fn new(key: &'static str, step: &'static str) -> Self {
        actor::record_audit_line(
            "gameplay.lifecycle",
            format!("{key} step={step} outcome=started"),
        );
        Self {
            key,
            step: Some(step),
            started: std::time::Instant::now(),
        }
    }
    fn step(&mut self, step: &'static str) {
        self.finish("ready");
        self.step = Some(step);
        self.started = std::time::Instant::now();
        actor::record_audit_line(
            "gameplay.lifecycle",
            format!("{} step={step} outcome=started", self.key),
        );
    }
    fn finish(&mut self, outcome: &'static str) {
        if let Some(step) = self.step.take() {
            actor::record_audit_line(
                "gameplay.lifecycle",
                format!(
                    "{} step={step} outcome={outcome} ms={}",
                    self.key,
                    self.started.elapsed().as_millis()
                ),
            );
        }
    }
}
impl Drop for AuditStage {
    fn drop(&mut self) {
        self.finish("failed");
    }
}

async fn serve_run(
    #[cfg(windows)] mut console_stop: watch::Receiver<bool>,
    database: tokio_postgres::Config,
    first_run: bool,
    fresh_seed: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    df_observe::install_audit_sink(actor::record_audit_line);
    let runtime_secret = runtime_launch::secret_from_environment()?;
    let startup_started = std::time::Instant::now();
    let mut startup = AuditStage::new("STARTUP", "hero-library");
    hero_library::configure(database.clone()).await;
    startup.step("configuration");
    let interactive = std::env::args().nth(4).is_none();
    // Every paid capability's switch, read once; generation sites ask `media_policy::gates()`.
    let gates = media_policy::Gates::from_environment()?;
    media_policy::install(gates);
    println!("{}", gates.startup_line());
    let mut adventure_settings = if interactive {
        Some(startup_configuration::adventure_settings(
            startup_configuration::AdventureInputs::from_environment(),
            &std::env::current_dir()?,
        )?)
    } else {
        None
    };
    if fresh_seed && let Some(settings) = adventure_settings.as_mut() {
        settings.seed = None;
    }
    let mut generated_adventure: Option<adventure::startup::PreparedAdventure> = None;
    // Names the server run for pages that keep a seat; empty (no check) in qualification.
    let mut run_identity = String::new();
    let web_root = std::env::args()
        .nth(2)
        .ok_or("generated gameplay web directory required")?;
    let asset_root = std::env::args()
        .nth(3)
        .ok_or("fixed gameplay asset directory required")?;
    startup.step("assets");
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
    if first_run {
        home_theme::install(std::path::Path::new(&asset_root))?;
    }
    startup.step("scene-configuration");
    let splat_directory = startup_configuration::splat_directory(
        std::env::var("DF_GAMEPLAY_SPLAT_DIR"),
        &std::path::Path::new(&asset_root).join("../splat"),
        &std::env::current_dir()?,
    )?;
    let battle_scene =
        startup_configuration::battle_scene(std::env::var("DF_GAMEPLAY_BATTLE_SCENE"))?;
    if first_run {
        journey::tactical::force_scene(battle_scene.as_deref()).map_err(std::io::Error::other)?;
    }
    // The display stages every fight on the scene the server resolved, which can be a newer
    // revision of the id asked for (a scan's collider-authored grid).
    let battle_scene = journey::tactical::forced_scene().map(str::to_owned);
    let test_fight = startup_configuration::test_fight(
        std::env::var("DF_GAMEPLAY_TEST_FIGHT"),
        std::env::var("DF_GAMEPLAY_TEST_FOE"),
    )?;
    if interactive && let Some(test) = &test_fight {
        println!(
            "TEST FIGHT enemies={} foe={} (development setting: the first choice starts the fight)",
            test.enemies,
            test.foe.as_deref().unwrap_or("the adventure's own")
        );
    }
    if first_run {
        journey::tactical::force_test_fight(test_fight).map_err(std::io::Error::other)?;
    }
    let battle_fps = startup_configuration::battle_fps(std::env::var("DF_GAMEPLAY_BATTLE_FPS"))?;
    let splat_lod = startup_configuration::splat_lod(std::env::var("DF_GAMEPLAY_SPLAT_LOD"))?;
    let battle_grid = startup_configuration::battle_grid(std::env::var("DF_GAMEPLAY_BATTLE_GRID"))?;
    let choice_hints_hidden = if interactive {
        startup_configuration::choice_hints_hidden(std::env::var("DF_GAMEPLAY_CHOICE_HINTS"))?
    } else {
        false
    };
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
    let mut settings = if interactive {
        startup_configuration::session_settings(
            startup_configuration::SessionInputs::from_environment(),
        )?
    } else {
        startup_configuration::SessionSettings::qualification()
    };
    if first_run && settings.listen.port() != 0 {
        join_link::install(&settings.join_origin)?;
    }
    let preparation = if interactive {
        Some(run_restart::preparing(settings.listen, settings.allowed_origins.clone()).await?)
    } else {
        None
    };
    if interactive {
        let rating = startup_configuration::content_rating_setting(std::env::var(
            "DF_GAMEPLAY_CONTENT_RATING",
        ))?;
        if first_run {
            startup_configuration::install_content_rating(rating)?;
        }
        println!("{}", rating.startup_line());
    }
    let resource_hub = resources::Hub::new();
    let resource_kit = resource_adapters::Kit::over(resource_hub.clone());
    startup.step("storyteller");
    let storyteller = std::sync::Arc::new(if interactive {
        let mode = startup_configuration::storyteller_mode(
            startup_configuration::StorytellerInputs::from_environment(),
            &std::env::current_dir()?,
        )?;
        match &mode {
            startup_configuration::StorytellerMode::On(settings) => {
                println!("STORYTELLER on model={}", settings.model);
            }
            startup_configuration::StorytellerMode::Off(reason) => {
                println!("STORYTELLER off ({reason})");
            }
        }
        storyteller::Storyteller::from_mode(mode)
    } else {
        storyteller::Storyteller::disabled()
    });
    storyteller.attach_kit(&resource_kit);
    startup.step("approach-model");
    let approach_model = approach_model::ApproachModel::configured_with_ledger(
        if interactive { jev_settings()? } else { None },
        resource_hub.ledger(),
    );
    if interactive && !media_policy::gates().allows(media_policy::Capability::Jev) {
        // The model's own line would blame a missing key; the switch turned it off.
        println!("APPROACH off (DF_GAMEPLAY_JEV=off): neutral reads only");
    } else {
        println!("{}", approach_model.startup_line());
    }
    startup.step("adventure");
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
            | startup_configuration::AdventureMode::Campaign
            | startup_configuration::AdventureMode::MerchantShop
            | startup_configuration::AdventureMode::FreeRoam => {
                let prepared_started = std::time::Instant::now();
                let mut prepared = adventure::startup::prepare(
                    settings,
                    &database,
                    &std::env::current_dir()?,
                    &storyteller,
                    &approach_model,
                )
                .await?;
                actor::record_audit_line(
                    "gameplay.adventure_prepared",
                    format!(
                        "ADVENTURE prepared fresh={} ms={} provider=storyteller attempts=unmeasured",
                        prepared.fresh,
                        prepared_started.elapsed().as_millis()
                    ),
                );
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
                if prepared
                    .loaded
                    .pack
                    .beat_script
                    .as_ref()
                    .is_some_and(|script| script.mode == beat_script::ScriptMode::Freeplay)
                {
                    println!(
                        "ADVENTURE mode=freeplay seed={} title={:?} cast={} rolls=0",
                        prepared.record.seed,
                        opening.title,
                        opening.cast.len()
                    );
                } else {
                    println!(
                        "ADVENTURE seed={} title={:?} location={} patron={} antagonist={} rolls={}",
                        prepared.record.seed,
                        opening.title,
                        prepared.record.entry("location")?,
                        prepared.record.entry("patron")?,
                        prepared.record.entry("antagonist")?,
                        prepared.record.rolls.len()
                    );
                }
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
        let run = database.get_dbname().unwrap_or_default().to_owned();
        loaded.pack.room_code = room_code::select(
            room_code::pinned(std::env::var("DF_GAMEPLAY_ROOM_CODE"))?,
            &run,
        );
        run_identity = room_code::run_identity(&run, &loaded.pack_revision.to_hex());
        run_restart::install_identity(&run_identity)?;
        println!("ROOM {}", loaded.pack.room_code);
        pack::install(loaded)?;
    }
    let stop_file = std::env::var_os("DF_GAMEPLAY_STOP_FILE").map(|base| {
        let mut path = std::path::PathBuf::from(base);
        path.push(format!("{}.stop", run_identity));
        path
    });
    if let Some(path) = &stop_file {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // Publish only the sanitized basename inside this launcher's own stop directory.
        // The launcher never guesses a marker belonging to another run.
        let marker = path
            .file_name()
            .ok_or_else(|| io::Error::other("stop marker absent"))?;
        std::fs::write(
            path.with_file_name("target.receipt"),
            marker.as_encoded_bytes(),
        )?;
        println!("STOP_REQUEST file={}", path.display());
    }
    // The join code encodes the room code the pack above has fixed, before anything is started.
    startup.step("join-code");
    let join_qr = join_link::JoinQr::for_room(&journey::room_code())
        .map_err(|_| io::Error::other("join QR code refused"))?;
    startup.step("voice");
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
    startup.step("speech-input");
    let speech = speech_input::SpeechInput::start(if interactive {
        speech_input::speech_settings(speech_input::SpeechInputs::from_environment())?
    } else {
        speech_input::SpeechSettings::Off(speech_input::SpeechOff::Qualification)
    })?;
    startup.step("qualification-settings");
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
    model::install_database_run(database.get_dbname().ok_or("run database missing")?)
        .map_err(|_| io::Error::other("run identity refused"))?;
    startup.step("initial-checkpoint");
    let cold =
        journey::initial().map_err(|_| io::Error::other("gameplay baseline validation failed"))?;
    let plates_enabled = startup_configuration::plates_enabled(
        media_policy::gates().input(media_policy::Capability::Plates),
    )?;
    let codec = live_codec_limits();
    startup.step("recovery-source");
    let recovery =
        model::recovery().map_err(|_| io::Error::other("source recovery inventory invalid"))?;
    // DEVCTL-001: admitted references for development candidate rebuilds. Used
    // only when the control surface is on; otherwise dropped unused.
    let dev_recovery = recovery.clone();
    startup.step("startup-identities");
    let fence = model::random::<16>().map_err(|_| io::Error::other("native fence unavailable"))?;
    let player_credential =
        model::random::<32>().map_err(|_| io::Error::other("local grant unavailable"))?;
    let display_credential =
        model::random::<32>().map_err(|_| io::Error::other("local grant unavailable"))?;

    startup.step("grant-connection");
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
    startup.step("session-admission");
    let initialization = df_persistence::local_demo_scope::admit_startup_with_adventure(
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
    )
    .await;
    startup.step("session-admission-result");
    let admitted = match initialization {
        Ok(admitted) => admitted,
        Err(error) => {
            actor::record_audit_line(
                "gameplay.startup_failed",
                format!(
                    "STARTUP failed step={} class={:?} ms={}",
                    error.stage,
                    error.class,
                    startup_started.elapsed().as_millis()
                ),
            );
            let refusal_report = if restart_phase == Some(restart_qualification::Phase::Contender) {
                restart_qualification::refusal(&error)
            } else if rest_phase == Some(rest_qualification::Phase::Contender) {
                rest_qualification::refusal(&error)
            } else {
                Ok(())
            };
            drop(grant_client);
            join_grant_driver(grant_driver).await?;
            refusal_report?;
            return Err(io::Error::other(format!(
                "owned gameplay admission refused: {} {:?}",
                error.stage, error.class
            ))
            .into());
        }
    };
    let revision = admitted.checkpoint.basis().revision;
    actor::record_audit_line(
        "gameplay.session_admitted",
        format!(
            "SESSION admitted restored={} rev={}:{} decisions={} participants={} ms={}",
            admitted.restored,
            revision.epoch().get(),
            revision.sequence(),
            admitted.checkpoint.state().decisions.len(),
            journey::participants(&admitted.checkpoint).len(),
            startup_started.elapsed().as_millis(),
        ),
    );
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
    voice
        .register_people(initial.state())
        .map_err(|_| io::Error::other("saved voice identities invalid"))?;
    let player_credential = admitted.player_credential;
    let display_credential = admitted.display_credential;
    let runtime = tokio::runtime::Handle::current();
    startup.step("repository-connection");
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
        maximum_receipt_bytes: df_persistence::local_demo_scope::RECEIPT_RETAINED_BYTES_MAX,
        verifier: Some(df_persistence::local_demo_scope::verifier()),
        recovery: Some(recovery),
    };
    startup.step("repository-setup");
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
    if let Some(preparation) = preparation {
        preparation.abort();
        if let Err(error) = preparation.await
            && !error.is_cancelled()
        {
            return Err(error.into());
        }
    }
    startup.step("http-listener");
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
    if settings.listen.port() == 0 {
        let bound = listener.local_addr()?;
        settings.resolve_bound_address(bound);
        if first_run {
            join_link::install(&settings.join_origin)?;
        }
    }
    startup.step("grant-issuer");
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
    // The table's settings: a choice a player stored earlier wins over the startup default.
    startup.step("table-settings");
    let table_settings = if interactive {
        match issuer.stored_table_settings().await {
            Ok(stored) => table_settings::TableSettings::startup(stored, choice_hints_hidden),
            Err(_) => {
                let closed = repository.close_owned().await;
                let released = issuer.release_owner_owned().await;
                let grants = issuer.close_owned().await;
                closed.map_err(|_| io::Error::other("table settings repository close failed"))?;
                released
                    .map_err(|_| io::Error::other("table settings exact-fence release failed"))?;
                grants.map_err(|_| io::Error::other("table settings grant close failed"))?;
                return Err(io::Error::other("table settings unavailable").into());
            }
        }
    } else {
        table_settings::TableSettings::startup(None, choice_hints_hidden)
    };
    if interactive {
        let shown = if table_settings.choice_hints_hidden {
            "off"
        } else {
            "on"
        };
        println!("TABLE choice_hints={shown}");
    }

    let (updates, receiver) = watch::channel(initial.clone());
    startup.step("campaign-worker");
    let (campaign_observer, campaign_worker) = if let Some(run) = generated_adventure
        .as_ref()
        .and_then(|prepared| prepared.campaign.clone())
    {
        let (observer, worker) = match adventure::campaign_completion::worker(run) {
            Ok(started) => started,
            Err(error) => {
                return startup_failure_after_admission(
                    error,
                    close_admitted_startup(&mut repository, &mut issuer, "campaign worker"),
                )
                .await
                .map_err(Into::into);
            }
        };
        observer.observe(&initial);
        (Some(observer), Some(worker))
    } else {
        (None, None)
    };
    let table_log = if interactive {
        table_log::TableLog::new()
    } else {
        table_log::TableLog::disabled()
    };
    // A generated adventure's NPCs have backstories and feelings about each hero; the harbor
    // pack and adventures saved before dossiers existed have none and keep today's behaviour.
    startup.step("social-state");
    let social = match &generated_adventure {
        // A run with no dossiers (free roam) still keeps who learned whose name (HS68).
        Some(prepared) => {
            println!(
                "ADVENTURE npc dossiers={} (attitude saved in {})",
                prepared.record.dossiers.len(),
                prepared.directory.display()
            );
            match social::SocialLedger::open(
                prepared.record.dossiers.clone(),
                &prepared.record.seed,
                Some(prepared.directory.clone()),
            ) {
                Ok(social) => social,
                Err(error) => {
                    return startup_failure_after_admission(
                        io::Error::other(error.to_string()),
                        close_admitted_startup(&mut repository, &mut issuer, "social ledger"),
                    )
                    .await
                    .map_err(Into::into);
                }
            }
        }
        _ => {
            // The built-in pack authors its own scene models; install their rooms before the
            // first view projects them. An interactive run keeps its social memory (names learned,
            // attitudes) in its own run directory, as local lore and the notebook do.
            let social = if interactive {
                let run = hex(initial.basis().run.as_bytes());
                let directory = std::env::current_dir()?
                    .join("development/runs/social")
                    .join(&run);
                if let Err(error) = std::fs::create_dir_all(&directory) {
                    return startup_failure_after_admission(
                        error,
                        close_admitted_startup(&mut repository, &mut issuer, "social directory"),
                    )
                    .await
                    .map_err(Into::into);
                }
                match social::SocialLedger::open(Vec::new(), &run, Some(directory)) {
                    Ok(social) => social,
                    Err(error) => {
                        return startup_failure_after_admission(
                            io::Error::other(error.to_string()),
                            close_admitted_startup(&mut repository, &mut issuer, "social ledger"),
                        )
                        .await
                        .map_err(Into::into);
                    }
                }
            } else {
                social::SocialLedger::none()
            };
            let scenes = pack::with_content(|content| {
                content
                    .scenes
                    .iter()
                    .chain(
                        content
                            .beat_script
                            .iter()
                            .flat_map(|script| script.scenes.values()),
                    )
                    .cloned()
                    .collect::<Vec<_>>()
            });
            if let Err(error) = social.install_pack_rooms(&scenes) {
                return startup_failure_after_admission(
                    io::Error::other(error.to_string()),
                    close_admitted_startup(&mut repository, &mut issuer, "social ledger"),
                )
                .await
                .map_err(Into::into);
            }
            social
        }
    };
    // NPC local canon (places, people, rumours they establish) persists beside the social ledger.
    startup.step("local-lore");
    let local_lore = match &generated_adventure {
        Some(prepared) => match npc_reply::local_lore::LocalLore::open(
            &prepared.record.seed,
            Some(prepared.directory.clone()),
        ) {
            Ok(lore) => Arc::new(lore),
            Err(error) => {
                return startup_failure_after_admission(
                    io::Error::other(error.to_string()),
                    close_admitted_startup(&mut repository, &mut issuer, "local lore"),
                )
                .await
                .map_err(Into::into);
            }
        },
        // An interactive run keeps the facts the narrative engine backfilled (REF-069) in its own
        // run directory, as the notebook does, so they survive a server restart.
        None if interactive => {
            let run = hex(initial.basis().run.as_bytes());
            let directory = std::env::current_dir()?
                .join("development/runs/lore")
                .join(&run);
            if let Err(error) = std::fs::create_dir_all(&directory) {
                return startup_failure_after_admission(
                    error,
                    close_admitted_startup(&mut repository, &mut issuer, "local lore directory"),
                )
                .await
                .map_err(Into::into);
            }
            match npc_reply::local_lore::LocalLore::open(&run, Some(directory)) {
                Ok(lore) => Arc::new(lore),
                Err(error) => {
                    return startup_failure_after_admission(
                        io::Error::other(error.to_string()),
                        close_admitted_startup(&mut repository, &mut issuer, "local lore"),
                    )
                    .await
                    .map_err(Into::into);
                }
            }
        }
        None => Arc::new(npc_reply::local_lore::LocalLore::in_memory()),
    };
    startup.step("notebook");
    let notebook = match &generated_adventure {
        Some(prepared) => match notebook::NotebookLedger::open(
            &prepared.record.seed,
            prepared.record.dossiers.clone(),
            prepared.record.fact_sheet.clone(),
            Some(&prepared.directory),
        ) {
            Ok(notebook) => notebook,
            Err(error) => {
                return startup_failure_after_admission(
                    io::Error::other(error.to_string()),
                    close_admitted_startup(&mut repository, &mut issuer, "notebook ledger"),
                )
                .await
                .map_err(Into::into);
            }
        },
        None if interactive => {
            let run = hex(initial.basis().run.as_bytes());
            let directory = std::env::current_dir()?
                .join("development/runs/notebook")
                .join(&run);
            if let Err(error) = std::fs::create_dir_all(&directory) {
                return startup_failure_after_admission(
                    error,
                    close_admitted_startup(&mut repository, &mut issuer, "notebook directory"),
                )
                .await
                .map_err(Into::into);
            }
            match notebook::NotebookLedger::open(&run, Vec::new(), Vec::new(), Some(&directory)) {
                Ok(notebook) => notebook,
                Err(error) => {
                    return startup_failure_after_admission(
                        io::Error::other(error.to_string()),
                        close_admitted_startup(&mut repository, &mut issuer, "notebook ledger"),
                    )
                    .await
                    .map_err(Into::into);
                }
            }
        }
        None => notebook::NotebookLedger::none(),
    };
    // DEVCTL-001: back/forward ring shared with the development surface (and the
    // owner's publication hook). Empty and unwired unless the surface is on.
    startup.step("development-history");
    let dev_history =
        std::sync::Arc::new(std::sync::Mutex::new(dev_control::history::History::new()));
    let (publication, intent_notifications) = actor::Publication::new(updates.clone());
    let mut publication = publication.with_campaign(campaign_observer.clone());
    // DEVCTL-001: the history bundle for the control surface; the publication
    // hook and the session owner share it only when the surface is on.
    // Otherwise no history is kept anywhere. The checkpoint travels with the
    // table-log floor and the social snapshot, so a rewind restores all of
    // it or none.
    let dev_bundle = dev_control::Bundle {
        history: dev_history,
        table: table_log.clone(),
        social: social.clone(),
        notebook: notebook.clone(),
        lore: local_lore.clone(),
    };
    let dev_on = dev_control::setting_on(settings.listen).unwrap_or(false);
    if dev_on {
        publication = publication.with_history(dev_bundle.clone());
    }
    let courier_reductions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    startup.step("durable-owner");
    let owner = match DurableOwner::new(
        repository,
        actor::Engine(courier_phase.map(|_| courier_reductions.clone())),
        publication,
        initial,
        df_persistence::local_demo_scope::RECEIPT_RETAINED_BYTES_MAX,
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
    // Living scenes: server-generated scene art that follows the story's mood, under the
    // server's one content rating.
    let rating = startup_configuration::content_rating();
    startup.step("imagery-configuration");
    let image_fixture = dev_control::direct::image_fixture(dev_on)?;
    let mut imagery_inputs = startup_configuration::ImageryInputs::from_environment();
    if image_fixture.is_some() {
        imagery_inputs.credential = Ok("test-credential".to_owned());
    }
    let imagery_settings = if interactive {
        startup_configuration::imagery_settings(imagery_inputs, &std::env::current_dir()?, rating)?
    } else {
        startup_configuration::ImagerySettings::Off(
            startup_configuration::ImageryOff::Qualification,
        )
    };
    let living_interval = match &imagery_settings {
        startup_configuration::ImagerySettings::On(configuration) => configuration.interval,
        startup_configuration::ImagerySettings::Off(_) => Duration::from_secs(30),
    };
    startup.step("imagery");
    let imagery = dev_control::direct::start_imagery(
        imagery_settings,
        generated_adventure
            .as_ref()
            .map(|prepared| prepared.directory.as_path()),
        image_fixture,
        resource_hub,
    )?;
    startup.step("living-scenes");
    let living = living_scenes::LivingScenes::start_with_local_lore(
        imagery.clone(),
        imagery::ReferenceStore::new(std::path::Path::new(&asset_root).join("generated")),
        (social.clone(), local_lore.clone()),
        rating,
        living_scenes::Timing::from_interval(living_interval),
        plates_enabled,
        receiver.clone(),
    );
    let splat_cache = std::env::current_dir()?.join("artifacts/cache");
    splat_world::register_stored_worlds(&splat_cache);
    splat_world::seed_proof_world(
        &splat_cache,
        &std::env::current_dir()?.join("artifacts/tmp/worldlabs/out/m11"),
    )?;
    startup.step("room-photograph");
    let room_photograph: splat_world::RoomPhotograph = {
        let living = living.clone();
        let imagery = imagery.clone();
        Arc::new(move |place| {
            let living = living.clone();
            let imagery = imagery.clone();
            let place = place.to_owned();
            Box::pin(async move {
                let depiction = living.describe_depiction(
                    df_render::art_contract::Role::GenerationReference,
                    "",
                    &df_render::art_contract::Subject::place(&place),
                )?;
                let key = depiction.asset.strip_prefix("/gameplay/imagery/")?;
                match imagery.image(key).await {
                    imagery::Lookup::Image { bytes, .. } => Some(bytes.to_vec()),
                    imagery::Lookup::Unregistered | imagery::Lookup::NotReady => None,
                }
            })
        })
    };
    startup.step("splat-worlds");
    let splat_worlds = match std::env::var("WORLDLABS_API_KEY") {
        Ok(key)
            if interactive
                && !key.is_empty()
                && media_policy::gates().allows(media_policy::Capability::SplatWorlds) =>
        {
            let maker = df_providers::NativeWorldLabs::new(
                df_providers::WorldLabsConfig::live(),
                &key,
                Duration::from_secs(180),
            )
            .map_err(|_| std::io::Error::other("World Labs configuration invalid"))?;
            splat_world::SplatWorlds::new(
                imagery.hub().clone(),
                Arc::new(maker),
                room_photograph,
                splat_cache,
            )
        }
        _ => splat_world::SplatWorlds::cached(imagery.hub().clone(), room_photograph, splat_cache),
    };
    splat_world::install(splat_worlds);
    let clip_source: character_clips::Source = {
        let living = living.clone();
        let imagery = imagery.clone();
        let references =
            imagery::ReferenceStore::new(std::path::Path::new(&asset_root).join("generated"));
        Arc::new(move |fighter| {
            let living = living.clone();
            let imagery = imagery.clone();
            let subject = fighter.depiction_subject.clone();
            let reviewed = fighter.reviewed_figure.clone();
            let references = references.clone();
            Box::pin(async move {
                let depiction = living.describe_depiction(
                    df_render::art_contract::Role::GenerationReference,
                    "",
                    &df_render::art_contract::Subject::character(subject),
                )?;
                if let Some(key) = depiction.asset.strip_prefix("/gameplay/imagery/") {
                    return match imagery.image(key).await {
                        imagery::Lookup::Image { bytes, format } => {
                            Some(character_clips::Depiction {
                                bytes: bytes.to_vec(),
                                format,
                                revision: key.to_owned(),
                            })
                        }
                        imagery::Lookup::Unregistered | imagery::Lookup::NotReady => None,
                    };
                }
                let reviewed = reviewed?;
                let reference = tokio::task::spawn_blocking(move || references.load(&reviewed))
                    .await
                    .ok()?
                    .ok()?;
                Some(character_clips::Depiction {
                    bytes: reference.bytes.to_vec(),
                    format: reference.format,
                    revision: reference.sha256,
                })
            })
        })
    };
    startup.step("character-clips");
    let _character_clip_worker = character_clips::start(
        imagery.hub().clone(),
        clip_source,
        std::env::current_dir()?.join("artifacts/cache/character-clips"),
        interactive,
        living.subscribe(),
    )?;
    startup.step("fight-foresight");
    let _fight_foresight_worker = fight_foresight::start(
        approach_model::ApproachModel::configured(if interactive { jev_settings()? } else { None }),
        table_log.clone(),
        receiver.clone(),
    );
    // PLAY-V10: the home screen's looping video, made from the still just in time.
    startup.step("home-video");
    let home_video_line = home_video::install(
        home_video::home_video_settings(
            home_video::HomeVideoInputs::from_environment(),
            &std::env::current_dir()?,
            !interactive,
        )?,
        std::path::Path::new(&asset_root),
        living.clone(),
    )?;
    // The voice, the video, the text and the transcription run on the one generation hub the
    // imagery owns, so every kind's state, failures and spend are listed in one place.
    voice.attach_kit(&resource_kit);
    speech.attach_kit(&resource_kit);
    home_video::attach_kit(&resource_kit)?;
    // The cost engine: every billed call on the hub's ledger is priced and kept per run.
    if interactive {
        println!(
            "{}",
            cost::open_run(
                imagery.hub().ledger(),
                &std::env::current_dir()?,
                database.get_dbname().unwrap_or_default(),
            )
        );
    }
    // The story memory: what the Dungeon Master remembers of this run, so the story stays
    // consistent (planning/story-memory.md). It is a help, never a gate: any failure turns it off.
    startup.step("memory");
    let memory_line = if interactive {
        // Keyed by the game's own database, which is unique per game and kept when it resumes;
        // the session basis carries a fixed placeholder run id, which every game would share.
        let run = database.get_dbname().unwrap_or_default().to_owned();
        let sources = memory::backfill::Sources {
            lore: local_lore.clone(),
            social: social.clone(),
            dossiers: generated_adventure
                .as_ref()
                .map(|prepared| prepared.record.dossiers.clone())
                .unwrap_or_default(),
            updates: receiver.clone(),
        };
        Some(
            memory::startup::start(
                &database,
                &run,
                resource_kit.hub(),
                &table_log,
                Some(sources),
                Some(storyteller.clone()),
                &tokio::runtime::Handle::current(),
            )
            .await
            .line,
        )
    } else {
        None
    };
    // Place music: the server's per-place spec and plan, with each place's set generated on
    // demand as a resource of the generation hub (the one the imagery owns). Generation needs
    // ELEVENLABS_API_KEY and ffmpeg; without them places keep the shared score and their beds.
    startup.step("music");
    let (music_line, music_routes) = if interactive {
        // Jev plans place music only when music may be generated.
        let jev = approach_model::ApproachModel::configured(
            if media_policy::gates().allows(media_policy::Capability::Music) {
                jev_settings()?
            } else {
                None
            },
        );
        let adapter = place_music::HubMusic::start(
            place_music::MusicInputs::from_environment(),
            &std::env::current_dir()?,
            imagery.hub().clone(),
        )?;
        let runtime = tokio::runtime::Handle::current();
        let wake = living.clone();
        adapter.start_worker(&runtime, {
            let wake = wake.clone();
            move || wake.signal_art()
        });
        let routes = adapter.router();
        let hub: Arc<dyn place_music::MusicHub> = adapter.clone();
        let music =
            place_music::PlaceMusic::new(jev, Some(hub), runtime, move || wake.signal_art());
        let line = format!("{}\n{}", adapter.startup_line(), music.startup_line());
        place_music::install(music);
        (Some(line), routes)
    } else {
        (None, Router::new())
    };
    startup.step("tempo");
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
    startup.step("actor-construction");
    let actor = actor::Actor {
        tempo,
        owner,
        table_log: table_log.clone(),
        notebook: notebook.clone(),
        social: social.clone(),
        // DEVCTL-001: the rewind entry refresh handle, set only when on.
        dev_history: dev_on.then(|| dev_bundle.history.clone()),
        table_settings,
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
        queued_wait: None,
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
    startup.step("owner-thread");
    let thread_setup = std::thread::Builder::new()
        .name("df-real-gameplay-owner".to_owned())
        // The owner projects whole views and replays receipts; give it the same headroom as the
        // runtime workers (see run_restart.rs).
        .stack_size(16 * 1024 * 1024)
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
            let release_started = std::time::Instant::now();
            let released = if drained.is_ok() {
                actor.issuer.release_owner()
            } else {
                Err(df_session::submission::RepositoryError::Unavailable)
            };
            actor::record_audit_line(
                "gameplay.shutdown",
                format!(
                    "SERVER shutdown step=fence_release outcome={} ms={}",
                    if released.is_ok() {
                        "released"
                    } else {
                        "failed"
                    },
                    release_started.elapsed().as_millis()
                ),
            );
            let remaining = actor.calls_remaining;
            let mut repository = actor.owner.into_repository();
            let close_started = std::time::Instant::now();
            let closed = repository.close();
            actor::record_audit_line(
                "gameplay.shutdown",
                format!(
                    "SERVER shutdown step=repo_close outcome={} ms={}",
                    if closed.is_ok() { "closed" } else { "failed" },
                    close_started.elapsed().as_millis()
                ),
            );
            let close_started = std::time::Instant::now();
            let grant_closed = actor.issuer.close();
            actor::record_audit_line(
                "gameplay.shutdown",
                format!(
                    "SERVER shutdown step=grant_close outcome={} ms={}",
                    if grant_closed.is_ok() {
                        "closed"
                    } else {
                        "failed"
                    },
                    close_started.elapsed().as_millis()
                ),
            );
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
    startup.step("creator-portraits");
    let portraits =
        creator_portrait_startup::start(imagery.hub().clone(), &std::env::current_dir()?)?;
    let mut restart_updates = receiver.clone();
    // Configured after the cost ledger opened: a Jev client configured before it has no meter
    // and refuses every call, so no spoken line would reach a person (#1478).
    let dialogue_judge =
        approach_model::ApproachModel::configured(if interactive { jev_settings()? } else { None });
    let service = Service {
        actor: sender.clone(),
        display_credential,
        updates: receiver,
        streams: Arc::new(Semaphore::new(settings.max_watch_streams)),
        voice: voice.clone(),
        npc_voice: Arc::new(
            npc_reply::NpcVoice::new(storyteller)
                .with_approach_model(dialogue_judge)
                .with_local_lore(local_lore),
        ),
        // DEVCTL-001: cloned, not moved: the control surface takes one below.
        table_log: table_log.clone(),
        social: social.clone(),
        living,
        look_ahead: Arc::default(),
        layout: dev_control::layout::LayoutState::new(),
        portraits,
    };
    startup.step("free-roam-neighbours");
    service.prepare_free_roam_current().await?;
    fight_foresight::bind_service(service.clone());
    // DEVCTL-001: the agent control surface. Off by default: with the setting
    // off this returns no routes and touches nothing (no file, no behaviour).
    startup.step("development-control");
    let dev_control = dev_control::setup(
        service.clone(),
        display_credential,
        settings.listen,
        &database,
        &std::env::current_dir()?,
        dev_recovery,
        dev_bundle.clone(),
        service.layout.clone(),
    )?;
    startup.step("rpc-bridge");
    let (incoming_sender, incoming) =
        NativeIncoming::bounded(NonZeroUsize::new(4).ok_or("invalid native incoming bound")?)?;
    let grpc = tonic::transport::Server::builder()
        .initial_stream_window_size(MESSAGE_BYTES as u32)
        .initial_connection_window_size(df_rpc_bridge::RECEIVE_BYTES as u32)
        .max_frame_size(FRAME_BYTES as u32)
        .http2_max_header_list_size(FRAME_BYTES as u32)
        .max_concurrent_streams(CONCURRENT_STREAMS)
        .add_service(
            rpc::presentation_telemetry_service_server::PresentationTelemetryServiceServer::new(
                presentation_ingress::PresentationIngress::new(service.clone()),
            )
            .max_decoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES)
            .max_encoding_message_size(1024),
        )
        .add_service(
            rpc::dialogue_service_server::DialogueServiceServer::new(service.clone())
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        )
        .add_service(
            rpc::hero_library_service_server::HeroLibraryServiceServer::new(service.clone())
                .max_decoding_message_size(1024)
                .max_encoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES),
        )
        .add_service(
            rpc::action_service_server::ActionServiceServer::new(service.clone())
                .max_decoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES)
                .max_encoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES),
        )
        .add_service(
            // A tactical view (cells, targets, budget, table log) is larger than a small
            // request: a view may be as large as one RPC message.
            rpc::session_service_server::SessionServiceServer::new(service.clone())
                .max_decoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES)
                .max_encoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES),
        )
        .add_service(
            rpc::room_service_server::RoomServiceServer::new(service.clone())
                .max_decoding_message_size(8192)
                .max_encoding_message_size(8192),
        )
        .serve_with_incoming(incoming);
    let player_page_actor = sender.clone();
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
        splat_lod,
        battle_grid,
    };
    let runtime_routes = runtime_launch::router(sender.clone(), state.clone(), runtime_secret)?;
    let player_page_run_identity = state.run_identity.clone();
    startup.step("http-router");
    let mut router = Router::new()
        .route("/gameplay", get(both))
        .route(
            "/gameplay/player",
            get(move || {
                room::player_page(player_page_actor.clone(), player_page_run_identity.clone())
            }),
        )
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
        .merge(splat_world::router())
        .merge(voice::router(voice.clone()))
        .merge(imagery::router(imagery.clone()))
        .merge(creator_portraits::router(service.clone()))
        .merge(hero_library::router())
        .merge(launcher_status::router(service.clone()))
        .merge(runtime_routes)
        .merge(music_routes)
        .merge(home_video::router())
        .merge(speech_input::router(
            speech.clone(),
            speech_input::actor_identity(sender.clone()),
            settings.allowed_origins.clone().into(),
        ))
        .merge(join_link::router(join_qr));
    if crate::fake_profile::is_enabled() {
        // Axum selects this exact static fake image before the existing dynamic imagery route.
        router = router.route_service(
            crate::fake_profile::PORTRAIT_PATH,
            tower_http::services::ServeFile::new(
                std::path::Path::new(&asset_root).join("scenes/mara-harbor-v4.png"),
            ),
        );
    }
    // DEVCTL-001: merged only when the surface is on; otherwise the game
    // router is exactly what it was.
    if let Some(routes) = dev_control.router {
        router = router.merge(routes);
    }
    let router = router.layer(axum::middleware::map_response(
        |mut response: axum::response::Response| async move {
            response.headers_mut().insert(
                axum::http::header::REFERRER_POLICY,
                axum::http::HeaderValue::from_static("no-referrer"),
            );
            response
        },
    ));
    // Kept before `service` moves into qualification so shutdown can stop free-roam look-ahead.
    let free_roam_look_ahead = service.look_ahead.clone();
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
        if let Some(line) = &memory_line {
            println!("{line}");
        }
        if let Some(line) = &music_line {
            println!("{line}");
        }
        println!("{home_video_line}");
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
        // DEVCTL-001: the one startup line for the control surface. The token
        // itself is never printed; the client reads the file.
        if let Some(line) = &dev_control.startup_line {
            println!("{line}");
        }
    }
    // DEVCTL-001: an authenticated POST /dev/shutdown ends the serve loop here,
    // so the teardown below releases the owner lease and the database resumes
    // immediately. With the surface off the sender is gone and this arm waits
    // forever.
    let mut dev_shutdown = dev_control.shutdown.clone();
    let mut task_panic = None;
    let mut stop_poll = tokio::time::interval(Duration::from_secs(1));
    stop_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let termination = async { terminate.recv().await };
    #[cfg(not(unix))]
    let termination = std::future::pending::<Option<()>>();
    #[cfg(windows)]
    let console_stopped = async {
        while !*console_stop.borrow() {
            if console_stop.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(windows))]
    let console_stopped = std::future::pending::<()>();
    startup.finish("ready");
    actor::record_audit_line(
        "gameplay.ready",
        format!(
            "READY startup_ms={} ms={}",
            startup_started.elapsed().as_millis(),
            startup_started.elapsed().as_millis()
        ),
    );
    let mut new_adventure = false;
    let outcome: Result<(), Box<dyn std::error::Error>> = {
        tokio::select! {
            result=grpc=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=grpc_error uptime_s={}", startup_started.elapsed().as_secs())); result.map_err(Into::into) },
            result=axum::serve(listener,router.into_make_service_with_connect_info::<std::net::SocketAddr>())=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=http_error uptime_s={}", startup_started.elapsed().as_secs())); result.map_err(Into::into) },
            _=async {
                loop {
                    if journey::new_adventure_requested(&restart_updates.borrow()) { break; }
                    if restart_updates.changed().await.is_err() { std::future::pending::<()>().await; }
                }
                // Publish preparation and return the receipt before retiring transports.
                tokio::time::sleep(Duration::from_millis(500)).await;
            }=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=new_adventure uptime_s={}", startup_started.elapsed().as_secs())); new_adventure = true; Ok(()) },
            result=tokio::signal::ctrl_c()=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=ctrl_c uptime_s={}", startup_started.elapsed().as_secs())); result.map_err(Into::into) },
            _=console_stopped=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=console uptime_s={}", startup_started.elapsed().as_secs())); Ok(()) },
            _=termination=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=sigterm uptime_s={}", startup_started.elapsed().as_secs())); Ok(()) },
            _=async {
                loop {
                    stop_poll.tick().await;
                    if stop_file.as_ref().is_some_and(|path| path.is_file()) {
                        break;
                    }
                }
            }=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=stop_file uptime_s={}", startup_started.elapsed().as_secs())); Ok(()) },
            _=async{match settings.session{Some(session)=>tokio::time::sleep(session).await,None=>std::future::pending().await}}=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=session_timer uptime_s={}", startup_started.elapsed().as_secs())); Ok(()) },
            _=async move {
                while !*dev_shutdown.borrow() {
                    if dev_shutdown.changed().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                }
            }=>{ actor::record_audit_line("gameplay.stopping", format!("SERVER stopping reason=dev_shutdown uptime_s={}", startup_started.elapsed().as_secs())); Ok(()) },
        }
    };
    free_roam_look_ahead.cancel();
    let mut shutdown = AuditStage::new("SERVER shutdown", "consumer");
    let qualification_outcome = if let Some((cancel, task)) = qualification {
        let _ = cancel.send(());
        match task.await {
            Ok(result) => result,
            Err(error) if error.is_panic() => {
                task_panic = Some(error.into_panic());
                Err(io::Error::other("qualification consumer panicked"))
            }
            Err(_) => Err(io::Error::other("qualification consumer did not finish")),
        }
    } else {
        Ok(())
    };
    let courier_outcome = if let Some(mut task) = courier {
        match tokio::time::timeout(Duration::from_secs(47), &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) if error.is_panic() => {
                task_panic.get_or_insert_with(|| error.into_panic());
                Err(io::Error::other("courier process consumer panicked"))
            }
            Ok(Err(_)) => Err(io::Error::other("courier process consumer cancelled")),
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
            Ok(Err(error)) if error.is_panic() => {
                task_panic.get_or_insert_with(|| error.into_panic());
                Err(io::Error::other(
                    "restart consumer panicked; cleanup required",
                ))
            }
            Ok(Err(_)) => Err(io::Error::other(
                "restart consumer cancelled; cleanup required",
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
            Ok(Err(error)) if error.is_panic() => {
                task_panic.get_or_insert_with(|| error.into_panic());
                Err(io::Error::other("rest consumer panicked; cleanup required"))
            }
            Ok(Err(_)) => Err(io::Error::other(
                "rest consumer cancelled; cleanup required",
            )),
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
            Ok(Err(error)) if error.is_panic() => {
                task_panic.get_or_insert_with(|| error.into_panic());
                Err(io::Error::other(
                    "courier NPC consumer panicked; cleanup required",
                ))
            }
            Ok(Err(_)) => Err(io::Error::other(
                "courier NPC consumer cancelled; cleanup required",
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
            Ok(Err(error)) if error.is_panic() => {
                task_panic.get_or_insert_with(|| error.into_panic());
                Err(io::Error::other("inn consumer panicked; cleanup required"))
            }
            Ok(Err(_)) => Err(io::Error::other("inn consumer cancelled; cleanup required")),
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
    shutdown.finish(
        if qualification_outcome.is_ok()
            && courier_outcome.is_ok()
            && restart_outcome.is_ok()
            && rest_outcome.is_ok()
            && npc_outcome.is_ok()
            && inn_outcome.is_ok()
        {
            "closed"
        } else {
            "failed"
        },
    );
    shutdown.step("actor-join");
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
    shutdown.step("actor-drain");
    // The cost log: every queued line is written before the run ends.
    tokio::task::spawn_blocking(cost::close_run).await?;
    drained.map_err(|_| io::Error::other("gameplay actor drain failed"))?;
    released.map_err(|_| io::Error::other("gameplay exact-fence release failed"))?;
    if interactive {
        println!("SESSION ownership released");
    }
    closed.map_err(|_| io::Error::other("gameplay repository close failed"))?;
    grant_closed.map_err(|_| io::Error::other("gameplay grant close failed"))?;
    shutdown.finish("closed");
    courier_committed?;
    courier_outcome?;
    qualification_outcome?;
    restart_outcome?;
    rest_outcome?;
    npc_outcome?;
    inn_outcome?;
    if let Some(payload) = task_panic {
        std::panic::resume_unwind(payload);
    }
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
    outcome?;
    Ok(new_adventure)
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

async fn startup_failure_after_admission<T>(
    error: io::Error,
    cleanup: impl std::future::Future<Output = Result<(), io::Error>>,
) -> Result<T, io::Error> {
    cleanup.await?;
    Err(error)
}

async fn close_admitted_startup(
    repository: &mut PostgresRepository<LocalDemoAuthority>,
    issuer: &mut LocalDemoScopeIssuer,
    step: &str,
) -> Result<(), io::Error> {
    let released = issuer.release_owner_owned().await;
    let closed = repository.close_owned().await;
    let grants = issuer.close_owned().await;
    released.map_err(|_| io::Error::other(format!("{step} exact-fence release failed")))?;
    closed.map_err(|_| io::Error::other(format!("{step} repository close failed")))?;
    grants.map_err(|_| io::Error::other(format!("{step} grant close failed")))
}

#[cfg(test)]
mod startup_cleanup_tests {
    use super::startup_failure_after_admission;
    use std::io;

    #[tokio::test]
    async fn post_admission_failure_awaits_cleanup_before_returning_the_startup_error() {
        let mut cleaned = false;
        let result: Result<(), io::Error> =
            startup_failure_after_admission(io::Error::other("worker creation failed"), async {
                cleaned = true;
                Ok(())
            })
            .await;

        assert_eq!(result.unwrap_err().to_string(), "worker creation failed");
        assert!(cleaned);
    }
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
    startup_configuration::database_config(std::env::var("DF_GAMEPLAY_DEMO_DATABASE"))
}
#[cfg(test)]
mod battle_scene_page_tests {
    use super::page;

    #[test]
    fn the_display_page_names_the_forced_battle_scene_and_the_player_page_never_does() {
        let credential = Some([7u8; 32]);
        let forced = page(false, credential, Some("scan-village"), None, 24, 2, true).0;
        assert!(forced.contains("data-battle-grid=\"on\""));
        assert!(forced.contains("data-battle-scene=\"scan-village\""));
        assert!(forced.contains("data-battle-fps=\"24\""));
        assert!(forced.contains("data-battle-splat-lod=\"2\""));
        let default = page(false, credential, None, None, 30, 1, false).0;
        assert!(!default.contains("data-battle-grid"));
        assert!(!default.contains("data-battle-scene"));
        assert!(default.contains("data-battle-fps=\"30\""));
        let player = page(true, None, Some("scan-village"), None, 24, 1, true).0;
        assert!(!player.contains("data-battle-grid"));
        assert!(!player.contains("data-battle-scene"));
        assert!(!player.contains("data-battle-fps"));
        assert!(!player.contains("data-battle-splat-lod"));
    }

    #[test]
    fn the_player_page_includes_and_escapes_the_opening_scene_asset() {
        let player = page(
            true,
            None,
            None,
            Some("assets/ui/generated/opening?x=\"&<>'"),
            30,
            1,
            false,
        )
        .0;

        assert!(player.contains(
            "data-join-scene-asset=\"assets/ui/generated/opening?x=&quot;&amp;&lt;&gt;&#39;\""
        ));
    }
}

#[cfg(test)]
mod lifecycle_audit_tests {
    use super::*;
    #[test]
    fn failed_startup_phase_emits_one_terminal_line_when_owner_returns_early() {
        {
            let _phase = AuditStage::new("STARTUP", "synthetic-audit-failure");
        }
        let lines =
            actor::audit_lines_containing("STARTUP step=synthetic-audit-failure outcome=failed");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("ms="));
    }
}

#[cfg(test)]
mod ingress_audit_tests {
    use super::*;
    #[test]
    fn malformed_creation_keeps_the_phone_operation_and_known_scope_in_its_refusal() {
        let basis = model::initial().unwrap().basis();
        let operation = vec![9; 16];
        let request = rpc::SubmitActionRequest {
            operation_id: Some(rpc::OperationId {
                value: Some(operation.clone()),
            }),
            action_kind: rpc::GameplayActionKind::CreateCharacter as i32,
            character: Some(rpc::CharacterSelection {
                name: "private-name".repeat(5),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut audit = submit_audit_context(&request, basis);
        audit.seat = Some(df_observe::seat_hash(&[8; 32]));
        let _scope = df_observe::audit_scope(audit);
        assert_eq!(
            validate_submit_shape(&request).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
        let lines = actor::audit_lines_containing("Capacity field=character_name_bytes actual=60");
        assert!(lines.iter().any(|line| line.contains(&format!(
            "op={}",
            crate::client_diagnostics::operation_hash(&operation)
        ))));
        assert!(lines.iter().all(|line| !line.contains("private-name")));
    }
}
