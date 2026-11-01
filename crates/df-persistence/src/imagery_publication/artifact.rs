//! Read-only campaign artifact projection; catalog/source authority precedes semantic comparison.
//! Producer job identities stay private, and owned references never become synthetic job results.
use super::{
    PostgresImagePublication, PublicationSourceAuthority, commit_publication, inspection, reusable,
    rows, selection, sources,
};
use df_assets::{
    AssetId, AssetKind, AssetState, Audience, DedupeScope, MediaRequestKey, Provenance,
    RetentionClass,
};
use df_imagery::execution::{DispatchId, ExecutionScope, ExecutionStageId};
use df_imagery::publication::{
    FrozenOutput, PublicationError, PublishedImage, ReadyArtifact, recipe_digest,
};
use df_imagery::{
    CampaignArtifactRecipe, ImageJob, ImageJobId, ImageOwner, ImageRequest, VersionedSnapshot,
    owner_of_bytes, owner_scope_bytes,
};
use tokio_postgres::{Row, Transaction};

const CANDIDATE_COLUMNS: &str = "SELECT r.owner_scope,r.job_id,r.stage_id,r.dispatch_id,r.asset_id, \
    r.publication_attempt,p.semantic_recipe_revision,p.semantic_recipe_bytes,p.original_recipe_key \
    FROM media.asset a JOIN imagery.image_job_result r ON r.asset_id=a.asset_id AND r.publication_attempt=a.attempt \
    JOIN imagery.image_publication_allocation p ON p.asset_id=r.asset_id AND p.publication_attempt=r.publication_attempt \
    AND p.job_id=r.job_id AND p.stage_id=r.allocation_stage_id AND p.owner_scope=r.owner_scope \
    WHERE a.state='ready' AND NOT r.private_reconciliation AND p.semantic_recipe_revision IS NOT NULL \
    AND p.semantic_recipe_bytes IS NOT NULL";

enum Target<'a> {
    Request(&'a ImageRequest, &'a CampaignArtifactRecipe),
    Asset(&'a ImageOwner, &'a AssetId),
    Reference(&'a ImageOwner, &'a ImageJobId),
}
impl Target<'_> {
    fn owner(&self) -> &ImageOwner {
        match self {
            Self::Request(request, _) => &request.identity.owner,
            Self::Asset(owner, _) | Self::Reference(owner, _) => owner,
        }
    }
}

struct Candidate {
    scope: ExecutionScope,
    asset: AssetId,
    attempt: df_assets::AttemptId,
    semantic: VersionedSnapshot,
    key: MediaRequestKey,
    job: ImageJob,
    selected_recipe: Option<VersionedSnapshot>,
    reference: Option<Reference>,
}
struct Reference {
    job: ImageJob,
    stage: ExecutionStageId,
    generation: i64,
    semantic: VersionedSnapshot,
    key: MediaRequestKey,
    asset: AssetId,
    attempt: df_assets::AttemptId,
    resolved_at_ms: i64,
    resolved_revision: i64,
}
struct Prepared {
    candidate: Candidate,
    recipe: CampaignArtifactRecipe,
    reference_recipe: Option<CampaignArtifactRecipe>,
    recipe_digest: [u8; 32],
    master: df_assets::ImageIntent,
    display: Option<df_assets::ImageIntent>,
    requires_selection: bool,
}
struct Projection {
    job: ImageJob,
    result: PublishedImage,
    inspection: Option<inspection::PublishedInspection>,
    key: MediaRequestKey,
    audience: Audience,
}

impl PostgresImagePublication {
    /// Finds only exact full artwork semantics under the current request's grant and source proof.
    pub async fn find_ready_artifact_with_authority(
        &self,
        request: &ImageRequest,
        config: &VersionedSnapshot,
        now_ms: i64,
        authority: &dyn PublicationSourceAuthority,
    ) -> Result<Option<ReadyArtifact>, PublicationError> {
        let Some(recipe) = df_imagery::campaign_artifact_recipe(request, config)
            .map_err(|_| PublicationError::InvalidRecord)?
        else {
            return Ok(None);
        };
        self.read_artifact_target(Target::Request(request, &recipe), now_ms, authority)
            .await
    }

    /// Reads an actual shared asset with current owner authority, never the producer's run grant.
    pub async fn read_campaign_artifact_with_authority(
        &self,
        current_owner: &ImageOwner,
        asset: &AssetId,
        now_ms: i64,
        authority: &dyn PublicationSourceAuthority,
    ) -> Result<Option<ReadyArtifact>, PublicationError> {
        self.read_artifact_target(Target::Asset(current_owner, asset), now_ms, authority)
            .await
    }

    /// Reads only a resolved, proven-unsent reference belonging to this exact current job.
    pub async fn read_artifact_with_authority(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
        now_ms: i64,
        authority: &dyn PublicationSourceAuthority,
    ) -> Result<Option<ReadyArtifact>, PublicationError> {
        self.read_artifact_target(Target::Reference(owner, job), now_ms, authority)
            .await
    }

    async fn read_artifact_target(
        &self,
        target: Target<'_>,
        now_ms: i64,
        authority: &dyn PublicationSourceAuthority,
    ) -> Result<Option<ReadyArtifact>, PublicationError> {
        if now_ms < 0 {
            return Err(PublicationError::InvalidRecord);
        }
        if !matches!(target.owner(), ImageOwner::Run { .. }) {
            return Ok(None);
        }
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(PublicationError::Catalog)?;
        let client = slot.client.as_mut().ok_or(PublicationError::Unavailable)?;
        let tx = client.transaction().await.map_err(rows::database_error)?;
        authority
            .lock_authority(&tx, target.owner(), now_ms)
            .await?;
        let candidate = load_candidate(&tx, &target).await?;
        commit_publication(tx).await?;
        let Some(candidate) = candidate else {
            return Ok(None);
        };
        let prepared = tokio::task::spawn_blocking(move || prepare(candidate))
            .await
            .map_err(|_| PublicationError::Unavailable)??;
        let tx = client.transaction().await.map_err(rows::database_error)?;
        let projection = prove(&tx, &target, &prepared, now_ms, authority).await;
        let projection = match projection {
            Ok(projection) => {
                commit_publication(tx).await?;
                projection
            }
            Err(PublicationError::BindingConflict | PublicationError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        tokio::task::spawn_blocking(move || project(projection))
            .await
            .map_err(|_| PublicationError::Unavailable)?
            .map(Some)
    }
}

async fn load_candidate(
    tx: &Transaction<'_>,
    target: &Target<'_>,
) -> Result<Option<Candidate>, PublicationError> {
    let reference = match target {
        Target::Reference(owner, job) => {
            let Some(reference) = load_reference(tx, owner, job).await? else {
                return Ok(None);
            };
            Some(reference)
        }
        _ => None,
    };
    let row = match target {
        Target::Request(_, wanted) => {
            let ImageOwner::Run { campaign, .. } = target.owner() else {
                return Ok(None);
            };
            let scope = DedupeScope::campaign(campaign);
            tx.query_opt(
                &format!("{CANDIDATE_COLUMNS} AND a.dedupe_scope=$1 AND a.request_key=$2"),
                &[
                    &scope.as_bytes().as_slice(),
                    &wanted.key.as_bytes().as_slice(),
                ],
            )
            .await
        }
        Target::Asset(_, asset) => {
            tx.query_opt(
                &format!("{CANDIDATE_COLUMNS} AND a.asset_id=$1"),
                &[&asset.as_bytes().as_slice()],
            )
            .await
        }
        Target::Reference(_, _) => {
            let reference = reference.as_ref().ok_or(PublicationError::InvalidRecord)?;
            tx.query_opt(
                &format!("{CANDIDATE_COLUMNS} AND a.asset_id=$1 AND a.attempt=$2"),
                &[
                    &reference.asset.as_bytes().as_slice(),
                    &reference.attempt.as_bytes().as_slice(),
                ],
            )
            .await
        }
    }
    .map_err(rows::database_error)?;
    let Some(row) = row else { return Ok(None) };
    let owner = owner_of_bytes(&row.get::<_, Vec<u8>>("owner_scope"))
        .ok_or(PublicationError::InvalidRecord)?;
    if !same_campaign(&owner, target.owner()) {
        return Ok(None);
    }
    let job_id = ImageJobId::from_bytes(&row.get::<_, Vec<u8>>("job_id"))
        .map_err(|_| PublicationError::InvalidRecord)?;
    let job = crate::read_in(tx, &owner, job_id)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    let selected_recipe = selection::read_in(tx, &owner, &job_id)
        .await?
        .map(|selected| selected.recipe);
    Ok(Some(Candidate {
        scope: ExecutionScope {
            owner,
            job: job_id,
            stage: ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>("stage_id"))
                .map_err(|_| PublicationError::InvalidRecord)?,
            dispatch: DispatchId::from_bytes(&row.get::<_, Vec<u8>>("dispatch_id"))
                .map_err(|_| PublicationError::InvalidRecord)?,
        },
        asset: AssetId::from_bytes(&row.get::<_, Vec<u8>>("asset_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        attempt: df_assets::AttemptId::from_bytes(&row.get::<_, Vec<u8>>("publication_attempt"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        semantic: rows::snapshot(&row, "semantic_recipe_revision", "semantic_recipe_bytes")?,
        key: key(&row, "original_recipe_key")?,
        job,
        selected_recipe,
        reference,
    }))
}

fn same_campaign(first: &ImageOwner, second: &ImageOwner) -> bool {
    matches!((first, second), (ImageOwner::Run { campaign:first, .. }, ImageOwner::Run { campaign:second, .. }) if first==second)
}
fn key(row: &Row, column: &str) -> Result<MediaRequestKey, PublicationError> {
    let bytes: [u8; 32] = row
        .get::<_, Vec<u8>>(column)
        .try_into()
        .map_err(|_| PublicationError::InvalidRecord)?;
    Ok(MediaRequestKey::from_bytes(bytes))
}
fn prepare(candidate: Candidate) -> Result<Prepared, PublicationError> {
    let recipe =
        df_imagery::retained_campaign_artifact_recipe(&candidate.job.intent, &candidate.job.config)
            .map_err(|_| PublicationError::InvalidRecord)?
            .ok_or(PublicationError::InvalidRecord)?;
    let reference_recipe = candidate
        .reference
        .as_ref()
        .map(|reference| {
            df_imagery::retained_campaign_artifact_recipe(
                &reference.job.intent,
                &reference.job.config,
            )
            .map_err(|_| PublicationError::InvalidRecord)?
            .ok_or(PublicationError::InvalidRecord)
        })
        .transpose()?;
    let selected = candidate
        .selected_recipe
        .as_ref()
        .or(candidate.job.recipe.as_ref())
        .ok_or(PublicationError::InvalidRecord)?;
    let frozen = FrozenOutput::decode(selected)?;
    let recipe_digest = recipe_digest(selected);
    let master = frozen
        .intent(df_assets::AssetVariant::Master)
        .ok_or(PublicationError::InvalidRecord)?;
    let display = frozen.intent(df_assets::AssetVariant::Display);
    let requires_selection = df_imagery::runtime::requires_final_selection(&candidate.job)
        .map_err(|_| PublicationError::InvalidRecord)?;
    Ok(Prepared {
        candidate,
        recipe,
        reference_recipe,
        recipe_digest,
        master,
        display,
        requires_selection,
    })
}
async fn prove(
    tx: &Transaction<'_>,
    target: &Target<'_>,
    prepared: &Prepared,
    now_ms: i64,
    authority: &dyn PublicationSourceAuthority,
) -> Result<Projection, PublicationError> {
    let candidate = &prepared.candidate;
    let locked = sources::lock_artifact_in(
        tx,
        &candidate.scope,
        candidate.asset,
        authority,
        target.owner(),
        now_ms,
    )
    .await?;
    match target {
        Target::Request(request, _) => {
            authority
                .authorize_artifact(tx, request, &locked.output, &locked.sources, now_ms)
                .await?
        }
        _ => {
            authority
                .authorize_artifact_read(
                    tx,
                    target.owner(),
                    &locked.output,
                    &locked.sources,
                    now_ms,
                )
                .await?
        }
    }
    check_semantics(target, prepared, &locked.output)?;
    lock_jobs(tx, candidate).await?;
    let current = crate::read_in(tx, &candidate.scope.owner, candidate.scope.job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    if current.intent != candidate.job.intent
        || current.config != candidate.job.config
        || current.recipe != candidate.job.recipe
        || current.resolved_key != Some(prepared.recipe.key)
    {
        return Err(PublicationError::BindingConflict);
    }
    rows::require_delivery_open_in(tx, &current.owner, &current.job).await?;
    if let Some(reference) = &candidate.reference {
        check_reference(tx, reference, prepared).await?;
    }
    let result = check_result(tx, &current, &candidate.scope, &locked, prepared).await?;
    reusable::check_winners(tx, &result, &locked.output)
        .await
        .map_err(|error| match error {
            PublicationError::BindingConflict => {
                PublicationError::SourceDenied(df_imagery::publication::SourceDenial::WinnerChanged)
            }
            error => error,
        })?;
    let inspection =
        inspection::read_published_inspection_in(tx, &current.owner, &current.job).await?;
    Ok(Projection {
        job: current,
        result,
        inspection,
        key: prepared.recipe.key,
        audience: locked.output.audience,
    })
}

fn check_semantics(
    target: &Target<'_>,
    prepared: &Prepared,
    asset: &df_assets::AssetRecord,
) -> Result<(), PublicationError> {
    let candidate = &prepared.candidate;
    if candidate.key != prepared.recipe.key || candidate.semantic != prepared.recipe.semantic {
        return Err(PublicationError::InvalidRecord);
    }
    let ImageOwner::Run { campaign, .. } = target.owner() else {
        return Err(PublicationError::BindingConflict);
    };
    if asset.state != AssetState::Ready
        || asset.kind != AssetKind::Image
        || asset.provenance != Provenance::Generated
        || asset.audience != Audience::CampaignShared(*campaign)
        || asset.campaign != Some(*campaign)
        || asset.retention != RetentionClass::CampaignMedia
        || asset.request_key != Some(prepared.recipe.key)
        || asset.dedupe_scope != DedupeScope::campaign(campaign)
        || asset.attempt != candidate.attempt
    {
        return Err(PublicationError::BindingConflict);
    }
    if let Target::Request(_, wanted) = target
        && **wanted != prepared.recipe
    {
        return Err(PublicationError::InvalidRecord);
    }
    Ok(())
}

async fn lock_jobs(tx: &Transaction<'_>, candidate: &Candidate) -> Result<(), PublicationError> {
    let mut jobs = vec![(&candidate.job.owner, candidate.job.job)];
    if let Some(reference) = &candidate.reference {
        jobs.push((&reference.job.owner, reference.job.job));
    }
    jobs.sort_unstable_by_key(|(owner, job)| (owner_scope_bytes(owner), *job.as_bytes()));
    jobs.dedup();
    for (owner, job) in jobs {
        tx.query_opt(
            "SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE",
            &[&owner_scope_bytes(owner), &job.as_bytes().as_slice()],
        )
        .await
        .map_err(rows::database_error)?
        .ok_or(PublicationError::NotFound)?;
    }
    Ok(())
}

async fn check_result(
    tx: &Transaction<'_>,
    job: &ImageJob,
    scope: &ExecutionScope,
    locked: &sources::LockedSources,
    prepared: &Prepared,
) -> Result<PublishedImage, PublicationError> {
    let candidate = &prepared.candidate;
    let selection = selection::read_in(tx, &job.owner, &job.job).await?;
    if selection.is_some() != candidate.selected_recipe.is_some() {
        return Err(PublicationError::InvalidRecord);
    }
    let recipe = if let Some(selected) = &selection {
        if selected.final_stage != scope.stage
            || selected.final_dispatch != scope.dispatch
            || selected.source_binding != locked.binding
        {
            return Err(PublicationError::BindingConflict);
        }
        &selected.recipe
    } else {
        if prepared.requires_selection {
            return Err(PublicationError::BindingConflict);
        }
        job.recipe
            .as_ref()
            .ok_or(PublicationError::BindingConflict)?
    };
    let expected = candidate
        .selected_recipe
        .as_ref()
        .or(candidate.job.recipe.as_ref())
        .ok_or(PublicationError::InvalidRecord)?;
    if recipe != expected {
        return Err(PublicationError::InvalidRecord);
    }
    let Some((result, _)) = rows::stored_result(tx, &job.owner, &job.job).await? else {
        return Err(PublicationError::NotFound);
    };
    if result.private_reconciliation
        || result.published_at_ms <= 0
        || result.published_at_ms >= job.absolute_deadline_ms
        || result.stage != scope.stage
        || result.dispatch != scope.dispatch
        || result.asset != candidate.asset
        || result.attempt != candidate.attempt
        || result.recipe_digest != prepared.recipe_digest
        || result.job_revision > job.revision
    {
        return Err(PublicationError::BindingConflict);
    }
    let expected_count = if prepared.display.is_some() { 2 } else { 1 };
    if result.variants.len() != expected_count {
        return Err(PublicationError::InvalidRecord);
    }
    for actual in &result.variants {
        let expected = match actual.variant {
            df_assets::AssetVariant::Master => Some(prepared.master),
            df_assets::AssetVariant::Display => prepared.display,
        };
        if expected.is_none_or(|expected| !expected.accepts(actual.intent, actual.alpha)) {
            return Err(PublicationError::InvalidRecord);
        }
    }
    let row=tx.query_one("SELECT semantic_recipe_revision,semantic_recipe_bytes,original_recipe_key FROM imagery.image_publication_allocation WHERE asset_id=$1 AND publication_attempt=$2",
        &[&candidate.asset.as_bytes().as_slice(),&candidate.attempt.as_bytes().as_slice()]).await.map_err(rows::database_error)?;
    if rows::snapshot(&row, "semantic_recipe_revision", "semantic_recipe_bytes")?
        != candidate.semantic
        || key(&row, "original_recipe_key")? != candidate.key
    {
        return Err(PublicationError::InvalidRecord);
    }
    Ok(result)
}
async fn load_reference(
    tx: &Transaction<'_>,
    owner: &ImageOwner,
    job: &ImageJobId,
) -> Result<Option<Reference>, PublicationError> {
    let Some(row)=tx.query_opt("SELECT * FROM imagery.image_artifact_dependency WHERE owner_scope=$1 AND job_id=$2 AND reference_state='resolved'",
        &[&owner_scope_bytes(owner),&job.as_bytes().as_slice()]).await.map_err(rows::database_error)? else {return Ok(None)};
    let actual = crate::read_in(tx, owner, *job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    Ok(Some(Reference {
        job: actual,
        stage: ExecutionStageId::from_bytes(&row.get::<_, Vec<u8>>("stage_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        generation: row.get("claim_generation"),
        semantic: rows::snapshot(&row, "semantic_recipe_revision", "semantic_recipe_bytes")?,
        key: key(&row, "semantic_key")?,
        asset: AssetId::from_bytes(&row.get::<_, Vec<u8>>("asset_id"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        attempt: df_assets::AttemptId::from_bytes(&row.get::<_, Vec<u8>>("publication_attempt"))
            .map_err(|_| PublicationError::InvalidRecord)?,
        resolved_at_ms: row
            .get::<_, Option<i64>>("resolved_at_ms")
            .ok_or(PublicationError::InvalidRecord)?,
        resolved_revision: row
            .get::<_, Option<i64>>("resolved_job_revision")
            .ok_or(PublicationError::InvalidRecord)?,
    }))
}

async fn check_reference(
    tx: &Transaction<'_>,
    reference: &Reference,
    prepared: &Prepared,
) -> Result<(), PublicationError> {
    let wanted = prepared
        .reference_recipe
        .as_ref()
        .ok_or(PublicationError::BindingConflict)?;
    if wanted != &prepared.recipe
        || reference.key != wanted.key
        || reference.semantic != wanted.semantic
        || reference.resolved_at_ms >= reference.job.absolute_deadline_ms
        || reference.resolved_revision <= 0
    {
        return Err(PublicationError::BindingConflict);
    }
    let row=tx.query_one(concat!(
        "SELECT j.intent_bytes,j.config_bytes, j.revision>=d.resolved_job_revision AND j.state<>'retired' ",
        "AND d.reference_state='resolved' AND d.stage_id=$3 AND d.claim_generation=$4 ",
        "AND d.resolved_at_ms=$5 AND d.resolved_job_revision=$6 AND d.asset_id=$7 AND d.publication_attempt=$8 ",
        "AND d.semantic_recipe_revision=$9 AND d.semantic_recipe_bytes=$10 AND d.semantic_key=$11 ",
        "AND s.claim_generation=d.claim_generation AND s.stage_state='released' AND NOT s.capacity_reserved ",
        "AND imagery.stage_has_finance(j.owner_scope,j.job_id,s.stage_id) IS FALSE ",
        "AND EXISTS(SELECT 1 FROM imagery.image_interest i WHERE i.owner_scope=j.owner_scope AND i.job_id=j.job_id AND i.state='attached') ",
        "AND NOT EXISTS(SELECT 1 FROM imagery.image_dispatch_attempt a WHERE a.job_id=j.job_id) ",
        "AND NOT EXISTS(SELECT 1 FROM imagery.image_publication_allocation a WHERE a.owner_scope=j.owner_scope AND a.job_id=j.job_id) ",
        "AND NOT EXISTS(SELECT 1 FROM imagery.image_job_result r WHERE r.owner_scope=j.owner_scope AND r.job_id=j.job_id) ",
        "AND NOT EXISTS(SELECT 1 FROM imagery.image_no_delivery n WHERE n.owner_scope=j.owner_scope AND n.job_id=j.job_id) AS valid ",
        "FROM imagery.image_job j JOIN imagery.image_artifact_dependency d ON d.owner_scope=j.owner_scope AND d.job_id=j.job_id ",
        "JOIN imagery.image_execution_stage s ON s.job_id=j.job_id AND s.stage_id=d.stage_id AND s.owner_scope=j.owner_scope ",
        "WHERE j.owner_scope=$1 AND j.job_id=$2"),
        &[&owner_scope_bytes(&reference.job.owner),&reference.job.job.as_bytes().as_slice(),&reference.stage.as_bytes().as_slice(),&reference.generation,
        &reference.resolved_at_ms,&reference.resolved_revision,&reference.asset.as_bytes().as_slice(),&reference.attempt.as_bytes().as_slice(),
        &i64::from(reference.semantic.revision()),&reference.semantic.bytes(),&reference.key.as_bytes().as_slice()]).await.map_err(rows::database_error)?;
    if !row.get::<_, bool>("valid")
        || row.get::<_, Vec<u8>>("intent_bytes") != reference.job.intent.bytes()
        || row.get::<_, Vec<u8>>("config_bytes") != reference.job.config.bytes()
    {
        return Err(PublicationError::BindingConflict);
    }
    Ok(())
}

fn project(projection: Projection) -> Result<ReadyArtifact, PublicationError> {
    let assessment = projection
        .inspection
        .map(|retained| match retained.evidence {
            inspection::PublishedInspectionEvidence::Decision(decision) => {
                df_imagery::runtime::project_inspection_decision(&retained.job, &decision)
            }
            inspection::PublishedInspectionEvidence::Local(local) => {
                df_imagery::runtime::project_local_inspection(&retained.job, &local)
            }
        })
        .transpose()
        .map_err(|_| PublicationError::BindingConflict)?;
    df_imagery::project_retained_campaign_artifact(
        &projection.job.intent,
        &projection.job.config,
        ReadyArtifact {
            asset: projection.result.asset,
            attempt: projection.result.attempt,
            audience: projection.audience,
            semantic_key: projection.key,
            published_at_ms: projection.result.published_at_ms,
            variants: projection.result.variants,
            inspection: assessment,
            staging: Vec::new(),
        },
    )
    .map_err(|_| PublicationError::BindingConflict)
}
