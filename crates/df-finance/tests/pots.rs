#![cfg(not(target_arch = "wasm32"))]

//! Funding epochs and in-flight holds (planning/master-launch-plan.md 5.3-5.5).

mod support;

use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::UsageSource;
use df_finance::service::FinanceError;
use support::attack::service_client;
use support::invariants;
use support::world::{World, operation, payer, purchase};

const INPUT_UNITS: u64 = 400_000;
const OUTPUT_UNITS: u64 = 100_000;

fn actual_units() -> Vec<(String, u64)> {
    vec![
        ("text_input".into(), INPUT_UNITS),
        ("text_output".into(), OUTPUT_UNITS),
    ]
}

async fn wait_for_waiters(world: &World, minimum: i64) {
    for _ in 0..10_000 {
        let waiters: i64 = world
            .admin
            .query_one(
                "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
             WHERE NOT l.granted AND a.datname = current_database()",
                &[],
            )
            .await
            .expect("pg_locks")
            .get(0);
        if waiters >= minimum {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("finance flows did not wait on PostgreSQL locks");
}

#[test]
fn pot_02_withdrawal_waits_for_admission_and_preserves_the_existing_hold() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut setup = world.service().await;
        setup
            .apply_purchase(purchase(payer(1), "pot_02", 5_000_000))
            .await
            .expect("purchase");
        setup
            .allocate(
                payer(1),
                world.session,
                operation(20),
                3_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let admitted = world.admit_command(20, INPUT_UNITS, OUTPUT_UNITS);
        setup.admit(&admitted).await.expect("initial hold");

        let holder = service_client(&world).await;
        holder
            .batch_execute("BEGIN")
            .await
            .expect("begin lock holder");
        holder.query_one(
            "SELECT finance.lock_flow('admission', $1::text::jsonb)::text",
            &[&serde_json::json!({"session": world.session.hex(), "principal": world.campaign.hex(),
                "operation": operation(21).hex(), "now_ms": support::attack::now_ms()}).to_string()],
        ).await.expect("hold session lock");

        let withdrawing = world.service().await;
        let session = world.session;
        let withdrawal_task = tokio::spawn(async move {
            let mut service = withdrawing;
            service
                .withdraw_allocation(payer(1), session, operation(23))
                .await
        });
        wait_for_waiters(&world, 1).await;
        let mut admitting = world.service().await;
        let command = world.admit_command(22, INPUT_UNITS, OUTPUT_UNITS);
        let admission_task = tokio::spawn(async move { admitting.admit(&command).await });
        wait_for_waiters(&world, 2).await;
        holder
            .batch_execute("ROLLBACK")
            .await
            .expect("release session lock");
        assert!(
            withdrawal_task
                .await
                .expect("withdrawal task")
                .expect("withdrawal commits")
                .is_some()
        );
        let admission = admission_task.await.expect("admission task");
        assert!(matches!(admission, Err(FinanceError::Admission(_))));
        assert_eq!(world.natural(2130, None, None, None).await, 135_000);
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            4_865_000
        );
        let existing_share: i64 = world
            .admin
            .query_one(
                "SELECT held_micro_dollars - charged_micro_dollars - released_micro_dollars \
             FROM finance.reservation_share WHERE reservation_id = \
             (SELECT reservation_id FROM finance.provider_attempt WHERE attempt_id = $1)",
                &[&admitted.attempt.as_bytes().to_vec()],
            )
            .await
            .expect("existing share")
            .get(0);
        assert_eq!(existing_share, 135_000);
        world.finish().await;
    });
}

#[test]
fn pot_03_host_ceiling_cannot_exceed_a_payers_approved_session_ceiling() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "pot_03", 5_000_000))
            .await
            .expect("purchase");
        world
            .seed("UPDATE finance.session_funding SET session_ceiling_micro_dollars = 50000")
            .await;
        let mut consent = world.consent();
        consent.approved_session_ceiling_micro_dollars = Some(50_000);
        service
            .allocate(payer(1), world.session, operation(30), 3_000_000, consent)
            .await
            .expect("allocate with payer ceiling");
        let command = world.admit_command(30, INPUT_UNITS, OUTPUT_UNITS);
        let host_update = world.scratch.connect().await;
        host_update
            .batch_execute("BEGIN; SET LOCAL session_replication_role = replica")
            .await
            .expect("begin host ceiling increase");
        host_update
            .query_one(
                "SELECT finance.lock_flow('admission', $1::text::jsonb)::text",
                &[&serde_json::json!({
                    "session": world.session.hex(),
                    "principal": world.campaign.hex(),
                    "operation": operation(31).hex(),
                    "now_ms": support::attack::now_ms()
                })
                .to_string()],
            )
            .await
            .expect("lock host policy update");
        host_update
            .execute(
                "UPDATE finance.session_funding SET session_ceiling_micro_dollars = 20000000",
                &[],
            )
            .await
            .expect("raise session ceiling");
        let mut admitting = world.service().await;
        let admission_task = tokio::spawn(async move { admitting.admit(&command).await });
        wait_for_waiters(&world, 1).await;
        host_update
            .batch_execute("COMMIT")
            .await
            .expect("publish host ceiling");
        assert!(matches!(
            admission_task.await.expect("admission task"),
            Err(FinanceError::Admission(_))
        ));
        assert_eq!(world.count("reservation").await, 0);
        world.finish().await;
    });
}

async fn prepare_and_send(
    service: &mut support::world::Service,
    attempt: df_commerce::finance_ids::AttemptId,
) {
    service
        .prepare_dispatch(attempt, Deadlines::default())
        .await
        .expect("prepare");
    service
        .mark_attempt_sent(attempt, [9; 16], Deadlines::default())
        .await
        .expect("send");
}

#[test]
fn pot_04_removed_contributor_keeps_the_original_hold_and_funds_no_new_work() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut setup = world.service().await;
        for (member, reference) in [(payer(1), "pot_04_1"), (payer(2), "pot_04_2")] {
            setup
                .apply_purchase(purchase(member, reference, 5_000_000))
                .await
                .expect("purchase");
            setup
                .allocate(
                    member,
                    world.session,
                    operation(reference.as_bytes()[reference.len() - 1]),
                    5_000_000,
                    world.consent(),
                )
                .await
                .expect("allocate");
        }
        let first = world.admit_command(41, INPUT_UNITS, OUTPUT_UNITS);
        setup.admit(&first).await.expect("first admission");
        prepare_and_send(&mut setup, first.attempt).await;

        let mut remaining_member = world.service_with_members(vec![payer(2)]).await;
        let next = world.admit_command(42, INPUT_UNITS, OUTPUT_UNITS);
        remaining_member.admit(&next).await.expect("next admission");
        prepare_and_send(&mut remaining_member, next.attempt).await;
        remaining_member
            .settle(first.attempt, actual_units(), UsageSource::Reported)
            .await
            .expect("original hold settles")
            .expect("settlement");

        let first_shares: Vec<(Vec<u8>, i64)> = world
            .admin
            .query(
                "SELECT a.payer_account_id, s.charged_micro_dollars FROM finance.reservation_share s \
                 JOIN finance.allocation a ON a.allocation_id = s.allocation_id \
                 WHERE s.reservation_id = (SELECT reservation_id FROM finance.provider_attempt WHERE attempt_id = $1) \
                 ORDER BY a.payer_account_id",
                &[&first.attempt.as_bytes().to_vec()],
            )
            .await
            .expect("first shares")
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
        assert_eq!(first_shares.len(), 2);
        assert_eq!(first_shares[0].1, 67_500);
        assert_eq!(first_shares[1].1, 67_500);

        let new_payer_ids: Vec<Vec<u8>> = world
            .admin
            .query(
                "SELECT a.payer_account_id FROM finance.reservation_share s \
                 JOIN finance.allocation a ON a.allocation_id = s.allocation_id \
                 WHERE s.reservation_id = (SELECT reservation_id FROM finance.provider_attempt WHERE attempt_id = $1)",
                &[&next.attempt.as_bytes().to_vec()],
            )
            .await
            .expect("next shares")
            .into_iter()
            .map(|row| row.get(0))
            .collect();
        assert_eq!(new_payer_ids, vec![payer(2).as_bytes().to_vec()]);
        assert_eq!(world.natural(2130, None, None, None).await, 135_000);
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn pot_05_allocation_epoch_changes_only_the_shares_of_future_reservations() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        for (member, reference, amount) in [
            (payer(1), "pot_05_1", 3_000_000),
            (payer(2), "pot_05_2", 1_000_000),
        ] {
            service
                .apply_purchase(purchase(member, reference, 5_000_000))
                .await
                .expect("purchase");
            service
                .allocate(
                    member,
                    world.session,
                    operation(reference.as_bytes()[reference.len() - 1]),
                    amount,
                    world.consent(),
                )
                .await
                .expect("allocate");
        }
        let first = world.admit_command(51, INPUT_UNITS, OUTPUT_UNITS);
        service.admit(&first).await.expect("first admission");
        prepare_and_send(&mut service, first.attempt).await;

        service
            .allocate(
                payer(2),
                world.session,
                operation(52),
                2_000_000,
                world.consent(),
            )
            .await
            .expect("raise contribution");
        let future = world.admit_command(53, INPUT_UNITS, OUTPUT_UNITS);
        service.admit(&future).await.expect("future admission");
        prepare_and_send(&mut service, future.attempt).await;
        service
            .settle(first.attempt, actual_units(), UsageSource::Reported)
            .await
            .expect("settle old epoch")
            .expect("settlement");

        let first_shares: Vec<i64> = world
            .admin
            .query(
                "SELECT s.held_micro_dollars FROM finance.reservation_share s \
                 WHERE s.reservation_id = (SELECT reservation_id FROM finance.provider_attempt WHERE attempt_id = $1) \
                 ORDER BY s.allocation_id",
                &[&first.attempt.as_bytes().to_vec()],
            )
            .await
            .expect("old epoch shares")
            .into_iter()
            .map(|row| row.get(0))
            .collect();
        let future_shares: Vec<i64> = world
            .admin
            .query(
                "SELECT s.held_micro_dollars FROM finance.reservation_share s \
                 WHERE s.reservation_id = (SELECT reservation_id FROM finance.provider_attempt WHERE attempt_id = $1) \
                 ORDER BY s.allocation_id",
                &[&future.attempt.as_bytes().to_vec()],
            )
            .await
            .expect("new epoch shares")
            .into_iter()
            .map(|row| row.get(0))
            .collect();
        assert_eq!(first_shares, vec![101_250, 33_750]);
        assert_eq!(future_shares, vec![67_500, 67_500]);
        assert_eq!(world.count("funding_epoch").await, 4);
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}
