//! The signed-in customer's endpoints: balances, credit, funding, statements and settings.
//! Every reply is a customer-safe wire type; a foreign id is `not_found`, the same as a missing
//! one. Money moves only through the finance and checkout ports, under the caller's operation id.

use axum::extract::{Path, Query, State, rejection::QueryRejection};
use df_account_app::wire::{
    Accepted, AccountStats, AllocateRequest, ApiError, AssetKind, AssetsView, ChangeEmailRequest,
    ChangePasswordRequest, CheckoutCreated, CheckoutView, CloseAccountRequest, ClosedView,
    ClosureBlocker, ClosureState, CreateCheckoutRequest, CreditOffers, DeviceKind, DeviceSession,
    EmailPreferencesRequest, ErrorCode, ExportRequest, ExportView, FundingView, OverviewView,
    PacePreset, PlaySessionsView, ProfileRequest, Refusal, RevokeSessionRequest, RevokedView,
    SetPaceRequest, SettingsView, SignInMethod, StatementFilter, StatementsView,
};
use df_customer::{
    AccountStatus, CloseResult, EmailChangeResult, ExportResult, PasswordChangeResult,
};
use df_customer::{
    AccountStore, DeviceLabel, DisplayNameRefusal, EmailSender, ProfileResult, Reauthentication,
    RevokeScope,
};
use serde::Deserialize;

use super::{
    guard::{
        ApiBody, ApiFailure, ApiResult, Nothing, Reply, SameSite, SessionPost, SignedIn,
        clear_secret_cookie, reply, secret_cookie,
    },
    ids,
    ports::{
        AccountExtrasPort, AssetsPort, CampaignDirectory, CheckoutPort, FinancePort, FundingTarget,
        Ports,
    },
    state::AppApiState,
};
use crate::security::{ClientInfo, SESSION_COOKIE};

const ACCEPTED: Accepted = Accepted { accepted: true };

/// The customer's own checkout and funding actions need a verified address (plan 5).
fn require_verified(customer: &df_customer::Authenticated) -> Result<(), ApiFailure> {
    if customer.account.is_verified {
        Ok(())
    } else {
        Err(ApiFailure::of(ErrorCode::NotVerified))
    }
}

fn refused(refusal: Refusal) -> ApiFailure {
    ApiFailure::new(ApiError::refused(refusal))
}

pub(super) async fn overview<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<OverviewView> {
    let view = state
        .ports
        .finance()
        .overview(caller.account.id, caller.account.is_verified)
        .await?;
    reply(view)
}

pub(super) async fn credit_offers<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(_caller): SignedIn,
) -> ApiResult<CreditOffers> {
    reply(state.ports.checkout().offers())
}

pub(super) async fn create_checkout<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<CreateCheckoutRequest>,
) -> ApiResult<CheckoutCreated> {
    require_verified(&caller)?;
    let offers = state.ports.checkout().offers();
    if !(offers.custom_minimum_cents..=offers.custom_maximum_cents).contains(&body.amount_cents) {
        return Err(refused(Refusal::AmountOutOfRange));
    }
    let operation = ids::operation(&body.operation)?;
    let created = state
        .ports
        .checkout()
        .create(caller.account.id, body.amount_cents, operation)
        .await?;
    reply(created)
}

pub(super) async fn checkout_status<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    Path(checkout_id): Path<String>,
) -> ApiResult<CheckoutView> {
    let checkout = ids::checkout(&checkout_id)?;
    reply(
        state
            .ports
            .checkout()
            .status(caller.account.id, checkout)
            .await?,
    )
}

async fn funding_view<S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    account: df_types::AccountId,
    targets: &[FundingTarget],
) -> Result<FundingView, ApiFailure> {
    Ok(state.ports.finance().funding(account, targets).await?)
}

pub(super) async fn funding<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<FundingView> {
    let targets = state
        .ports
        .campaigns()
        .funding_targets(caller.account.id)
        .await?;
    reply(funding_view(&state, caller.account.id, &targets).await?)
}

pub(super) async fn set_pace<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<SetPaceRequest>,
) -> ApiResult<FundingView> {
    let campaign = ids::campaign(&body.campaign_id)?;
    let operation = ids::operation(&body.operation)?;
    let targets = state
        .ports
        .campaigns()
        .funding_targets(caller.account.id)
        .await?;
    let target = targets
        .iter()
        .find(|target| target.campaign == campaign)
        .ok_or_else(|| ApiFailure::of(ErrorCode::NotFound))?;
    if !target.can_choose_pace {
        return Err(ApiFailure::of(ErrorCode::Forbidden));
    }
    if (body.preset == PacePreset::Custom) != body.custom_ceiling_cents_per_hour.is_some() {
        return Err(refused(Refusal::CeilingOutOfRange));
    }
    state
        .ports
        .finance()
        .set_pace(
            caller.account.id,
            campaign,
            body.preset,
            body.custom_ceiling_cents_per_hour,
            operation,
        )
        .await?;
    reply(funding_view(&state, caller.account.id, &targets).await?)
}

pub(super) async fn allocate<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<AllocateRequest>,
) -> ApiResult<FundingView> {
    require_verified(&caller)?;
    let session = ids::session(&body.session_id)?;
    let operation = ids::operation(&body.operation)?;
    let targets = state
        .ports
        .campaigns()
        .funding_targets(caller.account.id)
        .await?;
    if !targets
        .iter()
        .any(|target| target.live_session == Some(session))
    {
        return Err(ApiFailure::of(ErrorCode::NotFound));
    }
    if !body.consents_to_host_spending {
        return Err(refused(Refusal::ConsentRequired));
    }
    if body.amount_cents == 0 {
        return Err(refused(Refusal::AmountOutOfRange));
    }
    state
        .ports
        .finance()
        .allocate(caller.account.id, session, body.amount_cents, operation)
        .await?;
    reply(funding_view(&state, caller.account.id, &targets).await?)
}

#[derive(Deserialize)]
pub(super) struct StatementsQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    from_ms: Option<u64>,
    #[serde(default)]
    to_ms: Option<u64>,
}

pub(super) async fn statements<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    query: Result<Query<StatementsQuery>, QueryRejection>,
) -> ApiResult<StatementsView> {
    let Query(query) = query.map_err(|_| ApiFailure::of(ErrorCode::Invalid))?;
    let filter = match query.filter.as_deref() {
        None | Some("all") => StatementFilter::All,
        Some("top_ups") => StatementFilter::TopUps,
        Some("spend") => StatementFilter::Spend,
        Some(_) => return Err(ApiFailure::of(ErrorCode::Invalid)),
    };
    reply(
        state
            .ports
            .finance()
            .statements_filtered(
                caller.account.id,
                filter,
                query.from_ms,
                query.to_ms,
                query.cursor.as_deref(),
            )
            .await?,
    )
}

#[derive(Deserialize)]
pub(super) struct PageQuery {
    #[serde(default)]
    cursor: Option<String>,
}

pub(super) async fn play_sessions<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> ApiResult<PlaySessionsView> {
    let Query(query) = query.map_err(|_| ApiFailure::of(ErrorCode::Invalid))?;
    reply(
        state
            .ports
            .campaigns()
            .play_sessions(caller.account.id, query.cursor.as_deref())
            .await?,
    )
}

#[derive(Deserialize)]
pub(super) struct AssetsQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    campaign: Option<String>,
}

pub(super) async fn assets<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    query: Result<Query<AssetsQuery>, QueryRejection>,
) -> ApiResult<AssetsView> {
    let Query(query) = query.map_err(|_| ApiFailure::of(ErrorCode::Invalid))?;
    let kind = match query.kind.as_deref() {
        None | Some("") => None,
        Some("portrait") => Some(AssetKind::Portrait),
        Some("scene") => Some(AssetKind::Scene),
        Some("music") => Some(AssetKind::Music),
        Some("video") => Some(AssetKind::Video),
        Some(_) => return Err(ApiFailure::of(ErrorCode::Invalid)),
    };
    reply(
        state
            .ports
            .assets()
            .list(
                caller.account.id,
                kind,
                query.campaign.as_deref().filter(|text| !text.is_empty()),
                query.cursor.as_deref(),
            )
            .await?,
    )
}

/// Dashboard stats composed server-side from finance, campaigns, play history and assets.
pub(super) async fn account_stats<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<AccountStats> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64);
    let mut stats = state
        .ports
        .finance()
        .account_stats(caller.account.id, now_ms)
        .await?;
    let actor = super::ports::Actor {
        account: caller.account.id,
        display_name: caller
            .account
            .display_name
            .as_ref()
            .map_or_else(String::new, |name| name.as_str().to_owned()),
        is_verified: caller.account.is_verified,
    };
    if let Ok(dashboard) = state.ports.campaigns().dashboard(&actor).await {
        stats.campaigns_active = dashboard
            .campaigns
            .iter()
            .filter(|card| !card.is_archived)
            .count() as u64;
        stats.campaigns_archived = dashboard
            .campaigns
            .iter()
            .filter(|card| card.is_archived)
            .count() as u64;
        stats.characters = dashboard.campaigns.len() as u64;
        stats.players_hosted = dashboard
            .campaigns
            .iter()
            .map(|card| u64::from(card.party_size))
            .sum();
    }
    // Page through play history with the bounded page size the port already enforces.
    let mut cursor: Option<String> = None;
    let mut sessions = Vec::new();
    for _ in 0..50 {
        let page = state
            .ports
            .campaigns()
            .play_sessions(caller.account.id, cursor.as_deref())
            .await?;
        sessions.extend(page.sessions.iter().cloned());
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    stats.play_sessions = sessions.len() as u64;
    stats.played_seconds_total = sessions.iter().map(|row| row.duration_seconds).sum();
    let month_start_ms = sessions
        .iter()
        .map(|row| row.started_at_ms)
        .max()
        .unwrap_or(now_ms)
        .saturating_sub(31 * 24 * 60 * 60 * 1000);
    stats.played_seconds_this_month = sessions
        .iter()
        .filter(|row| row.started_at_ms >= month_start_ms)
        .map(|row| row.duration_seconds)
        .sum();
    stats.average_spend_per_table_hour_micro_dollars = stats
        .spent_lifetime_micro_dollars
        .saturating_mul(3600)
        .checked_div(stats.played_seconds_total)
        .unwrap_or(0);
    if let Ok(assets) = state
        .ports
        .assets()
        .list(caller.account.id, None, None, None)
        .await
    {
        stats.assets_generated = assets.total;
    }
    reply(stats)
}

const fn device_kind(label: DeviceLabel) -> DeviceKind {
    match label {
        DeviceLabel::Phone => DeviceKind::Phone,
        DeviceLabel::Tablet => DeviceKind::Tablet,
        DeviceLabel::Desktop => DeviceKind::Desktop,
        DeviceLabel::Unknown => DeviceKind::Unknown,
    }
}

/// What stops the account from closing: the money side's blockers, then hosted campaigns.
async fn closure_blockers<S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    account: df_types::AccountId,
) -> Result<Vec<ClosureBlocker>, ApiFailure> {
    let mut blockers: Vec<ClosureBlocker> = state
        .ports
        .campaigns()
        .closure_blockers(account)
        .await?
        .into_iter()
        .map(|campaign_name| ClosureBlocker::HostsActiveCampaign { campaign_name })
        .collect();
    blockers.extend(state.ports.finance().closure_blockers(account).await?);
    Ok(blockers)
}

pub(super) async fn settings<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<SettingsView> {
    let security = state.service.security_overview(&caller).await?;
    let preferences = state.ports.extras().preferences(caller.account.id).await?;
    let (export_status, _) = state
        .ports
        .extras()
        .export_status(caller.account.id)
        .await?;
    let mut sign_in_methods = Vec::new();
    if state.service.has_password_method(&caller).await? {
        sign_in_methods.push(SignInMethod::Password);
    }
    if state.service.google_is_linked(&caller).await? {
        sign_in_methods.push(SignInMethod::Google);
    }
    reply(SettingsView {
        email: caller
            .account
            .email
            .as_ref()
            .map_or_else(String::new, |email| email.display().to_owned()),
        display_name: caller
            .account
            .display_name
            .as_ref()
            .map(|name| name.as_str().to_owned()),
        sessions: security
            .sessions
            .iter()
            .map(|session| DeviceSession {
                session_id: ids::hex(session.id.as_bytes()),
                device: device_kind(session.device),
                signed_in_at_ms: session.created_at_ms,
                last_seen_at_ms: session.last_seen_at_ms,
                is_current: session.id == caller.session,
            })
            .collect(),
        closure_blockers: closure_blockers(&state, caller.account.id).await?,
        created_at_ms: caller.account.created_at_ms,
        is_verified: caller.account.is_verified,
        // Consent facts are stored at sign-up (B round 2c) but are not yet on the account
        // record the service returns; see P/request.md.
        terms_version: None,
        terms_accepted_at_ms: None,
        adult_attested_at_ms: None,
        closure_state: match caller.account.status {
            AccountStatus::Active => ClosureState::Open,
            AccountStatus::Closing | AccountStatus::Closed => ClosureState::Closing,
        },
        sign_in_methods,
        email_preferences: preferences,
        export_status,
    })
}

fn audit_customer(action: &str, account: df_types::AccountId) {
    df_observe::record_audit_line(
        "app_api.account",
        format!("ACCOUNT {action} account={}", ids::account_text(account)),
    );
}

pub(super) async fn change_email<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<ChangeEmailRequest>,
) -> ApiResult<Accepted> {
    let operation = ids::operation(&body.operation)?;
    match state
        .service
        .request_email_change(&caller, &body.password, &body.new_email, operation)
        .await?
    {
        EmailChangeResult::ConfirmationSent => {
            audit_customer("email_change_requested", caller.account.id);
            reply(ACCEPTED)
        }
        EmailChangeResult::WrongPassword => Err(refused(Refusal::WrongPassword)),
        EmailChangeResult::Refused(refusal) => Err(refused(match refusal {
            df_customer::EmailRefusal::Empty => Refusal::EmailEmpty,
            df_customer::EmailRefusal::TooLong => Refusal::EmailTooLong,
            df_customer::EmailRefusal::Malformed => Refusal::EmailMalformed,
        })),
        EmailChangeResult::SameAddress => Err(refused(Refusal::EmailMalformed)),
        EmailChangeResult::StaleForm => Err(ApiFailure::of(ErrorCode::StaleForm)),
        EmailChangeResult::ProviderManaged => Err(ApiFailure::of(ErrorCode::Unavailable)),
        EmailChangeResult::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
    }
}

pub(super) async fn change_password<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ClientInfo(client): ClientInfo,
    ApiBody(body): ApiBody<ChangePasswordRequest>,
) -> ApiResult<df_account_app::wire::SessionView> {
    match state
        .service
        .change_password(
            &caller,
            &body.current_password,
            &body.password,
            &body.confirmation,
            client,
        )
        .await?
    {
        PasswordChangeResult::Changed(issued) => {
            audit_customer("password_changed", caller.account.id);
            let token = issued.token.text();
            let Some(fresh) = state.service.authenticate(&token).await? else {
                return Err(ApiFailure::of(ErrorCode::Unavailable));
            };
            let view = super::session::signed_in_view(&state, &fresh).await?;
            Ok(Reply::new(view).with_cookie(secret_cookie(
                SESSION_COOKIE,
                &token,
                issued.max_age_s,
                SameSite::Lax,
            )))
        }
        PasswordChangeResult::WrongPassword => Err(refused(Refusal::WrongPassword)),
        PasswordChangeResult::Mismatch => Err(refused(Refusal::PasswordMismatch)),
        PasswordChangeResult::Refused(refusal) => Err(refused(match refusal {
            df_customer::PasswordRefusal::TooShort => Refusal::PasswordTooShort,
            df_customer::PasswordRefusal::TooLong => Refusal::PasswordTooLong,
            df_customer::PasswordRefusal::MatchesEmail => Refusal::PasswordMatchesEmail,
            df_customer::PasswordRefusal::Common => Refusal::PasswordCommon,
        })),
        PasswordChangeResult::ProviderManaged => Err(ApiFailure::of(ErrorCode::Unavailable)),
        PasswordChangeResult::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
    }
}

pub(super) async fn save_preferences<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<EmailPreferencesRequest>,
) -> ApiResult<Accepted> {
    let operation = ids::operation(&body.operation)?;
    state
        .ports
        .extras()
        .save_preferences(
            caller.account.id,
            df_account_app::wire::EmailPreferences {
                product_news: body.product_news,
                session_reminders: body.session_reminders,
                transactional_always_on: true,
            },
            operation,
        )
        .await?;
    audit_customer("email_preferences_changed", caller.account.id);
    reply(ACCEPTED)
}

pub(super) async fn request_export<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ClientInfo(client): ClientInfo,
    ApiBody(body): ApiBody<ExportRequest>,
) -> ApiResult<ExportView> {
    let operation = ids::operation(&body.operation)?;
    let proof = sensitive_proof(&state, &caller, &body.password).await?;
    match state.service.export(&caller, proof, client).await? {
        ExportResult::Document(_) => {}
        ExportResult::NotReauthenticated => return Err(refused(Refusal::WrongPassword)),
        ExportResult::Throttled => return Err(ApiFailure::of(ErrorCode::Throttled)),
    }
    let (status, requested_at_ms) = state
        .ports
        .extras()
        .request_export(caller.account.id, operation)
        .await?;
    audit_customer("export_requested", caller.account.id);
    reply(ExportView {
        status,
        requested_at_ms,
    })
}

pub(super) async fn revoke_session<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ClientInfo(client): ClientInfo,
    ApiBody(body): ApiBody<RevokeSessionRequest>,
) -> ApiResult<RevokedView> {
    let target = df_types::WebSessionId::from_hex(&body.session_id)
        .map_err(|_| ApiFailure::of(ErrorCode::NotFound))?;
    if target == caller.session {
        // Signing out this device ends the current session.
        state.service.logout(&caller, client).await?;
        audit_customer("session_revoked", caller.account.id);
        return reply(RevokedView { revoked: 1 });
    }
    let revoked = state
        .service
        .revoke_sessions(&caller, RevokeScope::One(target), client)
        .await?;
    if revoked == 0 {
        return Err(ApiFailure::of(ErrorCode::NotFound));
    }
    audit_customer("session_revoked", caller.account.id);
    reply(RevokedView { revoked })
}

pub(super) async fn revoke_all_sessions<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ClientInfo(client): ClientInfo,
    ApiBody(_nothing): ApiBody<Nothing>,
) -> ApiResult<RevokedView> {
    let revoked = state
        .service
        .revoke_sessions(&caller, RevokeScope::All, client)
        .await?;
    audit_customer("sessions_revoked_all", caller.account.id);
    reply(RevokedView { revoked })
}

pub(super) async fn save_profile<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<ProfileRequest>,
) -> ApiResult<Accepted> {
    ids::operation(&body.operation)?;
    match state
        .service
        .update_profile(&caller, &body.display_name)
        .await?
    {
        ProfileResult::Saved => {
            audit_customer("display_name_changed", caller.account.id);
            reply(ACCEPTED)
        }
        ProfileResult::Refused(DisplayNameRefusal::TooLong) => {
            Err(refused(Refusal::DisplayNameTooLong))
        }
        ProfileResult::Refused(DisplayNameRefusal::ControlCharacter) => {
            Err(refused(Refusal::DisplayNameControlCharacter))
        }
        ProfileResult::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
    }
}

pub(super) async fn revoke_other_sessions<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ClientInfo(client): ClientInfo,
    ApiBody(_nothing): ApiBody<Nothing>,
) -> ApiResult<RevokedView> {
    let revoked = state
        .service
        .revoke_sessions(&caller, RevokeScope::AllExcept(caller.session), client)
        .await?;
    reply(RevokedView { revoked })
}

/// Closes the account after the password is re-entered. Blockers are checked first, so a
/// refused closure changes nothing.
pub(super) async fn close_account<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<CloseAccountRequest>,
) -> ApiResult<ClosedView> {
    ids::operation(&body.operation)?;
    if !closure_blockers(&state, caller.account.id)
        .await?
        .is_empty()
    {
        return Err(ApiFailure::of(ErrorCode::Invalid));
    }
    let proof = sensitive_proof(&state, &caller, &body.password).await?;
    match state.service.close(&caller, proof).await? {
        CloseResult::Closed { delete_after_ms } => Ok(Reply::new(ClosedView { delete_after_ms })
            .with_cookie(clear_secret_cookie(SESSION_COOKIE, SameSite::Lax))),
        CloseResult::Blocked(_) => Err(ApiFailure::of(ErrorCode::Invalid)),
        CloseResult::NotReauthenticated => Err(refused(Refusal::WrongPassword)),
        CloseResult::Throttled => Err(ApiFailure::of(ErrorCode::Throttled)),
    }
}

async fn sensitive_proof<'a, S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    caller: &df_customer::Authenticated,
    password: &'a str,
) -> Result<Reauthentication<'a>, ApiFailure> {
    if password.is_empty() && state.service.google_is_linked(caller).await? {
        if !state.service.has_fresh_google_proof(caller).await? {
            return Err(refused(Refusal::ReauthenticationRequired));
        }
        return Ok(Reauthentication::RecentSignIn);
    }
    Ok(Reauthentication::Password(password))
}
