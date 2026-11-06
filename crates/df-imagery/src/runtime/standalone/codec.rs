//! Bounded raw and typed response envelopes and replay qualification.
use super::*;

// NativeHttp's technical response ceiling also bounds each retained analysis value.
const MAXIMUM_RESPONSE_BYTES: usize = 16_777_216;
const MAXIMUM_RAW_ENVELOPE_BYTES: usize = MAXIMUM_RESPONSE_BYTES * 5 + 128;
pub(super) fn write_result(
    out: &mut crate::intent_codec::primitives::Writer,
    result: Option<&ImageAnalysisResult>,
    maximum: usize,
) -> Result<(), RuntimeError> {
    out.boolean(result.is_some())
        .map_err(|_| RuntimeError::InvalidRecord)?;
    if let Some(result) = result {
        let bytes = analysis_codec::encode_result(result, maximum)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        out.counted(&bytes)
            .map_err(|_| RuntimeError::InvalidRecord)?;
    }
    Ok(())
}
pub(super) fn read_result(
    input: &mut crate::intent_codec::primitives::Reader<'_>,
    maximum: usize,
) -> Result<Option<ImageAnalysisResult>, RuntimeError> {
    if !input.boolean().map_err(|_| RuntimeError::InvalidRecord)? {
        return Ok(None);
    }
    let bytes = input
        .counted(maximum)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    analysis_codec::decode_result(bytes, maximum)
        .map(Some)
        .map_err(|_| RuntimeError::InvalidRecord)
}
pub(super) fn write_dependencies(
    out: &mut crate::intent_codec::primitives::Writer,
    deps: &Dependencies,
    maximum: usize,
) -> Result<(), RuntimeError> {
    for result in [&deps.segmentation, &deps.locations, &deps.identity] {
        write_result(out, result.as_ref(), maximum)?;
    }
    Ok(())
}
pub(super) fn read_dependencies(
    input: &mut crate::intent_codec::primitives::Reader<'_>,
    maximum: usize,
) -> Result<Dependencies, RuntimeError> {
    Ok(Dependencies {
        segmentation: read_result(input, maximum)?,
        locations: read_result(input, maximum)?,
        identity: read_result(input, maximum)?,
    })
}
fn payload_validity(result: &ImageAnalysisResult) -> u8 {
    match result {
        ImageAnalysisResult::PhotoInspection { assessment, .. }
            if matches!(assessment.verdict, PhotoInspectionVerdict::Unjudged(_)) =>
        {
            0
        }
        _ => 1,
    }
}
pub(super) fn encode_payload(
    result: &ImageAnalysisResult,
    dependencies: &Dependencies,
    maximum: usize,
) -> Result<Vec<u8>, RuntimeError> {
    let mut out = crate::intent_codec::primitives::Writer::new(maximum);
    out.push(1).map_err(|_| RuntimeError::InvalidRecord)?;
    out.push(payload_validity(result))
        .map_err(|_| RuntimeError::InvalidRecord)?;
    write_result(&mut out, Some(result), maximum)?;
    write_dependencies(&mut out, dependencies, maximum)?;
    Ok(out.finish())
}
pub(super) fn decode_payload(
    bytes: &[u8],
) -> Result<(ImageAnalysisResult, Dependencies), RuntimeError> {
    if bytes.len() > MAXIMUM_RESPONSE_BYTES {
        return Err(RuntimeError::InvalidRecord);
    }
    let mut input = crate::intent_codec::primitives::Reader::new(bytes);
    if input.byte().map_err(|_| RuntimeError::InvalidRecord)? != 1 {
        return Err(RuntimeError::InvalidRecord);
    }
    let validity = input.byte().map_err(|_| RuntimeError::InvalidRecord)?;
    let result =
        read_result(&mut input, MAXIMUM_RESPONSE_BYTES)?.ok_or(RuntimeError::InvalidRecord)?;
    if validity != payload_validity(&result) {
        return Err(RuntimeError::InvalidRecord);
    }
    let dependencies = read_dependencies(&mut input, MAXIMUM_RESPONSE_BYTES)?;
    input.finish().map_err(|_| RuntimeError::InvalidRecord)?;
    Ok((result, dependencies))
}
pub(super) fn encode_raw(
    response: &RawAnalysis,
    dependencies: &Dependencies,
    previous: Option<&ImageAnalysisResult>,
    prompt_index: usize,
    maximum: usize,
) -> Result<Vec<u8>, RuntimeError> {
    if maximum > MAXIMUM_RESPONSE_BYTES || response.body().len() > maximum {
        return Err(RuntimeError::UnsupportedCapability);
    }
    let envelope = maximum
        .checked_mul(5)
        .and_then(|size| size.checked_add(128))
        .ok_or(RuntimeError::InvalidConfiguration)?;
    let mut out = crate::intent_codec::primitives::Writer::new(envelope);
    out.push(1).map_err(|_| RuntimeError::InvalidRecord)?;
    match &response.response {
        AnalysisResponse::Http { status, body } => {
            out.push(1).map_err(|_| RuntimeError::InvalidRecord)?;
            out.u16_be(*status)
                .map_err(|_| RuntimeError::InvalidRecord)?;
            out.counted(body).map_err(|_| RuntimeError::InvalidRecord)?;
        }
        AnalysisResponse::Transport(failure) => {
            out.push(0).map_err(|_| RuntimeError::InvalidRecord)?;
            out.push(match failure {
                AnalysisTransportFailure::Transport => 0,
                AnalysisTransportFailure::Deadline => 1,
            })
            .map_err(|_| RuntimeError::InvalidRecord)?;
        }
    }
    out.boolean(response.segmentation.is_some())
        .map_err(|_| RuntimeError::InvalidRecord)?;
    if let Some((width, height)) = response.segmentation {
        out.u32_be(width).map_err(|_| RuntimeError::InvalidRecord)?;
        out.u32_be(height)
            .map_err(|_| RuntimeError::InvalidRecord)?;
    }
    out.u32_be(u32::try_from(prompt_index).map_err(|_| RuntimeError::InvalidRecord)?)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    write_result(&mut out, previous, maximum)?;
    write_dependencies(&mut out, dependencies, maximum)?;
    Ok(out.finish())
}
pub(super) fn decode_raw(
    bytes: &[u8],
) -> Result<
    (
        RawAnalysis,
        Dependencies,
        Option<ImageAnalysisResult>,
        usize,
    ),
    RuntimeError,
> {
    if bytes.len() > MAXIMUM_RAW_ENVELOPE_BYTES {
        return Err(RuntimeError::InvalidRecord);
    }
    let mut input = crate::intent_codec::primitives::Reader::new(bytes);
    if input.byte().map_err(|_| RuntimeError::InvalidRecord)? != 1 {
        return Err(RuntimeError::InvalidRecord);
    }
    let response = match input.byte().map_err(|_| RuntimeError::InvalidRecord)? {
        1 => {
            let status = input.u16_be().map_err(|_| RuntimeError::InvalidRecord)?;
            if !(100..=599).contains(&status) {
                return Err(RuntimeError::InvalidRecord);
            }
            let body = input
                .counted(MAXIMUM_RESPONSE_BYTES)
                .map_err(|_| RuntimeError::InvalidRecord)?
                .to_vec();
            AnalysisResponse::Http { status, body }
        }
        0 => AnalysisResponse::Transport(
            match input.byte().map_err(|_| RuntimeError::InvalidRecord)? {
                0 => AnalysisTransportFailure::Transport,
                1 => AnalysisTransportFailure::Deadline,
                _ => return Err(RuntimeError::InvalidRecord),
            },
        ),
        _ => return Err(RuntimeError::InvalidRecord),
    };
    let segmentation = if input.boolean().map_err(|_| RuntimeError::InvalidRecord)? {
        Some((
            input.u32_be().map_err(|_| RuntimeError::InvalidRecord)?,
            input.u32_be().map_err(|_| RuntimeError::InvalidRecord)?,
        ))
    } else {
        None
    };
    let prompt_index = usize::try_from(input.u32_be().map_err(|_| RuntimeError::InvalidRecord)?)
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let previous = read_result(&mut input, MAXIMUM_RESPONSE_BYTES)?;
    let dependencies = read_dependencies(&mut input, MAXIMUM_RESPONSE_BYTES)?;
    input.finish().map_err(|_| RuntimeError::InvalidRecord)?;
    Ok((
        RawAnalysis {
            response,
            segmentation,
        },
        dependencies,
        previous,
        prompt_index,
    ))
}
pub(super) fn interpret(
    response: &RawAnalysis,
    intent: &DecodedAnalysis,
    dependencies: &Dependencies,
    previous: Option<&ImageAnalysisResult>,
    prompt_index: usize,
    maximum: usize,
) -> Result<ImageAnalysisResult, RuntimeError> {
    if let ImageAnalysisOperation::PhotoInspection(spec) = &intent.operation {
        let assessment = match &response.response {
            AnalysisResponse::Transport(failure) => backend::unjudged_photo(
                spec,
                match failure {
                    AnalysisTransportFailure::Transport => PhotoInspectionUnjudgedReason::Transport,
                    AnalysisTransportFailure::Deadline => PhotoInspectionUnjudgedReason::Deadline,
                },
            ),
            AnalysisResponse::Http { status, .. } if *status != 200 => {
                backend::unjudged_photo(spec, PhotoInspectionUnjudgedReason::Provider)
            }
            AnalysisResponse::Http { body, .. } => match analysis::parse_response(body, maximum)
                .and_then(|value| backend::interpret_photo(&value, spec))
            {
                Ok(assessment) => assessment,
                Err(_) => {
                    backend::unjudged_photo(spec, PhotoInspectionUnjudgedReason::InvalidResponse)
                }
            },
        };
        return Ok(ImageAnalysisResult::PhotoInspection {
            source: spec.source.clone(),
            assessment,
        });
    }
    if response.status() != Some(200) {
        return Err(RuntimeError::ProviderRefused);
    }
    let value = analysis::parse_response(response.body(), maximum)?;
    if let ImageAnalysisOperation::Segmentation(spec) = &intent.operation {
        let (width, height) = response.segmentation.ok_or(RuntimeError::InvalidRecord)?;
        let prompt = *spec
            .prompts
            .get(prompt_index)
            .ok_or(RuntimeError::InvalidRecord)?;
        let mut masks = match previous {
            None if prompt_index == 0 => Vec::new(),
            Some(ImageAnalysisResult::Segmentation {
                source,
                contract_revision,
                masks,
            }) if source == &spec.source
                && *contract_revision == spec.contract_revision
                && masks.len() == prompt_index =>
            {
                masks.clone()
            }
            _ => return Err(RuntimeError::InvalidRecord),
        };
        masks.push(backend::parse_segmentation(
            &value,
            prompt,
            width,
            height,
            spec.maximum_masks,
        )?);
        Ok(ImageAnalysisResult::Segmentation {
            source: spec.source.clone(),
            contract_revision: spec.contract_revision,
            masks,
        })
    } else {
        backend::interpret_chat(
            &value,
            &intent.operation,
            dependencies.segmentation.as_ref(),
            dependencies.locations.as_ref(),
            dependencies.identity.as_ref(),
        )
    }
}
pub(super) fn http_failure(status: u16, body: &[u8]) -> FailureCause {
    match status {
        429 if super::super::quota_gate::raw_answer(429, body, false)
            == super::super::quota::QuotaAnswer::Exhausted =>
        {
            FailureCause::QuotaExhausted
        }
        429 => FailureCause::RateLimited,
        400..=499 => FailureCause::Refused,
        200..=299 => FailureCause::InvalidOutput,
        _ => FailureCause::Provider,
    }
}
pub(super) fn response_failure(response: &RawAnalysis) -> FailureCause {
    match &response.response {
        AnalysisResponse::Transport(AnalysisTransportFailure::Deadline) => {
            FailureCause::DeadlineExceeded
        }
        AnalysisResponse::Transport(AnalysisTransportFailure::Transport) => {
            FailureCause::TransportLost
        }
        AnalysisResponse::Http { status, body } => http_failure(*status, body),
    }
}
pub(super) fn supplier_failure(cause: FailureCause) -> RuntimeError {
    match cause {
        FailureCause::QuotaExhausted => RuntimeError::SupplierQuotaExhausted,
        FailureCause::RateLimited => RuntimeError::SupplierRateLimited,
        FailureCause::Refused => RuntimeError::ProviderRefused,
        _ => RuntimeError::SupplierFailure(cause),
    }
}

pub(super) fn validate_raw_usage(
    response: &RawAnalysis,
    usage: &RuntimeUsage,
) -> Result<(), RuntimeError> {
    let (evidence, units) = if response.segmentation.is_some() {
        (UsageEvidence::Estimated, usage.route.maxima.clone())
    } else {
        analysis::parse_response(response.body(), response.body().len())
            .map(|value| analysis::reported_usage(&value))
            .unwrap_or((UsageEvidence::Unknown, Vec::new()))
    };
    let units = if evidence == UsageEvidence::Unknown {
        usage.route.maxima.clone()
    } else {
        units
    };
    if evidence != usage.evidence || units != usage.units {
        return Err(RuntimeError::InvalidRecord);
    }
    Ok(())
}

pub(in crate::runtime) fn validate_standalone_evidence(
    job: &ImageJob,
    retained: &super::PublicationDecisionEvidence,
) -> Result<super::DecisionValidity, RuntimeError> {
    let intent = analysis_codec::decode_analysis(job.intent.bytes())
        .map_err(|_| RuntimeError::InvalidRecord)?;
    let decision = &retained.decision;
    let usage = &retained.usage;
    let config = super::retained_config(job)?;
    if retained.source_candidate.is_some()
        || intent.owner != job.owner
        || usage.route.model != cost_basis(job, &config)?.model.as_str()
        || decision.basis.stage.kind != kind(&intent.operation)
        || decision.basis.stage.input_revision != job.intent.revision()
        || analysis_codec::sources(&intent.operation)
            .iter()
            .any(|source| intent.audience.meet(&source.audience) != intent.audience)
        || decision.scope.owner != job.owner
        || decision.scope.job != job.job
        || usage.owner != job.owner
        || usage.job != job.job
        || usage.stage != decision.scope.stage
        || usage.dispatch != decision.scope.dispatch
        || usage.revision.0 != u64::from(decision.receipt.usage_revision)
        || decision.kind != decision_kind(&intent.operation)
        || decision.basis.schema_revision != 1
        || decision.basis.validator_revision != 1
        || decision.basis.stage.input_digest != <[u8; 32]>::from(Sha256::digest(job.intent.bytes()))
        || decision.basis.stage.config_digest
            != <[u8; 32]>::from(Sha256::digest(job.config.bytes()))
    {
        return Err(RuntimeError::InvalidRecord);
    }
    let (response, dependencies, previous, prompt_index) = decode_raw(
        usage
            .raw_output
            .as_deref()
            .ok_or(RuntimeError::InvalidRecord)?,
    )?;
    validate_raw_usage(&response, usage)?;
    let actual = interpret(
        &response,
        &intent,
        &dependencies,
        previous.as_ref(),
        prompt_index,
        response.body().len(),
    )?;
    if usage.usable != (payload_validity(&actual) == 1)
        || encode_payload(&actual, &dependencies, decision.payload().len())? != decision.payload()
    {
        return Err(RuntimeError::InvalidRecord);
    }
    if let (
        ImageAnalysisOperation::PhotoInspection(spec),
        ImageAnalysisResult::PhotoInspection { assessment, .. },
    ) = (&intent.operation, &actual)
    {
        let restored = backend::restore_photo_assessment(assessment, &spec.inspection)
            .map_err(|_| RuntimeError::InvalidRecord)?;
        if matches!(
            restored.verdict,
            super::inspection_policy::InspectionVerdict::Unjudged(_)
        ) {
            return Ok(super::DecisionValidity::Unjudged);
        }
    }
    Ok(super::DecisionValidity::Useful)
}

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;
