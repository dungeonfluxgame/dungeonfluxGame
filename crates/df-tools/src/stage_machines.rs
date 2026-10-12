//! The display's presentation state machines: closed (state, event) tables and the bounds
//! every waiting state obeys.
//!
//! Nothing on the shared display appears, changes or disappears except through one of these
//! machines. Each machine is a pure function from (state, event) to the next state; an event
//! that is not legal in a state yields `None` and is ignored (never a panic, never a change).
//! `stage_conductor` owns the machines and drives them from view arrival, voice events and
//! timers; this file is only the vocabulary, the tables and the named bounds, so the whole
//! timing policy is read in one place. All durations are milliseconds.
//!
//! Display presentation only: no machine decides an outcome, sends a request or delays what a
//! phone may do.

// ---------------------------------------------------------------------------------------------
// Bounds. Every waiting state has a watchdog built from these; a missed media event takes the
// normal exit path when its bound passes.

/// A line's bubble or narrator box animates in over this long (Opening).
pub(crate) const LINE_OPEN_MS: u32 = 250;
/// A line stays after its speech ends before the next line may open (Lingering).
pub(crate) const LINE_LINGER_MS: u32 = 700;
/// A line's bubble or box animates out over this long (Closing).
pub(crate) const LINE_CLOSE_MS: u32 = 400;
/// The last line of a scene stays on screen while the table decides; it still closes after
/// this long so a forgotten room does not keep a bubble for ever.
pub(crate) const FINAL_LINGER_CAP_MS: u32 = 45_000;
/// A requested clip that is not ready by now has failed (the line then runs on reading time).
pub(crate) const CLIP_READY_WAIT_MS: u32 = 5_000;
/// A ready clip told to play that has not started by now has failed (a blocked play).
pub(crate) const CLIP_START_WAIT_MS: u32 = 2_500;
/// Past the clip's own length (or, unknown, the line's reading time) a missing end signal
/// ends the clip.
pub(crate) const CLIP_END_GRACE_MS: u32 = 3_000;
/// Reading time of a line without sound: `READ_BASE_MS + READ_PER_CHARACTER_MS` per character,
/// kept between the two bounds, so pacing without audio matches pacing with it.
pub(crate) const READ_BASE_MS: u32 = 1_800;
pub(crate) const READ_PER_CHARACTER_MS: u32 = 35;
pub(crate) const READ_MINIMUM_MS: u32 = 2_500;
pub(crate) const READ_MAXIMUM_MS: u32 = 7_000;
/// The longest a waiting successor scene waits for the lines of the scene on screen (a scene is
/// a narration and a character line, each up to a clip long); then the line in progress is
/// closed so the display catches up.
pub(crate) const SCENE_WAIT_CAP_MS: u64 = 30_000;
/// A line skipped by catch-up is shown as text this long.
pub(crate) const CATCH_UP_TEXT_MS: u32 = 700;
/// A fight that starts while a line plays fades that line within this bound; the fight's
/// entrance then runs, so dice outcomes are never held behind story speech.
pub(crate) const FIGHT_LINE_FADE_MS: u32 = 500;
/// An actor walking on, and off.
pub(crate) const ACTOR_ENTER_MS: u32 = 800;
pub(crate) const ACTOR_EXIT_MS: u32 = 350;
/// The scene transition's own bounds: leaving, and the wordmark, title card and entrance.
pub(crate) const SCENE_EXIT_CAP_MS: u64 = 4_000;
pub(crate) const SCENE_ENTER_CAP_MS: u64 = 8_000;

/// The time a line is read in when no sound plays.
pub(crate) fn reading_time_ms(text: &str) -> u32 {
    let characters = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    READ_BASE_MS
        .saturating_add(READ_PER_CHARACTER_MS.saturating_mul(characters))
        .clamp(READ_MINIMUM_MS, READ_MAXIMUM_MS)
}

// ---------------------------------------------------------------------------------------------
// Scene

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SceneState {
    Hidden,
    /// Transition and title card, backdrop fade-in.
    Entering,
    /// The scene's lines, strictly in order.
    Presenting,
    /// Every line has been spoken; the choice chips are visible only now.
    AwaitingChoice,
    /// Bubbles and chips cleared, fade-out; the successor's Entering follows.
    Exiting,
}

impl SceneState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Entering => "entering",
            Self::Presenting => "presenting",
            Self::AwaitingChoice => "awaiting-choice",
            Self::Exiting => "exiting",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SceneEvent {
    /// The scene is shown at once without a transition (first view, reload, reconnect).
    Restore,
    EntranceDone,
    LinesDone,
    /// A line arrived after every line was done (a reply to a typed line).
    LinesResumed,
    ExitBegin,
    ExitDone,
}

pub(crate) fn scene_next(state: SceneState, event: SceneEvent) -> Option<SceneState> {
    use SceneEvent::*;
    use SceneState::*;
    match (state, event) {
        (_, Restore) => Some(Presenting),
        (Entering, EntranceDone) => Some(Presenting),
        (Presenting, LinesDone) => Some(AwaitingChoice),
        (AwaitingChoice, LinesResumed) => Some(Presenting),
        (Presenting | AwaitingChoice, ExitBegin) => Some(Exiting),
        (Exiting, ExitDone) => Some(Entering),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Line (a bubble or the narrator box)

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LineState {
    Queued,
    /// The bubble or box animates in as its clip starts.
    Opening,
    Speaking,
    /// The speech ended; the line stays a moment (or until the table decides).
    Lingering,
    Closing,
    Done,
}

impl LineState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Opening => "opening",
            Self::Speaking => "speaking",
            Self::Lingering => "lingering",
            Self::Closing => "closing",
            Self::Done => "done",
        }
    }

    /// The states in which a line is the one being heard or read: at most one line at a time.
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Opening | Self::Speaking | Self::Lingering)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LineEvent {
    /// The line's turn: its clip started, or it has no sound to wait for.
    Begin,
    OpenDone,
    /// The clip ended, failed or was skipped, or the reading time passed.
    SpeechDone,
    LingerDone,
    /// Forced: the scene exits, a fight begins, catch-up or a newer line replaces it.
    Close,
    CloseDone,
}

pub(crate) fn line_next(state: LineState, event: LineEvent) -> Option<LineState> {
    use LineEvent::*;
    use LineState::*;
    match (state, event) {
        (Queued, Begin) => Some(Opening),
        (Queued, Close) => Some(Done),
        (Opening, OpenDone) => Some(Speaking),
        (Opening | Speaking, SpeechDone) => Some(Lingering),
        (Lingering, LingerDone) => Some(Closing),
        (Opening | Speaking | Lingering, Close) => Some(Closing),
        (Closing, CloseDone) => Some(Done),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Voice clip

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClipState {
    /// No clip is expected (silent line, or sound locked or muted).
    Silent,
    Requested,
    Ready,
    Playing,
    Ended,
    Failed,
    /// Cancelled on purpose: catch-up, a fight, a scene exit.
    Skipped,
}

impl ClipState {
    pub(crate) fn is_over(self) -> bool {
        matches!(self, Self::Ended | Self::Failed | Self::Skipped)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClipEvent {
    Request,
    /// The audio bytes are decoded and can play.
    Loaded,
    /// The browser began playing it.
    Started,
    Finished,
    /// Fetch, decode or play refused.
    Error,
    Cancel,
    /// A watchdog bound passed.
    WaitExpired,
}

pub(crate) fn clip_next(state: ClipState, event: ClipEvent) -> Option<ClipState> {
    use ClipEvent::*;
    use ClipState::*;
    match (state, event) {
        (Silent, Request) => Some(Requested),
        (Requested, Loaded) => Some(Ready),
        (Requested | Ready, Started) => Some(Playing),
        (Playing, Finished) => Some(Ended),
        (Requested | Ready | Playing, Error) => Some(Failed),
        (Requested | Ready, WaitExpired) => Some(Failed),
        (Playing, WaitExpired) => Some(Ended),
        (Silent | Requested | Ready | Playing, Cancel) => Some(Skipped),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Actor (a hero or non-player figure)

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActorState {
    Offstage,
    Entering,
    Idle,
    /// Lit, with the line's bubble attached.
    Speaking,
    Exiting,
}

impl ActorState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Offstage => "offstage",
            Self::Entering => "entering",
            Self::Idle => "idle",
            Self::Speaking => "speaking",
            Self::Exiting => "exiting",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActorEvent {
    /// The actor is in the scene being shown.
    Cast,
    /// The actor is in the scene shown at once (no entrance).
    Restore,
    EnterDone,
    LineOpened,
    LineClosed,
    /// The actor is absent from the next scene.
    Leave,
    ExitDone,
}

pub(crate) fn actor_next(state: ActorState, event: ActorEvent) -> Option<ActorState> {
    use ActorEvent::*;
    use ActorState::*;
    match (state, event) {
        (Offstage | Exiting, Cast) => Some(Entering),
        (Offstage | Entering | Idle | Speaking | Exiting, Restore) => Some(Idle),
        (Entering, EnterDone) => Some(Idle),
        (Entering | Idle, LineOpened) => Some(Speaking),
        (Speaking, LineClosed) => Some(Idle),
        (Entering | Idle | Speaking, Leave) => Some(Exiting),
        (Exiting, ExitDone) => Some(Offstage),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE_STATES: [SceneState; 5] = [
        SceneState::Hidden,
        SceneState::Entering,
        SceneState::Presenting,
        SceneState::AwaitingChoice,
        SceneState::Exiting,
    ];
    const SCENE_EVENTS: [SceneEvent; 6] = [
        SceneEvent::Restore,
        SceneEvent::EntranceDone,
        SceneEvent::LinesDone,
        SceneEvent::LinesResumed,
        SceneEvent::ExitBegin,
        SceneEvent::ExitDone,
    ];
    const LINE_STATES: [LineState; 6] = [
        LineState::Queued,
        LineState::Opening,
        LineState::Speaking,
        LineState::Lingering,
        LineState::Closing,
        LineState::Done,
    ];
    const LINE_EVENTS: [LineEvent; 6] = [
        LineEvent::Begin,
        LineEvent::OpenDone,
        LineEvent::SpeechDone,
        LineEvent::LingerDone,
        LineEvent::Close,
        LineEvent::CloseDone,
    ];
    const CLIP_STATES: [ClipState; 7] = [
        ClipState::Silent,
        ClipState::Requested,
        ClipState::Ready,
        ClipState::Playing,
        ClipState::Ended,
        ClipState::Failed,
        ClipState::Skipped,
    ];
    const CLIP_EVENTS: [ClipEvent; 7] = [
        ClipEvent::Request,
        ClipEvent::Loaded,
        ClipEvent::Started,
        ClipEvent::Finished,
        ClipEvent::Error,
        ClipEvent::Cancel,
        ClipEvent::WaitExpired,
    ];
    const ACTOR_STATES: [ActorState; 5] = [
        ActorState::Offstage,
        ActorState::Entering,
        ActorState::Idle,
        ActorState::Speaking,
        ActorState::Exiting,
    ];
    const ACTOR_EVENTS: [ActorEvent; 7] = [
        ActorEvent::Cast,
        ActorEvent::Restore,
        ActorEvent::EnterDone,
        ActorEvent::LineOpened,
        ActorEvent::LineClosed,
        ActorEvent::Leave,
        ActorEvent::ExitDone,
    ];

    #[test]
    fn a_scene_walks_its_whole_cycle_in_order() {
        let mut state = SceneState::Hidden;
        for (event, expected) in [
            (SceneEvent::Restore, SceneState::Presenting),
            (SceneEvent::LinesDone, SceneState::AwaitingChoice),
            (SceneEvent::ExitBegin, SceneState::Exiting),
            (SceneEvent::ExitDone, SceneState::Entering),
            (SceneEvent::EntranceDone, SceneState::Presenting),
        ] {
            state = scene_next(state, event).expect("legal");
            assert_eq!(state, expected);
        }
    }

    #[test]
    fn a_scene_never_skips_its_entrance_or_exit() {
        // A successor never starts its entrance before the old scene has exited, and a
        // scene is never left while it is still entering.
        assert_eq!(
            scene_next(SceneState::Presenting, SceneEvent::ExitDone),
            None
        );
        assert_eq!(
            scene_next(SceneState::Entering, SceneEvent::ExitBegin),
            None
        );
        assert_eq!(scene_next(SceneState::Hidden, SceneEvent::ExitBegin), None);
        assert_eq!(scene_next(SceneState::Exiting, SceneEvent::LinesDone), None);
        assert_eq!(scene_next(SceneState::Hidden, SceneEvent::LinesDone), None);
    }

    #[test]
    fn chips_follow_awaiting_choice_and_a_late_line_returns_to_presenting() {
        assert_eq!(
            scene_next(SceneState::AwaitingChoice, SceneEvent::LinesResumed),
            Some(SceneState::Presenting)
        );
        assert_eq!(
            scene_next(SceneState::Presenting, SceneEvent::LinesResumed),
            None
        );
    }

    #[test]
    fn a_line_walks_queued_to_done_in_order() {
        let mut state = LineState::Queued;
        for (event, expected) in [
            (LineEvent::Begin, LineState::Opening),
            (LineEvent::OpenDone, LineState::Speaking),
            (LineEvent::SpeechDone, LineState::Lingering),
            (LineEvent::LingerDone, LineState::Closing),
            (LineEvent::CloseDone, LineState::Done),
        ] {
            state = line_next(state, event).expect("legal");
            assert_eq!(state, expected);
        }
    }

    #[test]
    fn a_queued_line_that_is_closed_was_never_shown() {
        assert_eq!(
            line_next(LineState::Queued, LineEvent::Close),
            Some(LineState::Done)
        );
    }

    #[test]
    fn a_forced_close_always_goes_through_closing() {
        for state in [
            LineState::Opening,
            LineState::Speaking,
            LineState::Lingering,
        ] {
            assert_eq!(line_next(state, LineEvent::Close), Some(LineState::Closing));
        }
    }

    #[test]
    fn a_clip_that_fails_or_is_blocked_ends_without_a_stuck_state() {
        assert_eq!(
            clip_next(ClipState::Requested, ClipEvent::WaitExpired),
            Some(ClipState::Failed)
        );
        assert_eq!(
            clip_next(ClipState::Ready, ClipEvent::WaitExpired),
            Some(ClipState::Failed)
        );
        assert_eq!(
            clip_next(ClipState::Playing, ClipEvent::WaitExpired),
            Some(ClipState::Ended)
        );
        assert_eq!(
            clip_next(ClipState::Ready, ClipEvent::Error),
            Some(ClipState::Failed)
        );
        assert_eq!(
            clip_next(ClipState::Playing, ClipEvent::Cancel),
            Some(ClipState::Skipped)
        );
    }

    #[test]
    fn a_clip_walks_requested_to_ended() {
        let mut state = ClipState::Silent;
        for (event, expected) in [
            (ClipEvent::Request, ClipState::Requested),
            (ClipEvent::Loaded, ClipState::Ready),
            (ClipEvent::Started, ClipState::Playing),
            (ClipEvent::Finished, ClipState::Ended),
        ] {
            state = clip_next(state, event).expect("legal");
            assert_eq!(state, expected);
        }
    }

    #[test]
    fn a_finished_clip_never_restarts() {
        for over in [ClipState::Ended, ClipState::Failed, ClipState::Skipped] {
            assert!(over.is_over());
            for event in CLIP_EVENTS {
                assert_eq!(clip_next(over, event), None, "{over:?} {event:?}");
            }
        }
    }

    #[test]
    fn an_actor_walks_on_speaks_and_walks_off() {
        let mut state = ActorState::Offstage;
        for (event, expected) in [
            (ActorEvent::Cast, ActorState::Entering),
            (ActorEvent::EnterDone, ActorState::Idle),
            (ActorEvent::LineOpened, ActorState::Speaking),
            (ActorEvent::LineClosed, ActorState::Idle),
            (ActorEvent::Leave, ActorState::Exiting),
            (ActorEvent::ExitDone, ActorState::Offstage),
        ] {
            state = actor_next(state, event).expect("legal");
            assert_eq!(state, expected);
        }
    }

    #[test]
    fn an_absent_actor_cannot_speak_or_leave() {
        assert_eq!(
            actor_next(ActorState::Offstage, ActorEvent::LineOpened),
            None
        );
        assert_eq!(actor_next(ActorState::Offstage, ActorEvent::Leave), None);
        assert_eq!(
            actor_next(ActorState::Exiting, ActorEvent::LineOpened),
            None
        );
    }

    /// Every pair of every table is answered without a panic, and a state's answer is the
    /// same however often it is asked (the tables are pure).
    #[test]
    fn every_pair_of_every_table_is_answered_deterministically() {
        for state in SCENE_STATES {
            for event in SCENE_EVENTS {
                assert_eq!(scene_next(state, event), scene_next(state, event));
            }
        }
        for state in LINE_STATES {
            for event in LINE_EVENTS {
                assert_eq!(line_next(state, event), line_next(state, event));
            }
        }
        for state in CLIP_STATES {
            for event in CLIP_EVENTS {
                assert_eq!(clip_next(state, event), clip_next(state, event));
            }
        }
        for state in ACTOR_STATES {
            for event in ACTOR_EVENTS {
                assert_eq!(actor_next(state, event), actor_next(state, event));
            }
        }
    }

    #[test]
    fn every_state_has_a_distinct_name_for_the_dom() {
        let scene: Vec<&str> = SCENE_STATES.iter().map(|state| state.name()).collect();
        let line: Vec<&str> = LINE_STATES.iter().map(|state| state.name()).collect();
        let actor: Vec<&str> = ACTOR_STATES.iter().map(|state| state.name()).collect();
        for names in [scene, line, actor] {
            let mut sorted = names.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), names.len(), "{names:?}");
        }
    }

    #[test]
    fn a_finished_line_ignores_every_event() {
        for event in LINE_EVENTS {
            assert_eq!(line_next(LineState::Done, event), None);
        }
    }

    #[test]
    fn reading_time_is_bounded_and_grows_with_length() {
        assert_eq!(reading_time_ms(""), READ_MINIMUM_MS);
        assert!(reading_time_ms(&"word ".repeat(40)) > READ_MINIMUM_MS);
        assert_eq!(reading_time_ms(&"word ".repeat(400)), READ_MAXIMUM_MS);
    }
}
