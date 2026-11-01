//! Bounded keyset pages of real owner-scoped receipts; payload and grant checks stay in the service.
//! Legacy jobs without an indexed content key remain searchable and never become a false miss.

use df_imagery::{CompatibleCursor, MAXIMUM_COMPATIBLE_CANDIDATES, compatible_lookup_key};
use df_imagery::{ImageOwner, JournalError, OperationView, VersionedSnapshot, owner_scope_bytes};
use df_types::OperationId;
use tokio_postgres::Transaction;

use super::rows::{
    JOB_COLUMNS, parse_job, parse_lookup_interest, parse_lookup_receipt, unavailable,
};

pub(crate) async fn compatible_candidates_in(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    intent: &VersionedSnapshot,
    config: &VersionedSnapshot,
    after: Option<CompatibleCursor>,
) -> Result<Vec<OperationView>, JournalError> {
    super::statements::require_read_committed(tx).await?;
    let key = compatible_lookup_key(intent, config)?;
    let key_version = i16::try_from(key.schema_version).map_err(|_| JournalError::InvalidRecord)?;
    let config_revision = i64::from(key.config_revision);
    let owner_scope = owner_scope_bytes(owner);
    let after_ms = after.map(|cursor| cursor.created_at_ms);
    let after_job = after.map(|cursor| cursor.job.as_bytes().to_vec());
    let limit =
        i64::try_from(MAXIMUM_COMPATIBLE_CANDIDATES).map_err(|_| JournalError::InvalidRecord)?;
    let query = format!(
        "SELECT {JOB_COLUMNS}, o.owner_scope AS op_owner, o.operation_id AS op_operation, \
         o.fingerprint AS op_fingerprint, o.job_id AS op_job, o.interest_id AS op_interest, \
         i.interest_id AS int_id, i.owner_scope AS int_owner, i.job_id AS int_job, \
         i.operation_id AS int_operation, i.state AS int_state, i.created_at_ms AS int_created, \
         i.withdrawn_at_ms AS int_withdrawn FROM imagery.image_job j \
         CROSS JOIN LATERAL (SELECT receipt.* FROM imagery.image_operation receipt \
             JOIN imagery.image_interest attached ON attached.interest_id=receipt.interest_id \
             AND attached.owner_scope=receipt.owner_scope AND attached.job_id=receipt.job_id \
             AND attached.operation_id=receipt.operation_id \
             WHERE receipt.job_id=j.job_id AND receipt.owner_scope=j.owner_scope \
             ORDER BY (attached.state='attached') DESC, receipt.created_at_ms, receipt.operation_id \
             LIMIT 1) o \
         JOIN imagery.image_interest i ON i.interest_id=o.interest_id \
         AND i.owner_scope=o.owner_scope AND i.job_id=o.job_id AND i.operation_id=o.operation_id \
         WHERE j.owner_scope=$1 AND (j.compatible_key=$2 OR j.compatible_key IS NULL \
         OR j.compatible_key_version IS DISTINCT FROM $6::smallint \
         OR j.config_revision<>$7::bigint) \
         AND ($3::bigint IS NULL OR (j.created_at_ms,j.job_id)<($3,$4::bytea)) \
         ORDER BY j.created_at_ms DESC,j.job_id DESC LIMIT $5"
    );
    let rows = tx
        .query(
            &query,
            &[
                &owner_scope,
                &key.digest.as_slice(),
                &after_ms,
                &after_job,
                &limit,
                &key_version,
                &config_revision,
            ],
        )
        .await
        .map_err(unavailable)?;
    rows.iter()
        .map(|row| {
            let operation_bytes: Vec<u8> = row.get("op_operation");
            let operation = OperationId::from_bytes(&operation_bytes)
                .map_err(|_| JournalError::InvalidRecord)?;
            let job = parse_job(row)?;
            if job.owner != *owner {
                return Err(JournalError::InvalidRecord);
            }
            let receipt = parse_lookup_receipt(row, owner, operation, job.job)?;
            let interest = parse_lookup_interest(row, owner, job.job, operation, receipt.interest)?;
            Ok(OperationView {
                receipt,
                job,
                interest,
            })
        })
        .collect()
}
