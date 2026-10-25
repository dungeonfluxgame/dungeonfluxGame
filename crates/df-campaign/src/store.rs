//! The port to the campaign tables, and the typed ports to the owners this crate depends on.
//!
//! `CampaignStore` is implemented by `df-persistence` (`campaign_store`, PostgreSQL): every
//! mutating method is one transaction that also appends its events, and every method takes the
//! locks in the order of launch decision 29 (accounts, then O1 rows, then `campaigns.campaign`,
//! then campaign child rows). Other owners sit behind their own small ports below, each with a
//! faithful fake in `fakes` (feature `test-fakes`) and a tracked TODO where the real adapter
//! belongs to that owner.

use std::future::Future;

use df_types::{AccountId, CampaignId, MemberId, OperationId, SessionId};

use crate::{
    errors::{Outcome, StoreResult},
    ids::{InvitationId, PairingId, ProposalId, RequestId, TicketId},
    join_invitation::JoinRole,
    records::{
        AccountStatus, CampaignChange, CampaignHead, CampaignRow, CodeRow, DeviceLabel, EventRow,
        InvitationRow, MemberRow, MemberSnapshot, NewCampaign, OldHostBecomes, PairingRow,
        RequestRow, RevokeReason, TicketRow, TransferKind,
    },
    roles::Delegations,
};

// ---- Typed ports to other owners ------------------------------------------------------------

/// What B's accounts know about an account, for names and creation checks.
///
/// TODO(S1-02): the real adapter reads B's account tables. `status` mirrors `accounts.account_status`.
#[derive(Clone, Debug)]
pub struct AccountProfile {
    pub display_name: Option<String>,
    pub verified: bool,
    pub tenant: Option<df_types::TenantId>,
    pub status: AccountStatus,
}

pub trait AccountDirectory: Send + Sync + 'static {
    fn profile(
        &self,
        account: AccountId,
    ) -> impl Future<Output = StoreResult<Option<AccountProfile>>> + Send;
}

/// Where a campaign's session stands, as O1 reports it. Only the distinctions the campaign
/// service needs: whether it is still a lobby, whether paid work may run, whether it has ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionPhase {
    Lobby,
    PreparingFundingClosed,
    PreparingFundingOpen,
    Playing,
    Ended,
}

impl SessionPhase {
    /// Archive and delete refuse only once the session is past the lobby (8.4).
    pub const fn is_past_lobby(self) -> bool {
        matches!(self, Self::PreparingFundingOpen | Self::Playing)
    }

    /// An ended session does not authorize a display ticket.
    pub const fn is_usable(self) -> bool {
        !matches!(self, Self::Ended)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionFacts {
    pub campaign: CampaignId,
    pub phase: SessionPhase,
}

/// O1's session directory. A read that fails is `Err`; callers treat an unreadable session as
/// ended (fail closed).
///
/// TODO(S1-03): the real adapter calls `open_session`, `campaign_of` and the phase read.
pub trait SessionDirectory: Send + Sync + 'static {
    fn session(
        &self,
        session: SessionId,
    ) -> impl Future<Output = StoreResult<Option<SessionFacts>>> + Send;

    /// The campaign's single non-ended session, if any.
    fn live_session_of(
        &self,
        campaign: CampaignId,
    ) -> impl Future<Output = StoreResult<Option<(SessionId, SessionPhase)>>> + Send;
}

/// F's side of a membership event. One method, a notification: it has no way to cancel, release
/// or reassign anything, so a leave or removal cannot touch an admitted reservation or job.
///
/// TODO(S1-04): the real consumer reads the feed through its own cursor; this port is only the
/// optional push.
pub trait FundingDirectory: Send + Sync + 'static {
    fn membership_event(
        &self,
        campaign: CampaignId,
        event_seq: u64,
    ) -> impl Future<Output = ()> + Send;
}

/// O1's three-part owner fence, as the caller presents it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnerFence {
    pub recovery_epoch: i64,
    pub claim_seq: i64,
    pub placement_id: [u8; 16],
}

impl OwnerFence {
    /// The bytes recorded in `writer_fence` for audit.
    pub fn audit_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32);
        bytes.extend_from_slice(&self.recovery_epoch.to_be_bytes());
        bytes.extend_from_slice(&self.claim_seq.to_be_bytes());
        bytes.extend_from_slice(&self.placement_id);
        bytes
    }
}

/// Who is revoking display tickets. A runtime may reach only the sessions its own placement
/// leases; the control plane may reach any.
///
/// This check is the enforced behaviour today; the runtime's `leased_sessions` come from
/// `df_runtime.leased_table_sessions()`. TODO(S1-05): add row security on `display_ticket` as a
/// second layer in a migration after O1's 0015.
#[derive(Clone, Debug)]
pub enum RevocationCaller {
    ControlPlane,
    Runtime { leased_sessions: Vec<SessionId> },
}

impl RevocationCaller {
    pub fn may_revoke(&self, session: SessionId) -> bool {
        match self {
            Self::ControlPlane => true,
            Self::Runtime { leased_sessions } => leased_sessions.contains(&session),
        }
    }
}

// ---- Values the store returns ---------------------------------------------------------------

/// What an invitation preview is built from. The service reads the host's display name from B.
#[derive(Clone, Debug)]
pub struct PreviewFacts {
    pub campaign: CampaignRow,
    pub host_account: AccountId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Accepted {
    pub campaign: CampaignId,
    pub member: MemberId,
    pub rejoined: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Approve(JoinRole),
    Deny,
}

/// What an invitation is created with: its role, life, uses and the digest of its token.
#[derive(Clone, Copy, Debug)]
pub struct NewInvitation {
    pub joins_as: JoinRole,
    pub lifetime_ms: u64,
    pub max_uses: u32,
    pub token_digest: [u8; 32],
}

/// A transfer proposal: the kind, the target member, and what the old host becomes (host kind).
#[derive(Clone, Copy, Debug)]
pub struct NewProposal {
    pub kind: TransferKind,
    pub to_member: MemberId,
    pub old_host_becomes: Option<OldHostBecomes>,
}

#[derive(Clone, Debug)]
pub struct NewPairing {
    pub device_digest: [u8; 32],
    pub device_label: DeviceLabel,
    /// The tight (IPv6 /64, IPv4 address) and loose (/48, /24) keyed address hashes.
    pub ip_key: [u8; 32],
    pub ip_key_loose: [u8; 32],
    /// The eight canonical symbols of the pairing code.
    pub user_code: String,
}

#[derive(Clone, Debug)]
pub struct Redeemed {
    pub ticket: TicketRow,
    pub reissued: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RetentionReport {
    pub deletions_started: u32,
    pub pairings_expired: u32,
}

/// The values a ticket verification returns.
#[derive(Clone, Debug)]
pub struct VerifiedTicket {
    pub ticket: TicketRow,
}

/// An exclusive keyset position for the discovery hint read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChangeKey {
    pub last_event_at_ms: u64,
    pub campaign: CampaignId,
}

// ---- The store ------------------------------------------------------------------------------

pub trait CampaignStore: Send + Sync + 'static {
    // Feed and snapshot reads.
    fn member_snapshot(
        &self,
        account: AccountId,
        campaign: CampaignId,
    ) -> impl Future<Output = StoreResult<Option<MemberSnapshot>>> + Send;

    fn campaign_heads(
        &self,
        campaigns: &[CampaignId],
    ) -> impl Future<Output = StoreResult<Vec<CampaignHead>>> + Send;

    fn campaign_heads_after(
        &self,
        after: Option<CampaignId>,
        limit: u32,
    ) -> impl Future<Output = StoreResult<Vec<CampaignHead>>> + Send;

    fn campaign_changes_since(
        &self,
        lower_bound_ms: u64,
        after: Option<ChangeKey>,
        limit: u32,
    ) -> impl Future<Output = StoreResult<Vec<CampaignChange>>> + Send;

    fn events_after(
        &self,
        campaign: CampaignId,
        after_seq: u64,
        limit: u32,
    ) -> impl Future<Output = StoreResult<Vec<EventRow>>> + Send;

    /// The primary's `statement_timestamp()` in milliseconds, for the poll anchor.
    fn database_time_ms(&self) -> impl Future<Output = StoreResult<u64>> + Send;

    /// Campaign counts by status, for the dev view.
    fn campaign_counts(&self) -> impl Future<Output = StoreResult<Vec<(String, u64)>>> + Send;

    // Campaigns.
    fn create_campaign(
        &self,
        new: NewCampaign,
        operation: OperationId,
        fingerprint: [u8; 32],
        now_ms: u64,
    ) -> impl Future<Output = Outcome<CampaignRow>> + Send;

    fn archive_campaign(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        typed_name: String,
        session_past_lobby: bool,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    fn unarchive_campaign(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    /// The owner, with the typed name, marks the campaign `deleting` (O2 purges from the event).
    fn request_delete(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        typed_name: String,
        session_past_lobby: bool,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    /// Flips archived campaigns whose scheduled deletion date has passed (5.1, 6).
    fn execute_scheduled_deletions(
        &self,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<RetentionReport>> + Send;

    // Invitations and room codes.
    fn create_invitation(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        invitation: NewInvitation,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<InvitationRow>> + Send;

    fn revoke_invitation(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        invitation: InvitationId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    fn preview_invitation(
        &self,
        token_digest: [u8; 32],
        now_ms: u64,
    ) -> impl Future<Output = Outcome<PreviewFacts>> + Send;

    fn accept_invitation(
        &self,
        account: AccountId,
        token_digest: [u8; 32],
        operation: OperationId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<Accepted>> + Send;

    fn rotate_code(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        code_digest: [u8; 32],
        lifetime_ms: u64,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<CodeRow>> + Send;

    fn lookup_code(
        &self,
        account: AccountId,
        code_digest: [u8; 32],
        now_ms: u64,
    ) -> impl Future<Output = Outcome<PreviewFacts>> + Send;

    fn request_join(
        &self,
        account: AccountId,
        code_digest: [u8; 32],
        now_ms: u64,
    ) -> impl Future<Output = Outcome<RequestRow>> + Send;

    fn decide_request(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        request: RequestId,
        decision: Decision,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<Option<MemberRow>>> + Send;

    // Members.
    fn remove_member(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    fn leave_campaign(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    fn allow_rejoin(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    /// Promotes a seated member to co-host, or sets a co-host's delegations.
    fn set_co_host(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        delegations: Delegations,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    fn demote_co_host(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    /// The host or a co-host steps out of or back into their own seat (a self route).
    fn set_seat(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        seated: bool,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    /// Changes a player to a contributor or a contributor to a player, within the limits.
    fn set_player_role(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        member: MemberId,
        role: JoinRole,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    fn propose_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: NewProposal,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<ProposalId>> + Send;

    fn accept_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: ProposalId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<TransferKind>> + Send;

    fn cancel_transfer(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        proposal: ProposalId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    // Display pairing.
    fn begin_pairing(
        &self,
        new: NewPairing,
        lifetime_ms: u64,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<PairingRow>> + Send;

    fn pairing_status(
        &self,
        device_digest: [u8; 32],
        now_ms: u64,
    ) -> impl Future<Output = Outcome<PairingRow>> + Send;

    fn lookup_pairing(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        user_code: String,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<PairingRow>> + Send;

    fn approve_pairing(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        session: SessionId,
        pairing: PairingId,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;

    /// `presented_ticket` is the digest of the ticket cookie the display sent, if any.
    fn redeem_pairing(
        &self,
        device_digest: [u8; 32],
        presented_ticket: Option<[u8; 32]>,
        new_ticket_digest: [u8; 32],
        session_usable: bool,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<Redeemed>> + Send;

    fn verify_ticket(
        &self,
        ticket_digest: [u8; 32],
        device_digest: [u8; 32],
        session_usable: bool,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<VerifiedTicket>> + Send;

    fn extend_ticket(
        &self,
        ticket_digest: [u8; 32],
        device_digest: [u8; 32],
        session_usable: bool,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<TicketRow>> + Send;

    /// Unpairs one ticket, or every ticket of the session when `ticket` is `None`.
    fn unpair(
        &self,
        actor: AccountId,
        campaign: CampaignId,
        session: SessionId,
        ticket: Option<TicketId>,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<u32>> + Send;

    /// Revokes every live ticket of a session for `reason` (session end, placement release). The
    /// caller's scope has already been checked; `writer_fence` is recorded for audit only.
    fn revoke_session_tickets(
        &self,
        session: SessionId,
        campaign: CampaignId,
        reason: RevokeReason,
        writer_fence: Option<OwnerFence>,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<u32>> + Send;

    /// O1's one state-setting write: `enter_service()`, then the fence, then the campaign lock.
    fn session_started(
        &self,
        campaign: CampaignId,
        fence: OwnerFence,
        now_ms: u64,
    ) -> impl Future<Output = Outcome<()>> + Send;
}
