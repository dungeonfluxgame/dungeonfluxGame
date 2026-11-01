//! Immutable consumed catalog winners, retained independently of supplier usage.

use super::usage::{invalid, storage};
use df_imagery::execution::ExecutionScope;
use df_imagery::owner_scope_bytes;
use df_imagery::runtime::{RuntimeError, SourceReceipt};
use serde_json::{Value, json};
use tokio_postgres::Transaction;

fn encoded(receipts: &[SourceReceipt]) -> String {
    Value::Array(
        receipts
            .iter()
            .map(|receipt| {
                json!({
                    "asset": receipt.pin.asset.as_bytes(),
                    "digest": receipt.pin.digest.as_bytes(),
                    "attempt": receipt.attempt.as_bytes(),
                    "write": receipt.write.as_bytes(),
                    "version": receipt.version.as_str(),
                    "suppression_generation": receipt.suppression_generation,
                    "retention": receipt.retention.label(),
                })
            })
            .collect(),
    )
    .to_string()
}

async fn decode(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    text: &str,
) -> Result<Vec<SourceReceipt>, RuntimeError> {
    let job = crate::imagery_store::read_in(tx, &scope.owner, scope.job)
        .await
        .map_err(RuntimeError::Journal)?
        .ok_or(RuntimeError::InvalidRecord)?;
    let pins = df_imagery::runtime::retained_sources(&job)?;
    let value: Value = invalid(serde_json::from_str(text))?;
    let values = value
        .as_array()
        .filter(|values| values.len() == pins.len())
        .ok_or(RuntimeError::InvalidRecord)?;
    values
        .iter()
        .zip(pins)
        .map(|(value, pin)| {
            let object = value
                .as_object()
                .filter(|object| object.len() == 7)
                .ok_or(RuntimeError::InvalidRecord)?;
            let asset: Vec<u8> = invalid(serde_json::from_value(
                object
                    .get("asset")
                    .cloned()
                    .ok_or(RuntimeError::InvalidRecord)?,
            ))?;
            let digest: Vec<u8> = invalid(serde_json::from_value(
                object
                    .get("digest")
                    .cloned()
                    .ok_or(RuntimeError::InvalidRecord)?,
            ))?;
            if asset != pin.asset.as_bytes() || digest != pin.digest.as_bytes() {
                return Err(RuntimeError::InvalidRecord);
            }
            let attempt: Vec<u8> = invalid(serde_json::from_value(
                object
                    .get("attempt")
                    .cloned()
                    .ok_or(RuntimeError::InvalidRecord)?,
            ))?;
            let write: Vec<u8> = invalid(serde_json::from_value(
                object
                    .get("write")
                    .cloned()
                    .ok_or(RuntimeError::InvalidRecord)?,
            ))?;
            let version = object
                .get("version")
                .and_then(Value::as_str)
                .and_then(|version| df_assets::ObjectVersion::new(version.to_owned()))
                .ok_or(RuntimeError::InvalidRecord)?;
            let suppression_generation = object
                .get("suppression_generation")
                .and_then(Value::as_u64)
                .ok_or(RuntimeError::InvalidRecord)?;
            let retention = object
                .get("retention")
                .and_then(Value::as_str)
                .and_then(df_assets::RetentionClass::from_label)
                .ok_or(RuntimeError::InvalidRecord)?;
            Ok(SourceReceipt {
                pin,
                attempt: invalid(df_assets::AttemptId::from_bytes(&attempt))?,
                write: invalid(df_assets::WriteId::from_bytes(&write))?,
                version,
                suppression_generation,
                retention,
            })
        })
        .collect()
}

/// Reads the exact consumed-source snapshot; absence is distinct from an explicit empty snapshot.
pub async fn read_sources_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
) -> Result<Option<Vec<SourceReceipt>>, RuntimeError> {
    let row = tx.query_opt("SELECT receipts::text FROM imagery.image_dispatch_sources WHERE dispatch_id=$1 AND owner_scope=$2 AND job_id=$3 AND stage_id=$4", &[
        &scope.dispatch.as_bytes().as_slice(), &owner_scope_bytes(&scope.owner),
        &scope.job.as_bytes().as_slice(), &scope.stage.as_bytes().as_slice(),
    ]).await.map_err(storage)?;
    let Some(row) = row else { return Ok(None) };
    decode(tx, scope, &row.get::<_, String>(0)).await.map(Some)
}

/// Records source evidence before first send; exact replay cannot replace a consumed winner.
pub async fn record_sources_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
    receipts: &[SourceReceipt],
) -> Result<(), RuntimeError> {
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended(encode($1::bytea,'hex'),45))",
        &[&scope.dispatch.as_bytes().as_slice()],
    )
    .await
    .map_err(storage)?;
    if let Some(existing) = read_sources_in(tx, scope).await? {
        return if existing == receipts {
            Ok(())
        } else {
            Err(RuntimeError::InvalidRecord)
        };
    }
    let row = tx.query_opt("SELECT dispatch_state FROM imagery.image_dispatch_attempt d JOIN imagery.image_job j ON j.job_id=d.job_id WHERE d.dispatch_id=$1 AND j.owner_scope=$2 AND d.job_id=$3 AND d.stage_id=$4", &[
        &scope.dispatch.as_bytes().as_slice(), &owner_scope_bytes(&scope.owner),
        &scope.job.as_bytes().as_slice(), &scope.stage.as_bytes().as_slice(),
    ]).await.map_err(storage)?.ok_or(RuntimeError::InvalidRecord)?;
    if row.get::<_, String>(0) != "prepared" {
        return Err(RuntimeError::InvalidRecord);
    }
    let text = encoded(receipts);
    if decode(tx, scope, &text).await? != receipts {
        return Err(RuntimeError::InvalidRecord);
    }
    let digest = df_imagery::runtime::source_receipts_digest(receipts)?;
    tx.execute("INSERT INTO imagery.image_dispatch_sources(dispatch_id,owner_scope,job_id,stage_id,receipts,source_digest) VALUES($1,$2,$3,$4,$5::text::jsonb,$6)", &[
        &scope.dispatch.as_bytes().as_slice(), &owner_scope_bytes(&scope.owner),
        &scope.job.as_bytes().as_slice(), &scope.stage.as_bytes().as_slice(), &text, &digest.as_slice(),
    ]).await.map_err(storage)?;
    Ok(())
}

/// Fingerprints only the actual immutable consumed-source snapshot for this candidate scope.
pub async fn sources_digest_in(
    tx: &Transaction<'_>,
    scope: &ExecutionScope,
) -> Result<Option<[u8; 32]>, RuntimeError> {
    tx.query_opt("SELECT source_digest FROM imagery.image_dispatch_sources WHERE dispatch_id=$1 AND owner_scope=$2 AND job_id=$3 AND stage_id=$4",
        &[&scope.dispatch.as_bytes().as_slice(),&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice()])
        .await.map_err(storage)?.map(|row|row.get::<_,Vec<u8>>(0).as_slice().try_into()
            .map_err(|_|RuntimeError::InvalidRecord)).transpose()
}
