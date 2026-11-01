//! Authorized sources, immutable dependencies and bounded request preparation.
use super::*;

impl<J, E, S, P, F, C, I, K, B, T, Q, U> NativeImageRuntime<J, E, S, P, F, C, I, K, B, T, Q, U>
where
    J: ImageJournal,
    E: ImageExecutionJournal,
    S: ImageSources,
    P: ImagePublisher,
    F: ImageDispatchAdmission,
    C: ServiceClock,
    I: RuntimeIds,
    K: ImageCredentials,
    B: ImageCostProjection,
    T: ImageRuntimeTelemetry,
    Q: ImageRuntimeCostPolicy,
    U: ImageUsageJournal,
{
    pub(super) async fn analysis_dependency(
        &self,
        intent: &DecodedAnalysis,
        handle: &crate::OperationHandle,
    ) -> Result<ResolvedDependency, RuntimeError> {
        if handle.owner != intent.owner {
            return Err(RuntimeError::SourceDenied);
        }
        let operation = self
            .journal
            .lookup_operation(&handle.owner, handle.operation)
            .await
            .map_err(RuntimeError::Journal)?
            .ok_or(RuntimeError::SourceDenied)?;
        if operation.interest.state != crate::InterestState::Attached
            || operation.job.state == crate::JobState::Retired
        {
            return Err(RuntimeError::SourceDenied);
        }
        let retained = analysis_codec::decode_analysis(operation.job.intent.bytes())
            .map_err(|_| RuntimeError::InvalidRecord)?;
        if retained.owner != intent.owner
            || retained.audience.meet(&intent.audience) != intent.audience
            || analysis_codec::source(&retained.operation)
                != analysis_codec::source(&intent.operation)
        {
            return Err(RuntimeError::SourceDenied);
        }
        let checkpoint = self
            .execution
            .read_analysis_checkpoint(&handle.owner, &operation.job.job)
            .await
            .map_err(RuntimeError::Execution)?
            .ok_or(RuntimeError::SourceDenied)?;
        let expected_kind = match &intent.operation {
            ImageAnalysisOperation::StagingIdentity(spec) if &spec.segmentation == handle => {
                StageKind::Segmentation
            }
            ImageAnalysisOperation::StagingIdentity(_) => StageKind::HeadLocations,
            ImageAnalysisOperation::StagingFacing(spec) if &spec.segmentation == handle => {
                StageKind::Segmentation
            }
            ImageAnalysisOperation::StagingFacing(_) => StageKind::StagingIdentity,
            _ => return Err(RuntimeError::InvalidRecord),
        };
        if kind(&retained.operation) != expected_kind {
            return Err(RuntimeError::SourceDenied);
        }
        if checkpoint.scope.owner != handle.owner
            || checkpoint.scope.job != operation.job.job
            || checkpoint.kind.stage_kind() != expected_kind
        {
            return Err(RuntimeError::InvalidRecord);
        }
        let result = analysis_result(&checkpoint)?;
        crate::service::analysis::validate_result_binding(&retained.operation, &result)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        Ok(ResolvedDependency {
            result,
            reference: AnalysisDependencyRef {
                handle: handle.clone(),
                checkpoint: AnalysisCheckpointReference::from_checkpoint(&checkpoint),
            },
        })
    }

    pub(super) async fn analysis_inputs(
        &self,
        intent: &DecodedAnalysis,
    ) -> Result<Inputs, RuntimeError> {
        let pins = analysis_codec::sources(&intent.operation);
        let mut bytes = Vec::with_capacity(pins.len());
        let mut receipts = Vec::with_capacity(pins.len());
        for pin in pins {
            let authorized = self.sources.read_source(&intent.owner, pin).await?;
            if authorized.receipt.pin != *pin
                || intent.audience.meet(&pin.audience) != intent.audience
            {
                return Err(RuntimeError::SourceDenied);
            }
            bytes.push(authorized.bytes);
            receipts.push(authorized.receipt);
        }
        let mut bytes = bytes.into_iter();
        let source = bytes.next().ok_or(RuntimeError::InvalidRecord)?;
        let (dependencies, dependency_refs) = self.analysis_dependencies(intent).await?;
        let mut face = None;
        let mut references = Vec::new();
        match &intent.operation {
            ImageAnalysisOperation::HeadLocations(spec) if spec.face.is_some() => {
                face = Some(bytes.next().ok_or(RuntimeError::InvalidRecord)?)
            }
            ImageAnalysisOperation::PhotoInspection(spec) => {
                for reference in &spec.references {
                    references.push((
                        reference.clone(),
                        bytes.next().ok_or(RuntimeError::InvalidRecord)?,
                    ));
                }
            }
            ImageAnalysisOperation::StagingIdentity(spec) => {
                for reference in &spec.references {
                    references.push((
                        reference.source.clone(),
                        bytes.next().ok_or(RuntimeError::InvalidRecord)?,
                    ));
                }
            }
            _ => {}
        }
        Ok(Inputs {
            source,
            face,
            references,
            receipts,
            dependencies,
            dependency_refs,
        })
    }

    pub(super) async fn analysis_dependencies(
        &self,
        intent: &DecodedAnalysis,
    ) -> Result<(Dependencies, Vec<AnalysisDependencyRef>), RuntimeError> {
        let mut dependencies = Dependencies::default();
        let mut references = Vec::new();
        match &intent.operation {
            ImageAnalysisOperation::StagingIdentity(spec) => {
                let dependency = self.analysis_dependency(intent, &spec.segmentation).await?;
                dependencies.segmentation = Some(dependency.result);
                references.push(dependency.reference);
                if let Some(locations) = &spec.locations {
                    let dependency = self.analysis_dependency(intent, locations).await?;
                    dependencies.locations = Some(dependency.result);
                    references.push(dependency.reference);
                }
            }
            ImageAnalysisOperation::StagingFacing(spec) => {
                let identity = self
                    .journal
                    .lookup_operation(&spec.identity.owner, spec.identity.operation)
                    .await
                    .map_err(RuntimeError::Journal)?
                    .ok_or(RuntimeError::SourceDenied)?;
                let retained = analysis_codec::decode_analysis(identity.job.intent.bytes())
                    .map_err(|_| RuntimeError::InvalidRecord)?;
                let ImageAnalysisOperation::StagingIdentity(identity) = retained.operation else {
                    return Err(RuntimeError::SourceDenied);
                };
                if identity.segmentation != spec.segmentation {
                    return Err(RuntimeError::SourceDenied);
                }
                let dependency = self.analysis_dependency(intent, &spec.segmentation).await?;
                dependencies.segmentation = Some(dependency.result);
                references.push(dependency.reference);
                let dependency = self.analysis_dependency(intent, &spec.identity).await?;
                dependencies.identity = Some(dependency.result);
                references.push(dependency.reference);
            }
            _ => {}
        }
        Ok((dependencies, references))
    }

    pub(super) async fn prepare_analysis(
        &self,
        intent: &DecodedAnalysis,
        inputs: Inputs,
        config: &ImageConfig,
        preparation: AnalysisPreparation,
    ) -> Result<
        (
            Prepared,
            Inputs,
            Option<super::super::quota::QuotaAdmission>,
        ),
        RuntimeError,
    > {
        let AnalysisPreparation {
            prompt_index,
            timeout,
            maximum,
            mode,
        } = preparation;
        let operation = intent.operation.clone();
        let (local, inputs) = tokio::task::spawn_blocking(move || {
            let local = backend::prepare_local_analysis(
                &operation,
                inputs.dependencies.segmentation.as_ref(),
                inputs.dependencies.locations.as_ref(),
                inputs.dependencies.identity.as_ref(),
            )?;
            Ok::<_, RuntimeError>((local, inputs))
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)??;
        if let Some(result) = local {
            return Ok((Prepared::Local(result), inputs, None));
        }
        if mode == PreparationMode::Fresh
            && (crate::service::analysis::analysis_policy(
                &self.settings.config,
                intent.purpose.purpose,
                intent.execution,
                true,
            ) != crate::PolicyDecision::Admit
                || !crate::service::analysis::analysis_route_present(
                    &self.settings.config,
                    &intent.operation,
                )
                || !crate::service::analysis::analysis_route_present(config, &intent.operation))
        {
            return Err(RuntimeError::Disabled);
        }
        let credential = if matches!(&intent.operation, ImageAnalysisOperation::Segmentation(_)) {
            let route = config
                .analysis
                .segmentation
                .as_ref()
                .ok_or(RuntimeError::InvalidConfiguration)?;
            self.credentials
                .credential(crate::BackendId::Fal, &route.account)?
        } else {
            self.credentials
                .analysis_credential(&chat_route(config, &intent.operation)?.account)?
        };
        let quota = if mode == PreparationMode::Fresh {
            let (endpoint, trusted) =
                if matches!(&intent.operation, ImageAnalysisOperation::Segmentation(_)) {
                    let route = config
                        .analysis
                        .segmentation
                        .as_ref()
                        .ok_or(RuntimeError::InvalidConfiguration)?;
                    (
                        route.endpoint.as_str(),
                        self.credentials
                            .quota_identity(crate::BackendId::Fal, &route.account),
                    )
                } else {
                    (
                        chat_route(config, &intent.operation)?.endpoint.as_str(),
                        None,
                    )
                };
            super::super::quota_gate::admit(
                &self.credentials,
                self.settings.config.quota,
                endpoint,
                &credential,
                trusted.as_ref(),
            )?
        } else {
            None
        };
        let intent = intent.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || {
            let prepared = prepare_paid(
                &intent,
                &inputs,
                &config,
                prompt_index,
                timeout,
                maximum,
                &credential,
            )?;
            Ok((prepared, inputs, quota))
        })
        .await
        .map_err(|_| RuntimeError::WorkerFailed)?
    }
}
fn prepare_paid(
    intent: &DecodedAnalysis,
    inputs: &Inputs,
    config: &ImageConfig,
    prompt_index: usize,
    timeout: Duration,
    maximum: usize,
    credential: &str,
) -> Result<Prepared, RuntimeError> {
    match &intent.operation {
        ImageAnalysisOperation::PhotoInspection(spec) => {
            Ok(Prepared::Photo(backend::PreparedPhotoInspection::prepare(
                backend::PhotoInspectionInput {
                    spec,
                    source: &inputs.source,
                    references: &inputs.references,
                    preparation: &config.analysis.preparation,
                    max_decoded_bytes: config.transforms.max_decoded_bytes,
                    timeout,
                    maximum_response_bytes: maximum,
                },
                credential,
            )?))
        }
        ImageAnalysisOperation::Segmentation(spec) => {
            let route = config
                .analysis
                .segmentation
                .as_ref()
                .ok_or(RuntimeError::InvalidConfiguration)?;
            let prompt = *spec
                .prompts
                .get(prompt_index)
                .ok_or(RuntimeError::InvalidRecord)?;
            Ok(Prepared::Segmentation(
                backend::PreparedSegmentation::prepare(
                    backend::SegmentationInput {
                        source: &inputs.source,
                        source_pin: &spec.source,
                        prompt,
                        preparation_recipe: spec.preparation,
                        maximum_masks: spec.maximum_masks,
                        route,
                        preparation: &config.analysis.segmentation_preparation,
                        max_decoded_bytes: config.transforms.max_decoded_bytes,
                        timeout,
                        maximum_response_bytes: maximum,
                    },
                    credential,
                )?,
            ))
        }
        _ => {
            let route = chat_route(config, &intent.operation)?;
            let input = backend::StandaloneChatInput {
                operation: &intent.operation,
                source: &inputs.source,
                face: inputs.face.as_deref(),
                references: &inputs.references,
                segmentation: inputs.dependencies.segmentation.as_ref(),
                locations: inputs.dependencies.locations.as_ref(),
                identity: inputs.dependencies.identity.as_ref(),
                route,
                preparation: &config.analysis.preparation,
                max_decoded_bytes: config.transforms.max_decoded_bytes,
                timeout,
                maximum_response_bytes: maximum,
            };
            Ok(Prepared::Chat(backend::PreparedStandaloneChat::prepare(
                input, credential,
            )?))
        }
    }
}
