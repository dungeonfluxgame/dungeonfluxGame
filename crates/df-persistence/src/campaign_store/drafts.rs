//! The create wizard on PostgreSQL (8.1): starting a draft, saving a step, resuming and
//! finishing. A draft is a `campaign` row in the `draft` state with no members and no events, so
//! nothing here moves the event counter until `finish` appends the creator's `joined`.
//!
//! Lock order: the owner's account row (B's share lock, which also refuses a closing account),
//! the account's creation lock, then the campaign row. Finishing takes the same locks in the same
//! order as `create_campaign`, so the cap on campaigns is exact.

use df_campaign::{
    config::Tunables,
    draft::{DraftRow, DraftStepData, may_finish, may_save, step_after},
    errors::{Failure, Outcome, Refusal, StoreError},
    ids::FeedEpoch,
    records::{AccountStatus, CampaignRow},
    secrets::random_identity,
};
use df_types::{AccountId, CampaignId, OperationId, TenantId};

use super::{
    campaigns::{existing_campaign, finish_creation},
    conn::Tx,
    invites::replay,
    queries as q,
    shared::refuse,
};

const CREATE_NAMESPACE: &str = "create_campaign";

async fn lock_active_owner(tx: &Tx<'_>, owner: AccountId) -> Outcome<()> {
    if q::lock_account(tx, owner).await? != AccountStatus::Active {
        return refuse(Refusal::AccountClosing);
    }
    Ok(())
}

pub(super) async fn start_draft(
    tx: &Tx<'_>,
    tunables: &Tunables,
    owner: AccountId,
    tenant: TenantId,
    now_ms: u64,
) -> Outcome<DraftRow> {
    lock_active_owner(tx, owner).await?;
    // The same per-account lock as a creation: the count below is exact.
    q::lock_campaign_creation(tx, owner).await?;
    if q::live_drafts_up_to(tx, owner, tunables.drafts_per_account, now_ms).await?
        >= tunables.drafts_per_account
    {
        return refuse(Refusal::DraftsFull);
    }
    let id_bytes = random_identity().map_err(|_| StoreError::RandomUnavailable)?;
    let id = CampaignId::from_bytes(&id_bytes).map_err(|_| StoreError::InvalidRecord)?;
    let epoch = FeedEpoch::generate().map_err(|_| StoreError::RandomUnavailable)?;
    let expires = now_ms.saturating_add(tunables.draft_retention_ms());
    q::insert_draft(tx, id, owner, tenant, epoch, now_ms, expires).await?;
    match q::read_draft(tx, id, owner, now_ms).await? {
        Some(draft) => Ok(draft),
        None => Err(Failure::Store(StoreError::InvalidRecord)),
    }
}

/// A foreign, finished or expired draft is `NotFound`, like a draft that never existed.
pub(super) async fn get_draft(
    tx: &Tx<'_>,
    owner: AccountId,
    draft: CampaignId,
    now_ms: u64,
) -> Outcome<DraftRow> {
    match q::read_draft(tx, draft, owner, now_ms).await? {
        Some(row) => Ok(row),
        None => refuse(Refusal::NotFound),
    }
}

pub(super) async fn save_draft_step(
    tx: &Tx<'_>,
    tunables: &Tunables,
    owner: AccountId,
    draft: CampaignId,
    data: DraftStepData,
    now_ms: u64,
) -> Outcome<DraftRow> {
    data.validate(tunables).map_err(Failure::Refused)?;
    lock_active_owner(tx, owner).await?;
    let Some(row) = q::lock_draft(tx, draft, owner, now_ms).await? else {
        return refuse(Refusal::NotFound);
    };
    if !may_save(row.step, data.number()) {
        return refuse(Refusal::InvalidInput);
    }
    let step = step_after(row.step, data.number());
    let expires = now_ms.saturating_add(tunables.draft_retention_ms());
    q::save_step(tx, &row, &data, step, expires).await?;
    match q::read_draft(tx, draft, owner, now_ms).await? {
        Some(saved) => Ok(saved),
        None => Err(Failure::Store(StoreError::InvalidRecord)),
    }
}

pub(super) async fn finish_draft(
    tx: &Tx<'_>,
    tunables: &Tunables,
    owner: AccountId,
    draft: CampaignId,
    operation: OperationId,
    fingerprint: [u8; 32],
    now_ms: u64,
) -> Outcome<CampaignRow> {
    if let Some(result) =
        replay(tx, CREATE_NAMESPACE, owner, operation, fingerprint, now_ms).await?
    {
        return existing_campaign(tx, &result).await;
    }
    lock_active_owner(tx, owner).await?;
    q::lock_campaign_creation(tx, owner).await?;
    if let Some(result) =
        replay(tx, CREATE_NAMESPACE, owner, operation, fingerprint, now_ms).await?
    {
        return existing_campaign(tx, &result).await;
    }
    let Some(row) = q::lock_draft(tx, draft, owner, now_ms).await? else {
        return refuse(Refusal::NotFound);
    };
    if !may_finish(row.step) {
        return refuse(Refusal::DraftIncomplete);
    }
    if q::owned_campaigns_up_to(tx, owner, tunables.campaigns_per_account).await?
        >= tunables.campaigns_per_account
    {
        return refuse(Refusal::TooManyCampaigns);
    }
    q::activate_draft(tx, draft).await?;
    finish_creation(tx, draft, owner, operation, fingerprint, now_ms).await
}
