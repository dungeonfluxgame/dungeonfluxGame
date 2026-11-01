//! Prepared one-call spatial questions and pure result interpretation.
use super::{Question, geometry, heads, pictures, reference_label, staging};
use crate::{
    backends::api::{check_credential, check_endpoint_url},
    config::AnalysisPreparationLimits,
    request::{AnalysisRoute, SourceRef},
    runtime::RuntimeError,
    service::analysis_types::*,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use df_providers::{
    HttpRequest, NativeHttp, NativeHttpRefusal, NativeHttpResponse, ProviderLimits,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

/// Authorized source bytes and resolved dependencies for one frozen spatial question.
pub(crate) struct StandaloneChatInput<'a> {
    pub(crate) operation: &'a ImageAnalysisOperation,
    pub(crate) source: &'a [u8],
    pub(crate) face: Option<&'a [u8]>,
    pub(crate) references: &'a [(SourceRef, Vec<u8>)],
    pub(crate) segmentation: Option<&'a ImageAnalysisResult>,
    pub(crate) locations: Option<&'a ImageAnalysisResult>,
    pub(crate) identity: Option<&'a ImageAnalysisResult>,
    pub(crate) route: &'a AnalysisRoute,
    pub(crate) preparation: &'a AnalysisPreparationLimits,
    pub(crate) max_decoded_bytes: u64,
    pub(crate) timeout: Duration,
    pub(crate) maximum_response_bytes: usize,
}
/// One owned exchange with prepared cost facts; execution returns raw HTTP evidence.
pub(crate) struct PreparedStandaloneChat {
    http: NativeHttp,
    request: HttpRequest,
    expires: Instant,
    pub(crate) text_bytes: u64,
    pub(crate) body_bytes: u64,
    pub(crate) picture_count: u64,
}
impl std::fmt::Debug for PreparedStandaloneChat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedStandaloneChat")
            .field("body_bytes", &self.body_bytes)
            .field("text_bytes", &self.text_bytes)
            .field("picture_count", &self.picture_count)
            .finish_non_exhaustive()
    }
}
pub(super) fn verify(pin: &SourceRef, bytes: &[u8]) -> Result<(), RuntimeError> {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    if pin.digest.as_bytes() != &digest {
        return Err(RuntimeError::SourceDenied);
    }
    Ok(())
}
fn result_source(result: &ImageAnalysisResult) -> &SourceRef {
    match result {
        ImageAnalysisResult::PhotoInspection { source, .. }
        | ImageAnalysisResult::HeadLocations { source, .. }
        | ImageAnalysisResult::Segmentation { source, .. }
        | ImageAnalysisResult::StagingIdentity { source, .. }
        | ImageAnalysisResult::StagingFacing { source, .. } => source,
    }
}
fn dependencies(
    source: &SourceRef,
    results: &[Option<&ImageAnalysisResult>],
) -> Result<(), RuntimeError> {
    if results
        .iter()
        .flatten()
        .any(|result| result_source(result) != source)
    {
        Err(RuntimeError::SourceDenied)
    } else {
        Ok(())
    }
}
struct IdentityContext {
    people: Vec<geometry::SegmentedPerson>,
    local: Vec<(String, usize)>,
    open_references: Vec<usize>,
    open_candidates: Vec<usize>,
}
fn identity_context(
    spec: &StagingIdentitySpec,
    segmentation: Option<&ImageAnalysisResult>,
    locations: Option<&ImageAnalysisResult>,
) -> Result<IdentityContext, RuntimeError> {
    dependencies(&spec.source, &[segmentation, locations])?;
    if spec.locations.is_some() != locations.is_some() {
        return Err(RuntimeError::InvalidRecord);
    }
    let people = geometry::people(segmentation.ok_or(RuntimeError::InvalidRecord)?)?;
    let local = geometry::locator_matches(&people, locations)?;
    let open_references = spec
        .references
        .iter()
        .enumerate()
        .filter(|(_, reference)| !local.iter().any(|(id, _)| id == reference.id.text()))
        .map(|(index, _)| index)
        .collect();
    let open_candidates = people
        .iter()
        .map(|person| person.candidate)
        .filter(|number| !local.iter().any(|(_, candidate)| candidate == number))
        .collect();
    Ok(IdentityContext {
        people,
        local,
        open_references,
        open_candidates,
    })
}
fn identity_result(
    spec: &StagingIdentitySpec,
    context: &IdentityContext,
    answer: Option<staging::IdentityAnswer>,
) -> Result<ImageAnalysisResult, RuntimeError> {
    let mut assignments = context.local.clone();
    if let Some(answer) = answer {
        for (reference, pick) in answer.assignments.iter().enumerate() {
            if let Some(number) = pick {
                let candidate = *context
                    .open_candidates
                    .get(number - 1)
                    .ok_or(RuntimeError::InvalidRecord)?;
                let index = *context
                    .open_references
                    .get(reference)
                    .ok_or(RuntimeError::InvalidRecord)?;
                assignments.push((spec.references[index].id.text().to_owned(), candidate));
            }
        }
    }
    let matches = spec
        .references
        .iter()
        .map(|reference| {
            Ok(AnalysisIdentityMatch {
                reference_id: reference.id.clone(),
                candidate: assignments
                    .iter()
                    .find(|(id, _)| id == reference.id.text())
                    .map(|(_, number)| u32::try_from(*number))
                    .transpose()
                    .map_err(|_| RuntimeError::InvalidRecord)?,
            })
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    Ok(ImageAnalysisResult::StagingIdentity {
        source: spec.source.clone(),
        contract_revision: spec.contract_revision,
        matches,
    })
}
/// Runs the exact deterministic staging policy without requiring any provider route or key.
pub(crate) fn prepare_local_analysis(
    operation: &ImageAnalysisOperation,
    segmentation: Option<&ImageAnalysisResult>,
    locations: Option<&ImageAnalysisResult>,
    identity: Option<&ImageAnalysisResult>,
) -> Result<Option<ImageAnalysisResult>, RuntimeError> {
    match operation {
        ImageAnalysisOperation::StagingIdentity(spec) => {
            let context = identity_context(spec, segmentation, locations)?;
            if staging::identity_request(
                context.open_references.len(),
                context.open_candidates.len(),
            )
            .is_none()
            {
                Ok(Some(identity_result(spec, &context, None)?))
            } else {
                Ok(None)
            }
        }
        ImageAnalysisOperation::StagingFacing(spec) => {
            dependencies(&spec.source, &[segmentation, identity])?;
            let people = geometry::people(segmentation.ok_or(RuntimeError::InvalidRecord)?)?;
            let numbers = people
                .iter()
                .map(|person| person.candidate)
                .collect::<Vec<_>>();
            if staging::facing_request(&numbers).is_none() {
                Ok(Some(ImageAnalysisResult::StagingFacing {
                    source: spec.source.clone(),
                    contract_revision: spec.contract_revision,
                    facings: Vec::new(),
                }))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}
/// Byte-bound convenience wrapper over the provider-independent local policy.
#[cfg(test)]
pub(crate) fn prepare_local_staging(
    input: &StandaloneChatInput<'_>,
) -> Result<Option<ImageAnalysisResult>, RuntimeError> {
    let source = input
        .operation
        .sources()
        .into_iter()
        .next()
        .ok_or(RuntimeError::InvalidRecord)?;
    verify(source, input.source)?;
    prepare_local_analysis(
        input.operation,
        input.segmentation,
        input.locations,
        input.identity,
    )
}

impl PreparedStandaloneChat {
    pub(crate) fn prepare(
        input: StandaloneChatInput<'_>,
        credential: &str,
    ) -> Result<Self, RuntimeError> {
        let expires = Instant::now()
            .checked_add(input.timeout)
            .ok_or(RuntimeError::InvalidConfiguration)?;
        if input.timeout.is_zero() {
            return Err(RuntimeError::Expired);
        }
        check_credential(credential).map_err(|_| RuntimeError::InvalidConfiguration)?;
        let allow_http = check_endpoint_url(input.route.endpoint.as_str())
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
        if input.route.maximum_output_tokens == 0
            || input.maximum_response_bytes == 0
            || input.max_decoded_bytes == 0
            || input.preparation.max_pixels == 0
            || input.preparation.max_picture_bytes == 0
            || input.preparation.max_request_bytes == 0
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        let retained = input.references.iter().try_fold(
            input.source.len() as u64 + input.face.map_or(0, |bytes| bytes.len() as u64),
            |total, (_, bytes)| {
                total
                    .checked_add(bytes.len() as u64)
                    .ok_or(RuntimeError::InvalidRecord)
            },
        )?;
        if retained > input.max_decoded_bytes {
            return Err(RuntimeError::InvalidRecord);
        }
        let (body, text_bytes, picture_count) = match input.operation {
            ImageAnalysisOperation::HeadLocations(spec) => {
                verify(&spec.source, input.source)?;
                let picture = pictures::vision(
                    input.source,
                    1024,
                    input.preparation,
                    input.max_decoded_bytes,
                    retained,
                )?;
                let face = match (&spec.face, input.face) {
                    (Some(pin), Some(bytes)) => {
                        verify(pin, bytes)?;
                        Some(pictures::vision(
                            bytes,
                            384,
                            input.preparation,
                            input.max_decoded_bytes,
                            retained + picture.capacity() as u64,
                        )?)
                    }
                    (None, None) => None,
                    _ => return Err(RuntimeError::InvalidRecord),
                };
                let body =
                    heads::request_body(&picture, face.as_deref(), &spec.targets, input.route)?;
                // Preserve the legacy head-locator text metric: serialized body excluding base64 payloads.
                let encoded = picture.len().div_ceil(3) * 4
                    + face.as_ref().map_or(0, |face| face.len().div_ceil(3) * 4);
                let text_bytes = body
                    .len()
                    .checked_sub(encoded)
                    .ok_or(RuntimeError::InvalidRecord)? as u64;
                (body, text_bytes, 1 + u64::from(face.is_some()))
            }
            ImageAnalysisOperation::StagingIdentity(spec) => {
                verify(&spec.source, input.source)?;
                let context = identity_context(spec, input.segmentation, input.locations)?;
                let question = staging::identity_request(
                    context.open_references.len(),
                    context.open_candidates.len(),
                )
                .ok_or(RuntimeError::InvalidRecord)?;
                let mut pictures = Vec::new();
                let mut held = retained;
                for (number, index) in context.open_references.iter().enumerate() {
                    let reference = &spec.references[*index];
                    let (_, bytes) = input
                        .references
                        .iter()
                        .find(|(pin, _)| pin == &reference.source)
                        .ok_or(RuntimeError::SourceDenied)?;
                    verify(&reference.source, bytes)?;
                    let jpeg = pictures::portrait(
                        bytes,
                        input.preparation,
                        input.max_decoded_bytes,
                        held,
                    )?;
                    held = held
                        .checked_add(jpeg.capacity() as u64)
                        .ok_or(RuntimeError::InvalidRecord)?;
                    pictures.push((reference_label(number + 1), jpeg));
                }
                let people = context
                    .people
                    .iter()
                    .filter(|person| context.open_candidates.contains(&person.candidate))
                    .collect::<Vec<_>>();
                let crops = pictures::crops(
                    input.source,
                    &people,
                    input.preparation,
                    input.max_decoded_bytes,
                    held,
                )?;
                for (number, (_, jpeg)) in crops.into_iter().enumerate() {
                    pictures.push((super::candidate_label(number + 1), jpeg));
                }
                question_body(
                    question,
                    pictures,
                    input.route,
                    input.preparation.max_request_bytes,
                )?
            }
            ImageAnalysisOperation::StagingFacing(spec) => {
                verify(&spec.source, input.source)?;
                dependencies(&spec.source, &[input.segmentation, input.identity])?;
                let people =
                    geometry::people(input.segmentation.ok_or(RuntimeError::InvalidRecord)?)?;
                let numbers = people
                    .iter()
                    .map(|person| person.candidate)
                    .collect::<Vec<_>>();
                let question =
                    staging::facing_request(&numbers).ok_or(RuntimeError::InvalidRecord)?;
                let refs = people.iter().collect::<Vec<_>>();
                let pictures = pictures::crops(
                    input.source,
                    &refs,
                    input.preparation,
                    input.max_decoded_bytes,
                    retained,
                )?;
                question_body(
                    question,
                    pictures,
                    input.route,
                    input.preparation.max_request_bytes,
                )?
            }
            ImageAnalysisOperation::Segmentation(_)
            | ImageAnalysisOperation::PhotoInspection(_) => {
                return Err(RuntimeError::UnsupportedCapability);
            }
        };
        if body.len() as u64 > input.preparation.max_request_bytes {
            return Err(RuntimeError::InvalidRecord);
        }
        let body_bytes = body.len() as u64;
        let http = NativeHttp::new(
            ProviderLimits::new(
                input.timeout,
                usize::try_from(input.preparation.max_request_bytes)
                    .map_err(|_| RuntimeError::UnsupportedCapability)?,
                input.maximum_response_bytes,
            )
            .build(),
            allow_http,
        )
        .map_err(|reason| {
            if reason == NativeHttpRefusal::InvalidLimits {
                RuntimeError::UnsupportedCapability
            } else {
                RuntimeError::InvalidConfiguration
            }
        })?;
        let request = HttpRequest::bearer_native(
            input.route.endpoint.as_str(),
            credential,
            "application/json".to_owned(),
            body,
        );
        Ok(Self {
            http,
            request,
            expires,
            text_bytes,
            body_bytes,
            picture_count,
        })
    }
    pub(crate) async fn execute(self) -> Result<NativeHttpResponse, RuntimeError> {
        let remaining = self.expires.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(RuntimeError::Expired);
        }
        self.http
            .execute(self.request, remaining, std::future::pending())
            .await
            .map_err(|failure| match failure.reason {
                NativeHttpRefusal::Deadline => RuntimeError::Expired,
                NativeHttpRefusal::Cancelled => RuntimeError::Cancelled,
                NativeHttpRefusal::ProviderDisabled => RuntimeError::Disabled,
                _ => RuntimeError::ProviderUnknown,
            })
    }
}
fn question_body(
    question: Question,
    pictures: Vec<(String, Vec<u8>)>,
    route: &AnalysisRoute,
    maximum_request_bytes: u64,
) -> Result<(Vec<u8>, u64, u64), RuntimeError> {
    let encoded = pictures.iter().try_fold(0u64, |total, (_, bytes)| {
        total
            .checked_add((bytes.len() as u64).div_ceil(3) * 4)
            .ok_or(RuntimeError::InvalidRecord)
    })?;
    if encoded > maximum_request_bytes {
        return Err(RuntimeError::InvalidRecord);
    }
    let text_bytes = (question.instruction.len() + question.schema.to_string().len()) as u64;
    let count = pictures.len() as u64;
    let mut content = vec![json!({"type":"text","text":question.instruction})];
    for (label, jpeg) in pictures {
        content.push(json!({"type":"text","text":format!("Picture {label}:")}));
        content.push(json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg)),
                "detail": "low",
            },
        }));
    }
    let body = serde_json::to_vec(&json!({
        "model": route.model.as_str(),
        "max_completion_tokens": route.maximum_output_tokens,
        "messages": [{ "role": "user", "content": content }],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": question.schema_name,
                "strict": true,
                "schema": question.schema,
            },
        },
    }))
    .map_err(|_| RuntimeError::InvalidRecord)?;
    Ok((body, text_bytes, count))
}
/// Replays the frozen question policy against a retained chat-completions response.
pub(crate) fn interpret_chat(
    value: &Value,
    operation: &ImageAnalysisOperation,
    segmentation: Option<&ImageAnalysisResult>,
    locations: Option<&ImageAnalysisResult>,
    identity: Option<&ImageAnalysisResult>,
) -> Result<ImageAnalysisResult, RuntimeError> {
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or(RuntimeError::InvalidRecord)?;
    match operation {
        ImageAnalysisOperation::HeadLocations(spec) => Ok(ImageAnalysisResult::HeadLocations {
            source: spec.source.clone(),
            contract_revision: spec.contract_revision,
            heads: heads::parse_heads(content, &spec.targets)?,
        }),
        ImageAnalysisOperation::StagingIdentity(spec) => {
            let answer = df_providers::parse_unique_json(
                content.as_bytes(),
                df_providers::TextSchemaLimits {
                    maximum_schema_bytes: content.len(),
                    maximum_depth: 16,
                    maximum_nodes: 4096,
                },
            )
            .map_err(|_| RuntimeError::InvalidRecord)?;
            let context = identity_context(spec, segmentation, locations)?;
            let read = staging::read_identity(
                &answer,
                context.open_references.len(),
                context.open_candidates.len(),
            )?;
            identity_result(spec, &context, Some(read))
        }
        ImageAnalysisOperation::StagingFacing(spec) => {
            dependencies(&spec.source, &[segmentation, identity])?;
            let answer = df_providers::parse_unique_json(
                content.as_bytes(),
                df_providers::TextSchemaLimits {
                    maximum_schema_bytes: content.len(),
                    maximum_depth: 16,
                    maximum_nodes: 4096,
                },
            )
            .map_err(|_| RuntimeError::InvalidRecord)?;
            let people = geometry::people(segmentation.ok_or(RuntimeError::InvalidRecord)?)?;
            let numbers = people
                .iter()
                .map(|person| person.candidate)
                .collect::<Vec<_>>();
            let facings = numbers
                .iter()
                .zip(staging::read_facing(&answer, &numbers)?)
                .map(|(number, facing)| {
                    Ok(AnalysisFacing {
                        candidate: u32::try_from(*number)
                            .map_err(|_| RuntimeError::InvalidRecord)?,
                        facing: facing.map(|facing| match facing {
                            staging::Facing::Left => AnalysisFacingDirection::Left,
                            staging::Facing::Right => AnalysisFacingDirection::Right,
                            staging::Facing::TowardCamera => AnalysisFacingDirection::TowardCamera,
                            staging::Facing::AwayFromCamera => {
                                AnalysisFacingDirection::AwayFromCamera
                            }
                        }),
                    })
                })
                .collect::<Result<Vec<_>, RuntimeError>>()?;
            Ok(ImageAnalysisResult::StagingFacing {
                source: spec.source.clone(),
                contract_revision: spec.contract_revision,
                facings,
            })
        }
        _ => Err(RuntimeError::UnsupportedCapability),
    }
}
