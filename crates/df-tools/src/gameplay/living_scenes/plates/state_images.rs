//! Settled visible changes share one edit; object cards share one request per object state.

use super::*;
use crate::gameplay::{
    adventure::scene_model::{SceneItemProvenance, SceneNow, SceneThing},
    imagery::{ImagePriority, item_image_request, state_edit},
    narrative_focus::{NarrativeFocus, Presentation, Stage},
};
use serde_json::Value;

const SETTLE: Duration = Duration::from_secs(3);
const BAKE_INTERVAL: Duration = Duration::from_secs(15);
const SCENE_CADENCE: Duration = Duration::from_secs(30);
pub(super) const MINIMUM_PRESENTATION: Duration = Duration::from_secs(25);
const MAXIMUM_PLATE_RENDERS: usize = 3;
/// After a failed fresh render for a capped edit chain, no new one is asked for this long.
const CHAIN_REFRESH_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Book {
    scene: String,
    observed: String,
    observed_graph: u64,
    pub(super) pending: Option<Pending>,
    in_flight: BTreeMap<String, Composition>,
    pub(super) last_started: Option<Duration>,
    last_shown: Option<Duration>,
    pub(super) shown: Option<(ImageKey, Option<PlatePlacement>)>,
    retained_base: Option<(ImageKey, Option<PlatePlacement>)>,
    pub(super) baked_revision: Option<u64>,
    ready: Option<Ready>,
    shown_sequence: u64,
    selected: Option<(ImageKey, Composition)>,
    pub(super) items: BTreeMap<String, Item>,
    pub(super) latest_items: BTreeMap<String, String>,
    item_shot: Option<ItemShot>,
    last_story_actor: Option<String>,
    focus: Option<Presentation>,
    /// The still on screen is an edit, so it holds content the fresh base does not have.
    pub(super) carried_edit: bool,
    /// The plate id of a fresh render asked for to replace a capped edit chain's base.
    pub(super) refresh: Option<String>,
    refresh_failed_at: Option<Duration>,
    cadence_deadline: Option<Duration>,
    cadence_sequence: u64,
    latest_story: Option<StoryBeat>,
    projection_deadline: Option<(ImageKey, Duration)>,
    pub(super) subject_selection: Option<subject::Selection>,
    framed_subject: Option<subject::Frame>,
}

#[derive(Clone)]
struct Composition {
    sequence: u64,
    scene: String,
    retired: bool,
    beat: u64,
    started_at: Duration,
    state: String,
    revision: u64,
    due: Duration,
}

struct Ready {
    key: ImageKey,
    composition: Composition,
    story_actor: Option<String>,
    fresh: bool,
}

struct ItemShot {
    id: String,
    hero: String,
    connected: bool,
    shown_at: Option<Duration>,
}

enum ItemRequest {
    Explicit,
    InspectionHands,
    Hand(ImagePriority),
}

#[derive(Clone, Copy)]
pub(super) enum ItemFocusPose {
    Recorded,
    InspectionHands,
}

impl Book {
    fn scene_in_flight(&self) -> usize {
        self.in_flight
            .values()
            .filter(|composition| composition.scene == self.scene)
            .count()
    }

    pub(super) fn has_in_flight(&self) -> bool {
        self.scene_in_flight() > 0 || self.refresh.is_some()
    }

    pub(super) fn note_effective_projection(&mut self, key: &ImageKey, now: Duration) {
        if self.is_item_shot(key) {
            if let Some(shot) = self.item_shot.as_mut()
                && shot.shown_at.is_none()
                && let Some(item) = self.items.get(&shot.id)
            {
                shot.shown_at = Some(now);
                crate::gameplay::actor::record_audit_diagnostic(
                    "gameplay.living_scene.subject_presented",
                    format!(
                        "PLATE subject kind=item_focus id={} asking_hero={} connected={} image_key={}",
                        item.thing.id,
                        shot.hero,
                        shot.connected,
                        key.as_str()
                    ),
                    serde_json::Map::from_iter([
                        ("subject_kind".into(), serde_json::json!("item_focus")),
                        ("connected".into(), serde_json::json!(shot.connected)),
                        ("subject_id".into(), serde_json::json!(item.thing.id)),
                        ("asking_hero".into(), serde_json::json!(shot.hero)),
                        ("image_key".into(), serde_json::json!(key.as_str())),
                        ("shown_at_ms".into(), serde_json::json!(now.as_millis())),
                        (
                            "minimum_read_ms".into(),
                            serde_json::json!(MINIMUM_PRESENTATION.as_millis()),
                        ),
                    ]),
                    &[],
                );
            }
            return;
        }
        if self
            .item_shot
            .as_ref()
            .is_some_and(|shot| shot.shown_at.is_some())
        {
            self.item_shot = None;
        }
        if self
            .projection_deadline
            .as_ref()
            .is_some_and(|(expected, _)| expected == key)
            && let Some((_, due)) = self.projection_deadline.take()
        {
            record_cadence(
                "projected",
                self.baked_revision.unwrap_or(self.observed_graph),
                due,
                now,
            );
        }
        let next = now.saturating_add(SCENE_CADENCE);
        self.last_shown = Some(now);
        self.cadence_deadline = Some(next);
        if let Some((expected, composition)) = self.selected.take() {
            if expected == *key {
                record_pipeline("presented", &composition, now, self.scene_in_flight());
            } else {
                self.selected = Some((expected, composition));
            }
        }
    }

    fn accepts_state(&self, state: &str) -> bool {
        let state = self
            .in_flight
            .get(state)
            .map_or(state, |composition| composition.state.as_str());
        if state == self.observed {
            return true;
        }
        match state.rsplit_once("|cadence_context=") {
            Some((observed, context)) => {
                observed == self.observed
                    && context
                        == self
                            .latest_story
                            .as_ref()
                            .map_or("none", |story| story.sha256.as_str())
            }
            None => state == self.observed,
        }
    }

    pub(super) fn note_initial_projection(&mut self, now: Duration) {
        if self.last_shown.is_none() {
            self.last_shown = Some(now);
            self.cadence_deadline = Some(now.saturating_add(SCENE_CADENCE));
        }
    }

    pub(super) fn is_item_shot(&self, key: &ImageKey) -> bool {
        self.item_shot
            .as_ref()
            .and_then(|shot| self.items.get(&shot.id))
            .and_then(|item| item.key.as_ref())
            == Some(key)
    }

    pub(super) fn item_projection(&self, now: Duration) -> Option<ImageKey> {
        let shot = self.item_shot.as_ref()?;
        if shot
            .shown_at
            .is_some_and(|shown| now.saturating_sub(shown) >= MINIMUM_PRESENTATION)
        {
            return None;
        }
        self.items.get(&shot.id)?.key.clone()
    }

    /// The first virtual time at which the current pending edit could pass its production
    /// settle and pacing gates. Tests use this instead of assuming that several expensive fake
    /// image operations fit inside a hard-coded wall-clock interval.
    pub(super) fn next_eligible_at(
        &self,
        _story_interval: Duration,
        face_last_started: Option<Duration>,
    ) -> Option<Duration> {
        let pending = self.pending.as_ref()?;
        let interval = BAKE_INTERVAL;
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
    pub(super) cadence_due: Option<Duration>,
    framed_subject: Option<subject::Frame>,
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
    if !book.accepts_state(state) {
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
        priority: ImagePriority,
        focus_hero: Option<String>,
        focus_pose: ItemFocusPose,
    },
}

pub(super) struct IdentitySnapshot {
    pub(super) scene: PlateScene,
    heroes: Vec<u8>,
    face: Option<Face>,
    shot_sequence: u64,
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

pub(super) fn reset_scene(book: &mut super::Book, view: &View, now: Duration) -> bool {
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
    let in_flight = images.scene_in_flight();
    for composition in images
        .in_flight
        .values_mut()
        .filter(|composition| !composition.retired)
    {
        composition.retired = true;
        record_pipeline("retired", composition, now, in_flight);
    }
    if let Some(ready) = images.ready.take() {
        record_pipeline("retired", &ready.composition, now, in_flight);
    }
    *images = Book {
        cadence_sequence: images.cadence_sequence,
        in_flight: std::mem::take(&mut images.in_flight),
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
    // A local spot changes the public scene title/key, but not the canonical place.
    // Admitted pictures still depict this place and remain useful until a newer one wins.
    if same_place(book, scene) && book.state_images.scene != view.scene_key {
        let base = book.fresh_base(true);
        let previous = book.state_images.scene.clone();
        let images = &mut book.state_images;
        if base.is_some() {
            images.retained_base = base;
        }
        images.scene = view.scene_key.clone();
        for composition in images
            .in_flight
            .values_mut()
            .filter(|work| !work.retired && work.scene == previous)
        {
            composition.scene = view.scene_key.clone();
        }
        if let Some(ready) = images.ready.as_mut() {
            ready.composition.scene = view.scene_key.clone();
        }
        if let Some((_, selected)) = images.selected.as_mut() {
            selected.scene = view.scene_key.clone();
        }
        if let Some(shown) = book.presented.as_mut() {
            shown.scene_key = view.scene_key.clone();
        }
    }
    let scene_changed = reset_scene(book, view, now);
    let story_changed = book
        .state_images
        .latest_story
        .as_ref()
        .map(|story| &story.sha256)
        != view.story.as_ref().map(|story| &story.sha256);
    book.state_images.latest_story = view.story.clone();
    if story_changed {
        book.state_images.observed_graph = view.graph_revision;
        // Only work not yet dispatched follows a new beat. Admitted renders retain their
        // composition and may still reach the screen in monotonically increasing order.
        if let Some(pending) = book.state_images.pending.as_mut() {
            pending.story = view.story.clone().map(Box::new);
            pending.revision = view.graph_revision;
        }
    }
    if scene_changed {
        book.state_images.observed_graph = view.graph_revision;
        book.state_images.cadence_deadline = Some(now.saturating_add(SCENE_CADENCE));
    }
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
    let selection = book
        .state_images
        .subject_selection
        .as_ref()
        .filter(|selection| selection.scene == view.scene_key);
    let framed_subject =
        selection.and_then(|selection| selection.resolve(&snapshot.model, &scene.cast));
    let focus = view.story.as_ref().and_then(|story| {
        let selection = selection?;
        let actor = scene.identity_cast.iter().find(|actor| {
            actor.id.strip_prefix("npc:").unwrap_or(&actor.id)
                == story
                    .actor_id
                    .strip_prefix("npc:")
                    .unwrap_or(&story.actor_id)
        })?;
        selection.narrative(
            &snapshot.model,
            story.sequence,
            actor.id.strip_prefix("npc:").unwrap_or(&actor.id),
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
    let frame_changed = images.framed_subject != framed_subject;
    images.framed_subject = framed_subject.clone();
    let focus_changed = focus.as_ref().is_some_and(|focus| {
        images
            .focus
            .as_ref()
            .is_none_or(|known| known.focus.subject != focus.subject)
    });
    let focus_changed = focus_changed || frame_changed;
    if focus_changed && focus.is_none() {
        images.focus = None;
    }
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
    let state = match serde_json::to_string(&framed_subject) {
        Ok(subject) => format!("{state}|committed_subject={subject}"),
        Err(_) => {
            record("gameplay.living_scene.invalid_focus", &[]);
            return;
        }
    };
    let state = if scene.hero_activity.is_empty() && scene.scene_outcomes.is_empty() {
        state
    } else {
        format!(
            "{state}|hero_activity={}",
            serde_json::json!({"heroes": scene.hero_activity, "outcomes": scene.scene_outcomes})
        )
    };
    let state = format!(
        "{state}|cast={:?}|speaker={:?}",
        scene
            .identity_cast
            .iter()
            .map(|actor| &actor.id)
            .collect::<Vec<_>>(),
        scene.speaker.as_ref().map(|speaker| &speaker.id)
    );
    if scene_changed && !focus_changed {
        images.observed = state;
        images.observed_graph = view.graph_revision;
        return;
    }
    // An arriving or newly speaking actor must be included even if the authored model lags
    // the public cast. IDs come from the committed projection, never from player prose.
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
        cadence_due: None,
        framed_subject: images.framed_subject.clone(),
        graph: None,
        focus: images.focus.as_ref().map(|focus| focus.focus.clone()),
    });
}

pub(super) fn same_place(book: &super::Book, scene: &PlateScene) -> bool {
    book.current
        .as_ref()
        .filter(|current| !current.in_combat)
        .and_then(|current| current.scene.as_ref())
        .is_some_and(|current| !scene.place_id.is_empty() && current.place_id == scene.place_id)
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
    images.pending = Some(Pending {
        snapshot: None,
        story: Some(Box::new(story.clone())),
        reason: "completed NPC story beat".to_owned(),
        state: story.sha256.clone(),
        revision: view.graph_revision,
        changed_at: now,
        cadence_due: None,
        framed_subject: images.framed_subject.clone(),
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
        if pending.focus.is_some() || pending.framed_subject.is_some() || pending.snapshot.is_some()
        {
            statuses.extend(identity.scene.identity_cast.iter().map(|actor| {
                let id = actor.id.strip_prefix("npc:").unwrap_or(&actor.id);
                let mut status = self.inner.canonical_npc_status(id);
                if status == ReferenceStatus::Missing && self.inner.request_cast_photograph(id) {
                    status = ReferenceStatus::Pending;
                }
                status
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
        // A speaker/reference update can already own the fresh base we need. Adopt that
        // bounded render and its completion instead of queuing a duplicate refresh behind it.
        if matches!(book.current_entry(), Some(super::Entry::Pending)) {
            book.state_images.refresh = Some(id);
            book.state_images.last_started = Some(now);
            refresh_decision(book, pipeline, "awaiting_inflight_fresh_base");
            return true;
        }
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
        book.state_images.last_started = Some(now);
        refresh_decision(book, pipeline, "edit_chain_capped");
        true
    }

    pub(super) fn poll_state_images(&self) {
        self.poll_state_images_at(self.inner.now());
    }

    pub(super) fn poll_state_images_at(&self, now: Duration) {
        let mut book = locked(&self.book);
        if book.presentation_held
            && book
                .presented
                .as_ref()
                .is_some_and(|shown| now.saturating_sub(shown.since) >= MINIMUM_PRESENTATION)
        {
            self.wake();
            book.presentation_held = false;
        }
        let Some(current) = &book.current else {
            return;
        };
        if book.state_images.item_shot.as_ref().is_some_and(|shot| {
            shot.shown_at
                .is_some_and(|shown| now.saturating_sub(shown) >= MINIMUM_PRESENTATION)
        }) {
            self.wake();
        }
        let blocked = if current.in_combat {
            Some("combat")
        } else if current.in_roll {
            Some("roll")
        } else {
            None
        };
        if let Some(reason) = blocked {
            refresh_decision(&mut book, "state", reason);
            return;
        }
        let scene = current.scene_key.clone();
        let graph_revision = current.graph_revision;
        let identities = current.scene.clone().map(|scene| {
            Box::new(IdentitySnapshot {
                shot_sequence: book.state_images.cadence_sequence.saturating_add(1),
                face: scene
                    .speaker
                    .as_ref()
                    .and_then(|speaker| self.inner.plate_wanted_face(&speaker.id)),
                scene,
                heroes: current.heroes.clone(),
            })
        });
        self.present_state_image(&mut book, now);
        if book.state_images.pending.is_none()
            && (book.shown().is_some()
                || book
                    .current
                    .as_ref()
                    .is_some_and(|current| current.held.is_some()))
            && let Some(deadline) = book.state_images.cadence_deadline
            && let Some(identity) = identities.as_deref()
            && book
                .state_images
                .last_started
                .is_none_or(|last| now.saturating_sub(last) >= SCENE_CADENCE)
        {
            let images = &mut book.state_images;
            images.observed_graph = graph_revision;
            let sequence = images.cadence_sequence.saturating_add(1);
            images.pending = Some(Pending {
                snapshot: identity.scene.snapshot.clone(),
                story: images.latest_story.clone().map(Box::new),
                reason: format!(
                    "REF-068 current narrative moment {sequence}; a new camera angle on the recorded scene"
                ),
                state: format!(
                    "{}|cadence_context={}",
                    images.observed,
                    images
                        .latest_story
                        .as_ref()
                        .map_or("none", |story| story.sha256.as_str())
                ),
                revision: images.observed_graph,
                changed_at: now.saturating_sub(SETTLE),
                graph: None,
                focus: images.focus.as_ref().map(|focus| focus.focus.clone()),
                cadence_due: Some(deadline),
                framed_subject: images.framed_subject.clone(),
            });
        }
        let Some(pending) = &book.state_images.pending else {
            refresh_decision(&mut book, "state", "no_visible_change");
            return;
        };
        let is_focus = pending.focus.is_some();
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
        let interval = BAKE_INTERVAL;
        let cadence_due = book
            .state_images
            .last_started
            .or(book.state_images.last_shown)
            .is_some_and(|last| now.saturating_sub(last) >= SCENE_CADENCE);
        if (now.saturating_sub(pending.changed_at) < SETTLE && !cadence_due)
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
        if book.state_images.scene_in_flight()
            + usize::from(book.state_images.refresh.is_some())
            + usize::from(matches!(book.edit, Edit::Pending(_)))
            + usize::from(book.direct.pending)
            >= MAXIMUM_PLATE_RENDERS
        {
            refresh_decision(&mut book, "state", "pipeline_full");
            return;
        }
        // A spot move can keep using this place's original fresh base while the new framing
        // renders. Never turn a previously edited picture into the base of another edit.
        let Some((base, _)) = book
            .fresh_base(true)
            .or_else(|| book.state_images.retained_base.clone())
        else {
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
        let sequence = book.state_images.cadence_sequence.saturating_add(1);
        let composition = Composition {
            sequence,
            scene: scene.clone(),
            retired: false,
            beat: pending.story.as_ref().map_or(0, |story| story.sequence),
            started_at: now,
            state: pending.state.clone(),
            revision: pending.revision,
            due: pending.cadence_due.unwrap_or(now),
        };
        // The completion carries an opaque unique token. It is looked up, never parsed; a
        // return to the same scene/state cannot adopt a previous visit's render.
        pending.state = format!("{}|composition={sequence}", pending.state);
        let completion_token = pending.state.clone();
        let focus_state = composition.state.clone();
        let focus_revision = pending.revision;
        let job = super::Job::State(Job::Bake {
            scene,
            pending,
            base,
            identities,
        });
        let mut queued = false;
        let mut wake_after = false;
        match self.jobs.try_send(job) {
            Ok(()) => {
                queued = true;
                crate::gameplay::actor::record_audit_line("gameplay.imagery.request", request_line);
                book.state_images.last_started = Some(now);
                book.state_images.cadence_sequence = sequence;
                book.state_images
                    .in_flight
                    .insert(completion_token, composition.clone());
                record_pipeline(
                    "start",
                    &composition,
                    now,
                    book.state_images.scene_in_flight(),
                );
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
                let mut pending = pending;
                pending.state = composition.state;
                book.state_images.pending = Some(pending);
                refresh_decision(&mut book, "state", "queue_full");
            }
            Err(_) => {
                wake_after = true;
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
        if queued || wake_after {
            drop(book);
            // The story-directed render has started (or failed to start): publish pending.
            self.wake();
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
            .photographic_request(
                &identity.scene,
                &spec,
                &identity.heroes,
                None,
                identity.face,
            )
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
        } else if let Some(story) = &pending.story
            && pending.snapshot.is_none()
        {
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
        let committed_place_canon = locked(&self.inner.state)
            .canonical
            .committed_canon(Some(&identity.scene.place_id), None);
        match self
            .plate_room_reference(&identity.scene, !committed_place_canon.is_empty())
            .await?
        {
            Some(reference) => references.push(reference),
            None if pending.focus.is_some() => return Err(ImageFailure::InvalidRequest),
            None => {}
        }
        let prompt = {
            let super::PlateIdentities {
                sheet,
                legend,
                people,
                holder_ids,
            } = self
                .identity_sheet(&identity.scene, &identity.heroes)
                .await?;
            let staging = self.spatial_staging(&identity.scene, &identity.heroes)?;
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
            let narrative = pending
                .story
                .as_ref()
                .map_or(identity.scene.story_text.as_str(), |story| {
                    story.text.as_str()
                });
            let narrative = format!("{narrative}\n{}", pending.reason);
            let model = pending
                .snapshot
                .as_ref()
                .or(identity.scene.snapshot.as_ref())
                .map(|snapshot| &snapshot.model);
            let mut prompt = prompt
                .with_cast_hands(&people, &holder_ids, model)
                .map_err(|_| ImageFailure::InvalidRequest)?
                .with_scene_context(
                    &narrative,
                    pending
                        .snapshot
                        .as_ref()
                        .or(identity.scene.snapshot.as_ref())
                        .map(|snapshot| &snapshot.model),
                )
                .map_err(|_| ImageFailure::InvalidRequest)?;
            if let Some(subject) = &pending.framed_subject {
                let framing =
                    serde_json::to_string(subject).map_err(|_| ImageFailure::InvalidRequest)?;
                prompt = prompt
                    .with_scene_context(&framing, model)
                    .map_err(|_| ImageFailure::InvalidRequest)?;
            }
            prompt = identity.scene.with_activity(prompt)?;
            if pending.focus.is_none() && identity.scene.room_revision.is_some() {
                prompt = prompt.with_canonical_room_reference(2);
            }
            prompt = prompt.with_cast_sheet(sheet_number, &legend);
            let party = super::SceneParty::new(identity.heroes.len(), sheet_number)
                .map_err(|_| ImageFailure::InvalidRequest)?;
            prompt = prompt
                .with_canonical_binding(&party.instruction())
                .with_scene_party(party);
            if pending.focus.is_none()
                && let (Some(number), Some(speaker)) = (speaker_number, active_speaker)
            {
                prompt =
                    prompt.with_canonical_speaker_reference(number, &speaker.id, &speaker.name);
            }
            prompt
                .with_staging_order(staging)
                .map_err(|_| ImageFailure::InvalidRequest)?
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
                let cadence = pending.cadence_due.is_some() && pending.framed_subject.is_none();
                let fresh = cadence
                    || (self.inner.imagery.flux_prompts()
                        && pending.focus.is_none()
                        && pending.story.is_some()
                        && pending.snapshot.is_none());
                let result = async {
                    let request = if cadence {
                        let identity = identities.as_deref().ok_or(ImageFailure::InvalidRequest)?;
                        let mut spec = identity.scene.stage(&identity.heroes)
                            .map_err(|_| ImageFailure::InvalidRequest)?;
                        spec.camera = crate::gameplay::shot::Camera::for_public_beat(
                            spec.camera, identity.scene.key,
                            pending.story.as_ref().map_or(shot::PublicBeat::Calm, |story| story.tempo),
                            identity.shot_sequence,
                            shot::PublicDialogue::SameSpeakerFollowup,
                        );
                        let face = identity.face;
                        let (mut request, _) = self.photographic_request(
                            &identity.scene, &spec, &identity.heroes, None, face,
                        ).await?;
                        if let Some(graph) = &pending.graph {
                            let public_moment = serde_json::to_string(graph).map_err(|_| ImageFailure::InvalidRequest)?;
                            request.prompt = request.prompt.with_scene_context(&public_moment, None)
                                .map_err(|_| ImageFailure::InvalidRequest)?;
                        }
                        request.prompt = request.prompt.with_scene_context(
                            &pending.reason, identity.scene.snapshot.as_ref().map(|snapshot| &snapshot.model),
                        ).map_err(|_| ImageFailure::InvalidRequest)?;
                        request
                    } else if fresh {
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
                    let mut request = request;
                    if let Some(identity) = identities.as_deref() {
                        let mut scene = identity.scene.clone();
                        scene.snapshot = pending.snapshot.clone().or(scene.snapshot);
                        scene.picture = std::sync::OnceLock::new();
                        request.prompt = super::photo_context(&scene, request.prompt)?;
                    }
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
                    if let Some(subject) = pending.framed_subject.as_ref() {
                        subject.record_request(pending.revision, &self.inner.imagery.key_for(&request));
                    }
                    let subject = format!("plate:{}", identities.as_deref().map_or("", |identity| identity.scene.place_id.as_str()));
                    let key = self.inner.imagery.generate_with_priority(
                        &subject, request, ImagePriority::Foreground,
                    ).await?;
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
                priority,
                focus_hero,
                focus_pose,
            } => {
                let result = async {
                    let request = item_image_request(self.inner.rating, &thing, detail.as_deref())?;
                    let canonical = self
                        .inner
                        .imagery
                        .generate_with_priority(&format!("item:{id}"), request, priority)
                        .await?;
                    let Some(hero) = focus_hero else {
                        return Ok(canonical);
                    };
                    let base = base.ok_or(ImageFailure::Interrupted)?;
                    let Lookup::Image { bytes, format } =
                        self.inner.imagery.image(base.as_str()).await
                    else {
                        return Err(ImageFailure::Interrupted);
                    };
                    let base = ReferenceImage::new(format, bytes);
                    let Lookup::Image { bytes, format } =
                        self.inner.imagery.image(canonical.as_str()).await
                    else {
                        return Err(ImageFailure::Interrupted);
                    };
                    let canonical = ReferenceImage::new(format, bytes);
                    let mut focus_thing = thing;
                    if matches!(focus_pose, ItemFocusPose::InspectionHands) {
                        focus_thing.holder_id = Some(hero.clone());
                    }
                    let request = ImageRequest::item_focus(
                        self.inner.rating,
                        &focus_thing,
                        detail.as_deref(),
                        &hero,
                        [base, canonical],
                    )?;
                    self.inner
                        .imagery
                        .generate_with_priority(
                            &format!("item-focus:{id}"),
                            request,
                            ImagePriority::Foreground,
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

    fn present_state_image(&self, book: &mut super::Book, now: Duration) {
        if book.state_images.item_shot.as_ref().is_some_and(|shot| {
            book.state_images
                .items
                .get(&shot.id)
                .is_some_and(|item| !item.failed)
        }) {
            return;
        }
        if book
            .current
            .as_ref()
            .is_none_or(|current| current.in_combat || current.in_roll)
            || book
                .state_images
                .last_shown
                .is_some_and(|last| now.saturating_sub(last) < SCENE_CADENCE)
        {
            return;
        }
        let Some(ready) = book.state_images.ready.take() else {
            return;
        };
        let Ready {
            key,
            composition,
            story_actor,
            fresh,
        } = ready;
        let layout = book.image_layouts.get(key.as_str()).cloned();
        focus_progress(
            &mut book.state_images,
            &composition.state,
            composition.revision,
            Stage::Published,
            Some(&key),
            now,
            None,
        );
        book.edit = Edit::Idle;
        let images = &mut book.state_images;
        images.shown = Some((key.clone(), layout));
        images.carried_edit = !fresh;
        images.baked_revision = Some(composition.revision);
        images.shown_sequence = composition.sequence;
        images.last_shown = Some(now);
        images.cadence_deadline = Some(now.saturating_add(SCENE_CADENCE));
        images.projection_deadline = Some((key.clone(), composition.due));
        images.selected = Some((key, composition));
        if let Some(actor) = story_actor {
            images.last_story_actor = Some(actor);
        }
        self.wake();
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
                let Some(composition) = book.state_images.in_flight.remove(&state) else {
                    // Unknown completions cannot release or present another request's work.
                    return;
                };
                if composition.retired {
                    // Retirement was audited at scene exit. Keep the cached result only.
                } else if book
                    .current
                    .as_ref()
                    .is_none_or(|current| current.scene_key != composition.scene)
                    || composition.sequence <= book.state_images.shown_sequence
                {
                    record_pipeline(
                        "retired",
                        &composition,
                        now,
                        book.state_images.scene_in_flight(),
                    );
                } else {
                    if scene != composition.scene {
                        record(
                            "gameplay.living_scene.plate_spot_completion",
                            &[
                                ("render.scene", scene),
                                ("presentation.scene", composition.scene.clone()),
                                ("sequence", composition.sequence.to_string()),
                            ],
                        );
                    }
                    match result {
                        Ok(key) => {
                            let ready = Ready {
                                key,
                                composition,
                                story_actor,
                                fresh,
                            };
                            if book.state_images.ready.as_ref().is_none_or(|waiting| {
                                waiting.composition.sequence < ready.composition.sequence
                            }) {
                                if let Some(older) = book.state_images.ready.replace(ready) {
                                    record_pipeline(
                                        "retired",
                                        &older.composition,
                                        now,
                                        book.state_images.scene_in_flight(),
                                    );
                                }
                            } else {
                                record_pipeline(
                                    "retired",
                                    &ready.composition,
                                    now,
                                    book.state_images.scene_in_flight(),
                                );
                            }
                            self.present_state_image(&mut book, now);
                        }
                        Err(failure) => {
                            record_pipeline(
                                "retired",
                                &composition,
                                now,
                                book.state_images.scene_in_flight(),
                            );
                            focus_progress(
                                &mut book.state_images,
                                &composition.state,
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

fn record_cadence(stage: &str, revision: u64, deadline: Duration, now: Duration) {
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.living_scene.plate_cadence",
        format!(
            "PLATE cadence stage={stage} revision={revision} deadline_ms={} at_ms={} in_time={}",
            deadline.as_millis(),
            now.as_millis(),
            now <= deadline
        ),
        serde_json::Map::from_iter([
            ("stage".into(), serde_json::json!(stage)),
            ("graph_revision".into(), serde_json::json!(revision)),
            (
                "deadline_ms".into(),
                serde_json::json!(deadline.as_millis()),
            ),
            ("at_ms".into(), serde_json::json!(now.as_millis())),
            ("in_time".into(), serde_json::json!(now <= deadline)),
            (
                "lateness_ms".into(),
                serde_json::json!(now.saturating_sub(deadline).as_millis()),
            ),
            (
                "observation_boundary".into(),
                serde_json::json!("server_plate_projection"),
            ),
        ]),
        &[],
    );
}

fn record_pipeline(stage: &str, composition: &Composition, now: Duration, in_flight: usize) {
    let age_ms = now.saturating_sub(composition.started_at).as_millis();
    crate::gameplay::actor::record_audit_diagnostic(
        "gameplay.living_scene.plate_cadence",
        format!(
            "PLATE cadence stage={stage} seq={} beat={} age_ms={age_ms} in_flight={in_flight}",
            composition.sequence, composition.beat
        ),
        serde_json::Map::from_iter([
            ("stage".into(), serde_json::json!(stage)),
            ("sequence".into(), serde_json::json!(composition.sequence)),
            ("beat".into(), serde_json::json!(composition.beat)),
            ("age_ms".into(), serde_json::json!(age_ms)),
            ("in_flight".into(), serde_json::json!(in_flight)),
            ("at_ms".into(), serde_json::json!(now.as_millis())),
        ]),
        &[],
    );
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

#[cfg(test)]
mod cadence_tests {
    use super::*;

    use super::super::tests::{
        admit_canonical_references, drain, finish_canonical_references, key, scene, shared, view,
    };

    fn pipeline_fixture() -> (
        Arc<Shared>,
        mpsc::Receiver<super::super::Job>,
        pack::Scene,
        View,
    ) {
        let mut current = scene(SceneKey::Opening, "HS61 cadence", Some("Kestrel"));
        current.setting.as_mut().unwrap().model = Some(
            serde_json::from_slice(include_bytes!(
                "../../../../../../content/merchant-shop/scene-model.json"
            ))
            .unwrap(),
        );
        let (shared, mut queue) = shared(16);
        admit_canonical_references(&shared.inner, &current);
        finish_canonical_references(&shared.inner);
        locked(&shared.inner.state)
            .canonical
            .fixture_hero(1, "Hero 1", key('a'));
        let projected = view(&current, &[1], false);
        shared.observe_with(&projected, Some(current.clone()), Vec::new());
        drain(&mut queue);
        {
            let mut book = locked(&shared.book);
            let id = book.current.as_ref().unwrap().id.clone().unwrap();
            book.insert(
                id,
                super::super::Entry::Ready {
                    key: key('a'),
                    placement: None,
                },
            );
        }
        shared.apply_at(
            &projected.scene_key,
            &mut rpc::GameplayScene::default(),
            Duration::ZERO,
        );
        (shared, queue, current, projected)
    }

    fn commit_beat(
        shared: &Shared,
        current: &pack::Scene,
        projected: &mut View,
        sequence: u64,
        now: Duration,
        modelled: bool,
    ) {
        projected.graph_revision = sequence;
        projected.story = Some(StoryBeat {
            actor_id: current.cast[0].id.clone(),
            sequence,
            speaker: current.cast[0].name.clone(),
            text: format!("Recorded vendor moment {sequence}: examining the wares."),
            sha256: sequence.to_string(),
            tempo: shot::PublicBeat::Calm,
            dialogue: shot::PublicDialogue::SameSpeakerFollowup,
        });
        let mut plate = PlateScene::for_view(projected, current).unwrap();
        if !modelled {
            plate.snapshot = None;
        }
        let mut book = locked(&shared.book);
        observe(&mut book, projected, &plate, now);
        let current = book.current.as_mut().unwrap();
        current.scene = Some(plate);
        current.graph_revision = sequence;
    }

    struct FakeRender {
        scene: String,
        pending: Pending,
        finishes_at: Duration,
    }

    impl FakeRender {
        fn from_job(job: super::super::Job, finishes_at: Duration) -> Self {
            let super::super::Job::State(Job::Bake { scene, pending, .. }) = job else {
                panic!("expected a plate bake")
            };
            Self {
                scene,
                pending,
                finishes_at,
            }
        }

        fn complete(self, shared: &Shared, now: Duration) {
            let sequence =
                locked(&shared.book).state_images.in_flight[&self.pending.state].sequence;
            shared.finish_state_image_at(
                Done::Bake {
                    scene: self.scene,
                    state: self.pending.state,
                    revision: self.pending.revision,
                    story_actor: self.pending.story.as_deref().map(StoryBeat::actor_key),
                    fresh: true,
                    result: Ok(ImageKey::parse(&format!("{sequence:064x}")).unwrap()),
                },
                now,
            );
        }
    }

    #[test]
    fn hs61_plates_pipeline_sixty_to_eighty_second_renders_for_five_minutes() {
        let (shared, mut queue, current, mut projected) = pipeline_fixture();
        let mut renders: Vec<FakeRender> = Vec::new();
        let mut starts = Vec::new();
        let mut presentations = Vec::new();
        let mut last_key = key('a').path();
        let mut maximum_in_flight = 0;
        for second in 0..=300 {
            let now = Duration::from_secs(second);
            if second % 7 == 0 {
                commit_beat(&shared, &current, &mut projected, second / 7 + 1, now, true);
            }
            shared.poll_state_images_at(now);
            for job in drain(&mut queue) {
                let mut render = FakeRender::from_job(job, now);
                let book = locked(&shared.book);
                let composition = &book.state_images.in_flight[&render.pending.state];
                assert_eq!(
                    composition.beat,
                    second / 7 + 1,
                    "compose the beat current at start"
                );
                render.finishes_at =
                    now + Duration::from_secs(60 + (composition.sequence - 1) % 5 * 5);
                starts.push(second);
                renders.push(render);
            }
            maximum_in_flight = maximum_in_flight.max(renders.len());
            assert!(renders.len() <= 3);
            let mut index = 0;
            while index < renders.len() {
                if renders[index].finishes_at <= now {
                    renders.remove(index).complete(&shared, now);
                } else {
                    index += 1;
                }
            }
            let mut output = rpc::GameplayScene::default();
            shared.apply_at(&projected.scene_key, &mut output, now);
            if output.live_plate != last_key {
                presentations.push(second);
                last_key = output.live_plate;
            }
        }
        assert_eq!(maximum_in_flight, 3);
        assert!(starts.windows(2).all(|pair| pair[1] - pair[0] >= 15));
        assert!(presentations.len() >= 7, "timeline: {presentations:?}");
        assert!(
            presentations
                .windows(2)
                .all(|pair| (25..=35).contains(&(pair[1] - pair[0]))),
            "timeline: {presentations:?}"
        );
        println!(
            "HS61 starts_s={starts:?} presented_s={presentations:?} max_in_flight={maximum_in_flight}"
        );
    }

    #[test]
    fn hs61_plates_next_start_captures_the_new_subject_item_and_mood() {
        use crate::gameplay::scene_mood::{Expression, HealthTier};
        let (shared, mut queue, current, mut projected) = pipeline_fixture();
        commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, true);
        shared.poll_state_images_at(Duration::from_secs(3));
        let first = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
        let target = {
            let book = locked(&shared.book);
            let model = &book
                .current
                .as_ref()
                .unwrap()
                .scene
                .as_ref()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .model;
            model
                .things
                .iter()
                .find(|thing| thing.in_scene && thing.publicly_visible)
                .unwrap()
                .id
                .clone()
        };
        locked(&shared.book).state_images.subject_selection = Some(subject::Selection {
            scene: projected.scene_key.clone(),
            hero: "Hero 1".into(),
            target: target.clone(),
        });
        let angry = Face {
            expression: Expression::Angry,
            tier: HealthTier::Healthy,
        };
        locked(&shared.inner.state)
            .plate_wants
            .insert(current.cast[0].id.clone(), angry);
        commit_beat(
            &shared,
            &current,
            &mut projected,
            2,
            Duration::from_secs(4),
            true,
        );
        shared.poll_state_images_at(Duration::from_secs(17));
        assert!(
            drain(&mut queue).is_empty(),
            "all starts respect the fifteen-second gate"
        );
        shared.poll_state_images_at(Duration::from_secs(18));
        let super::super::Job::State(Job::Bake {
            pending,
            identities,
            ..
        }) = drain(&mut queue).remove(0)
        else {
            panic!("next composition")
        };
        assert_eq!(first.pending.story.as_ref().unwrap().sequence, 1);
        assert_eq!(pending.story.as_ref().unwrap().sequence, 2);
        assert!(pending.framed_subject.is_some());
        assert_eq!(pending.focus.as_ref().unwrap().subject.id(), target);
        let identity = identities.unwrap();
        assert_eq!(identity.face, Some(angry));
        assert_eq!(
            identity.scene.story_text,
            projected.story.as_ref().unwrap().text
        );
        assert_eq!(
            identity.scene.speaker.as_ref().unwrap().id,
            projected.story.as_ref().unwrap().actor_id
        );
        locked(&shared.inner.state).plate_wants.clear();
        assert_eq!(
            identity.face,
            Some(angry),
            "later mood changes cannot mutate admitted work"
        );
        assert_eq!(locked(&shared.book).state_images.scene_in_flight(), 2);
    }

    #[test]
    fn hs61_plates_new_beats_do_not_cancel_renders_and_out_of_order_never_regresses() {
        let (shared, mut queue, current, mut projected) = pipeline_fixture();
        let mut renders = Vec::new();
        for (sequence, second) in [(1, 0), (2, 15), (3, 30)] {
            commit_beat(
                &shared,
                &current,
                &mut projected,
                sequence,
                Duration::from_secs(second),
                false,
            );
            shared.poll_state_images_at(Duration::from_secs(second + 3));
            let render = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
            assert_eq!(render.pending.story.as_ref().unwrap().sequence, sequence);
            renders.push(render);
        }
        shared.poll_state_images_at(Duration::from_secs(63));
        assert!(
            drain(&mut queue).is_empty(),
            "three admitted renders bound the pipeline"
        );
        // Second render takes 60 seconds, first takes 80: the newer composition wins.
        renders.remove(1).complete(&shared, Duration::from_secs(78));
        let mut output = rpc::GameplayScene::default();
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(78));
        let newer = output.live_plate.clone();
        renders.remove(0).complete(&shared, Duration::from_secs(83));
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(83));
        assert_eq!(
            output.live_plate, newer,
            "old completion cannot step backwards"
        );
        renders.remove(0).complete(&shared, Duration::from_secs(93));
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(102));
        assert_eq!(
            output.live_plate, newer,
            "hold a finished plate for reading time"
        );
        shared.poll_state_images_at(Duration::from_secs(108));
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(108));
        assert_ne!(
            output.live_plate, newer,
            "deliver waiting newer work within 35 seconds"
        );
        assert_eq!(locked(&shared.book).state_images.shown_sequence, 3);
    }

    #[test]
    fn hs61_plates_scene_exit_retires_work_even_after_returning_to_the_same_scene() {
        let (shared, mut queue, current, mut projected) = pipeline_fixture();
        commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, false);
        shared.poll_state_images_at(Duration::from_secs(3));
        let old = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
        let original = projected.clone();
        projected.scene_key = "another-scene".into();
        {
            let mut book = locked(&shared.book);
            reset_scene(&mut book, &projected, Duration::from_secs(10));
            assert!(
                book.state_images
                    .in_flight
                    .values()
                    .all(|composition| composition.retired)
            );
            reset_scene(&mut book, &original, Duration::from_secs(20));
        }
        projected = original;
        commit_beat(
            &shared,
            &current,
            &mut projected,
            2,
            Duration::from_secs(20),
            false,
        );
        shared.poll_state_images_at(Duration::from_secs(23));
        let newer = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
        old.complete(&shared, Duration::from_secs(83));
        {
            let book = locked(&shared.book);
            assert!(book.state_images.shown.is_none());
            assert!(book.state_images.ready.is_none());
            assert!(
                book.state_images.has_in_flight(),
                "an old completion cannot release newer work"
            );
        }
        newer.complete(&shared, Duration::from_secs(84));
        assert_eq!(locked(&shared.book).state_images.shown_sequence, 2);
    }

    #[test]
    fn hs65_local_spot_keeps_the_presented_plate_until_its_replacement_is_ready() {
        let (shared, mut queue, current, projected) = pipeline_fixture();
        let mut spot = current.clone();
        spot.title = "Oren's stall".into();
        spot.setting.as_mut().unwrap().short = "Oren's stall in the same courtyard".into();
        let at_stall = view(&spot, &[1], false);
        assert_ne!(projected.scene_key, at_stall.scene_key);
        shared.observe_with(&at_stall, Some(spot.clone()), Vec::new());
        drain(&mut queue);
        let mut output = rpc::GameplayScene::default();
        shared.apply_at(&at_stall.scene_key, &mut output, Duration::from_secs(10));
        assert_eq!(output.live_plate, key('a').path());
        assert_eq!(output.plate_state, rpc::PlateState::Ready as i32);
        assert_eq!(
            locked(&shared.book).presented.as_ref().unwrap().since,
            Duration::ZERO
        );

        spot.title = "Another place".into();
        spot.setting.as_mut().unwrap().place_id = "different-canonical-place".into();
        let elsewhere = view(&spot, &[1], false);
        shared.observe_with(&elsewhere, Some(spot), Vec::new());
        let mut output = rpc::GameplayScene::default();
        shared.apply_at(&elsewhere.scene_key, &mut output, Duration::from_secs(11));
        assert_ne!(
            output.live_plate,
            key('a').path(),
            "a real place change cannot hold the old picture"
        );
    }

    #[test]
    fn hs65_local_spot_keeps_in_flight_cadence_and_newer_stall_work_supersedes_it() {
        let (shared, mut queue, current, mut projected) = pipeline_fixture();
        commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, false);
        shared.poll_state_images_at(Duration::from_secs(3));
        let old = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
        let mut spot = current.clone();
        spot.title = "Oren's stall".into();
        let at_stall = view(&spot, &[1], false);
        shared.observe_with(&at_stall, Some(spot.clone()), Vec::new());
        drain(&mut queue);
        {
            let book = locked(&shared.book);
            assert_eq!(book.state_images.scene_in_flight(), 1);
            assert!(
                book.state_images
                    .in_flight
                    .values()
                    .all(|work| !work.retired)
            );
        }
        old.complete(&shared, Duration::from_secs(31));
        let mut output = rpc::GameplayScene::default();
        shared.apply_at(&at_stall.scene_key, &mut output, Duration::from_secs(31));
        assert_eq!(output.live_plate, format!("/gameplay/imagery/{:064x}", 1));
        projected = at_stall;
        commit_beat(
            &shared,
            &spot,
            &mut projected,
            2,
            Duration::from_secs(33),
            false,
        );
        shared.poll_state_images_at(Duration::from_secs(36));
        let newer = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
        newer.complete(&shared, Duration::from_secs(62));
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(62));
        assert_eq!(output.live_plate, format!("/gameplay/imagery/{:064x}", 2));
    }

    #[tokio::test]
    async fn hs65_committed_ware_is_foreground_and_the_next_shot_then_returns_to_the_scene() {
        use crate::gameplay::imagery::tests::DirectionFixture;
        let (mut shared, mut queue, current, mut projected) = pipeline_fixture();
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/tmp/lanes/HS71/item-shot")
            .join(std::process::id().to_string());
        let fixture = DirectionFixture::photographic(&cache);
        Arc::get_mut(&mut Arc::get_mut(&mut shared).unwrap().inner)
            .unwrap()
            .imagery = fixture.imagery.clone();
        let model = current.setting.as_ref().unwrap().model.as_ref().unwrap();
        let before = model.public_contents().unwrap();
        // Offer-only stock is projected for inspection, never materialized into inventory.
        let offer = &model.offers[0];
        let mut ware = model.things[0].clone();
        ware.id = offer.item_id.clone();
        ware.name = offer.name.clone();
        ware.detail = offer.description.clone();
        ware.picture_description = offer.description.clone();
        ware.owner_id = Some(offer.seller_id.clone());
        ware.holder_id = Some(offer.seller_id.clone());
        assert!(model.things.iter().all(|thing| thing.id != ware.id));
        // Unlike the old object-only card, a focus shot reads the real current plate bytes.
        let base = fixture
            .imagery
            .generate_with_priority(
                "fixture-scene",
                item_image_request(shared.inner.rating, &ware, None).unwrap(),
                ImagePriority::Foreground,
            )
            .await
            .unwrap();
        {
            let mut book = locked(&shared.book);
            let id = book.current.as_ref().unwrap().id.clone().unwrap();
            book.insert(
                id,
                super::super::Entry::Ready {
                    key: base.clone(),
                    placement: None,
                },
            );
            let shown = book.presented.as_mut().unwrap();
            shown.key = base.clone();
            shown.since = Duration::ZERO;
        }
        commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, true);
        shared.poll_state_images_at(Duration::from_secs(3));
        let cadence = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
        let plates = Plates {
            shared: Arc::clone(&shared),
            _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
        };
        plates
            .request_committed_item(
                "Brynn",
                &ware,
                Some("The folded linen has a blue hem.".into()),
                true,
            )
            .unwrap();
        let super::super::Job::State(job) = drain(&mut queue).remove(0) else {
            panic!("item job")
        };
        assert!(matches!(
            &job,
            Job::Item {
                priority: ImagePriority::Foreground,
                ..
            }
        ));
        let done = shared.render_state_image(job).await;
        let Done::Item {
            result: Ok(item_key),
            ..
        } = &done
        else {
            panic!("fake artifact_close_up completed")
        };
        let item_key = item_key.clone();
        assert_eq!(
            fixture.calls(),
            3,
            "current plate, canonical item, then one scene-focus edit"
        );
        assert!(fixture.prompt().contains(&ware.name));
        assert_eq!(
            fixture.image_count(),
            2,
            "scene and canonical item references"
        );
        assert!(fixture.prompt().contains("centre of the frame"));
        assert!(fixture.prompt().contains("softly out of focus"));
        assert!(!fixture.prompt().contains(&ware.closer_look));
        shared.finish_state_image_at(done, Duration::from_secs(5));
        cadence.complete(&shared, Duration::from_secs(20));
        let mut output = rpc::GameplayScene::default();
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(24));
        assert_eq!(
            output.live_plate,
            base.path(),
            "keep the scene's reading hold"
        );
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(25));
        assert_eq!(
            output.live_plate,
            item_key.path(),
            "the item precedes the ready cadence picture"
        );
        assert!(
            output.plate_layout.is_none(),
            "never lend the item the scene's people"
        );
        assert_eq!(
            locked(&shared.book)
                .state_images
                .item_shot
                .as_ref()
                .unwrap()
                .hero,
            "Brynn"
        );
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(49));
        assert_eq!(output.live_plate, item_key.path());
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(50));
        assert_eq!(output.live_plate, base.path());
        assert!(locked(&shared.book).state_images.item_shot.is_none());
        shared.poll_state_images_at(Duration::from_secs(80));
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(81));
        assert_eq!(output.live_plate, format!("/gameplay/imagery/{:064x}", 1));
        assert_eq!(model.public_contents().unwrap(), before);
        assert_eq!(
            locked(&shared.book)
                .current
                .as_ref()
                .unwrap()
                .scene
                .as_ref()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .model
                .public_contents()
                .unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn hs58_inventory_waits_for_a_plate_and_does_not_fill_the_plate_queue() {
        use super::super::tests::{drain, key, shared};
        let (shared, mut queue) = shared(16);
        let plates = Plates {
            shared: Arc::clone(&shared),
            _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
        };
        let card = |index| rpc::SceneItemCard {
            thing_id: format!("equipment-{index}"),
            name: "chain mail".into(),
            detail: "Iron chain mail".into(),
            picture_description: "Linked iron rings".into(),
            holder: "hero".into(),
            ..Default::default()
        };
        plates
            .request_hand_item_with_priority(&card(0), false, "{}", ImagePriority::Background)
            .unwrap();
        assert!(drain(&mut queue).is_empty());
        assert!(locked(&shared.book).state_images.items.is_empty());
        locked(&shared.book).state_images.shown = Some((key('a'), None));
        for index in 0..120 {
            plates
                .request_hand_item_with_priority(
                    &card(index),
                    false,
                    "{}",
                    ImagePriority::Background,
                )
                .unwrap();
        }
        assert_eq!(
            drain(&mut queue).len(),
            crate::gameplay::imagery::BACKGROUND_IN_FLIGHT
        );
        let first = locked(&shared.book)
            .state_images
            .items
            .keys()
            .next()
            .unwrap()
            .clone();
        shared.finish_state_image(Done::Item {
            id: first,
            result: Ok(key('b')),
        });
        plates
            .request_hand_item_with_priority(&card(119), false, "{}", ImagePriority::Background)
            .unwrap();
        assert_eq!(
            drain(&mut queue).len(),
            1,
            "later projections admit deferred cards as capacity returns"
        );
    }

    #[test]
    fn hs41_plates_player_text_cannot_be_mistaken_for_a_cadence_completion_marker() {
        let book = Book {
            observed: "canonical player text |cadence_context=ordinary words".into(),
            ..Default::default()
        };
        assert!(book.accepts_state(&book.observed));
        assert!(book.accepts_state(&format!("{}|cadence_context=none", book.observed)));
        assert!(!book.accepts_state("other state|cadence_context=none"));
    }
}

fn inventory_custody(scene_contents_json: &str, item_id: &str) -> (String, String) {
    let Some(contents) = serde_json::from_str::<Value>(scene_contents_json).ok() else {
        return (String::new(), String::new());
    };
    let Some(row) = contents
        .get("inventory")
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries
                .iter()
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

impl crate::gameplay::living_scenes::LivingScenes {
    /// Render-only temporary handling; authoritative stock, owner and holder remain unchanged.
    pub(in crate::gameplay) fn request_inspected_item_in_hands(
        &self,
        hero: &str,
        thing: &SceneThing,
        detail: Option<String>,
    ) -> Result<(), &'static str> {
        self.plates
            .as_ref()
            .ok_or("scene pictures are unavailable")?
            .request_item_with_visibility(
                hero,
                &thing.id,
                Some(thing.clone()),
                detail,
                true,
                ItemRequest::InspectionHands,
            )
    }
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
        self.request_item_with_visibility(
            hero,
            thing_id,
            None,
            detail,
            shared,
            ItemRequest::Explicit,
        )
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
            ItemRequest::Explicit,
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
        self.request_hand_item_with_priority(
            card,
            shared,
            scene_contents_json,
            ImagePriority::Foreground,
        )
    }

    pub(in crate::gameplay) fn request_hand_item_with_priority(
        &self,
        card: &rpc::SceneItemCard,
        shared: bool,
        scene_contents_json: &str,
        priority: ImagePriority,
    ) -> Result<(), &'static str> {
        let mut book = locked(&self.shared.book);
        // Projection repeatedly visits every starting inventory card. Admit that automatic
        // background work only after the current place has a plate; explicit examinations
        // still enter through request_item/request_committed_item immediately.
        if matches!(priority, ImagePriority::Background) && book.shown().is_none() {
            return Ok(());
        }
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
            let mut changed = false;
            if let Some(item) = book.state_images.items.get_mut(&id) {
                changed = shared && !item.shared;
                item.shared |= shared;
                if shared {
                    item.requested_at = now;
                }
            }
            let previous = book
                .state_images
                .latest_items
                .insert(card.holder.clone(), id.clone());
            changed |= previous.as_deref() != Some(id.as_str());
            drop(book);
            if changed {
                self.shared.wake();
            }
            return Ok(());
        }
        // Do not fill the plate director's bounded job set with inventory cards waiting for
        // background permits. Later projections retry cards that have not been admitted yet.
        if book
            .state_images
            .items
            .values()
            .filter(|item| item.key.is_none() && !item.failed)
            .count()
            >= crate::gameplay::imagery::BACKGROUND_IN_FLIGHT
        {
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
            ItemRequest::Hand(priority),
        )
    }

    fn request_item_with_visibility(
        &self,
        hero: &str,
        thing_id: &str,
        committed_thing: Option<SceneThing>,
        detail: Option<String>,
        shared: bool,
        request: ItemRequest,
    ) -> Result<(), &'static str> {
        let (explicitly_shared, priority, focus_pose) = match request {
            ItemRequest::Explicit => (shared, ImagePriority::Foreground, ItemFocusPose::Recorded),
            ItemRequest::InspectionHands => (
                true,
                ImagePriority::Foreground,
                ItemFocusPose::InspectionHands,
            ),
            ItemRequest::Hand(priority) => (false, priority, ItemFocusPose::Recorded),
        };
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
        let base = book.shown().map(|shown| shown.0);
        let focus = shared && explicitly_shared;
        let state = item_revision(&thing, detail.as_deref());
        let id = if focus {
            format!(
                "{}:{state}:focus:{hero}:{}:{}:inspection_hands={}",
                thing.id,
                thing.holder_id.as_deref().unwrap_or_default(),
                base.as_ref().map_or("", ImageKey::as_str),
                matches!(focus_pose, ItemFocusPose::InspectionHands),
            )
        } else {
            format!("{}:{state}", thing.id)
        };
        let now = self.shared.inner.now();
        let shot = (shared && explicitly_shared).then(|| ItemShot {
            id: id.clone(),
            hero: hero.to_owned(),
            connected: thing
                .finding_id
                .as_deref()
                .is_some_and(|id| id != format!("item-story-ordinary-{}", thing.id))
                || thing.clue_fact_id.is_some(),
            shown_at: None,
        });
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
            if let Some(shot) = shot {
                book.state_images.item_shot = Some(shot);
            }
            drop(book);
            self.shared.wake();
            return Ok(());
        }
        if book.state_images.items.len() >= 128 {
            return Err("object picture cache is full");
        }
        let admission = self.shared.jobs.try_send(super::Job::State(Job::Item {
            id: id.clone(),
            thing: thing.clone(),
            detail: detail.clone(),
            base,
            priority,
            focus_hero: focus.then(|| hero.to_owned()),
            focus_pose,
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
        if admission.is_ok()
            && let Some(shot) = shot
        {
            book.state_images.item_shot = Some(shot);
        }
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
            provenance: SceneItemProvenance {
                source_id: contents
                    .get("inventory")
                    .and_then(Value::as_array)
                    .and_then(|entries| {
                        entries.iter().find(|entry| {
                            entry.get("item_id").and_then(Value::as_str) == Some(&card.thing_id)
                        })
                    })
                    .and_then(|entry| entry.get("source_id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                ..SceneItemProvenance::default()
            },
        })
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}
