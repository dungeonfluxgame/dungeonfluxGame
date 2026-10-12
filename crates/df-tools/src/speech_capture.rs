//! The hold-to-talk control as a pure state machine, so its rules are testable without a
//! browser or a clock: the driver passes the time of each event in and carries out the
//! returned [`Command`].
//!
//! One recording exists at a time. A press opens the microphone, a release (or the fifteen
//! second cap) ends the recording and sends it, and the transcript that comes back is only
//! ever a draft in the dialogue text box: nothing is said until the player sends it.
use std::time::Duration;

use crate::speech_clip::{
    MAXIMUM_CLIP_BYTES, MAXIMUM_RECORDING, MINIMUM_CLIP, SAMPLE_RATE, SpeechOutcome,
};

/// Where the control is in its cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Idle,
    /// Pressed; the browser has not yet handed over the microphone.
    Requesting(Early),
    Recording {
        started: Duration,
    },
    /// Recording ended; the clip is being decoded, uploaded and transcribed.
    Listening,
    /// A transcript sits in the text box for the player to edit and send.
    Review,
}

/// What happened to a press before the microphone arrived.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Early {
    Holding,
    Released,
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

/// A message for the player: every failure has its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Notice {
    MicrophoneRefused,
    NoMicrophone,
    InsecureContext,
    CannotRecord,
    MicrophoneFailed,
    ReadyToTalk,
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
            Self::ReadyToTalk => "Microphone allowed. Press and hold the button to talk.",
            Self::Discarded => "Recording discarded.",
            Self::TooShort => "That was too short to hear. Hold the button while you speak.",
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

/// The hold-to-talk state.
#[derive(Debug)]
pub(crate) struct Capture {
    phase: Phase,
    notice: Option<Notice>,
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
        }
    }

    pub(crate) const fn phase(&self) -> Phase {
        self.phase
    }

    pub(crate) const fn notice(&self) -> Option<Notice> {
        self.notice
    }

    /// Whole seconds recorded so far, while recording.
    pub(crate) fn elapsed_seconds(&self, now: Duration) -> Option<u64> {
        match self.phase {
            Phase::Recording { started } => Some(now.saturating_sub(started).as_secs()),
            _ => None,
        }
    }

    /// A deliberate press. Only an idle control (or one holding a draft) starts recording.
    pub(crate) fn press(&mut self, environment: Environment) -> Option<Command> {
        match self.phase {
            Phase::Idle | Phase::Review => {}
            Phase::Listening => {
                self.notice = Some(Notice::Busy);
                return None;
            }
            Phase::Requesting(_) | Phase::Recording { .. } => return None,
        }
        if !environment.secure_context {
            self.notice = Some(Notice::InsecureContext);
            return None;
        }
        if !environment.can_record {
            self.notice = Some(Notice::CannotRecord);
            return None;
        }
        self.notice = None;
        self.phase = Phase::Requesting(Early::Holding);
        Some(Command::OpenMicrophone)
    }

    /// The browser handed over the microphone and the recorder started.
    pub(crate) fn microphone_ready(&mut self, now: Duration) -> Option<Command> {
        match self.phase {
            Phase::Requesting(Early::Holding) => {
                self.phase = Phase::Recording { started: now };
                None
            }
            Phase::Requesting(early) => {
                self.phase = Phase::Idle;
                self.notice = Some(match early {
                    Early::Cancelled => Notice::Discarded,
                    _ => Notice::ReadyToTalk,
                });
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

    /// The press ended. `inside` is whether the pointer is still over the control: letting go
    /// anywhere else cancels.
    pub(crate) fn release(&mut self, now: Duration, inside: bool) -> Option<Command> {
        match self.phase {
            Phase::Requesting(Early::Holding) => {
                self.phase = Phase::Requesting(if inside {
                    Early::Released
                } else {
                    Early::Cancelled
                });
                None
            }
            Phase::Recording { started } => {
                if !inside {
                    return self.discard(Notice::Discarded);
                }
                if now.saturating_sub(started) < MINIMUM_CLIP {
                    return self.discard(Notice::TooShort);
                }
                self.phase = Phase::Listening;
                self.notice = None;
                Some(Command::FinishRecording)
            }
            _ => None,
        }
    }

    /// Escape, sliding off the control, losing focus: the recording is dropped unheard.
    pub(crate) fn cancel(&mut self) -> Option<Command> {
        match self.phase {
            Phase::Requesting(Early::Holding | Early::Released) => {
                self.phase = Phase::Requesting(Early::Cancelled);
                None
            }
            Phase::Recording { .. } => self.discard(Notice::Discarded),
            Phase::Listening => {
                self.phase = Phase::Idle;
                self.notice = Some(Notice::Discarded);
                Some(Command::AbortUpload)
            }
            _ => None,
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

    fn recording(started: u64) -> Capture {
        let mut capture = Capture::new();
        assert_eq!(capture.press(SECURE), Some(Command::OpenMicrophone));
        assert_eq!(capture.microphone_ready(at(started)), None);
        capture
    }

    #[test]
    fn a_full_cycle_goes_idle_recording_listening_review_idle() {
        let mut capture = Capture::new();
        assert_eq!(capture.phase(), Phase::Idle);

        assert_eq!(capture.press(SECURE), Some(Command::OpenMicrophone));
        assert_eq!(capture.phase(), Phase::Requesting(Early::Holding));
        assert_eq!(capture.microphone_ready(at(1_000)), None);
        assert_eq!(capture.phase(), Phase::Recording { started: at(1_000) });
        assert_eq!(capture.elapsed_seconds(at(3_500)), Some(2));

        assert_eq!(
            capture.release(at(3_500), true),
            Some(Command::FinishRecording)
        );
        assert_eq!(capture.phase(), Phase::Listening);
        assert!(capture.decoded(40_000, 80_044));
        assert!(capture.heard(SpeechOutcome::Heard, "who sent you"));
        assert_eq!(capture.phase(), Phase::Review);
        assert_eq!(capture.notice(), None);

        capture.settle_draft();
        assert_eq!(capture.phase(), Phase::Idle);
    }

    #[test]
    fn a_recording_is_dropped_unheard_when_cancelled_or_released_elsewhere() {
        let mut escaped = recording(0);
        assert_eq!(escaped.cancel(), Some(Command::Discard));
        assert_eq!(escaped.phase(), Phase::Idle);
        assert_eq!(escaped.notice(), Some(Notice::Discarded));

        let mut slid_off = recording(0);
        assert_eq!(slid_off.release(at(5_000), false), Some(Command::Discard));
        assert_eq!(slid_off.phase(), Phase::Idle);
        assert_eq!(slid_off.notice(), Some(Notice::Discarded));
    }

    #[test]
    fn a_clip_shorter_than_the_minimum_is_discarded() {
        let mut capture = recording(1_000);

        assert_eq!(capture.release(at(1_399), true), Some(Command::Discard));

        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(capture.notice(), Some(Notice::TooShort));
        let mut exact = recording(1_000);
        assert_eq!(
            exact.release(at(1_400), true),
            Some(Command::FinishRecording)
        );
    }

    #[test]
    fn the_recording_stops_itself_at_fifteen_seconds() {
        let mut capture = recording(2_000);

        assert_eq!(capture.tick(at(16_999)), None);
        assert_eq!(capture.phase(), Phase::Recording { started: at(2_000) });
        assert_eq!(capture.tick(at(17_000)), Some(Command::FinishRecording));

        assert_eq!(capture.phase(), Phase::Listening);
        assert_eq!(capture.notice(), Some(Notice::Capped));
        // The release that follows the cap changes nothing.
        assert_eq!(capture.release(at(18_000), true), None);
        assert_eq!(capture.phase(), Phase::Listening);
    }

    #[test]
    fn only_one_recording_exists_at_a_time() {
        let mut capture = Capture::new();
        capture.press(SECURE);
        assert_eq!(capture.press(SECURE), None);
        capture.microphone_ready(at(0));
        assert_eq!(capture.press(SECURE), None);
        capture.release(at(1_000), true);

        assert_eq!(capture.press(SECURE), None);
        assert_eq!(capture.phase(), Phase::Listening);
        assert_eq!(capture.notice(), Some(Notice::Busy));
    }

    #[test]
    fn letting_go_before_the_microphone_arrives_opens_nothing() {
        let mut released = Capture::new();
        released.press(SECURE);
        assert_eq!(released.release(at(100), true), None);
        assert_eq!(released.phase(), Phase::Requesting(Early::Released));

        assert_eq!(released.microphone_ready(at(900)), Some(Command::Discard));

        assert_eq!(released.phase(), Phase::Idle);
        assert_eq!(released.notice(), Some(Notice::ReadyToTalk));

        let mut escaped = Capture::new();
        escaped.press(SECURE);
        assert_eq!(escaped.cancel(), None);
        assert_eq!(escaped.press(SECURE), None);
        assert_eq!(escaped.microphone_ready(at(900)), Some(Command::Discard));
        assert_eq!(escaped.notice(), Some(Notice::Discarded));
    }

    #[test]
    fn nothing_but_a_deliberate_press_asks_for_the_microphone() {
        let mut capture = Capture::new();
        assert_eq!(capture.tick(at(10_000)), None);
        assert_eq!(capture.release(at(10), true), None);
        assert_eq!(capture.cancel(), None);
        assert_eq!(capture.recorder_failed(), None);
        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(capture.press(SECURE), Some(Command::OpenMicrophone));
    }

    /// The browser module is not compiled for native tests, so its one rule is checked in the
    /// source: the only call that can raise a permission prompt sits in `open_microphone`, and
    /// that function starts only from the press command.
    #[test]
    fn the_prompting_browser_call_is_only_reachable_from_a_press() {
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
            capture.press(SECURE);
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
        assert_eq!(insecure.press(plain_http), None);
        assert_eq!(insecure.phase(), Phase::Idle);
        assert_eq!(insecure.notice(), Some(Notice::InsecureContext));
        assert!(insecure.notice().unwrap().message().contains("https"));

        let mut unsupported = Capture::new();
        let no_recorder = Environment {
            secure_context: true,
            can_record: false,
        };
        assert_eq!(unsupported.press(no_recorder), None);
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
            capture.release(at(2_000), true);

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
            capture.release(at(2_000), true);

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
        capture.release(at(2_000), true);

        assert!(!capture.heard(SpeechOutcome::Heard, "   "));

        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(capture.notice(), Some(Notice::NothingHeard));
    }

    #[test]
    fn cancelling_while_listening_abandons_the_upload() {
        let mut capture = recording(0);
        capture.release(at(2_000), true);

        assert_eq!(capture.cancel(), Some(Command::AbortUpload));

        assert_eq!(capture.phase(), Phase::Idle);
        // A late answer for the abandoned clip changes nothing.
        assert!(!capture.heard(SpeechOutcome::Heard, "late words"));
        assert_eq!(capture.phase(), Phase::Idle);
    }

    #[test]
    fn a_lost_connection_and_an_unreadable_recording_each_end_listening() {
        let mut lost = recording(0);
        lost.release(at(2_000), true);
        lost.upload_failed();
        assert_eq!(lost.notice(), Some(Notice::ConnectionLost));

        let mut unreadable = recording(0);
        unreadable.release(at(2_000), true);
        unreadable.unreadable();
        assert_eq!(unreadable.notice(), Some(Notice::Unreadable));
        assert_eq!(unreadable.phase(), Phase::Idle);
    }

    #[test]
    fn a_recorder_that_fails_mid_recording_is_dropped_with_its_own_message() {
        let mut capture = recording(0);

        assert_eq!(capture.recorder_failed(), Some(Command::Discard));

        assert_eq!(capture.phase(), Phase::Idle);
        assert_eq!(capture.notice(), Some(Notice::MicrophoneFailed));
        assert_eq!(capture.recorder_failed(), None);
    }

    #[test]
    fn a_draft_does_not_block_talking_again() {
        let mut capture = recording(0);
        capture.release(at(2_000), true);
        capture.decoded(40_000, 80_044);
        capture.heard(SpeechOutcome::Heard, "first");
        assert_eq!(capture.phase(), Phase::Review);

        assert_eq!(capture.press(SECURE), Some(Command::OpenMicrophone));
    }
}
