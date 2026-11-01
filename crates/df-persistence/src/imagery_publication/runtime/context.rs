//! Validates retained execution context and its current publication source authority.
use super::{NativeImagePublisher, catalog_error, conflict, proof_error, publication_error, scope};
use crate::imagery_publication::{commit_publication, rows, selection, sources};
use df_assets::{AssetVariant, Clock, ImageProofLimits, ObjectStore};
use df_imagery::owner_scope_bytes;
use df_imagery::publication::{FrozenOutput, PublicationError, recipe_digest};
use df_imagery::runtime::{PublicationContext, RuntimeError, RuntimeIds, retained_config};

impl<S: ObjectStore + 'static, I: RuntimeIds, C: Clock> NativeImagePublisher<S, I, C> {
    pub(super) async fn read_image_economic_use(
        &self,
        scope: &df_imagery::execution::ExecutionScope,
    ) -> Result<Option<df_imagery::runtime::ImageEconomicDisposition>, RuntimeError> {
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
        let owned=tx.query_opt("SELECT job_id FROM imagery.image_job WHERE owner_scope=$1 AND job_id=$2 FOR UPDATE", &[&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice()]).await.map_err(|error|publication_error(rows::database_error(error)))?;
        if owned.is_none() {
            return Err(RuntimeError::InvalidRecord);
        }
        let value = crate::imagery_publication::stage_economic_use_in(
            &tx,
            &scope.owner,
            &scope.job,
            &scope.stage,
            &scope.dispatch,
        )
        .await
        .map_err(publication_error)?;
        let result = value.map(|value| df_imagery::runtime::ImageEconomicDisposition {
            usage_revision: value.usage_revision(),
            usable: matches!(
                value,
                crate::imagery_publication::StageEconomicUse::SelectedCandidate { .. }
            ),
        });
        if value.is_some_and(|value| {
            matches!(
                value,
                crate::imagery_publication::StageEconomicUse::Decision { .. }
            )
        }) {
            return Err(conflict());
        }
        commit_publication(tx).await.map_err(publication_error)?;
        Ok(result)
    }

    pub(super) async fn final_selection(
        &self,
        context: &PublicationContext,
    ) -> Result<Option<selection::StoredSelection>, RuntimeError> {
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
        let selected = selection::read_in(&tx, &context.job.owner, &context.job.job)
            .await
            .map_err(publication_error)?;
        if selected.as_ref().is_some_and(|selected| {
            selected.final_stage != context.stage.stage
                || selected.final_dispatch != context.dispatch
        }) {
            return Err(conflict());
        }
        commit_publication(tx).await.map_err(publication_error)?;
        Ok(selected)
    }

    pub(super) async fn recipe(
        &self,
        context: &PublicationContext,
    ) -> Result<(FrozenOutput, [u8; 32], ImageProofLimits), RuntimeError> {
        let plan = context
            .job
            .recipe
            .as_ref()
            .ok_or(RuntimeError::InvalidRecord)?;
        let selected = self.final_selection(context).await?;
        if selected.is_none() && df_imagery::runtime::requires_final_selection(&context.job)? {
            return Err(conflict());
        }
        let snapshot = selected.as_ref().map_or(plan, |selected| &selected.recipe);
        let frozen = FrozenOutput::decode(snapshot).map_err(publication_error)?;
        let config = retained_config(&context.job)?;
        let pixels = [AssetVariant::Master, AssetVariant::Display]
            .into_iter()
            .filter_map(|variant| frozen.intent(variant))
            .map(|intent| u64::from(intent.width_px()) * u64::from(intent.height_px()))
            .max()
            .ok_or(RuntimeError::InvalidRecord)?;
        let limits = ImageProofLimits::new(
            config.transforms.max_output_bytes,
            pixels,
            config.transforms.max_decoded_bytes,
        )
        .map_err(proof_error)?;
        let digest = recipe_digest(snapshot);
        Ok((frozen, digest, limits))
    }

    pub(super) async fn check_context(
        &self,
        context: &PublicationContext,
    ) -> Result<(), RuntimeError> {
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
        let row=client.query_opt("SELECT s.claim_token,s.claim_generation,c.current_dispatch,d.claim_token AS dispatch_claim,d.claim_generation AS dispatch_generation FROM imagery.image_execution_stage s JOIN imagery.image_dispatch_attempt d USING(job_id,stage_id) JOIN imagery.image_stage_cursor c USING(job_id,stage_id) WHERE s.job_id=$1 AND s.stage_id=$2 AND d.dispatch_id=$3 AND s.owner_scope=$4", &[&context.job.job.as_bytes().as_slice(),&context.stage.stage.as_bytes().as_slice(),&context.dispatch.as_bytes().as_slice(),&owner_scope_bytes(&context.job.owner)]).await.map_err(|error|publication_error(rows::database_error(error)))?.ok_or(RuntimeError::InvalidRecord)?;
        let generation = i64::try_from(context.stage.claim_generation)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        if (context.recovery.is_none()
            && (row.get::<_, Vec<u8>>("claim_token") != context.claim.as_bytes()
                || row.get::<_, i64>("claim_generation") != generation))
            || row.get::<_, Vec<u8>>("dispatch_claim") != context.claim.as_bytes()
            || row.get::<_, i64>("dispatch_generation") != generation
            || row.get::<_, Vec<u8>>("current_dispatch") != context.dispatch.as_bytes()
        {
            return Err(conflict());
        }
        if let Some(recovery) = &context.recovery {
            if recovery.stage.owner != context.job.owner
                || recovery.stage.job != context.job.job
                || recovery.stage.stage != context.stage.stage
                || recovery.stage.current.dispatch != context.dispatch
                || recovery.stage.claim != context.claim
                || recovery.stage.claim_generation != context.stage.claim_generation
            {
                return Err(conflict());
            }
            let tx = client
                .transaction()
                .await
                .map_err(|error| publication_error(rows::database_error(error)))?;
            crate::imagery_execution::validate_recovery_in(&tx, recovery, self.now()?)
                .await
                .map_err(RuntimeError::Execution)?;
            tx.commit()
                .await
                .map_err(|_| publication_error(PublicationError::Unknown))?;
        }
        Ok(())
    }

    pub(super) async fn check_sources(
        &self,
        context: &PublicationContext,
    ) -> Result<(), RuntimeError> {
        let (asset, _) = self.ids(context)?;
        let now = self.now()?;
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
        let locked = sources::lock_in(
            &tx,
            &scope(context),
            asset,
            self.source_authority.as_deref(),
            now,
        )
        .await
        .map_err(publication_error)?;
        tx.query_one(
            "SELECT job_id FROM imagery.image_job WHERE job_id=$1 AND owner_scope=$2 FOR UPDATE",
            &[
                &context.job.job.as_bytes().as_slice(),
                &owner_scope_bytes(&context.job.owner),
            ],
        )
        .await
        .map_err(|error| publication_error(rows::database_error(error)))?;
        sources::authorize_in(&tx, &locked, self.source_authority.as_deref(), now)
            .await
            .map_err(publication_error)?;
        let selected = selection::read_in(&tx, &context.job.owner, &context.job.job)
            .await
            .map_err(publication_error)?;
        if selected
            .as_ref()
            .is_some_and(|selected| selected.source_binding != locked.binding)
        {
            return Err(conflict());
        }
        commit_publication(tx).await.map_err(publication_error)
    }
}
