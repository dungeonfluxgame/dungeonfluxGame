//! The JSON contract between the account app and its server API (`df-portal` `app_api`).
//!
//! Customer types carry only what is paid (coordinator decision 26): pace price per table-hour,
//! estimates, maximums, actual charges per line and balances. They have no field for the table
//! fee, the multiplier or a provider price, so a customer reply cannot leak one. Operator types
//! (`Admin*`, `LedgerEntry`) keep the full breakdown. Money is integer micro-dollars or cents.

use serde::{Deserialize, Serialize};

use crate::wire_campaign::Reason;

/// Every API path starts with this.
pub const API_PREFIX: &str = "/app/api";
pub const CURRENT_TERMS_VERSION: &str = "terms-2026-10";
/// The header every mutating request carries its CSRF token in.
pub const CSRF_HEADER: &str = "x-df-csrf";
/// The CSRF form name the API's tokens are derived for.
pub const CSRF_FORM: &str = "app-api";
/// The header the display device calls (redeem, renew) carry instead of a CSRF token. A custom
/// header forces a CORS preflight the server never answers (S1 8.5 step 4).
pub const DEVICE_HEADER: &str = "x-df-device";
/// Longest operator reason accepted (B plan 10.1).
pub const MAXIMUM_REASON_CHARS: usize = 500;

/// Why a request was refused, closed so the app can say what happened and what to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// No valid session; sign in again.
    SignedOut,
    /// Signed in, but this needs a capability the account does not hold.
    Forbidden,
    /// The email address is not verified yet.
    NotVerified,
    /// The request failed the CSRF, origin or content-type check.
    CsrfRefused,
    Throttled,
    /// Missing, or belonging to someone else (indistinguishable on purpose).
    NotFound,
    /// A field was refused; `refusal` says which and why.
    Invalid,
    /// The form was open too long or was sent twice with different fields.
    StaleForm,
    /// Nothing changed; trying again later is safe.
    Unavailable,
    /// Too much work in flight; trying again shortly is safe.
    Busy,
    /// A change may or may not have happened; look before repeating it.
    UnknownOutcome,
}

/// A refused field value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    EmailEmpty,
    EmailTooLong,
    EmailMalformed,
    PasswordTooShort,
    PasswordTooLong,
    PasswordMatchesEmail,
    PasswordCommon,
    PasswordMismatch,
    WrongPassword,
    AdultRequired,
    TermsRequired,
    DisplayNameTooLong,
    DisplayNameControlCharacter,
    AmountOutOfRange,
    ReasonRequired,
    ReasonTooLong,
    SupportCaseRequired,
    CeilingOutOfRange,
    ConfirmationRequired,
    ConsentRequired,
    CampaignNameRequired,
    CampaignNameTooLong,
    AdventureRequired,
    ContentRatingRequired,
    SeatsOutOfRange,
    CodeRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
    /// Why a campaign request was refused (S1's closed set); beside `code`, never instead of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
}

impl ApiError {
    pub const fn of(code: ErrorCode) -> Self {
        Self {
            code,
            refusal: None,
            reason: None,
        }
    }

    pub const fn refused(refusal: Refusal) -> Self {
        Self {
            code: ErrorCode::Invalid,
            refusal: Some(refusal),
            reason: None,
        }
    }

    pub const fn because(code: ErrorCode, reason: Reason) -> Self {
        Self {
            code,
            refusal: None,
            reason: Some(reason),
        }
    }
}

/// Who is signed in, and the CSRF token for this session or anonymous visit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
    pub csrf: String,
    pub account: Option<AccountView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_sign_in: Option<Vec<DevSignIn>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevSignIn {
    pub label: String,
    pub email: String,
    pub password: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountView {
    pub email: String,
    pub display_name: Option<String>,
    pub is_verified: bool,
    /// Present only for operators; the server re-checks every capability on every request.
    pub operator: Option<OperatorView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperatorView {
    pub capabilities: Vec<OperatorCapability>,
    /// Whether this sign-in passed the operator second factor (plan 9.13).
    pub has_second_factor: bool,
}

/// What an operator may do in the console. Payer, host or owner never imply any of these.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorCapability {
    /// Read accounts, ledgers, disputes, reconciliation, providers and the audit log.
    SupportInspect,
    Refund,
    /// A goodwill credit (promotional class, never cash value); there is no "set balance".
    GoodwillCredit,
    DisputeResponse,
    /// Pause or resume a provider, independent of game read-only access.
    ProviderControl,
    ContentReview,
}

/// A fresh idempotency key for one form, issued by the server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperationView {
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignUpRequest {
    pub email: String,
    pub password: String,
    pub is_adult: bool,
    pub accepts_terms: bool,
    pub terms_version: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignInRequest {
    pub email: String,
    pub password: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    pub token: String,
}

/// Verification replaces the sign-up password: the mailbox owner chooses the password.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyEmailRequest {
    pub token: String,
    pub password: String,
    pub confirmation: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyOutcome {
    Verified,
    AlreadyVerified,
    /// Unknown, expired or used link: one answer for all.
    InvalidLink,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub password: String,
    pub confirmation: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetOutcome {
    Reset,
    InvalidLink,
}

/// An accepted request whose result arrives by email (the same reply for every address).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Accepted {
    pub accepted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OverviewView {
    pub balance: BalanceView,
    pub trial: TrialView,
}

/// The five amounts of plan 9.7, in micro-dollars.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BalanceView {
    pub purchased_available_micro_dollars: u64,
    pub promotional_available_micro_dollars: u64,
    pub allocated_micro_dollars: u64,
    pub reserved_micro_dollars: u64,
    pub consumed_this_month_micro_dollars: u64,
}

/// The free-play credit granted when the email address is verified (decision 3 and D3).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum TrialView {
    AwaitingVerification {
        amount_micro_dollars: u64,
    },
    Active {
        granted_micro_dollars: u64,
        remaining_micro_dollars: u64,
    },
    Spent {
        granted_micro_dollars: u64,
    },
    /// This address already received a trial on another account.
    NotEligible,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreditOffers {
    /// B's processor port is in test mode: nobody is charged a real amount (plan 17, PAY).
    pub is_test_mode: bool,
    pub preset_amounts_cents: Vec<u32>,
    pub custom_minimum_cents: u32,
    pub custom_maximum_cents: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCheckoutRequest {
    pub amount_cents: u32,
    pub operation: String,
}

/// Where the browser goes to pay. The return URL never credits anything (PAY-01).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckoutCreated {
    pub checkout_id: String,
    pub redirect_url: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutStatus {
    /// Paid or not, the processor has not told us yet.
    Pending,
    Paid,
    Failed,
    Cancelled,
    Expired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckoutView {
    pub checkout_id: String,
    pub amount_cents: u32,
    pub status: CheckoutStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacePreset {
    Story,
    Immersive,
    Cinematic,
    Custom,
}

/// A preset's table ceiling per active table-hour; `None` for Custom (the host sets it).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PresetOffer {
    pub preset: PacePreset,
    pub ceiling_cents_per_hour: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FundingView {
    pub presets: Vec<PresetOffer>,
    /// True until the owner approves pricing: prices are shown as "proposed pilot pricing".
    pub is_proposed_pricing: bool,
    pub custom_minimum_cents_per_hour: u32,
    pub custom_maximum_cents_per_hour: u32,
    pub campaigns: Vec<CampaignFunding>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CampaignFunding {
    pub campaign_id: String,
    pub name: String,
    /// Only the host chooses the table's pace; others see it.
    pub can_choose_pace: bool,
    pub preset: PacePreset,
    pub ceiling_cents_per_hour: u32,
    /// The open lobby or running session whose pot this account can add to, if any.
    pub session: Option<SessionFunding>,
}

/// What the table has, in what the table pays (plan 9.9; F section 18.9, customer-safe by type).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionFunding {
    pub session_id: String,
    pub pot_available_micro_dollars: u64,
    pub open_holds_micro_dollars: u64,
    pub spent_micro_dollars: u64,
    /// What this account has put in and what of it the host may still use.
    pub own_allocation_micro_dollars: u64,
    pub own_remaining_micro_dollars: u64,
    /// How long the pot lasts at the pace ceiling, and at the recent rate when there is one.
    pub runway_at_ceiling_minutes: Option<u32>,
    pub runway_at_recent_rate_minutes: Option<u32>,
    pub funds: FundsState,
    /// Whether other players see each person's contribution (plan 9.9).
    pub contributions_visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundsState {
    Funded,
    Low,
    FundingRequired,
}

/// Adds credit from the caller's own balance to a session's pot, with the consent the plan asks for.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocateRequest {
    pub session_id: String,
    pub amount_cents: u32,
    pub consents_to_host_spending: bool,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPaceRequest {
    pub campaign_id: String,
    pub preset: PacePreset,
    pub custom_ceiling_cents_per_hour: Option<u32>,
    pub operation: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementFilter {
    #[default]
    All,
    TopUps,
    Spend,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatementTotals {
    pub topped_up_micro_dollars: u64,
    pub promotional_granted_micro_dollars: u64,
    pub spent_micro_dollars: u64,
    pub refunded_micro_dollars: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MonthlySummary {
    /// `YYYY-MM` in UTC.
    pub month: String,
    pub topped_up_micro_dollars: u64,
    pub promotional_granted_micro_dollars: u64,
    pub spent_micro_dollars: u64,
    pub refunded_micro_dollars: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CampaignSpend {
    pub campaign_name: String,
    pub spent_micro_dollars: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatementsView {
    pub lines: Vec<StatementLine>,
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub totals: StatementTotals,
    #[serde(default)]
    pub monthly: Vec<MonthlySummary>,
    #[serde(default)]
    pub per_campaign: Vec<CampaignSpend>,
}

/// One customer-safe statement line (plan 7.2): what, when, where, how much, its status.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StatementLine {
    pub line_id: String,
    pub at_ms: u64,
    pub category: StatementCategory,
    pub campaign_name: Option<String>,
    pub quantity: Quantity,
    pub direction: Direction,
    pub amount_micro_dollars: u64,
    pub status: LineStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementCategory {
    TableTime,
    Narration,
    SceneImage,
    Portrait,
    Music,
    Purchase,
    TrialCredit,
    GoodwillCredit,
    TechnicalCredit,
    Refund,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "unit")]
pub enum Quantity {
    None,
    Seconds { seconds: u64 },
    AudioTenthsOfMinute { tenths: u64 },
    Images { count: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Adds to the balance (purchase, credit, refund reversal).
    Credit,
    /// Spends from the balance.
    Charge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineStatus {
    Settled,
    /// A hold or a session still settling: may still go down, never up.
    Provisional,
    Pending,
    Refunded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignInMethod {
    Password,
    Google,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosureState {
    Open,
    Closing,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EmailPreferences {
    pub product_news: bool,
    pub session_reminders: bool,
    /// Transactional mail is always sent; shown so the form cannot imply otherwise.
    pub transactional_always_on: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatus {
    NotRequested,
    Requested,
    Ready,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsView {
    pub email: String,
    pub display_name: Option<String>,
    pub sessions: Vec<DeviceSession>,
    pub closure_blockers: Vec<ClosureBlocker>,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub is_verified: bool,
    #[serde(default)]
    pub terms_version: Option<String>,
    #[serde(default)]
    pub terms_accepted_at_ms: Option<u64>,
    #[serde(default)]
    pub adult_attested_at_ms: Option<u64>,
    #[serde(default = "default_closure_state")]
    pub closure_state: ClosureState,
    #[serde(default)]
    pub sign_in_methods: Vec<SignInMethod>,
    #[serde(default)]
    pub email_preferences: EmailPreferences,
    #[serde(default = "default_export_status")]
    pub export_status: ExportStatus,
}

fn default_closure_state() -> ClosureState {
    ClosureState::Open
}

fn default_export_status() -> ExportStatus {
    ExportStatus::NotRequested
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeviceSession {
    #[serde(default)]
    pub session_id: String,
    pub device: DeviceKind,
    pub signed_in_at_ms: u64,
    pub last_seen_at_ms: u64,
    pub is_current: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Phone,
    Tablet,
    Desktop,
    Unknown,
}

/// What must be settled before the account can close (J14; S1 8.7).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ClosureBlocker {
    HostsActiveCampaign { campaign_name: String },
    PurchasedBalance { amount_micro_dollars: u64 },
    PendingHolds { amount_micro_dollars: u64 },
    OpenDispute,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRequest {
    pub display_name: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseAccountRequest {
    pub password: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeEmailRequest {
    pub new_email: String,
    pub password: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub password: String,
    pub confirmation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailPreferencesRequest {
    pub product_news: bool,
    pub session_reminders: bool,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub password: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExportView {
    pub status: ExportStatus,
    pub requested_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeSessionRequest {
    /// One sign-in session id, as listed in `SettingsView::sessions`.
    pub session_id: String,
}

/// Plain numbers for the dashboard, computed server-side from the account's own records.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccountStats {
    pub played_seconds_total: u64,
    pub played_seconds_this_month: u64,
    pub play_sessions: u64,
    pub campaigns_active: u64,
    pub campaigns_archived: u64,
    pub characters: u64,
    pub players_hosted: u64,
    pub assets_generated: u64,
    pub spent_this_month_micro_dollars: u64,
    pub spent_lifetime_micro_dollars: u64,
    /// Lifetime spend divided by total table-hours; zero when nothing was played.
    pub average_spend_per_table_hour_micro_dollars: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaySessionRow {
    pub session_id: String,
    pub campaign_id: String,
    pub campaign_name: String,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub duration_seconds: u64,
    pub players: u32,
    pub spent_micro_dollars: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaySessionsView {
    pub sessions: Vec<PlaySessionRow>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Portrait,
    Scene,
    Music,
    Video,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetHoldState {
    Ready,
    Held,
    Retained,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssetRow {
    pub asset_id: String,
    pub kind: AssetKind,
    pub campaign_id: Option<String>,
    pub campaign_name: Option<String>,
    pub created_at_ms: u64,
    pub size_bytes: u64,
    /// Thumbnail through the existing media URL scheme (`/app/assets/...` is the bundle;
    /// media bytes use this server-provided path).
    pub thumbnail_url: Option<String>,
    pub hold_state: AssetHoldState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssetsView {
    pub assets: Vec<AssetRow>,
    pub next_cursor: Option<String>,
    pub total: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClosedView {
    pub delete_after_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevokedView {
    pub revoked: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdminSummary {
    pub accounts: u64,
    pub verified_accounts: u64,
    pub customer_liability_micro_dollars: u64,
    pub open_disputes: u64,
    pub reconciliation_differences: u64,
    pub providers_degraded: u64,
    pub review_pending: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdminAccountRow {
    pub account_id: String,
    pub email_masked: String,
    pub display_name: Option<String>,
    pub is_verified: bool,
    pub standing: Standing,
    pub balance_micro_dollars: u64,
    #[serde(default)]
    pub last_activity_at_ms: Option<u64>,
    #[serde(default)]
    pub lifetime_topups_micro_dollars: u64,
    #[serde(default)]
    pub lifetime_spend_micro_dollars: u64,
    #[serde(default)]
    pub campaign_names: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    Active,
    Restricted,
    Closing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdminAccountDetail {
    pub account: AdminAccountRow,
    pub ledger: Vec<LedgerEntry>,
    pub refundable_lots: Vec<RefundableLot>,
    /// Unmasked only when the caller holds the inspect capability; every read is audited.
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub terms_version: Option<String>,
    #[serde(default)]
    pub terms_accepted_at_ms: Option<u64>,
    #[serde(default)]
    pub adult_attested_at_ms: Option<u64>,
    #[serde(default = "default_closure_state")]
    pub closure_state: ClosureState,
    #[serde(default)]
    pub sign_in_methods: Vec<SignInMethod>,
    #[serde(default)]
    pub sessions: Vec<DeviceSession>,
    #[serde(default)]
    pub play_sessions: Vec<PlaySessionRow>,
    #[serde(default)]
    pub assets: Vec<AssetRow>,
    #[serde(default)]
    pub balance: Option<BalanceView>,
    #[serde(default)]
    pub campaign_funding: Vec<CampaignFunding>,
    #[serde(default)]
    pub disputes: Vec<DisputeRow>,
    #[serde(default)]
    pub audit: Vec<AuditRow>,
    #[serde(default)]
    pub statement_totals: StatementTotals,
    #[serde(default)]
    pub monthly: Vec<MonthlySummary>,
}

/// Tabs of the operator account page (client-side selection only).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdminTab {
    #[default]
    Details,
    SignInSessions,
    PlaySessions,
    Assets,
    Funding,
    History,
    RefundsDisputes,
    Audit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminRevokeSessionRequest {
    pub session_id: String,
    pub reason: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminAssetHoldRequest {
    pub held: bool,
    pub reason: String,
    pub operation: String,
}

/// A journal line with the full breakdown, for operators only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub entry_id: String,
    pub at_ms: u64,
    pub kind: LedgerKind,
    pub campaign_name: Option<String>,
    pub retail_micro_dollars: u64,
    pub table_fee_micro_dollars: u64,
    pub multiplier_permille: u32,
    pub provider: Option<String>,
    pub provider_cost_micro_dollars: u64,
    pub price_version: String,
    pub status: LineStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerKind {
    Purchase,
    TrialGrant,
    GoodwillCredit,
    Usage,
    Refund,
    DisputeHold,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RefundableLot {
    pub lot_id: String,
    pub purchased_at_ms: u64,
    pub refundable_cents: u32,
    pub card_suffix: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefundRequest {
    pub account_id: String,
    pub lot_id: String,
    pub amount_cents: u32,
    pub reason: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodwillCreditRequest {
    pub account_id: String,
    pub amount_cents: u32,
    pub reason: String,
    pub support_case: String,
    pub operation: String,
}

/// An operator action that is either done or waiting on the processor; always audited.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdminActionView {
    pub audit_id: String,
    pub outcome: ActionOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcome {
    Done,
    /// Sent to the processor; the final status arrives by webhook.
    Pending,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DisputeRow {
    pub dispute_id: String,
    pub account_id: String,
    pub amount_cents: u32,
    pub status: DisputeStatus,
    pub opened_at_ms: u64,
    pub evidence_due_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisputeStatus {
    NeedsResponse,
    UnderReview,
    Won,
    Lost,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasonRequest {
    pub reason: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationView {
    pub last_run_at_ms: Option<u64>,
    pub differences: Vec<ReconciliationDifference>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationDifference {
    pub kind: ReconciliationKind,
    pub reference: String,
    pub difference_micro_dollars: u64,
    pub found_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationKind {
    ProcessorCapture,
    ProcessorRefund,
    Payout,
    SupplierUsage,
    AgedReservation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderRow {
    pub provider_id: String,
    pub name: String,
    pub health: ProviderHealth,
    pub error_rate_permille: u32,
    pub open_exposure_micro_dollars: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Healthy,
    Degraded,
    Paused,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub item_id: String,
    pub kind: ReviewKind,
    pub campaign_name: String,
    pub submitted_at_ms: u64,
    /// `staged_at + quarantine_max_age` (O2 6.2): after it the upload is rejected unreviewed.
    pub review_deadline_ms: u64,
    pub flag: ReviewFlag,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKind {
    UploadedPortrait,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFlag {
    PossibleRealPerson,
    PossibleExplicitContent,
    ScreenUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Reject,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecisionRequest {
    pub decision: ReviewDecision,
    pub reason: String,
    pub operation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuditRow {
    pub audit_id: String,
    pub at_ms: u64,
    pub operator: String,
    pub action: AuditAction,
    pub target: String,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    Refund,
    GoodwillCredit,
    DisputeEvidence,
    ProviderPaused,
    ProviderResumed,
    ReviewApproved,
    ReviewRejected,
    EmailViewed,
    SignInSessionRevoked,
    AssetHeld,
    AssetReleased,
}
