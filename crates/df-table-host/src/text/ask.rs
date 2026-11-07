//! One question to the text service and the check of its answer: `TextRoute::ask`.
//!
//! A host feature that needs a model builds a `TextRequest` and a reader for its answer; `ask`
//! submits it under the feature's operation key, has the reader check a fresh answer, and tells
//! the service whether to keep it (a checked answer is qualified, an unchecked one is rejected and
//! never asked again under the same key). An answer the service already qualified replays through
//! the reader without a call. Every refusal is a [`TextAskError`] that names the service's closed
//! cause and, for an answer the reader refused, the reader's own error.

use std::fmt;

use df_ai::text::{TextOutcome, TextRequest, UnavailableReason};
use df_ai::text_attempts::{DurableTextError, OperationKey, Qualification, Submission, Verdict};

use super::{ServiceReason, TextRoute, TextRuntime, TextServiceCause};

/// What the text service gave for a question: the reader's value, and whether it was a replay of
/// an answer the service had already qualified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked<T> {
    pub value: T,
    pub replayed: bool,
}

/// Why the text service gave no usable answer; `E` is the reader's own error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextAskError<E> {
    /// The service gave no answer: `cause` is one of the service's own (not the reader's), and
    /// `reason` what it reported behind it, when it reported one.
    Service {
        cause: TextServiceCause,
        reason: Option<ServiceReason>,
    },
    /// The reader refused a fresh answer; the answer was rejected.
    AnswerRefused(E),
    /// The reader refused an answer the service replayed.
    ReplayRefused(E),
}

impl<E: fmt::Display> TextAskError<E> {
    /// The closed cause of this failure and the reason or the reader's own sentence behind it,
    /// when there is one.
    pub fn service_cause(&self) -> (TextServiceCause, Option<String>) {
        match self {
            Self::Service { cause, reason } => {
                (*cause, reason.map(|reason| reason.key().to_owned()))
            }
            Self::AnswerRefused(error) => {
                (TextServiceCause::AnswerRefused, Some(error.to_string()))
            }
            Self::ReplayRefused(error) => {
                (TextServiceCause::ReplayRefused, Some(error.to_string()))
            }
        }
    }
}

fn service<E>(cause: TextServiceCause) -> TextAskError<E> {
    TextAskError::Service {
        cause,
        reason: None,
    }
}

fn with_reason<E>(cause: TextServiceCause, reason: ServiceReason) -> TextAskError<E> {
    TextAskError::Service {
        cause,
        reason: Some(reason),
    }
}

/// What the service reported when it refused a question before sending it.
fn refusal_reason(error: DurableTextError) -> ServiceReason {
    match error {
        DurableTextError::Refused(_) => ServiceReason::RequestRefused,
        DurableTextError::Conflict => ServiceReason::Conflict,
        DurableTextError::Store(_) => ServiceReason::Store,
        DurableTextError::Unauthorized => ServiceReason::Unauthorized,
        DurableTextError::UnknownOperation => ServiceReason::UnknownOperation,
    }
}

/// What the service reported when it settled a question without an answer.
fn outcome_reason(outcome: &TextOutcome) -> ServiceReason {
    match outcome {
        TextOutcome::Answered(_) | TextOutcome::Unknown => ServiceReason::Unknown,
        TextOutcome::Unavailable(UnavailableReason::Configuration) => ServiceReason::Configuration,
        TextOutcome::Unavailable(UnavailableReason::Authentication) => {
            ServiceReason::Authentication
        }
        TextOutcome::Unavailable(UnavailableReason::UnsafeTools) => ServiceReason::UnsafeTools,
        TextOutcome::Unavailable(UnavailableReason::Interrupted) => ServiceReason::Interrupted,
        TextOutcome::Deadline => ServiceReason::Deadline,
        TextOutcome::Invalid => ServiceReason::Invalid,
        TextOutcome::Stale => ServiceReason::Stale,
    }
}

/// What the service reported when it did not keep a checked answer.
fn not_kept_reason(kept: Result<Qualification, DurableTextError>) -> ServiceReason {
    match kept {
        Ok(Qualification::Rejected) => ServiceReason::Rejected,
        Ok(Qualification::NotACandidate) => ServiceReason::NotACandidate,
        Ok(Qualification::Qualified | Qualification::Stale) => ServiceReason::Stale,
        Err(error) => refusal_reason(error),
    }
}

impl TextRuntime {
    /// Submits `request` under `key` to the default route, after one `TEXT started` line.
    #[cfg(test)]
    pub(crate) async fn submit(
        &self,
        key: &OperationKey,
        request: &TextRequest,
    ) -> Result<Submission, DurableTextError> {
        self.default_route.submit(key, request).await
    }

    /// [`TextRoute::ask`] over the default route.
    pub async fn ask<T, E: fmt::Display>(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        read: impl Fn(&serde_json::Value) -> Result<T, E>,
    ) -> Result<Asked<T>, TextAskError<E>> {
        self.default_route.ask(key, request, read).await
    }
}

impl TextRoute {
    /// Submits `request` under `key` to the service, after one `TEXT started` line naming the
    /// operation and its deadline. Every host feature submits through here.
    async fn submit(
        &self,
        key: &OperationKey,
        request: &TextRequest,
    ) -> Result<Submission, DurableTextError> {
        (self.record)(format!(
            "TEXT started purpose={} profile={} scope={} operation={} deadline_ms={}",
            request.purpose.key(),
            request.profile.as_str(),
            key.scope().as_str(),
            key.operation(),
            request.deadline.as_millis()
        ));
        self.service.submit(key, request).await
    }

    /// Asks the service `request` under `key` and reads its answer with `read`: a fresh answer
    /// that reads is qualified (kept), one that does not is rejected; an answer the service
    /// already qualified replays through `read` without a call.
    pub async fn ask<T, E: fmt::Display>(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        read: impl Fn(&serde_json::Value) -> Result<T, E>,
    ) -> Result<Asked<T>, TextAskError<E>> {
        let submitted = self
            .submit(key, request)
            .await
            .map_err(|error| with_reason(TextServiceCause::Refused, refusal_reason(error)))?;
        match submitted {
            Submission::Candidate { answer, .. } => match read(&answer) {
                Ok(value) => self.keep(key, request, value).await,
                Err(error) => {
                    // The rejection only stops this answer from being reused; the error below
                    // reports the failure either way, and the service records the verdict event.
                    let _ = self.service.qualify(key, request, Verdict::Rejected).await;
                    Err(TextAskError::AnswerRefused(error))
                }
            },
            Submission::Settled {
                outcome: TextOutcome::Answered(answer),
                replayed,
                ..
            } => read(&answer)
                .map(|value| Asked { value, replayed })
                .map_err(TextAskError::ReplayRefused),
            Submission::Settled { outcome, .. } => Err(with_reason(
                TextServiceCause::NoAnswer,
                outcome_reason(&outcome),
            )),
            Submission::Rejected { .. } => Err(service(TextServiceCause::RejectedBefore)),
            Submission::InFlight => Err(service(TextServiceCause::InFlight)),
            Submission::Unknown => Err(service(TextServiceCause::Lost)),
            Submission::Cancelled => Err(service(TextServiceCause::Cancelled)),
        }
    }

    /// Tells the service to keep the checked answer `value`.
    async fn keep<T, E>(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        value: T,
    ) -> Result<Asked<T>, TextAskError<E>> {
        match self.service.qualify(key, request, Verdict::Accepted).await {
            Ok(Qualification::Qualified) => Ok(Asked {
                value,
                replayed: false,
            }),
            other => Err(with_reason(
                TextServiceCause::NotKept,
                not_kept_reason(other),
            )),
        }
    }
}
