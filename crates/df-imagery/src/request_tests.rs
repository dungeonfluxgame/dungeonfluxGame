use super::*;
use crate::output::{
    AlphaHandling, Dimensions, Encoding, LossyQuality, OutputError, OutputProfile, RasterOutputSpec,
};
use crate::transform::{FlipAxis, ImageTransform, TransformError, TransformSource, TransformSpec};
use df_assets::{AssetId, Audience, MemberSet};
use df_types::{AccountId, CampaignId, MemberId, OperationId};

fn account() -> AccountId {
    AccountId::from_hex("00000000000000000000000000000011").unwrap()
}

fn operation() -> OperationId {
    OperationId::from_hex("00000000000000000000000000000022").unwrap()
}

fn member(byte: u8) -> MemberId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    MemberId::from_bytes(&bytes).unwrap()
}

fn members_audience() -> Audience {
    let campaign = CampaignId::from_hex("00000000000000000000000000000033").unwrap();
    Audience::Members(
        campaign,
        MemberSet::new(vec![member(1), member(2)]).unwrap(),
    )
}

fn source_ref() -> SourceRef {
    SourceRef {
        asset: AssetId::from_bytes(&[0x07; 16]).unwrap(),
        digest: AssetDigest::from_bytes([0x09; 32]),
        audience: Audience::PublicStatic,
    }
}

fn identity_with_fingerprint(byte: u8) -> OperationIdentity {
    OperationIdentity {
        owner: ImageOwner::Draft { account: account() },
        operation: operation(),
        fingerprint: [byte; 32],
    }
}

fn provider_output() -> OutputProfile {
    let dimensions = Dimensions::new(512, 512).unwrap();
    let encoding = Encoding::Jpeg {
        quality: LossyQuality::new(85).unwrap(),
    };
    let raster = RasterOutputSpec::new(dimensions, AlphaHandling::Opaque, encoding).unwrap();
    OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), raster)
}

fn local_output() -> OutputProfile {
    let raster = RasterOutputSpec::new(
        Dimensions::new(64, 64).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(85).unwrap(),
        },
    )
    .unwrap();
    OutputProfile::local(raster)
}

fn delivered() -> OutputProfile {
    provider_output()
}

fn generate_request() -> ImageRequest {
    ImageRequest {
        identity: identity_with_fingerprint(0x11),
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a tavern at dusk", 3).unwrap(),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: 7,
        output: delivered(),
    }
}

fn limits() -> RequestLimits {
    RequestLimits::default()
}

#[test]
fn prompt_debug_hides_text_in_the_normal_format() {
    let prompt = TextPrompt::new("PRIVATE_REVIEW_SENTINEL", 1).unwrap();
    let rendered = format!("{prompt:?}");
    assert!(!rendered.contains("PRIVATE_REVIEW_SENTINEL"), "{rendered}");
    assert!(rendered.contains("TextPrompt"), "{rendered}");
}

#[test]
fn prompt_debug_hides_text_in_the_alternate_format() {
    let prompt = TextPrompt::new("PRIVATE_REVIEW_SENTINEL", 1).unwrap();
    let rendered = format!("{prompt:#?}");
    assert!(!rendered.contains("PRIVATE_REVIEW_SENTINEL"), "{rendered}");
}

#[test]
fn enclosing_request_debug_hides_prompt_text_in_both_formats() {
    let request = generate_request();
    for rendered in [format!("{request:?}"), format!("{request:#?}")] {
        assert!(!rendered.contains("a tavern at dusk"), "{rendered}");
        assert!(rendered.contains("TextPrompt"), "{rendered}");
    }
}

#[test]
fn enclosing_edit_debug_hides_prompt_text() {
    let mut request = generate_request();
    request.operation = ImageOperation::Edit(EditSpec {
        prompt: TextPrompt::new("PRIVATE_EDIT_SENTINEL", 2).unwrap(),
        base: source_ref(),
        references: vec![source_ref()],
    });
    let rendered = format!("{request:?}");
    assert!(!rendered.contains("PRIVATE_EDIT_SENTINEL"), "{rendered}");
}

#[test]
fn empty_prompt_is_refused() {
    assert_eq!(
        TextPrompt::new("   ", 1).unwrap_err(),
        RequestError::EmptyPrompt
    );
}

#[test]
fn a_five_thousand_character_prompt_passes_construction_and_default_limits() {
    let text: String = std::iter::repeat_n('a', 5000).collect();
    let prompt = TextPrompt::new(&text, 1).unwrap();
    assert_eq!(prompt.len_chars(), 5000);
    let mut request = generate_request();
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt,
        references: Vec::new(),
    });
    request.validate(&limits()).unwrap();
}

#[test]
fn prompt_over_the_default_ceiling_is_refused_at_the_boundary() {
    let text: String = std::iter::repeat_n('a', DEFAULT_MAX_PROMPT_CHARS + 1).collect();
    let mut request = generate_request();
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new(&text, 1).unwrap(),
        references: Vec::new(),
    });
    assert_eq!(
        request.validate(&limits()).unwrap_err(),
        RequestError::PromptTooLong {
            len_chars: DEFAULT_MAX_PROMPT_CHARS + 1,
            max_chars: DEFAULT_MAX_PROMPT_CHARS,
        }
    );
}

#[test]
fn a_larger_configured_ceiling_admits_a_longer_prompt() {
    let text: String = std::iter::repeat_n('a', DEFAULT_MAX_PROMPT_CHARS + 1).collect();
    let mut request = generate_request();
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new(&text, 1).unwrap(),
        references: Vec::new(),
    });
    let raised = RequestLimits {
        max_prompt_chars: DEFAULT_MAX_PROMPT_CHARS + 1,
        ..limits()
    };
    request.validate(&raised).unwrap();
}

#[test]
fn a_zero_prompt_bound_refuses_every_request() {
    let request = generate_request();
    let empty = RequestLimits {
        max_prompt_chars: 0,
        ..limits()
    };
    assert_eq!(
        request.validate(&empty).unwrap_err(),
        RequestError::InvalidLimit {
            field: "max_prompt_chars",
        }
    );
}

#[test]
fn same_operation_with_different_fingerprint_conflicts() {
    let first = identity_with_fingerprint(0x11);
    let second = identity_with_fingerprint(0x22);
    assert!(first.conflicts_with(&second));
}

#[test]
fn same_operation_with_matching_fingerprint_does_not_conflict() {
    let first = identity_with_fingerprint(0x11);
    assert!(!first.conflicts_with(&first));
}

#[test]
fn withheld_audience_is_refused_before_provider_work() {
    let mut request = generate_request();
    request.audience = Audience::Withheld;
    assert_eq!(
        request.validate(&limits()).unwrap_err(),
        RequestError::WithheldAudience
    );
}

#[test]
fn too_many_references_are_refused() {
    let mut request = generate_request();
    for _ in 0..=DEFAULT_MAX_REFERENCES {
        request_references(&mut request).push(source_ref());
    }
    assert_eq!(
        request.validate(&limits()).unwrap_err(),
        RequestError::TooManyReferences {
            count: DEFAULT_MAX_REFERENCES + 1,
        }
    );
}

#[test]
fn a_larger_configured_reference_bound_admits_more_references() {
    let mut request = generate_request();
    for _ in 0..=DEFAULT_MAX_REFERENCES {
        request_references(&mut request).push(source_ref());
    }
    let raised = RequestLimits {
        max_references: DEFAULT_MAX_REFERENCES + 1,
        ..limits()
    };
    request.validate(&raised).unwrap();
}

fn request_references(request: &mut ImageRequest) -> &mut Vec<SourceRef> {
    match &mut request.operation {
        ImageOperation::Generate(spec) => &mut spec.references,
        ImageOperation::Edit(spec) => &mut spec.references,
        ImageOperation::Transform(_) => unreachable!(),
    }
}

#[test]
fn an_empty_member_set_is_none_and_never_generates() {
    assert_eq!(MemberSet::new(Vec::new()), None);
    assert!(MemberSet::new(vec![member(1)]).is_some());
}

#[test]
fn members_audience_round_trips_through_the_request() {
    let mut request = generate_request();
    request.audience = members_audience();
    request.validate(&limits()).unwrap();
}

#[test]
fn zero_asset_id_is_refused_by_the_catalog_type() {
    assert!(AssetId::from_bytes(&[0_u8; 16]).is_err());
}

#[test]
fn valid_generate_request_passes_validation() {
    generate_request().validate(&limits()).unwrap();
}

#[test]
fn a_format_invalid_output_cannot_be_constructed() {
    assert_eq!(
        RasterOutputSpec::new(
            Dimensions::new(64, 64).unwrap(),
            AlphaHandling::PreserveAlpha,
            Encoding::Jpeg {
                quality: LossyQuality::new(85).unwrap(),
            },
        )
        .unwrap_err(),
        OutputError::AlphaWithoutMatte
    );
}

#[test]
fn transform_step_failure_keeps_its_transform_cause() {
    let mut request = generate_request();
    request.operation = ImageOperation::Transform(TransformSpec {
        source: TransformSource::Image(source_ref()),
        steps: vec![ImageTransform::Flip(FlipAxis::Horizontal); DEFAULT_MAX_TRANSFORM_STEPS + 1],
    });
    request.output = local_output();
    assert_eq!(
        request.validate(&limits()).unwrap_err(),
        RequestError::InvalidTransform(TransformError::TooManySteps)
    );
}

#[test]
fn local_transform_needs_no_generated_size() {
    let mut request = generate_request();
    request.operation = ImageOperation::Transform(TransformSpec {
        source: TransformSource::Image(source_ref()),
        steps: Vec::new(),
    });
    request.output = local_output();
    assert_eq!(request.output.generated(), None);
    request.validate(&limits()).unwrap();
}

#[test]
fn generate_with_local_profile_is_refused() {
    let mut request = generate_request();
    request.output = local_output();
    assert_eq!(
        request.validate(&limits()).unwrap_err(),
        RequestError::ProfileMismatch
    );
}

#[test]
fn transform_with_provider_profile_is_refused() {
    let mut request = generate_request();
    request.operation = ImageOperation::Transform(TransformSpec {
        source: TransformSource::Image(source_ref()),
        steps: Vec::new(),
    });
    request.output = provider_output();
    assert_eq!(
        request.validate(&limits()).unwrap_err(),
        RequestError::ProfileMismatch
    );
}

#[test]
fn provider_generation_and_delivery_dimensions_stay_distinct() {
    let request = generate_request();
    let generated = request.output.generated().unwrap();
    let delivered = request.output.delivered().dimensions();
    assert_eq!(generated, Dimensions::new(1024, 1024).unwrap());
    assert_eq!(delivered, Dimensions::new(512, 512).unwrap());
    assert_ne!(generated.pixel_count(), delivered.pixel_count());
}
