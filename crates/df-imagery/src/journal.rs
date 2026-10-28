//! Scoped operation receipts and shared imagery jobs: the durable memory of image demand.
//!
//! This module owns the neutral types and the persistence port. It performs no provider calls,
//! pixel work, storage, scheduling, authorization or money movement. Every scope, pin and
//! fingerprint here is a caller claim; the later image service authorizes the caller, reference
//! and audience before calling this store, and this store still enforces exact stored-owner
//! equality on every access so one scope can never read or extend another's rows.
//!
//! An operation is one caller intent under one owner scope. The first operation creates a job;
//! later compatible operations join it. A job moves `Planning` to `RecipeFrozen` through one
//! compare-and-set freeze, and to `Retired` when its last interest withdraws. Receipts and
//! frozen facts are never deleted or rewritten.

use std::fmt;
use std::future::Future;

use df_assets::MediaRequestKey;
use df_types::{AccountId, CampaignId, OperationId, RunId};

pub use crate::request::{ImageOwner, OperationIdentity};

/// Version tag of the canonical owner-scope encoding below.
pub const OWNER_SCOPE_VERSION: u8 = 1;

/// Why supplied identity bytes were refused. Identities are caller-supplied, never minted here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalIdError {
    InvalidLength { actual: usize },
    InvalidHex,
    Zero,
}

macro_rules! journal_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            /// Accepts exactly 16 bytes with at least one nonzero byte.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, JournalIdError> {
                let value: [u8; 16] =
                    bytes.try_into().map_err(|_| JournalIdError::InvalidLength {
                        actual: bytes.len(),
                    })?;
                if value == [0; 16] {
                    return Err(JournalIdError::Zero);
                }
                Ok(Self(value))
            }

            /// Parses exactly 32 lowercase hexadecimal digits.
            pub fn from_hex(text: &str) -> Result<Self, JournalIdError> {
                let encoded = text.as_bytes();
                if encoded.len() != 32 {
                    return Err(JournalIdError::InvalidLength {
                        actual: encoded.len(),
                    });
                }
                let mut value = [0_u8; 16];
                for index in 0..16 {
                    let high = hex_nibble(encoded[index * 2]);
                    let low = hex_nibble(encoded[index * 2 + 1]);
                    match (high, low) {
                        (Some(high), Some(low)) => value[index] = (high << 4) | low,
                        _ => return Err(JournalIdError::InvalidHex),
                    }
                }
                if value == [0; 16] {
                    return Err(JournalIdError::Zero);
                }
                Ok(Self(value))
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }

            /// Lowercase hexadecimal, the only text form used in diagnostics.
            pub fn to_hex(&self) -> String {
                let mut text = String::with_capacity(32);
                for byte in self.0 {
                    text.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('?'));
                    text.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('?'));
                }
                text
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({})", stringify!($name), self.to_hex())
            }
        }
    };
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

journal_id!(
    /// One shared imagery job: the durable anchor compatible operations join.
    ImageJobId
);
journal_id!(
    /// One caller's stake in a job, bound to exactly one operation receipt.
    ImageInterestId
);

/// Why an immutable payload was refused before any durability work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    ZeroRevision,
    Empty,
    TooLarge { len_bytes: usize, max_bytes: usize },
}

/// One exact opaque canonical payload at a nonzero format revision: the intent the service
/// hashed into the fingerprint, or the configuration the job ran under. The bytes are opaque
/// here; this constructor never parses, authenticates or completes them. `Debug` reports only
/// the revision and length: payloads may carry private intent facts and never reach logs.
#[derive(Clone, Eq, PartialEq)]
pub struct VersionedSnapshot {
    revision: u32,
    bytes: Box<[u8]>,
}

impl VersionedSnapshot {
    /// The revision selects the canonical encoding; the byte limit is injected by composition.
    pub fn new(revision: u32, bytes: Vec<u8>, max_bytes: usize) -> Result<Self, SnapshotError> {
        if revision == 0 {
            return Err(SnapshotError::ZeroRevision);
        }
        if bytes.is_empty() {
            return Err(SnapshotError::Empty);
        }
        if bytes.len() > max_bytes {
            return Err(SnapshotError::TooLarge {
                len_bytes: bytes.len(),
                max_bytes,
            });
        }
        Ok(Self {
            revision,
            bytes: bytes.into_boxed_slice(),
        })
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for VersionedSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VersionedSnapshot")
            .field("revision", &self.revision)
            .field("len_bytes", &self.bytes.len())
            .finish()
    }
}

/// The 32-byte opaque sharing-policy digest the future service computes over source, audience,
/// lifetime and execution compatibility. Never a media request key: equal pictures under
/// different policies must not share a job.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SharingIdentity([u8; 32]);

impl SharingIdentity {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, JournalIdError> {
        let value: [u8; 32] = bytes
            .try_into()
            .map_err(|_| JournalIdError::InvalidLength {
                actual: bytes.len(),
            })?;
        Ok(Self(value))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Canonical owner bytes: version tag, owner tag, then the exact campaign/run or account ids.
/// Never `Debug` or display text: formatting is not canonical and would merge distinct owners.
pub fn owner_scope_bytes(owner: &ImageOwner) -> Vec<u8> {
    match owner {
        ImageOwner::Run { campaign, run } => {
            let mut bytes = Vec::with_capacity(34);
            bytes.push(OWNER_SCOPE_VERSION);
            bytes.push(0);
            bytes.extend_from_slice(campaign.as_bytes());
            bytes.extend_from_slice(run.as_bytes());
            bytes
        }
        ImageOwner::Draft { account } => {
            let mut bytes = Vec::with_capacity(18);
            bytes.push(OWNER_SCOPE_VERSION);
            bytes.push(1);
            bytes.extend_from_slice(account.as_bytes());
            bytes
        }
    }
}

/// Parses canonical owner bytes back into the typed owner; anything else is an invalid record.
pub fn owner_of_bytes(bytes: &[u8]) -> Option<ImageOwner> {
    if bytes.len() < 2 || bytes[0] != OWNER_SCOPE_VERSION {
        return None;
    }
    match (bytes[1], bytes.len()) {
        (0, 34) => {
            let campaign = CampaignId::from_bytes(&bytes[2..18]).ok()?;
            let run = RunId::from_bytes(&bytes[18..34]).ok()?;
            Some(ImageOwner::Run { campaign, run })
        }
        (1, 18) => {
            let account = AccountId::from_bytes(&bytes[2..18]).ok()?;
            Some(ImageOwner::Draft { account })
        }
        _ => None,
    }
}

/// A job is planned until the first recipe freeze, then frozen; the last withdrawal retires it
/// while preserving every frozen fact. There is no ready, publication or finance state here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobState {
    Planning,
    RecipeFrozen,
    Retired,
}

impl JobState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::RecipeFrozen => "recipe_frozen",
            Self::Retired => "retired",
        }
    }

    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "planning" => Some(Self::Planning),
            "recipe_frozen" => Some(Self::RecipeFrozen),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }
}

/// A caller's stake in a job: attached until withdrawn. A withdrawn row stays recorded; replay
/// of its operation returns it as-is and never re-attaches it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterestState {
    Attached,
    Withdrawn,
}

impl InterestState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Attached => "attached",
            Self::Withdrawn => "withdrawn",
        }
    }

    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "attached" => Some(Self::Attached),
            "withdrawn" => Some(Self::Withdrawn),
            _ => None,
        }
    }
}

/// The stored job: exact immutable snapshots, the absolute deadline, the compare-and-set
/// revision and the frozen recipe once set. Reads return these facts; nothing is invented.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageJob {
    pub job: ImageJobId,
    pub owner: ImageOwner,
    pub sharing: SharingIdentity,
    pub intent: VersionedSnapshot,
    pub config: VersionedSnapshot,
    pub absolute_deadline_ms: i64,
    pub state: JobState,
    pub revision: u64,
    pub resolved_key: Option<MediaRequestKey>,
    pub recipe: Option<VersionedSnapshot>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// The immutable receipt binding one scoped operation to its job and interest. Many operations
/// may reference one job; different owner scopes may use identical operation bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationReceipt {
    pub owner: ImageOwner,
    pub operation: OperationId,
    pub fingerprint: [u8; 32],
    pub job: ImageJobId,
    pub interest: ImageInterestId,
}

/// The stored interest row: which operation it belongs to and whether it still counts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageInterest {
    pub interest: ImageInterestId,
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub operation: OperationId,
    pub state: InterestState,
    pub created_at_ms: i64,
    pub withdrawn_at_ms: Option<i64>,
}

/// A receipt with its current job and interest, read as one coherent snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationView {
    pub receipt: OperationReceipt,
    pub job: ImageJob,
    pub interest: ImageInterest,
}

/// Where new demand goes: a fresh job under a fresh deadline, or an existing exact-owner job.
/// There is no fallback: a refused join never creates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AcceptTarget {
    Create {
        job: ImageJobId,
        absolute_deadline_ms: i64,
    },
    Join {
        job: ImageJobId,
    },
}

/// One atomic receipt lookup-or-create plus interest. All ids, snapshots and times are injected;
/// nothing reads the environment, clock entropy or a provider.
#[derive(Clone, Debug, PartialEq)]
pub struct AcceptCommand {
    pub identity: OperationIdentity,
    pub intent: VersionedSnapshot,
    pub config: VersionedSnapshot,
    pub sharing: SharingIdentity,
    pub interest: ImageInterestId,
    pub target: AcceptTarget,
    pub now_ms: i64,
}

/// Accepting demand either replays the stored view without touching it, or records new rows.
#[derive(Clone, Debug, PartialEq)]
pub enum AcceptOutcome {
    Replayed(OperationView),
    Created(OperationView),
    Joined(OperationView),
}

impl AcceptOutcome {
    pub fn view(&self) -> &OperationView {
        match self {
            Self::Replayed(view) | Self::Created(view) | Self::Joined(view) => view,
        }
    }
}

/// Compare-and-set freeze of the job's resolved recipe: the expected revision must match, the
/// frozen key and payload must equal the service's single canonical decision.
#[derive(Clone, Debug, PartialEq)]
pub struct FreezeCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub expected_revision: u64,
    pub key: MediaRequestKey,
    pub recipe: VersionedSnapshot,
    pub now_ms: i64,
}

/// A freeze either records the first decision or rereads the identical one.
#[derive(Clone, Debug, PartialEq)]
pub enum FreezeOutcome {
    Frozen { job: ImageJob },
    AlreadyFrozen { job: ImageJob },
}

/// One withdrawal with the job counters the caller needs next: interests still attached and the
/// job revision after the increment.
#[derive(Clone, Debug, PartialEq)]
pub struct Withdrawal {
    pub interest: ImageInterest,
    pub attached_remaining: u64,
    pub job_revision: u64,
}

/// Withdrawing twice is idempotent: the second call changes nothing and revives nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum WithdrawOutcome {
    Withdrawn(Withdrawal),
    AlreadyWithdrawn(Withdrawal),
}

/// Why a join cannot attach to a job. The caller picks another job or fresh demand; the store
/// never reroutes on its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinRefusal {
    Retired,
    Expired,
    SharingMismatch,
    IntentMismatch,
    ConfigMismatch,
}

/// Every way a journal call can fail. Callers branch on the variant: `Unavailable` and every
/// typed refusal changed nothing, while `Unknown` means the commit may have happened and the
/// caller must reread before retrying. No variant carries payload bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalError {
    /// Nothing was changed: unreachable, or the transaction failed before commit.
    Unavailable,
    /// The commit may or may not have taken effect; reread before any retry.
    Unknown,
    /// A stored row did not parse, or supplied values break a record invariant.
    InvalidRecord,
    /// The same scoped operation returns under a different fingerprint or intent.
    OperationConflict,
    /// A supplied job or interest id is already bound to different data; nothing was
    /// overwritten.
    IdentityConflict,
    /// The named job cannot take this operation; no fallback creation happened.
    JoinRefused(JoinRefusal),
    /// No such owned job, interest or operation.
    NotFound,
    /// The job already froze a different recipe; the first decision stands.
    RecipeConflict,
    /// The job moved on; retry against the returned revision.
    Stale { current_revision: u64 },
    /// The job retired; only an identical freeze replay still succeeds.
    Retired,
    /// The job deadline passed; only an identical freeze replay still succeeds.
    Expired,
}

/// True while a demand instant may still use the job: valid instants precede the deadline.
pub fn is_expired(absolute_deadline_ms: i64, now_ms: i64) -> bool {
    now_ms >= absolute_deadline_ms
}

/// The composition persistence port for scoped operations and recipe freezes. Every method is
/// one atomic transaction; queries are parameterized. Later catalog and finance composition may
/// call the transaction-taking statements directly to share one transaction.
pub trait ImageJournal: Send + Sync {
    /// Atomic receipt lookup-or-create plus interest: replay, create or explicit join.
    fn accept(
        &self,
        command: &AcceptCommand,
    ) -> impl Future<Output = Result<AcceptOutcome, JournalError>> + Send;

    /// The receipt with its current job and interest as one coherent snapshot, or `None`
    /// under a wrong owner or unknown operation. A wrong owner learns nothing.
    fn lookup_operation(
        &self,
        owner: &ImageOwner,
        operation: OperationId,
    ) -> impl Future<Output = Result<Option<OperationView>, JournalError>> + Send;

    /// The stored job with its retained snapshots, or `None` under a wrong owner.
    fn read_job(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
    ) -> impl Future<Output = Result<Option<ImageJob>, JournalError>> + Send;

    /// Planning at the expected revision becomes recipe-frozen; an identical frozen key and
    /// recipe rereads as frozen even after retirement or expiry.
    fn freeze_recipe(
        &self,
        command: &FreezeCommand,
    ) -> impl Future<Output = Result<FreezeOutcome, JournalError>> + Send;

    /// Attached becomes withdrawn and the job revision advances; the last withdrawal retires
    /// the job while preserving receipts and frozen facts.
    fn withdraw_interest(
        &self,
        owner: &ImageOwner,
        interest: ImageInterestId,
        now_ms: i64,
    ) -> impl Future<Output = Result<WithdrawOutcome, JournalError>> + Send;
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod journal_tests;
