//! One input as a recording keeps it: everything an input is except the words a player typed.
//!
//! It owns `RecordedInput`, the serialisable twin of an `InputEnvelope`'s origin, with
//! `RecordedInput::of` to read one off an envelope and `into_envelope` to build the envelope again.
//! A typed line is kept as its kind, its utterance id and its length class and never as text
//! (RES36), so a recording holds no player words: replaying a line needs the words from outside
//! (`LineTexts` in the runner), and `into_envelope` refuses text whose length class is not the
//! one recorded. Engine inputs keep every field; `ProposalOrigin` and `Readiness` have no serde of
//! their own, so they have twins here. It never decides what an input means.

use serde::{Deserialize, Serialize};

use crate::envelope::{
    CancelCause, EngineControl, EngineInput, InputEnvelope, InputKind, InputOrigin, PendingId,
    PlayerInput, ProposalOrigin, ProposerRef, Sender, TimerName, UtteranceId, ViewStamp,
};
use crate::event::FailureCause;
use crate::identity::{OperationId, SeatId};
use crate::ports::{ActionId, Readiness, ResourceHandle, RulingKey, RulingScope};
use crate::private_data::{LengthClass, Measured, PlayerText};
use crate::table_state::Presence;

/// Which kind of typed line a seat sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LineKind {
    SubmitText,
    Say,
    PrivateMessage { to: SeatId },
}

/// `ProposalOrigin` as a recording writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedOrigin {
    Npc,
    World,
    Story,
    Encounter,
    Timer,
    Machine,
}

impl From<ProposalOrigin> for RecordedOrigin {
    fn from(origin: ProposalOrigin) -> Self {
        match origin {
            ProposalOrigin::Npc => Self::Npc,
            ProposalOrigin::World => Self::World,
            ProposalOrigin::Story => Self::Story,
            ProposalOrigin::Encounter => Self::Encounter,
            ProposalOrigin::Timer => Self::Timer,
            ProposalOrigin::Machine => Self::Machine,
        }
    }
}

impl From<RecordedOrigin> for ProposalOrigin {
    fn from(origin: RecordedOrigin) -> Self {
        match origin {
            RecordedOrigin::Npc => Self::Npc,
            RecordedOrigin::World => Self::World,
            RecordedOrigin::Story => Self::Story,
            RecordedOrigin::Encounter => Self::Encounter,
            RecordedOrigin::Timer => Self::Timer,
            RecordedOrigin::Machine => Self::Machine,
        }
    }
}

/// `Readiness` as a recording writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedReadiness {
    Pending,
    Ready,
    Failed { cause: FailureCause },
}

impl From<Readiness> for RecordedReadiness {
    fn from(readiness: Readiness) -> Self {
        match readiness {
            Readiness::Pending => Self::Pending,
            Readiness::Ready => Self::Ready,
            Readiness::Failed(cause) => Self::Failed { cause },
        }
    }
}

impl From<RecordedReadiness> for Readiness {
    fn from(readiness: RecordedReadiness) -> Self {
        match readiness {
            RecordedReadiness::Pending => Self::Pending,
            RecordedReadiness::Ready => Self::Ready,
            RecordedReadiness::Failed { cause } => Self::Failed(cause),
        }
    }
}

/// Why the words for a recorded line could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineTextError {
    /// The input is a typed line and no text was given.
    Missing,
    /// The text given is not of the length class the recording holds.
    LengthChanged {
        recorded: LengthClass,
        given: LengthClass,
    },
}

impl std::fmt::Display for LineTextError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => formatter.write_str("a recorded line needs its text to be replayed"),
            Self::LengthChanged { recorded, given } => write!(
                formatter,
                "the text is {} and the recorded line was {}",
                given.key(),
                recorded.key()
            ),
        }
    }
}

impl std::error::Error for LineTextError {}

/// What an input was, without any text a player typed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "input", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedInput {
    Tap {
        seat: SeatId,
        action: ActionId,
    },
    Line {
        seat: SeatId,
        line: LineKind,
        utterance: u64,
        length: LengthClass,
    },
    SafetyCard {
        seat: SeatId,
    },
    PendingAnswer {
        seat: SeatId,
        pending: PendingId,
        choice: ActionId,
    },
    SeatPresence {
        seat: SeatId,
        presence: Presence,
    },
    RevokeRuling {
        key: RulingKey,
        scope: RulingScope,
    },
    CancelPending {
        pending: PendingId,
        cause: CancelCause,
    },
    ResumeTable,
    Proposal {
        origin: RecordedOrigin,
        proposer: ProposerRef,
    },
    Timer {
        timer: TimerName,
    },
    ResourceResult {
        handle: u64,
        readiness: RecordedReadiness,
    },
}

impl RecordedInput {
    fn line(seat: SeatId, line: LineKind, utterance: UtteranceId, text: &PlayerText) -> Self {
        Self::Line {
            seat,
            line,
            utterance: utterance.get(),
            length: LengthClass::of_length(text.expose().length_in_chars()),
        }
    }

    /// What the envelope's input was, with its text left out.
    pub fn of(envelope: &InputEnvelope) -> Self {
        match envelope.origin() {
            InputOrigin::Player { seat, input } => match input {
                PlayerInput::Tap { action } => Self::Tap {
                    seat: *seat,
                    action: action.clone(),
                },
                PlayerInput::SubmitText { utterance, text } => {
                    Self::line(*seat, LineKind::SubmitText, *utterance, text)
                }
                PlayerInput::Say { utterance, text } => {
                    Self::line(*seat, LineKind::Say, *utterance, text)
                }
                PlayerInput::PrivateMessage {
                    utterance,
                    to,
                    text,
                } => Self::line(
                    *seat,
                    LineKind::PrivateMessage { to: *to },
                    *utterance,
                    text,
                ),
                PlayerInput::SafetyCard => Self::SafetyCard { seat: *seat },
                PlayerInput::PendingAnswer { pending, choice } => Self::PendingAnswer {
                    seat: *seat,
                    pending: *pending,
                    choice: choice.clone(),
                },
            },
            InputOrigin::Engine(input) => match input {
                EngineInput::Control(control) => match control {
                    EngineControl::SeatPresence { seat, presence } => Self::SeatPresence {
                        seat: *seat,
                        presence: *presence,
                    },
                    EngineControl::RevokeRuling { key, scope } => Self::RevokeRuling {
                        key: key.clone(),
                        scope: *scope,
                    },
                    EngineControl::CancelPending { pending, cause } => Self::CancelPending {
                        pending: *pending,
                        cause: *cause,
                    },
                    EngineControl::ResumeTable => Self::ResumeTable,
                },
                EngineInput::Proposal { origin, proposer } => Self::Proposal {
                    origin: (*origin).into(),
                    proposer: proposer.clone(),
                },
                EngineInput::Timer { timer } => Self::Timer {
                    timer: timer.clone(),
                },
                EngineInput::ResourceResult { handle, readiness } => Self::ResourceResult {
                    handle: handle.get(),
                    readiness: (*readiness).into(),
                },
            },
        }
    }

    /// The kind of input, as the envelope built from it reports it.
    pub fn kind(&self) -> InputKind {
        match self {
            Self::Tap { .. } => InputKind::Tap,
            Self::Line { line, .. } => match line {
                LineKind::SubmitText => InputKind::SubmitText,
                LineKind::Say => InputKind::Say,
                LineKind::PrivateMessage { .. } => InputKind::PrivateMessage,
            },
            Self::SafetyCard { .. } => InputKind::SafetyCard,
            Self::PendingAnswer { .. } => InputKind::PendingAnswer,
            Self::SeatPresence { .. }
            | Self::RevokeRuling { .. }
            | Self::CancelPending { .. }
            | Self::ResumeTable
            | Self::Proposal { .. } => InputKind::EngineProposal,
            Self::Timer { .. } => InputKind::Timer,
            Self::ResourceResult { .. } => InputKind::ResourceResult,
        }
    }

    /// Who sent it.
    pub fn sender(&self) -> Sender {
        match self {
            Self::Tap { seat, .. }
            | Self::Line { seat, .. }
            | Self::SafetyCard { seat }
            | Self::PendingAnswer { seat, .. } => Sender::Seat(*seat),
            Self::SeatPresence { .. }
            | Self::RevokeRuling { .. }
            | Self::CancelPending { .. }
            | Self::ResumeTable
            | Self::Proposal { .. }
            | Self::Timer { .. }
            | Self::ResourceResult { .. } => Sender::Engine,
        }
    }

    /// The envelope this input was. A typed line needs its words, of the recorded length class;
    /// any other input ignores `text`.
    pub fn into_envelope(
        &self,
        operation_id: OperationId,
        view: ViewStamp,
        text: Option<&PlayerText>,
    ) -> Result<InputEnvelope, LineTextError> {
        let player = |seat: SeatId, input: PlayerInput| {
            InputEnvelope::from_player(operation_id, seat, view, input)
        };
        let engine = |input: EngineInput| InputEnvelope::from_engine(operation_id, view, input);
        Ok(match self {
            Self::Tap { seat, action } => player(
                *seat,
                PlayerInput::Tap {
                    action: action.clone(),
                },
            ),
            Self::Line {
                seat,
                line,
                utterance,
                length,
            } => {
                let text = text.ok_or(LineTextError::Missing)?;
                let given = LengthClass::of_length(text.expose().length_in_chars());
                if given != *length {
                    return Err(LineTextError::LengthChanged {
                        recorded: *length,
                        given,
                    });
                }
                let utterance = UtteranceId::new(*utterance);
                let text = text.clone();
                player(
                    *seat,
                    match line {
                        LineKind::SubmitText => PlayerInput::SubmitText { utterance, text },
                        LineKind::Say => PlayerInput::Say { utterance, text },
                        LineKind::PrivateMessage { to } => PlayerInput::PrivateMessage {
                            utterance,
                            to: *to,
                            text,
                        },
                    },
                )
            }
            Self::SafetyCard { seat } => player(*seat, PlayerInput::SafetyCard),
            Self::PendingAnswer {
                seat,
                pending,
                choice,
            } => player(
                *seat,
                PlayerInput::PendingAnswer {
                    pending: *pending,
                    choice: choice.clone(),
                },
            ),
            Self::SeatPresence { seat, presence } => {
                engine(EngineInput::Control(EngineControl::SeatPresence {
                    seat: *seat,
                    presence: *presence,
                }))
            }
            Self::RevokeRuling { key, scope } => {
                engine(EngineInput::Control(EngineControl::RevokeRuling {
                    key: key.clone(),
                    scope: *scope,
                }))
            }
            Self::CancelPending { pending, cause } => {
                engine(EngineInput::Control(EngineControl::CancelPending {
                    pending: *pending,
                    cause: *cause,
                }))
            }
            Self::ResumeTable => engine(EngineInput::Control(EngineControl::ResumeTable)),
            Self::Proposal { origin, proposer } => engine(EngineInput::Proposal {
                origin: (*origin).into(),
                proposer: proposer.clone(),
            }),
            Self::Timer { timer } => engine(EngineInput::Timer {
                timer: timer.clone(),
            }),
            Self::ResourceResult { handle, readiness } => engine(EngineInput::ResourceResult {
                handle: ResourceHandle::new(*handle),
                readiness: (*readiness).into(),
            }),
        })
    }
}
