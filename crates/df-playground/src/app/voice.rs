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
use serde::de::IgnoredAny;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlInputElement, KeyboardEvent};

use self::capture::Capture;
use super::state::{App, log_error, try_with_app, with_app, with_app_mut};
use crate::composer::{
    ComposerEvent, ComposerInputs, ComposerState, ComposerView, MicLook, Note, SpeechAvailability,
    TranscribeFailure, step, targets_for,
};
use crate::copy::status_blocks_input;
use crate::dom::js_message;
use crate::image_layout::cast;
use crate::limits::{
    CAPTURE_REFRESH_MS, COMPOSER_NOTE_MS, MAXIMUM_QUESTION_CHARACTERS, WORK_REQUEST_TIMEOUT_MS,
};
use crate::net::{Call, CallError, NetError, call};
use crate::page_log::{now_ms, with_log};
use crate::pages::screens::TalkMic;
use crate::path;
use crate::seat::wire::{SayKind, SeatErrorKind, SeatStt};
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

/// Where a recording's words go: the composer's box, or a seated phone's Talk field. The phone has
/// one recorder, so one recording at a time, for one of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum MicTarget {
    #[default]
    Composer,
    Talk,
}

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
    /// Where the recording's words go.
    target: MicTarget,
    /// The Talk page's microphone as it was last drawn, so it is drawn again only when it changes.
    talk_drawn: TalkMic,
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
    // A seat that plays a character speaks to the room or to someone in the scene, never to
    // itself. The Act composer's line goes to whom the player picked with it, else to the room:
    // the Talk page's choice is that page's own and never carries over to an Act line.
    let seated = app.seat_play.is_some();
    let (blocked, cast_names) = match app.shown.as_deref() {
        Some(Ok(fixture)) => (
            status_blocks_input(&app.controls.status_for(&fixture.phone.status)),
            fixture
                .tv
                .cast_layout()
                .map(|layout| cast(layout).into_iter().map(|member| member.name).collect())
                .unwrap_or_default(),
        ),
        _ => (true, Vec::new()),
    };
    let voice = &app.voice;
    // A seat talks by voice by its own switch, which the host keeps; the dev bar's is the
    // playground's.
    let seat_speech;
    let speech = if seated {
        seat_speech = seat_speech_of(super::pages::seat_stt());
        &seat_speech
    } else if app.controls.stt_on {
        &voice.speech
    } else {
        &SpeechAvailability::TurnedOff
    };
    let (targets, chosen) = if seated {
        (
            super::pages::targets(),
            Some(voice.chosen.as_deref().unwrap_or(super::pages::ROOM_TARGET)),
        )
    } else {
        (
            targets_for(&cast_names, &voice.personas),
            voice.chosen.as_deref(),
        )
    };
    let view = ComposerView::of(&ComposerInputs {
        state: voice.state,
        has_text: voice.has_text(),
        blocked,
        speech,
        targets: &targets,
        chosen,
        elapsed_ms: voice.elapsed_ms,
        level_permille: voice.level_permille,
        note: voice.note.as_ref(),
    });
    (view, voice.draft.clone())
}

/// Whether a seat can talk by voice, from the host's word on its switch and the table's speech.
fn seat_speech_of(stt: SeatStt) -> SpeechAvailability {
    match (stt.available, stt.on) {
        (false, _) => SpeechAvailability::NotConfigured { missing: None },
        (true, false) => SpeechAvailability::TurnedOff,
        (true, true) => SpeechAvailability::Ready,
    }
}

/// The Talk page's microphone: the seat's switch, and whether the one recorder is free or
/// recording for Talk.
pub(super) fn talk_mic(stt: SeatStt) -> TalkMic {
    if !stt.available {
        return TalkMic::Unavailable;
    }
    if !stt.on {
        return TalkMic::Off;
    }
    let (state, target) =
        try_with_app(|app| (app.voice.state, app.voice.target)).unwrap_or_default();
    match (state, target) {
        (ComposerState::Recording, MicTarget::Talk) => TalkMic::Recording,
        (ComposerState::Idle | ComposerState::Draft, _) => TalkMic::Ready,
        _ => TalkMic::Busy,
    }
}

/// The player tapped the Talk page's microphone: it starts a recording for the Talk field, or
/// stops the one it started.
pub(super) fn talk_mic_pressed() {
    match with_voice(|voice| (voice.state, voice.target)) {
        (ComposerState::Recording, MicTarget::Talk) => stop_recording(false),
        (ComposerState::Idle | ComposerState::Draft, _) => begin_recording(MicTarget::Talk),
        _ => {}
    }
}

/// Brings the composer up to date with whom it may speak to: the scene's people changed.
pub(super) fn refresh() {
    sync();
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
    // The Talk page shares the recorder: it is drawn again when its microphone changes.
    let mic = talk_mic(super::pages::seat_stt());
    if with_voice(|voice| std::mem::replace(&mut voice.talk_drawn, mic) != mic) {
        super::pages::refresh_talk();
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

/// Shows under the box, for a while, why the table did not take a line sent another way (a
/// suggested card), in the player's words.
pub(super) fn show_refusal(message: &str) {
    let mut message = message.to_owned();
    cut(&mut message, MAXIMUM_REFUSAL_CHARACTERS);
    show_note(Note::AskRefused(message));
}

/// Shows what became of a recording where its words were going: under the composer's box, or on
/// the Talk page.
fn show_mic_note(note: Note) {
    if with_voice(|voice| voice.target) == MicTarget::Talk {
        super::pages::note_problem(Some(note.text()));
        sync();
    } else {
        show_note(note);
    }
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

/// The person chose someone else to talk to with the composer's line (the Talk page keeps its own
/// choice).
pub fn select(id: String) {
    with_voice(|voice| voice.chosen = Some(id));
    sync();
}

fn on_mic() {
    match with_voice(|voice| voice.state) {
        ComposerState::Recording => stop_recording(false),
        ComposerState::Idle | ComposerState::Draft => begin_recording(MicTarget::Composer),
        _ => {}
    }
}

/// Asks the browser for the microphone, for the words of `target`. This is called from the click,
/// so the browser may ask the person.
fn begin_recording(target: MicTarget) {
    let ready = try_with_app(|app| composer_inputs(app).0.mic == MicLook::Ready).unwrap_or(false);
    if !ready {
        return;
    }
    let generation = with_voice(|voice| {
        voice.state = step(voice.state, ComposerEvent::MicPressed, voice.has_text());
        voice.target = target;
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
    show_mic_note(Note::Capture(error));
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
                show_mic_note(Note::ClippedAtLongest);
            }
            spawn_local(transcribe(recording, generation));
        }
    }
}

/// Sends a recording to the host and puts the text it writes down in the box as a draft. A
/// seated phone sends it on its seat's route, by its own switch.
async fn transcribe(mut recording: Vec<u8>, generation: u64) {
    let seated = with_app(|app| app.seat_play.is_some());
    let address = if seated {
        path::PLAY_SEAT_TRANSCRIBE
    } else {
        path::SPEECH_TRANSCRIBE
    };
    // Writing a recording down waits on the speech service: it is given the time of work.
    let request = Call::post_bytes(address, path::SPEECH_CONTENT_TYPE, &recording)
        .within(WORK_REQUEST_TIMEOUT_MS);
    let answer = if seated {
        super::seat::seat_call::<Transcript, SpeechErrorKind>(request).await
    } else {
        call::<Transcript, SpeechErrorKind>(request).await
    };
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
    match answer {
        Ok(transcript) => accept_transcript(&transcript.text),
        Err(CallError::Refused(rejection)) => {
            fail_transcribe(TranscribeFailure::of_kind(rejection.kind));
        }
        Err(CallError::Net(_)) => fail_transcribe(TranscribeFailure::Unreachable),
        Err(CallError::Unreadable { .. } | CallError::Unwritable { .. }) => {
            fail_transcribe(TranscribeFailure::Host);
        }
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
    if with_voice(|voice| voice.target) == MicTarget::Talk {
        // The words are the Talk field's: the composer comes back to rest as it was.
        let record = with_voice(|voice| {
            voice.state = ComposerState::at_rest(voice.has_text());
            voice.transcribe_record.take()
        });
        close_record(record, EndReason::Ended);
        super::pages::take_words(text);
        sync();
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
    show_mic_note(Note::Transcribe(failure));
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
        let asked = if character.is_empty() {
            "room"
        } else {
            character.as_str()
        };
        voice.ask_record = open_record(TimelineKind::Ask, asked);
        true
    });
    if !started {
        return;
    }
    sync();
    if with_app(|app| app.seat_play.is_some()) {
        let to = Some(character).filter(|id| id != super::pages::ROOM_TARGET);
        spawn_local(say_as_seat(question, to));
    } else {
        spawn_local(ask(character, question));
    }
}

/// Says the draft as the seat's character, as written, to `to` (the room when none), in answer
/// to what the table asks it.
async fn say_as_seat(line: String, to: Option<String>) {
    match super::seat::say(&line, to, SayKind::Answer, None).await {
        Ok(()) => asked(),
        Err(error) => {
            let why = match &error {
                CallError::Refused(rejection) => {
                    rejection.kind.map_or("refused", SeatErrorKind::key)
                }
                other => failure_key(other),
            };
            ask_failed(why, Note::AskRefused(super::seat::player_words(&error)));
        }
    }
}

/// The draft was taken: the box empties and the composer is ready again.
fn asked() {
    let seated = with_app(|app| app.seat_play.is_some());
    let record = with_voice(|voice| {
        voice.draft.clear();
        voice.state = step(voice.state, ComposerEvent::Answered, false);
        // A seat's Act line is put to someone only when they are picked with it: the next one
        // starts at the room again.
        if seated {
            voice.chosen = None;
        }
        voice.ask_record.take()
    });
    close_record(record, EndReason::Ended);
    put_text("", false);
    sync();
}

/// Puts the question to the host and waits for the answer. The host shows the character thinking,
/// then speaking, so the page only learns whether the question was taken.
async fn ask(character: String, question: String) {
    let request = Call::post(
        path::PERSONA_ASK,
        &AskRequest::spoken(&character, &question),
    );
    match call::<IgnoredAny, SpeechErrorKind>(request).await {
        Ok(_) => asked(),
        Err(CallError::Refused(rejection)) => {
            let mut message = rejection.message;
            cut(&mut message, MAXIMUM_REFUSAL_CHARACTERS);
            ask_failed(
                rejection.kind.map_or("refused", SpeechErrorKind::key),
                Note::AskRefused(message),
            );
        }
        Err(error) => {
            log_error(ErrorSource::Net, &error.to_string());
            ask_failed(failure_key(&error), Note::AskUnreachable);
        }
    }
}

/// The closed word of a call that gave no answer to read: not reached, or not a reply.
fn failure_key<K>(error: &CallError<K>) -> &'static str {
    match error {
        CallError::Refused(_) => "refused",
        CallError::Net(_) => "unreachable",
        CallError::Unreadable { .. } | CallError::Unwritable { .. } => "unreadable",
    }
}

/// The question was not taken (`why`, a closed word): the composer is ready again and shows `note`.
fn ask_failed(why: &'static str, note: Note) {
    let record = with_voice(|voice| {
        voice.state = step(voice.state, ComposerEvent::AskFailed, voice.has_text());
        voice.ask_record.take()
    });
    close_record(record, EndReason::Failed);
    note_failure(ErrorSource::Net, "question", why);
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

/// Why the voice switches (text to speech, speech to text and the music, which share the speech
/// provider's key) cannot do anything now: the host says speech is not set up.
pub(super) fn speech_unavailable(app: &App) -> Option<String> {
    app.voice.speech.not_set_up()
}

/// Asks the host whether speech is set up and who has a persona. Called when the page starts, when
/// the host comes back and now and then, so a change made at the command line shows up.
pub async fn refresh_host_state() {
    let speech = match call::<SpeechStatus, SpeechErrorKind>(Call::get(path::SPEECH_STATUS)).await {
        Ok(status) if status.configured => SpeechAvailability::Ready,
        Ok(status) => SpeechAvailability::NotConfigured {
            missing: status.missing,
        },
        // A host without the route, or one that answers with something else, cannot do speech.
        Err(CallError::Net(NetError::Status { .. })) => {
            SpeechAvailability::NotConfigured { missing: None }
        }
        Err(CallError::Net(_)) => SpeechAvailability::HostDown,
        Err(_) => SpeechAvailability::NotConfigured { missing: None },
    };
    let personas = call::<Vec<PersonaSummary>, SpeechErrorKind>(Call::get(path::PERSONA_LIST))
        .await
        .ok();
    with_voice(|voice| {
        voice.speech = speech;
        if let Some(personas) = personas {
            voice.personas = personas;
        }
    });
    super::toolbar::refresh_speech_switches();
    sync();
}
