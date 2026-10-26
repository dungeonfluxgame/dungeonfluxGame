//! S1's campaign endpoints, as the account app uses them. S1 owns every rule; these handlers only
//! apply the request guard, translate path and body into typed ids and call `CampaignDirectory`.
//! A foreign or malformed id is `not_found`. The invitation token travels in the body (it came
//! from a URL fragment); the only place it is kept is the `__Host-df_join` cookie.

use axum::{
    extract::{Path, State},
    http::HeaderMap,
};
use df_account_app::{
    wire::{Accepted, ErrorCode},
    wire_campaign::{
        ApproveRequestBody, CampaignCreated, CampaignOverview, CreateInvitationRequest,
        DashboardView, DraftView, InvitationCreated, InvitePreview, JoinAcceptRequest,
        JoinAccepted, JoinRequested, LobbyOpened, LobbyView, OperationRequest, PendingInvitation,
        RoomCodeJoinRequest, RoomCodeLookupRequest, SaveDraftRequest, TokenBody, WizardCatalog,
    },
};
use df_customer::{AccountStore, Authenticated, EmailSender};

use super::{
    guard::{
        ApiBody, ApiFailure, ApiResult, JOIN_COOKIE, Nothing, Reply, SameSite, SessionPost,
        SignedIn, VisitorPost, clear_secret_cookie, reply, secret_cookie,
    },
    ids,
    ports::{Actor, CampaignDirectory, FinancePort, Ports, RequestDecision},
    state::AppApiState,
};
use crate::security::cookie;

/// The invitation a person was holding stays for an hour (S1 8.2).
const JOIN_COOKIE_MAX_AGE_S: u64 = 60 * 60;
const ACCEPTED: Accepted = Accepted { accepted: true };
/// The create wizard has five steps (S1 8.1).
const WIZARD_STEPS: std::ops::RangeInclusive<u8> = 1..=5;

fn actor(caller: &Authenticated) -> Actor {
    let display_name = caller
        .account
        .display_name
        .as_ref()
        .map(|name| name.as_str().to_owned())
        .or_else(|| {
            caller
                .account
                .email
                .as_ref()
                .and_then(|email| email.display().split('@').next().map(str::to_owned))
        })
        .unwrap_or_else(|| "Player".to_owned());
    Actor {
        account: caller.account.id,
        display_name,
        is_verified: caller.account.is_verified,
    }
}

/// Creating a campaign needs a verified email (S1 decision 12); joining does not.
fn verified_actor(caller: &Authenticated) -> Result<Actor, ApiFailure> {
    if caller.account.is_verified {
        Ok(actor(caller))
    } else {
        Err(ApiFailure::of(ErrorCode::NotVerified))
    }
}

pub(super) async fn dashboard<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
) -> ApiResult<DashboardView> {
    reply(state.ports.campaigns().dashboard(&actor(&caller)).await?)
}

pub(super) async fn wizard_catalog<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(_caller): SignedIn,
) -> ApiResult<WizardCatalog> {
    reply(state.ports.campaigns().wizard_catalog().await?)
}

pub(super) async fn start_draft<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(_nothing): ApiBody<Nothing>,
) -> ApiResult<DraftView> {
    let who = verified_actor(&caller)?;
    let (draft_id, draft) = state.ports.campaigns().start_draft(&who).await?;
    reply(DraftView {
        draft_id,
        step: *WIZARD_STEPS.start(),
        draft,
    })
}

pub(super) async fn save_draft<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path((draft_id, step)): Path<(String, u8)>,
    ApiBody(body): ApiBody<SaveDraftRequest>,
) -> ApiResult<DraftView> {
    let who = verified_actor(&caller)?;
    if !WIZARD_STEPS.contains(&step) {
        return Err(ApiFailure::of(ErrorCode::NotFound));
    }
    let draft = state
        .ports
        .campaigns()
        .save_draft(&who, &draft_id, step, body.draft)
        .await?;
    reply(DraftView {
        draft_id,
        step,
        draft,
    })
}

pub(super) async fn finish_draft<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(draft_id): Path<String>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<CampaignCreated> {
    let who = verified_actor(&caller)?;
    let operation = ids::operation(&body.operation)?;
    let campaign = state
        .ports
        .campaigns()
        .finish_draft(&who, &draft_id, operation)
        .await?;
    reply(CampaignCreated {
        campaign_id: ids::hex(campaign.as_bytes()),
    })
}

pub(super) async fn campaign<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    Path(campaign_id): Path<String>,
) -> ApiResult<CampaignOverview> {
    let campaign = ids::campaign(&campaign_id)?;
    reply(
        state
            .ports
            .campaigns()
            .campaign(&actor(&caller), campaign)
            .await?,
    )
}

pub(super) async fn create_invitation<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(campaign_id): Path<String>,
    ApiBody(body): ApiBody<CreateInvitationRequest>,
) -> ApiResult<InvitationCreated> {
    let campaign = ids::campaign(&campaign_id)?;
    let operation = ids::operation(&body.operation)?;
    reply(
        state
            .ports
            .campaigns()
            .create_invitation(&actor(&caller), campaign, body.kind, operation)
            .await?,
    )
}

pub(super) async fn revoke_invitation<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path((campaign_id, invitation_id)): Path<(String, String)>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<CampaignOverview> {
    let campaign = ids::campaign(&campaign_id)?;
    ids::operation(&body.operation)?;
    reply(
        state
            .ports
            .campaigns()
            .revoke_invitation(&actor(&caller), campaign, &invitation_id)
            .await?,
    )
}

pub(super) async fn rotate_code<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(campaign_id): Path<String>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<CampaignOverview> {
    let campaign = ids::campaign(&campaign_id)?;
    ids::operation(&body.operation)?;
    reply(
        state
            .ports
            .campaigns()
            .rotate_code(&actor(&caller), campaign)
            .await?,
    )
}

pub(super) async fn approve_request<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path((campaign_id, request_id)): Path<(String, String)>,
    ApiBody(body): ApiBody<ApproveRequestBody>,
) -> ApiResult<CampaignOverview> {
    let campaign = ids::campaign(&campaign_id)?;
    ids::operation(&body.operation)?;
    reply(
        state
            .ports
            .campaigns()
            .decide_join_request(
                &actor(&caller),
                campaign,
                &request_id,
                RequestDecision::Approve(body.role),
            )
            .await?,
    )
}

pub(super) async fn deny_request<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path((campaign_id, request_id)): Path<(String, String)>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<CampaignOverview> {
    let campaign = ids::campaign(&campaign_id)?;
    ids::operation(&body.operation)?;
    reply(
        state
            .ports
            .campaigns()
            .decide_join_request(
                &actor(&caller),
                campaign,
                &request_id,
                RequestDecision::Deny,
            )
            .await?,
    )
}

pub(super) async fn leave<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(campaign_id): Path<String>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<Accepted> {
    let campaign = ids::campaign(&campaign_id)?;
    ids::operation(&body.operation)?;
    state
        .ports
        .campaigns()
        .leave(&actor(&caller), campaign)
        .await?;
    reply(ACCEPTED)
}

pub(super) async fn open_lobby<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(campaign_id): Path<String>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<LobbyOpened> {
    let campaign = ids::campaign(&campaign_id)?;
    ids::operation(&body.operation)?;
    let session = state
        .ports
        .campaigns()
        .open_lobby(&actor(&caller), campaign)
        .await?;
    reply(LobbyOpened {
        session_id: ids::hex(session.as_bytes()),
    })
}

pub(super) async fn lobby<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    Path(session_id): Path<String>,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    let view = state
        .ports
        .campaigns()
        .lobby(&actor(&caller), session)
        .await?;
    lobby_reply(&state, &caller, session, view).await
}

async fn lobby_reply<S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    caller: &Authenticated,
    session: df_types::SessionId,
    mut view: LobbyView,
) -> ApiResult<LobbyView> {
    if !view.has_started
        && state.ports.finance().host_credit_readiness(session).await? == Some(false)
    {
        if !view
            .start_blockers
            .contains(&df_account_app::wire_campaign::StartBlocker::FundingNotReady)
        {
            view.start_blockers
                .push(df_account_app::wire_campaign::StartBlocker::FundingNotReady);
        }
        view.funding_note = df_account_app::wire_campaign::FundingNote::WaitingForSponsor;
    }
    if !view.has_started
        && let Some(quote) = state
            .ports
            .finance()
            .host_credit(caller.account.id, session)
            .await?
    {
        view.funding_note = df_account_app::wire_campaign::FundingNote::HostCredit(quote);
        if quote.is_fundable {
            view.start_blockers.retain(|blocker| {
                *blocker != df_account_app::wire_campaign::StartBlocker::FundingNotReady
            });
        }
        if !quote.is_fundable
            && !view
                .start_blockers
                .contains(&df_account_app::wire_campaign::StartBlocker::FundingNotReady)
        {
            view.start_blockers
                .push(df_account_app::wire_campaign::StartBlocker::FundingNotReady);
        }
    }
    reply(view)
}

pub(super) async fn cancel_host_lobby<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<Accepted> {
    let session = ids::session(&session_id)?;
    ids::operation(&body.operation)?;
    state
        .ports
        .finance()
        .cancel_host_lobby(caller.account.id, session)
        .await?;
    state
        .ports
        .campaigns()
        .cancel_lobby(&actor(&caller), session)
        .await?;
    reply(ACCEPTED)
}

pub(super) async fn consent_host_credit<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<df_account_app::wire_campaign::HostConsentRequest>,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    let operation = ids::operation(&body.operation)?;
    state
        .ports
        .campaigns()
        .lobby(&actor(&caller), session)
        .await?;
    state
        .ports
        .finance()
        .consent_host_credit(caller.account.id, session, body, operation)
        .await?;
    df_observe::record_audit_line(
        "portal.host_credit_consented",
        format!("SPEND host_credit_consented session={session_id}"),
    );
    let view = state
        .ports
        .campaigns()
        .lobby(&actor(&caller), session)
        .await?;
    lobby_reply(&state, &caller, session, view).await
}

pub(super) async fn start_session<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    ids::operation(&body.operation)?;
    let before = state
        .ports
        .campaigns()
        .lobby(&actor(&caller), session)
        .await?;
    if !before.can_manage
        || state.ports.finance().host_credit_readiness(session).await? == Some(false)
    {
        return Err(ApiFailure::of(ErrorCode::Forbidden));
    }
    let view = state
        .ports
        .campaigns()
        .start_session(&actor(&caller), session)
        .await?;
    lobby_reply(&state, &caller, session, view).await
}

pub(super) async fn join_preview<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    _guard: VisitorPost,
    ApiBody(body): ApiBody<TokenBody>,
) -> ApiResult<InvitePreview> {
    reply(state.ports.campaigns().join_preview(&body.token).await?)
}

/// Remembers a usable invitation across sign in, sign up and verification without a token in
/// any URL or script-reachable storage.
pub(super) async fn join_hold<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    _guard: VisitorPost,
    ApiBody(body): ApiBody<TokenBody>,
) -> ApiResult<Accepted> {
    state.ports.campaigns().join_preview(&body.token).await?;
    if !body.token.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(ApiFailure::of(ErrorCode::Invalid));
    }
    Ok(Reply::new(ACCEPTED).with_cookie(secret_cookie(
        JOIN_COOKIE,
        &body.token,
        JOIN_COOKIE_MAX_AGE_S,
        SameSite::Lax,
    )))
}

/// The remembered invitation's preview, if it is still usable; an unusable one is forgotten.
pub(super) async fn join_pending<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    headers: HeaderMap,
) -> ApiResult<PendingInvitation> {
    let Some(token) = cookie(&headers, JOIN_COOKIE).filter(|token| !token.is_empty()) else {
        return reply(PendingInvitation { preview: None });
    };
    match state.ports.campaigns().join_preview(token).await {
        Ok(preview) => reply(PendingInvitation {
            preview: Some(preview),
        }),
        Err(_) => Ok(Reply::new(PendingInvitation { preview: None })
            .with_cookie(clear_secret_cookie(JOIN_COOKIE, SameSite::Lax))),
    }
}

pub(super) async fn join_accept<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    headers: HeaderMap,
    ApiBody(body): ApiBody<JoinAcceptRequest>,
) -> ApiResult<JoinAccepted> {
    let operation = ids::operation(&body.operation)?;
    let token = body
        .token
        .or_else(|| cookie(&headers, JOIN_COOKIE).map(str::to_owned))
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiFailure::of(ErrorCode::NotFound))?;
    let campaign = state
        .ports
        .campaigns()
        .join_accept(&actor(&caller), &token, operation)
        .await?;
    Ok(Reply::new(JoinAccepted {
        campaign_id: ids::hex(campaign.as_bytes()),
    })
    .with_cookie(clear_secret_cookie(JOIN_COOKIE, SameSite::Lax)))
}

pub(super) async fn code_lookup<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<RoomCodeLookupRequest>,
) -> ApiResult<InvitePreview> {
    reply(
        state
            .ports
            .campaigns()
            .code_lookup(&actor(&caller), &body.code)
            .await?,
    )
}

pub(super) async fn code_request<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<RoomCodeJoinRequest>,
) -> ApiResult<JoinRequested> {
    let operation = ids::operation(&body.operation)?;
    reply(
        state
            .ports
            .campaigns()
            .code_request(&actor(&caller), &body.code, operation)
            .await?,
    )
}

pub(super) async fn play_now<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<df_account_app::wire_campaign::PlayNowView> {
    let who = verified_actor(&caller)?;
    let operation = ids::operation(&body.operation)?;
    let campaign = state
        .ports
        .campaigns()
        .play_now_campaign(&who, operation)
        .await?;
    let session = state
        .ports
        .campaigns()
        .open_lobby(&who, campaign)
        .await
        .map_err(|error| launch_after_creation_failed(campaign, error))?;
    state
        .ports
        .finance()
        .prepare_host_session(who.account, session)
        .await
        .map_err(|error| launch_after_creation_failed(campaign, error))?;
    let view = state
        .ports
        .campaigns()
        .lobby(&who, session)
        .await
        .map_err(|error| launch_after_creation_failed(campaign, error))?;
    let mut view = view;
    if let Some(mut quote) = state
        .ports
        .finance()
        .host_credit(who.account, session)
        .await
        .map_err(|error| launch_after_creation_failed(campaign, error))?
    {
        // Pressing Play now is the host's approval to use their credit at this quote (owner
        // decision 2026-10-26); the dashboard says so beside the button. Only when the credit covers
        // the quote's cap; otherwise the lobby asks for funding as before. A retry finds it recorded.
        if quote.consented_at_ms.is_none()
            && quote.available_micro_dollars >= quote.cap_micro_dollars
        {
            state
                .ports
                .finance()
                .consent_host_credit(
                    who.account,
                    session,
                    df_account_app::wire_campaign::HostConsentRequest {
                        price_version: quote.price_version,
                        cap_micro_dollars: quote.cap_micro_dollars,
                        terms_version: quote.terms_version,
                        operation: body.operation.clone(),
                    },
                    operation,
                )
                .await
                .map_err(|error| launch_after_creation_failed(campaign, error))?;
            df_observe::record_audit_line(
                "portal.host_credit_consented",
                format!(
                    "SPEND host_credit_consented session={} via=play_now",
                    ids::hex(session.as_bytes())
                ),
            );
            view = state
                .ports
                .campaigns()
                .lobby(&who, session)
                .await
                .map_err(|error| launch_after_creation_failed(campaign, error))?;
            if let Some(refreshed) = state
                .ports
                .finance()
                .host_credit(who.account, session)
                .await
                .map_err(|error| launch_after_creation_failed(campaign, error))?
            {
                quote = refreshed;
            }
        }
        view.funding_note = df_account_app::wire_campaign::FundingNote::HostCredit(quote);
        if !quote.is_fundable {
            view.start_blockers
                .push(df_account_app::wire_campaign::StartBlocker::FundingNotReady);
        }
    }
    if !view.has_started && view.start_blockers.is_empty() {
        view = state
            .ports
            .campaigns()
            .start_session(&who, session)
            .await
            .map_err(|error| launch_after_creation_failed(campaign, error))?;
    }
    df_observe::record_audit_line(
        "portal.play_now",
        format!(
            "SESSION play_now session={} status={}",
            ids::hex(session.as_bytes()),
            if view.has_started {
                "started"
            } else {
                "funding_required"
            }
        ),
    );
    let game_unavailable = state
        .ports
        .campaigns()
        .game_links(&who, session)
        .await
        .map_err(|error| launch_after_creation_failed(campaign, error))?
        .is_none();
    reply(df_account_app::wire_campaign::PlayNowView {
        campaign_id: ids::hex(campaign.as_bytes()),
        game_unavailable,
        lobby: view,
    })
}

pub(super) async fn campaign_settings<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    Path(campaign): Path<String>,
) -> ApiResult<df_account_app::wire_campaign::CampaignSettingsView> {
    reply(
        state
            .ports
            .campaigns()
            .campaign_settings(&actor(&caller), ids::campaign(&campaign)?)
            .await?,
    )
}

pub(super) async fn save_campaign_settings<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(campaign): Path<String>,
    ApiBody(body): ApiBody<SaveDraftRequest>,
) -> ApiResult<df_account_app::wire_campaign::CampaignSettingsView> {
    let campaign = ids::campaign(&campaign)?;
    let view = state
        .ports
        .campaigns()
        .save_campaign_settings(&actor(&caller), campaign, body.draft)
        .await?;
    df_observe::record_audit_line(
        "portal.campaign_settings_saved",
        format!(
            "CAMPAIGN settings_changed campaign={}",
            ids::hex(campaign.as_bytes())
        ),
    );
    reply(view)
}

/// The pre-game launcher's view: whether the session has started and, once its runtime is
/// reachable, the table screen, the player screen and the room code. Never a developer detail.
pub(super) async fn game<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SignedIn(caller): SignedIn,
    Path(session_id): Path<String>,
) -> ApiResult<df_account_app::wire_campaign::GameLaunchView> {
    let session = ids::session(&session_id)?;
    let lobby = state
        .ports
        .campaigns()
        .lobby(&actor(&caller), session)
        .await?;
    let mut links = None;
    let reason = if !lobby.has_started {
        "session_not_started"
    } else {
        match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            state.ports.campaigns().game_links(&actor(&caller), session),
        )
        .await
        {
            Ok(Ok(Some(found)))
                if is_game_url(&found.display_url) && is_game_url(&found.player_url) =>
            {
                links = Some(found);
                "ready"
            }
            Ok(Ok(Some(_))) => "invalid_session_link",
            Ok(Ok(None)) => "placement_missing",
            Ok(Err(_)) => "runtime_unavailable",
            Err(_) => "runtime_deadline",
        }
    };
    if links.is_none() {
        df_observe::record_audit_line(
            "portal.game_unavailable",
            format!("SESSION game_unavailable session={session_id} reason={reason}"),
        );
    }
    reply(df_account_app::wire_campaign::GameLaunchView {
        session_id,
        campaign_name: lobby.campaign_name,
        has_started: lobby.has_started,
        links,
        poll_seconds: lobby.poll_seconds,
    })
}

/// Only an absolute http(s) address without credentials may be handed to the browser.
fn is_game_url(text: &str) -> bool {
    text.parse::<axum::http::Uri>().ok().is_some_and(|uri| {
        matches!(uri.scheme_str(), Some("http" | "https"))
            && uri
                .authority()
                .is_some_and(|authority| !authority.as_str().contains('@'))
    })
}

fn launch_after_creation_failed(
    campaign: df_types::CampaignId,
    error: super::ports::PortError,
) -> ApiFailure {
    df_observe::record_audit_line(
        "portal.play_now_partial",
        format!(
            "SESSION play_now_partial campaign={} reason={error:?}",
            ids::hex(campaign.as_bytes())
        ),
    );
    ApiFailure::of(ErrorCode::UnknownOutcome)
}
