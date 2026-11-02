//! The values a request and its answer are made of: keys, epochs, attempts, statuses, results and
//! the typed errors of asking.
//!
//! It owns plain data and the small rules on it (an epoch only moves forward, a status maps to the
//! `Readiness` of the port). It never schedules anything: `scheduler` does. A result holds model text as
//! `Narration`, whose `Debug` shows a length class only, so a request can be printed safely.

use std::fmt;

use df_table::event::FailureCause;
use df_table::identity::Correlation;
use df_table::ports::{PurposeName, Readiness, ResourceHandle};
use df_table::private_data::Narration;
use serde::{Deserialize, Serialize};

use super::purpose::QueueClass;

/// The identity of what a request would produce: the hash of its purpose, canonical inputs, model
/// and version, and scene epoch (H.8). Two requests with one key share one flight, so the second
/// never starts a second paid call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RequestKey(u64);

impl RequestKey {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    /// A key for a request that has no richer inputs than its purpose, the operation it serves
    /// and the epoch (what the `Broker` port knows). FNV-1a over those facts: stable across runs,
    /// and not a security boundary.
    pub fn of_operation(purpose: &PurposeName, correlation: &Correlation, epoch: Epoch) -> Self {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = OFFSET;
        let mut mix = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(PRIME);
            }
        };
        mix(purpose.as_str().as_bytes());
        mix(&[0]);
        mix(correlation.operation_id.as_bytes());
        mix(&epoch.get().to_le_bytes());
        Self(hash)
    }
}

/// The scene epoch a request was made for (RES42, K.15). A result for an epoch older than the
/// table's current one is rejected; a request for a later epoch is how look-ahead builds the next
/// scene early.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Epoch(u64);

impl Epoch {
    pub const FIRST: Self = Self(1);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One try at a request: the unit a lease, a provider call and a charge belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AttemptId(u64);

impl AttemptId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Who holds a lease: the worker that runs the provider call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkerId(u32);

impl WorkerId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// What a request asks for: a purpose, the key of what it would make, the epoch it is for and the
/// input it serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRequest {
    pub purpose: PurposeName,
    pub key: RequestKey,
    pub epoch: Epoch,
    pub correlation: Correlation,
}

/// What a finished call produced. Text is private model output; an image is described by its size
/// because the bytes live in the media store, never in the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceResult {
    Text(Narration),
    Image { width: u32, height: u32, bytes: u64 },
}

/// How a provider call failed. The broker maps it to a `FailureCause` and decides on a retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFault {
    /// The provider could not be reached or had nothing to give.
    Unavailable,
    /// The provider refused. A refusal is never retried with other wording or another model.
    Refused,
    /// Model text that is empty, over-long or the wrong shape (C.7).
    TextInvalid,
    /// An answer outside its size rule or of the wrong kind.
    InvalidAnswer,
}

impl ProviderFault {
    pub fn cause(self) -> FailureCause {
        match self {
            Self::Unavailable => FailureCause::Unavailable,
            Self::Refused => FailureCause::Refused,
            Self::TextInvalid => FailureCause::TextInvalid,
            Self::InvalidAnswer => FailureCause::InvalidAnswer,
        }
    }

    /// Whether the broker may try again: never for a refusal (J.4), and never for an answer that
    /// broke its size rule or kind, which another paid call would only repeat.
    pub fn is_retryable(self) -> bool {
        !matches!(self, Self::Refused | Self::InvalidAnswer)
    }
}

/// Where a request stands. It never holds a result: `take_result` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RequestStatus {
    Queued {
        class: QueueClass,
    },
    /// Waiting out a retry backoff; it is claimed again at `until_ms`.
    BackingOff {
        until_ms: u64,
    },
    InFlight {
        attempt: AttemptId,
    },
    Ready,
    Failed {
        cause: FailureCause,
    },
    /// Its last holder cancelled it.
    Cancelled,
    /// Its scene epoch passed before it finished, so its result is no longer wanted.
    Superseded,
    /// Its class queue was full and it was the oldest waiting request.
    Dropped,
}

impl RequestStatus {
    /// What the `Broker` port says: a request that is no longer wanted is `stale`, the closed
    /// cause for "a handle that is no longer current".
    pub fn readiness(self) -> Readiness {
        match self {
            Self::Queued { .. } | Self::BackingOff { .. } | Self::InFlight { .. } => {
                Readiness::Pending
            }
            Self::Ready => Readiness::Ready,
            Self::Failed { cause } => Readiness::Failed(cause),
            Self::Cancelled | Self::Superseded | Self::Dropped => {
                Readiness::Failed(FailureCause::Stale)
            }
        }
    }

    pub fn is_settled(self) -> bool {
        !matches!(
            self,
            Self::Queued { .. } | Self::BackingOff { .. } | Self::InFlight { .. }
        )
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Queued { .. } => "queued",
            Self::BackingOff { .. } => "backing_off",
            Self::InFlight { .. } => "in_flight",
            Self::Ready => "ready",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
            Self::Dropped => "dropped",
        }
    }
}

/// A request that settled, for the owner to turn into an input (readiness is pushed as well as
/// polled).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Completion {
    pub handle: ResourceHandle,
    pub readiness: Readiness,
}

/// Why a request was refused before it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    UnknownPurpose {
        purpose: PurposeName,
    },
    /// The request is for a scene epoch that has already passed.
    StaleEpoch {
        requested: Epoch,
        current: Epoch,
    },
    /// The live queue is full and the live class never drops.
    QueueFull {
        class: QueueClass,
    },
    /// The broker was closed with the table.
    Closed,
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPurpose { purpose } => {
                write!(formatter, "the purpose {purpose} is not registered")
            }
            Self::StaleEpoch { requested, current } => write!(
                formatter,
                "epoch {} has passed; the table is at epoch {}",
                requested.get(),
                current.get()
            ),
            Self::QueueFull { class } => {
                write!(formatter, "the {} queue is full", class.key())
            }
            Self::Closed => formatter.write_str("the broker is closed"),
        }
    }
}

impl std::error::Error for RequestError {}

/// Why a scene epoch could not advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochError {
    /// An epoch only moves forward.
    NotForward { current: Epoch, requested: Epoch },
}

impl fmt::Display for EpochError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotForward { current, requested } => write!(
                formatter,
                "epoch {} is not after the current epoch {}",
                requested.get(),
                current.get()
            ),
        }
    }
}

impl std::error::Error for EpochError {}

/// Why a result could not be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultError {
    /// The handle is unknown, or its request settled long enough ago to be forgotten.
    Unknown,
    /// The request has not finished.
    NotSettled,
    /// The request ended without a result, or the result was already taken.
    NoResult { status: RequestStatus },
}

impl fmt::Display for ResultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => formatter.write_str("that handle is not known"),
            Self::NotSettled => formatter.write_str("that request has not finished"),
            Self::NoResult { status } => {
                write!(formatter, "that request holds no result ({})", status.key())
            }
        }
    }
}

impl std::error::Error for ResultError {}
