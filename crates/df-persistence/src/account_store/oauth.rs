//! Google links and single-use OIDC attempts in the account store's existing bounded pool.

use df_customer::{
    AccountRecord, AccountStatus, GoogleResolution, GoogleResolved, GoogleSession, GoogleStore,
    GoogleStoreCall, OAuthAttempt, OAuthIntent, OAuthRefusal, OAuthSecret, StoreError, StoreResult,
    TokenDigest,
};
use df_types::{AccountId, WebSessionId};
use tokio_postgres::Transaction;

use super::{
    PostgresAccountStore,
    pool::{begin_transaction, with_client},
    rows::*,
};

const MAXIMUM_ATTEMPTS: i64 = 10_000;
const MAXIMUM_CLIENT_ATTEMPTS: i64 = 8;

pub(super) async fn purge_expired(transaction: &Transaction<'_>, now_ms: u64) -> StoreResult<()> {
    transaction
        .execute(
            "DELETE FROM oauth_attempt WHERE state_digest IN \
         (SELECT state_digest FROM oauth_attempt WHERE expires_at_ms <= $1 \
          ORDER BY expires_at_ms LIMIT 1024)",
            &[&to_i64(now_ms)?],
        )
        .await
        .map_err(unavailable)?;
    Ok(())
}

async fn lock_links(transaction: &Transaction<'_>) -> StoreResult<()> {
    transaction
        .query_one("SELECT pg_advisory_xact_lock(70000292)", &[])
        .await
        .map_err(unavailable)?;
    Ok(())
}

async fn account_for_subject(
    transaction: &Transaction<'_>,
    subject: &str,
) -> StoreResult<Option<AccountRecord>> {
    transaction
        .query_opt(
            &format!(
                "SELECT {ACCOUNT_COLUMNS} FROM google_identity g JOIN account a USING (account_id) \
         LEFT JOIN password_credential p USING (account_id) WHERE g.subject = $1 FOR UPDATE OF a"
            ),
            &[&subject],
        )
        .await
        .map_err(unavailable)?
        .map(|row| account_from_row(&row, 0))
        .transpose()
}

async fn account_for_id(
    transaction: &Transaction<'_>,
    account: AccountId,
) -> StoreResult<Option<AccountRecord>> {
    transaction.query_opt(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM account a LEFT JOIN password_credential p USING (account_id) \
         WHERE a.account_id = $1 FOR UPDATE OF a"
    ), &[&&account.as_bytes()[..]]).await.map_err(unavailable)?.map(|row| account_from_row(&row,0)).transpose()
}

async fn origin_is_live(
    transaction: &Transaction<'_>,
    session: Option<WebSessionId>,
    account: AccountId,
    generation: u64,
    now_ms: u64,
    idle_before_ms: u64,
) -> StoreResult<bool> {
    let Some(session) = session else {
        return Ok(true);
    };
    Ok(transaction
        .query_opt(
            "SELECT 1 FROM web_session WHERE session_id = $1 AND account_id = $2 \
         AND credential_generation = $3 AND revoked_at_ms IS NULL \
         AND absolute_expires_at_ms > $4 AND last_seen_at_ms >= $5 FOR SHARE",
            &[
                &&session.as_bytes()[..],
                &&account.as_bytes()[..],
                &to_i64(generation)?,
                &to_i64(now_ms)?,
                &to_i64(idle_before_ms)?,
            ],
        )
        .await
        .map_err(unavailable)?
        .is_some())
}

fn identity_refusal(account: &AccountRecord, request: &GoogleResolution) -> Option<OAuthRefusal> {
    let may_reopen = !request.intent.requires_fresh()
        && account.status == AccountStatus::Closing
        && account
            .delete_after_ms
            .is_some_and(|end| request.now_ms < end);
    if (account.status != AccountStatus::Active && !may_reopen) || !account.is_verified {
        return Some(OAuthRefusal::AccountUnavailable);
    }
    if account.email.as_ref().map(|email| email.normalized()) != Some(request.email.normalized()) {
        return Some(OAuthRefusal::Conflict);
    }
    if request.intent.requires_fresh()
        && (request.account != Some(account.id)
            || request.credential_generation != Some(account.credential_generation))
    {
        return Some(OAuthRefusal::ReauthenticationRequired);
    }
    None
}

async fn resolve_identity(
    transaction: &Transaction<'_>,
    request: GoogleResolution,
) -> StoreResult<GoogleResolved> {
    lock_links(transaction).await?;
    if let Some(account) = account_for_subject(transaction, &request.subject).await? {
        if let Some(reason) = identity_refusal(&account, &request) {
            return Ok(GoogleResolved::Refused(reason));
        }
        if !origin_is_live(
            transaction,
            request.originating_session,
            account.id,
            account.credential_generation,
            request.now_ms,
            request.idle_before_ms,
        )
        .await?
        {
            return Ok(GoogleResolved::Refused(
                OAuthRefusal::ReauthenticationRequired,
            ));
        }
        return Ok(GoogleResolved::Account(account));
    }
    match request.intent {
        OAuthIntent::SignIn | OAuthIntent::Reauthenticate => {
            Ok(GoogleResolved::Refused(OAuthRefusal::NoAccount))
        }
        OAuthIntent::Link => {
            let Some(id) = request.account else {
                return Ok(GoogleResolved::Refused(OAuthRefusal::Conflict));
            };
            let Some(account) = account_for_id(transaction, id).await? else {
                return Ok(GoogleResolved::Refused(OAuthRefusal::Conflict));
            };
            if let Some(reason) = identity_refusal(&account, &request) {
                return Ok(GoogleResolved::Refused(reason));
            }
            if !origin_is_live(
                transaction,
                request.originating_session,
                id,
                account.credential_generation,
                request.now_ms,
                request.idle_before_ms,
            )
            .await?
            {
                return Ok(GoogleResolved::Refused(
                    OAuthRefusal::ReauthenticationRequired,
                ));
            }
            let inserted = transaction.execute(
                "INSERT INTO google_identity(subject,account_id,linked_at_ms) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
                &[&request.subject,&&id.as_bytes()[..],&to_i64(request.now_ms)?],
            ).await.map_err(unavailable)?;
            if inserted == 0 {
                return Ok(GoogleResolved::Refused(OAuthRefusal::Conflict));
            }
            Ok(GoogleResolved::Account(account))
        }
        OAuthIntent::SignUp => {
            let terms = request
                .terms_version
                .as_deref()
                .ok_or(StoreError::InvalidRecord)?;
            let inserted = transaction.execute(
                "INSERT INTO account(account_id,email_normalized,email_display,email_verified_at_ms, \
                 status,credential_generation,created_at_ms,adult_attested_at,terms_version,terms_accepted_at) \
                 VALUES ($1,$2,$3,$4,'active',1,$4,$5,$6,$5) ON CONFLICT (email_normalized) DO NOTHING",
                &[&&request.new_account.as_bytes()[..],&request.email.normalized(),&request.email.display(),
                  &to_i64(request.now_ms)?,&to_i64(request.accepted_at_ms)?,&terms],
            ).await.map_err(unavailable)?;
            if inserted == 0 {
                return Ok(GoogleResolved::Refused(
                    OAuthRefusal::PasswordSignInRequired,
                ));
            }
            super::issue_tenant(transaction, request.new_account).await?;
            transaction.execute("INSERT INTO google_identity(subject,account_id,linked_at_ms) VALUES ($1,$2,$3)",
                &[&request.subject,&&request.new_account.as_bytes()[..],&to_i64(request.now_ms)?],
            ).await.map_err(unavailable)?;
            insert_event(
                transaction,
                request.new_account,
                df_customer::AccountEvent {
                    kind: df_customer::EventKind::SignedUp,
                    at_ms: request.now_ms,
                    device: df_customer::DeviceLabel::Unknown,
                },
            )
            .await?;
            let account = account_for_id(transaction, request.new_account)
                .await?
                .ok_or(StoreError::InvalidRecord)?;
            Ok(GoogleResolved::Created(account))
        }
    }
}

impl GoogleStore for PostgresAccountStore {
    fn save_attempt(&self, attempt: OAuthAttempt) -> GoogleStoreCall<'_, bool> {
        Box::pin(async move {
            with_client!(self, client => {
                let transaction = begin_transaction(client).await.map_err(unavailable)?;
                transaction.query_one("SELECT pg_advisory_xact_lock(70000291)",&[]).await.map_err(unavailable)?;
                purge_expired(&transaction,attempt.created_at_ms).await?;
                let row = transaction.query_one(
                    "SELECT count(*), count(*) FILTER (WHERE client_digest = $1) FROM oauth_attempt",
                    &[&&attempt.client_digest[..]],
                ).await.map_err(unavailable)?;
                let total:i64=row.try_get(0).map_err(unavailable)?;
                let client_count:i64=row.try_get(1).map_err(unavailable)?;
                if total >= MAXIMUM_ATTEMPTS || client_count >= MAXIMUM_CLIENT_ATTEMPTS {
                    commit(transaction).await?; return Ok(false);
                }
                transaction.execute(
                    "INSERT INTO oauth_attempt(state_digest,cookie_digest,client_digest,nonce,verifier,intent, \
                 account_id,session_id,credential_generation,terms_version,created_at_ms,expires_at_ms,redirect_uri) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                    &[&&attempt.state_digest.as_bytes()[..],&&attempt.cookie_digest.as_bytes()[..],&&attempt.client_digest[..],
                      &attempt.nonce.expose(),&attempt.verifier.expose(),&attempt.intent.label(),
                      &attempt.account.map(|id| id.as_bytes().to_vec()),&attempt.session.map(|id| id.as_bytes().to_vec()),
                      &attempt.credential_generation.map(to_i64).transpose()?,&attempt.terms_version,
                  &to_i64(attempt.created_at_ms)?,&to_i64(attempt.expires_at_ms)?,&attempt.redirect_uri],
                ).await.map_err(unavailable)?;
                commit(transaction).await?; Ok(true)
            })
        })
    }

    fn claim_attempt(
        &self,
        state: TokenDigest,
        cookie: TokenDigest,
        now_ms: u64,
    ) -> GoogleStoreCall<'_, Option<OAuthAttempt>> {
        Box::pin(async move {
            with_client!(self, client => {
                let transaction=begin_transaction(client).await.map_err(unavailable)?;
                let Some(row)=transaction.query_opt(
                    "DELETE FROM oauth_attempt WHERE state_digest = $1 AND cookie_digest = $2 \
                     AND expires_at_ms > $3 RETURNING client_digest,nonce,verifier,intent,account_id, \
                 session_id,credential_generation,terms_version,created_at_ms,expires_at_ms,redirect_uri",
                    &[&&state.as_bytes()[..],&&cookie.as_bytes()[..],&to_i64(now_ms)?],
                ).await.map_err(unavailable)? else { return Ok(None); };
                let client:Vec<u8>=row.try_get(0).map_err(unavailable)?;
                let account:Option<Vec<u8>>=row.try_get(4).map_err(unavailable)?;
                let session:Option<Vec<u8>>=row.try_get(5).map_err(unavailable)?;
                let generation:Option<i64>=row.try_get(6).map_err(unavailable)?;
                let intent:String=row.try_get(3).map_err(unavailable)?;
                let attempt=OAuthAttempt {
                redirect_uri:row.try_get(10).map_err(unavailable)?,
                    state_digest:state,cookie_digest:cookie,
                    client_digest:client.try_into().map_err(|_| StoreError::InvalidRecord)?,
                    nonce:OAuthSecret::new(row.try_get(1).map_err(unavailable)?),
                    verifier:OAuthSecret::new(row.try_get(2).map_err(unavailable)?),
                    intent:OAuthIntent::parse(&intent).ok_or(StoreError::InvalidRecord)?,
                    account:account.as_deref().map(account_id).transpose()?,
                    session:session.as_deref().map(session_id).transpose()?,
                    credential_generation:generation.map(to_u64).transpose()?,
                    terms_version:row.try_get(7).map_err(unavailable)?,
                    created_at_ms:to_u64(row.try_get(8).map_err(unavailable)?)?,
                    expires_at_ms:to_u64(row.try_get(9).map_err(unavailable)?)?,
                };
                commit(transaction).await?; Ok(Some(attempt))
            })
        })
    }

    fn resolve(&self, request: GoogleResolution) -> GoogleStoreCall<'_, GoogleResolved> {
        Box::pin(async move {
            with_client!(self, client => {
                let transaction=begin_transaction(client).await.map_err(unavailable)?;
                let outcome=resolve_identity(&transaction,request).await?;
                commit(transaction).await?; Ok(outcome)
            })
        })
    }

    fn is_linked(&self, account: AccountId) -> GoogleStoreCall<'_, bool> {
        Box::pin(async move {
            with_client!(self, client => {
                Ok(client.query_opt("SELECT 1 FROM google_identity WHERE account_id = $1",&[&&account.as_bytes()[..]]).await.map_err(unavailable)?.is_some())
            })
        })
    }

    fn unlink(
        &self,
        account: AccountId,
        generation: u64,
    ) -> GoogleStoreCall<'_, Result<(), OAuthRefusal>> {
        Box::pin(async move {
            with_client!(self, client => {
                let transaction=begin_transaction(client).await.map_err(unavailable)?;
                lock_links(&transaction).await?;
                let Some(row)=account_for_id(&transaction,account).await? else { return Ok(Err(OAuthRefusal::Conflict)); };
                if row.status != AccountStatus::Active || row.credential_generation != generation { return Ok(Err(OAuthRefusal::Conflict)); }
                let password_provider=transaction.query_opt("SELECT 1 FROM identity_link WHERE account_id = $1 AND provider = 'workos'",&[&&account.as_bytes()[..]]).await.map_err(unavailable)?.is_some();
                if row.password_hash.is_none() && !password_provider { return Ok(Err(OAuthRefusal::LastSignInMethod)); }
                transaction.execute("DELETE FROM google_identity WHERE account_id = $1",&[&&account.as_bytes()[..]]).await.map_err(unavailable)?;
                commit(transaction).await?; Ok(Ok(()))
            })
        })
    }

    fn create_google_session(&self, request: GoogleSession) -> GoogleStoreCall<'_, bool> {
        Box::pin(async move {
            with_client!(self, client => {
                let transaction=begin_transaction(client).await.map_err(unavailable)?;
                lock_links(&transaction).await?;
                let session=request.session;
                let Some(account)=account_for_id(&transaction,session.account).await? else { return Ok(false); };
                if transaction.query_opt("SELECT 1 FROM google_identity WHERE subject = $1 AND account_id = $2", &[&request.subject,&&session.account.as_bytes()[..]]).await.map_err(unavailable)?.is_none() { return Ok(false); }
                if account.status != AccountStatus::Active || account.credential_generation != session.credential_generation
                    || !origin_is_live(&transaction,request.originating_session,account.id,account.credential_generation,session.created_at_ms,request.idle_before_ms).await?
                { return Ok(false); }
                insert_session(&transaction,&session).await?;
                transaction.execute("INSERT INTO google_session_proof(session_id,reauthentication_expires_at_ms) VALUES ($1,$2)",
                    &[&&session.id.as_bytes()[..],&request.reauthentication_expires_at_ms.map(to_i64).transpose()?],
                ).await.map_err(unavailable)?;
                insert_event(&transaction,session.account,request.event).await?;
                commit(transaction).await?; Ok(true)
            })
        })
    }

    fn session_proof(&self, session: WebSessionId) -> GoogleStoreCall<'_, Option<Option<u64>>> {
        Box::pin(async move {
            with_client!(self, client => {
                client.query_opt("SELECT reauthentication_expires_at_ms FROM google_session_proof WHERE session_id = $1",&[&&session.as_bytes()[..]]).await.map_err(unavailable)?
                    .map(|row| row.try_get::<_,Option<i64>>(0).map_err(unavailable)?.map(to_u64).transpose()).transpose()
            })
        })
    }
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
