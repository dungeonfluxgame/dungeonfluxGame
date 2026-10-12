//! PLAY-V10: the home screen's looping video, over the still. Two muted inline `<video>` elements
//! stack in one layer. The first fades in once it can play through; near its end the second starts
//! from the beginning and the two cross-fade, so the wrap has no jump and no `loop` attribute is
//! used. Any error, a refused play, or leaving the lobby releases both elements and the still
//! stays on screen. Nothing here waits on the video, and no motion setting is consulted.
use super::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::JsCast;
use web_sys::HtmlVideoElement;

/// The next clip starts this many seconds before the current one ends.
const SWAP_BEFORE_END: f64 = 0.5;
/// The incoming clip has run this long, so the outgoing one can rest.
const SWAP_SETTLED: f64 = 0.6;

/// The loop layer of one lobby: its elements, the path it plays, and its frame callback.
pub(super) struct HomeLoop {
    layer: Element,
    playback: Rc<RefCell<Playback>>,
}

struct Playback {
    /// The layer the clips sit in; it is hidden whenever nothing is playing.
    layer: Element,
    path: Option<String>,
    clips: [HtmlVideoElement; 2],
    /// The clip on top: the one that is playing as the picture.
    front: usize,
    /// The clip that has started for a cross-fade and not yet taken over.
    swap: Option<usize>,
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
            clips: [first.clone(), second.clone()],
            front: 0,
            swap: None,
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

/// The first clip fades in once it can play through, and only once it is playing.
fn on_ready(state: &Rc<RefCell<Playback>>, index: usize) {
    let video = {
        let playback = state.borrow();
        if !playback.running || index != 0 || playback.front != 0 || playback.swap.is_some() {
            return;
        }
        playback.clips[0].clone()
    };
    if video.set_attribute("data-first", "").is_err() || video.set_attribute("data-on", "").is_err()
    {
        release(state);
        return;
    }
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

/// One frame of the loop: start the other clip near the end of the front one, then let the front
/// one rest once the other has run long enough to take over.
fn step(state: &Rc<RefCell<Playback>>) {
    let (front_index, front, incoming, swap) = {
        let playback = state.borrow();
        if !playback.running {
            return;
        }
        let front_index = playback.front;
        let incoming_index = 1 - front_index;
        (
            front_index,
            playback.clips[front_index].clone(),
            playback.clips[incoming_index].clone(),
            playback.swap,
        )
    };
    if swap.is_some() {
        // The incoming clip has been playing since the swap began; once it has run long enough
        // the outgoing one rests at its start, ready for the next wrap.
        if incoming.current_time() >= SWAP_SETTLED {
            let _rested = front.pause();
            front.set_current_time(0.0);
            let _removed = front.remove_attribute("data-on");
            let _removed = front.remove_attribute("data-first");
            let mut playback = state.borrow_mut();
            playback.front = 1 - front_index;
            playback.swap = None;
        }
    } else {
        let duration = front.duration();
        let near_end = duration.is_finite() && front.current_time() >= duration - SWAP_BEFORE_END;
        if near_end && !front.paused() {
            // The incoming clip starts from its beginning and fades in over the outgoing one.
            let _removed = incoming.remove_attribute("data-first");
            incoming.set_current_time(0.0);
            if incoming.set_attribute("data-on", "").is_err() {
                release(state);
                return;
            }
            let _removed = front.remove_attribute("data-on");
            let _removed = front.remove_attribute("data-first");
            state.borrow_mut().swap = Some(1 - front_index);
            start(state, &incoming);
        }
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
    playback.running = false;
    playback.path = None;
    playback.front = 0;
    playback.swap = None;
    let _hidden = playback.layer.set_attribute("hidden", "");
    let _state = playback.layer.set_attribute("data-state", "released");
    for video in &playback.clips {
        let _paused = video.pause();
        let _removed = video.remove_attribute("src");
        let _removed = video.remove_attribute("data-on");
        let _removed = video.remove_attribute("data-first");
        video.load();
    }
}
