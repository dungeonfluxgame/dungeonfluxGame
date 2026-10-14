//! Settled visible changes share one edit; object cards share one request per object state.

use super::*;
use crate::gameplay::{
    adventure::scene_model::{SceneNow, SceneThing},
    imagery::{item_picture, state_edit},
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
    last_started: Option<Duration>,
    pub(super) shown: Option<(ImageKey, Option<PlatePlacement>)>,
    pub(super) baked_revision: Option<u64>,
    ready: Option<(ImageKey, u64)>,
    pub(super) items: BTreeMap<String, Item>,
    pub(super) latest_items: BTreeMap<String, String>,
}

pub(super) struct Pending {
    snapshot: SceneNow,
    reason: String,
    state: String,
    revision: u64,
    changed_at: Duration,
    graph: Option<crate::gameplay::shot::scene_graph::SceneGraph>,
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

pub(super) enum Job {
    Bake {
        scene: String,
        pending: Pending,
        base: ImageKey,
    },
    Item {
        id: String,
        thing: SceneThing,
        detail: Option<String>,
        base: Option<ImageKey>,
    },
}

pub(super) enum Done {
    Bake {
        scene: String,
        state: String,
        revision: u64,
        result: Result<ImageKey, ImageFailure>,
    },
    Item {
        id: String,
        result: Result<ImageKey, ImageFailure>,
    },
}

pub(super) fn observe(book: &mut super::Book, view: &View, scene: &PlateScene, now: Duration) {
    let Some(snapshot) = &scene.snapshot else {
        return;
    };
    let Ok(state) = snapshot.model.picture_state() else {
        record(
            "gameplay.living_scene.invalid_snapshot",
            &[("scene", view.scene_key.clone())],
        );
        return;
    };
    let images = &mut book.state_images;
    if images.scene != view.scene_key {
        images.scene = view.scene_key.clone();
        images.observed = state;
        images.observed_graph = view.graph_revision;
        images.shown = None;
        images.pending = None;
        images.flying = false;
        images.baked_revision = None;
        images.ready = None;
        return;
    }
    if images.observed == state && images.observed_graph == view.graph_revision {
        return;
    }
    images.observed = state.clone();
    images.observed_graph = view.graph_revision;
    images.ready = None;
    let reason = snapshot
        .changes
        .last()
        .cloned()
        .unwrap_or_else(|| "visible scene state changed".into());
    let reason = images.pending.as_ref().map_or(reason.clone(), |pending| {
        format!("{}; {reason}", pending.reason)
    });
    images.pending = Some(Pending {
        snapshot: snapshot.clone(),
        reason,
        state,
        revision: view.graph_revision,
        changed_at: now,
        graph: None,
    });
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
        if let Some(snapshot) = current.and_then(|scene| scene.snapshot.as_ref()) {
            scene.scene_contents_json = snapshot.model.public_contents().unwrap_or_else(|error| {
                record(
                    "gameplay.living_scene.contents_failed",
                    &[("reason", error.to_string())],
                );
                String::new()
            });
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
        if current.in_combat || current.in_roll || book.state_images.flying {
            return;
        }
        let scene = current.scene_key.clone();
        if let Some((key, revision)) = book.state_images.ready.take() {
            let layout = book.shown().and_then(|shown| shown.1);
            book.edit = Edit::Idle;
            book.state_images.shown = Some((key, layout));
            book.state_images.baked_revision = Some(revision);
            self.wake();
        }
        let Some(pending) = &book.state_images.pending else {
            return;
        };
        if now.saturating_sub(pending.changed_at) < SETTLE
            || book
                .state_images
                .last_started
                .is_some_and(|last| now.saturating_sub(last) < BAKE_INTERVAL)
        {
            return;
        }
        let base = book.shown().map(|shown| shown.0);
        let Some(pending) = book.state_images.pending.take() else {
            return;
        };
        let mut pending = pending;
        pending.graph = locked(&self.inner.state)
            .scene_graph
            .clone()
            .filter(|graph| graph.revision == pending.revision);
        let request_line = format!(
            "IMAGERY request kind=scene_plate_edit reason={} state={}{}",
            pending.reason,
            pending.revision,
            if base.is_none() {
                " unavailable=current-still"
            } else {
                ""
            },
        );
        let Some(base) = base else {
            crate::gameplay::actor::record_audit_line("gameplay.imagery.request", request_line);
            // A visible change before the first still landed needs a fresh current-state plate.
            if self.inner.imagery.is_on() {
                let fresh = book.current.as_ref().and_then(|current| {
                    Some((
                        current.scene.clone()?,
                        current.heroes.clone(),
                        current.faces.clone(),
                        current.graph_revision,
                    ))
                });
                if let Some((scene, heroes, faces, revision)) = fresh {
                    let id = scene.plate_id(&heroes, &faces, revision);
                    if let Some(current) = book.current.as_mut() {
                        current.id = Some(id);
                    }
                    self.ensure(&mut book, &scene, &heroes, &faces, revision, false);
                }
            }
            return;
        };
        let job = super::Job::State(Job::Bake {
            scene,
            pending,
            base,
        });
        match self.jobs.try_send(job) {
            Ok(()) => {
                crate::gameplay::actor::record_audit_line("gameplay.imagery.request", request_line);
                book.state_images.flying = true;
                book.state_images.last_started = Some(now);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(super::Job::State(Job::Bake {
                pending,
                ..
            }))) => {
                book.state_images.pending = Some(pending);
            }
            Err(_) => record("gameplay.living_scene.worker_closed", &[]),
        }
    }

    pub(super) async fn render_state_image(&self, job: Job) -> Done {
        match job {
            Job::Bake {
                scene,
                pending,
                base,
            } => {
                let result = async {
                    let Lookup::Image { bytes, format } =
                        self.inner.imagery.image(base.as_str()).await
                    else {
                        return Err(ImageFailure::InvalidRequest);
                    };
                    let prompt = match &pending.graph {
                        Some(graph) => crate::gameplay::imagery::graph_edit(
                            self.inner.rating,
                            graph,
                            &pending.reason,
                        )?,
                        None => {
                            state_edit(self.inner.rating, &pending.snapshot.model, &pending.reason)?
                        }
                    };
                    self.inner
                        .imagery
                        .render(ImageRequest {
                            kind: ImageKind::ScenePlateEdit,
                            references: vec![ReferenceImage::new(format, bytes)],
                            prompt,
                            output: OutputSpec::PLATE,
                        })
                        .await
                }
                .await;
                Done::Bake {
                    scene,
                    state: pending.state,
                    revision: pending.revision,
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
                Done::Item { id, result }
            }
        }
    }

    pub(super) fn finish_state_image(&self, done: Done) {
        let mut book = locked(&self.book);
        match done {
            Done::Bake {
                scene,
                state,
                revision,
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
                                {
                                    book.state_images.ready = Some((key, revision));
                                } else {
                                    let layout = book.shown().and_then(|shown| shown.1);
                                    book.edit = Edit::Idle;
                                    book.state_images.shown = Some((key, layout));
                                    book.state_images.baked_revision = Some(revision);
                                }
                            }
                            Err(failure) => record(
                                "gameplay.living_scene.state_edit_failed",
                                &[("reason", failure.label().into())],
                            ),
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
