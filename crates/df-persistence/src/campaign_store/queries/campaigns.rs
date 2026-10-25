//! Campaign rows, the locks, the event append and the account status view.

use super::prelude::*;

const DB_NOW_MS: &str = "(extract(epoch from clock_timestamp()) * 1000)::bigint";

/// Takes B's account share lock through its definer function and returns the status. Always the
/// first lock of a path that creates a member or accepts a transfer.
pub(in super::super) async fn lock_account(
    tx: &Tx<'_>,
    account: AccountId,
) -> StoreResult<AccountStatus> {
    let row = tx
        .query_opt(
            "SELECT accounts.lock_account_status($1)",
            &[&account.as_bytes().as_slice()],
        )
        .await?;
    let status: Option<String> = match row {
        Some(row) => row.try_get(0).map_err(|_| StoreError::InvalidRecord)?,
        None => None,
    };
    Ok(AccountStatus::parse(status.as_deref()))
}

pub(in super::super) async fn lock_campaign(
    tx: &Tx<'_>,
    campaign: CampaignId,
) -> StoreResult<Option<CampaignRow>> {
    let sql = format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM campaigns.campaign WHERE campaign_id = $1 FOR UPDATE"
    );
    tx.query_opt(&sql, &[&campaign.as_bytes().as_slice()])
        .await?
        .map(|row| campaign_from_row(&row))
        .transpose()
}

/// Locks several campaigns in `campaign_id` order and returns the ones that exist.
pub(in super::super) async fn lock_campaigns_in_order(
    tx: &Tx<'_>,
    campaigns: &[CampaignId],
) -> StoreResult<Vec<CampaignRow>> {
    let mut ids: Vec<Vec<u8>> = campaigns.iter().map(|id| id.as_bytes().to_vec()).collect();
    ids.sort();
    ids.dedup();
    let direction = if tx.locks_reversed() { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM campaigns.campaign WHERE campaign_id = ANY($1) \
         ORDER BY campaign_id {direction} FOR UPDATE"
    );
    tx.query(&sql, &[&ids])
        .await?
        .iter()
        .map(campaign_from_row)
        .collect()
}

pub(in super::super) async fn read_campaign(
    tx: &Tx<'_>,
    campaign: CampaignId,
) -> StoreResult<Option<CampaignRow>> {
    let sql = format!("SELECT {CAMPAIGN_COLUMNS} FROM campaigns.campaign WHERE campaign_id = $1");
    tx.query_opt(&sql, &[&campaign.as_bytes().as_slice()])
        .await?
        .map(|row| campaign_from_row(&row))
        .transpose()
}

pub(in super::super) async fn insert_campaign(tx: &Tx<'_>, row: &CampaignRow) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.campaign (campaign_id, tenant_id, owner_account_id, name, \
         seat_count, content_rating, room_codes, default_co_host_delegations, \
         display_summary_enabled, planned_session_at_ms, status, revision, event_seq, \
         feed_epoch, last_event_at_ms, created_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
        &[
            &row.id.as_bytes().as_slice(),
            &row.tenant.as_bytes().as_slice(),
            &row.owner.as_bytes().as_slice(),
            &row.name,
            &to_i32(row.seat_count)?,
            &content_rating_text(row.content_rating),
            &row.room_codes.text(),
            &i16::from(row.default_co_host_delegations.bits()),
            &row.display_summary_enabled,
            &row.planned_session_at_ms.map(to_i64).transpose()?,
            &row.status.text(),
            &to_i64(row.revision)?,
            &to_i64(row.event_seq)?,
            &row.feed_epoch.as_bytes().as_slice(),
            &to_i64(row.last_event_at_ms)?,
            &to_i64(row.created_at_ms)?,
        ],
    )
    .await?;
    Ok(())
}

/// Writes the mutable columns of a campaign the caller holds locked. The revision is bumped by
/// the caller; the update applies only to the previous revision.
pub(in super::super) async fn update_campaign(tx: &Tx<'_>, row: &CampaignRow) -> StoreResult<bool> {
    let applied = tx
        .execute(
            "UPDATE campaigns.campaign SET owner_account_id = $2, seat_count = $3, \
             status = $4, revision = $5, archived_at_ms = $6, scheduled_delete_at_ms = $7 \
             WHERE campaign_id = $1 AND revision = $5::bigint - 1",
            &[
                &row.id.as_bytes().as_slice(),
                &row.owner.as_bytes().as_slice(),
                &to_i32(row.seat_count)?,
                &row.status.text(),
                &to_i64(row.revision)?,
                &row.archived_at_ms.map(to_i64).transpose()?,
                &row.scheduled_delete_at_ms.map(to_i64).transpose()?,
            ],
        )
        .await?;
    Ok(applied == 1)
}

/// The tenant of a campaign, read without a lock (the fenced write needs it before the lock).
pub(in super::super) async fn campaign_tenant(
    tx: &Tx<'_>,
    campaign: CampaignId,
) -> StoreResult<Option<df_types::TenantId>> {
    let row = tx
        .query_opt(
            "SELECT tenant_id FROM campaigns.campaign WHERE campaign_id = $1",
            &[&campaign.as_bytes().as_slice()],
        )
        .await?;
    row.map(|row| {
        let bytes: &[u8] = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
        df_types::TenantId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
    })
    .transpose()
}

/// Appends an event: bumps the campaign's gapless counter and stamp under the row lock the caller
/// already holds, then writes the row. Returns the new sequence number.
pub(in super::super) async fn append_event(
    tx: &Tx<'_>,
    campaign: CampaignId,
    event: &NewEvent,
    now_ms: u64,
) -> StoreResult<u64> {
    let bumped = tx
        .query_one(
            &format!(
                "UPDATE campaigns.campaign SET event_seq = event_seq + 1, \
                 last_event_at_ms = {DB_NOW_MS} WHERE campaign_id = $1 \
                 RETURNING event_seq, feed_epoch"
            ),
            &[&campaign.as_bytes().as_slice()],
        )
        .await?;
    let sequence: i64 = bumped.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    let epoch: &[u8] = bumped.try_get(1).map_err(|_| StoreError::InvalidRecord)?;
    let kind: EventKind = event.kind;
    tx.execute(
        "INSERT INTO campaigns.campaign_event (campaign_id, event_seq, scope, feed_epoch, \
         member_id, member_revision, revoking, at_ms, kind, actor_member, ticket_id, \
         writer_fence) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        &[
            &campaign.as_bytes().as_slice(),
            &sequence,
            &kind.scope().text(),
            &epoch,
            &event.member.map(|(member, _)| member.as_bytes().to_vec()),
            &event
                .member
                .map(|(_, revision)| to_i64(revision))
                .transpose()?,
            &event.revoking,
            &to_i64(now_ms)?,
            &kind.text(),
            &event.actor.map(|actor| actor.as_bytes().to_vec()),
            &event.ticket.map(|ticket| ticket.as_bytes().to_vec()),
            &event.writer_fence,
        ],
    )
    .await?;
    to_u64(sequence)
}

/// The account's status from B's read-only view; no lock.
pub(in super::super) async fn account_status_view(
    tx: &Tx<'_>,
    account: AccountId,
) -> StoreResult<AccountStatus> {
    let row = tx
        .query_opt(
            "SELECT status FROM accounts.account_status WHERE account_id = $1",
            &[&account.as_bytes().as_slice()],
        )
        .await?;
    let status: Option<String> = match row {
        Some(row) => Some(row.try_get(0).map_err(|_| StoreError::InvalidRecord)?),
        None => None,
    };
    Ok(AccountStatus::parse(status.as_deref()))
}

/// The account's status as B's closure role reads it, from the account table itself: that role
/// owns the table, and the read-only view is granted to the campaign service's role only. Used by
/// the closure paths, which run inside B's transaction.
pub(in super::super) async fn closure_account_status(
    tx: &Tx<'_>,
    account: AccountId,
) -> StoreResult<AccountStatus> {
    let row = tx
        .query_opt(
            "SELECT status FROM accounts.account WHERE account_id = $1",
            &[&account.as_bytes().as_slice()],
        )
        .await?;
    let status: Option<String> = match row {
        Some(row) => Some(row.try_get(0).map_err(|_| StoreError::InvalidRecord)?),
        None => None,
    };
    Ok(AccountStatus::parse(status.as_deref()))
}

/// Sets or clears a campaign's scheduled deletion date and bumps its revision. Closure uses this
/// narrow statement because B's role may write only these columns of `campaign`.
pub(in super::super) async fn set_campaign_schedule(
    tx: &Tx<'_>,
    campaign: CampaignId,
    scheduled_delete_at_ms: Option<u64>,
) -> StoreResult<u64> {
    let row = tx
        .query_one(
            "UPDATE campaigns.campaign SET scheduled_delete_at_ms = $2, revision = revision + 1 \
             WHERE campaign_id = $1 RETURNING revision",
            &[
                &campaign.as_bytes().as_slice(),
                &scheduled_delete_at_ms.map(to_i64).transpose()?,
            ],
        )
        .await?;
    to_u64(row.try_get(0).map_err(|_| StoreError::InvalidRecord)?)
}

/// Serializes the campaign creations of one account for the rest of the transaction, so the count
/// that follows cannot be raced past the bound. Taken after the account share lock and before the
/// count; nothing else takes it, so it adds no edge to the lock order. A hash collision between two
/// accounts only makes their creations wait for each other.
pub(in super::super) async fn lock_campaign_creation(
    tx: &Tx<'_>,
    owner: AccountId,
) -> StoreResult<()> {
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended('df_campaigns.create:' || encode($1::bytea, 'hex'), 0))",
        &[&owner.as_bytes().as_slice()],
    )
    .await?;
    Ok(())
}

/// Campaigns an account owns that are not being deleted (drafts are counted apart), counted up
/// to `cap`.
pub(in super::super) async fn owned_campaigns_up_to(
    tx: &Tx<'_>,
    owner: AccountId,
    cap: u32,
) -> StoreResult<u32> {
    let row = tx
        .query_one(
            "SELECT count(*) FROM (SELECT 1 FROM campaigns.campaign \
             WHERE owner_account_id = $1 AND status IN ('active', 'archived') LIMIT $2) AS bounded",
            &[&owner.as_bytes().as_slice(), &i64::from(cap)],
        )
        .await?;
    let count: i64 = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    u32::try_from(count).map_err(|_| StoreError::InvalidRecord)
}

/// Campaign counts by status, for the dev view.
pub(in super::super) async fn campaign_counts(tx: &Tx<'_>) -> StoreResult<Vec<(String, u64)>> {
    let rows = tx
        .query(
            "SELECT status, count(*) FROM campaigns.campaign GROUP BY status ORDER BY status",
            &[],
        )
        .await?;
    rows.iter()
        .map(|row| {
            let status: String = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
            let count: i64 = row.try_get(1).map_err(|_| StoreError::InvalidRecord)?;
            Ok((status, to_u64(count)?))
        })
        .collect()
}
