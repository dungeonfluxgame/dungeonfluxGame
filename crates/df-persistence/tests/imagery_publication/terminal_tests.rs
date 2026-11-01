//! Guarded output closure proves quota release separately from actual retained supplier liability.
use super::*;
use df_imagery::execution::*;
use df_imagery::publication::{NoDeliveryCause, NoDeliveryRequest, NoDeliveryStage};
use df_imagery::runtime::{
    ImageDispatchAdmission, ImagePublisher, qualify_no_delivery, qualify_required_preflight,
};

#[tokio::test]
async fn native_supplier_failure_requires_exact_no_output_seal_and_unknown_never_closes() {
    for outcome in [
        terminal_fixture::SupplierFailure::NoOutput,
        terminal_fixture::SupplierFailure::UnknownBilling,
        terminal_fixture::SupplierFailure::UnknownOutput,
    ] {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config).await;
        let fixture = Fixture::create(&client, false, true).await;
        let failure = terminal_fixture::fail_supplier(&mut client, &fixture, outcome).await;
        let publisher = selection_fixture::publisher(&scratch, &fixture).await;
        let proof = df_imagery::runtime::qualify_no_output(
            &failure.job,
            &failure.dispatch,
            &failure.basis,
            &failure.usage,
        );
        if outcome == terminal_fixture::SupplierFailure::NoOutput {
            assert!(
                publisher
                    .outcome(
                        &failure.job,
                        failure.dispatch.dispatch,
                        DispatchMode::Local,
                        Some(false)
                    )
                    .await
                    .is_err()
            );
            assert_eq!(
                client
                    .query_one("SELECT settlement FROM imagery.image_dispatch_attempt", &[])
                    .await
                    .unwrap()
                    .get::<_, String>(0),
                "held"
            );
            let proof = proof.unwrap();
            let request = NoDeliveryRequest {
                terminal: &failure.command,
                cause: NoDeliveryCause::NoOutput(&proof),
            };
            let receipt = publisher.no_delivery(&request).await.unwrap();
            assert_eq!(
                receipt.reason,
                df_imagery::runtime::NoDeliveryReason::SupplierNoOutput
            );
            publisher
                .outcome(
                    &failure.job,
                    failure.dispatch.dispatch,
                    DispatchMode::Local,
                    Some(false),
                )
                .await
                .unwrap();
            let revision = client
                .query_one("SELECT revision FROM imagery.image_job", &[])
                .await
                .unwrap()
                .get::<_, i64>(0);
            publisher
                .outcome(
                    &failure.job,
                    failure.dispatch.dispatch,
                    DispatchMode::Local,
                    Some(false),
                )
                .await
                .unwrap();
            assert_eq!(
                client
                    .query_one("SELECT revision FROM imagery.image_job", &[])
                    .await
                    .unwrap()
                    .get::<_, i64>(0),
                revision
            );
            assert_eq!(publisher.no_delivery(&request).await.unwrap(), receipt);
            assert_eq!(
                client
                    .query_one("SELECT settlement FROM imagery.image_dispatch_attempt", &[])
                    .await
                    .unwrap()
                    .get::<_, String>(0),
                "unusable_settled"
            );
            assert_eq!(
                client
                    .query_one("SELECT reserved_bytes FROM media.quota_usage", &[])
                    .await
                    .unwrap()
                    .get::<_, i64>(0),
                0
            );
        } else {
            assert!(proof.is_err());
            publisher
                .outcome(
                    &failure.job,
                    failure.dispatch.dispatch,
                    DispatchMode::Local,
                    None,
                )
                .await
                .unwrap();
            publisher
                .outcome(
                    &failure.job,
                    failure.dispatch.dispatch,
                    DispatchMode::Local,
                    Some(false),
                )
                .await
                .unwrap();
            assert_eq!(
                client
                    .query_one("SELECT settlement FROM imagery.image_dispatch_attempt", &[])
                    .await
                    .unwrap()
                    .get::<_, String>(0),
                // Any unknown supplier outcome keeps its liability; only no-output may hold.
                "unknown_outstanding"
            );
            assert_eq!(
                client
                    .query_one("SELECT count(*) FROM imagery.image_no_delivery", &[])
                    .await
                    .unwrap()
                    .get::<_, i64>(0),
                0
            );
            assert_eq!(
                client
                    .query_one("SELECT reserved_bytes FROM media.quota_usage", &[])
                    .await
                    .unwrap()
                    .get::<_, i64>(0),
                1000
            );
        }
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        drop(publisher);
        scratch.finish().await;
    }
}

#[tokio::test]
async fn local_unknown_transport_uses_actual_raw_revision_without_inventing_completion() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create(&client, false, true).await;
    let command = fixture.command();
    client.batch_execute("DELETE FROM imagery.image_dispatch_checkpoint; UPDATE imagery.image_dispatch_attempt SET dispatch_state='unknown',usage_revision=0").await.unwrap();
    client.execute("INSERT INTO imagery.image_dispatch_usage(dispatch_id,owner_scope,job_id,stage_id,revision,provider,model,cause,detail,maxima,evidence,units,usable,observed_at_ms) VALUES($1,$2,$3,$4,1,'fixture','fixture','image','generation','[{\"category\":\"output\",\"quantity\":1}]','unknown','[]',false,$5)",&[&command.dispatch.as_bytes().as_slice(),&owner_scope_bytes(&fixture.owner),&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice(),&(NOW+1)]).await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
        &owner_scope_bytes(&fixture.owner),
        &command.job.as_bytes().as_slice(),
        &command.stage.as_bytes().as_slice(),
        &command.dispatch.as_bytes().as_slice(),
        &(NOW + 10),
    ];
    assert!(
        tx.query_one(
            "SELECT imagery.mirror_local_transport_outcome($1,$2,$3,$4,$5)",
            params
        )
        .await
        .unwrap()
        .get::<_, bool>(0)
    );
    assert!(
        !tx.query_one(
            "SELECT imagery.mirror_local_transport_outcome($1,$2,$3,$4,$5)",
            params
        )
        .await
        .unwrap()
        .get::<_, bool>(0)
    );
    tx.commit().await.unwrap();
    assert_eq!(client.query_one("SELECT dispatch_state,usage_revision,settlement FROM imagery.image_dispatch_attempt",&[]).await.unwrap().get::<_,String>(0),"unknown");
    assert_eq!(
        client
            .query_one(
                "SELECT usage_revision FROM imagery.image_dispatch_attempt",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i32>(0),
        0
    );
    assert_eq!(
        client
            .query_one("SELECT settlement FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "unknown_outstanding"
    );
    client.execute("INSERT INTO imagery.image_dispatch_usage(dispatch_id,owner_scope,job_id,stage_id,revision,provider,model,cause,detail,maxima,evidence,units,usable,observed_at_ms) VALUES($1,$2,$3,$4,2,'fixture','fixture','image','generation','[{\"category\":\"output\",\"quantity\":1}]','reported','[{\"category\":\"output\",\"quantity\":1}]',false,$5)",&[&command.dispatch.as_bytes().as_slice(),&owner_scope_bytes(&fixture.owner),&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice(),&(NOW+2)]).await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    assert!(
        !tx.query_one(
            "SELECT imagery.mirror_local_transport_outcome($1,$2,$3,$4,$5)",
            params
        )
        .await
        .unwrap()
        .get::<_, bool>(0)
    );
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT settlement FROM imagery.image_dispatch_attempt", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "unknown_outstanding"
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    scratch.finish().await;
}

#[tokio::test]
async fn rejected_or_required_unjudged_seals_once_releases_quota_and_preserves_liability() {
    for outcome in [
        selection_fixture::Inspection::Rejected,
        selection_fixture::Inspection::Unjudged,
    ] {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config).await;
        let fixture = Fixture::create_with_pipeline(
            &client,
            false,
            true,
            Some(intent_fixture::inspection_pipeline(
                df_imagery::InspectionRequirement::Required,
            )),
        )
        .await;
        let case = terminal_fixture::inspection(&mut client, &fixture, outcome).await;
        let proof =
            qualify_no_delivery(&case.job, &case.evidence, case.selected.candidate.as_ref())
                .unwrap();
        let publisher = selection_fixture::publisher(&scratch, &fixture).await;
        let mut stale = case.command.clone();
        stale.expected_job_revision += 1;
        assert!(
            publisher
                .no_delivery(&NoDeliveryRequest {
                    terminal: &stale,
                    cause: NoDeliveryCause::Semantic(&proof)
                })
                .await
                .is_err()
        );
        assert_eq!(
            client
                .query_one("SELECT reserved_bytes FROM media.quota_usage", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            1000
        );
        let request = NoDeliveryRequest {
            terminal: &case.command,
            cause: NoDeliveryCause::Semantic(&proof),
        };
        let receipt = publisher.no_delivery(&request).await.unwrap();
        assert_eq!(receipt.expected_job_revision, case.job.revision);
        assert_eq!(receipt.job_revision, case.job.revision + 1);
        assert_eq!(publisher.no_delivery(&request).await.unwrap(), receipt);
        assert_eq!(
            publisher
                .read_no_delivery(&case.job.owner, &case.job.job)
                .await
                .unwrap(),
            Some(receipt.clone())
        );
        assert_eq!(
            client
                .query_one("SELECT reserved_bytes FROM media.quota_usage", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(
            client
                .query_one("SELECT state FROM media.asset", &[])
                .await
                .unwrap()
                .get::<_, String>(0),
            "rejected"
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM media.settlement_outbox", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_job_result", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        let candidate = case.selected.candidate.as_ref().unwrap();
        assert_eq!(
            publisher
                .image_economic_use(&candidate.scope)
                .await
                .unwrap(),
            Some(df_imagery::runtime::ImageEconomicDisposition {
                usage_revision: 1,
                usable: false
            })
        );
        assert_eq!(
            client
                .query_one(
                    "SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
                    &[&candidate.scope.dispatch.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, String>(0),
            "unknown_outstanding"
        );
        if matches!(outcome, selection_fixture::Inspection::Unjudged) {
            assert_eq!(client.query_one("SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1", &[&case.reference.scope.dispatch.as_bytes().as_slice()]).await.unwrap().get::<_,String>(0),"unknown_outstanding");
        }
        assert!(
            publisher
                .outcome(
                    &case.job,
                    candidate.scope.dispatch,
                    DispatchMode::Local,
                    Some(true)
                )
                .await
                .is_err()
        );
        publisher
            .outcome(
                &case.job,
                candidate.scope.dispatch,
                DispatchMode::Local,
                Some(false),
            )
            .await
            .unwrap();
        let settled_revision = client
            .query_one("SELECT revision FROM imagery.image_job", &[])
            .await
            .unwrap()
            .get::<_, i64>(0);
        publisher
            .outcome(
                &case.job,
                candidate.scope.dispatch,
                DispatchMode::Local,
                Some(false),
            )
            .await
            .unwrap();
        assert_eq!(
            client
                .query_one("SELECT revision FROM imagery.image_job", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            settled_revision
        );
        assert_eq!(
            client
                .query_one(
                    "SELECT settlement FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
                    &[&candidate.scope.dispatch.as_bytes().as_slice()]
                )
                .await
                .unwrap()
                .get::<_, String>(0),
            "unusable_settled"
        );
        let tx = client.transaction().await.unwrap();
        let lease = df_persistence::imagery_execution::claim_recovery_in(
            &tx,
            &ClaimRecoveryCommand {
                owner: case.job.owner.clone(),
                job: case.job.job,
                stage: case.reference.scope.stage,
                dispatch: case.reference.scope.dispatch,
                claim: ExecutionClaim::from_bytes(&[63; 16]).unwrap(),
                lease_ms: 100,
                now_ms: NOW + 1001,
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let tx = client.transaction().await.unwrap();
        let finish = df_persistence::imagery_execution::finish_recovery_in(
            &tx,
            &FinishRecoveryCommand {
                owner: case.job.owner.clone(),
                job: case.job.job,
                stage: case.reference.scope.stage,
                dispatch: case.reference.scope.dispatch,
                claim: lease.token,
                claim_generation: lease.generation,
                disposition: RecoveryDisposition::Completed,
                now_ms: NOW + 1002,
            },
        )
        .await;
        if matches!(outcome, selection_fixture::Inspection::Unjudged) {
            assert!(finish.is_err());
            tx.rollback().await.unwrap();
        } else {
            finish.unwrap();
            tx.commit().await.unwrap();
        }
        assert!(matches!(receipt.actor, NoDeliveryStage::Dispatch { .. }));
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM finance.provider_attempt", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        drop(publisher);
        scratch.finish().await;
    }
}

#[tokio::test]
async fn a_first_rejected_inspection_with_correction_remaining_cannot_seal_the_job() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let mut pipeline =
        intent_fixture::inspection_pipeline(df_imagery::InspectionRequirement::Required);
    pipeline.inspection.as_mut().unwrap().maximum_corrections = 1;
    let fixture = Fixture::create_with_pipeline(&client, false, true, Some(pipeline)).await;
    let case = terminal_fixture::inspection(
        &mut client,
        &fixture,
        selection_fixture::Inspection::Rejected,
    )
    .await;
    assert!(
        qualify_no_delivery(&case.job, &case.evidence, case.selected.candidate.as_ref()).is_err()
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_no_delivery", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        client
            .query_one("SELECT reserved_bytes FROM media.quota_usage", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1000
    );
    scratch.finish().await;
}

#[tokio::test]
async fn required_preflight_has_no_supplier_dispatch_and_closes_actual_execution_capacity() {
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create_with_pipeline(
        &client,
        false,
        true,
        Some(intent_fixture::inspection_pipeline(
            df_imagery::InspectionRequirement::Required,
        )),
    )
    .await;
    let (context, selected) = selection_fixture::prepare_candidate(&mut client, &fixture).await;
    let pending = terminal_fixture::claim_inspection(&mut client, &context, &selected).await;
    assert!(!client.query_one("SELECT has_table_privilege('df_imagery_output','imagery.image_finance_admission_binding','SELECT')",&[]).await.unwrap().get::<_,bool>(0));
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE df_imagery_output")
        .await
        .unwrap();
    assert_eq!(
        tx.query_one(
            "SELECT imagery.stage_has_finance($1,$2,$3)",
            &[
                &owner_scope_bytes(&pending.job.owner),
                &pending.job.job.as_bytes().as_slice(),
                &pending.claim.scope.stage.as_bytes().as_slice()
            ]
        )
        .await
        .unwrap()
        .get::<_, Option<bool>>(0),
        Some(false)
    );
    assert_eq!(
        tx.query_one(
            "SELECT imagery.stage_has_finance($1,$2,$3)",
            &[
                &owner_scope_bytes(&pending.job.owner),
                &pending.job.job.as_bytes().as_slice(),
                &[99u8; 16].as_slice()
            ]
        )
        .await
        .unwrap()
        .get::<_, Option<bool>>(0),
        None
    );
    tx.commit().await.unwrap();
    assert!(
        LocalInspectionCommand::new(
            &pending.job,
            pending.claim.clone(),
            pending.candidate.scope.dispatch,
            pending.source_digest,
            LocalInspectionReason::Preparation,
            NOW + 4
        )
        .is_err()
    );
    let proof = qualify_required_preflight(
        &pending.job,
        &pending.claim,
        &pending.candidate,
        pending.source_digest,
        LocalInspectionReason::Preparation,
        NOW + 4,
    )
    .unwrap();
    let actor = TerminalActor::UnsentStage(pending.claim.clone());
    let command = TerminalCommand {
        actor: actor.clone(),
        tail: actor,
        authority: TerminalAuthority::Claim(pending.claim.clone()),
        expected_job_revision: pending.job.revision,
        candidate_dispatch: Some(pending.candidate.scope.dispatch),
        now_ms: NOW + 10,
    };
    let publisher = selection_fixture::publisher(&scratch, &fixture).await;
    let receipt = publisher
        .no_delivery(&NoDeliveryRequest {
            terminal: &command,
            cause: NoDeliveryCause::RequiredPreflight(&proof),
        })
        .await
        .unwrap();
    assert!(matches!(receipt.actor, NoDeliveryStage::UnsentStage(_)));
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM imagery.image_dispatch_attempt WHERE stage_id=$1",
                &[&pending.claim.scope.stage.as_bytes().as_slice()]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM imagery.image_local_inspection", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(client.query_one("SELECT in_flight FROM imagery.image_execution_capacity WHERE capacity_scope='test'",&[]).await.unwrap().get::<_,i64>(0),0);
    assert_eq!(
        client
            .query_one("SELECT state FROM media.asset", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "rejected"
    );
    assert_eq!(
        client
            .query_one("SELECT reserved_bytes FROM media.quota_usage", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    drop(publisher);
    scratch.finish().await;
}

#[tokio::test]
async fn best_effort_local_checkpoint_publishes_explicit_unjudged_without_a_fake_decision() {
    use df_imagery::runtime::{ImageDispatchAdmission, PreparedImageOutput};
    let scratch = Scratch::open().await;
    let mut client = connect(&scratch.config).await;
    let fixture = Fixture::create_with_pipeline(
        &client,
        false,
        true,
        Some(intent_fixture::inspection_pipeline(
            df_imagery::InspectionRequirement::BestEffort,
        )),
    )
    .await;
    let (context, mut selected) = selection_fixture::prepare(&mut client, &fixture).await;
    let pending = terminal_fixture::claim_inspection(&mut client, &context, &selected).await;
    let command = LocalInspectionCommand::new(
        &pending.job,
        pending.claim.clone(),
        pending.candidate.scope.dispatch,
        pending.source_digest,
        LocalInspectionReason::Preparation,
        NOW + 4,
    )
    .unwrap();
    let tx = client.transaction().await.unwrap();
    let checkpoint = df_persistence::imagery_execution::record_local_inspection_in(&tx, &command)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    selected.local_inspections.push(checkpoint.clone());
    let publisher = selection_fixture::publisher(&scratch, &fixture).await;
    let mut wrong = selected.clone();
    wrong.local_inspections[0].source_digest[0] ^= 1;
    assert!(publisher.select_output(&context, &wrong).await.is_err());
    publisher.select_output(&context, &selected).await.unwrap();
    let output = publisher
        .store_output(
            &context,
            &PreparedImageOutput {
                master: MASTER.to_vec(),
                display: None,
            },
        )
        .await
        .unwrap();
    selection_fixture::complete_final(&mut client, &context, output).await;
    publisher
        .outcome(
            &context.job,
            context.dispatch,
            DispatchMode::Local,
            Some(true),
        )
        .await
        .unwrap();
    publisher.publish(&context).await.unwrap();
    let tx = client.transaction().await.unwrap();
    let published = read_published_inspection_in(&tx, &context.job.owner, &context.job.job)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(published.evidence,PublishedInspectionEvidence::Local(actual) if actual==checkpoint)
    );
    tx.commit().await.unwrap();
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM imagery.image_decision_checkpoint",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        client
            .query_one(
                "SELECT count(*) FROM imagery.image_dispatch_attempt WHERE stage_id=$1",
                &[&pending.claim.scope.stage.as_bytes().as_slice()]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    drop(publisher);
    scratch.finish().await;
}
