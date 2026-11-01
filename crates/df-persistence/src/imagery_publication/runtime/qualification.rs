//! Actual retained decision/raw evidence is qualified on bounded CPU work before publication locks.
use super::{NativeImagePublisher, catalog_error, conflict, publication_error};
use crate::imagery_publication::{
    commit_publication, decision_binding, local_inspections, rows, selection::StoredSelection,
};
use df_assets::{Clock, ObjectStore};
use df_imagery::execution::{DecisionKind, ExecutionScope};
use df_imagery::publication::{FinalDecisionRef, FinalOutputSelection};
use df_imagery::runtime::{
    PublicationContext, PublicationDecisionEvidence, PublicationEvidence, QualifiedPipeline,
    RuntimeError, RuntimeIds, qualify_publication,
};

impl<S: ObjectStore + 'static, I: RuntimeIds, C: Clock> NativeImagePublisher<S, I, C> {
    pub(super) async fn qualify_stored(
        &self,
        context: &PublicationContext,
        selected: &StoredSelection,
    ) -> Result<Option<QualifiedPipeline>, RuntimeError> {
        self.qualify_selection(
            context,
            &FinalOutputSelection {
                allocation_stage: selected.allocation_stage,
                candidate: selected.candidate.clone(),
                resolved_recipe: selected.recipe.clone(),
                decisions: selected.decisions.clone(),
                local_inspections: selected.local_inspections.clone(),
            },
        )
        .await
    }

    pub(super) async fn qualify_selection(
        &self,
        context: &PublicationContext,
        selected: &FinalOutputSelection,
    ) -> Result<Option<QualifiedPipeline>, RuntimeError> {
        let Some(candidate) = &selected.candidate else {
            return if selected.decisions.is_empty() && selected.local_inspections.is_empty() {
                Ok(None)
            } else {
                Err(conflict())
            };
        };
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
        let job = crate::read_in(&tx, &context.job.owner, context.job.job)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::InvalidRecord)?;
        if job.intent != context.job.intent
            || job.config != context.job.config
            || job.recipe != context.job.recipe
        {
            return Err(conflict());
        }
        let sources = decision_binding::verify_in(&tx, &job, candidate, &selected.decisions)
            .await
            .map_err(publication_error)?;
        local_inspections::verify_in(&tx, &job, &selected.local_inspections, &sources)
            .await
            .map_err(publication_error)?;
        let mut evidence = Vec::with_capacity(selected.decisions.len());
        let mut retained_bytes = 0u64;
        let config = df_imagery::runtime::retained_config(&job)?;
        for reference in &selected.decisions {
            let decision = crate::imagery_execution::read_decision_checkpoint_in(
                &tx,
                &job.owner,
                &job.job,
                &reference.scope.stage,
                &reference.scope.dispatch,
            )
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::InvalidRecord)?;
            if FinalDecisionRef::from_observation(&decision) != *reference {
                return Err(conflict());
            }
            let usage = crate::imagery_runtime::read_usage_in(
                &tx,
                &ExecutionScope {
                    owner: job.owner.clone(),
                    job: job.job,
                    stage: decision.scope.stage,
                    dispatch: decision.scope.dispatch,
                },
                df_imagery::cost::UsageRevision(u64::from(reference.usage_revision)),
            )
            .await?
            .ok_or(RuntimeError::InvalidRecord)?;
            let bytes = usage
                .raw_output
                .as_ref()
                .ok_or(RuntimeError::InvalidRecord)?
                .len();
            retained_bytes = retained_bytes
                .checked_add(u64::try_from(bytes).map_err(|_| RuntimeError::InvalidRecord)?)
                .ok_or(RuntimeError::InvalidRecord)?;
            if retained_bytes > config.transforms.max_decoded_bytes {
                return Err(RuntimeError::InvalidConfiguration);
            }
            let source_candidate = if decision.kind == DecisionKind::Inspection {
                let parent = decision
                    .basis
                    .stage
                    .parent_dispatch
                    .ok_or(RuntimeError::InvalidRecord)?;
                Some(
                    sources
                        .iter()
                        .find(|source| source.scope.dispatch == parent)
                        .ok_or(RuntimeError::InvalidRecord)?
                        .clone(),
                )
            } else {
                None
            };
            evidence.push(PublicationDecisionEvidence {
                decision,
                usage,
                source_candidate,
            });
        }
        commit_publication(tx).await.map_err(publication_error)?;
        let permit = self
            .decode_semaphore()?
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::WorkerFailed)?;
        let candidate = candidate.clone();
        let locals = selected.local_inspections.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            qualify_publication(
                &job,
                &PublicationEvidence {
                    decisions: evidence,
                    candidates: sources,
                    local_inspections: locals,
                },
                &candidate,
            )
            .map(Some)
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?
    }
}
