//! Invitations, room codes and join requests.

use super::prelude::*;

pub(in super::super) async fn invitation_by_digest(
    tx: &Tx<'_>,
    digest: [u8; 32],
) -> StoreResult<Option<InvitationRow>> {
    let sql = format!(
        "SELECT {INVITATION_COLUMNS} FROM campaigns.campaign_invitation WHERE token_digest = $1"
    );
    tx.query_opt(&sql, &[&digest.as_slice()])
        .await?
        .map(|row| invitation_from_row(&row))
        .transpose()
}

pub(in super::super) async fn invitation_by_id(
    tx: &Tx<'_>,
    campaign: CampaignId,
    invitation: InvitationId,
) -> StoreResult<Option<InvitationRow>> {
    let sql = format!(
        "SELECT {INVITATION_COLUMNS} FROM campaigns.campaign_invitation \
         WHERE campaign_id = $1 AND invitation_id = $2"
    );
    tx.query_opt(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &invitation.as_bytes().as_slice(),
        ],
    )
    .await?
    .map(|row| invitation_from_row(&row))
    .transpose()
}

pub(in super::super) async fn insert_invitation(
    tx: &Tx<'_>,
    row: &InvitationRow,
) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.campaign_invitation (invitation_id, campaign_id, token_digest, \
         created_by_member, joins_as, created_at_ms, expires_at_ms, max_uses, use_count, \
         revoked_at_ms) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        &[
            &row.id.as_bytes().as_slice(),
            &row.campaign.as_bytes().as_slice(),
            &row.token_digest.as_slice(),
            &row.created_by.as_bytes().as_slice(),
            &join_role_text(row.joins_as),
            &to_i64(row.created_at_ms)?,
            &to_i64(row.expires_at_ms)?,
            &to_i32(row.max_uses)?,
            &to_i32(row.use_count)?,
            &row.revoked_at_ms.map(to_i64).transpose()?,
        ],
    )
    .await?;
    Ok(())
}

pub(in super::super) async fn bump_invitation_use(
    tx: &Tx<'_>,
    invitation: InvitationId,
) -> StoreResult<()> {
    tx.execute(
        "UPDATE campaigns.campaign_invitation SET use_count = use_count + 1 WHERE invitation_id = $1",
        &[&invitation.as_bytes().as_slice()],
    )
    .await?;
    Ok(())
}

pub(in super::super) async fn revoke_invitation_row(
    tx: &Tx<'_>,
    invitation: InvitationId,
    now_ms: u64,
) -> StoreResult<u64> {
    tx.execute(
        "UPDATE campaigns.campaign_invitation SET revoked_at_ms = $2 \
         WHERE invitation_id = $1 AND revoked_at_ms IS NULL",
        &[&invitation.as_bytes().as_slice(), &to_i64(now_ms)?],
    )
    .await
}

/// Revokes every still-live invitation a member created. Returns how many.
pub(in super::super) async fn revoke_invitations_of(
    tx: &Tx<'_>,
    campaign: CampaignId,
    creator: MemberId,
    now_ms: u64,
) -> StoreResult<u64> {
    tx.execute(
        "UPDATE campaigns.campaign_invitation SET revoked_at_ms = $3 \
         WHERE campaign_id = $1 AND created_by_member = $2 AND revoked_at_ms IS NULL \
         AND expires_at_ms > $3",
        &[
            &campaign.as_bytes().as_slice(),
            &creator.as_bytes().as_slice(),
            &to_i64(now_ms)?,
        ],
    )
    .await
}

pub(in super::super) async fn code_by_digest(
    tx: &Tx<'_>,
    digest: [u8; 32],
) -> StoreResult<Option<CodeRow>> {
    let sql = format!("SELECT {CODE_COLUMNS} FROM campaigns.join_code WHERE code_digest = $1");
    tx.query_opt(&sql, &[&digest.as_slice()])
        .await?
        .map(|row| code_from_row(&row))
        .transpose()
}

pub(in super::super) async fn revoke_live_code(
    tx: &Tx<'_>,
    campaign: CampaignId,
    now_ms: u64,
) -> StoreResult<u64> {
    tx.execute(
        "UPDATE campaigns.join_code SET revoked_at_ms = $2 \
         WHERE campaign_id = $1 AND revoked_at_ms IS NULL",
        &[&campaign.as_bytes().as_slice(), &to_i64(now_ms)?],
    )
    .await
}

pub(in super::super) async fn insert_code(tx: &Tx<'_>, row: &CodeRow) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.join_code (code_digest, campaign_id, created_at_ms, expires_at_ms) \
         VALUES ($1, $2, $3, $4)",
        &[
            &row.code_digest.as_slice(),
            &row.campaign.as_bytes().as_slice(),
            &to_i64(row.created_at_ms)?,
            &to_i64(row.expires_at_ms)?,
        ],
    )
    .await?;
    Ok(())
}

pub(in super::super) async fn request_by_id(
    tx: &Tx<'_>,
    campaign: CampaignId,
    request: RequestId,
) -> StoreResult<Option<RequestRow>> {
    let sql = format!(
        "SELECT {REQUEST_COLUMNS} FROM campaigns.join_request \
         WHERE campaign_id = $1 AND request_id = $2"
    );
    tx.query_opt(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &request.as_bytes().as_slice(),
        ],
    )
    .await?
    .map(|row| request_from_row(&row))
    .transpose()
}

pub(in super::super) async fn pending_request_for(
    tx: &Tx<'_>,
    campaign: CampaignId,
    account: AccountId,
) -> StoreResult<Option<RequestRow>> {
    let sql = format!(
        "SELECT {REQUEST_COLUMNS} FROM campaigns.join_request \
         WHERE campaign_id = $1 AND account_id = $2 AND state = 'pending'"
    );
    tx.query_opt(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &account.as_bytes().as_slice(),
        ],
    )
    .await?
    .map(|row| request_from_row(&row))
    .transpose()
}

pub(in super::super) async fn pending_request_count(
    tx: &Tx<'_>,
    campaign: CampaignId,
) -> StoreResult<u32> {
    let row = tx
        .query_one(
            "SELECT count(*) FROM campaigns.join_request WHERE campaign_id = $1 AND state = 'pending'",
            &[&campaign.as_bytes().as_slice()],
        )
        .await?;
    let count: i64 = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    u32::try_from(count).map_err(|_| StoreError::InvalidRecord)
}

pub(in super::super) async fn insert_request(tx: &Tx<'_>, row: &RequestRow) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.join_request (request_id, campaign_id, account_id, state, \
         created_at_ms) VALUES ($1, $2, $3, $4, $5)",
        &[
            &row.id.as_bytes().as_slice(),
            &row.campaign.as_bytes().as_slice(),
            &row.account.as_bytes().as_slice(),
            &row.state.text(),
            &to_i64(row.created_at_ms)?,
        ],
    )
    .await?;
    Ok(())
}

pub(in super::super) async fn decide_request_row(
    tx: &Tx<'_>,
    request: RequestId,
    state: df_campaign::records::RequestState,
    approved_as: Option<df_campaign::join_invitation::JoinRole>,
    decided_by: MemberId,
    now_ms: u64,
) -> StoreResult<()> {
    tx.execute(
        "UPDATE campaigns.join_request SET state = $2, approved_as = $3, \
         decided_by_member = $4, decided_at_ms = $5 WHERE request_id = $1",
        &[
            &request.as_bytes().as_slice(),
            &state.text(),
            &approved_as.map(join_role_text),
            &decided_by.as_bytes().as_slice(),
            &to_i64(now_ms)?,
        ],
    )
    .await?;
    Ok(())
}
