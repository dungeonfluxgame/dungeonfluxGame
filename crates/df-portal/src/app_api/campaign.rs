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
        ApproveRequestBody, CampaignCreated, CampaignOverview, CharacterRequest,
        CreateInvitationRequest, DashboardView, DraftView, InvitationCreated, InvitePreview,
        JoinAcceptRequest, JoinAccepted, JoinRequested, LobbyOpened, LobbyView, MicrophoneRequest,
        OperationRequest, PairingApproveRequest, PairingLookup, PairingLookupRequest,
        PendingInvitation, ReadyRequest, RoomCodeJoinRequest, RoomCodeLookupRequest,
        SaveDraftRequest, TokenBody, WizardCatalog,
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

pub(super) async fn choose_character<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<CharacterRequest>,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    ids::operation(&body.operation)?;
    if (body.choice != df_account_app::wire_campaign::CharacterChoice::Undecided)
        != body.reference.is_some()
    {
        return Err(ApiFailure::of(ErrorCode::Invalid));
    }
    if let Some(reference) = &body.reference {
        ids::campaign(reference)?;
    }
    let view = state
        .ports
        .campaigns()
        .choose_character(&actor(&caller), session, body)
        .await?;
    lobby_reply(&state, &caller, session, view).await
}

async fn set_ready<S: AccountStore, M: EmailSender, P: Ports>(
    state: AppApiState<S, M, P>,
    caller: Authenticated,
    session_id: String,
    body: ReadyRequest,
    is_ready: bool,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    ids::operation(&body.operation)?;
    let view = state
        .ports
        .campaigns()
        .set_ready(&actor(&caller), session, is_ready)
        .await?;
    lobby_reply(&state, &caller, session, view).await
}

pub(super) async fn ready<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<ReadyRequest>,
) -> ApiResult<LobbyView> {
    set_ready(state, caller, session_id, body, true).await
}

pub(super) async fn unready<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<ReadyRequest>,
) -> ApiResult<LobbyView> {
    set_ready(state, caller, session_id, body, false).await
}

pub(super) async fn microphone<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<MicrophoneRequest>,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    ids::operation(&body.operation)?;
    let view = state
        .ports
        .campaigns()
        .report_microphone(&actor(&caller), session, body)
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
    let view = state
        .ports
        .campaigns()
        .start_session(&actor(&caller), session)
        .await?;
    lobby_reply(&state, &caller, session, view).await
}

pub(super) async fn lookup_display<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<PairingLookupRequest>,
) -> ApiResult<PairingLookup> {
    let session = ids::session(&session_id)?;
    reply(
        state
            .ports
            .campaigns()
            .lookup_display(&actor(&caller), session, &body.code)
            .await?,
    )
}

pub(super) async fn approve_display<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<PairingApproveRequest>,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    ids::operation(&body.operation)?;
    let view = state
        .ports
        .campaigns()
        .approve_display(&actor(&caller), session, &body.pairing_id)
        .await?;
    lobby_reply(&state, &caller, session, view).await
}

pub(super) async fn unpair_display<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path((session_id, ticket_id)): Path<(String, String)>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    ids::operation(&body.operation)?;
    let view = state
        .ports
        .campaigns()
        .unpair_display(&actor(&caller), session, &ticket_id)
        .await?;
    lobby_reply(&state, &caller, session, view).await
}

pub(super) async fn unpair_all_displays<S: AccountStore, M: EmailSender, P: Ports>(
    State(state): State<AppApiState<S, M, P>>,
    SessionPost(caller): SessionPost,
    Path(session_id): Path<String>,
    ApiBody(body): ApiBody<OperationRequest>,
) -> ApiResult<LobbyView> {
    let session = ids::session(&session_id)?;
    ids::operation(&body.operation)?;
    let view = state
        .ports
        .campaigns()
        .unpair_all_displays(&actor(&caller), session)
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
