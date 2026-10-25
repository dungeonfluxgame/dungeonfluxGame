//! Creating, archiving, unarchiving and deleting campaigns, and the fenced `session_started`
//! write (8.2, 8.7, 6).

use df_campaign::{
    config::Tunables,
    errors::{Failure, Outcome, Refusal, StoreError},
    ids::FeedEpoch,
    records::{
        AccountStatus, CampaignRow, CampaignStatus, EventKind, MemberRow, MemberStatus,
        NewCampaign, NewEvent, RevokeReason,
    },
    roles::{CampaignRole, Delegations, OwnerAction, OwnerRefusal, authorize_owner_action},
    secrets::random_identity,
    store::OwnerFence,
};
use df_types::{AccountId, CampaignId, OperationId, SessionId};

use super::{
    conn::Tx,
    invites::replay,
    queries as q,
    shared::{
        active_actor, emit, locked_campaign, new_member_id, refuse, revoke_tickets, save_campaign,
    },
};

const CREATE_NAMESPACE: &str = "create_campaign";

pub(super) async fn create_campaign(
    tx: &Tx<'_>,
    tunables: &Tunables,
    new: NewCampaign,
    operation: OperationId,
    fingerprint: [u8; 32],
    now_ms: u64,
) -> Outcome<CampaignRow> {
    let length = new.name.chars().count();
    if !(1..=80).contains(&length) || new.seat_count == 0 || new.seat_count > tunables.maximum_seats
    {
        return refuse(Refusal::InvalidInput);
    }
    if let Some(result) = replay(
        tx,
        CREATE_NAMESPACE,
        new.owner,
        operation,
        fingerprint,
        now_ms,
    )
    .await?
    {
        return existing_campaign(tx, &result).await;
    }
    // The creator becomes a member, so the account lock comes first.
    if q::lock_account(tx, new.owner).await? != AccountStatus::Active {
        return refuse(Refusal::AccountClosing);
    }
    // One creation per account at a time from here on: the count below is exact, not a bound that
    // two racing creations can pass together. The replay is read again behind the lock, because
    // the creation it waited for may have been this very operation.
    q::lock_campaign_creation(tx, new.owner).await?;
    if let Some(result) = replay(
        tx,
        CREATE_NAMESPACE,
        new.owner,
        operation,
        fingerprint,
        now_ms,
    )
    .await?
    {
        return existing_campaign(tx, &result).await;
    }
    if q::owned_campaigns_up_to(tx, new.owner, tunables.campaigns_per_account).await?
        >= tunables.campaigns_per_account
    {
        return refuse(Refusal::TooManyCampaigns);
    }
    let id_bytes = random_identity().map_err(|_| StoreError::RandomUnavailable)?;
    let id = CampaignId::from_bytes(&id_bytes).map_err(|_| StoreError::InvalidRecord)?;
    let epoch = FeedEpoch::generate().map_err(|_| StoreError::RandomUnavailable)?;
    let row = CampaignRow {
        id,
        tenant: new.tenant,
        owner: new.owner,
        name: new.name,
        seat_count: new.seat_count,
        content_rating: new.content_rating,
        room_codes: new.room_codes,
        default_co_host_delegations: new.default_co_host_delegations,
        display_summary_enabled: false,
        planned_session_at_ms: None,
        status: CampaignStatus::Active,
        revision: 1,
        event_seq: 0,
        feed_epoch: epoch,
        last_event_at_ms: 0,
        created_at_ms: now_ms,
        archived_at_ms: None,
        scheduled_delete_at_ms: None,
    };
    q::insert_campaign(tx, &row).await?;
    finish_creation(tx, id, new.owner, operation, fingerprint, now_ms).await
}

/// The tail every creation shares: the creator becomes the host and takes a seat, `joined` is
/// appended (the first event, number 1) and the operation receipt is written. The campaign row
/// exists and the caller holds the account lock and the account's creation lock.
pub(super) async fn finish_creation(
    tx: &Tx<'_>,
    id: CampaignId,
    owner: AccountId,
    operation: OperationId,
    fingerprint: [u8; 32],
    now_ms: u64,
) -> Outcome<CampaignRow> {
    let host = MemberRow {
        member: new_member_id()?,
        campaign: id,
        account: owner,
        role: CampaignRole::Host,
        holds_seat: true,
        delegations: Delegations::NONE,
        status: MemberStatus::Active,
        may_rejoin: false,
        removed_by_role: None,
        last_invitation: None,
        joined_at_ms: now_ms,
        ended_at_ms: None,
        revision: 1,
    };
    q::insert_member(tx, &host).await?;
    emit(
        tx,
        id,
        NewEvent::member(EventKind::Joined, host.member, 1, false, None),
        now_ms,
    )
    .await?;
    if !q::insert_operation(
        tx,
        CREATE_NAMESPACE,
        owner,
        operation.as_bytes(),
        fingerprint,
        id.as_bytes(),
        now_ms,
    )
    .await?
    {
        return Err(Failure::Store(StoreError::Retryable));
    }
    match q::read_campaign(tx, id).await? {
        Some(created) => Ok(created),
        None => Err(Failure::Store(StoreError::InvalidRecord)),
    }
}

pub(super) async fn existing_campaign(tx: &Tx<'_>, result: &[u8]) -> Outcome<CampaignRow> {
    let id = CampaignId::from_bytes(result).map_err(|_| StoreError::InvalidRecord)?;
    match q::read_campaign(tx, id).await? {
        Some(row) => Ok(row),
        None => Err(Failure::Store(StoreError::InvalidRecord)),
    }
}

fn owner_check(
    row: &CampaignRow,
    actor: AccountId,
    action: OwnerAction,
    typed_name: &str,
) -> Outcome<()> {
    match authorize_owner_action(row.owner == actor, action, typed_name == row.name) {
        Ok(()) => Ok(()),
        Err(OwnerRefusal::NotOwner) => refuse(Refusal::OwnerOnly),
        Err(OwnerRefusal::ConfirmationRequired) => refuse(Refusal::ConfirmationRequired),
    }
}

/// Revokes every live ticket of the campaign and cancels unredeemed approvals.
async fn close_displays(
    tx: &Tx<'_>,
    campaign: CampaignId,
    reason: RevokeReason,
    actor: Option<df_types::MemberId>,
    now_ms: u64,
) -> Outcome<()> {
    let tickets = q::live_tickets_of_campaign(tx, campaign, now_ms).await?;
    revoke_tickets(tx, campaign, &tickets, reason, actor, None, now_ms).await?;
    q::cancel_approved_pairings(tx, campaign, None).await?;
    Ok(())
}

pub(super) async fn archive_campaign(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    typed_name: &str,
    session_past_lobby: bool,
    now_ms: u64,
) -> Outcome<()> {
    let mut row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    owner_check(&row, actor, OwnerAction::Archive, typed_name)?;
    match row.status {
        CampaignStatus::Active => {}
        CampaignStatus::Deleting => return refuse(Refusal::AlreadyDeleting),
        CampaignStatus::Draft | CampaignStatus::Archived => {
            return refuse(Refusal::CampaignUnavailable);
        }
    }
    if session_past_lobby {
        return refuse(Refusal::SessionInProgress);
    }
    row.status = CampaignStatus::Archived;
    row.archived_at_ms = Some(now_ms);
    save_campaign(tx, &mut row).await?;
    close_displays(
        tx,
        campaign,
        RevokeReason::Archived,
        Some(acting.member),
        now_ms,
    )
    .await?;
    emit(
        tx,
        campaign,
        NewEvent::campaign(EventKind::Archived, Some(acting.member)),
        now_ms,
    )
    .await?;
    Ok(())
}

/// `request_delete`: the owner, with the typed name, marks the campaign `deleting`.
pub(super) async fn request_delete(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    typed_name: &str,
    session_past_lobby: bool,
    now_ms: u64,
) -> Outcome<()> {
    let mut row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    owner_check(&row, actor, OwnerAction::Delete, typed_name)?;
    match row.status {
        CampaignStatus::Active | CampaignStatus::Archived => {}
        CampaignStatus::Deleting => return refuse(Refusal::AlreadyDeleting),
        CampaignStatus::Draft => return refuse(Refusal::CampaignUnavailable),
    }
    if session_past_lobby {
        return refuse(Refusal::SessionInProgress);
    }
    row.status = CampaignStatus::Deleting;
    row.scheduled_delete_at_ms = None;
    save_campaign(tx, &mut row).await?;
    close_displays(
        tx,
        campaign,
        RevokeReason::Archived,
        Some(acting.member),
        now_ms,
    )
    .await?;
    emit(
        tx,
        campaign,
        NewEvent::campaign(EventKind::DeleteRequested, Some(acting.member)),
        now_ms,
    )
    .await?;
    Ok(())
}

/// `unarchive`: refused while the host's or the owner's account is not active. The order is: read
/// both account ids without a lock, take the account share locks in byte order (host and owner
/// share one order with B's close transaction and with each other), lock the campaign, re-verify.
/// The share locks are held until commit, so B's close transaction (which updates the account row
/// first) and this serialize: either the unarchive commits first and the closing account then
/// shows up as a blocker, or the close marks the account `closing` first and this is refused.
pub(super) async fn unarchive_campaign(
    tx: &Tx<'_>,
    actor: AccountId,
    campaign: CampaignId,
    now_ms: u64,
) -> Outcome<()> {
    // A campaign that does not exist and one the actor has no part in answer alike, before any
    // refusal that would show the campaign exists.
    let Some(visible) = q::read_campaign(tx, campaign).await? else {
        return refuse(Refusal::NotFound);
    };
    active_actor(tx, campaign, actor).await?;
    owner_check(&visible, actor, OwnerAction::TransferOwnership, "")?;
    let Some(host_account) = q::host_account(tx, campaign).await? else {
        // No active host: a missing host account counts as not active.
        return refuse(Refusal::HostAccountNotActive);
    };
    let owner_account = visible.owner;
    let mut order = vec![host_account, owner_account];
    order.sort_by_key(|account| *account.as_bytes());
    order.dedup();
    let mut statuses = Vec::with_capacity(order.len());
    for account in order {
        statuses.push((account, q::lock_account(tx, account).await?));
    }
    let status_of = |account: AccountId| {
        statuses
            .iter()
            .find(|(candidate, _)| *candidate == account)
            .map_or(AccountStatus::Missing, |(_, status)| *status)
    };
    let mut row = locked_campaign(tx, campaign).await?;
    let acting = active_actor(tx, campaign, actor).await?;
    owner_check(&row, actor, OwnerAction::TransferOwnership, "")?;
    match row.status {
        CampaignStatus::Archived => {}
        CampaignStatus::Deleting => return refuse(Refusal::AlreadyDeleting),
        CampaignStatus::Active | CampaignStatus::Draft => {
            return refuse(Refusal::CampaignUnavailable);
        }
    }
    if row.scheduled_delete_at_ms.is_some() {
        return refuse(Refusal::DeletionScheduled);
    }
    // Re-verify under the locks: still the same host and owner, both still active.
    if q::host_account(tx, campaign).await? != Some(host_account)
        || !status_of(host_account).is_active()
    {
        return refuse(Refusal::HostAccountNotActive);
    }
    if row.owner != owner_account {
        return refuse(Refusal::StaleRevision);
    }
    if !status_of(owner_account).is_active() {
        return refuse(Refusal::OwnerAccountNotActive);
    }
    row.status = CampaignStatus::Active;
    row.archived_at_ms = None;
    save_campaign(tx, &mut row).await?;
    emit(
        tx,
        campaign,
        NewEvent::campaign(EventKind::Unarchived, Some(acting.member)),
        now_ms,
    )
    .await?;
    Ok(())
}

/// The retention flip for one campaign: under the campaign lock, re-check that the date has
/// passed, that the schedule was not cancelled and that the status is still `archived`.
pub(super) async fn flip_scheduled_deletion(
    tx: &Tx<'_>,
    campaign: CampaignId,
    now_ms: u64,
) -> Outcome<bool> {
    let mut row = locked_campaign(tx, campaign).await?;
    let due = row.status == CampaignStatus::Archived
        && row.scheduled_delete_at_ms.is_some_and(|at| at <= now_ms);
    if !due {
        return Ok(false);
    }
    row.status = CampaignStatus::Deleting;
    save_campaign(tx, &mut row).await?;
    emit(
        tx,
        campaign,
        NewEvent::campaign(EventKind::DeleteRequested, None),
        now_ms,
    )
    .await?;
    Ok(true)
}

/// O1's one state-setting write: `enter_service()` first, then the six-argument fence, then the
/// campaign lock. A stale fence writes nothing.
pub(super) async fn session_started(
    tx: &Tx<'_>,
    campaign: CampaignId,
    session: SessionId,
    fence: OwnerFence,
    now_ms: u64,
) -> Outcome<()> {
    // The only thing this statement can wait on is a halt holding the service lock, so a lock
    // timeout here is the retryable `service_halting`.
    match tx.query("SELECT df_runtime.enter_service()", &[]).await {
        Ok(_) => {}
        Err(StoreError::Retryable) => return Err(Failure::Store(StoreError::ServiceHalting)),
        Err(error) => return Err(Failure::Store(error)),
    }
    let Some(tenant) = q::campaign_tenant(tx, campaign).await? else {
        return refuse(Refusal::NotFound);
    };
    let current = tx
        .query_one(
            "SELECT df_runtime.fence_current($1, $2, $3, $4, $5, 'game')",
            &[
                &tenant.as_bytes().as_slice(),
                &campaign.as_bytes().as_slice(),
                &fence.recovery_epoch,
                &fence.claim_seq,
                &fence.placement_id.as_slice(),
            ],
        )
        .await?;
    let current: bool = current.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    if !current {
        return Err(Failure::Store(StoreError::FenceStale));
    }
    let row = locked_campaign(tx, campaign).await?;
    if row.status != CampaignStatus::Active {
        return refuse(Refusal::CampaignUnavailable);
    }
    // The lobby is frozen under the same campaign lock every lobby change takes, so a change
    // either lands before the freeze or is refused after it.
    q::freeze_lobby(tx, session, campaign, now_ms).await?;
    Ok(())
}
