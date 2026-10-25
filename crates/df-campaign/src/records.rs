//! The values the campaign store keeps and returns (`planning/campaigns-and-table-research.md`
//! section 6). Times are milliseconds since the Unix epoch; ids are opaque. These are plain
//! values: holding one grants nothing.

use df_types::{AccountId, CampaignId, MemberId, SessionId, TenantId};

use crate::{
    ids::{FeedEpoch, InvitationId, PairingId, ProposalId, RequestId, TicketId},
    join_invitation::JoinRole,
    roles::{CampaignRole, Delegations},
};

/// The campaign lifecycle. `Draft` belongs to the create wizard (a later round).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CampaignStatus {
    Draft,
    Active,
    Archived,
    Deleting,
}

impl CampaignStatus {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Deleting => "deleting",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "draft" => Some(Self::Draft),
            "active" => Some(Self::Active),
            "archived" => Some(Self::Archived),
            "deleting" => Some(Self::Deleting),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoomCodes {
    Off,
    WithApproval,
}

impl RoomCodes {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::WithApproval => "with_approval",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(Self::Off),
            "with_approval" => Some(Self::WithApproval),
            _ => None,
        }
    }
}

/// The content rating a campaign was created with; it is the only rating text an invitation
/// preview shows.
pub use df_campaign_wire::ContentRating;

#[derive(Clone, Debug)]
pub struct CampaignRow {
    pub id: CampaignId,
    pub tenant: TenantId,
    pub owner: AccountId,
    pub name: String,
    pub seat_count: u32,
    pub content_rating: ContentRating,
    pub room_codes: RoomCodes,
    pub default_co_host_delegations: Delegations,
    pub display_summary_enabled: bool,
    pub planned_session_at_ms: Option<u64>,
    pub status: CampaignStatus,
    pub revision: u64,
    pub event_seq: u64,
    pub feed_epoch: FeedEpoch,
    pub last_event_at_ms: u64,
    pub created_at_ms: u64,
    pub archived_at_ms: Option<u64>,
    pub scheduled_delete_at_ms: Option<u64>,
}

/// What a creator supplies; everything else is set by the store.
#[derive(Clone, Debug)]
pub struct NewCampaign {
    pub tenant: TenantId,
    pub owner: AccountId,
    pub name: String,
    pub seat_count: u32,
    pub content_rating: ContentRating,
    pub room_codes: RoomCodes,
    pub default_co_host_delegations: Delegations,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberStatus {
    Active,
    Left,
    Removed,
}

impl MemberStatus {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Left => "left",
            Self::Removed => "removed",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Self::Active),
            "left" => Some(Self::Left),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

/// The text a member row stores for a role (a display is never a member).
pub const fn role_text(role: CampaignRole) -> &'static str {
    match role {
        CampaignRole::Host => "host",
        CampaignRole::CoHost => "co_host",
        CampaignRole::Player => "player",
        CampaignRole::Contributor => "contributor",
        CampaignRole::PairedDisplay => "paired_display",
    }
}

pub fn parse_role(text: &str) -> Option<CampaignRole> {
    match text {
        "host" => Some(CampaignRole::Host),
        "co_host" => Some(CampaignRole::CoHost),
        "player" => Some(CampaignRole::Player),
        "contributor" => Some(CampaignRole::Contributor),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct MemberRow {
    pub member: MemberId,
    pub campaign: CampaignId,
    pub account: AccountId,
    pub role: CampaignRole,
    pub holds_seat: bool,
    pub delegations: Delegations,
    pub status: MemberStatus,
    pub may_rejoin: bool,
    pub removed_by_role: Option<CampaignRole>,
    pub last_invitation: Option<InvitationId>,
    pub joined_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub revision: u64,
}

/// Seats and contributor slots in use, counted under the campaign lock.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SeatCounts {
    pub seats_in_use: u32,
    pub contributors_in_use: u32,
}

/// The account states the campaign service distinguishes. `Missing` counts as not active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountStatus {
    Active,
    Closing,
    Closed,
    Missing,
}

impl AccountStatus {
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn parse(text: Option<&str>) -> Self {
        match text {
            Some("active") => Self::Active,
            Some("closing") => Self::Closing,
            Some("closed") => Self::Closed,
            _ => Self::Missing,
        }
    }
}

/// The one read the edge, O2, F and B use (5.1). `authorized` is false for a member whose
/// account is not active, whatever the member row says.
#[derive(Clone, Debug)]
pub struct MemberSnapshot {
    pub member: MemberId,
    pub account: AccountId,
    pub campaign: CampaignId,
    pub tenant: TenantId,
    pub role: CampaignRole,
    pub holds_seat: bool,
    pub delegations: Delegations,
    pub status: MemberStatus,
    pub authorized: bool,
    pub member_revision: u64,
    pub campaign_status: CampaignStatus,
    pub feed_epoch: FeedEpoch,
    pub seat_count: u32,
    pub event_seq: u64,
}

#[derive(Clone, Debug)]
pub struct InvitationRow {
    pub id: InvitationId,
    pub campaign: CampaignId,
    pub token_digest: [u8; 32],
    pub created_by: MemberId,
    pub joins_as: JoinRole,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub max_uses: u32,
    pub use_count: u32,
    pub revoked_at_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct CodeRow {
    pub code_digest: [u8; 32],
    pub campaign: CampaignId,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestState {
    Pending,
    Approved,
    Denied,
    Withdrawn,
}

impl RequestState {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Withdrawn => "withdrawn",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "denied" => Some(Self::Denied),
            "withdrawn" => Some(Self::Withdrawn),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RequestRow {
    pub id: RequestId,
    pub campaign: CampaignId,
    pub account: AccountId,
    pub state: RequestState,
    pub approved_as: Option<JoinRole>,
    pub created_at_ms: u64,
    pub decided_by: Option<MemberId>,
    pub decided_at_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferKind {
    Host,
    Ownership,
}

impl TransferKind {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Ownership => "ownership",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "host" => Some(Self::Host),
            "ownership" => Some(Self::Ownership),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProposalState {
    Proposed,
    Accepted,
    Cancelled,
    Expired,
}

impl ProposalState {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "proposed" => Some(Self::Proposed),
            "accepted" => Some(Self::Accepted),
            "cancelled" => Some(Self::Cancelled),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

/// What the displaced host becomes in a host transfer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OldHostBecomes {
    Player,
    CoHost,
}

#[derive(Clone, Debug)]
pub struct ProposalRow {
    pub id: ProposalId,
    pub campaign: CampaignId,
    pub kind: TransferKind,
    pub from_member: MemberId,
    pub from_account: AccountId,
    pub to_member: MemberId,
    pub to_account: AccountId,
    pub old_host_becomes: Option<OldHostBecomes>,
    pub campaign_revision: u64,
    pub proposed_at_ms: u64,
    pub expires_at_ms: u64,
    pub state: ProposalState,
}

/// The coarse device class a display reports, from the same classification as sessions.
pub use df_campaign_wire::DeviceLabel;

/// A pairing's state is the pure policy's.
pub use crate::display_pairing::PairingState;

pub const fn pairing_state_text(state: PairingState) -> &'static str {
    match state {
        PairingState::Pending => "pending",
        PairingState::Approved => "approved",
        PairingState::Redeemed => "redeemed",
        PairingState::Cancelled => "cancelled",
        PairingState::Expired => "expired",
    }
}

pub fn parse_pairing_state(text: &str) -> Option<PairingState> {
    match text {
        "pending" => Some(PairingState::Pending),
        "approved" => Some(PairingState::Approved),
        "redeemed" => Some(PairingState::Redeemed),
        "cancelled" => Some(PairingState::Cancelled),
        "expired" => Some(PairingState::Expired),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct PairingRow {
    pub id: PairingId,
    pub campaign: Option<CampaignId>,
    pub user_code: Option<String>,
    pub device_digest: [u8; 32],
    pub device_label: DeviceLabel,
    pub ip_key: [u8; 32],
    pub ip_key_loose: [u8; 32],
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub state: PairingState,
    pub session: Option<SessionId>,
    pub approved_by: Option<MemberId>,
    pub approved_at_ms: Option<u64>,
    pub redeemed_at_ms: Option<u64>,
    pub reissue_count: u8,
    pub renewable_until_ms: Option<u64>,
}

/// Why a display ticket was revoked (a closed set).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevokeReason {
    Unpaired,
    HostTransferred,
    Archived,
    SessionEnded,
    ApproverAuthorityLost,
    Reissued,
    ExpiredCleanup,
}

impl RevokeReason {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Unpaired => "unpaired",
            Self::HostTransferred => "host_transferred",
            Self::Archived => "archived",
            Self::SessionEnded => "session_ended",
            Self::ApproverAuthorityLost => "approver_authority_lost",
            Self::Reissued => "reissued",
            Self::ExpiredCleanup => "expired_cleanup",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "unpaired" => Some(Self::Unpaired),
            "host_transferred" => Some(Self::HostTransferred),
            "archived" => Some(Self::Archived),
            "session_ended" => Some(Self::SessionEnded),
            "approver_authority_lost" => Some(Self::ApproverAuthorityLost),
            "reissued" => Some(Self::Reissued),
            "expired_cleanup" => Some(Self::ExpiredCleanup),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TicketRow {
    pub id: TicketId,
    pub token_digest: [u8; 32],
    pub pairing: PairingId,
    pub campaign: CampaignId,
    pub session: SessionId,
    pub device_digest: [u8; 32],
    pub device_label: DeviceLabel,
    pub approved_by: MemberId,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
    pub revoke_reason: Option<RevokeReason>,
}

/// Who an event concerns (5.1): a member, a display ticket, or the whole campaign.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventScope {
    Member,
    Ticket,
    Campaign,
}

impl EventScope {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Ticket => "ticket",
            Self::Campaign => "campaign",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "member" => Some(Self::Member),
            "ticket" => Some(Self::Ticket),
            "campaign" => Some(Self::Campaign),
            _ => None,
        }
    }
}

/// The closed set of event kinds. A consumer that meets an unknown kind treats it as a
/// campaign-scope revocation and alerts (5.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Joined,
    Rejoined,
    Left,
    Removed,
    RejoinAllowed,
    RoleChanged,
    DelegationsChanged,
    SeatChanged,
    InvitationsRevoked,
    HostTransferred,
    OwnershipTransferred,
    Archived,
    Unarchived,
    DeleteRequested,
    DeleteScheduled,
    DeleteCancelled,
    DisplayTicketRevoked,
}

impl EventKind {
    pub const ALL: [Self; 17] = [
        Self::Joined,
        Self::Rejoined,
        Self::Left,
        Self::Removed,
        Self::RejoinAllowed,
        Self::RoleChanged,
        Self::DelegationsChanged,
        Self::SeatChanged,
        Self::InvitationsRevoked,
        Self::HostTransferred,
        Self::OwnershipTransferred,
        Self::Archived,
        Self::Unarchived,
        Self::DeleteRequested,
        Self::DeleteScheduled,
        Self::DeleteCancelled,
        Self::DisplayTicketRevoked,
    ];

    pub const fn text(self) -> &'static str {
        match self {
            Self::Joined => "joined",
            Self::Rejoined => "rejoined",
            Self::Left => "left",
            Self::Removed => "removed",
            Self::RejoinAllowed => "rejoin_allowed",
            Self::RoleChanged => "role_changed",
            Self::DelegationsChanged => "delegations_changed",
            Self::SeatChanged => "seat_changed",
            Self::InvitationsRevoked => "invitations_revoked",
            Self::HostTransferred => "host_transferred",
            Self::OwnershipTransferred => "ownership_transferred",
            Self::Archived => "archived",
            Self::Unarchived => "unarchived",
            Self::DeleteRequested => "delete_requested",
            Self::DeleteScheduled => "delete_scheduled",
            Self::DeleteCancelled => "delete_cancelled",
            Self::DisplayTicketRevoked => "display_ticket_revoked",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.text() == text)
    }

    /// The scope fixed by the kind.
    pub const fn scope(self) -> EventScope {
        match self {
            Self::Joined
            | Self::Rejoined
            | Self::Left
            | Self::Removed
            | Self::RejoinAllowed
            | Self::RoleChanged
            | Self::DelegationsChanged
            | Self::SeatChanged
            | Self::HostTransferred
            | Self::OwnershipTransferred => EventScope::Member,
            Self::DisplayTicketRevoked => EventScope::Ticket,
            Self::InvitationsRevoked
            | Self::Archived
            | Self::Unarchived
            | Self::DeleteRequested
            | Self::DeleteScheduled
            | Self::DeleteCancelled => EventScope::Campaign,
        }
    }

    /// Whether the kind is always revoking, never, or depends on the change (`None`): a role,
    /// seat or delegation change is revoking when it lowers standing or clears a delegation, and a
    /// transfer row is revoking for the old holder only.
    pub const fn fixed_revoking(self) -> Option<bool> {
        match self {
            Self::Joined
            | Self::Rejoined
            | Self::RejoinAllowed
            | Self::InvitationsRevoked
            | Self::Unarchived
            | Self::DeleteCancelled => Some(false),
            Self::Left
            | Self::Removed
            | Self::Archived
            | Self::DeleteRequested
            | Self::DeleteScheduled
            | Self::DisplayTicketRevoked => Some(true),
            Self::RoleChanged
            | Self::DelegationsChanged
            | Self::SeatChanged
            | Self::HostTransferred
            | Self::OwnershipTransferred => None,
        }
    }
}

/// An event to append. The constructors keep the scope columns consistent with the kind; the
/// database CHECK constraint refuses any other shape.
#[derive(Clone, Debug)]
pub struct NewEvent {
    pub kind: EventKind,
    pub member: Option<(MemberId, u64)>,
    pub ticket: Option<TicketId>,
    pub revoking: bool,
    pub actor: Option<MemberId>,
    pub writer_fence: Option<Vec<u8>>,
}

impl NewEvent {
    /// A member-scope event carrying the member's new revision.
    pub fn member(
        kind: EventKind,
        member: MemberId,
        member_revision: u64,
        revoking: bool,
        actor: Option<MemberId>,
    ) -> Self {
        debug_assert_eq!(kind.scope(), EventScope::Member);
        debug_assert!(kind.fixed_revoking().is_none_or(|fixed| fixed == revoking));
        Self {
            kind,
            member: Some((member, member_revision)),
            ticket: None,
            revoking,
            actor,
            writer_fence: None,
        }
    }

    /// A campaign-scope event.
    pub fn campaign(kind: EventKind, actor: Option<MemberId>) -> Self {
        debug_assert_eq!(kind.scope(), EventScope::Campaign);
        Self {
            kind,
            member: None,
            ticket: None,
            revoking: kind.fixed_revoking().unwrap_or(false),
            actor,
            writer_fence: None,
        }
    }

    /// A ticket-scope event.
    pub fn ticket(ticket: TicketId, actor: Option<MemberId>) -> Self {
        Self {
            kind: EventKind::DisplayTicketRevoked,
            member: None,
            ticket: Some(ticket),
            revoking: true,
            actor,
            writer_fence: None,
        }
    }

    #[must_use]
    pub fn with_writer_fence(mut self, fence: Vec<u8>) -> Self {
        self.writer_fence = Some(fence);
        self
    }
}

#[derive(Clone, Debug)]
pub struct EventRow {
    pub campaign: CampaignId,
    pub event_seq: u64,
    pub scope: EventScope,
    pub feed_epoch: FeedEpoch,
    pub kind: EventKind,
    pub member: Option<MemberId>,
    pub member_revision: Option<u64>,
    pub revoking: bool,
    pub at_ms: u64,
    pub actor: Option<MemberId>,
    pub ticket: Option<TicketId>,
    pub writer_fence: Option<Vec<u8>>,
}

/// A campaign's head in the feed: where its events end and which epoch they belong to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CampaignHead {
    pub campaign: CampaignId,
    pub event_seq: u64,
    pub feed_epoch: FeedEpoch,
}

/// One entry of the discovery hint read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CampaignChange {
    pub campaign: CampaignId,
    pub event_seq: u64,
    pub feed_epoch: FeedEpoch,
    pub last_event_at_ms: u64,
}

/// Where a chunk of `leave_all` stopped (8.7 step 7).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaveChunk {
    /// More memberships may remain; resume after this campaign.
    Continue(CampaignId),
    Done,
    /// The account is no longer `closing` (a reopening login landed between chunks); nothing
    /// changed and the caller stops.
    Aborted,
}

/// What a chunk did, for the caller's audit lines and alerts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkReport {
    pub outcome: LeaveChunk,
    pub left: Vec<CampaignId>,
    pub handed_over: Vec<CampaignId>,
    /// Campaigns the account owns; scheduled deletion handles them.
    pub skipped_owned: Vec<CampaignId>,
    /// `closure_late_host`: the account was still host of an active campaign it does not own.
    pub late_hosts: Vec<CampaignId>,
    /// `closure_handover_skipped`: the owner was not active; the closing account kept the row.
    pub handover_skipped: Vec<CampaignId>,
}

/// What reopening did to the scheduled deletions of an account's owned campaigns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CancelReport {
    pub cancelled: Vec<CampaignId>,
    /// Already `deleting`: the cancellation is refused and the person is told.
    pub already_deleting: Vec<CampaignId>,
}
