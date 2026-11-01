//! Actual terminal demand and four finite no-dispatch states, with no invented claim tokens.
use super::terminal_regressions::{fixture_seal, install_seal_schema, revision};
use super::*;
use df_imagery::execution::*;
use df_persistence::imagery_execution::{
    close_terminal_in, scan_unsent_terminal_in, validate_terminal_tail_in,
};
fn command(proof: UnsentTerminal, now_ms: i64) -> TerminalCommand {
    let expected_job_revision = proof.job_revision();
    let (actor, authority, candidate_dispatch) = match proof {
        UnsentTerminal::Unclaimed(p) => (
            TerminalActor::UnclaimedStage(p.scope.clone()),
            TerminalAuthority::Unclaimed(p),
            None,
        ),
        UnsentTerminal::Claimed(p) => (
            TerminalActor::UnsentStage(p.claim.clone()),
            TerminalAuthority::Claim(p.claim),
            None,
        ),
        UnsentTerminal::Deferred(p) => (
            TerminalActor::DeferredStage(p.scope.clone()),
            TerminalAuthority::Deferred(p),
            None,
        ),
        UnsentTerminal::Released(p) => {
            let candidate = p.candidate_dispatch;
            (
                TerminalActor::ReleasedStage(p.scope.clone()),
                TerminalAuthority::Released(p),
                Some(candidate),
            )
        }
    };
    TerminalCommand {
        tail: actor.clone(),
        actor,
        authority,
        expected_job_revision,
        candidate_dispatch,
        now_ms,
    }
}
async fn cancel(client: &mut Client, interest: u8, now: i64) {
    let tx = client.transaction().await.unwrap();
    df_persistence::withdraw_in(
        &tx,
        &owner(),
        ImageInterestId::from_bytes(&id(interest)).unwrap(),
        now,
    )
    .await
    .expect("actual public journal withdrawal");
    tx.commit().await.unwrap();
}
#[test]
fn cancelled_before_first_claim_releases_pending_without_a_fake_claim_and_denies_live_foreign_or_stale()
 {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let before = revision(&client, 0x11).await;
        let live = UnclaimedTerminal {
            scope: StageScope {
                owner: owner(),
                job: job_id(0x11),
                stage: stage_id(0x21),
            },
            job_revision: before,
        };
        let tx = client.transaction().await.unwrap();
        assert!(
            scan_unsent_terminal_in(&tx, "live/generation", None, 128, at(1))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            validate_terminal_tail_in(&tx, &command(UnsentTerminal::Unclaimed(live), at(1))).await,
            Err(ExecutionError::OperationConflict)
        );
        tx.rollback().await.unwrap();
        cancel(&mut client, 0x12, at(2)).await;
        drop(client);
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.unwrap();
        let rows = scan_unsent_terminal_in(&tx, "live/generation", None, 128, at(3))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0], UnsentTerminal::Unclaimed(_)));
        let c = command(rows[0].clone(), at(3));
        let mut stale = c.clone();
        stale.expected_job_revision -= 1;
        assert_eq!(
            validate_terminal_tail_in(&tx, &stale).await,
            Err(ExecutionError::Stale {
                current_revision: c.expected_job_revision
            })
        );
        let mut foreign = match &rows[0] {
            UnsentTerminal::Unclaimed(p) => p.clone(),
            _ => unreachable!(),
        };
        foreign.scope.owner = draft_owner();
        assert_eq!(
            validate_terminal_tail_in(&tx, &command(UnsentTerminal::Unclaimed(foreign), at(3)))
                .await,
            Err(ExecutionError::NotFound)
        );
        assert_eq!(
            validate_terminal_tail_in(&tx, &c).await.unwrap().allocation,
            TerminalAllocation::NoAllocation
        );
        fixture_seal(&tx, &c).await;
        close_terminal_in(&tx, &c).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 0);
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_dispatch_usage", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        scratch.finish().await;
    });
}
#[test]
fn expiry_without_any_claim_releases_existing_pending_capacity() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let tx = client.transaction().await.unwrap();
        let rows = scan_unsent_terminal_in(&tx, "live/generation", None, 1, at(60_000))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let c = command(rows[0].clone(), at(60_000));
        validate_terminal_tail_in(&tx, &c).await.unwrap();
        fixture_seal(&tx, &c).await;
        close_terminal_in(&tx, &c).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 0);
        scratch.finish().await;
    });
}
#[test]
fn restart_after_claim_before_prepare_uses_actual_stored_claim_and_frees_active_capacity() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        cancel(&mut client, 0x12, at(2)).await;
        drop(client);
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.unwrap();
        let rows = scan_unsent_terminal_in(&tx, "live/generation", None, 1, at(3))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let p = match &rows[0] {
            UnsentTerminal::Claimed(p) => p,
            _ => panic!("actual admitted claim required"),
        };
        assert_eq!(p.claim.token, ClaimToken::from_bytes(&id(0x71)).unwrap());
        assert_eq!(p.claim.generation, 1);
        let c = command(rows[0].clone(), at(3));
        validate_terminal_tail_in(&tx, &c).await.unwrap();
        fixture_seal(&tx, &c).await;
        close_terminal_in(&tx, &c).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 0);
        scratch.finish().await;
    });
}
#[test]
fn cancelled_deferred_stage_uses_exact_marker_and_releases_pending_once() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.unwrap();
        defer_queued_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            at(20_000),
            at(11_000),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        cancel(&mut client, 0x12, at(11_001)).await;
        let tx = client.transaction().await.unwrap();
        let rows = scan_unsent_terminal_in(&tx, "live/generation", None, 1, at(11_002))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let p = match &rows[0] {
            UnsentTerminal::Deferred(p) => p.clone(),
            _ => panic!("actual deferred marker required"),
        };
        assert_eq!(p.not_before_ms, at(20_000));
        assert!(p.claim_generation > 0);
        let mut changed = p.clone();
        changed.claim_generation += 1;
        assert_eq!(
            validate_terminal_tail_in(&tx, &command(UnsentTerminal::Deferred(changed), at(11_002)))
                .await,
            Err(ExecutionError::BasisConflict)
        );
        let c = command(rows[0].clone(), at(11_002));
        validate_terminal_tail_in(&tx, &c).await.unwrap();
        fixture_seal(&tx, &c).await;
        close_terminal_in(&tx, &c).await.unwrap();
        close_terminal_in(&tx, &c).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 0);
        scratch.finish().await;
    });
}

#[test]
fn released_no_dispatch_tail_keeps_parent_unknown_projection_and_uses_real_pending_allocation() {
    runtime().block_on(async{
    use df_persistence::imagery_execution::{read_stage_basis_in,record_candidate_observation_in};
    let scratch=Scratch::open().await;let mut client=connect(&scratch.config()).await;setup_world(&mut client).await;install_seal_schema(&client).await;prepare_decision_stage(&mut client,StageKind::Generation).await;
    let tx=client.transaction().await.unwrap();let basis=read_stage_basis_in(&tx,&owner(),&job_id(0x11),&stage_id(0x21)).await.unwrap();tx.commit().await.unwrap();let bytes=candidate_png(false);let observation=candidate_observation(basis,&bytes,1,1);record_raw_usage(&scratch.config(),&observation.as_receipt().scope,1,Some(bytes),df_imagery::cost::UsageEvidence::Unknown,true).await;
    let tx=client.transaction().await.unwrap();record_candidate_observation_in(&tx,&observation).await.unwrap();complete_dispatch_in(&tx,&finish_command(0x11,0x21,0x81,0x71,1,CompletionOutcome::Candidate{observation_version:1,usage_revision:1},at(5))).await.unwrap();mirror_settlement_in(&tx,&owner(),&job_id(0x11),&stage_id(0x21),&dispatch_id(0x81),DispatchSettlement::UnknownOutstanding,at(6)).await.unwrap();let mut child=enqueue_command(0x11,0x22,at(7));child.kind=StageKind::Inspection;child.parent_dispatch=Some(dispatch_id(0x81));enqueue_stage_in(&tx,&child).await.unwrap();tx.commit().await.unwrap();
    claim(&mut client,0x11,0x22,0x72,0,at(8)).await;let tx=client.transaction().await.unwrap();release_stage_in(&tx,&ReleaseCommand::new(owner(),job_id(0x11),stage_id(0x22),ClaimToken::from_bytes(&id(0x72)).unwrap(),1,at(9)).unwrap()).await.unwrap();tx.commit().await.unwrap();cancel(&mut client,0x12,at(10)).await;
    let tx=client.transaction().await.unwrap();assert!(scan_unsent_terminal_in(&tx,"live/generation",None,128,at(11)).await.unwrap().is_empty(),"Released scope alone is insufficient without an actual pending allocation");tx.rollback().await.unwrap();insert_asset_and_allocation(&mut client,0x41,0x42,0x11,0x21).await;
    let tx=client.transaction().await.unwrap();let rows=scan_unsent_terminal_in(&tx,"live/generation",None,128,at(11)).await.unwrap();assert_eq!(rows.len(),1);let proof=match &rows[0]{UnsentTerminal::Released(p)=>p,_=>panic!("actual released pending-parent proof required")};assert_eq!(proof.candidate_dispatch,dispatch_id(0x81));let c=command(rows[0].clone(),at(11));assert!(matches!(validate_terminal_tail_in(&tx,&c).await.unwrap().allocation,TerminalAllocation::Existing{..}));
    // This execution-boundary fixture installs the real typed seal, not a fabricated Ready result.
    let rev=i64::try_from(c.expected_job_revision).unwrap();tx.execute("INSERT INTO imagery.image_no_delivery(job_id,owner_scope,actor_kind,actor_stage_id,tail_kind,tail_stage_id,candidate_stage_id,candidate_dispatch_id,allocation_kind,allocation_stage_id,asset_id,publication_attempt,expected_job_revision,sealed_job_revision,reason,evidence_binding,decision_binding,candidates_binding,local_inspection_binding,sealed_at_ms) VALUES($1,$2,'released_stage',$3,'released_stage',$3,$4,$5,'existing',$4,$6,$7,$8::bigint,$8::bigint+1::bigint,'retired',$9,'[]','[]','[]',$10)",&[&id(0x11).as_slice(),&df_imagery::owner_scope_bytes(&owner()),&id(0x22).as_slice(),&id(0x21).as_slice(),&id(0x81).as_slice(),&id(0x41).as_slice(),&id(0x42).as_slice(),&rev,&vec![1u8],&at(11)]).await.unwrap();close_terminal_in(&tx,&c).await.unwrap();let parent=read_dispatch_in(&tx,&owner(),&dispatch_id(0x81)).await.unwrap();assert_eq!(parent.state,DispatchState::Succeeded);assert_eq!(parent.settlement,DispatchSettlement::UnknownOutstanding);assert_eq!(parent.usage_revision,1);assert_eq!(tx.query_one("SELECT count(*) FROM imagery.image_job_result",&[]).await.unwrap().get::<_,i64>(0),0);tx.commit().await.unwrap();assert_eq!(capacity_counts(&client,"live/generation").await,(0,0,0));assert_eq!(dispatch_count(&client,0x11,0x22).await,0);scratch.finish().await;
});
}
