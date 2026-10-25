//! Member rows, seat counts and the membership chunks of account closure.

use super::prelude::*;

pub(in super::super) async fn member_by_account(
    tx: &Tx<'_>,
    campaign: CampaignId,
    account: AccountId,
) -> StoreResult<Option<MemberRow>> {
    let sql = format!(
        "SELECT {MEMBER_COLUMNS} FROM campaigns.campaign_member \
         WHERE campaign_id = $1 AND account_id = $2"
    );
    tx.query_opt(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &account.as_bytes().as_slice(),
        ],
    )
    .await?
    .map(|row| member_from_row(&row))
    .transpose()
}

/// A member of this campaign by id. A member of another campaign answers as missing.
pub(in super::super) async fn member_by_id(
    tx: &Tx<'_>,
    campaign: CampaignId,
    member: MemberId,
) -> StoreResult<Option<MemberRow>> {
    let sql = format!(
        "SELECT {MEMBER_COLUMNS} FROM campaigns.campaign_member \
         WHERE campaign_id = $1 AND member_id = $2"
    );
    tx.query_opt(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &member.as_bytes().as_slice(),
        ],
    )
    .await?
    .map(|row| member_from_row(&row))
    .transpose()
}

pub(in super::super) async fn insert_member(tx: &Tx<'_>, row: &MemberRow) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.campaign_member (member_id, campaign_id, account_id, role, \
         holds_seat, delegations, status, may_rejoin, removed_by_role, last_invitation_id, \
         joined_at_ms, ended_at_ms, revision) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        &[
            &row.member.as_bytes().as_slice(),
            &row.campaign.as_bytes().as_slice(),
            &row.account.as_bytes().as_slice(),
            &stored_role(row.role),
            &row.holds_seat,
            &i16::from(row.delegations.bits()),
            &row.status.text(),
            &row.may_rejoin,
            &row.removed_by_role.map(role_text),
            &row.last_invitation.map(|id| id.as_bytes().to_vec()),
            &to_i64(row.joined_at_ms)?,
            &row.ended_at_ms.map(to_i64).transpose()?,
            &to_i64(row.revision)?,
        ],
    )
    .await?;
    Ok(())
}

/// Writes the mutable columns of a member the caller holds locked. The revision is bumped by the
/// caller; the update applies only to the previous revision.
pub(in super::super) async fn update_member(tx: &Tx<'_>, row: &MemberRow) -> StoreResult<bool> {
    let applied = tx
        .execute(
            "UPDATE campaigns.campaign_member SET role = $3, holds_seat = $4, delegations = $5, \
             status = $6, may_rejoin = $7, removed_by_role = $8, last_invitation_id = $9, \
             joined_at_ms = $10, ended_at_ms = $11, revision = $12 \
             WHERE campaign_id = $1 AND member_id = $2 AND revision = $12::bigint - 1",
            &[
                &row.campaign.as_bytes().as_slice(),
                &row.member.as_bytes().as_slice(),
                &stored_role(row.role),
                &row.holds_seat,
                &i16::from(row.delegations.bits()),
                &row.status.text(),
                &row.may_rejoin,
                &row.removed_by_role.map(role_text),
                &row.last_invitation.map(|id| id.as_bytes().to_vec()),
                &to_i64(row.joined_at_ms)?,
                &row.ended_at_ms.map(to_i64).transpose()?,
                &to_i64(row.revision)?,
            ],
        )
        .await?;
    Ok(applied == 1)
}

pub(in super::super) async fn seat_counts(
    tx: &Tx<'_>,
    campaign: CampaignId,
) -> StoreResult<SeatCounts> {
    let row = tx
        .query_one(
            "SELECT count(*) FILTER (WHERE holds_seat), \
             count(*) FILTER (WHERE role = 'contributor') \
             FROM campaigns.campaign_member WHERE campaign_id = $1 AND status = 'active'",
            &[&campaign.as_bytes().as_slice()],
        )
        .await?;
    let seats: i64 = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    let contributors: i64 = row.try_get(1).map_err(|_| StoreError::InvalidRecord)?;
    Ok(SeatCounts {
        seats_in_use: u32::try_from(seats).map_err(|_| StoreError::InvalidRecord)?,
        contributors_in_use: u32::try_from(contributors).map_err(|_| StoreError::InvalidRecord)?,
    })
}

/// The active host's account, for the invitation preview.
pub(in super::super) async fn host_account(
    tx: &Tx<'_>,
    campaign: CampaignId,
) -> StoreResult<Option<AccountId>> {
    let row = tx
        .query_opt(
            "SELECT account_id FROM campaigns.campaign_member \
             WHERE campaign_id = $1 AND role = 'host' AND status = 'active'",
            &[&campaign.as_bytes().as_slice()],
        )
        .await?;
    row.map(|row| {
        let bytes: &[u8] = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
        super::super::rows::account_id(bytes)
    })
    .transpose()
}

/// Removes a member's lobby rows for sessions that have not ended (the caller passes none ended).
pub(in super::super) async fn clear_lobby_rows(
    tx: &Tx<'_>,
    campaign: CampaignId,
    member: MemberId,
) -> StoreResult<u64> {
    tx.execute(
        "DELETE FROM campaigns.lobby_member WHERE campaign_id = $1 AND member_id = $2",
        &[
            &campaign.as_bytes().as_slice(),
            &member.as_bytes().as_slice(),
        ],
    )
    .await
}

/// A chunk of an account's active memberships in `campaign_id` order, after `after`.
pub(in super::super) async fn memberships_chunk(
    tx: &Tx<'_>,
    account: AccountId,
    after: Option<CampaignId>,
    limit: u32,
) -> StoreResult<Vec<CampaignId>> {
    let after_bytes = after.map(|id| id.as_bytes().to_vec());
    let rows = tx
        .query(
            "SELECT campaign_id FROM campaigns.campaign_member \
             WHERE account_id = $1 AND status = 'active' \
             AND ($2::bytea IS NULL OR campaign_id > $2) \
             ORDER BY campaign_id LIMIT $3",
            &[
                &account.as_bytes().as_slice(),
                &after_bytes,
                &i64::from(limit),
            ],
        )
        .await?;
    rows.iter()
        .map(|row| {
            let bytes: &[u8] = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
            super::super::rows::campaign_id(bytes)
        })
        .collect()
}

/// Active members of a campaign (their accounts), for the deletion notice.
pub(in super::super) async fn active_member_accounts(
    tx: &Tx<'_>,
    campaign: CampaignId,
) -> StoreResult<Vec<AccountId>> {
    let rows = tx
        .query(
            "SELECT account_id FROM campaigns.campaign_member \
             WHERE campaign_id = $1 AND status = 'active' ORDER BY account_id",
            &[&campaign.as_bytes().as_slice()],
        )
        .await?;
    rows.iter()
        .map(|row| {
            let bytes: &[u8] = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
            super::super::rows::account_id(bytes)
        })
        .collect()
}
