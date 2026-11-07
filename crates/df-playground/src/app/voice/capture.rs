//! The microphone in the browser: asking for it, reading its samples and letting it go.
//!
//! A `Capture` owns everything the browser handed over for one recording (the stream and each of
//! its tracks, the audio context and its nodes, and the handlers) and gives all of it back when it
//! is finished, cancelled or dropped, whichever comes first, so no path leaves the microphone
//! open. The samples go straight into a `CaptureBuffer`, which holds them to the longest recording
//! and turns them into PCM; this module decides nothing about what a recording is.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioProcessingEvent, Event, MediaDevices, MediaStream,
    MediaStreamAudioSourceNode, MediaStreamConstraints, MediaStreamTrack, ScriptProcessorNode,
};

use crate::voice_capture::{CaptureBuffer, MicrophoneError, Taken, classify_media_error};

/// One microphone recording, from the moment the browser gave the microphone until it is released.
pub struct Capture {
    stream: MediaStream,
    context: AudioContext,
    source: MediaStreamAudioSourceNode,
    processor: ScriptProcessorNode,
    buffer: Rc<RefCell<Option<CaptureBuffer>>>,
    /// Set by the handlers when the microphone goes away or its samples cannot be read.
    fault: Rc<Cell<Option<MicrophoneError>>>,
    reached_longest: Rc<Cell<bool>>,
    _on_audio: Closure<dyn FnMut(AudioProcessingEvent)>,
    _on_track_ended: Closure<dyn FnMut(Event)>,
    released: bool,
}

/// The name of an exception the browser threw.
fn exception_name(error: &JsValue) -> String {
    Reflect::get(error, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default()
}

/// Stops every track of `stream`: this is what turns the microphone's indicator off.
fn stop_tracks(stream: &MediaStream) {
    let tracks = stream.get_tracks();
    for index in 0..tracks.length() {
        if let Ok(track) = tracks.get(index).dyn_into::<MediaStreamTrack>() {
            track.set_onended(None);
            track.stop();
        }
    }
}

/// Closes an audio context. The promise is dropped on purpose: closing cannot be undone and has
/// nothing to report to a recording that is already over.
fn close_context(context: &AudioContext) {
    if let Ok(closing) = context.close() {
        drop(closing);
    }
}

impl Capture {
    /// Asks for the microphone and starts reading it. Call it from the click that asked for it.
    pub async fn open() -> Result<Self, MicrophoneError> {
        let devices = media_devices()?;
        // Made before the first wait, inside the click, so the browser lets it run.
        let context = AudioContext::new().map_err(|_| MicrophoneError::Unsupported)?;
        let constraints = MediaStreamConstraints::new();
        constraints.set_audio(&JsValue::TRUE);
        let asked = devices.get_user_media_with_constraints(&constraints);
        let stream = match asked {
            Ok(promise) => JsFuture::from(promise).await,
            Err(thrown) => Err(thrown),
        };
        let stream = match stream {
            Ok(stream) => stream.unchecked_into::<MediaStream>(),
            Err(thrown) => {
                close_context(&context);
                return Err(classify_media_error(&exception_name(&thrown)));
            }
        };
        match Self::start(&context, &stream).await {
            Ok(capture) => Ok(capture),
            Err(error) => {
                // Late or failed: nothing may be left open.
                stop_tracks(&stream);
                close_context(&context);
                Err(error)
            }
        }
    }

    async fn start(context: &AudioContext, stream: &MediaStream) -> Result<Self, MicrophoneError> {
        if let Ok(resuming) = context.resume() {
            JsFuture::from(resuming)
                .await
                .map_err(|_| MicrophoneError::Failed)?;
        }
        let rate_hz = context.sample_rate().round();
        if !rate_hz.is_finite() || rate_hz < 1.0 {
            return Err(MicrophoneError::Failed);
        }
        let source = context
            .create_media_stream_source(stream)
            .map_err(|_| MicrophoneError::Failed)?;
        // The browser picks the block size; channel 0 is the microphone.
        let processor = context
            .create_script_processor()
            .map_err(|_| MicrophoneError::Failed)?;
        // A positive whole sample rate, so the conversion holds.
        let shared = Rc::new(RefCell::new(Some(CaptureBuffer::new(rate_hz as u32))));
        let fault = Rc::new(Cell::new(None));
        let reached_longest = Rc::new(Cell::new(false));

        let on_audio = {
            let (shared, fault, reached) = (shared.clone(), fault.clone(), reached_longest.clone());
            Closure::<dyn FnMut(AudioProcessingEvent)>::new(move |event: AudioProcessingEvent| {
                let block = event
                    .input_buffer()
                    .and_then(|input| input.get_channel_data(0));
                let Ok(block) = block else {
                    fault.set(Some(MicrophoneError::Failed));
                    return;
                };
                if let Ok(mut held) = shared.try_borrow_mut()
                    && let Some(buffer) = held.as_mut()
                    && buffer.take(&block) == Taken::ReachedLongest
                {
                    reached.set(true);
                }
            })
        };
        processor.set_onaudioprocess(Some(on_audio.as_ref().unchecked_ref()));
        let on_track_ended = {
            let fault = fault.clone();
            Closure::<dyn FnMut(Event)>::new(move |_: Event| {
                fault.set(Some(MicrophoneError::DeviceLost));
            })
        };
        let tracks = stream.get_audio_tracks();
        for index in 0..tracks.length() {
            if let Ok(track) = tracks.get(index).dyn_into::<MediaStreamTrack>() {
                track.set_onended(Some(on_track_ended.as_ref().unchecked_ref()));
            }
        }
        let wired = source
            .connect_with_audio_node(&processor)
            .and_then(|_| processor.connect_with_audio_node(&context.destination()));
        if wired.is_err() {
            processor.set_onaudioprocess(None);
            return Err(MicrophoneError::Failed);
        }
        Ok(Self {
            // `Clone::clone`, not the method: `MediaStream::clone` is the browser's own, which
            // makes a second stream with copies of the tracks, and stopping those would leave the
            // microphone's own tracks running.
            stream: Clone::clone(stream),
            context: Clone::clone(context),
            source,
            processor,
            buffer: shared,
            fault,
            reached_longest,
            _on_audio: on_audio,
            _on_track_ended: on_track_ended,
            released: false,
        })
    }

    /// How much has been recorded, in milliseconds.
    pub fn elapsed_ms(&self) -> u64 {
        self.buffer
            .try_borrow()
            .ok()
            .and_then(|held| held.as_ref().map(CaptureBuffer::elapsed_ms))
            .unwrap_or(0)
    }

    /// How loud the newest block was, from 0 to 1000.
    pub fn level_permille(&self) -> u16 {
        self.buffer
            .try_borrow()
            .ok()
            .and_then(|held| held.as_ref().map(CaptureBuffer::level_permille))
            .unwrap_or(0)
    }

    /// Why the recording cannot go on, when the microphone was lost or its samples unreadable.
    pub fn fault(&self) -> Option<MicrophoneError> {
        self.fault.get()
    }

    /// Whether the recording reached the longest allowed.
    pub fn reached_longest(&self) -> bool {
        self.reached_longest.get()
    }

    /// Stops the recording, gives the microphone back and returns the recording as PCM.
    pub fn finish(mut self) -> Result<Vec<u8>, MicrophoneError> {
        // Taken before the release, which drops whatever is left in the buffer.
        let held = self
            .buffer
            .try_borrow_mut()
            .ok()
            .and_then(|mut held| held.take());
        self.release();
        held.ok_or(MicrophoneError::Failed)?.finish()
    }

    /// Gives the microphone back and drops what was recorded.
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.processor.set_onaudioprocess(None);
        // Disconnecting a node that is already disconnected throws; the nodes are being thrown
        // away either way.
        let _ = self.source.disconnect();
        let _ = self.processor.disconnect();
        stop_tracks(&self.stream);
        close_context(&self.context);
        if let Ok(mut held) = self.buffer.try_borrow_mut() {
            held.take();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.release();
    }
}

/// The browser's media devices, or `Unsupported` when it has none (an old browser, or a page that
/// is not on a secure address).
fn media_devices() -> Result<MediaDevices, MicrophoneError> {
    let window = web_sys::window().ok_or(MicrophoneError::Unsupported)?;
    let devices = Reflect::get(&window.navigator(), &JsValue::from_str("mediaDevices"))
        .map_err(|_| MicrophoneError::Unsupported)?;
    if devices.is_undefined() || devices.is_null() {
        return Err(MicrophoneError::Unsupported);
    }
    devices
        .dyn_into::<MediaDevices>()
        .map_err(|_| MicrophoneError::Unsupported)
}
