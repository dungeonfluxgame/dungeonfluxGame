//! Durable attempts for text requests. One scoped operation key owns one attempt record. The
//! service writes a dispatch marker before any possible send and records the backend's result
//! after it. A structurally valid answer is stored as a candidate and becomes reusable only when
//! the feature qualifies it; a repeated submit replays a qualified answer and never an
//! unqualified one as a hit. A send whose result was never recorded resolves to `Unknown` and is
//! never resent.
//!
//! The store is a port. `df-persistence` implements it on PostgreSQL and `text_fixture` keeps an
//! in-memory one for fixtures. Composition supplies the owner token and the authority, so this
//! module reads no clock, randomness or environment.

use std::{fmt, future::Future, sync::Arc, time::Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::text::{
    LabelError, MAXIMUM_LABEL_BYTES, NoTextObserver, TextBackend, TextEventKind, TextObserver,
    TextOutcome, TextProfile, TextPurpose, TextRequest, TextRequestError, TextService, TextUsage,
    checked_label, kind_of,
};

/// The longest operation key, in bytes.
pub const MAXIMUM_OPERATION_KEY_BYTES: usize = MAXIMUM_LABEL_BYTES;

/// Why an operation key or scope was refused.
pub type OperationKeyError = LabelError;

/// The namespace an operation lives in: a tenant, campaign or local run, as the owner names it.
/// Equal operation ids in different scopes never share an attempt.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AttemptScope(String);

impl AttemptScope {
    /// A scope name follows the label rules ([`LabelError`]).
    pub fn new(value: &str) -> Result<Self, LabelError> {
        checked_label(value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The caller's name for one logical request inside a scope. Reusing a key with the same request
/// replays one attempt; reusing it with a different request is a conflict.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OperationKey {
    scope: AttemptScope,
    operation: String,
}

impl OperationKey {
    /// An operation id follows the label rules ([`LabelError`]).
    pub fn new(scope: &AttemptScope, operation: &str) -> Result<Self, LabelError> {
        Ok(Self {
            scope: scope.clone(),
            operation: checked_label(operation)?,
        })
    }

    pub fn scope(&self) -> &AttemptScope {
        &self.scope
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }
}

/// The process that holds a dispatch marker. Composition supplies a fresh value per process
/// start. The all-zero value is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptOwner([u8; 16]);

impl AttemptOwner {
    /// `None` for the all-zero value, which would look like an unset owner.
    pub fn new(value: [u8; 16]) -> Option<Self> {
        (value != [0; 16]).then_some(Self(value))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// A SHA-256 digest of every field that determines the answer.
pub type Fingerprint = [u8; 32];

/// The fingerprint of a request. Changing any field, including the profile, changes it.
pub fn fingerprint(request: &TextRequest) -> Fingerprint {
    let mut hasher = Sha256::new();
    absorb(&mut hasher, request.purpose.key().as_bytes());
    absorb(&mut hasher, request.profile.as_str().as_bytes());
    absorb(&mut hasher, request.output_name.as_bytes());
    absorb(&mut hasher, request.instructions.as_bytes());
    absorb(&mut hasher, request.facts.to_string().as_bytes());
    absorb(&mut hasher, request.output_schema.to_string().as_bytes());
    absorb(&mut hasher, &request.deadline.as_nanos().to_be_bytes());
    // Pictures by label, type and content digest: a different picture is a different request.
    // A text-only request absorbs nothing more, so its fingerprint is the one stored before
    // pictures existed and its recorded attempts still replay.
    if !request.images.is_empty() {
        absorb(&mut hasher, &length_prefix(request.images.len()));
    }
    for image in &request.images {
        absorb(&mut hasher, image.label.as_bytes());
        absorb(&mut hasher, image.media_type.mime().as_bytes());
        absorb(&mut hasher, &Sha256::digest(&image.bytes));
    }
    hasher.finalize().into()
}

/// Length-prefixed fields keep distinct field splits from producing the same input bytes.
fn absorb(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(length_prefix(bytes.len()));
    hasher.update(bytes);
}

/// A length as eight big-endian bytes. Every supported target has a `usize` of at most 64 bits,
/// so the saturating fallback is never taken.
fn length_prefix(length: usize) -> [u8; 8] {
    u64::try_from(length).unwrap_or(u64::MAX).to_be_bytes()
}

/// A digest of the owner's current basis for one operation: the revisions the answer depends on.
pub type Basis = [u8; 32];

/// The stored phase of an attempt, without its payload. Used by inspection surfaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptPhase {
    Accepted,
    Dispatched,
    Candidate,
    Settled,
    Rejected,
    Unknown,
    Cancelled,
}

impl AttemptPhase {
    /// Every phase, in transition order.
    pub const ALL: [Self; 7] = [
        Self::Accepted,
        Self::Dispatched,
        Self::Candidate,
        Self::Settled,
        Self::Rejected,
        Self::Unknown,
        Self::Cancelled,
    ];

    /// The stored key; the PostgreSQL `phase` CHECK lists the same values.
    pub fn key(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Dispatched => "dispatched",
            Self::Candidate => "candidate",
            Self::Settled => "settled",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
        }
    }

    /// The inverse of [`Self::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|phase| phase.key() == key)
    }
}

/// The kind of a settled outcome, without its payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Answered,
    Unavailable,
    Deadline,
    Invalid,
    Stale,
}

impl OutcomeKind {
    /// Every outcome kind, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Answered,
        Self::Unavailable,
        Self::Deadline,
        Self::Invalid,
        Self::Stale,
    ];

    /// The stored key; the PostgreSQL `outcome_kind` CHECK lists the same values.
    pub fn key(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Unavailable => "unavailable",
            Self::Deadline => "deadline",
            Self::Invalid => "invalid",
            Self::Stale => "stale",
        }
    }

    /// The inverse of [`Self::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// What the store has durably recorded for one operation.
#[derive(Clone, PartialEq)]
pub enum AttemptState {
    /// Admitted; no send marker exists, so nothing can have been sent.
    Accepted,
    /// A send may have started under this owner and no result is recorded.
    Dispatched { owner: AttemptOwner },
    /// The backend answered and the answer passed its structural check. It is private to the
    /// feature until the feature qualifies it.
    Candidate {
        answer: Value,
        usage: Option<TextUsage>,
    },
    /// A reusable result: a qualified answer, or an outcome that is not an answer.
    Settled {
        outcome: TextOutcome,
        usage: Option<TextUsage>,
    },
    /// The feature or an operator rejected the answer. It is never reusable.
    Rejected { usage: Option<TextUsage> },
    /// A send was recorded and its result was lost. Never resent.
    Unknown,
    /// Cancelled before any send.
    Cancelled,
}

/// Debug withholds a candidate's answer: it is model output and may carry private game facts.
impl fmt::Debug for AttemptState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accepted => formatter.write_str("Accepted"),
            Self::Dispatched { owner } => formatter
                .debug_struct("Dispatched")
                .field("owner", owner)
                .finish(),
            Self::Candidate { usage, .. } => formatter
                .debug_struct("Candidate")
                .field("answer", &"<withheld>")
                .field("usage", usage)
                .finish(),
            Self::Settled { outcome, usage } => formatter
                .debug_struct("Settled")
                .field("outcome", outcome)
                .field("usage", usage)
                .finish(),
            Self::Rejected { usage } => formatter
                .debug_struct("Rejected")
                .field("usage", usage)
                .finish(),
            Self::Unknown => formatter.write_str("Unknown"),
            Self::Cancelled => formatter.write_str("Cancelled"),
        }
    }
}

/// A stored attempt, as the store reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredAttempt {
    pub fingerprint: Fingerprint,
    pub state: AttemptState,
}

/// Whether admission created the attempt or found one already stored.
#[derive(Clone, Debug, PartialEq)]
pub enum Admission {
    Created,
    Existing(StoredAttempt),
}

/// Store failures. Every failure before a send leaves nothing to resend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptStoreError {
    /// The store could not be reached.
    Unreachable,
    /// The store did not answer in time.
    TimedOut,
    /// The store answered with an error for this statement.
    Rejected,
    /// A stored record or configuration does not match what this code writes or allows.
    Invalid,
}

impl AttemptStoreError {
    /// The closed cause key for logs; carries no connection or query detail.
    pub fn key(self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::TimedOut => "timed_out",
            Self::Rejected => "rejected",
            Self::Invalid => "invalid",
        }
    }
}

impl fmt::Display for AttemptStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unreachable => "the text attempt store could not be reached",
            Self::TimedOut => "the text attempt store did not answer in time",
            Self::Rejected => "the text attempt store rejected the statement",
            Self::Invalid => "a stored text attempt does not match what this code allows",
        })
    }
}

impl std::error::Error for AttemptStoreError {}

/// The facts an attempt is admitted with. The fingerprint decides replay versus conflict.
pub struct AttemptMeta<'a> {
    pub purpose: TextPurpose,
    pub profile: &'a TextProfile,
    pub output_name: &'a str,
    pub fingerprint: &'a Fingerprint,
}

/// The feature's semantic verdict on a candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Accepted,
    Rejected,
}

/// What the store did with a verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualifyResult {
    Applied,
    /// The attempt holds no candidate: it was never answered, or the verdict already landed.
    NotCandidate,
    /// The owner's basis moved after dispatch. The candidate was discarded and recorded Stale.
    BasisMoved,
}

/// The durable record port. Each transition is one atomic compare-and-set on the stated state;
/// `Ok(false)` means the record was in another state and nothing changed.
pub trait TextAttemptStore: Send + Sync {
    /// The attempt under `key`, if any. Reads only.
    fn lookup(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<Option<StoredAttempt>, AttemptStoreError>> + Send;

    /// Creates an accepted attempt when the key is new. Never changes an existing record.
    fn admit(
        &self,
        key: &OperationKey,
        meta: &AttemptMeta<'_>,
    ) -> impl Future<Output = Result<Admission, AttemptStoreError>> + Send;

    /// Accepted → Dispatched(owner), recording the basis the send was authorised under.
    fn begin_dispatch(
        &self,
        key: &OperationKey,
        owner: AttemptOwner,
        basis: Basis,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send;

    /// Dispatched → Candidate. The answer stays private until a verdict.
    fn record_candidate(
        &self,
        key: &OperationKey,
        answer: &Value,
        usage: Option<TextUsage>,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send;

    /// Dispatched → Settled for an outcome that is not an answer. A discarded answer keeps its
    /// usage: it was still paid for.
    fn settle(
        &self,
        key: &OperationKey,
        outcome: &TextOutcome,
        usage: Option<TextUsage>,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send;

    /// Candidate → Settled (accepted) or Rejected, if the owner still holds the recorded basis;
    /// otherwise Candidate → Settled(Stale) in the same step.
    fn qualify(
        &self,
        key: &OperationKey,
        verdict: Verdict,
        basis_now: Option<Basis>,
    ) -> impl Future<Output = Result<QualifyResult, AttemptStoreError>> + Send;

    /// Settled(answer) → Rejected: withdraws a reusable answer.
    fn invalidate(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send;

    /// Dispatched → Unknown. Used when the owner that dispatched is no longer this process, or the
    /// backend could not say whether its request was sent.
    fn mark_unknown(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send;

    /// Accepted → Cancelled.
    fn cancel(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send;
}

impl<T: TextAttemptStore> TextAttemptStore for Arc<T> {
    fn lookup(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<Option<StoredAttempt>, AttemptStoreError>> + Send {
        (**self).lookup(key)
    }

    fn admit(
        &self,
        key: &OperationKey,
        meta: &AttemptMeta<'_>,
    ) -> impl Future<Output = Result<Admission, AttemptStoreError>> + Send {
        (**self).admit(key, meta)
    }

    fn begin_dispatch(
        &self,
        key: &OperationKey,
        owner: AttemptOwner,
        basis: Basis,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        (**self).begin_dispatch(key, owner, basis)
    }

    fn record_candidate(
        &self,
        key: &OperationKey,
        answer: &Value,
        usage: Option<TextUsage>,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        (**self).record_candidate(key, answer, usage)
    }

    fn settle(
        &self,
        key: &OperationKey,
        outcome: &TextOutcome,
        usage: Option<TextUsage>,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        (**self).settle(key, outcome, usage)
    }

    fn qualify(
        &self,
        key: &OperationKey,
        verdict: Verdict,
        basis_now: Option<Basis>,
    ) -> impl Future<Output = Result<QualifyResult, AttemptStoreError>> + Send {
        (**self).qualify(key, verdict, basis_now)
    }

    fn invalidate(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        (**self).invalidate(key)
    }

    fn mark_unknown(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        (**self).mark_unknown(key)
    }

    fn cancel(
        &self,
        key: &OperationKey,
    ) -> impl Future<Output = Result<bool, AttemptStoreError>> + Send {
        (**self).cancel(key)
    }
}

/// What a submit returned. Exactly one of these comes back for each accepted submit.
#[derive(Clone, PartialEq)]
pub enum Submission {
    /// A reusable result: the backend's answer was qualified earlier (`replayed: true`), or the
    /// backend returned an outcome that is not an answer. `replayed: false` means this call made
    /// it.
    Settled {
        outcome: TextOutcome,
        replayed: bool,
        usage: Option<TextUsage>,
    },
    /// A structurally valid answer the feature has not qualified yet. It is not a cache hit: the
    /// feature must validate it and call `qualify` before relying on it.
    Candidate {
        answer: Value,
        replayed: bool,
        usage: Option<TextUsage>,
    },
    /// The feature or an operator rejected the answer. It is not reusable and is never regenerated
    /// under this key.
    Rejected { usage: Option<TextUsage> },
    /// Another live call owns this operation. Nothing was sent by this call; retry later.
    InFlight,
    /// A send was recorded but its result was not. Never resent.
    Unknown,
    /// Cancelled before any send.
    Cancelled,
}

impl fmt::Debug for Submission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settled {
                outcome,
                replayed,
                usage,
            } => formatter
                .debug_struct("Settled")
                .field("outcome", outcome)
                .field("replayed", replayed)
                .field("usage", usage)
                .finish(),
            Self::Candidate {
                replayed, usage, ..
            } => formatter
                .debug_struct("Candidate")
                .field("answer", &"<withheld>")
                .field("replayed", replayed)
                .field("usage", usage)
                .finish(),
            Self::Rejected { usage } => formatter
                .debug_struct("Rejected")
                .field("usage", usage)
                .finish(),
            Self::InFlight => formatter.write_str("InFlight"),
            Self::Unknown => formatter.write_str("Unknown"),
            Self::Cancelled => formatter.write_str("Cancelled"),
        }
    }
}

/// What a verdict did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Qualification {
    Qualified,
    Rejected,
    /// The attempt holds no candidate.
    NotACandidate,
    /// The owner's basis moved after dispatch; the candidate was discarded.
    Stale,
}

/// Why a call returned an error. Errors before dispatch never reached the backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableTextError {
    Refused(TextRequestError),
    Conflict,
    Store(AttemptStoreError),
    /// The owner does not hold this operation's basis now. Nothing was sent.
    Unauthorized,
    /// No attempt exists under this key.
    UnknownOperation,
}

impl fmt::Display for DurableTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => write!(formatter, "the text request was refused: {error}"),
            Self::Conflict => formatter.write_str("the operation key was used for another request"),
            Self::Store(error) => write!(formatter, "the text attempt store failed: {error}"),
            Self::Unauthorized => {
                formatter.write_str("the owner does not hold this operation's basis")
            }
            Self::UnknownOperation => formatter.write_str("no text attempt exists under this key"),
        }
    }
}

impl std::error::Error for DurableTextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(error) => Some(error),
            Self::Store(error) => Some(error),
            Self::Conflict | Self::Unauthorized | Self::UnknownOperation => None,
        }
    }
}

/// The owner's authority over one operation. The owner, not the feature, decides whether the
/// operation still has a basis. The service asks before it reads or writes the store, again
/// immediately before dispatch, and again after the backend answers, so a revoked or moved basis
/// never leaves a reusable answer behind.
pub trait TextAuthority: Send + Sync {
    /// The basis the owner holds now for `key`, or `None` when the owner no longer holds it.
    fn current_basis(&self, key: &OperationKey) -> impl Future<Output = Option<Basis>> + Send;
}

impl<T: TextAuthority> TextAuthority for Arc<T> {
    fn current_basis(&self, key: &OperationKey) -> impl Future<Output = Option<Basis>> + Send {
        (**self).current_basis(key)
    }
}

/// The text service with durable attempts. The owner token marks dispatches made by this process.
/// This is the only path to a backend: a `TextService` alone has no way to run a request.
pub struct DurableTextService<B, S, A, O = NoTextObserver> {
    text: TextService<B, O>,
    store: S,
    authority: A,
    owner: AttemptOwner,
}

impl<B: TextBackend, S: TextAttemptStore, A: TextAuthority, O: TextObserver>
    DurableTextService<B, S, A, O>
{
    /// `owner` must be fresh per process start, so a dispatch left by a crashed process is never
    /// mistaken for this one's in-flight call.
    pub fn new(text: TextService<B, O>, store: S, authority: A, owner: AttemptOwner) -> Self {
        Self {
            text,
            store,
            authority,
            owner,
        }
    }

    /// Serves one logical request under `key`. Refused requests are never admitted. Reports
    /// exactly one event for each submit that is accepted or refused.
    pub async fn submit(
        &self,
        key: &OperationKey,
        request: &TextRequest,
    ) -> Result<Submission, DurableTextError> {
        let started = Instant::now();
        if let Err(error) = self.text.check(request) {
            self.text
                .report(Some(key), request, TextEventKind::Refused(error), started);
            return Err(DurableTextError::Refused(error));
        }
        // Authority comes first: a request the owner does not hold never reads or writes the store.
        let Some(basis) = self.authority.current_basis(key).await else {
            self.text
                .report(Some(key), request, TextEventKind::Unauthorized, started);
            return Err(DurableTextError::Unauthorized);
        };
        let fingerprint = fingerprint(request);
        let meta = AttemptMeta {
            purpose: request.purpose,
            profile: &request.profile,
            output_name: &request.output_name,
            fingerprint: &fingerprint,
        };
        let stored = match self.store.admit(key, &meta).await {
            Ok(Admission::Created) => return self.dispatch(key, request, basis, started).await,
            Ok(Admission::Existing(stored)) => stored,
            Err(error) => return Err(self.store_failed(key, request, error, started)),
        };
        if stored.fingerprint != fingerprint {
            self.text
                .report(Some(key), request, TextEventKind::Conflict, started);
            return Err(DurableTextError::Conflict);
        }
        self.resume(key, request, stored.state, basis, started)
            .await
    }

    /// Records the feature's verdict on a candidate. The request must be the one that produced
    /// the attempt, so a verdict can only land on the exact request it was made for.
    pub async fn qualify(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        verdict: Verdict,
    ) -> Result<Qualification, DurableTextError> {
        let started = Instant::now();
        self.match_attempt(key, request, started).await?;
        // A basis the owner no longer holds is not an error here: the store records Stale.
        let basis_now = self.authority.current_basis(key).await;
        match self.store.qualify(key, verdict, basis_now).await {
            Ok(QualifyResult::Applied) => {
                let (kind, result) = match verdict {
                    Verdict::Accepted => (TextEventKind::Qualified, Qualification::Qualified),
                    Verdict::Rejected => (TextEventKind::Rejected, Qualification::Rejected),
                };
                self.text.report(Some(key), request, kind, started);
                Ok(result)
            }
            Ok(QualifyResult::NotCandidate) => {
                self.text
                    .report(Some(key), request, TextEventKind::NotCandidate, started);
                Ok(Qualification::NotACandidate)
            }
            Ok(QualifyResult::BasisMoved) => {
                self.text
                    .report(Some(key), request, TextEventKind::Stale, started);
                Ok(Qualification::Stale)
            }
            Err(error) => Err(self.store_failed(key, request, error, started)),
        }
    }

    /// Withdraws a reusable answer, for example when the feature later finds it wrong. `Ok(false)`
    /// means there was no reusable answer to withdraw.
    pub async fn invalidate(
        &self,
        key: &OperationKey,
        request: &TextRequest,
    ) -> Result<bool, DurableTextError> {
        let started = Instant::now();
        self.match_attempt(key, request, started).await?;
        match self.store.invalidate(key).await {
            Ok(changed) => {
                if changed {
                    self.text
                        .report(Some(key), request, TextEventKind::Invalidated, started);
                }
                Ok(changed)
            }
            Err(error) => Err(self.store_failed(key, request, error, started)),
        }
    }

    /// Cancels an operation that has not dispatched. `Ok(false)` means it dispatched, settled or
    /// was never admitted, so nothing changed. Cancellation reports no event; the next submit
    /// under the key reports `Cancelled`.
    pub async fn cancel(&self, key: &OperationKey) -> Result<bool, DurableTextError> {
        self.store
            .cancel(key)
            .await
            .map_err(DurableTextError::Store)
    }

    async fn resume(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        state: AttemptState,
        basis: Basis,
        started: Instant,
    ) -> Result<Submission, DurableTextError> {
        match state {
            AttemptState::Accepted => self.dispatch(key, request, basis, started).await,
            AttemptState::Dispatched { owner } if owner == self.owner => {
                self.text
                    .report(Some(key), request, TextEventKind::InFlight, started);
                Ok(Submission::InFlight)
            }
            AttemptState::Dispatched { .. } => self.lost(key, request, started).await,
            AttemptState::Candidate { answer, usage } => {
                self.text
                    .report(Some(key), request, TextEventKind::Replayed, started);
                Ok(Submission::Candidate {
                    answer,
                    replayed: true,
                    usage,
                })
            }
            AttemptState::Settled { outcome, usage } => {
                self.text
                    .report(Some(key), request, TextEventKind::Replayed, started);
                Ok(Submission::Settled {
                    outcome,
                    replayed: true,
                    usage,
                })
            }
            AttemptState::Rejected { usage } => {
                self.text
                    .report(Some(key), request, TextEventKind::Rejected, started);
                Ok(Submission::Rejected { usage })
            }
            AttemptState::Unknown => {
                self.text
                    .report(Some(key), request, TextEventKind::Unknown, started);
                Ok(Submission::Unknown)
            }
            AttemptState::Cancelled => {
                self.text
                    .report(Some(key), request, TextEventKind::Cancelled, started);
                Ok(Submission::Cancelled)
            }
        }
    }

    async fn dispatch(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        basis: Basis,
        started: Instant,
    ) -> Result<Submission, DurableTextError> {
        // The last look before the send: the store round trips above can outlast the basis.
        if self.authority.current_basis(key).await != Some(basis) {
            self.text
                .report(Some(key), request, TextEventKind::Unauthorized, started);
            return Err(DurableTextError::Unauthorized);
        }
        match self.store.begin_dispatch(key, self.owner, basis).await {
            Ok(true) => {}
            Ok(false) => {
                self.text
                    .report(Some(key), request, TextEventKind::InFlight, started);
                return Ok(Submission::InFlight);
            }
            Err(error) => return Err(self.store_failed(key, request, error, started)),
        }
        let receipt = self.text.call_backend(request).await;
        let usage = receipt.usage;
        if receipt.outcome == TextOutcome::Unknown {
            return self.lost(key, request, started).await;
        }
        // The basis may have moved while the request was in flight. A moved basis discards the
        // answer: it is recorded as Stale, never stored as a candidate.
        let held = self.authority.current_basis(key).await == Some(basis);
        let outcome = if held {
            receipt.outcome
        } else {
            TextOutcome::Stale
        };
        let recorded = match &outcome {
            TextOutcome::Answered(answer) => self.store.record_candidate(key, answer, usage).await,
            other => self.store.settle(key, other, usage).await,
        };
        match (recorded, outcome) {
            (Ok(true), TextOutcome::Answered(answer)) => {
                self.text
                    .report(Some(key), request, TextEventKind::Answered, started);
                Ok(Submission::Candidate {
                    answer,
                    replayed: false,
                    usage,
                })
            }
            (Ok(true), outcome) => {
                self.text
                    .report(Some(key), request, kind_of(&outcome), started);
                Ok(Submission::Settled {
                    outcome,
                    replayed: false,
                    usage,
                })
            }
            // The answer exists but its record does not: the honest durable state is Unknown.
            (Ok(false) | Err(_), _) => {
                self.text
                    .report(Some(key), request, TextEventKind::Unknown, started);
                Ok(Submission::Unknown)
            }
        }
    }

    /// A dispatch whose result is not recorded: another owner's, or one the backend could not
    /// account for. It resolves to Unknown and is never resent.
    async fn lost(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        started: Instant,
    ) -> Result<Submission, DurableTextError> {
        match self.store.mark_unknown(key).await {
            Ok(true) => {
                self.text
                    .report(Some(key), request, TextEventKind::Unknown, started);
                Ok(Submission::Unknown)
            }
            // Another caller changed the record first; its state decides the next submit.
            Ok(false) => {
                self.text
                    .report(Some(key), request, TextEventKind::InFlight, started);
                Ok(Submission::InFlight)
            }
            Err(error) => Err(self.store_failed(key, request, error, started)),
        }
    }

    /// The stored attempt under `key` must exist and have been made for this exact request.
    async fn match_attempt(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        started: Instant,
    ) -> Result<(), DurableTextError> {
        if let Err(error) = self.text.check(request) {
            self.text
                .report(Some(key), request, TextEventKind::Refused(error), started);
            return Err(DurableTextError::Refused(error));
        }
        match self.store.lookup(key).await {
            Ok(Some(stored)) if stored.fingerprint == fingerprint(request) => Ok(()),
            Ok(Some(_)) => {
                self.text
                    .report(Some(key), request, TextEventKind::Conflict, started);
                Err(DurableTextError::Conflict)
            }
            Ok(None) => {
                self.text
                    .report(Some(key), request, TextEventKind::NotCandidate, started);
                Err(DurableTextError::UnknownOperation)
            }
            Err(error) => Err(self.store_failed(key, request, error, started)),
        }
    }

    fn store_failed(
        &self,
        key: &OperationKey,
        request: &TextRequest,
        error: AttemptStoreError,
        started: Instant,
    ) -> DurableTextError {
        self.text.report(
            Some(key),
            request,
            TextEventKind::StoreFailed(error),
            started,
        );
        DurableTextError::Store(error)
    }
}

#[cfg(test)]
mod text_attempts_tests;
