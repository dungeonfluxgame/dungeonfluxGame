use super::*;
use crate::RequestOutcome;
use crate::publication::ReadyArtifact;
use df_types::{CampaignId, RunId};

fn public_request(run_byte: u8) -> ImageRequest {
    let mut request = request();
    let campaign = CampaignId::from_bytes(&[81; 16]).unwrap();
    let run = RunId::from_bytes(&[run_byte; 16]).unwrap();
    request.identity.owner = ImageOwner::Run { campaign, run };
    request.purpose.purpose = Purpose::CanonicalPerson;
    request.audience = Audience::CampaignShared(campaign);
    let ImageOperation::Generate(spec) = &mut request.operation else {
        unreachable!()
    };
    spec.prompt = TextPrompt::new("the exact stable public artwork text", 0xdf02_0001)
        .unwrap()
        .with_artwork(crate::ArtworkBinding {
            spec: crate::CampaignArtworkSpec {
                campaign,
                subject: crate::ArtworkSubject::Hero(
                    crate::ArtworkSubjectKey::new("library/committed-hero").unwrap(),
                ),
                public_revision: [82; 32],
            },
            selection: crate::RunSelectionFence {
                run,
                binding: [run_byte; 32],
            },
        });
    request
}

fn artifact(request: &ImageRequest, config: &ImageConfig) -> ReadyArtifact {
    let (_, config) = snapshots(request, config);
    let recipe = crate::campaign_artifact_recipe(request, &config)
        .unwrap()
        .unwrap();
    ReadyArtifact {
        asset: AssetId::from_bytes(&[83; 16]).unwrap(),
        attempt: AttemptId::from_bytes(&[84; 16]).unwrap(),
        audience: request.audience.clone(),
        semantic_key: recipe.key,
        published_at_ms: NOW_MS - 1,
        variants: vec![PublishedVariant {
            variant: AssetVariant::Master,
            write: WriteId::from_bytes(&[85; 16]).unwrap(),
            object_version: ObjectVersion::new("actual-test-artifact-version".to_owned()).unwrap(),
            byte_len: 23,
            sha256: [86; 32],
            intent: ImageIntent::new("image/png", 1, 1, ImageAlpha::Opaque).unwrap(),
            alpha: ImageAlpha::Opaque,
        }],
        inspection: None,
        staging: Vec::new(),
    }
}

#[tokio::test]
async fn authorized_artifact_hit_in_each_cache_enabled_mode_mints_no_demand_or_usage() {
    for mode in [GlobalMode::Live, GlobalMode::CacheOnly] {
        for execution in [
            ExecutionMode::Live,
            ExecutionMode::PreparedOnly,
            ExecutionMode::Replay,
        ] {
            let mut config = ImageConfig::defaults().unwrap();
            config.mode = mode;
            let mut request = public_request(87);
            request.execution = execution;
            let artifact = artifact(&request, &config);
            let mut service = service(config, Vec::new(), None);
            service.results.artifact = Some(artifact.clone());
            request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
            assert_eq!(
                service.request(&(), &request).await.unwrap(),
                RequestOutcome::ReusedArtifact(artifact)
            );
            assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
            assert_eq!(service.journal.pages.load(Ordering::SeqCst), 0);
            assert_eq!(service.results.reads.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn current_grant_refusal_never_delivers_artifact_or_starts_lookup_only_work() {
    let config = ImageConfig::defaults().unwrap();
    let mut request = public_request(87);
    request.execution = ExecutionMode::PreparedOnly;
    let artifact = artifact(&request, &config);
    let mut service = service(config, Vec::new(), None);
    service.results.artifact = Some(artifact);
    service.results.current_grant.store(false, Ordering::SeqCst);
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert!(matches!(
        service.request(&(), &request).await.unwrap(),
        RequestOutcome::Refused(_)
    ));
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn artifact_key_match_never_bypasses_exact_audience_output_or_inspection_checks() {
    let config = ImageConfig::defaults().unwrap();
    let mut request = public_request(87);
    request.execution = ExecutionMode::PreparedOnly;
    let original = artifact(&request, &config);
    let mut wrong_audience = original.clone();
    wrong_audience.audience = Audience::PublicStatic;
    let mut wrong_output = original.clone();
    wrong_output.variants[0].intent =
        ImageIntent::new("image/png", 2, 1, ImageAlpha::Opaque).unwrap();
    let mut wrong_inspection = original;
    wrong_inspection.inspection = Some(crate::PhotoInspectionAssessment {
        contract_revision: 1,
        verdict: crate::PhotoInspectionVerdict::Pass,
        visible_features: Vec::new(),
        source_binding: None,
    });
    for artifact in [wrong_audience, wrong_output, wrong_inspection] {
        let mut service = service(config.clone(), Vec::new(), None);
        service.results.artifact = Some(artifact);
        request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
        assert_eq!(
            service.request(&(), &request).await.unwrap_err(),
            ImageServiceError::InvalidRecord
        );
        assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn legacy_and_explicit_regeneration_never_search_public_artifact_lane() {
    let config = ImageConfig::defaults().unwrap();
    let mut regenerate = public_request(87);
    regenerate.freshness = Freshness::Regenerate {
        nonce: OperationId::from_bytes(&[88; 16]).unwrap(),
    };
    let mut legacy = public_request(87);
    let ImageOperation::Generate(spec) = &mut legacy.operation else {
        unreachable!()
    };
    spec.prompt = TextPrompt::new(spec.prompt.text(), spec.prompt.template_version).unwrap();
    for mut request in [regenerate, legacy] {
        request.execution = ExecutionMode::PreparedOnly;
        let service = service(config.clone(), Vec::new(), None);
        request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
        assert!(matches!(
            service.request(&(), &request).await.unwrap(),
            RequestOutcome::Refused(_)
        ));
        assert_eq!(service.results.reads.load(Ordering::SeqCst), 0);
        assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn an_existing_exact_receipt_is_never_replaced_by_a_new_artifact_hit() {
    let config = ImageConfig::defaults().unwrap();
    let original = public_request(87);
    let view = stored_view(&original, &config, 89);
    let artifact = artifact(&original, &config);
    let mut request = original;
    request.identity.operation = view.receipt.operation;
    request.execution = ExecutionMode::PreparedOnly;
    let mut service = service(config, vec![view], None);
    service.results.artifact = Some(artifact);
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert_eq!(
        service.request(&(), &request).await.unwrap_err(),
        ImageServiceError::Conflict
    );
    assert_eq!(service.results.reads.load(Ordering::SeqCst), 0);
    assert_eq!(service.ids.0.load(Ordering::SeqCst), 0);
}

#[path = "artifact_owned_projection_tests.rs"]
mod owned_projection;
