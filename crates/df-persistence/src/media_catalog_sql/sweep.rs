//! What the sweeper reads and changes: rows without a live lease, orphaned variant writes and
//! staged objects, and the settlement outbox.

use super::rows::*;
use df_assets::{
    AssetState, AttemptId, CatalogError, DeletionMark, DueRow, ObjectKey, ObjectVersion,
    OrphanStaged, OrphanVariant, OutboxEntry, StorageDomain, SweepRow, SweepWrite,
    is_write_deletable,
};
use tokio_postgres::Transaction;

pub(crate) async fn due_rows_in(
    tx: &Transaction<'_>,
    now_ms: i64,
    limit: usize,
) -> Result<Vec<DueRow>, CatalogError> {
    let limit = i64::try_from(limit).map_err(|_| CatalogError::InvalidRecord)?;
    let rows = tx
        .query(
            &format!(
                "SELECT {ASSET_COLUMNS} FROM media.asset WHERE state IN {OPEN_STATES} \
                 AND (step_owner IS NULL OR step_lease_until_ms < $1) \
                 ORDER BY updated_at_ms, asset_id LIMIT $2"
            ),
            &[&now_ms, &limit],
        )
        .await
        .map_err(unavailable)?;
    rows.iter()
        .map(|row| {
            let record = record_of(row)?;
            Ok(DueRow {
                asset: record.asset,
                attempt: record.attempt,
                state: record.state,
                provenance: record.provenance,
                staged: record.staged,
                created_at_ms: record.created_at_ms,
            })
        })
        .collect()
}

/// The coarse form of the 6.2 orphan rule; `begin_variant_deletion_in` re-evaluates the exact
/// rule under the asset row's share lock before marking anything.
pub(crate) async fn orphan_variants_in(
    tx: &Transaction<'_>,
    created_before_ms: i64,
    limit: usize,
) -> Result<Vec<OrphanVariant>, CatalogError> {
    let limit = i64::try_from(limit).map_err(|_| CatalogError::InvalidRecord)?;
    let rows = tx
        .query(
            "SELECT v.asset_id, v.attempt, v.write_id, v.variant, v.owner, v.state, \
             v.object_domain, v.object_key, v.object_version, v.byte_length, v.sha256, \
             v.created_at_ms \
             FROM media.asset_variant v JOIN media.asset a ON a.asset_id = v.asset_id \
             WHERE v.created_at_ms < $1 \
             AND NOT (a.ready_at_ms IS NOT NULL AND v.write_id = ANY(COALESCE(a.winning_writes, \
             ARRAY[]::bytea[]))) \
             AND (v.state = 'deleting' OR v.attempt <> a.attempt OR a.state = 'rejected' \
             OR a.ready_at_ms IS NOT NULL \
             OR v.owner IS DISTINCT FROM a.step_owner) \
             ORDER BY v.created_at_ms, v.write_id LIMIT $2",
            &[&created_before_ms, &limit],
        )
        .await
        .map_err(unavailable)?;
    rows.iter()
        .map(|row| {
            let record = variant_of(row)?;
            Ok(OrphanVariant {
                asset: record.asset,
                attempt: record.attempt,
                write: record.write,
                variant: record.variant,
                key: record.key,
                version: record.version,
            })
        })
        .collect()
}

pub(crate) async fn begin_variant_deletion_in(
    tx: &Transaction<'_>,
    orphan: &OrphanVariant,
) -> Result<DeletionMark, CatalogError> {
    let asset_row = tx
        .query_opt(
            "SELECT state, attempt, step_owner, winning_writes FROM media.asset \
             WHERE asset_id = $1 FOR SHARE",
            &[&orphan.asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    let variant_row = tx
        .query_opt(
            "SELECT owner, state FROM media.asset_variant WHERE asset_id = $1 AND attempt = $2 \
             AND write_id = $3 AND variant = $4 FOR UPDATE",
            &[
                &orphan.asset.as_bytes().as_slice(),
                &orphan.attempt.as_bytes().as_slice(),
                &orphan.write.as_bytes().as_slice(),
                &orphan.variant.label(),
            ],
        )
        .await
        .map_err(unavailable)?;
    let (Some(asset_row), Some(variant_row)) = (asset_row, variant_row) else {
        return Ok(DeletionMark::Gone);
    };
    let variant_state: String = variant_row.get("state");
    if variant_state == "deleting" {
        return Ok(DeletionMark::Marked);
    }
    let state_label: String = asset_row.get("state");
    let current_attempt: Vec<u8> = asset_row.get("attempt");
    let owner: Option<Vec<u8>> = asset_row.get("step_owner");
    let winners: Option<Vec<Vec<u8>>> = asset_row.get("winning_writes");
    let sweep_row = SweepRow {
        state: AssetState::from_label(&state_label).ok_or(CatalogError::InvalidRecord)?,
        current_attempt: attempt_of(&current_attempt)?,
        current_owner: owner.map(|bytes| claim_of(&bytes)).transpose()?,
    };
    let sweep_write = SweepWrite {
        attempt: orphan.attempt,
        owner: claim_of(&variant_row.get::<_, Vec<u8>>("owner"))?,
        is_winner: winners
            .unwrap_or_default()
            .iter()
            .any(|write| write == orphan.write.as_bytes()),
    };
    if !is_write_deletable(sweep_row, sweep_write) {
        return Ok(DeletionMark::NoLongerDeletable);
    }
    tx.execute(
        "UPDATE media.asset_variant SET state = 'deleting' WHERE asset_id = $1 AND attempt = $2 \
         AND write_id = $3 AND variant = $4 AND state IN ('intended', 'written')",
        &[
            &orphan.asset.as_bytes().as_slice(),
            &orphan.attempt.as_bytes().as_slice(),
            &orphan.write.as_bytes().as_slice(),
            &orphan.variant.label(),
        ],
    )
    .await
    .map_err(unavailable)?;
    Ok(DeletionMark::Marked)
}

pub(crate) async fn forget_variant_in(
    tx: &Transaction<'_>,
    orphan: &OrphanVariant,
) -> Result<(), CatalogError> {
    tx.execute(
        "DELETE FROM media.asset_variant WHERE asset_id = $1 AND attempt = $2 AND write_id = $3 \
         AND variant = $4",
        &[
            &orphan.asset.as_bytes().as_slice(),
            &orphan.attempt.as_bytes().as_slice(),
            &orphan.write.as_bytes().as_slice(),
            &orphan.variant.label(),
        ],
    )
    .await
    .map_err(unavailable)?;
    Ok(())
}

pub(crate) async fn orphan_staged_in(
    tx: &Transaction<'_>,
    limit: usize,
) -> Result<Vec<OrphanStaged>, CatalogError> {
    let limit = i64::try_from(limit).map_err(|_| CatalogError::InvalidRecord)?;
    let rows = tx
        .query(
            "SELECT asset_id, attempt, object_key, object_version FROM media.staged_orphan \
             UNION ALL \
             SELECT asset_id, attempt, staged_key, staged_version FROM media.asset \
             WHERE staged_key IS NOT NULL AND staged_deleted_at_ms IS NULL \
             AND (state = 'rejected' OR ready_at_ms IS NOT NULL) LIMIT $1",
            &[&limit],
        )
        .await
        .map_err(unavailable)?;
    rows.iter()
        .map(|row| {
            let key: String = row.get(2);
            let version: String = row.get(3);
            Ok(OrphanStaged {
                asset: asset_of(&row.get::<_, Vec<u8>>(0))?,
                attempt: attempt_of(&row.get::<_, Vec<u8>>(1))?,
                key: ObjectKey::from_stored(StorageDomain::Quarantine, &key)
                    .ok_or(CatalogError::InvalidRecord)?,
                version: ObjectVersion::new(version).ok_or(CatalogError::InvalidRecord)?,
            })
        })
        .collect()
}

pub(crate) async fn forget_staged_in(
    tx: &Transaction<'_>,
    orphan: &OrphanStaged,
    now_ms: i64,
) -> Result<(), CatalogError> {
    let (asset, attempt) = (
        orphan.asset.as_bytes().as_slice(),
        orphan.attempt.as_bytes().as_slice(),
    );
    tx.execute(
        "DELETE FROM media.staged_orphan WHERE asset_id = $1 AND attempt = $2",
        &[&asset, &attempt],
    )
    .await
    .map_err(unavailable)?;
    tx.execute(
        "UPDATE media.asset SET staged_deleted_at_ms = $3 WHERE asset_id = $1 AND attempt = $2 \
         AND staged_key IS NOT NULL",
        &[&asset, &attempt, &now_ms],
    )
    .await
    .map_err(unavailable)?;
    Ok(())
}

pub(crate) async fn pending_outbox_in(
    tx: &Transaction<'_>,
    limit: usize,
) -> Result<Vec<OutboxEntry>, CatalogError> {
    let limit = i64::try_from(limit).map_err(|_| CatalogError::InvalidRecord)?;
    let rows = tx
        .query(
            "SELECT outbox_seq, attempt, asset_id, outcome, reason, at_ms, consumed_at_ms \
             FROM media.settlement_outbox WHERE consumed_at_ms IS NULL \
             ORDER BY outbox_seq LIMIT $1",
            &[&limit],
        )
        .await
        .map_err(unavailable)?;
    rows.iter().map(outbox_of).collect()
}

pub(crate) async fn outbox_entry_in(
    tx: &Transaction<'_>,
    attempt: AttemptId,
) -> Result<Option<OutboxEntry>, CatalogError> {
    let row = tx
        .query_opt(
            "SELECT outbox_seq, attempt, asset_id, outcome, reason, at_ms, consumed_at_ms \
             FROM media.settlement_outbox WHERE attempt = $1",
            &[&attempt.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    row.as_ref().map(outbox_of).transpose()
}
