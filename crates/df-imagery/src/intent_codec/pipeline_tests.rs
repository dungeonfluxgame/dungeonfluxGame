use super::{CodecError, decode_intent, encode_intent};
use crate::config::{AccountHandle, ApiEndpoint, ModelName};
use crate::output::{
    AlphaHandling, Dimensions, Encoding, LossyQuality, OutputProfile, RasterOutputSpec,
};
use crate::request::*;
use df_assets::Audience;
use df_types::{AccountId, OperationId};

fn route() -> AnalysisRoute {
    AnalysisRoute {
        account: AccountHandle::new("creator-analysis").unwrap(),
        model: ModelName::new("jev-latest").unwrap(),
        endpoint: ApiEndpoint::new("http://127.0.0.1:55499/v1").unwrap(),
        maximum_output_tokens: 3000,
    }
}

fn pipeline() -> ImagePipeline {
    ImagePipeline {
        revision: 1,
        moderation: None,
        rating: ImageContentRating::Restrained,
        creator: Some(CreatorClassification {
            route: route(),
            contract_revision: 3,
            public_appearance: PrivateImageText::new("private public appearance"),
            general_prompt: None,
        }),
        inspection: Some(ImageInspection {
            requirement: crate::InspectionRequirement::BestEffort,
            route: route(),
            contract_revision: 1,
            kind: InspectionKind::ScenePlate,
            expected_heroes: Some(1),
            party_reference: Some(1),
            hand_custody: PrivateImageText::new("private hand custody"),
            staging: vec![StagedIdentity {
                locator_id: PrivateImageText::new("hero-1"),
                reference_id: PrivateImageText::new("portrait-1"),
                public_name: PrivateImageText::new("private name"),
                hero: true,
            }],
            maximum_corrections: 1,
            allow_visible_text: true,
        }),
    }
}

fn request(pipeline: Option<ImagePipeline>) -> ImageRequest {
    let account = AccountId::from_bytes(&[3; 16]).unwrap();
    let mut prompt = TextPrompt::new("an adult fantasy performer", 17).unwrap();
    if let Some(pipeline) = pipeline {
        prompt = prompt.with_pipeline(pipeline).unwrap();
    }
    let dimensions = Dimensions::new(1024, 1024).unwrap();
    ImageRequest {
        identity: OperationIdentity {
            owner: ImageOwner::Draft { account },
            operation: OperationId::from_bytes(&[4; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::CreatorPortrait,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt,
            references: vec![],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience: Audience::OwnerPrivate(account),
        expected_source_revision: 1,
        output: OutputProfile::Provider {
            generated: dimensions,
            delivered: RasterOutputSpec::new(
                dimensions,
                AlphaHandling::Opaque,
                Encoding::Jpeg {
                    quality: LossyQuality::new(88).unwrap(),
                },
            )
            .unwrap(),
        },
    }
}

#[test]
fn pipeline_round_trip_preserves_all_frozen_stage_inputs() {
    let expected = pipeline();
    let encoded = encode_intent(
        &request(Some(expected.clone())),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    assert_eq!(encoded.version(), 5);
    let decoded = decode_intent(encoded.bytes(), &RequestLimits::default(), 1_048_576).unwrap();
    assert_eq!(decoded.operation.pipeline(), Some(&expected));
    let legacy = encode_intent(&request(None), &RequestLimits::default(), 1_048_576).unwrap();
    assert_eq!(legacy.version(), 1);
    assert!(
        decode_intent(legacy.bytes(), &RequestLimits::default(), 1_048_576)
            .unwrap()
            .operation
            .pipeline()
            .is_none()
    );
    assert_ne!(legacy.fingerprint(), encoded.fingerprint());
}

#[test]
fn rating_and_private_stage_facts_change_the_canonical_intent_without_leaking_debug_text() {
    let original = pipeline();
    let first = encode_intent(
        &request(Some(original.clone())),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    let mut changed = original.clone();
    changed.rating = ImageContentRating::Mature;
    let second = encode_intent(
        &request(Some(changed)),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    assert_ne!(first.fingerprint(), second.fingerprint());
    let debug = format!("{original:?}");
    for private in [
        "private public appearance",
        "private hand custody",
        "private name",
        "hero-1",
    ] {
        assert!(!debug.contains(private));
    }
}

#[test]
fn every_truncated_pipeline_is_rejected_and_legacy_trailing_metadata_is_not_accepted() {
    let encoded = encode_intent(
        &request(Some(pipeline())),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    for length in 0..encoded.bytes().len() {
        assert!(
            decode_intent(
                &encoded.bytes()[..length],
                &RequestLimits::default(),
                1_048_576
            )
            .is_err()
        );
    }
    let mut bytes = encoded.bytes().to_vec();
    bytes[0] = 1;
    assert!(matches!(
        decode_intent(&bytes, &RequestLimits::default(), 1_048_576),
        Err(CodecError::Trailing { .. })
    ));
}

#[test]
fn invalid_party_pair_and_oversized_appearance_fail_before_admission() {
    let mut invalid = pipeline();
    invalid.inspection.as_mut().unwrap().party_reference = None;
    assert!(
        TextPrompt::new("portrait", 1)
            .unwrap()
            .with_pipeline(invalid)
            .is_err()
    );
    let mut invalid = pipeline();
    invalid.creator.as_mut().unwrap().public_appearance = PrivateImageText::new(&"x".repeat(8193));
    assert!(
        request(Some(invalid))
            .validate(&RequestLimits::default())
            .is_err()
    );
    let rating_only = ImagePipeline {
        revision: 1,
        moderation: None,
        rating: ImageContentRating::Mature,
        creator: None,
        inspection: None,
    };
    assert!(
        request(Some(rating_only))
            .validate(&RequestLimits::default())
            .is_ok()
    );
}

#[test]
fn current_config_preserves_codex_bounds_and_older_snapshots_use_frozen_defaults() {
    use crate::config::ImageConfig;
    let mut config = ImageConfig::defaults().unwrap();
    config.codex.max_stdout_bytes = 9_000_000;
    config.codex.max_stderr_bytes = 800_000;
    config.codex.max_message_bytes = 750_000;
    let encoded = super::encode_config(&config, 1_048_576).unwrap();
    assert_eq!(encoded.version(), 12);
    assert_eq!(
        super::decode_config(encoded.bytes(), 1_048_576).unwrap(),
        config
    );
    let mut version_four = encoded.bytes()[..encoded.bytes().len() - 124].to_vec();
    version_four[0] = 4;
    let decoded_four = super::decode_config(&version_four, 1_048_576).unwrap();
    assert_eq!(
        decoded_four.codex.max_stdout_bytes,
        config.codex.max_stdout_bytes
    );
    assert_eq!(
        decoded_four.codex.max_stderr_bytes,
        config.codex.max_stderr_bytes
    );
    assert_eq!(decoded_four.codex.max_message_bytes, 1024 * 1024);
    let mut historical = encoded.bytes()[..encoded.bytes().len() - 140].to_vec();
    historical[0] = 3;
    let decoded = super::decode_config(&historical, 1_048_576).unwrap();
    assert_eq!(decoded.codex.max_stdout_bytes, 4 * 1024 * 1024);
    assert_eq!(decoded.codex.max_stderr_bytes, 256 * 1024);
    assert_eq!(decoded.codex.max_message_bytes, 1024 * 1024);
}

#[test]
fn current_config_binds_trusted_analysis_routes_and_version_five_has_none() {
    let mut config = crate::ImageConfig::defaults().unwrap();
    let legacy = super::encode_config(&config, 1_048_576).unwrap();
    let mut v5 = legacy.bytes()[..legacy.bytes().len() - 116].to_vec();
    v5[0] = 5;
    let decoded = super::decode_config(&v5, 1_048_576).unwrap();
    assert_eq!(decoded.analysis.classification, None);
    assert_eq!(decoded.analysis.inspection, None);
    config.analysis.classification = Some(pipeline().creator.unwrap().route);
    config.analysis.inspection = Some(pipeline().inspection.unwrap().route);
    let encoded = super::encode_config(&config, 1_048_576).unwrap();
    assert_eq!(
        super::decode_config(encoded.bytes(), 1_048_576).unwrap(),
        config
    );
    assert_ne!(encoded.fingerprint(), legacy.fingerprint());
    let mut changed = config.clone();
    changed
        .analysis
        .classification
        .as_mut()
        .unwrap()
        .maximum_output_tokens += 1;
    assert_ne!(
        super::encode_config(&changed, 1_048_576)
            .unwrap()
            .fingerprint(),
        encoded.fingerprint()
    );
}

#[test]
fn inspection_requirement_is_explicit_and_old_pipeline_defaults_to_best_effort() {
    let expected = pipeline();
    let encoded = encode_intent(
        &request(Some(expected.clone())),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    assert_eq!(*encoded.bytes().last().unwrap(), 0);
    let mut old = encoded.bytes()[..encoded.bytes().len() - 3].to_vec();
    old[0] = 2;
    let decoded = decode_intent(&old, &RequestLimits::default(), 1_048_576).unwrap();
    assert_eq!(decoded.operation.pipeline(), Some(&expected));
    let mut required = expected.clone();
    required.inspection.as_mut().unwrap().requirement = InspectionRequirement::Required;
    let encoded_required = encode_intent(
        &request(Some(required.clone())),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    assert_ne!(encoded_required.fingerprint(), encoded.fingerprint());
    assert_eq!(
        decode_intent(
            encoded_required.bytes(),
            &RequestLimits::default(),
            1_048_576
        )
        .unwrap()
        .operation
        .pipeline(),
        Some(&required)
    );
    let mut invalid = encoded.bytes().to_vec();
    let requirement = invalid.len() - 3;
    invalid[requirement] = 9;
    assert!(matches!(
        decode_intent(&invalid, &RequestLimits::default(), 1_048_576),
        Err(CodecError::Malformed)
    ));
}

#[test]
fn purpose_model_and_quality_overrides_round_trip_and_version_six_has_none() {
    let mut config = crate::ImageConfig::defaults().unwrap();
    config.analysis.classification = Some(route());
    let legacy = super::encode_config(&config, 1_048_576).unwrap();
    let mut v6 = legacy.bytes()[..legacy.bytes().len() - crate::Purpose::COUNT * 3 - 78].to_vec();
    v6[0] = 6;
    let decoded = super::decode_config(&v6, 1_048_576).unwrap();
    assert_eq!(decoded.analysis, config.analysis);
    assert!(
        decoded
            .purposes
            .iter()
            .all(|purpose| purpose.quality_override.is_none() && purpose.model_override.is_none())
    );
    for quality in [
        crate::GenerationQuality::Low,
        crate::GenerationQuality::Medium,
        crate::GenerationQuality::High,
        crate::GenerationQuality::Auto,
        crate::GenerationQuality::LegacyNumeric(LossyQuality::new(90).unwrap()),
    ] {
        let gate = &mut config.purposes[crate::Purpose::AnimationFrame.index()];
        gate.quality_override = Some(quality);
        gate.model_override = Some(ModelName::new("sunburst-configured").unwrap());
        let encoded = super::encode_config(&config, 1_048_576).unwrap();
        assert_eq!(
            super::decode_config(encoded.bytes(), 1_048_576).unwrap(),
            config
        );
        assert_ne!(encoded.fingerprint(), legacy.fingerprint());
    }
}

#[test]
fn technical_moderation_is_separate_and_older_pipeline_has_none() {
    let plain = request(Some(pipeline()));
    let encoded = encode_intent(&plain, &RequestLimits::default(), 1_048_576).unwrap();
    let mut v3 = encoded.bytes()[..encoded.bytes().len() - 2].to_vec();
    v3[0] = 3;
    let legacy = decode_intent(&v3, &RequestLimits::default(), 1_048_576).unwrap();
    assert_eq!(legacy.operation.pipeline().unwrap().moderation, None);
    let mut low = pipeline();
    low.moderation = Some(ProviderModeration::Low);
    let encoded_low = encode_intent(
        &request(Some(low.clone())),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    assert_eq!(low.rating, ImageContentRating::Restrained);
    assert_ne!(encoded.fingerprint(), encoded_low.fingerprint());
    assert_eq!(
        decode_intent(encoded_low.bytes(), &RequestLimits::default(), 1_048_576)
            .unwrap()
            .operation
            .pipeline(),
        Some(&low)
    );
    let mut invalid = encoded_low.bytes().to_vec();
    let moderation_tag = invalid.len() - 2;
    invalid[moderation_tag] = 9;
    assert!(matches!(
        decode_intent(&invalid, &RequestLimits::default(), 1_048_576),
        Err(CodecError::Malformed)
    ));
}

#[test]
fn configured_moderation_and_preparation_preserve_all_historical_defaults() {
    let mut config = crate::ImageConfig::defaults().unwrap();
    let base = super::encode_config(&config, 1_048_576).unwrap();
    let mut v7 = base.bytes()[..base.bytes().len() - crate::Purpose::COUNT - 78].to_vec();
    v7[0] = 7;
    let old = super::decode_config(&v7, 1_048_576).unwrap();
    assert!(
        old.purposes
            .iter()
            .all(|gate| gate.moderation_override.is_none())
    );
    assert_eq!(old.analysis.preparation.max_pixels, 20_000_000);
    assert_eq!(old.analysis.preparation.max_picture_bytes, 1_048_576);
    assert_eq!(old.analysis.preparation.max_request_bytes, 16_777_216);
    config.purposes[crate::Purpose::AnimationFrame.index()].moderation_override =
        Some(ProviderModeration::Low);
    let with_moderation = super::encode_config(&config, 1_048_576).unwrap();
    let mut v8 = with_moderation.bytes()[..with_moderation.bytes().len() - 78].to_vec();
    v8[0] = 8;
    assert_eq!(super::decode_config(&v8, 1_048_576).unwrap(), config);
    config.analysis.preparation.max_pixels = 30_000_000;
    config.analysis.preparation.max_picture_bytes = 2_000_000;
    config.analysis.preparation.max_request_bytes = 30_000_000;
    let current = super::encode_config(&config, 1_048_576).unwrap();
    assert_eq!(
        super::decode_config(current.bytes(), 1_048_576).unwrap(),
        config
    );
    assert_ne!(current.fingerprint(), with_moderation.fingerprint());
    config.analysis.preparation.max_pixels = 0;
    assert!(super::encode_config(&config, 1_048_576).is_err());
    let mut invalid = base.bytes().to_vec();
    let moderation_at = invalid.len() - 78 - crate::Purpose::COUNT;
    invalid[moderation_at] = 9;
    assert!(matches!(
        super::decode_config(&invalid, 1_048_576),
        Err(CodecError::Malformed)
    ));
}

#[test]
fn distinct_creator_general_prompt_is_bound_private_and_historically_absent() {
    let old = pipeline();
    let base = encode_intent(
        &request(Some(old.clone())),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    for (version, removed) in [(2, 3), (3, 2), (4, 1)] {
        let mut bytes = base.bytes()[..base.bytes().len() - removed].to_vec();
        bytes[0] = version;
        let decoded = decode_intent(&bytes, &RequestLimits::default(), 1_048_576).unwrap();
        assert_eq!(decoded.operation.pipeline(), Some(&old));
    }
    let mut separate = old;
    separate.creator.as_mut().unwrap().general_prompt = Some(PrivateImageText::new(
        "private restrained alternate image prompt",
    ));
    let encoded = encode_intent(
        &request(Some(separate.clone())),
        &RequestLimits::default(),
        1_048_576,
    )
    .unwrap();
    assert_ne!(encoded.fingerprint(), base.fingerprint());
    assert_eq!(
        decode_intent(encoded.bytes(), &RequestLimits::default(), 1_048_576)
            .unwrap()
            .operation
            .pipeline(),
        Some(&separate)
    );
    assert!(!format!("{separate:?}").contains("private restrained alternate image prompt"));
    let mut bounded = RequestLimits::default();
    bounded.max_prompt_chars = 32;
    assert!(encode_intent(&request(Some(separate.clone())), &bounded, 1_048_576).is_err());
    separate.creator.as_mut().unwrap().general_prompt = Some(PrivateImageText::new(" "));
    assert!(
        TextPrompt::new("portrait", 1)
            .unwrap()
            .with_pipeline(separate)
            .is_err()
    );
}

#[test]
fn standalone_routes_and_segmentation_bounds_bind_config_and_v9_keeps_none() {
    let mut config = crate::ImageConfig::defaults().unwrap();
    let base = super::encode_config(&config, 1_048_576).unwrap();
    let mut v9 = base.bytes()[..base.bytes().len() - 54].to_vec();
    v9[0] = 9;
    let old = super::decode_config(&v9, 1_048_576).unwrap();
    assert_eq!(old.analysis.head_locations, None);
    assert_eq!(old.analysis.staging_identity, None);
    assert_eq!(old.analysis.staging_facing, None);
    assert_eq!(old.analysis.segmentation, None);
    assert_eq!(old.analysis.segmentation_preparation.max_pixels, 20_000_000);
    assert_eq!(
        old.analysis.segmentation_preparation.max_picture_bytes,
        3_145_728
    );
    assert_eq!(
        old.analysis.segmentation_preparation.max_request_bytes,
        6_291_456
    );
    assert!(
        old.purposes
            .iter()
            .all(|purpose| purpose.account_override.is_none())
    );
    config.purposes[crate::Purpose::CreatorPortrait.index()].account_override =
        Some(AccountHandle::new("creator-openai").unwrap());
    config.analysis.head_locations = Some(route());
    config.analysis.staging_identity = Some(route());
    config.analysis.staging_facing = Some(route());
    config.analysis.segmentation = Some(crate::config::SegmentationRoute {
        provider: crate::config::SegmentationProvider::Fal,
        account: AccountHandle::new("sam-account").unwrap(),
        model: ModelName::new("fal-ai/sam-3.1/image").unwrap(),
        endpoint: ApiEndpoint::new("http://127.0.0.1:55499").unwrap(),
    });
    config.analysis.segmentation_preparation.max_pixels = 30_000_000;
    config.analysis.segmentation_preparation.max_picture_bytes = 4_000_000;
    config.analysis.segmentation_preparation.max_request_bytes = 8_000_000;
    let encoded = super::encode_config(&config, 1_048_576).unwrap();
    assert_eq!(encoded.version(), 12);
    assert_eq!(
        super::decode_config(encoded.bytes(), 1_048_576).unwrap(),
        config
    );
    assert_ne!(encoded.fingerprint(), base.fingerprint());
    let mut invalid = base.bytes().to_vec();
    // First newly appended route tag must remain a strict boolean.
    let first_route = invalid.len() - 54;
    invalid[first_route] = 9;
    assert!(super::decode_config(&invalid, 1_048_576).is_err());
    config.analysis.segmentation_preparation.max_picture_bytes = 0;
    assert!(super::encode_config(&config, 1_048_576).is_err());
}

#[test]
fn quota_snapshot_v11_is_bound_and_v10_retains_its_frozen_policy() {
    let mut config = crate::ImageConfig::defaults().unwrap();
    let encoded = super::encode_config(&config, 1_048_576).unwrap();
    let tail = encoded.bytes().len() - 14;
    let mut v10 = encoded.bytes()[..tail].to_vec();
    v10[0] = 10;
    let old = super::decode_config(&v10, 1_048_576).unwrap();
    assert!(old.quota.enabled);
    assert_eq!(old.quota.cooldown_ms, 900_000);
    assert_eq!(old.quota.maximum_accounts, 1_024);
    assert_eq!(old, config);
    let mut bad_tag = encoded.bytes().to_vec();
    bad_tag[tail] = 2;
    assert!(super::decode_config(&bad_tag, 1_048_576).is_err());
    config.quota.enabled = false;
    config.quota.cooldown_ms = 45_000;
    config.quota.maximum_accounts = 23;
    let changed = super::encode_config(&config, 1_048_576).unwrap();
    assert_ne!(changed.fingerprint(), encoded.fingerprint());
    assert_eq!(
        super::decode_config(changed.bytes(), 1_048_576).unwrap(),
        config
    );
    config.quota.maximum_accounts = 0;
    assert!(super::encode_config(&config, 1_048_576).is_err());
}

#[test]
fn creator_bfl_account_is_explicit_in_v12_and_absent_in_v11() {
    let mut config = crate::ImageConfig::defaults().unwrap();
    config.quota.cooldown_ms = 123_000;
    let base = super::encode_config(&config, 1_048_576).unwrap();
    let mut v11 = base.bytes()[..base.bytes().len() - 1].to_vec();
    v11[0] = 11;
    let old = super::decode_config(&v11, 1_048_576).unwrap();
    assert_eq!(old.creator_bfl_account, None);
    assert_eq!(old.quota.cooldown_ms, 123_000);
    assert_eq!(old, config);
    config.creator_bfl_account = Some(AccountHandle::new("creator-bfl").unwrap());
    let alternate = super::encode_config(&config, 1_048_576).unwrap();
    assert_ne!(alternate.fingerprint(), base.fingerprint());
    assert_eq!(
        super::decode_config(alternate.bytes(), 1_048_576).unwrap(),
        config
    );
    let mut bad_tag = base.bytes().to_vec();
    *bad_tag.last_mut().unwrap() = 2;
    assert!(super::decode_config(&bad_tag, 1_048_576).is_err());
}
