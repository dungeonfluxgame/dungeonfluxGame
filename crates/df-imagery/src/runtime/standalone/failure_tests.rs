//! Negative economic evidence never accepts a useful response relabelled as failed.
use super::*;
use crate::cost::{CostRoute, CostUnits, UsageRevision};
use crate::*;

fn fixture() -> (ImageJob, DispatchView, StageBasis, RuntimeUsage) {
    let owner = ImageOwner::Draft {
        account: df_types::AccountId::from_bytes(&[1; 16]).unwrap(),
    };
    let source = SourceRef {
        asset: AssetId::from_bytes(&[2; 16]).unwrap(),
        digest: AssetDigest::from_bytes([3; 32]),
        audience: Audience::PublicStatic,
    };
    let intent = DecodedAnalysis {
        owner: owner.clone(),
        purpose: PurposedUse {
            purpose: Purpose::ReferencePrep,
            role: VariantRole::Reference,
        },
        operation: ImageAnalysisOperation::HeadLocations(HeadLocationsSpec {
            source,
            face: None,
            targets: vec![AnalysisTarget {
                id: PrivateImageText::new("hero-1"),
                description: PrivateImageText::new("An adult fictional dwarf"),
            }],
            contract_revision: 1,
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        audience: Audience::PublicStatic,
        expected_source_revision: 1,
    };
    let mut config = ImageConfig::defaults().unwrap();
    config.analysis.head_locations = Some(AnalysisRoute {
        account: AccountHandle::new("test-head").unwrap(),
        model: ModelName::new("test-model").unwrap(),
        endpoint: ApiEndpoint::new("http://127.0.0.1:9/unused").unwrap(),
        maximum_output_tokens: 100,
    });
    let encoded = crate::intent_codec::encode::encode_config(&config, 1_048_576).unwrap();
    let job = ImageJob {
        job: ImageJobId::from_bytes(&[4; 16]).unwrap(),
        owner: owner.clone(),
        sharing: SharingIdentity::from_bytes(&[5; 32]).unwrap(),
        intent: VersionedSnapshot::new(
            u32::from(analysis_codec::VERSION),
            analysis_codec::encode_analysis(&intent).unwrap(),
            1_048_576,
        )
        .unwrap(),
        config: VersionedSnapshot::new(
            u32::from(encoded.version()),
            encoded.bytes().to_vec(),
            1_048_576,
        )
        .unwrap(),
        absolute_deadline_ms: 10_000,
        state: JobState::Planning,
        revision: 4,
        resolved_key: None,
        recipe: None,
        created_at_ms: 1,
        updated_at_ms: 3,
    };
    let stage = ExecutionStageId::from_bytes(&[6; 16]).unwrap();
    let dispatch = DispatchView {
        owner: owner.clone(),
        job: job.job,
        stage,
        dispatch: DispatchId::from_bytes(&[7; 16]).unwrap(),
        sequence: 1,
        state: DispatchState::Failed,
        cause: Some(FailureCause::InvalidOutput),
        settlement: DispatchSettlement::Held,
        finance: None,
        local_accounting: true,
        supplier_job_id: None,
        usage_revision: 1,
        book_call_id: crate::cost::pinned_call_id(&[7; 16]),
        lineage_parent: None,
        claim_generation: 1,
        created_at_ms: 1,
        updated_at_ms: 3,
    };
    let basis = StageBasis {
        kind: StageKind::HeadLocations,
        input_revision: job.intent.revision(),
        input_digest: Sha256::digest(job.intent.bytes()).into(),
        config_revision: 1,
        config_digest: Sha256::digest(job.config.bytes()).into(),
        parent_dispatch: None,
    };
    let raw = RawAnalysis { response:AnalysisResponse::Http { status:200, body:serde_json::to_vec(&serde_json::json!({"choices":[{"message":{"content":"{\"heads\":\"invalid-schema\"}"}}],"usage":{"prompt_tokens":7,"completion_tokens":3}})).unwrap() }, segmentation:None };
    let usage = RuntimeUsage {
        owner,
        job: job.job,
        stage,
        dispatch: dispatch.dispatch,
        revision: UsageRevision(1),
        route: CostRoute {
            provider: "openai".to_owned(),
            model: "test-model".to_owned(),
            cause: "head_location".to_owned(),
            detail: "head_locator".to_owned(),
            maxima: vec![CostUnits {
                category: "text_input".to_owned(),
                quantity: 100,
            }],
        },
        evidence: UsageEvidence::Reported,
        units: vec![
            CostUnits {
                category: "text_input".to_owned(),
                quantity: 7,
            },
            CostUnits {
                category: "text_output".to_owned(),
                quantity: 3,
            },
        ],
        usable: false,
        at_ms: 3,
        raw_output: Some(encode_raw(&raw, &Dependencies::default(), None, 0, 4096).unwrap()),
    };
    (job, dispatch, basis, usage)
}

#[test]
fn actual_failed_schema_binds_exact_reported_usage_route_source_and_basis() {
    let (job, dispatch, basis, usage) = fixture();
    validate_standalone_failure(&job, &dispatch, &basis, &usage).unwrap();
    let mut changed = usage.clone();
    changed.units[0].quantity += 1;
    assert_eq!(
        validate_standalone_failure(&job, &dispatch, &basis, &changed),
        Err(RuntimeError::InvalidRecord)
    );
    changed = usage.clone();
    changed.evidence = UsageEvidence::Unknown;
    assert_eq!(
        validate_standalone_failure(&job, &dispatch, &basis, &changed),
        Err(RuntimeError::InvalidRecord)
    );
    changed = usage.clone();
    changed.route.model = "another-model".to_owned();
    assert_eq!(
        validate_standalone_failure(&job, &dispatch, &basis, &changed),
        Err(RuntimeError::InvalidRecord)
    );
    let mut different = basis.clone();
    different.input_digest[0] ^= 1;
    assert_eq!(
        validate_standalone_failure(&job, &dispatch, &different, &usage),
        Err(RuntimeError::InvalidRecord)
    );
    different = basis;
    different.parent_dispatch = Some(dispatch.dispatch);
    assert_eq!(
        validate_standalone_failure(&job, &dispatch, &different, &usage),
        Err(RuntimeError::InvalidRecord)
    );
}

#[test]
fn a_useful_head_response_and_forged_failure_cause_cannot_release_liability() {
    let (job, mut dispatch, basis, mut usage) = fixture();
    let (mut response, deps, previous, index) =
        decode_raw(usage.raw_output.as_deref().unwrap()).unwrap();
    let content = serde_json::json!({"heads":[{"id":"hero-1","left":0.1,"top":0.1,"right":0.2,"bottom":0.3}]}).to_string();
    response.response = AnalysisResponse::Http { status:200, body:serde_json::to_vec(&serde_json::json!({"choices":[{"message":{"content":content}}],"usage":{"prompt_tokens":7,"completion_tokens":3}})).unwrap() };
    usage.raw_output = Some(encode_raw(&response, &deps, previous.as_ref(), index, 4096).unwrap());
    assert_eq!(
        validate_standalone_failure(&job, &dispatch, &basis, &usage),
        Err(RuntimeError::InvalidRecord)
    );
    let (job, _, basis, usage) = fixture();
    dispatch.cause = Some(FailureCause::Refused);
    assert_eq!(
        validate_standalone_failure(&job, &dispatch, &basis, &usage),
        Err(RuntimeError::InvalidRecord)
    );
}
