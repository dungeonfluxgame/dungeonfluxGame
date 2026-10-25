//! Display pairings and tickets.

use super::prelude::*;

pub(in super::super) async fn pairing_by_device(
    tx: &Tx<'_>,
    device: [u8; 32],
) -> StoreResult<Option<PairingRow>> {
    let sql = format!(
        "SELECT {PAIRING_COLUMNS} FROM campaigns.display_pairing WHERE device_secret_digest = $1"
    );
    tx.query_opt(&sql, &[&device.as_slice()])
        .await?
        .map(|row| pairing_from_row(&row))
        .transpose()
}

pub(in super::super) async fn pending_pairing_by_code(
    tx: &Tx<'_>,
    code: &str,
) -> StoreResult<Option<PairingRow>> {
    let sql = format!(
        "SELECT {PAIRING_COLUMNS} FROM campaigns.display_pairing \
         WHERE user_code = $1 AND state = 'pending'"
    );
    tx.query_opt(&sql, &[&code])
        .await?
        .map(|row| pairing_from_row(&row))
        .transpose()
}

pub(in super::super) async fn pairing_by_id(
    tx: &Tx<'_>,
    pairing: PairingId,
) -> StoreResult<Option<PairingRow>> {
    let sql =
        format!("SELECT {PAIRING_COLUMNS} FROM campaigns.display_pairing WHERE pairing_id = $1");
    tx.query_opt(&sql, &[&pairing.as_bytes().as_slice()])
        .await?
        .map(|row| pairing_from_row(&row))
        .transpose()
}

pub(in super::super) async fn insert_pairing(tx: &Tx<'_>, row: &PairingRow) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.display_pairing (pairing_id, user_code, device_secret_digest, \
         device_label, ip_key, ip_key_loose, created_at_ms, expires_at_ms, state) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        &[
            &row.id.as_bytes().as_slice(),
            &row.user_code,
            &row.device_digest.as_slice(),
            &row.device_label.text(),
            &row.ip_key.as_slice(),
            &row.ip_key_loose.as_slice(),
            &to_i64(row.created_at_ms)?,
            &to_i64(row.expires_at_ms)?,
            &pairing_state_text(row.state),
        ],
    )
    .await?;
    Ok(())
}

/// Writes the columns a pairing changes after creation.
pub(in super::super) async fn update_pairing(tx: &Tx<'_>, row: &PairingRow) -> StoreResult<()> {
    tx.execute(
        "UPDATE campaigns.display_pairing SET campaign_id = $2, session_id = $3, state = $4, \
         approved_by_member = $5, approved_at_ms = $6, redeemed_at_ms = $7, reissue_count = $8, \
         renewable_until_ms = $9, user_code = $10 WHERE pairing_id = $1",
        &[
            &row.id.as_bytes().as_slice(),
            &row.campaign.map(|id| id.as_bytes().to_vec()),
            &row.session.map(|id| id.as_bytes().to_vec()),
            &pairing_state_text(row.state),
            &row.approved_by.map(|id| id.as_bytes().to_vec()),
            &row.approved_at_ms.map(to_i64).transpose()?,
            &row.redeemed_at_ms.map(to_i64).transpose()?,
            &i16::from(row.reissue_count),
            &row.renewable_until_ms.map(to_i64).transpose()?,
            &row.user_code,
        ],
    )
    .await?;
    Ok(())
}

/// Which of a pairing's two address keys a count is by.
#[derive(Clone, Copy, Debug)]
pub(in super::super) enum AddressKey {
    Tight,
    Loose,
}

/// Pending, unexpired pairings under one address key, counted up to `cap` (a caller only needs to
/// know whether the cap is reached, and an unauthenticated flood must not make this scan grow).
pub(in super::super) async fn pending_pairings_for_key(
    tx: &Tx<'_>,
    which: AddressKey,
    key: [u8; 32],
    now_ms: u64,
    cap: u32,
) -> StoreResult<u32> {
    let column = match which {
        AddressKey::Tight => "ip_key",
        AddressKey::Loose => "ip_key_loose",
    };
    let sql = format!(
        "SELECT count(*) FROM (SELECT 1 FROM campaigns.display_pairing \
         WHERE {column} = $1 AND state = 'pending' AND expires_at_ms > $2 LIMIT $3) AS bounded"
    );
    let row = tx
        .query_one(&sql, &[&key.as_slice(), &to_i64(now_ms)?, &i64::from(cap)])
        .await?;
    let count: i64 = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    u32::try_from(count).map_err(|_| StoreError::InvalidRecord)
}

/// All pending, unexpired pairings, counted up to `cap`.
pub(in super::super) async fn pending_pairings_total(
    tx: &Tx<'_>,
    now_ms: u64,
    cap: u32,
) -> StoreResult<u32> {
    let row = tx
        .query_one(
            "SELECT count(*) FROM (SELECT 1 FROM campaigns.display_pairing \
             WHERE state = 'pending' AND expires_at_ms > $1 LIMIT $2) AS bounded",
            &[&to_i64(now_ms)?, &i64::from(cap)],
        )
        .await?;
    let count: i64 = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    u32::try_from(count).map_err(|_| StoreError::InvalidRecord)
}

/// The pairing row locked for this transaction, so two approvals of one pairing serialize on it.
pub(in super::super) async fn pairing_by_id_for_update(
    tx: &Tx<'_>,
    pairing: PairingId,
) -> StoreResult<Option<PairingRow>> {
    let sql = format!(
        "SELECT {PAIRING_COLUMNS} FROM campaigns.display_pairing WHERE pairing_id = $1 FOR UPDATE"
    );
    tx.query_opt(&sql, &[&pairing.as_bytes().as_slice()])
        .await?
        .map(|row| pairing_from_row(&row))
        .transpose()
}

/// Moves a pending pairing to approved. The `WHERE state = 'pending'` makes it a compare-and-swap:
/// `false` means someone else decided it first.
pub(in super::super) async fn approve_pairing_row(
    tx: &Tx<'_>,
    row: &PairingRow,
) -> StoreResult<bool> {
    let changed = tx
        .execute(
            "UPDATE campaigns.display_pairing SET campaign_id = $2, session_id = $3, \
             state = 'approved', approved_by_member = $4, approved_at_ms = $5, \
             renewable_until_ms = $6, user_code = NULL \
             WHERE pairing_id = $1 AND state = 'pending'",
            &[
                &row.id.as_bytes().as_slice(),
                &row.campaign.map(|id| id.as_bytes().to_vec()),
                &row.session.map(|id| id.as_bytes().to_vec()),
                &row.approved_by.map(|id| id.as_bytes().to_vec()),
                &row.approved_at_ms.map(to_i64).transpose()?,
                &row.renewable_until_ms.map(to_i64).transpose()?,
            ],
        )
        .await?;
    Ok(changed == 1)
}

/// A pairing's live tickets (not revoked, not expired).
pub(in super::super) async fn live_tickets_of_pairing(
    tx: &Tx<'_>,
    pairing: PairingId,
    now_ms: u64,
) -> StoreResult<Vec<TicketRow>> {
    let sql = format!(
        "SELECT {TICKET_COLUMNS} FROM campaigns.display_ticket \
         WHERE pairing_id = $1 AND revoked_at_ms IS NULL AND expires_at_ms > $2 ORDER BY ticket_id"
    );
    tx.query(&sql, &[&pairing.as_bytes().as_slice(), &to_i64(now_ms)?])
        .await?
        .iter()
        .map(ticket_from_row)
        .collect()
}

pub(in super::super) async fn insert_ticket(tx: &Tx<'_>, row: &TicketRow) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.display_ticket (ticket_id, ticket_digest, pairing_id, campaign_id, \
         session_id, device_secret_digest, device_label, approved_by_member, issued_at_ms, \
         expires_at_ms) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        &[
            &row.id.as_bytes().as_slice(),
            &row.token_digest.as_slice(),
            &row.pairing.as_bytes().as_slice(),
            &row.campaign.as_bytes().as_slice(),
            &row.session.as_bytes().as_slice(),
            &row.device_digest.as_slice(),
            &row.device_label.text(),
            &row.approved_by.as_bytes().as_slice(),
            &to_i64(row.issued_at_ms)?,
            &to_i64(row.expires_at_ms)?,
        ],
    )
    .await?;
    Ok(())
}

pub(in super::super) async fn ticket_by_digest(
    tx: &Tx<'_>,
    digest: [u8; 32],
) -> StoreResult<Option<TicketRow>> {
    let sql =
        format!("SELECT {TICKET_COLUMNS} FROM campaigns.display_ticket WHERE ticket_digest = $1");
    tx.query_opt(&sql, &[&digest.as_slice()])
        .await?
        .map(|row| ticket_from_row(&row))
        .transpose()
}

/// A ticket by id together with its session and campaign: a ticket of another session or
/// campaign answers as missing.
pub(in super::super) async fn ticket_by_id(
    tx: &Tx<'_>,
    campaign: CampaignId,
    session: SessionId,
    ticket: TicketId,
) -> StoreResult<Option<TicketRow>> {
    let sql = format!(
        "SELECT {TICKET_COLUMNS} FROM campaigns.display_ticket \
         WHERE campaign_id = $1 AND session_id = $2 AND ticket_id = $3"
    );
    tx.query_opt(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &session.as_bytes().as_slice(),
            &ticket.as_bytes().as_slice(),
        ],
    )
    .await?
    .map(|row| ticket_from_row(&row))
    .transpose()
}

/// A ticket by its row id alone, to read what a revocation event names.
pub(in super::super) async fn live_tickets_of_session(
    tx: &Tx<'_>,
    campaign: CampaignId,
    session: SessionId,
    now_ms: u64,
) -> StoreResult<Vec<TicketRow>> {
    let sql = format!(
        "SELECT {TICKET_COLUMNS} FROM campaigns.display_ticket \
         WHERE campaign_id = $1 AND session_id = $2 AND revoked_at_ms IS NULL \
         AND expires_at_ms > $3 ORDER BY ticket_id"
    );
    tx.query(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &session.as_bytes().as_slice(),
            &to_i64(now_ms)?,
        ],
    )
    .await?
    .iter()
    .map(ticket_from_row)
    .collect()
}

pub(in super::super) async fn live_tickets_of_campaign(
    tx: &Tx<'_>,
    campaign: CampaignId,
    now_ms: u64,
) -> StoreResult<Vec<TicketRow>> {
    let sql = format!(
        "SELECT {TICKET_COLUMNS} FROM campaigns.display_ticket \
         WHERE campaign_id = $1 AND revoked_at_ms IS NULL AND expires_at_ms > $2 \
         ORDER BY ticket_id"
    );
    tx.query(&sql, &[&campaign.as_bytes().as_slice(), &to_i64(now_ms)?])
        .await?
        .iter()
        .map(ticket_from_row)
        .collect()
}

pub(in super::super) async fn live_tickets_approved_by(
    tx: &Tx<'_>,
    campaign: CampaignId,
    member: MemberId,
    now_ms: u64,
) -> StoreResult<Vec<TicketRow>> {
    let sql = format!(
        "SELECT {TICKET_COLUMNS} FROM campaigns.display_ticket \
         WHERE campaign_id = $1 AND approved_by_member = $2 AND revoked_at_ms IS NULL \
         AND expires_at_ms > $3 ORDER BY ticket_id"
    );
    tx.query(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &member.as_bytes().as_slice(),
            &to_i64(now_ms)?,
        ],
    )
    .await?
    .iter()
    .map(ticket_from_row)
    .collect()
}

pub(in super::super) async fn revoke_ticket_row(
    tx: &Tx<'_>,
    ticket: TicketId,
    reason: RevokeReason,
    now_ms: u64,
) -> StoreResult<bool> {
    let applied = tx
        .execute(
            "UPDATE campaigns.display_ticket SET revoked_at_ms = $2, revoke_reason = $3 \
             WHERE ticket_id = $1 AND revoked_at_ms IS NULL",
            &[
                &ticket.as_bytes().as_slice(),
                &to_i64(now_ms)?,
                &reason.text(),
            ],
        )
        .await?;
    Ok(applied == 1)
}

pub(in super::super) async fn set_ticket_expiry(
    tx: &Tx<'_>,
    ticket: TicketId,
    expires_at_ms: u64,
) -> StoreResult<()> {
    tx.execute(
        "UPDATE campaigns.display_ticket SET expires_at_ms = $2 WHERE ticket_id = $1",
        &[&ticket.as_bytes().as_slice(), &to_i64(expires_at_ms)?],
    )
    .await?;
    Ok(())
}

/// Cancels approved-but-unredeemed pairings of a campaign, or of one session of it.
pub(in super::super) async fn cancel_approved_pairings(
    tx: &Tx<'_>,
    campaign: CampaignId,
    session: Option<SessionId>,
) -> StoreResult<u64> {
    let session = session.map(|id| id.as_bytes().to_vec());
    tx.execute(
        "UPDATE campaigns.display_pairing SET state = 'cancelled', user_code = NULL \
         WHERE campaign_id = $1 AND state = 'approved' AND ($2::bytea IS NULL OR session_id = $2)",
        &[&campaign.as_bytes().as_slice(), &session],
    )
    .await
}

/// Approved-but-unredeemed pairings of a session (they count toward the display cap).
pub(in super::super) async fn approved_pairings_of_session(
    tx: &Tx<'_>,
    campaign: CampaignId,
    session: SessionId,
) -> StoreResult<u32> {
    let row = tx
        .query_one(
            "SELECT count(*) FROM campaigns.display_pairing \
             WHERE campaign_id = $1 AND session_id = $2 AND state = 'approved'",
            &[
                &campaign.as_bytes().as_slice(),
                &session.as_bytes().as_slice(),
            ],
        )
        .await?;
    let count: i64 = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    u32::try_from(count).map_err(|_| StoreError::InvalidRecord)
}
