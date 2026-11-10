//! The tempo routes and the music director behind them.
//!
//! `Core` holds the loaded table, the journey's tempo run and what the director plays. Every
//! change of that state is a boundary: an impulse, `tempo tick`, `tempo reset` or a wake-up the
//! director scheduled when a change was held back by the minimum dwell. A boundary advances the
//! engine to the host clock, asks `cue::decide` which profile should play, and answers at once.
//! Nothing runs per frame and nothing waits for a composition: a profile whose track is not in the
//! library is skipped with an audit line, and `tempo prepare` is where tracks are made.
//!
//! A track change itself is slow (the fade takes seconds), so it is an order to the conductor, one
//! task that applies orders in turn and sleeps until the next scheduled wake-up. The conductor
//! starts with the first order and stops with the routes. An order carries the generation of the
//! run that made it, and a change of table or a reset makes older orders stale; a stale order is
//! dropped and logged.
//!
//! Audit lines (`TEMPO ...`): `table`, `reset`, `tick`, `impulse` (kind, strength, phase,
//! intensity, inertia, momentum, target, fatigue), `decision` (action, reason, the wanted and the
//! playing profile), `cue` (a finished track change), `cue_skipped`, `prepared` and
//! `refused verb=.. error=<key>`. The music itself writes its own `MUSIC` and `PLAYGROUND` lines.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use df_playground::scene::SceneChange;
use df_types::saturating_millis;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::cue::{Choice, Playing, decide};
use super::limits::{FADE_STEPS, ORDER_QUEUE_CAPACITY};
use super::request::{
    DecisionAction, DecisionReason, DecisionView, ImpulseRequest, PlayingView, PrepareReport,
    PrepareStatus, Prepared, RecentView, TableLoaded, TempoError, TempoView,
};
use super::run::{Moment, TempoRun};
use super::table::MusicTable;
use crate::blocking::run_owned;
use crate::dev_http::{parse_body, reply};
use crate::music::routes::held_files;
use crate::music::{MusicService, MusicSource};
use crate::playground::apply_scene_change;
use crate::state::AppState;

/// The addresses of the tempo dev API.
pub mod path {
    /// Where the journey's tempo stands and what the director plays (GET).
    pub const TEMPO: &str = "/dev/tempo";
    /// Posts one impulse (POST an `ImpulseRequest`).
    pub const IMPULSE: &str = "/dev/tempo/impulse";
    /// Moves the engine to now and lets the director decide (POST, no body).
    pub const TICK: &str = "/dev/tempo/tick";
    /// Starts the journey again and lets the director play the base (POST, no body).
    pub const RESET: &str = "/dev/tempo/reset";
    /// Loads a music table (POST the table file).
    pub const TABLE: &str = "/dev/tempo/table";
    /// Makes or finds every profile's track (POST, no body).
    pub const PREPARE: &str = "/dev/tempo/prepare";
}

/// The track that is audible: what the last volume change set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Audible {
    pub(super) file: String,
    pub(super) volume_percent: u8,
}

/// One volume change of a track change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FadeStep {
    pub(super) file: String,
    pub(super) volume_percent: u8,
    /// How long to wait before the next step.
    pub(super) wait_ms: u64,
}

/// The volume changes that take the music from `from` to `to_file` at `to_volume`. The page plays
/// one track at a time, so the old track dips out in steps, the new one starts quietly and rises:
/// `crossfade_ms` is the whole of it. The same track, or no crossfade, is one change.
pub(super) fn fade_plan(
    from: Option<&Audible>,
    to_file: &str,
    to_volume: u8,
    crossfade_ms: u32,
) -> Vec<FadeStep> {
    let level = |volume: u8, step: u32| {
        u8::try_from(u32::from(volume) * step / FADE_STEPS).unwrap_or(volume)
    };
    let step = |file: &str, volume_percent: u8| FadeStep {
        file: file.to_owned(),
        volume_percent,
        wait_ms: 0,
    };
    if crossfade_ms == 0 || from.is_some_and(|audible| audible.file == to_file) {
        return vec![step(to_file, to_volume)];
    }
    let mut steps = Vec::new();
    if let Some(audible) = from {
        for dip in (1..FADE_STEPS).rev() {
            let volume = level(audible.volume_percent, dip);
            if volume > 0 {
                steps.push(step(&audible.file, volume));
            }
        }
    }
    let quiet = level(to_volume, 1).max(u8::from(to_volume > 0));
    steps.push(step(to_file, quiet));
    steps.extend((2..=FADE_STEPS).map(|rise| step(to_file, level(to_volume, rise))));
    let wait_ms = u64::from(crossfade_ms) / u64::try_from(steps.len()).unwrap_or(1);
    let last = steps.len().saturating_sub(1);
    for fade in steps.iter_mut().take(last) {
        fade.wait_ms = wait_ms;
    }
    steps
}

/// A track change the director ordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Switch {
    generation: u64,
    to: usize,
    file: String,
}

/// What the conductor is told.
enum Order {
    Switch(Switch),
    /// Decide again at this time (a change was held back by the dwell).
    Wake(Instant),
}

/// The loaded table with its run and what the director plays.
struct Loaded {
    table: Arc<MusicTable>,
    run: TempoRun,
    /// The cue the director decided on.
    playing: Option<Playing>,
    /// The track the page was last told to play.
    audible: Option<Audible>,
    last: Option<DecisionView>,
}

#[derive(Default)]
struct Session {
    loaded: Option<Loaded>,
    /// Counts loads and resets; an order of an older one is stale.
    generation: u64,
    /// Where each profile's track came from at the last `tempo prepare`.
    sources: HashMap<String, MusicSource>,
}

/// What a boundary leaves to do after it has answered.
struct Boundary {
    view: TempoView,
    switch: Option<Switch>,
    wake_at: Instant,
}

/// What moves the run at a boundary.
enum Step {
    /// A scheduled boundary of the director.
    Wake,
    Tick,
    Impulse(ImpulseRequest),
    Reset,
}

/// The state and the work of the director.
struct Core {
    app: Arc<AppState>,
    music: MusicService,
    started: Instant,
    session: Mutex<Session>,
}

impl Core {
    fn session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The run's clock: the host's monotonic time since the routes were made.
    fn now_ms(&self) -> u64 {
        saturating_millis(self.started.elapsed())
    }

    fn table(&self) -> Result<Arc<MusicTable>, TempoError> {
        self.session()
            .loaded
            .as_ref()
            .map(|loaded| Arc::clone(&loaded.table))
            .ok_or(TempoError::NoTable)
    }

    /// Installs a checked table and starts a run at rest. What plays stays as it is until the
    /// next boundary.
    fn install(&self, table: MusicTable) -> Result<TableLoaded, TempoError> {
        let run = TempoRun::new(&table, self.now_ms())?;
        let loaded = TableLoaded {
            version: table.version,
            base: table.base.clone(),
            profiles: table.profiles.iter().map(|p| p.id.clone()).collect(),
            impulses: table
                .impulses
                .iter()
                .map(|rule| rule.kind.clone())
                .collect(),
        };
        self.app.record_line(format!(
            "TEMPO table version={} profiles={} impulses={} base={}",
            loaded.version,
            loaded.profiles.len(),
            loaded.impulses.len(),
            loaded.base
        ));
        let mut session = self.session();
        let audible = session.loaded.take().and_then(|loaded| loaded.audible);
        session.sources.clear();
        session.generation += 1;
        session.loaded = Some(Loaded {
            table: Arc::new(table),
            run,
            playing: None,
            audible,
            last: None,
        });
        Ok(loaded)
    }

    /// The tracks of the table's profiles that the library holds, in table order.
    async fn ready_files(&self, table: &MusicTable) -> Vec<Option<String>> {
        let names: Vec<String> = table
            .profiles
            .iter()
            .map(|profile| profile.track_name(&table.director))
            .collect();
        match held_files(&self.app, &names).await {
            Ok(files) => files,
            Err(error) => {
                self.app
                    .record_line(format!("TEMPO library_unreadable error={}", error.key()));
                vec![None; names.len()]
            }
        }
    }

    /// One boundary: advance the engine to now, decide, answer. The library is listed before the
    /// state is locked, and the lock is never held across an await.
    async fn boundary(&self, step: Step) -> Result<Boundary, TempoError> {
        let table = self.table()?;
        if let Step::Impulse(request) = &step {
            request.check()?;
            if table.impulse(&request.kind).is_none() {
                return Err(TempoError::UnknownImpulse {
                    kind: request.kind.clone(),
                });
            }
        }
        let files = self.ready_files(&table).await;
        let mut guard = self.session();
        // The clock is read under the lock, so concurrent boundaries step the run in order.
        let now_ms = self.now_ms();
        let session = &mut *guard;
        let loaded = session.loaded.as_mut().ok_or(TempoError::NoTable)?;
        let scheduled = matches!(step, Step::Wake);
        self.move_run(loaded, &mut session.generation, step, now_ms)?;
        let (switch, decision) = self.choose(loaded, session.generation, &files, now_ms, scheduled);
        let wake_at = self.next_wake(&loaded.table, &decision, now_ms);
        let view = view_of(loaded, &session.sources, now_ms);
        Ok(Boundary {
            view,
            switch,
            wake_at,
        })
    }

    /// Moves the run for `step` to `now_ms`, and writes the step's audit line (a scheduled wake
    /// writes none).
    fn move_run(
        &self,
        loaded: &mut Loaded,
        generation: &mut u64,
        step: Step,
        now_ms: u64,
    ) -> Result<(), TempoError> {
        match step {
            Step::Reset => {
                loaded.run = TempoRun::new(&loaded.table, now_ms)?;
                loaded.playing = None;
                loaded.last = None;
                *generation += 1;
                self.app.record_line(format!("TEMPO reset at_ms={now_ms}"));
            }
            Step::Impulse(request) => {
                let rule = loaded.table.impulse(&request.kind).ok_or_else(|| {
                    TempoError::UnknownImpulse {
                        kind: request.kind.clone(),
                    }
                })?;
                let strength = request.intensity.unwrap_or(1_000);
                let note_chars = request.note.as_ref().map_or(0, |note| note.chars().count());
                loaded
                    .run
                    .impulse(rule, strength, request.note.clone(), now_ms)?;
                self.app.record_line(format!(
                    "TEMPO impulse kind={} strength={strength} note_chars={note_chars} {}",
                    request.kind,
                    state_fields(&loaded.run.moment(now_ms))
                ));
            }
            Step::Tick => {
                loaded.run.advance(now_ms)?;
                self.app.record_line(format!(
                    "TEMPO tick {}",
                    state_fields(&loaded.run.moment(now_ms))
                ));
            }
            Step::Wake => loaded.run.advance(now_ms)?,
        }
        Ok(())
    }

    /// Decides what plays, records it, and orders the change if one is due. `scheduled` is a
    /// wake-up: it writes a line only when its decision differs from the last one.
    fn choose(
        &self,
        loaded: &mut Loaded,
        generation: u64,
        files: &[Option<String>],
        now_ms: u64,
        scheduled: bool,
    ) -> (Option<Switch>, DecisionView) {
        let ready: Vec<bool> = files.iter().map(Option::is_some).collect();
        let decision = decide(
            &loaded.table,
            &loaded.run.moment(now_ms),
            loaded.playing,
            &ready,
        );
        let wanted = loaded.table.profiles[decision.wanted].id.clone();
        let (action, reason, until_ms) = match decision.choice {
            Choice::Keep(reason) => (DecisionAction::Keep, reason, None),
            Choice::Switch(reason) => (DecisionAction::Switch, reason, None),
            Choice::Hold { until_ms } => {
                (DecisionAction::Hold, DecisionReason::Dwell, Some(until_ms))
            }
            Choice::Skip => (DecisionAction::Skip, DecisionReason::NotPrepared, None),
        };
        let view = DecisionView {
            action,
            reason,
            wanted: wanted.clone(),
            until_ms,
        };
        let repeated = loaded.last.as_ref().is_some_and(|last| {
            (last.action, last.reason, &last.wanted) == (action, reason, &view.wanted)
        });
        if !(scheduled && repeated) {
            self.record_decision(loaded, &view, now_ms);
        }
        let mut switch = None;
        if let Choice::Switch(_) = decision.choice {
            loaded.playing = Some(Playing {
                index: decision.wanted,
                since_ms: now_ms,
            });
            // A switch is only chosen for a profile whose track is ready.
            if let Some(file) = files.get(decision.wanted).cloned().flatten() {
                switch = Some(Switch {
                    generation,
                    to: decision.wanted,
                    file,
                });
            }
        }
        loaded.last = Some(view.clone());
        (switch, view)
    }

    fn record_decision(&self, loaded: &Loaded, view: &DecisionView, now_ms: u64) {
        let playing = loaded.playing.map_or("none", |playing| {
            loaded.table.profiles[playing.index].id.as_str()
        });
        self.app.record_line(format!(
            "TEMPO decision action={} reason={} wanted={} playing={playing} at_ms={now_ms}",
            view.action.key(),
            view.reason.key(),
            view.wanted
        ));
        if view.action == DecisionAction::Skip {
            self.app.record_line(format!(
                "TEMPO cue_skipped profile={} reason=not_prepared",
                view.wanted
            ));
        }
    }

    /// When the director decides next: at the end of the dwell if a change waits for it, else at
    /// the table's tick. This is the only schedule: nothing runs per frame.
    fn next_wake(&self, table: &MusicTable, decision: &DecisionView, now_ms: u64) -> Instant {
        let tick_ms = table.director.tick_ms;
        let wait_ms = decision.until_ms.map_or(tick_ms, |until| {
            until.saturating_sub(now_ms).clamp(1, tick_ms)
        });
        self.started + Duration::from_millis(now_ms.saturating_add(wait_ms))
    }

    /// The tempo as it would stand now, without deciding or changing anything.
    fn state(&self) -> Result<TempoView, TempoError> {
        let guard = self.session();
        let now_ms = self.now_ms();
        let loaded = guard.loaded.as_ref().ok_or(TempoError::NoTable)?;
        let mut shown = loaded.run.clone();
        shown.advance(now_ms)?;
        Ok(view_with(loaded, &shown, &guard.sources, now_ms))
    }

    /// Makes or finds every profile's track, one after another.
    async fn prepare(&self) -> Result<PrepareReport, TempoError> {
        let table = self.table()?;
        let mut report = PrepareReport {
            profiles: Vec::new(),
            reused: 0,
            composed: 0,
            failed: 0,
            composed_seconds: 0,
        };
        for profile in &table.profiles {
            let request = profile.music_request(&table.director);
            let status = match self.music.ensure(&self.app, &request).await {
                Ok(ensured) => {
                    self.session()
                        .sources
                        .insert(profile.id.clone(), ensured.source);
                    match (ensured.source, ensured.compose_ms) {
                        (MusicSource::Composed, Some(compose_ms)) => {
                            report.composed += 1;
                            report.composed_seconds += profile.length_seconds;
                            PrepareStatus::Composed {
                                bytes: ensured.bytes,
                                compose_ms,
                            }
                        }
                        _ => {
                            report.reused += 1;
                            PrepareStatus::Reused {
                                bytes: ensured.bytes,
                            }
                        }
                    }
                }
                Err(error) => {
                    report.failed += 1;
                    PrepareStatus::Failed { error }
                }
            };
            report.profiles.push(Prepared {
                profile: profile.id.clone(),
                track: request.name,
                length_seconds: profile.length_seconds,
                status,
            });
        }
        self.app.record_line(format!(
            "TEMPO prepared profiles={} reused={} composed={} failed={} composed_seconds={}",
            report.profiles.len(),
            report.reused,
            report.composed,
            report.failed,
            report.composed_seconds
        ));
        Ok(report)
    }

    /// Carries out a track change: the volume steps of `fade_plan`, each a scene change, with the
    /// waits between. A stale order, or a step the scene refuses, ends it where it is.
    async fn apply(&self, order: Switch) {
        let Some((profile, table, audible)) = self.change_of(&order) else {
            self.app.record_line(format!(
                "TEMPO cue_dropped reason=stale generation={}",
                order.generation
            ));
            return;
        };
        let steps = fade_plan(
            audible.as_ref(),
            &order.file,
            profile.volume_percent,
            profile.crossfade_ms,
        );
        let step_count = steps.len();
        for (index, step) in steps.into_iter().enumerate() {
            if self.session().generation != order.generation {
                self.app.record_line(format!(
                    "TEMPO cue_dropped reason=stale profile={} step={index}",
                    profile.id
                ));
                return;
            }
            let change = SceneChange::Music {
                media: step.file.clone(),
                target: table.director.target,
                volume_percent: step.volume_percent,
                looping: profile.looping,
            };
            if apply_scene_change(&self.app, change).await.is_err() {
                self.app.record_line(format!(
                    "TEMPO cue_failed profile={} step={index} error=scene",
                    profile.id
                ));
                return;
            }
            self.set_audible(&step);
            if index + 1 < step_count {
                tokio::time::sleep(Duration::from_millis(step.wait_ms)).await;
            }
        }
        self.app.record_line(format!(
            "TEMPO cue profile={} track={} volume={} crossfade_ms={} steps={step_count} from={}",
            profile.id,
            order.file,
            profile.volume_percent,
            profile.crossfade_ms,
            audible
                .as_ref()
                .map_or("none", |audible| audible.file.as_str())
        ));
    }

    /// The profile, table and audible track of a track change, or none when the order is stale.
    fn change_of(
        &self,
        order: &Switch,
    ) -> Option<(super::table::Profile, Arc<MusicTable>, Option<Audible>)> {
        let session = self.session();
        let loaded = session.loaded.as_ref()?;
        if session.generation != order.generation {
            return None;
        }
        let profile = loaded.table.profiles.get(order.to)?.clone();
        Some((profile, Arc::clone(&loaded.table), loaded.audible.clone()))
    }

    fn set_audible(&self, step: &FadeStep) {
        if let Some(loaded) = self.session().loaded.as_mut() {
            loaded.audible = Some(Audible {
                file: step.file.clone(),
                volume_percent: step.volume_percent,
            });
        }
    }
}

/// The tempo's numbers for an audit line.
fn state_fields(moment: &Moment<'_>) -> String {
    format!(
        "phase={} intensity={} inertia={} momentum={} target={} fatigue={} at_ms={}",
        moment.phase.key(),
        moment.intensity,
        moment.inertia,
        moment.momentum.key(),
        moment.target,
        moment.fatigue,
        moment.now_ms
    )
}

fn view_of(loaded: &Loaded, sources: &HashMap<String, MusicSource>, now_ms: u64) -> TempoView {
    view_with(loaded, &loaded.run, sources, now_ms)
}

fn view_with(
    loaded: &Loaded,
    run: &TempoRun,
    sources: &HashMap<String, MusicSource>,
    now_ms: u64,
) -> TempoView {
    let moment = run.moment(now_ms);
    let table = &loaded.table;
    let playing = loaded.playing.and_then(|playing| {
        let profile = table.profiles.get(playing.index)?;
        Some(PlayingView {
            profile: profile.id.clone(),
            track: profile.track_name(&table.director),
            source: sources
                .get(&profile.id)
                .copied()
                .unwrap_or(MusicSource::Library),
            since_ms: playing.since_ms,
            volume_percent: profile.volume_percent,
        })
    });
    TempoView {
        policy: format!("tempo/{}", table.version),
        policy_version: table.version,
        now_ms,
        phase: moment.phase,
        intensity: moment.intensity,
        inertia: moment.inertia,
        momentum: moment.momentum,
        target: moment.target,
        fatigue: moment.fatigue,
        recent: moment
            .recent
            .iter()
            .map(|impulse| RecentView {
                kind: impulse.kind.clone(),
                intensity: impulse.intensity,
                at_ms: impulse.at_ms,
                note: impulse.note.clone(),
            })
            .collect(),
        playing,
        decision: loaded.last.clone(),
    }
}

/// The task that carries out orders in turn and keeps the next wake-up.
struct Conductor {
    orders: mpsc::Sender<Order>,
    task: JoinHandle<()>,
}

impl Conductor {
    fn start(core: Arc<Core>) -> Self {
        let (orders, inbox) = mpsc::channel(ORDER_QUEUE_CAPACITY);
        Self {
            orders,
            task: tokio::spawn(conduct(core, inbox)),
        }
    }

    /// Hands the boundary's leftover work to the task. A full queue drops the order and says so:
    /// the next boundary decides again.
    fn dispatch(&self, app: &AppState, boundary: &mut Boundary) {
        let orders = [
            boundary.switch.take().map(Order::Switch),
            Some(Order::Wake(boundary.wake_at)),
        ];
        for order in orders.into_iter().flatten() {
            if self.orders.try_send(order).is_err() {
                app.record_line("TEMPO order_dropped reason=queue_full".to_owned());
            }
        }
    }
}

impl Drop for Conductor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn conduct(core: Arc<Core>, mut inbox: mpsc::Receiver<Order>) {
    let mut wake: Option<Instant> = None;
    loop {
        let deadline = wake;
        let due = async move {
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            order = inbox.recv() => match order {
                None => break,
                Some(Order::Switch(switch)) => core.apply(switch).await,
                Some(Order::Wake(at)) => wake = Some(at),
            },
            () = due => {
                wake = core.wake().await;
            }
        }
    }
}

impl Core {
    /// A scheduled boundary: decide again, carry out a change at once, and say when to wake next.
    async fn wake(&self) -> Option<Instant> {
        match self.boundary(Step::Wake).await {
            Ok(boundary) => {
                if let Some(switch) = boundary.switch {
                    self.apply(switch).await;
                }
                Some(boundary.wake_at)
            }
            Err(error) => {
                self.app
                    .record_line(format!("TEMPO wake_failed error={}", error.key()));
                None
            }
        }
    }
}

struct TempoRoutes {
    core: Arc<Core>,
    conductor: OnceLock<Conductor>,
}

impl TempoRoutes {
    /// Runs a boundary and hands its leftover work to the conductor, started with the first.
    async fn step(&self, step: Step) -> Result<TempoView, TempoError> {
        let mut boundary = self.core.boundary(step).await?;
        let conductor = self
            .conductor
            .get_or_init(|| Conductor::start(Arc::clone(&self.core)));
        conductor.dispatch(&self.core.app, &mut boundary);
        Ok(boundary.view)
    }
}

/// Answers `result`, recording a refusal first as `TEMPO refused verb=.. error=<key>`.
fn answer<T: Serialize>(app: &AppState, verb: &str, result: Result<T, TempoError>) -> Response {
    if let Err(error) = &result {
        app.record_line(format!("TEMPO refused verb={verb} error={}", error.key()));
    }
    reply::<T, TempoError>(result)
}

async fn show(State(routes): State<Arc<TempoRoutes>>) -> Response {
    answer(&routes.core.app, "state", routes.core.state())
}

async fn impulse(State(routes): State<Arc<TempoRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<ImpulseRequest>(&bytes) {
        Ok(request) => routes.step(Step::Impulse(request)).await,
        Err(error) => Err(TempoError::InvalidBody(error)),
    };
    answer(&routes.core.app, "impulse", result)
}

async fn tick(State(routes): State<Arc<TempoRoutes>>) -> Response {
    let result = routes.step(Step::Tick).await;
    answer(&routes.core.app, "tick", result)
}

async fn restart(State(routes): State<Arc<TempoRoutes>>) -> Response {
    let result = routes.step(Step::Reset).await;
    answer(&routes.core.app, "reset", result)
}

async fn load(State(routes): State<Arc<TempoRoutes>>, bytes: Bytes) -> Response {
    let result = MusicTable::from_json(&bytes).and_then(|table| routes.core.install(table));
    answer(&routes.core.app, "table", result)
}

async fn prepare(State(routes): State<Arc<TempoRoutes>>) -> Response {
    let core = Arc::clone(&routes.core);
    let result = run_owned(async move { core.prepare().await }, TempoError::Stopped).await;
    answer(&routes.core.app, "prepare", result)
}

/// The tempo routes over `music`, the music engine's shared composer.
pub fn router(app: Arc<AppState>, music: MusicService) -> Router {
    let routes = Arc::new(TempoRoutes {
        core: Arc::new(Core {
            app,
            music,
            started: Instant::now(),
            session: Mutex::new(Session::default()),
        }),
        conductor: OnceLock::new(),
    });
    Router::new()
        .route(path::TEMPO, get(show))
        .route(path::IMPULSE, post(impulse))
        .route(path::TICK, post(tick))
        .route(path::RESET, post(restart))
        .route(path::TABLE, post(load))
        .route(path::PREPARE, post(prepare))
        .with_state(routes)
}
