//! Settled visible changes share one edit; object cards share one request per object state.

use super::*;
use crate::gameplay::{
    adventure::scene_model::{SceneItemProvenance, SceneNow, SceneThing},
    imagery::{item_picture, state_edit},
    narrative_focus::{self, Dialogue, NarrativeFocus, Presentation, Stage},
};
use serde_json::Value;

const SETTLE: Duration = Duration::from_secs(3);
const BAKE_INTERVAL: Duration = Duration::from_secs(10);
/// After a failed fresh render for a capped edit chain, no new one is asked for this long.
const CHAIN_REFRESH_COOLDOWN: Duration = Duration::from_secs(60);

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
    /// The finished image, its revision, its story actor and whether it was composed afresh.
    ready: Option<(ImageKey, u64, Option<String>, bool)>,
    pub(super) items: BTreeMap<String, Item>,
    pub(super) latest_items: BTreeMap<String, String>,
    last_story_actor: Option<String>,
    focus: Option<Presentation>,
    /// The still on screen is an edit, so it holds content the fresh base does not have.
    pub(super) carried_edit: bool,
    /// The plate id of a fresh render asked for to replace a capped edit chain's base.
    pub(super) refresh: Option<String>,
    refresh_failed_at: Option<Duration>,
}

impl Book {
    /// The first virtual time at which the current pending edit could pass its production
    /// settle and pacing gates. Tests use this instead of assuming that several expensive fake
    /// image operations fit inside a hard-coded wall-clock interval.
    pub(super) fn next_eligible_at(
        &self,
        story_interval: Duration,
        face_last_started: Option<Duration>,
    ) -> Option<Duration> {
        let pending = self.pending.as_ref()?;
        let interval = if pending.story.is_some() && pending.focus.is_none() {
            story_interval.max(BAKE_INTERVAL)
        } else {
            BAKE_INTERVAL
        };
        let mut eligible = pending.changed_at.saturating_add(SETTLE);
        for last_started in [self.last_started, face_last_started].into_iter().flatten() {
            eligible = eligible.max(last_started.saturating_add(interval));
        }
        Some(eligible)
    }
}

#[cfg_attr(test, derive(Clone))]
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

fn cast_identity<'a>(
    identity: &'a IdentitySnapshot,
    actor_id: &str,
) -> Option<&'a IdentityReference> {
    let actor_id = actor_id.strip_prefix("npc:").unwrap_or(actor_id);
    identity
        .scene
        .identity_cast
        .iter()
        .find(|member| member.id.strip_prefix("npc:").unwrap_or(&member.id) == actor_id)
}

fn state_speaker<'a>(
    pending: &Pending,
    identity: &'a IdentitySnapshot,
) -> Result<Option<&'a IdentityReference>, ImageFailure> {
    if let Some(focus) = &pending.focus {
        return cast_identity(identity, &focus.speaker_actor_id)
            .map(Some)
            .ok_or(ImageFailure::InvalidRequest);
    }
    if let Some(story) = &pending.story {
        let speaker =
            if story.actor_id.is_empty() {
                identity.scene.identity_cast.iter().find(|member| {
                    crate::voice_line::speaker_names_match(&member.name, &story.speaker)
                })
            } else {
                cast_identity(identity, &story.actor_id)
            };
        return speaker.map(Some).ok_or(ImageFailure::InvalidRequest);
    }
    match &identity.scene.speaker {
        Some(speaker) => cast_identity(identity, &speaker.id)
            .map(Some)
            .ok_or(ImageFailure::InvalidRequest),
        None => Ok(None),
    }
}

#[derive(Clone)]
pub(super) struct Item {
    pub(super) thing: SceneThing,
    pub(super) detail: Option<String>,
    pub(super) key: Option<ImageKey>,
    pub(super) failed: bool,
    pub(super) shared: bool,
    pub(super) explicitly_shared: bool,
    pub(super) requested_at: Duration,
}

/// Media decorates the server's latest durable item selection; stale requests cannot replace
/// a newer purchase/examination or fabricate ownership on reconnect.
fn enrich_item_card(card: &mut rpc::SceneItemCard, item: &Item) {
    if !item_card_matches(card, item) {
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

fn item_card_matches(card: &rpc::SceneItemCard, item: &Item) -> bool {
    card.thing_id == item.thing.id
        && card.name == item.thing.name
        && card.detail == item.thing.detail
        && (card.picture_description.is_empty()
            || card.picture_description == item.thing.picture_description)
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
        /// Composed afresh rather than edited from the plate on screen (FLUX story beats).
        fresh: bool,
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

/// Stores the fresh base that a capped edit chain asked for. Only the base of the next edit
/// changes: the still on screen stays up until that edit is ready.
pub(super) fn finish_chain_refresh(
    book: &mut super::Book,
    id: &str,
    result: Result<(ImageKey, Option<PlatePlacement>), ImageFailure>,
    now: Duration,
) {
    book.state_images.refresh = None;
    book.state_images.flying = false;
    let current = book
        .current
        .as_ref()
        .is_some_and(|current| current.id.as_deref() == Some(id));
    match result {
        Ok((key, placement)) if current => {
            book.insert(id.to_owned(), super::Entry::Ready { key, placement });
            book.state_images.carried_edit = false;
        }
        Ok(_) => {}
        Err(failure) if current => {
            book.state_images.refresh_failed_at = Some(now);
            record(
                "gameplay.living_scene.plate_refresh_failed",
                &[("reason", failure.label().into())],
            );
            refresh_decision(book, "chain", "edit_chain_refresh_failed");
        }
        Err(_) => {}
    }
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
    fn state_reference_status(
        &self,
        pending: &Pending,
        identity: &IdentitySnapshot,
    ) -> super::super::canonical::ReferenceStatus {
        use super::super::canonical::ReferenceStatus;
        let mut waiting = false;
        let mut statuses = Vec::new();
        if pending.focus.is_some() || identity.scene.room_revision.is_some() {
            statuses.push(identity.scene.room_revision.as_deref().map_or_else(
                || {
                    if pending.focus.is_some() {
                        ReferenceStatus::Failed(ImageFailure::InvalidRequest)
                    } else {
                        ReferenceStatus::Ready
                    }
                },
                |revision| {
                    self.inner
                        .canonical_room_status_at(&identity.scene.place_id, revision)
                },
            ));
        }
        if pending.focus.is_some() {
            statuses.extend(identity.scene.identity_cast.iter().map(|actor| {
                self.inner
                    .canonical_npc_status(actor.id.strip_prefix("npc:").unwrap_or(&actor.id))
            }));
            statuses.extend(
                identity
                    .heroes
                    .iter()
                    .map(|seat| self.inner.canonical_hero_status(*seat)),
            );
        }
        match state_speaker(pending, identity) {
            Ok(Some(speaker)) => {
                let id = speaker.id.strip_prefix("npc:").unwrap_or(&speaker.id);
                let mut status = self.inner.canonical_npc_status(id);
                // A speaker nobody has made a photograph of yet is asked for now, once (the art
                // contract's on-demand rule), and the edit waits for it.
                if status == ReferenceStatus::Missing && self.inner.request_cast_photograph(id) {
                    status = ReferenceStatus::Pending;
                }
                statuses.push(status);
            }
            Ok(None) => {}
            Err(failure) => return ReferenceStatus::Failed(failure),
        }
        for status in statuses {
            match status {
                ReferenceStatus::Failed(failure) => return ReferenceStatus::Failed(failure),
                ReferenceStatus::Missing | ReferenceStatus::Pending => waiting = true,
                ReferenceStatus::Ready => {}
            }
        }
        if waiting {
            ReferenceStatus::Pending
        } else {
            ReferenceStatus::Ready
        }
    }

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
                        && (item.explicitly_shared
                            || scene
                                .hand_items
                                .iter()
                                .any(|card| item_card_matches(card, item)))
                })
                .max_by_key(|item| item.requested_at),
        };
        if hero.is_some() {
            if let Some(card) = scene.item.as_mut()
                && let Some(item) = book
                    .state_images
                    .items
                    .values()
                    .filter(|item| item_card_matches(card, item))
                    .max_by_key(|item| item.requested_at)
            {
                enrich_item_card(card, item);
            }
            for card in &mut scene.hand_items {
                if let Some(item) = book
                    .state_images
                    .items
                    .values()
                    .filter(|item| item_card_matches(card, item))
                    .max_by_key(|item| item.requested_at)
                {
                    enrich_item_card(card, item);
                }
            }
            return;
        }
        for card in &mut scene.hand_items {
            if let Some(item) = book
                .state_images
                .items
                .values()
                .filter(|item| item.shared && item_card_matches(card, item))
                .max_by_key(|item| item.requested_at)
            {
                enrich_item_card(card, item);
            }
        }
        scene.item = item.map(|item| {
            let mut card = scene
                .hand_items
                .iter()
                .find(|card| card.thing_id == item.thing.id)
                .cloned()
                .or_else(|| {
                    scene
                        .item
                        .as_ref()
                        .filter(|card| card.thing_id == item.thing.id)
                        .cloned()
                })
                .unwrap_or_else(|| {
                    let (owner_id, holder_id) =
                        inventory_custody(&scene.scene_contents_json, &item.thing.id);
                    rpc::SceneItemCard {
                        thing_id: item.thing.id.clone(),
                        name: item.thing.name.clone(),
                        detail: item.detail.as_ref().map_or_else(
                            || item.thing.detail.clone(),
                            |trace| format!("{}\n{trace}", item.thing.detail),
                        ),
                        image_path: String::new(),
                        image_state: rpc::PlateState::Pending as i32,
                        state_revision: item_revision(&item.thing, item.detail.as_deref()),
                        observation_only: if owner_id.is_empty() || holder_id.is_empty() {
                            item.thing.holder_id.is_some()
                                && item.thing.owner_id.as_deref() != item.thing.holder_id.as_deref()
                        } else {
                            owner_id != holder_id
                        },
                        holder: item.thing.holder_id.clone().unwrap_or_default(),
                        owner_id,
                        holder_id,
                        picture_description: item.thing.picture_description.clone(),
                    }
                });
            enrich_item_card(&mut card, item);
            card.state_revision = item_revision(&item.thing, item.detail.as_deref());
            card
        });
    }

    pub(super) fn project_item_cards(&self, cards: &mut [rpc::SceneItemCard]) {
        let book = locked(&self.book);
        for card in cards {
            if let Some(item) = book
                .state_images
                .items
                .values()
                .filter(|item| item_card_matches(card, item))
                .max_by_key(|item| item.requested_at)
            {
                enrich_item_card(card, item);
            }
        }
    }

    /// Asks for a fresh render of the current scene to be the base of the next edit, instead of
    /// stacking the edit on the still on screen. The still stays up until that edit is ready.
    /// Returns whether the render was asked for.
    pub(super) fn request_chain_refresh(
        &self,
        book: &mut super::Book,
        now: Duration,
        pipeline: &'static str,
    ) -> bool {
        if book
            .state_images
            .refresh_failed_at
            .is_some_and(|failed| now.saturating_sub(failed) < CHAIN_REFRESH_COOLDOWN)
        {
            refresh_decision(book, pipeline, "edit_chain_refresh_cooling");
            return false;
        }
        let Some(current) = book.current.as_ref() else {
            return false;
        };
        let (Some(id), Some(scene)) = (current.id.clone(), current.scene.clone()) else {
            return false;
        };
        let Ok(spec) = scene.stage(&current.heroes) else {
            return false;
        };
        let job = super::Job::Fresh {
            id: id.clone(),
            graph_revision: current.graph_revision,
            scene,
            spec: Box::new(spec),
            heroes: current.heroes.clone(),
            faces: current.faces.clone(),
            ahead: false,
        };
        if self.jobs.try_send(job).is_err() {
            refresh_decision(book, pipeline, "queue_full");
            return false;
        }
        book.state_images.refresh = Some(id);
        book.state_images.flying = true;
        book.state_images.last_started = Some(now);
        refresh_decision(book, pipeline, "edit_chain_capped");
        true
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
        if can_show
            && let Some((key, revision, story_actor, fresh)) = book.state_images.ready.take()
        {
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
            book.state_images.carried_edit = !fresh;
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
        let is_focus = pending.focus.is_some();
        // Only a story beat's edit leaves the scene's state out of its prompt; every other edit
        // carries the canonical current scene.
        let lacks_scene_state = !is_focus && pending.story.is_some();
        use super::super::canonical::ReferenceStatus;
        let status = identities
            .as_ref()
            .map_or(ReferenceStatus::Missing, |identity| {
                self.state_reference_status(pending, identity)
            });
        match status {
            ReferenceStatus::Missing | ReferenceStatus::Pending => {
                refresh_decision(
                    &mut book,
                    if is_focus { "narrative_focus" } else { "state" },
                    "awaiting_canonical_references",
                );
                return;
            }
            ReferenceStatus::Failed(failure) => {
                if let Some(pending) = book.state_images.pending.take() {
                    if is_focus {
                        focus_progress(
                            &mut book.state_images,
                            &pending.state,
                            pending.revision,
                            Stage::Failed,
                            None,
                            now,
                            Some(failure.label()),
                        );
                    }
                }
                refresh_decision(
                    &mut book,
                    if is_focus { "narrative_focus" } else { "state" },
                    "canonical_reference_failed",
                );
                self.wake();
                return;
            }
            ReferenceStatus::Ready => {}
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
        // Every edit starts from the fresh plate, never from the still on screen: an edit of an
        // edit compounds the model's rendering loss. A story beat's edit has no scene state to
        // carry, so while the still on screen is an edit it waits for a fresh render instead.
        if lacks_scene_state && book.state_images.carried_edit {
            self.request_chain_refresh(&mut book, now, "state");
            return;
        }
        // Keep the settled change pending until the original plate is ready to edit.
        // Re-requesting a fresh plate here used to restage the room and lose the pending edit.
        let Some((base, _)) = book.fresh_base(true) else {
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
            "IMAGERY request kind=scene_plate_edit reason={} state={} base=fresh base_key={} carried_edit={}",
            pending.reason,
            pending.revision,
            base.as_str(),
            book.state_images.carried_edit,
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

    /// A story beat's camera move on the FLUX route. An edit of the plate on screen keeps its
    /// framing and poses, so the beat's plate is composed afresh with the beat's camera.
    async fn story_fresh_request(
        &self,
        pending: &Pending,
        identities: Option<&IdentitySnapshot>,
    ) -> Result<ImageRequest, ImageFailure> {
        let identity = identities.ok_or(ImageFailure::InvalidRequest)?;
        let camera = story_camera(pending, identity).ok_or(ImageFailure::InvalidRequest)?;
        let mut spec = identity
            .scene
            .stage(&identity.heroes)
            .map_err(|_| ImageFailure::InvalidRequest)?;
        spec.camera = camera;
        let (request, _) = self
            .photographic_request(&identity.scene, &spec, &identity.heroes, None, None)
            .await?;
        Ok(request)
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
        let identity = identities.ok_or(ImageFailure::InvalidRequest)?;
        match identity.scene.room_revision.as_deref() {
            Some(revision) => references.push(
                self.inner
                    .canonical_room_reference_at(&identity.scene.place_id, revision)
                    .await
                    .ok_or(ImageFailure::Interrupted)?,
            ),
            None if pending.focus.is_some() => return Err(ImageFailure::InvalidRequest),
            None => {}
        }
        let prompt = {
            let (sheet, legend, _) = self
                .identity_sheet(&identity.scene, &identity.heroes)
                .await?;
            let sheet_number = references.len() + 1;
            references.push(sheet);
            let active_speaker = state_speaker(pending, identity)?;
            let speaker_number = active_speaker.map(|_| references.len() + 1);
            if let Some(speaker) = active_speaker {
                let actor = speaker.id.strip_prefix("npc:").unwrap_or(&speaker.id);
                references.push(
                    self.inner
                        .canonical_npc_reference(actor)
                        .await
                        .ok_or(ImageFailure::Interrupted)?,
                );
            }
            let mut prompt = prompt;
            if pending.focus.is_none() && identity.scene.room_revision.is_some() {
                prompt = prompt.with_canonical_room_reference(2);
            }
            prompt = prompt.with_cast_sheet(sheet_number, &legend);
            if pending.focus.is_none()
                && let (Some(number), Some(speaker)) = (speaker_number, active_speaker)
            {
                prompt =
                    prompt.with_canonical_speaker_reference(number, &speaker.id, &speaker.name);
            }
            prompt
        };
        if prompt.character_count() > df_providers::IMAGE_EDIT_MAXIMUM_PROMPT_CHARACTERS {
            return Err(ImageFailure::InvalidRequest);
        }
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
                // A FLUX story beat is composed afresh (see `story_fresh_request`), not edited.
                let fresh = self.inner.imagery.flux_prompts()
                    && pending.focus.is_none()
                    && pending.story.is_some();
                let result = async {
                    let request = if fresh {
                        self.story_fresh_request(&pending, identities.as_deref())
                            .await?
                    } else {
                        let Lookup::Image { bytes, format } =
                            self.inner.imagery.image(base.as_str()).await
                        else {
                            return Err(ImageFailure::InvalidRequest);
                        };
                        self.state_image_request(
                            &pending,
                            ReferenceImage::new(format, bytes),
                            identities.as_deref(),
                        )
                        .await?
                    };
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
                    let subject = format!("plate:{}", identities.as_deref().map_or("", |identity| identity.scene.place_id.as_str()));
                    let key = self.inner.imagery.generate(&subject, request).await?;
                    if pending.focus.is_none()
                        && let Some(identity) = identities.as_deref()
                        && let Some(story) = pending.story.as_deref()
                        && let Some(camera) = story_camera(&pending, identity)
                    {
                        crate::gameplay::actor::record_audit_diagnostic(
                            "gameplay.living_scene.plate_story_edited",
                            format!(
                                "PLATE story_edit sequence={} shot_kind={} angle={} focus_reason={} focus_actor={} base=fresh base_key={} image_key={}",
                                story.sequence,
                                camera.kind(),
                                camera.angle(),
                                camera.focus_reason(),
                                story.actor_id,
                                base.as_str(),
                                key.as_str(),
                            ),
                            serde_json::Map::from_iter([
                                ("story_sequence".to_owned(), serde_json::json!(story.sequence)),
                                ("shot_kind".to_owned(), serde_json::json!(camera.kind())),
                                ("angle".to_owned(), serde_json::json!(camera.angle())),
                                ("focus_reason".to_owned(), serde_json::json!(camera.focus_reason())),
                                ("focus_actor".to_owned(), serde_json::json!(story.actor_id)),
                                ("base".to_owned(), serde_json::json!("fresh")),
                                ("base_key".to_owned(), serde_json::json!(base.as_str())),
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
                    fresh,
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
                        .generate(
                            &format!("item:{id}"),
                            ImageRequest {
                                kind: ImageKind::ArtifactCloseUp,
                                references,
                                prompt,
                                output: OutputSpec::CLOSE_UP,
                            },
                        )
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
                fresh,
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
                                    book.state_images.ready =
                                        Some((key, revision, story_actor, fresh));
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
                                    book.state_images.carried_edit = !fresh;
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
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            thing.id,
            thing.name,
            thing.picture_description,
            thing.detail,
            thing.closer_look,
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

fn inventory_custody(scene_contents_json: &str, item_id: &str) -> (String, String) {
    let Some(row) = serde_json::from_str::<Value>(scene_contents_json)
        .ok()
        .and_then(|contents| contents.get("inventory").and_then(Value::as_array).cloned())
        .and_then(|entries| {
            entries
                .into_iter()
                .find(|entry| entry.get("item_id").and_then(Value::as_str) == Some(item_id))
        })
    else {
        return (String::new(), String::new());
    };
    let id = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    (id("owner_id"), id("holder_id"))
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
        self.request_item_with_visibility(hero, thing_id, None, detail, shared, shared)
    }

    /// Uses the authoritative post-commit Thing. Generated stock becomes a canonical
    /// SceneThing only after a purchase, so resolving it through the authored plate snapshot
    /// would incorrectly report that it is absent from the scene.
    pub(in crate::gameplay) fn request_committed_item(
        &self,
        hero: &str,
        thing: &SceneThing,
        detail: Option<String>,
        shared: bool,
    ) -> Result<(), &'static str> {
        self.request_item_with_visibility(
            hero,
            &thing.id,
            Some(thing.clone()),
            detail,
            shared,
            shared,
        )
    }

    /// Makes an image available for a current hand projection without treating the projection
    /// itself as an explicit share action.
    pub(in crate::gameplay) fn request_hand_item(
        &self,
        card: &rpc::SceneItemCard,
        shared: bool,
        scene_contents_json: &str,
    ) -> Result<(), &'static str> {
        let mut book = locked(&self.shared.book);
        let now = self.shared.inner.now();
        let cached = book
            .state_images
            .items
            .iter()
            .filter(|(_, item)| {
                item.thing.id == card.thing_id
                    && item.thing.name == card.name
                    && item.thing.detail == card.detail
                    && (card.picture_description.is_empty()
                        || item.thing.picture_description == card.picture_description)
            })
            .max_by_key(|(_, item)| item.requested_at)
            .map(|(id, _)| id.clone());
        if let Some(id) = cached {
            if let Some(item) = book.state_images.items.get_mut(&id) {
                item.shared |= shared;
                if shared {
                    item.requested_at = now;
                }
            }
            book.state_images
                .latest_items
                .insert(card.holder.clone(), id);
            drop(book);
            self.shared.wake();
            return Ok(());
        }
        drop(book);
        let thing = self
            .authored_or_projected_item(card, scene_contents_json)
            .ok_or("object image details are unavailable in the current scene projection")?;
        self.request_item_with_visibility(
            &card.holder,
            &card.thing_id,
            Some(thing),
            None,
            shared,
            false,
        )
    }

    fn request_item_with_visibility(
        &self,
        hero: &str,
        thing_id: &str,
        committed_thing: Option<SceneThing>,
        detail: Option<String>,
        shared: bool,
        explicitly_shared: bool,
    ) -> Result<(), &'static str> {
        let mut book = locked(&self.shared.book);
        let thing = committed_thing
            .or_else(|| {
                book.current
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
            })
            .ok_or("object is not in the current scene")?;
        if thing.id != thing_id {
            return Err("committed object identity does not match the request");
        }
        let state = item_revision(&thing, detail.as_deref());
        let id = format!("{}:{state}", thing.id);
        let now = self.shared.inner.now();
        if let Some(item) = book.state_images.items.get_mut(&id) {
            // Holder and owner do not change the close-up image identity. Refresh them from the
            // authoritative snapshot so a reused image follows the current custody state.
            item.thing = thing;
            item.detail = detail;
            item.shared |= shared;
            item.explicitly_shared |= explicitly_shared;
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
                explicitly_shared,
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

    fn authored_or_projected_item(
        &self,
        card: &rpc::SceneItemCard,
        scene_contents_json: &str,
    ) -> Option<SceneThing> {
        let book = locked(&self.shared.book);
        let authored = book
            .current
            .as_ref()
            .and_then(|current| current.scene.as_ref())
            .and_then(|scene| scene.snapshot.as_ref())
            .and_then(|snapshot| {
                snapshot
                    .model
                    .things
                    .iter()
                    .find(|thing| thing.id == card.thing_id)
            })
            .cloned();
        if let Some(thing) = authored {
            return Some(thing);
        }
        let contents: Value = serde_json::from_str(scene_contents_json).ok()?;
        let visible = contents
            .get("things")
            .and_then(Value::as_array)
            .and_then(|things| {
                things
                    .iter()
                    .find(|thing| thing.get("id").and_then(Value::as_str) == Some(&card.thing_id))
            });
        let picture_description = visible
            .and_then(|thing| thing.get("look"))
            .and_then(Value::as_str)
            .filter(|description| !description.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| nonempty(&card.picture_description))?;
        let owner_id = nonempty(&card.owner_id);
        let holder_id = nonempty(&card.holder_id);
        Some(SceneThing {
            id: card.thing_id.clone(),
            name: card.name.clone(),
            short: card.name.clone(),
            detail: card.detail.clone(),
            picture_description,
            closer_look: String::new(),
            skill: crate::gameplay::pack::FindingSkill::Perception,
            dc: 0,
            finding_id: None,
            clue_fact_id: None,
            portable: true,
            value_cp: None,
            use_effect: String::new(),
            position: visible
                .and_then(|thing| thing.get("position"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            owner_id,
            in_use: false,
            in_scene: visible
                .and_then(|thing| thing.get("in_scene"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
            publicly_visible: visible
                .and_then(|thing| thing.get("publicly_visible"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            holder_id,
            open: visible
                .and_then(|thing| thing.get("open"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            lit: visible
                .and_then(|thing| thing.get("lit"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            broken: visible
                .and_then(|thing| thing.get("broken"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            searched_by: Vec::new(),
            aliases: Vec::new(),
            provenance: SceneItemProvenance::default(),
        })
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}
