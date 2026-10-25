//! The route table: the single statement of every route, its method, what it needs from the
//! caller and what authority the store checks for it. The router is built from it and the tests
//! walk it.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verb {
    Get,
    Post,
    Put,
}

/// What a route needs from the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthClass {
    /// No identity. A display ticket and a session are both ignored.
    Public,
    /// No identity, but B's pre-session CSRF token is required (it creates an unauthenticated row).
    PublicWithPresessionCsrf,
    /// A web session and its CSRF token (B's guard).
    Session,
    /// The display device cookie and the display request checks; no CSRF token.
    Device,
}

/// The authority the route checks inside the store transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gate {
    /// The role matrix: the capability for this route.
    Capability(crate::roles::Capability),
    /// The owner (typed name where the action destroys or archives).
    Owner,
    /// The host alone (promoting and demoting co-hosts, moving people between seats).
    HostOnly,
    /// The host proposes a host transfer, the owner an ownership transfer; the store decides which.
    HostOrOwner,
    /// Any active member acting on themselves.
    SelfMember,
    /// Any signed-in account (the object is theirs by construction).
    AnyAccount,
    /// Not a member action (public or device flows).
    NotMemberAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteId {
    CreateCampaign,
    Dashboard,
    Overview,
    CampaignSettings,
    CampaignSessions,
    OpenLobby,
    LobbyView,
    LobbyCharacter,
    LobbyReady,
    LobbyUnready,
    LobbyMicrophone,
    LobbyStart,
    StartDraft,
    GetDraft,
    SaveDraftStep,
    FinishDraft,
    MyMembership,
    Archive,
    Unarchive,
    RequestDelete,
    CreateInvitation,
    RevokeInvitation,
    RotateCode,
    JoinPreview,
    JoinAccept,
    CodeLookup,
    CodeRequest,
    ApproveRequest,
    DenyRequest,
    RemoveMember,
    AllowRejoin,
    SetCoHost,
    DemoteCoHost,
    Leave,
    SetSeat,
    SetPlayerRole,
    ProposeTransfer,
    AcceptTransfer,
    CancelTransfer,
    DisplayChallenge,
    DisplayStatus,
    DisplayRedeem,
    DisplayRenew,
    DisplayLookup,
    DisplayApprove,
    DisplayUnpair,
}

#[derive(Clone, Copy, Debug)]
pub struct RouteSpec {
    pub id: RouteId,
    pub verb: Verb,
    /// Relative to the mount point (`/app/api/`), axum path syntax.
    pub path: &'static str,
    pub auth: AuthClass,
    pub gate: Gate,
}

const fn spec(
    id: RouteId,
    verb: Verb,
    path: &'static str,
    auth: AuthClass,
    gate: Gate,
) -> RouteSpec {
    RouteSpec {
        id,
        verb,
        path,
        auth,
        gate,
    }
}

use AuthClass::{Device, Public, PublicWithPresessionCsrf, Session};
use Gate::{AnyAccount, HostOnly, HostOrOwner, NotMemberAction, Owner, SelfMember};
use Verb::{Get, Post, Put};

const INVITE: Gate = Gate::Capability(crate::roles::Capability::InviteOrRemovePlayers);
const START: Gate = Gate::Capability(crate::roles::Capability::StartOrPauseSession);

/// The display routes sit under `campaigns/{campaign}/sessions/{session}/display/` rather than the
/// design's `sessions/:id/lobby/display/`: a session id carries no foreign key and is looked up as
/// a pair with the campaign, so the campaign has to be in the path.
pub const ROUTES: &[RouteSpec] = &[
    spec(
        RouteId::CreateCampaign,
        Post,
        "/campaigns",
        Session,
        AnyAccount,
    ),
    spec(RouteId::Dashboard, Get, "/campaigns", Session, AnyAccount),
    spec(
        RouteId::Overview,
        Get,
        "/campaigns/{campaign}",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::CampaignSettings,
        Get,
        "/campaigns/{campaign}/settings",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::CampaignSessions,
        Get,
        "/campaigns/{campaign}/sessions",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::OpenLobby,
        Post,
        "/campaigns/{campaign}/lobby",
        Session,
        START,
    ),
    spec(
        RouteId::LobbyView,
        Get,
        "/campaigns/{campaign}/sessions/{session}/lobby",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::LobbyCharacter,
        Post,
        "/campaigns/{campaign}/sessions/{session}/lobby/character",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::LobbyReady,
        Post,
        "/campaigns/{campaign}/sessions/{session}/lobby/ready",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::LobbyUnready,
        Post,
        "/campaigns/{campaign}/sessions/{session}/lobby/unready",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::LobbyMicrophone,
        Post,
        "/campaigns/{campaign}/sessions/{session}/lobby/microphone",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::LobbyStart,
        Post,
        "/campaigns/{campaign}/sessions/{session}/lobby/start",
        Session,
        START,
    ),
    spec(
        RouteId::StartDraft,
        Post,
        "/campaigns/drafts",
        Session,
        AnyAccount,
    ),
    spec(
        RouteId::GetDraft,
        Get,
        "/campaigns/drafts/{draft}",
        Session,
        AnyAccount,
    ),
    spec(
        RouteId::SaveDraftStep,
        Put,
        "/campaigns/drafts/{draft}/steps/{step}",
        Session,
        AnyAccount,
    ),
    spec(
        RouteId::FinishDraft,
        Post,
        "/campaigns/drafts/{draft}/finish",
        Session,
        AnyAccount,
    ),
    spec(
        RouteId::MyMembership,
        Get,
        "/campaigns/{campaign}/me",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::Archive,
        Post,
        "/campaigns/{campaign}/archive",
        Session,
        Owner,
    ),
    spec(
        RouteId::Unarchive,
        Post,
        "/campaigns/{campaign}/unarchive",
        Session,
        Owner,
    ),
    spec(
        RouteId::RequestDelete,
        Post,
        "/campaigns/{campaign}/delete",
        Session,
        Owner,
    ),
    spec(
        RouteId::CreateInvitation,
        Post,
        "/campaigns/{campaign}/invitations",
        Session,
        INVITE,
    ),
    spec(
        RouteId::RevokeInvitation,
        Post,
        "/campaigns/{campaign}/invitations/{invitation}/revoke",
        Session,
        INVITE,
    ),
    spec(
        RouteId::RotateCode,
        Post,
        "/campaigns/{campaign}/code/rotate",
        Session,
        INVITE,
    ),
    spec(
        RouteId::JoinPreview,
        Post,
        "/join/preview",
        Public,
        NotMemberAction,
    ),
    spec(
        RouteId::JoinAccept,
        Post,
        "/join/accept",
        Session,
        AnyAccount,
    ),
    spec(
        RouteId::CodeLookup,
        Post,
        "/join/code/lookup",
        Session,
        AnyAccount,
    ),
    spec(
        RouteId::CodeRequest,
        Post,
        "/join/code/request",
        Session,
        AnyAccount,
    ),
    spec(
        RouteId::ApproveRequest,
        Post,
        "/campaigns/{campaign}/requests/{request}/approve",
        Session,
        INVITE,
    ),
    spec(
        RouteId::DenyRequest,
        Post,
        "/campaigns/{campaign}/requests/{request}/deny",
        Session,
        INVITE,
    ),
    spec(
        RouteId::RemoveMember,
        Post,
        "/campaigns/{campaign}/members/{member}/remove",
        Session,
        INVITE,
    ),
    spec(
        RouteId::AllowRejoin,
        Post,
        "/campaigns/{campaign}/members/{member}/allow-rejoin",
        Session,
        INVITE,
    ),
    spec(
        RouteId::SetCoHost,
        Post,
        "/campaigns/{campaign}/members/{member}/co-host",
        Session,
        HostOnly,
    ),
    spec(
        RouteId::DemoteCoHost,
        Post,
        "/campaigns/{campaign}/members/{member}/co-host/demote",
        Session,
        HostOnly,
    ),
    spec(
        RouteId::Leave,
        Post,
        "/campaigns/{campaign}/leave",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::SetSeat,
        Post,
        "/campaigns/{campaign}/seat",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::SetPlayerRole,
        Post,
        "/campaigns/{campaign}/members/{member}/role",
        Session,
        HostOnly,
    ),
    spec(
        RouteId::ProposeTransfer,
        Post,
        "/campaigns/{campaign}/transfers",
        Session,
        HostOrOwner,
    ),
    spec(
        RouteId::AcceptTransfer,
        Post,
        "/campaigns/{campaign}/transfers/{proposal}/accept",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::CancelTransfer,
        Post,
        "/campaigns/{campaign}/transfers/{proposal}/cancel",
        Session,
        SelfMember,
    ),
    spec(
        RouteId::DisplayChallenge,
        Post,
        "/display/challenge",
        PublicWithPresessionCsrf,
        NotMemberAction,
    ),
    spec(
        RouteId::DisplayStatus,
        Get,
        "/display/status",
        Device,
        NotMemberAction,
    ),
    spec(
        RouteId::DisplayRedeem,
        Post,
        "/display/redeem",
        Device,
        NotMemberAction,
    ),
    spec(
        RouteId::DisplayRenew,
        Post,
        "/display/renew",
        Device,
        NotMemberAction,
    ),
    spec(
        RouteId::DisplayLookup,
        Post,
        "/campaigns/{campaign}/sessions/{session}/display/lookup",
        Session,
        START,
    ),
    spec(
        RouteId::DisplayApprove,
        Post,
        "/campaigns/{campaign}/sessions/{session}/display/approve",
        Session,
        START,
    ),
    spec(
        RouteId::DisplayUnpair,
        Post,
        "/campaigns/{campaign}/sessions/{session}/display/unpair",
        Session,
        START,
    ),
];
