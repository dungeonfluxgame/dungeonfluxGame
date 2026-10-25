//! Operation receipts (idempotency).

use super::prelude::*;

pub(in super::super) async fn operation_result(
    tx: &Tx<'_>,
    namespace: &str,
    account: AccountId,
    operation: &[u8],
) -> StoreResult<Option<([u8; 32], Vec<u8>)>> {
    let row = tx
        .query_opt(
            "SELECT fingerprint, result_ref FROM campaigns.campaign_operation \
             WHERE namespace = $1 AND account_id = $2 AND operation_id = $3",
            &[&namespace, &account.as_bytes().as_slice(), &operation],
        )
        .await?;
    row.map(|row| {
        let fingerprint: &[u8] = row.try_get(0).map_err(|_| StoreError::InvalidRecord)?;
        let result: Vec<u8> = row.try_get(1).map_err(|_| StoreError::InvalidRecord)?;
        let fingerprint: [u8; 32] = fingerprint
            .try_into()
            .map_err(|_| StoreError::InvalidRecord)?;
        Ok((fingerprint, result))
    })
    .transpose()
}

pub(in super::super) async fn insert_operation(
    tx: &Tx<'_>,
    namespace: &str,
    account: AccountId,
    operation: &[u8],
    fingerprint: [u8; 32],
    result: &[u8],
    now_ms: u64,
) -> StoreResult<bool> {
    // A concurrent twin that committed first makes this a no-op; the caller then rolls back and
    // the retry finds the first result.
    let inserted = tx
        .execute(
            "INSERT INTO campaigns.campaign_operation (namespace, account_id, operation_id, \
             fingerprint, result_ref, created_at_ms) VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT DO NOTHING",
            &[
                &namespace,
                &account.as_bytes().as_slice(),
                &operation,
                &fingerprint.as_slice(),
                &result,
                &to_i64(now_ms)?,
            ],
        )
        .await?;
    Ok(inserted == 1)
}
