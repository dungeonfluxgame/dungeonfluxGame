//! A prepared Ready candidate confers no authority; the fresh writer transaction reproves it.
use super::{
    Candidate, PostgresImagePublication, Prepared, Projection, PublicationSourceAuthority, Target,
    commit_publication, inspection, load_candidate, prepare, project, prove, rows,
};
use df_assets::{AssetId, AttemptId};
use df_imagery::execution::LocalInspectionCheckpoint;
use df_imagery::publication::{FinalDecisionRef, PublicationError, ReadyArtifact};
use df_imagery::{CampaignArtifactRecipe, ImageJob};
use tokio_postgres::Transaction;

/// The actual waiting job and elected target, never a forged producer request.
pub(in crate::imagery_publication) struct ArtifactJobTarget<'a> {
    pub job: &'a ImageJob,
    pub asset: AssetId,
    pub attempt: AttemptId,
    pub wanted: &'a CampaignArtifactRecipe,
    pub now_ms: i64,
    pub authority: &'a dyn PublicationSourceAuthority,
}

/// Immutable precomputation only; the writer cannot construct or use it as an access grant.
pub(in crate::imagery_publication) struct PreparedArtifact {
    ready: Prepared,
    artifact: ReadyArtifact,
    inspection: InspectionIdentity,
    job: ImageJob,
    own_recipe: CampaignArtifactRecipe,
}

#[derive(Eq, PartialEq)]
enum InspectionIdentity {
    Absent,
    Decision(FinalDecisionRef),
    Local(LocalInspectionCheckpoint),
}

fn inspection_identity(projection: &Projection) -> InspectionIdentity {
    match projection
        .inspection
        .as_ref()
        .map(|inspection| &inspection.evidence)
    {
        None => InspectionIdentity::Absent,
        Some(inspection::PublishedInspectionEvidence::Decision(decision)) => {
            InspectionIdentity::Decision(FinalDecisionRef::from_observation(decision))
        }
        Some(inspection::PublishedInspectionEvidence::Local(local)) => {
            InspectionIdentity::Local(local.clone())
        }
    }
}

impl PostgresImagePublication {
    /// Prepares outside the writer transaction; no artifact or reference is admitted by this read.
    pub(in crate::imagery_publication) async fn prepare_ready_artifact_for_job(
        &self,
        target: &ArtifactJobTarget<'_>,
    ) -> Result<Option<PreparedArtifact>, PublicationError> {
        if target.now_ms < 0 {
            return Err(PublicationError::InvalidRecord);
        }
        let mut slot = self
            .pool
            .checkout()
            .await
            .map_err(PublicationError::Catalog)?;
        let client = slot.client.as_mut().ok_or(PublicationError::Unavailable)?;
        let tx = client.transaction().await.map_err(rows::database_error)?;
        target
            .authority
            .lock_authority(&tx, &target.job.owner, target.now_ms)
            .await?;
        let candidate =
            load_candidate(&tx, &Target::Asset(&target.job.owner, &target.asset)).await?;
        let Some(candidate) = candidate else {
            return Ok(None);
        };
        if candidate.attempt != target.attempt {
            return Ok(None);
        }
        let projection = read_projection(&tx, &candidate).await?;
        commit_publication(tx).await?;
        let job = target.job.clone();
        tokio::task::spawn_blocking(move || prepare_projection(candidate, projection, job))
            .await
            .map_err(|_| PublicationError::Unavailable)?
            .map(Some)
    }
}

async fn read_projection(
    tx: &Transaction<'_>,
    candidate: &Candidate,
) -> Result<Projection, PublicationError> {
    let Some((result, _)) =
        rows::stored_result(tx, &candidate.job.owner, &candidate.job.job).await?
    else {
        return Err(PublicationError::NotFound);
    };
    let inspection =
        inspection::read_published_inspection_in(tx, &candidate.job.owner, &candidate.job.job)
            .await?;
    Ok(Projection {
        job: candidate.job.clone(),
        result,
        inspection,
        key: candidate.key,
        audience: df_imagery::runtime::retained_audience(&candidate.job)
            .map_err(|_| PublicationError::InvalidRecord)?,
    })
}

fn prepare_projection(
    candidate: Candidate,
    projection: Projection,
    job: ImageJob,
) -> Result<PreparedArtifact, PublicationError> {
    let ready = prepare(candidate)?;
    let inspection = inspection_identity(&projection);
    let artifact = project(projection)?;
    let own_recipe = df_imagery::retained_campaign_artifact_recipe(&job.intent, &job.config)
        .map_err(|_| PublicationError::InvalidRecord)?
        .ok_or(PublicationError::InvalidRecord)?;
    Ok(PreparedArtifact {
        ready,
        artifact,
        inspection,
        job,
        own_recipe,
    })
}

/// Caller holds no imagery locks; true still requires its real unsent-stage CAS and reference write.
pub(in crate::imagery_publication) async fn prove_prepared_artifact_in(
    tx: &Transaction<'_>,
    target: &ArtifactJobTarget<'_>,
    prepared: &PreparedArtifact,
) -> Result<bool, PublicationError> {
    if target.now_ms < 0 {
        return Err(PublicationError::InvalidRecord);
    }
    if target.asset != prepared.artifact.asset
        || target.attempt != prepared.artifact.attempt
        || target.job.owner != prepared.job.owner
        || target.job.job != prepared.job.job
        || target.job.intent != prepared.job.intent
        || target.job.config != prepared.job.config
    {
        return Err(PublicationError::InvalidRecord);
    }
    let access = Target::Asset(&target.job.owner, &target.asset);
    let projection = match prove(
        tx,
        &access,
        &prepared.ready,
        target.now_ms,
        target.authority,
        Some(target.job),
    )
    .await
    {
        Ok(projection) => projection,
        Err(PublicationError::NotFound | PublicationError::BindingConflict) => return Ok(false),
        Err(error) => return Err(error),
    };
    if prepared.own_recipe != *target.wanted
        || prepared.own_recipe != prepared.ready.recipe
        || !same_projection(&projection, prepared)
        || inspection_identity(&projection) != prepared.inspection
    {
        return Err(PublicationError::InvalidRecord);
    }
    let current = crate::read_in(tx, &target.job.owner, target.job.job)
        .await
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::NotFound)?;
    if current.intent != prepared.job.intent
        || current.config != prepared.job.config
        || current.resolved_key != Some(target.wanted.key)
    {
        return Err(PublicationError::InvalidRecord);
    }
    rows::require_delivery_open_in(tx, &current.owner, &current.job).await?;
    Ok(true)
}

fn same_projection(projection: &Projection, prepared: &PreparedArtifact) -> bool {
    let result = &projection.result;
    let expected = &prepared.artifact;
    result.asset == expected.asset
        && result.attempt == expected.attempt
        && result.variants == expected.variants
        && result.published_at_ms == expected.published_at_ms
        && projection.audience == expected.audience
        && projection.key == expected.semantic_key
}
