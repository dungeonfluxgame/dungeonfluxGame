//! Host and ownership transfers (J08, 8.6). A host transfer swaps roles and never touches the
//! owner; an ownership transfer changes only the owner. Acceptance follows the lock order: the
//! target's account share lock, then the campaign lock, then re-verification.

use df_campaign::{
    config::Tunables,
    errors::{Outcome, Refusal, StoreError},
    ids::ProposalId,
    records::{
        CampaignRow, EventKind, MemberRow, MemberStatus, NewEvent, OldHostBecomes, ProposalRow,
        ProposalState, RevokeReason, TransferKind,
    },
    roles::{CampaignRole, Delegations},
    secrets::random_identity,
    store::NewProposal,
};
use df_types::{AccountId, CampaignId};

use super::{
    conn::Tx,
    queries as q,
    shared::{
        Standing, active_actor, drop_authority, emit, locked_campaign, refuse, require_active,
        revoke_tickets, save_campaign, save_member,
    },
};

/// A host target is an active seated player or any active co-host; an ownership target is an
/// active seated member or co-host. A contributor is neither.
fn eligible(kind: TransferKind, target: &MemberRow) -> bool {
    if target.status != MemberStatus::Active {
        return false;
    }
    match kind {
        TransferKind::Host => {
            target.role == CampaignRole::CoHost
                || (target.role == CampaignRole::Player && target.holds_seat)
        }
        TransferKind::Ownership => target.holds_seat || target.role == CampaignRole::CoHost,
    }
}

pub(super) async fn propose_transfer(
    tx: &Tx<'_>,
    tunables: &Tunables,
    actor: AccountId,
    campaign: CampaignId,
    proposal: NewProposal,
    now_ms: u64,
) -> Outcome<ProposalId> {
    let NewProposal {
        kind,
        to_member,
        old_host_becomes,
    } = proposal;
    let row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    require_active(&row)?;
    match kind {
        TransferKind::Host if acting.role != CampaignRole::Host => {
            return refuse(Refusal::NotAllowed);
        }
        TransferKind::Ownership if row.owner != actor => return refuse(Refusal::OwnerOnly),
        _ => {}
    }
    let Some(target) = q::member_by_id(tx, campaign, to_member).await? else {
        return refuse(Refusal::NotFound);
    };
    if target.status != MemberStatus::Active {
        return refuse(Refusal::NotFound);
    }
    if target.member == acting.member {
        return refuse(Refusal::InvalidInput);
    }
    if !eligible(kind, &target) {
        return refuse(Refusal::TransferTargetNotAllowed);
    }
    if !q::account_status_view(tx, target.account)
        .await?
        .is_active()
    {
        return refuse(Refusal::AccountClosing);
    }
    let becomes = match kind {
        TransferKind::Host => {
            let becomes = old_host_becomes.unwrap_or(OldHostBecomes::Player);
            // A seatless player cannot exist.
            if becomes == OldHostBecomes::Player && !acting.holds_seat {
                return refuse(Refusal::InvalidInput);
            }
            Some(becomes)
        }
        TransferKind::Ownership => None,
    };
    q::cancel_proposals_in(tx, campaign, Some(kind), None).await?;
    let id_bytes = random_identity().map_err(|_| StoreError::RandomUnavailable)?;
    let proposal = ProposalRow {
        id: ProposalId::from_bytes(&id_bytes).ok_or(StoreError::InvalidRecord)?,
        campaign,
        kind,
        from_member: acting.member,
        from_account: actor,
        to_member,
        to_account: target.account,
        old_host_becomes: becomes,
        campaign_revision: row.revision,
        proposed_at_ms: now_ms,
        expires_at_ms: now_ms.saturating_add(tunables.transfer_expiry_ms()),
        state: ProposalState::Proposed,
    };
    q::insert_proposal(tx, &proposal).await?;
    Ok(proposal.id)
}

pub(super) async fn accept_transfer(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    proposal: ProposalId,
    now_ms: u64,
) -> Outcome<TransferKind> {
    let Some(first) = q::proposal_by_id(tx, campaign, proposal).await? else {
        return refuse(Refusal::NotFound);
    };
    if first.to_account != actor {
        return refuse(Refusal::NotFound);
    }
    // The account share lock first: it serializes with B's close transaction.
    let account = q::lock_account(tx, actor).await?;
    let mut row = locked_campaign(tx, campaign).await?;
    let Some(pending) = q::proposal_by_id(tx, campaign, proposal).await? else {
        return refuse(Refusal::NotFound);
    };
    if pending.state != ProposalState::Proposed || row.revision != pending.campaign_revision {
        return refuse(Refusal::StaleRevision);
    }
    if !account.is_active() {
        return refuse(Refusal::AccountClosing);
    }
    if now_ms >= pending.expires_at_ms {
        return refuse(Refusal::ProposalExpired);
    }
    require_active(&row)?;
    if pending.kind == TransferKind::Ownership && row.scheduled_delete_at_ms.is_some() {
        return refuse(Refusal::DeletionScheduled);
    }
    let Some(mut target) = q::member_by_id(tx, campaign, pending.to_member).await? else {
        return refuse(Refusal::StaleRevision);
    };
    if target.account != actor || !eligible(pending.kind, &target) {
        return refuse(Refusal::TransferTargetNotAllowed);
    }
    let Some(mut old) = q::member_by_id(tx, campaign, pending.from_member).await? else {
        return refuse(Refusal::StaleRevision);
    };
    if old.status != MemberStatus::Active {
        return refuse(Refusal::StaleRevision);
    }
    match pending.kind {
        TransferKind::Host => {
            host_swap(
                tx,
                &row,
                &mut old,
                &mut target,
                pending.old_host_becomes,
                now_ms,
            )
            .await?;
        }
        TransferKind::Ownership => {
            if row.owner != pending.from_account {
                return refuse(Refusal::StaleRevision);
            }
            row.owner = actor;
            save_member(tx, &mut old).await?;
            save_member(tx, &mut target).await?;
            let by = Some(target.member);
            emit(
                tx,
                campaign,
                NewEvent::member(
                    EventKind::OwnershipTransferred,
                    old.member,
                    old.revision,
                    true,
                    by,
                ),
                now_ms,
            )
            .await?;
            emit(
                tx,
                campaign,
                NewEvent::member(
                    EventKind::OwnershipTransferred,
                    target.member,
                    target.revision,
                    false,
                    by,
                ),
                now_ms,
            )
            .await?;
        }
    }
    save_campaign(tx, &mut row).await?;
    q::set_proposal_state(tx, proposal, ProposalState::Accepted).await?;
    q::cancel_proposals_in(tx, campaign, None, Some(proposal)).await?;
    Ok(pending.kind)
}

/// Demote the old host first (the unique index is not deferrable), then promote the target. Two
/// member rows are appended, revoking for the old host; every display ticket of the campaign is
/// revoked and the displaced host's links are handled.
pub(super) async fn host_swap(
    tx: &Tx<'_>,
    campaign: &CampaignRow,
    old: &mut MemberRow,
    target: &mut MemberRow,
    becomes: Option<OldHostBecomes>,
    now_ms: u64,
) -> Outcome<()> {
    if old.role != CampaignRole::Host {
        return refuse(Refusal::StaleRevision);
    }
    let before = Standing::of(old.role, old.delegations);
    match becomes.unwrap_or(OldHostBecomes::Player) {
        OldHostBecomes::Player => {
            if !old.holds_seat {
                return refuse(Refusal::StaleRevision);
            }
            old.role = CampaignRole::Player;
            old.delegations = Delegations::NONE;
        }
        OldHostBecomes::CoHost => {
            old.role = CampaignRole::CoHost;
            old.delegations = campaign.default_co_host_delegations;
        }
    }
    save_member(tx, old).await?;
    target.role = CampaignRole::Host;
    target.delegations = Delegations::NONE;
    save_member(tx, target).await?;
    let by = Some(target.member);
    emit(
        tx,
        campaign.id,
        NewEvent::member(
            EventKind::HostTransferred,
            old.member,
            old.revision,
            true,
            by,
        ),
        now_ms,
    )
    .await?;
    emit(
        tx,
        campaign.id,
        NewEvent::member(
            EventKind::HostTransferred,
            target.member,
            target.revision,
            false,
            by,
        ),
        now_ms,
    )
    .await?;
    drop_authority(
        tx,
        campaign.id,
        old.member,
        before,
        Standing::of(old.role, old.delegations),
        by,
        now_ms,
    )
    .await?;
    let tickets = q::live_tickets_of_campaign(tx, campaign.id, now_ms).await?;
    revoke_tickets(
        tx,
        campaign.id,
        &tickets,
        RevokeReason::HostTransferred,
        by,
        None,
        now_ms,
    )
    .await?;
    q::cancel_approved_pairings(tx, campaign.id, None).await?;
    Ok(())
}

pub(super) async fn cancel_transfer(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    proposal: ProposalId,
) -> Outcome<()> {
    locked_campaign(tx, campaign).await?;
    let Some(row) = q::proposal_by_id(tx, campaign, proposal).await? else {
        return refuse(Refusal::NotFound);
    };
    // The proposer withdraws it; the target declines it. Anyone else sees a missing proposal.
    if row.from_account != actor && row.to_account != actor {
        return refuse(Refusal::NotFound);
    }
    if row.state != ProposalState::Proposed {
        return refuse(Refusal::StaleRevision);
    }
    q::set_proposal_state(tx, proposal, ProposalState::Cancelled).await?;
    Ok(())
}
