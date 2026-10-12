//! Adaptive background music on the shared display only.
//!
//! The browser decides no story: for every accepted display view `music_score::tempo_of` turns
//! what the server already shows into a phase and intensity, and this module plays the
//! server-provided score (`/assets/music/<track>`) accordingly. Four sample-aligned loops
//! (bed, motif, tension, combat) start together on one Web Audio clock and only their gains
//! move, smoothly, as the tempo rises and releases. A location ambience loops beside them, and
//! one-shot stingers land on the next beat. Phones never reach this module.
//!
//! Sound follows the one shared state of `live_voice`: nothing is requested or heard before its
//! unlock gesture, its mute control silences the music too, and music ducks while a voice clip
//! plays. A hidden tab suspends the audio clock and, on return, the mix jumps to the present
//! rather than replaying what happened. The music volume (0 to 1) is read from
//! `sessionStorage` under `df.music.volume`. A debug snapshot of the mix is kept in the
//! `data-music-state` attribute of the display root.
use super::*;
use crate::music_score::{
    CueMemory, DUCK_ATTACK_SECONDS, DUCK_DECIBELS, DUCK_RELEASE_SECONDS, Facts, LAYER_TRACKS,
    LOOP_SECONDS, Outcome, Phase, Stinger, Tempo, effective_phase, facts_of, mix_for, next_beat,
    stinger_between, tempo_of,
};
use gloo_timers::callback::Interval;
use std::collections::{HashMap, HashSet, VecDeque};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, AudioParam,
    AudioScheduledSourceNode, BiquadFilterNode, BiquadFilterType, Event, GainNode, Response,
};

const MUSIC_PATH: &str = "/assets/music";
const VOLUME_KEY: &str = "df.music.volume";
/// Music clearly under the voice: the layers are mastered near -23 LUFS and the voice near -16.
const DEFAULT_VOLUME: f32 = 0.85;
const STATE_ATTRIBUTE: &str = "data-music-state";
const MAXIMUM_TRACK_BYTES: u32 = 6 * 1024 * 1024;
const TICK_MILLIS: u32 = 100;
/// Layers start this far ahead of the audio clock so all four are scheduled before they begin.
const START_LEAD_SECONDS: f64 = 0.15;
const MASTER_SECONDS: f64 = 0.4;
const FILTER_SECONDS: f64 = 2.0;
const BED_CROSSFADE_SECONDS: f64 = 3.0;
/// Mix jump after a hidden tab or a context restart: quick, but never a click.
const RESYNC_SECONDS: f64 = 0.5;
const AMBIENT_LEVEL: f32 = 0.5;
const STINGER_LEVEL: f32 = 0.8;
const MINIMUM_GAIN_STEP: f32 = 0.002;
const FIRED_MEMORY: usize = 8;

/// Everything the mixer sets at one moment; a change of any field starts a glide.
#[derive(Clone, Copy, PartialEq)]
struct Applied {
    layers: [f32; 4],
    ambient: f32,
    filter_hertz: f32,
    master: f32,
    duck: f32,
}

struct Ambient {
    track: &'static str,
    gain: GainNode,
    source: AudioBufferSourceNode,
}

/// The audio graph: layers -> low-pass -> duck -> master, with the ambience and stingers
/// joining at the duck so all of it yields to narration.
struct Graph {
    context: AudioContext,
    master: GainNode,
    duck: GainNode,
    filter: BiquadFilterNode,
    layers: [GainNode; 4],
    ambient_bus: GainNode,
    ambient: Option<Ambient>,
    /// Audio-clock time the four layers started together; the beat grid hangs from it.
    started_at: Option<f64>,
    /// Kept so the looping sources stay referenced for the page's lifetime.
    sources: Vec<AudioBufferSourceNode>,
}

struct Latest {
    tempo: Tempo,
    facts: Facts,
    release_since: Option<f64>,
}

struct Director {
    document: Document,
    root: Element,
    graph: Option<Graph>,
    buffers: HashMap<&'static str, AudioBuffer>,
    requested: HashSet<&'static str>,
    missing: Vec<&'static str>,
    latest: Option<Latest>,
    enabled: bool,
    speaking: bool,
    volume: f32,
    applied: Option<Applied>,
    resync: bool,
    fired: VecDeque<&'static str>,
    published: String,
    failure: Option<String>,
    ticker: Option<Interval>,
    /// The server tempo cues consumed on the current watch connection.
    cues: CueMemory,
    connection: u64,
    listeners: Vec<Closure<dyn FnMut(Event)>>,
}

type Shared = Rc<RefCell<Director>>;

thread_local! {static DIRECTOR:RefCell<Option<Shared>>=const{RefCell::new(None)};}

fn failure_text(error: &JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(error, &JsValue::from_str("message"))
                .ok()
                .and_then(|message| message.as_string())
        })
        .unwrap_or_else(|| "browser audio error".to_owned())
}

fn duck_gain() -> f32 {
    10_f32.powf(DUCK_DECIBELS / 20.0)
}

/// Starts a smooth change of `param` to `to`. The current value is pinned first so a ramp that
/// is interrupted by a newer one continues from where the sound actually is.
fn glide(
    param: &AudioParam,
    from: Option<f32>,
    to: f32,
    now: f64,
    rise: f64,
    fall: f64,
    resync: bool,
) -> Result<(), JsValue> {
    let unchanged = from.is_some_and(|from| (from - to).abs() < MINIMUM_GAIN_STEP);
    if unchanged && !resync {
        return Ok(());
    }
    let seconds = if resync {
        RESYNC_SECONDS
    } else if from.is_none_or(|from| to > from) {
        rise
    } else {
        fall
    };
    param.cancel_scheduled_values(now)?;
    param.set_value_at_time(param.value(), now)?;
    param.linear_ramp_to_value_at_time(to, now + seconds)?;
    Ok(())
}

fn silent_gain(
    context: &AudioContext,
    destination: &web_sys::AudioNode,
) -> Result<GainNode, JsValue> {
    let gain = context.create_gain()?;
    gain.gain().set_value(0.0);
    gain.connect_with_audio_node(destination)?;
    Ok(gain)
}

/// A looping source into `destination`. `loop_seconds` bounds the loop to the exact bar grid;
/// a source without it loops its whole buffer.
fn looping_source(
    context: &AudioContext,
    buffer: &AudioBuffer,
    destination: &GainNode,
    loop_seconds: Option<f64>,
) -> Result<AudioBufferSourceNode, JsValue> {
    let source = context.create_buffer_source()?;
    source.set_buffer(Some(buffer));
    source.set_loop(true);
    if let Some(seconds) = loop_seconds {
        source.set_loop_start(0.0);
        source.set_loop_end(seconds);
    }
    source.connect_with_audio_node(destination)?;
    Ok(source)
}

impl Graph {
    fn new() -> Result<Self, JsValue> {
        let context = AudioContext::new()?;
        let master = context.create_gain()?;
        master.gain().set_value(0.0);
        master.connect_with_audio_node(&context.destination())?;
        let duck = context.create_gain()?;
        duck.connect_with_audio_node(&master)?;
        let filter = context.create_biquad_filter()?;
        filter.set_type(BiquadFilterType::Lowpass);
        filter.frequency().set_value(1_400.0);
        filter.connect_with_audio_node(&duck)?;
        let layers = [
            silent_gain(&context, &filter)?,
            silent_gain(&context, &filter)?,
            silent_gain(&context, &filter)?,
            silent_gain(&context, &filter)?,
        ];
        let ambient_bus = silent_gain(&context, &duck)?;
        Ok(Self {
            context,
            master,
            duck,
            filter,
            layers,
            ambient_bus,
            ambient: None,
            started_at: None,
            sources: Vec::new(),
        })
    }

    fn state_name(&self) -> &'static str {
        match self.context.state() {
            AudioContextState::Running => "running",
            AudioContextState::Suspended => "suspended",
            AudioContextState::Closed => "closed",
            _ => "unknown",
        }
    }
}

/// Fire-and-forget on a browser promise whose outcome is visible as the context state
/// (`resume` can be refused by the autoplay policy; the next tick simply asks again).
fn settle(promise: js_sys::Promise) {
    spawn_local(async move {
        let _settled = JsFuture::from(promise).await;
    });
}

impl Director {
    fn fail(&mut self, error: &JsValue) {
        self.failure = Some(failure_text(error));
    }

    /// `connection` identifies the watch connection the view came from; a new one starts a new
    /// server cue sequence, so the consumed sequence is forgotten.
    fn accept(&mut self, tempo: Tempo, facts: Facts, connection: u64) {
        if connection != self.connection {
            self.connection = connection;
            self.cues.reset();
        }
        let previous = self.latest.take();
        // A cue already consumed (the same one on a later revision, or a stale one) changes
        // nothing: the tempo that is playing stays, and its release hold keeps running.
        let news = self.cues.consume(&tempo);
        let tempo = match previous.as_ref() {
            Some(previous) if !news => previous.tempo,
            _ => tempo,
        };
        let release_since = (tempo.phase == Phase::Release).then(|| {
            previous
                .as_ref()
                .filter(|previous| previous.tempo.phase == Phase::Release)
                .and_then(|previous| previous.release_since)
                .unwrap_or_else(js_sys::Date::now)
        });
        // A first view after a reconnect is a state, not a story change: no stinger for it.
        let stinger = previous
            .as_ref()
            .and_then(|previous| stinger_between(&previous.facts, &facts));
        self.latest = Some(Latest {
            tempo,
            facts,
            release_since,
        });
        if let Some(stinger) = stinger
            && let Err(error) = self.play_stinger(stinger)
        {
            self.fail(&error);
        }
    }

    /// One pass of housekeeping: shared sound state, tab visibility, the audio graph, the
    /// mix and the debug snapshot. Returns the tracks that now need fetching.
    fn step(&mut self) -> Result<Vec<&'static str>, JsValue> {
        if let Some(sound) = live_voice::sound_state() {
            self.enabled = sound.enabled;
            self.speaking = sound.speaking;
        }
        self.volume = stored_volume();
        let hidden = self.document.hidden();
        if self.enabled && self.graph.is_none() {
            self.graph = Some(Graph::new()?);
        }
        let Some(graph) = self.graph.as_ref() else {
            self.publish();
            return Ok(Vec::new());
        };
        let state = graph.context.state();
        if hidden {
            if state == AudioContextState::Running {
                settle(graph.context.suspend()?);
            }
            self.publish();
            return Ok(Vec::new());
        }
        if self.enabled && state == AudioContextState::Suspended {
            settle(graph.context.resume()?);
            self.resync = true;
        }
        let wanted = self.tracks_to_fetch();
        self.try_start_layers()?;
        self.apply_bed()?;
        self.retarget()?;
        self.publish();
        Ok(wanted)
    }

    /// Layers first (they start together), then the location ambience, then the stingers.
    fn tracks_to_fetch(&mut self) -> Vec<&'static str> {
        if !self.enabled {
            return Vec::new();
        }
        let bed = self.latest.as_ref().map(|latest| latest.facts.bed.track());
        let wanted = LAYER_TRACKS
            .into_iter()
            .chain(bed)
            .chain(Stinger::ALL.map(Stinger::track));
        let mut fresh = Vec::new();
        for track in wanted {
            if self.requested.insert(track) {
                fresh.push(track);
            }
        }
        fresh
    }

    fn try_start_layers(&mut self) -> Result<(), JsValue> {
        let Some(graph) = self.graph.as_mut() else {
            return Ok(());
        };
        let settled = LAYER_TRACKS
            .iter()
            .all(|track| self.buffers.contains_key(track) || self.missing.contains(track));
        if graph.started_at.is_some() || !self.enabled || !settled {
            return Ok(());
        }
        let when = graph.context.current_time() + START_LEAD_SECONDS;
        for (index, track) in LAYER_TRACKS.iter().enumerate() {
            let Some(buffer) = self.buffers.get(track) else {
                continue;
            };
            let source = looping_source(
                &graph.context,
                buffer,
                &graph.layers[index],
                Some(LOOP_SECONDS),
            )?;
            source.start_with_when(when)?;
            graph.sources.push(source);
        }
        graph.started_at = Some(when);
        Ok(())
    }

    /// Crossfades to the ambience of the current location once its track is loaded.
    fn apply_bed(&mut self) -> Result<(), JsValue> {
        let (Some(graph), Some(latest)) = (self.graph.as_mut(), self.latest.as_ref()) else {
            return Ok(());
        };
        let wanted = latest.facts.bed.track();
        let playing = graph
            .ambient
            .as_ref()
            .is_some_and(|ambient| ambient.track == wanted);
        let Some(buffer) = self.buffers.get(wanted) else {
            return Ok(());
        };
        if playing || !self.enabled {
            return Ok(());
        }
        let now = graph.context.current_time();
        let gain = silent_gain(&graph.context, &graph.ambient_bus)?;
        let source = looping_source(&graph.context, buffer, &gain, None)?;
        source.start_with_when(now)?;
        glide(
            &gain.gain(),
            Some(0.0),
            1.0,
            now,
            BED_CROSSFADE_SECONDS,
            BED_CROSSFADE_SECONDS,
            false,
        )?;
        if let Some(old) = graph.ambient.take() {
            glide(
                &old.gain.gain(),
                Some(1.0),
                0.0,
                now,
                BED_CROSSFADE_SECONDS,
                BED_CROSSFADE_SECONDS,
                false,
            )?;
            AudioScheduledSourceNode::stop_with_when(
                &old.source,
                now + BED_CROSSFADE_SECONDS + 0.1,
            )?;
        }
        graph.ambient = Some(Ambient {
            track: wanted,
            gain,
            source,
        });
        Ok(())
    }

    /// Glides every layer, the ambience, the filter, the duck and the master toward the mix the
    /// current tempo asks for.
    fn retarget(&mut self) -> Result<(), JsValue> {
        let (Some(graph), Some(latest)) = (self.graph.as_ref(), self.latest.as_ref()) else {
            return Ok(());
        };
        let held = latest
            .release_since
            .map_or(0.0, |since| js_sys::Date::now() - since);
        let phase = effective_phase(latest.tempo, held);
        let sombre = latest.facts.outcome == Some(Outcome::Defeat);
        let mix = mix_for(phase, latest.tempo.intensity, sombre);
        let target = Applied {
            layers: if graph.started_at.is_some() {
                mix.layers.in_track_order()
            } else {
                [0.0; 4]
            },
            ambient: mix.ambient * AMBIENT_LEVEL,
            filter_hertz: mix.filter_hertz,
            master: if self.enabled { self.volume } else { 0.0 },
            duck: if self.speaking { duck_gain() } else { 1.0 },
        };
        let previous = self.applied;
        if previous == Some(target) && !self.resync {
            return Ok(());
        }
        let now = graph.context.current_time();
        let resync = self.resync;
        for (index, layer) in graph.layers.iter().enumerate() {
            glide(
                &layer.gain(),
                previous.map(|applied| applied.layers[index]),
                target.layers[index],
                now,
                mix.rise_seconds,
                mix.fall_seconds,
                resync,
            )?;
        }
        glide(
            &graph.ambient_bus.gain(),
            previous.map(|applied| applied.ambient),
            target.ambient,
            now,
            mix.rise_seconds,
            mix.fall_seconds,
            resync,
        )?;
        glide(
            &graph.filter.frequency(),
            previous.map(|applied| applied.filter_hertz),
            target.filter_hertz,
            now,
            FILTER_SECONDS,
            FILTER_SECONDS,
            resync,
        )?;
        glide(
            &graph.master.gain(),
            previous.map(|applied| applied.master),
            target.master,
            now,
            MASTER_SECONDS,
            MASTER_SECONDS,
            resync,
        )?;
        glide(
            &graph.duck.gain(),
            previous.map(|applied| applied.duck),
            target.duck,
            now,
            DUCK_RELEASE_SECONDS,
            DUCK_ATTACK_SECONDS,
            resync,
        )?;
        self.applied = Some(target);
        self.resync = false;
        Ok(())
    }

    /// Plays a stinger on the next beat. One that cannot play now (sound off, hidden tab,
    /// track not loaded) is skipped, never queued: an old flourish would be wrong later.
    fn play_stinger(&mut self, stinger: Stinger) -> Result<(), JsValue> {
        let Some(graph) = self.graph.as_ref() else {
            return Ok(());
        };
        let track = stinger.track();
        let Some(buffer) = self.buffers.get(track) else {
            return Ok(());
        };
        if !self.enabled
            || self.document.hidden()
            || graph.context.state() != AudioContextState::Running
        {
            return Ok(());
        }
        let when = next_beat(graph.context.current_time(), graph.started_at);
        let gain = graph.context.create_gain()?;
        gain.gain().set_value(STINGER_LEVEL);
        gain.connect_with_audio_node(&graph.duck)?;
        let source = graph.context.create_buffer_source()?;
        source.set_buffer(Some(buffer));
        source.connect_with_audio_node(&gain)?;
        source.start_with_when(when)?;
        if self.fired.len() >= FIRED_MEMORY {
            self.fired.pop_front();
        }
        self.fired.push_back(track);
        Ok(())
    }

    /// The read-only debug snapshot: phase, intensity, target gains, bed, ducking.
    fn publish(&mut self) {
        let latest = self.latest.as_ref();
        let gains = self.applied.map_or_else(
            || "null".to_owned(),
            |applied| {
                format!(
                    "{{\"bed\":{},\"motif\":{},\"tension\":{},\"combat\":{},\"ambient\":{}}}",
                    applied.layers[0],
                    applied.layers[1],
                    applied.layers[2],
                    applied.layers[3],
                    applied.ambient
                )
            },
        );
        let snapshot = format!(
            "{{\"sound\":{},\"context\":{},\"phase\":{},\"intensity\":{},\"sequence\":{},\"cue\":{},\
\"gains\":{gains},\"filter_hertz\":{},\"master\":{},\"bed\":{},\"bed_playing\":{},\
\"layers_started\":{},\"ducked\":{},\"volume\":{},\"loaded\":{},\"missing\":{},\
\"stingers\":{},\"error\":{}}}",
            text(if self.enabled { "on" } else { "off" }),
            text(self.graph.as_ref().map_or("none", Graph::state_name)),
            optional_text(latest.map(|latest| latest.tempo.phase.name())),
            optional_number(latest.map(|latest| latest.tempo.intensity)),
            optional_number(latest.map(|latest| latest.tempo.sequence)),
            optional_number(latest.and_then(|latest| latest.tempo.cue)),
            optional_number(self.applied.map(|applied| applied.filter_hertz)),
            optional_number(self.applied.map(|applied| applied.master)),
            optional_text(latest.map(|latest| latest.facts.bed.track())),
            optional_text(
                self.graph
                    .as_ref()
                    .and_then(|graph| graph.ambient.as_ref())
                    .map(|ambient| ambient.track)
            ),
            self.graph
                .as_ref()
                .is_some_and(|graph| graph.started_at.is_some()),
            self.speaking,
            self.volume,
            self.buffers.len(),
            list(self.missing.iter().copied()),
            list(self.fired.iter().copied()),
            optional_text(self.failure.as_deref()),
        );
        if snapshot != self.published {
            let _published = self.root.set_attribute(STATE_ATTRIBUTE, &snapshot);
            self.published = snapshot;
        }
    }
}

/// A JSON string literal (the browser escapes it).
fn text(value: &str) -> String {
    js_sys::JSON::stringify(&JsValue::from_str(value))
        .ok()
        .and_then(|literal| literal.as_string())
        .unwrap_or_else(|| "null".to_owned())
}

fn optional_text(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), text)
}

fn optional_number<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".to_owned(), |number| number.to_string())
}

fn list<'a>(values: impl Iterator<Item = &'a str>) -> String {
    format!("[{}]", values.map(text).collect::<Vec<_>>().join(","))
}

/// Called once per newly applied view. Music is best effort: a failure is recorded in the
/// debug snapshot and never affects rendering.
pub(super) fn observe(client: &Rc<RefCell<Client>>) {
    let (document, root, view, connection) = {
        let state = client.borrow();
        if state.role != "display" {
            return;
        }
        let Some(view) = state
            .views
            .as_ref()
            .and_then(|views| views.current().cloned())
        else {
            return;
        };
        (
            state.document.clone(),
            state.root.clone(),
            view,
            state.generation,
        )
    };
    let sequence = view
        .revision
        .as_ref()
        .and_then(|revision| revision.sequence)
        .unwrap_or(0);
    let Some(rpc::view_message::Audience::Display(display)) = view.audience else {
        return;
    };
    let Some(journey) = display.journey.as_ref() else {
        return;
    };
    let scene = display.scene.clone().unwrap_or_default();
    let tempo = tempo_of(&scene, journey, sequence);
    super::live_battlefield::set_tempo(tempo.intensity);
    let facts = facts_of(&scene, journey, &display.scene_asset);
    match director(&document, &root) {
        Ok(shared) => {
            shared.borrow_mut().accept(tempo, facts, connection);
            tick(&shared);
        }
        Err(_) => {
            let _marked = root.set_attribute("data-music", "unavailable");
        }
    }
}

fn director(document: &Document, root: &Element) -> Result<Shared, JsValue> {
    let existing = DIRECTOR.with(|slot| slot.borrow().clone());
    if let Some(shared) = existing {
        return Ok(shared);
    }
    let created = create(document, root)?;
    DIRECTOR.with(|slot| *slot.borrow_mut() = Some(created.clone()));
    Ok(created)
}

fn create(document: &Document, root: &Element) -> Result<Shared, JsValue> {
    let shared = Rc::new(RefCell::new(Director {
        document: document.clone(),
        root: root.clone(),
        graph: None,
        buffers: HashMap::new(),
        requested: HashSet::new(),
        missing: Vec::new(),
        latest: None,
        enabled: false,
        speaking: false,
        volume: stored_volume(),
        applied: None,
        resync: false,
        fired: VecDeque::new(),
        published: String::new(),
        failure: None,
        ticker: None,
        cues: CueMemory::default(),
        connection: 0,
        listeners: Vec::new(),
    }));
    for name in ["pointerdown", "keydown", "click"] {
        let weak = Rc::downgrade(&shared);
        let callback = Closure::wrap(Box::new(move |_event: Event| {
            if let Some(shared) = weak.upgrade() {
                on_gesture(&shared);
            }
        }) as Box<dyn FnMut(Event)>);
        document.add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())?;
        shared.borrow_mut().listeners.push(callback);
    }
    let weak = Rc::downgrade(&shared);
    let ticker = Interval::new(TICK_MILLIS, move || {
        if let Some(shared) = weak.upgrade() {
            tick(&shared);
        }
    });
    shared.borrow_mut().ticker = Some(ticker);
    Ok(shared)
}

/// The browser lets an audio context run only after a user gesture, so the context is made and
/// resumed inside one. This starts no fetch and makes no sound: the shared unlock and mute
/// state still decide whether any music is requested or heard.
fn on_gesture(shared: &Shared) {
    let Ok(mut state) = shared.try_borrow_mut() else {
        return;
    };
    if state.graph.is_none() {
        match Graph::new() {
            Ok(graph) => state.graph = Some(graph),
            Err(error) => state.fail(&error),
        }
    }
    let resumed = state
        .graph
        .as_ref()
        .filter(|graph| graph.context.state() == AudioContextState::Suspended)
        .map(|graph| graph.context.resume());
    match resumed {
        Some(Ok(promise)) => settle(promise),
        Some(Err(error)) => state.fail(&error),
        None => {}
    }
}

fn tick(shared: &Shared) {
    let Ok(mut state) = shared.try_borrow_mut() else {
        return;
    };
    let wanted = match state.step() {
        Ok(wanted) => wanted,
        Err(error) => {
            state.fail(&error);
            Vec::new()
        }
    };
    drop(state);
    for track in wanted {
        spawn_local(load(Rc::clone(shared), track));
    }
}

/// Fetches and decodes one track. A missing or undecodable track is recorded and simply not
/// played; it is never requested again.
async fn load(shared: Shared, track: &'static str) {
    let context = shared
        .borrow()
        .graph
        .as_ref()
        .map(|graph| graph.context.clone());
    let Some(context) = context else {
        return;
    };
    let outcome = fetch_track(&context, track).await;
    {
        let mut state = shared.borrow_mut();
        match outcome {
            Ok(buffer) => {
                state.buffers.insert(track, buffer);
            }
            Err(_) => state.missing.push(track),
        }
    }
    tick(&shared);
}

/// Anything but a non-empty, bounded 200 response that decodes is an error.
async fn fetch_track(context: &AudioContext, track: &str) -> Result<AudioBuffer, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let response: Response =
        JsFuture::from(window.fetch_with_str(&format!("{MUSIC_PATH}/{track}")))
            .await?
            .dyn_into()?;
    if response.status() != 200 {
        return Err(JsValue::from_str("music track unavailable"));
    }
    let bytes: js_sys::ArrayBuffer = JsFuture::from(response.array_buffer()?).await?.dyn_into()?;
    if bytes.byte_length() == 0 || bytes.byte_length() > MAXIMUM_TRACK_BYTES {
        return Err(JsValue::from_str("music track size refused"));
    }
    JsFuture::from(context.decode_audio_data(&bytes)?)
        .await?
        .dyn_into()
}

/// The remembered music volume of this tab, 0 to 1. Unavailable or invalid storage means the
/// default.
fn stored_volume() -> f32 {
    web_sys::window()
        .and_then(|window| window.session_storage().ok().flatten())
        .and_then(|storage| storage.get_item(VOLUME_KEY).ok().flatten())
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|volume| volume.is_finite())
        .map_or(DEFAULT_VOLUME, |volume| volume.clamp(0.0, 1.0))
}
