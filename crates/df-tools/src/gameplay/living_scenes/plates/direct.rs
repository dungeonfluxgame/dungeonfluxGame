//! Directed plates share the ordinary request builder, budget, cache and image worker.

use super::*;
use serde_json::{Value, json};
use tokio::sync::oneshot;

type RenderReply = oneshot::Sender<Result<Value, ImageFailure>>;

#[derive(Default)]
pub(super) struct Book {
    generation: u64,
    pub(super) shown: Option<(ImageKey, Option<PlatePlacement>)>,
    pub(super) keep: Option<String>,
    pub(super) description: Option<String>,
    pub(super) pending: bool,
    results: BTreeMap<String, Value>,
    order: VecDeque<String>,
}

impl Book {
    pub(super) fn reset(&mut self) {
        self.generation += 1;
        self.shown = None;
        self.keep = None;
        self.description = None;
        self.pending = false;
    }

    fn remember(&mut self, key: &ImageKey, value: Value) {
        if self
            .results
            .insert(key.as_str().to_owned(), value)
            .is_none()
        {
            self.order.push_back(key.as_str().to_owned());
        }
        while self.order.len() > MAXIMUM_ENTRIES {
            if let Some(key) = self.order.pop_front() {
                self.results.remove(&key);
            }
        }
    }
}

pub(super) struct Work {
    generation: u64,
    scene_key: String,
    pub(super) scene: PlateScene,
    pub(super) spec: ShotSpec,
    pub(super) heroes: Vec<u8>,
    pub(super) faces: Vec<(u8, ImageKey)>,
    pub(super) text: String,
    keep: bool,
    reply: RenderReply,
}

impl Plates {
    /// Called by the session owner only. The queue is bounded and no provider work blocks it.
    pub(in crate::gameplay) fn direct(
        &self,
        text: String,
        keep: bool,
    ) -> Result<oneshot::Receiver<Result<Value, ImageFailure>>, &'static str> {
        if !self.shared.inner.imagery.is_on() {
            return Err("imagery is off");
        }
        if self.shared.inner.imagery.budget_remaining() == 0 {
            return Err("image budget is spent");
        }
        let mut book = locked(&self.shared.book);
        let current = book.current.as_ref().ok_or("no current scene")?;
        if current.in_combat {
            return Err("the game is in a fight on the 3D battlefield; scene plates are not shown");
        }
        let scene = current
            .scene
            .clone()
            .ok_or("the current scene has no scene plate")?;
        let spec = scene
            .stage(&current.heroes)
            .map_err(|_| "the current scene cannot be staged")?;
        let (reply, receive) = oneshot::channel();
        let generation = book.direct.generation + 1;
        let work = Work {
            generation,
            scene_key: current.scene_key.clone(),
            scene,
            spec,
            heroes: current.heroes.clone(),
            faces: current.faces.clone(),
            text: text.clone(),
            keep,
            reply,
        };
        self.shared
            .jobs
            .try_send(Job::Directed(Box::new(work)))
            .map_err(|_| "plate render queue is full or stopped")?;
        book.direct.generation = generation;
        book.direct.keep = keep.then_some(text.clone());
        book.direct.description = Some(text);
        book.direct.pending = true;
        book.edit = Edit::Idle;
        Ok(receive)
    }

    pub(in crate::gameplay) fn clear_direction(&self) {
        let mut book = locked(&self.shared.book);
        book.direct.reset();
        book.edit = Edit::Idle;
        drop(book);
        self.shared.wake();
    }

    pub(in crate::gameplay) fn direct_status(&self, key: &str) -> Option<Value> {
        locked(&self.shared.book).direct.results.get(key).cloned()
    }
}

impl Shared {
    /// Bypass the refresh interval and scene entrance delay. Only roll quiet remains.
    async fn wait_for_roll(&self, scene_key: &str, generation: u64) -> Result<(), ImageFailure> {
        loop {
            {
                let book = locked(&self.book);
                if book.direct.generation != generation
                    || book
                        .current
                        .as_ref()
                        .is_none_or(|current| current.scene_key != scene_key || current.in_combat)
                {
                    return Err(ImageFailure::Interrupted);
                }
            }
            let quiet_left = {
                let state = locked(&self.inner.state);
                state
                    .pacer
                    .last_action
                    .map(|action| {
                        self.inner
                            .timing
                            .roll_quiet
                            .saturating_sub(self.inner.now().saturating_sub(action))
                    })
                    .unwrap_or(Duration::ZERO)
            };
            if quiet_left.is_zero() {
                return Ok(());
            }
            tokio::time::sleep(quiet_left.min(POLL)).await;
        }
    }

    pub(super) async fn render_directed(&self, work: Work) -> Done {
        let Work {
            generation,
            scene_key,
            scene,
            spec,
            heroes,
            faces,
            text,
            keep,
            reply,
        } = work;
        let prepared = self
            .fresh_request(&scene, &spec, &heroes, &faces, Some(&text))
            .await;
        let (request, _locator_face) = match prepared {
            Ok(prepared) => prepared,
            Err(reason) => {
                let _ = reply.send(Err(reason));
                return Done::Directed {
                    generation,
                    scene_key,
                    key: None,
                    result: Err(reason),
                };
            }
        };
        let key = self.inner.imagery.key_for(&request);
        let value = json!({"kind": "plate", "scene": scene_key, "key": key.as_str(),
            "state": "pending", "keep": keep});
        locked(&self.book).direct.remember(&key, value.clone());
        let preview: String = text.chars().take(80).collect();
        println!(
            "DEV DIRECT kind=plate scene={} key={} keep={} {}",
            scene_key,
            &key.as_str()[..12],
            keep,
            preview
        );
        record(
            "gameplay.dev.direct",
            &[
                ("dev.kind", "plate".to_owned()),
                ("dev.scene", scene_key.clone()),
                ("dev.key", key.as_str()[..12].to_owned()),
                ("dev.keep", keep.to_string()),
                ("dev.direction", preview),
            ],
        );
        let _ = reply.send(Ok(value));
        let result = async {
            self.wait_for_roll(&scene_key, generation).await?;
            let rendered = self.inner.imagery.render(request).await?;
            self.wait_for_roll(&scene_key, generation).await?;
            let placement = match &spec.speaker {
                Some(speaker) => {
                    let actor_references = self.locator_references(&scene, &heroes).await?;
                    Some(
                        self.measured_or_predicted(
                            &rendered,
                            &spec,
                            speaker,
                            &actor_references,
                            &scene.locator_actors(),
                            scene.speaker.as_ref().map(|speaker| speaker.id.as_str()),
                        )
                        .await,
                    )
                }
                None => None,
            };
            Ok((rendered, placement))
        }
        .await;
        Done::Directed {
            generation,
            scene_key,
            key: Some(key),
            result,
        }
    }

    pub(super) fn finish_directed(
        &self,
        generation: u64,
        scene_key: &str,
        key: Option<ImageKey>,
        result: Result<(ImageKey, Option<PlatePlacement>), ImageFailure>,
    ) {
        let mut book = locked(&self.book);
        let current = book.direct.generation == generation
            && book
                .current
                .as_ref()
                .is_some_and(|current| current.scene_key == scene_key && !current.in_combat);
        if let Some(key) = key {
            let result = if current {
                result
            } else {
                Err(ImageFailure::Interrupted)
            };
            let (state, reason) = match &result {
                Ok(_) => ("ready", None),
                Err(reason) => ("failed", Some(reason.label())),
            };
            book.direct.remember(
                &key,
                json!({"kind": "plate", "scene": scene_key,
                "key": key.as_str(), "state": state, "reason": reason}),
            );
            if current && let Ok(shown) = result {
                book.state_images.shown = None;
                book.direct.shown = Some(shown);
                book.edit = Edit::Idle;
            }
        }
        if current {
            book.direct.pending = false;
        }
        let covers = book.covers();
        drop(book);
        self.inner.set_plate_covers(covers);
        self.wake();
    }
}
