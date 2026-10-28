//! Failure-focused checks for the canonical intent and config codec. Golden
//! vectors pin the full wire layout as literals: any layout change breaks
//! these bytes, and any byte change breaks the digests pinned beside them.

use sha2::{Digest, Sha256};

use crate::config::{AccountHandle, BackendId, GlobalMode, ImageConfig};
use crate::intent_codec::{
    CodecError, decode_config, decode_intent, encode_config, encode_intent,
};
use crate::output::{
    AlphaHandling, Dimensions, Encoding, LossyQuality, OutputProfile, RasterOutputSpec, Rgb,
};
use crate::request::{
    AssetDigest, EditSpec, ExecutionMode, Freshness, GenerateSpec, ImageOperation, ImageOwner,
    ImageRequest, OperationIdentity, Purpose, PurposedUse, RequestError, RequestLimits, SourceRef,
    TextPrompt, Urgency, VariantRole,
};
use crate::transform::{
    AlignAnchor, Anchor1D, BlendMode, BlurAlgorithm, BlurSpec, BrightnessOffset, CanvasSpec,
    ColorEffect, CompositeLayer, CoverageRef, ExposureStops, FlipAxis, ImageTransform,
    Interpolation, MaskChannel, MaskMode, MaskSource, MaskSpec, PadSpec, PixelRect, ResizeMode,
    ResizeSpec, RotateSpec, SharpenSpec, TransformError, TransformSource, TransformSpec,
};
use crate::{AssetId, Audience, MemberSet};
use df_types::{AccountId, CampaignId, MemberId, OperationId, RunId};

const MAX_BYTES: usize = 1_048_576;

fn account(byte: u8) -> AccountId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    AccountId::from_bytes(&bytes).unwrap()
}

fn campaign(byte: u8) -> CampaignId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    CampaignId::from_bytes(&bytes).unwrap()
}

fn run(byte: u8) -> RunId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    RunId::from_bytes(&bytes).unwrap()
}

fn operation(byte: u8) -> OperationId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    OperationId::from_bytes(&bytes).unwrap()
}

fn member(byte: u8) -> MemberId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    MemberId::from_bytes(&bytes).unwrap()
}

fn asset(byte: u8) -> AssetId {
    let mut bytes = [0_u8; 16];
    bytes[15] = byte;
    AssetId::from_bytes(&bytes).unwrap()
}

fn limits() -> RequestLimits {
    RequestLimits::default()
}

fn source_ref(byte: u8) -> SourceRef {
    SourceRef {
        asset: asset(byte),
        digest: AssetDigest::from_bytes([byte; 32]),
        audience: Audience::PublicStatic,
    }
}

fn coverage_ref(byte: u8, revision: u32) -> CoverageRef {
    CoverageRef {
        coverage: source_ref(byte),
        analyzed: source_ref(byte.wrapping_add(1)),
        analysis_revision: revision,
    }
}

fn provider_output() -> OutputProfile {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(64, 64).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(85).unwrap(),
        },
    )
    .unwrap();
    OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), delivered)
}

fn local_output() -> OutputProfile {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(32, 32).unwrap(),
        AlphaHandling::Opaque,
        Encoding::Png,
    )
    .unwrap();
    OutputProfile::local(delivered)
}

fn generate_request() -> ImageRequest {
    ImageRequest {
        identity: OperationIdentity {
            owner: ImageOwner::Draft {
                account: account(0x11),
            },
            operation: operation(0x22),
            fingerprint: [0x33; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a tavern at dusk", 3).unwrap(),
            references: vec![source_ref(0x07)],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: 7,
        output: provider_output(),
    }
}

fn transform_request() -> ImageRequest {
    let canvas = CanvasSpec::new(
        64,
        64,
        Rgb {
            red: 1,
            green: 2,
            blue: 3,
        },
    )
    .unwrap();
    let resize = ResizeSpec::new(
        32,
        32,
        ResizeMode::Contain,
        AlignAnchor::center(),
        Interpolation::Bilinear,
        false,
        None,
    )
    .unwrap();
    ImageRequest {
        identity: OperationIdentity {
            owner: ImageOwner::Run {
                campaign: campaign(0x0c),
                run: run(0x0d),
            },
            operation: operation(0x22),
            fingerprint: [0x33; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ReferencePrep,
            role: VariantRole::Reference,
        },
        operation: ImageOperation::Transform(TransformSpec {
            source: TransformSource::Canvas(canvas),
            steps: vec![
                ImageTransform::Resize(resize),
                ImageTransform::Flip(FlipAxis::Horizontal),
            ],
        }),
        execution: ExecutionMode::Replay,
        freshness: Freshness::Regenerate {
            nonce: operation(0x44),
        },
        urgency: Urgency::Visible,
        audience: Audience::CampaignShared(campaign(0x0c)),
        expected_source_revision: 9,
        output: local_output(),
    }
}

/// One intent exercising every transform step shape, both mask sources, both
/// mask channels and modes, two layers and a coverage source.
fn full_coverage_request() -> ImageRequest {
    let fill = Rgb {
        red: 9,
        green: 8,
        blue: 7,
    };
    let steps = vec![
        ImageTransform::Resize(
            ResizeSpec::new(
                48,
                40,
                ResizeMode::Cover,
                AlignAnchor {
                    x: Anchor1D::Start,
                    y: Anchor1D::End,
                },
                Interpolation::Bicubic,
                true,
                Some(fill),
            )
            .unwrap(),
        ),
        ImageTransform::Crop(PixelRect::new(2, 4, 16, 12).unwrap()),
        ImageTransform::Pad(PadSpec::new(56, 52, AlignAnchor::center(), fill).unwrap()),
        ImageTransform::Rotate(RotateSpec::QuarterTurns(1)),
        ImageTransform::Rotate(RotateSpec::Arbitrary {
            degrees: 45.0,
            expand_canvas: true,
            fill,
            interpolation: Interpolation::Lanczos3,
        }),
        ImageTransform::Flip(FlipAxis::Vertical),
        ImageTransform::Brighten(BrightnessOffset::new(0.5).unwrap()),
        ImageTransform::Expose(ExposureStops::new(-1.25).unwrap()),
        ImageTransform::Color(ColorEffect::contrast(1.5, 0.5).unwrap()),
        ImageTransform::Color(ColorEffect::saturation(0.25).unwrap()),
        ImageTransform::Color(ColorEffect::hue_shift(90.0).unwrap()),
        ImageTransform::Color(ColorEffect::Grayscale),
        ImageTransform::Blur(BlurSpec::new(2.0, BlurAlgorithm::Gaussian).unwrap()),
        ImageTransform::Sharpen(SharpenSpec::new(1.5, 0.75, 10).unwrap()),
        ImageTransform::Mask(
            MaskSpec::new(
                MaskSource::Asset(source_ref(0x0a)),
                Dimensions::new(32, 32).unwrap(),
                AlignAnchor::center(),
                MaskChannel::Alpha,
                false,
                1.5,
                MaskMode::Multiply,
            )
            .unwrap(),
        ),
        ImageTransform::Mask(
            MaskSpec::new(
                MaskSource::Coverage(coverage_ref(0x0b, 4)),
                Dimensions::new(24, 20).unwrap(),
                AlignAnchor {
                    x: Anchor1D::End,
                    y: Anchor1D::Start,
                },
                MaskChannel::Luminance,
                true,
                0.5,
                MaskMode::Replace,
            )
            .unwrap(),
        ),
        ImageTransform::Composite(vec![
            CompositeLayer::new(source_ref(0x0c), -3, 4, 500, BlendMode::SourceOver).unwrap(),
            CompositeLayer::new(source_ref(0x0d), 7, -8, 1000, BlendMode::SourceOver).unwrap(),
        ]),
    ];
    let mut request = transform_request();
    request.operation = ImageOperation::Transform(TransformSpec {
        source: TransformSource::Coverage(coverage_ref(0x09, 3)),
        steps,
    });
    request
}

fn patched(base: &[u8], at: usize, bytes: &[u8]) -> Vec<u8> {
    let mut out = base.to_vec();
    out[at..at + bytes.len()].copy_from_slice(bytes);
    out
}

/// Canonical bytes of [`generate_request`]: version, ScenePlate, Primary,
/// Generate, Draft owner, prompt, one reference, Live, ReuseCompatible,
/// PublicStatic, revision 7, then the 1024-wide provider JPEG-85 output.
const GENERATE_GOLDEN: &[u8] = &[
    0x01, 0x03, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x10, 0x61, 0x20, 0x74,
    0x61, 0x76, 0x65, 0x72, 0x6e, 0x20, 0x61, 0x74, 0x20, 0x64, 0x75, 0x73, 0x6b, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
    0x07, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00,
    0x04, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00,
    0x55,
];

/// Canonical bytes of [`transform_request`]: version, ReferencePrep,
/// Reference, Transform, Run owner, canvas source, contain resize plus
/// horizontal flip, Replay, regenerate nonce, shared audience, revision 9,
/// then the local 32-wide opaque PNG output.
const TRANSFORM_GOLDEN: &[u8] = &[
    0x01, 0x0a, 0x03, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0d, 0x01, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x40, 0x01, 0x02,
    0x03, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x20, 0x00, 0x01,
    0x01, 0x01, 0x00, 0x00, 0x04, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x09, 0x01, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x20, 0x00, 0x02,
];

/// Canonical bytes of the default config snapshot: version, Live, OpenAi,
/// twelve enabled purpose gates, four backend gates on the default handle,
/// transform limits, scheduler bounds and request limits.
const CONFIG_GOLDEN: &[u8] = &[
    0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x07,
    0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x07,
    0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x07,
    0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x07,
    0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1e, 0x00, 0x00, 0x00, 0x03,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xaa, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x7d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08,
];

#[test]
fn generate_intent_bytes_match_the_pinned_literal() {
    let encoded = encode_intent(&generate_request(), &limits(), MAX_BYTES).unwrap();
    assert_eq!(encoded.version(), 1);
    assert_eq!(encoded.bytes().len(), 129);
    assert_eq!(encoded.bytes(), GENERATE_GOLDEN);
}

#[test]
fn generate_intent_fingerprint_is_the_digest_of_the_pinned_bytes() {
    let encoded = encode_intent(&generate_request(), &limits(), MAX_BYTES).unwrap();
    let pinned: [u8; 32] = Sha256::digest(GENERATE_GOLDEN).into();
    assert_eq!(encoded.fingerprint(), &pinned);
    assert_ne!(encoded.fingerprint(), &[0_u8; 32]);
}

#[test]
fn transform_intent_bytes_match_the_pinned_literal() {
    let encoded = encode_intent(&transform_request(), &limits(), MAX_BYTES).unwrap();
    assert_eq!(encoded.version(), 1);
    assert_eq!(encoded.bytes().len(), 124);
    assert_eq!(encoded.bytes(), TRANSFORM_GOLDEN);
}

#[test]
fn transform_intent_fingerprint_is_the_digest_of_the_pinned_bytes() {
    let encoded = encode_intent(&transform_request(), &limits(), MAX_BYTES).unwrap();
    let pinned: [u8; 32] = Sha256::digest(TRANSFORM_GOLDEN).into();
    assert_eq!(encoded.fingerprint(), &pinned);
    assert_ne!(encoded.fingerprint(), &[0_u8; 32]);
}

#[test]
fn config_snapshot_bytes_match_the_pinned_literal() {
    let snapshot = encode_config(&ImageConfig::defaults().unwrap(), MAX_BYTES).unwrap();
    assert_eq!(snapshot.version(), 1);
    assert_eq!(snapshot.bytes().len(), 204);
    assert_eq!(snapshot.bytes(), CONFIG_GOLDEN);
}

#[test]
fn config_snapshot_fingerprint_is_the_digest_of_the_pinned_bytes() {
    let snapshot = encode_config(&ImageConfig::defaults().unwrap(), MAX_BYTES).unwrap();
    let pinned: [u8; 32] = Sha256::digest(CONFIG_GOLDEN).into();
    assert_eq!(snapshot.fingerprint(), &pinned);
    assert_ne!(snapshot.fingerprint(), &[0_u8; 32]);
}

fn assert_intent_round_trip(request: &ImageRequest) {
    let encoded = encode_intent(request, &limits(), MAX_BYTES).unwrap();
    let decoded = decode_intent(encoded.bytes(), &limits(), MAX_BYTES).unwrap();
    assert_eq!(decoded.owner, request.identity.owner);
    assert_eq!(decoded.purpose, request.purpose);
    assert_eq!(decoded.operation, request.operation);
    assert_eq!(decoded.execution, request.execution);
    assert_eq!(decoded.freshness, request.freshness);
    assert_eq!(decoded.audience, request.audience);
    assert_eq!(
        decoded.expected_source_revision,
        request.expected_source_revision
    );
    assert_eq!(decoded.output, request.output);
}

#[test]
fn generate_intent_round_trips_through_validated_constructors() {
    assert_intent_round_trip(&generate_request());
}

#[test]
fn transform_intent_round_trips_with_ordered_steps() {
    assert_intent_round_trip(&transform_request());
}

#[test]
fn full_coverage_transform_round_trips_every_step_shape() {
    assert_intent_round_trip(&full_coverage_request());
}

#[test]
fn every_purpose_index_round_trips() {
    for index in 0..Purpose::COUNT {
        let mut request = generate_request();
        request.purpose.purpose = Purpose::from_index(index).unwrap();
        assert_intent_round_trip(&request);
    }
}

#[test]
fn every_role_execution_freshness_and_owner_round_trips() {
    for role in [
        VariantRole::Primary,
        VariantRole::Thumbnail,
        VariantRole::Mask,
        VariantRole::Reference,
    ] {
        let mut request = generate_request();
        request.purpose.role = role;
        assert_intent_round_trip(&request);
    }
    for execution in [
        ExecutionMode::Live,
        ExecutionMode::PreparedOnly,
        ExecutionMode::Replay,
    ] {
        let mut request = generate_request();
        request.execution = execution;
        assert_intent_round_trip(&request);
    }
    let mut regenerate = generate_request();
    regenerate.freshness = Freshness::Regenerate {
        nonce: operation(0x55),
    };
    assert_intent_round_trip(&regenerate);
    let mut run_owned = generate_request();
    run_owned.identity.owner = ImageOwner::Run {
        campaign: campaign(0x0c),
        run: run(0x0d),
    };
    assert_intent_round_trip(&run_owned);
}

#[test]
fn every_audience_shape_round_trips() {
    let campaign_id = campaign(0x0c);
    let audiences = [
        Audience::PublicStatic,
        Audience::CampaignShared(campaign_id),
        Audience::Members(
            campaign_id,
            MemberSet::new(vec![member(0x02), member(0x01)]).unwrap(),
        ),
        Audience::Host(campaign_id),
        Audience::OwnerPrivate(account(0x0e)),
    ];
    for audience in audiences {
        let mut request = generate_request();
        request.audience = audience;
        assert_intent_round_trip(&request);
    }
    let mut shared_source = generate_request();
    if let ImageOperation::Generate(spec) = &mut shared_source.operation {
        spec.references = vec![SourceRef {
            asset: asset(0x07),
            digest: AssetDigest::from_bytes([0x07; 32]),
            audience: Audience::CampaignShared(campaign_id),
        }];
    }
    assert_intent_round_trip(&shared_source);
}

#[test]
fn encoding_is_deterministic_for_equal_intent() {
    let first = encode_intent(&generate_request(), &limits(), MAX_BYTES).unwrap();
    let second = encode_intent(&generate_request(), &limits(), MAX_BYTES).unwrap();
    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.fingerprint(), second.fingerprint());
}

#[test]
fn fingerprint_is_sha256_over_the_canonical_bytes() {
    let encoded = encode_intent(&generate_request(), &limits(), MAX_BYTES).unwrap();
    let expected: [u8; 32] = Sha256::digest(encoded.bytes()).into();
    assert_eq!(encoded.fingerprint(), &expected);
    assert_ne!(encoded.fingerprint(), &[0_u8; 32]);
}

#[test]
fn material_fields_change_the_bytes_and_fingerprint() {
    let base = encode_intent(&generate_request(), &limits(), MAX_BYTES).unwrap();
    let mut prompt = generate_request();
    if let ImageOperation::Generate(spec) = &mut prompt.operation {
        spec.prompt = TextPrompt::new("a harbor at dawn", 3).unwrap();
    }
    let changed = encode_intent(&prompt, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), changed.bytes());
    assert_ne!(base.fingerprint(), changed.fingerprint());

    let mut template = generate_request();
    if let ImageOperation::Generate(spec) = &mut template.operation {
        spec.prompt = TextPrompt::new("a tavern at dusk", 9).unwrap();
    }
    let changed = encode_intent(&template, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), changed.bytes());

    let mut audience = generate_request();
    audience.audience = Audience::CampaignShared(campaign(0x0c));
    let changed = encode_intent(&audience, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), changed.bytes());

    let mut revision = generate_request();
    revision.expected_source_revision = 8;
    let changed = encode_intent(&revision, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), changed.bytes());

    let mut execution = generate_request();
    execution.execution = ExecutionMode::Replay;
    let changed = encode_intent(&execution, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), changed.bytes());

    let mut purpose = generate_request();
    purpose.purpose.purpose = Purpose::PartyMural;
    purpose.purpose.role = VariantRole::Thumbnail;
    let changed = encode_intent(&purpose, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), changed.bytes());

    let mut owner = generate_request();
    owner.identity.owner = ImageOwner::Draft {
        account: account(0x12),
    };
    let changed = encode_intent(&owner, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), changed.bytes());

    let mut output = generate_request();
    output.output = local_output();
    if let ImageOperation::Generate(spec) = &mut output.operation {
        spec.references = Vec::new();
    }
    output.operation = ImageOperation::Transform(TransformSpec {
        source: TransformSource::Canvas(
            CanvasSpec::new(
                16,
                16,
                Rgb {
                    red: 1,
                    green: 1,
                    blue: 1,
                },
            )
            .unwrap(),
        ),
        steps: Vec::new(),
    });
    // An empty step list stays a valid no-op recipe, so swapping the operation
    // and output together is material rather than refused.
    let changed = encode_intent(&output, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), changed.bytes());
    assert_intent_round_trip(&output);
}

#[test]
fn excluded_caller_fields_leave_the_bytes_stable() {
    let base = encode_intent(&generate_request(), &limits(), MAX_BYTES).unwrap();
    let mut changed_id = generate_request();
    changed_id.identity.operation = operation(0x99);
    let recoded = encode_intent(&changed_id, &limits(), MAX_BYTES).unwrap();
    assert_eq!(base.bytes(), recoded.bytes());

    let mut changed_print = generate_request();
    changed_print.identity.fingerprint = [0x99; 32];
    let recoded = encode_intent(&changed_print, &limits(), MAX_BYTES).unwrap();
    assert_eq!(base.bytes(), recoded.bytes());

    let mut changed_urgency = generate_request();
    changed_urgency.urgency = Urgency::Speculative;
    let recoded = encode_intent(&changed_urgency, &limits(), MAX_BYTES).unwrap();
    assert_eq!(base.bytes(), recoded.bytes());
    assert_eq!(base.fingerprint(), recoded.fingerprint());
}

#[test]
fn reference_order_is_material() {
    let mut request = generate_request();
    if let ImageOperation::Generate(spec) = &mut request.operation {
        spec.references = vec![source_ref(0x07), source_ref(0x08)];
    }
    let ordered = encode_intent(&request, &limits(), MAX_BYTES).unwrap();
    if let ImageOperation::Generate(spec) = &mut request.operation {
        spec.references = vec![source_ref(0x08), source_ref(0x07)];
    }
    let swapped = encode_intent(&request, &limits(), MAX_BYTES).unwrap();
    assert_ne!(ordered.bytes(), swapped.bytes());
}

#[test]
fn member_set_order_is_canonical() {
    let members = |first: u8, second: u8| {
        Audience::Members(
            campaign(0x0c),
            MemberSet::new(vec![member(first), member(second)]).unwrap(),
        )
    };
    let mut forward = generate_request();
    forward.audience = members(0x01, 0x02);
    let mut backward = generate_request();
    backward.audience = members(0x02, 0x01);
    let first = encode_intent(&forward, &limits(), MAX_BYTES).unwrap();
    let second = encode_intent(&backward, &limits(), MAX_BYTES).unwrap();
    assert_eq!(first.bytes(), second.bytes());
    let decoded = decode_intent(first.bytes(), &limits(), MAX_BYTES).unwrap();
    assert_eq!(decoded.audience, members(0x01, 0x02));
}

#[test]
fn negative_zero_floats_canonicalize() {
    let mut positive = transform_request();
    if let ImageOperation::Transform(spec) = &mut positive.operation {
        spec.steps
            .push(ImageTransform::Brighten(BrightnessOffset::new(0.0).unwrap()));
    }
    let mut negative = transform_request();
    if let ImageOperation::Transform(spec) = &mut negative.operation {
        spec.steps
            .push(ImageTransform::Brighten(BrightnessOffset::new(-0.0).unwrap()));
    }
    let first = encode_intent(&positive, &limits(), MAX_BYTES).unwrap();
    let second = encode_intent(&negative, &limits(), MAX_BYTES).unwrap();
    assert_eq!(first.bytes(), second.bytes());
    let decoded = decode_intent(first.bytes(), &limits(), MAX_BYTES).unwrap();
    assert_eq!(decoded.operation, positive.operation);
}

#[test]
fn transform_sources_change_the_bytes() {
    let base = encode_intent(&transform_request(), &limits(), MAX_BYTES).unwrap();
    let mut imaged = transform_request();
    if let ImageOperation::Transform(spec) = &mut imaged.operation {
        spec.source = TransformSource::Image(source_ref(0x07));
    }
    let recoded = encode_intent(&imaged, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), recoded.bytes());

    let mut covered = transform_request();
    if let ImageOperation::Transform(spec) = &mut covered.operation {
        spec.source = TransformSource::Coverage(coverage_ref(0x09, 3));
    }
    let recoded = encode_intent(&covered, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), recoded.bytes());

    let mut revised = transform_request();
    if let ImageOperation::Transform(spec) = &mut revised.operation {
        spec.source = TransformSource::Coverage(coverage_ref(0x09, 4));
    }
    let recoded = encode_intent(&revised, &limits(), MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), recoded.bytes());
    assert_intent_round_trip(&revised);
}

#[test]
fn every_resize_field_changes_the_bytes() {
    let field = |resize: ResizeSpec| {
        let mut request = transform_request();
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![ImageTransform::Resize(resize)];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap()
    };
    let base = field(
        ResizeSpec::new(
            32,
            32,
            ResizeMode::Contain,
            AlignAnchor::center(),
            Interpolation::Bilinear,
            false,
            None,
        )
        .unwrap(),
    );
    let center = AlignAnchor::center();
    let variants = [
        ResizeSpec::new(
            40,
            32,
            ResizeMode::Contain,
            center,
            Interpolation::Bilinear,
            false,
            None,
        )
        .unwrap(),
        ResizeSpec::new(
            32,
            40,
            ResizeMode::Contain,
            center,
            Interpolation::Bilinear,
            false,
            None,
        )
        .unwrap(),
        ResizeSpec::new(
            32,
            32,
            ResizeMode::Cover,
            center,
            Interpolation::Bilinear,
            false,
            None,
        )
        .unwrap(),
        ResizeSpec::new(
            32,
            32,
            ResizeMode::Stretch,
            center,
            Interpolation::Bilinear,
            false,
            None,
        )
        .unwrap(),
        ResizeSpec::new(
            32,
            32,
            ResizeMode::Pad,
            center,
            Interpolation::Bilinear,
            false,
            Some(Rgb {
                red: 1,
                green: 2,
                blue: 3,
            }),
        )
        .unwrap(),
        ResizeSpec::new(
            32,
            32,
            ResizeMode::Contain,
            AlignAnchor {
                x: Anchor1D::Start,
                y: Anchor1D::End,
            },
            Interpolation::Bilinear,
            false,
            None,
        )
        .unwrap(),
        ResizeSpec::new(
            32,
            32,
            ResizeMode::Contain,
            center,
            Interpolation::Lanczos3,
            false,
            None,
        )
        .unwrap(),
        ResizeSpec::new(
            32,
            32,
            ResizeMode::Contain,
            center,
            Interpolation::Bilinear,
            true,
            None,
        )
        .unwrap(),
        ResizeSpec::new(
            32,
            32,
            ResizeMode::Contain,
            center,
            Interpolation::Bilinear,
            false,
            Some(Rgb {
                red: 4,
                green: 5,
                blue: 6,
            }),
        )
        .unwrap(),
    ];
    for variant in variants {
        assert_ne!(field(variant).bytes(), base.bytes());
    }
}

#[test]
fn crop_pad_and_flip_fields_change_the_bytes() {
    let step_case = |step: ImageTransform| {
        let mut request = transform_request();
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![step];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap()
    };
    let base_crop = step_case(ImageTransform::Crop(PixelRect::new(1, 2, 16, 12).unwrap()));
    let moved_crop = step_case(ImageTransform::Crop(PixelRect::new(2, 2, 16, 12).unwrap()));
    assert_ne!(base_crop.bytes(), moved_crop.bytes());

    let fill = Rgb {
        red: 1,
        green: 2,
        blue: 3,
    };
    let base_pad = step_case(ImageTransform::Pad(
        PadSpec::new(48, 44, AlignAnchor::center(), fill).unwrap(),
    ));
    let resized_pad = step_case(ImageTransform::Pad(
        PadSpec::new(52, 44, AlignAnchor::center(), fill).unwrap(),
    ));
    assert_ne!(base_pad.bytes(), resized_pad.bytes());
    let recolored_pad = step_case(ImageTransform::Pad(
        PadSpec::new(
            48,
            44,
            AlignAnchor::center(),
            Rgb {
                red: 9,
                green: 9,
                blue: 9,
            },
        )
        .unwrap(),
    ));
    assert_ne!(base_pad.bytes(), recolored_pad.bytes());

    let horizontal = step_case(ImageTransform::Flip(FlipAxis::Horizontal));
    let vertical = step_case(ImageTransform::Flip(FlipAxis::Vertical));
    assert_ne!(horizontal.bytes(), vertical.bytes());
}

#[test]
fn rotation_shapes_change_the_bytes() {
    let step_case = |step: ImageTransform| {
        let mut request = transform_request();
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![step];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap()
    };
    let quarter = step_case(ImageTransform::Rotate(RotateSpec::QuarterTurns(1)));
    let half = step_case(ImageTransform::Rotate(RotateSpec::QuarterTurns(2)));
    assert_ne!(quarter.bytes(), half.bytes());

    let arbitrary = step_case(ImageTransform::Rotate(RotateSpec::Arbitrary {
        degrees: 45.0,
        expand_canvas: true,
        fill: Rgb {
            red: 1,
            green: 2,
            blue: 3,
        },
        interpolation: Interpolation::Bicubic,
    }));
    assert_ne!(quarter.bytes(), arbitrary.bytes());

    let tilted = step_case(ImageTransform::Rotate(RotateSpec::Arbitrary {
        degrees: 30.0,
        expand_canvas: true,
        fill: Rgb {
            red: 1,
            green: 2,
            blue: 3,
        },
        interpolation: Interpolation::Bicubic,
    }));
    assert_ne!(arbitrary.bytes(), tilted.bytes());
}

#[test]
fn brightness_and_exposure_values_change_the_bytes() {
    let step_case = |step: ImageTransform| {
        let mut request = transform_request();
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![step];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap()
    };
    let dim = step_case(ImageTransform::Brighten(BrightnessOffset::new(0.5).unwrap()));
    let dimmer = step_case(ImageTransform::Brighten(BrightnessOffset::new(-0.5).unwrap()));
    assert_ne!(dim.bytes(), dimmer.bytes());

    let up = step_case(ImageTransform::Expose(ExposureStops::new(1.0).unwrap()));
    let down = step_case(ImageTransform::Expose(ExposureStops::new(-1.0).unwrap()));
    assert_ne!(up.bytes(), down.bytes());
    assert_ne!(dim.bytes(), up.bytes());
}

#[test]
fn color_effect_variants_change_the_bytes() {
    let step_case = |step: ImageTransform| {
        let mut request = transform_request();
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![step];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap()
    };
    let contrast = step_case(ImageTransform::Color(
        ColorEffect::contrast(1.5, 0.5).unwrap(),
    ));
    let stronger = step_case(ImageTransform::Color(
        ColorEffect::contrast(2.0, 0.5).unwrap(),
    ));
    assert_ne!(contrast.bytes(), stronger.bytes());
    let shifted_pivot = step_case(ImageTransform::Color(
        ColorEffect::contrast(1.5, 0.25).unwrap(),
    ));
    assert_ne!(contrast.bytes(), shifted_pivot.bytes());

    let saturation = step_case(ImageTransform::Color(
        ColorEffect::saturation(0.5).unwrap(),
    ));
    assert_ne!(contrast.bytes(), saturation.bytes());
    let hue = step_case(ImageTransform::Color(
        ColorEffect::hue_shift(90.0).unwrap(),
    ));
    assert_ne!(saturation.bytes(), hue.bytes());
    let gray = step_case(ImageTransform::Color(ColorEffect::Grayscale));
    assert_ne!(hue.bytes(), gray.bytes());
}

#[test]
fn blur_and_sharpen_fields_change_the_bytes() {
    let step_case = |step: ImageTransform| {
        let mut request = transform_request();
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![step];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap()
    };
    let boxed = step_case(ImageTransform::Blur(
        BlurSpec::new(2.0, BlurAlgorithm::Box).unwrap(),
    ));
    let wider = step_case(ImageTransform::Blur(
        BlurSpec::new(3.0, BlurAlgorithm::Box).unwrap(),
    ));
    assert_ne!(boxed.bytes(), wider.bytes());
    let gaussian = step_case(ImageTransform::Blur(
        BlurSpec::new(2.0, BlurAlgorithm::Gaussian).unwrap(),
    ));
    assert_ne!(boxed.bytes(), gaussian.bytes());

    let soft = step_case(ImageTransform::Sharpen(
        SharpenSpec::new(1.0, 0.5, 4).unwrap(),
    ));
    let strong = step_case(ImageTransform::Sharpen(
        SharpenSpec::new(1.0, 0.75, 4).unwrap(),
    ));
    assert_ne!(soft.bytes(), strong.bytes());
    let edged = step_case(ImageTransform::Sharpen(
        SharpenSpec::new(1.0, 0.5, 9).unwrap(),
    ));
    assert_ne!(soft.bytes(), edged.bytes());
}

#[test]
fn mask_fields_change_the_bytes() {
    let step_case = |step: ImageTransform| {
        let mut request = transform_request();
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![step];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap()
    };
    let asset_mask = ImageTransform::Mask(
        MaskSpec::new(
            MaskSource::Asset(source_ref(0x0a)),
            Dimensions::new(32, 32).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Alpha,
            false,
            1.5,
            MaskMode::Multiply,
        )
        .unwrap(),
    );
    let coverage_mask = ImageTransform::Mask(
        MaskSpec::new(
            MaskSource::Coverage(coverage_ref(0x0b, 4)),
            Dimensions::new(32, 32).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Alpha,
            false,
            1.5,
            MaskMode::Multiply,
        )
        .unwrap(),
    );
    assert_ne!(step_case(asset_mask.clone()).bytes(), step_case(coverage_mask).bytes());

    let luminance = ImageTransform::Mask(
        MaskSpec::new(
            MaskSource::Asset(source_ref(0x0a)),
            Dimensions::new(32, 32).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Luminance,
            false,
            1.5,
            MaskMode::Multiply,
        )
        .unwrap(),
    );
    assert_ne!(step_case(asset_mask.clone()).bytes(), step_case(luminance).bytes());

    let inverted = ImageTransform::Mask(
        MaskSpec::new(
            MaskSource::Asset(source_ref(0x0a)),
            Dimensions::new(32, 32).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Alpha,
            true,
            1.5,
            MaskMode::Multiply,
        )
        .unwrap(),
    );
    assert_ne!(step_case(asset_mask.clone()).bytes(), step_case(inverted).bytes());

    let replaced = ImageTransform::Mask(
        MaskSpec::new(
            MaskSource::Asset(source_ref(0x0a)),
            Dimensions::new(32, 32).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Alpha,
            false,
            1.5,
            MaskMode::Replace,
        )
        .unwrap(),
    );
    assert_ne!(step_case(asset_mask.clone()).bytes(), step_case(replaced).bytes());

    let softer = ImageTransform::Mask(
        MaskSpec::new(
            MaskSource::Asset(source_ref(0x0a)),
            Dimensions::new(32, 32).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Alpha,
            false,
            2.5,
            MaskMode::Multiply,
        )
        .unwrap(),
    );
    assert_ne!(step_case(asset_mask.clone()).bytes(), step_case(softer).bytes());

    let smaller = ImageTransform::Mask(
        MaskSpec::new(
            MaskSource::Asset(source_ref(0x0a)),
            Dimensions::new(24, 20).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Alpha,
            false,
            1.5,
            MaskMode::Multiply,
        )
        .unwrap(),
    );
    assert_ne!(step_case(asset_mask.clone()).bytes(), step_case(smaller.clone()).bytes());
    let mut both = transform_request();
    if let ImageOperation::Transform(spec) = &mut both.operation {
        spec.steps = vec![asset_mask, smaller];
    }
    assert_intent_round_trip(&both);
}

#[test]
fn composite_layers_change_the_bytes() {
    let composite_case = |layers: Vec<CompositeLayer>| {
        let mut request = transform_request();
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![ImageTransform::Composite(layers)];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap()
    };
    let base = composite_case(vec![
        CompositeLayer::new(source_ref(0x0c), -3, 4, 500, BlendMode::SourceOver).unwrap(),
    ]);
    let moved = composite_case(vec![
        CompositeLayer::new(source_ref(0x0c), 5, 4, 500, BlendMode::SourceOver).unwrap(),
    ]);
    assert_ne!(base.bytes(), moved.bytes());
    let faded = composite_case(vec![
        CompositeLayer::new(source_ref(0x0c), -3, 4, 250, BlendMode::SourceOver).unwrap(),
    ]);
    assert_ne!(base.bytes(), faded.bytes());
    let swapped_source = composite_case(vec![
        CompositeLayer::new(source_ref(0x0d), -3, 4, 500, BlendMode::SourceOver).unwrap(),
    ]);
    assert_ne!(base.bytes(), swapped_source.bytes());
    let longer = composite_case(vec![
        CompositeLayer::new(source_ref(0x0c), -3, 4, 500, BlendMode::SourceOver).unwrap(),
        CompositeLayer::new(source_ref(0x0d), 7, -8, 1000, BlendMode::SourceOver).unwrap(),
    ]);
    assert_ne!(base.bytes(), longer.bytes());
}

fn raster_case(alpha: AlphaHandling, encoding: Encoding) -> Vec<u8> {
    let delivered = RasterOutputSpec::new(Dimensions::new(64, 64).unwrap(), alpha, encoding).unwrap();
    let mut request = generate_request();
    request.output = OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), delivered);
    encode_intent(&request, &limits(), MAX_BYTES).unwrap().bytes().to_vec()
}

#[test]
fn output_profile_fields_change_the_bytes() {
    let opaque_jpeg = raster_case(
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(85).unwrap(),
        },
    );
    assert_eq!(opaque_jpeg, GENERATE_GOLDEN);

    let other_quality = raster_case(
        AlphaHandling::Opaque,
        Encoding::Jpeg {
            quality: LossyQuality::new(60).unwrap(),
        },
    );
    assert_ne!(opaque_jpeg, other_quality);

    let webp_lossless = raster_case(AlphaHandling::Opaque, Encoding::WebP { quality: None });
    assert_ne!(opaque_jpeg, webp_lossless);
    let webp_quality = raster_case(
        AlphaHandling::Opaque,
        Encoding::WebP {
            quality: Some(LossyQuality::new(60).unwrap()),
        },
    );
    assert_ne!(webp_lossless, webp_quality);

    let png = raster_case(AlphaHandling::Opaque, Encoding::Png);
    assert_ne!(opaque_jpeg, png);

    let preserved = raster_case(AlphaHandling::PreserveAlpha, Encoding::Png);
    assert_ne!(png, preserved);

    let matte = raster_case(
        AlphaHandling::Matte(Rgb {
            red: 10,
            green: 20,
            blue: 30,
        }),
        Encoding::Png,
    );
    assert_ne!(png, matte);
    let matte_other = raster_case(
        AlphaHandling::Matte(Rgb {
            red: 11,
            green: 20,
            blue: 30,
        }),
        Encoding::Png,
    );
    assert_ne!(matte, matte_other);

    let smaller = {
        let delivered =
            RasterOutputSpec::new(Dimensions::new(48, 64).unwrap(), AlphaHandling::Opaque, Encoding::Png)
                .unwrap();
        let mut request = generate_request();
        request.output = OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), delivered);
        encode_intent(&request, &limits(), MAX_BYTES).unwrap().bytes().to_vec()
    };
    assert_ne!(png, smaller);

    let mut local = transform_request();
    assert_intent_round_trip(&local);
    local.output = provider_output();
    assert!(encode_intent(&local, &limits(), MAX_BYTES).is_err());
}

#[test]
fn mismatched_operation_and_output_profiles_are_refused() {
    let mut generate_local = generate_request();
    generate_local.output = local_output();
    assert!(encode_intent(&generate_local, &limits(), MAX_BYTES).is_err());

    let mut edit_local = generate_request();
    edit_local.operation = ImageOperation::Edit(EditSpec {
        prompt: TextPrompt::new("sharpen the portrait", 5).unwrap(),
        base: source_ref(0x09),
        references: vec![source_ref(0x07)],
    });
    edit_local.output = local_output();
    assert!(encode_intent(&edit_local, &limits(), MAX_BYTES).is_err());

    let mut transform_remote = transform_request();
    transform_remote.output = provider_output();
    assert!(encode_intent(&transform_remote, &limits(), MAX_BYTES).is_err());
}

#[test]
fn withheld_audience_is_refused_before_encoding() {
    let mut request = generate_request();
    request.audience = Audience::Withheld;
    assert!(encode_intent(&request, &limits(), MAX_BYTES).is_err());
}

#[test]
fn prompt_too_long_is_refused_on_encode_and_decode() {
    let tight = RequestLimits {
        max_prompt_chars: 4,
        max_references: 8,
        max_transform_steps: 32,
        max_composite_layers: 8,
    };
    assert!(encode_intent(&generate_request(), &tight, MAX_BYTES).is_err());
    assert!(decode_intent(GENERATE_GOLDEN, &tight, MAX_BYTES).is_err());
    let mut short = generate_request();
    if let ImageOperation::Generate(spec) = &mut short.operation {
        spec.prompt = TextPrompt::new("dusk", 3).unwrap();
    }
    let encoded = encode_intent(&short, &tight, MAX_BYTES).unwrap();
    let decoded = decode_intent(encoded.bytes(), &tight, MAX_BYTES).unwrap();
    assert_eq!(decoded.operation, short.operation);
}

#[test]
fn overfull_counts_are_refused_on_encode_and_decode() {
    let mut many_refs = generate_request();
    if let ImageOperation::Generate(spec) = &mut many_refs.operation {
        spec.references = (0..9).map(source_ref).collect();
    }
    assert!(encode_intent(&many_refs, &limits(), MAX_BYTES).is_err());
    assert!(matches!(
        decode_intent(&patched(GENERATE_GOLDEN, 45, &[0, 0, 0, 9]), &limits(), MAX_BYTES),
        Err(CodecError::Request(RequestError::TooManyReferences { count: 9 }))
    ));

    let mut many_steps = transform_request();
    if let ImageOperation::Transform(spec) = &mut many_steps.operation {
        spec.steps = vec![ImageTransform::Flip(FlipAxis::Horizontal); 33];
    }
    assert!(encode_intent(&many_steps, &limits(), MAX_BYTES).is_err());

    let mut many_layers = transform_request();
    if let ImageOperation::Transform(spec) = &mut many_layers.operation {
        spec.steps = vec![ImageTransform::Composite(
            (0..9)
                .map(|byte| {
                    CompositeLayer::new(source_ref(byte), 0, 0, 500, BlendMode::SourceOver)
                        .unwrap()
                })
                .collect(),
        )];
    }
    assert!(encode_intent(&many_layers, &limits(), MAX_BYTES).is_err());

    let mut empty_layers = transform_request();
    if let ImageOperation::Transform(spec) = &mut empty_layers.operation {
        spec.steps = vec![ImageTransform::Composite(Vec::new())];
    }
    assert!(matches!(
        encode_intent(&empty_layers, &limits(), MAX_BYTES),
        Err(CodecError::Request(RequestError::InvalidTransform(
            TransformError::EmptyLayers
        )))
    ));

    assert!(matches!(
        decode_intent(&patched(TRANSFORM_GOLDEN, 49, &[0, 0, 0, 33]), &limits(), MAX_BYTES),
        Err(CodecError::Request(RequestError::InvalidTransform(
            TransformError::TooManySteps
        )))
    ));

    let mut one_layer = transform_request();
    if let ImageOperation::Transform(spec) = &mut one_layer.operation {
        spec.steps = vec![ImageTransform::Composite(vec![
            CompositeLayer::new(source_ref(0x0c), 0, 0, 500, BlendMode::SourceOver).unwrap(),
        ])];
    }
    let encoded = encode_intent(&one_layer, &limits(), MAX_BYTES).unwrap();
    assert!(matches!(
        decode_intent(&patched(encoded.bytes(), 54, &[0, 0, 0, 9]), &limits(), MAX_BYTES),
        Err(CodecError::Request(RequestError::InvalidTransform(
            TransformError::TooManyLayers
        )))
    ));
    assert!(matches!(
        decode_intent(&patched(encoded.bytes(), 54, &[0, 0, 0, 0]), &limits(), MAX_BYTES),
        Err(CodecError::Request(RequestError::InvalidTransform(
            TransformError::EmptyLayers
        )))
    ));
}

#[test]
fn config_snapshot_round_trips_every_field() {
    let mut config = ImageConfig::defaults().unwrap();
    config.mode = GlobalMode::LocalOnly;
    config.default_backend = BackendId::CodexCli;
    config.purposes[Purpose::ScenePlate.index()].backend_override = Some(BackendId::Fal);
    config.purposes[Purpose::ScenePlate.index()].enabled = false;
    config.request_limits.max_prompt_chars = 1024;
    let snapshot = encode_config(&config, MAX_BYTES).unwrap();
    let expected: [u8; 32] = Sha256::digest(snapshot.bytes()).into();
    assert_eq!(snapshot.fingerprint(), &expected);
    let decoded = decode_config(snapshot.bytes(), MAX_BYTES).unwrap();
    assert_eq!(decoded, config);
}

#[test]
fn full_field_config_round_trips() {
    let mut config = ImageConfig::defaults().unwrap();
    config.mode = GlobalMode::CacheOnly;
    config.default_backend = BackendId::Bfl;
    for (index, gate) in config.purposes.iter_mut().enumerate() {
        gate.enabled = index % 2 == 0;
        gate.backend_override = match index % 3 {
            0 => Some(BackendId::OpenAi),
            1 => Some(BackendId::Fal),
            _ => None,
        };
    }
    for (index, gate) in config.backends.iter_mut().enumerate() {
        gate.enabled = index == 2;
        gate.account = AccountHandle::new("staging-9").unwrap();
        gate.max_concurrent_jobs = 7 + index as u32;
    }
    config.transforms.enabled = false;
    config.transforms.max_concurrency = 5;
    config.transforms.max_decoded_bytes = 1_024;
    config.transforms.max_output_bytes = 2_048;
    config.transforms.queue_capacity = 9;
    config.transforms.deadline_secs = 45;
    config.scheduler.foreground_limit = 9;
    config.scheduler.presence_limit = 8;
    config.scheduler.background_limit = 7;
    config.scheduler.max_pending_jobs = 6;
    config.scheduler.max_pending_bytes = 5_000;
    config.scheduler.queue_age_secs = 400;
    config.scheduler.job_deadline_secs = 300;
    config.scheduler.stage_deadline_secs = 200;
    config.request_limits.max_prompt_chars = 512;
    config.request_limits.max_references = 3;
    config.request_limits.max_transform_steps = 9;
    config.request_limits.max_composite_layers = 2;
    let snapshot = encode_config(&config, MAX_BYTES).unwrap();
    let decoded = decode_config(snapshot.bytes(), MAX_BYTES).unwrap();
    assert_eq!(decoded, config);
}

#[test]
fn config_mode_and_backend_change_the_snapshot() {
    let base = encode_config(&ImageConfig::defaults().unwrap(), MAX_BYTES).unwrap();
    for mode in [
        GlobalMode::Live,
        GlobalMode::LocalOnly,
        GlobalMode::CacheOnly,
        GlobalMode::Off,
    ] {
        for backend in [
            BackendId::OpenAi,
            BackendId::Fal,
            BackendId::Bfl,
            BackendId::CodexCli,
        ] {
            let mut config = ImageConfig::defaults().unwrap();
            config.mode = mode;
            config.default_backend = backend;
            let recoded = encode_config(&config, MAX_BYTES).unwrap();
            if mode == GlobalMode::Live && backend == BackendId::OpenAi {
                assert_eq!(base.bytes(), recoded.bytes());
            } else {
                assert_ne!(base.bytes(), recoded.bytes());
            }
            let decoded = decode_config(recoded.bytes(), MAX_BYTES).unwrap();
            assert_eq!(decoded, config);
        }
    }
}

#[test]
fn every_purpose_gate_changes_the_snapshot() {
    let base = encode_config(&ImageConfig::defaults().unwrap(), MAX_BYTES).unwrap();
    for index in 0..Purpose::COUNT {
        let mut disabled = ImageConfig::defaults().unwrap();
        disabled.purposes[index].enabled = false;
        let recoded = encode_config(&disabled, MAX_BYTES).unwrap();
        assert_ne!(base.bytes(), recoded.bytes());

        let mut rerouted = ImageConfig::defaults().unwrap();
        rerouted.purposes[index].backend_override = Some(BackendId::Bfl);
        let recoded = encode_config(&rerouted, MAX_BYTES).unwrap();
        assert_ne!(base.bytes(), recoded.bytes());
        let decoded = decode_config(recoded.bytes(), MAX_BYTES).unwrap();
        assert_eq!(decoded, rerouted);
    }
}

#[test]
fn every_backend_gate_changes_the_snapshot() {
    let base = encode_config(&ImageConfig::defaults().unwrap(), MAX_BYTES).unwrap();
    for index in 0..BackendId::COUNT {
        let mut toggled = ImageConfig::defaults().unwrap();
        toggled.backends[index].enabled = !toggled.backends[index].enabled;
        let recoded = encode_config(&toggled, MAX_BYTES).unwrap();
        assert_ne!(base.bytes(), recoded.bytes());

        let mut rebound = ImageConfig::defaults().unwrap();
        rebound.backends[index].account = AccountHandle::new("quiet-relay-3").unwrap();
        let recoded = encode_config(&rebound, MAX_BYTES).unwrap();
        assert_ne!(base.bytes(), recoded.bytes());

        let mut busier = ImageConfig::defaults().unwrap();
        busier.backends[index].max_concurrent_jobs += 1;
        let recoded = encode_config(&busier, MAX_BYTES).unwrap();
        assert_ne!(base.bytes(), recoded.bytes());
        let decoded = decode_config(recoded.bytes(), MAX_BYTES).unwrap();
        assert_eq!(decoded, busier);
    }
}

#[test]
fn every_transform_and_scheduler_bound_changes_the_snapshot() {
    let base = encode_config(&ImageConfig::defaults().unwrap(), MAX_BYTES).unwrap();
    let mut each = ImageConfig::defaults().unwrap();
    each.transforms.enabled = false;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.transforms.max_concurrency += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.transforms.max_decoded_bytes += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.transforms.max_output_bytes += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.transforms.queue_capacity += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.transforms.deadline_secs += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());

    each = ImageConfig::defaults().unwrap();
    each.scheduler.foreground_limit += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.scheduler.presence_limit += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.scheduler.background_limit += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.scheduler.max_pending_jobs += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.scheduler.max_pending_bytes += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.scheduler.queue_age_secs += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.scheduler.job_deadline_secs += 1;
    assert_ne!(base.bytes(), encode_config(&each, MAX_BYTES).unwrap().bytes());
    each = ImageConfig::defaults().unwrap();
    each.scheduler.stage_deadline_secs += 1;
    let recoded = encode_config(&each, MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), recoded.bytes());
    let decoded = decode_config(recoded.bytes(), MAX_BYTES).unwrap();
    assert_eq!(decoded, each);
}

#[test]
fn every_request_limit_changes_the_snapshot() {
    let base = encode_config(&ImageConfig::defaults().unwrap(), MAX_BYTES).unwrap();
    let mut chars = ImageConfig::defaults().unwrap();
    chars.request_limits.max_prompt_chars += 1;
    assert_ne!(base.bytes(), encode_config(&chars, MAX_BYTES).unwrap().bytes());
    let mut refs = ImageConfig::defaults().unwrap();
    refs.request_limits.max_references += 1;
    assert_ne!(base.bytes(), encode_config(&refs, MAX_BYTES).unwrap().bytes());
    let mut steps = ImageConfig::defaults().unwrap();
    steps.request_limits.max_transform_steps += 1;
    assert_ne!(base.bytes(), encode_config(&steps, MAX_BYTES).unwrap().bytes());
    let mut layers = ImageConfig::defaults().unwrap();
    layers.request_limits.max_composite_layers += 1;
    let recoded = encode_config(&layers, MAX_BYTES).unwrap();
    assert_ne!(base.bytes(), recoded.bytes());
    let decoded = decode_config(recoded.bytes(), MAX_BYTES).unwrap();
    assert_eq!(decoded, layers);
}

#[test]
fn byte_bound_is_enforced_on_encode_and_decode() {
    let request = generate_request();
    let encoded = encode_intent(&request, &limits(), MAX_BYTES).unwrap();
    assert!(encoded.bytes().len() < MAX_BYTES);
    assert!(encode_intent(&request, &limits(), 10).is_err());
    let exact = encode_intent(&request, &limits(), encoded.bytes().len()).unwrap();
    assert_eq!(exact.bytes(), encoded.bytes());
    assert!(decode_intent(encoded.bytes(), &limits(), encoded.bytes().len() - 1).is_err());
    assert!(decode_intent(encoded.bytes(), &limits(), 0).is_err());
    assert!(encode_intent(&request, &limits(), encoded.bytes().len() - 1).is_err());
}

#[test]
fn config_rejects_bounds_versions_and_trailing_bytes() {
    let snapshot = encode_config(&ImageConfig::defaults().unwrap(), MAX_BYTES).unwrap();
    assert!(encode_config(&ImageConfig::defaults().unwrap(), 10).is_err());
    assert!(decode_config(snapshot.bytes(), snapshot.bytes().len() - 1).is_err());
    let mut trailing = snapshot.bytes().to_vec();
    trailing.push(0);
    assert!(decode_config(&trailing, MAX_BYTES).is_err());
    let mut versioned = snapshot.bytes().to_vec();
    versioned[0] = 2;
    assert!(matches!(
        decode_config(&versioned, MAX_BYTES),
        Err(CodecError::BadVersion { actual: 2 })
    ));
    let mut tagged = snapshot.bytes().to_vec();
    tagged[1] = 9;
    assert!(decode_config(&tagged, MAX_BYTES).is_err());
}

#[test]
fn unknown_tags_are_rejected_at_every_wire_position() {
    for (base, at) in [
        (GENERATE_GOLDEN, 2),
        (GENERATE_GOLDEN, 3),
        (GENERATE_GOLDEN, 4),
        (GENERATE_GOLDEN, 97),
        (GENERATE_GOLDEN, 98),
        (GENERATE_GOLDEN, 99),
        (GENERATE_GOLDEN, 100),
        (GENERATE_GOLDEN, 109),
        (GENERATE_GOLDEN, 126),
        (GENERATE_GOLDEN, 127),
        (TRANSFORM_GOLDEN, 37),
        (TRANSFORM_GOLDEN, 53),
        (TRANSFORM_GOLDEN, 62),
        (TRANSFORM_GOLDEN, 63),
        (TRANSFORM_GOLDEN, 64),
        (TRANSFORM_GOLDEN, 65),
        (TRANSFORM_GOLDEN, 66),
        (TRANSFORM_GOLDEN, 67),
        (TRANSFORM_GOLDEN, 69),
        (TRANSFORM_GOLDEN, 113),
    ] {
        assert!(
            decode_intent(&patched(base, at, &[9]), &limits(), MAX_BYTES).is_err(),
            "tag at offset {at} was accepted"
        );
    }
    assert!(matches!(
        decode_intent(&patched(GENERATE_GOLDEN, 1, &[12]), &limits(), MAX_BYTES),
        Err(CodecError::Malformed)
    ));
    assert!(matches!(
        decode_config(&patched(CONFIG_GOLDEN, 1, &[9]), MAX_BYTES),
        Err(CodecError::Malformed)
    ));
    assert!(matches!(
        decode_config(&patched(CONFIG_GOLDEN, 2, &[9]), MAX_BYTES),
        Err(CodecError::Malformed)
    ));
    assert!(matches!(
        decode_config(&patched(CONFIG_GOLDEN, 4, &[9]), MAX_BYTES),
        Err(CodecError::Malformed)
    ));
}

#[test]
fn deep_step_tags_are_rejected() {
    let step_case = |step: ImageTransform| {
        let mut request = transform_request();
        request.identity.owner = ImageOwner::Draft {
            account: account(0x11),
        };
        if let ImageOperation::Transform(spec) = &mut request.operation {
            spec.steps = vec![step];
        }
        encode_intent(&request, &limits(), MAX_BYTES).unwrap().bytes().to_vec()
    };
    let rotate = step_case(ImageTransform::Rotate(RotateSpec::QuarterTurns(1)));
    assert!(decode_intent(&patched(&rotate, 38, &[9]), &limits(), MAX_BYTES).is_err());
    assert!(decode_intent(&patched(&rotate, 39, &[9]), &limits(), MAX_BYTES).is_err());

    let color = step_case(ImageTransform::Color(ColorEffect::Grayscale));
    assert!(decode_intent(&patched(&color, 38, &[9]), &limits(), MAX_BYTES).is_err());

    let flipped = step_case(ImageTransform::Flip(FlipAxis::Horizontal));
    assert!(decode_intent(&patched(&flipped, 38, &[9]), &limits(), MAX_BYTES).is_err());

    let mask = step_case(ImageTransform::Mask(
        MaskSpec::new(
            MaskSource::Asset(source_ref(0x0a)),
            Dimensions::new(32, 32).unwrap(),
            AlignAnchor::center(),
            MaskChannel::Alpha,
            false,
            1.5,
            MaskMode::Multiply,
        )
        .unwrap(),
    ));
    assert!(decode_intent(&patched(&mask, 38, &[9]), &limits(), MAX_BYTES).is_err());
    assert!(decode_intent(&patched(&mask, 98, &[9]), &limits(), MAX_BYTES).is_err());
    assert!(decode_intent(&patched(&mask, 99, &[9]), &limits(), MAX_BYTES).is_err());
    assert!(decode_intent(&patched(&mask, 104, &[9]), &limits(), MAX_BYTES).is_err());

    let composite = step_case(ImageTransform::Composite(vec![
        CompositeLayer::new(source_ref(0x0c), 0, 0, 500, BlendMode::SourceOver).unwrap(),
    ]));
    assert!(decode_intent(&patched(&composite, 101, &[9]), &limits(), MAX_BYTES).is_err());
}

#[test]
fn every_strict_prefix_is_truncated() {
    for end in 0..GENERATE_GOLDEN.len() {
        assert!(
            decode_intent(&GENERATE_GOLDEN[..end], &limits(), MAX_BYTES).is_err(),
            "prefix len {end} was accepted"
        );
    }
    for end in 0..TRANSFORM_GOLDEN.len() {
        assert!(
            decode_intent(&TRANSFORM_GOLDEN[..end], &limits(), MAX_BYTES).is_err(),
            "prefix len {end} was accepted"
        );
    }
    for end in 0..CONFIG_GOLDEN.len() {
        assert!(
            decode_config(&CONFIG_GOLDEN[..end], MAX_BYTES).is_err(),
            "prefix len {end} was accepted"
        );
    }
}

#[test]
fn trailing_bytes_are_rejected_with_the_extra_count() {
    let mut one = GENERATE_GOLDEN.to_vec();
    one.push(0);
    assert!(matches!(
        decode_intent(&one, &limits(), MAX_BYTES),
        Err(CodecError::Trailing { extra_bytes: 1 })
    ));
    let mut three = TRANSFORM_GOLDEN.to_vec();
    three.extend_from_slice(&[0, 0, 0]);
    assert!(matches!(
        decode_intent(&three, &limits(), MAX_BYTES),
        Err(CodecError::Trailing { extra_bytes: 3 })
    ));
    let mut config_trailing = CONFIG_GOLDEN.to_vec();
    config_trailing.extend_from_slice(&[0, 0]);
    assert!(matches!(
        decode_config(&config_trailing, MAX_BYTES),
        Err(CodecError::Trailing { extra_bytes: 2 })
    ));
}

#[test]
fn version_mismatch_names_the_bad_version() {
    assert!(matches!(
        decode_intent(&patched(GENERATE_GOLDEN, 0, &[2]), &limits(), MAX_BYTES),
        Err(CodecError::BadVersion { actual: 2 })
    ));
    assert!(decode_intent(&[], &limits(), MAX_BYTES).is_err());
}

#[test]
fn unproven_vector_counts_are_refused_before_allocation() {
    assert!(matches!(
        decode_intent(&patched(GENERATE_GOLDEN, 45, &[0, 0, 0, 8]), &limits(), MAX_BYTES),
        Err(CodecError::TooLong)
    ));
    let generous = RequestLimits {
        max_prompt_chars: 32_000,
        max_references: 4_294_967_295,
        max_transform_steps: 32,
        max_composite_layers: 8,
    };
    assert!(matches!(
        decode_intent(
            &patched(GENERATE_GOLDEN, 45, &[0xFF, 0xFF, 0xFF, 0xFF]),
            &generous,
            MAX_BYTES
        ),
        Err(CodecError::TooLong)
    ));

    let mut members_request = generate_request();
    members_request.audience = Audience::Members(
        campaign(0x0c),
        MemberSet::new(vec![member(0x01), member(0x02)]).unwrap(),
    );
    let encoded = encode_intent(&members_request, &limits(), MAX_BYTES).unwrap();
    assert!(matches!(
        decode_intent(
            &patched(encoded.bytes(), 117, &[0xFF, 0xFF, 0xFF, 0xFF]),
            &limits(),
            MAX_BYTES
        ),
        Err(CodecError::TooLong)
    ));
    assert!(matches!(
        decode_intent(&patched(encoded.bytes(), 117, &[0, 0, 0, 8]), &limits(), MAX_BYTES),
        Err(CodecError::TooLong)
    ));

    let loose = RequestLimits {
        max_prompt_chars: 32_000,
        max_references: 8,
        max_transform_steps: 1_000,
        max_composite_layers: 8,
    };
    assert!(matches!(
        decode_intent(
            &patched(TRANSFORM_GOLDEN, 49, &[0, 0, 1, 244]),
            &loose,
            MAX_BYTES
        ),
        Err(CodecError::TooLong)
    ));
}

#[test]
fn non_finite_floats_are_rejected() {
    let mut request = transform_request();
    request.identity.owner = ImageOwner::Draft {
        account: account(0x11),
    };
    if let ImageOperation::Transform(spec) = &mut request.operation {
        spec.steps = vec![ImageTransform::Brighten(BrightnessOffset::new(1.5).unwrap())];
    }
    let encoded = encode_intent(&request, &limits(), MAX_BYTES).unwrap();
    assert!(matches!(
        decode_intent(&patched(encoded.bytes(), 38, &[0x7f, 0xc0, 0, 0]), &limits(), MAX_BYTES),
        Err(CodecError::NonFinite { field: "brightness" })
    ));
    assert!(matches!(
        decode_intent(&patched(encoded.bytes(), 38, &[0x7f, 0x80, 0, 0]), &limits(), MAX_BYTES),
        Err(CodecError::NonFinite { field: "brightness" })
    ));
}

#[test]
fn invalid_text_and_ids_are_rejected() {
    assert!(matches!(
        decode_intent(&patched(GENERATE_GOLDEN, 29, &[0xFF]), &limits(), MAX_BYTES),
        Err(CodecError::InvalidText)
    ));
    assert!(matches!(
        decode_intent(&patched(GENERATE_GOLDEN, 25, &[0, 0, 0, 0]), &limits(), MAX_BYTES),
        Err(CodecError::Request(RequestError::EmptyPrompt))
    ));
    assert!(matches!(
        decode_intent(&patched(GENERATE_GOLDEN, 5, &[0; 16]), &limits(), MAX_BYTES),
        Err(CodecError::InvalidId)
    ));
}

#[test]
fn debug_output_redacts_prompt_text_and_handles() {
    let encoded = encode_intent(&generate_request(), &limits(), MAX_BYTES).unwrap();
    let rendered = format!("{encoded:?}");
    assert!(!rendered.contains("tavern"), "{rendered}");
    let decoded = decode_intent(encoded.bytes(), &limits(), MAX_BYTES).unwrap();
    let rendered = format!("{decoded:?}");
    assert!(!rendered.contains("tavern"), "{rendered}");

    let mut config = ImageConfig::defaults().unwrap();
    for gate in &mut config.backends {
        gate.account = AccountHandle::new("quiet-relay-3").unwrap();
    }
    let snapshot = encode_config(&config, MAX_BYTES).unwrap();
    let rendered = format!("{snapshot:?}");
    assert!(!rendered.contains("quiet-relay-3"), "{rendered}");
    assert!(!rendered.contains("default"), "{rendered}");
}

#[test]
fn edit_intent_with_base_and_members_round_trips() {
    let mut request = generate_request();
    request.operation = ImageOperation::Edit(EditSpec {
        prompt: TextPrompt::new("sharpen the portrait", 5).unwrap(),
        base: source_ref(0x09),
        references: vec![source_ref(0x07)],
    });
    request.purpose = PurposedUse {
        purpose: Purpose::SceneEdit,
        role: VariantRole::Mask,
    };
    request.audience = Audience::Members(
        campaign(0x0c),
        MemberSet::new(vec![member(0x02), member(0x01)]).unwrap(),
    );
    request.freshness = Freshness::Regenerate {
        nonce: operation(0x55),
    };
    request.execution = ExecutionMode::PreparedOnly;
    assert_intent_round_trip(&request);
}
