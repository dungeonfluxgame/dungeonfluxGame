//! One catalog election precedes dispatch. A loser retains only its own unsent dependency.
//! Current authority and catalog locks precede sorted producer/consumer jobs and E capacity.
use super::{PublicationSourceAuthority, rows, sources};
use df_assets::{Admission, AdmitOutcome, AssetId, AssetRecord, AssetState, AttemptId};
use df_imagery::execution::{ReleaseCommand, StageClaim, StageScope};
use df_imagery::runtime::{DispatchAdmission, RuntimeError};
use df_imagery::{CampaignArtifactRecipe, ImageOwner, owner_of_bytes, owner_scope_bytes};
use tokio_postgres::Transaction;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Candidate {
    pub(super) asset: AssetId,
    pub(super) attempt: AttemptId,
}

pub(super) enum Election {
    Producer,
    Pending,
}

fn database(error: tokio_postgres::Error) -> RuntimeError {
    RuntimeError::Publication(rows::database_error(error))
}
fn conflict() -> RuntimeError {
    RuntimeError::InvalidRecord
}

pub(super) fn claim(input: &DispatchAdmission<'_>) -> StageClaim {
    StageClaim {
        scope: StageScope {
            owner: input.job.owner.clone(),
            job: input.job.job,
            stage: input.stage.stage,
        },
        token: input.claim,
        generation: input.stage.claim_generation,
    }
}

pub(super) async fn candidate_in(
    tx: &Transaction<'_>,
    admission: &Admission,
) -> Result<Option<Candidate>, RuntimeError> {
    let row = tx.query_opt("SELECT asset_id,attempt FROM media.asset WHERE dedupe_scope=$1 AND request_key=$2 AND state IN ('uploading','quarantined','validated','ready')", &[&admission.dedupe_scope.as_bytes().as_slice(), &admission.request_key.ok_or_else(conflict)?.as_bytes().as_slice()]).await.map_err(database)?;
    row.map(|row| {
        Ok(Candidate {
            asset: AssetId::from_bytes(&row.get::<_, Vec<u8>>(0)).map_err(|_| conflict())?,
            attempt: AttemptId::from_bytes(&row.get::<_, Vec<u8>>(1)).map_err(|_| conflict())?,
        })
    })
    .transpose()
}

async fn lock_catalog_in(
    tx: &Transaction<'_>,
    input: &DispatchAdmission<'_>,
    admission: &Admission,
    authority: &dyn PublicationSourceAuthority,
    now_ms: i64,
) -> Result<Option<Candidate>, RuntimeError> {
    authority
        .lock_authority(tx, &input.job.owner, now_ms)
        .await
        .map_err(RuntimeError::Publication)?;
    crate::media_catalog_sql::lock_quota(tx, admission.charge_scope)
        .await
        .map_err(|error| {
            RuntimeError::Publication(df_imagery::publication::PublicationError::Catalog(error))
        })?;
    let candidate = candidate_in(tx, admission).await?;
    let mut assets = input
        .sources
        .iter()
        .map(|source| source.pin.asset)
        .chain(std::iter::once(admission.asset))
        .chain(candidate.map(|candidate| candidate.asset))
        .collect::<Vec<_>>();
    assets.sort_unstable_by_key(|asset| *asset.as_bytes());
    assets.dedup();
    for asset in assets {
        tx.query_opt(
            "SELECT asset_id FROM media.asset WHERE asset_id=$1 FOR UPDATE",
            &[&asset.as_bytes().as_slice()],
        )
        .await
        .map_err(database)?;
    }
    sources::lock_admission_in(
        tx,
        input,
        Some(admission.charge_scope),
        Some(authority),
        now_ms,
    )
    .await
    .map_err(RuntimeError::Publication)?;
    Ok(candidate)
}

async fn lock_jobs_in(
    tx: &Transaction<'_>,
    input: &DispatchAdmission<'_>,
    producer: Option<(ImageOwner, df_imagery::ImageJobId)>,
) -> Result<(), RuntimeError> {
    let mut jobs = vec![(input.job.owner.clone(), input.job.job)];
    jobs.extend(producer);
    jobs.sort_unstable_by_key(|(owner, job)| (owner_scope_bytes(owner), *job.as_bytes()));
    jobs.dedup();
    for (owner, job) in jobs {
        tx.query_opt(
            "SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
            &[&owner_scope_bytes(&owner), &job.as_bytes().as_slice()],
        )
        .await
        .map_err(database)?
        .ok_or_else(conflict)?;
    }
    Ok(())
}

pub(super) async fn check_claim_in(
    tx: &Transaction<'_>,
    input: &DispatchAdmission<'_>,
    recipe: &CampaignArtifactRecipe,
    now_ms: i64,
    allow_allocation: bool,
    config_digest: &[u8; 32],
) -> Result<(), RuntimeError> {
    let job = tx
        .query_opt(
            "SELECT * FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
            &[
                &owner_scope_bytes(&input.job.owner),
                &input.job.job.as_bytes().as_slice(),
            ],
        )
        .await
        .map_err(database)?
        .ok_or_else(conflict)?;
    let stage = tx.query_opt("SELECT * FROM imagery.image_execution_stage WHERE owner_scope=$1 AND job_id=$2 AND stage_id=$3 FOR UPDATE", &[&owner_scope_bytes(&input.job.owner), &input.job.job.as_bytes().as_slice(), &input.stage.stage.as_bytes().as_slice()]).await.map_err(database)?.ok_or_else(conflict)?;
    if job.get::<_, Vec<u8>>("intent_bytes") != input.job.intent.bytes()
        || job.get::<_, i64>("intent_revision") != i64::from(input.job.intent.revision())
        || job.get::<_, Vec<u8>>("config_bytes") != input.job.config.bytes()
        || job.get::<_, i64>("config_revision") != i64::from(input.job.config.revision())
        || job.get::<_, Option<Vec<u8>>>("recipe_bytes").as_deref()
            != input.job.recipe.as_ref().map(|recipe| recipe.bytes())
        || job.get::<_, Option<i64>>("recipe_revision")
            != input
                .job
                .recipe
                .as_ref()
                .map(|recipe| i64::from(recipe.revision()))
        || job.get::<_, String>("state") != "recipe_frozen"
        || job.get::<_, i64>("absolute_deadline_ms") <= now_ms
        || job
            .get::<_, Option<Vec<u8>>>("resolved_recipe_key")
            .as_deref()
            != Some(recipe.key.as_bytes().as_slice())
        || job
            .get::<_, Option<Vec<u8>>>("planned_output_stage")
            .as_deref()
            != Some(input.stage.stage.as_bytes().as_slice())
        || stage.get::<_, String>("stage_state") != "admitted"
        || !stage.get::<_, bool>("capacity_reserved")
        || stage.get::<_, String>("stage_kind") != input.stage.kind.label()
        || stage.get::<_, Vec<u8>>("config_digest").as_slice() != config_digest.as_slice()
        || stage.get::<_, Vec<u8>>("claim_token") != input.claim.as_bytes()
        || stage.get::<_, i64>("claim_generation")
            != i64::try_from(input.stage.claim_generation).map_err(|_| conflict())?
        || stage.get::<_, i64>("lease_until_ms") <= now_ms
        || stage.get::<_, bool>("local_accounting") != input.stage.local_accounting
    {
        return Err(conflict());
    }
    let valid: bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM imagery.image_interest WHERE owner_scope=$1 AND job_id=$2 AND state='attached') AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt WHERE job_id=$2) AND NOT EXISTS(SELECT 1 FROM imagery.image_job_result WHERE owner_scope=$1 AND job_id=$2) AND NOT EXISTS(SELECT 1 FROM imagery.image_no_delivery WHERE owner_scope=$1 AND job_id=$2) AND NOT EXISTS(SELECT 1 FROM imagery.image_execution_stage s WHERE s.owner_scope=$1 AND s.job_id=$2 AND imagery.stage_has_finance($1,$2,s.stage_id) IS DISTINCT FROM false) AND ($3 OR NOT EXISTS(SELECT 1 FROM imagery.image_publication_allocation WHERE owner_scope=$1 AND job_id=$2))", &[&owner_scope_bytes(&input.job.owner), &input.job.job.as_bytes().as_slice(), &allow_allocation]).await.map_err(database)?.get(0);
    if !valid {
        return Err(conflict());
    }
    Ok(())
}

async fn producer_in(
    tx: &Transaction<'_>,
    candidate: Candidate,
    admission: &Admission,
    recipe: &CampaignArtifactRecipe,
) -> Result<(ImageOwner, df_imagery::ImageJobId), RuntimeError> {
    let record = crate::media_catalog_sql::asset_in(tx, candidate.asset)
        .await
        .map_err(|error| {
            RuntimeError::Publication(df_imagery::publication::PublicationError::Catalog(error))
        })?
        .ok_or_else(conflict)?;
    check_catalog(&record, candidate, admission)?;
    let row = tx.query_opt("SELECT owner_scope,job_id,original_recipe_key,semantic_recipe_revision,semantic_recipe_bytes FROM imagery.image_publication_allocation WHERE asset_id=$1 AND publication_attempt=$2", &[&candidate.asset.as_bytes().as_slice(), &candidate.attempt.as_bytes().as_slice()]).await.map_err(database)?.ok_or(RuntimeError::UnsupportedCapability)?;
    if row.get::<_, Vec<u8>>("original_recipe_key") != recipe.key.as_bytes()
        || row.get::<_, Option<i64>>("semantic_recipe_revision")
            != Some(i64::from(recipe.semantic.revision()))
        || row
            .get::<_, Option<Vec<u8>>>("semantic_recipe_bytes")
            .as_deref()
            != Some(recipe.semantic.bytes())
    {
        return Err(conflict());
    }
    let owner = owner_of_bytes(&row.get::<_, Vec<u8>>("owner_scope")).ok_or_else(conflict)?;
    let Some(campaign) = admission.campaign else {
        return Err(conflict());
    };
    if !matches!(&owner,ImageOwner::Run{campaign:actual,..} if *actual==campaign) {
        return Err(conflict());
    }
    Ok((
        owner,
        df_imagery::ImageJobId::from_bytes(&row.get::<_, Vec<u8>>("job_id"))
            .map_err(|_| conflict())?,
    ))
}

fn check_catalog(
    record: &AssetRecord,
    candidate: Candidate,
    admission: &Admission,
) -> Result<(), RuntimeError> {
    if record.attempt != candidate.attempt
        || record.kind != admission.kind
        || record.provenance != admission.provenance
        || record.campaign != admission.campaign
        || record.audience != admission.audience
        || record.retention != admission.retention
        || record.dedupe_scope != admission.dedupe_scope
        || record.request_key != admission.request_key
        || !matches!(
            record.state,
            AssetState::Uploading
                | AssetState::Quarantined
                | AssetState::Validated
                | AssetState::Ready
        )
    {
        return Err(conflict());
    }
    Ok(())
}

pub(super) struct ElectionContext<'a, 'b> {
    pub(super) input: &'a DispatchAdmission<'b>,
    pub(super) admission: &'a Admission,
    pub(super) recipe: &'a CampaignArtifactRecipe,
    pub(super) authority: &'a dyn PublicationSourceAuthority,
    pub(super) now_ms: i64,
    pub(super) not_before_ms: i64,
    pub(super) config_digest: [u8; 32],
}
pub(super) async fn elect_in(
    tx: &Transaction<'_>,
    context: &ElectionContext<'_, '_>,
) -> Result<Election, RuntimeError> {
    let ElectionContext {
        input,
        admission,
        recipe,
        authority,
        now_ms,
        not_before_ms,
        config_digest,
    } = *context;
    let candidate = lock_catalog_in(tx, input, admission, authority, now_ms).await?;
    if let Some(candidate) = candidate {
        let producer = producer_in(tx, candidate, admission, recipe).await?;
        lock_jobs_in(tx, input, Some(producer)).await?;
        let own = candidate.asset == admission.asset && candidate.attempt == admission.attempt;
        check_claim_in(tx, input, recipe, now_ms, own, &config_digest).await?;
        if own {
            if !input.stage.local_accounting {
                return Err(RuntimeError::UnsupportedCapability);
            }
            return Ok(Election::Producer);
        }
        record_dependency_in(
            tx,
            input,
            recipe,
            &DependencyWrite {
                candidate,
                generation: input.stage.claim_generation,
                state: DependencyState::Waiting,
                now_ms,
                next_check_ms: not_before_ms,
            },
        )
        .await?;
        crate::imagery_execution::defer_claimed_queued_in(tx, &claim(input), not_before_ms, now_ms)
            .await
            .map_err(RuntimeError::Execution)?;
        return Ok(Election::Pending);
    }
    if !matches!(
        crate::media_catalog_sql::lookup_in(tx, admission.dedupe_scope, &recipe.key)
            .await
            .map_err(|error| RuntimeError::Publication(
                df_imagery::publication::PublicationError::Catalog(error)
            ))?,
        df_assets::CatalogLookup::Absent
    ) {
        return Err(RuntimeError::SourceDenied);
    }
    if !input.stage.local_accounting
        || input.config.accounting.mode != df_imagery::AccountingMode::RecordOnly
    {
        return Err(RuntimeError::UnsupportedCapability);
    }
    lock_jobs_in(tx, input, None).await?;
    check_claim_in(tx, input, recipe, now_ms, false, &config_digest).await?;
    tx.query_one(
        "SELECT imagery.release_artifact_dependency($1,$2,$3,$4,$5,$6)",
        &[
            &owner_scope_bytes(&input.job.owner),
            &input.job.job.as_bytes().as_slice(),
            &input.stage.stage.as_bytes().as_slice(),
            &input.claim.as_bytes().as_slice(),
            &i64::try_from(input.stage.claim_generation).map_err(|_| conflict())?,
            &now_ms,
        ],
    )
    .await
    .map_err(database)?;
    if crate::media_catalog::admit_generation(tx, admission, now_ms)
        .await
        .map_err(|error| {
            RuntimeError::Publication(df_imagery::publication::PublicationError::Catalog(error))
        })?
        != AdmitOutcome::Admitted
    {
        return Err(conflict());
    }
    tx.query_one(
        "SELECT imagery.allocate_local_artifact_output($1,$2,$3,$4,$5,$6,$7,$8)",
        &[
            &owner_scope_bytes(&input.job.owner),
            &input.job.job.as_bytes().as_slice(),
            &input.stage.stage.as_bytes().as_slice(),
            &admission.asset.as_bytes().as_slice(),
            &admission.attempt.as_bytes().as_slice(),
            &now_ms,
            &i64::from(recipe.semantic.revision()),
            &recipe.semantic.bytes(),
        ],
    )
    .await
    .map_err(database)?;
    Ok(Election::Producer)
}

pub(super) async fn resolve_in(
    tx: &Transaction<'_>,
    input: &DispatchAdmission<'_>,
    recipe: &CampaignArtifactRecipe,
    candidate: Candidate,
    config_digest: [u8; 32],
    now_ms: i64,
) -> Result<(), RuntimeError> {
    check_claim_in(tx, input, recipe, now_ms, false, &config_digest).await?;
    let released = crate::imagery_execution::release_stage_in(
        tx,
        &ReleaseCommand::new(
            input.job.owner.clone(),
            input.job.job,
            input.stage.stage,
            input.claim,
            input.stage.claim_generation,
            now_ms,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?,
    )
    .await
    .map_err(RuntimeError::Execution)?;
    record_dependency_in(
        tx,
        input,
        recipe,
        &DependencyWrite {
            candidate,
            generation: released.claim_generation,
            state: DependencyState::Resolved,
            now_ms,
            next_check_ms: now_ms,
        },
    )
    .await
}

enum DependencyState {
    Waiting,
    Resolved,
}
struct DependencyWrite {
    candidate: Candidate,
    generation: u64,
    state: DependencyState,
    now_ms: i64,
    next_check_ms: i64,
}
async fn record_dependency_in(
    tx: &Transaction<'_>,
    input: &DispatchAdmission<'_>,
    recipe: &CampaignArtifactRecipe,
    command: &DependencyWrite,
) -> Result<(), RuntimeError> {
    let candidate = command.candidate;
    let generation = command.generation;
    let now_ms = command.now_ms;
    let next_check_ms = command.next_check_ms;
    let state = match command.state {
        DependencyState::Waiting => "waiting",
        DependencyState::Resolved => "resolved",
    };
    tx.query_one(
        "SELECT imagery.record_artifact_dependency($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
        &[
            &owner_scope_bytes(&input.job.owner),
            &input.job.job.as_bytes().as_slice(),
            &input.stage.stage.as_bytes().as_slice(),
            &input.claim.as_bytes().as_slice(),
            &i64::try_from(generation).map_err(|_| conflict())?,
            &i64::from(recipe.semantic.revision()),
            &recipe.semantic.bytes(),
            &recipe.key.as_bytes().as_slice(),
            &candidate.asset.as_bytes().as_slice(),
            &candidate.attempt.as_bytes().as_slice(),
            &state,
            &now_ms,
            &next_check_ms,
        ],
    )
    .await
    .map_err(database)?;
    Ok(())
}
