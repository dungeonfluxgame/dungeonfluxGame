//! The transitions that end or complete an attempt: finalize, reject, supersede and tombstone.

use super::locks::*;
use super::rows::*;
use df_assets::{
    AssetId, AssetState, AssetVariant, AttemptId, CatalogError, FinalizeOutcome, FinalizeRequest,
    FinalizeResult, HeldStep, Provenance, RejectGuard, RejectReason, RejectResult, SupersedeResult,
    TombstoneReason, TombstoneResult, WriteId,
};
use tokio_postgres::Transaction;

/// `Validated` to `Ready`: quota row, then asset row `FOR UPDATE`, every winner checked `written`.
pub(crate) async fn finalize_in(
    tx: &Transaction<'_>,
    held: HeldStep,
    request: &FinalizeRequest,
    now_ms: i64,
) -> Result<FinalizeResult, CatalogError> {
    let Some((usage, record)) = lock_quota_then_asset(tx, held.asset).await? else {
        return Ok(FinalizeResult::Superseded);
    };
    let winner_writes: Vec<WriteId> = request.winners.iter().map(|winner| winner.write).collect();
    if record.attempt == held.attempt
        && record.state == AssetState::Ready
        && record.winning_writes == winner_writes
    {
        let committed = record.published.as_ref().map_or(0, |facts| facts.byte_len);
        return Ok(FinalizeResult::Ready {
            committed_bytes: committed,
        });
    }
    if record.attempt != held.attempt || record.state == AssetState::Rejected {
        return Ok(FinalizeResult::Superseded);
    }
    if record.state != AssetState::Validated
        || record.lease.is_none_or(|lease| lease.owner != held.owner)
    {
        return Ok(FinalizeResult::LeaseLost);
    }
    let written = written_winners(tx, held, request).await?;
    let totals = match written {
        Winners::Missing(variant) => return Ok(FinalizeResult::WinnerMissing { variant }),
        Winners::Present(totals) => totals,
    };
    let reserved_bytes = reservation_bytes(tx, held.asset).await?;
    // Generated output within the kind maximum always commits (D21: any excess is company loss);
    // only an upload can be refused for room.
    let is_generated = record.provenance == Provenance::Generated;
    if !is_generated
        && totals.bytes > reserved_bytes
        && usage.committed_bytes.saturating_add(totals.bytes) > request.byte_limit
    {
        let outcome = reject_locked(tx, &record, RejectReason::QuotaExceeded, now_ms).await?;
        return Ok(FinalizeResult::Rejected(outcome));
    }
    release_reservation(tx, held.asset).await?;
    adjust_usage(tx, record.charge_scope, 0, to_i64(totals.bytes)?, 0).await?;
    let writes: Vec<Vec<u8>> = winner_writes
        .iter()
        .map(|write| write.as_bytes().to_vec())
        .collect();
    let width = request
        .width_px
        .map(i32::try_from)
        .transpose()
        .map_err(|_| CatalogError::InvalidRecord)?;
    let height = request
        .height_px
        .map(i32::try_from)
        .transpose()
        .map_err(|_| CatalogError::InvalidRecord)?;
    tx.execute(
        "UPDATE media.asset SET state = 'ready', step = NULL, step_owner = NULL, \
         step_lease_until_ms = NULL, ready_at_ms = $2, updated_at_ms = $2, winning_writes = $3, \
         mime = $4, byte_length = $5, sha256 = $6, width_px = $7, height_px = $8 \
         WHERE asset_id = $1",
        &[
            &held.asset.as_bytes().as_slice(),
            &now_ms,
            &writes,
            &request.mime,
            &to_i64(totals.bytes)?,
            &totals.master_sha256.as_slice(),
            &width,
            &height,
        ],
    )
    .await
    .map_err(unavailable)?;
    insert_outbox(tx, held.attempt, held.asset, FinalizeOutcome::Ready, now_ms).await?;
    Ok(FinalizeResult::Ready {
        committed_bytes: totals.bytes,
    })
}

pub(super) struct WinnerTotals {
    bytes: u64,
    master_sha256: [u8; 32],
}

pub(super) enum Winners {
    Present(WinnerTotals),
    Missing(AssetVariant),
}

pub(super) async fn written_winners(
    tx: &Transaction<'_>,
    held: HeldStep,
    request: &FinalizeRequest,
) -> Result<Winners, CatalogError> {
    let rows = tx
        .query(
            "SELECT write_id, variant, state, byte_length, sha256 FROM media.asset_variant \
             WHERE asset_id = $1 AND attempt = $2",
            &[
                &held.asset.as_bytes().as_slice(),
                &held.attempt.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(unavailable)?;
    let mut bytes = 0_u64;
    let mut master_sha256 = None;
    for winner in &request.winners {
        let found = rows.iter().find(|row| {
            row.get::<_, Vec<u8>>("write_id") == winner.write.as_bytes()
                && row.get::<_, String>("variant") == winner.variant.label()
                && row.get::<_, String>("state") == "written"
        });
        let Some(row) = found else {
            return Ok(Winners::Missing(winner.variant));
        };
        bytes = bytes
            .checked_add(to_u64(row.get("byte_length"))?)
            .ok_or(CatalogError::InvalidRecord)?;
        if winner.variant == AssetVariant::Master {
            let sha256: Vec<u8> = row.get("sha256");
            master_sha256 =
                Some(<[u8; 32]>::try_from(sha256).map_err(|_| CatalogError::InvalidRecord)?);
        }
    }
    Ok(match master_sha256 {
        Some(master_sha256) => Winners::Present(WinnerTotals {
            bytes,
            master_sha256,
        }),
        None => Winners::Missing(AssetVariant::Master),
    })
}

pub(super) async fn reservation_bytes(
    tx: &Transaction<'_>,
    asset: AssetId,
) -> Result<u64, CatalogError> {
    let row = tx
        .query_opt(
            "SELECT reserved_bytes FROM media.quota_reservation WHERE asset_id = $1",
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    row.map_or(Ok(0), |row| to_u64(row.get(0)))
}

pub(crate) async fn reject_in(
    tx: &Transaction<'_>,
    asset: AssetId,
    attempt: AttemptId,
    reason: RejectReason,
    guard: RejectGuard,
    now_ms: i64,
) -> Result<RejectResult, CatalogError> {
    let Some((_, record)) = lock_quota_then_asset(tx, asset).await? else {
        return Ok(RejectResult::NotCurrent);
    };
    if record.attempt != attempt {
        return Ok(RejectResult::NotCurrent);
    }
    if !matches!(
        record.state,
        AssetState::Uploading | AssetState::Quarantined | AssetState::Validated
    ) {
        return Ok(RejectResult::AlreadyEnded {
            state: record.state,
        });
    }
    let is_held = record.lease.is_some_and(|lease| lease.until_ms >= now_ms);
    let is_satisfied = match guard {
        RejectGuard::Current => true,
        RejectGuard::WhileUploading => record.state == AssetState::Uploading,
        RejectGuard::WhileUnleased => !is_held,
    };
    if !is_satisfied {
        return Ok(RejectResult::GuardFailed {
            state: record.state,
        });
    }
    Ok(RejectResult::Rejected(
        reject_locked(tx, &record, reason, now_ms).await?,
    ))
}

pub(crate) async fn supersede_in(
    tx: &Transaction<'_>,
    asset: AssetId,
    from: AttemptId,
    to: AttemptId,
    now_ms: i64,
) -> Result<SupersedeResult, CatalogError> {
    let Some((_, record)) = lock_quota_then_asset(tx, asset).await? else {
        return Ok(SupersedeResult::NotCurrent);
    };
    let can_move = record.attempt == from
        && matches!(
            record.state,
            AssetState::Uploading | AssetState::Quarantined | AssetState::Validated
        );
    if !can_move {
        return Ok(SupersedeResult::NotCurrent);
    }
    if let Some(staged) = &record.staged {
        tx.execute(
            "INSERT INTO media.staged_orphan (asset_id, attempt, object_key, object_version, \
             superseded_at_ms) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
            &[
                &asset.as_bytes().as_slice(),
                &from.as_bytes().as_slice(),
                &staged.key.path(),
                &staged.version.as_str(),
                &now_ms,
            ],
        )
        .await
        .map_err(unavailable)?;
    }
    tx.execute(
        "UPDATE media.asset SET attempt = $3, state = 'uploading', step = NULL, \
         step_owner = NULL, step_lease_until_ms = NULL, staged_key = NULL, staged_version = NULL, \
         staged_byte_length = NULL, staged_sha256 = NULL, staged_at_ms = NULL, \
         staged_deleted_at_ms = NULL, updated_at_ms = $4 WHERE asset_id = $1 AND attempt = $2",
        &[
            &asset.as_bytes().as_slice(),
            &from.as_bytes().as_slice(),
            &to.as_bytes().as_slice(),
            &now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    insert_outbox(tx, from, asset, FinalizeOutcome::Superseded, now_ms).await?;
    Ok(SupersedeResult::Superseded)
}

pub(crate) async fn tombstone_in(
    tx: &Transaction<'_>,
    asset: AssetId,
    reason: TombstoneReason,
    now_ms: i64,
) -> Result<TombstoneResult, CatalogError> {
    let Some((_, record)) = lock_quota_then_asset(tx, asset).await? else {
        return Ok(TombstoneResult::NotFound);
    };
    if !matches!(record.state, AssetState::Ready | AssetState::Archived) {
        return Ok(TombstoneResult::NotDeletable {
            state: record.state,
        });
    }
    let referenced = tx
        .query_opt(
            "SELECT 1 FROM media.asset_reference WHERE asset_id = $1 AND presentation = 'live' \
             LIMIT 1",
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    if referenced.is_some() {
        return Ok(TombstoneResult::LiveReferences);
    }
    let released = record.published.as_ref().map_or(0, |facts| facts.byte_len);
    adjust_usage(tx, record.charge_scope, 0, -to_i64(released)?, 0).await?;
    let generation = to_i64(record.suppression_generation.saturating_add(1))?;
    tx.execute(
        "UPDATE media.asset SET state = 'deleted', suppression_generation = $2, \
         updated_at_ms = $3 WHERE asset_id = $1",
        &[&asset.as_bytes().as_slice(), &generation, &now_ms],
    )
    .await
    .map_err(unavailable)?;
    tx.execute(
        "INSERT INTO media.tombstone (asset_id, suppression_generation, reason, at_ms) \
         VALUES ($1, $2, $3, $4)",
        &[
            &asset.as_bytes().as_slice(),
            &generation,
            &reason.label(),
            &now_ms,
        ],
    )
    .await
    .map_err(unavailable)?;
    Ok(TombstoneResult::Deleted {
        released_bytes: released,
    })
}
