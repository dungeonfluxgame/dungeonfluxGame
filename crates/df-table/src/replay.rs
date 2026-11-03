//! Recorded sessions and the replay of them: a diagnostic and test tool, never the way to recover
//! a table (O.6, H.17, RES19).
//!
//! It owns the recording format (`RecordedSession`, `RecordedTurn`, `RecordedInput`), the
//! `SessionRecorder` a host feeds with every committed input, and, with the `testkit` fakes, the
//! runner that records a script against a scripted world (`record`) and replays a recording
//! against a fresh one (`Replay`). A recording is the table at the start, then one `RecordedTurn`
//! per committed input: the input (its typed text kept as a length class only, RES36), its
//! correlation, how it ended, the dice drawn, the effects committed and three state hashes: before
//! the input, after it and once settled (after the host's commit stamp, released events, issued
//! requests and narration). The recording is versioned and serialisable (`to_json`, `from_json`).
//!
//! A recording is refused, with a typed `SessionError`, when its schema version is not the one
//! this engine writes, when its state hash chain is broken (a turn that does not start from the
//! hash the one before it settled at, or the first from the hash the table started at), when its
//! turns are out of order or when it cannot be read. A replay that passes those checks runs every
//! recorded input against the scripted ports of a fresh world and compares everything it
//! recorded; any difference is a divergence, and a tampered turn shows as one. Replay never draws
//! a die of its own and never calls a port a script does not cover: it is the scripted ports'
//! answers that are checked against the recording, not the recording that is trusted.
//!
//! Replay is supported only where the table started in the state the recording started in and the
//! reducers are the ones that wrote it: a different start is `StartsElsewhere`, and the state hash
//! is checked at every step (RES19). It reads no clock, draws no die and touches no file: the
//! host reads and writes the text. A recording holds the committed effects, so it is operator data.

mod input;
#[cfg(any(test, feature = "testkit"))]
mod runner;

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::effects::TableEffect;
use crate::envelope::{InputEnvelope, PendingId};
use crate::identity::{Correlation, EvidenceRef, TableId};
use crate::machine_slot::ErrorClass;
use crate::outcome::{DeclineCode, OutcomeKind};
use crate::pending::StoredDraw;
use crate::pipeline::TurnRecord;

pub use input::{LineKind, LineTextError, RecordedInput, RecordedOrigin, RecordedReadiness};
#[cfg(any(test, feature = "testkit"))]
pub use runner::{
    Divergence, LineTexts, Recording, Replay, ReplayError, ReplayReport, TurnField, golden_script,
    record,
};

/// The schema version of `RecordedSession`. Raised when the saved shape changes; a recording of
/// another version is refused and never read wrongly (H.12).
pub const RECORDING_SCHEMA_VERSION: u32 = 1;

/// The most turns one recording holds. Deliberately huge so it never limits a session; scope it
/// down later (owner, 2026-11-02).
pub const MAXIMUM_RECORDED_TURNS: usize = 1_000_000;

/// Why a recording could not be made or was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    /// The recording is of a schema version this engine does not write.
    UnsupportedSchema {
        found: u64,
        expected: u32,
    },
    /// The text is not a recording, or one with a field of the wrong shape.
    Undecodable {
        class: ErrorClass,
    },
    TooManyTurns,
    /// Turn `turn` (counting from zero) does not start from the state hash the one before it
    /// settled at; for turn zero, the one the table started at.
    ChainBroken {
        turn: usize,
        expected: u64,
        found: u64,
    },
    /// Turn `turn` is not numbered after the one before it.
    TurnOutOfOrder {
        turn: usize,
    },
    /// The turn record says another kind of input or another sender than the input it was run for.
    InputDisagrees {
        turn: usize,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema { found, expected } => write!(
                formatter,
                "a recording at schema version {found} was refused; this engine reads {expected}"
            ),
            Self::Undecodable { class } => {
                write!(formatter, "the recording is not readable ({})", class.key())
            }
            Self::TooManyTurns => write!(
                formatter,
                "a recording holds at most {MAXIMUM_RECORDED_TURNS} turns"
            ),
            Self::ChainBroken {
                turn,
                expected,
                found,
            } => write!(
                formatter,
                "turn {turn} starts from state hash {found} and the chain says {expected}"
            ),
            Self::TurnOutOfOrder { turn } => {
                write!(
                    formatter,
                    "turn {turn} is not numbered after the one before"
                )
            }
            Self::InputDisagrees { turn } => write!(
                formatter,
                "the record of turn {turn} names another kind or sender than its input"
            ),
        }
    }
}

impl std::error::Error for SessionError {}

/// One committed input as a recording keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedTurn {
    pub input: RecordedInput,
    pub evidence: Option<EvidenceRef>,
    pub correlation: Correlation,
    pub outcome: OutcomeKind,
    pub declined: Option<DeclineCode>,
    pub pending: Option<PendingId>,
    /// Every die drawn for the input, in order.
    pub draws: Vec<StoredDraw>,
    /// The effects committed, in order.
    pub effects: Vec<TableEffect>,
    pub state_hash_before: u64,
    pub state_hash_after: u64,
    /// The state hash once the host finished with the turn: the commit stamp, the released events,
    /// the issued requests and the narration. The next turn starts from it.
    pub state_hash_settled: u64,
    /// The turn answered an operation that was committed before (`TurnRecord::replayed`).
    pub replayed: bool,
}

impl RecordedTurn {
    /// The turn `record` says the input `envelope` made, settled at `settled_state_hash`; `turn`
    /// is its place in the session, for the error. The record must agree with the input about its
    /// kind and its sender.
    pub fn of(
        turn: usize,
        envelope: &InputEnvelope,
        record: &TurnRecord,
        settled_state_hash: u64,
    ) -> Result<Self, SessionError> {
        let input = RecordedInput::of(envelope);
        if record.kind != input.kind() || record.sender != input.sender() {
            return Err(SessionError::InputDisagrees { turn });
        }
        Ok(Self {
            input,
            evidence: envelope.evidence(),
            correlation: record.correlation,
            outcome: record.outcome,
            declined: record.declined,
            pending: record.pending,
            draws: record.draws.clone(),
            effects: record.effects.clone(),
            state_hash_before: record.state_hash_before,
            state_hash_after: record.state_hash_after,
            state_hash_settled: settled_state_hash,
            replayed: record.replayed,
        })
    }
}

/// A recorded session: the table it ran on, the state it started in and every committed input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedSession {
    schema_version: u32,
    table: TableId,
    initial_state_hash: u64,
    turns: Vec<RecordedTurn>,
}

impl RecordedSession {
    pub fn table(&self) -> TableId {
        self.table
    }

    pub fn initial_state_hash(&self) -> u64 {
        self.initial_state_hash
    }

    pub fn turns(&self) -> &[RecordedTurn] {
        &self.turns
    }

    /// The state hash the session ends at: the last turn's settled hash, or the starting hash when
    /// there is no turn.
    pub fn final_state_hash(&self) -> u64 {
        self.turns
            .last()
            .map_or(self.initial_state_hash, |turn| turn.state_hash_settled)
    }

    /// Checks the schema version, the size, the numbering and the state hash chain.
    pub fn verify(&self) -> Result<(), SessionError> {
        if self.schema_version != RECORDING_SCHEMA_VERSION {
            return Err(SessionError::UnsupportedSchema {
                found: u64::from(self.schema_version),
                expected: RECORDING_SCHEMA_VERSION,
            });
        }
        if self.turns.len() > MAXIMUM_RECORDED_TURNS {
            return Err(SessionError::TooManyTurns);
        }
        let mut expected = self.initial_state_hash;
        let mut previous = None;
        for (index, turn) in self.turns.iter().enumerate() {
            if turn.state_hash_before != expected {
                return Err(SessionError::ChainBroken {
                    turn: index,
                    expected,
                    found: turn.state_hash_before,
                });
            }
            if previous.is_some_and(|last| turn.correlation.turn_seq <= last) {
                return Err(SessionError::TurnOutOfOrder { turn: index });
            }
            previous = Some(turn.correlation.turn_seq);
            expected = turn.state_hash_settled;
        }
        Ok(())
    }

    /// The recording as JSON text. The host writes it; the crate touches no file.
    pub fn to_json(&self) -> Result<String, SessionError> {
        serde_json::to_string(self).map_err(|error| SessionError::Undecodable {
            class: error.classify().into(),
        })
    }

    /// Reads a recording and refuses one that does not verify. The schema version is read before
    /// the rest, so a recording of another version is refused as that and not as unreadable.
    pub fn from_json(text: &str) -> Result<Self, SessionError> {
        let unreadable = |error: serde_json::Error| SessionError::Undecodable {
            class: error.classify().into(),
        };
        let value: Value = serde_json::from_str(text).map_err(unreadable)?;
        let found = value.get("schema_version").and_then(Value::as_u64).ok_or(
            SessionError::Undecodable {
                class: ErrorClass::Data,
            },
        )?;
        if found != u64::from(RECORDING_SCHEMA_VERSION) {
            return Err(SessionError::UnsupportedSchema {
                found,
                expected: RECORDING_SCHEMA_VERSION,
            });
        }
        let session: Self = serde_json::from_value(value).map_err(unreadable)?;
        session.verify()?;
        Ok(session)
    }
}

/// Builds a recording turn by turn, as a host does after each commit. Every turn is checked as it
/// is added, so a recording it finishes always verifies.
#[derive(Debug, Clone)]
pub struct SessionRecorder {
    session: RecordedSession,
    maximum_turns: usize,
}

impl SessionRecorder {
    /// Starts a recording of `table`, which stands at `initial_state_hash`.
    pub fn begin(table: TableId, initial_state_hash: u64) -> Self {
        Self {
            session: RecordedSession {
                schema_version: RECORDING_SCHEMA_VERSION,
                table,
                initial_state_hash,
                turns: Vec::new(),
            },
            maximum_turns: MAXIMUM_RECORDED_TURNS,
        }
    }

    /// Refuses a turn once `maximum_turns` are recorded, for a host or a test that wants a bound
    /// below `MAXIMUM_RECORDED_TURNS`.
    pub fn holding_at_most(mut self, maximum_turns: usize) -> Self {
        self.maximum_turns = maximum_turns.min(MAXIMUM_RECORDED_TURNS);
        self
    }

    /// Adds the turn `record` made of the input `envelope`, with the state hash the table settled
    /// at after the host finished with it. A turn that does not start where the last one settled,
    /// that is not numbered after it or that is past the bound is refused and the recording is as
    /// it was.
    pub fn record(
        &mut self,
        envelope: &InputEnvelope,
        record: &TurnRecord,
        settled_state_hash: u64,
    ) -> Result<(), SessionError> {
        let index = self.session.turns.len();
        if index >= self.maximum_turns {
            return Err(SessionError::TooManyTurns);
        }
        let turn = RecordedTurn::of(index, envelope, record, settled_state_hash)?;
        let expected = self.session.final_state_hash();
        if turn.state_hash_before != expected {
            return Err(SessionError::ChainBroken {
                turn: index,
                expected,
                found: turn.state_hash_before,
            });
        }
        let behind_last = self
            .session
            .turns
            .last()
            .is_some_and(|last| turn.correlation.turn_seq <= last.correlation.turn_seq);
        if behind_last {
            return Err(SessionError::TurnOutOfOrder { turn: index });
        }
        self.session.turns.push(turn);
        Ok(())
    }

    /// How many turns are recorded.
    pub fn len(&self) -> usize {
        self.session.turns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.session.turns.is_empty()
    }

    pub fn finish(self) -> RecordedSession {
        self.session
    }
}
