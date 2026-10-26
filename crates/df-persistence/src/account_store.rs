//! PostgreSQL adapter for `df_customer::AccountStore` (schema `accounts` of the hosted database,
//! with versioned account migrations). Every mutating method is one transaction; queries are
//! parameterized and scoped by account id. A failure before commit is `Unavailable`; a failed or
//! lost commit is `Unknown`. Credentials never appear in errors or logs.

use df_customer::{
    AccountEvent, AccountRecord, AccountStatus, AccountStore, AccountsSummary, CloseOutcome,
    CompletionOutcome, ConfirmChangeOutcome, CreateOutcome, CsrfSecret, DeviceLabel, DisplayName,
    EmailAddress, EmailChangeDecision, EmailChangeOutcome, EmailChangeRequest, EventKind,
    IdentityProviderKind, NewAccount, NewIdentityAccount, NewSession, NewToken, OperationClaim,
    OperationKey, PasswordChange, PasswordChangeOutcome, PasswordHash, PurgeCounts, ReceiptOutcome,
    RecentAccount, ResetOutcome, ResetPolicy, RetentionCutoffs, RevokeReason, RevokeScope,
    SessionRecord, SessionSummary, StoreError, StoreResult, TokenDigest, TokenPurpose,
    VerificationTarget, VerifyOutcome,
};
use df_types::{AccountId, TenantId, WebSessionId};

mod closing;
mod oauth;
mod pool;
mod resets;
mod rows;

pub use pool::{AccountStoreOpenError, PostgresAccountStore};
use pool::{UNIQUE_VIOLATION, begin_transaction, with_client};
use rows::*;

#[cfg(test)]
impl PostgresAccountStore {
    pub(crate) async fn stalled_statement_is_cut_off(&self) -> StoreResult<bool> {
        with_client!(self, client => {
            let transaction = client.transaction().await.map_err(unavailable)?;
            transaction
                .batch_execute("SET LOCAL transaction_timeout = '100ms'")
                .await
                .map_err(unavailable)?;
            Ok(transaction.query_one("SELECT pg_sleep(1)", &[]).await.is_err())
        })
    }
}

impl AccountStore for PostgresAccountStore {
    async fn create_account(&self, account: NewAccount) -> StoreResult<CreateOutcome> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let inserted = transaction
                .execute(
                    "INSERT INTO account (account_id, email_normalized, email_display, status, \
                     credential_generation, created_at_ms, adult_attested_at, terms_version, \
                     terms_accepted_at) VALUES ($1, $2, $3, 'active', 1, $4, $5, $6, $7) \
                     ON CONFLICT (email_normalized) DO NOTHING",
                    &[
                        &&account.id.as_bytes()[..],
                        &account.email.normalized(),
                        &account.email.display(),
                        &to_i64(account.created_at_ms)?,
                        &to_i64(account.adult_attested_at)?,
                        &account.terms_version,
                        &to_i64(account.terms_accepted_at)?,
                    ],
                )
                .await
                .map_err(unavailable)?;
            if inserted == 0 {
                return Ok(CreateOutcome::EmailTaken);
            }
            transaction
                .execute(
                    "INSERT INTO password_credential (account_id, phc_hash, updated_at_ms) \
                     VALUES ($1, $2, $3)",
                    &[
                        &&account.id.as_bytes()[..],
                        &account.password_hash.phc(),
                        &to_i64(account.created_at_ms)?,
                    ],
                )
                .await
                .map_err(unavailable)?;
            insert_token(&transaction, &account.verification).await?;
            insert_event(
                &transaction,
                account.id,
                AccountEvent {
                    kind: EventKind::SignedUp,
                    at_ms: account.created_at_ms,
                    device: DeviceLabel::Unknown,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(CreateOutcome::Created)
        })
    }

    async fn create_identity_account(
        &self,
        account: NewIdentityAccount,
    ) -> StoreResult<CreateOutcome> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let created = to_i64(account.created_at_ms)?;
            let verified_at = account.is_verified.then_some(created);
            let inserted = transaction
                .execute(
                    "INSERT INTO account (account_id, email_normalized, email_display, \
                     email_verified_at_ms, status, credential_generation, created_at_ms, \
                     adult_attested_at, terms_version, terms_accepted_at) \
                     VALUES ($1, $2, $3, $4, 'active', 1, $5, $6, $7, $8) \
                     ON CONFLICT (email_normalized) DO NOTHING",
                    &[
                        &&account.id.as_bytes()[..],
                        &account.email.normalized(),
                        &account.email.display(),
                        &verified_at,
                        &created,
                        &account.adult_attested_at.map(to_i64).transpose()?,
                        &account.terms_version,
                        &account.terms_accepted_at.map(to_i64).transpose()?,
                    ],
                )
                .await
                .map_err(unavailable)?;
            if inserted == 0 {
                return Ok(CreateOutcome::EmailTaken);
            }
            if account.is_verified {
                issue_tenant(&transaction, account.id).await?;
            }
            if let Some(token) = &account.verification {
                insert_token(&transaction, token).await?;
            }
            transaction
                .execute(
                    "INSERT INTO identity_link (provider, subject, account_id, linked_at_ms) \
                     VALUES ($1, $2, $3, $4)",
                    &[
                        &account.provider.label(),
                        &account.subject,
                        &&account.id.as_bytes()[..],
                        &created,
                    ],
                )
                .await
                .map_err(unavailable)?;
            insert_event(
                &transaction,
                account.id,
                AccountEvent {
                    kind: EventKind::SignedUp,
                    at_ms: account.created_at_ms,
                    device: DeviceLabel::Unknown,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(CreateOutcome::Created)
        })
    }

    async fn account_for_identity(
        &self,
        provider: IdentityProviderKind,
        subject: &str,
    ) -> StoreResult<Option<AccountRecord>> {
        with_client!(self, client => {
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {ACCOUNT_COLUMNS} FROM identity_link l \
                     JOIN account a ON a.account_id = l.account_id \
                     LEFT JOIN password_credential p ON p.account_id = a.account_id \
                     WHERE l.provider = $1 AND l.subject = $2"
                    ),
                    &[&provider.label(), &subject],
                )
                .await
                .map_err(unavailable)?;
            row.map(|row| account_from_row(&row, 0)).transpose()
        })
    }

    async fn identity_for_account(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
    ) -> StoreResult<Option<String>> {
        with_client!(self, client => {
            let row = client
                .query_opt(
                    "SELECT subject FROM identity_link WHERE account_id = $1 AND provider = $2 \
                     ORDER BY linked_at_ms, subject LIMIT 1",
                    &[&&account.as_bytes()[..], &provider.label()],
                )
                .await
                .map_err(unavailable)?;
            row.map(|row| row.try_get(0).map_err(unavailable)).transpose()
        })
    }

    async fn identities_of(
        &self,
        account: AccountId,
    ) -> StoreResult<Vec<(IdentityProviderKind, String)>> {
        with_client!(self, client => {
            let rows = client
                .query(
                    "SELECT provider, subject FROM identity_link WHERE account_id = $1 \
                     ORDER BY provider, subject",
                    &[&&account.as_bytes()[..]],
                )
                .await
                .map_err(unavailable)?;
            rows.iter()
                .map(|row| {
                    let provider: String = row.try_get(0).map_err(unavailable)?;
                    Ok((
                        IdentityProviderKind::parse(&provider).ok_or(StoreError::InvalidRecord)?,
                        row.try_get(1).map_err(unavailable)?,
                    ))
                })
                .collect()
        })
    }

    async fn unlink_identity(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
        subject: &str,
    ) -> StoreResult<()> {
        with_client!(self, client => {
            client
                .execute(
                    "DELETE FROM identity_link WHERE account_id = $1 AND provider = $2 \
                     AND subject = $3",
                    &[&&account.as_bytes()[..], &provider.label(), &subject],
                )
                .await
                .map_err(|_| StoreError::Unknown)?;
            Ok(())
        })
    }

    async fn link_identity(
        &self,
        account: AccountId,
        provider: IdentityProviderKind,
        subject: &str,
        now_ms: u64,
    ) -> StoreResult<()> {
        with_client!(self, client => {
            client
                .execute(
                    "INSERT INTO identity_link (provider, subject, account_id, linked_at_ms) \
                     VALUES ($1, $2, $3, $4) ON CONFLICT (provider, subject) DO NOTHING",
                    &[
                        &provider.label(),
                        &subject,
                        &&account.as_bytes()[..],
                        &to_i64(now_ms)?,
                    ],
                )
                .await
                .map_err(|_| StoreError::Unknown)?;
            Ok(())
        })
    }

    async fn account_by_email(&self, normalized_email: &str) -> StoreResult<Option<AccountRecord>> {
        with_client!(self, client => {
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {ACCOUNT_COLUMNS} FROM account a \
                         LEFT JOIN password_credential p USING (account_id) \
                         WHERE a.email_normalized = $1"
                    ),
                    &[&normalized_email],
                )
                .await
                .map_err(unavailable)?;
            row.map(|row| account_from_row(&row, 0)).transpose()
        })
    }

    async fn account(&self, id: AccountId) -> StoreResult<Option<AccountRecord>> {
        with_client!(self, client => {
            let row = client
                .query_opt(
                    &format!(
                        "SELECT {ACCOUNT_COLUMNS} FROM account a \
                         LEFT JOIN password_credential p USING (account_id) \
                         WHERE a.account_id = $1"
                    ),
                    &[&&id.as_bytes()[..]],
                )
                .await
                .map_err(unavailable)?;
            row.map(|row| account_from_row(&row, 0)).transpose()
        })
    }

    async fn tenant_for(&self, account: AccountId) -> StoreResult<Option<TenantId>> {
        with_client!(self, client => {
            let row = client.query_opt(
                "SELECT t.tenant_id FROM tenant t JOIN account a ON a.account_id = t.owner_account_id \
                 WHERE a.account_id = $1 AND a.email_verified_at_ms IS NOT NULL \
                 AND a.status = 'active' AND t.standing = 'active'",
                &[&&account.as_bytes()[..]],
            ).await.map_err(unavailable)?;
            row.map(|row| {
                let bytes: Vec<u8> = row.try_get(0).map_err(unavailable)?;
                TenantId::from_bytes(&bytes).map_err(|_| StoreError::InvalidRecord)
            }).transpose()
        })
    }

    async fn issue_token(&self, token: NewToken) -> StoreResult<()> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            transaction
                .execute(
                    "UPDATE account_token SET consumed_at_ms = $3 WHERE account_id = $1 \
                     AND purpose = $2 AND consumed_at_ms IS NULL",
                    &[
                        &&token.account.as_bytes()[..],
                        &token.purpose.label(),
                        &to_i64(token.created_at_ms)?,
                    ],
                )
                .await
                .map_err(unavailable)?;
            insert_token(&transaction, &token).await?;
            commit(transaction).await
        })
    }

    async fn verification_target(
        &self,
        digest: TokenDigest,
        now_ms: u64,
    ) -> StoreResult<Option<VerificationTarget>> {
        with_client!(self, client => {
            let Some(row) = client
                .query_opt(
                    "SELECT t.account_id, t.purpose, t.expires_at_ms, t.consumed_at_ms, \
                     a.email_verified_at_ms FROM account_token t JOIN account a USING (account_id) \
                     WHERE t.token_digest = $1",
                    &[&&digest.as_bytes()[..]],
                )
                .await
                .map_err(unavailable)?
            else {
                return Ok(None);
            };
            let account = account_id(&row.try_get::<_, Vec<u8>>(0).map_err(unavailable)?)?;
            let purpose: String = row.try_get(1).map_err(unavailable)?;
            let expires: i64 = row.try_get(2).map_err(unavailable)?;
            let consumed: Option<i64> = row.try_get(3).map_err(unavailable)?;
            let verified: Option<i64> = row.try_get(4).map_err(unavailable)?;
            if TokenPurpose::parse(&purpose) != Some(TokenPurpose::VerifyEmail) {
                return Ok(None);
            }
            let is_open = consumed.is_none() && to_i64(now_ms)? < expires;
            Ok((verified.is_some() || is_open).then_some(VerificationTarget {
                account,
                already_verified: verified.is_some(),
            }))
        })
    }

    async fn verify_email(
        &self,
        digest: TokenDigest,
        now_ms: u64,
        replacement: Option<PasswordHash>,
    ) -> StoreResult<VerifyOutcome> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let Some(row) = transaction
                .query_opt(
                    "SELECT t.account_id, t.purpose, t.expires_at_ms, t.consumed_at_ms, \
                     a.email_verified_at_ms FROM account_token t JOIN account a USING (account_id) \
                     WHERE t.token_digest = $1 FOR UPDATE OF t, a",
                    &[&&digest.as_bytes()[..]],
                )
                .await
                .map_err(unavailable)?
            else {
                return Ok(VerifyOutcome::Invalid);
            };
            let account = account_id(&row.try_get::<_, Vec<u8>>(0).map_err(unavailable)?)?;
            let purpose: String = row.try_get(1).map_err(unavailable)?;
            let expires: i64 = row.try_get(2).map_err(unavailable)?;
            let consumed: Option<i64> = row.try_get(3).map_err(unavailable)?;
            let verified: Option<i64> = row.try_get(4).map_err(unavailable)?;
            if TokenPurpose::parse(&purpose) != Some(TokenPurpose::VerifyEmail) {
                return Ok(VerifyOutcome::Invalid);
            }
            if verified.is_some() {
                return Ok(VerifyOutcome::AlreadyVerified(account));
            }
            if consumed.is_some() || to_i64(now_ms)? >= expires {
                return Ok(VerifyOutcome::Invalid);
            }
            let now = to_i64(now_ms)?;
            transaction
                .execute(
                    "UPDATE account_token SET consumed_at_ms = $2 WHERE token_digest = $1",
                    &[&&digest.as_bytes()[..], &now],
                )
                .await
                .map_err(unavailable)?;
            // Proof of the mailbox ends every session made before it.
            transaction
                .execute(
                    "UPDATE account SET email_verified_at_ms = $2, \
                     credential_generation = credential_generation + 1 WHERE account_id = $1",
                    &[&&account.as_bytes()[..], &now],
                )
                .await
                .map_err(unavailable)?;
            issue_tenant(&transaction, account).await?;
            if let Some(hash) = &replacement {
                transaction
                    .execute(
                        "UPDATE password_credential SET phc_hash = $2, updated_at_ms = $3 \
                         WHERE account_id = $1",
                        &[&&account.as_bytes()[..], &hash.phc(), &now],
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
                    kind: EventKind::Verified,
                    at_ms: now_ms,
                    device: DeviceLabel::Unknown,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(VerifyOutcome::Verified(account))
        })
    }

    async fn create_session(
        &self,
        session: NewSession,
        event: Option<AccountEvent>,
    ) -> StoreResult<()> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            insert_session(&transaction, &session).await?;
            if let Some(event) = event {
                insert_event(&transaction, session.account, event).await?;
            }
            commit(transaction).await
        })
    }

    async fn session_by_digest(
        &self,
        digest: TokenDigest,
    ) -> StoreResult<Option<(SessionRecord, AccountRecord)>> {
        with_client!(self, client => {
            let Some(row) = client
                .query_opt(
                    &format!(
                        "SELECT s.session_id, s.credential_generation, s.created_at_ms, \
                         s.last_seen_at_ms, s.revoked_at_ms, s.csrf_secret, s.device_label, \
                         {ACCOUNT_COLUMNS} FROM web_session s \
                         JOIN account a ON a.account_id = s.account_id \
                         LEFT JOIN password_credential p ON p.account_id = s.account_id \
                         WHERE s.token_digest = $1"
                    ),
                    &[&&digest.as_bytes()[..]],
                )
                .await
                .map_err(unavailable)?
            else {
                return Ok(None);
            };
            let account = account_from_row(&row, 7)?;
            let csrf: Vec<u8> = row.try_get(5).map_err(unavailable)?;
            let csrf: [u8; 32] = csrf.try_into().map_err(|_| StoreError::InvalidRecord)?;
            let revoked: Option<i64> = row.try_get(4).map_err(unavailable)?;
            let label: String = row.try_get(6).map_err(unavailable)?;
            let session = SessionRecord {
                id: session_id(&row.try_get::<_, Vec<u8>>(0).map_err(unavailable)?)?,
                account: account.id,
                credential_generation: to_u64(row.try_get(1).map_err(unavailable)?)?,
                created_at_ms: to_u64(row.try_get(2).map_err(unavailable)?)?,
                last_seen_at_ms: to_u64(row.try_get(3).map_err(unavailable)?)?,
                is_revoked: revoked.is_some(),
                csrf_secret: CsrfSecret::from_bytes(csrf),
                device: device(&label)?,
            };
            Ok(Some((session, account)))
        })
    }

    async fn touch_session(&self, id: WebSessionId, now_ms: u64) -> StoreResult<()> {
        with_client!(self, client => {
            client
                .execute(
                    "UPDATE web_session SET last_seen_at_ms = GREATEST(last_seen_at_ms, $2) \
                     WHERE session_id = $1",
                    &[&&id.as_bytes()[..], &to_i64(now_ms)?],
                )
                .await
                .map_err(|_| StoreError::Unknown)?;
            Ok(())
        })
    }

    async fn revoke_sessions(
        &self,
        account: AccountId,
        scope: RevokeScope,
        reason: RevokeReason,
        now_ms: u64,
        event: Option<AccountEvent>,
    ) -> StoreResult<u64> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let revoked = revoke(&transaction, account, scope, reason, now_ms).await?;
            if let Some(event) = event {
                insert_event(&transaction, account, event).await?;
            }
            commit(transaction).await?;
            Ok(revoked)
        })
    }

    async fn sessions(
        &self,
        account: AccountId,
        now_ms: u64,
        idle_before_ms: u64,
    ) -> StoreResult<Vec<SessionSummary>> {
        with_client!(self, client => {
            let rows = client
                .query(
                    "SELECT session_id, device_label, created_at_ms, last_seen_at_ms \
                     FROM web_session WHERE account_id = $1 AND revoked_at_ms IS NULL \
                     AND absolute_expires_at_ms > $2 AND last_seen_at_ms >= $3 \
                     ORDER BY last_seen_at_ms DESC, session_id",
                    &[
                        &&account.as_bytes()[..],
                        &to_i64(now_ms)?,
                        &to_i64(idle_before_ms)?,
                    ],
                )
                .await
                .map_err(unavailable)?;
            rows.iter()
                .map(|row| {
                    let label: String = row.try_get(1).map_err(unavailable)?;
                    Ok(SessionSummary {
                        id: session_id(&row.try_get::<_, Vec<u8>>(0).map_err(unavailable)?)?,
                        device: device(&label)?,
                        created_at_ms: to_u64(row.try_get(2).map_err(unavailable)?)?,
                        last_seen_at_ms: to_u64(row.try_get(3).map_err(unavailable)?)?,
                    })
                })
                .collect()
        })
    }

    async fn change_password(&self, change: PasswordChange) -> StoreResult<PasswordChangeOutcome> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let account = &change.account.as_bytes()[..];
            let current: Option<i64> = transaction
                .query_opt(
                    "SELECT credential_generation FROM account WHERE account_id = $1 FOR UPDATE",
                    &[&account],
                )
                .await
                .map_err(unavailable)?
                .map(|row| row.try_get(0))
                .transpose()
                .map_err(unavailable)?;
            if current != Some(to_i64(change.expected_generation)?) {
                return Ok(PasswordChangeOutcome::StaleGeneration);
            }
            transaction
                .execute(
                    "UPDATE account SET credential_generation = credential_generation + 1 \
                     WHERE account_id = $1",
                    &[&account],
                )
                .await
                .map_err(unavailable)?;
            transaction
                .execute(
                    "UPDATE password_credential SET phc_hash = $2, updated_at_ms = $3 \
                     WHERE account_id = $1",
                    &[&account, &change.new_hash.phc(), &to_i64(change.at_ms)?],
                )
                .await
                .map_err(unavailable)?;
            revoke(
                &transaction,
                change.account,
                RevokeScope::All,
                RevokeReason::PasswordChanged,
                change.at_ms,
            )
            .await?;
            insert_session(&transaction, &change.replacement).await?;
            insert_event(
                &transaction,
                change.account,
                AccountEvent {
                    kind: EventKind::PasswordChanged,
                    at_ms: change.at_ms,
                    device: change.replacement.device,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(PasswordChangeOutcome::Changed)
        })
    }

    async fn rehash_password(
        &self,
        account: AccountId,
        expected_generation: u64,
        new_hash: PasswordHash,
    ) -> StoreResult<()> {
        with_client!(self, client => {
            client
                .execute(
                    "UPDATE password_credential p SET phc_hash = $3 FROM account a \
                     WHERE p.account_id = $1 AND a.account_id = p.account_id \
                     AND a.credential_generation = $2",
                    &[
                        &&account.as_bytes()[..],
                        &to_i64(expected_generation)?,
                        &new_hash.phc(),
                    ],
                )
                .await
                .map_err(|_| StoreError::Unknown)?;
            Ok(())
        })
    }

    async fn reset_password(
        &self,
        digest: TokenDigest,
        new_hash: PasswordHash,
        now_ms: u64,
        policy: ResetPolicy,
    ) -> StoreResult<ResetOutcome> {
        self.reset(digest, now_ms, policy, Some(new_hash)).await
    }

    async fn consume_reset_token(
        &self,
        digest: TokenDigest,
        now_ms: u64,
        policy: ResetPolicy,
    ) -> StoreResult<ResetOutcome> {
        self.reset(digest, now_ms, policy, None).await
    }

    async fn request_email_change(
        &self,
        request: EmailChangeRequest,
        now_ms: u64,
    ) -> StoreResult<EmailChangeOutcome> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let account = &request.account.as_bytes()[..];
            transaction
                .execute(
                    "SELECT 1 FROM account WHERE account_id = $1 FOR UPDATE",
                    &[&account],
                )
                .await
                .map_err(unavailable)?;
            let taken = transaction
                .query_opt(
                    "SELECT 1 FROM account WHERE email_normalized = $1",
                    &[&request.new_email.normalized()],
                )
                .await
                .map_err(unavailable)?
                .is_some();
            if taken {
                return Ok(EmailChangeOutcome::AddressTaken);
            }
            let now = to_i64(now_ms)?;
            transaction
                .execute(
                    "UPDATE account_token SET consumed_at_ms = $2 WHERE consumed_at_ms IS NULL \
                     AND email_change_id IN (SELECT email_change_id FROM email_change \
                     WHERE account_id = $1 AND is_open)",
                    &[&account, &now],
                )
                .await
                .map_err(unavailable)?;
            transaction
                .execute(
                    "UPDATE email_change SET is_open = false WHERE account_id = $1 AND is_open",
                    &[&account],
                )
                .await
                .map_err(unavailable)?;
            transaction
                .execute(
                    "INSERT INTO email_change (email_change_id, account_id, \
                     new_email_normalized, new_email_display, expires_at_ms) \
                     VALUES ($1, $2, $3, $4, $5)",
                    &[
                        &&request.id[..],
                        &account,
                        &request.new_email.normalized(),
                        &request.new_email.display(),
                        &to_i64(request.expires_at_ms)?,
                    ],
                )
                .await
                .map_err(unavailable)?;
            insert_token(&transaction, &request.old_address_token).await?;
            insert_token(&transaction, &request.new_address_token).await?;
            insert_event(
                &transaction,
                request.account,
                AccountEvent {
                    kind: EventKind::EmailChangeRequested,
                    at_ms: now_ms,
                    device: DeviceLabel::Unknown,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(EmailChangeOutcome::Requested)
        })
    }

    async fn confirm_email_change(
        &self,
        digest: TokenDigest,
        decision: EmailChangeDecision,
        now_ms: u64,
    ) -> StoreResult<ConfirmChangeOutcome> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let now = to_i64(now_ms)?;
            let Some(row) = transaction
                .query_opt(
                    "SELECT t.purpose, c.email_change_id, c.account_id, c.new_email_normalized, \
                     c.new_email_display, c.old_confirmed, c.new_confirmed \
                     FROM account_token t JOIN email_change c USING (email_change_id) \
                     WHERE t.token_digest = $1 AND t.consumed_at_ms IS NULL \
                     AND t.expires_at_ms > $2 AND c.is_open AND c.expires_at_ms > $2 \
                     FOR UPDATE OF t, c",
                    &[&&digest.as_bytes()[..], &now],
                )
                .await
                .map_err(unavailable)?
            else {
                return Ok(ConfirmChangeOutcome::Invalid);
            };
            let purpose: String = row.try_get(0).map_err(unavailable)?;
            let change: Vec<u8> = row.try_get(1).map_err(unavailable)?;
            let account = account_id(&row.try_get::<_, Vec<u8>>(2).map_err(unavailable)?)?;
            let new_email = EmailAddress::from_stored(
                row.try_get(3).map_err(unavailable)?,
                row.try_get(4).map_err(unavailable)?,
            );
            let mut old_confirmed: bool = row.try_get(5).map_err(unavailable)?;
            let mut new_confirmed: bool = row.try_get(6).map_err(unavailable)?;
            match TokenPurpose::parse(&purpose) {
                Some(TokenPurpose::EmailChangeOld) => old_confirmed = true,
                Some(TokenPurpose::EmailChangeNew) => new_confirmed = true,
                _ => return Ok(ConfirmChangeOutcome::Invalid),
            }
            if decision == EmailChangeDecision::Cancel {
                transaction
                    .execute(
                        "UPDATE email_change SET is_open = false WHERE email_change_id = $1",
                        &[&change],
                    )
                    .await
                    .map_err(unavailable)?;
                transaction
                    .execute(
                        "UPDATE account_token SET consumed_at_ms = $2 \
                         WHERE email_change_id = $1 AND consumed_at_ms IS NULL",
                        &[&change, &now],
                    )
                    .await
                    .map_err(unavailable)?;
                commit(transaction).await?;
                return Ok(ConfirmChangeOutcome::Cancelled { account });
            }
            transaction
                .execute(
                    "UPDATE account_token SET consumed_at_ms = $2 WHERE token_digest = $1",
                    &[&&digest.as_bytes()[..], &now],
                )
                .await
                .map_err(unavailable)?;
            let is_complete = old_confirmed && new_confirmed;
            transaction
                .execute(
                    "UPDATE email_change SET old_confirmed = $2, new_confirmed = $3, \
                     is_open = NOT $4 WHERE email_change_id = $1",
                    &[&change, &old_confirmed, &new_confirmed, &is_complete],
                )
                .await
                .map_err(unavailable)?;
            if !is_complete {
                commit(transaction).await?;
                return Ok(ConfirmChangeOutcome::AwaitingOtherAddress);
            }
            let Some(old_row) = transaction
                .query_opt(
                    "SELECT email_normalized, email_display FROM account \
                     WHERE account_id = $1 FOR UPDATE",
                    &[&&account.as_bytes()[..]],
                )
                .await
                .map_err(unavailable)?
            else {
                return Ok(ConfirmChangeOutcome::Invalid);
            };
            let (Some(old_normalized), Some(old_display)) = (
                old_row.try_get::<_, Option<String>>(0).map_err(unavailable)?,
                old_row.try_get::<_, Option<String>>(1).map_err(unavailable)?,
            ) else {
                return Ok(ConfirmChangeOutcome::Invalid);
            };
            let changed = transaction
                .execute(
                    "UPDATE account SET email_normalized = $2, email_display = $3, \
                     credential_generation = credential_generation + 1, \
                     email_verified_at_ms = $4 WHERE account_id = $1",
                    &[
                        &&account.as_bytes()[..],
                        &new_email.normalized(),
                        &new_email.display(),
                        &now,
                    ],
                )
                .await;
            if let Err(error) = changed {
                if error.code().map(|code| code.code()) == Some(UNIQUE_VIOLATION) {
                    // Another account took the address meanwhile; the whole change rolls back.
                    return Ok(ConfirmChangeOutcome::AddressTaken);
                }
                return Err(StoreError::Unavailable);
            }
            revoke(
                &transaction,
                account,
                RevokeScope::All,
                RevokeReason::EmailChanged,
                now_ms,
            )
            .await?;
            insert_event(
                &transaction,
                account,
                AccountEvent {
                    kind: EventKind::EmailChanged,
                    at_ms: now_ms,
                    device: DeviceLabel::Unknown,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(ConfirmChangeOutcome::Completed {
                account,
                old_email: EmailAddress::from_stored(old_normalized, old_display),
                new_email,
            })
        })
    }

    async fn update_profile(
        &self,
        account: AccountId,
        display_name: Option<DisplayName>,
    ) -> StoreResult<()> {
        with_client!(self, client => {
            client
                .execute(
                    "UPDATE account SET display_name = $2 WHERE account_id = $1",
                    &[
                        &&account.as_bytes()[..],
                        &display_name.as_ref().map(DisplayName::as_str),
                    ],
                )
                .await
                .map_err(|_| StoreError::Unknown)?;
            Ok(())
        })
    }

    async fn close_account(
        &self,
        account: AccountId,
        now_ms: u64,
        delete_after_ms: u64,
    ) -> StoreResult<CloseOutcome> {
        self.close(account, now_ms, delete_after_ms).await
    }

    async fn reopen_account(&self, account: AccountId, now_ms: u64) -> StoreResult<bool> {
        self.reopen(account, now_ms).await
    }

    async fn due_closures(&self, now_ms: u64, limit: u32) -> StoreResult<Vec<AccountId>> {
        self.due(now_ms, limit).await
    }

    async fn complete_closure(
        &self,
        account: AccountId,
        now_ms: u64,
    ) -> StoreResult<CompletionOutcome> {
        self.complete(account, now_ms).await
    }

    async fn purge_closed(&self, account: AccountId, now_ms: u64) -> StoreResult<bool> {
        self.purge_one(account, now_ms).await
    }

    async fn record_event(&self, account: AccountId, event: AccountEvent) -> StoreResult<()> {
        with_client!(self, client => {
            insert_event(&*client, account, event)
                .await
                .map_err(|_| StoreError::Unknown)
        })
    }

    async fn events(&self, account: AccountId, limit: u32) -> StoreResult<Vec<AccountEvent>> {
        with_client!(self, client => {
            let rows = client
                .query(
                    "SELECT kind, at_ms, device_label FROM account_event WHERE account_id = $1 \
                     ORDER BY event_id DESC LIMIT $2",
                    &[&&account.as_bytes()[..], &i64::from(limit)],
                )
                .await
                .map_err(unavailable)?;
            rows.iter()
                .map(|row| {
                    let kind: String = row.try_get(0).map_err(unavailable)?;
                    let label: String = row.try_get(2).map_err(unavailable)?;
                    Ok(AccountEvent {
                        kind: EventKind::parse(&kind).ok_or(StoreError::InvalidRecord)?,
                        at_ms: to_u64(row.try_get(1).map_err(unavailable)?)?,
                        device: device(&label)?,
                    })
                })
                .collect()
        })
    }

    async fn claim_operation(
        &self,
        key: OperationKey,
        fingerprint: [u8; 32],
        now_ms: u64,
    ) -> StoreResult<OperationClaim> {
        with_client!(self, client => {
            let inserted = client
                .execute(
                    "INSERT INTO operation_receipt (namespace, principal, operation_id, \
                     fingerprint, created_at_ms) VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT DO NOTHING",
                    &[
                        &key.namespace.label(),
                        &&key.principal[..],
                        &&key.operation.as_bytes()[..],
                        &&fingerprint[..],
                        &to_i64(now_ms)?,
                    ],
                )
                .await
                .map_err(|_| StoreError::Unknown)?;
            if inserted == 1 {
                return Ok(OperationClaim::New);
            }
            let row = client
                .query_one(
                    "SELECT fingerprint, outcome FROM operation_receipt \
                     WHERE namespace = $1 AND principal = $2 AND operation_id = $3",
                    &[
                        &key.namespace.label(),
                        &&key.principal[..],
                        &&key.operation.as_bytes()[..],
                    ],
                )
                .await
                .map_err(unavailable)?;
            let stored: Vec<u8> = row.try_get(0).map_err(unavailable)?;
            if stored != fingerprint {
                return Ok(OperationClaim::Conflict);
            }
            let outcome: Option<String> = row.try_get(1).map_err(unavailable)?;
            Ok(match outcome.as_deref() {
                None => OperationClaim::InProgress,
                Some(label) if label == ReceiptOutcome::Done.label() => {
                    OperationClaim::Replayed(ReceiptOutcome::Done)
                }
                Some(_) => return Err(StoreError::InvalidRecord),
            })
        })
    }

    async fn finish_operation(
        &self,
        key: OperationKey,
        outcome: ReceiptOutcome,
    ) -> StoreResult<()> {
        with_client!(self, client => {
            client
                .execute(
                    "UPDATE operation_receipt SET outcome = $4 \
                     WHERE namespace = $1 AND principal = $2 AND operation_id = $3",
                    &[
                        &key.namespace.label(),
                        &&key.principal[..],
                        &&key.operation.as_bytes()[..],
                        &outcome.label(),
                    ],
                )
                .await
                .map_err(|_| StoreError::Unknown)?;
            Ok(())
        })
    }

    async fn release_operation(&self, key: OperationKey) -> StoreResult<()> {
        with_client!(self, client => {
            client
                .execute(
                    "DELETE FROM operation_receipt WHERE namespace = $1 AND principal = $2 \
                     AND operation_id = $3 AND outcome IS NULL",
                    &[
                        &key.namespace.label(),
                        &&key.principal[..],
                        &&key.operation.as_bytes()[..],
                    ],
                )
                .await
                .map_err(|_| StoreError::Unknown)?;
            Ok(())
        })
    }

    async fn purge(&self, cutoffs: RetentionCutoffs) -> StoreResult<PurgeCounts> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            oauth::purge_expired(&transaction, cutoffs.now_ms).await?;
            let sessions = transaction
                .execute(
                    "DELETE FROM web_session WHERE revoked_at_ms < $1 \
                     OR absolute_expires_at_ms < $1 OR last_seen_at_ms < $2",
                    &[
                        &to_i64(cutoffs.sessions_ended_before_ms)?,
                        &to_i64(cutoffs.sessions_last_seen_before_ms)?,
                    ],
                )
                .await
                .map_err(unavailable)?;
            let tokens_before = to_i64(cutoffs.tokens_before_ms)?;
            let tokens = transaction
                .execute(
                    "DELETE FROM account_token WHERE expires_at_ms < $1",
                    &[&tokens_before],
                )
                .await
                .map_err(unavailable)?;
            transaction
                .execute(
                    "DELETE FROM email_change c WHERE c.expires_at_ms < $1 AND NOT EXISTS \
                     (SELECT 1 FROM account_token t WHERE t.email_change_id = c.email_change_id)",
                    &[&tokens_before],
                )
                .await
                .map_err(unavailable)?;
            let receipts = transaction
                .execute(
                    "DELETE FROM operation_receipt WHERE created_at_ms < $1",
                    &[&to_i64(cutoffs.receipts_before_ms)?],
                )
                .await
                .map_err(unavailable)?;
            commit(transaction).await?;
            Ok(PurgeCounts {
                sessions,
                tokens,
                receipts,
                deleted_accounts: 0,
            })
        })
    }

    async fn summary(&self, recent: u32) -> StoreResult<AccountsSummary> {
        with_client!(self, client => {
            let mut summary = AccountsSummary::default();
            for row in client
                .query(
                    "SELECT status, count(*), count(email_verified_at_ms) FROM account \
                     GROUP BY status",
                    &[],
                )
                .await
                .map_err(unavailable)?
            {
                let status: String = row.try_get(0).map_err(unavailable)?;
                let count = to_u64(row.try_get(1).map_err(unavailable)?)?;
                summary.verified += to_u64(row.try_get(2).map_err(unavailable)?)?;
                match AccountStatus::parse(&status).ok_or(StoreError::InvalidRecord)? {
                    AccountStatus::Active => summary.active = count,
                    AccountStatus::Closing => summary.closing = count,
                    AccountStatus::Closed => summary.closed = count,
                }
            }
            let rows = client
                .query(
                    &format!(
                        "SELECT {ACCOUNT_COLUMNS}, \
                         (SELECT count(*) FROM web_session s WHERE s.account_id = a.account_id \
                          AND s.revoked_at_ms IS NULL), \
                         e.kind, e.at_ms, e.device_label \
                         FROM account a LEFT JOIN password_credential p USING (account_id) \
                         LEFT JOIN LATERAL (SELECT kind, at_ms, device_label FROM account_event \
                          WHERE account_id = a.account_id ORDER BY event_id DESC LIMIT 1) e ON true \
                         ORDER BY a.created_at_ms DESC, a.account_id LIMIT $1"
                    ),
                    &[&i64::from(recent)],
                )
                .await
                .map_err(unavailable)?;
            for row in rows {
                let account = account_from_row(&row, 0)?;
                let kind: Option<String> = row.try_get(11).map_err(unavailable)?;
                let last_event = match kind {
                    Some(kind) => {
                        let label: String = row.try_get(13).map_err(unavailable)?;
                        Some(AccountEvent {
                            kind: EventKind::parse(&kind).ok_or(StoreError::InvalidRecord)?,
                            at_ms: to_u64(row.try_get(12).map_err(unavailable)?)?,
                            device: device(&label)?,
                        })
                    }
                    None => None,
                };
                summary.recent.push(RecentAccount {
                    id: account.id,
                    email: account.email,
                    status: account.status,
                    is_verified: account.is_verified,
                    active_sessions: to_u64(row.try_get(10).map_err(unavailable)?)?,
                    last_event,
                });
            }
            Ok(summary)
        })
    }
}

#[cfg(test)]
impl PostgresAccountStore {
    /// Runs SQL for tests that seed the campaign model the closure scenarios use.
    pub(crate) async fn test_batch(&self, sql: &str) {
        let mut slot = self.pool.checkout().await.expect("test connection");
        let client = slot.client.as_mut().expect("test client");
        client.batch_execute(sql).await.expect("test sql");
    }

    pub(crate) async fn test_query(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Vec<tokio_postgres::Row> {
        let mut slot = self.pool.checkout().await.expect("test connection");
        let client = slot.client.as_mut().expect("test client");
        client.query(sql, params).await.expect("test query")
    }
}

#[cfg(test)]
#[path = "account_store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "account_closure_tests.rs"]
mod closure_tests;

async fn issue_tenant(
    transaction: &tokio_postgres::Transaction<'_>,
    account: AccountId,
) -> StoreResult<()> {
    transaction.execute(
        "INSERT INTO tenant (owner_account_id) VALUES ($1) ON CONFLICT (owner_account_id) DO NOTHING",
        &[&&account.as_bytes()[..]],
    ).await.map_err(|error| {
        df_observe::record_audit_line(
            "account.tenant_issuance_failed",
            format!(
                "ACCOUNT tenant_issuance_failed account={} sqlstate={}",
                df_customer::HexId::of(account.as_bytes()).as_str(),
                error.code().map_or("connection", |code| code.code()),
            ),
        );
        unavailable(error)
    })?;
    Ok(())
}
