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
    let heads = r#"{"heads":[{"id":"hero-1","left":0.1,"top":0.1,"right":0.2,"bottom":0.3}]}"#;
    let body = serde_json::to_vec(&serde_json::json!({
        "choices": [{"message": {"content": heads}}],
        "usage": {"prompt_tokens": 12, "completion_tokens": 8},
    }))
    .unwrap();
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
        assert!(matches!(
            &result,
            ImageAnalysisResult::PhotoInspection { assessment, .. }
                if assessment.verdict == PhotoInspectionVerdict::Unjudged(expected)
        ));
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
