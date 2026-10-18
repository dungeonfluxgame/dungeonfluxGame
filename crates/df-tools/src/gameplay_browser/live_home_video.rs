//! PLAY-V10: the home screen's looping video, over the still. Two muted inline `<video>` elements
//! stack in one layer. The first fades in once it can play through; near its end the second starts
//! from the beginning and fades in on top of it, while the outgoing clip stays fully opaque
//! underneath until the incoming one is fully opaque (HOME-01, see `swap`); only then does the
//! outgoing clip rest, so no wrap ever shows the layer underneath. Any error, a refused play, or
//! leaving the lobby releases both elements and the still stays on screen. Nothing here waits on
//! the video, and no motion setting is consulted.
use super::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsCast;
use web_sys::HtmlVideoElement;

#[path = "live_home_swap.rs"]
mod swap;
use swap::{Swap, SwapStep};

/// The loop layer of one lobby: its elements, the path it plays, and its frame callback.
pub(super) struct HomeLoop {
    layer: Element,
    playback: Rc<RefCell<Playback>>,
}

struct Playback {
    /// The layer the clips sit in; it is hidden whenever nothing is playing.
    layer: Element,
    path: Option<String>,
    /// The still the current loop was made from, for the DISPLAY records; empty for the
    /// scene-plate loops that share this player and keep their own records.
    still_key: String,
    clips: [HtmlVideoElement; 2],
    /// The A/B swap decisions: which clip is the picture, which is fading in over it.
    swap: Swap,
    running: bool,
    /// Kept for the page's life, so a frame already requested never calls a dropped closure.
    frame: Option<Closure<dyn FnMut()>>,
    /// Listeners stay attached for the page's life; a clip's behaviour follows `path`.
    listeners: Vec<Closure<dyn FnMut()>>,
}

impl HomeLoop {
    pub(super) fn new(document: &Document, layer: Element) -> Result<Self, JsValue> {
        let first = clip(document, &layer)?;
        let second = clip(document, &layer)?;
        let playback = Rc::new(RefCell::new(Playback {
            layer: layer.clone(),
            path: None,
            still_key: String::new(),
            clips: [first.clone(), second.clone()],
            swap: Swap::new(),
            running: false,
            frame: None,
            listeners: Vec::new(),
        }));
        let mut listeners = Vec::new();
        for (index, video) in [first, second].into_iter().enumerate() {
            let weak = Rc::downgrade(&playback);
            let ready = Closure::wrap(Box::new(move || {
                if let Some(state) = weak.upgrade() {
                    on_ready(&state, index);
                }
            }) as Box<dyn FnMut()>);
            video.add_event_listener_with_callback(
                "canplaythrough",
                ready.as_ref().unchecked_ref(),
            )?;
            listeners.push(ready);
            let weak = Rc::downgrade(&playback);
            let failed = Closure::wrap(Box::new(move || {
                if let Some(state) = weak.upgrade() {
                    release(&state);
                }
            }) as Box<dyn FnMut()>);
            video.add_event_listener_with_callback("error", failed.as_ref().unchecked_ref())?;
            listeners.push(failed);
        }
        let weak = Rc::downgrade(&playback);
        let frame = Closure::wrap(Box::new(move || {
            if let Some(state) = weak.upgrade() {
                step(&state);
            }
        }) as Box<dyn FnMut()>);
        {
            let mut state = playback.borrow_mut();
            state.frame = Some(frame);
            state.listeners = listeners;
        }
        let home = Self { layer, playback };
        home.hide();
        Ok(home)
    }

    /// Plays `path` as the loop, or releases it when there is none.
    pub(super) fn apply(&self, path: Option<&str>) -> Result<(), JsValue> {
        match path {
            None => {
                release(&self.playback);
                Ok(())
            }
            Some(path) => {
                if self.playback.borrow().path.as_deref() == Some(path) {
                    return Ok(());
                }
                self.load(path)
            }
        }
    }

    /// Names the still the lobby's loop was made from, so its DISPLAY records carry the
    /// loop's source still key. The scene-plate loops never call this and stay silent.
    pub(super) fn note_still(&self, still_key: &str) {
        self.playback.borrow_mut().still_key = still_key.to_owned();
    }

    /// Stops and releases both clips; the still is the picture again.
    pub(super) fn release(&self) {
        release(&self.playback);
    }

    fn load(&self, path: &str) -> Result<(), JsValue> {
        let mut state = self.playback.borrow_mut();
        release_locked(&mut state);
        state.path = Some(path.to_owned());
        for video in &state.clips {
            video.set_src(path);
            video.load();
        }
        state.running = true;
        drop(state);
        self.layer.remove_attribute("hidden")?;
        self.layer.set_attribute("data-state", "loading")?;
        request_frame(&self.playback);
        Ok(())
    }

    fn hide(&self) {
        let _hidden = self.layer.set_attribute("hidden", "");
    }
}

fn clip(document: &Document, layer: &Element) -> Result<HtmlVideoElement, JsValue> {
    let video = document
        .create_element("video")?
        .dyn_into::<HtmlVideoElement>()?;
    video.set_attribute("class", "tv-loop-clip")?;
    video.set_attribute("muted", "")?;
    video.set_attribute("playsinline", "")?;
    video.set_attribute("preload", "auto")?;
    video.set_muted(true);
    layer.append_child(&video)?;
    Ok(video)
}

/// A DISPLAY record of the home loop opening, wrapping or being released, keyed by the
/// still its loop was made from, so a flicker can be measured from the logs. The shared
/// media path hashes the still and the path; no picture bytes or text are recorded.
fn record_loop(playback: &Playback, kind: &'static str, reason: &'static str, crossfade_ms: u32) {
    if playback.still_key.is_empty() {
        return;
    }
    let Some(path) = playback.path.as_deref() else {
        return;
    };
    super::presentation_timing::media(kind, &playback.still_key, path, reason, crossfade_ms);
}

/// The first clip fades in once it can play through, and only once it is playing.
fn on_ready(state: &Rc<RefCell<Playback>>, index: usize) {
    let video = {
        let mut playback = state.borrow_mut();
        if !playback.running
            || index != 0
            || playback.swap.front() != 0
            || playback.swap.incoming().is_some()
        {
            return;
        }
        playback.swap.mark_shown(0);
        playback.clips[0].clone()
    };
    if video.set_attribute("data-first", "").is_err() || video.set_attribute("data-on", "").is_err()
    {
        release(state);
        return;
    }
    {
        let playback = state.borrow();
        record_loop(&playback, "clip-shown", "ready", 900);
    }
    let layer = state.borrow().layer.clone();
    let _state = layer.set_attribute("data-state", "playing");
    start(state, &video);
}

/// Starts a clip; a refused play releases the loop, so the still is what the lobby shows.
fn start(state: &Rc<RefCell<Playback>>, video: &HtmlVideoElement) {
    match video.play() {
        Ok(promise) => {
            let state = Rc::clone(state);
            wasm_bindgen_futures::spawn_local(async move {
                if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
                    release(&state);
                }
            });
        }
        Err(_) => release(state),
    }
}

/// One frame of the loop: near the front clip's end, start the other clip from its
/// beginning, fading in on top while the outgoing clip stays fully opaque; once the
/// incoming clip reports full opacity, rest the outgoing one (HOME-01).
fn step(state: &Rc<RefCell<Playback>>) {
    let (action, front, incoming, incoming_on, incoming_top) = {
        let mut playback = state.borrow_mut();
        if !playback.running {
            return;
        }
        let front_index = playback.swap.front();
        let incoming_index = 1 - front_index;
        let front = playback.clips[front_index].clone();
        let incoming = playback.clips[incoming_index].clone();
        let action = if playback.swap.incoming().is_some() {
            playback.swap.settle(incoming.current_time())
        } else {
            playback
                .swap
                .begin(front.current_time(), front.duration(), front.paused())
        };
        // The DOM mirrors the machine: the incoming clip is on, stacked on top, while
        // the outgoing one stays on underneath at full opacity.
        let incoming_on = playback.swap.is_on(incoming_index);
        let incoming_top = playback.swap.top() == incoming_index;
        (action, front, incoming, incoming_on, incoming_top)
    };
    match action {
        SwapStep::StartIncoming(_) => {
            // The incoming clip fades in on top; the outgoing one keeps `data-on` at
            // full opacity underneath until the incoming one is fully opaque.
            let _removed = incoming.remove_attribute("data-first");
            incoming.set_current_time(0.0);
            if !incoming_on
                || !incoming_top
                || incoming.set_attribute("data-on", "").is_err()
                || incoming.set_attribute("data-top", "").is_err()
            {
                release(state);
                return;
            }
            start(state, &incoming);
        }
        SwapStep::RestOutgoing(_) => {
            let _rested = front.pause();
            front.set_current_time(0.0);
            let _removed = front.remove_attribute("data-on");
            let _removed = front.remove_attribute("data-first");
            let _removed = incoming.remove_attribute("data-top");
            let playback = state.borrow();
            record_loop(&playback, "clip-shown", "changed", 500);
        }
        SwapStep::Idle => {}
    }
    request_frame(state);
}

fn request_frame(state: &Rc<RefCell<Playback>>) {
    let playback = state.borrow();
    if !playback.running {
        return;
    }
    let (Some(window), Some(frame)) = (web_sys::window(), playback.frame.as_ref()) else {
        return;
    };
    let _requested = window.request_animation_frame(frame.as_ref().unchecked_ref());
}

fn release(state: &Rc<RefCell<Playback>>) {
    release_locked(&mut state.borrow_mut());
}

fn release_locked(playback: &mut Playback) {
    if playback.path.is_some() {
        record_loop(playback, "clip-ended", "disposed", 0);
    }
    playback.running = false;
    playback.path = None;
    playback.swap.release();
    let _hidden = playback.layer.set_attribute("hidden", "");
    let _state = playback.layer.set_attribute("data-state", "released");
    for video in &playback.clips {
        let _paused = video.pause();
        let _removed = video.remove_attribute("src");
        let _removed = video.remove_attribute("data-on");
        let _removed = video.remove_attribute("data-first");
        let _removed = video.remove_attribute("data-top");
        video.load();
    }
}
