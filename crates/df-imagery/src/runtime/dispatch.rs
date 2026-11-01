//! Private provider selection and immutable supplier checkpoint recovery.

use super::ports::*;
use crate::backends::api::{
    self, ApiError, ImageBackground, ImageEdit, ImageEditModel, ImageFormat, ImageModeration,
    ImageQuality, OperationalBounds, ReferenceImage,
};
use crate::backends::api::{
    bfl::{BflConfig, BflExecutor, BflPortraitCheckpoint},
    fal::{FalAlphaCheckpoint, FalConfig, FalExecutor, FalGenerationCheckpoint},
    openai::{OpenAiConfig, OpenAiExecutor},
};
use crate::backends::codex_cli;
use crate::execution::{ImageExecutionJournal, SupplierCheckpoint, SupplierReport};
use crate::intent_codec::DecodedIntent;
use crate::{
    AlignAnchor, AlphaHandling, AssetDigest, BackendId, GenerationQuality, ImageConfig,
    ImageOperation, ImageTransform, Interpolation, OutputFormat, ResizeMode, ResizeSpec, SourceRef,
    TransformSource, TransformSpec,
};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

pub(super) struct ExecutedImage {
    pub bytes: Vec<u8>,
    pub units: Vec<crate::cost::CostUnits>,
    pub evidence: crate::cost::UsageEvidence,
}

pub(super) struct DispatchFailure {
    pub error: RuntimeError,
    pub usage: Option<ObservedDispatchUsage>,
    pub api: Option<api::ApiFailure>,
}

pub(super) struct ObservedDispatchUsage {
    pub units: Vec<crate::cost::CostUnits>,
    pub evidence: crate::cost::UsageEvidence,
    pub raw: Vec<u8>,
}

impl From<RuntimeError> for DispatchFailure {
    fn from(error: RuntimeError) -> Self {
        Self {
            error,
            usage: None,
            api: None,
        }
    }
}

pub(super) fn backend(config: &ImageConfig, intent: &DecodedIntent) -> BackendId {
    config
        .purpose(intent.purpose.purpose)
        .backend_override
        .unwrap_or(config.default_backend)
}

pub(super) fn model(config: &ImageConfig, intent: &DecodedIntent) -> Result<String, RuntimeError> {
    let selected = backend(config, intent);
    let gate = config.backend(selected);
    let configured = config
        .purpose(intent.purpose.purpose)
        .model_override
        .as_ref()
        .or(match &intent.operation {
            ImageOperation::Generate(spec) if spec.references.is_empty() => {
                gate.generation_model.as_ref()
            }
            _ => gate.edit_model.as_ref(),
        })
        .or(gate.model.as_ref());
    if selected == BackendId::CodexCli {
        if config
            .purpose(intent.purpose.purpose)
            .model_override
            .is_some()
            || [
                &gate.model,
                &gate.generation_model,
                &gate.edit_model,
                &gate.alpha_model,
            ]
            .iter()
            .any(|model| model.is_some())
        {
            return Err(RuntimeError::UnsupportedCapability);
        }
        return Ok("codex-image-profile".to_owned());
    }
    Ok(configured
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| match selected {
            BackendId::OpenAi => api::GPT_25_FLARE_MODEL_ID.to_owned(),
            BackendId::Fal => match &intent.operation {
                ImageOperation::Generate(spec) if spec.references.is_empty() => {
                    api::fal::FLUX_ULTRA.to_owned()
                }
                _ => api::fal::FLUX_KONTEXT_MULTI.to_owned(),
            },
            BackendId::Bfl => api::bfl::BFL_FLUX3_IMAGE.to_owned(),
            BackendId::CodexCli => "codex-image-profile".to_owned(),
        }))
}

pub(super) fn account(config: &ImageConfig, intent: &DecodedIntent) -> crate::AccountHandle {
    config
        .purpose(intent.purpose.purpose)
        .account_override
        .as_ref()
        .unwrap_or(&config.backend(backend(config, intent)).account)
        .clone()
}
pub(super) fn moderation(config: &ImageConfig, intent: &DecodedIntent) -> ImageModeration {
    match config.purpose(intent.purpose.purpose).moderation_override {
        Some(crate::ProviderModeration::Low) => ImageModeration::Low,
        Some(crate::ProviderModeration::Auto) => ImageModeration::Auto,
        None if intent
            .operation
            .pipeline()
            .is_some_and(|pipeline| pipeline.rating == crate::ImageContentRating::Mature) =>
        {
            ImageModeration::Low
        }
        None => ImageModeration::Auto,
    }
}

pub(super) fn quality(config: &ImageConfig, intent: &DecodedIntent) -> Option<GenerationQuality> {
    config
        .purpose(intent.purpose.purpose)
        .quality_override
        .or(config.quality.provider_quality)
}

pub(super) fn preflight(config: &ImageConfig, intent: &DecodedIntent) -> Result<(), RuntimeError> {
    if !config.purpose(intent.purpose.purpose).enabled {
        return Err(RuntimeError::Disabled);
    }
    if matches!(intent.operation, ImageOperation::Transform(_)) {
        return if config.transforms.enabled {
            Ok(())
        } else {
            Err(RuntimeError::Disabled)
        };
    }
    let selected = backend(config, intent);
    if !config.backend(selected).enabled {
        return Err(RuntimeError::Disabled);
    }
    if matches!(
        quality(config, intent),
        Some(GenerationQuality::LegacyNumeric(_) | GenerationQuality::Auto)
    ) {
        return Err(RuntimeError::UnsupportedCapability);
    }
    if selected == BackendId::CodexCli {
        if quality(config, intent).is_some() {
            return Err(RuntimeError::UnsupportedCapability);
        }
        if !std::path::Path::new(config.codex.executable.as_str()).is_absolute()
            || !std::path::Path::new(config.codex.workdir.as_str()).is_absolute()
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
    }
    let resolved_model = model(config, intent)?;
    if matches!(selected, BackendId::Fal | BackendId::Bfl) {
        api::check_model_slug(&resolved_model).map_err(api_error)?;
    } else if selected == BackendId::OpenAi {
        api::check_model_id(&resolved_model).map_err(api_error)?;
    }

    let dimensions = intent
        .output
        .generated()
        .ok_or(RuntimeError::InvalidRecord)?;
    if selected != BackendId::CodexCli {
        let width = dimensions.width_px();
        let height = dimensions.height_px();
        if !width.is_multiple_of(16)
            || !height.is_multiple_of(16)
            || width.max(height) > 3840
            || u64::from(width.max(height)) > u64::from(width.min(height)) * 3
            || !(655_360..=4_194_304).contains(&dimensions.pixel_count())
            || references(&intent.operation).len() > 4
        {
            return Err(RuntimeError::UnsupportedCapability);
        }
        if selected == BackendId::Bfl
            && (width != 1024
                || height != 1536
                || !references(&intent.operation).is_empty()
                || matches!(
                    intent.output.delivered().alpha(),
                    AlphaHandling::PreserveAlpha
                ))
        {
            return Err(RuntimeError::UnsupportedCapability);
        }
    }
    Ok(())
}

pub(super) struct PinnedCredentials {
    backend: BackendId,
    account: crate::AccountHandle,
    secret: Option<String>,
    quota: Option<std::sync::Arc<super::QuotaRegistry>>,
    quota_identity: Option<[u8; 32]>,
}
impl ImageCredentials for PinnedCredentials {
    fn quota_registry(&self) -> Option<std::sync::Arc<super::QuotaRegistry>> {
        self.quota.clone()
    }
    fn quota_identity(
        &self,
        backend: BackendId,
        account: &crate::AccountHandle,
    ) -> Option<[u8; 32]> {
        if backend == self.backend && *account == self.account {
            self.quota_identity
        } else {
            None
        }
    }
    fn credential(
        &self,
        backend: BackendId,
        account: &crate::AccountHandle,
    ) -> Result<String, RuntimeError> {
        if backend != self.backend || *account != self.account {
            return Err(RuntimeError::InvalidRecord);
        }
        self.secret
            .clone()
            .ok_or(RuntimeError::UnsupportedCapability)
    }
}

pub(super) fn pin_credentials<K: ImageCredentials>(
    config: &ImageConfig,
    intent: &DecodedIntent,
    credentials: &K,
) -> Result<PinnedCredentials, RuntimeError> {
    let backend = backend(config, intent);
    let gate = config.backend(backend);
    let account = account(config, intent);
    let secret = if matches!(intent.operation, ImageOperation::Transform(_))
        || backend == BackendId::CodexCli
    {
        None
    } else {
        let secret = credentials.credential(backend, &account)?;
        api::check_credential(&secret).map_err(api_error)?;
        if let Some(endpoint) = &gate.endpoint {
            api::check_endpoint_url(endpoint.as_str()).map_err(api_error)?;
        }
        Some(secret)
    };
    let quota_identity = credentials.quota_identity(backend, &account);
    Ok(PinnedCredentials {
        backend,
        account,
        secret,
        quota: credentials.quota_registry(),
        quota_identity,
    })
}

pub(super) fn pin_backend_credentials(
    backend: BackendId,
    account: &crate::AccountHandle,
    credentials: &impl ImageCredentials,
) -> Result<PinnedCredentials, RuntimeError> {
    let secret = credentials.credential(backend, account)?;
    api::check_credential(&secret).map_err(api_error)?;
    Ok(PinnedCredentials {
        backend,
        account: account.clone(),
        secret: Some(secret),
        quota: credentials.quota_registry(),
        quota_identity: credentials.quota_identity(backend, account),
    })
}

pub(super) fn admit_image_quota(
    config: &ImageConfig,
    intent: &DecodedIntent,
    policy: crate::config::QuotaSettings,
    credentials: &PinnedCredentials,
) -> Result<Option<super::quota::QuotaAdmission>, RuntimeError> {
    let Some(secret) = credentials.secret.as_deref() else {
        return Ok(None);
    };
    let endpoint = config
        .backend(credentials.backend)
        .endpoint
        .as_ref()
        .map_or(
            match credentials.backend {
                BackendId::OpenAi => api::openai::LIVE_ENDPOINT,
                BackendId::Fal => api::fal::QUEUE_ORIGIN,
                BackendId::Bfl => api::bfl::LIVE_BFL_ORIGIN,
                BackendId::CodexCli => return Ok(None),
            },
            |endpoint| endpoint.as_str(),
        );
    if matches!(intent.operation, ImageOperation::Transform(_)) {
        return Ok(None);
    }
    super::quota_gate::admit(
        credentials,
        policy,
        endpoint,
        secret,
        credentials.quota_identity.as_ref(),
    )
}

pub(super) struct ResolvedInputs {
    pub images: Vec<(SourceRef, Vec<u8>)>,
    pub receipts: Vec<SourceReceipt>,
    pub analysis: Option<crate::ImageAnalysisObservation>,
}

pub(super) async fn resolve_inputs<S: ImageSources>(
    intent: &DecodedIntent,
    sources: &S,
) -> Result<ResolvedInputs, RuntimeError> {
    let references = references(&intent.operation);
    let mut images = Vec::with_capacity(references.len());
    let mut receipts = Vec::with_capacity(references.len());
    for pin in &references {
        let source = sources.read_source(&intent.owner, pin).await?;
        if source.receipt.pin != **pin
            || <[u8; 32]>::from(Sha256::digest(&source.bytes)) != *pin.digest.as_bytes()
        {
            return Err(RuntimeError::SourceDenied);
        }
        images.push(((*pin).clone(), source.bytes));
        receipts.push(source.receipt);
    }
    let analysis = if let ImageOperation::Transform(crate::TransformSpec {
        source:
            TransformSource::SegmentationMask {
                analyzed,
                segmentation,
                ..
            },
        ..
    }) = &intent.operation
    {
        if segmentation.owner != intent.owner {
            return Err(RuntimeError::SourceDenied);
        }
        let observation = sources.read_analysis(&intent.owner, segmentation).await?;
        let crate::ImageAnalysisResult::Segmentation { source, .. } = &observation.result else {
            return Err(RuntimeError::SourceDenied);
        };
        if source != analyzed {
            return Err(RuntimeError::SourceDenied);
        }
        Some(observation)
    } else {
        None
    };
    Ok(ResolvedInputs {
        images,
        receipts,
        analysis,
    })
}

pub(super) struct DispatchInput<'a> {
    pub context: &'a PublicationContext,
    pub intent: &'a DecodedIntent,
    pub config: &'a ImageConfig,
    pub timeout: Duration,
    pub resume: bool,
    pub telemetry: &'a dyn ImageRuntimeTelemetry,
    pub event_context: crate::telemetry::EventContext,
}

pub(super) async fn execute<E: ImageExecutionJournal, K: ImageCredentials>(
    input: DispatchInput<'_>,
    journal: &E,
    retained: ResolvedInputs,
    credentials: &K,
) -> Result<ExecutedImage, DispatchFailure> {
    if !matches!(input.intent.operation, ImageOperation::Transform(_))
        && backend(input.config, input.intent) == BackendId::CodexCli
    {
        if input.resume {
            return Err(RuntimeError::ProviderUnknown.into());
        }
        return codex(
            input.context,
            input.intent,
            input.config,
            retained.images,
            input.timeout,
            input.telemetry,
            input.event_context.clone(),
        )
        .await;
    }
    execute_api(input, journal, retained, credentials).await
}

async fn execute_api<E: ImageExecutionJournal, K: ImageCredentials>(
    input: DispatchInput<'_>,
    journal: &E,
    retained: ResolvedInputs,
    credentials: &K,
) -> Result<ExecutedImage, DispatchFailure> {
    let analysis = retained.analysis;
    let retained = retained.images;
    let DispatchInput {
        context,
        intent,
        config,
        timeout,
        resume,
        telemetry,
        event_context,
    } = input;
    let started = Instant::now();
    let remaining = || timeout.saturating_sub(started.elapsed());
    if let ImageOperation::Transform(spec) = &intent.operation {
        let spec = spec.clone();
        let output = intent.output.delivered();
        let limits = config.request_limits;
        let transforms = config.transforms;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(RuntimeError::Expired)?;
        let result = tokio::task::spawn_blocking(move || {
            let resolve = |source: &SourceRef| {
                retained
                    .iter()
                    .find(|(pin, _)| pin == source)
                    .map(|(_, bytes)| bytes.clone())
                    .ok_or(crate::raster::RasterError::ResolveFailed)
            };
            let cancel = || {
                if Instant::now() < deadline {
                    Ok(())
                } else {
                    Err(crate::raster::RasterError::Cancelled)
                }
            };
            if let Some(analysis) = analysis {
                crate::raster::execute_segmentation_mask(
                    &spec,
                    &output,
                    &limits,
                    &transforms,
                    &analysis.result,
                    &resolve,
                    &cancel,
                )
            } else {
                crate::raster::execute_transform(
                    &spec,
                    &output,
                    &limits,
                    &transforms,
                    &resolve,
                    &cancel,
                )
            }
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?
        .map_err(|_| RuntimeError::UnsupportedCapability)?;
        return Ok(ExecutedImage {
            bytes: result.bytes,
            units: Vec::new(),
            evidence: crate::cost::UsageEvidence::Estimated,
        });
    }
    let selected = backend(config, intent);
    if selected == BackendId::CodexCli {
        if resume {
            return Err(RuntimeError::ProviderUnknown.into());
        }
        return Err(RuntimeError::InvalidRecord.into());
    }
    let credential = credentials.credential(selected, &account(config, intent))?;
    let api_references = retained
        .iter()
        .map(|(_, bytes)| {
            let format = match image::guess_format(bytes) {
                Ok(image::ImageFormat::Png) => ImageFormat::Png,
                Ok(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
                Ok(image::ImageFormat::WebP) => ImageFormat::Webp,
                _ => return Err(RuntimeError::SourceDenied.into()),
            };
            Ok(ReferenceImage { format, bytes })
        })
        .collect::<Result<Vec<_>, DispatchFailure>>()?;
    let prompt = prompt(intent)?;
    let dimensions = intent
        .output
        .generated()
        .ok_or(RuntimeError::InvalidRecord)?;
    let output = match intent.output.delivered().encoding().format() {
        OutputFormat::Png => ImageFormat::Png,
        OutputFormat::Jpeg => ImageFormat::Jpeg,
        OutputFormat::WebP => ImageFormat::Webp,
    };
    let edit = ImageEdit {
        model: if selected == BackendId::OpenAi {
            ImageEditModel::parse_model_id(&model(config, intent)?).map_err(api_error)?
        } else {
            ImageEditModel::Custom(model(config, intent)?)
        },
        prompt: prompt.text(),
        references: &api_references,
        width: dimensions.width_px(),
        height: dimensions.height_px(),
        quality: match quality(config, intent) {
            Some(GenerationQuality::Low) => ImageQuality::Low,
            Some(GenerationQuality::High) => ImageQuality::High,
            _ => ImageQuality::Medium,
        },
        background: if matches!(
            intent.output.delivered().alpha(),
            AlphaHandling::PreserveAlpha
        ) {
            ImageBackground::Transparent
        } else {
            ImageBackground::Opaque
        },
        output,
        moderation: moderation(config, intent),
    };
    let gate = config.backend(selected);
    let bounds = OperationalBounds::compatibility_defaults();
    let units;
    let evidence;
    let bytes = match selected {
        BackendId::OpenAi => {
            if resume {
                return Err(RuntimeError::ProviderUnknown.into());
            }
            let endpoint = if let Some(base) = &gate.endpoint {
                let base = base.as_str().trim_end_matches('/');
                OpenAiConfig::operator(
                    &format!("{base}/v1/images/edits"),
                    &format!("{base}/v1/images/generations"),
                )
                .map_err(api_error)?
            } else {
                OpenAiConfig::live()
            };
            let image = OpenAiExecutor::new(endpoint, &credential, bounds, timeout)
                .map_err(api_error)?
                .edit_observed(&edit, timeout)
                .await
                .map_err(openai_failure)?;
            evidence = if image.usage == api::ImageUsage::default() {
                crate::cost::UsageEvidence::Estimated
            } else {
                crate::cost::UsageEvidence::Reported
            };
            units = if image.usage == api::ImageUsage::default() {
                Vec::new()
            } else {
                vec![
                    unit("text_input", image.usage.text_input_tokens),
                    unit("image_input", image.usage.image_input_tokens),
                    unit("image_output", image.usage.output_tokens),
                ]
            };
            image.bytes
        }
        BackendId::Fal => {
            let endpoint = if let Some(base) = &gate.endpoint {
                FalConfig::operator(base.as_str()).map_err(api_error)?
            } else {
                FalConfig::live()
            };
            let endpoint = endpoint
                .with_models(
                    &model(config, intent)?,
                    &model(config, intent)?,
                    gate.alpha_model
                        .as_ref()
                        .map_or(api::fal::BACKGROUND_REMOVAL, |name| name.as_str()),
                )
                .map_err(api_error)?;
            let executor =
                FalExecutor::new(endpoint, &credential, bounds, timeout).map_err(api_error)?;
            let checkpoint = if resume {
                let stored = journal
                    .read_supplier_checkpoint(
                        &context.job.owner,
                        &context.job.job,
                        &context.stage.stage,
                        &context.dispatch,
                    )
                    .await
                    .map_err(RuntimeError::Execution)?
                    .ok_or(RuntimeError::ProviderUnknown)?;
                FalGenerationCheckpoint::decode(
                    std::str::from_utf8(&stored.checkpoint_bytes)
                        .map_err(|_| RuntimeError::InvalidRecord)?,
                )
                .map_err(api_error)?
            } else {
                let checkpoint = executor
                    .submit_generation_observed(&edit, remaining())
                    .await
                    .map_err(api_failure)?;
                persist_supplier(
                    context,
                    journal,
                    &checkpoint.origin,
                    &checkpoint.endpoint,
                    &checkpoint.request_id,
                    checkpoint.encode(),
                )
                .await?;
                telemetry.observe(crate::telemetry::ImageEvent::new(
                    crate::telemetry::EventKind::SupplierCheckpointed,
                    event_context.clone(),
                ));
                checkpoint
            };
            let (bytes, _, _) = executor
                .resume_generation_observed(&checkpoint, remaining())
                .await
                .map_err(api_failure)?;
            units = vec![unit("images", 1)];
            evidence = crate::cost::UsageEvidence::Estimated;
            bytes
        }
        BackendId::Bfl => {
            let endpoint = if let Some(base) = &gate.endpoint {
                BflConfig::operator(base.as_str()).map_err(api_error)?
            } else {
                BflConfig::live()
            };
            let endpoint = endpoint
                .with_route(&model(config, intent)?)
                .map_err(api_error)?;
            let executor =
                BflExecutor::new(endpoint, &credential, bounds, timeout).map_err(api_error)?;
            let checkpoint = if resume {
                let stored = journal
                    .read_supplier_checkpoint(
                        &context.job.owner,
                        &context.job.job,
                        &context.stage.stage,
                        &context.dispatch,
                    )
                    .await
                    .map_err(RuntimeError::Execution)?
                    .ok_or(RuntimeError::ProviderUnknown)?;
                BflPortraitCheckpoint::decode(
                    std::str::from_utf8(&stored.checkpoint_bytes)
                        .map_err(|_| RuntimeError::InvalidRecord)?,
                )
                .map_err(api_error)?
            } else {
                let checkpoint = executor
                    .submit_portrait_observed(&edit, remaining())
                    .await
                    .map_err(api_failure)?;
                persist_supplier(
                    context,
                    journal,
                    &checkpoint.origin,
                    &checkpoint.model_route,
                    &checkpoint.request_id,
                    checkpoint.encode(),
                )
                .await?;
                telemetry.observe(crate::telemetry::ImageEvent::new(
                    crate::telemetry::EventKind::SupplierCheckpointed,
                    event_context.clone(),
                ));
                checkpoint
            };
            let image = executor
                .resume_portrait_diagnosed(&checkpoint, remaining())
                .await
                .map_err(|failure| {
                    let mut event = crate::telemetry::ImageEvent::new(
                        crate::telemetry::EventKind::Failed,
                        event_context.clone(),
                    )
                    .failed(crate::telemetry::FailureKind::ProviderUnknown);
                    event.measurements.provider_step = Some(match failure.stage {
                        "submit" => crate::telemetry::ProviderStep::Submit,
                        "poll" => crate::telemetry::ProviderStep::Poll,
                        "delivery" => crate::telemetry::ProviderStep::Delivery,
                        "download" => crate::telemetry::ProviderStep::Download,
                        "validate" => crate::telemetry::ProviderStep::Validate,
                        _ => crate::telemetry::ProviderStep::Unknown,
                    });
                    event.measurements.provider_status = failure.http_status;
                    event.measurements.delivery_identified = Some(failure.delivery_host.is_some());
                    event.measurements.admission_refused = Some(failure.admission_reason.is_some());
                    telemetry.observe(event);
                    api_failure(failure.observation)
                })?;
            let credits = image
                .credits
                .as_deref()
                .and_then(|text| text.parse::<u64>().ok());
            units = credits.map_or_else(Vec::new, |credits| vec![unit("credits", credits)]);
            evidence = if credits.is_some() {
                crate::cost::UsageEvidence::Reported
            } else {
                crate::cost::UsageEvidence::Estimated
            };
            image.image.bytes
        }
        BackendId::CodexCli => return Err(RuntimeError::InvalidRecord.into()),
    };
    Ok(ExecutedImage {
        bytes,
        units,
        evidence,
    })
}

pub(super) async fn execute_alpha<E: ImageExecutionJournal, K: ImageCredentials>(
    input: DispatchInput<'_>,
    journal: &E,
    source: Option<Vec<u8>>,
    credentials: &K,
) -> Result<ExecutedImage, DispatchFailure> {
    execute_alpha_inner(input, journal, source, credentials).await
}

async fn execute_alpha_inner<E: ImageExecutionJournal, K: ImageCredentials>(
    input: DispatchInput<'_>,
    journal: &E,
    source: Option<Vec<u8>>,
    credentials: &K,
) -> Result<ExecutedImage, DispatchFailure> {
    let config = input.config;
    let context = input.context;
    let gate = config.backend(BackendId::Fal);
    if !gate.enabled {
        return Err(RuntimeError::Disabled.into());
    }
    let credential = credentials.credential(BackendId::Fal, &gate.account)?;
    let endpoint = if let Some(endpoint) = &gate.endpoint {
        FalConfig::operator(endpoint.as_str()).map_err(api_error)?
    } else {
        FalConfig::live()
    };
    let endpoint = endpoint
        .with_models(
            api::fal::FLUX_ULTRA,
            api::fal::FLUX_KONTEXT_MULTI,
            gate.alpha_model
                .as_ref()
                .map_or(api::fal::BACKGROUND_REMOVAL, |name| name.as_str()),
        )
        .map_err(api_error)?;
    let executor = FalExecutor::new(
        endpoint,
        &credential,
        OperationalBounds::compatibility_defaults(),
        input.timeout,
    )
    .map_err(api_error)?;
    let started = Instant::now();
    let remaining = || input.timeout.saturating_sub(started.elapsed());
    let checkpoint = if input.resume {
        let stored = journal
            .read_supplier_checkpoint(
                &context.job.owner,
                &context.job.job,
                &context.stage.stage,
                &context.dispatch,
            )
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::ProviderUnknown)?;
        FalAlphaCheckpoint::decode(
            std::str::from_utf8(&stored.checkpoint_bytes)
                .map_err(|_| RuntimeError::InvalidRecord)?,
        )
        .map_err(api_error)?
    } else {
        let source = source.as_deref().ok_or(RuntimeError::InvalidRecord)?;
        let format = match image::guess_format(source) {
            Ok(image::ImageFormat::Png) => ImageFormat::Png,
            Ok(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
            Ok(image::ImageFormat::WebP) => ImageFormat::Webp,
            _ => return Err(RuntimeError::InvalidRecord.into()),
        };
        let checkpoint = executor
            .submit_alpha_removal_observed(source, format, remaining())
            .await
            .map_err(api_failure)?;
        persist_supplier(
            context,
            journal,
            &checkpoint.origin,
            &checkpoint.endpoint,
            &checkpoint.request_id,
            checkpoint.encode(),
        )
        .await?;
        input.telemetry.observe(crate::telemetry::ImageEvent::new(
            crate::telemetry::EventKind::SupplierCheckpointed,
            input.event_context.clone(),
        ));
        checkpoint
    };
    let (bytes, _, _receipt) = executor
        .resume_alpha_removal_observed(&checkpoint, remaining())
        .await
        .map_err(api_failure)?;
    Ok(ExecutedImage {
        bytes,
        units: vec![unit("images", 1)],
        evidence: crate::cost::UsageEvidence::Estimated,
    })
}

async fn persist_supplier<E: ImageExecutionJournal>(
    context: &PublicationContext,
    journal: &E,
    origin: &str,
    model: &str,
    request: &str,
    encoded: String,
) -> Result<(), RuntimeError> {
    let checkpoint = SupplierCheckpoint::new(
        crate::execution::ExecutionScope {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage: context.stage.stage,
            dispatch: context.dispatch,
        },
        crate::execution::SupplierResume {
            checkpoint_version: 1,
            origin: origin.to_owned(),
            model: model.to_owned(),
            checkpoint_bytes: encoded.into_bytes(),
        },
        context.now_ms,
    )
    .map_err(|_| RuntimeError::InvalidRecord)?;
    journal
        .record_supplier_checkpoint(&checkpoint)
        .await
        .map_err(RuntimeError::Execution)?;
    let report = SupplierReport::new(
        context.job.owner.clone(),
        context.job.job,
        context.stage.stage,
        context.dispatch,
        request.to_owned(),
        context.now_ms,
    )
    .map_err(|_| RuntimeError::InvalidRecord)?;
    journal
        .report_supplier(&report)
        .await
        .map_err(RuntimeError::Execution)?;
    Ok(())
}

fn unit(category: &str, quantity: u64) -> crate::cost::CostUnits {
    crate::cost::CostUnits {
        category: category.to_owned(),
        quantity,
    }
}

fn api_failure(failure: api::ApiFailure) -> DispatchFailure {
    DispatchFailure {
        error: api_error(failure.error),
        usage: None,
        api: Some(failure),
    }
}

fn openai_failure(failure: api::ApiFailure) -> DispatchFailure {
    let mut observed = retain_api_failure(&failure);
    if let Some(usage) = failure
        .response
        .as_ref()
        .and_then(|response| api::openai::reported_image_usage(&response.body))
    {
        observed.evidence = crate::cost::UsageEvidence::Reported;
        observed.units = vec![
            unit("text_input", usage.text_input_tokens),
            unit("image_input", usage.image_input_tokens),
            unit("image_output", usage.output_tokens),
        ];
    }
    DispatchFailure {
        error: api_error(failure.error),
        usage: Some(observed),
        api: Some(failure),
    }
}

pub(super) fn retain_api_failure(failure: &api::ApiFailure) -> ObservedDispatchUsage {
    let contract = match failure.submission {
        api::SubmissionEvidence::RejectedSubmit(api::DeclineContract::OpenAiImages429V1) => {
            Some("openai-images-429-v1")
        }
        api::SubmissionEvidence::RejectedSubmit(api::DeclineContract::FalQueue429V1) => {
            Some("fal-queue-429-v1")
        }
        api::SubmissionEvidence::RejectedSubmit(api::DeclineContract::BflImages429V1) => {
            Some("bfl-images-429-v1")
        }
        _ => None,
    };
    let raw = serde_json::json!({
        "schema_revision":2,"kind":"image-api-failure",
        "exchange":match failure.exchange {api::ApiExchange::Submit=>"submit",api::ApiExchange::Poll=>"poll",api::ApiExchange::Result=>"result",api::ApiExchange::Delivery=>"delivery"},
        "submission":match failure.submission {api::SubmissionEvidence::BeforeTransport=>"before_transport",api::SubmissionEvidence::RejectedSubmit(_)=>"rejected_submit",api::SubmissionEvidence::AcceptedJob=>"accepted_job",api::SubmissionEvidence::Unknown=>"unknown"},
        "decline_contract":contract,
        "status":failure.response.as_ref().map(|response|response.status),
        "body":failure.response.as_ref().map(|response|&response.body),
        "transport_request_id":failure.response.as_ref().and_then(|response|response.transport_request_id.as_ref()).map(|id|id.as_str()),
        "retry_after_values":failure.response.as_ref().map(|response|response.retry_after.raw_values()),
    }).to_string().into_bytes();
    ObservedDispatchUsage {
        raw,
        units: Vec::new(),
        evidence: crate::cost::UsageEvidence::Unknown,
    }
}

fn api_error(error: ApiError) -> RuntimeError {
    match error {
        ApiError::QuotaExhausted => RuntimeError::SupplierQuotaExhausted,
        ApiError::RateLimited => RuntimeError::SupplierRateLimited,
        ApiError::Refused | ApiError::Unauthorized => RuntimeError::ProviderRefused,
        ApiError::InvalidRequest => RuntimeError::UnsupportedCapability,
        ApiError::Output => {
            RuntimeError::SupplierFailure(crate::execution::FailureCause::InvalidOutput)
        }
        ApiError::Provider(_) => {
            RuntimeError::SupplierFailure(crate::execution::FailureCause::Provider)
        }
        ApiError::Transport(api::NativeHttpRefusal::Deadline) => {
            RuntimeError::SupplierFailure(crate::execution::FailureCause::DeadlineExceeded)
        }
        ApiError::Transport(_) => {
            RuntimeError::SupplierFailure(crate::execution::FailureCause::TransportLost)
        }
    }
}

fn prompt(intent: &DecodedIntent) -> Result<&crate::TextPrompt, RuntimeError> {
    match &intent.operation {
        ImageOperation::Generate(spec) => Ok(&spec.prompt),
        ImageOperation::Edit(spec) => Ok(&spec.prompt),
        _ => Err(RuntimeError::InvalidRecord),
    }
}

pub(super) fn references(operation: &ImageOperation) -> Vec<&SourceRef> {
    let mut pins = Vec::new();
    match operation {
        ImageOperation::Generate(spec) => pins.extend(&spec.references),
        ImageOperation::Edit(spec) => {
            pins.push(&spec.base);
            pins.extend(&spec.references);
        }
        ImageOperation::Transform(spec) => {
            match &spec.source {
                TransformSource::Image(pin) => pins.push(pin),
                TransformSource::SegmentationMask { analyzed, .. } => pins.push(analyzed),
                TransformSource::Coverage(coverage) => {
                    pins.push(&coverage.coverage);
                    pins.push(&coverage.analyzed);
                }
                TransformSource::Canvas(_) => {}
            }
            for step in &spec.steps {
                match step {
                    ImageTransform::Mask(mask) => match mask.source() {
                        crate::MaskSource::Asset(pin) => pins.push(pin),
                        crate::MaskSource::Coverage(coverage) => {
                            pins.push(&coverage.coverage);
                            pins.push(&coverage.analyzed);
                        }
                    },
                    ImageTransform::Composite(layers) => {
                        pins.extend(layers.iter().map(|layer| layer.source()))
                    }
                    _ => {}
                }
            }
        }
    }
    pins
}

pub(super) async fn normalize_output(
    context: &PublicationContext,
    intent: &DecodedIntent,
    config: &ImageConfig,
    image: ExecutedImage,
    timeout: Duration,
) -> Result<ExecutedImage, RuntimeError> {
    let ExecutedImage {
        bytes,
        units,
        evidence,
    } = image;
    let pin = SourceRef {
        asset: crate::AssetId::from_bytes(&super::planned_publication(&context.stage)?.asset)
            .map_err(|_| RuntimeError::InvalidRecord)?,
        digest: AssetDigest::from_bytes(Sha256::digest(&bytes).into()),
        audience: intent.audience.clone(),
    };
    let dimensions = intent.output.delivered().dimensions();
    let resize = ResizeSpec::new(
        dimensions.width_px(),
        dimensions.height_px(),
        ResizeMode::Stretch,
        AlignAnchor::center(),
        Interpolation::Lanczos3,
        true,
        None,
    )
    .map_err(|_| RuntimeError::InvalidRecord)?;
    let spec = TransformSpec {
        source: TransformSource::Image(pin),
        steps: vec![ImageTransform::Resize(resize)],
    };
    let output = intent.output.delivered();
    let limits = config.request_limits;
    let transforms = config.transforms;
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(RuntimeError::Expired)?;
    let output = tokio::task::spawn_blocking(move || {
        crate::raster::execute_transform(
            &spec,
            &output,
            &limits,
            &transforms,
            &|_| Ok(bytes.clone()),
            &|| {
                if Instant::now() < deadline {
                    Ok(())
                } else {
                    Err(crate::raster::RasterError::Cancelled)
                }
            },
        )
    })
    .await
    .map_err(|_| RuntimeError::WorkerFailed)?
    .map_err(|_| RuntimeError::UnsupportedCapability)?;
    Ok(ExecutedImage {
        bytes: output.bytes,
        units,
        evidence,
    })
}

async fn codex(
    context: &PublicationContext,
    intent: &DecodedIntent,
    config: &ImageConfig,
    references: Vec<(SourceRef, Vec<u8>)>,
    timeout: Duration,
    telemetry: &dyn ImageRuntimeTelemetry,
    event_context: crate::telemetry::EventContext,
) -> Result<ExecutedImage, DispatchFailure> {
    let directory =
        std::path::PathBuf::from(config.codex.workdir.as_str()).join(context.dispatch.to_hex());
    let scratch = directory.join("scratch");
    let inputs = directory.join("inputs");
    let outputs = directory.join("outputs");
    for path in [&scratch, &inputs, &outputs] {
        tokio::fs::create_dir_all(path)
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
    }
    let mut staged = Vec::with_capacity(references.len());
    for (index, (pin, bytes)) in references.into_iter().enumerate() {
        let path = inputs.join(format!("reference-{index}.png"));
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|_| RuntimeError::StorageUnavailable)?;
        let digest = hex(pin.digest.as_bytes());
        staged.push(
            codex_cli::StagedReference::new(path, &digest)
                .map_err(|_| RuntimeError::InvalidConfiguration)?,
        );
    }
    let mut cli =
        codex_cli::CodexCliConfig::new(std::path::PathBuf::from(config.codex.executable.as_str()))
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .with_request_limits(&config.request_limits)
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .with_max_artifact_bytes(config.transforms.max_output_bytes)
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .with_max_message_bytes(config.codex.max_message_bytes)
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .with_max_stdout_bytes(config.codex.max_stdout_bytes)
            .map_err(|_| RuntimeError::InvalidConfiguration)?
            .with_max_stderr_bytes(config.codex.max_stderr_bytes)
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
    if let Some(agent) = &config.codex.agent_model {
        cli = cli
            .with_agent_model(agent.as_str())
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
    }
    let request = codex_cli::CodexImageRequest::new(
        prompt(intent)?.clone(),
        if matches!(intent.operation, ImageOperation::Edit(_)) {
            codex_cli::ImageKind::Edit
        } else {
            codex_cli::ImageKind::Generate
        },
        staged,
        intent
            .output
            .generated()
            .ok_or(RuntimeError::InvalidRecord)?,
        if matches!(
            intent.output.delivered().alpha(),
            AlphaHandling::PreserveAlpha
        ) {
            codex_cli::AlphaRequirement::PreserveAlpha
        } else {
            codex_cli::AlphaRequirement::Either
        },
        intent.output.delivered().encoding().format(),
    );
    telemetry.observe(crate::telemetry::ImageEvent::new(
        crate::telemetry::EventKind::CodexStarted,
        event_context.clone(),
    ));
    let started = std::time::Instant::now();
    let outcome = codex_cli::execute(
        &cli,
        &request,
        &codex_cli::AttemptDirs::new(scratch, inputs, outputs),
        &codex_cli::process::TokioProcessRunner,
        timeout,
        None,
    )
    .await;
    telemetry.observe(codex_outcome_event(&outcome, event_context).elapsed(started.elapsed()));
    match outcome {
        codex_cli::CodexOutcome::Ready { receipt, usage } => {
            let observed = codex_usage(usage);
            let bytes = tokio::fs::read(receipt.path())
                .await
                .map_err(|_| DispatchFailure {
                    error: RuntimeError::StorageUnavailable,
                    usage: Some(codex_usage(usage)),
                    api: None,
                })?;
            Ok(ExecutedImage {
                bytes,
                units: observed.units,
                evidence: observed.evidence,
            })
        }
        codex_cli::CodexOutcome::Failed { error, usage }
            if matches!(
                &error,
                codex_cli::CodexError::NoArtifact | codex_cli::CodexError::LostOutput
            ) =>
        {
            Err(DispatchFailure {
                error: RuntimeError::ProviderUnknown,
                usage: Some(codex_usage(usage)),
                api: None,
            })
        }
        codex_cli::CodexOutcome::Failed { error, usage } => Err(DispatchFailure {
            error: RuntimeError::SupplierFailure(match error {
                codex_cli::CodexError::Refused { .. } => crate::execution::FailureCause::Refused,
                codex_cli::CodexError::GenerationFailed {
                    kind: codex_cli::GenerationFailureKind::UsageLimited { .. },
                } => crate::execution::FailureCause::QuotaExhausted,
                codex_cli::CodexError::NoArtifact
                | codex_cli::CodexError::Artifact(_)
                | codex_cli::CodexError::Unsupported { .. } => {
                    crate::execution::FailureCause::InvalidOutput
                }
                codex_cli::CodexError::Transport { .. } | codex_cli::CodexError::LostOutput => {
                    crate::execution::FailureCause::TransportLost
                }
                _ => crate::execution::FailureCause::Provider,
            }),
            usage: Some(codex_usage(usage)),
            api: None,
        }),
        codex_cli::CodexOutcome::Unknown { reason, usage } => Err(DispatchFailure {
            error: RuntimeError::SupplierFailure(if reason == codex_cli::UnknownReason::Deadline {
                crate::execution::FailureCause::DeadlineExceeded
            } else {
                crate::execution::FailureCause::TransportLost
            }),
            usage: Some(codex_usage(usage)),
            api: None,
        }),
    }
}

fn codex_usage(usage: codex_cli::ObservedUsage) -> ObservedDispatchUsage {
    let mut units = vec![unit("calls", 1)];
    for (category, quantity) in [
        ("text_input", usage.input_tokens()),
        ("text_output", usage.output_tokens()),
        ("text_cached_input", usage.cached_input_tokens()),
        ("text_reasoning_output", usage.reasoning_output_tokens()),
    ] {
        if let Some(quantity) = quantity {
            units.push(unit(category, quantity));
        }
    }
    let cumulative = usage.cumulative_usage();
    let raw = serde_json::json!({
        "schema_revision": 1,
        "thread_cumulative": {
            "input_tokens": cumulative.input_tokens,
            "output_tokens": cumulative.output_tokens,
            "cached_input_tokens": cumulative.cached_input_tokens,
            "reasoning_output_tokens": cumulative.reasoning_output_tokens,
            "total_tokens": cumulative.total_tokens,
        },
        "input_tokens": usage.input_tokens(),
        "output_tokens": usage.output_tokens(),
        "cached_input_tokens": usage.cached_input_tokens(),
        "reasoning_output_tokens": usage.reasoning_output_tokens(),
        "total_tokens": usage.total_tokens(),
    })
    .to_string()
    .into_bytes();
    ObservedDispatchUsage {
        units,
        evidence: if usage.is_unknown() {
            crate::cost::UsageEvidence::Unknown
        } else {
            crate::cost::UsageEvidence::Reported
        },
        raw,
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    text
}

/// Lifecycle event for one Codex turn. Carries only the outcome class and a failure kind; the
/// prompt, paths, stderr and supplier identifiers never reach telemetry.
fn codex_outcome_event(
    outcome: &codex_cli::CodexOutcome,
    context: crate::telemetry::EventContext,
) -> crate::telemetry::ImageEvent {
    use crate::telemetry::{EventKind, FailureKind, ImageEvent};
    match outcome {
        codex_cli::CodexOutcome::Ready { .. } => {
            ImageEvent::new(EventKind::CodexCompleted, context)
        }
        codex_cli::CodexOutcome::Failed { error, .. } => {
            let kind = match error {
                codex_cli::CodexError::Refused { .. }
                | codex_cli::CodexError::TurnFailed { .. }
                | codex_cli::CodexError::GenerationFailed {
                    kind: codex_cli::GenerationFailureKind::Reported { .. },
                } => FailureKind::ProviderRefused,
                codex_cli::CodexError::GenerationFailed {
                    kind: codex_cli::GenerationFailureKind::UsageLimited { .. },
                } => FailureKind::Capacity,
                codex_cli::CodexError::Artifact(_) | codex_cli::CodexError::Unsupported { .. } => {
                    FailureKind::Decode
                }
                codex_cli::CodexError::Configuration(_) | codex_cli::CodexError::Request(_) => {
                    FailureKind::Validation
                }
                codex_cli::CodexError::Transport { .. }
                | codex_cli::CodexError::Protocol(_)
                | codex_cli::CodexError::NoArtifact
                | codex_cli::CodexError::LostOutput
                | codex_cli::CodexError::Internal => FailureKind::ProviderUnknown,
            };
            ImageEvent::new(EventKind::Failed, context).failed(kind)
        }
        codex_cli::CodexOutcome::Unknown { .. } => {
            ImageEvent::new(EventKind::Unknown, context).failed(FailureKind::ProviderUnknown)
        }
    }
}
