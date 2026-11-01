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
mod tests {
    use super::*;
    fn source() -> crate::SourceRef {
        crate::SourceRef {
            asset: crate::AssetId::from_bytes(&[1; 16]).unwrap(),
            digest: crate::AssetDigest::from_bytes([2; 32]),
            audience: crate::Audience::PublicStatic,
        }
    }
    fn intent() -> DecodedAnalysis {
        DecodedAnalysis {
            owner: crate::ImageOwner::Draft {
                account: df_types::AccountId::from_bytes(&[3; 16]).unwrap(),
            },
            purpose: crate::PurposedUse {
                purpose: crate::Purpose::ScenePlate,
                role: crate::VariantRole::Reference,
            },
            operation: ImageAnalysisOperation::HeadLocations(HeadLocationsSpec {
                source: source(),
                face: None,
                targets: vec![AnalysisTarget {
                    id: crate::PrivateImageText::new("hero-1"),
                    description: crate::PrivateImageText::new("A dwarf wearing a red cloak"),
                }],
                contract_revision: 1,
            }),
            execution: crate::ExecutionMode::Live,
            freshness: crate::Freshness::ReuseCompatible,
            audience: crate::Audience::PublicStatic,
            expected_source_revision: 0,
        }
    }
    fn response() -> RawAnalysis {
        let body=serde_json::to_vec(&serde_json::json!({"choices":[{"message":{"content":"{\"heads\":[{\"id\":\"hero-1\",\"left\":0.1,\"top\":0.1,\"right\":0.2,\"bottom\":0.3}]}"}}],"usage":{"prompt_tokens":12,"completion_tokens":8}})).unwrap();
        RawAnalysis {
            response: AnalysisResponse::Http { status: 200, body },
            segmentation: None,
        }
    }
    #[test]
    fn retained_raw_replays_the_exact_typed_head_decision_without_new_transport() {
        let intent = intent();
        let deps = Dependencies::default();
        let original = response();
        let result = interpret(&original, &intent, &deps, None, 0, 4096).unwrap();
        let bytes = encode_raw(&original, &deps, None, 0, 4096).unwrap();
        let (restored, deps, previous, index) = decode_raw(&bytes).unwrap();
        assert_eq!(
            interpret(&restored, &intent, &deps, previous.as_ref(), index, 4096).unwrap(),
            result
        );
        let payload = encode_payload(&result, &deps, 4096).unwrap();
        assert_eq!(decode_payload(&payload).unwrap().0, result);
        assert_eq!(payload.get(1), Some(&1));
        let mut forged = payload;
        forged[1] = 0;
        assert_eq!(
            decode_payload(&forged).unwrap_err(),
            RuntimeError::InvalidRecord
        );
    }
    #[test]
    fn oversized_body_truncated_envelope_and_trailing_bytes_never_become_decisions() {
        let deps = Dependencies::default();
        let original = response();
        assert!(encode_raw(&original, &deps, None, 0, original.body().len() - 1).is_err());
        let bytes = encode_raw(&original, &deps, None, 0, 4096).unwrap();
        for length in 0..bytes.len() {
            assert!(decode_raw(&bytes[..length]).is_err());
        }
        let mut bytes = bytes;
        bytes.push(0);
        assert!(decode_raw(&bytes).is_err());
    }
    #[test]
    fn provider_refusal_and_malformed_head_response_are_preserved_as_failures() {
        let intent = intent();
        let deps = Dependencies::default();
        let mut raw = response();
        raw.response = AnalysisResponse::Http {
            status: 429,
            body: raw.body().to_vec(),
        };
        assert_eq!(
            interpret(&raw, &intent, &deps, None, 0, 4096).unwrap_err(),
            RuntimeError::ProviderRefused
        );
        raw.response = AnalysisResponse::Http {
            status: 200,
            body: br#"{"choices":[]}"#.to_vec(),
        };
        assert_eq!(
            interpret(&raw, &intent, &deps, None, 0, 4096).unwrap_err(),
            RuntimeError::InvalidRecord
        );
    }
    #[test]
    fn an_unjudged_photo_cannot_advertise_useful_supplier_work_in_its_header() {
        let result = ImageAnalysisResult::PhotoInspection {
            source: source(),
            assessment: PhotoInspectionAssessment {
                contract_revision: 1,
                verdict: PhotoInspectionVerdict::Unjudged(
                    PhotoInspectionUnjudgedReason::InvalidResponse,
                ),
                visible_features: vec![],
                source_binding: None,
            },
        };
        let payload = encode_payload(&result, &Dependencies::default(), 4096).unwrap();
        assert_eq!(payload.get(1), Some(&0));
        assert_eq!(decode_payload(&payload).unwrap().0, result);
        let mut forged = payload;
        forged[1] = 1;
        assert_eq!(
            decode_payload(&forged).unwrap_err(),
            RuntimeError::InvalidRecord
        );
    }
    #[test]
    fn photo_transport_provider_and_schema_failures_replay_as_explicit_unjudged_results() {
        let mut intent = intent();
        intent.operation = ImageAnalysisOperation::PhotoInspection(PhotoInspectionSpec {
            source: source(),
            references: vec![],
            inspection: crate::ImageInspection {
                route: crate::AnalysisRoute {
                    account: crate::AccountHandle::new("analysis-fixture").unwrap(),
                    model: crate::ModelName::new("vision-fixture").unwrap(),
                    endpoint: crate::ApiEndpoint::new("http://127.0.0.1:9/unused").unwrap(),
                    maximum_output_tokens: 500,
                },
                requirement: crate::InspectionRequirement::BestEffort,
                contract_revision: 1,
                kind: crate::InspectionKind::ScenePlate,
                expected_heroes: None,
                party_reference: None,
                hand_custody: crate::PrivateImageText::new(""),
                staging: vec![],
                maximum_corrections: 0,
                allow_visible_text: false,
            },
        });
        for (response, expected) in [
            (
                AnalysisResponse::Transport(AnalysisTransportFailure::Transport),
                PhotoInspectionUnjudgedReason::Transport,
            ),
            (
                AnalysisResponse::Transport(AnalysisTransportFailure::Deadline),
                PhotoInspectionUnjudgedReason::Deadline,
            ),
            (
                AnalysisResponse::Http {
                    status: 503,
                    body: vec![],
                },
                PhotoInspectionUnjudgedReason::Provider,
            ),
            (
                AnalysisResponse::Http {
                    status: 200,
                    body: b"invalid json".to_vec(),
                },
                PhotoInspectionUnjudgedReason::InvalidResponse,
            ),
        ] {
            let raw = RawAnalysis {
                response,
                segmentation: None,
            };
            let encoded = encode_raw(&raw, &Dependencies::default(), None, 0, 4096).unwrap();
            let (restored, deps, prior, index) = decode_raw(&encoded).unwrap();
            let result = interpret(&restored, &intent, &deps, prior.as_ref(), index, 4096).unwrap();
            assert!(
                matches!(&result,ImageAnalysisResult::PhotoInspection {assessment,..} if assessment.verdict==PhotoInspectionVerdict::Unjudged(expected))
            );
            assert_eq!(
                encode_payload(&result, &deps, 4096).unwrap().get(1),
                Some(&0)
            );
        }
    }

    #[test]
    fn missing_usage_replay_accepts_normalized_unknown_maxima_and_refuses_changed_exposure() {
        let mut response = response();
        let mut value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        value.as_object_mut().unwrap().remove("usage");
        response.response = AnalysisResponse::Http {
            status: 200,
            body: serde_json::to_vec(&value).unwrap(),
        };
        let parsed = analysis::parse_response(response.body(), 4096).unwrap();
        assert_eq!(
            analysis::reported_usage(&parsed),
            (UsageEvidence::Unknown, Vec::new())
        );
        let dependencies = Dependencies::default();
        let encoded = encode_raw(&response, &dependencies, None, 0, 4096).unwrap();
        let (retained, dependencies, previous, index) = decode_raw(&encoded).unwrap();
        assert!(
            interpret(
                &retained,
                &intent(),
                &dependencies,
                previous.as_ref(),
                index,
                4096
            )
            .is_ok()
        );
        let maxima = vec![
            crate::cost::CostUnits {
                category: "text_input".to_owned(),
                quantity: 100,
            },
            crate::cost::CostUnits {
                category: "text_output".to_owned(),
                quantity: 100,
            },
        ];
        let mut usage = RuntimeUsage {
            owner: intent().owner,
            job: crate::ImageJobId::from_bytes(&[4; 16]).unwrap(),
            stage: ExecutionStageId::from_bytes(&[5; 16]).unwrap(),
            dispatch: DispatchId::from_bytes(&[6; 16]).unwrap(),
            revision: crate::cost::UsageRevision(1),
            route: crate::cost::CostRoute {
                provider: "openai".to_owned(),
                model: "head-fixture".to_owned(),
                cause: "head_location".to_owned(),
                detail: "head_locator".to_owned(),
                maxima: maxima.clone(),
            },
            evidence: UsageEvidence::Unknown,
            units: maxima.clone(),
            usable: true,
            at_ms: 1,
            raw_output: Some(encoded),
        };
        validate_raw_usage(&retained, &usage).unwrap();
        usage.units.clear();
        assert_eq!(
            validate_raw_usage(&retained, &usage),
            Err(RuntimeError::InvalidRecord)
        );
        usage.units = maxima;
        usage.units[0].quantity += 1;
        assert_eq!(
            validate_raw_usage(&retained, &usage),
            Err(RuntimeError::InvalidRecord)
        );
    }

    #[test]
    fn actual_http_and_transport_failures_keep_their_closed_semantic_causes() {
        let body = br#"{"error":{"code":"insufficient_quota"}}"#;
        assert_eq!(http_failure(429, body), FailureCause::QuotaExhausted);
        for (status, expected) in [
            (429, FailureCause::RateLimited),
            (401, FailureCause::Refused),
            (403, FailureCause::Refused),
            (503, FailureCause::Provider),
            (200, FailureCause::InvalidOutput),
        ] {
            assert_eq!(http_failure(status, b"{}"), expected);
        }
        for (failure, expected) in [
            (
                AnalysisTransportFailure::Deadline,
                FailureCause::DeadlineExceeded,
            ),
            (
                AnalysisTransportFailure::Transport,
                FailureCause::TransportLost,
            ),
        ] {
            assert_eq!(
                response_failure(&RawAnalysis {
                    response: AnalysisResponse::Transport(failure),
                    segmentation: None
                }),
                expected
            );
        }
    }
}
