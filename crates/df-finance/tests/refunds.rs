#![cfg(not(target_arch = "wasm32"))]

//! Refunds and disputes (planning/finance-ledger-research.md 14, FIN-05).

mod support;

use df_commerce::dispute::DisputeOutcome;
use df_commerce::finance_ids::{LotId, RefundLotId};
use df_commerce::ledger::CurrencyTag;
use df_finance::fakes::{FakeRefundMode, FakeRefundProcessor};
use df_finance::refunds::RefundPayOutcome;
use support::attack::service_client;
use support::invariants;
use support::world::{World, operation, payer, purchase};

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

async fn refund_lots(world: &World, payer_tag: u8) -> Vec<(RefundLotId, LotId, i64, String)> {
    world
        .admin
        .query(
            "SELECT rl.refund_lot_id, rl.lot_id, rl.principal_micro_dollars, rl.state \
             FROM finance.refund_lot rl JOIN finance.refund r ON r.refund_id = rl.refund_id \
             WHERE r.payer_account_id = $1 ORDER BY rl.principal_micro_dollars DESC",
            &[&payer(payer_tag).as_bytes().to_vec()],
        )
        .await
        .expect("refund lots")
        .iter()
        .map(|row| {
            (
                RefundLotId::from_slice(&row.get::<_, Vec<u8>>(0)).expect("refund lot id"),
                LotId::from_slice(&row.get::<_, Vec<u8>>(1)).expect("lot id"),
                row.get(2),
                row.get(3),
            )
        })
        .collect()
}

#[test]
fn ref_01_partial_refund_spans_several_lots_to_the_original_sources() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        for (payment_ref, amount) in [
            ("ref01_a", 1_000_000),
            ("ref01_b", 2_000_000),
            ("ref01_c", 3_000_000),
        ] {
            service
                .apply_purchase(purchase(payer(1), payment_ref, amount))
                .await
                .expect("purchase");
        }
        let lots = world.lots_of(payer(1)).await;
        assert_eq!(lots.len(), 3);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis();
        world
            .seed(&format!(
                "UPDATE finance.funding_lot SET created_at_ms = {} WHERE lot_id = '\\x{}'; \
                 UPDATE finance.funding_lot SET created_at_ms = {} WHERE lot_id = '\\x{}'; \
                 UPDATE finance.funding_lot SET created_at_ms = {} WHERE lot_id = '\\x{}'",
                now_ms - 3000,
                lots[0].hex(),
                now_ms - 2000,
                lots[1].hex(),
                now_ms - 1000,
                lots[2].hex()
            ))
            .await;
        let refund = service
            .request_refund(payer(1), operation(90), 4_500_000, CurrencyTag::USD)
            .await
            .expect("request refund");
        let rows = refund_lots(&world, 1).await;
        assert_eq!(rows.len(), 2, "newest two lots fund the refund: {rows:?}");
        assert_eq!(rows[0].1, lots[2], "newest lot is refunded first");
        assert_eq!(rows[0].2, 3_000_000);
        assert_eq!(rows[1].1, lots[1]);
        assert_eq!(rows[1].2, 1_500_000);
        assert_eq!(
            world
                .natural(2110, Some(payer(1)), Some(lots[2]), None)
                .await,
            0
        );
        assert_eq!(
            world
                .natural(2110, Some(payer(1)), Some(lots[1]), None)
                .await,
            500_000
        );
        assert_eq!(
            world
                .natural(2110, Some(payer(1)), Some(lots[0]), None)
                .await,
            1_000_000
        );
        assert_eq!(
            world.natural(2500, None, Some(lots[2]), None).await,
            3_000_000
        );
        let processor = FakeRefundProcessor::new(FakeRefundMode::Succeed);
        for (refund_lot, lot, _, _) in &rows {
            let outcome = service
                .pay_refund_lot(*refund_lot, &processor)
                .await
                .expect("pay refund lot");
            assert_eq!(
                outcome,
                RefundPayOutcome::Paid {
                    processor_refund_ref: format!("fake-refund-{}", refund_lot.hex())
                }
            );
            let stored: String = world
                .admin
                .query_one(
                    "SELECT processor_refund_ref FROM finance.refund_lot WHERE refund_lot_id = $1",
                    &[&refund_lot.as_bytes().to_vec()],
                )
                .await
                .expect("stored processor reference")
                .get(0);
            assert_eq!(stored, format!("fake-refund-{}", refund_lot.hex()));
            let _ = lot;
        }
        assert_eq!(processor.paid_count(), 2);
        assert_eq!(world.line_sum(1100).await, 1_500_000);
        assert_eq!(world.natural(2500, None, None, None).await, 0);
        let refund_state: String = world
            .admin
            .query_one(
                "SELECT state FROM finance.refund WHERE refund_id = $1",
                &[&refund.as_bytes().to_vec()],
            )
            .await
            .expect("refund state")
            .get(0);
        assert_eq!(refund_state, "paid");
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn ref_02_refund_call_timeout_is_idempotent_and_credit_stays_unusable_until_confirmed() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "ref02", 5_000_000))
            .await
            .expect("purchase");
        let refund = service
            .request_refund(payer(1), operation(91), 2_000_000, CurrencyTag::USD)
            .await
            .expect("request refund");
        let rows = refund_lots(&world, 1).await;
        assert_eq!(rows.len(), 1);
        let refund_lot = rows[0].0;
        let processor = FakeRefundProcessor::new(FakeRefundMode::TimeoutOnce);
        assert_eq!(
            service
                .pay_refund_lot(refund_lot, &processor)
                .await
                .expect("timed-out pay"),
            RefundPayOutcome::Unknown
        );
        assert_eq!(processor.attempts(), 1);
        assert_eq!(processor.paid_count(), 0);
        let state: String = world
            .admin
            .query_one(
                "SELECT state FROM finance.refund_lot WHERE refund_lot_id = $1",
                &[&refund_lot.as_bytes().to_vec()],
            )
            .await
            .expect("refund lot state")
            .get(0);
        assert_eq!(state, "unknown");
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            3_000_000
        );
        assert_eq!(world.natural(2500, None, None, None).await, 2_000_000);
        assert_eq!(
            world
                .admin
                .query_one(
                    "SELECT count(*) FROM finance.journal_entry WHERE kind = 'refund_paid'",
                    &[]
                )
                .await
                .expect("refund paid entries")
                .get::<_, i64>(0),
            0
        );

        let holder = service_client(&world).await;
        holder.batch_execute("BEGIN").await.expect("begin holder");
        holder
            .query_one(
                "SELECT finance.lock_flow('refund_request', $1::text::jsonb)::text",
                &[
                    &serde_json::json!({"payer": payer(1).hex(), "operation": operation(91).hex()})
                        .to_string(),
                ],
            )
            .await
            .expect("hold payer lock");
        let duplicate = world.service().await;
        let duplicate_task = tokio::spawn(async move {
            let mut service = duplicate;
            service
                .request_refund(payer(1), operation(91), 2_000_000, CurrencyTag::USD)
                .await
        });
        wait_for_waiters(&world, 1).await;
        holder.batch_execute("ROLLBACK").await.expect("release");
        assert_eq!(
            duplicate_task
                .await
                .expect("duplicate task")
                .expect("duplicate request"),
            refund,
            "the same operation returns the first refund, never a second one"
        );
        assert_eq!(world.count("refund").await, 1);
        assert_eq!(world.count("refund_lot").await, 1);

        assert_eq!(
            service
                .pay_refund_lot(refund_lot, &processor)
                .await
                .expect("retry pay"),
            RefundPayOutcome::Paid {
                processor_refund_ref: format!("fake-refund-{}", refund_lot.hex())
            }
        );
        assert_eq!(processor.attempts(), 2);
        assert_eq!(
            processor.paid_count(),
            1,
            "one processor payment per refund lot"
        );
        assert_eq!(
            service
                .pay_refund_lot(refund_lot, &processor)
                .await
                .expect("third pay"),
            RefundPayOutcome::AlreadyPaid
        );
        assert_eq!(processor.attempts(), 2, "a paid lot is not called again");
        assert_eq!(world.natural(2500, None, None, None).await, 0);
        assert_eq!(
            world.natural(2110, Some(payer(1)), None, None).await,
            3_000_000
        );
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}

#[test]
fn ref_03_one_contributors_dispute_after_usage_affects_only_that_contributors_lots() {
    support::run(async {
        let Some(world) = World::open().await else {
            return;
        };
        let mut service = world.service().await;
        service
            .apply_purchase(purchase(payer(1), "ref03_a", 5_000_000))
            .await
            .expect("purchase a");
        service
            .apply_purchase(purchase(payer(2), "ref03_b", 5_000_000))
            .await
            .expect("purchase b");
        for tag in [1_u8, 2] {
            service
                .allocate(
                    payer(tag),
                    world.session,
                    operation(92 + tag),
                    3_000_000,
                    world.consent(),
                )
                .await
                .expect("allocate");
        }
        let lot_a = world.lots_of(payer(1)).await[0];
        let lot_b = world.lots_of(payer(2)).await[0];
        let allocation_b = world.allocation_of(payer(2)).await.expect("allocation b");
        let command = world.admit_command(95, 400_000, 100_000);
        service.admit(&command).await.expect("admission");
        service
            .prepare_dispatch(
                command.attempt,
                df_commerce::lifecycle::Deadlines::default(),
            )
            .await
            .expect("prepare");
        service
            .mark_attempt_sent(
                command.attempt,
                [9; 16],
                df_commerce::lifecycle::Deadlines::default(),
            )
            .await
            .expect("send");
        assert!(
            service
                .settle(
                    command.attempt,
                    vec![
                        ("text_input".into(), 400_000),
                        ("text_output".into(), 100_000)
                    ],
                    df_commerce::settlement::UsageSource::Reported,
                )
                .await
                .expect("settle")
                .is_some()
        );
        let other_before = (
            world.natural(2110, Some(payer(2)), Some(lot_b), None).await,
            world
                .natural(2120, None, Some(lot_b), Some(allocation_b))
                .await,
            world.natural(2140, None, Some(lot_b), None).await,
        );
        let frozen_before = world.natural(2110, Some(payer(1)), Some(lot_a), None).await
            + world.natural(2120, None, Some(lot_a), None).await;
        assert!(frozen_before > 0);
        service
            .open_dispute(lot_a, "fake", "ref03-dispute", 5_000_000, 0)
            .await
            .expect("open dispute")
            .expect("dispute event");
        assert_eq!(
            world.natural(2140, None, Some(lot_a), None).await,
            frozen_before
        );
        assert_eq!(
            world.natural(2110, Some(payer(1)), Some(lot_a), None).await,
            0
        );
        assert_eq!(world.natural(2120, None, Some(lot_a), None).await, 0);
        assert_eq!(
            (
                world.natural(2110, Some(payer(2)), Some(lot_b), None).await,
                world
                    .natural(2120, None, Some(lot_b), Some(allocation_b))
                    .await,
                world.natural(2140, None, Some(lot_b), None).await,
            ),
            other_before,
            "the other contributor is untouched by the dispute"
        );
        let lot_b_state: String = world
            .admin
            .query_one(
                "SELECT state FROM finance.funding_lot WHERE lot_id = $1",
                &[&lot_b.as_bytes().to_vec()],
            )
            .await
            .expect("lot b state")
            .get(0);
        assert_eq!(lot_b_state, "active");
        service
            .close_dispute(
                lot_a,
                "fake",
                "ref03-dispute",
                DisputeOutcome::Lost {
                    disputed_micro_dollars: 5_000_000,
                },
            )
            .await
            .expect("close dispute")
            .expect("close event");
        assert_eq!(world.natural(2140, None, Some(lot_a), None).await, 0);
        let lot_a_state: String = world
            .admin
            .query_one(
                "SELECT state FROM finance.funding_lot WHERE lot_id = $1",
                &[&lot_a.as_bytes().to_vec()],
            )
            .await
            .expect("lot a state")
            .get(0);
        assert_eq!(lot_a_state, "charged_back");
        assert_eq!(
            (
                world.natural(2110, Some(payer(2)), Some(lot_b), None).await,
                world
                    .natural(2120, None, Some(lot_b), Some(allocation_b))
                    .await,
                world.natural(2140, None, Some(lot_b), None).await,
            ),
            other_before,
            "the loss is never shifted to the other contributor"
        );
        assert!(world.line_sum(5500).await > 0, "the loss is the platform's");
        invariants::all(&world.admin).await.expect("invariants");
        world.finish().await;
    });
}
