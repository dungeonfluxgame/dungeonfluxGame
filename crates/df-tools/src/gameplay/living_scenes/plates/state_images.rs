//! Settled visible changes share one edit; object cards share one request per object state.

use super::*;
use crate::gameplay::{
    adventure::scene_model::{SceneNow, SceneThing},
    imagery::{item_picture, state_edit},
    narrative_focus::{self, Dialogue, NarrativeFocus, Presentation, Stage},
};

const SETTLE: Duration = Duration::from_secs(3);
const BAKE_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct Book {
    scene: String,
    observed: String,
    observed_graph: u64,
    pub(super) pending: Option<Pending>,
    pub(super) flying: bool,
    pub(super) last_started: Option<Duration>,
    last_shown: Option<Duration>,
    pub(super) shown: Option<(ImageKey, Option<PlatePlacement>)>,
    pub(super) baked_revision: Option<u64>,
    ready: Option<(ImageKey, u64, Option<String>)>,
    pub(super) items: BTreeMap<String, Item>,
    pub(super) latest_items: BTreeMap<String, String>,
    last_story_actor: Option<String>,
    focus: Option<Presentation>,
}

pub(super) struct Pending {
    snapshot: Option<SceneNow>,
    pub(super) story: Option<Box<StoryBeat>>,
    reason: String,
    pub(super) state: String,
    pub(super) revision: u64,
    changed_at: Duration,
    graph: Option<crate::gameplay::shot::scene_graph::SceneGraph>,
    pub(super) focus: Option<NarrativeFocus>,
}

fn record_focus(focus: &Presentation, failure: Option<&str>) {
    let mut fields = focus.public_view().as_object().cloned().unwrap_or_default();
    fields.insert("resource_type".into(), serde_json::json!("plate"));
    fields.insert(
        "focus_kind".into(),
        serde_json::json!(focus.focus.subject.kind()),
    );
    fields.insert(
        "focus_id".into(),
        serde_json::json!(focus.focus.subject.id()),
    );
    fields.insert(
        "dialogue_sequence".into(),
        serde_json::json!(focus.focus.dialogue_sequence),
    );
    fields.insert("failure_category".into(), serde_json::json!(failure));
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.narrative.visual_focus",
        format!(
            "NARRATIVE visual_focus sequence={} kind={} id={} stage={:?} request_revision={} published_revision={:?} image_key={} elapsed_ms={}",
            focus.focus.dialogue_sequence,
            focus.focus.subject.kind(),
            focus.focus.subject.id(),
            focus.stage,
            focus.request_revision,
            focus.published_revision,
            focus.image_key.as_deref().unwrap_or("none"),
            focus.elapsed_ms
        ),
        fields,
        &[
            ("narrative.focus_id", focus.focus.subject.id().to_owned()),
            (
                "narrative.dialogue_sequence",
                focus.focus.dialogue_sequence.to_string(),
            ),
        ],
    );
}

fn focus_progress(
    book: &mut Book,
    state: &str,
    revision: u64,
    stage: Stage,
    image: Option<&ImageKey>,
    now: Duration,
    failure: Option<&str>,
) {
    if book.observed != state {
        return;
    }
    if let Some(focus) = book
        .focus
        .as_mut()
        .filter(|focus| focus.request_revision == revision)
    {
        focus.stage = stage;
        focus.elapsed_ms = now.saturating_sub(focus.selected_at).as_millis();
        if let Some(image) = image {
            focus.image_key = Some(image.as_str().to_owned());
        }
        if stage == Stage::Published {
            focus.published_revision = Some(revision);
        }
        record_focus(focus, failure);
    }
}

fn story_camera(
    pending: &Pending,
    identities: &IdentitySnapshot,
) -> Option<crate::gameplay::shot::Camera> {
    let story = pending.story.as_deref()?;
    let base = identities.scene.stage(&identities.heroes).ok()?.camera;
    Some(crate::gameplay::shot::Camera::for_public_beat(
        base,
        identities.scene.key,
        story.tempo,
        story.sequence,
        story.dialogue,
    ))
}

#[derive(Clone)]
pub(super) struct Item {
    pub(super) thing: SceneThing,
    pub(super) detail: Option<String>,
    pub(super) key: Option<ImageKey>,
    pub(super) failed: bool,
    pub(super) shared: bool,
    pub(super) requested_at: Duration,
}

/// Media decorates the server's latest durable item selection; stale requests cannot replace
/// a newer purchase/examination or fabricate ownership on reconnect.
fn enrich_item_card(card: &mut rpc::SceneItemCard, item: &Item) {
    if card.thing_id != item.thing.id || card.detail != item.thing.detail {
        return;
    }
    if let Some(detail) = &item.detail {
        card.detail.push_str(&format!("\n{detail}"));
    }
    card.image_path = item.key.as_ref().map(ImageKey::path).unwrap_or_default();
    card.image_state = if item.key.is_some() {
        rpc::PlateState::Ready
    } else if item.failed {
        rpc::PlateState::Failed
    } else {
        rpc::PlateState::Pending
    } as i32;
}

pub(super) enum Job {
    Bake {
        scene: String,
        pending: Pending,
        base: ImageKey,
        identities: Option<Box<IdentitySnapshot>>,
    },
    Item {
        id: String,
        thing: SceneThing,
        detail: Option<String>,
        base: Option<ImageKey>,
    },
}

pub(super) struct IdentitySnapshot {
    scene: PlateScene,
    heroes: Vec<u8>,
}

pub(super) enum Done {
    Bake {
        scene: String,
        state: String,
        revision: u64,
        story_actor: Option<String>,
        result: Result<ImageKey, ImageFailure>,
    },
    Item {
        id: String,
        result: Result<ImageKey, ImageFailure>,
    },
}

pub(super) fn reset_scene(book: &mut super::Book, view: &View) -> bool {
    let images = &mut book.state_images;
    if images.scene == view.scene_key {
        return false;
    }
    // A retained public log row from the previous scene cannot start a new scene's edit.
    let retained_story = view
        .story
        .as_ref()
        .filter(|story| images.observed == story.sha256)
        .map(|story| story.sha256.clone())
        .unwrap_or_default();
    *images = Book {
        scene: view.scene_key.clone(),
        observed: retained_story,
        ..Book::default()
    };
    true
}

pub(super) fn observe(book: &mut super::Book, view: &View, scene: &PlateScene, now: Duration) {
    let scene_changed = reset_scene(book, view);
    let Some(snapshot) = &scene.snapshot else {
        observe_story(book, view, scene, now);
        return;
    };
    let Ok(state) = snapshot.model.picture_state() else {
        record(
            "gameplay.living_scene.invalid_snapshot",
            &[("scene", view.scene_key.clone())],
        );
        return;
    };
    let focus = view.story.as_ref().and_then(|story| {
        let actor = scene.identity_cast.iter().find(|actor| {
            if story.actor_id.is_empty() {
                crate::voice_line::speaker_names_match(&actor.name, &story.speaker)
            } else {
                actor.id.strip_prefix("npc:").unwrap_or(&actor.id)
                    == story
                        .actor_id
                        .strip_prefix("npc:")
                        .unwrap_or(&story.actor_id)
            }
        })?;
        narrative_focus::select(
            &snapshot.model,
            Dialogue {
                sequence: story.sequence,
                actor_id: actor.id.strip_prefix("npc:").unwrap_or(&actor.id),
                text: &story.text,
            },
        )
    });
    let images = &mut book.state_images;
    if images
        .focus
        .as_ref()
        .is_some_and(|focus| !focus.focus.subject.remains_current(&snapshot.model))
    {
        images.focus = None;
    }
    let focus_changed = focus.as_ref().is_some_and(|focus| {
        images
            .focus
            .as_ref()
            .is_none_or(|known| known.focus.subject != focus.subject)
    });
    if focus_changed && let Some(focus) = focus {
        images.focus = Some(Presentation {
            focus,
            request_revision: view.graph_revision,
            published_revision: None,
            image_key: None,
            stage: Stage::Selected,
            selected_at: now,
            elapsed_ms: 0,
        });
        if let Some(focus) = images.focus.as_ref() {
            record_focus(focus, None);
        }
    }
    let state = match images.focus.as_ref() {
        Some(focus) => match serde_json::to_string(&focus.focus.subject) {
            Ok(subject) => format!("{state}|narrative_focus={subject}"),
            Err(_) => {
                record("gameplay.living_scene.invalid_focus", &[]);
                return;
            }
        },
        None => state,
    };
    if scene_changed && !focus_changed {
        images.observed = state;
        images.observed_graph = view.graph_revision;
        return;
    }
    // Who speaks and their expression change the graph, not the room's visible state.
    if images.observed == state {
        return;
    }
    if let Some(focus) = images
        .focus
        .as_mut()
        .filter(|focus| focus.request_revision != view.graph_revision)
    {
        focus.request_revision = view.graph_revision;
        focus.published_revision = None;
        focus.image_key = None;
        focus.stage = Stage::Selected;
        focus.selected_at = now;
        focus.elapsed_ms = 0;
        record_focus(focus, None);
    }
    images.observed = state.clone();
    images.observed_graph = view.graph_revision;
    images.ready = None;
    let reason = if focus_changed {
        "committed dialogue selects a canonical narrative subject".to_owned()
    } else {
        snapshot
            .changes
            .last()
            .cloned()
            .unwrap_or_else(|| "visible scene state changed".into())
    };
    let reason = images.pending.as_ref().map_or(reason.clone(), |pending| {
        format!("{}; {reason}", pending.reason)
    });
    images.pending = Some(Pending {
        snapshot: Some(snapshot.clone()),
        story: if focus_changed {
            view.story.clone().map(Box::new)
        } else {
            None
        },
        reason,
        state,
        revision: view.graph_revision,
        changed_at: now,
        graph: None,
        focus: images.focus.as_ref().map(|focus| focus.focus.clone()),
    });
}

fn observe_story(book: &mut super::Book, view: &View, scene: &PlateScene, now: Duration) {
    let images = &mut book.state_images;
    let Some(story) = &view.story else {
        refresh_decision(book, "story", "no_committed_npc_reply");
        return;
    };
    if !scene.identity_cast.iter().any(|member| {
        if story.actor_id.is_empty() {
            member.name == story.speaker
        } else {
            member.id.strip_prefix("npc:") == Some(story.actor_id.as_str())
        }
    }) {
        refresh_decision(book, "story", "speaker_not_in_scene");
        return;
    }
    if images.observed == story.sha256 {
        refresh_decision(book, "story", "unchanged_reply");
        return;
    }
    images.observed = story.sha256.clone();
    images.observed_graph = view.graph_revision;
    images.ready = None;
    images.pending = Some(Pending {
        snapshot: None,
        story: Some(Box::new(story.clone())),
        reason: "completed NPC story beat".to_owned(),
        state: story.sha256.clone(),
        revision: view.graph_revision,
        changed_at: now,
        graph: None,
        focus: None,
    });
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.living_scene.plate_story_observed",
        format!(
            "PLATE story sequence={} sha256={} revision={}",
            story.sequence, story.sha256, view.graph_revision
        ),
        serde_json::Map::from_iter([
            (
                "story_sequence".to_owned(),
                serde_json::json!(story.sequence),
            ),
            ("story_sha256".to_owned(), serde_json::json!(story.sha256)),
            (
                "graph_revision".to_owned(),
                serde_json::json!(view.graph_revision),
            ),
        ]),
        &[
            ("story.sequence", story.sequence.to_string()),
            ("story.sha256", story.sha256.clone()),
        ],
    );
    refresh_decision(book, "story", "pending_settle");
}

impl Shared {
    pub(super) fn project_scene_contents(
        &self,
        scene: &mut rpc::GameplayScene,
        hero: Option<&str>,
    ) {
        let book = locked(&self.book);
        if book
            .current
            .as_ref()
            .is_none_or(|current| current.scene_key != scene_identity("", scene).0)
        {
            return;
        }
        let current = book
            .current
            .as_ref()
            .and_then(|current| current.scene.as_ref());
        if scene.scene_contents_json.is_empty()
            && let Some(snapshot) = current.and_then(|scene| scene.snapshot.as_ref())
        {
            scene.scene_contents_json = snapshot.model.public_contents().unwrap_or_else(|error| {
                record(
                    "gameplay.living_scene.contents_failed",
                    &[("reason", error.to_string())],
                );
                String::new()
            });
        }
        if let Some(focus) = book.state_images.focus.as_ref() {
            match serde_json::from_str::<serde_json::Value>(&scene.scene_contents_json) {
                Ok(mut contents) => {
                    contents["narrative_focus"] = focus.public_view();
                    match serde_json::to_string(&contents) {
                        Ok(contents) => scene.scene_contents_json = contents,
                        Err(error) => record(
                            "gameplay.living_scene.contents_failed",
                            &[("reason", error.to_string())],
                        ),
                    }
                }
                Err(error) => record(
                    "gameplay.living_scene.contents_failed",
                    &[("reason", error.to_string())],
                ),
            }
        }
        let item = match hero {
            Some(hero) => book
                .state_images
                .latest_items
                .get(hero)
                .and_then(|id| book.state_images.items.get(id)),
            None => book
                .state_images
                .items
                .values()
                .filter(|item| {
                    item.shared
                        && self.inner.now().saturating_sub(item.requested_at)
                            < Duration::from_secs(8)
                })
                .max_by_key(|item| item.requested_at),
        };
        if hero.is_some() {
            if let (Some(card), Some(item)) = (scene.item.as_mut(), item) {
                enrich_item_card(card, item);
            }
            return;
        }
        scene.item = item.map(|item| rpc::SceneItemCard {
            thing_id: item.thing.id.clone(),
            name: item.thing.name.clone(),
            detail: item.detail.as_ref().map_or_else(
                || item.thing.detail.clone(),
                |trace| format!("{}\n{trace}", item.thing.detail),
            ),
            image_path: item.key.as_ref().map(ImageKey::path).unwrap_or_default(),
            image_state: if item.key.is_some() {
                rpc::PlateState::Ready
            } else if item.failed {
                rpc::PlateState::Failed
            } else {
                rpc::PlateState::Pending
            } as i32,
            state_revision: item_revision(&item.thing, item.detail.as_deref()),
            observation_only: item.thing.holder_id.is_some()
                && item.thing.owner_id.as_deref() != item.thing.holder_id.as_deref(),
            holder: item.thing.holder_id.clone().unwrap_or_default(),
        });
    }

    pub(super) fn poll_state_images(&self) {
        self.poll_state_images_at(self.inner.now());
    }

    pub(super) fn poll_state_images_at(&self, now: Duration) {
        let mut book = locked(&self.book);
        let Some(current) = &book.current else {
            return;
        };
        let blocked = if current.in_combat {
            Some("combat")
        } else if current.in_roll {
            Some("roll")
        } else if book.state_images.flying {
            Some("render_in_flight")
        } else {
            None
        };
        if let Some(reason) = blocked {
            refresh_decision(&mut book, "state", reason);
            return;
        }
        let scene = current.scene_key.clone();
        let identities = current.scene.clone().map(|scene| {
            Box::new(IdentitySnapshot {
                scene,
                heroes: current.heroes.clone(),
            })
        });
        let can_show = book
            .state_images
            .last_shown
            .is_none_or(|last| now.saturating_sub(last) >= BAKE_INTERVAL);
        if can_show && let Some((key, revision, story_actor)) = book.state_images.ready.take() {
            let layout = book.image_layouts.get(key.as_str()).cloned();
            let observed = book.state_images.observed.clone();
            focus_progress(
                &mut book.state_images,
                &observed,
                revision,
                Stage::Published,
                Some(&key),
                now,
                None,
            );
            book.edit = Edit::Idle;
            book.state_images.shown = Some((key, layout));
            book.state_images.baked_revision = Some(revision);
            book.state_images.last_shown = Some(now);
            if let Some(actor) = story_actor {
                book.state_images.last_story_actor = Some(actor);
            }
            self.wake();
        }
        let Some(pending) = &book.state_images.pending else {
            refresh_decision(&mut book, "state", "no_visible_change");
            return;
        };
        if pending.focus.is_some()
            && identities.as_ref().is_none_or(|identity| {
                self.inner
                    .canonical_room_key(&identity.scene.place_id)
                    .is_none()
            })
        {
            refresh_decision(
                &mut book,
                "narrative_focus",
                "awaiting_canonical_room_reference",
            );
            return;
        }
        let interval = if pending.story.is_some() && pending.focus.is_none() {
            self.inner.timing.interval.max(BAKE_INTERVAL)
        } else {
            BAKE_INTERVAL
        };
        if now.saturating_sub(pending.changed_at) < SETTLE
            || book
                .state_images
                .last_started
                .is_some_and(|last| now.saturating_sub(last) < interval)
            || book
                .face_refresh
                .last_started
                .is_some_and(|last| now.saturating_sub(last) < interval)
        {
            refresh_decision(&mut book, "state", "settling_or_rate_limited");
            return;
        }
        // Keep the settled change pending until the original plate is ready to edit.
        // Re-requesting a fresh plate here used to restage the room and lose the pending edit.
        let Some(base) = book.shown().map(|shown| shown.0) else {
            refresh_decision(&mut book, "state", "awaiting_base_plate");
            return;
        };
        let Some(pending) = book.state_images.pending.take() else {
            return;
        };
        let mut pending = pending;
        // Coalesced, failed and unsent replies are not prior shots. Compare with the last
        // successfully published public story edit, at the point this job is dispatched.
        if let Some(story) = pending.story.as_mut() {
            story.dialogue = match book.state_images.last_story_actor.as_deref() {
                Some(actor) if actor == story.actor_key() => {
                    crate::gameplay::shot::PublicDialogue::SameSpeakerFollowup
                }
                Some(_) => crate::gameplay::shot::PublicDialogue::SpeakerChange,
                None => crate::gameplay::shot::PublicDialogue::FirstReply,
            };
        }
        pending.graph = locked(&self.inner.state)
            .scene_graph
            .clone()
            .filter(|graph| graph.revision == pending.revision);
        let request_line = format!(
            "IMAGERY request kind=scene_plate_edit reason={} state={}",
            pending.reason, pending.revision,
        );
        let focus_state = pending.state.clone();
        let focus_revision = pending.revision;
        let job = super::Job::State(Job::Bake {
            scene,
            pending,
            base,
            identities,
        });
        match self.jobs.try_send(job) {
            Ok(()) => {
                crate::gameplay::actor::record_audit_line("gameplay.imagery.request", request_line);
                book.state_images.flying = true;
                book.state_images.last_started = Some(now);
                focus_progress(
                    &mut book.state_images,
                    &focus_state,
                    focus_revision,
                    Stage::Queued,
                    None,
                    now,
                    None,
                );
                refresh_decision(&mut book, "state", "queued");
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(super::Job::State(Job::Bake {
                pending,
                ..
            }))) => {
                book.state_images.pending = Some(pending);
                refresh_decision(&mut book, "state", "queue_full");
            }
            Err(_) => {
                focus_progress(
                    &mut book.state_images,
                    &focus_state,
                    focus_revision,
                    Stage::Failed,
                    None,
                    now,
                    Some("worker_closed"),
                );
                record("gameplay.living_scene.worker_closed", &[]);
            }
        }
    }

    pub(super) async fn state_image_request(
        &self,
        pending: &Pending,
        base: ReferenceImage,
        identities: Option<&IdentitySnapshot>,
    ) -> Result<ImageRequest, ImageFailure> {
        let prompt = if let Some(focus) = &pending.focus {
            crate::gameplay::imagery::narrative_focus::reveal(
                self.inner.rating,
                &pending
                    .snapshot
                    .as_ref()
                    .ok_or(ImageFailure::InvalidRequest)?
                    .model,
                focus,
            )?
        } else if let Some(story) = &pending.story {
            let identity = identities.ok_or(ImageFailure::InvalidRequest)?;
            let camera = story_camera(pending, identity).ok_or(ImageFailure::InvalidRequest)?;
            crate::gameplay::imagery::story_beat_edit(
                self.inner.rating,
                &story.actor_id,
                &story.speaker,
                &story.text,
                camera,
            )?
        } else {
            match &pending.graph {
                Some(graph) => {
                    crate::gameplay::imagery::graph_edit(self.inner.rating, graph, &pending.reason)?
                }
                None => state_edit(
                    self.inner.rating,
                    &pending
                        .snapshot
                        .as_ref()
                        .ok_or(ImageFailure::InvalidRequest)?
                        .model,
                    &pending.reason,
                )?,
            }
        };
        let mut references = vec![base];
        if pending.focus.is_some() {
            let identity = identities.ok_or(ImageFailure::InvalidRequest)?;
            references.push(
                self.inner
                    .canonical_room_reference(&identity.scene.place_id)
                    .await
                    .ok_or(ImageFailure::InvalidRequest)?,
            );
        }
        let prompt = if self.inner.references.photographic_only() || pending.focus.is_some() {
            let identity = identities.ok_or(ImageFailure::InvalidRequest)?;
            let (sheet, legend) = self
                .identity_sheet(&identity.scene, &identity.heroes)
                .await?;
            let sheet_number = references.len() + 1;
            references.push(sheet);
            let actor = pending
                .focus
                .as_ref()
                .map(|focus| focus.speaker_actor_id.as_str())
                .or_else(|| pending.story.as_ref().map(|story| story.actor_id.as_str()));
            if let Some(speaker) = identity.scene.identity_cast.iter().find(|member| {
                if actor.is_none_or(str::is_empty) {
                    pending
                        .story
                        .as_ref()
                        .is_some_and(|story| member.name == story.speaker)
                } else {
                    member.id.strip_prefix("npc:").unwrap_or(&member.id)
                        == actor
                            .unwrap_or_default()
                            .strip_prefix("npc:")
                            .unwrap_or(actor.unwrap_or_default())
                }
            }) {
                let (figure, face) = self.speaker_art(&speaker.sprite).await;
                references.extend(face.or(figure));
            }
            prompt.with_cast_sheet(sheet_number, &legend)
        } else {
            prompt
        };
        Ok(ImageRequest {
            kind: ImageKind::ScenePlateEdit,
            references,
            prompt,
            output: OutputSpec::PLATE,
        })
    }

    pub(super) async fn render_state_image(&self, job: Job) -> Done {
        let started = std::time::Instant::now();
        match job {
            Job::Bake {
                scene,
                pending,
                base,
                identities,
            } => {
                let result = async {
                    let Lookup::Image { bytes, format } =
                        self.inner.imagery.image(base.as_str()).await
                    else {
                        return Err(ImageFailure::InvalidRequest);
                    };
                    let request = self
                        .state_image_request(
                            &pending,
                            ReferenceImage::new(format, bytes),
                            identities.as_deref(),
                        )
                        .await?;
                    if let Some(focus) = pending.focus.as_ref() {
                        let key = self.inner.imagery.key_for(&request);
                        focus_progress(&mut locked(&self.book).state_images, &pending.state,
                            pending.revision, Stage::Requested, Some(&key), self.inner.now(), None);
                        crate::gameplay::actor::record_audit_diagnostic(
                            "gameplay.narrative.visual_focus_requested",
                            format!("NARRATIVE visual_focus_requested sequence={} kind={} id={} revision={} image_key={}",
                                focus.dialogue_sequence, focus.subject.kind(), focus.subject.id(), pending.revision, key.as_str()),
                            serde_json::Map::from_iter([
                                ("resource_type".into(), serde_json::json!("plate")),
                                ("stage".into(), serde_json::json!("requested")),
                                ("dialogue_sequence".into(), serde_json::json!(focus.dialogue_sequence)),
                                ("focus_kind".into(), serde_json::json!(focus.subject.kind())),
                                ("focus_id".into(), serde_json::json!(focus.subject.id())),
                                ("request_revision".into(), serde_json::json!(pending.revision)),
                                ("image_key".into(), serde_json::json!(key.as_str())),
                                ("reference_sha256".into(), serde_json::json!(request.references.iter().map(|reference| &reference.sha256).collect::<Vec<_>>())),
                            ]),
                            &[("image.key", key.as_str().to_owned())],
                        );
                    }
                    let key = self.inner.imagery.render(request).await?;
                    if pending.focus.is_none()
                        && let Some(identity) = identities.as_deref()
                        && let Some(story) = pending.story.as_deref()
                        && let Some(camera) = story_camera(&pending, identity)
                    {
                        crate::gameplay::actor::record_audit_diagnostic(
                            "gameplay.living_scene.plate_story_edited",
                            format!(
                                "PLATE story_edit sequence={} shot_kind={} angle={} focus_reason={} focus_actor={} image_key={}",
                                story.sequence,
                                camera.kind(),
                                camera.angle(),
                                camera.focus_reason(),
                                story.actor_id,
                                key.as_str(),
                            ),
                            serde_json::Map::from_iter([
                                ("story_sequence".to_owned(), serde_json::json!(story.sequence)),
                                ("shot_kind".to_owned(), serde_json::json!(camera.kind())),
                                ("angle".to_owned(), serde_json::json!(camera.angle())),
                                ("focus_reason".to_owned(), serde_json::json!(camera.focus_reason())),
                                ("focus_actor".to_owned(), serde_json::json!(story.actor_id)),
                                ("image_key".to_owned(), serde_json::json!(key.as_str())),
                            ]),
                            &[("image.key", key.as_str().to_owned())],
                        );
                    }
                    if let Some(identity) = identities.as_deref() {
                        self.measure_scene_plate(&key, &identity.scene, &identity.heroes)
                            .await;
                    }
                    Ok(key)
                }
                .await;
                crate::gameplay::actor::record_audit_line(
                    "gameplay.imagery_complete",
                    crate::gameplay::living_scenes::image_completion_line(
                        "scene_plate_edit",
                        started.elapsed(),
                        result.as_ref(),
                    ),
                );
                Done::Bake {
                    scene,
                    state: pending.state,
                    revision: pending.revision,
                    story_actor: pending.story.as_deref().map(StoryBeat::actor_key),
                    result,
                }
            }
            Job::Item {
                id,
                thing,
                detail,
                base,
            } => {
                let result = async {
                    let mut references = Vec::new();
                    if let Some(base) = base
                        && let Lookup::Image { bytes, format } =
                            self.inner.imagery.image(base.as_str()).await
                    {
                        references.push(ReferenceImage::new(format, bytes));
                    }
                    let prompt = item_picture(self.inner.rating, &thing, detail.as_deref())?;
                    self.inner
                        .imagery
                        .render(ImageRequest {
                            kind: ImageKind::ArtifactCloseUp,
                            references,
                            prompt,
                            output: OutputSpec::CLOSE_UP,
                        })
                        .await
                }
                .await;
                crate::gameplay::actor::record_audit_line(
                    "gameplay.imagery_complete",
                    crate::gameplay::living_scenes::image_completion_line(
                        "item",
                        started.elapsed(),
                        result.as_ref(),
                    ),
                );
                Done::Item { id, result }
            }
        }
    }

    pub(super) fn finish_state_image(&self, done: Done) {
        self.finish_state_image_at(done, self.inner.now());
    }

    pub(super) fn finish_state_image_at(&self, done: Done, now: Duration) {
        let mut book = locked(&self.book);
        match done {
            Done::Bake {
                scene,
                state,
                revision,
                story_actor,
                result,
            } => {
                // A late bake is never published over a more recent composition.
                if book
                    .current
                    .as_ref()
                    .is_some_and(|current| current.scene_key == scene)
                {
                    book.state_images.flying = false;
                    if book.state_images.observed == state
                        && book.state_images.observed_graph == revision
                    {
                        match result {
                            Ok(key) => {
                                if book
                                    .current
                                    .as_ref()
                                    .is_some_and(|current| current.in_combat || current.in_roll)
                                    || book.state_images.last_shown.is_some_and(|last| {
                                        now.saturating_sub(last) < BAKE_INTERVAL
                                    })
                                {
                                    book.state_images.ready = Some((key, revision, story_actor));
                                } else {
                                    let layout = book.image_layouts.get(key.as_str()).cloned();
                                    focus_progress(
                                        &mut book.state_images,
                                        &state,
                                        revision,
                                        Stage::Published,
                                        Some(&key),
                                        now,
                                        None,
                                    );
                                    book.edit = Edit::Idle;
                                    book.state_images.shown = Some((key, layout));
                                    book.state_images.baked_revision = Some(revision);
                                    book.state_images.last_shown = Some(now);
                                    if let Some(actor) = story_actor {
                                        book.state_images.last_story_actor = Some(actor);
                                    }
                                }
                            }
                            Err(failure) => {
                                focus_progress(
                                    &mut book.state_images,
                                    &state,
                                    revision,
                                    Stage::Failed,
                                    None,
                                    now,
                                    Some(failure.label()),
                                );
                                record(
                                    "gameplay.living_scene.state_edit_failed",
                                    &[("reason", failure.label().into())],
                                );
                            }
                        }
                    } else {
                        refresh_decision(&mut book, "story", "superseded_by_newer_beat");
                    }
                }
            }
            Done::Item { id, result } => {
                if let Some(item) = book.state_images.items.get_mut(&id) {
                    match result {
                        Ok(key) => {
                            item.key = Some(key);
                            if item.shared {
                                item.requested_at = self.inner.now();
                            }
                        }
                        Err(failure) => {
                            item.failed = true;
                            record(
                                "gameplay.living_scene.item_failed",
                                &[("reason", failure.label().into())],
                            );
                        }
                    }
                }
            }
        }
        drop(book);
        self.wake();
    }
}

fn item_revision(thing: &SceneThing, detail: Option<&str>) -> u64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            thing.id,
            thing.picture_description,
            thing.detail,
            thing.open,
            thing.lit,
            thing.broken,
            detail.unwrap_or_default()
        )
        .as_bytes(),
    );
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes)
}

impl Plates {
    /// Called only for a committed examination/take/share. The detail must already be listener-safe.
    pub(in crate::gameplay) fn request_item(
        &self,
        hero: &str,
        thing_id: &str,
        detail: Option<String>,
        shared: bool,
    ) -> Result<(), &'static str> {
        let mut book = locked(&self.shared.book);
        let thing = book
            .current
            .as_ref()
            .and_then(|current| current.scene.as_ref())
            .and_then(|scene| scene.snapshot.as_ref())
            .and_then(|snapshot| {
                snapshot
                    .model
                    .things
                    .iter()
                    .find(|thing| thing.id == thing_id)
            })
            .cloned()
            .ok_or("object is not in the current scene")?;
        let state = item_revision(&thing, detail.as_deref());
        let id = format!("{}:{state}", thing.id);
        let now = self.shared.inner.now();
        if let Some(item) = book.state_images.items.get_mut(&id) {
            item.shared |= shared;
            if shared {
                item.requested_at = now;
            }
            book.state_images.latest_items.insert(hero.to_owned(), id);
            drop(book);
            self.shared.wake();
            return Ok(());
        }
        if book.state_images.items.len() >= 128 {
            return Err("object picture cache is full");
        }
        let base = book.shown().map(|shown| shown.0);
        let admission = self.shared.jobs.try_send(super::Job::State(Job::Item {
            id: id.clone(),
            thing: thing.clone(),
            detail: detail.clone(),
            base,
        }));
        book.state_images.items.insert(
            id.clone(),
            Item {
                thing,
                detail,
                key: None,
                failed: admission.is_err(),
                shared,
                requested_at: now,
            },
        );
        book.state_images.latest_items.insert(hero.to_owned(), id);
        drop(book);
        self.shared.wake();
        match admission {
            Ok(()) => {
                crate::gameplay::actor::record_audit_line(
                    "gameplay.imagery.request",
                    format!("IMAGERY request kind=item reason={thing_id} state={state}"),
                );
                Ok(())
            }
            Err(error) => {
                record(
                    "gameplay.living_scene.item_admission_failed",
                    &[("reason", error.to_string())],
                );
                Err("object picture queue is unavailable")
            }
        }
    }
}
