//! Account-owned PostgreSQL PAT persistence. Secrets and pepper keys never enter SQL.
use super::{
    PostgresAccountStore,
    pool::{begin_transaction, with_client},
    rows::{account_id, commit, to_i64, to_u64, unavailable},
};
use df_customer::{
    StoreError, StoreResult,
    integration::{
        CreateTokenOutcome, MAXIMUM_ACTIVE_TOKENS, PersonalTokenStore, Scope, StoredToken, TokenId,
        TokenMetadata,
    },
};
use df_types::AccountId;
use tokio_postgres::Row;

const METADATA: &str = "token_id, account_id, label, scopes, created_at_ms, expires_at_ms, revoked_at_ms, last_used_at_ms";

fn metadata(row: &Row) -> StoreResult<TokenMetadata> {
    let id: Vec<u8> = row.try_get(0).map_err(unavailable)?;
    let account: Vec<u8> = row.try_get(1).map_err(unavailable)?;
    let scopes: Vec<String> = row.try_get(3).map_err(unavailable)?;
    let optional_time = |index| -> StoreResult<Option<u64>> {
        row.try_get::<_, Option<i64>>(index)
            .map_err(unavailable)?
            .map(to_u64)
            .transpose()
    };
    Ok(TokenMetadata {
        id: TokenId(id.try_into().map_err(|_| StoreError::InvalidRecord)?),
        account: account_id(&account)?,
        label: row.try_get(2).map_err(unavailable)?,
        scopes: scopes
            .iter()
            .map(|s| Scope::parse(s).ok_or(StoreError::InvalidRecord))
            .collect::<StoreResult<_>>()?,
        created_at_ms: to_u64(row.try_get(4).map_err(unavailable)?)?,
        expires_at_ms: to_u64(row.try_get(5).map_err(unavailable)?)?,
        revoked_at_ms: optional_time(6)?,
        last_used_at_ms: optional_time(7)?,
    })
}

impl PersonalTokenStore for PostgresAccountStore {
    async fn integration_sessions(
        &self,
        account: AccountId,
        after: Option<TokenId>,
        limit: u32,
    ) -> StoreResult<Vec<df_customer::integration::SignInSession>> {
        with_client!(self, client => {
            let rows = client.query("SELECT \
                            session_id,device_label,created_at_ms,last_seen_at_ms,absolute_expires_at_ms,revoked_at_ms IS NOT NULL FROM web_session WHERE account_id = $1 AND ($2::bytea IS NULL OR session_id > $2) ORDER BY session_id LIMIT $3", &[&account.as_bytes().as_slice(), &after.map(|id| id.0.to_vec()), &i64::from(limit.min(101))]).await.map_err(unavailable)?;
            rows.iter()
                .map(|row| {
                    let id: Vec<u8> = row.try_get(0).map_err(unavailable)?;
                    let label: &str = row.try_get(1).map_err(unavailable)?;
                    Ok(df_customer::integration::SignInSession {
                        id: TokenId(id.try_into().map_err(|_| StoreError::InvalidRecord)?),
                        device_label: super::rows::device(label)?.label(),
                        created_at_ms: to_u64(row.try_get(2).map_err(unavailable)?)?,
                        last_seen_at_ms: to_u64(row.try_get(3).map_err(unavailable)?)?,
                        expires_at_ms: to_u64(row.try_get(4).map_err(unavailable)?)?,
                        revoked: row.try_get(5).map_err(unavailable)?,
                    })
                })
                .collect()
        })
    }
    async fn create_personal_token(&self, token: StoredToken) -> StoreResult<CreateTokenOutcome> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let m = &token.metadata;
            let owner = m.account.as_bytes().as_slice();
            let account = transaction.query_opt("SELECT status, email_verified_at_ms IS NOT NULL FROM account WHERE account_id = $1 FOR UPDATE", &[&owner]).await.map_err(unavailable)?;
            let Some(account) = account else {
                return Ok(CreateTokenOutcome::InactiveAccount);
            };
            if account.try_get::<_, &str>(0).map_err(unavailable)? != "active"
                || !account.try_get::<_, bool>(1).map_err(unavailable)?
            {
                return Ok(CreateTokenOutcome::InactiveAccount);
            }
            let now = to_i64(m.created_at_ms)?;
            let active: i64 = transaction
                .query_one(
                    "SELECT count(*) FROM personal_token WHERE account_id = $1 AND revoked_at_ms IS NULL \
                            AND expires_at_ms > $2",
                    &[&owner, &now],
                )
                .await
                .map_err(unavailable)?
                .try_get(0)
                .map_err(unavailable)?;
            if active >= MAXIMUM_ACTIVE_TOKENS as i64 {
                return Ok(CreateTokenOutcome::LimitReached);
            }
            transaction.execute("INSERT INTO \
                            personal_token(token_id,account_id,label,scopes,pepper_id,secret_digest,created_at_ms,expires_at_ms) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)", &[&m.id.0.as_slice(), &owner, &m.label, &m.scopes.iter().map(|s| s.name()).collect::<Vec<_>>(), &i64::from(token.pepper_id), &token.digest.as_slice(), &now, &to_i64(m.expires_at_ms)?]).await.map_err(unavailable)?;
            // Historical metadata is bounded per account; live tokens are never removed.
            transaction.execute("DELETE FROM personal_token WHERE account_id = $1 AND token_id IN (SELECT token_id \
                            FROM personal_token WHERE account_id = $1 AND (revoked_at_ms IS NOT NULL OR \
                            expires_at_ms <= $2) ORDER BY created_at_ms DESC, token_id DESC OFFSET 90 LIMIT 100)", &[&owner, &now]).await.map_err(unavailable)?;
            commit(transaction).await?;
            Ok(CreateTokenOutcome::Created)
        })
    }

    async fn personal_tokens(&self, account: AccountId) -> StoreResult<Vec<TokenMetadata>> {
        with_client!(self, client => {
            let rows = client.query(&format!("SELECT {METADATA} FROM personal_token WHERE account_id = $1 ORDER BY created_at_ms \
                            DESC, token_id DESC LIMIT 100"), &[&account.as_bytes().as_slice()]).await.map_err(unavailable)?;
            rows.iter().map(metadata).collect()
        })
    }

    async fn personal_token(&self, id: TokenId) -> StoreResult<Option<StoredToken>> {
        with_client!(self, client => {
            let row = client.query_opt(&format!("SELECT {METADATA}, pepper_id, secret_digest FROM personal_token WHERE token_id = $1"), &[&id.0.as_slice()]).await.map_err(unavailable)?;
            row.map(|row| {
                let key: i64 = row.try_get(8).map_err(unavailable)?;
                let digest: Vec<u8> = row.try_get(9).map_err(unavailable)?;
                Ok(StoredToken {
                    metadata: metadata(&row)?,
                    pepper_id: u32::try_from(key).map_err(|_| StoreError::InvalidRecord)?,
                    digest: digest.try_into().map_err(|_| StoreError::InvalidRecord)?,
                })
            })
            .transpose()
        })
    }

    async fn revoke_personal_token(
        &self,
        account: AccountId,
        id: TokenId,
        now_ms: u64,
    ) -> StoreResult<bool> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let count = transaction
                .execute(
                    "UPDATE personal_token SET revoked_at_ms = COALESCE(revoked_at_ms, $3) WHERE \
                            account_id = $1 AND token_id = $2",
                    &[
                        &account.as_bytes().as_slice(),
                        &id.0.as_slice(),
                        &to_i64(now_ms)?,
                    ],
                )
                .await
                .map_err(unavailable)?;
            commit(transaction).await?;
            Ok(count == 1)
        })
    }

    async fn used_personal_token(
        &self,
        id: TokenId,
        old_digest: [u8; 32],
        pepper_id: u32,
        digest: [u8; 32],
        now_ms: u64,
    ) -> StoreResult<bool> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let count = transaction.execute("UPDATE personal_token t SET last_used_at_ms = GREATEST(last_used_at_ms, $5), \
                            pepper_id = $3, secret_digest = $4 WHERE token_id = $1 AND secret_digest = $2 AND \
                            revoked_at_ms IS NULL AND expires_at_ms > $5 AND EXISTS (SELECT 1 FROM account a \
                            WHERE a.account_id = t.account_id AND a.status = 'active' AND a.email_verified_at_ms \
                            IS NOT NULL)", &[&id.0.as_slice(), &old_digest.as_slice(), &i64::from(pepper_id), &digest.as_slice(), &to_i64(now_ms)?]).await.map_err(unavailable)?;
            commit(transaction).await?;
            Ok(count == 1)
        })
    }
}

#[cfg(test)]
#[path = "integration_tests.rs"]
mod tests;
