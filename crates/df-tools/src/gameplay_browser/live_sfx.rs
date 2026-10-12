//! Sound effects on the shared display only.
//!
//! One small Web Audio engine shares `live_voice`'s state: effects exist only after the first
//! gesture unlocked audio, follow the same mute toggle, are skipped (never queued) while the
//! tab is hidden or sound is muted, and play quieter while a narration clip is speaking.
//! Effects are decoration: a missing file, a failed decode or an absent audio context simply
//! means silence, and nothing here delays input or decides any game outcome.
//!
//! A cue can be scheduled a short time ahead on the audio clock (the dice land after the
//! rattle), so a sequence needs no timers. A later view cancels cues that have not started.
use super::*;
use std::{collections::HashMap, time::Duration};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, GainNode, Response};

const SFX_PATH: &str = "/assets/ui/generated/sfx";
const MAXIMUM_EFFECT_BYTES: u32 = 2 * 1024 * 1024;
/// Effects are normalised to about -16 LUFS; narration sits at the same level, so effects
/// drop to this fraction while a clip is speaking.
const DUCKED_VOLUME: f32 = 0.3;
const LOG_ATTRIBUTE: &str = "data-sfx-log";
const LOG_LIMIT: usize = 40;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Cue {
    DiceRattle,
    DiceLand,
    Hit,
    Miss,
    Critical,
    TurnChime,
    HeroReady,
    PartyGathered,
    SceneTransition,
    VictoryFanfare,
    DefeatToll,
    BubblePop,
    HealChime,
}

impl Cue {
    const ALL: [Self; 13] = [
        Self::DiceRattle,
        Self::DiceLand,
        Self::Hit,
        Self::Miss,
        Self::Critical,
        Self::TurnChime,
        Self::HeroReady,
        Self::PartyGathered,
        Self::SceneTransition,
        Self::VictoryFanfare,
        Self::DefeatToll,
        Self::BubblePop,
        Self::HealChime,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::DiceRattle => "dice-rattle",
            Self::DiceLand => "dice-land",
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Critical => "critical",
            Self::TurnChime => "turn-chime",
            Self::HeroReady => "hero-ready",
            Self::PartyGathered => "party-gathered",
            Self::SceneTransition => "scene-transition",
            Self::VictoryFanfare => "victory-fanfare",
            Self::DefeatToll => "defeat-toll",
            Self::BubblePop => "bubble-pop",
            Self::HealChime => "heal-chime",
        }
    }

    /// Relative level of the cue against the others, applied before ducking.
    fn volume(self) -> f32 {
        match self {
            Self::BubblePop => 0.55,
            Self::SceneTransition | Self::TurnChime | Self::HealChime => 0.8,
            _ => 1.0,
        }
    }
}

enum Loaded {
    Loading,
    Ready(AudioBuffer),
    /// Absent, refused or undecodable: the cue stays silent for this page.
    Missing,
}

struct Engine {
    document: Document,
    context: AudioContext,
    master: GainNode,
    buffers: HashMap<Cue, Loaded>,
    /// Cues scheduled on the audio clock: the node and its start time.
    scheduled: Vec<(AudioBufferSourceNode, f64)>,
    log: Vec<&'static str>,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Engine {
    fn create(document: &Document) -> Result<Self, JsValue> {
        let context = AudioContext::new()?;
        let master = context.create_gain()?;
        master.connect_with_audio_node(&context.destination())?;
        Ok(Self {
            document: document.clone(),
            context,
            master,
            buffers: Cue::ALL.iter().map(|cue| (*cue, Loaded::Loading)).collect(),
            scheduled: Vec::new(),
            log: Vec::new(),
        })
    }

    fn schedule(&mut self, cue: Cue, delay: Duration, ducked: bool) -> Result<(), JsValue> {
        let Some(Loaded::Ready(buffer)) = self.buffers.get(&cue) else {
            return Ok(());
        };
        let now = self.context.current_time();
        self.scheduled.retain(|(_, start)| *start > now);
        let source = self.context.create_buffer_source()?;
        source.set_buffer(Some(buffer));
        let level = self.context.create_gain()?;
        level
            .gain()
            .set_value(cue.volume() * if ducked { DUCKED_VOLUME } else { 1.0 });
        source.connect_with_audio_node(&level)?;
        level.connect_with_audio_node(&self.master)?;
        let start = now + delay.as_secs_f64();
        source.start_with_when(start)?;
        self.scheduled.push((source, start));
        self.record(cue);
        Ok(())
    }

    /// Cues that have not started are obsolete once a newer view arrives. A cue that is
    /// already sounding is left to finish so nothing cuts off with a click.
    fn cancel_unstarted(&mut self) {
        let now = self.context.current_time();
        for (source, start) in self.scheduled.drain(..) {
            if start > now {
                // The node was started with a future time, so stopping it now cancels it.
                // A browser that refuses leaves a short decorative cue to play out.
                let scheduled: &web_sys::AudioScheduledSourceNode = source.unchecked_ref();
                let _cancelled = scheduled.stop_with_when(0.0);
            }
        }
    }

    /// Diagnostic trail on the document element: the names of the last cues played.
    fn record(&mut self, cue: Cue) {
        if self.log.len() >= LOG_LIMIT {
            self.log.remove(0);
        }
        self.log.push(cue.name());
        if let Some(element) = self.document.document_element() {
            let _marked = element.set_attribute(LOG_ATTRIBUTE, &self.log.join(" "));
        }
    }
}

/// Called by `live_voice` whenever the unlock or mute state changes: creates the engine inside
/// the unlocking gesture, starts loading the effects and applies the mute choice.
pub(super) fn sync() {
    let Some(muted) = live_voice::effects_muted() else {
        return;
    };
    if UNAVAILABLE.with(std::cell::Cell::get) {
        return;
    }
    let created = ENGINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let Some(document) = web_sys::window().and_then(|window| window.document()) else {
                UNAVAILABLE.with(|flag| flag.set(true));
                return false;
            };
            match Engine::create(&document) {
                Ok(engine) => *slot = Some(engine),
                Err(_) => {
                    UNAVAILABLE.with(|flag| flag.set(true));
                    return false;
                }
            }
            return true;
        }
        false
    });
    ENGINE.with(|slot| {
        if let Some(engine) = slot.borrow().as_ref() {
            engine
                .master
                .gain()
                .set_value(if muted { 0.0 } else { 1.0 });
            // Resuming inside the gesture lets the browser start the clock; a refusal means
            // effects stay silent until the next gesture calls `sync` again.
            let _resumed = engine.context.resume();
        }
    });
    if created {
        for cue in Cue::ALL {
            spawn_local(load(cue));
        }
    }
}

async fn load(cue: Cue) {
    let loaded = match fetch_decode(cue).await {
        Ok(buffer) => Loaded::Ready(buffer),
        Err(_) => Loaded::Missing,
    };
    ENGINE.with(|slot| {
        if let Some(engine) = slot.borrow_mut().as_mut() {
            engine.buffers.insert(cue, loaded);
        }
    });
}

async fn fetch_decode(cue: Cue) -> Result<AudioBuffer, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window absent"))?;
    let response: Response =
        JsFuture::from(window.fetch_with_str(&format!("{SFX_PATH}/{}", cue.name())))
            .await?
            .dyn_into()?;
    if response.status() != 200 {
        return Err(JsValue::from_str("effect unavailable"));
    }
    let bytes: js_sys::ArrayBuffer = JsFuture::from(response.array_buffer()?).await?.dyn_into()?;
    if bytes.byte_length() == 0 || bytes.byte_length() > MAXIMUM_EFFECT_BYTES {
        return Err(JsValue::from_str("effect size refused"));
    }
    let context = ENGINE
        .with(|slot| slot.borrow().as_ref().map(|engine| engine.context.clone()))
        .ok_or_else(|| JsValue::from_str("audio engine absent"))?;
    JsFuture::from(context.decode_audio_data(&bytes)?)
        .await?
        .dyn_into()
}

/// Plays `cue` after `delay`, unless sound is locked or muted or the tab is hidden, in which
/// case the cue is dropped. Failure to schedule is silence by design.
pub(super) fn play(cue: Cue, delay: Duration) {
    let Some(gate) = live_voice::effects_gate() else {
        return;
    };
    ENGINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(engine) = slot.as_mut() else {
            return;
        };
        if engine.document.hidden() {
            return;
        }
        let _scheduled = engine.schedule(cue, delay, gate.narrating);
    });
}

/// Drops every cue of earlier views that has not started yet.
pub(super) fn cancel_unstarted() {
    ENGINE.with(|slot| {
        if let Some(engine) = slot.borrow_mut().as_mut() {
            engine.cancel_unstarted();
        }
    });
}
