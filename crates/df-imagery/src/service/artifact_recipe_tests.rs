use super::*;
use crate::*;
use df_types::{CampaignId, OperationId, RunId};

fn request(run_byte: u8) -> ImageRequest {
    let campaign = CampaignId::from_bytes(&[1; 16]).unwrap();
    let run = RunId::from_bytes(&[run_byte; 16]).unwrap();
    ImageRequest {
        identity: OperationIdentity {
            owner: ImageOwner::Run { campaign, run },
            operation: OperationId::from_bytes(&[run_byte; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::CanonicalPerson,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("the complete stable public hero prompt", 0xdf02_0001)
                .unwrap()
                .with_artwork(ArtworkBinding {
                    spec: CampaignArtworkSpec {
                        campaign,
                        subject: ArtworkSubject::Hero(
                            ArtworkSubjectKey::new("library/committed-public-hero").unwrap(),
                        ),
                        public_revision: [2; 32],
                    },
                    selection: RunSelectionFence {
                        run,
                        binding: [run_byte; 32],
                    },
                }),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        audience: Audience::CampaignShared(campaign),
        expected_source_revision: 7,
        output: OutputProfile::provider(
            Dimensions::new(1024, 1024).unwrap(),
            RasterOutputSpec::new(
                Dimensions::new(2, 3).unwrap(),
                AlphaHandling::Opaque,
                Encoding::Png,
            )
            .unwrap(),
        ),
        urgency: Urgency::Visible,
    }
}

fn config(config: &ImageConfig) -> VersionedSnapshot {
    let encoded = crate::intent_codec::encode_config(config, PROTOCOL_MAX_SNAPSHOT_BYTES).unwrap();
    VersionedSnapshot::new(
        u32::from(encoded.version()),
        encoded.bytes().to_vec(),
        PROTOCOL_MAX_SNAPSHOT_BYTES,
    )
    .unwrap()
}

fn recipe(request: &ImageRequest, config: &ImageConfig) -> CampaignArtifactRecipe {
    campaign_artifact_recipe(request, &self::config(config))
        .unwrap()
        .unwrap()
}

#[test]
fn separate_run_selection_keeps_full_campaign_semantics_equal() {
    let config = ImageConfig::defaults().unwrap();
    let first = request(3);
    let second = request(4);
    assert_ne!(first.identity.owner, second.identity.owner);
    assert_ne!(first.operation.artwork(), second.operation.artwork());
    assert_eq!(recipe(&first, &config), recipe(&second, &config));
}

#[test]
fn complete_actual_prompt_and_public_revision_remain_semantic() {
    let config = ImageConfig::defaults().unwrap();
    let original = request(3);
    let expected = recipe(&original, &config);
    let mut changed = original.clone();
    let ImageOperation::Generate(spec) = &mut changed.operation else {
        unreachable!()
    };
    let binding = spec.prompt.artwork().unwrap().clone();
    spec.prompt = TextPrompt::new(
        "different actual provider text",
        spec.prompt.template_version,
    )
    .unwrap()
    .with_artwork(binding);
    assert_ne!(recipe(&changed, &config), expected);
    let mut changed = original.clone();
    let ImageOperation::Generate(spec) = &mut changed.operation else {
        unreachable!()
    };
    let mut binding = spec.prompt.artwork().unwrap().clone();
    binding.spec.public_revision = [8; 32];
    spec.prompt = spec.prompt.clone().with_artwork(binding);
    assert_ne!(recipe(&changed, &config), expected);
}

#[test]
fn source_fence_model_account_and_creator_route_remain_semantic() {
    let config = ImageConfig::defaults().unwrap();
    let original = request(3);
    let expected = recipe(&original, &config);
    let mut changed = original.clone();
    changed.expected_source_revision += 1;
    assert_ne!(recipe(&changed, &config), expected);
    let mut changed_config = config.clone();
    changed_config.backends[BackendId::OpenAi.index()].model =
        Some(crate::config::ModelName::new("different-model").unwrap());
    assert_ne!(recipe(&original, &changed_config), expected);
    let mut changed_config = config.clone();
    changed_config.backends[BackendId::OpenAi.index()].account =
        AccountHandle::new("other").unwrap();
    assert_ne!(recipe(&original, &changed_config), expected);
    let mut changed_config = config.clone();
    changed_config.creator_bfl_account = Some(AccountHandle::new("alternate").unwrap());
    assert_ne!(recipe(&original, &changed_config), expected);
}

#[test]
fn only_operational_modes_and_capacity_are_canonicalized() {
    let config = ImageConfig::defaults().unwrap();
    let original = request(3);
    let expected = recipe(&original, &config);
    let mut changed = original.clone();
    changed.execution = ExecutionMode::PreparedOnly;
    let mut changed_config = config.clone();
    changed_config.mode = GlobalMode::CacheOnly;
    changed_config.backends[BackendId::OpenAi.index()].enabled = false;
    changed_config.backends[BackendId::OpenAi.index()].max_concurrent_jobs += 1;
    changed_config.scheduler.max_pending_jobs += 1;
    changed_config.quota.cooldown_ms += 1;
    assert_eq!(recipe(&changed, &changed_config), expected);
}

#[test]
fn legacy_no_spec_is_ineligible_instead_of_rewriting_prompt_bytes() {
    let mut legacy = request(3);
    let ImageOperation::Generate(spec) = &mut legacy.operation else {
        unreachable!()
    };
    spec.prompt = TextPrompt::new(spec.prompt.text(), spec.prompt.template_version).unwrap();
    let config = self::config(&ImageConfig::defaults().unwrap());
    assert_eq!(campaign_artifact_recipe(&legacy, &config).unwrap(), None);
}
