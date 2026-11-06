//! Boundary proof: original fixture pixels travel through actual service and native transform work.
use super::*;
use crate::*;
use df_types::{AccountId, OperationId};
use image::{ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;

struct Authority(ImageOwner);
impl ImageAuthorizer for Authority {
    type Principal = ();
    fn authorize_request(&self, _principal: &(), request: &ImageRequest) -> Result<(), AuthDenied> {
        if request.identity.owner == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_owner(
        &self,
        _principal: &(),
        owner: &ImageOwner,
        _action: ServiceAction,
    ) -> Result<(), AuthDenied> {
        if *owner == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
    fn authorize_stored(
        &self,
        _principal: &(),
        _action: ServiceAction,
        view: &OperationView,
        stored: &StoredImageAuthorization<'_>,
    ) -> Result<(), AuthDenied> {
        if view.job.owner == self.0 && *stored.owner == self.0 {
            Ok(())
        } else {
            Err(AuthDenied)
        }
    }
}
fn owner() -> ImageOwner {
    ImageOwner::Draft {
        account: AccountId::from_bytes(&[1; 16]).unwrap(),
    }
}
fn pixels() -> Vec<u8> {
    let image = RgbaImage::from_fn(2, 1, |column, _| {
        if column == 0 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 128])
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    bytes.into_inner()
}

#[tokio::test]
async fn real_service_worker_publishes_transformed_fixture_pixels() {
    let fixture = TransformFixture::new(owner(), Authority(owner()))
        .await
        .unwrap();
    let source = fixture
        .import_source(pixels(), Audience::PublicStatic)
        .await
        .unwrap();
    let service = fixture.service();
    let mut request = ImageRequest {
        identity: OperationIdentity {
            owner: owner(),
            operation: OperationId::from_bytes(&[2; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ReferencePrep,
            role: VariantRole::Reference,
        },
        operation: ImageOperation::Transform(TransformSpec {
            source: TransformSource::Image(source),
            steps: vec![ImageTransform::Flip(FlipAxis::Horizontal)],
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: 1,
        output: OutputProfile::local(
            RasterOutputSpec::new(
                Dimensions::new(2, 1).unwrap(),
                AlphaHandling::PreserveAlpha,
                Encoding::Png,
            )
            .unwrap(),
        ),
    };
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    let handle = OperationHandle {
        owner: request.identity.owner.clone(),
        operation: request.identity.operation,
    };
    match service.request(&(), &request).await {
        Ok(RequestOutcome::Recorded(_) | RequestOutcome::Replayed(_))
        | Err(ImageServiceError::Unknown { .. }) => {}
        other => panic!("request refused: {other:?}"),
    }
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match service.read(&(), &handle).await {
                Ok(view) if view.result.is_some() => break view.result.unwrap(),
                Ok(view) => assert!(!matches!(
                    view.execution,
                    ImageExecutionState::Unknown | ImageExecutionState::Failed { .. }
                )),
                Err(ImageServiceError::Unknown { .. }) => {}
                Err(error) => panic!("read failed: {error:?}"),
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let bytes = fixture.read_asset(result.asset).await.unwrap();
    let output = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(output.get_pixel(0, 0).0, [0, 0, 255, 128]);
    assert_eq!(output.get_pixel(1, 0).0, [255, 0, 0, 255]);
    assert_eq!(fixture.config().mode, GlobalMode::LocalOnly);
    assert!(fixture.config().backends.iter().all(|gate| !gate.enabled));
    drop(service);
    fixture.shutdown().await.unwrap();
}

#[tokio::test]
async fn source_pins_match_exact_imported_bytes_and_paid_demand_is_refused() {
    use sha2::{Digest, Sha256};
    let fixture = TransformFixture::new(owner(), Authority(owner()))
        .await
        .unwrap();
    let bytes = pixels();
    let source = fixture
        .import_source(bytes.clone(), Audience::PublicStatic)
        .await
        .unwrap();
    assert_eq!(
        *source.digest.as_bytes(),
        <[u8; 32]>::from(Sha256::digest(&bytes))
    );
    assert_eq!(fixture.read_asset(source.asset).await.unwrap(), bytes);
    assert!(
        fixture
            .import_source(vec![0, 1, 2], Audience::PublicStatic)
            .await
            .is_err()
    );
    let service = fixture.service();
    let mut paid = ImageRequest {
        identity: OperationIdentity {
            owner: owner(),
            operation: OperationId::from_bytes(&[3; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ScenePlate,
            role: VariantRole::Primary,
        },
        operation: ImageOperation::Generate(GenerateSpec {
            prompt: TextPrompt::new("fixture never dispatches this", 1).unwrap(),
            references: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: 1,
        output: OutputProfile::provider(
            Dimensions::new(2, 1).unwrap(),
            RasterOutputSpec::new(
                Dimensions::new(2, 1).unwrap(),
                AlphaHandling::Opaque,
                Encoding::Png,
            )
            .unwrap(),
        ),
    };
    paid.identity.fingerprint = service.canonical_fingerprint(&paid).unwrap();
    assert!(matches!(
        service.request(&(), &paid).await.unwrap(),
        RequestOutcome::Refused(_)
    ));
    drop(service);
    fixture.shutdown().await.unwrap();
}

async fn queued_store() -> (Store, crate::execution::StageView) {
    let ids = FixtureIds::new();
    let store = Store::new(owner(), ids.clone()).unwrap();
    let mut config = ImageConfig::defaults().unwrap();
    config.mode = GlobalMode::LocalOnly;
    let service =
        ImageService::new(store.clone(), config, FixtureClock, ids, Authority(owner())).unwrap();
    let mut request = ImageRequest {
        identity: OperationIdentity {
            owner: owner(),
            operation: OperationId::from_bytes(&[9; 16]).unwrap(),
            fingerprint: [0; 32],
        },
        purpose: PurposedUse {
            purpose: Purpose::ReferencePrep,
            role: VariantRole::Reference,
        },
        operation: ImageOperation::Transform(TransformSpec {
            source: TransformSource::Canvas(
                CanvasSpec::new(
                    1,
                    1,
                    Rgb {
                        red: 0,
                        green: 0,
                        blue: 0,
                    },
                )
                .unwrap(),
            ),
            steps: Vec::new(),
        }),
        execution: ExecutionMode::Live,
        freshness: Freshness::ReuseCompatible,
        urgency: Urgency::Present,
        audience: Audience::PublicStatic,
        expected_source_revision: 1,
        output: OutputProfile::local(
            RasterOutputSpec::new(
                Dimensions::new(1, 1).unwrap(),
                AlphaHandling::Opaque,
                Encoding::Png,
            )
            .unwrap(),
        ),
    };
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    assert!(matches!(
        service.request(&(), &request).await.unwrap(),
        RequestOutcome::Recorded(_)
    ));
    let stage = store.lock().unwrap().jobs[0].stage.clone();
    (store, stage)
}

#[tokio::test]
async fn child_lookup_uses_retained_parent_kind_job_and_owner() {
    use crate::execution::{DispatchId, ImageExecutionJournal, StageKind};
    let (store, stage) = queued_store().await;
    let parent = DispatchId::from_bytes(&[4; 16]).unwrap();
    assert!(
        store
            .read_child_stage(&owner(), &stage.job, &parent, StageKind::LocalTransform)
            .await
            .unwrap()
            .is_none()
    );
    store.lock().unwrap().jobs[0].basis.parent_dispatch = Some(parent);
    assert_eq!(
        store
            .read_child_stage(&owner(), &stage.job, &parent, StageKind::LocalTransform)
            .await
            .unwrap(),
        Some(stage.clone())
    );
    assert!(
        store
            .read_child_stage(&owner(), &stage.job, &parent, StageKind::Inspection)
            .await
            .unwrap()
            .is_none()
    );
    let other = ImageOwner::Draft {
        account: AccountId::from_bytes(&[8; 16]).unwrap(),
    };
    assert!(
        store
            .read_child_stage(&other, &stage.job, &parent, StageKind::LocalTransform)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn deferred_queue_preserves_due_hints_and_revokes_only_expired_unsent_claims() {
    use crate::execution::{
        ClaimCommand, ClaimToken, ExecutionError, ImageExecutionJournal, StageState,
    };
    let (store, stage) = queued_store().await;
    let now = super::state::NOW_MS;
    store
        .defer_queued(&crate::execution::DeferQueuedCommand {
            scope: crate::execution::StageScope {
                owner: owner(),
                job: stage.job,
                stage: stage.stage,
            },
            not_before_ms: now + 100,
            notice: None,
            now_ms: now,
        })
        .await
        .unwrap();
    assert!(
        store
            .scan_queue(store.scope.as_str(), 1, now)
            .await
            .unwrap()
            .is_empty()
    );
    let deferred = store
        .read_stage(&owner(), &stage.job, &stage.stage)
        .await
        .unwrap();
    store
        .defer_queued(&crate::execution::DeferQueuedCommand {
            scope: crate::execution::StageScope {
                owner: owner(),
                job: stage.job,
                stage: stage.stage,
            },
            not_before_ms: now + 50,
            notice: None,
            now_ms: now,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .read_stage(&owner(), &stage.job, &stage.stage)
            .await
            .unwrap(),
        deferred
    );
    let token = ClaimToken::from_bytes(&[5; 16]).unwrap();
    let early = ClaimCommand::new(owner(), stage.job, stage.stage, 1, token, 10, now + 99).unwrap();
    assert!(matches!(
        store.claim(&early).await,
        Err(ExecutionError::Deferred { not_before_ms }) if not_before_ms == now + 100
    ));
    let ready =
        ClaimCommand::new(owner(), stage.job, stage.stage, 1, token, 10, now + 100).unwrap();
    store.claim(&ready).await.unwrap();
    assert!(matches!(
        store
            .defer_queued(&crate::execution::DeferQueuedCommand {
                scope: crate::execution::StageScope {
                    owner: owner(),
                    job: stage.job,
                    stage: stage.stage
                },
                not_before_ms: now + 200,
                notice: None,
                now_ms: now + 101
            })
            .await,
        Err(ExecutionError::LostClaim)
    ));
    store
        .defer_queued(&crate::execution::DeferQueuedCommand {
            scope: crate::execution::StageScope {
                owner: owner(),
                job: stage.job,
                stage: stage.stage,
            },
            not_before_ms: now + 200,
            notice: None,
            now_ms: now + 111,
        })
        .await
        .unwrap();
    let restored = store
        .read_stage(&owner(), &stage.job, &stage.stage)
        .await
        .unwrap();
    assert_eq!(restored.state, StageState::Planned);
    assert_eq!(restored.lease_until_ms, 0);
    assert!(store.lock().unwrap().jobs[0].claim.is_none());
    assert!(
        store
            .scan_queue(store.scope.as_str(), 1, now + 199)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .scan_queue(store.scope.as_str(), 1, now + 200)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn local_transform_has_no_inspection_receipt_and_scopes_reads() {
    use crate::execution::{ExecutionError, ExecutionStageId, ImageExecutionJournal};
    let (store, stage) = queued_store().await;
    assert!(
        store
            .read_local_inspection(&owner(), &stage.job, &stage.stage)
            .await
            .unwrap()
            .is_none()
    );
    let other = ImageOwner::Draft {
        account: AccountId::from_bytes(&[8; 16]).unwrap(),
    };
    assert!(matches!(
        store
            .read_local_inspection(&other, &stage.job, &stage.stage)
            .await,
        Err(ExecutionError::NotFound)
    ));
    let missing = ExecutionStageId::from_bytes(&[9; 16]).unwrap();
    assert!(matches!(
        store
            .read_local_inspection(&owner(), &stage.job, &missing)
            .await,
        Err(ExecutionError::NotFound)
    ));
}

#[tokio::test]
async fn unsent_scan_reads_only_actual_expired_prepared_dispatches_without_mutation() {
    use crate::execution::{
        ClaimCommand, ClaimToken, DispatchId, DispatchMode, ImageExecutionJournal, MarkSentCommand,
        PrepareCommand, StageClaim, StageScope,
    };
    let (store, stage) = queued_store().await;
    let now = super::state::NOW_MS;
    let token = ClaimToken::from_bytes(&[5; 16]).unwrap();
    let claimed = store
        .claim(&ClaimCommand::new(owner(), stage.job, stage.stage, 1, token, 10, now).unwrap())
        .await
        .unwrap();
    let claim = StageClaim {
        scope: StageScope {
            owner: owner(),
            job: stage.job,
            stage: stage.stage,
        },
        token,
        generation: claimed.claim_generation,
    };
    let dispatch = DispatchId::from_bytes(&[6; 16]).unwrap();
    store
        .prepare(&PrepareCommand::new(claim.clone(), dispatch, DispatchMode::Local, now).unwrap())
        .await
        .unwrap();
    let before = store
        .read_stage(&owner(), &stage.job, &stage.stage)
        .await
        .unwrap();
    assert!(
        store
            .scan_unsent_prepared(store.scope.as_str(), None, 1, now + 9)
            .await
            .unwrap()
            .is_empty()
    );
    let found = store
        .scan_unsent_prepared(store.scope.as_str(), None, 1, now + 10)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scope.dispatch, dispatch);
    assert_eq!(found[0].claim, claim);
    assert_eq!(found[0].original_lease_until_ms, now + 10);
    assert!(!found[0].terminal_condition);
    assert!(!found[0].sealed);
    assert_eq!(
        store
            .read_stage(&owner(), &stage.job, &stage.stage)
            .await
            .unwrap(),
        before
    );
    assert!(
        store
            .scan_unsent_prepared(store.scope.as_str(), Some(dispatch), 1, now + 10)
            .await
            .unwrap()
            .is_empty()
    );
    store
        .mark_sent(
            &MarkSentCommand::new(
                owner(),
                stage.job,
                stage.stage,
                dispatch,
                token,
                claimed.claim_generation,
                now + 1,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        store
            .scan_unsent_prepared(store.scope.as_str(), None, 1, now + 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn terminal_scan_uses_real_unclaimed_claimed_and_deferred_proofs_without_mutation() {
    use crate::execution::{
        ClaimCommand, ClaimToken, ImageExecutionJournal, StageClaim, StageScope, TerminalCursor,
        UnsentTerminal,
    };
    for phase in 0..3 {
        let (store, stage) = queued_store().await;
        let now = super::state::NOW_MS;
        assert!(
            store
                .scan_unsent_terminal(store.scope.as_str(), None, 1, now)
                .await
                .unwrap()
                .is_empty()
        );
        let token = ClaimToken::from_bytes(&[5; 16]).unwrap();
        let mut actual_claim = None;
        if phase > 0 {
            let claimed = store
                .claim(
                    &ClaimCommand::new(owner(), stage.job, stage.stage, 1, token, 10, now).unwrap(),
                )
                .await
                .unwrap();
            actual_claim = Some(StageClaim {
                scope: StageScope {
                    owner: owner(),
                    job: stage.job,
                    stage: stage.stage,
                },
                token,
                generation: claimed.claim_generation,
            });
        }
        if phase == 2 {
            store
                .defer_queued(&crate::execution::DeferQueuedCommand {
                    scope: crate::execution::StageScope {
                        owner: owner(),
                        job: stage.job,
                        stage: stage.stage,
                    },
                    not_before_ms: now + 100,
                    notice: None,
                    now_ms: now + 10,
                })
                .await
                .unwrap();
        }
        let deadline = store.lock().unwrap().jobs[0].job.absolute_deadline_ms;
        let before = store
            .read_stage(&owner(), &stage.job, &stage.stage)
            .await
            .unwrap();
        let found = store
            .scan_unsent_terminal(store.scope.as_str(), None, 1, deadline)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        match (&found[0], phase) {
            (UnsentTerminal::Unclaimed(proof), 0) => {
                assert_eq!(proof.scope.stage, stage.stage);
                assert_eq!(before.claim_generation, 0);
            }
            (UnsentTerminal::Claimed(proof), 1) => {
                assert_eq!(Some(&proof.claim), actual_claim.as_ref())
            }
            (UnsentTerminal::Deferred(proof), 2) => {
                assert_eq!(
                    proof.claim_generation,
                    actual_claim.as_ref().unwrap().generation
                );
                assert_eq!(proof.not_before_ms, now + 100);
                assert!(store.lock().unwrap().jobs[0].claim.is_none());
            }
            _ => panic!("terminal proof does not match actual stored transition"),
        }
        assert_eq!(
            store
                .read_stage(&owner(), &stage.job, &stage.stage)
                .await
                .unwrap(),
            before
        );
        assert!(
            store
                .scan_unsent_terminal(
                    store.scope.as_str(),
                    Some(TerminalCursor {
                        job: stage.job,
                        stage: stage.stage
                    }),
                    1,
                    deadline
                )
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .scan_unsent_terminal(store.scope.as_str(), None, 0, deadline)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn unknown_semantic_cause_is_adopted_once_without_rewriting_unknown_settlement() {
    use crate::execution::{
        ClaimCommand, ClaimToken, CompleteCommand, CompletionOutcome, DispatchId, DispatchMode,
        DispatchSettlement, DispatchState, ExecutionError, FailureCause, ImageExecutionJournal,
        MarkSentCommand, PrepareCommand, StageClaim, StageScope,
    };
    let (store, stage) = queued_store().await;
    let now = super::state::NOW_MS;
    let token = ClaimToken::from_bytes(&[5; 16]).unwrap();
    let claimed = store
        .claim(&ClaimCommand::new(owner(), stage.job, stage.stage, 1, token, 100, now).unwrap())
        .await
        .unwrap();
    let claim = StageClaim {
        scope: StageScope {
            owner: owner(),
            job: stage.job,
            stage: stage.stage,
        },
        token,
        generation: claimed.claim_generation,
    };
    let dispatch = DispatchId::from_bytes(&[6; 16]).unwrap();
    store
        .prepare(&PrepareCommand::new(claim.clone(), dispatch, DispatchMode::Local, now).unwrap())
        .await
        .unwrap();
    store
        .mark_sent(
            &MarkSentCommand::new(
                owner(),
                stage.job,
                stage.stage,
                dispatch,
                token,
                claimed.claim_generation,
                now + 1,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let actual_scope = crate::execution::ExecutionScope {
        owner: owner(),
        job: stage.job,
        stage: stage.stage,
        dispatch,
    };
    assert!(
        store
            .read_decline_reference(&actual_scope)
            .await
            .unwrap()
            .is_none()
    );
    let mut foreign_scope = actual_scope.clone();
    foreign_scope.owner = ImageOwner::Draft {
        account: AccountId::from_bytes(&[8; 16]).unwrap(),
    };
    assert_eq!(
        store.read_decline_reference(&foreign_scope).await,
        Err(ExecutionError::NotFound)
    );
    assert_eq!(
        store
            .requeue_declined(&crate::execution::DeclineRetryCommand {
                claim: claim.clone(),
                dispatch,
                now_ms: now + 2
            })
            .await,
        Err(ExecutionError::BasisConflict)
    );
    assert_eq!(
        store
            .complete(
                &CompleteCommand::new(
                    claim.clone(),
                    dispatch,
                    CompletionOutcome::Declined {
                        observation_version: 1,
                        usage_revision: 1
                    },
                    now + 2
                )
                .unwrap()
            )
            .await,
        Err(ExecutionError::ObservationMissing)
    );
    let unknown = store
        .complete(
            &CompleteCommand::new(claim.clone(), dispatch, CompletionOutcome::Unknown, now + 2)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.state, DispatchState::Unknown);
    assert_eq!(unknown.settlement, DispatchSettlement::UnknownOutstanding);
    assert_eq!(unknown.cause, None);
    let outcome = CompletionOutcome::UnknownWithCause {
        cause: FailureCause::DeadlineExceeded,
    };
    let observed = store
        .complete(&CompleteCommand::new(claim.clone(), dispatch, outcome.clone(), now + 3).unwrap())
        .await
        .unwrap();
    assert_eq!(observed.state, DispatchState::Unknown);
    assert_eq!(observed.settlement, DispatchSettlement::UnknownOutstanding);
    assert_eq!(observed.cause, Some(FailureCause::DeadlineExceeded));
    let revision = store.lock().unwrap().jobs[0].job.revision;
    assert_eq!(
        store
            .complete(&CompleteCommand::new(claim.clone(), dispatch, outcome, now + 4).unwrap())
            .await
            .unwrap(),
        observed
    );
    assert_eq!(
        store
            .complete(
                &CompleteCommand::new(claim.clone(), dispatch, CompletionOutcome::Unknown, now + 5)
                    .unwrap()
            )
            .await
            .unwrap(),
        observed
    );
    assert_eq!(store.lock().unwrap().jobs[0].job.revision, revision);
    assert_eq!(
        store
            .complete(
                &CompleteCommand::new(
                    claim.clone(),
                    dispatch,
                    CompletionOutcome::UnknownWithCause {
                        cause: FailureCause::Provider
                    },
                    now + 6
                )
                .unwrap()
            )
            .await,
        Err(ExecutionError::OperationConflict)
    );
    let progress = store
        .read_progress(&owner(), stage.job, now + 7)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(progress.state, ImageExecutionState::Unknown);
    assert_eq!(progress.failure, Some(FailureCause::DeadlineExceeded));
    let known = store
        .complete(
            &CompleteCommand::new(
                claim.clone(),
                dispatch,
                CompletionOutcome::Failed {
                    cause: FailureCause::InvalidOutput,
                },
                now + 8,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(known.state, DispatchState::Failed);
    assert_eq!(known.cause, Some(FailureCause::InvalidOutput));
    assert_eq!(
        store
            .complete(
                &CompleteCommand::new(
                    claim,
                    dispatch,
                    CompletionOutcome::UnknownWithCause {
                        cause: FailureCause::DeadlineExceeded
                    },
                    now + 9
                )
                .unwrap()
            )
            .await
            .unwrap(),
        known
    );
}

#[tokio::test]
async fn queued_notice_replay_is_read_only_reason_changes_bump_and_actual_claim_clears_it() {
    use crate::execution::{
        ClaimCommand, ClaimToken, DeferQueuedCommand, ImageExecutionJournal, QueueDeferralNotice,
        QuotaDeferralReason, StageScope,
    };
    let (store, stage) = queued_store().await;
    let now = super::state::NOW_MS;
    let scope = StageScope {
        owner: owner(),
        job: stage.job,
        stage: stage.stage,
    };
    assert_eq!(
        store
            .read_progress(&owner(), stage.job, now)
            .await
            .unwrap()
            .unwrap()
            .operational_notice,
        None
    );
    let command = DeferQueuedCommand::new(
        scope.clone(),
        now + 100,
        Some(QuotaDeferralReason::Cooldown),
        now,
    )
    .unwrap();
    store.defer_queued(&command).await.unwrap();
    let progress = store
        .read_progress(&owner(), stage.job, now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        progress.operational_notice,
        Some(QueueDeferralNotice {
            reason: QuotaDeferralReason::Cooldown,
            retry_at_ms: now + 100
        })
    );
    assert_eq!(
        progress.failure, None,
        "an operational notice is not supplier failure evidence"
    );
    store
        .defer_queued(&DeferQueuedCommand {
            now_ms: now + 1,
            ..command.clone()
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .read_progress(&owner(), stage.job, now + 1)
            .await
            .unwrap()
            .unwrap(),
        progress
    );
    store
        .defer_queued(&DeferQueuedCommand {
            notice: Some(QuotaDeferralReason::Probing),
            now_ms: now + 2,
            ..command.clone()
        })
        .await
        .unwrap();
    let changed = store
        .read_progress(&owner(), stage.job, now + 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.revision, progress.revision + 1);
    assert_eq!(
        changed.operational_notice,
        Some(QueueDeferralNotice {
            reason: QuotaDeferralReason::Probing,
            retry_at_ms: now + 100
        })
    );
    store
        .defer_queued(&DeferQueuedCommand {
            not_before_ms: now + 50,
            notice: None,
            now_ms: now + 3,
            ..command
        })
        .await
        .unwrap();
    assert_eq!(store.lock().unwrap().jobs[0].not_before_ms, now + 100);
    assert_eq!(
        store
            .read_progress(&owner(), stage.job, now + 3)
            .await
            .unwrap()
            .unwrap()
            .operational_notice,
        None
    );
    store
        .defer_queued(
            &DeferQueuedCommand::new(
                scope,
                now + 100,
                Some(QuotaDeferralReason::Cooldown),
                now + 4,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let token = ClaimToken::from_bytes(&[5; 16]).unwrap();
    store
        .claim(
            &ClaimCommand::new(owner(), stage.job, stage.stage, 1, token, 10, now + 100).unwrap(),
        )
        .await
        .unwrap();
    let claimed = store
        .read_progress(&owner(), stage.job, now + 100)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.operational_notice, None);
    assert_eq!(claimed.failure, None);
}
