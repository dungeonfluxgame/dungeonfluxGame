//! The reads of the membership feed (5.1): the one-statement snapshot, the heads, the keyset
//! catch-up and the stamp-based discovery hint. All run on the primary through the store's pool.

use df_campaign::{
    errors::{StoreError, StoreResult},
    records::{
        CampaignChange, CampaignHead, CampaignStatus, EventRow, MemberSnapshot, MemberStatus,
        parse_role,
    },
    roles::Delegations,
    store::ChangeKey,
};
use df_types::{AccountId, CampaignId};

use super::{
    conn::Pool,
    rows::{
        EVENT_COLUMNS, account_id, campaign_id, event_from_row, head_from_row, member_id, to_i64,
        to_u32, to_u64,
    },
};

/// The member row, the campaign's `event_seq` and `feed_epoch` and the account's status are read
/// by a single statement, so one MVCC snapshot: an event cannot commit between them and be
/// skipped by a consumer that starts from the snapshot's `event_seq`. An account that is not
/// `active` (closing, closed or missing) is not authorized, whatever the member row says.
pub(super) const SNAPSHOT_SQL: &str = "SELECT m.member_id, m.account_id, c.campaign_id, c.tenant_id, \
    m.role, m.holds_seat, m.delegations, m.status, m.revision, c.status, c.feed_epoch, \
    c.seat_count, c.event_seq, a.status \
    FROM campaigns.campaign_member m \
    JOIN campaigns.campaign c ON c.campaign_id = m.campaign_id \
    LEFT JOIN accounts.account_status a ON a.account_id = m.account_id \
    WHERE m.campaign_id = $1 AND m.account_id = $2";

pub(super) async fn member_snapshot(
    pool: &Pool,
    account: AccountId,
    campaign: CampaignId,
) -> StoreResult<Option<MemberSnapshot>> {
    let rows = pool
        .read(
            SNAPSHOT_SQL,
            &[
                &campaign.as_bytes().as_slice(),
                &account.as_bytes().as_slice(),
            ],
        )
        .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let col = |error: tokio_postgres::Error| {
        let _ = error;
        StoreError::InvalidRecord
    };
    let role: String = row.try_get(4).map_err(col)?;
    let delegations: i16 = row.try_get(6).map_err(col)?;
    let member_status: String = row.try_get(7).map_err(col)?;
    let campaign_status: String = row.try_get(9).map_err(col)?;
    let epoch: &[u8] = row.try_get(10).map_err(col)?;
    let account_status: Option<String> = row.try_get(13).map_err(col)?;
    let member_status = MemberStatus::parse(&member_status).ok_or(StoreError::InvalidRecord)?;
    let account_active = account_status.as_deref() == Some("active");
    Ok(Some(MemberSnapshot {
        member: member_id(row.try_get(0).map_err(col)?)?,
        account: account_id(row.try_get(1).map_err(col)?)?,
        campaign: campaign_id(row.try_get(2).map_err(col)?)?,
        tenant: df_types::TenantId::from_bytes(row.try_get(3).map_err(col)?)
            .map_err(|_| StoreError::InvalidRecord)?,
        role: parse_role(&role).ok_or(StoreError::InvalidRecord)?,
        holds_seat: row.try_get(5).map_err(col)?,
        delegations: u8::try_from(delegations)
            .ok()
            .and_then(Delegations::from_bits)
            .ok_or(StoreError::InvalidRecord)?,
        status: member_status,
        authorized: member_status == MemberStatus::Active && account_active,
        member_revision: to_u64(row.try_get(8).map_err(col)?)?,
        campaign_status: CampaignStatus::parse(&campaign_status)
            .ok_or(StoreError::InvalidRecord)?,
        feed_epoch: df_campaign::ids::FeedEpoch::from_bytes(epoch)
            .ok_or(StoreError::InvalidRecord)?,
        seat_count: to_u32(row.try_get(11).map_err(col)?)?,
        event_seq: to_u64(row.try_get(12).map_err(col)?)?,
    }))
}

pub(super) async fn campaign_heads(
    pool: &Pool,
    campaigns: &[CampaignId],
) -> StoreResult<Vec<CampaignHead>> {
    let ids: Vec<Vec<u8>> = campaigns.iter().map(|id| id.as_bytes().to_vec()).collect();
    let rows = pool
        .read(
            "SELECT campaign_id, event_seq, feed_epoch FROM campaigns.campaign \
             WHERE campaign_id = ANY($1) ORDER BY campaign_id",
            &[&ids],
        )
        .await?;
    rows.iter().map(head_from_row).collect()
}

pub(super) async fn campaign_heads_after(
    pool: &Pool,
    after: Option<CampaignId>,
    limit: u32,
) -> StoreResult<Vec<CampaignHead>> {
    let after = after.map(|id| id.as_bytes().to_vec());
    let rows = pool
        .read(
            "SELECT campaign_id, event_seq, feed_epoch FROM campaigns.campaign \
             WHERE ($1::bytea IS NULL OR campaign_id > $1) ORDER BY campaign_id LIMIT $2",
            &[&after, &i64::from(limit)],
        )
        .await?;
    rows.iter().map(head_from_row).collect()
}

/// The discovery hint: campaigns whose last event stamp is at or after `lower_bound_ms`, in
/// (stamp, id) order after an exclusive keyset position. A missed hint costs the next sweep,
/// never an event.
pub(super) async fn campaign_changes_since(
    pool: &Pool,
    lower_bound_ms: u64,
    after: Option<ChangeKey>,
    limit: u32,
) -> StoreResult<Vec<CampaignChange>> {
    let lower = to_i64(lower_bound_ms)?;
    let after_stamp = after.map(|key| to_i64(key.last_event_at_ms)).transpose()?;
    let after_id = after.map(|key| key.campaign.as_bytes().to_vec());
    let rows = pool
        .read(
            "SELECT campaign_id, event_seq, feed_epoch, last_event_at_ms FROM campaigns.campaign \
             WHERE last_event_at_ms >= $1 \
             AND ($2::bigint IS NULL OR (last_event_at_ms, campaign_id) > ($2::bigint, $3::bytea)) \
             ORDER BY last_event_at_ms, campaign_id LIMIT $4",
            &[&lower, &after_stamp, &after_id, &i64::from(limit)],
        )
        .await?;
    rows.iter()
        .map(|row| {
            let head = head_from_row(row)?;
            let stamp: i64 = row.try_get(3).map_err(|_| StoreError::InvalidRecord)?;
            Ok(CampaignChange {
                campaign: head.campaign,
                event_seq: head.event_seq,
                feed_epoch: head.feed_epoch,
                last_event_at_ms: to_u64(stamp)?,
            })
        })
        .collect()
}

pub(super) async fn events_after(
    pool: &Pool,
    campaign: CampaignId,
    after_seq: u64,
    limit: u32,
) -> StoreResult<Vec<EventRow>> {
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM campaigns.campaign_event \
         WHERE campaign_id = $1 AND event_seq > $2 ORDER BY event_seq LIMIT $3"
    );
    let rows = pool
        .read(
            &sql,
            &[
                &campaign.as_bytes().as_slice(),
                &to_i64(after_seq)?,
                &i64::from(limit),
            ],
        )
        .await?;
    rows.iter().map(event_from_row).collect()
}

/// The primary's `statement_timestamp()` in milliseconds: the poll anchor. Not `clock_timestamp()`,
/// which would be later than the statement's snapshot and eat the overlap.
pub(super) async fn database_time_ms(pool: &Pool) -> StoreResult<u64> {
    let rows = pool
        .read(
            "SELECT (extract(epoch from statement_timestamp()) * 1000)::bigint",
            &[],
        )
        .await?;
    let row = rows.first().ok_or(StoreError::InvalidRecord)?;
    to_u64(row.try_get(0).map_err(|_| StoreError::InvalidRecord)?)
}
