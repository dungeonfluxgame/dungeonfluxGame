//! Sound on the page: music that plays until it is stopped, sounds played once, and an honest
//! account of what the browser allowed.
//!
//! A browser plays sound only after the person has clicked or pressed a key on the page, so a
//! sound an agent asks for may be refused. This module never hides that: a refusal is recorded and
//! reported to the host, where `df-table dev clients` shows it, so the agent knows to ask a person
//! to click the page. It decides nothing about what to play: the scene overlay does, and `scene`
//! has already checked the volumes.

use std::cell::RefCell;

use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::HtmlAudioElement;

use crate::controls::Device;
use crate::media::{AudioState, MAXIMUM_REPORTED_ERRORS};
use crate::scene::{Cue, Track};

const MEDIA_ADDRESS: &str = "/playground/media/";

#[derive(Default)]
struct Engine {
    /// Whether the person has clicked or pressed a key on the page.
    gesture_seen: bool,
    /// Whether the browser has let a sound start. It does so without a click in some settings (a
    /// page opened by automation, a site the person allowed), and a sound that played is better
    /// proof than a guess about clicks.
    played_ok: bool,
    /// The newest cue id this page has taken in, so it plays each cue once. `None` until the
    /// first frame: cues from before the page opened are not replayed.
    seen_cue: Option<u64>,
    /// The newest cue this page played or tried to play.
    last_cue: u64,
    music: Option<(Track, HtmlAudioElement)>,
    /// The browser refused the music, so it is retried when the person clicks.
    music_blocked: bool,
    effects: Vec<HtmlAudioElement>,
    errors: Vec<String>,
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::default());
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

fn volume_of(percent: u8) -> f64 {
    f64::from(percent.min(100)) / 100.0
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

fn describe(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .unwrap_or_else(|| "the browser refused".to_owned())
}

/// Starts an audio element. A refusal is recorded; for music it also marks it blocked.
fn start(audio: &HtmlAudioElement, media: String, is_music: bool) {
    let promise = match audio.play() {
        Ok(promise) => promise,
        Err(error) => {
            note_error(format!("could not play '{media}': {}", describe(&error)));
            return;
        }
    };
    spawn_local(async move {
        match JsFuture::from(promise).await {
            Ok(_) => with_engine(|engine| engine.played_ok = true),
            Err(error) => note_refusal(&error, &media, is_music),
        }
    });
}

/// Records why a sound did not start. A lock is the browser waiting for someone to click the page.
fn note_refusal(error: &JsValue, media: &str, is_music: bool) {
    if is_lock(error) {
        if is_music {
            with_engine(|engine| engine.music_blocked = true);
        }
        note_error(format!(
            "'{media}' was blocked: the browser plays sound only after someone clicks the page"
        ));
    } else {
        note_error(format!("could not play '{media}': {}", describe(error)));
    }
}

fn user_activation_seen() -> bool {
    web_sys::window()
        .map(|window| window.navigator().user_activation().has_been_active())
        .unwrap_or(false)
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
        start(&audio, media, true);
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

/// Makes the music match what the scene wants for this device: nothing, the same track as before
/// (left alone), or a new track that replaces the old one.
pub fn sync_music(desired: Option<&Track>, device: Device) {
    let wanted = desired.filter(|track| track.target.plays_on(device));
    let unchanged = with_engine(|engine| match (&engine.music, wanted) {
        (Some((current, _)), Some(wanted)) => current == wanted,
        (None, None) => true,
        _ => false,
    });
    if unchanged {
        return;
    }
    if let Some((_, audio)) = with_engine(|engine| {
        engine.music_blocked = false;
        engine.music.take()
    }) {
        let _ = audio.pause();
        audio.set_src("");
    }
    let Some(track) = wanted else {
        return;
    };
    match new_audio(&track.media, track.volume, track.looping) {
        Ok(audio) => {
            with_engine(|engine| engine.music = Some((track.clone(), audio.clone())));
            start(&audio, track.media.clone(), true);
        }
        Err(error) => note_error(format!(
            "could not play '{}': {}",
            track.media,
            describe(&error)
        )),
    }
}

/// Plays each cue this page has not taken in yet, once. The first frame only learns the count, so
/// a page that opens does not replay old sounds; a count that went down means the host restarted.
pub fn play_cues(cues: &[Cue], cue_count: u64, device: Device) {
    let seen = with_engine(|engine| engine.seen_cue);
    with_engine(|engine| engine.seen_cue = Some(cue_count));
    let fresh: Vec<&Cue> = match seen {
        Some(seen) if cue_count >= seen => cues.iter().filter(|cue| cue.id > seen).collect(),
        _ => Vec::new(),
    };
    with_engine(|engine| engine.effects.retain(|audio| !audio.ended()));
    for cue in fresh {
        if !cue.target.plays_on(device) {
            continue;
        }
        with_engine(|engine| engine.last_cue = engine.last_cue.max(cue.id));
        match new_audio(&cue.media, cue.volume, false) {
            Ok(audio) => {
                with_engine(|engine| engine.effects.push(audio.clone()));
                start(&audio, cue.media.clone(), false);
            }
            Err(error) => note_error(format!(
                "could not play '{}': {}",
                cue.media,
                describe(&error)
            )),
        }
    }
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
