//! Unit tests for the private-principal image authorizer: draft equality,
//! current run membership and audience, and authorized reference pins. The
//! principals here come from the test-only constructors; production builds
//! them only through the async resolvers over stored bindings.

use super::imagery_identity::{
    AssetReferenceFacts, ExactAssetAllowlist, ImageryAuthorizer, ImageryPrincipal,
};
use df_assets::{AssetId, Audience, MemberSet};
use df_imagery::{
    AssetDigest, AuthDenied, Dimensions, Encoding, ExecutionMode, Freshness, GenerateSpec,
    ImageAuthorizer as _, ImageOperation, ImageOwner, ImageRequest, OperationHandle,
    OperationIdentity, OperationView, OutputProfile, Purpose, PurposedUse, RasterOutputSpec,
    ServiceAction, SourceRef, StoredImageAuthorization, TextPrompt, Urgency,
};
use df_imagery::{
    ImageInterest, ImageInterestId, ImageJob, ImageJobId, InterestState, JobState,
    OperationReceipt, SharingIdentity, VersionedSnapshot,
};
use df_types::{AccountId, CampaignId, MemberId, OperationId, RunId};

fn id(byte: u8) -> [u8; 16] {
    [byte; 16]
}

fn hosted() -> ImageryPrincipal {
    ImageryPrincipal::hosted_for_tests(id(0x61), id(0x52), id(0x55), id(0x62), id(0x01))
}

fn local() -> ImageryPrincipal {
    ImageryPrincipal::local_for_tests(id(0x6a), id(0x6d))
}

fn run_owner() -> ImageOwner {
    ImageOwner::Run {
        campaign: CampaignId::from_bytes(&id(0x52)).expect("campaign"),
        run: RunId::from_bytes(&id(0x55)).expect("run"),
    }
}

fn own_draft() -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_bytes(&id(0x61)).expect("account"),
    }
}

fn output() -> OutputProfile {
    let delivered = RasterOutputSpec::new(
        Dimensions::new(64, 64).expect("dimensions"),
        df_imagery::AlphaHandling::Opaque,
        Encoding::Png,
    )
    .expect("output");
    OutputProfile::Local { delivered }
}

fn source(asset: u8) -> SourceRef {
    SourceRef {
        asset: AssetId::from_bytes(&id(asset)).expect("asset"),
        digest: AssetDigest::from_bytes([asset; 32]),
        audience: Audience::PublicStatic,
    }
}

fn request(owner: ImageOwner, audience: Audience, references: Vec<SourceRef>) -> ImageRequest {
    ImageRequest {
        identity: OperationIdentity {
            owner,
            operation: OperationId::from_bytes(&id(0x40)).expect("operation"),
            fingerprint: [0x41; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: df_imagery::VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("a calm harbor at dawn", 1).expect("prompt"),
            references,
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience,
        expected_source_revision: 1,
        output: output(),
    }
}

fn authorizer(assets: &[u8]) -> ImageryAuthorizer<ExactAssetAllowlist> {
    ImageryAuthorizer::new(ExactAssetAllowlist::new(
        assets.iter().map(|asset| source(*asset)).collect(),
    ))
}

fn campaign_audience_of(member: u8) -> Audience {
    let campaign = CampaignId::from_bytes(&id(0x52)).expect("campaign");
    let member = MemberId::from_bytes(&id(member)).expect("member");
    Audience::Members(campaign, MemberSet::new(vec![member]).expect("members"))
}

fn stored_view(owner: ImageOwner) -> (OperationView, OperationHandle) {
    let operation = OperationId::from_bytes(&id(0x40)).expect("operation");
    let job = ImageJobId::from_bytes(&id(0x71)).expect("job");
    let interest = ImageInterestId::from_bytes(&id(0x72)).expect("interest");
    let snapshot = VersionedSnapshot::new(1, vec![0x31; 64], 1 << 20).expect("snapshot");
    let view = OperationView {
        receipt: OperationReceipt {
            owner: owner.clone(),
            operation,
            fingerprint: [0x41; 32],
            job,
            interest,
        },
        job: ImageJob {
            job,
            owner: owner.clone(),
            sharing: SharingIdentity::from_bytes(&[0x33; 32]).expect("sharing"),
            intent: snapshot.clone(),
            config: snapshot,
            absolute_deadline_ms: NOW_MS,
            state: JobState::Planning,
            revision: 1,
            resolved_key: None,
            recipe: None,
            created_at_ms: NOW_MS,
            updated_at_ms: NOW_MS,
        },
        interest: ImageInterest {
            interest,
            owner: owner.clone(),
            job,
            operation,
            state: InterestState::Attached,
            created_at_ms: NOW_MS,
            withdrawn_at_ms: None,
        },
    };
    (view, OperationHandle { owner, operation })
}

const NOW_MS: i64 = 1_700_000_000_000;

#[test]
fn hosted_run_owner_authorizes_its_current_run_and_audience() {
    let authorizer = authorizer(&[]);
    let principal = hosted();
    let request = request(run_owner(), campaign_audience_of(0x62), Vec::new());
    assert!(authorizer.authorize_request(&principal, &request).is_ok());
    assert!(
        authorizer
            .authorize_owner(&principal, &request.identity.owner, ServiceAction::Read)
            .is_ok()
    );
}

#[test]
fn hosted_run_owner_refuses_another_run() {
    let authorizer = authorizer(&[]);
    let other = ImageOwner::Run {
        campaign: CampaignId::from_bytes(&id(0x52)).expect("campaign"),
        run: RunId::from_bytes(&id(0x59)).expect("run"),
    };
    let request = request(other, campaign_audience_of(0x62), Vec::new());
    assert_eq!(
        authorizer.authorize_request(&hosted(), &request),
        Err(AuthDenied)
    );
}

#[test]
fn draft_must_equal_the_principal_account() {
    let authorizer = authorizer(&[]);
    assert!(
        authorizer
            .authorize_request(
                &hosted(),
                &request(own_draft(), Audience::PublicStatic, Vec::new())
            )
            .is_ok()
    );
    let wrong = ImageOwner::Draft {
        account: AccountId::from_bytes(&id(0x69)).expect("account"),
    };
    assert_eq!(
        authorizer.authorize_request(
            &hosted(),
            &request(wrong, Audience::PublicStatic, Vec::new())
        ),
        Err(AuthDenied)
    );
    assert!(
        authorizer
            .authorize_request(
                &local(),
                &request(
                    ImageOwner::Draft {
                        account: AccountId::from_bytes(&id(0x6a)).expect("account"),
                    },
                    Audience::PublicStatic,
                    Vec::new()
                )
            )
            .is_ok()
    );
}

#[test]
fn local_guest_never_authorizes_a_run() {
    let authorizer = authorizer(&[]);
    let request = request(run_owner(), Audience::PublicStatic, Vec::new());
    assert_eq!(
        authorizer.authorize_request(&local(), &request),
        Err(AuthDenied)
    );
    assert_eq!(
        authorizer.authorize_owner(&local(), &request.identity.owner, ServiceAction::Read),
        Err(AuthDenied)
    );
}

#[test]
fn members_audience_requires_current_membership() {
    let authorizer = authorizer(&[]);
    assert!(
        authorizer
            .authorize_request(
                &hosted(),
                &request(run_owner(), campaign_audience_of(0x62), Vec::new())
            )
            .is_ok()
    );
    assert_eq!(
        authorizer.authorize_request(
            &hosted(),
            &request(run_owner(), campaign_audience_of(0x69), Vec::new())
        ),
        Err(AuthDenied)
    );
    assert_eq!(
        authorizer.authorize_request(
            &hosted(),
            &request(run_owner(), Audience::Withheld, Vec::new())
        ),
        Err(AuthDenied)
    );
}

#[test]
fn cross_campaign_audience_is_refused() {
    let authorizer = authorizer(&[]);
    let other = Audience::CampaignShared(CampaignId::from_bytes(&id(0x5a)).expect("campaign"));
    assert_eq!(
        authorizer.authorize_request(&hosted(), &request(run_owner(), other, Vec::new())),
        Err(AuthDenied)
    );
}

#[test]
fn source_pins_need_authorized_reference_facts_and_allow_no_all() {
    let pinned = request(run_owner(), campaign_audience_of(0x62), vec![source(0xa1)]);
    assert_eq!(
        authorizer(&[]).authorize_request(&hosted(), &pinned),
        Err(AuthDenied)
    );
    assert!(
        authorizer(&[0xa1])
            .authorize_request(&hosted(), &pinned)
            .is_ok()
    );
    let unpinned = request(run_owner(), campaign_audience_of(0x62), Vec::new());
    assert!(
        authorizer(&[])
            .authorize_request(&hosted(), &unpinned)
            .is_ok()
    );
}

#[test]
fn stored_views_reauthorize_owner_audience_and_pins() {
    let authorizer = authorizer(&[0xa1]);
    let (view, handle) = stored_view(run_owner());
    let operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("a calm harbor at dawn", 1).expect("prompt"),
        references: vec![source(0xa1)],
    });
    let stored = StoredImageAuthorization {
        handle: &handle,
        owner: &handle.owner,
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: df_imagery::VariantRole::Primary,
        },
        operation: &operation,
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        audience: &campaign_audience_of(0x62),
        expected_source_revision: 1,
        output: &output(),
    };
    assert!(
        authorizer
            .authorize_stored(&hosted(), ServiceAction::Read, &view, &stored)
            .is_ok()
    );
    let wrong = ImageOwner::Draft {
        account: AccountId::from_bytes(&id(0x69)).expect("account"),
    };
    let (other_view, other_handle) = stored_view(wrong);
    let other_stored = StoredImageAuthorization {
        handle: &other_handle,
        owner: &other_handle.owner,
        purpose: stored.purpose,
        operation: stored.operation,
        execution: stored.execution,
        freshness: stored.freshness,
        audience: stored.audience,
        expected_source_revision: stored.expected_source_revision,
        output: stored.output,
    };
    assert_eq!(
        authorizer.authorize_stored(&hosted(), ServiceAction::Read, &other_view, &other_stored),
        Err(AuthDenied)
    );
}

struct DenyAll;

impl AssetReferenceFacts for DenyAll {
    fn reference_authorized(&self, _source: &SourceRef) -> bool {
        false
    }
}

#[test]
fn deny_all_facts_refuse_every_pinned_request() {
    let authorizer = ImageryAuthorizer::new(DenyAll);
    let pinned = request(run_owner(), campaign_audience_of(0x62), vec![source(0xa1)]);
    assert_eq!(
        authorizer.authorize_request(&hosted(), &pinned),
        Err(AuthDenied)
    );
}

#[test]
fn private_and_host_audiences_reject_other_accounts_and_players_on_every_boundary() {
    let campaign = CampaignId::from_bytes(&id(0x52)).unwrap();
    for audience in [
        Audience::OwnerPrivate(AccountId::from_bytes(&id(0x69)).unwrap()),
        Audience::Host(campaign),
    ] {
        let authorizer = authorizer(&[]);
        let request = request(run_owner(), audience, Vec::new());
        assert_eq!(
            authorizer.authorize_request(&hosted(), &request),
            Err(AuthDenied)
        );
        let (view, handle) = stored_view(run_owner());
        let stored = StoredImageAuthorization {
            handle: &handle,
            owner: &handle.owner,
            purpose: request.purpose,
            operation: &request.operation,
            execution: request.execution,
            freshness: request.freshness,
            audience: &request.audience,
            expected_source_revision: 1,
            output: &request.output,
        };
        for action in [ServiceAction::Read, ServiceAction::Cancel] {
            assert_eq!(
                authorizer.authorize_stored(&hosted(), action, &view, &stored),
                Err(AuthDenied)
            );
        }
    }
    let private = Audience::OwnerPrivate(AccountId::from_bytes(&id(0x61)).unwrap());
    assert!(
        authorizer(&[])
            .authorize_request(&hosted(), &request(run_owner(), private, Vec::new()))
            .is_ok()
    );
    let other = Audience::OwnerPrivate(AccountId::from_bytes(&id(0x61)).unwrap());
    let own = ImageOwner::Draft {
        account: AccountId::from_bytes(&id(0x6a)).unwrap(),
    };
    assert_eq!(
        authorizer(&[]).authorize_request(&local(), &request(own, other, Vec::new())),
        Err(AuthDenied)
    );
}

#[test]
fn same_asset_with_wrong_digest_or_audience_is_rejected_for_generate_edit_and_transform() {
    for wrong_audience in [false, true] {
        let mut pin = source(0xa1);
        if wrong_audience {
            pin.audience = campaign_audience_of(0x62);
        } else {
            pin.digest = AssetDigest::from_bytes([0xff; 32]);
        }
        let operations = [
            ImageOperation::Generate(GenerateSpec {
                prompt: TextPrompt::new("harbor", 1).unwrap(),
                references: vec![pin.clone()],
            }),
            ImageOperation::Edit(df_imagery::EditSpec {
                prompt: TextPrompt::new("harbor", 1).unwrap(),
                base: pin.clone(),
                references: Vec::new(),
            }),
            ImageOperation::Transform(df_imagery::TransformSpec {
                source: df_imagery::TransformSource::Image(pin),
                steps: Vec::new(),
            }),
        ];
        for operation in operations {
            let mut request = request(run_owner(), campaign_audience_of(0x62), Vec::new());
            request.operation = operation;
            assert_eq!(
                authorizer(&[0xa1]).authorize_request(&hosted(), &request),
                Err(AuthDenied)
            );
        }
    }
}

#[test]
fn local_server_run_is_exact_and_has_no_draft_account_or_guest_elevation() {
    let principal = ImageryPrincipal::local_run_for_tests(id(0x52), id(0x55), id(0x56));
    let authorizer = authorizer(&[]);
    assert_eq!(
        principal.mode(),
        df_persistence::imagery_identity::AuthorityMode::LocalRun
    );
    assert_eq!(principal.account(), None);
    assert_eq!(principal.local_run_owner(), Some(&run_owner()));
    for audience in [
        Audience::PublicStatic,
        Audience::CampaignShared(CampaignId::from_bytes(&id(0x52)).unwrap()),
        Audience::Host(CampaignId::from_bytes(&id(0x52)).unwrap()),
    ] {
        assert!(
            authorizer
                .authorize_request(&principal, &request(run_owner(), audience, Vec::new()))
                .is_ok()
        );
    }
    for owner in [
        own_draft(),
        ImageOwner::Run {
            campaign: CampaignId::from_bytes(&id(0x52)).unwrap(),
            run: RunId::from_bytes(&id(0x59)).unwrap(),
        },
        ImageOwner::Run {
            campaign: CampaignId::from_bytes(&id(0x58)).unwrap(),
            run: RunId::from_bytes(&id(0x55)).unwrap(),
        },
    ] {
        assert_eq!(
            authorizer.authorize_request(
                &principal,
                &request(owner, Audience::PublicStatic, Vec::new())
            ),
            Err(AuthDenied)
        );
    }
    assert_eq!(
        authorizer.authorize_request(
            &local(),
            &request(run_owner(), Audience::PublicStatic, Vec::new())
        ),
        Err(AuthDenied)
    );
}

#[test]
fn local_server_run_never_reads_private_or_members_audiences_even_with_source_facts() {
    let principal = ImageryPrincipal::local_run_for_tests(id(0x52), id(0x55), id(0x56));
    for audience in [
        Audience::OwnerPrivate(AccountId::from_bytes(&id(0x61)).unwrap()),
        campaign_audience_of(0x62),
        Audience::Host(CampaignId::from_bytes(&id(0x58)).unwrap()),
    ] {
        let candidate = request(run_owner(), audience, Vec::new());
        let authorizer = authorizer(&[]);
        assert_eq!(
            authorizer.authorize_request(&principal, &candidate),
            Err(AuthDenied)
        );
        let (view, handle) = stored_view(run_owner());
        let stored = StoredImageAuthorization {
            handle: &handle,
            owner: &handle.owner,
            purpose: candidate.purpose,
            operation: &candidate.operation,
            execution: candidate.execution,
            freshness: candidate.freshness,
            audience: &candidate.audience,
            expected_source_revision: 1,
            output: &candidate.output,
        };
        for action in [ServiceAction::Read, ServiceAction::Cancel] {
            assert_eq!(
                authorizer.authorize_stored(&principal, action, &view, &stored),
                Err(AuthDenied)
            );
        }
        let private = SourceRef {
            audience: candidate.audience.clone(),
            ..source(0xa1)
        };
        let facts = ExactAssetAllowlist::new(vec![private.clone()]);
        let authorizer = ImageryAuthorizer::new(facts);
        assert_eq!(
            authorizer.authorize_request(
                &principal,
                &request(run_owner(), Audience::PublicStatic, vec![private])
            ),
            Err(AuthDenied)
        );
    }
}

#[test]
fn catalog_source_policy_reuses_exact_run_and_audience_scope_without_guest_elevation() {
    let authorizer = authorizer(&[]);
    let principal = ImageryPrincipal::local_run_for_tests(id(0x52), id(0x55), id(0x56));
    let shared = SourceRef {
        audience: Audience::CampaignShared(CampaignId::from_bytes(&id(0x52)).unwrap()),
        ..source(0xa1)
    };
    assert!(
        authorizer
            .authorize_catalog_source(&principal, &run_owner(), &shared)
            .is_ok()
    );
    let other = ImageOwner::Run {
        campaign: CampaignId::from_bytes(&id(0x52)).unwrap(),
        run: RunId::from_bytes(&id(0x59)).unwrap(),
    };
    assert_eq!(
        authorizer.authorize_catalog_source(&principal, &other, &shared),
        Err(AuthDenied)
    );
    assert_eq!(
        authorizer.authorize_catalog_source(&local(), &run_owner(), &shared),
        Err(AuthDenied)
    );
    for audience in [
        Audience::OwnerPrivate(AccountId::from_bytes(&id(0x61)).unwrap()),
        campaign_audience_of(0x62),
        Audience::Host(CampaignId::from_bytes(&id(0x58)).unwrap()),
    ] {
        let pin = SourceRef {
            audience,
            ..source(0xa1)
        };
        assert_eq!(
            authorizer.authorize_catalog_source(&principal, &run_owner(), &pin),
            Err(AuthDenied)
        );
    }
}

#[test]
fn catalog_source_policy_preserves_current_hosted_and_private_draft_boundaries() {
    let authorizer = authorizer(&[]);
    let account = AccountId::from_bytes(&id(0x6a)).unwrap();
    let draft = ImageOwner::Draft { account };
    let private = SourceRef {
        audience: Audience::OwnerPrivate(account),
        ..source(0xa1)
    };
    assert!(
        authorizer
            .authorize_catalog_source(&local(), &draft, &private)
            .is_ok()
    );
    assert_eq!(
        authorizer.authorize_catalog_source(&hosted(), &own_draft(), &private),
        Err(AuthDenied)
    );
    let host = SourceRef {
        audience: Audience::Host(CampaignId::from_bytes(&id(0x52)).unwrap()),
        ..source(0xa1)
    };
    assert_eq!(
        authorizer.authorize_catalog_source(&hosted(), &run_owner(), &host),
        Err(AuthDenied)
    );
}

struct AnalysisFacts {
    base: SourceRef,
    dependency: OperationHandle,
    current: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl AssetReferenceFacts for AnalysisFacts {
    fn reference_authorized(&self, source: &SourceRef) -> bool {
        source == &self.base
    }
    fn analysis_authorized(&self, handle: &OperationHandle, analyzed: &SourceRef) -> bool {
        self.current.load(std::sync::atomic::Ordering::Acquire)
            && handle == &self.dependency
            && analyzed == &self.base
    }
}
fn mask_request(owner: ImageOwner, dependency: OperationHandle) -> ImageRequest {
    let mut request = request(owner, Audience::PublicStatic, Vec::new());
    request.operation = ImageOperation::Transform(df_imagery::TransformSpec {
        source: df_imagery::TransformSource::SegmentationMask {
            analyzed: source(0xa1),
            segmentation: dependency,
            prompt: df_imagery::SegmentationPrompt::Person,
            mask_index: 0,
            contract_revision: 1,
        },
        steps: Vec::new(),
    });
    request
}
#[test]
fn segmentation_masks_require_current_exact_analysis_and_base_facts_on_request_and_read() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let dependency = OperationHandle {
        owner: run_owner(),
        operation: OperationId::from_bytes(&id(0x31)).unwrap(),
    };
    let request = mask_request(run_owner(), dependency.clone());
    assert_eq!(
        authorizer(&[0xa1]).authorize_request(&hosted(), &request),
        Err(AuthDenied),
        "base access cannot grant analytical access"
    );
    let current = Arc::new(AtomicBool::new(true));
    let policy = ImageryAuthorizer::new(AnalysisFacts {
        base: source(0xa1),
        dependency,
        current: current.clone(),
    });
    assert!(policy.authorize_request(&hosted(), &request).is_ok());
    let (view, handle) = stored_view(run_owner());
    let stored = StoredImageAuthorization {
        handle: &handle,
        owner: &handle.owner,
        purpose: request.purpose,
        operation: &request.operation,
        execution: request.execution,
        freshness: request.freshness,
        audience: &request.audience,
        expected_source_revision: 1,
        output: &request.output,
    };
    assert!(
        policy
            .authorize_stored(&hosted(), ServiceAction::Read, &view, &stored)
            .is_ok()
    );
    current.store(false, Ordering::Release);
    assert_eq!(
        policy.authorize_request(&hosted(), &request),
        Err(AuthDenied)
    );
    assert_eq!(
        policy.authorize_stored(&hosted(), ServiceAction::Read, &view, &stored),
        Err(AuthDenied)
    );
}
#[test]
fn segmentation_analysis_handle_never_substitutes_for_exact_work_owner_or_base_pin() {
    use std::sync::{Arc, atomic::AtomicBool};
    for dependency_owner in [
        own_draft(),
        ImageOwner::Run {
            campaign: CampaignId::from_bytes(&id(0x52)).unwrap(),
            run: RunId::from_bytes(&id(0x59)).unwrap(),
        },
    ] {
        let dependency = OperationHandle {
            owner: dependency_owner,
            operation: OperationId::from_bytes(&id(0x31)).unwrap(),
        };
        let policy = ImageryAuthorizer::new(AnalysisFacts {
            base: source(0xa1),
            dependency: dependency.clone(),
            current: Arc::new(AtomicBool::new(true)),
        });
        assert_eq!(
            policy.authorize_request(&hosted(), &mask_request(run_owner(), dependency)),
            Err(AuthDenied)
        );
    }
    let dependency = OperationHandle {
        owner: run_owner(),
        operation: OperationId::from_bytes(&id(0x31)).unwrap(),
    };
    let policy = ImageryAuthorizer::new(AnalysisFacts {
        base: source(0xa1),
        dependency: dependency.clone(),
        current: Arc::new(AtomicBool::new(true)),
    });
    let mut request = mask_request(run_owner(), dependency);
    let ImageOperation::Transform(spec) = &mut request.operation else {
        unreachable!()
    };
    let df_imagery::TransformSource::SegmentationMask { analyzed, .. } = &mut spec.source else {
        unreachable!()
    };
    *analyzed = source(0xa2);
    assert_eq!(
        policy.authorize_request(&hosted(), &request),
        Err(AuthDenied)
    );
    let ImageOperation::Transform(spec) = &mut request.operation else {
        unreachable!()
    };
    let df_imagery::TransformSource::SegmentationMask {
        analyzed,
        segmentation,
        ..
    } = &mut spec.source
    else {
        unreachable!()
    };
    *analyzed = source(0xa1);
    segmentation.operation = OperationId::from_bytes(&id(0x32)).unwrap();
    assert_eq!(
        policy.authorize_request(&hosted(), &request),
        Err(AuthDenied)
    );
}
