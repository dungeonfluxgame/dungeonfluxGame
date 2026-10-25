//! Step leases, the staged object and variant write bookkeeping: every statement that decides
//! whether a worker may still write.

use super::locks::*;
use super::rows::*;
use df_assets::{
    AssetId, AssetState, AttemptId, CatalogError, ClaimOutcome, HeldStep, LeaseOutcome,
    StageOutcome, StagedObjectRecord, StepClaim, StorageDomain, VariantIntent, VariantWritten,
};
use tokio_postgres::Transaction;

pub(crate) async fn claim_step_in(
    tx: &Transaction<'_>,
    claim: StepClaim,
    now_ms: i64,
) -> Result<ClaimOutcome, CatalogError> {
    let updated = tx
        .execute(
            "UPDATE media.asset SET step = $4, step_owner = $5, step_lease_until_ms = $6, \
             updated_at_ms = $7 WHERE asset_id = $1 AND attempt = $2 AND state = $3 \
             AND (step_owner IS NULL OR step_lease_until_ms < $7)",
            &[
                &claim.asset.as_bytes().as_slice(),
                &claim.attempt.as_bytes().as_slice(),
                &claim.expected_state.label(),
                &claim.step.label(),
                &claim.owner.as_bytes().as_slice(),
                &claim.lease_until_ms,
                &now_ms,
            ],
        )
        .await
        .map_err(unavailable)?;
    if updated == 1 {
        return Ok(ClaimOutcome::Claimed);
    }
    let row = tx
        .query_opt(
            "SELECT attempt, state, step_lease_until_ms FROM media.asset WHERE asset_id = $1",
            &[&claim.asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    let Some(row) = row else {
        return Ok(ClaimOutcome::Stale);
    };
    let attempt: Vec<u8> = row.get(0);
    let state: String = row.get(1);
    let until: Option<i64> = row.get(2);
    Ok(match until {
        Some(until_ms)
            if attempt == claim.attempt.as_bytes() && state == claim.expected_state.label() =>
        {
            ClaimOutcome::Held { until_ms }
        }
        _ => ClaimOutcome::Stale,
    })
}

pub(crate) async fn renew_step_in(
    tx: &Transaction<'_>,
    held: HeldStep,
    lease_until_ms: i64,
    now_ms: i64,
) -> Result<LeaseOutcome, CatalogError> {
    let updated = tx
        .execute(
            &format!(
                "UPDATE media.asset SET step_lease_until_ms = $4, updated_at_ms = $5 \
                 WHERE asset_id = $1 AND attempt = $2 AND step_owner = $3 \
                 AND state IN {OPEN_STATES}"
            ),
            &[
                &held.asset.as_bytes().as_slice(),
                &held.attempt.as_bytes().as_slice(),
                &held.owner.as_bytes().as_slice(),
                &lease_until_ms,
                &now_ms,
            ],
        )
        .await
        .map_err(unavailable)?;
    if updated == 1 {
        return Ok(LeaseOutcome::Done);
    }
    loss_of(tx, held).await
}

pub(crate) async fn record_staged_in(
    tx: &Transaction<'_>,
    asset: AssetId,
    attempt: AttemptId,
    staged: &StagedObjectRecord,
    now_ms: i64,
) -> Result<StageOutcome, CatalogError> {
    if staged.key.domain() != StorageDomain::Quarantine {
        return Err(CatalogError::InvalidRecord);
    }
    let length = to_i64(staged.manifest.byte_len)?;
    let updated = tx
        .execute(
            "UPDATE media.asset SET staged_key = $3, staged_version = $4, \
             staged_byte_length = $5, staged_sha256 = $6, staged_at_ms = $7, \
             state = 'quarantined', updated_at_ms = $8 \
             WHERE asset_id = $1 AND attempt = $2 AND state = 'uploading'",
            &[
                &asset.as_bytes().as_slice(),
                &attempt.as_bytes().as_slice(),
                &staged.key.path(),
                &staged.version.as_str(),
                &length,
                &staged.manifest.sha256.as_slice(),
                &staged.staged_at_ms,
                &now_ms,
            ],
        )
        .await
        .map_err(unavailable)?;
    if updated == 1 {
        return Ok(StageOutcome::Staged);
    }
    let row = tx
        .query_opt(
            "SELECT attempt, state FROM media.asset WHERE asset_id = $1",
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    let Some(row) = row else {
        return Ok(StageOutcome::NotCurrent);
    };
    let current: Vec<u8> = row.get(0);
    let state: String = row.get(1);
    if current != attempt.as_bytes() {
        return Ok(StageOutcome::NotCurrent);
    }
    Ok(match state.as_str() {
        "quarantined" | "validated" => StageOutcome::AlreadyStaged,
        other => StageOutcome::NotAccepting {
            state: AssetState::from_label(other).ok_or(CatalogError::InvalidRecord)?,
        },
    })
}

pub(crate) async fn mark_validated_in(
    tx: &Transaction<'_>,
    held: HeldStep,
    now_ms: i64,
) -> Result<LeaseOutcome, CatalogError> {
    let updated = tx
        .execute(
            "UPDATE media.asset SET state = 'validated', step = 'publish', updated_at_ms = $4 \
             WHERE asset_id = $1 AND attempt = $2 AND step_owner = $3 \
             AND state IN ('quarantined', 'validated')",
            &[
                &held.asset.as_bytes().as_slice(),
                &held.attempt.as_bytes().as_slice(),
                &held.owner.as_bytes().as_slice(),
                &now_ms,
            ],
        )
        .await
        .map_err(unavailable)?;
    if updated == 1 {
        return Ok(LeaseOutcome::Done);
    }
    loss_of(tx, held).await
}

/// Records the intent to write, conditional on the live lease. The asset row is read `FOR SHARE`,
/// so a concurrent finalize or supersede (which holds it `FOR UPDATE`) is waited for and its
/// result seen.
pub(crate) async fn intend_variant_in(
    tx: &Transaction<'_>,
    held: HeldStep,
    intent: &VariantIntent,
    now_ms: i64,
) -> Result<LeaseOutcome, CatalogError> {
    let allowed = tx
        .query_opt(
            "SELECT 1 FROM media.asset WHERE asset_id = $1 AND attempt = $2 AND step_owner = $3 \
             AND step_lease_until_ms > $4 AND state = 'validated' FOR SHARE",
            &[
                &held.asset.as_bytes().as_slice(),
                &held.attempt.as_bytes().as_slice(),
                &held.owner.as_bytes().as_slice(),
                &now_ms,
            ],
        )
        .await
        .map_err(unavailable)?;
    if allowed.is_none() {
        return loss_of(tx, held).await;
    }
    tx.execute(
        "INSERT INTO media.asset_variant (asset_id, attempt, write_id, variant, owner, state, \
         object_domain, object_key, created_at_ms) \
         VALUES ($1, $2, $3, $4, $5, 'intended', $6, $7, $8) ON CONFLICT DO NOTHING",
        &[
            &held.asset.as_bytes().as_slice(),
            &held.attempt.as_bytes().as_slice(),
            &intent.write.as_bytes().as_slice(),
            &intent.variant.label(),
            &held.owner.as_bytes().as_slice(),
            &intent.key.domain().label(),
            &intent.key.path(),
            &now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    Ok(LeaseOutcome::Done)
}

pub(crate) async fn mark_variant_written_in(
    tx: &Transaction<'_>,
    asset: AssetId,
    attempt: AttemptId,
    written: &VariantWritten,
) -> Result<bool, CatalogError> {
    let length = to_i64(written.manifest.byte_len)?;
    let updated = tx
        .execute(
            "UPDATE media.asset_variant SET state = 'written', object_version = $5, \
             byte_length = $6, sha256 = $7 WHERE asset_id = $1 AND attempt = $2 \
             AND write_id = $3 AND variant = $4 AND state = 'intended'",
            &[
                &asset.as_bytes().as_slice(),
                &attempt.as_bytes().as_slice(),
                &written.write.as_bytes().as_slice(),
                &written.variant.label(),
                &written.version.as_str(),
                &length,
                &written.manifest.sha256.as_slice(),
            ],
        )
        .await
        .map_err(unavailable)?;
    Ok(updated == 1)
}
