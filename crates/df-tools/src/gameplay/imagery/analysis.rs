//! Canonical analysis consumers: actual source pins and one durable service lifecycle.
//! Preparation, providers, usage, coalescing and retained results belong to ImageService.

use std::time::Duration;

use df_imagery::{
    AnalysisMaskSet, AnalysisReference, AnalysisTarget, ExecutionMode, Freshness,
    HeadLocationsSpec, ImageAnalysisOperation, ImageAnalysisRequest, ImageAnalysisResult,
    ImageExecutionState, ImageOwner, ImageServiceError, OperationHandle, OperationIdentity,
    PrivateImageText, Purpose, PurposedUse, RequestOutcome, SegmentationPreparation,
    SegmentationPrompt, SegmentationSpec, SourceRef, StagingFacingSpec, StagingIdentitySpec,
    Urgency, VariantRole, WatchObservation,
};
use df_providers::{HeadBox, LocatedHead, MaskRows, MaskRun, SegmentInstance};
use sha2::{Digest, Sha256};

use super::{ImageFailure, ImageKey, Imagery, ReferenceImage};

/// An observed result and the actual durable operation that produced it.
pub(in crate::gameplay) struct Observation<T> {
    pub(in crate::gameplay) handle: OperationHandle,
    pub(in crate::gameplay) source: SourceRef,
    pub(in crate::gameplay) value: T,
    pub(in crate::gameplay) usage: Option<df_imagery::runtime::ImageUsageSummary>,
}

pub(in crate::gameplay) async fn heads(
    imagery: &Imagery,
    key: &ImageKey,
    face: Option<&ReferenceImage>,
    targets: &[(&str, &str)],
) -> Result<Observation<Vec<LocatedHead>>, ImageFailure> {
    let source = imagery
        .pin_reference(&imagery.reference(key).await?)
        .await?;
    let face = match face {
        Some(face) => Some(imagery.pin_reference(face).await?),
        None => None,
    };
    let operation = ImageAnalysisOperation::HeadLocations(HeadLocationsSpec {
        source: source.clone(),
        face,
        targets: targets
            .iter()
            .map(|(id, description)| AnalysisTarget {
                id: PrivateImageText::new(id),
                description: PrivateImageText::new(description),
            })
            .collect(),
        contract_revision: 1,
    });
    let observed = execute(imagery, "head-locations", operation).await?;
    let ImageAnalysisResult::HeadLocations { heads, .. } = observed.value else {
        return Err(ImageFailure::BadOutput);
    };
    Ok(Observation {
        handle: observed.handle,
        source,
        usage: observed.usage,
        value: heads
            .into_iter()
            .map(|head| {
                let [left, top, right, bottom] = head.head.coordinates();
                LocatedHead {
                    id: head.id.text().to_owned(),
                    head: HeadBox {
                        left,
                        top,
                        right,
                        bottom,
                    },
                }
            })
            .collect(),
    })
}

pub(in crate::gameplay) async fn segments(
    imagery: &Imagery,
    key: &ImageKey,
    preparation: SegmentationPreparation,
    prompts: Vec<SegmentationPrompt>,
    maximum_masks: u32,
) -> Result<Observation<Vec<AnalysisMaskSet>>, ImageFailure> {
    let source = imagery
        .pin_reference(&imagery.reference(key).await?)
        .await?;
    let operation = ImageAnalysisOperation::Segmentation(SegmentationSpec {
        source: source.clone(),
        preparation,
        prompts,
        maximum_masks,
        contract_revision: 1,
    });
    let observed = execute(imagery, "segmentation", operation).await?;
    let ImageAnalysisResult::Segmentation { masks, .. } = observed.value else {
        return Err(ImageFailure::BadOutput);
    };
    for set in &masks {
        set.validate().map_err(|_| ImageFailure::BadOutput)?;
    }
    Ok(Observation {
        handle: observed.handle,
        source,
        value: masks,
        usage: observed.usage,
    })
}

pub(in crate::gameplay) async fn identity(
    imagery: &Imagery,
    segmentation: &Observation<Vec<AnalysisMaskSet>>,
    locations: Option<&OperationHandle>,
    references: &[(String, ReferenceImage)],
) -> Result<Observation<Vec<df_imagery::AnalysisIdentityMatch>>, ImageFailure> {
    let mut pins = Vec::with_capacity(references.len());
    for (id, reference) in references {
        pins.push(AnalysisReference {
            id: PrivateImageText::new(id),
            source: imagery.pin_reference(reference).await?,
        });
    }
    let operation = ImageAnalysisOperation::StagingIdentity(StagingIdentitySpec {
        source: segmentation.source.clone(),
        segmentation: segmentation.handle.clone(),
        locations: locations.cloned(),
        references: pins,
        contract_revision: 1,
    });
    let observed = execute(imagery, "staging-identity", operation).await?;
    let ImageAnalysisResult::StagingIdentity { matches, .. } = observed.value else {
        return Err(ImageFailure::BadOutput);
    };
    Ok(Observation {
        handle: observed.handle,
        source: observed.source,
        value: matches,
        usage: observed.usage,
    })
}

pub(in crate::gameplay) async fn facing(
    imagery: &Imagery,
    segmentation: &Observation<Vec<AnalysisMaskSet>>,
    identity: &Observation<Vec<df_imagery::AnalysisIdentityMatch>>,
) -> Result<Observation<Vec<df_imagery::AnalysisFacing>>, ImageFailure> {
    if segmentation.source != identity.source {
        return Err(ImageFailure::InvalidRequest);
    }
    let operation = ImageAnalysisOperation::StagingFacing(StagingFacingSpec {
        source: segmentation.source.clone(),
        segmentation: segmentation.handle.clone(),
        identity: identity.handle.clone(),
        contract_revision: 1,
    });
    let observed = execute(imagery, "staging-facing", operation).await?;
    let ImageAnalysisResult::StagingFacing { facings, .. } = observed.value else {
        return Err(ImageFailure::BadOutput);
    };
    Ok(Observation {
        handle: observed.handle,
        source: observed.source,
        value: facings,
        usage: observed.usage,
    })
}

/// Reads an actual completed head observation; raw caller boxes are never promoted to a dependency.
pub(in crate::gameplay) async fn locations(
    imagery: &Imagery,
    handle: &OperationHandle,
    source: &SourceRef,
) -> Result<Vec<crate::gameplay::plate_segments::identity::LocatedPerson>, ImageFailure> {
    let context = imagery.consumer_context().await?;
    if handle.owner != context.owner {
        return Err(ImageFailure::Refused);
    }
    let view = context
        .service
        .read(&context.principal, handle)
        .await
        .map_err(failure)?;
    let Some(ImageAnalysisResult::HeadLocations {
        source: actual,
        contract_revision: 1,
        heads,
    }) = view.analysis_result
    else {
        return Err(ImageFailure::BadOutput);
    };
    if actual != *source || !matches!(view.execution, ImageExecutionState::Completed) {
        return Err(ImageFailure::BadOutput);
    }
    Ok(heads
        .into_iter()
        .map(|head| {
            let [left, top, right, bottom] = head.head.coordinates();
            crate::gameplay::plate_segments::identity::LocatedPerson {
                id: head.id.text().to_owned(),
                head: HeadBox {
                    left,
                    top,
                    right,
                    bottom,
                },
            }
        })
        .collect())
}

/// Converts normalized provider geometry without changing indices, score bits or pixel runs.
pub(in crate::gameplay) fn instances(
    set: &AnalysisMaskSet,
) -> Result<Vec<SegmentInstance>, ImageFailure> {
    set.validate().map_err(|_| ImageFailure::BadOutput)?;
    set.instances
        .iter()
        .map(|instance| {
            Ok(SegmentInstance {
                index: instance.index,
                score: instance.score.get(),
                mask: MaskRows::from_runs(
                    set.width_px,
                    set.height_px,
                    instance
                        .runs
                        .iter()
                        .map(|run| MaskRun {
                            start: run.start,
                            length: run.length,
                        })
                        .collect(),
                )
                .map_err(|_| ImageFailure::BadOutput)?,
            })
        })
        .collect()
}

pub(super) async fn execute(
    imagery: &Imagery,
    subject: &str,
    operation: ImageAnalysisOperation,
) -> Result<Observation<ImageAnalysisResult>, ImageFailure> {
    execute_with_freshness(imagery, subject, operation, Freshness::ReuseCompatible).await
}

pub(super) async fn execute_with_freshness(
    imagery: &Imagery,
    subject: &str,
    operation: ImageAnalysisOperation,
    freshness: Freshness,
) -> Result<Observation<ImageAnalysisResult>, ImageFailure> {
    let context = imagery.consumer_context().await?;
    let source = operation_source(&operation).clone();
    let audience = match &context.owner {
        ImageOwner::Run { campaign, .. } => df_imagery::Audience::CampaignShared(*campaign),
        ImageOwner::Draft { account } => df_imagery::Audience::OwnerPrivate(*account),
    };
    let mut request = ImageAnalysisRequest {
        identity: OperationIdentity {
            owner: context.owner.clone(),
            operation: operation_id(subject.as_bytes())?,
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ReferencePrep,
            role: VariantRole::Reference,
        },
        operation,
        execution: ExecutionMode::Live,
        freshness,
        urgency: Urgency::Present,
        audience,
        expected_source_revision: context.expected_source_revision,
    };
    request.identity.fingerprint = context
        .service
        .canonical_analysis_fingerprint(&request)
        .map_err(failure)?;
    let mut identity = Sha256::new();
    identity.update(b"df-gameplay/image-analysis-operation/1");
    identity.update(subject.as_bytes());
    identity.update(request.identity.fingerprint);
    request.identity.operation = operation_id(&identity.finalize())?;
    let mut view = match context
        .service
        .request_analysis(&context.principal, &request)
        .await
        .map_err(failure)?
    {
        RequestOutcome::Recorded(view) | RequestOutcome::Replayed(view) => view,
        RequestOutcome::Reused(mut view) => {
            // Reuse retains the authorized historical operation, rather than minting this
            // request's handle. Only a complete decision of the exact requested kind/source
            // may cross this boundary.
            if view.handle.owner != request.identity.owner
                || !matches!(view.execution, ImageExecutionState::Completed)
            {
                return Err(ImageFailure::BadOutput);
            }
            if let Some(cause) = view.failure {
                return Err(super::central::semantic_failure(cause));
            }
            let result = view.analysis_result.take().ok_or(ImageFailure::BadOutput)?;
            if !result_matches(&request.operation, &result) {
                return Err(ImageFailure::BadOutput);
            }
            return Ok(Observation {
                handle: view.handle,
                source,
                value: result,
                usage: view.usage,
            });
        }
        RequestOutcome::ReusedArtifact(_) => return Err(ImageFailure::BadOutput),
        RequestOutcome::Refused(_) => return Err(ImageFailure::Off),
    };
    let waiting = async {
        loop {
            if view.handle.owner != request.identity.owner
                || view.handle.operation != request.identity.operation
            {
                return Err(ImageFailure::BadOutput);
            }
            if let Some(cause) = view.failure {
                return Err(super::central::semantic_failure(cause));
            }
            if let Some(result) = view.analysis_result.take() {
                if !matches!(view.execution, ImageExecutionState::Completed)
                    || !result_matches(&request.operation, &result)
                {
                    return Err(ImageFailure::BadOutput);
                }
                return Ok(Observation {
                    handle: view.handle,
                    source,
                    value: result,
                    usage: view.usage,
                });
            }
            if view.operational_notice.is_some() {
                // Withdraw this synchronous caller's actual interest before surfacing quota.
                // A raced dispatch keeps its existing liability; cancellation is no refund claim.
                let current = imagery.consumer_context().await?;
                if current.owner != context.owner {
                    return Err(ImageFailure::Refused);
                }
                match context
                    .service
                    .cancel(&current.principal, &view.handle)
                    .await
                    .map_err(failure)?
                {
                    df_imagery::CancelOutcome::Withdrawn(_)
                    | df_imagery::CancelOutcome::AlreadyWithdrawn(_) => {
                        return Err(ImageFailure::QuotaExhausted);
                    }
                }
            }
            match view.execution {
                ImageExecutionState::Completed | ImageExecutionState::Ready => {
                    return Err(ImageFailure::BadOutput);
                }
                ImageExecutionState::Unknown
                | ImageExecutionState::Uncertain
                | ImageExecutionState::Withdrawn => return Err(ImageFailure::Interrupted),
                ImageExecutionState::Failed { .. } | ImageExecutionState::NeedsRevision => {
                    return Err(ImageFailure::Provider);
                }
                ImageExecutionState::Expired => return Err(ImageFailure::DeadlineExceeded),
                _ => {}
            }
            tokio::time::sleep(context.poll_interval).await;
            let current = imagery.consumer_context().await?;
            if current.owner != context.owner {
                return Err(ImageFailure::Refused);
            }
            if current.now_ms >= view.absolute_deadline_ms {
                return Err(ImageFailure::DeadlineExceeded);
            }
            match context
                .service
                .watch(&current.principal, &view.handle, view.job_revision)
                .await
                .map_err(failure)?
            {
                WatchObservation::Changed(changed) => view = *changed,
                WatchObservation::Unchanged { .. } => {}
            }
        }
    };
    tokio::time::timeout(
        Duration::from_secs(context.config.scheduler.job_deadline_secs),
        waiting,
    )
    .await
    .map_err(|_| ImageFailure::DeadlineExceeded)?
}

fn operation_source(operation: &ImageAnalysisOperation) -> &SourceRef {
    match operation {
        ImageAnalysisOperation::PhotoInspection(spec) => &spec.source,
        ImageAnalysisOperation::HeadLocations(spec) => &spec.source,
        ImageAnalysisOperation::Segmentation(spec) => &spec.source,
        ImageAnalysisOperation::StagingIdentity(spec) => &spec.source,
        ImageAnalysisOperation::StagingFacing(spec) => &spec.source,
    }
}

fn result_matches(operation: &ImageAnalysisOperation, result: &ImageAnalysisResult) -> bool {
    let expected = operation_source(operation);
    match (operation, result) {
        (
            ImageAnalysisOperation::PhotoInspection(_),
            ImageAnalysisResult::PhotoInspection { source, assessment },
        ) => source == expected && assessment.contract_revision == 1,
        (
            ImageAnalysisOperation::HeadLocations(_),
            ImageAnalysisResult::HeadLocations {
                source,
                contract_revision,
                ..
            },
        )
        | (
            ImageAnalysisOperation::Segmentation(_),
            ImageAnalysisResult::Segmentation {
                source,
                contract_revision,
                ..
            },
        )
        | (
            ImageAnalysisOperation::StagingIdentity(_),
            ImageAnalysisResult::StagingIdentity {
                source,
                contract_revision,
                ..
            },
        )
        | (
            ImageAnalysisOperation::StagingFacing(_),
            ImageAnalysisResult::StagingFacing {
                source,
                contract_revision,
                ..
            },
        ) => source == expected && *contract_revision == 1,
        _ => false,
    }
}

fn operation_id(bytes: &[u8]) -> Result<df_types::OperationId, ImageFailure> {
    let digest = Sha256::digest(bytes);
    df_types::OperationId::from_bytes(&digest[..16]).map_err(|_| ImageFailure::InvalidRequest)
}

fn failure(error: ImageServiceError) -> ImageFailure {
    match error {
        ImageServiceError::InvalidRequest(_) | ImageServiceError::Conflict => {
            ImageFailure::InvalidRequest
        }
        ImageServiceError::Unknown { .. } => ImageFailure::Interrupted,
        ImageServiceError::NotAuthorizedOrNotFound => ImageFailure::Refused,
        ImageServiceError::InvalidRecord => ImageFailure::BadOutput,
        ImageServiceError::Unavailable => ImageFailure::Provider,
    }
}
