//! The talk control as a pure state machine, so its rules are testable without a browser or a
//! clock: the driver passes the time of each event in and carries out the returned [`Command`].
//!
//! A tap starts listening and the next tap stops it. A press counts as a tap only when it is
//! released over the control: a cancelled pointer or a drag off the control abandons the press
//! and toggles nothing. One recording exists at a time. The first tap opens the microphone, the
//! second tap (or the fifteen second cap) ends the recording and sends it, and the transcript that
//! comes back is only ever a draft in the dialogue text box: nothing is said until the player sends
//! it. Listening also ends without a tap when the page is hidden, the connection is lost, the text
//! box goes away, or the player starts typing.
use std::time::Duration;

use crate::speech_clip::{
    MAXIMUM_CLIP_BYTES, MAXIMUM_RECORDING, MINIMUM_CLIP, SAMPLE_RATE, SpeechOutcome,
};

/// Where the control is in its cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Idle,
    /// Tapped; the browser has not yet handed over the microphone.
    Requesting(Early),
    Recording {
        started: Duration,
    },
    /// Recording ended; the clip is being decoded, uploaded and transcribed.
    Listening,
    /// A transcript sits in the text box for the player to edit and send.
    Review,
}

/// What became of a start before the microphone arrived.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Early {
    /// The recording begins as soon as the microphone is handed over.
    Wanted,
    /// Interrupted while starting: the microphone is released unused.
    Cancelled,
}

/// What the page tells the control about the place it runs in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Environment {
    pub(crate) secure_context: bool,
    pub(crate) can_record: bool,
}

/// Why the browser did not hand over a microphone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MicrophoneFault {
    PermissionRefused,
    NoMicrophone,
    Other,
}

/// Work for the driver. The control never touches the browser itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    /// Ask for the microphone and start a recorder when it arrives.
    OpenMicrophone,
    /// Stop the recorder and send what it captured.
    FinishRecording,
    /// Stop the recorder and every track, and drop what was captured.
    Discard,
    /// Give up on the clip being heard.
    AbortUpload,
}

/// Why listening ends without a tap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Interruption {
    /// The player pressed Escape or the Cancel button.
    Cancelled,
    /// The page was hidden.
    PageHidden,
    /// The phone lost its connection to the game.
    ConnectionLost,
    /// The scene changed so that the text box went away.
    SceneChanged,
    /// The player started typing in the text box.
    Typing,
}

impl Interruption {
    const fn notice(self) -> Notice {
        match self {
            Self::ConnectionLost => Notice::ConnectionLost,
            Self::Cancelled | Self::PageHidden | Self::SceneChanged | Self::Typing => {
                Notice::Discarded
            }
        }
    }
}

/// A message for the player: every failure has its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Notice {
    MicrophoneRefused,
    NoMicrophone,
    InsecureContext,
    CannotRecord,
    MicrophoneFailed,
    Discarded,
    TooShort,
    Empty,
    TooLarge,
    TooLong,
    Capped,
    Busy,
    NothingHeard,
    Unavailable,
    BudgetSpent,
    ProviderFailed,
    TimedOut,
    NotAccepted,
    Unreadable,
    ConnectionLost,
}

impl Notice {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::MicrophoneRefused => {
                "The microphone is blocked for this page. Allow it in your browser's site \
                 settings, or type instead."
            }
            Self::NoMicrophone => "No microphone was found on this device. You can type instead.",
            Self::InsecureContext => {
                "Your browser only allows the microphone on https or localhost, and this page \
                 was opened over plain http. Open the game over https, or type instead."
            }
            Self::CannotRecord => "This browser cannot record audio. You can type instead.",
            Self::MicrophoneFailed => {
                "The microphone could not be started. Try again, or type instead."
            }
            Self::Discarded => "Recording discarded.",
            Self::TooShort => "That was too short to hear. Tap to start, speak, then tap again.",
            Self::Empty => "Nothing was recorded. Check your microphone, or type instead.",
            Self::TooLarge => "That recording was too large to send. Try a shorter line.",
            Self::TooLong => "That recording was too long. Keep it under fifteen seconds.",
            Self::Capped => "Stopped at fifteen seconds.",
            Self::Busy => "Your last recording is still being heard. Try again in a moment.",
            Self::NothingHeard => "Nothing was heard. Try again, or type instead.",
            Self::Unavailable => "Speech input is unavailable right now. You can type instead.",
            Self::BudgetSpent => "Speech input is used up for this game. You can still type.",
            Self::ProviderFailed => "Speech recognition failed. Try again, or type instead.",
            Self::TimedOut => "Speech recognition took too long. Try again, or type instead.",
            Self::NotAccepted => {
                "The server did not accept this recording. Reconnect, then try again."
            }
            Self::Unreadable => "That recording could not be read. Try again, or type instead.",
            Self::ConnectionLost => {
                "The recording could not be sent. Check your connection, or type instead."
            }
        }
    }

    /// The notice a refused or failed upload shows.
    pub(crate) const fn for_outcome(outcome: SpeechOutcome) -> Self {
        match outcome {
            SpeechOutcome::Heard | SpeechOutcome::Available | SpeechOutcome::NothingHeard => {
                Self::NothingHeard
            }
            SpeechOutcome::Unavailable => Self::Unavailable,
            SpeechOutcome::BudgetSpent => Self::BudgetSpent,
            SpeechOutcome::Busy => Self::Busy,
            SpeechOutcome::TooLarge => Self::TooLarge,
            SpeechOutcome::TooLong => Self::TooLong,
            SpeechOutcome::TooShort => Self::TooShort,
            SpeechOutcome::UnsupportedType | SpeechOutcome::Malformed => Self::Unreadable,
            SpeechOutcome::Unauthenticated | SpeechOutcome::Refused => Self::NotAccepted,
            SpeechOutcome::ProviderFailed => Self::ProviderFailed,
            SpeechOutcome::TimedOut => Self::TimedOut,
        }
    }
}

/// The talk state.
#[derive(Debug)]
pub(crate) struct Capture {
    phase: Phase,
    notice: Option<Notice>,
    /// A pointer or key is down on the control and counts as a tap if released there.
    pressed: bool,
}

impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}

impl Capture {
    pub(crate) const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            notice: None,
            pressed: false,
        }
    }

    pub(crate) const fn phase(&self) -> Phase {
        self.phase
    }

    pub(crate) const fn notice(&self) -> Option<Notice> {
        self.notice
    }

    /// Whether the control is listening for the player's words: the microphone is opening for a
    /// tap or is recording. The table's speech-intent mark follows this.
    pub(crate) const fn is_listening(&self) -> bool {
        matches!(
            self.phase,
            Phase::Requesting(Early::Wanted) | Phase::Recording { .. }
        )
    }

    /// Whether a recording or its upload is under way, so typing must interrupt it.
    pub(crate) const fn is_busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Requesting(_) | Phase::Recording { .. } | Phase::Listening
        )
    }

    /// Whole seconds recorded so far, while recording.
    pub(crate) fn elapsed_seconds(&self, now: Duration) -> Option<u64> {
        match self.phase {
            Phase::Recording { started } => Some(now.saturating_sub(started).as_secs()),
            _ => None,
        }
    }

    /// A pointer or key went down on the control. Nothing starts until it is released there.
    pub(crate) const fn press_down(&mut self) {
        self.pressed = true;
    }

    /// The press ended. Released over the control after a press on it, this is a tap: it starts
    /// listening when the control is idle or holds a draft, and stops it while recording. A tap
    /// while the microphone is opening is ignored, and so is one while the clip is being heard.
    pub(crate) fn release(
        &mut self,
        now: Duration,
        inside: bool,
        environment: Environment,
    ) -> Option<Command> {
        let tapped = std::mem::take(&mut self.pressed) && inside;
        if !tapped {
            return None;
        }
        match self.phase {
            Phase::Idle | Phase::Review => self.start(environment),
            Phase::Recording { started } => self.stop(now, started),
            Phase::Listening => {
                self.notice = Some(Notice::Busy);
                None
            }
            Phase::Requesting(_) => None,
        }
    }

    /// The press stopped being a tap: the pointer was cancelled or dragged off the control, or the
    /// key lost focus. A recording already under way goes on until the next tap.
    pub(crate) const fn abandon_press(&mut self) {
        self.pressed = false;
    }

    fn start(&mut self, environment: Environment) -> Option<Command> {
        if !environment.secure_context {
            self.notice = Some(Notice::InsecureContext);
            return None;
        }
        if !environment.can_record {
            self.notice = Some(Notice::CannotRecord);
            return None;
        }
        self.notice = None;
        self.phase = Phase::Requesting(Early::Wanted);
        Some(Command::OpenMicrophone)
    }

    fn stop(&mut self, now: Duration, started: Duration) -> Option<Command> {
        if now.saturating_sub(started) < MINIMUM_CLIP {
            return self.discard(Notice::TooShort);
        }
        self.phase = Phase::Listening;
        self.notice = None;
        Some(Command::FinishRecording)
    }

    /// The browser handed over the microphone and the recorder started.
    pub(crate) fn microphone_ready(&mut self, now: Duration) -> Option<Command> {
        match self.phase {
            Phase::Requesting(Early::Wanted) => {
                self.phase = Phase::Recording { started: now };
                None
            }
            Phase::Requesting(Early::Cancelled) => {
                self.phase = Phase::Idle;
                self.notice = Some(Notice::Discarded);
                Some(Command::Discard)
            }
            // A microphone nobody is waiting for must not stay open.
            _ => Some(Command::Discard),
        }
    }

    pub(crate) fn microphone_failed(&mut self, fault: MicrophoneFault) {
        if !matches!(self.phase, Phase::Requesting(_)) {
            return;
        }
        self.phase = Phase::Idle;
        self.notice = Some(match fault {
            MicrophoneFault::PermissionRefused => Notice::MicrophoneRefused,
            MicrophoneFault::NoMicrophone => Notice::NoMicrophone,
            MicrophoneFault::Other => Notice::MicrophoneFailed,
        });
    }

    /// Ends listening for a reason other than a tap. A start still opening its microphone is
    /// cancelled, a recording is dropped unheard, and an upload is abandoned so that a late
    /// transcript cannot replace what the player typed. An idle control or a draft is unaffected.
    pub(crate) fn interrupt(&mut self, why: Interruption) -> Option<Command> {
        self.pressed = false;
        match self.phase {
            Phase::Requesting(_) => {
                self.phase = Phase::Requesting(Early::Cancelled);
                None
            }
            Phase::Recording { .. } => self.discard(why.notice()),
            Phase::Listening => {
                self.phase = Phase::Idle;
                self.notice = Some(why.notice());
                Some(Command::AbortUpload)
            }
            Phase::Idle | Phase::Review => None,
        }
    }

    /// Time passing; ends the recording at the fifteen second cap.
    pub(crate) fn tick(&mut self, now: Duration) -> Option<Command> {
        let Phase::Recording { started } = self.phase else {
            return None;
        };
        if now.saturating_sub(started) < MAXIMUM_RECORDING {
            return None;
        }
        self.phase = Phase::Listening;
        self.notice = Some(Notice::Capped);
        Some(Command::FinishRecording)
    }

    /// The recorder failed while recording or while handing over its data.
    pub(crate) fn recorder_failed(&mut self) -> Option<Command> {
        match self.phase {
            Phase::Recording { .. } | Phase::Listening => self.discard(Notice::MicrophoneFailed),
            _ => None,
        }
    }

    /// The recording was decoded to `samples` mono samples at the clip sample rate and
    /// encoded to `encoded_bytes`. Returns whether it should be uploaded.
    pub(crate) fn decoded(&mut self, samples: usize, encoded_bytes: usize) -> bool {
        if self.phase != Phase::Listening {
            return false;
        }
        let refusal = if samples == 0 {
            Some(Notice::Empty)
        } else if (samples as u64) * 1_000_000 / u64::from(SAMPLE_RATE)
            < MINIMUM_CLIP.as_micros() as u64
        {
            Some(Notice::TooShort)
        } else if encoded_bytes > MAXIMUM_CLIP_BYTES {
            Some(Notice::TooLarge)
        } else {
            None
        };
        if let Some(notice) = refusal {
            self.phase = Phase::Idle;
            self.notice = Some(notice);
            return false;
        }
        true
    }

    /// The server answered with `outcome`; `transcript` is non-empty only for a hearing.
    /// Returns whether a draft is now waiting for the player.
    pub(crate) fn heard(&mut self, outcome: SpeechOutcome, transcript: &str) -> bool {
        if self.phase != Phase::Listening {
            return false;
        }
        if outcome == SpeechOutcome::Heard && !transcript.trim().is_empty() {
            self.phase = Phase::Review;
            self.notice = None;
            return true;
        }
        self.phase = Phase::Idle;
        self.notice = Some(Notice::for_outcome(outcome));
        false
    }

    /// The upload never reached an answer.
    pub(crate) fn upload_failed(&mut self) {
        if self.phase == Phase::Listening {
            self.phase = Phase::Idle;
            self.notice = Some(Notice::ConnectionLost);
        }
    }

    /// The decoder or encoder could not turn the recording into a clip.
    pub(crate) fn unreadable(&mut self) {
        if self.phase == Phase::Listening {
            self.phase = Phase::Idle;
            self.notice = Some(Notice::Unreadable);
        }
    }

    /// The player sent or cleared the draft.
    pub(crate) fn settle_draft(&mut self) {
        if self.phase == Phase::Review {
            self.phase = Phase::Idle;
            self.notice = None;
        }
    }

    fn discard(&mut self, notice: Notice) -> Option<Command> {
        self.phase = Phase::Idle;
        self.notice = Some(notice);
        Some(Command::Discard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECURE: Environment = Environment {
        secure_context: true,
        can_record: true,
    };

    fn at(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    /// A complete tap: the press goes down and is released over the control at `milliseconds`.
    fn tap_in(
        capture: &mut Capture,
        milliseconds: u64,
        environment: Environment,
    ) -> Option<Command> {
        capture.press_down();
        capture.release(at(milliseconds), true, environment)
    }

    fn tap(capture: &mut Capture, milliseconds: u64) -> Option<Command> {
        tap_in(capture, milliseconds, SECURE)
    }

    fn recording(started: u64) -> Capture {
        let mut capture = Capture::new();
        assert_eq!(tap(&mut capture, started), Some(Command::OpenMicrophone));
        assert_eq!(capture.microphone_ready(at(started)), None);
        capture
    }

    #[test]
    fn a_tap_starts_listening_and_a_second_tap_stops_to_review() {
        let mut capture = Capture::new();
        assert_eq!(capture.phase(), Phase::Idle);

        assert_eq!(tap(&mut capture, 0), Some(Command::OpenMicrophone));
        assert_eq!(capture.phase(), Phase::Requesting(Early::Wanted));
        assert!(capture.is_listening());
        assert_eq!(capture.microphone_ready(at(1_000)), None);
        assert_eq!(capture.phase(), Phase::Recording { started: at(1_000) });
        assert_eq!(capture.elapsed_seconds(at(3_500)), Some(2));

        assert_eq!(tap(&mut capture, 3_500), Some(Command::FinishRecording));
        assert_eq!(capture.phase(), Phase::Listening);
        assert!(!capture.is_listening());
        assert!(capture.decoded(40_000, 80_044));
        assert!(capture.heard(SpeechOutcome::Heard, "who sent you"));
        assert_eq!(capture.phase(), Phase::Review);
        assert_eq!(capture.notice(), None);

        capture.settle_draft();
        assert_eq!(capture.phase(), Phase::Idle);
    }

    #[test]
    fn a_cancelled_pointer_or_a_drag_off_the_control_does_not_toggle() {
        let mut capture = recording(0);

        capture.press_down();
        capture.abandon_press();
        assert_eq!(capture.release(at(5_000), true, SECURE), None);

        capture.press_down();
        assert_eq!(capture.release(at(5_000), false, SECURE), None);

        assert_eq!(capture.phase(), Phase::Recording { started: at(0) });
        assert_eq!(capture.notice(), None);
    }

    #[test]
    fn a_release_without_a_press_on_the_control_is_not_a_tap() {
        let mut capture = Capture::new();
        assert_eq!(capture.tick(at(10_000)), None);
        assert_eq!(capture.release(at(10), true, SECURE), None);
        assert_eq!(capture.interrupt(Interruption::Cancelled), None);
        assert_eq!(capture.recorder_failed(), None);
        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(tap(&mut capture, 20), Some(Command::OpenMicrophone));
    }

    #[test]
    fn a_clip_shorter_than_the_minimum_is_discarded() {
        let mut capture = recording(1_000);

        assert_eq!(tap(&mut capture, 1_399), Some(Command::Discard));

        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(capture.notice(), Some(Notice::TooShort));
        let mut exact = recording(1_000);
        assert_eq!(tap(&mut exact, 1_400), Some(Command::FinishRecording));
    }

    #[test]
    fn the_recording_stops_itself_at_fifteen_seconds() {
        let mut capture = recording(2_000);

        assert_eq!(capture.tick(at(16_999)), None);
        assert_eq!(capture.phase(), Phase::Recording { started: at(2_000) });
        assert_eq!(capture.tick(at(17_000)), Some(Command::FinishRecording));

        assert_eq!(capture.phase(), Phase::Listening);
        assert_eq!(capture.notice(), Some(Notice::Capped));
        // A tap after the cap changes nothing: the clip is already being heard.
        assert_eq!(tap(&mut capture, 18_000), None);
        assert_eq!(capture.phase(), Phase::Listening);
    }

    #[test]
    fn only_one_recording_exists_and_taps_while_starting_or_stopping_are_ignored() {
        let mut capture = Capture::new();
        assert_eq!(tap(&mut capture, 0), Some(Command::OpenMicrophone));
        assert_eq!(tap(&mut capture, 10), None);
        assert_eq!(capture.phase(), Phase::Requesting(Early::Wanted));
        assert_eq!(capture.notice(), None);
        capture.microphone_ready(at(0));
        assert_eq!(tap(&mut capture, 1_000), Some(Command::FinishRecording));

        assert_eq!(tap(&mut capture, 1_100), None);
        assert_eq!(capture.phase(), Phase::Listening);
        assert_eq!(capture.notice(), Some(Notice::Busy));
    }

    #[test]
    fn a_start_cancelled_before_the_microphone_arrives_releases_it_unused() {
        let mut capture = Capture::new();
        assert_eq!(tap(&mut capture, 0), Some(Command::OpenMicrophone));

        assert_eq!(capture.interrupt(Interruption::Cancelled), None);
        assert!(!capture.is_listening());
        assert_eq!(capture.microphone_ready(at(900)), Some(Command::Discard));

        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(capture.notice(), Some(Notice::Discarded));
        // A tap after that starts a fresh recording.
        assert_eq!(tap(&mut capture, 1_000), Some(Command::OpenMicrophone));
    }

    #[test]
    fn the_prompting_browser_call_is_only_reachable_from_a_press() {
        // The browser module is not compiled for native tests, so its one rule is checked in the
        // source: the only call that can raise a permission prompt sits in `open_microphone`, and
        // that function starts only from the press command.
        let source = include_str!("gameplay_browser/talk_control.rs");
        assert_eq!(source.matches("get_user_media").count(), 1);
        assert_eq!(source.matches("enumerate_devices").count(), 0);
        assert_eq!(source.matches("permissions()").count(), 0);
        assert_eq!(source.matches("open_microphone(").count(), 2);
        let start = source.find("Some(Command::OpenMicrophone) =>").unwrap();
        assert!(source[start..start + 200].contains("open_microphone(weak)"));
        let call = source.find("get_user_media").unwrap();
        let function = source.find("async fn open_microphone").unwrap();
        let next_function = source[function + 1..].find("\nfn ").unwrap() + function;
        assert!((function..next_function).contains(&call));
    }

    #[test]
    fn a_microphone_nobody_is_waiting_for_is_released() {
        let mut capture = Capture::new();

        assert_eq!(capture.microphone_ready(at(0)), Some(Command::Discard));
        assert_eq!(capture.phase(), Phase::Idle);
    }

    #[test]
    fn microphone_faults_each_have_their_own_message() {
        let mut messages = Vec::new();
        for (fault, notice) in [
            (
                MicrophoneFault::PermissionRefused,
                Notice::MicrophoneRefused,
            ),
            (MicrophoneFault::NoMicrophone, Notice::NoMicrophone),
            (MicrophoneFault::Other, Notice::MicrophoneFailed),
        ] {
            let mut capture = Capture::new();
            tap(&mut capture, 0);
            capture.microphone_failed(fault);
            assert_eq!(capture.phase(), Phase::Idle);
            assert_eq!(capture.notice(), Some(notice));
            messages.push(notice.message());
        }
        messages.sort_unstable();
        messages.dedup();
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn an_insecure_page_or_a_browser_without_recording_says_so_and_never_opens_the_microphone() {
        let mut insecure = Capture::new();
        let plain_http = Environment {
            secure_context: false,
            can_record: true,
        };
        assert_eq!(tap_in(&mut insecure, 0, plain_http), None);
        assert_eq!(insecure.phase(), Phase::Idle);
        assert_eq!(insecure.notice(), Some(Notice::InsecureContext));
        assert!(insecure.notice().unwrap().message().contains("https"));

        let mut unsupported = Capture::new();
        let no_recorder = Environment {
            secure_context: true,
            can_record: false,
        };
        assert_eq!(tap_in(&mut unsupported, 0, no_recorder), None);
        assert_eq!(unsupported.notice(), Some(Notice::CannotRecord));
    }

    #[test]
    fn an_empty_or_oversized_decoded_clip_is_not_uploaded() {
        for (samples, bytes, notice) in [
            (0, 44, Notice::Empty),
            (3_000, 6_044, Notice::TooShort),
            (100_000, MAXIMUM_CLIP_BYTES + 1, Notice::TooLarge),
        ] {
            let mut capture = recording(0);
            tap(&mut capture, 2_000);

            assert!(!capture.decoded(samples, bytes));

            assert_eq!(capture.phase(), Phase::Idle);
            assert_eq!(capture.notice(), Some(notice));
        }
    }

    #[test]
    fn every_server_outcome_ends_listening_with_its_own_message() {
        let failures = [
            SpeechOutcome::NothingHeard,
            SpeechOutcome::Unavailable,
            SpeechOutcome::BudgetSpent,
            SpeechOutcome::Busy,
            SpeechOutcome::TooLarge,
            SpeechOutcome::TooLong,
            SpeechOutcome::TooShort,
            SpeechOutcome::Malformed,
            SpeechOutcome::Unauthenticated,
            SpeechOutcome::ProviderFailed,
            SpeechOutcome::TimedOut,
        ];
        let mut messages = Vec::new();
        for outcome in failures {
            let mut capture = recording(0);
            tap(&mut capture, 2_000);

            assert!(!capture.heard(outcome, ""));

            assert_eq!(capture.phase(), Phase::Idle);
            messages.push(capture.notice().unwrap().message());
        }
        messages.sort_unstable();
        messages.dedup();
        assert_eq!(messages.len(), failures.len());
    }

    #[test]
    fn a_hearing_with_no_words_is_nothing_heard_not_a_draft() {
        let mut capture = recording(0);
        tap(&mut capture, 2_000);

        assert!(!capture.heard(SpeechOutcome::Heard, "   "));

        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(capture.notice(), Some(Notice::NothingHeard));
    }

    #[test]
    fn cancelling_while_listening_abandons_the_upload() {
        let mut capture = recording(0);
        tap(&mut capture, 2_000);

        assert_eq!(
            capture.interrupt(Interruption::Cancelled),
            Some(Command::AbortUpload)
        );

        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(capture.notice(), Some(Notice::Discarded));
        // A late answer for the abandoned clip changes nothing.
        assert!(!capture.heard(SpeechOutcome::Heard, "late words"));
        assert_eq!(capture.phase(), Phase::Idle);
    }

    #[test]
    fn hiding_the_page_ends_a_recording_unheard() {
        let mut capture = recording(0);

        assert_eq!(
            capture.interrupt(Interruption::PageHidden),
            Some(Command::Discard)
        );

        assert_eq!(capture.phase(), Phase::Idle);
        assert!(!capture.is_listening());
        assert_eq!(capture.notice(), Some(Notice::Discarded));
    }

    #[test]
    fn losing_the_connection_ends_a_recording_or_an_upload_with_its_own_message() {
        let mut recording_lost = recording(0);
        assert_eq!(
            recording_lost.interrupt(Interruption::ConnectionLost),
            Some(Command::Discard)
        );
        assert_eq!(recording_lost.phase(), Phase::Idle);
        assert_eq!(recording_lost.notice(), Some(Notice::ConnectionLost));

        let mut upload_lost = recording(0);
        assert_eq!(tap(&mut upload_lost, 2_000), Some(Command::FinishRecording));
        assert_eq!(
            upload_lost.interrupt(Interruption::ConnectionLost),
            Some(Command::AbortUpload)
        );
        assert_eq!(upload_lost.phase(), Phase::Idle);
        assert_eq!(upload_lost.notice(), Some(Notice::ConnectionLost));
    }

    #[test]
    fn a_failed_upload_and_an_unreadable_recording_each_end_listening() {
        let mut lost = recording(0);
        assert_eq!(tap(&mut lost, 2_000), Some(Command::FinishRecording));
        lost.upload_failed();
        assert_eq!(lost.notice(), Some(Notice::ConnectionLost));

        let mut unreadable = recording(0);
        assert_eq!(tap(&mut unreadable, 2_000), Some(Command::FinishRecording));
        unreadable.unreadable();
        assert_eq!(unreadable.notice(), Some(Notice::Unreadable));
        assert_eq!(unreadable.phase(), Phase::Idle);
    }

    #[test]
    fn typing_ends_listening_and_a_late_transcript_is_not_applied() {
        let mut recording_typed = recording(0);
        assert_eq!(
            recording_typed.interrupt(Interruption::Typing),
            Some(Command::Discard)
        );
        assert_eq!(recording_typed.phase(), Phase::Idle);

        let mut upload_typed = recording(0);
        assert_eq!(
            tap(&mut upload_typed, 2_000),
            Some(Command::FinishRecording)
        );
        assert_eq!(
            upload_typed.interrupt(Interruption::Typing),
            Some(Command::AbortUpload)
        );
        // A transcript that comes back later must not replace what the player typed.
        assert!(!upload_typed.heard(SpeechOutcome::Heard, "late words"));
        assert_eq!(upload_typed.phase(), Phase::Idle);
    }

    #[test]
    fn typing_while_idle_or_holding_a_draft_changes_nothing() {
        let mut idle = Capture::new();
        assert_eq!(idle.interrupt(Interruption::Typing), None);
        assert_eq!(idle.phase(), Phase::Idle);

        let mut draft = recording(0);
        assert_eq!(tap(&mut draft, 2_000), Some(Command::FinishRecording));
        assert!(draft.decoded(40_000, 80_044));
        assert!(draft.heard(SpeechOutcome::Heard, "words"));

        assert_eq!(draft.interrupt(Interruption::Typing), None);
        assert_eq!(draft.phase(), Phase::Review);
    }

    #[test]
    fn the_text_box_going_away_ends_a_start_and_a_recording() {
        let mut starting = Capture::new();
        assert_eq!(tap(&mut starting, 0), Some(Command::OpenMicrophone));
        assert_eq!(starting.interrupt(Interruption::SceneChanged), None);
        assert_eq!(starting.microphone_ready(at(300)), Some(Command::Discard));
        assert_eq!(starting.phase(), Phase::Idle);

        let mut recording_now = recording(0);
        assert_eq!(
            recording_now.interrupt(Interruption::SceneChanged),
            Some(Command::Discard)
        );
        assert_eq!(recording_now.phase(), Phase::Idle);
        assert_eq!(recording_now.notice(), Some(Notice::Discarded));
    }

    #[test]
    fn the_table_mark_follows_only_the_microphone_opening_or_recording() {
        let mut capture = Capture::new();
        assert!(!capture.is_listening());

        tap(&mut capture, 0);
        assert!(capture.is_listening());
        capture.microphone_ready(at(0));
        assert!(capture.is_listening());

        tap(&mut capture, 2_000);
        assert!(!capture.is_listening());
        assert!(capture.is_busy());
    }

    #[test]
    fn a_draft_does_not_block_talking_again() {
        let mut capture = recording(0);
        tap(&mut capture, 2_000);
        capture.decoded(40_000, 80_044);
        capture.heard(SpeechOutcome::Heard, "first");
        assert_eq!(capture.phase(), Phase::Review);

        assert_eq!(tap(&mut capture, 3_000), Some(Command::OpenMicrophone));
    }
}
