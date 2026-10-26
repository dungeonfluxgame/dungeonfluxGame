//! Pure rules of the adaptive score: which tempo a display view implies, how that tempo mixes
//! the layers, which place music the server planned and which one-shot stinger a story change
//! earns.
//!
//! Nothing here touches the browser, so the rules are checked natively. The Web Audio player
//! (`gameplay_browser::live_music`) only applies what this module decides. The music follows
//! what the server already shows; it never decides game state.
// Native builds compile this module only to run its tests; the player that uses all of it is
// browser-only, so the native build sees part of it as unused.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use df_protocol::common as rpc;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "gameplay_browser/view_delivery.rs"]
mod delivery_boundary;

/// Versioned public scene extension; generated RPC scene_contents_json carries this alongside
/// the existing public environment facts. Paths remain ordinary hub-served audio routes.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlaceAudioPlan {
    pub(crate) version: u32,
    pub(crate) entry_stinger: String,
    pub(crate) layers: Vec<AmbientLayerPlan>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AmbientLayerPlan {
    pub(crate) kind: AmbientKind,
    pub(crate) key: String,
    pub(crate) status: String,
    pub(crate) detail: String,
    pub(crate) path: String,
    pub(crate) gain_permille: u16,
    pub(crate) speech_gain_permille: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AmbientKind {
    Water,
    Rain,
    Wind,
    Crowd,
    Fire,
    Work,
    Birds,
    Animals,
    Insects,
    Room,
}

impl AmbientKind {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) const ALL: [Self; 10] = [
        Self::Water,
        Self::Rain,
        Self::Wind,
        Self::Crowd,
        Self::Fire,
        Self::Work,
        Self::Birds,
        Self::Animals,
        Self::Insects,
        Self::Room,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Water => "water",
            Self::Rain => "rain",
            Self::Wind => "wind",
            Self::Crowd => "crowd",
            Self::Fire => "fire",
            Self::Work => "work",
            Self::Birds => "birds",
            Self::Animals => "animals",
            Self::Insects => "insects",
            Self::Room => "room",
        }
    }

    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Water => "water moving against the public environment's surfaces",
            Self::Rain => "rain falling on the public environment's surfaces",
            Self::Wind => "wind moving through the public environment",
            Self::Crowd => "indistinct crowd murmur without intelligible words",
            Self::Fire => "steady fire crackle",
            Self::Work => "distant repeated work sounds",
            Self::Birds => "birds present in the public environment",
            Self::Animals => "animals present in the public environment",
            Self::Insects => "insects present in the public environment",
            Self::Room => "quiet enclosed room tone",
        }
    }
}

/// Server mix policy: a busy public tempo raises each judged layer without restarting it.
/// Speech targets are carried explicitly so the player only follows actual line timing.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn ambient_gain(activity: u16, intensity: u16, speech: bool) -> u16 {
    let gain = 180
        + u32::from(activity.min(1000)) * 220 / 1000
        + u32::from(intensity.min(1000)) * 180 / 1000;
    (if speech { gain * 35 / 100 } else { gain }) as u16
}

/// Playback work at the browser boundary. A target-only update never starts a source.
#[derive(Debug, PartialEq)]
pub(crate) enum AmbientChange {
    Start(String, f32),
    Gain(String, f32),
    Stop(String),
}

pub(crate) fn ambient_changes(
    playing: &[(&str, f32)],
    desired: &[(&str, f32)],
) -> Vec<AmbientChange> {
    let mut changes = Vec::new();
    for (path, _) in playing {
        if !desired.iter().any(|(target, _)| target == path) {
            changes.push(AmbientChange::Stop((*path).to_owned()));
        }
    }
    for (path, gain) in desired {
        match playing.iter().find(|(current, _)| current == path) {
            None => changes.push(AmbientChange::Start((*path).to_owned(), *gain)),
            Some((_, old)) if old != gain => {
                changes.push(AmbientChange::Gain((*path).to_owned(), *gain))
            }
            Some(_) => {}
        }
    }
    changes
}

pub(crate) fn place_audio_of(scene: &rpc::GameplayScene) -> Option<PlaceAudioPlan> {
    let contents: serde_json::Value = serde_json::from_str(&scene.scene_contents_json).ok()?;
    let plan: PlaceAudioPlan = serde_json::from_value(contents.get("tempo_audio")?.clone()).ok()?;
    if plan.version != 1
        || plan.layers.len() > 3
        || ![
            "sting-reveal",
            "sting-combat",
            "sting-victory",
            "sting-defeat",
        ]
        .contains(&plan.entry_stinger.as_str())
        || plan.layers.iter().enumerate().any(|(index, layer)| {
            layer.gain_permille > 1000
                || layer.speech_gain_permille > layer.gain_permille
                || !["ready", "pending", "failed", "fallback"].contains(&layer.status.as_str())
                || (layer.status == "ready") == layer.path.is_empty()
                || plan.layers[..index]
                    .iter()
                    .any(|other| other.kind == layer.kind)
                || (!layer.path.is_empty()
                    && (layer.key.len() != 32
                        || !layer.key.bytes().all(|byte| byte.is_ascii_hexdigit())
                        || layer.path != format!("/gameplay/music/{}/ambient", layer.key)))
        })
    {
        return None;
    }
    Some(plan)
}

/// Empty public contents is the existing absence contract, so audio may add the first member.
pub(crate) fn scene_object_for_audio(
    contents: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    if contents.is_empty() {
        return Some(serde_json::Map::new());
    }
    match serde_json::from_str(contents).ok()? {
        serde_json::Value::Object(object) => Some(object),
        _ => None,
    }
}

/// A short opening uses a shared stinger's first bar at its recorded 84 BPM. The
/// score enters at the next bar within that clip, with a half-beat complementary crossfade.
/// Returns (handoff start, crossfade end), seconds relative to stinger start.
pub(crate) const ENTRY_CROSSFADE_SECONDS: f64 = BEAT_SECONDS / 2.0;

pub(crate) fn entry_handoff(duration: f64) -> Option<(f64, f64)> {
    let bar = 4.0 * BEAT_SECONDS;
    if !duration.is_finite() || duration < bar + ENTRY_CROSSFADE_SECONDS {
        return None;
    }
    Some((bar, bar + ENTRY_CROSSFADE_SECONDS))
}

/// An entry is an observed lobby-to-story edge. Reconnect and first snapshots never replay it.
#[derive(Default)]
pub(crate) struct EntryMemory {
    seen: Option<bool>,
}

impl EntryMemory {
    pub(crate) fn observe(&mut self, lobby: bool) -> bool {
        let entry = self.seen == Some(true) && !lobby;
        self.seen = Some(lobby);
        entry
    }
}

/// Tempo, key and loop length shared by every `score-*` layer and every stinger in
/// `assets/music`. Layers are cut from the same bar grid, so they stack sample-aligned.
pub(crate) const BPM: f64 = 84.0;
pub(crate) const BEAT_SECONDS: f64 = 60.0 / BPM;
/// 16 bars of 4/4: exactly 2 016 000 samples at 44.1 kHz (45.714 s).
pub(crate) const LOOP_SECONDS: f64 = 64.0 * BEAT_SECONDS;

/// REF-066: every pacing band has the same 2,016,000-frame loop. Different bar counts
/// let the player change tempo at the existing loop boundary without stretching audio.
/// Energy is a generation instruction (permille), independent of HS18's live layer gains.
///
/// | Band | BPM | Energy | 4/4 bars | Loop seconds |
/// | --- | ---: | ---: | ---: | ---: |
/// | calm | 63 | 200 | 12 | 45.7142857 |
/// | steady | 84 | 400 | 16 | 45.7142857 |
/// | driving | 105 | 650 | 20 | 45.7142857 |
/// | urgent | 126 | 850 | 24 | 45.7142857 |
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PacingBand {
    Calm,
    #[default]
    Steady,
    Driving,
    Urgent,
}

impl PacingBand {
    pub(crate) const ALL: [Self; 4] = [Self::Calm, Self::Steady, Self::Driving, Self::Urgent];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Calm => "calm",
            Self::Steady => "steady",
            Self::Driving => "driving",
            Self::Urgent => "urgent",
        }
    }

    pub(crate) fn bpm(self) -> u32 {
        match self {
            Self::Calm => 63,
            Self::Steady => 84,
            Self::Driving => 105,
            Self::Urgent => 126,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn bars(self) -> u32 {
        match self {
            Self::Calm => 12,
            Self::Steady => 16,
            Self::Driving => 20,
            Self::Urgent => 24,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn energy(self) -> u16 {
        match self {
            Self::Calm => 200,
            Self::Steady => 400,
            Self::Driving => 650,
            Self::Urgent => 850,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn neighbours(self) -> &'static [Self] {
        match self {
            Self::Calm => &[Self::Steady],
            Self::Steady => &[Self::Calm, Self::Driving],
            Self::Driving => &[Self::Steady, Self::Urgent],
            Self::Urgent => &[Self::Driving],
        }
    }
}

/// How long a Release keeps the resolving motif before the mix settles to the calm bed when
/// the tempo is derived from the view, or a cue arrives without a `hold_millis`. A server
/// Release cue carries the tuned hold in its `hold_millis`.
pub(crate) const RELEASE_HOLD_MILLIS: f64 = 14_000.0;
/// Line ducking: how far the music gets out of the way while a line is presented, by suite.
/// A presented line (voiced or caption-only, every role) always wins over the mix; the release
/// stays slow so the music returns smoothly instead of bouncing back up.
pub(crate) const DUCK_RELEASE_SECONDS: f64 = 0.6;
/// The duck is fully down within 150 ms of the voice starting: the housekeeping step finds a
/// new line within one 50 ms tick and this ramp finishes 80 ms later.
pub(crate) const DUCK_LINE_ATTACK_SECONDS: f64 = 0.08;
/// A ducked line holds through gaps shorter than this, so back-to-back lines never pump.
pub(crate) const LINE_GAP_HOLD_SECONDS: f64 = 0.8;
/// Thinning under speech: the low-pass closes to this and the two busiest layers halve, so
/// consonants (2-4 kHz) and percussion/brass transients stop masking words even before the
/// level duck has fully landed. Level alone leaves the words masked in a full battle mix.
pub(crate) const LINE_FILTER_HERTZ: f32 = 2_000.0;
pub(crate) const LINE_BUSY_SCALE: f32 = 0.5;
/// Measured loudness anchors for the depth math. Every score layer measures -23.4 LUFS
/// (`assets/music/source-manifest.json`); narration sits near -16 LUFS, the level the
/// effects engine normalises to (`live_sfx.rs`).
pub(crate) const LAYER_LUFS: f32 = -23.4;
pub(crate) const VOICE_LUFS: f32 = -16.0;
/// How far the music drops while a line is presented, by suite. Measured, not tasted: the
/// full battle mix (-20.9 LUFS by power sum over [0.63, 0.28, 0.55, 1.00]) needs -15 dB to
/// sit ~20 dB under the voice, and the quiet opening (-22.0 LUFS over [0.90, 0.75, 0, 0])
/// needs -12 dB to sit ~18 dB under it. Both clear the 12 dB bar with margin; thinning
/// (above) only adds to it. A battle dips heavier than a quiet scene, as it must.
pub(crate) const DUCK_BATTLE_DECIBELS: f32 = -15.0;
pub(crate) const DUCK_LINE_DECIBELS: f32 = -12.0;
/// A big sound effect dips the music briefly and shallowly: audible room, never a hole.
/// The voice cannot say which line speaks, so narrator and character lines share one depth
/// (a per-speaker hook is proposed in the MUSIC-001 handoff).
pub(crate) const DUCK_SFX_DECIBELS: f32 = -3.0;
pub(crate) const DUCK_SFX_ATTACK_SECONDS: f64 = 0.06;
pub(crate) const DUCK_SFX_RELEASE_SECONDS: f64 = 0.5;
/// How long after a big effect the shallow dip holds before releasing, in wall-clock seconds.
pub(crate) const DUCK_SFX_HOLD_SECONDS: f64 = 0.9;
/// Minimum layer glide when the phase itself changes (into/out of a fight, a scene change, the
/// end of the adventure): a transition is a crossfade, never a cut. Fast deliberate drops (the
/// Release percussion fall) keep their own shorter time; only slower glides are lengthened.
pub(crate) const TRANSITION_SECONDS: f64 = 3.0;

/// Moment-to-moment phase of the story, in the order the tempo engine describes them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Calm,
    Build,
    Anticipation,
    Peak,
    Release,
}

impl Phase {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Phase::Calm => "calm",
            Phase::Build => "build",
            Phase::Anticipation => "anticipation",
            Phase::Peak => "peak",
            Phase::Release => "release",
        }
    }
}

/// A continuous intensity (0..=1000) with its phase and the view sequence it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tempo {
    pub(crate) phase: Phase,
    pub(crate) intensity: u16,
    pub(crate) sequence: u64,
    /// The server cue's own sequence; `None` when the tempo was derived from the view.
    pub(crate) cue: Option<u64>,
    /// How long a Release lasts before the mix settles to calm.
    pub(crate) release_hold_millis: f64,
}

/// The newest server cue sequence already consumed on this watch connection.
///
/// A cue is consumed by its own sequence, whatever gameplay revision the view that carries it
/// has: the same cue arrives with many views and must not restart the release hold, and a cue
/// older than one already consumed is stale. The server resets its sequence on a new
/// authenticated watch connection, so the memory is reset with it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CueMemory {
    newest: Option<u64>,
}

impl CueMemory {
    /// Forgets the consumed sequence: a new watch connection starts a new sequence.
    pub(crate) fn reset(&mut self) {
        self.newest = None;
    }

    /// Whether `tempo` is news. A tempo derived from the view always is; a cue is news only
    /// when its sequence is newer than every one consumed so far.
    pub(crate) fn consume(&mut self, tempo: &Tempo) -> bool {
        let Some(cue) = tempo.cue else {
            return true;
        };
        if self.newest.is_some_and(|newest| cue <= newest) {
            return false;
        }
        self.newest = Some(cue);
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Victory,
    Defeat,
}

/// Where the story is, as far as stingers and the ambient bed care.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Facts {
    pub(crate) in_combat: bool,
    pub(crate) antagonist_speaking: bool,
    pub(crate) outcome: Option<Outcome>,
    pub(crate) bed: Bed,
}

/// Hides an ending until the display has acknowledged the newest resolved combat beat.
pub(crate) fn facts_after_presentation(
    mut facts: Facts,
    presented_sequence: u64,
    latest_sequence: u64,
) -> Facts {
    if presented_sequence < latest_sequence {
        facts.outcome = None;
    }
    facts
}

/// Location family of the ambient bed. Beds carry no melody, so every place shares one score.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Bed {
    Harbour,
    Market,
    Forest,
    Mountain,
    Library,
    Marsh,
    Graveyard,
    Farm,
    Night,
}

impl Bed {
    /// Every bed, which is also the closed vocabulary of setting families a place's music spec
    /// may name (`Night` is the neutral family).
    pub(crate) const ALL: [Bed; 9] = [
        Bed::Harbour,
        Bed::Market,
        Bed::Forest,
        Bed::Mountain,
        Bed::Library,
        Bed::Marsh,
        Bed::Graveyard,
        Bed::Farm,
        Bed::Night,
    ];

    /// The closed label used in content and on the wire.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Bed::Harbour => "harbour",
            Bed::Market => "market",
            Bed::Forest => "forest",
            Bed::Mountain => "mountain",
            Bed::Library => "library",
            Bed::Marsh => "marsh",
            Bed::Graveyard => "graveyard",
            Bed::Farm => "farm",
            Bed::Night => "neutral",
        }
    }

    /// The bed whose closed label is exactly `label`. Equality on a closed vocabulary only; it
    /// never looks inside free text.
    pub(crate) fn from_label(label: &str) -> Option<Bed> {
        Bed::ALL.into_iter().find(|bed| bed.label() == label)
    }

    /// Only the server names a bed's file (the plan carries the path); the display never does.
    /// The served track name, `/assets/music/<track>`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn track(self) -> &'static str {
        match self {
            Bed::Harbour => "amb-harbour",
            Bed::Market => "amb-market",
            Bed::Forest => "amb-forest",
            Bed::Mountain => "amb-mountain",
            Bed::Library => "amb-library",
            Bed::Marsh => "amb-marsh",
            Bed::Graveyard => "amb-graveyard",
            Bed::Farm => "amb-farm",
            Bed::Night => "amb-night",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stinger {
    Reveal,
    Combat,
    Victory,
    Defeat,
}

impl Stinger {
    pub(crate) const ALL: [Stinger; 4] = [
        Stinger::Reveal,
        Stinger::Combat,
        Stinger::Victory,
        Stinger::Defeat,
    ];
}

/// The four sample-aligned layers, in the order the player starts them.
pub(crate) const LAYER_TRACKS: [&str; 4] =
    ["score-bed", "score-motif", "score-tension", "score-combat"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LayerGains {
    pub(crate) bed: f32,
    pub(crate) motif: f32,
    pub(crate) tension: f32,
    pub(crate) combat: f32,
}

impl LayerGains {
    pub(crate) fn in_track_order(self) -> [f32; 4] {
        [self.bed, self.motif, self.tension, self.combat]
    }
}

/// What one phase asks of the mixer. Gains are linear; the low-pass opens as the story heats up.
/// `rise_seconds` and `fall_seconds` are the smoothing of a layer getting louder or quieter, so
/// a Release can drop percussion quickly while the rest settles slowly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Mix {
    pub(crate) layers: LayerGains,
    pub(crate) ambient: f32,
    pub(crate) filter_hertz: f32,
    pub(crate) rise_seconds: f64,
    pub(crate) fall_seconds: f64,
}

const fn mix(
    gains: [f32; 4],
    ambient: f32,
    filter_hertz: f32,
    rise_seconds: f64,
    fall_seconds: f64,
) -> Mix {
    Mix {
        layers: LayerGains {
            bed: gains[0],
            motif: gains[1],
            tension: gains[2],
            combat: gains[3],
        },
        ambient,
        filter_hertz,
        rise_seconds,
        fall_seconds,
    }
}

/// The tuning table: layer thresholds and gains per phase, `[bed, motif, tension, combat]`.
/// Kept as plain data so a tuning file can replace it later without touching the player.
/// Inertia is built in: every ramp is 1.5 to 4 seconds except the deliberate 0.8 second drop
/// of percussion and tension when a Release begins.
pub(crate) const MIX_TABLE: [(Phase, Mix); 5] = [
    // Calm: the bed alone, a little muffled, with the location's ambience forward.
    (
        Phase::Calm,
        mix([0.90, 0.0, 0.0, 0.0], 1.00, 1_400.0, 4.0, 4.0),
    ),
    // Build: the melodic motif enters over the bed.
    (
        Phase::Build,
        mix([0.90, 0.75, 0.0, 0.0], 0.85, 3_200.0, 3.0, 4.0),
    ),
    // Anticipation: the low pulse and tremolo strings join; the filter opens.
    (
        Phase::Anticipation,
        mix([0.80, 0.50, 0.80, 0.0], 0.60, 6_000.0, 2.5, 3.0),
    ),
    // Peak: percussion and brass at full over a quieter bed, filter wide open.
    (
        Phase::Peak,
        mix([0.70, 0.35, 0.55, 1.00], 0.35, 18_000.0, 1.5, 2.0),
    ),
    // Release: percussion and tension fall away in 0.8 s, the motif resolves.
    (
        Phase::Release,
        mix([0.90, 0.80, 0.0, 0.0], 0.85, 4_000.0, 3.0, 0.8),
    ),
];

/// A defeat is sombre: the motif stays low and the sound muffled.
const SOMBRE_MOTIF_SCALE: f32 = 0.45;
pub(crate) const SOMBRE_FILTER_HERTZ: f32 = 1_800.0;
/// Within Anticipation and Peak the defining layer scales with intensity between these floors
/// and 1000, never below this fraction of its table gain.
const MINIMUM_EMPHASIS: f32 = 0.7;
const ANTICIPATION_FLOOR: u16 = 450;
const PEAK_FLOOR: u16 = 750;

pub(crate) fn mix_for(phase: Phase, intensity: u16, sombre: bool) -> Mix {
    let mut mix = MIX_TABLE
        .iter()
        .find(|(candidate, _)| *candidate == phase)
        .map_or(MIX_TABLE[0].1, |(_, mix)| *mix);
    match phase {
        Phase::Anticipation => {
            mix.layers.tension *= emphasis(intensity, ANTICIPATION_FLOOR);
        }
        Phase::Peak => {
            mix.layers.combat *= emphasis(intensity, PEAK_FLOOR);
        }
        _ => {}
    }
    if sombre {
        mix.layers.motif *= SOMBRE_MOTIF_SCALE;
        mix.filter_hertz = mix.filter_hertz.min(SOMBRE_FILTER_HERTZ);
    }
    mix
}

/// Keeps an intense social/reveal cue urgent without importing combat percussion into a
/// non-combat scene. The server tempo phase remains unchanged and still drives the melodic and
/// tension layers; the scene facts decide whether the combat layer is public and appropriate.
pub(crate) fn scene_layer_gains(mut layers: [f32; 4], in_combat: bool) -> [f32; 4] {
    if !in_combat {
        // Social scenes keep the same tempo arc, but leave room for speech and the location bed.
        // The tension pulse is reduced more than the bed/motif; combat retains the full table.
        layers[0] *= 0.78;
        layers[1] *= 0.78;
        layers[2] *= 0.55;
        layers[3] = 0.0;
    }
    layers
}

fn emphasis(intensity: u16, floor: u16) -> f32 {
    let span = f32::from(1_000 - floor);
    let above = f32::from(intensity.clamp(floor, 1_000) - floor);
    MINIMUM_EMPHASIS + (1.0 - MINIMUM_EMPHASIS) * above / span
}

/// Linear gain for a ducking depth in decibels.
pub(crate) fn gain_for_decibels(decibels: f32) -> f32 {
    10_f32.powf(decibels / 20.0)
}

/// The duck depth for what is actually playing: a battle dips heavier than a quiet scene.
pub(crate) fn duck_decibels(suite: Suite) -> f32 {
    match suite {
        Suite::Battle => DUCK_BATTLE_DECIBELS,
        Suite::Main | Suite::Finale => DUCK_LINE_DECIBELS,
    }
}

/// Whether the scene line machine's published state means a line is being heard or read.
/// The player reads this off the stage root's `data-line-state`: the narrator's box, every
/// character's and hero's bubble, fight cries and commentary, and the loading-screen mirror
/// all run this same machine, so no role is exempt. Intent loaders (`composing`,
/// `submitted`), lingering captions, and closed lines do not duck: typing is not a line, and a
/// finished one is over.
pub(crate) fn line_presenting(line_state: &str) -> bool {
    matches!(line_state, "opening" | "speaking")
}

/// Whether a line seen this recently still holds the duck: gaps shorter than
/// `LINE_GAP_HOLD_SECONDS` never pump the music between back-to-back lines.
pub(crate) fn gap_hold_active(ms_since_line: f64) -> bool {
    ms_since_line < LINE_GAP_HOLD_SECONDS * 1_000.0
}

/// How long an unvoiced (caption-only) line holds the duck at minimum: the display's own
/// reading-time rule, reused here — never a second rule. The line machine itself stays in
/// its speaking state for this long, so following the machine holds through the reading;
/// the player additionally floors a newly seen caption line at the minimum below.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn unvoiced_hold_ms(text: &str) -> u32 {
    crate::stage_machines::reading_time_ms(text)
}

/// How long a newly appeared action call-out holds the duck: the bubble's own `--hold`
/// dwell from its inline style (`--appear:0ms;--hold:5500ms;--fade:500ms`). These bubbles
/// fade by CSS but stay in the DOM, so the music reads the timing the display already
/// computed for that bubble, never a second rule. `None` when the style carries no hold:
/// machine-driven bubbles carry no inline style, and the machine already covers those.
pub(crate) fn bubble_hold_ms(style: &str) -> Option<f64> {
    style.split(';').find_map(|part| {
        let (name, value) = part.split_once(':')?;
        if name.trim() != "--hold" {
            return None;
        }
        value
            .trim()
            .strip_suffix("ms")?
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|ms| *ms > 0.0)
    })
}

/// The mixed score level in LUFS for linear layer gains, by power sum over the measured
/// per-layer loudness. Ambience (~-30 LUFS at fractional gains) is negligible beside it, so
/// the score alone decides the depth. Silence (all zero) has no level: negative infinity.
pub(crate) fn mix_level_lufs(layers: [f32; 4]) -> f32 {
    let power: f32 = layers.iter().map(|gain| gain * gain).sum();
    if power <= 0.0 {
        return f32::NEG_INFINITY;
    }
    LAYER_LUFS + 10.0 * power.log10()
}

/// How often the arrangement may move, in audio-clock seconds: half a loop (8 bars at 84 BPM,
/// ~22.9 s). The player advances the arrangement only when this epoch changes, so every change
/// lands on a bar boundary and glides have room to breathe.
pub(crate) const ARRANGE_SECONDS: f64 = LOOP_SECONDS / 2.0;

/// One voicing of a phase: multipliers on the phase table's `[bed, motif, tension, combat]`
/// gains, in track order. Scales only ever thin the table out; an arrangement never adds a
/// layer the phase did not ask for. `stillness` (all zero) leaves the ambience alone: quiet
/// scenes are allowed real quiet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Arrangement {
    pub(crate) name: &'static str,
    pub(crate) scales: [f32; 4],
}

const fn arrangement(name: &'static str, scales: [f32; 4]) -> Arrangement {
    Arrangement { name, scales }
}

const CALM_ARRANGEMENTS: [Arrangement; 3] = [
    arrangement("bed", [1.0, 1.0, 1.0, 1.0]),
    arrangement("bed-soft", [0.6, 1.0, 1.0, 1.0]),
    arrangement("stillness", [0.0, 0.0, 0.0, 0.0]),
];
const BUILD_ARRANGEMENTS: [Arrangement; 3] = [
    arrangement("bed-and-motif", [1.0, 1.0, 1.0, 1.0]),
    arrangement("motif-resting", [1.0, 0.0, 1.0, 1.0]),
    arrangement("bed-alone", [0.8, 0.0, 1.0, 1.0]),
];
const ANTICIPATION_ARRANGEMENTS: [Arrangement; 3] = [
    arrangement("full", [1.0, 1.0, 1.0, 1.0]),
    arrangement("pulse-only", [0.7, 0.0, 1.0, 1.0]),
    arrangement("motif-resting", [1.0, 0.0, 1.0, 1.0]),
];
const PEAK_ARRANGEMENTS: [Arrangement; 3] = [
    arrangement("full", [1.0, 1.0, 1.0, 1.0]),
    arrangement("motif-resting", [1.0, 0.0, 1.0, 1.0]),
    arrangement("tension-out", [1.0, 1.0, 0.0, 1.0]),
];
const RELEASE_ARRANGEMENTS: [Arrangement; 3] = [
    arrangement("resolving", [1.0, 1.0, 1.0, 1.0]),
    arrangement("bed-alone", [1.0, 0.0, 1.0, 1.0]),
    arrangement("stillness", [0.0, 0.0, 0.0, 0.0]),
];

/// The voicings one phase may rotate through, in a fixed order.
pub(crate) fn arrangements_for(phase: Phase) -> &'static [Arrangement] {
    match phase {
        Phase::Calm => &CALM_ARRANGEMENTS,
        Phase::Build => &BUILD_ARRANGEMENTS,
        Phase::Anticipation => &ANTICIPATION_ARRANGEMENTS,
        Phase::Peak => &PEAK_ARRANGEMENTS,
        Phase::Release => &RELEASE_ARRANGEMENTS,
    }
}

/// Which voicing sounds: a pure deterministic function of the phase, the place, a session seed
/// and the arrangement epoch (`epoch` counts `ARRANGE_SECONDS` slices of the audio clock, so a
/// controlled clock makes it unit-testable). The state never repeats its immediate predecessor
/// while another exists.
pub(crate) fn arrangement_for(
    phase: Phase,
    bed: Bed,
    seed: u64,
    epoch: u64,
    previous: Option<usize>,
) -> usize {
    let states = arrangements_for(phase);
    let count = states.len();
    if count <= 1 {
        return 0;
    }
    let hash = arrangement_hash(seed, phase, bed, epoch);
    let mut index = (hash % count as u64) as usize;
    if previous == Some(index) {
        let stride = 1 + (hash >> 33) as usize % (count - 1);
        index = (index + stride) % count;
    }
    index
}

fn arrangement_hash(seed: u64, phase: Phase, bed: Bed, epoch: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for mut word in [seed, phase as u64, bed as u64, epoch] {
        for _ in 0..8 {
            hash ^= word & 0xff;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
            word >>= 8;
        }
    }
    hash
}

/// Place and chapter colour: the same four layers, coloured per place or fight with what Web
/// Audio already offers (filter, balance, ambience level). Derived only from facts the display
/// view and the server's place music plan carry (the setting family, combat). Scales multiply the
/// arrangement-weighted table gains; the filter scale multiplies the phase filter in Hertz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Colour {
    pub(crate) name: &'static str,
    pub(crate) filter_scale: f32,
    pub(crate) ambient_scale: f32,
    pub(crate) layers: [f32; 4],
}

/// The colour for a place, or for a fight anywhere. A fight never sounds like the same room it
/// interrupted: combat has its own forward colour whatever the bed.
pub(crate) fn colour_for(bed: Bed, in_combat: bool) -> Colour {
    if in_combat {
        return Colour {
            name: "battle-forward",
            filter_scale: 1.0,
            ambient_scale: 0.8,
            layers: [0.9, 0.8, 1.0, 1.0],
        };
    }
    match bed {
        Bed::Harbour | Bed::Market | Bed::Farm | Bed::Library => Colour {
            name: "hearth-warm",
            filter_scale: 0.65,
            ambient_scale: 1.1,
            layers: [1.0, 1.0, 0.9, 1.0],
        },
        Bed::Forest | Bed::Mountain | Bed::Marsh => Colour {
            name: "wild-open",
            filter_scale: 1.15,
            ambient_scale: 1.2,
            layers: [0.9, 1.0, 1.0, 1.0],
        },
        Bed::Graveyard | Bed::Night => Colour {
            name: "night-thin",
            filter_scale: 0.8,
            ambient_scale: 1.0,
            layers: [0.9, 0.85, 1.1, 1.0],
        },
    }
}

/// Which score suite plays: the main suite, the battle suite for combat chapters, the finale
/// suite once an outcome is decided. Derived from facts the view already carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Suite {
    Main,
    Battle,
    Finale,
}

impl Suite {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Suite::Main => "main",
            Suite::Battle => "battle",
            Suite::Finale => "finale",
        }
    }
}

pub(crate) fn suite_for(in_combat: bool, outcome: Option<Outcome>) -> Suite {
    if in_combat {
        Suite::Battle
    } else if outcome.is_some() {
        Suite::Finale
    } else {
        Suite::Main
    }
}

/// The served track for one score layer: `/assets/music/<track>`. `layer` is the track-order
/// index into `LAYER_TRACKS`; out-of-range layers fall back to the bed so a bad index is silent,
/// never a panic. `alternate` picks the `-b` take where the suite has one (the main suite only);
/// a suite that has no alternates plays its own base take.
pub(crate) fn layer_track(suite: Suite, layer: usize, alternate: bool) -> &'static str {
    match (suite, layer, alternate) {
        (Suite::Main, 0, false) => "score-bed",
        (Suite::Main, 1, false) => "score-motif",
        (Suite::Main, 2, false) => "score-tension",
        (Suite::Main, 3, false) => "score-combat",
        (Suite::Main, 0, true) => "score-bed-b",
        (Suite::Main, 1, true) => "score-motif-b",
        (Suite::Main, 2, true) => "score-tension",
        (Suite::Main, 3, true) => "score-combat",
        (Suite::Battle, 0, _) => "b-score-bed",
        (Suite::Battle, 1, _) => "b-score-motif",
        (Suite::Battle, 2, _) => "b-score-tension",
        (Suite::Battle, 3, _) => "b-score-combat",
        // Until recorded, Finale reuses Main's quieter set; missing alternates use base tracks.
        (Suite::Finale, 0, _) => "score-bed-b",
        (Suite::Finale, 1, _) => "score-motif-b",
        (Suite::Finale, 2, _) => "score-tension",
        (Suite::Finale, 3, _) => "score-combat",
        _ => "score-bed",
    }
}

/// How many served takes each stinger has: the base take plus `-2` and `-3`.
pub(crate) const STINGER_TAKES: usize = 3;

/// The served track for one take of a stinger. Out-of-range takes fall back to the base take.
pub(crate) fn stinger_track(kind: Stinger, take: usize) -> &'static str {
    match (kind, take) {
        (Stinger::Reveal, 0) => "sting-reveal",
        (Stinger::Reveal, 1) => "sting-reveal-2",
        (Stinger::Reveal, 2) => "sting-reveal-3",
        (Stinger::Combat, 0) => "sting-combat",
        (Stinger::Combat, 1) => "sting-combat-2",
        (Stinger::Combat, 2) => "sting-combat-3",
        (Stinger::Victory, 0) => "sting-victory",
        (Stinger::Victory, 1) => "sting-victory-2",
        (Stinger::Victory, 2) => "sting-victory-3",
        (Stinger::Defeat, 0) => "sting-defeat",
        (Stinger::Defeat, 1) => "sting-defeat-2",
        (Stinger::Defeat, 2) => "sting-defeat-3",
        (Stinger::Reveal, _) => "sting-reveal",
        (Stinger::Combat, _) => "sting-combat",
        (Stinger::Victory, _) => "sting-victory",
        (Stinger::Defeat, _) => "sting-defeat",
    }
}

/// The next take to play: round-robin from the last one, so the same take never plays twice
/// running while another exists. `takes` is how many takes are actually loaded; with one take
/// (or none) there is nothing to rotate and the base take plays.
pub(crate) fn next_take(last: Option<usize>, takes: usize) -> usize {
    if takes <= 1 {
        return 0;
    }
    match last {
        None => 0,
        Some(previous) => (previous + 1) % takes,
    }
}

/// The next beat of the layer grid that is still far enough ahead to schedule. The layers
/// started together at `started_at`, so a one-shot placed on a beat lands in time with them.
/// Before the layers run there is no grid and the one-shot plays at once.
#[cfg(test)]
pub(crate) fn next_beat(now: f64, started_at: Option<f64>) -> f64 {
    next_pacing_beat(now, started_at, PacingBand::Steady)
}

pub(crate) fn next_pacing_beat(now: f64, started_at: Option<f64>, band: PacingBand) -> f64 {
    const MINIMUM_LEAD_SECONDS: f64 = 0.02;
    let Some(start) = started_at.filter(|start| *start <= now) else {
        return now;
    };
    let beat_seconds = 60.0 / f64::from(band.bpm());
    let beat = start + ((now - start) / beat_seconds).ceil() * beat_seconds;
    if beat - now < MINIMUM_LEAD_SECONDS {
        beat + beat_seconds
    } else {
        beat
    }
}

/// A Release settles to the calm bed once its hold has passed.
pub(crate) fn effective_phase(tempo: Tempo, release_held_millis: f64) -> Phase {
    if tempo.phase == Phase::Release && release_held_millis >= tempo.release_hold_millis {
        Phase::Calm
    } else {
        tempo.phase
    }
}

/// THE SINGLE PLACE THE TEMPO COMES FROM. The server's `TempoCue { intensity_permille, phase,
/// sequence, hold_millis }` when the view carries one; only when the cue is absent (or names
/// no phase) is it derived from what the display view already shows.
pub(crate) fn tempo_of(
    scene: &rpc::GameplayScene,
    journey: &rpc::JourneyView,
    sequence: u64,
) -> Tempo {
    if let Some(cue) = journey
        .tempo
        .as_ref()
        .and_then(|cue| tempo_from_cue(cue, sequence))
    {
        return cue;
    }
    let facts = facts_of(scene, journey, "");
    let (phase, intensity) = match rpc::JourneyPhase::try_from(journey.phase) {
        Ok(rpc::JourneyPhase::Opening) => (Phase::Build, 350),
        Ok(rpc::JourneyPhase::Dialogue) if facts.antagonist_speaking => (Phase::Anticipation, 700),
        Ok(rpc::JourneyPhase::Dialogue) => (Phase::Anticipation, 550),
        Ok(rpc::JourneyPhase::Combat) => (Phase::Peak, combat_intensity(journey)),
        Ok(rpc::JourneyPhase::Complete) => complete_tempo(scene, journey, facts.outcome),
        _ => (Phase::Calm, 150),
    };
    Tempo {
        phase,
        intensity,
        sequence,
        cue: None,
        release_hold_millis: RELEASE_HOLD_MILLIS,
    }
}

fn tempo_from_cue(cue: &rpc::TempoCue, sequence: u64) -> Option<Tempo> {
    let phase = match rpc::TempoPhase::try_from(cue.phase).ok()? {
        rpc::TempoPhase::Calm => Phase::Calm,
        rpc::TempoPhase::Build => Phase::Build,
        rpc::TempoPhase::Anticipation => Phase::Anticipation,
        rpc::TempoPhase::Peak => Phase::Peak,
        rpc::TempoPhase::Release => Phase::Release,
        rpc::TempoPhase::Unspecified => return None,
    };
    Some(Tempo {
        phase,
        intensity: u16::try_from(cue.intensity_permille.min(1_000)).unwrap_or(1_000),
        sequence,
        cue: Some(cue.sequence),
        // The server sends the hold with a Release cue only; no hold means the standard one.
        release_hold_millis: if cue.hold_millis == 0 {
            RELEASE_HOLD_MILLIS
        } else {
            f64::from(cue.hold_millis)
        },
    })
}

/// 750 to 1000: starts high and climbs as the most hurt hero's health fraction falls and as
/// the rounds go on.
fn combat_intensity(journey: &rpc::JourneyView) -> u16 {
    let lowest_permille = journey
        .party
        .iter()
        .filter(|hero| hero.maximum_hit_points > 0)
        .map(|hero| {
            u64::from(hero.hit_points.min(hero.maximum_hit_points)) * 1_000
                / u64::from(hero.maximum_hit_points)
        })
        .min()
        .unwrap_or(1_000);
    let rounds = journey
        .combat
        .as_ref()
        .map_or(0, |combat| combat.round.min(5));
    let wounded = 1_000_u64.saturating_sub(lowest_permille) / 5;
    let intensity = 750 + wounded + u64::from(rounds) * 10;
    u16::try_from(intensity.min(1_000)).unwrap_or(1_000)
}

fn complete_tempo(
    scene: &rpc::GameplayScene,
    journey: &rpc::JourneyView,
    outcome: Option<Outcome>,
) -> (Phase, u16) {
    if scene.destination == rpc::HarborDestination::HarborInn as i32 {
        return (Phase::Calm, 200);
    }
    match outcome {
        Some(Outcome::Victory) => (Phase::Release, 450),
        Some(Outcome::Defeat) => (Phase::Release, 200),
        None if journey.combat.is_some() => (Phase::Release, 350),
        None => (Phase::Release, 300),
    }
}

/// Facts for stingers, the sombre defeat mix and the place colour. The bed is the place's
/// setting family, which only the server's music plan names (`facts_with_plan`); without a plan
/// it is the neutral family. `display_asset` is ignored: an asset name decides nothing about
/// the music. It is kept so existing callers keep their call shape; new code calls
/// `facts_with_plan`.
pub(crate) fn facts_of(
    scene: &rpc::GameplayScene,
    journey: &rpc::JourneyView,
    _display_asset: &str,
) -> Facts {
    facts_with_plan(scene, journey, None)
}

/// Facts from the server's own data only: the journey phase and party, the combat state, and
/// the scene cast's role and speaking flags. No asset, portrait or speaker name is read. The
/// setting family is the one the server's music plan names, or neutral when there is none.
pub(crate) fn facts_with_plan(
    scene: &rpc::GameplayScene,
    journey: &rpc::JourneyView,
    plan: Option<&rpc::PlaceMusicPlan>,
) -> Facts {
    let phase = rpc::JourneyPhase::try_from(journey.phase).ok();
    let finished = journey
        .combat
        .as_ref()
        .is_some_and(|combat| combat.finished);
    let outcome = (phase == Some(rpc::JourneyPhase::Complete)
        && finished
        && scene.destination != rpc::HarborDestination::HarborInn as i32)
        .then(|| {
            if journey
                .party
                .iter()
                .any(|hero| hero.hit_points > 0 && !hero.unconscious)
            {
                Outcome::Victory
            } else {
                Outcome::Defeat
            }
        });
    Facts {
        in_combat: phase == Some(rpc::JourneyPhase::Combat),
        // The cast's own role says who is speaking: the server sets `speaking` and `role` on
        // every scene actor.
        antagonist_speaking: phase == Some(rpc::JourneyPhase::Dialogue)
            && scene
                .cast
                .iter()
                .any(|actor| actor.speaking && actor.role == rpc::SceneRole::Antagonist as i32),
        outcome,
        bed: plan
            .and_then(|plan| Bed::from_label(&plan.setting_family))
            .unwrap_or(Bed::Night),
    }
}

/// What the display should play for the place it is in, from the server's plan. The shared
/// score is always the fallback; the place's own layers are used only when the plan says they
/// are ready and they share the shared score's timebase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PlaceMusic {
    /// No plan, or one the client cannot use: the shared score, no ambience.
    SharedScore,
    /// The shared score with an existing ambience bed.
    Fallback { ambience: Option<String> },
    /// The place's generated layers (bed, motif, tension, combat) and ambience.
    Ready {
        key: String,
        band: PacingBand,
        layers: [String; 4],
        ambience: Option<String>,
    },
}

/// The route generated place sets are served from, and the one existing beds are served from.
pub(crate) const GENERATED_MUSIC_ROUTE: &str = "/gameplay/music/";
pub(crate) const SHARED_MUSIC_ROUTE: &str = "/assets/music/";
/// Sample rate of the score and of every generated layer.
pub(crate) const SAMPLE_RATE: u32 = 44_100;
/// Frames in one 16-bar loop: 64 beats at 84 BPM.
pub(crate) const LOOP_SAMPLES: u32 = 2_016_000;

fn served_music_path(path: &str) -> bool {
    (path.starts_with(GENERATED_MUSIC_ROUTE) || path.starts_with(SHARED_MUSIC_ROUTE))
        && path.split('/').all(|segment| segment != "..")
        && path.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

/// Reads the server's plan. A set whose timebase differs from the shared score's, or whose
/// paths are not served music paths, is not used: the explicit fallback of the tempo design.
pub(crate) fn place_music_of(plan: Option<&rpc::PlaceMusicPlan>) -> PlaceMusic {
    let Some(plan) = plan else {
        return PlaceMusic::SharedScore;
    };
    let ambience = Some(plan.ambience_path.clone()).filter(|path| served_music_path(path));
    let state = rpc::PlaceMusicState::try_from(plan.state).ok();
    if state != Some(rpc::PlaceMusicState::Ready) {
        return PlaceMusic::Fallback { ambience };
    }
    let band = PacingBand::ALL
        .into_iter()
        .find(|band| band.bpm() == plan.bpm);
    let same_timebase = plan.sample_rate == SAMPLE_RATE && plan.loop_samples == LOOP_SAMPLES;
    let layers: Result<[String; 4], _> = plan.layer_paths.clone().try_into();
    match (layers, band) {
        (Ok(layers), Some(band))
            if same_timebase
                && layers
                    .iter()
                    .all(|path| path.starts_with(GENERATED_MUSIC_ROUTE))
                && layers.iter().all(|path| served_music_path(path)) =>
        {
            PlaceMusic::Ready {
                key: plan.spec_key.clone(),
                band,
                layers,
                ambience,
            }
        }
        _ => PlaceMusic::Fallback { ambience },
    }
}

/// Copies only a validated server music plan for the same public place. Revision and
/// connection freshness are checked by the delivery owner before calling this boundary.
pub(crate) fn merge_place_music(shown: &mut rpc::ViewMessage, new: &rpc::ViewMessage) -> bool {
    let (
        Some(rpc::view_message::Audience::Display(shown)),
        Some(rpc::view_message::Audience::Display(new)),
    ) = (shown.audience.as_mut(), new.audience.as_ref())
    else {
        return false;
    };
    let (Some(old_scene), Some(new_scene)) = (&shown.scene, &new.scene) else {
        return false;
    };
    if old_scene.location_id != new_scene.location_id
        || old_scene.title != new_scene.title
        || old_scene.source_scene_key != new_scene.source_scene_key
    {
        return false;
    }
    if new.place_music.as_ref().is_some_and(|plan| {
        plan.state == rpc::PlaceMusicState::Ready as i32
            && !matches!(place_music_of(Some(plan)), PlaceMusic::Ready { .. })
    }) {
        return false;
    }
    let music_changed = shown.place_music != new.place_music;
    // Equal contents carry equal tempo_audio: a duplicate view returns before any JSON parse.
    let contents_changed = old_scene.scene_contents_json != new_scene.scene_contents_json;
    if !music_changed && !contents_changed {
        return false;
    }
    let audio = contents_changed
        .then(|| place_audio_of(new_scene))
        .flatten();
    let mut audio_contents = scene_object_for_audio(&old_scene.scene_contents_json);
    let audio_changed = audio_contents.is_some()
        && audio
            .as_ref()
            .is_some_and(|plan| Some(plan.clone()) != place_audio_of(old_scene));
    if !audio_changed && !music_changed {
        return false;
    }
    if audio_changed
        && let Some(scene) = shown.scene.as_mut()
        && let Some(mut contents) = audio_contents.take()
    {
        contents.insert("tempo_audio".into(), serde_json::json!(audio));
        scene.scene_contents_json = serde_json::Value::Object(contents).to_string();
    }
    shown.place_music.clone_from(&new.place_music);
    true
}

/// The served path of a shared-score or bed track name.
pub(crate) fn shared_path(track: &str) -> String {
    format!("{SHARED_MUSIC_ROUTE}{track}")
}

/// The four layer paths to loop for `suite`. The place's own layers replace the main and finale
/// suites once they are ready; a fight keeps the shared battle suite, because it never sounds
/// like the room it interrupted. Until the place's layers are ready, and whenever they cannot be
/// used, the shared score plays.
pub(crate) fn wanted_layer_paths(suite: Suite, alternate: bool, place: &PlaceMusic) -> [String; 4] {
    match (suite, place) {
        (Suite::Battle, _) | (_, PlaceMusic::SharedScore | PlaceMusic::Fallback { .. }) => {
            [0, 1, 2, 3].map(|layer| shared_path(layer_track(suite, layer, alternate)))
        }
        (_, PlaceMusic::Ready { layers, .. }) => layers.clone(),
    }
}

/// The ambience path the place asks for, if any. Without one the score plays alone.
pub(crate) fn ambience_path_of(place: &PlaceMusic) -> Option<&str> {
    match place {
        PlaceMusic::SharedScore => None,
        PlaceMusic::Fallback { ambience } | PlaceMusic::Ready { ambience, .. } => {
            ambience.as_deref()
        }
    }
}

/// When new layer voices should start so the swap lands on the loop grid: the next loop
/// boundary of the layers that started together at `started_at`, pushed one loop later when
/// that is closer than the crossfade plus the scheduling lead. The old voices fade out across
/// the same crossfade the new ones fade in across.
pub(crate) fn swap_boundary(
    now: f64,
    started_at: f64,
    crossfade_seconds: f64,
    lead_seconds: f64,
) -> f64 {
    let elapsed = (now - started_at).max(0.0);
    let boundary = started_at + (elapsed / LOOP_SECONDS).ceil() * LOOP_SECONDS;
    if boundary - now < crossfade_seconds + lead_seconds {
        boundary + LOOP_SECONDS
    } else {
        boundary
    }
}

/// The stinger a change between two accepted views earns, if any. One stinger per change:
/// an outcome outranks a battle start, which outranks an antagonist reveal.
/// An outcome already present on reconnect may update the bed after presentation, but never
/// replays its fanfare. The caller clears this baseline when a fresh fight starts.
pub(crate) fn stinger_with_baseline(
    previous: &Facts,
    next: &Facts,
    baseline: Option<Outcome>,
) -> Option<Stinger> {
    stinger_between(previous, next).filter(|stinger| {
        !matches!(stinger, Stinger::Victory | Stinger::Defeat) || next.outcome != baseline
    })
}

pub(crate) fn stinger_between(previous: &Facts, next: &Facts) -> Option<Stinger> {
    if next.outcome != previous.outcome {
        match next.outcome {
            Some(Outcome::Victory) => return Some(Stinger::Victory),
            Some(Outcome::Defeat) => return Some(Stinger::Defeat),
            None => {}
        }
    }
    if !previous.in_combat && next.in_combat {
        return Some(Stinger::Combat);
    }
    (!previous.antagonist_speaking && next.antagonist_speaking).then_some(Stinger::Reveal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn hs54_reconnect_outcome_baseline_suppresses_deferred_fanfare() {
        let before = Facts {
            in_combat: false,
            antagonist_speaking: false,
            outcome: None,
            bed: Bed::Night,
        };
        for outcome in [Outcome::Victory, Outcome::Defeat] {
            let next = Facts {
                outcome: Some(outcome),
                ..before
            };
            assert!(stinger_between(&before, &next).is_some());
            assert_eq!(stinger_with_baseline(&before, &next, Some(outcome)), None);
            assert!(stinger_with_baseline(&before, &next, None).is_some());
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn live_music_equal_revision_ambient_completion_reaches_the_actual_delivery_store() {
        use super::delivery_boundary::{GameplayViews, ViewRole, ViewScope};
        let scope = ViewScope::from_wire(
            Some(&rpc::SessionId {
                value: Some(vec![1; 16]),
            }),
            Some(&rpc::RunId {
                value: Some(vec![2; 16]),
            }),
            Some(&rpc::ClientBindingId {
                value: Some(vec![3; 16]),
            }),
            ViewRole::Display,
        )
        .unwrap();
        let connection = df_client::connection::RpcConnection::new(());
        let generation = connection.generation();
        let mut views = GameplayViews::new(scope, generation.clone());
        let key = "a".repeat(32);
        let mut plan = PlaceAudioPlan {
            version: 1,
            entry_stinger: "sting-victory".into(),
            layers: vec![AmbientLayerPlan {
                kind: AmbientKind::Water,
                key: key.clone(),
                status: "pending".into(),
                detail: String::new(),
                path: String::new(),
                gain_permille: 350,
                speech_gain_permille: 120,
            }],
        };
        let mut incoming = rpc::ViewMessage {
            revision: Some(rpc::SessionRevision {
                epoch: Some(rpc::RecoveryEpoch { value: Some(1) }),
                sequence: Some(8),
            }),
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView {
                        presentation_sequence: 42,
                        ..Default::default()
                    }),
                    scene: Some(rpc::GameplayScene {
                        location_id: "quay".into(),
                        scene_contents_json: serde_json::json!({"tempo_audio":plan}).to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
        };
        views.accept(&generation, scope, incoming.clone()).unwrap();
        plan.layers[0].status = "ready".into();
        plan.layers[0].path = format!("/gameplay/music/{key}/ambient");
        if let Some(rpc::view_message::Audience::Display(display)) = &mut incoming.audience {
            display.scene.as_mut().unwrap().scene_contents_json =
                serde_json::json!({"tempo_audio":plan}).to_string();
        }
        let (acceptance, changed) = views
            .accept_with_art(&generation, scope, incoming.clone(), merge_place_music)
            .unwrap();
        assert!(changed);
        assert!(matches!(
            acceptance,
            df_client::connection_views::ConnectionViewAcceptance::View(
                df_client::revisions::ViewAcceptance::Duplicate { .. }
            )
        ));
        assert_eq!(views.current(), Some(&incoming));
        assert!(
            !views
                .accept_with_art(&generation, scope, incoming, merge_place_music)
                .unwrap()
                .1
        );
    }

    #[test]
    fn entry_stinger_hands_off_on_a_bar_with_a_short_overlap_and_never_replays_a_line() {
        let mut entry = EntryMemory::default();
        assert!(!entry.observe(true));
        assert!(!entry.observe(true));
        assert!(entry.observe(false));
        for _ in 0..10 {
            assert!(!entry.observe(false));
        }
        assert!(
            !EntryMemory::default().observe(false),
            "reconnect is a snapshot"
        );
        let (handoff, end) = entry_handoff(4.0).unwrap();
        assert!((handoff - 4.0 * BEAT_SECONDS).abs() < 0.0001);
        assert!((end - handoff - ENTRY_CROSSFADE_SECONDS).abs() < 0.0001);
        assert!(end < 4.0, "stinger still has audio throughout the handoff");
        assert_eq!(entry_handoff(1.0), None);
        assert_eq!(entry_handoff(f64::NAN), None);
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../assets/music/source-manifest.json"))
                .unwrap();
        for kind in Stinger::ALL {
            let file = format!("{}.mp3", stinger_track(kind, 0));
            let duration = manifest["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["file"] == file)
                .unwrap()["duration_ms"]
                .as_f64()
                .unwrap()
                / 1000.0;
            assert!(
                entry_handoff(duration).unwrap().1 < duration,
                "{file} must cover the handoff"
            );
        }
    }

    #[test]
    fn live_music_ambient_lines_and_busy_scenes_change_gains_without_restarting_layers() {
        let quiet = f32::from(ambient_gain(250, 150, false)) / 1000.0;
        let busy = f32::from(ambient_gain(900, 700, false)) / 1000.0;
        let speaking = f32::from(ambient_gain(900, 700, true)) / 1000.0;
        assert!(busy > quiet && speaking < quiet);
        let playing = [("water", busy), ("gulls", quiet)];
        assert!(ambient_changes(&playing, &playing).is_empty());
        assert_eq!(
            ambient_changes(&playing, &[("water", speaking), ("gulls", quiet)]),
            vec![AmbientChange::Gain("water".into(), speaking)]
        );
        assert_eq!(
            ambient_changes(&playing, &[("water", busy), ("rain", quiet)]),
            vec![
                AmbientChange::Stop("gulls".into()),
                AmbientChange::Start("rain".into(), quiet)
            ]
        );
    }

    #[test]
    fn layered_audio_validates_routes_and_merges_without_replacing_public_scene_facts() {
        let key = "a".repeat(32);
        let plan = PlaceAudioPlan {
            version: 1,
            entry_stinger: "sting-victory".into(),
            layers: vec![AmbientLayerPlan {
                kind: AmbientKind::Water,
                key: key.clone(),
                status: "ready".into(),
                detail: String::new(),
                path: format!("/gameplay/music/{key}/ambient"),
                gain_permille: 350,
                speech_gain_permille: 120,
            }],
        };
        let scene = rpc::GameplayScene {
            location_id: "quay".into(),
            scene_contents_json: serde_json::json!({"weather":"rain","tempo_audio":plan})
                .to_string(),
            ..Default::default()
        };
        assert_eq!(place_audio_of(&scene), Some(plan.clone()));
        let mut shown = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    scene: Some(rpc::GameplayScene {
                        scene_contents_json: "{\"weather\":\"rain\"}".into(),
                        ..scene.clone()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let new = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    scene: Some(scene.clone()),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert!(merge_place_music(&mut shown, &new));
        assert!(!merge_place_music(&mut shown, &new));
        let Some(rpc::view_message::Audience::Display(display)) = shown.audience else {
            panic!("display");
        };
        let contents: serde_json::Value =
            serde_json::from_str(&display.scene.unwrap().scene_contents_json).unwrap();
        assert_eq!(contents["weather"], "rain");
        let mut bad = plan;
        bad.layers[0].path = "https://untrusted.invalid/loop".into();
        assert!(
            place_audio_of(&rpc::GameplayScene {
                scene_contents_json: serde_json::json!({"tempo_audio":bad}).to_string(),
                ..scene
            })
            .is_none()
        );
    }

    fn hero(hit_points: u32, maximum: u32) -> rpc::PartyMemberView {
        rpc::PartyMemberView {
            hit_points,
            maximum_hit_points: maximum,
            unconscious: hit_points == 0,
            ..Default::default()
        }
    }

    fn journey(phase: rpc::JourneyPhase) -> rpc::JourneyView {
        rpc::JourneyView {
            phase: phase as i32,
            party: vec![hero(20, 20), hero(20, 20)],
            ..Default::default()
        }
    }

    fn combat(round: u32, finished: bool) -> rpc::CombatView {
        rpc::CombatView {
            round,
            finished,
            ..Default::default()
        }
    }

    fn scene(asset: &str) -> rpc::GameplayScene {
        rpc::GameplayScene {
            scene_asset: asset.to_owned(),
            ..Default::default()
        }
    }

    fn tempo(scene: &rpc::GameplayScene, journey: &rpc::JourneyView) -> Tempo {
        tempo_of(scene, journey, 7)
    }

    #[test]
    fn score_loop_is_exactly_sixteen_bars_of_sample_aligned_layers() {
        assert_eq!((LOOP_SECONDS * 44_100.0).round() as u64, 2_016_000);
        assert!((LOOP_SECONDS - 45.714_285).abs() < 1e-5);
    }

    #[test]
    fn story_phases_map_to_rising_then_releasing_tempo() {
        let quiet = scene("");
        let lobby = tempo(&quiet, &journey(rpc::JourneyPhase::Room));
        let opening = tempo(&quiet, &journey(rpc::JourneyPhase::Opening));
        let dialogue = tempo(&quiet, &journey(rpc::JourneyPhase::Dialogue));
        let mut fighting = journey(rpc::JourneyPhase::Combat);
        fighting.combat = Some(combat(1, false));
        let battle = tempo(&quiet, &fighting);
        let mut won = journey(rpc::JourneyPhase::Complete);
        won.combat = Some(combat(3, true));
        let victory = tempo(&quiet, &won);
        let mut inn = scene("");
        inn.destination = rpc::HarborDestination::HarborInn as i32;
        let destination = tempo(&inn, &won);

        let phases = [lobby, opening, dialogue, battle, victory, destination].map(|t| t.phase);
        assert_eq!(
            phases,
            [
                Phase::Calm,
                Phase::Build,
                Phase::Anticipation,
                Phase::Peak,
                Phase::Release,
                Phase::Calm
            ]
        );
        assert_eq!(lobby.sequence, 7);
        assert!(lobby.intensity < opening.intensity && opening.intensity < dialogue.intensity);
        assert!(dialogue.intensity < battle.intensity && battle.intensity > victory.intensity);
        assert!(destination.intensity < victory.intensity);
    }

    #[test]
    fn combat_climbs_as_a_hero_is_hurt_and_as_rounds_pass() {
        let quiet = scene("");
        let mut fresh = journey(rpc::JourneyPhase::Combat);
        fresh.combat = Some(combat(1, false));
        let mut hurt = fresh.clone();
        hurt.party = vec![hero(20, 20), hero(4, 20)];
        let mut late = hurt.clone();
        late.combat = Some(combat(9, false));

        let first = tempo(&quiet, &fresh).intensity;
        let wounded = tempo(&quiet, &hurt).intensity;
        let long_fight = tempo(&quiet, &late).intensity;
        assert!((750..=1_000).contains(&first));
        assert!(first < wounded && wounded < long_fight && long_fight <= 1_000);
    }

    #[test]
    fn intense_dialogue_keeps_combat_layer_silent() {
        let gains = scene_layer_gains([0.7, 0.8, 0.9, 1.0], false);
        assert!((gains[0] - 0.546).abs() < 1e-6);
        assert!((gains[1] - 0.624).abs() < 1e-6);
        assert!((gains[2] - 0.495).abs() < 1e-6);
        assert_eq!(gains[3], 0.0);
        assert_eq!(scene_layer_gains([0.7, 0.8, 0.9, 1.0], true)[3], 1.0);
    }

    fn actor(name: &str, role: rpc::SceneRole, speaking: bool) -> rpc::SceneActor {
        rpc::SceneActor {
            name: name.to_owned(),
            role: role as i32,
            speaking,
            ..Default::default()
        }
    }

    #[test]
    fn an_antagonist_speaker_raises_dialogue_and_is_a_reveal() {
        let quiet = scene("");
        let mut villain = scene("");
        villain.cast = vec![
            actor("Marek", rpc::SceneRole::Patron, false),
            actor("Orsolya", rpc::SceneRole::Antagonist, true),
        ];
        let talk = journey(rpc::JourneyPhase::Dialogue);
        let friendly = tempo(&quiet, &talk);
        let hostile = tempo(&villain, &talk);
        assert_eq!(hostile.phase, Phase::Anticipation);
        assert!(hostile.intensity > friendly.intensity);
        let before = facts_of(&quiet, &talk, "");
        let after = facts_of(&villain, &talk, "");
        assert_eq!(stinger_between(&before, &after), Some(Stinger::Reveal));
        assert_eq!(stinger_between(&after, &after), None);
    }

    #[test]
    fn facts_never_inspect_asset_portrait_or_speaker_names() {
        // Names that a keyword reader would take for a villain, on a cast that says otherwise.
        let mut scene = scene("/assets/ui/generated/portrait-ant-bandit");
        scene.speaker_portrait = "/assets/ui/generated/portrait-ant-bandit".to_owned();
        scene.place = "Bandit Quay".to_owned();
        scene.location_id = "place-bandit-market".to_owned();
        scene.cast = vec![actor("Bandit", rpc::SceneRole::Bystander, true)];
        let mut talk = journey(rpc::JourneyPhase::Dialogue);
        talk.speaker = "Bandit".to_owned();
        let facts = facts_of(&scene, &talk, "scene-lobby-harbor");
        assert!(!facts.antagonist_speaking);
        assert_eq!(facts.bed, Bed::Night);
        // The same view with only the cast's role changed is the reveal.
        scene.cast = vec![actor("Anyone", rpc::SceneRole::Antagonist, true)];
        assert!(facts_of(&scene, &talk, "").antagonist_speaking);
        // An antagonist on stage who is not the one speaking is no reveal.
        scene.cast = vec![actor("Anyone", rpc::SceneRole::Antagonist, false)];
        assert!(!facts_of(&scene, &talk, "").antagonist_speaking);
    }

    fn plan(state: rpc::PlaceMusicState, family: &str) -> rpc::PlaceMusicPlan {
        rpc::PlaceMusicPlan {
            spec_label: "relaxed warm day interior".to_owned(),
            setting_family: family.to_owned(),
            state: state as i32,
            spec_key: "a".repeat(64),
            layer_paths: ["bed", "motif", "tension", "combat"]
                .map(|layer| format!("{GENERATED_MUSIC_ROUTE}{}/{layer}", "a".repeat(64)))
                .to_vec(),
            ambience_path: format!("{SHARED_MUSIC_ROUTE}amb-market"),
            fallback_note: String::new(),
            bpm: BPM as u32,
            loop_samples: LOOP_SAMPLES,
            sample_rate: SAMPLE_RATE,
        }
    }

    #[test]
    fn the_setting_family_comes_only_from_the_servers_plan_label() {
        let quiet = scene("/assets/ui/generated/scene-salt-quay-arrival");
        let room = journey(rpc::JourneyPhase::Room);
        assert_eq!(facts_of(&quiet, &room, "").bed, Bed::Night);
        let planned = plan(rpc::PlaceMusicState::Fallback, "market");
        assert_eq!(
            facts_with_plan(&quiet, &room, Some(&planned)).bed,
            Bed::Market
        );
        let unknown = plan(rpc::PlaceMusicState::Fallback, "a quay at night");
        assert_eq!(
            facts_with_plan(&quiet, &room, Some(&unknown)).bed,
            Bed::Night
        );
        for bed in Bed::ALL {
            assert_eq!(Bed::from_label(bed.label()), Some(bed));
        }
    }

    #[test]
    fn a_ready_plan_with_the_shared_timebase_names_the_places_layers() {
        let ready = plan(rpc::PlaceMusicState::Ready, "market");
        match place_music_of(Some(&ready)) {
            PlaceMusic::Ready {
                layers, ambience, ..
            } => {
                assert!(layers[0].ends_with("/bed") && layers[3].ends_with("/combat"));
                assert_eq!(ambience.as_deref(), Some("/assets/music/amb-market"));
            }
            other => panic!("expected the place's layers, got {other:?}"),
        }
        assert_eq!(place_music_of(None), PlaceMusic::SharedScore);
    }

    #[test]
    fn a_fallback_plan_keeps_the_shared_score_with_its_chosen_bed() {
        let fallback = plan(rpc::PlaceMusicState::Fallback, "market");
        assert_eq!(
            place_music_of(Some(&fallback)),
            PlaceMusic::Fallback {
                ambience: Some("/assets/music/amb-market".to_owned())
            }
        );
        let mut silent = fallback;
        silent.ambience_path.clear();
        assert_eq!(
            place_music_of(Some(&silent)),
            PlaceMusic::Fallback { ambience: None }
        );
    }

    #[test]
    fn the_places_layers_replace_the_shared_score_once_ready_but_never_a_fight() {
        let shared = wanted_layer_paths(Suite::Main, false, &PlaceMusic::SharedScore);
        assert_eq!(shared[0], "/assets/music/score-bed");
        assert_eq!(shared[3], "/assets/music/score-combat");
        let fallback = PlaceMusic::Fallback {
            ambience: Some("/assets/music/amb-market".to_owned()),
        };
        assert_eq!(wanted_layer_paths(Suite::Main, false, &fallback), shared);
        let ready = place_music_of(Some(&plan(rpc::PlaceMusicState::Ready, "market")));
        let own = wanted_layer_paths(Suite::Main, false, &ready);
        assert_ne!(own, shared);
        assert!(
            own.iter()
                .all(|path| path.starts_with(GENERATED_MUSIC_ROUTE))
        );
        assert_eq!(wanted_layer_paths(Suite::Finale, true, &ready), own);
        let battle = wanted_layer_paths(Suite::Battle, false, &ready);
        assert_eq!(battle[0], "/assets/music/b-score-bed");
        assert_eq!(
            ambience_path_of(&fallback),
            Some("/assets/music/amb-market")
        );
        assert_eq!(ambience_path_of(&PlaceMusic::SharedScore), None);
    }

    #[test]
    fn the_swap_to_the_places_layers_lands_on_a_loop_boundary_with_room_to_crossfade() {
        let started = 10.0;
        // Mid-loop: the next boundary is a whole number of loops after the start.
        let boundary = swap_boundary(started + 5.0, started, TRANSITION_SECONDS + 1.0, 0.15);
        assert!(((boundary - started) / LOOP_SECONDS).fract().abs() < 1e-9);
        assert!(boundary - (started + 5.0) >= TRANSITION_SECONDS + 1.0 + 0.15);
        // Too close to the boundary to fade across it: the following boundary is used.
        let near = started + LOOP_SECONDS - 1.0;
        let later = swap_boundary(near, started, 4.0, 0.15);
        assert!((later - (started + 2.0 * LOOP_SECONDS)).abs() < 1e-9);
    }

    #[test]
    fn layers_that_do_not_share_the_timebase_or_route_are_not_used() {
        let mut other_tempo = plan(rpc::PlaceMusicState::Ready, "market");
        other_tempo.bpm = 96;
        let mut other_loop = plan(rpc::PlaceMusicState::Ready, "market");
        other_loop.loop_samples = LOOP_SAMPLES - 1;
        let mut three_layers = plan(rpc::PlaceMusicState::Ready, "market");
        three_layers.layer_paths.pop();
        let mut escaped = plan(rpc::PlaceMusicState::Ready, "market");
        escaped.layer_paths[1] = format!("{GENERATED_MUSIC_ROUTE}../../etc/passwd");
        let mut elsewhere = plan(rpc::PlaceMusicState::Ready, "market");
        elsewhere.layer_paths[2] = "https://example.invalid/track".to_owned();
        for rejected in [other_tempo, other_loop, three_layers, escaped, elsewhere] {
            assert!(matches!(
                place_music_of(Some(&rejected)),
                PlaceMusic::Fallback { .. }
            ));
        }
    }

    #[test]
    fn live_music_pacing_variants_share_the_loop_grid_and_keep_the_battle_suite() {
        for band in PacingBand::ALL {
            #[cfg(not(target_arch = "wasm32"))]
            let bars = band.bars();
            #[cfg(target_arch = "wasm32")]
            let bars = match band {
                PacingBand::Calm => 12,
                PacingBand::Steady => 16,
                PacingBand::Driving => 20,
                PacingBand::Urgent => 24,
            };
            let seconds = f64::from(bars) * 4.0 * 60.0 / f64::from(band.bpm());
            assert!((seconds - LOOP_SECONDS).abs() < 1e-9);
            assert_eq!(
                (seconds * f64::from(SAMPLE_RATE)).round() as u32,
                LOOP_SAMPLES
            );
            let beat = next_pacing_beat(7.1, Some(0.0), band);
            assert!(beat > 7.1);
            let beat_index = beat * f64::from(band.bpm()) / 60.0;
            assert!((beat_index - beat_index.round()).abs() < 1e-8);
            let mut variant = plan(rpc::PlaceMusicState::Ready, "market");
            variant.bpm = band.bpm();
            let parsed = place_music_of(Some(&variant));
            assert!(matches!(parsed, PlaceMusic::Ready { .. }));
            assert_eq!(
                wanted_layer_paths(Suite::Main, false, &parsed),
                variant.layer_paths.as_slice()
            );
            assert_eq!(
                wanted_layer_paths(Suite::Battle, false, &parsed)[0],
                "/assets/music/b-score-bed"
            );
            for now in [0.0, 7.0, LOOP_SECONDS - 0.05, LOOP_SECONDS + 3.0] {
                let boundary = swap_boundary(now, 0.0, 4.0, 0.15);
                assert!(boundary >= now + 4.15);
                assert!((boundary / seconds - (boundary / seconds).round()).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn live_music_same_revision_merge_validates_variants_and_place_identity() {
        let view = |band: PacingBand| {
            let mut music = plan(rpc::PlaceMusicState::Ready, "market");
            music.bpm = band.bpm();
            music.spec_key = band.label().into();
            rpc::ViewMessage {
                audience: Some(rpc::view_message::Audience::Display(
                    rpc::DisplayGameplayView {
                        scene: Some(rpc::GameplayScene {
                            location_id: "shop".into(),
                            title: "Shop".into(),
                            ..Default::default()
                        }),
                        place_music: Some(music),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }
        };
        let mut shown = view(PacingBand::Steady);
        let ready = view(PacingBand::Driving);
        assert!(merge_place_music(&mut shown, &ready));
        assert_eq!(shown, ready);
        assert!(!merge_place_music(&mut shown, &ready));
        let mut wrong_place = view(PacingBand::Urgent);
        if let Some(rpc::view_message::Audience::Display(display)) = &mut wrong_place.audience {
            display.scene.as_mut().unwrap().location_id = "elsewhere".into();
        }
        assert!(!merge_place_music(&mut shown, &wrong_place));
        let mut invalid = view(PacingBand::Urgent);
        if let Some(rpc::view_message::Audience::Display(display)) = &mut invalid.audience {
            display.place_music.as_mut().unwrap().loop_samples = 1;
        }
        assert!(!merge_place_music(&mut shown, &invalid));
        assert_eq!(shown, ready);
    }

    #[test]
    fn victory_defeat_and_battle_start_each_earn_one_stinger() {
        let quiet = scene("");
        let calm = facts_of(&quiet, &journey(rpc::JourneyPhase::Room), "");
        let mut fighting = journey(rpc::JourneyPhase::Combat);
        fighting.combat = Some(combat(1, false));
        let battle = facts_of(&quiet, &fighting, "");
        let mut won = journey(rpc::JourneyPhase::Complete);
        won.combat = Some(combat(2, true));
        let victory = facts_of(&quiet, &won, "");
        let mut lost = won.clone();
        lost.party = vec![hero(0, 20), hero(0, 20)];
        let defeat = facts_of(&quiet, &lost, "");

        assert_eq!(stinger_between(&calm, &battle), Some(Stinger::Combat));
        assert_eq!(stinger_between(&battle, &victory), Some(Stinger::Victory));
        assert_eq!(stinger_between(&battle, &defeat), Some(Stinger::Defeat));
        assert_eq!(stinger_between(&victory, &victory), None);
        assert_eq!(tempo(&quiet, &lost).phase, Phase::Release);
        assert!(tempo(&quiet, &lost).intensity < tempo(&quiet, &won).intensity);
    }

    #[test]
    fn mix_adds_layers_with_tempo_and_a_release_drops_percussion() {
        let calm = mix_for(Phase::Calm, 150, false).layers;
        let build = mix_for(Phase::Build, 350, false).layers;
        let anticipation = mix_for(Phase::Anticipation, 550, false).layers;
        let peak = mix_for(Phase::Peak, 1_000, false).layers;
        let release = mix_for(Phase::Release, 450, false);
        assert!(calm.bed > 0.0 && calm.motif == 0.0 && calm.tension == 0.0 && calm.combat == 0.0);
        assert!(build.motif > 0.0 && build.tension == 0.0);
        assert!(anticipation.tension > 0.0 && anticipation.combat == 0.0);
        assert!(peak.combat > 0.0 && peak.tension > 0.0);
        assert_eq!(release.layers.tension, 0.0);
        assert_eq!(release.layers.combat, 0.0);
        assert!(release.layers.motif > 0.0);
        assert!(release.fall_seconds < 1.0);
    }

    #[test]
    fn every_ramp_except_the_release_drop_is_a_slow_one() {
        for (phase, mix) in MIX_TABLE {
            assert!((1.5..=4.0).contains(&mix.rise_seconds), "{phase:?} rise");
            let release_drop = phase == Phase::Release;
            assert!(
                release_drop || (1.5..=4.0).contains(&mix.fall_seconds),
                "{phase:?} fall"
            );
        }
    }

    #[test]
    fn intensity_scales_the_defining_layer_and_defeat_is_sombre() {
        let soft = mix_for(Phase::Peak, 750, false).layers.combat;
        let full = mix_for(Phase::Peak, 1_000, false).layers.combat;
        assert!(soft < full && (full - 1.0).abs() < 1e-6);
        let plain = mix_for(Phase::Release, 450, false);
        let sombre = mix_for(Phase::Release, 200, true);
        assert!(sombre.layers.motif < plain.layers.motif);
        assert!(sombre.filter_hertz < plain.filter_hertz);
    }

    #[test]
    fn one_shots_land_on_the_next_beat_of_the_layer_grid() {
        assert_eq!(next_beat(5.0, None), 5.0);
        assert_eq!(next_beat(5.0, Some(6.0)), 5.0);
        let start = 10.0;
        let beat = next_beat(start + 1.0, Some(start));
        assert!((beat - (start + 2.0 * BEAT_SECONDS)).abs() < 1e-9);
        assert!(beat > start + 1.0 && beat - (start + 1.0) <= BEAT_SECONDS);
        // Too close to the beat to schedule: take the following one.
        let almost = start + BEAT_SECONDS - 0.005;
        assert!((next_beat(almost, Some(start)) - (start + 2.0 * BEAT_SECONDS)).abs() < 1e-9);
    }

    #[test]
    fn a_release_settles_to_calm_only_after_its_hold() {
        let release = Tempo {
            phase: Phase::Release,
            intensity: 450,
            sequence: 1,
            cue: None,
            release_hold_millis: RELEASE_HOLD_MILLIS,
        };
        assert_eq!(
            effective_phase(release, RELEASE_HOLD_MILLIS - 1.0),
            Phase::Release
        );
        assert_eq!(effective_phase(release, RELEASE_HOLD_MILLIS), Phase::Calm);
        let peak = Tempo {
            phase: Phase::Peak,
            ..release
        };
        assert_eq!(effective_phase(peak, 1.0e9), Phase::Peak);
    }

    fn cue(phase: rpc::TempoPhase, intensity: u32, sequence: u64, hold: u32) -> rpc::TempoCue {
        rpc::TempoCue {
            intensity_permille: intensity,
            phase: phase as i32,
            sequence,
            hold_millis: hold,
        }
    }

    #[test]
    fn a_server_cue_is_the_tempo_and_the_view_is_only_the_fallback() {
        let quiet = scene("");
        let mut cued = journey(rpc::JourneyPhase::Opening);
        cued.tempo = Some(cue(rpc::TempoPhase::Peak, 880, 9, 400));
        let from_cue = tempo(&quiet, &cued);
        assert_eq!(from_cue.phase, Phase::Peak);
        assert_eq!(from_cue.intensity, 880);
        assert_eq!(from_cue.cue, Some(9));
        assert_eq!(from_cue.sequence, 7, "the view sequence stays the view's");
        assert_eq!(from_cue.release_hold_millis, 400.0);

        cued.tempo = None;
        let derived = tempo(&quiet, &cued);
        assert_eq!(derived.phase, Phase::Build);
        assert_eq!(derived.cue, None);
        assert_eq!(derived.release_hold_millis, RELEASE_HOLD_MILLIS);

        cued.tempo = Some(cue(rpc::TempoPhase::Unspecified, 900, 3, 100));
        assert_eq!(tempo(&quiet, &cued).phase, Phase::Build);
        cued.tempo = Some(cue(rpc::TempoPhase::Calm, 5_000, 4, 100));
        assert_eq!(tempo(&quiet, &cued).intensity, 1_000);
    }

    #[test]
    fn a_cue_is_consumed_once_by_its_own_sequence_and_a_reconnect_resets_it() {
        let quiet = scene("");
        let mut memory = CueMemory::default();
        let mut view = journey(rpc::JourneyPhase::Combat);
        view.tempo = Some(cue(rpc::TempoPhase::Peak, 800, 5, 250));
        assert!(memory.consume(&tempo_of(&quiet, &view, 10)));
        // The same cue on a later gameplay revision is not news.
        assert!(!memory.consume(&tempo_of(&quiet, &view, 11)));
        // An older cue arriving late is stale.
        view.tempo = Some(cue(rpc::TempoPhase::Calm, 100, 4, 250));
        assert!(!memory.consume(&tempo_of(&quiet, &view, 12)));
        view.tempo = Some(cue(rpc::TempoPhase::Release, 400, 6, 250));
        assert!(memory.consume(&tempo_of(&quiet, &view, 13)));
        // A derived tempo never touches the memory.
        view.tempo = None;
        assert!(memory.consume(&tempo_of(&quiet, &view, 14)));
        view.tempo = Some(cue(rpc::TempoPhase::Release, 400, 6, 250));
        assert!(!memory.consume(&tempo_of(&quiet, &view, 15)));
        // The server restarts its sequence on a new watch connection.
        memory.reset();
        view.tempo = Some(cue(rpc::TempoPhase::Build, 300, 1, 250));
        assert!(memory.consume(&tempo_of(&quiet, &view, 1)));
    }

    #[test]
    fn a_cue_without_a_hold_uses_the_standard_release_hold() {
        let quiet = scene("");
        let mut view = journey(rpc::JourneyPhase::Complete);
        view.tempo = Some(cue(rpc::TempoPhase::Release, 450, 2, 0));
        let release = tempo(&quiet, &view);
        assert_eq!(release.release_hold_millis, RELEASE_HOLD_MILLIS);
        assert_eq!(
            effective_phase(release, RELEASE_HOLD_MILLIS - 1.0),
            Phase::Release,
            "a release is not cut to the server's minimum cue gap"
        );
    }

    #[test]
    fn a_cue_hold_decides_when_a_release_settles() {
        let quiet = scene("");
        let mut view = journey(rpc::JourneyPhase::Complete);
        view.tempo = Some(cue(rpc::TempoPhase::Release, 450, 2, 3_000));
        let release = tempo(&quiet, &view);
        assert_eq!(effective_phase(release, 2_999.0), Phase::Release);
        assert_eq!(effective_phase(release, 3_000.0), Phase::Calm);
    }

    #[test]
    fn arrangement_epochs_are_half_a_loop_on_the_bar_grid() {
        assert!((ARRANGE_SECONDS * 2.0 - LOOP_SECONDS).abs() < 1e-9);
        let bars = ARRANGE_SECONDS / (4.0 * BEAT_SECONDS);
        assert!(
            (bars - 8.0).abs() < 1e-9,
            "an epoch is a whole number of bars"
        );
    }

    const PHASES: [Phase; 5] = [
        Phase::Calm,
        Phase::Build,
        Phase::Anticipation,
        Phase::Peak,
        Phase::Release,
    ];
    const BEDS: [Bed; 9] = [
        Bed::Harbour,
        Bed::Market,
        Bed::Forest,
        Bed::Mountain,
        Bed::Library,
        Bed::Marsh,
        Bed::Graveyard,
        Bed::Farm,
        Bed::Night,
    ];

    #[test]
    fn arrangement_is_deterministic_and_never_repeats_itself() {
        for phase in PHASES {
            for bed in BEDS {
                for seed in [1u64, 7, 0x9e37_79b9] {
                    // Arrange: a controlled clock, sweeping forty epochs.
                    let mut previous = None;
                    for epoch in 0..40 {
                        // Act: the pure function of tempo, place and session seed.
                        let index = arrangement_for(phase, bed, seed, epoch, previous);
                        // Assert: in range, deterministic, and no immediate repeat.
                        assert!(index < arrangements_for(phase).len());
                        assert_eq!(arrangement_for(phase, bed, seed, epoch, previous), index);
                        assert_ne!(previous, Some(index));
                        previous = Some(index);
                    }
                }
            }
        }
    }

    #[test]
    fn arrangement_only_thins_the_phase_table_and_never_adds_layers() {
        for phase in PHASES {
            // Scales multiply the table gains, so a zero table gain stays silent whatever the
            // state; the check is that no state ever amplifies past the table.
            for state in arrangements_for(phase) {
                for scale in state.scales {
                    assert!((0.0..=1.0).contains(&scale), "{phase:?} {}", state.name);
                }
            }
        }
    }

    #[test]
    fn quiet_scenes_may_rest_on_ambience_alone() {
        // Arrange: every roster must offer stillness where the brief allows it.
        for phase in [Phase::Calm, Phase::Release] {
            assert!(
                arrangements_for(phase)
                    .iter()
                    .any(|state| state.scales == [0.0; 4]),
                "{phase:?} offers no ambience-alone state"
            );
        }
        for phase in [Phase::Build, Phase::Anticipation, Phase::Peak] {
            assert!(
                arrangements_for(phase)
                    .iter()
                    .all(|state| state.scales != [0.0; 4]),
                "{phase:?} must not go silent mid-story"
            );
        }
        // Act: sweep seeds and epochs for a Calm stillness.
        let found = (1..200u64).any(|seed| {
            let mut previous = None;
            (0..40u64).any(|epoch| {
                let index = arrangement_for(Phase::Calm, Bed::Night, seed, epoch, previous);
                previous = Some(index);
                arrangements_for(Phase::Calm)[index].scales == [0.0; 4]
            })
        });
        // Assert: real quiet is reachable, not just listed.
        assert!(found, "no seed reached stillness in forty epochs");
    }

    #[test]
    fn colour_comes_from_place_and_fight_not_the_contract() {
        // Arrange: facts the display view already carries; no protobuf involved.
        let taproom = Facts {
            in_combat: false,
            antagonist_speaking: false,
            outcome: None,
            bed: Bed::Market,
        };
        let pass = Facts {
            bed: Bed::Mountain,
            ..taproom
        };
        let fight = Facts {
            in_combat: true,
            ..pass
        };
        // Act.
        let warm = colour_for(taproom.bed, taproom.in_combat);
        let open = colour_for(pass.bed, pass.in_combat);
        let battle = colour_for(fight.bed, fight.in_combat);
        // Assert: the taproom, the pass and a fight do not sound like the same room.
        assert_ne!(warm.name, open.name);
        assert_ne!(battle.name, open.name);
        assert_eq!(colour_for(Bed::Forest, true), battle);
        assert!(warm.filter_scale < open.filter_scale);
        assert!(open.ambient_scale > battle.ambient_scale);
    }

    #[test]
    fn finale_facts_wait_for_the_last_combat_beat_to_be_presented() {
        let victory = Facts {
            in_combat: false,
            antagonist_speaking: false,
            outcome: Some(Outcome::Victory),
            bed: Bed::Mountain,
        };

        let queued = facts_after_presentation(victory, 8, 9);
        let presented = facts_after_presentation(victory, 9, 9);

        assert_eq!(queued.outcome, None);
        assert_eq!(suite_for(queued.in_combat, queued.outcome), Suite::Main);
        assert_eq!(presented.outcome, Some(Outcome::Victory));
        assert_eq!(
            suite_for(presented.in_combat, presented.outcome),
            Suite::Finale
        );
    }

    #[test]
    fn ducking_is_deep_for_lines_and_shallow_for_effects() {
        // Arrange: the depths in force while a line plays, by suite, and the effects dip.
        let battle = duck_decibels(Suite::Battle);
        let quiet = duck_decibels(Suite::Main);
        // Assert: battles dip heavier than quiet scenes, and any line dips deeper than effects.
        assert!(battle < quiet, "a battle dips heavier than a quiet scene");
        assert_eq!(duck_decibels(Suite::Finale), quiet);
        assert!(
            quiet < DUCK_SFX_DECIBELS,
            "a line dips deeper than an effect"
        );
        assert!(gain_for_decibels(battle) < gain_for_decibels(quiet));
        assert!((gain_for_decibels(DUCK_BATTLE_DECIBELS) - 0.178).abs() < 0.002);
        assert!((gain_for_decibels(DUCK_LINE_DECIBELS) - 0.251).abs() < 0.002);
        assert!((gain_for_decibels(DUCK_SFX_DECIBELS) - 0.708).abs() < 0.002);
        assert!(
            std::hint::black_box(DUCK_LINE_ATTACK_SECONDS)
                < std::hint::black_box(DUCK_RELEASE_SECONDS)
        );
        assert!(
            std::hint::black_box(DUCK_SFX_ATTACK_SECONDS)
                < std::hint::black_box(DUCK_LINE_ATTACK_SECONDS)
        );
        assert!(
            std::hint::black_box(DUCK_SFX_HOLD_SECONDS) > 0.0
                && std::hint::black_box(DUCK_SFX_RELEASE_SECONDS) > 0.0
        );
    }

    #[test]
    fn duck_depth_puts_music_twelve_db_under_voice_in_battle_and_calm() {
        // Arrange: the full battle mix and the quiet opening, as layer gains the player
        // applies (Peak with battle colour; Build with hearth colour), over the measured
        // -23.4 LUFS per layer and narration near -16 LUFS.
        let battle = [0.63, 0.28, 0.55, 1.00];
        let opening = [0.90, 0.75, 0.0, 0.0];
        // Act: the ducked level is the mix plus the depth.
        let battle_ducked = mix_level_lufs(battle) + duck_decibels(Suite::Battle);
        let opening_ducked = mix_level_lufs(opening) + duck_decibels(Suite::Main);
        // Assert: at least 12 dB under the voice in both, with at least 5 dB of margin.
        for (name, ducked) in [("battle", battle_ducked), ("opening", opening_ducked)] {
            let under = VOICE_LUFS - ducked;
            assert!(
                under >= 17.0,
                "{name} sits only {under:.1} dB under the voice"
            );
        }
    }

    #[test]
    fn gap_hold_bridges_short_gaps_between_lines_only() {
        // Arrange/act/assert: the hold boundary at 0.8 s, exclusive.
        assert!(gap_hold_active(0.0));
        assert!(gap_hold_active(799.9));
        assert!(!gap_hold_active(800.0));
        assert!(!gap_hold_active(5_000.0));
    }

    #[test]
    fn unvoiced_hold_reuses_the_display_reading_time() {
        use crate::stage_machines::{READ_MAXIMUM_MS, READ_MINIMUM_MS, reading_time_ms};
        // Arrange: lines from empty to very long.
        let empty = "";
        let short = "Quiet now.";
        let long = &"The pass holds its breath while the torches gutter one by one. ".repeat(8);
        // Act/assert: the music hold is exactly the display's dwell, never a second rule.
        assert_eq!(unvoiced_hold_ms(empty), reading_time_ms(empty));
        assert_eq!(unvoiced_hold_ms(short), reading_time_ms(short));
        assert_eq!(unvoiced_hold_ms(long), reading_time_ms(long));
        assert_eq!(unvoiced_hold_ms(empty), READ_MINIMUM_MS);
        assert_eq!(unvoiced_hold_ms(long), READ_MAXIMUM_MS);
        assert!(unvoiced_hold_ms(long) > unvoiced_hold_ms(short));
    }

    #[test]
    fn a_caption_holds_through_reading_time_and_the_short_gap_only() {
        let text = "The gate opens.";
        let reading_ms = f64::from(unvoiced_hold_ms(text));
        let gap_ms = LINE_GAP_HOLD_SECONDS * 1_000.0;

        assert_eq!(
            unvoiced_hold_ms(text),
            crate::stage_machines::reading_time_ms(text)
        );
        assert!(reading_ms > 0.0);
        assert!(gap_hold_active(0.0));
        assert!(gap_hold_active(gap_ms - 0.1));
        assert!(!gap_hold_active(gap_ms));
    }

    #[test]
    fn action_callout_hold_is_the_bubbles_own_dwell() {
        // Arrange: the inline styles the display writes on unmanaged call-out bubbles.
        // Act/assert: the music reuses that dwell, and asks for nothing when there is none.
        assert_eq!(
            bubble_hold_ms("--appear:0ms;--hold:5500ms;--fade:500ms"),
            Some(5_500.0)
        );
        assert_eq!(
            bubble_hold_ms("--appear:200ms;--hold:6235ms;--fade:500ms"),
            Some(6_235.0)
        );
        assert_eq!(bubble_hold_ms(""), None);
        assert_eq!(bubble_hold_ms("--appear:0ms;--fade:500ms"), None);
        assert_eq!(bubble_hold_ms("--hold:0ms"), None);
        assert_eq!(bubble_hold_ms("--hold:soon"), None);
    }

    #[test]
    fn every_role_ducks_through_the_same_machine_states() {
        use crate::stage_machines::LineState;
        // Arrange: every machine state, and the names the display publishes for them.
        let states = [
            LineState::Composing,
            LineState::Submitted,
            LineState::Cancelled,
            LineState::Queued,
            LineState::Opening,
            LineState::Speaking,
            LineState::Lingering,
            LineState::Closing,
            LineState::Done,
        ];
        // Act/assert: the music reads the same states the machine calls active, so the
        // narrator's box, a character's bubble, a hero's typed bubble, a fight cry, a
        // commentary line and the loading-screen mirror all duck — narrator, NPC and hero
        // with no exemption — while typing, submitted drafts and closed lines never do.
        for state in states {
            assert_eq!(
                line_presenting(state.name()),
                matches!(state, LineState::Opening | LineState::Speaking),
                "{state:?} disagrees with the line machine"
            );
        }
        assert!(line_presenting("opening"));
        assert!(line_presenting("speaking"));
        assert!(!line_presenting("lingering"));
        assert!(!line_presenting("idle"));
        assert!(!line_presenting("composing"));
        assert!(!line_presenting("submitted"));
        assert!(!line_presenting("closing"));
        assert!(!line_presenting("done"));
    }

    #[test]
    fn takes_rotate_without_an_immediate_repeat() {
        assert_eq!(next_take(None, 0), 0);
        assert_eq!(next_take(Some(0), 1), 0);
        assert_eq!(next_take(None, 3), 0);
        // Arrange/act/assert: a full rotation from every starting take.
        let cases = [
            (None, vec![0, 1, 2, 0, 1, 2]),
            (Some(0), vec![1, 2, 0, 1, 2, 0]),
            (Some(1), vec![2, 0, 1, 2, 0, 1]),
            (Some(2), vec![0, 1, 2, 0, 1, 2]),
        ];
        for (start, expected) in cases {
            let mut last = start;
            let mut played = Vec::new();
            for _ in 0..6 {
                let take = next_take(last, STINGER_TAKES);
                assert_ne!(last, Some(take));
                played.push(take);
                last = Some(take);
            }
            assert_eq!(played, expected);
        }
    }

    #[test]
    fn suites_follow_the_chapter_kind_and_tracks_stay_servable() {
        assert_eq!(suite_for(false, None), Suite::Main);
        assert_eq!(suite_for(true, None), Suite::Battle);
        assert_eq!(suite_for(true, Some(Outcome::Victory)), Suite::Battle);
        assert_eq!(suite_for(false, Some(Outcome::Victory)), Suite::Finale);
        assert_eq!(suite_for(false, Some(Outcome::Defeat)), Suite::Finale);
        assert_eq!(Suite::Main.name(), "main");
        // Every layer suite/take combination resolves to a fixed track name.
        for suite in [Suite::Main, Suite::Battle, Suite::Finale] {
            for layer in 0..4 {
                for alternate in [false, true] {
                    let track = layer_track(suite, layer, alternate);
                    assert!(!track.is_empty() && !track.contains('/'));
                }
            }
        }
        assert_eq!(layer_track(Suite::Main, 1, true), "score-motif-b");
        assert_eq!(layer_track(Suite::Battle, 1, false), "b-score-motif");
        assert_eq!(layer_track(Suite::Finale, 1, false), "score-motif-b");
        assert_eq!(layer_track(Suite::Main, 9, false), "score-bed");
        for kind in Stinger::ALL {
            for take in 0..STINGER_TAKES {
                assert!(!stinger_track(kind, take).is_empty());
            }
        }
        assert_eq!(stinger_track(Stinger::Combat, 0), "sting-combat");
        assert_eq!(stinger_track(Stinger::Combat, 2), "sting-combat-3");
        assert_eq!(stinger_track(Stinger::Combat, 9), "sting-combat");
    }

    #[test]
    fn every_suite_layer_and_variant_uses_an_existing_music_file() {
        let music_directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/music");

        for suite in [Suite::Main, Suite::Battle, Suite::Finale] {
            for layer in 0..4 {
                for alternate in [false, true] {
                    let track = layer_track(suite, layer, alternate);
                    let path = music_directory.join(format!("{track}.mp3"));
                    assert!(
                        path.is_file(),
                        "{suite:?} layer {layer} alternate={alternate} resolves to missing {}",
                        path.display()
                    );
                }
            }
        }
    }
}
