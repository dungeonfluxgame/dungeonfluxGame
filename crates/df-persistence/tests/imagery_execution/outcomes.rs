//! Closed operational failure causes preserve unresolved exposure and exact same-dispatch recovery.
use super::terminal_regressions::install_seal_schema;
use super::*;
#[test]
fn every_closed_failure_cause_survives_unknown_restart_without_usage_or_fresh_send() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        let tx = client.transaction().await.unwrap();
        set_capacity_bounds_in(
            &tx,
            "live/generation",
            &CapacityBounds::new(16, 64, 1 << 20).unwrap(),
            at(0),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let causes = [
            FailureCause::Refused,
            FailureCause::InvalidOutput,
            FailureCause::TransportLost,
            FailureCause::RateLimited,
            FailureCause::QuotaExhausted,
            FailureCause::DeadlineExceeded,
            FailureCause::Provider,
        ];
        for (index, cause) in causes.into_iter().enumerate() {
            retain_sent_stage(
                &mut client,
                u8::try_from(index).unwrap(),
                Some(CompletionOutcome::UnknownWithCause { cause }),
            )
            .await;
        }
        drop(client);
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.unwrap();
        for (index, cause) in causes.into_iter().enumerate() {
            let i = u8::try_from(index).unwrap();
            let stored = read_dispatch_in(&tx, &owner(), &dispatch_id(0x80 + i))
                .await
                .unwrap();
            assert_eq!(stored.state, DispatchState::Unknown);
            assert_eq!(stored.settlement, DispatchSettlement::UnknownOutstanding);
            assert_eq!(stored.cause, Some(cause));
            assert_eq!(stored.usage_revision, 0);
            assert_eq!(FailureCause::from_key(cause.key()), Some(cause));
            let replay = complete_dispatch_in(
                &tx,
                &finish_command(
                    0x30 + i,
                    0x40 + i,
                    0x80 + i,
                    0x70 + i,
                    1,
                    CompletionOutcome::UnknownWithCause { cause },
                    at(5),
                ),
            )
            .await
            .unwrap();
            assert_eq!(replay, stored);
        }
        assert_eq!(
            FailureCause::from_key("untrusted arbitrary provider body"),
            None
        );
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client, "live/generation").await, (7, 0, 0));
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_dispatch_usage", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM imagery.image_dispatch_attempt", &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            7
        );
        scratch.finish().await;
    });
}
#[test]
fn unknown_cause_adopts_once_conflicts_then_same_dispatch_known_candidate_clears_cause() {
    runtime().block_on(async {
        use df_persistence::imagery_execution::{
            read_stage_basis_in, record_candidate_observation_in,
        };
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        prepare_decision_stage(&mut client, StageKind::Generation).await;
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
        let recorded = complete_dispatch_in(
            &tx,
            &finish_command(
                0x11,
                0x21,
                0x81,
                0x71,
                1,
                CompletionOutcome::UnknownWithCause {
                    cause: FailureCause::RateLimited,
                },
                at(5),
            ),
        )
        .await
        .unwrap();
        assert_eq!(recorded.cause, Some(FailureCause::RateLimited));
        assert_eq!(recorded.settlement, DispatchSettlement::UnknownOutstanding);
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x71,
                    1,
                    CompletionOutcome::UnknownWithCause {
                        cause: FailureCause::Provider
                    },
                    at(6)
                )
            )
            .await,
            Err(ExecutionError::OperationConflict)
        );
        assert_eq!(
            read_dispatch_in(&tx, &owner(), &dispatch_id(0x81))
                .await
                .unwrap(),
            recorded
        );
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(0x11, 0x21, 0x81, 0x71, 1, CompletionOutcome::Unknown, at(6))
            )
            .await
            .unwrap(),
            recorded
        );
        tx.commit().await.unwrap();
        let bytes = candidate_png(false);
        let observation = candidate_observation(basis, &bytes, 1, 1);
        record_raw_usage(
            &scratch.config(),
            &observation.as_receipt().scope,
            1,
            Some(bytes),
            df_imagery::cost::UsageEvidence::Reported,
            true,
        )
        .await;
        let tx = client.transaction().await.unwrap();
        record_candidate_observation_in(&tx, &observation)
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
                CompletionOutcome::Candidate {
                    observation_version: 1,
                    usage_revision: 1,
                },
                at(7),
            ),
        )
        .await
        .unwrap();
        assert_eq!(known.state, DispatchState::Succeeded);
        assert_eq!(known.cause, None);
        assert_eq!(known.settlement, DispatchSettlement::UnknownOutstanding);
        assert_eq!(
            complete_dispatch_in(
                &tx,
                &finish_command(
                    0x11,
                    0x21,
                    0x81,
                    0x71,
                    1,
                    CompletionOutcome::UnknownWithCause {
                        cause: FailureCause::DeadlineExceeded
                    },
                    at(8)
                )
            )
            .await
            .unwrap(),
            known
        );
        tx.commit().await.unwrap();
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 1);
        scratch.finish().await;
    });
}
#[test]
fn scoped_finance_predicate_refuses_foreign_or_missing_stage_without_financial_select_permission() {
    runtime().block_on(async {
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let tx = client.transaction().await.unwrap();
        tx.batch_execute("SET LOCAL ROLE df_imagery_output")
            .await
            .unwrap();
        let query = "SELECT imagery.stage_has_finance($1,$2,$3)";
        assert_eq!(
            tx.query_one(
                query,
                &[
                    &df_imagery::owner_scope_bytes(&owner()),
                    &id(0x11).as_slice(),
                    &id(0x21).as_slice()
                ]
            )
            .await
            .unwrap()
            .get::<_, Option<bool>>(0),
            Some(false)
        );
        assert_eq!(
            tx.query_one(
                query,
                &[
                    &df_imagery::owner_scope_bytes(&draft_owner()),
                    &id(0x11).as_slice(),
                    &id(0x21).as_slice()
                ]
            )
            .await
            .unwrap()
            .get::<_, Option<bool>>(0),
            None
        );
        assert_eq!(
            tx.query_one(
                query,
                &[
                    &df_imagery::owner_scope_bytes(&owner()),
                    &id(0x11).as_slice(),
                    &id(0x22).as_slice()
                ]
            )
            .await
            .unwrap()
            .get::<_, Option<bool>>(0),
            None
        );
        let denied = tx
            .query("SELECT * FROM imagery.image_finance_admission_binding", &[])
            .await
            .expect_err("no financial table visibility granted");
        assert_eq!(denied.code().map(|code| code.code()), Some("42501"));
        tx.rollback().await.unwrap();
        client
            .batch_execute("DROP TABLE imagery.image_finance_admission_binding")
            .await
            .unwrap();
        assert_eq!(
            client
                .query_one(
                    query,
                    &[
                        &df_imagery::owner_scope_bytes(&owner()),
                        &id(0x11).as_slice(),
                        &id(0x21).as_slice()
                    ]
                )
                .await
                .unwrap()
                .get::<_, Option<bool>>(0),
            Some(false)
        );
        assert_eq!(
            client
                .query_one(
                    query,
                    &[
                        &df_imagery::owner_scope_bytes(&draft_owner()),
                        &id(0x11).as_slice(),
                        &id(0x21).as_slice()
                    ]
                )
                .await
                .unwrap()
                .get::<_, Option<bool>>(0),
            None
        );
        scratch.finish().await;
    });
}

#[test]
fn operational_quota_notice_is_durable_idempotent_and_cleared_on_claim() {
    runtime().block_on(async {
        use df_imagery::execution::{DeferQueuedCommand, QuotaDeferralReason, StageScope};
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        let mut command = DeferQueuedCommand::new(
            StageScope {
                owner: owner(),
                job: job_id(0x11),
                stage: stage_id(0x21),
            },
            at(100),
            Some(QuotaDeferralReason::Cooldown),
            at(1),
        )
        .unwrap();
        let tx = client.transaction().await.unwrap();
        df_persistence::imagery_execution::defer_queued_in(&tx, &command)
            .await
            .unwrap();
        let first = read_stage_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
            .await
            .unwrap();
        assert_eq!(queued_notice(&tx).await.as_deref(), Some("cooldown"));
        command.now_ms = at(2);
        df_persistence::imagery_execution::defer_queued_in(&tx, &command)
            .await
            .unwrap();
        assert_eq!(
            read_stage_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
                .await
                .unwrap(),
            first
        );
        command.notice = Some(QuotaDeferralReason::Probing);
        command.now_ms = at(3);
        df_persistence::imagery_execution::defer_queued_in(&tx, &command)
            .await
            .unwrap();
        assert_eq!(queued_notice(&tx).await.as_deref(), Some("probing"));
        assert!(
            read_stage_in(&tx, &owner(), &job_id(0x11), &stage_id(0x21))
                .await
                .unwrap()
                .job_revision
                > first.job_revision
        );
        command.notice = None;
        command.now_ms = at(4);
        df_persistence::imagery_execution::defer_queued_in(&tx, &command)
            .await
            .unwrap();
        assert_eq!(queued_notice(&tx).await, None);
        command.notice = Some(QuotaDeferralReason::Cooldown);
        command.now_ms = at(5);
        df_persistence::imagery_execution::defer_queued_in(&tx, &command)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        drop(client);
        let mut client = connect(&scratch.config()).await;
        let tx = client.transaction().await.unwrap();
        assert_eq!(queued_notice(&tx).await.as_deref(), Some("cooldown"));
        claim_stage_in(
            &tx,
            &claim_command(0x11, 0x21, GATE_REVISION, 0x71, 0, at(101)),
        )
        .await
        .unwrap();
        assert_eq!(queued_notice(&tx).await, None);
        tx.commit().await.unwrap();
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
async fn queued_notice(tx: &tokio_postgres::Transaction<'_>) -> Option<String> {
    tx.query_one(
        "SELECT queued_notice FROM imagery.image_execution_stage WHERE job_id=$1",
        &[&id(0x11).as_slice()],
    )
    .await
    .unwrap()
    .get(0)
}
#[test]
fn actual47_upgrades_installed_legacy42_actor_checks_and_narrow_output_close() {
    runtime().block_on(async {
        use df_imagery::execution::{TerminalActor, TerminalAuthority, TerminalCommand, UnsentTerminal};
        use df_persistence::imagery_execution::{scan_unsent_terminal_in, validate_terminal_tail_in, close_terminal_in};
        let scratch = Scratch::open().await;
        let mut client = connect(&scratch.config()).await;
        setup_world(&mut client).await;
        install_seal_schema(&client).await;
        accept_and_enqueue(&mut client, 0x10, 0x11, 0x12, 0x21, at(0)).await;
        client.batch_execute(concat!(
            "DO $$ DECLARE item record; BEGIN FOR item IN SELECT conname FROM pg_constraint ",
            "WHERE conrelid='imagery.image_no_delivery'::regclass AND contype='c' ",
            "AND (pg_get_constraintdef(oid) LIKE '%actor_kind%' OR pg_get_constraintdef(oid) LIKE '%tail_kind%') ",
            "LOOP EXECUTE format('ALTER TABLE imagery.image_no_delivery DROP CONSTRAINT %I',item.conname); ",
            "END LOOP; END $$; ALTER TABLE imagery.image_no_delivery ",
            "ADD CONSTRAINT previous42_actor_kind CHECK(actor_kind IN ('dispatch','unsent_stage')),",
            "ADD CONSTRAINT previous42_tail_kind CHECK(tail_kind IN ('dispatch','unsent_stage')),",
            "ADD CONSTRAINT previous42_actor_dispatch CHECK((actor_kind='dispatch' ",
            "AND actor_dispatch_id IS NOT NULL AND actor_usage_revision IS NOT NULL) OR ",
            "(actor_kind='unsent_stage' AND actor_dispatch_id IS NULL AND actor_usage_revision IS NULL)),",
            "ADD CONSTRAINT previous42_tail_dispatch CHECK((tail_kind='dispatch' ",
            "AND tail_dispatch_id IS NOT NULL AND tail_usage_revision IS NOT NULL) OR ",
            "(tail_kind='unsent_stage' AND tail_dispatch_id IS NULL AND tail_usage_revision IS NULL))",
        )).await.unwrap();
        let upgrade = include_str!("../../migrations/0047_imagery_execution_outcomes.sql");
        client.batch_execute(upgrade).await.expect("actual upgrade over previous42 checks");
        client.batch_execute(upgrade).await.expect("exact upgrade replay");
        let tx = client.transaction().await.unwrap();
        df_persistence::withdraw_in(&tx, &owner(), ImageInterestId::from_bytes(&id(0x12)).unwrap(), at(2)).await.unwrap();
        tx.commit().await.unwrap();
        let tx = client.transaction().await.unwrap();
        let rows = scan_unsent_terminal_in(&tx, "live/generation", None, 1, at(3)).await.unwrap();
        let proof = match &rows[0] {
            UnsentTerminal::Unclaimed(proof) => proof.clone(),
            _ => panic!("never-claimed actual stage required"),
        };
        let actor = TerminalActor::UnclaimedStage(proof.scope.clone());
        let command = TerminalCommand {
            actor: actor.clone(), tail: actor, authority: TerminalAuthority::Unclaimed(proof.clone()),
            expected_job_revision: proof.job_revision, candidate_dispatch: None, now_ms: at(3),
        };
        tx.batch_execute("SET LOCAL ROLE df_imagery_output").await.unwrap();
        validate_terminal_tail_in(&tx, &command).await.expect("actual scoped role proof after47");
        tx.batch_execute("RESET ROLE").await.unwrap();
        super::terminal_regressions::fixture_seal(&tx, &command).await;
        tx.batch_execute("SET LOCAL ROLE df_imagery_output").await.unwrap();
        close_terminal_in(&tx, &command).await.expect("upgraded narrow close");
        tx.commit().await.unwrap();
        assert_eq!(capacity_counts(&client, "live/generation").await, (0, 0, 0));
        assert_eq!(dispatch_count(&client, 0x11, 0x21).await, 0);
        scratch.finish().await;
    });
}
