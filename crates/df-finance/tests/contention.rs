#![cfg(not(target_arch = "wasm32"))]

//! Real contention on the ranks below the session lock (planning/finance-ledger-research.md 19.1).
//! Two tables (sessions) fund their allocations from the same lot of one payer, so an admission in
//! one and a settlement in the other meet only at the lot, rank 6: one holds it, the other is seen
//! waiting in the lock tables, and neither deadlocks. A lock wait that outlasts the role timeout
//! (`55P03`) is the safe outcome and passes through; `40P01` is the failure.

mod support;

use std::time::Duration;

use df_commerce::finance_ids::SessionKey;
use df_commerce::funding_flows::SessionPolicy;
use df_commerce::lifecycle::Deadlines;
use df_commerce::settlement::UsageSource;
use df_finance::admit::AdmissionOutcome;
use df_finance::service::FinanceError;
use serde_json::json;
use support::attack::service_client;
use support::world::{World, operation, payer, purchase};

const SECOND: SessionKey = SessionKey::from_array([0x5a; 16]);

/// Waits until some backend of the test database waits for a lock, or gives up.
async fn wait_for_a_lock_waiter(world: &World) -> bool {
    for _ in 0..200 {
        let waiting: i64 = world
            .admin
            .query_one(
                "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
                  WHERE NOT l.granted AND a.datname = current_database()",
                &[],
            )
            .await
            .expect("pg_locks")
            .get(0);
        if waiting > 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

fn assert_no_deadlock<T: std::fmt::Debug>(result: &Result<T, FinanceError>) {
    if let Err(error) = result {
        assert_ne!(error.sqlstate(), Some("40P01"), "{error:?}");
    }
}

struct Tables {
    world: World,
    settling: df_commerce::finance_ids::AttemptId,
}

/// Payer 1 funds both tables from one purchased lot; table two has a sent attempt to settle.
async fn two_tables_on_one_lot() -> Option<Tables> {
    let world = World::open().await?;
    let mut service = world.service().await;
    service
        .apply_purchase(purchase(payer(1), "pay_shared", 20_000_000))
        .await
        .expect("purchase");
    world
        .open_session(
            &mut service,
            SECOND,
            SessionPolicy {
                hourly_ceiling_micro_dollars: 20_000_000,
                session_ceiling_micro_dollars: None,
                per_request_threshold_micro_dollars: 5_000_000,
            },
        )
        .await;
    for (session, tag) in [(world.session, 1), (SECOND, 2)] {
        service
            .allocate(
                payer(1),
                session,
                operation(tag),
                6_000_000,
                world.consent(),
            )
            .await
            .expect("allocate");
    }
    let mut command = world.admit_command(7, 400_000, 100_000);
    command.session = SECOND;
    service.admit(&command).await.expect("admitted");
    service
        .prepare_dispatch(command.attempt, Deadlines::default())
        .await
        .expect("prepared");
    service
        .mark_attempt_sent(command.attempt, [9; 16], Deadlines::default())
        .await
        .expect("sent");
    let settling = command.attempt;
    drop(service);
    Some(Tables { world, settling })
}

#[test]
fn an_admission_waits_for_a_settlement_that_holds_the_shared_lot_and_neither_deadlocks() {
    support::run(async {
        let Some(tables) = two_tables_on_one_lot().await else {
            return;
        };
        let world = &tables.world;
        // The settlement's locks, taken by hand and held: session two, its allocation, the lot.
        let holder = service_client(world).await;
        holder.batch_execute("BEGIN").await.expect("begin");
        holder
            .query_one(
                "SELECT finance.lock_flow('settlement', $1::text::jsonb)::text",
                &[&json!({"attempt": tables.settling.hex()}).to_string()],
            )
            .await
            .expect("the settlement's locks");
        let mut admitting = world.service().await;
        let command = world.admit_command(8, 400_000, 100_000);
        let release = async {
            assert!(
                wait_for_a_lock_waiter(world).await,
                "the admission never waited on the lot"
            );
            holder.batch_execute("ROLLBACK").await.expect("release");
        };
        let (outcome, ()) = tokio::join!(admitting.admit(&command), release);
        assert_no_deadlock(&outcome);
        match &outcome {
            Ok(admitted) => assert!(
                matches!(admitted, AdmissionOutcome::Admitted { .. }),
                "{admitted:?}"
            ),
            Err(error) => assert!(error.is_lock_timeout(), "{error:?}"),
        }
        support::invariants::all(&world.admin)
            .await
            .expect("invariants");
        tables.world.finish().await;
    });
}

#[test]
fn a_settlement_waits_for_an_admission_that_holds_the_shared_lot_and_neither_deadlocks() {
    support::run(async {
        let Some(tables) = two_tables_on_one_lot().await else {
            return;
        };
        let world = &tables.world;
        // The admission's locks on table one, held: session one, its allocation, the lot FOR SHARE.
        let holder = service_client(world).await;
        holder.batch_execute("BEGIN").await.expect("begin");
        holder
            .query_one(
                "SELECT finance.lock_flow('admission', $1::text::jsonb)::text",
                &[&json!({
                    "session": world.session.hex(),
                    "principal": world.campaign.hex(),
                    "operation": operation(9).hex(),
                    "now_ms": support::attack::now_ms(),
                })
                .to_string()],
            )
            .await
            .expect("the admission's locks");
        let mut settling = world.service().await;
        let release = async {
            assert!(
                wait_for_a_lock_waiter(world).await,
                "the settlement never waited on the lot"
            );
            holder.batch_execute("ROLLBACK").await.expect("release");
        };
        let (outcome, ()) = tokio::join!(
            settling.settle(
                tables.settling,
                vec![("text_input".into(), 100_000), ("text_output".into(), 0)],
                UsageSource::Reported
            ),
            release
        );
        assert_no_deadlock(&outcome);
        match &outcome {
            Ok(settled) => assert!(settled.is_some()),
            Err(error) => assert!(error.is_lock_timeout(), "{error:?}"),
        }
        support::invariants::all(&world.admin)
            .await
            .expect("invariants");
        tables.world.finish().await;
    });
}

#[test]
fn two_first_grants_for_the_same_new_account_make_one_grant_and_no_raw_error() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let (mut a, mut b) = (world.service().await, world.service().await);
        let (first, second) = tokio::join!(
            a.grant_trial(payer(5), [5; 32], support::world::CURRENCY),
            b.grant_trial(payer(5), [6; 32], support::world::CURRENCY)
        );
        let (first, second) = (first.expect("first"), second.expect("second"));
        assert_eq!(
            first.is_some() as u8 + second.is_some() as u8,
            1,
            "exactly one grants"
        );
        assert_eq!(world.count("funding_lot").await, 1);
        // The same for two first purchases of the same payment.
        let (mut c, mut d) = (world.service().await, world.service().await);
        let (third, fourth) = tokio::join!(
            c.apply_purchase(purchase(payer(6), "pay_race", 1_000_000)),
            d.apply_purchase(purchase(payer(6), "pay_race", 1_000_000))
        );
        let (third, fourth) = (third.expect("third"), fourth.expect("fourth"));
        assert_eq!(third.is_some() as u8 + fourth.is_some() as u8, 1);
        world.finish().await;
    });
}

#[test]
fn a_quote_cannot_be_decided_through_another_session() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        world
            .open_session(
                &mut service,
                SECOND,
                SessionPolicy {
                    hourly_ceiling_micro_dollars: 20_000_000,
                    session_ceiling_micro_dollars: None,
                    per_request_threshold_micro_dollars: 5_000_000,
                },
            )
            .await;
        let command = world.admit_command(1, 1000, 1000);
        let quote = service
            .create_quote(
                &world.version,
                df_finance::held::QuoteCommand {
                    session: world.session,
                    route: command.route.clone(),
                    work_class: command.work_class,
                    maximum_units: command.maximum_units.clone(),
                    estimate_units: command.estimate_units.clone(),
                    request_digest: command.request_digest,
                    lifetime_ms: 600_000,
                },
            )
            .await
            .expect("quote");
        let wrong = service.decide_quote(SECOND, quote, true).await;
        assert!(matches!(wrong, Err(FinanceError::Stale)), "{wrong:?}");
        service
            .decide_quote(world.session, quote, true)
            .await
            .expect("its own session decides");
        world.finish().await;
    });
}
