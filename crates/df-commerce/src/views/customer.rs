//! Customer-safe finance views (D26, planning/finance-ledger-research.md 18.9). These types have no
//! field for the table fee, the markup, a provider, a provider price, a supplier cost or a margin,
//! and this module imports nothing from `views::operator`. Every string is a closed enum code, an
//! id in hex, a schema constant, or the output of a closed formatter; the price version label is
//! opaque. A change here must update the golden schema beside it (`customer_golden.txt`).

use serde::{Deserialize, Serialize};

use crate::digest::sha256;
use crate::finance_ids::PriceVersionId;
use crate::rounding::cents_half_up;

pub const BALANCE_SCHEMA: &str = "dungeonflux.finance.balance/1";
pub const SESSION_FUNDING_SCHEMA: &str = "dungeonflux.finance.session_funding/1";
pub const BUDGET_CHIP_SCHEMA: &str = "dungeonflux.finance.budget_chip/1";
pub const QUOTE_SCHEMA: &str = "dungeonflux.finance.quote/1";
pub const STATEMENT_SCHEMA: &str = "dungeonflux.finance.statement/1";
pub const TRANSACTION_LIST_SCHEMA: &str = "dungeonflux.finance.transaction_list/1";
pub const REFUND_PREVIEW_SCHEMA: &str = "dungeonflux.finance.refund_preview/1";
pub const PURCHASE_STATUS_SCHEMA: &str = "dungeonflux.finance.purchase_status/1";

/// An amount and its display rounding (rule 4.10): exact micro-dollars and whole cents, half up.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MoneyView {
    pub micro_dollars: u64,
    pub display_cents: i64,
}

impl MoneyView {
    pub fn new(micro_dollars: u64) -> Self {
        let cents = cents_half_up(i128::from(micro_dollars));
        Self {
            micro_dollars,
            display_cents: i64::try_from(cents).unwrap_or(i64::MAX),
        }
    }
}

/// A short code derived from the version id by hashing, never a date range, a rate or a name
/// someone typed: `pv-` and five characters of lowercase base 32.
pub fn price_version_label(id: PriceVersionId) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut input = b"price_version_label".to_vec();
    input.extend_from_slice(id.as_bytes());
    let digest = sha256(&input);
    let bits = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    let mut label = String::from("pv-");
    for shift in [27_u32, 22, 17, 12, 7] {
        label.push(char::from(
            ALPHABET[usize::try_from((bits >> shift) & 31).unwrap_or(0)],
        ));
    }
    label
}

/// The units a statement line may count in; `quantity_text` is only ever built from one of these.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuantityUnit {
    Tokens,
    Images,
    Seconds,
    Calls,
}

/// The one formatter of `quantity_text`: the count and a unit suffix from a closed set.
pub fn quantity_text(unit: QuantityUnit, quantity: u64) -> String {
    let suffix = match unit {
        QuantityUnit::Tokens => "tok",
        QuantityUnit::Images => "img",
        QuantityUnit::Seconds => "s",
        QuantityUnit::Calls => "call",
    };
    format!("{quantity}{suffix}")
}

/// What a statement line is for: a closed vocabulary, never a provider or model name.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryLabel {
    Narration,
    Image,
    Voice,
    TableTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineStatus {
    Settled,
    PendingHold,
    Credited,
    TechnicalFailureCredited,
    ReleasedNotCharged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FundsState {
    Funded,
    Low,
    Unfunded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentStatus {
    Current,
    NeedsReconsent,
    Withdrawn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndTreatment {
    ReturnToAvailable,
    CarryToNextSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputType {
    Narration,
    Image,
    Voice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationPolicy {
    FreeBeforeSend,
    ChargedIfDelivered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    Purchase,
    Allocation,
    Release,
    Usage,
    Credit,
    Refund,
    Dispute,
    Adjustment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PurchaseState {
    Pending,
    Succeeded,
    Failed,
}

/// The five balances of plan 9.7, in the caller's currency.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BalanceView {
    pub schema: &'static str,
    pub currency: String,
    pub purchased_available: MoneyView,
    pub promotional_available: MoneyView,
    pub allocated_to_sessions: MoneyView,
    pub reserved_for_pending_work: MoneyView,
    pub consumed_this_period: MoneyView,
    pub proposed_pilot: bool,
}

/// The caller's own allocation in a session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OwnAllocationView {
    pub authorized: MoneyView,
    pub drawn: MoneyView,
    pub authorization_remaining: MoneyView,
    pub consent: ConsentStatus,
}

/// A payer's consent line; the payer and amount appear only when the table shows contributions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConsentLineView {
    pub status: ConsentStatus,
    pub payer: Option<String>,
    pub amount: Option<MoneyView>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RunwayView {
    pub at_selected_maximum_ms: u64,
    pub at_recent_rate_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionFundingView {
    pub schema: &'static str,
    pub price_version_label: String,
    pub price_effective_at_ms: i64,
    pub hourly_ceiling: MoneyView,
    pub session_ceiling: Option<MoneyView>,
    pub pot_available: MoneyView,
    pub open_holds: MoneyView,
    pub spent: MoneyView,
    pub runway: Option<RunwayView>,
    pub funds_state: FundsState,
    pub own_allocation: Option<OwnAllocationView>,
    pub consent: Vec<ConsentLineView>,
    pub end_of_session: EndTreatment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BudgetChipView {
    pub schema: &'static str,
    pub pot: MoneyView,
    pub held: MoneyView,
    pub hourly_maximum: MoneyView,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LatencyRangeView {
    pub from_ms: u64,
    pub to_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuoteView {
    pub schema: &'static str,
    pub output_type: OutputType,
    pub estimate: MoneyView,
    pub maximum: MoneyView,
    pub own_share_estimate: MoneyView,
    pub expires_at_ms: i64,
    pub latency: Option<LatencyRangeView>,
    pub cancellation: CancellationPolicy,
    pub price_version_label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatementLineView {
    pub category: CategoryLabel,
    pub at_ms: i64,
    pub session: String,
    pub campaign: String,
    pub quantity_text: String,
    pub price_version_label: String,
    pub charge: MoneyView,
    pub reference: String,
    pub status: LineStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct StatementTotalsView {
    pub charged: MoneyView,
    pub credited: MoneyView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatementView {
    pub schema: &'static str,
    pub lines: Vec<StatementLineView>,
    pub totals: StatementTotalsView,
    pub provisional: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TransactionRowView {
    pub kind: TransactionKind,
    pub at_ms: i64,
    pub amount: MoneyView,
    pub reference: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TransactionListView {
    pub schema: &'static str,
    pub rows: Vec<TransactionRowView>,
    pub next_cursor: Option<String>,
}

/// A page of the caller's transactions; unknown fields are refused.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransactionListRequest {
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RefundLotView {
    pub lot: String,
    pub amount: MoneyView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RefundPreviewView {
    pub schema: &'static str,
    pub lots: Vec<RefundLotView>,
    pub released_allocations: MoneyView,
    pub pending_holds: MoneyView,
    pub tax: MoneyView,
    pub card_suffix: Option<String>,
    pub manual_payout: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PurchaseStatusView {
    pub schema: &'static str,
    pub state: PurchaseState,
    pub lot: Option<String>,
}

/// The admission outcome a customer-facing caller may see: what is held, never the supplier side.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AdmissionView {
    pub reservation: String,
    pub attempt: String,
    pub held: MoneyView,
    pub price_version_label: String,
}
