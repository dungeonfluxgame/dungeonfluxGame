//! Leaving, removal, readmission, co-host delegations and seats (4.1, 8.6). Every change here takes
//! the campaign lock, authorizes the actor from the member row read under it, and appends its
//! events in the same transaction.

use df_campaign::{
    config::Tunables,
    errors::{Outcome, Refusal},
    join_invitation::JoinRole,
    records::{CampaignStatus, EventKind, MemberStatus, NewEvent},
    roles::{CampaignRole, Delegations, RemoveRefusal, may_allow_rejoin, may_remove},
};
use df_types::{AccountId, CampaignId, MemberId};

use super::{
    conn::Tx,
    queries as q,
    shared::{
        Standing, active_actor, drop_authority, emit, end_membership, locked_campaign, refuse,
        require_active, save_member,
    },
};

pub(super) async fn remove_member(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    member: MemberId,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    require_active(&row)?;
    let Some(mut target) = q::member_by_id(tx, campaign, member).await? else {
        return refuse(Refusal::NotFound);
    };
    if target.status != MemberStatus::Active {
        return refuse(Refusal::NotFound);
    }
    match may_remove(
        acting.role,
        acting.delegations,
        target.role,
        target.account == row.owner,
    ) {
        Ok(()) => {}
        Err(RemoveRefusal::NotAllowed) => return refuse(Refusal::NotAllowed),
        Err(RemoveRefusal::TargetIsHost) => return refuse(Refusal::CannotRemoveHost),
        Err(RemoveRefusal::TargetIsOwner) => return refuse(Refusal::CannotRemoveOwner),
        Err(RemoveRefusal::TargetOutranks) => return refuse(Refusal::TargetOutranks),
    }
    end_membership(
        tx,
        campaign,
        &mut target,
        MemberStatus::Removed,
        Some(acting.role),
        Some(acting.member),
        now_ms,
    )
    .await
}

pub(super) async fn leave_campaign(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let mut acting = active_actor(tx, campaign, actor).await?;
    if row.status == CampaignStatus::Deleting {
        return refuse(Refusal::AlreadyDeleting);
    }
    if acting.role == CampaignRole::Host {
        return refuse(Refusal::HostMustTransferFirst);
    }
    if acting.account == row.owner {
        return refuse(Refusal::OwnerMustTransferFirst);
    }
    let own = Some(acting.member);
    end_membership(
        tx,
        campaign,
        &mut acting,
        MemberStatus::Left,
        None,
        own,
        now_ms,
    )
    .await
}

pub(super) async fn allow_rejoin(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    member: MemberId,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    require_active(&row)?;
    let Some(mut target) = q::member_by_id(tx, campaign, member).await? else {
        return refuse(Refusal::NotFound);
    };
    if target.status != MemberStatus::Removed || target.may_rejoin {
        return refuse(Refusal::NotFound);
    }
    let Some(removed_by) = target.removed_by_role else {
        return refuse(Refusal::NotFound);
    };
    if !may_allow_rejoin(acting.role, acting.delegations, removed_by) {
        return refuse(Refusal::NotAllowed);
    }
    target.may_rejoin = true;
    save_member(tx, &mut target).await?;
    emit(
        tx,
        campaign,
        NewEvent::member(
            EventKind::RejoinAllowed,
            target.member,
            target.revision,
            false,
            Some(acting.member),
        ),
        now_ms,
    )
    .await?;
    Ok(())
}

/// Promotes a seated player to co-host, or sets a co-host's delegations. Only the host does it.
pub(super) async fn set_co_host(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    member: MemberId,
    delegations: Delegations,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    if acting.role != CampaignRole::Host {
        return refuse(Refusal::NotAllowed);
    }
    require_active(&row)?;
    let Some(mut target) = q::member_by_id(tx, campaign, member).await? else {
        return refuse(Refusal::NotFound);
    };
    if target.status != MemberStatus::Active {
        return refuse(Refusal::NotFound);
    }
    let before = Standing::of(target.role, target.delegations);
    let (kind, revoking) = match target.role {
        CampaignRole::Player => {
            if !target.holds_seat {
                return refuse(Refusal::NotSeated);
            }
            (EventKind::RoleChanged, false)
        }
        CampaignRole::CoHost => {
            let cleared = target.delegations.bits() & !delegations.bits() != 0;
            (EventKind::DelegationsChanged, cleared)
        }
        CampaignRole::Contributor => return refuse(Refusal::ContributorCannotBeCoHost),
        CampaignRole::Host | CampaignRole::PairedDisplay => return refuse(Refusal::NotAllowed),
    };
    target.role = CampaignRole::CoHost;
    target.delegations = delegations;
    save_member(tx, &mut target).await?;
    drop_authority(
        tx,
        campaign,
        target.member,
        before,
        Standing::of(target.role, target.delegations),
        Some(acting.member),
        now_ms,
    )
    .await?;
    emit(
        tx,
        campaign,
        NewEvent::member(
            kind,
            target.member,
            target.revision,
            revoking,
            Some(acting.member),
        ),
        now_ms,
    )
    .await?;
    Ok(())
}

pub(super) async fn demote_co_host(
    tx: &Tx<'_>,
    tunables: &Tunables,
    actor: AccountId,
    campaign: CampaignId,
    member: MemberId,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    if acting.role != CampaignRole::Host {
        return refuse(Refusal::NotAllowed);
    }
    require_active(&row)?;
    let Some(mut target) = q::member_by_id(tx, campaign, member).await? else {
        return refuse(Refusal::NotFound);
    };
    if target.status != MemberStatus::Active || target.role != CampaignRole::CoHost {
        return refuse(Refusal::NotFound);
    }
    let before = Standing::of(target.role, target.delegations);
    // A seatless co-host cannot become a player: a seatless non-host is a contributor.
    if target.holds_seat {
        target.role = CampaignRole::Player;
    } else {
        let counts = q::seat_counts(tx, campaign).await?;
        if counts.contributors_in_use >= tunables.maximum_contributors {
            return refuse(Refusal::ContributorsFull);
        }
        target.role = CampaignRole::Contributor;
    }
    target.delegations = Delegations::NONE;
    save_member(tx, &mut target).await?;
    drop_authority(
        tx,
        campaign,
        target.member,
        before,
        Standing::NONE,
        Some(acting.member),
        now_ms,
    )
    .await?;
    emit(
        tx,
        campaign,
        NewEvent::member(
            EventKind::RoleChanged,
            target.member,
            target.revision,
            true,
            Some(acting.member),
        ),
        now_ms,
    )
    .await?;
    Ok(())
}

/// The host or a co-host steps out of or back into their own seat. Players and contributors
/// cannot use this route: their seat is a role change.
pub(super) async fn set_seat(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    seated: bool,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let mut acting = active_actor(tx, campaign, actor).await?;
    require_active(&row)?;
    if !acting.role.may_be_seatless() {
        return refuse(Refusal::NotAllowed);
    }
    if acting.holds_seat == seated {
        return Ok(());
    }
    if seated {
        let counts = q::seat_counts(tx, campaign).await?;
        if counts.seats_in_use >= row.seat_count {
            return refuse(Refusal::SeatsFull);
        }
    } else {
        q::clear_lobby_rows(tx, campaign, acting.member).await?;
    }
    acting.holds_seat = seated;
    save_member(tx, &mut acting).await?;
    emit(
        tx,
        campaign,
        NewEvent::member(
            EventKind::SeatChanged,
            acting.member,
            acting.revision,
            !seated,
            None,
        ),
        now_ms,
    )
    .await?;
    Ok(())
}

/// Host only: a co-host with the invite bit cannot move people between seats. Changes a player to a
/// contributor (freeing the seat) or a contributor to a player (taking
/// one), bounded by the contributor limit and the seat count under the campaign lock.
pub(super) async fn set_player_role(
    tx: &Tx<'_>,
    tunables: &Tunables,
    actor: AccountId,
    campaign: CampaignId,
    member: MemberId,
    role: JoinRole,
    now_ms: u64,
) -> Outcome<()> {
    let row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    if acting.role != CampaignRole::Host {
        return refuse(Refusal::NotAllowed);
    }
    require_active(&row)?;
    let Some(mut target) = q::member_by_id(tx, campaign, member).await? else {
        return refuse(Refusal::NotFound);
    };
    if target.status != MemberStatus::Active
        || !matches!(
            target.role,
            CampaignRole::Player | CampaignRole::Contributor
        )
    {
        return refuse(Refusal::NotFound);
    }
    let counts = q::seat_counts(tx, campaign).await?;
    let revoking = match (target.role, role) {
        (CampaignRole::Player, JoinRole::Player)
        | (CampaignRole::Contributor, JoinRole::Contributor) => {
            return Ok(());
        }
        (_, JoinRole::Contributor) => {
            if counts.contributors_in_use >= tunables.maximum_contributors {
                return refuse(Refusal::ContributorsFull);
            }
            target.role = CampaignRole::Contributor;
            target.holds_seat = false;
            q::clear_lobby_rows(tx, campaign, target.member).await?;
            true
        }
        (_, JoinRole::Player) => {
            if counts.seats_in_use >= row.seat_count {
                return refuse(Refusal::SeatsFull);
            }
            target.role = CampaignRole::Player;
            target.holds_seat = true;
            false
        }
    };
    save_member(tx, &mut target).await?;
    emit(
        tx,
        campaign,
        NewEvent::member(
            EventKind::RoleChanged,
            target.member,
            target.revision,
            revoking,
            Some(acting.member),
        ),
        now_ms,
    )
    .await?;
    Ok(())
}
