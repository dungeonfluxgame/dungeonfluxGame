//! The owner directory and its definer functions on a scratch PostgreSQL (hosted runtime design
//! 6.1-6.3): placement, claim, renewal with segment rolls, release, the fence, the service halt
//! and money reconstruction. Each test reports SKIPPED without a scratch server.

mod runtime_support;

use runtime_support::*;
use tokio_postgres::Client;

const REPLACED_FUNDING_COVERS_AN_HOUR: &str = "CREATE OR REPLACE FUNCTION df_runtime.meter_funded_until(\
    p_tenant_id bytea, p_campaign_id bytea, p_session_id bytea) RETURNS timestamptz \
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, df_runtime, pg_temp \
    AS $$ SELECT clock_timestamp() + interval '1 hour' $$";

const REPLACED_FUNDING_FAILS: &str = "CREATE OR REPLACE FUNCTION df_runtime.meter_funded_until(\
    p_tenant_id bytea, p_campaign_id bytea, p_session_id bytea) RETURNS timestamptz \
    LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, df_runtime, pg_temp \
    AS $$ BEGIN RAISE EXCEPTION 'funding unavailable'; END $$";

async fn count(client: &Client, sql: &str, table: &Table) -> i64 {
    client
        .query_one(sql, &[&table.tenant, &table.campaign])
        .await
        .expect("count")
        .get(0)
}

async fn open_session_with_segment(
    client: &Client,
    table: &Table,
    opened_seconds_ago: i32,
) -> Vec<u8> {
    let session = vec![0x5e; 16];
    client
        .execute(
            "INSERT INTO df_runtime.table_sessions (tenant_id, campaign_id, session_id, phase, created_at) \
             VALUES ($1, $2, $3, 'active', clock_timestamp())",
            &[&table.tenant, &table.campaign, &session],
        )
        .await
        .expect("table session");
    client
        .execute(
            "INSERT INTO df_runtime.meter_segments (tenant_id, session_id, opened_at) \
             VALUES ($1, $2, clock_timestamp() - make_interval(secs => $3))",
            &[&table.tenant, &session, &f64::from(opened_seconds_ago)],
        )
        .await
        .expect("segment");
    session
}

#[test]
fn a_placement_request_creates_one_released_owner_row_and_repeating_it_changes_nothing() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        request_placement(&client, &table).await;
        request_placement(&client, &table).await;
        let owner = owner_row(&client, &table).await;
        assert_eq!(
            (owner.state.as_str(), owner.recovery_epoch, owner.claim_seq),
            ("released", 1, 0)
        );
        assert_eq!(
            count(&client, "SELECT count(*) FROM df_runtime.placement_requests WHERE tenant_id = $1 AND campaign_id = $2", &table).await,
            1
        );
        scratch.finish().await;
    });
}

#[test]
fn placing_a_released_table_records_the_placement_and_a_second_runner_changes_nothing() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let (first, second) = (placement_for(1), placement_for(2));
        register_worker(&client, &first.worker).await;
        request_placement(&client, &table).await;
        assert_eq!(
            place(&client, &table, &first).await,
            Some(Fence {
                recovery_epoch: 1,
                claim_seq: 0
            })
        );
        assert!(place(&client, &table, &second).await.is_none());
        let owner = owner_row(&client, &table).await;
        assert_eq!(owner.state, "placing");
        assert_eq!(owner.placement_id, Some(first.id.clone()));
        assert_eq!(
            count(&client, "SELECT count(*) FROM df_runtime.placement_requests WHERE tenant_id = $1 AND campaign_id = $2", &table).await,
            0,
            "a placed request is consumed"
        );
        scratch.finish().await;
    });
}

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
        let (client_a, client_b) = (scratch.connect().await, scratch.connect().await);
        let (table_a, table_b) = (table.clone(), table.clone());
        let (placement_a, placement_b) = (first.clone(), second.clone());
        let racer_a = tokio::spawn(async move { place(&client_a, &table_a, &placement_a).await });
        let racer_b = tokio::spawn(async move { place(&client_b, &table_b, &placement_b).await });
        let (won_a, won_b) = (
            racer_a.await.expect("racer a"),
            racer_b.await.expect("racer b"),
        );
        assert_eq!(
            [won_a.is_some(), won_b.is_some()]
                .iter()
                .filter(|won| **won)
                .count(),
            1
        );
        let live = count(
            &client,
            "SELECT count(*) FROM df_runtime.placements WHERE tenant_id = $1 AND campaign_id = $2 AND ended_at IS NULL",
            &table,
        )
        .await;
        assert_eq!(live, 1);
        scratch.finish().await;
    });
}

#[test]
fn a_claim_needs_the_placements_own_token_and_works_once() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let (first, other) = (placement_for(1), placement_for(2));
        register_worker(&client, &first.worker).await;
        request_placement(&client, &table).await;
        place(&client, &table, &first).await.expect("placed");
        assert!(
            claim_with(&client, &table, &first, &other.token, 30, &[0xee; 16])
                .await
                .is_none()
        );
        let claimed = claim(&client, &table, &first).await.expect("claimed");
        assert_eq!(
            claimed.fence,
            Fence {
                recovery_epoch: 1,
                claim_seq: 1
            }
        );
        assert_eq!(claimed.money_state, "reconciled");
        assert!(claimed.reconstruction_id.is_none());
        assert!(
            claim(&client, &table, &first).await.is_none(),
            "a serving owner cannot claim again"
        );
        scratch.finish().await;
    });
}

#[test]
fn each_claim_strictly_increases_the_claim_sequence() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let first = placement_for(1);
        register_worker(&client, &first.worker).await;
        request_placement(&client, &table).await;
        let mut sequences = Vec::new();
        for tag in 1..=3 {
            let next = placement_for(tag);
            if tag > 1 {
                expire_leases(&client, &table).await;
            }
            place(&client, &table, &next).await.expect("placed");
            sequences.push(
                claim(&client, &table, &next)
                    .await
                    .expect("claimed")
                    .fence
                    .claim_seq,
            );
        }
        assert_eq!(sequences, [1, 2, 3]);
        scratch.finish().await;
    });
}

#[test]
fn a_live_serving_owner_cannot_be_superseded() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, first, _) = serving_table(&client, 1, 1).await;
        assert!(place(&client, &table, &placement_for(2)).await.is_none());
        assert_eq!(
            owner_row(&client, &table).await.placement_id,
            Some(first.id)
        );
        scratch.finish().await;
    });
}

#[test]
fn a_superseded_owner_can_neither_renew_nor_release_nor_pass_the_fence() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, old, old_fence) = serving_table(&client, 1, 1).await;
        expire_leases(&client, &table).await;
        let new = placement_for(2);
        place(&client, &table, &new).await.expect("superseded");
        let new_fence = claim(&client, &table, &new).await.expect("claimed").fence;
        assert!(
            renew(&client, &table, &old, old_fence, true, false)
                .await
                .is_none()
        );
        let released = as_runtime(
            &client,
            &old.token,
            "SELECT df_runtime.release_clean($1, $2, $3, $4, $5)",
            &[
                &table.tenant,
                &table.campaign,
                &old_fence.recovery_epoch,
                &old_fence.claim_seq,
                &old.id,
            ],
        )
        .await
        .expect("release");
        assert!(!released[0].get::<_, bool>(0));
        assert!(!fence_is_current(&client, &table, old_fence, &old.id, "game").await);
        assert!(fence_is_current(&client, &table, new_fence, &new.id, "game").await);
        scratch.finish().await;
    });
}

#[test]
fn the_fence_needs_all_three_parts_and_paid_work_needs_a_reconciled_state() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        assert!(fence_is_current(&client, &table, fence, &placement.id, "game").await);
        assert!(fence_is_current(&client, &table, fence, &placement.id, "paid").await);
        let wrong_epoch = Fence {
            recovery_epoch: fence.recovery_epoch + 1,
            ..fence
        };
        let wrong_sequence = Fence {
            claim_seq: fence.claim_seq + 1,
            ..fence
        };
        assert!(!fence_is_current(&client, &table, wrong_epoch, &placement.id, "game").await);
        assert!(!fence_is_current(&client, &table, wrong_sequence, &placement.id, "game").await);
        assert!(!fence_is_current(&client, &table, fence, &[0x99; 16], "game").await);
        client
            .execute(
                "UPDATE df_runtime.campaign_owners SET money_state = 'reconstructing', reconstruction_id = $3 \
                 WHERE tenant_id = $1 AND campaign_id = $2",
                &[&table.tenant, &table.campaign, &vec![0xee_u8; 16]],
            )
            .await
            .expect("reconstructing");
        assert!(fence_is_current(&client, &table, fence, &placement.id, "game").await);
        assert!(!fence_is_current(&client, &table, fence, &placement.id, "paid").await);
        expire_leases(&client, &table).await;
        assert!(!fence_is_current(&client, &table, fence, &placement.id, "game").await);
        scratch.finish().await;
    });
}

#[test]
fn a_halted_service_refuses_every_directory_function() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        client
            .execute(
                "UPDATE df_meta.service_state SET serving = 'halted_regression'",
                &[],
            )
            .await
            .expect("halt");
        assert!(!fence_is_current(&client, &table, fence, &placement.id, "game").await);
        assert!(
            renew(&client, &table, &placement, fence, true, false)
                .await
                .is_none()
        );
        expire_leases(&client, &table).await;
        assert!(place(&client, &table, &placement_for(2)).await.is_none());
        scratch.finish().await;
    });
}

#[test]
fn a_fenced_call_during_a_held_halt_raises_service_halting_without_waiting() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let (halter, caller) = (scratch.connect().await, scratch.connect().await);
        halter
            .batch_execute(&format!("BEGIN; {}", halt_sql(&caller).await))
            .await
            .expect("halt holds the lock");
        let outcome = tokio::time::timeout(
            SHORT,
            as_role(
                &caller,
                "df_control",
                "SELECT df_runtime.fence_current($1, $2, 1, 1, $3, 'game')",
                &[&vec![0xa1_u8; 16], &vec![0xc1_u8; 16], &vec![1_u8; 16]],
            ),
        )
        .await
        .expect("a fenced call must not wait for a halt");
        assert_eq!(
            sqlstate(&outcome.expect_err("refused")).as_deref(),
            Some("DF002")
        );
        halter
            .batch_execute("ROLLBACK")
            .await
            .expect("release the halt");
        scratch.finish().await;
    });
}

#[test]
fn a_fenced_call_queued_behind_a_waiting_halt_raises_service_halting_without_waiting() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let (holder, caller, observer) = (
            scratch.connect().await,
            scratch.connect().await,
            scratch.connect().await,
        );
        let halter = scratch.connect().await;
        let halt_statement = halt_sql(&observer).await;
        holder
            .batch_execute("BEGIN; SELECT df_runtime.enter_service()")
            .await
            .expect("in-flight fenced transaction");
        let halt = tokio::spawn(async move {
            halter
                .batch_execute(&format!("BEGIN; {halt_statement}; COMMIT"))
                .await
        });
        let queued = async {
            loop {
                let waiting: i64 = observer
                    .query_one(
                        "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted",
                        &[],
                    )
                    .await
                    .expect("pg_locks")
                    .get(0);
                if waiting > 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(SHORT, queued)
            .await
            .expect("the halt queues behind the shared holder");
        let outcome = tokio::time::timeout(
            SHORT,
            as_role(
                &caller,
                "df_control",
                "SELECT df_runtime.fence_current($1, $2, 1, 1, $3, 'game')",
                &[&vec![0xa1_u8; 16], &vec![0xc1_u8; 16], &vec![1_u8; 16]],
            ),
        )
        .await
        .expect("a fenced call must not queue behind a waiting halt");
        assert_eq!(
            sqlstate(&outcome.expect_err("refused")).as_deref(),
            Some("DF002")
        );
        holder
            .batch_execute("COMMIT")
            .await
            .expect("fenced transaction finishes");
        tokio::time::timeout(SHORT, halt)
            .await
            .expect("the halt proceeds once the shared holder is gone")
            .expect("halt task")
            .expect("halt");
        scratch.finish().await;
    });
}

#[test]
fn a_halt_with_a_lock_timeout_gives_up_while_a_fenced_transaction_is_in_flight() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let (holder, halter) = (scratch.connect().await, scratch.connect().await);
        let halt = halt_sql(&holder).await;
        holder
            .batch_execute("BEGIN; SELECT df_runtime.enter_service()")
            .await
            .expect("in-flight fenced transaction");
        halter
            .batch_execute("BEGIN; SET LOCAL lock_timeout = '200ms'")
            .await
            .expect("halt transaction");
        let attempt = halter.batch_execute(&halt).await;
        assert_eq!(
            sqlstate(&attempt.expect_err("timed out")).as_deref(),
            Some("55P03")
        );
        halter.batch_execute("ROLLBACK").await.expect("rollback");
        holder.batch_execute("COMMIT").await.expect("commit");
        halter
            .batch_execute(&format!("BEGIN; {halt}; COMMIT"))
            .await
            .expect("the retry holds the lock once nothing is in flight");
        scratch.finish().await;
    });
}

#[test]
fn claim_and_renew_project_the_fence_and_lease_into_the_game_session_and_a_missing_row_raises() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let table = table_for(1);
        let placement = placement_for(1);
        let game_session = vec![0x6a_u8; 16];
        register_worker(&client, &placement.worker).await;
        request_placement(&client, &table).await;
        place(&client, &table, &placement).await.expect("placed");
        let registered = as_runtime(
            &client,
            &placement.token,
            "SELECT df_runtime.register_game_session($1)",
            &[&game_session],
        )
        .await
        .expect("register");
        assert!(registered[0].get::<_, bool>(0));
        client
            .execute(
                "INSERT INTO df_game.sessions (tenant_id, session_id, run_id, recovery_epoch, in_epoch_sequence, owner_fence, lease_until) \
                 VALUES ($1::bytea, $2::bytea, $3::bytea, 1, 0, $4::bytea, clock_timestamp() - interval '1 hour')",
                &[&table.tenant, &game_session, &vec![0x01_u8; 16], &vec![0x02_u8; 16]],
            )
            .await
            .expect("game session");
        let fence = claim(&client, &table, &placement)
            .await
            .expect("claimed")
            .fence;
        let projected: bool = client
            .query_one(
                "SELECT owner_fence = df_runtime.fence_digest($3, $4, $5) AND lease_until > clock_timestamp() \
                 FROM df_game.sessions WHERE tenant_id = $1::bytea AND session_id = $2::bytea",
                &[&table.tenant, &game_session, &fence.recovery_epoch, &fence.claim_seq, &placement.id],
            )
            .await
            .expect("projection")
            .get(0);
        assert!(projected, "the claim projects the fence and a live lease");
        client
            .execute(
                "UPDATE df_game.sessions SET lease_until = clock_timestamp() + interval '1 second'",
                &[],
            )
            .await
            .expect("shorten the projected lease");
        renew(&client, &table, &placement, fence, true, false)
            .await
            .expect("renewed");
        let extended: bool = client
            .query_one("SELECT lease_until > clock_timestamp() + interval '20 seconds' FROM df_game.sessions", &[])
            .await
            .expect("lease")
            .get(0);
        assert!(extended, "the renewal extends the projected lease");
        client
            .execute(
                "UPDATE df_game.sessions SET owner_fence = '\\x0000000000000000000000000000ffff'",
                &[],
            )
            .await
            .expect("a projection that no longer carries the owner fence");
        let lease_before: f64 = client
            .query_one(
                "SELECT extract(epoch FROM lease_until)::float8 FROM df_runtime.campaign_owners",
                &[],
            )
            .await
            .expect("lease")
            .get(0);
        let failed = as_runtime(
            &client,
            &placement.token,
            "SELECT * FROM df_runtime.renew($1, $2, $3, $4, $5, 30, true, false, 60)",
            &[
                &table.tenant,
                &table.campaign,
                &fence.recovery_epoch,
                &fence.claim_seq,
                &placement.id,
            ],
        )
        .await;
        assert_eq!(
            sqlstate(&failed.expect_err("mismatch")).as_deref(),
            Some("DF004")
        );
        let lease_after: f64 = client
            .query_one(
                "SELECT extract(epoch FROM lease_until)::float8 FROM df_runtime.campaign_owners",
                &[],
            )
            .await
            .expect("lease")
            .get(0);
        assert_eq!(
            lease_before, lease_after,
            "a failed projection renews nothing"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_session_registered_to_another_campaign_is_refused() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let first_table = table_for(1);
        let second_table = Table {
            tenant: first_table.tenant.clone(),
            campaign: vec![0xd1; 16],
        };
        let (_, first, _) = serving_table_of(&client, &first_table, 1).await;
        let (_, second, _) = serving_table_of(&client, &second_table, 2).await;
        let shared = vec![0x6b_u8; 16];
        let sql = "SELECT df_runtime.register_game_session($1)";
        let owned: bool = as_runtime(&client, &first.token, sql, &[&shared])
            .await
            .expect("first")[0]
            .get(0);
        let stolen: bool = as_runtime(&client, &second.token, sql, &[&shared])
            .await
            .expect("second")[0]
            .get(0);
        assert!(owned && !stolen);
        scratch.finish().await;
    });
}

#[test]
fn a_crash_without_a_release_makes_the_next_claim_reconstruct_money() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, _, _) = serving_table(&client, 1, 1).await;
        expire_leases(&client, &table).await;
        let next = placement_for(2);
        place(&client, &table, &next).await.expect("placed");
        let claimed = claim(&client, &table, &next).await.expect("claimed");
        assert_eq!(claimed.money_state, "reconstructing");
        assert_eq!(claimed.reconstruction_id, Some(vec![0xee; 16]));
        scratch.finish().await;
    });
}

#[test]
fn a_clean_release_lets_the_next_claim_stay_reconciled() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        let released = as_runtime(
            &client,
            &placement.token,
            "SELECT df_runtime.release_clean($1, $2, $3, $4, $5)",
            &[
                &table.tenant,
                &table.campaign,
                &fence.recovery_epoch,
                &fence.claim_seq,
                &placement.id,
            ],
        )
        .await
        .expect("release");
        assert!(released[0].get::<_, bool>(0));
        let owner_after = owner_row(&client, &table).await;
        assert_eq!(
            (owner_after.state.as_str(), owner_after.released_cleanly),
            ("released", true)
        );
        assert!(owner_after.placement_id.is_none());
        let next = placement_for(2);
        place(&client, &table, &next).await.expect("placed");
        let claimed = claim(&client, &table, &next).await.expect("claimed");
        assert_eq!(claimed.fence.claim_seq, 2);
        assert_eq!(claimed.money_state, "reconciled");
        assert!(claimed.reconstruction_id.is_none());
        scratch.finish().await;
    });
}

#[test]
fn a_runner_release_after_an_exit_is_unclean_and_a_never_claimed_placement_keeps_the_flag() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        let release_after_exit = |fence: Fence, id: Vec<u8>, table: Table| {
            let client = &client;
            async move {
                as_role(
                    client,
                    "df_runner",
                    "SELECT df_runtime.release_after_exit($1, $2, $3, $4, $5)",
                    &[
                        &table.tenant,
                        &table.campaign,
                        &fence.recovery_epoch,
                        &fence.claim_seq,
                        &id,
                    ],
                )
                .await
                .expect("release after exit")[0]
                    .get::<_, bool>(0)
            }
        };
        assert!(release_after_exit(fence, placement.id.clone(), table.clone()).await);
        assert!(!owner_row(&client, &table).await.released_cleanly);
        let never_claimed = placement_for(2);
        let placed = place(&client, &table, &never_claimed)
            .await
            .expect("placed");
        assert!(release_after_exit(placed, never_claimed.id.clone(), table.clone()).await);
        let kept = owner_row(&client, &table).await;
        assert_eq!(kept.state, "released");
        assert!(
            !kept.released_cleanly,
            "an unclean predecessor stays unclean"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_release_closes_the_open_segment_at_the_last_proven_progress() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        open_session_with_segment(&client, &table, 120).await;
        as_runtime(
            &client,
            &placement.token,
            "SELECT df_runtime.release_clean($1, $2, $3, $4, $5)",
            &[
                &table.tenant,
                &table.campaign,
                &fence.recovery_epoch,
                &fence.claim_seq,
                &placement.id,
            ],
        )
        .await
        .expect("release");
        let closed: bool = client
            .query_one(
                "SELECT s.closed_at = o.last_progress_at AND s.close_reason = 'release' \
                 FROM df_runtime.meter_segments s, df_runtime.campaign_owners o",
                &[],
            )
            .await
            .expect("segment")
            .get(0);
        assert!(closed);
        scratch.finish().await;
    });
}

#[test]
fn a_new_placement_closes_the_lost_owners_open_segment_at_its_last_progress() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, _, _) = serving_table(&client, 1, 1).await;
        open_session_with_segment(&client, &table, 120).await;
        expire_leases(&client, &table).await;
        place(&client, &table, &placement_for(2))
            .await
            .expect("placed");
        let closed: bool = client
            .query_one(
                "SELECT s.closed_at = o.last_progress_at AND s.close_reason = 'owner_lost' \
                 FROM df_runtime.meter_segments s, df_runtime.campaign_owners o",
                &[],
            )
            .await
            .expect("segment")
            .get(0);
        assert!(closed);
        scratch.finish().await;
    });
}

#[test]
fn a_rolling_renewal_closes_at_the_last_progress_and_opens_the_next_segment_at_the_same_instant() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        open_session_with_segment(&client, &table, 120).await;
        client
            .batch_execute(REPLACED_FUNDING_COVERS_AN_HOUR)
            .await
            .expect("funding");
        let renewed = renew(&client, &table, &placement, fence, false, true)
            .await
            .expect("renewed");
        assert!(renewed.rolled && !renewed.roll_failed && !renewed.funding_required);
        let rows = client
            .query(
                "SELECT closed_at = (SELECT last_progress_at FROM df_runtime.campaign_owners), \
                 closed_at IS NULL FROM df_runtime.meter_segments ORDER BY segment_id",
                &[],
            )
            .await
            .expect("segments");
        assert_eq!(rows.len(), 2);
        assert!(
            rows[0].get::<_, bool>(0),
            "closed at the last proven progress, not at the renewal time"
        );
        assert!(rows[1].get::<_, bool>(1), "the next segment is open");
        let touching: bool = client
            .query_one(
                "SELECT (SELECT closed_at FROM df_runtime.meter_segments WHERE closed_at IS NOT NULL) \
                      = (SELECT opened_at FROM df_runtime.meter_segments WHERE closed_at IS NULL)",
                &[],
            )
            .await
            .expect("touching")
            .get(0);
        assert!(touching);
        scratch.finish().await;
    });
}

#[test]
fn a_segment_opened_after_the_last_progress_rolls_closed_and_empty() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        open_session_with_segment(&client, &table, 0).await;
        client
            .batch_execute(REPLACED_FUNDING_COVERS_AN_HOUR)
            .await
            .expect("funding");
        renew(&client, &table, &placement, fence, false, true)
            .await
            .expect("renewed");
        let empty: bool = client
            .query_one(
                "SELECT closed_at = opened_at FROM df_runtime.meter_segments ORDER BY segment_id LIMIT 1",
                &[],
            )
            .await
            .expect("segment")
            .get(0);
        assert!(empty);
        scratch.finish().await;
    });
}

#[test]
fn a_roll_without_funding_closes_the_segment_opens_none_and_asks_for_funding() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        open_session_with_segment(&client, &table, 120).await;
        let renewed = renew(&client, &table, &placement, fence, true, true)
            .await
            .expect("renewed");
        assert!(!renewed.rolled && !renewed.roll_failed && renewed.funding_required);
        assert_eq!(open_segments(&client).await, 0);
        scratch.finish().await;
    });
}

#[test]
fn an_error_in_the_roll_loses_only_the_roll_and_never_the_lease_renewal() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        open_session_with_segment(&client, &table, 120).await;
        client
            .batch_execute(REPLACED_FUNDING_FAILS)
            .await
            .expect("failing funding");
        let lease_before: f64 = client
            .query_one(
                "SELECT extract(epoch FROM lease_until)::float8 FROM df_runtime.campaign_owners",
                &[],
            )
            .await
            .expect("lease")
            .get(0);
        let renewed = renew(&client, &table, &placement, fence, true, true)
            .await
            .expect("renewed");
        assert!(renewed.roll_failed && !renewed.rolled && !renewed.funding_required);
        assert!(
            renewed.lease_until_epoch_s >= lease_before,
            "the lease renewal still happened"
        );
        let segments = client
            .query_one("SELECT count(*), count(*) FILTER (WHERE closed_at IS NULL) FROM df_runtime.meter_segments", &[])
            .await
            .expect("segments");
        assert_eq!(
            (segments.get::<_, i64>(0), segments.get::<_, i64>(1)),
            (1, 1),
            "the sub-block rolled back the close"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_renewal_without_a_roll_leaves_the_segments_alone() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        open_session_with_segment(&client, &table, 120).await;
        let renewed = renew(&client, &table, &placement, fence, true, false)
            .await
            .expect("renewed");
        assert!(!renewed.rolled && !renewed.roll_failed && !renewed.funding_required);
        assert_eq!(open_segments(&client).await, 1);
        scratch.finish().await;
    });
}

async fn reconstructing_table(client: &Client) -> (Table, Placement, Fence) {
    let (table, _, _) = serving_table(client, 1, 1).await;
    expire_leases(client, &table).await;
    let next = placement_for(2);
    place(client, &table, &next).await.expect("placed");
    let claimed = claim(client, &table, &next).await.expect("claimed");
    assert_eq!(claimed.money_state, "reconstructing");
    (table, next, claimed.fence)
}

async fn record_money_state(
    client: &Client,
    role: &str,
    table: &Table,
    id: &[u8],
    outcome: &str,
) -> Result<bool, tokio_postgres::Error> {
    as_role(
        client,
        role,
        "SELECT df_runtime.record_money_state($1, $2, $3, $4)",
        &[&table.tenant, &table.campaign, &id, &outcome],
    )
    .await
    .map(|rows| rows[0].get(0))
}

#[test]
fn a_reconstruction_result_reconciles_once_and_a_stale_id_changes_nothing() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, _, _) = reconstructing_table(&client).await;
        assert!(
            !record_money_state(&client, "df_finance", &table, &[0x11; 16], "reconstructed")
                .await
                .expect("stale")
        );
        assert_eq!(
            owner_row(&client, &table).await.money_state,
            "reconstructing"
        );
        assert!(
            record_money_state(&client, "df_finance", &table, &[0xee; 16], "reconstructed")
                .await
                .expect("current")
        );
        assert_eq!(owner_row(&client, &table).await.money_state, "reconciled");
        assert!(
            !record_money_state(&client, "df_finance", &table, &[0xee; 16], "failed")
                .await
                .expect("late failure")
        );
        assert_eq!(owner_row(&client, &table).await.money_state, "reconciled");
        scratch.finish().await;
    });
}

#[test]
fn a_failed_reconstruction_freezes_and_a_later_success_for_the_same_id_lifts_the_freeze() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, _, _) = reconstructing_table(&client).await;
        assert!(
            record_money_state(&client, "df_finance", &table, &[0xee; 16], "failed")
                .await
                .expect("failed")
        );
        assert_eq!(owner_row(&client, &table).await.money_state, "frozen");
        assert!(
            record_money_state(&client, "df_finance", &table, &[0xee; 16], "reconstructed")
                .await
                .expect("lifted")
        );
        assert_eq!(owner_row(&client, &table).await.money_state, "reconciled");
        scratch.finish().await;
    });
}

#[test]
fn an_unknown_reconstruction_outcome_is_refused() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, _, _) = reconstructing_table(&client).await;
        let refused =
            record_money_state(&client, "df_finance", &table, &[0xee; 16], "perhaps").await;
        assert_eq!(
            sqlstate(&refused.expect_err("refused")).as_deref(),
            Some("22023")
        );
        scratch.finish().await;
    });
}

#[test]
fn the_runtime_role_cannot_execute_record_money_state() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, _) = reconstructing_table(&client).await;
        let refused = as_runtime(
            &client,
            &placement.token,
            "SELECT df_runtime.record_money_state($1, $2, $3, 'reconstructed')",
            &[&table.tenant, &table.campaign, &vec![0xee_u8; 16]],
        )
        .await;
        assert_eq!(
            sqlstate(&refused.expect_err("refused")).as_deref(),
            Some("42501")
        );
        for role in ["df_control", "df_runner", "df_egress"] {
            let refused =
                record_money_state(&client, role, &table, &[0xee; 16], "reconstructed").await;
            assert_eq!(
                sqlstate(&refused.expect_err("refused")).as_deref(),
                Some("42501"),
                "{role}"
            );
        }
        assert_eq!(
            owner_row(&client, &table).await.money_state,
            "reconstructing"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_deadline_freezes_only_the_current_reconstruction_and_never_reaches_reconciled() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, _) = reconstructing_table(&client).await;
        let deadline = |token: String, id: Vec<u8>, table: Table| {
            let client = &client;
            async move {
                as_runtime(
                    client,
                    &token,
                    "SELECT df_runtime.record_reconstruction_deadline($1, $2, $3)",
                    &[&table.tenant, &table.campaign, &id],
                )
                .await
                .expect("deadline")[0]
                    .get::<_, bool>(0)
            }
        };
        assert!(
            !deadline(placement.token.clone(), vec![0x11; 16], table.clone()).await,
            "another id"
        );
        assert!(deadline(placement.token.clone(), vec![0xee; 16], table.clone()).await);
        assert_eq!(owner_row(&client, &table).await.money_state, "frozen");
        assert!(
            !deadline(placement.token.clone(), vec![0xee; 16], table.clone()).await,
            "already frozen"
        );
        scratch.finish().await;
    });
}

#[test]
fn a_late_deadline_never_overwrites_a_committed_reconstruction() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, _) = reconstructing_table(&client).await;
        assert!(
            record_money_state(&client, "df_finance", &table, &[0xee; 16], "reconstructed")
                .await
                .expect("committed")
        );
        let late = as_runtime(
            &client,
            &placement.token,
            "SELECT df_runtime.record_reconstruction_deadline($1, $2, $3)",
            &[&table.tenant, &table.campaign, &vec![0xee_u8; 16]],
        )
        .await
        .expect("deadline");
        assert!(!late[0].get::<_, bool>(0));
        assert_eq!(owner_row(&client, &table).await.money_state, "reconciled");
        scratch.finish().await;
    });
}

#[test]
fn a_deadline_with_a_superseded_placement_token_changes_nothing() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, old, _) = reconstructing_table(&client).await;
        expire_leases(&client, &table).await;
        let newer = placement_for(3);
        place(&client, &table, &newer).await.expect("superseded");
        let stale = as_runtime(
            &client,
            &old.token,
            "SELECT df_runtime.record_reconstruction_deadline($1, $2, $3)",
            &[&table.tenant, &table.campaign, &vec![0xee_u8; 16]],
        )
        .await
        .expect("deadline");
        assert!(!stale[0].get::<_, bool>(0));
        assert_eq!(
            owner_row(&client, &table).await.money_state,
            "reconstructing"
        );
        scratch.finish().await;
    });
}

async fn open_segments(client: &Client) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM df_runtime.meter_segments WHERE closed_at IS NULL",
            &[],
        )
        .await
        .expect("open segments")
        .get(0)
}

#[test]
fn a_restore_raises_the_epoch_only_upward_and_leaves_the_service_restoring() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        assert_eq!(
            service_state(&client, "df_finance").await,
            ("serving".to_string(), 1)
        );
        assert!(
            !begin_restore(&client, 1).await,
            "the same epoch is not a restore"
        );
        assert!(begin_restore(&client, 5).await);
        assert_eq!(
            service_state(&client, "df_finance").await,
            ("restoring".to_string(), 5)
        );
        assert!(
            !begin_restore(&client, 4).await,
            "a lower epoch never replaces a higher one"
        );
        assert_eq!(
            service_state(&client, "df_control").await,
            ("restoring".to_string(), 5)
        );
        scratch.finish().await;
    });
}

#[test]
fn nothing_places_claims_renews_or_passes_a_fence_while_the_service_is_restoring() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, placement, fence) = serving_table(&client, 1, 1).await;
        assert!(begin_restore(&client, 5).await);
        assert!(!fence_is_current(&client, &table, fence, &placement.id, "game").await);
        assert!(!fence_is_current(&client, &table, fence, &placement.id, "paid").await);
        assert!(
            renew(&client, &table, &placement, fence, true, false)
                .await
                .is_none()
        );
        expire_leases(&client, &table).await;
        let next = placement_for(2);
        assert!(place(&client, &table, &next).await.is_none());
        scratch.finish().await;
    });
}

#[test]
fn a_claim_after_a_completed_restore_runs_at_the_raised_epoch_and_reconstructs_money() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        let (table, _, _) = serving_table(&client, 1, 1).await;
        assert!(begin_restore(&client, 5).await);
        assert!(
            restore_complete(&client, "df_finance_replay", 5)
                .await
                .expect("complete")
        );
        expire_leases(&client, &table).await;
        let next = placement_for(2);
        place(&client, &table, &next).await.expect("placed");
        let claimed = claim(&client, &table, &next).await.expect("claimed");
        assert_eq!(claimed.fence.recovery_epoch, 5);
        assert_eq!(claimed.money_state, "reconstructing");
        scratch.finish().await;
    });
}

#[test]
fn only_the_replay_role_completes_a_restore_and_only_at_the_restoring_epoch() {
    run(async {
        let Some(scratch) = Scratch::open().await else {
            return;
        };
        let client = scratch.connect().await;
        assert!(
            !restore_complete(&client, "df_finance_replay", 1)
                .await
                .expect("not restoring")
        );
        assert!(begin_restore(&client, 5).await);
        for role in [
            "df_finance",
            "df_control",
            "df_runner",
            "df_egress",
            "df_campaign_runtime",
        ] {
            let refused = restore_complete(&client, role, 5).await;
            assert_eq!(
                sqlstate(&refused.expect_err("refused")).as_deref(),
                Some("42501"),
                "{role}"
            );
        }
        assert!(
            !restore_complete(&client, "df_finance_replay", 4)
                .await
                .expect("wrong epoch")
        );
        assert_eq!(service_state(&client, "df_finance").await.0, "restoring");
        assert!(
            restore_complete(&client, "df_finance_replay", 5)
                .await
                .expect("complete")
        );
        assert_eq!(service_state(&client, "df_finance").await.0, "serving");
        assert!(
            !restore_complete(&client, "df_finance_replay", 5)
                .await
                .expect("again"),
            "completing twice changes nothing"
        );
        scratch.finish().await;
    });
}
