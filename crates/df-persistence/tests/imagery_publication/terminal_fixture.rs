//! Actual selected decision and claimed no-dispatch inspection prerequisites for terminal tests.
use super::*;
use df_imagery::execution::*;
use df_imagery::publication::{FinalDecisionRef, FinalOutputSelection};
use df_imagery::runtime::{PublicationContext, PublicationDecisionEvidence, PublicationEvidence};

pub(super) struct InspectionCase {
    pub selected: FinalOutputSelection,
    pub job: df_imagery::ImageJob,
    pub evidence: PublicationEvidence,
    pub reference: FinalDecisionRef,
    pub command: TerminalCommand,
}

pub(super) async fn inspection(
    client: &mut Client,
    fixture: &Fixture,
    outcome: selection_fixture::Inspection,
) -> InspectionCase {
    let (context, selected) = selection_fixture::prepare_candidate(client, fixture).await;
    let reference = selection_fixture::inspect(client, &context, &selected, outcome).await;
    let tx = client.transaction().await.unwrap();
    let job = df_persistence::read_in(&tx, &context.job.owner, context.job.job)
        .await
        .unwrap()
        .unwrap();
    let decision = df_persistence::imagery_execution::read_decision_checkpoint_in(
        &tx,
        &job.owner,
        &job.job,
        &reference.scope.stage,
        &reference.scope.dispatch,
    )
    .await
    .unwrap()
    .unwrap();
    let usage = df_persistence::imagery_runtime::read_usage_in(
        &tx,
        &reference.scope,
        df_imagery::cost::UsageRevision(1),
    )
    .await
    .unwrap()
    .unwrap();
    tx.commit().await.unwrap();
    let candidate = selected.candidate.as_ref().unwrap();
    let evidence = PublicationEvidence {
        decisions: vec![PublicationDecisionEvidence {
            decision,
            usage,
            source_candidate: Some(candidate.clone()),
        }],
        candidates: vec![candidate.clone()],
        local_inspections: vec![],
    };
    let command = TerminalCommand {
        actor: TerminalActor::Dispatch(reference.scope.clone()),
        tail: TerminalActor::Dispatch(reference.scope.clone()),
        authority: TerminalAuthority::Claim(StageClaim {
            scope: StageScope {
                owner: job.owner.clone(),
                job: job.job,
                stage: reference.scope.stage,
            },
            token: ExecutionClaim::from_bytes(&[42; 16]).unwrap(),
            generation: 1,
        }),
        expected_job_revision: job.revision,
        candidate_dispatch: Some(candidate.scope.dispatch),
        now_ms: NOW + 10,
    };
    InspectionCase {
        selected,
        job,
        evidence,
        reference,
        command,
    }
}

pub(super) struct UnsentInspection {
    pub job: df_imagery::ImageJob,
    pub claim: StageClaim,
    pub candidate: CandidateReceipt,
    pub source_digest: [u8; 32],
}

pub(super) struct FailedSupplier {
    pub job: df_imagery::ImageJob,
    pub dispatch: DispatchView,
    pub basis: StageBasis,
    pub usage: df_imagery::runtime::RuntimeUsage,
    pub command: TerminalCommand,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum SupplierFailure {
    NoOutput,
    UnknownBilling,
    UnknownOutput,
}

pub(super) async fn fail_supplier(
    client: &mut Client,
    fixture: &Fixture,
    outcome: SupplierFailure,
) -> FailedSupplier {
    let original = fixture.command();
    let scope = ExecutionScope {
        owner: fixture.owner.clone(),
        job: original.job,
        stage: original.stage,
        dispatch: original.dispatch,
    };
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("DELETE FROM imagery.image_dispatch_checkpoint; UPDATE imagery.image_dispatch_attempt SET dispatch_state='sent',usage_revision=0").await.unwrap();
    tx.execute("INSERT INTO imagery.image_dispatch_usage(dispatch_id,owner_scope,job_id,stage_id,revision,provider,model,cause,detail,maxima,evidence,units,usable,observed_at_ms,raw_output) VALUES($1,$2,$3,$4,1,'fixture','fixture','image','generation','[{\"category\":\"output\",\"quantity\":1}]',$5,'[]',false,$6,$7)",&[&scope.dispatch.as_bytes().as_slice(),&owner_scope_bytes(&scope.owner),&scope.job.as_bytes().as_slice(),&scope.stage.as_bytes().as_slice(),&if outcome==SupplierFailure::UnknownBilling {"unknown"} else {"reported"},&(NOW+1),&b"supplier returned no image".as_slice()]).await.unwrap();
    let claim = StageClaim {
        scope: StageScope {
            owner: scope.owner.clone(),
            job: scope.job,
            stage: scope.stage,
        },
        token: original.claim,
        generation: 1,
    };
    let complete = CompleteCommand::new(
        claim.clone(),
        scope.dispatch,
        if outcome == SupplierFailure::NoOutput {
            CompletionOutcome::Unusable {
                cause: FailureCause::Provider,
                usage_revision: 1,
            }
        } else {
            CompletionOutcome::Unknown
        },
        NOW + 2,
    )
    .unwrap();
    df_persistence::imagery_execution::complete_dispatch_in(&tx, &complete)
        .await
        .unwrap();
    let dispatch =
        df_persistence::imagery_execution::read_dispatch_in(&tx, &scope.owner, &scope.dispatch)
            .await
            .unwrap();
    let basis = df_persistence::imagery_execution::read_stage_basis_in(
        &tx,
        &scope.owner,
        &scope.job,
        &scope.stage,
    )
    .await
    .unwrap();
    let usage = df_persistence::imagery_runtime::read_usage_in(
        &tx,
        &scope,
        df_imagery::cost::UsageRevision(1),
    )
    .await
    .unwrap()
    .unwrap();
    let job = df_persistence::read_in(&tx, &scope.owner, scope.job)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    let command = TerminalCommand {
        actor: TerminalActor::Dispatch(scope.clone()),
        tail: TerminalActor::Dispatch(scope),
        authority: TerminalAuthority::Claim(claim),
        expected_job_revision: job.revision,
        candidate_dispatch: None,
        now_ms: NOW + 10,
    };
    FailedSupplier {
        job,
        dispatch,
        basis,
        usage,
        command,
    }
}

pub(super) async fn claim_inspection(
    client: &mut Client,
    context: &PublicationContext,
    selected: &FinalOutputSelection,
) -> UnsentInspection {
    let candidate = selected.candidate.as_ref().unwrap().clone();
    let claim = StageClaim {
        scope: StageScope {
            owner: context.job.owner.clone(),
            job: context.job.job,
            stage: ExecutionStageId::from_bytes(&[41; 16]).unwrap(),
        },
        token: ExecutionClaim::from_bytes(&[42; 16]).unwrap(),
        generation: 1,
    };
    let tx = client.transaction().await.unwrap();
    tx.execute("INSERT INTO imagery.image_execution_stage(job_id,stage_id,owner_scope,stage_kind,stage_state,claim_token,claim_generation,lease_until_ms,input_revision,input_digest,config_revision,config_digest,capacity_scope,capacity_reserved,local_accounting,parent_stage_id,parent_dispatch,created_at_ms,updated_at_ms) SELECT job_id,$1,owner_scope,'inspection','admitted',$2,1,$3,1,$4,config_revision,config_digest,capacity_scope,true,true,stage_id,$5,$6,$6 FROM imagery.image_execution_stage WHERE job_id=$7 AND stage_id=$8", &[
        &claim.scope.stage.as_bytes().as_slice(),&claim.token.as_bytes().as_slice(),&(NOW+1000),&candidate.descriptor.sha256.as_slice(),&candidate.scope.dispatch.as_bytes().as_slice(),&(NOW+3),&context.job.job.as_bytes().as_slice(),&candidate.scope.stage.as_bytes().as_slice(),
    ]).await.unwrap();
    tx.execute("UPDATE imagery.image_execution_capacity SET in_flight=in_flight+1 WHERE capacity_scope='test'",&[]).await.unwrap();
    tx.execute(
        "UPDATE imagery.image_job SET revision=revision+1,updated_at_ms=$1 WHERE job_id=$2",
        &[&(NOW + 3), &context.job.job.as_bytes().as_slice()],
    )
    .await
    .unwrap();
    let job = df_persistence::read_in(&tx, &context.job.owner, context.job.job)
        .await
        .unwrap()
        .unwrap();
    let source_digest = df_persistence::imagery_runtime::sources_digest_in(&tx, &candidate.scope)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();
    UnsentInspection {
        job,
        claim,
        candidate,
        source_digest,
    }
}
