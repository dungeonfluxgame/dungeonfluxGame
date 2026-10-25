//! Forced interleavings of the owner directory, the service lock and the restore (hosted runtime
//! design 6.1-6.3). Every race holds the first session inside an open transaction, waits until
//! the second session is really blocked on a lock (`pg_stat_activity`), and only then lets the
//! first one commit, so the outcome does not depend on timing. Each test reports SKIPPED
//! without a scratch server.

mod runtime_support;

use runtime_support::*;

#[test]
fn two_runners_racing_for_one_table_start_one_owner() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let (first, second) = (placement_for(1), placement_for(2));
        register_worker(&client, &first.worker).await;
        request_placement(&client, &table).await;
        let (holder, racer, observer) = (
            scratch.connect().await,
            scratch.connect().await,
            scratch.connect().await,
        );
        let racer_pid = backend_pid(&racer).await;
        begin_as(&holder, "df_runner", None).await;
        let held = holder
            .query(
                PLACE_SQL,
                &[
                    &table.tenant,
                    &table.campaign,
                    &first.id,
                    &first.worker,
                    &first.token_digest(),
                ],
            )
            .await
            .expect("the first runner places inside its open transaction");
        assert_eq!(held.len(), 1);
        let (racing_table, racing_placement) = (table.clone(), second.clone());
        let racing =
            tokio::spawn(async move { place(&racer, &racing_table, &racing_placement).await });
        wait_until_blocked(&observer, racer_pid).await;
        commit(&holder).await;
        assert!(
            racing.await.expect("racer").is_none(),
            "the second runner waited on the row lock, then found the table placed"
        );
        assert_eq!(
            owner_row(&client, &table).await.placement_id,
            Some(first.id)
        );
        scratch.finish().await;
    });
}

#[test]
fn a_claim_in_flight_makes_a_runner_release_of_the_placing_fence_change_nothing() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let placement = placement_for(1);
        register_worker(&client, &placement.worker).await;
        request_placement(&client, &table).await;
        let placing = place(&client, &table, &placement).await.expect("placed");
        let (claimer, releaser, observer) = (
            scratch.connect().await,
            scratch.connect().await,
            scratch.connect().await,
        );
        let releaser_pid = backend_pid(&releaser).await;
        begin_as(&claimer, "df_campaign_runtime", Some(&placement.token)).await;
        claimer
            .query(
                CLAIM_SQL,
                &[
                    &table.tenant,
                    &table.campaign,
                    &placement.id,
                    &30_i32,
                    &vec![0xee_u8; 16],
                ],
            )
            .await
            .expect("claim inside its open transaction");
        let (release_table, release_id) = (table.clone(), placement.id.clone());
        let release = tokio::spawn(async move {
            as_role(
                &releaser,
                "df_runner",
                "SELECT df_runtime.release_after_exit($1, $2, $3, $4, $5)",
                &[
                    &release_table.tenant,
                    &release_table.campaign,
                    &placing.recovery_epoch,
                    &placing.claim_seq,
                    &release_id,
                ],
            )
            .await
            .expect("release")[0]
                .get::<_, bool>(0)
        });
        wait_until_blocked(&observer, releaser_pid).await;
        commit(&claimer).await;
        assert!(
            !release.await.expect("release task"),
            "the claim bumped the sequence, so the placing fence no longer matches"
        );
        let owner = owner_row(&client, &table).await;
        assert_eq!((owner.state.as_str(), owner.claim_seq), ("serving", 1));
        scratch.finish().await;
    });
}

#[test]
fn a_renewal_in_flight_keeps_a_runner_from_superseding_the_owner_whose_lease_it_extends() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        client
            .execute(
                "UPDATE df_runtime.campaign_owners \
                 SET lease_until = clock_timestamp() + interval '1500 milliseconds'",
                &[],
            )
            .await
            .expect("a lease about to end");
        let (renewer, runner, observer) = (
            scratch.connect().await,
            scratch.connect().await,
            scratch.connect().await,
        );
        let runner_pid = backend_pid(&runner).await;
        begin_as(&renewer, "df_campaign_runtime", Some(&placement.token)).await;
        renewer
            .query(
                RENEW_SQL,
                &[
                    &table.tenant,
                    &table.campaign,
                    &fence.recovery_epoch,
                    &fence.claim_seq,
                    &placement.id,
                    &true,
                    &false,
                ],
            )
            .await
            .expect("renewal inside its open transaction");
        let (racing_table, racing_placement) = (table.clone(), placement_for(2));
        let supersede =
            tokio::spawn(async move { place(&runner, &racing_table, &racing_placement).await });
        wait_until_blocked(&observer, runner_pid).await;
        // The committed lease is over by now, so without the row lock the runner would supersede.
        let ended = async {
            loop {
                let over: bool = observer
                    .query_one(
                        "SELECT lease_until < clock_timestamp() FROM df_runtime.campaign_owners",
                        &[],
                    )
                    .await
                    .expect("lease")
                    .get(0);
                if over {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        };
        tokio::time::timeout(SHORT, ended)
            .await
            .expect("the committed lease runs out");
        commit(&renewer).await;
        assert!(
            supersede.await.expect("runner").is_none(),
            "the extended lease is live, so the table is not up for placement"
        );
        assert_eq!(
            owner_row(&client, &table).await.placement_id,
            Some(placement.id)
        );
        scratch.finish().await;
    });
}

#[test]
fn a_restore_waits_for_an_in_flight_claim_and_then_expires_it() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let placement = placement_for(1);
        register_worker(&client, &placement.worker).await;
        request_placement(&client, &table).await;
        place(&client, &table, &placement).await.expect("placed");
        let (claimer, operator, observer) = (
            scratch.connect().await,
            scratch.connect().await,
            scratch.connect().await,
        );
        let operator_pid = backend_pid(&operator).await;
        begin_as(&claimer, "df_campaign_runtime", Some(&placement.token)).await;
        claimer
            .query(
                CLAIM_SQL,
                &[
                    &table.tenant,
                    &table.campaign,
                    &placement.id,
                    &30_i32,
                    &vec![0xee_u8; 16],
                ],
            )
            .await
            .expect("claim inside its open transaction");
        let restore = tokio::spawn(async move {
            operator
                .query_one("SELECT df_runtime.begin_restore(5, 5000, 3)", &[])
                .await
                .expect("restore")
                .get::<_, bool>(0)
        });
        wait_until_blocked(&observer, operator_pid).await;
        commit(&claimer).await;
        assert!(restore.await.expect("operator task"));
        let owner = owner_row(&client, &table).await;
        assert_eq!(owner.claim_seq, 1, "the claim finished before the restore");
        let expired: bool = client
            .query_one(
                "SELECT lease_until <= clock_timestamp() FROM df_runtime.campaign_owners",
                &[],
            )
            .await
            .expect("lease")
            .get(0);
        assert!(
            expired,
            "the restore ended the lease the claim had just taken"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_restore_expires_every_lease_below_the_new_epoch_and_forces_reconstruction() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let placement = placement_for(1);
        register_worker(&client, &placement.worker).await;
        request_placement(&client, &table).await;
        place(&client, &table, &placement).await.expect("placed");
        add_game_session(&client, &table, &[0x6a; 16]).await;
        let fence = claim(&client, &table, &placement)
            .await
            .expect("claimed")
            .fence;
        assert!(begin_restore(&client, 5).await);
        assert!(
            restore_complete(&client, "df_finance_replay", 5)
                .await
                .expect("complete")
        );
        assert!(
            renew(&client, &table, &placement, fence, true, false)
                .await
                .is_none(),
            "the old owner cannot renew its way past the restore"
        );
        assert!(!fence_is_current(&client, &table, fence, &placement.id, "paid").await);
        assert!(!fence_is_current(&client, &table, fence, &placement.id, "game").await);
        assert!(!owner_row(&client, &table).await.released_cleanly);
        let leases_over: bool = client
            .query_one(
                "SELECT (SELECT lease_until <= clock_timestamp() FROM df_game.sessions) \
                 AND (SELECT lease_until <= clock_timestamp() FROM df_runtime.placement_secrets)",
                &[],
            )
            .await
            .expect("leases")
            .get(0);
        assert!(leases_over, "the projected and the secret leases ended too");
        let next = placement_for(2);
        place(&client, &table, &next).await.expect("superseded");
        let claimed = claim(&client, &table, &next).await.expect("claimed");
        assert_eq!(claimed.fence.recovery_epoch, 5);
        assert_eq!(claimed.money_state, "reconstructing");
        scratch.finish().await;
    });
}

#[test]
fn a_restore_changes_nothing_for_an_owner_at_the_new_epoch() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        assert!(begin_restore(&client, 5).await);
        assert!(
            restore_complete(&client, "df_finance_replay", 5)
                .await
                .expect("complete")
        );
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        assert_eq!(fence.recovery_epoch, 5);
        assert!(
            !begin_restore(&client, 5).await,
            "the same epoch again is no restore"
        );
        assert!(
            renew(&client, &table, &placement, fence, true, false)
                .await
                .is_some()
        );
        scratch.finish().await;
    });
}

#[test]
fn a_runtime_cannot_read_the_service_lock_key_from_the_state_table() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (_, placement, _) = serving_table(&client, 1, 1).await;
        let refused = as_runtime(
            &client,
            &placement.token,
            "SELECT lock_key_high FROM df_meta.service_state",
            &[],
        )
        .await;
        assert_eq!(
            sqlstate(&refused.expect_err("refused")).as_deref(),
            Some("42501")
        );
        scratch.finish().await;
    });
}

#[test]
fn a_runtime_that_learns_the_key_and_holds_the_service_lock_cannot_block_the_halt() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        let (hostile, in_flight, caller) = (
            scratch.login_as("df_campaign_runtime").await,
            scratch.connect().await,
            scratch.connect().await,
        );
        // pg_locks shows the advisory key of any held lock to every role: wait for a legitimate
        // fenced transaction, read the key from it as the runtime, then take the lock for good.
        in_flight
            .batch_execute("BEGIN; SELECT df_runtime.enter_service()")
            .await
            .expect("a fenced transaction");
        let learned = as_runtime(
            &hostile,
            &placement.token,
            "SELECT classid::oid::int8::int4, objid::oid::int8::int4 FROM pg_locks \
             WHERE locktype = 'advisory' AND objsubid = 2 AND granted LIMIT 1",
            &[],
        )
        .await
        .expect("pg_locks is readable");
        let (high, low): (i32, i32) = (learned[0].get(0), learned[0].get(1));
        assert_eq!(
            (high, low),
            service_key(&client).await,
            "the key leaks through pg_locks"
        );
        in_flight.batch_execute("COMMIT").await.expect("commit");
        hostile
            .batch_execute(&format!(
                "BEGIN; SET LOCAL ROLE df_campaign_runtime; \
                 SET LOCAL idle_in_transaction_session_timeout = 0; \
                 SET LOCAL statement_timeout = 0; \
                 SELECT pg_advisory_xact_lock({high}, {low})"
            ))
            .await
            .expect("the hostile runtime holds the service lock and idles");
        let blocked = tokio::time::timeout(
            SHORT,
            as_role(
                &caller,
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
            sqlstate(&blocked.expect_err("service halting")).as_deref(),
            Some("DF002")
        );
        assert!(
            client
                .query_one("SELECT df_runtime.begin_restore(5, 200, 2)", &[])
                .await
                .expect("the restore terminates the holder and proceeds")
                .get::<_, bool>(0)
        );
        assert!(
            hostile.query("SELECT 1", &[]).await.is_err(),
            "the holder's backend was terminated"
        );
        assert_ne!(
            service_key(&client).await,
            (high, low),
            "the restore replaced the key"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_registered_session_without_a_game_row_does_not_wedge_claims_and_renewals() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let placement = placement_for(1);
        register_worker(&client, &placement.worker).await;
        request_placement(&client, &table).await;
        place(&client, &table, &placement).await.expect("placed");
        let registered = as_runtime(
            &client,
            &placement.token,
            "SELECT df_runtime.register_game_session($1)",
            &[&vec![0x6a_u8; 16]],
        )
        .await
        .expect("register");
        assert!(registered[0].get::<_, bool>(0));
        let fence = claim(&client, &table, &placement)
            .await
            .expect("the claim succeeds although the session has no game row yet")
            .fence;
        assert!(
            renew(&client, &table, &placement, fence, true, false)
                .await
                .is_some()
        );
        scratch.finish().await;
    });
}

#[test]
fn the_placement_token_and_the_role_do_not_outlive_their_transaction() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let placement = placement_for(1);
        as_runtime(&client, &placement.token, "SELECT 1", &[])
            .await
            .expect("a call with a token");
        let after: Option<String> = client
            .query_one("SELECT current_setting('df.placement_token', true)", &[])
            .await
            .expect("setting")
            .get(0);
        assert!(after.as_deref().is_none_or(str::is_empty), "{after:?}");
        let role: String = client
            .query_one("SELECT current_user::text", &[])
            .await
            .expect("role")
            .get(0);
        assert_ne!(role, "df_campaign_runtime");
        scratch.finish().await;
    });
}
