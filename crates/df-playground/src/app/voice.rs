//! Talking to a character: the microphone, the box with its draft, and the question.
//!
//! A person taps the microphone (a deliberate click, which is when the browser may ask them),
//! speaks, taps again, reads the text the host wrote down, changes it and presses Send. This
//! module runs that on the page: `composer` says which state follows which event and what each
//! control looks like, `voice_capture` holds a recording to its bounds, and `capture` is the
//! browser side that reads the microphone. The host decides what the character says and shows it
//! on the screen, so the page only sends the question and lets the scene follow.
//!
//! The recording and the transcript are request-lived: the audio lives in the capture until it is
//! sent, the text lives in the box, and neither is stored, logged or put in a report. Only the
//! timeline records (when a recording, a transcription and a question began and ended) and a
//! closed key for a failure are noted. Every path that leaves a recording state gives the
//! microphone back, and a late answer from the browser or the host after a cancel is dropped.

mod capture;

// TODO(DEV-12): a browser-target test that every path out of a recording (stop, cancel, page
// hidden, host lost, failure, a late permission after cancel) stops every track.
// TODO(DEV-12): a browser-target test of the composer redraw keeping the draft, focus and caret.

use gloo_timers::future::TimeoutFuture;
use serde::de::{DeserializeOwned, IgnoredAny};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlInputElement, KeyboardEvent};

use self::capture::Capture;
use super::state::{App, log_error, try_with_app, with_app_mut};
use crate::composer::{
    ComposerEvent, ComposerInputs, ComposerState, ComposerView, MicLook, Note, SpeechAvailability,
    TranscribeFailure, step, targets_for,
};
use crate::copy::status_blocks_input;
use crate::dev_reply::{HostReply, Rejection};
use crate::dom::js_message;
use crate::image_layout::cast;
use crate::limits::{CAPTURE_REFRESH_MS, COMPOSER_NOTE_MS, MAXIMUM_QUESTION_CHARACTERS};
use crate::net::{NetError, fetch_text, post_bytes, post_json};
use crate::page_log::{now_ms, with_log};
use crate::path;
use crate::timeline::{EndReason, ErrorSource, TimelineKind, cut};
use crate::view;
use crate::voice_capture::MicrophoneError;
use crate::voice_wire::{AskRequest, PersonaSummary, SpeechErrorKind, SpeechStatus, Transcript};

/// The id of the timeline record of a microphone recording.
const CAPTURE_RECORD_ID: &str = "microphone";
/// The id of the timeline record of a recording on its way to text.
const TRANSCRIBE_RECORD_ID: &str = "speech";
/// Longest refusal sentence from the host shown in the composer, in characters.
const MAXIMUM_REFUSAL_CHARACTERS: usize = 240;

/// The composer's memory, part of the page's state. The draft is the box's text, kept here so a
/// redraw of the phone gives the box back what was in it.
#[derive(Default)]
pub struct Voice {
    state: ComposerState,
    draft: String,
    speech: SpeechAvailability,
    personas: Vec<PersonaSummary>,
    chosen: Option<String>,
    note: Option<Note>,
    /// Counts the notes shown, so a note is taken away only by the timer that was set for it.
    note_serial: u64,
    capture: Option<Capture>,
    /// Counts the recordings and transcriptions begun or abandoned, so an answer that arrives after
    /// its recording was cancelled is recognised and dropped.
    generation: u64,
    elapsed_ms: u64,
    level_permille: u16,
    capture_record: Option<u64>,
    transcribe_record: Option<u64>,
    ask_record: Option<u64>,
}

impl Voice {
    fn has_text(&self) -> bool {
        !self.draft.trim().is_empty()
    }
}

fn with_voice<R>(change: impl FnOnce(&mut Voice) -> R) -> R {
    with_app_mut(|app| change(&mut app.voice))
}

/// What the composer shows now and the text in its box, read from the page's state and the
/// composer's memory.
pub fn composer_inputs(app: &App) -> (ComposerView, String) {
    let (blocked, cast_names) = match app.shown.as_deref() {
        Some(Ok(fixture)) => (
            status_blocks_input(&app.controls.status_for(&fixture.phone.status)),
            fixture
                .tv
                .backdrop_layout()
                .map(|layout| cast(layout).into_iter().map(|member| member.name).collect())
                .unwrap_or_default(),
        ),
        _ => (true, Vec::new()),
    };
    let voice = &app.voice;
    let targets = targets_for(&cast_names, &voice.personas);
    let view = ComposerView::of(&ComposerInputs {
        state: voice.state,
        has_text: voice.has_text(),
        blocked,
        speech: &voice.speech,
        targets: &targets,
        chosen: voice.chosen.as_deref(),
        elapsed_ms: voice.elapsed_ms,
        level_permille: voice.level_permille,
        note: voice.note.as_ref(),
    });
    (view, voice.draft.clone())
}

/// Brings the composer's elements up to date, in place, so the box keeps its focus and caret.
fn sync() {
    let Some((view, _)) = try_with_app(composer_inputs) else {
        return;
    };
    if let Err(error) = view::composer::sync(&view) {
        log_error(
            ErrorSource::Draw,
            &format!("could not update the composer: {}", js_message(&error)),
        );
    }
}

/// Puts text in the composer's box, giving it the focus when asked.
fn put_text(text: &str, focus: bool) {
    if let Err(error) = view::composer::set_text(text, focus) {
        log_error(
            ErrorSource::Draw,
            &format!("could not fill the composer: {}", js_message(&error)),
        );
    }
}

fn open_record(kind: TimelineKind, id: &str) -> Option<u64> {
    let now = now_ms();
    with_log(|log| log.open(kind, id, now, None, None))
}

fn close_record(record: Option<u64>, reason: EndReason) {
    if let Some(seq) = record {
        let now = now_ms();
        with_log(|log| log.close(seq, now, reason));
    }
}

/// Notes a failure for the report by its closed key. Nothing the person said goes in it.
fn note_failure(source: ErrorSource, what: &str, key: &str) {
    crate::page_log::note_error(source, &format!("{what}: {key}"));
}

/// Shows `note` under the box for a while.
fn show_note(note: Note) {
    let serial = with_voice(|voice| {
        voice.note_serial += 1;
        voice.note = Some(note);
        voice.note_serial
    });
    sync();
    spawn_local(async move {
        TimeoutFuture::new(u32::try_from(COMPOSER_NOTE_MS).unwrap_or(u32::MAX)).await;
        let expired = with_voice(|voice| {
            let expired = voice.note_serial == serial;
            if expired {
                voice.note = None;
            }
            expired
        });
        if expired {
            sync();
        }
    });
}

/// What the person asked for with a click on one of the composer's controls. Returns whether the
/// action was the composer's.
pub fn on_action(action: &str) -> bool {
    match action {
        "mic" => {
            on_mic();
            true
        }
        "mic-cancel" => {
            abandon(ComposerEvent::CancelPressed);
            true
        }
        "send" => {
            send();
            true
        }
        _ => false,
    }
}

/// The person chose someone else to talk to.
pub fn select(id: String) {
    with_voice(|voice| voice.chosen = Some(id));
    sync();
}

fn on_mic() {
    match with_voice(|voice| voice.state) {
        ComposerState::Recording => stop_recording(false),
        ComposerState::Idle | ComposerState::Draft => begin_recording(),
        _ => {}
    }
}

/// Asks the browser for the microphone. This is called from the click, so the browser may ask the
/// person.
fn begin_recording() {
    let ready = try_with_app(|app| composer_inputs(app).0.mic == MicLook::Ready).unwrap_or(false);
    if !ready {
        return;
    }
    let generation = with_voice(|voice| {
        voice.state = step(voice.state, ComposerEvent::MicPressed, voice.has_text());
        voice.generation += 1;
        voice.note = None;
        voice.elapsed_ms = 0;
        voice.level_permille = 0;
        voice.generation
    });
    sync();
    spawn_local(async move {
        let opened = Capture::open().await;
        let asking = with_voice(|voice| {
            voice.generation == generation && voice.state == ComposerState::RequestingMic
        });
        match opened {
            Ok(capture) if asking => start_recording(capture, generation),
            // Cancelled while the browser was asking: dropping the capture gives the microphone
            // back, so a late permission leaves nothing open.
            Ok(capture) => drop(capture),
            Err(error) if asking => fail_capture(error),
            Err(_) => {}
        }
    });
}

fn start_recording(capture: Capture, generation: u64) {
    with_voice(|voice| {
        voice.state = step(voice.state, ComposerEvent::MicGranted, voice.has_text());
        voice.capture = Some(capture);
        voice.capture_record = open_record(TimelineKind::Capture, CAPTURE_RECORD_ID);
    });
    sync();
    spawn_local(watch(generation));
}

/// What a look at a running recording found.
enum Watched {
    Over,
    Going,
    Lost(MicrophoneError),
    ReachedLongest,
}

/// Follows a recording: its time and level, a microphone that goes away, and the longest allowed.
async fn watch(generation: u64) {
    loop {
        TimeoutFuture::new(u32::try_from(CAPTURE_REFRESH_MS).unwrap_or(u32::MAX)).await;
        let watched = with_voice(|voice| {
            if voice.generation != generation || voice.state != ComposerState::Recording {
                return Watched::Over;
            }
            let Some(capture) = &voice.capture else {
                return Watched::Over;
            };
            if let Some(error) = capture.fault() {
                return Watched::Lost(error);
            }
            voice.elapsed_ms = capture.elapsed_ms();
            voice.level_permille = capture.level_permille();
            if capture.reached_longest() {
                Watched::ReachedLongest
            } else {
                Watched::Going
            }
        });
        match watched {
            Watched::Over => return,
            Watched::Going => sync(),
            Watched::Lost(error) => {
                fail_capture(error);
                return;
            }
            Watched::ReachedLongest => {
                stop_recording(true);
                return;
            }
        }
    }
}

/// The recording could not start or go on: the microphone is given back and the box is left as
/// it was.
fn fail_capture(error: MicrophoneError) {
    let record = with_voice(|voice| {
        voice.state = step(voice.state, ComposerEvent::CaptureFailed, voice.has_text());
        voice.capture = None;
        voice.capture_record.take()
    });
    close_record(record, EndReason::Failed);
    note_failure(ErrorSource::Audio, "microphone", error.key());
    show_note(Note::Capture(error));
}

/// The person stopped the recording (or it reached the longest): it is sent to be written down.
fn stop_recording(reached_longest: bool) {
    let capture = with_voice(|voice| {
        if voice.state == ComposerState::Recording {
            voice.capture.take()
        } else {
            None
        }
    });
    let Some(capture) = capture else {
        return;
    };
    match capture.finish() {
        Err(error) => fail_capture(error),
        Ok(recording) => {
            let generation = with_voice(|voice| {
                close_record(voice.capture_record.take(), EndReason::Ended);
                voice.state = step(voice.state, ComposerEvent::StopPressed, voice.has_text());
                voice.generation += 1;
                voice.transcribe_record =
                    open_record(TimelineKind::Transcribe, TRANSCRIBE_RECORD_ID);
                voice.generation
            });
            sync();
            if reached_longest {
                show_note(Note::ClippedAtLongest);
            }
            spawn_local(transcribe(recording, generation));
        }
    }
}

/// Why a host request did not give a value.
enum ReplyProblem {
    /// The host refused, with the kind of its typed error and its own sentence.
    Refused(Rejection),
    /// The host could not be reached.
    Unreachable,
    /// The host answered with something that is not a reply.
    Unreadable,
}

/// Reads the host's answer to a request. A refusal arrives with an error status, so the body of
/// one is read as a reply too.
fn read_reply<T: DeserializeOwned>(answer: Result<String, NetError>) -> Result<T, ReplyProblem> {
    let text = match answer {
        Ok(text) => text,
        Err(NetError::Status { body, .. }) => body,
        Err(_) => return Err(ReplyProblem::Unreachable),
    };
    serde_json::from_str::<HostReply<T>>(&text)
        .map_err(|_| ReplyProblem::Unreadable)?
        .into_result()
        .map_err(ReplyProblem::Refused)
}

/// Sends a recording to the host and puts the text it writes down in the box as a draft.
async fn transcribe(mut recording: Vec<u8>, generation: u64) {
    let answer = post_bytes(
        path::SPEECH_TRANSCRIBE,
        path::SPEECH_CONTENT_TYPE,
        &recording,
    )
    .await;
    // The recording is request-lived: scrub it as soon as the host has answered.
    recording.fill(0);
    drop(recording);
    let current = with_voice(|voice| {
        voice.generation == generation && voice.state == ComposerState::Transcribing
    });
    if !current {
        // Cancelled, or the page was hidden: the answer is not applied.
        return;
    }
    match read_reply::<Transcript>(answer) {
        Ok(transcript) => accept_transcript(&transcript.text),
        Err(ReplyProblem::Refused(rejection)) => {
            fail_transcribe(TranscribeFailure::of_kind(rejection.kind));
        }
        Err(ReplyProblem::Unreachable) => fail_transcribe(TranscribeFailure::Unreachable),
        Err(ReplyProblem::Unreadable) => fail_transcribe(TranscribeFailure::Host),
    }
}

/// Puts the text the host wrote down in the box, after what is already there. It is a draft: the
/// person reads it, changes it and presses Send.
fn accept_transcript(text: &str) {
    let text = text.trim();
    if text.is_empty() {
        fail_transcribe(TranscribeFailure::NothingHeard);
        return;
    }
    let draft = with_voice(|voice| {
        close_record(voice.transcribe_record.take(), EndReason::Ended);
        let mut draft = if voice.draft.trim().is_empty() {
            text.to_owned()
        } else {
            format!("{} {text}", voice.draft.trim_end())
        };
        cut(&mut draft, MAXIMUM_QUESTION_CHARACTERS);
        voice.draft = draft.clone();
        voice.state = step(voice.state, ComposerEvent::TranscriptReady, true);
        draft
    });
    put_text(&draft, true);
    sync();
}

fn fail_transcribe(failure: TranscribeFailure) {
    let record = with_voice(|voice| {
        voice.state = step(
            voice.state,
            ComposerEvent::TranscriptFailed,
            voice.has_text(),
        );
        voice.transcribe_record.take()
    });
    close_record(record, EndReason::Failed);
    note_failure(ErrorSource::Net, "transcription", failure.key());
    show_note(Note::Transcribe(failure));
}

/// Gives up what the composer was doing with the microphone or the recording: the person cancelled,
/// the page was hidden, or the host went away. The microphone is given back and a late answer is
/// dropped.
fn abandon(event: ComposerEvent) {
    let records = with_voice(|voice| {
        let before = voice.state;
        voice.state = step(before, event, voice.has_text());
        if voice.state == before {
            return None;
        }
        voice.capture = None;
        voice.generation += 1;
        Some((voice.capture_record.take(), voice.transcribe_record.take()))
    });
    if let Some((capture_record, transcribe_record)) = records {
        close_record(capture_record, EndReason::Removed);
        close_record(transcribe_record, EndReason::Removed);
        sync();
    }
}

/// Presses Send: puts the draft to the chosen character.
fn send() {
    let Some((view, draft)) = try_with_app(composer_inputs) else {
        return;
    };
    let (true, Some(character)) = (view.send_enabled, view.selected) else {
        return;
    };
    let question = draft.trim().to_owned();
    let started = with_voice(|voice| {
        voice.state = step(voice.state, ComposerEvent::SendPressed, voice.has_text());
        if voice.state != ComposerState::Sending {
            return false;
        }
        voice.note = None;
        voice.ask_record = open_record(TimelineKind::Ask, &character);
        true
    });
    if !started {
        return;
    }
    sync();
    spawn_local(ask(character, question));
}

/// Puts the question to the host and waits for the answer. The host shows the character thinking,
/// then speaking, so the page only learns whether the question was taken.
async fn ask(character: String, question: String) {
    let body = match serde_json::to_string(&AskRequest::spoken(&character, &question)) {
        Ok(body) => body,
        Err(error) => {
            log_error(
                ErrorSource::Net,
                &format!("could not write the question: {error}"),
            );
            ask_failed(&ReplyProblem::Unreadable);
            return;
        }
    };
    let answer = post_json(path::PERSONA_ASK, &body).await;
    match read_reply::<IgnoredAny>(answer) {
        Ok(_) => {
            let record = with_voice(|voice| {
                voice.draft.clear();
                voice.state = step(voice.state, ComposerEvent::Answered, false);
                voice.ask_record.take()
            });
            close_record(record, EndReason::Ended);
            put_text("", false);
            sync();
        }
        Err(problem) => ask_failed(&problem),
    }
}

fn ask_failed(problem: &ReplyProblem) {
    let record = with_voice(|voice| {
        voice.state = step(voice.state, ComposerEvent::AskFailed, voice.has_text());
        voice.ask_record.take()
    });
    close_record(record, EndReason::Failed);
    let note = match problem {
        ReplyProblem::Refused(rejection) => {
            note_failure(
                ErrorSource::Net,
                "question",
                rejection.kind.map_or("refused", SpeechErrorKind::key),
            );
            let mut message = rejection.message.clone();
            cut(&mut message, MAXIMUM_REFUSAL_CHARACTERS);
            Note::AskRefused(message)
        }
        ReplyProblem::Unreachable => {
            note_failure(ErrorSource::Net, "question", "unreachable");
            Note::AskUnreachable
        }
        ReplyProblem::Unreadable => {
            note_failure(ErrorSource::Net, "question", "unreadable");
            Note::AskUnreachable
        }
    };
    show_note(note);
}

/// Whether `event` happened in the composer's box.
fn field_of(event: &Event) -> Option<HtmlInputElement> {
    let field = event.target()?.dyn_into::<HtmlInputElement>().ok()?;
    (field.get_attribute("data-composer").as_deref() == Some("field")).then_some(field)
}

/// The person typed in the box (or emptied it): the draft follows, and typing ends a recording
/// nobody accepted yet.
pub fn on_input(event: &Event) {
    let Some(field) = field_of(event) else {
        return;
    };
    let text = field.value();
    let record = with_voice(|voice| {
        voice.draft = text;
        let before = voice.state;
        voice.state = step(before, ComposerEvent::Edited, voice.has_text());
        if before.holds_microphone() && !voice.state.holds_microphone() {
            voice.capture = None;
            voice.generation += 1;
            voice.capture_record.take()
        } else {
            None
        }
    });
    close_record(record, EndReason::Removed);
    sync();
}

/// Enter in the box sends the draft.
pub fn on_enter(event: &Event) {
    if field_of(event).is_none() {
        return;
    }
    let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    if key.key() == "Enter" && !key.shift_key() && !key.is_composing() {
        key.prevent_default();
        send();
    }
}

/// The page was hidden or shown: an unaccepted recording ends when it is hidden.
pub fn on_visibility_change() {
    if web_sys::window()
        .and_then(|window| window.document())
        .is_some_and(|document| document.hidden())
    {
        abandon(ComposerEvent::PageHidden);
    }
}

/// The host cannot be reached: a recording or a transcription ends, and the microphone is off
/// until the host is back.
pub fn on_host_lost() {
    with_voice(|voice| voice.speech = SpeechAvailability::HostDown);
    abandon(ComposerEvent::HostLost);
    sync();
}

/// Asks the host whether speech is set up and who has a persona. Called when the page starts, when
/// the host comes back and now and then, so a change made at the command line shows up.
pub async fn refresh_host_state() {
    let speech = match read_reply::<SpeechStatus>(fetch_text(path::SPEECH_STATUS).await) {
        Ok(status) if status.configured => SpeechAvailability::Ready,
        Ok(status) => SpeechAvailability::NotConfigured {
            missing: status.missing,
        },
        Err(ReplyProblem::Unreachable) => SpeechAvailability::HostDown,
        // A host without the route, or one that answers with something else, cannot do speech.
        Err(_) => SpeechAvailability::NotConfigured { missing: None },
    };
    let personas = read_reply::<Vec<PersonaSummary>>(fetch_text(path::PERSONA_LIST).await).ok();
    with_voice(|voice| {
        voice.speech = speech;
        if let Some(personas) = personas {
            voice.personas = personas;
        }
    });
    sync();
}
