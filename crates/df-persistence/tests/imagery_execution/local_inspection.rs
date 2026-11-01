//! Real canonical service acceptance, decoded candidate and no-dispatch BestEffort evidence.
use super::*;
use df_assets::{Clock, IdSource};
use df_imagery::execution::*;
use df_imagery::*;
use df_persistence::imagery_execution::{
    read_local_inspection_in, read_stage_basis_in, record_candidate_observation_in,
    record_local_inspection_in, scan_queue_in,
};
use df_persistence::imagery_runtime::{PostgresRuntimeJournal, record_sources_in};
use std::sync::Arc;
#[derive(Clone)]
struct Ids(Arc<AtomicU64>);
impl IdSource for Ids {
    fn next_id_bytes(&self) -> [u8; 16] {
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let mut b = [0; 16];
        b[8..].copy_from_slice(&n.to_be_bytes());
        b
    }
}
#[derive(Clone)]
struct FixedClock;
impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        NOW_MS
    }
}
struct Authorizer(ImageOwner);
impl ImageAuthorizer for Authorizer {
    type Principal = ();
    fn authorize_request(&self, _: &(), r: &ImageRequest) -> Result<(), AuthDenied> {
        if r.identity.owner == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_owner(&self, _: &(), o: &ImageOwner, _: ServiceAction) -> Result<(), AuthDenied> {
        if *o == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_stored(
        &self,
        _: &(),
        _: ServiceAction,
        v: &OperationView,
        s: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        if v.job.owner == self.0 && *s.owner == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
}
fn route() -> AnalysisRoute {
    AnalysisRoute {
        account: AccountHandle::new("inspection-account").unwrap(),
        model: ModelName::new("jev-latest").unwrap(),
        endpoint: ApiEndpoint::new("http://127.0.0.1:55499/v1").unwrap(),
        maximum_output_tokens: 3000,
    }
}
fn request(requirement: InspectionRequirement) -> ImageRequest {
    let pipeline = ImagePipeline {
        revision: 1,
        moderation: None,
        rating: ImageContentRating::Restrained,
        creator: None,
        inspection: Some(ImageInspection {
            requirement,
            route: route(),
            contract_revision: 1,
            kind: InspectionKind::ScenePlate,
            expected_heroes: None,
            party_reference: None,
            hand_custody: PrivateImageText::new("private custody"),
            staging: vec![],
            maximum_corrections: 1,
            allow_visible_text: false,
        }),
    };
    let dim = Dimensions::new(1, 1).unwrap();
    let account = AccountId::from_bytes(&id(0x61)).unwrap();
    ImageRequest {
        identity: OperationIdentity {
            owner: ImageOwner::Draft { account },
            operation: OperationId::from_bytes(&id(0x62)).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("private scene prompt", 17)
                .unwrap()
                .with_pipeline(pipeline)
                .unwrap(),
            references: vec![],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Visible,
        audience: Audience::OwnerPrivate(account),
        expected_source_revision: 1,
        output: OutputProfile::Provider {
            generated: dim,
            delivered: RasterOutputSpec::new(dim, AlphaHandling::Opaque, Encoding::Png).unwrap(),
        },
    }
}
#[test]
fn best_effort_preflight_checkpoint_replays_after_restart_without_dispatch_usage_or_money() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        let ids = Ids(Arc::new(AtomicU64::new(1000)));
        let journal = PostgresRuntimeJournal::open_existing(
            scratch.config(),
            PostgresTransport::PlaintextLoopbackOnly,
            2,
            Arc::new(ids.clone()),
            CapacityScope::new("live/generation".into()).unwrap(),
            GATE_REVISION,
            true,
        )
        .unwrap();
        let mut config = ImageConfig::defaults().unwrap();
        config.accounting.mode = AccountingMode::RecordOnly;
        config.analysis.inspection = Some(route());
        let service = ImageService::new(
            journal.clone(),
            config,
            FixedClock,
            ids,
            Authorizer(draft_owner()),
        )
        .unwrap()
        .with_config_revision(GATE_REVISION)
        .unwrap();
        let mut request = request(InspectionRequirement::BestEffort);
        request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
        assert!(matches!(
            service.request(&(), &request).await.unwrap(),
            RequestOutcome::Recorded(_)
        ));
        let retained = journal
            .lookup_operation(&request.identity.owner, request.identity.operation)
            .await
            .unwrap()
            .unwrap()
            .job;
        let tx = client.transaction().await.unwrap();
        let queued = scan_queue_in(&tx, "live/generation", 10, at(1))
            .await
            .unwrap();
        assert_eq!(queued.len(), 1);
        let parent = queued[0].stage;
        let claimed = claim_stage_in(
            &tx,
            &ClaimCommand::new(
                retained.owner.clone(),
                retained.job,
                parent,
                GATE_REVISION,
                ClaimToken::from_bytes(&id(0x71)).unwrap(),
                LEASE_MS,
                at(1),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let claim = StageClaim {
            scope: StageScope {
                owner: retained.owner.clone(),
                job: retained.job,
                stage: parent,
            },
            token: ClaimToken::from_bytes(&id(0x71)).unwrap(),
            generation: claimed.claim_generation,
        };
        let scope = ExecutionScope {
            owner: retained.owner.clone(),
            job: retained.job,
            stage: parent,
            dispatch: dispatch_id(0x81),
        };
        prepare_dispatch_in(
            &tx,
            &PrepareCommand::new(claim.clone(), scope.dispatch, DispatchMode::Local, at(2))
                .unwrap(),
        )
        .await
        .unwrap();
        record_sources_in(&tx, &scope, &[]).await.unwrap();
        mark_sent_in(
            &tx,
            &MarkSentCommand::new(
                scope.owner.clone(),
                scope.job,
                scope.stage,
                scope.dispatch,
                claim.token,
                claim.generation,
                at(3),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let basis = read_stage_basis_in(&tx, &scope.owner, &scope.job, &scope.stage)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let bytes = candidate_png(false);
        let observation = CandidateObservation::inspect(
            scope.clone(),
            basis,
            1,
            ObservationReceipt {
                usage_revision: 1,
                observed_at_ms: at(4),
            },
            &bytes,
            df_assets::ImageProofLimits::new(1024, 1, 16).unwrap(),
            df_assets::ImageIntent::new("image/png", 1, 1, df_assets::ImageAlpha::Opaque).unwrap(),
        )
        .unwrap();
        record_raw_usage(
            &scratch.config(),
            &scope,
            1,
            Some(bytes),
            df_imagery::cost::UsageEvidence::Reported,
            true,
        )
        .await;
        let tx = client.transaction().await.unwrap();
        let candidate = record_candidate_observation_in(&tx, &observation)
            .await
            .unwrap();
        complete_dispatch_in(
            &tx,
            &CompleteCommand::new(
                claim,
                scope.dispatch,
                CompletionOutcome::Candidate {
                    observation_version: 1,
                    usage_revision: 1,
                },
                at(5),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let planned = claimed.planned_publication.expect("actual media plan");
        let child = EnqueueCommand::new(
            StageScope {
                owner: scope.owner.clone(),
                job: scope.job,
                stage: stage_id(0x22),
            },
            StageBasis {
                kind: StageKind::Inspection,
                input_revision: 1,
                input_digest: candidate.descriptor.sha256,
                config_revision: candidate.basis.config_revision,
                config_digest: candidate.basis.config_digest,
                parent_dispatch: Some(scope.dispatch),
            },
            planned,
            CapacityScope::new("live/generation".into()).unwrap(),
            true,
            at(6),
        )
        .unwrap();
        enqueue_stage_in(&tx, &child).await.unwrap();
        let child_view = claim_stage_in(
            &tx,
            &ClaimCommand::new(
                scope.owner.clone(),
                scope.job,
                stage_id(0x22),
                GATE_REVISION,
                ClaimToken::from_bytes(&id(0x72)).unwrap(),
                LEASE_MS,
                at(7),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let child_claim = StageClaim {
            scope: StageScope {
                owner: scope.owner.clone(),
                job: scope.job,
                stage: stage_id(0x22),
            },
            token: ClaimToken::from_bytes(&id(0x72)).unwrap(),
            generation: child_view.claim_generation,
        };
        let digest =
            df_imagery::runtime::source_receipts_digest(&[]).expect("canonical empty sources");
        let command = LocalInspectionCommand::new(
            &retained,
            child_claim.clone(),
            scope.dispatch,
            digest,
            LocalInspectionReason::Preparation,
            at(8),
        )
        .unwrap();
        let tx = client.transaction().await.unwrap();
        let wrong_sources = LocalInspectionCommand::new(
            &retained,
            child_claim.clone(),
            scope.dispatch,
            [0xEE; 32],
            LocalInspectionReason::Preparation,
            at(8),
        )
        .unwrap();
        assert_eq!(
            record_local_inspection_in(&tx, &wrong_sources).await,
            Err(ExecutionError::BasisConflict)
        );
        let expired = LocalInspectionCommand::new(
            &retained,
            child_claim.clone(),
            scope.dispatch,
            digest,
            LocalInspectionReason::Preparation,
            at(11_000),
        )
        .unwrap();
        assert_eq!(
            record_local_inspection_in(&tx, &expired).await,
            Err(ExecutionError::LostClaim)
        );
        let checkpoint = record_local_inspection_in(&tx, &command)
            .await
            .expect("actual retained BestEffort preflight");
        assert_eq!(checkpoint.candidate_scope, scope);
        assert_eq!(checkpoint.reason, LocalInspectionReason::Preparation);
        assert_eq!(
            read_stage_in(&tx, &scope.owner, &scope.job, &child_claim.scope.stage)
                .await
                .unwrap()
                .state,
            StageState::Released
        );
        tx.commit().await.unwrap();
        drop(client);
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            record_local_inspection_in(&tx, &command).await.unwrap(),
            checkpoint
        );
        assert_eq!(
            read_local_inspection_in(&tx, &scope.owner, &scope.job, &child_claim.scope.stage)
                .await
                .unwrap(),
            Some(checkpoint)
        );
        let released_revision =
            read_stage_in(&tx, &scope.owner, &scope.job, &child_claim.scope.stage)
                .await
                .unwrap()
                .job_revision;
        assert_eq!(
            prepare_dispatch_in(
                &tx,
                &PrepareCommand::new(child_claim, dispatch_id(0x82), DispatchMode::Local, at(9))
                    .unwrap()
            )
            .await,
            Err(ExecutionError::Stale {
                current_revision: released_revision
            })
        );
        tx.rollback().await.unwrap();
        assert_eq!(
            client
                .query_one(
                    "SELECT count(*) FROM imagery.image_dispatch_attempt WHERE stage_id=$1",
                    &[&id(0x22).as_slice()]
                )
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(
            client
                .query_one(
                    "SELECT count(*) FROM imagery.image_dispatch_usage WHERE stage_id=$1",
                    &[&id(0x22).as_slice()]
                )
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(
            client
                .query_one(
                    "SELECT count(*) FROM imagery.image_finance_admission_binding",
                    &[]
                )
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        scratch.finish().await;
    });
}
