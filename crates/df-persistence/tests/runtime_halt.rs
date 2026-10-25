//! The control plane's remedies for the service lock (hosted runtime design 6.1, 6.3):
//! `break_service_lock` and `halt_service` end a hostile or stuck hold, replace the key and leave
//! the epoch alone; prepared transactions are refused. Each test reports SKIPPED without a
//! scratch server.

mod runtime_support;

use runtime_support::*;

/// A runtime that logged in for real learns the key from `pg_locks` and holds the lock.
async fn hostile_holder(
    scratch: &Scratch,
    client: &tokio_postgres::Client,
    token: &str,
) -> tokio_postgres::Client {
    let (hostile, in_flight) = (
        scratch.login_as("df_campaign_runtime").await,
        scratch.connect().await,
    );
    in_flight
        .batch_execute("BEGIN; SELECT df_runtime.enter_service()")
        .await
        .expect("a fenced transaction");
    let learned = as_runtime(
        &hostile,
        token,
        "SELECT classid::oid::int8::int4, objid::oid::int8::int4 FROM pg_locks \
         WHERE locktype = 'advisory' AND objsubid = 2 AND granted LIMIT 1",
        &[],
    )
    .await
    .expect("pg_locks is readable");
    let (high, low): (i32, i32) = (learned[0].get(0), learned[0].get(1));
    assert_eq!((high, low), service_key(client).await);
    in_flight.batch_execute("COMMIT").await.expect("commit");
    hostile
        .batch_execute(&format!(
            "BEGIN; SET LOCAL idle_in_transaction_session_timeout = 0; \
             SET LOCAL statement_timeout = 0; SELECT pg_advisory_xact_lock({high}, {low})"
        ))
        .await
        .expect("the hostile runtime holds the service lock and idles");
    hostile
}

#[test]
fn the_control_plane_breaks_a_hostile_hold_and_replaces_the_key_without_touching_the_epoch() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        let held_key = service_key(&client).await;
        let hostile = hostile_holder(&scratch, &client, &placement.token).await;
        let blocked = tokio::time::timeout(
            SHORT,
            as_role(
                &scratch.connect().await,
                "df_control",
                "SELECT df_runtime.fence_current($1, $2, $3, $4, $5, 'game')",
                &[
                    &table.tenant,
                    &table.campaign,
                    &fence.recovery_epoch,
                    &fence.claim_seq,
                    &placement.id,
                ],
            ),
        )
        .await
        .expect("a fenced call does not wait");
        assert_eq!(
            sqlstate(&blocked.expect_err("DF002 while held")).as_deref(),
            Some("DF002")
        );
        assert_eq!(
            service_state(&client, "df_control").await,
            ("serving".to_string(), 1),
            "no halt is recorded, so the control plane sees DF002 with a serving state"
        );
        as_role(
            &client,
            "df_control",
            "SELECT df_runtime.break_service_lock(200, 2)",
            &[],
        )
        .await
        .expect("the control plane breaks the hold");
        assert!(
            hostile.query("SELECT 1", &[]).await.is_err(),
            "the holder was terminated"
        );
        assert_ne!(service_key(&client).await, held_key, "the key was replaced");
        assert_eq!(
            service_state(&client, "df_control").await,
            ("serving".to_string(), 1),
            "neither the state nor the epoch moves"
        );
        assert!(fence_is_current(&client, &table, fence, &placement.id, "game").await);
        scratch.finish().await;
    });
}

#[test]
fn halting_the_service_records_the_halt_keeps_the_epoch_and_refuses_the_fence() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        let before = service_key(&client).await;
        let halted = as_role(
            &client,
            "df_control",
            "SELECT df_runtime.halt_service(200, 2)",
            &[],
        )
        .await
        .expect("halt")[0]
            .get::<_, bool>(0);
        assert!(halted);
        assert_eq!(
            service_state(&client, "df_control").await,
            ("halted_regression".to_string(), 1)
        );
        assert_ne!(service_key(&client).await, before);
        assert!(!fence_is_current(&client, &table, fence, &placement.id, "game").await);
        let again = as_role(
            &client,
            "df_control",
            "SELECT df_runtime.halt_service(200, 2)",
            &[],
        )
        .await
        .expect("halt again")[0]
            .get::<_, bool>(0);
        assert!(!again, "already halted");
        scratch.finish().await;
    });
}

#[test]
fn a_halt_does_not_override_a_restore_in_progress() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        assert!(begin_restore(&client, 5).await);
        let halted = as_role(
            &client,
            "df_control",
            "SELECT df_runtime.halt_service(200, 2)",
            &[],
        )
        .await
        .expect("halt")[0]
            .get::<_, bool>(0);
        assert!(!halted);
        assert_eq!(
            service_state(&client, "df_control").await,
            ("restoring".to_string(), 5)
        );
        scratch.finish().await;
    });
}

#[test]
fn only_the_control_plane_may_break_the_lock_halt_or_purge_import_jobs() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        for role in [
            "df_campaign_runtime",
            "df_runner",
            "df_egress",
            "df_finance",
            "df_finance_replay",
            "df_importer",
        ] {
            for sql in [
                "SELECT df_runtime.break_service_lock(200, 2)",
                "SELECT df_runtime.halt_service(200, 2)",
                "SELECT df_runtime.purge_import_jobs(0)",
            ] {
                let refused = as_role(&client, role, sql, &[]).await;
                assert_eq!(
                    sqlstate(&refused.expect_err(sql)).as_deref(),
                    Some("42501"),
                    "{role}: {sql}"
                );
            }
        }
        assert_eq!(
            service_state(&client, "df_control").await,
            ("serving".to_string(), 1)
        );
        scratch.finish().await;
    });
}

#[test]
fn prepared_transactions_are_refused_and_every_service_role_can_run_the_check() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        for (maximum, pending) in [(1_i32, 0_i64), (0, 2), (4, 1)] {
            let refused = client
                .query_one(
                    "SELECT df_meta.refuse_prepared_transactions($1, $2)",
                    &[&maximum, &pending],
                )
                .await;
            assert_eq!(
                sqlstate(&refused.expect_err("refused")).as_deref(),
                Some("DF006"),
                "{maximum} {pending}"
            );
        }
        client
            .query_one("SELECT df_meta.refuse_prepared_transactions(0, 0)", &[])
            .await
            .expect("none enabled, none pending");
        for role in [
            "df_campaign_runtime",
            "df_control",
            "df_runner",
            "df_egress",
            "df_finance",
            "df_finance_replay",
        ] {
            as_role(
                &client,
                role,
                "SELECT df_runtime.require_no_prepared_transactions()",
                &[],
            )
            .await
            .unwrap_or_else(|error| {
                panic!("{role} on a server with max_prepared_transactions = 0: {error:?}")
            });
        }
        scratch.finish().await;
    });
}
