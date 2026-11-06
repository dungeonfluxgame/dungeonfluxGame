//! The first slice of the unified text service: a platform-neutral request and outcome, a
//! backend port, a configurable service that validates requests and checks answer structure,
//! and an observer port that receives one event per request.
//!
//! Backends own provider details. The service owns no prompt text, no account, no money and no
//! retry policy; a backend returns one outcome. Logging goes through `TextObserver`, so the
//! composition root decides where events land (the shared telemetry path), not this crate.

use std::fmt;
use std::io;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::text_attempts::{AttemptStoreError, OperationKey};

/// Default instruction budget, in bytes. Generous so development prompts are never cut short;
/// the model's context window is the real bound, and the backend reports it.
pub const DEFAULT_INSTRUCTION_BYTES: usize = 1024 * 1024;
/// Default structured-facts budget, in serialized bytes. Generous for the same reason.
pub const DEFAULT_FACTS_BYTES: usize = 16 * 1024 * 1024;
/// Default deadline ceiling for one request. Long enough for a slow local model.
pub const DEFAULT_MAXIMUM_DEADLINE: Duration = Duration::from_secs(60 * 60);

/// The largest instruction budget a composition root may configure, in bytes.
const CEILING_INSTRUCTION_BYTES: usize = 64 * 1024 * 1024;
/// The largest facts budget a composition root may configure, in serialized bytes.
const CEILING_FACTS_BYTES: usize = 256 * 1024 * 1024;
/// The longest deadline a composition root may configure.
const CEILING_DEADLINE: Duration = Duration::from_secs(24 * 60 * 60);

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

impl fmt::Display for TextLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroBudget => "a text budget or deadline is zero",
            Self::AboveCeiling => "a text budget or deadline is above its built-in ceiling",
        })
    }
}

impl std::error::Error for TextLimitsError {}

impl TextLimits {
    /// Zero budgets and budgets above the built-in ceilings are refused.
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextPurpose {
    /// The storyteller: wording and creation for the game.
    Storyteller,
    /// Closed semantic judgments over listed options (intent, addressee, consent and similar).
    SemanticJudgment,
    /// Batched classification of an inbound support message.
    SupportJudgment,
    /// A suggested support action. A human reviews it; it never sends or acts.
    SupportDraft,
    /// Operator fixture traffic from the development probe; never a game feature.
    DevProbe,
    /// Which scanned person in a picture is which named character (the imagery API prepares the
    /// pictures and checks the answer).
    CharacterIdentity,
}

impl TextPurpose {
    /// Every purpose, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Storyteller,
        Self::SemanticJudgment,
        Self::SupportJudgment,
        Self::SupportDraft,
        Self::DevProbe,
        Self::CharacterIdentity,
    ];

    /// The stable key for logs, metrics and stores; never prompt text.
    pub fn key(self) -> &'static str {
        match self {
            Self::Storyteller => "storyteller",
            Self::SemanticJudgment => "semantic_judgment",
            Self::SupportJudgment => "support_judgment",
            Self::SupportDraft => "support_draft",
            Self::DevProbe => "dev_probe",
            Self::CharacterIdentity => "character_identity",
        }
    }

    /// The inverse of [`Self::key`], for stores that persist the purpose.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|purpose| purpose.key() == key)
    }
}

/// The longest scope, profile, operation or output-name label, in bytes.
pub const MAXIMUM_LABEL_BYTES: usize = 128;

/// Why a label (scope, profile, operation id or output name) was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelError {
    Empty,
    TooLong,
    ControlCharacter,
}

impl fmt::Display for LabelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "the label is empty",
            Self::TooLong => "the label is longer than its byte limit",
            Self::ControlCharacter => "the label holds a control character",
        })
    }
}

impl std::error::Error for LabelError {}

/// A label is non-empty, at most [`MAXIMUM_LABEL_BYTES`] and free of control characters. The
/// PostgreSQL attempt table bounds its label columns at 128 *characters*; a label within 128
/// bytes always fits, so the byte bound here is the stricter one. Keep the two in step.
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
    /// A profile name follows the label rules ([`LabelError`]).
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
    /// Authorized pictures the model sees, in order, each introduced by its label. Empty for a
    /// text-only request. A backend that cannot send pictures answers `Unavailable`.
    pub images: Vec<TextImage>,
}

/// The most pictures one request carries.
pub const MAXIMUM_IMAGES: usize = 1_024;
/// The largest total size of all pictures in one request, in bytes.
pub const MAXIMUM_IMAGE_BYTES: usize = 128 * 1024 * 1024;

/// The media type of a picture part.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextImageType {
    Jpeg,
    Png,
    Webp,
}

impl TextImageType {
    /// The IANA media type sent with the picture.
    pub fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

/// One picture part: a short label the instructions refer to (such as `Candidate 3`), its media
/// type and its bytes. Debug output names the label and size, never the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct TextImage {
    pub label: String,
    pub media_type: TextImageType,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for TextImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextImage")
            .field("label", &self.label)
            .field("media_type", &self.media_type)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// Why a request could not be served. Labels are stable and carry no request content.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    Configuration,
    Authentication,
    UnsafeTools,
    Interrupted,
}

impl UnavailableReason {
    const ALL: [Self; 4] = [
        Self::Configuration,
        Self::Authentication,
        Self::UnsafeTools,
        Self::Interrupted,
    ];

    /// The stable key for logs and stores; never request content.
    pub fn key(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Authentication => "authentication",
            Self::UnsafeTools => "unsafe_tools",
            Self::Interrupted => "interrupted",
        }
    }

    /// The inverse of [`Self::key`], for stores that persist the reason.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.key() == key)
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
    /// The output schema is not a JSON object schema, or the output name breaks the label rules
    /// ([`LabelError`]).
    InvalidSchema,
    /// Too many pictures, too many picture bytes, an empty picture or an unlabelled one.
    InvalidImages,
}

impl TextRequestError {
    /// The stable key for logs; carries no request content.
    pub fn key(self) -> &'static str {
        match self {
            Self::EmptyInstructions => "empty_instructions",
            Self::InstructionsTooLarge => "instructions_too_large",
            Self::FactsTooLarge => "facts_too_large",
            Self::InvalidDeadline => "invalid_deadline",
            Self::InvalidSchema => "invalid_schema",
            Self::InvalidImages => "invalid_images",
        }
    }
}

impl fmt::Display for TextRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyInstructions => "the text request has no instructions",
            Self::InstructionsTooLarge => "the text request instructions are over their budget",
            Self::FactsTooLarge => "the text request facts are over their budget",
            Self::InvalidDeadline => "the text request deadline is zero or above the ceiling",
            Self::InvalidSchema => "the text request output schema or output name is invalid",
            Self::InvalidImages => "the text request pictures are invalid",
        })
    }
}

impl std::error::Error for TextRequestError {}

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
    /// Runs one request to one receipt within `request.deadline`. Never retries.
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
    /// The stable key for logs and metrics; never prompt or answer text.
    pub fn key(self) -> &'static str {
        match self {
            Self::Refused(error) => error.key(),
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

    /// A closed reason key for logs, when the event has one. Never prompt or answer text.
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Refused(error) => Some(error.key()),
            Self::Unavailable(reason) => Some(reason.key()),
            Self::StoreFailed(error) => Some(error.key()),
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
    /// The tokens the supplier reported for the call this event settles; `None` when no call was
    /// made by this request (a replay, a refusal) or the supplier reported none.
    pub usage: Option<TextUsage>,
    /// Which submit of the operation this is, from 1: a durable service counts the submits of each
    /// operation key it served since it started (a retry under the same key is the next
    /// number); a verdict or withdrawal carries the operation's latest count; a request with no
    /// operation is 1.
    pub attempt: u32,
}

/// Receives one event per request. The default observer keeps nothing.
pub trait TextObserver: Send + Sync {
    /// Records one event. It runs on the caller's path, so it must not block.
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
    /// A service with the default limits and no observer.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            limits: TextLimits::default(),
            observer: NoTextObserver,
        }
    }
}

impl<B: TextBackend, O: TextObserver> TextService<B, O> {
    /// Replaces the request budgets.
    pub fn with_limits(mut self, limits: TextLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Replaces the observer that receives one event per request.
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
        // A request with no operation is its own first and only attempt.
        let facts = |usage| EventFacts { usage, attempt: 1 };
        if let Err(error) = self.check(request) {
            let kind = TextEventKind::Refused(error);
            self.report(None, request, kind, started, facts(None));
            return Err(error);
        }
        let receipt = self.call_backend(request).await;
        let kind = kind_of(&receipt.outcome);
        self.report(None, request, kind, started, facts(receipt.usage));
        Ok(receipt.outcome)
    }

    /// Checks a request without calling the backend or reporting an event.
    pub(crate) fn check(&self, request: &TextRequest) -> Result<(), TextRequestError> {
        if request.instructions.trim().is_empty() {
            return Err(TextRequestError::EmptyInstructions);
        }
        if request.instructions.len() > self.limits.maximum_instruction_bytes {
            return Err(TextRequestError::InstructionsTooLarge);
        }
        if !serializes_within(&request.facts, self.limits.maximum_facts_bytes) {
            return Err(TextRequestError::FactsTooLarge);
        }
        if request.deadline.is_zero() || request.deadline > self.limits.maximum_deadline {
            return Err(TextRequestError::InvalidDeadline);
        }
        let object_schema =
            request.output_schema.get("type").and_then(Value::as_str) == Some("object");
        // The output name is stored with the attempt, so it follows the label rules here instead
        // of failing later as a store rejection after admission.
        if checked_label(&request.output_name).is_err() || !object_schema {
            return Err(TextRequestError::InvalidSchema);
        }
        if !images_are_valid(&request.images) {
            return Err(TextRequestError::InvalidImages);
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
        facts: EventFacts,
    ) {
        self.observer.observe(&TextEvent {
            purpose: request.purpose,
            profile: request.profile.clone(),
            scope: key.map(|key| key.scope().as_str().to_owned()),
            operation: key.map(|key| key.operation().to_owned()),
            output_name: request.output_name.clone(),
            kind,
            elapsed: started.elapsed(),
            usage: facts.usage,
            attempt: facts.attempt,
        });
    }
}

/// What an event says about its call beyond its kind: the usage this request's call reported and
/// which submit of the operation it is (see [`TextEvent`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct EventFacts {
    pub(crate) usage: Option<TextUsage>,
    pub(crate) attempt: u32,
}

/// Whether `value` serializes to at most `maximum_bytes`. Serialization stops at the first byte
/// past the budget, so an oversized value is never written out in full.
fn serializes_within(value: &Value, maximum_bytes: usize) -> bool {
    let mut counter = BoundedByteCounter {
        remaining_bytes: maximum_bytes,
    };
    serde_json::to_writer(&mut counter, value).is_ok()
}

/// A writer that keeps no bytes and fails once more than its budget has been written.
struct BoundedByteCounter {
    remaining_bytes: usize,
}

impl io::Write for BoundedByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(buffer.len())
            .ok_or_else(|| io::Error::other("serialized value is over its byte budget"))?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// At most [`MAXIMUM_IMAGES`] pictures of at most [`MAXIMUM_IMAGE_BYTES`] in total, each one
/// labelled and non-empty.
fn images_are_valid(images: &[TextImage]) -> bool {
    let total_bytes = images
        .iter()
        .try_fold(0usize, |total, image| total.checked_add(image.bytes.len()));
    images.len() <= MAXIMUM_IMAGES
        && total_bytes.is_some_and(|total| total <= MAXIMUM_IMAGE_BYTES)
        && images
            .iter()
            .all(|image| !image.bytes.is_empty() && !image.label.trim().is_empty())
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
mod text_tests;
