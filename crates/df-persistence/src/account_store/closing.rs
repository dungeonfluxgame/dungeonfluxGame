//! Account closure and its completion (J14; `planning/campaigns-and-table-research.md` 8.7).
//!
//! Closing is one transaction: the account row is locked and set `closing`, the campaign owner's
//! participant says what blocks, blockers roll it back, otherwise it commits together with the
//! scheduled deletions. Completion runs after the window, one account at a time: a check under the
//! account row lock, the campaign owner's chunked `leave_all` (each chunk its own transaction),
//! and only then, in a transaction of its own, the deletion of the account's personal data. The
//! service deletes the identity provider's user between the last two, and only for an account the
//! first two passed.

use df_customer::{
    AccountEvent, CloseOutcome, ClosureCheck, CompletionOutcome, DeviceLabel, EventKind,
    LeaveChunk, RevokeReason, RevokeScope, StoreResult,
};
use df_types::{AccountId, CampaignId};

use super::{
    PostgresAccountStore,
    pool::{begin_transaction, with_client},
    rows::*,
};
use crate::account_closure;

/// Chunks one completion attempt runs before it yields; the next pass continues. Each chunk
/// leaves a bounded number of campaigns, so a very large membership list cannot hold one pass.
const MAXIMUM_CHUNKS_PER_ATTEMPT: usize = 50;

/// What one completion chunk found.
enum Chunk {
    NotDue,
    Blocked(ClosureCheck),
    Step(LeaveChunk),
}

impl PostgresAccountStore {
    pub(super) async fn close(
        &self,
        account: AccountId,
        now_ms: u64,
        delete_after_ms: u64,
    ) -> StoreResult<CloseOutcome> {
        let participant = self.participant();
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            // The account row is the first lock: a concurrent transfer acceptance either commits
            // before this or finds the account `closing`.
            let row = transaction
                .query_opt(
                    "SELECT status, delete_after_ms FROM account WHERE account_id = $1 \
                     FOR UPDATE",
                    &[&&account.as_bytes()[..]],
                )
                .await
                .map_err(unavailable)?;
            let Some(row) = row else {
                return Ok(CloseOutcome::Blocked(ClosureCheck::default()));
            };
            let status: String = row.try_get(0).map_err(unavailable)?;
            if status == "closing" {
                let at: Option<i64> = row.try_get(1).map_err(unavailable)?;
                return Ok(CloseOutcome::AlreadyClosing {
                    delete_after_ms: at.map(to_u64).transpose()?.unwrap_or(delete_after_ms),
                });
            }
            if status != "active" {
                return Ok(CloseOutcome::Blocked(ClosureCheck::default()));
            }
            transaction
                .execute(
                    "UPDATE account SET status = 'closing', delete_after_ms = $2 \
                     WHERE account_id = $1",
                    &[&&account.as_bytes()[..], &to_i64(delete_after_ms)?],
                )
                .await
                .map_err(unavailable)?;
            let check = account_closure::closure_check(participant, &transaction, account).await?;
            if !check.blocking.is_empty() {
                // Dropping the transaction rolls it back: the account is not `closing`.
                return Ok(CloseOutcome::Blocked(check));
            }
            let scheduled =
                account_closure::schedule(participant, &transaction, account, delete_after_ms)
                    .await?;
            revoke(
                &transaction,
                account,
                RevokeScope::All,
                RevokeReason::AccountClosed,
                now_ms,
            )
            .await?;
            insert_event(
                &transaction,
                account,
                AccountEvent {
                    kind: EventKind::Closed,
                    at_ms: now_ms,
                    device: DeviceLabel::Unknown,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(CloseOutcome::Closed {
                delete_after_ms,
                scheduled,
            })
        })
    }

    /// Reopens a closing account whose window has not passed. Exactly one row must change; any
    /// other count rolls back and answers `false`.
    pub(super) async fn reopen(&self, account: AccountId, now_ms: u64) -> StoreResult<bool> {
        let participant = self.participant();
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let reopened = transaction
                .execute(
                    "UPDATE account SET status = 'active', delete_after_ms = NULL \
                     WHERE account_id = $1 AND status = 'closing' AND delete_after_ms > $2",
                    &[&&account.as_bytes()[..], &to_i64(now_ms)?],
                )
                .await
                .map_err(unavailable)?;
            if reopened != 1 {
                return Ok(false);
            }
            account_closure::cancel(participant, &transaction, account).await?;
            insert_event(
                &transaction,
                account,
                AccountEvent {
                    kind: EventKind::Reopened,
                    at_ms: now_ms,
                    device: DeviceLabel::Unknown,
                },
            )
            .await?;
            commit(transaction).await?;
            Ok(true)
        })
    }

    pub(super) async fn due(&self, now_ms: u64, limit: u32) -> StoreResult<Vec<AccountId>> {
        with_client!(self, client => {
            let rows = client
                .query(
                    "SELECT account_id FROM account WHERE status = 'closing' \
                     AND delete_after_ms <= $1 ORDER BY delete_after_ms, account_id LIMIT $2",
                    &[&to_i64(now_ms)?, &i64::from(limit)],
                )
                .await
                .map_err(unavailable)?;
            rows.iter()
                .map(|row| account_id(&row.try_get::<_, Vec<u8>>(0).map_err(unavailable)?))
                .collect()
        })
    }

    /// One completion chunk: its own transaction, which starts by locking the account row and
    /// checking the account is still `closing` and due. The first chunk also re-checks the
    /// blockers (a transfer may have made the account a host since it closed).
    async fn chunk(
        &self,
        account: AccountId,
        now_ms: u64,
        after: Option<CampaignId>,
    ) -> StoreResult<Chunk> {
        let participant = self.participant();
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let due = transaction
                .query_opt(
                    "SELECT 1 FROM account WHERE account_id = $1 AND status = 'closing' \
                     AND delete_after_ms <= $2 FOR UPDATE",
                    &[&&account.as_bytes()[..], &to_i64(now_ms)?],
                )
                .await
                .map_err(unavailable)?;
            if due.is_none() {
                return Ok(Chunk::NotDue);
            }
            if after.is_none() {
                let check =
                    account_closure::closure_check(participant, &transaction, account).await?;
                if !check.blocking.is_empty() {
                    return Ok(Chunk::Blocked(check));
                }
            }
            let step =
                account_closure::leave_chunk(participant, &transaction, account, after).await?;
            commit(transaction).await?;
            Ok(Chunk::Step(step))
        })
    }

    pub(super) async fn complete(
        &self,
        account: AccountId,
        now_ms: u64,
    ) -> StoreResult<CompletionOutcome> {
        let mut after = None;
        for _ in 0..MAXIMUM_CHUNKS_PER_ATTEMPT {
            match self.chunk(account, now_ms, after).await? {
                Chunk::NotDue => return Ok(CompletionOutcome::NotDue),
                Chunk::Blocked(check) => return Ok(CompletionOutcome::Blocked(check)),
                Chunk::Step(LeaveChunk::Done) => return Ok(CompletionOutcome::Completed),
                Chunk::Step(LeaveChunk::Continue(last)) => after = Some(last),
            }
        }
        Ok(CompletionOutcome::Continues)
    }

    /// Deletes one completed account's personal data, in one transaction, if it is still
    /// `closing`, due, and has no provider link left.
    pub(super) async fn purge_one(&self, account: AccountId, now_ms: u64) -> StoreResult<bool> {
        with_client!(self, client => {
            let transaction = begin_transaction(client).await.map_err(unavailable)?;
            let ready = transaction
                .query_opt(
                    "SELECT 1 FROM account a WHERE a.account_id = $1 AND a.status = 'closing' \
                     AND a.delete_after_ms <= $2 AND NOT EXISTS \
                     (SELECT 1 FROM identity_link l WHERE l.account_id = a.account_id) \
                     FOR UPDATE OF a",
                    &[&&account.as_bytes()[..], &to_i64(now_ms)?],
                )
                .await
                .map_err(unavailable)?;
            if ready.is_none() {
                return Ok(false);
            }
            let id = account.as_bytes().to_vec();
            for statement in [
                "DELETE FROM web_session WHERE account_id = $1",
                "DELETE FROM account_token WHERE account_id = $1",
                "DELETE FROM email_change WHERE account_id = $1",
                "DELETE FROM account_event WHERE account_id = $1",
                "DELETE FROM password_credential WHERE account_id = $1",
                "UPDATE account SET email_normalized = NULL, email_display = NULL, \
                 display_name = NULL, email_verified_at_ms = NULL, status = 'closed', \
                 delete_after_ms = NULL WHERE account_id = $1",
            ] {
                transaction
                    .execute(statement, &[&id])
                    .await
                    .map_err(unavailable)?;
            }
            commit(transaction).await?;
            Ok(true)
        })
    }
}
