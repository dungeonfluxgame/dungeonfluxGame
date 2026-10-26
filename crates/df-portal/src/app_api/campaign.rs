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
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

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
const MAXIMUM_UNAVAILABLE_AUDIT_SESSIONS: usize = 1_024;
const UNAVAILABLE_AUDIT_INTERVAL: Duration = Duration::from_secs(60);

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

/// The host's live table, a started session first, else an open lobby; `None` when they host none.
async fn hosted_live_table<S: AccountStore, M: EmailSender, P: Ports>(
    state: &AppApiState<S, M, P>,
    who: &super::ports::Actor,
) -> Result<Option<(df_types::CampaignId, df_types::SessionId)>, ApiFailure> {
    let dashboard = state.ports.campaigns().dashboard(who).await?;
    let mut open_lobby = None;
    for card in dashboard.campaigns {
        if card.is_archived || card.role != df_account_app::wire_campaign::CampaignRole::Host {
            continue;
        }
        let Some(live) = card.live_session_id.as_deref() else {
            continue;
        };
        let (campaign, session) = (ids::campaign(&card.campaign_id)?, ids::session(live)?);
        if state
            .ports
            .campaigns()
            .lobby(who, session)
            .await?
            .has_started
        {
            return Ok(Some((campaign, session)));
        }
        open_lobby.get_or_insert((campaign, session));
    }
    Ok(open_lobby)
}

pub(super) async fn play_now<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<df_account_app::wire_campaign::PlayNowView> {
    let who = verified_actor(&caller)?;
    let operation = ids::operation(&body.operation)?;
    // One table at a time: Play now returns to the host's running (or still open) table instead of
    // starting a second one that would also spend.
    let (campaign, session) = match hosted_live_table(&state, &who).await? {
        Some(table) => {
            df_observe::record_audit_line(
                "portal.play_now",
                format!(
                    "SESSION play_now_resumed session={}",
                    ids::hex(table.1.as_bytes())
                ),
            );
            table
        }
        None => {
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
            (campaign, session)
        }
    };
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
    let configured_launcher_url = std::env::var("DF_GAME_LAUNCHER_URL").ok();
    // The account hands off to the game launcher, never to the screens themselves.
    let (reason, launcher_url) = if !lobby.has_started {
        ("session_not_started", None)
    } else if !lobby.can_manage {
        ("member_cannot_manage", None)
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let campaigns = state.ports.campaigns();
            let who = actor(&caller);
            let links = campaigns.game_links(&who, session).await?;
            let launcher = campaigns.launcher_url(&who, session).await?;
            Ok::<_, super::ports::PortError>((links, launcher))
        })
        .await
        {
            Ok(Ok((Some(found), placed_launcher_url)))
                if is_game_url(&found.display_url) && is_game_url(&found.player_url) =>
            {
                match game_launcher_url(
                    lobby.can_manage,
                    lobby.has_started,
                    true,
                    placed_launcher_url.as_deref(),
                    configured_launcher_url.as_deref(),
                ) {
                    Some(url) => ("ready", Some(url)),
                    None => ("launcher_missing", None),
                }
            }
            Ok(Ok((Some(_), _))) => ("invalid_session_link", None),
            Ok(Ok((None, _))) => ("placement_missing", None),
            Ok(Err(_)) => ("runtime_unavailable", None),
            Err(_) => ("runtime_deadline", None),
        }
    };
    if lobby.can_manage && launcher_url.is_none() {
        record_game_unavailable(&session_id, reason, Instant::now());
    }
    reply(df_account_app::wire_campaign::GameLaunchView {
        session_id,
        campaign_name: lobby.campaign_name,
        has_started: lobby.has_started,
        can_manage: lobby.can_manage,
        launcher_url,
        poll_seconds: lobby.poll_seconds,
    })
}

/// This deployment's game launcher (the desktop launcher's screen, served to the browser), from
/// DF_GAME_LAUNCHER_URL. Hosted sessions get a per-session launcher address with their placement.
pub(super) fn game_launcher_url(
    can_manage: bool,
    has_started: bool,
    links_ready: bool,
    placed_url: Option<&str>,
    configured_url: Option<&str>,
) -> Option<String> {
    if can_manage && has_started && links_ready {
        match placed_url {
            Some(url) => is_game_url(url).then(|| url.to_owned()),
            None => configured_url
                .filter(|url| is_game_url(url))
                .map(str::to_owned),
        }
    } else {
        None
    }
}

fn record_game_unavailable(session_id: &str, reason: &str, now: Instant) {
    static LAST_UNAVAILABLE: OnceLock<Mutex<GameUnavailableAudits>> = OnceLock::new();
    let mut audits = LAST_UNAVAILABLE
        .get_or_init(|| Mutex::new(GameUnavailableAudits(HashMap::new())))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !audits.should_record(session_id, reason, now) {
        return;
    }
    drop(audits);
    df_observe::record_audit_line(
        "portal.game_unavailable",
        format!("SESSION game_unavailable session={session_id} reason={reason}"),
    );
}

pub(super) struct GameUnavailableAudits(HashMap<String, (String, Instant)>);

impl GameUnavailableAudits {
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self(HashMap::new())
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    pub(super) fn should_record(&mut self, session_id: &str, reason: &str, now: Instant) -> bool {
        let should_record = self
            .0
            .get(session_id)
            .is_none_or(|(previous_reason, recorded_at)| {
                previous_reason != reason
                    || now.saturating_duration_since(*recorded_at) >= UNAVAILABLE_AUDIT_INTERVAL
            });
        if !should_record {
            return false;
        }
        if !self.0.contains_key(session_id)
            && self.0.len() >= MAXIMUM_UNAVAILABLE_AUDIT_SESSIONS
            && let Some(oldest_session) = self
                .0
                .iter()
                .min_by_key(|(_, (_, recorded_at))| *recorded_at)
                .map(|(session, _)| session.clone())
        {
            self.0.remove(&oldest_session);
        }
        self.0
            .insert(session_id.to_owned(), (reason.to_owned(), now));
        true
    }
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
