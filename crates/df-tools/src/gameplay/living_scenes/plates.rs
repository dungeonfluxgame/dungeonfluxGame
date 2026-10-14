//! Scene plates: one composed film still for every story scene, made just in time (PLAY-V09).
//!
//! A plate is the whole scene as one photographic picture: the heroes seen from behind, the
//! speaker the lit focal point, extras and foreground objects, one light. It replaces the painted
//! backdrop with separate cut-outs on the shared display (the display draws those cut-outs only
//! when a plate has failed or none is planned; until the plate is ready the scene shows its
//! establishing shot, PLAY-V12).
//!
//! The director here watches the display's view. The moment a scene becomes current it requests
//! that scene's plate, and the plates of the scenes that can follow it (the beat script knows the
//! successors, a won fight included), so a successor is usually ready when the party arrives.
//! Nothing in the game waits for a plate or changes with it, and a
//! request that cannot be queued is dropped and made again by a later view. When the mood tracker
//! settles on a lasting face for the speaker, the current plate is edited (the experiments showed
//! an edit keeps the room and a fresh render re-stages it) instead of re-drawing the cut-out.
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
        beat_script::{self, Beat, BeatKind, BeatScript, BeatTransition, BeatTrigger},
        imagery::{
            ImageFailure, ImageKey, ImageKind, ImageRequest, Lookup, OutputSpec, PlaceFacts,
            PlateReferences, ReferenceImage, hero_locator_phrase, party_sheet, plate_moment,
            scene_plate,
        },
        pack::{self, Age, Gender, SceneKey},
        scene_mood::Face,
        shot::{
            self, HeroHead, PlateLayout, PlatePlacement, ShotError, ShotInput, ShotSpec,
            SpeakerInput,
        },
    },
    Inner, WorkerGuard, locked, record,
};

mod direct;

/// Plates kept in the book; the oldest not in use are forgotten first.
const MAXIMUM_ENTRIES: usize = 32;
/// Requests that may wait for the worker; a further one is dropped and made again later.
const QUEUE_CAPACITY: usize = 16;
/// The most successor scenes whose plates are requested ahead of time.
const MAXIMUM_SUCCESSORS: usize = 6;
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
    /// The adventure record's `exterior` for the place.
    exterior: bool,
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
            exterior: setting.exterior,
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
    fn plate_id(&self, heroes: &[u8], faces: &[(u8, ImageKey)], graph_revision: u64) -> String {
        let party: String = heroes.iter().map(|art| art.to_string()).collect();
        // A hurt hero's health variant makes another picture, so it is part of the name.
        let hurt: String = faces
            .iter()
            .map(|(art, key)| format!("{art}{}", &key.as_str()[..8]))
            .collect();
        format!(
            "{}|{}|{:?}|{party}|{hurt}|graph-{graph_revision}",
            self.title, self.place_id, self.key,
        )
    }

    fn stage(&self, heroes: &[u8]) -> Result<ShotSpec, ShotError> {
        let others: Vec<&str> = self.others.iter().map(String::as_str).collect();
        shot::direct(&ShotInput {
            scene_key: self.key,
            place_id: &self.place_id,
            place_kind: &self.place_kind,
            exterior: self.exterior,
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
pub(super) fn locate(title: &str, asset: &str) -> Option<pack::Scene> {
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

/// How many transitions deep the scenes that can follow are looked for: a travel scene's own
/// destination, and the ending after it, are made while the party is still on the way.
const MAXIMUM_LOOKAHEAD: usize = 3;

/// The scenes that can follow `current`, nearest first, from the beat script's transitions (the
/// canonical script when the pack gives none). A fight is never plated, so its win leads on to the
/// scene after it (a loss is not made ahead of time); a journey and an ending lead on to what
/// they lead to, so the first picture of a new place is requested while the party can still
/// choose to go there.
fn successors(current: &pack::Scene) -> Vec<pack::Scene> {
    pack::with_content(|content| {
        let script = content
            .beat_script
            .as_ref()
            .unwrap_or_else(|| beat_script::canonical());
        successors_in(script, &content.scenes, current)
    })
}

fn successors_in(
    script: &BeatScript,
    scenes: &[pack::Scene],
    current: &pack::Scene,
) -> Vec<pack::Scene> {
    {
        let scene_of = |beat: &Beat| match &beat.scene_variant {
            Some(variant) => script.scenes.get(variant).cloned(),
            None => scenes
                .iter()
                .find(|candidate| candidate.key == beat.scene)
                .cloned(),
        };
        let mut found: Vec<pack::Scene> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        let mut queue: VecDeque<(&Beat, usize)> = VecDeque::new();
        for beat in &script.beats {
            let here = scene_of(beat)
                .is_some_and(|scene| scene.title == current.title && scene.key == current.key);
            if here {
                queue.extend(
                    beat.transitions
                        .iter()
                        .filter_map(|transition| Some((script.find(&transition.target)?, 1))),
                );
            }
        }
        while let Some((beat, depth)) = queue.pop_front() {
            if seen.contains(&beat.id.as_str()) {
                continue;
            }
            seen.push(&beat.id);
            let onward: Vec<&BeatTransition> = match beat.kind {
                BeatKind::Fight => beat
                    .transitions
                    .iter()
                    .filter(|won| won.trigger == BeatTrigger::FightWon)
                    .collect(),
                BeatKind::Travel | BeatKind::Ending if depth < MAXIMUM_LOOKAHEAD => {
                    beat.transitions.iter().collect()
                }
                _ => Vec::new(),
            };
            if beat.kind != BeatKind::Fight
                && let Some(next) = scene_of(beat)
                && next.title != current.title
                && !found.iter().any(|known| known.title == next.title)
            {
                found.push(next);
            }
            let deeper = if beat.kind == BeatKind::Fight {
                depth
            } else {
                depth + 1
            };
            queue.extend(
                onward
                    .into_iter()
                    .filter_map(|transition| Some((script.find(&transition.target)?, deeper))),
            );
        }
        found
    }
}

/// What one display view says about plates.
struct View {
    /// Names the scene the same way living scenes do: the title and the authored painting.
    scene_key: String,
    title: String,
    asset: String,
    in_combat: bool,
    in_roll: bool,
    graph_revision: u64,
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

fn view_of(message: &rpc::ViewMessage, graph_revision: u64) -> Option<View> {
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        return None;
    };
    let scene = display.scene.as_ref()?;
    let journey = display.journey.as_ref();
    let in_combat = journey.and_then(|journey| rpc::JourneyPhase::try_from(journey.phase).ok())
        == Some(rpc::JourneyPhase::Combat);
    let in_roll = journey.is_some_and(|journey| {
        journey.table_log.last().is_some_and(|line| {
            rpc::TableKind::try_from(line.kind).ok() == Some(rpc::TableKind::Action)
        })
    });
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
        in_roll,
        graph_revision,
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
        placement: Option<PlatePlacement>,
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
    /// The health variants of hurt heroes the scene's plate was named with.
    faces: Vec<(u8, ImageKey)>,
    /// A story scene that never gets a plate (it has no place to stage): the display holds its
    /// establishing picture instead of drawing cut-outs.
    unplated: bool,
    in_combat: bool,
    in_roll: bool,
    graph_revision: u64,
}

struct Book {
    current: Option<Current>,
    entries: BTreeMap<String, Entry>,
    order: VecDeque<String>,
    edit: Edit,
    direct: direct::Book,
}

impl Book {
    fn new() -> Self {
        Self {
            current: None,
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            edit: Edit::Idle,
            direct: direct::Book::default(),
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
    fn shown(&self) -> Option<(ImageKey, Option<PlatePlacement>)> {
        if let Some(shown) = &self.direct.shown {
            return match &self.edit {
                Edit::Ready { key, .. } => Some((key.clone(), shown.1.clone())),
                _ => Some(shown.clone()),
            };
        }
        let Some(Entry::Ready { key, placement }) = self.current_entry() else {
            return None;
        };
        match &self.edit {
            Edit::Ready { key: edited, .. } => Some((edited.clone(), placement.clone())),
            _ => Some((key.clone(), placement.clone())),
        }
    }

    fn covers(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| !current.in_combat)
            && (self.direct.shown.is_some()
                || matches!(self.current_entry(), Some(Entry::Ready { .. })))
    }
}

/// One request for the worker.
enum Job {
    Directed(Box<direct::Work>),
    Fresh {
        id: String,
        graph_revision: u64,
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
    /// Finds the heads in a plate that is already shown with its predicted layout.
    Locate {
        id: String,
        work: Box<Locate>,
    },
}

/// What finding the heads in a finished plate needs.
struct Locate {
    key: ImageKey,
    spec: ShotSpec,
    speaker: shot::SpeakerShot,
    face: Option<ReferenceImage>,
}

enum Done {
    Directed {
        generation: u64,
        scene_key: String,
        key: Option<ImageKey>,
        result: Result<(ImageKey, Option<PlatePlacement>), ImageFailure>,
    },
    Located {
        id: String,
        key: ImageKey,
        elapsed: Duration,
        placement: PlatePlacement,
    },
    Fresh {
        id: String,
        graph_revision: u64,
        ahead: bool,
        elapsed: Duration,
        result: Result<(ImageKey, Option<PlatePlacement>), ImageFailure>,
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
    pub(super) fn observe(&self, message: &rpc::ViewMessage, graph_revision: u64) {
        let Some(view) = view_of(message, graph_revision) else {
            return;
        };
        self.shared.observe(&view);
    }

    /// Adds the plate to the scene of a view about to be sent (to the display or a phone).
    pub(super) fn apply_to_message(&self, message: &mut rpc::ViewMessage) {
        match message.audience.as_mut() {
            Some(rpc::view_message::Audience::Display(display)) => {
                let Some(scene) = display.scene.as_mut() else {
                    return;
                };
                let (scene_key, _) = scene_identity(&display.scene_asset, scene);
                display.live_plate_graph_revision =
                    self.shared.apply(&scene_key, scene).unwrap_or_default();
                self.shared.project_scene_clip(&scene_key, scene);
            }
            Some(rpc::view_message::Audience::Player(player)) => {
                let Some(scene) = player.scene.as_mut() else {
                    return;
                };
                let (scene_key, _) = scene_identity("", scene);
                let _revision = self.shared.apply(&scene_key, scene);
                scene.scene_video_state = rpc::HomeVideoState::None as i32;
                scene.scene_video_path.clear();
            }
            None => {}
        }
    }
}

fn wire_side(side: shot::Side) -> i32 {
    (match side {
        shot::Side::Left => rpc::PlateSide::Left,
        shot::Side::Right => rpc::PlateSide::Right,
    }) as i32
}

fn wire_layout(placement: &PlatePlacement) -> rpc::PlateLayout {
    let speaker = &placement.speaker;
    rpc::PlateLayout {
        bubble_side: wire_side(speaker.bubble_side),
        head_x_permille: u32::from(speaker.head_x_permille),
        head_y_permille: u32::from(speaker.head_y_permille),
        head_width_permille: u32::from(speaker.head_width_permille),
        head_height_permille: u32::from(speaker.head_height_permille),
        heroes: placement
            .heroes
            .iter()
            .map(|hero| rpc::PlateHead {
                hero_index: u32::from(hero.art.saturating_sub(1)),
                bubble_side: wire_side(hero.layout.bubble_side),
                head_x_permille: u32::from(hero.layout.head_x_permille),
                head_y_permille: u32::from(hero.layout.head_y_permille),
                head_width_permille: u32::from(hero.layout.head_width_permille),
                head_height_permille: u32::from(hero.layout.head_height_permille),
            })
            .collect(),
    }
}

/// One head as it is kept beside the cached plate (art 0 is the speaker).
#[derive(serde::Deserialize, serde::Serialize)]
struct StoredHead {
    art: u8,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    bubble_left: bool,
}

/// The measured placement as it is kept beside the cached plate, so a later server run does not
/// ask the vision model again.
#[derive(serde::Deserialize, serde::Serialize)]
struct StoredPlacement {
    speaker: StoredHead,
    heroes: Vec<StoredHead>,
}

impl StoredHead {
    fn of(art: u8, layout: &PlateLayout) -> Self {
        Self {
            art,
            x: layout.head_x_permille,
            y: layout.head_y_permille,
            width: layout.head_width_permille,
            height: layout.head_height_permille,
            bubble_left: layout.bubble_side == shot::Side::Left,
        }
    }

    /// `None` for numbers outside the plate: a note that has been edited or damaged is not trusted.
    fn layout(&self) -> Option<PlateLayout> {
        (self.x <= 1000
            && self.y <= 1000
            && (1..=400).contains(&self.width)
            && (1..=500).contains(&self.height))
        .then_some(PlateLayout {
            bubble_side: if self.bubble_left {
                shot::Side::Left
            } else {
                shot::Side::Right
            },
            head_x_permille: self.x,
            head_y_permille: self.y,
            head_width_permille: self.width,
            head_height_permille: self.height,
        })
    }
}

impl StoredPlacement {
    fn of(placement: &PlatePlacement) -> Self {
        Self {
            speaker: StoredHead::of(0, &placement.speaker),
            heroes: placement
                .heroes
                .iter()
                .map(|hero| StoredHead::of(hero.art, &hero.layout))
                .collect(),
        }
    }

    fn placement(&self) -> Option<PlatePlacement> {
        let heroes = self
            .heroes
            .iter()
            .filter_map(|head| {
                Some(HeroHead {
                    art: head.art,
                    layout: head.layout()?,
                })
            })
            .collect();
        Some(PlatePlacement {
            speaker: self.speaker.layout()?,
            heroes,
        })
    }
}

impl Shared {
    fn current_clip_context(
        &self,
        scene_key: &str,
        plate_key: &str,
    ) -> Option<(String, &'static str, bool)> {
        let book = locked(&self.book);
        let current = book.current.as_ref()?;
        if current.scene_key != scene_key
            || book
                .shown()
                .as_ref()
                .is_none_or(|(key, _)| key.as_str() != plate_key)
        {
            return None;
        }
        let directed = book.direct.shown.is_some();
        let (description, reason) =
            if directed && let Some(description) = book.direct.description.clone() {
                (description, "directed-plate")
            } else {
                let scene = current.scene.as_ref()?;
                let spec = scene.stage(&current.heroes).ok()?;
                (
                    super::super::home_video::scene_description(&spec),
                    if matches!(&book.edit, Edit::Ready { key, .. } if key.as_str() == plate_key) {
                        "plate-edit"
                    } else {
                        "scene-plate"
                    },
                )
            };
        Some((description, reason, current.in_combat || current.in_roll))
    }

    fn project_scene_clip(&self, scene_key: &str, scene: &mut rpc::GameplayScene) {
        scene.scene_video_state = rpc::HomeVideoState::None as i32;
        scene.scene_video_path.clear();
        let Some(plate_key) = scene
            .live_plate
            .strip_prefix("/gameplay/imagery/")
            .filter(|key| key.len() == 64)
        else {
            return;
        };
        let Some((description, reason, blocked)) = self.current_clip_context(scene_key, plate_key)
        else {
            return;
        };
        if blocked {
            super::super::home_video::request_scene_clip(
                plate_key,
                Vec::new(),
                &description,
                reason,
                true,
            );
            return;
        }
        let view = super::super::home_video::scene_clip_view(plate_key);
        scene.scene_video_state = view.proto_state() as i32;
        scene.scene_video_path = view.path();
        if view.proto_state() != rpc::HomeVideoState::None {
            return;
        }
        let imagery = self.inner.imagery.clone();
        let plate_key = plate_key.to_owned();
        tokio::spawn(async move {
            if let Lookup::Image { bytes, .. } = imagery.image(&plate_key).await {
                super::super::home_video::request_scene_clip(
                    &plate_key,
                    bytes.to_vec(),
                    &description,
                    reason,
                    false,
                );
            }
        });
    }

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
        let graph_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.graph_revision != view.graph_revision);
        let combat_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.in_combat != view.in_combat);
        let roll_changed = book
            .current
            .as_ref()
            .is_some_and(|current| current.in_roll != view.in_roll);
        let mut woke = false;
        if scene_changed || party_changed || graph_changed {
            let scene = located.as_ref().and_then(PlateScene::of).or_else(|| {
                (!scene_changed && !party_changed)
                    .then(|| {
                        book.current
                            .as_ref()
                            .and_then(|current| current.scene.clone())
                    })
                    .flatten()
            });
            let faces = self.inner.hero_face_keys(&view.hero_names);
            let id = scene
                .as_ref()
                .map(|scene| scene.plate_id(&view.heroes, &faces, view.graph_revision));
            let old_id = graph_changed
                .then(|| book.current.as_ref().and_then(|current| current.id.clone()))
                .flatten();
            if let Some(old_id) = old_id
                && id.as_ref() != Some(&old_id)
                && matches!(book.entries.get(&old_id), Some(Entry::Pending))
            {
                book.entries.remove(&old_id);
                book.order.retain(|known| known != &old_id);
            }
            if scene_changed || graph_changed {
                book.edit = Edit::Idle;
                book.direct.reset();
                // A plate that failed is asked for again once when its scene comes back.
                if let Some(id) = &id
                    && matches!(book.entries.get(id), Some(Entry::Failed))
                {
                    book.entries.remove(id);
                    book.order.retain(|known| known != id);
                }
            }
            let unplated = scene.is_none()
                && located.as_ref().is_none_or(|located| {
                    !matches!(located.key, SceneKey::Room | SceneKey::Combat)
                });
            book.current = Some(Current {
                scene_key: view.scene_key.clone(),
                id,
                scene: scene.clone(),
                heroes: view.heroes.clone(),
                faces: faces.clone(),
                unplated,
                in_combat: view.in_combat,
                in_roll: view.in_roll,
                graph_revision: view.graph_revision,
            });
            // The lobby has no plate; the opening plate is requested when that scene becomes current.
            if !view.in_combat {
                if let Some(scene) = &scene {
                    self.ensure(
                        &mut book,
                        scene,
                        &view.heroes,
                        &faces,
                        view.graph_revision,
                        false,
                    );
                }
                if scene.is_some() && (scene_changed || party_changed) {
                    for next in next
                        .iter()
                        .take(MAXIMUM_SUCCESSORS)
                        .filter_map(PlateScene::of)
                    {
                        self.ensure(
                            &mut book,
                            &next,
                            &view.heroes,
                            &faces,
                            view.graph_revision,
                            scene.is_some(),
                        );
                    }
                }
            }
            woke = true;
        } else if (combat_changed || roll_changed)
            && let Some(current) = book.current.as_mut()
        {
            current.in_combat = view.in_combat;
            current.in_roll = view.in_roll;
            woke = true;
        }
        if !woke && !view.in_combat && book.current_entry().is_none() {
            // The scene's request was dropped (a full queue): it is made again by this view.
            let retry = book.current.as_ref().and_then(|current| {
                Some((
                    current.scene.clone()?,
                    current.heroes.clone(),
                    current.faces.clone(),
                    current.graph_revision,
                ))
            });
            if let Some((scene, heroes, faces, graph_revision)) = retry {
                self.ensure(&mut book, &scene, &heroes, &faces, graph_revision, false);
            }
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
        graph_revision: u64,
        ahead: bool,
    ) {
        let id = scene.plate_id(heroes, faces, graph_revision);
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
            graph_revision,
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

    fn apply(&self, scene_key: &str, scene: &mut rpc::GameplayScene) -> Option<u64> {
        let book = locked(&self.book);
        let Some(current) = &book.current else {
            return None;
        };
        if current.scene_key != scene_key || current.in_combat {
            return None;
        }
        if book.direct.shown.is_some()
            && let Some((key, placement)) = book.shown()
        {
            scene.live_plate = key.path();
            scene.plate_layout = placement.as_ref().map(wire_layout);
            scene.plate_state = rpc::PlateState::Ready as i32;
            return Some(current.graph_revision);
        }
        match book.current_entry() {
            Some(Entry::Ready { .. }) => {
                if let Some((key, placement)) = book.shown() {
                    scene.live_plate = key.path();
                    scene.plate_layout = placement.as_ref().map(wire_layout);
                    scene.plate_state = rpc::PlateState::Ready as i32;
                    return Some(current.graph_revision);
                }
            }
            Some(Entry::Failed) => {
                scene.plate_state = rpc::PlateState::Failed as i32;
            }
            Some(Entry::NotPlanned) => {
                scene.plate_state = rpc::PlateState::Unavailable as i32;
            }
            // A planned scene whose request is not in the book yet is pending too: the display
            // never mistakes a plate that is on its way for a scene without one.
            Some(Entry::Pending) | None => {
                if current.scene.is_some() {
                    scene.plate_state = rpc::PlateState::Pending as i32;
                } else if current.unplated {
                    scene.plate_state = rpc::PlateState::Unavailable as i32;
                }
            }
        }
        None
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
            Job::Directed(work) => {
                tasks.spawn(async move { shared.render_directed(*work).await });
            }
            Job::Fresh {
                id,
                graph_revision,
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
                    let result = result.map(|(key, placement, locate)| {
                        // The plate is shown with its predicted layout as soon as it lands; the
                        // measured one follows. A full queue keeps the prediction.
                        if let Some(work) = locate {
                            let _kept_prediction = shared.jobs.try_send(Job::Locate {
                                id: id.clone(),
                                work,
                            });
                        }
                        (key, placement)
                    });
                    Done::Fresh {
                        id,
                        graph_revision,
                        ahead,
                        elapsed: started.elapsed(),
                        result,
                    }
                });
            }
            Job::Locate { id, work } => {
                tasks.spawn(async move {
                    let started = tokio::time::Instant::now();
                    let placement = shared
                        .measured_or_predicted(
                            &work.key,
                            &work.spec,
                            &work.speaker,
                            work.face.as_ref(),
                        )
                        .await;
                    Done::Located {
                        id,
                        key: work.key,
                        elapsed: started.elapsed(),
                        placement,
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

    /// The art that shows a speaker in a plate: the full-length portrait (or figure) and the close
    /// portrait of the face, each when it exists.
    async fn speaker_art(&self, sprite: &str) -> (Option<ReferenceImage>, Option<ReferenceImage>) {
        let stem = sprite.rsplit('/').next().unwrap_or(sprite);
        let Some(name) = stem.strip_prefix("sprite-") else {
            return (None, None);
        };
        let mut figure = None;
        for candidate in [
            format!("portrait-{name}-full"),
            format!("figure-{name}"),
            stem.to_owned(),
        ] {
            if let Ok(art) = self.inner.load(candidate).await {
                figure = Some(art);
                break;
            }
        }
        let face = self.inner.load(format!("portrait-{name}")).await.ok();
        (figure, face)
    }

    async fn fresh_request(
        &self,
        scene: &PlateScene,
        spec: &ShotSpec,
        heroes: &[u8],
        faces: &[(u8, ImageKey)],
        direction: Option<&str>,
    ) -> Result<(ImageRequest, Option<ReferenceImage>), ImageFailure> {
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
        // The cast's art leads: the full-length figure and a close portrait of the face. Both
        // go into every plate that shows the speaker, because a figure alone let the same person
        // drift between two scenes' plates.
        let (figure, face) = match &scene.speaker {
            Some(speaker) => self.speaker_art(&speaker.sprite).await,
            None => (None, None),
        };
        let has_speaker_art = figure.is_some();
        let has_speaker_face = has_speaker_art && face.is_some();
        let locator_face = face.clone().or_else(|| figure.clone());
        references.extend(figure);
        if has_speaker_face {
            references.extend(face);
        }
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
                speaker_face: has_speaker_face,
                place: has_place,
            },
        )
        .map_err(|_| ImageFailure::InvalidRequest)?;
        let prompt = match direction {
            Some(direction) => prompt.with_development_direction(direction),
            None => prompt,
        };
        Ok((
            ImageRequest {
                kind: ImageKind::ScenePlate,
                references,
                prompt,
                output: OutputSpec::PLATE,
            },
            locator_face,
        ))
    }

    async fn render_fresh(
        &self,
        scene: &PlateScene,
        spec: &ShotSpec,
        heroes: &[u8],
        faces: &[(u8, ImageKey)],
    ) -> Result<(ImageKey, Option<PlatePlacement>, Option<Box<Locate>>), ImageFailure> {
        let (request, locator_face) = self.fresh_request(scene, spec, heroes, faces, None).await?;
        let key = self.inner.imagery.render(request).await?;
        let Some(speaker) = &spec.speaker else {
            return Ok((key, None, None));
        };
        if let Some(stored) = self.stored_placement(&key).await {
            return Ok((key, Some(stored), None));
        }
        let predicted = PlatePlacement {
            speaker: speaker.layout,
            heroes: Vec::new(),
        };
        let work = Locate {
            key: key.clone(),
            spec: spec.clone(),
            speaker: speaker.clone(),
            face: locator_face,
        };
        Ok((key, Some(predicted), Some(Box::new(work))))
    }

    /// The placement kept beside a cached plate by an earlier measurement, when it is intact.
    async fn stored_placement(&self, key: &ImageKey) -> Option<PlatePlacement> {
        let note = self.inner.imagery.read_note(key).await?;
        serde_json::from_slice::<StoredPlacement>(&note)
            .ok()?
            .placement()
    }

    /// Where the heads are in the finished plate, as the vision model finds them: the speaker's
    /// and those of the heroes it asked for. A placement found before is kept beside the cached
    /// plate and read back, so a later server run does not ask again. When the speaker cannot be
    /// found (no provider, a failure, an implausible answer) the layout the director predicted
    /// stands, with no hero positions. The image model does not place anyone where it is asked.
    async fn measured_or_predicted(
        &self,
        key: &ImageKey,
        spec: &ShotSpec,
        speaker: &shot::SpeakerShot,
        face: Option<&ReferenceImage>,
    ) -> PlatePlacement {
        if let Some(stored) = self.stored_placement(key).await {
            return stored;
        }
        let speaker_description = format!("{}, {}", speaker.look, speaker.task);
        let hero_ids: Vec<String> = spec
            .heroes
            .iter()
            .map(|hero| format!("hero-{}", hero.art))
            .collect();
        let mut targets: Vec<(&str, &str)> = vec![("speaker", speaker_description.as_str())];
        for (hero, id) in spec.heroes.iter().zip(&hero_ids) {
            targets.push((id.as_str(), hero_locator_phrase(hero.art)));
        }
        let started = tokio::time::Instant::now();
        let found = self.inner.imagery.locate_heads(key, face, &targets).await;
        record(
            "gameplay.living_scene.head_located",
            &[
                (
                    "scene.outcome",
                    found
                        .as_ref()
                        .map_or_else(|failure| failure.label(), |_| "located")
                        .to_owned(),
                ),
                (
                    "scene.elapsed_ms",
                    started.elapsed().as_millis().to_string(),
                ),
            ],
        );
        let located = found.unwrap_or_default();
        let Some(head) = located.iter().find(|head| head.id == "speaker") else {
            return PlatePlacement {
                speaker: speaker.layout,
                heroes: Vec::new(),
            };
        };
        let heroes = spec
            .heroes
            .iter()
            .filter_map(|hero| {
                let id = format!("hero-{}", hero.art);
                let found = located.iter().find(|head| head.id == id)?;
                Some(HeroHead {
                    art: hero.art,
                    layout: shot::measured_layout(
                        found.head.left,
                        found.head.top,
                        found.head.right,
                        found.head.bottom,
                    ),
                })
            })
            .collect();
        let placement = PlatePlacement {
            speaker: shot::measured_layout(
                head.head.left,
                head.head.top,
                head.head.right,
                head.head.bottom,
            ),
            heroes,
        };
        if let Ok(note) = serde_json::to_vec(&StoredPlacement::of(&placement)) {
            self.inner.imagery.write_note(key, note).await;
        }
        placement
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
        let (figure, _) = match &scene.speaker {
            Some(speaker) => self.speaker_art(&speaker.sprite).await,
            None => (None, None),
        };
        let has_speaker_art = figure.is_some();
        references.extend(figure);
        let prompt = plate_moment(
            self.inner.rating,
            face.expression,
            face.tier,
            has_speaker_art,
        )
        .map_err(|_| ImageFailure::InvalidRequest)?;
        let direction = locked(&self.book).direct.keep.clone();
        let prompt = match direction {
            Some(direction) => prompt.with_development_direction(&direction),
            None => prompt,
        };
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
        if let Done::Directed {
            generation,
            scene_key,
            key,
            result,
        } = done
        {
            self.finish_directed(generation, &scene_key, key, result);
            return;
        }
        let (kind, elapsed, outcome) = match &done {
            Done::Directed { .. } => unreachable!("handled above"),
            Done::Fresh {
                elapsed, result, ..
            } => ("plate", *elapsed, result.as_ref().map(|_| ())),
            Done::Edit {
                elapsed, result, ..
            } => ("plate_edit", *elapsed, result.as_ref().map(|_| ())),
            Done::Located { elapsed, .. } => ("plate_heads", *elapsed, Ok(())),
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
            Done::Directed { .. } => unreachable!("handled above"),
            Done::Located {
                id, key, placement, ..
            } => {
                // The measured heads replace the prediction of the same plate, if it is still there.
                if let Some(Entry::Ready {
                    key: shown,
                    placement: kept,
                }) = book.entries.get_mut(&id)
                    && *shown == key
                {
                    *kept = Some(placement);
                }
            }
            Done::Fresh {
                id,
                graph_revision,
                ahead,
                result,
                ..
            } => {
                let entry = match result {
                    Ok((key, placement)) => Entry::Ready { key, placement },
                    Err(_) => Entry::Failed,
                };
                let current_graph = book.current.as_ref().is_some_and(|current| {
                    current.graph_revision == graph_revision
                        && current.id.as_deref() == Some(id.as_str())
                });
                // Current plates from an older graph are dropped. Ahead plates remain cached
                // until that scene becomes current and its graph revision is known.
                if (current_graph || ahead) && book.entries.contains_key(&id) {
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
        if book.direct.pending {
            return;
        }
        let Some(current) = &book.current else {
            return;
        };
        let (Some(scene), false) = (current.scene.clone(), current.in_combat) else {
            return;
        };
        let Some(speaker) = scene.speaker.clone() else {
            return;
        };
        let base = if book.direct.keep.is_some() {
            book.direct.shown.as_ref().map(|shown| shown.0.clone())
        } else {
            None
        }
        .or_else(|| match book.current_entry() {
            Some(Entry::Ready { key, .. }) => Some(key.clone()),
            _ => None,
        });
        let Some(base) = base else {
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
                    if book.direct.keep.is_none() {
                        book.direct.shown = None;
                    }
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
            .filter_map(|(art, name)| Some((*art, state.hero_faces.get(name)?.stage.clone()?)))
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
