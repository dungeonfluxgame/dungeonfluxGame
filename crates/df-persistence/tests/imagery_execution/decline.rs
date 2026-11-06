//! Execution boundary fixtures install immutable protocol metadata as administrator evidence.
//! Actual HTTP qualification and durable Book acknowledgment are covered by native producer suites.
use super::*;
use df_imagery::execution::{DeclineRetryCommand, ExecutionScope, StageClaim, StageScope};
use df_persistence::imagery_execution::{
    read_decline_reference_in, read_stage_basis_in, requeue_declined_in, scan_recovery_in,
};
use sha2::{Digest, Sha256};

fn scope() -> ExecutionScope {
    ExecutionScope {
        owner: owner(),
        job: job_id(0x11),
        stage: stage_id(0x21),
        dispatch: dispatch_id(0x81),
    }
}
fn original_claim() -> StageClaim {
    StageClaim {
        scope: StageScope {
            owner: owner(),
            job: job_id(0x11),
            stage: stage_id(0x21),
        },
        token: ClaimToken::from_bytes(&id(0x71)).unwrap(),
        generation: 1,
    }
}
fn response() -> Vec<u8> {
    serde_json::json!({"schema_revision":2,"kind":"image-api-failure","exchange":"submit","submission":"rejected_submit","decline_contract":"openai-images-429-v1","status":429,"body":br#"{"error":{"code":"rate_limit_exceeded"}}"#.to_vec(),"transport_request_id":"owned-loopback-request","retry_after_values":Vec::<Vec<u8>>::new()}).to_string().into_bytes()
}
async fn fixture_observation(client: &Client, raw: &[u8], version: i64, maximum_attempts: i64) {
    let hash: [u8; 32] = Sha256::digest(raw).into();
    let body = br#"{"error":{"code":"rate_limit_exceeded"}}"#;
    let metadata = serde_json::json!({"schema_revision":2,"contract":"openai-images-429-v1","status":429,"body_sha256":Sha256::digest(body).to_vec(),"body_bytes":body.len(),"transport_request_id_sha256":null,"retry_after_values":[],"backoff_ms":1000}).to_string();
    client.execute(concat!(
        "INSERT INTO imagery.image_decline_observation(dispatch_id,observation_version,owner_scope,job_id,stage_id,usage_revision,raw_sha256,raw_bytes,provider,model,cause,detail,metadata,retry_not_before_ms,max_attempts,retry_deadline_ms,config_snapshot_revision,observed_at_ms) ",
        "VALUES($1,$2,$3,$4,$5,1,$6,$7,'fixture','actual-candidate','imagery','candidate',$8,$9,$10,30000,1,$11)"
    ), &[&id(0x81).as_slice(),&version,&df_imagery::owner_scope_bytes(&owner()),&id(0x11).as_slice(),&id(0x21).as_slice(),&hash.as_slice(),&i64::try_from(raw.len()).unwrap(),&metadata,&at(1004),&maximum_attempts,&at(4)]).await.unwrap();
}
async fn retain_raw(scratch: &Scratch, bytes: Vec<u8>) {
    record_raw_usage(
        &scratch.config(),
        &scope(),
        1,
        Some(bytes),
        df_imagery::cost::UsageEvidence::Unknown,
        false,
    )
    .await;
}
fn declined(version: u32) -> CompleteCommand {
    finish_command(
        0x11,
        0x21,
        0x81,
        0x71,
        1,
        CompletionOutcome::Declined {
            observation_version: version,
            usage_revision: 1,
        },
        at(5),
    )
}
#[test]
fn decline_completion_atomically_reserves_retry_and_fresh_refusal_releases_only_execution_capacity()
{
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client,StageKind::Generation).await;
        let raw=response();
        fixture_observation(&client,&raw,1,2).await;
        retain_raw(&scratch,raw).await;
        let other=connect(&scratch.config()).await;
        let tx=client.transaction().await.unwrap();
        let completed=complete_dispatch_in(&tx,&declined(1)).await.unwrap();
        assert_eq!(completed.state,DispatchState::Declined);
        assert_eq!(completed.settlement,DispatchSettlement::UnknownOutstanding);
        assert_eq!(other.query_one("SELECT dispatch_state FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",&[&id(0x81).as_slice()]).await.unwrap().get::<_,String>(0),"sent");
        tx.commit().await.unwrap();
        let visible=other.query_one(concat!("SELECT d.dispatch_state,s.retry_dispatch_id,s.stage_state FROM imagery.image_dispatch_attempt d ","JOIN imagery.image_execution_stage s USING(job_id,stage_id) WHERE d.dispatch_id=$1"),&[&id(0x81).as_slice()]).await.unwrap();
        assert_eq!(visible.get::<_,String>(0),"declined");
        assert_eq!(visible.get::<_,Option<Vec<u8>>>(1),Some(id(0x81).to_vec()));
        assert_eq!(visible.get::<_,String>(2),"planned");
        assert_eq!(capacity_counts(&client,"live/generation").await,(0,1,128));
        let tx=client.transaction().await.unwrap();
        let command=DeclineRetryCommand{claim:original_claim(),dispatch:dispatch_id(0x81),now_ms:at(6)};
        let before=read_stage_in(&tx,&owner(),&job_id(0x11),&stage_id(0x21)).await.unwrap();
        assert_eq!(requeue_declined_in(&tx,&command).await.unwrap(),before);
        assert_eq!(claim_stage_in(&tx,&claim_command(0x11,0x21,GATE_REVISION,0x72,0,at(99))).await,Err(ExecutionError::Deferred{not_before_ms:at(1004)}));
        let fresh=claim_stage_in(&tx,&claim_command(0x11,0x21,GATE_REVISION,0x72,0,at(1101))).await.unwrap();
        assert_eq!(fresh.claim_generation,3);
        let stopped=release_stage_in(&tx,&ReleaseCommand::new(owner(),job_id(0x11),stage_id(0x21),ClaimToken::from_bytes(&id(0x72)).unwrap(),3,at(1102)).unwrap()).await.unwrap();
        assert_eq!(stopped.state,StageState::Released);
        assert_eq!(requeue_declined_in(&tx,&command).await,Err(ExecutionError::Expired));
        let retained=read_decline_reference_in(&tx,&scope()).await.unwrap().unwrap();
        assert_eq!(retained.finance,Some(hosted_mode(0x81).finance().unwrap()));
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client,"live/generation").await,(0,0,0));
        assert_eq!(dispatch_count(&client,0x11,0x21).await,1);
        let tx=client.transaction().await.unwrap();
        let recovery=scan_recovery_in(&tx,"live/generation",None,10,at(11000)).await.unwrap();
        assert_eq!(recovery.len(),1);
        assert_eq!(recovery[0].current.settlement,DispatchSettlement::UnknownOutstanding);
        assert_eq!(recovery[0].current.dispatch,dispatch_id(0x81));
        tx.commit().await.unwrap();
        scratch.finish().await;
    });
}
#[test]
fn fresh_decline_attempt_keeps_same_stage_basis_and_historical_full_hold() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client, StageKind::Generation).await;
        let raw = response();
        fixture_observation(&client, &raw, 1, 2).await;
        retain_raw(&scratch, raw).await;
        let tx = client.transaction().await.unwrap();
        complete_dispatch_in(&tx, &declined(1)).await.unwrap();
        let original = read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .unwrap();
        let fresh = claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION, 0x72, 0, at(1101)),
        )
        .await
        .unwrap();
        let next = prepare_dispatch_in(
            &tx,
            &prepare_command(
                0x11,
                0x21,
                0x82,
                0x72,
                fresh.claim_generation,
                hosted_mode(0x82),
                at(1102),
            ),
        )
        .await
        .unwrap();
        assert_eq!(next.sequence, 1);
        assert_eq!(next.lineage_parent, Some(dispatch_id(0x81)));
        assert_ne!(
            next.finance,
            read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
                .await
                .unwrap()
                .finance
        );
        assert_eq!(
            read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
                .await
                .unwrap(),
            original
        );
        assert_eq!(original.parent_dispatch, None);
        let retained = read_decline_reference_in(&tx, &scope())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.scope.dispatch, dispatch_id(0x81));
        assert_eq!(
            read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
                .await
                .unwrap()
                .settlement,
            DispatchSettlement::UnknownOutstanding
        );
        let mut foreign = scope();
        foreign.owner = draft_owner();
        assert_eq!(
            read_decline_reference_in(&tx, &foreign).await.unwrap(),
            None
        );
        tx.commit().await.unwrap();
        let tx = client.transaction().await.unwrap();
        let recovered = scan_recovery_in(&tx, "live/generation", None, 10, at(11000))
            .await
            .unwrap();
        assert!(
            recovered
                .iter()
                .any(|row| row.current.dispatch == dispatch_id(0x81))
        );
        let lease = df_persistence::imagery_execution::claim_recovery_in(
            &tx,
            &recovery_command(0x11, 0x21, 0x81, 0x91, at(11000)),
        )
        .await
        .unwrap();
        assert_eq!(lease.stage.current.dispatch, dispatch_id(0x81));
        assert_eq!(
            df_persistence::imagery_execution::finish_recovery_in(
                &tx,
                &df_imagery::execution::FinishRecoveryCommand {
                    owner: owner(),
                    job: job_id(0x11),
                    stage: stage_id(0x21),
                    dispatch: dispatch_id(0x81),
                    claim: lease.token,
                    claim_generation: lease.generation,
                    disposition: df_imagery::execution::RecoveryDisposition::Completed,
                    now_ms: at(11001),
                }
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        tx.commit().await.unwrap();
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 2);
        assert_eq!(capacity_counts(&client, "live/generation").await, (1, 0, 0));
        scratch.finish().await;
    });
}
#[test]
fn decline_missing_or_changed_raw_never_completes_and_cap_does_not_revive() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client, StageKind::Generation).await;
        let raw = response();
        fixture_observation(&client, &raw, 1, 1).await;
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            complete_dispatch_in(&tx, &declined(1)).await,
            Err(ExecutionError::ObservationMissing)
        );
        tx.commit().await.unwrap();
        retain_raw(&scratch, raw.clone()).await;
        let tx = client.transaction().await.unwrap();
        complete_dispatch_in(&tx, &declined(1)).await.unwrap();
        assert_eq!(
            requeue_declined_in(
                &tx,
                &DeclineRetryCommand {
                    claim: original_claim(),
                    dispatch: dispatch_id(0x81),
                    now_ms: at(6)
                }
            )
            .await,
            Err(ExecutionError::Expired)
        );
        assert_eq!(
            claim_stage_in(
                &tx,
                &claim_command(0x11, 0x21, GATE_REVISION, 0x72, 0, at(11000))
            )
            .await,
            Err(ExecutionError::Superseded)
        );
        tx.commit().await.unwrap();
        let mut changed = raw;
        changed.push(b' ');
        fixture_observation(&client, &changed, 2, 1).await;
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            complete_dispatch_in(&tx, &declined(2)).await,
            Err(ExecutionError::ObservationMissing)
        );
        let selected = read_decline_reference_in(&tx, &scope())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected.observation_version, 1);
        assert_eq!(selected.receipt.usage_revision, 1);
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn sealed_record_only_supplier_remains_discoverable_until_cost_ack_but_cpu_never_needs_fake_call() {
    runtime().block_on(async {
        use df_imagery::execution::{TerminalActor,TerminalAuthority,TerminalCommand};
        use df_persistence::imagery_execution::{close_terminal_in,validate_terminal_tail_in,scan_unsent_prepared_in};
        let scratch=Scratch::open().await;
        let mut client=connect(&scratch.config()).await;
        setup_world(&mut client).await;
        super::terminal_regressions::install_seal_schema(&client).await;
        for (job,stage,operation,interest,token,dispatch,kind,capacity) in [
            (0x11,0x21,0x10,0x12,0x71,0x81,StageKind::Generation,"live/generation"),
            (0x31,0x41,0x30,0x32,0x72,0x82,StageKind::LocalTransform,"local/transform"),
        ] {
            let mut enqueue=enqueue_command(job,stage,at(0));
            enqueue.kind=kind;enqueue.local_accounting=true;enqueue.capacity=CapacityScope::new(capacity).unwrap();
            let tx=client.transaction().await.unwrap();
            accept_with_execution_in(&tx,&accept_command(operation,job,interest,DEADLINE_MS,at(0)),&enqueue).await.unwrap();
            tx.commit().await.unwrap();
            claim(&mut client,job,stage,token,0,at(1)).await;
            let tx=client.transaction().await.unwrap();
            prepare_dispatch_in(&tx,&prepare_command(job,stage,dispatch,token,1,DispatchMode::Local,at(2))).await.unwrap();
            tx.commit().await.unwrap();
            let rev=super::terminal_regressions::revision(&client,job).await;
            let scope=ExecutionScope{owner:owner(),job:job_id(job),stage:stage_id(stage),dispatch:dispatch_id(dispatch)};
            let actor=TerminalActor::Dispatch(scope.clone());
            let terminal=TerminalCommand{actor:actor.clone(),tail:actor,authority:TerminalAuthority::Claim(StageClaim{scope:StageScope{owner:owner(),job:job_id(job),stage:stage_id(stage)},token:ClaimToken::from_bytes(&id(token)).unwrap(),generation:1}),expected_job_revision:rev,candidate_dispatch:None,now_ms:at(60000)};
            let tx=client.transaction().await.unwrap();
            validate_terminal_tail_in(&tx,&terminal).await.unwrap();
            super::terminal_regressions::fixture_seal(&tx,&terminal).await;
            close_terminal_in(&tx,&terminal).await.unwrap();
            tx.commit().await.unwrap();
            let tx=client.transaction().await.unwrap();
            let rows=scan_unsent_prepared_in(&tx,capacity,None,10,at(60001)).await.unwrap();
            let stored=read_dispatch_in(&tx,&owner(),&scope.dispatch).await.unwrap();
            assert_eq!(stored.state,DispatchState::Released);
            assert_eq!(stored.settlement,DispatchSettlement::Released);
            assert_eq!(stored.finance,None);
            assert_eq!(rows.len(),if kind==StageKind::LocalTransform{0}else{1});
            let ack:bool=tx.query_one("SELECT cost_unsent_ack FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",&[&scope.dispatch.as_bytes().as_slice()]).await.unwrap().get(0);
            assert_eq!(ack,kind==StageKind::LocalTransform);
            assert_eq!(tx.query_one("SELECT count(*) FROM imagery.image_dispatch_usage WHERE dispatch_id=$1",&[&scope.dispatch.as_bytes().as_slice()]).await.unwrap().get::<_,i64>(0),0);
            tx.commit().await.unwrap();
            assert_eq!(capacity_counts(&client,capacity).await,(0,0,0));
        }
        scratch.finish().await;
    });
}
