//! Real raw candidate evidence and a distinct final CPU stage over one original allocation.
use super::*;
use df_imagery::execution::*;
use df_imagery::publication::FinalOutputSelection;
use df_imagery::runtime::*;
use df_persistence::imagery_execution::{complete_dispatch_in, record_candidate_observation_in};
use df_persistence::media_catalog::PostgresMediaCatalog;
use df_persistence::postgres_tls::PostgresTransport;
use std::sync::Arc;

pub(super) struct Ids(AtomicU64);
impl IdSource for Ids {
    fn next_id_bytes(&self) -> [u8; 16] {
        let mut bytes = [77; 16];
        bytes[8..].copy_from_slice(&self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes());
        bytes
    }
}
pub(super) struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        NOW + 10
    }
}
pub(super) type Publisher = NativeImagePublisher<NativeFileStore, Ids, TestClock>;

pub(super) async fn publisher(scratch: &Scratch, fixture: &Fixture) -> Publisher {
    let transport = PostgresTransport::PlaintextLoopbackOnly;
    let mut config = scratch.config.clone();
    config.options("-c role=df_imagery_output");
    NativeImagePublisher::new(
        RuntimePublicationSettings {
            catalog: CatalogConfig::default(),
            layout: KeyLayout::new(ObjectEnvironment::new("test").unwrap()),
            decode_concurrency: 1,
        },
        RuntimePublicationPorts {
            catalog: Arc::new(
                PostgresMediaCatalog::open_existing(config.clone(), transport.clone(), 2)
                    .await
                    .unwrap(),
            ),
            publication: PostgresImagePublication::open_existing(config, transport, 2)
                .await
                .unwrap(),
            objects: Arc::new(NativeFileStore::new(&fixture.root, 1024 * 1024).unwrap()),
            ids: Arc::new(Ids(AtomicU64::new(100))),
            clock: Arc::new(TestClock),
        },
    )
    .unwrap()
}

pub(super) async fn prepare(
    client: &mut Client,
    fixture: &Fixture,
) -> (PublicationContext, FinalOutputSelection) {
    prepare_stage(client, fixture, true, "reported").await
}

pub(super) async fn prepare_candidate(
    client: &mut Client,
    fixture: &Fixture,
) -> (PublicationContext, FinalOutputSelection) {
    prepare_stage(client, fixture, false, "reported").await
}

pub(super) async fn prepare_unknown_candidate(
    client: &mut Client,
    fixture: &Fixture,
) -> (PublicationContext, FinalOutputSelection) {
    prepare_stage(client, fixture, true, "unknown").await
}

async fn prepare_stage(
    client: &mut Client,
    fixture: &Fixture,
    final_output: bool,
    evidence: &str,
) -> (PublicationContext, FinalOutputSelection) {
    let command = fixture.command();
    let tx = client.transaction().await.unwrap();
    df_persistence::imagery_execution::set_capacity_bounds_in(
        &tx,
        "test",
        &CapacityBounds::new(2, 4, 1_048_576).unwrap(),
        NOW,
    )
    .await
    .unwrap();
    let job = df_persistence::read_in(&tx, &fixture.owner, command.job)
        .await
        .unwrap()
        .unwrap();
    let config_digest: [u8; 32] = Sha256::digest(job.config.bytes()).into();
    tx.batch_execute("DELETE FROM imagery.image_dispatch_checkpoint; UPDATE imagery.image_dispatch_attempt SET dispatch_state='sent',usage_revision=0").await.unwrap();
    tx.execute("INSERT INTO imagery.image_dispatch_usage(dispatch_id,owner_scope,job_id,stage_id,revision,provider,model,cause,detail,maxima,evidence,units,usable,observed_at_ms,raw_output) VALUES($1,$2,$3,$4,1,'fixture','fixture','image','generation','[{\"category\":\"output\",\"quantity\":1}]',$7,$8::text::jsonb,true,$5,$6)", &[&command.dispatch.as_bytes().as_slice(),&owner_scope_bytes(&fixture.owner),&command.job.as_bytes().as_slice(),&command.stage.as_bytes().as_slice(),&(NOW+1),&MASTER,&evidence,&if evidence=="unknown" {"[]"} else {"[{\"category\":\"output\",\"quantity\":1}]"}]).await.unwrap();
    let basis = StageBasis {
        kind: StageKind::Generation,
        input_revision: 1,
        input_digest: [14; 32],
        config_revision: 1,
        config_digest,
        parent_dispatch: None,
    };
    let observed = CandidateObservation::inspect(
        ExecutionScope {
            owner: fixture.owner.clone(),
            job: command.job,
            stage: command.stage,
            dispatch: command.dispatch,
        },
        basis,
        1,
        ObservationReceipt {
            usage_revision: 1,
            observed_at_ms: NOW + 1,
        },
        MASTER,
        ImageProofLimits::default(),
        fixture.proofs[0].intent(),
    )
    .unwrap();
    let candidate = record_candidate_observation_in(&tx, &observed)
        .await
        .unwrap();
    let complete = CompleteCommand::new(
        StageClaim {
            scope: StageScope {
                owner: fixture.owner.clone(),
                job: command.job,
                stage: command.stage,
            },
            token: command.claim,
            generation: 1,
        },
        command.dispatch,
        CompletionOutcome::Candidate {
            observation_version: 1,
            usage_revision: 1,
        },
        NOW + 2,
    )
    .unwrap();
    complete_dispatch_in(&tx, &complete).await.unwrap();
    tx.execute("UPDATE imagery.image_dispatch_attempt SET settlement='unknown_outstanding' WHERE dispatch_id=$1", &[&command.dispatch.as_bytes().as_slice()]).await.unwrap();
    let mut execution = b"resolved recipe with original plan and measured candidate".to_vec();
    execution.extend_from_slice(&recipe_digest(job.recipe.as_ref().unwrap()));
    execution.extend_from_slice(&candidate.descriptor.sha256);
    let recipe = FrozenOutput::new(fixture.proofs[0].intent(), None, execution)
        .unwrap()
        .snapshot()
        .unwrap();
    if !final_output {
        let stage = df_persistence::imagery_execution::read_stage_in(
            &tx,
            &fixture.owner,
            &command.job,
            &command.stage,
        )
        .await
        .unwrap();
        let job = df_persistence::read_in(&tx, &fixture.owner, command.job)
            .await
            .unwrap()
            .unwrap();
        tx.commit().await.unwrap();
        return (
            PublicationContext {
                job,
                stage,
                dispatch: command.dispatch,
                claim: command.claim,
                now_ms: NOW + 10,
                usage_revision: 1,
                recovery: None,
            },
            FinalOutputSelection {
                allocation_stage: command.stage,
                candidate: Some(candidate),
                resolved_recipe: recipe,
                decisions: vec![],
                local_inspections: vec![],
            },
        );
    }
    let final_stage = ExecutionStageId::from_bytes(&[31; 16]).unwrap();
    let final_dispatch = DispatchId::from_bytes(&[33; 16]).unwrap();
    let final_claim = ExecutionClaim::from_bytes(&[32; 16]).unwrap();
    tx.execute("INSERT INTO imagery.image_execution_stage(job_id,stage_id,owner_scope,stage_kind,stage_state,claim_token,claim_generation,lease_until_ms,input_revision,input_digest,config_revision,config_digest,capacity_scope,local_accounting,parent_stage_id,parent_dispatch,created_at_ms,updated_at_ms) SELECT job_id,$1,owner_scope,'local_transform','admitted',$2,1,$7,$3,$4,config_revision,config_digest,capacity_scope,true,$6,$5,created_at_ms,updated_at_ms FROM imagery.image_execution_stage WHERE stage_id=$6", &[&final_stage.as_bytes().as_slice(),&final_claim.as_bytes().as_slice(),&i64::from(recipe.revision()),&recipe_digest(&recipe).as_slice(),&command.dispatch.as_bytes().as_slice(),&command.stage.as_bytes().as_slice(),&(NOW+1000)]).await.unwrap();
    tx.execute("INSERT INTO imagery.image_dispatch_attempt(dispatch_id,job_id,stage_id,dispatch_seq,dispatch_state,usage_revision,book_call_id,claim_token,claim_generation,claim_lease_until_ms,local_accounting,created_at_ms,updated_at_ms) VALUES($1,$2,$3,0,'prepared',0,$4,$5,1,$7,true,$6,$6)", &[&final_dispatch.as_bytes().as_slice(),&command.job.as_bytes().as_slice(),&final_stage.as_bytes().as_slice(),&book_call_id_for(&final_dispatch),&final_claim.as_bytes().as_slice(),&NOW,&(NOW+1000)]).await.unwrap();
    tx.execute("INSERT INTO imagery.image_stage_cursor(job_id,stage_id,current_dispatch,updated_at_ms) VALUES($1,$2,$3,$4)", &[&command.job.as_bytes().as_slice(),&final_stage.as_bytes().as_slice(),&final_dispatch.as_bytes().as_slice(),&NOW]).await.unwrap();
    let receipts = df_persistence::imagery_runtime::read_sources_in(&tx, &candidate.scope)
        .await
        .unwrap()
        .unwrap();
    df_persistence::imagery_runtime::record_sources_in(
        &tx,
        &ExecutionScope {
            owner: fixture.owner.clone(),
            job: command.job,
            stage: final_stage,
            dispatch: final_dispatch,
        },
        &receipts,
    )
    .await
    .unwrap();
    tx.execute("UPDATE media.asset SET state='uploading',step=NULL,step_owner=NULL,step_lease_until_ms=NULL WHERE asset_id=$1", &[&fixture.held.asset.as_bytes().as_slice()]).await.unwrap();
    let stage = df_persistence::imagery_execution::read_stage_in(
        &tx,
        &fixture.owner,
        &command.job,
        &final_stage,
    )
    .await
    .unwrap();
    let job = df_persistence::read_in(&tx, &fixture.owner, command.job)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    (
        PublicationContext {
            job,
            stage,
            dispatch: final_dispatch,
            claim: final_claim,
            now_ms: NOW + 10,
            usage_revision: 0,
            recovery: None,
        },
        FinalOutputSelection {
            allocation_stage: command.stage,
            candidate: Some(candidate),
            resolved_recipe: recipe,
            decisions: vec![],
            local_inspections: vec![],
        },
    )
}

pub(super) async fn complete_final(
    client: &mut Client,
    context: &PublicationContext,
    checkpoint: OutputCheckpoint,
) {
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "UPDATE imagery.image_dispatch_attempt SET dispatch_state='sent' WHERE dispatch_id=$1",
        &[&context.dispatch.as_bytes().as_slice()],
    )
    .await
    .unwrap();
    let command = CompleteCommand::new(
        StageClaim {
            scope: StageScope {
                owner: context.job.owner.clone(),
                job: context.job.job,
                stage: context.stage.stage,
            },
            token: context.claim,
            generation: context.stage.claim_generation,
        },
        context.dispatch,
        CompletionOutcome::Succeeded {
            usage_revision: checkpoint.usage_revision,
            checkpoint,
        },
        NOW + 10,
    )
    .unwrap();
    complete_dispatch_in(&tx, &command).await.unwrap();
    tx.commit().await.unwrap();
}

#[derive(Clone, Copy)]
pub(super) enum Inspection {
    Pass,
    Rejected,
    Unjudged,
    UnjudgedKnown,
}

pub(super) async fn inspect(
    client: &mut Client,
    context: &PublicationContext,
    selected: &FinalOutputSelection,
    outcome: Inspection,
) -> df_imagery::publication::FinalDecisionRef {
    use df_persistence::imagery_execution::record_decision_observation_in;
    let candidate = selected.candidate.as_ref().unwrap();
    let stage = ExecutionStageId::from_bytes(&[41; 16]).unwrap();
    let dispatch = DispatchId::from_bytes(&[43; 16]).unwrap();
    let claim = ExecutionClaim::from_bytes(&[42; 16]).unwrap();
    let answer = serde_json::json!({"frame":if matches!(outcome,Inspection::Rejected){"grid_or_collage"}else{"single_photograph"},
        "medium":"photograph","text":"none","dress":"no_people","setting":"period","people_present":"none"});
    let body = serde_json::json!({"choices":[{"message":{"content":answer.to_string()}}]})
        .to_string()
        .into_bytes();
    let raw=serde_json::json!({"schema_revision":1,"status":if matches!(outcome,Inspection::Unjudged|Inspection::UnjudgedKnown){503}else{200},"body":body}).to_string().into_bytes();
    let (verdict, failure, unjudged) = match outcome {
        Inspection::Pass => ("pass", serde_json::Value::Null, serde_json::Value::Null),
        Inspection::Rejected => (
            "failed",
            serde_json::json!({"reason":"grid_or_collage","expected":null,"seen":null}),
            serde_json::Value::Null,
        ),
        Inspection::Unjudged | Inspection::UnjudgedKnown => (
            "unjudged",
            serde_json::Value::Null,
            serde_json::json!("provider"),
        ),
    };
    let payload=serde_json::json!({"schema_revision":1,"contract_revision":1,"verdict":verdict,"failure":failure,
        "unjudged_reason":unjudged,"visible_features":[],"source_binding":null}).to_string().into_bytes();
    let tx = client.transaction().await.unwrap();
    tx.execute("INSERT INTO imagery.image_execution_stage(job_id,stage_id,owner_scope,stage_kind,stage_state,claim_token,claim_generation,lease_until_ms,input_revision,input_digest,config_revision,config_digest,capacity_scope,local_accounting,parent_stage_id,parent_dispatch,created_at_ms,updated_at_ms) SELECT job_id,$1,owner_scope,'inspection','admitted',$2,1,$6,1,$3,config_revision,config_digest,capacity_scope,true,stage_id,$4,created_at_ms,updated_at_ms FROM imagery.image_execution_stage WHERE stage_id=$5", &[&stage.as_bytes().as_slice(),&claim.as_bytes().as_slice(),&candidate.descriptor.sha256.as_slice(),&candidate.scope.dispatch.as_bytes().as_slice(),&candidate.scope.stage.as_bytes().as_slice(),&(NOW+1000)]).await.unwrap();
    tx.execute("INSERT INTO imagery.image_dispatch_attempt(dispatch_id,job_id,stage_id,dispatch_seq,dispatch_state,usage_revision,book_call_id,claim_token,claim_generation,claim_lease_until_ms,local_accounting,created_at_ms,updated_at_ms) VALUES($1,$2,$3,0,'sent',0,$4,$5,1,$7,true,$6,$6)", &[&dispatch.as_bytes().as_slice(),&context.job.job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&book_call_id_for(&dispatch),&claim.as_bytes().as_slice(),&NOW,&(NOW+1000)]).await.unwrap();
    tx.execute("INSERT INTO imagery.image_stage_cursor(job_id,stage_id,current_dispatch,updated_at_ms) VALUES($1,$2,$3,$4)", &[&context.job.job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&dispatch.as_bytes().as_slice(),&NOW]).await.unwrap();
    tx.execute("INSERT INTO imagery.image_dispatch_usage(dispatch_id,owner_scope,job_id,stage_id,revision,provider,model,cause,detail,maxima,evidence,units,usable,observed_at_ms,raw_output) VALUES($1,$2,$3,$4,1,'openai','fixture-model','image','inspection','[{\"category\":\"output\",\"quantity\":1}]',$5,'[{\"category\":\"output\",\"quantity\":1}]',$6,$7,$8)", &[&dispatch.as_bytes().as_slice(),&owner_scope_bytes(&context.job.owner),&context.job.job.as_bytes().as_slice(),&stage.as_bytes().as_slice(),&if matches!(outcome,Inspection::Unjudged){"unknown"}else{"reported"},&!matches!(outcome,Inspection::Unjudged|Inspection::UnjudgedKnown),&(NOW+3),&raw]).await.unwrap();
    let basis = df_persistence::imagery_execution::read_stage_basis_in(
        &tx,
        &context.job.owner,
        &context.job.job,
        &stage,
    )
    .await
    .unwrap();
    let observation = DecisionObservation::new(
        ExecutionScope {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage,
            dispatch,
        },
        DecisionKind::Inspection,
        1,
        DecisionBasis {
            schema_revision: 1,
            validator_revision: 1,
            stage: basis,
        },
        payload,
        ObservationReceipt {
            usage_revision: 1,
            observed_at_ms: NOW + 3,
        },
        262_144,
    )
    .unwrap();
    record_decision_observation_in(&tx, &observation)
        .await
        .unwrap();
    let complete = CompleteCommand::new(
        StageClaim {
            scope: StageScope {
                owner: context.job.owner.clone(),
                job: context.job.job,
                stage,
            },
            token: claim,
            generation: 1,
        },
        dispatch,
        CompletionOutcome::Decision {
            observation_version: 1,
            usage_revision: 1,
        },
        NOW + 4,
    )
    .unwrap();
    complete_dispatch_in(&tx, &complete).await.unwrap();
    tx.execute(
        "UPDATE imagery.image_dispatch_attempt SET settlement=$1 WHERE dispatch_id=$2",
        &[
            &if matches!(outcome, Inspection::Unjudged) {
                "unknown_outstanding"
            } else if matches!(outcome, Inspection::UnjudgedKnown) {
                "unusable_settled"
            } else {
                "settled"
            },
            &dispatch.as_bytes().as_slice(),
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    df_imagery::publication::FinalDecisionRef::from_observation(&observation)
}

pub(super) struct DecisionEvidence {
    pub proof: df_imagery::runtime::QualifiedDecision,
    pub usage: df_imagery::runtime::RuntimeUsage,
}

pub(super) async fn economic_proof(
    client: &mut Client,
    context: &PublicationContext,
    selected: &FinalOutputSelection,
    reference: &df_imagery::publication::FinalDecisionRef,
) -> DecisionEvidence {
    let tx = client.transaction().await.unwrap();
    let job = df_persistence::read_in(&tx, &context.job.owner, context.job.job)
        .await
        .unwrap()
        .unwrap();
    let decision = df_persistence::imagery_execution::read_decision_checkpoint_in(
        &tx,
        &reference.scope.owner,
        &reference.scope.job,
        &reference.scope.stage,
        &reference.scope.dispatch,
    )
    .await
    .unwrap()
    .unwrap();
    let usage = df_persistence::imagery_runtime::read_usage_in(
        &tx,
        &reference.scope,
        df_imagery::cost::UsageRevision(u64::from(reference.usage_revision)),
    )
    .await
    .unwrap()
    .unwrap();
    tx.commit().await.unwrap();
    let proof = df_imagery::runtime::qualify_economic_decision(
        &job,
        &df_imagery::runtime::PublicationDecisionEvidence {
            decision,
            usage: usage.clone(),
            source_candidate: selected.candidate.clone(),
        },
    )
    .unwrap();
    DecisionEvidence { proof, usage }
}
