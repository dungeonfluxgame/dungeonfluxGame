use super::encode::encode_decoded_intent;
use super::{CodecError, decode_intent, encode_intent};
use crate::*;
use df_types::{CampaignId, OperationId, RunId};

const MAXIMUM_BYTES: usize = 1_048_576;

fn request(with_pipeline: bool) -> ImageRequest {
    let campaign = CampaignId::from_bytes(&[1; 16]).unwrap();
    let run = RunId::from_bytes(&[2; 16]).unwrap();
    let artwork = ArtworkBinding {
        spec: CampaignArtworkSpec {
            campaign,
            subject: ArtworkSubject::Hero(ArtworkSubjectKey::new("public-library/hero").unwrap()),
            public_revision: [3; 32],
        },
        selection: RunSelectionFence {
            run,
            binding: [4; 32],
        },
    };
    let mut prompt = TextPrompt::new(
        "one public authored hero with the stable campaign binding",
        2,
    )
    .unwrap()
    .with_artwork(artwork);
    if with_pipeline {
        prompt = prompt
            .with_pipeline(ImagePipeline {
                revision: 1,
                moderation: Some(ProviderModeration::Auto),
                rating: ImageContentRating::Mature,
                creator: None,
                inspection: None,
            })
            .unwrap();
    }
    ImageRequest {
        identity: OperationIdentity {
            owner: ImageOwner::Run { campaign, run },
            operation: OperationId::from_bytes(&[5; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::CanonicalPerson,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt,
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
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
    }
}

#[test]
fn artwork_v6_round_trips_exact_metadata_with_and_without_pipeline() {
    for with_pipeline in [false, true] {
        let request = request(with_pipeline);
        let encoded = encode_intent(&request, &RequestLimits::default(), MAXIMUM_BYTES).unwrap();
        assert_eq!(encoded.version(), 6);
        let decoded =
            decode_intent(encoded.bytes(), &RequestLimits::default(), MAXIMUM_BYTES).unwrap();
        assert_eq!(decoded.owner, request.identity.owner);
        assert_eq!(decoded.operation, request.operation);
        assert_eq!(decoded.operation.artwork(), request.operation.artwork());
        let round_trip = encode_decoded_intent(&decoded, MAXIMUM_BYTES).unwrap();
        assert_eq!(round_trip.bytes(), encoded.bytes());
    }
}

#[test]
fn changing_run_selection_changes_the_receipt_but_preserves_the_complete_provider_text() {
    let first = request(true);
    let mut next = first.clone();
    let run = RunId::from_bytes(&[6; 16]).unwrap();
    let ImageOwner::Run { campaign, .. } = first.identity.owner else {
        unreachable!()
    };
    next.identity.owner = ImageOwner::Run { campaign, run };
    let ImageOperation::Generate(spec) = &mut next.operation else {
        unreachable!()
    };
    let mut metadata = spec.prompt.artwork().unwrap().clone();
    metadata.selection.run = run;
    metadata.selection.binding = [8; 32];
    let text = spec.prompt.text().to_owned();
    spec.prompt = spec.prompt.clone().with_artwork(metadata);
    assert_eq!(spec.prompt.text(), text);
    assert_eq!(
        first.operation.artwork().unwrap().spec.binding_digest(),
        next.operation.artwork().unwrap().spec.binding_digest(),
    );
    let first = encode_intent(&first, &RequestLimits::default(), MAXIMUM_BYTES).unwrap();
    let next = encode_intent(&next, &RequestLimits::default(), MAXIMUM_BYTES).unwrap();
    assert_ne!(first.fingerprint(), next.fingerprint());
}

#[test]
fn artwork_intent_rejects_every_truncation_trailing_bytes_and_unknown_spec_version() {
    let request = request(false);
    let encoded = encode_intent(&request, &RequestLimits::default(), MAXIMUM_BYTES).unwrap();
    for end in 0..encoded.bytes().len() {
        assert!(
            decode_intent(
                &encoded.bytes()[..end],
                &RequestLimits::default(),
                MAXIMUM_BYTES,
            )
            .is_err()
        );
    }
    let mut trailing = encoded.bytes().to_vec();
    trailing.push(0);
    assert!(matches!(
        decode_intent(&trailing, &RequestLimits::default(), MAXIMUM_BYTES),
        Err(CodecError::Trailing { .. }),
    ));
    let key_bytes = request
        .operation
        .artwork()
        .unwrap()
        .spec
        .subject
        .key()
        .as_str()
        .len();
    let metadata_bytes = 4 + 16 + 1 + 4 + key_bytes + 32 + 16 + 32;
    let start = encoded.bytes().len() - metadata_bytes;
    let mut unknown = encoded.bytes().to_vec();
    unknown[start..start + 4].copy_from_slice(&2_u32.to_be_bytes());
    assert!(matches!(
        decode_intent(&unknown, &RequestLimits::default(), MAXIMUM_BYTES),
        Err(CodecError::Request(RequestError::InvalidArtwork(
            ArtworkError::UnsupportedVersion { actual: 2 },
        ))),
    ));
    let mut oversized = encoded.bytes().to_vec();
    oversized[start + 21..start + 25].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(
        decode_intent(&oversized, &RequestLimits::default(), MAXIMUM_BYTES),
        Err(CodecError::TooLong),
    ));
}

#[test]
fn valid_explicit_regeneration_keeps_full_artwork_metadata() {
    let mut request = request(true);
    request.freshness = Freshness::Regenerate {
        nonce: OperationId::from_bytes(&[9; 16]).unwrap(),
    };
    let encoded = encode_intent(&request, &RequestLimits::default(), MAXIMUM_BYTES).unwrap();
    let decoded = decode_intent(encoded.bytes(), &RequestLimits::default(), MAXIMUM_BYTES).unwrap();
    assert_eq!(decoded.freshness, request.freshness);
    assert_eq!(decoded.operation.artwork(), request.operation.artwork());
}

#[test]
fn current_run_scope_mismatch_is_refused_before_encoding_any_artwork_intent() {
    let mut request = request(false);
    let ImageOwner::Run { campaign, .. } = request.identity.owner else {
        unreachable!()
    };
    request.identity.owner = ImageOwner::Run {
        campaign,
        run: RunId::from_bytes(&[10; 16]).unwrap(),
    };
    assert!(matches!(
        encode_intent(&request, &RequestLimits::default(), MAXIMUM_BYTES),
        Err(CodecError::Request(RequestError::InvalidArtwork(
            ArtworkError::OwnerScopeMismatch,
        ))),
    ));
}
