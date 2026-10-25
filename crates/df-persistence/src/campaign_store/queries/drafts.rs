//! Draft campaigns: the rows of the create wizard (8.1). A draft is a `campaign` row in the
//! `draft` state; only its owner reaches it, and an expired one is as good as missing.

use df_campaign::{
    draft::{DraftRow, DraftStepData},
    ids::FeedEpoch,
};

use super::prelude::*;
use crate::campaign_store::rows::{DRAFT_COLUMNS, draft_from_row};

pub(in super::super) async fn insert_draft(
    tx: &Tx<'_>,
    id: CampaignId,
    owner: AccountId,
    tenant: df_types::TenantId,
    epoch: FeedEpoch,
    now_ms: u64,
    expires_at_ms: u64,
) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.campaign (campaign_id, tenant_id, owner_account_id, name, \
         seat_count, status, draft_step, revision, event_seq, feed_epoch, last_event_at_ms, \
         created_at_ms, draft_expires_at_ms) \
         VALUES ($1, $2, $3, 'Untitled campaign', 4, 'draft', 1, 1, 0, $4, 0, $5, $6)",
        &[
            &id.as_bytes().as_slice(),
            &tenant.as_bytes().as_slice(),
            &owner.as_bytes().as_slice(),
            &epoch.as_bytes().as_slice(),
            &to_i64(now_ms)?,
            &to_i64(expires_at_ms)?,
        ],
    )
    .await?;
    Ok(())
}

/// The owner's live draft, locked. A foreign, finished or expired one is `None`.
pub(in super::super) async fn lock_draft(
    tx: &Tx<'_>,
    draft: CampaignId,
    owner: AccountId,
    now_ms: u64,
) -> StoreResult<Option<DraftRow>> {
    let sql = format!(
        "SELECT {DRAFT_COLUMNS} FROM campaigns.campaign WHERE campaign_id = $1 \
         AND owner_account_id = $2 AND status = 'draft' AND draft_expires_at_ms > $3 FOR UPDATE"
    );
    tx.query_opt(
        &sql,
        &[
            &draft.as_bytes().as_slice(),
            &owner.as_bytes().as_slice(),
            &to_i64(now_ms)?,
        ],
    )
    .await?
    .map(|row| draft_from_row(&row))
    .transpose()
}

pub(in super::super) async fn read_draft(
    tx: &Tx<'_>,
    draft: CampaignId,
    owner: AccountId,
    now_ms: u64,
) -> StoreResult<Option<DraftRow>> {
    let sql = format!(
        "SELECT {DRAFT_COLUMNS} FROM campaigns.campaign WHERE campaign_id = $1 \
         AND owner_account_id = $2 AND status = 'draft' AND draft_expires_at_ms > $3"
    );
    tx.query_opt(
        &sql,
        &[
            &draft.as_bytes().as_slice(),
            &owner.as_bytes().as_slice(),
            &to_i64(now_ms)?,
        ],
    )
    .await?
    .map(|row| draft_from_row(&row))
    .transpose()
}

pub(in super::super) async fn list_drafts(
    tx: &Tx<'_>,
    owner: AccountId,
    now_ms: u64,
) -> StoreResult<Vec<DraftRow>> {
    let sql = format!(
        "SELECT {DRAFT_COLUMNS} FROM campaigns.campaign WHERE owner_account_id = $1 \
         AND status = 'draft' AND draft_expires_at_ms > $2 ORDER BY created_at_ms, campaign_id"
    );
    tx.query(&sql, &[&owner.as_bytes().as_slice(), &to_i64(now_ms)?])
        .await?
        .iter()
        .map(draft_from_row)
        .collect()
}

/// Live drafts of an account, counted up to `cap`.
pub(in super::super) async fn live_drafts_up_to(
    tx: &Tx<'_>,
    owner: AccountId,
    cap: u32,
    now_ms: u64,
) -> StoreResult<u32> {
    let row = tx
        .query_one(
            "SELECT count(*) FROM (SELECT 1 FROM campaigns.campaign WHERE owner_account_id = $1 \
             AND status = 'draft' AND draft_expires_at_ms > $2 LIMIT $3) AS bounded",
            &[
                &owner.as_bytes().as_slice(),
                &to_i64(now_ms)?,
                &i64::from(cap),
            ],
        )
        .await?;
    let count: i64 = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
    u32::try_from(count).map_err(|_| StoreError::InvalidRecord)
}

/// Writes one step's columns, moves the draft to `new_step`, pushes the expiry out and bumps the
/// revision. The caller holds the draft locked.
pub(in super::super) async fn save_step(
    tx: &Tx<'_>,
    draft: &DraftRow,
    data: &DraftStepData,
    new_step: u8,
    expires_at_ms: u64,
) -> StoreResult<()> {
    let id = draft.id.as_bytes().to_vec();
    let step = i16::from(new_step);
    let expires = to_i64(expires_at_ms)?;
    match data {
        DraftStepData::About {
            name,
            adventure_kind,
            content_rating,
            seat_count,
        } => {
            tx.execute(
                "UPDATE campaigns.campaign SET name = $2, adventure_kind = $3, \
                 content_rating = $4, seat_count = $5, draft_step = $6, draft_expires_at_ms = $7, \
                 revision = revision + 1 WHERE campaign_id = $1 AND status = 'draft'",
                &[
                    &id,
                    &name.trim(),
                    adventure_kind,
                    &content_rating_text(*content_rating),
                    &to_i32(*seat_count)?,
                    &step,
                    &expires,
                ],
            )
            .await?;
        }
        DraftStepData::Table {
            voice_default,
            picture_default,
            spend_preset,
            custom_hourly_micro_dollars,
            session_ceiling_micro_dollars,
        } => {
            tx.execute(
                "UPDATE campaigns.campaign SET voice_default = $2, picture_default = $3, \
                 spend_preset = $4, custom_hourly_micro_dollars = $5, \
                 session_ceiling_micro_dollars = $6, draft_step = $7, draft_expires_at_ms = $8, \
                 revision = revision + 1 WHERE campaign_id = $1 AND status = 'draft'",
                &[
                    &id,
                    &voice_default.text(),
                    &picture_default.text(),
                    &spend_preset.text(),
                    &custom_hourly_micro_dollars.map(to_i64).transpose()?,
                    &session_ceiling_micro_dollars.map(to_i64).transpose()?,
                    &step,
                    &expires,
                ],
            )
            .await?;
        }
        DraftStepData::Joining {
            default_co_host_delegations,
            room_codes,
            funding_model,
        } => {
            let sponsor = df_campaign::draft::sponsor_of(*funding_model, draft.owner);
            tx.execute(
                "UPDATE campaigns.campaign SET default_co_host_delegations = $2, room_codes = $3, \
                 funding_model = $4, sponsor_account_id = $5, draft_step = $6, \
                 draft_expires_at_ms = $7, revision = revision + 1 \
                 WHERE campaign_id = $1 AND status = 'draft'",
                &[
                    &id,
                    &i16::from(default_co_host_delegations.bits()),
                    &room_codes.text(),
                    &funding_model.text(),
                    &sponsor.map(|account| account.as_bytes().to_vec()),
                    &step,
                    &expires,
                ],
            )
            .await?;
        }
        // The hero choice has no column yet (TODO(S1-09)) and the summary saves nothing; both
        // only move the draft along.
        DraftStepData::Hero(_) | DraftStepData::Review => {
            tx.execute(
                "UPDATE campaigns.campaign SET draft_step = $2, draft_expires_at_ms = $3, \
                 revision = revision + 1 WHERE campaign_id = $1 AND status = 'draft'",
                &[&id, &step, &expires],
            )
            .await?;
        }
    }
    Ok(())
}

/// Flips a locked draft to an active campaign.
pub(in super::super) async fn activate_draft(tx: &Tx<'_>, draft: CampaignId) -> StoreResult<()> {
    tx.execute(
        "UPDATE campaigns.campaign SET status = 'active', draft_step = NULL, \
         draft_expires_at_ms = NULL, revision = revision + 1 \
         WHERE campaign_id = $1 AND status = 'draft'",
        &[&draft.as_bytes().as_slice()],
    )
    .await?;
    Ok(())
}

/// Deletes every draft of an account (closure) and returns how many went.
pub(in super::super) async fn delete_drafts_of(tx: &Tx<'_>, owner: AccountId) -> StoreResult<u64> {
    tx.execute(
        "DELETE FROM campaigns.campaign WHERE owner_account_id = $1 AND status = 'draft'",
        &[&owner.as_bytes().as_slice()],
    )
    .await
}

/// Deletes drafts whose expiry has passed, at most `limit` per pass, and returns how many went.
pub(in super::super) async fn delete_expired_drafts(
    tx: &Tx<'_>,
    now_ms: u64,
    limit: u32,
) -> StoreResult<u64> {
    tx.execute(
        "DELETE FROM campaigns.campaign WHERE campaign_id IN (\
            SELECT campaign_id FROM campaigns.campaign WHERE status = 'draft' \
            AND draft_expires_at_ms <= $1 ORDER BY draft_expires_at_ms LIMIT $2 \
            FOR UPDATE SKIP LOCKED)",
        &[&to_i64(now_ms)?, &i64::from(limit)],
    )
    .await
}
