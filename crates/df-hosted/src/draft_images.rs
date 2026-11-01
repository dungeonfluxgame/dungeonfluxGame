//! Hosted draft image disclosure: the production adapter behind the portal route.
//!
//! The adapter buffers bytes through the shared core (`df_assets::deliver_draft_image`) and then
//! performs the disclosure linearization point: one SQL statement reading the session, account,
//! asset and selected winning variant from the same PostgreSQL snapshot, checked against the
//! exact buffered basis. A revocation or tombstone committed before that snapshot denies; a
//! revocation committed after it cannot retract the already authorized response bytes. No
//! stronger atomicity is claimed.
//!
//! The adapter never sees cookies or digests. Entry authentication stays in `CustomerService`;
//! this module receives only the authenticated account, session and basis. Session validity
//! reuses `df_auth::web_session_policy::check_session` with the customer-configured
//! `SessionLifetimes` passed at construction; the stored absolute expiry is rechecked as well.
//!
//! Required privileges are deliberately narrow (root grants these, never superuser):
//! SELECT on `accounts.web_session` (session, account, generation, times, revocation,
//! absolute expiry), on `accounts.account` (status, credential generation), and on
//! `media.asset` plus `media.asset_variant` (the basis columns below). The connection is
//! dedicated to this adapter and bounded by the propagated operation deadline.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use df_assets::{
    ALLOWED_IMAGE_MIMES, AssetId, CatalogStore, Deadline, DeliveryBasis, DeliveryBudget,
    DraftDeliveryError, DraftDeliveryLimits, DraftDeliveryRequest, DraftImageCandidate,
    DraftImageVariant, ObjectStore, deliver_draft_image,
};
use df_auth::web_session_policy::{SessionCheck, StoredSession, check_session};
use df_customer::{AccountStatus, AccountStore, CustomerService, EmailSender};
use df_persistence::media_catalog::PostgresMediaCatalog;
use df_types::{AccountId, WebSessionId};

use crate::database::Connection;

/// One final snapshot read of session, account, asset and winning variant. A single statement is
/// a single snapshot; every comparison below runs against that row in Rust. Account tables
/// are schema-qualified so the connection search_path cannot resolve them by accident.
const DISCLOSURE_SNAPSHOT: &str = "SELECT s.credential_generation, s.created_at_ms, \
    s.last_seen_at_ms, s.revoked_at_ms, s.absolute_expires_at_ms, s.account_id, \
    a.status, a.credential_generation, \
    m.state, m.kind, m.provenance, m.campaign_id, \
    m.charge_scope_kind, m.charge_scope_id, m.origin_account, \
    m.audience_kind, m.audience_account, \
    m.attempt, m.winning_writes, m.mime, m.byte_length, m.sha256, \
    m.suppression_generation, \
    v.state, v.object_domain, v.object_version, v.byte_length, v.sha256 \
    FROM accounts.web_session s \
    JOIN accounts.account a ON a.account_id = s.account_id \
    JOIN media.asset m ON m.asset_id = $1 \
    JOIN media.asset_variant v ON v.asset_id = m.asset_id \
    AND v.attempt = m.attempt AND v.write_id = $2 AND v.variant = $3 \
    WHERE s.session_id = $4 AND s.account_id = $5 AND a.account_id = $5";

/// The hosted disclosure adapter: production catalog, object store and account authority behind
/// the portal's narrow delivery interface.
pub struct DraftImageAdapter<C, O> {
    catalog: C,
    objects: O,
    database: Connection,
    sessions: df_auth::web_session_policy::SessionLifetimes,
    limits: DraftDeliveryLimits,
    budget: DeliveryBudget,
    operation_timeout: Duration,
}

impl<C: CatalogStore + 'static, O: ObjectStore + 'static> DraftImageAdapter<C, O> {
    pub(crate) fn new(
        catalog: C,
        objects: O,
        database: Connection,
        sessions: df_auth::web_session_policy::SessionLifetimes,
        limits: DraftDeliveryLimits,
        budget: DeliveryBudget,
        operation_timeout: Duration,
    ) -> Self {
        Self {
            catalog,
            objects,
            database,
            sessions,
            limits,
            budget,
            operation_timeout,
        }
    }

    pub fn policy(&self) -> df_portal::draft_images::DraftImagePolicy {
        df_portal::draft_images::DraftImagePolicy {
            operation_timeout: self.operation_timeout,
        }
    }

    async fn authorize(
        &self,
        owner: AccountId,
        session: WebSessionId,
        basis: &DeliveryBasis,
        deadline: Deadline,
    ) -> Result<(), DraftDeliveryError> {
        if deadline.has_passed() {
            return Err(DraftDeliveryError::DeadlineExceeded);
        }
        let remaining = deadline
            .instant()
            .saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(DraftDeliveryError::DeadlineExceeded);
        }
        let asset = basis.asset();
        let write = basis.write();
        let variant = basis.variant().label();
        let parameters: [&(dyn tokio_postgres::types::ToSql + Sync); 5] = [
            &asset.as_bytes().as_slice(),
            &write.as_bytes().as_slice(),
            &variant,
            &session.as_bytes().as_slice(),
            &owner.as_bytes().as_slice(),
        ];
        let snapshot = self
            .database
            .client
            .query_opt(DISCLOSURE_SNAPSHOT, &parameters);
        let row = tokio::time::timeout(remaining, snapshot)
            .await
            .map_err(|_| DraftDeliveryError::DeadlineExceeded)?
            .map_err(|_| DraftDeliveryError::Unavailable)?;
        if deadline.has_passed() {
            return Err(DraftDeliveryError::DeadlineExceeded);
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| DraftDeliveryError::Unavailable)
            .and_then(|elapsed| {
                u64::try_from(elapsed.as_millis().min(u128::from(u64::MAX)))
                    .map_err(|_| DraftDeliveryError::Unavailable)
            })?;
        let Some(row) = row else {
            return Err(DraftDeliveryError::NotFound);
        };
        check_snapshot(&row, owner, basis, self.sessions, now_ms)
    }
}

fn as_u64(value: i64) -> Result<u64, DraftDeliveryError> {
    u64::try_from(value).map_err(|_| DraftDeliveryError::NotFound)
}

fn check_snapshot(
    row: &tokio_postgres::Row,
    owner: AccountId,
    basis: &DeliveryBasis,
    sessions: df_auth::web_session_policy::SessionLifetimes,
    now_ms: u64,
) -> Result<(), DraftDeliveryError> {
    let row_account: Vec<u8> = row.try_get(5).map_err(|_| DraftDeliveryError::NotFound)?;
    if row_account.as_slice() != owner.as_bytes() {
        return Err(DraftDeliveryError::NotFound);
    }
    let status: String = row.try_get(6).map_err(|_| DraftDeliveryError::NotFound)?;
    if AccountStatus::parse(&status) != Some(AccountStatus::Active) {
        return Err(DraftDeliveryError::NotFound);
    }
    let stored = StoredSession {
        created_at_ms: as_u64(row.try_get(1).map_err(|_| DraftDeliveryError::NotFound)?)?,
        last_seen_at_ms: as_u64(row.try_get(2).map_err(|_| DraftDeliveryError::NotFound)?)?,
        is_revoked: row
            .try_get::<_, Option<i64>>(3)
            .map_err(|_| DraftDeliveryError::NotFound)?
            .is_some(),
        credential_generation: as_u64(row.try_get(0).map_err(|_| DraftDeliveryError::NotFound)?)?,
    };
    let current_generation: u64 =
        as_u64(row.try_get(7).map_err(|_| DraftDeliveryError::NotFound)?)?;
    if !matches!(
        check_session(stored, current_generation, sessions, now_ms),
        SessionCheck::Active { .. }
    ) {
        return Err(DraftDeliveryError::NotFound);
    }
    let absolute_expires_ms: u64 =
        as_u64(row.try_get(4).map_err(|_| DraftDeliveryError::NotFound)?)?;
    if absolute_expires_ms <= now_ms {
        return Err(DraftDeliveryError::NotFound);
    }
    let text = |index: usize| -> Result<String, DraftDeliveryError> {
        row.try_get(index).map_err(|_| DraftDeliveryError::NotFound)
    };
    if text(8)? != "ready"
        || text(9)? != "image"
        || text(10)? != "generated"
        || row
            .try_get::<_, Option<Vec<u8>>>(11)
            .map_err(|_| DraftDeliveryError::NotFound)?
            .is_some()
    {
        return Err(DraftDeliveryError::NotFound);
    }
    if text(12)? != "account" {
        return Err(DraftDeliveryError::NotFound);
    }
    let scope: Vec<u8> = row.try_get(13).map_err(|_| DraftDeliveryError::NotFound)?;
    if scope.as_slice() != owner.as_bytes() {
        return Err(DraftDeliveryError::NotFound);
    }
    let origin: Option<Vec<u8>> = row.try_get(14).map_err(|_| DraftDeliveryError::NotFound)?;
    if origin.as_deref() != Some(owner.as_bytes().as_slice()) {
        return Err(DraftDeliveryError::NotFound);
    }
    if text(15)? != "owner_private" {
        return Err(DraftDeliveryError::NotFound);
    }
    let audience: Option<Vec<u8>> = row.try_get(16).map_err(|_| DraftDeliveryError::NotFound)?;
    if audience.as_deref() != Some(owner.as_bytes().as_slice()) {
        return Err(DraftDeliveryError::NotFound);
    }
    let attempt: Vec<u8> = row.try_get(17).map_err(|_| DraftDeliveryError::NotFound)?;
    if attempt.as_slice() != basis.attempt().as_bytes() {
        return Err(DraftDeliveryError::NotFound);
    }
    let winners: Option<Vec<Vec<u8>>> =
        row.try_get(18).map_err(|_| DraftDeliveryError::NotFound)?;
    if !winners
        .unwrap_or_default()
        .iter()
        .any(|write| write.as_slice() == basis.write().as_bytes().as_slice())
    {
        return Err(DraftDeliveryError::NotFound);
    }
    let mime: Option<String> = row.try_get(19).map_err(|_| DraftDeliveryError::NotFound)?;
    if mime.as_deref() != Some(basis.mime()) {
        return Err(DraftDeliveryError::NotFound);
    }
    if mime
        .as_deref()
        .is_none_or(|mime| !ALLOWED_IMAGE_MIMES.contains(&mime))
    {
        return Err(DraftDeliveryError::NotFound);
    }
    let aggregate_len: Option<i64> = row.try_get(20).map_err(|_| DraftDeliveryError::NotFound)?;
    if aggregate_len != i64::try_from(basis.published_byte_len()).ok() {
        return Err(DraftDeliveryError::NotFound);
    }
    let aggregate_digest: Option<Vec<u8>> =
        row.try_get(21).map_err(|_| DraftDeliveryError::NotFound)?;
    if aggregate_digest.as_deref() != Some(basis.published_digest().as_slice()) {
        return Err(DraftDeliveryError::NotFound);
    }
    if as_u64(row.try_get(22).map_err(|_| DraftDeliveryError::NotFound)?)?
        != basis.suppression_generation()
    {
        return Err(DraftDeliveryError::NotFound);
    }
    if text(23)? != "written" || text(24)? != "private_media" {
        return Err(DraftDeliveryError::NotFound);
    }
    let version: Option<String> = row.try_get(25).map_err(|_| DraftDeliveryError::NotFound)?;
    if version.as_deref() != Some(basis.version()) {
        return Err(DraftDeliveryError::NotFound);
    }
    let stored_len: Option<i64> = row.try_get(26).map_err(|_| DraftDeliveryError::NotFound)?;
    if stored_len != i64::try_from(basis.byte_len()).ok() {
        return Err(DraftDeliveryError::NotFound);
    }
    let stored_digest: Option<Vec<u8>> =
        row.try_get(27).map_err(|_| DraftDeliveryError::NotFound)?;
    if stored_digest.as_deref() != Some(basis.digest().as_slice()) {
        return Err(DraftDeliveryError::NotFound);
    }
    Ok(())
}

impl<C: CatalogStore + 'static, O: ObjectStore + 'static> df_portal::draft_images::DraftImageSource
    for DraftImageAdapter<C, O>
{
    async fn buffer_candidate(
        &self,
        owner: AccountId,
        asset: AssetId,
        variant: DraftImageVariant,
        deadline: Deadline,
    ) -> Result<DraftImageCandidate, DraftDeliveryError> {
        deliver_draft_image(
            &self.catalog,
            &self.objects,
            &self.budget,
            &DraftDeliveryRequest {
                owner: &owner,
                asset,
                variant,
                limits: self.limits,
                deadline,
            },
        )
        .await
    }

    async fn authorize_release(
        &self,
        owner: AccountId,
        session: WebSessionId,
        basis: DeliveryBasis,
        deadline: Deadline,
    ) -> Result<(), DraftDeliveryError> {
        self.authorize(owner, session, &basis, deadline).await
    }
}

/// Validate read authority at startup without reading an account's private data.
pub(crate) async fn check_disclosure_reader(
    database: &Connection,
) -> Result<(), DraftDeliveryError> {
    let row = database.client.query_one(
        "SELECT r.rolsuper OR r.rolcreaterole OR r.rolcreatedb OR r.rolreplication OR r.rolbypassrls \
         OR has_any_column_privilege(current_user, 'media.asset', 'INSERT,UPDATE') \
         OR has_any_column_privilege(current_user, 'media.asset_variant', 'INSERT,UPDATE') \
         OR has_any_column_privilege(current_user, 'accounts.account', 'INSERT,UPDATE') \
         OR has_any_column_privilege(current_user, 'accounts.web_session', 'INSERT,UPDATE') \
         OR has_table_privilege(current_user, 'media.asset', 'INSERT,DELETE,TRUNCATE') \
         OR has_table_privilege(current_user, 'media.asset_variant', 'INSERT,DELETE,TRUNCATE') \
         OR has_table_privilege(current_user, 'accounts.account', 'INSERT,DELETE,TRUNCATE') \
         OR has_table_privilege(current_user, 'accounts.web_session', 'INSERT,DELETE,TRUNCATE') \
         OR has_column_privilege(current_user, 'accounts.web_session', 'token_digest', 'SELECT') \
         OR has_column_privilege(current_user, 'accounts.web_session', 'csrf_secret', 'SELECT') \
         FROM pg_roles r WHERE r.rolname = current_user", &[],
    ).await.map_err(|_| DraftDeliveryError::Unavailable)?;
    if row
        .try_get::<_, bool>(0)
        .map_err(|_| DraftDeliveryError::Unavailable)?
    {
        return Err(DraftDeliveryError::Unavailable);
    }
    let absent: &[u8] = &[];
    database
        .client
        .query_opt(
            DISCLOSURE_SNAPSHOT,
            &[&absent, &absent, &"display", &absent, &absent],
        )
        .await
        .map_err(|_| DraftDeliveryError::Unavailable)?;
    Ok(())
}

/// The production adapter over the deployed PostgreSQL catalog and native file store.
pub(crate) fn production_adapter(
    catalog: PostgresMediaCatalog,
    objects: df_assets::NativeFileStore,
    database: Connection,
    sessions: df_auth::web_session_policy::SessionLifetimes,
    limits: DraftDeliveryLimits,
    budget: DeliveryBudget,
    operation_timeout: Duration,
) -> DraftImageAdapter<PostgresMediaCatalog, df_assets::NativeFileStore> {
    DraftImageAdapter::new(
        catalog,
        objects,
        database,
        sessions,
        limits,
        budget,
        operation_timeout,
    )
}

/// The exact router the host mounts alongside `app_api::compose`, before common middleware.
/// Integration tests exercise this same constructor.
pub fn router<
    S: AccountStore,
    M: EmailSender,
    C: CatalogStore + 'static,
    O: ObjectStore + 'static,
>(
    service: CustomerService<S, M>,
    adapter: Arc<DraftImageAdapter<C, O>>,
) -> axum::Router {
    let policy = adapter.policy();
    df_portal::draft_images::router(service, adapter, policy)
}
