//! An in-memory [`CatalogStore`] for tests. It applies the same rules as the PostgreSQL adapter
//! (the shared contract suite runs against both) under one mutex, so every operation is atomic.
//! It is never an authority for existence; PostgreSQL is (decision 2).

use crate::asset_lifecycle::{
    AssetState, FinalizeOutcome, RejectReason, SweepRow, SweepWrite, is_write_deletable,
};
use crate::catalog_record::{
    AssetRecord, DedupeScope, HeldStep, OutboxEntry, Provenance, PublishedFacts, QuotaUsage,
    StagedObjectRecord, Step, StepLease, TombstoneReason, VariantRecord, VariantState,
};
use crate::catalog_store::{
    Admission, AdmitOutcome, CatalogError, CatalogLookup, CatalogStore, ClaimOutcome, DeletionMark,
    DueRow, FinalizeRequest, FinalizeResult, LeaseOutcome, NotAdmitted, OrphanStaged,
    OrphanVariant, RejectGuard, RejectResult, StageOutcome, StepClaim, SupersedeResult,
    TombstoneResult, VariantIntent, VariantWritten,
};
use crate::media_ids::{AssetId, AttemptId};
use crate::media_request_key::MediaRequestKey;
use crate::object_store::{AssetVariant, StorageDomain};
use crate::upload_ticket::{
    UploadPurpose, UploadTicket, UploadTicketClaim, UploadTicketRejection,
    validate_upload_ticket_binding,
};
use crate::{QuotaRefusal, QuotaScope};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The bytes one admission holds against its scope until its attempt ends.
#[derive(Clone, Copy)]
struct Reservation {
    scope: QuotaScope,
    bytes: u64,
    counts_upload: bool,
}

struct Row {
    record: AssetRecord,
    reservation: Option<Reservation>,
}

struct SupersededStaged {
    asset: AssetId,
    attempt: AttemptId,
    staged: StagedObjectRecord,
}

#[derive(Default)]
struct State {
    rows: HashMap<AssetId, Row>,
    variants: Vec<VariantRecord>,
    usage: BTreeMap<QuotaScope, QuotaUsage>,
    outbox: Vec<OutboxEntry>,
    superseded_staged: Vec<SupersededStaged>,
    deleted_staged: BTreeSet<(AssetId, AttemptId)>,
    live_references: BTreeSet<AssetId>,
    next_sequence: i64,
    used_uploads: BTreeSet<crate::media_ids::UploadId>,
}

#[derive(Default)]
pub struct InMemoryCatalog {
    state: Mutex<State>,
}

/// The fake's data stay consistent when a test thread panics while holding the lock.
fn locked(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn orphan_of(record: &VariantRecord) -> OrphanVariant {
    OrphanVariant {
        asset: record.asset,
        attempt: record.attempt,
        write: record.write,
        variant: record.variant,
        key: record.key.clone(),
        version: record.version.clone(),
    }
}

fn is_live_state(state: AssetState) -> bool {
    state.is_live()
}

fn lease_is_free(record: &AssetRecord, now_ms: i64) -> bool {
    record.lease.is_none_or(|lease| lease.until_ms < now_ms)
}

/// Why a held step no longer applies: another attempt or a rejected row (`Superseded`), or a
/// newer claim on the same attempt, finished or not (`LeaseLost`).
fn loss_of(row: Option<&Row>, held: HeldStep) -> LeaseOutcome {
    match row {
        Some(row)
            if row.record.attempt == held.attempt && row.record.state != AssetState::Rejected =>
        {
            LeaseOutcome::LeaseLost
        }
        _ => LeaseOutcome::Superseded,
    }
}

impl State {
    fn usage_mut(&mut self, scope: QuotaScope) -> &mut QuotaUsage {
        self.usage.entry(scope).or_default()
    }

    fn record_outbox(
        &mut self,
        attempt: AttemptId,
        asset: AssetId,
        outcome: FinalizeOutcome,
        at_ms: i64,
    ) {
        if self.outbox.iter().any(|entry| entry.attempt == attempt) {
            return;
        }
        self.next_sequence += 1;
        self.outbox.push(OutboxEntry {
            sequence: self.next_sequence,
            attempt,
            asset,
            outcome,
            at_ms,
            is_consumed: false,
        });
    }

    fn release_reservation(&mut self, asset: AssetId) {
        let Some(reservation) = self
            .rows
            .get_mut(&asset)
            .and_then(|row| row.reservation.take())
        else {
            return;
        };
        let usage = self.usage_mut(reservation.scope);
        usage.reserved_bytes -= reservation.bytes;
        if reservation.counts_upload {
            usage.in_flight_uploads -= 1;
        }
    }

    fn reject_row(&mut self, asset: AssetId, reason: RejectReason, now_ms: i64) -> FinalizeOutcome {
        self.release_reservation(asset);
        let outcome = FinalizeOutcome::of_rejection(reason);
        let Some(row) = self.rows.get_mut(&asset) else {
            return outcome;
        };
        row.record.state = AssetState::Rejected;
        row.record.reject_reason = Some(reason);
        row.record.lease = None;
        row.record.updated_at_ms = now_ms;
        let attempt = row.record.attempt;
        self.record_outbox(attempt, asset, outcome, now_ms);
        outcome
    }

    fn sweep_inputs(&self, variant: &VariantRecord) -> Option<(SweepRow, SweepWrite)> {
        let row = &self.rows.get(&variant.asset)?.record;
        Some((
            SweepRow {
                state: row.state,
                current_attempt: row.attempt,
                current_owner: row.lease.map(|lease| lease.owner),
            },
            SweepWrite {
                attempt: variant.attempt,
                owner: variant.owner,
                is_winner: row.winning_writes.contains(&variant.write),
            },
        ))
    }
}

impl InMemoryCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test hook: a `live` reference holds the asset (the references table arrives in round 7).
    pub fn hold_live_reference(&self, asset: AssetId) {
        locked(&self.state).live_references.insert(asset);
    }

    /// Number of asset rows held.
    pub fn row_count(&self) -> usize {
        locked(&self.state).rows.len()
    }
}

impl CatalogStore for InMemoryCatalog {
    async fn claim_upload_ticket(
        &self,
        ticket: &UploadTicket,
        account: df_types::AccountId,
        campaign: df_types::CampaignId,
        purpose: UploadPurpose,
        now_ms: i64,
    ) -> Result<UploadTicketClaim, CatalogError> {
        if let Err(reason) = validate_upload_ticket_binding(
            ticket,
            account,
            campaign,
            purpose,
            &ticket.object_key,
            now_ms,
        ) {
            return Ok(UploadTicketClaim::Refused(reason));
        }
        if ticket.object_key.domain() != StorageDomain::Quarantine {
            return Ok(UploadTicketClaim::Refused(
                UploadTicketRejection::WrongObjectKey,
            ));
        }
        let mut state = locked(&self.state);
        if state.used_uploads.contains(&ticket.upload) {
            return Ok(UploadTicketClaim::Refused(
                UploadTicketRejection::AlreadyUsed,
            ));
        }
        let current = state.rows.get(&ticket.asset).is_some_and(|row| {
            row.record.attempt == ticket.attempt
                && row.record.state == AssetState::Uploading
                && row.record.campaign == Some(campaign)
                && row.record.provenance == Provenance::Uploaded
                && row.record.kind == crate::catalog_record::AssetKind::Image
        });
        if !current {
            return Ok(UploadTicketClaim::Refused(
                UploadTicketRejection::WrongObjectKey,
            ));
        }
        state.used_uploads.insert(ticket.upload);
        Ok(UploadTicketClaim::Claimed)
    }

    async fn admit(
        &self,
        admission: &Admission,
        now_ms: i64,
    ) -> Result<AdmitOutcome, CatalogError> {
        let mut state = locked(&self.state);
        if let Some(key) = &admission.request_key
            && let Some(existing) = state.rows.values().find(|row| {
                row.record.dedupe_scope == admission.dedupe_scope
                    && row.record.request_key.as_ref() == Some(key)
                    && row.record.state.is_live()
            })
        {
            let record = &existing.record;
            return Ok(AdmitOutcome::NotAdmitted(
                if record.state == AssetState::Ready {
                    NotAdmitted::Ready {
                        asset: record.asset,
                    }
                } else {
                    NotAdmitted::InFlight {
                        attempt: record.attempt,
                    }
                },
            ));
        }
        let usage = *state.usage_mut(admission.charge_scope);
        if let Some(limit) = admission.in_flight_upload_limit
            && usage.in_flight_uploads >= limit
        {
            return Ok(AdmitOutcome::NotAdmitted(NotAdmitted::UploadLimit));
        }
        let held = usage.committed_bytes + usage.reserved_bytes;
        let wanted = held.checked_add(admission.reserve_bytes);
        if wanted.is_none_or(|wanted| wanted > admission.byte_limit) {
            return Ok(AdmitOutcome::NotAdmitted(NotAdmitted::QuotaFull(
                QuotaRefusal {
                    scope: admission.charge_scope,
                    needed_bytes: admission.reserve_bytes,
                    available_bytes: admission.byte_limit.saturating_sub(held),
                },
            )));
        }
        let counts_upload = admission.in_flight_upload_limit.is_some();
        let usage = state.usage_mut(admission.charge_scope);
        usage.reserved_bytes += admission.reserve_bytes;
        if counts_upload {
            usage.in_flight_uploads += 1;
        }
        state.rows.insert(
            admission.asset,
            Row {
                record: AssetRecord {
                    asset: admission.asset,
                    kind: admission.kind,
                    provenance: admission.provenance,
                    charge_scope: admission.charge_scope,
                    origin_account: admission.origin_account,
                    campaign: admission.campaign,
                    audience: admission.audience.clone(),
                    dedupe_scope: admission.dedupe_scope,
                    request_key: admission.request_key,
                    retention: admission.retention,
                    state: AssetState::Uploading,
                    reject_reason: None,
                    attempt: admission.attempt,
                    lease: None,
                    staged: None,
                    winning_writes: Vec::new(),
                    published: None,
                    suppression_generation: 0,
                    ready_at_ms: None,
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                },
                reservation: Some(Reservation {
                    scope: admission.charge_scope,
                    bytes: admission.reserve_bytes,
                    counts_upload,
                }),
            },
        );
        Ok(AdmitOutcome::Admitted)
    }

    async fn lookup(
        &self,
        scope: DedupeScope,
        key: &MediaRequestKey,
    ) -> Result<CatalogLookup, CatalogError> {
        let state = locked(&self.state);
        let matching: Vec<&AssetRecord> = state
            .rows
            .values()
            .map(|row| &row.record)
            .filter(|record| {
                record.dedupe_scope == scope && record.request_key.as_ref() == Some(key)
            })
            .collect();
        if let Some(live) = matching.iter().find(|record| is_live_state(record.state)) {
            return Ok(if live.state == AssetState::Ready {
                CatalogLookup::Ready { asset: live.asset }
            } else {
                CatalogLookup::InFlight {
                    attempt: live.attempt,
                }
            });
        }
        if let Some(archived) = matching
            .iter()
            .filter(|record| record.state == AssetState::Archived)
            .max_by_key(|record| record.updated_at_ms)
        {
            return Ok(CatalogLookup::Archived {
                asset: archived.asset,
            });
        }
        let suppressed = matching.iter().any(|record| {
            matches!(
                record.state,
                AssetState::Deleted | AssetState::Purging | AssetState::Purged
            )
        });
        Ok(if suppressed {
            CatalogLookup::Suppressed
        } else {
            CatalogLookup::Absent
        })
    }

    async fn asset(&self, asset: AssetId) -> Result<Option<AssetRecord>, CatalogError> {
        Ok(locked(&self.state)
            .rows
            .get(&asset)
            .map(|row| row.record.clone()))
    }

    async fn variants(&self, asset: AssetId) -> Result<Vec<VariantRecord>, CatalogError> {
        Ok(locked(&self.state)
            .variants
            .iter()
            .filter(|variant| variant.asset == asset)
            .cloned()
            .collect())
    }

    async fn quota_usage(&self, scope: QuotaScope) -> Result<QuotaUsage, CatalogError> {
        Ok(locked(&self.state)
            .usage
            .get(&scope)
            .copied()
            .unwrap_or_default())
    }

    async fn claim_step(
        &self,
        claim: StepClaim,
        now_ms: i64,
    ) -> Result<ClaimOutcome, CatalogError> {
        let mut state = locked(&self.state);
        let Some(row) = state.rows.get_mut(&claim.asset) else {
            return Ok(ClaimOutcome::Stale);
        };
        let record = &mut row.record;
        if record.attempt != claim.attempt || record.state != claim.expected_state {
            return Ok(ClaimOutcome::Stale);
        }
        if let Some(lease) = record.lease
            && lease.until_ms >= now_ms
        {
            return Ok(ClaimOutcome::Held {
                until_ms: lease.until_ms,
            });
        }
        record.lease = Some(StepLease {
            step: claim.step,
            owner: claim.owner,
            until_ms: claim.lease_until_ms,
        });
        record.updated_at_ms = now_ms;
        Ok(ClaimOutcome::Claimed)
    }

    async fn renew_step(
        &self,
        held: HeldStep,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<LeaseOutcome, CatalogError> {
        let mut state = locked(&self.state);
        let row = state.rows.get_mut(&held.asset);
        match row {
            Some(row)
                if row.record.attempt == held.attempt
                    && row
                        .record
                        .lease
                        .is_some_and(|lease| lease.owner == held.owner)
                    && is_live_state(row.record.state) =>
            {
                if let Some(lease) = row.record.lease.as_mut() {
                    lease.until_ms = lease_until_ms;
                }
                row.record.updated_at_ms = now_ms;
                Ok(LeaseOutcome::Done)
            }
            other => Ok(loss_of(other.map(|row| &*row), held)),
        }
    }

    async fn record_staged(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        staged: &StagedObjectRecord,
        now_ms: i64,
    ) -> Result<StageOutcome, CatalogError> {
        if staged.key.domain() != StorageDomain::Quarantine {
            return Err(CatalogError::InvalidRecord);
        }
        let mut state = locked(&self.state);
        let Some(row) = state.rows.get_mut(&asset) else {
            return Ok(StageOutcome::NotCurrent);
        };
        if row.record.attempt != attempt {
            return Ok(StageOutcome::NotCurrent);
        }
        match row.record.state {
            AssetState::Uploading => {
                row.record.staged = Some(staged.clone());
                row.record.state = AssetState::Quarantined;
                row.record.updated_at_ms = now_ms;
                Ok(StageOutcome::Staged)
            }
            AssetState::Quarantined | AssetState::Validated => Ok(StageOutcome::AlreadyStaged),
            other => Ok(StageOutcome::NotAccepting { state: other }),
        }
    }

    async fn mark_validated(
        &self,
        held: HeldStep,
        now_ms: i64,
    ) -> Result<LeaseOutcome, CatalogError> {
        let mut state = locked(&self.state);
        let row = state.rows.get_mut(&held.asset);
        match row {
            Some(row)
                if row.record.attempt == held.attempt
                    && matches!(
                        row.record.state,
                        AssetState::Quarantined | AssetState::Validated
                    )
                    && row
                        .record
                        .lease
                        .is_some_and(|lease| lease.owner == held.owner) =>
            {
                row.record.state = AssetState::Validated;
                if let Some(lease) = row.record.lease.as_mut() {
                    lease.step = Step::Publish;
                }
                row.record.updated_at_ms = now_ms;
                Ok(LeaseOutcome::Done)
            }
            other => Ok(loss_of(other.map(|row| &*row), held)),
        }
    }

    async fn intend_variant(
        &self,
        held: HeldStep,
        intent: &VariantIntent,
        now_ms: i64,
    ) -> Result<LeaseOutcome, CatalogError> {
        let mut state = locked(&self.state);
        let allowed = state.rows.get(&held.asset).is_some_and(|row| {
            row.record.attempt == held.attempt
                && row.record.state == AssetState::Validated
                && row
                    .record
                    .lease
                    .is_some_and(|lease| lease.owner == held.owner && lease.until_ms > now_ms)
        });
        if !allowed {
            return Ok(loss_of(state.rows.get(&held.asset), held));
        }
        let exists = state.variants.iter().any(|variant| {
            variant.asset == held.asset
                && variant.attempt == held.attempt
                && variant.write == intent.write
                && variant.variant == intent.variant
        });
        if !exists {
            state.variants.push(VariantRecord {
                asset: held.asset,
                attempt: held.attempt,
                write: intent.write,
                variant: intent.variant,
                owner: held.owner,
                state: VariantState::Intended,
                key: intent.key.clone(),
                version: None,
                manifest: None,
                created_at_ms: now_ms,
            });
        }
        Ok(LeaseOutcome::Done)
    }

    async fn mark_variant_written(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        written: &VariantWritten,
    ) -> Result<bool, CatalogError> {
        let mut state = locked(&self.state);
        let found = state.variants.iter_mut().find(|variant| {
            variant.asset == asset
                && variant.attempt == attempt
                && variant.write == written.write
                && variant.variant == written.variant
                && variant.state == VariantState::Intended
        });
        let Some(variant) = found else {
            return Ok(false);
        };
        variant.state = VariantState::Written;
        variant.version = Some(written.version.clone());
        variant.manifest = Some(written.manifest);
        Ok(true)
    }

    async fn finalize(
        &self,
        held: HeldStep,
        request: &FinalizeRequest,
        now_ms: i64,
    ) -> Result<FinalizeResult, CatalogError> {
        let mut state = locked(&self.state);
        let Some(row) = state.rows.get(&held.asset) else {
            return Ok(FinalizeResult::Superseded);
        };
        let record = &row.record;
        let winner_writes: Vec<_> = request.winners.iter().map(|winner| winner.write).collect();
        if record.attempt == held.attempt
            && record.state == AssetState::Ready
            && record.winning_writes == winner_writes
        {
            let committed = record.published.as_ref().map_or(0, |facts| facts.byte_len);
            return Ok(FinalizeResult::Ready {
                committed_bytes: committed,
            });
        }
        let owns = record.lease.is_some_and(|lease| lease.owner == held.owner);
        if record.attempt != held.attempt || record.state == AssetState::Rejected {
            return Ok(FinalizeResult::Superseded);
        }
        if record.state != AssetState::Validated || !owns {
            return Ok(FinalizeResult::LeaseLost);
        }
        let mut total_bytes = 0_u64;
        let mut master_sha256 = None;
        for winner in &request.winners {
            let written = state.variants.iter().find(|variant| {
                variant.asset == held.asset
                    && variant.attempt == held.attempt
                    && variant.write == winner.write
                    && variant.variant == winner.variant
                    && variant.state == VariantState::Written
            });
            let Some(manifest) = written.and_then(|variant| variant.manifest) else {
                return Ok(FinalizeResult::WinnerMissing {
                    variant: winner.variant,
                });
            };
            total_bytes = total_bytes.saturating_add(manifest.byte_len);
            if winner.variant == AssetVariant::Master {
                master_sha256 = Some(manifest.sha256);
            }
        }
        let Some(sha256) = master_sha256 else {
            return Ok(FinalizeResult::WinnerMissing {
                variant: AssetVariant::Master,
            });
        };
        let scope = record.charge_scope;
        let is_generated = record.provenance == Provenance::Generated;
        let reserved = state.rows.get(&held.asset).and_then(|row| row.reservation);
        let usage = *state.usage_mut(scope);
        let reserved_bytes = reserved.map_or(0, |reservation| reservation.bytes);
        if !is_generated
            && total_bytes > reserved_bytes
            && usage.committed_bytes.saturating_add(total_bytes) > request.byte_limit
        {
            let outcome = state.reject_row(held.asset, RejectReason::QuotaExceeded, now_ms);
            return Ok(FinalizeResult::Rejected(outcome));
        }
        state.release_reservation(held.asset);
        state.usage_mut(scope).committed_bytes += total_bytes;
        let Some(row) = state.rows.get_mut(&held.asset) else {
            return Ok(FinalizeResult::Superseded);
        };
        row.record.state = AssetState::Ready;
        row.record.lease = None;
        row.record.ready_at_ms = Some(now_ms);
        row.record.updated_at_ms = now_ms;
        row.record.winning_writes = winner_writes;
        row.record.published = Some(PublishedFacts {
            mime: request.mime.clone(),
            byte_len: total_bytes,
            sha256,
            width_px: request.width_px,
            height_px: request.height_px,
        });
        state.record_outbox(held.attempt, held.asset, FinalizeOutcome::Ready, now_ms);
        Ok(FinalizeResult::Ready {
            committed_bytes: total_bytes,
        })
    }

    async fn reject(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        reason: RejectReason,
        guard: RejectGuard,
        now_ms: i64,
    ) -> Result<RejectResult, CatalogError> {
        let mut state = locked(&self.state);
        let Some(row) = state.rows.get(&asset) else {
            return Ok(RejectResult::NotCurrent);
        };
        if row.record.attempt != attempt {
            return Ok(RejectResult::NotCurrent);
        }
        if !is_live_state(row.record.state) || row.record.state == AssetState::Ready {
            return Ok(RejectResult::AlreadyEnded {
                state: row.record.state,
            });
        }
        let is_held = row
            .record
            .lease
            .is_some_and(|lease| lease.until_ms >= now_ms);
        let is_satisfied = match guard {
            RejectGuard::Current => true,
            RejectGuard::WhileUploading => row.record.state == AssetState::Uploading,
            RejectGuard::WhileUnleased => !is_held,
        };
        if !is_satisfied {
            return Ok(RejectResult::GuardFailed {
                state: row.record.state,
            });
        }
        Ok(RejectResult::Rejected(
            state.reject_row(asset, reason, now_ms),
        ))
    }

    async fn supersede(
        &self,
        asset: AssetId,
        from: AttemptId,
        to: AttemptId,
        now_ms: i64,
    ) -> Result<SupersedeResult, CatalogError> {
        let mut state = locked(&self.state);
        let Some(row) = state.rows.get_mut(&asset) else {
            return Ok(SupersedeResult::NotCurrent);
        };
        let record = &mut row.record;
        let can_move = record.attempt == from
            && matches!(
                record.state,
                AssetState::Uploading | AssetState::Quarantined | AssetState::Validated
            );
        if !can_move {
            return Ok(SupersedeResult::NotCurrent);
        }
        let staged = record.staged.take();
        record.attempt = to;
        record.state = AssetState::Uploading;
        record.lease = None;
        record.updated_at_ms = now_ms;
        if let Some(staged) = staged {
            state.superseded_staged.push(SupersededStaged {
                asset,
                attempt: from,
                staged,
            });
        }
        state.record_outbox(from, asset, FinalizeOutcome::Superseded, now_ms);
        Ok(SupersedeResult::Superseded)
    }

    async fn tombstone(
        &self,
        asset: AssetId,
        _reason: TombstoneReason,
        now_ms: i64,
    ) -> Result<TombstoneResult, CatalogError> {
        let mut state = locked(&self.state);
        let Some(row) = state.rows.get(&asset) else {
            return Ok(TombstoneResult::NotFound);
        };
        let record = &row.record;
        if !matches!(record.state, AssetState::Ready | AssetState::Archived) {
            return Ok(TombstoneResult::NotDeletable {
                state: record.state,
            });
        }
        if state.live_references.contains(&asset) {
            return Ok(TombstoneResult::LiveReferences);
        }
        let (scope, bytes) = (
            record.charge_scope,
            record.published.as_ref().map_or(0, |facts| facts.byte_len),
        );
        state.usage_mut(scope).committed_bytes -= bytes;
        if let Some(row) = state.rows.get_mut(&asset) {
            row.record.state = AssetState::Deleted;
            row.record.suppression_generation += 1;
            row.record.updated_at_ms = now_ms;
        }
        Ok(TombstoneResult::Deleted {
            released_bytes: bytes,
        })
    }

    async fn due_rows(&self, now_ms: i64, limit: usize) -> Result<Vec<DueRow>, CatalogError> {
        let state = locked(&self.state);
        let mut due: Vec<&AssetRecord> = state
            .rows
            .values()
            .map(|row| &row.record)
            .filter(|record| {
                matches!(
                    record.state,
                    AssetState::Uploading | AssetState::Quarantined | AssetState::Validated
                ) && lease_is_free(record, now_ms)
            })
            .collect();
        due.sort_by_key(|record| (record.updated_at_ms, record.asset));
        Ok(due
            .into_iter()
            .take(limit)
            .map(|record| DueRow {
                asset: record.asset,
                attempt: record.attempt,
                state: record.state,
                provenance: record.provenance,
                staged: record.staged.clone(),
                created_at_ms: record.created_at_ms,
            })
            .collect())
    }

    async fn orphan_variants(
        &self,
        created_before_ms: i64,
        limit: usize,
    ) -> Result<Vec<OrphanVariant>, CatalogError> {
        let state = locked(&self.state);
        Ok(state
            .variants
            .iter()
            .filter(|variant| variant.created_at_ms < created_before_ms)
            .filter(|variant| {
                variant.state == VariantState::Deleting
                    || state
                        .sweep_inputs(variant)
                        .is_some_and(|(row, write)| is_write_deletable(row, write))
            })
            .take(limit)
            .map(orphan_of)
            .collect())
    }

    async fn begin_variant_deletion(
        &self,
        orphan: &OrphanVariant,
    ) -> Result<DeletionMark, CatalogError> {
        let mut state = locked(&self.state);
        let position = state.variants.iter().position(|variant| {
            variant.asset == orphan.asset
                && variant.attempt == orphan.attempt
                && variant.write == orphan.write
                && variant.variant == orphan.variant
        });
        let Some(position) = position else {
            return Ok(DeletionMark::Gone);
        };
        if state.variants[position].state == VariantState::Deleting {
            return Ok(DeletionMark::Marked);
        }
        let deletable = state
            .sweep_inputs(&state.variants[position])
            .is_some_and(|(row, write)| is_write_deletable(row, write));
        if !deletable {
            return Ok(DeletionMark::NoLongerDeletable);
        }
        state.variants[position].state = VariantState::Deleting;
        Ok(DeletionMark::Marked)
    }

    async fn forget_variant(&self, orphan: &OrphanVariant) -> Result<(), CatalogError> {
        locked(&self.state).variants.retain(|variant| {
            !(variant.asset == orphan.asset
                && variant.attempt == orphan.attempt
                && variant.write == orphan.write
                && variant.variant == orphan.variant)
        });
        Ok(())
    }

    async fn orphan_staged(&self, limit: usize) -> Result<Vec<OrphanStaged>, CatalogError> {
        let state = locked(&self.state);
        let superseded = state.superseded_staged.iter().map(|old| OrphanStaged {
            asset: old.asset,
            attempt: old.attempt,
            key: old.staged.key.clone(),
            version: old.staged.version.clone(),
        });
        let ended = state
            .rows
            .values()
            .map(|row| &row.record)
            .filter(|record| record.state == AssetState::Rejected || record.ready_at_ms.is_some())
            .filter(|record| {
                !state
                    .deleted_staged
                    .contains(&(record.asset, record.attempt))
            })
            .filter_map(|record| {
                record.staged.as_ref().map(|staged| OrphanStaged {
                    asset: record.asset,
                    attempt: record.attempt,
                    key: staged.key.clone(),
                    version: staged.version.clone(),
                })
            });
        Ok(superseded.chain(ended).take(limit).collect())
    }

    async fn forget_staged(&self, orphan: &OrphanStaged, _now_ms: i64) -> Result<(), CatalogError> {
        let mut state = locked(&self.state);
        state
            .superseded_staged
            .retain(|old| !(old.asset == orphan.asset && old.attempt == orphan.attempt));
        state.deleted_staged.insert((orphan.asset, orphan.attempt));
        Ok(())
    }

    async fn pending_outbox(&self, limit: usize) -> Result<Vec<OutboxEntry>, CatalogError> {
        Ok(locked(&self.state)
            .outbox
            .iter()
            .filter(|entry| !entry.is_consumed)
            .take(limit)
            .copied()
            .collect())
    }

    async fn outbox_entry(&self, attempt: AttemptId) -> Result<Option<OutboxEntry>, CatalogError> {
        Ok(locked(&self.state)
            .outbox
            .iter()
            .find(|entry| entry.attempt == attempt)
            .copied())
    }
}
