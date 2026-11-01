//! Statements of the immutable admission binding: each function takes the
//! caller's transaction and performs one step. Every query is parameterized;
//! failures keep the SQLSTATE and never owner bytes, ids or digests.

use df_assets::{Admission, MediaRequestKey};
use df_commerce::snapshot::ReceiptRow;
use df_finance::{
    ImageStageBinding, ImageStageHookOutcome, PublicationAttemptId, service::FinanceError,
};
use df_imagery::{ImageJobId as JournalJobId, VersionedSnapshot};
use tokio_postgres::Transaction;

/// Owned unique constraints of migration 0039: a violation here is a
/// deterministic conflict, never an outage. Any other database error stays
/// unavailable.
const OWNED_UNIQUE_CONSTRAINTS: [&str; 6] = [
    "image_publication_allocation_pkey",
    "image_publication_allocation_asset_unique",
    "image_publication_allocation_job_stage",
    "image_publication_allocation_full_tuple",
    "image_finance_admission_binding_pkey",
    "image_finance_admission_binding_scoped_operation",
];

pub(super) fn record_conflict(reason: &str) {
    df_observe::record_audit_line(
        "IMAGERY",
        format!("IMAGERY admission_conflict reason={reason}"),
    );
}

pub(super) fn record_unavailable(sqlstate: Option<&str>) {
    df_observe::record_audit_line(
        "IMAGERY",
        format!(
            "IMAGERY admission_unavailable sqlstate={}",
            sqlstate.unwrap_or("none")
        ),
    );
}

pub(super) fn conflict_of(reason: &str) -> FinanceError {
    record_conflict(reason);
    FinanceError::ImageStageConflict
}

fn unavailable_of(error: &tokio_postgres::Error) -> FinanceError {
    let (sqlstate, detail) = match error.as_db_error() {
        Some(db) => (
            Some(db.code().code().to_owned()),
            "imagery admission unavailable".to_owned(),
        ),
        None => (None, "imagery admission unavailable".to_owned()),
    };
    record_unavailable(sqlstate.as_deref());
    FinanceError::ImageStageUnavailable { sqlstate, detail }
}

/// A unique violation of an owned binding constraint is a deterministic
/// conflict; a foreign-key violation is a deterministic data mismatch
/// (cross-owner job, wrong tuple, missing receipt); anything else keeps its
/// SQLSTATE as unavailable. No payload ever enters the detail.
fn statement_failure(error: tokio_postgres::Error) -> FinanceError {
    let db = error.as_db_error();
    let code = db.map(|db| db.code().code()).unwrap_or("");
    if code == "23505"
        && db
            .and_then(|db| db.constraint())
            .is_some_and(|name| OWNED_UNIQUE_CONSTRAINTS.contains(&name))
    {
        record_conflict("owned_unique_violation");
        return FinanceError::ImageStageConflict;
    }
    if code == "23503" {
        record_conflict("reference_mismatch");
        return FinanceError::ImageStageConflict;
    }
    unavailable_of(&error)
}

/// The transaction must run at `READ COMMITTED`: row locks carry the
/// guarantees and no caller may inherit a stricter default.
pub(super) async fn require_read_committed(tx: &Transaction<'_>) -> Result<(), FinanceError> {
    let setting: String = tx
        .query_one("SELECT current_setting('transaction_isolation', true)", &[])
        .await
        .map_err(|error| unavailable_of(&error))?
        .get(0);
    if setting == "read committed" {
        return Ok(());
    }
    df_observe::record_audit_line(
        "IMAGERY",
        format!("IMAGERY admission_wrong_isolation setting={setting}"),
    );
    Err(FinanceError::ImageStageUnavailable {
        sqlstate: None,
        detail: "imagery admission requires read committed".to_owned(),
    })
}

/// Exact stored finance mapping, read under the outer finance locks before
/// any media or job lock of this module: the receipt attempt and reservation,
/// the provider attempt's effect/tenant/campaign/media asset, the session
/// through its reservation chain, and the raw scoped receipt fingerprint.
/// `effect_id` is not unique and `operation_id` is not a provider-attempt
/// column, so neither is looked up there; the session comes from the
/// reservation row, never assumed onto the attempt.
pub(super) async fn check_finance_mapping(
    tx: &Transaction<'_>,
    binding: &ImageStageBinding,
    receipt: &ReceiptRow,
) -> Result<(), FinanceError> {
    if receipt.attempt.as_bytes() != binding.attempt.as_bytes() {
        return Err(conflict_of("receipt_attempt_mismatch"));
    }
    let row = tx
        .query_opt(
            "SELECT reservation_id, effect_id, campaign_id, tenant_id, media_asset_id \
             FROM finance.provider_attempt WHERE attempt_id = $1",
            &[&binding.attempt.as_bytes().as_slice()],
        )
        .await
        .map_err(statement_failure)?;
    let Some(row) = row else {
        return Err(conflict_of("provider_attempt_missing"));
    };
    let reservation: Vec<u8> = row.get("reservation_id");
    let effect: Vec<u8> = row.get("effect_id");
    let campaign: Option<Vec<u8>> = row.get("campaign_id");
    let tenant: Option<Vec<u8>> = row.get("tenant_id");
    let media_asset: Option<Vec<u8>> = row.get("media_asset_id");
    if reservation.as_slice() != receipt.reservation.as_bytes().as_slice()
        || effect.as_slice() != binding.effect.as_bytes().as_slice()
        || campaign.as_deref() != Some(binding.campaign.as_bytes().as_slice())
        || tenant.as_deref() != Some(binding.tenant.as_bytes().as_slice())
        || media_asset.as_deref() != Some(binding.asset.as_bytes().as_slice())
    {
        return Err(conflict_of("provider_attempt_mismatch"));
    }
    let row = tx
        .query_opt(
            "SELECT session_id FROM finance.reservation WHERE reservation_id = $1",
            &[&receipt.reservation.as_bytes().as_slice()],
        )
        .await
        .map_err(statement_failure)?;
    let session: Option<Vec<u8>> = row.map(|row| row.get("session_id"));
    if session.as_deref() != Some(binding.session.as_bytes().as_slice()) {
        return Err(conflict_of("reservation_session_mismatch"));
    }
    let row = tx
        .query_opt(
            "SELECT fingerprint FROM finance.operation_receipt \
             WHERE namespace = 'admission' AND principal = $1 AND operation_id = $2",
            &[
                &binding.campaign.as_bytes().as_slice(),
                &binding.operation.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(statement_failure)?;
    let Some(row) = row else {
        return Err(conflict_of("operation_receipt_missing"));
    };
    let fingerprint: Vec<u8> = row.get("fingerprint");
    if fingerprint.as_slice() != receipt.fingerprint.as_slice() {
        return Err(conflict_of("operation_receipt_mismatch"));
    }
    Ok(())
}

/// Runs the existing catalog admission in the caller's transaction (finance
/// rows already locked, D25). A key conflict adds nothing and aborts nothing;
/// the caller rolls its own work back on refusal.
pub(super) async fn admit_catalog(
    tx: &Transaction<'_>,
    admission: &Admission,
    now_ms: i64,
) -> Result<(), FinanceError> {
    match crate::media_catalog::admit_generation(tx, admission, now_ms).await {
        Ok(df_assets::AdmitOutcome::Admitted) => Ok(()),
        Ok(df_assets::AdmitOutcome::NotAdmitted(refusal)) => {
            df_observe::record_audit_line("IMAGERY", "IMAGERY admission_refused".to_owned());
            Err(FinanceError::ImageStageRefusal(super::map_not_admitted(
                &refusal,
            )))
        }
        Err(error) => Err(super::map_catalog_error(&error)),
    }
}

/// The just-admitted asset row, locked to keep the current-attempt
/// comparison stable, must still name the original identity, quota scope,
/// origin, audience and request key. There is no FK to the mutable attempt;
/// this comparison is the first-allocation guard.
pub(crate) async fn check_catalog_identity(
    tx: &Transaction<'_>,
    admission: &Admission,
) -> Result<(), FinanceError> {
    let row = tx
        .query_opt(
            "SELECT attempt, charge_scope_kind, charge_scope_id, origin_account, campaign_id, \
             audience_kind, audience_campaign, audience_account, audience_members, request_key, \
             dedupe_scope \
             FROM media.asset WHERE asset_id = $1 FOR UPDATE",
            &[&admission.asset.as_bytes().as_slice()],
        )
        .await
        .map_err(statement_failure)?;
    let Some(row) = row else {
        return Err(conflict_of("catalog_asset_missing"));
    };
    let attempt: Vec<u8> = row.get("attempt");
    let scope_kind: String = row.get("charge_scope_kind");
    let scope_id: Vec<u8> = row.get("charge_scope_id");
    let origin: Option<Vec<u8>> = row.get("origin_account");
    let campaign: Option<Vec<u8>> = row.get("campaign_id");
    let dedupe: Vec<u8> = row.get("dedupe_scope");
    let audience_kind: String = row.get("audience_kind");
    let audience_campaign: Option<Vec<u8>> = row.get("audience_campaign");
    let audience_account: Option<Vec<u8>> = row.get("audience_account");
    let audience_members: Option<Vec<Vec<u8>>> = row.get("audience_members");
    let request_key: Option<Vec<u8>> = row.get("request_key");
    let expected_campaign = admission
        .campaign
        .as_ref()
        .map(|id| id.as_bytes().as_slice());
    let expected_origin = admission
        .origin_account
        .as_ref()
        .map(|id| id.as_bytes().as_slice());
    let expected_key = admission
        .request_key
        .as_ref()
        .map(|key| key.as_bytes().as_slice());
    let audience_ok = audience_kind == admission.audience.label()
        && match &admission.audience {
            df_assets::Audience::PublicStatic | df_assets::Audience::Withheld => {
                audience_campaign.is_none() && audience_account.is_none()
            }
            df_assets::Audience::CampaignShared(id) | df_assets::Audience::Host(id) => {
                audience_campaign.as_deref() == Some(id.as_bytes().as_slice())
                    && audience_account.is_none()
            }
            df_assets::Audience::Members(id, members) => {
                let expected: Vec<Vec<u8>> = members
                    .members()
                    .iter()
                    .map(|member| member.as_bytes().to_vec())
                    .collect();
                audience_campaign.as_deref() == Some(id.as_bytes().as_slice())
                    && audience_members.as_ref() == Some(&expected)
            }
            df_assets::Audience::OwnerPrivate(account) => {
                audience_account.as_deref() == Some(account.as_bytes().as_slice())
            }
        };
    if attempt.as_slice() != admission.attempt.as_bytes().as_slice()
        || scope_kind != "campaign"
        || scope_id.as_slice() != expected_campaign.unwrap_or(&[])
        || origin.as_deref() != expected_origin
        || campaign.as_deref() != expected_campaign
        || !audience_ok
        || request_key.as_deref() != expected_key
        || dedupe.as_slice() != admission.dedupe_scope.as_bytes().as_slice()
    {
        return Err(conflict_of("catalog_identity_mismatch"));
    }
    Ok(())
}

fn other_refusal(reason: &str) -> FinanceError {
    df_observe::record_audit_line(
        "IMAGERY",
        format!("IMAGERY admission_refusal reason={reason}"),
    );
    FinanceError::ImageStageRefusal(df_finance::StageRefusal::Other {
        reason: reason.to_owned(),
    })
}

/// The imagery job, locked after the quota/asset rows, must be frozen at the
/// expected revision with the identical recipe/config snapshots, resolved
/// key and deadline, and keep at least one attached interest. A retired,
/// expired or interest-less job refuses; the outer finance transaction rolls
/// everything back. A changed revision refuses as stale: the caller rereads
/// the job, rebuilds the port and retries. Scope misses never leak the other
/// owner id.
pub(crate) struct FrozenJobCheck<'a> {
    pub owner_scope: &'a [u8],
    pub job: &'a JournalJobId,
    pub recipe_key: &'a MediaRequestKey,
    pub recipe: &'a VersionedSnapshot,
    pub config: &'a VersionedSnapshot,
    pub expected_revision: u64,
    pub absolute_deadline_ms: i64,
    pub now_ms: i64,
}
pub(crate) async fn check_frozen_job(
    tx: &Transaction<'_>,
    check: FrozenJobCheck<'_>,
) -> Result<(), FinanceError> {
    let FrozenJobCheck {
        owner_scope,
        job,
        recipe_key,
        recipe,
        config,
        expected_revision,
        absolute_deadline_ms,
        now_ms,
    } = check;
    let row = tx
        .query_opt(
            "SELECT owner_scope, state, config_revision, config_bytes, resolved_recipe_key, \
             recipe_revision, recipe_bytes, revision, absolute_deadline_ms \
             FROM imagery.image_job WHERE job_id = $1 AND owner_scope = $2 FOR UPDATE",
            &[&job.as_bytes().as_slice(), &owner_scope],
        )
        .await
        .map_err(statement_failure)?;
    let Some(row) = row else {
        return Err(conflict_of("job_missing_or_foreign_owner"));
    };
    let stored_scope: Vec<u8> = row.get("owner_scope");
    if stored_scope.as_slice() != owner_scope {
        return Err(conflict_of("job_owner_mismatch"));
    }
    let state: String = row.get("state");
    if state == "retired" {
        return Err(other_refusal("job_retired"));
    }
    if state != "recipe_frozen" {
        return Err(other_refusal("job_not_frozen"));
    }
    let deadline: i64 = row.get("absolute_deadline_ms");
    if deadline != absolute_deadline_ms {
        return Err(conflict_of("job_deadline_mismatch"));
    }
    if now_ms >= deadline {
        return Err(other_refusal("job_expired"));
    }
    let revision = u64::try_from(row.get::<_, i64>("revision"))
        .map_err(|_| conflict_of("job_revision_decode"))?;
    if revision != expected_revision {
        return Err(other_refusal("job_stale"));
    }
    let key: Option<Vec<u8>> = row.get("resolved_recipe_key");
    let recipe_revision: Option<i64> = row.get("recipe_revision");
    let recipe_bytes: Option<Vec<u8>> = row.get("recipe_bytes");
    let config_revision: i64 = row.get("config_revision");
    let config_bytes: Vec<u8> = row.get("config_bytes");
    let same_recipe = key.as_deref() == Some(recipe_key.as_bytes().as_slice())
        && recipe_revision == Some(i64::from(recipe.revision()))
        && recipe_bytes.as_deref() == Some(recipe.bytes())
        && config_revision == i64::from(config.revision())
        && config_bytes.as_slice() == config.bytes();
    if !same_recipe {
        return Err(conflict_of("job_recipe_mismatch"));
    }
    let attached: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM imagery.image_interest \
             WHERE job_id = $1 AND state = 'attached'",
            &[&job.as_bytes().as_slice()],
        )
        .await
        .map_err(statement_failure)?
        .get(0);
    if attached < 1 {
        return Err(other_refusal("job_no_interests"));
    }
    Ok(())
}

/// A preexisting allocation under this publication attempt is a typed
/// conflict (retry-not-enabled), checked before the catalog admission so a
/// new receipt reusing an old attempt conflicts instead of failing in the
/// catalog. The insert below re-checks for the concurrent race.
pub(super) async fn check_no_prior_allocation(
    tx: &Transaction<'_>,
    publication_attempt: &PublicationAttemptId,
) -> Result<(), FinanceError> {
    let existing = tx
        .query_opt(
            "SELECT 1 FROM imagery.image_publication_allocation WHERE publication_attempt = $1",
            &[&publication_attempt.as_bytes().as_slice()],
        )
        .await
        .map_err(statement_failure)?;
    if existing.is_some() {
        return Err(conflict_of("allocation_already_exists"));
    }
    Ok(())
}

/// Inserts the immutable allocation. A preexisting allocation under this
/// publication attempt is a typed conflict (retry-not-enabled), never a
/// silent re-admission or reassignment: the first decision stands.
pub(crate) struct PublicationAllocationInsert<'a> {
    pub publication_attempt: PublicationAttemptId,
    pub admission: &'a Admission,
    pub owner_scope: &'a [u8],
    pub job: &'a JournalJobId,
    pub stage: &'a [u8; 16],
    pub recipe_key: &'a MediaRequestKey,
    pub recipe: &'a VersionedSnapshot,
    pub config: &'a VersionedSnapshot,
    pub now_ms: i64,
}
pub(crate) async fn insert_allocation(
    tx: &Transaction<'_>,
    insert: PublicationAllocationInsert<'_>,
) -> Result<(), FinanceError> {
    let PublicationAllocationInsert {
        publication_attempt,
        admission,
        owner_scope,
        job,
        stage,
        recipe_key,
        recipe,
        config,
        now_ms,
    } = insert;
    let existing = tx
        .query_opt(
            "SELECT job_id, stage_id, asset_id, owner_scope \
             FROM imagery.image_publication_allocation WHERE publication_attempt = $1",
            &[&publication_attempt.as_bytes().as_slice()],
        )
        .await
        .map_err(statement_failure)?;
    if existing.is_some() {
        return Err(conflict_of("allocation_already_exists"));
    }
    tx.execute(
        "INSERT INTO imagery.image_publication_allocation (publication_attempt, asset_id, job_id, \
         stage_id, owner_scope, original_recipe_key, original_recipe_revision, \
         original_recipe_bytes, original_config_revision, original_config_bytes, created_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        &[
            &publication_attempt.as_bytes().as_slice(),
            &admission.asset.as_bytes().as_slice(),
            &job.as_bytes().as_slice(),
            &stage.as_slice(),
            &owner_scope,
            &recipe_key.as_bytes().as_slice(),
            &i64::from(recipe.revision()),
            &recipe.bytes(),
            &i64::from(config.revision()),
            &config.bytes(),
            &now_ms,
        ],
    )
    .await
    .map_err(statement_failure)?;
    Ok(())
}

/// Inserts the exact finance receipt replay of the first admission.
pub(super) async fn insert_binding(
    tx: &Transaction<'_>,
    binding: &ImageStageBinding,
    receipt: &ReceiptRow,
    owner_scope: &[u8],
    now_ms: i64,
) -> Result<(), FinanceError> {
    tx.execute(
        "INSERT INTO imagery.image_finance_admission_binding (finance_attempt, publication_attempt, \
         job_id, stage_id, asset_id, owner_scope, tenant_id, campaign_id, session_id, \
         operation_id, effect_id, receipt_namespace, receipt_fingerprint, reservation_id, \
         created_at_ms) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'admission', $12, $13, $14)",
        &[
            &binding.attempt.as_bytes().as_slice(),
            &binding.publication_attempt.as_bytes().as_slice(),
            &binding.job.as_bytes().as_slice(),
            &binding.stage.as_bytes().as_slice(),
            &binding.asset.as_bytes().as_slice(),
            &owner_scope,
            &binding.tenant.as_bytes().as_slice(),
            &binding.campaign.as_bytes().as_slice(),
            &binding.session.as_bytes().as_slice(),
            &binding.operation.as_bytes().as_slice(),
            &binding.effect.as_bytes().as_slice(),
            &receipt.fingerprint.as_slice(),
            &receipt.reservation.as_bytes().as_slice(),
            &now_ms,
        ],
    )
    .await
    .map_err(statement_failure)?;
    Ok(())
}

/// Read-only replay: the exact scoped binding plus the immutable allocation
/// must name the complete mapping, receipt and frozen snapshots. The job may
/// be retired or expired and `asset.attempt` may have moved on; only the
/// allocation's history and the asset row's existence are required. No quota
/// reservation, no catalog re-admission, no new ids, no repair, no dispatch,
/// no fresh deadline. A missing linkage is `MissingLink`; any mismatch is a
/// conflict.
pub(super) async fn verify_stored_linkage(
    tx: &Transaction<'_>,
    binding: &ImageStageBinding,
    receipt: &ReceiptRow,
    recipe_key: &MediaRequestKey,
    recipe: &VersionedSnapshot,
    config: &VersionedSnapshot,
) -> Result<ImageStageHookOutcome, FinanceError> {
    let row = tx
        .query_opt(
            "SELECT b.publication_attempt AS publication_attempt, b.job_id AS job_id, \
             b.stage_id AS stage_id, b.asset_id AS asset_id, b.owner_scope AS owner_scope, \
             b.tenant_id AS tenant_id, b.campaign_id AS campaign_id, b.session_id AS session_id, \
             b.operation_id AS operation_id, b.effect_id AS effect_id, \
             b.receipt_namespace AS receipt_namespace, b.receipt_fingerprint AS receipt_fingerprint, \
             b.reservation_id AS reservation_id, a.original_recipe_key AS original_recipe_key, \
             a.original_recipe_revision AS original_recipe_revision, \
             a.original_recipe_bytes AS original_recipe_bytes, \
             a.original_config_revision AS original_config_revision, \
             a.original_config_bytes AS original_config_bytes \
             FROM imagery.image_finance_admission_binding b \
             JOIN imagery.image_publication_allocation a
               ON a.publication_attempt = b.publication_attempt
              AND a.job_id = b.job_id AND a.stage_id = b.stage_id
              AND a.asset_id = b.asset_id AND a.owner_scope = b.owner_scope \
             WHERE b.finance_attempt = $1",
            &[&binding.attempt.as_bytes().as_slice()],
        )
        .await
        .map_err(statement_failure)?;
    let Some(row) = row else {
        return Err(FinanceError::ImageStageMissingLink);
    };
    let stored: Vec<u8> = row.get("publication_attempt");
    let job: Vec<u8> = row.get("job_id");
    let stage: Vec<u8> = row.get("stage_id");
    let asset: Vec<u8> = row.get("asset_id");
    let owner_scope: Vec<u8> = row.get("owner_scope");
    let tenant: Vec<u8> = row.get("tenant_id");
    let campaign: Vec<u8> = row.get("campaign_id");
    let session: Vec<u8> = row.get("session_id");
    let operation: Vec<u8> = row.get("operation_id");
    let effect: Vec<u8> = row.get("effect_id");
    let namespace: String = row.get("receipt_namespace");
    let fingerprint: Vec<u8> = row.get("receipt_fingerprint");
    let reservation: Vec<u8> = row.get("reservation_id");
    let stored_key: Vec<u8> = row.get("original_recipe_key");
    let stored_recipe_revision: i64 = row.get("original_recipe_revision");
    let stored_recipe: Vec<u8> = row.get("original_recipe_bytes");
    let stored_config_revision: i64 = row.get("original_config_revision");
    let stored_config: Vec<u8> = row.get("original_config_bytes");
    if stored.as_slice() != binding.publication_attempt.as_bytes().as_slice()
        || job.as_slice() != binding.job.as_bytes().as_slice()
        || stage.as_slice() != binding.stage.as_bytes().as_slice()
        || asset.as_slice() != binding.asset.as_bytes().as_slice()
        || owner_scope.as_slice() != binding.owner_scope.as_bytes()
        || tenant.as_slice() != binding.tenant.as_bytes().as_slice()
        || campaign.as_slice() != binding.campaign.as_bytes().as_slice()
        || session.as_slice() != binding.session.as_bytes().as_slice()
        || operation.as_slice() != binding.operation.as_bytes().as_slice()
        || effect.as_slice() != binding.effect.as_bytes().as_slice()
        || namespace != "admission"
        || fingerprint.as_slice() != receipt.fingerprint.as_slice()
        || reservation.as_slice() != receipt.reservation.as_bytes().as_slice()
        || receipt.attempt.as_bytes() != binding.attempt.as_bytes()
        || stored_key.as_slice() != recipe_key.as_bytes().as_slice()
        || stored_recipe_revision != i64::from(recipe.revision())
        || stored_recipe.as_slice() != recipe.bytes()
        || stored_config_revision != i64::from(config.revision())
        || stored_config.as_slice() != config.bytes()
    {
        return Err(conflict_of("replay_mismatch"));
    }
    check_finance_mapping(tx, binding, receipt).await?;
    let asset_exists = tx
        .query_opt(
            "SELECT 1 FROM media.asset WHERE asset_id = $1",
            &[&binding.asset.as_bytes().as_slice()],
        )
        .await
        .map_err(statement_failure)?
        .is_some();
    if !asset_exists {
        return Err(FinanceError::ImageStageMissingLink);
    }
    Ok(ImageStageHookOutcome::Existing)
}
