//! Row mapping and the statements several store methods share.

use df_customer::{
    AccountEvent, AccountRecord, AccountStatus, DeviceLabel, DisplayName, EmailAddress, NewSession,
    NewToken, PasswordHash, RevokeReason, RevokeScope, StoreError, StoreResult,
};
use df_types::{AccountId, WebSessionId};
use tokio_postgres::{GenericClient, Row, Transaction};

pub(super) fn unavailable(_: tokio_postgres::Error) -> StoreError {
    StoreError::Unavailable
}

pub(super) fn to_i64(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn to_u64(value: i64) -> StoreResult<u64> {
    u64::try_from(value).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn account_id(bytes: &[u8]) -> StoreResult<AccountId> {
    AccountId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn session_id(bytes: &[u8]) -> StoreResult<WebSessionId> {
    WebSessionId::from_bytes(bytes).map_err(|_| StoreError::InvalidRecord)
}

pub(super) fn device(label: &str) -> StoreResult<DeviceLabel> {
    DeviceLabel::parse(label).ok_or(StoreError::InvalidRecord)
}

pub(super) async fn commit(transaction: Transaction<'_>) -> StoreResult<()> {
    transaction.commit().await.map_err(|_| StoreError::Unknown)
}

pub(super) const ACCOUNT_COLUMNS: &str = "a.account_id, a.email_normalized, a.email_display, \
    a.email_verified_at_ms, a.display_name, a.status, a.credential_generation, a.created_at_ms, \
    a.delete_after_ms, p.phc_hash";

pub(super) fn account_from_row(row: &Row, first: usize) -> StoreResult<AccountRecord> {
    let get_text = |index: usize| -> StoreResult<Option<String>> {
        row.try_get(first + index).map_err(unavailable)
    };
    let email = match (get_text(1)?, get_text(2)?) {
        (Some(normalized), Some(display)) => Some(EmailAddress::from_stored(normalized, display)),
        (None, None) => None,
        _ => return Err(StoreError::InvalidRecord),
    };
    let verified_at: Option<i64> = row.try_get(first + 3).map_err(unavailable)?;
    let status: String = row.try_get(first + 5).map_err(unavailable)?;
    let generation: i64 = row.try_get(first + 6).map_err(unavailable)?;
    let created: i64 = row.try_get(first + 7).map_err(unavailable)?;
    let delete_after: Option<i64> = row.try_get(first + 8).map_err(unavailable)?;
    let id: Vec<u8> = row.try_get(first).map_err(unavailable)?;
    Ok(AccountRecord {
        id: account_id(&id)?,
        email,
        is_verified: verified_at.is_some(),
        display_name: get_text(4)?.map(DisplayName::from_stored),
        status: AccountStatus::parse(&status).ok_or(StoreError::InvalidRecord)?,
        credential_generation: to_u64(generation)?,
        password_hash: get_text(9)?.map(PasswordHash::from_phc),
        created_at_ms: to_u64(created)?,
        delete_after_ms: delete_after.map(to_u64).transpose()?,
    })
}

pub(super) async fn insert_event(
    client: &impl GenericClient,
    account: AccountId,
    event: AccountEvent,
) -> StoreResult<()> {
    client
        .execute(
            "INSERT INTO account_event (account_id, kind, at_ms, device_label) \
             VALUES ($1, $2, $3, $4)",
            &[
                &&account.as_bytes()[..],
                &event.kind.label(),
                &to_i64(event.at_ms)?,
                &event.device.label(),
            ],
        )
        .await
        .map_err(unavailable)?;
    Ok(())
}

pub(super) async fn insert_token(client: &impl GenericClient, token: &NewToken) -> StoreResult<()> {
    let email_change = token.email_change.map(|id| id.to_vec());
    client
        .execute(
            "INSERT INTO account_token (token_id, token_digest, account_id, purpose, \
             email_change_id, credential_generation, created_at_ms, expires_at_ms, \
             attempt_limit) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &&token.id[..],
                &&token.digest.as_bytes()[..],
                &&token.account.as_bytes()[..],
                &token.purpose.label(),
                &email_change,
                &to_i64(token.credential_generation)?,
                &to_i64(token.created_at_ms)?,
                &to_i64(token.expires_at_ms)?,
                &i32::try_from(token.attempt_limit).map_err(|_| StoreError::InvalidRecord)?,
            ],
        )
        .await
        .map_err(unavailable)?;
    Ok(())
}

pub(super) async fn insert_session(
    client: &impl GenericClient,
    session: &NewSession,
) -> StoreResult<()> {
    client
        .execute(
            "INSERT INTO web_session (session_id, token_digest, account_id, \
             credential_generation, created_at_ms, last_seen_at_ms, absolute_expires_at_ms, \
             csrf_secret, device_label) VALUES ($1, $2, $3, $4, $5, $5, $6, $7, $8)",
            &[
                &&session.id.as_bytes()[..],
                &&session.digest.as_bytes()[..],
                &&session.account.as_bytes()[..],
                &to_i64(session.credential_generation)?,
                &to_i64(session.created_at_ms)?,
                &to_i64(session.absolute_expires_at_ms)?,
                &&session.csrf_secret.as_bytes()[..],
                &session.device.label(),
            ],
        )
        .await
        .map_err(unavailable)?;
    Ok(())
}

pub(super) async fn revoke(
    client: &impl GenericClient,
    account: AccountId,
    scope: RevokeScope,
    reason: RevokeReason,
    now_ms: u64,
) -> StoreResult<u64> {
    let account = &account.as_bytes()[..];
    let now = to_i64(now_ms)?;
    let base = "UPDATE web_session SET revoked_at_ms = $2, revoke_reason = $3 \
                WHERE account_id = $1 AND revoked_at_ms IS NULL";
    let revoked = match scope {
        RevokeScope::All => {
            client
                .execute(base, &[&account, &now, &reason.label()])
                .await
        }
        RevokeScope::One(id) => {
            client
                .execute(
                    &format!("{base} AND session_id = $4"),
                    &[&account, &now, &reason.label(), &&id.as_bytes()[..]],
                )
                .await
        }
        RevokeScope::AllExcept(id) => {
            client
                .execute(
                    &format!("{base} AND session_id <> $4"),
                    &[&account, &now, &reason.label(), &&id.as_bytes()[..]],
                )
                .await
        }
    };
    revoked.map_err(unavailable)
}
