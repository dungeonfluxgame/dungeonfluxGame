//! The retention pass (`service-operations.md` retention classes): expired sessions and tokens,
//! receipts past 90 days, and the personal data of closed accounts whose reopening window ended.
//! One owned task; each pass is one bounded store call.

use std::{sync::Arc, time::Duration};

use df_types::AccountId;

use crate::{
    closure::CompletionOutcome,
    ports::AccountStore,
    records::RetentionCutoffs,
    service::{
        PROVIDER_DEADLINE, RETENTION_INTERVAL, STORE_DEADLINE, Shared, account_label, audit,
    },
};

/// Due accounts closed out per retention pass; the rest wait for the next.
const ACCOUNTS_PER_PASS: u32 = 20;
/// Store deadlines a completion (a handful of chunk transactions) may use in all.
const CHUNK_BUDGET: u32 = 6;
/// Revoked or expired sessions are kept this long for the security history, then deleted.
pub const SESSION_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Expired tokens are kept this long, then deleted.
pub const TOKEN_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
/// Operation receipts are kept this long; older operation ids are refused as expired.
pub const RECEIPT_RETENTION: Duration = Duration::from_secs(90 * 24 * 60 * 60);

fn ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Sessions last used before this are idle-expired now.
pub(crate) fn idle_before_ms(now_ms: u64, idle: Duration) -> u64 {
    now_ms.saturating_sub(ms(idle))
}

pub(crate) fn cutoffs(now_ms: u64, idle: Duration) -> RetentionCutoffs {
    let ended_before_ms = now_ms.saturating_sub(ms(SESSION_RETENTION));
    RetentionCutoffs {
        now_ms,
        sessions_ended_before_ms: ended_before_ms,
        sessions_last_seen_before_ms: ended_before_ms.saturating_sub(ms(idle)),
        tokens_before_ms: now_ms.saturating_sub(ms(TOKEN_RETENTION)),
        receipts_before_ms: now_ms.saturating_sub(ms(RECEIPT_RETENTION)),
    }
}

pub(crate) async fn run<S: AccountStore, M>(shared: Arc<Shared<S, M>>) {
    let mut interval = tokio::time::interval(RETENTION_INTERVAL);
    loop {
        interval.tick().await;
        pass(&shared).await;
    }
}

/// One retention pass: the accounts whose closing window has passed, one at a time, then the
/// general cleanup. Failures are per account: it is audited and skipped, and the pass goes on.
pub(crate) async fn pass<S: AccountStore, M>(shared: &Shared<S, M>) {
    close_out_due_accounts(shared).await;
    let cutoffs = cutoffs(shared.clock.now_ms(), shared.tunables.sessions.idle);
    match tokio::time::timeout(STORE_DEADLINE, shared.store.purge(cutoffs)).await {
        Ok(Ok(counts)) => audit(
            "account.retention",
            format!(
                "ACCOUNT retention sessions={} tokens={} receipts={}",
                counts.sessions, counts.tokens, counts.receipts
            ),
        ),
        Ok(Err(_)) | Err(_) => audit(
            "account.retention",
            "ACCOUNT retention_failed reason=store_unavailable".to_owned(),
        ),
    }
}

/// Closes out at most `ACCOUNTS_PER_PASS` due accounts. For each, in order: the completion check
/// and the campaign side (blockers keep the account `closing` and delete nothing), then the
/// provider's user (deleted, and its link dropped, only for an account that passed), then the
/// account's personal data.
async fn close_out_due_accounts<S: AccountStore, M>(shared: &Shared<S, M>) {
    let due = match tokio::time::timeout(
        STORE_DEADLINE,
        shared
            .store
            .due_closures(shared.clock.now_ms(), ACCOUNTS_PER_PASS),
    )
    .await
    {
        Ok(Ok(due)) => due,
        Ok(Err(_)) | Err(_) => {
            audit(
                "account.closure_completion",
                "ACCOUNT closure_completion_failed reason=store_unavailable".to_owned(),
            );
            return;
        }
    };
    for account in due {
        let outcome = close_out(shared, account).await;
        audit(
            "account.closure_completion",
            format!(
                "ACCOUNT closure_completion account={} outcome={outcome}",
                account_label(account)
            ),
        );
    }
}

async fn close_out<S: AccountStore, M>(shared: &Shared<S, M>, account: AccountId) -> &'static str {
    let now_ms = shared.clock.now_ms();
    match tokio::time::timeout(
        STORE_DEADLINE * CHUNK_BUDGET,
        shared.store.complete_closure(account, now_ms),
    )
    .await
    {
        Ok(Ok(CompletionOutcome::Completed)) => {}
        Ok(Ok(CompletionOutcome::Blocked(_))) => return "blocked",
        Ok(Ok(CompletionOutcome::NotDue)) => return "not_due",
        Ok(Ok(CompletionOutcome::Continues)) => return "continues",
        Ok(Err(_)) | Err(_) => return "completion_failed",
    }
    if !delete_provider_users(shared, account).await {
        return "provider_user_kept";
    }
    match tokio::time::timeout(STORE_DEADLINE, shared.store.purge_closed(account, now_ms)).await {
        Ok(Ok(true)) => "purged",
        Ok(Ok(false)) => "not_purged",
        Ok(Err(_)) | Err(_) => "purge_failed",
    }
}

/// Deletes the account's provider users and drops their links. `false` when any is kept (a
/// provider failure, or a store failure dropping the link): the account is retried next pass.
async fn delete_provider_users<S: AccountStore, M>(
    shared: &Shared<S, M>,
    account: AccountId,
) -> bool {
    let Some(provider) = shared.identity.erased() else {
        return true;
    };
    let identities =
        match tokio::time::timeout(STORE_DEADLINE, shared.store.identities_of(account)).await {
            Ok(Ok(identities)) => identities,
            Ok(Err(_)) | Err(_) => return false,
        };
    for (kind, subject) in identities {
        let deleted = tokio::time::timeout(PROVIDER_DEADLINE, provider.delete_user(&subject)).await;
        let unlinked = matches!(deleted, Ok(Ok(())))
            && matches!(
                tokio::time::timeout(
                    STORE_DEADLINE,
                    shared.store.unlink_identity(account, kind, &subject),
                )
                .await,
                Ok(Ok(()))
            );
        audit(
            "account.provider_user_deleted",
            format!(
                "ACCOUNT provider_user_deleted account={} provider={} done={unlinked}",
                account_label(account),
                kind.label()
            ),
        );
        if !unlinked {
            return false;
        }
    }
    true
}
