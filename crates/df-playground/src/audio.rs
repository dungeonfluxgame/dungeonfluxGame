//! Sound on the page: music that plays until it is stopped, sounds played once, and an honest
//! account of what the browser allowed.
//!
//! A browser plays sound only after the person has clicked or pressed a key on the page, so a
//! sound an agent asks for may be refused. This module never hides that: a refusal is recorded and
//! reported to the host, where `df-table dev clients` shows it, so the agent knows to ask a person
//! to click the page. It decides nothing about what to play: the scene overlay does, `sound_plan`
//! says which cues are new and what a change of music means, and `scene` has already checked the
//! volumes.

use std::cell::RefCell;

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Event, HtmlAudioElement};

use crate::controls::Device;
use crate::dom::js_message;
use crate::limits::{MAXIMUM_LIVE_EFFECTS, MAXIMUM_REPORTED_ERRORS};
use crate::media::AudioState;
use crate::scene::{Cue, Track};
use crate::sound_plan::{MusicChange, fresh_cues, music_change, volume_of};

const MEDIA_ADDRESS: &str = "/playground/media/";

#[derive(Default)]
struct Engine {
    /// Whether the person has clicked or pressed a key on the page.
    gesture_seen: bool,
    /// Whether the browser has let a sound start. It does so without a click in some settings (a
    /// page opened by automation, a site the person allowed), and a sound that played is better
    /// proof than a guess about clicks.
    played_ok: bool,
    /// The newest cue count this page has taken in, so it plays each cue once. `None` until the
    /// first frame of a run of the host: cues from before the page opened are not replayed.
    seen_cue: Option<u64>,
    /// The newest cue this page played or tried to play.
    last_cue: u64,
    music: Option<(Track, HtmlAudioElement)>,
    /// The browser refused the music, so it is retried when the person clicks.
    music_blocked: bool,
    /// Sounds started and not yet over. One the browser refuses or cannot load is let go at once.
    effects: Vec<HtmlAudioElement>,
    errors: Vec<String>,
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::default());
    /// The one handler every effect's `error` event calls.
    static EFFECT_FAILED: Closure<dyn FnMut(Event)> =
        Closure::<dyn FnMut(Event)>::new(|event: Event| on_effect_error(&event));
}

fn with_engine<R>(change: impl FnOnce(&mut Engine) -> R) -> R {
    ENGINE.with(|engine| change(&mut engine.borrow_mut()))
}

/// What the page can say about its sound.
pub struct Status {
    pub state: AudioState,
    pub last_cue: u64,
    /// The music playing now, by media name.
    pub music: Option<String>,
    pub errors: Vec<String>,
}

/// Records something that went wrong, for the report. A repeat of the newest error is dropped.
pub fn note_error(message: String) {
    with_engine(|engine| {
        if engine.errors.last() != Some(&message) {
            engine.errors.push(message);
            if engine.errors.len() > MAXIMUM_REPORTED_ERRORS {
                engine.errors.remove(0);
            }
        }
    });
}

/// Forgets which cues were taken in, because the host that numbered them started again.
pub fn forget_cues() {
    with_engine(|engine| engine.seen_cue = None);
}

fn new_audio(media: &str, volume: u8, looping: bool) -> Result<HtmlAudioElement, JsValue> {
    let audio = HtmlAudioElement::new_with_src(&format!("{MEDIA_ADDRESS}{media}"))?;
    audio.set_volume(volume_of(volume));
    audio.set_loop(looping);
    Ok(audio)
}

/// Whether a refusal means the browser has not been given permission to play yet.
fn is_lock(error: &JsValue) -> bool {
    js_sys::Reflect::get(error, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .is_some_and(|name| name == "NotAllowedError")
}

/// What a started sound is, which decides what a refusal means for it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Music,
    Effect,
}

/// Lets an effect go: it will not play, so nothing should keep it.
fn release_effect(audio: &HtmlAudioElement) {
    with_engine(|engine| engine.effects.retain(|held| held != audio));
}

/// Starts an audio element. A refusal is recorded; for music it also marks it blocked, and an
/// effect that did not start is let go.
fn start(audio: &HtmlAudioElement, media: String, kind: Kind) {
    let promise = match audio.play() {
        Ok(promise) => promise,
        Err(error) => {
            note_error(format!("could not play '{media}': {}", js_message(&error)));
            if kind == Kind::Effect {
                release_effect(audio);
            }
            return;
        }
    };
    let audio = audio.clone();
    spawn_local(async move {
        match JsFuture::from(promise).await {
            Ok(_) => with_engine(|engine| engine.played_ok = true),
            Err(error) => {
                note_refusal(&error, &media, kind);
                if kind == Kind::Effect {
                    release_effect(&audio);
                }
            }
        }
    });
}

/// Records why a sound did not start. A lock is the browser waiting for someone to click the page.
fn note_refusal(error: &JsValue, media: &str, kind: Kind) {
    if is_lock(error) {
        if kind == Kind::Music {
            with_engine(|engine| engine.music_blocked = true);
        }
        note_error(format!(
            "'{media}' was blocked: the browser plays sound only after someone clicks the page"
        ));
    } else {
        note_error(format!("could not play '{media}': {}", js_message(error)));
    }
}

/// An effect's file failed to load or play: let it go.
fn on_effect_error(event: &Event) {
    let Some(audio) = event
        .target()
        .and_then(|target| target.dyn_into::<HtmlAudioElement>().ok())
    else {
        return;
    };
    note_error(format!("a sound did not load: {}", audio.src()));
    release_effect(&audio);
}

fn user_activation_seen() -> bool {
    // No window is no activation: the sound stays reported as locked.
    web_sys::window().is_some_and(|window| window.navigator().user_activation().has_been_active())
}

/// Called when the person clicks or presses a key on the page. Returns whether this changed what
/// the page can say about its sound. Music that was blocked is started again.
pub fn unlock() -> bool {
    let first = with_engine(|engine| !std::mem::replace(&mut engine.gesture_seen, true));
    let retry = with_engine(|engine| {
        if engine.music_blocked {
            engine.music_blocked = false;
            engine
                .music
                .as_ref()
                .map(|(track, audio)| (track.media.clone(), audio.clone()))
        } else {
            None
        }
    });
    if let Some((media, audio)) = retry {
        start(&audio, media, Kind::Music);
    }
    first
}

pub fn state() -> AudioState {
    if with_engine(|engine| engine.gesture_seen || engine.played_ok) || user_activation_seen() {
        AudioState::Unlocked
    } else {
        AudioState::Locked
    }
}

fn stop_music() {
    if let Some((_, audio)) = with_engine(|engine| {
        engine.music_blocked = false;
        engine.music.take()
    }) {
        // A pause that fails leaves nothing playing that the next line does not also drop.
        if let Err(error) = audio.pause() {
            note_error(format!("could not pause the music: {}", js_message(&error)));
        }
        audio.set_src("");
    }
}

/// Makes the music match what the scene wants for this device: nothing, the same song as before
/// (left alone, or with its volume and loop changed where it plays) or another song that replaces
/// the old one.
pub fn sync_music(desired: Option<&Track>, device: Device) {
    let wanted = desired.filter(|track| track.target.plays_on(device));
    let change =
        with_engine(|engine| music_change(engine.music.as_ref().map(|(track, _)| track), wanted));
    match change {
        MusicChange::Keep => {}
        MusicChange::Update { volume, looping } => with_engine(|engine| {
            if let Some((track, audio)) = &mut engine.music {
                audio.set_volume(volume_of(volume));
                audio.set_loop(looping);
                track.volume = volume;
                track.looping = looping;
            }
        }),
        MusicChange::Stop => stop_music(),
        MusicChange::Replace => {
            stop_music();
            if let Some(track) = wanted {
                start_music(track);
            }
        }
    }
}

fn start_music(track: &Track) {
    match new_audio(&track.media, track.volume, track.looping) {
        Ok(audio) => {
            with_engine(|engine| engine.music = Some((track.clone(), audio.clone())));
            start(&audio, track.media.clone(), Kind::Music);
        }
        Err(error) => note_error(format!(
            "could not play '{}': {}",
            track.media,
            js_message(&error)
        )),
    }
}

/// Plays each cue this page has not taken in yet, once. The first frame of a run of the host only
/// learns the count, so a page that opens does not replay old sounds; a count that is not newer
/// than the one taken in plays nothing.
pub fn play_cues(cues: &[Cue], cue_count: u64, device: Device) {
    let seen = with_engine(|engine| engine.seen_cue);
    let fresh = fresh_cues(seen, cue_count, cues, device);
    with_engine(|engine| {
        engine.seen_cue = Some(fresh.seen);
        engine.effects.retain(|audio| !audio.ended());
    });
    if fresh.missed > 0 {
        note_error(format!(
            "{} sounds were made while this page was not listening and are no longer kept",
            fresh.missed
        ));
    }
    for cue in fresh.play {
        with_engine(|engine| engine.last_cue = engine.last_cue.max(cue.id));
        match new_audio(&cue.media, cue.volume, false) {
            Ok(audio) => {
                hold_effect(&audio);
                start(&audio, cue.media.clone(), Kind::Effect);
            }
            Err(error) => note_error(format!(
                "could not play '{}': {}",
                cue.media,
                js_message(&error)
            )),
        }
    }
}

/// Keeps an effect until it is over, listening for it to fail to load. At most
/// `MAXIMUM_LIVE_EFFECTS` are held; the oldest is let go first.
fn hold_effect(audio: &HtmlAudioElement) {
    EFFECT_FAILED.with(|handler| {
        if let Err(error) =
            audio.add_event_listener_with_callback("error", handler.as_ref().unchecked_ref())
        {
            note_error(format!(
                "could not watch a sound for failure: {}",
                js_message(&error)
            ));
        }
    });
    with_engine(|engine| {
        engine.effects.push(audio.clone());
        if engine.effects.len() > MAXIMUM_LIVE_EFFECTS {
            engine.effects.remove(0);
        }
    });
}

pub fn status() -> Status {
    let state = state();
    with_engine(|engine| Status {
        state,
        last_cue: engine.last_cue,
        music: engine
            .music
            .as_ref()
            .filter(|(_, audio)| !audio.paused() && !engine.music_blocked)
            .map(|(track, _)| track.media.clone()),
        errors: engine.errors.clone(),
    })
}
