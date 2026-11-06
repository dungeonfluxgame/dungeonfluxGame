//! The neutral imagery cost projection contract: how one durable dispatch attempt and its
//! usage revisions project onto the existing cost book under one stable call id.
//!
//! # The contract
//!
//! * The dispatch journal is the durable authority. The book is a projection that never
//!   overrides it and never becomes a second money ledger.
//! * One durable 16-byte dispatch id maps to exactly one stable book call id
//!   ([`pinned_call_id`]): redelivery of the SAME dispatch replays under the same id and never
//!   mints a second charge identity. A real paid retry is a NEW dispatch with new bytes and a
//!   new call id; the old attempt's record (unknown or settled) is retained, never rewritten.
//! * Usage arrives with a monotonic [`UsageRevision`] and [`UsageEvidence`]. An exact replay of
//!   a stored revision is a no-op; a stale revision is refused; a newer revision replaces the
//!   totals, rollups and recent entry under the same call id and emits a correction event.
//! * Unknown usage is outstanding exposure at the call's maxima, never zero. Lines without a
//!   tariff stay visible as unpriced with the priced lines summed; billed-but-unusable work stays
//!   attributed; proven-unsent work records nothing and stays terminal through recovery.
//! * `Applied` is a local projection acknowledgement only, never a durability receipt: the
//!   dispatch journal owns the raw usage and retains it until the run log holds the revision.
//!   Durable terminal records may leave the bounded cache and reload from the existing cost
//!   log. Unpersisted/open records apply transient backpressure through ProjectionFull.
//!
//! This crate never imports `df-tools` or finance vocabulary: routes, units and evidence are
//! plain data. The tools-side adapter (`df-tools` gameplay cost owner) implements
//! [`ImageCostProjection`] over the real book.

use crate::listed::listed_enum;

/// The prefix of every stable book call id derived from a dispatch id.
pub const PINNED_CALL_PREFIX: &str = "img-";

/// The stable book call id for a durable 16-byte dispatch id: `img-` plus 32 lowercase hex
/// digits. Deterministic in the dispatch id alone, so a retry reuses it without randomness.
pub fn pinned_call_id(dispatch: &[u8; 16]) -> String {
    format!("{PINNED_CALL_PREFIX}{}", crate::lower_hex(dispatch))
}

/// One durable dispatch attempt, by its 16-byte journal identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DispatchId([u8; 16]);

impl DispatchId {
    /// Binds the journal's dispatch identity. Every byte matters; there is no default.
    pub fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// The raw journal identity.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// The stable book call id for this dispatch (see [`pinned_call_id`]).
    pub fn call_id(&self) -> String {
        pinned_call_id(&self.0)
    }
}

/// The journal's monotonic usage revision for one dispatch: each new supplier report advances
/// it, and the book refuses anything older than what it already holds.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UsageRevision(pub u64);

listed_enum! {
    keyed
    /// Where the quantities of one usage report came from. `Unknown` is outstanding exposure at
    /// the call's maxima, never a zero.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum UsageEvidence {
        /// The supplier's own usage report.
        Reported,
        /// What the server counted or estimated.
        Estimated,
        /// The call was sent and its outcome is unknown; the quantities are the call's maxima.
        Unknown,
    }
}

impl UsageEvidence {
    /// The word the book and the audit line use for this evidence.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Estimated => "estimated",
            Self::Unknown => "unknown",
        }
    }
}

/// One quantity of one category: a category name and an integer quantity. A name the book
/// does not price stays visible as unpriced; it never zeroes the priced lines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CostUnits {
    /// The category name (`text_input`, `image_output`, `credits`, ...).
    pub category: String,
    /// The integer quantity in that category's own unit.
    pub quantity: u64,
}

/// The route of a call to project: names only, checked against the book's vocabularies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CostRoute {
    /// Who bills the call (`openai`, `fal`, `bfl`, ...).
    pub provider: String,
    /// The billed model or endpoint.
    pub model: String,
    /// What in the game caused the call.
    pub cause: String,
    /// A short label narrowing the cause.
    pub detail: String,
    /// The most the call can use, one entry per category its route declares.
    pub maxima: Vec<CostUnits>,
}

/// One usage report for a pinned call: the revision, its evidence, the quantities, whether
/// the billed output was usable, and the caller's clock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageReport<'a> {
    /// The journal's revision for this report.
    pub revision: UsageRevision,
    /// Where the quantities came from.
    pub evidence: UsageEvidence,
    /// The quantities used. Unknown always uses maxima. Empty Reported/Estimated also means
    /// maxima; explicitly zero usage requires a nonempty line with quantity zero.
    pub units: &'a [CostUnits],
    /// False for a call that was billed and produced nothing usable: a company loss, kept
    /// attributed, never charged on.
    pub usable: bool,
    /// When the usage was recorded, in milliseconds.
    pub at_ms: u64,
}

/// Opening a pinned call: started, an exact replay, or already settled under this id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinnedStart {
    /// The call is now open under this id.
    Opened,
    /// The same route is already open under this id: nothing changed.
    ReplayedOpen,
    /// This id already settled: no second charge identity is opened.
    AlreadySettled,
}

/// Applying a usage report: replaced the stored revision, or an exact replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinnedCorrection {
    /// The newer revision replaced the stored one under the same call id.
    Applied,
    /// The same revision with identical evidence: nothing changed.
    Replayed,
}

/// Recording proven-unsent work: settled without a record, or an exact replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinnedUnsent {
    /// Proven never sent: nothing was recorded and nothing can settle it later.
    Recorded,
    /// Already recorded unsent: nothing changed.
    Replayed,
}

/// One coherent read of the existing Book; no quote, reserve or customer refund authority.
#[derive(Clone, Eq, PartialEq)]
pub struct PinnedCostReceipt {
    pub call_id: String,
    /// The usage revision the Book holds for this call.
    pub revision: UsageRevision,
    pub route: CostRoute,
    pub evidence: UsageEvidence,
    pub units: Vec<CostUnits>,
    /// False for billed work that produced nothing usable.
    pub usable: bool,
    /// None when any line is unpriced or no quantity was booked; never an invented zero.
    pub amount: Option<df_types::Money>,
    /// True once the existing cost log has synced this revision.
    pub durable: bool,
}

impl std::fmt::Debug for PinnedCostReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PinnedCostReceipt")
            .field("revision", &self.revision)
            .field("evidence", &self.evidence)
            .field("unit_lines", &self.units.len())
            .field("has_amount", &self.amount.is_some())
            .field("durable", &self.durable)
            .finish_non_exhaustive()
    }
}

/// Whether the existing cost log has durably stored a no-delivery tombstone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsentDurability {
    /// Sealed in memory only; not an acknowledgement, so the caller retries the same seal.
    Pending,
    /// The tombstone is in the durable cost log.
    Durable,
}

/// Trusted infrastructure response from the actual Book seal command, never a client request.
#[derive(Clone, Eq, PartialEq)]
pub struct PinnedUnsentSeal {
    pub call_id: String,
    pub durability: UnsentDurability,
}
impl std::fmt::Debug for PinnedUnsentSeal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PinnedUnsentSeal")
            .field("durability", &self.durability)
            .finish_non_exhaustive()
    }
}

/// A runtime-verified same-call durable seal. Infrastructure consumers cannot mint this proof.
#[derive(Clone, Eq, PartialEq)]
pub struct DurableUnsentReceipt {
    call_id: String,
}
impl DurableUnsentReceipt {
    pub fn call_id(&self) -> &str {
        &self.call_id
    }
    pub fn durability(&self) -> UnsentDurability {
        UnsentDurability::Durable
    }
    pub(crate) fn from_seal(
        seal: PinnedUnsentSeal,
        expected_dispatch: &[u8; 16],
    ) -> Result<Self, CostError> {
        if seal.call_id != pinned_call_id(expected_dispatch) {
            return Err(CostError::UnknownCall);
        }
        if seal.durability != UnsentDurability::Durable {
            return Err(CostError::ProjectionFull);
        }
        Ok(Self {
            call_id: seal.call_id,
        })
    }
}
impl std::fmt::Debug for DurableUnsentReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableUnsentReceipt")
            .field("call_id", &self.call_id)
            .finish()
    }
}

/// Why a pinned projection was refused. Redacted by construction: revisions only, never
/// prompts, prices, credentials or payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CostError {
    /// The installed port has no authoritative no-delivery sealer; no acknowledgement is safe.
    UnsentSealUnavailable,
    /// The route names a provider the book does not know.
    UnknownProvider,
    /// The route names a cause the book does not know.
    UnknownCause,
    /// A units entry names a category the book does not know.
    UnknownCategory,
    /// No call was started under this id. Start it first; recovery replays the start.
    UnknownCall,
    /// The same id names a different route.
    RouteConflict,
    /// The revision is older than the stored one; state is unchanged.
    StaleRevision {
        /// The revision the book holds.
        stored: UsageRevision,
        /// The refused revision.
        got: UsageRevision,
    },
    /// The same revision carries different evidence; state is unchanged.
    RevisionConflict {
        /// The revision both reports claim.
        revision: UsageRevision,
    },
    /// This id already settled with usage (or as unsent) and cannot take the other path.
    AlreadySettled,
    /// The book is not open yet. The server opens it before dispatch; this is a caller bug.
    BookNotOpen,
    /// The bounded retention window holds no more pinned identities: the journal keeps the raw
    /// usage and the caller retries later. Refused rather than forgetting a terminal identity.
    ProjectionFull,
    /// Historical log lacks a frozen tariff; retain journal usage for explicit repair.
    PriceBasisUnavailable,
    /// Recorded cost data exceeds recovery bounds or fails its stored consistency checks.
    CorruptLog,
}

impl std::fmt::Display for CostError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsentSealUnavailable => {
                formatter.write_str("no authoritative unsent sealer is installed")
            }
            Self::UnknownProvider => {
                formatter.write_str("the cost book does not know the provider")
            }
            Self::UnknownCause => formatter.write_str("the cost book does not know the cause"),
            Self::UnknownCategory => {
                formatter.write_str("the cost book does not know the unit category")
            }
            Self::UnknownCall => formatter.write_str("no call was started under this id"),
            Self::RouteConflict => formatter.write_str("the call id names a different route"),
            Self::StaleRevision { stored, got } => write!(
                formatter,
                "usage revision {} is older than the stored {}",
                got.0, stored.0
            ),
            Self::RevisionConflict { revision } => write!(
                formatter,
                "usage revision {} carries different evidence",
                revision.0
            ),
            Self::AlreadySettled => formatter.write_str("the call already settled"),
            Self::BookNotOpen => formatter.write_str("the cost book is not open"),
            Self::ProjectionFull => formatter.write_str("the cost projection window is full"),
            Self::PriceBasisUnavailable => {
                formatter.write_str("the historical price basis is unavailable")
            }
            Self::CorruptLog => formatter.write_str("the recorded cost data is corrupt"),
        }
    }
}

impl std::error::Error for CostError {}

/// An owned Send future for the projection's bounded storage worker.
pub type CostFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// The neutral port the cost owner implements: one stable call id per dispatch, with
/// idempotent start and same-id correction.
pub trait ImageCostProjection: Send + Sync {
    /// The stable book call id for `dispatch` (see [`pinned_call_id`]).
    fn call_id_for(&self, dispatch: &[u8; 16]) -> String {
        pinned_call_id(dispatch)
    }

    /// Opens the pinned call for `call_id` on `route`. Idempotent per id: an exact replay
    /// answers [`PinnedStart::ReplayedOpen`], and a replay after settle answers
    /// [`PinnedStart::AlreadySettled`] instead of opening a second charge identity.
    fn start_pinned<'a>(
        &'a self,
        call_id: &'a str,
        route: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>>;

    /// Applies `report` to the open or settled call: the first report settles it, a newer
    /// revision replaces the totals under the same id as a correction, an exact replay is a
    /// no-op, and a stale or conflicting revision is refused.
    fn correct<'a>(
        &'a self,
        call_id: &'a str,
        report: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>>;

    /// Records the call as proven unsent (refused submit, cancelled before dispatch): nothing
    /// is recorded, and later usage for the id is refused. A sent attempt cannot take this
    /// path.
    fn record_unsent<'a>(
        &'a self,
        call_id: &'a str,
    ) -> CostFuture<'a, Result<PinnedUnsent, CostError>>;

    /// Seals an exact canonical call only after stored authority proves no delivery.
    /// Unlike record_unsent, absence creates an existing-format tombstone to block late starts.
    /// Pending is not an acknowledgement; callers retain the proof and retry this same command.
    fn seal_unsent<'a>(
        &'a self,
        _call_id: &'a str,
    ) -> CostFuture<'a, Result<PinnedUnsentSeal, CostError>> {
        Box::pin(async { Err(CostError::UnsentSealUnavailable) })
    }

    /// Reads an optional same-call snapshot from the existing Book. Absence is pending,
    /// never a zero amount; implementers without a Book reader safely return no receipt.
    fn read_receipt<'a>(
        &'a self,
        _call_id: &'a str,
    ) -> CostFuture<'a, Result<Option<PinnedCostReceipt>, CostError>> {
        Box::pin(async { Ok(None) })
    }

    /// True only after the existing cost log has synced this exact revision (or unsent marker).
    /// A false result requires retaining journal usage and retrying the same id and revision.
    fn is_durable<'a>(
        &'a self,
        call_id: &'a str,
        revision: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>>;
}

impl<T: ImageCostProjection + ?Sized> ImageCostProjection for std::sync::Arc<T> {
    fn call_id_for(&self, dispatch: &[u8; 16]) -> String {
        self.as_ref().call_id_for(dispatch)
    }

    fn start_pinned<'a>(
        &'a self,
        call_id: &'a str,
        route: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        self.as_ref().start_pinned(call_id, route)
    }

    fn correct<'a>(
        &'a self,
        call_id: &'a str,
        report: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        self.as_ref().correct(call_id, report)
    }

    fn record_unsent<'a>(
        &'a self,
        call_id: &'a str,
    ) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        self.as_ref().record_unsent(call_id)
    }

    fn seal_unsent<'a>(
        &'a self,
        call_id: &'a str,
    ) -> CostFuture<'a, Result<PinnedUnsentSeal, CostError>> {
        self.as_ref().seal_unsent(call_id)
    }

    fn read_receipt<'a>(
        &'a self,
        call_id: &'a str,
    ) -> CostFuture<'a, Result<Option<PinnedCostReceipt>, CostError>> {
        self.as_ref().read_receipt(call_id)
    }

    fn is_durable<'a>(
        &'a self,
        call_id: &'a str,
        revision: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        self.as_ref().is_durable(call_id, revision)
    }
}

#[cfg(test)]
#[path = "cost_tests.rs"]
mod tests;
