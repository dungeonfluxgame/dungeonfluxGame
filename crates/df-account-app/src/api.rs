//! Every call the app makes to its server, and how each reply is read.
//!
//! A call is a value, so flows are tested natively: `request` gives the method, path, body and the
//! guard that proves the request came from our own page; `decode` turns a status and body into a
//! typed `Reply`. A mutation whose answer never arrives is `UnknownOutcome` (look before
//! repeating), a read that fails is `Unavailable` (plan 11).

pub mod integration;

use serde::{Serialize, de::DeserializeOwned};

use crate::wire::{
    API_PREFIX, Accepted, AccountStats, AdminAccountDetail, AdminAccountRow, AdminActionView,
    AdminSummary, AllocateRequest, ApiError, AssetsView, AuditRow, ChangeEmailRequest,
    ChangePasswordRequest, CheckoutCreated, CheckoutView, CloseAccountRequest, ClosedView,
    CreateCheckoutRequest, CreditOffers, DisputeRow, EmailPreferencesRequest, ErrorCode,
    ExportRequest, ExportView, ForgotPasswordRequest, FundingView, GoodwillCreditRequest,
    OperationView, OverviewView, PlaySessionsView, ProfileRequest, ProviderRow, ReasonRequest,
    ReconciliationView, RefundRequest, ResetOutcome, ResetPasswordRequest, ReviewDecisionRequest,
    ReviewItem, RevokeSessionRequest, RevokedView, SessionView, SetPaceRequest, SettingsView,
    SignInRequest, SignUpRequest, StatementFilter, StatementsView, VerifyEmailRequest,
    VerifyOutcome,
};
use crate::wire_campaign::{
    ApproveRequestBody, CampaignCreated, CampaignOverview, CreateInvitationRequest, DashboardView,
    DraftView, InvitationCreated, InvitePreview, JoinAcceptRequest, JoinAccepted, JoinRequested,
    LobbyOpened, LobbyView, OperationRequest, PendingInvitation, RoomCodeJoinRequest,
    RoomCodeLookupRequest, SaveDraftRequest, TokenBody, WizardCatalog,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Get,
    Post,
    Put,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
        }
    }
}

/// What proves a request came from our own page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Guard {
    /// Reads change nothing and carry no token.
    None,
    /// The session's (or pre-session) CSRF token in the `x-df-csrf` header.
    Csrf,
    /// The display device's calls (redeem, renew): the `SameSite=Strict` device cookie plus a custom
    /// `x-df-device` header; no token, because a TV that sat idle has none that is fresh (S1 8.5).
    Device,
}

/// One HTTP request, ready for `fetch`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequest {
    pub method: Method,
    pub path: String,
    pub body: Option<String>,
    pub guard: Guard,
}

/// The provider-side operations an operator can act on by id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderSwitch {
    Pause,
    Resume,
}

/// A host's answer to a room-code request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JoinDecision {
    Approve(ApproveRequestBody),
    Deny(OperationRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApiCall {
    Integration(integration::Call),
    Session,
    Operation,
    SignUp(SignUpRequest),
    SignIn(SignInRequest),
    SignOut,
    VerifyEmail(VerifyEmailRequest),
    ResendVerification,
    ForgotPassword(ForgotPasswordRequest),
    ResetPassword(ResetPasswordRequest),
    Overview,
    CreditOffers,
    CreateCheckout(CreateCheckoutRequest),
    Checkout {
        checkout_id: String,
    },
    Funding,
    SetPace(SetPaceRequest),
    Allocate(AllocateRequest),
    Statements {
        cursor: Option<String>,
        filter: StatementFilter,
        from_ms: Option<u64>,
        to_ms: Option<u64>,
    },
    AccountStats,
    PlaySessions {
        cursor: Option<String>,
    },
    Assets {
        kind: Option<crate::wire::AssetKind>,
        campaign: Option<String>,
        cursor: Option<String>,
    },
    Settings,
    SaveProfile(ProfileRequest),
    ChangeEmail(ChangeEmailRequest),
    ChangePassword(ChangePasswordRequest),
    SavePreferences(EmailPreferencesRequest),
    RequestExport(ExportRequest),
    RevokeSession(RevokeSessionRequest),
    RevokeOtherSessions,
    RevokeAllSessions,
    CloseAccount(CloseAccountRequest),
    AdminSummary,
    AdminAccounts {
        query: String,
        standing: Option<String>,
        verified: Option<String>,
        has_balance: Option<String>,
        sort: Option<String>,
    },
    AdminAccount {
        account_id: String,
    },
    Refund(RefundRequest),
    GoodwillCredit(GoodwillCreditRequest),
    Disputes,
    DisputeEvidence {
        dispute_id: String,
        request: ReasonRequest,
    },
    Reconciliation,
    Providers,
    SwitchProvider {
        provider_id: String,
        switch: ProviderSwitch,
        request: ReasonRequest,
    },
    ReviewQueue,
    DecideReview {
        item_id: String,
        request: ReviewDecisionRequest,
    },
    Audit,
    Dashboard,
    PlayNow(OperationRequest),
    CampaignSettings {
        campaign_id: String,
    },
    SaveCampaignSettings {
        campaign_id: String,
        request: SaveDraftRequest,
    },
    JoinPending,
    WizardCatalog,
    StartDraft,
    SaveDraft {
        draft_id: String,
        step: u8,
        request: SaveDraftRequest,
    },
    FinishDraft {
        draft_id: String,
        request: OperationRequest,
    },
    Campaign {
        campaign_id: String,
    },
    CreateInvitation {
        campaign_id: String,
        request: CreateInvitationRequest,
    },
    RevokeInvitation {
        campaign_id: String,
        invitation_id: String,
        request: OperationRequest,
    },
    RotateCode {
        campaign_id: String,
        request: OperationRequest,
    },
    DecideJoinRequest {
        campaign_id: String,
        request_id: String,
        decision: JoinDecision,
    },
    LeaveCampaign {
        campaign_id: String,
        request: OperationRequest,
    },
    OpenLobby {
        campaign_id: String,
        request: OperationRequest,
    },
    Lobby {
        session_id: String,
    },
    GameLaunch {
        session_id: String,
    },
    StartSession {
        session_id: String,
        request: OperationRequest,
    },
    CancelLobby {
        session_id: String,
        request: OperationRequest,
    },
    ConsentHostFunding {
        session_id: String,
        request: crate::wire_campaign::HostConsentRequest,
    },
    JoinPreview(TokenBody),
    JoinHold(TokenBody),
    JoinAccept(JoinAcceptRequest),
    CodeLookup(RoomCodeLookupRequest),
    CodeRequest(RoomCodeJoinRequest),
}

/// A decoded answer; `Err` is the server's typed refusal or a transport failure.
// One reply per request, moved once from the runtime into the model; boxing every variant
// would ripple through all reply handling for no runtime benefit.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reply {
    Integration(integration::Answer),
    Session(Result<SessionView, ApiError>),
    Operation(Result<OperationView, ApiError>),
    SignedUp(Result<Accepted, ApiError>),
    SignedIn(Result<SessionView, ApiError>),
    SignedOut(Result<SessionView, ApiError>),
    Verified(Result<VerifyOutcome, ApiError>),
    Resent(Result<Accepted, ApiError>),
    ForgotSent(Result<Accepted, ApiError>),
    Reset(Result<ResetOutcome, ApiError>),
    Overview(Result<OverviewView, ApiError>),
    CreditOffers(Result<CreditOffers, ApiError>),
    CheckoutCreated(Result<CheckoutCreated, ApiError>),
    Checkout(Result<CheckoutView, ApiError>),
    Funding(Result<FundingView, ApiError>),
    PaceSaved(Result<FundingView, ApiError>),
    Allocated(Result<FundingView, ApiError>),
    Statements {
        is_continuation: bool,
        result: Result<StatementsView, ApiError>,
    },
    AccountStats(Result<AccountStats, ApiError>),
    PlaySessions {
        is_continuation: bool,
        result: Result<PlaySessionsView, ApiError>,
    },
    Assets {
        is_continuation: bool,
        result: Result<AssetsView, ApiError>,
    },
    Settings(Result<SettingsView, ApiError>),
    ProfileSaved(Result<Accepted, ApiError>),
    EmailChangeSent(Result<Accepted, ApiError>),
    PasswordChanged(Result<SessionView, ApiError>),
    PreferencesSaved(Result<Accepted, ApiError>),
    ExportRequested(Result<ExportView, ApiError>),
    Revoked(Result<RevokedView, ApiError>),
    Closed(Result<ClosedView, ApiError>),
    AdminSummary(Result<AdminSummary, ApiError>),
    AdminAccounts(Result<Vec<AdminAccountRow>, ApiError>),
    AdminAccount(Result<AdminAccountDetail, ApiError>),
    AdminAction(Result<AdminActionView, ApiError>),
    Disputes(Result<Vec<DisputeRow>, ApiError>),
    Reconciliation(Result<ReconciliationView, ApiError>),
    Providers(Result<Vec<ProviderRow>, ApiError>),
    ReviewQueue(Result<Vec<ReviewItem>, ApiError>),
    Audit(Result<Vec<AuditRow>, ApiError>),
    Dashboard(Result<DashboardView, ApiError>),
    PlayNow(Result<crate::wire_campaign::PlayNowView, ApiError>),
    CampaignSettings(Result<crate::wire_campaign::CampaignSettingsView, ApiError>),
    JoinPending(Result<PendingInvitation, ApiError>),
    WizardCatalog(Result<WizardCatalog, ApiError>),
    Draft(Result<DraftView, ApiError>),
    CampaignCreated(Result<CampaignCreated, ApiError>),
    Campaign(Result<CampaignOverview, ApiError>),
    InvitationCreated(Result<InvitationCreated, ApiError>),
    Left(Result<Accepted, ApiError>),
    LobbyOpened(Result<LobbyOpened, ApiError>),
    Lobby(Result<LobbyView, ApiError>),
    GameLaunch(Result<crate::wire_campaign::GameLaunchView, ApiError>),
    LobbyCancelled(Result<Accepted, ApiError>),
    JoinPreview(Result<InvitePreview, ApiError>),
    JoinHeld(Result<Accepted, ApiError>),
    Joined(Result<JoinAccepted, ApiError>),
    CodeRequested(Result<JoinRequested, ApiError>),
}

fn get(path: String) -> HttpRequest {
    HttpRequest {
        method: Method::Get,
        path,
        body: None,
        guard: Guard::None,
    }
}

fn send(method: Method, path: String, body: &impl Serialize) -> HttpRequest {
    HttpRequest {
        method,
        path,
        // These request types are plain strings, numbers and booleans; serialization cannot fail.
        body: Some(serde_json::to_string(body).unwrap_or_default()),
        guard: Guard::Csrf,
    }
}

fn post(path: String, body: &impl Serialize) -> HttpRequest {
    send(Method::Post, path, body)
}

fn post_empty(path: String) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        path,
        body: Some("{}".to_owned()),
        guard: Guard::Csrf,
    }
}

fn at(tail: &str) -> String {
    format!("{API_PREFIX}/{tail}")
}

/// Percent-encodes a query value (anything but unreserved characters).
fn encode_component(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

impl ApiCall {
    pub fn request(&self) -> HttpRequest {
        match self {
            Self::Integration(call) => call.request(),
            Self::Session => get(at("session")),
            Self::Operation => get(at("operation")),
            Self::SignUp(body) => post(at("sign-up"), body),
            Self::SignIn(body) => post(at("sign-in"), body),
            Self::SignOut => post_empty(at("sign-out")),
            Self::VerifyEmail(body) => post(at("verify-email"), body),
            Self::ResendVerification => post_empty(at("verify-email/resend")),
            Self::ForgotPassword(body) => post(at("password/forgot"), body),
            Self::ResetPassword(body) => post(at("password/reset"), body),
            Self::Overview => get(at("overview")),
            Self::CreditOffers => get(at("credit/offers")),
            Self::CreateCheckout(body) => post(at("credit/checkouts"), body),
            Self::Checkout { checkout_id } => get(at(&format!("credit/checkouts/{checkout_id}"))),
            Self::Funding => get(at("funding")),
            Self::SetPace(body) => post(at("funding/pace"), body),
            Self::Allocate(body) => post(at("funding/allocate"), body),
            Self::Statements {
                cursor,
                filter,
                from_ms,
                to_ms,
            } => {
                let filter_name = match filter {
                    StatementFilter::All => "all",
                    StatementFilter::TopUps => "top_ups",
                    StatementFilter::Spend => "spend",
                };
                let mut path = format!("statements?filter={filter_name}");
                if let Some(cursor) = cursor {
                    path.push_str(&format!("&cursor={}", encode_component(cursor)));
                }
                if let Some(from_ms) = from_ms {
                    path.push_str(&format!("&from_ms={from_ms}"));
                }
                if let Some(to_ms) = to_ms {
                    path.push_str(&format!("&to_ms={to_ms}"));
                }
                get(at(&path))
            }
            Self::AccountStats => get(at("account-stats")),
            Self::PlaySessions { cursor } => match cursor {
                None => get(at("play-sessions")),
                Some(cursor) => get(at(&format!(
                    "play-sessions?cursor={}",
                    encode_component(cursor)
                ))),
            },
            Self::Assets {
                kind,
                campaign,
                cursor,
            } => {
                let mut path = "assets?".to_owned();
                if let Some(kind) = kind {
                    let name = match kind {
                        crate::wire::AssetKind::Portrait => "portrait",
                        crate::wire::AssetKind::Scene => "scene",
                        crate::wire::AssetKind::Music => "music",
                        crate::wire::AssetKind::Video => "video",
                    };
                    path.push_str(&format!("kind={name}&"));
                }
                if let Some(campaign) = campaign {
                    path.push_str(&format!("campaign={}&", encode_component(campaign)));
                }
                if let Some(cursor) = cursor {
                    path.push_str(&format!("cursor={}", encode_component(cursor)));
                }
                get(at(&path))
            }
            Self::Settings => get(at("settings")),
            Self::SaveProfile(body) => post(at("settings/profile"), body),
            Self::ChangeEmail(body) => post(at("settings/email"), body),
            Self::ChangePassword(body) => post(at("settings/password"), body),
            Self::SavePreferences(body) => post(at("settings/preferences"), body),
            Self::RequestExport(body) => post(at("settings/export"), body),
            Self::RevokeSession(body) => post(at("settings/sessions/revoke"), body),
            Self::RevokeOtherSessions => post_empty(at("settings/sessions/revoke-others")),
            Self::RevokeAllSessions => post_empty(at("settings/sessions/revoke-all")),
            Self::CloseAccount(body) => post(at("settings/close"), body),
            Self::AdminSummary => get(at("admin/summary")),
            Self::AdminAccounts {
                query,
                standing,
                verified,
                has_balance,
                sort,
            } => {
                let mut path = format!("admin/accounts?q={}", encode_component(query));
                if let Some(value) = standing {
                    path.push_str(&format!("&standing={}", encode_component(value)));
                }
                if let Some(value) = verified {
                    path.push_str(&format!("&verified={}", encode_component(value)));
                }
                if let Some(value) = has_balance {
                    path.push_str(&format!("&has_balance={}", encode_component(value)));
                }
                if let Some(value) = sort {
                    path.push_str(&format!("&sort={}", encode_component(value)));
                }
                get(at(&path))
            }
            Self::AdminAccount { account_id } => get(at(&format!("admin/accounts/{account_id}"))),
            Self::Refund(body) => post(at("admin/refunds"), body),
            Self::GoodwillCredit(body) => post(at("admin/credits"), body),
            Self::Disputes => get(at("admin/disputes")),
            Self::DisputeEvidence {
                dispute_id,
                request,
            } => post(
                at(&format!("admin/disputes/{dispute_id}/evidence")),
                request,
            ),
            Self::Reconciliation => get(at("admin/reconciliation")),
            Self::Providers => get(at("admin/providers")),
            Self::SwitchProvider {
                provider_id,
                switch,
                request,
            } => {
                let verb = match switch {
                    ProviderSwitch::Pause => "pause",
                    ProviderSwitch::Resume => "resume",
                };
                post(
                    at(&format!("admin/providers/{provider_id}/{verb}")),
                    request,
                )
            }
            Self::ReviewQueue => get(at("admin/review")),
            Self::DecideReview { item_id, request } => {
                post(at(&format!("admin/review/{item_id}/decision")), request)
            }
            Self::Audit => get(at("admin/audit")),
            other => other.campaign_request(),
        }
    }

    fn campaign_request(&self) -> HttpRequest {
        match self {
            Self::Dashboard => get(at("campaigns")),
            Self::PlayNow(request) => post(at("campaigns/play-now"), request),
            Self::CampaignSettings { campaign_id } => {
                get(at(&format!("campaigns/{campaign_id}/settings")))
            }
            Self::SaveCampaignSettings {
                campaign_id,
                request,
            } => send(
                Method::Put,
                at(&format!("campaigns/{campaign_id}/settings")),
                request,
            ),
            Self::JoinPending => get(at("join/pending")),
            Self::WizardCatalog => get(at("campaigns/new")),
            Self::StartDraft => post_empty(at("campaigns/drafts")),
            Self::SaveDraft {
                draft_id,
                step,
                request,
            } => send(
                Method::Put,
                at(&format!("campaigns/drafts/{draft_id}/steps/{step}")),
                request,
            ),
            Self::FinishDraft { draft_id, request } => {
                post(at(&format!("campaigns/drafts/{draft_id}/finish")), request)
            }
            Self::Campaign { campaign_id } => get(at(&format!("campaigns/{campaign_id}"))),
            Self::CreateInvitation {
                campaign_id,
                request,
            } => post(at(&format!("campaigns/{campaign_id}/invitations")), request),
            Self::RevokeInvitation {
                campaign_id,
                invitation_id,
                request,
            } => post(
                at(&format!(
                    "campaigns/{campaign_id}/invitations/{invitation_id}/revoke"
                )),
                request,
            ),
            Self::RotateCode {
                campaign_id,
                request,
            } => post(at(&format!("campaigns/{campaign_id}/code/rotate")), request),
            Self::DecideJoinRequest {
                campaign_id,
                request_id,
                decision: JoinDecision::Approve(body),
            } => post(
                at(&format!(
                    "campaigns/{campaign_id}/requests/{request_id}/approve"
                )),
                body,
            ),
            Self::DecideJoinRequest {
                campaign_id,
                request_id,
                decision: JoinDecision::Deny(body),
            } => post(
                at(&format!(
                    "campaigns/{campaign_id}/requests/{request_id}/deny"
                )),
                body,
            ),
            Self::LeaveCampaign {
                campaign_id,
                request,
            } => post(at(&format!("campaigns/{campaign_id}/leave")), request),
            Self::OpenLobby {
                campaign_id,
                request,
            } => post(at(&format!("campaigns/{campaign_id}/lobby")), request),
            other => other.session_request(),
        }
    }

    fn session_request(&self) -> HttpRequest {
        match self {
            Self::CancelLobby {
                session_id,
                request,
            } => post(at(&format!("sessions/{session_id}/lobby/cancel")), request),
            Self::Lobby { session_id } => get(at(&format!("sessions/{session_id}/lobby"))),
            Self::GameLaunch { session_id } => get(at(&format!("sessions/{session_id}/game"))),
            Self::StartSession {
                session_id,
                request,
            } => post(at(&format!("sessions/{session_id}/lobby/start")), request),
            Self::ConsentHostFunding {
                session_id,
                request,
            } => post(
                at(&format!("sessions/{session_id}/lobby/funding/consent")),
                request,
            ),
            Self::JoinPreview(body) => post(at("join/preview"), body),
            Self::JoinHold(body) => post(at("join/hold"), body),
            Self::JoinAccept(body) => post(at("join/accept"), body),
            Self::CodeLookup(body) => post(at("join/code/lookup"), body),
            Self::CodeRequest(body) => post(at("join/code/request"), body),
            _ => unreachable!("every call is matched by `request`, `campaign_request` or here"),
        }
    }

    pub fn is_mutation(&self) -> bool {
        self.request().method != Method::Get
    }

    /// Reads the server's answer.
    pub fn decode(&self, status: u16, body: &str) -> Reply {
        if let Self::Integration(call) = self {
            return Reply::Integration(call.decode(status, body));
        }
        let is_mutation = self.is_mutation();
        macro_rules! read {
            () => {
                decode(status, body, is_mutation)
            };
        }
        match self {
            Self::Session => Reply::Session(read!()),
            Self::Operation => Reply::Operation(read!()),
            Self::SignUp(_) => Reply::SignedUp(read!()),
            Self::SignIn(_) => Reply::SignedIn(read!()),
            Self::SignOut => Reply::SignedOut(read!()),
            Self::VerifyEmail(_) => Reply::Verified(read!()),
            Self::ResendVerification => Reply::Resent(read!()),
            Self::ForgotPassword(_) => Reply::ForgotSent(read!()),
            Self::ResetPassword(_) => Reply::Reset(read!()),
            Self::Overview => Reply::Overview(read!()),
            Self::CreditOffers => Reply::CreditOffers(read!()),
            Self::CreateCheckout(_) => Reply::CheckoutCreated(read!()),
            Self::Checkout { .. } => Reply::Checkout(read!()),
            Self::Funding => Reply::Funding(read!()),
            Self::SetPace(_) => Reply::PaceSaved(read!()),
            Self::Allocate(_) => Reply::Allocated(read!()),
            Self::Statements { cursor, .. } => Reply::Statements {
                is_continuation: cursor.is_some(),
                result: read!(),
            },
            Self::AccountStats => Reply::AccountStats(read!()),
            Self::PlaySessions { cursor } => Reply::PlaySessions {
                is_continuation: cursor.is_some(),
                result: read!(),
            },
            Self::Assets { cursor, .. } => Reply::Assets {
                is_continuation: cursor.is_some(),
                result: read!(),
            },
            Self::Settings => Reply::Settings(read!()),
            Self::SaveProfile(_) => Reply::ProfileSaved(read!()),
            Self::ChangeEmail(_) => Reply::EmailChangeSent(read!()),
            Self::ChangePassword(_) => Reply::PasswordChanged(read!()),
            Self::SavePreferences(_) => Reply::PreferencesSaved(read!()),
            Self::RequestExport(_) => Reply::ExportRequested(read!()),
            Self::RevokeSession(_) | Self::RevokeOtherSessions | Self::RevokeAllSessions => {
                Reply::Revoked(read!())
            }
            Self::CloseAccount(_) => Reply::Closed(read!()),
            Self::AdminSummary => Reply::AdminSummary(read!()),
            Self::AdminAccounts { .. } => Reply::AdminAccounts(read!()),
            Self::AdminAccount { .. } => Reply::AdminAccount(read!()),
            Self::Refund(_)
            | Self::GoodwillCredit(_)
            | Self::DisputeEvidence { .. }
            | Self::SwitchProvider { .. }
            | Self::DecideReview { .. } => Reply::AdminAction(read!()),
            Self::Disputes => Reply::Disputes(read!()),
            Self::Reconciliation => Reply::Reconciliation(read!()),
            Self::Providers => Reply::Providers(read!()),
            Self::ReviewQueue => Reply::ReviewQueue(read!()),
            Self::Audit => Reply::Audit(read!()),
            other => other.decode_campaign(status, body),
        }
    }

    fn decode_campaign(&self, status: u16, body: &str) -> Reply {
        let is_mutation = self.is_mutation();
        macro_rules! read {
            () => {
                decode(status, body, is_mutation)
            };
        }
        match self {
            Self::Dashboard => Reply::Dashboard(read!()),
            Self::PlayNow(_) => Reply::PlayNow(read!()),
            Self::CampaignSettings { .. } | Self::SaveCampaignSettings { .. } => {
                Reply::CampaignSettings(read!())
            }
            Self::JoinPending => Reply::JoinPending(read!()),
            Self::WizardCatalog => Reply::WizardCatalog(read!()),
            Self::StartDraft | Self::SaveDraft { .. } => Reply::Draft(read!()),
            Self::FinishDraft { .. } => Reply::CampaignCreated(read!()),
            Self::Campaign { .. }
            | Self::RevokeInvitation { .. }
            | Self::RotateCode { .. }
            | Self::DecideJoinRequest { .. } => Reply::Campaign(read!()),
            Self::CreateInvitation { .. } => Reply::InvitationCreated(read!()),
            Self::LeaveCampaign { .. } => Reply::Left(read!()),
            Self::OpenLobby { .. } => Reply::LobbyOpened(read!()),
            Self::CancelLobby { .. } => Reply::LobbyCancelled(read!()),
            Self::GameLaunch { .. } => Reply::GameLaunch(read!()),
            Self::Lobby { .. } | Self::ConsentHostFunding { .. } | Self::StartSession { .. } => {
                Reply::Lobby(read!())
            }
            Self::JoinPreview(_) | Self::CodeLookup(_) => Reply::JoinPreview(read!()),
            Self::JoinHold(_) => Reply::JoinHeld(read!()),
            Self::JoinAccept(_) => Reply::Joined(read!()),
            Self::CodeRequest(_) => Reply::CodeRequested(read!()),
            _ => unreachable!("every account call is matched in `decode`"),
        }
    }

    /// The reply for a request that never got an answer (network failure or deadline).
    pub fn lost(&self) -> Reply {
        let error = ApiError::of(if self.is_mutation() {
            ErrorCode::UnknownOutcome
        } else {
            ErrorCode::Unavailable
        });
        let body = serde_json::to_string(&error).unwrap_or_default();
        self.decode(0, &body)
    }
}

fn decode<T: DeserializeOwned>(status: u16, body: &str, is_mutation: bool) -> Result<T, ApiError> {
    if (200..300).contains(&status) {
        return serde_json::from_str(body).map_err(|_| {
            ApiError::of(if is_mutation {
                ErrorCode::UnknownOutcome
            } else {
                ErrorCode::Unavailable
            })
        });
    }
    if let Ok(error) = serde_json::from_str::<ApiError>(body) {
        return Err(error);
    }
    Err(ApiError::of(match status {
        401 => ErrorCode::SignedOut,
        403 => ErrorCode::Forbidden,
        404 => ErrorCode::NotFound,
        429 => ErrorCode::Throttled,
        _ if is_mutation => ErrorCode::UnknownOutcome,
        _ => ErrorCode::Unavailable,
    }))
}

#[cfg(test)]
mod api_tests;
