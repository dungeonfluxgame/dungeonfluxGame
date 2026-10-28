//! Admission (reserving quota and inserting the row) and the lookups the hub reads.

use super::locks::*;
use super::rows::*;
use df_assets::{
    Admission, AdmitOutcome, AssetId, AssetRecord, CatalogError, CatalogLookup, DedupeScope,
    MediaRequestKey, NotAdmitted, QuotaRefusal, VariantRecord,
};
// Root wires the shared `df_assets` re-exports for these two helpers; the SQL adapter
// calls them by their final exported paths once `lib.rs` gains them.
// Expected `df_assets` additions: `admission_owner`, `record_owner`, `StorageOwner`.
use tokio_postgres::{Row, Transaction};

/// Reserves quota and inserts the row, inside the caller's transaction (F's admission calls this
/// after its own locks). A key conflict adds nothing and aborts nothing.
pub(crate) async fn admit_in(
    tx: &Transaction<'_>,
    admission: &Admission,
    now_ms: i64,
) -> Result<AdmitOutcome, CatalogError> {
    let incoming = match df_assets::admission_owner(admission) {
        Ok(owner) => owner,
        Err(reason) => return Ok(AdmitOutcome::NotAdmitted(reason)),
    };
    let usage = lock_quota(tx, admission.charge_scope).await?;
    let (scope_kind, scope_id) = scope_columns(admission.charge_scope);
    let audience = audience_columns(&admission.audience);
    let request_key = admission.request_key.map(|key| key.as_bytes().to_vec());
    let inserted = tx
        .query_opt(
            "INSERT INTO media.asset (asset_id, kind, provenance, charge_scope_kind, \
             charge_scope_id, origin_account, campaign_id, audience_kind, audience_campaign, \
             audience_account, audience_members, dedupe_scope, request_key, state, attempt, \
             retention_class, created_at_ms, updated_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'uploading', $14, \
             $15, $16, $16) \
             ON CONFLICT (dedupe_scope, request_key) WHERE request_key IS NOT NULL AND state IN \
             ('uploading', 'quarantined', 'validated', 'ready') DO NOTHING RETURNING asset_id",
            &[
                &admission.asset.as_bytes().as_slice(),
                &admission.kind.label(),
                &admission.provenance.label(),
                &scope_kind,
                &scope_id,
                &admission
                    .origin_account
                    .map(|account| account.as_bytes().to_vec()),
                &admission
                    .campaign
                    .map(|campaign| campaign.as_bytes().to_vec()),
                &audience.kind,
                &audience.campaign,
                &audience.account,
                &audience.members,
                &admission.dedupe_scope.as_bytes().as_slice(),
                &request_key,
                &admission.attempt.as_bytes().as_slice(),
                &admission.retention.label(),
                &now_ms,
            ],
        )
        .await
        .map_err(unavailable)?;
    if inserted.is_none() {
        return conflicting_row(tx, admission, incoming).await;
    }
    let held = usage.committed_bytes.checked_add(usage.reserved_bytes);
    let wanted = held.and_then(|held| held.checked_add(admission.reserve_bytes));
    let refusal = if admission
        .in_flight_upload_limit
        .is_some_and(|limit| usage.in_flight_uploads >= limit)
    {
        Some(NotAdmitted::UploadLimit)
    } else if wanted.is_none_or(|wanted| wanted > admission.byte_limit)
        || i64::try_from(admission.reserve_bytes).is_err()
    {
        Some(NotAdmitted::QuotaFull(QuotaRefusal {
            scope: admission.charge_scope,
            needed_bytes: admission.reserve_bytes,
            available_bytes: held.map_or(0, |held| admission.byte_limit.saturating_sub(held)),
        }))
    } else {
        None
    };
    if let Some(refusal) = refusal {
        // The row is ours and uncommitted: taking it back leaves nothing behind.
        tx.execute(
            "DELETE FROM media.asset WHERE asset_id = $1",
            &[&admission.asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
        return Ok(AdmitOutcome::NotAdmitted(refusal));
    }
    let counts_upload = admission.in_flight_upload_limit.is_some();
    let reserved = to_i64(admission.reserve_bytes)?;
    tx.execute(
        "INSERT INTO media.quota_reservation (asset_id, scope_kind, scope_id, reserved_bytes, \
         counts_upload) VALUES ($1, $2, $3, $4, $5)",
        &[
            &admission.asset.as_bytes().as_slice(),
            &scope_kind,
            &scope_id,
            &reserved,
            &counts_upload,
        ],
    )
    .await
    .map_err(unavailable)?;
    adjust_usage(
        tx,
        admission.charge_scope,
        reserved,
        0,
        i32::from(counts_upload),
    )
    .await?;
    Ok(AdmitOutcome::Admitted)
}

pub(super) async fn conflicting_row(
    tx: &Transaction<'_>,
    admission: &Admission,
    incoming: Option<df_assets::StorageOwner>,
) -> Result<AdmitOutcome, CatalogError> {
    let row = tx
        .query_opt(
            &format!(
                "SELECT {ASSET_COLUMNS} FROM media.asset WHERE dedupe_scope = $1 \
                 AND request_key = $2 AND state IN {LIVE_STATES}"
            ),
            &[
                &admission.dedupe_scope.as_bytes().as_slice(),
                &admission.request_key.map(|key| key.as_bytes().to_vec()),
            ],
        )
        .await
        .map_err(unavailable)?;
    // A conflict whose row vanished again (its transaction rolled back) is a transient failure.
    let row = row.ok_or(CatalogError::Unavailable)?;
    let existing = record_of(&row)?;
    let existing_owner = df_assets::record_owner(&existing);
    let draft_involved = matches!(
        incoming,
        Some(df_assets::StorageOwner::AccountDraft(_))
    ) || matches!(
        existing_owner,
        Some(df_assets::StorageOwner::AccountDraft(_))
    );
    if draft_involved && incoming != existing_owner {
        return Ok(AdmitOutcome::NotAdmitted(NotAdmitted::OwnerUnsupported));
    }
    Ok(AdmitOutcome::NotAdmitted(if existing.state == df_assets::AssetState::Ready {
        NotAdmitted::Ready {
            asset: existing.asset,
        }
    } else {
        NotAdmitted::InFlight {
            attempt: existing.attempt,
        }
    }))
}

pub(crate) async fn lookup_in(
    tx: &Transaction<'_>,
    scope: DedupeScope,
    key: &MediaRequestKey,
) -> Result<CatalogLookup, CatalogError> {
    let rows = tx
        .query(
            "SELECT asset_id, attempt, state FROM media.asset WHERE dedupe_scope = $1 \
             AND request_key = $2 ORDER BY updated_at_ms DESC",
            &[&scope.as_bytes().as_slice(), &key.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    let states: Vec<(String, Row)> = rows.into_iter().map(|row| (row.get(2), row)).collect();
    let live = states.iter().find(|(state, _)| {
        matches!(
            state.as_str(),
            "uploading" | "quarantined" | "validated" | "ready"
        )
    });
    if let Some((state, row)) = live {
        return Ok(if state == "ready" {
            CatalogLookup::Ready {
                asset: asset_of(&row.get::<_, Vec<u8>>(0))?,
            }
        } else {
            CatalogLookup::InFlight {
                attempt: attempt_of(&row.get::<_, Vec<u8>>(1))?,
            }
        });
    }
    if let Some((_, row)) = states.iter().find(|(state, _)| state == "archived") {
        return Ok(CatalogLookup::Archived {
            asset: asset_of(&row.get::<_, Vec<u8>>(0))?,
        });
    }
    let suppressed = states
        .iter()
        .any(|(state, _)| matches!(state.as_str(), "deleted" | "purging" | "purged"));
    Ok(if suppressed {
        CatalogLookup::Suppressed
    } else {
        CatalogLookup::Absent
    })
}

pub(crate) async fn asset_in(
    tx: &Transaction<'_>,
    asset: AssetId,
) -> Result<Option<AssetRecord>, CatalogError> {
    let row = tx
        .query_opt(
            &format!("SELECT {ASSET_COLUMNS} FROM media.asset WHERE asset_id = $1"),
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    row.as_ref().map(record_of).transpose()
}

pub(crate) async fn variants_in(
    tx: &Transaction<'_>,
    asset: AssetId,
) -> Result<Vec<VariantRecord>, CatalogError> {
    let rows = tx
        .query(
            &format!(
                "SELECT {VARIANT_COLUMNS} FROM media.asset_variant WHERE asset_id = $1 \
                 ORDER BY created_at_ms, write_id, variant"
            ),
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(unavailable)?;
    rows.iter().map(variant_of).collect()
}
