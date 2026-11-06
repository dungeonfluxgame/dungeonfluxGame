use super::*;
use df_imagery::runtime::usage_projection::{ImageUsageAmount, UsageProjectionLimits};

#[tokio::test]
async fn usage_projection_reads_scoped_latest_bounded_metadata_without_supplier_payloads() {
    let scratch = Scratch::open().await;
    let config = scratch.config.clone();
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    let mut client = connect(&config).await;
    client.batch_execute("CREATE TABLE IF NOT EXISTS imagery.schema_migration(version integer PRIMARY KEY,applied_at timestamptz NOT NULL DEFAULT now()); INSERT INTO imagery.schema_migration(version) VALUES(38),(40) ON CONFLICT DO NOTHING").await.unwrap();
    let ids = TestIds(Arc::new(AtomicU64::new(500)));
    let execution = PostgresImageExecution::open(config.clone(), transport.clone(), 1)
        .await
        .unwrap();
    let scope = CapacityScope::new("runtime/raw-receipts").unwrap();
    execution
        .set_capacity_bounds(
            scope.as_str(),
            &CapacityBounds::new(2, 2, 4_194_304).unwrap(),
            NOW,
        )
        .await
        .unwrap();
    execution
        .set_execution_gate(scope.as_str(), 1, true, NOW)
        .await
        .unwrap();
    let journal = PostgresRuntimeJournal::open_existing(
        config.clone(),
        transport.clone(),
        1,
        Arc::new(ids.clone()),
        scope.clone(),
        1,
        true,
    )
    .unwrap();
    let image_config = ImageConfig::defaults().unwrap();
    let service = ImageService::new(
        journal,
        image_config,
        TestClock,
        ids,
        TestAuthority(owner()),
    )
    .unwrap();
    let mut request = request();
    let output = request.output.delivered();
    request.output = OutputProfile::provider(Dimensions::new(1024, 1024).unwrap(), output);
    request.operation = ImageOperation::Generate(GenerateSpec {
        prompt: TextPrompt::new("fixture response", 1).unwrap(),
        references: vec![],
    });
    request.identity.fingerprint = service.canonical_fingerprint(&request).unwrap();
    let view = match service.request(&(), &request).await.unwrap() {
        RequestOutcome::Recorded(view) => view,
        _ => panic!("new request"),
    };
    let mut empty_client = connect(&config).await;
    let empty_tx = empty_client.transaction().await.unwrap();
    let empty = read_usage_projection_in(
        &empty_tx,
        &owner(),
        view.job,
        UsageProjectionLimits {
            max_attempts: 8,
            max_observations: 8,
            max_unit_lines: 16,
            max_word_bytes: 128,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(empty.attempts.is_empty());
    assert!(empty.observations.is_empty());
    empty_tx.commit().await.unwrap();
    drop(empty_client);
    let queued = execution
        .scan_queue(scope.as_str(), 1, NOW)
        .await
        .unwrap()
        .remove(0);
    let claim = df_imagery::execution::ClaimToken::from_bytes(&[13; 16]).unwrap();
    let stage = execution
        .claim(&ClaimCommand::new(owner(), view.job, queued.stage, 1, claim, 10_000, NOW).unwrap())
        .await
        .unwrap();
    let dispatch = df_imagery::execution::DispatchId::from_bytes(&[14; 16]).unwrap();
    execution
        .prepare(
            &PrepareCommand::new(
                StageClaim {
                    scope: StageScope {
                        owner: owner(),
                        job: view.job,
                        stage: queued.stage,
                    },
                    token: claim,
                    generation: stage.claim_generation,
                },
                dispatch,
                DispatchMode::Local,
                NOW,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    execution
        .mark_sent(
            &MarkSentCommand::new(
                owner(),
                view.job,
                queued.stage,
                dispatch,
                claim,
                stage.claim_generation,
                NOW,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let mut restricted = config.clone();
    restricted.options("-c role=df_imagery_usage");
    let usage =
        PostgresImageUsage::open_existing(restricted.clone(), transport.clone(), 1).unwrap();
    let record = RuntimeUsage {
        owner: owner(),
        job: view.job,
        stage: queued.stage,
        dispatch,
        revision: UsageRevision(1),
        route: CostRoute {
            provider: "openai".to_owned(),
            model: "fixture-image-model".to_owned(),
            cause: "scene_plate".to_owned(),
            detail: "receipt-test".to_owned(),
            maxima: vec![CostUnits {
                category: "image_output".to_owned(),
                quantity: 1056,
            }],
        },
        evidence: UsageEvidence::Reported,
        units: vec![CostUnits {
            category: "image_output".to_owned(),
            quantity: 720,
        }],
        usable: true,
        at_ms: NOW as u64,
        raw_output: Some(vec![137, 80, 78, 71, 13, 10, 26, 10]),
    };
    usage.record_usage(&record).await.unwrap();
    let limits = UsageProjectionLimits {
        max_attempts: 8,
        max_observations: 8,
        max_unit_lines: 16,
        max_word_bytes: 128,
    };
    let tx = client.transaction().await.unwrap();
    let rows = read_usage_projection_in(&tx, &owner(), view.job, limits)
        .await
        .unwrap()
        .unwrap();
    let revision: i64 = tx
        .query_one(
            "SELECT revision FROM imagery.image_job WHERE job_id=$1",
            &[&view.job.as_bytes().as_slice()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows.job_revision, revision as u64);
    assert_eq!(rows.attempts.len(), 1);
    assert_eq!(
        rows.attempts[0].mode,
        df_imagery::execution::DispatchMode::Local
    );
    assert_eq!(rows.observations.len(), 1);
    assert_eq!(rows.observations[0].revision, UsageRevision(1));
    assert!(rows.observations[0].raw_output.is_none());
    assert_eq!(rows.observations[0].units, record.units);
    let foreign = ImageOwner::Draft {
        account: AccountId::from_bytes(&[99; 16]).unwrap(),
    };
    assert!(
        read_usage_projection_in(&tx, &foreign, view.job, limits)
            .await
            .unwrap()
            .is_none()
    );
    let tiny = UsageProjectionLimits {
        max_word_bytes: 2,
        ..limits
    };
    assert!(matches!(
        read_usage_projection_in(&tx, &owner(), view.job, tiny).await,
        Err(JournalError::InvalidRecord)
    ));
    tx.commit().await.unwrap();
    // Book awaits receive an owned snapshot after the transaction has ended.
    let summary = df_persistence::imagery_runtime::usage_projection::project_rows(
        rows,
        &owner(),
        view.job,
        &NoCost,
        limits,
    )
    .await
    .unwrap();
    assert_eq!(summary.stages[0].amount, ImageUsageAmount::Pending);
    let mut latest = record.clone();
    latest.revision = UsageRevision(3);
    latest.units[0].quantity = 0;
    latest.usable = false;
    latest.raw_output = Some(vec![42; 65_536]);
    usage.record_usage(&latest).await.unwrap();
    let retry = df_imagery::execution::DispatchId::from_bytes(&[15; 16]).unwrap();
    let basis=client.query_one(
        "SELECT input_revision,input_digest,config_revision,config_digest FROM imagery.image_execution_stage WHERE job_id=$1 AND stage_id=$2",
        &[&view.job.as_bytes().as_slice(),&queued.stage.as_bytes().as_slice()],
    ).await.unwrap();
    let second_stage = df_imagery::execution::ExecutionStageId::from_bytes(&[16; 16]).unwrap();
    execution
        .enqueue(
            &EnqueueCommand::new(
                StageScope {
                    owner: owner(),
                    job: view.job,
                    stage: second_stage,
                },
                StageBasis {
                    kind: StageKind::Generation,
                    input_revision: u32::try_from(basis.get::<_, i64>("input_revision")).unwrap(),
                    input_digest: basis.get::<_, Vec<u8>>("input_digest").try_into().unwrap(),
                    config_revision: u64::try_from(basis.get::<_, i64>("config_revision")).unwrap(),
                    config_digest: basis.get::<_, Vec<u8>>("config_digest").try_into().unwrap(),
                    parent_dispatch: None,
                },
                stage.planned_publication,
                scope.clone(),
                true,
                NOW,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let second_claim = df_imagery::execution::ClaimToken::from_bytes(&[17; 16]).unwrap();
    let second = execution
        .claim(
            &ClaimCommand::new(
                owner(),
                view.job,
                second_stage,
                1,
                second_claim,
                10_000,
                NOW,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    execution
        .prepare(
            &PrepareCommand::new(
                StageClaim {
                    scope: StageScope {
                        owner: owner(),
                        job: view.job,
                        stage: second_stage,
                    },
                    token: second_claim,
                    generation: second.claim_generation,
                },
                retry,
                DispatchMode::Local,
                NOW,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    let rows = read_usage_projection_in(&tx, &owner(), view.job, limits)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rows.attempts.len(), 2);
    assert_eq!(rows.observations.len(), 1);
    assert_eq!(rows.observations[0].revision, UsageRevision(3));
    assert_eq!(rows.observations[0].units[0].quantity, 0);
    assert!(!rows.observations[0].usable);
    assert!(rows.observations[0].raw_output.is_none());
    assert!(matches!(
        read_usage_projection_in(
            &tx,
            &owner(),
            view.job,
            UsageProjectionLimits {
                max_attempts: 1,
                ..limits
            }
        )
        .await,
        Err(JournalError::InvalidRecord)
    ));
    tx.commit().await.unwrap();
    assert!(client.execute("UPDATE imagery.image_dispatch_attempt SET book_call_id='wrong-book-call' WHERE dispatch_id=$1",
        &[&retry.as_bytes().as_slice()]).await.is_err(),"immutable dispatch lineage must reject call identity changes");
    let tx = client.transaction().await.unwrap();
    let canonical = read_usage_projection_in(&tx, &owner(), view.job, limits)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(canonical.attempts.len(), 2);
    assert!(
        canonical
            .attempts
            .iter()
            .any(|attempt| attempt.scope.stage == second_stage && attempt.scope.dispatch == retry)
    );
    assert_eq!(canonical.observations.len(), 1);
    assert_eq!(canonical.observations[0].dispatch, dispatch);
    assert_eq!(canonical.observations[0].revision, UsageRevision(3));
    assert_eq!(canonical.observations[0].units[0].quantity, 0);
    assert!(canonical.observations[0].raw_output.is_none());
    let stored_call: String = tx
        .query_one(
            "SELECT book_call_id FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
            &[&retry.as_bytes().as_slice()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored_call, pinned_call_id(retry.as_bytes()));
    tx.commit().await.unwrap();
    drop(usage);
    drop(service);
    drop(execution);
    drop(client);
    scratch.finish().await;
}
