//! Off-lock campaign semantics are pinned to the actual immutable job before catalog election.
use super::{Error, StageAllocation, StageConfig};
use df_imagery::{CampaignArtifactRecipe, ImageJob, ImageOwner};

pub(super) struct CampaignAllocation {
    pub(super) intent_revision: u32,
    pub(super) intent_digest: [u8; 32],
    pub(super) recipe: CampaignArtifactRecipe,
}
impl CampaignAllocation {
    pub(super) fn qualify(config: &StageConfig, job: &ImageJob) -> Result<Self, Error> {
        let StageAllocation::Output(output) = &config.allocation else {
            return Err(Error::LinkConflict);
        };
        let recipe = df_imagery::retained_campaign_artifact_recipe(&job.intent, &job.config)
            .map_err(|_| Error::InvalidRecord)?
            .ok_or(Error::CatalogRefused)?;
        if !matches!(job.owner, ImageOwner::Run { .. })
            || config.claim.scope.owner != job.owner
            || config.claim.scope.job != job.job
            || job.recipe.as_ref() != Some(&output.recipe)
            || job.config != output.config
            || job.resolved_key != Some(recipe.key)
            || output.recipe_key != recipe.key
            || job.revision != output.expected_job_revision
            || job.absolute_deadline_ms != output.absolute_deadline_ms
        {
            return Err(Error::LinkConflict);
        }
        Ok(Self {
            intent_revision: job.intent.revision(),
            intent_digest: df_commerce::digest::sha256(job.intent.bytes()),
            recipe,
        })
    }
}
