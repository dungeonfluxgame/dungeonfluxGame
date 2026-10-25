//! The rows a flow decides on, as the database returned them under the flow's locks
//! (planning/finance-ledger-research.md 17.6 `lock_flow`). The pure decisions take these and
//! nothing else, so a flow cannot depend on state it did not lock.

use crate::finance_ids::{
    AllocationId, AttemptId, CampaignKey, LotId, PayerId, PriceVersionId, QuoteId, ReservationId,
    SessionKey, TenantKey, UsageEventId,
};
use crate::funding::{LotClass, LotState};
use crate::ledger::CurrencyTag;
use crate::returns::AllocationStatus;

/// A session's funding row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionState {
    pub session: SessionKey,
    pub tenant: TenantKey,
    pub campaign: CampaignKey,
    pub price_version: PriceVersionId,
    pub currency: CurrencyTag,
    pub open: bool,
    pub current_epoch: i64,
    pub charged_total_micro_dollars: u64,
    pub hourly_ceiling_micro_dollars: u64,
    pub session_ceiling_micro_dollars: Option<u64>,
    pub per_request_threshold_micro_dollars: u64,
    pub revision: i64,
}

/// One payer's lot as an allocation sees it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationLotState {
    pub lot: LotId,
    pub class: LotClass,
    pub state: LotState,
    pub created_at_ms: i64,
    pub draw_rank: i64,
    pub allocated_micro_dollars: u64,
}

/// One allocation of the epoch, with its payer's standing and lots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationState {
    pub allocation: AllocationId,
    pub payer: PayerId,
    pub status: AllocationStatus,
    pub authorized_micro_dollars: u64,
    pub drawn_micro_dollars: u64,
    pub returned_micro_dollars: u64,
    pub charged_micro_dollars: u64,
    pub revision: i64,
    pub payer_active: bool,
    pub approved_hourly_ceiling_micro_dollars: u64,
    pub approved_session_ceiling_micro_dollars: Option<u64>,
    pub charged_last_hour_micro_dollars: u64,
    pub open_holds_micro_dollars: u64,
    pub lots: Vec<AllocationLotState>,
}

/// A lot row with the totals a flow updates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LotRow {
    pub lot: LotId,
    pub payer: PayerId,
    pub class: LotClass,
    pub promo_source: Option<String>,
    pub processor: Option<String>,
    pub currency: CurrencyTag,
    pub state: LotState,
    pub original_micro_dollars: u64,
    pub consumed_micro_dollars: u64,
    pub extinguished_micro_dollars: u64,
    pub created_at_ms: i64,
    pub revision: i64,
}

/// A quote's state (9.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuoteStatus {
    Open,
    Approved,
    Declined,
    Expired,
    Used,
}

impl QuoteStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Approved => "approved",
            Self::Declined => "declined",
            Self::Expired => "expired",
            Self::Used => "used",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "open" => Some(Self::Open),
            "approved" => Some(Self::Approved),
            "declined" => Some(Self::Declined),
            "expired" => Some(Self::Expired),
            "used" => Some(Self::Used),
            _ => None,
        }
    }
}

/// A stored quote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuoteRow {
    pub quote: QuoteId,
    pub session: SessionKey,
    pub request_digest: [u8; 32],
    pub price_version: PriceVersionId,
    pub retail_maximum_micro_dollars: u64,
    pub expires_at_ms: i64,
    pub status: QuoteStatus,
    pub revision: i64,
}

/// A stored receipt of an admitted operation (11.2 step 2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptRow {
    pub fingerprint: [u8; 32],
    pub reservation: ReservationId,
    pub attempt: AttemptId,
    pub retail_maximum_micro_dollars: u64,
    pub supplier_maximum_micro_dollars: u64,
    pub price_version: PriceVersionId,
}

/// A reservation share as stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShareRow {
    pub reservation: ReservationId,
    pub allocation: AllocationId,
    pub lot: LotId,
    pub class: LotClass,
    pub held_micro_dollars: u64,
    pub charged_micro_dollars: u64,
    pub released_micro_dollars: u64,
    pub reversed_micro_dollars: u64,
    pub revision: i64,
}

/// A reservation as stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReservationRow {
    pub reservation: ReservationId,
    pub session: SessionKey,
    pub retail_maximum_micro_dollars: u64,
    pub held: bool,
    pub price_version: PriceVersionId,
    pub revision: i64,
}

/// The dispatch dimension of an attempt (11.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchState {
    Reserved,
    DispatchPrepared,
    Sent,
    ResultKnown,
    OutcomeUnknown,
    VerifiedUnsent,
    CancelledUnsent,
    FailedUnknown,
}

impl DispatchState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::DispatchPrepared => "dispatch_prepared",
            Self::Sent => "sent",
            Self::ResultKnown => "result_known",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::VerifiedUnsent => "verified_unsent",
            Self::CancelledUnsent => "cancelled_unsent",
            Self::FailedUnknown => "failed_unknown",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "reserved" => Some(Self::Reserved),
            "dispatch_prepared" => Some(Self::DispatchPrepared),
            "sent" => Some(Self::Sent),
            "result_known" => Some(Self::ResultKnown),
            "outcome_unknown" => Some(Self::OutcomeUnknown),
            "verified_unsent" => Some(Self::VerifiedUnsent),
            "cancelled_unsent" => Some(Self::CancelledUnsent),
            "failed_unknown" => Some(Self::FailedUnknown),
            _ => None,
        }
    }

    /// Nothing has left for the provider, so the supplier exposure can close with the hold.
    pub fn is_unsent(self) -> bool {
        matches!(
            self,
            Self::Reserved | Self::DispatchPrepared | Self::VerifiedUnsent | Self::CancelledUnsent
        )
    }
}

/// A provider attempt as stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptRow {
    pub attempt: AttemptId,
    pub reservation: ReservationId,
    pub provider: String,
    pub model: String,
    pub supplier_exposure_micro_dollars: u64,
    pub dispatch: DispatchState,
    pub customer_held: bool,
    pub supplier_open: bool,
    pub media_asset: Option<[u8; 16]>,
    pub campaign: Option<CampaignKey>,
    pub revision: i64,
}

/// A usage event as stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageRow {
    pub usage_event: UsageEventId,
    pub attempt: AttemptId,
    pub session: SessionKey,
    pub retail_micro_dollars: u64,
    pub reversed_micro_dollars: u64,
    pub price_version: PriceVersionId,
    pub revision: i64,
}

/// An allocation row with the totals a settlement updates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationRow {
    pub allocation: AllocationId,
    pub payer: PayerId,
    pub status: AllocationStatus,
    pub returned_micro_dollars: u64,
    pub charged_micro_dollars: u64,
    pub revision: i64,
}
