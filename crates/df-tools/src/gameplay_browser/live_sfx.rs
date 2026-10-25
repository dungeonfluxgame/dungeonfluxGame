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
use df_render::battle::{ALL_CLIPS, ContentRating, SoundEvent, VariationPicker};
use std::{collections::HashMap, time::Duration};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{AudioBuffer, AudioBufferSourceNode, AudioContext, GainNode, Response};

const SFX_PATH: &str = "/assets/ui/generated/sfx";
const MAXIMUM_EFFECT_BYTES: u32 = 2 * 1024 * 1024;
/// Effects are normalised to about -16 LUFS; narration sits at the same level, so effects
/// drop to this fraction while a clip is speaking.
const DUCKED_VOLUME: f32 = 0.3;
const LOG_ATTRIBUTE: &str = "data-sfx-log";
/// A whole tactical fight's worth of footsteps, weapon and voice events fits.
const LOG_LIMIT: usize = 1_500;
const LOG_COUNT_ATTRIBUTE: &str = "data-sfx-count";

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
    /// A combat clip from the battlefield's pack, named by file.
    Clip(&'static str),
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
            Self::Clip(name) => name,
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
    logged_count: u64,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The 3D battlefield is staging the blows, so the flat view's hit and miss sounds stay quiet.
    static BATTLEFIELD_OWNS_COMBAT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The battlefield's own victory or defeat sting has played for this fight, so the flat
    /// view's banner sting for the same ending stays quiet: one ending, one sting.
    static ENDING_STING_PLAYED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PICKER: RefCell<VariationPicker> = RefCell::new(VariationPicker::new(0x9e37_79b9));
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
            buffers: Cue::ALL
                .iter()
                .copied()
                .chain(ALL_CLIPS.iter().map(|name| Cue::Clip(name)))
                .map(|cue| (cue, Loaded::Loading))
                .collect(),
            scheduled: Vec::new(),
            log: Vec::new(),
            logged_count: 0,
        })
    }

    fn schedule(
        &mut self,
        cue: Cue,
        delay: Duration,
        ducked: bool,
        gain: f32,
        rate: f32,
    ) -> Result<(), JsValue> {
        let Some(Loaded::Ready(buffer)) = self.buffers.get(&cue) else {
            return Ok(());
        };
        let now_s = self.context.current_time();
        self.scheduled.retain(|(_, start_s)| *start_s > now_s);
        let source = self.context.create_buffer_source()?;
        source.set_buffer(Some(buffer));
        source.playback_rate().set_value(rate.clamp(0.5, 2.0));
        let level = self.context.create_gain()?;
        level
            .gain()
            .set_value(cue.volume() * gain * if ducked { DUCKED_VOLUME } else { 1.0 });
        source.connect_with_audio_node(&level)?;
        level.connect_with_audio_node(&self.master)?;
        let start_s = now_s + delay.as_secs_f64();
        source.start_with_when(start_s)?;
        self.scheduled.push((source, start_s));
        self.record(cue);
        Ok(())
    }

    /// Cues that have not started are obsolete once a newer view arrives. A cue that is
    /// already sounding is left to finish so nothing cuts off with a click.
    fn cancel_unstarted(&mut self) {
        let now_s = self.context.current_time();
        for (source, start_s) in self.scheduled.drain(..) {
            if start_s > now_s {
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
        self.logged_count += 1;
        if let Some(element) = self.document.document_element() {
            let _marked = element.set_attribute(LOG_ATTRIBUTE, &self.log.join(" "));
            let _counted =
                element.set_attribute(LOG_COUNT_ATTRIBUTE, &self.logged_count.to_string());
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
                    live_voice::failure_diagnostic("sfx", "failed");
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
            match engine.context.resume() {
                Ok(resumed) => spawn_local(async move {
                    // Page-owned diagnostic observation is bounded; dropping it never
                    // cancels or changes the browser's actual audio resume operation.
                    match futures::future::select(
                        Box::pin(wasm_bindgen_futures::JsFuture::from(resumed)),
                        Box::pin(gloo_timers::future::TimeoutFuture::new(5_000)),
                    )
                    .await
                    {
                        futures::future::Either::Left((Ok(_), _)) => {}
                        futures::future::Either::Left((Err(_), _)) => {
                            live_voice::failure_diagnostic("sfx", "blocked")
                        }
                        futures::future::Either::Right(_) => {
                            live_voice::failure_diagnostic("sfx", "failed")
                        }
                    }
                }),
                Err(_) => live_voice::failure_diagnostic("sfx", "blocked"),
            }
        }
    });
    if created {
        for cue in Cue::ALL {
            spawn_local(load(cue));
        }
        for name in ALL_CLIPS {
            spawn_local(load(Cue::Clip(name)));
        }
    }
}

async fn load(cue: Cue) {
    let loaded = match fetch_decode(cue).await {
        Ok(buffer) => Loaded::Ready(buffer),
        Err(_) => {
            live_voice::failure_diagnostic("sfx", "failed");
            Loaded::Missing
        }
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
    play_shaped(
        cue,
        delay,
        1.0,
        1.0,
        matches!(cue, Cue::VictoryFanfare | Cue::DefeatToll),
    );
}

/// As [`play`] with a level and a playback speed (above 1 raises the pitch).
fn play_shaped(cue: Cue, delay: Duration, gain: f32, rate: f32, is_ending: bool) {
    if matches!(cue, Cue::Hit | Cue::Miss) && BATTLEFIELD_OWNS_COMBAT.with(std::cell::Cell::get) {
        return;
    }
    let Some(gate) = live_voice::effects_gate() else {
        return;
    };
    ENGINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(engine) = slot.as_mut() else {
            return;
        };
        if engine.document.hidden()
            || !matches!(engine.buffers.get(&cue), Some(Loaded::Ready(_)))
            || (is_ending && !claim_ending_sting())
        {
            return;
        }
        let _scheduled = engine.schedule(cue, delay, gate.narrating, gain, rate);
    });
}

/// Plays one battlefield sound at this moment: picks among the event's clips without repeating
/// the last, at the event's level and pitch. Gated exactly like every other effect (silent
/// before the unlock, when muted or hidden, and quieter under narration).
pub(super) fn play_event(event: &SoundEvent, rating: ContentRating) {
    let ending = matches!(
        event.kind,
        df_render::battle::SoundKind::VictorySting | df_render::battle::SoundKind::DefeatSting
    );
    let Some(clip) = PICKER.with(|picker| picker.borrow_mut().pick(event.clips(rating))) else {
        return;
    };
    play_shaped(
        Cue::Clip(clip),
        Duration::ZERO,
        event.kind.gain(),
        event.rate,
        ending,
    );
}

/// Claims the one audible ending sting shared by the effects and music buses.
pub(super) fn claim_ending_sting() -> bool {
    if ENDING_STING_PLAYED.get() {
        return false;
    }
    ENDING_STING_PLAYED.set(true);
    true
}

/// A fight was entered: the next ending may sting once.
pub(super) fn reset_ending_sting() {
    ENDING_STING_PLAYED.set(false);
}

/// While the 3D battlefield stages blows, the flat view's own hit and miss sounds are left out
/// (the battlefield plays the weapon and voice sounds at the moment of each blow).
pub(super) fn set_battlefield_owns_combat(owns: bool) {
    if owns {
        ENDING_STING_PLAYED.set(false);
    }
    BATTLEFIELD_OWNS_COMBAT.with(|flag| flag.set(owns));
}

#[cfg(test)]
mod tests {
    use super::{claim_ending_sting, reset_ending_sting};

    #[test]
    fn ending_sting_is_shared_across_buses_and_resets_for_the_next_fight() {
        reset_ending_sting();
        assert!(claim_ending_sting());
        assert!(!claim_ending_sting());
        reset_ending_sting();
        assert!(claim_ending_sting());
        reset_ending_sting();
    }
}

/// Drops every cue of earlier views that has not started yet.
pub(super) fn cancel_unstarted() {
    ENGINE.with(|slot| {
        if let Some(engine) = slot.borrow_mut().as_mut() {
            engine.cancel_unstarted();
        }
    });
}
