//! The phone's composer as a state machine and as what it shows: whether the microphone can be
//! used, what each control looks like, and what the composer says when something goes wrong.
//!
//! A person types, or taps the microphone, speaks, reads the text that comes back, changes it and
//! sends it. The states are closed, and `step` is the only table that moves between them: an event
//! that makes no sense in a state is ignored, never an error. The transcript is a draft in the
//! box and never an action; only the person pressing Send asks the character anything. The page
//! owns the browser side (`app::voice`); `ComposerView::of` turns the state into the words and
//! flags the elements show, so native tests cover every row.

use crate::limits::{MAXIMUM_CAPTURE_MS, MAXIMUM_QUESTION_CHARACTERS};
use crate::voice_capture::MicrophoneError;
use crate::voice_wire::{MissingSetting, PersonaSummary, SpeechErrorKind};

/// The character the composer talks to when it is on the scene and has a persona.
pub const DEFAULT_TARGET: &str = "innkeeper";

/// Where the composer is. `Draft` is a box with text in it and nothing running; `Idle` is an empty
/// one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ComposerState {
    #[default]
    Idle,
    /// The microphone was asked for and the person has not answered.
    RequestingMic,
    Recording,
    /// A recording is on its way to text.
    Transcribing,
    Draft,
    /// The question is on its way to the character.
    Sending,
}

/// What happened to the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEvent {
    MicPressed,
    /// The person allowed the microphone and the recording began.
    MicGranted,
    /// The microphone could not be had or went away, or the recording was too short to send.
    CaptureFailed,
    /// The person stopped the recording and it is on its way to text.
    StopPressed,
    CancelPressed,
    PageHidden,
    HostLost,
    /// The person typed in the box (or emptied it).
    Edited,
    TranscriptReady,
    TranscriptFailed,
    SendPressed,
    Answered,
    AskFailed,
}

impl ComposerState {
    /// The closed key the page marks the composer with.
    pub fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::RequestingMic => "requesting_mic",
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
            Self::Draft => "draft",
            Self::Sending => "sending",
        }
    }

    /// The state of a box at rest: a draft when it has text.
    pub fn at_rest(has_text: bool) -> Self {
        if has_text { Self::Draft } else { Self::Idle }
    }

    /// Whether a recording is being asked for or made, so the microphone must be released when the
    /// composer leaves this state.
    pub fn holds_microphone(self) -> bool {
        matches!(self, Self::RequestingMic | Self::Recording)
    }
}

/// The state after `event`. `has_text` says whether the box has text, which decides where a
/// composer that stops what it was doing comes to rest. An event with no meaning in a state leaves
/// it as it is.
pub fn step(state: ComposerState, event: ComposerEvent, has_text: bool) -> ComposerState {
    use ComposerEvent as Event;
    use ComposerState as State;
    let rest = State::at_rest(has_text);
    match (state, event) {
        (State::Idle | State::Draft, Event::MicPressed) => State::RequestingMic,
        (State::RequestingMic, Event::MicGranted) => State::Recording,
        (State::RequestingMic | State::Recording, Event::CaptureFailed) => rest,
        (State::Recording, Event::StopPressed) => State::Transcribing,
        (
            State::RequestingMic | State::Recording | State::Transcribing,
            Event::CancelPressed | Event::PageHidden | Event::HostLost,
        ) => rest,
        // Typing ends a recording nobody accepted yet, and edits the box otherwise.
        (State::RequestingMic | State::Recording | State::Idle | State::Draft, Event::Edited) => {
            rest
        }
        (State::Transcribing, Event::TranscriptReady) => State::Draft,
        (State::Transcribing, Event::TranscriptFailed) => rest,
        (State::Draft, Event::SendPressed) if has_text => State::Sending,
        (State::Sending, Event::Answered) => State::Idle,
        (State::Sending, Event::AskFailed) => rest,
        (unchanged, _) => unchanged,
    }
}

/// Whether speech to text can be used, from what the host said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SpeechAvailability {
    /// The host has not been asked yet.
    #[default]
    Checking,
    Ready,
    /// The host says it is not set up; `missing` is what it says to set.
    NotConfigured {
        missing: Option<MissingSetting>,
    },
    /// The host could not be asked.
    HostDown,
    /// The speech-to-text switch is off (`Controls::stt_on`).
    TurnedOff,
}

/// A character the composer can talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The persona's id, which is also the character's name in the scene's layout.
    pub id: String,
    pub name: String,
}

/// The characters of the shown scene that have a persona, in the order the scene lists them.
/// `cast` is the names of the scene's characters.
pub fn targets_for(cast: &[String], personas: &[PersonaSummary]) -> Vec<Target> {
    let mut targets: Vec<Target> = Vec::new();
    for name in cast {
        if targets.iter().any(|target| &target.id == name) {
            continue;
        }
        if let Some(persona) = personas.iter().find(|persona| &persona.id == name) {
            targets.push(Target {
                id: persona.id.clone(),
                name: persona.display_name.clone(),
            });
        }
    }
    targets
}

/// The character to talk to: the one chosen if the scene still has them, else the innkeeper, else
/// the first.
pub fn resolve_target<'a>(targets: &'a [Target], chosen: Option<&str>) -> Option<&'a Target> {
    chosen
        .and_then(|id| targets.iter().find(|target| target.id == id))
        .or_else(|| targets.iter().find(|target| target.id == DEFAULT_TARGET))
        .or_else(|| targets.first())
}

/// Why the host would not turn a recording into text, as far as the composer says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscribeFailure {
    NotConfigured,
    TooShort,
    /// The speech service heard nothing it could write down.
    NothingHeard,
    /// The host or the speech service failed.
    Host,
    /// The host could not be reached.
    Unreachable,
}

impl TranscribeFailure {
    /// The failure for the kind of a host refusal (none when it is not a speech refusal).
    pub fn of_kind(kind: Option<SpeechErrorKind>) -> Self {
        match kind {
            Some(SpeechErrorKind::NotConfigured) => Self::NotConfigured,
            Some(SpeechErrorKind::AudioTooShort | SpeechErrorKind::AudioEmpty) => Self::TooShort,
            _ => Self::Host,
        }
    }

    /// The closed key the page notes in its report.
    pub fn key(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::TooShort => "too_short",
            Self::NothingHeard => "nothing_heard",
            Self::Host => "host",
            Self::Unreachable => "unreachable",
        }
    }
}

/// What the composer says under its box, briefly, when something did not go as hoped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    Capture(MicrophoneError),
    /// The recording reached the longest allowed and was sent as it stood.
    ClippedAtLongest,
    Transcribe(TranscribeFailure),
    /// The host refused the question, in its own words.
    AskRefused(String),
    /// The question did not reach the host.
    AskUnreachable,
}

impl Note {
    /// The sentence the composer shows.
    pub fn text(&self) -> String {
        match self {
            Self::Capture(MicrophoneError::PermissionDenied) => {
                "The microphone is blocked. Allow it for this page, or type instead.".to_owned()
            }
            Self::Capture(MicrophoneError::NoDevice) => {
                "No microphone found. Type instead.".to_owned()
            }
            Self::Capture(MicrophoneError::Unsupported) => {
                "This browser cannot record here. Type instead.".to_owned()
            }
            Self::Capture(MicrophoneError::DeviceLost) => {
                "The microphone stopped working. Try again, or type.".to_owned()
            }
            Self::Capture(MicrophoneError::TooShort) => {
                "That was too short. Hold the microphone a little longer.".to_owned()
            }
            Self::Capture(MicrophoneError::Failed) => {
                "The recording failed. Try again, or type.".to_owned()
            }
            Self::ClippedAtLongest => format!(
                "Recording stopped at {} seconds.",
                MAXIMUM_CAPTURE_MS / 1_000
            ),
            Self::Transcribe(TranscribeFailure::NotConfigured) => {
                "Voice is not set up on the host. Type instead.".to_owned()
            }
            Self::Transcribe(TranscribeFailure::TooShort) => {
                "That was too short to make out. Try again.".to_owned()
            }
            Self::Transcribe(TranscribeFailure::NothingHeard) => {
                "Couldn't make that out. Type it instead.".to_owned()
            }
            Self::Transcribe(TranscribeFailure::Host) => {
                "The host could not write that down. Type it instead.".to_owned()
            }
            Self::Transcribe(TranscribeFailure::Unreachable) => {
                "The host could not be reached. Type it instead.".to_owned()
            }
            Self::AskRefused(message) => message.clone(),
            Self::AskUnreachable => "The host could not be reached. Try sending again.".to_owned(),
        }
    }
}

/// How the microphone button looks and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicLook {
    /// Pressing it does nothing; `reason` says why.
    Unavailable {
        reason: String,
    },
    Ready,
    /// The browser is asking the person.
    Waiting,
    Recording,
    /// A recording or a question is on its way.
    Busy,
}

impl MicLook {
    /// What the button says.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Waiting => "Waiting...",
            Self::Recording => "Stop",
            Self::Unavailable { .. } | Self::Ready | Self::Busy => "Talk",
        }
    }

    /// What the button's tooltip says.
    pub fn title(&self) -> String {
        match self {
            Self::Unavailable { reason } => reason.clone(),
            Self::Ready => "Record a question".to_owned(),
            Self::Waiting => "Waiting for the browser to ask about the microphone".to_owned(),
            Self::Recording => "Stop recording and write it down".to_owned(),
            Self::Busy => "Working on the last one".to_owned(),
        }
    }

    /// Whether a click on the button does anything.
    pub fn enabled(&self) -> bool {
        matches!(self, Self::Ready | Self::Recording)
    }
}

/// Everything the composer's elements show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerView {
    pub state: ComposerState,
    pub targets: Vec<Target>,
    /// The character the question goes to.
    pub selected: Option<String>,
    pub mic: MicLook,
    /// What the strip above the box says while the microphone is in use: that the browser is
    /// asking, the running time of the recording as `m:ss`, or that the words are being written
    /// down.
    pub live: Option<String>,
    /// How loud the newest sound was, from 0 to 1000, while a recording is going.
    pub level_permille: u16,
    pub field_enabled: bool,
    pub field_placeholder: &'static str,
    pub send_enabled: bool,
    pub send_label: &'static str,
    /// Longest question, in characters.
    pub field_limit: usize,
    pub note: Option<String>,
}

/// What `ComposerView::of` reads.
pub struct ComposerInputs<'a> {
    pub state: ComposerState,
    pub has_text: bool,
    /// The phone's status says input is not possible now.
    pub blocked: bool,
    pub speech: &'a SpeechAvailability,
    pub targets: &'a [Target],
    pub chosen: Option<&'a str>,
    pub elapsed_ms: u64,
    pub level_permille: u16,
    pub note: Option<&'a Note>,
}

/// Seconds as `m:ss`.
pub fn format_elapsed(elapsed_ms: u64) -> String {
    let seconds = elapsed_ms / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn unavailable_reason(inputs: &ComposerInputs<'_>, has_target: bool) -> Option<String> {
    if inputs.blocked {
        return Some("Not available right now".to_owned());
    }
    match inputs.speech {
        SpeechAvailability::Checking => Some("Checking the microphone...".to_owned()),
        SpeechAvailability::HostDown => Some("The host could not be reached".to_owned()),
        SpeechAvailability::TurnedOff => Some("Speech to text is turned off".to_owned()),
        SpeechAvailability::NotConfigured { missing } => Some(match missing {
            Some(missing) => format!(
                "Voice is not set up on the host (missing {})",
                missing.variable()
            ),
            None => "Voice is not set up on the host".to_owned(),
        }),
        SpeechAvailability::Ready if !has_target => {
            Some("No one here has a persona to talk to".to_owned())
        }
        SpeechAvailability::Ready => None,
    }
}

impl ComposerView {
    pub fn of(inputs: &ComposerInputs<'_>) -> Self {
        let target = resolve_target(inputs.targets, inputs.chosen);
        let state = inputs.state;
        let recording = state == ComposerState::Recording;
        let mic = match state {
            ComposerState::RequestingMic => MicLook::Waiting,
            ComposerState::Recording => MicLook::Recording,
            ComposerState::Transcribing | ComposerState::Sending => MicLook::Busy,
            ComposerState::Idle | ComposerState::Draft => {
                match unavailable_reason(inputs, target.is_some()) {
                    Some(reason) => MicLook::Unavailable { reason },
                    None => MicLook::Ready,
                }
            }
        };
        let field_placeholder = match state {
            ComposerState::RequestingMic => "Allow the microphone...",
            ComposerState::Recording => "Listening...",
            ComposerState::Transcribing => "Writing that down...",
            ComposerState::Sending => "Asking...",
            ComposerState::Idle | ComposerState::Draft => "Say or do something",
        };
        let in_flight = matches!(state, ComposerState::Transcribing | ComposerState::Sending);
        Self {
            state,
            targets: inputs.targets.to_vec(),
            selected: target.map(|target| target.id.clone()),
            mic,
            live: match state {
                ComposerState::RequestingMic => Some("Waiting for permission...".to_owned()),
                ComposerState::Recording => Some(format_elapsed(inputs.elapsed_ms)),
                ComposerState::Transcribing => Some("Writing that down...".to_owned()),
                _ => None,
            },
            level_permille: if recording { inputs.level_permille } else { 0 },
            field_enabled: !inputs.blocked && !in_flight,
            field_placeholder,
            send_enabled: state == ComposerState::Draft
                && inputs.has_text
                && !inputs.blocked
                && target.is_some(),
            send_label: if state == ComposerState::Sending {
                "Sending..."
            } else {
                "Send"
            },
            field_limit: MAXIMUM_QUESTION_CHARACTERS,
            note: inputs.note.map(Note::text),
        }
    }
}
