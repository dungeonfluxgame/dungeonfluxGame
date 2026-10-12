//! Living scenes: the shared display's backdrop and characters follow the mood of the story.
//!
//! The director watches the display views the server sends and the social ledger's record of
//! how the last exchange landed. When the story's mood changes it asks the image service for
//! (1) a changed expression on the character in focus, cached forever per sprite and
//! expression, and (2) at most once per interval, the backdrop re-framed and re-lit. A finished
//! image is published only as presentation: the display view the server sends carries the new
//! art path and watchers are woken the way a new table line wakes them, with the same revision,
//! the same offers and the same game state. Nothing here waits for an image, changes an
//! outcome, or runs during combat, a dice roll or the first seconds of a scene.
//!
//! The director's one worker task owns every render task it starts (a `JoinSet`), ends when
//! the last handle is dropped, and reports each outcome to its state, never to the game.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use df_protocol::common as rpc;
use tokio::{sync::watch, task::JoinSet, time::Instant};

use super::{
    adventure::{dossier::Attitude, setting::SettingKind},
    imagery::{
        CharacterFacts, CharacterRole, ImageFailure, ImageKey, ImageKind, ImageRequest, Imagery,
        LifeStage, OutputSpec, PlaceFacts, ReferenceImage, ReferenceStore, cast_expression,
        scene_refresh,
    },
    pack::{self, SceneKey},
    scene_mood::{
        HealthTier, Beat, CastState, ExchangeSignal, Face, Hysteresis, Look, MoodInputs,
        MoodTracker, SceneTone, TableKind, Tempo, TempoLevel,
    },
    social::SocialLedger,
    startup_configuration::ContentRating,
};

/// How many of the latest table lines the mood reads (kinds only).
const RECENT_TABLE_LINES: usize = 6;

/// The timing of living scenes. Production values come from the validated interval.
#[derive(Clone, Copy, Debug)]
pub(super) struct Timing {
    /// The least time between two scene refreshes.
    pub(super) interval: Duration,
    /// No refresh starts or lands this soon after a scene begins.
    pub(super) scene_start_quiet: Duration,
    /// No refresh starts or lands this soon after an accepted action (which may be a roll).
    pub(super) roll_quiet: Duration,
    /// With no story event for this many intervals, a gentle alternate angle is allowed.
    pub(super) idle_after_intervals: u32,
    /// An alternate angle comes at most once per this many intervals.
    pub(super) idle_every_intervals: u32,
    /// How often the worker re-evaluates.
    pub(super) tick: Duration,
    pub(super) hysteresis: Hysteresis,
}

impl Timing {
    pub(super) fn from_interval(interval: Duration) -> Self {
        Self {
            interval,
            scene_start_quiet: Duration::from_secs(8),
            roll_quiet: Duration::from_secs(6),
            idle_after_intervals: 4,
            idle_every_intervals: 3,
            tick: Duration::from_secs(1),
            hysteresis: Hysteresis::default(),
        }
    }
}

/// Why a scene refresh was started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RefreshReason {
    /// The mood or shot differs from what is on screen.
    Change,
    /// Nothing has happened for a long while: a gentle alternate angle.
    AlternateAngle,
}

/// When a repaint may start and land. Pure: every method takes the time explicitly.
#[derive(Debug)]
pub(super) struct Pacer {
    timing: Timing,
    scene_started: Duration,
    last_refresh_started: Option<Duration>,
    last_action: Option<Duration>,
    last_story_event: Duration,
    last_alternate: Option<Duration>,
    alternates: usize,
}

impl Pacer {
    pub(super) fn new(timing: Timing, now: Duration) -> Self {
        Self {
            timing,
            scene_started: now,
            last_refresh_started: None,
            last_action: None,
            last_story_event: now,
            last_alternate: None,
            alternates: 0,
        }
    }

    pub(super) fn begin_scene(&mut self, now: Duration) {
        *self = Self::new(self.timing, now);
    }

    /// An accepted action happened (it may be a dice roll).
    pub(super) fn note_action(&mut self, now: Duration) {
        self.last_action = Some(now);
        self.last_story_event = now;
    }

    /// A social exchange happened.
    pub(super) fn note_story_event(&mut self, now: Duration) {
        self.last_story_event = now;
    }

    fn quiet(&self, now: Duration) -> bool {
        now.saturating_sub(self.scene_started) < self.timing.scene_start_quiet
            || self
                .last_action
                .is_some_and(|action| now.saturating_sub(action) < self.timing.roll_quiet)
    }

    /// Whether a refresh may start now, and why. Never inside the interval, never in combat,
    /// never in the quiet windows.
    pub(super) fn refresh_due(
        &self,
        now: Duration,
        in_combat: bool,
        worth_rendering: bool,
    ) -> Option<RefreshReason> {
        if in_combat || self.quiet(now) {
            return None;
        }
        if self
            .last_refresh_started
            .is_some_and(|started| now.saturating_sub(started) < self.timing.interval)
        {
            return None;
        }
        if worth_rendering {
            return Some(RefreshReason::Change);
        }
        let idle_for = now.saturating_sub(self.last_story_event);
        let idle_long = idle_for >= self.timing.interval * self.timing.idle_after_intervals;
        let alternate_due = self.last_alternate.is_none_or(|last| {
            now.saturating_sub(last) >= self.timing.interval * self.timing.idle_every_intervals
        });
        (idle_long && alternate_due).then_some(RefreshReason::AlternateAngle)
    }

    pub(super) fn refresh_started(&mut self, now: Duration, reason: RefreshReason) {
        self.last_refresh_started = Some(now);
        if reason == RefreshReason::AlternateAngle {
            self.last_alternate = Some(now);
            self.alternates += 1;
        }
    }

    /// How many alternate angles have been started in this scene.
    pub(super) fn alternates(&self) -> usize {
        self.alternates
    }

    /// Whether a finished repaint may land now: not in combat, not in a quiet window.
    pub(super) fn may_publish(&self, now: Duration, in_combat: bool) -> bool {
        !in_combat && !self.quiet(now)
    }
}

/// One character on stage, from the view and the pack.
#[derive(Clone, Debug)]
struct Member {
    id: String,
    role: CharacterRole,
    life_stage: LifeStage,
    name: String,
    /// The cut-out's asset stem, as the scene cast names it.
    sprite: String,
    /// The social ledger's id of this character's dossier, when the adventure gives one.
    dossier_id: Option<String>,
    opposes: bool,
    starting_attitude: Option<Attitude>,
}

/// The facts of the current scene that do not change while it lasts.
#[derive(Clone, Debug)]
struct SceneFacts {
    place: Option<PlaceFacts>,
    tone: SceneTone,
    beat: Beat,
    members: Vec<Member>,
}

/// What one display view says about the stage.
#[derive(Clone, Debug)]
struct Stage {
    /// Names the scene: the title and the authored backdrop path.
    scene_key: String,
    scene_asset: String,
    revision: Option<(u64, u64)>,
    in_combat: bool,
    tempo: Option<Tempo>,
    speaking_ids: BTreeSet<String>,
    recent_kinds: Vec<TableKind>,
    title: String,
    cast: Vec<StageCast>,
    /// Hit points of named combatants the view still carries: `(name, current, maximum)`.
    hurt: Vec<(String, u32, u32)>,
}

#[derive(Clone, Debug)]
struct StageCast {
    id: String,
    sprite: String,
}

fn stage_of(message: &rpc::ViewMessage) -> Option<Stage> {
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        return None;
    };
    let scene = display.scene.as_ref()?;
    let journey = display.journey.as_ref();
    let phase = journey.and_then(|journey| rpc::JourneyPhase::try_from(journey.phase).ok());
    let in_combat = phase == Some(rpc::JourneyPhase::Combat);
    let tempo = journey
        .and_then(|journey| journey.tempo.as_ref())
        .and_then(|cue| {
            let phase = match rpc::TempoPhase::try_from(cue.phase).ok()? {
                rpc::TempoPhase::Calm => TempoLevel::Calm,
                rpc::TempoPhase::Build => TempoLevel::Build,
                rpc::TempoPhase::Anticipation => TempoLevel::Anticipation,
                rpc::TempoPhase::Peak => TempoLevel::Peak,
                rpc::TempoPhase::Release => TempoLevel::Release,
                rpc::TempoPhase::Unspecified => return None,
            };
            Some(Tempo {
                phase,
                intensity_permille: u16::try_from(cue.intensity_permille).unwrap_or(u16::MAX),
            })
        });
    let recent_kinds = journey
        .map(|journey| {
            let lines = &journey.table_log;
            lines
                .iter()
                .skip(lines.len().saturating_sub(RECENT_TABLE_LINES))
                .filter_map(|line| match rpc::TableKind::try_from(line.kind).ok()? {
                    rpc::TableKind::Speech => Some(TableKind::Speech),
                    rpc::TableKind::Action => Some(TableKind::Action),
                    rpc::TableKind::Unspecified => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let asset = if scene.scene_asset.is_empty() {
        display.scene_asset.clone()
    } else {
        scene.scene_asset.clone()
    };
    Some(Stage {
        scene_key: format!("{}|{}", scene.title, asset),
        scene_asset: asset,
        revision: message
            .revision
            .as_ref()
            .and_then(|revision| Some((revision.epoch.as_ref()?.value?, revision.sequence?))),
        in_combat,
        tempo,
        speaking_ids: scene
            .cast
            .iter()
            .filter(|actor| actor.speaking)
            .map(|actor| actor.id.clone())
            .collect(),
        recent_kinds,
        title: scene.title.clone(),
        cast: scene
            .cast
            .iter()
            .map(|actor| StageCast {
                id: actor.id.clone(),
                sprite: actor.sprite.clone(),
            })
            .collect(),
        hurt: journey
            .and_then(|journey| journey.combat.as_ref())
            .map(|combat| {
                combat
                    .participants
                    .iter()
                    .map(|member| {
                        (
                            member.name.clone(),
                            member.hit_points,
                            member.maximum_hit_points,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn tone_of(kind: SettingKind) -> SceneTone {
    match kind {
        SettingKind::GraveyardGate | SettingKind::UndercroftStudy | SettingKind::CiderCellar => {
            SceneTone::Grim
        }
        SettingKind::Inn
        | SettingKind::TeaHouse
        | SettingKind::FarmKitchen
        | SettingKind::Shop
        | SettingKind::LanternShop
        | SettingKind::WaystationHall
        | SettingKind::Bothy
        | SettingKind::RefugeHall
        | SettingKind::PilgrimsHall
        | SettingKind::Apothecary => SceneTone::Cozy,
        _ => SceneTone::Neutral,
    }
}

fn role_of(role: pack::CastRole) -> CharacterRole {
    match role {
        pack::CastRole::Patron => CharacterRole::Patron,
        pack::CastRole::Antagonist => CharacterRole::Antagonist,
        pack::CastRole::Contact => CharacterRole::Contact,
        pack::CastRole::Bystander => CharacterRole::Bystander,
    }
}

/// The facts of the pack scene a display view shows: matched by its authored backdrop and title.
fn facts_of(stage: &Stage, social: &SocialLedger) -> SceneFacts {
    pack::with_content(|content| {
        let wanted = stage.scene_asset.trim_start_matches('/');
        let matches = |scene: &&pack::Scene, asset: &str| {
            asset.trim_start_matches('/') == wanted && scene.title == stage.title
        };
        // The pack's own scenes first, then the scenes a beat script gives its peaceful beats
        // (their painting may be replaced by the adventure's own art for that variant).
        let overrides = pack::active().map(|loaded| loaded.script_scene_assets.clone());
        let scene = content
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
                        matches(&scene, asset).then_some(scene)
                    })
                })
            });
        let Some(scene) = scene else {
            return SceneFacts {
                place: None,
                tone: SceneTone::Neutral,
                beat: Beat::Conversation,
                members: Vec::new(),
            };
        };
        let members = scene
            .cast
            .iter()
            .map(|member| {
                let role = match member.role {
                    pack::CastRole::Patron => rpc::SceneRole::Patron,
                    pack::CastRole::Antagonist => rpc::SceneRole::Antagonist,
                    pack::CastRole::Contact => rpc::SceneRole::Contact,
                    pack::CastRole::Bystander => rpc::SceneRole::Bystander,
                };
                let dossier = social.dossier_of(role, &member.id);
                Member {
                    id: member.id.clone(),
                    role: role_of(member.role),
                    life_stage: LifeStage::from_age(member.age),
                    name: member.name.clone(),
                    sprite: member.sprite.clone(),
                    dossier_id: dossier.map(|dossier| dossier.npc_id.clone()),
                    opposes: dossier.is_some_and(|dossier| dossier.role.opposes_party()),
                    starting_attitude: dossier.map(|dossier| dossier.starting_attitude),
                }
            })
            .collect();
        SceneFacts {
            place: scene
                .setting
                .as_ref()
                .and_then(|setting| PlaceFacts::new(&setting.name, &setting.short)),
            tone: scene
                .setting
                .as_ref()
                .map_or(SceneTone::Neutral, |setting| tone_of(setting.kind)),
            beat: match scene.key {
                SceneKey::Victory => Beat::Victory,
                SceneKey::Defeat => Beat::Defeat,
                SceneKey::Room | SceneKey::Combat => Beat::Confrontation,
                SceneKey::Opening
                | SceneKey::DialogueSeal
                | SceneKey::DialogueEscort
                | SceneKey::Inn => Beat::Conversation,
            },
            members,
        }
    })
}

/// A published character overlay: the expression variant of one cut-out.
#[derive(Clone, Debug)]
struct SpriteOverlay {
    /// The authored cut-out this variant replaces; it only applies while the cast names it.
    source_sprite: String,
    key: ImageKey,
}

#[derive(Clone, Debug)]
struct BackdropOverlay {
    key: ImageKey,
}

/// A finished scene refresh waiting for a quiet moment to land.
#[derive(Debug)]
struct ReadyBackdrop {
    key: ImageKey,
    look: Look,
    alternate: bool,
}

struct State {
    stage: Option<Stage>,
    facts: Option<SceneFacts>,
    tracker: MoodTracker,
    pacer: Pacer,
    backdrop: Option<BackdropOverlay>,
    sprites: BTreeMap<String, SpriteOverlay>,
    ready_backdrop: Option<ReadyBackdrop>,
    refresh_in_flight: bool,
    expression_in_flight: BTreeSet<String>,
    /// Faces that failed in this scene are not asked for again.
    failed_faces: BTreeSet<(String, Face)>,
    last_revision: Option<(u64, u64)>,
    budget_exhausted: bool,
}

/// What the worker should start.
enum Job {
    Expression {
        member: Member,
        face: Face,
    },
    Refresh {
        place: PlaceFacts,
        scene_asset: String,
        look: Look,
        reason: RefreshReason,
    },
}

enum Done {
    Expression {
        npc_id: String,
        face: Face,
        source_sprite: String,
        result: Result<ImageKey, ImageFailure>,
    },
    Refresh {
        look: Look,
        reason: RefreshReason,
        scene_key: String,
        result: Result<ImageKey, ImageFailure>,
    },
}

struct Inner {
    imagery: Imagery,
    references: ReferenceStore,
    social: Arc<SocialLedger>,
    rating: ContentRating,
    timing: Timing,
    state: Mutex<State>,
    /// Counts overlay changes so watchers rebuild their view.
    wake: watch::Sender<u64>,
    /// The scene key of the last published overlay change, for tests and diagnostics.
    origin: Instant,
}

/// Ends the worker with the last handle.
struct WorkerGuard(tokio::task::JoinHandle<()>);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The living-scenes director: a cheap handle shared by the view path.
#[derive(Clone)]
pub(super) struct LivingScenes {
    inner: Option<Arc<Inner>>,
    _worker: Option<Arc<WorkerGuard>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One diagnostic event. Its fields are closed-vocabulary labels (a mood, a shot, an outcome)
/// and never a prompt, a reference or an image.
fn record(event: &'static str, fields: &[(&'static str, String)]) {
    let context = df_observe::OperationContext {
        trace_parent: String::new(),
        build: crate::BUILD_ID.to_owned(),
    };
    let mut span = df_observe::begin(&context, "gameplay.living_scene");
    span.event(event, fields);
    span.finish_unmeasured("recorded");
}

impl LivingScenes {
    /// Living scenes that do nothing: imagery is off.
    pub(super) fn disabled() -> Self {
        Self {
            inner: None,
            _worker: None,
        }
    }

    /// Starts the director and its worker task. Must be called inside the runtime.
    pub(super) fn start(
        imagery: Imagery,
        references: ReferenceStore,
        social: Arc<SocialLedger>,
        rating: ContentRating,
        timing: Timing,
    ) -> Self {
        if !imagery.is_on() {
            return Self::disabled();
        }
        let origin = Instant::now();
        let inner = Arc::new(Inner {
            imagery,
            references,
            social,
            rating,
            timing,
            state: Mutex::new(State::new(timing)),
            wake: watch::channel(0).0,
            origin,
        });
        let worker = tokio::spawn(Arc::clone(&inner).run());
        Self {
            inner: Some(inner),
            _worker: Some(Arc::new(WorkerGuard(worker))),
        }
    }

    /// Wakes when the published art changed, so watchers rebuild their view.
    pub(super) fn subscribe(&self) -> Option<watch::Receiver<u64>> {
        self.inner.as_ref().map(|inner| inner.wake.subscribe())
    }

    /// Notes a display view the server is sending: its scene, cast, tempo and revision. Player
    /// views are ignored.
    pub(super) fn observe(&self, message: &rpc::ViewMessage) {
        let Some(inner) = &self.inner else {
            return;
        };
        let Some(stage) = stage_of(message) else {
            return;
        };
        inner.observe(stage);
    }

    /// Adds the published art to a display view about to be sent. Presentation only: no other
    /// field of the view changes, and nothing is added in combat or for another scene.
    pub(super) fn apply(&self, message: &mut rpc::ViewMessage) {
        let Some(inner) = &self.inner else {
            return;
        };
        let Some(stage) = stage_of(message) else {
            return;
        };
        inner.apply(&stage, message);
    }
}

impl Inner {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn observe(&self, stage: Stage) {
        self.observe_at(stage, self.now());
    }

    fn observe_at(&self, stage: Stage, now: Duration) {
        let mut state = locked(&self.state);
        let changed_scene = state
            .stage
            .as_ref()
            .is_none_or(|known| known.scene_key != stage.scene_key);
        let mut had_overlay = false;
        if changed_scene {
            had_overlay = state.backdrop.is_some() || !state.sprites.is_empty();
            state.facts = Some(facts_of(&stage, &self.social));
            state.tracker.reset_scene();
            state.pacer.begin_scene(now);
            state.backdrop = None;
            state.sprites.clear();
            state.ready_backdrop = None;
            state.failed_faces.clear();
            state.last_revision = stage.revision;
        } else if stage.revision != state.last_revision {
            state.last_revision = stage.revision;
            state.pacer.note_action(now);
        }
        state.stage = Some(stage);
        drop(state);
        if had_overlay {
            self.wake.send_modify(|count| *count += 1);
        }
    }

    fn apply(&self, stage: &Stage, message: &mut rpc::ViewMessage) {
        let state = locked(&self.state);
        let current = state
            .stage
            .as_ref()
            .is_some_and(|known| known.scene_key == stage.scene_key);
        if !current || stage.in_combat {
            return;
        }
        let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
            return;
        };
        let Some(scene) = display.scene.as_mut() else {
            return;
        };
        if let Some(backdrop) = &state.backdrop {
            scene.live_backdrop = backdrop.key.path();
        }
        for actor in &mut scene.cast {
            if let Some(overlay) = state.sprites.get(&actor.id)
                && overlay.source_sprite == actor.sprite
            {
                actor.live_sprite = overlay.key.path();
            }
        }
    }

    /// The worker: ticks, follows the social ledger, and owns every render task it starts.
    async fn run(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(self.timing.tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut exchanges = self.social.subscribe_exchanges();
        let mut tasks: JoinSet<Done> = JoinSet::new();
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = exchanges.changed() => {}
                Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                    // A render task that panicked or was cancelled has nothing to report.
                    if let Ok(done) = done {
                        self.finish(done);
                    }
                }
            }
            for job in self.evaluate() {
                self.spawn(&mut tasks, job);
            }
        }
    }

    fn spawn(self: &Arc<Self>, tasks: &mut JoinSet<Done>, job: Job) {
        let inner = Arc::clone(self);
        match job {
            Job::Expression { member, face } => {
                tasks.spawn(async move {
                    let result = inner.render_expression(&member, face).await;
                    Done::Expression {
                        npc_id: member.id,
                        face,
                        source_sprite: member.sprite,
                        result,
                    }
                });
            }
            Job::Refresh {
                place,
                scene_asset,
                look,
                reason,
            } => {
                let scene_key = locked(&self.state)
                    .stage
                    .as_ref()
                    .map(|stage| stage.scene_key.clone())
                    .unwrap_or_default();
                tasks.spawn(async move {
                    let result = inner.render_refresh(&place, &scene_asset, look).await;
                    Done::Refresh {
                        look,
                        reason,
                        scene_key,
                        result,
                    }
                });
            }
        }
    }

    async fn load(&self, name: String) -> Result<ReferenceImage, ImageFailure> {
        let references = self.references.clone();
        tokio::task::spawn_blocking(move || references.load(&name))
            .await
            .map_err(|_| ImageFailure::Interrupted)?
            .map_err(|_| ImageFailure::InvalidRequest)
    }

    /// The art the stage shows for this character: the full-body twin of the cut-out where there
    /// is one (every non-player character stands with their feet on screen), otherwise the
    /// cut-out. The expression is made from that art so it replaces it in place, floor contact
    /// and framing included.
    async fn load_figure(&self, sprite: &str) -> Result<ReferenceImage, ImageFailure> {
        let stem = sprite.rsplit('/').next().unwrap_or(sprite);
        if let Some(name) = stem.strip_prefix("sprite-")
            && let Ok(figure) = self.load(format!("figure-{name}")).await
        {
            return Ok(figure);
        }
        self.load(sprite.to_owned()).await
    }

    async fn render_expression(
        &self,
        member: &Member,
        face: Face,
    ) -> Result<ImageKey, ImageFailure> {
        let sprite = self.load_figure(&member.sprite).await?;
        let prompt = cast_expression(
            self.rating,
            CharacterFacts {
                role: member.role,
                life_stage: member.life_stage,
            },
            face.expression,
            face.tier,
            // No character's art derives from a real person's likeness today.
            false,
        )
        .map_err(|_| ImageFailure::InvalidRequest)?;
        self.imagery
            .render(ImageRequest {
                kind: ImageKind::CastExpression,
                references: vec![sprite],
                prompt,
                output: OutputSpec::CUTOUT,
            })
            .await
    }

    async fn render_refresh(
        &self,
        place: &PlaceFacts,
        scene_asset: &str,
        look: Look,
    ) -> Result<ImageKey, ImageFailure> {
        let backdrop = self.load(scene_asset.to_owned()).await?;
        self.imagery
            .render(ImageRequest {
                kind: ImageKind::SceneRefresh,
                references: vec![backdrop],
                prompt: scene_refresh(self.rating, place, look),
                output: OutputSpec::BACKDROP,
            })
            .await
    }

    /// Records a finished render. A scene refresh waits for a quiet moment before it lands.
    fn finish(&self, done: Done) {
        let (kind, outcome) = match &done {
            Done::Expression { result, .. } => ("expression", result),
            Done::Refresh { result, .. } => ("refresh", result),
        };
        record(
            "gameplay.living_scene.render_finished",
            &[
                ("scene.kind", kind.to_owned()),
                (
                    "scene.outcome",
                    outcome
                        .as_ref()
                        .map_or_else(|failure| failure.label(), |_| "ready")
                        .to_owned(),
                ),
            ],
        );
        let mut state = locked(&self.state);
        match done {
            Done::Expression {
                npc_id,
                face,
                source_sprite,
                result,
            } => {
                state.expression_in_flight.remove(&npc_id);
                match result {
                    Ok(key) => {
                        let current = state.stage.as_ref().is_some_and(|stage| {
                            stage
                                .cast
                                .iter()
                                .any(|cast| cast.id == npc_id && cast.sprite == source_sprite)
                        });
                        if current {
                            state
                                .sprites
                                .insert(npc_id.clone(), SpriteOverlay { source_sprite, key });
                            state.tracker.mark_face_shown(&npc_id, face);
                            drop(state);
                            self.wake.send_modify(|count| *count += 1);
                        }
                    }
                    Err(failure) => {
                        state.failed_faces.insert((npc_id, face));
                        if failure == ImageFailure::BudgetExhausted {
                            state.budget_exhausted = true;
                        }
                    }
                }
            }
            Done::Refresh {
                look,
                reason,
                scene_key,
                result,
            } => {
                state.refresh_in_flight = false;
                let current = state
                    .stage
                    .as_ref()
                    .is_some_and(|stage| stage.scene_key == scene_key);
                match result {
                    Ok(key) if current => {
                        state.ready_backdrop = Some(ReadyBackdrop {
                            key,
                            look,
                            alternate: reason == RefreshReason::AlternateAngle,
                        });
                    }
                    Ok(_) => {}
                    Err(ImageFailure::BudgetExhausted) => state.budget_exhausted = true,
                    Err(_) => {}
                }
            }
        }
    }

    /// One decision pass: lands a ready backdrop when quiet, reverts to the authored art when
    /// the mood has calmed, and returns the renders to start.
    fn evaluate(&self) -> Vec<Job> {
        self.evaluate_at(self.now())
    }

    /// [`Self::evaluate`] at an explicit time, so tests control the clock.
    fn evaluate_at(&self, now: Duration) -> Vec<Job> {
        let mut state = locked(&self.state);
        let (Some(stage), Some(facts)) = (state.stage.clone(), state.facts.clone()) else {
            return Vec::new();
        };
        let in_combat = stage.in_combat;
        let mut wake = false;
        if let Some(ready) = state.ready_backdrop.take() {
            if state.pacer.may_publish(now, in_combat) {
                state.backdrop = Some(BackdropOverlay { key: ready.key });
                if ready.alternate {
                    // An alternate angle is not owed again: the mood is already on screen.
                    let target = state.tracker.shown_look();
                    state.tracker.mark_scene_shown(Look {
                        shot: ready.look.shot,
                        ..target
                    });
                } else {
                    state.tracker.mark_scene_shown(ready.look);
                }
                wake = true;
            } else {
                state.ready_backdrop = Some(ready);
            }
        }
        let cast: Vec<CastState> = facts
            .members
            .iter()
            .filter(|member| stage.cast.iter().any(|cast| cast.id == member.id))
            .map(|member| CastState {
                id: member.id.clone(),
                attitude: member.starting_attitude,
                opposes: member.opposes,
                speaking: stage.speaking_ids.contains(&member.id),
                tier: stage
                    .hurt
                    .iter()
                    .find(|(name, _, _)| *name == member.name)
                    .map_or(HealthTier::Healthy, |(_, current, maximum)| {
                        HealthTier::from_hit_points(*current, *maximum)
                    }),
            })
            .collect();
        // The ledger names a dossier ("patron:merchant"); the cast names its member by id.
        let exchange = self.social.latest_exchange().map(|record| ExchangeSignal {
            sequence: record.sequence,
            npc_id: facts
                .members
                .iter()
                .find(|member| member.dossier_id.as_deref() == Some(record.npc_id.as_str()))
                .map_or(record.npc_id, |member| member.id.clone()),
            approach: record.approach,
            effect: record.effect,
            attitude: record.attitude,
            opposes: record.opposes,
        });
        if exchange
            .as_ref()
            .is_some_and(|signal| signal.sequence > state.tracker_handled())
        {
            state.pacer.note_story_event(now);
        }
        let inputs = MoodInputs {
            tempo: stage.tempo,
            tone: facts.tone,
            beat: facts.beat,
            cast: &cast,
            exchange: exchange.as_ref(),
            recent_kinds: &stage.recent_kinds,
        };
        let verdict = state.tracker.update(now, &inputs);
        let mut jobs = Vec::new();
        if !in_combat {
            for (id, face) in &verdict.face_changes {
                if *face == Face::AUTHORED {
                    // The authored cut-out is the neutral, unhurt face: reverting costs nothing.
                    if state.sprites.remove(id).is_some() {
                        wake = true;
                    }
                    state.tracker.mark_face_shown(id, Face::AUTHORED);
                    continue;
                }
                let Some(member) = facts.members.iter().find(|member| member.id == *id) else {
                    continue;
                };
                if state.expression_in_flight.contains(id)
                    || state.failed_faces.contains(&(id.clone(), *face))
                    || state.budget_exhausted
                {
                    continue;
                }
                state.expression_in_flight.insert(id.clone());
                record(
                    "gameplay.living_scene.face_requested",
                    &[
                        ("scene.expression", face.expression.label().to_owned()),
                        ("scene.tier", face.tier.label().to_owned()),
                    ],
                );
                jobs.push(Job::Expression {
                    member: member.clone(),
                    face: *face,
                });
            }
        }
        let authored = verdict.look == Look::AUTHORED;
        if authored && verdict.scene_worth_rendering && state.pacer.may_publish(now, in_combat) {
            // The mood has calmed back to the authored framing: show the authored painting.
            if state.backdrop.take().is_some() {
                wake = true;
            }
            state.ready_backdrop = None;
            state.tracker.mark_scene_shown(Look::AUTHORED);
        } else if let Some(place) = &facts.place
            && !state.refresh_in_flight
            && state.ready_backdrop.is_none()
            && !state.budget_exhausted
        {
            let worth = verdict.scene_worth_rendering;
            if let Some(reason) = state.pacer.refresh_due(now, in_combat, worth) {
                let look = match reason {
                    RefreshReason::Change => verdict.look,
                    RefreshReason::AlternateAngle => {
                        alternate_look(verdict.look, state.pacer.alternates())
                    }
                };
                state.pacer.refresh_started(now, reason);
                state.refresh_in_flight = true;
                record(
                    "gameplay.living_scene.refresh_requested",
                    &[
                        ("scene.mood", look.mood.label().to_owned()),
                        ("scene.shot", look.shot.label().to_owned()),
                        ("scene.lighting", look.lighting.label().to_owned()),
                        (
                            "scene.reason",
                            match reason {
                                RefreshReason::Change => "change",
                                RefreshReason::AlternateAngle => "alternate_angle",
                            }
                            .to_owned(),
                        ),
                    ],
                );
                jobs.push(Job::Refresh {
                    place: place.clone(),
                    scene_asset: stage.scene_asset.clone(),
                    look,
                    reason,
                });
            }
        }
        drop(state);
        if wake {
            self.wake.send_modify(|count| *count += 1);
        }
        jobs
    }
}

impl State {
    fn new(timing: Timing) -> Self {
        Self {
            stage: None,
            facts: None,
            tracker: MoodTracker::new(timing.hysteresis),
            pacer: Pacer::new(timing, Duration::ZERO),
            backdrop: None,
            sprites: BTreeMap::new(),
            ready_backdrop: None,
            refresh_in_flight: false,
            expression_in_flight: BTreeSet::new(),
            failed_faces: BTreeSet::new(),
            last_revision: None,
            budget_exhausted: false,
        }
    }

    /// The newest exchange sequence the mood has already handled.
    fn tracker_handled(&self) -> u64 {
        self.tracker.handled_sequence()
    }
}

/// A gentle change of angle for a quiet scene: the same mood and light, another camera.
fn alternate_look(target: Look, alternates: usize) -> Look {
    const ANGLES: [super::scene_mood::Shot; 4] = [
        super::scene_mood::Shot::MediumConversation,
        super::scene_mood::Shot::LowAngle,
        super::scene_mood::Shot::HighAngle,
        super::scene_mood::Shot::OverShoulderTowardSpeaker,
    ];
    Look {
        shot: ANGLES[alternates % ANGLES.len()],
        ..target
    }
}

#[cfg(test)]
#[path = "living_scenes_tests.rs"]
mod tests;
