//! Transfer proposals.

use super::prelude::*;

pub(in super::super) async fn proposal_by_id(
    tx: &Tx<'_>,
    campaign: CampaignId,
    proposal: ProposalId,
) -> StoreResult<Option<ProposalRow>> {
    let sql = format!(
        "SELECT {PROPOSAL_COLUMNS} FROM campaigns.transfer_proposal \
         WHERE campaign_id = $1 AND proposal_id = $2"
    );
    tx.query_opt(
        &sql,
        &[
            &campaign.as_bytes().as_slice(),
            &proposal.as_bytes().as_slice(),
        ],
    )
    .await?
    .map(|row| proposal_from_row(&row))
    .transpose()
}

pub(in super::super) async fn insert_proposal(tx: &Tx<'_>, row: &ProposalRow) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO campaigns.transfer_proposal (proposal_id, campaign_id, kind, from_member, \
         from_account, to_member, to_account, old_host_becomes, campaign_revision, \
         proposed_at_ms, expires_at_ms, state) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        &[
            &row.id.as_bytes().as_slice(),
            &row.campaign.as_bytes().as_slice(),
            &row.kind.text(),
            &row.from_member.as_bytes().as_slice(),
            &row.from_account.as_bytes().as_slice(),
            &row.to_member.as_bytes().as_slice(),
            &row.to_account.as_bytes().as_slice(),
            &row.old_host_becomes.map(|becomes| match becomes {
                df_campaign::records::OldHostBecomes::Player => "player",
                df_campaign::records::OldHostBecomes::CoHost => "co_host",
            }),
            &to_i64(row.campaign_revision)?,
            &to_i64(row.proposed_at_ms)?,
            &to_i64(row.expires_at_ms)?,
            &row.state.text(),
        ],
    )
    .await?;
    Ok(())
}

pub(in super::super) async fn set_proposal_state(
    tx: &Tx<'_>,
    proposal: ProposalId,
    state: df_campaign::records::ProposalState,
) -> StoreResult<()> {
    tx.execute(
        "UPDATE campaigns.transfer_proposal SET state = $2 WHERE proposal_id = $1",
        &[&proposal.as_bytes().as_slice(), &state.text()],
    )
    .await?;
    Ok(())
}

/// The campaigns that hold a `proposed` row made by this account (for the closure lock set).
pub(in super::super) async fn campaigns_with_proposals_from(
    tx: &Tx<'_>,
    account: AccountId,
) -> StoreResult<Vec<CampaignId>> {
    let rows = tx
        .query(
            "SELECT DISTINCT campaign_id FROM campaigns.transfer_proposal \
             WHERE from_account = $1 AND state = 'proposed' ORDER BY campaign_id",
            &[&account.as_bytes().as_slice()],
        )
        .await?;
    rows.iter()
        .map(|row| {
            let bytes: &[u8] = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
            super::super::rows::campaign_id(bytes)
        })
        .collect()
}

/// Cancels the account's `proposed` rows in the given (already locked) campaigns.
pub(in super::super) async fn cancel_proposals_from(
    tx: &Tx<'_>,
    account: AccountId,
    campaigns: &[CampaignId],
) -> StoreResult<u64> {
    let ids: Vec<Vec<u8>> = campaigns.iter().map(|id| id.as_bytes().to_vec()).collect();
    tx.execute(
        "UPDATE campaigns.transfer_proposal SET state = 'cancelled' \
         WHERE from_account = $1 AND state = 'proposed' AND campaign_id = ANY($2)",
        &[&account.as_bytes().as_slice(), &ids],
    )
    .await
}

/// Cancels the `proposed` rows of a campaign, optionally only one kind, optionally sparing one.
pub(in super::super) async fn cancel_proposals_in(
    tx: &Tx<'_>,
    campaign: CampaignId,
    kind: Option<df_campaign::records::TransferKind>,
    except: Option<ProposalId>,
) -> StoreResult<u64> {
    let kind = kind.map(|kind| kind.text());
    let except = except.map(|id| id.as_bytes().to_vec());
    tx.execute(
        "UPDATE campaigns.transfer_proposal SET state = 'cancelled' \
         WHERE campaign_id = $1 AND state = 'proposed' \
         AND ($2::text IS NULL OR kind = $2) AND ($3::bytea IS NULL OR proposal_id <> $3)",
        &[&campaign.as_bytes().as_slice(), &kind, &except],
    )
    .await
}
