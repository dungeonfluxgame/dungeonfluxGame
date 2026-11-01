//! Eligible initial artwork stages elect one producer or retain an actual owned unsent dependency.
use super::*;
use crate::imagery_publication::{
    artifact::{self as reader, ArtifactJobTarget},
    artifact_admission::{self as writer, Election, ElectionContext},
};
use df_imagery::runtime::ArtifactAdmission;

impl<S: ObjectStore + 'static, I: RuntimeIds, C: Clock> NativeImagePublisher<S, I, C> {
    pub(in crate::imagery_publication::runtime) async fn admit_artifact(
        &self,
        input: &DispatchAdmission<'_>,
    ) -> Result<ArtifactAdmission, RuntimeError> {
        let intent = input.job.intent.clone();
        let config = input.job.config.clone();
        let (recipe, config_digest) = tokio::task::spawn_blocking(move || {
            df_imagery::retained_campaign_artifact_recipe(&intent, &config)
                .map(|recipe| (recipe, df_commerce::digest::sha256(config.bytes())))
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let Some(recipe) = recipe else {
            return self.admit(input).await.map(ArtifactAdmission::Admitted);
        };
        if !matches!(input.stage.kind, StageKind::Generation | StageKind::Edit)
            || retained_config(input.job)? != *input.config
            || input.stage.owner != input.job.owner
            || input.stage.job != input.job.job
            || input.job.resolved_key != Some(recipe.key)
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let authority = self
            .source_authority
            .as_deref()
            .ok_or(RuntimeError::UnsupportedCapability)?;
        let admission = self.build_admission(input)?;
        let now = self.now()?;
        let candidate = {
            let mut slot = self
                .ports
                .publication
                .pool
                .checkout()
                .await
                .map_err(catalog_error)?;
            let client = slot
                .client
                .as_mut()
                .ok_or(RuntimeError::StorageUnavailable)?;
            let tx = client
                .transaction()
                .await
                .map_err(|error| publication_error(rows::database_error(error)))?;
            let candidate = writer::candidate_in(&tx, &admission).await?;
            crate::imagery_publication::commit_publication(tx)
                .await
                .map_err(publication_error)?;
            candidate
        };
        let mut target = candidate.map(|candidate| ArtifactJobTarget {
            job: input.job,
            asset: candidate.asset,
            attempt: candidate.attempt,
            wanted: &recipe,
            now_ms: now,
            authority,
        });
        let prepared = match &target {
            Some(target) => self
                .ports
                .publication
                .prepare_ready_artifact_for_job(target)
                .await
                .map_err(publication_error)?,
            None => None,
        };
        let now = self.now()?;
        if let Some(target) = &mut target {
            target.now_ms = now;
        }
        let mut slot = self
            .ports
            .publication
            .pool
            .checkout()
            .await
            .map_err(catalog_error)?;
        let client = slot
            .client
            .as_mut()
            .ok_or(RuntimeError::StorageUnavailable)?;
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(|error| publication_error(rows::database_error(error)))?;
        let outcome = if let (Some(target), Some(prepared), Some(candidate)) =
            (&target, &prepared, candidate)
        {
            if !reader::prove_prepared_artifact_in(&tx, target, prepared)
                .await
                .map_err(publication_error)?
            {
                return Err(RuntimeError::SourceDenied);
            }
            writer::resolve_in(&tx, input, &recipe, candidate, config_digest, now).await?;
            ArtifactAdmission::ArtifactResolved
        } else {
            let backoff_ms = i64::try_from(input.config.retry.backoff_secs)
                .ok()
                .and_then(|seconds| seconds.checked_mul(1_000))
                .ok_or(RuntimeError::InvalidConfiguration)?
                .max(1);
            let not_before_ms = now
                .checked_add(backoff_ms)
                .ok_or(RuntimeError::InvalidConfiguration)?
                .min(input.job.absolute_deadline_ms.saturating_sub(1));
            if not_before_ms <= now {
                return Err(RuntimeError::SourceDenied);
            }
            match writer::elect_in(
                &tx,
                &ElectionContext {
                    input,
                    admission: &admission,
                    recipe: &recipe,
                    authority,
                    now_ms: now,
                    not_before_ms,
                    config_digest,
                },
            )
            .await?
            {
                Election::Producer => ArtifactAdmission::Admitted(DispatchMode::Local),
                Election::Pending => ArtifactAdmission::ArtifactPending,
            }
        };
        // A lost acknowledgement is Unknown, never permission to adopt or prepare another dispatch.
        crate::imagery_publication::commit_publication(tx)
            .await
            .map_err(publication_error)?;
        Ok(outcome)
    }
}
