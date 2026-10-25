//! Adaptive background music on the shared display only.
//!
//! The browser decides no story: for every accepted display view `music_score::tempo_of` turns
//! what the server already shows into a phase and intensity, and this module plays the
//! server-provided score (`/assets/music/<track>`) accordingly. Four sample-aligned loops
//! (bed, motif, tension, combat) start together on one Web Audio clock; which of them sound
//! changes per arrangement epoch while their gains follow the tempo. A location ambience loops
//! beside them, and one-shot stingers land on the next beat, rotating takes. Phones never reach
//! this module.
//!
//! Sound follows the one shared state of `live_voice`: nothing is requested or heard before its
//! unlock gesture, its mute control silences the music too, and music ducks under every
//! presented line — a voice clip on the air, or a caption-only line the scene line machine is
//! showing (read off the stage root's `data-line-state`, so that module is untouched) — and
//! shallow under big sound effects (read from the effects engine's `data-sfx-*` attributes).
//! A hidden tab suspends the audio clock and, on return, the mix jumps to the present rather
//! than replaying what happened. The music volume (0 to 1) is read from `sessionStorage`
//! under `df.music.volume`. Score, ambience and stingers run on separate buses into one music
//! master with a limiter on the sum. A debug snapshot of the mix is kept in the
//! `data-music-state` attribute of the display root.
use super::*;
use crate::music_score::{
    ARRANGE_SECONDS, Bed, CueMemory, DUCK_LINE_ATTACK_SECONDS, DUCK_RELEASE_SECONDS,
    DUCK_SFX_ATTACK_SECONDS, DUCK_SFX_DECIBELS, DUCK_SFX_HOLD_SECONDS, DUCK_SFX_RELEASE_SECONDS,
    Facts, GENERATED_MUSIC_ROUTE, LAYER_TRACKS, LINE_BUSY_SCALE, LINE_FILTER_HERTZ, LOOP_SECONDS,
    Outcome, PacingBand, Phase, PlaceMusic, SOMBRE_FILTER_HERTZ, STINGER_TAKES, Stinger, Suite,
    TRANSITION_SECONDS, Tempo, VOICE_LUFS, ambience_path_of, arrangement_for, arrangements_for,
    bubble_hold_ms, colour_for, duck_decibels, effective_phase, facts_after_presentation,
    facts_with_plan, gain_for_decibels, gap_hold_active, line_presenting, mix_for, mix_level_lufs,
    next_pacing_beat, next_take, place_music_of, scene_layer_gains, shared_path, stinger_track,
    stinger_with_baseline, suite_for, swap_boundary, tempo_of, unvoiced_hold_ms,
    wanted_layer_paths,
};
use crate::music_score::{
    AmbientChange, EntryMemory, PlaceAudioPlan, ambient_changes, entry_handoff, place_audio_of,
};
use gloo_timers::callback::Interval;
use std::collections::{HashMap, HashSet, VecDeque};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, AudioNode, AudioParam,
    AudioScheduledSourceNode, BiquadFilterNode, BiquadFilterType, Event, GainNode, Response,
};

const VOLUME_KEY: &str = "df.music.volume";
/// Music clearly under the voice: the layers are mastered near -23 LUFS and the voice near -16.
const DEFAULT_VOLUME: f32 = 0.85;
const STATE_ATTRIBUTE: &str = "data-music-state";
const MAXIMUM_TRACK_BYTES: u32 = 6 * 1024 * 1024;
/// Housekeeping runs this often: a new line is found within one tick, and the line-duck ramp
/// (80 ms) then lands fully down inside 150 ms of the voice starting.
const TICK_MILLIS: u32 = 50;
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
/// Duck down/up edges kept in the debug snapshot, oldest first.
const DUCK_LOG_MEMORY: usize = 8;
/// Suite or take change: the new voices start exactly on the next loop boundary while the old
/// ones fade out across it, so a chapter change is a crossfade, never a cut and never a gap.
const SWAP_CROSSFADE_SECONDS: f64 = 4.0;
const MINIMUM_FILTER_HERTZ: f32 = 400.0;
const MAXIMUM_FILTER_HERTZ: f32 = 18_000.0;
/// Limiter on the music sum: stacked layers plus a stinger cannot clip the room.
const LIMITER_THRESHOLD_DB: f32 = -12.0;
const LIMITER_KNEE_DB: f32 = 20.0;
const LIMITER_RATIO: f32 = 6.0;
const LIMITER_ATTACK_SECONDS: f32 = 0.003;
const LIMITER_RELEASE_SECONDS: f32 = 0.25;
/// The effects engine's diagnostic attributes, read here so `live_sfx` needs no hook: the count
/// says an effect played, the trailing log name says whether it was a big one.
const SFX_COUNT_ATTRIBUTE: &str = "data-sfx-count";
const SFX_LOG_ATTRIBUTE: &str = "data-sfx-log";
/// A big sound effect earns the music a short shallow dip. Names are `live_sfx` cue names.
const BIG_SFX: &[&str] = &[
    "critical",
    "victory-fanfare",
    "defeat-toll",
    "scene-transition",
    "party-gathered",
];

/// Colour the running place layers using only the server's phase and bounded intensity.
/// No track/path changes: a conversation keeps the bed's playhead and shared loop grid.
fn roleplay_layers(mut layers: [f32; 4], phase: Phase, intensity_permille: u16) -> [f32; 4] {
    let intensity = f32::from(intensity_permille.min(1000)) / 1000.0;
    layers[0] *= 0.85 + 0.15 * intensity;
    layers[1] *= match phase {
        Phase::Calm | Phase::Release => 0.9 + 0.1 * intensity,
        _ => 0.6 + 0.4 * intensity,
    };
    layers[2] *= match phase {
        Phase::Calm | Phase::Release => 0.25 * intensity,
        _ => 0.3 + 0.7 * intensity,
    };
    layers[3] = 0.0;
    layers
}

#[cfg(test)]
mod roleplay_tests {
    use super::*;

    #[test]
    fn live_music_intensity_and_warmth_mix_existing_layers_without_combat() {
        let quiet = roleplay_layers([1.0; 4], Phase::Anticipation, 100);
        let tense = roleplay_layers([1.0; 4], Phase::Anticipation, 600);
        let warm = roleplay_layers([1.0; 4], Phase::Calm, 200);
        assert!(tense[2] > quiet[2]);
        assert!(warm[1] > tense[1]);
        assert!(warm[2] < quiet[2]);
        assert_eq!(tense[3], 0.0);
        assert_eq!(
            roleplay_layers([1.0; 4], Phase::Build, u16::MAX),
            roleplay_layers([1.0; 4], Phase::Build, 1000)
        );
    }
}

/// Everything the mixer sets at one moment; a change of any field starts a glide.
#[derive(Clone, Copy, PartialEq)]
struct Applied {
    layers: [f32; 4],
    ambient: f32,
    score_bus: f32,
    stinger_bus: f32,
    filter_hertz: f32,
    master: f32,
    duck: f32,
}

struct Ambient {
    label: &'static str,
    /// Served path of the loop.
    track: String,
    gain: GainNode,
    source: AudioBufferSourceNode,
    target: f32,
}

/// One looping layer voice: its own gain lets a suite or take swap crossfade voice by voice
/// while the shared layer bus keeps the mix target. Which track each voice plays is tracked on
/// the director (`layer_tracks`), so the voice carries only its nodes.
struct LayerVoice {
    node: AudioBufferSourceNode,
    gain: GainNode,
}

/// The audio graph: layers -> low-pass -> score bus -> duck -> music master -> limiter, with
/// the ambience bus and the stinger bus joining at the duck so all of it yields to narration
/// and big effects, and the limiter keeps the stacked sum from clipping.
struct Graph {
    context: AudioContext,
    master: GainNode,
    // Wired once at creation and never re-read: the field is the reference that keeps the node
    // alive for the page's lifetime.
    #[allow(dead_code)]
    music_master: GainNode,
    #[allow(dead_code)]
    sum_limiter: AudioNode,
    duck: GainNode,
    filter: BiquadFilterNode,
    layers: [GainNode; 4],
    score_bus: GainNode,
    stinger_bus: GainNode,
    ambient_bus: GainNode,
    ambient: Option<Ambient>,
    ambient_layers: Vec<Ambient>,
    /// Audio-clock time the four layers started together; the beat grid hangs from it.
    started_at: Option<f64>,
    /// Kept so the looping sources stay referenced for the page's lifetime.
    voices: Vec<LayerVoice>,
}

struct Latest {
    /// What the server planned for this place; the shared score until its own set is ready.
    place: PlaceMusic,
    tempo: Tempo,
    facts: Facts,
    release_since: Option<f64>,
}

struct Director {
    audio_plan: Option<PlaceAudioPlan>,
    entry_memory: EntryMemory,
    pending_entry: Option<(String, f64)>,
    entry_until: f64,
    entry_handoff_at: Option<f64>,
    entry_voice: Option<LayerVoice>,
    document: Document,
    root: Element,
    graph: Option<Graph>,
    /// Decoded tracks by served path (shared score, beds and a place's own generated files).
    buffers: HashMap<String, AudioBuffer>,
    requested: HashSet<String>,
    missing: Vec<String>,
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
    /// Outcome facts held until the display acknowledges the last resolved combat beat.
    pending_outcome: Option<Outcome>,
    outcome_baseline: Option<Outcome>,
    listeners: Vec<Closure<dyn FnMut(Event)>>,
    /// Session seed for the arrangement choice and the alternate-take pick.
    seed: u64,
    /// Whether this session prefers the `-b` layer takes where their files exist.
    layer_alternate: bool,
    /// The layer tracks actually looping, if the layers have started.
    layer_tracks: Option<[String; 4]>,
    /// Do not replace a scheduled variant before its boundary and crossfade have completed.
    layer_switch_until: f64,
    playing_band: PacingBand,
    scheduled_band: Option<(PacingBand, f64)>,
    /// Current arrangement epoch on the audio clock, and the voicing in use.
    epoch: Option<u64>,
    arrangement: usize,
    arrange_phase: Option<Phase>,
    arrange_bed: Option<Bed>,
    /// Place colour and suite in use, for the debug snapshot.
    colour: &'static str,
    suite: Suite,
    /// Last take position played per stinger, in `Stinger::ALL` order.
    stinger_takes: [Option<usize>; 4],
    /// Last effects counter seen; a jump means an effect played since the last tick.
    sfx_count: Option<u64>,
    /// Wall-clock milliseconds until which the shallow effects dip holds.
    sfx_dip_until: f64,
    /// Wall-clock milliseconds of the last tick with any line signal up: the duck holds
    /// while this is fresher than the gap bound, so back-to-back lines never pump the music.
    line_seen_ms: Option<f64>,
    /// Wall-clock milliseconds until which a newly seen caption line holds at least the
    /// display's own minimum dwell, even if its machine already moved on.
    caption_floor_until: f64,
    /// The stage root's `data-line-id` last seen, to spot a new line for the caption floor.
    last_line_id: Option<String>,
    /// The action call-out nodes standing on the last tick: only a newly appeared node
    /// retriggers the hold, because faded ones stay in the DOM (and a repeated cry is a new
    /// node even when its text matches, so node identity, not text, is the edge).
    action_nodes: Vec<Element>,
    /// Why the duck sits where it does: `voice`, `caption`, `sfx` or `none`.
    duck_reason: &'static str,
    /// The last duck down/up edges with wall-clock timestamps, oldest first.
    duck_log: VecDeque<String>,
    /// This housekeeping step's own cost and its maximum, in milliseconds: the duck cannot
    /// start sooner than a step runs.
    tick_ms: f64,
    tick_ms_max: f64,
    /// Effective phase the mix last glided toward; a change lengthens the rise.
    last_phase: Option<Phase>,
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

/// The limiter on the music sum. Built through untyped calls because the crate's web-sys
/// feature list (owned by another worker's `Cargo.toml`, which this task must not edit) has no
/// `DynamicsCompressor` binding yet; the handoff proposes that one-line cleanup.
fn sum_limiter(context: &AudioContext) -> Result<AudioNode, JsValue> {
    let create: js_sys::Function =
        js_sys::Reflect::get(context, &JsValue::from_str("createDynamicsCompressor"))?
            .dyn_into()?;
    let node: AudioNode = create.call0(context)?.dyn_into()?;
    for (name, value) in [
        ("threshold", LIMITER_THRESHOLD_DB),
        ("knee", LIMITER_KNEE_DB),
        ("ratio", LIMITER_RATIO),
        ("attack", LIMITER_ATTACK_SECONDS),
        ("release", LIMITER_RELEASE_SECONDS),
    ] {
        let param: AudioParam =
            js_sys::Reflect::get(&node, &JsValue::from_str(name))?.dyn_into()?;
        param.set_value(value);
    }
    Ok(node)
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

/// A new layer voice fades in from the loop boundary it starts on: silence until then, full
/// across the swap crossfade. Anchored at `boundary`, not at now.
fn fade_voice_in(gain: &AudioParam, boundary: f64) -> Result<(), JsValue> {
    gain.set_value_at_time(0.0, boundary)?;
    gain.linear_ramp_to_value_at_time(1.0, boundary + SWAP_CROSSFADE_SECONDS)?;
    Ok(())
}

/// An old layer voice holds its level until the loop boundary, then fades out across the same
/// crossfade its replacement fades in across.
fn fade_voice_out(gain: &AudioParam, now: f64, boundary: f64) -> Result<(), JsValue> {
    gain.cancel_scheduled_values(now)?;
    gain.set_value_at_time(gain.value(), now)?;
    gain.set_value_at_time(1.0, boundary)?;
    gain.linear_ramp_to_value_at_time(0.0, boundary + SWAP_CROSSFADE_SECONDS)?;
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
        let sum_limiter = sum_limiter(&context)?;
        sum_limiter.connect_with_audio_node(&context.destination())?;
        let music_master = context.create_gain()?;
        music_master.gain().set_value(1.0);
        music_master.connect_with_audio_node(&sum_limiter)?;
        let master = context.create_gain()?;
        master.gain().set_value(0.0);
        master.connect_with_audio_node(&music_master)?;
        let duck = context.create_gain()?;
        duck.connect_with_audio_node(&master)?;
        let filter = context.create_biquad_filter()?;
        filter.set_type(BiquadFilterType::Lowpass);
        filter.frequency().set_value(1_400.0);
        let score_bus = context.create_gain()?;
        score_bus.gain().set_value(1.0);
        filter.connect_with_audio_node(&score_bus)?;
        score_bus.connect_with_audio_node(&duck)?;
        let layers = [
            silent_gain(&context, &filter)?,
            silent_gain(&context, &filter)?,
            silent_gain(&context, &filter)?,
            silent_gain(&context, &filter)?,
        ];
        let ambient_bus = silent_gain(&context, &duck)?;
        let stinger_bus = context.create_gain()?;
        stinger_bus.gain().set_value(1.0);
        stinger_bus.connect_with_audio_node(&duck)?;
        Ok(Self {
            context,
            master,
            music_master,
            sum_limiter,
            duck,
            filter,
            layers,
            score_bus,
            stinger_bus,
            ambient_bus,
            ambient: None,
            ambient_layers: Vec::new(),
            started_at: None,
            voices: Vec::new(),
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
    fn accept(
        &mut self,
        tempo: Tempo,
        facts: Facts,
        place: PlaceMusic,
        connection: u64,
        audio: Option<PlaceAudioPlan>,
        lobby: bool,
    ) {
        // The first view of a new watch connection is a state, not a story change: nothing to compare.
        let fresh_connection = connection != self.connection;
        if connection != self.connection {
            self.entry_memory = EntryMemory::default();
            self.pending_entry = None;
            self.connection = connection;
            self.cues.reset();
            self.pending_outcome = None;
        }
        if self.entry_memory.observe(lobby) && self.enabled && !self.document.hidden() {
            let entry = audio
                .as_ref()
                .map_or("sting-reveal", |plan| &plan.entry_stinger);
            self.pending_entry = Some((entry.to_owned(), js_sys::Date::now() + 2000.0));
        }
        self.audio_plan = audio;
        let outcome = facts.outcome;
        if fresh_connection {
            self.outcome_baseline = outcome;
        } else if facts.in_combat && outcome.is_none() {
            self.outcome_baseline = None;
        }
        let facts = match presentation_sequences(&self.document) {
            Some((presented, latest)) => facts_after_presentation(facts, presented, latest),
            None => facts_after_presentation(facts, 0, 1),
        };
        self.pending_outcome = outcome.filter(|_| facts.outcome.is_none());
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
            .filter(|_| !fresh_connection)
            .and_then(|previous| {
                stinger_with_baseline(&previous.facts, &facts, self.outcome_baseline)
            })
            .filter(|_| self.pending_entry.is_none());
        self.latest = Some(Latest {
            place,
            tempo,
            facts,
            release_since,
        });
        self.release_unused_places();
        if let Some(stinger) = stinger
            && let Err(error) = self.play_stinger(stinger)
        {
            self.fail(&error);
        }
    }

    /// Drops decoded place sets the mix can no longer reach. Each visited place holds roughly
    /// 70 MB of layers at a 48 kHz context. A voice that still plays keeps its own buffer.
    fn release_unused_places(&mut self) {
        let mut keep: HashSet<String> = self.wanted_layer_tracks().into_iter().collect();
        keep.extend(self.layer_tracks.iter().flatten().cloned());
        keep.extend(
            self.audio_plan
                .iter()
                .flat_map(|plan| plan.layers.iter().map(|layer| layer.path.clone())),
        );
        keep.extend(
            self.latest
                .as_ref()
                .and_then(|latest| ambience_path_of(&latest.place))
                .map(str::to_owned),
        );
        for graph in self.graph.iter() {
            keep.extend(graph.ambient.iter().map(|ambient| ambient.track.clone()));
            keep.extend(graph.ambient_layers.iter().map(|voice| voice.track.clone()));
        }
        let reachable =
            |track: &String| !track.starts_with(GENERATED_MUSIC_ROUTE) || keep.contains(track);
        self.buffers.retain(|track, _| reachable(track));
        self.requested.retain(|track| reachable(track));
    }

    /// One pass of housekeeping: shared sound state, tab visibility, the audio graph, the
    /// mix and the debug snapshot. Returns the tracks that now need fetching.
    fn step(&mut self) -> Result<Vec<String>, JsValue> {
        if let Some(sound) = live_voice::sound_state() {
            if self.enabled != sound.enabled {
                self.set_sound_output(sound.enabled)?;
            }
            self.speaking = sound.speaking;
        }
        // Before any sound is on there is nothing to duck or mix: skip the DOM polling and the snapshot churn.
        if !self.enabled && self.graph.is_none() {
            if self.published.is_empty() {
                self.publish();
            }
            return Ok(Vec::new());
        }
        self.volume = stored_volume();
        self.watch_sfx();
        self.watch_line();
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
            if self.entry_until > 0.0
                && let Some(graph) = self.graph.as_mut()
            {
                let now = graph.context.current_time();
                if let Some(voice) = self.entry_voice.take() {
                    AudioScheduledSourceNode::stop_with_when(&voice.node, now)?;
                }
                for voice in graph.voices.drain(..) {
                    AudioScheduledSourceNode::stop_with_when(&voice.node, now)?;
                }
                graph.score_bus.gain().cancel_scheduled_values(now)?;
                graph.score_bus.gain().set_value_at_time(1.0, now)?;
                graph.started_at = None;
                self.layer_tracks = None;
                self.entry_until = 0.0;
                self.entry_handoff_at = None;
                super::presentation_timing::sound(&self.root, "music-entry", "cancelled-hidden");
            }
            self.pending_entry = None;
            self.publish();
            return Ok(Vec::new());
        }
        if self.enabled && state == AudioContextState::Suspended {
            settle(graph.context.resume()?);
            self.resync = true;
        }
        let wanted = self.tracks_to_fetch();
        self.apply_entry()?;
        if let Some(graph) = &self.graph {
            let now = graph.context.current_time();
            if self.entry_handoff_at.is_some_and(|at| now >= at) {
                self.entry_handoff_at = None;
                super::presentation_timing::sound(&self.root, "music-entry", "handoff-started");
            }
            if self.entry_until > 0.0 && now >= self.entry_until {
                self.entry_until = 0.0;
                self.entry_voice = None;
                super::presentation_timing::sound(&self.root, "music-entry", "handoff-ended");
            }
        }
        self.ensure_layers()?;
        self.apply_bed()?;
        self.retarget()?;
        self.apply_ambient_layers()?;
        self.release_unused_places();
        self.publish();
        Ok(wanted)
    }

    /// A mute click must stop the output now; the normal mixer poll and master glide are for
    /// score changes and volume changes, not for a user's explicit mute choice.
    fn set_sound_output(&mut self, enabled: bool) -> Result<(), JsValue> {
        self.enabled = enabled;
        if let Some(graph) = self.graph.as_ref() {
            let now = graph.context.current_time();
            let output = graph.music_master.gain();
            output.cancel_scheduled_values(now)?;
            output.set_value_at_time(if enabled { 1.0 } else { 0.0 }, now)?;
            if !enabled {
                let master = graph.master.gain();
                master.cancel_scheduled_values(now)?;
                master.set_value_at_time(0.0, now)?;
                // Muted: suspend the audio clock so the loops stop costing CPU. Resuming keeps the grid.
                settle(graph.context.suspend()?);
            }
        }
        if !enabled && let Some(applied) = self.applied.as_mut() {
            applied.master = 0.0;
        }
        self.publish();
        Ok(())
    }

    /// A jump in the effects engine's counter means an effect played since the last tick; a big
    /// one (by its logged cue name) holds the shallow dip open. First sight only adopts the
    /// counter so a page load never dips.
    fn watch_sfx(&mut self) {
        let element = self.document.document_element();
        let count = element
            .as_ref()
            .and_then(|element| element.get_attribute(SFX_COUNT_ATTRIBUTE))
            .and_then(|value| value.parse::<u64>().ok());
        let Some(seen) = count else {
            return;
        };
        let Some(known) = self.sfx_count else {
            self.sfx_count = Some(seen);
            return;
        };
        if seen <= known {
            return;
        }
        self.sfx_count = Some(seen);
        let big = element
            .as_ref()
            .and_then(|element| element.get_attribute(SFX_LOG_ATTRIBUTE))
            .is_some_and(|log| {
                log.split(' ')
                    .next_back()
                    .is_some_and(|last| BIG_SFX.contains(&last))
            });
        if big {
            self.sfx_dip_until = js_sys::Date::now() + DUCK_SFX_HOLD_SECONDS * 1_000.0;
        }
    }

    /// A line is being presented when the scene line machine says so on the stage root (the
    /// narrator's box, every character's and hero's bubble, fight cries and commentary, and
    /// the loading-screen mirror all run this same machine: no role is exempt), while a hero
    /// holds push-to-talk (the speech-intent loader), or while a fresh action or roll-readout
    /// bubble stands. Attribute presence only; line text is never read. Opening and speaking
    /// retrigger the hold; lingering lines do not. A newly seen line floors it at minimum dwell.
    fn watch_line(&mut self) {
        let now = js_sys::Date::now();
        let stage = self.document.query_selector(".tv-stage").ok().flatten();
        let machine = stage
            .as_ref()
            .and_then(|root| root.get_attribute("data-line-state"))
            .is_some_and(|state| line_presenting(&state));
        let line_id = stage
            .as_ref()
            .and_then(|root| root.get_attribute("data-line-id"));
        // Action call-outs outside the line machine fade by CSS but stay in the DOM, so bare
        // presence would latch the duck on: only a newly appeared bubble retriggers, and it
        // holds for the bubble's own `--hold` dwell from its inline style (machine-driven
        // bubbles carry no style; the machine already covers those).
        let mut fresh_action = false;
        let mut standing = Vec::new();
        if let Ok(bubbles) = self
            .document
            .query_selector_all(".tv-bubble[data-kind=\"action\"]")
        {
            for index in 0..bubbles.length() {
                let element = bubbles
                    .item(index)
                    .and_then(|node| node.dyn_into::<Element>().ok());
                let Some(element) = element else {
                    continue;
                };
                if !self
                    .action_nodes
                    .iter()
                    .any(|known| known.is_same_node(Some(&element)))
                {
                    fresh_action = true;
                    let hold = element
                        .get_attribute("style")
                        .and_then(|style| bubble_hold_ms(&style));
                    if let Some(hold) = hold {
                        self.caption_floor_until = self.caption_floor_until.max(now + hold);
                    }
                }
                standing.push(element);
            }
        }
        self.action_nodes = standing;
        let push_to_talk = self
            .document
            .query_selector(".tv-intent[data-intent=\"speaking\"]")
            .ok()
            .flatten()
            .is_some();
        if line_id != self.last_line_id {
            self.last_line_id = line_id.clone();
            if line_id.is_some() {
                self.caption_floor_until = now + f64::from(unvoiced_hold_ms(""));
            }
        }
        if machine || fresh_action || push_to_talk {
            self.line_seen_ms = Some(now);
        }
    }

    /// Remembers a duck down/up edge with its wall-clock timestamp, so a check can assert the
    /// duck against the line's own start and end. Capped; the oldest edge falls off.
    fn note_duck(&mut self, reason: &'static str) {
        if reason == self.duck_reason {
            return;
        }
        let now = js_sys::Date::now() as u64;
        let edge = if reason == "none" {
            format!("up {}->none @{now}", self.duck_reason)
        } else {
            match &self.last_line_id {
                Some(id) => format!("down {reason} line {id} @{now}"),
                None => format!("down {reason} @{now}"),
            }
        };
        if self.duck_log.len() >= DUCK_LOG_MEMORY {
            self.duck_log.pop_front();
        }
        self.duck_log.push_back(edge);
        self.duck_reason = reason;
    }

    /// The suite the current view asks for: combat chapters get the battle suite, a decided
    /// outcome the finale suite, everything else the main suite.
    fn wanted_suite(&self) -> Suite {
        self.latest.as_ref().map_or(Suite::Main, |latest| {
            suite_for(latest.facts.in_combat, latest.facts.outcome)
        })
    }

    /// The four layer paths for the wanted suite, this session's take pick and the server's
    /// plan for the place: the place's own layers once ready, the shared score otherwise.
    fn wanted_layer_tracks(&self) -> [String; 4] {
        let place = self
            .latest
            .as_ref()
            .map_or(&PlaceMusic::SharedScore, |latest| &latest.place);
        wanted_layer_paths(self.wanted_suite(), self.layer_alternate, place)
    }

    /// The tracks actually looped per layer: the wanted one, or the main-suite base when that
    /// file is absent. A place's own layers stack only as a set, so one that failed to load
    /// sends all four back to the shared score of the same suite. `None` while the choice still
    /// depends on a fetch in flight.
    fn resolved_layer_tracks(&self) -> Option<[String; 4]> {
        let wanted = self.wanted_layer_tracks();
        let own_set_failed = wanted
            .iter()
            .any(|track| track.starts_with(GENERATED_MUSIC_ROUTE) && self.missing.contains(track));
        let source = if own_set_failed {
            wanted_layer_paths(
                self.wanted_suite(),
                self.layer_alternate,
                &PlaceMusic::SharedScore,
            )
        } else {
            wanted
        };
        let mut resolved = source.clone();
        for (index, track) in source.iter().enumerate() {
            if self.buffers.contains_key(track) {
                continue;
            }
            if !self.missing.contains(track) {
                return None;
            }
            let base = shared_path(LAYER_TRACKS[index]);
            if self.buffers.contains_key(&base) || self.missing.contains(&base) {
                resolved[index] = base;
            } else {
                return None;
            }
        }
        Some(resolved)
    }

    /// Layers first (they start together), then the location ambience, then the stingers.
    /// Stinger takes all ride along: rotation needs them loaded, and a file that is not on
    /// disk is recorded once and never requested again.
    fn tracks_to_fetch(&mut self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let bed = self
            .latest
            .as_ref()
            .and_then(|latest| ambience_path_of(&latest.place))
            .map(str::to_owned);
        let mut wanted: Vec<String> = self.wanted_layer_tracks().into_iter().collect();
        wanted.extend(LAYER_TRACKS.iter().map(|track| shared_path(track)));
        for kind in Stinger::ALL {
            for take in 0..STINGER_TAKES {
                wanted.push(shared_path(stinger_track(kind, take)));
            }
        }
        wanted.extend(bed);
        if let Some(plan) = &self.audio_plan {
            wanted.extend(
                plan.layers
                    .iter()
                    .filter(|layer| layer.status == "ready")
                    .map(|layer| layer.path.clone())
                    .filter(|path| !path.is_empty()),
            );
        }
        let mut fresh = Vec::new();
        for track in wanted {
            if self.requested.insert(track.clone()) {
                fresh.push(track);
            }
        }
        fresh
    }

    /// Starts the layers together on one clock, then keeps them on the suite the story asks
    /// for: a suite or take change starts new voices exactly on the next loop boundary while
    /// the old ones fade out across it. No cut, no gap; every voice stays on the bar grid.
    fn ensure_layers(&mut self) -> Result<(), JsValue> {
        if self
            .graph
            .as_ref()
            .is_some_and(|graph| graph.context.current_time() < self.entry_until)
        {
            return Ok(());
        }
        if let Some((band, at)) = self.scheduled_band
            && self
                .graph
                .as_ref()
                .is_some_and(|graph| graph.context.current_time() >= at)
        {
            self.playing_band = band;
            self.scheduled_band = None;
            super::presentation_timing::sound(&self.root, "music-variant", "boundary-reached");
        }
        if self.layer_switch_until > 0.0
            && self
                .graph
                .as_ref()
                .is_some_and(|graph| graph.context.current_time() >= self.layer_switch_until)
        {
            self.layer_switch_until = 0.0;
            super::presentation_timing::sound(&self.root, "music-variant", "crossfade-ended");
        }
        if self.wanted_suite() == self.suite
            && self
                .graph
                .as_ref()
                .is_some_and(|graph| graph.context.current_time() < self.layer_switch_until)
        {
            return Ok(());
        }
        let Some(resolved) = self.resolved_layer_tracks() else {
            return Ok(());
        };
        if !self.enabled {
            return Ok(());
        }
        let band = match self.latest.as_ref().map(|latest| &latest.place) {
            Some(PlaceMusic::Ready { band, layers, .. }) if layers == &resolved => *band,
            _ => PacingBand::Steady,
        };
        if self.layer_tracks.is_none() {
            let Some(graph) = self.graph.as_mut() else {
                return Ok(());
            };
            if graph.started_at.is_some() {
                return Ok(());
            }
            let when = graph.context.current_time() + START_LEAD_SECONDS;
            for (index, track) in resolved.iter().enumerate() {
                let Some(buffer) = self.buffers.get(track) else {
                    continue;
                };
                let gain = silent_gain(&graph.context, &graph.layers[index])?;
                gain.gain().set_value(1.0);
                let source = looping_source(&graph.context, buffer, &gain, Some(LOOP_SECONDS))?;
                source.start_with_when(when)?;
                graph.voices.push(LayerVoice { node: source, gain });
            }
            graph.started_at = Some(when);
            self.layer_tracks = Some(resolved);
            self.playing_band = band;
            self.suite = self.wanted_suite();
            return Ok(());
        }
        if self.layer_tracks.as_ref() == Some(&resolved) {
            return Ok(());
        }
        let Some(graph) = self.graph.as_mut() else {
            return Ok(());
        };
        let now = graph.context.current_time();
        let start = graph.started_at.unwrap_or(now);
        let boundary = swap_boundary(now, start, SWAP_CROSSFADE_SECONDS, START_LEAD_SECONDS);
        let retiring = graph.voices.len();
        for (index, track) in resolved.iter().enumerate() {
            let Some(buffer) = self.buffers.get(track) else {
                continue;
            };
            let gain = silent_gain(&graph.context, &graph.layers[index])?;
            let source = looping_source(&graph.context, buffer, &gain, Some(LOOP_SECONDS))?;
            source.start_with_when(boundary)?;
            fade_voice_in(&gain.gain(), boundary)?;
            graph.voices.push(LayerVoice { node: source, gain });
        }
        for voice in &graph.voices[..retiring] {
            fade_voice_out(&voice.gain.gain(), now, boundary)?;
            AudioScheduledSourceNode::stop_with_when(
                &voice.node,
                boundary + SWAP_CROSSFADE_SECONDS + 0.1,
            )?;
        }
        graph.voices.drain(..retiring);
        self.layer_switch_until = boundary + SWAP_CROSSFADE_SECONDS;
        self.scheduled_band = Some((band, boundary));
        super::presentation_timing::sound(&self.root, "music-variant", "scheduled");
        self.root
            .set_attribute("data-music-switch-boundary", &boundary.to_string())?;
        self.root.set_attribute(
            "data-music-switch-crossfade",
            &SWAP_CROSSFADE_SECONDS.to_string(),
        )?;
        self.layer_tracks = Some(resolved);
        self.suite = self.wanted_suite();
        Ok(())
    }

    /// Crossfades to the ambience the server's plan names once its track is loaded. A plan with
    /// no ambience fades the current one out and leaves the score alone.
    fn apply_bed(&mut self) -> Result<(), JsValue> {
        let (Some(graph), Some(latest)) = (self.graph.as_mut(), self.latest.as_ref()) else {
            return Ok(());
        };
        if !self.enabled {
            return Ok(());
        }
        let layered = self.audio_plan.as_ref().is_some_and(|plan| {
            plan.layers.is_empty()
                || plan
                    .layers
                    .iter()
                    .any(|layer| !layer.path.is_empty() && self.buffers.contains_key(&layer.path))
        });
        let wanted = if layered {
            None
        } else {
            ambience_path_of(&latest.place)
        };
        let playing = graph.ambient.as_ref().map(|ambient| ambient.track.as_str());
        if playing == wanted {
            return Ok(());
        }
        let now = graph.context.current_time();
        let incoming = match wanted {
            Some(path) => {
                let Some(buffer) = self.buffers.get(path) else {
                    return Ok(());
                };
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
                Some(Ambient {
                    label: "ambient-bed",
                    track: path.to_owned(),
                    gain,
                    source,
                    target: 1.0,
                })
            }
            None => None,
        };
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
        graph.ambient = incoming;
        Ok(())
    }

    /// Layer identity is its immutable hub path; line/tempo updates glide gains only.
    fn apply_ambient_layers(&mut self) -> Result<(), JsValue> {
        let Some(graph) = self.graph.as_mut() else {
            return Ok(());
        };
        if !self.enabled {
            return Ok(());
        }
        let now = graph.context.current_time();
        let speech = self.duck_reason == "voice" || self.duck_reason == "caption";
        let desired = self
            .audio_plan
            .as_ref()
            .map(|plan| plan.layers.as_slice())
            .unwrap_or(&[]);
        let playing: Vec<_> = graph
            .ambient_layers
            .iter()
            .map(|voice| (voice.track.as_str(), voice.target))
            .collect();
        let targets: Vec<_> = desired
            .iter()
            .filter(|layer| !layer.path.is_empty() && self.buffers.contains_key(&layer.path))
            .map(|layer| {
                (
                    layer.path.as_str(),
                    f32::from(if speech {
                        layer.speech_gain_permille
                    } else {
                        layer.gain_permille
                    }) / 1000.0,
                )
            })
            .collect();
        let changes = ambient_changes(&playing, &targets);
        for change in changes {
            match change {
                AmbientChange::Start(path, target) => {
                    let Some(buffer) = self.buffers.get(&path) else {
                        continue;
                    };
                    let gain = silent_gain(&graph.context, &graph.ambient_bus)?;
                    let source = looping_source(&graph.context, buffer, &gain, Some(24.0))?;
                    source.start_with_when(now)?;
                    glide(
                        &gain.gain(),
                        Some(0.0),
                        target,
                        now,
                        BED_CROSSFADE_SECONDS,
                        BED_CROSSFADE_SECONDS,
                        false,
                    )?;
                    let label = desired
                        .iter()
                        .find(|layer| layer.path == path)
                        .map_or("ambient-layer", |layer| layer.kind.label());
                    graph.ambient_layers.push(Ambient {
                        label,
                        track: path,
                        gain,
                        source,
                        target,
                    });
                    super::presentation_timing::sound(&self.root, label, "started");
                }
                AmbientChange::Gain(path, target) => {
                    if let Some(voice) = graph
                        .ambient_layers
                        .iter_mut()
                        .find(|voice| voice.track == path)
                    {
                        glide(
                            &voice.gain.gain(),
                            Some(voice.target),
                            target,
                            now,
                            BED_CROSSFADE_SECONDS,
                            DUCK_LINE_ATTACK_SECONDS,
                            false,
                        )?;
                        voice.target = target;
                        super::presentation_timing::sound(&self.root, voice.label, "gain-changed");
                    }
                }
                AmbientChange::Stop(path) => {
                    if let Some(index) = graph
                        .ambient_layers
                        .iter()
                        .position(|voice| voice.track == path)
                    {
                        let voice = graph.ambient_layers.remove(index);
                        glide(
                            &voice.gain.gain(),
                            Some(voice.target),
                            0.0,
                            now,
                            BED_CROSSFADE_SECONDS,
                            BED_CROSSFADE_SECONDS,
                            false,
                        )?;
                        AudioScheduledSourceNode::stop_with_when(
                            &voice.source,
                            now + BED_CROSSFADE_SECONDS,
                        )?;
                        super::presentation_timing::sound(&self.root, voice.label, "retiring");
                    }
                }
            }
        }
        Ok(())
    }

    /// The entry hit owns one short bar, then yields continuously to the living score.
    fn apply_entry(&mut self) -> Result<(), JsValue> {
        let Some((track, expires)) = self.pending_entry.clone() else {
            return Ok(());
        };
        if !self.enabled || js_sys::Date::now() > expires {
            self.pending_entry = None;
            super::presentation_timing::sound(&self.root, "music-entry", "skipped-unavailable");
            return Ok(());
        }
        let Some(resolved) = self.resolved_layer_tracks() else {
            return Ok(());
        };
        let Some(buffer) = self.buffers.get(&shared_path(&track)) else {
            return Ok(());
        };
        let Some((handoff, end)) = entry_handoff(buffer.duration()) else {
            self.pending_entry = None;
            return Ok(());
        };
        let Some(graph) = self.graph.as_mut() else {
            return Ok(());
        };
        if graph.context.state() != AudioContextState::Running {
            return Ok(());
        }
        let start = graph.context.current_time() + START_LEAD_SECONDS;
        let gain = silent_gain(&graph.context, &graph.stinger_bus)?;
        gain.gain().set_value_at_time(STINGER_LEVEL, start)?;
        gain.gain()
            .set_value_at_time(STINGER_LEVEL, start + handoff)?;
        gain.gain().linear_ramp_to_value_at_time(0.0, start + end)?;
        let source = graph.context.create_buffer_source()?;
        source.set_buffer(Some(buffer));
        source.connect_with_audio_node(&gain)?;
        source.start_with_when(start)?;
        AudioScheduledSourceNode::stop_with_when(&source, start + end)?;
        self.entry_voice = Some(LayerVoice { node: source, gain });
        let score = graph.score_bus.gain();
        let now = graph.context.current_time();
        score.cancel_scheduled_values(now)?;
        score.set_value_at_time(score.value(), now)?;
        score.linear_ramp_to_value_at_time(0.0, start)?;
        score.set_value_at_time(0.0, start + handoff)?;
        score.linear_ramp_to_value_at_time(1.0, start + end)?;
        for voice in graph.voices.drain(..) {
            AudioScheduledSourceNode::stop_with_when(&voice.node, start)?;
        }
        for (index, path) in resolved.iter().enumerate() {
            let Some(buffer) = self.buffers.get(path) else {
                continue;
            };
            let gain = silent_gain(&graph.context, &graph.layers[index])?;
            gain.gain().set_value(1.0);
            let source = looping_source(&graph.context, buffer, &gain, Some(LOOP_SECONDS))?;
            source.start_with_when(start + handoff)?;
            graph.voices.push(LayerVoice { node: source, gain });
        }
        graph.started_at = Some(start + handoff);
        self.entry_until = start + end;
        self.entry_handoff_at = Some(start + handoff);
        self.layer_switch_until = 0.0;
        self.scheduled_band = None;
        self.playing_band = match self.latest.as_ref().map(|latest| &latest.place) {
            Some(PlaceMusic::Ready { band, layers, .. }) if layers == &resolved => *band,
            _ => PacingBand::Steady,
        };
        self.layer_tracks = Some(resolved);
        self.pending_entry = None;
        self.suite = self.wanted_suite();
        self.root.set_attribute(
            "data-tempo-entry",
            &format!(
                "TEMPO entry stinger={track} handoff_ms={} crossfade_ms={}",
                handoff * 1000.0,
                (end - handoff) * 1000.0
            ),
        )?;
        super::presentation_timing::sound(&self.root, "music-entry", "stinger-scheduled");
        Ok(())
    }

    /// Glides every layer, the ambience, the buses, the filter, the duck and the master
    /// toward the mix the current tempo asks for: the phase table, thinned by the current
    /// arrangement voicing, coloured by place or fight, with a lengthened rise when the phase
    /// itself changed so a fight starts as a crossfade, not a cut.
    fn retarget(&mut self) -> Result<(), JsValue> {
        // The duck reason first, before the graph borrows below: a presented line always wins
        // over an effect, so a big hit mid-line never bounces the music back up. A clip on
        // the air is `voice`; a line the machine shows without one is `caption`.
        let now_ms = js_sys::Date::now();
        let line_hold = self
            .line_seen_ms
            .is_some_and(|seen| gap_hold_active(now_ms - seen))
            || now_ms < self.caption_floor_until;
        let reason = if self.speaking {
            "voice"
        } else if line_hold {
            "caption"
        } else if now_ms < self.sfx_dip_until {
            "sfx"
        } else {
            "none"
        };
        self.note_duck(reason);
        let (Some(graph), Some(latest)) = (self.graph.as_ref(), self.latest.as_ref()) else {
            return Ok(());
        };
        let held = latest
            .release_since
            .map_or(0.0, |since| js_sys::Date::now() - since);
        let phase = effective_phase(latest.tempo, held);
        let sombre = latest.facts.outcome == Some(Outcome::Defeat);
        let mix = mix_for(phase, latest.tempo.intensity, sombre);
        let colour = colour_for(latest.facts.bed, latest.facts.in_combat);
        if let Some(started_at) = graph.started_at {
            let elapsed = (graph.context.current_time() - started_at).max(0.0);
            let epoch = (elapsed / ARRANGE_SECONDS) as u64;
            let key_changed =
                self.arrange_phase != Some(phase) || self.arrange_bed != Some(latest.facts.bed);
            if self.epoch != Some(epoch) || key_changed {
                let previous = (!key_changed).then_some(self.arrangement);
                self.arrangement =
                    arrangement_for(phase, latest.facts.bed, self.seed, epoch, previous);
                self.epoch = Some(epoch);
                self.arrange_phase = Some(phase);
                self.arrange_bed = Some(latest.facts.bed);
            }
        }
        let scales = arrangements_for(phase)
            .get(self.arrangement)
            .map_or([1.0; 4], |state| state.scales);
        let table = mix.layers.in_track_order();
        let mut layers = [
            table[0] * scales[0] * colour.layers[0],
            table[1] * scales[1] * colour.layers[1],
            table[2] * scales[2] * colour.layers[2],
            table[3] * scales[3] * colour.layers[3],
        ];
        layers = scene_layer_gains(layers, latest.facts.in_combat);
        let roleplay = !latest.facts.in_combat
            && latest.facts.outcome.is_none()
            && self.pending_outcome.is_none();
        if roleplay {
            layers = roleplay_layers(layers, phase, latest.tempo.intensity);
        }
        let mut filter_hertz = (mix.filter_hertz * colour.filter_scale)
            .clamp(MINIMUM_FILTER_HERTZ, MAXIMUM_FILTER_HERTZ);
        if sombre {
            filter_hertz = filter_hertz.min(SOMBRE_FILTER_HERTZ);
        }
        // The depth follows what is actually playing: a battle dips heavier than a scene.
        let depth = duck_decibels(self.suite);
        let (duck, duck_rise, duck_fall) = match reason {
            "sfx" => (
                gain_for_decibels(DUCK_SFX_DECIBELS),
                DUCK_SFX_RELEASE_SECONDS,
                DUCK_SFX_ATTACK_SECONDS,
            ),
            "none" => (1.0, DUCK_RELEASE_SECONDS, DUCK_LINE_ATTACK_SECONDS),
            _ => (
                gain_for_decibels(depth),
                DUCK_RELEASE_SECONDS,
                DUCK_LINE_ATTACK_SECONDS,
            ),
        };
        // Thin the mix under speech as well as dropping its level: the low-pass closes over
        // the consonant band and the two busiest layers halve, so words stay clear even in a
        // full battle mix. An effects-only dip thins nothing.
        if reason == "voice" || reason == "caption" {
            filter_hertz = filter_hertz.min(LINE_FILTER_HERTZ);
            layers[2] *= LINE_BUSY_SCALE;
            layers[3] *= LINE_BUSY_SCALE;
        }
        self.colour = colour.name;
        let target = Applied {
            layers: if graph.started_at.is_some() {
                layers
            } else {
                [0.0; 4]
            },
            ambient: if self
                .audio_plan
                .as_ref()
                .is_some_and(|plan| !plan.layers.is_empty())
            {
                AMBIENT_LEVEL
            } else {
                mix.ambient * AMBIENT_LEVEL * colour.ambient_scale
            },
            score_bus: 1.0,
            stinger_bus: 1.0,
            filter_hertz,
            master: if self.enabled { self.volume } else { 0.0 },
            duck,
        };
        let previous = self.applied;
        if previous == Some(target) && !self.resync {
            self.last_phase = Some(phase);
            return Ok(());
        }
        // A phase change (into/out of a fight, a scene change, the end of the adventure)
        // lengthens the rise into a crossfade; deliberate fast drops keep their own fall.
        let rise = if self.last_phase != Some(phase) {
            mix.rise_seconds.max(TRANSITION_SECONDS)
        } else {
            mix.rise_seconds
        };
        let fall = if roleplay {
            mix.fall_seconds.max(TRANSITION_SECONDS)
        } else {
            mix.fall_seconds
        };
        let now = graph.context.current_time();
        let resync = self.resync;
        for (index, layer) in graph.layers.iter().enumerate() {
            glide(
                &layer.gain(),
                previous.map(|applied| applied.layers[index]),
                target.layers[index],
                now,
                rise,
                fall,
                resync,
            )?;
        }
        glide(
            &graph.ambient_bus.gain(),
            previous.map(|applied| applied.ambient),
            target.ambient,
            now,
            rise,
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
            duck_rise,
            duck_fall,
            resync,
        )?;
        self.applied = Some(target);
        self.last_phase = Some(phase);
        self.resync = false;
        Ok(())
    }

    /// Plays a stinger on the next beat, rotating takes so the same one never plays twice
    /// running while another is loaded. One that cannot play now (sound off, hidden tab, no
    /// take loaded) is skipped, never queued: an old flourish would be wrong later.
    fn play_stinger(&mut self, stinger: Stinger) -> Result<(), JsValue> {
        let Some(graph) = self.graph.as_ref() else {
            return Ok(());
        };
        if !self.enabled
            || self.document.hidden()
            || graph.context.state() != AudioContextState::Running
        {
            return Ok(());
        }
        let index = stinger_index(stinger);
        let mut loaded = Vec::new();
        for take in 0..STINGER_TAKES {
            let track = stinger_track(stinger, take);
            if self.buffers.contains_key(&shared_path(track)) {
                loaded.push(track);
            }
        }
        if loaded.is_empty() {
            return Ok(());
        }
        let position = next_take(self.stinger_takes[index], loaded.len());
        let track = loaded[position];
        let Some(buffer) = self.buffers.get(&shared_path(track)) else {
            return Ok(());
        };
        if matches!(stinger, Stinger::Victory | Stinger::Defeat)
            && !super::live_sfx::claim_ending_sting()
        {
            return Ok(());
        }
        let now = graph.context.current_time();
        let band = self
            .scheduled_band
            .filter(|(_, at)| now >= *at)
            .map_or(self.playing_band, |(band, _)| band);
        let when = next_pacing_beat(now, graph.started_at, band);
        let gain = graph.context.create_gain()?;
        gain.gain().set_value(STINGER_LEVEL);
        gain.connect_with_audio_node(&graph.stinger_bus)?;
        let source = graph.context.create_buffer_source()?;
        source.set_buffer(Some(buffer));
        source.connect_with_audio_node(&gain)?;
        source.start_with_when(when)?;
        self.stinger_takes[index] = Some(position);
        if self.fired.len() >= FIRED_MEMORY {
            self.fired.pop_front();
        }
        self.fired.push_back(track);
        Ok(())
    }

    /// Promotes an ending only after the stage reports that its newest beat is presented.
    fn release_presented_outcome(&mut self) {
        let Some(outcome) = self.pending_outcome else {
            return;
        };
        let Some((presented, latest)) = presentation_sequences(&self.document) else {
            return;
        };
        if presented < latest {
            return;
        }
        self.pending_outcome = None;
        let stinger = {
            let Some(current) = self.latest.as_mut() else {
                return;
            };
            let previous = current.facts;
            current.facts.outcome = Some(outcome);
            stinger_with_baseline(&previous, &current.facts, self.outcome_baseline)
        };
        if let Some(stinger) = stinger
            && let Err(error) = self.play_stinger(stinger)
        {
            self.fail(&error);
        }
    }

    /// The read-only debug snapshot: phase, intensity, target gains, buses, arrangement,
    /// colour, suite, takes, bed and ducking. A harness can check the whole mix from here
    /// without listening.
    fn publish(&mut self) {
        let ambient: Vec<_> = self
            .graph
            .as_ref()
            .into_iter()
            .flat_map(|graph| &graph.ambient_layers)
            .map(|voice| {
                serde_json::json!({"kind":voice.label,"path":voice.track,
                "gain_target":voice.target,"gain_now":voice.gain.gain().value()})
            })
            .collect();
        let _published = self.root.set_attribute(
            "data-music-ambient",
            &serde_json::json!(ambient).to_string(),
        );
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
        let buses = self.applied.map_or_else(
            || "null".to_owned(),
            |applied| {
                format!(
                    "{{\"score\":{},\"ambient\":{},\"stinger\":{},\"duck\":{},\"master\":{}}}",
                    self.graph
                        .as_ref()
                        .map_or(applied.score_bus, |graph| graph.score_bus.gain().value()),
                    applied.ambient,
                    applied.stinger_bus,
                    applied.duck,
                    applied.master
                )
            },
        );
        let arrangement = self.arrange_phase.map(|phase| {
            arrangements_for(phase)
                .get(self.arrangement)
                .map_or("unknown", |state| state.name)
        });
        let takes = self
            .stinger_takes
            .iter()
            .map(|take| take.map_or("null".to_owned(), |take| take.to_string()))
            .collect::<Vec<_>>()
            .join(",");
        // The duck gain sounding right now, sampled off the parameter (the buses carry the
        // target it is gliding toward), and the computed score level beside the voice level, so
        // a check asserts the 12 dB bar without listening.
        let duck_now = self
            .graph
            .as_ref()
            .map(|graph| (graph.duck.gain().value() * 1_000.0).round() / 1_000.0)
            .unwrap_or(1.0);
        let music_db = self
            .applied
            .map(|applied| mix_level_lufs(applied.layers))
            .unwrap_or(f32::NEG_INFINITY);
        let duck_log = list(self.duck_log.iter().map(String::as_str));
        let snapshot = format!(
            "{{\"sound\":{},\"context\":{},\"phase\":{},\"intensity\":{},\"sequence\":{},\"cue\":{},\
\"gains\":{gains},\"buses\":{buses},\"filter_hertz\":{},\"master\":{},\"bed\":{},\"bed_playing\":{},\
\"layers_started\":{},\"layers_playing\":{},\"suite\":{},\"arrangement\":{},\"colour\":{},\
\"alternate_takes\":{},\"stinger_takes\":[{takes}],\"ducked\":{},\"duck_reason\":{},\
\"duck_now\":{duck_now},\"music_db\":{},\"voice_db\":{:.1},\"duck_log\":{duck_log},\
\"tick_ms\":{:.1},\"tick_ms_max\":{:.1},\
\"sfx_count\":{},\"volume\":{},\"loaded\":{},\"missing\":{},\"stingers\":{},\"place\":{},\"pacing_band\":{},\"error\":{}}}",
            text(if self.enabled { "on" } else { "off" }),
            text(self.graph.as_ref().map_or("none", Graph::state_name)),
            optional_text(latest.map(|latest| latest.tempo.phase.name())),
            optional_number(latest.map(|latest| latest.tempo.intensity)),
            optional_number(latest.map(|latest| latest.tempo.sequence)),
            optional_number(latest.and_then(|latest| latest.tempo.cue)),
            optional_number(self.applied.map(|applied| applied.filter_hertz)),
            optional_number(self.applied.map(|applied| applied.master)),
            optional_text(latest.and_then(|latest| ambience_path_of(&latest.place))),
            optional_text(
                self.graph
                    .as_ref()
                    .and_then(|graph| graph.ambient.as_ref())
                    .map(|ambient| ambient.track.as_str())
            ),
            self.graph
                .as_ref()
                .is_some_and(|graph| graph.started_at.is_some()),
            list(self.layer_tracks.iter().flatten().map(String::as_str)),
            text(self.suite.name()),
            optional_text(arrangement),
            text(self.colour),
            self.layer_alternate,
            self.duck_reason != "none",
            text(self.duck_reason),
            finite_float(music_db),
            VOICE_LUFS,
            self.tick_ms,
            self.tick_ms_max,
            optional_number(self.sfx_count),
            self.volume,
            self.buffers.len(),
            list(self.missing.iter().map(String::as_str)),
            list(self.fired.iter().copied()),
            text(latest.map_or("none", |latest| match &latest.place {
                PlaceMusic::SharedScore => "shared",
                PlaceMusic::Fallback { .. } => "fallback",
                PlaceMusic::Ready { .. } => "ready",
            })),
            text(self.playing_band.label()),
            optional_text(self.failure.as_deref()),
        );
        if snapshot != self.published {
            let _published = self.root.set_attribute(STATE_ATTRIBUTE, &snapshot);
            self.published = snapshot;
        }
    }
}

/// `Stinger::ALL` order, matching `Director::stinger_takes`.
fn stinger_index(stinger: Stinger) -> usize {
    match stinger {
        Stinger::Reveal => 0,
        Stinger::Combat => 1,
        Stinger::Victory => 2,
        Stinger::Defeat => 3,
    }
}

/// A JSON string literal (the browser escapes it).
fn text(value: &str) -> String {
    js_sys::JSON::stringify(&JsValue::from_str(value))
        .ok()
        .and_then(|literal| literal.as_string())
        .unwrap_or_else(|| "null".to_owned())
}

/// A finite level in LUFS with one decimal, or null when there is no level (silence).
fn finite_float(value: f32) -> String {
    if value.is_finite() {
        format!("{value:.1}")
    } else {
        "null".to_owned()
    }
}

/// Wall-clock milliseconds on the page clock; 0 when the clock is unavailable.
fn performance_now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
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
    let (document, root, connection, tempo, facts, place, audio, lobby) = {
        let state = client.borrow();
        if state.role != "display" {
            return;
        }
        let Some(view) = state.views.as_ref().and_then(|views| views.current()) else {
            return;
        };
        let sequence = view
            .revision
            .as_ref()
            .and_then(|revision| revision.sequence)
            .unwrap_or(0);
        let Some(rpc::view_message::Audience::Display(display)) = &view.audience else {
            return;
        };
        let Some(journey) = display.journey.as_ref() else {
            return;
        };
        let empty = rpc::GameplayScene::default();
        let scene = display.scene.as_ref().unwrap_or(&empty);
        let tempo = tempo_of(scene, journey, sequence);
        let facts = facts_with_plan(scene, journey, display.place_music.as_ref());
        let place = place_music_of(display.place_music.as_ref());
        let audio = place_audio_of(scene);
        let lobby = journey.phase == rpc::JourneyPhase::Room as i32
            || journey.phase == rpc::JourneyPhase::CharacterCreation as i32;
        (
            state.document.clone(),
            state.root.clone(),
            state.generation,
            tempo,
            facts,
            place,
            audio,
            lobby,
        )
    };
    super::live_battlefield::set_tempo(tempo.intensity);
    match director(&document, &root) {
        Ok(shared) => {
            shared
                .borrow_mut()
                .accept(tempo, facts, place, connection, audio, lobby);
            tick(&shared);
        }
        Err(_) => {
            super::presentation_timing::sound(&root, "music", "failed");
            let _marked = root.set_attribute("data-music", "unavailable");
        }
    }
}

/// Equal-revision generated music is presentation data. The delivery owner calls this
/// alongside its art merge so a finished variant does not wait for a gameplay action.
pub(super) fn merge_plan(shown: &mut rpc::ViewMessage, new: &rpc::ViewMessage) -> bool {
    crate::music_score::merge_place_music(shown, new)
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
    // One seed per page: the arrangement rotation and the alternate-take pick vary session to
    // session, while the pure rules stay unit-testable with controlled seeds.
    let seed = js_sys::Date::now() as u64;
    let shared = Rc::new(RefCell::new(Director {
        audio_plan: None,
        entry_memory: EntryMemory::default(),
        pending_entry: None,
        entry_until: 0.0,
        entry_handoff_at: None,
        entry_voice: None,
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
        pending_outcome: None,
        outcome_baseline: None,
        listeners: Vec::new(),
        seed,
        layer_alternate: seed % 2 == 1,
        layer_tracks: None,
        layer_switch_until: 0.0,
        playing_band: PacingBand::Steady,
        scheduled_band: None,
        epoch: None,
        arrangement: 0,
        arrange_phase: None,
        arrange_bed: None,
        colour: "none",
        suite: Suite::Main,
        stinger_takes: [None; 4],
        sfx_count: None,
        sfx_dip_until: 0.0,
        line_seen_ms: None,
        caption_floor_until: 0.0,
        last_line_id: None,
        action_nodes: Vec::new(),
        duck_reason: "none",
        duck_log: VecDeque::new(),
        tick_ms: 0.0,
        tick_ms_max: 0.0,
        last_phase: None,
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
    if let Some(sound) = live_voice::sound_state()
        && let Err(error) = state.set_sound_output(sound.enabled)
    {
        state.fail(&error);
    }
    if state.graph.is_none() {
        match Graph::new() {
            Ok(graph) => state.graph = Some(graph),
            Err(error) => state.fail(&error),
        }
    }
    let resumed = state
        .graph
        .as_ref()
        .filter(|graph| state.enabled && graph.context.state() == AudioContextState::Suspended)
        .map(|graph| graph.context.resume());
    match resumed {
        Some(Ok(promise)) => settle(promise),
        Some(Err(error)) => state.fail(&error),
        None => {}
    }
}

fn tick(shared: &Shared) {
    let started = performance_now();
    let Ok(mut state) = shared.try_borrow_mut() else {
        return;
    };
    let wanted = match state.step() {
        Ok(wanted) => wanted,
        Err(error) => {
            super::presentation_timing::sound(&state.root, "music", "failed");
            state.fail(&error);
            Vec::new()
        }
    };
    state.release_presented_outcome();
    state.tick_ms = performance_now() - started;
    state.tick_ms_max = state.tick_ms_max.max(state.tick_ms);
    drop(state);
    for track in wanted {
        spawn_local(load(Rc::clone(shared), track));
    }
}

fn presentation_sequences(document: &Document) -> Option<(u64, u64)> {
    let stage = document.query_selector(".tv-stage").ok().flatten()?;
    let presented = stage
        .get_attribute("data-presented-seq")?
        .parse::<u64>()
        .ok()?;
    let latest = stage
        .get_attribute("data-latest-seq")?
        .parse::<u64>()
        .ok()?;
    Some((presented, latest))
}

/// Fetches and decodes one track. A missing or undecodable track is recorded and simply not
/// played; it is never requested again.
async fn load(shared: Shared, track: String) {
    let context = shared
        .borrow()
        .graph
        .as_ref()
        .map(|graph| graph.context.clone());
    let Some(context) = context else {
        return;
    };
    let outcome = fetch_track(&context, &track).await;
    {
        let mut state = shared.borrow_mut();
        match outcome {
            Ok(buffer) => {
                state.buffers.insert(track, buffer);
                state.release_unused_places();
            }
            Err(_) => state.missing.push(track),
        }
    }
    tick(&shared);
}

/// Anything but a non-empty, bounded 200 response that decodes is an error.
async fn fetch_track(context: &AudioContext, track: &str) -> Result<AudioBuffer, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let response: Response = JsFuture::from(window.fetch_with_str(track))
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
