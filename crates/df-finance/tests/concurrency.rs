#![cfg(not(target_arch = "wasm32"))]

//! Concurrent flows against one database: attempts that share a lot settle together, the same
//! admission arrives from several connections at once, and every flow runs at the same time as
//! every other. Nothing may deadlock (40P01), time out on a lock (55P03), or leave an invariant
//! broken (planning/finance-ledger-research.md 19.1: one lock order for every flow).

mod support;

use df_commerce::finance_ids::{AttemptId, UsageEventId};
use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::{ReleaseReason, UsageSource};
use df_finance::admit::AdmissionOutcome;
use df_finance::service::FinanceError;
use serde_json::json;
use support::attack::service_client;
use support::invariants;
use support::world::{Service, World, credit_command, operation, payer, purchase};

const TOKENS: [(&str, u64); 2] = [("text_input", 400_000), ("text_output", 100_000)];

fn units(scale: u64) -> Vec<(String, u64)> {
    TOKENS
        .iter()
        .map(|(name, count)| ((*name).to_owned(), count * scale / 4))
        .collect()
}

/// Fails the test on a deadlock or a lock timeout; any other error is the caller's to judge.
fn assert_no_lock_failure<T: std::fmt::Debug>(name: &str, result: &Result<T, FinanceError>) {
    if let Err(error) = result {
        let code = error.sqlstate().unwrap_or_default();
        assert!(code != "40P01" && code != "55P03", "{name}: {error:?}");
    }
}

async fn admit_and_send(world: &World, service: &mut Service, tag: u8) -> AttemptId {
    let command = world.admit_command(tag, 400_000, 100_000);
    service.admit(&command).await.expect("admitted");
    service
        .prepare_dispatch(command.attempt, Deadlines::default())
        .await
        .expect("prepared");
    service
        .mark_attempt_sent(command.attempt, [9; 16], Deadlines::default())
        .await
        .expect("sent");
    command.attempt
}

#[test]
fn six_settlements_that_share_one_lot_all_commit() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut setup = world.service().await;
        setup
            .apply_purchase(purchase(payer(1), "pay_1", 20_000_000))
            .await
            .expect("purchase");
        setup
            .allocate(
                payer(1),
                world.session,
                operation(1),
                20_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let mut attempts = Vec::new();
        for tag in 1..=6 {
            attempts.push(admit_and_send(&world, &mut setup, tag).await);
        }
        let mut services = Vec::new();
        for _ in 0..6 {
            services.push(world.service().await);
        }
        let [mut s1, mut s2, mut s3, mut s4, mut s5, mut s6]: [Service; 6] =
            services.try_into().ok().expect("six");
        let results = tokio::join!(
            s1.settle(attempts[0], units(4), UsageSource::Reported),
            s2.settle(attempts[1], units(4), UsageSource::Reported),
            s3.settle(attempts[2], units(4), UsageSource::Reported),
            s4.settle(attempts[3], units(4), UsageSource::Reported),
            s5.settle(attempts[4], units(4), UsageSource::Reported),
            s6.settle(attempts[5], units(4), UsageSource::Reported),
        );
        for (index, result) in [
            &results.0, &results.1, &results.2, &results.3, &results.4, &results.5,
        ]
        .into_iter()
        .enumerate()
        {
            assert_no_lock_failure("settle", result);
            assert!(
                matches!(result, Ok(Some(_))),
                "settlement {index}: {result:?}"
            );
        }
        // 6 attempts of 400,000 input and 100,000 output tokens at 0.15 / 0.75 retail.
        let consumed: i64 = world
            .admin
            .query_one(
                "SELECT sum(consumed_micro_dollars)::bigint FROM finance.funding_lot",
                &[],
            )
            .await
            .expect("lot")
            .get(0);
        assert_eq!(consumed, 6 * (60_000 + 75_000));
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn pot_01_six_concurrent_admissions_never_reserve_or_dispatch_above_the_pot() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut setup = world.service().await;
        setup
            .apply_purchase(purchase(payer(1), "pot_01", 1_000_000))
            .await
            .expect("purchase");
        setup
            .allocate(
                payer(1),
                world.session,
                operation(61),
                200_000,
                world.consent(),
            )
            .await
            .expect("small pot");

        let holder = service_client(&world).await;
        holder.batch_execute("BEGIN").await.expect("begin");
        holder
            .query_one(
                "SELECT finance.lock_flow('admission', $1::text::jsonb)::text",
                &[&json!({
                    "session": world.session.hex(),
                    "principal": world.campaign.hex(),
                    "operation": operation(62).hex(),
                    "now_ms": support::attack::now_ms(),
                })
                .to_string()],
            )
            .await
            .expect("hold the session admission lock");

        let mut services = Vec::new();
        for _ in 0..6 {
            services.push(world.service().await);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f]: [Service; 6] = services
            .try_into()
            .ok()
            .expect("six independent connections");
        let commands = [
            world.admit_command(63, 400_000, 100_000),
            world.admit_command(64, 400_000, 100_000),
            world.admit_command(65, 400_000, 100_000),
            world.admit_command(66, 400_000, 100_000),
            world.admit_command(67, 400_000, 100_000),
            world.admit_command(68, 400_000, 100_000),
        ];
        let release = async {
            let mut waiting = false;
            for _ in 0..10_000 {
                let count: i64 = world
                    .admin
                    .query_one(
                        "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
                         WHERE NOT l.granted AND a.datname = current_database()",
                        &[],
                    )
                    .await
                    .expect("pg_locks")
                    .get(0);
                if count >= 6 {
                    waiting = true;
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert!(waiting, "six admissions did not wait on PostgreSQL locks");
            holder.batch_execute("ROLLBACK").await.expect("release");
        };
        let (results, ()) = tokio::join!(
            async {
                tokio::join!(
                    a.admit(&commands[0]),
                    b.admit(&commands[1]),
                    c.admit(&commands[2]),
                    d.admit(&commands[3]),
                    e.admit(&commands[4]),
                    f.admit(&commands[5]),
                )
            },
            release
        );

        let outcomes = [
            results.0, results.1, results.2, results.3, results.4, results.5,
        ];
        let mut admitted = Vec::new();
        for result in outcomes {
            match result {
                Ok(AdmissionOutcome::Admitted { attempt, .. }) => admitted.push(attempt),
                Err(FinanceError::Admission(_)) => {}
                other => panic!("unexpected admission result: {other:?}"),
            }
        }
        assert_eq!(admitted.len(), 1, "one 135,000 micro-dollar hold fits");
        let mut dispatcher = world.service().await;
        dispatcher
            .prepare_dispatch(admitted[0], Deadlines::default())
            .await
            .expect("prepare the admitted attempt");
        dispatcher
            .mark_attempt_sent(admitted[0], [9; 16], Deadlines::default())
            .await
            .expect("send the admitted attempt");

        let held = world.natural(2130, None, None, None).await;
        let available = world.natural(2120, None, None, None).await;
        assert_eq!(held, 135_000);
        assert_eq!(available, 65_000);
        assert_eq!(world.count("provider_attempt").await, 1);
        assert_eq!(world.count("reservation").await, 1);
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn the_same_admission_from_four_connections_admits_once() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut setup = world.service().await;
        setup
            .apply_purchase(purchase(payer(1), "pay_1", 10_000_000))
            .await
            .expect("purchase");
        setup
            .allocate(
                payer(1),
                world.session,
                operation(1),
                10_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
        let command = world.admit_command(1, 400_000, 100_000);
        let (mut a, mut b, mut c, mut d) = (
            world.service().await,
            world.service().await,
            world.service().await,
            world.service().await,
        );
        let results = tokio::join!(
            a.admit(&command),
            b.admit(&command),
            c.admit(&command),
            d.admit(&command)
        );
        let all = [&results.0, &results.1, &results.2, &results.3];
        for result in all {
            assert_no_lock_failure("admit", result);
            assert!(result.is_ok(), "{result:?}");
        }
        let fresh = all
            .iter()
            .filter(|result| matches!(result, Ok(AdmissionOutcome::Admitted { .. })))
            .count();
        assert_eq!(
            fresh, 1,
            "exactly one connection admits; the others see the stored outcome: {all:?}"
        );
        assert_eq!(world.count("reservation").await, 1);
        assert_eq!(world.count("operation_receipt").await, 1);
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn every_flow_at_once_never_deadlocks_and_keeps_every_invariant() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut setup = world.service().await;
        for tag in 1..=2 {
            setup
                .apply_purchase(purchase(payer(tag), &format!("pay_{tag}"), 30_000_000))
                .await
                .expect("purchase");
            setup
                .allocate(
                    payer(tag),
                    world.session,
                    operation(tag),
                    10_000_000,
                    world.consent(),
                )
                .await
                .expect("allocate");
        }
        for round in 0..3_u8 {
            let base = round * 10;
            // Work in flight for the round: two sent attempts to settle, one admitted to cancel, one
            // settled to credit.
            let to_settle_a = admit_and_send(&world, &mut setup, base + 1).await;
            let to_settle_b = admit_and_send(&world, &mut setup, base + 2).await;
            let command = world.admit_command(base + 3, 400_000, 100_000);
            setup.admit(&command).await.expect("admitted");
            let to_cancel = command.attempt;
            let to_credit = admit_and_send(&world, &mut setup, base + 4).await;
            setup
                .settle(to_credit, units(4), UsageSource::Reported)
                .await
                .expect("settled")
                .expect("committed");
            let usage: UsageEventId = UsageEventId::from_slice(
                &world
                    .admin
                    .query_one(
                        "SELECT usage_event_id FROM finance.usage_event WHERE attempt_id = $1",
                        &[&to_credit.as_bytes().to_vec()],
                    )
                    .await
                    .expect("usage")
                    .get::<_, Vec<u8>>(0),
            )
            .expect("id");

            let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
                world.service().await,
                world.service().await,
                world.service().await,
                world.service().await,
                world.service().await,
                world.service().await,
                world.service().await,
                world.service().await,
            );
            let new_admission = world.admit_command(base + 5, 400_000, 100_000);
            let case = format!("case-{round}");
            let late = format!("late_{round}");
            let results = tokio::join!(
                a.settle(to_settle_a, units(4), UsageSource::Reported),
                b.settle(to_settle_b, units(2), UsageSource::Reported),
                c.release(to_cancel, ReleaseReason::NotDispatched),
                d.credit(credit_command(usage, operation(100 + round), 1_000, &case)),
                e.allocate(
                    payer(1),
                    world.session,
                    operation(50 + round),
                    500_000,
                    world.consent()
                ),
                f.allocate(
                    payer(2),
                    world.session,
                    operation(60 + round),
                    500_000,
                    world.consent()
                ),
                g.admit(&new_admission),
                h.apply_purchase(purchase(payer(1), &late, 1_000_000)),
            );
            assert_no_lock_failure("settle a", &results.0);
            assert_no_lock_failure("settle b", &results.1);
            assert_no_lock_failure("release", &results.2);
            assert_no_lock_failure("credit", &results.3);
            assert_no_lock_failure("allocate 1", &results.4);
            assert_no_lock_failure("allocate 2", &results.5);
            assert_no_lock_failure("admit", &results.6);
            assert_no_lock_failure("purchase", &results.7);
            assert!(
                matches!(results.0, Ok(Some(_))) && matches!(results.1, Ok(Some(_))),
                "{:?} {:?}",
                results.0,
                results.1
            );
            assert!(matches!(results.2, Ok(Some(_))), "{:?}", results.2);
            assert!(results.3.is_ok(), "{:?}", results.3);
            invariants::all(&world.admin)
                .await
                .unwrap_or_else(|failure| panic!("round {round}: {failure}"));
        }
        world.finish().await;
    });
}
