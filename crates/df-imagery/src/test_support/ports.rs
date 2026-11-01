//! Exact in-memory source/publication edges; every paid capability refuses.
use super::state::{MAXIMUM_BYTES, MAXIMUM_RECORDS, Store};
use crate::cost::{
    CostError, CostFuture, CostRoute, ImageCostProjection, PinnedCorrection, PinnedStart,
    PinnedUnsent, UsageReport, UsageRevision,
};
use crate::execution::*;
use crate::publication::{
    FrozenOutput, ImageResults, PublicationError, PublishedImage, PublishedVariant, recipe_digest,
};
use crate::runtime::*;
use crate::{
    ImageExecutionState, ImageJobId, ImageObservations, ImageOwner, ImageProgress, JobState,
    JournalError, SourceRef,
};
use df_assets::{AssetVariant, ImageAlpha};

impl ImageSources for Store {
    async fn read_source(
        &self,
        owner: &ImageOwner,
        source: &SourceRef,
    ) -> Result<AuthorizedImageSource, RuntimeError> {
        let state = self.lock()?;
        if owner != &state.owner {
            return Err(RuntimeError::SourceDenied);
        }
        let asset = state
            .assets
            .iter()
            .find(|asset| asset.receipt.pin == *source)
            .ok_or(RuntimeError::SourceDenied)?;
        Ok(AuthorizedImageSource {
            bytes: asset.bytes.clone(),
            receipt: asset.receipt.clone(),
        })
    }
}
impl ImagePublisher for Store {
    async fn store_output(
        &self,
        context: &PublicationContext,
        output: &PreparedImageOutput,
    ) -> Result<OutputCheckpoint, RuntimeError> {
        if output.display.is_some() {
            return Err(RuntimeError::UnsupportedCapability);
        }
        let audience = retained_audience(&context.job)?;
        let publication = context
            .stage
            .planned_publication
            .ok_or(RuntimeError::InvalidRecord)?;
        let asset = self.asset(
            output.master.clone(),
            audience,
            Some((publication.asset, publication.attempt)),
        )?;
        let recipe = context
            .job
            .recipe
            .as_ref()
            .ok_or(RuntimeError::InvalidRecord)?;
        let expected = FrozenOutput::decode(recipe)
            .map_err(RuntimeError::Publication)?
            .intent(AssetVariant::Master)
            .ok_or(RuntimeError::InvalidRecord)?;
        if !expected.accepts(asset.intent, asset.alpha) {
            return Err(RuntimeError::InvalidRecord);
        }
        let checkpoint = OutputCheckpoint::new(
            &context.dispatch,
            OutputObject {
                storage_key: format!("fixture/{}", context.dispatch.to_hex()),
                object_version: asset.receipt.version.as_str().to_owned(),
                byte_len: u64::try_from(asset.bytes.len())
                    .map_err(|_| RuntimeError::InvalidRecord)?,
                sha256: *asset.receipt.pin.digest.as_bytes(),
            },
            OutputImage {
                mime: asset.intent.mime().to_owned(),
                width_px: Some(asset.intent.width_px()),
                height_px: Some(asset.intent.height_px()),
                alpha: match asset.alpha {
                    ImageAlpha::Opaque => CheckpointAlpha::Opaque,
                    ImageAlpha::Transparent => CheckpointAlpha::Transparent,
                },
            },
            recipe_digest(recipe),
            context.usage_revision,
        )
        .map_err(|_| RuntimeError::InvalidRecord)?;
        let mut state = self.lock()?;
        if state.byte_len.saturating_add(asset.bytes.len()) > MAXIMUM_BYTES {
            return Err(RuntimeError::StorageUnavailable);
        }
        let entry = state
            .job_mut(&context.job.owner, context.job.job)
            .map_err(RuntimeError::Execution)?;
        entry
            .check_claim(context.claim, context.stage.claim_generation)
            .map_err(RuntimeError::Execution)?;
        if !entry.dispatch.as_ref().is_some_and(|dispatch| {
            dispatch.dispatch == context.dispatch && dispatch.state == DispatchState::Sent
        }) {
            return Err(RuntimeError::InvalidRecord);
        }
        if let Some(prior) = &entry.checkpoint {
            return if entry
                .staged
                .as_ref()
                .is_some_and(|staged| staged.bytes == output.master)
            {
                Ok(prior.clone())
            } else {
                Err(RuntimeError::InvalidRecord)
            };
        }
        entry.checkpoint = Some(checkpoint.clone());
        entry.staged = Some(asset);
        Ok(checkpoint)
    }
    async fn publish(&self, context: &PublicationContext) -> Result<PublishedImage, RuntimeError> {
        let mut state = self.lock()?;
        if let Some(result) = &state
            .job(&context.job.owner, context.job.job)
            .map_err(RuntimeError::Execution)?
            .result
        {
            return if result.dispatch == context.dispatch {
                Ok(result.clone())
            } else {
                Err(RuntimeError::InvalidRecord)
            };
        }
        if state.assets.len() >= MAXIMUM_RECORDS {
            return Err(RuntimeError::StorageUnavailable);
        }
        let entry = state
            .job_mut(&context.job.owner, context.job.job)
            .map_err(RuntimeError::Execution)?;
        entry
            .check_claim(context.claim, context.stage.claim_generation)
            .map_err(RuntimeError::Execution)?;
        if entry.job.state != JobState::RecipeFrozen
            || !entry.dispatch.as_ref().is_some_and(|dispatch| {
                dispatch.dispatch == context.dispatch && dispatch.state == DispatchState::Succeeded
            })
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let asset = entry.staged.take().ok_or(RuntimeError::InvalidRecord)?;
        let checkpoint = entry
            .checkpoint
            .as_ref()
            .ok_or(RuntimeError::InvalidRecord)?
            .clone();
        entry
            .bump(context.now_ms)
            .map_err(RuntimeError::Execution)?;
        let result = PublishedImage {
            job: entry.job.job,
            stage: entry.stage.stage,
            dispatch: context.dispatch,
            asset: asset.receipt.pin.asset,
            attempt: asset.receipt.attempt,
            recipe_digest: checkpoint.recipe_digest,
            job_revision: entry.job.revision,
            published_at_ms: context.now_ms,
            private_reconciliation: false,
            variants: vec![PublishedVariant {
                variant: AssetVariant::Master,
                write: asset.receipt.write,
                object_version: asset.receipt.version.clone(),
                byte_len: checkpoint.byte_len,
                sha256: checkpoint.sha256,
                intent: asset.intent,
                alpha: asset.alpha,
            }],
        };
        entry.result = Some(result.clone());
        state.byte_len += asset.bytes.len();
        state.assets.push(asset);
        Ok(result)
    }
}
impl ImageResults for Store {
    async fn read_result(
        &self,
        owner: &ImageOwner,
        job: &ImageJobId,
    ) -> Result<Option<PublishedImage>, PublicationError> {
        let state = self.lock().map_err(|_| PublicationError::Unavailable)?;
        Ok(state
            .jobs
            .iter()
            .find(|entry| &entry.job.owner == owner && entry.job.job == *job)
            .and_then(|entry| entry.result.clone()))
    }
}
impl ImageObservations for Store {
    async fn read_progress(
        &self,
        owner: &ImageOwner,
        job: ImageJobId,
        now_ms: i64,
    ) -> Result<Option<ImageProgress>, JournalError> {
        let state = self.lock().map_err(|_| JournalError::Unavailable)?;
        let Some(entry) = state
            .jobs
            .iter()
            .find(|entry| &entry.job.owner == owner && entry.job.job == job)
        else {
            return Ok(None);
        };
        let execution = if entry.job.state == JobState::Retired {
            ImageExecutionState::Withdrawn
        } else if entry.result.is_some() {
            ImageExecutionState::Ready
        } else if now_ms >= entry.job.absolute_deadline_ms {
            ImageExecutionState::Expired
        } else {
            match entry.dispatch.as_ref().map(|dispatch| dispatch.state) {
                None | Some(DispatchState::Prepared) => ImageExecutionState::Queued,
                Some(DispatchState::Declined) => return Err(JournalError::InvalidRecord),
                Some(DispatchState::Sent) => ImageExecutionState::Running,
                Some(DispatchState::Succeeded) => ImageExecutionState::Publishing,
                Some(DispatchState::Unknown) => ImageExecutionState::Unknown,
                Some(DispatchState::Released) => ImageExecutionState::Withdrawn,
                Some(DispatchState::Failed) => ImageExecutionState::Failed {
                    cause: entry
                        .dispatch
                        .as_ref()
                        .and_then(|dispatch| dispatch.cause)
                        .ok_or(JournalError::InvalidRecord)?,
                },
            }
        };
        Ok(Some(ImageProgress {
            revision: entry.job.revision,
            state: execution,
            assessment: None,
            failure: entry.dispatch.as_ref().and_then(|dispatch| dispatch.cause),
            operational_notice: entry.queued_notice,
        }))
    }
}
impl ImageDispatchAdmission for Store {
    async fn admit(&self, input: &DispatchAdmission<'_>) -> Result<DispatchMode, RuntimeError> {
        let state = self.lock()?;
        if input.job.owner != state.owner
            || input.stage.kind != StageKind::LocalTransform
            || input.quote.is_some()
            || input.config.accounting.mode != crate::AccountingMode::RecordOnly
        {
            return Err(RuntimeError::UnsupportedCapability);
        }
        for source in input.sources {
            if !state.assets.iter().any(|asset| asset.receipt == *source) {
                return Err(RuntimeError::SourceDenied);
            }
        }
        Ok(DispatchMode::Local)
    }
    async fn mark_sent(
        &self,
        input: &DispatchAdmission<'_>,
        mode: DispatchMode,
    ) -> Result<SendAuthorization, RuntimeError> {
        if !mode.is_local() {
            return Err(RuntimeError::UnsupportedCapability);
        }
        self.admit(input).await?;
        let mut state = self.lock()?;
        let entry = state
            .job_mut(&input.job.owner, input.job.job)
            .map_err(RuntimeError::Execution)?;
        if entry.dispatch.as_ref().map(|row| row.dispatch) != Some(input.dispatch) {
            return Err(RuntimeError::InvalidRecord);
        }
        entry
            .check_claim(input.claim, input.stage.claim_generation)
            .map_err(RuntimeError::Execution)?;
        if entry.sent {
            return Ok(SendAuthorization::AlreadySent);
        }
        entry.sent = true;
        Ok(SendAuthorization::New)
    }
    async fn outcome(
        &self,
        job: &crate::ImageJob,
        dispatch: DispatchId,
        mode: DispatchMode,
        _usable: Option<bool>,
    ) -> Result<(), RuntimeError> {
        if !mode.is_local() {
            return Err(RuntimeError::UnsupportedCapability);
        }
        self.lock()?
            .dispatch(&job.owner, dispatch)
            .map_err(RuntimeError::Execution)?;
        Ok(())
    }
}
impl ImageUsageJournal for Store {
    async fn record_sources(
        &self,
        scope: &ExecutionScope,
        sources: &[SourceReceipt],
    ) -> Result<(), RuntimeError> {
        let mut state = self.lock()?;
        let entry = state
            .dispatch(&scope.owner, scope.dispatch)
            .map_err(RuntimeError::Execution)?;
        if entry.job.job != scope.job || entry.stage.stage != scope.stage {
            return Err(RuntimeError::InvalidRecord);
        }
        if let Some((_, prior)) = state.source_snapshots.iter().find(|(key, _)| key == scope) {
            return if prior == sources {
                Ok(())
            } else {
                Err(RuntimeError::InvalidRecord)
            };
        }
        if state.source_snapshots.len() >= MAXIMUM_RECORDS {
            return Err(RuntimeError::StorageUnavailable);
        }
        state
            .source_snapshots
            .push((scope.clone(), sources.to_vec()));
        Ok(())
    }
    async fn read_sources(
        &self,
        scope: &ExecutionScope,
    ) -> Result<Option<Vec<SourceReceipt>>, RuntimeError> {
        Ok(self
            .lock()?
            .source_snapshots
            .iter()
            .find(|(key, _)| key == scope)
            .map(|(_, value)| value.clone()))
    }
    async fn pending_usage(
        &self,
        scope: &str,
        _limit: u32,
    ) -> Result<Vec<RuntimeUsage>, RuntimeError> {
        if scope != self.scope.as_str() {
            return Err(RuntimeError::InvalidRecord);
        }
        Ok(Vec::new())
    }
    async fn latest_usage(
        &self,
        _owner: &ImageOwner,
        _dispatch: DispatchId,
    ) -> Result<Option<RuntimeUsage>, RuntimeError> {
        Ok(None)
    }
    async fn record_usage(&self, _usage: &RuntimeUsage) -> Result<(), RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
    async fn acknowledge_projection(
        &self,
        _owner: &ImageOwner,
        _dispatch: DispatchId,
        _revision: UsageRevision,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
}

pub(super) struct LocalOnly;
impl ImageCredentials for LocalOnly {
    fn credential(
        &self,
        _backend: crate::BackendId,
        _account: &crate::AccountHandle,
    ) -> Result<String, RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
}
pub(super) struct FixtureObserver(
    pub(super) Option<std::sync::Arc<dyn crate::telemetry::ImageObserver>>,
);
impl ImageRuntimeTelemetry for FixtureObserver {
    fn observe(&self, event: crate::telemetry::ImageEvent) {
        if let Some(observer) = &self.0 {
            observer.observe(event);
        }
    }
    fn record(
        &self,
        _job: &crate::ImageJob,
        _stage: &StageView,
        _dispatch: Option<DispatchId>,
        _phase: RuntimePhase,
        _at_ms: i64,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}
impl ImageRuntimeCostPolicy for LocalOnly {
    fn route(
        &self,
        _job: &crate::ImageJob,
        _config: &crate::ImageConfig,
        _backend: crate::BackendId,
        _model: &str,
    ) -> Result<CostRoute, RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }
}
impl ImageCostProjection for LocalOnly {
    fn start_pinned<'a>(
        &'a self,
        _call_id: &'a str,
        _route: &'a CostRoute,
    ) -> CostFuture<'a, Result<PinnedStart, CostError>> {
        Box::pin(async { Err(CostError::BookNotOpen) })
    }
    fn correct<'a>(
        &'a self,
        _call_id: &'a str,
        _report: &'a UsageReport<'a>,
    ) -> CostFuture<'a, Result<PinnedCorrection, CostError>> {
        Box::pin(async { Err(CostError::BookNotOpen) })
    }
    fn record_unsent<'a>(
        &'a self,
        _call_id: &'a str,
    ) -> CostFuture<'a, Result<PinnedUnsent, CostError>> {
        Box::pin(async { Err(CostError::BookNotOpen) })
    }
    fn is_durable<'a>(
        &'a self,
        _call_id: &'a str,
        _revision: Option<UsageRevision>,
    ) -> CostFuture<'a, Result<bool, CostError>> {
        Box::pin(async { Err(CostError::BookNotOpen) })
    }
}
