//! The reset transaction.

use df_customer::{
    AccountEvent, AccountStatus, DeviceLabel, EventKind, PasswordHash, ResetOutcome, ResetPolicy,
    ResetSnapshot, RevokeReason, RevokeScope, StoreError, StoreResult, TokenDigest, TokenPurpose,
};

use super::{
    PostgresAccountStore,
    pool::{begin_transaction, with_client},
    rows::*,
};

impl PostgresAccountStore {
    /// The reset transaction shared by a local reset (which stores `new_hash`) and a provider
    /// reset (which does not): the token's snapshot is loaded under lock and run through
    /// `policy`; on success the generation bumps, every session is revoked and the event is
    /// recorded.
    pub(super) async fn reset(
        &self,
        digest: TokenDigest,
        now_ms: u64,
        policy: ResetPolicy,
        new_hash: Option<PasswordHash>,
    ) -> StoreResult<ResetOutcome> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let Some(row) = transaction
                .query_opt(
                    "SELECT t.account_id, t.purpose, t.expires_at_ms, t.consumed_at_ms, \
                     t.attempts, t.attempt_limit, a.credential_generation, a.status \
                     FROM account_token t JOIN account a USING (account_id) \
                     WHERE t.token_digest = $1 FOR UPDATE OF t, a",
                    &[&&digest.as_bytes()[..]],
                )
                .await
                .map_err(unavailable)?
            else {
                return Ok(ResetOutcome::Invalid);
            };
            let account = account_id(&row.try_get::<_, Vec<u8>>(0).map_err(unavailable)?)?;
            let purpose: String = row.try_get(1).map_err(unavailable)?;
            let consumed: Option<i64> = row.try_get(3).map_err(unavailable)?;
            let attempts: i32 = row.try_get(4).map_err(unavailable)?;
            let attempt_limit: i32 = row.try_get(5).map_err(unavailable)?;
            let status: String = row.try_get(7).map_err(unavailable)?;
            let mut snapshot = ResetSnapshot {
                account,
                purpose_is_reset: TokenPurpose::parse(&purpose) == Some(TokenPurpose::PasswordReset),
                expires_at_ms: to_u64(row.try_get(2).map_err(unavailable)?)?,
                attempts: u32::try_from(attempts).map_err(|_| StoreError::InvalidRecord)?,
                attempt_limit: u32::try_from(attempt_limit).map_err(|_| StoreError::InvalidRecord)?,
                is_consumed: consumed.is_some(),
                credential_generation: to_u64(row.try_get(6).map_err(unavailable)?)?,
                is_account_active: AccountStatus::parse(&status) == Some(AccountStatus::Active),
            };
            let decision = policy(&mut snapshot);
            let now = to_i64(now_ms)?;
            transaction
                .execute(
                    "UPDATE account_token SET attempts = $2, consumed_at_ms = \
                     CASE WHEN $3 THEN COALESCE(consumed_at_ms, $4) ELSE consumed_at_ms END \
                     WHERE token_digest = $1",
                    &[
                        &&digest.as_bytes()[..],
                        &i32::try_from(snapshot.attempts).map_err(|_| StoreError::InvalidRecord)?,
                        &snapshot.is_consumed,
                        &now,
                    ],
                )
                .await
                .map_err(unavailable)?;
            if let Err(refusal) = decision {
                commit(transaction).await?;
                return Ok(ResetOutcome::Refused(refusal));
            }
            transaction
                .execute(
                    "UPDATE account SET credential_generation = $2, \
                     email_verified_at_ms = COALESCE(email_verified_at_ms, $3) \
                     WHERE account_id = $1",
                    &[
                        &&account.as_bytes()[..],
                        &to_i64(snapshot.credential_generation)?,
                        &now,
                    ],
                )
                .await
                .map_err(unavailable)?;
            super::issue_tenant(&transaction, account).await?;
            if let Some(new_hash) = &new_hash {
                transaction
                    .execute(
                        "UPDATE password_credential SET phc_hash = $2, updated_at_ms = $3 \
                         WHERE account_id = $1",
                        &[&&account.as_bytes()[..], &new_hash.phc(), &now],
                    )
                    .await
                    .map_err(unavailable)?;
            }
            revoke(
                &transaction,
                account,
                RevokeScope::All,
                RevokeReason::PasswordReset,
                now_ms,
            )
            .await?;
            insert_event(
                &transaction,
                account,
                AccountEvent {
                    kind: EventKind::PasswordReset,
                    at_ms: now_ms,
                    device: DeviceLabel::Unknown,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(ResetOutcome::Reset(account))
        })
    }
}
