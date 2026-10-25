//! The recurring sweep (planning/asset-storage-research.md 6.2): rows nobody holds a lease on are
//! resumed or ended by what F reports about their provider attempt, and objects no row needs are
//! deleted by the orphan rule. A row that may still be paid for is never freed: only a
//! supplier-side terminal, `OutputGone`, or an age limit ends it, and only a terminal transition
//! reports to F.

use crate::asset_lifecycle::{AssetState, RejectReason};
use crate::catalog_ports::{
    CancelOutcome, Dispatcher, FinanceAttempts, MediaValidator, ProviderAttemptState, RefetchAnswer,
};
use crate::catalog_record::Provenance;
use crate::catalog_service::{Advance, CatalogService, ServiceFailure, audit};
use crate::catalog_store::{
    CatalogStore, DeletionMark, DueRow, OrphanStaged, OrphanVariant, RejectGuard,
};
use crate::media_ids::AttemptId;
use crate::object_store::{ObjectKey, ObjectStore, ObjectVersion, VersionListing};
use std::collections::BTreeMap;

/// The most attempts whose re-fetch back-off is remembered; the soonest-due entry is forgotten
/// first when the bound is reached, which only makes that attempt eligible earlier.
const MAXIMUM_TRACKED_ATTEMPTS: usize = 1024;

/// When each attempt may next be asked of the dispatcher, doubling up to a cap.
pub(crate) struct RefetchBackoff {
    base_ms: i64,
    cap_ms: i64,
    next: BTreeMap<AttemptId, (i64, i64)>,
}

impl RefetchBackoff {
    pub(crate) fn new(base_ms: i64, cap_ms: i64) -> Self {
        Self {
            base_ms,
            cap_ms,
            next: BTreeMap::new(),
        }
    }

    fn is_due(&self, attempt: AttemptId, now_ms: i64) -> bool {
        self.next
            .get(&attempt)
            .is_none_or(|(due_ms, _)| *due_ms <= now_ms)
    }

    fn record_request(&mut self, attempt: AttemptId, now_ms: i64) {
        let delay_ms = self
            .next
            .get(&attempt)
            .map_or(self.base_ms, |(_, previous)| {
                previous.saturating_mul(2).min(self.cap_ms)
            });
        if self.next.len() >= MAXIMUM_TRACKED_ATTEMPTS
            && !self.next.contains_key(&attempt)
            && let Some(soonest) = self
                .next
                .iter()
                .min_by_key(|(_, (due_ms, _))| *due_ms)
                .map(|(attempt, _)| *attempt)
        {
            self.next.remove(&soonest);
        }
        self.next.insert(attempt, (now_ms + delay_ms, delay_ms));
    }

    fn forget(&mut self, attempt: AttemptId) {
        self.next.remove(&attempt);
    }
}

/// What one sweep pass did; the dev view and metrics read it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SweepReport {
    pub examined: usize,
    pub advanced: usize,
    pub rejected: usize,
    /// Rows left alone because the attempt may still be paid for or is within its window.
    pub kept_in_flight: usize,
    pub refetch_requested: usize,
    pub variants_deleted: usize,
    pub staged_deleted: usize,
    /// Work skipped because F, the dispatcher or the object store did not answer.
    pub deferred: usize,
}

impl<C, S, F, D, V> CatalogService<C, S, F, D, V>
where
    C: CatalogStore,
    S: ObjectStore,
    F: FinanceAttempts,
    D: Dispatcher,
    V: MediaValidator,
{
    /// One bounded pass: due rows first, then orphaned variants and staged objects.
    pub async fn sweep_once(&self) -> Result<SweepReport, ServiceFailure> {
        let now_ms = self.clock.now_ms();
        let batch = self.config.sweeper_batch_size;
        let mut report = SweepReport::default();
        for row in self.catalog.due_rows(now_ms, batch).await? {
            report.examined += 1;
            if let Err(failure) = self.sweep_row(&row, now_ms, &mut report).await {
                // One row's failure must not stop the pass: the others, and the orphan sweep, run.
                report.deferred += 1;
                audit(format!(
                    "ASSET sweep_row_failed asset={} attempt={} reason={failure:?}",
                    row.asset.to_hex(),
                    row.attempt.to_hex()
                ));
            }
        }
        self.sweep_orphans(now_ms, batch, &mut report).await?;
        Ok(report)
    }

    async fn sweep_row(
        &self,
        row: &DueRow,
        now_ms: i64,
        report: &mut SweepReport,
    ) -> Result<(), ServiceFailure> {
        match row.state {
            AssetState::Uploading if row.provenance == Provenance::Generated => {
                self.sweep_unstaged(row, now_ms, report).await
            }
            AssetState::Quarantined | AssetState::Validated => {
                let expired = row.state == AssetState::Quarantined
                    && row.staged.as_ref().is_some_and(|staged| {
                        now_ms >= staged.staged_at_ms + self.config.quarantine_max_age_ms
                    });
                if expired {
                    return self.end_row(row, RejectReason::StagedExpired, report).await;
                }
                match self.advance(row.asset).await? {
                    Advance::Ready => report.advanced += 1,
                    Advance::Ended(_) => report.rejected += 1,
                    _ => report.deferred += 1,
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn end_row(
        &self,
        row: &DueRow,
        reason: RejectReason,
        report: &mut SweepReport,
    ) -> Result<(), ServiceFailure> {
        // The rejection requires that no live lease holds the row at commit: a worker that claimed
        // it after `due_rows` read it keeps its work.
        let ended = self
            .end_attempt(row.asset, row.attempt, reason, RejectGuard::WhileUnleased)
            .await?;
        match ended {
            Advance::Ended(_) => report.rejected += 1,
            Advance::Busy => report.deferred += 1,
            _ => {}
        }
        Ok(())
    }

    /// A generated row with nothing staged yet: F's attempt state decides (6.2 sweeper list).
    async fn sweep_unstaged(
        &self,
        row: &DueRow,
        now_ms: i64,
        report: &mut SweepReport,
    ) -> Result<(), ServiceFailure> {
        let Ok(state) = self.finance.attempt_state(row.attempt).await else {
            report.deferred += 1;
            return Ok(());
        };
        match state {
            ProviderAttemptState::Reserved => {
                match self.finance.cancel_unclaimed_effect(row.attempt).await {
                    Ok(CancelOutcome::Cancelled) => {
                        self.end_row(row, RejectReason::NotDispatched, report).await
                    }
                    Ok(CancelOutcome::AlreadyClaimed) => {
                        report.kept_in_flight += 1;
                        Ok(())
                    }
                    Err(_) => {
                        report.deferred += 1;
                        Ok(())
                    }
                }
            }
            ProviderAttemptState::Cancelled => {
                self.end_row(row, RejectReason::NotDispatched, report).await
            }
            ProviderAttemptState::DispatchPrepared
            | ProviderAttemptState::Sent
            | ProviderAttemptState::OutcomeUnknown
            | ProviderAttemptState::CustomerReleased => {
                report.kept_in_flight += 1;
                Ok(())
            }
            ProviderAttemptState::ResultKnown {
                succeeded: false, ..
            } => {
                self.end_row(row, RejectReason::SupplierFailed, report)
                    .await
            }
            ProviderAttemptState::ResultKnown {
                succeeded: true,
                at_ms,
            } => {
                if now_ms < at_ms + self.config.staging_deadline_ms {
                    report.kept_in_flight += 1;
                    return Ok(());
                }
                self.request_refetch(row, now_ms, report).await
            }
            ProviderAttemptState::FailedAfterReconciliation => {
                self.end_row(row, RejectReason::UnknownOutcomeFailed, report)
                    .await
            }
        }
    }

    /// Only a positive `OutputGone` frees the key; every other answer leaves the row in flight and
    /// the next request waits out its back-off.
    async fn request_refetch(
        &self,
        row: &DueRow,
        now_ms: i64,
        report: &mut SweepReport,
    ) -> Result<(), ServiceFailure> {
        let is_due = self
            .refetch_backoff
            .lock()
            .map_or(true, |backoff| backoff.is_due(row.attempt, now_ms));
        if !is_due {
            report.kept_in_flight += 1;
            return Ok(());
        }
        let answer = self
            .dispatcher
            .request_refetch(row.asset, row.attempt)
            .await;
        audit(format!(
            "ASSET refetch_requested asset={} attempt={} answer={answer:?}",
            row.asset.to_hex(),
            row.attempt.to_hex()
        ));
        if answer == RefetchAnswer::OutputGone {
            if let Ok(mut backoff) = self.refetch_backoff.lock() {
                backoff.forget(row.attempt);
            }
            return self.end_row(row, RejectReason::OutputLost, report).await;
        }
        if let Ok(mut backoff) = self.refetch_backoff.lock() {
            backoff.record_request(row.attempt, now_ms);
        }
        report.refetch_requested += 1;
        Ok(())
    }

    async fn sweep_orphans(
        &self,
        now_ms: i64,
        batch: usize,
        report: &mut SweepReport,
    ) -> Result<(), ServiceFailure> {
        let created_before_ms = now_ms - self.config.cleanup_grace_ms;
        for orphan in self
            .catalog
            .orphan_variants(created_before_ms, batch)
            .await?
        {
            match self.delete_orphan_variant(&orphan).await {
                Ok(Some(true)) => report.variants_deleted += 1,
                Ok(Some(false)) => report.deferred += 1,
                Ok(None) => {}
                Err(failure) => {
                    report.deferred += 1;
                    audit(format!(
                        "ASSET orphan_failed asset={} attempt={} reason={failure:?}",
                        orphan.asset.to_hex(),
                        orphan.attempt.to_hex()
                    ));
                }
            }
        }
        for orphan in self.catalog.orphan_staged(batch).await? {
            match self.delete_orphan_staged(&orphan, now_ms).await {
                Ok(true) => report.staged_deleted += 1,
                Ok(false) => report.deferred += 1,
                Err(failure) => {
                    report.deferred += 1;
                    audit(format!(
                        "ASSET orphan_failed asset={} attempt={} reason={failure:?}",
                        orphan.asset.to_hex(),
                        orphan.attempt.to_hex()
                    ));
                }
            }
        }
        Ok(())
    }

    /// `Some(true)` deleted, `Some(false)` the store failed, `None` the rule no longer allows it.
    async fn delete_orphan_variant(
        &self,
        orphan: &OrphanVariant,
    ) -> Result<Option<bool>, ServiceFailure> {
        if self.catalog.begin_variant_deletion(orphan).await? != DeletionMark::Marked {
            return Ok(None);
        }
        audit(format!(
            "ASSET variant_deleting asset={} attempt={} variant={}",
            orphan.asset.to_hex(),
            orphan.attempt.to_hex(),
            orphan.variant.label()
        ));
        if !self
            .delete_every_version(&orphan.key, orphan.version.as_ref())
            .await
        {
            return Ok(Some(false));
        }
        self.catalog.forget_variant(orphan).await?;
        Ok(Some(true))
    }

    async fn delete_orphan_staged(
        &self,
        orphan: &OrphanStaged,
        now_ms: i64,
    ) -> Result<bool, ServiceFailure> {
        if !self
            .delete_every_version(&orphan.key, Some(&orphan.version))
            .await
        {
            return Ok(false);
        }
        self.catalog.forget_staged(orphan, now_ms).await?;
        Ok(true)
    }

    /// Deletes every version the store lists for the key; where listing is unsupported, the
    /// recorded version (a lifecycle rule removes any other). `false` when the store failed.
    async fn delete_every_version(
        &self,
        key: &ObjectKey,
        recorded: Option<&ObjectVersion>,
    ) -> bool {
        let versions = match self.objects.list_versions(key, self.deadline()).await {
            Ok(versions) => versions,
            Err(VersionListing::Unsupported) => recorded.into_iter().cloned().collect(),
            Err(VersionListing::Failed(_)) => return false,
        };
        for version in &versions {
            if self
                .objects
                .delete_version(key, version, self.deadline())
                .await
                .is_err()
            {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(n: u16) -> AttemptId {
        let mut bytes = [0_u8; 16];
        bytes[14..].copy_from_slice(&n.to_be_bytes());
        bytes[0] = 1;
        AttemptId::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn the_back_off_doubles_up_to_its_cap_and_waits_out_each_delay() {
        let mut backoff = RefetchBackoff::new(1_000, 3_000);
        assert!(backoff.is_due(attempt(1), 0));
        backoff.record_request(attempt(1), 0);
        assert!(!backoff.is_due(attempt(1), 999));
        assert!(backoff.is_due(attempt(1), 1_000));
        backoff.record_request(attempt(1), 1_000);
        assert!(!backoff.is_due(attempt(1), 2_999));
        assert!(backoff.is_due(attempt(1), 3_000));
        backoff.record_request(attempt(1), 3_000);
        assert!(
            !backoff.is_due(attempt(1), 5_999),
            "the delay stays at the 3,000 ms cap"
        );
        assert!(backoff.is_due(attempt(1), 6_000));
        backoff.forget(attempt(1));
        assert!(backoff.is_due(attempt(1), 0));
    }

    #[test]
    fn the_back_off_remembers_a_bounded_number_of_attempts_and_forgets_the_soonest_due() {
        let mut backoff = RefetchBackoff::new(1_000, 1_000);
        for n in 0..u16::try_from(MAXIMUM_TRACKED_ATTEMPTS).unwrap() {
            backoff.record_request(attempt(n + 1), i64::from(n));
        }
        backoff.record_request(attempt(60_000), 5_000);
        assert_eq!(backoff.next.len(), MAXIMUM_TRACKED_ATTEMPTS);
        assert!(
            backoff.is_due(attempt(1), 0),
            "the soonest-due entry was forgotten"
        );
        assert!(!backoff.is_due(attempt(2), 0));
    }
}
