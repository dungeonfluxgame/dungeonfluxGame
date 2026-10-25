//! Account closure on PostgreSQL (J14; `planning/campaigns-and-table-research.md` 8.7). The close
//! transaction takes the account row first (the lock `accounts.lock_account_status` shares with
//! the campaign owner's transfer acceptance), then asks the campaign owner's participant, in the
//! same transaction, which campaigns block. Blockers roll the transaction back. Otherwise the
//! participant schedules the deletion of the owned archived campaigns and cancels the account's
//! outgoing transfer proposals, and `closing` commits together with that.
//!
//! The participant runs on this store's connection and role, so the campaign owner grants that
//! role `EXECUTE` on the functions it calls (or the participant uses definer functions); the
//! lock order is accounts first, then campaigns (decision 29).

use std::{future::Future, pin::Pin};

use df_customer::{ClosureCheck, LeaveChunk, ScheduledDeletion, StoreResult};
use df_types::{AccountId, CampaignId};
use tokio_postgres::Transaction;

pub type ParticipantCall<'a, T> = Pin<Box<dyn Future<Output = StoreResult<T>> + Send + 'a>>;

/// The campaign owner's side of account closure, called inside this store's transaction.
/// Every method that touches several campaigns locks them in `campaign_id` order.
pub trait ClosureParticipant: Send + Sync + 'static {
    /// Blocking campaigns (active, owned or hosted) and owned archived campaigns.
    fn closure_campaigns<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        account: AccountId,
    ) -> ParticipantCall<'a, ClosureCheck>;

    /// Sets the deletion date of each owned archived campaign, deletes the account's drafts,
    /// cancels its outgoing transfer proposals and reports who to tell.
    fn schedule_archived_deletions<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        account: AccountId,
        delete_after_ms: u64,
    ) -> ParticipantCall<'a, Vec<ScheduledDeletion>>;

    /// Clears the deletion dates the closing set.
    fn cancel_scheduled_deletions<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        account: AccountId,
    ) -> ParticipantCall<'a, ()>;

    /// Leaves up to a bounded number of the campaigns the account only played in, in
    /// `campaign_id` order after `after`, in this transaction (B has locked the account row and
    /// checked it is still `closing`). `Continue(last)` says where the next chunk starts.
    fn leave_chunk<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        account: AccountId,
        after: Option<CampaignId>,
    ) -> ParticipantCall<'a, LeaveChunk>;
}

/// What a missing participant means: no campaigns exist (development without the campaign
/// service). Hosted composition always registers one.
pub(crate) async fn closure_check(
    participant: Option<&dyn ClosureParticipant>,
    transaction: &Transaction<'_>,
    account: AccountId,
) -> StoreResult<ClosureCheck> {
    match participant {
        Some(participant) => participant.closure_campaigns(transaction, account).await,
        None => Ok(ClosureCheck::default()),
    }
}

pub(crate) async fn schedule(
    participant: Option<&dyn ClosureParticipant>,
    transaction: &Transaction<'_>,
    account: AccountId,
    delete_after_ms: u64,
) -> StoreResult<Vec<ScheduledDeletion>> {
    match participant {
        Some(participant) => {
            participant
                .schedule_archived_deletions(transaction, account, delete_after_ms)
                .await
        }
        None => Ok(Vec::new()),
    }
}

pub(crate) async fn cancel(
    participant: Option<&dyn ClosureParticipant>,
    transaction: &Transaction<'_>,
    account: AccountId,
) -> StoreResult<()> {
    match participant {
        Some(participant) => {
            participant
                .cancel_scheduled_deletions(transaction, account)
                .await
        }
        None => Ok(()),
    }
}

pub(crate) async fn leave_chunk(
    participant: Option<&dyn ClosureParticipant>,
    transaction: &Transaction<'_>,
    account: AccountId,
    after: Option<CampaignId>,
) -> StoreResult<LeaveChunk> {
    match participant {
        Some(participant) => participant.leave_chunk(transaction, account, after).await,
        None => Ok(LeaveChunk::Done),
    }
}

/// Why a store refused to open for a timeout policy reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutRefusal {
    /// The role's effective `transaction_timeout` is unset (0) or could not be read.
    NotSet,
    /// It is longer than the policy's maximum.
    TooLong,
}

/// The `transaction_timeout` the opened role must have by default. The closure transaction bumps
/// campaign event counters; a transaction that outlives the feed readers' overlap could commit an
/// event they have already read past (`campaigns-and-table-research.md` 5.1), so the bound is a
/// role setting, not a per-transaction request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutPolicy {
    /// Hosted composition: positive and at most this many milliseconds, or refuse to start.
    AtMostMs(u64),
    /// Local development and tests on a scratch server whose roles are not provisioned.
    NotChecked,
}

/// Milliseconds in a `SHOW transaction_timeout` answer (`0`, `10s`, `5min`, `250ms`, `1h`).
pub(crate) fn timeout_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let (number, unit) = text.split_at(digits);
    let number: u64 = number.parse().ok()?;
    let factor = match unit.trim() {
        "" | "ms" => 1,
        "s" => 1_000,
        "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    number.checked_mul(factor)
}

pub(crate) fn check_timeout(policy: TimeoutPolicy, reported: &str) -> Result<(), TimeoutRefusal> {
    let TimeoutPolicy::AtMostMs(limit) = policy else {
        return Ok(());
    };
    match timeout_ms(reported) {
        None | Some(0) => Err(TimeoutRefusal::NotSet),
        Some(ms) if ms > limit => Err(TimeoutRefusal::TooLong),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts_are_normalized_to_milliseconds() {
        assert_eq!(timeout_ms("0"), Some(0));
        assert_eq!(timeout_ms("10s"), Some(10_000));
        assert_eq!(timeout_ms("250ms"), Some(250));
        assert_eq!(timeout_ms("2min"), Some(120_000));
        assert_eq!(timeout_ms("1h"), Some(3_600_000));
        assert_eq!(timeout_ms("soon"), None);
    }

    #[test]
    fn zero_missing_and_long_timeouts_are_refused_at_startup() {
        let policy = TimeoutPolicy::AtMostMs(10_000);
        assert_eq!(check_timeout(policy, "0"), Err(TimeoutRefusal::NotSet));
        assert_eq!(check_timeout(policy, "??"), Err(TimeoutRefusal::NotSet));
        assert_eq!(check_timeout(policy, "11s"), Err(TimeoutRefusal::TooLong));
        assert_eq!(check_timeout(policy, "1min"), Err(TimeoutRefusal::TooLong));
        assert_eq!(check_timeout(policy, "10s"), Ok(()));
        assert_eq!(check_timeout(policy, "5000"), Ok(()));
        assert_eq!(check_timeout(TimeoutPolicy::NotChecked, "0"), Ok(()));
    }
}
