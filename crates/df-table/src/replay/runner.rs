//! Records a script against a scripted world and replays a recording against a fresh one (H.17).
//!
//! It owns `record` (runs a script of inputs and returns the `Recording`: the session and the
//! words of its typed lines, which never enter the session), `Replay` (runs a recording against a
//! world the caller builds again and compares every record) and `golden_script` (turns the inputs
//! of a golden script into a script to record). The world is the golden runner's `World`: the
//! scripted fakes of every port and the scene machine, so what is checked is that the same inputs
//! meet the same answers and reach the same states, hashes and records.
//!
//! `Replay::run` stops at the first divergence with a typed error, which is how a tampered turn is
//! refused; `Replay::diff` goes on and lists every divergence, which is `replay --diff`; `until`
//! stops after a turn number, which is `replay --until`. A world that does not start at the state
//! hash the recording started at is refused before any input runs.
//!
//! It runs inputs through `World::run_with` with the recorded correlation, and does what a host
//! does after a commit: it releases the turn, issues its requests and runs its narration. It never
//! draws a die or reads a clock itself: the world's fakes do.

use std::collections::BTreeMap;
use std::fmt;

use crate::element::NameError;
use crate::envelope::{
    EngineInput, InputEnvelope, InputOrigin, PlayerInput, ProposalOrigin, ProposerRef, TimerName,
    UtteranceId, ViewStamp,
};
use crate::golden::{GoldenInput, GoldenStep, World, WorldError};
use crate::identity::{OperationId, SeatId, TableId, TurnSeq};
use crate::listed::listed_enum;
use crate::ports::ActionId;
use crate::private_data::PlayerText;
use crate::replay::{LineTextError, RecordedSession, RecordedTurn, SessionError, SessionRecorder};
use crate::testkit::correlation;

/// The words of the typed lines of a session, by the operation that carried them. They are kept
/// beside a recording and never inside it (RES36).
#[derive(Debug, Clone, Default)]
pub struct LineTexts {
    texts: BTreeMap<OperationId, PlayerText>,
}

impl LineTexts {
    pub fn new() -> Self {
        Self::default()
    }

    /// The words of the line the operation carried.
    pub fn with(mut self, operation_id: OperationId, text: &str) -> Self {
        self.texts.insert(operation_id, PlayerText::new(text));
        self
    }

    pub fn get(&self, operation_id: OperationId) -> Option<&PlayerText> {
        self.texts.get(&operation_id)
    }

    fn insert(&mut self, operation_id: OperationId, text: PlayerText) {
        self.texts.insert(operation_id, text);
    }
}

/// A session and the words of its typed lines.
#[derive(Debug, Clone)]
pub struct Recording {
    pub session: RecordedSession,
    pub lines: LineTexts,
}

listed_enum! {
    /// A part of a recorded turn that a replay can find different.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum TurnField {
        Correlation,
        Outcome,
        Declined,
        Pending,
        Draws,
        Effects,
        StateHashBefore,
        StateHashAfter,
        StateHashSettled,
        Replayed,
    }
}

/// A turn that replayed differently from how it was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// The place of the turn in the session, counting from zero.
    pub turn: usize,
    pub fields: Vec<TurnField>,
}

/// Why a script could not be recorded or a recording could not be replayed.
#[derive(Debug)]
pub enum ReplayError {
    /// The recording was refused, or a record disagreed with its input.
    Refused(SessionError),
    /// The scripted world could not be built or run.
    World(WorldError),
    /// The world runs another table than the one the recording is of.
    OtherTable { recorded: TableId, world: TableId },
    /// The world starts at another state hash than the recording did.
    StartsElsewhere { expected: u64, found: u64 },
    /// The words for a recorded line were missing or are not the recorded length.
    Line { turn: usize, error: LineTextError },
    /// A turn replayed differently (`Replay::run` stops at the first).
    Diverged(Divergence),
    /// A name in a golden script is not a valid name.
    BadName(NameError),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => fmt::Display::fmt(error, formatter),
            Self::World(error) => fmt::Display::fmt(error, formatter),
            Self::OtherTable { recorded, world } => write!(
                formatter,
                "the world runs table {world} and the recording is of table {recorded}"
            ),
            Self::StartsElsewhere { expected, found } => write!(
                formatter,
                "the world starts at state hash {found} and the recording at {expected}"
            ),
            Self::Line { turn, error } => write!(formatter, "turn {turn}: {error}"),
            Self::Diverged(divergence) => write!(
                formatter,
                "turn {} replayed differently in {:?}",
                divergence.turn, divergence.fields
            ),
            Self::BadName(error) => write!(formatter, "a name in the script is invalid: {error}"),
        }
    }
}

impl std::error::Error for ReplayError {}

impl From<SessionError> for ReplayError {
    fn from(error: SessionError) -> Self {
        Self::Refused(error)
    }
}

impl From<WorldError> for ReplayError {
    fn from(error: WorldError) -> Self {
        Self::World(error)
    }
}

fn text_of(origin: &InputOrigin) -> Option<&PlayerText> {
    match origin {
        InputOrigin::Player { input, .. } => match input {
            PlayerInput::SubmitText { text, .. }
            | PlayerInput::Say { text, .. }
            | PlayerInput::PrivateMessage { text, .. } => Some(text),
            PlayerInput::Tap { .. }
            | PlayerInput::SafetyCard
            | PlayerInput::PendingAnswer { .. } => None,
        },
        InputOrigin::Engine(_) => None,
    }
}

/// Runs `script` in `world`, one input after another, as a host would, and records every turn.
pub fn record(world: &mut World, script: &[InputOrigin]) -> Result<Recording, ReplayError> {
    let mut recorder = SessionRecorder::begin(world.table.table_id(), world.table.state_hash());
    let mut lines = LineTexts::new();
    for origin in script {
        let probe = correlation(world.next_turn());
        let view = ViewStamp::new(probe.revision);
        let envelope = match origin {
            InputOrigin::Player { seat, input } => {
                InputEnvelope::from_player(probe.operation_id, *seat, view, input.clone())
            }
            InputOrigin::Engine(input) => {
                InputEnvelope::from_engine(probe.operation_id, view, input.clone())
            }
        };
        if let Some(text) = text_of(origin) {
            lines.insert(probe.operation_id, text.clone());
        }
        let result = world.run_envelope(&envelope)?;
        world.narrate(&result)?;
        recorder.record(&envelope, &result.record, world.table.state_hash())?;
    }
    Ok(Recording {
        session: recorder.finish(),
        lines,
    })
}

/// The script of a golden session's inputs, numbered as a fresh world numbers them.
pub fn golden_script(steps: &[GoldenStep]) -> Result<Vec<InputOrigin>, ReplayError> {
    let name = |text: &str| ActionId::new(text).map_err(ReplayError::BadName);
    steps
        .iter()
        .zip(1_u64..)
        .map(|(step, turn)| match &step.input {
            GoldenInput::Tap { seat, action } => Ok(InputOrigin::Player {
                seat: SeatId::new(*seat),
                input: PlayerInput::Tap {
                    action: name(action)?,
                },
            }),
            GoldenInput::Text { seat, line } => Ok(InputOrigin::Player {
                seat: SeatId::new(*seat),
                input: PlayerInput::SubmitText {
                    utterance: UtteranceId::new(turn),
                    text: PlayerText::new(*line),
                },
            }),
            GoldenInput::Answer {
                seat,
                pending,
                choice,
            } => Ok(InputOrigin::Player {
                seat: SeatId::new(*seat),
                input: PlayerInput::PendingAnswer {
                    pending: crate::envelope::PendingId::new(*pending),
                    choice: name(choice)?,
                },
            }),
            GoldenInput::Timer(timer) => Ok(InputOrigin::Engine(EngineInput::Timer {
                timer: TimerName::new(timer).map_err(ReplayError::BadName)?,
            })),
            GoldenInput::MachineProposal => Ok(InputOrigin::Engine(EngineInput::Proposal {
                origin: ProposalOrigin::Machine,
                proposer: ProposerRef::new("scene").map_err(ReplayError::BadName)?,
            })),
        })
        .collect()
}

/// What a replay did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplayReport {
    pub turns_replayed: usize,
    /// Every turn that differed; `Replay::run` never returns a report with one.
    pub divergences: Vec<Divergence>,
    /// The state hash the replayed world settled at after the last turn it ran.
    pub final_state_hash: u64,
}

fn differences(recorded: &RecordedTurn, replayed: &RecordedTurn) -> Vec<TurnField> {
    let checks = [
        (
            TurnField::Correlation,
            recorded.correlation == replayed.correlation,
        ),
        (TurnField::Outcome, recorded.outcome == replayed.outcome),
        (TurnField::Declined, recorded.declined == replayed.declined),
        (TurnField::Pending, recorded.pending == replayed.pending),
        (TurnField::Draws, recorded.draws == replayed.draws),
        (TurnField::Effects, recorded.effects == replayed.effects),
        (
            TurnField::StateHashBefore,
            recorded.state_hash_before == replayed.state_hash_before,
        ),
        (
            TurnField::StateHashAfter,
            recorded.state_hash_after == replayed.state_hash_after,
        ),
        (
            TurnField::StateHashSettled,
            recorded.state_hash_settled == replayed.state_hash_settled,
        ),
        (TurnField::Replayed, recorded.replayed == replayed.replayed),
    ];
    checks
        .into_iter()
        .filter(|(_, same)| !same)
        .map(|(field, _)| field)
        .collect()
}

/// A recording to replay, the words of its typed lines and where to stop.
#[derive(Clone, Copy)]
pub struct Replay<'a> {
    session: &'a RecordedSession,
    lines: Option<&'a LineTexts>,
    until: Option<TurnSeq>,
}

impl<'a> Replay<'a> {
    pub fn of(session: &'a RecordedSession) -> Self {
        Self {
            session,
            lines: None,
            until: None,
        }
    }

    /// The words of the typed lines, which a recording does not hold.
    pub fn with_lines(mut self, lines: &'a LineTexts) -> Self {
        self.lines = Some(lines);
        self
    }

    /// Stops after the last turn numbered at most `last`.
    pub fn until(mut self, last: TurnSeq) -> Self {
        self.until = Some(last);
        self
    }

    /// Replays against the world `make` builds, and stops at the first turn that differs.
    pub fn run(
        &self,
        make: impl FnOnce() -> Result<World, WorldError>,
    ) -> Result<ReplayReport, ReplayError> {
        self.play(make, true)
    }

    /// Replays against the world `make` builds and lists every turn that differs.
    pub fn diff(
        &self,
        make: impl FnOnce() -> Result<World, WorldError>,
    ) -> Result<ReplayReport, ReplayError> {
        self.play(make, false)
    }

    fn play(
        &self,
        make: impl FnOnce() -> Result<World, WorldError>,
        stop_at_first: bool,
    ) -> Result<ReplayReport, ReplayError> {
        self.session.verify()?;
        let mut world = make()?;
        if world.table.table_id() != self.session.table() {
            return Err(ReplayError::OtherTable {
                recorded: self.session.table(),
                world: world.table.table_id(),
            });
        }
        let found = world.table.state_hash();
        let expected = self.session.initial_state_hash();
        if found != expected {
            return Err(ReplayError::StartsElsewhere { expected, found });
        }
        let mut report = ReplayReport {
            final_state_hash: found,
            ..ReplayReport::default()
        };
        for (index, recorded) in self.session.turns().iter().enumerate() {
            if self
                .until
                .is_some_and(|last| recorded.correlation.turn_seq > last)
            {
                break;
            }
            let operation_id = recorded.correlation.operation_id;
            let text = self.lines.and_then(|lines| lines.get(operation_id));
            let envelope = recorded
                .input
                .into_envelope(
                    operation_id,
                    ViewStamp::new(recorded.correlation.revision),
                    text,
                )
                .map_err(|error| ReplayError::Line { turn: index, error })?;
            let envelope = match recorded.evidence {
                Some(evidence) => envelope.with_evidence(evidence),
                None => envelope,
            };
            let result = world.run_with(&envelope, recorded.correlation)?;
            world.narrate(&result)?;
            let settled = world.table.state_hash();
            let replayed = RecordedTurn::of(index, &envelope, &result.record, settled)?;
            report.turns_replayed += 1;
            report.final_state_hash = settled;
            let fields = differences(recorded, &replayed);
            if fields.is_empty() {
                continue;
            }
            let divergence = Divergence {
                turn: index,
                fields,
            };
            if stop_at_first {
                return Err(ReplayError::Diverged(divergence));
            }
            report.divergences.push(divergence);
        }
        Ok(report)
    }
}
