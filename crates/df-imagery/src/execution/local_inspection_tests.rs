//! Canonical retained-policy proof construction is independent of caller assertions.
use super::*;
use crate::execution::*;
use crate::*;
use df_assets::Audience;
use df_types::{AccountId, OperationId};

fn retained(requirement: InspectionRequirement) -> ImageJob {
    let account = AccountId::from_bytes(&[3; 16]).expect("account");
    let route = AnalysisRoute {
        account: AccountHandle::new("analysis-private-account").expect("account"),
        model: ModelName::new("jev-latest").expect("model"),
        endpoint: ApiEndpoint::new("http://127.0.0.1:55499/v1").expect("endpoint"),
        maximum_output_tokens: 3000,
    };
    let pipeline = ImagePipeline {
        revision: 1,
        moderation: None,
        rating: ImageContentRating::Restrained,
        creator: None,
        inspection: Some(ImageInspection {
            requirement,
            route: route.clone(),
            contract_revision: 1,
            kind: InspectionKind::ScenePlate,
            expected_heroes: None,
            party_reference: None,
            hand_custody: PrivateImageText::new("private inspection text"),
            staging: vec![],
            maximum_corrections: 1,
            allow_visible_text: false,
        }),
    };
    let dimensions = Dimensions::new(1, 1).expect("dimensions");
    let request = ImageRequest {
        identity: OperationIdentity {
            owner: ImageOwner::Draft { account },
            operation: OperationId::from_bytes(&[4; 16]).expect("op"),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("private prompt", 17)
                .expect("prompt")
                .with_pipeline(pipeline)
                .expect("pipeline"),
            references: vec![],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience: Audience::OwnerPrivate(account),
        expected_source_revision: 1,
        output: OutputProfile::Provider {
            generated: dimensions,
            delivered: RasterOutputSpec::new(dimensions, AlphaHandling::Opaque, Encoding::Png)
                .expect("output"),
        },
    };
    let mut config = ImageConfig::defaults().expect("config");
    config.analysis.inspection = Some(route);
    let intent = crate::intent_codec::encode_intent(&request, &config.request_limits, 1 << 20)
        .expect("canonical intent");
    let encoded = crate::intent_codec::encode_config(&config, 1 << 20).expect("canonical config");
    ImageJob {
        job: ImageJobId::from_bytes(&[5; 16]).expect("job"),
        owner: request.identity.owner,
        sharing: SharingIdentity::from_bytes(&[6; 32]).expect("sharing"),
        intent: VersionedSnapshot::new(
            u32::from(intent.version()),
            intent.bytes().to_vec(),
            1 << 20,
        )
        .expect("intent"),
        config: VersionedSnapshot::new(
            u32::from(encoded.version()),
            encoded.bytes().to_vec(),
            1 << 20,
        )
        .expect("config"),
        absolute_deadline_ms: 1000,
        state: JobState::Planning,
        revision: 1,
        resolved_key: None,
        recipe: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}
fn claim(job: &ImageJob) -> StageClaim {
    StageClaim {
        scope: StageScope {
            owner: job.owner.clone(),
            job: job.job,
            stage: ExecutionStageId::from_bytes(&[7; 16]).expect("stage"),
        },
        token: ClaimToken::from_bytes(&[8; 16]).expect("token"),
        generation: 1,
    }
}
#[test]
fn local_unjudged_policy_is_derived_from_retained_best_effort_intent_and_debug_is_private() {
    let job = retained(InspectionRequirement::BestEffort);
    let command = LocalInspectionCommand::new(
        &job,
        claim(&job),
        DispatchId::from_bytes(&[9; 16]).expect("dispatch"),
        [10; 32],
        LocalInspectionReason::Preparation,
        2,
    )
    .expect("retained BestEffort");
    assert_eq!(command.intent_snapshot_revision(), job.intent.revision());
    assert_eq!(command.config_snapshot_revision(), job.config.revision());
    assert_eq!(
        *command.intent_digest(),
        <[u8; 32]>::from(sha2::Sha256::digest(job.intent.bytes()))
    );
    let debug = format!("{command:?}");
    for private in [
        "private prompt",
        "private inspection text",
        "analysis-private-account",
        "08080808",
        "09090909",
    ] {
        assert!(!debug.contains(private), "private metadata leaked");
    }
    let required = retained(InspectionRequirement::Required);
    assert_eq!(
        LocalInspectionCommand::new(
            &required,
            claim(&required),
            DispatchId::from_bytes(&[9; 16]).expect("dispatch"),
            [10; 32],
            LocalInspectionReason::Disabled,
            2
        ),
        Err(LocalInspectionError::NotBestEffort)
    );
}
#[test]
fn local_unjudged_policy_rejects_foreign_claim_and_malformed_retained_snapshot() {
    let mut job = retained(InspectionRequirement::BestEffort);
    let mut wrong = claim(&job);
    wrong.scope.job = ImageJobId::from_bytes(&[11; 16]).expect("foreign job");
    assert_eq!(
        LocalInspectionCommand::new(
            &job,
            wrong,
            DispatchId::from_bytes(&[9; 16]).expect("dispatch"),
            [10; 32],
            LocalInspectionReason::Preparation,
            2
        ),
        Err(LocalInspectionError::InvalidRecord)
    );
    let matching = claim(&job);
    let valid_config = job.config.clone();
    job.config = VersionedSnapshot::new(999, job.config.bytes().to_vec(), 1 << 20)
        .expect("mismatched codec revision");
    assert_eq!(
        LocalInspectionCommand::new(
            &job,
            matching.clone(),
            DispatchId::from_bytes(&[9; 16]).unwrap(),
            [10; 32],
            LocalInspectionReason::Preparation,
            2
        ),
        Err(LocalInspectionError::InvalidRecord)
    );
    job.config = valid_config;
    job.intent = VersionedSnapshot::new(1, vec![255], 1 << 20).expect("opaque snapshot");
    assert_eq!(
        LocalInspectionCommand::new(
            &job,
            matching,
            DispatchId::from_bytes(&[9; 16]).expect("dispatch"),
            [10; 32],
            LocalInspectionReason::Preparation,
            2
        ),
        Err(LocalInspectionError::InvalidRecord)
    );
}
