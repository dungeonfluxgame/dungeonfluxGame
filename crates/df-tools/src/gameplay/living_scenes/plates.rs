//! Scene plates: one composed film still for every story scene, made just in time (PLAY-V09).
//!
//! A plate is the whole scene as one photographic picture: the heroes seen from behind, the
//! speaker the lit focal point, extras and foreground objects, one light. It replaces the painted
//! backdrop with separate cut-outs on the shared display (the display keeps those cut-outs only
//! while a plate is pending or has failed).
//!
//! The director here watches the display's view. The moment a scene becomes current it requests
//! that scene's plate, and the plates of the scenes that can follow it (the beat script knows the
//! successors), so a successor is usually ready when the party arrives. Nothing waits for a
//! plate: a pending or failed plate leaves the scene exactly as it was, and a request that cannot
//! be queued is dropped and made again by a later view. When the mood tracker settles on a lasting
//! face for the speaker, the current plate is edited (the experiments showed an edit keeps the
//! room and a fresh render re-stages it) instead of re-drawing the cut-out.
//!
//! The one worker task owns every render it starts (a `JoinSet`), ends with the last handle, and
//! reports each outcome to the plate book, never to the game.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use df_protocol::common as rpc;
use tokio::{sync::mpsc, task::JoinSet};

use super::{
    super::{
        beat_script::BeatKind,
        imagery::{
            ImageFailure, ImageKey, ImageKind, ImageRequest, Lookup, OutputSpec, PlaceFacts,
            PlateReferences, ReferenceImage, party_sheet, plate_moment, scene_plate,
        },
        pack::{self, Age, Gender, SceneKey},
        scene_mood::Face,
        shot::{self, PlateLayout, ShotError, ShotInput, ShotSpec, SpeakerInput},
    },
    Inner, WorkerGuard, locked, record,
};

/// Plates kept in the book; the oldest not in use are forgotten first.
const MAXIMUM_ENTRIES: usize = 32;
/// Requests that may wait for the worker; a further one is dropped and made again later.
const QUEUE_CAPACITY: usize = 16;
/// The most successor scenes whose plates are requested ahead of time.
const MAXIMUM_SUCCESSORS: usize = 3;
/// A plate for a scene that may come next waits this long before it is made, so the plate of the
/// scene on screen is always the first to reach the image service (the service runs two calls at
/// a time and serves them in the order they arrive).
const AHEAD_DELAY: Duration = Duration::from_secs(2);
/// How often the worker looks for a lasting face to edit into the current plate.
const POLL: Duration = Duration::from_secs(1);

/// What the plate director needs to know about one scene, from the pack.
#[derive(Clone, Debug)]
struct PlateScene {
    title: String,
    key: SceneKey,
    asset: String,
    place_id: String,
    place_kind: String,
    place: Option<PlaceFacts>,
    story_text: String,
    speaker: Option<Speaker>,
    others: Vec<String>,
}

#[derive(Clone, Debug)]
struct Speaker {
    id: String,
    sprite: String,
    gender: Option<Gender>,
    age: Option<Age>,
}

impl PlateScene {
    /// `None` for a scene that never gets a plate: the lobby and a fight.
    fn of(scene: &pack::Scene) -> Option<Self> {
        if matches!(scene.key, SceneKey::Room | SceneKey::Combat) {
            return None;
        }
        let setting = scene.setting.as_ref()?;
        let place_kind = serde_json::to_value(setting.kind)
            .ok()?
            .as_str()
            .map(str::to_owned)?;
        let speaker = scene
            .cast
            .iter()
            .find(|member| !scene.speaker.is_empty() && member.name == scene.speaker);
        let others = scene
            .cast
            .iter()
            .filter(|member| speaker.is_none_or(|speaker| speaker.id != member.id))
            .map(|member| member.sprite.clone())
            .collect();
        Some(Self {
            title: scene.title.clone(),
            key: scene.key,
            asset: scene.asset.clone(),
            place_id: setting.place_id.clone(),
            place_kind,
            place: PlaceFacts::new(&setting.name, &setting.short),
            story_text: format!("{} {}", scene.narration, scene.dialogue),
            speaker: speaker.map(|member| Speaker {
                id: member.id.clone(),
                sprite: member.sprite.clone(),
                gender: member.gender,
                age: member.age,
            }),
            others,
        })
    }

    /// The plate of this scene for this party: the same scene and party always have one name.
    fn plate_id(&self, heroes: &[u8], faces: &[(u8, ImageKey)]) -> String {
        let party: String = heroes.iter().map(|art| art.to_string()).collect();
        // A hurt hero's health variant makes another picture, so it is part of the name.
        let hurt: String = faces
            .iter()
            .map(|(art, key)| format!("{art}{}", &key.as_str()[..8]))
            .collect();
        format!(
            "{}|{}|{:?}|{party}|{hurt}",
            self.title, self.place_id, self.key
        )
    }

    fn stage(&self, heroes: &[u8]) -> Result<ShotSpec, ShotError> {
        let others: Vec<&str> = self.others.iter().map(String::as_str).collect();
        shot::direct(&ShotInput {
            scene_key: self.key,
            place_id: &self.place_id,
            place_kind: &self.place_kind,
            title: &self.title,
            story_text: &self.story_text,
            speaker: self.speaker.as_ref().map(|speaker| SpeakerInput {
                sprite: &speaker.sprite,
                gender: speaker.gender,
                age: speaker.age,
            }),
            others: &others,
            heroes,
        })
    }
}

/// The pack scene a display view shows: matched by its painting and title, the way living scenes
/// match it.
fn locate(title: &str, asset: &str) -> Option<pack::Scene> {
    pack::with_content(|content| {
        let wanted = asset.trim_start_matches('/');
        let matches = |scene: &pack::Scene, asset: &str| {
            asset.trim_start_matches('/') == wanted && scene.title == title
        };
        let overrides = pack::active().map(|loaded| loaded.script_scene_assets.clone());
        content
            .scenes
            .iter()
            .find(|scene| matches(scene, &scene.asset))
            .or_else(|| {
                content.beat_script.as_ref().and_then(|script| {
                    script.scenes.iter().find_map(|(id, scene)| {
                        let asset = overrides
                            .as_ref()
                            .and_then(|assets| assets.get(id))
                            .unwrap_or(&scene.asset);
                        matches(scene, asset).then_some(scene)
                    })
                })
            })
            .cloned()
    })
}

/// The scenes that can follow `current`, from the beat script's transitions (fights are not
/// plated, so they are skipped). Without a script the legacy journey's order applies.
fn successors(current: &pack::Scene) -> Vec<pack::Scene> {
    pack::with_content(|content| {
        let mut found: Vec<pack::Scene> = Vec::new();
        let add = |scene: pack::Scene, found: &mut Vec<pack::Scene>| {
            if scene.title != current.title && !found.iter().any(|known| known.title == scene.title)
            {
                found.push(scene);
            }
        };
        let Some(script) = &content.beat_script else {
            let next: &[SceneKey] = match current.key {
                SceneKey::Opening => &[SceneKey::DialogueSeal, SceneKey::DialogueEscort],
                SceneKey::Victory => &[SceneKey::Inn],
                _ => &[],
            };
            for key in next {
                if let Some(scene) = content.scenes.iter().find(|scene| scene.key == *key) {
                    add(scene.clone(), &mut found);
                }
            }
            return found;
        };
        let scene_of = |scene: &pack::SceneKey, variant: &Option<String>| match variant {
            Some(variant) => script.scenes.get(variant).cloned(),
            None => content
                .scenes
                .iter()
                .find(|candidate| candidate.key == *scene)
                .cloned(),
        };
        for beat in &script.beats {
            let Some(scene) = scene_of(&beat.scene, &beat.scene_variant) else {
                continue;
            };
            if scene.title != current.title || scene.key != current.key {
                continue;
            }
            for transition in &beat.transitions {
                let Some(target) = script
                    .beats
                    .iter()
                    .find(|next| next.id == transition.target)
                else {
                    continue;
                };
                if target.kind == BeatKind::Fight {
                    continue;
                }
                if let Some(next) = scene_of(&target.scene, &target.scene_variant) {
                    add(next, &mut found);
                }
            }
        }
        found
    })
}

/// What one display view says about plates.
struct View {
    /// Names the scene the same way living scenes do: the title and the authored painting.
    scene_key: String,
    title: String,
    asset: String,
    in_combat: bool,
    /// The art numbers of the heroes present, in party order.
    heroes: Vec<u8>,
    /// Their names, to find a hero's health variant.
    hero_names: Vec<(u8, String)>,
}

fn scene_identity(display_asset: &str, scene: &rpc::GameplayScene) -> (String, String) {
    let asset = if scene.scene_asset.is_empty() {
        display_asset.to_owned()
    } else {
        scene.scene_asset.clone()
    };
    (format!("{}|{}", scene.title, asset), asset)
}

fn view_of(message: &rpc::ViewMessage) -> Option<View> {
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        return None;
    };
    let scene = display.scene.as_ref()?;
    let journey = display.journey.as_ref();
    let in_combat = journey.and_then(|journey| rpc::JourneyPhase::try_from(journey.phase).ok())
        == Some(rpc::JourneyPhase::Combat);
    let hero_names: Vec<(u8, String)> = journey
        .map(|journey| {
            journey
                .party
                .iter()
                .enumerate()
                .take(shot::MAXIMUM_HEROES)
                .filter(|(_, member)| member.character_ready)
                .filter_map(|(index, member)| {
                    Some((u8::try_from(index + 1).ok()?, member.name.clone()))
                })
                .collect()
        })
        .unwrap_or_default();
    let heroes = hero_names.iter().map(|(art, _)| *art).collect();
    let (scene_key, asset) = scene_identity(&display.scene_asset, scene);
    Some(View {
        scene_key,
        title: scene.title.clone(),
        asset,
        in_combat,
        heroes,
        hero_names,
    })
}

#[derive(Clone, Debug)]
enum Entry {
    /// Asked for; the render is queued or flying.
    Pending,
    Ready {
        key: ImageKey,
        layout: Option<PlateLayout>,
    },
    Failed,
    /// The scene cannot be staged (a creature speaks, nobody is present): no plate, no failure.
    NotPlanned,
}

#[derive(Clone, Debug)]
enum Edit {
    Idle,
    Pending(Face),
    Ready { face: Face, key: ImageKey },
    Failed(Face),
}

/// The scene on screen and what is known about its plate.
struct Current {
    scene_key: String,
    id: Option<String>,
    scene: Option<PlateScene>,
    heroes: Vec<u8>,
    in_combat: bool,
}

struct Book {
    current: Option<Current>,
    entries: BTreeMap<String, Entry>,
    order: VecDeque<String>,
    edit: Edit,
}

impl Book {
    fn new() -> Self {
        Self {
            current: None,
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            edit: Edit::Idle,
        }
    }

    fn insert(&mut self, id: String, entry: Entry) {
        if self.entries.insert(id.clone(), entry).is_none() {
            self.order.push_back(id);
        }
        let in_use = self.current.as_ref().and_then(|current| current.id.clone());
        while self.order.len() > MAXIMUM_ENTRIES {
            let Some(position) = self
                .order
                .iter()
                .position(|known| Some(known) != in_use.as_ref())
            else {
                break;
            };
            if let Some(forgotten) = self.order.remove(position) {
                self.entries.remove(&forgotten);
            }
        }
    }

    fn current_entry(&self) -> Option<&Entry> {
        let id = self.current.as_ref()?.id.as_ref()?;
        self.entries.get(id)
    }

    /// The plate the display should show now, with its layout: the edited plate while an edit of
    /// the wanted face is ready, otherwise the first one.
    fn shown(&self) -> Option<(ImageKey, Option<PlateLayout>)> {
        let Some(Entry::Ready { key, layout }) = self.current_entry() else {
            return None;
        };
        match &self.edit {
            Edit::Ready { key: edited, .. } => Some((edited.clone(), *layout)),
            _ => Some((key.clone(), *layout)),
        }
    }

    fn covers(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| !current.in_combat)
            && matches!(self.current_entry(), Some(Entry::Ready { .. }))
    }
}

/// One request for the worker.
enum Job {
    Fresh {
        id: String,
        scene: PlateScene,
        spec: Box<ShotSpec>,
        heroes: Vec<u8>,
        /// The health variants (already made and cached) of hurt heroes, by art number.
        faces: Vec<(u8, ImageKey)>,
        /// A plate for a scene that may come next, not the one on screen.
        ahead: bool,
    },
    Edit {
        scene_key: String,
        scene: PlateScene,
        base: ImageKey,
        face: Face,
    },
}

enum Done {
    Fresh {
        id: String,
        elapsed: Duration,
        result: Result<(ImageKey, Option<PlateLayout>), ImageFailure>,
    },
    Edit {
        scene_key: String,
        speaker_id: Option<String>,
        face: Face,
        elapsed: Duration,
        result: Result<ImageKey, ImageFailure>,
    },
}

struct Shared {
    inner: Arc<Inner>,
    book: Mutex<Book>,
    jobs: mpsc::Sender<Job>,
}

/// The plate director: a cheap handle owned by the living-scenes handle.
#[derive(Clone)]
pub(super) struct Plates {
    shared: Arc<Shared>,
    _worker: Arc<WorkerGuard>,
}

impl Plates {
    /// Starts the director and its worker task. Must be called inside the runtime.
    pub(super) fn start(inner: Arc<Inner>) -> Self {
        let (jobs, queue) = mpsc::channel(QUEUE_CAPACITY);
        let shared = Arc::new(Shared {
            inner,
            book: Mutex::new(Book::new()),
            jobs,
        });
        let worker = tokio::spawn(Arc::clone(&shared).run(queue));
        Self {
            shared,
            _worker: Arc::new(WorkerGuard(worker)),
        }
    }

    /// Notes a display view: a new scene gets its plate and its successors' plates requested.
    pub(super) fn observe(&self, message: &rpc::ViewMessage) {
        let Some(view) = view_of(message) else {
            return;
        };
        self.shared.observe(&view);
    }

    /// Adds the plate to the scene of a view about to be sent (to the display or a phone).
    pub(super) fn apply_to_message(&self, message: &mut rpc::ViewMessage) {
        let (scene, display_asset) = match message.audience.as_mut() {
            Some(rpc::view_message::Audience::Display(display)) => {
                (display.scene.as_mut(), display.scene_asset.clone())
            }
            Some(rpc::view_message::Audience::Player(player)) => {
                (player.scene.as_mut(), String::new())
            }
            None => return,
        };
        let Some(scene) = scene else {
            return;
        };
        let (scene_key, _) = scene_identity(&display_asset, scene);
        self.shared.apply(&scene_key, scene);
    }
}

fn wire_layout(layout: &PlateLayout) -> rpc::PlateLayout {
    rpc::PlateLayout {
        bubble_side: match layout.bubble_side {
            shot::Side::Left => rpc::PlateSide::Left,
            shot::Side::Right => rpc::PlateSide::Right,
        } as i32,
        head_x_permille: u32::from(layout.head_x_permille),
        head_y_permille: u32::from(layout.head_y_permille),
        head_width_permille: u32::from(layout.head_width_permille),
        head_height_permille: u32::from(layout.head_height_permille),
    }
}

impl Shared {
    fn wake(&self) {
        self.inner.wake.send_modify(|count| *count += 1);
    }

    fn observe(&self, view: &View) {
        let changed = locked(&self.book).current.as_ref().is_none_or(|current| {
            current.scene_key != view.scene_key || current.heroes != view.heroes
        });
        let (located, next) = if changed {
            let located = locate(&view.title, &view.asset);
            let next = located.as_ref().map(successors).unwrap_or_default();
            (located, next)
        } else {
            (None, Vec::new())
        };
        self.observe_with(view, located, next);
    }

    /// [`Self::observe`] with the pack's scene and its successors given, so tests need no pack.
    fn observe_with(&self, view: &View, located: Option<pack::Scene>, next: Vec<pack::Scene>) {
        let mut book = locked(&self.book);
        let scene_changed = book
            .current
            .as_ref()
            .is_none_or(|current| current.scene_key != view.scene_key);
        let party_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.heroes != view.heroes);
        let combat_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.in_combat != view.in_combat);
        let mut woke = false;
        if scene_changed || party_changed {
            let scene = located.as_ref().and_then(PlateScene::of);
            let faces = self.inner.hero_face_keys(&view.hero_names);
            let id = scene
                .as_ref()
                .map(|scene| scene.plate_id(&view.heroes, &faces));
            if scene_changed {
                book.edit = Edit::Idle;
                // A plate that failed is asked for again once when its scene comes back.
                if let Some(id) = &id
                    && matches!(book.entries.get(id), Some(Entry::Failed))
                {
                    book.entries.remove(id);
                    book.order.retain(|known| known != id);
                }
            }
            book.current = Some(Current {
                scene_key: view.scene_key.clone(),
                id,
                scene: scene.clone(),
                heroes: view.heroes.clone(),
                in_combat: view.in_combat,
            });
            if let Some(scene) = scene
                && !view.in_combat
            {
                self.ensure(&mut book, &scene, &view.heroes, &faces, false);
                for next in next
                    .iter()
                    .take(MAXIMUM_SUCCESSORS)
                    .filter_map(PlateScene::of)
                {
                    self.ensure(&mut book, &next, &view.heroes, &faces, true);
                }
            }
            woke = true;
        } else if combat_changed && let Some(current) = book.current.as_mut() {
            current.in_combat = view.in_combat;
            woke = true;
        }
        let covers = book.covers();
        drop(book);
        self.inner.set_plate_covers(covers);
        if woke {
            self.wake();
        }
    }

    /// Asks for the plate of `scene` for this party unless the book already knows it.
    fn ensure(
        &self,
        book: &mut Book,
        scene: &PlateScene,
        heroes: &[u8],
        faces: &[(u8, ImageKey)],
        ahead: bool,
    ) {
        let id = scene.plate_id(heroes, faces);
        if book.entries.contains_key(&id) {
            return;
        }
        let spec = match scene.stage(heroes) {
            Ok(spec) => spec,
            Err(_) => {
                book.insert(id, Entry::NotPlanned);
                return;
            }
        };
        let job = Job::Fresh {
            id: id.clone(),
            scene: scene.clone(),
            spec: Box::new(spec),
            heroes: heroes.to_vec(),
            faces: faces.to_vec(),
            ahead,
        };
        // A full queue (the worker is busy or gone) drops the request: the next view asks again,
        // and nothing waits.
        if self.jobs.try_send(job).is_ok() {
            record(
                "gameplay.living_scene.plate_requested",
                &[("scene.kind", "plate".to_owned())],
            );
            book.insert(id, Entry::Pending);
        }
    }

    fn apply(&self, scene_key: &str, scene: &mut rpc::GameplayScene) {
        let book = locked(&self.book);
        let Some(current) = &book.current else {
            return;
        };
        if current.scene_key != scene_key || current.in_combat {
            return;
        }
        match book.current_entry() {
            Some(Entry::Pending) => {
                scene.plate_state = rpc::PlateState::Pending as i32;
            }
            Some(Entry::Ready { .. }) => {
                if let Some((key, layout)) = book.shown() {
                    scene.live_plate = key.path();
                    scene.plate_layout = layout.as_ref().map(wire_layout);
                    scene.plate_state = rpc::PlateState::Ready as i32;
                }
            }
            Some(Entry::Failed) => {
                scene.plate_state = rpc::PlateState::Failed as i32;
            }
            Some(Entry::NotPlanned) | None => {}
        }
    }

    /// The worker: takes requests, owns every render task, and every tick looks for a lasting face
    /// to edit into the current plate.
    async fn run(self: Arc<Self>, mut queue: mpsc::Receiver<Job>) {
        let mut ticker = tokio::time::interval(POLL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut tasks: JoinSet<Done> = JoinSet::new();
        loop {
            tokio::select! {
                Some(job) = queue.recv() => self.spawn(&mut tasks, job),
                Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                    // A render task that panicked or was cancelled has nothing to report.
                    if let Ok(done) = done {
                        self.finish(done);
                    }
                }
                _ = ticker.tick() => self.poll_face(),
            }
        }
    }

    fn spawn(self: &Arc<Self>, tasks: &mut JoinSet<Done>, job: Job) {
        let shared = Arc::clone(self);
        match job {
            Job::Fresh {
                id,
                scene,
                spec,
                heroes,
                faces,
                ahead,
            } => {
                tasks.spawn(async move {
                    if ahead {
                        tokio::time::sleep(AHEAD_DELAY).await;
                    }
                    let started = tokio::time::Instant::now();
                    let result = shared.render_fresh(&scene, &spec, &heroes, &faces).await;
                    Done::Fresh {
                        id,
                        elapsed: started.elapsed(),
                        result,
                    }
                });
            }
            Job::Edit {
                scene_key,
                scene,
                base,
                face,
            } => {
                tasks.spawn(async move {
                    let started = tokio::time::Instant::now();
                    let result = shared.render_edit(&scene, &base, face).await;
                    Done::Edit {
                        scene_key,
                        speaker_id: scene.speaker.map(|speaker| speaker.id),
                        face,
                        elapsed: started.elapsed(),
                        result,
                    }
                });
            }
        }
    }

    /// The art that shows a speaker in a plate: the full-length portrait where there is one.
    async fn speaker_art(&self, sprite: &str) -> Option<ReferenceImage> {
        let stem = sprite.rsplit('/').next().unwrap_or(sprite);
        let name = stem.strip_prefix("sprite-")?;
        for candidate in [
            format!("portrait-{name}-full"),
            format!("figure-{name}"),
            format!("portrait-{name}"),
            stem.to_owned(),
        ] {
            if let Ok(art) = self.inner.load(candidate).await {
                return Some(art);
            }
        }
        None
    }

    async fn render_fresh(
        &self,
        scene: &PlateScene,
        spec: &ShotSpec,
        heroes: &[u8],
        faces: &[(u8, ImageKey)],
    ) -> Result<(ImageKey, Option<PlateLayout>), ImageFailure> {
        let mut references = Vec::new();
        let mut sorted = heroes.to_vec();
        sorted.sort_unstable();
        if !sorted.is_empty() {
            let mut figures = Vec::with_capacity(sorted.len());
            for art in &sorted {
                // A hurt hero stands in the plate as the cut-out stage shows them: their health
                // variant, taken only from what is already made and cached.
                let variant = match faces.iter().find(|(known, _)| known == art) {
                    Some((_, key)) => match self.inner.imagery.image(key.as_str()).await {
                        Lookup::Image { bytes, format } => Some(ReferenceImage::new(format, bytes)),
                        Lookup::Unregistered | Lookup::NotReady => None,
                    },
                    None => None,
                };
                figures.push(match variant {
                    Some(variant) => variant,
                    None => self.inner.load(format!("figure-hero-{art}")).await?,
                });
            }
            let sheet = tokio::task::spawn_blocking(move || party_sheet(&figures))
                .await
                .map_err(|_| ImageFailure::Interrupted)?
                .map_err(|_| ImageFailure::InvalidRequest)?;
            references.push(sheet);
        }
        let speaker = match &scene.speaker {
            Some(speaker) => self.speaker_art(&speaker.sprite).await,
            None => None,
        };
        let has_speaker_art = speaker.is_some();
        references.extend(speaker);
        let place = self.inner.load(scene.asset.clone()).await.ok();
        let has_place = place.is_some();
        references.extend(place);
        let prompt = scene_plate(
            self.inner.rating,
            spec,
            scene.place.as_ref(),
            PlateReferences {
                party_sheet: !sorted.is_empty(),
                speaker: has_speaker_art,
                place: has_place,
            },
        )
        .map_err(|_| ImageFailure::InvalidRequest)?;
        let key = self
            .inner
            .imagery
            .render(ImageRequest {
                kind: ImageKind::ScenePlate,
                references,
                prompt,
                output: OutputSpec::PLATE,
            })
            .await?;
        Ok((key, spec.speaker.as_ref().map(|speaker| speaker.layout)))
    }

    async fn render_edit(
        &self,
        scene: &PlateScene,
        base: &ImageKey,
        face: Face,
    ) -> Result<ImageKey, ImageFailure> {
        let Lookup::Image { bytes, format } = self.inner.imagery.image(base.as_str()).await else {
            return Err(ImageFailure::InvalidRequest);
        };
        let mut references = vec![ReferenceImage::new(format, bytes)];
        let speaker = match &scene.speaker {
            Some(speaker) => self.speaker_art(&speaker.sprite).await,
            None => None,
        };
        let has_speaker_art = speaker.is_some();
        references.extend(speaker);
        let prompt = plate_moment(
            self.inner.rating,
            face.expression,
            face.tier,
            has_speaker_art,
        )
        .map_err(|_| ImageFailure::InvalidRequest)?;
        self.inner
            .imagery
            .render(ImageRequest {
                kind: ImageKind::ScenePlateEdit,
                references,
                prompt,
                output: OutputSpec::PLATE,
            })
            .await
    }

    /// Records a finished render. Every generation is logged with its time; the run manifest
    /// holds its cost.
    fn finish(&self, done: Done) {
        let (kind, elapsed, outcome) = match &done {
            Done::Fresh {
                elapsed, result, ..
            } => ("plate", *elapsed, result.as_ref().map(|_| ())),
            Done::Edit {
                elapsed, result, ..
            } => ("plate_edit", *elapsed, result.as_ref().map(|_| ())),
        };
        record(
            "gameplay.living_scene.render_finished",
            &[
                ("scene.kind", kind.to_owned()),
                (
                    "scene.outcome",
                    outcome
                        .map_or_else(|failure| failure.label(), |()| "ready")
                        .to_owned(),
                ),
                ("scene.elapsed_ms", elapsed.as_millis().to_string()),
            ],
        );
        let mut book = locked(&self.book);
        match done {
            Done::Fresh { id, result, .. } => {
                let entry = match result {
                    Ok((key, layout)) => Entry::Ready { key, layout },
                    Err(_) => Entry::Failed,
                };
                // A plate for a scene the book has already forgotten is not remembered.
                if book.entries.contains_key(&id) {
                    book.entries.insert(id, entry);
                }
            }
            Done::Edit {
                scene_key,
                speaker_id,
                face,
                result,
                ..
            } => {
                let current = book
                    .current
                    .as_ref()
                    .is_some_and(|current| current.scene_key == scene_key);
                if current && matches!(&book.edit, Edit::Pending(pending) if *pending == face) {
                    match result {
                        Ok(key) => {
                            book.edit = Edit::Ready { face, key };
                            if let Some(speaker_id) = speaker_id {
                                self.inner.note_plate_face_shown(&speaker_id, face);
                            }
                        }
                        Err(_) => book.edit = Edit::Failed(face),
                    }
                }
            }
        }
        let covers = book.covers();
        drop(book);
        self.inner.set_plate_covers(covers);
        self.wake();
    }

    /// One look at the speaker's wanted face: a lasting change of it edits the current plate, and
    /// a return to the neutral face returns to the first plate.
    fn poll_face(&self) {
        let mut book = locked(&self.book);
        let Some(current) = &book.current else {
            return;
        };
        let (Some(scene), false) = (current.scene.clone(), current.in_combat) else {
            return;
        };
        let Some(speaker) = scene.speaker.clone() else {
            return;
        };
        let Some(Entry::Ready { key: base, .. }) = book.current_entry().cloned() else {
            return;
        };
        let scene_key = current.scene_key.clone();
        let wanted = self
            .inner
            .plate_wanted_face(&speaker.id)
            .filter(|face| *face != Face::AUTHORED);
        match (wanted, book.edit.clone()) {
            (None, Edit::Idle) => {}
            (None, _) => {
                let had_edit = matches!(book.edit, Edit::Ready { .. });
                book.edit = Edit::Idle;
                drop(book);
                if had_edit {
                    self.inner
                        .note_plate_face_shown(&speaker.id, Face::AUTHORED);
                    self.wake();
                }
            }
            (Some(face), Edit::Pending(pending)) if pending == face => {}
            (Some(face), Edit::Ready { face: shown, .. } | Edit::Failed(shown))
                if shown == face => {}
            (Some(face), _) => {
                let job = Job::Edit {
                    scene_key,
                    scene,
                    base,
                    face,
                };
                if self.jobs.try_send(job).is_ok() {
                    book.edit = Edit::Pending(face);
                    record(
                        "gameplay.living_scene.plate_edit_requested",
                        &[
                            ("scene.expression", face.expression.label().to_owned()),
                            ("scene.tier", face.tier.label().to_owned()),
                        ],
                    );
                }
            }
        }
    }
}

impl Inner {
    /// The health variants the health pipeline has made for these heroes, by art number.
    fn hero_face_keys(&self, heroes: &[(u8, String)]) -> Vec<(u8, ImageKey)> {
        let state = locked(&self.state);
        heroes
            .iter()
            .filter_map(|(art, name)| Some((*art, state.hero_faces.get(name)?.clone())))
            .collect()
    }

    /// Whether the plate on screen covers the cut-outs: their expression and the re-lit
    /// backdrop are then never drawn (nobody would see them).
    fn set_plate_covers(&self, covers: bool) {
        locked(&self.state).plate_covers = covers;
    }

    /// The face the mood last asked for on this character, if any.
    fn plate_wanted_face(&self, id: &str) -> Option<Face> {
        locked(&self.state).plate_wants.get(id).copied()
    }

    /// The plate now shows `face`, so the mood tracker owes nothing more for it.
    fn note_plate_face_shown(&self, id: &str, face: Face) {
        locked(&self.state).tracker.mark_face_shown(id, face);
    }
}

#[cfg(test)]
#[path = "plates_tests.rs"]
mod tests;
