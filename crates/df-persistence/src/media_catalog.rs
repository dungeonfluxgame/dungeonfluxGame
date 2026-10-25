//! PostgreSQL adapter for `df_assets::CatalogStore` (schema `media`, migrations 0010 to 0014).
//! PostgreSQL is the authority for whether an asset exists. Every trait method is one transaction;
//! queries are parameterized; a failure before commit is `Unavailable` and a failed or lost commit
//! is `Unknown`. The statements live in `media_catalog_sql`, taking the caller's transaction, so
//! F's paid-admission transaction can call [`admit_generation`] after its own locks.

use std::sync::Arc;

use df_assets::{
    Admission, AdmitOutcome, AssetId, AssetRecord, AttemptId, CatalogError, CatalogLookup,
    CatalogStore, ClaimOutcome, DedupeScope, DeletionMark, DueRow, FinalizeRequest, FinalizeResult,
    HeldStep, LeaseOutcome, MediaRequestKey, OrphanStaged, OrphanVariant, OutboxEntry, QuotaScope,
    QuotaUsage, RejectGuard, RejectReason, RejectResult, StageOutcome, StagedObjectRecord,
    StepClaim, StorageDomain, SupersedeResult, TombstoneReason, TombstoneResult, UploadPurpose,
    UploadTicket, UploadTicketClaim, UploadTicketRejection, VariantIntent, VariantRecord,
    VariantWritten, validate_upload_ticket_binding,
};
use tokio_postgres::{Client, Config, Transaction};

use crate::media_catalog_sql as sql;
use crate::media_pool::Pool;
use crate::postgres_tls::PostgresTransport;

const MIGRATIONS: [(i32, &str); 5] = [
    (10, include_str!("../migrations/0010_media_catalog.sql")),
    (11, include_str!("../migrations/0011_character_library.sql")),
    (12, include_str!("../migrations/0012_media_exports.sql")),
    (13, include_str!("../migrations/0013_recovery_journal.sql")),
    (14, include_str!("../migrations/0014_media_sweep.sql")),
];

/// Opening the catalog failed; nothing was served.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaCatalogOpenError {
    Transport(crate::postgres_tls::TransportRefusal),
    Unreachable,
    MigrationFailed,
}

/// Reserves quota and inserts the row inside the caller's transaction. F's admission transaction
/// calls this after locking its finance rows (D25); a key conflict adds nothing and does not abort
/// the caller's transaction, so the caller rolls back its own work on `NotAdmitted`.
///
/// The transaction must run at `READ COMMITTED`: under `REPEATABLE READ` or stricter, an `ON
/// CONFLICT DO NOTHING` against a row committed after the snapshot raises a serialization
/// failure (40001), and the "never aborts the caller's transaction" guarantee no longer holds.
pub async fn admit_generation(
    tx: &Transaction<'_>,
    admission: &Admission,
    now_ms: i64,
) -> Result<AdmitOutcome, CatalogError> {
    sql::admit_in(tx, admission, now_ms).await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadTicketClaimError {
    Refused(UploadTicketRejection),
    Catalog(CatalogError),
}

/// Atomically consumes one exact upload ticket in the caller's transaction. The row's immutable
/// account, campaign, purpose, attempt, key, digest, and expiry are checked again in PostgreSQL.
pub async fn claim_upload_ticket(
    tx: &Transaction<'_>,
    ticket: &UploadTicket,
    account: df_types::AccountId,
    campaign: df_types::CampaignId,
    purpose: UploadPurpose,
    now_ms: i64,
) -> Result<UploadTicketClaim, UploadTicketClaimError> {
    validate_upload_ticket_binding(
        ticket,
        account,
        campaign,
        purpose,
        &ticket.object_key,
        now_ms,
    )
    .map_err(UploadTicketClaimError::Refused)?;
    if ticket.object_key.domain() != StorageDomain::Quarantine {
        return Err(UploadTicketClaimError::Refused(
            UploadTicketRejection::WrongObjectKey,
        ));
    }
    let byte_len = i64::try_from(ticket.declared_bytes)
        .map_err(|_| UploadTicketClaimError::Refused(UploadTicketRejection::DigestMismatch))?;
    let changed = tx
        .execute(
            "UPDATE media.upload u SET used_at_ms = $9 \
             WHERE u.upload_id = $1 AND u.asset_id = $2 AND u.account_id = $3 \
               AND u.purpose = $4 AND u.declared_bytes = $5 AND u.declared_sha256 = $6 \
               AND u.attempt = $7 AND u.object_key = $8 AND u.ticket_expires_ms = $10 \
               AND u.ticket_expires_ms > $9 \
               AND u.used_at_ms IS NULL AND EXISTS ( \
                 SELECT 1 FROM media.asset a WHERE a.asset_id = u.asset_id \
                   AND a.campaign_id = $11 AND a.attempt = $7 AND a.state = 'uploading' \
                   AND a.provenance = 'uploaded' AND a.kind = 'image')",
            &[
                &ticket.upload.as_bytes().as_slice(),
                &ticket.asset.as_bytes().as_slice(),
                &account.as_bytes().as_slice(),
                &ticket.purpose.label(),
                &byte_len,
                &ticket.expected_sha256.as_slice(),
                &ticket.attempt.as_bytes().as_slice(),
                &ticket.object_key.path(),
                &now_ms,
                &ticket.expires_at_ms,
                &campaign.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(|_| UploadTicketClaimError::Catalog(CatalogError::Unavailable))?;
    if changed == 1 {
        Ok(UploadTicketClaim::Claimed)
    } else {
        Ok(UploadTicketClaim::Refused(
            UploadTicketRejection::AlreadyUsed,
        ))
    }
}

/// The PostgreSQL media catalog. Clones share one connection pool.
#[derive(Clone)]
pub struct PostgresMediaCatalog {
    pool: Arc<Pool>,
}

impl PostgresMediaCatalog {
    pub async fn open(
        config: Config,
        transport: PostgresTransport,
        connections: usize,
    ) -> Result<Self, MediaCatalogOpenError> {
        crate::postgres_tls::check_transport(&config, &transport)
            .map_err(MediaCatalogOpenError::Transport)?;
        let pool = Pool::new(config, transport, connections);
        {
            let mut slot = pool
                .checkout()
                .await
                .map_err(|_| MediaCatalogOpenError::Unreachable)?;
            let client = slot
                .client
                .as_mut()
                .ok_or(MediaCatalogOpenError::Unreachable)?;
            migrate(client)
                .await
                .map_err(|_| MediaCatalogOpenError::MigrationFailed)?;
        }
        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// Settlement reports F consumed are marked in F's transaction; the catalog only reads them.
    async fn commit(transaction: Transaction<'_>) -> Result<(), CatalogError> {
        transaction.commit().await.map_err(|error| {
            sql::record_failure(&error);
            CatalogError::Unknown
        })
    }
}

/// Applies each migration once, in order, serialised by an advisory lock.
async fn migrate(client: &mut Client) -> Result<(), tokio_postgres::Error> {
    let transaction = client.transaction().await?;
    transaction
        .batch_execute(
            "SELECT pg_advisory_xact_lock(hashtext('df_media_migration')); \
             CREATE SCHEMA IF NOT EXISTS media; \
             CREATE TABLE IF NOT EXISTS media.schema_migration ( \
                 version integer PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())",
        )
        .await?;
    for (version, script) in MIGRATIONS {
        let applied = transaction
            .query_opt(
                "SELECT 1 FROM media.schema_migration WHERE version = $1",
                &[&version],
            )
            .await?
            .is_some();
        if !applied {
            transaction.batch_execute(script).await?;
            transaction
                .execute(
                    "INSERT INTO media.schema_migration (version) VALUES ($1)",
                    &[&version],
                )
                .await?;
        }
    }
    transaction.commit().await
}

/// Runs `$body` (an expression using `.await` and `?`) in one transaction bound to `$tx`; an early
/// return drops the transaction, which rolls it back.
macro_rules! in_transaction {
    ($self:expr, $tx:ident => $body:expr) => {{
        let mut slot = $self.pool.checkout().await?;
        let client = slot.client.as_mut().ok_or(CatalogError::Unavailable)?;
        let $tx = client
            .transaction()
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        let value = $body;
        Self::commit($tx).await?;
        Ok(value)
    }};
}

impl CatalogStore for PostgresMediaCatalog {
    async fn claim_upload_ticket(
        &self,
        ticket: &UploadTicket,
        account: df_types::AccountId,
        campaign: df_types::CampaignId,
        purpose: UploadPurpose,
        now_ms: i64,
    ) -> Result<UploadTicketClaim, CatalogError> {
        in_transaction!(self, tx => {
            match claim_upload_ticket(&tx, ticket, account, campaign, purpose, now_ms).await {
                Ok(result) => result,
                Err(UploadTicketClaimError::Refused(reason)) => UploadTicketClaim::Refused(reason),
                Err(UploadTicketClaimError::Catalog(error)) => return Err(error),
            }
        })
    }

    async fn admit(
        &self,
        admission: &Admission,
        now_ms: i64,
    ) -> Result<AdmitOutcome, CatalogError> {
        in_transaction!(self, tx => admit_generation(&tx, admission, now_ms).await?)
    }

    async fn lookup(
        &self,
        scope: DedupeScope,
        key: &MediaRequestKey,
    ) -> Result<CatalogLookup, CatalogError> {
        in_transaction!(self, tx => sql::lookup_in(&tx, scope, key).await?)
    }

    async fn asset(&self, asset: AssetId) -> Result<Option<AssetRecord>, CatalogError> {
        in_transaction!(self, tx => sql::asset_in(&tx, asset).await?)
    }

    async fn variants(&self, asset: AssetId) -> Result<Vec<VariantRecord>, CatalogError> {
        in_transaction!(self, tx => sql::variants_in(&tx, asset).await?)
    }

    async fn quota_usage(&self, scope: QuotaScope) -> Result<QuotaUsage, CatalogError> {
        in_transaction!(self, tx => sql::quota_usage_in(&tx, scope).await?)
    }

    async fn claim_step(
        &self,
        claim: StepClaim,
        now_ms: i64,
    ) -> Result<ClaimOutcome, CatalogError> {
        in_transaction!(self, tx => sql::claim_step_in(&tx, claim, now_ms).await?)
    }

    async fn renew_step(
        &self,
        held: HeldStep,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<LeaseOutcome, CatalogError> {
        in_transaction!(self, tx => sql::renew_step_in(&tx, held, lease_until_ms, now_ms).await?)
    }

    async fn record_staged(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        staged: &StagedObjectRecord,
        now_ms: i64,
    ) -> Result<StageOutcome, CatalogError> {
        in_transaction!(self, tx => sql::record_staged_in(&tx, asset, attempt, staged, now_ms).await?)
    }

    async fn mark_validated(
        &self,
        held: HeldStep,
        now_ms: i64,
    ) -> Result<LeaseOutcome, CatalogError> {
        in_transaction!(self, tx => sql::mark_validated_in(&tx, held, now_ms).await?)
    }

    async fn intend_variant(
        &self,
        held: HeldStep,
        intent: &VariantIntent,
        now_ms: i64,
    ) -> Result<LeaseOutcome, CatalogError> {
        in_transaction!(self, tx => sql::intend_variant_in(&tx, held, intent, now_ms).await?)
    }

    async fn mark_variant_written(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        written: &VariantWritten,
    ) -> Result<bool, CatalogError> {
        in_transaction!(self, tx => sql::mark_variant_written_in(&tx, asset, attempt, written).await?)
    }

    async fn finalize(
        &self,
        held: HeldStep,
        request: &FinalizeRequest,
        now_ms: i64,
    ) -> Result<FinalizeResult, CatalogError> {
        in_transaction!(self, tx => sql::finalize_in(&tx, held, request, now_ms).await?)
    }

    async fn reject(
        &self,
        asset: AssetId,
        attempt: AttemptId,
        reason: RejectReason,
        guard: RejectGuard,
        now_ms: i64,
    ) -> Result<RejectResult, CatalogError> {
        in_transaction!(self, tx => sql::reject_in(&tx, asset, attempt, reason, guard, now_ms).await?)
    }

    async fn supersede(
        &self,
        asset: AssetId,
        from: AttemptId,
        to: AttemptId,
        now_ms: i64,
    ) -> Result<SupersedeResult, CatalogError> {
        in_transaction!(self, tx => sql::supersede_in(&tx, asset, from, to, now_ms).await?)
    }

    async fn tombstone(
        &self,
        asset: AssetId,
        reason: TombstoneReason,
        now_ms: i64,
    ) -> Result<TombstoneResult, CatalogError> {
        in_transaction!(self, tx => sql::tombstone_in(&tx, asset, reason, now_ms).await?)
    }

    async fn due_rows(&self, now_ms: i64, limit: usize) -> Result<Vec<DueRow>, CatalogError> {
        in_transaction!(self, tx => sql::due_rows_in(&tx, now_ms, limit).await?)
    }

    async fn orphan_variants(
        &self,
        created_before_ms: i64,
        limit: usize,
    ) -> Result<Vec<OrphanVariant>, CatalogError> {
        in_transaction!(self, tx => sql::orphan_variants_in(&tx, created_before_ms, limit).await?)
    }

    async fn begin_variant_deletion(
        &self,
        orphan: &OrphanVariant,
    ) -> Result<DeletionMark, CatalogError> {
        in_transaction!(self, tx => sql::begin_variant_deletion_in(&tx, orphan).await?)
    }

    async fn forget_variant(&self, orphan: &OrphanVariant) -> Result<(), CatalogError> {
        in_transaction!(self, tx => sql::forget_variant_in(&tx, orphan).await?)
    }

    async fn orphan_staged(&self, limit: usize) -> Result<Vec<OrphanStaged>, CatalogError> {
        in_transaction!(self, tx => sql::orphan_staged_in(&tx, limit).await?)
    }

    async fn forget_staged(&self, orphan: &OrphanStaged, now_ms: i64) -> Result<(), CatalogError> {
        in_transaction!(self, tx => sql::forget_staged_in(&tx, orphan, now_ms).await?)
    }

    async fn pending_outbox(&self, limit: usize) -> Result<Vec<OutboxEntry>, CatalogError> {
        in_transaction!(self, tx => sql::pending_outbox_in(&tx, limit).await?)
    }

    async fn outbox_entry(&self, attempt: AttemptId) -> Result<Option<OutboxEntry>, CatalogError> {
        in_transaction!(self, tx => sql::outbox_entry_in(&tx, attempt).await?)
    }
}
