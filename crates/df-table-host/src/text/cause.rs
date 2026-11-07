//! Why the text service gave no usable answer, as closed keys.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Why the text service gave no usable answer to a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextServiceCause {
    /// It refused the question.
    Refused,
    /// It did not keep a checked answer.
    NotKept,
    /// Its answer was not one the caller's reader accepts; the answer was rejected.
    AnswerRefused,
    /// It replayed an answer the caller's reader refuses.
    ReplayRefused,
    /// It settled the question without an answer.
    NoAnswer,
    /// It rejected this exact question's answer before and does not ask again.
    RejectedBefore,
    /// It is still answering this question.
    InFlight,
    /// It sent the question but lost its result; it is never resent.
    Lost,
    Cancelled,
}

impl TextServiceCause {
    pub const ALL: [Self; 9] = [
        Self::Refused,
        Self::NotKept,
        Self::AnswerRefused,
        Self::ReplayRefused,
        Self::NoAnswer,
        Self::RejectedBefore,
        Self::InFlight,
        Self::Lost,
        Self::Cancelled,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::NotKept => "not_kept",
            Self::AnswerRefused => "answer_refused",
            Self::ReplayRefused => "replay_refused",
            Self::NoAnswer => "no_answer",
            Self::RejectedBefore => "rejected_before",
            Self::InFlight => "in_flight",
            Self::Lost => "lost",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cause| cause.key() == key)
    }
}

impl fmt::Display for TextServiceCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Refused => "refused the question",
            Self::NotKept => "did not keep the answer",
            Self::AnswerRefused => "gave an answer the reader refused; it was rejected",
            Self::ReplayRefused => "replayed an answer the reader refuses",
            Self::NoAnswer => "gave no answer",
            Self::RejectedBefore => concat!(
                "rejected this exact question's answer before and does not ask again; ",
                "change the question"
            ),
            Self::InFlight => "is still answering this question; try again shortly",
            Self::Lost => "sent this question but its result was lost; it is never resent",
            Self::Cancelled => "cancelled the question",
        })
    }
}

/// What the service reported behind a [`TextServiceCause::Refused`], `NoAnswer` or `NotKept`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceReason {
    /// The service refused the request before sending it.
    RequestRefused,
    /// The operation key was used for another request.
    Conflict,
    /// The attempt store failed.
    Store,
    /// The owner does not hold the operation's basis.
    Unauthorized,
    /// No attempt exists under the key.
    UnknownOperation,
    /// The backend is not configured.
    Configuration,
    /// The backend refused its credentials.
    Authentication,
    /// The backend would have run unsafe tools.
    UnsafeTools,
    /// The call was interrupted.
    Interrupted,
    /// The deadline elapsed.
    Deadline,
    /// The backend returned something that is not a checked answer.
    Invalid,
    /// The owner's basis changed in flight; the answer was discarded.
    Stale,
    /// The request may have reached the supplier and no result is known.
    Unknown,
    /// The service rejected the candidate.
    Rejected,
    /// The attempt held no candidate.
    NotACandidate,
}

impl ServiceReason {
    pub const ALL: [Self; 15] = [
        Self::RequestRefused,
        Self::Conflict,
        Self::Store,
        Self::Unauthorized,
        Self::UnknownOperation,
        Self::Configuration,
        Self::Authentication,
        Self::UnsafeTools,
        Self::Interrupted,
        Self::Deadline,
        Self::Invalid,
        Self::Stale,
        Self::Unknown,
        Self::Rejected,
        Self::NotACandidate,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::RequestRefused => "request_refused",
            Self::Conflict => "conflict",
            Self::Store => "store",
            Self::Unauthorized => "unauthorized",
            Self::UnknownOperation => "unknown_operation",
            Self::Configuration => "configuration",
            Self::Authentication => "authentication",
            Self::UnsafeTools => "unsafe_tools",
            Self::Interrupted => "interrupted",
            Self::Deadline => "deadline",
            Self::Invalid => "invalid",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
            Self::Rejected => "rejected",
            Self::NotACandidate => "not_a_candidate",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.key() == key)
    }
}
