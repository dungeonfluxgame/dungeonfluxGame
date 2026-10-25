//! The campaign owner's side of B's account closure: B's `ClosureParticipant`, implemented over the
//! closure functions in `closure.rs`. Every method runs inside B's transaction (a borrowed `Tx`),
//! requires B's role to carry a `transaction_timeout` (refused with `TimeoutNotSet` otherwise)
//! and locks campaigns in `campaign_id` order. B's error type has three variants, so the cause of
//! a failure is kept in an audit line before it is folded into one.

use std::sync::Arc;

use df_campaign::{
    audit::{AuditLine, failure_word},
    clock::Clock,
    config::Tunables,
    errors::{Failure, StoreError},
    records::{CancelReport, ChunkReport, LeaveChunk},
};
use df_customer::{
    BlockReason, BlockingCampaign, ClosureCheck, LeaveChunk as CustomerChunk,
    OwnedArchivedCampaign, ScheduledDeletion, StoreError as CustomerError,
    StoreResult as CustomerResult,
};
use df_types::{AccountId, CampaignId};
use tokio_postgres::Transaction;

use super::{closure, conn::Tx};
use crate::account_closure::{ClosureParticipant, ParticipantCall};

pub struct PgClosureParticipant {
    tunables: Tunables,
    clock: Arc<dyn Clock>,
    /// Test only: lock campaigns in the wrong order, to show the deadlock a test must catch.
    #[cfg(test)]
    reversed_locks: bool,
}

impl PgClosureParticipant {
    pub(super) fn new(tunables: Tunables, clock: Arc<dyn Clock>) -> Self {
        Self {
            tunables,
            clock,
            #[cfg(test)]
            reversed_locks: false,
        }
    }

    /// Test only: the same participant, taking campaign locks in descending order.
    #[cfg(test)]
    pub fn with_reversed_lock_order(mut self) -> Self {
        self.reversed_locks = true;
        self
    }

    fn tx<'a>(&self, transaction: &'a Transaction<'a>) -> Tx<'a> {
        #[cfg(test)]
        if self.reversed_locks {
            return Tx::borrowed_with_reversed_locks(transaction);
        }
        Tx::borrowed(transaction)
    }

    /// One chunk of `leave_all` with the full report: B's statement that takes the account row
    /// begins the transaction, and the chunk returns `Aborted` if the account is no longer
    /// `closing`. B's `leave_chunk` below wraps it. The late-host and skipped-hand-over cases are audited here, the latter as an alert.
    pub async fn leave_all_chunk(
        &self,
        transaction: &Transaction<'_>,
        account: AccountId,
        after: Option<CampaignId>,
    ) -> CustomerResult<ChunkReport> {
        let tx = self.tx(transaction);
        let report =
            closure::leave_all_chunk(&tx, &self.tunables, account, after, self.clock.now_ms())
                .await
                .map_err(customer_error)?;
        for campaign in &report.late_hosts {
            AuditLine::new("CAMPAIGN", "closure_late_host")
                .campaign(*campaign)
                .actor(account)
                .reason("host_of_active_campaign")
                .emit("campaign.closure_late_host");
        }
        for campaign in &report.handover_skipped {
            AuditLine::new("FEED", "handover_skipped")
                .campaign(*campaign)
                .actor(account)
                .reason("owner_not_active")
                .emit("feed.handover_skipped");
        }
        Ok(report)
    }

    /// Reopening: clears the deletion dates the closing set, in the reopening transaction.
    /// Reports the campaigns that were already `deleting` so B can tell the person.
    pub async fn cancel_scheduled_deletions_report(
        &self,
        transaction: &Transaction<'_>,
        account: AccountId,
    ) -> CustomerResult<CancelReport> {
        let tx = self.tx(transaction);
        let report =
            closure::cancel_scheduled_deletions(&tx, &self.tunables, account, self.clock.now_ms())
                .await
                .map_err(customer_error)?;
        for campaign in &report.cancelled {
            AuditLine::new("CAMPAIGN", "delete_cancelled")
                .campaign(*campaign)
                .actor(account)
                .reason("reopened")
                .emit("campaign.delete_cancelled");
        }
        for campaign in &report.already_deleting {
            AuditLine::new("CAMPAIGN", "refused")
                .campaign(*campaign)
                .actor(account)
                .reason("already_deleting")
                .emit("campaign.refused");
        }
        Ok(report)
    }
}

/// B's error has three variants; the cause is audited before it is folded into one.
fn customer_error(failure: Failure) -> CustomerError {
    AuditLine::new("CAMPAIGN", "closure_error")
        .reason(failure_word(failure))
        .emit("campaign.closure_error");
    match failure {
        Failure::Store(StoreError::Unknown) => CustomerError::Unknown,
        Failure::Store(StoreError::InvalidRecord) => CustomerError::InvalidRecord,
        _ => CustomerError::Unavailable,
    }
}

impl ClosureParticipant for PgClosureParticipant {
    fn closure_campaigns<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        account: AccountId,
    ) -> ParticipantCall<'a, ClosureCheck> {
        Box::pin(async move {
            let tx = self.tx(transaction);
            let found = closure::closure_campaigns(&tx, &self.tunables, account)
                .await
                .map_err(customer_error)?;
            if !found.blocking.is_empty() {
                AuditLine::new("CAMPAIGN", "closure_blocked")
                    .actor(account)
                    .reason("owns_or_hosts_active")
                    .emit("campaign.closure_blocked");
            }
            Ok(ClosureCheck {
                blocking: found
                    .blocking
                    .into_iter()
                    .map(|(campaign, name, owns, hosts)| BlockingCampaign {
                        campaign,
                        name,
                        reason: match (owns, hosts) {
                            (true, true) => BlockReason::OwnsAndHosts,
                            (true, false) => BlockReason::Owns,
                            _ => BlockReason::Hosts,
                        },
                    })
                    .collect(),
                owned_archived: found
                    .owned_archived
                    .into_iter()
                    .map(|(campaign, name)| OwnedArchivedCampaign { campaign, name })
                    .collect(),
            })
        })
    }

    fn schedule_archived_deletions<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        account: AccountId,
        delete_after_ms: u64,
    ) -> ParticipantCall<'a, Vec<ScheduledDeletion>> {
        Box::pin(async move {
            let tx = self.tx(transaction);
            let scheduled = closure::schedule_archived_deletions(
                &tx,
                &self.tunables,
                account,
                delete_after_ms,
                self.clock.now_ms(),
            )
            .await
            .map_err(customer_error)?;
            for item in &scheduled {
                AuditLine::new("CAMPAIGN", "delete_scheduled")
                    .campaign(item.campaign)
                    .actor(account)
                    .reason("closure")
                    .emit("campaign.delete_scheduled");
            }
            Ok(scheduled
                .into_iter()
                .map(|item| ScheduledDeletion {
                    campaign: item.campaign,
                    name: item.name,
                    members: item.members,
                })
                .collect())
        })
    }

    fn cancel_scheduled_deletions<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        account: AccountId,
    ) -> ParticipantCall<'a, ()> {
        Box::pin(async move {
            self.cancel_scheduled_deletions_report(transaction, account)
                .await
                .map(|_| ())
        })
    }

    /// Runs exactly one chunk, in B's transaction, once per transaction (B's completion loop
    /// holds the account row and has checked the account is still `closing`). `Continue` carries
    /// the last campaign left so B starts the next chunk after it. `Aborted` cannot happen once B
    /// has checked under its own lock; if it does, the chunk is refused loudly (the cause is
    /// audited) and rolls back with B's transaction instead of reporting progress.
    fn leave_chunk<'a>(
        &'a self,
        transaction: &'a Transaction<'_>,
        account: AccountId,
        after: Option<CampaignId>,
    ) -> ParticipantCall<'a, CustomerChunk> {
        Box::pin(async move {
            let report = self.leave_all_chunk(transaction, account, after).await?;
            match report.outcome {
                LeaveChunk::Done => Ok(CustomerChunk::Done),
                LeaveChunk::Continue(last) => Ok(CustomerChunk::Continue(last)),
                LeaveChunk::Aborted => {
                    AuditLine::new("CAMPAIGN", "closure_error")
                        .actor(account)
                        .reason("account_reopened")
                        .emit("campaign.closure_error");
                    Err(CustomerError::Unavailable)
                }
            }
        })
    }
}
