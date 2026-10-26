//! The ports `app_api` calls, each owned by the consumer (this module) and implemented by the
//! system that owns the data: finance (F), the processor checkout (B), campaigns (S1), operator
//! grants and audit (B round 3) and the upload review queue (O2). `fakes` holds a faithful
//! in-memory implementation of each for tests and the preview.
//!
//! Customer reads return the customer-safe wire views directly: those types have no field for a
//! table fee, a multiplier or a provider price (decision 26), so a port cannot leak one. Only the
//! operator methods return the full breakdown.

use std::future::Future;

use df_account_app::{
    api::ProviderSwitch,
    wire::{
        AccountStats, ActionOutcome, ApiError, AssetKind, AssetsView, AuditAction, AuditRow,
        CheckoutCreated, CheckoutView, ClosureBlocker, CreditOffers, DisputeRow, EmailPreferences,
        ErrorCode, ExportStatus, FundingView, LedgerEntry, OperatorCapability, OverviewView,
        PacePreset, PlaySessionsView, ProviderRow, ReconciliationView, RefundableLot, Refusal,
        ReviewDecision, ReviewItem, StatementFilter, StatementsView,
    },
    wire_campaign::{
        CampaignDraft, CampaignOverview, CharacterRequest, DashboardView, DisplayChallenge,
        DisplayRedeemed, DisplayRenewed, DisplayState, InvitationCreated, InvitationKind,
        InvitePreview, JoinRequested, LobbyView, MicrophoneRequest, PairingLookup, Reason,
        WizardCatalog,
    },
};
use df_customer::DeviceLabel;
use df_types::{AccountId, CampaignId, CheckoutId, OperationId, SessionId};

/// Why a port call did not produce its value. Maps one to one onto the API envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortError {
    /// Missing, or someone else's: the same answer for both.
    NotFound,
    Forbidden,
    Throttled,
    /// Nothing changed; trying again later is safe.
    Unavailable,
    Busy,
    /// A change may or may not have happened.
    UnknownOutcome,
    /// A field was refused.
    Refused(Refusal),
    /// A campaign request was refused for one of S1's closed reasons.
    Because(ErrorCode, Reason),
}

impl PortError {
    pub const fn api_error(self) -> ApiError {
        match self {
            Self::NotFound => ApiError::of(ErrorCode::NotFound),
            Self::Forbidden => ApiError::of(ErrorCode::Forbidden),
            Self::Throttled => ApiError::of(ErrorCode::Throttled),
            Self::Unavailable => ApiError::of(ErrorCode::Unavailable),
            Self::Busy => ApiError::of(ErrorCode::Busy),
            Self::UnknownOutcome => ApiError::of(ErrorCode::UnknownOutcome),
            Self::Refused(refusal) => ApiError::refused(refusal),
            Self::Because(code, reason) => ApiError::because(code, reason),
        }
    }
}

pub type PortResult<T> = Result<T, PortError>;

/// A campaign the caller can see, with what funding needs to know about it (S1 owns membership,
/// F owns the money).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundingTarget {
    pub campaign: CampaignId,
    pub name: String,
    /// Only the host chooses the table's pace.
    pub can_choose_pace: bool,
    /// The open lobby or running session whose pot the caller can add to.
    pub live_session: Option<SessionId>,
}

/// The signed-in person as the campaign directory needs to know them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Actor {
    pub account: AccountId,
    pub display_name: String,
    pub is_verified: bool,
}

/// The finance owner's headline numbers for the operator console.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinanceSummary {
    pub customer_liability_micro_dollars: u64,
    pub open_disputes: u64,
    pub reconciliation_differences: u64,
    pub providers_degraded: u64,
}

/// Who is acting, why, and under which idempotency key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorAct {
    pub operator: AccountId,
    pub reason: String,
    pub operation: OperationId,
}

/// Finance (F): balances, the trial, funding, statements and the operator ledger.
pub trait FinancePort: Send + Sync + 'static {
    /// Durably cancels a host's unstarted lobby, then returns its unreserved credit.
    fn cancel_host_lobby(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> impl Future<Output = PortResult<()>> + Send;

    /// Only the current host sees their own credit and a versioned customer price.
    fn host_credit(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> impl Future<Output = PortResult<Option<df_account_app::wire_campaign::HostCreditView>>> + Send;

    /// Records explicit consent without allocating credit. The session's host is rechecked.
    fn consent_host_credit(
        &self,
        account: AccountId,
        session: SessionId,
        request: df_account_app::wire_campaign::HostConsentRequest,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<()>> + Send;

    /// The five balances and the trial. A verified account's trial is granted once, on the first
    /// call after verification, and never again (TODO(F): the verification hook posts it).
    fn overview(
        &self,
        account: AccountId,
        is_verified: bool,
    ) -> impl Future<Output = PortResult<OverviewView>> + Send;

    fn funding(
        &self,
        account: AccountId,
        targets: &[FundingTarget],
    ) -> impl Future<Output = PortResult<FundingView>> + Send;

    fn set_pace(
        &self,
        account: AccountId,
        campaign: CampaignId,
        preset: PacePreset,
        custom_ceiling_cents_per_hour: Option<u32>,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<()>> + Send;

    /// Moves `amount_cents` of the caller's own credit into a session's pot.
    fn allocate(
        &self,
        account: AccountId,
        session: SessionId,
        amount_cents: u32,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<()>> + Send;

    fn statements(
        &self,
        account: AccountId,
        cursor: Option<&str>,
    ) -> impl Future<Output = PortResult<StatementsView>> + Send;

    /// Statements with a direction filter and an inclusive date range; totals, the per-month
    /// summary and per-campaign spend cover the whole filtered range, not just the page.
    fn statements_filtered(
        &self,
        account: AccountId,
        filter: StatementFilter,
        from_ms: Option<u64>,
        to_ms: Option<u64>,
        cursor: Option<&str>,
    ) -> impl Future<Output = PortResult<StatementsView>> + Send;

    /// Money parts of the dashboard stats; the handler fills play, campaign and asset counts
    /// from their own ports.
    fn account_stats(
        &self,
        account: AccountId,
        now_ms: u64,
    ) -> impl Future<Output = PortResult<AccountStats>> + Send;

    /// Lifetime top-ups, lifetime spend and the last money activity, for the operator list.
    fn operator_account_totals(
        &self,
        account: AccountId,
    ) -> impl Future<Output = PortResult<(u64, u64, Option<u64>)>> + Send;

    /// What the money side requires before the account can close.
    fn closure_blockers(
        &self,
        account: AccountId,
    ) -> impl Future<Output = PortResult<Vec<ClosureBlocker>>> + Send;

    fn operator_summary(&self) -> impl Future<Output = PortResult<FinanceSummary>> + Send;

    fn operator_balance(&self, account: AccountId) -> impl Future<Output = PortResult<u64>> + Send;

    /// The full journal and the lots a refund can draw on.
    fn operator_ledger(
        &self,
        account: AccountId,
    ) -> impl Future<Output = PortResult<(Vec<LedgerEntry>, Vec<RefundableLot>)>> + Send;

    /// A refund of at most the lot's refundable amount.
    fn refund(
        &self,
        act: OperatorAct,
        account: AccountId,
        lot_id: &str,
        amount_cents: u32,
    ) -> impl Future<Output = PortResult<ActionOutcome>> + Send;

    /// A promotional-class credit with no cash value.
    fn goodwill_credit(
        &self,
        act: OperatorAct,
        account: AccountId,
        amount_cents: u32,
        support_case: &str,
    ) -> impl Future<Output = PortResult<ActionOutcome>> + Send;

    fn disputes(&self) -> impl Future<Output = PortResult<Vec<DisputeRow>>> + Send;

    fn dispute_evidence(
        &self,
        act: OperatorAct,
        dispute_id: &str,
    ) -> impl Future<Output = PortResult<ActionOutcome>> + Send;

    fn reconciliation(&self) -> impl Future<Output = PortResult<ReconciliationView>> + Send;

    /// Providers with the full breakdown: operators only.
    fn providers(&self) -> impl Future<Output = PortResult<Vec<ProviderRow>>> + Send;

    fn switch_provider(
        &self,
        act: OperatorAct,
        provider_id: &str,
        switch: ProviderSwitch,
    ) -> impl Future<Output = PortResult<ActionOutcome>> + Send;
}

/// The processor's hosted checkout (B). Creating a checkout never credits; only the processor's
/// webhook does, and `status` only reports what the webhook recorded.
pub trait CheckoutPort: Send + Sync + 'static {
    fn offers(&self) -> CreditOffers;

    fn create(
        &self,
        account: AccountId,
        amount_cents: u32,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<CheckoutCreated>> + Send;

    /// Another account's checkout is `NotFound`, the same as a missing one.
    fn status(
        &self,
        account: AccountId,
        checkout: CheckoutId,
    ) -> impl Future<Output = PortResult<CheckoutView>> + Send;
}

/// A host's answer to a room-code request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestDecision {
    Approve(InvitationKind),
    Deny,
}

/// The device secret of a pairing and how long its cookie lives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceSecret {
    pub secret: String,
    pub max_age_s: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedChallenge {
    pub challenge: DisplayChallenge,
    pub device: DeviceSecret,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedeemedTicket {
    pub redeemed: DisplayRedeemed,
    /// The ticket cookie's value and its remaining life.
    pub ticket: DeviceSecret,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenewedTicket {
    pub renewed: DisplayRenewed,
    pub ticket: DeviceSecret,
}

/// Campaigns, invitations, the lobby and display pairing (S1). Every id from the caller is
/// checked against the caller's membership; a foreign id is `NotFound`.
pub trait CampaignDirectory: Send + Sync + 'static {
    fn dashboard(&self, who: &Actor) -> impl Future<Output = PortResult<DashboardView>> + Send;

    /// Play history of one account, newest first, paged by an opaque cursor (S1 owns the data).
    fn play_sessions(
        &self,
        account: AccountId,
        cursor: Option<&str>,
    ) -> impl Future<Output = PortResult<PlaySessionsView>> + Send;

    fn funding_targets(
        &self,
        account: AccountId,
    ) -> impl Future<Output = PortResult<Vec<FundingTarget>>> + Send;

    /// Names of the active campaigns the account hosts or owns (closing is blocked by them).
    fn closure_blockers(
        &self,
        account: AccountId,
    ) -> impl Future<Output = PortResult<Vec<String>>> + Send;

    fn wizard_catalog(&self) -> impl Future<Output = PortResult<WizardCatalog>> + Send;

    fn start_draft(
        &self,
        who: &Actor,
    ) -> impl Future<Output = PortResult<(String, CampaignDraft)>> + Send;

    fn save_draft(
        &self,
        who: &Actor,
        draft_id: &str,
        step: u8,
        draft: CampaignDraft,
    ) -> impl Future<Output = PortResult<CampaignDraft>> + Send;

    fn finish_draft(
        &self,
        who: &Actor,
        draft_id: &str,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<CampaignId>> + Send;

    fn campaign(
        &self,
        who: &Actor,
        campaign: CampaignId,
    ) -> impl Future<Output = PortResult<CampaignOverview>> + Send;

    /// The link is shown once; the directory keeps only a digest of it.
    fn create_invitation(
        &self,
        who: &Actor,
        campaign: CampaignId,
        kind: InvitationKind,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<InvitationCreated>> + Send;

    fn revoke_invitation(
        &self,
        who: &Actor,
        campaign: CampaignId,
        invitation_id: &str,
    ) -> impl Future<Output = PortResult<CampaignOverview>> + Send;

    fn rotate_code(
        &self,
        who: &Actor,
        campaign: CampaignId,
    ) -> impl Future<Output = PortResult<CampaignOverview>> + Send;

    fn decide_join_request(
        &self,
        who: &Actor,
        campaign: CampaignId,
        request_id: &str,
        decision: RequestDecision,
    ) -> impl Future<Output = PortResult<CampaignOverview>> + Send;

    fn leave(
        &self,
        who: &Actor,
        campaign: CampaignId,
    ) -> impl Future<Output = PortResult<()>> + Send;

    fn open_lobby(
        &self,
        who: &Actor,
        campaign: CampaignId,
    ) -> impl Future<Output = PortResult<SessionId>> + Send;

    fn lobby(
        &self,
        who: &Actor,
        session: SessionId,
    ) -> impl Future<Output = PortResult<LobbyView>> + Send;

    fn choose_character(
        &self,
        who: &Actor,
        session: SessionId,
        request: CharacterRequest,
    ) -> impl Future<Output = PortResult<LobbyView>> + Send;

    fn set_ready(
        &self,
        who: &Actor,
        session: SessionId,
        is_ready: bool,
    ) -> impl Future<Output = PortResult<LobbyView>> + Send;

    fn report_microphone(
        &self,
        who: &Actor,
        session: SessionId,
        report: MicrophoneRequest,
    ) -> impl Future<Output = PortResult<LobbyView>> + Send;

    fn start_session(
        &self,
        who: &Actor,
        session: SessionId,
    ) -> impl Future<Output = PortResult<LobbyView>> + Send;

    /// Lookup never approves: the host confirms after seeing which device is waiting.
    fn lookup_display(
        &self,
        who: &Actor,
        session: SessionId,
        code: &str,
    ) -> impl Future<Output = PortResult<PairingLookup>> + Send;

    fn approve_display(
        &self,
        who: &Actor,
        session: SessionId,
        pairing_id: &str,
    ) -> impl Future<Output = PortResult<LobbyView>> + Send;

    fn unpair_display(
        &self,
        who: &Actor,
        session: SessionId,
        ticket_id: &str,
    ) -> impl Future<Output = PortResult<LobbyView>> + Send;

    fn unpair_all_displays(
        &self,
        who: &Actor,
        session: SessionId,
    ) -> impl Future<Output = PortResult<LobbyView>> + Send;

    /// The preview of a usable invitation; every failure omits it and carries only a reason.
    fn join_preview(&self, token: &str) -> impl Future<Output = PortResult<InvitePreview>> + Send;

    fn join_accept(
        &self,
        who: &Actor,
        token: &str,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<CampaignId>> + Send;

    fn code_lookup(
        &self,
        who: &Actor,
        code: &str,
    ) -> impl Future<Output = PortResult<InvitePreview>> + Send;

    fn code_request(
        &self,
        who: &Actor,
        code: &str,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<JoinRequested>> + Send;

    fn display_challenge(
        &self,
        device: DeviceLabel,
    ) -> impl Future<Output = PortResult<IssuedChallenge>> + Send;

    fn display_status(
        &self,
        device_secret: &str,
    ) -> impl Future<Output = PortResult<DisplayState>> + Send;

    fn display_redeem(
        &self,
        device_secret: &str,
    ) -> impl Future<Output = PortResult<RedeemedTicket>> + Send;

    fn display_renew(
        &self,
        device_secret: &str,
        ticket: &str,
    ) -> impl Future<Output = PortResult<RenewedTicket>> + Send;
}

/// What an account may do in the operator console.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorGrant {
    pub capabilities: Vec<OperatorCapability>,
    /// Whether the operator passed the second factor for this sign-in.
    pub has_second_factor: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEntry {
    pub operator: AccountId,
    pub operator_label: String,
    pub action: AuditAction,
    pub target: String,
    pub reason: String,
    pub operation: OperationId,
}

/// Operator grants, second-factor state and the audit log (B round 3).
pub trait OperatorDirectory: Send + Sync + 'static {
    fn grant(
        &self,
        account: AccountId,
    ) -> impl Future<Output = PortResult<Option<OperatorGrant>>> + Send;

    /// Appends the entry; a repeated operation id returns the first entry's id.
    fn append_audit(&self, entry: AuditEntry) -> impl Future<Output = PortResult<String>> + Send;

    fn audit(&self) -> impl Future<Output = PortResult<Vec<AuditRow>>> + Send;

    /// Revokes one sign-in session of another account (B owns the real session store).
    fn revoke_account_session(
        &self,
        act: OperatorAct,
        account: AccountId,
        session_id: &str,
    ) -> impl Future<Output = PortResult<ActionOutcome>> + Send;
}

/// Generated media of an account (O2's media catalog owns the data; see `df-assets`
/// `AssetRecord`/`AssetKind`/`AssetState` in `catalog_record.rs` and `CatalogService`).
pub trait AssetsPort: Send + Sync + 'static {
    fn list(
        &self,
        account: AccountId,
        kind: Option<AssetKind>,
        campaign_id: Option<&str>,
        cursor: Option<&str>,
    ) -> impl Future<Output = PortResult<AssetsView>> + Send;

    /// Holds or releases one asset of the account; a foreign asset is `NotFound`.
    fn set_hold(
        &self,
        act: OperatorAct,
        account: AccountId,
        asset_id: &str,
        held: bool,
    ) -> impl Future<Output = PortResult<ActionOutcome>> + Send;
}

/// Email preferences and data-export status (B's account service owns the real store).
pub trait AccountExtrasPort: Send + Sync + 'static {
    fn preferences(
        &self,
        account: AccountId,
    ) -> impl Future<Output = PortResult<EmailPreferences>> + Send;

    fn save_preferences(
        &self,
        account: AccountId,
        preferences: EmailPreferences,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<()>> + Send;

    fn export_status(
        &self,
        account: AccountId,
    ) -> impl Future<Output = PortResult<(ExportStatus, Option<u64>)>> + Send;

    fn request_export(
        &self,
        account: AccountId,
        operation: OperationId,
    ) -> impl Future<Output = PortResult<(ExportStatus, Option<u64>)>> + Send;
}

/// The uploaded-portrait review queue (O2).
pub trait ReviewQueue: Send + Sync + 'static {
    fn pending(&self) -> impl Future<Output = PortResult<Vec<ReviewItem>>> + Send;

    fn decide(
        &self,
        act: OperatorAct,
        item_id: &str,
        decision: ReviewDecision,
    ) -> impl Future<Output = PortResult<ActionOutcome>> + Send;
}

/// The five ports a deployment composes. One bundle keeps the state and handlers at three type
/// parameters; the fakes are one implementation and composition supplies the real one.
pub trait Ports: Send + Sync + 'static {
    type Finance: FinancePort;
    type Checkout: CheckoutPort;
    type Campaigns: CampaignDirectory;
    type Operators: OperatorDirectory;
    type Review: ReviewQueue;
    type Assets: AssetsPort;
    type Extras: AccountExtrasPort;

    fn finance(&self) -> &Self::Finance;
    fn checkout(&self) -> &Self::Checkout;
    fn campaigns(&self) -> &Self::Campaigns;
    fn operators(&self) -> &Self::Operators;
    fn review(&self) -> &Self::Review;
    fn assets(&self) -> &Self::Assets;
    fn extras(&self) -> &Self::Extras;
}
