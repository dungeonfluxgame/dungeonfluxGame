//! Quota and asset row locking in the D25 order, reservation release, the settlement outbox
//! insert and the terminal rejection that every quota-changing transition shares.

use super::rows::*;
use df_assets::{
    AssetId, AssetRecord, AttemptId, CatalogError, FinalizeOutcome, HeldStep, LeaseOutcome,
    QuotaScope, QuotaUsage, RejectReason,
};
use tokio_postgres::Transaction;

/// Creates the scope's usage row if absent, then locks it `FOR UPDATE` (10).
pub(crate) async fn lock_quota(
    tx: &Transaction<'_>,
    scope: QuotaScope,
) -> Result<QuotaUsage, CatalogError> {
    let (kind, id) = scope_columns(scope);
    tx.execute(
        "INSERT INTO media.quota_usage (scope_kind, scope_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
        &[&kind, &id],
    )
    .await
    .map_err(unavailable)?;
    let row = tx
        .query_one(
            "SELECT committed_bytes, reserved_bytes, in_flight_uploads, template_count \
             FROM media.quota_usage WHERE scope_kind = $1 AND scope_id = $2 FOR UPDATE",
            &[&kind, &id],
        )
        .await
        .map_err(unavailable)?;
    usage_of(&row)
}

pub(super) async fn adjust_usage(
    tx: &Transaction<'_>,
    scope: QuotaScope,
    reserved_delta: i64,
    committed_delta: i64,
    uploads_delta: i32,
) -> Result<(), CatalogError> {
    let (kind, id) = scope_columns(scope);
    tx.execute(
        "UPDATE media.quota_usage SET reserved_bytes = reserved_bytes + $3, \
         committed_bytes = committed_bytes + $4, in_flight_uploads = in_flight_uploads + $5 \
         WHERE scope_kind = $1 AND scope_id = $2",
        &[
            &kind,
            &id,
            &reserved_delta,
            &committed_delta,
            &uploads_delta,
        ],
    )
    .await
    .map_err(unavailable)?;
    Ok(())
}

pub(crate) async fn quota_usage_in(
    tx: &Transaction<'_>,
    scope: QuotaScope,
) -> Result<QuotaUsage, CatalogError> {
    let (kind, id) = scope_columns(scope);
    let row = tx
        .query_opt(
            "SELECT committed_bytes, reserved_bytes, in_flight_uploads, template_count \
             FROM media.quota_usage WHERE scope_kind = $1 AND scope_id = $2",
            &[&kind, &id],
        )
        .await
        .map_err(unavailable)?;
    row.as_ref().map_or(Ok(QuotaUsage::default()), usage_of)
}

/// The asset's charge scope, read without a lock so the quota row can be locked first.
pub(super) async fn charge_scope_of(
    tx: &Transaction<'_>,
    asset: AssetId,
) -> Result<Option<QuotaScope>, CatalogError> {
    let row = tx
        .query_opt(
            "SELECT charge_scope_kind, charge_scope_id FROM media.asset WHERE asset_id = $1",
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    row.map(|row| {
        let kind: String = row.get(0);
        let id: Vec<u8> = row.get(1);
        scope_of(&kind, &id)
    })
    .transpose()
}

/// Quota row first, then the asset row `FOR UPDATE`; `None` when the asset does not exist.
pub(super) async fn lock_quota_then_asset(
    tx: &Transaction<'_>,
    asset: AssetId,
) -> Result<Option<(QuotaUsage, AssetRecord)>, CatalogError> {
    let Some(scope) = charge_scope_of(tx, asset).await? else {
        return Ok(None);
    };
    let usage = lock_quota(tx, scope).await?;
    let record = lock_asset(tx, asset).await?;
    match record {
        Some(record) if record.charge_scope != scope => Err(CatalogError::ScopeMoved),
        Some(record) => Ok(Some((usage, record))),
        None => Ok(None),
    }
}

pub(super) async fn lock_asset(
    tx: &Transaction<'_>,
    asset: AssetId,
) -> Result<Option<AssetRecord>, CatalogError> {
    let row = tx
        .query_opt(
            &format!("SELECT {ASSET_COLUMNS} FROM media.asset WHERE asset_id = $1 FOR UPDATE"),
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    row.as_ref().map(record_of).transpose()
}

/// Drops the asset's reservation and gives its bytes (and upload slot) back to the scope. The
/// caller holds the quota row lock.
pub(super) async fn release_reservation(
    tx: &Transaction<'_>,
    asset: AssetId,
) -> Result<u64, CatalogError> {
    let row = tx
        .query_opt(
            "DELETE FROM media.quota_reservation WHERE asset_id = $1 \
             RETURNING scope_kind, scope_id, reserved_bytes, counts_upload",
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    let Some(row) = row else {
        return Ok(0);
    };
    let kind: String = row.get("scope_kind");
    let id: Vec<u8> = row.get("scope_id");
    let reserved: i64 = row.get("reserved_bytes");
    let counts_upload: bool = row.get("counts_upload");
    let uploads_delta = if counts_upload { -1 } else { 0 };
    adjust_usage(tx, scope_of(&kind, &id)?, -reserved, 0, uploads_delta).await?;
    to_u64(reserved)
}

pub(super) async fn insert_outbox(
    tx: &Transaction<'_>,
    attempt: AttemptId,
    asset: AssetId,
    outcome: FinalizeOutcome,
    now_ms: i64,
) -> Result<(), CatalogError> {
    let reason = outcome.reason().map(RejectReason::label);
    tx.execute(
        "INSERT INTO media.settlement_outbox (attempt, asset_id, outcome, reason, at_ms) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT (attempt) DO NOTHING",
        &[
            &attempt.as_bytes().as_slice(),
            &asset.as_bytes().as_slice(),
            &outcome.label(),
            &reason,
            &now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    Ok(())
}

/// The terminal rejection with the quota and asset rows already locked.
pub(super) async fn reject_locked(
    tx: &Transaction<'_>,
    record: &AssetRecord,
    reason: RejectReason,
    now_ms: i64,
) -> Result<FinalizeOutcome, CatalogError> {
    release_reservation(tx, record.asset).await?;
    tx.execute(
        "UPDATE media.asset SET state = 'rejected', reject_reason = $2, step = NULL, \
         step_owner = NULL, step_lease_until_ms = NULL, updated_at_ms = $3 WHERE asset_id = $1",
        &[
            &record.asset.as_bytes().as_slice(),
            &reason.label(),
            &now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    let outcome = FinalizeOutcome::of_rejection(reason);
    insert_outbox(tx, record.attempt, record.asset, outcome, now_ms).await?;
    Ok(outcome)
}

/// Why a held step no longer applies: another attempt or an ended row is `Superseded`, a newer claim
/// on the same attempt is `LeaseLost`.
pub(super) async fn loss_of(
    tx: &Transaction<'_>,
    held: HeldStep,
) -> Result<LeaseOutcome, CatalogError> {
    let row = tx
        .query_opt(
            "SELECT attempt, state FROM media.asset WHERE asset_id = $1",
            &[&held.asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    let Some(row) = row else {
        return Ok(LeaseOutcome::Superseded);
    };
    let attempt: Vec<u8> = row.get(0);
    let state: String = row.get(1);
    Ok(
        if attempt == held.attempt.as_bytes() && state != "rejected" {
            LeaseOutcome::LeaseLost
        } else {
            LeaseOutcome::Superseded
        },
    )
}
