//! Account closure on the campaign side (J14, 8.7). These run inside B's close transaction (a
//! borrowed `Tx`) or the completion chunks. Every method that touches several campaigns locks
//! them in `campaign_id` order, and every chunk begins with the account lock.

use df_campaign::{
    config::Tunables,
    errors::Outcome,
    records::{
        AccountStatus, CampaignRow, CampaignStatus, CancelReport, ChunkReport, EventKind,
        LeaveChunk, MemberStatus, NewEvent,
    },
    roles::CampaignRole,
};
use df_types::{AccountId, CampaignId};

use super::{
    conn::Tx,
    queries as q,
    shared::{emit, end_membership},
    transfers::host_swap,
};

fn timeout_ms(tunables: &Tunables) -> u64 {
    u64::from(tunables.feed_transaction_timeout_seconds) * 1_000
}

/// Active campaigns the account owns or hosts block closing; owned archived ones are listed.
pub struct Closure {
    pub blocking: Vec<(CampaignId, String, bool, bool)>,
    pub owned_archived: Vec<(CampaignId, String)>,
}

pub(super) async fn closure_campaigns(
    tx: &Tx<'_>,
    tunables: &Tunables,
    account: AccountId,
) -> Outcome<Closure> {
    tx.require_timeout(timeout_ms(tunables)).await?;
    let rows = tx
        .query(
            "SELECT c.campaign_id, c.name, c.owner_account_id = $1 AS owns, \
             EXISTS (SELECT 1 FROM campaigns.campaign_member m \
                     WHERE m.campaign_id = c.campaign_id AND m.account_id = $1 \
                     AND m.role = 'host' AND m.status = 'active') AS hosts \
             FROM campaigns.campaign c \
             WHERE c.status = 'active' AND (c.owner_account_id = $1 OR EXISTS ( \
                 SELECT 1 FROM campaigns.campaign_member m \
                 WHERE m.campaign_id = c.campaign_id AND m.account_id = $1 \
                 AND m.role = 'host' AND m.status = 'active')) \
             ORDER BY c.campaign_id",
            &[&account.as_bytes().as_slice()],
        )
        .await?;
    let mut blocking = Vec::new();
    for row in &rows {
        let id: &[u8] = row
            .try_get(0)
            .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?;
        blocking.push((
            super::rows::campaign_id(id)?,
            row.try_get::<_, String>(1)
                .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?,
            row.try_get::<_, bool>(2)
                .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?,
            row.try_get::<_, bool>(3)
                .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?,
        ));
    }
    let archived = tx
        .query(
            "SELECT campaign_id, name FROM campaigns.campaign \
             WHERE owner_account_id = $1 AND status = 'archived' ORDER BY campaign_id",
            &[&account.as_bytes().as_slice()],
        )
        .await?;
    let mut owned_archived = Vec::new();
    for row in &archived {
        let id: &[u8] = row
            .try_get(0)
            .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?;
        owned_archived.push((
            super::rows::campaign_id(id)?,
            row.try_get::<_, String>(1)
                .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?,
        ));
    }
    Ok(Closure {
        blocking,
        owned_archived,
    })
}

/// A scheduled deletion and the accounts to tell.
pub struct Scheduled {
    pub campaign: CampaignId,
    pub name: String,
    pub members: Vec<AccountId>,
}

/// Sets the deletion date of each owned archived campaign and cancels the account's outgoing
/// proposals. The lock set is the sorted union of the owned archived campaigns and every campaign
/// with a `proposed` row made by the account.
pub(super) async fn schedule_archived_deletions(
    tx: &Tx<'_>,
    tunables: &Tunables,
    account: AccountId,
    delete_after_ms: u64,
    now_ms: u64,
) -> Outcome<Vec<Scheduled>> {
    tx.require_timeout(timeout_ms(tunables)).await?;
    // An unfinished wizard has no members and no events; it goes with the account.
    q::delete_drafts_of(tx, account).await?;
    // The notice is never shorter than the configured minimum, whatever the caller asked for.
    let delete_after_ms =
        delete_after_ms.max(now_ms.saturating_add(tunables.closure_archived_delete_notice_ms()));
    let owned = tx
        .query(
            "SELECT campaign_id FROM campaigns.campaign \
             WHERE owner_account_id = $1 AND status = 'archived' ORDER BY campaign_id",
            &[&account.as_bytes().as_slice()],
        )
        .await?;
    let mut set: Vec<CampaignId> = Vec::new();
    for row in &owned {
        let id: &[u8] = row
            .try_get(0)
            .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?;
        set.push(super::rows::campaign_id(id)?);
    }
    set.extend(q::campaigns_with_proposals_from(tx, account).await?);
    set.sort();
    set.dedup();
    let locked = q::lock_campaigns_in_order(tx, &set).await?;
    let mut scheduled = Vec::new();
    for row in locked {
        if row.owner == account && row.status == CampaignStatus::Archived {
            q::set_campaign_schedule(tx, row.id, Some(delete_after_ms)).await?;
            emit(
                tx,
                row.id,
                NewEvent::campaign(EventKind::DeleteScheduled, None),
                now_ms,
            )
            .await?;
            scheduled.push(Scheduled {
                campaign: row.id,
                name: row.name.clone(),
                members: q::active_member_accounts(tx, row.id).await?,
            });
        }
    }
    q::cancel_proposals_from(tx, account, &set).await?;
    Ok(scheduled)
}

/// Reopening: clears the deletion dates the closing set. A campaign already `deleting` stays so.
pub(super) async fn cancel_scheduled_deletions(
    tx: &Tx<'_>,
    tunables: &Tunables,
    account: AccountId,
    now_ms: u64,
) -> Outcome<CancelReport> {
    tx.require_timeout(timeout_ms(tunables)).await?;
    let rows = tx
        .query(
            "SELECT campaign_id FROM campaigns.campaign \
             WHERE owner_account_id = $1 AND scheduled_delete_at_ms IS NOT NULL \
             ORDER BY campaign_id",
            &[&account.as_bytes().as_slice()],
        )
        .await?;
    let mut ids = Vec::new();
    for row in &rows {
        let id: &[u8] = row
            .try_get(0)
            .map_err(|_| df_campaign::errors::StoreError::InvalidRecord)?;
        ids.push(super::rows::campaign_id(id)?);
    }
    let mut report = CancelReport::default();
    for row in q::lock_campaigns_in_order(tx, &ids).await? {
        match row.status {
            CampaignStatus::Archived if row.scheduled_delete_at_ms.is_some() => {
                q::set_campaign_schedule(tx, row.id, None).await?;
                emit(
                    tx,
                    row.id,
                    NewEvent::campaign(EventKind::DeleteCancelled, None),
                    now_ms,
                )
                .await?;
                report.cancelled.push(row.id);
            }
            CampaignStatus::Deleting => report.already_deleting.push(row.id),
            _ => {}
        }
    }
    Ok(report)
}

/// One resumable, abortable chunk of `leave_all`. It begins with B's account lock; if the account
/// is no longer `closing` (a reopening login landed between chunks) it changes nothing and says so.
pub(super) async fn leave_all_chunk(
    tx: &Tx<'_>,
    tunables: &Tunables,
    account: AccountId,
    after: Option<CampaignId>,
    now_ms: u64,
) -> Outcome<ChunkReport> {
    tx.require_timeout(timeout_ms(tunables)).await?;
    let mut report = ChunkReport {
        outcome: LeaveChunk::Done,
        left: Vec::new(),
        handed_over: Vec::new(),
        skipped_owned: Vec::new(),
        late_hosts: Vec::new(),
        handover_skipped: Vec::new(),
    };
    // B's statement that takes the account row came first; this reads the status it holds.
    if q::closure_account_status(tx, account).await? != AccountStatus::Closing {
        report.outcome = LeaveChunk::Aborted;
        return Ok(report);
    }
    let ids = q::memberships_chunk(tx, account, after, tunables.leave_all_chunk).await?;
    let locked = q::lock_campaigns_in_order(tx, &ids).await?;
    for campaign in &locked {
        leave_one(tx, campaign, account, now_ms, &mut report).await?;
    }
    report.outcome = if ids.len() as u32 >= tunables.leave_all_chunk {
        match ids.last() {
            Some(last) => LeaveChunk::Continue(*last),
            None => LeaveChunk::Done,
        }
    } else {
        LeaveChunk::Done
    };
    Ok(report)
}

async fn leave_one(
    tx: &Tx<'_>,
    campaign: &CampaignRow,
    account: AccountId,
    now_ms: u64,
    report: &mut ChunkReport,
) -> Outcome<()> {
    // Owned campaigns are handled by scheduled deletion: the membership and host role stay.
    if campaign.owner == account {
        report.skipped_owned.push(campaign.id);
        return Ok(());
    }
    let Some(mut me) = q::member_by_account(tx, campaign.id, account).await? else {
        return Ok(());
    };
    if me.status != MemberStatus::Active {
        return Ok(());
    }
    if me.role == CampaignRole::Host && campaign.status != CampaignStatus::Deleting {
        if campaign.status == CampaignStatus::Active {
            report.late_hosts.push(campaign.id);
        }
        let Some(mut owner) = q::member_by_account(tx, campaign.id, campaign.owner).await? else {
            report.handover_skipped.push(campaign.id);
            return Ok(());
        };
        let owner_active = owner.status == MemberStatus::Active
            && q::closure_account_status(tx, campaign.owner)
                .await?
                .is_active();
        if !owner_active {
            report.handover_skipped.push(campaign.id);
            return Ok(());
        }
        // The same events as a host transfer; the owner may be seatless and stays so. The old
        // host becomes a co-host for the swap and then leaves.
        host_swap(
            tx,
            campaign,
            &mut me,
            &mut owner,
            Some(df_campaign::records::OldHostBecomes::CoHost),
            now_ms,
        )
        .await?;
        report.handed_over.push(campaign.id);
    }
    let own = Some(me.member);
    end_membership(
        tx,
        campaign.id,
        &mut me,
        MemberStatus::Left,
        None,
        own,
        now_ms,
    )
    .await?;
    report.left.push(campaign.id);
    Ok(())
}
