//! Rules shared by the store's operations: the locked campaign, the acting member, admission of a
//! member, loss of authority and event writing. Everything here runs inside a transaction the
//! caller opened and, where it matters, locked in the order of launch decision 29.

use df_campaign::{
    config::Tunables,
    errors::{Failure, Outcome, Refusal, StoreResult},
    join_invitation::{JoinContext, JoinUse, MemberState},
    records::{
        AccountStatus, CampaignRow, CampaignStatus, EventKind, MemberRow, MemberStatus, NewEvent,
        RevokeReason, TicketRow,
    },
    roles::{Authorized, CampaignRole, Capability, Delegations, authorize},
    secrets::random_identity,
};
use df_types::{AccountId, CampaignId, MemberId};

use super::{conn::Tx, queries as q};

pub(super) fn refuse<T>(refusal: Refusal) -> Outcome<T> {
    Err(Failure::Refused(refusal))
}

/// A fresh member id from the random source.
pub(super) fn new_member_id() -> Outcome<MemberId> {
    let bytes =
        random_identity().map_err(|_| df_campaign::errors::StoreError::RandomUnavailable)?;
    MemberId::from_bytes(&bytes)
        .map_err(|_| Failure::Store(df_campaign::errors::StoreError::InvalidRecord))
}

/// Locks the campaign row. A missing campaign is `NotFound`, like a foreign one.
pub(super) async fn locked_campaign(tx: &Tx<'_>, campaign: CampaignId) -> Outcome<CampaignRow> {
    match q::lock_campaign(tx, campaign).await? {
        Some(row) => Ok(row),
        None => refuse(Refusal::NotFound),
    }
}

/// The refusal for a campaign that cannot take this change.
pub(super) fn require_active(campaign: &CampaignRow) -> Outcome<()> {
    match campaign.status {
        CampaignStatus::Active => Ok(()),
        CampaignStatus::Deleting => refuse(Refusal::AlreadyDeleting),
        CampaignStatus::Draft | CampaignStatus::Archived => refuse(Refusal::CampaignUnavailable),
    }
}

/// The actor's active member row. A non-member and a former member answer as `NotFound`.
pub(super) async fn active_actor(
    tx: &Tx<'_>,
    campaign: CampaignId,
    account: AccountId,
) -> Outcome<MemberRow> {
    match q::member_by_account(tx, campaign, account).await? {
        Some(row) if row.status == MemberStatus::Active => Ok(row),
        _ => refuse(Refusal::NotFound),
    }
}

/// The role matrix, applied to the member row read under the lock.
pub(super) fn require_capability(member: &MemberRow, capability: Capability) -> Outcome<()> {
    // Only a full grant authorizes the action: a request-only or own-only cell does not.
    match authorize(member.role, member.delegations, capability) {
        Ok(Authorized::Full) => Ok(()),
        Ok(_) | Err(_) => refuse(Refusal::NotAllowed),
    }
}

/// Writes a member whose fields the caller changed, bumping its revision.
pub(super) async fn save_member(tx: &Tx<'_>, row: &mut MemberRow) -> Outcome<()> {
    row.revision += 1;
    if q::update_member(tx, row).await? {
        Ok(())
    } else {
        refuse(Refusal::StaleRevision)
    }
}

/// Writes a campaign whose fields the caller changed, bumping its revision.
pub(super) async fn save_campaign(tx: &Tx<'_>, row: &mut CampaignRow) -> Outcome<()> {
    row.revision += 1;
    if q::update_campaign(tx, row).await? {
        Ok(())
    } else {
        refuse(Refusal::StaleRevision)
    }
}

pub(super) fn member_state(existing: Option<&MemberRow>) -> MemberState {
    match existing {
        None => MemberState::NotAMember,
        Some(row) => match row.status {
            MemberStatus::Active => MemberState::Active,
            MemberStatus::Left => MemberState::Left,
            MemberStatus::Removed if row.may_rejoin => MemberState::RemovedMayRejoin,
            MemberStatus::Removed => MemberState::RemovedNotAllowed,
        },
    }
}

/// Seats and contributor slots free right now, counted under the campaign lock.
pub(super) async fn join_context(
    tx: &Tx<'_>,
    campaign: &CampaignRow,
    existing: Option<&MemberRow>,
    account: AccountStatus,
    tunables: &Tunables,
    now_ms: u64,
) -> StoreResult<JoinContext> {
    let counts = q::seat_counts(tx, campaign.id).await?;
    Ok(JoinContext {
        member: member_state(existing),
        campaign_active: campaign.status == CampaignStatus::Active,
        account_active: account.is_active(),
        seats_free: campaign.seat_count.saturating_sub(counts.seats_in_use),
        contributors_free: tunables
            .maximum_contributors
            .saturating_sub(counts.contributors_in_use),
        now_ms,
    })
}

/// Appends an event for a campaign whose row the caller holds locked.
pub(super) async fn emit(
    tx: &Tx<'_>,
    campaign: CampaignId,
    event: NewEvent,
    now_ms: u64,
) -> StoreResult<u64> {
    q::append_event(tx, campaign, &event, now_ms).await
}

pub(super) struct Admitted {
    pub member: MemberRow,
    pub rejoined: bool,
}

/// Inserts a new member, or reactivates the same row and id of a returning one (revision plus
/// one, role from the use, delegations cleared, `may_rejoin` cleared), and appends `joined` or
/// `rejoined`. The caller has taken the account lock and the campaign lock and re-verified.
pub(super) async fn admit(
    tx: &Tx<'_>,
    campaign: &CampaignRow,
    account: AccountId,
    existing: Option<MemberRow>,
    admission: JoinUse,
    invitation: Option<df_campaign::ids::InvitationId>,
    now_ms: u64,
) -> Outcome<Admitted> {
    use df_campaign::join_invitation::JoinRole;
    let (role, holds_seat) = match admission.joins_as {
        JoinRole::Player => (CampaignRole::Player, true),
        JoinRole::Contributor => (CampaignRole::Contributor, false),
    };
    let (member, rejoined) = match existing {
        Some(mut row) => {
            row.role = role;
            row.holds_seat = holds_seat;
            row.delegations = Delegations::NONE;
            row.status = MemberStatus::Active;
            row.may_rejoin = false;
            row.removed_by_role = None;
            row.last_invitation = invitation;
            row.joined_at_ms = now_ms;
            row.ended_at_ms = None;
            save_member(tx, &mut row).await?;
            (row, true)
        }
        None => {
            let row = MemberRow {
                member: new_member_id()?,
                campaign: campaign.id,
                account,
                role,
                holds_seat,
                delegations: Delegations::NONE,
                status: MemberStatus::Active,
                may_rejoin: false,
                removed_by_role: None,
                last_invitation: invitation,
                joined_at_ms: now_ms,
                ended_at_ms: None,
                revision: 1,
            };
            q::insert_member(tx, &row).await?;
            (row, false)
        }
    };
    let kind = if rejoined {
        EventKind::Rejoined
    } else {
        EventKind::Joined
    };
    emit(
        tx,
        campaign.id,
        NewEvent::member(kind, member.member, member.revision, false, None),
        now_ms,
    )
    .await?;
    Ok(Admitted { member, rejoined })
}

/// What a member's role and delegations let them do about links and displays.
#[derive(Clone, Copy, Debug)]
pub(super) struct Standing {
    pub invites: bool,
    pub pairs: bool,
}

impl Standing {
    pub(super) fn of(role: CampaignRole, delegations: Delegations) -> Self {
        Self {
            invites: authorize(role, delegations, Capability::InviteOrRemovePlayers)
                == Ok(Authorized::Full),
            pairs: authorize(role, delegations, Capability::StartOrPauseSession)
                == Ok(Authorized::Full),
        }
    }

    pub(super) const NONE: Self = Self {
        invites: false,
        pairs: false,
    };
}

/// Links outlive neither their creator's right nor their seat (8.6): when a member loses the
/// right to invite, every unexpired invitation they created is revoked and the live room code is
/// revoked, with one `invitations_revoked` event. Display tickets they approved are revoked when
/// they lose the right to pair. Done in the transaction of the change that lowered their standing.
pub(super) async fn drop_authority(
    tx: &Tx<'_>,
    campaign: CampaignId,
    member: MemberId,
    before: Standing,
    after: Standing,
    actor: Option<MemberId>,
    now_ms: u64,
) -> Outcome<()> {
    if before.invites && !after.invites {
        let invitations = q::revoke_invitations_of(tx, campaign, member, now_ms).await?;
        let codes = q::revoke_live_code(tx, campaign, now_ms).await?;
        if invitations + codes > 0 {
            df_campaign::audit::AuditLine::new("INVITE", "revoked_with_creator")
                .campaign(campaign)
                .member(member)
                .reason("lost_invite_right")
                .emit("invite.revoked_with_creator");
            emit(
                tx,
                campaign,
                NewEvent::campaign(EventKind::InvitationsRevoked, actor),
                now_ms,
            )
            .await?;
        }
    }
    if before.pairs && !after.pairs {
        let tickets = q::live_tickets_approved_by(tx, campaign, member, now_ms).await?;
        revoke_tickets(
            tx,
            campaign,
            &tickets,
            RevokeReason::ApproverAuthorityLost,
            actor,
            None,
            now_ms,
        )
        .await?;
    }
    Ok(())
}

/// Revokes each ticket and appends one `display_ticket_revoked` row per ticket that was live.
pub(super) async fn revoke_tickets(
    tx: &Tx<'_>,
    campaign: CampaignId,
    tickets: &[TicketRow],
    reason: RevokeReason,
    actor: Option<MemberId>,
    writer_fence: Option<Vec<u8>>,
    now_ms: u64,
) -> StoreResult<u32> {
    let mut revoked = 0;
    for ticket in tickets {
        if q::revoke_ticket_row(tx, ticket.id, reason, now_ms).await? {
            let mut event = NewEvent::ticket(ticket.id, actor);
            event.writer_fence = writer_fence.clone();
            emit(tx, campaign, event, now_ms).await?;
            revoked += 1;
        }
    }
    Ok(revoked)
}

/// Ends a member's participation (left or removed): frees the seat, clears their open-session
/// lobby rows and drops their authority, then appends the revoking event with the new revision.
pub(super) async fn end_membership(
    tx: &Tx<'_>,
    campaign: CampaignId,
    member: &mut MemberRow,
    status: MemberStatus,
    removed_by: Option<CampaignRole>,
    actor: Option<MemberId>,
    now_ms: u64,
) -> Outcome<()> {
    let before = Standing::of(member.role, member.delegations);
    member.status = status;
    member.holds_seat = false;
    member.may_rejoin = false;
    member.removed_by_role = removed_by;
    member.ended_at_ms = Some(now_ms);
    save_member(tx, member).await?;
    q::clear_lobby_rows(tx, campaign, member.member).await?;
    drop_authority(
        tx,
        campaign,
        member.member,
        before,
        Standing::NONE,
        actor,
        now_ms,
    )
    .await?;
    let kind = match status {
        MemberStatus::Removed => EventKind::Removed,
        _ => EventKind::Left,
    };
    emit(
        tx,
        campaign,
        NewEvent::member(kind, member.member, member.revision, true, actor),
        now_ms,
    )
    .await?;
    Ok(())
}
