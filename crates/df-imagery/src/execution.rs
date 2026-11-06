//! Neutral durable execution contracts: one logical stage, many dispatch attempts.
//!
//! This module owns the neutral types and the pure claim, sequence, capacity
//! and recovery decisions for image execution. It performs no provider calls,
//! implicit pixel work, storage, scheduling, authorization, finance writes or money
//! movement, and it never reads the clock, the environment or a credential.
//! Every scope, pin and fingerprint here is a caller claim; the service
//! authorizes the caller before calling the persistence port, which still
//! enforces exact stored-owner equality on every access.
//!
//! A stage is planned once per job with an immutable kind and input/config
//! basis plus the job-level planned publication identity. The stage row owns
//! the mutable claim triple (token, generation, lease deadline). Each send
//! is one dispatch row with a fresh id, an increasing sequence, immutable
//! retry lineage and an all-or-none finance tuple (local work carries none).
//! Dispatch rows own send, outcome and recovery state. Execution changes bump
//! the existing job revision in the same transaction so watchers observe them.
//!
//! The provider executor and the demand journal live elsewhere; this module
//! never dispatches bytes and never invents proof of live provider execution.

use std::fmt;

pub use crate::journal::ImageJobId;
use crate::request::ImageOwner;

mod claims;

pub use claims::{
    capacity_admits, claim_owns, completion_releases_capacity, lease_is_live, next_sequence,
    pending_admits, sender_may_complete,
};

pub mod port;

pub use port::{CapacityBounds, ExecutionGate, ImageExecutionJournal, QueuedStage};

mod decline;
pub use decline::{
    DeclineObservation, DeclineReceipt, DeclineRetryCommand, DeclineRetryPolicy, DeclineRoute,
    SubmitDeclineContract,
};

mod deferral;
pub use deferral::{DeferQueuedCommand, QueueDeferralNotice, QuotaDeferralReason};

mod queue;
pub use queue::{QueueCursor, QueuePage};

mod unclaimed;
pub use unclaimed::{
    ClaimedTerminal, DeferredTerminal, ReleasedTerminal, TerminalCursor, UnclaimedTerminal,
    UnsentTerminal,
};

mod unsent_projection;
pub use unsent_projection::AcknowledgeUnsentProjection;

mod unsent;
pub use unsent::UnsentPrepared;

mod recovery;

pub use recovery::{
    ClaimRecoveryCommand, FinishRecoveryCommand, ReconciliationCommand, ReconciliationDispatch,
    RecoveryDisposition, RecoveryLease, RecoveryStage,
};

mod decision;
pub use decision::{
    DEFAULT_DECISION_PAYLOAD_BYTES, DecisionBasis, DecisionKind, DecisionObservation,
    DecisionReceipt, DecisionReference, DecisionScope, ExecutionScope,
    MAXIMUM_SQL_DECISION_PAYLOAD_BYTES, ObservationReceipt, StageBasis,
};

mod construction;
pub use construction::{OutputImage, OutputObject, StageClaim, StageScope, SupplierResume};

mod candidate;
pub use candidate::{
    CANDIDATE_VALIDATOR_REVISION, CandidateDescriptor, CandidateError, CandidateObservation,
    CandidateReceipt,
};

/// Longest capacity scope text the durable bound table accepts.
pub const MAX_CAPACITY_SCOPE_BYTES: usize = 128;
/// Longest supplier correlation id the dispatch row accepts.
pub const MAX_SUPPLIER_JOB_ID_BYTES: usize = 256;
/// Longest staged object key the checkpoint row accepts.
pub const MAX_STORAGE_KEY_BYTES: usize = 512;
/// Maximum opaque version size, matching the existing asset object-store contract.
pub const MAXIMUM_OBJECT_VERSION_BYTES: usize = 1024;
/// Longest media type text the checkpoint row accepts.
pub const MAX_MIME_BYTES: usize = 128;
/// Longest stable book call id the dispatch and checkpoint rows accept.
pub const MAX_BOOK_CALL_ID_BYTES: usize = 80;
/// Longest supplier origin text a supplier checkpoint accepts.
pub const MAX_SUPPLIER_ORIGIN_BYTES: usize = 64;
/// Longest supplier model text a supplier checkpoint accepts.
pub const MAX_SUPPLIER_MODEL_BYTES: usize = 128;
/// Longest supplier checkpoint payload the journal retains for resume.
pub const MAX_SUPPLIER_CHECKPOINT_BYTES: usize = 1_048_576;

macro_rules! execution_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            /// Accepts exactly 16 bytes with at least one nonzero byte.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, ExecutionSetupError> {
                let value: [u8; 16] =
                    bytes.try_into().map_err(|_| ExecutionSetupError::ZeroId)?;
                if value == [0; 16] {
                    return Err(ExecutionSetupError::ZeroId);
                }
                Ok(Self(value))
            }

            /// Parses exactly 32 lowercase hexadecimal digits.
            pub fn from_hex(text: &str) -> Result<Self, ExecutionSetupError> {
                let encoded = text.as_bytes();
                if encoded.len() != 32 {
                    return Err(ExecutionSetupError::ZeroId);
                }
                let mut value = [0_u8; 16];
                for index in 0..16 {
                    let high = hex_nibble(encoded[index * 2]);
                    let low = hex_nibble(encoded[index * 2 + 1]);
                    match (high, low) {
                        (Some(high), Some(low)) => value[index] = (high << 4) | low,
                        _ => return Err(ExecutionSetupError::ZeroId),
                    }
                }
                if value == [0; 16] {
                    return Err(ExecutionSetupError::ZeroId);
                }
                Ok(Self(value))
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }

            /// Lowercase hexadecimal, the only text form used in diagnostics.
            pub fn to_hex(&self) -> String {
                crate::lower_hex(&self.0)
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

execution_id!(
    /// One logical stage of a job: planned once, then claimed and dispatched.
    ExecutionStageId
);
execution_id!(
    /// One send attempt of a stage: fresh per dispatch, never reused.
    DispatchId
);
execution_id!(
    /// One claim token: minted fresh per claim, never reused across stages.
    ClaimToken
);

/// Why a command was refused before any durability work. Nothing was written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionSetupError {
    ZeroId,
    Empty,
    TooLong { max_bytes: usize },
    BadTime,
    BadLease,
    BadRevision,
    ZeroDimension,
    ZeroBytes,
    PublicationIdentityMismatch,
}

/// What one logical stage produces. Closed so a new kind needs a contract change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageKind {
    Classification,
    Generation,
    Edit,
    Extraction,
    Inspection,
    Correction,
    LocalTransform,
    PhotoInspection,
    HeadLocations,
    Segmentation,
    StagingIdentity,
    StagingFacing,
}

impl StageKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Classification => "classification",
            Self::Generation => "generation",
            Self::Edit => "edit",
            Self::Extraction => "extraction",
            Self::Inspection => "inspection",
            Self::Correction => "correction",
            Self::LocalTransform => "local_transform",
            Self::PhotoInspection => "photo_inspection",
            Self::HeadLocations => "head_locations",
            Self::Segmentation => "segmentation",
            Self::StagingIdentity => "staging_identity",
            Self::StagingFacing => "staging_facing",
        }
    }

    pub fn is_standalone(self) -> bool {
        matches!(
            self,
            Self::PhotoInspection
                | Self::HeadLocations
                | Self::Segmentation
                | Self::StagingIdentity
                | Self::StagingFacing
        )
    }

    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "classification" => Some(Self::Classification),
            "generation" => Some(Self::Generation),
            "edit" => Some(Self::Edit),
            "extraction" => Some(Self::Extraction),
            "inspection" => Some(Self::Inspection),
            "correction" => Some(Self::Correction),
            "local_transform" => Some(Self::LocalTransform),
            "photo_inspection" => Some(Self::PhotoInspection),
            "head_locations" => Some(Self::HeadLocations),
            "segmentation" => Some(Self::Segmentation),
            "staging_identity" => Some(Self::StagingIdentity),
            "staging_facing" => Some(Self::StagingFacing),
            _ => None,
        }
    }
}

/// Claim lifecycle of a stage. Send and outcome state live on dispatch rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageState {
    Planned,
    Admitted,
    Released,
}

impl StageState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Admitted => "admitted",
            Self::Released => "released",
        }
    }

    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "planned" => Some(Self::Planned),
            "admitted" => Some(Self::Admitted),
            "released" => Some(Self::Released),
            _ => None,
        }
    }
}

/// Send and outcome lifecycle of one dispatch attempt. `Released` is the
/// cancelled-before-send marker; only a dispatch proven unsent may cancel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchState {
    Declined,
    Prepared,
    Sent,
    Succeeded,
    Failed,
    Unknown,
    Released,
}

impl DispatchState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Declined => "declined",
            Self::Prepared => "prepared",
            Self::Sent => "sent",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Released => "released",
        }
    }

    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "declined" => Some(Self::Declined),
            "prepared" => Some(Self::Prepared),
            "sent" => Some(Self::Sent),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "unknown" => Some(Self::Unknown),
            "released" => Some(Self::Released),
            _ => None,
        }
    }
}

/// Finance settlement of one dispatch attempt. The finance rows stay
/// authoritative; this column only mirrors what the worker proved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchSettlement {
    Held,
    Settled,
    UnusableSettled,
    UnknownOutstanding,
    Released,
}

impl DispatchSettlement {
    pub fn label(self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::Settled => "settled",
            Self::UnusableSettled => "unusable_settled",
            Self::UnknownOutstanding => "unknown_outstanding",
            Self::Released => "released",
        }
    }

    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "held" => Some(Self::Held),
            "settled" => Some(Self::Settled),
            "unusable_settled" => Some(Self::UnusableSettled),
            "unknown_outstanding" => Some(Self::UnknownOutstanding),
            "released" => Some(Self::Released),
            _ => None,
        }
    }
}

/// Why a dispatch failed. Closed: a new cause is a contract change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureCause {
    Refused,
    InvalidOutput,
    TransportLost,
    RateLimited,
    QuotaExhausted,
    DeadlineExceeded,
    Provider,
}

impl FailureCause {
    pub fn label(self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::InvalidOutput => "invalid_output",
            Self::TransportLost => "transport_lost",
            Self::RateLimited => "rate_limited",
            Self::QuotaExhausted => "quota_exhausted",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Provider => "provider",
        }
    }

    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "refused" => Some(Self::Refused),
            "invalid_output" => Some(Self::InvalidOutput),
            "transport_lost" => Some(Self::TransportLost),
            "rate_limited" => Some(Self::RateLimited),
            "quota_exhausted" => Some(Self::QuotaExhausted),
            "deadline_exceeded" => Some(Self::DeadlineExceeded),
            "provider" => Some(Self::Provider),
            _ => None,
        }
    }
}

/// The neutral finance identity of one hosted dispatch: all three ids or
/// none. Local work always carries none. These are opaque bytes here; the
/// persistence port converts them to finance ids without changing them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinanceTuple {
    pub attempt: [u8; 16],
    pub operation: [u8; 16],
    pub effect: [u8; 16],
}

impl FinanceTuple {
    /// All three ids must name something; partial tuples never exist.
    pub fn new(
        attempt: [u8; 16],
        operation: [u8; 16],
        effect: [u8; 16],
    ) -> Result<Self, ExecutionSetupError> {
        if attempt == [0; 16] || operation == [0; 16] || effect == [0; 16] {
            return Err(ExecutionSetupError::ZeroId);
        }
        Ok(Self {
            attempt,
            operation,
            effect,
        })
    }
}

/// Whether a dispatch may spend hosted money or only local work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchMode {
    Local,
    Hosted(FinanceTuple),
}

impl DispatchMode {
    pub fn finance(&self) -> Option<FinanceTuple> {
        match self {
            Self::Local => None,
            Self::Hosted(tuple) => Some(*tuple),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }
}

fn check_text(value: &str, max_bytes: usize, required: bool) -> Result<(), ExecutionSetupError> {
    if value.is_empty() {
        return if required {
            Err(ExecutionSetupError::Empty)
        } else {
            Ok(())
        };
    }
    if value.len() > max_bytes {
        return Err(ExecutionSetupError::TooLong { max_bytes });
    }
    Ok(())
}

fn check_time(now_ms: i64) -> Result<(), ExecutionSetupError> {
    if now_ms <= 0 {
        return Err(ExecutionSetupError::BadTime);
    }
    Ok(())
}

/// One durable queue and concurrency scope. The bound lives in the database;
/// no process counter is durability proof.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapacityScope(String);

impl CapacityScope {
    pub fn new(scope: String) -> Result<Self, ExecutionSetupError> {
        check_text(&scope, MAX_CAPACITY_SCOPE_BYTES, true)?;
        Ok(Self(scope))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Plans one logical stage with its immutable basis and the job-level planned
/// publication identity. The planned pair is allocated before classification;
/// it names no asset row and never substitutes for the 0039 admission binding.
#[derive(Clone, Debug, PartialEq)]
pub struct EnqueueCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub kind: StageKind,
    pub input_revision: u32,
    pub input_digest: [u8; 32],
    pub config_revision: u64,
    pub config_digest: [u8; 32],
    pub planned_publication: Option<PublicationIdentity>,
    pub capacity: CapacityScope,
    pub local_accounting: bool,
    pub parent_dispatch: Option<DispatchId>,
    pub now_ms: i64,
}

impl EnqueueCommand {
    pub fn new(
        scope: StageScope,
        basis: StageBasis,
        publication: impl Into<Option<PublicationIdentity>>,
        capacity: CapacityScope,
        local_accounting: bool,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        let StageScope { owner, job, stage } = scope;
        let StageBasis {
            kind,
            input_revision,
            input_digest,
            config_revision,
            config_digest,
            parent_dispatch,
        } = basis;
        let planned_publication = publication.into();
        if input_revision == 0 || config_revision == 0 {
            return Err(ExecutionSetupError::BadRevision);
        }
        if kind.is_standalone() != planned_publication.is_none() {
            return Err(ExecutionSetupError::PublicationIdentityMismatch);
        }
        if planned_publication
            .is_some_and(|identity| identity.asset == [0; 16] || identity.attempt == [0; 16])
        {
            return Err(ExecutionSetupError::ZeroId);
        }

        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            kind,
            input_revision,
            input_digest,
            config_revision,
            config_digest,
            planned_publication,
            capacity,
            local_accounting,
            parent_dispatch,
            now_ms,
        })
    }
    /// Binds this child to a successful parent stage, never to an inferred retry parent.
    pub fn with_parent(mut self, parent: DispatchId) -> Self {
        self.parent_dispatch = Some(parent);
        self
    }
}

/// Claims a stage for one owner: the token is fresh, the expected revision
/// names the actively applied execution gate the worker holds (never the
/// job's immutable config snapshot), and the lease must be positive. The
/// store additionally refuses when the lease window reaches past the
/// retained job deadline, so no fixed ceiling is baked in here.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub expected_config_revision: u64,
    pub claim: ClaimToken,
    pub lease_ms: i64,
    pub now_ms: i64,
}

impl ClaimCommand {
    pub fn new(
        owner: ImageOwner,
        job: ImageJobId,
        stage: ExecutionStageId,
        expected_config_revision: u64,
        claim: ClaimToken,
        lease_ms: i64,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        if expected_config_revision == 0 {
            return Err(ExecutionSetupError::BadRevision);
        }
        if lease_ms <= 0 {
            return Err(ExecutionSetupError::BadLease);
        }
        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            expected_config_revision,
            claim,
            lease_ms,
            now_ms,
        })
    }
}

/// Prepares one send: a fresh dispatch id, the next sequence, and the mode.
/// The finance Sent marker still commits through the finance owner first;
/// this row is the durable execution half that transport reads.
#[derive(Clone, Debug, PartialEq)]
pub struct PrepareCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub claim_generation: u64,
    pub mode: DispatchMode,
    pub now_ms: i64,
}

impl PrepareCommand {
    pub fn new(
        claim: StageClaim,
        dispatch: DispatchId,
        mode: DispatchMode,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        let StageClaim {
            scope: StageScope { owner, job, stage },
            token: claim,
            generation: claim_generation,
        } = claim;

        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            dispatch,
            claim,
            claim_generation,
            mode,
            now_ms,
        })
    }
}

/// Commits the durable Sent marker after the finance owner committed its own.
/// Transport launches only after this row commits; there is no resend after
/// an ambiguous send, only reconciliation by the stable dispatch id.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkSentCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub claim_generation: u64,
    pub now_ms: i64,
}

impl MarkSentCommand {
    pub fn new(
        owner: ImageOwner,
        job: ImageJobId,
        stage: ExecutionStageId,
        dispatch: DispatchId,
        claim: ClaimToken,
        claim_generation: u64,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            dispatch,
            claim,
            claim_generation,
            now_ms,
        })
    }
}

/// Correlates a supplier receipt with its dispatch. Recording the same id
/// twice is one update; a different id over a recorded one is a conflict.
#[derive(Clone, Debug, PartialEq)]
pub struct SupplierReport {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub supplier_job_id: String,
    pub now_ms: i64,
}

impl SupplierReport {
    pub fn new(
        owner: ImageOwner,
        job: ImageJobId,
        stage: ExecutionStageId,
        dispatch: DispatchId,
        supplier_job_id: String,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        check_text(&supplier_job_id, MAX_SUPPLIER_JOB_ID_BYTES, true)?;
        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            dispatch,
            supplier_job_id,
            now_ms,
        })
    }
}

/// The stable book call id for one dispatch: derived from the dispatch id
/// alone, so crash recovery corrects the same call instead of opening one.
/// This neutral value never imports the cost owner's unfinished types.
pub fn book_call_id_for(dispatch: &DispatchId) -> String {
    format!("img-{}", dispatch.to_hex())
}

/// Validated alpha fact bound into an output checkpoint. Closed so a new
/// handling needs a contract change; the publication seam reads this, never
/// a freeform caller string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointAlpha {
    Opaque,
    Transparent,
}

impl CheckpointAlpha {
    pub fn label(self) -> &'static str {
        match self {
            Self::Opaque => "opaque",
            Self::Transparent => "transparent",
        }
    }

    pub fn parse_label(label: &str) -> Option<Self> {
        match label {
            "opaque" => Some(Self::Opaque),
            "transparent" => Some(Self::Transparent),
            _ => None,
        }
    }
}

/// Output checkpoint for one dispatch: the staged object locator plus the
/// metadata publication must verify against the catalog. The storage key alone proves nothing:
/// the immutable object version, exact byte length, content digest,
/// validated metadata (mime, dimensions, alpha) and usage revision must all
/// match the staged bytes, and the stable book call id is bound at
/// construction. Terminal replay compares the full checkpoint; a conflicting
/// replay is refused and the winning write stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputCheckpoint {
    pub storage_key: String,
    pub object_version: String,
    pub byte_len: u64,
    pub sha256: [u8; 32],
    pub recipe_digest: [u8; 32],
    pub mime: String,
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    pub alpha: CheckpointAlpha,
    pub usage_revision: u32,
    pub book_call_id: String,
}

impl OutputCheckpoint {
    pub fn new(
        dispatch: &DispatchId,
        object: OutputObject,
        image: OutputImage,
        recipe_digest: [u8; 32],
        usage_revision: u32,
    ) -> Result<Self, ExecutionSetupError> {
        let OutputObject {
            storage_key,
            object_version,
            byte_len,
            sha256,
        } = object;
        let OutputImage {
            mime,
            width_px,
            height_px,
            alpha,
        } = image;

        check_text(&storage_key, MAX_STORAGE_KEY_BYTES, true)?;
        check_text(&mime, MAX_MIME_BYTES, true)?;
        if object_version.is_empty() {
            return Err(ExecutionSetupError::Empty);
        }
        if object_version.len() > MAXIMUM_OBJECT_VERSION_BYTES {
            return Err(ExecutionSetupError::TooLong {
                max_bytes: MAXIMUM_OBJECT_VERSION_BYTES,
            });
        }
        if byte_len == 0 {
            return Err(ExecutionSetupError::ZeroBytes);
        }
        if width_px == Some(0) || height_px == Some(0) {
            return Err(ExecutionSetupError::ZeroDimension);
        }
        Ok(Self {
            storage_key,
            object_version,
            byte_len,
            sha256,
            recipe_digest,
            mime,
            width_px,
            height_px,
            alpha,
            usage_revision,
            book_call_id: book_call_id_for(dispatch),
        })
    }
}

/// Versioned supplier resume checkpoint for one dispatch: the bounded opaque
/// bytes plus the origin, model and stage references a fal/BFL worker needs
/// to resume or reconcile without resending. The supplier correlation id
/// alone cannot resume work, so it is never the only retained supplier fact.
#[derive(Clone, Eq, PartialEq)]
pub struct SupplierCheckpoint {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub checkpoint_version: u32,
    pub origin: String,
    pub model: String,
    pub checkpoint_bytes: Vec<u8>,
    pub now_ms: i64,
}

impl fmt::Debug for SupplierCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupplierCheckpoint")
            .field("checkpoint_version", &self.checkpoint_version)
            .field("len_bytes", &self.checkpoint_bytes.len())
            .finish_non_exhaustive()
    }
}

impl SupplierCheckpoint {
    pub fn new(
        scope: ExecutionScope,
        resume: SupplierResume,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        let ExecutionScope {
            owner,
            job,
            stage,
            dispatch,
        } = scope;
        let SupplierResume {
            checkpoint_version,
            origin,
            model,
            checkpoint_bytes,
        } = resume;

        if checkpoint_version == 0 {
            return Err(ExecutionSetupError::BadRevision);
        }
        check_text(&origin, MAX_SUPPLIER_ORIGIN_BYTES, true)?;
        check_text(&model, MAX_SUPPLIER_MODEL_BYTES, true)?;
        if checkpoint_bytes.is_empty() {
            return Err(ExecutionSetupError::Empty);
        }
        if checkpoint_bytes.len() > MAX_SUPPLIER_CHECKPOINT_BYTES {
            return Err(ExecutionSetupError::TooLong {
                max_bytes: MAX_SUPPLIER_CHECKPOINT_BYTES,
            });
        }
        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            dispatch,
            checkpoint_version,
            origin,
            model,
            checkpoint_bytes,
            now_ms,
        })
    }
}

/// How a dispatch attempt ended. A checkpoint rides only on success; failure
/// and unknown carry no bytes.
#[derive(Clone, Debug, PartialEq)]
pub enum CompletionOutcome {
    /// Proved submit rejection; old financial exposure remains fully outstanding.
    Declined {
        observation_version: u32,
        usage_revision: u32,
    },
    Succeeded {
        usage_revision: u32,
        checkpoint: OutputCheckpoint,
    },
    Decision {
        observation_version: u32,
        usage_revision: u32,
    },
    Candidate {
        observation_version: u32,
        usage_revision: u32,
    },
    /// Definitive unusable output with a durable reported/estimated failure receipt.
    Unusable {
        cause: FailureCause,
        usage_revision: u32,
    },
    Failed {
        cause: FailureCause,
    },
    Unknown,
    /// A closed actual transport cause, with the same unresolved outcome and liability as Unknown.
    UnknownWithCause {
        cause: FailureCause,
    },
}

/// Completes one dispatch. Only the cursor's current dispatch may complete;
/// a late result for a superseded dispatch is refused, never finalized.
#[derive(Clone, Debug, PartialEq)]
pub struct CompleteCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub claim_generation: u64,
    pub outcome: CompletionOutcome,
    pub now_ms: i64,
}

impl CompleteCommand {
    pub fn new(
        claim: StageClaim,
        dispatch: DispatchId,
        outcome: CompletionOutcome,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        let StageClaim {
            scope: StageScope { owner, job, stage },
            token: claim,
            generation: claim_generation,
        } = claim;

        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            dispatch,
            claim,
            claim_generation,
            outcome,
            now_ms,
        })
    }
}

/// Persists which dispatch a refetch was requested for.
#[derive(Clone, Debug, PartialEq)]
pub struct RefetchCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub now_ms: i64,
}

impl RefetchCommand {
    pub fn new(
        owner: ImageOwner,
        job: ImageJobId,
        stage: ExecutionStageId,
        dispatch: DispatchId,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            dispatch,
            now_ms,
        })
    }
}

/// Releases a stage before send and retires its durable capacity reserve.
/// Only a stage holding no sent dispatch may release; sent work reconciles.
#[derive(Clone, Debug, PartialEq)]
pub struct ReleaseCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub claim: ClaimToken,
    pub claim_generation: u64,
    pub now_ms: i64,
}

impl ReleaseCommand {
    pub fn new(
        owner: ImageOwner,
        job: ImageJobId,
        stage: ExecutionStageId,
        claim: ClaimToken,
        claim_generation: u64,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            claim,
            claim_generation,
            now_ms,
        })
    }
}

/// A normal local egress gate distinguishes a new marker from historical replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalSentOutcome {
    New,
    AlreadySent,
}

/// The stored stage with its current claim and job revision.
#[derive(Clone, Debug, PartialEq)]
pub struct StageView {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub kind: StageKind,
    pub state: StageState,
    pub claim_generation: u64,
    pub lease_until_ms: i64,
    pub local_accounting: bool,
    pub planned_publication: Option<PublicationIdentity>,
    pub job_revision: u64,
}

/// Cancels one prepared dispatch before any byte leaves. Only a dispatch
/// proven unsent may cancel: the finance owner releases its hold first, then
/// this records the durable cancelled marker. A sent dispatch refuses.
#[derive(Clone, Debug, PartialEq)]
pub struct CancelCommand {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub claim: ClaimToken,
    pub claim_generation: u64,
    pub now_ms: i64,
}

impl CancelCommand {
    pub fn new(
        owner: ImageOwner,
        job: ImageJobId,
        stage: ExecutionStageId,
        dispatch: DispatchId,
        claim: ClaimToken,
        claim_generation: u64,
        now_ms: i64,
    ) -> Result<Self, ExecutionSetupError> {
        check_time(now_ms)?;
        Ok(Self {
            owner,
            job,
            stage,
            dispatch,
            claim,
            claim_generation,
            now_ms,
        })
    }
}

/// The stored dispatch with its send, outcome and finance identity.
#[derive(Clone, Debug, PartialEq)]
pub struct DispatchView {
    pub owner: ImageOwner,
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub dispatch: DispatchId,
    pub sequence: u32,
    pub state: DispatchState,
    pub cause: Option<FailureCause>,
    pub settlement: DispatchSettlement,
    pub finance: Option<FinanceTuple>,
    pub local_accounting: bool,
    pub supplier_job_id: Option<String>,
    pub usage_revision: u32,
    pub book_call_id: String,
    pub lineage_parent: Option<DispatchId>,
    pub claim_generation: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Recovery answer for one planned or published identity: the owning
/// job/stage, the current output dispatch, the persisted refetch selection,
/// and the local checkpoint when the work was local-only.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedExecution {
    pub job: ImageJobId,
    pub stage: ExecutionStageId,
    pub current: Option<DispatchView>,
    pub refetch: Option<DispatchId>,
    pub local_checkpoint: Option<OutputCheckpoint>,
}

/// Where new demand goes with its durable reserve, or the identical stage.
#[derive(Clone, Debug, PartialEq)]
pub enum EnqueueOutcome {
    Enqueued(StageView),
    Existing(StageView),
}

impl EnqueueOutcome {
    pub fn view(&self) -> &StageView {
        match self {
            Self::Enqueued(view) | Self::Existing(view) => view,
        }
    }
}

/// Every way an execution call can fail. Callers branch on the variant:
/// `Unavailable` changed nothing, `Unknown` means the commit may have taken
/// effect so the caller must reread by id before retrying. No variant carries
/// prompts, payload bytes or foreign ids beyond the involved record keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionError {
    /// Nothing was changed: unreachable, or the transaction failed pre-commit.
    Unavailable,
    /// The commit may or may not have taken effect; reread before retrying.
    Unknown,
    /// No such owned job, stage or dispatch. A wrong owner learns nothing.
    NotFound,
    /// A stored row did not parse; nothing was written.
    InvalidRecord,
    /// Current source authority or the exact consumed catalog winner was refused.
    SourceDenied,
    /// No attached interest holds the job; demand lapsed with no waiter.
    DemandLapsed,
    /// A supplied stage or dispatch id is already bound to different data.
    IdentityConflict,
    /// The same scoped operation returns under a different fingerprint, or a
    /// terminal dispatch is completed again with a conflicting outcome.
    OperationConflict,
    /// The same stage was enqueued under a different immutable basis.
    BasisConflict,
    /// The job already planned another publication identity for this work.
    PlannedIdentityConflict,
    /// The job moved on; retry against the returned revision.
    Stale { current_revision: u64 },
    /// A write carrying an older token or generation; the lease moved on.
    LostClaim,
    /// The job deadline passed; only an identical replay still succeeds.
    Expired,
    /// The job retired; only an identical replay still succeeds.
    Retired,
    /// No durable capacity slot was free; nothing was reserved.
    CapacityFull,
    /// This queued stage is deliberately deferred until the stored due time.
    Deferred { not_before_ms: i64 },
    /// A durable no-delivery seal forbids creating or publishing further output.
    OutputSealed,
    /// The stored config revision moved past the expected one.
    ConfigFenced { current_revision: u64 },
    /// The operator execution gate disabled this scope: claims, prepares and
    /// sends refuse. Reconciliation of already-sent work still proceeds and
    /// outstanding liability is retained.
    Disabled,
    /// A new observation exceeds the currently configured host scope bound.
    DecisionPayloadTooLarge { maximum_bytes: u64 },
    /// Raw usage evidence is not durable for this exact observed decision revision.
    ObservationMissing,
    /// A dispatch already correlated with another supplier id or checkpoint.
    SupplierConflict,
    /// A late result for a dispatch the cursor already superseded, or a new
    /// send while the cursor still names an unresolved dispatch. Nothing was
    /// superseded silently: the current attempt stands.
    Superseded,
}

/// Resolves planned or publication identity to its job and stage for
/// recovery: the planned pair reaches the stage, the published pair reaches
/// the same stage through the retained allocation. This is identity routing
/// only; it authorizes nothing and reads no finance state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionIdentity {
    pub asset: [u8; 16],
    pub attempt: [u8; 16],
}

impl ExecutionIdentity {
    pub fn new(asset: [u8; 16], attempt: [u8; 16]) -> Result<Self, ExecutionSetupError> {
        if asset == [0; 16] || attempt == [0; 16] {
            return Err(ExecutionSetupError::ZeroId);
        }
        Ok(Self { asset, attempt })
    }
}

#[cfg(test)]
#[path = "execution_tests.rs"]
mod execution_tests;

mod local_inspection;
pub use local_inspection::{
    LocalInspectionCheckpoint, LocalInspectionCommand, LocalInspectionError, LocalInspectionReason,
};

mod terminal;
pub use terminal::{
    TerminalActor, TerminalAllocation, TerminalAuthority, TerminalCommand, TerminalStageSnapshot,
    TerminalTail, TerminalValidation,
};

mod analysis;
pub use analysis::{
    AnalysisCheckpoint, AnalysisCheckpointReference, AnalysisDependencyRef, AnalysisStageReference,
    LocalAnalysisCheckpoint, LocalAnalysisReference,
};

/// A planned media publication identity; standalone analysis carries none.
pub type PublicationIdentity = ExecutionIdentity;
