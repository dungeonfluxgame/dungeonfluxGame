use crate::composer::{
    ComposerEvent, ComposerInputs, ComposerState, ComposerView, MicLook, Note, SpeechAvailability,
    Target, TranscribeFailure, format_elapsed, resolve_target, step, targets_for,
};
use crate::voice_capture::MicrophoneError;
use crate::voice_wire::{MissingSetting, PersonaSummary, SpeechErrorKind};

use ComposerEvent as Event;
use ComposerState as State;

fn persona(id: &str, name: &str) -> PersonaSummary {
    PersonaSummary {
        id: id.to_owned(),
        display_name: name.to_owned(),
        job: String::new(),
    }
}

fn target(id: &str) -> Target {
    Target {
        id: id.to_owned(),
        name: id.to_owned(),
    }
}

#[test]
fn a_spoken_question_goes_from_idle_through_recording_and_a_draft_to_sent() {
    let mut state = State::Idle;
    for (event, has_text, expected) in [
        (Event::MicPressed, false, State::RequestingMic),
        (Event::MicGranted, false, State::Recording),
        (Event::StopPressed, false, State::Transcribing),
        (Event::TranscriptReady, true, State::Draft),
        (Event::SendPressed, true, State::Sending),
        (Event::Answered, false, State::Idle),
    ] {
        state = step(state, event, has_text);
        assert_eq!(state, expected, "{event:?}");
    }
}

#[test]
fn cancelling_a_recording_returns_to_the_text_that_was_there() {
    assert_eq!(
        step(State::Recording, Event::CancelPressed, false),
        State::Idle
    );
    assert_eq!(
        step(State::Recording, Event::CancelPressed, true),
        State::Draft
    );
    assert_eq!(
        step(State::RequestingMic, Event::CancelPressed, true),
        State::Draft
    );
}

#[test]
fn a_hidden_page_or_a_lost_host_ends_a_recording_that_nobody_accepted() {
    for event in [Event::PageHidden, Event::HostLost] {
        for state in [State::RequestingMic, State::Recording, State::Transcribing] {
            assert_eq!(
                step(state, event, false),
                State::Idle,
                "{state:?} {event:?}"
            );
        }
    }
}

#[test]
fn typing_ends_a_recording_and_edits_a_draft() {
    assert_eq!(step(State::Recording, Event::Edited, true), State::Draft);
    assert_eq!(step(State::Draft, Event::Edited, false), State::Idle);
    assert_eq!(step(State::Idle, Event::Edited, true), State::Draft);
}

#[test]
fn a_failed_capture_or_transcription_leaves_the_box_as_it_was() {
    assert_eq!(
        step(State::Recording, Event::CaptureFailed, true),
        State::Draft
    );
    assert_eq!(
        step(State::RequestingMic, Event::CaptureFailed, false),
        State::Idle
    );
    assert_eq!(
        step(State::Transcribing, Event::TranscriptFailed, true),
        State::Draft
    );
}

#[test]
fn a_failed_question_keeps_the_text_to_send_again() {
    assert_eq!(step(State::Sending, Event::AskFailed, true), State::Draft);
}

#[test]
fn a_lost_host_does_not_cancel_a_question_already_sent() {
    assert_eq!(step(State::Sending, Event::HostLost, true), State::Sending);
}

#[test]
fn an_event_with_no_meaning_in_a_state_changes_nothing() {
    assert_eq!(step(State::Idle, Event::StopPressed, false), State::Idle);
    assert_eq!(
        step(State::Idle, Event::TranscriptReady, false),
        State::Idle
    );
    assert_eq!(
        step(State::Sending, Event::MicPressed, true),
        State::Sending
    );
    assert_eq!(
        step(State::Transcribing, Event::MicPressed, false),
        State::Transcribing
    );
    assert_eq!(step(State::Idle, Event::SendPressed, false), State::Idle);
}

#[test]
fn an_empty_draft_cannot_be_sent() {
    assert_eq!(step(State::Draft, Event::SendPressed, false), State::Draft);
}

#[test]
fn only_a_request_or_a_recording_holds_the_microphone() {
    assert!(State::RequestingMic.holds_microphone());
    assert!(State::Recording.holds_microphone());
    assert!(!State::Transcribing.holds_microphone());
    assert!(!State::Draft.holds_microphone());
}

#[test]
fn the_composer_lists_the_scenes_characters_that_have_a_persona() {
    let cast = vec![
        "innkeeper".to_owned(),
        "guard".to_owned(),
        "innkeeper".to_owned(),
    ];
    let personas = vec![persona("innkeeper", "Marta Venn"), persona("cook", "Bram")];
    let targets = targets_for(&cast, &personas);
    assert_eq!(
        targets,
        vec![Target {
            id: "innkeeper".to_owned(),
            name: "Marta Venn".to_owned()
        }]
    );
    assert!(targets_for(&[], &personas).is_empty());
}

#[test]
fn the_innkeeper_is_the_default_target_and_a_missing_choice_falls_back() {
    let targets = vec![target("guard"), target("innkeeper")];
    assert_eq!(
        resolve_target(&targets, None).map(|t| t.id.as_str()),
        Some("innkeeper")
    );
    assert_eq!(
        resolve_target(&targets, Some("guard")).map(|t| t.id.as_str()),
        Some("guard")
    );
    assert_eq!(
        resolve_target(&targets, Some("gone")).map(|t| t.id.as_str()),
        Some("innkeeper")
    );
    let others = vec![target("guard")];
    assert_eq!(
        resolve_target(&others, None).map(|t| t.id.as_str()),
        Some("guard")
    );
    assert!(resolve_target(&[], None).is_none());
}

#[test]
fn elapsed_time_reads_as_minutes_and_seconds() {
    assert_eq!(format_elapsed(0), "0:00");
    assert_eq!(format_elapsed(7_400), "0:07");
    assert_eq!(format_elapsed(60_000), "1:00");
    assert_eq!(format_elapsed(125_000), "2:05");
}

fn view(state: State, has_text: bool, speech: &SpeechAvailability) -> ComposerView {
    let targets = vec![target("innkeeper")];
    ComposerView::of(&ComposerInputs {
        state,
        has_text,
        blocked: false,
        speech,
        targets: &targets,
        chosen: None,
        elapsed_ms: 7_000,
        level_permille: 300,
        note: None,
    })
}

#[test]
fn the_microphone_is_enabled_only_when_the_host_says_speech_is_set_up() {
    assert_eq!(
        view(State::Idle, false, &SpeechAvailability::Ready).mic,
        MicLook::Ready
    );
    for speech in [
        SpeechAvailability::Checking,
        SpeechAvailability::HostDown,
        SpeechAvailability::NotConfigured {
            missing: Some(MissingSetting::SpeechKey),
        },
    ] {
        let MicLook::Unavailable { reason } = view(State::Idle, false, &speech).mic else {
            panic!("the microphone should be off for {speech:?}");
        };
        assert!(!reason.is_empty());
    }
}

#[test]
fn a_missing_setting_is_named_in_the_reason_the_microphone_is_off() {
    let speech = SpeechAvailability::NotConfigured {
        missing: Some(MissingSetting::SpeechKey),
    };
    let MicLook::Unavailable { reason } = view(State::Idle, false, &speech).mic else {
        panic!("the microphone should be off");
    };
    assert!(
        reason.contains(MissingSetting::SpeechKey.variable()),
        "{reason}"
    );
}

#[test]
fn a_recording_shows_its_time_level_and_a_cancel_and_nothing_else_does() {
    let recording = view(State::Recording, false, &SpeechAvailability::Ready);
    assert_eq!(recording.mic, MicLook::Recording);
    assert_eq!(recording.live.as_deref(), Some("0:07"));
    assert_eq!(recording.level_permille, 300);
    assert!(!recording.send_enabled);
    let idle = view(State::Idle, false, &SpeechAvailability::Ready);
    assert_eq!(idle.live, None);
    assert_eq!(idle.level_permille, 0);
}

#[test]
fn send_is_enabled_only_for_a_draft_with_text_and_a_target() {
    assert!(view(State::Draft, true, &SpeechAvailability::Ready).send_enabled);
    assert!(!view(State::Idle, false, &SpeechAvailability::Ready).send_enabled);
    assert!(!view(State::Sending, true, &SpeechAvailability::Ready).send_enabled);
    assert!(!view(State::Transcribing, true, &SpeechAvailability::Ready).send_enabled);
    let no_one = ComposerView::of(&ComposerInputs {
        state: State::Draft,
        has_text: true,
        blocked: false,
        speech: &SpeechAvailability::Ready,
        targets: &[],
        chosen: None,
        elapsed_ms: 0,
        level_permille: 0,
        note: None,
    });
    assert!(!no_one.send_enabled);
    assert!(matches!(no_one.mic, MicLook::Unavailable { .. }));
}

#[test]
fn a_question_in_flight_shows_a_pending_send_and_locks_the_box() {
    let sending = view(State::Sending, true, &SpeechAvailability::Ready);
    assert_eq!(sending.send_label, "Sending...");
    assert!(!sending.field_enabled);
    assert_eq!(sending.mic, MicLook::Busy);
    let transcribing = view(State::Transcribing, false, &SpeechAvailability::Ready);
    assert!(!transcribing.field_enabled);
    assert_eq!(transcribing.field_placeholder, "Writing that down...");
    assert_eq!(transcribing.live.as_deref(), Some("Writing that down..."));
}

#[test]
fn a_blocked_phone_turns_everything_off() {
    let targets = vec![target("innkeeper")];
    let blocked = ComposerView::of(&ComposerInputs {
        state: State::Draft,
        has_text: true,
        blocked: true,
        speech: &SpeechAvailability::Ready,
        targets: &targets,
        chosen: None,
        elapsed_ms: 0,
        level_permille: 0,
        note: None,
    });
    assert!(!blocked.field_enabled);
    assert!(!blocked.send_enabled);
    assert!(matches!(blocked.mic, MicLook::Unavailable { .. }));
}

#[test]
fn the_note_says_why_in_a_short_sentence() {
    let permission = Note::Capture(MicrophoneError::PermissionDenied).text();
    assert!(permission.contains("blocked"), "{permission}");
    assert_ne!(
        permission,
        Note::Capture(MicrophoneError::NoDevice).text(),
        "each microphone failure has its own sentence"
    );
    assert!(Note::ClippedAtLongest.text().contains("60"));
    assert_eq!(
        Note::AskRefused("the innkeeper is busy".to_owned()).text(),
        "the innkeeper is busy"
    );
    let targets = vec![target("innkeeper")];
    let note = Note::Transcribe(TranscribeFailure::Host);
    let shown = ComposerView::of(&ComposerInputs {
        state: State::Idle,
        has_text: false,
        blocked: false,
        speech: &SpeechAvailability::Ready,
        targets: &targets,
        chosen: None,
        elapsed_ms: 0,
        level_permille: 0,
        note: Some(&note),
    });
    assert_eq!(shown.note, Some(note.text()));
}

#[test]
fn a_host_refusal_kind_maps_to_a_transcribe_failure() {
    let failure = TranscribeFailure::of_kind;
    assert_eq!(
        failure(Some(SpeechErrorKind::NotConfigured)),
        TranscribeFailure::NotConfigured
    );
    assert_eq!(
        failure(Some(SpeechErrorKind::AudioTooShort)),
        TranscribeFailure::TooShort
    );
    assert_eq!(
        failure(Some(SpeechErrorKind::AudioEmpty)),
        TranscribeFailure::TooShort
    );
    assert_eq!(
        failure(Some(SpeechErrorKind::Stopped)),
        TranscribeFailure::Host
    );
    assert_eq!(failure(None), TranscribeFailure::Host);
}
