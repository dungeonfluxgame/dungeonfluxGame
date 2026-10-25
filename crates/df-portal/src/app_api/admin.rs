//! The operator console's endpoints. Every route re-checks, on every request, that the caller
//! holds a current grant with the second factor and the capability the route needs; payer, host
//! or owner never imply one. A mutation also needs a reason of 1 to 500 characters and writes an
//! audit entry keyed by its operation id. A customer never reaches the operator types.

use axum::extract::{Path, Query, State, rejection::QueryRejection};
use df_account_app::{
    api::ProviderSwitch,
    wire::{
        ActionOutcome, AdminAccountDetail, AdminAccountRow, AdminActionView, AdminAssetHoldRequest,
        AdminRevokeSessionRequest, AdminSummary, ApiError, AuditAction, AuditRow, ClosureState,
        DisputeRow, ErrorCode, GoodwillCreditRequest, MAXIMUM_REASON_CHARS, OperatorCapability,
        ProviderRow, ReasonRequest, ReconciliationView, RefundRequest, Refusal, ReviewDecision,
        ReviewDecisionRequest, ReviewItem, SignInMethod, Standing,
    },
};
use df_customer::{AccountStatus, AccountStore, Authenticated, EmailSender, RecentAccount};
use df_types::{AccountId, OperationId};
use serde::Deserialize;

use super::{
    guard::{ApiBody, ApiFailure, ApiResult, SessionPost, SignedIn, reply},
    ids,
    ports::{
        AssetsPort, AuditEntry, CampaignDirectory, FinancePort, OperatorAct, OperatorDirectory,
        Ports, ReviewQueue,
    },
    state::AppApiState,
};

/// An operator who passed every check for one route.
struct Operator {
    account: AccountId,
    label: String,
}

fn refused(refusal: Refusal) -> ApiFailure {
    ApiFailure::new(ApiError::refused(refusal))
}

/// Forbidden unless the caller holds a grant, passed the second factor and has `capability`.
async fn operator<S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    caller: &Authenticated,
    capability: OperatorCapability,
) -> Result<Operator, ApiFailure> {
    state
        .ports
        .operators()
        .grant(caller.account.id)
        .await?
        .filter(|grant| grant.has_second_factor && grant.capabilities.contains(&capability))
        .ok_or_else(|| ApiFailure::of(ErrorCode::Forbidden))?;
    Ok(Operator {
        account: caller.account.id,
        label: caller
            .account
            .email
            .as_ref()
            .map_or_else(String::new, |email| email.masked()),
    })
}

/// The reason an operator gives: 1 to 500 characters once trimmed.
fn reason_of(text: &str) -> Result<String, ApiFailure> {
    let reason = text.trim();
    if reason.is_empty() {
        return Err(refused(Refusal::ReasonRequired));
    }
    if reason.chars().count() > MAXIMUM_REASON_CHARS {
        return Err(refused(Refusal::ReasonTooLong));
    }
    Ok(reason.to_owned())
}

fn act_of(operator: &Operator, reason: &str, operation: OperationId) -> OperatorAct {
    OperatorAct {
        operator: operator.account,
        reason: reason.to_owned(),
        operation,
    }
}

/// Records the audit entry of a mutation that already happened. A failure here is reported as
/// an unknown outcome: the action stands, and the operator must look before repeating it.
async fn audited<S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    operator: &Operator,
    action: AuditAction,
    target: String,
    reason: String,
    operation: OperationId,
    outcome: ActionOutcome,
) -> ApiResult<AdminActionView> {
    let audit_id = state
        .ports
        .operators()
        .append_audit(AuditEntry {
            operator: operator.account,
            operator_label: operator.label.clone(),
            action,
            target,
            reason,
            operation,
        })
        .await
        .map_err(|_| ApiFailure::of(ErrorCode::UnknownOutcome))?;
    reply(AdminActionView { audit_id, outcome })
}

pub(super) async fn summary<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<AdminSummary> {
    operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    let accounts = state.service.dev_summary().await?;
    let finance = state.ports.finance().operator_summary().await?;
    let review_pending = state.ports.review().pending().await?.len() as u64;
    reply(AdminSummary {
        accounts: accounts.active + accounts.closing + accounts.closed,
        verified_accounts: accounts.verified,
        customer_liability_micro_dollars: finance.customer_liability_micro_dollars,
        open_disputes: finance.open_disputes,
        reconciliation_differences: finance.reconciliation_differences,
        providers_degraded: finance.providers_degraded,
        review_pending,
    })
}

const fn standing_of(status: AccountStatus) -> Standing {
    match status {
        AccountStatus::Active => Standing::Active,
        AccountStatus::Closing | AccountStatus::Closed => Standing::Closing,
    }
}

async fn account_row<S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    recent: &RecentAccount,
) -> Result<AdminAccountRow, ApiFailure> {
    let (topups, spend, last_activity) = state
        .ports
        .finance()
        .operator_account_totals(recent.id)
        .await?;
    let targets = state
        .ports
        .campaigns()
        .funding_targets(recent.id)
        .await
        .unwrap_or_default();
    Ok(AdminAccountRow {
        account_id: ids::account_text(recent.id),
        email_masked: recent
            .email
            .as_ref()
            .map_or_else(|| "closed".to_owned(), |email| email.masked()),
        display_name: None,
        is_verified: recent.is_verified,
        standing: standing_of(recent.status),
        balance_micro_dollars: state.ports.finance().operator_balance(recent.id).await?,
        last_activity_at_ms: last_activity,
        lifetime_topups_micro_dollars: topups,
        lifetime_spend_micro_dollars: spend,
        campaign_names: targets.iter().map(|target| target.name.clone()).collect(),
    })
}

#[derive(Deserialize)]
pub(super) struct AccountsQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    standing: Option<String>,
    #[serde(default)]
    verified: Option<String>,
    #[serde(default)]
    has_balance: Option<String>,
    #[serde(default)]
    sort: Option<String>,
}

/// TODO(B): the account store has no exact-match lookup yet, so search covers the most recent
/// accounts the development summary lists. B's `OperatorDirectory` replaces it.
pub(super) async fn accounts<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    query: Result<Query<AccountsQuery>, QueryRejection>,
) -> ApiResult<Vec<AdminAccountRow>> {
    operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    let Query(query) = query.map_err(|_| ApiFailure::of(ErrorCode::Invalid))?;
    let needle = query.q.trim().to_ascii_lowercase();
    let summary = state.service.dev_summary().await?;
    let mut rows = Vec::new();
    for recent in &summary.recent {
        let row = account_row(&state, recent).await?;
        let is_match = needle.is_empty()
            || row.account_id == needle
            || recent
                .email
                .as_ref()
                .is_some_and(|email| email.normalized().contains(&needle))
            || row
                .campaign_names
                .iter()
                .any(|name| name.to_ascii_lowercase().contains(&needle));
        let standing_ok = match query.standing.as_deref() {
            None | Some("") => true,
            Some("active") => row.standing == Standing::Active,
            Some("restricted") => row.standing == Standing::Restricted,
            Some("closing") => row.standing == Standing::Closing,
            Some(_) => return Err(ApiFailure::of(ErrorCode::Invalid)),
        };
        let verified_ok = match query.verified.as_deref() {
            None | Some("") => true,
            Some("yes" | "true") => row.is_verified,
            Some("no" | "false") => !row.is_verified,
            Some(_) => return Err(ApiFailure::of(ErrorCode::Invalid)),
        };
        let balance_ok = match query.has_balance.as_deref() {
            None | Some("") => true,
            Some("yes" | "true") => row.balance_micro_dollars > 0,
            Some("no" | "false") => row.balance_micro_dollars == 0,
            Some(_) => return Err(ApiFailure::of(ErrorCode::Invalid)),
        };
        if is_match && standing_ok && verified_ok && balance_ok {
            rows.push(row);
        }
    }
    match query.sort.as_deref() {
        None | Some("") | Some("last_activity") => {
            rows.sort_by_key(|row| std::cmp::Reverse(row.last_activity_at_ms));
        }
        Some("balance") => rows.sort_by_key(|row| std::cmp::Reverse(row.balance_micro_dollars)),
        Some("lifetime_topups") => {
            rows.sort_by_key(|row| std::cmp::Reverse(row.lifetime_topups_micro_dollars));
        }
        Some("lifetime_spend") => {
            rows.sort_by_key(|row| std::cmp::Reverse(row.lifetime_spend_micro_dollars));
        }
        Some(_) => return Err(ApiFailure::of(ErrorCode::Invalid)),
    }
    rows.truncate(100);
    reply(rows)
}

pub(super) async fn account<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    Path(account_id): Path<String>,
) -> ApiResult<AdminAccountDetail> {
    let operator = operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    let id = ids::account(&account_id)?;
    let summary = state.service.dev_summary().await?;
    let recent = summary
        .recent
        .iter()
        .find(|recent| recent.id == id)
        .ok_or_else(|| ApiFailure::of(ErrorCode::NotFound))?;
    let (ledger, refundable_lots) = state.ports.finance().operator_ledger(id).await?;
    let row = account_row(&state, recent).await?;
    // Every read of the unmasked email is audited.
    let operation = state.service.new_operation()?;
    let _audit = state
        .ports
        .operators()
        .append_audit(AuditEntry {
            operator: operator.account,
            operator_label: operator.label.clone(),
            action: AuditAction::EmailViewed,
            target: account_id.clone(),
            reason: "account detail viewed".to_owned(),
            operation,
        })
        .await;
    let overview = state
        .ports
        .finance()
        .overview(id, recent.is_verified)
        .await?;
    let targets = state
        .ports
        .campaigns()
        .funding_targets(id)
        .await
        .unwrap_or_default();
    let funding = state.ports.finance().funding(id, &targets).await?;
    let play = state
        .ports
        .campaigns()
        .play_sessions(id, None)
        .await
        .unwrap_or(df_account_app::wire::PlaySessionsView {
            sessions: Vec::new(),
            next_cursor: None,
        });
    let assets = state
        .ports
        .assets()
        .list(id, None, None, None)
        .await
        .unwrap_or(df_account_app::wire::AssetsView {
            assets: Vec::new(),
            next_cursor: None,
            total: 0,
        });
    let statements = state
        .ports
        .finance()
        .statements_filtered(
            id,
            df_account_app::wire::StatementFilter::All,
            None,
            None,
            None,
        )
        .await?;
    let all_audit = state.ports.operators().audit().await?;
    let account_audit: Vec<AuditRow> = all_audit
        .into_iter()
        .filter(|row| row.target.contains(&account_id))
        .collect();
    let disputes: Vec<df_account_app::wire::DisputeRow> = state
        .ports
        .finance()
        .disputes()
        .await?
        .into_iter()
        .filter(|dispute| dispute.account_id == account_id)
        .collect();
    reply(AdminAccountDetail {
        account: row,
        ledger,
        refundable_lots,
        email: recent
            .email
            .as_ref()
            .map(|email| email.display().to_owned()),
        created_at_ms: 0,
        terms_version: None,
        terms_accepted_at_ms: None,
        adult_attested_at_ms: None,
        closure_state: match recent.status {
            AccountStatus::Active => ClosureState::Open,
            AccountStatus::Closing | AccountStatus::Closed => ClosureState::Closing,
        },
        sign_in_methods: vec![SignInMethod::Password],
        sessions: Vec::new(),
        play_sessions: play.sessions,
        assets: assets.assets,
        balance: Some(overview.balance),
        campaign_funding: funding.campaigns,
        disputes,
        audit: account_audit,
        statement_totals: statements.totals,
        monthly: statements.monthly,
    })
}

pub(super) async fn revoke_account_session<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(account_id): Path<String>,
    ApiBody(body): ApiBody<AdminRevokeSessionRequest>,
) -> ApiResult<AdminActionView> {
    let operator = operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    let reason = reason_of(&body.reason)?;
    let account = ids::account(&account_id)?;
    let operation = ids::operation(&body.operation)?;
    let outcome = state
        .ports
        .operators()
        .revoke_account_session(
            act_of(&operator, &reason, operation),
            account,
            &body.session_id,
        )
        .await?;
    audited(
        &state,
        &operator,
        AuditAction::SignInSessionRevoked,
        format!("{account_id}:{}", body.session_id),
        reason,
        operation,
        outcome,
    )
    .await
}

async fn set_asset_hold<S: AccountStore, M: EmailSender, P: Ports>(
    state: AppApiState<S, M, P>,
    caller: df_customer::Authenticated,
    account_id: String,
    asset_id: String,
    body: AdminAssetHoldRequest,
    held: bool,
) -> ApiResult<AdminActionView> {
    let operator = operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    let reason = reason_of(&body.reason)?;
    let account = ids::account(&account_id)?;
    let operation = ids::operation(&body.operation)?;
    let outcome = state
        .ports
        .assets()
        .set_hold(
            act_of(&operator, &reason, operation),
            account,
            &asset_id,
            held,
        )
        .await?;
    audited(
        &state,
        &operator,
        if held {
            AuditAction::AssetHeld
        } else {
            AuditAction::AssetReleased
        },
        format!("{account_id}:{asset_id}"),
        reason,
        operation,
        outcome,
    )
    .await
}

pub(super) async fn hold_asset<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path((account_id, asset_id)): Path<(String, String)>,
    ApiBody(body): ApiBody<AdminAssetHoldRequest>,
) -> ApiResult<AdminActionView> {
    set_asset_hold(state, caller, account_id, asset_id, body, true).await
}

pub(super) async fn release_asset<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path((account_id, asset_id)): Path<(String, String)>,
    ApiBody(body): ApiBody<AdminAssetHoldRequest>,
) -> ApiResult<AdminActionView> {
    set_asset_hold(state, caller, account_id, asset_id, body, false).await
}

pub(super) async fn refund<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<RefundRequest>,
) -> ApiResult<AdminActionView> {
    let operator = operator(&state, &caller, OperatorCapability::Refund).await?;
    let reason = reason_of(&body.reason)?;
    let account = ids::account(&body.account_id)?;
    let operation = ids::operation(&body.operation)?;
    if body.amount_cents == 0 {
        return Err(refused(Refusal::AmountOutOfRange));
    }
    let outcome = state
        .ports
        .finance()
        .refund(
            act_of(&operator, &reason, operation),
            account,
            &body.lot_id,
            body.amount_cents,
        )
        .await?;
    audited(
        &state,
        &operator,
        AuditAction::Refund,
        format!("{}:{}", body.account_id, body.lot_id),
        reason,
        operation,
        outcome,
    )
    .await
}

pub(super) async fn goodwill_credit<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<GoodwillCreditRequest>,
) -> ApiResult<AdminActionView> {
    let operator = operator(&state, &caller, OperatorCapability::GoodwillCredit).await?;
    let reason = reason_of(&body.reason)?;
    let account = ids::account(&body.account_id)?;
    let operation = ids::operation(&body.operation)?;
    if body.support_case.trim().is_empty() {
        return Err(refused(Refusal::SupportCaseRequired));
    }
    if body.amount_cents == 0 {
        return Err(refused(Refusal::AmountOutOfRange));
    }
    let outcome = state
        .ports
        .finance()
        .goodwill_credit(
            act_of(&operator, &reason, operation),
            account,
            body.amount_cents,
            body.support_case.trim(),
        )
        .await?;
    audited(
        &state,
        &operator,
        AuditAction::GoodwillCredit,
        body.account_id,
        reason,
        operation,
        outcome,
    )
    .await
}

pub(super) async fn disputes<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<Vec<DisputeRow>> {
    operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    reply(state.ports.finance().disputes().await?)
}

pub(super) async fn dispute_evidence<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(dispute_id): Path<String>,
    ApiBody(body): ApiBody<ReasonRequest>,
) -> ApiResult<AdminActionView> {
    let operator = operator(&state, &caller, OperatorCapability::DisputeResponse).await?;
    let reason = reason_of(&body.reason)?;
    let operation = ids::operation(&body.operation)?;
    let outcome = state
        .ports
        .finance()
        .dispute_evidence(act_of(&operator, &reason, operation), &dispute_id)
        .await?;
    audited(
        &state,
        &operator,
        AuditAction::DisputeEvidence,
        dispute_id,
        reason,
        operation,
        outcome,
    )
    .await
}

pub(super) async fn reconciliation<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<ReconciliationView> {
    operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    reply(state.ports.finance().reconciliation().await?)
}

pub(super) async fn providers<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<Vec<ProviderRow>> {
    operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    reply(state.ports.finance().providers().await?)
}

async fn switch_provider<S: AccountStore, M: EmailSender, P: Ports>(
    state: AppApiState<S, M, P>,
    caller: Authenticated,
    provider_id: String,
    body: ReasonRequest,
    switch: ProviderSwitch,
) -> ApiResult<AdminActionView> {
    let operator = operator(&state, &caller, OperatorCapability::ProviderControl).await?;
    let reason = reason_of(&body.reason)?;
    let operation = ids::operation(&body.operation)?;
    let outcome = state
        .ports
        .finance()
        .switch_provider(act_of(&operator, &reason, operation), &provider_id, switch)
        .await?;
    let action = match switch {
        ProviderSwitch::Pause => AuditAction::ProviderPaused,
        ProviderSwitch::Resume => AuditAction::ProviderResumed,
    };
    audited(
        &state,
        &operator,
        action,
        provider_id,
        reason,
        operation,
        outcome,
    )
    .await
}

pub(super) async fn pause_provider<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(provider_id): Path<String>,
    ApiBody(body): ApiBody<ReasonRequest>,
) -> ApiResult<AdminActionView> {
    switch_provider(state, caller, provider_id, body, ProviderSwitch::Pause).await
}

pub(super) async fn resume_provider<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(provider_id): Path<String>,
    ApiBody(body): ApiBody<ReasonRequest>,
) -> ApiResult<AdminActionView> {
    switch_provider(state, caller, provider_id, body, ProviderSwitch::Resume).await
}

pub(super) async fn review_queue<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<Vec<ReviewItem>> {
    operator(&state, &caller, OperatorCapability::ContentReview).await?;
    reply(state.ports.review().pending().await?)
}

pub(super) async fn decide_review<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(item_id): Path<String>,
    ApiBody(body): ApiBody<ReviewDecisionRequest>,
) -> ApiResult<AdminActionView> {
    let operator = operator(&state, &caller, OperatorCapability::ContentReview).await?;
    let reason = reason_of(&body.reason)?;
    let operation = ids::operation(&body.operation)?;
    let outcome = state
        .ports
        .review()
        .decide(
            act_of(&operator, &reason, operation),
            &item_id,
            body.decision,
        )
        .await?;
    let action = match body.decision {
        ReviewDecision::Approve => AuditAction::ReviewApproved,
        ReviewDecision::Reject => AuditAction::ReviewRejected,
    };
    audited(
        &state, &operator, action, item_id, reason, operation, outcome,
    )
    .await
}

pub(super) async fn audit<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<Vec<AuditRow>> {
    operator(&state, &caller, OperatorCapability::SupportInspect).await?;
    reply(state.ports.operators().audit().await?)
}
