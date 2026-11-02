//! The first slice of the unified text service: a platform-neutral request and outcome, a
//! backend port, a configurable service that validates requests and checks answer structure,
//! and an observer port that receives one event per request.
//!
//! Backends own provider details. The service owns no prompt text, no account, no money and no
//! retry policy; a backend returns one outcome. Logging goes through `TextObserver`, so the
//! composition root decides where events land (the shared telemetry path), not this crate.

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::text_attempts::{AttemptStoreError, OperationKey};

/// Default instruction budget, in bytes.
pub const DEFAULT_INSTRUCTION_BYTES: usize = 64 * 1024;
/// Default structured-facts budget, in serialized bytes.
pub const DEFAULT_FACTS_BYTES: usize = 256 * 1024;
/// Default deadline ceiling for one request.
pub const DEFAULT_MAXIMUM_DEADLINE: Duration = Duration::from_secs(600);

/// Hard ceilings a composition root may not raise past.
const CEILING_INSTRUCTION_BYTES: usize = 1024 * 1024;
const CEILING_FACTS_BYTES: usize = 4 * 1024 * 1024;
const CEILING_DEADLINE: Duration = Duration::from_secs(3600);

/// Request budgets. Built once by the composition root and fixed for the service's lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextLimits {
    maximum_instruction_bytes: usize,
    maximum_facts_bytes: usize,
    maximum_deadline: Duration,
}

/// Why a set of limits was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextLimitsError {
    ZeroBudget,
    AboveCeiling,
}

impl TextLimits {
    pub fn new(
        maximum_instruction_bytes: usize,
        maximum_facts_bytes: usize,
        maximum_deadline: Duration,
    ) -> Result<Self, TextLimitsError> {
        if maximum_instruction_bytes == 0 || maximum_facts_bytes == 0 || maximum_deadline.is_zero()
        {
            return Err(TextLimitsError::ZeroBudget);
        }
        if maximum_instruction_bytes > CEILING_INSTRUCTION_BYTES
            || maximum_facts_bytes > CEILING_FACTS_BYTES
            || maximum_deadline > CEILING_DEADLINE
        {
            return Err(TextLimitsError::AboveCeiling);
        }
        Ok(Self {
            maximum_instruction_bytes,
            maximum_facts_bytes,
            maximum_deadline,
        })
    }

    pub fn maximum_instruction_bytes(&self) -> usize {
        self.maximum_instruction_bytes
    }

    pub fn maximum_facts_bytes(&self) -> usize {
        self.maximum_facts_bytes
    }

    pub fn maximum_deadline(&self) -> Duration {
        self.maximum_deadline
    }
}

impl Default for TextLimits {
    fn default() -> Self {
        Self {
            maximum_instruction_bytes: DEFAULT_INSTRUCTION_BYTES,
            maximum_facts_bytes: DEFAULT_FACTS_BYTES,
            maximum_deadline: DEFAULT_MAXIMUM_DEADLINE,
        }
    }
}

/// Which caller asked for text. Feature owners keep their prompt wording; this names the slot.
/// New purposes are added here as features adopt the service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextPurpose {
    Storyteller,
    /// Closed semantic judgments over listed options (intent, addressee, consent and similar).
    SemanticJudgment,
    /// Batched classification of an inbound support message.
    SupportJudgment,
    /// A suggested support action. A human reviews it; it never sends or acts.
    SupportDraft,
    /// Operator fixture traffic from the development probe; never a game feature.
    DevProbe,
}

impl TextPurpose {
    pub const ALL: [Self; 5] = [
        Self::Storyteller,
        Self::SemanticJudgment,
        Self::SupportJudgment,
        Self::SupportDraft,
        Self::DevProbe,
    ];

    /// A stable label for logs and metrics; never prompt text.
    pub fn label(self) -> &'static str {
        match self {
            Self::Storyteller => "storyteller",
            Self::SemanticJudgment => "semantic_judgment",
            Self::SupportJudgment => "support_judgment",
            Self::SupportDraft => "support_draft",
            Self::DevProbe => "dev_probe",
        }
    }

    /// The inverse of [`Self::label`], for stores that persist the purpose.
    pub fn parse(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|purpose| purpose.label() == label)
    }
}

/// The longest scope, profile or operation label, in bytes.
pub const MAXIMUM_LABEL_BYTES: usize = 128;

/// Why a label (scope, profile or operation id) was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelError {
    Empty,
    TooLong,
    ControlCharacter,
}

pub(crate) fn checked_label(value: &str) -> Result<String, LabelError> {
    if value.is_empty() {
        return Err(LabelError::Empty);
    }
    if value.len() > MAXIMUM_LABEL_BYTES {
        return Err(LabelError::TooLong);
    }
    if value.chars().any(char::is_control) {
        return Err(LabelError::ControlCharacter);
    }
    Ok(value.to_owned())
}

/// The operator-defined name of the backend, model and mode a request runs under. Reusing an
/// operation under a different profile is a conflict, never a replay of another model's answer.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TextProfile(String);

impl TextProfile {
    pub fn new(value: &str) -> Result<Self, LabelError> {
        checked_label(value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One text request. `output_schema` is a JSON Schema object the answer must satisfy.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRequest {
    pub purpose: TextPurpose,
    pub profile: TextProfile,
    pub instructions: String,
    pub facts: Value,
    pub output_name: String,
    pub output_schema: Value,
    pub deadline: Duration,
}

/// Why a request could not be served. Labels are stable and carry no request content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    Configuration,
    Authentication,
    UnsafeTools,
    Interrupted,
}

impl UnavailableReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Authentication => "authentication",
            Self::UnsafeTools => "unsafe_tools",
            Self::Interrupted => "interrupted",
        }
    }

    /// The inverse of [`Self::label`], for stores that persist the reason.
    pub fn parse(label: &str) -> Option<Self> {
        [
            Self::Configuration,
            Self::Authentication,
            Self::UnsafeTools,
            Self::Interrupted,
        ]
        .into_iter()
        .find(|reason| reason.label() == label)
    }
}

/// What a backend or the service reports. Only `Answered` carries model output.
#[derive(Clone, PartialEq)]
pub enum TextOutcome {
    /// A structurally checked JSON object.
    Answered(Value),
    /// The request could not be sent or was refused before any model work.
    Unavailable(UnavailableReason),
    /// The deadline elapsed before an answer arrived.
    Deadline,
    /// The provider or CLI returned something that is not a checked answer.
    Invalid,
    /// The owner's basis changed while the request was in flight. The answer, if any, was
    /// discarded and is never stored or replayed.
    Stale,
    /// The request may have reached the supplier and no result is known. Never resent.
    Unknown,
}

/// Debug never prints the answer: it is model output and may carry private game facts.
impl std::fmt::Debug for TextOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Answered(_) => formatter.write_str("Answered(<withheld>)"),
            Self::Unavailable(reason) => {
                formatter.debug_tuple("Unavailable").field(reason).finish()
            }
            Self::Deadline => formatter.write_str("Deadline"),
            Self::Invalid => formatter.write_str("Invalid"),
            Self::Stale => formatter.write_str("Stale"),
            Self::Unknown => formatter.write_str("Unknown"),
        }
    }
}

/// A request the service refuses before any backend call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextRequestError {
    EmptyInstructions,
    InstructionsTooLarge,
    FactsTooLarge,
    InvalidDeadline,
    InvalidSchema,
}

impl TextRequestError {
    /// A stable label for logs; carries no request content.
    pub fn label(self) -> &'static str {
        match self {
            Self::EmptyInstructions => "empty_instructions",
            Self::InstructionsTooLarge => "instructions_too_large",
            Self::FactsTooLarge => "facts_too_large",
            Self::InvalidDeadline => "invalid_deadline",
            Self::InvalidSchema => "invalid_schema",
        }
    }
}

/// Token evidence for one supplier attempt, as the supplier reported it. `total_tokens` is the
/// supplier total. Every other category is optional: a supplier that does not report it leaves it
/// unknown, never zero. Subset categories (cached input, reasoning) are not added to the total.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextUsage {
    pub total_tokens: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
}

/// What a backend returns: the outcome and any usage the supplier reported. Usage can exist for an
/// outcome that is not an answer (a rejected or discarded result was still paid for).
#[derive(Clone, Debug, PartialEq)]
pub struct TextReceipt {
    pub outcome: TextOutcome,
    pub usage: Option<TextUsage>,
}

impl From<TextOutcome> for TextReceipt {
    fn from(outcome: TextOutcome) -> Self {
        Self {
            outcome,
            usage: None,
        }
    }
}

/// The backend port. Implementations are private to a composition root.
pub trait TextBackend: Send + Sync {
    fn complete(&self, request: &TextRequest) -> impl Future<Output = TextReceipt> + Send;
}

/// What happened to one request, as a log line would describe it. Carries no prompt or answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEventKind {
    Refused(TextRequestError),
    Answered,
    Invalid,
    Unavailable(UnavailableReason),
    Deadline,
    /// An operation key was reused with a different request.
    Conflict,
    /// The same settled answer was returned again; no backend call happened.
    Replayed,
    /// Another live call owns this operation; nothing was sent by this call.
    InFlight,
    /// A send was recorded but its result was not; the request is never resent.
    Unknown,
    /// The operation was cancelled before dispatch.
    Cancelled,
    /// The attempt store failed before any send, so nothing was sent.
    StoreFailed(AttemptStoreError),
    /// The owner no longer holds the operation's basis, so nothing was sent.
    Unauthorized,
    /// The owner's basis changed during the call; the result was discarded.
    Stale,
    /// The feature accepted the candidate, which is now reusable.
    Qualified,
    /// The feature rejected the candidate, which is never reusable.
    Rejected,
    /// A reusable answer was withdrawn by the feature or an operator.
    Invalidated,
    /// A verdict arrived for an attempt that holds no candidate.
    NotCandidate,
}

impl TextEventKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Refused(error) => error.label(),
            Self::Answered => "answered",
            Self::Invalid => "invalid",
            Self::Unavailable(_) => "unavailable",
            Self::Deadline => "deadline",
            Self::Conflict => "conflict",
            Self::Replayed => "replayed",
            Self::InFlight => "in_flight",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
            Self::StoreFailed(_) => "store_failed",
            Self::Unauthorized => "unauthorized",
            Self::Stale => "stale",
            Self::Qualified => "qualified",
            Self::Rejected => "rejected",
            Self::Invalidated => "invalidated",
            Self::NotCandidate => "not_candidate",
        }
    }

    /// A closed reason label for logs, when the event has one. Never prompt or answer text.
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Refused(error) => Some(error.label()),
            Self::Unavailable(reason) => Some(reason.label()),
            Self::StoreFailed(error) => Some(error.label()),
            _ => None,
        }
    }
}

/// One record per request, passed to the observer exactly once.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEvent {
    pub purpose: TextPurpose,
    pub profile: TextProfile,
    /// The operation's scope and id, absent for events that are not about one operation.
    pub scope: Option<String>,
    pub operation: Option<String>,
    pub output_name: String,
    pub kind: TextEventKind,
    pub elapsed: Duration,
}

/// Receives one event per request. The default observer keeps nothing.
pub trait TextObserver: Send + Sync {
    fn observe(&self, event: &TextEvent);
}

/// Shared observers (for example one telemetry sink behind several services) need no wrapper.
impl<T: TextObserver> TextObserver for std::sync::Arc<T> {
    fn observe(&self, event: &TextEvent) {
        (**self).observe(event);
    }
}

/// A run may or may not have somewhere to log: `None` keeps nothing.
impl<T: TextObserver> TextObserver for Option<T> {
    fn observe(&self, event: &TextEvent) {
        if let Some(observer) = self {
            observer.observe(event);
        }
    }
}

/// The observer for callers that do not record events.
pub struct NoTextObserver;

impl TextObserver for NoTextObserver {
    fn observe(&self, _: &TextEvent) {}
}

/// The one text entry point for callers that do not own a provider.
pub struct TextService<B, O = NoTextObserver> {
    backend: B,
    limits: TextLimits,
    observer: O,
}

impl<B: TextBackend> TextService<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            limits: TextLimits::default(),
            observer: NoTextObserver,
        }
    }
}

impl<B: TextBackend, O: TextObserver> TextService<B, O> {
    pub fn with_limits(mut self, limits: TextLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_observer<P: TextObserver>(self, observer: P) -> TextService<B, P> {
        TextService {
            backend: self.backend,
            limits: self.limits,
            observer,
        }
    }

    /// Validates the request, asks the backend once, keeps only structurally valid answers and
    /// reports exactly one event.
    #[cfg(test)]
    pub(crate) async fn submit(
        &self,
        request: &TextRequest,
    ) -> Result<TextOutcome, TextRequestError> {
        let started = Instant::now();
        if let Err(error) = self.check(request) {
            self.report(None, request, TextEventKind::Refused(error), started);
            return Err(error);
        }
        let outcome = self.call_backend(request).await.outcome;
        self.report(None, request, kind_of(&outcome), started);
        Ok(outcome)
    }

    /// Checks a request without calling the backend or reporting an event.
    pub(crate) fn check(&self, request: &TextRequest) -> Result<(), TextRequestError> {
        if request.instructions.trim().is_empty() {
            return Err(TextRequestError::EmptyInstructions);
        }
        if request.instructions.len() > self.limits.maximum_instruction_bytes {
            return Err(TextRequestError::InstructionsTooLarge);
        }
        if serde_json::to_vec(&request.facts)
            .map_or(true, |bytes| bytes.len() > self.limits.maximum_facts_bytes)
        {
            return Err(TextRequestError::FactsTooLarge);
        }
        if request.deadline.is_zero() || request.deadline > self.limits.maximum_deadline {
            return Err(TextRequestError::InvalidDeadline);
        }
        let object_schema =
            request.output_schema.get("type").and_then(Value::as_str) == Some("object");
        if request.output_name.is_empty() || !object_schema {
            return Err(TextRequestError::InvalidSchema);
        }
        Ok(())
    }

    /// Asks the backend once and keeps only structurally valid answers. The request must already
    /// have passed [`Self::check`].
    pub(crate) async fn call_backend(&self, request: &TextRequest) -> TextReceipt {
        let receipt = self.backend.complete(request).await;
        let outcome = match receipt.outcome {
            TextOutcome::Answered(value) if value.is_object() => TextOutcome::Answered(value),
            TextOutcome::Answered(_) => TextOutcome::Invalid,
            other => other,
        };
        TextReceipt {
            outcome,
            usage: receipt.usage,
        }
    }

    /// Reports exactly one event for a request, measured from `started`.
    pub(crate) fn report(
        &self,
        key: Option<&OperationKey>,
        request: &TextRequest,
        kind: TextEventKind,
        started: Instant,
    ) {
        self.observer.observe(&TextEvent {
            purpose: request.purpose,
            profile: request.profile.clone(),
            scope: key.map(|key| key.scope().as_str().to_owned()),
            operation: key.map(|key| key.operation().to_owned()),
            output_name: request.output_name.clone(),
            kind,
            elapsed: started.elapsed(),
        });
    }
}

pub(crate) fn kind_of(outcome: &TextOutcome) -> TextEventKind {
    match outcome {
        TextOutcome::Answered(_) => TextEventKind::Answered,
        TextOutcome::Invalid => TextEventKind::Invalid,
        TextOutcome::Unavailable(reason) => TextEventKind::Unavailable(*reason),
        TextOutcome::Deadline => TextEventKind::Deadline,
        TextOutcome::Stale => TextEventKind::Stale,
        TextOutcome::Unknown => TextEventKind::Unknown,
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
