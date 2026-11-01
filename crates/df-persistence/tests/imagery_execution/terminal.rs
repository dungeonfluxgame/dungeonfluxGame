//! Execution-boundary checks use the real immutable publication seal schema.
//! Seal inserts below are administrator fixtures; semantic rejection and catalog cleanup are
//! independently owned by the publication suite, and no finance ledger release is asserted.
use super::*;
use df_imagery::execution::{
    ExecutionScope, StageClaim, TerminalActor, TerminalAllocation, TerminalAuthority,
    TerminalCommand,
};
use df_persistence::imagery_execution::{
    advance_execution_gate_in, close_terminal_in, validate_terminal_tail_in,
};

pub(super) async fn install_seal_schema(client: &Client) {
    client
        .batch_execute("SELECT pg_advisory_lock(hashtext('df_imagery_test_role_migrations'))")
        .await
        .expect("role lock");
    client.batch_execute("CREATE TABLE IF NOT EXISTS media.schema_migration(version integer PRIMARY KEY); INSERT INTO media.schema_migration(version) SELECT generate_series(10,14) ON CONFLICT DO NOTHING").await.expect("actual media migration history");
    client
        .batch_execute(include_str!(
            "../../migrations/0042_imagery_publication.sql"
        ))
        .await
        .expect("real publication schema");
    client
        .batch_execute("SELECT pg_advisory_unlock(hashtext('df_imagery_test_role_migrations'))")
        .await
        .expect("role unlock");
}
fn stage_claim(job: u8, stage: u8, token: u8) -> StageClaim {
    StageClaim {
        scope: df_imagery::execution::StageScope {
            owner: owner(),
            job: job_id(job),
            stage: stage_id(stage),
        },
        token: ClaimToken::from_bytes(&id(token)).expect("claim"),
        generation: 1,
    }
}
pub(super) async fn revision(client: &Client, job: u8) -> u64 {
    u64::try_from(
        client
            .query_one(
                "SELECT revision FROM imagery.image_job WHERE job_id=$1",
                &[&id(job).as_slice()],
            )
            .await
            .expect("revision")
            .get::<_, i64>(0),
    )
    .expect("positive revision")
}
fn dispatch_terminal(
    job: u8,
    stage: u8,
    dispatch: u8,
    token: u8,
    revision: u64,
    now: i64,
) -> TerminalCommand {
    let actor = TerminalActor::Dispatch(ExecutionScope {
        owner: owner(),
        job: job_id(job),
        stage: stage_id(stage),
        dispatch: dispatch_id(dispatch),
    });
    TerminalCommand {
        actor: actor.clone(),
        tail: actor,
        authority: TerminalAuthority::Claim(stage_claim(job, stage, token)),
        expected_job_revision: revision,
        candidate_dispatch: None,
        now_ms: now,
    }
}
pub(super) async fn fixture_seal(tx: &tokio_postgres::Transaction<'_>, command: &TerminalCommand) {
    let scope = command.tail.scope();
    let (kind, dispatch, usage) = match &command.tail {
        TerminalActor::Dispatch(s) => {
            ("dispatch", Some(s.dispatch.as_bytes().to_vec()), Some(0i32))
        }
        TerminalActor::UnsentStage(_) => ("unsent_stage", None, None),
        TerminalActor::UnclaimedStage(_) => ("unclaimed_stage", None, None),
        TerminalActor::DeferredStage(_) => ("deferred_stage", None, None),
        TerminalActor::ReleasedStage(_) => ("released_stage", None, None),
    };
    let revision = i64::try_from(command.expected_job_revision).expect("revision");
    tx.execute("INSERT INTO imagery.image_no_delivery(job_id,owner_scope,actor_kind,actor_stage_id,actor_dispatch_id,actor_usage_revision,tail_kind,tail_stage_id,tail_dispatch_id,tail_usage_revision,allocation_kind,expected_job_revision,sealed_job_revision,reason,evidence_binding,decision_binding,candidates_binding,local_inspection_binding,sealed_at_ms) VALUES($1,$2,$3,$4,$5,$6,$3,$4,$5,$6,'no_allocation',$7::bigint,$7::bigint+1::bigint,'expired',$8,'[]','[]','[]',$9)",
        &[&scope.job.as_bytes().as_slice(),&df_imagery::owner_scope_bytes(&scope.owner),&kind,&scope.stage.as_bytes().as_slice(),&dispatch,&usage,&revision,&vec![1u8],&command.now_ms]).await.expect("boundary fixture seal");
}

#[test]
fn terminal_completed_dispatch_uses_actual_original_deadline_and_rejects_stale_or_expired_claim() {
    runtime().block_on(async{
    let scratch=Scratch::open().await;let mut client=connect(&scratch.config()).await;setup_world(&mut client).await;
    retain_sent_stage(&mut client,0,Some(CompletionOutcome::Failed{cause:FailureCause::InvalidOutput})).await;
    let released = client.transaction().await.unwrap();
    release_stage_in(&released, &ReleaseCommand::new(owner(),job_id(0x30),stage_id(0x40),ClaimToken::from_bytes(&id(0x70)).unwrap(),1,at(5)).unwrap()).await.expect("explicit terminal stage release");
    released.commit().await.unwrap();
    let rev=revision(&client,0x30).await;
    let tx=client.transaction().await.expect("terminal tx");
    let original:i64=tx.query_one("SELECT claim_lease_until_ms FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",&[&id(0x80).as_slice()]).await.expect("persisted deadline").get(0);
    assert_eq!(original,at(1)+LEASE_MS);
    let command=dispatch_terminal(0x30,0x40,0x80,0x70,rev,at(6));
    let proof=validate_terminal_tail_in(&tx,&command).await.expect("released terminal original live lease");
    assert_eq!(proof.tail.stage.state,StageState::Released);assert_eq!(proof.allocation,TerminalAllocation::NoAllocation);
    assert_eq!(validate_terminal_tail_in(&tx,&TerminalCommand{now_ms:original,..command.clone()}).await,Err(ExecutionError::LostClaim));
    assert_eq!(validate_terminal_tail_in(&tx,&TerminalCommand{expected_job_revision:rev-1,..command}).await,Err(ExecutionError::Stale{current_revision:rev}));
    tx.rollback().await.expect("readonly proof");scratch.finish().await;
});
}

#[test]
fn terminal_expired_unsent_claim_releases_capacity_exactly_once_and_never_creates_dispatch() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let rev = revision(&client, 0x11).await;
        let c = stage_claim(0x11, 0x21, 0x71);
        let command = TerminalCommand {
            actor: TerminalActor::UnsentStage(c.clone()),
            tail: TerminalActor::UnsentStage(c.clone()),
            authority: TerminalAuthority::Claim(c),
            expected_job_revision: rev,
            candidate_dispatch: None,
            now_ms: at(60_000),
        };
        let tx = client.transaction().await.expect("expired cleanup");
        assert_eq!(
            validate_terminal_tail_in(&tx, &command)
                .await
                .expect("exact expired no-dispatch claim")
                .allocation,
            TerminalAllocation::NoAllocation
        );
        fixture_seal(&tx, &command).await;
        assert_eq!(
            close_terminal_in(&tx, &command).await.expect("close"),
            rev + 1
        );
        assert_eq!(
            close_terminal_in(&tx, &command)
                .await
                .expect("exact close replay"),
            rev + 1
        );
        tx.commit().await.expect("atomic close");
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 0);
        let row = client
            .query_one(
                "SELECT state,revision FROM imagery.image_job WHERE job_id=$1",
                &[&id(0x11).as_slice()],
            )
            .await
            .expect("retired job");
        assert_eq!(row.get::<_, String>(0), "retired");
        assert_eq!(row.get::<_, i64>(1), i64::try_from(rev + 1).unwrap());
        scratch.finish().await;
    });
}

#[test]
fn terminal_prepared_hosted_tail_frees_execution_capacity_and_preserves_financial_hold() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        claim(&mut client, 0x11, 0x21, 0x71, 0, at(1)).await;
        let tx = client.transaction().await.expect("prepare");
        let prepared = prepare_dispatch_in(
            &tx,
            &prepare_command(0x11, 0x21, 0x81, 0x71, 1, hosted_mode(0x81), at(2)),
        )
        .await
        .expect("prepared");
        tx.commit().await.expect("prepare commit");
        let command =
            dispatch_terminal(0x11, 0x21, 0x81, 0x71, revision(&client, 0x11).await, at(3));
        let tx = client.transaction().await.expect("close prepared");
        validate_terminal_tail_in(&tx, &command)
            .await
            .expect("current prepared tail");
        fixture_seal(&tx, &command).await;
        close_terminal_in(&tx, &command)
            .await
            .expect("close prepared");
        let closed = read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
            .await
            .expect("closed dispatch");
        assert_eq!(closed.state, DispatchState::Released);
        assert_eq!(closed.settlement, DispatchSettlement::Held);
        assert_eq!(closed.finance, prepared.finance);
        tx.commit().await.expect("seal commit");
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        let tx = client.transaction().await.expect("sealed egress");
        assert_eq!(
            df_persistence::imagery_execution::check_output_open_in(&tx, &owner(), &job_id(0x11))
                .await,
            Err(ExecutionError::OutputSealed)
        );
        tx.rollback().await.expect("no mutation");
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_dispatch_usage", &[])
                .await
                .expect("no fabricated usage")
                .get::<_, i64>(0),
            0
        );
        scratch.finish().await;
    });
}

#[test]
fn startup_gate_compare_and_set_preserves_monotonic_revision_across_restart_race() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.expect("startup");
        let first = advance_execution_gate_in(&tx, "mount/start", 0, true, at(0))
            .await
            .expect("first startup");
        assert_eq!(first.applied_revision, 1);
        tx.commit().await.expect("first");
        let tx = client.transaction().await.expect("restart");
        assert_eq!(
            advance_execution_gate_in(&tx, "mount/start", 1, true, at(1))
                .await
                .expect("same-enabled restart")
                .applied_revision,
            2
        );
        tx.commit().await.expect("second");
        let tx = client.transaction().await.expect("stale competing mount");
        assert_eq!(
            advance_execution_gate_in(&tx, "mount/start", 1, false, at(2)).await,
            Err(ExecutionError::ConfigFenced {
                current_revision: 2
            })
        );
        assert_eq!(
            read_execution_gate_in(&tx, "mount/start")
                .await
                .expect("unchanged")
                .applied_revision,
            2
        );
        tx.commit().await.expect("stale readonly");
        scratch.finish().await;
    });
}

#[test]
fn stored_job_digests_backfill_from_actual_snapshots_and_cannot_be_caller_rewritten() {
    runtime().block_on(async{
    let scratch=Scratch::open().await;let mut client=connect(&scratch.config()).await;setup_world(&mut client).await;
    client.batch_execute("ALTER TABLE imagery.image_job DROP COLUMN intent_digest, DROP COLUMN config_digest").await.expect("simulate existing pre-metadata schema");
    accept_and_enqueue(&mut client,0x10,0x11,0x12,0x21,at(0)).await;
    client.batch_execute(MIGRATION_0040).await.expect("metadata additive backfill");
    let row=client.query_one("SELECT intent_digest=pg_catalog.sha256(intent_bytes),config_digest=pg_catalog.sha256(config_bytes) FROM imagery.image_job WHERE job_id=$1",&[&id(0x11).as_slice()]).await.expect("actual backfilled hashes");assert!(row.get::<_,bool>(0));assert!(row.get::<_,bool>(1));
    assert_eq!(client.query_one("SELECT count(*) FROM pg_attribute WHERE attrelid='imagery.image_job'::regclass AND attname IN ('intent_digest','config_digest') AND attgenerated='s'",&[]).await.expect("generated columns").get::<_,i64>(0),2);
    let error=client.execute("UPDATE imagery.image_job SET intent_digest=$1 WHERE job_id=$2",&[&vec![0u8;32],&id(0x11).as_slice()]).await.expect_err("caller digest write forbidden");assert_eq!(error.code().map(|c|c.code()),Some("428C9"));scratch.finish().await;
});
}

#[test]
fn terminal_never_acquired_planned_sentinel_is_not_cleanup_authority() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let never = StageClaim {
            scope: df_imagery::execution::StageScope {
                owner: owner(),
                job: job_id(0x11),
                stage: stage_id(0x21),
            },
            token: ClaimToken::from_bytes(&id(0x21)).unwrap(),
            generation: 0,
        };
        let command = TerminalCommand {
            actor: TerminalActor::UnsentStage(never.clone()),
            tail: TerminalActor::UnsentStage(never.clone()),
            authority: TerminalAuthority::Claim(never),
            expected_job_revision: revision(&client, 0x11).await,
            candidate_dispatch: None,
            now_ms: at(60_000),
        };
        let tx = client.transaction().await.unwrap();
        assert_eq!(
            validate_terminal_tail_in(&tx, &command).await,
            Err(ExecutionError::LostClaim)
        );
        tx.rollback().await.unwrap();
        assert_eq!(
            capacity_counts(&client, "live/generation").await,
            (0, 1, 128)
        );
        scratch.finish().await;
    });
}

#[test]
fn candidate_metadata_snapshot_reader_does_not_acquire_dispatch_lock_under_owned_job_lock() {
    runtime().block_on(async{
    let scratch=Scratch::open().await;let mut client=connect(&scratch.config()).await;setup_world(&mut client).await;let receipt=retain_candidate(&mut client,&scratch.config()).await;
    let mut second=connect(&scratch.config()).await;let a=client.transaction().await.unwrap();a.batch_execute("SET LOCAL statement_timeout='500ms'").await.unwrap();read_stage_in(&a,&owner(),&job_id(0x11),&stage_id(0x21)).await.unwrap();
    let b=second.transaction().await.unwrap();b.query_one("SELECT dispatch_id FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1 FOR UPDATE",&[&id(0x81).as_slice()]).await.unwrap();
    assert_eq!(df_persistence::imagery_execution::read_candidate_reference_in(&a,&owner(),&job_id(0x11),&stage_id(0x21),&dispatch_id(0x81)).await.expect("metadata read does not wait for dispatch lock"),Some(receipt));
    b.rollback().await.unwrap();a.rollback().await.unwrap();scratch.finish().await;
});
}

#[test]
fn sealed_unknown_retains_liability_then_accepts_late_same_dispatch_decision_and_financial_projection()
 {
    runtime().block_on(async {
        use df_imagery::execution::{FinishRecoveryCommand, RecoveryDisposition};
        use df_persistence::imagery_execution::{
            claim_recovery_in, finish_recovery_in, read_stage_basis_in,
            record_decision_observation_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        prepare_decision_stage(&mut client, StageKind::Classification).await;
        let tx = client.transaction().await.unwrap();
        complete_dispatch_in(
            &tx,
            &finish_command(0x11, 0x21, 0x81, 0x71, 1, CompletionOutcome::Unknown, at(4)),
        )
        .await
        .unwrap();
        let basis = read_stage_basis_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let rev = revision(&client, 0x11).await;
        let tx = client.transaction().await.unwrap();
        let lease = claim_recovery_in(&tx, &recovery_command(0x11, 0x21, 0x81, 0x91, at(60_000)))
            .await
            .unwrap();
        let claimed_revision = read_stage_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .unwrap()
            .job_revision;
        assert!(claimed_revision > rev);
        let mut command = dispatch_terminal(0x11, 0x21, 0x81, 0x71, claimed_revision, at(60_001));
        command.authority = TerminalAuthority::Recovery(lease.clone());
        validate_terminal_tail_in(&tx, &command)
            .await
            .expect("paid Unknown tail may seal output");
        fixture_seal(&tx, &command).await;
        close_terminal_in(&tx, &command).await.unwrap();
        let unknown = read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
            .await
            .unwrap();
        assert_eq!(unknown.state, DispatchState::Unknown);
        assert_eq!(unknown.settlement, DispatchSettlement::UnknownOutstanding);
        let mut ack = FinishRecoveryCommand {
            owner: owner(),
            job: job_id(0x11),
            stage: stage_id(0x21),
            dispatch: dispatch_id(0x81),
            claim: lease.token,
            claim_generation: lease.generation,
            disposition: RecoveryDisposition::Completed,
            now_ms: at(60_002),
        };
        assert_eq!(
            finish_recovery_in(&tx, &ack).await,
            Err(ExecutionError::OperationConflict)
        );
        tx.commit().await.unwrap();
        let observation = observed_decision(
            basis,
            br#"{"route":"general","schema_revision":1}"#.to_vec(),
            1,
        );
        record_decision_usage(&scratch.config(), &observation).await;
        let tx = client.transaction().await.unwrap();
        record_decision_observation_in(&tx, &observation)
            .await
            .unwrap();
        let known = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::Decision {
                    observation_version: 1,
                    usage_revision: 1,
                },
                at(60_003),
            ),
        )
        .await
        .expect("late correlated outcome retained privately");
        assert_eq!(known.state, DispatchState::Succeeded);
        assert_eq!(known.settlement, DispatchSettlement::UnknownOutstanding);
        mirror_settlement_in(
            &tx,
            &owner(),
            &job_id(0x11),
            &stage_id(0x21),
            &dispatch_id(0x81),
            DispatchSettlement::Settled,
            at(60_004),
        )
        .await
        .expect("finance-owned mirror remains open after seal");
        ack.now_ms = at(60_005);
        finish_recovery_in(&tx, &ack)
            .await
            .expect("actual terminal projection and scoped seal permit ack");
        assert_eq!(
            df_persistence::imagery_execution::check_output_open_in(&tx, &owner(), &job_id(0x11))
                .await,
            Err(ExecutionError::OutputSealed)
        );
        assert_eq!(
            tx.query_one("SELECT count(*) FROM imagery.image_job_result", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(
            tx.query_one("SELECT count(*) FROM media.asset", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        tx.commit().await.unwrap();
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}

#[test]
fn historical_prepared_discovery_never_uses_age_as_release_authority_and_retries_after_seal_crash()
{
    runtime().block_on(async {
        use df_persistence::imagery_execution::scan_unsent_prepared_in;
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        for index in 0..2u8 {
            accept_and_enqueue(
                &mut client,
                0x10 + index,
                0x30 + index,
                0x50 + index,
                0x40 + index,
                at(0),
            )
            .await;
            claim(
                &mut client,
                0x30 + index,
                0x40 + index,
                0x70 + index,
                0,
                at(1),
            )
            .await;
            let tx = client.transaction().await.unwrap();
            prepare_dispatch_in(
                &tx,
                &prepare_command(
                    0x30 + index,
                    0x40 + index,
                    0x80 + index,
                    0x70 + index,
                    1,
                    hosted_mode(0x80 + index),
                    at(2),
                ),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        let live_revision = revision(&client, 0x30).await;
        let tx = client.transaction().await.unwrap();
        assert!(
            scan_unsent_prepared_in(&tx, "live/generation", None, 10, at(3))
                .await
                .unwrap()
                .is_empty()
        );
        let page = scan_unsent_prepared_in(&tx, "live/generation", None, 1, at(11_000))
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].scope.dispatch, dispatch_id(0x80));
        assert!(!page[0].terminal_condition);
        assert!(!page[0].sealed);
        assert_eq!(page[0].current.state, DispatchState::Prepared);
        assert_eq!(page[0].current.settlement, DispatchSettlement::Held);
        assert_eq!(
            validate_terminal_tail_in(
                &tx,
                &dispatch_terminal(0x30, 0x40, 0x80, 0x70, live_revision, at(11_000))
            )
            .await,
            Err(ExecutionError::LostClaim)
        );
        let next = scan_unsent_prepared_in(
            &tx,
            "live/generation",
            Some(page[0].scope.dispatch),
            1,
            at(11_000),
        )
        .await
        .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].scope.dispatch, dispatch_id(0x81));
        assert_eq!(
            scan_unsent_prepared_in(&tx, "live/generation", None, 129, at(11_000)).await,
            Err(ExecutionError::InvalidRecord)
        );
        tx.commit().await.unwrap();
        client
            .execute(
                "UPDATE imagery.image_job SET state='retired' WHERE job_id=$1",
                &[&id(0x31).as_slice()],
            )
            .await
            .unwrap();
        let command = dispatch_terminal(
            0x31,
            0x41,
            0x81,
            0x71,
            revision(&client, 0x31).await,
            at(11_001),
        );
        let tx = client.transaction().await.unwrap();
        validate_terminal_tail_in(&tx, &command)
            .await
            .expect("actual retired original/current Prepared authority");
        fixture_seal(&tx, &command).await;
        close_terminal_in(&tx, &command).await.unwrap();
        tx.commit().await.unwrap();
        drop(client);
        // Crash before any finance-owned release: the exact sealed Held tail remains discoverable.
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.unwrap();
        let retry = scan_unsent_prepared_in(
            &tx,
            "live/generation",
            Some(dispatch_id(0x80)),
            1,
            at(11_002),
        )
        .await
        .unwrap();
        assert_eq!(retry.len(), 1);
        assert!(retry[0].terminal_condition);
        assert!(retry[0].sealed);
        assert_eq!(retry[0].current.state, DispatchState::Released);
        assert_eq!(retry[0].current.settlement, DispatchSettlement::Held);
        assert_eq!(retry[0].current.finance, hosted_mode(0x81).finance());
        mirror_settlement_in(
            &tx,
            &owner(),
            &job_id(0x31),
            &stage_id(0x41),
            &dispatch_id(0x81),
            DispatchSettlement::Released,
            at(11_003),
        )
        .await
        .expect("trusted component mirror; funded adapter verifies actual financial cancellation");
        tx.commit().await.unwrap();
        let tx = client.transaction().await.unwrap();
        let pending_book = scan_unsent_prepared_in(
            &tx,
            "live/generation",
            Some(dispatch_id(0x80)),
            1,
            at(11_004),
        )
        .await
        .unwrap();
        assert_eq!(pending_book.len(), 1);
        assert_eq!(pending_book[0].scope.dispatch, dispatch_id(0x81));
        assert_eq!(pending_book[0].current.state, DispatchState::Released);
        assert_eq!(
            pending_book[0].current.settlement,
            DispatchSettlement::Released
        );
        assert!(pending_book[0].sealed);
        assert!(
            !tx.query_one(
                "SELECT cost_unsent_ack FROM imagery.image_dispatch_attempt WHERE dispatch_id=$1",
                &[&id(0x81).as_slice()]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
        );
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client, "live/generation").await, (1, 0, 0));
        assert_eq!(dispatch_count(&client, 0x30, 0x40).await, 1);
        assert_eq!(dispatch_count(&client, 0x31, 0x41).await, 1);
        scratch.finish().await;
    });
}
